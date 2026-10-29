//! **Los verbos que son de CockroachDB** y no del esqueleto: `catalogo`, `check`, `explorar`, `testigo` y `cambio` (ADR 0080, paso 6).
//! Los cuatro de lectura (`capacidades`, `leer`, `estimar`, `servir`) están en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por stdout, o el motivo del fallo (que va a stderr, literal, sin la
//! credencial).
//!
//! **Lo que ve el usuario es lo que puede leer.** `pg_class` de CockroachDB enseña todas las tablas de la base aunque el usuario no tenga permiso
//! sobre ellas (medido en 22.2, 24.3 y 25.4), así que el catálogo filtra con `has_table_privilege(…, 'SELECT')`: no se descubre lo que no se puede
//! leer. Las columnas **ocultas** (la de *shard* de una clave *hash-sharded*) aparecen en `pg_attribute` y en las claves, y se descartan.

use crate::lectura::{Cockroach, Sesion, fallo_de};
use crate::leer::citado;
use crate::tipos::traducir;
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Foranea, Tabla, escribir};
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::ExitCode;

/// Los esquemas que mantiene el servidor: no son datos de nadie.
const ESQUEMAS_DEL_SERVIDOR: &str =
    "'pg_catalog', 'information_schema', 'crdb_internal', 'pg_extension'";

/// Lo que puede tardar la huella de `cambio` antes de rendirse (ver [`cambio_con`]).
pub const HUELLA_MS: u64 = 30_000;

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-cockroach: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(
            entrada.trim(),
            args.first().map_or("cockroach", String::as_str),
        ),
        "check" => check(&entrada),
        "explorar" => explorar(&entrada),
        "testigo" => testigo(&entrada),
        "cambio" => cambio(&entrada),
        otro => Err(format!(
            "verbo desconocido `{otro}` (capacidades, leer, estimar, servir, catalogo, check, explorar, testigo, cambio)"
        )),
    };
    match hecho {
        Ok(salida) => {
            println!("{salida}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-read-cockroach: {m}");
            ExitCode::FAILURE
        }
    }
}

/// Un fallo del conector, como el texto que `ore` enseña: sin la credencial.
fn texto(f: Fallo, url: &str) -> String {
    f.tapado(url).mensaje
}

fn abrir(url: &str) -> Result<Sesion, String> {
    Cockroach::conectar(url).map_err(|f| texto(f, url))
}

// ── Lo que hay en el servidor ───────────────────────────────────────────────

/// Una tabla o vista que el usuario puede leer.
#[derive(Debug, Clone, PartialEq)]
pub struct Objeto {
    pub esquema: String,
    pub nombre: String,
    /// `table`, `view` o `materializedView`.
    pub clase: String,
    pub comentario: Option<String>,
    /// El identificador del objeto en el servidor (`pg_class.oid`, que es el de su descriptor).
    pub oid: i64,
}

impl Objeto {
    pub fn completo(&self) -> String {
        format!("{}.{}", self.esquema, self.nombre)
    }
}

fn clase_de(relkind: &str) -> &'static str {
    match relkind {
        "v" => "view",
        "m" => "materializedView",
        _ => "table",
    }
}

/// Las tablas y vistas que el usuario puede leer, de la base de la URL.
pub fn objetos(c: &mut postgres::Client) -> Result<Vec<Objeto>, Fallo> {
    let filas = c
        .query(
            &format!(
                "SELECT n.nspname::text, c.relname::text, c.relkind::text, obj_description(c.oid, 'pg_class')::text, c.oid::int8 \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.relkind IN ('r', 'v', 'm', 'p') AND n.nspname NOT IN ({ESQUEMAS_DEL_SERVIDOR}) \
                   AND has_table_privilege(c.oid, 'SELECT') \
                 ORDER BY 1, 2"
            ),
            &[],
        )
        .map_err(|e| fallo_de(&e))?;
    Ok(filas
        .iter()
        .map(|f| Objeto {
            esquema: f.get(0),
            nombre: f.get(1),
            clase: clase_de(&f.get::<_, String>(2)).to_string(),
            comentario: f.get::<_, Option<String>>(3).filter(|c| !c.is_empty()),
            oid: f.get(4),
        })
        .collect())
}

