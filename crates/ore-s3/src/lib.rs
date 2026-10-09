//! **S3, una vez**: la firma SigV4 y las lecturas de un bucket.
//!
//! Dos consumidores: el lago en R2 (`ore-store-r2`), que habla S3, y un bucket
//! del cliente como fuente (`ore-read-s3`, ADR 0046 E4). La firma vivía en
//! `ore-store/src/r2.rs`; un segundo cliente con la suya habrían sido dos
//! firmas, y la que falla lo dice con `SignatureDoesNotMatch`, que se lee como
//! una credencial mala.
//!
//! # Lo que se midió antes (0046 F1, un bucket real en `eu-north-1`)
//!
//! - **Listar es un permiso aparte**: `s3:ListBucket` va sobre el ARN del
//!   bucket, y sin él S3 contesta `403` **también** a una clave que no existe.
//!   Por eso [`Respuesta`] devuelve el estado y el código de AWS en vez de un
//!   error opaco: quien pregunta tiene que poder decir qué acción falta.
//! - **El `ETag` no es la huella del contenido** (un fichero subido por partes
//!   lleva `"…-N"`); `ChecksumCRC64NVME` de tipo `FULL_OBJECT` sí, y lo da un
//!   `HEAD` con `x-amz-checksum-mode: ENABLED` ([`cabeza`]).
//! - **La lectura por rangos funciona, sufijo incluido** (`bytes=-8`): el pie
//!   de un Parquet y el índice de un zip se leen sin bajar el fichero.
//! - Las claves llevan espacios y mayúsculas (`Nueva carpeta/Foto Portada
//!   2026.JPG`): se codifican **por segmento**, una vez, y el mismo texto va a
//!   la URL y a la firma.

// La firma y el bucket viven en `ore-sigv4`, que no sabe hablar por la red: el
// firmante de `ore-serve` (0046 E9·3) la enlaza sin arrastrar un cliente HTTP.
pub use ore_sigv4::{Bucket, firma};
pub mod huella;
pub mod origen;

pub use firma::{Credencial, base64, hex, sha256};

/// Lo que contestó S3, **sea lo que sea**: un `403` o un `301` también son
/// respuestas, y el código de AWS que traen es lo único accionable.
#[derive(Debug, Clone)]
pub struct Respuesta {
    pub estado: u16,
    pub cabeceras: Vec<(String, String)>,
    pub cuerpo: Vec<u8>,
}

impl Respuesta {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.estado)
    }

    pub fn cabecera(&self, k: &str) -> Option<&str> {
        self.cabeceras
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }

    /// `(Code, Message)` del XML de error de S3, si lo trae.
    pub fn error_de_aws(&self) -> Option<(String, String)> {
        let t = String::from_utf8_lossy(&self.cuerpo);
        let code = etiqueta(&t, "Code")?;
        Some((code, etiqueta(&t, "Message").unwrap_or_default()))
    }

    /// Una línea para un mensaje: el estado y, si lo hay, el código de AWS.
    pub fn motivo(&self) -> String {
        match self.error_de_aws() {
            Some((c, m)) if m.is_empty() => format!("{} {c}", self.estado),
            Some((c, m)) => format!("{} {c}: {m}", self.estado),
            None => format!("{}", self.estado),
        }
    }
}

/// El `User-Agent`: el borde de Cloudflare (R2) rechaza peticiones sin uno
/// reconocible con `error code: 1010`, que se lee como un fallo de
/// autenticación y no lo es.
const AGENTE: &str = concat!("ore-s3/", env!("CARGO_PKG_VERSION"));

