//! **La forma del catálogo**: lo que un lector emite y lo que `discover` acepta.
//!
//! # Por qué esto sube aquí
//!
//! Vivía entera dentro de `ore-cli`, en el `struct Catalogo` del inductor y su
//! `leer`. Y eso significaba que **la forma existía en un solo sitio y era el
//! consumidor**: los cuatro productores —la receta de BigQuery, los dos drivers
//! y el catálogo escrito a mano de las pruebas— son programas aparte que
//! escribían JSON a pelo, así que lo que compartían no era un tipo, era **haber
//! leído el mismo fichero**.
//!
//! Es exactamente el argumento por el que este crate existe. La cabecera de
//! `lib.rs` lo dice de la petición —*«un contrato repetido en cada
//! implementación es un contrato que diverge en la tercera»*— y de los cuatro
//! verbos del protocolo, la salida de `catalogo` es la mayor y la única que se
//! había quedado fuera.
//!
//! # Qué cuesta que no esté declarada
//!
//! No que los productores divergan: divergen a propósito y está bien. Un
//! fichero `.ndjson` no tiene claves foráneas y BigQuery no publica
//! `uniqueKeys`; la ausencia es una respuesta y el lector la trata como tal
//! (P4). Medido, cinco de ellas las emite un solo productor.
//!
//! Lo que cuesta es que **nada diga cuál es la lista**, y por eso un productor
//! nuevo no puede saber qué se está dejando. Se entera el día que una inducción
//! salga más pobre y nadie sepa por qué, porque **una tabla sin `primaryKey` y
//! una tabla cuyo driver se olvidó de emitirlo se ven exactamente igual**.
//!
//! [`FORMA`] es esa lista, y su censo la ata al lector: una clave que se lea y
//! no esté declarada **no compila la suite**.
//!
//! # Las dos caras viajan opacas, y es a propósito
//!
//! `reads` y `changes` se guardan como [`Json`] y no se interpretan aquí. Qué
//! se le puede pedir a un origen lo sabe quien traduce las consultas, y esta
//! pieza no es esa: si lo supiera, el vocabulario viviría en dos sitios.

use ore_core::json::Json;
use ore_core::parse::{self, Node, Style};
use std::collections::BTreeMap;

/// Dónde cuelga cada clave. Son cuatro niveles y no hay más anidamiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nivel {
    Raiz,
    Tabla,
    Columna,
    Foranea,
}

/// **El vocabulario del catálogo, entero.** Dieciocho claves en cuatro
/// niveles —`name` es la única que sale en dos, y `columns` también—.
///
/// Esta es la declaración que no existía en ningún sitio: ni un esquema en
/// `vendor/oos/schemas`, ni un tipo aquí, ni prosa. Su censo —abajo— comprueba
/// que el lector no lea ninguna que no esté.
pub const FORMA: &[(&str, Nivel, &str)] = &[
    ("source", Nivel::Raiz, "de qué fuente declarada vino"),
    ("tables", Nivel::Raiz, "los objetos que el origen tiene"),
    ("name", Nivel::Tabla, "opaco: sus reglas son del origen"),
    (
        "columns",
        Nivel::Tabla,
        "las columnas, en el orden del origen",
    ),
    ("kind", Nivel::Tabla, "`table`, `view` o `materializedView`"),
    (
        "primaryKey",
        Nivel::Tabla,
        "la clave, si el origen la declara",
    ),
    (
        "uniqueKeys",
        Nivel::Tabla,
        "las alternativas: lo que permite `toKey`",
    ),
    (
        "foreignKeys",
        Nivel::Tabla,
        "solo lo que el catálogo DECLARA",
    ),
    (
        "rows",
        Nivel::Tabla,
        "cuántas filas, si el origen las cuenta",
    ),
    ("reads", Nivel::Tabla, "la cara `I`, opaca a esta pieza"),
    ("changes", Nivel::Tabla, "la cara `D`, opaca a esta pieza"),
    (
        "type",
        Nivel::Columna,
        "ya traducido al sistema de tipos de OOS",
    ),
    (
        "sourceType",
        Nivel::Columna,
        "lo que dijo el origen cuando no se supo traducir. Se cita",
    ),
    (
        "required",
        Nivel::Columna,
        "si el origen la declara obligatoria",
    ),
    (
        "description",
        Nivel::Columna,
        "escrita en el origen por quien conoce el dato. Es un hecho",
    ),
    (
        // Lo emite `ore-read-postgres` y HOY NO LO LEE NADIE: el inductor no
        // tiene campo para él. Está aquí porque la forma tiene que poder
        // llevar lo que un productor dice —si no, capturar un catálogo lo
        // perdería— y porque declararlo es lo que hace visible que falta su
        // lector, en vez de que se caiga en silencio.
        "enum",
        Nivel::Columna,
        "los valores declarados, en el orden de declaración del origen",
    ),
    ("references", Nivel::Foranea, "la tabla referenciada"),
    (
        "toColumns",
        Nivel::Foranea,
        "las columnas del DESTINO: SQL no obliga a referenciar la clave",
    ),
];