/// Las filas que CockroachDB cree que tiene cada tabla (`oid` → filas), de sus estadísticas. **Estimadas, nunca contadas**: una tabla sin
/// estadísticas (recién creada) no sale, y un cero tampoco (no se distingue de «sin estadísticas», y decir 0 filas de una tabla que no se ha
/// mirado haría proponer borrarla). Si el usuario no puede ver la vista, no hay estimaciones y ya está.
pub fn estimaciones(c: &mut postgres::Client) -> HashMap<i64, u64> {
    c.query(
        "SELECT table_id::int8, estimated_row_count::int8 FROM crdb_internal.table_row_statistics WHERE estimated_row_count > 0",
        &[],
    )
    .map(|filas| {
        filas
            .iter()
            .filter_map(|f| Some((f.get::<_, i64>(0), u64::try_from(f.get::<_, i64>(1)).ok()?)))
            .collect()
    })
    .unwrap_or_default()
}

/// Una columna como la dice el catálogo del servidor.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnaCrdb {
    pub esquema: String,
    pub tabla: String,
    pub attnum: i64,
    pub nombre: String,
    pub formato: String,
    pub familia: String,
    pub base: Option<String>,
    pub obligatoria: bool,
    pub comentario: Option<String>,
    pub tipo_oid: i64,
}

/// Las columnas ocultas de cada tabla, por `(esquema, tabla, columna)`.
fn ocultas(c: &mut postgres::Client) -> Result<HashSet<(String, String, String)>, Fallo> {
    let filas = c
        .query(
            "SELECT table_schema::text, table_name::text, column_name::text FROM information_schema.columns WHERE is_hidden = 'YES'",
            &[],
        )
        .map_err(|e| fallo_de(&e))?;
    Ok(filas
        .iter()
        .map(|f| (f.get(0), f.get(1), f.get(2)))
        .collect())
}

/// Las columnas visibles de lo que el usuario puede leer, en el orden de cada tabla.
pub fn columnas_del_servidor(c: &mut postgres::Client) -> Result<Vec<ColumnaCrdb>, Fallo> {
    let ocultas = ocultas(c)?;
    let filas = c
        .query(
            &format!(
                "SELECT n.nspname::text, c.relname::text, a.attnum::int8, a.attname::text, format_type(a.atttypid, a.atttypmod)::text, \
                        t.typtype::text, CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod)::text END, \
                        a.attnotnull, col_description(c.oid, a.attnum::int)::text, a.atttypid::int8 \
                 FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
                      JOIN pg_type t ON t.oid = a.atttypid \
                 WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r', 'v', 'm', 'p') \
                   AND n.nspname NOT IN ({ESQUEMAS_DEL_SERVIDOR}) AND has_table_privilege(c.oid, 'SELECT') \
                 ORDER BY 1, 2, 3"
            ),
            &[],
        )
        .map_err(|e| fallo_de(&e))?;
    Ok(filas
        .iter()
        .map(|f| ColumnaCrdb {
            esquema: f.get(0),
            tabla: f.get(1),
            attnum: f.get(2),
            nombre: f.get(3),
            formato: f.get(4),
            familia: f.get(5),
            base: f.get(6),
            obligatoria: f.get(7),
            comentario: f.get::<_, Option<String>>(8).filter(|c| !c.is_empty()),
            tipo_oid: f.get(9),
        })
        .filter(|k| !ocultas.contains(&(k.esquema.clone(), k.tabla.clone(), k.nombre.clone())))
        .collect())
}

