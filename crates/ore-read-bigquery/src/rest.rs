//! **La API REST de BigQuery**, lo justo para leer: una consulta, su sondeo y
//! sus páginas; el esquema de una tabla; la lista de datasets.
//!
//! # Lo que se pide siempre, y por qué
//!
//! - `formatOptions.useInt64Timestamp`: sin él un TIMESTAMP llega como un float
//!   en notación científica (`2.534023008E11` para el último microsegundo del
//!   año 9999) y los microsegundos se pierden. Medido el 2026-09-26.
//! - Parámetros con nombre y tipados en el cuerpo: los valores ya no viajan por
//!   `argv`, que es lo que `bq --parameter` obligaba a pagar.
//!
//! # La truncación, otra vez
//!
//! El CLI cortaba en `--max_rows` sin decirlo y el driver pedía una fila de más
//! para notarlo. Por REST no hay tope que alcanzar —se pagina hasta el final y
//! cada página se entrega según llega—, pero la pregunta sigue: **¿llegó todo?**
//! La respuesta la da el propio servidor en `totalRows`, y al terminar se
//! compara con lo contado. Si no coincide, se niega.
//!
//! # El transporte es un rasgo
//!
//! Para que el sondeo y la paginación se prueben contra respuestas grabadas de
//! verdad (`tests/rest/`) sin red: [`Transporte`] lo implementa [`Http`] y, en
//! las pruebas, una lista de respuestas.
use serde_json::{Value, json};

pub const API: &str = "https://bigquery.googleapis.com/bigquery/v2";

/// Cuánto espera el servidor antes de contestar `jobComplete: false`. Diez
/// segundos es el valor por defecto de la API; se escribe para que se vea.
const ESPERA_MS: u64 = 10_000;

pub trait Transporte {
    /// `POST {API}/{ruta}` con un cuerpo JSON.
    fn post(&self, ruta: &str, cuerpo: &Value) -> Result<Value, String>;
    /// `GET {API}/{ruta}?{consulta}`.
    fn get(&self, ruta: &str, consulta: &[(&str, String)]) -> Result<Value, String>;
}

/// El de verdad: HTTPS con el token de la cuenta que corre.
pub struct Http {
    credencial: ore_gcp::Credencial,
    agente: ureq::Agent,
}

impl Http {
    pub fn del_entorno() -> Result<Http, String> {
        Ok(Http {
            credencial: ore_gcp::Credencial::del_entorno(),
            agente: ore_gcp::cliente()?,
        })
    }

    fn enviar(&self, r: ureq::Request, cuerpo: Option<&Value>) -> Result<Value, String> {
        let r = r
            .set(
                "authorization",
                &format!("Bearer {}", self.credencial.token()?),
            )
            .set("user-agent", "ore-read-bigquery/0.2")
            .timeout(std::time::Duration::from_secs(300));
        let respuesta = match cuerpo {
            Some(c) => r
                .set("content-type", "application/json")
                .send_string(&c.to_string()),
            None => r.call(),
        };
        match respuesta {
            Ok(ok) => serde_json::from_reader(ok.into_reader())
                .map_err(|e| format!("lo que devolvió BigQuery no es JSON: {e}")),
            // El mensaje de Google, literal: dice qué falta (un permiso, una
            // tabla, una coma) y resumirlo lo esconde.
            Err(ureq::Error::Status(codigo, r)) => {
                let texto = r.into_string().unwrap_or_default();
                let mensaje = serde_json::from_str::<Value>(&texto)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(String::from))
                    .unwrap_or(texto);
                Err(format!("BigQuery contestó {codigo}: {mensaje}"))
            }
            Err(e) => Err(format!("no se pudo hablar con BigQuery: {e}")),
        }
    }
}

impl Transporte for Http {
    fn post(&self, ruta: &str, cuerpo: &Value) -> Result<Value, String> {
        self.enviar(self.agente.post(&format!("{API}/{ruta}")), Some(cuerpo))
    }
    fn get(&self, ruta: &str, consulta: &[(&str, String)]) -> Result<Value, String> {
        let mut r = self.agente.get(&format!("{API}/{ruta}"));
        for (k, v) in consulta {
            r = r.query(k, v);
        }
        self.enviar(r, None)
    }
}