/// **Un cliente para todo el proceso**, y es la mitad del tiempo: medido en
/// F1, la primera petición a un bucket cuesta ~1,3 s (TLS) y las siguientes
/// ~100 ms si la conexión se reutiliza. Con un cliente por petición, catalogar
/// el bucket de F1 tardaba 14 s.
fn agente() -> Result<ureq::Agent, String> {
    static AGENTE_: std::sync::OnceLock<Result<ureq::Agent, String>> = std::sync::OnceLock::new();
    AGENTE_
        .get_or_init(|| {
            let tls = native_tls::TlsConnector::new()
                .map_err(|e| format!("no se pudo abrir el TLS de la plataforma: {e}"))?;
            // Sin redirecciones: un bucket de otra región contesta `301` con la
            // suya en `x-amz-bucket-region`, y seguirlo a ciegas perdería la
            // firma y el motivo.
            Ok(ureq::AgentBuilder::new()
                .tls_connector(std::sync::Arc::new(tls))
                .redirects(0)
                .timeout(std::time::Duration::from_secs(60))
                .build())
        })
        .clone()
}

/// **Un cliente para bajar ficheros enteros** (0046 E8·2): sin plazo total
/// —un fichero de gigas tarda lo que tarda— y con uno de inactividad, que es
/// lo que distingue un flujo lento de uno muerto. Con las conexiones vivas que
/// pidan los hilos que bajan en paralelo.
fn agente_de_flujo() -> Result<ureq::Agent, String> {
    static A: std::sync::OnceLock<Result<ureq::Agent, String>> = std::sync::OnceLock::new();
    A.get_or_init(|| {
        let tls = native_tls::TlsConnector::new()
            .map_err(|e| format!("no se pudo abrir el TLS de la plataforma: {e}"))?;
        Ok(ureq::AgentBuilder::new()
            .tls_connector(std::sync::Arc::new(tls))
            .redirects(0)
            .max_idle_connections(128)
            .max_idle_connections_per_host(128)
            .timeout_connect(std::time::Duration::from_secs(30))
            .timeout_read(std::time::Duration::from_secs(60))
            .build())
    })
    .clone()
}

static PETICIONES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Cuántas peticiones y cuántos bytes de cuerpo lleva el proceso: lo que un
/// catálogo le cuesta al bucket del cliente, que es una cifra que se dice.
pub fn contadores() -> (usize, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    (PETICIONES.load(Relaxed), BYTES.load(Relaxed))
}

/// **Una petición firmada.** `consulta` en claro —se codifica aquí, una vez,
/// para la URL y para la firma—; `cabeceras` además de las de SigV4. Solo
/// falla por transporte: todo estado HTTP vuelve como [`Respuesta`].
pub fn pedir(
    b: &Bucket,
    metodo: &str,
    clave: Option<&str>,
    consulta: &[(&str, String)],
    cabeceras: Vec<(String, String)>,
) -> Result<Respuesta, String> {
    let resp = enviar(b, metodo, clave, consulta, cabeceras)?;
    respuesta(b, metodo, resp)
}

/// La petición firmada y enviada; el cuerpo, sin leer.
fn enviar(
    b: &Bucket,
    metodo: &str,
    clave: Option<&str>,
    consulta: &[(&str, String)],
    cabeceras: Vec<(String, String)>,
) -> Result<ureq::Response, String> {
    enviar_por(&agente()?, b, metodo, clave, consulta, cabeceras)
}

fn enviar_por(
    agente: &ureq::Agent,
    b: &Bucket,
    metodo: &str,
    clave: Option<&str>,
    consulta: &[(&str, String)],
    cabeceras: Vec<(String, String)>,
) -> Result<ureq::Response, String> {
    let ruta = b.ruta(clave);
    let mut ps: Vec<(String, String)> = consulta
        .iter()
        .map(|(k, v)| (firma::uri(k), firma::uri(v)))
        .collect();
    ps.sort();
    let canonica: String = ps
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let vacio = hex(&sha256(b""));
    let firmadas = firma::firmar(
        &b.credencial,
        &b.region,
        &b.host(),
        metodo,
        &ruta,
        &canonica,
        cabeceras,
        &vacio,
    );
    let url = if canonica.is_empty() {
        format!("{}{ruta}", b.endpoint.trim_end_matches('/'))
    } else {
        format!("{}{ruta}?{canonica}", b.endpoint.trim_end_matches('/'))
    };
    let mut r = agente.request(metodo, &url).set("user-agent", AGENTE);
    for (k, v) in &firmadas {
        if k != "host" {
            r = r.set(k, v);
        }
    }
    match r.call() {
        Ok(x) => Ok(x),
        Err(ureq::Error::Status(_, x)) => Ok(x),
        Err(e) => Err(format!("{metodo} {}: {e}", b.host())),
    }
}

