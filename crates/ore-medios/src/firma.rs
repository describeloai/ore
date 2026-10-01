//! **Firmar en lote**: una URL de lectura por blob, en paralelo, con la cuenta
//! que el proceso tiene viva —un solo cliente y sus conexiones, que es lo que
//! baja 100 firmas de 52 s a 0,69 s (B2·0)—. El tipo y la disposición van
//! dentro de la firma: quien tenga la URL no puede cambiar cómo se sirve.

use ore_store::almacen::Almacen;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Cuántas firmas a la vez (B2·0: 32 a la vez, ~320 por segundo).
pub const HILOS: usize = 32;

/// Lo que se firma de un blob.
pub struct Pedida {
    pub blob: String,
    pub tipo: Option<String>,
    pub disposicion: Option<String>,
}

/// Una URL por pedida, en su orden; el fallo de una no cambia las demás.
pub fn firmar(
    cuenta: &Arc<dyn Almacen>,
    pedidas: &[Pedida],
    segundos: u64,
) -> Vec<Result<String, String>> {
    let hechas: Mutex<Vec<Option<Result<String, String>>>> = Mutex::new(vec![None; pedidas.len()]);
    let siguiente = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..HILOS.min(pedidas.len()) {
            s.spawn(|| {
                loop {
                    let i = siguiente.fetch_add(1, Ordering::Relaxed);
                    let Some(p) = pedidas.get(i) else { return };
                    let mut r: Vec<(&str, &str)> = Vec::new();
                    if let Some(t) = &p.tipo {
                        r.push(("response-content-type", t));
                    }
                    if let Some(d) = &p.disposicion {
                        r.push(("response-content-disposition", d));
                    }
                    let u =
                        cuenta.firmar_lectura(&ore_store::blobs::clave_de(&p.blob), segundos, &r);
                    hechas.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(u);
                }
            });
        }
    });
    hechas
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .map(|u| u.unwrap_or_else(|| Err("sin firmar".into())))
        .collect()
}

#[cfg(test)]
pub mod pruebas {
    use super::*;

    /// Un almacén que firma sin red: la URL lleva la clave y la vida, y un
    /// blob `malo` falla.
    pub struct Firmante;

    impl Almacen for Firmante {
        fn base(&self) -> String {
            "gs://pruebas".into()
        }
        fn leer(&self, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn existe(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        fn subir(&self, _: &str, _: &[u8]) -> Result<bool, String> {
            Ok(false)
        }
        fn listar(&self, _: &str) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        fn borrar(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn leer_bytes(&self, _: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(None)
        }
        fn firmar_lectura(
            &self,
            clave: &str,
            segundos: u64,
            r: &[(&str, &str)],
        ) -> Result<String, String> {
            if clave.ends_with("malo") {
                return Err("no se firma".into());
            }
            Ok(format!(
                "https://firmada/{clave}?vida={segundos}&r={}",
                r.len()
            ))
        }
    }

    #[test]
    fn en_su_orden_y_el_fallo_de_una_no_tumba_las_demas() {
        let c: Arc<dyn Almacen> = Arc::new(Firmante);
        let pedidas: Vec<Pedida> = ["aa", "malo", "bb"]
            .iter()
            .map(|b| Pedida {
                blob: b.to_string(),
                tipo: Some("application/pdf".into()),
                disposicion: None,
            })
            .collect();
        let r = firmar(&c, &pedidas, 300);
        assert!(
            r[0].as_ref()
                .unwrap()
                .contains("blobs/sha256/aa?vida=300&r=1"),
            "{:?}",
            r[0]
        );
        assert!(r[1].is_err());
        assert!(r[2].as_ref().unwrap().contains("blobs/sha256/bb"));
    }
}
