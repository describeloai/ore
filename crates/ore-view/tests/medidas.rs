//! **Las medidas.** Cuánto cuesta mantener, cuánto cuesta recomputar, y dónde
//! se cruzan.
//!
//! # Por qué se cuenta trabajo y no se mide tiempo
//!
//! Porque un reloj mide la máquina de quien mide —su caché, su carga, su
//! compilador— y este proyecto entero descansa en que dos ejecuciones den lo
//! mismo. Contar **filas miradas por un operador** mide el algoritmo: es
//! exacto, es entero y es reproducible, y se puede contar dentro de un motor
//! que tiene prohibido leer la hora.
//!
//! Por eso estas cifras se **afirman** en vez de imprimirse. Una medida que
//! cambia con la máquina no cabe en un `assert_eq!`; una que no cambia, sí — y
//! entonces deja de ser la anécdota de una tarde y pasa a ser algo que se rompe
//! si alguien empeora el motor.
//!
//! # Qué sale
//!
//! | | integradores | cruce sobre una base de 1000 |
//! |---|---|---|
//! | **lineal** — filtra y proyecta | 0 | **no hay** |
//! | **junta** — dos hojas por una clave | 2 | **no hay** |
//! | **agregado** — suma por grupo | 1 | **20 filas** con 20 grupos · **223** con 250 |
//!
//! Dos conclusiones, y las dos cambian lo que el Cost Model debería hacer:
//!
//! > **Con los integradores indexados, mantener gana siempre salvo en el
//! > agregado.** Un paso lineal mira el delta; uno de junta, el delta y lo que
//! > empareja. Ninguno mira la base. El único que la mira es el agregado, que
//! > tiene que releer los grupos que el Δ toca.
//!
//! > **Y dónde se cruza el agregado es dato, no plan.** El mismo documento, con
//! > veinte grupos y con doscientos cincuenta, se cruza en el 2 % y en el
//! > 22,3 % de la base. Un umbral global no puede acertar en los dos.
//!
//! Corre con `cargo test -p ore-view --test medidas -- --nocapture` para ver la
//! tabla.

use std::collections::{BTreeMap, BTreeSet};

use ore_core::types::parse_type;
use ore_view::delta_compiler::{Circuito, Fila, Trabajo, Zset, recomputar_contando};
use ore_view::plan::{Agregacion, Agregado, Comparador, Expr, Junta, Lectura, Nodo, Valor};
use ore_view::{Hoja, RefreshMode, analizar};

const PEDIDOS: (&str, &str) = ("lago", "ventas.pedidos");
const PAISES: (&str, &str) = ("lago", "ref.paises");

/// Filas de la base sobre la que se mide.
const BASE: u64 = 1000;
/// Cuántos países distintos en el caso corriente: `BASE / GRUPOS` filas por
/// grupo, que es lo que decide el coste de un paso de agregado.
const GRUPOS: u64 = 20;

fn t(s: &str) -> ore_core::types::Type {
    parse_type(s).expect("tipo")
}

fn pedidos() -> Nodo {
    Nodo::Lee(Lectura {
        datasource: PEDIDOS.0.into(),
        objeto: PEDIDOS.1.into(),
        campos: [
            ("id".to_string(), t("Integer")),
            ("pais".to_string(), t("String")),
            ("total".to_string(), t("Decimal")),
        ]
        .into(),
    })
}

fn paises() -> Nodo {
    Nodo::Lee(Lectura {
        datasource: PAISES.0.into(),
        objeto: PAISES.1.into(),
        campos: [
            ("codigo".to_string(), t("String")),
            ("region".to_string(), t("String")),
        ]
        .into(),
    })
}

fn pais(i: u64, grupos: u64) -> Valor {
    Valor::Cadena(format!("p{:03}", i % grupos))
}

