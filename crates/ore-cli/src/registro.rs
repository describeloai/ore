//! **El registro de copias.** Una operación, tres consumidores.
//!
//! `handoff-tablas` dijo del puntero físico: *se registra una vez, con sus dos
//! caras*. Esto dice lo mismo de la copia, un piso más arriba:
//!
//! > **Una materialización es un plan que ya está calculado en algún sitio. Se
//! > registra una vez, y dice tres cosas: qué contesta, dónde vive, y hasta
//! > cuándo fue cierta.**
//!
//! # Por qué un módulo y no un trozo de `vista.rs`
//!
//! Lo miran `ore view`, el planificador y —cuando la topología deje de tener
//! ruta propia— el ejecutor. Si se construyera en tres sitios divergiría en el
//! que ninguna prueba ejerce, que es exactamente lo que le pasó a la topología:
//! es una vista materializada escrita a mano en el paradigma anterior, y por eso
//! nadie la reconoce como copia.
//!
//! # Lo que el registro **no** sabe
//!
//! Y no por falta de sitio, sino porque saberlo lo cerraría:
//!
//! - **el formato del destino.** Un CSR firmado, una tabla de un almacén, un
//!   objeto en R2. El formato es una propiedad del destino;
//! - **para qué sirve la copia.** «Índice de aristas» y «caché de carga útil»
//!   son etiquetas humanas: aquí las dos son un plan calculado en un sitio;
//! - **cómo se puebla.** Poblar es de quien tenga el cómputo.
//!
//! La medida de si está bien es una pregunta: *añadir una clase nueva de copia,
//! ¿cuesta un mecanismo o cuesta registrar un plan?*
//!
//! # Lo que este peldaño **no** hace
//!
//! No puebla, no refresca y no borra. Su valor entero es que después de él
//! **hay un sitio**.

use std::collections::BTreeMap;

use ore_core::link::Package;
use ore_core::types::Type;
use ore_core::vistas;
use ore_view::{Catalogo, Expr, FilterTree, Lectura, Materializacion, Nodo, esquema};

/// **Quién la refresca hoy.** No es una propiedad de la copia —por eso no está
/// en `Materializacion`— sino del árbol en este momento: *estar registrada y
/// estar mantenida son dos cosas*, y separarlas es lo que permite mover una sin
/// la otra.
///
/// El vocabulario es cerrado a propósito. Un mecanismo nuevo obliga a añadir una
/// variante, y añadirla pone roja la prueba que los enumera — que es el único
/// modo de que «hay tres mecanismos» deje de ser algo que hay que recordar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Camino {
    /// Nadie. La copia está declarada y ninguna ruta del árbol la puebla.
    Nadie,
    /// `ore index build` y `ore index refresh`, con marca de agua propia. El
    /// circuito Δ no la cubre, y hasta que la cubra esta ruta no se borra.
    IndiceDeTopologia,
}

impl Camino {
    /// Todos. La prueba que los enumera se apoya en esto **y** en el `match`
    /// exhaustivo de abajo, de modo que una variante nueva rompa la compilación
    /// antes de romper la afirmación.
    pub const TODOS: &'static [Camino] = &[Camino::Nadie, Camino::IndiceDeTopologia];

    pub fn nombre(self) -> &'static str {
        match self {
            Camino::Nadie => "nadie",
            Camino::IndiceDeTopologia => "índice de topología",
        }
    }

    pub fn porque(self) -> &'static str {
        match self {
            Camino::Nadie => "declarada y sin poblar: ninguna ruta del árbol la escribe todavía",
            Camino::IndiceDeTopologia => {
                "`ore index refresh`, con marca de agua propia y ajena al circuito Δ"
            }
        }
    }
}

/// Una copia registrada, con lo que el registro **no** guarda de ella: quién la
/// refresca. El plan, el destino y el testigo están en el árbol, por su nombre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copia {
    pub nombre: String,
    pub camino: Camino,
}