/// Los valores de cada `ENUM`, por el `oid` de su tipo y **en el orden de declaración** (que no es el alfabético).
fn enums(c: &mut postgres::Client) -> HashMap<i64, Vec<String>> {
    let mut m: HashMap<i64, Vec<String>> = HashMap::new();
    if let Ok(filas) = c.query(
        "SELECT enumtypid::int8, enumlabel::text FROM pg_enum ORDER BY enumtypid, enumsortorder",
        &[],
    ) {
        for f in &filas {
            m.entry(f.get(0)).or_default().push(f.get(1));
        }
    }
    m
}

/// La clave primaria y las únicas de cada `(esquema, tabla)`.
pub type ClavesPorTabla = BTreeMap<(String, String), (Vec<String>, Vec<Vec<String>>)>;

/// Una fila de `table_constraints` × `key_column_usage`: `(esquema, tabla, restricción, tipo, columna, posición)`.
type FilaClave = (String, String, String, String, String, i64);

fn filas_de_claves(c: &mut postgres::Client) -> Result<Vec<FilaClave>, Fallo> {
    let filas = c
        .query(
            &format!(
                "SELECT t.table_schema::text, t.table_name::text, t.constraint_name::text, t.constraint_type::text, k.column_name::text, \
                        k.ordinal_position::int8 \
                 FROM information_schema.table_constraints t JOIN information_schema.key_column_usage k \
                   ON k.constraint_name = t.constraint_name AND k.table_schema = t.table_schema AND k.table_name = t.table_name \
                 WHERE t.constraint_type IN ('PRIMARY KEY', 'UNIQUE') AND t.table_schema NOT IN ({ESQUEMAS_DEL_SERVIDOR}) \
                 ORDER BY 1, 2, 3, 6"
            ),
            &[],
        )
        .map_err(|e| fallo_de(&e))?;
    Ok(filas
        .iter()
        .map(|f| (f.get(0), f.get(1), f.get(2), f.get(3), f.get(4), f.get(5)))
        .collect())
}

/// La clave primaria y las claves únicas de cada tabla, **sin las columnas ocultas** (la de *shard*). Una restricción que se queda sin columnas
/// visibles no existe para el catálogo, y una única idéntica a la primaria tampoco se repite.
pub fn claves(filas: &[FilaClave], ocultas: &HashSet<(String, String, String)>) -> ClavesPorTabla {
    let mut por_restriccion: BTreeMap<(String, String, String), (String, Vec<String>)> =
        BTreeMap::new();
    for (esq, tab, nombre, tipo, col, _) in filas {
        if ocultas.contains(&(esq.clone(), tab.clone(), col.clone())) {
            continue;
        }
        por_restriccion
            .entry((esq.clone(), tab.clone(), nombre.clone()))
            .or_insert_with(|| (tipo.clone(), Vec::new()))
            .1
            .push(col.clone());
    }
    let mut m = ClavesPorTabla::new();
    for ((esq, tab, _), (tipo, cols)) in por_restriccion {
        let e = m.entry((esq, tab)).or_default();
        if tipo == "PRIMARY KEY" {
            e.0 = cols;
        } else {
            e.1.push(cols);
        }
    }
    for (clave, unicas) in m.values_mut() {
        unicas.retain(|u| u != clave);
        // El nombre de una restricción es arbitrario (`u_hash`, `t_c_key`…): el orden sale de las columnas, que es lo que dice algo.
        unicas.sort();
    }
    m
}

