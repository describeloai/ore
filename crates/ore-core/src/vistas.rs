//! Las vistas: la pieza que absorbe al `Binding`.
//!
//! Una vista dice **qué existe físicamente y cómo se llama**: de dónde sale
//! —una fuente declarada, u **otra vista**—, qué campos expone, qué filas son
//! suyas, qué sabe hacer el origen y, si se copia, dónde. No lleva significado:
//! `is:`, los conceptos y las etiquetas siguen en la entidad (`v1alpha7/01-view`
//! §2). Y la flecha se invierte: el binding nombraba a la entidad, y ahora **la
//! entidad nombra a la vista** con `backedBy`. Así una vista existe antes de que
//! nadie modele nada, y varias entidades pueden respaldarse de la misma sin
//! duplicarla.
//!
//! Este módulo es lo que el resto del núcleo necesita saber de una vista sin
//! abrir su documento: **su raíz** —a qué fuente y objeto llega una cadena de
//! vistas, y con qué nombres de columna— y **sus comprobaciones**. Lo que no
//! está aquí es el álgebra: el IR, el linaje por columna y el reescritor viven
//! en `ore-view`, que depende de este crate y no al revés.
//!
//! # Lo que se comprueba, y con qué código
//!
//! | | |
//! |---|---|
//! | `from.datasource`, `materialized.datasource` o el `datasource` de una tabla sin declarar | `OOS2004` — el mismo que para `datasourceRef`, porque es el mismo defecto |
//! | `from.view`, `from.table`, `backedBy`, un campo o un filtro que nombran lo que no existe | `OOS2018` |
//! | una cadena de vistas que vuelve sobre sí misma | `OOS2019` |
//! | la vista que respalda una entidad no expone su clave o sus `via` | `OOS2011` — lo que necesita columna, dicho de la vista |
//! | una vista cuya **raíz de lectura** no se deja leer y no lleva `materialized` | `OOS2020` — v1alpha8 |
//! | una copia de un flujo que solo anexa respaldando una entidad **mutable** | `OOS2021` — v1alpha8 |
//! | una propiedad de una entidad que su vista no expone, sin `derivedFrom` | `OOS2022` — v1alpha8, y la otra cara de haber retirado la federación |
//!
//! # v1alpha8 · la tabla, y por qué `OOS2018` llega ahora hasta el suelo
//!
//! Hasta v1alpha8 el puntero físico vivía **dentro** de la vista, y con él el
//! límite de lo comprobable: ningún documento decía qué columnas tenía la
//! fuente, así que la comprobación de nombres cubría el eslabón vista→vista y
//! **creía** el último tramo — el que toca el mundo. `kind: Table` declara las
//! columnas, y por eso la misma regla, con el mismo código, alcanza ahora la
//! columna física.
//!
//! Las dos versiones conviven sin condicionales repartidos: la diferencia está
//! en `Fuente`, que ahora tiene tres variantes, y en `raiz()`, que sabe llegar
//! por los dos caminos. **Todo lo de encima —`flow`, `governance`, el ejecutor,
//! `ore view`— llama a `raiz()` y no se entera**, que es lo que la absorción
//! V0–V3 compró y aquí se cobra.
//!
//! El flujo de etiquetas atraviesa la cadena en `flow`: la entidad hereda del
//! datasource **raíz** de su vista, y una vista con `materialized` instancia el
//! conducto `materialization.payload` como lo hacía el eje `payload` del binding.

use std::collections::{BTreeMap, BTreeSet};

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package};
use crate::parse::Node;

/// De dónde sale una vista: de una tabla, de otra vista, o —v1alpha7— del
/// puntero físico que la vista llevaba dentro.
///
/// Las dos primeras variantes dicen lo mismo del mundo y no son la misma cosa
/// para quien las escribe: `Datasource` es el contrato físico **repetido en
/// cada vista que toca la fuente**, y `Tabla` es el mismo contrato **nombrado
/// una vez**. Por eso la primera no se borra —un documento v1alpha7 sigue
/// compilando— y por eso no se fusionan: fusionarlas sería volver a tener un
/// sitio donde el objeto se describe y otro donde se describe otra vez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fuente {
    /// v1alpha7: `from: {datasource, object}`, el puntero dentro de la vista.
    Datasource {
        datasource: String,
        objeto: String,
    },
    /// v1alpha8: `from: {table}`, el nombre cualificado de un `kind: Table`.
    Tabla(String),
    Vista(String),
    /// v1alpha12: `from: {dataset}`, el nombre cualificado de un
    /// `kind: Dataset`. Un mantenido es un eslabón más (tiene `from`); un
    /// escrito es suelo (no lo tiene): es nuestro, y ahí terminan las cadenas.
    Dataset(String),
}

/// Cómo codifica una tabla los cambios que emite: la cara `D`.
///
/// Son exactamente las tres formas que Flink documenta de convertir una tabla
/// dinámica en un flujo, más la ausencia. El vocabulario es **cerrado** por lo
/// mismo que `predicatePushdown`: cada modo dice qué **pesos** son legales en
/// un delta, y si un perfil pudiera inventar una codificación el mantenedor no
/// podría razonar sobre los que le llegan — un delta con un peso ilegal
/// entraría sin que nadie lo notara.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// No emite cambios, o no se sabe si los emite. No se inventa.
    Ninguno,
    /// Solo altas: solo `+1`. Una marca de agua no ve borrados.
    Anexa,
    /// Un borrado retracta; una actualización retracta la vieja y añade la
    /// nueva. `-1` y `+1`. El *Change Data Feed* de Delta es esto con cuatro
    /// nombres.
    Retracta,
    /// `+1` por clave, `-1` por *tombstone*. Exige clave única.
    Upsert,
}

impl Modo {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Modo::Ninguno,
            "append" => Modo::Anexa,
            "retract" => Modo::Retracta,
            "upsert" => Modo::Upsert,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Modo::Ninguno => "none",
            Modo::Anexa => "append",
            Modo::Retracta => "retract",
            Modo::Upsert => "upsert",
        }
    }
}

/// La hoja de una cadena de vistas, ya compuesta: **a qué fuente y objeto se
/// llega**, y con qué nombre físico se pide cada campo de la vista de arriba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raiz {
    pub datasource: String,
    pub objeto: String,
    /// Campo de la vista → columna física en la raíz. La composición de los
    /// renombres de toda la cadena.
    pub columnas: BTreeMap<String, String>,
    /// Los filtros de **toda** la cadena, ya en columnas físicas de la raíz:
    /// `(columna, valores)`. Una vista sobre otra hereda las filas que la de
    /// abajo ya recortó — lo que no está en la de abajo no está en ninguna.
    pub filtros: Vec<(String, Vec<String>)>,
    /// Campo agregado → **qué agregado es**, con su columna ya bajada a física.
    /// `sobre: None` es exactamente `count()`, que no lee ninguna.
    ///
    /// **Va aparte de `columnas` y no dentro**, y la diferencia no es de
    /// estilo: `masa` no *es* `salary`, es su suma. Media docena de sitios
    /// leen `columnas` como una identidad —el destino de la copia, el tipo de
    /// un eslabón, la reescritura del registro— y meter ahí un agregado los
    /// haría afirmar que la copia guarda `salary`.
    ///
    /// Lo que sí necesita saberlo es **la etiqueta**: la suma de un sueldo
    /// clasificado sigue estando clasificada mientras nadie desclasifique, y
    /// sin este mapa la etiqueta de `salary` no llega a `masa`. Es la misma
    /// figura que `filtros` — columnas que se leen sin exponerse.
    pub agrega: BTreeMap<String, Agregado>,
    /// El nombre cualificado de la `Table` de la que sale, si sale de una.
    ///
    /// `None` en una cadena v1alpha7, donde el objeto no es un documento y no
    /// tiene nombre que dar. Quien necesite las dos caras —el planificador, el
    /// mantenedor— pregunta por aquí; quien solo necesite dónde vive el dato
    /// tiene `datasource` y `objeto` en los dos casos, y por eso no se entera.
    pub tabla: Option<String>,
}

/// Por qué una cadena no llega a una raíz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinRaiz {
    /// `from.view` nombra una vista que no existe. Lleva la que la nombró.
    NoExiste { vista: String, desde: String },
    /// `from.table` nombra una tabla que no existe. Es un caso aparte y no el
    /// mismo con otro nombre: una cadena que no llega a la tabla no llega al
    /// suelo, y el mensaje tiene que decir qué se buscaba para que se entienda
    /// que falta un documento, no un eslabón más.
    TablaNoExiste { tabla: String, desde: String },
    /// La cadena vuelve sobre sí misma. La cadena entera, para que el mensaje
    /// la enseñe.
    Ciclo(Vec<String>),
    /// Una vista sin `from` que resuelva. El esquema lo impide; esto es lo que
    /// pasa si se llega aquí sin haberlo validado.
    SinFrom(String),
    /// v1alpha14. Una vista SQL no es un eslabón de una cadena: su consulta
    /// puede leer varias fuentes, y lo que sale de cada una lo dice su linaje,
    /// no una raíz.
    Consulta(String),
}

impl Package {
    /// La vista con este nombre cualificado.
    pub fn view(&self, qname: &str) -> Option<&Loaded> {
        let qname = crate::normalize::a_corto(qname);
        self.of(Kind::View)
            .find(|d| d.qname().as_deref() == Some(qname.as_ref()))
    }

    /// Resuelve una referencia a vista **tal como la escribió el autor**: la
    /// forma corta vale dentro del mismo espacio de nombres (N1), igual que
    /// para una entidad.
    pub fn resolve_view(&self, referencia: &str, desde: &Loaded) -> Option<&Loaded> {
        self.view(&crate::link::cualificar(referencia, desde))
    }

    /// La tabla con este nombre cualificado.
    pub fn table(&self, qname: &str) -> Option<&Loaded> {
        let qname = crate::normalize::a_corto(qname);
        self.of(Kind::Table)
            .find(|d| d.qname().as_deref() == Some(qname.as_ref()))
    }

    /// Todas las tablas del paquete.
    pub fn tables(&self) -> impl Iterator<Item = &Loaded> {
        self.of(Kind::Table)
    }

    /// Resuelve una referencia a tabla con la misma regla que a una vista: la
    /// forma corta vale dentro del mismo espacio de nombres (N1).
    pub fn resolve_table(&self, referencia: &str, desde: &Loaded) -> Option<&Loaded> {
        self.table(&crate::link::cualificar(referencia, desde))
    }

    /// v1alpha12. El dataset con este nombre cualificado.
    pub fn dataset(&self, qname: &str) -> Option<&Loaded> {
        let qname = crate::normalize::a_corto(qname);
        self.of(Kind::Dataset)
            .find(|d| d.qname().as_deref() == Some(qname.as_ref()))
    }

    /// Todos los datasets del paquete.
    pub fn datasets(&self) -> impl Iterator<Item = &Loaded> {
        self.of(Kind::Dataset)
    }

    /// Resuelve una referencia a dataset con la misma regla (N1).
    pub fn resolve_dataset(&self, referencia: &str, desde: &Loaded) -> Option<&Loaded> {
        self.dataset(&crate::link::cualificar(referencia, desde))
    }

    /// Lo que una vista o un dataset tiene debajo por nombre: una vista **o un
    /// dataset**. Es la búsqueda de `backedBy` y de la cadena, y es UNA para
    /// que las dos no diverjan en qué admiten.
    pub fn resolve_lectura(&self, referencia: &str, desde: &Loaded) -> Option<&Loaded> {
        self.resolve_view(referencia, desde)
            .or_else(|| self.resolve_dataset(referencia, desde))
    }
}

/// v1alpha12. **Un dataset mantenido**: tiene `from`, el sistema cumple su
/// plan. Es lo que hasta v1alpha11 era una vista con `materialized`.
pub fn es_mantenido(d: &Loaded) -> bool {
    d.kind == Kind::Dataset && d.section("from").is_some()
}

/// v1alpha12. **Un dataset escrito**: sin `from`, lo llena código y sus
/// `columns` siguen a la tabla Iceberg. Es suelo: la cadena termina en él.
pub fn es_escrito(d: &Loaded) -> bool {
    d.kind == Kind::Dataset && d.section("from").is_none()
}

/// **Copia datos**: un dataset mantenido, o una vista de v1alpha7/8 con
/// `materialized`. Es el predicado que instancia el conducto
/// `materialization.payload` y el que mira el mantenimiento (`OOS2023`,
/// `OOS2029`): cambió de documento en v1alpha12 y no de regla.
pub fn es_copia(d: &Loaded) -> bool {
    es_mantenido(d) || (d.kind == Kind::View && d.section("materialized").is_some())
}

/// `spec.from` de una vista.
pub fn fuente(v: &Loaded) -> Option<Fuente> {
    let from = v.section("from")?;
    let q = |r: &str| crate::link::cualificar(r, v);
    if let Some((_, vista)) = from.get("view") {
        return Some(Fuente::Vista(q(vista.as_str()?)));
    }
    if let Some((_, tabla)) = from.get("table") {
        return Some(Fuente::Tabla(q(tabla.as_str()?)));
    }
    if let Some((_, dataset)) = from.get("dataset") {
        return Some(Fuente::Dataset(q(dataset.as_str()?)));
    }
    let datasource = from.get("datasource")?.1.as_str()?.to_string();
    let objeto = from
        .get("object")
        .and_then(|(_, o)| o.as_str())
        .unwrap_or("")
        .to_string();
    Some(Fuente::Datasource { datasource, objeto })
}

/// v1alpha14. **Una vista SQL**: su cuerpo es una consulta (`spec.sql`), y
/// lo que expone es su contrato (`spec.columns`).
pub fn es_sql(v: &Loaded) -> bool {
    v.kind == Kind::View && v.section("sql").is_some()
}

/// v1alpha14. El contrato de una vista SQL: los nombres de `spec.columns`.
pub fn contrato(v: &Loaded) -> BTreeSet<String> {
    columnas(v)
}

/// v1alpha14. Lo que un nombre escrito en una consulta nombra: una vista, un
/// dataset o una tabla del árbol, con la regla de nombres de siempre (una
/// parte es su schema; dos, `default`; tres, completa).
pub fn fuente_sql<'a>(pkg: &'a Package, nombre: &str, desde: &Loaded) -> Option<&'a Loaded> {
    pkg.resolve_view(nombre, desde)
        .or_else(|| pkg.resolve_dataset(nombre, desde))
        .or_else(|| pkg.resolve_table(nombre, desde))
}

/// v1alpha14. Las columnas que una fuente de una consulta deja nombrar: las de
/// una tabla, o lo que una vista o un dataset exponen.
pub fn columnas_que_expone(pkg: &Package, d: &Loaded) -> BTreeSet<String> {
    if d.kind == Kind::Table {
        columnas(d)
    } else {
        expone_en(pkg, d).into_keys().collect()
    }
}

/// v1alpha14. **La consulta de una vista SQL, analizada contra el árbol**: lo
/// que lee, lo que proyecta y su linaje a un nivel (`vista_sql`). El árbol
/// expande sus `*` y decide una columna sin calificar entre dos fuentes.
/// `None` si no es una vista SQL.
pub fn consulta(
    pkg: &Package,
    v: &Loaded,
) -> Option<Result<crate::vista_sql::Consulta, crate::vista_sql::Fallo>> {
    let sql = v.section("sql")?.as_str()?;
    let columnas_de =
        |n: &str| fuente_sql(pkg, n, v).map(|d| columnas_que_expone(pkg, d).into_iter().collect());
    Some(crate::vista_sql::analizar(sql, &columnas_de))
}

/// Lo que un documento lee **por nombre**, un eslabón: la fuente de una vista
/// estructurada o de un dataset mantenido, o lo que la consulta de una vista
/// SQL nombra. Lo que no resuelve no sale: eso lo dice `OOS2018`.
pub fn lee_directo<'a>(pkg: &'a Package, d: &'a Loaded) -> Vec<&'a Loaded> {
    if es_sql(d) {
        let Some(Ok(c)) = consulta(pkg, d) else {
            return vec![];
        };
        return c.lee.iter().filter_map(|n| fuente_sql(pkg, n, d)).collect();
    }
    match fuente(d) {
        Some(Fuente::Vista(q)) => pkg.view(&q).into_iter().collect(),
        Some(Fuente::Dataset(q)) => pkg.dataset(&q).into_iter().collect(),
        Some(Fuente::Tabla(q)) => pkg.table(&q).into_iter().collect(),
        _ => vec![],
    }
}

