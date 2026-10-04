//! **El banco de BigQuery**: el dataset `ore_kit` (o `BQ_KIT_DATASET`) del
//! proyecto `BQ_KIT_PROYECTO`, con la semilla cargada por DDL.
//!
//! Dos modos, como la cinta del conector (`ore-read-bigquery`, `rest::Cinta`):
//!
//! - **De verdad** (sin `ORE_BQ_CINTA_MODO=reproducir`): crea el dataset si no
//!   está y carga `tipos`, `vacia` y `grande` con `CREATE OR REPLACE TABLE … AS
//!   SELECT`. `grande` se genera dentro de BigQuery (`GENERATE_ARRAY`), así
//!   que cargarla no procesa bytes; leerla entera son ~40 MB. La credencial es
//!   la del conector (`ore-gcp`: el metadata server, o `ORE_GCP_TOKEN`).
//! - **Reproduciendo** una cinta grabada: no carga nada —la cinta ya trae lo
//!   que BigQuery contestó— y no habla con nadie. Es lo que corre el CI.
//!
//! BigQuery no tiene una consulta que tarde lo que se le pida ni sesiones que
//! contar, y la credencial no viaja en la URL: los casos 9, 10 y la credencial
//! mala del 12 no aplican.

use super::Banco;
use crate::semilla::{FILAS, GRANDE, TIPOS, Tabla};
use serde_json::{Value, json};

pub struct BigQuery {
    proyecto: String,
    dataset: String,
    ubicacion: String,
    /// `false` si se reproduce una cinta.
    de_verdad: bool,
}

const API: &str = "https://bigquery.googleapis.com/bigquery/v2";

/// El tipo de BigQuery de cada tipo de OOS de la semilla: el de la columna, y
/// el del `CAST` (que no admite parámetros).
fn tipo(oos: &str) -> (&'static str, &'static str) {
    match oos {
        "Integer" => ("INT64", "INT64"),
        "Decimal<12, 2>" => ("NUMERIC(12, 2)", "NUMERIC"),
        "Float" => ("FLOAT64", "FLOAT64"),
        "String" => ("STRING", "STRING"),
        "Date" => ("DATE", "DATE"),
        "DateTime" => ("DATETIME", "DATETIME"),
        "DateTimeTz" => ("TIMESTAMP", "TIMESTAMP"),
        "Boolean" => ("BOOL", "BOOL"),
        otro => panic!("la semilla no usa `{otro}`"),
    }
}

/// Un valor de la semilla como expresión de GoogleSQL: siempre un `CAST` de
/// una cadena, que es la forma que admite el mínimo de un `INT64` y un nulo
/// con tipo.
fn literal(v: Option<&str>, oos: &str) -> String {
    let base = tipo(oos).1;
    let Some(t) = v else {
        return format!("CAST(NULL AS {base})");
    };
    let t = match oos {
        "DateTime" => t.replacen('T', " ", 1),
        "DateTimeTz" => t.replacen('T', " ", 1).replace('Z', "+00"),
        _ => t.to_string(),
    };
    let t = t
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n");
    format!("CAST('{t}' AS {base})")
}

impl BigQuery {
    pub fn new(proyecto: &str, dataset: &str, ubicacion: &str, de_verdad: bool) -> BigQuery {
        BigQuery {
            proyecto: proyecto.to_string(),
            dataset: dataset.to_string(),
            ubicacion: ubicacion.to_string(),
            de_verdad,
        }
    }

    fn pedir(&self, metodo: &str, ruta: &str, cuerpo: Option<&Value>) -> Result<Value, String> {
        let token = ore_gcp::Credencial::del_entorno().token()?;
        let agente = ore_gcp::cliente()?;
        let r = agente
            .request(metodo, &format!("{API}/{ruta}"))
            .set("authorization", &format!("Bearer {token}"));
        let r = match cuerpo {
            Some(c) => r
                .set("content-type", "application/json")
                .send_string(&c.to_string()),
            None => r.call(),
        };
        match r {
            Ok(ok) => serde_json::from_reader(ok.into_reader()).map_err(|e| e.to_string()),
            Err(ureq::Error::Status(c, r)) => Err(format!(
                "BigQuery contestó {c}: {}",
                r.into_string().unwrap_or_default()
            )),
            Err(e) => Err(format!("no se llega a BigQuery: {e}")),
        }
    }

