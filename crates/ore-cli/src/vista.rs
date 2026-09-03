//! `ore view` — el motor de vistas, alimentado por un paquete OOS de verdad.
//!
//! Es **la absorción**: `ore-view` se construyó libre, sin saber qué es un
//! paquete, y este módulo es la única costura entre los dos. Lee las `View` del
//! paquete y las convierte en el IR del motor; lee el retículo, las etiquetas
//! efectivas y los datasources y los convierte en su clasificación; lee las
//! capacidades y las convierte en las suyas. Y entonces le pregunta lo que solo
//! el motor sabe contestar:
//!
//! - **qué plan** es cada vista, con su identidad;
//! - **qué columnas** produce y de qué tipo;
//! - **de qué columna raíz** sale cada una, y por qué arista — incluida la
//!   arista `INDIRECT`, la del `where`, que el núcleo no ve;
//! - **si se puede mantener** incrementalmente, y si no, todos los motivos;
//! - **qué empuja** al origen y qué queda de residuo, sin abrir una conexión;
//! - **si compila** la copia: la clasificación de una materialización se
//!   hereda por el linaje, no se recalcula sobre la tabla.
//!
//! # Lo que esto añade a `ore validate`, y por qué son dos
//!
//! El núcleo ya comprueba la vista materializada por sus **campos**: lo que se
//! copia lleva lo que llevan sus columnas. El motor comprueba además por lo que
//! **decide qué filas salen**: una vista que recorta por `nationalId` y expone
//! solo `id` no copia el DNI, y aun así revela quién lo tiene. Es el flujo
//! implícito de Denning, y `ore validate` no lo mira porque el núcleo no tiene
//! linaje por columna. El día que lo tenga, esta comprobación se moverá allí;
//! hasta entonces vive aquí y **se niega igual**.
//!
//! # Lo que no hace
//!
//! No ejecuta, no mide y no abre nada. Contesta desde el árbol de ficheros, que
//! es lo único que `ore` sabe leer.

use std::collections::{BTreeMap, BTreeSet};

use ore_core::document::Kind;
use ore_core::flow::{Axis, Lattice};
use ore_core::link::{Loaded, Package};
use ore_core::parse::Node;
use ore_core::types::{Type, parse_type};
use ore_core::vistas;
use ore_view::refresh_analyzer::analizar_con;
use ore_view::{
    Capacidades, Catalogo, Clase, Clasificacion, Comparador, Emite, Expr, Lectura, Nodo, Raiz,
    Valor, Vista, comprobar, esquema, linaje, repartir,
};

/// El conducto que una vista materializada instancia. El mismo que el eje
/// `payload` del binding, porque es la misma cosa con otro dueño.
pub(crate) const CONDUCTO: &str = "materialization.payload";

