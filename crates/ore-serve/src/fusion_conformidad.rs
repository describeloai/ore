//! ⭐ **La conformidad de la fusión** (0079 §19.2, ·5c·5c·1, Q1): la regla de hoy
//! ([`Forja::fusionar_puntero`], tres vías sobre ficheros de git) y el plan del catálogo
//! (`plan_fusion` de la v3, sobre filas) dan lo mismo, caso por caso: el mismo `(caso, resultado)`
//! por activo y el mismo bloqueo por algo «sin elegir» (el 409 de `propuestas.rs`).
//!
//! Cada caso monta los mismos tres punteros (el de `main` en el punto de salida, el de `main` hoy y
//! el de la rama) dos veces: en un repositorio de git (un commit base, la rama y `main` encima) y en
//! un catálogo (`main` sembrado, la rama nace, y escriben los dos por [`Almacen::escribir`]).
//!
//! Es un hijo de `git` (por `#[path]`) para poder llamar a `fusionar_puntero`, que es privada.
//! Necesita `ORE_CATALOGO_PRUEBAS`; sin ella, lo dice y no mide nada.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use ore_catalogo::almacen::{Almacen, CambioRuta, EsperadoMl, EsquemaDePrueba, QueRuta};
use ore_core::json::Json;

use super::Forja;

const RUTA: &str = "datasets/ventas/default/t.json";

/// El puntero entero: su ubicación (y el `dataset` que el catálogo le pondría), un campo que no es
/// la ubicación (`comprobado`) y, si lo escribió alguien, `escrito_por`.
fn puntero(v: u32, comprobado: &str, escrito_por: Option<&str>) -> Json {
    let mut campos = vec![
        (
            "metadata_location",
            Json::s(format!(
                "gs://b/ore/v2/catalogo/ventas/default/t/metadata/{v:05}-x.metadata.json"
            )),
        ),
        ("dataset", Json::s("catalogo/ventas/default/t")),
        ("comprobado", Json::s(comprobado)),
    ];
    if let Some(p) = escrito_por {
        campos.push(("escrito_por", Json::s(p)));
    }
    Json::obj(campos)
}

/// Un caso: `main` en el punto de salida, y lo que escriben después `main` y la rama
/// (`None` = nada; `Some(None)` = lo retira). `elegido`: lo que quien fusiona eligió.
struct Caso {
    nombre: &'static str,
    base: Option<Json>,
    main: Option<Option<Json>>,
    rama: Option<Option<Json>>,
    elegido: Option<&'static str>,
}

fn casos() -> Vec<Caso> {
    let p = puntero;
    let ana = Some("persona:ana");
    let leo = Some("persona:leo");
    let base = || Some(p(1, "x", None));
    let mut v = vec![
        Caso {
            nombre: "la rama no lo tocó",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: None,
            elegido: None,
        },
        Caso {
            nombre: "solo la rama lo cambió",
            base: base(),
            main: None,
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
        },
        Caso {
            nombre: "solo la rama lo borró",
            base: base(),
            main: None,
            rama: Some(None),
            elegido: None,
        },
        Caso {
            nombre: "nació en la rama",
            base: None,
            main: None,
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
        },
        Caso {
            nombre: "los dos escribieron lo mismo",
            base: base(),
            main: Some(Some(p(3, "x", ana))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
        },
        Caso {
            nombre: "los dos, con receta",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: Some(Some(p(3, "x", None))),
            elegido: None,
        },
        Caso {
            nombre: "Q1: la rama reescribió lo mismo y main se movió",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: Some(Some(p(1, "x", None))),
            elegido: None,
        },
        Caso {
            nombre: "Q1: main solo refrescó otro campo y la rama escribió",
            base: base(),
            main: Some(Some(p(1, "y", None))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
        },
        Caso {
            nombre: "Q1: main solo refrescó otro campo y la rama reconstruyó",
            base: base(),
            main: Some(Some(p(1, "y", None))),
            rama: Some(Some(p(3, "x", None))),
            elegido: None,
        },
        Caso {
            nombre: "Q1: la rama lo borró y main lo cambió",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: Some(None),
            elegido: None,
        },
        Caso {
            nombre: "Q1: la rama lo borró y main lo cambió, elegida la rama",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: Some(None),
            elegido: Some("rama"),
        },
        Caso {
            nombre: "nació en los dos, distinto",
            base: None,
            main: Some(Some(p(2, "x", leo))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
        },
        Caso {
            nombre: "main lo borró y la rama lo cambió",
            base: base(),
            main: Some(None),
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
        },
        Caso {
            nombre: "main lo borró y la rama lo reconstruyó",
            base: base(),
            main: Some(None),
            rama: Some(Some(p(3, "x", None))),
            elegido: None,
        },
        Caso {
            nombre: "los dos lo borraron",
            base: base(),
            main: Some(None),
            rama: Some(None),
            elegido: None,
        },
    ];
    for e in [Some("rama"), Some("main"), None] {
        v.push(Caso {
            nombre: "los dos, sin receta",
            base: base(),
            main: Some(Some(p(2, "x", leo))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: e,
        });
    }
    v
}

fn corre(dir: &Path, args: &[&str]) -> String {
    let s = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "s")
        .env("GIT_AUTHOR_EMAIL", "s@x")
        .env("GIT_COMMITTER_NAME", "s")
        .env("GIT_COMMITTER_EMAIL", "s@x")
        .output()
        .unwrap();
    assert!(
        s.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&s.stderr)
    );
    String::from_utf8_lossy(&s.stdout).trim().to_string()
}

/// Deja el fichero del puntero como dice `p` (`None`: que no esté) y hace un commit.
fn commit_con(dir: &Path, p: &Option<Json>, mensaje: &str) {
    let f = dir.join(RUTA);
    match p {
        Some(p) => {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, p.pretty()).unwrap();
        }
        None => {
            let _ = std::fs::remove_file(&f);
        }
    }
    corre(dir, &["add", "-A"]);
    corre(dir, &["commit", "-q", "--allow-empty", "-m", mensaje]);
}

/// Lo que importa de un puntero para comparar los dos lados: ubicación, `comprobado` y quién.
type Huella = Option<(Option<String>, Option<String>, Option<String>)>;

fn huella(texto: Option<&str>) -> Huella {
    let n = ore_core::parse::parse(texto?).ok()?;
    let c = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|v| *v != "null")
            .map(String::from)
    };
    Some((c("metadata_location"), c("comprobado"), c("escrito_por")))
}

