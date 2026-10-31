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
//! **Quién llama** ([`Credencial`]): un portador (un token del IdP, en las pruebas y en un proceso de confianza) o,
//! en un puesto, **el token de su pod** (`x-ore-pod`). Con él, `ore-serve` escribe a nombre de la persona que abrió
//! el puesto, en su rama, y deja al pod como agente (`sujeto_del_puesto`): el scheduler y los ejecutores de una
//! sesión son pods de su puesto y heredan esa identidad sin más (C10·3, *identity propagation*).
//!
//! **Lo que se lee** (C10·2), por las mismas rutas que Spark, DuckDB y PyIceberg:
//!
//! - [`CatalogoV1`]: los nombres de cada sentencia, con `loadTable` (y `loadView` si es una vista) en paralelo. `ore-serve`
//!   decide ahí el acceso, la rama del puesto y lo que el puesto lee, y presta la credencial de esa tabla;
//! - [`LagosV1`]: el lago de cada tabla con la credencial prestada para ella, que pide `loadCredentials` cuando hace
//!   falta y antes de que caduque. Un ejecutor la canjea él mismo: la credencial no viaja en el plan.

use crate::catalogo::ListaOre;
use crate::escritura::{FalloCommit, Puerta};
use crate::foto::{Foto, Nombre, Objeto};
use crate::lagos::{Lagos, bucket_de};
use async_trait::async_trait;
use datafusion::catalog::CatalogProviderList;
use datafusion::common::TableReference;
use datafusion::execution::SessionState;
use iceberg::spec::{Schema, TableMetadata};
use iceberg::table::Table;
use iceberg::{TableRequirement, TableUpdate};
use ore_runtime::Fallo;
use ore_runtime::zocalos::Catalogo;
use ore_store::almacen::Almacen;
use ore_store::lago::Lago;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// **Quién llama a `/v1`.**
#[derive(Debug, Clone)]
pub enum Credencial {
    /// Un token del IdP, como `Authorization: Bearer`.
    Portador(String),
    /// El token proyectado del pod (`x-ore-pod`), que el kubelet renueva: se lee en cada petición.
    Pod(PathBuf),
}

/// La cabecera de una petición a `/v1` con esta credencial.
fn cabecera(c: &Credencial) -> Result<(&'static str, String), String> {
    match c {
        Credencial::Portador(t) => Ok(("Authorization", format!("Bearer {t}"))),
        Credencial::Pod(ruta) => std::fs::read_to_string(ruta)
            .map(|t| ("x-ore-pod", t.trim().to_string()))
            .map_err(|e| format!("el token del pod `{}`: {e}", ruta.display())),
    }
}

/// Las esperas entre los reintentos de un commit: las del SDK (`_ESPERAS`).
pub const ESPERAS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

/// **La puerta de `/v1`**: dónde está `ore-serve`, quién llama y el lago de cada tabla.
#[derive(Clone)]
pub struct PuertaV1 {
    url: String,
    credencial: Credencial,
    lagos: Arc<dyn Lagos>,
    agente: ureq::Agent,
    esperas: Vec<Duration>,
}

impl std::fmt::Debug for PuertaV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PuertaV1({})", self.url)
    }
}

impl PuertaV1 {
    /// Con un portador y un lago para todo.
    pub fn new(url: impl Into<String>, token: impl Into<String>, lago: Lago) -> PuertaV1 {
        PuertaV1::con(url, Credencial::Portador(token.into()), Arc::new(lago))
    }

