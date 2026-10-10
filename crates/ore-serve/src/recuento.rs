//! **Una línea por petición** (0047 M2.3): cuántas y de qué clase, nunca de quién ni sobre qué.
//!
//! ```text
//! acceso · GET /paquetes/{}/vistas/{} · 200 · 12 ms · persona
//! ```
//!
//! Hasta hoy `ore-serve` no dejaba ninguna, y el balanceador no registra sus backends. Aunque
//! los registrara, no vería lo que llega desde dentro: el puesto, los Jobs, el agente. Sin
//! contar no se sabe cuánto costaría preguntar a `ore-iam` en cada ruta, que es lo que M2 mide.
//!
//! ⛔ **El camino va sin nombres.** Un segmento se escribe tal cual sólo si es un LITERAL del
//!   enrutador; si no, `{}`. Así el registro no dice qué paquete, qué tabla ni qué persona, y
//!   tampoco sale el sujeto: sólo su clase.
//!
//! ⭐ **El vocabulario sale de `rutas.rs` mismo**, leído al compilar. Una ruta nueva trae sus
//!   literales sin que nadie se acuerde de copiarlos aquí. Es la lección de M1 § 7: `rutas::mapa`,
//!   escrito aparte, ya no anunciaba 34 rutas.
//!
//! ⭐ Y es el primer trozo de A4: por este sitio pasará `puede`.

use crate::rutas::Servidor;
use ore_entrada::http::{Peticion, Salida};
use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::time::Instant;

/// Los ficheros que enrutan: `rutas.rs` y el catálogo Iceberg, que monta su propio enrutador
/// bajo `/v1` (`catalogo.rs`). Un enrutador nuevo en otro fichero se añade aquí; la prueba
/// `los_literales_son_de_camino` pide uno de cada.
const FUENTES: [&str; 2] = [include_str!("rutas.rs"), include_str!("catalogo.rs")];

/// Los literales de los patrones de camino: cada `"palabra"` de un `[…]` que es un patrón
/// (`["paquetes", n, "vistas"]`, `["v1", resto @ ..]`, `matches!(seg, ["puestos", ..])`).
fn literales() -> &'static BTreeSet<&'static str> {
    static L: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    L.get_or_init(|| {
        FUENTES
            .iter()
            .flat_map(|f| brazos(f))
            .flatten()
            .filter_map(|s| s.literal)
            .collect()
    })
}

/// Un segmento de un patrón: el literal, o `None` si es una variable (`n`, `_`, `..`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Segmento {
    literal: Option<&'static str>,
    resto: bool,
}

/// Un elemento de un patrón de camino: `"literal"` en minúsculas, una variable, `_`, `..` o
/// `resto @ ..`. Si un `[…]` tiene algo más (una llamada, una mayúscula, un número suelto), no es
/// un patrón de camino y no aporta literales.
fn segmento(s: &'static str) -> Option<Segmento> {
    if let Some(l) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let camino = l.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && l.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c));
        return camino.then_some(Segmento {
            literal: Some(l),
            resto: false,
        });
    }
    let resto = s == ".." || s.ends_with("@ ..");
    let variable = s
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    (resto || variable).then_some(Segmento {
        literal: None,
        resto,
    })
}

/// Los patrones de camino de un fichero: cada `["…` en posición de patrón —tras `(`, `,` o `|`,
/// como en `("GET", [...])`, `(_, [...])` o `| [...]`—, con al menos un literal y sólo elementos
/// de patrón.
///
/// ⛔ Sin las pruebas del fichero (su `#[cfg(test)] mod`): sus caminos llevan nombres de
///   ejemplo (`ventas`, `espana`) que, en el vocabulario, saldrían tal cual en el registro el día
///   que alguien llame así a un paquete. Y sin las listas que no son patrones (`for x in [...]`,
///   `= [...]`): nombres de campos, argumentos de órdenes.
fn brazos(fuente: &'static str) -> Vec<Vec<Segmento>> {
    let fin = fuente.find("#[cfg(test)]\nmod ").unwrap_or(fuente.len());
    let fuente = &fuente[..fin];
    let mut out = Vec::new();
    let mut desde = 0;
    while let Some(i) = fuente[desde..].find("[\"") {
        let abre = desde + i + 1;
        desde = abre;
        let antes = fuente[..abre - 1].trim_end();
        if !antes.ends_with(['(', ',', '|']) {
            continue;
        }
        let Some(cierra) = fuente[abre..].find(']') else {
            break;
        };
        let dentro = &fuente[abre..abre + cierra];
        if dentro.contains('\n') && dentro.lines().count() > 3 {
            continue;
        }
        let segs: Option<Vec<Segmento>> = dentro
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(segmento)
            .collect();
        if let Some(segs) = segs.filter(|s| s.iter().any(|x| x.literal.is_some())) {
            out.push(segs);
        }
    }
    out
}

/// El camino sin nombres: cada segmento que no es un literal del enrutador, `{}`.
pub fn patron(ruta: &str) -> String {
    let l = literales();
    let segs: Vec<&str> = ruta
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| if l.contains(s) { s } else { "{}" })
        .collect();
    format!("/{}", segs.join("/"))
}

