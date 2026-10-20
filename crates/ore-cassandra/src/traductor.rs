//! **De una `Peticion` a CQL** (ficha 0070 §4.4). Una función pura: sin red y sin
//! servidor, porque «el `SELECT` pide solo las columnas que hacen falta» y «nunca
//! `ALLOW FILTERING`» tienen que ser un aserto y no una promesa (0008).
//!
//! ```text
//!   Peticion { proyeccion, filtros, claves, limit, timeout, rango }
//!       │   + Tabla (roles, tipos e índices de system_schema)
//!       ▼
//!   Plan { modo, cql, juegos, leidas, residual, limit… }
//! ```
//!
//! # Lo que el servidor admite
//!
//! Medido en X0 (§3.2): sin `ALLOW FILTERING`, solo la **clave de partición**
//! (`=`, `IN`), los **rangos de clustering** (tras fijar el prefijo) y lo que un
//! **índice** cubra (`=` en uno clásico; `=` y rangos en un SAI). `!=`, `IS NULL`
//! y `LIKE` no existen ni con `ALLOW FILTERING`. Tres modos, de más a menos fino:
//!
//! | modo | cuándo | qué hace |
//! |---|---|---|
//! | [`Modo::Particion`] | toda la clave de partición viene fijada (`=`/`IN`, o `claves`) | un `SELECT` por combinación; sobre el clustering empuja el prefijo y un rango |
//! | [`Modo::Indice`] | hay un predicado que un índice cubre | un `SELECT … WHERE <índice>`, en un solo flujo |
//! | [`Modo::Recorrido`] | el resto | `WHERE token(clave) > ? AND token(clave) <= ?`, en paralelo (el transporte pone los rangos) |
//!
//! # Lo que no se empuja, el conector lo evalúa
//!
//! La spec manda que lo que no se empuja lo evalúe el motor, y que el conector
//! devuelva **exactamente** lo pedido. Aquí el conector **filtra él mismo** lo que
//! el servidor no sabe ([`crate::residual`]), con la semántica de SQL (un nulo no
//! casa con nada). Lo que no se sabe escribir **ni así** —bytes, colecciones,
//! duraciones— se rechaza con `operador`; nunca se ignora.
//!
//! `orderBy` se rechaza: el servidor solo ordena dentro de una partición.

use crate::esquema::{Columna, Indice, Rol, Tabla, citar};
use crate::tipos::Tipo;
use ore_core::tipos::Valor as Analizado;
use ore_driver::{Codigo, Fallo, Filtro, Peticion, Valor};
use std::time::Duration;

/// Los operadores del protocolo 2 (`docs/federation.md` §1.2), los diez.
pub const OPERADORES: &[&str] = &[
    "eq",
    "neq",
    "in",
    "lt",
    "le",
    "gt",
    "ge",
    "like",
    "isNull",
    "isNotNull",
];

/// Cuántas peticiones al servidor se hacen, como mucho, por un producto de `IN`
/// sobre la clave de partición. Pasado esto, se recorre y se filtra.
pub const MAX_JUEGOS: usize = 1000;

/// Un valor ya escrito para el servidor, en la unidad del driver.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    Texto(String),
    Entero(i64),
    Logico(bool),
    /// Milisegundos desde 1970-01-01T00:00Z.
    Instante(i64),
    /// Días desde 1970-01-01.
    Fecha(i32),
    /// Nanosegundos desde la medianoche.
    Hora(i64),
    /// El UUID, en su forma canónica (minúsculas, con guiones).
    Uuid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Particion,
    Indice,
    Recorrido,
}

/// Lo que el que llama puede pedir que cambie.
#[derive(Debug, Clone, Copy, Default)]
pub struct Opciones {
    /// Aunque un índice cubra un predicado, recorrer por rangos de token: un SAI
    /// con un rango **ancho** es ~14 veces más lento por fila que un escaneo
    /// (medido en X0, §3.4). Quien conoce la selectividad del cursor lo decide.
    pub preferir_recorrido: bool,
}

/// Qué se le pide a Cassandra, y qué queda por hacer aquí.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub modo: Modo,
    /// El `SELECT`, con `?` por cada enlace. En [`Modo::Recorrido`] termina en
    /// `token(…) > ? AND token(…) <= ?` y esos dos los pone el transporte.
    pub cql: String,
    /// Los enlaces de cada ejecución. [`Modo::Recorrido`]: vacío, y no significa
    /// «nada que leer». En los otros, **vacío es nada que leer** (un `IN ()`).
    pub juegos: Vec<Vec<Bind>>,
    /// La columna de cada enlace, en el orden de los enlaces de cada juego: el
    /// transporte escribe cada uno con el tipo **exacto** de su columna (`int`,
    /// `timeuuid`…).
    pub enlaces: Vec<String>,
    /// Las columnas que el `SELECT` pide, en orden: la proyección, lo que los
    /// filtros del conector miran y la primera de clustering (una partición solo
    /// con estáticas devuelve una fila con ella a `null`, X0 §3.1).
    pub leidas: Vec<String>,
    /// Lo que el servidor no sabe y evalúa el conector.
    pub residual: Vec<Filtro>,
    /// Un recorte por clave que no es la de partición: lo evalúa el conector.
    pub claves: Option<(Vec<String>, Vec<Vec<String>>)>,
    /// La columna de clustering cuyo `null` marca una fila **solo-estática**, que
    /// no es una fila de la tabla y se salta.
    pub sin_clustering: Option<String>,
    /// Como mucho tantas filas, **en total**: quien lee para al llegar.
    pub limit: Option<u64>,
    /// El `LIMIT` que va en el `SELECT`: solo si no queda nada por filtrar aquí
    /// y es una sola ejecución.
    pub limit_servidor: Option<u64>,
    /// El tiempo límite, **del cliente**: el servidor no tiene uno por consulta.
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Traduccion {
    pub plan: Plan,
    /// `(propiedad, columna)`, en el orden de la petición: lo que sale en cada
    /// fila y de dónde se saca.
    pub rutas: Vec<(String, String)>,
}