/// `n` pedidos repartidos entre `grupos` países. Determinista: las mismas
/// entradas dan las mismas filas, que es lo que permite afirmar un número.
fn filas_pedidos(desde: u64, n: u64, grupos: u64) -> Zset {
    Zset::de((desde..desde + n).map(|i| {
        (
            Fila::from([
                ("id".to_string(), Valor::Entero(i as i64)),
                ("pais".to_string(), pais(i, grupos)),
                ("total".to_string(), Valor::Decimal(format!("{i}"))),
            ]),
            1,
        )
    }))
}

fn filas_paises(grupos: u64) -> Zset {
    Zset::de((0..grupos).map(|i| {
        (
            Fila::from([
                ("codigo".to_string(), pais(i, grupos)),
                ("region".to_string(), Valor::Cadena(format!("r{}", i % 4))),
            ]),
            1,
        )
    }))
}

// ── las tres formas ─────────────────────────────────────────────────────────

fn lineal() -> Nodo {
    Nodo::Proyecta {
        entrada: Box::new(Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Compara {
                op: Comparador::Distinto,
                izquierda: Box::new(Expr::campo("pais")),
                derecha: Box::new(Expr::Literal(Valor::Cadena("p019".into()))),
            },
        }),
        campos: [
            ("pais".to_string(), Expr::campo("pais")),
            ("total".to_string(), Expr::campo("total")),
        ]
        .into(),
    }
}

fn junta() -> Nodo {
    Nodo::Proyecta {
        entrada: Box::new(Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(paises()),
            tipo: Junta::Interna,
            sobre: vec![("pais".to_string(), "codigo".to_string())],
        }),
        campos: [
            ("pais".to_string(), Expr::campo("pais")),
            ("region".to_string(), Expr::campo("region")),
            ("total".to_string(), Expr::campo("total")),
        ]
        .into(),
    }
}

fn agregado() -> Nodo {
    Nodo::Agrupa {
        entrada: Box::new(pedidos()),
        por: BTreeSet::from(["pais".to_string()]),
        agregados: BTreeMap::from([(
            "suma".to_string(),
            Agregacion {
                funcion: Agregado::Suma,
                sobre: Some("total".to_string()),
            },
        )]),
    }
}

// ── la medición ─────────────────────────────────────────────────────────────

fn hojas(pedidos: Zset, paises: Option<u64>) -> BTreeMap<Hoja, Zset> {
    let mut m = BTreeMap::new();
    m.insert((PEDIDOS.0.to_string(), PEDIDOS.1.to_string()), pedidos);
    if let Some(g) = paises {
        m.insert(
            (PAISES.0.to_string(), PAISES.1.to_string()),
            filas_paises(g),
        );
    }
    m
}

/// Lo que cuesta **un paso** de `d` filas sobre una base de [`BASE`], y lo que
/// costaría recomputar la vista entera con esas mismas filas dentro.
///
/// El circuito se ceba primero con la base —que es lo que hace una sesión al
/// arrancar— y lo que se mide es el paso **siguiente**. Medir el primero sería
/// medir la carga inicial, que nadie llama mantenimiento.
fn medir(plan: &Nodo, con_paises: bool, grupos: u64, d: u64) -> (Trabajo, Trabajo) {
    let dim = con_paises.then_some(grupos);
    let mut c = Circuito::compilar(plan).expect("se mantiene");
    c.paso(&hojas(filas_pedidos(0, BASE, grupos), dim))
        .expect("la carga inicial");
    let antes = c.trabajo();
    // El delta trae solo pedidos: la tabla de dimensión ya está y no cambia,
    // que es lo que la hace una tabla de dimensión.
    c.paso(&hojas(filas_pedidos(BASE, d, grupos), None))
        .expect("el paso");
    let incremental = c.trabajo() - antes;

    let (_, recomputo) = recomputar_contando(plan, &hojas(filas_pedidos(0, BASE + d, grupos), dim))
        .expect("recomputa");
    (incremental, recomputo)
}