/// De qué clase es quien pide. Se verifica otra vez el testigo —microsegundos— para no tocar la
/// firma de cada ruta; un testigo que no vale es `sin`.
///
/// ⭐ `fondo` (ADR 0060 C1·2): una persona, pero en una consulta que nadie pidió con la mano —el
///   refresco de una vista de la consola, `x-ore-fondo: 1`—. La línea es la señal con que la
///   malla duerme tras 15 min sin actividad: una pestaña olvidada no debe tenerla despierta. Su
///   actividad de la organización es la misma que la de `persona` (alguien la hizo).
fn clase(s: &Servidor, p: &Peticion) -> &'static str {
    let Some(proveedor) = s.identidad.as_ref() else {
        return "sin";
    };
    match proveedor(&p.cabeceras) {
        Err(_) => "sin",
        Ok(i) if crate::puestos::es_agente(&i) => "agente",
        Ok(i) => match i.tipo.as_deref() {
            Some("aprovisionador") => "aprovisionador",
            _ if i.agente.is_some() => "delegado",
            _ if es_de_fondo(p) => "fondo",
            _ => "persona",
        },
    }
}

/// `x-ore-fondo: 1`: lo pide la consola sola, no la persona (C1·2).
fn es_de_fondo(p: &Peticion) -> bool {
    p.cabeceras
        .get("x-ore-fondo")
        .is_some_and(|v| v.trim() == "1")
}

/// Atiende y deja la línea. `/salud` no: es la sonda del balanceador, cada pocos segundos, y no
/// es nadie pidiendo nada. En un flujo, el tiempo es lo que tardó en abrirse.
///
/// ⭐ Y lo que escribe, a la actividad de la organización (0047 A6.3), por el
///   buzón: la respuesta no lo espera.
pub fn atendiendo(s: &Servidor, p: &Peticion) -> Salida {
    let t = Instant::now();
    let (salida, rastro) = crate::acceso::con_testigo(p, || s.atender_flujo(p));
    if p.ruta.trim_matches('/') != "salud" {
        let codigo = match &salida {
            Salida::Una(r) => r.codigo,
            Salida::Flujo(_) => 200,
            Salida::Bytes(b) => b.codigo,
        };
        let clase = clase(s, p);
        eprintln!(
            "acceso · {} {} · {} · {} ms · {}",
            p.metodo,
            patron(&p.ruta),
            codigo,
            t.elapsed().as_millis(),
            clase
        );
        // Con sujeto: alguien lo hizo.
        if matches!(clase, "persona" | "fondo" | "delegado" | "agente") {
            s.a_la_actividad(p, &salida, rastro.token, rastro.contado, rastro.preguntado);
        }
    }
    salida
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Cada brazo, con sus variables rellenas de un nombre que no es literal, vuelve a su patrón.
    /// Si un brazo nuevo se escribe de una forma que `brazos` no reconoce, sus literales no
    /// estarían en el vocabulario y su camino saldría `{}`: esto lo caza.
    #[test]
    fn cada_brazo_vuelve_a_su_patron() {
        let todos: Vec<_> = FUENTES.iter().flat_map(|f| brazos(f)).collect();
        assert!(
            todos.len() > 100,
            "sólo {} brazos: el lector de rutas.rs se ha perdido",
            todos.len()
        );
        for b in todos.iter().filter(|b| !b.iter().any(|s| s.resto)) {
            let concreto: Vec<&str> = b.iter().map(|s| s.literal.unwrap_or("x9-nombre")).collect();
            let esperado: Vec<&str> = b.iter().map(|s| s.literal.unwrap_or("{}")).collect();
            assert_eq!(
                patron(&concreto.join("/")),
                format!("/{}", esperado.join("/"))
            );
        }
    }

    /// La consola marca sus refrescos con `x-ore-fondo: 1` (ADR 0060 C1·2); sólo `1` vale.
    #[test]
    fn la_consulta_de_fondo_se_reconoce() {
        let con = |v: Option<&str>| {
            let mut cabeceras = std::collections::BTreeMap::new();
            if let Some(v) = v {
                cabeceras.insert("x-ore-fondo".to_string(), v.to_string());
            }
            Peticion {
                metodo: "GET".into(),
                ruta: "/celdas".into(),
                cabeceras,
                cuerpo: String::new(),
                consulta: Default::default(),
            }
        };
        assert!(es_de_fondo(&con(Some("1"))));
        assert!(es_de_fondo(&con(Some(" 1 "))));
        assert!(!es_de_fondo(&con(Some("0"))));
        assert!(!es_de_fondo(&con(Some("si"))));
        assert!(!es_de_fondo(&con(None)));
    }

    #[test]
    fn un_nombre_no_sale() {
        assert_eq!(
            patron("/paquetes/rrhh/vistas/nominas"),
            "/paquetes/{}/vistas/{}"
        );
        assert_eq!(
            patron("/fuentes/ventas-pg/catalogar"),
            "/fuentes/{}/catalogar"
        );
        assert_eq!(patron("/"), "/");
    }

    #[test]
    fn los_literales_son_de_camino() {
        let l = literales();
        for m in [
            "GET",
            "POST",
            "authorization",
            "x-ore-rama",
            "ventas",
            "--sql",
            ".",
        ] {
            assert!(!l.contains(m), "`{m}` no es un segmento de camino");
        }
        for m in [
            "paquetes",
            "vistas",
            "puestos",
            "fuentes",
            "catalogar",
            "v1",
        ] {
            assert!(l.contains(m), "falta `{m}`");
        }
    }
}
