//! `ore-read-postgres` — el lector de PostgreSQL, **fuera** del compilador.
//!
//! # Por qué esto es un programa y no una receta
//!
//! `ore` trae dentro la receta de BigQuery: ejecuta `bq`, que el usuario ya tiene
//! y ya autenticó, y traduce lo que dice. Cuesta cero dependencias. La pregunta
//! honesta era si Postgres podía resolverse igual, delegando en `psql`.
//!
//! No, y por una razón que se ve midiendo: **`psql` no estaba instalado.** `bq`
//! viene con el SDK de Google, que es *cómo se usa* BigQuery; `psql` viene con
//! las herramientas de cliente de Postgres, que mucha gente que consulta una base
//! de datos no tiene. Un lector que exige instalar un cliente de línea de órdenes
//! antes de leer un catálogo no es un lector: es un requisito previo.
//!
//! Y hay una segunda razón, más limpia: si esto delegara en `psql` **no sería un
//! driver**, sería una receta con otro sombrero, y entonces lo correcto sería
//! meterla dentro de `ore` como la de BigQuery y no tener un binario aparte. Lo
//! que justifica que exista fuera es exactamente lo que trae dentro.
//!
//! # El precio, medido
//!
//! Dos escalas, porque miden cosas distintas: lo que se **enlaza** (`cargo tree
//! -e normal`) y el **cierre del lock**, que incluye ademas lo que corre en
//! tiempo de construccion —macros y `build.rs`— y que es la escala que vigila el
//! guardian, por ser la conservadora.
//!
//! | | enlazado | cierre del lock | qué arrastra |
//! |---|---|---|---|
//! | `ore` | **28** | **32** | nada nativo |
//! | esto, con `native-tls` | **71** | **114** | `schannel` en Windows, OpenSSL en Linux |
//! | esto, con `rustls` | 85 | — | `aws-lc-sys` — C, cmake y nasm |
//!
//! Ese salto es el motivo de que la costura exista. Un driver es gordo porque
//! hablar TLS y SCRAM con un servidor ajeno es gordo, y no hay forma de que
//! adelgace. Lo que sí se puede decidir es **dónde cae el peso**, y cae aquí.
//!
//! No es una promesa: `crates/ore-cli/tests/dependencias.rs` lee el cierre de
//! `ore-cli` en `Cargo.lock` y falla si alguna de estas crates aparece. Estar en
//! directorios distintos no demuestra nada — `cargo` no mira los directorios.
//!
//! # El contrato con `ore`
//!
//! Lo fija `lector.rs`: la URL entra por **stdin** —nunca por `argv`, que lee
//! cualquier proceso de la máquina— y el catálogo sale por **stdout**. El nombre
//! de la fuente llega como primer argumento, porque no es un secreto y el
//! catálogo tiene que decir de dónde viene.
//!
//! # Lo que este lector NO emite, y es una decisión
//!
//! **El número de filas.** `pg_class.reltuples` es la estimación del
//! planificador, no un recuento: vale `-1` hasta que alguien ejecuta `ANALYZE`, y
//! en la base de pruebas valía `-1` para una tabla con cincuenta filas dentro. El
//! inductor propone revisar una tabla con cero filas por si es un resto; darle una
//! estimación sin analizar le haría proponer borrar tablas vivas. Un `count(*)`
//! por tabla sería un hecho, y también un escaneo completo del almacén. Entre una
//! conjetura barata y un hecho carísimo, no emitir nada es lo correcto: el campo
//! es opcional justamente para esto.

use ore_core::json::Json;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::process::ExitCode;