/// Una columna, ya traducida al sistema de tipos de OOS por el lector.
#[derive(Clone, Debug, Default)]
pub struct Columna {
    pub nombre: String,
    /// `None` cuando el lector **no supo** traducir el tipo del origen. No es un
    /// hueco a rellenar: es la conjetura que esta pieza no toma.
    pub tipo: Option<String>,
    /// Lo que dijo el origen cuando `tipo` es `None`. Se **cita**, nunca se
    /// interpreta: interpretarlo sería saber de BigQuery, y la costura existe
    /// justo para no saberlo.
    pub origen: Option<String>,
    pub obligatoria: bool,
    /// Los valores que el origen declara, **en su orden de declaración**: el
    /// esquema dice que reordenarlos es un cambio observable.
    pub valores: Vec<String>,
    /// Escrita en el origen por quien conoce el dato. Es un hecho, y de los
    /// buenos: `pedidos.fecha` es un `String` cuya descripción dice «Formato
    /// DDMMAAAA, viene del AS/400». Perderla sería perder lo mejor del catálogo.
    pub descripcion: Option<String>,
}

/// Una clave foránea, tal y como la declara el origen.
#[derive(Clone, Debug, Default)]
pub struct Foranea {
    /// Las columnas locales, en el orden del origen.
    pub columnas: Vec<String>,
    /// La tabla referenciada.
    pub destino: String,
    /// Las columnas del DESTINO, emparejadas en orden con `columnas`. SQL no
    /// obliga a referenciar la clave primaria, y saber cuáles son es lo único
    /// que permite no emitir una relación verde y equivocada.
    pub destino_columnas: Vec<String>,
}

/// Una tabla del origen. `nombre` es **opaco**: sus reglas son del sistema de
/// origen y por eso viaja tal cual.
#[derive(Clone, Debug, Default)]
pub struct Tabla {
    pub nombre: String,
    pub columnas: Vec<Columna>,
    pub clave: Vec<String>,
    /// Claves alternativas declaradas por el origen. No son adorno: son lo que
    /// permite enlazar contra otra identidad —`toKey`— y lo que hace posible la
    /// resolución determinista entre fuentes.
    pub unicas: Vec<Vec<String>>,
    pub foraneas: Vec<Foranea>,
    pub filas: Option<u64>,
    /// `table`, `view` o `materializedView`, tal y como lo dijo el origen.
    pub clase: String,
    /// La cara `I` del objeto, **tal como la declaró el driver**: se transcribe,
    /// no se interpreta. Ver la cabecera.
    pub lee: Option<Json>,
    /// La cara `D`, igual: la sondeó el driver preguntándole al servidor.
    pub cambia: Option<Json>,
}

/// Lo que el lector entrega.
#[derive(Clone, Debug, Default)]
pub struct Catalogo {
    pub fuente: String,
    pub tablas: Vec<Tabla>,
}

impl Catalogo {
    /// De qué fuente vino. Lo necesita quien tenga que comprobar que el
    /// repositorio la declara.
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
                        valores: lista(c, "enum"),
                        descripcion: cadena("description"),
                    })
                })
                .collect();
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
                lee: t.get("reads").map(|(_, v)| de_node(v)),
                cambia: t.get("changes").map(|(_, v)| de_node(v)),
            });
        }
        Ok(Catalogo { fuente, tablas })
    }
}

