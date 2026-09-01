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
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

// ── El catálogo ─────────────────────────────────────────────────────────────

/// Una columna, ya traducida al sistema de tipos de OOS por el lector.
#[derive(Clone)]
struct Columna {
    nombre: String,
    /// `None` cuando el lector **no supo** traducir el tipo del origen. No es un
    /// hueco a rellenar: es la conjetura que este modulo no toma.
    tipo: Option<String>,
    /// Lo que dijo el origen cuando `tipo` es `None`. Se **cita**, nunca se
    /// interpreta: interpretarlo seria saber de BigQuery, y la costura existe
    /// justo para no saberlo.
    origen: Option<String>,
    obligatoria: bool,
    /// Escrita en el origen por quien conoce el dato. Es un hecho, y de los
    /// buenos: `pedidos.fecha` es un `String` cuya descripcion dice «Formato
    /// DDMMAAAA, viene del AS/400». Perderla seria perder lo mejor del catalogo.
    descripcion: Option<String>,
}

/// Una clave foranea, tal y como la declara el origen.
#[derive(Clone)]
struct Foranea {
    /// Las columnas locales, en el orden del origen.
    columnas: Vec<String>,
    /// La tabla referenciada.
    destino: String,
    /// Las columnas del DESTINO, emparejadas en orden con `columnas`. SQL no
    /// obliga a referenciar la clave primaria, y saber cuales son es lo unico
    /// que permite no emitir una relacion verde y equivocada.
    destino_columnas: Vec<String>,
}

/// Una tabla del origen. `nombre` es **opaco**: sus reglas son del sistema de
/// origen y por eso viaja tal cual al `Binding`.
#[derive(Clone)]
struct Tabla {
    nombre: String,
    columnas: Vec<Columna>,
    clave: Vec<String>,
    /// Claves alternativas declaradas por el origen. No son adorno: son lo que
    /// permite enlazar contra otra identidad —`toKey`— y lo que hace posible la
    /// resolucion determinista entre fuentes.
    unicas: Vec<Vec<String>>,
    /// `(columnas locales, tabla destino)` — solo lo que el catálogo DECLARA.
    foraneas: Vec<Foranea>,
    filas: Option<u64>,
    /// `table`, `view` o `materializedView`, tal y como lo dijo el origen.
    clase: String,
    /// Los objetos FÍSICOS que sostienen esta entidad. Casi siempre uno —ella
    /// misma—, y varios cuando una respuesta unió una familia fechada: una
    /// entidad servida desde N tablas es N bindings, que es exactamente lo que
    /// el ejecutor ya sabe federar.
    objetos: Vec<Objeto>,
}

/// Un objeto del origen y las columnas que tiene **dentro**.
///
/// Casi siempre son las de su tabla. Cuando una respuesta une una familia
/// fechada no lo son: la hermana de 2019 puede no tener la columna que se añadió
/// en 2024, y un binding que se la atribuyera sería un mapeo verde y falso.
#[derive(Clone)]
struct Objeto {
    nombre: String,
    columnas: Vec<String>,
}

/// Lo que el lector entrega.
pub struct Catalogo {
    fuente: String,
    tablas: Vec<Tabla>,
}

impl Catalogo {
    /// De qué fuente vino. Lo necesita quien tenga que comprobar que el
    /// repositorio la declara: un binding la referencia por nombre.
    pub fn fuente(&self) -> &str {
        &self.fuente
    }

