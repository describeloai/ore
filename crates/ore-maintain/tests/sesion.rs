//! El mantenedor delegado, ejercido por su protocolo.
//!
//! # Lo que de verdad se afirma aquí
//!
//! Que **el circuito Δ y el estado parcial corren**, que era lo único que las
//! doce piezas del motor no habían hecho nunca: el Delta Compiler era la
//! semántica y el Partial State Store el contrato, y los dos se probaban dentro
//! de su crate con planes escritos a mano. Aquí hay un proceso, un protocolo y
//! una sesión que dura.
//!
//! Y la afirmación que sostiene todo lo demás:
//!
//! > **Lo que sale de mantener es lo que saldría de recomputar.**
//!
//! Si eso no fuera cierto, el resto —el dictamen, la *upquery*, el desalojo— no
//! valdría nada, porque estaría acelerando una respuesta equivocada.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::process::{Command, Stdio};

use ore_core::json::Json;
use ore_core::parse::{Node, parse};
use ore_core::types::parse_type;
use ore_maintain::Sesion;
use ore_view::Hoja;
use ore_view::delta_compiler::{Fila, Zset, recomputar};
use ore_view::plan::{Expr, Junta, Lectura, Nodo, Valor};

const BUNDLE: &str = "sha256:2b3c";

fn t(s: &str) -> ore_core::types::Type {
    parse_type(s).expect("tipo")
}

fn pedidos() -> Nodo {
    Nodo::Lee(Lectura {
        datasource: "lago".into(),
        objeto: "ventas.pedidos".into(),
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
        datasource: "referencias".into(),
        objeto: "ref.paises".into(),
        campos: [
            ("codigo".to_string(), t("String")),
            ("region".to_string(), t("String")),
        ]
        .into(),
    })
}

