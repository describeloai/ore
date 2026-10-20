//! **El banco de MongoDB**: la base `kit` en un MongoDB de pruebas (un
//! `mongo:7` en local o en el CI, o un Atlas de desarrollo con permiso de
//! escritura).
//!
//! `MONGO_KIT_URL` es la de un usuario que **escribe**: con ella se carga la
//! semilla y se pregunta por las operaciones y las sesiones. El conector lee
//! con la misma URL: un `mongo:7` sin `--auth` no tiene roles, así que
//! `url_alternativa` y `url_mala` son `None` y esos casos salen «no aplica».
//!
//! **Lo que la semilla pierde en Mongo, y es lo que el kit mide:** BSON no
//! tiene fecha sin hora ni hora local (solo `Date`, un instante), su `Date` es
//! de milisegundos (la semilla trae microsegundos) y `Decimal128` no lleva
//! precisión (la semilla dice `Decimal<12, 2>`). Se carga como lo cargaría
//! quien tiene esos datos en Mongo —`Date` y `Decimal128`— y el kit dice qué
//! sale distinto.
//!
//! Detrás de la feature `mongodb`: el cliente oficial son ~220 crates y no es
//! cosa de quien solo prueba Postgres.

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use mongodb::bson::{Bson, DateTime, Decimal128, Document, doc};
use mongodb::sync::Client;
use ore_core::json::Json;
use std::collections::BTreeMap;
use std::str::FromStr;

const BASE: &str = "kit";
/// Lo que duerme la vista lenta por documento.
/// Un `$function` de JS solo mira `maxTimeMS` **entre documentos** (medido: una
/// función que duerme 20 s no se corta hasta que termina), así que la vista es de
/// 10⁶ documentos que duermen poco cada uno y no devuelve ninguno.
const SUEÑO: &str = "sleep(20)";

pub struct Mongo {
    url: String,
    cliente: Client,
}

impl Mongo {
    pub fn new(url: &str) -> Result<Mongo, String> {
        let cliente = Client::with_uri_str(url)
            .map_err(|e| format!("no se llega al MongoDB de pruebas: {e}"))?;
        Ok(Mongo {
            url: url.to_string(),
            cliente,
        })
    }

    fn admin(&self, c: Document) -> Result<Document, String> {
        self.cliente
            .database("admin")
            .run_command(c)
            .run()
            .map_err(|e| e.to_string())
    }