/// Un camino de lectura que vuelve a `desde`, si lo hay: la cadena para el
/// mensaje de `OOS2019`.
fn vuelve(pkg: &Package, desde: &Loaded) -> Option<Vec<String>> {
    fn ir<'a>(
        pkg: &'a Package,
        objetivo: &Loaded,
        d: &'a Loaded,
        camino: &mut Vec<(Kind, String)>,
    ) -> Option<Vec<String>> {
        for abajo in lee_directo(pkg, d) {
            let clave = (abajo.kind, abajo.qname().unwrap_or_default());
            if abajo.kind == objetivo.kind && abajo.qname() == objetivo.qname() {
                let mut c: Vec<String> = camino.iter().map(|(_, n)| n.clone()).collect();
                c.push(clave.1);
                return Some(c);
            }
            if camino.contains(&clave) {
                continue;
            }
            camino.push(clave);
            if let Some(c) = ir(pkg, objetivo, abajo, camino) {
                return Some(c);
            }
            camino.pop();
        }
        None
    }
    let desde = pkg
        .docs
        .iter()
        .find(|d| d.kind == desde.kind && d.qname() == desde.qname())?;
    let mut camino = vec![(desde.kind, desde.qname().unwrap_or_default())];
    ir(pkg, desde, desde, &mut camino)
}

/// Las columnas que una tabla declara.
///
/// Es lo único verdaderamente nuevo de v1alpha8, y lo que hace comprobable lo
/// que antes no lo era. El nombre es **opaco** —puede llevar puntos si el
/// origen es anidado— y por eso no es un identificador.
///
/// v1alpha12: las de un dataset escrito también, y con la misma forma.
pub fn columnas(t: &Loaded) -> BTreeSet<String> {
    t.section("columns")
        .map(|c| {
            c.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Alias de [`columnas`] para donde una variable local se llama igual.
fn columnas_de(t: &Loaded) -> BTreeSet<String> {
    columnas(t)
}

/// **El tipo de cada columna de una tabla**, el que el conector tradujo:
/// `columns.<c>.type` (0032 §3; `01-table.md` §5.0). Una columna sin `type` no
/// está en el mapa —el conector no supo traducirla, y es texto para quien la
/// lea—, y una con un `type` que no analiza tampoco: eso lo dice `OOS3xxx`, no
/// esto.
///
/// Es la única fuente del tipo para lo que cuelga de la tabla: la vista no tipa,
/// el agregado se deriva, la entidad afina. Antes el tipo bajaba SOLO de la
/// entidad, y la copia de una tabla sin entidad salía entera como texto.
pub fn tipos_de_columnas(t: &Loaded) -> BTreeMap<String, crate::types::Type> {
    t.section("columns")
        .map(|c| {
            c.entries()
                .iter()
                .filter_map(|(k, v)| {
                    let tipo = v.get("type").and_then(|(_, t)| t.as_str())?;
                    Some((
                        k.as_str()?.to_string(),
                        crate::types::parse_type(tipo).ok()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// La cara `I`: si a esta tabla se le puede pedir algo.
///
/// `reads: none` significa *no se le puede pedir nada* — un tema de Kafka se
/// escribe, no se pregunta. Es lo único que hace falta saber para `OOS2020`;
/// **qué** se le puede pedir lo lee el planificador, y eso es de `ore-cli`.
pub fn se_lee(t: &Loaded) -> bool {
    t.section("reads")
        .is_none_or(|r| r.as_str() != Some("none"))
}

/// La cara `D`: cómo codifica sus cambios. Un modo fuera del vocabulario se lee
/// como `Ninguno` **aquí** y lo rechaza la forma con `OOS1004`: esta función no
/// es el sitio donde se decide qué es legal.
pub fn modo(t: &Loaded) -> Modo {
    t.section("changes")
        .and_then(|c| c.get("mode"))
        .and_then(|(_, m)| m.as_str())
        .and_then(Modo::parse)
        .unwrap_or(Modo::Ninguno)
}

/// **Los cinco agregados de OOS.** Vocabulario cerrado, y lo es por lo mismo
/// que `changes.mode`: si un documento pudiera inventar una función, el motor
/// no sabría qué estado hace falta por grupo para mantenerla, y una copia se
/// quedaría desactualizada sin que nadie lo notase.
///
/// Son exactamente los cinco que el IR sabe incrementalizar o rechazar con un
/// motivo — `count` y `sum` con un acumulador, `min` y `max` guardando el
/// multiconjunto porque no son invertibles bajo baja, y `avg` **rechazado**.
pub const AGREGADOS: &[&str] = &["count", "sum", "min", "max", "avg"];

/// Un agregado escrito en `fields`: la función y sobre qué columna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agregado {
    pub funcion: String,
    /// `None` solo para `count`: contar filas no necesita columna.
    pub sobre: Option<String>,
}

/// **¿Este valor de `fields` es un agregado?**
///
/// El discriminante es que termine en `)`, y no es una heurística: un nombre de
/// columna no lleva paréntesis —lo dice [`es_nombre_de_columna`], que ya
/// existía—. Así que `sum(importe)` no puede ser una columna llamada así, y una
/// llamada mal escrita **no se degrada a columna**: es un error de forma.
///
/// Devuelve `None` si no es una llamada; `Some(Err)` si lo es y está mal.
pub fn agregado(valor: &str) -> Option<Result<Agregado, String>> {
    let t = valor.trim();
    if !t.ends_with(')') {
        return None;
    }
    let Some((nombre, resto)) = t.split_once('(') else {
        return Some(Err(format!("`{t}` cierra un paréntesis que no abre")));
    };
    let nombre = nombre.trim();
    let arg = resto[..resto.len() - 1].trim();
    if !AGREGADOS.contains(&nombre) {
        return Some(Err(format!(
            "`{nombre}` no es un agregado de OOS: {}",
            AGREGADOS.join(" · ")
        )));
    }
    // `count(x)` se niega en vez de admitirse. En SQL cuenta los no nulos, y
    // este motor no distingue: lo trataría como `count()` y daría OTRO NÚMERO
    // sin decirlo. Un agregado que contesta de más en silencio es peor que uno
    // que no está.
    if nombre == "count" {
        return Some(if arg.is_empty() {
            Ok(Agregado {
                funcion: nombre.to_string(),
                sobre: None,
            })
        } else {
            Err(format!(
                "`count({arg})` no: en SQL cuenta los no nulos y aquí no se distingue, \
                 así que daría otro número sin decirlo. Escribe `count()`"
            ))
        });
    }
    Some(if arg.is_empty() {
        Err(format!(
            "`{nombre}()` sin columna: solo `count` cuenta filas"
        ))
    } else if !es_nombre_de_columna(arg) {
        Err(format!("`{arg}` no es un nombre de columna"))
    } else {
        Ok(Agregado {
            funcion: nombre.to_string(),
            sobre: Some(arg.to_string()),
        })
    })
}

/// Los agregados de una vista: **nombre de salida → qué agrega**.
///
/// Van en `fields` y no en una clave aparte porque **la salida de una vista es
/// una sola lista de columnas**: repartirla en dos mapas obligaría a juntarlos
/// mentalmente para saber qué sale, dejaría a `moved` y `reserved` sin decir a
/// cuál alcanzan, y admitiría que los dos reclamasen el mismo nombre. En el IR
/// tampoco hay dos: `Proyecta.campos` es uno.
pub fn agregados(v: &Loaded) -> BTreeMap<String, Agregado> {
    let mut out = BTreeMap::new();
    let Some(fs) = v.section("fields") else {
        return out;
    };
    for (k, val) in fs.entries() {
        let (Some(nombre), Some(txt)) = (k.as_str(), val.as_str()) else {
            continue;
        };
        if let Some(Ok(a)) = agregado(txt) {
            out.insert(nombre.to_string(), a);
        }
    }
    out
}

/// **Qué campos expone una vista**, y de dónde sale cada uno *tal como está
/// escrito* — una columna, o la llamada que lo agrega.
///
/// Es la otra mitad de [`campos`], y son dos porque son dos preguntas:
///
/// | | contesta | quién pregunta |
/// |---|---|---|
/// | [`campos`] | *de qué **columna** sale este campo* | el plan, el linaje, `OOS2018` sobre la tabla, las etiquetas de raíz |
/// | `expone` | *qué campos **da** esta vista* | la entidad —`OOS2011`, `OOS2022`—, la vista de encima, `version.field` |
///
/// Antes de `groupBy` las dos daban lo mismo y bastaba una. Con un agregado
/// dejan de coincidir, y confundirlas tiene un síntoma muy concreto: una
/// entidad respaldada por una vista que agrupa recibía *«`hr.por_pais` no
/// expone `n`»* sobre una vista que **sí** lo expone.
pub fn expone(v: &Loaded) -> BTreeMap<String, String> {
    if es_sql(v) {
        return contrato(v).into_iter().map(|c| (c.clone(), c)).collect();
    }
    let mut out = campos(v);
    let Some(fs) = v.section("fields") else {
        return out;
    };
    for (k, val) in fs.entries() {
        let (Some(nombre), Some(txt)) = (k.as_str(), val.as_str()) else {
            continue;
        };
        if agregado(txt).is_some() {
            out.insert(nombre.to_string(), txt.trim().to_string());
        }
    }
    out
}

/// v1alpha12. **Lo que un documento expone, sabido con el paquete.**
///
/// Una vista lo dice sola (`expone`): sus `fields`. Un dataset **escrito** expone
/// sus `columns` con sus nombres; un **mantenido** sin `fields` expone **todo lo
/// que `from` expone, con sus nombres** —la copia identidad, que hoy no existe
/// porque una vista exige `fields`—, y con `fields` lo que ellos digan, como
/// una vista. Es contra lo que se resuelven `fields`, `where`, `groupBy` y
/// `backedBy` de quien lo lea (`OOS2018`, `OOS2011`, `OOS2022`).
pub fn expone_en(pkg: &Package, d: &Loaded) -> BTreeMap<String, String> {
    if d.kind != Kind::Dataset {
        return expone(d);
    }
    if es_escrito(d) {
        return columnas(d).into_iter().map(|c| (c.clone(), c)).collect();
    }
    if d.section("fields").is_some() {
        return expone(d);
    }
    match fuente(d) {
        Some(Fuente::Tabla(qn)) => pkg
            .table(&qn)
            .map(|t| columnas(t).into_iter().map(|c| (c.clone(), c)).collect())
            .unwrap_or_default(),
        Some(Fuente::Vista(qn)) => pkg
            .view(&qn)
            .map(|v| expone(v).into_keys().map(|k| (k.clone(), k)).collect())
            .unwrap_or_default(),
        Some(Fuente::Dataset(qn)) => pkg
            .dataset(&qn)
            .map(|x| {
                expone_en(pkg, x)
                    .into_keys()
                    .map(|k| (k.clone(), k))
                    .collect()
            })
            .unwrap_or_default(),
        _ => BTreeMap::new(),
    }
}

/// **Los seis comparadores de `having`.** Vocabulario cerrado, y el orden
/// importa al leerlo: los de dos caracteres van primero, o `>=` se leería como
/// `>` con un `=` colgando.
///
/// Aquí sí hay rangos, y en `where` no. No es una incoherencia: el `where`
/// recorta por una **columna**, y un rango sobre una columna clasificada ordena
/// en vez de particionar — ahí empieza la fuga, y por eso su gramática es
/// igualdad, pertenencia y ausencia. `having` recorta por un **agregado**, y
/// entonces el rango es justo lo que hace falta: `count() >= 8` es un umbral de
/// k-anonimidad, no un canal lateral. Y lo que el agregado lea sigue gobernado:
/// el linaje deja una arista `INDIRECT` desde la clave de grupo, así que un
/// `having` sobre `sum(salary)` arrastra la etiqueta de `salary` igual que la
/// arrastraba la suma.
pub const COMPARADORES: &[&str] = &[">=", "<=", "!=", "==", ">", "<"];

/// `">= 8"` → `(">=", "8")`. `None` si no empieza por un comparador.
pub fn condicion(txt: &str) -> Option<(&'static str, String)> {
    let t = txt.trim();
    COMPARADORES
        .iter()
        .find_map(|op| t.strip_prefix(*op).map(|resto| (*op, resto)))
        .map(|(op, resto)| (op, resto.trim().to_string()))
}

/// El `having` de una vista: **campo agregado → condición**, sin analizar.
pub fn teniendo(v: &Loaded) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(h) = v.section("having") else {
        return out;
    };
    for (k, val) in h.entries() {
        if let (Some(campo), Some(txt)) = (k.as_str(), val.as_str()) {
            out.insert(campo.to_string(), txt.trim().to_string());
        }
    }
    out
}

/// Las columnas por las que agrupa una vista, en el orden en que las declara.
pub fn agrupacion(v: &Loaded) -> Vec<String> {
    v.section("groupBy")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|i| i.as_str().map(str::to_string))
        .collect()
}

/// Campo → nombre en la fuente. Admite la forma breve y la expandida, como el
/// mapeo del binding: la canónica es la expandida.
///
/// **Un agregado no sale por aquí.** Los cinco lectores de `fields` preguntan
/// «de qué columna sale este campo», y de un agregado la respuesta es que de
/// ninguna: sale de un conjunto de filas. Devolver `count()` como si fuera un
/// nombre de columna los haría buscarla en la tabla y no encontrarla.
///
/// La forma expandida se retira en v1alpha8 —existía para llevar
/// `physicalType`, y el tipo físico lo dice ahora `columns`— y **esta función
/// sigue leyéndola**, porque sigue habiendo documentos v1alpha7 que la usan.
/// Lo que decide qué se admite es `spec_keys_en`, no esto.
pub fn campos(v: &Loaded) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(fs) = v.section("fields") else {
        return out;
    };
    for (k, val) in fs.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let col = val.as_str().map(str::to_string).or_else(|| {
            val.get("column")
                .and_then(|(_, c)| c.as_str())
                .map(str::to_string)
        });
        if let Some(col) = col
            && agregado(&col).is_none()
        {
            out.insert(nombre.to_string(), col);
        }
    }
    out
}

/// `spec.where` de una vista: `(nombre en la fuente, valores)`. La gramática es
/// la del `selector` del binding —igualdad, pertenencia, ausencia— y por lo
/// mismo: un predicado sobre una columna clasificada es un canal lateral, y
/// una partición solo revela pertenencia.
pub fn filtros(v: &Loaded) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let Some(w) = v.section("where") else {
        return out;
    };
    for (k, val) in w.entries() {
        let Some(col) = k.as_str() else { continue };
        let valores: Vec<String> = match val {
            Node::Sequence { items, .. } => items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect(),
            _ => val.as_str().map(str::to_string).into_iter().collect(),
        };
        out.push((col.to_string(), valores));
    }
    out
}

/// La entidad nombra a su vista: `spec.backedBy`, resuelta.
pub fn respaldo<'a>(pkg: &'a Package, e: &Loaded) -> Option<&'a Loaded> {
    let r = e.section("backedBy")?.as_str()?;
    pkg.resolve_lectura(r, e)
}

/// La cadena de una vista hasta su raíz, en orden: ella primero.
///
/// Es la operación que hace que *«un pipeline es una cadena de vistas»* sea
/// una estructura: componer renombres y filtros no necesita un concepto nuevo,
/// solo seguir `from.view` hasta que deje de haberlo.
pub fn cadena<'a>(pkg: &'a Package, v: &'a Loaded) -> Result<Vec<&'a Loaded>, SinRaiz> {
    let mut vistos: Vec<String> = Vec::new();
    // v1alpha12: una vista y un dataset pueden llamarse igual —la pregunta
    // sobre su dataset, `from: {dataset: <el mismo nombre>}`—, así que el
    // ciclo se mira por (kind, nombre) y no por nombre.
    let mut vistos_con_kind: Vec<(Kind, String)> = Vec::new();
    let mut fila: Vec<&Loaded> = Vec::new();
    let mut actual = v;
    loop {
        let qn = actual.qname().unwrap_or_default();
        if vistos_con_kind.contains(&(actual.kind, qn.clone())) {
            vistos.push(qn);
            return Err(SinRaiz::Ciclo(vistos));
        }
        vistos_con_kind.push((actual.kind, qn.clone()));
        vistos.push(qn.clone());
        fila.push(actual);
        match fuente(actual) {
            None if es_sql(actual) => return Err(SinRaiz::Consulta(qn)),
            // v1alpha12: un dataset escrito no tiene `from` y no le falta: es
            // suelo por derecho. Lo que se tiene no sale de nada de fuera.
            None if es_escrito(actual) => return Ok(fila),
            None => return Err(SinRaiz::SinFrom(qn)),
            // Las dos formas de tocar el suelo. Una vista NO sale de una tabla
            // y de otra vista a la vez: `from` es exactamente una de dos, y por
            // eso aquí no hay que decidir nada.
            Some(Fuente::Datasource { .. }) | Some(Fuente::Tabla(_)) => return Ok(fila),
            Some(Fuente::Vista(otra)) => match pkg.view(&otra) {
                Some(n) => actual = n,
                None => {
                    return Err(SinRaiz::NoExiste {
                        vista: otra,
                        desde: qn,
                    });
                }
            },
            // v1alpha12: un dataset es un eslabón más. Si es mantenido se
            // sigue por su `from`; si es escrito, la vuelta siguiente toca
            // suelo arriba.
            Some(Fuente::Dataset(otro)) => match pkg.dataset(&otro) {
                Some(n) => actual = n,
                None => {
                    return Err(SinRaiz::NoExiste {
                        vista: otro,
                        desde: qn,
                    });
                }
            },
        }
    }
}

/// Por qué una vista no se puede invertir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoInvertible {
    /// Un campo no sale de una columna: sale de **calcularla**. Escribir por
    /// ahí exigiría deshacer el cálculo, y no todos se deshacen.
    CampoCalculado { vista: String, campo: String },
    /// La vista declara algo que esta guarda no sabe clasificar.
    ///
    /// **Y por eso el defecto es «no».** Una clave nueva en el vocabulario que
    /// nadie clasifique llega aquí, en vez de colarse como invertible por no
    /// haberla mirado.
    ConstruccionDesconocida { vista: String, clave: String },
    /// La clave está clasificada, y su respuesta es que no.
    ///
    /// Es la que faltaba: antes de `groupBy` el vocabulario entero se repartía
    /// entre neutras e invertibles, así que un «no» solo podía llegar como
    /// *desconocida* — y eso confunde una decisión tomada con un descuido.
    NoSeDeshace { vista: String, clave: String },
}

/// Las claves de una `View` que **no cambian qué filas ni qué columnas salen**,
/// y por eso no afectan a la invertibilidad.
///
/// `moved` y `reserved` son neutras y merecen decirse: hablan de **nombres a
/// través del tiempo**, no del conjunto que se responde. Un campo anunciado
/// sigue saliendo de la columna que `fields` diga mientras exista, y uno
/// retirado ya no está en `fields` — así que no hay nada que deshacer que no
/// dijera ya la proyección.
const NEUTRAS: &[&str] = &["owner", "freshness", "materialized", "moved", "reserved"];

/// Las que sí, y son invertibles las tres.
///
/// | | por qué |
/// |---|---|
/// | `from` | una sola entrada. Es la primera condición que PostgreSQL exige a una vista auto-actualizable |
/// | `fields` | renombrar es una biyección; proyectar pierde columnas, así que la escritura es **parcial**, no ambigua |
/// | `where` | recortar es invertible: la fila escrita cumple el predicado, o se cae de la vista |
const INVERTIBLES: &[&str] = &["from", "fields", "where"];

/// Las que **no**, y esta lista nace con `groupBy`.
///
/// | | por qué |
/// |---|---|
/// | `groupBy` | de una agregación no se vuelve: la fila de salida es un conjunto de filas de entrada, y saber el total no dice cuáles eran |
/// | `having` | recorta por el agregado, así que deshacerlo exigiría deshacerlo primero. Y no es un `where`: aquel se cumple fila a fila, éste sólo se sabe del grupo entero |
///
/// Son, término a término, las dos primeras condiciones que PostgreSQL exige
/// para que una vista sea auto-actualizable y que ésta ya no cumple.
const NO_INVERTIBLES: &[&str] = &["groupBy", "having"];

/// **Por qué vistas escribe la ontología.** Derivado, nunca declarado.
///
/// Existe una `Function` con un `effect` sobre una propiedad de una entidad, y
/// esa entidad se respalda de esta vista. Es el mismo camino que recorre la
/// lectura, leído al revés, y por eso *por dónde se lee* y *por dónde se
/// escribe* no pueden divergir.
///
/// Es el sujeto de `OOS2024` y `OOS2025`, y es lo que hace que **ser espejo o
/// ser registro se decida por vista**: la misma vista, sin una función que la
/// escriba, compila virtual y refleja el origen exactamente.
pub fn escritas(pkg: &Package) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for f in pkg.docs.iter().filter(|d| d.kind == Kind::Function) {
        for e in f.section("effects").map(|n| n.items()).unwrap_or(&[]) {
            let Some(qn) = e.get("writes").and_then(|(_, v)| v.as_str()) else {
                continue;
            };
            let Some((entidad_qn, _)) = qn.rsplit_once('.') else {
                continue;
            };
            let Some(entidad) = pkg.entity(entidad_qn) else {
                continue;
            };
            if let Some(v) = respaldo(pkg, entidad)
                && let Some(vqn) = v.qname()
            {
                out.insert(vqn);
            }
        }
    }
    out
}

