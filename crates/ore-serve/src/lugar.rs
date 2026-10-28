//! ⭐ **EL LUGAR DONDE SE TRABAJA** (0079 §19.1, E1a·5b·3): el directorio del
//! árbol, para los documentos, y **la puerta de los punteros** de esa ref
//! ([`ore_core::punteros::Punteros`]).
//!
//! Los manejadores siguen recibiendo un `&Path` de la costura de siempre
//! (`leyendo`, `leyendo_en`, `escribiendo`, `escribiendo_en`, `moviendo_datos`),
//! y lo que toca un puntero abre aquí su puerta: [`Lugar::de`] es **la única
//! función que la construye**. Hoy es [`Ficheros`] sobre el directorio, que la
//! costura ya montó superpuesto si es una rama (0044 C.2 ③), y que publica la
//! costura con el resto del árbol (un `git add -A`), como siempre.
//!
//! ⛔ **·5c cambia la implementación AQUÍ y en ningún otro sitio**: la puerta
//!   del catálogo sobre la ref del árbol. La ref la conoce la costura (es la
//!   rama de `leyendo_en`/`escribiendo_en`); se le dará a [`Lugar::de`] por el
//!   mismo camino que la copia de la rama (`copia::EnRama`), sin cambiar el
//!   tipo de los cierres.
//!
//! | | Qué |
//! |---|---|
//! | [`Lugar::de`] | el directorio y su puerta |
//! | [`Lugar::puntero`] | el de un nombre, como lo leía `leer_en` (`None` si no está o no es un nombre) |
//! | [`Lugar::en_una_rama`] | si el árbol es el de una rama con lo no tocado leído de `main` |
//! | [`ramas`] | la puerta del ciclo de vida de las ramas (crear, borrar, traer `main`) |
//! | [`segundos`] | un `cuando` de [`ore_core::punteros::Version`] a segundos de época |

use std::path::Path;

use ore_core::parse::Node;
use ore_core::punteros::{Ficheros, Punteros, Ramas};
use ore_entrada::http::Respuesta;

/// El directorio de un árbol y la puerta de sus punteros.
pub(crate) struct Lugar<'a> {
    raiz: &'a Path,
    punteros: Box<dyn Punteros + 'a>,
}

impl<'a> Lugar<'a> {
    /// ⭐ **La puerta de un árbol: la única que se construye en ore-serve.**
    pub(crate) fn de(raiz: &'a Path) -> Lugar<'a> {
        Lugar {
            raiz,
            punteros: Box::new(Ficheros::en(raiz)),
        }
    }

    /// El directorio, para los documentos.
    #[allow(dead_code)]
    pub(crate) fn raiz(&self) -> &Path {
        self.raiz
    }

    pub(crate) fn punteros(&self) -> &dyn Punteros {
        self.punteros.as_ref()
    }

    /// El puntero de `corto`, si lo hay: lo que daba `leer_en(datasets, corto)`.
    /// Un nombre que no es `base.n` ni `base.schema.n`, o un almacén que no
    /// contesta, es «no hay», como entonces.
    pub(crate) fn puntero(&self, corto: &str) -> Option<Node> {
        self.punteros.uno(corto).ok().flatten().map(|l| l.puntero)
    }

    /// **¿Es el árbol de una rama, con lo que no tocó leído de `main`?** Lo
    /// que decide si una respuesta dice `de: main | rama` (0044 C.2 ③).
    ///
    /// ⚠️ La puerta no dice su ref (el contrato no lo tiene): hoy lo dice el
    ///   [`crate::git::AL_DIA`] que deja la superposición —que existe sólo en
    ///   una rama por detrás de `main`, lo mismo que miraba `de_main`—; en ·5c,
    ///   la ref con la que se abrió el lugar.
    pub(crate) fn en_una_rama(&self) -> bool {
        self.raiz.join(crate::git::AL_DIA).is_file()
    }
}

/// ⭐ **El ciclo de vida de las ramas** ([`Ramas`]), para las propuestas: la
/// única que se construye. Con [`Ficheros`] no hace nada —crear, borrar y
/// traer una rama de git ya mueve sus punteros—, y por eso no necesita un
/// árbol; ·5c pone aquí el catálogo (su tabla de refs).
pub(crate) fn ramas() -> Box<dyn Ramas> {
    Box::new(Ficheros::en(Path::new("")))
}

/// Lo que no pudo la puerta de las ramas, como respuesta.
pub(crate) fn fallo_de_ramas(que: &str, e: ore_core::punteros::Fallo) -> Respuesta {
    Respuesta::error(e.codigo(), format!("{que}: {e}"))
}

/// Un instante RFC 3339 (`%cI` de git: `2026-10-28T10:00:00+02:00`) a segundos
/// de época, con su desfase.
pub(crate) fn segundos(cuando: &str) -> Option<i64> {
    let base = ore_core::frescura::epoca(cuando)?;
    let resto = cuando.get(19..)?;
    // Las fracciones de segundo, si las hay, no cuentan.
    let resto = resto.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let desfase = match resto {
        "" | "Z" | "z" => 0,
        r => {
            let signo = match r.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let (h, m) = r[1..].split_once(':').unwrap_or((&r[1..], "0"));
            signo * (h.parse::<i64>().ok()? * 3600 + m.parse::<i64>().ok()? * 60)
        }
    };
    Some(base - desfase)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_segundos_de_un_cuando_de_git_cuentan_su_desfase() {
        let utc = segundos("2026-10-28T08:00:00Z").unwrap();
        assert_eq!(segundos("2026-10-28T10:00:00+02:00"), Some(utc));
        assert_eq!(segundos("2026-10-28T03:00:00-05:00"), Some(utc));
        assert_eq!(segundos("2026-10-28T08:00:00.123Z"), Some(utc));
        assert_eq!(segundos("ayer"), None);
    }

    #[test]
    fn el_lugar_lee_por_la_puerta_y_sabe_si_es_una_rama() {
        let d = std::env::temp_dir().join(format!(
            "ore-serve-lugar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(d.join("datasets/ventas/default")).unwrap();
        std::fs::write(
            d.join("datasets/ventas/default/pedidos.json"),
            "{\"estado\": \"copiada\"}\n",
        )
        .unwrap();
        let l = Lugar::de(&d);
        let p = l.puntero("ventas.pedidos").unwrap();
        assert_eq!(
            p.get("estado").and_then(|(_, v)| v.as_str()),
            Some("copiada")
        );
        assert!(l.puntero("ventas.otro").is_none());
        assert!(l.puntero("sin_base").is_none(), "un no-nombre es «no hay»");
        assert!(!l.en_una_rama());
        std::fs::write(
            d.join(crate::git::AL_DIA),
            "{\"main\": \"m\", \"rutas\": {}}\n",
        )
        .unwrap();
        assert!(l.en_una_rama());
        let _ = std::fs::remove_dir_all(&d);
    }
}