/// Una consulta: el texto y sus parámetros en la forma de `ore-sql`
/// (`nombre:TIPO:valor`).
pub struct Consulta<'a> {
    pub texto: &'a str,
    pub parametros: &'a [String],
    /// Sin crear un job si el servidor puede evitarlo (`JOB_CREATION_OPTIONAL`):
    /// ~320 ms frente a ~500 medidos. Solo para lo que cabe en una respuesta.
    pub sin_job: bool,
}

/// `nombre:TIPO:valor` → el parámetro de la API. El valor puede llevar `:`
/// (un instante), así que se parte dos veces y no más.
pub fn parametro(p: &str) -> Result<Value, String> {
    let mut partes = p.splitn(3, ':');
    match (partes.next(), partes.next(), partes.next()) {
        (Some(n), Some(t), Some(v)) if !n.is_empty() && !t.is_empty() => Ok(json!({
            "name": n,
            "parameterType": {"type": t},
            "parameterValue": {"value": v},
        })),
        _ => Err(format!("`{p}` no tiene la forma `nombre:TIPO:valor`")),
    }
}

/// El cuerpo de `jobs.query`.
pub fn cuerpo(c: &Consulta) -> Result<Value, String> {
    let mut cuerpo = json!({
        "query": c.texto,
        "useLegacySql": false,
        "formatOptions": {"useInt64Timestamp": true},
        "timeoutMs": ESPERA_MS,
    });
    if !c.parametros.is_empty() {
        cuerpo["parameterMode"] = json!("NAMED");
        cuerpo["queryParameters"] = Value::Array(
            c.parametros
                .iter()
                .map(|p| parametro(p))
                .collect::<Result<_, _>>()?,
        );
    }
    if c.sin_job {
        cuerpo["jobCreationMode"] = json!("JOB_CREATION_OPTIONAL");
    }
    // Un techo de facturación, si el operador lo pone. No hay uno por defecto:
    // cualquier número sería inventado, y uno bajo haría fallar una copia
    // legítima con un mensaje de dinero.
    if let Some(b) = std::env::var("ORE_BQ_TOPE_BYTES")
        .ok()
        .filter(|s| !s.is_empty())
    {
        cuerpo["maximumBytesBilled"] = json!(b);
    }
    Ok(cuerpo)
}

/// Ejecuta una consulta y entrega sus filas **página a página**, con el esquema
/// del resultado. Devuelve cuántas llegaron, que es exactamente `totalRows` o
/// un error.
pub fn consultar(
    t: &dyn Transporte,
    proyecto: &str,
    c: &Consulta,
    mut por_pagina: impl FnMut(&[Value], &[Value]) -> Result<(), String>,
) -> Result<u64, String> {
    let mut r = t.post(&format!("projects/{proyecto}/queries"), &cuerpo(c)?)?;
    let referencia = r.get("jobReference").cloned();
    let pedir = |extra: Vec<(&'static str, String)>| -> Result<Value, String> {
        let job = referencia.as_ref().ok_or(
            "BigQuery no terminó la consulta en el plazo y no creó un job al que volver a \
             preguntar (JOB_CREATION_OPTIONAL): no hay forma de seguir",
        )?;
        let id = job["jobId"].as_str().ok_or("`jobReference` sin `jobId`")?;
        let mut q = vec![
            ("timeoutMs", ESPERA_MS.to_string()),
            ("formatOptions.useInt64Timestamp", "true".to_string()),
        ];
        if let Some(l) = job["location"].as_str() {
            q.push(("location", l.to_string()));
        }
        q.extend(extra);
        t.get(&format!("projects/{proyecto}/queries/{id}"), &q)
    };

    // El sondeo: `jobComplete: false` no es un error, es «todavía no».
    while r["jobComplete"] != Value::Bool(true) {
        errores_del_job(&r)?;
        r = pedir(Vec::new())?;
    }
    errores_del_job(&r)?;

    let campos = r["schema"]["fields"]
        .as_array()
        .cloned()
        .ok_or("la respuesta terminada no trae `schema.fields`")?;
    let total: u64 = match &r["totalRows"] {
        Value::String(s) => s
            .parse()
            .map_err(|_| format!("`totalRows` no es un número: {s}"))?,
        Value::Null => 0,
        otro => return Err(format!("`totalRows` no es un número: {otro}")),
    };
    let mut llegaron: u64 = 0;
    loop {
        let filas = r["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        llegaron += filas.len() as u64;
        por_pagina(&campos, filas)?;
        match r["pageToken"].as_str() {
            Some(tok) => r = pedir(vec![("pageToken", tok.to_string())])?,
            None => break,
        }
    }
    if llegaron != total {
        return Err(format!(
            "BigQuery dijo {total} filas y llegaron {llegaron}. Una copia con menos filas de \
             las que hay responde, y sus números son de menos: se niega"
        ));
    }
    Ok(llegaron)
}

/// Un job puede terminar mal: `errors` o `status.errorResult`.
fn errores_del_job(r: &Value) -> Result<(), String> {
    let e = r["errors"]
        .as_array()
        .and_then(|a| a.first())
        .or_else(|| r["status"].get("errorResult"));
    match e {
        Some(e) => Err(format!(
            "la consulta falló en BigQuery: {}",
            e["message"].as_str().unwrap_or("(sin mensaje)")
        )),
        None => Ok(()),
    }
}

/// El esquema de una tabla, sin job y sin coste: `tables.get`.
pub fn esquema(
    t: &dyn Transporte,
    proyecto: &str,
    dataset: &str,
    tabla: &str,
) -> Result<Vec<Value>, String> {
    let r = t.get(
        &format!("projects/{proyecto}/datasets/{dataset}/tables/{tabla}"),
        &[],
    )?;
    r["schema"]["fields"]
        .as_array()
        .cloned()
        .ok_or_else(|| format!("`{dataset}.{tabla}` no trae esquema"))
}

/// Los datasets de un proyecto, todas las páginas.
pub fn datasets(t: &dyn Transporte, proyecto: &str) -> Result<Vec<String>, String> {
    let mut fuera = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut q = vec![("maxResults", "1000".to_string())];
        if let Some(tok) = &token {
            q.push(("pageToken", tok.clone()));
        }
        let r = t.get(&format!("projects/{proyecto}/datasets"), &q)?;
        for d in r["datasets"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(n) = d["datasetReference"]["datasetId"].as_str() {
                fuera.push(n.to_string());
            }
        }
        match r["nextPageToken"].as_str() {
            Some(t) => token = Some(t.to_string()),
            None => return Ok(fuera),
        }
    }
}

