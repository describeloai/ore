//! **Las rutas** (`docs/federation.md` §3) y el registro de las lecturas.
//!
//! | | ruta | qué |
//! |---|---|---|
//! | leer | `POST /v1/read` | una lectura; responde un flujo Arrow con *trailers* |
//! | cancelar | `DELETE /v1/read/{id}` | corta una lectura en curso, también en el origen |
//! | estado | `GET /v1/read/{id}` | cómo va o cómo terminó (las últimas 1000) |
//! | conectores | `GET /v1/connectors` | lo que declara cada familia |
//! | orígenes | `GET /v1/origins` | por origen: activas, en cola, latencias, cortes, errores |
//! | salud | `GET /v1/health` | viva y con conectores |
//!
//! ⛔ Sólo la llama `ore-serve` (lo cierra la NetworkPolicy de la malla): aquí
//!   no se decide quién puede leer qué. Eso es del coordinador (F4).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use ore_core::json::Json;
use ore_driver::{Codigo, Fallo};
use ore_entrada::http::{Bytes, Finales, Peticion, Salida};

use crate::capacidades::Capacidades;
use crate::cotas::Presupuesto;
use crate::fondo::{Fondo, Saturado};
use crate::lectura::{self, Control, Final, Inicio};

/// Las lecturas terminadas que se recuerdan para `GET /v1/read/{id}`.
const RECUERDO: usize = 1000;

/// El tipo de un flujo Arrow IPC.
pub const ARROW: &str = "application/vnd.apache.arrow.stream";

/// Los *trailers* de una lectura.
const FINALES: [&str; 5] = [
    "ore-estado",
    "ore-motivo",
    "ore-filas",
    "ore-bytes",
    "ore-ms",
];

pub struct Pasarela {
    pub fondo: Arc<Fondo>,
    pub conectores: BTreeMap<String, Capacidades>,
    en_curso: Mutex<HashMap<String, Arc<Control>>>,
    terminadas: Mutex<VecDeque<(String, Final)>>,
}

impl Pasarela {
    /// Pregunta a cada familia sus capacidades. Una que no contesta no entra
    /// (y se dice); sin ninguna, la pasarela no está sana.
    pub fn new(fondo: Arc<Fondo>, tipos: &[String]) -> Arc<Pasarela> {
        let mut conectores = BTreeMap::new();
        for t in tipos {
            match Capacidades::preguntar(&fondo.programa(t)) {
                Ok(c) => {
                    eprintln!(
                        "ore-federation: conector `{t}` · {} operadores",
                        c.operadores.len()
                    );
                    conectores.insert(t.clone(), c);
                }
                Err(e) => eprintln!("ore-federation: ⚠ sin conector `{t}`: {e}"),
            }
        }
        Arc::new(Pasarela {
            fondo,
            conectores,
            en_curso: Mutex::new(HashMap::new()),
            terminadas: Mutex::new(VecDeque::new()),
        })
    }

    pub fn atender(self: &Arc<Self>, p: &Peticion) -> Salida {
        let s = p.segmentos();
        match (p.metodo.as_str(), s.as_slice()) {
            ("GET", ["v1", "health"]) => self.salud(),
            ("GET", ["v1", "connectors"]) => json(200, self.conectores_json(), vec![]),
            ("GET", ["v1", "origins"]) => json(200, self.origenes(), vec![]),
            ("POST", ["v1", "read"]) => self.leer(&p.cuerpo),
            ("GET", ["v1", "read", id]) => self.estado(id),
            ("DELETE", ["v1", "read", id]) => self.cancelar(id),
            (_, ["v1", "read", ..]) | (_, ["v1", "health" | "connectors" | "origins"]) => {
                error(405, "operador", "método no admitido", false)
            }
            _ => error(404, "objeto", "no hay tal ruta", false),
        }
    }

    fn salud(&self) -> Salida {
        let tipos: Vec<Json> = self
            .conectores
            .keys()
            .map(|k| Json::s(k.as_str()))
            .collect();
        if tipos.is_empty() {
            return json(
                503,
                Json::obj([
                    ("estado", Json::s("sin-conectores")),
                    ("conectores", Json::Arr(tipos)),
                ]),
                vec![],
            );
        }
        json(
            200,
            Json::obj([("estado", Json::s("ok")), ("conectores", Json::Arr(tipos))]),
            vec![],
        )
    }