fn esquema(msg: impl Into<String>) -> Fallo {
    Fallo::new(Codigo::Esquema, msg)
}

/// Traduce una petición. `Err` es siempre un [`Fallo`] del conector: `operador`
/// si quien pidió mandó lo que el conector no sabe escribir, `esquema` si una
/// columna ya no está.
pub fn traducir(p: &Peticion, t: &Tabla, o: &Opciones) -> Result<Traduccion, Fallo> {
    if p.fichero.is_some() || p.listado.is_some() {
        return Err(Fallo::operador(
            "`fichero` y `listado` son de los orígenes de objetos: no aplican a Cassandra",
        ));
    }
    if p.proyeccion.is_empty() {
        return Err(Fallo::operador(
            "la proyección está vacía: no se pide `SELECT *` ni nada (0008: la proyección es una lista cerrada)",
        ));
    }
    if !p.orden.is_empty() {
        return Err(Fallo::operador(
            "`orderBy`: Cassandra solo ordena dentro de una partición; este conector no sabe ordenar",
        ));
    }
    for (_, col) in &p.proyeccion {
        t.columna(col).ok_or_else(|| {
            esquema(format!(
                "la columna `{col}` ya no está en `{}.{}`",
                t.keyspace, t.nombre
            ))
        })?;
    }

    // Todo lo que filtra: los filtros, y el rango del cursor (start exclusivo,
    // end inclusivo, como `ore-sql`).
    let mut filtros: Vec<Filtro> = p.filtros.clone();
    if p.start.is_some() || p.end.is_some() {
        let Some(c) = p.cursor.as_deref() else {
            return Err(Fallo::operador(
                "un rango por la posición del propio origen no existe en Cassandra (no hay change streams): ficha 0070 §5",
            ));
        };
        if let Some(s) = &p.start {
            filtros.push(Filtro::uno(c, "gt", s));
        }
        if let Some(e) = &p.end {
            filtros.push(Filtro::uno(c, "le", e));
        }
    }
    let literales: Vec<Literal> = filtros
        .iter()
        .map(|f| literal(f, t))
        .collect::<Result<_, _>>()?;

    let mut usado = vec![false; filtros.len()];
    let mut juegos: Vec<Vec<Bind>> = Vec::new();
    let mut donde: Vec<String> = Vec::new();
    let mut enlaces: Vec<String> = Vec::new();
    let mut claves_residual = None;

    // ── 1 · la clave de partición ───────────────────────────────────────────
    let particion = t.particion();
    let nombres_particion: Vec<&str> = particion.iter().map(|c| c.nombre.as_str()).collect();
    let por_claves = claves_de_particion(p, t, &nombres_particion)?;
    let mut modo = Modo::Recorrido;

    if let Some(j) = por_claves {
        modo = Modo::Particion;
        juegos = j;
        donde.extend(
            particion
                .iter()
                .map(|c| format!("{} = ?", citar(&c.nombre))),
        );
        enlaces.extend(particion.iter().map(|c| c.nombre.clone()));
    } else {
        if !p.claves.is_empty() {
            claves_residual = Some((p.clave_columnas.clone(), p.claves.clone()));
        }
        if let Some((j, d)) = particion_por_filtros(&particion, &filtros, &literales, &mut usado) {
            modo = Modo::Particion;
            juegos = j;
            donde.extend(d);
            enlaces.extend(particion.iter().map(|c| c.nombre.clone()));
        }
    }

    // ── 2 · el clustering, solo con la partición fijada ─────────────────────
    if modo == Modo::Particion {
        let (d, _) = clustering(t, &filtros, &literales, &mut usado);
        donde.extend(d.iter().map(|(sql, _, _)| sql.clone()));
        enlaces.extend(d.iter().map(|(_, c, _)| c.clone()));
        for juego in &mut juegos {
            juego.extend(d.iter().flat_map(|(_, _, b)| b.clone()));
        }
    }

    // ── 3 · un índice, si no hay partición ──────────────────────────────────
    if modo == Modo::Recorrido && !o.preferir_recorrido {
        let (d, c, b) = por_indice(t, &filtros, &literales, &mut usado);
        if !d.is_empty() {
            modo = Modo::Indice;
            donde = d;
            enlaces = c;
            juegos = vec![b];
        }
    }

    // ── 4 · lo que el servidor no sabe, lo evalúa el conector ───────────────
    let residual: Vec<Filtro> = filtros
        .iter()
        .zip(&usado)
        .filter(|(_, u)| !**u)
        .map(|(f, _)| f.clone())
        .collect();

    // Las columnas que el `SELECT` pide.
    let mut leidas: Vec<String> = Vec::new();
    let mut pedir = |c: &str| {
        if !leidas.iter().any(|x| x == c) {
            leidas.push(c.to_string());
        }
    };
    for (_, c) in &p.proyeccion {
        pedir(c);
    }
    for f in &residual {
        pedir(&f.columna);
    }
    if let Some((cols, _)) = &claves_residual {
        for c in cols {
            pedir(c);
        }
    }
    let sin_clustering = t.clustering().first().map(|c| c.nombre.clone());
    if let Some(c) = &sin_clustering {
        pedir(c);
    }

    let hay_estaticas = t.columnas.iter().any(|c| c.rol == Rol::Estatica);
    let una_ejecucion = match modo {
        Modo::Recorrido => false,
        _ => juegos.len() == 1,
    };
    let limit_servidor = match p.limit {
        Some(n)
            if residual.is_empty()
                && claves_residual.is_none()
                && una_ejecucion
                && !hay_estaticas =>
        {
            Some(n)
        }
        _ => None,
    };

    let mut cql = format!(
        "SELECT {} FROM {}",
        leidas
            .iter()
            .map(|c| citar(c))
            .collect::<Vec<_>>()
            .join(", "),
        t.objeto_citado()
    );
    match modo {
        Modo::Recorrido => {
            let tok = format!(
                "token({})",
                particion
                    .iter()
                    .map(|c| citar(&c.nombre))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            cql.push_str(&format!(" WHERE {tok} > ? AND {tok} <= ?"));
        }
        _ => {
            cql.push_str(" WHERE ");
            cql.push_str(&donde.join(" AND "));
        }
    }
    if let Some(n) = limit_servidor {
        cql.push_str(&format!(" LIMIT {n}"));
    }

    Ok(Traduccion {
        plan: Plan {
            modo,
            cql,
            juegos,
            enlaces,
            leidas,
            residual,
            claves: claves_residual,
            sin_clustering,
            limit: p.limit,
            limit_servidor,
            timeout: p.timeout_ms.map(Duration::from_millis),
        },
        rutas: p.proyeccion.clone(),
    })
}

// ── Los literales ───────────────────────────────────────────────────────────

/// El valor de un filtro, ya analizado para el tipo de su columna.
#[derive(Debug, Clone)]
pub(crate) enum Literal {
    Uno(Analizado),
    Lista(Vec<Analizado>),
    Ninguno,
}

/// Analiza el valor de un filtro con el físico de la columna y comprueba que el
/// operador tiene sentido. `Err` es `operador` (o `esquema` si la columna ya no
/// está): un filtro que no se sabe leer no se ignora.
pub(crate) fn literal(f: &Filtro, t: &Tabla) -> Result<Literal, Fallo> {
    if !OPERADORES.contains(&f.operador.as_str()) {
        return Err(Fallo::operador(format!(
            "`{}` no es un operador del vocabulario (eq, neq, in, lt, le, gt, ge, like, isNull, isNotNull)",
            f.operador
        )));
    }
    let c = t.columna(&f.columna).ok_or_else(|| {
        esquema(format!(
            "la columna `{}` ya no está en `{}.{}`",
            f.columna, t.keyspace, t.nombre
        ))
    })?;
    if matches!(c.tipo, Tipo::Binario | Tipo::Json | Tipo::Duracion) {
        return Err(Fallo::operador(format!(
            "`{}` sobre `{}`: la columna es {} ({}) y no se compara como SQL; no se filtra",
            f.operador,
            f.columna,
            c.tipo.oos(),
            c.tipo.fisico_origen()
        )));
    }
    let fisico = c.tipo.fisico();
    let uno = |v: &str| {
        fisico.analizar(v).ok_or_else(|| {
            Fallo::operador(format!(
                "`{}` sobre `{}`: `{v}` no es un valor de {}",
                f.operador,
                f.columna,
                c.tipo.oos()
            ))
        })
    };
    match (f.operador.as_str(), &f.valor) {
        ("isNull" | "isNotNull", Valor::Ninguno) => Ok(Literal::Ninguno),
        ("in", Valor::Lista(l)) => Ok(Literal::Lista(
            l.iter().map(|v| uno(v)).collect::<Result<_, _>>()?,
        )),
        ("like", Valor::Uno(v)) => {
            if fisico != ore_core::tipos::Fisico::Texto {
                return Err(Fallo::operador(format!(
                    "`like` sobre `{}`: la columna es {}, no texto",
                    f.columna,
                    c.tipo.oos()
                )));
            }
            Ok(Literal::Uno(Analizado::Texto(v.clone())))
        }
        ("eq" | "neq" | "lt" | "le" | "gt" | "ge", Valor::Uno(v)) => Ok(Literal::Uno(uno(v)?)),
        (op, _) => Err(Fallo::operador(format!(
            "`{op}` sobre `{}`: el valor no tiene la forma del operador",
            f.columna
        ))),
    }
}

/// El enlace de un valor para la columna, o `None` si no se puede empujar tal
/// cual (no cabe en el ancho del tipo, o pierde precisión): entonces lo evalúa el
/// conector, que da el mismo resultado sin riesgo.
fn bind(c: &Columna, v: &Analizado) -> Option<Bind> {
    use crate::tipos::Cql;
    match (&c.tipo, v) {
        (Tipo::Texto, Analizado::Texto(s)) => Some(Bind::Texto(s.clone())),
        (Tipo::Entero, Analizado::Entero(n)) => {
            let (min, max) = match c.cql.sin_frozen() {
                Cql::TinyInt => (i64::from(i8::MIN), i64::from(i8::MAX)),
                Cql::SmallInt => (i64::from(i16::MIN), i64::from(i16::MAX)),
                Cql::Int => (i64::from(i32::MIN), i64::from(i32::MAX)),
                _ => (i64::MIN, i64::MAX),
            };
            (min..=max).contains(n).then_some(Bind::Entero(*n))
        }
        (Tipo::Logico, Analizado::Logico(b)) => Some(Bind::Logico(*b)),
        // Un instante de CQL tiene milisegundos: con microsegundos sueltos un `eq`
        // no casaría nada y un `<=` sí casaría de más. Que lo decida el conector.
        (Tipo::Instante, Analizado::Instante(us)) => {
            (us % 1000 == 0).then_some(Bind::Instante(us / 1000))
        }
        (Tipo::Fecha, Analizado::Fecha(d)) => Some(Bind::Fecha(*d)),
        (Tipo::Hora, Analizado::Hora(us)) => us.checked_mul(1000).map(Bind::Hora),
        (Tipo::Uuid, Analizado::Texto(s)) => uuid_canonico(s).map(Bind::Uuid),
        _ => None,
    }
}

/// `8-4-4-4-12` hexadecimal, en minúsculas; `None` si no lo es.
fn uuid_canonico(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let bien = b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        });
    bien.then(|| s.to_ascii_lowercase())
}