/// **El emisor.** El único, y por lo mismo que el de las vistas: un catálogo
/// escrito por un driver y uno escrito por otro tienen que ser el mismo texto,
/// o hay cuatro emisores y divergen en el caso que ninguna prueba ejerce.
///
/// Sale ordenado y legible porque un catálogo capturado se lee y se **difunde
/// en un diff**: ese diff es la detección de deriva de un origen.
pub fn escribir(c: &Catalogo) -> String {
    let tablas: Vec<Json> = c
        .tablas
        .iter()
        .map(|t| {
            let mut o: BTreeMap<String, Json> = BTreeMap::new();
            o.insert("name".into(), Json::s(&t.nombre));
            o.insert("kind".into(), Json::s(&t.clase));
            o.insert(
                "columns".into(),
                Json::Arr(
                    t.columnas
                        .iter()
                        .map(|c| {
                            let mut m: BTreeMap<String, Json> = BTreeMap::new();
                            m.insert("name".into(), Json::s(&c.nombre));
                            if let Some(x) = &c.tipo {
                                m.insert("type".into(), Json::s(x));
                            }
                            if let Some(x) = &c.origen {
                                m.insert("sourceType".into(), Json::s(x));
                            }
                            if c.obligatoria {
                                m.insert("required".into(), Json::Bool(true));
                            }
                            if let Some(x) = &c.descripcion {
                                m.insert("description".into(), Json::s(x));
                            }
                            if !c.valores.is_empty() {
                                m.insert("enum".into(), textos(&c.valores));
                            }
                            Json::Obj(m)
                        })
                        .collect(),
                ),
            );
            if !t.clave.is_empty() {
                o.insert("primaryKey".into(), textos(&t.clave));
            }
            if !t.unicas.is_empty() {
                o.insert(
                    "uniqueKeys".into(),
                    Json::Arr(t.unicas.iter().map(|u| textos(u)).collect()),
                );
            }
            if !t.foraneas.is_empty() {
                o.insert(
                    "foreignKeys".into(),
                    Json::Arr(
                        t.foraneas
                            .iter()
                            .map(|f| {
                                let mut m: BTreeMap<String, Json> = BTreeMap::new();
                                m.insert("columns".into(), textos(&f.columnas));
                                m.insert("references".into(), Json::s(&f.destino));
                                if !f.destino_columnas.is_empty() {
                                    m.insert("toColumns".into(), textos(&f.destino_columnas));
                                }
                                Json::Obj(m)
                            })
                            .collect(),
                    ),
                );
            }
            if let Some(n) = t.filas {
                o.insert("rows".into(), Json::Int(n as i64));
            }
            if let Some(x) = &t.lee {
                o.insert("reads".into(), x.clone());
            }
            if let Some(x) = &t.cambia {
                o.insert("changes".into(), x.clone());
            }
            Json::Obj(o)
        })
        .collect();
    Json::obj([
        ("source", Json::s(&c.fuente)),
        ("tables", Json::Arr(tablas)),
    ])
    .pretty()
}

fn textos(v: &[String]) -> Json {
    Json::Arr(v.iter().map(Json::s).collect())
}