#[cfg(test)]
pub mod pruebas {
    use super::*;
    use std::cell::RefCell;

    /// Una respuesta grabada de `tests/rest/`.
    pub fn grabada(nombre: &str) -> Value {
        let ruta = format!("{}/tests/rest/{nombre}.json", env!("CARGO_MANIFEST_DIR"));
        let texto = std::fs::read_to_string(&ruta).unwrap_or_else(|e| panic!("{ruta}: {e}"));
        serde_json::from_str::<Value>(&texto).unwrap()["respuesta"].clone()
    }

    /// Un transporte que contesta, en orden, lo que se le dio, y apunta qué se
    /// le pidió.
    pub struct Guion {
        pub respuestas: RefCell<Vec<Result<Value, String>>>,
        pub pedidas: RefCell<Vec<String>>,
    }

    impl Guion {
        pub fn new(r: Vec<Result<Value, String>>) -> Guion {
            Guion {
                respuestas: RefCell::new(r.into_iter().rev().collect()),
                pedidas: RefCell::new(Vec::new()),
            }
        }
        fn siguiente(&self, que: String) -> Result<Value, String> {
            self.pedidas.borrow_mut().push(que);
            self.respuestas
                .borrow_mut()
                .pop()
                .expect("se pidió más de lo guionizado")
        }
    }

    impl Transporte for Guion {
        fn post(&self, ruta: &str, cuerpo: &Value) -> Result<Value, String> {
            self.siguiente(format!("POST {ruta} {cuerpo}"))
        }
        fn get(&self, ruta: &str, consulta: &[(&str, String)]) -> Result<Value, String> {
            let q: Vec<String> = consulta.iter().map(|(k, v)| format!("{k}={v}")).collect();
            self.siguiente(format!("GET {ruta}?{}", q.join("&")))
        }
    }

