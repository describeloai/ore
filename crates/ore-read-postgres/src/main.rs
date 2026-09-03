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

mod sql;

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
       -- La IDENTIDAD DE REPLICACION: que trae el changelog cuando alguien
       -- borra o actualiza. Es lo que decide `changes.mode`, y es un hecho del
       -- objeto que solo se sabe preguntandoselo al servidor.
       c.relreplident::text                 AS identidad,
       -- Y con `relreplident = 'i'`, POR QUE INDICE. Sin sus columnas no se
       -- puede declarar un `upsert`: la clave es lo que empareja un tombstone
       -- con la fila que retira, y un upsert sin clave no dice que quita.
       ARRAY(SELECT ra.attname
               FROM pg_index ix
               JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(num, ord) ON TRUE
               JOIN pg_attribute ra ON ra.attrelid = ix.indrelid AND ra.attnum = k.num
              WHERE ix.indrelid = c.oid AND ix.indisreplident
              ORDER BY k.ord)               AS identidad_columnas,
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
    // El verbo es explícito desde que hay dos. Antes estaba implícito en que
    // solo hubiera uno; deducirlo del contenido de stdin sería adivinar.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (verbo, fuente) = match args.first().map(String::as_str) {
        Some("leer") => ("leer", args.get(1).cloned().unwrap_or_default()),
        Some("catalogo") => ("catalogo", args.get(1).cloned().unwrap_or_default()),
        Some("testigo") => ("testigo", args.get(1).cloned().unwrap_or_default()),
        // La forma anterior —`ore-read-postgres <fuente>`— sigue significando
        // `catalogo`: quien la usara no tiene por qué enterarse de que ahora
        // hay dos verbos.
        otro => ("catalogo", otro.unwrap_or("postgres").to_string()),
    };

    let mut entrada = String::new();
    std::io::stdin()
        .read_to_string(&mut entrada)
        .map_err(|e| format!("no se pudo leer stdin: {e}"))?;
    if entrada.trim().is_empty() {
        return Err(
            "no llegó nada por stdin. `catalogo` espera una URL y `leer` una \
             petición JSON, y las dos van por ahí y no por la línea de órdenes"
                .into(),
        );
    }
    if verbo == "leer" {
        return filas(&entrada);
    }
    if verbo == "testigo" {
        return testigo(&entrada);
    }
    let url = entrada.trim();

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

    // El sondeo de la cara `D`. Es una pregunta al servidor y no una opción de
    // este programa: si el clúster no está en `logical`, ningún objeto suyo
    // emite cambios, y decir otra cosa sería inventarlo.
    let wal_level: String = cliente
        .query_one("SELECT current_setting('wal_level')", &[])
        .map(|r| r.get::<_, String>(0))
        .unwrap_or_else(|_| String::new());
    if wal_level != "logical" {
        // Por stderr, que es donde va lo que no es el catálogo. Y se avisa en
        // vez de callar: un `changes: none` en cuarenta tablas tiene el mismo
        // aspecto que un origen que de verdad no cambia nunca.
        eprintln!(
            "ore-read-postgres: aviso · `wal_level = {}` y no `logical`, así que ninguna tabla \
             declara cambios. Sin decodificación lógica no hay changelog que leer, y lo \
             materializado sobre esto no se podrá refrescar incrementalmente",
            if wal_level.is_empty() {
                "?"
            } else {
                &wal_level
            }
        );
    }

    Ok(armar(&fuente, &filas, &unicas, &wal_level).pretty())
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
    /// `relkind` sin traducir. `clase` ya lo dice para quien lee el catálogo;
    /// esto es para quien tiene que **decidir** con ello, y son dos preguntas:
    /// una vista y una tabla se enseñan distinto y se sondean distinto.
    relkind: String,
    /// `relreplident`, tal cual. Decide `changes.mode`.
    identidad: String,
    /// Las columnas del índice de identidad de replicación, si lo hay.
    identidad_columnas: Vec<Json>,
    columnas: Vec<Json>,
    clave: Vec<Json>,
    /// destino -> pares (columna local, columna del destino), en orden.
    foraneas: BTreeMap<String, Vec<(String, String)>>,
}