/// Una respuesta entera, con su cuerpo.
fn respuesta(b: &Bucket, metodo: &str, resp: ureq::Response) -> Result<Respuesta, String> {
    let estado = resp.status();
    let cabeceras: Vec<(String, String)> = resp
        .headers_names()
        .into_iter()
        .filter_map(|n| resp.header(&n).map(|v| (n.clone(), v.to_string())))
        .collect();
    let mut cuerpo = Vec::new();
    use std::io::Read as _;
    resp.into_reader()
        .take(512 * 1024 * 1024)
        .read_to_end(&mut cuerpo)
        .map_err(|e| format!("{metodo} {}: el cuerpo no se pudo leer: {e}", b.host()))?;
    PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    BYTES.fetch_add(cuerpo.len(), std::sync::atomic::Ordering::Relaxed);
    Ok(Respuesta {
        estado,
        cabeceras,
        cuerpo,
    })
}

// Un objeto de un listado, una versión y lo que dice al abrirla: los de
// cualquier origen de objetos (ADR 0061 O0·1).
pub use ore_objetos::{Abierto, Objeto, Version};

/// Una página de `ListObjectsV2`.
#[derive(Debug, Clone, Default)]
pub struct Pagina {
    pub objetos: Vec<Objeto>,
    /// Con delimitador: los «directorios» de este nivel.
    pub prefijos: Vec<String>,
    pub siguiente: Option<String>,
}

/// `ListObjectsV2`, una página. `Err` lleva la [`Respuesta`] entera: un `403`
/// aquí es la falta de `s3:ListBucket`, y hay que poder decirlo.
pub fn listar(
    b: &Bucket,
    prefijo: &str,
    delimitador: Option<&str>,
    continuacion: Option<&str>,
    maximo: Option<u32>,
) -> Result<Pagina, Result<Respuesta, String>> {
    let mut q: Vec<(&str, String)> = vec![("list-type", "2".into()), ("prefix", prefijo.into())];
    if let Some(d) = delimitador {
        q.push(("delimiter", d.into()));
    }
    if let Some(c) = continuacion {
        q.push(("continuation-token", c.into()));
    }
    if let Some(m) = maximo {
        q.push(("max-keys", m.to_string()));
    }
    let r = pedir(b, "GET", None, &q, Vec::new()).map_err(Err)?;
    if !r.ok() {
        return Err(Ok(r));
    }
    Ok(leer_pagina(&String::from_utf8_lossy(&r.cuerpo)))
}

/// El XML de `ListObjectsV2`, leído por sus etiquetas. No entra un analizador
/// de XML para esto: lo que hace falta está entre etiquetas conocidas y sin
/// anidar, y las entidades se deshacen.
pub fn leer_pagina(xml: &str) -> Pagina {
    let mut p = Pagina::default();
    for bloque in xml.split("<Contents>").skip(1) {
        let bloque = bloque.split("</Contents>").next().unwrap_or("");
        let Some(clave) = etiqueta(bloque, "Key") else {
            continue;
        };
        p.objetos.push(Objeto {
            clave,
            tamano: etiqueta(bloque, "Size")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            etag: etiqueta(bloque, "ETag").unwrap_or_default(),
            modificado: etiqueta(bloque, "LastModified").unwrap_or_default(),
        });
    }
    for bloque in xml.split("<CommonPrefixes>").skip(1) {
        if let Some(pr) = etiqueta(bloque, "Prefix") {
            p.prefijos.push(pr);
        }
    }
    if etiqueta(xml, "IsTruncated").as_deref() == Some("true") {
        p.siguiente = etiqueta(xml, "NextContinuationToken");
    }
    p
}