pub fn ver(path: &std::path::Path) -> std::process::ExitCode {
    let pkg = match crate::cargar_valido(path, true) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let vistas: Vec<&Loaded> = pkg.of_view();
    if vistas.is_empty() {
        println!("sin vistas · el paquete no declara ningún `kind: View`");
        return std::process::ExitCode::SUCCESS;
    }

    let lat = ore_core::flow::lattices(&pkg);
    let tipos = tipos_de_raiz(&pkg);
    let catalogo = Catalogo::con(
        vistas
            .iter()
            .filter_map(|v| Some(Vista::nueva(&v.qname()?, cuerpo(&pkg, v, &tipos)))),
    );
    let clasificacion = Clasificacion {
        reticulos: lat.clone(),
        de_raiz: etiquetas_de_raiz(&pkg, &lat),
    };
    let conductos = ore_core::flow::clearances(&pkg, &lat);
    let capacidades = capacidades_por_fuente(&pkg);
    // El registro, del paquete entero y no de cada vista: una copia la puede
    // servir la consulta de otra, así que la unidad es el paquete. Se construye
    // UNA vez, antes del bucle, porque dentro se cotejan planes contra él.
    let inventario = crate::registro::construir(&pkg, &catalogo, &tipos);
    let restricciones = crate::registro::restricciones(&pkg);
    let cambios = cambios_por_fuente(&pkg);

    let mut fugas = 0usize;
    let mut degradadas = 0usize;
    for v in &vistas {
        let Some(qn) = v.qname() else { continue };
        println!("{qn}");

        let plan = match catalogo.expandir(&qn) {
            Ok(p) => p,
            Err(e) => {
                println!("  plan      no se expande · {}", e.como_texto());
                fugas += 1;
                continue;
            }
        };
        let eslabones = vistas::cadena(&pkg, v).map(|c| c.len()).unwrap_or(1);
        println!(
            "  plan      {}  ({eslabones} {})",
            plan.digest(),
            if eslabones == 1 {
                "vista"
            } else {
                "vistas encadenadas"
            }
        );
        if let Ok(r) = vistas::raiz(&pkg, v) {
            println!("  raíz      {} · {}", r.datasource, r.objeto);
            // Las dos caras del objeto, y **qué regla las usa**. Una vista
            // v1alpha7 no las tiene: su puntero no es un documento, así que no
            // hay nada que enseñar — y esa ausencia también dice algo.
            if let Some(tqn) = r.tabla.as_deref()
                && let Some(tabla) = pkg.table(tqn)
            {
                println!("  caras     {}", caras(tabla));
                println!("            {}", raiz_de_lectura(&pkg, v, tabla));
            }
        }

        let esquema_de = match esquema(&plan) {
            Ok(e) => e,
            Err(d) => {
                println!("  esquema   no tipa · {}", d.como_texto());
                fugas += 1;
                continue;
            }
        };
        println!(
            "  esquema   {}",
            esquema_de
                .iter()
                .map(|(c, t)| format!("{c}: {t}"))
                .collect::<Vec<_>>()
                .join(" · ")
        );

        let lin = match linaje(&plan) {
            Ok(l) => l,
            Err(d) => {
                println!("  linaje    no se sigue · {}", d.como_texto());
                fugas += 1;
                continue;
            }
        };
        for (salida, aristas) in &lin {
            for a in aristas {
                println!(
                    "  linaje    {salida} ← {}·{}.{}  {}",
                    a.raiz.datasource,
                    a.raiz.objeto,
                    a.raiz.campo,
                    match a.clase {
                        Clase::Directo(d) => format!("DIRECT · {d:?}"),
                        Clase::Indirecto(i) => format!("INDIRECT · {i:?}"),
                    }
                );
            }
        }

        // El modo de refresco se sabe antes de escribir la vista, con todos
        // los motivos — y no al refrescarla y por la factura.
        //
        // Y se le pasa la cara `D`: sin ella el analizador decidía por la forma
        // del plan y diría `INCREMENTAL` de una vista sobre una tabla que
        // declara `changes: { mode: none }`. El compilador ya lo sabía; el motor
        // no se lo preguntaba.
        let refresco = analizar_con(&plan, &cambios).como_texto(&qn);
        for linea in refresco.lines() {
            let linea = linea.trim_start();
            if linea.starts_with(&qn) {
                println!(
                    "  refresco  {}",
                    linea
                        .trim_start_matches(qn.as_str())
                        .trim_start_matches([' ', '→'])
                );
            } else if !linea.is_empty() {
                println!("            {linea}");
            }
        }

        // El reparto: qué hace cada origen y qué queda, sin abrir nada.
        match repartir(&plan, &capacidades) {
            Ok(r) => {
                for p in &r.peticiones {
                    println!(
                        "  empuje    {}·{} recibe {} {}",
                        p.datasource,
                        p.objeto,
                        p.filtros.len(),
                        if p.filtros.len() == 1 {
                            "filtro"
                        } else {
                            "filtros"
                        }
                    );
                }
                let residuo = r.residuo.lecturas().len();
                println!(
                    "            residuo: {}",
                    if r.residuo.canonico() == plan.canonico() {
                        "el plan entero — el origen no aplica nada".to_string()
                    } else {
                        format!("{residuo} {}", if residuo == 1 { "hoja" } else { "hojas" })
                    }
                );
            }
            Err(e) => {
                println!("  empuje    rechazado · {}", e.como_texto());
            }
        }

        // El cotejo: de las copias registradas, cuáles contestan **este** plan.
        // Solo se dice algo cuando hay candidatas — el índice invertido ya las
        // filtró, y anunciar «ninguna» en cada vista de un paquete sin copias
        // sería ruido.
        //
        // Y el sello va con ellas: la clasificación de una copia se **hereda**.
        // Recalcular el linaje sobre su tabla perdería lo que la copia lleva
        // puesto por haber filtrado, que es justo lo que no se ve mirándola.
        for (nombre, r) in
            crate::registro::cotejos(&inventario, &plan, &clasificacion, &restricciones)
        {
            let suya = if nombre == qn { " — su copia" } else { "" };
            match r {
                Ok(rw) => {
                    println!(
                        "  cotejo    la contesta `{nombre}`{suya} · {}",
                        match rw.compensation.len() {
                            0 => "sin compensación".to_string(),
                            1 => "1 conyunto de compensación".to_string(),
                            n => format!("{n} conyuntos de compensación"),
                        }
                    );
                    let sellado: Vec<String> = rw
                        .label_seal
                        .iter()
                        .filter(|(_, ls)| !ls.is_empty())
                        .map(|(c, ls)| {
                            format!(
                                "{c} {{{}}}",
                                ls.iter()
                                    .map(|(e, n)| format!("{e}:{n}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        })
                        .collect();
                    if !sellado.is_empty() {
                        println!("            sello heredado: {}", sellado.join(" · "));
                    }
                }
                Err(e) => {
                    println!("  cotejo    `{nombre}` no la contesta");
                    for linea in e.como_texto().lines() {
                        println!("            {}", linea.trim());
                    }
                }
            }
        }

        // El flujo. Virtual: no hay copia, nada que autorizar. Materializada:
        // la copia lleva lo que llevan sus columnas raíz, por derivación Y por
        // influencia.
        match v.section("materialized") {
            None => println!("  flujo     virtual — cada lectura va al origen; nada que copiar"),
            Some(m) => {
                let destino = format!(
                    "{}·{}",
                    m.get("datasource")
                        .and_then(|(_, x)| x.as_str())
                        .unwrap_or("?"),
                    m.get("table").and_then(|(_, x)| x.as_str()).unwrap_or("?")
                );
                let autoriza: BTreeMap<String, String> = conductos
                    .get(CONDUCTO)
                    .map(|ls| {
                        ls.iter()
                            .map(|(k, (n, _))| (k.clone(), n.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let veredicto = comprobar(&lin, &clasificacion, &autoriza);
                let efectivas: Vec<String> = veredicto
                    .efectivas
                    .iter()
                    .filter(|(_, ls)| !ls.is_empty())
                    .map(|(c, ls)| {
                        format!(
                            "{c} {{{}}}",
                            ls.iter()
                                .map(|(e, n)| format!("{e}:{n}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
                    .collect();
                if veredicto.compila() {
                    println!(
                        "  flujo     materializada en {destino} · `{CONDUCTO}` compila{}",
                        if efectivas.is_empty() {
                            String::new()
                        } else {
                            format!(" · sellada: {}", efectivas.join(" · "))
                        }
                    );
                } else {
                    println!("  flujo     materializada en {destino} · `{CONDUCTO}` NO compila");
                    for f in &veredicto.fugas {
                        for linea in f.como_texto().lines() {
                            println!("            {linea}");
                        }
                    }
                    fugas += veredicto.fugas.len();
                }

                // La frescura que la copia promete, y si se va a poder
                // comprobar. **No es una fuga**: declararla sobre una tabla sin
                // testigo es legal. Es una DEGRADACIÓN, y la diferencia es toda
                // la gracia — servir lo viejo como fresco es el fallo que este
                // proyecto no puede permitirse, y para un agente saber que el
                // contexto está degradado es la diferencia entre abstenerse y
                // alucinar.
                if let Some(f) = v.section("freshness").and_then(|x| x.as_str()) {
                    match crate::registro::frescura_comprobable(&pkg, v) {
                        Ok(m) => println!("  frescura  {f} · comprobable con {}", m.como_texto()),
                        Err(()) => {
                            println!(
                                "  frescura  {f} · DEGRADADA — la tabla declara `witness: none`, \
                                 así que la copia no puede decir hasta cuándo fue cierta"
                            );
                            degradadas += 1;
                        }
                    }
                }
            }
        }
        println!();
    }

    crate::registro::imprimir(&inventario, &restricciones);
    // Una copia declarada que no entra en el registro no es un detalle: el
    // motor iria al origen sin que nadie sepa por que.
    fugas += inventario.fuera.len();

    // Lo degradado se dice al final y **no cambia el codigo de salida**: una
    // frescura que no se puede comprobar no invalida el paquete, avisa de que
    // hay una promesa que nadie va a poder verificar.
    if degradadas > 0 {
        println!();
        println!(
            "degradado · {degradadas} {} declara una frescura que no se puede comprobar",
            if degradadas == 1 { "copia" } else { "copias" }
        );
    }

    if fugas > 0 {
        eprintln!(
            "error: {fugas} {} · el motor de vistas se niega a compilar lo de arriba",
            if fugas == 1 { "fuga" } else { "fugas" }
        );
        return std::process::ExitCode::from(65); // EX_DATAERR
    }
    std::process::ExitCode::SUCCESS
}

/// Las dos caras de una tabla, en el vocabulario en que están escritas.
///
/// Se enseñan con los nombres de OOS —`reads`, `changes`, `witness`— y no
/// traducidos: quien lee esto tiene el documento delante, y un segundo
/// vocabulario para lo mismo obligaría a traducir de vuelta para arreglarlo.
fn caras(tabla: &Loaded) -> String {
    let lista = |n: &Node| -> String {
        n.items()
            .iter()
            .filter_map(|i| i.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut partes: Vec<String> = Vec::new();

    match tabla.section("reads") {
        // `reads: none` — un tópico se escribe, no se pregunta. Es la cara que
        // `OOS2020` mira.
        Some(r) if r.as_str() == Some("none") => partes.push("reads: none".to_string()),
        Some(r) => {
            partes.push(format!(
                "reads: {}",
                r.get("predicatePushdown")
                    .map(|(_, v)| lista(v))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "nada empujable".to_string())
            ));
            if let Some((_, f)) = r.get("fullScan") {
                partes.push(format!("fullScan: {}", f.as_str().unwrap_or("?")));
            }
            if let Some((_, rf)) = r.get("requiredFilters")
                && !rf.items().is_empty()
            {
                partes.push(format!("requiredFilters: {}", lista(rf)));
            }
        }
        None => partes.push("reads: sin declarar".to_string()),
    }

    if let Some(c) = tabla.section("changes") {
        let campo = |k: &str| c.get(k).and_then(|(_, v)| v.as_str()).unwrap_or("?");
        partes.push(format!("changes: {}", campo("mode")));
        partes.push(format!("witness: {}", campo("witness")));
        if let Some((_, k)) = c.get("key") {
            partes.push(format!("key: {}", lista(k)));
        }
        if let Some((_, f)) = c.get("field") {
            partes.push(format!("field: {}", f.as_str().unwrap_or("?")));
        }
        if let Some((_, r)) = c.get("retention") {
            partes.push(format!("retention: {}", r.as_str().unwrap_or("?")));
        }
    }
    partes.join(" · ")
}

/// De dónde salen de verdad las filas, y la regla que lo decidió.
///
/// La distinción raíz / raíz de lectura no se enseña por precisión: se enseña
/// porque **es la que decide si el paquete compila**. Un usuario que ve
/// `reads: none` y una vista virtual tiene ahí la explicación de `OOS2020` sin
/// tener que provocarlo.
fn raiz_de_lectura(pkg: &Package, v: &Loaded, tabla: &Loaded) -> String {
    let copia = vistas::raiz_de_lectura(pkg, v);
    let donde = copia.and_then(|c| c.section("materialized")).map(|m| {
        format!(
            "{}·{}",
            m.get("datasource")
                .and_then(|(_, x)| x.as_str())
                .unwrap_or("?"),
            m.get("table").and_then(|(_, x)| x.as_str()).unwrap_or("?")
        )
    });
    let modo = vistas::modo(tabla);
    match (donde, vistas::se_lee(tabla), modo) {
        (Some(d), false, _) => format!(
            "raíz de lectura: la copia en {d} — `OOS2020` la exige: lo que no se puede leer se \
             debe materializar"
        ),
        (Some(d), true, vistas::Modo::Anexa) => format!(
            "raíz de lectura: la copia en {d} — `OOS2021`: sin retractación solo respalda \
             `nature: event`"
        ),
        (Some(d), true, _) => format!("raíz de lectura: la copia en {d}"),
        (None, _, _) => "raíz de lectura: la tabla".to_string(),
    }
}

trait Vistas {
    fn of_view(&self) -> Vec<&Loaded>;
}

impl Vistas for Package {
    fn of_view(&self) -> Vec<&Loaded> {
        let mut v: Vec<&Loaded> = self.docs.iter().filter(|d| d.kind == Kind::View).collect();
        v.sort_by_key(|d| d.qname());
        v
    }
}

// ── El paquete → el IR ──────────────────────────────────────────────────────

/// Tipo de cada columna raíz: `(datasource, objeto, columna) → Type`.
///
/// La vista no tipa —es física— así que el tipo baja de **la entidad**: sus
/// propiedades se llaman como los campos de su vista, y la cadena los lleva
/// hasta la columna. Lo que ninguna entidad nombra es `String`, que es lo
/// único que se puede afirmar de una columna de la que solo se sabe el nombre.
pub(crate) fn tipos_de_raiz(pkg: &Package) -> BTreeMap<(String, String, String), Type> {
    let mut out = BTreeMap::new();
    for e in pkg.entities() {
        let Some(v) = vistas::respaldo(pkg, e) else {
            continue;
        };
        let Ok(raiz) = vistas::raiz(pkg, v) else {
            continue;
        };
        let Some(props) = e.section("properties") else {
            continue;
        };
        for (k, p) in props.entries() {
            let Some(nombre) = k.as_str() else { continue };
            let Some(col) = raiz.columnas.get(nombre) else {
                continue;
            };
            let Some(t) = p
                .get("type")
                .and_then(|(_, t)| t.as_str())
                .and_then(|t| parse_type(t).ok())
            else {
                continue;
            };
            out.insert(
                (raiz.datasource.clone(), raiz.objeto.clone(), col.clone()),
                t,
            );
        }
    }
    out
}

/// Con qué tipo se ve un campo desde una vista: lo que hace falta para tipar
/// el literal de un `where` como la columna que compara.
type Tipador<'a> = Box<dyn Fn(&str) -> Type + 'a>;

/// A qué objeto físico toca una vista: `(datasource, objeto)`, venga el puntero
/// de v1alpha7 —dentro de la vista— o de v1alpha8 —una `Table` aparte.
///
/// **Una operación, tres consumidores**: el cuerpo del IR, las etiquetas de
/// raíz y las capacidades preguntan lo mismo y tienen que verlo igual. Escrito
/// tres veces divergiría en el que ninguna prueba ejerce, que es exactamente
/// lo que le pasó al binding.
///
/// `None` si la vista sale de otra vista, o si la tabla que nombra no existe —
/// eso lo dice `OOS2018` y aquí no se repite.
fn objeto_fisico(pkg: &Package, v: &Loaded) -> Option<(String, String)> {
    match vistas::fuente(v)? {
        vistas::Fuente::Datasource { datasource, objeto } => Some((datasource, objeto)),
        vistas::Fuente::Tabla(qn) => {
            let t = pkg.table(&qn)?;
            Some((
                t.section("datasource")?.as_str()?.to_string(),
                t.section("object")?.as_str()?.to_string(),
            ))
        }
        vistas::Fuente::Vista(_) => None,
    }
}

/// El cuerpo de una vista en el IR: `Proyecta(Filtra(Lee | Referencia))`.
///
/// Es exactamente el vocabulario de v1alpha7 —seleccionar, renombrar,
/// recortar— y ni una operación más. Lo que la gramática no tiene, el plan no lo
/// tiene.
pub(crate) fn cuerpo(
    pkg: &Package,
    v: &Loaded,
    tipos: &BTreeMap<(String, String, String), Type>,
) -> Nodo {
    let campos = vistas::campos(v);
    let filtros = vistas::filtros(v);

    // La hoja, y con qué nombre se ve cada cosa desde esta vista.
    let (hoja, tipo_de): (Nodo, Tipador<'_>) = match vistas::fuente(v) {
        Some(vistas::Fuente::Vista(abajo)) => {
            // Los tipos de la de abajo son los de su esquema; se resuelven al
            // expandir. Para tipar el literal de un filtro se mira la raíz.
            let raiz = vistas::raiz(pkg, v).ok();
            let tipos = tipos.clone();
            let abajo_doc = pkg.view(&abajo);
            let f = move |campo_abajo: &str| -> Type {
                // Campo de la vista de abajo → su columna raíz → su tipo.
                let col = abajo_doc
                    .and_then(|d| vistas::raiz(pkg, d).ok())
                    .and_then(|r| r.columnas.get(campo_abajo).cloned());
                match (&raiz, col) {
                    (Some(r), Some(c)) => tipos
                        .get(&(r.datasource.clone(), r.objeto.clone(), c))
                        .cloned()
                        .unwrap_or_else(|| Type::Scalar("String".into())),
                    _ => Type::Scalar("String".into()),
                }
            };
            (Nodo::Referencia(abajo), Box::new(f))
        }
        // Las dos versiones convergen aquí, y **`Lectura` no cambia**: `Lectura`
        // YA ERA la tabla —datasource, objeto y columnas—, escrita antes de que
        // la tabla existiera en la gramática. Por eso una reforma de la
        // gramática no toca el motor.
        //
        // Lo que falta es de T2: los `campos` salen de lo que la vista usa y no
        // de `columns`, y las capacidades siguen leyéndose de la vista.
        Some(vistas::Fuente::Datasource { .. }) | Some(vistas::Fuente::Tabla(_)) => {
            let (datasource, objeto) = objeto_fisico(pkg, v).unwrap_or_default();
            let mut columnas: BTreeMap<String, Type> = BTreeMap::new();
            let tipo = |c: &str| {
                tipos
                    .get(&(datasource.clone(), objeto.clone(), c.to_string()))
                    .cloned()
                    .unwrap_or_else(|| Type::Scalar("String".into()))
            };
            // v1alpha8 · las columnas de la hoja son **las de la tabla**, no las
            // que esta vista usa. Es la diferencia entre describir el objeto y
            // describir a quien lo consulta, y es para lo que la tabla existe:
            // el `Lee` deja de ser la huella de una consulta y pasa a ser el
            // objeto. Dos vistas sobre la misma tabla producen ahora la misma
            // hoja, que es lo que permite reconocer que comparten origen.
            if let Some(vistas::Fuente::Tabla(qn)) = vistas::fuente(v)
                && let Some(tabla) = pkg.table(&qn)
            {
                for c in vistas::columnas(tabla) {
                    let t = tipo(&c);
                    columnas.insert(c, t);
                }
            }
            for c in campos.values() {
                columnas.entry(c.clone()).or_insert_with(|| tipo(c));
            }
            // Las columnas del `where` también se leen, aunque no se expongan:
            // por eso existe la arista INDIRECT.
            for (c, _) in &filtros {
                columnas.entry(c.clone()).or_insert_with(|| tipo(c));
            }
            let ds = datasource.clone();
            let ob = objeto.clone();
            let tipos = tipos.clone();
            let f = move |c: &str| -> Type {
                tipos
                    .get(&(ds.clone(), ob.clone(), c.to_string()))
                    .cloned()
                    .unwrap_or_else(|| Type::Scalar("String".into()))
            };
            (
                Nodo::Lee(Lectura {
                    datasource,
                    objeto,
                    campos: columnas,
                }),
                Box::new(f),
            )
        }
        None => (
            Nodo::Lee(Lectura {
                datasource: String::new(),
                objeto: String::new(),
                campos: BTreeMap::new(),
            }),
            Box::new(|_| Type::Scalar("String".into())),
        ),
    };

    let filtrada = if filtros.is_empty() {
        hoja
    } else {
        let mut cond: Vec<Expr> = Vec::new();
        for (col, valores) in &filtros {
            let t = tipo_de(col);
            cond.push(match valores.as_slice() {
                [] => Expr::EsNulo(Box::new(Expr::campo(col))),
                [uno] => Expr::Compara {
                    op: Comparador::Igual,
                    izquierda: Box::new(Expr::campo(col)),
                    derecha: Box::new(Expr::Literal(literal(uno, &t))),
                },
                varios => Expr::EnConjunto {
                    campo: col.clone(),
                    valores: varios.iter().map(|x| literal(x, &t)).collect(),
                },
            });
        }
        Nodo::Filtra {
            entrada: Box::new(hoja),
            predicado: if cond.len() == 1 {
                cond.remove(0)
            } else {
                Expr::Y(cond)
            },
        }
    };

    Nodo::Proyecta {
        entrada: Box::new(filtrada),
        campos: campos
            .iter()
            .map(|(campo, en_fuente)| (campo.clone(), Expr::campo(en_fuente)))
            .collect(),
    }
}

/// Un literal de `where`, tipado como la columna que compara. Sin esto un
/// `where: { edad: 30 }` sería una cadena contra un entero, y el Schema Resolver
/// lo rechazaría con razón.
fn literal(raw: &str, t: &Type) -> Valor {
    match t {
        Type::Scalar(s) if s == "Integer" => raw
            .parse::<i64>()
            .map(Valor::Entero)
            .unwrap_or_else(|_| Valor::Cadena(raw.to_string())),
        Type::Scalar(s) if s == "Boolean" => match raw {
            "true" => Valor::Booleano(true),
            "false" => Valor::Booleano(false),
            _ => Valor::Cadena(raw.to_string()),
        },
        Type::Scalar(s) if s == "Decimal" => Valor::Decimal(raw.to_string()),
        Type::Parametric { .. } => Valor::Decimal(raw.to_string()),
        _ => Valor::Cadena(raw.to_string()),
    }
}

// ── El paquete → la clasificación ───────────────────────────────────────────

/// Qué lleva puesto cada columna raíz, por las dos vías que el núcleo conoce:
/// las etiquetas del datasource, y las de cada propiedad de cada entidad
/// respaldada por una vista, bajadas por la cadena hasta la columna.
pub(crate) fn etiquetas_de_raiz(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
) -> BTreeMap<Raiz, BTreeMap<String, String>> {
    let mut out: BTreeMap<Raiz, BTreeMap<String, String>> = BTreeMap::new();

    // Las columnas raíz que existen: las de cada hoja, por campos y por filtros.
    let mut hojas: Vec<(String, String, BTreeSet<String>)> = Vec::new();
    for v in pkg.docs.iter().filter(|d| d.kind == Kind::View) {
        let Some((datasource, objeto)) = objeto_fisico(pkg, v) else {
            continue;
        };
        let mut cols: BTreeSet<String> = vistas::campos(v).into_values().collect();
        cols.extend(vistas::filtros(v).into_iter().map(|(c, _)| c));
        hojas.push((datasource, objeto, cols));
    }

    // Vía 1 · el datasource etiqueta todo lo que sale de él.
    let ds_labels: BTreeMap<String, Vec<(String, String)>> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::OntologyConfig)
        .filter_map(|c| c.section("datasources"))
        .flat_map(|n| n.items().iter())
        .filter_map(|ds| {
            let nombre = ds.get("name")?.1.as_str()?.to_string();
            Some((nombre, labels_de(ds)))
        })
        .collect();
    for (ds, ob, cols) in &hojas {
        for c in cols {
            let raiz = Raiz {
                datasource: ds.clone(),
                objeto: ob.clone(),
                campo: c.clone(),
            };
            let entrada = out.entry(raiz).or_default();
            for (eje, nivel) in ds_labels.get(ds).into_iter().flatten() {
                subir(entrada, lat, eje, nivel);
            }
        }
    }

    // Vía 2 · la entidad, por la cadena.
    let efectivas = ore_core::flow::efectivas(pkg, lat);
    for e in pkg.entities() {
        let Some(v) = vistas::respaldo(pkg, e) else {
            continue;
        };
        let Ok(raiz) = vistas::raiz(pkg, v) else {
            continue;
        };
        let eqn = e.qname().unwrap_or_default();
        for (prop, col) in &raiz.columnas {
            let Some(ls) = efectivas.get(&format!("{eqn}.{prop}")) else {
                continue;
            };
            let entrada = out
                .entry(Raiz {
                    datasource: raiz.datasource.clone(),
                    objeto: raiz.objeto.clone(),
                    campo: col.clone(),
                })
                .or_default();
            for (eje, nivel) in ls {
                subir(entrada, lat, eje, nivel);
            }
        }
    }
    out
}

fn labels_de(n: &Node) -> Vec<(String, String)> {
    n.get("labels")
        .map(|(_, l)| {
            l.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Combina como manda el eje: confidencialidad por arriba, integridad por
/// abajo. La misma regla que el Flow Checker aplica al propagar, aplicada aquí
/// al reunir lo que dos fuentes dicen de la misma columna.
fn subir(
    ls: &mut BTreeMap<String, String>,
    lat: &BTreeMap<String, Lattice>,
    eje: &str,
    nivel: &str,
) {
    let Some(l) = lat.get(eje) else {
        ls.entry(eje.to_string())
            .or_insert_with(|| nivel.to_string());
        return;
    };
    let (Some(nuevo), actual) = (l.index(nivel), ls.get(eje).and_then(|a| l.index(a))) else {
        return;
    };
    let gana = match (actual, l.axis) {
        (None, _) => true,
        (Some(a), Axis::Confidentiality) => nuevo > a,
        (Some(a), Axis::Integrity) => nuevo < a,
    };
    if gana {
        ls.insert(eje.to_string(), nivel.to_string());
    }
}

// ── El paquete → las capacidades ────────────────────────────────────────────

/// Las capacidades de cada fuente. El vocabulario de OOS
/// —`predicatePushdown`, `fullScan`, `requiredFilters`— es el mismo desde el
/// binding, y se traduce al del motor sin inventar nada: `like` y `fullText`
/// no tienen equivalente y no se traducen.
///
/// **Dónde vive el contrato es lo que cambia en v1alpha8**, no el vocabulario:
/// era `capabilities` de la vista y es `reads` de la tabla. Por eso
/// `Capacidades::de_oos` no se toca — sería una segunda traducción del mismo
/// vocabulario, y un contrato escrito dos veces diverge en el tercer consumidor.
/// **La cara `D` por hoja: qué cambios emite cada objeto físico.**
///
/// Solo de las tablas, y no de las vistas v1alpha7: aquellas declaraban
/// `version.witness` —*qué fecha el cambio*— y **no decían qué llega**. Es
/// precisamente el medio contrato que `changes` vino a completar, así que una
/// hoja v1alpha7 se queda **fuera del mapa**, que es como se dice *no lo
/// declaró* sin decir *no emite*.
fn cambios_por_fuente(pkg: &Package) -> BTreeMap<(String, String), Emite> {
    let mut out = BTreeMap::new();
    for t in pkg.docs.iter().filter(|d| d.kind == Kind::Table) {
        let (Some(ds), Some(ob)) = (
            t.section("datasource").and_then(|v| v.as_str()),
            t.section("object").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let e = match vistas::modo(t) {
            vistas::Modo::Ninguno => Emite::Nada,
            vistas::Modo::Anexa => Emite::Altas,
            vistas::Modo::Retracta => Emite::Retracciones,
            vistas::Modo::Upsert => Emite::Upserts,
        };
        out.insert((ds.to_string(), ob.to_string()), e);
    }
    out
}

fn capacidades_por_fuente(pkg: &Package) -> BTreeMap<String, Capacidades> {
    let mut out: BTreeMap<String, Capacidades> = BTreeMap::new();
    for v in pkg.docs.iter().filter(|d| d.kind == Kind::View) {
        let Some((datasource, _)) = objeto_fisico(pkg, v) else {
            continue;
        };

        // v1alpha8 · el contrato es del objeto. Y `requiredFilters` **ya son
        // columnas**: lo exige el origen, y el origen habla de columnas. Por
        // eso aquí no se traduce nada, que es justo lo que la línea de abajo
        // tiene que hacer y era el síntoma de que el campo estaba mal colocado.
        //
        // `reads: none` es un escalar y no un mapa, así que `de_oos` devuelve
        // lo que devuelve para un contrato vacío: nada empujable y sin
        // recorrido. Es la lectura correcta —a un tópico no se le pide nada— y
        // no hace falta un caso especial para llegar a ella.
        if let Some(vistas::Fuente::Tabla(qn)) = vistas::fuente(v)
            && let Some(tabla) = pkg.table(&qn)
            && let Some(reads) = tabla.section("reads")
        {
            out.insert(datasource, Capacidades::de_oos(reads));
            continue;
        }

        // v1alpha7 · el contrato repetido dentro de cada vista que toca la
        // fuente. Se queda mientras haya documentos que lo escriban así.
        let Some(caps) = v.section("capabilities") else {
            continue;
        };
        let mut c = Capacidades::de_oos(caps);
        // Y aquí sí hay que traducir: sus `requiredFilters` vienen en nombres
        // de campo de la vista, y lo que el planificador empuja son columnas.
        let campos = vistas::campos(v);
        c.filtros_obligatorios = c
            .filtros_obligatorios
            .iter()
            .filter_map(|f| campos.get(f).cloned())
            .collect();
        out.insert(datasource, c);
    }
    out
}