/// Una que se declaró y no entró, con el motivo. Se cuentan: una copia que se
/// cae en silencio es una consulta que se sirve del origen sin que nadie sepa
/// por qué.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fuera {
    pub nombre: String,
    pub porque: String,
}

#[derive(Debug, Default)]
pub struct Inventario {
    pub arbol: FilterTree,
    pub copias: Vec<Copia>,
    pub fuera: Vec<Fuera>,
}

impl Inventario {
    pub fn cuantas(&self) -> usize {
        self.copias.len()
    }

    /// Cuántas por cada camino, **incluidos los caminos con cero**. Es la forma
    /// que la prueba enumera: sin los ceros, un mecanismo que deja de usarse
    /// desaparecería del inventario sin que nadie lo borre.
    pub fn por_camino(&self) -> BTreeMap<Camino, usize> {
        let mut m: BTreeMap<Camino, usize> = Camino::TODOS.iter().map(|c| (*c, 0usize)).collect();
        for c in &self.copias {
            *m.entry(c.camino).or_default() += 1;
        }
        m
    }
}

/// **Construye el registro del paquete entero.**
///
/// Dos procedencias, un solo índice — que es el punto: para el `FilterTree` las
/// dos son un plan calculado en un sitio.
pub fn construir(
    pkg: &Package,
    catalogo: &Catalogo,
    tipos: &BTreeMap<(String, String, String), Type>,
) -> Inventario {
    let mut inv = Inventario::default();
    for (nombre, plan, tabla) in declaradas(pkg, catalogo) {
        meter(&mut inv, nombre, plan, tabla, Camino::Nadie);
    }
    for (nombre, plan, tabla) in topologia(pkg, tipos) {
        meter(&mut inv, nombre, plan, tabla, Camino::IndiceDeTopologia);
    }
    inv
}

/// El registro propiamente dicho, con la parte que **no se puede comprobar
/// desde aquí** dicha donde se ve.
///
/// La declaración de OOS dice **dónde** vive la copia y no **qué columnas**
/// tiene: `materialized` lleva `datasource` y `table`, y nada más. Así que los
/// campos del destino se construyen desde el plan, que es la única opción
/// honesta —inventarlos sería peor— y tiene una consecuencia que conviene
/// escribir en vez de descubrir:
///
/// > **`Registro::TablaNoCorresponde` no puede dispararse nunca por este
/// > camino.** Existe para quien registre una copia cuyo destino ya conoce por
/// > otra vía —el ejecutor, leyendo un almacén de verdad— y ahí sí es la
/// > comprobación que separa un registro bueno de uno que *parece* bueno.
fn meter(inv: &mut Inventario, nombre: String, plan: Nodo, tabla: Lectura, camino: Camino) {
    match inv
        .arbol
        .registrar(Materializacion::nueva(nombre.clone(), plan, tabla))
    {
        Ok(()) => inv.copias.push(Copia { nombre, camino }),
        Err(r) => inv.fuera.push(Fuera {
            nombre,
            porque: r.como_texto(),
        }),
    }
}

/// Las que el paquete declara: una `View` con `materialized`.
fn declaradas(pkg: &Package, catalogo: &Catalogo) -> Vec<(String, Nodo, Lectura)> {
    let mut out = Vec::new();
    for v in pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::View)
    {
        let (Some(qn), Some(m)) = (v.qname(), v.section("materialized")) else {
            continue;
        };
        // Una que no expande o no tipa ya la denuncia `ore view` por su propia
        // línea, con el desajuste entero. Aquí solo no entra.
        let Ok(plan) = catalogo.expandir(&qn) else {
            continue;
        };
        let Ok(campos) = esquema(&plan) else { continue };
        out.push((
            qn,
            plan,
            Lectura {
                datasource: cadena(m, "datasource"),
                objeto: cadena(m, "table"),
                campos,
            },
        ));
    }
    out
}