/// El **cruce**: el delta más pequeño para el que mantener ya no sale más
/// barato que recomputar. `None` si no lo hay ni con un delta del tamaño de la
/// base — que es un resultado y no un fallo de la medición.
///
/// Por bisección, porque medir un punto cuesta cebar un circuito entero. Y **se
/// comprueba que el punto encontrado es un cruce**: que en `d` mantener ya no
/// gana y en `d-1` todavía ganaba. Una bisección sobre algo que no fuera
/// monótono daría un número con esta misma cara y sin significado.
fn cruce(plan: &Nodo, con_paises: bool, grupos: u64) -> Option<u64> {
    let cruza = |d: u64| {
        let (i, r) = medir(plan, con_paises, grupos, d);
        i >= r
    };
    if !cruza(BASE) {
        return None;
    }
    let (mut lo, mut hi) = (1, BASE);
    while lo < hi {
        let medio = lo + (hi - lo) / 2;
        if cruza(medio) {
            hi = medio;
        } else {
            lo = medio + 1;
        }
    }
    assert!(cruza(lo), "el cruce tiene que cruzar");
    assert!(lo == 1 || !cruza(lo - 1), "y el de antes, no");
    Some(lo)
}

/// Un porcentaje con un decimal, **en aritmética entera**. No hay coma flotante
/// en este proyecto, tampoco para enseñar una cifra.
fn por_ciento(num: u64, den: u64) -> String {
    let m = num * 1000 / den;
    format!("{},{} %", m / 10, m % 10)
}

// ── lo que sale ─────────────────────────────────────────────────────────────

/// **Un plan lineal no tiene cruce: mantener siempre gana.**
///
/// Y no es una casualidad de estos números. Un paso mira las filas del delta y
/// un recómputo mira las de la base entera, así que mientras el delta sea más
/// pequeño que la base —que es lo que significa que sea un delta— incrementar
/// gana. Es la forma en la que el Cost Model no tiene nada que decidir.
#[test]
fn un_plan_lineal_no_tiene_cruce() {
    let plan = lineal();
    assert!(matches!(
        analizar(&plan),
        RefreshMode::Incremental { ref state } if state.is_empty()
    ));

    // Un delta de una fila: dos miradas —filtrarla y proyectarla— contra las
    // 1952 de recomputar. Tres órdenes de magnitud.
    let (i1, r1) = medir(&plan, false, GRUPOS, 1);
    assert_eq!((i1, r1), (2, 1952));

    // Ni siquiera con un delta del tamaño de la base entera.
    let (im, rm) = medir(&plan, false, GRUPOS, BASE);
    assert!(im < rm, "{im} < {rm}");
    assert_eq!(cruce(&plan, false, GRUPOS), None);

    println!("\n  lineal     Δ=1  mantener {i1:>6}   recomputar {r1:>6}   sin cruce");
}

/// **Una junta tampoco se cruza**, y esa cifra es la que dice que el integrador
/// está indexado de verdad.
///
/// Un paso cuesta el delta y lo que empareja: probar contra el índice del otro
/// lado y guardarse en el suyo. Nada proporcional a la base. Con el integrador
/// **plano** que esta máquina tuvo hasta hoy, `I(a)⋈Δb` recorría el integrador
/// entero, así que un paso costaba la base y mantener no ganaba nunca — la
/// incrementalización estaba escrita y no ocurría.
///
/// Lo destapó intentar medirlo. **Medir una pieza es la forma más barata de
/// descubrir que no hace lo que dice.**
#[test]
fn una_junta_con_el_integrador_indexado_tampoco_se_cruza() {
    let plan = junta();
    assert!(matches!(
        analizar(&plan),
        RefreshMode::Incremental { ref state } if state.len() == 2
    ));

    let (i1, r1) = medir(&plan, true, GRUPOS, 1);
    assert_eq!((i1, r1), (5, 3023));
    assert_eq!(cruce(&plan, true, GRUPOS), None);

    println!("  junta      Δ=1  mantener {i1:>6}   recomputar {r1:>6}   sin cruce");
}