/// Si el servidor entiende este operador sobre este tipo **igual que SQL**.
fn empuja(c: &Columna, op: &str) -> bool {
    let orden = matches!(op, "lt" | "le" | "gt" | "ge");
    match (&c.tipo, op) {
        (Tipo::Entero | Tipo::Texto | Tipo::Instante | Tipo::Fecha | Tipo::Hora, "eq" | "in") => {
            true
        }
        (Tipo::Entero | Tipo::Texto | Tipo::Instante | Tipo::Fecha | Tipo::Hora, _) => orden,
        (Tipo::Logico | Tipo::Uuid, "eq" | "in") => true,
        // `float` y `double` se comparan con otra anchura en el servidor; los
        // decimales y los enteros sin tope, con otra escala; `inet`, con otro
        // orden que el texto. Los evalúa el conector.
        _ => false,
    }
}

fn comparador(op: &str) -> &'static str {
    match op {
        "lt" => "<",
        "le" => "<=",
        "gt" => ">",
        "ge" => ">=",
        _ => "=",
    }
}

// ── La clave de partición ───────────────────────────────────────────────────

/// `claves` (recorte por clave) que son **exactamente** la clave de partición:
/// una ejecución por clave, sin producto de `IN`.
fn claves_de_particion(
    p: &Peticion,
    t: &Tabla,
    particion: &[&str],
) -> Result<Option<Vec<Vec<Bind>>>, Fallo> {
    if p.claves.is_empty() {
        return Ok(None);
    }
    if p.clave_columnas.is_empty() {
        return Err(Fallo::operador("hay `claves` y no `claveColumnas`"));
    }
    for k in &p.claves {
        if k.len() != p.clave_columnas.len() {
            return Err(Fallo::operador(
                "una clave no tiene tantos valores como `claveColumnas`",
            ));
        }
    }
    for c in &p.clave_columnas {
        t.columna(c).ok_or_else(|| {
            esquema(format!(
                "la columna de clave `{c}` ya no está en `{}.{}`",
                t.keyspace, t.nombre
            ))
        })?;
    }
    let mut a: Vec<&str> = p.clave_columnas.iter().map(String::as_str).collect();
    let mut b: Vec<&str> = particion.to_vec();
    a.sort_unstable();
    b.sort_unstable();
    if a != b {
        return Ok(None);
    }
    let mut juegos: Vec<Vec<Bind>> = Vec::new();
    for k in &p.claves {
        let mut juego = Vec::with_capacity(particion.len());
        for nombre in particion {
            let i = p
                .clave_columnas
                .iter()
                .position(|c| c == nombre)
                .expect("comprobado arriba");
            let c = t.columna(nombre).expect("comprobado arriba");
            let analizado = c.tipo.fisico().analizar(&k[i]).ok_or_else(|| {
                Fallo::operador(format!(
                    "la clave `{nombre}`: `{}` no es un valor de {}",
                    k[i],
                    c.tipo.oos()
                ))
            })?;
            match bind(c, &analizado) {
                Some(b) if empuja(c, "eq") => juego.push(b),
                // Una clave que no se puede escribir para el servidor (no cabe
                // en el ancho, o el tipo no se empuja): se recorre y se filtra.
                _ => return Ok(None),
            }
        }
        if !juegos.contains(&juego) {
            juegos.push(juego);
        }
    }
    Ok(Some(juegos))
}