    /// Lee un catálogo en JSON. Se analiza con el analizador de YAML porque
    /// **JSON es un subconjunto de YAML** y `ore-core` no lleva uno de JSON
    /// (ADR 0002).
    pub fn leer(texto: &str) -> Result<Self, String> {
        let raiz = parse::parse(texto).map_err(|e| format!("el catálogo no analiza: {e:?}"))?;
        let fuente = raiz
            .get("source")
            .and_then(|(_, v)| v.as_str())
            .ok_or("el catálogo no dice de qué `source` viene")?
            .to_string();

        let mut tablas = Vec::new();
        for t in raiz.get("tables").map(|(_, v)| v.items()).unwrap_or(&[]) {
            let Some(nombre) = t.get("name").and_then(|(_, v)| v.as_str()) else {
                continue;
            };
            let columnas: Vec<Columna> = t
                .get("columns")
                .map(|(_, v)| v.items())
                .unwrap_or(&[])
                .iter()
                .filter_map(|c| {
                    let cadena = |k: &str| {
                        c.get(k)
                            .and_then(|(_, v)| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                    };
                    Some(Columna {
                        nombre: c.get("name")?.1.as_str()?.to_string(),
                        tipo: cadena("type"),
                        origen: cadena("sourceType"),
                        obligatoria: c
                            .get("required")
                            .and_then(|(_, v)| v.as_str())
                            .is_some_and(|r| r == "true"),
                        descripcion: cadena("description"),
                    })
                })
                .collect();
            let objetos = vec![Objeto {
                nombre: nombre.to_string(),
                columnas: columnas_de(&columnas),
            }];
            tablas.push(Tabla {
                nombre: nombre.to_string(),
                columnas,
                clave: lista(t, "primaryKey"),
                unicas: t
                    .get("uniqueKeys")
                    .map(|(_, v)| v.items())
                    .unwrap_or(&[])
                    .iter()
                    .map(lista_de)
                    .filter(|k: &Vec<String>| !k.is_empty())
                    .collect(),
                foraneas: t
                    .get("foreignKeys")
                    .map(|(_, v)| v.items())
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|f| {
                        Some(Foranea {
                            columnas: lista(f, "columns"),
                            destino: f.get("references")?.1.as_str()?.to_string(),
                            destino_columnas: lista(f, "toColumns"),
                        })
                    })
                    .collect(),
                filas: t
                    .get("rows")
                    .and_then(|(_, v)| v.as_str())
                    .and_then(|s| s.parse().ok()),
                clase: t
                    .get("kind")
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("table")
                    .to_string(),
                objetos,
            });
        }
        Ok(Catalogo { fuente, tablas })
    }
}

fn columnas_de(columnas: &[Columna]) -> Vec<String> {
    columnas.iter().map(|c| c.nombre.clone()).collect()
}

fn lista_de(n: &Node) -> Vec<String> {
    n.items()
        .iter()
        .filter_map(|i| i.as_str())
        .map(String::from)
        .collect()
}

