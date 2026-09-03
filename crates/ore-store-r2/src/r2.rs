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

use sha2::{Digest, Sha256};

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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha256(b: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b);
    h.finalize().to_vec()
}

/// HMAC-SHA256, RFC 2104, sobre el `sha2` que el árbol ya enlaza.
///
/// # Por qué esto y no la crate `hmac`
///
/// La regla del proyecto es no reimplementar **primitivas** criptográficas, y
/// por eso `sha2` se enlaza. HMAC no es una primitiva: es una construcción de
/// seis líneas sobre una, completamente especificada y con vectores oficiales
/// —el de AWS está justo abajo, en las pruebas.
///
/// Y enlazarla costaba caro por un motivo que solo se ve mirando el `Cargo.lock`:
/// `hmac 0.12` arrastra `digest 0.10` **entera** al lado de la `0.11` que el
/// árbol ya usa, y con ella `crypto-common`, `generic-array` y `block-buffer`
/// duplicados. `dependencias.rs` lo vio y se puso rojo — que es exactamente
/// para lo que existe.
fn hmac(clave: &[u8], datos: &str) -> Vec<u8> {
    const BLOQUE: usize = 64;
    let mut k = [0u8; BLOQUE];
    // Una clave más larga que el bloque se resume primero; una más corta se
    // rellena con ceros. Las dos cosas las manda el RFC.
    if clave.len() > BLOQUE {
        k[..32].copy_from_slice(&sha256(clave));
    } else {
        k[..clave.len()].copy_from_slice(clave);
    }
    let mut dentro = Sha256::new();
    dentro.update(k.map(|b| b ^ 0x36));
    dentro.update(datos.as_bytes());

    let mut fuera = Sha256::new();
    fuera.update(k.map(|b| b ^ 0x5c));
    fuera.update(dentro.finalize());
    fuera.finalize().to_vec()
}

/// Base64 estándar, que es como S3 quiere el checksum. Son doce líneas y evita
/// una dependencia más en un binario que ya enlaza TLS.
fn base64(b: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for t in b.chunks(3) {
        let n = ((t[0] as u32) << 16)
            | ((*t.get(1).unwrap_or(&0) as u32) << 8)
            | (*t.get(2).unwrap_or(&0) as u32);
        for i in 0..4 {
            if i <= t.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `YYYYMMDDTHHMMSSZ` y `YYYYMMDD`, del reloj del sistema. Es lo único de este
/// programa que no es determinista, y tiene que serlo: SigV4 fecha la firma.
fn ahora() -> (String, String) {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (dias, resto) = ((s / 86_400) as i64, s % 86_400);
    // Del día juliano al calendario civil (Howard Hinnant, `civil_from_days`).
    let z = dias + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let fecha = format!("{y:04}{m:02}{d:02}");
    let hora = format!(
        "{fecha}T{:02}{:02}{:02}Z",
        resto / 3600,
        (resto % 3600) / 60,
        resto % 60
    );
    (hora, fecha)
}

/// La firma de una petición. Cabeceras **ordenadas**, que es lo que exige el
/// esquema: la firma es sobre una forma canónica, como todo lo demás aquí.
fn firmar(
    c: &Cuenta,
    metodo: &str,
    ruta: &str,
    mut cabeceras: Vec<(String, String)>,
    hash_cuerpo: &str,
) -> Vec<(String, String)> {
    let (marca, fecha) = ahora();
    cabeceras.push(("host".into(), c.host()));
    cabeceras.push(("x-amz-content-sha256".into(), hash_cuerpo.to_string()));
    cabeceras.push(("x-amz-date".into(), marca.clone()));
    cabeceras.sort_by(|a, b| a.0.cmp(&b.0));

    let firmadas: Vec<String> = cabeceras.iter().map(|(k, _)| k.clone()).collect();
    let lista = firmadas.join(";");
    let canonicas: String = cabeceras
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();

    let peticion = format!("{metodo}\n{ruta}\n\n{canonicas}\n{lista}\n{hash_cuerpo}");
    let ambito = format!("{fecha}/{}/s3/aws4_request", c.region);
    let por_firmar = format!(
        "AWS4-HMAC-SHA256\n{marca}\n{ambito}\n{}",
        hex(&sha256(peticion.as_bytes()))
    );

    let k = hmac(format!("AWS4{}", c.secreto).as_bytes(), &fecha);
    let k = hmac(&k, &c.region);
    let k = hmac(&k, "s3");
    let k = hmac(&k, "aws4_request");
    let firma = hex(&hmac(&k, &por_firmar));

    cabeceras.push((
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{ambito}, SignedHeaders={lista}, Signature={firma}",
            c.clave
        ),
    ));
    cabeceras
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Los vectores de RFC 4648. Un base64 mal hecho no falla aquí: falla en el
    /// servidor, con un `BadDigest` que parece otra cosa.
    #[test]
    fn el_base64_es_el_de_siempre() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// **El vector oficial de AWS** para la derivación de la clave de firma
    /// (SigV4, *Signature Calculation Examples*). Si esto se mueve, ninguna
    /// petición se firma bien y el error que sale es `SignatureDoesNotMatch`,
    /// que se lee como una credencial mala.
    #[test]
    fn la_clave_de_firma_es_la_del_vector_de_aws() {
        let k = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20150830");
        let k = hmac(&k, "us-east-1");
        let k = hmac(&k, "iam");
        let k = hmac(&k, "aws4_request");
        assert_eq!(
            hex(&k),
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
    }

    /// La fecha sale del reloj y el resto del programa es determinista, así que
    /// esto es lo único que hay que comprobar a mano: que tiene la forma que
    /// SigV4 exige, y que las dos concuerdan.
    #[test]
    fn la_marca_tiene_la_forma_que_sigv4_exige() {
        let (marca, fecha) = ahora();
        assert_eq!(fecha.len(), 8, "{fecha}");
        assert_eq!(marca.len(), 16, "{marca}");
        assert!(marca.starts_with(&fecha) && marca.ends_with('Z'), "{marca}");
        assert!(fecha.starts_with("20"), "{fecha}");
    }
}
