//! El inductor: de un **catálogo** a un paquete en `DRAFT`.
//!
//! # La costura, y por qué está aquí
//!
//! `discover` son dos actos, igual que `source add` lo era: **leer un catálogo**
//! y **proponer una ontología**. Este módulo es el segundo, y es puro — sin red,
//! sin credenciales, sin driver. Lo que sabe del origen se lo cuenta el catálogo;
//! lo que sabe de OOS lo sabe él.
//!
//! Cada uno sabe **una** cosa: el lector conoce el sistema de tipos de su fuente,
//! el inductor conoce el de OOS. Por eso el catálogo llega con tipos de OOS ya
//! traducidos: si llegara con `NUMERIC` o `int8`, este fichero tendría que saber
//! de BigQuery y de Postgres, y la costura no serviría de nada.
//!
//! # La regla que gobierna todo lo de abajo
//!
//! > **Se emite lo que es un hecho. Se reporta lo que es una conjetura.**
//!
//! Una tabla es una entidad: es un hecho. Una columna llamada `id_cliente` que
//! *parece* apuntar a `clientes` es una conjetura, y `01-package` §5 fija qué
//! hacer con ella — *la decisión pendiente se marca; **NO DEBE** inventarse*.
//!
//! Y hay una consecuencia que conviene ver venir: **lo inducido no compila.**
//! Una entidad sin clave primaria falla con `OOS2010`, y está bien que falle —
//! inventar la clave sería lo único peor. Las decisiones pendientes **son los
//! diagnósticos**: `ore validate` es la cola de revisión dicha en la voz del
//! compilador, y `ore review` es su cara interactiva.
//!
//! # Y por eso inducir es una función de DOS cosas
//!
//! `inducir_con(catálogo, decisiones)`. Contestar no retoca un documento
//! emitido: vuelve a inducir con la decisión puesta, porque hay respuestas que no
//! caben en una edición local —resolver una colisión crea dos entidades donde no
//! había ninguna— y porque un documento retocado deja de estar garantizado por lo
//! que este módulo garantiza. Lo que sale de aquí es siempre una inducción de
//! algo. El detalle está en `revision.rs`.
//!
//! # Lo que NO hace, y no por falta de tiempo
//!
//! **No acuña conceptos.** Un nombre de columna repetido en tres tablas es una
//! *candidata* a concepto, no un concepto: acuñar uno por columna repetida es la
//! inflación que `02-property` §6.2 nombra —*cuatro mil columnas producen cuatro
//! mil conceptos, que es igual que no tener vocabulario*—. Se reporta para que la
//! unificación se decida **una vez y no quince**, y el concepto lo acuña **quien
//! contesta**: entonces sí se escribe, porque `is` exige que exista.
//!
//! **No singulariza ni convierte a camelCase.** `pedidos` da `Pedidos`, no
//! `Pedido`: singularizar es adivinar un idioma. Y `id_pedido` se queda como
//! está, porque renombrarlo rompería la correspondencia con el nombre físico a
//! cambio de estética. Renombrar es de `review`, donde hay un humano.

use crate::vocabulario::Vocabulario;
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

// ── El catálogo ────────────────────────────────────────────────
//
// **La forma ya no vive aquí.** Subía a `ore_driver::catalogo`, con su censo y
// su emisor, porque tenerla aquí significaba que la única definición del
// contrato era el consumidor: los cuatro productores escriben JSON a pelo, y lo
// que compartían no era un tipo —era haber leído este fichero—.

pub use ore_driver::catalogo::Catalogo;
use ore_driver::catalogo::Tabla;

/// Un objeto del origen y las columnas que tiene **dentro**.
///
/// Casi siempre son las de su tabla. Cuando una respuesta une una familia
/// fechada no lo son: la hermana de 2019 puede no tener la columna que se añadió
/// en 2024, y un binding que se la atribuyera sería un mapeo verde y falso.
struct Objeto {
    nombre: String,
    columnas: Vec<String>,
}

/// **Derivado, y por eso ya no es un campo.**
///
/// El catálogo traía `objetos`, y desde que el binding se retiró siempre tenía
/// exactamente UNO, cuyo nombre y columnas eran los de la tabla. Un campo que se
/// puede computar y aun así se declara es una oportunidad de escribirlo mal
/// —P2—, así que se computa.
fn objeto_de(t: &Tabla) -> Objeto {
    Objeto {
        nombre: t.nombre.clone(),
        columnas: t.columnas.iter().map(|c| c.nombre.clone()).collect(),
    }
}

fn lista_de(n: &Node) -> Vec<String> {
    n.items()
        .iter()
        .filter_map(|i| i.as_str())
        .map(String::from)
        .collect()
}

// ── Las decisiones ──────────────────────────────────────────────────────────

/// Las clases de pregunta que el inductor sabe hacer, y son **todas** las que
/// sabe hacer.
///
/// Es una taxonomía cerrada a propósito: `ore review` tiene un formulario por
/// clase y el `match` que los reparte es exhaustivo. Una clase nueva sin
/// formulario no compila, que es la única forma de que una pregunta no se quede
/// sin nadie que la sepa contestar.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Clase {
    /// Dos tablas dan el mismo identificador de OOS.
    Colision,
    /// El origen no declara clave primaria.
    Clave,
    /// El lector no supo traducir el tipo de una columna.
    Tipo,
    /// Ninguna columna se pudo tipar: no hay entidad que escribir.
    Vacio,
    /// El origen la declara vista, y una vista es una proyección.
    Vista,
    /// Cero filas: ¿viva y vacía, o un resto?
    Filas,
    /// Una columna repetida entre tablas: ¿el mismo concepto?
    Concepto,
    /// Un parecido de nombres que podría ser una relación.
    Relacion,
    /// Hermanas numeradas: una familia fragmentada por fecha.
    Familia,
    /// Quién responde por el paquete. No la hace el catálogo —una base de datos
    /// no sabe de equipos— y aun así hay que tomarla: `spec.owner` es
    /// obligatorio, el inductor escribe `cambiame` porque no puede inventar un
    /// handle, y `cambiame` **no valida**. Era la única decisión que quedaba
    /// entre contestar la cola y un paquete en verde, y no estaba en la cola.
    Dueno,
    /// Cómo se clasifica un concepto **recién acuñado**.
    ///
    /// Es la segunda mitad de la séptima pregunta y no un adorno: la etiqueta de
    /// un concepto es la tercera fuente de herencia de la clasificación efectiva,
    /// y la clasificación efectiva es lo que poda la superficie emitida. Un
    /// concepto sin etiquetas **no gobierna nada** — la columna que lo habla sale
    /// servida en el SDL exactamente igual que si nadie hubiera contestado. Un
    /// concepto acuñado sin esta pregunta sería una decisión con aspecto de
    /// tomada.
    Clasificacion,
}

/// **Cómo se contesta** una clase de pregunta. Vive aquí, junto a `Clase`, y no
/// junto al formulario que la pinta: es una propiedad de la PREGUNTA, no de
/// quien la hace. `ore review` la usa para leer una línea; `informe_json` la
/// sirve para que un cliente sepa qué control dibujar.
///
/// ⛔ Y el `match` de [`Clase::forma`] es exhaustivo a propósito: **una clase
/// nueva no compila** hasta que alguien decide cómo se contesta. Es la única
/// forma de que el inductor no pueda estrenar una pregunta que nadie sabe
/// responder — y desde que se sirve por JSON, eso alcanza también a una
/// interfaz que todavía no existe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Forma {
    /// Una palabra: un tipo, un concepto, `si`, `no`, `omitir`.
    Palabra,
    /// Varias columnas: una clave primaria.
    Columnas,
    /// Un nombre por cada sujeto: una colisión.
    Nombres,
    /// `eje: nivel`, una o varias: la clasificación de un concepto.
    Etiquetas,
}

impl Forma {
    /// El nombre que viaja por el cable. En minúsculas y sin acentos porque lo
    /// lee un cliente, no una persona.
    pub const fn nombre(self) -> &'static str {
        match self {
            Forma::Palabra => "palabra",
            Forma::Columnas => "columnas",
            Forma::Nombres => "nombres",
            Forma::Etiquetas => "etiquetas",
        }
    }
}

impl Clase {
    /// El prefijo del identificador de una decisión de esta clase.
    ///
    /// **Es interfaz.** La izquierda de cada línea de un fichero de respuestas
    /// sale de aquí, así que cambiarla invalida los ficheros ya escritos —
    /// igual que cambiar el nombre de una opción de la línea de órdenes.
    /// Cómo se contesta esta clase. Ver [`Forma`].
    pub const fn forma(self) -> Forma {
        match self {
            Clase::Colision => Forma::Nombres,
            Clase::Clasificacion => Forma::Etiquetas,
            Clase::Clave => Forma::Columnas,
            Clase::Tipo
            | Clase::Vacio
            | Clase::Vista
            | Clase::Filas
            | Clase::Concepto
            | Clase::Relacion
            | Clase::Familia
            | Clase::Dueno => Forma::Palabra,
        }
    }

    pub const fn prefijo(self) -> &'static str {
        match self {
            Clase::Colision => "colision",
            Clase::Clave => "clave",
            Clase::Tipo => "tipo",
            Clase::Vacio => "vacio",
            Clase::Vista => "vista",
            Clase::Filas => "filas",
            Clase::Concepto => "concepto",
            Clase::Relacion => "relacion",
            Clase::Familia => "familia",
            Clase::Dueno => "dueno",
            Clase::Clasificacion => "clasificacion",
        }
    }
}

/// La palabra que significa *no lo emitas*. Una sola en todo el vocabulario de
/// respuestas: dos sinónimos para una decisión son dos formas de escribirla mal.
pub const OMITIR: &str = "omitir";

/// Lo que alguien contesta a una pregunta.
///
/// Tres formas y no una porque las preguntas no son la misma clase de cosa: una
/// clave primaria es una **lista** de columnas, una colisión es un nombre **por
/// cada** tabla, y el resto caben en una palabra.
#[derive(Clone, Debug, PartialEq)]
pub enum Respuesta {
    Palabra(String),
    Lista(Vec<String>),
    Mapa(BTreeMap<String, String>),
}

impl Respuesta {
    pub fn palabra(&self) -> Option<&str> {
        match self {
            Respuesta::Palabra(p) => Some(p.as_str()),
            _ => None,
        }
    }
    fn es(&self, que: &str) -> bool {
        self.palabra().is_some_and(|p| p.eq_ignore_ascii_case(que))
    }
    fn omite(&self) -> bool {
        self.es(OMITIR)
    }
}

/// Las respuestas dadas, por identificador de decisión.
///
/// No son un estado aparte: se consumen, y lo que producen son **los ficheros
/// inducidos**. Que inducir sea una función de `(catálogo, decisiones)` es lo
/// que hace reproducible la revisión — el mismo catálogo y las mismas respuestas
/// dan el mismo paquete, byte a byte, sin volver a tocar la fuente.
#[derive(Default)]
pub struct Decisiones(BTreeMap<String, Respuesta>);

impl Decisiones {
    /// Lee un fichero de respuestas: un mapa `answers` de identificador a lo que
    /// se contesta. Se analiza con el analizador de YAML, así que un JSON vale
    /// igual — es un subconjunto (ADR 0002).
    pub fn leer(texto: &str) -> Result<Self, String> {
        let raiz = parse::parse(texto).map_err(|e| format!("no analiza: {e:?}"))?;
        let Some((_, mapa)) = raiz.get("answers") else {
            return Err("no hay un mapa `answers` en la raíz".into());
        };
        let mut out = BTreeMap::new();
        for (k, v) in mapa.entries() {
            let Some(id) = k.as_str() else { continue };
            let r = if let Some(s) = v.as_str() {
                Respuesta::Palabra(s.to_string())
            } else if !v.entries().is_empty() {
                Respuesta::Mapa(
                    v.entries()
                        .iter()
                        .filter_map(|(a, b)| {
                            Some((a.as_str()?.to_string(), b.as_str()?.to_string()))
                        })
                        .collect(),
                )
            } else {
                Respuesta::Lista(lista_de(v))
            };
            out.insert(id.to_string(), r);
        }
        Ok(Decisiones(out))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }

    pub fn responder(&mut self, id: impl Into<String>, r: Respuesta) {
        self.0.insert(id.into(), r);
    }

    /// Funde otras respuestas sobre estas. Las de `otras` mandan: son las que
    /// alguien acaba de dar, y contestar otra vez una pregunta ya contestada es
    /// **cambiar de opinión**, que es una cosa legítima y tiene que poder
    /// hacerse sin editar un fichero a mano.
    pub fn fundir(&mut self, otras: Decisiones) {
        self.0.extend(otras.0);
    }

    /// Las respuestas, en el mismo formato que `--answers` lee.
    ///
    /// **En JSON, y el nombre del fichero importa.** `ore validate` carga todo
    /// `.yaml` del árbol y le exige `apiVersion`: un registro de respuestas con
    /// esa extensión rompe el paquete al que pertenece, y lo dijo ejecutarlo.
    /// Los tres apuntes que `discover` deja al lado —catálogo, cola y
    /// respuestas— son `.json` por lo mismo: no son documentos de la ontología y
    /// no pueden parecerlo.
    ///
    /// Se escribe con el emisor de JSON de `ore-core`, así que las claves salen
    /// ordenadas y el fichero es estable: dos revisiones con las mismas
    /// respuestas producen los mismos bytes.
    pub fn json(&self) -> Json {
        Json::obj([(
            "answers",
            Json::Obj(
                self.0
                    .iter()
                    .map(|(id, r)| {
                        let v = match r {
                            Respuesta::Palabra(p) => Json::s(p),
                            Respuesta::Lista(v) => Json::Arr(v.iter().map(Json::s).collect()),
                            Respuesta::Mapa(m) => {
                                Json::Obj(m.iter().map(|(k, v)| (k.clone(), Json::s(v))).collect())
                            }
                        };
                        (id.clone(), v)
                    })
                    .collect(),
            ),
        )])
    }

    fn de(&self, id: &str) -> Option<&Respuesta> {
        self.0.get(id)
    }

    /// `true` si alguien contestó `omitir` a esta decisión.
    fn omite(&self, id: &str) -> bool {
        self.de(id).is_some_and(Respuesta::omite)
    }
}

/// El identificador de una decisión, tal y como se escribe en las respuestas.
fn id(clase: Clase, sujeto: &str) -> String {
    format!("{}/{sujeto}", clase.prefijo())
}

// ── Lo inducido ─────────────────────────────────────────────────────────────

/// Una decisión que el inductor **no toma**.
pub struct Pendiente {
    /// Lo que se escribe a la izquierda en un fichero de respuestas. Se deriva
    /// de la clase y del sujeto, así que es estable entre ejecuciones: volver a
    /// descubrir no invalida las respuestas ya escritas.
    pub id: String,
    pub clase: Clase,
    pub sujeto: String,
    pub que: String,
    pub porque: String,
    /// Lo que se puede contestar. Vacío significa **texto libre**, y lo que vale
    /// ahí lo dice el formulario de su clase.
    pub opciones: Vec<String>,
}

fn pendiente(
    clase: Clase,
    sujeto_id: &str,
    sujeto: impl Into<String>,
    que: impl Into<String>,
    porque: impl Into<String>,
    opciones: Vec<String>,
) -> Pendiente {
    Pendiente {
        id: id(clase, sujeto_id),
        clase,
        sujeto: sujeto.into(),
        que: que.into(),
        porque: porque.into(),
        opciones,
    }
}

pub struct Induccion {
    pub ficheros: BTreeMap<String, String>,
    pub pendientes: Vec<Pendiente>,
    /// Respuestas que no corresponden a ninguna decisión de este catálogo. Se
    /// dicen en vez de ignorarse: una respuesta que no llega a ninguna parte
    /// tiene exactamente el mismo aspecto que una que sí.
    pub huerfanas: Vec<String>,
}

/// Inducir sin nada decidido y sin vocabulario publicado.
///
/// Solo lo usan las comprobaciones, y por eso no está en el binario: en el
/// camino real siempre hay un repositorio del que leer conceptos, aunque sea
/// para no encontrar ninguno.
#[cfg(test)]
pub fn inducir(cat: &Catalogo, paquete: &str) -> Induccion {
    inducir_con(
        cat,
        paquete,
        &Decisiones::default(),
        &Vocabulario::default(),
    )
}

/// Induce un paquete **con las decisiones que alguien ya contestó**.
///
/// `paquete` es el espacio de nombres y el nombre del `Package`. El resultado no
/// se escribe: se devuelve, para que quien llama decida —y para que esto se
/// pueda comprobar sin tocar el disco.
///
/// El orden de los pasos no es estético. Unir una familia fechada cambia
/// **cuántas tablas hay** y todo lo demás cuenta tablas; cerrar el tipo de una
/// columna cambia si su tabla tiene algo que emitir. Por eso las decisiones se
/// aplican **al catálogo** y no al resultado: lo que sale de aquí es siempre una
/// inducción de algo, nunca una inducción retocada.
#[cfg(test)]
pub fn inducir_con(
    cat: &Catalogo,
    paquete: &str,
    dec: &Decisiones,
    voc: &Vocabulario,
) -> Induccion {
    inducir_con_regla(cat, paquete, dec, voc, &Regla::default())
}

/// **Lo que el alcance manda sobre la inducción** (ORE 0027 P1): dos reglas que
/// alguien declaró, y que el inductor aplica en vez de proponer.
#[derive(Default, Clone)]
pub struct Regla {
    /// La base es estándar: cada tabla sale con su `Dataset` (0033).
    pub estandar: bool,
    /// Qué tablas se modelan (`Entity` y su cola). `None` = todas.
    pub modeladas: Option<BTreeSet<String>>,
    /// Qué tablas se copian una a una en una base foránea.
    pub copiadas: BTreeSet<String>,
    /// Los schemas renombrados del alcance (0038 P6): el del origen → el del
    /// paquete. Sólo cambia DÓNDE y CÓMO se llama lo emitido; las preguntas
    /// siguen con el schema del origen —son del origen—, y sus respuestas
    /// siguen valiendo.
    pub schemas: BTreeMap<String, String>,
}

impl Regla {
    fn modela(&self, tabla: &str) -> bool {
        self.modeladas.as_ref().is_none_or(|m| m.contains(tabla))
    }

    /// ¿Se copia esta tabla? Por la clase, o una a una.
    fn copia(&self, tabla: &str) -> bool {
        self.estandar || self.copiadas.contains(tabla)
    }

    /// El schema de una tabla EN EL PAQUETE: el del origen, o el nombre que
    /// alguien le dio.
    fn schema_de(&self, tabla: &str) -> String {
        let s = schema_de(tabla);
        self.schemas.get(&s).cloned().unwrap_or(s)
    }
}