/// Si cada columna de la clave de partición tiene un `=` o un `IN` que el
/// servidor entiende: los juegos de enlaces (el producto de los `IN`) y el
/// `WHERE`. Marca los filtros que consume.
fn particion_por_filtros(
    particion: &[&Columna],
    filtros: &[Filtro],
    lit: &[Literal],
    usado: &mut [bool],
) -> Option<(Vec<Vec<Bind>>, Vec<String>)> {
    let mut por_columna: Vec<(usize, Vec<Bind>)> = Vec::new();
    for c in particion {
        let hallado = filtros.iter().enumerate().find_map(|(i, f)| {
            if f.columna != c.nombre || usado[i] || !matches!(f.operador.as_str(), "eq" | "in") {
                return None;
            }
            if !empuja(c, &f.operador) {
                return None;
            }
            let valores: Vec<&Analizado> = match &lit[i] {
                Literal::Uno(v) => vec![v],
                Literal::Lista(l) => l.iter().collect(),
                Literal::Ninguno => return None,
            };
            let binds: Option<Vec<Bind>> = valores.into_iter().map(|v| bind(c, v)).collect();
            let mut binds = binds?;
            let mut vistos: Vec<Bind> = Vec::with_capacity(binds.len());
            binds.retain(|b| {
                if vistos.contains(b) {
                    false
                } else {
                    vistos.push(b.clone());
                    true
                }
            });
            Some((i, binds))
        });
        por_columna.push(hallado?);
    }
    // El producto, en el orden de la clave. Un `IN ()` en una columna deja la
    // lista vacía: no hay nada que leer.
    let mut total = 1usize;
    for (_, b) in &por_columna {
        total = total.saturating_mul(b.len());
    }
    if total > MAX_JUEGOS {
        return None;
    }
    let mut juegos: Vec<Vec<Bind>> = vec![Vec::new()];
    for (_, binds) in &por_columna {
        let mut siguiente = Vec::with_capacity(juegos.len() * binds.len());
        for j in &juegos {
            for b in binds {
                let mut n = j.clone();
                n.push(b.clone());
                siguiente.push(n);
            }
        }
        juegos = siguiente;
    }
    if total == 0 {
        juegos.clear();
    }
    for (i, _) in &por_columna {
        usado[*i] = true;
    }
    Some((
        juegos,
        particion
            .iter()
            .map(|c| format!("{} = ?", citar(&c.nombre)))
            .collect(),
    ))
}

