//! **Los bytes de un objeto, del origen al almacén** (0046 E8·2): el contrato
//! común por el que un lector (`ore-read-<tipo> bajar`) entrega ficheros y el
//! almacén (`ore-store blobs`) los guarda por contenido. No es de S3: un
//! lector de GCS, de Azure o de un SFTP entrega lo mismo.
//!
//! Un flujo de **tramas**, una por ítem, en el orden que el lector quiera:
//!
//! ```text
//! {"clave":…,"version":…,"huella":…,"tipo":…,"tamano":N}\n   la cabecera
//! <N bytes>                                                   el contenido
//! {"fin":"ok"}\n  ó  {"fin":"error","motivo":…}\n             el cierre
//! ```
//!
//! o, si el ítem no se pudo ni empezar, una sola línea
//! `{"clave":…,"version":…,"error":…}`.
//!
//! El cierre existe porque la huella del origen (el CRC64NVME de S3) se coteja
//! **mientras** pasan los bytes: sólo al final se sabe si eran los que el
//! manifiesto dice, y un ítem que no cuadra se descarta entero —el almacén no
//! lo guarda—. Si la conexión se corta a mitad, el lector completa los `N`
//! bytes con ceros y cierra con error: el flujo sigue siendo legible y los
//! demás ítems pasan.

use ore_core::json::Json;
use std::io::{BufRead, Write};

/// La cabecera de un ítem que llega con sus bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cabecera {
    pub clave: String,
    pub version: String,
    pub huella: String,
    pub tipo: String,
    pub tamano: u64,
}

/// Lo que se lee al empezar una trama.
#[derive(Debug, PartialEq, Eq)]
pub enum Trama {
    /// Vienen `tamano` bytes y un cierre.
    Bytes(Cabecera),
    /// El ítem no llega, y por qué.
    Fallo {
        clave: String,
        version: String,
        motivo: String,
    },
}

pub fn escribir_cabecera(w: &mut dyn Write, c: &Cabecera) -> std::io::Result<()> {
    let j = Json::obj([
        ("clave", Json::s(&c.clave)),
        ("version", Json::s(&c.version)),
        ("huella", Json::s(&c.huella)),
        ("tipo", Json::s(&c.tipo)),
        ("tamano", Json::Int(c.tamano as i64)),
    ]);
    writeln!(w, "{}", j.jcs())
}

pub fn escribir_cierre(w: &mut dyn Write, r: Result<(), &str>) -> std::io::Result<()> {
    let j = match r {
        Ok(()) => Json::obj([("fin", Json::s("ok"))]),
        Err(m) => Json::obj([("fin", Json::s("error")), ("motivo", Json::s(m))]),
    };
    writeln!(w, "{}", j.jcs())
}

pub fn escribir_fallo(
    w: &mut dyn Write,
    clave: &str,
    version: &str,
    motivo: &str,
) -> std::io::Result<()> {
    let j = Json::obj([
        ("clave", Json::s(clave)),
        ("version", Json::s(version)),
        ("error", Json::s(motivo)),
    ]);
    writeln!(w, "{}", j.jcs())
}

fn campo(n: &ore_core::parse::Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
}

/// La trama siguiente, o `None` al acabar el flujo. Tras una
/// [`Trama::Bytes`], quien lee toma exactamente `tamano` bytes y después
/// llama a [`leer_cierre`].
pub fn leer_trama(r: &mut dyn BufRead) -> Result<Option<Trama>, String> {
    let mut l = String::new();
    loop {
        l.clear();
        if r.read_line(&mut l)
            .map_err(|e| format!("el flujo de bytes no se pudo leer: {e}"))?
            == 0
        {
            return Ok(None);
        }
        if !l.trim().is_empty() {
            break;
        }
    }
    let n = ore_core::parse::parse(l.trim())
        .map_err(|e| format!("una cabecera de trama no analiza: {e:?}"))?;
    let clave = campo(&n, "clave").ok_or("una trama sin `clave`")?;
    let version = campo(&n, "version").unwrap_or_default();
    if let Some(motivo) = campo(&n, "error") {
        return Ok(Some(Trama::Fallo {
            clave,
            version,
            motivo,
        }));
    }
    let tamano = campo(&n, "tamano")
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| format!("la trama de `{clave}` no dice su `tamano`"))?;
    Ok(Some(Trama::Bytes(Cabecera {
        clave,
        version,
        huella: campo(&n, "huella").unwrap_or_default(),
        tipo: campo(&n, "tipo").unwrap_or_default(),
        tamano,
    })))
}

/// El cierre de una trama: `Ok(())` si el lector dio los bytes por buenos.
pub fn leer_cierre(r: &mut dyn BufRead) -> Result<Result<(), String>, String> {
    let mut l = String::new();
    r.read_line(&mut l)
        .map_err(|e| format!("el cierre de una trama no se pudo leer: {e}"))?;
    let n = ore_core::parse::parse(l.trim())
        .map_err(|_| format!("una trama no se cerró bien: `{}`", l.trim()))?;
    match campo(&n, "fin").as_deref() {
        Some("ok") => Ok(Ok(())),
        Some("error") => Ok(Err(campo(&n, "motivo").unwrap_or_default())),
        _ => Err(format!("una trama no se cerró bien: `{}`", l.trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    #[test]
    fn un_flujo_de_tramas_va_y_vuelve() {
        let mut w = Vec::new();
        let c = Cabecera {
            clave: "r/a b.pdf".into(),
            version: "v1".into(),
            huella: "crc64nvme:x".into(),
            tipo: "application/pdf".into(),
            tamano: 5,
        };
        escribir_cabecera(&mut w, &c).unwrap();
        w.extend_from_slice(b"\n{}\0x");
        escribir_cierre(&mut w, Ok(())).unwrap();
        escribir_fallo(&mut w, "r/c.pdf", "v2", "404").unwrap();
        escribir_cabecera(
            &mut w,
            &Cabecera {
                tamano: 2,
                ..c.clone()
            },
        )
        .unwrap();
        w.extend_from_slice(b"\0\0");
        escribir_cierre(&mut w, Err("la huella no cuadra")).unwrap();

        let mut r = std::io::BufReader::new(&w[..]);
        let Some(Trama::Bytes(c1)) = leer_trama(&mut r).unwrap() else {
            panic!()
        };
        assert_eq!(c1, c);
        let mut b = Vec::new();
        (&mut r).take(c1.tamano).read_to_end(&mut b).unwrap();
        assert_eq!(
            b, b"\n{}\0x",
            "los bytes pasan tal cual, con saltos y nulos"
        );
        assert_eq!(leer_cierre(&mut r).unwrap(), Ok(()));
        assert_eq!(
            leer_trama(&mut r).unwrap(),
            Some(Trama::Fallo {
                clave: "r/c.pdf".into(),
                version: "v2".into(),
                motivo: "404".into()
            })
        );
        let Some(Trama::Bytes(c3)) = leer_trama(&mut r).unwrap() else {
            panic!()
        };
        let mut b = Vec::new();
        (&mut r).take(c3.tamano).read_to_end(&mut b).unwrap();
        assert_eq!(
            leer_cierre(&mut r).unwrap(),
            Err("la huella no cuadra".to_string())
        );
        assert_eq!(leer_trama(&mut r).unwrap(), None);
    }
}