/// Pedidos junto a su región, proyectado a `(pais, region, total)`. Con junta,
/// que es lo que hace que el circuito tenga **estado**: dos integradores.
fn vista() -> Nodo {
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

fn sesion_json(plan: &Nodo, extra: Vec<(&'static str, Json)>) -> String {
    let mut campos = vec![
        ("plan", plan.json()),
        ("clave", Json::Arr(vec![Json::s("pais")])),
        ("bundle", Json::s(BUNDLE)),
        ("capacidad", Json::Int(8)),
    ];
    campos.extend(extra);
    Json::obj(campos).jcs()
}

fn abrir(texto: &str) -> Result<Sesion, String> {
    Sesion::abrir(&parse(texto).expect("la sesión analiza"))
}

/// Manda una orden y devuelve la respuesta ya analizada.
fn orden(s: &mut Sesion, json: Json) -> Node {
    let respuesta = s.atender(&parse(&json.jcs()).expect("la orden analiza"));
    parse(&respuesta.jcs()).expect("la respuesta analiza")
}

fn campo(n: &Node, k: &str) -> String {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn fila(pares: &[(&str, Valor)]) -> Fila {
    pares
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

/// Qué sabe hacer cada origen, en el vocabulario de OOS: `lago` empuja
/// igualdad y pertenencia, y `referencias` es una tabla de dimensión que se
/// recorre entera sin drama.
fn capacidades(recorrido_del_lago: &str) -> Json {
    let lago = Json::obj([
        (
            "predicatePushdown",
            Json::Arr(vec![Json::s("eq"), Json::s("in")]),
        ),
        ("fullScan", Json::s(recorrido_del_lago.to_string())),
    ]);
    let referencias = Json::obj([("fullScan", Json::s("cheap"))]);
    Json::Obj(
        [
            ("lago".to_string(), lago),
            ("referencias".to_string(), referencias),
        ]
        .into(),
    )
}

fn cadena(s: &str) -> Valor {
    Valor::Cadena(s.to_string())
}

fn dec(s: &str) -> Valor {
    Valor::Decimal(s.to_string())
}

/// Una hoja con filas, tal como viaja en una orden `delta`.
fn hoja(datasource: &str, objeto: &str, z: &Zset) -> Json {
    Json::obj([
        ("datasource", Json::s(datasource.to_string())),
        ("objeto", Json::s(objeto)),
        ("filas", z.json()),
    ])
}

// ── 1 · el Refresh Analyzer, en la puerta ───────────────────────────────────

/// **Una vista que no se mantiene no abre sesión**, y el rechazo trae todos los
/// motivos. Es el análisis de M6 puesto donde sirve: antes de la primera fila,
/// no a la tercera hora de refrescos.
#[test]
fn una_vista_que_no_se_mantiene_no_abre_sesion() {
    let plan = Nodo::Limita {
        entrada: Box::new(vista()),
        n: 10,
    };
    let e = abrir(&sesion_json(&plan, vec![]))
        .err()
        .expect("no se puede mantener");
    assert!(
        e.contains("no se mantiene incrementalmente") && e.contains("top-N"),
        "{e}"
    );
}

/// Y una sesión sin `bundle` tampoco: sin él, un relleno de otra compilación no
/// se distingue de uno bueno. Es `ReglaDistinta` de E1, un piso más abajo.
#[test]
fn sin_bundle_no_hay_sesion() {
    let plan = vista();
    let sin = Json::obj([
        ("plan", plan.json()),
        ("clave", Json::Arr(vec![Json::s("pais")])),
    ])
    .jcs();
    let e = abrir(&sin).err().expect("sin bundle no");
    assert!(e.contains("bundle"), "{e}");
}

// ── 2 · un fallo es un plan, y con capacidades es una petición ───────────────

#[test]
fn un_fallo_devuelve_una_upquery_y_lo_que_se_le_pide_a_cada_hoja() {
    let caps = capacidades("cheap");
    let mut s = abrir(&sesion_json(&vista(), vec![("capacidades", caps)])).expect("abre");

    let r = orden(
        &mut s,
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ]),
    );
    assert_eq!(campo(&r, "presente"), "false");

    // La upquery es el plan de la vista filtrado a la clave, y viaja como plan.
    let upquery = r.get("upquery").expect("hay upquery").1;
    let leido = Nodo::de(upquery).expect("la upquery es un plan que se lee");
    assert!(matches!(leido, Nodo::Filtra { .. }), "{leido:?}");

    // Y con capacidades, además, lo que se le pide a cada hoja: el Pushdown
    // Planner bajó el predicado hasta las dos hojas, que es lo que convierte el
    // fallo en una búsqueda por clave en vez de un escaneo.
    let peticiones = r.get("peticiones").expect("hay peticiones").1.items();
    assert_eq!(peticiones.len(), 2, "{peticiones:?}");
    let objetos: Vec<String> = peticiones.iter().map(|p| campo(p, "objeto")).collect();
    assert!(
        objetos.contains(&"ventas.pedidos".to_string()),
        "{objetos:?}"
    );
    assert!(objetos.contains(&"ref.paises".to_string()), "{objetos:?}");
    // Ninguna lleva URL: la credencial no es de esta pieza, y quien invoca
    // elige la identidad — la misma frontera que traza el driver.
    assert!(
        peticiones.iter().all(|p| p.get("url").is_none()),
        "la petición no lleva credencial: {peticiones:?}"
    );

    // Y leer la MISMA clave ausente otra vez no produce una segunda upquery en
    // vuelo: se coalescen, que es la regla de Noria.
    orden(
        &mut s,
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ]),
    );
    let e = orden(&mut s, Json::obj([("op", Json::s("estado"))]));
    assert_eq!(campo(&e, "fallos"), "2");
    assert_eq!(campo(&e, "calientes"), "0");
}

/// **Un fallo que el origen no sabe contestar se rechaza sin abrir nada.**
///
/// La *upquery* de esta vista lleva la clave a la hoja de pedidos, y la hoja de
/// referencias se queda sin filtro: hay que recorrerla entera. Si su fuente lo
/// prohíbe, la repoblación es imposible — y eso se sabe **aquí**, antes de que
/// nadie abra una conexión, que es toda la gracia de declarar en vez de
/// intentar.
#[test]
fn un_fallo_que_el_origen_no_puede_contestar_se_rechaza_antes_de_abrir_nada() {
    let caps = Json::Obj(
        [
            (
                "lago".to_string(),
                Json::obj([("fullScan", Json::s("forbidden"))]),
            ),
            (
                "referencias".to_string(),
                Json::obj([("fullScan", Json::s("forbidden"))]),
            ),
        ]
        .into(),
    );
    let mut s = abrir(&sesion_json(&vista(), vec![("capacidades", caps)])).expect("abre");
    let r = orden(
        &mut s,
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ]),
    );
    assert_eq!(campo(&r, "presente"), "false");
    assert!(r.get("peticiones").is_none(), "{r:?}");
    assert!(campo(&r, "rechazo").contains("recorrido completo"), "{r:?}");
}