/// **Con las reglas del alcance** (ORE 0027 P1).
///
/// **El catálogo no modela** (C1): una tabla del alcance da siempre su `Table`
/// (el puntero físico, con todas las columnas y su `physicalType`) y su `View`
/// trivial; la `Entity` —y las decisiones de modelado: colisión, clave, tipo,
/// vacío, vista, concepto, relación, familia— sólo si el alcance la nombra en
/// `entities`. Es el reparto de un catálogo de activos y una ontología: una
/// fila se copia sin identidad; se **modela** con ella.
///
/// **La base estándar** (I4b): `estandar` es «todo lo que entra se copia a la
/// celda», y el inductor lo APLICA: cada tabla sale con un **`Dataset`** que
/// lleva el plan (0033) —y ninguna `View` que sea la misma cosa—. Con
/// clave —la del origen o la contestada— la tabla pasa a `upsert` y el refresco
/// puede ser por diferencia; sin ella la copia es una instantánea que se
/// sustituye entera. La única que espera es la de una tabla **modelada** sin
/// clave: una copia que sólo anexa no puede respaldar una entidad (`OOS2021`),
/// y la decisión `clave` lo dice.
///
/// No contradice lo de arriba —«la copia no se propone»—: no se propone, se
/// deriva de una regla que alguien declaró. Y por eso sobrevive a `review`:
/// lo que sale de aquí es siempre `inducir(catálogo, alcance, respuestas)`.
pub fn inducir_con_regla(
    cat: &Catalogo,
    paquete: &str,
    dec: &Decisiones,
    voc: &Vocabulario,
    regla: &Regla,
) -> Induccion {
    let estandar = regla.estandar;
    let mut ficheros = BTreeMap::new();
    let mut pendientes = Vec::new();

    // ── El catálogo: lo que NO se modela ────────────────────────────────────
    //
    // Sale antes y aparte: no pasa por familias, colisiones, tipos ni claves.
    // Lo único que se le pregunta es si una tabla con cero filas entra.
    let (sin_modelar, con_modelo): (Vec<Tabla>, Vec<Tabla>) = cat
        .tablas
        .iter()
        .cloned()
        .partition(|t| !regla.modela(&t.nombre));
    let cat = Catalogo {
        fuente: cat.fuente.clone(),
        tablas: con_modelo,
    };
    // El nombre de la vista sin modelar es el que tendría su entidad; si dos
    // tablas del alcance lo comparten, el físico entero. Sin decisión: cuando
    // se modele, `colision` la nombra y la vista la sigue.
    let mut cuantos: BTreeMap<String, usize> = BTreeMap::new();
    // Por schema (0038 P5): dos `pedidos` en dos schemas no chocan.
    for t in sin_modelar.iter().chain(cat.tablas.iter()) {
        *cuantos
            .entry(format!("{}/{}", schema_de(&t.nombre), entidad(&t.nombre)))
            .or_default() += 1;
    }
    let owner_catalogo = dec
        .de(&id(Clase::Dueno, paquete))
        .and_then(Respuesta::palabra)
        .filter(|h| handle(h))
        .unwrap_or("cambiame")
        .to_string();
    for t in &sin_modelar {
        if dec.omite(&id(Clase::Filas, &t.nombre)) {
            continue;
        }
        if t.filas == Some(0) && dec.de(&id(Clase::Filas, &t.nombre)).is_none() {
            pendientes.push(pendiente(
                Clase::Filas,
                &t.nombre,
                &t.nombre,
                "cero filas",
                "puede ser una tabla viva y vacía o un resto. El inductor no \
                 distingue una cosa de la otra, y borrarla sería decidirlo",
                vec!["mantener".into(), OMITIR.into()],
            ));
        }
        let base = entidad(&t.nombre);
        let sch = regla.schema_de(&t.nombre);
        let vista = if cuantos
            .get(&format!("{}/{base}", schema_de(&t.nombre)))
            .copied()
            .unwrap_or(0)
            > 1
        {
            identificador(sin_schema(&t.nombre))
        } else {
            minuscula_inicial(&base)
        };
        let objeto = &objeto_de(t);
        let sufijo = format!(
            "{}__{}.yaml",
            capitalizar(&vista),
            identificador(sin_schema(&objeto.nombre))
        );
        // Sin entidad no hay a quién respaldar: la copia no espera a nada. Con
        // clave del origen, `upsert`; sin ella, instantánea.
        let clave = clave_de(t, dec);
        let se_copia = regla.copia(&t.nombre);
        let copia = se_copia.then_some(if clave.is_empty() { None } else { Some(clave) });
        ficheros.insert(
            en_schema(&sch, format!("tables/{sufijo}")),
            con_schema(
                tabla_yaml(
                    paquete,
                    &cat.fuente,
                    t,
                    objeto,
                    copia.as_ref().and_then(|c| c.as_deref()),
                ),
                &sch,
                paquete,
            ),
        );
        // 0033: lo que se copia es un `Dataset` con el plan de la vista dentro;
        // la vista sólo existe cuando NO se copia (la pregunta sobre lo de fuera).
        ficheros.insert(
            en_schema(
                &sch,
                format!("{}/{sufijo}", if se_copia { "datasets" } else { "views" }),
            ),
            con_schema(
                vista_yaml(&vista, paquete, &sch, &owner_catalogo, t, objeto, se_copia),
                &sch,
                paquete,
            ),
        );
    }
    let cat = &cat;

    // ⓪ El dueño, antes que nada: desde v1alpha8 lo llevan DOS documentos —el
    //    paquete y cada vista— y sigue siendo UNA decisión. Resolverlo dentro
    //    de cada emisor lo habría convertido en dos preguntas que se pueden
    //    contestar distinto.
    let (package_yaml, pend_paquete) = paquete_yaml(paquete, dec);
    let owner = dec
        .de(&id(Clase::Dueno, paquete))
        .and_then(Respuesta::palabra)
        .filter(|h| handle(h))
        .unwrap_or("cambiame")
        .to_string();

    // ① Las familias fechadas.
    let (tablas, pend) = familias(&cat.tablas, dec);
    pendientes.extend(pend);

    // ② Los tipos que una respuesta cerró: una columna con tipo ya no es una
    //    conjetura, es un hecho que alguien firmó.
    let tablas: Vec<Tabla> = tablas.into_iter().map(|t| con_tipos(&t, dec)).collect();

    // ③ Los conceptos: los que ya existían y alguien eligió, y los que una
    //    respuesta acuñó.
    let c = conceptos(&tablas, paquete, dec, voc);
    ficheros.extend(c.ficheros);
    pendientes.extend(c.pendientes);
    let mapeo = c.mapeo;

    // ④ Los nombres de entidad, con la colisión resuelta si lo está. Una tabla
    //    que no sale aquí es una que no se emite: o colisiona sin decidir, o
    //    alguien dijo `omitir`.
    let (nombres, pend) = nombres(&tablas, dec);
    pendientes.extend(pend);

    // La clave primaria de cada tabla —la declarada o la decidida—, para saber
    // si una foránea apunta a ella o a otra cosa. Solo cuando apunta a otra cosa
    // hace falta `toKey`: lo derivable no se declara.
    let claves: BTreeMap<String, Vec<String>> = tablas
        .iter()
        .map(|t| (t.nombre.clone(), clave_de(t, dec)))
        .collect();

    for t in &tablas {
        let Some(nombre) = nombres.get(&t.nombre) else {
            continue;
        };
        // Quien confirmó que esta tabla no entra ya no tiene que decir de qué
        // tipo son sus columnas. Seguir preguntándolo es cómo una cola de trece
        // decisiones se queda en dos que ya nadie va a contestar.
        if dec.omite(&id(Clase::Vacio, &t.nombre)) {
            continue;
        }

        // Una vista es una PROYECCION de algo. Puede ser la entidad, o puede ser
        // un informe sobre ella: emitirla sin mas duplicaria el concepto.
        if t.clase != "table" && dec.de(&id(Clase::Vista, &t.nombre)).is_none() {
            pendientes.push(pendiente(
                Clase::Vista,
                &t.nombre,
                &t.nombre,
                format!("el origen la declara `{}`", t.clase),
                "una vista es una proyeccion, y una proyeccion puede ser la \
                 entidad o puede ser un informe sobre ella. Emitirla como \
                 entidad sin mas duplicaria el concepto",
                vec!["entidad".into(), OMITIR.into()],
            ));
        }

        // Una columna cuyo tipo el lector no supo traducir no se emite: no hay
        // tipo que poner, y `Opaque` afirmaria «no hay estructura dentro» de algo
        // que el origen acaba de enumerar. Se nombra, con lo que dijo el origen.
        for c in t.columnas.iter().filter(|c| c.tipo.is_none()) {
            let sujeto = format!("{}.{}", t.nombre, c.nombre);
            if dec.de(&id(Clase::Tipo, &sujeto)).is_some() {
                continue;
            }
            let origen = c
                .origen
                .as_deref()
                .unwrap_or("un tipo que no se sabe traducir");
            let mut opciones: Vec<String> = ore_core::types::escalares()
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            opciones.push(OMITIR.into());
            pendientes.push(pendiente(
                Clase::Tipo,
                &sujeto,
                &sujeto,
                "sin tipo de OOS",
                format!("el origen dice `{origen}`. {}", no_se_traduce(origen)),
                opciones,
            ));
        }

        // Y si NINGUNA columna se pudo tipar, no hay entidad que escribir: un
        // `properties` vacio no valida, y llenarlo seria inventarlo.
        if t.columnas.iter().all(|c| c.tipo.is_none()) {
            if !dec.omite(&id(Clase::Vacio, &t.nombre)) {
                pendientes.push(pendiente(
                    Clase::Vacio,
                    &t.nombre,
                    &t.nombre,
                    "ninguna columna tiene tipo de OOS",
                    "no queda nada que emitir. `properties` exige al menos una, y \
                     rellenarla seria inventar el modelo entero. Se cierra dando tipo \
                     a alguna columna, o confirmando que esta tabla no entra",
                    vec![OMITIR.into()],
                ));
            }
            continue;
        }

        if claves.get(&t.nombre).is_none_or(Vec::is_empty) {
            pendientes.push(pendiente(
                Clase::Clave,
                &t.nombre,
                &t.nombre,
                "sin clave primaria",
                if estandar {
                    "el origen no la declara. `01-package` §5: NO DEBE inferirse — \
                     sin clave no hay identidad, y una identidad inventada es peor \
                     que ninguna. Y la COPIA de esta tabla espera esta clave: la base \
                     es estándar, y una copia sin identidad no puede respaldar la \
                     entidad (OOS2021)"
                } else {
                    "el origen no la declara. `01-package` §5: NO DEBE inferirse — \
                     sin clave no hay identidad, y una identidad inventada es peor \
                     que ninguna"
                },
                t.columnas.iter().map(|c| c.nombre.clone()).collect(),
            ));
        }

        let (extra, pend) = relaciones_decididas(t, &nombres, &claves, dec);
        pendientes.extend(pend);

        // La vista se llama como la entidad, con la inicial en minuscula. No
        // es un nombre inventado: la entidad ya lo tiene decidido —por la regla
        // de nombrado o por quien resolvio la colision— y heredarlo evita pedir
        // una segunda respuesta para la misma cosa.
        let vista = minuscula_inicial(nombre);
        let sch = regla.schema_de(&t.nombre);
        ficheros.insert(
            en_schema(&sch, format!("entities/{nombre}.yaml")),
            con_schema(
                entidad_yaml(
                    nombre,
                    paquete,
                    &vista,
                    t,
                    &Resuelto {
                        claves: &claves,
                        nombres: &nombres,
                        mapeo: &mapeo,
                        schemas: &regla.schemas,
                    },
                    &extra,
                ),
                &sch,
                paquete,
            ),
        );
        // **Uno**, y por eso ya no es un bucle: una vista sale de UN sitio.
        let objeto = &objeto_de(t);
        // El nombre del fichero lleva **la entidad delante**, y no solo la
        // tabla. Se midio perdiendo un documento: `rubix_demo_ventas.Pedidos`
        // y `rubix_demo_ventas.pedidos` son dos tablas y daban dos ficheros
        // que en Windows —y en macOS— SON EL MISMO. El segundo piso al
        // primero, quedo el nombre de uno con el contenido del otro, y
        // `PedidosLegacy` se quedo sin puntero fisico. `ore validate` salio
        // verde, porque una entidad sin fuente es legal en DRAFT.
        //
        // La entidad delante lo cierra sin inventar nada: los nombres de
        // entidad ya son unicos porque **eso es lo que la decision de
        // colision resolvio**, asi que el fichero hereda esa unicidad en vez
        // de pedir una segunda respuesta.
        let sufijo = format!(
            "{}__{}.yaml",
            identificador(nombre),
            identificador(sin_schema(&objeto.nombre))
        );
        // La copia, si la base es estándar y la tabla tiene con qué: la clave
        // del origen o la contestada, que `claves` ya funde. Aquí sí espera:
        // esta vista respalda una entidad, y sin identidad no se mantiene.
        let copia = if regla.copia(&t.nombre) {
            claves.get(&t.nombre).filter(|k| !k.is_empty()).cloned()
        } else {
            None
        };
        ficheros.insert(
            en_schema(&sch, format!("tables/{sufijo}")),
            con_schema(
                tabla_yaml(paquete, &cat.fuente, t, objeto, copia.as_deref()),
                &sch,
                paquete,
            ),
        );
        ficheros.insert(
            en_schema(
                &sch,
                format!(
                    "{}/{sufijo}",
                    if copia.is_some() { "datasets" } else { "views" }
                ),
            ),
            con_schema(
                vista_yaml(&vista, paquete, &sch, &owner, t, objeto, copia.is_some()),
                &sch,
                paquete,
            ),
        );

        if t.filas == Some(0) && dec.de(&id(Clase::Filas, &t.nombre)).is_none() {
            pendientes.push(pendiente(
                Clase::Filas,
                &t.nombre,
                &t.nombre,
                "cero filas",
                "puede ser una tabla viva y vacía o un resto. El inductor no \
                 distingue una cosa de la otra, y borrarla sería decidirlo",
                vec!["mantener".into(), OMITIR.into()],
            ));
        }
    }

    ficheros.insert("package.yaml".into(), package_yaml);
    // Los schemas del origen que entran: cada uno, declarado (01 §2; una
    // carpeta no hace un schema).
    let schemas: BTreeSet<String> = ficheros
        .keys()
        .filter_map(|k| k.split_once('/'))
        .map(|(a, _)| a.to_string())
        .filter(|a| {
            ![
                "tables",
                "views",
                "datasets",
                "entities",
                "concepts",
                "functions",
                "actions",
                "models",
                "lattices",
                "rulesets",
                "interfaces",
                "resolutions",
                "policies",
            ]
            .contains(&a.as_str())
        })
        .collect();
    for sch in schemas {
        ficheros.insert(
            format!("{sch}/schema.yaml"),
            schema_yaml(&sch, paquete, &owner),
        );
    }
    pendientes.extend(pend_paquete);
    pendientes.extend(candidatas_a_concepto(&tablas, dec, voc));
    pendientes.extend(relaciones_no_declaradas(&tablas, &nombres, dec));

    // Lo que se contestó y no llegó a ninguna pregunta. Una errata en un
    // identificador no puede tener el mismo aspecto que una decisión tomada.
    let todas_antes: Vec<Tabla> = cat
        .tablas
        .iter()
        .cloned()
        .chain(sin_modelar.iter().cloned())
        .collect();
    let huerfanas = huerfanas(dec, &todas_antes, &tablas, &c.acunados);

    Induccion {
        ficheros,
        pendientes,
        huerfanas,
    }
}

/// Respuestas que no corresponden a ninguna pregunta de este catálogo.
///
/// No basta con mirar la cola: una decisión contestada **desaparece** de ella,
/// así que sin el conjunto de lo contestable toda respuesta correcta parecería
/// huérfana en la segunda pasada. Se calcula sobre las tablas de antes y de
/// después de unir familias, porque las dos son sujetos legítimos.
fn huerfanas(
    dec: &Decisiones,
    antes: &[Tabla],
    despues: &[Tabla],
    acunados: &[String],
) -> Vec<String> {
    let mut posibles: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // El sujeto de una clasificación es un concepto, y un concepto no existe
    // hasta que alguien lo acuña: se enumeran los acuñados, no las tablas.
    for c in acunados {
        posibles.insert(id(Clase::Clasificacion, c));
    }
    for tablas in [antes, despues] {
        for clave in claves_de_colision(tablas).into_values() {
            posibles.insert(id(Clase::Colision, &clave));
        }
    }
    for t in antes.iter().chain(despues) {
        for c in [Clase::Clave, Clase::Vista, Clase::Filas, Clase::Vacio] {
            posibles.insert(id(c, &t.nombre));
        }
        posibles.insert(id(Clase::Familia, &raiz(&t.nombre)));
        for col in &t.columnas {
            let s = format!("{}.{}", t.nombre, col.nombre);
            posibles.insert(id(Clase::Tipo, &s));
            posibles.insert(id(Clase::Relacion, &s));
            posibles.insert(id(
                Clase::Concepto,
                &concepto_id(&col.nombre, col.tipo.as_deref()),
            ));
        }
    }
    dec.ids()
        .filter(|i| !posibles.contains(i.as_str()))
        // El dueño es del paquete, no de una tabla: su sujeto es el nombre del
        // paquete, que aquí no se conoce.
        .filter(|i| !i.starts_with(Clase::Dueno.prefijo()))
        .cloned()
        .collect()
}

/// Por qué el lector no supo traducir un tipo, dicho **para ese tipo**.
///
/// El mensaje era el mismo para todos —«puede ser un objeto embebido o una
/// entidad aparte»— y a un `numeric` eso no le dice nada: su pregunta real es
/// cuántos decimales y en qué moneda, que es otra decisión y tiene su propia
/// sintaxis en OOS. Un motivo que no es el motivo cuesta la revisión entera.
fn no_se_traduce(origen: &str) -> &'static str {
    let o = origen.to_ascii_lowercase();
    if o.starts_with("numeric") || o.starts_with("decimal") || o.starts_with("money") {
        return "Un decimal sin precisión declarada no es un tipo cerrado, y la pregunta \
                es cuántos decimales y en qué moneda: `Money<EUR, 2>` lo dice y `Decimal` \
                lo calla. Las dos respuestas son correctas para columnas distintas";
    }
    "Puede ser un objeto embebido o una entidad aparte, y las dos lecturas son \
     modelos distintos: elegir una es modelar, no traducir"
}

// ── Las decisiones, aplicadas al catálogo ───────────────────────────────────