/// Una página de `ListObjectVersions`: las versiones y las marcas, y por dónde
/// seguir.
pub fn leer_pagina_de_versiones(xml: &str) -> (Vec<Version>, Option<(String, String)>) {
    let mut out = Vec::new();
    for (abre, cierra, marca) in [
        ("<Version>", "</Version>", false),
        ("<DeleteMarker>", "</DeleteMarker>", true),
    ] {
        for bloque in xml.split(abre).skip(1) {
            let bloque = bloque.split(cierra).next().unwrap_or("");
            let Some(clave) = etiqueta(bloque, "Key") else {
                continue;
            };
            out.push(Version {
                clave,
                version: etiqueta(bloque, "VersionId").unwrap_or_else(|| "null".into()),
                actual: etiqueta(bloque, "IsLatest").as_deref() == Some("true"),
                marca,
                tamano: etiqueta(bloque, "Size")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                etag: etiqueta(bloque, "ETag").unwrap_or_default(),
                modificado: etiqueta(bloque, "LastModified").unwrap_or_default(),
            });
        }
    }
    let siguiente = (etiqueta(xml, "IsTruncated").as_deref() == Some("true")).then(|| {
        (
            etiqueta(xml, "NextKeyMarker").unwrap_or_default(),
            etiqueta(xml, "NextVersionIdMarker").unwrap_or_default(),
        )
    });
    (out, siguiente)
}

/// **Todas las versiones y marcas bajo un prefijo**, página a página
/// (`s3:ListBucketVersions`, que la política de lectura ya pide).
pub fn listar_versiones(
    b: &Bucket,
    prefijo: &str,
) -> Result<Vec<Version>, Result<Respuesta, String>> {
    let mut out = Vec::new();
    let mut desde: Option<(String, String)> = None;
    loop {
        let mut q: Vec<(&str, String)> =
            vec![("versions", String::new()), ("prefix", prefijo.into())];
        if let Some((k, v)) = &desde {
            q.push(("key-marker", k.clone()));
            if !v.is_empty() {
                q.push(("version-id-marker", v.clone()));
            }
        }
        let r = pedir(b, "GET", None, &q, Vec::new()).map_err(Err)?;
        if !r.ok() {
            return Err(Ok(r));
        }
        let (vs, siguiente) = leer_pagina_de_versiones(&String::from_utf8_lossy(&r.cuerpo));
        out.extend(vs);
        match siguiente {
            Some(s) if Some(&s) != desde.as_ref() => desde = Some(s),
            _ => return Ok(out),
        }
    }
}

/// El `HEAD` de UNA versión, con su checksum (`x-amz-checksum-crc64nvme`).
pub fn cabeza_de(b: &Bucket, clave: &str, version: &str) -> Result<Respuesta, String> {
    pedir(
        b,
        "HEAD",
        Some(clave),
        &[("versionId", version.into())],
        vec![("x-amz-checksum-mode".into(), "ENABLED".into())],
    )
}

/// Todo lo que hay bajo un prefijo, página a página.
pub fn listar_todo(b: &Bucket, prefijo: &str) -> Result<Vec<Objeto>, Result<Respuesta, String>> {
    let mut out = Vec::new();
    let mut siguiente: Option<String> = None;
    loop {
        let p = listar(b, prefijo, None, siguiente.as_deref(), None)?;
        out.extend(p.objetos);
        match p.siguiente {
            Some(s) => siguiente = Some(s),
            None => return Ok(out),
        }
    }
}

