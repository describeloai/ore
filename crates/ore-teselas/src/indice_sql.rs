//! **El índice de la capa en SQL** (ADR 0072 I2·2): lo que DuckDB ejecuta en el Job de la capa para
//! construir un índice desde una tabla del lago. Lo genera Rust con los datos de la receta
//! ([`crate::indice::Receta`]), y son cuatro sentencias, en este orden:
//!
//! 1. [`sql_lectura`]: la fuente como vista. Las filas vivas del snapshot (sin las que tienen un
//!    borrado por posición), con su identidad `(fichero, fila)` y la geometría ya en 3857.
//! 2. [`SQL_EXTENSION_Y_CUENTA`]: en una pasada, la extensión (para congelar la rejilla) y la
//!    cuenta de la fuente (el lado «fuente» de I-1).
//! 3. [`sql_indice`]: el índice. Caja, trozos de lo grande y hueco, clave Hilbert sobre la rejilla
//!    congelada, orden y Parquet con los ids de campo de la tabla Iceberg que los registrará.
//! 4. [`sql_cuenta_indice`]: la cuenta de lo escrito (sólo el trozo 0).
//!
//! DuckDB hace lo que mejor sabe (leer, transformar, ordenar fuera de memoria: 240 M filas en 112 s,
//! I0·d), y ORE lo que es suyo: la identidad, la receta, registrar y comprobar.

use crate::indice::{
    COL_CAJA, COL_CLAVE, COL_FICHERO, COL_FILA, COL_GEOM, COL_ID, COL_TROZO, Receta, Rejilla,
};

/// La mitad del lado de Web Mercator: el origen de la rejilla de teselas en 3857.
pub const MERCATOR: f64 = 20_037_508.342_789_244;

/// Un fichero de datos vivo de la fuente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FicheroFuente {
    /// El ordinal en la tabla de ficheros del índice: la mitad de la identidad.
    pub ordinal: u32,
    /// Por donde lo lee DuckDB (`https://storage.googleapis.com/…`, o una ruta local).
    pub ruta: String,
    /// Como lo nombra Iceberg (`gs://…`): lo que dice un borrado por posición.
    pub uri: String,
}

/// **Lo que se lee de la fuente** para construir su índice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lectura {
    pub ficheros: Vec<FicheroFuente>,
    /// Los ficheros de borrados por posición vivos (por donde los lee DuckDB).
    pub borrados: Vec<String>,
    /// La columna geo de la fuente y su CRS.
    pub geometria: String,
    /// `true`: DuckDB la lee como `BLOB` con WKB (un Parquet sin marcas geo); `false`: ya como
    /// `GEOMETRY` (el tipo lógico de Parquet que escribe ORE, o GeoParquet). DuckDB lo decide al
    /// enlazar, así que se pregunta antes ([`sql_tipo_de_la_geometria`]) y no hay una lectura que
    /// valga para las dos.
    pub en_wkb: bool,
    pub crs: String,
    /// Las columnas que la capa usa (estilo, etiquetas, filtro de quien mira, tiempo).
    pub columnas: Vec<String>,
    /// El suelo de la capa: la rejilla por la que se parte lo grande.
    pub suelo: u32,
}

