//! **Las escrituras de definición** (0079 §19.2, E1a·5c·5c·1): la v3 contra un Postgres de verdad.
//! Los cerrojos de una fusión (y el 429 de `escribir`), el plan de fusión caso por caso (las reglas
//! de hoy y las tres diferencias de Q1), la aplicación idempotente por `git:<sha>`, la tardía, la
//! rama al día después, y la migración v2 → v3.
//!
//! Necesita `ORE_CATALOGO_PRUEBAS` (ver `la_puerta_en_el_catalogo.rs`); sin ella, cada prueba lo
//! dice y no mide nada. Cada una trabaja en su esquema y lo borra al acabar.

use std::collections::BTreeMap;
use std::time::Duration;

use ore_catalogo::almacen::{
    Almacen, CambioRuta, EsperadoMl, EsquemaDePrueba, FalloAplicar, FalloCerrar, FalloEscritura,
    QueRuta,
};
use ore_core::json::Json;
use postgres::{Client, NoTls};

fn prueba() -> Option<(EsquemaDePrueba, Almacen)> {
    let Ok(url) = std::env::var("ORE_CATALOGO_PRUEBAS") else {
        eprintln!("⚠ sin ORE_CATALOGO_PRUEBAS: las escrituras de definición no se prueban aquí");
        return None;
    };
    Some(EsquemaDePrueba::crear(&url, "d").expect("esquema de prueba"))
}

fn ruta(n: &str) -> String {
    format!("datasets/ventas/default/{n}.json")
}

/// Un puntero entero: su `metadata_location` (y el `dataset` que el catálogo le pone), un campo
/// que no es la ubicación (`comprobado`) y, si lo escribió alguien, `escrito_por`.
fn puntero(n: &str, v: u32, comprobado: &str, escrito_por: Option<&str>) -> Json {
    let mut campos = vec![
        (
            "metadata_location",
            Json::s(format!(
                "gs://b/ore/v2/catalogo/ventas/default/{n}/metadata/{v:05}-x.metadata.json"
            )),
        ),
        ("dataset", Json::s(format!("catalogo/ventas/default/{n}"))),
        ("comprobado", Json::s(comprobado)),
    ];
    if let Some(p) = escrito_por {
        campos.push(("escrito_por", Json::s(p)));
    }
    Json::obj(campos)
}

fn sustituir(n: &str, p: &Json) -> CambioRuta {
    CambioRuta {
        rutas: vec![ruta(n)],
        esperado: EsperadoMl::Cualquiera,
        que: QueRuta::Sustituir(p.clone()),
    }
}

fn retirar(n: &str) -> CambioRuta {
    CambioRuta {
        rutas: vec![ruta(n)],
        esperado: EsperadoMl::Cualquiera,
        que: QueRuta::Retirar,
    }
}

fn en(a: &mut Almacen, r: &str, c: CambioRuta) {
    a.escribir(r, "persona:ana", "commit", None, &[c]).unwrap();
}

/// El texto que ve una ref en `n`, como `jsonb` normalizado (para comparar con un [`Json`]).
fn ve(a: &mut Almacen, r: &str, n: &str) -> Option<String> {
    a.efectivo(r, &[ruta(n)]).unwrap().map(|(_, t)| t)
}

fn jsonb(c: &mut Client, j: &Json) -> String {
    c.query_one("SELECT ($1::text::jsonb)::text", &[&j.pretty()])
        .unwrap()
        .get(0)
}