/// Hoy: el caso en git y `fusionar_puntero`. Devuelve `(caso, resultado)` y lo que queda en el
/// árbol fusionado.
fn por_git(c: &Caso, n: usize) -> (&'static str, &'static str, Huella) {
    let d = std::env::temp_dir().join(format!("ore-fusion-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    corre(&d, &["init", "-q", "-b", "main"]);
    commit_con(&d, &c.base, "base");
    let desde = corre(&d, &["rev-parse", "HEAD"]);
    corre(&d, &["checkout", "-q", "-b", "rama"]);
    match &c.rama {
        Some(r) => commit_con(&d, r, "rama"),
        None => {
            corre(&d, &["commit", "-q", "--allow-empty", "-m", "rama"]);
        }
    }
    corre(&d, &["checkout", "-q", "main"]);
    match &c.main {
        Some(m) => commit_con(&d, m, "main"),
        None => {
            corre(&d, &["commit", "-q", "--allow-empty", "-m", "main"]);
        }
    }
    // El nombre que `fusionar_puntero` da a `datasets/ventas/default/t.json` (lo comprueba abajo).
    let nombre = ore_core::normalize::corto("ventas", "default", "t");
    let elegidos: BTreeMap<String, String> = c
        .elegido
        .map(|e| BTreeMap::from([(nombre.clone(), e.to_string())]))
        .unwrap_or_default();
    let forja = Forja {
        url: String::new(),
        testigo: String::new(),
    };
    let dato = forja
        .fusionar_puntero(&d, &desde, "HEAD", "rama", RUTA, &elegidos)
        .unwrap_or_else(|e| panic!("{}: {e}", c.nombre));
    assert_eq!(dato.activo, nombre, "{}", c.nombre);
    let queda = huella(std::fs::read_to_string(d.join(RUTA)).ok().as_deref());
    let _ = std::fs::remove_dir_all(&d);
    (dato.caso, dato.resultado, queda)
}

fn cambio(p: &Option<Json>) -> CambioRuta {
    CambioRuta {
        rutas: vec![RUTA.into()],
        esperado: EsperadoMl::Cualquiera,
        que: p
            .as_ref()
            .map_or(QueRuta::Retirar, |p| QueRuta::Sustituir(p.clone())),
    }
}

/// Lo que el catálogo devuelve de un caso: `(caso, resultado, sin elegir, lo que queda en main
/// tras aplicar el plan)`.
type DelCatalogo = (String, String, bool, Huella);

/// El catálogo: el mismo caso, `plan_fusion` y, si se puede, `aplicar_plan`. En git el orden en
/// que escriben `main` y la rama no cuenta (cada uno en su commit); en el catálogo la rama lee
/// `main` al día, así que se prueban los dos órdenes.
fn por_catalogo(a: &mut Almacen, c: &Caso, rama_primero: bool) -> DelCatalogo {
    if c.base.is_some() {
        a.escribir("main", "s", "commit", None, &[cambio(&c.base)])
            .unwrap();
    }
    a.crear_rama("rama", "main").unwrap();
    let mut escritos: Vec<(&str, &Option<Json>)> = Vec::new();
    if let Some(m) = &c.main {
        escritos.push(("main", m));
    }
    if let Some(r) = &c.rama {
        escritos.push(("rama", r));
    }
    if rama_primero {
        escritos.reverse();
    }
    for (r, p) in escritos {
        a.escribir(r, "s", "commit", None, &[cambio(p)]).unwrap();
    }
    // Lo elegido va por ruta: quien llama pasa el nombre a sus rutas.
    let elegidos: BTreeMap<String, String> = c
        .elegido
        .map(|e| BTreeMap::from([(RUTA.to_string(), e.to_string())]))
        .unwrap_or_default();
    let plan = a
        .plan_fusion("rama", Some(&[RUTA.to_string()]), &elegidos)
        .unwrap();
    assert_eq!(plan.pasos.len(), 1, "{}", c.nombre);
    let p = &plan.pasos[0];
    if !plan.sin_elegir {
        a.aplicar_plan("rama", "sha", "s", &plan.texto, false)
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.nombre));
    }
    let queda = a
        .efectivo("main", &[RUTA.to_string()])
        .unwrap()
        .and_then(|(_, t)| huella(Some(&t)));
    (p.caso.clone(), p.resultado.clone(), plan.sin_elegir, queda)
}