/// **La topología, que es una copia desde siempre y no lo dice.**
///
/// `ore-exec/plan.rs::lecturas_de_aristas` la construye a mano: por cada
/// relación con `via`, una proyección de dos columnas —la clave de la entidad y
/// la del enlace— sobre la fuente física de la entidad, refrescada por marca de
/// agua. Eso es una vista materializada, escrita en el paradigma anterior.
///
/// Aquí se reconstruye desde el sustrato: la fuente física de una entidad es la
/// raíz de la vista que la respalda —`backedBy`—, sin pasar por ningún binding.
/// Y el destino nombra su formato, `oretopo`, porque el formato es propiedad del
/// destino y no del registro — [ADR 0006](../../../docs/decisions/0006-el-artefacto-de-topologia.md).
///
/// Las que `lecturas_de_aristas` descarta se descartan igual y por lo mismo: una
/// clave o una `via` compuesta es una tupla, y aplanarla aquí inventaría una
/// codificación que nadie declaró.
fn topologia(
    pkg: &Package,
    tipos: &BTreeMap<(String, String, String), Type>,
) -> Vec<(String, Nodo, Lectura)> {
    let mut out = Vec::new();
    for e in pkg.entities() {
        let (Some(qn), Some(rels)) = (e.qname(), e.section("relations")) else {
            continue;
        };
        let clave = lista(e.section("primaryKey"));
        if clave.len() != 1 {
            continue;
        }
        let Some(v) = vistas::respaldo(pkg, e) else {
            continue;
        };
        let Ok(raiz) = vistas::raiz(pkg, v) else {
            continue;
        };
        for (rk, rv) in rels.entries() {
            let Some(rel) = rk.as_str() else { continue };
            let via = lista(rv.get("via").map(|(_, x)| x));
            if via.len() != 1 {
                continue;
            }
            let (Some(cd), Some(ch)) = (raiz.columnas.get(&clave[0]), raiz.columnas.get(&via[0]))
            else {
                continue;
            };
            let tipo = |c: &str| {
                tipos
                    .get(&(raiz.datasource.clone(), raiz.objeto.clone(), c.to_string()))
                    .cloned()
                    .unwrap_or_else(|| Type::Scalar("String".into()))
            };
            // El `Lee` es el objeto, como en cualquier vista: dos copias sobre
            // la misma raíz comparten hoja, y es lo que hace que el índice
            // invertido del Filter Tree sirva para las dos.
            let mut columnas: BTreeMap<String, Type> = BTreeMap::new();
            for c in [cd, ch] {
                columnas.insert(c.clone(), tipo(c));
            }
            // `desde` y `hasta`: los mismos nombres que el driver ya devuelve,
            // así que lo que sale de aquí ya es una arista y no hay protocolo
            // nuevo. Es la prueba de que la fase ③ era el protocolo correcto.
            let plan = Nodo::Proyecta {
                entrada: Box::new(Nodo::Lee(Lectura {
                    datasource: raiz.datasource.clone(),
                    objeto: raiz.objeto.clone(),
                    campos: columnas,
                })),
                campos: BTreeMap::from([
                    ("desde".to_string(), Expr::campo(cd)),
                    ("hasta".to_string(), Expr::campo(ch)),
                ]),
            };
            let Ok(campos) = esquema(&plan) else { continue };
            let nombre = format!("{qn}.{rel}");
            out.push((
                nombre.clone(),
                plan,
                Lectura {
                    datasource: "oretopo".to_string(),
                    objeto: nombre,
                    campos,
                },
            ));
        }
    }
    out
}

fn cadena(n: &ore_core::parse::Node, k: &str) -> String {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("?")
        .to_string()
}

fn lista(n: Option<&ore_core::parse::Node>) -> Vec<String> {
    n.map(|x| {
        x.items()
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect()
    })
    .unwrap_or_default()
}