    /// Con esta credencial y estos lagos.
    pub fn con(url: impl Into<String>, credencial: Credencial, lagos: Arc<dyn Lagos>) -> PuertaV1 {
        PuertaV1 {
            url: url.into().trim_end_matches('/').to_string(),
            credencial,
            lagos,
            agente: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(120))
                .build(),
            esperas: ESPERAS.to_vec(),
        }
    }

    /// La misma, con otros lagos.
    pub fn con_lagos(mut self, lagos: Arc<dyn Lagos>) -> PuertaV1 {
        self.lagos = lagos;
        self
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

    fn vista(&self, n: &Nombre) -> String {
        format!(
            "{}/v1/{}/namespaces/{}/views/{}",
            self.url, n.base, n.schema, n.nombre
        )
    }

    /// Una petición, en un hilo de bloqueo: `(código, cuerpo)`, con `0` si la red no contestó.
    async fn pedir(
        &self,
        metodo: &'static str,
        url: String,
        cuerpo: Option<Value>,
        clave: Option<String>,
    ) -> (u16, Value) {
        let yo = self.clone();
        tokio::task::spawn_blocking(move || yo.pedir_ya(metodo, &url, cuerpo, clave.as_deref()))
            .await
            .unwrap_or_else(|e| (0, json!({ "error": e.to_string() })))
    }

    /// La misma petición, en este hilo: el almacén prestado la hace desde el `FileIO`, que es síncrono.
    fn pedir_ya(
        &self,
        metodo: &str,
        url: &str,
        cuerpo: Option<Value>,
        clave: Option<&str>,
    ) -> (u16, Value) {
        let (h, v) = match cabecera(&self.credencial) {
            Ok(c) => c,
            Err(e) => return (0, json!({ "error": e })),
        };
        let mut r = self.agente.request(metodo, url).set(h, &v);
        if let Some(k) = clave {
            r = r.set("Idempotency-Key", k);
        }
        if metodo == "GET" {
            r = r.set("X-Iceberg-Access-Delegation", "vended-credentials");
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
        // Lo que nace se escribe con la credencial que da crear.
        self.lagos.sembrar(nombre, &r);
        self.lagos
            .de(nombre, meta.location())?
            .esbozada(meta, &dataset)
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

// ── C10·2: el lago prestado ──────────────────────────────────────────────────────────────────────────────────

/// Lo que se renueva antes de que caduque: el FileIO no avisa a mitad de una lectura.
const ANTES_DE_CADUCAR_MS: i64 = 5 * 60 * 1000;

/// Cómo se hace un almacén con una credencial prestada: `(bucket, token) → almacén`. En producción, la cuenta de
/// GCS con ese token; en las pruebas, el almacén en memoria.
pub type ConCredencial = Arc<dyn Fn(&str, &str) -> Result<Arc<dyn Almacen>, String> + Send + Sync>;

/// Una credencial prestada: su token y cuándo caduca (ms desde 1970).
#[derive(Debug, Clone)]
struct Prestada {
    token: String,
    caduca_ms: Option<i64>,
}

/// La credencial del `storage-credentials` (o del `config`) de una respuesta de `/v1` que cubre `ubicacion`: la de
/// prefijo más largo, como pide la spec.
fn prestada_de(r: &Value, ubicacion: &str) -> Option<Prestada> {
    let mut mejor: Option<(usize, &Value)> = None;
    for c in r["storage-credentials"].as_array().into_iter().flatten() {
        let prefijo = c["prefix"].as_str().unwrap_or_default();
        if (ubicacion.is_empty() || ubicacion.starts_with(prefijo))
            && mejor.is_none_or(|(n, _)| prefijo.len() > n)
        {
            mejor = Some((prefijo.len(), &c["config"]));
        }
    }
    let config = mejor.map(|(_, c)| c).unwrap_or(&r["config"]);
    let token = config["gcs.oauth2.token"].as_str()?.to_string();
    let caduca_ms = match &config["gcs.oauth2.token-expires-at"] {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    };
    Some(Prestada { token, caduca_ms })
}

fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// **Los lagos prestados** (C10·2): uno por tabla, con la credencial que `/v1` presta para ella.
#[derive(Clone)]
pub struct LagosV1 {
    puerta: PuertaV1,
    con_credencial: ConCredencial,
    prestadas: Arc<Mutex<HashMap<Nombre, Prestada>>>,
}

impl std::fmt::Debug for LagosV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LagosV1({})", self.puerta.url)
    }
}

impl LagosV1 {
    /// Sobre GCS: la cuenta del bucket de cada tabla con el token prestado.
    pub fn gcs(url: impl Into<String>, credencial: Credencial) -> LagosV1 {
        LagosV1::con(
            url,
            credencial,
            Arc::new(|bucket: &str, token: &str| {
                Ok(Arc::new(ore_store::gcs::Cuenta::con_token(bucket, token)?) as Arc<dyn Almacen>)
            }),
        )
    }

    pub fn con(
        url: impl Into<String>,
        credencial: Credencial,
        con_credencial: ConCredencial,
    ) -> LagosV1 {
        let mut l = LagosV1 {
            // Sin lagos todavía: los suyos son éstos, y se ponen abajo.
            puerta: PuertaV1::con(url, credencial, Arc::new(SinLagos)),
            con_credencial,
            prestadas: Default::default(),
        };
        l.puerta = l.puerta.clone().con_lagos(Arc::new(l.clone()));
        l
    }

    /// La puerta del commit con estos lagos: la misma credencial y el mismo `ore-serve`.
    pub fn puerta(&self) -> PuertaV1 {
        self.puerta.clone()
    }

    /// La credencial vigente de `nombre`: la sembrada o la última pedida si le queda vida; si no, `loadCredentials`.
    fn prestada(&self, nombre: &Nombre, ubicacion: &str) -> Result<Prestada, String> {
        if let Some(p) = self.prestadas.lock().unwrap().get(nombre)
            && p.caduca_ms
                .is_none_or(|c| c - ANTES_DE_CADUCAR_MS > ahora_ms())
        {
            return Ok(p.clone());
        }
        let url = format!("{}/credentials", self.puerta.tabla(nombre));
        let (c, r) = self.puerta.pedir_ya("GET", &url, None, None);
        if c != 200 {
            return Err(format!(
                "`{nombre}`: /v1 no prestó la credencial ({c}): {}",
                mensaje(&r)
            ));
        }
        let p = prestada_de(&r, ubicacion)
            .ok_or_else(|| format!("`{nombre}`: /v1 contestó sin credencial de GCS: {r}"))?;
        self.prestadas
            .lock()
            .unwrap()
            .insert(nombre.clone(), p.clone());
        Ok(p)
    }
}

impl Lagos for LagosV1 {
    fn de(&self, nombre: &Nombre, ubicacion: &str) -> Result<Lago, String> {
        let bucket = if ubicacion.is_empty() {
            String::new()
        } else {
            bucket_de(ubicacion)?
        };
        Ok(Lago::nuevo(Arc::new(AlmacenPrestado {
            lagos: self.clone(),
            nombre: nombre.clone(),
            ubicacion: ubicacion.to_string(),
            bucket,
            vigente: Mutex::new(None),
        })))
    }

    fn sembrar(&self, nombre: &Nombre, prestado: &Value) {
        if let Some(p) = prestada_de(prestado, "") {
            self.prestadas.lock().unwrap().insert(nombre.clone(), p);
        }
    }
}

/// Sin lagos: el hueco mientras se construye [`LagosV1`].
#[derive(Debug)]
struct SinLagos;

impl Lagos for SinLagos {
    fn de(&self, nombre: &Nombre, _: &str) -> Result<Lago, String> {
        Err(format!("`{nombre}`: sin lago"))
    }
}

/// **El almacén de una tabla con su credencial prestada**: cada operación va al almacén del token vigente, y lo
/// rehace cuando cambia.
struct AlmacenPrestado {
    lagos: LagosV1,
    nombre: Nombre,
    ubicacion: String,
    bucket: String,
    vigente: Mutex<Option<(String, Arc<dyn Almacen>)>>,
}

impl AlmacenPrestado {
    fn almacen(&self) -> Result<Arc<dyn Almacen>, String> {
        if self.bucket.is_empty() {
            return Err(format!(
                "`{}`: no se sabe dónde están sus ficheros",
                self.nombre
            ));
        }
        let p = self.lagos.prestada(&self.nombre, &self.ubicacion)?;
        let mut v = self.vigente.lock().unwrap();
        if let Some((token, a)) = v.as_ref()
            && *token == p.token
        {
            return Ok(a.clone());
        }
        let a = (self.lagos.con_credencial)(&self.bucket, &p.token)?;
        *v = Some((p.token, a.clone()));
        Ok(a)
    }
}

impl Almacen for AlmacenPrestado {
    fn base(&self) -> String {
        format!("gs://{}", self.bucket)
    }
    fn leer(&self, clave: &str) -> Result<Option<String>, String> {
        self.almacen()?.leer(clave)
    }
    fn existe(&self, clave: &str) -> Result<bool, String> {
        self.almacen()?.existe(clave)
    }
    fn tamano(&self, clave: &str) -> Result<Option<u64>, String> {
        self.almacen()?.tamano(clave)
    }
    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
        self.almacen()?.subir(clave, cuerpo)
    }
    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
        self.almacen()?.listar(prefijo)
    }
    fn borrar(&self, clave: &str) -> Result<(), String> {
        self.almacen()?.borrar(clave)
    }
    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
        self.almacen()?.leer_bytes(clave)
    }
    fn leer_rango(
        &self,
        clave: &str,
        rango: Option<(u64, u64)>,
    ) -> Result<Option<Vec<u8>>, String> {
        self.almacen()?.leer_rango(clave, rango)
    }
}