/// Si se puede escribir a través de esta vista.
///
/// # Por qué hoy no puede fallar, y aun así está
///
/// **Son dos motivos independientes, y confundirlos lleva a leer mal el
/// vocabulario entero.** El primero es que nadie llama a esto: `OOS7013` está
/// **reservado** —ADR 0018— porque escribir desde la ontología aterriza en la
/// copia, la copia guarda el vocabulario de la vista, y entonces un edit cae
/// *dentro* de `Q` y no hay nada que invertir. La invertibilidad es la primera
/// regla del producto que escribe **de vuelta en los orígenes**, que es otro.
///
/// El segundo es el de abajo. Y el corolario que hay que tener presente antes
/// de proponer ampliar la gramática: **lo que sostiene hoy el fragmento
/// estrecho no es esto**, que está aparcado, sino el precio en la regla de
/// flujo y la mantenibilidad incremental. Esta guarda no defiende una frontera
/// viva: guarda la **puerta** por la que se cruzaría.
///
/// El vocabulario de `View` en v1alpha8 es exactamente el fragmento invertible
/// —`00-scope` §6.1 lo dice, y no se buscó: se descubrió al migrar—. No hay
/// junta, ni agregado, ni `distinct`, ni límite, así que **ningún documento OOS
/// puede violar esto hoy**, y se dice aquí en vez de dejar que alguien lo
/// deduzca de que ningún caso lo ejerce.
///
/// Lo que esta guarda hace es que el día que la gramática crezca, el
/// constructor nuevo tenga que **decidir** si es invertible, en vez de heredar
/// un «sí» que nadie escribió. El defecto es `ConstruccionDesconocida`, y el
/// censo de `crate::document` lo ata al vocabulario para que no se pueda
/// añadir una clave sin pasar por aquí.
///
/// Es la misma forma que el IR de `ore-view`, que tiene `Agrupa` y `Une` con
/// sus reglas medidas y ningún documento que los produzca: la máquina está
/// lista antes que el vocabulario, a propósito.
pub fn invertible(v: &Loaded) -> Result<(), NoInvertible> {
    let qn = v.qname().unwrap_or_default();
    for (k, _) in v.root.get("spec").map(|(_, s)| s.entries()).unwrap_or(&[]) {
        let Some(clave) = k.as_str() else { continue };
        if NO_INVERTIBLES.contains(&clave) {
            return Err(NoInvertible::NoSeDeshace {
                vista: qn,
                clave: clave.to_string(),
            });
        }
        if NEUTRAS.contains(&clave)
            || INVERTIBLES.contains(&clave)
            || crate::document::is_extension(clave)
        {
            continue;
        }
        return Err(NoInvertible::ConstruccionDesconocida {
            vista: qn,
            clave: clave.to_string(),
        });
    }
    // Un campo tiene que salir de UNA columna, nombrada. Esto dejó de ser
    // inalcanzable el día que `fields` admitió un agregado: un `total:
    // sum(importe)` llega aquí y se niega, que es exactamente lo que la cabecera
    // anunciaba que pasaría cuando la gramática creciera.
    if let Some((campo, _)) = agregados(v).into_iter().next() {
        return Err(NoInvertible::CampoCalculado { vista: qn, campo });
    }
    for (campo, origen) in campos(v) {
        if !es_nombre_de_columna(&origen) {
            return Err(NoInvertible::CampoCalculado { vista: qn, campo });
        }
    }
    Ok(())
}

#[cfg(test)]
mod censo {
    use super::*;
    use crate::document::{ApiVersion, Kind};

    /// **El vocabulario de `View` está clasificado entero.**
    ///
    /// Esta es la prueba que le da dientes a [`invertible`]. Sin ella la guarda
    /// sería decorativa: alguien añade `groupBy` al vocabulario, nadie lo
    /// clasifica, y la guarda empieza a rechazarlo TODO en silencio —el defecto
    /// es «no»— o, peor, si el defecto fuera «sí», lo aceptaría todo.
    ///
    /// Con esto, añadir una clave a `View` sin decir si se invierte **no
    /// compila la suite**. Es el mismo mecanismo que el censo del registro de
    /// códigos, y por la misma razón: una lista que se puede ampliar sin mirar
    /// deja de significar algo.
    #[test]
    fn el_vocabulario_de_view_esta_clasificado_entero() {
        let mut sin_clasificar: Vec<&str> = Kind::View
            .spec_keys_en(ApiVersion::V1Alpha8)
            .iter()
            .copied()
            .filter(|k| {
                !NEUTRAS.contains(k) && !INVERTIBLES.contains(k) && !NO_INVERTIBLES.contains(k)
            })
            .collect();
        sin_clasificar.sort_unstable();
        assert!(
            sin_clasificar.is_empty(),
            "estas claves de `View` no están clasificadas en `vistas.rs`: {sin_clasificar:?}.\n\
             Di si cada una cambia qué filas o qué columnas salen —y entonces si eso se \
             invierte— antes de que un efecto pase por encima de ella sin mirar."
        );
    }

    /// Y la simétrica: nada clasificado que ya no exista.
    #[test]
    fn no_se_clasifica_lo_que_ya_no_esta_en_el_vocabulario() {
        let vocabulario = Kind::View.spec_keys_en(ApiVersion::V1Alpha8);
        let huerfanas: Vec<&&str> = NEUTRAS
            .iter()
            .chain(INVERTIBLES)
            .chain(NO_INVERTIBLES)
            .filter(|k| !vocabulario.contains(k))
            .collect();
        assert!(
            huerfanas.is_empty(),
            "clasificadas y ya inexistentes: {huerfanas:?}"
        );
    }
}

/// Un nombre de columna, no una expresión.
///
/// Deliberadamente permisivo con lo que un origen puede llamar columna
/// —`"Worker_Reference.ID"` es un nombre legal, y `01-table` lo dice—; lo que
/// descarta es lo que **solo** puede ser cómputo: operadores, llamadas, comas.
fn es_nombre_de_columna(s: &str) -> bool {
    !s.is_empty()
        && !s.chars().any(|c| {
            matches!(
                c,
                '+' | '-' | '*' | '/' | '(' | ')' | ',' | '|' | '<' | '>' | '=' | '\''
            )
        })
}

/// La raíz de una vista: fuente, objeto, columnas compuestas y filtros.
///
/// Un campo que en algún eslabón no resuelve **no aparece** en `columnas`: la
/// comprobación de que resuelva es de `comprobar`, con `OOS2018`, y aquí no se
/// inventa una columna para lo que no tiene.
pub fn raiz(pkg: &Package, v: &Loaded) -> Result<Raiz, SinRaiz> {
    let fila = cadena(pkg, v)?;
    let hoja = fila.last().expect("una cadena tiene al menos un eslabón");
    // Los dos caminos al suelo, y el único sitio del núcleo donde se distinguen.
    // Todo lo que llama a `raiz()` recibe la misma forma y no se entera de por
    // cuál vino: es lo que permite que un paquete tenga vistas de las dos
    // versiones a la vez sin que nadie más lleve un condicional.
    let (datasource, objeto, tabla) = match fuente(hoja) {
        Some(Fuente::Datasource { datasource, objeto }) => (datasource, objeto, None),
        Some(Fuente::Tabla(qn)) => {
            let Some(t) = pkg.table(&qn) else {
                return Err(SinRaiz::TablaNoExiste {
                    tabla: qn,
                    desde: hoja.qname().unwrap_or_default(),
                });
            };
            (
                t.section("datasource")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
                t.section("object")
                    .and_then(|o| o.as_str())
                    .unwrap_or_default()
                    .to_string(),
                Some(qn),
            )
        }
        // v1alpha12: la hoja es un dataset escrito. Es nuestro: vive en el lago
        // del inquilino con el nombre físico que `write()` le dio, y no tiene
        // `Table` (sus caras se saben, no se declaran). Quien necesite las dos
        // caras pregunta por `suelo()`.
        None if es_escrito(hoja) => (
            "lago".to_string(),
            format!(
                "{}_{}",
                hoja.meta("namespace")
                    .and_then(|n| n.as_str())
                    .unwrap_or(""),
                hoja.meta("name").and_then(|n| n.as_str()).unwrap_or("")
            ),
            None,
        ),
        _ => unreachable!("la cadena termina en una tabla o en una fuente por construcción"),
    };

    // De abajo arriba: la hoja nombra columnas físicas; cada eslabón de encima
    // nombra campos del de abajo, y se sustituyen.
    //
    // v1alpha12: un dataset escrito como hoja nombra SUS columnas, con sus
    // nombres —identidad—; y un mantenido sin `fields` expone todo lo del
    // eslabón de abajo, con sus nombres: la copia identidad no renombra nada.
    let mut columnas: BTreeMap<String, String> = if es_escrito(hoja) {
        columnas_de(hoja)
            .into_iter()
            .map(|c| (c.clone(), c))
            .collect()
    } else if es_mantenido(hoja) && hoja.section("fields").is_none() {
        // Un mantenido sobre una tabla, sin `fields`: expone las columnas de
        // la tabla tal cual.
        match fuente(hoja) {
            Some(Fuente::Tabla(qn)) => pkg
                .table(&qn)
                .map(|t| columnas_de(t).into_iter().map(|c| (c.clone(), c)).collect())
                .unwrap_or_default(),
            _ => BTreeMap::new(),
        }
    } else {
        campos(hoja)
    };
    let mut agrega: BTreeMap<String, Agregado> = agregados(hoja);
    let mut filtros_fisicos: Vec<(String, Vec<String>)> = filtros(hoja);
    for eslabon in fila.iter().rev().skip(1) {
        let de_abajo = columnas;
        let agrega_abajo = agrega;
        // v1alpha12: un dataset mantenido sin `fields` es la identidad sobre lo
        // de abajo: mismas columnas, mismos agregados, y sus filtros se suman.
        if es_mantenido(eslabon) && eslabon.section("fields").is_none() {
            columnas = de_abajo.clone();
            agrega = agrega_abajo.clone();
            for (campo, valores) in filtros(eslabon) {
                if let Some(c) = de_abajo.get(&campo) {
                    filtros_fisicos.push((c.clone(), valores));
                }
            }
            continue;
        }
        // Un eslabón de encima nombra campos del de abajo. Si el que nombra era
        // un agregado allí, **sigue siéndolo aquí**: renombrar una suma no la
        // convierte en una columna.
        columnas = BTreeMap::new();
        agrega = BTreeMap::new();
        for (campo, en_fuente) in campos(eslabon) {
            if let Some(c) = de_abajo.get(&en_fuente) {
                columnas.insert(campo, c.clone());
            } else if let Some(sobre) = agrega_abajo.get(&en_fuente) {
                agrega.insert(campo, sobre.clone());
            }
        }
        // Y lo que este eslabón agrega de nuevo: agrega un campo del de abajo,
        // así que lo que lee de verdad es la columna de ese campo.
        for (campo, a) in agregados(eslabon) {
            let sobre = a.sobre.and_then(|s| de_abajo.get(&s).cloned());
            agrega.insert(
                campo,
                Agregado {
                    funcion: a.funcion,
                    sobre,
                },
            );
        }
        for (campo, valores) in filtros(eslabon) {
            if let Some(c) = de_abajo.get(&campo) {
                filtros_fisicos.push((c.clone(), valores));
            }
        }
    }
    Ok(Raiz {
        datasource,
        objeto,
        columnas,
        filtros: filtros_fisicos,
        agrega,
        tabla,
    })
}