/// Las claves foráneas, por tabla de origen. Una cuyo destino el usuario no puede leer no se puede nombrar y no sale.
fn foraneas(
    c: &mut postgres::Client,
    nombres: &HashMap<(String, String, i64), String>,
) -> Result<HashMap<(String, String), Vec<Foranea>>, Fallo> {
    let filas = c
        .query(
            &format!(
                "SELECT n.nspname::text, r.relname::text, dn.nspname::text, d.relname::text, con.conkey::int8[], con.confkey::int8[] \
                 FROM pg_constraint con JOIN pg_class r ON r.oid = con.conrelid JOIN pg_namespace n ON n.oid = r.relnamespace \
                      JOIN pg_class d ON d.oid = con.confrelid JOIN pg_namespace dn ON dn.oid = d.relnamespace \
                 WHERE con.contype = 'f' AND n.nspname NOT IN ({ESQUEMAS_DEL_SERVIDOR}) \
                 ORDER BY 1, 2, con.conname"
            ),
            &[],
        )
        .map_err(|e| fallo_de(&e))?;
    let mut m: HashMap<(String, String), Vec<Foranea>> = HashMap::new();
    for f in &filas {
        let (esq, tab, desq, dtab): (String, String, String, String) =
            (f.get(0), f.get(1), f.get(2), f.get(3));
        let (conkey, confkey): (Vec<i64>, Vec<i64>) = (f.get(4), f.get(5));
        let locales: Option<Vec<String>> = conkey
            .iter()
            .map(|n| nombres.get(&(esq.clone(), tab.clone(), *n)).cloned())
            .collect();
        let remotas: Option<Vec<String>> = confkey
            .iter()
            .map(|n| nombres.get(&(desq.clone(), dtab.clone(), *n)).cloned())
            .collect();
        if let (Some(columnas), Some(destino_columnas)) = (locales, remotas) {
            m.entry((esq, tab)).or_default().push(Foranea {
                columnas,
                destino: format!("{desq}.{dtab}"),
                destino_columnas,
            });
        }
    }
    Ok(m)
}

// ── Los verbos ───────────────────────────────────────────────────────────────

/// **`check`**: ¿responde esta fuente? Paso a paso, y el motivo del fallo va **literal** (es lo único accionable): abre (red, TLS,
/// credencial, versión), lista lo que la URL deja ver y, si hay algo, **lee una fila de una tabla** (el permiso de lectura).
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = (|| -> Result<(), Fallo> {
        let mut s = Cockroach::conectar(&url)?;
        let todos = objetos(&mut s.cliente)?;
        if let Some(o) = todos.iter().find(|o| o.clase == "table") {
            s.cliente
                .query(
                    &format!("SELECT 1 FROM {} LIMIT 1", citado(&o.completo())),
                    &[],
                )
                .map_err(|e| fallo_de(&e))?;
        }
        Ok(())
    })();
    Ok(match hecho {
        Ok(()) => ore_driver::comprobacion(true, None),
        Err(f) => ore_driver::comprobacion(false, Some(&texto(f, &url))),
    })
}

/// **`catalogo`**: cada tabla y vista que el usuario puede leer, con los tipos del catálogo del servidor (no se infiere nada), su clave primaria,
/// sus claves únicas y foráneas, sus comentarios y los valores de sus `ENUM`. Las columnas ocultas no salen.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err(
            "no llegó nada por stdin. `catalogo` espera la URL de CockroachDB por ahí".into(),
        );
    }
    let mut s = abrir(url)?;
    let c = &mut s.cliente;
    let objs = objetos(c).map_err(|f| texto(f, url))?;
    let cols = columnas_del_servidor(c).map_err(|f| texto(f, url))?;
    let valores = enums(c);
    let filas_k = filas_de_claves(c).map_err(|f| texto(f, url))?;
    let ocultas = ocultas(c).map_err(|f| texto(f, url))?;
    let claves = claves(&filas_k, &ocultas);
    let nombres: HashMap<(String, String, i64), String> = cols
        .iter()
        .map(|k| {
            (
                (k.esquema.clone(), k.tabla.clone(), k.attnum),
                k.nombre.clone(),
            )
        })
        .collect();
    let mut foraneas = foraneas(c, &nombres).map_err(|f| texto(f, url))?;
    let filas = estimaciones(c);
    let mut por_tabla: BTreeMap<(String, String), Vec<ColumnaCrdb>> = BTreeMap::new();
    for k in cols {
        por_tabla
            .entry((k.esquema.clone(), k.tabla.clone()))
            .or_default()
            .push(k);
    }
    let mut tablas = Vec::new();
    for o in &objs {
        let clave = (o.esquema.clone(), o.nombre.clone());
        let cs = por_tabla.remove(&clave).unwrap_or_default();
        let (pk, unicas) = claves.get(&clave).cloned().unwrap_or_default();
        for a in avisos(o, &cs) {
            eprintln!("ore-read-cockroach: aviso · `{}`: {a}", o.completo());
        }
        let mut t = tabla_de(o, &cs, &valores);
        t.clave = pk;
        t.unicas = unicas;
        t.foraneas = foraneas.remove(&clave).unwrap_or_default();
        t.filas_estimadas = filas.get(&o.oid).copied();
        tablas.push(t);
    }
    Ok(escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
        objetos: Vec::new(),
    }))
}

