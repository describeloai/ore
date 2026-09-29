//! **La subida**, firmada con SigV4 y condicional.
//!
//! Lo que este módulo hace es exactamente lo que el
//! [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md) midió
//! contra un R2 de verdad antes de escribirse:
//!
//! | | |
//! |---|---|
//! | `If-None-Match: *` en `PutObject` | **la honra** — la segunda da `412` |
//! | `ChecksumSHA256` en el `PUT` | **lo valida el servidor**: uno malo da `BadDigest` |
//! | `CopyObject` con `If-None-Match: *` | **no protege** |
//!
//! Por eso aquí no hay `CopyObject`: se construye el artefacto entero en local,
//! **se conoce su digest antes de subir**, y se sube directo al nombre
//! definitivo con las dos garantías puestas. La ruta multiparte —para lo que no
//! cabe en un `PUT`— queda fuera de este peldaño y se dice.
//!
//! # La credencial no viaja por `argv`
//!
//! Se lee del entorno, que es la misma doctrina que `source add` aplica desde
//! v1alpha1: *declara dónde buscar el secreto, no cuál es*. `argv` lo lee
//! cualquier proceso de la máquina.
//!
//! # Y el `User-Agent`
//!
//! El borde de Cloudflare rechaza peticiones sin uno reconocible con `error code:
//! 1010`, que **se lee como un fallo de autenticación y no lo es**. Costó un rato
//! encontrarlo y por eso está escrito aquí y en el ADR.

use crate::almacen::Almacen;
use std::io::Read as _;

pub struct Cuenta {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub clave: String,
    pub secreto: String,
}

impl Cuenta {
    /// Del entorno, y con un error que dice **cuál** falta.
    pub fn del_entorno() -> Result<Cuenta, String> {
        let v =
            |k: &str| std::env::var(k).map_err(|_| format!("falta la variable de entorno `{k}`"));
        Ok(Cuenta {
            endpoint: v("ORE_R2_S3_ENDPOINT")?,
            bucket: v("ORE_R2_BUCKET")?,
            region: v("ORE_R2_REGION").unwrap_or_else(|_| "auto".to_string()),
            clave: v("ORE_R2_ACCESS_KEY_ID")?,
            secreto: v("ORE_R2_SECRET_ACCESS_KEY")?,
        })
    }

    fn host(&self) -> String {
        self.endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string()
    }
}

// La firma vive en `ore-s3` desde 0046 E4: la comparte con el lector de un
// bucket del cliente, y dos firmas divergirian en el caso que nadie prueba.
use ore_s3::{base64, hex, sha256};

fn credencial(c: &Cuenta) -> ore_s3::Credencial {
    ore_s3::Credencial {
        clave: c.clave.clone(),
        secreto: c.secreto.clone(),
        token: None,
    }
}

/// La firma de una petición. Cabeceras **ordenadas**, que es lo que exige el
/// esquema: la firma es sobre una forma canónica, como todo lo demás aquí.
fn firmar(
    c: &Cuenta,
    metodo: &str,
    ruta: &str,
    cabeceras: Vec<(String, String)>,
    hash_cuerpo: &str,
) -> Vec<(String, String)> {
    firmar_con_consulta(c, metodo, ruta, "", cabeceras, hash_cuerpo)
}

/// Lo mismo, con cadena de consulta. Existe porque enumerar por prefijo la
/// necesita —`?list-type=2&prefix=…`— y SigV4 la firma **aparte** de la ruta,
/// con los parámetros **ordenados por nombre**.
fn firmar_con_consulta(
    c: &Cuenta,
    metodo: &str,
    ruta: &str,
    consulta: &str,
    cabeceras: Vec<(String, String)>,
    hash_cuerpo: &str,
) -> Vec<(String, String)> {
    let mut ps: Vec<&str> = consulta.split('&').filter(|s| !s.is_empty()).collect();
    ps.sort_unstable();
    ore_s3::firma::firmar(
        &credencial(c),
        &c.region,
        &c.host(),
        metodo,
        ruta,
        &ps.join("&"),
        cabeceras,
        hash_cuerpo,
    )
}

fn uri(s: &str) -> String {
    ore_s3::firma::uri(s)
}

