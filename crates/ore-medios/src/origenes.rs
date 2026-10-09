//! **De qué proveedor es una fuente** (ADR 0061 O0·3): `ore-serve` manda la URL
//! de la fuente con su credencial (`fuente`), y su esquema dice quién la lee.
//! `s3://` (S3 y los que hablan su API), `gs://` (GCS, ADR 0061 O2·3: la URL
//! no lleva secreto y se lee con la cuenta de este proceso, o suplantando la
//! del cliente), `az://` (Azure Blob, O3·3: la cuenta de este proceso,
//! federada en la app de Entra del cliente) y `sharepoint://` (O5·3: la misma
//! federación, con `Sites.Selected`; sin URLs firmadas, los bytes pasan por
//! aquí); un proveedor nuevo es una rama aquí, con su [`ore_objetos::Origen`].

use crate::servicio::problema;
use ore_entrada::http::Respuesta;
use ore_objetos::Origen;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type Compartido = Arc<dyn Origen + Send + Sync>;

/// Los clientes de GCS, de Azure y de SharePoint, por URL, entre peticiones: cada uno guarda
/// su token (el suplantado de GCS, una hora; el de Entra, 45 minutos) y en
/// Azure la clave de delegación, que pedir por petición sería una ida a IAM o
/// a Entra por cada ítem. Sus URLs no llevan secreto, así que pueden ser la
/// llave (una `s3://` sí: no se guarda). Pocos: si se llena, se vacía.
const GUARDADOS: usize = 64;

fn guardado(
    fuente: &str,
    crear: impl FnOnce() -> Result<Compartido, String>,
) -> Result<Compartido, String> {
    static GUARDADO: OnceLock<Mutex<HashMap<String, Compartido>>> = OnceLock::new();
    let m = GUARDADO.get_or_init(Default::default);
    let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(o) = m.get(fuente) {
        return Ok(o.clone());
    }
    let o = crear()?;
    if m.len() >= GUARDADOS {
        m.clear();
    }
    m.insert(fuente.to_string(), o.clone());
    Ok(o)
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
        "gs" => guardado(fuente, || {
            ore_gcs::Gcs::de_url(fuente).map(|g| Arc::new(g) as Compartido)
        })
        .map(|o| Box::new(o) as Box<dyn Origen + Send + Sync>)
        .map_err(no_se_lee),
        "az" => guardado(fuente, || {
            ore_azure::Azure::de_url(fuente).map(|a| Arc::new(a) as Compartido)
        })
        .map(|o| Box::new(o) as Box<dyn Origen + Send + Sync>)
        .map_err(no_se_lee),
        "sharepoint" => guardado(fuente, || {
            ore_graph::Graph::de_url(fuente).map(|g| Arc::new(g) as Compartido)
        })
        .map(|o| Box::new(o) as Box<dyn Origen + Send + Sync>)
        .map_err(no_se_lee),
        // ADR 0061 O4·3 (D-O1): un SFTP no versiona; sus colecciones son
        // mantenidas y lo que se sirve sale del lago, nunca de aquí.
        "sftp" => Err(problema(
            422,
            "media/origen",
            "un SFTP no versiona: sus colecciones sólo son mantenidas y se sirven de la copia en \
             el lago, nunca del origen (ADR 0061, D-O1)"
                .to_string(),
        )),
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
        assert!(super::de("az://cuenta/cubo/docs/?tenant=t&cliente=c").is_ok());
        let e = super::de("az://cuenta/cubo?tenant=t&cliente=c&sig=secreto")
            .err()
            .unwrap();
        assert_eq!(e.codigo, 502);
        assert!(!e.cuerpo.jcs().contains("secreto"), "{}", e.cuerpo.jcs());
        assert!(
            super::de("sharepoint://contoso.sharepoint.com/sites/x/Documentos/?tenant=t&cliente=c")
                .is_ok()
        );
        let e = super::de(
            "sharepoint://contoso.sharepoint.com/Documentos?tenant=t&cliente=c&secreto=s",
        )
        .err()
        .unwrap();
        assert_eq!(e.codigo, 502);
        assert!(!e.cuerpo.jcs().contains("=s"), "{}", e.cuerpo.jcs());
        let e = super::de("sftp://u:clave@h/x").err().unwrap();
        assert_eq!(e.codigo, 422);
        let t = e.cuerpo.jcs();
        assert!(t.contains("D-O1") && !t.contains("clave"), "{t}");
        let e = super::de("ftp://u:clave@h/x").err().unwrap();
        assert_eq!(e.codigo, 501);
        let t = e.cuerpo.jcs();
        assert!(t.contains("`ftp`") && !t.contains("clave"), "{t}");
        let e = super::de("s3://cubo/?region=x").err().unwrap();
        assert_eq!(e.codigo, 502, "sin credencial: {}", e.cuerpo.jcs());
    }
}