/// Lo que quien cataloga debe saber de esta tabla y el catálogo no puede llevar.
fn avisos(o: &Objeto, cs: &[ColumnaCrdb]) -> Vec<String> {
    let mut a = Vec::new();
    let texto: Vec<&str> = cs
        .iter()
        .filter(|k| {
            traducir(&k.formato, &k.familia, k.base.as_deref())
                .is_none_or(|t| !crate::tipos::exacto(&t) && !t.starts_with("list<"))
        })
        .map(|k| k.nombre.as_str())
        .collect();
    if !texto.is_empty() {
        a.push(format!(
            "se leen como su texto (no hay un tipo exacto en OOS): {}",
            texto.join(", ")
        ));
    }
    if o.clase != "table" {
        a.push("es una vista: no tiene clave, y su huella (`cambio`) es la de su resultado".into());
    }
    a
}

/// Una tabla o vista de CockroachDB, como la tabla del catálogo.
fn tabla_de(o: &Objeto, cs: &[ColumnaCrdb], valores: &HashMap<i64, Vec<String>>) -> Tabla {
    let columnas = cs
        .iter()
        .map(|k| Columna {
            nombre: k.nombre.clone(),
            // `None`: este conector no supo traducirlo (no es un hueco a rellenar).
            tipo: traducir(&k.formato, &k.familia, k.base.as_deref()),
            // El tipo de CockroachDB tal cual (`numeric(12,2)`): con él, el motor sabe lo que el escalar no lleva.
            origen: Some(k.formato.clone()),
            obligatoria: k.obligatoria,
            valores: valores.get(&k.tipo_oid).cloned().unwrap_or_default(),
            descripcion: k.comentario.clone(),
        })
        .collect();
    Tabla {
        nombre: o.completo(),
        columnas,
        clase: o.clase.clone(),
        lee: Some(lee()),
        cambia: Some(cambios()),
        ..Tabla::default()
    }
}

/// La cara `I`: lo que la inducción declara que se puede empujar, y el recorrido completo.
fn lee() -> Json {
    Json::obj([
        (
            "predicatePushdown",
            Json::Arr(
                ore_driver::EMPUJE_INDUCIDO
                    .iter()
                    .map(|o| Json::s(*o))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s("cheap")),
    ])
}

/// La cara `D`. **No se declara lo que no se hace**: CockroachDB tiene changefeeds y una columna `crdb_internal_mvcc_timestamp`, pero la lectura
/// todavía no recorta por ellos (ADR 0080 §5, paso 7). La huella por filas del verbo `cambio` es otra cosa.
fn cambios() -> Json {
    Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))])
}