/// La clave primaria que va a llevar una tabla: la que declaró el origen, o la
/// que alguien contestó. Una columna contestada que no existe no se escribe —
/// escribirla produciría un `primaryKey` que apunta a nada.
fn clave_de(t: &Tabla, dec: &Decisiones) -> Vec<String> {
    if !t.clave.is_empty() {
        return t.clave.clone();
    }
    match dec.de(&id(Clase::Clave, &t.nombre)) {
        Some(Respuesta::Lista(cols)) => cols
            .iter()
            .filter(|c| t.columnas.iter().any(|x| x.nombre == **c))
            .cloned()
            .collect(),
        Some(Respuesta::Palabra(c)) if !c.eq_ignore_ascii_case(OMITIR) => t
            .columnas
            .iter()
            .filter(|x| x.nombre == *c)
            .map(|x| x.nombre.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// Los tipos que una respuesta cerró.
///
/// Se comprueban con el analizador de tipos de `ore-core` y no contra una lista
/// de aquí: `Money<EUR, 2>` es una respuesta legítima a un `numeric` y una lista
/// de escalares la habría rechazado.
fn con_tipos(t: &Tabla, dec: &Decisiones) -> Tabla {
    let mut t = t.clone();
    for c in t.columnas.iter_mut().filter(|c| c.tipo.is_none()) {
        let Some(r) = dec.de(&id(Clase::Tipo, &format!("{}.{}", t.nombre, c.nombre))) else {
            continue;
        };
        let Some(p) = r.palabra() else { continue };
        if p.eq_ignore_ascii_case(OMITIR) {
            continue;
        }
        if ore_core::types::parse_type(p).is_ok() {
            c.tipo = Some(p.to_string());
        }
    }
    t
}

/// La raíz de un nombre de tabla sin su sufijo de dígitos.
///
/// `ventas.evento_20190101` y `ventas.pedidos_2024` dan `ventas.evento` y
/// `ventas.pedidos`; `ventas.pedidos` se da a sí misma. Eso último es el arreglo:
/// la versión anterior exigía **dos** nombres con dígitos, y `pedidos` +
/// `pedidos_2024` —el caso más común de un almacén real— pasaba de largo porque
/// `pedidos` no lleva ninguno.
fn raiz(tabla: &str) -> String {
    let (prefijo, ultimo) = match tabla.rsplit_once('.') {
        Some((p, u)) => (format!("{p}."), u),
        None => (String::new(), tabla),
    };
    let sin = ultimo.trim_end_matches(|c: char| c.is_ascii_digit());
    let sin = sin.trim_end_matches('_');
    if sin.len() > 1 {
        format!("{prefijo}{sin}")
    } else {
        tabla.to_string()
    }
}

/// Las familias fragmentadas por fecha, y lo que se decidió sobre ellas.
///
/// Sin respuesta se reportan y no se tocan. Con `separadas`, cada hermana sigue
/// siendo su propia entidad y la pregunta se cierra. Con `omitir`, se van.
///
/// # Unir dejó de ser una respuesta posible, y no por comodidad
///
/// Hasta v1alpha8 unir una familia era **una entidad servida desde N tablas**, y
/// eso se escribía con N bindings. El binding se retiró, y lo que lo sustituye
/// —una vista sobre una tabla— sale de **un** sitio: `from` es exactamente una
/// de dos formas, y el vocabulario no tiene junta.
///
/// No es un hueco de implementación: `v1alpha8/00-scope` §6 deja unir fuera a
/// propósito, porque una junta trae dos raíces y el precio en la regla de flujo
/// se decide **antes** de admitir la operación. El IR del motor de vistas ya la
/// tiene con sus reglas; la gramática la admitirá cuando se decida su coste.
///
/// Así que la pregunta se hace igual —una familia fechada sigue siendo un hecho
/// que quien revisa tiene que ver— y las respuestas son dos. Ofrecer una columna
/// sería ofrecer algo que luego no se puede escribir, y una respuesta que no se
/// puede honrar es peor que una pregunta sin respuesta cómoda.
fn familias(tablas: &[Tabla], dec: &Decisiones) -> (Vec<Tabla>, Vec<Pendiente>) {
    let mut familia: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, t) in tablas.iter().enumerate() {
        familia.entry(raiz(&t.nombre)).or_default().push(i);
    }
    // Una familia exige al menos una hermana CON sufijo: dos tablas cuya raíz
    // coincide sin que ninguna esté numerada no son una familia, son dos tablas.
    familia.retain(|r, v| v.len() > 1 && v.iter().any(|i| tablas[*i].nombre != *r));

    let mut fuera: Vec<usize> = Vec::new();
    let mut pendientes = Vec::new();

    for (r, miembros) in &familia {
        let sujeto = miembros
            .iter()
            .map(|i| tablas[*i].nombre.clone())
            .collect::<Vec<_>>()
            .join(" · ");
        const OPCIONES: &str = "separadas";
        let opciones = || vec![OPCIONES.to_string(), OMITIR.to_string()];
        let porque = "el sufijo numérico es el patrón de una tabla fragmentada por fecha, y \
             verlo es lo que evita modelar la misma cosa N veces. UNIRLAS NO SE PUEDE \
             ESCRIBIR: una vista sale de un sitio y el vocabulario no tiene junta — \
             `v1alpha8/00-scope` §6 la deja fuera a propósito, porque una junta trae dos \
             raíces y su precio en la regla de flujo se decide antes de admitirla";
        match dec.de(&id(Clase::Familia, r)) {
            None => pendientes.push(pendiente(
                Clase::Familia,
                r,
                sujeto,
                format!("familia fechada de `{}`", entidad(r)),
                porque,
                opciones(),
            )),
            Some(x) if x.es("separadas") => {}
            Some(x) if x.omite() => fuera.extend(miembros.iter().copied()),
            // Una respuesta que nombra una columna es la de antes: alguien
            // pidió unir. No se ignora en silencio —eso dejaría a esa persona
            // creyendo que unió algo— y no se honra a medias: la pregunta sigue
            // abierta y dice qué cambió.
            Some(_) => pendientes.push(pendiente(
                Clase::Familia,
                r,
                sujeto,
                "unir una familia ya no se puede escribir",
                porque,
                opciones(),
            )),
        }
    }

    let out: Vec<Tabla> = tablas
        .iter()
        .enumerate()
        .filter(|(i, _)| !fuera.contains(i))
        .map(|(_, t)| t.clone())
        .collect();
    (out, pendientes)
}

/// El identificador de una candidata a concepto. Lleva el tipo dentro porque
/// `email: String` y `email: Integer` no son la misma pregunta.
fn concepto_id(columna: &str, tipo: Option<&str>) -> String {
    format!("{columna}.{}", identificador(tipo.unwrap_or("?")))
}

/// El nombre de entidad de cada tabla, con las colisiones resueltas donde lo
/// estén. Una tabla ausente del mapa es una que **no se emite**.
fn nombres(tablas: &[Tabla], dec: &Decisiones) -> (BTreeMap<String, String>, Vec<Pendiente>) {
    // Por schema (0038 P5): la entidad se llama igual en dos schemas sin
    // chocar. La pregunta, si la hay, se llama como siempre (las respuestas de
    // antes siguen valiendo); `<schema>.<Entidad>` sólo si sale en varios.
    let claves = claves_de_colision(tablas);
    let mut por_nombre: BTreeMap<String, Vec<&Tabla>> = BTreeMap::new();
    for t in tablas {
        por_nombre
            .entry(claves[&t.nombre].clone())
            .or_default()
            .push(t);
    }

    let mut out = BTreeMap::new();
    let mut pendientes = Vec::new();
    for (nombre, grupo) in &por_nombre {
        // El nombre de la entidad: sin el schema de la clave.
        let entidad_de = |t: &Tabla| entidad(&t.nombre);
        if grupo.len() == 1 {
            let t = grupo[0];
            // `omitir` a una vista o a una tabla vacía la saca del paquete: es
            // la única respuesta que decide que algo NO existe.
            // Las tres respuestas que deciden que algo NO existe. `omitir` a la
            // clave es la tercera y no es simetría: sin identidad no hay entidad,
            // y emitirla igualmente dejaría un OOS2010 que nadie puede cerrar.
            if [Clase::Vista, Clase::Filas, Clase::Clave]
                .iter()
                .any(|c| dec.omite(&id(*c, &t.nombre)))
            {
                continue;
            }
            out.insert(t.nombre.clone(), entidad_de(t));
            continue;
        }

        match dec.de(&id(Clase::Colision, nombre)) {
            // Un nombre por cada tabla: las dos existen y se llaman distinto.
            Some(Respuesta::Mapa(m)) => {
                let mut completo = true;
                for t in grupo {
                    match m.get(&t.nombre).map(|n| identificador(n)) {
                        Some(n) if !n.is_empty() => {
                            out.insert(t.nombre.clone(), capitalizar(&n));
                        }
                        _ => completo = false,
                    }
                }
                if !completo {
                    pendientes.push(colision(
                        nombre,
                        grupo,
                        Some(
                            "faltan tablas por nombrar. Emitir solo las nombradas decidiría \
                         que las demás no existen",
                        ),
                    ));
                }
            }
            // Una tabla: esa se queda con el nombre y las demás no se emiten.
            Some(Respuesta::Palabra(t)) if !t.eq_ignore_ascii_case(OMITIR) => {
                match grupo.iter().find(|x| x.nombre == *t) {
                    Some(elegida) => {
                        out.insert(elegida.nombre.clone(), entidad_de(elegida));
                    }
                    None => pendientes.push(colision(
                        nombre,
                        grupo,
                        Some("la tabla contestada no es ninguna de las que colisionan"),
                    )),
                }
            }
            Some(_) => {}
            None => pendientes.push(colision(nombre, grupo, None)),
        }
    }
    (out, pendientes)
}

fn colision(nombre: &str, grupo: &[&Tabla], nota: Option<&str>) -> Pendiente {
    let porque = "dos tablas dan el mismo identificador de OOS. Elegir una \
                  automáticamente decidiría cuál de las dos existe";
    pendiente(
        Clase::Colision,
        nombre,
        grupo
            .iter()
            .map(|t| t.nombre.clone())
            .collect::<Vec<_>>()
            .join(" · "),
        format!("colisionan en `{nombre}`"),
        match nota {
            Some(n) => format!("{porque}. {n}"),
            None => porque.to_string(),
        },
        grupo.iter().map(|t| t.nombre.clone()).collect(),
    )
}

/// Los conceptos que una respuesta eligió o acuñó, y dónde se hablan.
///
/// Dos caminos, y la diferencia importa. Si la respuesta nombra un concepto que
/// **ya existe** —el de un paquete de vocabulario importado—, aquí no se escribe
/// nada: se apunta. Si nombra uno nuevo, se **acuña**, porque `is` exige que el
/// concepto exista —`OOS2001`— y dejar la referencia colgando sería peor que no
/// preguntar.
///
/// Acuñar no contradice el aviso de `02-property` §6.2 —cuatro mil columnas no
/// pueden dar cuatro mil conceptos— porque no acuña el inductor: acuña quien
/// contesta, una vez, para todas las apariciones. Pero acuñar **abre otra
/// pregunta**, y esa es la que faltaba: un concepto sin clasificación no gobierna
/// nada, y la columna que lo habla sigue saliendo servida en la superficie
/// emitida como si nadie hubiera contestado.
/// Lo que sale de decidir los conceptos.
struct Conceptos {
    /// Los `concepts/*.yaml` que hubo que escribir.
    ficheros: BTreeMap<String, String>,
    /// `tabla.columna` → concepto cualificado. Es lo que acaba en un `is`.
    mapeo: BTreeMap<String, String>,
    pendientes: Vec<Pendiente>,
    /// Los que se acuñaron **aquí**. Los que ya existían no están: su
    /// clasificación la decidió quien publicó el vocabulario, y volver a
    /// preguntarla sería reabrir una decisión ajena.
    acunados: Vec<String>,
}

fn conceptos(tablas: &[Tabla], paquete: &str, dec: &Decisiones, voc: &Vocabulario) -> Conceptos {
    let mut ficheros = BTreeMap::new();
    let mut mapeo: BTreeMap<String, String> = BTreeMap::new();
    let mut pendientes = Vec::new();
    let mut acunados = Vec::new();

    for ((columna, tipo), donde) in repetidas(tablas) {
        let pregunta = id(Clase::Concepto, &concepto_id(&columna, Some(&tipo)));
        let Some(r) = dec.de(&pregunta) else { continue };
        let Some(p) = r.palabra().map(str::trim).filter(|p| !p.is_empty()) else {
            continue;
        };
        if p.eq_ignore_ascii_case("no") || p.eq_ignore_ascii_case(OMITIR) {
            continue;
        }

        // ── El que ya existe: se apunta y no se escribe ──────────────────────
        if let Some(c) = voc.de(p) {
            // El tipo lo pone el concepto. Apuntar a uno de otro tipo no es un
            // error de estilo: **retipa la columna en silencio**, y el esquema
            // prohíbe declarar los dos para que no haya a quién apelar.
            if c.tipo != tipo {
                pendientes.push(pendiente(
                    Clase::Concepto,
                    &concepto_id(&columna, Some(&tipo)),
                    format!("`{columna}: {tipo}` en {}", donde.join(", ")),
                    format!("`{p}` es un concepto de tipo `{}`", c.tipo),
                    "`is` no redeclara el tipo: lo toma del concepto. Apuntar a uno de \
                     otro tipo retiparía la columna sin decirlo, y el esquema prohíbe \
                     escribir los dos para que no haya a quién apelar si dejan de coincidir",
                    opciones_de_concepto(voc, &columna, &tipo),
                ));
                continue;
            }
            for t in &donde {
                mapeo.insert(format!("{t}.{columna}"), c.qname.clone());
            }
            continue;
        }

        // ── El nuevo: se acuña, y con él aparece su clasificación ───────────
        let nombre = identificador(p.rsplit('.').next().unwrap_or(p));
        if nombre.is_empty() {
            continue;
        }
        let qname = format!("{paquete}.{nombre}");
        let (etiquetas, pend) = clasificacion(&qname, dec, voc);
        pendientes.extend(pend);
        ficheros.insert(
            format!("concepts/{nombre}.yaml"),
            concepto_yaml(&nombre, paquete, &tipo, &columna, &donde, &etiquetas),
        );
        for t in &donde {
            mapeo.insert(format!("{t}.{columna}"), qname.clone());
        }
        acunados.push(qname);
    }
    Conceptos {
        ficheros,
        mapeo,
        pendientes,
        acunados,
    }
}

/// Cómo se clasifica un concepto acuñado: las etiquetas que lleva, y la pregunta
/// si nadie la contestó.
///
/// `sin_clasificar` es una respuesta legítima y **hay que darla**: hay conceptos
/// que no son sensibles —`legalName` no lo es— y el paquete de vocabulario de
/// referencia tiene uno así. Lo que no es legítimo es que lo decida el silencio.
fn clasificacion(
    qname: &str,
    dec: &Decisiones,
    voc: &Vocabulario,
) -> (Vec<(String, String)>, Vec<Pendiente>) {
    let ejes: Vec<&crate::vocabulario::Reticulo> = voc.ejes().collect();
    let abrir = |nota: Option<&str>| {
        let porque = "la etiqueta de un concepto es la tercera fuente de la clasificación \
                      efectiva, y la clasificación efectiva es lo que poda la superficie \
                      emitida. Sin ella este concepto no gobierna nada: la columna que lo \
                      habla sale servida igual que si nadie hubiera contestado";
        vec![pendiente(
            Clase::Clasificacion,
            qname,
            format!("el concepto `{qname}`"),
            "acuñado sin clasificar",
            match nota {
                Some(n) => format!("{porque}. {n}"),
                None => porque.to_string(),
            },
            niveles(&ejes),
        )]
    };

    // Sin un retículo en el repositorio no hay con qué clasificar, y preguntarlo
    // sería pedir que se elija de una lista vacía. Es la misma decisión que
    // `ore init` ya marca: sin escala no hay nada que gobernar.
    if ejes.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let Some(r) = dec.de(&id(Clase::Clasificacion, qname)) else {
        return (Vec::new(), abrir(None));
    };
    match r {
        x if x.es("sin_clasificar") => (Vec::new(), Vec::new()),
        Respuesta::Mapa(m) => {
            let puestas: Vec<(String, String)> = m
                .iter()
                .filter(|(eje, nivel)| {
                    ejes.iter()
                        .any(|r| r.qname == **eje && r.niveles.contains(nivel))
                })
                .map(|(e, n)| (e.clone(), n.clone()))
                .collect();
            if puestas.is_empty() {
                return (
                    Vec::new(),
                    abrir(Some("Ningún eje y nivel de la respuesta existe")),
                );
            }
            (puestas, Vec::new())
        }
        // `gdpr.sensitivity: high` en una palabra: es EXACTAMENTE lo que la cola
        // enseña en `options`, y una opción que se ofrece y no se acepta al
        // contestarla es peor que no ofrecerla.
        Respuesta::Palabra(p) if p.contains(':') => {
            let (eje, nivel) = p.split_once(':').expect("acaba de comprobarse");
            let (eje, nivel) = (eje.trim(), nivel.trim());
            if ejes
                .iter()
                .any(|r| r.qname == eje && r.niveles.iter().any(|n| n == nivel))
            {
                (vec![(eje.to_string(), nivel.to_string())], Vec::new())
            } else {
                (
                    Vec::new(),
                    abrir(Some("Ese eje y ese nivel no están en ningún retículo")),
                )
            }
        }
        // Un nivel a secas cuando hay UN solo retículo: no hay ambigüedad que
        // resolver, y escribir `gdpr.sensitivity: high` entero cada vez es donde
        // salen las erratas.
        Respuesta::Palabra(nivel) => match ejes.as_slice() {
            [unico] if unico.niveles.contains(nivel) => {
                (vec![(unico.qname.clone(), nivel.clone())], Vec::new())
            }
            [_] => (Vec::new(), abrir(Some("Ese nivel no está en el retículo"))),
            _ => (
                Vec::new(),
                abrir(Some(
                    "Hay más de un retículo, así que un nivel a secas no dice de cuál es",
                )),
            ),
        },
        _ => (Vec::new(), abrir(Some("La respuesta no nombra un nivel"))),
    }
}

/// `eje: nivel` para cada nivel de cada retículo, y la salida honesta.
fn niveles(ejes: &[&crate::vocabulario::Reticulo]) -> Vec<String> {
    let mut out: Vec<String> = ejes
        .iter()
        .flat_map(|r| r.niveles.iter().map(|n| format!("{}: {n}", r.qname)))
        .collect();
    out.push("sin_clasificar".into());
    out
}

/// Lo que se puede contestar a la séptima pregunta: los conceptos que ya
/// existen y sirven, y `no`.
///
/// Un nombre nuevo también vale y por eso las opciones no son cerradas — pero
/// **ofrecer lo publicado va primero**, porque acuñar cuando ya existe el
/// concepto es la inflación por otra puerta.
fn opciones_de_concepto(voc: &Vocabulario, columna: &str, tipo: &str) -> Vec<String> {
    let mut out: Vec<String> = voc
        .candidatos(columna, tipo)
        .into_iter()
        .map(|c| {
            if c.etiquetas.is_empty() {
                c.qname.clone()
            } else {
                format!("{}  ({})", c.qname, c.etiquetas.join(" · "))
            }
        })
        .collect();
    out.push("no".into());
    out
}

// ── Las conjeturas, que se reportan y no se escriben ────────────────────────

/// Los pares `(columna, tipo)` que aparecen en más de una tabla, con dónde.
fn repetidas(tablas: &[Tabla]) -> BTreeMap<(String, String), Vec<String>> {
    let mut donde: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for t in tablas {
        for c in &t.columnas {
            // Sin tipo no hay «mismo tipo» que comprobar, y agrupar por nombre a
            // secas es justo el parecido que este modulo no toma por identidad.
            let Some(tipo) = &c.tipo else { continue };
            donde
                .entry((c.nombre.clone(), tipo.clone()))
                .or_default()
                .push(t.nombre.clone());
        }
    }
    // El filtro estructural se queda: `id`, `*_id` y `*_at` son andamiaje de la
    // tabla, no conceptos de nadie. El de «aparece más de una vez» se movió a
    // quien pregunta, porque ya no es el único motivo para preguntar.
    donde.retain(|(n, _), _| !estructural(n));
    donde
}

/// Un nombre de columna que se repite en varias tablas **con el mismo tipo** es
/// una candidata a concepto. No se acuña: se muestran juntas para que la
/// unificación se decida una vez.
fn candidatas_a_concepto(tablas: &[Tabla], dec: &Decisiones, voc: &Vocabulario) -> Vec<Pendiente> {
    repetidas(tablas)
        .into_iter()
        // **Dos motivos para preguntar, y solo uno estaba.**
        //
        // El original era la repetición: una columna en varias tablas es una
        // candidata a concepto, y decidirlo una vez vale por todas. Se midió
        // contra un dataset de verdad y faltaba el otro: `nif` sale en UNA
        // tabla y `gdpr.nationalId` la lleva de sinónimo; `nom` sale en una y
        // `gdpr.fullName` también. Eran las dos mejores acuñaciones
        // disponibles y **la cola no las mencionaba**.
        //
        // Ampliar no reabre la inflación que el filtro evitaba, porque los dos
        // motivos piden cosas distintas: repetir invita a ACUÑAR —caro, hay
        // que clasificar— y parecerse invita a APUNTAR, que no crea nada.
        .filter(|((n, tipo), v)| v.len() > 1 || !voc.candidatos(n, tipo).is_empty())
        .filter(|((n, tipo), _)| {
            dec.de(&id(Clase::Concepto, &concepto_id(n, Some(tipo))))
                .is_none()
        })
        .map(|((n, tipo), v)| {
            // Con vocabulario publicado la pregunta cambia de forma: deja de ser
            // «invéntale un nombre» y pasa a ser «¿es alguno de estos?». Sin él
            // solo queda acuñar, que es la respuesta cara.
            let porque = if voc.candidatos(&n, &tipo).is_empty() {
                "acuñar uno por columna repetida es la inflación que produce cuatro mil \
                 conceptos. Decidirlo una vez vale por todas las apariciones — y no hay \
                 ningún concepto publicado de este tipo al que apuntar, así que contestar \
                 con un nombre lo ACUÑA, y entonces hay que clasificarlo"
            } else {
                "hay un concepto publicado que dice ser esto, y apuntarlo hereda su \
                 clasificación sin escribirla. Acuñar uno nuevo cuando ya existe es la \
                 inflación por la otra puerta"
            };
            pendiente(
                Clase::Concepto,
                &concepto_id(&n, Some(&tipo)),
                format!("`{n}: {tipo}` en {}", v.join(", ")),
                "¿el mismo concepto?",
                porque,
                opciones_de_concepto(voc, &n, &tipo),
            )
        })
        .collect()
}

/// Una columna `X_id` cuando existe una entidad que casa con `X`. En un origen
/// sin claves foráneas —BigQuery no las tiene— es la única pista que hay, y es
/// una pista, no un hecho.
fn relaciones_no_declaradas(
    tablas: &[Tabla],
    nombres: &BTreeMap<String, String>,
    dec: &Decisiones,
) -> Vec<Pendiente> {
    let mut out = Vec::new();
    for t in tablas.iter().filter(|t| nombres.contains_key(&t.nombre)) {
        for (c, destino) in parecidos(t, nombres) {
            let sujeto = format!("{}.{}", t.nombre, c);
            if dec.de(&id(Clase::Relacion, &sujeto)).is_some() {
                continue;
            }
            out.push(pendiente(
                Clase::Relacion,
                &sujeto,
                &sujeto,
                format!("¿una relación hacia `{destino}`?"),
                "el origen no declara la clave foránea, así que esto es un \
                 parecido de nombres. Emitir la arista convertiría una \
                 coincidencia en una afirmación sobre el grafo",
                vec!["si".into(), "no".into()],
            ));
        }
    }
    out
}

/// Los parecidos de nombres de una tabla: `(columna, entidad destino)`.
fn parecidos(t: &Tabla, nombres: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let declaradas: Vec<&String> = t.foraneas.iter().flat_map(|f| &f.columnas).collect();
    let propia = entidad(&t.nombre);
    let mut out = Vec::new();
    for c in &t.columnas {
        // Ya declarada como foránea, o parte de la clave: `pedidos.id_pedido`
        // es la identidad de `pedidos`, no una arista de la tabla a sí misma.
        // El sufijo `_id` dice «esto identifica algo», y la mitad de las veces
        // ese algo es la propia fila.
        if declaradas.contains(&&c.nombre) || t.clave.contains(&c.nombre) {
            continue;
        }
        let Some(r) = c
            .nombre
            .strip_suffix("_id")
            .or_else(|| c.nombre.strip_prefix("id_"))
        else {
            continue;
        };
        if r.is_empty() {
            continue;
        }
        let destino = nombres
            .values()
            .find(|n| n.to_ascii_lowercase().starts_with(&r.to_ascii_lowercase()));
        if let Some(d) = destino.filter(|d| **d != propia) {
            out.push((c.nombre.clone(), d.clone()));
        }
    }
    out
}

/// Las relaciones que alguien confirmó, listas para escribirse.
///
/// Se niega a emitir la arista cuando el destino no tiene UNA clave de una sola
/// columna: `via` se empareja posición a posición con la clave del destino, y
/// una arista de aridad distinta pasa por escrita y une por pares que no son.
fn relaciones_decididas(
    t: &Tabla,
    nombres: &BTreeMap<String, String>,
    claves: &BTreeMap<String, Vec<String>>,
    dec: &Decisiones,
) -> (Vec<(String, String)>, Vec<Pendiente>) {
    let mut out = Vec::new();
    let mut pendientes = Vec::new();
    for (c, destino) in parecidos(t, nombres) {
        let sujeto = format!("{}.{}", t.nombre, c);
        let Some(r) = dec.de(&id(Clase::Relacion, &sujeto)) else {
            continue;
        };
        if !r.es("si") {
            continue;
        }
        let tabla_destino = nombres.iter().find(|(_, n)| **n == destino).map(|(t, _)| t);
        let aridad = tabla_destino
            .and_then(|td| claves.get(td))
            .map_or(0, Vec::len);
        if aridad == 1 {
            out.push((c, destino));
            continue;
        }
        pendientes.push(pendiente(
            Clase::Relacion,
            &sujeto,
            &sujeto,
            format!("no se pudo emitir la relación hacia `{destino}`"),
            format!(
                "el destino no tiene una clave primaria de UNA columna —tiene {aridad}—, \
                 y `via` se empareja posición a posición con ella. Una arista de aridad \
                 distinta tiene el mismo aspecto que una correcta y une por pares que no son. \
                 Se cierra dándole clave al destino"
            ),
            vec!["si".into(), "no".into()],
        ));
    }
    (out, pendientes)
}

/// Fontanería: no necesita concepto y llenaría el informe de ruido. Se midió
/// sobre un esquema real —48% de las columnas— antes de escribir esta lista.
fn estructural(nombre: &str) -> bool {
    let n = nombre.to_ascii_lowercase();
    n == "id"
        || n == "uuid"
        || n == "slug"
        || n.ends_with("_id")
        || n.ends_with("_at")
        || n.ends_with("_by")
        || n.starts_with("id_")
}

// ── Los documentos ──────────────────────────────────────────────────────────

/// El paquete, y su dueño si alguien lo dijo.
///
/// `cambiame` es un marcador y **falla al validar** con `OOS2009`, que está
/// bien: un dueño inventado sería lo único peor que ninguno, porque `team:datos`
/// se resuelve contra CODEOWNERS y un handle que no existe deja el paquete sin
/// nadie que responda mientras aparenta lo contrario.
/// **El manifiesto de un paquete.** El único emisor, y lo usan los dos que
/// escriben uno: la inducción y `ore package new`.
///
/// Los cuatro campos de `metadata` son OBLIGATORIOS en el esquema publicado
/// —`name`, `version`, `status`, `domain`— y `spec.owner` también, así que no
/// hay nada opcional que omitir: lo que se decide es **qué valor**, y esa
/// decisión es de quien llama.
///
/// # `status`, que es la única donde los dos no coinciden
///
/// La inducción escribe `active` y `ore package new` escribe `draft`, y el
/// segundo es el que se puede defender: `01-package` §2.3 deriva de `status` la
/// `oos.maturity` **por defecto** de lo que el paquete contenga, y un paquete
/// recién creado no contiene nada — llamarlo `active` sería afirmar `STABLE`
/// sobre lo que no existe. Que la inducción escriba `active` viene de antes y se
/// deja como está: cambiarlo mueve la madurez efectiva de todo lo inducido, que
/// es otra medida.
pub fn documento_paquete(nombre: &str, owner: &str, estado: &str, dominio: &str) -> String {
    // El texto vive en el núcleo (`ore_core::paquetes`): desde 0035 ⑦.1 lo
    // escribe también `ore-serve`, al darle sitio a un proyecto, y dos copias
    // de la misma forma divergen en el caso que ninguna prueba ejerce.
    ore_core::paquetes::documento(nombre, owner, estado, dominio)
}

fn paquete_yaml(paquete: &str, dec: &Decisiones) -> (String, Vec<Pendiente>) {
    let respuesta = dec
        .de(&id(Clase::Dueno, paquete))
        .and_then(Respuesta::palabra)
        .filter(|h| handle(h));
    let owner = respuesta.unwrap_or("cambiame");
    // El emisor es UNO, y lo comparte con `ore package new`: un manifiesto
    // escrito por la inducción y uno escrito a mano tienen que ser el mismo
    // texto, o hay dos emisores y divergen en el caso que ninguna prueba ejerce.
    let doc = documento_paquete(paquete, owner, "active", paquete);
    if respuesta.is_some() {
        return (doc, Vec::new());
    }
    (
        doc,
        vec![pendiente(
            Clase::Dueno,
            paquete,
            format!("el paquete `{paquete}`"),
            "sin dueño",
            "`spec.owner` es obligatorio y de él heredan las políticas de Cedar. \
             El inductor escribe `cambiame`, que NO valida: un handle inventado \
             dejaría el paquete sin nadie que responda aparentando lo contrario",
            vec!["team:<handle>".into(), "user:<handle>".into()],
        )],
    )
}

/// `team:<handle>` o `user:<handle>`. La forma exacta la comprueba `OOS2009`;
/// aquí solo se rechaza lo que seguro no lo es, para no escribir en el documento
/// una respuesta que lo va a romper.
fn handle(h: &str) -> bool {
    ["team:", "user:"]
        .iter()
        .any(|p| h.strip_prefix(p).is_some_and(|r| !r.trim().is_empty()))
}

/// Un tipo, escrito donde una coma separa.
///
/// `{ type: Money<EUR, 2> }` no es lo que parece: en un mapa de flujo la coma es
/// el separador, así que eso declara una propiedad `2>`. Lo dijo `ore validate`
/// —`OOS1005`, clave desconocida— sobre un documento que nadie escribió a mano,
/// que es justo la clase de error que un emisor tiene que hacer imposible.
fn en_flujo(tipo: &str) -> String {
    if tipo.contains([',', '{', '}', '[', ']', ':']) {
        entrecomillar(tipo)
    } else {
        tipo.to_string()
    }
}

/// Una cadena de YAML entre comillas dobles. La descripcion viene del origen y
/// puede traer cualquier cosa dentro; escaparla mal romperia el documento.
fn entrecomillar(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Lo que la inducción resuelve ANTES de emitir, y que cada emisor consulta.
///
/// Van juntos porque se calculan juntos y no tiene sentido tener uno sin los
/// otros: la clave de una tabla, cómo se llama su entidad, y qué concepto habla
/// cada columna. Un emisor que recibiera dos de los tres estaría emitiendo con
/// media inducción.
struct Resuelto<'a> {
    /// Tabla → su clave primaria: la declarada, o la decidida.
    claves: &'a BTreeMap<String, Vec<String>>,
    /// Tabla → nombre de entidad, **solo de las que se emiten**. Es la única
    /// fuente del nombre de un destino: derivarlo de la tabla con `entidad()`
    /// da un nombre que puede no existir, y da el EQUIVOCADO cuando una
    /// colisión se resolvió con otro.
    nombres: &'a BTreeMap<String, String>,
    /// `tabla.columna` → el concepto que habla, si alguien lo eligió.
    mapeo: &'a BTreeMap<String, String>,
    /// Los schemas renombrados de la regla (0038 P6): a dónde apunta una
    /// relación hacia una tabla de otro schema.
    schemas: &'a BTreeMap<String, String>,
}

fn entidad_yaml(
    nombre: &str,
    paquete: &str,
    vista: &str,
    t: &Tabla,
    r: &Resuelto<'_>,
    extra: &[(String, String)],
) -> String {
    let Resuelto {
        claves,
        nombres,
        mapeo,
        schemas,
    } = *r;
    // El schema de una tabla en el paquete: el del origen, o su nombre nuevo.
    let schema_de = |tabla: &str| {
        let s = schema_de(tabla);
        schemas.get(&s).cloned().unwrap_or(s)
    };
    let clave = claves.get(&t.nombre).cloned().unwrap_or_default();
    let mut s = String::new();
    let _ = write!(
        s,
        "apiVersion: oos.dev/v1alpha8\n\
         kind: Entity\n\
         metadata:\n  \
           name: {nombre}\n  \
           namespace: {paquete}\n  \
           labels: {{ oos.maturity: DRAFT }}\n\
         spec:\n  \
           nature: entity\n  \
           backedBy: {vista}\n"
    );
    if clave.is_empty() {
        s.push_str(
            "  # Sin clave primaria: el origen no la declara y NO DEBE inferirse.\n\
             \x20 # `ore validate` lo dirá con OOS2010, que es esta decisión escrita\n\
             \x20 # en la voz del compilador.\n",
        );
    } else {
        // Por `identificador`, igual que las propiedades: si no, una columna que
        // empiece por digito produce una clave que nombra algo inexistente.
        let _ = writeln!(
            s,
            "  primaryKey: [{}]",
            clave
                .iter()
                .map(|c| identificador(c))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !t.unicas.is_empty() {
        s.push_str("  uniqueKeys:\n");
        for k in &t.unicas {
            let _ = writeln!(
                s,
                "    - [{}]",
                k.iter()
                    .map(|c| identificador(c))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    s.push_str("  properties:\n");
    for c in &t.columnas {
        let Some(tipo) = &c.tipo else {
            // No se omite en silencio: una columna que desaparece sin decirlo es
            // peor que una que falta y lo dice.
            let _ = writeln!(
                s,
                "    # {}: el origen dice `{}`, que no es un tipo de OOS.",
                c.nombre,
                c.origen.as_deref().unwrap_or("?")
            );
            continue;
        };
        let obligatoria = if c.obligatoria {
            "  # NOT NULL en el origen"
        } else {
            ""
        };
        // `is` y `type` se excluyen por construcción —el esquema lo dice con un
        // `oneOf`—: el tipo lo pone el concepto, y si la copia deja de coincidir
        // no hay nada a lo que apelar para decidir quién gana.
        let declara = match mapeo.get(&format!("{}.{}", t.nombre, c.nombre)) {
            Some(concepto) => format!("is: {concepto}"),
            None => format!("type: {}", en_flujo(tipo)),
        };
        match &c.descripcion {
            None => {
                let _ = writeln!(
                    s,
                    "    {}: {{ {declara} }}{obligatoria}",
                    identificador(&c.nombre)
                );
            }
            Some(d) => {
                let _ = writeln!(
                    s,
                    "    {}:{obligatoria}\n      {declara}\n      description: {}",
                    identificador(&c.nombre),
                    entrecomillar(d)
                );
            }
        }
    }
    // `via` es una secuencia desde que se cerro la decision del enlace compuesto,
    // asi que una foranea de varias columnas se dice ENTERA. Antes habia que
    // reportarla: recortarla a su primera columna produce un join que une de
    // menos y tiene exactamente el mismo aspecto que uno correcto.
    //
    // El ORDEN es el que declaro el origen y no se toca: `via` se empareja
    // posicion a posicion con la clave del destino, y reordenarlo por estetica
    // enlazaria por pares distintos.
    // El cuerpo se construye ANTES de escribir `relations:`. Una foranea cuyo
    // destino no se emite no produce arista, asi que las que quedan pueden ser
    // CERO — y `relations:` sin nada debajo es `null`, que no valida. Titular
    // despues de contar es lo unico que lo cierra.
    let mut cuerpo = String::new();
    let mut aristas = 0usize;
    // Y lo que se cayo se DICE. Misma regla que la columna sin tipo de aqui
    // arriba: una relacion que desaparece sin decirlo es peor que una que falta
    // y lo dice.
    let mut caidas = String::new();
    for f in &t.foraneas {
        // `required` es un hecho del origen, no un valor por defecto. Y lo es
        // solo si TODAS las columnas del enlace son NOT NULL: con una que
        // admita nulos, la fila puede no enlazar.
        let obligatoria = f
            .columnas
            .iter()
            .all(|col| t.columnas.iter().any(|c| c.nombre == *col && c.obligatoria));
        let ident = |cs: &[String]| {
            cs.iter()
                .map(|c| identificador(c))
                .collect::<Vec<_>>()
                .join(", ")
        };
        // `toKey` solo cuando el origen NO apunta a la clave primaria del
        // destino. SQL permite referenciar cualquier UNIQUE, y callarlo
        // emitiria un enlace contra la identidad equivocada que pasa la
        // comprobacion de aridad y tipos por casualidad.
        let primaria = claves.get(&f.destino);
        let a_otra_clave = !f.destino_columnas.is_empty()
            && primaria.is_none_or(|p| {
                let (mut x, mut y) = (f.destino_columnas.clone(), p.clone());
                x.sort();
                y.sort();
                x != y
            });
        let to_key = if a_otra_clave {
            format!("      toKey: [{}]\n", ident(&f.destino_columnas))
        } else {
            String::new()
        };
        // El nombre del destino sale de `nombres` y de ningun otro sitio.
        let Some(destino) = nombres.get(&f.destino) else {
            let _ = writeln!(
                caidas,
                "  # Sin relacion hacia `{}`: esa tabla no entra en este paquete.\n  \
                 # La declara el origen como foranea desde [{}].",
                f.destino,
                ident(&f.columnas)
            );
            continue;
        };
        aristas += 1;
        // En su schema (0038 P5): `p.X` si es de `default`, `p.s.X` si no.
        let objetivo = ore_core::normalize::corto(paquete, &schema_de(&f.destino), destino);
        let _ = write!(
            cuerpo,
            "    {}:\n      target: {objetivo}\n      cardinality: many_to_one\n      via: [{}]\n{to_key}      required: {obligatoria}\n",
            identificador(destino).to_lowercase(),
            ident(&f.columnas)
        );
    }
    // Las que el origen NO declara y alguien confirmó al revisar. Van
    // marcadas: quien lea esto dentro de un año tiene que poder distinguir
    // un hecho del catálogo de una decisión de una persona.
    for (columna, destino) in extra {
        let obligatoria = t
            .columnas
            .iter()
            .any(|c| c.nombre == *columna && c.obligatoria);
        aristas += 1;
        // En el schema de la tabla de esa entidad (0038 P5).
        let sch = nombres
            .iter()
            .find(|(_, e)| *e == destino)
            .map(|(tabla, _)| schema_de(tabla))
            .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
        let objetivo = ore_core::normalize::corto(paquete, &sch, destino);
        let _ = write!(
            cuerpo,
            "    # No la declara el origen: la confirmó una persona al revisar.\n    \
             {}:\n      target: {objetivo}\n      \
             cardinality: many_to_one\n      via: [{}]\n      \
             required: {obligatoria}\n",
            destino.to_lowercase(),
            identificador(columna)
        );
    }
    // `aristas` y no `cuerpo.is_empty()`: son lo mismo hoy y dejarian de serlo
    // en cuanto algo escriba en el cuerpo sin ser una arista.
    if aristas > 0 {
        s.push_str("  relations:\n");
        s.push_str(&cuerpo);
    }
    s.push_str(&caidas);
    s
}

/// Un concepto acuñado al revisar.
///
/// `aiContext.synonyms` lleva el nombre físico de la columna y no un invento: es
/// lo único que se sabe de cómo se dice esto ahí fuera, y es exactamente para lo
/// que sirve el campo.
fn concepto_yaml(
    nombre: &str,
    paquete: &str,
    tipo: &str,
    columna: &str,
    donde: &[String],
    etiquetas: &[(String, String)],
) -> String {
    let labels = if etiquetas.is_empty() {
        // Sin etiquetas se dice **en el propio documento**. Un concepto que calla
        // su clasificación y otro que no la necesita tienen el mismo aspecto.
        "  # Sin clasificar: este concepto no eleva la etiqueta de nada.\n".to_string()
    } else {
        format!(
            "  labels: {{ {} }}\n",
            etiquetas
                .iter()
                .map(|(e, n)| format!("{e}: {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "apiVersion: oos.dev/v1alpha4\n\
         kind: Concept\n\
         metadata: {{ name: {nombre}, namespace: {paquete} }}\n\
         spec:\n  \
           type: {tipo}\n\
         {labels}  \
           description: {}\n  \
           aiContext: {{ synonyms: [{columna}] }}\n",
        entrecomillar(&format!(
            "Acuñado al revisar el descubrimiento: `{columna}` aparece en {}.",
            donde.join(", ")
        ))
    )
}

/// Un escalar de YAML: se entrecomilla lo que no es un identificador simple.
///
/// Los nombres físicos son **opacos** —pueden llevar puntos, espacios o empezar
/// por dígito— y un `Worker_Reference.ID` sin comillas sigue analizando pero
/// deja de ser lo que era el día que alguien meta un `:`.
fn escalar_yaml(s: &str) -> String {
    let simple = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !s.starts_with(|c: char| c.is_ascii_digit());
    if simple {
        s.to_string()
    } else {
        entrecomillar(s)
    }
}

/// Transcribe un nodo del catálogo a YAML. **No lo interpreta.**
///
/// El vocabulario de `reads` y de `changes` lo fija el esquema de OOS, y el
/// inductor no es el sitio donde se decide qué es legal: si lo supiera habría
/// dos sitios diciéndolo, y el día que discrepen ninguno diría cuál manda. Lo
/// que llega del driver se copia; lo que no encaje lo dirá `ore validate`,
/// que es quien tiene el esquema.
/// Una de las dos caras, a YAML.
///
/// Viaja **opaca** —el catálogo la guarda como `Json` y esta pieza no la
/// interpreta— y vuelve a nodo por el camino por el que llegó: **JSON es un
/// subconjunto de YAML** (ADR 0002), así que releerla es total y no hace falta
/// una segunda gramática para la misma forma.
fn cara_yaml(j: &Json, sangria: usize) -> String {
    parse::parse(&j.jcs())
        .map(|n| transcribir(&n, sangria))
        .unwrap_or_default()
}

fn transcribir(n: &Node, sangria: usize) -> String {
    let ind = " ".repeat(sangria);
    match n {
        Node::Mapping { entries, .. } => {
            let mut s = String::new();
            for (k, v) in entries {
                let Some(clave) = k.as_str() else { continue };
                match v {
                    Node::Mapping { .. } => {
                        let _ = writeln!(s, "{ind}{clave}:");
                        s.push_str(&transcribir(v, sangria + 2));
                    }
                    _ => {
                        let _ = writeln!(s, "{ind}{clave}: {}", transcribir(v, 0).trim_end());
                    }
                }
            }
            s
        }
        // En línea: una lista de escalares es un valor, no una sección.
        Node::Sequence { items, .. } => format!(
            "[{}]",
            items
                .iter()
                .filter_map(|i| i.as_str())
                .map(escalar_yaml)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Node::Scalar { .. } => escalar_yaml(n.as_str().unwrap_or("")),
    }
}

/// `kind: Table` — **el puntero al objeto físico, registrado una vez**.
///
/// Es la mitad del descubrimiento que **no es un borrador**: que el objeto
/// existe, qué columnas tiene, qué se le puede pedir y qué cambios emite son
/// cuatro hechos del origen, no cuatro conjeturas. Por eso esto se emite sin
/// revisión, y por eso `discover` deja de inferir aquí y pasa a espejar — lo
/// que un catálogo foráneo hace al crearse.
///
/// Lo que **no** lleva, y no por falta de sitio: `materialized` y `freshness`.
/// Son decisiones de operación con coste, y proponerlas sería exactamente
/// inventar. Van en la vista, vacías, y `OOS2020` dice dónde no pueden quedar
/// vacías.
fn tabla_yaml(
    paquete: &str,
    fuente: &str,
    t: &Tabla,
    objeto: &Objeto,
    clave_de_la_copia: Option<&[String]>,
) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "apiVersion: oos.dev/v1alpha8\n\
         kind: Table\n\
         metadata: {{ name: {}, namespace: {paquete} }}\n\
         spec:\n  \
           datasource: {fuente}\n  \
           object: {}\n  \
           columns:\n",
        nombre_de_tabla(objeto),
        entrecomillar(&objeto.nombre)
    );
    for c in t
        .columnas
        .iter()
        .filter(|c| objeto.columnas.contains(&c.nombre))
    {
        // El nombre físico ENTERO, sin pasar por `identificador`: la tabla es
        // el objeto tal cual está. Renombrar es de la vista, y ese es
        // exactamente el reparto que hace que dos vistas puedan compartir un
        // objeto sin repetir su contrato.
        let _ = write!(s, "    {}:", escalar_yaml(&c.nombre));
        // Las dos cosas que el conector sabe, y las dos se escriben (0032 §3;
        // `01-table.md` §5.0): `type`, el escalar de OOS que tradujo —antes se
        // tiraba aquí «porque el tipo es de la entidad», y la copia de una
        // tabla sin entidad salía entera como texto—, y `physicalType`, el
        // tipo del origen citado, que lleva la precisión y la escala. Una
        // columna que el conector no supo traducir lleva solo la cita.
        let mut partes: Vec<String> = Vec::new();
        if let Some(t) = &c.tipo {
            partes.push(format!("type: {}", escalar_yaml(t)));
        }
        if let Some(o) = &c.origen {
            partes.push(format!("physicalType: {}", escalar_yaml(o)));
        }
        if partes.is_empty() {
            s.push_str(" {}\n");
        } else {
            let _ = writeln!(s, " {{ {} }}", partes.join(", "));
        }
    }
    match &t.lee {
        Some(n) => {
            let _ = writeln!(s, "  reads:");
            s.push_str(&cara_yaml(n, 4));
        }
        // Un catálogo que no declara la cara `I` no dice que no se pueda leer:
        // dice que su driver no declaró nada. `none` afirmaría lo primero, y
        // arrastraría un `OOS2020` sobre una vista que nadie ha podido revisar.
        None => s.push_str(
            "  # El driver no declaró qué se puede empujar a este origen.\n  reads: {}\n",
        ),
    }
    match (&t.cambia, clave_de_la_copia) {
        // La base es estándar y la tabla tiene clave: la copia se funde por
        // ella. El testigo sigue siendo el que el driver sondeó — eso no lo
        // cambia una clave—; lo que cambia es que ahora hay con qué retirar
        // una fila, que es lo que `upsert` afirma y `append` no podía.
        (cambia, Some(clave)) => {
            let testigo = cambia
                .as_ref()
                .and_then(|n| match n {
                    Json::Obj(m) => m.get("witness").cloned(),
                    _ => None,
                })
                .unwrap_or(Json::s("none"));
            let _ = writeln!(s, "  changes:");
            let _ = writeln!(s, "    mode: upsert");
            let _ = writeln!(
                s,
                "    key: [{}]",
                clave
                    .iter()
                    .map(|c| escalar_yaml(c))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            s.push_str(&cara_yaml(&Json::obj([("witness", testigo)]), 4));
        }
        (Some(n), None) => {
            let _ = writeln!(s, "  changes:");
            s.push_str(&cara_yaml(n, 4));
        }
        (None, None) => s.push_str(
            "  # El driver no sondeó los cambios. No se sabe, y no se inventa.\n  \
             changes: { mode: none, witness: none }\n",
        ),
    }
    s
}

/// La vista trivial: **el objeto expuesto tal cual, con nombres de
/// identificador**.
///
/// Existe porque `backedBy` nombra una vista y **nunca una tabla**. Si nombrara
/// la tabla, las propiedades de la entidad tendrían que llamarse como las
/// columnas físicas y lo semántico volvería a saber de lo físico. Cuesta tres
/// líneas, y esas tres líneas son las que dicen *«esto se expone»*.
///
/// Expone **todas** las columnas del objeto, también las que la entidad no
/// modela por no saber traducir su tipo: su contrato las dice `String`, que es
/// lo único que se afirma de ellas, y tiparlas más tarde es cambiar el contrato
/// (añadir no rompe; cambiar un tipo sí, v1alpha14 §9).
///
/// **Y sale en `DRAFT`, como la entidad.** Esas tres líneas dicen *«esto se
/// expone»*, y eso es una decisión que nadie ha tomado todavía: la propuso esta
/// máquina mirando un catálogo. Hasta que la vista admitió `oos.maturity` no
/// había forma de decirlo, y una vista adivinada era indistinguible de una
/// acordada — con la ayuda del comando afirmando que las proponía en `DRAFT`.
fn vista_yaml(
    vista: &str,
    paquete: &str,
    schema: &str,
    owner: &str,
    t: &Tabla,
    objeto: &Objeto,
    copia: bool,
) -> String {
    let de_la_tabla: Vec<_> = t
        .columnas
        .iter()
        .filter(|c| objeto.columnas.contains(&c.nombre))
        .collect();
    let campos: Vec<(String, String)> = de_la_tabla
        .iter()
        .map(|c| (identificador(&c.nombre), c.nombre.clone()))
        .collect();
    let tabla = nombre_de_tabla(objeto);
    if copia {
        return documento_dataset(vista, paquete, owner, &tabla, &campos, &[]);
    }
    let tipos: BTreeMap<String, String> = de_la_tabla
        .iter()
        .filter_map(|c| Some((c.nombre.clone(), c.tipo.clone()?)))
        .collect();
    documento_vista(vista, paquete, schema, owner, &tabla, &campos, &tipos)
}

/// **Cómo se llama la `Table` de un objeto: `<objeto>_t`.** En v1alpha14 un
/// nombre es una sola cosa en su schema (`OOS2035`), y la vista que expone el
/// objeto —o el dataset que lo copia— se llama como él. La tabla es el hecho
/// del origen y sólo la nombran quienes la leen; la pregunta es lo que el
/// resto del árbol nombra. Su `object` es el del origen, sin sufijo. El mismo
/// que `ore migrate v1alpha14` da a una tabla que se llamaba como su vista.
fn nombre_de_tabla(objeto: &Objeto) -> String {
    format!("{}_t", identificador(sin_schema(&objeto.nombre)))
}

/// **El emisor de la `View` que el inductor propone** (v1alpha14, ADR 0040
/// paso 6): el objeto expuesto con nombres de identificador, escrito como la
/// consulta que es. La consulta la escribe **la traducción de la forma**
/// (`linaje::como_sql`, la de §7 que usan la migración y el núcleo al
/// servir): se escribe la forma de siempre y se traduce, así que una vista
/// inducida y una migrada son el mismo texto. Y legible, una columna por línea
/// cuando no cabe (`migrar_v14::legible`).
///
/// El contrato (`columns`) lleva el tipo que el conector tradujo de cada
/// columna, y `String` donde no supo: es lo único que se afirma de ella, y lo
/// que el núcleo le daba al tiparla.
///
/// `campos` va **en orden**, el del origen: reordenar aquí sería decidir.
///
/// Una vista es sólo la pregunta (0033): ni `freshness` ni copia. Lo que se
/// tiene lo dice un `Dataset` (`documento_dataset`), que lleva la forma.
///
/// `ore view add`, el segundo emisor que esto tenía, se retiró en el paso 5:
/// una vista nueva nace de un `CREATE VIEW` en un puesto.
pub fn documento_vista(
    vista: &str,
    paquete: &str,
    schema: &str,
    owner: &str,
    tabla: &str,
    campos: &[(String, String)],
    tipos: &BTreeMap<String, String>,
) -> String {
    let con_schema = if en_default(schema) {
        String::new()
    } else {
        format!("\n  schema: {schema}")
    };
    // La forma, como se escribía hasta v1alpha13, sólo para traducirla.
    let mut forma = format!(
        "apiVersion: oos.dev/v1alpha13\nkind: View\n\
         metadata:\n  name: {vista}\n  namespace: {paquete}{con_schema}\n\
         spec:\n  owner: o\n  from: {{ table: {} }}\n  fields:\n",
        escalar_yaml(tabla)
    );
    plan_yaml(&mut forma, campos, &[]);
    let sql = parse::parse(&forma)
        .ok()
        .and_then(|root| {
            ore_core::linaje::como_sql(&ore_core::link::Loaded {
                path: Path::new("vista.yaml").to_path_buf(),
                kind: ore_core::document::Kind::View,
                root,
            })
        })
        .map(|s| crate::migrar_v14::legible(&s))
        .unwrap_or_default();
    let mut s = String::new();
    let _ = write!(
        s,
        "apiVersion: oos.dev/v1alpha14\n\
         kind: View\n\
         metadata:\n  \
           name: {vista}\n  \
           namespace: {paquete}{con_schema}\n  \
           labels: {{ oos.maturity: DRAFT }}\n\
         spec:\n  \
           owner: \"{owner}\"\n  \
           dialect: duckdb\n  \
           sql: |\n"
    );
    for l in sql.lines() {
        let _ = writeln!(s, "    {l}");
    }
    s.push_str("  columns:\n");
    for (prop, col) in campos {
        let tipo = tipos.get(col).map(String::as_str).unwrap_or("String");
        let _ = writeln!(
            s,
            "    {}: {{ type: {} }}",
            escalar_yaml(prop),
            escalar_yaml(tipo)
        );
    }
    s
}

/// **El dataset con su plan** (0033): lo que hasta aquí era una `View` con
/// `materialized`. La forma de la vista que sería —los mismos campos, en el
/// mismo orden, el mismo recorte—, que v1alpha14 mantiene en el dataset: la
/// vista la escribe como consulta (`documento_vista`) y el dataset la cumple.
/// Sin `labels`: un dataset no tiene madurez que acordar, tiene bytes.
pub fn documento_dataset(
    nombre: &str,
    paquete: &str,
    owner: &str,
    tabla: &str,
    campos: &[(String, String)],
    recorte: &[(String, Vec<String>)],
) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "apiVersion: oos.dev/v1alpha12\n\
         kind: Dataset\n\
         metadata:\n  \
           name: {nombre}\n  \
           namespace: {paquete}\n\
         # La copia en la celda: la base es estándar. El plan es el de la vista\n\
         # que sería; aquí vive porque es lo que se tiene (0033).\n\
         spec:\n  \
           owner: \"{owner}\"\n  \
           from: {{ table: {tabla} }}\n"
    );
    s.push_str("  fields:\n");
    plan_yaml(&mut s, campos, recorte);
    s
}

/// Los campos y el recorte, tal cual, para una vista o un dataset.
fn plan_yaml(s: &mut String, campos: &[(String, String)], recorte: &[(String, Vec<String>)]) {
    for (prop, col) in campos {
        let _ = writeln!(s, "    {prop}: {}", escalar_yaml(col));
    }
    if !recorte.is_empty() {
        let _ = writeln!(s, "  where:");
        for (col, valores) in recorte {
            let _ = match valores.as_slice() {
                [uno] => writeln!(s, "    {col}: {}", escalar_yaml(uno)),
                varios => writeln!(
                    s,
                    "    {col}: [{}]",
                    varios
                        .iter()
                        .map(|v| escalar_yaml(v))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
        }
    }
}

// ── El schema del origen (0038 P5) ──────────────────────────────────────────
//
// `discover` lleva el schema del origen al del catálogo: `public.ai_insights`
// de la fuente es `<base>.public.ai_insights`, no `<base>.default.public_ai_insights`
// (medido, `medida-discover-con-schema.py`: se aplanaba, y dos `pedidos` en dos
// schemas eran una colisión que sólo una persona podía deshacer). El nombre
// físico sigue entero en `object`; lo demás —la Table, su View o su Dataset,
// su Entity— vive en la carpeta del schema, en v1alpha13 y con
// `metadata.schema`. Lo que no trae schema (un fichero) es de `default`, y
// sale como siempre: un árbol de hoy no cambia.

/// El schema del catálogo de un nombre del origen: el segmento anterior a la
/// tabla (`public.x` → `public`; `proyecto.dataset.x` → `dataset`), o `default`.
fn schema_de(nombre: &str) -> String {
    let mut segs = nombre.rsplit('.');
    segs.next();
    match segs.next().map(identificador) {
        Some(s) if s.to_lowercase() != ore_core::normalize::SCHEMA_POR_DEFECTO => s,
        _ => ore_core::normalize::SCHEMA_POR_DEFECTO.to_string(),
    }
}

fn en_default(schema: &str) -> bool {
    schema == ore_core::normalize::SCHEMA_POR_DEFECTO
}

/// El nombre de la tabla sin su schema: lo que se nombra dentro de él.
fn sin_schema(nombre: &str) -> &str {
    if nombre.contains('.') {
        nombre.rsplit('.').next().unwrap_or(nombre)
    } else {
        nombre
    }
}

/// La ruta de un fichero del paquete, dentro de la carpeta de su schema.
fn en_schema(schema: &str, rel: String) -> String {
    if en_default(schema) {
        rel
    } else {
        format!("{schema}/{rel}")
    }
}

/// Un documento emitido, en su schema: v1alpha13 y `metadata.schema` (01 §3).
/// En `default`, tal cual.
fn con_schema(texto: String, schema: &str, paquete: &str) -> String {
    if en_default(schema) {
        return texto;
    }
    let mut t = texto;
    // v1alpha13 es la primera que tiene schemas; una posterior (la vista, en
    // v1alpha14) ya los tiene, y ya lo lleva escrito.
    if t.contains("apiVersion: oos.dev/v1alpha14") {
        return t;
    }
    if let Some(i) = t.find("apiVersion: oos.dev/v1alpha") {
        let fin = t[i..].find('\n').map(|f| i + f).unwrap_or(t.len());
        t.replace_range(i..fin, "apiVersion: oos.dev/v1alpha13");
    }
    let en_linea = format!(", namespace: {paquete} }}");
    let en_bloque = format!("\n  namespace: {paquete}\n");
    if t.contains(&en_linea) {
        t = t.replacen(
            &en_linea,
            &format!(", namespace: {paquete}, schema: {schema} }}"),
            1,
        );
    } else if t.contains(&en_bloque) {
        t = t.replacen(
            &en_bloque,
            &format!("\n  namespace: {paquete}\n  schema: {schema}\n"),
            1,
        );
    }
    t
}

/// Cómo se llama la pregunta de colisión de cada tabla: por su entidad, que es
/// como se ha llamado siempre (y las respuestas de antes siguen valiendo); y
/// `<schema>.<Entidad>` sólo cuando esa entidad sale en MÁS de un schema del
/// origen —ahí son dos preguntas distintas, o ninguna—. Se agrupa por el
/// valor: dos tablas con la misma clave son las que pueden colisionar.
fn claves_de_colision(tablas: &[Tabla]) -> BTreeMap<String, String> {
    let mut schemas: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in tablas {
        schemas
            .entry(entidad(&t.nombre))
            .or_default()
            .insert(schema_de(&t.nombre));
    }
    tablas
        .iter()
        .map(|t| {
            let e = entidad(&t.nombre);
            let clave = if schemas.get(&e).is_some_and(|s| s.len() > 1) {
                format!("{}.{e}", schema_de(&t.nombre))
            } else {
                e
            };
            (t.nombre.clone(), clave)
        })
        .collect()
}

/// El documento de un schema del paquete (v1alpha13 01 §2).
fn schema_yaml(schema: &str, paquete: &str, owner: &str) -> String {
    // Sin dueño decidido, ninguno: el de un schema es opcional y responde el
    // del paquete, que ya lleva la pregunta (una sola, no una por schema).
    let spec = if handle(owner) {
        format!(
            "spec:
  owner: {owner}
"
        )
    } else {
        "spec: {}
"
        .to_string()
    };
    format!(
        "apiVersion: oos.dev/v1alpha13
kind: Schema
metadata:
  name: {schema}
  namespace: {paquete}
# El schema `{schema}` del origen (0038 P5): lo que ahí vive se llama
# `{paquete}.{schema}.<nombre>`.
{spec}"
    )
}

// ── Nombres ─────────────────────────────────────────────────────────────────

/// El nombre de entidad de una tabla. Se toma el último segmento —el esquema y
/// el catálogo son del origen, no del dominio— y se capitaliza. **No se
/// singulariza**: eso sería adivinar un idioma.
fn entidad(tabla: &str) -> String {
    let ultimo = tabla.rsplit(['.', '/']).next().unwrap_or(tabla);
    capitalizar(&identificador(ultimo))
}

/// La inicial en minúscula. El gemelo exacto de `capitalizar`, y por eso está
/// al lado: una entidad se llama `Clientes` y su vista `clientes`, que es el
/// mismo nombre visto desde la capa de abajo.
fn minuscula_inicial(id: &str) -> String {
    let mut c = id.chars();
    match c.next() {
        Some(p) => p.to_lowercase().collect::<String>() + c.as_str(),
        None => id.to_string(),
    }
}

fn capitalizar(id: &str) -> String {
    let mut c = id.chars();
    match c.next() {
        Some(p) => p.to_uppercase().collect::<String>() + c.as_str(),
        None => id.to_string(),
    }
}

/// Lo que no encaja en `^[a-zA-Z][a-zA-Z0-9_]*$` se sustituye por `_`. Un
/// identificador que empezara por dígito lleva `t_` delante, y eso **sí** es
/// inventar un carácter — por eso el nombre físico sigue entero en `columns` de
/// la tabla y en el valor de `fields` de la vista, que son los dos sitios donde
/// lo físico se dice tal cual. (Decía «en el binding»: se retiró en v1alpha8.)
fn identificador(bruto: &str) -> String {
    let mut out = String::new();
    for c in bruto.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        return "sin_nombre".into();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("t_{out}")
    } else {
        out
    }
}

// ── El informe ──────────────────────────────────────────────────────────────

/// Lo que se puede contestar, cuando no se puede adivinar.
///
/// No se enseñan las opciones de todas las clases y no es por ahorrar líneas:
/// `si`/`no`, `entidad`/`omitir` y las columnas de la propia tabla se derivan de
/// la pregunta, y repetirlas es ruido. Las de estas dos **no**: los conceptos
/// publicados y los niveles de un retículo viven en otro sitio del repositorio,
/// y quien lee la cola no tiene por qué saber cuáles son.
pub fn sugerencias(p: &Pendiente) -> Option<String> {
    if !matches!(p.clase, Clase::Concepto | Clase::Clasificacion) {
        return None;
    }
    const MUESTRA: usize = 4;
    let utiles: Vec<&String> = p.opciones.iter().filter(|o| *o != "no").collect();
    if utiles.is_empty() {
        return None;
    }
    let mostradas: Vec<String> = utiles.iter().take(MUESTRA).map(|o| (*o).clone()).collect();
    let cola = if utiles.len() > MUESTRA {
        format!(", y {} más", utiles.len() - MUESTRA)
    } else {
        String::new()
    };
    Some(format!("{}{cola}", mostradas.join("  ·  ")))
}

pub fn informe(ind: &Induccion, destino: &Path) -> String {
    // Por la carpeta del kind, esté en la raíz del paquete o en la de un
    // schema (0038 P5: `public/tables/…`).
    let cuantos = |carpeta: &str| {
        ind.ficheros
            .keys()
            .filter(|k| k.rsplit('/').nth(1) == Some(carpeta))
            .count()
    };
    let (entidades, tablas, vistas, datasets) = (
        cuantos("entities"),
        cuantos("tables"),
        cuantos("views"),
        cuantos("datasets"),
    );
    // Decía «entidades y sus bindings», y hacía años que no emitía ninguno: los
    // bindings se retiraron en v1alpha8. Un mensaje que nombra lo que ya no se
    // escribe es peor que uno que calla, porque enseña el paradigma anterior a
    // quien está viendo el paquete por primera vez.
    let mut s = format!(
        "  ✓ {entidades} entidades, {tablas} tablas, {vistas} vistas y {datasets} datasets en {}\n\
         \x20 ✓ todas en DRAFT: nada de esto es verdad todavía\n\n",
        destino.display()
    );
    if ind.pendientes.is_empty() {
        s.push_str("  Sin decisiones pendientes.\n");
        return s;
    }
    let _ = writeln!(
        s,
        "  {} decisiones te esperan. Ninguna se ha tomado por ti:\n",
        ind.pendientes.len()
    );
    for p in &ind.pendientes {
        let _ = writeln!(s, "  · {} — {}", p.sujeto, p.que);
        let _ = writeln!(s, "    {}", p.porque);
        if let Some(o) = sugerencias(p) {
            let _ = writeln!(s, "    → {o}");
        }
        // El identificador es lo que se escribe a la izquierda en un fichero de
        // respuestas. Sin él, contestar en diferido exige adivinarlo.
        let _ = writeln!(s, "    {}\n", p.id);
    }
    s.push_str("  ore review <ruta>     ·   la cola, una pregunta cada vez\n");
    s.push_str("  ore validate <ruta>   ·   las que el compilador ya sabe decir\n");
    s
}

/// El informe también en JSON, porque `ore review` lo lee y una persona no es el
/// único consumidor de esto.
///
/// Cada decisión lleva su `id` y sus `options`, que son las dos cosas que hacen
/// falta para contestarla sin haber visto la pantalla: el identificador es la
/// izquierda de una línea de un fichero de respuestas, y las opciones son la
/// derecha. Una cola serializada sin ellos se puede leer y no se puede contestar.
///
/// ── ⛔⛔ Y `form`, QUE FALTABA Y NO SE VEÍA ────────────────────────────────
///
/// `options` es una lista en todas las clases, y la respuesta NO siempre lo es:
///
/// ```text
///   clave/ventas.clientes   ["id"]         una lista
///   dueno/ventas            "team:datos"   una cadena
/// ```
///
/// ⇒ Sin decirlo, un cliente que pinte esta cola **tiene que saberse las once
///   clases** y cuáles admiten varias — o sea, tener una segunda copia de algo
///   que ya vive aquí. Y esta misma ruta se prohíbe eso: *«reordenar u omitir
///   aquí sería una segunda opinión sobre lo que hay que preguntar»*.
///
/// ⭐ Se sirve [`Forma`], que es la palabra que el motor ya usaba, y son CUATRO
///   casos y no dos. Así la interfaz no elige el control: lo lee.
///
/// ⚠️ Y hereda la propiedad de su `match`: **una clase nueva no compila** hasta
///   que alguien decide cómo se contesta. Desde hoy eso alcanza también a una
///   pantalla que todavía no existe.
pub fn informe_json(ind: &Induccion) -> Json {
    Json::obj([(
        "pending",
        Json::Arr(
            ind.pendientes
                .iter()
                .map(|p| {
                    Json::obj([
                        ("id", Json::s(&p.id)),
                        ("class", Json::s(p.clase.prefijo())),
                        ("form", Json::s(p.clase.forma().nombre())),
                        ("subject", Json::s(&p.sujeto)),
                        ("decision", Json::s(&p.que)),
                        ("because", Json::s(&p.porque)),
                        (
                            "options",
                            Json::Arr(p.opciones.iter().map(Json::s).collect()),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// La regla de la base estándar (0033): una tabla CON clave sale con su
    /// `Dataset` (en `datasets/`, con el plan) y su tabla en `upsert` por esa
    /// clave; una sin clave sale con su `View` (en `views/`, sin copia) — y la
    /// decisión `clave` dice que la copia la espera. Con la clave contestada,
    /// el dataset aparece en la re-inducción.
    #[test]
    fn la_base_estandar_copia_lo_que_tiene_clave_y_espera_lo_demas() {
        let cat = Catalogo::leer(CATALOGO).unwrap();
        let todas = |estandar| Regla {
            estandar,
            modeladas: None,
            copiadas: BTreeSet::new(),
            schemas: BTreeMap::new(),
        };
        let sin = inducir_con_regla(
            &cat,
            "ventas",
            &Decisiones::default(),
            &Vocabulario::default(),
            &todas(false),
        );
        for (k, t) in &sin.ficheros {
            assert!(
                !k.starts_with("rubix_demo_ventas/datasets/") && !t.contains("kind: Dataset"),
                "foránea con copia en {k}:
{t}"
            );
        }
        let con = inducir_con_regla(
            &cat,
            "ventas",
            &Decisiones::default(),
            &Vocabulario::default(),
            &todas(true),
        );
        let vista = &con.ficheros["rubix_demo_ventas/datasets/Facturas__facturas.yaml"];
        assert!(
            vista.contains("kind: Dataset") && vista.contains("from: { table: facturas_t }"),
            "{vista}"
        );
        assert!(
            !con.ficheros
                .contains_key("rubix_demo_ventas/views/Facturas__facturas.yaml"),
            "la copia no deja una vista que sea la misma cosa"
        );
        let tabla = &con.ficheros["rubix_demo_ventas/tables/Facturas__facturas.yaml"];
        assert!(
            tabla.contains(
                "mode: upsert
    key: [id_factura]"
            ),
            "{tabla}"
        );
        let clientes = &con.ficheros["rubix_demo_ventas/views/Clientes__clientes.yaml"];
        assert!(
            clientes.contains("kind: View"),
            "sin clave y con copia:
{clientes}"
        );
        let clave = con
            .pendientes
            .iter()
            .find(|p| p.id == "clave/rubix_demo_ventas.clientes")
            .expect("la decisión clave de clientes");
        assert!(
            clave.porque.contains("la COPIA de esta tabla espera"),
            "{}",
            clave.porque
        );
        // contestada la clave, la copia aparece
        let dec = Decisiones::leer(
            "answers:
  clave/rubix_demo_ventas.clientes: [id]
",
        )
        .unwrap();
        let despues =
            inducir_con_regla(&cat, "ventas", &dec, &Vocabulario::default(), &todas(true));
        let clientes = &despues.ficheros["rubix_demo_ventas/datasets/Clientes__clientes.yaml"];
        assert!(clientes.contains("kind: Dataset"), "{clientes}");
        let tabla = &despues.ficheros["rubix_demo_ventas/tables/Clientes__clientes.yaml"];
        assert!(
            tabla.contains(
                "mode: upsert
    key: [id]"
            ),
            "{tabla}"
        );
    }

    /// El catálogo no modela: sin `entities`, una tabla da Table + View y
    /// ninguna decisión de modelado; con la base estándar, la copia no espera
    /// a ninguna clave. Modelada, vuelve a tener Entity y su cola.
    #[test]
    fn el_catalogo_no_modela_y_la_copia_no_espera() {
        let cat = Catalogo::leer(CATALOGO).unwrap();
        let ninguna = Regla {
            estandar: true,
            modeladas: Some(BTreeSet::new()),
            copiadas: BTreeSet::new(),
            schemas: BTreeMap::new(),
        };
        let i = inducir_con_regla(
            &cat,
            "ventas",
            &Decisiones::default(),
            &Vocabulario::default(),
            &ninguna,
        );
        assert!(
            !i.ficheros
                .keys()
                .any(|k| k.starts_with("rubix_demo_ventas/entities/")),
            "{:?}",
            i.ficheros.keys()
        );
        assert!(
            !i.ficheros.keys().any(|k| k.starts_with("concepts/")),
            "{:?}",
            i.ficheros.keys()
        );
        let datasets: Vec<&String> = i
            .ficheros
            .keys()
            .filter(|k| k.starts_with("rubix_demo_ventas/datasets/"))
            .collect();
        assert_eq!(
            datasets.len(),
            7,
            "un dataset por tabla del catálogo: {datasets:?}"
        );
        assert!(
            !i.ficheros
                .keys()
                .any(|k| k.starts_with("rubix_demo_ventas/views/")),
            "estándar y sin modelar: se copia sin esperar, y sin vistas que sean la misma cosa"
        );
        for k in &datasets {
            assert!(
                i.ficheros[*k].contains("kind: Dataset"),
                "estándar y sin modelar: se copia sin esperar · {k}"
            );
        }
        // sin clave, la tabla se queda como el origen la dijo; con clave, upsert
        let clientes = &i.ficheros["rubix_demo_ventas/tables/Clientes__clientes.yaml"];
        assert!(!clientes.contains("upsert"), "{clientes}");
        let facturas = &i.ficheros["rubix_demo_ventas/tables/Facturas__facturas.yaml"];
        assert!(
            facturas.contains("mode: upsert\n    key: [id_factura]"),
            "{facturas}"
        );
        // la colisión de nombres (Pedidos / pedidos) se resuelve con el físico, sin preguntar
        assert!(
            i.ficheros
                .contains_key("rubix_demo_ventas/datasets/Pedidos__pedidos.yaml"),
            "{:?}",
            i.ficheros.keys()
        );
        let clases: BTreeSet<String> = i
            .pendientes
            .iter()
            .map(|p| p.id.split('/').next().unwrap().to_string())
            .collect();
        assert_eq!(
            clases.iter().cloned().collect::<Vec<_>>(),
            vec!["dueno", "filas"],
            "{clases:?}"
        );
        // modelada una, vuelve su entidad y su cola
        let una = Regla {
            estandar: true,
            modeladas: Some(["rubix_demo_ventas.clientes".to_string()].into()),
            copiadas: BTreeSet::new(),
            schemas: BTreeMap::new(),
        };
        // y en una foránea, una tabla copiada una a una: sólo ésa
        let suelta = Regla {
            estandar: false,
            modeladas: Some(BTreeSet::new()),
            copiadas: ["rubix_demo_ventas.facturas".to_string()].into(),
            schemas: BTreeMap::new(),
        };
        let f = inducir_con_regla(
            &cat,
            "ventas",
            &Decisiones::default(),
            &Vocabulario::default(),
            &suelta,
        );
        assert!(
            f.ficheros["rubix_demo_ventas/datasets/Facturas__facturas.yaml"]
                .contains("kind: Dataset")
        );
        assert!(
            f.ficheros["rubix_demo_ventas/views/Clientes__clientes.yaml"].contains("kind: View")
        );
        let m = inducir_con_regla(
            &cat,
            "ventas",
            &Decisiones::default(),
            &Vocabulario::default(),
            &una,
        );
        assert!(
            m.ficheros
                .contains_key("rubix_demo_ventas/entities/Clientes.yaml"),
            "{:?}",
            m.ficheros.keys()
        );
        assert!(
            m.pendientes
                .iter()
                .any(|p| p.id == "clave/rubix_demo_ventas.clientes")
        );
        assert!(
            m.ficheros["rubix_demo_ventas/views/Clientes__clientes.yaml"].contains("kind: View"),
            "modelada sin clave: la copia espera"
        );
    }

    const CATALOGO: &str = r#"{
      "source": "bq_ventas",
      "tables": [
        { "name": "rubix_demo_ventas.pedidos",
          "columns": [
            { "name": "id_pedido", "type": "Integer", "required": true },
            { "name": "id_cliente", "type": "Integer", "required": true },
            { "name": "total", "type": "Decimal" }
          ],
          "primaryKey": ["id_pedido"], "rows": "50000" },
        { "name": "rubix_demo_ventas.Pedidos",
          "columns": [ { "name": "Id", "type": "Integer" } ], "rows": "120" },
        { "name": "rubix_demo_ventas.facturas",
          "columns": [
            { "name": "id_factura", "type": "Integer", "required": true },
            { "name": "id_cliente", "type": "Integer" },
            { "name": "total", "type": "Decimal" }
          ],
          "primaryKey": ["id_factura"], "rows": "900" },
        { "name": "rubix_demo_ventas.clientes",
          "columns": [
            { "name": "id", "type": "Integer", "required": true },
            { "name": "total", "type": "Decimal" }
          ],
          "rows": "5000" },
        { "name": "rubix_demo_ventas.evento_20190101",
          "columns": [ { "name": "id", "type": "Integer" } ], "rows": "8000" },
        { "name": "rubix_demo_ventas.evento_20190102",
          "columns": [ { "name": "id", "type": "Integer" } ], "rows": "8000" },
        { "name": "rubix_demo_ventas.mov_bak",
          "columns": [ { "name": "f1", "type": "String" } ], "rows": "0" }
      ]
    }"#;

    fn inducido() -> Induccion {
        inducir(&Catalogo::leer(CATALOGO).unwrap(), "ventas")
    }

    #[test]
    fn una_tabla_es_una_entidad_y_eso_es_un_hecho() {
        let i = inducido();
        assert!(
            i.ficheros
                .contains_key("rubix_demo_ventas/entities/Facturas.yaml")
        );
        // Tres documentos y no dos: el objeto, lo que se expone de él, y lo que
        // significa. Cada capa sabe solo lo suyo.
        let tabla = &i.ficheros["rubix_demo_ventas/tables/Facturas__facturas.yaml"];
        let vista = &i.ficheros["rubix_demo_ventas/views/Facturas__facturas.yaml"];
        let entidad = &i.ficheros["rubix_demo_ventas/entities/Facturas.yaml"];

        // El nombre físico viaja ENTERO a la tabla: es opaco y es del origen.
        assert!(
            tabla.contains(r#"object: "rubix_demo_ventas.facturas""#),
            "{tabla}"
        );
        assert!(tabla.contains("kind: Table"), "{tabla}");
        // Y la entidad nombra a la VISTA, nunca a la tabla: si nombrara la
        // tabla, sus propiedades tendrían que llamarse como las columnas
        // físicas y lo semántico volvería a saber de lo físico.
        assert!(entidad.contains("backedBy: facturas"), "{entidad}");
        // Y la lee por su tabla, que se llama `<objeto>_t`: la vista ya se
        // llama `facturas`, y en v1alpha14 un nombre es una sola cosa.
        assert!(
            vista.contains(r#"FROM "ventas"."rubix_demo_ventas"."facturas_t""#),
            "{vista}"
        );
        assert!(tabla.contains("name: facturas_t"), "{tabla}");
    }

    /// La tabla lleva el tipo que el conector tradujo (0032 §3). Antes se tiraba
    /// aquí «porque el tipo es de la entidad», y una tabla sin entidad se copiaba
    /// entera como texto. La cita del origen va al lado cuando la hay; una
    /// columna sin traducir lleva solo la cita.
    #[test]
    fn la_tabla_lleva_el_tipo_que_el_conector_tradujo_y_la_cita_del_origen() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.pedidos",
              "columns": [
                { "name": "id", "type": "Integer", "sourceType": "bigint" },
                { "name": "total", "type": "Decimal", "sourceType": "numeric(10,2)" },
                { "name": "pais", "type": "String" },
                { "name": "payload", "sourceType": "jsonb" }
              ],
              "primaryKey": ["id"] }
          ]
        }"#;
        let i = inducir(&Catalogo::leer(CAT).unwrap(), "ventas");
        let tabla = &i.ficheros["public/tables/Pedidos__pedidos.yaml"];
        assert!(
            tabla.contains("    id: { type: Integer, physicalType: bigint }"),
            "{tabla}"
        );
        assert!(
            tabla.contains("    total: { type: Decimal, physicalType: \"numeric(10,2)\" }"),
            "{tabla}"
        );
        assert!(tabla.contains("    pais: { type: String }"), "{tabla}");
        assert!(
            tabla.contains("    payload: { physicalType: jsonb }"),
            "{tabla}"
        );
    }

    /// La colisión no se resuelve: se reporta. Elegir una decidiría cuál de las
    /// dos tablas existe.
    #[test]
    fn dos_tablas_que_colisionan_no_se_emiten() {
        let i = inducido();
        assert!(i.pendientes.iter().any(|p| p.que.contains("colisionan")));
        // Y ninguna de las dos sale: emitir una sería elegir.
        assert!(
            !i.ficheros
                .values()
                .any(|f| f.contains("source: \"rubix_demo_ventas.Pedidos\""))
        );
    }

    #[test]
    fn sin_clave_no_se_inventa_una() {
        let i = inducido();
        assert!(
            i.pendientes
                .iter()
                .any(|p| p.sujeto.contains("clientes") && p.que.contains("clave")),
            "no reportó la falta de clave"
        );
        let e = &i.ficheros["rubix_demo_ventas/entities/Clientes.yaml"];
        assert!(!e.contains("primaryKey"), "se inventó una clave:\n{e}");
    }

    #[test]
    fn las_fragmentadas_por_fecha_se_ven_juntas() {
        let i = inducido();
        let p = i
            .pendientes
            .iter()
            .find(|p| p.que.contains("familia fechada"))
            .expect("no vio la familia");
        assert!(
            p.sujeto.contains("evento_20190101") && p.sujeto.contains("evento_20190102"),
            "{}",
            p.sujeto
        );
    }

    /// El caso que se escapaba, y es el más común de un almacén real: `pedidos`
    /// y `pedidos_2024`. La regla anterior exigía DOS nombres con sufijo
    /// numérico, y la tabla viva no lleva ninguno — así que la familia entera
    /// pasaba de largo justo cuando había algo que decidir.
    #[test]
    fn una_hermana_sin_digitos_tambien_hace_familia() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.pedidos",
              "columns": [ { "name": "id", "type": "Integer" } ], "primaryKey": ["id"] },
            { "name": "public.pedidos_2024",
              "columns": [ { "name": "id", "type": "Integer" } ], "primaryKey": ["id"] }
          ]
        }"#;
        let i = inducir(&Catalogo::leer(CAT).unwrap(), "ventas");
        let p = i
            .pendientes
            .iter()
            .find(|p| p.clase == Clase::Familia)
            .expect("no vio la familia de `pedidos`");
        assert_eq!(p.id, "familia/public.pedidos");
    }

    /// Y dos tablas cuya raíz coincide sin que ninguna esté numerada NO son una
    /// familia: son dos tablas. La regla exige al menos una hermana con sufijo.
    #[test]
    fn dos_tablas_parecidas_no_son_una_familia() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.pedido",
              "columns": [ { "name": "id", "type": "Integer" } ], "primaryKey": ["id"] },
            { "name": "public.pedidos",
              "columns": [ { "name": "id", "type": "Integer" } ], "primaryKey": ["id"] }
          ]
        }"#;
        let i = inducir(&Catalogo::leer(CAT).unwrap(), "ventas");
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Familia));
    }

    /// `total` está en dos tablas con el mismo tipo. Se muestran **juntas** para
    /// que la unificación se decida una vez — y NO se acuña un concepto.
    #[test]
    fn una_columna_repetida_es_candidata_y_no_concepto() {
        let i = inducido();
        let p = i
            .pendientes
            .iter()
            .find(|p| p.sujeto.contains("`total: Decimal`"))
            .expect("no agrupó la columna repetida");
        assert!(p.sujeto.contains("pedidos") && p.sujeto.contains("clientes"));
        assert!(!i.ficheros.keys().any(|k| k.starts_with("concepts/")));
    }

    /// El caso que antes no se podia escribir. `via` era un identificador, asi
    /// que una foranea compuesta se emitia recortada a su primera columna — un
    /// join que une de menos con el mismo aspecto que uno correcto. Cerrada la
    /// decision, se dice entera y EN ORDEN.
    #[test]
    fn una_foranea_compuesta_se_dice_entera() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.clientes",
              "columns": [
                { "name": "id", "type": "Integer", "required": true },
                { "name": "cod_pais", "type": "String", "required": true }
              ],
              "primaryKey": ["id", "cod_pais"] },
            { "name": "public.facturas",
              "columns": [
                { "name": "id_factura", "type": "Integer", "required": true },
                { "name": "id_cliente", "type": "Integer", "required": true },
                { "name": "cod_pais", "type": "String", "required": true }
              ],
              "primaryKey": ["id_factura"],
              "foreignKeys": [
                { "columns": ["id_cliente", "cod_pais"], "references": "public.clientes" }
              ] }
          ]
        }"#;
        let i = inducir(&Catalogo::leer(CAT).unwrap(), "ventas");
        let f = &i.ficheros["public/entities/Facturas.yaml"];
        assert!(f.contains("via: [id_cliente, cod_pais]"), "{f}");
        // Apunta a la clave primaria del destino, asi que `toKey` sobra (P2).
        assert!(
            !f.contains("toKey"),
            "declaro lo derivable:
{f}"
        );
        // Las dos columnas son NOT NULL, asi que el enlace es obligatorio: eso
        // lo dice el origen, no un valor por defecto.
        assert!(f.contains("required: true"), "{f}");
        // Y ya no queda nada que reportar sobre ella.
        assert!(
            !i.pendientes.iter().any(|p| p.que.contains("compuesta")),
            "sigue reportando lo que ya sabe emitir"
        );
    }

    /// SQL no obliga a referenciar la clave primaria, y callarlo emitiria un
    /// enlace contra la identidad equivocada que pasa la comprobacion de aridad
    /// y tipos por casualidad: verde y falso.
    #[test]
    fn una_foranea_contra_otra_clave_dice_cual() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.clientes",
              "columns": [
                { "name": "id",  "type": "Integer", "required": true },
                { "name": "nif", "type": "String",  "required": true }
              ],
              "primaryKey": ["id"],
              "uniqueKeys": [["nif"]] },
            { "name": "public.facturas",
              "columns": [
                { "name": "id_factura",  "type": "Integer", "required": true },
                { "name": "nif_cliente", "type": "String",  "required": true }
              ],
              "primaryKey": ["id_factura"],
              "foreignKeys": [
                { "columns": ["nif_cliente"], "references": "public.clientes",
                  "toColumns": ["nif"] }
              ] }
          ]
        }"#;
        let i = inducir(&Catalogo::leer(CAT).unwrap(), "ventas");
        let f = &i.ficheros["public/entities/Facturas.yaml"];
        assert!(f.contains("via: [nif_cliente]"), "{f}");
        assert!(
            f.contains("toKey: [nif]"),
            "no dijo contra qué clave enlaza:\n{f}"
        );
        // Y el destino tiene que DECLARAR esa clave, o `toKey` apunta a algo que
        // no identifica. El lector trae las UNIQUE del origen justo para esto:
        // sin ellas la cadena emitía un `toKey` que su propio destino no
        // sostenía, y salía OOS3006 sobre un documento que nadie escribió.
        let c = &i.ficheros["public/entities/Clientes.yaml"];
        assert!(c.contains("uniqueKeys:") && c.contains("- [nif]"), "{c}");
    }

    /// Y una tabla no se relaciona consigo misma por llamarse como su clave.
    /// `pedidos.id_pedido` es la IDENTIDAD de `pedidos`: el sufijo `_id` dice
    /// «esto identifica algo», y la mitad de las veces ese algo es la propia
    /// fila. Salió sobre datos reales, no leyendo.
    #[test]
    fn la_clave_propia_no_es_una_arista_a_si_misma() {
        let i = inducido();
        assert!(
            !i.pendientes
                .iter()
                .any(|p| p.sujeto.ends_with("facturas.id_factura")),
            "propuso una relación de una tabla consigo misma"
        );
    }

    /// `id_cliente` se parece a `Clientes`, y un parecido no es una arista.
    #[test]
    fn un_parecido_de_nombres_no_se_convierte_en_arista() {
        let i = inducido();
        assert!(
            i.pendientes
                .iter()
                .any(|p| p.sujeto.ends_with("facturas.id_cliente") && p.que.contains("relación")),
            "no reportó la relación posible"
        );
        assert!(
            !i.ficheros["rubix_demo_ventas/entities/Facturas.yaml"].contains("relations"),
            "emitió una arista que nadie declaró"
        );
    }

    #[test]
    fn una_tabla_vacia_no_se_borra_sola() {
        let i = inducido();
        assert!(i.pendientes.iter().any(|p| p.sujeto.contains("mov_bak")));
        assert!(
            i.ficheros
                .contains_key("rubix_demo_ventas/entities/Mov_bak.yaml")
        );
    }

    fn decisiones(respuestas: &[(&str, Respuesta)]) -> Decisiones {
        let mut d = Decisiones::default();
        for (k, v) in respuestas {
            d.responder(*k, v.clone());
        }
        d
    }

    fn con(respuestas: &[(&str, Respuesta)]) -> Induccion {
        inducir_con(
            &Catalogo::leer(CATALOGO).unwrap(),
            "ventas",
            &decisiones(respuestas),
            &Vocabulario::default(),
        )
    }

    /// El motivo de un tipo sin traducir tiene que ser SU motivo. A un `numeric`
    /// se le decía «puede ser un objeto embebido o una entidad aparte», que no es
    /// su pregunta: la suya es cuántos decimales y en qué moneda.
    #[test]
    fn un_decimal_sin_precision_pregunta_por_la_moneda() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.pedidos",
              "columns": [
                { "name": "id", "type": "Integer" },
                { "name": "importe", "sourceType": "numeric" },
                { "name": "payload", "sourceType": "jsonb" }
              ],
              "primaryKey": ["id"] }
          ]
        }"#;
        let i = inducir(&Catalogo::leer(CAT).unwrap(), "ventas");
        let p = |c: &str| {
            i.pendientes
                .iter()
                .find(|p| p.sujeto.ends_with(c))
                .unwrap_or_else(|| panic!("no preguntó por {c}"))
                .porque
                .clone()
        };
        assert!(p("importe").contains("moneda"), "{}", p("importe"));
        assert!(
            !p("importe").contains("objeto embebido"),
            "{}",
            p("importe")
        );
        assert!(p("payload").contains("objeto embebido"), "{}", p("payload"));
    }

    /// La respuesta ESCRIBE en lo inducido. No hay un estado aparte que consultar
    /// después: el `primaryKey` está en el documento o la decisión no se tomó.
    #[test]
    fn una_clave_contestada_se_escribe_y_cierra_la_pregunta() {
        let i = con(&[(
            "clave/rubix_demo_ventas.clientes",
            Respuesta::Lista(vec!["id".into()]),
        )]);
        assert!(
            i.ficheros["rubix_demo_ventas/entities/Clientes.yaml"].contains("primaryKey: [id]"),
            "{}",
            i.ficheros["rubix_demo_ventas/entities/Clientes.yaml"]
        );
        assert!(
            !i.pendientes
                .iter()
                .any(|p| p.id == "clave/rubix_demo_ventas.clientes")
        );
    }

    /// Y una columna que no existe no es una clave. Contestar mal no puede tener
    /// el mismo aspecto que contestar: la pregunta sigue viva.
    #[test]
    fn una_clave_que_nombra_lo_que_no_existe_no_cierra_nada() {
        let i = con(&[(
            "clave/rubix_demo_ventas.clientes",
            Respuesta::Lista(vec!["no_existe".into()]),
        )]);
        assert!(!i.ficheros["rubix_demo_ventas/entities/Clientes.yaml"].contains("primaryKey"));
        assert!(
            i.pendientes
                .iter()
                .any(|p| p.id == "clave/rubix_demo_ventas.clientes")
        );
    }

    /// Las dos tablas existen y se llaman distinto. Antes no salía ninguna:
    /// emitir una habría decidido cuál de las dos existe.
    #[test]
    fn una_colision_contestada_emite_las_dos() {
        let i = con(&[(
            "colision/Pedidos",
            Respuesta::Mapa(BTreeMap::from([
                (
                    "rubix_demo_ventas.pedidos".to_string(),
                    "Pedidos".to_string(),
                ),
                (
                    "rubix_demo_ventas.Pedidos".to_string(),
                    "PedidosViejos".to_string(),
                ),
            ])),
        )]);
        assert!(
            i.ficheros
                .contains_key("rubix_demo_ventas/entities/Pedidos.yaml")
        );
        assert!(
            i.ficheros
                .contains_key("rubix_demo_ventas/entities/PedidosViejos.yaml")
        );
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Colision));
        // Y cada una conserva su nombre físico, que es del origen y es opaco.
        let b = &i.ficheros["rubix_demo_ventas/tables/PedidosViejos__Pedidos.yaml"];
        assert!(b.contains(r#"object: "rubix_demo_ventas.Pedidos""#), "{b}");
        // La flecha va al revés que en el binding: la entidad nombra a su vista.
        let e = &i.ficheros["rubix_demo_ventas/entities/PedidosViejos.yaml"];
        assert!(e.contains("backedBy: pedidosViejos"), "{e}");
    }

    /// **Dos ficheros que solo se distinguen por las mayusculas son UN fichero**
    /// en Windows y en macOS, y esto se midio perdiendo uno.
    ///
    /// `rubix_demo_ventas.Pedidos` y `rubix_demo_ventas.pedidos` daban
    /// `bindings/rubix_demo_ventas_Pedidos.yaml` y su gemelo en minusculas: dos
    /// claves distintas en memoria y **la misma ruta en disco**. El segundo piso
    /// al primero, quedo el nombre de uno con el contenido del otro, y una de las
    /// dos entidades se quedo sin puntero fisico. `ore validate` salio verde,
    /// porque una entidad sin fuente es legal en DRAFT — el peor final posible.
    ///
    /// Con v1alpha8 el riesgo se DUPLICA: donde habia un binding hay ahora una
    /// tabla y una vista, asi que la misma colision perderia dos documentos.
    ///
    /// La entidad delante lo cierra sin pedir una respuesta nueva: esos nombres
    /// ya son unicos porque **es lo que la decision de colision resolvio**.
    #[test]
    fn dos_tablas_que_solo_difieren_en_mayusculas_dan_dos_ficheros() {
        let i = con(&[(
            "colision/Pedidos",
            Respuesta::Mapa(BTreeMap::from([
                (
                    "rubix_demo_ventas.pedidos".to_string(),
                    "Pedidos".to_string(),
                ),
                (
                    "rubix_demo_ventas.Pedidos".to_string(),
                    "PedidosViejos".to_string(),
                ),
            ])),
        )]);
        let rutas: Vec<&String> = i
            .ficheros
            .keys()
            .filter(|k| {
                k.starts_with("rubix_demo_ventas/tables/")
                    || k.starts_with("rubix_demo_ventas/views/")
            })
            .collect();
        for (n, a) in rutas.iter().enumerate() {
            for b in rutas.iter().skip(n + 1) {
                assert!(
                    !a.eq_ignore_ascii_case(b),
                    "`{a}` y `{b}` son el mismo fichero en un sistema que no                      distingue mayusculas, asi que uno de los dos se pierde"
                );
            }
        }
        // Y las dos de la colision estan, cada una con su entidad delante y con
        // sus DOS documentos: el objeto y lo que se expone de el.
        for r in [
            "rubix_demo_ventas/tables/Pedidos__pedidos.yaml",
            "rubix_demo_ventas/tables/PedidosViejos__Pedidos.yaml",
            "rubix_demo_ventas/views/Pedidos__pedidos.yaml",
            "rubix_demo_ventas/views/PedidosViejos__Pedidos.yaml",
        ] {
            assert!(i.ficheros.contains_key(r), "falta {r}: {rutas:?}");
        }
    }

    /// Una vista que resulta ser un informe no es una entidad, y dejar de
    /// emitirla es la única respuesta que lo dice.
    #[test]
    fn omitir_algo_lo_saca_del_paquete() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.v_activos", "kind": "view",
              "columns": [ { "name": "id", "type": "Integer" } ], "primaryKey": ["id"] }
          ]
        }"#;
        let d = decisiones(&[("vista/public.v_activos", Respuesta::Palabra(OMITIR.into()))]);
        let i = inducir_con(
            &Catalogo::leer(CAT).unwrap(),
            "ventas",
            &d,
            &Vocabulario::default(),
        );
        assert!(!i.ficheros.keys().any(|k| k.starts_with("entities/")));
        // Y con ella se van sus preguntas: lo que no existe no tiene clave ni
        // filas que decidir. Solo queda la del dueño, que es del paquete.
        assert!(
            i.pendientes.iter().all(|p| p.clase == Clase::Dueno),
            "{:?}",
            i.pendientes.iter().map(|p| &p.id).collect::<Vec<_>>()
        );
    }

    /// **`separadas` es ahora la única forma de quedarse las dos.** Cada hermana
    /// es su entidad, con su tabla y su vista, y cada una lleva SOLO las columnas
    /// que su objeto tiene: atribuirle a la de 2023 una columna de 2024 sería un
    /// mapeo verde y falso, y eso no cambia porque cambie la gramática.
    #[test]
    fn una_familia_separada_da_una_entidad_por_hermana() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.pedidos_2023",
              "columns": [
                { "name": "id", "type": "Integer" },
                { "name": "fecha", "type": "Date" }
              ],
              "primaryKey": ["id"] },
            { "name": "public.pedidos_2024",
              "columns": [
                { "name": "id", "type": "Integer" },
                { "name": "fecha", "type": "Date" },
                { "name": "canal", "type": "String" }
              ],
              "primaryKey": ["id"] }
          ]
        }"#;
        let d = decisiones(&[(
            "familia/public.pedidos",
            Respuesta::Palabra("separadas".into()),
        )]);
        let i = inducir_con(
            &Catalogo::leer(CAT).unwrap(),
            "ventas",
            &d,
            &Vocabulario::default(),
        );
        for f in [
            "public/entities/Pedidos_2023.yaml",
            "public/entities/Pedidos_2024.yaml",
            "public/tables/Pedidos_2023__pedidos_2023.yaml",
            "public/views/Pedidos_2024__pedidos_2024.yaml",
        ] {
            assert!(i.ficheros.contains_key(f), "{:?}", i.ficheros.keys());
        }
        // Y la pregunta se cierra: `separadas` es una respuesta, no un aplazo.
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Familia));

        let viejo = &i.ficheros["public/tables/Pedidos_2023__pedidos_2023.yaml"];
        assert!(
            !viejo.contains("canal"),
            "atribuyó una columna que no está ahí:\n{viejo}"
        );
        assert!(i.ficheros["public/tables/Pedidos_2024__pedidos_2024.yaml"].contains("canal"));
    }

    /// **Unir dejó de poder escribirse, y una respuesta vieja no se traga.**
    ///
    /// Hasta v1alpha8 nombrar una columna unía la familia: una entidad servida
    /// desde N tablas, que eran N bindings. El binding se retiró y una vista
    /// sale de un sitio, así que la respuesta ya no se puede honrar — y una que
    /// no se puede honrar se dice, no se ignora: quien la escribió se quedaría
    /// creyendo que unió algo.
    #[test]
    fn una_respuesta_que_pedia_unir_reabre_la_pregunta() {
        const CAT: &str = r#"{
          "source": "pg",
          "tables": [
            { "name": "public.log_2023",
              "columns": [ { "name": "id", "type": "Integer" } ], "primaryKey": ["id"] },
            { "name": "public.log_2024",
              "columns": [
                { "name": "id", "type": "Integer" },
                { "name": "ts", "type": "DateTimeTz" }
              ],
              "primaryKey": ["id"] }
          ]
        }"#;
        let d = decisiones(&[("familia/public.log", Respuesta::Palabra("ts".into()))]);
        let i = inducir_con(
            &Catalogo::leer(CAT).unwrap(),
            "ventas",
            &d,
            &Vocabulario::default(),
        );
        let p = i
            .pendientes
            .iter()
            .find(|p| p.clase == Clase::Familia)
            .expect("se tragó una respuesta que no podía honrar");
        assert!(p.que.contains("ya no se puede escribir"), "{}", p.que);
        // Y dice POR QUÉ, que es lo que separa una limitación de un capricho:
        // el vocabulario no tiene junta, y no por olvido.
        assert!(p.porque.contains("junta"), "{}", p.porque);
        // Las dos hermanas siguen emitiéndose por separado: la familia se ve,
        // y no unirla no es perderla.
        for e in [
            "public/entities/Log_2023.yaml",
            "public/entities/Log_2024.yaml",
        ] {
            assert!(i.ficheros.contains_key(e), "{:?}", i.ficheros.keys());
        }
    }

    /// Acuñar un concepto lo ESCRIBE: `is` exige que exista —`OOS2001`— y dejar
    /// la referencia colgando sería peor que no preguntar.
    #[test]
    fn un_concepto_acunado_se_escribe_y_se_habla() {
        let i = con(&[(
            "concepto/total.Decimal",
            Respuesta::Palabra("importeTotal".into()),
        )]);
        let c = i
            .ficheros
            .get("concepts/importeTotal.yaml")
            .expect("no acuñó el concepto");
        assert!(
            c.contains("kind: Concept") && c.contains("type: Decimal"),
            "{c}"
        );
        // Y donde está el concepto NO está el tipo: el esquema lo prohíbe con un
        // `oneOf`, porque no hay orden al que apelar si la copia deja de coincidir.
        let e = &i.ficheros["rubix_demo_ventas/entities/Facturas.yaml"];
        assert!(e.contains("total: { is: ventas.importeTotal }"), "{e}");
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Concepto));
    }

    /// Una arista que alguien confirmó se emite **marcada**: quien lea esto
    /// dentro de un año tiene que poder distinguir un hecho del catálogo de una
    /// decisión de una persona.
    #[test]
    fn una_relacion_confirmada_se_emite_y_se_dice_que_la_confirmo_alguien() {
        let i = con(&[
            (
                "clave/rubix_demo_ventas.clientes",
                Respuesta::Lista(vec!["id".into()]),
            ),
            (
                "relacion/rubix_demo_ventas.facturas.id_cliente",
                Respuesta::Palabra("si".into()),
            ),
        ]);
        let f = &i.ficheros["rubix_demo_ventas/entities/Facturas.yaml"];
        assert!(f.contains("via: [id_cliente]"), "{f}");
        assert!(
            f.contains("target: ventas.rubix_demo_ventas.Clientes"),
            "{f}"
        );
        assert!(f.contains("la confirmó una persona"), "{f}");
    }

    /// Y no se emite contra un destino sin identidad de una columna: `via` se
    /// empareja posición a posición con la clave del destino, y una arista de
    /// aridad distinta tiene el mismo aspecto que una correcta.
    #[test]
    fn una_relacion_hacia_un_destino_sin_clave_no_se_emite() {
        let i = con(&[(
            "relacion/rubix_demo_ventas.facturas.id_cliente",
            Respuesta::Palabra("si".into()),
        )]);
        assert!(!i.ficheros["rubix_demo_ventas/entities/Facturas.yaml"].contains("relations"));
        assert!(
            i.pendientes
                .iter()
                .any(|p| p.clase == Clase::Relacion && p.que.contains("no se pudo emitir")),
            "aceptó una arista que no podía escribir"
        );
    }

    /// Una respuesta que no llega a ninguna pregunta no puede tener el mismo
    /// aspecto que una decisión tomada.
    #[test]
    fn una_respuesta_a_nada_se_dice() {
        let i = con(&[
            ("clave/no.existe", Respuesta::Lista(vec!["id".into()])),
            (
                "clave/rubix_demo_ventas.clientes",
                Respuesta::Lista(vec!["id".into()]),
            ),
        ]);
        assert_eq!(i.huerfanas, vec!["clave/no.existe".to_string()]);
    }

    /// Los identificadores son interfaz: se escriben a mano en un fichero de
    /// respuestas, y cambiarlos invalida los que ya existan.
    #[test]
    fn el_identificador_de_una_decision_es_estable() {
        let i = inducido();
        let ids: Vec<&str> = i.pendientes.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"colision/Pedidos"), "{ids:?}");
        assert!(ids.contains(&"clave/rubix_demo_ventas.clientes"), "{ids:?}");
        assert!(ids.contains(&"filas/rubix_demo_ventas.mov_bak"), "{ids:?}");
        assert!(ids.contains(&"concepto/total.Decimal"), "{ids:?}");
        assert!(
            ids.contains(&"relacion/rubix_demo_ventas.facturas.id_cliente"),
            "{ids:?}"
        );
    }

    const REPETIDA: &str = r#"{
      "source": "pg",
      "tables": [
        { "name": "public.clientes",
          "columns": [
            { "name": "id", "type": "Integer", "required": true },
            { "name": "email", "type": "String" }
          ],
          "primaryKey": ["id"] },
        { "name": "public.pedidos",
          "columns": [
            { "name": "id_pedido", "type": "Integer", "required": true },
            { "name": "email", "type": "String" }
          ],
          "primaryKey": ["id_pedido"] }
      ]
    }"#;

    fn vocabulario(conceptos: &[(&str, &str, &[&str])], reticulo: Option<&str>) -> Vocabulario {
        Vocabulario {
            conceptos: conceptos
                .iter()
                .map(|(q, t, etiquetas)| crate::vocabulario::Concepto {
                    qname: (*q).to_string(),
                    tipo: (*t).to_string(),
                    etiquetas: etiquetas.iter().map(|e| (*e).to_string()).collect(),
                    sinonimos: vec!["correo".into()],
                })
                .collect(),
            reticulos: reticulo
                .map(|q| crate::vocabulario::Reticulo {
                    qname: q.to_string(),
                    niveles: ["none", "low", "medium", "high", "critical"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                })
                .into_iter()
                .collect(),
        }
    }

    fn con_voc(respuestas: &[(&str, Respuesta)], voc: &Vocabulario) -> Induccion {
        inducir_con(
            &Catalogo::leer(REPETIDA).unwrap(),
            "ventas",
            &decisiones(respuestas),
            voc,
        )
    }

    /// La pregunta deja de ser «invéntale un nombre» en cuanto hay vocabulario
    /// publicado, y enseña **qué clasificación se hereda** al elegirlo: es la
    /// diferencia entre elegir a ciegas y elegir.
    #[test]
    fn la_septima_pregunta_ofrece_lo_publicado() {
        let voc = vocabulario(
            &[("gdpr.personalEmail", "String", &["gdpr.sensitivity: high"])],
            Some("gdpr.sensitivity"),
        );
        let i = con_voc(&[], &voc);
        let p = i
            .pendientes
            .iter()
            .find(|p| p.clase == Clase::Concepto)
            .expect("no preguntó por la columna repetida");
        assert!(
            p.opciones
                .iter()
                .any(|o| o.contains("gdpr.personalEmail") && o.contains("high")),
            "{:?}",
            p.opciones
        );
    }

    /// Apuntar a un concepto que ya existe **no escribe nada**: acuñar una copia
    /// de algo publicado es la inflación por la otra puerta.
    #[test]
    fn apuntar_a_lo_publicado_no_acuna_una_copia() {
        let voc = vocabulario(
            &[("gdpr.personalEmail", "String", &["gdpr.sensitivity: high"])],
            Some("gdpr.sensitivity"),
        );
        let i = con_voc(
            &[(
                "concepto/email.String",
                Respuesta::Palabra("gdpr.personalEmail".into()),
            )],
            &voc,
        );
        assert!(
            !i.ficheros.keys().any(|k| k.starts_with("concepts/")),
            "acuñó una copia de un concepto que ya existía"
        );
        assert!(i.ficheros["public/entities/Clientes.yaml"].contains("is: gdpr.personalEmail"));
        // Y no pregunta su clasificación: la decidió quien publicó el
        // vocabulario, y reabrirla sería reabrir una decisión ajena.
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Clasificacion));
    }

    /// `is` no redeclara el tipo: lo toma del concepto. Apuntar a uno de otro
    /// tipo retiparía la columna sin decirlo.
    #[test]
    fn un_concepto_de_otro_tipo_no_se_acepta() {
        let voc = vocabulario(&[("gdpr.edad", "Integer", &[])], Some("gdpr.sensitivity"));
        let i = con_voc(
            &[(
                "concepto/email.String",
                Respuesta::Palabra("gdpr.edad".into()),
            )],
            &voc,
        );
        let p = i
            .pendientes
            .iter()
            .find(|p| p.clase == Clase::Concepto)
            .expect("se tragó un concepto de otro tipo");
        assert!(p.que.contains("Integer"), "{}", p.que);
        assert!(!i.ficheros["public/entities/Clientes.yaml"].contains("is:"));
    }

    /// Acuñar abre la pregunta que faltaba. Un concepto sin clasificación no
    /// gobierna nada: la columna que lo habla sale servida en la superficie
    /// emitida igual que si nadie hubiera contestado.
    #[test]
    fn acunar_abre_la_pregunta_de_la_clasificacion() {
        let voc = vocabulario(&[], Some("gdpr.sensitivity"));
        let i = con_voc(
            &[(
                "concepto/email.String",
                Respuesta::Palabra("correoPersonal".into()),
            )],
            &voc,
        );
        let p = i
            .pendientes
            .iter()
            .find(|p| p.clase == Clase::Clasificacion)
            .expect("acuñó un concepto y no preguntó cómo se clasifica");
        assert_eq!(p.id, "clasificacion/ventas.correoPersonal");
        assert!(p.opciones.contains(&"gdpr.sensitivity: high".to_string()));
        assert!(p.opciones.contains(&"sin_clasificar".to_string()));
        // Y mientras tanto el documento dice en voz alta que no gobierna nada.
        let c = &i.ficheros["concepts/correoPersonal.yaml"];
        assert!(c.contains("# Sin clasificar"), "{c}");
    }

    /// Contestada, la etiqueta se escribe. Se admite el texto EXACTO que la cola
    /// ofrece en `options`, porque ofrecer una opción que no se acepta al
    /// contestarla es peor que no ofrecerla.
    #[test]
    fn la_clasificacion_contestada_se_escribe() {
        let voc = vocabulario(&[], Some("gdpr.sensitivity"));
        let i = con_voc(
            &[
                (
                    "concepto/email.String",
                    Respuesta::Palabra("correoPersonal".into()),
                ),
                (
                    "clasificacion/ventas.correoPersonal",
                    Respuesta::Palabra("gdpr.sensitivity: high".into()),
                ),
            ],
            &voc,
        );
        let c = &i.ficheros["concepts/correoPersonal.yaml"];
        assert!(c.contains("labels: { gdpr.sensitivity: high }"), "{c}");
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Clasificacion));
    }

    /// Y `sin_clasificar` es una respuesta legítima —`legalName` no es
    /// sensible—, pero **hay que darla**: lo que no vale es que lo decida el
    /// silencio.
    #[test]
    fn sin_clasificar_es_una_respuesta_y_no_un_silencio() {
        let voc = vocabulario(&[], Some("gdpr.sensitivity"));
        let i = con_voc(
            &[
                (
                    "concepto/email.String",
                    Respuesta::Palabra("correoPersonal".into()),
                ),
                (
                    "clasificacion/ventas.correoPersonal",
                    Respuesta::Palabra("sin_clasificar".into()),
                ),
            ],
            &voc,
        );
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Clasificacion));
        assert!(i.ficheros["concepts/correoPersonal.yaml"].contains("# Sin clasificar"));
    }

    /// Sin un retículo no hay con qué clasificar, y preguntarlo sería pedir que
    /// se elija de una lista vacía. Es la decisión que `ore init` ya marca: sin
    /// escala no hay nada que gobernar.
    #[test]
    fn sin_reticulo_no_se_pregunta_la_clasificacion() {
        let voc = vocabulario(&[], None);
        let i = con_voc(
            &[(
                "concepto/email.String",
                Respuesta::Palabra("correoPersonal".into()),
            )],
            &voc,
        );
        assert!(i.ficheros.contains_key("concepts/correoPersonal.yaml"));
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Clasificacion));
    }

    /// El fichero de respuestas es **interfaz**: lo escribe `review` y lo puede
    /// escribir una persona. Lo que sale de una revisión tiene que poder entrar
    /// en la siguiente, o revisar en dos sentadas no es reproducible.
    #[test]
    fn lo_que_escribe_una_revision_lo_lee_la_siguiente() {
        let d = decisiones(&[
            (
                "clave/public.log",
                Respuesta::Lista(vec!["id".into(), "ts".into()]),
            ),
            // Una coma dentro de un escalar es un separador donde no debe: es el
            // error que `OOS1005` sacó de un documento que nadie escribió a mano.
            (
                "tipo/public.p.importe",
                Respuesta::Palabra("Money<EUR, 2>".into()),
            ),
        ]);
        let ida = d.json().pretty();
        let vuelta = Decisiones::leer(&ida).expect("no se pudo releer lo escrito");
        assert_eq!(
            vuelta
                .de("tipo/public.p.importe")
                .and_then(Respuesta::palabra),
            Some("Money<EUR, 2>")
        );
        assert_eq!(vuelta.json().pretty(), ida, "no es estable entre pasadas");
    }

    /// Y un fichero escrito a mano en YAML vale igual: JSON es un subconjunto.
    #[test]
    fn un_fichero_de_respuestas_escrito_a_mano_se_lee() {
        let d = Decisiones::leer(
            "answers:
  \"clave/public.log\": [id]
  vista/public.v: omitir
",
        )
        .expect("no leyó un fichero escrito a mano");
        assert!(d.omite("vista/public.v"));
        assert_eq!(
            d.de("clave/public.log"),
            Some(&Respuesta::Lista(vec!["id".to_string()]))
        );
    }

    #[test]
    fn los_nombres_no_se_singularizan_ni_se_inventan() {
        assert_eq!(entidad("rubix_demo_ventas.pedidos"), "Pedidos");
        assert_eq!(entidad("public.tb_order"), "Tb_order");
        assert_eq!(identificador("2024_total"), "t_2024_total");
        assert_eq!(identificador("first.name"), "first_name");
    }
}

