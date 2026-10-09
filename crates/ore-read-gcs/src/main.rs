//! `ore-read-gcs` — el lector de un bucket de GCS de un cliente, **la familia
//! de los objetos** (ADR 0061 O2·2).
//!
//! Lo de todos los almacenes de objetos vive en `ore-read-objetos`; aquí queda
//! lo de GCS: leer su URL (`gs://bucket/prefijo`, con `?suplantar=` si se lee
//! como una cuenta del cliente, D-O2), preguntarle a GCS qué permisos tiene de
//! verdad la identidad (`check`, `acceso.rs`), explorar sus carpetas, y
//! clasificar lo que contesta. El bucket es su [`ore_objetos::Origen`] por
//! `ore_gcs::Gcs`.
//!
//! La URL `gs://` no lleva secreto —el token es el de la cuenta de la celda, o
//! el que obtiene suplantando—, pero va por stdin como la de cualquier lector.

mod acceso;

use ore_gcs::Gcs;
use ore_objetos::Origen;
use ore_read_objetos::driver::{self, Proveedor};

struct Google;

impl Proveedor for Google {
    const NOMBRE: &'static str = "ore-read-gcs";
    /// GCS fija por generación (siempre la hay), firma V4 pidiéndole la firma a
    /// IAM (`signBlob`), da el `crc32c` sin bajar, y su credencial es corta (el
    /// token de la cuenta, o el suplantado: una hora).
    const OBJETOS: ore_objetos::Capacidades = ore_objetos::Capacidades {
        fija: ore_objetos::Fija::Version,
        firma: true,
        huella: Some("crc32c"),
        credencial_corta: true,
    };
    /// El cliente, con su token (y el suplantado, renovado antes de caducar),
    /// vive lo que la fuente: `servir` lo guarda entre peticiones.
    type Fuente = Gcs;

    fn leer(url: &str) -> Result<Gcs, String> {
        Gcs::de_url(url)
    }

    fn prefijo(g: &Gcs) -> &str {
        &g.fuente.prefijo
    }

    fn origen(g: &Gcs) -> Box<dyn Origen + Sync + '_> {
        Box::new(g)
    }

    fn comprobar(g: &Gcs) -> String {
        acceso::comprobar(g)
    }

    fn explorar(g: &Gcs) -> Result<String, String> {
        acceso::explorar(g)
    }

    /// Lo que GCS dice (`403 … does not have storage.objects.list access`,
    /// `404 No such object`), y lo de la identidad (suplantar, el metadata
    /// server).
    fn fallo(m: String) -> ore_driver::Fallo {
        driver::clasificar(
            m,
            &[
                "401",
                "403",
                "does not have storage.",
                "Anonymous caller",
                "no se pudo suplantar",
                "metadata server",
                "signBlob",
            ],
            &["404", "No such object", "does not exist", "ya no está"],
        )
    }

    fn contadores() -> (usize, usize) {
        ore_gcs::contadores()
    }
}

fn main() -> std::process::ExitCode {
    driver::main::<Google>()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_driver::Codigo;

    #[test]
    fn lo_que_gcs_contesta_se_dice_en_el_codigo_del_contrato() {
        let c = |m: &str| Google::fallo(m.to_string()).codigo;
        assert_eq!(
            c(
                "no se pudo listar `docs/`: 403 ore-driver@x.iam.gserviceaccount.com does not have storage.objects.list access to the Google Cloud Storage bucket."
            ),
            Codigo::Credencial
        );
        assert_eq!(
            c("no se pudo suplantar a `lector@cliente.iam.gserviceaccount.com` (403): …"),
            Codigo::Credencial
        );
        assert_eq!(
            c("no se pudo leer `a.pdf`: 404 No such object: cubo/a.pdf"),
            Codigo::Objeto
        );
        assert_eq!(c("GCS no contesta: Connection refused"), Codigo::Conexion);
    }

    #[test]
    fn sus_capacidades_dicen_generacion_firma_y_crc32c() {
        assert_eq!(
            Google::OBJETOS.json(),
            r#"{"credencial_corta":true,"fija":"version","firma":true,"huella":"crc32c"}"#
        );
    }
}
