//! El **protocolo del driver**: la petición que el motor manda y la fila que
//! vuelve.
//!
//! La petición es un **fragmento del plan**, no SQL
//! (`docs/decisions/0008-el-protocolo-del-driver.md`): la misma para todos los
//! drivers, y **traducir es del driver**. Si viajara SQL, el ejecutor tendría
//! que conocer el dialecto de cada origen, y añadir una familia de fuentes
//! dejaría de ser un binario nuevo para ser un cambio en el planificador.
//!
//! # Por qué el SQL se construye en una función pura
//!
//! Porque *«el SQL emitido contiene solo las columnas proyectadas»* tiene que
//! ser un **aserto** y no una promesa, y un aserto que exigiera un servidor no
//! se ejecutaría nunca en la suite.
//!
//! Y ahí es donde la máscara se hace efectiva: una propiedad `redact` no está en
//! el plan, luego no está en la petición, luego **no puede estar en el SQL**. La
//! salvaguarda es estructural — no hay ningún punto donde alguien pueda
//! olvidarse de aplicarla, porque no hay nada que aplicar.
//!
//! # Por qué esto es un crate y no un módulo
//!
//! Vivía dentro de `ore-read-postgres`, y al escribir el segundo driver quedó
//! claro qué significaba eso: la petición es **el contrato** entre el motor y
//! cualquier fuente, y un contrato repetido en cada implementación es un
//! contrato que diverge en la tercera.
//!
//! Lo que **no** vive aquí es la traducción. `sql()` se queda en el driver de
//! PostgreSQL porque es de PostgreSQL, y el driver de ficheros no tiene nada
//! parecido — que es justamente la prueba de que la petición no era SQL.
//!
//! # El analizador que no hizo falta
//!
//! La petición es JSON, y **JSON es un subconjunto de YAML**: la lee el mismo
//! `ore_core::parse` que lee los documentos. Añadir un analizador de JSON para
//! esto habría sido la segunda gramática para la misma forma.

/// La forma del **catálogo**: lo que un lector emite y lo que `discover`
/// acepta. Vivía dentro del consumidor, que es la manera de tener un contrato
/// sin tenerlo — ver su cabecera.
pub mod catalogo;

/// Los **bytes de un objeto**, del lector al almacén (0046 E8·2): el flujo de
/// tramas de `bajar` y `blobs`.
pub mod tramas;

pub mod capacidades;
/// **El conector v2** (ADR 0053 F2·0, `docs/federation.md` §1): los errores
/// tipados, lo que un conector declara y el bucle de `servir`.
pub mod fallo;
pub mod servir;

pub use fallo::{Codigo, Fallo, tapar};

/// Un filtro de la petición: `columna operador valor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filtro {
    pub columna: String,
    pub operador: String,
    pub valor: Valor,
}

/// Lo que lleva un filtro a la derecha: un valor (`eq`, `lt`, `like`…), una
/// lista (`in`) o nada (`isNull`, `isNotNull`). Cada operador lleva **la
/// suya** y ninguna otra: [`leer_peticion`] rechaza el resto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Valor {
    Uno(String),
    Lista(Vec<String>),
    Ninguno,
}

impl Filtro {
    /// `columna operador valor`, con un valor.
    pub fn uno(columna: &str, operador: &str, valor: &str) -> Filtro {
        Filtro {
            columna: columna.into(),
            operador: operador.into(),
            valor: Valor::Uno(valor.into()),
        }
    }

    /// El valor, si es uno.
    pub fn valor(&self) -> Option<&str> {
        match &self.valor {
            Valor::Uno(v) => Some(v),
            _ => None,
        }
    }
}

/// Un criterio de orden de la petición (`orderBy`). Los nulos van **al
/// final** en los dos sentidos, que es lo que hace DuckDB, el motor que recibe
/// las filas: un `ORDER BY … LIMIT n` empujado tiene que dar las mismas n que
/// daría el motor (`docs/federation.md` §1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orden {
    pub columna: String,
    pub descendente: bool,
}