fn armar(fuente: &str, filas: &[postgres::Row], unicas: &[postgres::Row], wal_level: &str) -> Json {
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
            let relkind = cadena("clase").unwrap_or_else(|| "r".into());
            Acc {
                clase: clase(&relkind),
                relkind,
                identidad: cadena("identidad").unwrap_or_else(|| "d".into()),
                identidad_columnas: f
                    .get::<_, Vec<String>>("identidad_columnas")
                    .iter()
                    .map(Json::s)
                    .collect(),
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
            // Las dos caras del objeto. Van EN EL CATALOGO y no en el inductor
            // porque solo el driver las sabe: qué se puede empujar es de quien
            // traduce, y qué cambios salen es de quien preguntó al servidor.
            o.insert("reads".into(), reads());
            // La clave del upsert: la primaria si la identidad es la de por
            // defecto, y las columnas del índice si alguien eligió otro.
            let identidad_clave = if a.identidad == "i" {
                a.identidad_columnas.clone()
            } else {
                a.clave.clone()
            };
            o.insert(
                "changes".into(),
                changes(wal_level, &a.relkind, &a.identidad, &identidad_clave),
            );
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

// ── Las dos caras ───────────────────────────────────────────────────────────

/// La cara `I`: **lo que este driver sabe empujar**, no lo que PostgreSQL sabe.
///
/// La distinción no es escrúpulo: `reads` es el contrato con el que el
/// planificador decide qué baja al origen, y lo que baja se construye en
/// `sql::sql`. Declarar `neq`, `range` o `isNull` sería prometer una traducción
/// que no existe, y el precio lo paga quien menos lo ve — un filtro que el
/// driver no sabe poner **se cae de la petición**, y una consulta devuelve más
/// filas de las que pidió sin que nadie vea un error.
///
/// Así que aquí se declara lo que hay: `eq`, y el recorrido completo. Ensanchar
/// esto es un cambio en `sql.rs` primero y en esta lista después, en ese orden.
///
/// `gt` existe en el protocolo y **no** aparece: es de la marca de agua, y el
/// `range` de OOS son las cuatro comparaciones. Declararlo por la mitad sería
/// prometer las otras tres.
fn reads() -> Json {
    Json::obj([
        ("predicatePushdown", Json::Arr(vec![Json::s("eq")])),
        ("fullScan", Json::s("cheap")),
    ])
}

/// La cara `D`, **sondeada**: qué cambios puede emitir este objeto de verdad.
///
/// No es una conjetura ni una preferencia: sale de `wal_level` y de la identidad
/// de replicación del objeto, que son dos hechos que el servidor contesta. Por
/// eso puede emitirse sin revisión, que es lo que separa este descubrimiento de
/// uno con un modelo dentro.
///
/// | lo que dice el servidor | lo que emite | por qué |
/// |---|---|---|
/// | no es una tabla (`v`, `m`, `f`) | `{none, none}` | una vista no tiene flujo propio; una materializada se refresca, que no es un changelog; una foránea tiene sus datos en otra fuente |
/// | `wal_level` no es `logical` | `{none, none}` | sin decodificación lógica no sale ningún cambio, y decir otra cosa sería inventarlo |
/// | `REPLICA IDENTITY FULL` | `{retract, log}` | el changelog trae la imagen previa entera: un borrado retracta y una actualización retracta y añade |
/// | `DEFAULT` con clave primaria, o `USING INDEX` | `{upsert, log}` | llega la clave y no la fila vieja, que es exactamente un tombstone por clave |
/// | `DEFAULT` sin clave primaria, o `NOTHING` | `{append, log}` | un borrado no se puede decodificar, así que del flujo solo salen altas |
///
/// La última fila es la que más cuesta y la que más vale. `append` NO es la
/// respuesta cómoda: es la que hace que `OOS2021` rechace materializar esa tabla
/// para respaldar una entidad mutable, que es justo lo que pasaría de verdad —
/// las filas borradas se quedarían en la copia y nadie vería nada.
fn changes(wal_level: &str, relkind: &str, identidad: &str, clave: &[Json]) -> Json {
    let ninguno = || Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))]);
    if !matches!(relkind, "r" | "p") || wal_level != "logical" {
        return ninguno();
    }
    match identidad {
        "f" => Json::obj([("mode", Json::s("retract")), ("witness", Json::s("log"))]),
        _ if !clave.is_empty() => Json::obj([
            ("mode", Json::s("upsert")),
            ("key", Json::Arr(clave.to_vec())),
            ("witness", Json::s("log")),
        ]),
        // `d` sin clave primaria, `n`, o `i` sin columnas: no hay con qué
        // retirar una fila. Solo las altas sobreviven al viaje.
        _ => Json::obj([("mode", Json::s("append")), ("witness", Json::s("log"))]),
    }
}

