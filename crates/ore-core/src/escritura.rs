//! **La escritura** (`docs/escrituras.md`, E1): lo que una escritura en el
//! árbol deja dicho de sí —qué leyó y desde dónde—, una forma para todos los
//! caminos.
//!
//! Hasta aquí cada camino dejaba su `procedencia` en el puntero
//! (`datasets/<b>/<s>/<n>.json`) con una forma distinta: `inputs` y
//! `transform` dentro de un `@transform` (también el que `ore-serve` envuelve
//! alrededor del SQL de la sesión, que no es ningún transform), `leidas` fuera,
//! `build` desde un Build. Y cada lector miraba una parte: `derivedFrom` las dos
//! listas, el linaje sólo `leidas`. Aquí se lee **cualquiera de las formas**, la
//! vieja y la nueva (`escritura` en el puntero, que escribirá E2), a una sola:
//!
//! | campo | qué es |
//! |---|---|
//! | `lee` | lo que se leyó para escribir, por su nombre cualificado, sin repetidos; `en_vivo` si se leyó en el origen |
//! | `desde` | de dónde: `query` (una hoja), `sesion`, `notebook`, `build`, `api` (un motor de fuera por el catálogo REST) o `nose` |
//! | `id`, `ejecucion`, `commit`, `sentencia` | la hoja, el repositorio o el build; la celda o la ejecución; el commit del código; lo que corrió |
//! | `puesto`, `anclada_a`, `datos_de` | como en la procedencia: el puesto, la colección a la que se ancla, de cuándo era cada cosa leída (0063 S6b) |
//!
//! **`derivedFrom` sale de aquí** ([`derivado_de`]): todo lo leído que el árbol
//! conoce —vista, dataset, colección o, desde v1alpha33 `03`, tabla—, sin el
//! propio activo ni la colección a la que se ancla. Es por donde baja la
//! clasificación (`flow`), así que lo que se lee de una tabla de un origen deja
//! de perderse.

use std::collections::{BTreeMap, BTreeSet};

use crate::document::Kind;
use crate::json::Json;
use crate::link::Package;

/// De dónde salió una escritura, como el `entity_type` del linaje de Databricks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desde {
    /// Una hoja de Query.
    Query,
    /// La sesión de un repositorio (sus celdas, o un `.sql` corrido en ella).
    Sesion,
    /// Un notebook.
    Notebook,
    /// Un Build: lo escribe el `Transform` del árbol.
    Build,
    /// Un motor de fuera, por el catálogo REST con un token.
    Api,
    /// No se sabe: una escritura de antes que no lo dejó dicho.
    NoSeSabe,
}

impl Desde {
    pub fn como_texto(self) -> &'static str {
        match self {
            Desde::Query => "query",
            Desde::Sesion => "sesion",
            Desde::Notebook => "notebook",
            Desde::Build => "build",
            Desde::Api => "api",
            Desde::NoSeSabe => "nose",
        }
    }

    pub fn de_texto(t: &str) -> Desde {
        match t {
            "query" => Desde::Query,
            "sesion" => Desde::Sesion,
            "notebook" => Desde::Notebook,
            "build" => Desde::Build,
            "api" => Desde::Api,
            _ => Desde::NoSeSabe,
        }
    }
}

/// Una cosa leída para escribir.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Leido {
    /// Su nombre cualificado, como lo dijo quien lo leyó.
    pub nombre: String,
    /// Leída en el origen (una `Table` en vivo), no del lago.
    pub en_vivo: bool,
}

/// Lo que una escritura deja dicho de sí.
#[derive(Debug, Clone, PartialEq)]
pub struct Escritura {
    /// Sin repetidos, en orden.
    pub lee: Vec<Leido>,
    pub desde: Desde,
    pub id: Option<String>,
    pub ejecucion: Option<String>,
    pub commit: Option<String>,
    pub sentencia: Option<String>,
    pub puesto: Option<String>,
    pub anclada_a: Option<String>,
    /// 0063 S6b: de cuándo era cada cosa leída, tal cual.
    pub datos_de: Option<Json>,
}

