//! **Los punteros de los datasets en el árbol** (0038 P2): dónde vive el de
//! cada nombre, cómo se lee el de antes, y qué nombre dice cada fichero.
//!
//! ```text
//! datasets/<base>/<schema>/<nombre>.json      ventas/default/pedidos.json
//! ```
//!
//! Un separador que ningún nombre lleva: `a_b.c` y `a.b_c` eran el mismo
//! `a_b_c.json` (medido, `medida-los-punteros.sh` M2), y borrar `ventas` se
//! llevaba los de `ventas_eu` (M4). **El de antes** (`datasets/<p>_<n>.json`,
//! sólo `default`) se sigue leyendo hasta que `ore migrate` lo mueva, y quien
//! escribe un puntero lo deja en su sitio y retira el de antes.
//!
//! **En el lago**, un dataset nuevo se llama `catalogo/<base>/<schema>/<n>`
//! ([`dataset_nuevo`]) y no `datasets/<base>/…`: `datasets/ventas_x` —la tabla
//! `x` de `ventas`, de antes— es prefijo de `datasets/ventas_x/default/n`, y la
//! credencial prestada a la primera (acotada a su prefijo) alcanzaría la
//! segunda. Lo que ya está no se mueve: el puntero guarda su `dataset`.

use crate::normalize::SCHEMA_POR_DEFECTO;

pub mod crudo;
mod ficheros;
pub mod grabadora;
pub mod protocolo;
mod puerta;

pub use ficheros::Ficheros;
pub use puerta::{
    Cambio, Contexto, De, Esperado, Fallo, Leido, Punteros, Que, Ramas, Reclamados, Version,
};

/// **De dónde se lee un puntero en una rama** (0044 C.2 ③).
#[derive(Debug, PartialEq, Eq)]
pub enum Manda<'a> {
    /// El de la rama: lo cambió, o lo creó.
    Rama,
    /// El de `main` de hoy (su contenido, por su huella): la rama no lo tocó.
    Main(&'a str),
    /// Ninguno: la rama lo borró, o no lo tocó y `main` lo retiró.
    Ninguno,
}

/// La regla, sobre la huella del puntero (el blob de git, o lo que identifique
/// su contenido) en **la rama**, en **el punto del que salió** y en **`main`
/// hoy**. Lo que la rama cambió o creó es suyo; lo que borró, no está; lo que
/// no tocó se lee de `main` al día —lo que `main` creó después, también; lo
/// que retiró, tampoco—. Es la *fallback branch* de Foundry y el `--defer` de
/// dbt, contra `main` de hoy y no contra `main` congelado.
pub fn manda<'a>(rama: Option<&str>, base: Option<&str>, main: Option<&'a str>) -> Manda<'a> {
    match (rama, base) {
        (Some(r), b) if b != Some(r) => Manda::Rama,
        (None, Some(_)) => Manda::Ninguno,
        _ => main.map_or(Manda::Ninguno, Manda::Main),
    }
}

/// `(base, schema, nombre)` de una forma corta (`p.n` o `p.s.n`).
pub fn partes(corto: &str) -> Option<(&str, &str, &str)> {
    let v: Vec<&str> = corto.split('.').collect();
    match v.as_slice() {
        [p, n] if !p.is_empty() && !n.is_empty() => Some((p, SCHEMA_POR_DEFECTO, n)),
        [p, s, n] if !p.is_empty() && !s.is_empty() && !n.is_empty() => Some((p, s, n)),
        _ => None,
    }
}

/// **Cómo se llaman los resultados de una función** (0029 F4a; 0038 P6c),
/// bajo `resultados/`: el dataset (`resultados/<esto>`), su puntero
/// (`<esto>.json`) y el informe de cada corrida (`<esto>_<corrida>.json`).
///
/// En `default`, `p_f`: el nombre de siempre, y nada que migrar. En otro
/// schema, `p/s/f`: con `_` en vez de `/` sería `p_s_f`, que es también lo
/// de la función `s_f` de `default` —dos funciones, un fichero—.
pub fn resultados_de(corto: &str) -> String {
    match partes(corto) {
        Some((b, s, n)) if s != SCHEMA_POR_DEFECTO => format!("{b}/{s}/{n}"),
        Some((b, _, n)) => format!("{b}_{n}"),
        None => corto.replace('.', "_"),
    }
}

/// El nombre en el lago de un dataset que nace: `catalogo/<base>/<schema>/<n>`.
pub fn dataset_nuevo(corto: &str) -> String {
    match partes(corto) {
        Some((b, s, n)) => format!("catalogo/{b}/{s}/{n}"),
        None => format!("catalogo/{}", corto.replace('.', "/")),
    }
}