/// Un texto entre comillas simples para SQL.
fn texto(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Un identificador entre comillas dobles para SQL.
fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Un doble como literal de SQL que vuelve al mismo `f64`.
fn doble(v: f64) -> String {
    format!("{v:?}")
}

fn lista(rutas: &[String]) -> String {
    format!(
        "[{}]",
        rutas
            .iter()
            .map(|r| texto(r))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// **1 · La fuente como vista** (`_ore_fuente`): una fila por fila viva del snapshot, con
/// `_id_u` (el id `u64` de la posición: `ordinal << 40 | fila`), `_fichero`, `_fila`, `_g` (la
/// geometría en 3857, nula si la fuente la trae nula) y las columnas de la capa.
pub fn sql_lectura(l: &Lectura) -> Result<String, String> {
    if l.ficheros.is_empty() {
        return Err("una fuente sin ficheros no tiene índice".into());
    }
    if l.ficheros.iter().any(|f| f.ordinal >= 1 << 24) {
        return Err("más de 2^24 ficheros: no caben en el id de una posición".into());
    }
    let valores = l
        .ficheros
        .iter()
        .map(|f| format!("({}, {}, {})", f.ordinal, texto(&f.ruta), texto(&f.uri)))
        .collect::<Vec<_>>()
        .join(",\n  ");
    let rutas: Vec<String> = l.ficheros.iter().map(|f| f.ruta.clone()).collect();
    let g = match l.en_wkb {
        true => format!("ST_GeomFromWKB(s.{})", ident(&l.geometria)),
        false => format!("s.{}", ident(&l.geometria)),
    };
    let en_3857 = match l.crs.as_str() {
        "EPSG:3857" => g,
        crs => format!("ST_Transform({g}, {}, 'EPSG:3857')", texto(crs)),
    };
    let columnas: String = l
        .columnas
        .iter()
        .map(|c| format!(",\n  s.{}", ident(c)))
        .collect();
    let borrados = match l.borrados.as_slice() {
        [] => String::new(),
        b => format!(
            "\nWHERE NOT EXISTS (\n  SELECT 1 FROM read_parquet({}) d\n  WHERE d.file_path = f.uri AND d.pos = s.file_row_number)",
            lista(b)
        ),
    };
    Ok(format!(
        "SET geometry_always_xy = true;
SET preserve_insertion_order = true;
CREATE OR REPLACE TEMP TABLE _ore_ficheros(ordinal INTEGER, ruta VARCHAR, uri VARCHAR);
INSERT INTO _ore_ficheros VALUES
  {valores};
CREATE OR REPLACE TEMP VIEW _ore_fuente AS
SELECT
  ((f.ordinal::UBIGINT << 40) | s.file_row_number::UBIGINT) AS _id_u,
  f.ordinal AS _fichero,
  s.file_row_number AS _fila,
  CASE WHEN s.{geo} IS NULL THEN NULL ELSE {en_3857} END AS _g{columnas}
FROM read_parquet({rutas}, filename = true, file_row_number = true) s
JOIN _ore_ficheros f ON f.ruta = s.filename{borrados};
",
        geo = ident(&l.geometria),
        rutas = lista(&rutas),
    ))
}

/// **0 · Cómo lee DuckDB la geometría de la fuente** (`tipo`: `BLOB` o `GEOMETRY`), en su primer
/// fichero: todos los de una tabla tienen el mismo esquema.
pub fn sql_tipo_de_la_geometria(l: &Lectura) -> Result<String, String> {
    let f = l
        .ficheros
        .first()
        .ok_or("una fuente sin ficheros no tiene índice")?;
    Ok(format!(
        "SELECT typeof({}) AS tipo FROM read_parquet({}) LIMIT 1;
",
        ident(&l.geometria),
        texto(&f.ruta)
    ))
}

/// **2 · La extensión y la cuenta de la fuente**, en una pasada: `xmin, ymin, xmax, ymax` (de lo
/// finito: el polo en 3857 es infinito), `filas` y `suma_ids` (la suma `u64` con desbordamiento).
pub const SQL_EXTENSION_Y_CUENTA: &str = "SELECT
  min(ST_XMin(_g)) FILTER (WHERE isfinite(ST_XMin(_g))) AS xmin,
  min(ST_YMin(_g)) FILTER (WHERE isfinite(ST_YMin(_g))) AS ymin,
  max(ST_XMax(_g)) FILTER (WHERE isfinite(ST_XMax(_g))) AS xmax,
  max(ST_YMax(_g)) FILTER (WHERE isfinite(ST_YMax(_g))) AS ymax,
  count(*) AS filas,
  (coalesce(sum(_id_u::HUGEINT), 0) % 18446744073709551616)::UBIGINT AS suma_ids
FROM _ore_fuente;
";

/// **La clave de orden** de una geometría `g` (una expresión SQL): la Hilbert de DuckDB (16 bits por
/// eje) del centro de su caja sobre `rejilla`, con lo de fuera pegado al borde; nula si `g` es nula
/// o vacía (`greatest` de DuckDB ignora un nulo, así que se dice aparte).
pub fn clave_sql(rejilla: &Rejilla, g: &str) -> String {
    let (x0, y0, x1, y1) = (
        doble(rejilla.x0),
        doble(rejilla.y0),
        doble(rejilla.x1),
        doble(rejilla.y1),
    );
    format!(
        "CASE WHEN {g} IS NULL OR ST_IsEmpty({g}) THEN NULL ELSE ST_Hilbert(
    ST_Point(greatest(least((ST_XMin({g}) + ST_XMax({g})) / 2, {x1}), {x0}),
             greatest(least((ST_YMin({g}) + ST_YMax({g})) / 2, {y1}), {y0})),
    ST_MakeBox2D(ST_Point({x0}, {y0}), ST_Point({x1}, {y1}))) END"
    )
}

/// **La clave de una fila del índice desde su caja** (`geom_bbox`, I4·3): la misma que
/// [`clave_sql`] da a su geometría —el centro de la caja, pegado a la rejilla—, sin leer el WKB. Lo
/// nulo o vacío no tiene caja, y su clave es nula, como allí.
pub fn clave_sql_de_caja(rejilla: &Rejilla, caja: &str) -> String {
    let (x0, y0, x1, y1) = (
        doble(rejilla.x0),
        doble(rejilla.y0),
        doble(rejilla.x1),
        doble(rejilla.y1),
    );
    format!(
        "CASE WHEN {caja} IS NULL THEN NULL ELSE ST_Hilbert(
    ST_Point(greatest(least(({caja}.xmin + {caja}.xmax) / 2, {x1}), {x0}),
             greatest(least(({caja}.ymin + {caja}.ymax) / 2, {y1}), {y0})),
    ST_MakeBox2D(ST_Point({x0}, {y0}), ST_Point({x1}, {y1}))) END"
    )
}

/// **Un tramo de la clave** `[desde, hasta)`: el índice de una fuente que no cabe en el disco de un
/// Job se construye tramo a tramo (I2·4), cada uno en su fichero, que se sube y se borra antes del
/// siguiente. Sin `desde`, desde el principio; sin `hasta`, hasta el final, y con lo nulo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tramo {
    pub desde: Option<u64>,
    pub hasta: Option<u64>,
}

impl Tramo {
    /// Los tramos que dejan unos cortes (ordenados, sin repetir): `n + 1` tramos para `n` cortes.
    pub fn de_cortes(cortes: &[u64]) -> Vec<Tramo> {
        let mut c: Vec<u64> = cortes.to_vec();
        c.sort_unstable();
        c.dedup();
        let mut out = Vec::with_capacity(c.len() + 1);
        let mut desde = None;
        for k in c {
            out.push(Tramo {
                desde,
                hasta: Some(k),
            });
            desde = Some(k);
        }
        out.push(Tramo { desde, hasta: None });
        out
    }

    fn filtro(&self, k: &str) -> String {
        match (self.desde, self.hasta) {
            (None, None) => "true".into(),
            (Some(a), None) => format!("({k} >= {a} OR {k} IS NULL)"),
            (None, Some(b)) => format!("{k} < {b}"),
            (Some(a), Some(b)) => format!("{k} >= {a} AND {k} < {b}"),
        }
    }
}