/// Un documento, una consulta.
///
/// Va contra `pg_catalog` y no contra `information_schema`, y eso salió midiendo:
/// **las vistas materializadas no aparecen en `information_schema`**. Un lector
/// escrito contra el catálogo estándar habría perdido `mv_pedidos_por_pais` sin
/// decir nada, que es la peor forma de perder algo.
///
/// `NOT c.relispartition` no es una optimización. Las particiones de una tabla
/// particionada son `pg_class` de pleno derecho con `relkind = 'r'`: sin ese
/// filtro, una tabla partida por meses produciría la entidad **más doce copias**.
const CATALOGO: &str = "\
SELECT n.nspname                            AS esquema,
       c.relname                            AS tabla,
       c.relkind::text                      AS clase,
       a.attname                            AS columna,
       a.attnotnull                         AS obligatoria,
       format_type(a.atttypid, a.atttypmod) AS tipo,
       t.typtype::text                      AS familia,
       CASE WHEN t.typtype = 'd'
            THEN format_type(t.typbasetype, a.atttypmod) END AS base,
       ARRAY(SELECT e.enumlabel::text FROM pg_enum e
              WHERE e.enumtypid = a.atttypid
              ORDER BY e.enumsortorder)     AS valores,
       d.description                        AS descripcion,
       EXISTS (SELECT 1 FROM pg_constraint p
                WHERE p.conrelid = c.oid AND p.contype = 'p'
                  AND a.attnum = ANY (p.conkey)) AS clave,
       (SELECT rn.nspname || '.' || rc.relname
          FROM pg_constraint f
          JOIN pg_class rc     ON rc.oid = f.confrelid
          JOIN pg_namespace rn ON rn.oid = rc.relnamespace
         WHERE f.conrelid = c.oid AND f.contype = 'f'
           AND a.attnum = ANY (f.conkey)
         LIMIT 1)                           AS referencia,
       -- La columna del destino EMPAREJADA con esta. `conkey` y `confkey` son
       -- dos arrays alineados por posicion, asi que la pareja de la columna
       -- local esta en el mismo indice. Sin esto no se sabe si la foranea
       -- apunta a la clave primaria o a otra UNIQUE, y SQL permite las dos:
       -- emitir la relacion sin saberlo la deja verde y equivocada.
       (SELECT ra.attname
          FROM pg_constraint f
          JOIN LATERAL generate_subscripts(f.conkey, 1) AS i ON TRUE
          JOIN pg_attribute ra
            ON ra.attrelid = f.confrelid AND ra.attnum = f.confkey[i]
         WHERE f.conrelid = c.oid AND f.contype = 'f'
           AND f.conkey[i] = a.attnum
         LIMIT 1)                           AS ref_columna
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
JOIN pg_type t      ON t.oid = a.atttypid
LEFT JOIN pg_description d ON d.objoid = c.oid AND d.objsubid = a.attnum
WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND NOT c.relispartition
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND n.nspname !~ '^pg_'
ORDER BY n.nspname, c.relname, a.attnum";

/// Las claves alternativas, en una segunda consulta.
///
/// Va por `pg_index` y no por `pg_constraint` porque una restriccion UNIQUE
/// siempre tiene indice detras, y `CREATE UNIQUE INDEX` a secas no tiene
/// restriccion: mirar solo las restricciones perderia la mitad.
///
/// Se excluyen las **parciales** —`indpred IS NOT NULL`— y las de **expresion**
/// —un 0 en `indkey`—. Un indice unico parcial no garantiza unicidad global:
/// tomarlo por clave alternativa afirmaria una identidad que el origen no
/// sostiene, que es exactamente lo que este programa no hace.
///
/// No es una llamada por tabla: son dos consultas para todo el esquema.
const UNICAS: &str = "SELECT n.nspname AS esquema,
       c.relname AS tabla,
       ARRAY(SELECT a.attname
               FROM unnest(i.indkey) WITH ORDINALITY AS k(num, ord)
               JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.num
              ORDER BY k.ord) AS columnas
FROM pg_index i
JOIN pg_class c      ON c.oid = i.indrelid
JOIN pg_namespace n  ON n.oid = c.relnamespace
WHERE i.indisunique AND NOT i.indisprimary AND i.indpred IS NULL
  AND 0 <> ALL (i.indkey)
  AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname !~ '^pg_'
ORDER BY n.nspname, c.relname, i.indexrelid";