// ── El clustering ───────────────────────────────────────────────────────────

/// Con la partición fijada: `=` sobre el prefijo de las columnas de clustering,
/// y un rango sobre la siguiente (un límite inferior y uno superior, como mucho).
/// Devuelve cada cláusula con su columna y sus enlaces, y cuántas columnas empujó.
fn clustering(
    t: &Tabla,
    filtros: &[Filtro],
    lit: &[Literal],
    usado: &mut [bool],
) -> (Vec<(String, String, Vec<Bind>)>, usize) {
    let mut salida: Vec<(String, String, Vec<Bind>)> = Vec::new();
    let mut empujadas = 0;
    for c in t.clustering() {
        // Un `=` fija la columna y se sigue con la siguiente.
        let igual = filtros.iter().enumerate().find_map(|(i, f)| {
            if usado[i] || f.columna != c.nombre || f.operador != "eq" || !empuja(c, "eq") {
                return None;
            }
            match &lit[i] {
                Literal::Uno(v) => bind(c, v).map(|b| (i, b)),
                _ => None,
            }
        });
        if let Some((i, b)) = igual {
            usado[i] = true;
            salida.push((
                format!("{} = ?", citar(&c.nombre)),
                c.nombre.clone(),
                vec![b],
            ));
            empujadas += 1;
            continue;
        }
        // Sin `=`: un rango, y no se sigue (CQL exige contigüidad).
        let (mut inferior, mut superior) = (false, false);
        for (i, f) in filtros.iter().enumerate() {
            if usado[i] || f.columna != c.nombre || !empuja(c, &f.operador) {
                continue;
            }
            let es_inferior = matches!(f.operador.as_str(), "gt" | "ge");
            let es_superior = matches!(f.operador.as_str(), "lt" | "le");
            if !(es_inferior || es_superior)
                || (es_inferior && inferior)
                || (es_superior && superior)
            {
                continue;
            }
            let Literal::Uno(v) = &lit[i] else { continue };
            let Some(b) = bind(c, v) else { continue };
            usado[i] = true;
            if es_inferior {
                inferior = true;
            } else {
                superior = true;
            }
            salida.push((
                format!("{} {} ?", citar(&c.nombre), comparador(&f.operador)),
                c.nombre.clone(),
                vec![b],
            ));
        }
        if inferior || superior {
            empujadas += 1;
        }
        break;
    }
    (salida, empujadas)
}

// ── Los índices ─────────────────────────────────────────────────────────────

