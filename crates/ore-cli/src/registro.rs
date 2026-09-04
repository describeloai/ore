//! **El registro de copias.** Una operación, tres consumidores.
//!
//! De la tabla se dijo: *el puntero físico se registra una vez, con sus dos
//! caras* —`spec/v1alpha8/01-table`—. Esto dice lo mismo de la copia, un piso
//! más arriba:
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

use ore_core::link::{Loaded, Package};
use ore_core::types::Type;
use ore_core::vistas;
use ore_view::{
    Catalogo, Clasificacion, Etiquetas, Expr, FilterTree, Hoja, Lectura, Marca, Materializacion,
    NoContesta, Nodo, Raiz, Restriccion, Rewrite, Testigo, comprobar, cotejar, esquema, linaje,
    sello,
};

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
    /// Cuanto guarda el origen su changelog, si lo dice. `None` es **no se
    /// sabe**, que no es lo mismo que «para siempre».
    pub retencion: Option<String>,
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
    for (nombre, plan, tabla, testigo, retencion) in declaradas(pkg, catalogo) {
        meter(
            &mut inv,
            nombre,
            plan,
            tabla,
            testigo,
            retencion,
            Camino::Nadie,
        );
    }
    // **La topología va sin marca, y no es un olvido.** La suya es una fecha que
    // el operador pasa a `index refresh --marca`, y esa no está en el
    // vocabulario de `changes.witness`. Es otro síntoma de lo mismo: mientras
    // tenga ruta de refresco propia, tiene también testigo propio. Se cierra
    // cuando la topología deje de derivarse y pase a ser una vista declarada.
    for (nombre, plan, tabla) in topologia(pkg, tipos) {
        meter(
            &mut inv,
            nombre,
            plan,
            tabla,
            Testigo::vacio(),
            // Y sin retencion, por lo mismo: el `.oretopo` no declara cuanto
            // guarda, porque no guarda un changelog — se reconstruye entero.
            None,
            Camino::IndiceDeTopologia,
        );
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
#[allow(clippy::too_many_arguments)]
fn meter(
    inv: &mut Inventario,
    nombre: String,
    plan: Nodo,
    tabla: Lectura,
    testigo: Testigo,
    retencion: Option<String>,
    camino: Camino,
) {
    match inv
        .arbol
        .registrar(Materializacion::nueva(nombre.clone(), plan, tabla).con_testigo(testigo))
    {
        Ok(()) => inv.copias.push(Copia {
            nombre,
            camino,
            retencion,
        }),
        Err(r) => inv.fuera.push(Fuera {
            nombre,
            porque: r.como_texto(),
        }),
    }
}

/// **La marca de una copia: con qué se fecharía.**
///
/// Sale de `changes.witness` de la tabla de la que la vista lee, y no de la
/// vista: *qué prueba qué versión de los datos se leyó* es una propiedad del
/// objeto, como `reads` y como `mode`. La vista no puede fechar mejor que su
/// origen.
///
/// `Ninguna` es una respuesta legal y tiene precio, y la especificación lo dice
/// donde se declara: *sin testigo no hay marca, y sin marca lo materializado no
/// puede decir hasta cuándo era cierto*. Eso es lo que
/// [`frescura_comprobable`] convierte en una línea de `ore view`.
pub fn marca_de(pkg: &Package, v: &Loaded) -> Marca {
    let Ok(r) = vistas::raiz(pkg, v) else {
        return Marca::Ninguna;
    };
    let Some(c) = r
        .tabla
        .as_deref()
        .and_then(|qn| pkg.table(qn))
        .and_then(|t| t.section("changes"))
    else {
        return Marca::Ninguna;
    };
    match c.get("witness").and_then(|(_, x)| x.as_str()) {
        Some("snapshot") => Marca::Instantanea,
        Some("log") => Marca::Registro,
        // `witness: field` sin `field` no compila —lo comprueba el núcleo— así
        // que llegar aquí sin él solo pasa con un documento que no pasó por
        // `ore validate`. Se degrada a `Ninguna` en vez de inventar la columna.
        Some("field") => c
            .get("field")
            .and_then(|(_, f)| f.as_str())
            .map(|f| Marca::Campo(f.to_string()))
            .unwrap_or(Marca::Ninguna),
        _ => Marca::Ninguna,
    }
}

/// **Cuanto guarda el origen su changelog**, si lo dice.
///
/// `changes.retention` lleva desde v1alpha8 declarando para que sirve —*«quien
/// planifique un refresco lo usa para saber si puede llegar tarde»*— y no tenia
/// ningun consumidor. Este es el que le faltaba.
///
/// `None` es **no se sabe**, y no se convierte en «para siempre»: la
/// especificacion lo dice donde declara el campo — *no se inventa, ausente
/// significa que no se sabe*.
pub fn retencion_de(pkg: &Package, v: &Loaded) -> Option<String> {
    let r = vistas::raiz(pkg, v).ok()?;
    pkg.table(r.tabla.as_deref()?)?
        .section("changes")?
        .get("retention")
        .and_then(|(_, x)| x.as_str())
        .map(String::from)
}

/// **Si la frescura que una copia declara se puede llegar a comprobar.**
///
/// `Ok` con la marca que la fecharía; `Err` cuando no hay ninguna. No es un
/// error de compilación —declarar `freshness` sobre una tabla sin testigo es
/// legal— es una **degradación**, y la diferencia importa: servir lo viejo como
/// fresco es el fallo que este proyecto no puede permitirse, y para un agente
/// saber que el contexto está degradado es la diferencia entre abstenerse y
/// alucinar.
pub fn frescura_comprobable(pkg: &Package, v: &Loaded) -> Result<Marca, ()> {
    match marca_de(pkg, v) {
        Marca::Ninguna => Err(()),
        m => Ok(m),
    }
}

/// Las que el paquete declara: una `View` con `materialized`.
fn declaradas(
    pkg: &Package,
    catalogo: &Catalogo,
) -> Vec<(String, Nodo, Lectura, Testigo, Option<String>)> {
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
            // La marca sí; el valor no, y no por falta de sitio: **nada puebla
            // una copia todavía**. Que la marca entre ya es lo que permite
            // decir, hoy y sin poblar nada, que una frescura no se va a poder
            // comprobar nunca.
            Testigo {
                marca: marca_de(pkg, v),
                valor: None,
            },
            retencion_de(pkg, v),
        ));
    }
    out
}