    fn conectores_json(&self) -> Json {
        Json::obj([(
            "conectores",
            Json::Obj(
                self.conectores
                    .iter()
                    .map(|(k, c)| (k.clone(), Json::Crudo(c.json.clone())))
                    .collect(),
            ),
        )])
    }

    fn origenes(&self) -> Json {
        let filas = self
            .fondo
            .estado()
            .into_iter()
            .map(|(origen, activas, en_cola, libres, m)| {
                let mut o = BTreeMap::new();
                o.insert("origen".to_string(), Json::s(origen));
                o.insert("activas".to_string(), Json::Int(activas as i64));
                o.insert("enCola".to_string(), Json::Int(en_cola as i64));
                o.insert("calientes".to_string(), Json::Int(libres as i64));
                o.insert("lecturas".to_string(), Json::Int(m.lecturas as i64));
                o.insert("completas".to_string(), Json::Int(m.completas as i64));
                o.insert("cortadas".to_string(), Json::Int(m.cortadas as i64));
                o.insert("errores".to_string(), Json::Int(m.errores as i64));
                o.insert("saturadas".to_string(), Json::Int(m.saturadas as i64));
                o.insert(
                    "procesosLanzados".to_string(),
                    Json::Int(m.procesos_lanzados as i64),
                );
                if let Some(v) = m.percentil(0.5) {
                    o.insert("p50Ms".to_string(), Json::Int(v as i64));
                }
                if let Some(v) = m.percentil(0.95) {
                    o.insert("p95Ms".to_string(), Json::Int(v as i64));
                }
                Json::Obj(o)
            })
            .collect();
        Json::obj([("origenes", Json::Arr(filas))])
    }

    fn estado(&self, id: &str) -> Salida {
        if let Some(c) = self.en_curso.lock().expect("en curso").get(id) {
            return json(
                200,
                Json::obj([
                    ("id", Json::s(id)),
                    ("estado", Json::s("en-curso")),
                    (
                        "filas",
                        Json::Int(c.filas.load(std::sync::atomic::Ordering::SeqCst) as i64),
                    ),
                    (
                        "bytes",
                        Json::Int(c.bytes.load(std::sync::atomic::Ordering::SeqCst) as i64),
                    ),
                    ("ms", Json::Int(c.empezo.elapsed().as_millis() as i64)),
                ]),
                vec![],
            );
        }
        let t = self.terminadas.lock().expect("terminadas");
        match t.iter().rev().find(|(i, _)| i == id) {
            Some((_, f)) => json(200, final_json(id, f), vec![]),
            None => error(404, "objeto", "no hay una lectura con ese id", false),
        }
    }

    fn cancelar(&self, id: &str) -> Salida {
        let c = self.en_curso.lock().expect("en curso").get(id).cloned();
        match c {
            Some(c) => {
                c.cortar("cancelada");
                json(
                    202,
                    Json::obj([("id", Json::s(id)), ("estado", Json::s("cancelando"))]),
                    vec![],
                )
            }
            None => error(
                404,
                "objeto",
                "no hay una lectura en curso con ese id",
                false,
            ),
        }
    }

    /// **`POST /v1/read`**.
    fn leer(self: &Arc<Self>, cuerpo: &str) -> Salida {
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(_) => return error(400, "operador", "el cuerpo no es JSON", false),
        };
        let cadena = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let Some(id) = cadena("id").filter(|v| nombre_valido(v, true)) else {
            return error(
                400,
                "operador",
                "falta `id`, o no es un identificador",
                false,
            );
        };
        let Some(origen) = cadena("origen").filter(|v| nombre_valido(v, false)) else {
            return error(400, "operador", "falta `origen`, o no es un nombre", false);
        };
        let Some(tipo) = cadena("tipo") else {
            return error(400, "operador", "falta `tipo`", false);
        };
        let Some(caps) = self.conectores.get(&tipo) else {
            return error(
                400,
                "operador",
                &format!("no hay conector de `{tipo}`"),
                false,
            );
        };
        let Some(url) = cadena("url").filter(|u| !u.is_empty()) else {
            return error(400, "operador", "falta `url`", false);
        };
        let Some((_, pet)) = n.get("peticion") else {
            return error(400, "operador", "falta `peticion`", false);
        };
        let presupuesto = match presupuesto(&n, self.fondo.cotas.presupuesto) {
            Ok(p) => p,
            Err(m) => return error(400, "operador", &m, false),
        };