/// **El agregado sí se cruza, y dónde depende de los datos.**
///
/// Un paso recomputa los grupos que el Δ toca, dos veces —antes y después— así
/// que cuesta lo que pesan esos grupos. Es el único operador cuyo paso mira
/// filas que no venían en el delta.
///
/// Y de ahí sale la conclusión que decide qué debería hacer el Cost Model: el
/// **mismo plan**, con veinte grupos y con doscientos cincuenta, se cruza en el
/// 2 % y en el 22,3 % de la base. La diferencia no está en el documento. **Un
/// umbral global no puede acertar en los dos**, y el 5 % de Snowflake cae justo
/// entre medias.
#[test]
fn el_agregado_se_cruza_y_donde_depende_de_los_datos() {
    let plan = agregado();
    assert!(matches!(
        analizar(&plan),
        RefreshMode::Incremental { ref state } if state.len() == 1
    ));

    // 20 grupos · 50 filas por grupo.
    let (i1, r1) = medir(&plan, false, GRUPOS, 1);
    assert_eq!((i1, r1), (103, 2002));
    let gordos = cruce(&plan, false, GRUPOS).expect("se cruza");
    assert_eq!(gordos, 20);
    assert_eq!(por_ciento(gordos, BASE), "2,0 %");

    // 250 grupos · 4 filas por grupo. El mismo plan, otros datos.
    let (i2, r2) = medir(&plan, false, 250, 1);
    assert_eq!((i2, r2), (11, 2002));
    let finos = cruce(&plan, false, 250).expect("se cruza");
    assert_eq!(finos, 223);
    assert_eq!(por_ciento(finos, BASE), "22,3 %");

    // Y el 5 % de Snowflake queda entre los dos: recomputaría de menos en el
    // primero y de más en el segundo.
    let cinco_por_ciento = BASE * 5 / 100;
    assert!(gordos < cinco_por_ciento && cinco_por_ciento < finos);

    println!(
        "  agregado   Δ=1  mantener {i1:>6}   recomputar {r1:>6}   cruce en Δ={gordos} (2,0 %) · 20 grupos"
    );
    println!(
        "             Δ=1  mantener {i2:>6}   recomputar {r2:>6}   cruce en Δ={finos} (22,3 %) · 250 grupos\n"
    );
}

/// **Indexar los integradores no cambió ni una fila.**
///
/// Los dos índices nuevos —el de la junta y el del agregado— cambian *cuánto*
/// cuesta un paso, y no pueden cambiar *qué* sale. Se comprueba sobre las tres
/// formas, con altas y con una retractación, porque una baja es donde un índice
/// mal mantenido se nota.
#[test]
fn indexar_los_integradores_no_cambio_ni_una_fila() {
    for (nombre, plan, con_paises) in [
        ("lineal", lineal(), false),
        ("junta", junta(), true),
        ("agregado", agregado(), false),
    ] {
        let dim = con_paises.then_some(GRUPOS);
        let mut c = Circuito::compilar(&plan).expect("se mantiene");
        let mut acumulado = Zset::nuevo();
        for (i, delta) in [
            filas_pedidos(0, 50, GRUPOS),
            filas_pedidos(50, 25, GRUPOS),
            // Se van diez de las primeras.
            filas_pedidos(0, 10, GRUPOS).negado(),
        ]
        .into_iter()
        .enumerate()
        {
            let d = c
                .paso(&hojas(delta, if i == 0 { dim } else { None }))
                .expect("el paso");
            acumulado.sumar(&d);
        }

        let (de_una_vez, _) =
            recomputar_contando(&plan, &hojas(filas_pedidos(10, 65, GRUPOS), dim)).unwrap();
        assert_eq!(acumulado, de_una_vez, "{nombre}");
    }
}