fn crudo(p: &EsquemaDePrueba) -> Client {
    let url = std::env::var("ORE_CATALOGO_PRUEBAS").unwrap();
    let mut c = Client::connect(&url, NoTls).unwrap();
    c.batch_execute(&format!("SET search_path TO {}", p.esquema))
        .unwrap();
    c
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

// ── Los cerrojos ────────────────────────────────────────────────────────────────────────────────

#[test]
fn un_cerrojo_da_429_hasta_soltarlo_y_uno_caducado_no_cuenta() {
    let Some((_p, mut a)) = prueba() else { return };
    en(&mut a, "main", sustituir("a", &puntero("a", 1, "x", None)));
    a.crear_rama("r", "main").unwrap();
    a.cerrar(
        &s(&["main", "r"]),
        &[ruta("a")],
        "fusion:1",
        Duration::from_secs(60),
    )
    .unwrap();
    // En main y en la rama: 429, en el cambio que toca lo cerrado.
    for r in ["main", "r"] {
        let f = a
            .escribir(
                r,
                "s",
                "commit",
                None,
                &[
                    sustituir("b", &puntero("b", 1, "x", None)),
                    sustituir("a", &puntero("a", 2, "x", None)),
                ],
            )
            .unwrap_err();
        assert_eq!(f, FalloEscritura::Cerrado { indice: 1 }, "en {r}");
    }
    assert!(ve(&mut a, "main", "b").is_none(), "todo o nada");
    // Lo que no está cerrado se escribe.
    en(&mut a, "main", sustituir("b", &puntero("b", 1, "x", None)));
    // Una repetición de algo ya escrito devuelve su commit aunque ahora esté cerrado.
    let id = a.escribir("main", "s", "commit", Some("k1"), &[]).unwrap();
    assert_eq!(
        a.escribir("main", "s", "commit", Some("k1"), &[retirar("a")]),
        Ok(id)
    );
    // Otro dueño no puede cerrar lo mismo; el mismo, renueva.
    assert!(matches!(
        a.cerrar(&s(&["main"]), &[ruta("a")], "fusion:2", Duration::from_secs(60)),
        Err(FalloCerrar::Ocupado { ref dueno, .. }) if dueno == "fusion:1"
    ));
    a.cerrar(
        &s(&["main"]),
        &[ruta("a")],
        "fusion:1",
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(a.soltar("fusion:1").unwrap(), 2);
    en(&mut a, "main", sustituir("a", &puntero("a", 2, "x", None)));
    // Caducado: no cuenta, y otro dueño lo pisa.
    a.cerrar(&s(&["main"]), &[ruta("a")], "fusion:3", Duration::ZERO)
        .unwrap();
    en(&mut a, "main", sustituir("a", &puntero("a", 3, "x", None)));
    a.cerrar(
        &s(&["main"]),
        &[ruta("a")],
        "fusion:4",
        Duration::from_secs(60),
    )
    .unwrap();
    assert!(matches!(
        a.escribir("main", "s", "commit", None, &[retirar("a")]),
        Err(FalloEscritura::Cerrado { indice: 0 })
    ));
    // Dos dueños en rutas distintas conviven.
    a.cerrar(
        &s(&["main"]),
        &[ruta("b")],
        "fusion:5",
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(a.soltar("fusion:4").unwrap(), 1);
    en(&mut a, "main", retirar("a"));
    assert!(matches!(
        a.escribir("main", "s", "commit", None, &[retirar("b")]),
        Err(FalloEscritura::Cerrado { .. })
    ));
    // El cerrojo de main no cierra la rama (la fusión cierra las dos refs a propósito).
    en(&mut a, "r", sustituir("b", &puntero("b", 9, "x", None)));
}

// ── El plan, caso por caso ──────────────────────────────────────────────────────────────────────

/// Un caso: lo de `main` en el punto de salida, y lo que escriben luego main y la rama
/// (`None` = nada; `Some(None)` = retirar).
struct Caso {
    nombre: &'static str,
    base: Option<Json>,
    main: Option<Option<Json>>,
    rama: Option<Option<Json>>,
    elegido: Option<&'static str>,
    esperado: (&'static str, &'static str),
}

fn casos() -> Vec<Caso> {
    let p = |v, c, e: Option<&str>| puntero("t", v, c, e);
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
            esperado: ("promocion", "main"),
        },
        Caso {
            nombre: "solo la rama lo cambió",
            base: base(),
            main: None,
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
            esperado: ("promocion", "rama"),
        },
        Caso {
            nombre: "solo la rama lo borró",
            base: base(),
            main: None,
            rama: Some(None),
            elegido: None,
            esperado: ("promocion", "rama"),
        },
        Caso {
            nombre: "nació en la rama",
            base: None,
            main: None,
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
            esperado: ("promocion", "rama"),
        },
        Caso {
            nombre: "los dos lo mismo",
            base: base(),
            main: Some(Some(p(3, "x", ana))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
            esperado: ("promocion", "rama"),
        },
        Caso {
            nombre: "los dos, con receta",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: Some(Some(p(3, "x", None))),
            elegido: None,
            esperado: ("reconstruir", "main"),
        },
        // Q1 ②: la rama reescribió lo mismo y main se movió: no lo tocó.
        Caso {
            nombre: "la rama reescribió lo mismo y main se movió",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: Some(Some(p(1, "x", None))),
            elegido: None,
            esperado: ("promocion", "main"),
        },
        // Q1 ①: main solo refrescó un campo que no es la ubicación: es cambio (el puntero entero).
        Caso {
            nombre: "main refrescó otro campo y la rama escribió",
            base: base(),
            main: Some(Some(p(1, "y", None))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
            esperado: ("conflicto", "sin elegir"),
        },
        Caso {
            nombre: "main refrescó otro campo y la rama reconstruyó",
            base: base(),
            main: Some(Some(p(1, "y", None))),
            rama: Some(Some(p(3, "x", None))),
            elegido: None,
            esperado: ("reconstruir", "main"),
        },
        // Q1 ③: un borrado nunca tiene receta.
        Caso {
            nombre: "la rama lo borró y main lo cambió",
            base: base(),
            main: Some(Some(p(2, "x", None))),
            rama: Some(None),
            elegido: None,
            esperado: ("conflicto", "sin elegir"),
        },
        // Main primero: cuando la rama quiere borrarlo ya no está y no deja lápida: «no lo tocó».
        // En git es `promocion/rama` (la conformidad de ore-serve lo dice); queda lo mismo: nada.
        Caso {
            nombre: "los dos lo borraron",
            base: base(),
            main: Some(None),
            rama: Some(None),
            elegido: None,
            esperado: ("promocion", "main"),
        },
        Caso {
            nombre: "nació en los dos, distinto",
            base: None,
            main: Some(Some(p(2, "x", leo))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: None,
            esperado: ("conflicto", "sin elegir"),
        },
    ];
    for (e, r) in [
        (Some("rama"), "rama"),
        (Some("main"), "main"),
        (None, "sin elegir"),
    ] {
        v.push(Caso {
            nombre: "los dos, sin receta",
            base: base(),
            main: Some(Some(p(2, "x", leo))),
            rama: Some(Some(p(3, "x", ana))),
            elegido: e,
            esperado: ("conflicto", r),
        });
    }
    v
}

/// Monta el caso en el catálogo: main con la base, la rama nace, y escriben los dos.
fn montar(a: &mut Almacen, c: &Caso) {
    if let Some(b) = &c.base {
        en(a, "main", sustituir("t", b));
    }
    a.crear_rama("r", "main").unwrap();
    if let Some(m) = &c.main {
        en(
            a,
            "main",
            m.as_ref().map_or(retirar("t"), |p| sustituir("t", p)),
        );
    }
    if let Some(r) = &c.rama {
        en(
            a,
            "r",
            r.as_ref().map_or(retirar("t"), |p| sustituir("t", p)),
        );
    }
}

fn elegidos(c: &Caso) -> BTreeMap<String, String> {
    c.elegido
        .map(|e| BTreeMap::from([(ruta("t"), e.to_string())]))
        .unwrap_or_default()
}

#[test]
fn el_plan_caso_por_caso() {
    for c in casos() {
        let Some((_p, mut a)) = prueba() else { return };
        montar(&mut a, &c);
        let plan = a
            .plan_fusion("r", Some(&[ruta("t")]), &elegidos(&c))
            .unwrap();
        assert_eq!(plan.pasos.len(), 1, "{}", c.nombre);
        let paso = &plan.pasos[0];
        assert_eq!(
            (paso.caso.as_str(), paso.resultado.as_str()),
            c.esperado,
            "{} ({:?})",
            c.nombre,
            c.elegido
        );
        assert_eq!(
            plan.sin_elegir,
            c.esperado.1 == "sin elegir",
            "{}",
            c.nombre
        );
        // Sin alcance: todo lo propio de la rama; lo que no tocó no sale.
        let todo = a.plan_fusion("r", None, &elegidos(&c)).unwrap();
        let propio = c.rama.is_some() && !(c.rama == Some(None) && c.main == Some(None));
        assert_eq!(todo.pasos.len(), usize::from(propio), "{}", c.nombre);
    }
}

// ── Aplicar ─────────────────────────────────────────────────────────────────────────────────────

/// main: a, b, c, d. La rama cambia a (sola), borra b (sola), hace nacer e, reconstruye c (main
/// también lo movió) y deja d como estaba.
fn escenario(a: &mut Almacen) {
    for n in ["a", "b", "c", "d"] {
        en(a, "main", sustituir(n, &puntero(n, 1, "x", None)));
    }
    a.crear_rama("r", "main").unwrap();
    en(a, "main", sustituir("c", &puntero("c", 2, "x", None)));
    en(a, "r", sustituir("a", &puntero("a", 3, "x", Some("ana"))));
    en(a, "r", retirar("b"));
    en(a, "r", sustituir("e", &puntero("e", 1, "x", Some("ana"))));
    en(a, "r", sustituir("c", &puntero("c", 3, "x", None)));
    en(a, "r", sustituir("d", &puntero("d", 1, "x", None)));
}

#[test]
fn aplicar_es_idempotente_por_el_sha_y_la_rama_se_pone_al_dia() {
    let Some((p, mut a)) = prueba() else { return };
    escenario(&mut a);
    let plan = a.plan_fusion("r", None, &BTreeMap::new()).unwrap();
    assert!(!plan.sin_elegir);
    let caso: BTreeMap<String, (String, String)> = plan
        .pasos
        .iter()
        .map(|p| (p.ruta.clone(), (p.caso.clone(), p.resultado.clone())))
        .collect();
    let par = |c: &str, r: &str| (c.to_string(), r.to_string());
    assert_eq!(caso[&ruta("a")], par("promocion", "rama"));
    assert_eq!(caso[&ruta("b")], par("promocion", "rama"));
    assert_eq!(caso[&ruta("c")], par("reconstruir", "main"));
    assert_eq!(caso[&ruta("d")], par("promocion", "main"));
    assert_eq!(caso[&ruta("e")], par("promocion", "rama"));

    assert!(!a.aplicado("abc").unwrap());
    let antes = a.cuentas().unwrap();
    let hecha = a
        .aplicar_plan("r", "abc", "persona:ana", &plan.texto, false)
        .unwrap();
    assert!(!hecha.repetida && hecha.pendientes.is_empty());
    assert_eq!(hecha.aplicados, vec![ruta("a"), ruta("b"), ruta("e")]);
    assert!(a.aplicado("abc").unwrap());
    let mut c = crudo(&p);
    // main: a y e los de la rama, b fuera, c y d los suyos.
    assert_eq!(
        ve(&mut a, "main", "a"),
        Some(jsonb(&mut c, &puntero("a", 3, "x", Some("ana"))))
    );
    assert_eq!(ve(&mut a, "main", "b"), None);
    assert_eq!(
        ve(&mut a, "main", "c"),
        Some(jsonb(&mut c, &puntero("c", 2, "x", None)))
    );
    assert_eq!(
        ve(&mut a, "main", "e"),
        Some(jsonb(&mut c, &puntero("e", 1, "x", Some("ana"))))
    );
    // La rama, al día: nada propio, y ve lo de main.
    let propios: i64 = c
        .query_one("SELECT count(*) FROM estado WHERE ref = 'r'", &[])
        .unwrap()
        .get(0);
    assert_eq!(propios, 0);
    for n in ["a", "b", "c", "d", "e"] {
        assert_eq!(ve(&mut a, "r", n), ve(&mut a, "main", n), "{n}");
    }
    let (punto, ultimo): (i64, i64) = {
        let f = c
            .query_one(
                "SELECT (SELECT punto_salida FROM refs WHERE nombre = 'r'),
                        (SELECT max(id) FROM commits WHERE ref = 'main')",
                &[],
            )
            .unwrap();
        (f.get(0), f.get(1))
    };
    assert_eq!(punto, ultimo);
    assert_eq!(punto, hecha.commit);
    // Historia y outbox: tres movimientos.
    let despues = a.cuentas().unwrap();
    assert_eq!(despues[0] - antes[0], 1, "un commit");
    assert_eq!(despues[2] - antes[2], 3, "historia");
    assert_eq!(despues[3] - antes[3], 3, "outbox");

    // La segunda vez: lo mismo, sin tocar nada.
    let otra = a
        .aplicar_plan("r", "abc", "persona:ana", &plan.texto, false)
        .unwrap();
    assert!(otra.repetida);
    assert_eq!(otra.commit, hecha.commit);
    assert_eq!(otra.aplicados, hecha.aplicados);
    assert_eq!(a.cuentas().unwrap(), despues);
}

#[test]
fn un_plan_sin_elegir_o_de_otra_rama_no_se_aplica() {
    let Some((_p, mut a)) = prueba() else { return };
    let c = casos()
        .into_iter()
        .find(|c| c.nombre == "los dos, sin receta" && c.elegido.is_none())
        .unwrap();
    assert_eq!(c.esperado.1, "sin elegir");
    montar(&mut a, &c);
    let plan = a.plan_fusion("r", None, &BTreeMap::new()).unwrap();
    assert!(plan.sin_elegir);
    assert!(matches!(
        a.aplicar_plan("r", "s1", "x", &plan.texto, false),
        Err(FalloAplicar::NoSeAplica(_))
    ));
    a.crear_rama("otra", "main").unwrap();
    let plan = a
        .plan_fusion(
            "r",
            None,
            &BTreeMap::from([(ruta("t"), "rama".to_string())]),
        )
        .unwrap();
    assert!(matches!(
        a.aplicar_plan("otra", "s1", "x", &plan.texto, false),
        Err(FalloAplicar::NoSeAplica(_))
    ));
    assert!(!a.aplicado("s1").unwrap());
}

#[test]
fn con_el_cerrojo_puesto_main_no_se_mueve_y_si_se_mueve_es_un_fallo_del_protocolo() {
    let Some((_p, mut a)) = prueba() else { return };
    escenario(&mut a);
    let plan = a.plan_fusion("r", None, &BTreeMap::new()).unwrap();
    // Alguien se salta el cerrojo (no debería poder): sin tardía, nada se aplica.
    en(&mut a, "main", sustituir("a", &puntero("a", 7, "x", None)));
    assert!(matches!(
        a.aplicar_plan("r", "s2", "x", &plan.texto, false),
        Err(FalloAplicar::Protocolo(_))
    ));
    assert!(!a.aplicado("s2").unwrap());
    assert!(ve(&mut a, "main", "e").is_none(), "nada se aplicó");
}

#[test]
fn la_tardia_deja_lo_de_main_y_lo_dice_pendiente() {
    let Some((p, mut a)) = prueba() else { return };
    escenario(&mut a);
    let rutas: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|n| ruta(n)).collect();
    a.cerrar(&s(&["main", "r"]), &rutas, "fusion:9", Duration::ZERO)
        .unwrap();
    let plan = a.plan_fusion("r", Some(&rutas), &BTreeMap::new()).unwrap();
    // El cerrojo caducó: alguien escribe a en main antes de aplicar.
    en(
        &mut a,
        "main",
        sustituir("a", &puntero("a", 7, "x", Some("leo"))),
    );
    let hecha = a.aplicar_plan("r", "s3", "x", &plan.texto, true).unwrap();
    assert!(hecha.tardia);
    assert_eq!(hecha.aplicados, vec![ruta("b"), ruta("e")]);
    assert_eq!(hecha.pendientes.len(), 1);
    assert_eq!(hecha.pendientes[0].ruta, ruta("a"));
    let mut c = crudo(&p);
    let leo = jsonb(&mut c, &puntero("a", 7, "x", Some("leo")));
    let ana = jsonb(&mut c, &puntero("a", 3, "x", Some("ana")));
    assert_eq!(hecha.pendientes[0].se_queda.as_deref(), Some(leo.as_str()));
    assert_eq!(hecha.pendientes[0].no_entra.as_deref(), Some(ana.as_str()));
    // Gana main; lo pendiente se queda en la rama, que sigue viéndolo suyo.
    assert_eq!(ve(&mut a, "main", "a"), Some(leo));
    assert_eq!(ve(&mut a, "r", "a"), Some(ana));
    assert!(ve(&mut a, "main", "b").is_none());
    // Repetida, igual (con lo pendiente).
    let otra = a.aplicar_plan("r", "s3", "x", &plan.texto, true).unwrap();
    assert!(otra.repetida);
    assert_eq!(otra.pendientes, hecha.pendientes);
}

#[test]
fn aplicar_una_definicion_es_escribir_idempotente_por_el_sha() {
    let Some((_p, mut a)) = prueba() else { return };
    en(&mut a, "main", sustituir("a", &puntero("a", 1, "x", None)));
    let id = a
        .aplicar_definicion("main", "d1", "persona:ana", "retirar", &[retirar("a")])
        .unwrap();
    assert!(a.aplicado("d1").unwrap());
    assert!(ve(&mut a, "main", "a").is_none());
    en(&mut a, "main", sustituir("a", &puntero("a", 2, "x", None)));
    assert_eq!(
        a.aplicar_definicion("main", "d1", "persona:ana", "retirar", &[retirar("a")]),
        Ok(id),
        "la segunda vez no toca nada"
    );
    assert!(ve(&mut a, "main", "a").is_some());
    a.cerrar(&s(&["main"]), &[ruta("a")], "f", Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        a.aplicar_definicion("main", "d2", "persona:ana", "retirar", &[retirar("a")]),
        Err(FalloEscritura::Cerrado { indice: 0 })
    );
    assert!(!a.aplicado("d2").unwrap());
}

// ── La migración ────────────────────────────────────────────────────────────────────────────────

#[test]
fn de_v2_a_v3_sin_perder_nada() {
    let Ok(url) = std::env::var("ORE_CATALOGO_PRUEBAS") else {
        eprintln!("⚠ sin ORE_CATALOGO_PRUEBAS: la migración v2 → v3 no se prueba aquí");
        return;
    };
    let (p, mut a) = EsquemaDePrueba::crear(&url, "m").unwrap();
    en(&mut a, "main", sustituir("a", &puntero("a", 1, "x", None)));
    a.crear_rama("r", "main").unwrap();
    en(
        &mut a,
        "r",
        sustituir("a", &puntero("a", 2, "x", Some("ana"))),
    );
    drop(a);
    // Se deshace la v3 a mano: el esquema queda como lo dejó un binario de la v2.
    let mut c = crudo(&p);
    c.batch_execute(
        "DROP FUNCTION aplicado(text); DROP FUNCTION aplicar_definicion(text, text, text, text, jsonb);
         DROP FUNCTION aplicar_plan(text, text, text, jsonb, boolean);
         DROP FUNCTION traer_main_limpiando(text);
         DROP FUNCTION plan_fusion(text, text[], jsonb); DROP FUNCTION main_puntero_en(text, bigint);
         DROP FUNCTION escribir(text, text, text, text, jsonb);
         ALTER FUNCTION escribir_sin_cerrojos(text, text, text, text, jsonb) RENAME TO escribir;
         DROP FUNCTION soltar(text); DROP FUNCTION cerrar(text[], text[], text, interval);
         DROP FUNCTION tomar_rutas(text[], text[]); DROP TABLE cerrojos;
         ALTER TABLE commits DROP COLUMN resultado;
         DELETE FROM esquema_version WHERE version = 3;",
    )
    .unwrap();
    let mut a = Almacen::conectar(&url, &p.esquema).unwrap();
    let v: i32 = c
        .query_one("SELECT max(version) FROM esquema_version", &[])
        .unwrap()
        .get(0);
    assert_eq!(v, 3);
    assert_eq!(
        ve(&mut a, "main", "a"),
        Some(jsonb(&mut c, &puntero("a", 1, "x", None)))
    );
    assert_eq!(
        ve(&mut a, "r", "a"),
        Some(jsonb(&mut c, &puntero("a", 2, "x", Some("ana"))))
    );
    let plan = a.plan_fusion("r", None, &BTreeMap::new()).unwrap();
    assert_eq!(plan.pasos[0].resultado, "rama");
}