/// Lo que el motor pide. Nombres físicos ya resueltos: el driver no conoce el
/// modelo, solo el objeto y sus columnas.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Peticion {
    pub url: String,
    pub objeto: String,
    /// propiedad → columna. La clave es lo que sale en la fila; el valor, lo que
    /// va en el `SELECT`.
    pub proyeccion: Vec<(String, String)>,
    pub clave_columnas: Vec<String>,
    pub claves: Vec<Vec<String>>,
    /// `(columna, operador, valor)`.
    ///
    /// Dos operadores, y la asimetría tiene motivo. Un **ámbito** solo produce
    /// `eq`, y eso está cerrado en `v1alpha3/02-ruleset` §4.2.2 porque su lado
    /// derecho es **un atributo del principal**: con una comparación de orden, la
    /// presencia de una fila revelaría algo que el principal no traía.
    ///
    /// La **marca de agua** no tiene principal. Es el progreso del propio motor
    /// al refrescar, no depende de quién pregunta y no puede filtrar por nadie.
    /// Por eso `gt` es admisible aquí y no allí.
    ///
    /// **Desde el protocolo 2** (ADR 0053) son los diez de [`OPERADORES`]: la
    /// lectura en vivo empuja lo que el plan de quien consulta filtra, y eso ya
    /// no es sólo un ámbito ni una marca de agua. La asimetría de arriba sigue
    /// siendo del ámbito, que sigue produciendo sólo `eq`.
    pub filtros: Vec<Filtro>,

    // ── Lo que el protocolo 2 añade · ADR 0053 ──────────────────────────────
    /// Quién es esta petición dentro de un `servir`: su respuesta lo repite.
    pub id: Option<String>,
    /// Como mucho tantas filas. Sólo llega si nada de lo que queda en el motor
    /// quita filas antes (v1alpha24 §3).
    pub limit: Option<u64>,
    /// `orderBy`: en este orden, con los nulos al final.
    pub orden: Vec<Orden>,
    /// El tiempo que tiene la lectura, aplicado **en el origen**.
    pub timeout_ms: Option<u64>,

    /// **En qué forma quiere las filas quien pide** (ADR 0043): `arrow` pide un
    /// flujo Arrow IPC por stdout, con los campos de la proyección como
    /// nombres. Es una preferencia, no una exigencia: un driver que no sabe, o
    /// que para esta petición no puede, contesta en texto —una fila JSON por
    /// línea— y quien lee distingue las dos por el primer byte.
    pub formato: Option<String>,

    // ── El rango · ADR 0016 B ───────────────────────────────────────────────
    //
    // **Y por qué estos tres se llaman en inglés cuando los de arriba no.**
    //
    // Porque la industria ya les puso nombre a estos y a los de arriba no.
    // Iceberg lee con `start-snapshot-id` y `end-snapshot-id`; Delta con
    // `startingVersion`; BigQuery con `start_timestamp` y `end_timestamp`; y a
    // la columna que ordena el avance, Airbyte y medio sector la llaman *cursor
    // field*. Quien escriba un driver nuevo viene de ahí.
    //
    // La regla, dicha una vez: **donde la industria tiene un nombre, se usa el
    // suyo; donde no, el nuestro.** Es la misma que la gramática de OOS ya
    // sigue —`witness`, `mode`, `retention`, `predicatePushdown`— y la misma por
    // la que `changes.mode` habla con el vocabulario de Flink.
    /// Desde dónde. **Exclusivo**: lo que ya estaba en la copia anterior no se
    /// vuelve a pedir. `None` es *desde el principio*.
    pub start: Option<String>,
    /// Hasta dónde. `None` es *hasta donde estés ahora*, que es lo que hacen
    /// Iceberg y Delta al omitirlo.
    ///
    /// Con él, **el desfase desaparece**: el testigo y las filas son el mismo
    /// instante, y la copia puede ser atómica. Sin él se acepta que lo ocurrido
    /// durante la lectura se re-entregue en el refresco siguiente.
    pub end: Option<String>,
    /// La columna que ordena el avance, cuando el testigo es `witness: field`.
    /// **`None` significa que el rango es sobre la posición del propio origen**
    /// —un LSN, un snapshot— y no sobre ninguna columna.
    ///
    /// Es el *cursor field* del sector, con su nombre.
    pub cursor: Option<String>,

    // ── Los ficheros · 0046 E6 ──────────────────────────────────────────────
    /// **Cómo se leen los ficheros**, cuando el objeto es una `Table` con
    /// `format` (v1alpha16 `03` §1). Viene del árbol —lo que el catálogo dedujo
    /// y alguien confirmó— para que el driver **no vuelva a adivinarlo**: un
    /// formato que se deduce al leer deja de ser el escrito. `None` para
    /// cualquier otro objeto; los drivers que no leen ficheros no lo miran.
    pub fichero: Option<Fichero>,
}