/// **2b · Los cortes de la clave** para `tramos` tramos de filas parecidas: los cuantiles
/// aproximados (t-digest, memoria fija) de la clave del centro de cada fila de la fuente.
pub fn sql_cortes(rejilla: &Rejilla, tramos: usize) -> String {
    let q: Vec<String> = (1..tramos)
        .map(|i| format!("{:?}", i as f64 / tramos as f64))
        .collect();
    format!(
        "SELECT approx_quantile({}, [{}]) AS cortes FROM _ore_fuente WHERE _g IS NOT NULL;
",
        clave_sql(rejilla, "_g"),
        q.join(", ")
    )
}

/// **3 · El índice, o un tramo suyo**: lo escribe en el fichero `fichero` **en un solo
/// Parquet**. Medido en I2·3: con `FILE_SIZE_BYTES` DuckDB reparte en ficheros
/// escribiendo en paralelo e intercala tramos ordenados (21 saltos atrás en 3 M filas, en múltiplos
/// de su vector de 2 048); sin él, el orden sale entero. Va con
/// las columnas de [`crate::indice::esquema`] en su orden y los ids de campo `ids` (los de
/// `Lago::ids_de_tabla_nueva`). Lo grande y hueco de la receta se parte por las celdas de las
/// teselas del suelo; el orden es la clave Hilbert de DuckDB (16 bits por eje) del centro de la
/// caja sobre `rejilla`, con lo de fuera pegado al borde (y lo nulo o vacío al final: `greatest` de
/// DuckDB ignora un nulo, así que se dice aparte), y luego id y trozo para que sea determinista.
/// Lo leen dos ramas (enteras y trozos) y DuckDB lo guarda en el disco (`common_subplan`, aun con
/// `NOT MATERIALIZED`): por eso el tramo se filtra **antes**, por la clave de la fila entera, que es la
/// de su único trozo; lo que se parte entra siempre (sus trozos pueden caer en otro tramo) y el filtro
/// final decide por la clave de cada trozo. Sin ello, los 600 M guardaban la fuente entera, 50 GiB,
/// con tramos de 25 M (I2·4).
pub fn sql_indice(
    l: &Lectura,
    receta: &Receta,
    rejilla: &Rejilla,
    ids: &[(String, i32)],
    fichero: &str,
    tramo: &Tramo,
) -> Result<String, String> {
    if receta.bits != 16 {
        return Err(format!(
            "la receta `{}` pide {} bits por eje y `ST_Hilbert` da 16",
            receta.nombre, receta.bits
        ));
    }
    let celda = 2.0 * MERCATOR / f64::from(1u32 << l.suelo);
    let umbral = celda * f64::from(receta.trozo_celdas);
    let hueco = f64::from(receta.hueco_milesimas) / 1000.0;
    let c = doble(celda);
    let o = doble(-MERCATOR);
    let columnas: String = l
        .columnas
        .iter()
        .map(|c| format!(", {}", ident(c)))
        .collect();
    let caja_de = |g: &str| {
        format!(
            "{{'xmin': ST_XMin({g}), 'ymin': ST_YMin({g}), 'xmax': ST_XMax({g}), 'ymax': ST_YMax({g})}}"
        )
    };
    let celda_de = |ix: &str, iy: &str| {
        format!(
            "ST_MakeEnvelope({o} + {ix} * {c}, {o} + {iy} * {c}, {o} + ({ix} + 1) * {c}, {o} + ({iy} + 1) * {c})"
        )
    };
    Ok(format!(
        "COPY (
WITH base AS (
  SELECT *, ST_XMin(_g) AS _x0, ST_YMin(_g) AS _y0, ST_XMax(_g) AS _x1, ST_YMax(_g) AS _y1,
    ST_GeometryType(_g)::VARCHAR AS _tipo
  FROM _ore_fuente
), partir AS (
  SELECT *, coalesce(
    NOT ST_IsEmpty(_g)
    AND (_x1 - _x0 > {umbral} OR _y1 - _y0 > {umbral})
    AND (_tipo IN ('LINESTRING', 'MULTILINESTRING')
      OR (_tipo IN ('POLYGON', 'MULTIPOLYGON') AND ST_Area(_g) < {hueco} * (_x1 - _x0) * (_y1 - _y0))),
    false) AS _parte
  FROM base
), del_tramo AS (
  SELECT * FROM partir WHERE _parte OR {filtro_fila}
), enteras AS (
  SELECT _id_u, _fichero, _fila, 0 AS _trozo, _g{columnas} FROM del_tramo WHERE NOT _parte
), celdas AS (
  SELECT p._id_u, p._fichero, p._fila, rx.ix, ry.iy,
    ST_Intersection(p._g, {env}) AS _g{columnas_p}
  FROM del_tramo p,
    range(floor((p._x0 - {o}) / {c})::BIGINT, floor((p._x1 - {o}) / {c})::BIGINT + 1) rx(ix),
    range(floor((p._y0 - {o}) / {c})::BIGINT, floor((p._y1 - {o}) / {c})::BIGINT + 1) ry(iy)
  WHERE p._parte AND ST_Intersects(p._g, {env})
), trozos AS (
  SELECT _id_u, _fichero, _fila,
    (row_number() OVER (PARTITION BY _id_u ORDER BY ix, iy) - 1) AS _trozo, _g{columnas}
  FROM celdas WHERE NOT ST_IsEmpty(_g)
), todo AS (
  SELECT * FROM enteras UNION ALL BY NAME SELECT * FROM trozos
), ordenado AS (
  SELECT *, {clave} AS _k FROM todo
)
SELECT
  (CASE WHEN _id_u >= 9223372036854775808 THEN _id_u::HUGEINT - 18446744073709551616
        ELSE _id_u::HUGEINT END)::BIGINT AS {col_id},
  _fichero::INTEGER AS {col_fichero},
  _fila::BIGINT AS {col_fila},
  NULL::VARCHAR AS {col_clave},
  _trozo::INTEGER AS {col_trozo},
  ST_AsWKB(_g) AS {col_geom},
  CASE WHEN _g IS NULL OR ST_IsEmpty(_g) THEN NULL ELSE {caja} END AS {col_caja}{columnas}
FROM ordenado
WHERE {filtro}
ORDER BY _k NULLS LAST, _id_u, _trozo
) TO {salida} (FORMAT parquet, COMPRESSION zstd, ROW_GROUP_SIZE {grupo}, FIELD_IDS {ids});
",
        env = celda_de("rx.ix", "ry.iy"),
        columnas_p = l
            .columnas
            .iter()
            .map(|c| format!(", p.{}", ident(c)))
            .collect::<String>(),
        caja = caja_de("_g"),
        col_id = ident(COL_ID),
        col_fichero = ident(COL_FICHERO),
        col_fila = ident(COL_FILA),
        col_clave = ident(COL_CLAVE),
        col_trozo = ident(COL_TROZO),
        col_geom = ident(COL_GEOM),
        col_caja = ident(COL_CAJA),
        clave = clave_sql(rejilla, "_g"),
        filtro = tramo.filtro("_k"),
        filtro_fila = tramo.filtro(&format!("({})", clave_sql(rejilla, "_g"))),
        salida = texto(fichero),
        grupo = receta.grupo_de_filas,
        ids = field_ids(ids)?,
    ))
}