/// `HEAD` con la huella: `x-amz-checksum-crc64nvme` en la respuesta, si S3 la
/// calculó (por defecto desde 2025, medido en F1).
pub fn cabeza(b: &Bucket, clave: &str) -> Result<Respuesta, String> {
    pedir(
        b,
        "HEAD",
        Some(clave),
        &[],
        vec![("x-amz-checksum-mode".into(), "ENABLED".into())],
    )
}

/// **Un objeto entero, en flujo, y sólo si sigue siendo el listado** (0046
/// E6): `If-Match` con su ETag. S3 es consistente por clave y no entre claves,
/// así que una tabla de varios ficheros se lee fijando cada uno a lo que el
/// listado dijo; si alguno cambió entre medias, `412` y la lectura se para en
/// vez de mezclar dos versiones. El cuerpo no se guarda: se lee según llega.
pub fn abrir(
    b: &Bucket,
    clave: &str,
    etag: &str,
) -> Result<Box<dyn std::io::Read + Send>, Respuesta> {
    let resp = enviar(
        b,
        "GET",
        Some(clave),
        &[],
        vec![("if-match".into(), etag.into())],
    )
    .map_err(|e| Respuesta {
        estado: 0,
        cabeceras: Vec::new(),
        cuerpo: e.into_bytes(),
    })?;
    if !(200..300).contains(&resp.status()) {
        return Err(respuesta(b, "GET", resp).unwrap_or_else(|e| Respuesta {
            estado: 0,
            cabeceras: Vec::new(),
            cuerpo: e.into_bytes(),
        }));
    }
    PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(Box::new(resp.into_reader()))
}

/// **Una versión entera, en flujo** (0046 E8·2): lo que una colección
/// mantenida baja, fijado a la versión que su manifiesto dice —no a lo que hay
/// ahora—, así que un ítem retirado se sigue copiando mientras su versión
/// exista. Sin plazo total (ver [`agente_de_flujo`]).
pub fn abrir_version(
    b: &Bucket,
    clave: &str,
    version: &str,
) -> Result<(Box<dyn std::io::Read + Send>, Abierto), Respuesta> {
    let fallo = |e: String| Respuesta {
        estado: 0,
        cabeceras: Vec::new(),
        cuerpo: e.into_bytes(),
    };
    let resp = enviar_por(
        &agente_de_flujo().map_err(fallo)?,
        b,
        "GET",
        Some(clave),
        &[("versionId", version.into())],
        vec![("x-amz-checksum-mode".into(), "ENABLED".into())],
    )
    .map_err(fallo)?;
    if !(200..300).contains(&resp.status()) {
        return Err(respuesta(b, "GET", resp).unwrap_or_else(fallo));
    }
    PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let a = Abierto {
        tamano: resp.header("content-length").and_then(|v| v.parse().ok()),
        tipo: resp.header("content-type").map(String::from),
        // Lo que S3 dice al abrirla (`x-amz-checksum-mode: ENABLED`), como la
        // escribe la colección.
        huella: resp
            .header("x-amz-checksum-crc64nvme")
            .map(|c| format!("crc64nvme:{c}")),
    };
    Ok((Box::new(resp.into_reader()), a))
}

/// Lo que S3 contestó a una lectura que salió bien: el estado (`200`/`206`),
/// sus cabeceras y el cuerpo **sin leer**.
pub struct Leido {
    pub estado: u16,
    pub cabeceras: Vec<(String, String)>,
    pub lector: Box<dyn std::io::Read + Send>,
}

