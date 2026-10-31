//! `ore-read-gdrive` — el lector de una carpeta de Google Drive de un cliente,
//! **la familia de los objetos** (ADR 0082).
//!
//! Lo de todos los almacenes de objetos vive en `ore-read-objetos`; aquí queda
//! lo de Drive: leer su URL (`gdrive://<id de la carpeta>/<prefijo>`, la
//! carpeta que el cliente compartió con la cuenta de esta celda), probar qué
//! deja hacer de verdad (`check`, `acceso.rs`), explorar sus carpetas, y
//! clasificar lo que Drive contesta. La carpeta es su [`ore_objetos::Origen`]
//! por `ore_gdrive::Gdrive`.

mod acceso;

use ore_gdrive::Gdrive;
use ore_objetos::Origen;
use ore_read_objetos::driver::{self, Proveedor};

struct Drive;

impl Proveedor for Drive {
    const NOMBRE: &'static str = "ore-read-gdrive";
    /// Drive fija por revisión (la actual, X0 B2), no firma, da el `md5` sin
    /// bajar el fichero, y su credencial es corta (el token suplantado).
    const OBJETOS: ore_objetos::Capacidades = ore_objetos::Capacidades {
        fija: ore_objetos::Fija::Version,
        firma: false,
        huella: Some("md5"),
        credencial_corta: true,
    };
    /// El cliente, con su token y lo ya listado, vive lo que la fuente.
    type Fuente = Gdrive;

    fn leer(url: &str) -> Result<Gdrive, String> {
        Gdrive::de_url(url)
    }

    fn prefijo(g: &Gdrive) -> &str {
        &g.fuente.prefijo
    }

    fn origen(g: &Gdrive) -> Box<dyn Origen + Sync + '_> {
        Box::new(g)
    }

    fn comprobar(g: &Gdrive) -> String {
        acceso::comprobar(g)
    }

    fn explorar(g: &Gdrive) -> Result<String, String> {
        acceso::explorar(g)
    }

    /// Lo que Drive dice (`403`, `404 File not found`…), y lo de la identidad.
    fn fallo(m: String) -> ore_driver::Fallo {
        driver::clasificar(
            m,
            &[
                "401",
                "403",
                "insufficientFilePermissions",
                "accessNotConfigured",
                "serviceAccountTokenCreator",
                "could not impersonate",
                "no se pudo suplantar",
                "ORE_GDRIVE_CUENTA",
            ],
            &[
                "404",
                "File not found",
                "is not in the Drive folder",
                "no longer exists",
            ],
        )
    }

    fn contadores() -> (usize, usize) {
        let (peticiones, bytes, _) = ore_gdrive::contadores();
        (peticiones, bytes)
    }
}

fn main() -> std::process::ExitCode {
    driver::main::<Drive>()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_driver::Codigo;

    #[test]
    fn lo_que_drive_contesta_se_dice_en_el_codigo_del_contrato() {
        let c = |m: &str| Drive::fallo(m.to_string()).codigo;
        assert_eq!(
            c("could not list `docs/`: 403 The caller does not have permission"),
            Codigo::Credencial
        );
        assert_eq!(
            c("no se pudo suplantar `x@p.iam.gserviceaccount.com` (403): el cliente tiene que dar"),
            Codigo::Credencial
        );
        assert_eq!(
            c("could not read `a.csv` (version i@r): 404 no longer exists"),
            Codigo::Objeto
        );
        assert_eq!(
            c("could not list `x/`: the folder `x` of `x/` is not in the Drive folder"),
            Codigo::Objeto
        );
        assert_eq!(
            c("Drive is not responding: Connection refused"),
            Codigo::Conexion
        );
    }

    #[test]
    fn sus_capacidades_dicen_version_sin_firma_y_md5() {
        assert_eq!(
            Drive::OBJETOS.json(),
            r#"{"credencial_corta":true,"fija":"version","firma":false,"huella":"md5"}"#
        );
    }
}