/// `FIELD_IDS` de DuckDB: `{'col': id, 'estructura': {'__duckdb_field_id': id, 'hijo': id}}`.
fn field_ids(ids: &[(String, i32)]) -> Result<String, String> {
    fn nivel(ids: &[(String, i32)], padre: &str) -> Vec<String> {
        ids.iter()
            .filter(|(r, _)| match padre {
                "" => !r.contains('.'),
                p => r
                    .strip_prefix(p)
                    .and_then(|r| r.strip_prefix('.'))
                    .is_some_and(|r| !r.contains('.')),
            })
            .map(|(r, id)| {
                let nombre = r.rsplit('.').next().unwrap_or(r);
                let hijos = nivel(ids, r);
                if hijos.is_empty() {
                    format!("{}: {id}", texto(nombre))
                } else {
                    format!(
                        "{}: {{'__duckdb_field_id': {id}, {}}}",
                        texto(nombre),
                        hijos.join(", ")
                    )
                }
            })
            .collect()
    }
    let top = nivel(ids, "");
    if top.is_empty() {
        return Err("sin ids de campo no se escribe un índice: Iceberg lee por id".into());
    }
    Ok(format!("{{{}}}", top.join(", ")))
}

/// **4 · La cuenta del índice escrito**: sólo el trozo 0 (una línea partida en cinco es una fila
/// de la fuente), con el id de vuelta a `u64`.
pub fn sql_cuenta_indice(ficheros: &[String]) -> String {
    format!(
        "SELECT count(*) AS filas,
  (coalesce(sum({id}::HUGEINT + CASE WHEN {id} < 0 THEN 18446744073709551616 ELSE 0 END), 0)
    % 18446744073709551616)::UBIGINT AS suma_ids
FROM read_parquet({glob})
WHERE {trozo} = 0;
",
        id = ident(COL_ID),
        trozo = ident(COL_TROZO),
        glob = lista(ficheros),
    )
}

/// Los ids de campo de un fichero de borrados por posición (la spec de Iceberg).
const ID_FILE_PATH: i32 = 2147483546;
const ID_POS: i32 = 2147483545;

