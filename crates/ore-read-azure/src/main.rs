//! `ore-read-azure` — el lector de un contenedor de Azure Blob (o ADLS Gen2)
//! de un cliente, **la familia de los objetos** (ADR 0061 O3·2).
//!
//! Lo de todos los almacenes de objetos vive en `ore-read-objetos`; aquí queda
//! lo de Azure: leer su URL (`az://cuenta/contenedor/prefijo?tenant=…&cliente=…`,
//! la app de Entra del cliente que confía en la cuenta de Google de esta celda,
//! D-O3), probar qué deja hacer de verdad (`check`, `acceso.rs`), explorar sus
//! carpetas y clasificar lo que contesta. El contenedor es su
//! [`ore_objetos::Origen`] por `ore_azure::Azure`.
//!
//! La URL `az://` no lleva secreto —el token sale del canje en Entra—, pero va
//! por stdin como la de cualquier lector.

mod acceso;

use ore_azure::Azure;
use ore_objetos::Origen;
use ore_read_objetos::driver::{self, Proveedor};

struct Microsoft;

impl Proveedor for Microsoft {
    const NOMBRE: &'static str = "ore-read-azure";
    /// Azure fija por `versionId` si la cuenta versiona (si no, por ETag: lo
    /// dice `check`), firma con la clave de delegación, da el `Content-MD5` sin
    /// bajar cuando el blob lo tiene, y su credencial es corta (el token del
    /// canje en Entra).
    const OBJETOS: ore_objetos::Capacidades = ore_objetos::Capacidades {
        fija: ore_objetos::Fija::Version,
        firma: true,
        huella: Some("md5"),
        credencial_corta: true,
    };
    /// El cliente, con su token y su clave de delegación, vive lo que la
    /// fuente: `servir` lo guarda entre peticiones.
    type Fuente = Azure;

    fn leer(url: &str) -> Result<Azure, String> {
        Azure::de_url(url)
    }

    fn prefijo(a: &Azure) -> &str {
        &a.fuente.prefijo
    }

    fn origen(a: &Azure) -> Box<dyn Origen + Sync + '_> {
        Box::new(a)
    }

    fn comprobar(a: &Azure) -> String {
        acceso::comprobar(a)
    }

    fn explorar(a: &Azure) -> Result<String, String> {
        acceso::explorar(a)
    }

    /// Lo que Azure dice (`403 AuthorizationPermissionMismatch`, `404
    /// BlobNotFound`…), y lo de la federación con Entra (`AADSTS…`).
    fn fallo(m: String) -> ore_driver::Fallo {
        driver::clasificar(
            m,
            &[
                "401",
                "403",
                "AuthorizationPermissionMismatch",
                "AuthorizationFailure",
                "AuthenticationFailed",
                "InvalidAuthenticationInfo",
                "AADSTS",
                "Entra",
                "credencial federada",
            ],
            &["404", "BlobNotFound", "ContainerNotFound", "ya no está"],
        )
    }

    fn contadores() -> (usize, usize) {
        ore_azure::contadores()
    }
}

fn main() -> std::process::ExitCode {
    driver::main::<Microsoft>()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_driver::Codigo;

    #[test]
    fn lo_que_azure_contesta_se_dice_en_el_codigo_del_contrato() {
        let c = |m: &str| Microsoft::fallo(m.to_string()).codigo;
        assert_eq!(
            c(
                "no se pudo listar `docs/`: 403 AuthorizationPermissionMismatch This request is not authorized to perform this operation using this permission."
            ),
            Codigo::Credencial
        );
        assert_eq!(
            c(
                "la app `x` no tiene una credencial federada para la cuenta de Google de esta celda (400: AADSTS70021: …)"
            ),
            Codigo::Credencial
        );
        assert_eq!(
            c("no se pudo leer `a.pdf`: 404 BlobNotFound The specified blob does not exist."),
            Codigo::Objeto
        );
        assert_eq!(c("Azure no contesta: Connection refused"), Codigo::Conexion);
    }

    #[test]
    fn sus_capacidades_dicen_version_firma_y_md5() {
        assert_eq!(
            Microsoft::OBJETOS.json(),
            r#"{"credencial_corta":true,"fija":"version","firma":true,"huella":"md5"}"#
        );
    }
}