/// El `User-Agent`. Sin uno reconocible, el borde de Cloudflare devuelve
/// `error code: 1010`, que se lee como un fallo de autenticación y no lo es.
const AGENTE: &str = concat!("ore-store-r2/", env!("CARGO_PKG_VERSION"));

fn url(c: &Cuenta, ruta: &str) -> String {
    format!("{}{ruta}", c.endpoint.trim_end_matches('/'))
}

/// El cliente, con el TLS **de la plataforma** enganchado a mano.
///
/// `ureq` con `native-tls` no lo cablea solo: sin esto las peticiones salen con
/// *«cannot make HTTPS request because no TLS backend is configured»*, que es
/// otro error que se lee como una cosa y es otra.
///
/// Y es TLS del sistema y no una pila propia por lo mismo que documenta
/// `ore-read-postgres`: **la CA privada de la empresa ya está donde el sistema
/// operativo la busca**, y un almacén al que no se puede llegar desde detrás de
/// un proxy corporativo no sirve para lo que este almacén existe.
fn cliente() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new()
        .map_err(|e| format!("no se pudo abrir el TLS de la plataforma: {e}"))?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .build())
}

/// Lee un objeto entero. Solo se usa para el **recibo**, que son 71 bytes: la
/// clave del artefacto que una cabecera ya produjo.
pub fn leer(c: &Cuenta, clave: &str) -> Result<Option<String>, String> {
    let ruta = format!("/{}/{clave}", c.bucket);
    let vacio = hex(&sha256(b""));
    let cab = firmar(c, "GET", &ruta, Vec::new(), &vacio);
    let mut r = cliente()?.get(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.call() {
        Ok(resp) => resp
            .into_string()
            .map(|s| Some(s.trim().to_string()))
            .map_err(|e| format!("el recibo `{clave}` no se pudo leer: {e}")),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(format!("el `GET` de `{clave}` falla: {e}")),
    }
}

/// **El paso 4 del ciclo: se sabe si hay que copiar sin copiar nada.**
///
/// Es el que paga el diseño entero. Un `HEAD` sobre el nombre del digest evita
/// leer una sola fila del origen cuando la copia ya está.
pub fn existe(c: &Cuenta, clave: &str) -> Result<bool, String> {
    let ruta = format!("/{}/{clave}", c.bucket);
    let vacio = hex(&sha256(b""));
    let cab = firmar(c, "HEAD", &ruta, Vec::new(), &vacio);
    let mut r = cliente()?.head(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.call() {
        Ok(_) => Ok(true),
        Err(ureq::Error::Status(404, _)) => Ok(false),
        Err(e) => Err(format!("el `HEAD` de `{clave}` falla: {e}")),
    }
}

/// El `HEAD`, con lo que dice: `Content-Length`. `None` es que no está.
pub fn tamano(c: &Cuenta, clave: &str) -> Result<Option<u64>, String> {
    let ruta = format!("/{}/{clave}", c.bucket);
    let vacio = hex(&sha256(b""));
    let cab = firmar(c, "HEAD", &ruta, Vec::new(), &vacio);
    let mut r = cliente()?.head(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.call() {
        Ok(resp) => Ok(Some(
            resp.header("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        )),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(format!("el `HEAD` de `{clave}` falla: {e}")),
    }
}

/// **La subida, con las dos garantías que R2 honra.**
///
/// `If-None-Match: *` para no reescribir, y `ChecksumSHA256` para que **la
/// integridad la valide el servidor** en vez de confiar en el cliente. Un
/// `PreconditionFailed` no es un error: es que ya estaba, y el nombre es el
/// contenido, así que ya estaba **lo mismo**.
pub fn subir(c: &Cuenta, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
    let ruta = format!("/{}/{clave}", c.bucket);
    let digest = sha256(cuerpo);
    let cab = firmar(
        c,
        "PUT",
        &ruta,
        vec![
            ("content-length".into(), cuerpo.len().to_string()),
            ("if-none-match".into(), "*".into()),
            ("x-amz-checksum-sha256".into(), base64(&digest)),
        ],
        &hex(&digest),
    );
    let mut r = cliente()?.put(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.send_bytes(cuerpo) {
        Ok(_) => Ok(true),
        // Ya estaba. Y como el nombre es el contenido, ya estaba lo mismo.
        Err(ureq::Error::Status(412, _)) => Ok(false),
        Err(e) => Err(format!("la subida de `{clave}` falla: {e}")),
    }
}

/// **Un blob** (0046 E8·2): las mismas dos garantías que [`subir`] —`If-None-
/// Match: *` y `ChecksumSHA256`, que el servidor coteja antes de escribir—,
/// con su `Content-Type` y el cuerpo **en flujo**: el digest se conoce de
/// antes (lo calculó quien lo trajo), así que no hace falta tenerlo en memoria
/// para firmar.
pub fn poner_blob(c: &Cuenta, b: &crate::almacen::Blob) -> Result<bool, String> {
    let ruta = format!("/{}/{}", c.bucket, b.clave);
    let cab = firmar(
        c,
        "PUT",
        &ruta,
        vec![
            ("content-length".into(), b.tamano.to_string()),
            ("content-type".into(), b.tipo.clone()),
            ("if-none-match".into(), "*".into()),
            ("x-amz-checksum-sha256".into(), base64(&b.sha256)),
        ],
        &hex(&b.sha256),
    );
    let mut r = cliente()?.put(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.send(b.cuerpo.lector()?) {
        Ok(_) => Ok(true),
        Err(ureq::Error::Status(412, _)) => Ok(false),
        Err(ureq::Error::Status(c_, r)) => Err(format!(
            "el almacén rechazó `{}` ({c_}): {}",
            b.clave,
            r.into_string()
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect::<String>()
        )),
        Err(e) => Err(format!("la subida de `{}` falla: {e}", b.clave)),
    }
}

/// Un rango de un objeto, o entero.
pub fn leer_rango(
    c: &Cuenta,
    clave: &str,
    rango: Option<(u64, u64)>,
) -> Result<Option<Vec<u8>>, String> {
    let ruta = format!("/{}/{clave}", c.bucket);
    let vacio = hex(&sha256(b""));
    let mut cabeceras = Vec::new();
    if let Some((a, z)) = rango {
        cabeceras.push(("range".to_string(), format!("bytes={a}-{z}")));
    }
    let cab = firmar(c, "GET", &ruta, cabeceras, &vacio);
    let mut r = cliente()?.get(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.call() {
        Ok(resp) => {
            let mut b = Vec::new();
            resp.into_reader()
                .read_to_end(&mut b)
                .map_err(|e| format!("`{clave}` no se pudo leer: {e}"))?;
            Ok(Some(b))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(ureq::Error::Status(416, _)) => Ok(Some(Vec::new())),
        Err(e) => Err(format!("el `GET` de `{clave}` falla: {e}")),
    }
}

/// Enumera por prefijo. Es lo único que la recogida necesita del almacén, y R2
/// lo honra — medido en el ADR 0015 antes de escribirlo.
///
/// Sin paginar, y se dice: mil recibos de un mismo plan son mil refrescos sin
/// recoger, y ese es otro problema que este no arregla.
pub fn listar(c: &Cuenta, prefijo: &str) -> Result<Vec<String>, String> {
    // El valor va codificado **una vez** y se usa el mismo texto en la URL y en
    // la firma: si difirieran, el servidor firmaría otra cosa que el cliente.
    let consulta = format!("list-type=2&prefix={}", uri(prefijo));
    let canonica = format!("/{}", c.bucket);
    let ruta = format!("{canonica}?{consulta}");
    let vacio = hex(&sha256(b""));
    let cab = firmar_con_consulta(c, "GET", &canonica, &consulta, Vec::new(), &vacio);
    let mut r = cliente()?.get(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    let cuerpo = r
        .call()
        .map_err(|e| format!("no se pudo enumerar `{prefijo}`: {e}"))?
        .into_string()
        .map_err(|e| format!("la respuesta de la enumeración no se pudo leer: {e}"))?;
    // El XML de S3, leído por sus etiquetas. No entra un analizador de XML para
    // una lista de claves: lo que hace falta es lo que hay entre `<Key>` y su
    // cierre, y eso es una partición.
    Ok(cuerpo
        .split("<Key>")
        .skip(1)
        .filter_map(|t| t.split_once("</Key>").map(|(k, _)| k.to_string()))
        .collect())
}

/// Borra. Uno a uno y no en lote: la recogida es una operación deliberada y
/// pocas, y un `DeleteObjects` en lote pide firmar un cuerpo XML para ahorrar
/// unas cuantas peticiones que nadie está contando.
pub fn borrar(c: &Cuenta, clave: &str) -> Result<(), String> {
    let ruta = format!("/{}/{clave}", c.bucket);
    let vacio = hex(&sha256(b""));
    let cab = firmar(c, "DELETE", &ruta, Vec::new(), &vacio);
    let mut r = cliente()?.delete(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.call() {
        Ok(_) => Ok(()),
        // Ya no estaba. La recogida es idempotente por diseño: dos pasadas
        // sobre el mismo estado hacen lo mismo que una.
        Err(ureq::Error::Status(404, _)) => Ok(()),
        Err(e) => Err(format!("no se pudo borrar `{clave}`: {e}")),
    }
}

/// Lo mismo que [`leer`], pero en bytes: una copia es binaria.
pub fn leer_bytes(c: &Cuenta, clave: &str) -> Result<Option<Vec<u8>>, String> {
    let ruta = format!("/{}/{clave}", c.bucket);
    let vacio = hex(&sha256(b""));
    let cab = firmar(c, "GET", &ruta, Vec::new(), &vacio);
    let mut r = cliente()?.get(&url(c, &ruta)).set("user-agent", AGENTE);
    for (k, v) in &cab {
        r = r.set(k, v);
    }
    match r.call() {
        Ok(resp) => {
            let mut b = Vec::new();
            resp.into_reader()
                .read_to_end(&mut b)
                .map_err(|e| format!("la copia `{clave}` no se pudo leer entera: {e}"))?;
            Ok(Some(b))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(format!("el `GET` de `{clave}` falla: {e}")),
    }
}

impl Almacen for Cuenta {
    fn base(&self) -> String {
        format!("s3://{}", self.bucket)
    }

    /// Lo que S3 y R2 entienden (`s3.*` de la spec REST): las credenciales de
    /// esta cuenta **tal cual**, sin acotar. R2 tiene credenciales temporales
    /// por API y S3 tiene STS con política inline; ninguna de las dos está
    /// puesta todavía, y se dice (`acotada: false`) para que quien preste sepa
    /// qué presta. En local (el S3 de mentira) es lo que hay.
    fn prestar(&self, _prefijo: &str) -> Result<crate::almacen::Prestamo, String> {
        Ok(crate::almacen::Prestamo {
            config: [
                ("s3.access-key-id".to_string(), self.clave.clone()),
                ("s3.secret-access-key".to_string(), self.secreto.clone()),
                ("s3.endpoint".to_string(), self.endpoint.clone()),
                ("s3.region".to_string(), self.region.clone()),
                ("s3.path-style-access".to_string(), "true".to_string()),
            ]
            .into(),
            caduca_ms: None,
            acotada: false,
        })
    }

    fn tamano(&self, clave: &str) -> Result<Option<u64>, String> {
        tamano(self, clave)
    }

    fn leer(&self, clave: &str) -> Result<Option<String>, String> {
        leer(self, clave)
    }
    fn existe(&self, clave: &str) -> Result<bool, String> {
        existe(self, clave)
    }
    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
        subir(self, clave, cuerpo)
    }
    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
        listar(self, prefijo)
    }
    fn borrar(&self, clave: &str) -> Result<(), String> {
        borrar(self, clave)
    }
    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
        leer_bytes(self, clave)
    }
    fn poner_blob(&self, b: &crate::almacen::Blob) -> Result<bool, String> {
        poner_blob(self, b)
    }
    fn leer_rango(
        &self,
        clave: &str,
        rango: Option<(u64, u64)>,
    ) -> Result<Option<Vec<u8>>, String> {
        leer_rango(self, clave, rango)
    }
}

// La firma y sus vectores (el de la clave y el ejemplo de S3 entero) viven
// en `ore-s3`.