/// **La topología, que es una copia desde siempre y no lo dice.**
///
/// **Quién decide qué aristas hay: [`ore_core::aristas`], y solo él.**
///
/// Aquí no queda derivación, solo traducción a un plan. Era la mitad de I4: la
/// otra mitad la ponía `ore-exec`, que traducía **las mismas** aristas a su
/// `Lectura`, y una prueba comprobaba que los dos listaban lo mismo.
///
/// **Ese gemelo se retiró con él.** Lo que I4 arregló sigue arreglado —la
/// derivación es una y vive en [`ore_core::aristas`]— pero ya no hay dos
/// consumidores que la mantengan honesta, y eso conviene saberlo: una
/// derivación con un solo lector se puede torcer sin que nada la contradiga.
///
/// El destino nombra su formato —`oretopo`— porque el formato es propiedad del
/// destino y no del registro:
/// [ADR 0006](../../../docs/decisions/0006-el-artefacto-de-topologia.md).
fn topologia(
    pkg: &Package,
    tipos: &BTreeMap<(String, String, String), Type>,
) -> Vec<(String, Nodo, Lectura)> {
    let mut out = Vec::new();
    for a in ore_core::aristas::aristas(pkg) {
        let tipo = |c: &str| {
            tipos
                .get(&(a.datasource.clone(), a.objeto.clone(), c.to_string()))
                .cloned()
                .unwrap_or_else(|| Type::Scalar("String".into()))
        };
        // El `Lee` es el objeto, como en cualquier vista: dos copias sobre la
        // misma raíz comparten hoja, y es lo que hace que el índice invertido
        // del Filter Tree sirva para las dos.
        let columnas: BTreeMap<String, Type> = [&a.desde, &a.hasta]
            .into_iter()
            .map(|c| (c.clone(), tipo(c)))
            .collect();
        // `desde` y `hasta`: los mismos nombres que el driver ya devuelve, así
        // que lo que sale de aquí ya es una arista y no hay protocolo nuevo.
        let plan = Nodo::Proyecta {
            entrada: Box::new(Nodo::Lee(Lectura {
                datasource: a.datasource.clone(),
                objeto: a.objeto.clone(),
                campos: columnas,
            })),
            campos: BTreeMap::from([
                ("desde".to_string(), Expr::campo(&a.desde)),
                ("hasta".to_string(), Expr::campo(&a.hasta)),
            ]),
        };
        let Ok(campos) = esquema(&plan) else { continue };
        out.push((
            a.nombre.clone(),
            plan,
            Lectura {
                datasource: "oretopo".to_string(),
                objeto: a.nombre,
                campos,
            },
        ));
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
pub fn imprimir(inv: &Inventario, restricciones: &[Restriccion]) {
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
        // **El horizonte.** `changes.retention` llevaba desde v1alpha8
        // declarando para qué sirve y sin ningún consumidor; este es el suyo.
        //
        // Lo que se puede decir hoy es **cuánto guarda el origen**, y con eso ya
        // se sabe si un refresco puede llegar tarde. Lo que NO se puede decir es
        // si esta copia concreta ya caducó: eso exige el **valor** del testigo
        // —que va vacío hasta R2— y un reloj, y `ore` no lee el reloj por
        // invariante. Cuando los dos estén, la resta es
        // `ore_core::frescura::alcance`, que ya está escrita y probada.
        //
        // Y la ausencia **no se rellena**: sin `retention` el origen no promete
        // guardar para siempre, promete no decirlo.
        println!(
            "    horizonte {}",
            match &c.retencion {
                Some(r) => format!("el origen guarda {r} de cambios"),
                None => "sin declarar · el origen no dice cuánto guarda, así que no se afirma nada"
                    .to_string(),
            }
        );
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
    // Con qué cuenta el cotejo para probar una junta sin pérdida. Va aquí y no
    // en cada vista porque son del paquete, y se dicen aunque sean cero: con
    // cero, **ninguna** materialización con una hoja de más podrá contestar
    // nunca, y eso explica un «no la contesta» que si no parece un fallo.
    let (u, r) = restricciones.iter().fold((0, 0), |(u, r), x| match x {
        Restriccion::Unica { .. } => (u + 1, r),
        Restriccion::Referencial { .. } => (u, r + 1),
    });
    println!(
        "  restricciones  {u} {} · {r} {}",
        if u == 1 { "única" } else { "únicas" },
        if r == 1 {
            "referencial"
        } else {
            "referenciales"
        }
    );
}

// ── El cotejo ────────────────────────────────────────────────────────────────

/// **Las restricciones declaradas del paquete**, en los términos del View
/// Matcher — que no sabe de dónde salen, solo qué garantizan.
///
/// Hacen falta para una sola cosa, y es la que Oracle llama *«juntas sin
/// pérdida»*: una materialización que lee una hoja **de más** solo contesta si
/// esa junta ni pierde ni duplica filas, y eso no se supone, se declara.
///
/// Tres procedencias, y las tres son declaraciones que ya existían:
///
/// | de dónde | qué garantiza |
/// |---|---|
/// | `changes.key` de una tabla `upsert` | única, y la especificación **la exige**: sin ella el mantenedor no sabría qué retracta un *tombstone* |
/// | `primaryKey` y `uniqueKeys` de una entidad | única, sobre la raíz de la vista que la respalda |
/// | una relación con `via` **y `required: true`** | referencial, de la columna del enlace a la clave del destino |
///
/// # Por qué `required: true` y no toda relación
///
/// Una referencial afirma que **toda** fila de un lado casa con una del otro, y
/// es exactamente lo que prueba que la junta no pierde filas. Una relación
/// opcional no lo afirma: `manager` con `required: false` son los empleados sin
/// jefe, y una junta interna por `managerId` los tira. Declararla igual sería
/// darle al matcher permiso para perder filas en silencio, que es el fallo que
/// esta pieza existe para no cometer. Sin declaración no se supone ninguna.
pub fn restricciones(pkg: &Package) -> Vec<Restriccion> {
    let mut out: Vec<Restriccion> = Vec::new();

    // 1 · La tabla `upsert`, que trae su clave declarada por obligación.
    for t in pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Table)
    {
        if vistas::modo(t) != vistas::Modo::Upsert {
            continue;
        }
        let columnas = lista(
            t.section("changes")
                .and_then(|c| c.get("key"))
                .map(|(_, v)| v),
        );
        let (Some(ds), Some(ob)) = (
            t.section("datasource").and_then(|v| v.as_str()),
            t.section("object").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if !columnas.is_empty() {
            out.push(Restriccion::Unica {
                hoja: (ds.to_string(), ob.to_string()),
                columnas,
            });
        }
    }

    // 2 y 3 · Lo de la ontología, bajado a columnas por la raíz de su respaldo.
    for e in pkg.entities() {
        let Some((hoja, columnas)) = raiz_de_entidad(pkg, e) else {
            continue;
        };
        let fisicas = |props: &[String]| -> Option<Vec<String>> {
            props.iter().map(|p| columnas.get(p).cloned()).collect()
        };
        for claves in [e.section("primaryKey").map(|k| lista(Some(k)))]
            .into_iter()
            .flatten()
            .chain(
                e.section("uniqueKeys")
                    .into_iter()
                    .flat_map(|u| u.items().iter().map(|i| lista(Some(i))).collect::<Vec<_>>()),
            )
        {
            if let Some(cols) = fisicas(&claves)
                && !cols.is_empty()
            {
                out.push(Restriccion::Unica {
                    hoja: hoja.clone(),
                    columnas: cols,
                });
            }
        }

        let Some(rels) = e.section("relations") else {
            continue;
        };
        for (_, rv) in rels.entries() {
            if rv.get("required").and_then(|(_, v)| v.as_str()) != Some("true") {
                continue;
            }
            let via = lista(rv.get("via").map(|(_, v)| v));
            let Some(destino) = rv.get("target").and_then(|(_, v)| v.as_str()) else {
                continue;
            };
            let Some(te) = pkg.entity(destino) else {
                continue;
            };
            let Some((thoja, tcolumnas)) = raiz_de_entidad(pkg, te) else {
                continue;
            };
            // `toKey` cuando el enlace no va contra la primaria — que es legal en
            // SQL y es como enlazan los sistemas heredados: por NIF, por DUNS.
            let clave = match rv.get("toKey").map(|(_, v)| lista(Some(v))) {
                Some(k) if !k.is_empty() => k,
                _ => lista(te.section("primaryKey")),
            };
            let (Some(desde), Some(hacia)) = (
                fisicas(&via),
                clave
                    .iter()
                    .map(|p| tcolumnas.get(p).cloned())
                    .collect::<Option<Vec<_>>>(),
            ) else {
                continue;
            };
            if desde.is_empty() || desde.len() != hacia.len() {
                continue;
            }
            out.push(Restriccion::Referencial {
                desde: (hoja.clone(), desde),
                hacia: (thoja, hacia),
            });
        }
    }
    out.sort_by_key(|r| format!("{r:?}"));
    out.dedup();
    out
}

/// La hoja física de una entidad y su mapa propiedad → columna, por el sustrato:
/// la raíz de la vista que la respalda. Ni un binding de por medio.
fn raiz_de_entidad(
    pkg: &Package,
    e: &ore_core::link::Loaded,
) -> Option<(Hoja, BTreeMap<String, String>)> {
    let v = vistas::respaldo(pkg, e)?;
    let r = vistas::raiz(pkg, v).ok()?;
    Some(((r.datasource, r.objeto), r.columnas))
}

/// **Qué copias contestan este plan**, cada una con su compensación y su sello.
///
/// El sello es lo que hace que esto no sea una reescritura cualquiera: la
/// clasificación de una copia **se hereda, no se recalcula**. Una vista que
/// recortó por `nif` produce filas `critical` aunque `nif` no esté entre sus
/// columnas, y recalcular el linaje sobre la tabla copiada perdería esa
/// etiqueta. Por eso las raíces del plan reescrito son las columnas de la copia
/// y sus etiquetas son las selladas.
pub fn cotejos(
    inv: &Inventario,
    plan: &Nodo,
    clasificacion: &Clasificacion,
    restricciones: &[Restriccion],
) -> Vec<(String, Result<Rewrite, NoContesta>)> {
    inv.arbol
        .candidatas(plan)
        .into_iter()
        .map(|m| {
            let sellada = Clasificacion {
                reticulos: clasificacion.reticulos.clone(),
                de_raiz: sello_de(m, clasificacion),
            };
            (m.nombre.clone(), cotejar(plan, m, &sellada, restricciones))
        })
        .collect()
}

/// La clasificación efectiva de una copia, colgada de las columnas de su tabla.
///
/// Se pide **sin autorización** —`Etiquetas::new()`— a propósito: aquí no se
/// decide si la copia compila, que eso es del conducto y ya lo hace `ore view`
/// por su lado. Aquí solo se quiere saber qué lleva puesto cada columna.
fn sello_de(m: &Materializacion, c: &Clasificacion) -> BTreeMap<Raiz, Etiquetas> {
    let Ok(lin) = linaje(&m.plan) else {
        return BTreeMap::new();
    };
    sello(m, &comprobar(&lin, c, &Etiquetas::new()).efectivas)
}

/// **Qué columnas identifican una fila de esta copia.**
///
/// Es `changes.key` de la tabla de la que la vista lee, traducido a los nombres
/// de la vista — el almacén funde por lo que ve en las filas, y lo que ve son
/// **campos**, no columnas físicas.
///
/// Vacía cuando la tabla no declara clave, y eso tiene consecuencia: sin clave
/// no se puede fundir un incremento, así que la copia solo se rehace entera. Es
/// la otra cara de `OOS2023`, que rechaza justo la combinación en la que eso
/// además sería incorrecto.
pub fn clave_de(pkg: &Package, v: &Loaded) -> Vec<String> {
    let Ok(r) = vistas::raiz(pkg, v) else {
        return Vec::new();
    };
    let columnas: Vec<String> = r
        .tabla
        .as_deref()
        .and_then(|qn| pkg.table(qn))
        .and_then(|t| t.section("changes"))
        .and_then(|c| c.get("key"))
        .map(|(_, k)| {
            k.items()
                .iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    // Columna física → campo de la vista. Si alguna no se expone, la clave no
    // sirve: fundir por una columna que no está en las filas juntaría todo.
    let mut out = Vec::new();
    for col in columnas {
        match r.columnas.iter().find(|(_, c)| **c == col) {
            Some((campo, _)) => out.push(campo.clone()),
            None => return Vec::new(),
        }
    }
    out
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
    /// algo que hay que recordar leyendo el código de otro binario.
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