// ── 3 · no se cree lo que le dan ────────────────────────────────────────────

#[test]
fn un_relleno_bajo_otro_bundle_se_rechaza_y_uno_que_nadie_pidio_tambien() {
    let mut s = abrir(&sesion_json(&vista(), vec![])).expect("abre");
    let filas = Zset::de([(
        fila(&[
            ("pais", cadena("ES")),
            ("region", cadena("sur")),
            ("total", dec("10")),
        ]),
        1,
    )]);

    // Sin haberla pedido.
    let r = orden(
        &mut s,
        Json::obj([
            ("op", Json::s("rellenar")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
            ("filas", filas.json()),
            ("marca", Json::Int(1)),
            ("bundle", Json::s(BUNDLE)),
        ]),
    );
    assert!(campo(&r, "error").contains("nadie pidió"), "{r:?}");

    // Pedida, pero computada bajo otra compilación.
    orden(
        &mut s,
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ]),
    );
    let r = orden(
        &mut s,
        Json::obj([
            ("op", Json::s("rellenar")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
            ("filas", filas.json()),
            ("marca", Json::Int(1)),
            ("bundle", Json::s("sha256:otro")),
        ]),
    );
    assert!(campo(&r, "error").contains("sha256:otro"), "{r:?}");

    // Y con el bundle bueno, entra.
    let r = orden(
        &mut s,
        Json::obj([
            ("op", Json::s("rellenar")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
            ("filas", filas.json()),
            ("marca", Json::Int(1)),
            ("bundle", Json::s(BUNDLE)),
        ]),
    );
    assert_eq!(campo(&r, "ok"), "true", "{r:?}");
    let e = orden(&mut s, Json::obj([("op", Json::s("estado"))]));
    assert_eq!(campo(&e, "calientes"), "1");
    assert_eq!(campo(&e, "filas"), "1");
}

// ── 4 · la afirmación que sostiene el resto ─────────────────────────────────

/// **Lo que sale de mantener es lo que saldría de recomputar.**
///
/// Cuatro pasos sobre una junta —altas en los dos lados, y una baja— y al final
/// la suma de los Δ de salida se compara con `Q` sobre las bases enteras. Es la
/// ecuación de DBSP, `Q^Δ = D∘Q∘I`, comprobada **a través del protocolo** y no
/// dentro del crate que la implementa.
#[test]
fn lo_que_sale_de_mantener_es_lo_que_saldria_de_recomputar() {
    let mut s = abrir(&sesion_json(&vista(), vec![])).expect("abre");

    let paso = |ped: Zset, pai: Zset, marca: i64| {
        Json::obj([
            ("op", Json::s("delta")),
            ("marca", Json::Int(marca)),
            (
                "hojas",
                Json::Arr(vec![
                    hoja("lago", "ventas.pedidos", &ped),
                    hoja("referencias", "ref.paises", &pai),
                ]),
            ),
        ])
    };

    let p1 = Zset::de([
        (
            fila(&[
                ("id", Valor::Entero(1)),
                ("pais", cadena("ES")),
                ("total", dec("10.50")),
            ]),
            1,
        ),
        (
            fila(&[
                ("id", Valor::Entero(2)),
                ("pais", cadena("PT")),
                ("total", dec("3")),
            ]),
            1,
        ),
    ]);
    let r1 = Zset::de([(
        fila(&[("codigo", cadena("ES")), ("region", cadena("sur"))]),
        1,
    )]);
    // Segundo paso: llega la región de PT —y con ella emparejan filas que ya
    // estaban—, y un pedido nuevo.
    let p2 = Zset::de([(
        fila(&[
            ("id", Valor::Entero(3)),
            ("pais", cadena("ES")),
            ("total", dec("7")),
        ]),
        1,
    )]);
    let r2 = Zset::de([(
        fila(&[("codigo", cadena("PT")), ("region", cadena("oeste"))]),
        1,
    )]);
    // Tercero: se retracta el pedido 1.
    let p3 = Zset::de([(
        fila(&[
            ("id", Valor::Entero(1)),
            ("pais", cadena("ES")),
            ("total", dec("10.50")),
        ]),
        -1,
    )]);

    let mut acumulado = Zset::nuevo();
    for (ped, pai, marca) in [
        (p1.clone(), r1.clone(), 1),
        (p2.clone(), r2.clone(), 2),
        (p3.clone(), Zset::nuevo(), 3),
    ] {
        let r = orden(&mut s, paso(ped, pai, marca));
        assert!(r.get("error").is_none(), "{r:?}");
        let salida = Zset::leer(r.get("salida").expect("hay salida").1).expect("es un Z-set");
        acumulado.sumar(&salida);
    }

    // `Q` sobre las bases enteras.
    let mut base_pedidos = p1;
    base_pedidos.sumar(&p2);
    base_pedidos.sumar(&p3);
    let mut base_paises = r1;
    base_paises.sumar(&r2);
    let bases: BTreeMap<Hoja, Zset> = [
        (
            ("lago".to_string(), "ventas.pedidos".to_string()),
            base_pedidos,
        ),
        (
            ("referencias".to_string(), "ref.paises".to_string()),
            base_paises,
        ),
    ]
    .into();
    let de_una_vez = recomputar(&vista(), &bases).expect("recomputa");

    assert_eq!(
        acumulado, de_una_vez,
        "mantener y recomputar tienen que dar lo mismo"
    );
}

// ── 5 · el estado parcial descarta lo que no sostiene ───────────────────────

/// Un delta para una clave **ausente se descarta**, y no es una pérdida: la
/// próxima lectura la repone desde la fuente, que es la verdad. Es la regla de
/// Noria, y aquí se ve funcionando: `PT` nunca se pidió, así que su Δ no se
/// guarda.
#[test]
fn un_delta_de_una_clave_que_no_se_sostiene_se_descarta() {
    let mut s = abrir(&sesion_json(&vista(), vec![])).expect("abre");

    // Se calienta `ES` y solo `ES`.
    orden(
        &mut s,
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ]),
    );
    orden(
        &mut s,
        Json::obj([
            ("op", Json::s("rellenar")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
            ("filas", Zset::nuevo().json()),
            ("marca", Json::Int(1)),
            ("bundle", Json::s(BUNDLE)),
        ]),
    );

    let ped = Zset::de([
        (
            fila(&[
                ("id", Valor::Entero(1)),
                ("pais", cadena("ES")),
                ("total", dec("10")),
            ]),
            1,
        ),
        (
            fila(&[
                ("id", Valor::Entero(2)),
                ("pais", cadena("PT")),
                ("total", dec("3")),
            ]),
            1,
        ),
    ]);
    let pai = Zset::de([
        (
            fila(&[("codigo", cadena("ES")), ("region", cadena("sur"))]),
            1,
        ),
        (
            fila(&[("codigo", cadena("PT")), ("region", cadena("oeste"))]),
            1,
        ),
    ]);
    let r = orden(
        &mut s,
        Json::obj([
            ("op", Json::s("delta")),
            ("marca", Json::Int(2)),
            (
                "hojas",
                Json::Arr(vec![
                    hoja("lago", "ventas.pedidos", &ped),
                    hoja("referencias", "ref.paises", &pai),
                ]),
            ),
        ]),
    );
    assert_eq!(campo(&r, "aplicadas"), "1", "`ES` está: se aplica · {r:?}");
    assert_eq!(
        campo(&r, "descartadasAusentes"),
        "1",
        "`PT` no se sostiene: se descarta · {r:?}"
    );
}