fn lista(n: &Node, clave: &str) -> Vec<String> {
    n.get(clave)
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
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

impl Clase {
    /// El prefijo del identificador de una decisión de esta clase.
    ///
    /// **Es interfaz.** La izquierda de cada línea de un fichero de respuestas
    /// sale de aquí, así que cambiarla invalida los ficheros ya escritos —
    /// igual que cambiar el nombre de una opción de la línea de órdenes.
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
pub fn inducir_con(
    cat: &Catalogo,
    paquete: &str,
    dec: &Decisiones,
    voc: &Vocabulario,
) -> Induccion {
    let mut ficheros = BTreeMap::new();
    let mut pendientes = Vec::new();

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
                "el origen no la declara. `01-package` §5: NO DEBE inferirse — \
                 sin clave no hay identidad, y una identidad inventada es peor \
                 que ninguna",
                t.columnas.iter().map(|c| c.nombre.clone()).collect(),
            ));
        }

        let (extra, pend) = relaciones_decididas(t, &nombres, &claves, dec);
        pendientes.extend(pend);

        ficheros.insert(
            format!("entities/{nombre}.yaml"),
            entidad_yaml(nombre, paquete, t, &claves, &mapeo, &extra),
        );
        for objeto in &t.objetos {
            ficheros.insert(
                format!("bindings/{}.yaml", identificador(&objeto.nombre)),
                binding_yaml(nombre, paquete, &cat.fuente, t, objeto),
            );
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
    }

    let (package_yaml, pend) = paquete_yaml(paquete, dec);
    ficheros.insert("package.yaml".into(), package_yaml);
    pendientes.extend(pend);
    pendientes.extend(candidatas_a_concepto(&tablas, dec, voc));
    pendientes.extend(relaciones_no_declaradas(&tablas, &nombres, dec));

    // Lo que se contestó y no llegó a ninguna pregunta. Una errata en un
    // identificador no puede tener el mismo aspecto que una decisión tomada.
    let huerfanas = huerfanas(dec, &cat.tablas, &tablas, &c.acunados);

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
    for t in antes.iter().chain(despues) {
        for c in [Clase::Clave, Clase::Vista, Clase::Filas, Clase::Vacio] {
            posibles.insert(id(c, &t.nombre));
        }
        posibles.insert(id(Clase::Colision, &entidad(&t.nombre)));
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
/// siendo su propia entidad y la pregunta se cierra. Con el nombre de una
/// columna, se **unen**: una entidad servida desde N tablas, que es N bindings —
/// exactamente lo que el ejecutor ya sabe federar, y por eso se puede escribir
/// sin inventar nada.
fn familias(tablas: &[Tabla], dec: &Decisiones) -> (Vec<Tabla>, Vec<Pendiente>) {
    let mut familia: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, t) in tablas.iter().enumerate() {
        familia.entry(raiz(&t.nombre)).or_default().push(i);
    }
    // Una familia exige al menos una hermana CON sufijo: dos tablas cuya raíz
    // coincide sin que ninguna esté numerada no son una familia, son dos tablas.
    familia.retain(|r, v| v.len() > 1 && v.iter().any(|i| tablas[*i].nombre != *r));

    let mut fuera: Vec<usize> = Vec::new();
    let mut nuevas: Vec<Tabla> = Vec::new();
    let mut pendientes = Vec::new();

    for (r, miembros) in &familia {
        let sujeto = miembros
            .iter()
            .map(|i| tablas[*i].nombre.clone())
            .collect::<Vec<_>>()
            .join(" · ");
        let respuesta = dec.de(&id(Clase::Familia, r));
        let columnas: Vec<String> = tablas[miembros[0]]
            .columnas
            .iter()
            .map(|c| c.nombre.clone())
            .collect();
        match respuesta {
            None => pendientes.push(pendiente(
                Clase::Familia,
                r,
                sujeto,
                format!("¿una sola entidad `{}` con eje temporal?", entidad(r)),
                "el sufijo numérico es el patrón de una tabla fragmentada por fecha. \
                 Unirlas exige nombrar la columna de tiempo, y eso no está en el catálogo",
                {
                    let mut o = vec!["separadas".to_string(), OMITIR.to_string()];
                    o.extend(columnas);
                    o
                },
            )),
            Some(x) if x.es("separadas") => {}
            Some(x) if x.omite() => fuera.extend(miembros.iter().copied()),
            Some(x) => {
                let Some(eje) = x.palabra() else { continue };
                match unir(tablas, miembros, r, eje) {
                    Ok(t) => {
                        fuera.extend(miembros.iter().copied());
                        nuevas.push(t);
                    }
                    // La respuesta no se puede honrar sin inventar, así que la
                    // pregunta sigue abierta y dice por qué. Callarlo dejaría a
                    // alguien creyendo que unió algo.
                    Err(motivo) => pendientes.push(pendiente(
                        Clase::Familia,
                        r,
                        sujeto,
                        format!("no se pudo unir por `{eje}`"),
                        motivo,
                        {
                            let mut o = vec!["separadas".to_string(), OMITIR.to_string()];
                            o.extend(columnas);
                            o
                        },
                    )),
                }
            }
        }
    }

    let mut out: Vec<Tabla> = tablas
        .iter()
        .enumerate()
        .filter(|(i, _)| !fuera.contains(i))
        .map(|(_, t)| t.clone())
        .collect();
    out.extend(nuevas);
    (out, pendientes)
}