/// Sin partición fijada: lo que un índice cubre. Un SAI admite `=` y rangos, y
/// todos los que haya se juntan con `AND`; un índice clásico, un solo `=` (y solo
/// si no hay SAI). Devuelve el `WHERE`, la columna de cada enlace y los enlaces; vacío si no hay nada.
fn por_indice(
    t: &Tabla,
    filtros: &[Filtro],
    lit: &[Literal],
    usado: &mut [bool],
) -> (Vec<String>, Vec<String>, Vec<Bind>) {
    let mut donde = Vec::new();
    let mut columnas = Vec::new();
    let mut binds = Vec::new();
    for (i, f) in filtros.iter().enumerate() {
        let Some(c) = t.columna(&f.columna) else {
            continue;
        };
        if usado[i] || c.indice != Indice::Sai || !empuja(c, &f.operador) {
            continue;
        }
        if matches!(f.operador.as_str(), "in") {
            continue; // SAI no admite `IN` (5.0): lo evalúa el conector.
        }
        let Literal::Uno(v) = &lit[i] else { continue };
        let Some(b) = bind(c, v) else { continue };
        usado[i] = true;
        donde.push(format!(
            "{} {} ?",
            citar(&c.nombre),
            comparador(&f.operador)
        ));
        columnas.push(c.nombre.clone());
        binds.push(b);
    }
    if donde.is_empty() {
        // Un índice clásico: un `=`, uno solo.
        for (i, f) in filtros.iter().enumerate() {
            let Some(c) = t.columna(&f.columna) else {
                continue;
            };
            if usado[i] || c.indice != Indice::Clasico || f.operador != "eq" || !empuja(c, "eq") {
                continue;
            }
            let Literal::Uno(v) = &lit[i] else { continue };
            let Some(b) = bind(c, v) else { continue };
            usado[i] = true;
            donde.push(format!("{} = ?", citar(&c.nombre)));
            columnas.push(c.nombre.clone());
            binds.push(b);
            break;
        }
    }
    (donde, columnas, binds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esquema::pruebas::{clasico, col, pedidos, sai};
    use ore_driver::Orden;

    fn pet(cols: &[&str], filtros: Vec<Filtro>) -> Peticion {
        Peticion {
            objeto: "ore_prueba.pedidos".into(),
            proyeccion: cols
                .iter()
                .map(|c| (c.to_string(), c.to_string()))
                .collect(),
            filtros,
            ..Default::default()
        }
    }

    fn eventos() -> Tabla {
        Tabla::de_system_schema(
            "ore_prueba",
            "eventos",
            &[
                col("id", "partition_key", 0, "none", "int"),
                col("actualizado", "regular", -1, "none", "timestamp"),
                col("cuerpo", "regular", -1, "none", "text"),
                col("n", "regular", -1, "none", "int"),
            ],
            &[sai("ev_act", "actualizado"), clasico("ev_n", "n")],
        )
        .unwrap()
    }

    fn tr(p: &Peticion, t: &Tabla) -> Plan {
        traducir(p, t, &Opciones::default()).unwrap().plan
    }

    #[test]
    fn sin_filtros_se_recorre_por_rangos_de_token() {
        let t = pedidos(&[]);
        let plan = tr(&pet(&["total", "estado"], vec![]), &t);
        assert_eq!(plan.modo, Modo::Recorrido);
        assert_eq!(
            plan.cql,
            "SELECT \"total\", \"estado\", \"hora\" FROM \"ore_prueba\".\"pedidos\" WHERE token(\"cliente\", \"dia\") > ? AND token(\"cliente\", \"dia\") <= ?"
        );
        assert!(plan.juegos.is_empty() && plan.residual.is_empty());
        // La primera de clustering se pide para saltar las filas solo-estáticas.
        assert_eq!(plan.sin_clustering.as_deref(), Some("hora"));
    }

    #[test]
    fn nunca_allow_filtering() {
        let t = pedidos(&[]);
        for f in [
            Filtro::uno("estado", "eq", "abierto"),
            Filtro::uno("total", "gt", "10"),
            Filtro::uno("nota", "like", "a%"),
        ] {
            let plan = tr(&pet(&["total"], vec![f]), &t);
            assert!(
                !plan.cql.to_ascii_uppercase().contains("ALLOW"),
                "{}",
                plan.cql
            );
            assert_eq!(plan.modo, Modo::Recorrido);
            assert_eq!(plan.residual.len(), 1);
        }
    }

    #[test]
    fn la_clave_de_particion_entera_es_una_consulta_por_particion() {
        let t = pedidos(&[]);
        let plan = tr(
            &pet(
                &["total"],
                vec![
                    Filtro::uno("cliente", "eq", "7"),
                    Filtro::uno("dia", "eq", "2026-10-20"),
                ],
            ),
            &t,
        );
        assert_eq!(plan.modo, Modo::Particion);
        assert_eq!(
            plan.cql,
            "SELECT \"total\", \"hora\" FROM \"ore_prueba\".\"pedidos\" WHERE \"cliente\" = ? AND \"dia\" = ?"
        );
        assert_eq!(plan.juegos.len(), 1);
        assert_eq!(plan.juegos[0][0], Bind::Entero(7));
        assert!(matches!(plan.juegos[0][1], Bind::Fecha(_)));
        assert!(plan.residual.is_empty());
    }

    #[test]
    fn media_clave_de_particion_no_basta() {
        let t = pedidos(&[]);
        let plan = tr(
            &pet(&["total"], vec![Filtro::uno("cliente", "eq", "7")]),
            &t,
        );
        assert_eq!(plan.modo, Modo::Recorrido);
        assert_eq!(plan.residual.len(), 1);
    }

    #[test]
    fn un_in_sobre_la_particion_es_un_producto_de_ejecuciones() {
        let t = pedidos(&[]);
        let plan = tr(
            &pet(
                &["total"],
                vec![
                    Filtro {
                        columna: "cliente".into(),
                        operador: "in".into(),
                        valor: Valor::Lista(vec!["1".into(), "2".into(), "2".into()]),
                    },
                    Filtro::uno("dia", "eq", "2026-10-20"),
                ],
            ),
            &t,
        );
        assert_eq!(plan.modo, Modo::Particion);
        assert_eq!(plan.juegos.len(), 2, "el 2 repetido se quita");
        // Un `IN ()` no deja pasar ninguna fila: nada que leer.
        let vacio = tr(
            &pet(
                &["total"],
                vec![
                    Filtro {
                        columna: "cliente".into(),
                        operador: "in".into(),
                        valor: Valor::Lista(vec![]),
                    },
                    Filtro::uno("dia", "eq", "2026-10-20"),
                ],
            ),
            &t,
        );
        assert_eq!(vacio.modo, Modo::Particion);
        assert!(vacio.juegos.is_empty());
    }

    #[test]
    fn demasiadas_ejecuciones_se_recorren_y_se_filtran() {
        let t = Tabla::de_system_schema(
            "k",
            "t",
            &[
                col("a", "partition_key", 0, "none", "int"),
                col("b", "partition_key", 1, "none", "int"),
                col("v", "regular", -1, "none", "text"),
            ],
            &[],
        )
        .unwrap();
        let lista = |c: &str| Filtro {
            columna: c.into(),
            operador: "in".into(),
            valor: Valor::Lista((0..50).map(|n| n.to_string()).collect()),
        };
        let mut p = pet(&["v"], vec![lista("a"), lista("b")]);
        p.objeto = "k.t".into();
        let plan = tr(&p, &t);
        assert_eq!(plan.modo, Modo::Recorrido, "50 × 50 > {MAX_JUEGOS}");
        assert_eq!(plan.residual.len(), 2);
    }

    #[test]
    fn el_clustering_empuja_el_prefijo_y_un_rango() {
        let t = pedidos(&[]);
        let plan = tr(
            &pet(
                &["total"],
                vec![
                    Filtro::uno("cliente", "eq", "7"),
                    Filtro::uno("dia", "eq", "2026-10-20"),
                    Filtro::uno("hora", "ge", "2026-10-20T10:00:00Z"),
                    Filtro::uno("hora", "lt", "2026-10-20T11:00:00Z"),
                    Filtro::uno("id", "eq", "60588980-cc6e-11f1-8a8f-a74bef58c055"),
                ],
            ),
            &t,
        );
        assert_eq!(plan.modo, Modo::Particion);
        assert!(
            plan.cql.ends_with(
                "WHERE \"cliente\" = ? AND \"dia\" = ? AND \"hora\" >= ? AND \"hora\" < ?"
            ),
            "{}",
            plan.cql
        );
        // `id` va tras un rango: CQL exige contigüidad, lo evalúa el conector.
        assert_eq!(plan.residual.len(), 1);
        assert_eq!(plan.residual[0].columna, "id");
        assert_eq!(plan.juegos[0].len(), 4);
        assert!(matches!(
            plan.juegos[0][2],
            Bind::Instante(1_792_000_000_000..=1_793_000_000_000)
        ));
    }

    #[test]
    fn el_clustering_sin_la_particion_no_se_empuja() {
        let t = pedidos(&[]);
        let plan = tr(
            &pet(
                &["total"],
                vec![Filtro::uno("hora", "ge", "2026-10-20T10:00:00Z")],
            ),
            &t,
        );
        assert_eq!(plan.modo, Modo::Recorrido);
        assert_eq!(plan.residual.len(), 1);
    }

    #[test]
    fn un_sai_cubre_la_igualdad_y_los_rangos_pero_no_el_in() {
        let t = eventos();
        let mut p = pet(
            &["id"],
            vec![
                Filtro::uno("actualizado", "gt", "2026-10-20T10:30:00Z"),
                Filtro::uno("actualizado", "le", "2026-10-20T11:30:00Z"),
                Filtro::uno("cuerpo", "eq", "b"),
            ],
        );
        p.objeto = "ore_prueba.eventos".into();
        let plan = tr(&p, &t);
        assert_eq!(plan.modo, Modo::Indice);
        assert_eq!(
            plan.cql,
            "SELECT \"id\", \"cuerpo\" FROM \"ore_prueba\".\"eventos\" WHERE \"actualizado\" > ? AND \"actualizado\" <= ?"
        );
        assert_eq!(plan.juegos.len(), 1);
        assert_eq!(plan.juegos[0].len(), 2);
        assert_eq!(plan.residual, [Filtro::uno("cuerpo", "eq", "b")]);

        // Quien conoce la selectividad puede pedir recorrer.
        let r = traducir(
            &p,
            &t,
            &Opciones {
                preferir_recorrido: true,
            },
        )
        .unwrap()
        .plan;
        assert_eq!(r.modo, Modo::Recorrido);
        assert_eq!(r.residual.len(), 3);
    }

    #[test]
    fn un_indice_clasico_es_solo_igualdad_y_uno() {
        let t = eventos();
        let mut p = pet(&["id"], vec![Filtro::uno("n", "eq", "2")]);
        p.objeto = "ore_prueba.eventos".into();
        assert_eq!(tr(&p, &t).modo, Modo::Indice);
        let mut p = pet(&["id"], vec![Filtro::uno("n", "gt", "2")]);
        p.objeto = "ore_prueba.eventos".into();
        let plan = tr(&p, &t);
        assert_eq!(plan.modo, Modo::Recorrido);
        assert_eq!(plan.residual.len(), 1);
    }

    #[test]
    fn el_rango_del_cursor_es_start_exclusivo_y_end_inclusivo() {
        let t = eventos();
        let mut p = pet(&["id"], vec![]);
        p.objeto = "ore_prueba.eventos".into();
        p.cursor = Some("actualizado".into());
        p.start = Some("2026-10-20T10:00:00Z".into());
        p.end = Some("2026-10-20T12:00:00Z".into());
        let plan = tr(&p, &t);
        assert_eq!(plan.modo, Modo::Indice);
        assert!(
            plan.cql
                .ends_with("WHERE \"actualizado\" > ? AND \"actualizado\" <= ?"),
            "{}",
            plan.cql
        );
        // Sin `cursor`, un rango por la posición del origen no existe.
        p.cursor = None;
        assert!(
            traducir(&p, &t, &Opciones::default())
                .unwrap_err()
                .mensaje
                .contains("0070")
        );
    }

    #[test]
    fn lo_que_el_servidor_no_tiene_lo_evalua_el_conector() {
        let t = pedidos(&[]);
        for (c, op, v) in [
            ("estado", "neq", "x"),
            ("estado", "like", "a%"),
            ("nota", "isNull", ""),
            ("nota", "isNotNull", ""),
        ] {
            let f = if v.is_empty() {
                Filtro {
                    columna: c.into(),
                    operador: op.into(),
                    valor: Valor::Ninguno,
                }
            } else {
                Filtro::uno(c, op, v)
            };
            let plan = tr(&pet(&["total"], vec![f]), &t);
            assert_eq!(plan.residual.len(), 1, "{op}");
            assert!(
                plan.leidas.contains(&c.to_string()),
                "se pide la columna que se filtra: {op}"
            );
        }
    }

    #[test]
    fn un_valor_que_no_cabe_en_el_ancho_no_se_empuja_y_no_falla() {
        let t = Tabla::de_system_schema(
            "k",
            "t",
            &[
                col("a", "partition_key", 0, "none", "int"),
                col("v", "regular", -1, "none", "text"),
            ],
            &[],
        )
        .unwrap();
        let mut p = pet(&["v"], vec![Filtro::uno("a", "eq", "99999999999")]);
        p.objeto = "k.t".into();
        let plan = tr(&p, &t);
        // Ningún `int` vale eso: el conector lo evalúa y no devuelve nada.
        assert_eq!(plan.modo, Modo::Recorrido);
        assert_eq!(plan.residual.len(), 1);
    }

    #[test]
    fn un_instante_con_microsegundos_sueltos_no_se_empuja() {
        let t = pedidos(&[]);
        let f = |h: &str| {
            vec![
                Filtro::uno("cliente", "eq", "7"),
                Filtro::uno("dia", "eq", "2026-10-20"),
                Filtro::uno("hora", "le", h),
            ]
        };
        let entero = tr(&pet(&["total"], f("2026-10-20T10:00:00.123Z")), &t);
        assert!(entero.cql.contains("\"hora\" <= ?") && entero.residual.is_empty());
        let suelto = tr(&pet(&["total"], f("2026-10-20T10:00:00.123456Z")), &t);
        let donde = suelto.cql.split(" WHERE ").nth(1).unwrap_or("");
        assert!(!donde.contains("\"hora\""), "{}", suelto.cql);
        assert_eq!(suelto.residual.len(), 1);
    }

    #[test]
    fn las_claves_que_son_la_particion_son_una_ejecucion_por_clave() {
        let t = pedidos(&[]);
        let mut p = pet(&["total"], vec![]);
        p.clave_columnas = vec!["dia".into(), "cliente".into()];
        p.claves = vec![
            vec!["2026-10-20".into(), "1".into()],
            vec!["2026-10-21".into(), "2".into()],
            vec!["2026-10-20".into(), "1".into()],
        ];
        let plan = tr(&p, &t);
        assert_eq!(plan.modo, Modo::Particion);
        assert_eq!(plan.juegos.len(), 2, "la repetida se quita");
        // Los enlaces van en el orden de la clave de partición, no en el de la petición.
        assert_eq!(plan.juegos[0][0], Bind::Entero(1));
        assert!(plan.claves.is_none());
    }

    #[test]
    fn otras_claves_las_evalua_el_conector() {
        let t = pedidos(&[]);
        let mut p = pet(&["total"], vec![]);
        p.clave_columnas = vec!["estado".into()];
        p.claves = vec![vec!["a".into()], vec!["b".into()]];
        let plan = tr(&p, &t);
        assert_eq!(plan.modo, Modo::Recorrido);
        assert!(plan.claves.is_some());
        assert!(plan.leidas.contains(&"estado".to_string()));
    }

    #[test]
    fn el_limit_va_al_servidor_solo_si_no_queda_nada_por_filtrar() {
        let t = eventos();
        let mut p = pet(&["id"], vec![Filtro::uno("id", "eq", "1")]);
        p.objeto = "ore_prueba.eventos".into();
        p.limit = Some(5);
        let plan = tr(&p, &t);
        assert_eq!(plan.limit_servidor, Some(5));
        assert!(plan.cql.ends_with(" LIMIT 5"));
        // Con algo que el conector filtra, el servidor no puede cortar.
        p.filtros.push(Filtro::uno("cuerpo", "neq", "x"));
        let plan = tr(&p, &t);
        assert_eq!((plan.limit, plan.limit_servidor), (Some(5), None));
        assert!(!plan.cql.contains("LIMIT"));
        // Ni en un recorrido: cada rango cortaría por su cuenta.
        let mut p = pet(&["id"], vec![]);
        p.objeto = "ore_prueba.eventos".into();
        p.limit = Some(5);
        assert_eq!(tr(&p, &t).limit_servidor, None);
    }

    #[test]
    fn el_tiempo_es_del_cliente() {
        let t = eventos();
        let mut p = pet(&["id"], vec![]);
        p.timeout_ms = Some(1500);
        assert_eq!(tr(&p, &t).timeout, Some(Duration::from_millis(1500)));
    }

    #[test]
    fn lo_que_no_se_sabe_escribir_se_rechaza_no_se_ignora() {
        let t = pedidos(&[]);
        let malo = |p: &Peticion| traducir(p, &t, &Opciones::default()).unwrap_err();

        let e = malo(&pet(&["total"], vec![Filtro::uno("total", "raro", "1")]));
        assert_eq!(e.codigo, Codigo::Operador);

        let mut p = pet(&["total"], vec![]);
        p.orden = vec![Orden {
            columna: "hora".into(),
            descendente: false,
        }];
        assert!(malo(&p).mensaje.contains("orderBy"));

        let e = malo(&pet(&[], vec![]));
        assert!(e.mensaje.contains("proyección está vacía"));

        // Un valor que no es del tipo de su columna.
        let e = malo(&pet(&["total"], vec![Filtro::uno("cliente", "eq", "abc")]));
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(
            e.mensaje.contains("no es un valor de Integer"),
            "{}",
            e.mensaje
        );

        // Una columna que ya no está: `esquema`, no `operador`.
        let e = malo(&pet(&["no_existe"], vec![]));
        assert_eq!(e.codigo, Codigo::Esquema);
        let e = malo(&pet(&["total"], vec![Filtro::uno("fantasma", "eq", "1")]));
        assert_eq!(e.codigo, Codigo::Esquema);

        // `like` solo sobre texto.
        let e = malo(&pet(&["total"], vec![Filtro::uno("cliente", "like", "1%")]));
        assert!(e.mensaje.contains("no texto"));
    }

    #[test]
    fn bytes_colecciones_y_duraciones_no_se_filtran() {
        let t = Tabla::de_system_schema(
            "k",
            "t",
            &[
                col("a", "partition_key", 0, "none", "int"),
                col("b", "regular", -1, "none", "blob"),
                col("l", "regular", -1, "none", "list<int>"),
                col("d", "regular", -1, "none", "duration"),
            ],
            &[],
        )
        .unwrap();
        for c in ["b", "l", "d"] {
            let mut p = pet(&["a"], vec![Filtro::uno(c, "eq", "x")]);
            p.objeto = "k.t".into();
            let e = traducir(&p, &t, &Opciones::default()).unwrap_err();
            assert_eq!(e.codigo, Codigo::Operador, "{c}");
            assert!(
                e.mensaje.contains("no se compara como SQL"),
                "{c}: {}",
                e.mensaje
            );
        }
        // Pero se pueden leer.
        let mut p = pet(&["a", "b", "l", "d"], vec![]);
        p.objeto = "k.t".into();
        assert!(traducir(&p, &t, &Opciones::default()).is_ok());
    }

    #[test]
    fn un_nombre_con_comillas_no_es_sintaxis() {
        let t = Tabla::de_system_schema(
            "k\"s",
            "t\"x",
            &[
                col("a\"b", "partition_key", 0, "none", "int"),
                col("v", "regular", -1, "none", "text"),
            ],
            &[],
        )
        .unwrap();
        let mut p = pet(&["a\"b"], vec![]);
        p.proyeccion = vec![("a".into(), "a\"b".into())];
        let plan = tr(&p, &t);
        assert!(
            plan.cql.starts_with(
                "SELECT \"a\"\"b\" FROM \"k\"\"s\".\"t\"\"x\" WHERE token(\"a\"\"b\")"
            ),
            "{}",
            plan.cql
        );
    }
}