// ── 6 · el dictamen viaja y no se obedece solo ──────────────────────────────

/// **La tercera decisión del protocolo.** Con un umbral que dice recomputar, el
/// dictamen sale `recomputar` **y el paso se aplica igual**: saltarlo dejaría a
/// los integradores sin ver ese Δ y todos los pasos siguientes darían mal.
#[test]
fn el_dictamen_dice_recomputar_y_el_paso_se_aplica_igual() {
    let umbral = Json::obj([(
        "umbral",
        Json::obj([("numerador", Json::Int(1)), ("denominador", Json::Int(100))]),
    )]);
    let mut s = abrir(&sesion_json(&vista(), vec![("politica", umbral)])).expect("abre");

    orden(
        &mut s,
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ]),
    );
    orden(
        &mut s,
        Json::obj([
            ("op", Json::s("rellenar")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
            (
                "filas",
                Zset::de([(
                    fila(&[
                        ("pais", cadena("ES")),
                        ("region", cadena("sur")),
                        ("total", dec("1")),
                    ]),
                    1,
                )])
                .json(),
            ),
            ("marca", Json::Int(1)),
            ("bundle", Json::s(BUNDLE)),
        ]),
    );

    let ped = Zset::de([(
        fila(&[
            ("id", Valor::Entero(9)),
            ("pais", cadena("ES")),
            ("total", dec("5")),
        ]),
        1,
    )]);
    let pai = Zset::de([(
        fila(&[("codigo", cadena("ES")), ("region", cadena("sur"))]),
        1,
    )]);
    let r = orden(
        &mut s,
        Json::obj([
            ("op", Json::s("delta")),
            ("marca", Json::Int(2)),
            (
                "hojas",
                Json::Arr(vec![
                    hoja("lago", "ventas.pedidos", &ped),
                    hoja("referencias", "ref.paises", &pai),
                ]),
            ),
        ]),
    );
    assert_eq!(campo(&r, "decision"), "recomputar", "{r:?}");
    assert!(campo(&r, "porque").contains("1/100"), "{r:?}");
    // Y aun así se aplicó: el dictamen informa, no manda.
    assert_eq!(campo(&r, "aplicadas"), "1", "{r:?}");
    // Los dos integradores de la junta salen en el dictamen: el estado cuesta.
    assert_eq!(campo(&r, "integradores"), "2", "{r:?}");
}