    /// Las operaciones en curso sobre la vista lenta. En una vista, `currentOp`
    /// enseña el comando **de fuera** (`aggregate: "lenta"`), no el `$function` de
    /// dentro: se reconocen por el nombre de la vista, no por su cuerpo.
    fn operaciones(&self) -> Vec<Bson> {
        let Ok(r) = self.admin(doc! {"currentOp": 1, "active": true}) else {
            return Vec::new();
        };
        r.get_array("inprog")
            .map(|a| {
                a.iter()
                    .filter_map(Bson::as_document)
                    .filter(|op| {
                        op.get_document("command").is_ok_and(|c| {
                            ["aggregate", "find"]
                                .iter()
                                .any(|k| c.get_str(k) == Ok("lenta"))
                        })
                    })
                    .filter_map(|op| op.get("opid").cloned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Un valor de la semilla, en texto canónico, como lo guardaría Mongo.
fn bson_de(tipo: &str, v: &str) -> Result<Bson, String> {
    let malo = |e: &dyn std::fmt::Display| format!("`{v}` no es `{tipo}`: {e}");
    Ok(match tipo {
        "Integer" => Bson::Int64(v.parse().map_err(|e| malo(&e))?),
        "Float" => Bson::Double(v.parse().map_err(|e| malo(&e))?),
        "Boolean" => Bson::Boolean(v == "true"),
        "String" => Bson::String(v.to_string()),
        "Decimal<12, 2>" => Bson::Decimal128(Decimal128::from_str(v).map_err(|e| malo(&e))?),
        // Mongo solo tiene el instante: la fecha es su medianoche en UTC y la
        // hora local, el mismo instante en UTC.
        "Date" => Bson::DateTime(
            DateTime::parse_rfc3339_str(format!("{v}T00:00:00Z")).map_err(|e| malo(&e))?,
        ),
        "DateTime" => {
            Bson::DateTime(DateTime::parse_rfc3339_str(format!("{v}Z")).map_err(|e| malo(&e))?)
        }
        "DateTimeTz" => Bson::DateTime(DateTime::parse_rfc3339_str(v).map_err(|e| malo(&e))?),
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

impl Banco for Mongo {
    fn familia(&self) -> &'static str {
        "mongodb"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let bd = self.cliente.database(BASE);
        let fallo = |e: mongodb::error::Error| format!("la semilla no se carga: {e}");
        // `grande` sólo se rehace si no está entera: son 10⁶ documentos.
        let grande_bien = bd
            .collection::<Document>("grande")
            .estimated_document_count()
            .run()
            .unwrap_or(0)
            == GRANDE;
        for nombre in ["tipos", "vacia", "lenta"] {
            bd.collection::<Document>(nombre)
                .drop()
                .run()
                .map_err(fallo)?;
        }
        let tipos = bd.collection::<Document>("tipos");
        let docs: Vec<Document> = FILAS
            .iter()
            .map(|fila| {
                let mut d = Document::new();
                for (c, v) in TIPOS.iter().zip(fila.iter()) {
                    let valor = match v {
                        Some(v) => bson_de(c.tipo, v)?,
                        None => Bson::Null,
                    };
                    d.insert(if c.nombre == "id" { "_id" } else { c.nombre }, valor);
                }
                // El kit pide `id`; en Mongo la clave es `_id`, y se copia.
                let id = d.get("_id").cloned().unwrap_or(Bson::Null);
                d.insert("id", id);
                Ok(d)
            })
            .collect::<Result<_, String>>()?;
        tipos.insert_many(docs).run().map_err(fallo)?;
        bd.create_collection("vacia").run().map_err(fallo)?;
        if !grande_bien {
            let grande = bd.collection::<Document>("grande");
            grande.drop().run().map_err(fallo)?;
            let mut lote = Vec::with_capacity(10_000);
            for g in 1..=GRANDE as i64 {
                lote.push(doc! {
                    "_id": g, "id": g, "grupo": g % 100,
                    "importe": Decimal128::from_str(&format!("{}.{:02}", g / 100, g % 100))
                        .map_err(|e| e.to_string())?,
                    "nota": format!("fila-{g}"),
                });
                if lote.len() == 10_000 {
                    grande
                        .insert_many(std::mem::take(&mut lote))
                        .run()
                        .map_err(fallo)?;
                }
            }
        }
        // Lo que tarda en dar su primera fila: una vista que duerme en cada documento.
        bd.run_command(doc! {
            "create": "lenta", "viewOn": "grande",
            "pipeline": [{"$match": {"$expr": {"$function": {
                "body": format!("function() {{ {SUEÑO}; return false }}"),
                "args": [], "lang": "js",
            }}}}],
        })
        .run()
        .map_err(fallo)?;
        let n = bd
            .collection::<Document>("tipos")
            .count_documents(doc! {})
            .run()
            .map_err(fallo)?;
        if n != semilla::FILAS.len() as u64 {
            return Err(format!("`kit.tipos` tiene {n} documentos"));
        }
        Ok(())
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    fn url_mala(&self) -> Option<String> {
        None
    }

    /// Lo que el motor manda y el dato no lleva: los tipos que la tabla declara
    /// (`tipos`, ADR 0069 M3). Sin ellos un `Date` es un instante de BSON.
    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        peticion.insert(
            "tipos".into(),
            Json::Arr(
                tabla
                    .columnas()
                    .iter()
                    .map(|c| Json::Arr(vec![Json::s(c.nombre), Json::s(c.tipo)]))
                    .collect(),
            ),
        );
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{BASE}.{}", tabla.nombre())
    }

    fn milisegundos(&self) -> bool {
        true
    }

    fn lenta(&self) -> Option<String> {
        Some(format!("{BASE}.lenta"))
    }

    fn consultas_vivas(&mut self, _objeto: &str) -> Option<u64> {
        Some(self.operaciones().len() as u64)
    }

    /// Los **clientes** del conector con conexiones abiertas. Un cliente de
    /// Mongo abre varias (monitorización y pool), así que se cuentan por el
    /// `appName` de cada uno, que el conector pone distinto a cada sesión.
    fn sesiones(&mut self) -> Option<u64> {
        let cur = self
            .cliente
            .database("admin")
            .aggregate(vec![
                doc! {"$currentOp": {"allUsers": true, "idleConnections": true}},
                doc! {"$match": {"appName": {"$regex": "^ore-read-mongodb"}}},
                doc! {"$group": {"_id": "$appName"}},
            ])
            .run()
            .ok()?;
        Some(cur.filter_map(Result::ok).count() as u64)
    }

    fn limpiar(&mut self) {
        for id in self.operaciones() {
            let _ = self.admin(doc! {"killOp": 1, "op": id});
        }
    }
}