    /// Una sentencia, hasta que termina.
    fn ejecutar(&self, sql: &str) -> Result<(), String> {
        let p = &self.proyecto;
        let mut r = self.pedir(
            "POST",
            &format!("projects/{p}/queries"),
            Some(&json!({
                "query": sql, "useLegacySql": false, "timeoutMs": 60000,
                "location": self.ubicacion,
            })),
        )?;
        while r["jobComplete"] != Value::Bool(true) {
            let id = r["jobReference"]["jobId"]
                .as_str()
                .ok_or("una sentencia sin job")?
                .to_string();
            r = self.pedir(
                "GET",
                &format!(
                    "projects/{p}/queries/{id}?timeoutMs=60000&location={}",
                    self.ubicacion
                ),
                None,
            )?;
        }
        match r["errors"].as_array().and_then(|e| e.first()) {
            Some(e) => Err(format!("la semilla no se carga: {e}")),
            None => Ok(()),
        }
    }

    fn tabla(&self, t: Tabla) -> String {
        format!("`{}.{}.{}`", self.proyecto, self.dataset, t.nombre())
    }
}

impl Banco for BigQuery {
    fn familia(&self) -> &'static str {
        "bigquery"
    }

    fn cargar(&mut self) -> Result<(), String> {
        if !self.de_verdad {
            return Ok(());
        }
        match self.pedir(
            "POST",
            &format!("projects/{}/datasets", self.proyecto),
            Some(&json!({
                "datasetReference": {"projectId": self.proyecto, "datasetId": self.dataset},
                "location": self.ubicacion,
            })),
        ) {
            Ok(_) => {}
            Err(e) if e.contains("contestó 409") => {}
            Err(e) => return Err(format!("el dataset no se crea: {e}")),
        }
        let columnas = TIPOS
            .iter()
            .map(|c| format!("{} {}", c.nombre, tipo(c.tipo).0))
            .collect::<Vec<_>>()
            .join(", ");
        let filas = FILAS
            .iter()
            .map(|f| {
                let campos = f
                    .iter()
                    .zip(TIPOS)
                    .map(|(v, c)| format!("{} AS {}", literal(*v, c.tipo), c.nombre))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("STRUCT({campos})")
            })
            .collect::<Vec<_>>()
            .join(",\n  ");
        self.ejecutar(&format!(
            "CREATE OR REPLACE TABLE {} ({columnas}) AS SELECT * FROM UNNEST([\n  {filas}\n])",
            self.tabla(Tabla::Tipos)
        ))?;
        self.ejecutar(&format!(
            "CREATE OR REPLACE TABLE {} ({columnas})",
            self.tabla(Tabla::Vacia)
        ))?;
        self.ejecutar(&format!(
            "CREATE OR REPLACE TABLE {} (id INT64, grupo INT64, importe NUMERIC(12, 2), \
             nota STRING) AS SELECT id, MOD(id, 100) AS grupo, \
             CAST(id AS NUMERIC) / 100 AS importe, CONCAT('fila-', CAST(id AS STRING)) AS nota \
             FROM UNNEST(GENERATE_ARRAY(1, {GRANDE})) AS id",
            self.tabla(Tabla::Grande)
        ))
    }

    fn url(&self) -> String {
        format!("bigquery://{}/{}", self.proyecto, self.dataset)
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// La credencial de BigQuery no va en la URL: es la de la cuenta que
    /// corre. Una URL «mala» no existe.
    fn url_mala(&self) -> Option<String> {
        None
    }

    fn objeto(&self, t: Tabla) -> String {
        format!("{}.{}", self.dataset, t.nombre())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_valor_de_la_semilla_es_un_cast_seguro() {
        assert_eq!(literal(None, "Integer"), "CAST(NULL AS INT64)");
        assert_eq!(
            literal(Some("-9223372036854775808"), "Integer"),
            "CAST('-9223372036854775808' AS INT64)"
        );
        assert_eq!(
            literal(Some("o'neil"), "String"),
            r"CAST('o\'neil' AS STRING)"
        );
        assert_eq!(
            literal(Some("línea\nnueva"), "String"),
            r"CAST('línea\nnueva' AS STRING)"
        );
        assert_eq!(
            literal(Some("2000-02-29T12:30:00.5Z"), "DateTimeTz"),
            "CAST('2000-02-29 12:30:00.5+00' AS TIMESTAMP)"
        );
        assert_eq!(
            literal(Some("2099-12-31T23:59:59.999999"), "DateTime"),
            "CAST('2099-12-31 23:59:59.999999' AS DATETIME)"
        );
    }
}