/// ⚠ **La única diferencia, y no es de la regla:** los dos lo borraron, `main` primero. En git la
/// rama lo borró de su copia (`r = m = nada`: `promocion/rama`); en el catálogo la rama lee `main`
/// al día, así que cuando quiere borrarlo ya no está, no deja lápida y para el plan «no lo tocó»
/// (`promocion/main`). Lo que queda es lo mismo (nada) y ninguno bloquea. Con la rama primero, la
/// lápida existe y sale `promocion/rama`, como hoy.
fn diferencia_conocida(c: &Caso, rama_primero: bool) -> bool {
    !rama_primero
        && c.base.is_some()
        && matches!(c.main, Some(None))
        && matches!(c.rama, Some(None))
}

#[test]
fn la_fusion_del_catalogo_es_la_de_hoy() {
    let Ok(url) = std::env::var("ORE_CATALOGO_PRUEBAS") else {
        eprintln!("⚠ sin ORE_CATALOGO_PRUEBAS: la conformidad de la fusión no se prueba aquí");
        return;
    };
    let mut distintos = Vec::new();
    for (n, c) in casos().iter().enumerate() {
        let hoy = por_git(c, n);
        for rama_primero in [false, true] {
            let (_p, mut a) = EsquemaDePrueba::crear(&url, "f").expect("esquema de prueba");
            let (caso, resultado, sin_elegir, queda) = por_catalogo(&mut a, c, rama_primero);
            let orden = if rama_primero {
                "rama 1.º"
            } else {
                "main 1.º"
            };
            eprintln!(
                "{:<58} {:<5} {orden} hoy {:>11}/{:<10} catálogo {:>11}/{:<10}",
                c.nombre,
                c.elegido.unwrap_or("-"),
                hoy.0,
                hoy.1,
                caso,
                resultado
            );
            let bloquea_hoy = hoy.1 == "sin elegir";
            // Lo que queda en main: lo mismo (si no se bloquea; si se bloquea, nada se aplicó).
            let mismo_efecto = bloquea_hoy == sin_elegir && (sin_elegir || hoy.2 == queda);
            let misma_regla = (hoy.0, hoy.1) == (caso.as_str(), resultado.as_str());
            if diferencia_conocida(c, rama_primero) {
                assert_eq!((hoy.0, hoy.1), ("promocion", "rama"), "{}", c.nombre);
                assert_eq!((caso.as_str(), resultado.as_str()), ("promocion", "main"));
                assert!(mismo_efecto && queda.is_none(), "{}", c.nombre);
            } else if !(misma_regla && mismo_efecto) {
                distintos.push(format!("{} ({orden})", c.nombre));
            }
        }
    }
    assert!(distintos.is_empty(), "distintos: {distintos:?}");
}