    fn sql(t: &str) -> Consulta<'_> {
        Consulta {
            texto: t,
            parametros: &[],
            sin_job: false,
        }
    }

    #[test]
    fn tres_paginas_grabadas_llegan_enteras_y_en_orden() {
        let g = Guion::new(vec![
            Ok(grabada("pagina-1")),
            Ok(grabada("pagina-2")),
            Ok(grabada("pagina-3")),
        ]);
        let mut ids = Vec::new();
        let n = consultar(&g, "p", &sql("SELECT id"), |_, filas| {
            ids.extend(
                filas
                    .iter()
                    .map(|f| f["f"][0]["v"].as_str().unwrap().to_string()),
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(n, 8);
        assert_eq!(ids.first().unwrap(), "ore-e2e-p1");
        assert_eq!(ids.last().unwrap(), "ore-e2e-p8");
        let pedidas = g.pedidas.borrow();
        assert!(pedidas[1].contains("pageToken="), "{}", pedidas[1]);
        assert!(pedidas[1].contains("location=EU"), "{}", pedidas[1]);
        assert!(
            pedidas[1].contains("formatOptions.useInt64Timestamp=true"),
            "{}",
            pedidas[1]
        );
    }

    /// La guarda de la truncación: si faltan páginas, `totalRows` no cuadra.
    #[test]
    fn si_falta_una_pagina_se_niega() {
        let mut ultima = grabada("pagina-2");
        ultima.as_object_mut().unwrap().remove("pageToken");
        let g = Guion::new(vec![Ok(grabada("pagina-1")), Ok(ultima)]);
        let e = consultar(&g, "p", &sql("SELECT id"), |_, _| Ok(())).unwrap_err();
        assert!(e.contains("dijo 8 filas y llegaron 6"), "{e}");
    }

    /// `jobComplete: false` (grabado de una consulta de ~25 s) se sondea por
    /// `getQueryResults` hasta que termina.
    #[test]
    fn una_consulta_larga_se_sondea() {
        let g = Guion::new(vec![
            Ok(grabada("larga-incompleta")),
            Ok(json!({"jobComplete": false})),
            Ok(grabada("pedidos-query")),
        ]);
        let n = consultar(&g, "p", &sql("SELECT larga"), |_, _| Ok(())).unwrap();
        assert_eq!(n, 8);
        assert_eq!(g.pedidas.borrow().len(), 3);
        assert!(g.pedidas.borrow()[1].starts_with("GET projects/p/queries/"));
    }

    #[test]
    fn el_error_del_servidor_llega_con_su_mensaje() {
        let g = Guion::new(vec![Err("BigQuery contestó 400: Syntax error".into())]);
        let e = consultar(&g, "p", &sql("SELEC 1"), |_, _| Ok(())).unwrap_err();
        assert!(e.contains("Syntax error"), "{e}");
    }

    #[test]
    fn un_job_que_falla_no_se_da_por_bueno() {
        let g = Guion::new(vec![Ok(json!({
            "jobComplete": true,
            "errors": [{"message": "Access Denied: Table x"}]
        }))]);
        let e = consultar(&g, "p", &sql("SELECT 1"), |_, _| Ok(())).unwrap_err();
        assert!(e.contains("Access Denied"), "{e}");
    }

    #[test]
    fn el_cuerpo_pide_instantes_enteros_y_parametros_tipados() {
        let ps = vec!["p0:TIMESTAMP:2026-09-01 10:00:00+00".to_string()];
        let c = cuerpo(&Consulta {
            texto: "SELECT 1",
            parametros: &ps,
            sin_job: true,
        })
        .unwrap();
        assert_eq!(c["formatOptions"]["useInt64Timestamp"], true);
        assert_eq!(c["parameterMode"], "NAMED");
        assert_eq!(
            c["queryParameters"][0]["parameterType"]["type"],
            "TIMESTAMP"
        );
        assert_eq!(
            c["queryParameters"][0]["parameterValue"]["value"], "2026-09-01 10:00:00+00",
            "el valor conserva sus `:`"
        );
        assert_eq!(c["jobCreationMode"], "JOB_CREATION_OPTIONAL");
        assert!(parametro("sin-tipo").is_err());
    }

    #[test]
    fn el_esquema_de_tables_get_trae_required() {
        let g = Guion::new(vec![Ok(grabada("pedidos-tables-get"))]);
        let campos = esquema(&g, "p", "ventas", "pedidos").unwrap();
        assert_eq!(campos[0]["name"], "id");
        assert_eq!(campos[0]["mode"], "REQUIRED");
    }

    #[test]
    fn los_datasets_se_leen_de_la_respuesta_grabada() {
        let g = Guion::new(vec![Ok(grabada("datasets-list"))]);
        assert!(datasets(&g, "p").unwrap().contains(&"ventas".to_string()));
    }
}