fn main() -> ExitCode {
    match intentar() {
        Ok(catalogo) => {
            println!("{catalogo}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            // `ore` muestra este texto literal, sin resumirlo. Es lo único
            // accionable que va a ver quien lo ejecute.
            eprintln!("ore-read-postgres: {m}");
            ExitCode::FAILURE
        }
    }
}

fn intentar() -> Result<String, String> {
    let fuente = std::env::args().nth(1).unwrap_or_else(|| "postgres".into());

    let mut url = String::new();
    std::io::stdin()
        .read_to_string(&mut url)
        .map_err(|e| format!("no se pudo leer la URL de stdin: {e}"))?;
    let url = url.trim();
    if url.is_empty() {
        return Err(
            "no llegó ninguna URL por stdin. La espera de `ore discover \
                    --source`, que se la pasa por ahí y no por la línea de órdenes"
                .into(),
        );
    }

    let tls = postgres_native_tls::MakeTlsConnector::new(
        native_tls::TlsConnector::new().map_err(|e| format!("no se pudo preparar TLS: {e}"))?,
    );
    let mut cliente = postgres::Client::connect(url, tls)
        // El mensaje del servidor va entero: «password authentication failed» y
        // «no pg_hba.conf entry» se arreglan solos en cuanto se leen, y
        // resumirlos los convierte en una tarde.
        .map_err(|e| format!("no se pudo conectar: {e}"))?;

    let filas = cliente
        .query(CATALOGO, &[])
        .map_err(|e| format!("la consulta del catálogo falló: {e}"))?;
    let unicas = cliente
        .query(UNICAS, &[])
        .map_err(|e| format!("la consulta de claves alternativas falló: {e}"))?;

    Ok(armar(&fuente, &filas, &unicas).pretty())
}

// ── La costura de tipos ─────────────────────────────────────────────────────

/// Del sistema de tipos de PostgreSQL al vocabulario de escalares de OOS.
///
/// `None` significa **no lo sé traducir**, y no se rellena con `Opaque`.
/// `Opaque` afirma *«hay un valor y su interior no se modela»*: cierto de un
/// `bytea`, falso de un tipo compuesto como `direccion (calle, cp, pais)`, cuya
/// estructura el catálogo acaba de enumerar. Traducirlo tiraría un hecho;
/// convertirlo en entidades anidadas inventaría un modelo. Se reporta.
fn escalar(pg: &str) -> Option<&'static str> {
    // `numeric(12,2)` y `character varying(255)` son el mismo tipo que sin
    // paréntesis: la precisión es una restricción, no otro tipo.
    let base = pg.split_once('(').map_or(pg, |(b, _)| b).trim();
    Some(match base {
        "smallint" | "integer" | "bigint" | "int2" | "int4" | "int8" | "oid" | "smallserial"
        | "serial" | "bigserial" => "Integer",
        "numeric" | "decimal" | "money" => "Decimal",
        "real" | "double precision" | "float4" | "float8" => "Float",
        "boolean" | "bool" => "Boolean",
        "text" | "character varying" | "character" | "varchar" | "char" | "bpchar" | "name"
        | "citext" => "String",
        // Un UUID tiene una forma canónica en texto y se compara como texto.
        // `Opaque` diría que no se puede mirar dentro, y sí se puede.
        "uuid" => "String",
        "date" => "Date",
        "time without time zone" | "time with time zone" | "time" | "timetz" => "Time",
        "timestamp without time zone" | "timestamp" => "DateTime",
        // La distinción existe en los dos sistemas de tipos y se conserva.
        "timestamp with time zone" | "timestamptz" => "DateTimeTz",
        "bytea" | "json" | "jsonb" | "xml" | "inet" | "cidr" | "macaddr" | "macaddr8"
        | "interval" | "tsvector" | "tsquery" | "bit" | "bit varying" | "point" | "line"
        | "lseg" | "box" | "path" | "polygon" | "circle" | "pg_lsn" | "txid_snapshot" => "Opaque",
        _ => return None,
    })
}

/// El tipo de una columna, resuelto.
///
/// - Un **dominio** (`typtype = 'd'`) es su tipo base con una restricción encima:
///   `nif` es `text` que además cumple algo. El tipo base es un hecho; la
///   restricción no tiene dónde ir, y perderla es mejor que inventarle un sitio.
/// - Una **enumeración** (`typtype = 'e'`) es `String` **más sus valores**, que
///   el esquema de OOS admite como secuencia. Es información que Postgres tiene y
///   BigQuery no: dejarla caer haría este lector peor que su fuente.
/// - Un **array** es `list<X>` si `X` se sabe traducir.
fn traducir(tipo: &str, familia: &str, base: Option<&str>) -> Option<String> {
    let t = tipo.trim();
    if let Some(dentro) = t.strip_suffix("[]") {
        return escalar(dentro.trim()).map(|e| format!("list<{e}>"));
    }
    match familia {
        "d" => escalar(base?).map(String::from),
        "e" => Some("String".into()),
        _ => escalar(t).map(String::from),
    }
}