/// La **raíz de lectura**: de dónde salen de verdad las filas.
///
/// Es la vista `materialized` más cercana bajando por la cadena —ella misma
/// incluida—, y `None` cuando no hay ninguna y por tanto se lee del objeto.
///
/// La distinción con la raíz es todo el asunto de `OOS2020`, y no es un
/// tecnicismo: si la regla mirara la raíz, una vista virtual sobre una
/// materializada sobre un flujo fallaría, y obligaría a materializar dos veces
/// lo mismo. Hay dónde preguntar; está un eslabón más abajo.
pub fn raiz_de_lectura<'a>(pkg: &'a Package, v: &'a Loaded) -> Option<&'a Loaded> {
    // v1alpha12: es **el primer dataset bajando** —mantenido o escrito, ella
    // misma incluida—; y sigue siendo la vista con `materialized` para los
    // documentos de v1alpha7/8, que siguen compilando. Misma operación, otro
    // predicado: `es_copia` más el suelo escrito.
    cadena(pkg, v)
        .ok()?
        .into_iter()
        .find(|e| e.kind == Kind::Dataset || e.section("materialized").is_some())
}

/// v1alpha12. **El suelo de una cadena, como documento**: la `Table` en la que
/// termina, o el dataset **escrito** en el que termina. Es donde están las dos
/// caras —`changes.mode`, `changes.key`, `witness`— para quien las necesite
/// (`OOS2021`, `OOS2023`, `OOS2024`): la tabla las declara porque espeja; el
/// dataset escrito las declara porque las admite.
pub fn suelo<'a>(pkg: &'a Package, v: &'a Loaded) -> Option<&'a Loaded> {
    let fila = cadena(pkg, v).ok()?;
    let hoja = fila.last().copied()?;
    if es_escrito(hoja) {
        return Some(hoja);
    }
    match fuente(hoja)? {
        Fuente::Tabla(qn) => pkg.table(&qn),
        _ => None,
    }
}

/// Las fuentes físicas de una entidad: la raíz de la vista que la respalda.
///
/// Es lo que `governance` necesita para `OOS8005` y lo que `flow` necesita para
/// heredar la ubicación — y las dos deben verlo igual.
///
/// Devolvía un conjunto porque una entidad podía tener varios bindings, cada
/// uno con su fuente. Con `Binding` retirado el conjunto tiene como mucho un
/// elemento, y se mantiene el tipo: quien pregunta *«de dónde sale esto»*
/// pregunta lo mismo, y cambiar la firma obligaría a decidir aquí qué pasa
/// cuando no hay ninguna.
pub fn datasources_de(pkg: &Package, e: &Loaded) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let Some(v) = respaldo(pkg, e) else {
        return out;
    };
    // v1alpha14: una vista SQL puede leer de varias fuentes, y la entidad sale
    // de todas ellas: de cada tabla raíz de su linaje.
    if crate::linaje::usa_sql(pkg, v) {
        for raices in crate::linaje::linaje(pkg, v).unwrap_or_default().values() {
            for (r, _) in raices {
                if let Some(ds) = pkg
                    .table(&r.doc)
                    .and_then(|t| t.section("datasource"))
                    .and_then(|d| d.as_str())
                {
                    out.insert(ds.to_string());
                }
            }
        }
        return out;
    }
    if let Ok(r) = raiz(pkg, v) {
        out.insert(r.datasource);
    }
    out
}

/// A qué campo de `objetivo` llega cada campo de `desde`, siguiendo la cadena
/// hacia abajo. `None` si `objetivo` no está en la cadena de `desde`.
///
/// Es lo que hace que una etiqueta puesta en una entidad **viaje hasta la
/// vista que se materializa**: la entidad nombra campos de su vista, la vista
/// los renombra de la de abajo, y la de abajo es la que se copia.
pub fn proyectar(
    pkg: &Package,
    desde: &Loaded,
    objetivo: &str,
) -> Option<BTreeMap<String, String>> {
    let fila = cadena(pkg, desde).ok()?;
    let pos = fila
        .iter()
        .position(|v| v.qname().as_deref() == Some(objetivo))?;
    // Identidad en `desde`, y se compone bajando hasta `objetivo`.
    // Una vista proyecta sus `campos` (los agregados no salen de una columna
    // y los sella la derivacion, no esto); un dataset, lo que expone.
    let mut mapa: BTreeMap<String, String> = if desde.kind == Kind::Dataset {
        expone_en(pkg, desde)
    } else {
        campos(desde)
    }
    .keys()
    .map(|k| (k.clone(), k.clone()))
    .collect();
    for eslabon in &fila[..pos] {
        // v1alpha12: un dataset sin `fields` no renombra nada.
        if eslabon.kind == Kind::Dataset && eslabon.section("fields").is_none() {
            continue;
        }
        let renombres = campos(eslabon);
        mapa = mapa
            .into_iter()
            .filter_map(|(origen, actual)| {
                renombres.get(&actual).map(|abajo| (origen, abajo.clone()))
            })
            .collect();
    }
    Some(mapa)
}

// ── Enlazado ────────────────────────────────────────────────────────────────

fn datasources_declarados(pkg: &Package) -> BTreeSet<String> {
    pkg.of(Kind::OntologyConfig)
        .filter_map(|c| c.section("datasources"))
        .flat_map(|n| n.items())
        .filter_map(|it| {
            it.get("name")
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        })
        .collect()
}

fn no_declarado(v: &Loaded, nodo: &Node, campo: &str, declarados: &BTreeSet<String>) -> Diagnostic {
    let r = nodo.as_str().unwrap_or("");
    Diagnostic::new(
        Code::Oos2004,
        &v.path,
        format!("`{campo}: {r}` no está declarado en el manifiesto raíz"),
    )
    .at(nodo.pos())
    .help(if declarados.is_empty() {
        "el manifiesto no declara ningún datasource".to_string()
    } else {
        format!(
            "declarados: {}",
            declarados.iter().cloned().collect::<Vec<_>>().join(" · ")
        )
    })
}

fn no_expone(
    path: &std::path::Path,
    nodo: &Node,
    que: String,
    vista: &str,
    expone: &BTreeMap<String, String>,
) -> Diagnostic {
    Diagnostic::new(Code::Oos2018, path, que)
        .at(nodo.pos())
        .help(if expone.is_empty() {
            format!("`{vista}` no expone ningún campo")
        } else {
            format!(
                "`{vista}` expone: {}",
                expone.keys().cloned().collect::<Vec<_>>().join(" · ")
            )
        })
}

/// El gemelo de `no_expone` con el sujeto de v1alpha8: lo que se nombra no es
/// una columna de la tabla.
///
/// Es el mismo código y no el mismo mensaje, y la diferencia importa: *«la
/// vista de abajo no lo expone»* invita a mirar otra vista, y aquí no hay otra
/// vista — hay un objeto que no tiene esa columna, y la ayuda tiene que
/// enseñar las que sí tiene.
fn no_es_columna(
    path: &std::path::Path,
    nodo: &Node,
    que: String,
    tabla: &str,
    cols: &BTreeSet<String>,
) -> Diagnostic {
    Diagnostic::new(Code::Oos2018, path, que)
        .at(nodo.pos())
        .help(if cols.is_empty() {
            format!("`{tabla}` no declara ninguna columna")
        } else {
            format!(
                "`{tabla}` tiene: {}",
                cols.iter().cloned().collect::<Vec<_>>().join(" · ")
            )
        })
}

/// Dónde señalar en un documento que copia: `materialized` en una vista de
/// v1alpha7/8, `from` en un dataset mantenido.
fn donde_copia(v: &Loaded) -> Option<crate::diag::Pos> {
    v.section("materialized")
        .or_else(|| v.section("from"))
        .map(|n| n.pos())
}

