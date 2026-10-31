//! **La puerta de `/v1`** (0081 §7.5.3, decisión 5; C5): el commit de producción. El runtime commitea como cualquier
//! motor y como el SDK: por el catálogo REST de Iceberg que sirve `ore-serve`, que pone la idempotencia, las
//! promesas, el dueño y el documento (ADR 0031). Las mismas rutas y la misma forma que `write()` del SDK:
//!
//! - **una tabla que nace**: `POST /v1/<base>/namespaces/<schema>/tables` con `stage-create`; lo que contesta es el
//!   esbozo sobre el que se escriben los ficheros;
//! - **el commit**: `POST /v1/<base>/namespaces/<schema>/tables/<nombre>` (`updateTable`) con su `Idempotency-Key`.
//!   Un 5xx o un corte se reintentan **con la misma clave** (0,5, 1 y 2 s): el catálogo no aplica dos veces. Un 409
//!   es que alguien escribió después de la foto;
//! - **retirar**: `DELETE` de la tabla (`dropTable`). El REST de Iceberg no lleva el puntero esperado: el CAS de
//!   retirar es el de `ore-serve`.
//!
//! La credencial con que se escriben los bytes (la que `/v1` presta en `config`) es del frente 4: aquí, el `Lago`
//! del catálogo.

use crate::escritura::{FalloCommit, Puerta};
use crate::foto::Nombre;
use async_trait::async_trait;
use iceberg::spec::{Schema, TableMetadata};
use iceberg::table::Table;
use iceberg::{TableRequirement, TableUpdate};
use ore_store::lago::Lago;
use serde_json::{Value, json};
use std::time::Duration;

/// Las esperas entre los reintentos de un commit: las del SDK (`_ESPERAS`).
pub const ESPERAS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

/// **La puerta de `/v1`**: dónde está `ore-serve`, con qué token y el lago de los bytes.
#[derive(Clone)]
pub struct PuertaV1 {
    url: String,
    token: String,
    lago: Lago,
    agente: ureq::Agent,
    esperas: Vec<Duration>,
}

impl std::fmt::Debug for PuertaV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PuertaV1({})", self.url)
    }
}

impl PuertaV1 {
    pub fn new(url: impl Into<String>, token: impl Into<String>, lago: Lago) -> PuertaV1 {
        PuertaV1 {
            url: url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            lago,
            agente: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(120))
                .build(),
            esperas: ESPERAS.to_vec(),
        }
    }

    /// Con otras esperas (las pruebas).
    pub fn con_esperas(mut self, esperas: Vec<Duration>) -> PuertaV1 {
        self.esperas = esperas;
        self
    }

    fn tablas(&self, n: &Nombre) -> String {
        format!("{}/v1/{}/namespaces/{}/tables", self.url, n.base, n.schema)
    }

    fn tabla(&self, n: &Nombre) -> String {
        format!("{}/{}", self.tablas(n), n.nombre)
    }

    /// Una petición, en un hilo de bloqueo: `(código, cuerpo)`, con `0` si la red no contestó.
    async fn pedir(
        &self,
        metodo: &'static str,
        url: String,
        cuerpo: Option<Value>,
        clave: Option<String>,
    ) -> (u16, Value) {
        let (agente, token) = (self.agente.clone(), self.token.clone());
        tokio::task::spawn_blocking(move || {
            let mut r = agente
                .request(metodo, &url)
                .set("Authorization", &format!("Bearer {token}"));
            if let Some(k) = &clave {
                r = r.set("Idempotency-Key", k);
            }
            let respuesta = match cuerpo {
                Some(c) => r.send_json(c),
                None => r.call(),
            };
            let (codigo, texto) = match respuesta {
                Ok(r) => (r.status(), r.into_string().unwrap_or_default()),
                Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
                Err(e) => return (0, json!({ "error": e.to_string() })),
            };
            let cuerpo = serde_json::from_str(&texto).unwrap_or(json!({ "error": texto }));
            (codigo, cuerpo)
        })
        .await
        .unwrap_or_else(|e| (0, json!({ "error": e.to_string() })))
    }

    /// Una petición con sus reintentos, **con la misma clave**, mientras la red o el catálogo fallen.
    async fn con_reintentos(
        &self,
        metodo: &'static str,
        url: String,
        cuerpo: Option<Value>,
        clave: &str,
    ) -> (u16, Value) {
        let mut ultimo = (0, json!({ "error": "sin respuesta" }));
        for espera in std::iter::once(Duration::ZERO).chain(self.esperas.iter().copied()) {
            if !espera.is_zero() {
                tokio::time::sleep(espera).await;
            }
            let r = self
                .pedir(metodo, url.clone(), cuerpo.clone(), Some(clave.to_string()))
                .await;
            if r.0 != 0 && r.0 < 500 {
                return r;
            }
            ultimo = r;
        }
        ultimo
    }
}

