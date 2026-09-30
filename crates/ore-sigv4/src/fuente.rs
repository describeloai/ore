//! **La coordenada de un bucket**: `s3://<bucket>[/<prefijo>]?region=…`.
//!
//! La credencial viaja en la cadena de consulta —`access_key_id`,
//! `secret_access_key` y, si es temporal, `session_token`— porque es la forma
//! en que `ore source add` separa el secreto de la conexión: la URL entera va
//! al cofre y a una variable de entorno, nunca al árbol, y el alta la tapa por
//! el NOMBRE de esos parámetros (0046 F0). Sin ellos, las del entorno
//! (`AWS_ACCESS_KEY_ID`…), que es como se prueba en local.
//!
//! `endpoint=` apunta a un S3 compatible (R2, MinIO): el bucket va entonces en
//! la ruta.

use crate::{Bucket, Credencial};

/// El bucket y el prefijo que la fuente abarca (sin barra inicial; con la
/// final si no es vacío).
pub struct Fuente {
    pub bucket: Bucket,
    pub prefijo: String,
}

/// `%XX` → el byte. `+` NO es un espacio: eso es de los formularios, y un
/// secreto de AWS lleva `+` y `/`.
fn descodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn leer(url: &str) -> Result<Fuente, String> {
    let resto = url
        .trim()
        .strip_prefix("s3://")
        .ok_or("la URL de un bucket es `s3://<bucket>[/<prefijo>]?region=…`")?;
    let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let (bucket, prefijo) = camino.split_once('/').unwrap_or((camino, ""));
    if bucket.is_empty() {
        return Err("la URL no nombra bucket: `s3://<bucket>`".into());
    }
    let mut ps: Vec<(String, String)> = Vec::new();
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        ps.push((descodificar(k), descodificar(v)));
    }
    let param = |k: &str| {
        ps.iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let entorno = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    // ⛔ Un rol (0046 E9b) no es una credencial: se canjea ANTES, fuera de aquí
    //   (`ore-sts`), porque esto no habla por la red y el firmante tampoco.
    if param("role_arn").is_some() && param("access_key_id").is_none() {
        return Err(
            "la fuente es un rol (`role_arn`): se canjea antes por una credencial temporal              (`ore-sts`, `ore-asumir-rol`)"
                .into(),
        );
    }
    let clave = param("access_key_id")
        .or_else(|| entorno("AWS_ACCESS_KEY_ID"))
        .ok_or(
            "falta la credencial: `access_key_id` y `secret_access_key` en la URL (o \
             `AWS_ACCESS_KEY_ID` en el entorno)",
        )?;
    let secreto = param("secret_access_key")
        .or_else(|| entorno("AWS_SECRET_ACCESS_KEY"))
        .ok_or("falta `secret_access_key` en la URL (o `AWS_SECRET_ACCESS_KEY` en el entorno)")?;
    let token = param("session_token").or_else(|| entorno("AWS_SESSION_TOKEN"));
    let region = param("region")
        .or_else(|| entorno("AWS_REGION"))
        .or_else(|| entorno("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|| "us-east-1".into());
    let credencial = Credencial {
        clave,
        secreto,
        token,
    };
    let bucket = match param("endpoint") {
        Some(e) => Bucket {
            endpoint: e.trim_end_matches('/').to_string(),
            bucket: bucket.to_string(),
            region,
            en_ruta: true,
            credencial,
        },
        None => Bucket::de_aws(bucket, &region, credencial),
    };
    let prefijo = descodificar(prefijo);
    let prefijo = prefijo.trim_start_matches('/');
    let prefijo = if prefijo.is_empty() || prefijo.ends_with('/') {
        prefijo.to_string()
    } else {
        format!("{prefijo}/")
    };
    Ok(Fuente { bucket, prefijo })
}

/// La URL sin credencial: la que se puede enseñar (en `explorar`, en un
/// mensaje). La región se queda: no es un secreto y hace falta.
pub fn publica(f: &Fuente, prefijo: &str) -> String {
    format!(
        "s3://{}/{prefijo}?region={}",
        f.bucket.bucket, f.bucket.region
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_url_da_bucket_prefijo_region_y_credencial() {
        let f = leer(
            "s3://cubo/Nueva%20carpeta?region=eu-north-1&access_key_id=AK&secret_access_key=a%2Fb%2Bc",
        )
        .unwrap();
        assert_eq!(f.bucket.bucket, "cubo");
        assert_eq!(f.prefijo, "Nueva carpeta/");
        assert_eq!(f.bucket.region, "eu-north-1");
        assert_eq!(f.bucket.credencial.clave, "AK");
        assert_eq!(f.bucket.credencial.secreto, "a/b+c");
        assert_eq!(
            f.bucket.endpoint,
            "https://cubo.s3.eu-north-1.amazonaws.com"
        );
    }

    #[test]
    fn un_mas_en_el_secreto_es_un_mas() {
        let f = leer("s3://c?access_key_id=a&secret_access_key=x+y/z").unwrap();
        assert_eq!(f.bucket.credencial.secreto, "x+y/z");
    }

    #[test]
    fn un_s3_compatible_lleva_el_bucket_en_la_ruta() {
        let f = leer("s3://lago?endpoint=https://r2.example&access_key_id=a&secret_access_key=b")
            .unwrap();
        assert!(f.bucket.en_ruta);
        assert_eq!(f.bucket.ruta(Some("x")), "/lago/x");
        assert_eq!(f.prefijo, "");
    }

    #[test]
    fn un_rol_sin_canjear_no_se_confunde_con_el_entorno() {
        // Sin esto, un `role_arn` caería a `AWS_ACCESS_KEY_ID` del entorno y
        // firmaría con OTRA credencial sin decirlo.
        let e = leer("s3://cubo?region=x&role_arn=arn:aws:iam::123456789012:role/r")
            .err()
            .unwrap();
        assert!(e.contains("role_arn") && e.contains("ore-sts"), "{e}");
    }

    #[test]
    fn lo_publico_no_lleva_la_credencial() {
        let f = leer("s3://cubo/?region=eu-north-1&access_key_id=AK&secret_access_key=SK").unwrap();
        let p = publica(&f, "fotos/");
        assert!(!p.contains("AK") && !p.contains("SK"), "{p}");
    }
}