/// **`explorar`**: los esquemas con tablas o vistas que el usuario puede leer, en la base de la URL.
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let mut s = abrir(&url)?;
    let objs = objetos(&mut s.cliente).map_err(|f| texto(f, &url))?;
    let base: String = s
        .cliente
        .query_one("SELECT current_database()::text", &[])
        .map(|f| f.get(0))
        .map_err(|e| texto(fallo_de(&e), &url))?;
    let mut por_esquema: BTreeMap<&str, usize> = BTreeMap::new();
    for o in &objs {
        *por_esquema.entry(o.esquema.as_str()).or_default() += 1;
    }
    let contiene: Vec<Json> = por_esquema
        .iter()
        .map(|(e, n)| Json::obj([("nombre", Json::s(*e)), ("objetos", Json::Int(*n as i64))]))
        .collect();
    let nota = format!(
        "la base `{base}`: el catálogo recorre todos sus esquemas, menos los del servidor, y solo lo que el usuario puede leer"
    );
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
}

/// **`testigo`**: hasta dónde está el origen. **`none`**, que es una respuesta cierta (ADR 0016): CockroachDB sí puede fechar una lectura con un
/// instante (`cluster_logical_timestamp()` y `AS OF SYSTEM TIME`), pero la lectura todavía no acepta un rango por él, y declarar un `snapshot`
/// que no se puede honrar sería afirmar lo que no se cumple. Es candidato del paso 7.
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    // Se abre para decir que el origen responde antes de decir que no sabe fecharse.
    abrir(&url)?;
    Ok(ore_driver::testigo("none", None))
}

/// `(esquema, tabla)` de lo que pide `objeto`: sin esquema, el de la sesión.
fn partes_del_objeto(c: &mut postgres::Client, objeto: &str) -> Result<(String, String), Fallo> {
    let p: Vec<&str> = objeto.split('.').collect();
    match p[..] {
        [t] if !t.is_empty() => {
            let esq: String = c
                .query_one("SELECT current_schema()::text", &[])
                .map(|f| f.get(0))
                .map_err(|e| fallo_de(&e))?;
            Ok((esq, t.to_string()))
        }
        [e, t] if !e.is_empty() && !t.is_empty() => Ok((e.to_string(), t.to_string())),
        _ => Err(Fallo::new(
            Codigo::Objeto,
            format!("`{objeto}` no es `tabla` ni `esquema.tabla`"),
        )),
    }
}

/// **`cambio`** (ADR 0063 S3): ¿cambió ESTA tabla desde la última copia? CockroachDB no tiene un contador de cambios por tabla que se lea sin
/// recorrerla, así que la huella es la de **sus filas**: cuántas son y la suma del `fnv64` del texto de cada una, en una sola sentencia (un
/// instante). Cualquier `INSERT`, `UPDATE` o `DELETE` la cambia (medido: también en una vista). **Igual quiere decir que no cambió**; el coste es
/// una lectura entera, por eso tiene un tope de tiempo ([`HUELLA_MS`]): una tabla que no cabe en él da `respalda: false`, y la copia no se fía.
pub fn cambio(entrada: &str) -> Result<String, String> {
    cambio_con(entrada, HUELLA_MS)
}

/// [`cambio`] con el tope de la huella a elegir.
pub fn cambio_con(entrada: &str, limite_ms: u64) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let mut s = abrir(&url)?;
    let c = &mut s.cliente;
    let (esq, tab) = partes_del_objeto(c, &objeto).map_err(|f| texto(f, &url))?;
    let objs = objetos(c).map_err(|f| texto(f, &url))?;
    let Some(o) = objs.iter().find(|o| o.esquema == esq && o.nombre == tab) else {
        return Err(format!(
            "`{esq}.{tab}` no existe, o el usuario no puede leerla"
        ));
    };
    // Las columnas de ahora, para compararlas con la `Table` declarada (S5a).
    let columnas: Vec<(String, Option<String>)> = columnas_del_servidor(c)
        .map_err(|f| texto(f, &url))?
        .into_iter()
        .filter(|k| k.esquema == esq && k.tabla == tab)
        .map(|k| {
            (
                k.nombre.clone(),
                traducir(&k.formato, &k.familia, k.base.as_deref()),
            )
        })
        .collect();
    match huella_de_filas(c, &o.completo(), limite_ms) {
        Ok(h) => Ok(ore_driver::huella_con_columnas(
            "hash-de-filas",
            Some(&h),
            true,
            &columnas,
        )),
        Err(f) if f.codigo == Codigo::Tiempo => {
            eprintln!(
                "ore-read-cockroach: aviso · `{}` no se recorre en {limite_ms} ms: sin huella que respalde (el incremental por columna o un changefeed \
                 la sustituyen)",
                o.completo()
            );
            Ok(ore_driver::huella_con_columnas(
                "sin-contadores",
                None,
                false,
                &columnas,
            ))
        }
        Err(f) => Err(texto(f, &url)),
    }
}