/// El mensaje de un error de `/v1` (`{"error": {"message": …}}` de la spec, o lo que haya).
fn mensaje(r: &Value) -> String {
    r.pointer("/error/message")
        .or_else(|| r.get("error"))
        .map(|v| {
            v.as_str()
                .map(String::from)
                .unwrap_or_else(|| v.to_string())
        })
        .unwrap_or_else(|| r.to_string())
}

/// El cuerpo de un `updateTable`, como lo manda el SDK.
pub fn cuerpo_del_commit(
    nombre: &Nombre,
    requisitos: &[TableRequirement],
    cambios: &[TableUpdate],
) -> Result<Value, String> {
    Ok(json!({
        "identifier": { "namespace": [nombre.schema], "name": nombre.nombre },
        "requirements": serde_json::to_value(requisitos).map_err(|e| e.to_string())?,
        "updates": serde_json::to_value(cambios).map_err(|e| e.to_string())?,
    }))
}

#[async_trait]
impl Puerta for PuertaV1 {
    async fn esbozar(&self, nombre: &Nombre, esquema: Schema) -> Result<Table, String> {
        let cuerpo = json!({
            "name": nombre.nombre,
            "stage-create": true,
            "schema": serde_json::to_value(&esquema).map_err(|e| e.to_string())?,
            "properties": {},
        });
        let (c, r) = self
            .pedir("POST", self.tablas(nombre), Some(cuerpo), None)
            .await;
        if c != 200 {
            return Err(format!("`{nombre}`: /v1 contestó {c}: {}", mensaje(&r)));
        }
        let meta: TableMetadata = serde_json::from_value(r["metadata"].clone())
            .map_err(|e| format!("`{nombre}`: el esbozo de /v1 no es una tabla: {e}"))?;
        let dataset = format!(
            "catalogo/{}/{}/{}",
            nombre.base, nombre.schema, nombre.nombre
        );
        self.lago.esbozada(meta, &dataset)
    }

    async fn confirmar(
        &self,
        nombre: &Nombre,
        _tabla: &Table,
        requisitos: Vec<TableRequirement>,
        cambios: Vec<TableUpdate>,
        clave: &str,
    ) -> Result<String, FalloCommit> {
        let cuerpo = cuerpo_del_commit(nombre, &requisitos, &cambios).map_err(FalloCommit::Otro)?;
        let (c, r) = self
            .con_reintentos("POST", self.tabla(nombre), Some(cuerpo), clave)
            .await;
        match c {
            200 => r["metadata-location"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| {
                    FalloCommit::Otro(format!("/v1 contestó 200 sin metadata-location: {r}"))
                }),
            409 => Err(FalloCommit::Conflicto { hay: None }),
            // Sin respuesta, o un 5xx en el último intento: no se sabe si aplicó.
            c if c == 0 || c >= 500 => Err(FalloCommit::SinRespuesta(format!(
                "/v1 no contestó bien tras {} intentos con la misma Idempotency-Key ({c}): {}",
                self.esperas.len() + 1,
                mensaje(&r)
            ))),
            c => Err(FalloCommit::Otro(format!(
                "/v1 contestó {c}: {}",
                mensaje(&r)
            ))),
        }
    }

    async fn retirar(
        &self,
        nombre: &Nombre,
        _esperado: &str,
        clave: &str,
    ) -> Result<(), FalloCommit> {
        let (c, r) = self
            .con_reintentos("DELETE", self.tabla(nombre), None, clave)
            .await;
        match c {
            200 | 204 => Ok(()),
            409 => Err(FalloCommit::Conflicto { hay: None }),
            c => Err(FalloCommit::Otro(format!(
                "/v1 contestó {c}: {}",
                mensaje(&r)
            ))),
        }
    }
}