/// El `format` de una `Table` de ficheros, y los tipos congelados de sus
/// columnas (v1alpha16 `03` §1 y §1.1).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Fichero {
    /// `parquet`, `csv` o `jsonl`.
    pub tipo: String,
    /// El *glob* sobre la clave relativa al prefijo.
    pub patron: Option<String>,
    /// Las claves `k=v` del camino que son columnas.
    pub particiones: Vec<String>,
    /// Sólo csv. Por defecto, sí.
    pub cabecera: bool,
    /// Sólo csv. Por defecto, `,`.
    pub separador: char,
    /// Sólo csv. Por defecto, `utf-8`.
    pub codificacion: Option<String>,
    /// Columna física → tipo de OOS, tal como la tabla lo declara. Una columna
    /// sin tipo no está: es texto.
    pub tipos: Vec<(String, String)>,
}

impl Fichero {
    /// Si la tabla declara la columna rescatada (`03` §1.1): lo que no encaja
    /// va ahí y la lectura sigue; si no, lo que no encaja la para.
    pub fn rescata(&self) -> bool {
        self.tipos
            .iter()
            .any(|(c, _)| c == ore_core::document::COLUMNA_RESCATADA)
    }
}

fn fichero_de(n: &ore_core::parse::Node) -> Result<Fichero, String> {
    let f = n
        .get("format")
        .map(|(_, v)| v)
        .ok_or("`fichero` sin `format`")?;
    let cadena = |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let tipo = cadena("type").ok_or("`fichero.format` sin `type`")?;
    let separador = match cadena("delimiter") {
        None => ',',
        Some(s) => {
            let mut cs = s.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) => c,
                _ => return Err(format!("`delimiter: {s}` no es un carácter")),
            }
        }
    };
    Ok(Fichero {
        tipo,
        patron: cadena("match"),
        particiones: f
            .get("partitions")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        cabecera: cadena("header").is_none_or(|h| h != "false"),
        separador,
        codificacion: cadena("encoding"),
        // `[[columna, tipo], …]`, en el orden de la tabla: sin cabecera, el
        // orden es lo que dice qué campo es qué columna.
        tipos: n
            .get("tipos")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|par| match par.items() {
                        [c, t] => Some((c.as_str()?.to_string(), t.as_str()?.to_string())),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// **El cuarto verbo: ¿responde esta fuente?**
///
/// Los otros tres piden algo —un catálogo, unas filas, un ordinal— y por eso
/// los tres fallan por el mismo sitio cuando la fuente no está: al primer
/// intento de usarla, y con el error del trabajo que se estaba haciendo. Una
/// credencial caducada se descubría diciendo *«el catálogo no analiza»*.
///
/// Esto separa las dos preguntas, que es lo mismo que `source add` hace con el
/// secreto y `discover` con leer y proponer: **fallan por separado, así que se
/// piden por separado.** Airbyte tiene `check` por la misma razón.
///
/// La petición es la coordenada de [`leer_coordenada`] —basta `url`— y la
/// respuesta es esto: `{"ok": true}` o `{"ok": false, "porque": "…"}`. El
/// motivo va **literal**, porque el mensaje del servidor es lo único accionable
/// que existe: «password authentication failed» se arregla solo en cuanto se
/// lee, y resumirlo convierte cinco minutos en una tarde.
pub fn comprobacion(ok: bool, porque: Option<&str>) -> String {
    let mut o = std::collections::BTreeMap::new();
    o.insert("ok".to_string(), ore_core::json::Json::Bool(ok));
    if let Some(p) = porque {
        o.insert("porque".to_string(), ore_core::json::Json::s(p));
    }
    ore_core::json::Json::Obj(o).jcs()
}

/// **Los operadores que una petición sabe expresar.** Uno solo.
///
/// Es la lista que decide tres cosas que hasta ahora se decidían por separado y
/// podían discrepar: qué puede llevar una petición, qué traduce
/// [`ore_sql`](https://docs.rs/ore-sql) —que tiene su propia copia y una prueba
/// que la coteja contra esta, porque depende de este crate y no al revés— y qué
/// puede declarar un catálogo en `reads.predicatePushdown`.
///
/// Declarar de más en el catálogo no es optimismo: el planificador cuenta con
/// que el origen recorta, calcula menos residuo, y lo que llega es más de lo
/// pedido.
///
/// **Diez desde el protocolo 2** (ADR 0053 F2·0): el vocabulario de
/// `reads.predicatePushdown` desplegado —`range` es `lt/le/gt/ge`, `null` es
/// `isNull/isNotNull`—. Que la petición los sepa llevar no dice que un
/// conector los sepa poner: eso lo declara cada uno en sus
/// [`capacidades`], y uno que recibe lo que no declaró se niega
/// (`docs/federation.md` §1.2).
/// **Lo que la inducción declara en `reads.predicatePushdown`** de una tabla
/// de un conector v2 (decisión de 0053 F5·3, con el usuario): las familias
/// que el conector sabe poner —los diez [`OPERADORES`]— **menos `like`**, que
/// suele ser caro en el origen (un `%x%` sin índice); el dueño lo añade si lo
/// quiere. Es la política por defecto, no un techo: la tabla se edita.
pub const EMPUJE_INDUCIDO: &[&str] = &["eq", "neq", "in", "range", "isNull"];

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

/// Un filtro de la petición, o por qué no lo es.
fn filtro_de(f: &ore_core::parse::Node) -> Result<Filtro, String> {
    use ore_core::parse::Node;
    let op = f
        .get("operador")
        .and_then(|(_, o)| o.as_str())
        .unwrap_or("eq");
    if !OPERADORES.contains(&op) {
        return Err(format!(
            "`{op}` no es un operador que esta petición sepa expresar. Los que hay son {}. \
             Servir la petición sin ese filtro devolvería más filas de las pedidas y no \
             fallaría, así que no se sirve",
            OPERADORES.join(", ")
        ));
    }
    let Some(columna) = f.get("columna").and_then(|(_, c)| c.as_str()) else {
        return Err("un filtro sin `columna` no dice qué recortar".into());
    };
    let valor = match (op, f.get("valor").map(|(_, v)| v)) {
        ("isNull" | "isNotNull", None) => Valor::Ninguno,
        ("isNull" | "isNotNull", Some(_)) => {
            return Err(format!("`{op}` sobre `{columna}` no lleva valor"));
        }
        ("in", Some(Node::Sequence { items, .. })) => Valor::Lista(
            items
                .iter()
                .map(|i| {
                    i.as_str().map(String::from).ok_or_else(|| {
                        format!("la lista de `in` sobre `{columna}` lleva algo que no es un valor")
                    })
                })
                .collect::<Result<_, _>>()?,
        ),
        ("in", _) => return Err(format!("`in` sobre `{columna}` lleva una lista")),
        (_, Some(v)) => Valor::Uno(
            v.as_str()
                .ok_or_else(|| format!("`{op}` sobre `{columna}` lleva un valor, no una lista"))?
                .to_string(),
        ),
        (_, None) => {
            return Err(format!(
                "un filtro `{op}` sobre `{columna}` sin `valor` no dice qué recortar"
            ));
        }
    };
    Ok(Filtro {
        columna: columna.to_string(),
        operador: op.to_string(),
        valor,
    })
}

/// Un entero sin signo de la petición, o por qué no lo es.
fn natural(n: &ore_core::parse::Node, k: &str) -> Result<Option<u64>, String> {
    match n.get(k).map(|(_, v)| v) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Some)
            .ok_or_else(|| format!("`{k}` es un entero sin signo")),
    }
}

pub fn leer_peticion(texto: &str) -> Result<Peticion, String> {
    let n = ore_core::parse::parse(texto).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    let cadena = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let lista = |k: &str| -> Vec<String> {
        n.get(k)
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };

    let proyeccion: Vec<(String, String)> = n
        .get("proyeccion")
        .map(|(_, v)| {
            v.entries()
                .iter()
                .filter_map(|(k, c)| Some((k.as_str()?.to_string(), c.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let claves: Vec<Vec<String>> = n
        .get("claves")
        .map(|(_, v)| {
            v.items()
                .iter()
                .map(|t| {
                    t.items()
                        .iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();

    // Vocabulario cerrado. Un operador que no está aquí **no se ignora: se
    // descarta la petición entera**, y esto devuelve un error.
    //
    // Y hasta hoy no era verdad. Estaba escrito así y el código hacía un
    // `filter_map` con `return None`, que **descarta ese filtro y se queda con
    // los demás** — exactamente la dirección insegura que el comentario decía
    // evitar: la consulta devuelve más filas de las que se pidieron y nadie ve
    // un error. Lo destapó juntar la traducción de los dos drivers y preguntar
    // quién manda sobre lo que se puede empujar.
    let filtros: Vec<Filtro> = n
        .get("filtros")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .map(filtro_de)
        .collect::<Result<_, _>>()?;

    let mut orden: Vec<Orden> = Vec::new();
    for o in n.get("orderBy").map(|(_, v)| v.items()).unwrap_or(&[]) {
        let columna = o
            .get("columna")
            .and_then(|(_, c)| c.as_str())
            .ok_or("un `orderBy` sin `columna` no dice por qué ordenar")?;
        let descendente = match o.get("direccion").and_then(|(_, d)| d.as_str()) {
            None | Some("asc") => false,
            Some("desc") => true,
            Some(d) => return Err(format!("`direccion: {d}` no es `asc` ni `desc`")),
        };
        orden.push(Orden {
            columna: columna.to_string(),
            descendente,
        });
    }

    let opcional = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let p = Peticion {
        url: cadena("url"),
        objeto: cadena("objeto"),
        proyeccion,
        clave_columnas: lista("claveColumnas"),
        claves,
        filtros,
        start: opcional("start"),
        end: opcional("end"),
        cursor: opcional("cursor"),
        formato: opcional("formato"),
        fichero: match n.get("fichero") {
            None => None,
            Some((_, f)) => Some(fichero_de(f)?),
        },
        id: opcional("id"),
        limit: natural(&n, "limit")?,
        orden,
        timeout_ms: natural(&n, "timeoutMs")?,
    };
    if p.objeto.is_empty() {
        return Err("la petición no nombra ningún objeto".into());
    }
    if p.proyeccion.is_empty() {
        // Un plan con proyección vacía **no llega a lanzar el driver**, así que
        // si llega es que alguien construyó la petición a mano.
        return Err("la proyección está vacía: no hay nada que pedir".into());
    }
    Ok(p)
}

/// Una fila, como objeto JSON con las **propiedades** como claves.
///
/// No las columnas físicas: el nombre físico es del binding y no tiene por qué
/// salir del driver.
/// **Lo que un driver DEBE hacer con un rango que no sabe servir: negarse.**
///
/// Es la mitad que hace que el campo valga. Un driver que reciba `start` y lo
/// ignore devuelve **otras filas de las que se pidieron** — todas en vez del
/// incremento— y eso **no falla: se sirve**. La copia sale con filas de más, la
/// consulta responde, los números salen, y nadie ve nada.
///
/// Por eso la comprobación se escribe una vez aquí y no en cada driver: la que
/// se repite en tres sitios es la que falta en el cuarto.
///
/// Devuelve el mensaje del rechazo, o `None` si se puede servir. `puedo` es lo
/// que el driver sabe hacer: `cursor` si sabe recortar por una columna,
/// `posicion` si sabe leer un rango del changelog del origen.
pub fn rango_servible(p: &Peticion, puedo_cursor: bool, puedo_posicion: bool) -> Option<String> {
    if p.start.is_none() && p.end.is_none() {
        return None;
    }
    match (&p.cursor, puedo_cursor, puedo_posicion) {
        // Un rango sobre una columna, y este driver sabe recortar por columna.
        (Some(_), true, _) => None,
        (Some(c), false, _) => Some(format!(
            "se pidió un rango sobre `{c}` y este driver no sabe recortarlo. Devolver las filas de \
             ahora sería servir de más sin que nadie lo note, así que no se sirve"
        )),
        // Sin `cursor`, el rango es sobre la posición del propio origen.
        (None, _, true) => None,
        (None, _, false) => Some(
            "se pidió un rango sobre la posición del origen y este driver no sabe leer su \
             changelog: solo sabe leer el estado presente. Devolverlo entero sería servir de más"
                .into(),
        ),
    }
}

/// **La petición del tercer verbo: una coordenada, no un fragmento de plan.**
///
/// `{"objeto": "...", "url": "..."}` y nada más. Se lee aparte de
/// [`leer_peticion`] y no reusándola, y el motivo salió al construirlo: aquella
/// **rechaza una proyección vacía**, con razón — *«el plan que la produjera no
/// habría llegado a lanzar el driver»*. Pero preguntar hasta dónde está un
/// origen no proyecta nada, así que reusarla obligaría a mandar una proyección
/// de mentira para pasar una comprobación que aquí no aplica.
///
/// Dos formas para dos preguntas, y cada una con la validación de la suya.
pub fn leer_coordenada(texto: &str) -> Result<(String, String), String> {
    let n = ore_core::parse::parse(texto).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    let cadena = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let (url, objeto) = (cadena("url"), cadena("objeto"));
    if url.is_empty() {
        return Err("la petición no trae `url`: no hay a dónde preguntar".into());
    }
    Ok((url, objeto))
}

/// **La respuesta del tercer verbo: hasta dónde está el origen ahora.**
///
/// Normativo: [ADR 0016](../../../docs/decisions/0016-el-testigo-y-el-rango.md),
/// decisión A. La petición es la misma `Peticion` —basta `url` y `objeto`— y
/// esto es lo que vuelve: un ordinal y **nada más**.
///
/// # Por qué un verbo y no un campo de los otros dos
///
/// Porque las tres cosas caducan a ritmos distintos: el catálogo cuando alguien
/// altera la tabla, las filas en cada consulta, y **el testigo en cada
/// confirmación**. Meterlo en `leer` sería peor que en `catalogo`: llegaría
/// **con** las filas, y quien pregunta lo hace para decidir **si hace falta
/// leerlas**.
///
/// # El vocabulario no se inventa
///
/// Es el de `changes.witness` de la tabla — `none`, `snapshot`, `log`, `field`—
/// y los cuatro son **ordinales**: quien los recibe los compara, no los
/// interpreta ni los convierte.
///
/// `valor: None` con `modo: "none"` es la respuesta de un origen que **no sabe
/// fecharse**, y es una respuesta cierta. Devolver «ahora» inventaría una marca
/// que el origen no respalda.
pub fn testigo(modo: &str, valor: Option<&str>) -> String {
    let mut o = std::collections::BTreeMap::new();
    o.insert("modo".to_string(), ore_core::json::Json::s(modo));
    if let Some(v) = valor {
        o.insert("valor".to_string(), ore_core::json::Json::s(v));
    }
    ore_core::json::Json::Obj(o).jcs()
}

pub fn fila(p: &Peticion, valores: &[Option<String>]) -> String {
    // `null` y la cadena vacía no son lo mismo, y un driver que los confundiera
    // haría indistinguible «no hay dato» de «hay dato y está vacío». El JSON de
    // este árbol no tiene `null` (ADR 0002): **un nulo es la propiedad
    // AUSENTE de la fila**, que es lo que el almacén ya entiende (`carga.rs`:
    // «una columna ausente es un hueco, y aquí se escribe como nulo») y lo que
    // la semántica de vistas hace (`EsNulo` sobre lo que no está). Esto emitía
    // `""` para un nulo, y se midió en `demo` (0027 P1 I5): un `Integer` nulo
    // llegaba como `""` y la copia, con razón, no inventaba la conversión.
    let obj: std::collections::BTreeMap<String, ore_core::json::Json> = p
        .proyeccion
        .iter()
        .zip(valores)
        .filter_map(|((prop, _), v)| {
            v.as_ref()
                .map(|x| (prop.clone(), ore_core::json::Json::s(x.as_str())))
        })
        .collect();
    ore_core::json::Json::Obj(obj).jcs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peticion() -> Peticion {
        Peticion {
            url: "postgres://x".into(),
            objeto: "public.employees".into(),
            proyeccion: vec![
                ("baseSalary".into(), "base_pay".into()),
                ("employeeId".into(), "employee_id".into()),
            ],
            clave_columnas: vec!["employee_id".into()],
            claves: vec![vec!["emp-7".into()], vec!["emp-9".into()]],
            filtros: vec![Filtro::uno("cost_center", "eq", "finanzas")],
            ..Default::default()
        }
    }

    /// **Un operador que no está en el vocabulario descarta la petición
    /// entera**, y no solo ese filtro.
    ///
    /// La diferencia es la que separa un error de una fuga: quedarse con los
    /// demás filtros devuelve más filas de las pedidas, la consulta responde,
    /// los números salen y nadie ve nada.
    #[test]
    fn un_operador_desconocido_descarta_la_peticion_y_no_solo_su_filtro() {
        let texto = r#"{"objeto":"t","url":"x://y","proyeccion":{"a":"c"},
            "filtros":[{"columna":"pais","operador":"between","valor":"ES"},
                       {"columna":"cc","operador":"eq","valor":"finanzas"}]}"#;
        let e = leer_peticion(texto).expect_err("se niega");
        assert!(e.contains("`between`"), "{e}");
        assert!(e.contains("mas filas") || e.contains("más filas"), "{e}");
    }

    /// Y uno que sí está pasa, con los suyos.
    #[test]
    fn los_operadores_del_vocabulario_pasan() {
        let texto = r#"{"objeto":"t","url":"x://y","proyeccion":{"a":"c"},
            "filtros":[{"columna":"m","operador":"gt","valor":"7"},
                       {"columna":"cc","operador":"eq","valor":"f"}]}"#;
        let p = leer_peticion(texto).expect("analiza");
        assert_eq!(p.filtros.len(), 2, "{:?}", p.filtros);
    }

    /// **El protocolo 2** (ADR 0053): cada operador con la forma de su valor,
    /// `limit`, `orderBy`, `timeoutMs` y el `id` de `servir`.
    #[test]
    fn la_peticion_v2_lleva_listas_nulos_limite_orden_y_tiempo() {
        let p = leer_peticion(
            r#"{"id":"r-1","objeto":"olist.customers","url":"x://y",
                "proyeccion":{"id":"customer_id"},
                "filtros":[{"columna":"estado","operador":"in","valor":["SP","RJ"]},
                           {"columna":"baja","operador":"isNull"},
                           {"columna":"alta","operador":"ge","valor":"2026-01-01"},
                           {"columna":"nombre","operador":"like","valor":"Jo_%"},
                           {"columna":"zona","operador":"in","valor":[]}],
                "limit":1000,
                "orderBy":[{"columna":"customer_id","direccion":"desc"},{"columna":"alta"}],
                "timeoutMs":30000}"#,
        )
        .expect("analiza");
        assert_eq!(p.id.as_deref(), Some("r-1"));
        assert_eq!(
            p.filtros[0].valor,
            Valor::Lista(vec!["SP".into(), "RJ".into()])
        );
        assert_eq!(p.filtros[1].valor, Valor::Ninguno);
        assert_eq!(p.filtros[2], Filtro::uno("alta", "ge", "2026-01-01"));
        assert_eq!(p.filtros[3].valor(), Some("Jo_%"));
        assert_eq!(
            p.filtros[4].valor,
            Valor::Lista(vec![]),
            "un `in` vacío es una lista"
        );
        assert_eq!(p.limit, Some(1000));
        assert_eq!(
            p.orden,
            [
                Orden {
                    columna: "customer_id".into(),
                    descendente: true
                },
                Orden {
                    columna: "alta".into(),
                    descendente: false
                },
            ]
        );
        assert_eq!(p.timeout_ms, Some(30000));
    }

    /// **Cada operador lleva lo suyo y nada más**: un `in` con un valor suelto,
    /// un `isNull` con valor o un `eq` con una lista no se interpretan, se
    /// rechazan. Interpretarlos sería adivinar qué filas se querían.
    #[test]
    fn un_valor_que_no_es_el_de_su_operador_se_rechaza() {
        let con = |f: &str| {
            leer_peticion(&format!(
                r#"{{"objeto":"t","url":"x","proyeccion":{{"a":"a"}},"filtros":[{f}]}}"#
            ))
        };
        for (f, dice) in [
            (r#"{"columna":"a","operador":"in","valor":"ES"}"#, "lista"),
            (
                r#"{"columna":"a","operador":"isNull","valor":"x"}"#,
                "no lleva valor",
            ),
            (
                r#"{"columna":"a","operador":"eq","valor":["x"]}"#,
                "no una lista",
            ),
            (r#"{"columna":"a","operador":"lt"}"#, "sin `valor`"),
            (r#"{"operador":"eq","valor":"x"}"#, "sin `columna`"),
        ] {
            let e = con(f).expect_err(f);
            assert!(e.contains(dice), "{f}: {e}");
        }
        let mal = |k: &str| {
            leer_peticion(&format!(
                r#"{{"objeto":"t","url":"x","proyeccion":{{"a":"a"}},{k}}}"#
            ))
        };
        assert!(mal(r#""limit":-1"#).is_err());
        assert!(mal(r#""timeoutMs":"pronto""#).is_err());
        assert!(mal(r#""orderBy":[{"columna":"a","direccion":"arriba"}]"#).is_err());
    }

    /// **Una petición v1 se lee igual que antes**: sin los campos nuevos, todo
    /// lo nuevo queda vacío y nadie que pida como antes nota nada.
    #[test]
    fn una_peticion_v1_queda_como_era() {
        let p = leer_peticion(
            r#"{"objeto":"t","url":"x","proyeccion":{"a":"a"},
                "filtros":[{"columna":"m","operador":"gt","valor":"7"}]}"#,
        )
        .expect("analiza");
        assert_eq!(p.filtros, [Filtro::uno("m", "gt", "7")]);
        assert_eq!((p.id, p.limit, p.timeout_ms), (None, None, None));
        assert!(p.orden.is_empty());
    }

    /// La petición es JSON, y la lee el mismo analizador que los documentos.
    #[test]
    fn la_peticion_se_lee_con_el_analizador_de_siempre() {
        let texto = r#"{"claveColumnas":["employee_id"],"claves":[["emp-7"]],
            "filtros":[{"columna":"cost_center","operador":"eq","valor":"finanzas"}],
            "objeto":"public.employees","proyeccion":{"baseSalary":"base_pay"},
            "url":"postgres://x"}"#;
        let p = leer_peticion(texto).expect("analiza");
        assert_eq!(p.objeto, "public.employees");
        assert_eq!(
            p.proyeccion,
            vec![("baseSalary".to_string(), "base_pay".to_string())]
        );
        assert_eq!(p.claves, vec![vec!["emp-7".to_string()]]);
        assert_eq!(
            p.filtros,
            vec![Filtro::uno("cost_center", "eq", "finanzas")]
        );
    }

    /// Una proyección vacía no es una petición: el plan que la produjera no
    /// habría llegado a lanzar el driver.
    #[test]
    fn una_proyeccion_vacia_se_rechaza() {
        let texto = r#"{"objeto":"t","proyeccion":{},"url":"x"}"#;
        assert!(leer_peticion(texto).is_err());
    }

    /// La fila sale con **propiedades**, no con columnas físicas.
    #[test]
    fn la_fila_habla_el_vocabulario_del_modelo() {
        let p = peticion();
        let f = fila(&p, &[Some("1000".into()), Some("emp-7".into())]);
        assert_eq!(f, r#"{"baseSalary":"1000","employeeId":"emp-7"}"#);
        assert!(!f.contains("base_pay"), "{f}");
    }
}

#[cfg(test)]
mod ficheros {
    use super::*;

    /// **El `format` de la tabla llega entero, con sus tipos**, y lo que no
    /// dice toma el valor por defecto de la spec: cabecera sí, coma.
    #[test]
    fn el_formato_de_la_tabla_llega_con_sus_tipos() {
        let p = leer_peticion(
            r#"{"url":"s3://b","objeto":"v/pedidos/","proyeccion":{"id":"id"},
                "fichero":{"format":{"type":"csv","delimiter":";","partitions":["fecha"]},
                           "tipos":[["id","String"],["total","Decimal<12, 2>"],["_rescued_data","String"]]}}"#,
        )
        .expect("petición");
        let f = p.fichero.expect("fichero");
        assert_eq!(f.tipo, "csv");
        assert_eq!(f.separador, ';');
        assert!(f.cabecera);
        assert_eq!(f.particiones, ["fecha"]);
        assert!(f.rescata());
        assert_eq!(
            f.tipos[1],
            ("total".to_string(), "Decimal<12, 2>".to_string())
        );
        let sin = leer_peticion(r#"{"url":"s3://b","objeto":"x","proyeccion":{"id":"id"}}"#)
            .expect("petición");
        assert!(
            sin.fichero.is_none(),
            "una petición sin ficheros queda como era"
        );
    }
}

#[cfg(test)]
mod rango {
    use super::*;

    fn p(start: Option<&str>, cursor: Option<&str>) -> Peticion {
        Peticion {
            start: start.map(String::from),
            cursor: cursor.map(String::from),
            ..Default::default()
        }
    }

    /// **Sin rango, nada que comprobar.** Un driver que no sepa servir rangos
    /// sigue sirviendo todas las peticiones que no lo llevan.
    #[test]
    fn una_peticion_sin_rango_la_sirve_cualquiera() {
        assert_eq!(rango_servible(&p(None, None), false, false), None);
    }

    /// **Y la que sí lo lleva se rechaza si no se puede honrar.**
    ///
    /// Es la mitad que hace que el campo valga. Ignorarlo devolvería las filas
    /// de ahora en vez del incremento, y eso **no falla: se sirve** — la copia
    /// sale con filas de más y nadie ve nada.
    #[test]
    fn un_rango_que_no_se_puede_honrar_se_rechaza_y_dice_por_que() {
        // Sobre una columna, y el driver no sabe recortar por columna.
        let e = rango_servible(&p(Some("100"), Some("updated_at")), false, true).expect("rechaza");
        assert!(e.contains("updated_at"), "{e}");
        assert!(e.contains("servir de más"), "{e}");

        // Sobre la posición del origen, y el driver solo lee el estado presente.
        let e = rango_servible(&p(Some("0/1A2B"), None), true, false).expect("rechaza");
        assert!(e.contains("changelog"), "{e}");
    }

    /// Y cada driver declara lo suyo: los dos casos que sí se sirven.
    #[test]
    fn cada_driver_declara_lo_que_sabe() {
        assert_eq!(
            rango_servible(&p(Some("100"), Some("t")), true, false),
            None
        );
        assert_eq!(rango_servible(&p(Some("0/1A"), None), false, true), None);
    }
}