/// `"<suma de fnv64>:<filas>"` de la tabla, en una transacción de solo lectura con un tope de tiempo.
fn huella_de_filas(
    c: &mut postgres::Client,
    objeto: &str,
    limite_ms: u64,
) -> Result<String, Fallo> {
    c.batch_execute("BEGIN READ ONLY")
        .map_err(|e| fallo_de(&e))?;
    let hecho = c
        .batch_execute(&format!("SET LOCAL statement_timeout = {limite_ms}"))
        .and_then(|()| {
            c.query_one(
                &format!(
                    "SELECT count(*)::text, coalesce(sum(fnv64(t::STRING)), 0)::text FROM {} AS t",
                    citado(objeto)
                ),
                &[],
            )
            .map(|f| format!("{}:{}", f.get::<_, String>(1), f.get::<_, String>(0)))
        })
        .map_err(|e| fallo_de(&e));
    match hecho {
        Ok(h) => {
            c.batch_execute("COMMIT").map_err(|e| fallo_de(&e))?;
            Ok(h)
        }
        Err(f) => {
            let _ = c.batch_execute("ROLLBACK");
            Err(f)
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn objeto(clase: &str) -> Objeto {
        Objeto {
            esquema: "public".into(),
            nombre: "t".into(),
            clase: clase.into(),
            comentario: None,
            oid: 7,
        }
    }

    fn col(n: &str, formato: &str, familia: &str, attnum: i64, obligatoria: bool) -> ColumnaCrdb {
        ColumnaCrdb {
            esquema: "public".into(),
            tabla: "t".into(),
            attnum,
            nombre: n.into(),
            formato: formato.into(),
            familia: familia.into(),
            base: None,
            obligatoria,
            comentario: None,
            tipo_oid: 100 + attnum,
        }
    }

    fn fila(t: &str, nombre: &str, tipo: &str, c: &str, pos: i64) -> FilaClave {
        (
            "public".into(),
            t.into(),
            nombre.into(),
            tipo.into(),
            c.into(),
            pos,
        )
    }

    #[test]
    fn la_tabla_del_catalogo_lleva_tipos_obligatoriedad_valores_y_comentarios() {
        let mut cs = vec![
            col("id", "bigint", "b", 1, true),
            col("importe", "numeric(12,2)", "b", 2, false),
            col("estado", "estado", "e", 3, false),
            col("datos", "jsonb", "b", 4, false),
        ];
        cs[1].comentario = Some("Importe con IVA".into());
        let mut v = HashMap::new();
        v.insert(103, vec!["b".to_string(), "a".to_string()]);
        let t = tabla_de(&objeto("table"), &cs, &v);
        assert_eq!(t.nombre, "public.t");
        assert_eq!(t.clase, "table");
        let tipos: Vec<_> = t.columnas.iter().map(|c| c.tipo.clone()).collect();
        assert_eq!(
            tipos,
            vec![
                Some("Integer".into()),
                Some("Decimal<12, 2>".into()),
                Some("String".into()),
                Some("Opaque".into())
            ]
        );
        assert_eq!(t.columnas[1].origen.as_deref(), Some("numeric(12,2)"));
        assert_eq!(
            t.columnas[1].descripcion.as_deref(),
            Some("Importe con IVA")
        );
        assert!(t.columnas[0].obligatoria && !t.columnas[1].obligatoria);
        assert_eq!(
            t.columnas[2].valores,
            vec!["b", "a"],
            "en el orden de declaración, no el alfabético"
        );
        assert!(t.columnas[0].valores.is_empty());
    }

    #[test]
    fn las_vistas_se_distinguen_de_las_tablas() {
        assert_eq!(
            tabla_de(&objeto("view"), &[], &HashMap::new()).clase,
            "view"
        );
        assert_eq!(
            tabla_de(&objeto("materializedView"), &[], &HashMap::new()).clase,
            "materializedView"
        );
        assert_eq!(clase_de("r"), "table");
        assert_eq!(clase_de("p"), "table");
        assert_eq!(clase_de("v"), "view");
        assert_eq!(clase_de("m"), "materializedView");
    }

    #[test]
    fn las_claves_saltan_la_columna_oculta_y_no_repiten_la_primaria() {
        let ocultas: HashSet<_> = [(
            "public".to_string(),
            "t".to_string(),
            "crdb_internal_h_shard_16".to_string(),
        )]
        .into();
        let filas = vec![
            fila("t", "t_pkey", "PRIMARY KEY", "a", 1),
            fila("t", "t_pkey", "PRIMARY KEY", "b", 2),
            fila("t", "t_c_key", "UNIQUE", "c", 1),
            fila("t", "u_hash", "UNIQUE", "crdb_internal_h_shard_16", 1),
            fila("t", "u_hash", "UNIQUE", "h", 2),
            // Una única que repite la primaria no se vuelve a decir.
            fila("t", "t_a_b_key", "UNIQUE", "a", 1),
            fila("t", "t_a_b_key", "UNIQUE", "b", 2),
        ];
        let k = claves(&filas, &ocultas);
        let (pk, unicas) = &k[&("public".to_string(), "t".to_string())];
        assert_eq!(pk, &vec!["a".to_string(), "b".to_string()]);
        assert_eq!(unicas, &vec![vec!["c".to_string()], vec!["h".to_string()]]);
    }

    #[test]
    fn una_clave_hash_sharded_sigue_teniendo_su_columna_real() {
        let ocultas: HashSet<_> = [(
            "public".to_string(),
            "t".to_string(),
            "crdb_internal_id_shard_16".to_string(),
        )]
        .into();
        let filas = vec![
            fila("t", "t_pkey", "PRIMARY KEY", "crdb_internal_id_shard_16", 1),
            fila("t", "t_pkey", "PRIMARY KEY", "id", 2),
        ];
        let k = claves(&filas, &ocultas);
        assert_eq!(
            k[&("public".to_string(), "t".to_string())].0,
            vec!["id".to_string()]
        );
    }

    #[test]
    fn los_avisos_dicen_lo_que_se_lee_como_texto_y_que_una_vista_no_tiene_clave() {
        let cs = vec![
            col("id", "bigint", "b", 1, true),
            col("datos", "jsonb", "b", 2, false),
            col("raro", "tsquery", "b", 3, false),
        ];
        let a = avisos(&objeto("view"), &cs).join("\n");
        assert!(
            a.contains("datos") && a.contains("raro") && !a.contains("id,"),
            "{a}"
        );
        assert!(a.contains("es una vista"), "{a}");
        let a = avisos(&objeto("table"), &[col("id", "bigint", "b", 1, true)]).join("\n");
        assert!(a.is_empty(), "{a}");
    }

    #[test]
    fn el_origen_no_declara_cambios_que_no_lee() {
        assert_eq!(cambios().jcs(), r#"{"mode":"none","witness":"none"}"#);
        assert!(lee().jcs().contains("\"fullScan\":\"cheap\""));
    }
}
