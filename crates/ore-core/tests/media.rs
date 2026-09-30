//! 0046 E9·4 · **De qué colección es la huella de cada columna.** El caso de
//! conformidad `v1alpha16/valid/a-reference-to-an-item`: la Entity `Contrato`,
//! respaldada por el Dataset `legal.registro`, declara `documento` como
//! `Media<legal.archivo.contratos>`. Quien lee `registro` tiene que poder saber
//! que `documento` es la huella de un ítem de esa colección.

use std::path::Path;

fn caso() -> ore_core::link::Package {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha16/valid/a-reference-to-an-item/input");
    ore_core::validate::cargar_paquete(&dir).0
}

#[test]
fn la_columna_de_la_entidad_lleva_su_coleccion() {
    let pkg = caso();
    let registro = pkg
        .docs
        .iter()
        .find(|d| d.qname().as_deref() == Some("legal.registro"))
        .expect("el dataset `legal.registro`");
    let m = ore_core::vistas::media_de(&pkg, registro);
    assert_eq!(
        m.into_iter().collect::<Vec<_>>(),
        vec![(
            "documento".to_string(),
            "legal.archivo.contratos".to_string()
        )]
    );
}

#[test]
fn lo_que_no_respalda_a_nadie_no_lleva_nada() {
    let pkg = caso();
    let coleccion = pkg
        .docs
        .iter()
        .find(|d| d.qname().as_deref() == Some("legal.archivo.contratos"))
        .expect("la colección");
    assert!(ore_core::vistas::media_de(&pkg, coleccion).is_empty());
}
