//! `ore-read-sharepoint` — el lector de una biblioteca de SharePoint (o un
//! OneDrive) de un cliente, **la familia de los objetos** (ADR 0061 O5·2).
//!
//! Lo de todos los almacenes de objetos vive en `ore-read-objetos`; aquí queda
//! lo de SharePoint: leer su URL
//! (`sharepoint://host/sites/x/biblioteca/prefijo?tenant=…&cliente=…`, la app
//! de Entra del cliente que confía en la cuenta de Google de esta celda y tiene
//! la concesión `read` de `Sites.Selected` sobre el sitio, D-O5), probar qué
//! deja hacer de verdad (`check`, `acceso.rs`), explorar sus carpetas y
//! bibliotecas, y clasificar lo que Graph contesta. La biblioteca es su
//! [`ore_objetos::Origen`] por `ore_graph::Graph`.

mod acceso;

use ore_graph::Graph;
use ore_objetos::Origen;
use ore_read_objetos::driver::{self, Proveedor};

struct SharePoint;

impl Proveedor for SharePoint {
    const NOMBRE: &'static str = "ore-read-sharepoint";
    /// SharePoint fija por versión (la actual, vigilada por su `cTag`), no
    /// firma (la URL de descarga es de Microsoft, al portador y sin fijar a la
    /// versión: D-O5), da el `quickXorHash` de la actual sin bajarla, y su
    /// credencial es corta (el token del canje en Entra).
    const OBJETOS: ore_objetos::Capacidades = ore_objetos::Capacidades {
        fija: ore_objetos::Fija::Version,
        firma: false,
        huella: Some("quickxor"),
        credencial_corta: true,
    };
    /// El cliente, con su token y los ids del sitio y la biblioteca, vive lo
    /// que la fuente.
    type Fuente = Graph;

    fn leer(url: &str) -> Result<Graph, String> {
        Graph::de_url(url)
    }

    fn prefijo(g: &Graph) -> &str {
        &g.fuente.prefijo
    }

    fn origen(g: &Graph) -> Box<dyn Origen + Sync + '_> {
        Box::new(g)
    }

    fn comprobar(g: &Graph) -> String {
        acceso::comprobar(g)
    }

    fn explorar(g: &Graph) -> Result<String, String> {
        acceso::explorar(g)
    }

    /// Lo que Graph dice (`403 accessDenied`, `404 itemNotFound`…), y lo de la
    /// federación con Entra (`AADSTS…`).
    fn fallo(m: String) -> ore_driver::Fallo {
        driver::clasificar(
            m,
            &[
                "401",
                "403",
                "accessDenied",
                "InvalidAuthenticationToken",
                "AADSTS",
                "Entra",
                "credencial federada",
            ],
            &[
                "404",
                "itemNotFound",
                "no hay una biblioteca",
                "ya no está",
                "ya no tiene",
            ],
        )
    }

    fn contadores() -> (usize, usize) {
        let (peticiones, bytes, _) = ore_graph::contadores();
        (peticiones, bytes)
    }
}

fn main() -> std::process::ExitCode {
    driver::main::<SharePoint>()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_driver::Codigo;

    #[test]
    fn lo_que_graph_contesta_se_dice_en_el_codigo_del_contrato() {
        let c = |m: &str| SharePoint::fallo(m.to_string()).codigo;
        assert_eq!(
            c("no se pudo listar `docs/`: 403 accessDenied Access denied"),
            Codigo::Credencial
        );
        assert_eq!(
            c(
                "la app `x` no tiene una credencial federada para la cuenta de Google de esta celda (400: AADSTS70021: …)"
            ),
            Codigo::Credencial
        );
        assert_eq!(
            c("no se pudo leer `a.pdf` (versión 1.0): ya no está"),
            Codigo::Objeto
        );
        assert_eq!(
            c("no se pudo listar ``: 404 bibliotecaNoEsta no hay una biblioteca `X` en el sitio"),
            Codigo::Objeto
        );
        assert_eq!(c("Graph no contesta: Connection refused"), Codigo::Conexion);
    }

    #[test]
    fn sus_capacidades_dicen_version_sin_firma_y_quickxor() {
        assert_eq!(
            SharePoint::OBJETOS.json(),
            r#"{"credencial_corta":true,"fija":"version","firma":false,"huella":"quickxor"}"#
        );
    }
}