        // La petición que va al conector: con su `id`, su `url`, en Arrow, y
        // con el presupuesto empujado al origen (`limit` y `timeoutMs`).
        let Json::Obj(mut m) = Json::de_node(pet) else {
            return error(400, "operador", "`peticion` no es un objeto", false);
        };
        let natural = |k: &str| {
            pet.get(k)
                .and_then(|(_, v)| v.as_str())
                .and_then(|v| v.parse::<u64>().ok())
        };
        m.insert("id".into(), Json::s(id.as_str()));
        m.insert("url".into(), Json::s(url.as_str()));
        m.insert("formato".into(), Json::s("arrow"));
        // `filas + 1`: así se distingue «había más» (se corta) de «eran justas».
        let tope = presupuesto.filas.saturating_add(1);
        if caps.limit {
            let limit = natural("limit").map_or(tope, |l| l.min(tope));
            m.insert("limit".into(), Json::Int(limit as i64));
        }
        let ms = natural("timeoutMs").map_or(presupuesto.ms, |t| t.min(presupuesto.ms));
        m.insert("timeoutMs".into(), Json::Int(ms as i64));
        let linea = Json::Obj(m).jcs();
        let p = match ore_driver::leer_peticion(&linea) {
            Ok(p) => p,
            Err(e) => return error(400, "operador", &ore_driver::tapar(&e, &url), false),
        };
        if let Err(f) = caps.admite(&p) {
            return fallo(400, &f, vec![]);
        }

        // Un id es una lectura: repetirlo es un error de quien llama.
        let control = Control::new(&id, &origen);
        {
            let mut c = self.en_curso.lock().expect("en curso");
            let repetida = c.contains_key(&id)
                || self
                    .terminadas
                    .lock()
                    .expect("terminadas")
                    .iter()
                    .any(|(i, _)| i == &id);
            if repetida {
                return error(409, "operador", "ya hubo una lectura con ese id", false);
            }
            c.insert(id.clone(), control.clone());
        }

        let plaza = match self.fondo.tomar(&origen, &tipo, &url) {
            Ok(Ok(p)) => p,
            Ok(Err(s)) => {
                let motivo = match s {
                    Saturado::ColaLlena => "la cola del origen está llena",
                    Saturado::EsperaAgotada => "se agotó la espera en la cola del origen",
                };
                self.terminar(
                    &id,
                    &Final {
                        estado: "error",
                        motivo: "saturado".into(),
                        filas: 0,
                        bytes: 0,
                        ms: control.empezo.elapsed().as_millis() as u64,
                    },
                );
                return json(
                    503,
                    Json::obj([
                        ("codigo", Json::s("saturado")),
                        ("mensaje", Json::s(motivo)),
                        ("reintentable", Json::Bool(true)),
                    ]),
                    vec![("retry-after".into(), "1".into())],
                );
            }
            Err(e) => {
                let f = Fallo::new(Codigo::Conexion, e);
                self.terminar(
                    &id,
                    &Final {
                        estado: "error",
                        motivo: format!("conexion: {}", f.mensaje),
                        filas: 0,
                        bytes: 0,
                        ms: 0,
                    },
                );
                return fallo(502, &f, vec![]);
            }
        };

        let yo = Arc::clone(self);
        let (id2, origen2, tipo2) = (id.clone(), origen.clone(), tipo.clone());
        let al_terminar = Box::new(move |f: &Final| {
            eprintln!(
                "lectura {id2} · {origen2} · {tipo2} · {}{} · {} filas · {} B · {} ms",
                f.estado,
                if f.motivo.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", f.motivo)
                },
                f.filas,
                f.bytes,
                f.ms
            );
            yo.terminar(&id2, f);
        });
        match lectura::empezar(
            &self.fondo,
            plaza,
            &linea,
            control,
            presupuesto,
            al_terminar,
        ) {
            Inicio::Error { http, fallo: f, .. } => fallo(http, &f, vec![]),
            Inicio::Flujo { lector, final_ } => Salida::Bytes(Bytes {
                codigo: 200,
                cabeceras: vec![
                    ("content-type".into(), ARROW.into()),
                    ("ore-lectura".into(), id),
                ],
                largo: None,
                lector,
                finales: Some(Finales {
                    nombres: FINALES.iter().map(|s| s.to_string()).collect(),
                    valores: Box::new(move || {
                        let f = final_.lock().expect("final").clone();
                        match f {
                            Some(f) => vec![
                                ("ore-estado".into(), f.estado.into()),
                                ("ore-motivo".into(), f.motivo),
                                ("ore-filas".into(), f.filas.to_string()),
                                ("ore-bytes".into(), f.bytes.to_string()),
                                ("ore-ms".into(), f.ms.to_string()),
                            ],
                            None => vec![("ore-estado".into(), "error".into())],
                        }
                    }),
                }),
            }),
        }
    }

    fn terminar(&self, id: &str, f: &Final) {
        self.en_curso.lock().expect("en curso").remove(id);
        let mut t = self.terminadas.lock().expect("terminadas");
        t.push_back((id.to_string(), f.clone()));
        while t.len() > RECUERDO {
            t.pop_front();
        }
    }
}