// ── 7 · y todo eso, por stdin ───────────────────────────────────────────────

/// El protocolo de verdad: un proceso, líneas por stdin, una respuesta por
/// orden y el informe al cerrar. Sin esto lo anterior probaría una biblioteca,
/// no un programa delegado.
#[test]
fn la_sesion_entera_por_stdin_y_el_informe_al_cerrar() {
    let mut hijo = Command::new(env!("CARGO_BIN_EXE_ore-maintain"))
        .arg("mantener")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("no se pudo lanzar `ore-maintain`");

    let filas = Zset::de([(
        fila(&[
            ("pais", cadena("ES")),
            ("region", cadena("sur")),
            ("total", dec("10")),
        ]),
        1,
    )]);
    let lineas = [
        sesion_json(&vista(), vec![]),
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ])
        .jcs(),
        Json::obj([
            ("op", Json::s("rellenar")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
            ("filas", filas.json()),
            ("marca", Json::Int(4)),
            ("bundle", Json::s(BUNDLE)),
        ])
        .jcs(),
        Json::obj([
            ("op", Json::s("leer")),
            ("clave", Json::Arr(vec![cadena("ES").json()])),
        ])
        .jcs(),
        // Una línea que no analiza: se contesta y la sesión SIGUE.
        "{esto no es json".to_string(),
        Json::obj([("op", Json::s("estado"))]).jcs(),
    ];
    {
        let stdin = hijo.stdin.as_mut().expect("stdin");
        for l in &lineas {
            writeln!(stdin, "{l}").expect("escribir");
        }
    }
    let salida = hijo.wait_with_output().expect("esperar");
    assert!(
        salida.status.success(),
        "{}",
        String::from_utf8_lossy(&salida.stderr)
    );
    let texto = String::from_utf8_lossy(&salida.stdout);
    let respuestas: Vec<&str> = texto.lines().collect();
    assert_eq!(
        respuestas.len(),
        6,
        "una por orden, más el informe:\n{texto}"
    );

    // ① el fallo, ② el relleno, ③ el acierto con sus filas.
    assert!(respuestas[0].contains("\"presente\":false"), "{texto}");
    assert!(respuestas[1].contains("\"ok\":true"), "{texto}");
    assert!(
        respuestas[2].contains("\"presente\":true") && respuestas[2].contains("\"marca\":4"),
        "{texto}"
    );
    // ④ la línea rota se contesta y no cierra nada.
    assert!(respuestas[3].contains("no analiza"), "{texto}");
    // ⑤ el estado, ⑥ el informe.
    assert!(respuestas[4].contains("\"aciertos\":1"), "{texto}");
    assert!(
        respuestas[5].contains("\"op\":\"fin\"") && respuestas[5].contains("\"rellenos\":1"),
        "{texto}"
    );
}