/// **El tercer verbo: hasta donde esta el servidor ahora.**
///
/// Normativo: [ADR 0016](../../../docs/decisions/0016-el-testigo-y-el-rango.md),
/// decision A. `changes()` dice **con que** se fecha; esto dice **cuanto vale
/// ahora mismo**, y son dos preguntas con dos cadencias: la primera cambia
/// cuando alguien altera la tabla, la segunda **en cada confirmacion**.
///
/// # Por que `pg_current_wal_lsn()` y no un reloj
///
/// Porque un LSN es una **posicion de confirmacion**: un orden total sin
/// empates, y replayable. Un `now()` no lo es — dos transacciones pueden
/// compartir instante, y entonces retomar desde ahi pierde o repite. Es la
/// diferencia entre `witness: log` y `witness: field`, y es la que decide si
/// una copia puede ser atomica.
///
/// Y es lo mismo que hace Debezium, que tiene este problema exacto y lo
/// documenta: lee la posicion del log al empezar la instantanea y despues
/// *«continues streaming from the position that it read in Step 2»*.
///
/// # El alcance es del SERVIDOR, no del objeto
///
/// Un LSN vale para todo el cluster, asi que la peticion trae `objeto` y aqui
/// **no se usa**. No sobra: con `witness: field` haria falta, y el protocolo es
/// uno para todos los drivers. Que el modo decida el alcance es lo que el ADR
/// 0016 contesto mirando como lo hacen los demas — un snapshot de Iceberg es de
/// una tabla; un LSN, del servidor.
///
/// # Y si no hay decodificacion logica
///
/// Se contesta `none`, por lo mismo que `changes()` devuelve `{none, none}`:
/// sin `wal_level = logical` no sale ningun cambio de este servidor, asi que un
/// LSN no fecha nada que se pueda volver a pedir. Devolverlo igual seria dar una
/// marca que no respalda un refresco.
fn testigo(entrada: &str) -> Result<String, String> {
    let (url, _objeto) = ore_driver::leer_coordenada(entrada)?;

    let tls = postgres_native_tls::MakeTlsConnector::new(
        native_tls::TlsConnector::new().map_err(|e| format!("no se pudo preparar TLS: {e}"))?,
    );
    let mut cliente =
        postgres::Client::connect(&url, tls).map_err(|e| format!("no se pudo conectar: {e}"))?;

    let wal_level: String = cliente
        .query_one("SELECT current_setting('wal_level')", &[])
        .map(|r| r.get::<_, String>(0))
        .unwrap_or_default();
    if wal_level != "logical" {
        eprintln!(
            "ore-read-postgres: aviso · `wal_level = {}` y no `logical`: este servidor no emite              cambios, asi que no hay testigo que dar",
            if wal_level.is_empty() {
                "?"
            } else {
                &wal_level
            }
        );
        return Ok(ore_driver::testigo("none", None));
    }

    // `pg_current_wal_lsn()` en un primario; en una replica no existe y hay que
    // pedir `pg_last_wal_replay_lsn()`. Se intenta el segundo si el primero
    // falla, en vez de decidirlo por una bandera: quien conecta sabe a que
    // servidor va, y este programa no tiene por que preguntarselo.
    let lsn: Option<String> = cliente
        .query_one("SELECT pg_current_wal_lsn()::text", &[])
        .or_else(|_| cliente.query_one("SELECT pg_last_wal_replay_lsn()::text", &[]))
        .ok()
        .and_then(|r| r.try_get::<_, String>(0).ok());

    match lsn {
        Some(v) => Ok(ore_driver::testigo("log", Some(&v))),
        // Se pudo conectar y no hay posicion: es una respuesta, no un fallo.
        None => Ok(ore_driver::testigo("none", None)),
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

// ── El segundo verbo ────────────────────────────────────────────────────────

/// Devuelve filas, en NDJSON: una por línea.
///
/// La conexión se abre **de solo lectura** pidiéndoselo al servidor, no
/// prometiéndolo. La diferencia es la de siempre: un motor que no escribe porque
/// no tiene código para escribir tiene una **propiedad**; uno que promete no
/// hacerlo tiene una **política** — y aquí el código de escritura existe, lo
/// trae este mismo driver, así que la propiedad hay que comprarla.
fn filas(peticion: &str) -> Result<String, String> {
    let p = ore_driver::leer_peticion(peticion)?;

    // **Sabe recortar por columna y no sabe leer su changelog.** Es la misma
    // frase que `changes()` ya decia al reves: este driver hace UN `SELECT`
    // sobre el estado presente. Un rango sobre el WAL exige decodificacion
    // logica, una ranura de replicacion y una sesion que dure — otro programa.
    //
    // Negarse es lo unico seguro: devolver el estado presente cuando se pidio
    // un incremento no falla, se sirve, y la copia sale con filas de mas.
    if let Some(porque) = ore_driver::rango_servible(&p, true, false) {
        return Err(porque);
    }

    let (consulta, params) = sql::sql(&p);

    let tls = postgres_native_tls::MakeTlsConnector::new(
        native_tls::TlsConnector::new().map_err(|e| format!("no se pudo preparar TLS: {e}"))?,
    );
    let mut cliente =
        postgres::Client::connect(&p.url, tls).map_err(|e| format!("no se pudo conectar: {e}"))?;

    cliente
        .simple_query("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")
        .map_err(|e| format!("no se pudo abrir la sesión en solo lectura: {e}"))?;

    let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|v| v as &(dyn postgres::types::ToSql + Sync))
        .collect();
    let resultado = cliente
        .query(consulta.as_str(), &refs)
        .map_err(|e| format!("la consulta falló: {e}\n  {consulta}"))?;

    let mut out = String::new();
    for fila in &resultado {
        // Todo sale como texto: el driver no interpreta tipos, y convertirlos
        // aquí sería una segunda costura de tipos al lado de la que ya existe
        // para el catálogo.
        let valores: Vec<Option<String>> = (0..p.proyeccion.len())
            .map(|i| fila.try_get::<_, Option<String>>(i).unwrap_or(None))
            .collect();
        out.push_str(&ore_driver::fila(&p, &valores));
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

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

    // ── Las dos caras ───────────────────────────────────────────────────────
    //
    // Todo lo de aquí corre **sin servidor**, que es la razón de que `changes`
    // sea una función pura: lo que decide qué puede refrescarse
    // incrementalmente en toda una empresa no puede probarse solo cuando hay
    // un PostgreSQL delante.

    fn modo(j: &Json) -> String {
        j.jcs()
    }

    /// Sin decodificación lógica no sale ningún cambio, y da igual cómo esté
    /// configurada la tabla: no hay de dónde leerlos.
    #[test]
    fn sin_wal_logical_ninguna_tabla_declara_cambios() {
        for identidad in ["d", "f", "i", "n"] {
            assert_eq!(
                modo(&changes("replica", "r", identidad, &[Json::s("id")])),
                r#"{"mode":"none","witness":"none"}"#,
                "identidad {identidad}"
            );
        }
    }

    /// `REPLICA IDENTITY FULL` trae la imagen previa entera: un borrado
    /// retracta y una actualización retracta y añade. Es el único caso en que
    /// se puede afirmar `retract`.
    #[test]
    fn identidad_full_retracta() {
        assert_eq!(
            modo(&changes("logical", "r", "f", &[])),
            r#"{"mode":"retract","witness":"log"}"#
        );
    }

    /// Con la identidad por defecto llega **la clave** y no la fila vieja, que
    /// es exactamente un tombstone por clave.
    #[test]
    fn identidad_por_defecto_con_clave_es_un_upsert() {
        assert_eq!(
            modo(&changes("logical", "r", "d", &[Json::s("id")])),
            r#"{"key":["id"],"mode":"upsert","witness":"log"}"#
        );
    }

    /// **La que más vale, y la que más cuesta.** Sin clave primaria un borrado
    /// no se puede decodificar, así que del flujo solo salen altas.
    ///
    /// `append` no es la respuesta cómoda: es la que hace que `OOS2021` rechace
    /// materializar esto para respaldar una entidad mutable — que es justo lo
    /// que pasaría de verdad, con las filas borradas quedándose en la copia y
    /// nadie viendo nada.
    #[test]
    fn sin_con_que_retirar_una_fila_solo_quedan_las_altas() {
        for identidad in ["d", "n", "i"] {
            assert_eq!(
                modo(&changes("logical", "r", identidad, &[])),
                r#"{"mode":"append","witness":"log"}"#,
                "identidad {identidad}"
            );
        }
    }

    /// Una vista no tiene flujo propio, una materializada se refresca —que no
    /// es un changelog— y una foránea tiene sus datos en otra fuente.
    #[test]
    fn lo_que_no_es_una_tabla_no_emite_cambios() {
        for relkind in ["v", "m", "f"] {
            assert_eq!(
                modo(&changes("logical", relkind, "f", &[Json::s("id")])),
                r#"{"mode":"none","witness":"none"}"#,
                "relkind {relkind}"
            );
        }
    }

    /// La cara `I` declara **lo que `sql.rs` sabe traducir**, no lo que
    /// PostgreSQL sabe hacer. Declarar de más se paga donde menos se ve: un
    /// filtro que el driver no sabe poner se cae de la petición, y la consulta
    /// devuelve más filas de las que pidió sin que nadie vea un error.
    #[test]
    fn la_cara_de_lectura_no_promete_lo_que_no_traduce() {
        let j = reads().jcs();
        assert_eq!(j, r#"{"fullScan":"cheap","predicatePushdown":["eq"]}"#);
        for inventado in ["neq", "range", "isNull", "like", "fullText", "gt"] {
            assert!(!j.contains(inventado), "{j} promete `{inventado}`");
        }
    }
}