/// Lo que `ore view` imprime del paquete: las copias con sus **tres caras**, y
/// quién las refresca — que es la cuarta cosa, y no es de la copia.
pub fn imprimir(inv: &Inventario) {
    // El reparto por camino sale **con los ceros**, y por eso va en la primera
    // línea: un mecanismo con cero copias sigue siendo un mecanismo que hay que
    // mantener, y esconderlo es cómo se llega a tener tres sin saberlo.
    println!(
        "registro · {} {} · {}",
        inv.cuantas(),
        if inv.cuantas() == 1 {
            "copia"
        } else {
            "copias"
        },
        inv.por_camino()
            .iter()
            .map(|(c, n)| format!("{} {n}", c.nombre()))
            .collect::<Vec<_>>()
            .join(" · ")
    );
    if inv.copias.is_empty() && inv.fuera.is_empty() {
        println!("  ninguna · el paquete no declara ninguna materialización");
    }
    for c in &inv.copias {
        let Some(m) = inv.arbol.de(&c.nombre) else {
            continue;
        };
        println!("  {}", c.nombre);
        println!("    plan      {}", m.plan.digest());
        println!("    destino   {}·{}", m.tabla.datasource, m.tabla.objeto);
        println!("    testigo   {}", m.testigo.como_texto());
        println!(
            "    refresco  {} — {}",
            c.camino.nombre(),
            c.camino.porque()
        );
    }
    for f in &inv.fuera {
        println!("  {} · NO ENTRA", f.nombre);
        println!("    {}", f.porque);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **El inventario de mecanismos de refresco.**
    ///
    /// El `handoff` pidió una prueba que enumere *«cuántas y por qué camino se
    /// refresca cada una, de modo que añadir un mecanismo nuevo la ponga
    /// roja»*. Son dos afirmaciones y hacen falta las dos:
    ///
    /// - el `match` es **exhaustivo**, así que un `Camino` nuevo no compila
    ///   hasta que alguien lo nombre y diga por qué existe;
    /// - la cuenta está escrita, así que nombrarlo tampoco basta: hay que venir
    ///   aquí y decidir que son tres.
    ///
    /// Y esto es lo que hace que *«hay tres mecanismos de refresco»* deje de ser
    /// algo que hay que recordar leyendo `ore-exec`.
    #[test]
    fn los_caminos_de_refresco_estan_enumerados_y_cada_uno_dice_por_que() {
        let dichos: Vec<(&str, &str)> = Camino::TODOS
            .iter()
            .map(|c| match c {
                Camino::Nadie => (c.nombre(), c.porque()),
                Camino::IndiceDeTopologia => (c.nombre(), c.porque()),
            })
            .collect();

        assert_eq!(
            dichos.len(),
            2,
            "hay {} caminos de refresco y esta prueba conocía 2. Si has añadido \
             uno, la copia que lo usa tiene que decir por qué no le vale ninguno \
             de los que ya estaban: {dichos:?}",
            dichos.len()
        );
        assert_eq!(dichos[0].0, "nadie");
        assert_eq!(dichos[1].0, "índice de topología");
        for (nombre, porque) in &dichos {
            assert!(
                !porque.trim().is_empty(),
                "`{nombre}` no dice por qué existe"
            );
        }
    }

    /// Los ceros cuentan. Un mecanismo que deja de usarse tiene que seguir
    /// apareciendo con cero copias hasta que alguien lo borre a mano: si
    /// desapareciera del inventario, borrarlo dejaría de ser una decisión.
    #[test]
    fn el_reparto_por_camino_incluye_los_caminos_vacios() {
        let vacio = Inventario::default();
        assert_eq!(vacio.cuantas(), 0);
        assert_eq!(vacio.por_camino().len(), Camino::TODOS.len());
        assert!(vacio.por_camino().values().all(|n| *n == 0));
    }
}
