//! **`ore-firmar-s3`: URLs prefirmadas de los ítems de una colección virtual**
//! (0046 E9·3) — el firmante de `ore-serve`, **sin red**. Una virtual no tiene bytes en el lago: se sirven **del origen**, fijados
//! a la versión que el manifiesto dice (`versionId`), con una URL SigV4 que
//! caduca, con el tipo y la disposición dentro de la firma.
//!
//! Entra `{url, segundos?, items: [{clave, version, tipo?, disposicion?}]}`; sale
//! `{segundos, firmadas: [{clave, version, url}]}`, en el orden pedido.
//!
//! ⭐ No PUEDE abrir un socket: depende de `ore-sigv4` (la firma, `sha2`) y de
//!   `serde_json`, y `ore-cli/tests/dependencias.rs` lo vigila. Prefirmar es una
//!   cuenta local (`ore_sigv4::firma::prefirmar`, la del ejemplo oficial de AWS).
//!   Por eso cabe en la imagen de `ore-serve`, que promete no poder leer un
//!   origen aunque tenga, al servir, la credencial de la fuente. La URL lleva la **clave de acceso** —no el
//!   secreto— y la firma; quien la tenga lee ese objeto, en esa versión, hasta
//!   que caduque (medido en E9: expirada 403, manipulada 403).
//!
//! ⚠️ La URL es tan corta como la credencial con la que se firma: con una de STS,
//!   caduca con ella aunque `segundos` diga más.

use ore_sigv4::fuente::Fuente;
use serde_json::{Value, json};

/// Lo que vive una URL: 5 minutos por defecto; de 30 s a una hora.
const VIDA_POR_DEFECTO: u64 = 300;
const VIDA_MINIMA: u64 = 30;
const VIDA_MAXIMA: u64 = 3600;

fn firmar(f: &Fuente, peticion: &Value) -> Result<String, String> {
    let segundos = peticion
        .get("segundos")
        .and_then(|s| {
            s.as_u64()
                .or_else(|| s.as_str().and_then(|t| t.parse().ok()))
        })
        .unwrap_or(VIDA_POR_DEFECTO)
        .clamp(VIDA_MINIMA, VIDA_MAXIMA);
    let items = peticion
        .get("items")
        .and_then(Value::as_array)
        .ok_or("a `firmar` le faltan `items`")?;
    let b = &f.bucket;
    let mut firmadas = Vec::with_capacity(items.len());
    for it in items {
        let c = |k: &str| it.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        let clave = c("clave").ok_or("un ítem sin `clave`")?;
        let version = c("version").unwrap_or("");
        let mut extra: Vec<(&str, &str)> = Vec::new();
        // ⭐ `null` TAMBIÉN se fija (0049 B3·0, medido): es la versión de un objeto
        //   subido antes de activar el versionado, y en un bucket versionado
        //   sigue ahí aunque se sobrescriba la clave. No fijarla servía la
        //   actual —otro contenido— bajo la referencia de la vieja. S3 acepta
        //   `versionId=null` también en un bucket que nunca se versionó.
        if !version.is_empty() {
            extra.push(("versionId", version));
        }
        if let Some(t) = c("tipo") {
            extra.push(("response-content-type", t));
        }
        if let Some(d) = c("disposicion") {
            extra.push(("response-content-disposition", d));
        }
        let ruta = b.ruta(Some(clave));
        let q = ore_sigv4::firma::prefirmar(
            &b.credencial,
            &b.region,
            &b.host(),
            &ruta,
            &extra,
            segundos,
        );
        firmadas.push(
            json!({"clave": clave, "version": version, "url": format!("{}{ruta}?{q}", b.endpoint)}),
        );
    }
    Ok(json!({"segundos": segundos, "firmadas": firmadas}).to_string())
}

/// La petición por stdin (la URL lleva la credencial: nunca por `argv`), la
/// respuesta por stdout, y el porqué de un fallo por stderr —sin la URL—.
fn main() -> std::process::ExitCode {
    let mut entrada = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada).is_err() {
        eprintln!("ore-firmar-s3: no se pudo leer stdin");
        return std::process::ExitCode::FAILURE;
    }
    let hecho = serde_json::from_str::<Value>(&entrada)
        .map_err(|e| format!("la petición no es JSON: {e}"))
        .and_then(|n| {
            let f = ore_sigv4::fuente::leer(n.get("url").and_then(Value::as_str).unwrap_or(""))?;
            firmar(&f, &n)
        });
    match hecho {
        Ok(s) => {
            println!("{s}");
            std::process::ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-firmar-s3: {m}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fuente() -> Fuente {
        ore_sigv4::fuente::leer(
            "s3://mi-bucket/docs/?region=eu-north-1&access_key_id=AKIAEJEMPLO&secret_access_key=secreto",
        )
        .unwrap()
    }

    #[test]
    fn una_url_por_item_fijada_a_su_version_y_sin_el_secreto() {
        let r = firmar(
            &fuente(),
            &json!({"segundos": "99999", "items": [
                {"clave": "docs/contrato 1.pdf", "version": "v1", "tipo": "application/pdf", "disposicion": "inline"},
                {"clave": "docs/b.jpg", "version": "null"}]}),
        )
        .unwrap();
        let r: Value = serde_json::from_str(&r).unwrap();
        assert_eq!(r["segundos"], 3600);
        let u = r["firmadas"][0]["url"].as_str().unwrap();
        assert!(
            u.starts_with("https://mi-bucket.s3.eu-north-1.amazonaws.com/docs/contrato%201.pdf?"),
            "{u}"
        );
        for p in [
            "versionId=v1",
            "response-content-type=application%2Fpdf",
            "X-Amz-Expires=3600",
            "X-Amz-Credential=AKIAEJEMPLO%2F",
            "X-Amz-Signature=",
        ] {
            assert!(u.contains(p), "{p} en {u}");
        }
        assert!(!u.contains("secreto"));
        let u = r["firmadas"][1]["url"].as_str().unwrap();
        assert!(
            u.contains("versionId=null"),
            "una versión `null` también se fija: {u}"
        );
    }
}