/// **El nombre en el lago de una tabla, por su `metadata_location`**: lo que
/// hay entre `ore/v2/` y su `/metadata/` (`s3://b/ore/v2/datasets/ventas_x/
/// metadata/00001-….metadata.json` → `datasets/ventas_x`). Es lo cierto —donde
/// están los bytes—, y lo que un puntero tiene que reclamar: un nombre que no
/// fuera el suyo dejaría la tabla huérfana para `recoger-huerfanas`.
pub fn dataset_de_ubicacion(ml: &str) -> Option<String> {
    let (_, resto) = ml.split_once("/ore/v2/")?;
    let (d, _) = resto.rsplit_once("/metadata/")?;
    (!d.is_empty()).then(|| d.to_string())
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::crudo::*;
    #[allow(unused_imports)]
    use std::path::{Path, PathBuf};
    /// Las siete formas de 0044 C.2 ③.
    #[test]
    fn manda_la_rama_lo_suyo_y_main_lo_demas() {
        use super::{Manda, manda};
        // la rama lo cambió, o lo creó: suyo, diga lo que diga main
        assert_eq!(manda(Some("r"), Some("b"), Some("m")), Manda::Rama);
        assert_eq!(manda(Some("r"), None, None), Manda::Rama);
        // la rama lo borró: no está, aunque main siga
        assert_eq!(manda(None, Some("b"), Some("m")), Manda::Ninguno);
        // no lo tocó: el de main de hoy
        assert_eq!(manda(Some("b"), Some("b"), Some("m")), Manda::Main("m"));
        assert_eq!(manda(Some("b"), Some("b"), Some("b")), Manda::Main("b"));
        // main lo creó después: también
        assert_eq!(manda(None, None, Some("m")), Manda::Main("m"));
        // no lo tocó y main lo retiró: tampoco
        assert_eq!(manda(Some("b"), Some("b"), None), Manda::Ninguno);
    }

    use super::*;

    #[test]
    fn los_resultados_de_una_funcion_en_default_y_en_un_schema() {
        assert_eq!(resultados_de("ia.f"), "ia_f");
        assert_eq!(resultados_de("ia.default.f"), "ia_f");
        assert_eq!(resultados_de("ia.espana.f"), "ia/espana/f");
        // la de `default` que se llama `espana_f` no choca con la de `espana`
        assert_ne!(resultados_de("ia.espana_f"), resultados_de("ia.espana.f"));
    }

    #[test]
    fn la_ruta_el_legado_y_el_nombre_en_el_lago() {
        let d = Path::new("datasets");
        assert_eq!(
            ruta_en(d, "ventas.pedidos").unwrap(),
            d.join("ventas").join("default").join("pedidos.json")
        );
        assert_eq!(
            ruta_en(d, "ventas.espana.pedidos").unwrap(),
            d.join("ventas").join("espana").join("pedidos.json")
        );
        assert_eq!(
            legado_en(d, "ventas.pedidos").unwrap(),
            d.join("ventas_pedidos.json")
        );
        assert_eq!(legado_en(d, "ventas.espana.pedidos"), None);
        assert_eq!(ruta("a_b.c").unwrap(), "datasets/a_b/default/c.json");
        assert_eq!(ruta("a.b_c").unwrap(), "datasets/a/default/b_c.json");
        assert_eq!(
            dataset_nuevo("ventas.pedidos"),
            "catalogo/ventas/default/pedidos"
        );
        assert_eq!(partes("x"), None);
        assert_eq!(
            dataset_de_ubicacion(
                "s3://copia/ore/v2/datasets/ventas_x/metadata/00001-a.metadata.json"
            )
            .as_deref(),
            Some("datasets/ventas_x")
        );
        assert_eq!(
            dataset_de_ubicacion("s3://c/ore/v2/catalogo/v/default/p/metadata/1.metadata.json")
                .as_deref(),
            Some("catalogo/v/default/p")
        );
        assert_eq!(dataset_de_ubicacion("gs://b/otra/cosa.json"), None);
    }

    #[test]
    fn la_clave_de_un_fichero_nuevo_y_de_uno_de_antes() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            clave_de(&p("ventas/default/pedidos.json"), None).unwrap(),
            "ventas.pedidos"
        );
        assert_eq!(
            clave_de(&p("ventas/espana/pedidos.json"), None).unwrap(),
            "ventas.espana.pedidos"
        );
        assert_eq!(
            clave_de(&p("ventas_pedidos.json"), None).unwrap(),
            "ventas.pedidos"
        );
        let n = crate::parse::parse(r#"{"tabla":"a_b.c"}"#).unwrap();
        assert_eq!(clave_de(&p("a_b_c.json"), Some(&n)).unwrap(), "a_b.c");
        let n = crate::parse::parse(r#"{"vista":"ventas.default.v"}"#).unwrap();
        assert_eq!(clave_de(&p("ventas_v.json"), Some(&n)).unwrap(), "ventas.v");
        assert_eq!(clave_de(&p("a/b.json"), None), None);
    }

    #[test]
    fn se_lee_el_de_su_sitio_y_si_no_el_de_antes() {
        let d = std::env::temp_dir().join(format!("ore-punteros-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("ventas/default")).unwrap();
        std::fs::write(
            d.join("ventas_viejo.json"),
            r#"{"dataset":"datasets/ventas_viejo"}"#,
        )
        .unwrap();
        std::fs::write(d.join("ventas_nuevo.json"), r#"{"dataset":"antes"}"#).unwrap();
        std::fs::write(
            d.join("ventas/default/nuevo.json"),
            r#"{"dataset":"ahora"}"#,
        )
        .unwrap();
        let (r, _) = leer_en(&d, "ventas.viejo").unwrap();
        assert_eq!(r, d.join("ventas_viejo.json"));
        let t = todos_en(&d);
        assert_eq!(t.len(), 2, "{:?}", t.keys());
        assert_eq!(
            t["ventas.nuevo"].1.get("dataset").unwrap().1.as_str(),
            Some("ahora")
        );
        retirar_legado(&d, "ventas.nuevo", &d.join("ventas/default/nuevo.json"));
        assert!(!d.join("ventas_nuevo.json").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