/// De un nodo analizado al valor JSON que era.
///
/// Exacto, no heurístico: `Node` distingue las tres formas, y `Style` distingue
/// un escalar **sin comillas** —sujeto a resolución implícita de tipo— de uno
/// entrecomillado, que siempre es una cadena. Así que `true` vuelve a ser un
/// booleano y `"true"` vuelve a ser el texto, que es lo que hace que un catálogo
/// leído y vuelto a escribir sea el mismo catálogo.
fn de_node(n: &Node) -> Json {
    match n {
        Node::Mapping { entries, .. } => Json::Obj(
            entries
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), de_node(v))))
                .collect(),
        ),
        Node::Sequence { items, .. } => Json::Arr(items.iter().map(de_node).collect()),
        Node::Scalar {
            raw,
            style: Style::Plain,
            ..
        } => match raw.as_str() {
            "true" => Json::Bool(true),
            "false" => Json::Bool(false),
            otro => match otro.parse::<i64>() {
                Ok(n) => Json::Int(n),
                Err(_) => Json::s(otro),
            },
        },
        Node::Scalar { raw, .. } => Json::s(raw),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Un catálogo **máximo**: las diecisiete claves, con valor. Es la única
    /// forma de que el ida y vuelta signifique algo — con la mitad puestas,
    /// perder la otra mitad no se notaría.
    const MAXIMO: &str = r#"{
      "source": "crm_prod",
      "tables": [
        { "name": "public.clientes",
          "kind": "table",
          "columns": [
            { "name": "id", "type": "Integer", "required": true },
            { "name": "raro", "sourceType": "hstore", "enum": ["a", "b"] },
            { "name": "fecha", "type": "String", "description": "DDMMAAAA, del AS/400" }
          ],
          "primaryKey": ["id"],
          "uniqueKeys": [["nif"]],
          "foreignKeys": [
            { "columns": ["pais"], "references": "public.paises", "toColumns": ["cod"] }
          ],
          "rows": 50000,
          "reads": { "predicatePushdown": ["eq"], "fullScan": "cheap", "projectionPushdown": true },
          "changes": { "mode": "upsert", "witness": "log", "key": ["id"] } }
      ]
    }"#;

    /// **El censo.** Lo que le da dientes a [`FORMA`].
    ///
    /// Se lee a sí mismo: una clave que el lector pregunte y que nadie haya
    /// declarado no compila la suite. Sin esto `FORMA` sería una lista bonita
    /// que el código puede dejar atrás sin que se rompa nada — que es
    /// exactamente la situación de la que sale este módulo.
    #[test]
    fn la_forma_declara_todo_lo_que_el_lector_pregunta() {
        let yo = include_str!("catalogo.rs");
        let cuerpo = &yo[yo.find("pub fn leer(").expect("sin lector")
            ..yo.find("pub fn escribir(").expect("sin emisor")];
        let mut preguntadas: Vec<String> = Vec::new();
        for pat in ["get(\"", "lista(t, \"", "lista(f, \"", "cadena(\""] {
            let mut resto = cuerpo;
            while let Some(i) = resto.find(pat) {
                resto = &resto[i + pat.len()..];
                if let Some(j) = resto.find('"') {
                    preguntadas.push(resto[..j].to_string());
                }
            }
        }
        preguntadas.sort();
        preguntadas.dedup();
        let sin_declarar: Vec<&String> = preguntadas
            .iter()
            .filter(|k| !FORMA.iter().any(|(n, _, _)| *n == k.as_str()))
            .collect();
        assert!(
            sin_declarar.is_empty(),
            "el lector pregunta por {sin_declarar:?} y `FORMA` no lo declara.\n\
             Dilo ahí —con su nivel y qué es— o un productor nuevo no sabrá que existe."
        );
        assert!(
            preguntadas.len() >= 15,
            "el censo no encontró casi nada: {preguntadas:?}"
        );
    }

    /// **Lo que se lee se escribe.** El ida y vuelta sobre el catálogo máximo:
    /// si el emisor se dejara una clave, la segunda vuelta no cuadraría.
    #[test]
    fn un_catalogo_leido_y_escrito_es_el_mismo_catalogo() {
        let una = escribir(&Catalogo::leer(MAXIMO).expect("el máximo analiza"));
        let dos = escribir(&Catalogo::leer(&una).expect("lo escrito se relee"));
        assert_eq!(una, dos, "el ida y vuelta no es estable");
        for (k, _, _) in FORMA {
            assert!(
                una.contains(&format!("\"{k}\"")),
                "`{k}` esta declarada y el emisor no la escribe:\n{una}"
            );
        }
    }

    /// Y que el ida y vuelta **conserva el tipo**. Sin `Style`, `true` volvería
    /// como la cadena `"true"` y un catálogo capturado cambiaría de forma cada
    /// vez que pasa por aquí — que es ruido en el diff que detecta la deriva.
    #[test]
    fn las_caras_conservan_su_tipo_al_volver() {
        let c = Catalogo::leer(MAXIMO).unwrap();
        let escrito = escribir(&c);
        assert!(
            escrito.contains("\"projectionPushdown\": true"),
            "{escrito}"
        );
        assert!(escrito.contains("\"rows\": 50000"), "{escrito}");
    }

    /// Una tabla sin nada opcional no inventa nada: las claves ausentes siguen
    /// ausentes. Es P4 en el emisor — omitir es cerrar, no abrir.
    #[test]
    fn lo_que_el_origen_no_dijo_no_se_escribe() {
        let c = Catalogo::leer(r#"{"source":"f","tables":[{"name":"t","columns":[]}]}"#).unwrap();
        let s = escribir(&c);
        for k in [
            "primaryKey",
            "uniqueKeys",
            "foreignKeys",
            "rows",
            "reads",
            "changes",
        ] {
            assert!(!s.contains(k), "se inventó `{k}`:\n{s}");
        }
        assert!(s.contains("\"kind\": \"table\""), "{s}");
    }
}