// ── C10·2: el catálogo por /v1 ───────────────────────────────────────────────────────────────────────────────

/// **El catálogo de un puesto** (C10·2): lo que cada sentencia nombra, por `/v1`, como la persona del puesto. Cada
/// nombre se pide con `loadTable` (si es una vista, `loadView`, y entonces también lo que la vista nombra), y lo que
/// contesta es la foto de la sentencia: el puntero, los metadatos ya abiertos y la credencial de esa tabla.
///
/// Un mismo nombre se resuelve una vez por sentencia, así que dos menciones ven el mismo snapshot. Dos tablas
/// distintas se leen cada una en su `loadTable`, en paralelo: es lo que da el REST de Iceberg, que no tiene todavía
/// una foto de varias tablas (la propuesta de su número de commit del catálogo sigue abierta).
#[derive(Debug, Clone)]
pub struct CatalogoV1 {
    lagos: LagosV1,
    escribe: bool,
}

impl CatalogoV1 {
    /// Un catálogo que lee; [`CatalogoV1::que_escribe`] le da la puerta.
    pub fn new(lagos: LagosV1) -> CatalogoV1 {
        CatalogoV1 {
            lagos,
            escribe: false,
        }
    }

    pub fn que_escribe(mut self) -> CatalogoV1 {
        self.escribe = true;
        self
    }

