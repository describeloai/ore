//! **La URL V4 de GCS** (`GOOG4-RSA-SHA256`), sin red: la petición canónica y
//! el texto que se firma. Quien firma de verdad es IAM (`signBlob`, [`crate::Gcs`]):
//! la organización prohíbe las claves de cuenta, así que no hay clave en ningún
//! sitio. Lo mismo que firma el lago (`ore-store`), probado aquí contra los
//! vectores de conformidad de Google (`googleapis/conformance-tests`,
//! `storage/v1/v4_signatures.json`).

use ore_sigv4::firma::{hex, sha256, uri};

/// Lo que se firma de una lectura.
pub struct Pedida<'a> {
    /// `storage.googleapis.com`.
    pub host: &'a str,
    pub bucket: &'a str,
    pub clave: &'a str,
    /// El correo de la cuenta que firma.
    pub firmante: &'a str,
    /// `20190201T090000Z` y `20190201`.
    pub marca: &'a str,
    pub fecha: &'a str,
    pub segundos: u64,
    /// Lo demás de la consulta: `generation`, `response-content-type`…
    pub extra: &'a [(&'a str, &'a str)],
}

/// La ruta, la consulta y la petición canónica.
pub fn canonica(p: &Pedida<'_>) -> (String, String, String) {
    let alcance = format!("{}/auto/storage/goog4_request", p.fecha);
    let mut ps: Vec<(String, String)> = vec![
        ("X-Goog-Algorithm".into(), "GOOG4-RSA-SHA256".into()),
        (
            "X-Goog-Credential".into(),
            format!("{}/{alcance}", p.firmante),
        ),
        ("X-Goog-Date".into(), p.marca.to_string()),
        ("X-Goog-Expires".into(), p.segundos.to_string()),
        ("X-Goog-SignedHeaders".into(), "host".into()),
    ];
    ps.extend(p.extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let mut ps: Vec<(String, String)> = ps.into_iter().map(|(k, v)| (uri(&k), uri(&v))).collect();
    ps.sort();
    let consulta = ps
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let ruta = std::iter::once(p.bucket)
        .chain(p.clave.split('/'))
        .map(uri)
        .fold(String::new(), |r, s| r + "/" + &s);
    let can = format!(
        "GET\n{ruta}\n{consulta}\nhost:{}\n\nhost\nUNSIGNED-PAYLOAD",
        p.host
    );
    (ruta, consulta, can)
}

/// El texto que se firma: el algoritmo, el instante, el alcance y el hash de
/// la petición canónica.
pub fn por_firmar(p: &Pedida<'_>, canonica: &str) -> String {
    format!(
        "GOOG4-RSA-SHA256\n{}\n{}/auto/storage/goog4_request\n{}",
        p.marca,
        p.fecha,
        hex(&sha256(canonica.as_bytes()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUENTA: &str = "test-iam-credentials@dummy-project-id.iam.gserviceaccount.com";

    fn simple(marca: &str, fecha: &str, segundos: u64) -> Pedida<'static> {
        Pedida {
            host: "storage.googleapis.com",
            bucket: "test-bucket",
            clave: "test-object",
            firmante: CUENTA,
            marca: Box::leak(marca.to_string().into_boxed_str()),
            fecha: Box::leak(fecha.to_string().into_boxed_str()),
            segundos,
            extra: &[],
        }
    }

    /// Los vectores de Google: «Simple GET» y «Vary expiration and timestamp».
    /// El hash de la petición canónica es el que el vector dice en su texto a
    /// firmar: si casa, la canónica es la de Google, byte a byte.
    #[test]
    fn la_peticion_canonica_es_la_de_los_vectores_de_google() {
        for (marca, fecha, s, esperado) in [
            (
                "20190201T090000Z",
                "20190201",
                10,
                "00e2fb794ea93d7adb703edaebdd509821fcc7d4f1a79ac5c8d2b394df109320",
            ),
            (
                "20190301T090000Z",
                "20190301",
                20,
                "779f19fdb6fd381390e2d5af04947cf21750277ee3c20e0c97b7e46a1dff8907",
            ),
        ] {
            let p = simple(marca, fecha, s);
            let (ruta, consulta, can) = canonica(&p);
            assert_eq!(ruta, "/test-bucket/test-object");
            assert!(consulta.contains("X-Goog-Credential=test-iam-credentials%40dummy-project-id"));
            assert_eq!(hex(&sha256(can.as_bytes())), esperado, "{can}");
            let t = por_firmar(&p, &can);
            assert!(t.starts_with(&format!("GOOG4-RSA-SHA256\n{marca}\n{fecha}/auto/")));
            assert!(t.ends_with(esperado));
        }
    }

    #[test]
    fn la_generacion_y_la_respuesta_van_dentro_de_la_firma() {
        let p = Pedida {
            extra: &[
                ("generation", "17"),
                ("response-content-type", "application/pdf"),
            ],
            clave: "docs/Nueva carpeta/a b.pdf",
            ..simple("20261009T120000Z", "20261009", 60)
        };
        let (ruta, consulta, _) = canonica(&p);
        assert_eq!(ruta, "/test-bucket/docs/Nueva%20carpeta/a%20b.pdf");
        assert!(consulta.contains("generation=17"), "{consulta}");
        assert!(
            consulta.contains("response-content-type=application%2Fpdf"),
            "{consulta}"
        );
    }
}