/// El presupuesto de la petición, o el de por defecto. Cada campo, un entero
/// positivo.
fn presupuesto(n: &ore_core::parse::Node, defecto: Presupuesto) -> Result<Presupuesto, String> {
    let Some((_, p)) = n.get("presupuesto") else {
        return Ok(defecto);
    };
    let campo = |k: &str, d: u64| -> Result<u64, String> {
        match p.get(k).and_then(|(_, v)| v.as_str()) {
            None => Ok(d),
            Some(v) => v
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or_else(|| format!("`presupuesto.{k}` no es un entero positivo")),
        }
    };
    Ok(Presupuesto {
        filas: campo("filas", defecto.filas)?,
        bytes: campo("bytes", defecto.bytes)?,
        ms: campo("ms", defecto.ms)?,
    })
}

/// Un id (`a-z A-Z 0-9 _ - . :`) o un nombre de origen (sin `:`), hasta 128.
fn nombre_valido(v: &str, es_id: bool) -> bool {
    !v.is_empty()
        && v.len() <= 128
        && v.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') || (es_id && c == ':')
        })
}

pub fn final_json(id: &str, f: &Final) -> Json {
    Json::obj([
        ("id", Json::s(id)),
        ("estado", Json::s(f.estado)),
        ("motivo", Json::s(f.motivo.as_str())),
        ("filas", Json::Int(f.filas as i64)),
        ("bytes", Json::Int(f.bytes as i64)),
        ("ms", Json::Int(f.ms as i64)),
    ])
}

/// Una respuesta JSON con su largo y las cabeceras que haga falta.
fn json(codigo: u16, cuerpo: Json, mut cabeceras: Vec<(String, String)>) -> Salida {
    let b = format!("{}\n", cuerpo.jcs()).into_bytes();
    cabeceras.push(("content-type".into(), "application/json".into()));
    Salida::Bytes(Bytes {
        codigo,
        cabeceras,
        largo: Some(b.len() as u64),
        lector: Box::new(std::io::Cursor::new(b)),
        finales: None,
    })
}

/// Un fallo antes del primer byte: `{"codigo", "mensaje", "reintentable"}`.
fn fallo(codigo: u16, f: &Fallo, cabeceras: Vec<(String, String)>) -> Salida {
    json(codigo, Json::Obj(f.campos()), cabeceras)
}

fn error(codigo: u16, cod: &str, mensaje: &str, reintentable: bool) -> Salida {
    json(
        codigo,
        Json::obj([
            ("codigo", Json::s(cod)),
            ("mensaje", Json::s(mensaje)),
            ("reintentable", Json::Bool(reintentable)),
        ]),
        vec![],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_nombres_y_el_presupuesto_se_validan() {
        assert!(nombre_valido("8f1c-uuid:2", true));
        assert!(!nombre_valido("a:b", false));
        assert!(!nombre_valido("a b", true));
        assert!(!nombre_valido("", true));
        let d = Presupuesto {
            filas: 10,
            bytes: 20,
            ms: 30,
        };
        let n = ore_core::parse::parse(r#"{"presupuesto":{"filas":5}}"#).unwrap();
        assert_eq!(
            presupuesto(&n, d).unwrap(),
            Presupuesto {
                filas: 5,
                bytes: 20,
                ms: 30
            }
        );
        let n = ore_core::parse::parse(r#"{"presupuesto":{"ms":0}}"#).unwrap();
        assert!(presupuesto(&n, d).is_err());
        let n = ore_core::parse::parse(r#"{}"#).unwrap();
        assert_eq!(presupuesto(&n, d).unwrap(), d);
    }
}