    /// El codec de este catálogo (C8), con los mismos lagos y la misma puerta.
    pub fn codec(&self) -> crate::CodecOre {
        crate::CodecOre::new(
            Arc::new(self.lagos.clone()),
            self.escribe
                .then(|| Arc::new(self.lagos.puerta()) as Arc<dyn Puerta>),
        )
    }

    /// Un nombre, por `/v1`: un dataset abierto, una vista con su SQL, o nada.
    async fn resolver(&self, n: &Nombre) -> Result<Option<(Objeto, Option<Table>)>, Fallo> {
        let puerta = &self.lagos.puerta;
        let (c, r) = puerta.pedir("GET", puerta.tabla(n), None, None).await;
        match c {
            200 => {
                let ml = r["metadata-location"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let meta: TableMetadata = serde_json::from_value(r["metadata"].clone())
                    .map_err(|e| Fallo::Catalogo(format!("`{n}`: los metadatos de /v1: {e}")))?;
                self.lagos.sembrar(n, &r);
                let tabla = self
                    .lagos
                    .de(n, meta.location())
                    .and_then(|l| l.de_metadatos(meta, Some(&ml), &n.to_string()))
                    .map_err(|e| Fallo::Catalogo(format!("`{n}`: {e}")))?;
                return Ok(Some((
                    Objeto::Dataset {
                        metadata_location: ml,
                    },
                    Some(tabla),
                )));
            }
            404 => {}
            c => {
                return Err(Fallo::Catalogo(format!(
                    "`{n}`: /v1 contestó {c} a loadTable: {}",
                    mensaje(&r)
                )));
            }
        }
        let (c, r) = puerta.pedir("GET", puerta.vista(n), None, None).await;
        match c {
            200 => {
                let actual = &r["metadata"]["current-version-id"];
                let sql = r["metadata"]["versions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|v| &v["version-id"] == actual)
                    .and_then(|v| v["representations"].as_array())
                    .into_iter()
                    .flatten()
                    .find(|x| x["type"] == "sql")
                    .and_then(|x| x["sql"].as_str())
                    .ok_or_else(|| {
                        Fallo::Catalogo(format!("`{n}`: la vista de /v1 no trae SQL"))
                    })?;
                Ok(Some((
                    Objeto::Vista {
                        sql: sql.to_string(),
                    },
                    None,
                )))
            }
            404 => Ok(None),
            c => Err(Fallo::Catalogo(format!(
                "`{n}`: /v1 contestó {c} a loadView: {}",
                mensaje(&r)
            ))),
        }
    }
}

/// Los nombres de una consulta de una vista, en su base y su schema.
fn nombrados_por(sql: &str, n: &Nombre) -> Vec<Nombre> {
    let Ok(sentencias) = datafusion::sql::parser::DFParser::parse_sql(sql) else {
        return Vec::new();
    };
    let mut fuera = Vec::new();
    for s in sentencias {
        let Ok((refs, _ctes)) = datafusion::sql::resolve::resolve_table_references(&s, true) else {
            continue;
        };
        for r in refs {
            let r = r.resolve(&n.base, &n.schema);
            fuera.push(Nombre::new(&r.catalog, &r.schema, &r.table));
        }
    }
    fuera
}

#[async_trait]
impl Catalogo for CatalogoV1 {
    async fn preparar(
        &self,
        base: &SessionState,
        referencias: &[TableReference],
    ) -> Result<Arc<dyn CatalogProviderList>, Fallo> {
        let c = &base.config().options().catalog;
        let mut por_pedir: BTreeSet<Nombre> = referencias
            .iter()
            .map(|r| r.clone().resolve(&c.default_catalog, &c.default_schema))
            .filter(|r| &*r.schema != "information_schema")
            .map(|r| Nombre::new(&r.catalog, &r.schema, &r.table))
            .collect();
        let mut foto = Foto::default();
        let mut abiertas = BTreeMap::new();
        let mut vistos = BTreeSet::new();
        while !por_pedir.is_empty() {
            let ronda: Vec<Nombre> = std::mem::take(&mut por_pedir)
                .into_iter()
                .filter(|n| vistos.insert(n.clone()))
                .collect();
            let hechos = futures::future::join_all(ronda.iter().map(|n| self.resolver(n))).await;
            for (n, hecho) in ronda.into_iter().zip(hechos) {
                match hecho? {
                    Some((Objeto::Vista { sql }, _)) => {
                        por_pedir.extend(nombrados_por(&sql, &n));
                        foto.objetos.insert(n, Objeto::Vista { sql });
                    }
                    Some((o, t)) => {
                        if let Some(t) = t {
                            abiertas.insert(n.clone(), t);
                        }
                        foto.objetos.insert(n, o);
                    }
                    None => {}
                }
            }
        }
        Ok(ListaOre::nueva(
            foto,
            abiertas,
            Arc::new(self.lagos.clone()),
            self.escribe
                .then(|| Arc::new(self.lagos.puerta()) as Arc<dyn Puerta>),
            base.clone(),
        ))
    }

    fn en_el_tiempo(
        &self,
        tabla: &Arc<dyn datafusion::catalog::TableProvider>,
        punto: &ore_runtime::viaje::Punto,
    ) -> Result<Arc<dyn datafusion::catalog::TableProvider>, Fallo> {
        let t = tabla
            .as_ref()
            .downcast_ref::<crate::tabla::TablaOre>()
            .ok_or_else(|| {
                Fallo::Catalogo("sólo un dataset del lago viaja en el tiempo; una vista, no".into())
            })?;
        Ok(Arc::new(t.en(punto).map_err(Fallo::Catalogo)?))
    }
}
