//! **Quién responde del suelo**, por la CLI pública.
//!
//! Los casos de `conformance/v1alpha8` cubren lo normativo sobre la
//! configuración. Lo que no cubren son las tres consecuencias de haber elegido
//! exigir el dueño **donde se puede bajar el gobierno** y no en todo documento:
//!
//! 1. un **retículo** sin `requiresGovernance` no exige nada de nadie, y por
//!    eso no se le pide dueño — igual que a una configuración sin suelos;
//! 2. un documento **de v1alpha7** no ve la regla, que es lo que deja intacto
//!    todo lo anterior;
//! 3. y quien lo declara mal falla **a cualquier versión**: escribirlo es
//!    afirmar algo, y un nombre libre no se resuelve contra `CODEOWNERS`.
//!
//! La segunda es la que protege la invariante; la primera, la que impide que
//! esto sea un peaje.

use std::path::{Path, PathBuf};
use std::process::Command;

fn paquete(etiqueta: &str, ficheros: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-suelo-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, txt) in ficheros {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
    dir
}

fn validar(dir: &Path) -> String {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("validate")
        .arg(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    String::from_utf8_lossy(&s.stdout).to_string() + String::from_utf8_lossy(&s.stderr).as_ref()
}

const PAQUETE: &str = "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
     metadata: { name: hr, version: 1.0.0, status: active, domain: people }\n\
     spec: { owner: team:data }\n";

/// La configuración, con o sin suelo y con el `owner` que se le pase.
fn config(version: &str, suelo: bool, owner: Option<&str>) -> String {
    let etiquetas = if suelo {
        "\n    labels:\n      gdpr.sensitivity: high"
    } else {
        ""
    };
    let dueno = owner.map(|o| format!("owner: {o}\n")).unwrap_or_default();
    format!(
        "apiVersion: oos.dev/{version}\nkind: OntologyConfig\n\
         metadata: {{ name: x, version: 0.1.0 }}\n{dueno}datasources:\n  \
         - name: erp\n    type: postgres\n    connectionEnv: ERP_URL{etiquetas}\n"
    )
}

/// El retículo, con o sin la exigencia que obliga a tener dueño.
fn reticulo(exige: bool) -> String {
    let g = if exige {
        "\n  requiresGovernance:\n    high: [constraint]"
    } else {
        ""
    };
    format!(
        "apiVersion: oos.dev/v1alpha8\nkind: Lattice\n\
         metadata: {{ name: sensitivity, namespace: gdpr }}\nspec:\n  \
         levels: [none, low, high]{g}\n"
    )
}

fn arbol(etiqueta: &str, cfg: &str, ret: &str) -> PathBuf {
    paquete(
        etiqueta,
        &[
            ("ontology.config.yaml", cfg),
            ("package.yaml", PAQUETE),
            ("lattices/s.yaml", ret),
        ],
    )
}

/// **Sin exigencia no se pide dueño** — ni al retículo ni a la configuración.
///
/// Es lo que impide que esto sea un peaje: el sujeto de la regla no es el
/// documento, es **la decisión de fijar un mínimo**. Un retículo que solo
/// declara una escala no apaga la exigencia de nadie, porque no impone ninguna.
#[test]
fn sin_nada_que_bajar_no_se_pide_dueno() {
    let dir = arbol(
        "sin-sujeto",
        &config("v1alpha8", false, None),
        &reticulo(false),
    );
    let out = validar(&dir);
    assert!(
        !out.contains("OOS2009"),
        "ni suelo ni `requiresGovernance`: no hay a quién mirar:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Y los dos controles, uno por documento: en cuanto aparece lo que baja el
    // gobierno, se pide quién responde.
    let dir = arbol(
        "suelo-solo",
        &config("v1alpha8", true, None),
        &reticulo(false),
    );
    let out = validar(&dir);
    assert!(
        out.contains("OOS2009") && out.contains("datasource"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    let dir = arbol(
        "exigencia-sola",
        &config("v1alpha8", false, None),
        &reticulo(true),
    );
    let out = validar(&dir);
    assert!(
        out.contains("OOS2009") && out.contains("requiresGovernance"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Un documento de v1alpha7 no ve la regla.**
///
/// El campo no existía, así que exigirlo hacia atrás cambiaría un resultado de
/// un borrador ya publicado. La misma configuración, con la misma clasificación
/// y sin dueño, compila en v1alpha7 y no en v1alpha8 — y esa diferencia es
/// deliberada, no un descuido.
#[test]
fn el_borrador_viejo_no_ve_la_regla() {
    let dir = arbol(
        "v7",
        &config("v1alpha7", true, None),
        "apiVersion: oos.dev/v1alpha3\nkind: Lattice\n\
         metadata: { name: sensitivity, namespace: gdpr }\nspec:\n  \
         levels: [none, low, high]\n  requiresGovernance:\n    high: [constraint]\n",
    );
    let out = validar(&dir);
    assert!(
        !out.contains("OOS2009"),
        "v1alpha7 y v1alpha3 no ven esto:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Declararlo mal falla a cualquier versión**, y no es lo mismo que exigirlo.
///
/// Omitir es una cosa —y solo se cobra desde v1alpha8—; escribir `owner: Ana`
/// es otra: afirma que alguien responde, y un nombre libre no se resuelve
/// contra `CODEOWNERS`. Un dueño que no se puede resolver tiene exactamente el
/// mismo aspecto que uno que sí.
#[test]
fn un_dueno_que_no_es_un_handle_falla_aunque_sea_viejo() {
    let dir = arbol(
        "handle-malo",
        &config("v1alpha7", true, Some("Ana")),
        &reticulo(false),
    );
    let out = validar(&dir);
    assert!(
        out.contains("OOS2009") && out.contains("no es un handle"),
        "escribirlo mal se dice, aunque omitirlo no se cobre aquí:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