/// v1alpha14 · las comprobaciones de una vista SQL (`01-la-vista-es-sql`
/// §3–§4): es UNA consulta que lee por nombre (`OOS2038`), lo que nombra existe
/// (`OOS2018`), lo que lee no vuelve sobre ella (`OOS2019`), su contrato es lo
/// que proyecta (`OOS2039`) y lo que lee se deja leer (`OOS2020`).
fn comprobar_sql(pkg: &Package, v: &Loaded, out: &mut Vec<Diagnostic>) {
    use crate::vista_sql::Fallo;
    let qn = v.qname().unwrap_or_default();
    let Some(nodo) = v.section("sql") else { return };
    let Some(r) = consulta(pkg, v) else { return };
    let c = match r {
        Ok(c) => c,
        Err(f) => {
            out.push(
                Diagnostic::new(
                    Code::Oos2038,
                    &v.path,
                    format!("`{qn}`: {}", f.como_texto()),
                )
                .at(nodo.pos())
                .help(match f {
                    Fallo::LeePorFuncion(_) => {
                        "una vista lee por NOMBRE: una tabla, una vista o un dataset del árbol. \
                         Lo que se lee por función no tiene linaje, ni etiqueta, ni conducto. \
                         Registra esos bytes como una `Table` o un `Dataset` y léelos por su \
                         nombre"
                    }
                    _ => {
                        "el cuerpo de una vista es UNA consulta `SELECT` —con `WITH`, `UNION`, \
                         joins, expresiones, agregados o ventanas— y nada que escriba"
                    }
                }),
            );
            return;
        }
    };

    // OOS2018 · cada nombre del árbol que lee, existe.
    let mut fuentes: BTreeMap<String, &Loaded> = BTreeMap::new();
    let mut falta = false;
    for n in &c.lee {
        match fuente_sql(pkg, n, v) {
            Some(d) => {
                fuentes.insert(n.clone(), d);
            }
            None => {
                falta = true;
                out.push(
                    Diagnostic::new(
                        Code::Oos2018,
                        &v.path,
                        format!("`{qn}` lee `{n}`, que no existe"),
                    )
                    .at(nodo.pos())
                    .help(
                        "lo que una vista lee DEBE ser una `Table`, una `View` o un `Dataset` del \
                         paquete o de una dependencia, nombrado en una, dos o tres partes (una \
                         es su schema; dos, `default`)",
                    ),
                );
            }
        }
    }
    if falta {
        return;
    }

    // OOS2019 · lo que lee no vuelve sobre ella.
    if let Some(ciclo) = vuelve(pkg, v) {
        out.push(
            Diagnostic::new(
                Code::Oos2019,
                &v.path,
                format!("lo que `{qn}` lee vuelve sobre ella: {}", ciclo.join(" → ")),
            )
            .at(nodo.pos())
            .help(
                "una vista se define por lo que lee, y una que se lee a sí misma no se define: \
                 ninguna de las del ciclo tiene de dónde salir",
            ),
        );
        return;
    }

    // OOS2018 · cada columna que la consulta nombra existe en su fuente.
    let mut nombradas: BTreeSet<&crate::vista_sql::Ref> = BTreeSet::new();
    for col in &c.columnas {
        nombradas.extend(col.directas.iter());
        nombradas.extend(col.derivadas.iter());
        nombradas.extend(col.indirectas.iter());
    }
    nombradas.extend(c.indirectas.iter());
    for p in &c.predicados {
        nombradas.extend(p.mira.iter());
    }
    let mut sin_columna = false;
    for r in nombradas {
        let Some(d) = fuentes.get(&r.fuente) else {
            continue;
        };
        let tiene = columnas_que_expone(pkg, d);
        if tiene.iter().any(|t| t.eq_ignore_ascii_case(&r.columna)) {
            continue;
        }
        sin_columna = true;
        let fqn = d.qname().unwrap_or_default();
        out.push(
            Diagnostic::new(
                Code::Oos2018,
                &v.path,
                format!("`{qn}` lee `{}` de `{fqn}`, que no la tiene", r.columna),
            )
            .at(nodo.pos())
            .help(if tiene.is_empty() {
                format!("`{fqn}` no expone ninguna columna")
            } else {
                format!(
                    "`{fqn}` tiene: {}",
                    tiene.into_iter().collect::<Vec<_>>().join(" · ")
                )
            }),
        );
    }
    for s in &c.sin_fuente {
        sin_columna = true;
        out.push(
            Diagnostic::new(
                Code::Oos2018,
                &v.path,
                format!("`{qn}` nombra `{s}`, que no es de ninguna de sus fuentes"),
            )
            .at(nodo.pos()),
        );
    }
    if sin_columna {
        return;
    }

    // OOS2039 · el contrato es lo que la consulta proyecta: ni uno de más, ni
    // uno de menos. El orden no significa nada, como no lo significaba `fields`.
    let proyecta: Vec<&str> = c.columnas.iter().map(|x| x.nombre.as_str()).collect();
    let declara = contrato(v);
    let de_mas: Vec<&String> = declara
        .iter()
        .filter(|n| !proyecta.iter().any(|p| p.eq_ignore_ascii_case(n)))
        .collect();
    let de_menos: Vec<&str> = proyecta
        .iter()
        .copied()
        .filter(|p| !declara.iter().any(|n| n.eq_ignore_ascii_case(p)))
        .collect();
    if !de_mas.is_empty() || !de_menos.is_empty() {
        let pos = v.section("columns").map(Node::pos).unwrap_or(nodo.pos());
        let mut que = Vec::new();
        if !de_mas.is_empty() {
            que.push(format!(
                "el contrato nombra {} y la consulta no lo proyecta",
                de_mas
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !de_menos.is_empty() {
            que.push(format!(
                "la consulta proyecta {} y el contrato no lo nombra",
                de_menos
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        out.push(
            Diagnostic::new(
                Code::Oos2039,
                &v.path,
                format!("`{qn}`: {}", que.join("; ")),
            )
            .at(pos)
            .help(format!(
                "`columns` no se inventa: se deriva de la consulta, describiéndola. La \
                     consulta proyecta: {}",
                proyecta.join(" · ")
            )),
        );
    }

    // OOS2020 · lo que no se puede leer no se lee: una tabla con `reads: none`
    // no contesta consultas. Una vista SQL es virtual; para leer de una copia,
    // se lee el dataset.
    for d in fuentes.values() {
        if d.kind != Kind::Table || se_lee(d) {
            continue;
        }
        let tqn = d.qname().unwrap_or_default();
        out.push(
            Diagnostic::new(
                Code::Oos2020,
                &v.path,
                format!("`{qn}` lee `{tqn}`, que declara `reads: none`: no hay dónde preguntar"),
            )
            .at(nodo.pos())
            .help(
                "una tabla con `reads: none` no responde consultas, solo emite cambios. Lee de un \
                 dataset que la copie —`kind: Dataset` con `from: { table }`—",
            ),
        );
    }
}

/// Las comprobaciones de enlazado de las tablas, las vistas y `backedBy`.
pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let declarados = datasources_declarados(pkg);
    // Qué vistas escribe la ontología. Se calcula una vez: es del paquete
    // entero, no de cada vista.
    let escritas_por_la_ontologia = escritas(pkg);

    // ── Las tablas ──────────────────────────────────────────────────────────
    //
    // Una tabla se sostiene sola: es el puntero a un objeto que existe, y
    // existe lo consulte alguien o no. Lo único que se le puede preguntar aquí
    // es si la fuente está declarada y si lo que sus dos caras nombran son
    // columnas suyas — que es exactamente lo que un esquema JSON no alcanza,
    // porque exige que un campo ESTÉ y no puede saber si lo que dice EXISTE.
    for tabla in pkg.of(Kind::Table) {
        let tqn = tabla.qname().unwrap_or_default();
        let cols = columnas(tabla);

        // OOS2004 · el mismo código que `datasourceRef` y que `from.datasource`.
        // Que el sujeto haya cambiado tres veces y el código sea el mismo es la
        // afirmación de que la tabla no cambia la regla, cambia el sujeto.
        if let Some(ds) = tabla.section("datasource")
            && !declarados.contains(ds.as_str().unwrap_or(""))
        {
            out.push(no_declarado(tabla, ds, "datasource", &declarados));
        }

        if let Some(cambios) = tabla.section("changes") {
            // OOS2018 · la clave del upsert. Sin clave real, un tombstone no
            // dice qué fila retira y el mantenedor aplicaría un `-1` a nada.
            if let Some((_, k)) = cambios.get("key") {
                for i in k.items() {
                    let Some(c) = i.as_str() else { continue };
                    if !cols.contains(c) {
                        out.push(no_es_columna(
                            &tabla.path,
                            i,
                            format!("`{tqn}` declara `changes.key: {c}`, que no es columna suya"),
                            &tqn,
                            &cols,
                        ));
                    }
                }
            }
            // OOS2018 · la marca de agua. Una que no es columna no la lee nadie,
            // y el refresco incremental no tendría por dónde empezar.
            if let Some((_, f)) = cambios.get("field") {
                let c = f.as_str().unwrap_or("");
                if !cols.contains(c) {
                    out.push(no_es_columna(
                        &tabla.path,
                        f,
                        format!("`{tqn}` declara `changes.field: {c}`, que no es columna suya"),
                        &tqn,
                        &cols,
                    ));
                }
            }
        }

        // OOS2018 · un filtro exigido que no es columna no lo puede poner nadie.
        // Cambia de sujeto respecto al binding, donde eran PROPIEDADES: lo exige
        // el origen, y el origen habla de columnas.
        if let Some((_, rf)) = tabla
            .section("reads")
            .and_then(|r| r.get("requiredFilters"))
        {
            for i in rf.items() {
                let Some(c) = i.as_str() else { continue };
                if !cols.contains(c) {
                    out.push(no_es_columna(
                        &tabla.path,
                        i,
                        format!("`{tqn}` exige filtrar por `{c}`, que no es columna suya"),
                        &tqn,
                        &cols,
                    ));
                }
            }
        }
    }

    // ── v1alpha12 · el dataset escrito se sostiene solo, como una tabla ─────
    //
    // Sus `columns` son lo que tiene; `changes.key` tiene que nombrarlas. Es
    // la regla de `changes.key` de la tabla con el sujeto cambiado.
    for d in pkg.of(Kind::Dataset).filter(|d| es_escrito(d)) {
        let dqn = d.qname().unwrap_or_default();
        let cols = columnas(d);
        // OOS2018 · OOS2019 · `derivedFrom` (W3.7 gobierno ③): lo que el código
        // leyó para escribirlo, vistas o datasets, y no él mismo. Por aquí baja
        // la clasificación (`flow::carga_de`), así que un nombre que no
        // resuelve es una carga que nadie ve.
        if let Some(df) = d.section("derivedFrom") {
            for i in df.items() {
                let Some(nombre) = i.as_str() else { continue };
                if nombre == dqn {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2019,
                            &d.path,
                            format!("`{dqn}` dice `derivedFrom` de sí mismo"),
                        )
                        .at(i.pos())
                        .help(
                            "lo que un dataset leyó para escribirse no puede ser él: sería \
                             llevar lo que ya lleva, y la cadena volvería sobre sí",
                        ),
                    );
                    continue;
                }
                if pkg.resolve_view(nombre, d).is_none() && pkg.resolve_dataset(nombre, d).is_none()
                {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2018,
                            &d.path,
                            format!("`derivedFrom: {nombre}` no existe"),
                        )
                        .at(i.pos())
                        .help(
                            "lo que el código leyó tiene que ser una vista o un dataset del \
                             paquete o de una dependencia: es por donde baja la clasificación a \
                             lo escrito, y lo que no está no clasifica nada",
                        ),
                    );
                }
            }
        }
        if let Some((_, k)) = d.section("changes").and_then(|c| c.get("key")) {
            for i in k.items() {
                let Some(c) = i.as_str() else { continue };
                if !cols.contains(c) {
                    out.push(no_es_columna(
                        &d.path,
                        i,
                        format!("`{dqn}` declara `changes.key: {c}`, que no es columna suya"),
                        &dqn,
                        &cols,
                    ));
                }
            }
        }
    }

    // Las vistas y, desde v1alpha12, los datasets mantenidos: el mismo plan,
    // las mismas reglas. Lo que es solo de la vista (`OOS2025`, `version`)
    // pregunta por el kind.
    for v in pkg
        .of(Kind::View)
        .chain(pkg.of(Kind::Dataset).filter(|d| es_mantenido(d)))
    {
        let qn = v.qname().unwrap_or_default();

        // v1alpha14: la vista SQL tiene sus propias comprobaciones, y ninguna
        // de las de la forma estructurada le alcanza.
        if es_sql(v) {
            comprobar_sql(pkg, v, out);
            continue;
        }

        // ── OOS2032 y OOS2033 · la agrupación cuadra consigo misma ──────────
        //
        // Las dos se contestan sin salir de la vista, así que van antes de
        // resolver la raíz: valen igual sobre una tabla y sobre otra vista.
        let ags = agregados(v);
        let por = agrupacion(v);
        if por.is_empty() {
            // OOS2033 · y esta NO es la regla de SQL, que admite un `count(*)`
            // sin agrupar. Se midió: el linaje de un agregado global sale
            // VACÍO —no viene de ninguna columna raíz—, así que la
            // comprobación de flujo no tiene nada que mirar y la cardinalidad
            // de la tabla sale sin gobierno. Con `groupBy`, el mismo agregado
            // gana una arista INDIRECTA por cada clave.
            if let Some((campo, _)) = ags.iter().next() {
                out.push(
                    Diagnostic::new(
                        Code::Oos2033,
                        &v.path,
                        format!("`{qn}.{campo}` agrega sobre toda la tabla"),
                    )
                    .at(v.section("fields").map(Node::pos).unwrap_or(v.root.pos()))
                    .help(
                        "un agregado global no sale de ninguna columna, así que su linaje es \
                         vacío y la regla de flujo no tiene nada que comprobar: el número de \
                         filas se publicaría sin gobierno. Declara `groupBy` con las columnas \
                         que parten el conjunto — cada una le da al agregado una arista",
                    ),
                );
            }
        } else {
            // OOS2032 · la regla de SQL, y por la misma razón: una columna que
            // no agrupa ni se agrega no tiene UN valor por grupo, tiene varios,
            // y elegir uno sería inventárselo.
            for (campo, col) in campos(v) {
                if por.contains(&col) {
                    continue;
                }
                out.push(
                    Diagnostic::new(
                        Code::Oos2032,
                        &v.path,
                        format!("`{qn}.{campo}` sale de `{col}`, que no se agrupa"),
                    )
                    .at(v.section("fields").map(Node::pos).unwrap_or(v.root.pos()))
                    .help(format!(
                        "en un grupo `{col}` tiene varios valores y esta vista pide uno: o entra \
                         en `groupBy` —hoy agrupa por {}— o sale agregada, `{campo}: \
                         max({col})`",
                        por.join(" · ")
                    )),
                );
            }
        }

        // ── OOS2034 · `having` filtra por lo que sólo se sabe agrupando ─────
        //
        // Nombrar una clave de grupo no está prohibido por gusto: ese predicado
        // es un `where`, y un `where` BAJA AL ORIGEN mientras que un `having`
        // no puede. Escribirlo aquí no da otro resultado, da el mismo más caro.
        for (campo, _) in teniendo(v) {
            if ags.contains_key(&campo) {
                continue;
            }
            let de_columna = campos(v).contains_key(&campo);
            out.push(
                Diagnostic::new(
                    Code::Oos2034,
                    &v.path,
                    format!("`{qn}` tiene `having` sobre `{campo}`, que no es un agregado"),
                )
                .at(v.section("having").map(Node::pos).unwrap_or(v.root.pos()))
                .help(if de_columna {
                    format!(
                        "`{campo}` sale de una columna, así que su predicado es un `where`: \
                         allí recorta ANTES de agrupar y el origen puede aplicarlo. Aquí \
                         recortaría después, con el mismo resultado y leyendo la tabla entera"
                    )
                } else {
                    format!(
                        "`having` nombra campos de esta vista, y `{campo}` no es ninguno. Los \
                         agregados que hay son: {}",
                        if ags.is_empty() {
                            "ninguno — esta vista no agrega nada".to_string()
                        } else {
                            ags.keys().cloned().collect::<Vec<_>>().join(" · ")
                        }
                    )
                }),
            );
        }

        // ── OOS2025 · lo que se escribe se debe materializar ────────────────
        //
        // El gemelo exacto de `OOS2020` leído por el otro lado. Una vista
        // virtual es una pregunta que se hace al origen cada vez: no tiene
        // estado, así que no tiene dónde sostener una edición — y el origen no
        // se toca, porque el puntero es de solo lectura (ADR 0018).
        //
        // Y es lo que decide si una vista es ESPEJO o REGISTRO, por vista y no
        // por producto: sin una función que la escriba, esta misma vista
        // compila virtual y refleja el origen exactamente.
        // v1alpha12: «es virtual» es «no hay dataset en su cadena» —ella misma
        // incluida—, y el remedio es un dataset. Un `backedBy` sobre un dataset
        // escrito no llega aquí: `escritas()` devuelve lo que respalda, y un
        // dataset no es una vista.
        if v.kind == Kind::View && escritas_por_la_ontologia.contains(&qn) {
            if raiz_de_lectura(pkg, v).is_none() {
                out.push(
                    Diagnostic::new(
                        Code::Oos2025,
                        &v.path,
                        format!("la ontología escribe por `{qn}`, que es virtual"),
                    )
                    .at(v.root.pos())
                    .help(
                        "una vista virtual no tiene dónde sostener una edición, y el origen no \
                         se toca: el puntero es de solo lectura. Respáldala en un dataset —un \
                         `kind: Dataset` con `from: { view }`, o sácala de uno— — es el gemelo \
                         de `OOS2020`, que exige lo mismo cuando lo que no se puede es leer",
                    ),
                );
            }
            // ── OOS2024 · y con qué se identifica la fila que toca un edit ──
            //
            // Remedio distinto que el de arriba —aquél se arregla en la vista,
            // este en la tabla— y por eso son dos códigos y no uno.
            // v1alpha12: el suelo puede ser un dataset escrito, y entonces la
            // clave es la suya (`changes.key`).
            if let Some(tabla) = suelo(pkg, v)
                && tabla
                    .section("changes")
                    .and_then(|c| c.get("key"))
                    .is_none()
            {
                let tqn = tabla.qname().unwrap_or_default();
                out.push(
                    Diagnostic::new(
                        Code::Oos2024,
                        &tabla.path,
                        format!(
                            "la ontología escribe por `{qn}`, y su raíz `{tqn}` no declara \
                             `changes.key`"
                        ),
                    )
                    .at(tabla.root.pos())
                    .help(
                        "un edit dice «la propiedad tal de la fila tal»; sin clave, «la fila \
                         tal» no nombra ninguna. Declara `changes.key` con las columnas que \
                         identifican una fila — es la misma que hace fundible un incremento y \
                         la misma que retira un tombstone, y por eso no hay una segunda",
                    ),
                );
            }
        }

        let Some(from) = v.section("from") else {
            continue;
        };

        // OOS2004 · la fuente, declarada. El mismo código que `datasourceRef`
        // porque es exactamente el mismo defecto con otro nombre de campo.
        if let Some((_, ds)) = from.get("datasource")
            && !declarados.contains(ds.as_str().unwrap_or(""))
        {
            out.push(no_declarado(v, ds, "from.datasource", &declarados));
        }
        if let Some((_, ds)) = v.section("materialized").and_then(|m| m.get("datasource"))
            && !declarados.contains(ds.as_str().unwrap_or(""))
        {
            out.push(no_declarado(v, ds, "materialized.datasource", &declarados));
        }

        // OOS2018 · v1alpha8 · la tabla existe, y tiene las columnas que esta
        // vista nombra. Es la misma regla que para `from.view` con el sujeto
        // cambiado, y es **la primera vez que llega hasta la columna física**:
        // hasta que la tabla no declaró `columns` no había contra qué comprobar,
        // así que el último tramo —el que toca el mundo— se creía.
        if let Some((_, nodo)) = from.get("table") {
            let referencia = nodo.as_str().unwrap_or("");
            match pkg.resolve_table(referencia, v) {
                None => out.push(
                    Diagnostic::new(
                        Code::Oos2018,
                        &v.path,
                        format!("`from.table: {referencia}` no existe"),
                    )
                    .at(nodo.pos())
                    .help(
                        "una vista sale de una tabla del paquete o de una dependencia. Una                          cadena que no llega al suelo no tiene raíz, y sin raíz no hay de dónde                          heredar etiquetas ni de dónde leer",
                    ),
                ),
                Some(tabla) => {
                    let cols = columnas(tabla);
                    let tqn = tabla.qname().unwrap_or_default();
                    let mios = campos(v);
                    // OOS2018 · lo que agrega, y por lo que agrupa, también son
                    // columnas de la tabla. Es el mismo código porque es el
                    // mismo defecto: un nombre que no existe.
                    let ags = agregados(v);
                    if let Some(fs) = v.section("fields") {
                        for (k, val) in fs.entries() {
                            let Some(campo) = k.as_str() else { continue };
                            if let Some(a) = ags.get(campo) {
                                if let Some(sobre) = &a.sobre
                                    && !cols.contains(sobre)
                                {
                                    out.push(no_es_columna(
                                        &v.path,
                                        val,
                                        format!(
                                            "`{qn}.{campo}` agrega `{sobre}`, que `{tqn}` no tiene"
                                        ),
                                        &tqn,
                                        &cols,
                                    ));
                                }
                                continue;
                            }
                            let col = mios.get(campo).cloned().unwrap_or_default();
                            if !cols.contains(&col) {
                                out.push(no_es_columna(
                                    &v.path,
                                    val,
                                    format!("`{qn}.{campo}` lee `{col}`, que `{tqn}` no tiene"),
                                    &tqn,
                                    &cols,
                                ));
                            }
                        }
                    }
                    if let Some(w) = v.section("where") {
                        for (k, _) in w.entries() {
                            let Some(col) = k.as_str() else { continue };
                            if !cols.contains(col) {
                                out.push(no_es_columna(
                                    &v.path,
                                    k,
                                    format!("`{qn}` filtra por `{col}`, que `{tqn}` no tiene"),
                                    &tqn,
                                    &cols,
                                ));
                            }
                        }
                    }
                    if let Some(g) = v.section("groupBy") {
                        for i in g.items() {
                            let Some(col) = i.as_str() else { continue };
                            if !cols.contains(col) {
                                out.push(no_es_columna(
                                    &v.path,
                                    i,
                                    format!("`{qn}` agrupa por `{col}`, que `{tqn}` no tiene"),
                                    &tqn,
                                    &cols,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // OOS2018 · la vista de abajo existe, y expone lo que esta le pide.
        // OOS2019 · y la cadena no vuelve sobre sí misma.
        //
        // v1alpha12: y el dataset de abajo igual. Es la misma rama con el
        // sujeto cambiado —lo que el de abajo EXPONE—, y `expone_en` sabe
        // decirlo de los dos.
        let de_abajo = match (from.get("view"), from.get("dataset")) {
            (Some((_, nodo)), _) => Some((
                "view",
                nodo,
                pkg.resolve_view(nodo.as_str().unwrap_or(""), v),
            )),
            (None, Some((_, nodo))) => Some((
                "dataset",
                nodo,
                pkg.resolve_dataset(nodo.as_str().unwrap_or(""), v),
            )),
            _ => None,
        };
        if let Some((clave, nodo, resuelto)) = de_abajo {
            let referencia = nodo.as_str().unwrap_or("");
            let Some(abajo) = resuelto else {
                out.push(
                    Diagnostic::new(
                        Code::Oos2018,
                        &v.path,
                        format!("`from.{clave}: {referencia}` no existe"),
                    )
                    .at(nodo.pos())
                    .help(if clave == "view" {
                        "una vista sobre otra necesita que la otra esté en el paquete o en \
                         una dependencia. Resolver un nombre exige el paquete entero: es lo \
                         que un esquema JSON no alcanza"
                    } else {
                        "leer de un dataset exige que el dataset esté en el paquete o en una \
                         dependencia: es lo que se tiene, y lo que no se tiene no se lee"
                    }),
                );
                continue;
            };
            match cadena(pkg, v) {
                Err(SinRaiz::Ciclo(c)) => {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2019,
                            &v.path,
                            format!(
                                "la cadena de vistas vuelve sobre sí misma: {}",
                                c.join(" → ")
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "una vista se define por lo que tiene debajo, y una que se tiene a \
                             sí misma debajo no se define. Ninguna de las de la cadena tiene \
                             raíz, así que ninguna se puede leer",
                        ),
                    );
                    continue;
                }
                Err(_) => continue,
                Ok(_) => {}
            }
            let expone = expone_en(pkg, abajo);
            let abajo_qn = abajo.qname().unwrap_or_default();
            let mios = campos(v);
            // OOS2018 · lo que agrega, y por lo que agrupa, también son campos
            // de la vista de abajo. Es la misma regla que sobre una tabla, con
            // el sujeto cambiado: allí columnas, aquí lo que la otra expone.
            //
            // ⛔ Medido el 2026-09-16: esta rama resolvía cada campo con
            //   `campos(v)`, que EXCLUYE los agregados a propósito, así que
            //   `total: "sum(baseSalary)"` sobre una vista salía como *«lee ``,
            //   que no expone»* — un nombre vacío en el diagnóstico y ninguna
            //   forma de agrupar sobre una vista. La rama de la tabla los
            //   trataba aparte desde el principio; ésta no.
            let ags = agregados(v);
            if let Some(fs) = v.section("fields") {
                for (k, val) in fs.entries() {
                    let Some(campo) = k.as_str() else { continue };
                    if let Some(a) = ags.get(campo) {
                        if let Some(sobre) = &a.sobre {
                            match expone.get(sobre) {
                                None => out.push(no_expone(
                                    &v.path,
                                    val,
                                    format!(
                                        "`{qn}.{campo}` agrega `{sobre}`, que `{abajo_qn}` no expone"
                                    ),
                                    &abajo_qn,
                                    &expone,
                                )),
                                // Agregar lo que abajo ya es un agregado no
                                // llega a ninguna columna: el plan baja cada
                                // agregado a la columna de la que sale, y de
                                // `sum(n)` con `n: count()` no sale ninguna.
                                Some(de) if agregado(de).is_some() => out.push(
                                    Diagnostic::new(
                                        Code::Oos2018,
                                        &v.path,
                                        format!(
                                            "`{qn}.{campo}` agrega `{sobre}`, que en `{abajo_qn}` ya es un agregado (`{de}`)"
                                        ),
                                    )
                                    .at(val.pos())
                                    .help(
                                        "un agregado se baja a la columna de la que sale, y de un \
                                         agregado no sale ninguna. Agrega la columna en la vista \
                                         que la tiene, o agrupa aquí por lo que abajo es un campo",
                                    ),
                                ),
                                Some(_) => {}
                            }
                        }
                        continue;
                    }
                    let en_fuente = mios.get(campo).cloned().unwrap_or_default();
                    if !expone.contains_key(&en_fuente) {
                        out.push(no_expone(
                            &v.path,
                            val,
                            format!("`{qn}.{campo}` lee `{en_fuente}`, que `{abajo_qn}` no expone"),
                            &abajo_qn,
                            &expone,
                        ));
                    }
                }
            }
            if let Some(g) = v.section("groupBy") {
                for i in g.items() {
                    let Some(campo) = i.as_str() else { continue };
                    if !expone.contains_key(campo) {
                        out.push(no_expone(
                            &v.path,
                            i,
                            format!("`{qn}` agrupa por `{campo}`, que `{abajo_qn}` no expone"),
                            &abajo_qn,
                            &expone,
                        ));
                    }
                }
            }
            if let Some(w) = v.section("where") {
                for (k, _) in w.entries() {
                    let Some(campo) = k.as_str() else { continue };
                    if !expone.contains_key(campo) {
                        out.push(no_expone(
                            &v.path,
                            k,
                            format!("`{qn}` filtra por `{campo}`, que `{abajo_qn}` no expone"),
                            &abajo_qn,
                            &expone,
                        ));
                    }
                }
            }
        }

        // OOS2018 · el testigo por campo nombra un campo de la vista.
        if v.kind == Kind::View
            && let Some(ver) = v.section("version")
            && let Some((_, f)) = ver.get("field")
        {
            let campo = f.as_str().unwrap_or("");
            let expone = expone(v);
            if !expone.contains_key(campo) {
                out.push(no_expone(
                    &v.path,
                    f,
                    format!("`{qn}` declara `version.field: {campo}`, que no está en `fields`"),
                    &qn,
                    &expone,
                ));
            }
        }
    }

    // ── OOS2020 · lo que no se puede leer se debe materializar ──────────────
    //
    // En su propio recorrido y no dentro del de arriba: aquel tiene `continue`
    // en cada rama de error, y una regla que solo se comprueba cuando ninguna
    // otra falló es una regla que un día deja de comprobarse sin que se note.
    for v in pkg.of(Kind::View) {
        // Hay copia en la cadena —ella misma incluida—: se lee de ahí.
        if raiz_de_lectura(pkg, v).is_some() {
            continue;
        }
        let Ok(fila) = cadena(pkg, v) else { continue };
        let hoja = fila.last().copied().expect("una cadena tiene un eslabón");
        let Some(Fuente::Tabla(tqn)) = fuente(hoja) else {
            continue;
        };
        let Some(tabla) = pkg.table(&tqn) else {
            continue;
        };
        if se_lee(tabla) {
            continue;
        }
        let qn = v.qname().unwrap_or_default();
        let pos = v.section("from").map(|f| f.pos());
        let mut d = Diagnostic::new(
            Code::Oos2020,
            &v.path,
            format!("`{qn}` es virtual y `{tqn}` declara `reads: none`: no hay dónde preguntar"),
        )
        .help(
            "una tabla con `reads: none` no responde consultas, solo emite cambios — un tema se              escribe, no se pregunta. Esta vista promete un sitio donde preguntar que no existe,              y lo promete al compilar para fallar al consultar. Ponle `materialized`, o sácala de              una vista de abajo que ya lo lleve",
        );
        if let Some(p) = pos {
            d = d.at(p);
        }
        out.push(d);
    }

    // ── OOS2029 · lo que no se proyecta en el origen no se copia ────────────
    //
    // **La palabra que le faltaba a `reads`.** Sabía decir que un origen no
    // empuja ningún filtro —`predicatePushdown: []`— y no sabía decir que
    // tampoco empuja **la proyección**: que lee la fila entera y descarta
    // columnas después de tenerlas.
    //
    // La diferencia no es de rendimiento. La máscara de este árbol es
    // **estructural**, y `ore-driver` lo dice con estas palabras:
    //
    // > *«una propiedad `redact` no está en el plan, luego no está en la
    // > petición, luego NO PUEDE ESTAR EN EL SQL. La salvaguarda es estructural
    // > — no hay ningún punto donde alguien pueda olvidarse de aplicarla,
    // > porque no hay nada que aplicar.»*
    //
    // Con `projectionPushdown: false` eso deja de ser cierto: la columna
    // enmascarada **sale del origen** y alguien la tira. Es otra garantía —una
    // que se aplica en vez de no existir— y hasta hoy el árbol no tenía
    // vocabulario para distinguirlas.
    //
    // # Por qué se prohíbe la COPIA y no la lectura
    //
    // Porque una lectura virtual mueve la fila entera al proceso del lector y
    // ahí se acaba: no queda artefacto. Una copia sí queda, **sellada con una
    // clasificación calculada sobre los campos de la vista** — y esa cuenta es
    // falsa si lo que cruzó fue el objeto entero. El sello no mentiría sobre lo
    // que contiene la copia; mentiría sobre lo que se movió para hacerla.
    //
    // # El valor por defecto es `true`, y no es P4
    //
    // P4 dice que omitir es cerrar, y aquí cerrar sería `false`. No se aplica:
    // el protocolo del driver **exige** empujar la proyección —lo afirma su
    // cabecera y lo prueba `ore-sql`—, así que un lector que no lo haga está
    // incumpliendo el contrato, no ejerciendo una opción. `false` es una
    // **confesión**, y por eso hay que escribirla.
    for v in pkg.of(Kind::View).chain(pkg.of(Kind::Dataset)) {
        if !es_copia(v) {
            continue;
        }
        let Ok(r) = raiz(pkg, v) else { continue };
        let Some(tabla) = r.tabla.as_deref().and_then(|qn| pkg.table(qn)) else {
            continue;
        };
        let proyecta = tabla
            .section("reads")
            .and_then(|x| x.get("projectionPushdown"))
            .and_then(|(_, b)| b.as_str())
            .is_none_or(|b| b != "false");
        if proyecta {
            continue;
        }
        let qn = v.qname().unwrap_or_default();
        let tqn = r.tabla.as_deref().unwrap_or_default();
        let mut d = Diagnostic::new(
            Code::Oos2029,
            &v.path,
            format!(
                "`{qn}` se copia de `{tqn}`, que declara `projectionPushdown: false`: la fila \
                 entera sale del origen y las columnas se descartan después"
            ),
        )
        .help(
            "la máscara de este modelo es estructural —lo que no está en el plan no está en la \
             petición y no puede estar en la consulta—, y con un origen que no proyecta deja de \
             serlo: lo enmascarado sale y alguien lo tira. La copia se sellaría con la \
             clasificación de los campos de la vista, que no es lo que se movió. Léela virtual, o \
             cópiala desde un objeto que sí proyecte",
        );
        if let Some(p) = donde_copia(v) {
            d = d.at(p);
        }
        out.push(d);
    }

    // ── OOS2023 · la pareja decide la garantía ──────────────────────────────
    //
    // `witness: field` fecha por una columna, y eso es **at-least-once por
    // construcción**: la columna es siempre mayor o igual que sí misma, así que
    // el solape se re-entrega en cada refresco. Airbyte lo documenta con esas
    // palabras para su *cursor field*, que es el mismo mecanismo, y admite
    // además que se pueden **perder** filas si la columna no se mantiene al
    // modificar una.
    //
    // Con `upsert` o `retract` hay clave y re-entregar es idempotente. Con
    // `append` no hay con qué deduplicar: **cada refresco suma el solape, para
    // siempre**, y nadie lo ve hasta que llega la factura.
    //
    // Es la tercera regla que mira las dos caras a la vez, con `OOS2020` y
    // `OOS2021`, y como ellas **es sobre la copia y no sobre la tabla**: un log
    // de eventos fechado por una columna de tiempo es legítimo y existe. Lo que
    // no se puede es **mantener una copia suya**.
    for v in pkg.of(Kind::View).chain(pkg.of(Kind::Dataset)) {
        // Solo si esta vista es la que se copia. Una virtual encima de una copia
        // no declara nada, y la de abajo ya se comprueba por su cuenta.
        // v1alpha12: «la que se copia» es el dataset mantenido, o la vista de
        // v1alpha7/8 con `materialized`.
        if !es_copia(v) {
            continue;
        }
        let Ok(r) = raiz(pkg, v) else { continue };
        let Some(tabla) = r.tabla.as_deref().and_then(|qn| pkg.table(qn)) else {
            continue;
        };
        let por_columna = tabla
            .section("changes")
            .and_then(|c| c.get("witness"))
            .and_then(|(_, w)| w.as_str())
            == Some("field");
        if !(por_columna && modo(tabla) == Modo::Anexa) {
            continue;
        }
        let qn = v.qname().unwrap_or_default();
        let tqn = r.tabla.as_deref().unwrap_or_default();
        let mut d = Diagnostic::new(
            Code::Oos2023,
            &v.path,
            format!(
                "`{qn}` se copia de `{tqn}`, que declara `{{ mode: append, witness: field }}`: no \
                 hay clave con la que deduplicar lo que se re-entrega"
            ),
        )
        .help(
            "fechar por una columna es at-least-once: la columna es mayor o IGUAL que sí misma, \
             así que cada refresco vuelve a traer el borde y sin clave no hay forma de quitarlo — \
             el solape se acumula para siempre y no da ningún síntoma hasta la factura. La tabla \
             es legítima; lo que no se puede es mantener una copia suya. Declara `key` con \
             `mode: upsert`, o fecha por `witness: log` o `snapshot`, que nombran una posición \
             replayable",
        );
        if let Some(p) = donde_copia(v) {
            d = d.at(p);
        }
        out.push(d);
    }

    // `backedBy` · la entidad nombra a su vista, o a su dataset (v1alpha12).
    for e in pkg.entities() {
        let Some(b) = e.section("backedBy") else {
            continue;
        };
        let referencia = b.as_str().unwrap_or("");
        let qn = e.qname().unwrap_or_default();
        let Some(v) = pkg.resolve_lectura(referencia, e) else {
            out.push(
                Diagnostic::new(
                    Code::Oos2018,
                    &e.path,
                    format!("`backedBy: {referencia}` no existe"),
                )
                .at(b.pos())
                .help(
                    "la entidad nombra a la vista —o al dataset— que la respalda, y no al \
                     revés: tiene que existir antes. Es lo que permite descubrir y exponer una \
                     fuente antes de modelar nada sobre ella",
                ),
            );
            continue;
        };
        let expone = expone_en(pkg, v);
        let vista_qn = v.qname().unwrap_or_default();

        // OOS2011 · lo que necesita columna: la clave y los `via`. La misma
        // regla del binding, dicha de la vista.
        let mut exigidas: Vec<(String, &Node)> = Vec::new();
        if let Some(k) = e.section("primaryKey") {
            for i in k.items() {
                if let Some(p) = i.as_str() {
                    exigidas.push((p.to_string(), i));
                }
            }
        }
        if let Some(rels) = e.section("relations") {
            for (_, rv) in rels.entries() {
                if let Some((_, via)) = rv.get("via") {
                    for i in via.items() {
                        if let Some(p) = i.as_str() {
                            exigidas.push((p.to_string(), i));
                        }
                    }
                }
            }
        }
        for (p, nodo) in exigidas {
            if !expone.contains_key(&p) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2011,
                        &e.path,
                        format!("`{vista_qn}` no expone `{p}`, que `{qn}` necesita como columna"),
                    )
                    .at(nodo.pos())
                    .help(
                        "sin la clave no hay resolución de instancia, ni índice de topología, \
                         ni recurso identificable en una política; sin la columna de un enlace, \
                         la relación se declara y no se puede recorrer. Añade el campo a la \
                         vista o quítalo de la entidad",
                    ),
                );
            }
        }

        // ── OOS2022 · una propiedad sin campo no tiene de dónde salir ────────
        //
        // **La otra cara de haber retirado la federación.** `03-binding` §2.1
        // admitía que una entidad tuviera varios bindings, «cada uno cubre un
        // subconjunto de sus propiedades»: con eso, una cobertura parcial no
        // solo era legal, era el mecanismo, y preguntar de dónde sale una
        // propiedad no tenía respuesta local.
        //
        // v1alpha8 retira eso (`00-scope` §6): una entidad sale de UNA vista. Y
        // en cuanto no hay otro documento donde mirar, una propiedad sin campo
        // pasa de «la cubre otro» a «no la cubre nadie».
        //
        // Sin esto, la migración que esta versión pide produce el fallo que este
        // proyecto persigue: se escribe la vista con la mitad de los campos, la
        // entidad sigue declarando el doble, COMPILA EN VERDE, y las propiedades
        // huérfanas responden vacío para siempre. Se midió sobre un paquete de
        // tres propiedades y dos sin campo: `ok · sin errores`.
        //
        // **Y solo de v1alpha8.** Un documento anterior declaró su versión, y esa
        // versión sí admitía cobertura parcial: v1alpha1 porque otro binding la
        // cubría, y v1alpha7 porque el binding seguía en la gramática.
        // Aplicársela cambiaría lo que significa un documento ya escrito, y el
        // invariante que esta versión sostuvo en cinco peldaños es que **no
        // cambia un solo resultado de v1alpha1 a v1alpha7**.
        //
        // Se midió sin la puerta: `conformance/v1alpha7` caía de 13/13 a 12/13
        // —`valid/entity-backed-by-view`—, `acme-retail` dejaba de validar y
        // tres pruebas de `cache.rs` caían detrás de ella.
        if e.version()
            .is_some_and(|ver| ver >= crate::document::ApiVersion::V1Alpha8)
            && let Some(props) = e.section("properties")
        {
            for (k, cuerpo) in props.entries() {
                let Some(prop) = k.as_str() else { continue };
                // `derivedFrom` es la excepción, y es la única: una propiedad
                // derivada declara de qué otras sale, y eso ES su origen.
                // Exigirle además una columna sería exigirle que esté calculada
                // en la fuente — justo lo que la migración de
                // `Binding.properties.<x>.expression` deja de poder hacer.
                if cuerpo.get("derivedFrom").is_some() || expone.contains_key(prop) {
                    continue;
                }
                out.push(
                    Diagnostic::new(
                        Code::Oos2022,
                        &e.path,
                        format!("`{vista_qn}` no expone `{prop}`, que `{qn}` declara"),
                    )
                    .at(k.pos())
                    .help(
                        "una entidad sale de UNA vista, así que una propiedad que la vista no \
                         da no tiene de dónde salir: responde vacía y nada lo dice. Añade el \
                         campo a la vista, declara `derivedFrom` si de verdad se computa, o \
                         quita la propiedad. Con bindings esto era legal porque otro binding \
                         podía cubrirla; en v1alpha8 no hay otro",
                    ),
                );
            }
        }

        // ── OOS2021 · sin retractación no se mantiene lo mutable ─────────────
        //
        // El peor modo de fallo del motor, porque **no produce ningún síntoma**:
        // la vista se materializa, la consulta responde, los números salen — y
        // son los de antes. Sin este código se derivaría en silencio, y por eso
        // Foundry lo documenta como una limitación en vez de rechazarlo.
        //
        // Exige las tres cosas a la vez, y ninguna sobra: la entidad es MUTABLE
        // —un hecho ocurrido no se retira, y por eso un `nature: event` sí se
        // respalda de un `append`—; hay una COPIA en la cadena —una vista
        // virtual lee del origen, que sí tiene el estado presente—; y la raíz
        // SOLO ANEXA.
        // v1alpha12: el suelo puede ser un dataset escrito, y entonces «solo
        // anexa» es su propio `changes.mode: append`. Mismo código, mismo
        // mensaje: lo que se tiene copiando altas no es el estado presente.
        if e.section("nature").and_then(|n| n.as_str()) == Some("entity")
            && raiz_de_lectura(pkg, v).is_some()
            && let Some(tabla) = suelo(pkg, v)
            && modo(tabla) == Modo::Anexa
        {
            let tqn = tabla.qname().unwrap_or_default();
            out.push(
                Diagnostic::new(
                    Code::Oos2021,
                    &e.path,
                    format!(
                        "`{qn}` es `nature: entity` y se respalda de una copia de `{tqn}`, que \
                         solo anexa"
                    ),
                )
                .at(b.pos())
                .help(
                    "una entidad es una cosa que cambia y sigue siendo la misma, así que \
                     mantener su estado presente exige poder QUITAR lo que dejó de ser cierto. \
                     Un `changes.mode: append` no puede: lo que se copia no es el estado \
                     presente, es el histórico con las filas viejas dentro. La consulta \
                     responde, los números salen, y son los de antes. Un `nature: event` sí se \
                     respalda de un `append`",
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use std::path::PathBuf;

    fn doc(kind: Kind, texto: &str) -> Loaded {
        Loaded {
            path: PathBuf::from(format!("{}.yaml", kind.as_str())),
            kind,
            root: parse(texto).expect("yaml"),
        }
    }

    /// **La guarda de invertibilidad, ejercida por las tres ramas.**
    ///
    /// Hasta `groupBy` esto no lo podía disparar ningún documento: el
    /// vocabulario de `View` era exactamente el fragmento invertible y una
    /// agrupación ni pasaba de `OOS1005`. **Ya no.** Las dos primeras ramas de
    /// abajo salen ahora de documentos conformes, y esa es la diferencia entre
    /// una máquina escrita y una ejercida.
    ///
    /// Y las tres respuestas son distintas a propósito:
    ///
    /// - `NoSeDeshace` — clasificado, y la respuesta es no;
    /// - `CampoCalculado` — el campo no sale de una columna;
    /// - `ConstruccionDesconocida` — **el defecto**, para lo que nadie
    ///   clasificó. Si alguien amplía el vocabulario y se olvida, esto niega
    ///   la escritura en vez de concederla por descuido, y el censo de arriba
    ///   hace que además la suite se caiga.
    #[test]
    fn la_guarda_de_invertibilidad_niega_lo_que_no_sabe_clasificar() {
        // Lo que hoy se puede escribir: renombra, recorta y proyecta. Invertible.
        let buena = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: empleados, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               freshness: 15m\n  \
               fields: { id: employee_id, pais: country }\n  \
               where: { deleted: \"false\" }\n",
        );
        assert_eq!(invertible(&buena), Ok(()));

        // Una agrupación. Clasificada, y la respuesta es que de una agregación
        // no se vuelve. Este documento SÍ es conforme.
        let agrupada = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: por_pais, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               fields: { pais: country }\n  \
               groupBy: [country]\n",
        );
        assert_eq!(
            invertible(&agrupada),
            Err(NoInvertible::NoSeDeshace {
                vista: "hr.por_pais".to_string(),
                clave: "groupBy".to_string(),
            })
        );

        // Un agregado, que es el otro camino y llega al otro motivo: el campo
        // no sale de una columna, sale de un conjunto de filas.
        let agregada = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: cuentas, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               fields: { n: \"count()\" }\n",
        );
        assert_eq!(
            invertible(&agregada),
            Err(NoInvertible::CampoCalculado {
                vista: "hr.cuentas".to_string(),
                campo: "n".to_string(),
            })
        );

        // Y un campo que sale de calcularlo por otra vía: una expresión que la
        // gramática no admite, y que por eso no llega a ser un agregado.
        let calculada = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: importes, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               fields: { total: \"precio * cantidad\" }\n",
        );
        assert_eq!(
            invertible(&calculada),
            Err(NoInvertible::CampoCalculado {
                vista: "hr.importes".to_string(),
                campo: "total".to_string(),
            })
        );

        // Una extensión de proveedor no decide nada sobre las filas, así que no
        // niega: `x-` es el mecanismo declarado para lo que no es del estándar.
        let con_extension = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: empleados, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               fields: { id: employee_id }\n  \
               x-acme-nota: \"la que usa nominas\"\n",
        );
        assert_eq!(invertible(&con_extension), Ok(()));
    }

    fn paquete(docs: Vec<Loaded>) -> Package {
        Package {
            root: PathBuf::from("."),
            docs,
            cedar: Vec::new(),
            generated: Vec::new(),
            sobres: Vec::new(),
        }
    }

    fn config() -> Loaded {
        doc(
            Kind::OntologyConfig,
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: x, version: 0.1.0 }\n\
             datasources:\n  - { name: erp, type: postgres, connectionEnv: ERP_URL }\n",
        )
    }

    fn vista(nombre: &str, from: &str, fields: &str, extra: &str) -> Loaded {
        doc(
            Kind::View,
            &format!(
                "apiVersion: oos.dev/v1alpha7\nkind: View\nmetadata: {{ name: {nombre}, namespace: hr }}\n\
                 spec:\n  owner: team:hr\n  from: {from}\n  version: {{ witness: none }}\n  fields:\n{fields}{extra}"
            ),
        )
    }

    fn base() -> Loaded {
        vista(
            "empleados",
            "{ datasource: erp, object: public.employees }",
            "    employeeId: employee_id\n    nationalId: { column: national_id, physicalType: varchar(16) }\n    pais: country\n",
            "  where: { deleted: 'false', country: [ES, PT] }\n",
        )
    }

    #[test]
    fn la_raiz_compone_renombres_y_filtros() {
        let iberia = vista(
            "iberia",
            "{ view: empleados }",
            "    id: employeeId\n    dni: nationalId\n",
            "  where: { pais: ES }\n",
        );
        let pkg = paquete(vec![config(), base(), iberia]);
        let r = raiz(&pkg, pkg.view("hr.iberia").unwrap()).unwrap();
        assert_eq!(r.datasource, "erp");
        assert_eq!(r.objeto, "public.employees");
        assert_eq!(
            r.columnas.get("id").map(String::as_str),
            Some("employee_id")
        );
        assert_eq!(
            r.columnas.get("dni").map(String::as_str),
            Some("national_id")
        );
        // `pais` no lo expone `iberia`: no aparece.
        assert!(!r.columnas.contains_key("pais"));
        // Los filtros de abajo se heredan y el de arriba llega en columna física.
        assert_eq!(
            r.filtros,
            vec![
                ("deleted".to_string(), vec!["false".to_string()]),
                (
                    "country".to_string(),
                    vec!["ES".to_string(), "PT".to_string()]
                ),
                ("country".to_string(), vec!["ES".to_string()]),
            ]
        );
    }

    #[test]
    fn proyectar_baja_los_nombres_hasta_la_vista_que_se_copia() {
        let iberia = vista(
            "iberia",
            "{ view: empleados }",
            "    id: employeeId\n    dni: nationalId\n",
            "",
        );
        let pkg = paquete(vec![config(), base(), iberia]);
        let m = proyectar(&pkg, pkg.view("hr.iberia").unwrap(), "hr.empleados").unwrap();
        assert_eq!(m.get("dni").map(String::as_str), Some("nationalId"));
        assert_eq!(m.get("id").map(String::as_str), Some("employeeId"));
        assert!(proyectar(&pkg, pkg.view("hr.empleados").unwrap(), "hr.iberia").is_none());
    }

    #[test]
    fn un_ciclo_es_oos2019_y_una_vista_ausente_oos2018() {
        let a = vista("a", "{ view: b }", "    x: x\n", "");
        let b = vista("b", "{ view: a }", "    x: x\n", "");
        let pkg = paquete(vec![config(), a, b]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert!(out.iter().any(|d| d.code == Code::Oos2019), "{out:?}");

        let suelta = vista("suelta", "{ view: nadie }", "    x: x\n", "");
        let pkg = paquete(vec![config(), suelta]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::Oos2018);
    }

    #[test]
    fn lo_que_la_de_abajo_no_expone_es_oos2018() {
        let iberia = vista(
            "iberia",
            "{ view: empleados }",
            "    salario: baseSalary\n",
            "  where: { ciudad: Vigo }\n",
        );
        let pkg = paquete(vec![config(), base(), iberia]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        let codigos: Vec<Code> = out.iter().map(|d| d.code).collect();
        assert_eq!(codigos, vec![Code::Oos2018, Code::Oos2018], "{out:?}");
    }

    /// Una vista v1alpha8 sobre otra: la forma que agrupa.
    fn vista8_sobre(nombre: &str, from: &str, fields: &str, extra: &str) -> Loaded {
        doc(
            Kind::View,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: {{ name: {nombre}, namespace: hr }}\n\
                 spec:\n  owner: team:hr\n  from: {from}\n  fields:\n{fields}{extra}"
            ),
        )
    }

    fn solo_2018(pkg: &Package) -> Vec<Code> {
        let mut out = Vec::new();
        comprobar(pkg, &mut out);
        out.iter()
            .map(|d| d.code)
            .filter(|c| *c == Code::Oos2018)
            .collect()
    }

    /// ⛔ Medido el 2026-09-16: agrupar sobre una TABLA compilaba y agrupar
    /// sobre una VISTA salía como *«lee ``, que no expone»*, porque esta rama
    /// resolvía los campos con `campos`, que excluye los agregados a propósito.
    #[test]
    fn agregar_sobre_una_vista_compila() {
        let por_pais = vista8_sobre(
            "porPais",
            "{ view: empleados }",
            "    pais: pais\n    n: \"count()\"\n    ids: \"max(employeeId)\"\n",
            "  groupBy: [pais]\n",
        );
        let pkg = paquete(vec![config(), base(), por_pais]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn agregar_lo_que_la_de_abajo_no_expone_es_oos2018() {
        let por_pais = vista8_sobre(
            "porPais",
            "{ view: empleados }",
            "    pais: pais\n    total: \"sum(salario)\"\n",
            "  groupBy: [pais]\n",
        );
        let pkg = paquete(vec![config(), base(), por_pais]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(solo_2018(&pkg), vec![Code::Oos2018]);
        assert!(
            out.iter().any(|d| d.message.contains("agrega `salario`")),
            "el diagnóstico tiene que nombrar lo que agrega, no un nombre vacío: {out:?}"
        );
    }

    #[test]
    fn agrupar_por_lo_que_la_de_abajo_no_expone_es_oos2018() {
        let por_ciudad = vista8_sobre(
            "porCiudad",
            "{ view: empleados }",
            "    n: \"count()\"\n",
            "  groupBy: [ciudad]\n",
        );
        let pkg = paquete(vec![config(), base(), por_ciudad]);
        assert_eq!(solo_2018(&pkg), vec![Code::Oos2018]);
    }

    /// De `sum(n)` con `n: count()` abajo no sale ninguna columna.
    #[test]
    fn agregar_un_agregado_es_oos2018() {
        let por_pais = vista8_sobre(
            "porPais",
            "{ view: empleados }",
            "    pais: pais\n    n: \"count()\"\n",
            "  groupBy: [pais]\n",
        );
        let encima = vista8_sobre(
            "total",
            "{ view: porPais }",
            "    pais: pais\n    s: \"sum(n)\"\n",
            "  groupBy: [pais]\n",
        );
        let pkg = paquete(vec![config(), base(), por_pais, encima]);
        assert_eq!(solo_2018(&pkg), vec![Code::Oos2018]);
    }

    #[test]
    fn la_fuente_sin_declarar_es_oos2004_con_las_dos_caras() {
        let v = vista(
            "v",
            "{ datasource: lago, object: t }",
            "    x: x\n",
            "  materialized: { datasource: otro, table: t2 }\n",
        );
        let pkg = paquete(vec![config(), v]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.code == Code::Oos2004));
    }

    #[test]
    fn backed_by_exige_la_clave_y_resuelve_en_corto() {
        let e = doc(
            Kind::Entity,
            "apiVersion: oos.dev/v1alpha7\nkind: Entity\nmetadata: { name: Employee, namespace: hr }\n\
             spec:\n  nature: entity\n  primaryKey: [employeeId]\n  backedBy: empleados\n\
             properties:\n    employeeId: { type: String }\n",
        );
        let pkg = paquete(vec![config(), base(), e]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert!(out.is_empty(), "{out:?}");

        let e2 = doc(
            Kind::Entity,
            "apiVersion: oos.dev/v1alpha7\nkind: Entity\nmetadata: { name: Employee, namespace: hr }\n\
             spec:\n  nature: entity\n  primaryKey: [id]\n  backedBy: empleados\n\
             properties:\n    id: { type: String }\n",
        );
        let pkg = paquete(vec![config(), base(), e2]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::Oos2011);
        assert_eq!(
            datasources_de(&pkg, pkg.entity("hr.Employee").unwrap()),
            BTreeSet::from(["erp".to_string()])
        );
    }

    #[test]
    fn el_testigo_por_campo_nombra_un_campo() {
        // A mano y no con el helper, que fija `witness: none`.
        let v = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha7\nkind: View\nmetadata: { name: v, namespace: hr }\n\
             spec:\n  owner: team:hr\n  from: { datasource: erp, object: t }\n  \
             version: { witness: field, field: updatedAt }\n  fields:\n    x: x\n",
        );
        let pkg = paquete(vec![config(), v]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].code, Code::Oos2018);
    }

    // ── v1alpha8 · la tabla ─────────────────────────────────────────────────

    fn tabla(nombre: &str, spec: &str) -> Loaded {
        doc(
            Kind::Table,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
                 metadata: {{ name: {nombre}, namespace: erp }}\nspec:\n{spec}"
            ),
        )
    }

    /// La tabla de referencia: las dos caras puestas, y se deja leer.
    fn employees() -> Loaded {
        tabla(
            "employees",
            "  datasource: erp\n  object: public.employees\n  \
             columns:\n    employee_id: {}\n    national_id: {}\n    country: {}\n    deleted: {}\n  \
             reads: { predicatePushdown: [eq, in], fullScan: cheap }\n  \
             changes: { mode: retract, witness: log }\n",
        )
    }

    /// `columns.<c>.type` es el tipo; la columna sin él no está, y la cita
    /// (`physicalType`) sola no tipa: es un hecho del origen, no una traducción.
    #[test]
    fn el_tipo_de_una_columna_es_el_que_el_conector_tradujo() {
        let t = tabla(
            "employees",
            "  datasource: erp\n  object: public.employees\n  \
             columns:\n    employee_id: { type: String, physicalType: \"varchar(16)\" }\n    \
             salary: { type: Decimal, physicalType: \"numeric(12,2)\" }\n    \
             hired_on: { type: Date }\n    \
             bonus: { type: \"Money<EUR, 2>\" }\n    \
             address: { physicalType: address_t }\n    \
             raw: {}\n    \
             mal: { type: Numero }\n  \
             reads: none\n  changes: { mode: append, witness: log }\n",
        );
        let tipos = tipos_de_columnas(&t);
        let s = |n: &str| tipos.get(n).map(|t| t.to_string());
        assert_eq!(s("employee_id").as_deref(), Some("String"));
        assert_eq!(s("salary").as_deref(), Some("Decimal"));
        assert_eq!(s("hired_on").as_deref(), Some("Date"));
        assert_eq!(s("bonus").as_deref(), Some("Money<EUR, 2>"));
        assert_eq!(s("address"), None, "la cita sola no tipa");
        assert_eq!(s("raw"), None);
        assert_eq!(s("mal"), None, "un tipo que no es de OOS no se inventa");
        assert_eq!(tipos.len(), 4);
    }

    /// Un tema: se escribe, no se pregunta. Y solo anexa, para `OOS2021`.
    fn topico(modo: &str) -> Loaded {
        tabla(
            "orders",
            &format!(
                "  datasource: erp\n  object: orders.v2\n  \
                 columns:\n    order_id: {{}}\n    total: {{}}\n  \
                 reads: none\n  changes: {{ mode: {modo}, witness: log }}\n"
            ),
        )
    }

    fn vista8(nombre: &str, spec: &str) -> Loaded {
        doc(
            Kind::View,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: {{ name: {nombre}, namespace: hr }}\nspec:\n  owner: team:hr\n{spec}"
            ),
        )
    }

    fn codigos(pkg: &Package) -> Vec<Code> {
        let mut out = Vec::new();
        comprobar(pkg, &mut out);
        out.into_iter().map(|d| d.code).collect()
    }

    /// La cadena llega al suelo por el camino nuevo, y **lo que llega es lo
    /// mismo**: quien llama a `raiz()` no se entera de por cuál de los dos vino.
    #[test]
    fn la_raiz_atraviesa_una_tabla_y_da_la_misma_forma() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employees }\n  fields:\n    id: employee_id\n    dni: national_id\n  \
             where: { deleted: 'false' }\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        let r = raiz(&pkg, pkg.view("hr.empleados").unwrap()).unwrap();
        assert_eq!(r.datasource, "erp");
        assert_eq!(r.objeto, "public.employees");
        assert_eq!(
            r.columnas.get("dni").map(String::as_str),
            Some("national_id")
        );
        assert_eq!(
            r.filtros,
            vec![("deleted".to_string(), vec!["false".to_string()])]
        );
        // Lo único que cambia: ahora hay un documento que nombrar.
        assert_eq!(r.tabla.as_deref(), Some("erp.employees"));
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// **OOS2018 llega hasta el suelo.** En v1alpha7 esto compilaba, y no por
    /// indulgencia: no había ningún documento contra el que comprobarlo.
    #[test]
    fn un_campo_que_no_es_columna_de_la_tabla_no_compila() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employees }\n  fields:\n    dni: nif\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2018]);
    }

    #[test]
    fn un_filtro_que_no_es_columna_de_la_tabla_no_compila() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employees }\n  fields:\n    id: employee_id\n  where: { borrado: 'false' }\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2018]);
    }

    #[test]
    fn una_tabla_que_no_existe_no_es_una_raiz() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employes }\n  fields:\n    id: employee_id\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2018]);
    }

    /// Las dos caras nombran columnas suyas, y eso un esquema no lo puede
    /// mirar: exige que el campo ESTÉ, no puede saber si lo que dice EXISTE.
    #[test]
    fn las_dos_caras_nombran_columnas_de_la_tabla() {
        let mala = tabla(
            "orders",
            "  datasource: erp\n  object: orders.v2\n  columns:\n    order_id: {}\n  \
             reads: { requiredFilters: [tenant_id] }\n  \
             changes: { mode: upsert, key: [order_key], witness: field, field: updated_at }\n",
        );
        let pkg = paquete(vec![config(), mala]);
        // Tres nombres inventados, tres diagnósticos, un solo código.
        assert_eq!(
            codigos(&pkg),
            vec![Code::Oos2018, Code::Oos2018, Code::Oos2018]
        );
    }

    #[test]
    fn la_fuente_de_una_tabla_se_declara_en_el_manifiesto() {
        let t = doc(
            Kind::Table,
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: a, namespace: crm }\nspec:\n  \
             datasource: salesforce\n  object: Account\n  columns:\n    Id: {}\n  \
             reads: none\n  changes: { mode: none, witness: none }\n",
        );
        let pkg = paquete(vec![config(), t]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2004]);
    }

    /// **OOS2020 · lo que no se puede leer se debe materializar.**
    #[test]
    fn una_vista_virtual_sobre_algo_que_no_se_lee_no_compila() {
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n",
        );
        let pkg = paquete(vec![config(), topico("upsert"), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2020]);
    }

    #[test]
    fn con_la_copia_puesta_la_misma_vista_compila() {
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n  \
             materialized: { datasource: erp, table: cache.pedidos }\n",
        );
        let pkg = paquete(vec![config(), topico("upsert"), v]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    // ── OOS2029 · la palabra que le faltaba a `reads` ──────────────────────

    /// Una tabla que **confiesa** que no empuja la proyección.
    fn sin_proyeccion() -> Loaded {
        tabla(
            "orders",
            "  datasource: erp\n  object: public.orders\n  \
             columns:\n    order_id: {}\n    dni: {}\n  \
             reads: { fullScan: cheap, projectionPushdown: false }\n  \
             changes: { mode: none, witness: none }\n",
        )
    }

    /// **La copia no compila.** Lo que se sellaría con la clasificación de los
    /// campos de la vista no es lo que se movió: sale la fila entera.
    #[test]
    fn una_copia_desde_algo_que_no_proyecta_no_compila() {
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n  \
             materialized: { datasource: erp, table: cache.pedidos }\n",
        );
        let pkg = paquete(vec![config(), sin_proyeccion(), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2029]);
    }

    /// **Y la misma vista, virtual, sí.** Una lectura mueve la fila al proceso
    /// del lector y ahí se acaba; no queda artefacto que clasificar.
    #[test]
    fn la_misma_vista_virtual_si_compila() {
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n",
        );
        let pkg = paquete(vec![config(), sin_proyeccion(), v]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// Y omitir la palabra **no** la cierra: el protocolo exige empujar la
    /// proyección, así que `false` es una confesión y no una opción. Una tabla
    /// que no dice nada la empuja.
    #[test]
    fn omitir_la_palabra_no_es_confesarla() {
        let t = tabla(
            "orders",
            "  datasource: erp\n  object: public.orders\n  \
             columns:\n    order_id: {}\n  \
             reads: { fullScan: cheap }\n  changes: { mode: none, witness: none }\n",
        );
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n  \
             materialized: { datasource: erp, table: cache.pedidos }\n",
        );
        let pkg = paquete(vec![config(), t, v]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    // ── OOS2023 · la pareja decide la garantía ─────────────────────────────

    /// Una tabla fechada por columna, con el modo que se le pase.
    fn por_columna(modo: &str, clave: &str) -> Loaded {
        tabla(
            "clicks",
            &format!(
                "  datasource: erp\n  object: public.clicks\n  \
                 columns:\n    click_id: {{}}\n    ocurrio_en: {{}}\n  \
                 reads: {{ fullScan: cheap }}\n  \
                 changes: {{ mode: {modo}, {clave}witness: field, field: ocurrio_en }}\n"
            ),
        )
    }

    fn copia_de_clicks() -> Loaded {
        vista8(
            "clics",
            "  from: { table: erp.clicks }\n  fields:\n    id: click_id\n    \
             cuando: ocurrio_en\n  materialized: { datasource: erp, table: cache.clics }\n",
        )
    }

    /// **`{ witness: field, mode: append }` no se puede mantener.**
    ///
    /// Fechar por una columna es at-least-once —la columna es mayor o IGUAL que
    /// sí misma, así que el borde se re-entrega— y sin clave no hay con qué
    /// quitarlo. El solape se acumula **para siempre**, y no da ningún síntoma:
    /// la copia responde, los números salen, y son de más.
    ///
    /// Antes de este código el árbol no solo lo aceptaba: `ore view`
    /// **recomendaba `INCREMENTAL`** sobre esta pareja exacta.
    #[test]
    fn una_copia_fechada_por_columna_y_solo_anexa_no_compila() {
        let pkg = paquete(vec![config(), por_columna("append", ""), copia_de_clicks()]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2023]);
    }

    /// **Y el rechazo es de la COMBINACIÓN, no del modo ni del testigo.**
    ///
    /// Sin esta prueba, `OOS2023` podría estar mirando solo `witness: field` y
    /// nadie lo notaría: prohibiría fechar por columna, que es legítimo y es lo
    /// único que muchos orígenes saben hacer. Con clave, re-entregar es
    /// idempotente y no pasa nada.
    #[test]
    fn la_misma_tabla_con_clave_si_se_copia() {
        let pkg = paquete(vec![
            config(),
            por_columna("upsert", "key: [click_id], "),
            copia_de_clicks(),
        ]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// **Y es sobre la copia, no sobre la tabla.** Un log de eventos fechado por
    /// una columna de tiempo es legítimo y existe; lo que no se puede es
    /// mantener una copia suya. Sin `materialized`, no hay nada que rechazar.
    #[test]
    fn la_tabla_que_solo_anexa_es_legitima_mientras_nadie_la_copie() {
        let virtual_ = vista8(
            "clics",
            "  from: { table: erp.clicks }\n  fields:\n    id: click_id\n",
        );
        let pkg = paquete(vec![config(), por_columna("append", ""), virtual_]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// La distinción raíz / raíz de lectura, que es todo el asunto: si la regla
    /// mirara la raíz, esto fallaría y obligaría a copiar dos veces lo mismo.
    #[test]
    fn una_virtual_sobre_una_copia_sobre_un_topico_lee_de_la_copia() {
        let abajo = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n    total: total\n  \
             materialized: { datasource: erp, table: cache.pedidos }\n",
        );
        let arriba = vista8(
            "iberia",
            "  from: { view: pedidos }\n  fields:\n    id: id\n",
        );
        let pkg = paquete(vec![config(), topico("upsert"), abajo, arriba]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
        // Y la raíz de lectura de la de arriba es la copia, no el tópico.
        let a = pkg.view("hr.iberia").unwrap();
        assert_eq!(
            raiz_de_lectura(&pkg, a).and_then(|v| v.qname()).as_deref(),
            Some("hr.pedidos")
        );
    }

    fn entidad(naturaleza: &str, extra: &str) -> Loaded {
        doc(
            Kind::Entity,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: {{ name: Pedido, namespace: hr }}\nspec:\n  nature: {naturaleza}\n\
                 {extra}  backedBy: pedidos\n  properties:\n    id: {{ type: String }}\n"
            ),
        )
    }

    fn copia_de(modo: &str) -> Vec<Loaded> {
        vec![
            config(),
            topico(modo),
            vista8(
                "pedidos",
                "  from: { table: erp.orders }\n  fields:\n    id: order_id\n  \
                 materialized: { datasource: erp, table: cache.pedidos }\n",
            ),
        ]
    }

    /// **OOS2021 · sin retractación no se mantiene lo mutable.** El peor modo
    /// de fallo del motor: sin este código la copia se deriva en silencio y los
    /// números salen — los de antes.
    #[test]
    fn una_copia_de_lo_que_solo_anexa_no_respalda_una_entidad_mutable() {
        let mut docs = copia_de("append");
        docs.push(entidad("entity", "  primaryKey: [id]\n"));
        let pkg = paquete(docs);
        assert_eq!(codigos(&pkg), vec![Code::Oos2021]);
    }

    /// Y la mitad que **sí** compila: un hecho ocurrido no se retira.
    #[test]
    fn una_copia_de_lo_que_solo_anexa_si_respalda_un_evento() {
        let mut docs = copia_de("append");
        docs.push(entidad("event", "  timeKey: id\n"));
        let pkg = paquete(docs);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// Y con retractación, lo mutable se mantiene: la regla es sobre el modo,
    /// no sobre materializar.
    #[test]
    fn una_copia_de_lo_que_retracta_si_respalda_una_entidad() {
        let mut docs = copia_de("retract");
        docs.push(entidad("entity", "  primaryKey: [id]\n"));
        let pkg = paquete(docs);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// Una vista VIRTUAL sobre un `append` que se deja leer compila: se lee del
    /// origen, que sí tiene el estado presente. La regla es sobre la COPIA.
    #[test]
    fn sin_copia_un_append_si_respalda_una_entidad() {
        let plana = tabla(
            "orders",
            "  datasource: erp\n  object: orders.v2\n  columns:\n    order_id: {}\n  \
             reads: { fullScan: cheap }\n  changes: { mode: append, witness: log }\n",
        );
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n",
        );
        let pkg = paquete(vec![
            config(),
            plana,
            v,
            entidad("entity", "  primaryKey: [id]\n"),
        ]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// Las dos versiones en el mismo paquete. Si esto fallara, la migración
    /// sería un salto, y un salto sobre un árbol grande no se da.
    #[test]
    fn una_vista_v1alpha7_y_una_tabla_v1alpha8_conviven() {
        let pkg = paquete(vec![config(), base(), employees(), {
            vista8(
                "nuevos",
                "  from: { table: erp.employees }\n  fields:\n    id: employee_id\n",
            )
        }]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
        // Y las dos llegan al mismo suelo por caminos distintos.
        let vieja = raiz(&pkg, pkg.view("hr.empleados").unwrap()).unwrap();
        let nueva = raiz(&pkg, pkg.view("hr.nuevos").unwrap()).unwrap();
        assert_eq!(vieja.datasource, nueva.datasource);
        assert_eq!(vieja.objeto, nueva.objeto);
        assert_eq!(vieja.tabla, None);
        assert_eq!(nueva.tabla.as_deref(), Some("erp.employees"));
    }
}
