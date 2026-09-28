//! **SigV4**, la firma de S3. Viene de `ore-store/src/r2.rs`, donde se midió
//! contra un R2 de verdad (ADR 0015), y aquí gana la credencial temporal
//! (`x-amz-security-token`) y la fecha inyectable para probarla contra el
//! ejemplo oficial de AWS.

use sha2::{Digest, Sha256};

/// Con qué se firma. `token` es el de una credencial temporal (STS); sin él,
/// una clave de acceso de un usuario IAM.
#[derive(Clone)]
pub struct Credencial {
    pub clave: String,
    pub secreto: String,
    pub token: Option<String>,
}

/// El secreto no se imprime nunca, ni en un `{:?}` de depuración.
impl std::fmt::Debug for Credencial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credencial")
            .field("clave", &self.clave)
            .field("secreto", &"***")
            .field("token", &self.token.as_ref().map(|_| "***"))
            .finish()
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn sha256(b: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b);
    h.finalize().to_vec()
}

/// HMAC-SHA256, RFC 2104, sobre el `sha2` que el árbol ya enlaza.
///
/// No es una primitiva: es una construcción de seis líneas sobre una, con
/// vectores oficiales. Y la crate `hmac` arrastraba `digest 0.10` entera al
/// lado de la `0.11` del árbol (`dependencias.rs` lo vio).
pub fn hmac(clave: &[u8], datos: &str) -> Vec<u8> {
    const BLOQUE: usize = 64;
    let mut k = [0u8; BLOQUE];
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

/// RFC 3986, que es lo que SigV4 exige de cada segmento, clave y valor — **y
/// `/` se codifica** dentro de un valor de la consulta. Costó un `403`
/// averiguarlo: la cadena firmada y la enviada no coincidían.
pub fn uri(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Base64 estándar, que es como S3 quiere un checksum.
pub fn base64(b: &[u8]) -> String {
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

/// `YYYYMMDDTHHMMSSZ` y `YYYYMMDD`, del reloj del sistema. Lo único no
/// determinista de la firma, y tiene que serlo: SigV4 la fecha.
pub fn ahora() -> (String, String) {
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

/// **La firma de una petición, ahora.** `ruta` ya canónica (cada segmento
/// codificado); `consulta` ya canónica (codificada y ordenada). Devuelve las
/// cabeceras a enviar, `authorization` incluida.
#[allow(clippy::too_many_arguments)]
pub fn firmar(
    c: &Credencial,
    region: &str,
    host: &str,
    metodo: &str,
    ruta: &str,
    consulta: &str,
    cabeceras: Vec<(String, String)>,
    hash_cuerpo: &str,
) -> Vec<(String, String)> {
    let (marca, fecha) = ahora();
    firmar_en(
        c,
        region,
        host,
        metodo,
        ruta,
        consulta,
        cabeceras,
        hash_cuerpo,
        &marca,
        &fecha,
    )
}

/// La misma firma en un instante dado: es lo que deja probarla contra el
/// ejemplo oficial.
#[allow(clippy::too_many_arguments)]
pub fn firmar_en(
    c: &Credencial,
    region: &str,
    host: &str,
    metodo: &str,
    ruta: &str,
    consulta: &str,
    mut cabeceras: Vec<(String, String)>,
    hash_cuerpo: &str,
    marca: &str,
    fecha: &str,
) -> Vec<(String, String)> {
    for (k, _) in cabeceras.iter_mut() {
        *k = k.to_ascii_lowercase();
    }
    cabeceras.push(("host".into(), host.to_string()));
    cabeceras.push(("x-amz-content-sha256".into(), hash_cuerpo.to_string()));
    cabeceras.push(("x-amz-date".into(), marca.to_string()));
    if let Some(t) = &c.token {
        cabeceras.push(("x-amz-security-token".into(), t.clone()));
    }
    cabeceras.sort_by(|a, b| a.0.cmp(&b.0));

    let lista = cabeceras
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonicas: String = cabeceras
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let peticion = format!("{metodo}\n{ruta}\n{consulta}\n{canonicas}\n{lista}\n{hash_cuerpo}");
    let ambito = format!("{fecha}/{region}/s3/aws4_request");
    let por_firmar = format!(
        "AWS4-HMAC-SHA256\n{marca}\n{ambito}\n{}",
        hex(&sha256(peticion.as_bytes()))
    );
    let k = hmac(format!("AWS4{}", c.secreto).as_bytes(), fecha);
    let k = hmac(&k, region);
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// El vector oficial de AWS para la derivación de la clave de firma.
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

    /// **El ejemplo de S3 de la documentación de SigV4**: `GET /test.txt` con
    /// `Range: bytes=0-9` sobre `examplebucket`, el 24-05-2013. Es la firma
    /// entera —ruta, cabeceras, ámbito—, no solo la clave.
    #[test]
    fn la_firma_es_la_del_ejemplo_de_s3() {
        let c = Credencial {
            clave: "AKIAIOSFODNN7EXAMPLE".into(),
            secreto: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            token: None,
        };
        let cab = firmar_en(
            &c,
            "us-east-1",
            "examplebucket.s3.amazonaws.com",
            "GET",
            "/test.txt",
            "",
            vec![("Range".into(), "bytes=0-9".into())],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "20130524T000000Z",
            "20130524",
        );
        let auth = &cab.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn la_marca_tiene_la_forma_que_sigv4_exige() {
        let (marca, fecha) = ahora();
        assert_eq!(fecha.len(), 8, "{fecha}");
        assert_eq!(marca.len(), 16, "{marca}");
        assert!(marca.starts_with(&fecha) && marca.ends_with('Z'), "{marca}");
    }

    #[test]
    fn el_secreto_no_se_imprime() {
        let c = Credencial {
            clave: "AK".into(),
            secreto: "muy-secreto".into(),
            token: Some("tok-secreto".into()),
        };
        let s = format!("{c:?}");
        assert!(
            !s.contains("muy-secreto") && !s.contains("tok-secreto"),
            "{s}"
        );
    }
}
