//! **Una puerta en memoria, para las pruebas de este crate**: lo que se prueba aquí es el transporte
//! (el protocolo) y la grabadora, no el almacén. `ore-core` no puede usar el catálogo, que depende de
//! él; lo que el almacén promete lo prueba `ore-catalogo` contra Postgres.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::{
    Cambio, Contexto, De, Esperado, Fallo, Leido, Punteros, Que, Ramas, Reclamados, Version,
    dataset_de_ubicacion, partes,
};
use crate::json::Json;

#[derive(Default)]
struct Estado {
    hoy: BTreeMap<String, String>,
    historia: BTreeMap<String, Vec<String>>,
    escrituras: u64,
}

/// Se clona como un `Arc`: dos clones ven lo mismo, como dos conexiones a un almacén.
#[derive(Clone, Default)]
pub(crate) struct Memoria(Arc<Mutex<Estado>>);

fn nodo(t: &str) -> crate::parse::Node {
    crate::parse::parse(t).expect("un puntero guardado se lee")
}

fn leido(t: &str) -> Leido {
    Leido {
        puntero: nodo(t),
        texto: t.to_string(),
        de: De::Aqui,
    }
}

fn ml_de(t: &str) -> Option<String> {
    nodo(t)
        .get("metadata_location")
        .and_then(|(_, v)| v.as_str().map(String::from))
}

/// El `dataset` sale de la `metadata_location`, como en cualquier puerta.
fn con_su_dataset(mut m: BTreeMap<String, Json>) -> String {
    if let Some(d) = match m.get("metadata_location") {
        Some(Json::Str(ml)) => dataset_de_ubicacion(ml),
        _ => None,
    } {
        m.insert("dataset".into(), Json::s(d));
    }
    Json::Obj(m).pretty() + "\n"
}

impl Punteros for Memoria {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo> {
        if partes(nombre).is_none() {
            return Err(Fallo::Nombre(nombre.into()));
        }
        Ok(self.0.lock().unwrap().hoy.get(nombre).map(|t| leido(t)))
    }

    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        let e = self.0.lock().unwrap();
        Ok(e.hoy.iter().map(|(k, t)| (k.clone(), leido(t))).collect())
    }

    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo> {
        let e = self.0.lock().unwrap();
        let vs = e
            .historia
            .get(nombre)
            .map(Vec::as_slice)
            .unwrap_or_default();
        Ok(vs
            .iter()
            .enumerate()
            .rev()
            .take(hasta)
            .map(|(i, t)| Version {
                id: format!("v{i}"),
                cuando: "2026-11-05T00:00:00Z".into(),
                quien: Some("quien".into()),
                puntero: nodo(t),
                texto: t.clone(),
            })
            .collect())
    }

    fn escribir(&self, cambios: &[Cambio], _: &Contexto) -> Result<(), Fallo> {
        let mut e = self.0.lock().unwrap();
        // Todos o ninguno: primero se comprueba todo.
        for c in cambios {
            if partes(&c.nombre).is_none() {
                return Err(Fallo::Nombre(c.nombre.clone()));
            }
            let previo = e.hoy.get(&c.nombre);
            let ml = previo.and_then(|t| ml_de(t));
            let cumple = match &c.esperado {
                Esperado::Cualquiera => true,
                Esperado::Nada => previo.is_none(),
                Esperado::Ml(m) => ml.as_deref() == Some(m.as_str()),
            };
            if !cumple {
                return Err(Fallo::Conflicto {
                    nombre: c.nombre.clone(),
                    esperado: c.esperado.clone(),
                    hay: ml,
                });
            }
        }
        for c in cambios {
            let escrito = match &c.que {
                Que::Retirar => None,
                Que::Sustituir(Json::Obj(m)) => Some(con_su_dataset(m.clone())),
                Que::Sustituir(_) => {
                    return Err(Fallo::Almacen(format!(
                        "el puntero de `{}` no es un objeto",
                        c.nombre
                    )));
                }
                Que::Poner { campos, quitar } => {
                    let mut m = match e.hoy.get(&c.nombre).map(|t| Json::de_node_fiel(&nodo(t))) {
                        Some(Json::Obj(m)) => m,
                        _ => BTreeMap::new(),
                    };
                    for k in quitar {
                        m.remove(k);
                    }
                    m.extend(campos.iter().cloned());
                    Some(con_su_dataset(m))
                }
            };
            match escrito {
                Some(t) => {
                    e.historia
                        .entry(c.nombre.clone())
                        .or_default()
                        .push(t.clone());
                    e.hoy.insert(c.nombre.clone(), t);
                }
                None => {
                    e.hoy.remove(&c.nombre);
                }
            }
        }
        e.escrituras += 1;
        Ok(())
    }

    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        let e = self.0.lock().unwrap();
        let mut r = Reclamados::default();
        for t in e.hoy.values() {
            let n = nodo(t);
            let c = |k: &str| n.get(k).and_then(|(_, v)| v.as_str().map(String::from));
            let (Some(d), Some(m)) = (c("dataset"), c("metadata_location")) else {
                continue;
            };
            r.datasets.insert(d.clone());
            r.ubicaciones
                .entry(d.clone())
                .or_default()
                .insert(m.clone());
            if c("kind").as_deref() == Some("MediaCollection") {
                r.colecciones.insert((d, m));
            }
        }
        Ok(r)
    }

    fn version(&self) -> Result<String, Fallo> {
        Ok(self.0.lock().unwrap().escrituras.to_string())
    }
}

/// Las ramas no hacen nada aquí: contestan.
impl Ramas for Memoria {
    fn crear(&self, _: &str, _: &str) -> Result<(), Fallo> {
        Ok(())
    }
    fn borrar(&self, _: &str) -> Result<(), Fallo> {
        Ok(())
    }
    fn traer(&self, _: &str) -> Result<(), Fallo> {
        Ok(())
    }
}