impl Leido {
    pub fn cabecera(&self, k: &str) -> Option<&str> {
        self.cabeceras
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
}

/// **Una versión, fijada dos veces, entera o por rango, en flujo** (0049 B3·1):
/// lo que la puerta de lectura sirve de una colección virtual.
///
/// - `versionId` si hay versión —**también `null`**, la de un objeto anterior
///   al versionado (medido en B3·0)—;
/// - `If-Match` con el ETag que el manifiesto anotó: si la versión ya no es la
///   que se listó, `412` (medido en B3·0), y quien lee lo dice como
///   `media/cambiado` en vez de servir otros bytes;
/// - `rango`, la cabecera `Range` tal cual (`bytes=0-1023`, `bytes=-8`).
///
/// Sin plazo total (un vídeo tarda lo que tarda), con el de inactividad de
/// [`agente_de_flujo`]. Todo lo que no es 2xx vuelve como [`Respuesta`], con
/// su código de AWS.
pub fn leer_fijado(
    b: &Bucket,
    clave: &str,
    version: &str,
    etag: &str,
    rango: Option<&str>,
) -> Result<Leido, Respuesta> {
    let fallo = |e: String| Respuesta {
        estado: 0,
        cabeceras: Vec::new(),
        cuerpo: e.into_bytes(),
    };
    let consulta: Vec<(&str, String)> = if version.is_empty() {
        Vec::new()
    } else {
        vec![("versionId", version.to_string())]
    };
    let mut cabeceras: Vec<(String, String)> = Vec::new();
    if !etag.is_empty() {
        cabeceras.push(("if-match".into(), etag.to_string()));
    }
    if let Some(r) = rango {
        cabeceras.push(("range".into(), r.to_string()));
    }
    let resp = enviar_por(
        &agente_de_flujo().map_err(fallo)?,
        b,
        "GET",
        Some(clave),
        &consulta,
        cabeceras,
    )
    .map_err(fallo)?;
    if !(200..300).contains(&resp.status()) {
        return Err(respuesta(b, "GET", resp).unwrap_or_else(fallo));
    }
    PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let estado = resp.status();
    let cabeceras: Vec<(String, String)> = resp
        .headers_names()
        .into_iter()
        .filter_map(|n| resp.header(&n).map(|v| (n.clone(), v.to_string())))
        .collect();
    Ok(Leido {
        estado,
        cabeceras,
        lector: Box::new(resp.into_reader()),
    })
}

/// Un rango de bytes **de la versión que el listado dijo** (`If-Match`).
pub fn rango_de(b: &Bucket, clave: &str, rango: &str, etag: &str) -> Result<Respuesta, String> {
    pedir(
        b,
        "GET",
        Some(clave),
        &[],
        vec![
            ("range".into(), format!("bytes={rango}")),
            ("if-match".into(), etag.into()),
        ],
    )
}

/// Un rango de bytes: `bytes=0-15`, o el sufijo `bytes=-8`.
pub fn rango(b: &Bucket, clave: &str, rango: &str) -> Result<Respuesta, String> {
    pedir(
        b,
        "GET",
        Some(clave),
        &[],
        vec![("range".into(), format!("bytes={rango}"))],
    )
}

/// El texto de la primera `<t>…</t>`, con las entidades de XML deshechas.
pub fn etiqueta(xml: &str, t: &str) -> Option<String> {
    let abre = format!("<{t}>");
    let cierra = format!("</{t}>");
    let i = xml.find(&abre)? + abre.len();
    let j = xml[i..].find(&cierra)? + i;
    Some(entidades(&xml[i..j]))
}

fn entidades(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#34;", "\"")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **La página de versiones** como la da S3 (el experimento de E7, en
    /// pequeño): una versión vieja y la vigente de `a.pdf`, y la marca que
    /// dejó borrar `b.pdf`, con su versión anterior.
    #[test]
    fn una_pagina_de_versiones_trae_versiones_y_marcas() {
        let xml = "<ListVersionsResult><IsTruncated>true</IsTruncated>            <NextKeyMarker>b.pdf</NextKeyMarker><NextVersionIdMarker>s1</NextVersionIdMarker>            <Version><Key>a.pdf</Key><VersionId>6G</VersionId><IsLatest>true</IsLatest>            <LastModified>2026-09-29T10:59:07.000Z</LastModified><ETag>&quot;e2&quot;</ETag><Size>35030</Size></Version>            <Version><Key>a.pdf</Key><VersionId>.N</VersionId><IsLatest>false</IsLatest>            <LastModified>2026-09-29T10:57:56.000Z</LastModified><ETag>&quot;e1&quot;</ETag><Size>34479</Size></Version>            <DeleteMarker><Key>b.pdf</Key><VersionId>wk</VersionId><IsLatest>true</IsLatest>            <LastModified>2026-09-29T11:00:07.000Z</LastModified></DeleteMarker>            <Version><Key>b.pdf</Key><VersionId>s1</VersionId><IsLatest>false</IsLatest>            <LastModified>2026-09-29T10:57:56.000Z</LastModified><ETag>&quot;e3&quot;</ETag><Size>35030</Size></Version>            </ListVersionsResult>";
        let (vs, siguiente) = leer_pagina_de_versiones(xml);
        assert_eq!(siguiente, Some(("b.pdf".into(), "s1".into())));
        assert_eq!(vs.len(), 4);
        let a = vs.iter().find(|v| v.clave == "a.pdf" && v.actual).unwrap();
        assert_eq!(
            (a.version.as_str(), a.etag.as_str(), a.tamano),
            ("6G", "\"e2\"", 35030)
        );
        let m = vs.iter().find(|v| v.marca).unwrap();
        assert_eq!((m.clave.as_str(), m.actual), ("b.pdf", true));
    }

    fn b(en_ruta: bool) -> Bucket {
        Bucket {
            endpoint: "https://x".into(),
            bucket: "cubo".into(),
            region: "eu-north-1".into(),
            en_ruta,
            credencial: Credencial {
                clave: "AK".into(),
                secreto: "SK".into(),
                token: None,
            },
        }
    }

    /// Medido en F1: `Nueva carpeta/Foto Portada 2026.JPG`. Cada segmento se
    /// codifica una vez; las barras no.
    #[test]
    fn la_ruta_codifica_por_segmento() {
        assert_eq!(
            b(false).ruta(Some("Nueva carpeta/Foto Portada 2026.JPG")),
            "/Nueva%20carpeta/Foto%20Portada%202026.JPG"
        );
        assert_eq!(b(true).ruta(Some("a/b=c")), "/cubo/a/b%3Dc");
        assert_eq!(b(false).ruta(None), "/");
    }

    #[test]
    fn un_listado_se_lee_por_sus_etiquetas() {
        let xml = r#"<?xml version="1.0"?><ListBucketResult><IsTruncated>true</IsTruncated>
<Contents><Key>Nueva carpeta/a &amp; b.csv</Key><LastModified>2026-09-28T10:00:00.000Z</LastModified><ETag>&quot;abc-4&quot;</ETag><Size>2613</Size></Contents>
<Contents><Key>x.pdf</Key><LastModified>2026-09-28T10:00:01.000Z</LastModified><ETag>"d"</ETag><Size>7</Size></Contents>
<CommonPrefixes><Prefix>fotos/</Prefix></CommonPrefixes>
<NextContinuationToken>tok</NextContinuationToken></ListBucketResult>"#;
        let p = leer_pagina(xml);
        assert_eq!(p.objetos.len(), 2);
        assert_eq!(p.objetos[0].clave, "Nueva carpeta/a & b.csv");
        assert_eq!(p.objetos[0].etag, "\"abc-4\"");
        assert_eq!(p.objetos[0].tamano, 2613);
        assert_eq!(p.prefijos, ["fotos/"]);
        assert_eq!(p.siguiente.as_deref(), Some("tok"));
        let sin =
            leer_pagina("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>");
        assert!(sin.siguiente.is_none());
    }

    #[test]
    fn el_error_de_aws_se_lee() {
        let r = Respuesta {
            estado: 403,
            cabeceras: vec![],
            cuerpo: b"<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>"
                .to_vec(),
        };
        assert_eq!(r.motivo(), "403 AccessDenied: Access Denied");
    }
}