fn texto(m: &BTreeMap<String, Json>, k: &str) -> Option<String> {
    match m.get(k) {
        Some(Json::Str(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

fn lista(m: &BTreeMap<String, Json>, k: &str) -> Vec<String> {
    match m.get(k) {
        Some(Json::Arr(a)) => a
            .iter()
            .filter_map(|x| match x {
                Json::Str(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl Escritura {
    /// La escritura de un JSON: la forma nueva (`lee`, `desde`) o la
    /// procedencia de antes (`inputs`/`leidas`, `transform`, `build`). `None`
    /// si no es un objeto.
    pub fn de_json(j: &Json) -> Option<Escritura> {
        let Json::Obj(m) = j else { return None };
        let mut e = Escritura {
            lee: Vec::new(),
            desde: Desde::NoSeSabe,
            id: None,
            ejecucion: None,
            commit: None,
            sentencia: None,
            puesto: texto(m, "puesto"),
            anclada_a: texto(m, "anclada_a"),
            datos_de: m.get("datos_de").cloned(),
        };
        let mut lee: BTreeSet<Leido> = BTreeSet::new();
        if let Some(Json::Obj(d)) = m.get("desde") {
            // La forma nueva.
            e.desde = texto(d, "tipo").map_or(Desde::NoSeSabe, |t| Desde::de_texto(&t));
            e.id = texto(d, "id");
            e.ejecucion = texto(d, "ejecucion");
            e.commit = texto(d, "commit");
            e.sentencia = texto(d, "sentencia");
            if let Some(Json::Arr(a)) = m.get("lee") {
                for x in a {
                    if let Json::Obj(l) = x
                        && let Some(nombre) = texto(l, "nombre")
                    {
                        let en_vivo = matches!(l.get("en_vivo"), Some(Json::Bool(true)));
                        lee.insert(Leido { nombre, en_vivo });
                    }
                }
            }
        } else {
            // La procedencia de antes. Un `build` es un Build; si no, lo
            // escribió una sesión —también con `transform`: es el que el SDK
            // pone alrededor del SQL de la sesión, o uno llamado a mano—.
            let en_vivo: BTreeSet<String> = lista(m, "leidas_en_vivo").into_iter().collect();
            let mut nombres = lista(m, "inputs");
            nombres.extend(lista(m, "leidas"));
            for n in nombres {
                let v = en_vivo.contains(&n);
                lee.insert(Leido {
                    nombre: n,
                    en_vivo: v,
                });
            }
            for n in en_vivo {
                lee.insert(Leido {
                    nombre: n,
                    en_vivo: true,
                });
            }
            match m.get("build") {
                Some(Json::Obj(b)) => {
                    e.desde = Desde::Build;
                    e.id = texto(b, "id");
                    e.commit = texto(b, "commit");
                }
                _ => {
                    e.desde = Desde::Sesion;
                    e.id = texto(m, "codigo");
                }
            }
        }
        // Un nombre leído en vivo y del lago es uno: en vivo, que es lo que pesa.
        let mut vistos: BTreeMap<String, bool> = BTreeMap::new();
        for l in lee {
            *vistos.entry(l.nombre).or_default() |= l.en_vivo;
        }
        e.lee = vistos
            .into_iter()
            .map(|(nombre, en_vivo)| Leido { nombre, en_vivo })
            .collect();
        Some(e)
    }

    /// La forma nueva, para el puntero (E2).
    pub fn a_json(&self) -> Json {
        let lee = self
            .lee
            .iter()
            .map(|l| {
                let mut m = vec![("nombre", Json::s(&l.nombre))];
                if l.en_vivo {
                    m.push(("en_vivo", Json::Bool(true)));
                }
                Json::obj(m)
            })
            .collect();
        let mut desde = vec![("tipo", Json::s(self.desde.como_texto()))];
        for (k, v) in [
            ("id", &self.id),
            ("ejecucion", &self.ejecucion),
            ("commit", &self.commit),
            ("sentencia", &self.sentencia),
        ] {
            if let Some(v) = v {
                desde.push((k, Json::s(v)));
            }
        }
        let mut m = vec![("lee", Json::Arr(lee)), ("desde", Json::obj(desde))];
        for (k, v) in [("puesto", &self.puesto), ("anclada_a", &self.anclada_a)] {
            if let Some(v) = v {
                m.push((k, Json::s(v)));
            }
        }
        if let Some(d) = &self.datos_de {
            m.push(("datos_de", d.clone()));
        }
        Json::obj(m)
    }

    /// Los nombres leídos, en orden.
    pub fn nombres(&self) -> impl Iterator<Item = &str> {
        self.lee.iter().map(|l| l.nombre.as_str())
    }
}

/// La escritura de un puntero: su `escritura` o, si es de antes, su
/// `procedencia`.
pub fn del_puntero(puntero: &Json) -> Option<Escritura> {
    let Json::Obj(m) = puntero else { return None };
    m.get("escritura")
        .or_else(|| m.get("procedencia"))
        .and_then(Escritura::de_json)
}

/// Qué es en el árbol un nombre leído, por su nombre cualificado: una vista,
/// un dataset, una colección o una tabla (v1alpha33 `03`). `None` si el árbol
/// no lo tiene.
pub fn clase_de(pkg: &Package, nombre: &str) -> Option<Kind> {
    if pkg.view(nombre).is_some() {
        Some(Kind::View)
    } else if pkg.dataset(nombre).is_some() {
        Some(Kind::Dataset)
    } else if pkg.collection(nombre).is_some() {
        Some(Kind::MediaCollection)
    } else if pkg.table(nombre).is_some() {
        Some(Kind::Table)
    } else {
        None
    }
}

/// **El `derivedFrom` de lo escrito** (v1alpha12 §5, v1alpha19 `01` §2,
/// v1alpha33 `03`): lo que la escritura leyó y `existe` dice que el árbol
/// tiene, sin el propio activo (`OOS2019`) ni la colección a la que se ancla
/// (su relación es `anchoredTo`). En la forma corta, sin repetidos.
pub fn derivado_de(
    e: &Escritura,
    propio: &str,
    anclada: Option<&str>,
    existe: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    derivado(e.nombres(), propio, anclada, existe)
}

/// [`derivado_de`] sobre los nombres ya sacados.
pub fn derivado<'a>(
    nombres: impl IntoIterator<Item = &'a str>,
    propio: &str,
    anclada: Option<&str>,
    existe: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    let corto = |n: &str| crate::normalize::a_corto(n).into_owned();
    let propio = corto(propio);
    let anclada = anclada.map(corto);
    nombres
        .into_iter()
        .filter(|n| corto(n) != propio && anclada.as_deref() != Some(corto(n).as_str()))
        .filter(|n| existe(n))
        .map(corto)
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn de(texto: &str) -> Escritura {
        let n = crate::parse::parse(texto).unwrap();
        Escritura::de_json(&Json::de_node(&n)).unwrap()
    }

    fn nombres(e: &Escritura) -> Vec<&str> {
        e.nombres().collect()
    }

    #[test]
    fn la_procedencia_de_una_sesion_es_lo_que_leyo() {
        let e = de(r#"{"puesto": "p1", "leidas": ["ventas.pedidos", "ventas.clientes"]}"#);
        assert_eq!(e.desde, Desde::Sesion);
        assert_eq!(nombres(&e), ["ventas.clientes", "ventas.pedidos"]);
        assert_eq!(e.puesto.as_deref(), Some("p1"));
    }

    #[test]
    fn el_transform_fingido_del_sql_de_la_sesion_es_una_sesion() {
        // Lo que `ore-serve` envolvía en `@transform` para el SQL de la sesión:
        // `inputs` cuenta como leído, y no es un Build.
        let e = de(
            r#"{"puesto": "p1", "transform": "ejemplo", "inputs": ["standard_test.public.products"]}"#,
        );
        assert_eq!(e.desde, Desde::Sesion);
        assert_eq!(nombres(&e), ["standard_test.public.products"]);
    }

    #[test]
    fn un_build_dice_cual_y_lo_leido_en_vivo_se_marca() {
        let e = de(
            r#"{"inputs": ["o.public.pagos", "ventas.zonas"], "transform": "pipeline/x.yaml",
                "build": {"id": "trabajo-ana-1", "commit": "abc1234"},
                "leidas_en_vivo": ["o.public.pagos"]}"#,
        );
        assert_eq!(e.desde, Desde::Build);
        assert_eq!(e.id.as_deref(), Some("trabajo-ana-1"));
        assert_eq!(e.commit.as_deref(), Some("abc1234"));
        assert_eq!(
            e.lee,
            [
                Leido {
                    nombre: "o.public.pagos".into(),
                    en_vivo: true
                },
                Leido {
                    nombre: "ventas.zonas".into(),
                    en_vivo: false
                },
            ]
        );
    }

    #[test]
    fn la_forma_nueva_va_y_vuelve() {
        let e = Escritura {
            lee: vec![
                Leido {
                    nombre: "o.public.pagos".into(),
                    en_vivo: true,
                },
                Leido {
                    nombre: "ventas.zonas".into(),
                    en_vivo: false,
                },
            ],
            desde: Desde::Query,
            id: Some("queries/limpieza.sql".into()),
            ejecucion: Some("7".into()),
            commit: None,
            sentencia: Some("create or replace dataset ventas.x as select 1".into()),
            puesto: Some("p1".into()),
            anclada_a: None,
            datos_de: None,
        };
        let j = e.a_json();
        assert_eq!(Escritura::de_json(&j).as_ref(), Some(&e));
        let txt = j.jcs();
        assert!(
            txt.contains(r#""desde":{"ejecucion":"7","id":"queries/limpieza.sql","sentencia":"#),
            "{txt}"
        );
    }

    #[test]
    fn el_puntero_prefiere_su_escritura_a_la_procedencia() {
        let p = crate::parse::parse(
            r#"{"procedencia": {"leidas": ["a.viejo"]},
                "escritura": {"lee": [{"nombre": "a.nuevo"}], "desde": {"tipo": "query"}}}"#,
        )
        .unwrap();
        let e = del_puntero(&Json::de_node(&p)).unwrap();
        assert_eq!(e.desde, Desde::Query);
        assert_eq!(nombres(&e), ["a.nuevo"]);
        let solo = crate::parse::parse(r#"{"procedencia": {"leidas": ["a.viejo"]}}"#).unwrap();
        assert_eq!(
            nombres(&del_puntero(&Json::de_node(&solo)).unwrap()),
            ["a.viejo"]
        );
        assert!(del_puntero(&Json::obj([])).is_none());
    }

    #[test]
    fn derived_from_sin_si_mismo_ni_su_ancla_y_solo_lo_que_existe() {
        let e = de(
            r#"{"leidas": ["ventas.default.salida", "ventas.pedidos", "docs.contratos", "ventas.borrado"]}"#,
        );
        let d = derivado_de(&e, "ventas.salida", Some("docs.default.contratos"), |n| {
            n != "ventas.borrado"
        });
        assert_eq!(d.into_iter().collect::<Vec<_>>(), ["ventas.pedidos"]);
    }
}