#[cfg(test)]
mod emisor {
    use super::*;

    fn tipos(xs: &[(&str, &str)]) -> BTreeMap<String, String> {
        xs.iter()
            .map(|(c, t)| (c.to_string(), t.to_string()))
            .collect()
    }

    /// **La vista que el inductor propone es v1alpha14** (0040 paso 6): la
    /// consulta que traduce su forma, en DRAFT, con el contrato de los tipos
    /// que el conector tradujo y `String` donde no supo. Sin operación: ni
    /// `freshness` ni copia, que son de un `Dataset`.
    #[test]
    fn una_vista_sobre_una_tabla_sale_en_draft_y_como_consulta() {
        let s = documento_vista(
            "clientes_eu",
            "ventas",
            "default",
            "team:ventas",
            "clientes_t",
            &[
                ("id".into(), "id".into()),
                ("pais".into(), "cod_pais".into()),
            ],
            &tipos(&[("id", "Integer")]),
        );
        assert!(
            s.starts_with("apiVersion: oos.dev/v1alpha14\nkind: View\n"),
            "{s}"
        );
        assert!(s.contains("labels: { oos.maturity: DRAFT }"), "{s}");
        assert!(s.contains("  dialect: duckdb\n"), "{s}");
        assert!(
            s.contains("  sql: |\n    SELECT \"id\", \"cod_pais\" AS \"pais\"\n    FROM \"ventas\".\"clientes_t\"\n"),
            "{s}"
        );
        assert!(
            s.contains("  columns:\n    id: { type: Integer }\n    pais: { type: String }\n"),
            "{s}"
        );
        assert!(!s.contains("freshness:") && !s.contains("from:"), "{s}");
        // Y es una vista v1alpha14 que el núcleo lee: su consulta es la suya.
        let v = ore_core::link::Loaded {
            path: Path::new("v.yaml").to_path_buf(),
            kind: ore_core::document::Kind::View,
            root: parse::parse(&s).unwrap(),
        };
        assert!(ore_core::vistas::es_sql(&v));
        assert_eq!(
            ore_core::vistas::contrato(&v)
                .into_iter()
                .collect::<Vec<_>>(),
            ["id", "pais"]
        );
    }

