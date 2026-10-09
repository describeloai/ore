//! **De qué proveedor es una fuente** (ADR 0061 O0·3): `ore-serve` manda la URL
//! de la fuente con su credencial (`fuente`), y su esquema dice quién la lee.
//! `s3://` (S3 y los que hablan su API) y `gs://` (GCS, ADR 0061 O2·3: la
//! URL no lleva secreto y se lee con la cuenta de este proceso, o suplantando
//! la del cliente); un proveedor nuevo es una rama aquí, con su
//! [`ore_objetos::Origen`].

use crate::servicio::problema;
use ore_entrada::http::Respuesta;
use ore_objetos::Origen;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Los clientes de GCS, por URL, entre peticiones: cada uno guarda su token (y
/// el suplantado, una hora), que pedir por petición sería una ida a IAM por
/// cada ítem. La URL `gs://` no lleva secreto, así que puede ser la llave.
/// Pocos: si se llena, se vacía.
const GUARDADOS: usize = 64;

fn gcs(fuente: &str) -> Result<Arc<ore_gcs::Gcs>, String> {
    static GUARDADO: OnceLock<Mutex<HashMap<String, Arc<ore_gcs::Gcs>>>> = OnceLock::new();
    let m = GUARDADO.get_or_init(Default::default);
    let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(g) = m.get(fuente) {
        return Ok(g.clone());
    }
    let g = Arc::new(ore_gcs::Gcs::de_url(fuente)?);
    if m.len() >= GUARDADOS {
        m.clear();
    }
    m.insert(fuente.to_string(), g.clone());
    Ok(g)
}

/// El origen de una fuente, o el porqué en el idioma del contrato: una que no
/// se lee, `502`; uno que no se sabe servir todavía, `501`. Nunca se repite la
/// URL: lleva la credencial.
pub fn de(fuente: &str) -> Result<Box<dyn Origen + Send + Sync>, Respuesta> {
    let esquema = fuente.split_once("://").map(|(e, _)| e).unwrap_or("");
    let no_se_lee = |e: String| {
        problema(
            502,
            "media/origen",
            format!("la fuente no se pudo leer: {e}"),
        )
    };
    match esquema {
        "s3" => ore_s3::origen::Cubo::de_url(fuente)
            .map(|c| Box::new(c) as Box<dyn Origen + Send + Sync>)
            .map_err(no_se_lee),
        "gs" => gcs(fuente)
            .map(|g| Box::new(g) as Box<dyn Origen + Send + Sync>)
            .map_err(no_se_lee),
        otro => Err(problema(
            501,
            "media/origen",
            format!(
                "un origen `{}` no se sabe servir todavía (0061)",
                if otro.is_empty() { "sin esquema" } else { otro }
            ),
        )),
    }
}

#[cfg(test)]
mod pruebas {
    #[test]
    fn el_esquema_dice_el_proveedor_y_la_url_no_se_repite() {
        assert!(super::de("s3://cubo/?region=x&access_key_id=a&secret_access_key=b").is_ok());
        assert!(super::de("gs://cubo/docs/").is_ok());
        let e = super::de("gs://cubo/?clave=secreto").err().unwrap();
        assert_eq!(e.codigo, 502);
        assert!(!e.cuerpo.jcs().contains("secreto"), "{}", e.cuerpo.jcs());
        let e = super::de("az://cuenta/contenedor?sas=secreto")
            .err()
            .unwrap();
        assert_eq!(e.codigo, 501);
        let t = e.cuerpo.jcs();
        assert!(t.contains("`az`") && !t.contains("secreto"), "{t}");
        let e = super::de("s3://cubo/?region=x").err().unwrap();
        assert_eq!(e.codigo, 502, "sin credencial: {}", e.cuerpo.jcs());
    }
}