/// **Lo que sale del índice al seguir a su tabla** (I4·2): las filas del índice cuya fila de la
/// fuente ya no está —su fichero se retiró (`retirados`, por ordinal), o un borrado por posición
/// nuevo de la fuente la quitó (`borrados_de_la_fuente`, casados por `ficheros`: los ordinales y
/// URIs de los ficheros que ya estaban)—, **todos sus trozos**. Quedan en `_ore_quitar`, marcadas
/// `ya` si un borrado vivo del índice (`borrados_del_indice`) ya las quitaba; las que no, a
/// `salida`, como un fichero de borrados por posición (`file_path` con la URI que la tabla del
/// índice da a su fichero, `pos`). La cuenta, con [`SQL_CUENTA_QUITAR`].
///
/// `indice`: los ficheros vivos del índice, `(uri, por donde los lee DuckDB)`.
pub fn sql_quitar(
    indice: &[(String, String)],
    borrados_del_indice: &[String],
    retirados: &[u32],
    borrados_de_la_fuente: &[String],
    ficheros: &[(u32, String)],
    salida: &str,
) -> String {
    let valores = |filas: Vec<String>| {
        if filas.is_empty() {
            "SELECT NULL, NULL WHERE false".to_string()
        } else {
            format!("VALUES {}", filas.join(", "))
        }
    };
    let rutas: Vec<String> = indice.iter().map(|(_, r)| r.clone()).collect();
    let uris = valores(
        indice
            .iter()
            .map(|(u, r)| format!("({}, {})", texto(r), texto(u)))
            .collect(),
    );
    let ya = if borrados_del_indice.is_empty() {
        "SELECT NULL::VARCHAR AS file_path, NULL::BIGINT AS pos WHERE false".to_string()
    } else {
        format!(
            "SELECT file_path, pos FROM read_parquet({})",
            lista(borrados_del_indice)
        )
    };
    let fuente = if borrados_de_la_fuente.is_empty() {
        "SELECT NULL::INTEGER AS ordinal, NULL::BIGINT AS fila WHERE false".to_string()
    } else {
        format!(
            "SELECT DISTINCT f.ordinal::INTEGER AS ordinal, d.pos AS fila
  FROM read_parquet({}) d
  JOIN ({}) f(ordinal, uri) ON f.uri = d.file_path",
            lista(borrados_de_la_fuente),
            valores(
                ficheros
                    .iter()
                    .map(|(o, u)| format!("({o}, {})", texto(u)))
                    .collect()
            )
        )
    };
    let retirados = if retirados.is_empty() {
        "false".to_string()
    } else {
        format!(
            "i.{} IN ({})",
            ident(COL_FICHERO),
            retirados
                .iter()
                .map(|o| o.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "CREATE OR REPLACE TEMP TABLE _ore_uri_del_indice(ruta VARCHAR, uri VARCHAR);
INSERT INTO _ore_uri_del_indice {uris};
CREATE OR REPLACE TEMP TABLE _ore_borrados_de_la_fuente AS {fuente};
CREATE OR REPLACE TEMP TABLE _ore_quitar AS
WITH ya AS ({ya})
SELECT u.uri AS file_path, i.file_row_number AS pos, i.{id} AS id, i.{fichero} AS fichero,
  i.{trozo} AS trozo,
  EXISTS (SELECT 1 FROM ya WHERE ya.file_path = u.uri AND ya.pos = i.file_row_number) AS ya
FROM read_parquet({rutas}, filename = true, file_row_number = true) i
JOIN _ore_uri_del_indice u ON u.ruta = i.filename
WHERE {retirados}
  OR EXISTS (SELECT 1 FROM _ore_borrados_de_la_fuente b
             WHERE b.ordinal = i.{fichero} AND b.fila = i.{fila});
COPY (SELECT file_path, pos FROM _ore_quitar WHERE NOT ya ORDER BY file_path, pos)
  TO {salida} (FORMAT parquet, COMPRESSION zstd, FIELD_IDS {{file_path: {ID_FILE_PATH}, pos: {ID_POS}}});
",
        id = ident(COL_ID),
        fichero = ident(COL_FICHERO),
        fila = ident(COL_FILA),
        trozo = ident(COL_TROZO),
        rutas = lista(&rutas),
        salida = texto(salida),
    )
}

/// **La cuenta de lo que sale** (tras [`sql_quitar`]), de una fila por fila de la fuente (los
/// trozos 0): las que salen ahora y su suma de ids, las que ya estaban fuera, cuántas posiciones
/// se escriben, y lo que dicen los borrados nuevos de la fuente sobre ficheros que no se retiran
/// (filas y suma de sus ids, `ordinal << 40 | fila`). El Job lo coteja con lo que dice la fuente.
pub fn sql_cuenta_quitar(retirados: &[u32]) -> String {
    let fuera = if retirados.is_empty() {
        "true".to_string()
    } else {
        format!(
            "ordinal NOT IN ({})",
            retirados
                .iter()
                .map(|o| o.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let suma = |e: &str| {
        format!(
            "(coalesce(sum({e}::HUGEINT + CASE WHEN {e} < 0 THEN 18446744073709551616 ELSE 0 END), 0) % 18446744073709551616)::UBIGINT"
        )
    };
    format!(
        "SELECT
  (SELECT count(*) FROM _ore_quitar WHERE trozo = 0 AND NOT ya) AS filas,
  (SELECT {s_quitar} FROM _ore_quitar WHERE trozo = 0 AND NOT ya) AS suma_ids,
  (SELECT count(*) FROM _ore_quitar WHERE trozo = 0 AND ya) AS ya_filas,
  (SELECT {s_ya} FROM _ore_quitar WHERE trozo = 0 AND ya) AS ya_suma_ids,
  (SELECT count(*) FROM _ore_quitar WHERE NOT ya) AS posiciones,
  (SELECT count(*) FROM _ore_borrados_de_la_fuente WHERE {fuera}) AS fuente_filas,
  (SELECT ({s_fuente}) FROM _ore_borrados_de_la_fuente WHERE {fuera}) AS fuente_suma_ids;
",
        s_quitar = suma("id"),
        s_ya = suma("id"),
        s_fuente = "(coalesce(sum(((ordinal::UBIGINT << 40) | fila::UBIGINT)::HUGEINT), 0) % 18446744073709551616)::UBIGINT",
    )
}

/// **Fundir tramos del índice** (I4·3, la compactación por niveles): la vista `_ore_fundir` con lo
/// vivo de unos ficheros del índice —sin lo que sus borrados por posición quitan— y la clave de cada
/// fila desde su caja (`_k`). `ficheros`: `(uri, por donde los lee DuckDB)`; `borrados`: los ficheros
/// de borrados vivos del índice (por donde se leen).
pub fn sql_vista_fundir(
    ficheros: &[(String, String)],
    borrados: &[String],
    rejilla: &Rejilla,
) -> String {
    let rutas: Vec<String> = ficheros.iter().map(|(_, r)| r.clone()).collect();
    let uris = ficheros
        .iter()
        .map(|(u, r)| format!("({}, {})", texto(r), texto(u)))
        .collect::<Vec<_>>()
        .join(", ");
    let ya = if borrados.is_empty() {
        "SELECT NULL::VARCHAR AS file_path, NULL::BIGINT AS pos WHERE false".to_string()
    } else {
        format!(
            "SELECT file_path, pos FROM read_parquet({})",
            lista(borrados)
        )
    };
    format!(
        "CREATE OR REPLACE TEMP TABLE _ore_uri_a_fundir(ruta VARCHAR, uri VARCHAR);
INSERT INTO _ore_uri_a_fundir VALUES {uris};
CREATE OR REPLACE TEMP TABLE _ore_ya_borradas AS {ya};
CREATE OR REPLACE TEMP VIEW _ore_fundir AS
SELECT i.* EXCLUDE (filename, file_row_number), {clave} AS _k
FROM read_parquet({rutas}, filename = true, file_row_number = true) i
JOIN _ore_uri_a_fundir u ON u.ruta = i.filename
WHERE NOT EXISTS (SELECT 1 FROM _ore_ya_borradas b WHERE b.file_path = u.uri AND b.pos = i.file_row_number);
",
        clave = clave_sql_de_caja(rejilla, &format!("i.{}", ident(COL_CAJA))),
        rutas = lista(&rutas),
    )
}

/// La cuenta de lo vivo a fundir (sobre `_ore_fundir`): filas y suma de ids, de los trozos 0.
pub const SQL_CUENTA_FUNDIR: &str = "SELECT count(*) AS filas,
  (coalesce(sum(\"_ore_id\"::HUGEINT + CASE WHEN \"_ore_id\" < 0 THEN 18446744073709551616 ELSE 0 END), 0)
    % 18446744073709551616)::UBIGINT AS suma_ids
FROM _ore_fundir WHERE \"_ore_trozo\" = 0;
";

/// Los cortes de la clave para fundir en `tramos` tramos (sobre `_ore_fundir`).
pub fn sql_cortes_fundir(tramos: usize) -> String {
    let q: Vec<String> = (1..tramos)
        .map(|i| format!("{:?}", i as f64 / tramos as f64))
        .collect();
    format!(
        "SELECT approx_quantile(_k, [{}]) AS cortes FROM _ore_fundir WHERE _k IS NOT NULL;\n",
        q.join(", ")
    )
}

/// **Un tramo de lo fundido**, ordenado como el índice (clave, id, trozo) y con los ids de campo de
/// su tabla, en `fichero`.
pub fn sql_fundir(ids: &[(String, i32)], fichero: &str, tramo: &Tramo) -> Result<String, String> {
    Ok(format!(
        "COPY (SELECT * EXCLUDE (_k) FROM _ore_fundir WHERE {filtro}
  ORDER BY _k NULLS LAST, {id}, {trozo})
TO {salida} (FORMAT parquet, COMPRESSION zstd, ROW_GROUP_SIZE {grupo}, FIELD_IDS {ids});
",
        filtro = tramo.filtro("_k"),
        id = ident(COL_ID),
        trozo = ident(COL_TROZO),
        salida = texto(fichero),
        grupo = crate::indice::I1.grupo_de_filas,
        ids = field_ids(ids)?,
    ))
}

/// **5 · La profundidad de lo escrito** (la vara de H0 e I0·d): cuántos grupos de filas cubren, de
/// media, un punto con datos. Los grupos, con las cajas de sus pies; los puntos, 2 000 centros de
/// caja al azar (con semilla: la misma muestra cada vez). 1 es perfecto; sin orden, el número de
/// grupos.
pub fn sql_profundidad(ficheros: &[String]) -> String {
    let glob = lista(ficheros);
    let c = |campo: &str, f: &str, stat: &str| {
        format!(
            "{f}(try_cast({stat} AS DOUBLE)) FILTER (WHERE path_in_schema = '{COL_CAJA}, {campo}') AS {campo}"
        )
    };
    format!(
        "WITH grupos AS (
  SELECT file_name, row_group_id, {x0}, {y0}, {x1}, {y1}
  FROM parquet_metadata({glob}) GROUP BY ALL
), puntos AS (
  SELECT row_number() OVER () AS n,
    ({caja}.xmin + {caja}.xmax) / 2 AS x, ({caja}.ymin + {caja}.ymax) / 2 AS y
  FROM read_parquet({glob}) WHERE {caja} IS NOT NULL
  USING SAMPLE reservoir(2000 ROWS) REPEATABLE (7)
)
SELECT coalesce(avg(c), 0) AS profundidad, (SELECT count(*) FROM grupos) AS grupos FROM (
  SELECT count(*) AS c FROM puntos JOIN grupos
    ON x BETWEEN grupos.xmin AND grupos.xmax AND y BETWEEN grupos.ymin AND grupos.ymax
  GROUP BY n);
",
        x0 = c("xmin", "min", "stats_min_value"),
        y0 = c("ymin", "min", "stats_min_value"),
        x1 = c("xmax", "max", "stats_max_value"),
        y1 = c("ymax", "max", "stats_max_value"),
        caja = ident(COL_CAJA),
    )
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::indice::{I1, Identidad, Rejilla};

    pub(crate) fn lectura() -> Lectura {
        Lectura {
            ficheros: vec![
                FicheroFuente {
                    ordinal: 0,
                    ruta: "/datos/a.parquet".into(),
                    uri: "gs://cubo/t/viajes/data/a.parquet".into(),
                },
                FicheroFuente {
                    ordinal: 1,
                    ruta: "/datos/b.parquet".into(),
                    uri: "gs://cubo/t/viajes/data/b.parquet".into(),
                },
            ],
            borrados: vec!["/datos/borrados.parquet".into()],
            geometria: "geom".into(),
            en_wkb: true,
            crs: "EPSG:4326".into(),
            columnas: vec!["tarifa".into(), "cuando".into()],
            suelo: 13,
        }
    }

    /// La rejilla de la prueba de DuckDB (`pruebas-de-fuego/geo/i2/`): Nueva York en 3857, con margen.
    pub(crate) fn rejilla() -> Rejilla {
        Rejilla {
            x0: -8_300_000.0,
            y0: 4_900_000.0,
            x1: -8_150_000.0,
            y1: 5_050_000.0,
        }
    }

    fn ids() -> Vec<(String, i32)> {
        let arrow = crate::indice::esquema(
            &I1,
            &[
                arrow_schema::Field::new("tarifa", arrow_schema::DataType::Float64, true),
                arrow_schema::Field::new("cuando", arrow_schema::DataType::Int64, true),
            ],
        )
        .unwrap();
        ore_store::lago::Lago::ids_de_tabla_nueva(&arrow).unwrap()
    }

    #[test]
    fn la_lectura_lleva_identidad_borrados_y_3857() {
        let s = sql_lectura(&lectura()).unwrap();
        assert!(
            s.contains("(0, '/datos/a.parquet', 'gs://cubo/t/viajes/data/a.parquet')"),
            "{s}"
        );
        assert!(s.contains("((f.ordinal::UBIGINT << 40) | s.file_row_number::UBIGINT) AS _id_u"));
        assert!(s.contains("ST_Transform(ST_GeomFromWKB(s.\"geom\"), 'EPSG:4326', 'EPSG:3857')"));
        assert!(s.contains("d.file_path = f.uri AND d.pos = s.file_row_number"));
        // sin borrados, sin el filtro; en 3857, sin transformar; ya `GEOMETRY`, sin leer WKB
        let sin = Lectura {
            borrados: vec![],
            crs: "EPSG:3857".into(),
            en_wkb: false,
            ..lectura()
        };
        assert!(
            sql_lectura(&sin)
                .unwrap()
                .contains("ELSE s.\"geom\" END AS _g")
        );
        assert!(!sql_lectura(&sin).unwrap().contains("ST_GeomFromWKB"));
        assert_eq!(
            sql_tipo_de_la_geometria(&lectura()).unwrap(),
            "SELECT typeof(\"geom\") AS tipo FROM read_parquet('/datos/a.parquet') LIMIT 1;
"
        );
        let s = sql_lectura(&sin).unwrap();
        assert!(
            !s.contains("NOT EXISTS") && !s.contains("ST_Transform"),
            "{s}"
        );
        // las comillas no se escapan de su sitio
        let raro = Lectura {
            columnas: vec!["a\"b".into()],
            ..sin
        };
        assert!(sql_lectura(&raro).unwrap().contains("s.\"a\"\"b\""));
        assert!(
            sql_lectura(&Lectura {
                ficheros: vec![],
                ..lectura()
            })
            .is_err()
        );
    }

    #[test]
    fn el_indice_lleva_las_columnas_del_esquema_y_sus_ids() {
        let s = sql_indice(
            &lectura(),
            &I1,
            &rejilla(),
            &ids(),
            "/trabajo/indice/indice.parquet",
            &Tramo::default(),
        )
        .unwrap();
        // en el orden de `indice::esquema`
        let pos = |c: &str| {
            s.find(&format!("AS \"{c}\""))
                .unwrap_or_else(|| panic!("{c}: {s}"))
        };
        let orden = [
            COL_ID,
            COL_FICHERO,
            COL_FILA,
            COL_CLAVE,
            COL_TROZO,
            COL_GEOM,
            COL_CAJA,
        ];
        assert!(orden.windows(2).all(|w| pos(w[0]) < pos(w[1])));
        // los ids de campo, con los hijos de la caja
        assert!(s.contains("'_ore_id': 1"), "{s}");
        assert!(s.contains("'geom_bbox': {'__duckdb_field_id': "), "{s}");
        assert!(s.contains("'xmin': "), "{s}");
        // las celdas del suelo (z13: 4 891,97 m) y el umbral de 4 celdas
        assert!(s.contains("4891.96981025128"), "{s}");
        assert!(s.contains("ROW_GROUP_SIZE 100000"));
        // lo que leen dos ramas no se guarda entero antes del filtro del tramo (I2·4)
        assert!(s.contains("WHERE _parte OR true\n"), "{s}");
        // con tramos, lo entero se filtra por su clave antes de las dos ramas
        let t = sql_indice(
            &lectura(),
            &I1,
            &rejilla(),
            &ids(),
            "/x",
            &Tramo::de_cortes(&[5])[1],
        )
        .unwrap();
        assert!(t.contains("WHERE _parte OR ((CASE"), "{t}");
        assert!(t.contains("WHERE (_k >= 5 OR _k IS NULL)"), "{t}");
        // un solo fichero: repartir en varios intercala tramos (I2·3)
        assert!(
            s.contains("TO '/trabajo/indice/indice.parquet'") && !s.contains("FILE_SIZE_BYTES")
        );
        assert!(
            sql_indice(
                &lectura(),
                &Receta { bits: 32, ..I1 },
                &rejilla(),
                &ids(),
                "/x",
                &Tramo::default()
            )
            .is_err()
        );
        assert!(sql_indice(&lectura(), &I1, &rejilla(), &[], "/x", &Tramo::default()).is_err());
    }

    #[test]
    fn quitar_casa_por_ordinal_y_por_posicion_y_respeta_lo_ya_borrado() {
        let indice = [(
            "gs://c/t/_ore_capa/i1/s0/data/indice-0000.parquet".to_string(),
            "/trabajo/i/indice-0000.parquet".to_string(),
        )];
        let s = sql_quitar(
            &indice,
            &["/trabajo/i/borrados-1.parquet".into()],
            &[3, 7],
            &["/trabajo/f/borrados.parquet".into()],
            &[(0, "gs://c/t/data/a.parquet".into())],
            "/trabajo/salida.parquet",
        );
        assert!(s.contains("i.\"_ore_fichero\" IN (3, 7)"), "{s}");
        assert!(s.contains("('/trabajo/i/indice-0000.parquet', 'gs://c/t/_ore_capa/i1/s0/data/indice-0000.parquet')"), "{s}");
        assert!(
            s.contains("FIELD_IDS {file_path: 2147483546, pos: 2147483545}"),
            "{s}"
        );
        assert!(s.contains("WHERE NOT ya"), "{s}");
        // sin retirados ni borrados: nada casa
        let n = sql_quitar(&indice, &[], &[], &[], &[], "/x");
        assert!(n.contains("WHERE false\n  OR EXISTS"), "{n}");
        assert!(sql_cuenta_quitar(&[3]).contains("ordinal NOT IN (3)"));
    }

    #[test]
    fn fundir_lee_lo_vivo_y_ordena_por_la_clave_de_la_caja() {
        let r = rejilla();
        // la clave de la caja es la de la geometría: el mismo centro, pegado igual
        let g = clave_sql(&r, "g");
        let c = clave_sql_de_caja(&r, "c");
        assert_eq!(
            g.replace("(ST_XMin(g) + ST_XMax(g))", "X")
                .replace("(ST_YMin(g) + ST_YMax(g))", "Y")
                .split("THEN NULL")
                .nth(1),
            c.replace("(c.xmin + c.xmax)", "X")
                .replace("(c.ymin + c.ymax)", "Y")
                .split("THEN NULL")
                .nth(1)
        );
        let v = sql_vista_fundir(
            &[(
                "gs://c/i/data/indice-0000.parquet".into(),
                "/t/indice-0000.parquet".into(),
            )],
            &["/t/borrados-1.parquet".into()],
            &r,
        );
        assert!(v.contains("EXCLUDE (filename, file_row_number)"), "{v}");
        assert!(v.contains("read_parquet(['/t/borrados-1.parquet'])"), "{v}");
        let f = sql_fundir(&ids(), "/t/nivel1-x-0000.parquet", &Tramo::default()).unwrap();
        assert!(
            f.contains("ORDER BY _k NULLS LAST, \"_ore_id\", \"_ore_trozo\""),
            "{f}"
        );
        assert!(f.contains("EXCLUDE (_k)") && f.contains("FIELD_IDS"), "{f}");
        assert!(sql_cortes_fundir(3).contains("[0.3333333333333333, 0.6666666666666666]"));
    }

    #[test]
    fn los_tramos_cubren_la_clave_entera_sin_pisarse() {
        let t = Tramo::de_cortes(&[30, 10, 20, 20]);
        assert_eq!(t.len(), 4, "tres cortes distintos, cuatro tramos");
        assert_eq!(
            t[0],
            Tramo {
                desde: None,
                hasta: Some(10)
            }
        );
        assert_eq!(
            t[3],
            Tramo {
                desde: Some(30),
                hasta: None
            }
        );
        assert!(t.windows(2).all(|w| w[0].hasta == w[1].desde));
        assert_eq!(t[0].filtro("_k"), "_k < 10");
        assert_eq!(t[1].filtro("_k"), "_k >= 10 AND _k < 20");
        // lo nulo (geometría nula o vacía) va al último tramo, y sólo a él
        assert_eq!(t[3].filtro("_k"), "(_k >= 30 OR _k IS NULL)");
        assert_eq!(Tramo::de_cortes(&[]), [Tramo::default()]);
        assert_eq!(Tramo::default().filtro("_k"), "true");
        let s = sql_cortes(&rejilla(), 4);
        assert!(
            s.contains("approx_quantile(") && s.contains("[0.25, 0.5, 0.75]"),
            "{s}"
        );
    }

    /// Escribe el SQL de la prueba para correrlo en DuckDB (`pruebas-de-fuego/geo/i2/duckdb.py`), con
    /// las claves Hilbert que da Rust para unos puntos: DuckDB tiene que dar las mismas.
    #[test]
    fn escribe_el_sql_de_ejemplo() {
        let Ok(dir) = std::env::var("ORE_I2_SQL") else {
            return;
        };
        let l = lectura();
        let r = rejilla();
        let puntos = [
            (-8_240_000.0, 4_970_000.0),
            (-8_299_999.0, 4_900_001.0),
            (-8_150_001.0, 5_049_999.0),
            (-8_225_000.0, 4_975_000.0),
        ];
        let claves: Vec<String> = puntos
            .iter()
            .map(|(x, y)| format!("[{x:?}, {y:?}, {}]", r.clave([*x, *y, *x, *y], 16)))
            .collect();
        let id = Identidad::Posicion {
            fichero: 1,
            fila: 2,
        }
        .id()
        .unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, t: &str| std::fs::write(format!("{dir}/{n}"), t).unwrap();
        w("1-lectura.sql", &sql_lectura(&l).unwrap());
        w("2-extension.sql", SQL_EXTENSION_Y_CUENTA);
        // en dos tramos, cortados en una de las claves de arriba: cada uno, su fichero
        let tramos = Tramo::de_cortes(&[715_827_882]);
        let ficheros: Vec<String> = (0..tramos.len())
            .map(|i| format!("/trabajo/indice/indice-{i:04}.parquet"))
            .collect();
        for (i, t) in tramos.iter().enumerate() {
            w(
                &format!("3-indice-{i}.sql"),
                &sql_indice(&l, &I1, &r, &ids(), &ficheros[i], t).unwrap(),
            );
        }
        w("4-cuenta.sql", &sql_cuenta_indice(&ficheros));
        w("5-profundidad.sql", &sql_profundidad(&ficheros));
        w(
            "rust.json",
            &format!(
                "{{\"rejilla\": [{:?}, {:?}, {:?}, {:?}], \"claves\": [{}], \"id_1_2\": {id}}}\n",
                r.x0,
                r.y0,
                r.x1,
                r.y1,
                claves.join(", ")
            ),
        );
    }
}