    /// Y el dataset que copia el mismo objeto lleva la forma, que en v1alpha14
    /// sigue siendo suya: los mismos campos, en el mismo orden.
    #[test]
    fn el_dataset_lleva_la_forma_con_su_recorte() {
        let d = documento_dataset(
            "clientes_eu",
            "ventas",
            "team:ventas",
            "clientes_t",
            &[
                ("id".into(), "id".into()),
                ("pais".into(), "cod_pais".into()),
            ],
            &[
                ("borrado".into(), vec!["false".into()]),
                ("pais".into(), vec!["ES".into(), "PT".into()]),
            ],
        );
        assert!(d.contains("kind: Dataset") && !d.contains("labels"), "{d}");
        assert!(d.contains("from: { table: clientes_t }"), "{d}");
        assert!(d.contains("    id: id\n    pais: cod_pais\n"), "{d}");
        assert!(d.contains("    borrado: false\n"), "{d}");
        assert!(d.contains("    pais: [ES, PT]\n"), "{d}");
    }

    /// En un schema que no es `default`: el documento lo dice, la consulta
    /// nombra la tabla con él, y `con_schema` no la baja a v1alpha13 ni le
    /// escribe el schema dos veces.
    #[test]
    fn en_un_schema_la_consulta_lo_nombra() {
        let s = documento_vista(
            "clientes",
            "ventas",
            "espana",
            "team:ventas",
            "clientes_t",
            &[("id".into(), "id".into())],
            &BTreeMap::new(),
        );
        assert!(s.contains("  namespace: ventas\n  schema: espana\n"), "{s}");
        assert!(
            s.contains("FROM \"ventas\".\"espana\".\"clientes_t\""),
            "{s}"
        );
        assert_eq!(con_schema(s.clone(), "espana", "ventas"), s);
    }

    /// Un nombre que no es un identificador sale entrecomillado en la consulta,
    /// y la columna del contrato se llama como la propiedad.
    #[test]
    fn una_columna_con_forma_rara_se_entrecomilla() {
        let s = documento_vista(
            "v",
            "p",
            "default",
            "o",
            "t_t",
            &[("ref".into(), "Worker_Reference.ID".into())],
            &BTreeMap::new(),
        );
        assert!(
            s.contains("SELECT \"Worker_Reference.ID\" AS \"ref\""),
            "{s}"
        );
        assert!(s.contains("    ref: { type: String }\n"), "{s}");
    }

    /// La tabla de un objeto se llama `<objeto>_t`: la vista o el dataset que
    /// lo exponen se llaman como él, y en v1alpha14 un nombre es una cosa.
    #[test]
    fn la_tabla_de_un_objeto_lleva_su_sufijo() {
        let o = Objeto {
            nombre: "public.clientes".into(),
            columnas: vec![],
        };
        assert_eq!(nombre_de_tabla(&o), "clientes_t");
    }
}