/// `relkind`, tal y como lo dice el catálogo.
///
/// `f` —una tabla foránea— se nombra aparte a propósito: sus datos viven en OTRA
/// fuente, y eso es justo lo que un repositorio ontológico necesita que alguien
/// mire antes de declararla aquí.
fn clase(relkind: &str) -> &'static str {
    match relkind {
        "v" => "view",
        "m" => "materializedView",
        "f" => "foreignTable",
        _ => "table",
    }
}

// ── El catálogo ─────────────────────────────────────────────────────────────

struct Acc {
    clase: &'static str,
    columnas: Vec<Json>,
    clave: Vec<Json>,
    /// destino -> pares (columna local, columna del destino), en orden.
    foraneas: BTreeMap<String, Vec<(String, String)>>,
}

fn armar(fuente: &str, filas: &[postgres::Row], unicas: &[postgres::Row]) -> Json {
    // Indice tabla -> claves alternativas, antes del bucle principal.
    let mut alternativas: BTreeMap<String, Vec<Json>> = BTreeMap::new();
    for u in unicas {
        let (Some(e), Some(t)) = (
            u.get::<_, Option<String>>("esquema"),
            u.get::<_, Option<String>>("tabla"),
        ) else {
            continue;
        };
        let cols: Vec<String> = u.get("columnas");
        if !cols.is_empty() {
            alternativas
                .entry(format!("{e}.{t}"))
                .or_default()
                .push(Json::Arr(cols.iter().map(Json::s).collect()));
        }
    }

    let mut orden: Vec<String> = Vec::new();
    let mut tablas: BTreeMap<String, Acc> = BTreeMap::new();

    for f in filas {
        let cadena = |k: &str| f.get::<_, Option<String>>(k).filter(|s| !s.is_empty());
        let (Some(esquema), Some(tabla), Some(columna), Some(tipo)) = (
            cadena("esquema"),
            cadena("tabla"),
            cadena("columna"),
            cadena("tipo"),
        ) else {
            continue;
        };
        let cualificado = format!("{esquema}.{tabla}");
        let acc = tablas.entry(cualificado.clone()).or_insert_with(|| {
            orden.push(cualificado.clone());
            Acc {
                clase: clase(cadena("clase").as_deref().unwrap_or("r")),
                columnas: Vec::new(),
                clave: Vec::new(),
                foraneas: BTreeMap::new(),
            }
        });

        let familia = cadena("familia").unwrap_or_default();
        let base = cadena("base");
        let mut c: BTreeMap<String, Json> = BTreeMap::new();
        c.insert("name".into(), Json::s(&columna));
        match traducir(&tipo, &familia, base.as_deref()) {
            Some(t) => {
                c.insert("type".into(), Json::s(t));
            }
            // Sin `type`. `sourceType` se cita aguas abajo, nunca se interpreta.
            None => {
                c.insert("sourceType".into(), Json::s(&tipo));
            }
        }
        if f.get::<_, Option<bool>>("obligatoria") == Some(true) {
            c.insert("required".into(), Json::Bool(true));
        }
        let valores: Vec<String> = f.get("valores");
        if !valores.is_empty() {
            // El orden es el de declaración (`enumsortorder`), y se conserva:
            // el esquema dice que reordenarlos es un cambio observable.
            c.insert(
                "enum".into(),
                Json::Arr(valores.iter().map(Json::s).collect()),
            );
        }
        if let Some(d) = cadena("descripcion") {
            c.insert("description".into(), Json::s(d));
        }
        acc.columnas.push(Json::Obj(c));

        if f.get::<_, Option<bool>>("clave") == Some(true) {
            acc.clave.push(Json::s(&columna));
        }
        if let Some(r) = cadena("referencia") {
            // Una foránea compuesta llega como una fila por columna, todas
            // apuntando a la misma tabla: se agrupan en UNA arista, y las
            // columnas de los dos lados se mantienen emparejadas en orden.
            let par = (columna, cadena("ref_columna").unwrap_or_default());
            acc.foraneas.entry(r).or_default().push(par);
        }
    }

    let tablas = orden
        .into_iter()
        .filter_map(|t| {
            let a = tablas.remove(&t)?;
            let mut o: BTreeMap<String, Json> = BTreeMap::new();
            o.insert("name".into(), Json::s(&t));
            o.insert("kind".into(), Json::s(a.clase));
            o.insert("columns".into(), Json::Arr(a.columnas));
            if !a.clave.is_empty() {
                o.insert("primaryKey".into(), Json::Arr(a.clave));
            }
            if let Some(u) = alternativas.remove(&t) {
                o.insert("uniqueKeys".into(), Json::Arr(u));
            }
            if !a.foraneas.is_empty() {
                o.insert(
                    "foreignKeys".into(),
                    Json::Arr(
                        a.foraneas
                            .into_iter()
                            .map(|(destino, pares)| {
                                Json::obj([
                                    (
                                        "columns",
                                        Json::Arr(pares.iter().map(|(l, _)| Json::s(l)).collect()),
                                    ),
                                    ("references", Json::s(destino)),
                                    (
                                        "toColumns",
                                        Json::Arr(pares.iter().map(|(_, r)| Json::s(r)).collect()),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                );
            }
            Some(Json::Obj(o))
        })
        .collect();

    Json::obj([("source", Json::s(fuente)), ("tables", Json::Arr(tablas))])
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// La distinción que los dos sistemas de tipos hacen y que se conserva.
    #[test]
    fn con_zona_y_sin_zona_no_son_el_mismo_tipo() {
        assert_eq!(
            traducir("timestamp with time zone", "b", None).as_deref(),
            Some("DateTimeTz")
        );
        assert_eq!(
            traducir("timestamp without time zone", "b", None).as_deref(),
            Some("DateTime")
        );
    }

    /// La precisión es una restricción, no otro tipo.
    #[test]
    fn la_precision_no_cambia_el_tipo() {
        assert_eq!(
            traducir("numeric(12,2)", "b", None).as_deref(),
            Some("Decimal")
        );
        assert_eq!(
            traducir("character varying(255)", "b", None).as_deref(),
            Some("String")
        );
    }

    /// Un dominio es su tipo base con una restricción encima. `nif` es `text`.
    #[test]
    fn un_dominio_es_su_tipo_base() {
        assert_eq!(
            traducir("nif", "d", Some("text")).as_deref(),
            Some("String")
        );
        // Y si la base tampoco se sabe traducir, no se inventa.
        assert_eq!(traducir("raro", "d", Some("direccion")), None);
    }

    /// El caso que decide la doctrina: un tipo compuesto **no** es `Opaque`.
    /// `Opaque` dice «no hay estructura que modelar» y el catálogo la enumeró.
    #[test]
    fn un_compuesto_no_se_disfraza_de_opaque() {
        assert_eq!(traducir("direccion", "c", None), None);
        assert_eq!(traducir("bytea", "b", None).as_deref(), Some("Opaque"));
        assert_eq!(traducir("jsonb", "b", None).as_deref(), Some("Opaque"));
    }

    #[test]
    fn un_array_de_escalar_es_una_lista() {
        assert_eq!(
            traducir("text[]", "b", None).as_deref(),
            Some("list<String>")
        );
        assert_eq!(
            traducir("bigint[]", "b", None).as_deref(),
            Some("list<Integer>")
        );
        // Un array de compuestos sigue sin saberse traducir.
        assert_eq!(traducir("direccion[]", "b", None), None);
    }

    /// Un UUID se compara y se une como texto: `Opaque` diría que no se puede
    /// mirar dentro, y sí se puede.
    #[test]
    fn un_uuid_es_texto() {
        assert_eq!(traducir("uuid", "b", None).as_deref(), Some("String"));
    }

    #[test]
    fn las_vistas_se_distinguen_de_las_tablas() {
        assert_eq!(clase("r"), "table");
        assert_eq!(clase("p"), "table");
        assert_eq!(clase("v"), "view");
        assert_eq!(clase("m"), "materializedView");
        assert_eq!(clase("f"), "foreignTable");
    }
}
