//! **De qué proveedor es una fuente** (ADR 0061 O0·3): `ore-serve` manda la URL
//! de la fuente con su credencial (`fuente`), y su esquema dice quién la lee.
//! Hoy `s3://` (S3 y los que hablan su API); un proveedor nuevo es una rama
//! aquí, con su [`ore_objetos::Origen`].

use crate::servicio::problema;
use ore_entrada::http::Respuesta;
use ore_objetos::Origen;

/// El origen de una fuente, o el porqué en el idioma del contrato: una que no
/// se lee, `502`; uno que no se sabe servir todavía, `501`. Nunca se repite la
/// URL: lleva la credencial.
pub fn de(fuente: &str) -> Result<Box<dyn Origen + Send + Sync>, Respuesta> {
    let esquema = fuente.split_once("://").map(|(e, _)| e).unwrap_or("");
    match esquema {
        "s3" => ore_s3::origen::Cubo::de_url(fuente)
            .map(|c| Box::new(c) as Box<dyn Origen + Send + Sync>)
            .map_err(|e| {
                problema(
                    502,
                    "media/origen",
                    format!("la fuente no se pudo leer: {e}"),
                )
            }),
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
        let e = super::de("gs://cubo/?clave=secreto").err().unwrap();
        assert_eq!(e.codigo, 501);
        let t = e.cuerpo.jcs();
        assert!(t.contains("`gs`") && !t.contains("secreto"), "{t}");
        let e = super::de("s3://cubo/?region=x").err().unwrap();
        assert_eq!(e.codigo, 502, "sin credencial: {}", e.cuerpo.jcs());
    }
}