/// Une las hermanas de una familia en una tabla sola.
///
/// Se niega en los dos casos donde unir exigiría decidir algo que nadie dijo: si
/// el eje no está en todas —la unión tendría filas sin fecha— o si no comparten
/// clave primaria —la identidad de la unión no sería la de ninguna—.
fn unir(tablas: &[Tabla], miembros: &[usize], raiz: &str, eje: &str) -> Result<Tabla, String> {
    let faltan: Vec<&str> = miembros
        .iter()
        .map(|i| &tablas[*i])
        .filter(|t| !t.columnas.iter().any(|c| c.nombre == eje))
        .map(|t| t.nombre.as_str())
        .collect();
    if !faltan.is_empty() {
        return Err(format!(
            "`{eje}` no está en {}. Una unión por un eje que la mitad no tiene deja \
             filas sin sitio en el tiempo, y ponerlas en alguno sería inventarlo",
            faltan.join(", ")
        ));
    }
    let primera = &tablas[miembros[0]];
    if miembros.iter().any(|i| tablas[*i].clave != primera.clave) {
        return Err(
            "las hermanas no declaran la misma clave primaria. La identidad de la unión \
             no puede ser la de una de ellas, y elegirla sería decidir cuál manda"
                .into(),
        );
    }

    // Las columnas, en el orden de la primera hermana y con lo que las demás
    // añadan detrás. Perder una columna porque una hermana vieja no la tiene
    // sería perder un hecho del origen.
    let mut columnas: Vec<Columna> = primera.columnas.clone();
    for i in &miembros[1..] {
        for c in &tablas[*i].columnas {
            if !columnas.iter().any(|x| x.nombre == c.nombre) {
                columnas.push(c.clone());
            }
        }
    }
    let filas = miembros
        .iter()
        .map(|i| tablas[*i].filas)
        .try_fold(0u64, |a, f| f.map(|n| a + n));

    Ok(Tabla {
        nombre: raiz.to_string(),
        columnas,
        clave: primera.clave.clone(),
        unicas: primera.unicas.clone(),
        foraneas: primera.foraneas.clone(),
        filas,
        clase: primera.clase.clone(),
        objetos: miembros
            .iter()
            .flat_map(|i| tablas[*i].objetos.clone())
            .collect(),
    })
}

/// El identificador de una candidata a concepto. Lleva el tipo dentro porque
/// `email: String` y `email: Integer` no son la misma pregunta.
fn concepto_id(columna: &str, tipo: Option<&str>) -> String {
    format!("{columna}.{}", identificador(tipo.unwrap_or("?")))
}

/// El nombre de entidad de cada tabla, con las colisiones resueltas donde lo
/// estén. Una tabla ausente del mapa es una que **no se emite**.
fn nombres(tablas: &[Tabla], dec: &Decisiones) -> (BTreeMap<String, String>, Vec<Pendiente>) {
    let mut por_nombre: BTreeMap<String, Vec<&Tabla>> = BTreeMap::new();
    for t in tablas {
        por_nombre.entry(entidad(&t.nombre)).or_default().push(t);
    }

    let mut out = BTreeMap::new();
    let mut pendientes = Vec::new();
    for (nombre, grupo) in &por_nombre {
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
            out.insert(t.nombre.clone(), nombre.clone());
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
                        out.insert(elegida.nombre.clone(), nombre.clone());
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
    donde.retain(|(n, _), v| v.len() > 1 && !estructural(n));
    donde
}

/// Un nombre de columna que se repite en varias tablas **con el mismo tipo** es
/// una candidata a concepto. No se acuña: se muestran juntas para que la
/// unificación se decida una vez.
fn candidatas_a_concepto(tablas: &[Tabla], dec: &Decisiones, voc: &Vocabulario) -> Vec<Pendiente> {
    repetidas(tablas)
        .into_iter()
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
                "acuñar uno por columna repetida es la inflación que produce cuatro mil \
                 conceptos. Decidirlo una vez vale por todas las apariciones — y apuntar a \
                 uno publicado hereda su clasificación sin escribirla"
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
fn paquete_yaml(paquete: &str, dec: &Decisiones) -> (String, Vec<Pendiente>) {
    let respuesta = dec
        .de(&id(Clase::Dueno, paquete))
        .and_then(Respuesta::palabra)
        .filter(|h| handle(h));
    let owner = respuesta.unwrap_or("cambiame");
    let doc = format!(
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Package\n\
         metadata: {{ name: {paquete}, version: 0.1.0, status: active, domain: {paquete} }}\n\
         spec: {{ owner: \"{owner}\" }}\n"
    );
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

fn entidad_yaml(
    nombre: &str,
    paquete: &str,
    t: &Tabla,
    claves: &BTreeMap<String, Vec<String>>,
    mapeo: &BTreeMap<String, String>,
    extra: &[(String, String)],
) -> String {
    let clave = claves.get(&t.nombre).cloned().unwrap_or_default();
    let mut s = String::new();
    let _ = write!(
        s,
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Entity\n\
         metadata:\n  \
           name: {nombre}\n  \
           namespace: {paquete}\n  \
           labels: {{ oos.maturity: DRAFT }}\n\
         spec:\n  \
           nature: entity\n"
    );
    if t.objetos.len() > 1 {
        // Una familia unida. Se dice DÓNDE está el eje y **no** se escribe un
        // `temporal`: historiar el dato es una decisión de gobierno con su
        // propia forma, y derivarla de que las tablas lleven fecha en el nombre
        // sería exactamente inventarla.
        let _ = writeln!(
            s,
            "  # Une {} objetos del origen, uno por binding. El eje temporal viene de",
            t.objetos.len()
        );
        s.push_str("  # su nombre; `spec.temporal` es otra decisión y no se deriva de aquí.\n");
    }
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
    if !t.foraneas.is_empty() || !extra.is_empty() {
        s.push_str("  relations:\n");
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
            let _ = write!(
                s,
                "    {}:\n      target: {paquete}.{}\n      cardinality: many_to_one\n      via: [{}]\n{to_key}      required: {obligatoria}\n",
                identificador(&entidad(&f.destino)).to_lowercase(),
                entidad(&f.destino),
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
            let _ = write!(
                s,
                "    # No la declara el origen: la confirmó una persona al revisar.\n    \
                 {}:\n      target: {paquete}.{destino}\n      \
                 cardinality: many_to_one\n      via: [{}]\n      \
                 required: {obligatoria}\n",
                destino.to_lowercase(),
                identificador(columna)
            );
        }
    }
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
         kind: Property\n\
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

fn binding_yaml(nombre: &str, paquete: &str, fuente: &str, t: &Tabla, objeto: &Objeto) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Binding\n\
         metadata: {{ name: {}, namespace: {paquete} }}\n\
         spec:\n  \
           targetEntity: {paquete}.{nombre}\n  \
           datasourceRef: {fuente}\n  \
           source: \"{}\"\n  \
           properties:\n",
        identificador(&objeto.nombre),
        objeto.nombre
    );
    for c in t
        .columnas
        .iter()
        .filter(|c| objeto.columnas.contains(&c.nombre))
    {
        let _ = writeln!(s, "    {}: \"{}\"", identificador(&c.nombre), c.nombre);
    }
    s
}

// ── Nombres ─────────────────────────────────────────────────────────────────

/// El nombre de entidad de una tabla. Se toma el último segmento —el esquema y
/// el catálogo son del origen, no del dominio— y se capitaliza. **No se
/// singulariza**: eso sería adivinar un idioma.
fn entidad(tabla: &str) -> String {
    let ultimo = tabla.rsplit(['.', '/']).next().unwrap_or(tabla);
    capitalizar(&identificador(ultimo))
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
/// inventar un carácter — por eso queda anotado en el binding, donde el nombre
/// físico sigue entero.
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
    let entidades = ind
        .ficheros
        .keys()
        .filter(|k| k.starts_with("entities/"))
        .count();
    let mut s = format!(
        "  ✓ {entidades} entidades y sus bindings en {}\n\
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
        assert!(i.ficheros.contains_key("entities/Facturas.yaml"));
        assert!(
            i.ficheros
                .contains_key("bindings/rubix_demo_ventas_facturas.yaml")
        );
        // El nombre físico viaja ENTERO al binding: es opaco y es del origen.
        let b = &i.ficheros["bindings/rubix_demo_ventas_facturas.yaml"];
        assert!(b.contains(r#"source: "rubix_demo_ventas.facturas""#), "{b}");
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
        let e = &i.ficheros["entities/Clientes.yaml"];
        assert!(!e.contains("primaryKey"), "se inventó una clave:\n{e}");
    }

    #[test]
    fn las_fragmentadas_por_fecha_se_ven_juntas() {
        let i = inducido();
        let p = i
            .pendientes
            .iter()
            .find(|p| p.que.contains("eje temporal"))
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
        let f = &i.ficheros["entities/Facturas.yaml"];
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
        let f = &i.ficheros["entities/Facturas.yaml"];
        assert!(f.contains("via: [nif_cliente]"), "{f}");
        assert!(
            f.contains("toKey: [nif]"),
            "no dijo contra qué clave enlaza:\n{f}"
        );
        // Y el destino tiene que DECLARAR esa clave, o `toKey` apunta a algo que
        // no identifica. El lector trae las UNIQUE del origen justo para esto:
        // sin ellas la cadena emitía un `toKey` que su propio destino no
        // sostenía, y salía OOS3006 sobre un documento que nadie escribió.
        let c = &i.ficheros["entities/Clientes.yaml"];
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
            !i.ficheros["entities/Facturas.yaml"].contains("relations"),
            "emitió una arista que nadie declaró"
        );
    }

    #[test]
    fn una_tabla_vacia_no_se_borra_sola() {
        let i = inducido();
        assert!(i.pendientes.iter().any(|p| p.sujeto.contains("mov_bak")));
        assert!(i.ficheros.contains_key("entities/Mov_bak.yaml"));
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
            i.ficheros["entities/Clientes.yaml"].contains("primaryKey: [id]"),
            "{}",
            i.ficheros["entities/Clientes.yaml"]
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
        assert!(!i.ficheros["entities/Clientes.yaml"].contains("primaryKey"));
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
        assert!(i.ficheros.contains_key("entities/Pedidos.yaml"));
        assert!(i.ficheros.contains_key("entities/PedidosViejos.yaml"));
        assert!(!i.pendientes.iter().any(|p| p.clase == Clase::Colision));
        // Y cada una conserva su nombre físico, que es del origen y es opaco.
        let b = &i.ficheros["bindings/rubix_demo_ventas_Pedidos.yaml"];
        assert!(b.contains("targetEntity: ventas.PedidosViejos"), "{b}");
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

    /// Unir es una entidad servida desde N tablas, que es N bindings — lo que el
    /// ejecutor ya sabe federar. Y cada binding mapea SOLO las columnas que su
    /// objeto tiene: atribuirle a la hermana de 2019 una columna de 2024 sería un
    /// mapeo verde y falso.
    #[test]
    fn unir_una_familia_da_una_entidad_y_un_binding_por_hermana() {
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
        let d = decisiones(&[("familia/public.pedidos", Respuesta::Palabra("fecha".into()))]);
        let i = inducir_con(
            &Catalogo::leer(CAT).unwrap(),
            "ventas",
            &d,
            &Vocabulario::default(),
        );
        assert!(
            i.ficheros.contains_key("entities/Pedidos.yaml"),
            "{:?}",
            i.ficheros.keys()
        );
        assert!(i.ficheros.contains_key("bindings/public_pedidos_2023.yaml"));
        assert!(i.ficheros.contains_key("bindings/public_pedidos_2024.yaml"));
        let viejo = &i.ficheros["bindings/public_pedidos_2023.yaml"];
        assert!(
            !viejo.contains("canal"),
            "atribuyó una columna que no está ahí:\n{viejo}"
        );
        assert!(i.ficheros["bindings/public_pedidos_2024.yaml"].contains("canal"));
        // La unión conserva la columna nueva: perderla sería perder un hecho.
        assert!(i.ficheros["entities/Pedidos.yaml"].contains("canal"));
    }

    /// Un eje que la mitad de las hermanas no tiene deja filas sin sitio en el
    /// tiempo. La respuesta no se puede honrar, y callarlo dejaría a alguien
    /// creyendo que unió algo.
    #[test]
    fn no_une_por_un_eje_que_no_esta_en_todas() {
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
        assert!(p.porque.contains("public.log_2023"), "{}", p.porque);
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
            c.contains("kind: Property") && c.contains("type: Decimal"),
            "{c}"
        );
        // Y donde está el concepto NO está el tipo: el esquema lo prohíbe con un
        // `oneOf`, porque no hay orden al que apelar si la copia deja de coincidir.
        let e = &i.ficheros["entities/Facturas.yaml"];
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
        let f = &i.ficheros["entities/Facturas.yaml"];
        assert!(f.contains("via: [id_cliente]"), "{f}");
        assert!(f.contains("target: ventas.Clientes"), "{f}");
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
        assert!(!i.ficheros["entities/Facturas.yaml"].contains("relations"));
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
        assert!(i.ficheros["entities/Clientes.yaml"].contains("is: gdpr.personalEmail"));
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
        assert!(!i.ficheros["entities/Clientes.yaml"].contains("is:"));
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
