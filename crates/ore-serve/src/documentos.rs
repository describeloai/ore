//! Los documentos del árbol, por `kind` (Ontology Forge): un motor y una
//! tabla de kinds — hoy `Entity`, `View`, `Table`, `Concept`, `Interface` y
//! `TrainedModel` (v1alpha11, W3.7 ②).
//!
//! `GET /documentos/{kind}` · `GET|PUT|DELETE /documentos/{kind}/{ns}/{n}` ·
//! `GET /conceptos`.
//!
//! # Concept e Interface (medido el 2026-09-17, `medida-forge-concept-e-interface.py`)
//!
//! - **`OOS9004` es el estado entre dos escrituras, no un defecto de una**.
//!   Un `Concept` nuevo nunca podría entrar solo: nadie lo habla todavía, eso
//!   es `OOS9004`, y es un diagnóstico NUEVO que la puerta rechazaría; al revés
//!   tampoco, el `is` primero es `OOS2001`. Y el fire test (caso 18) midió el
//!   espejo: quitar el `is` de la única propiedad que lo habla también hace
//!   nuevo el `OOS9004`, y retirar el concepto antes es 409 porque está
//!   hablado — **ningún orden entraba**. Así que la puerta entera **tolera
//!   `OOS9004`** y lo dice en la respuesta (`sinHablar: [conceptos]`); el
//!   árbol lo seguirá diciendo (`/derivados/diagnosticos`) hasta que alguien
//!   lo hable o lo retire. No es una fila de la tabla: es del motor.
//! - **El motor recorre también la raíz**: `ore init` pone `interfaces/` ahí
//!   y el compilador acepta documentos fuera de `packages/` (el kind es el
//!   discriminante, no el directorio). Lo que no está en un paquete sale sin
//!   `paquete`; uno nuevo se escribe en `packages/<ns>/<carpeta>/`.
//! - **Quién nombra**: a un concepto, `Entity.properties.*.is` e
//!   `Interface.requires`; a una interfaz, `Entity.implements`.
//! - **`GET /conceptos`** = los `Concept` del árbol más los importados de
//!   `vendor/*.oob` (que no se escriben por aquí: son vocabulario publicado),
//!   cada uno con quién lo habla y quién lo exige. Un `.oob` es la forma
//!   canónica en JCS y se lee como cualquier documento.
//!
//! # Por qué `/documentos/{kind}` y no `/documentos?kind=…`
//!
//! La puerta **descarta la cadena de consulta a propósito** (`http.rs`: *ningún
//! dato entra por la URL*). El `kind` no es un dato, pero tampoco hace falta
//! abrir la consulta para decirlo: es un segmento, como `{n}` en `/modelos/{n}`.
//! Y se resuelve contra [`KINDS`]: un kind que no esté en la tabla es 404 con
//! la lista de los que sí, y la medida (`medida-forge-contra-serve.py`) lee la
//! misma tabla, así que lo que no se sirve no cuenta como servido. Los kinds
//! de I2 entran como filas, cada una medida antes de escribirla.
//!
//! # La figura, que es la de `/modelos`
//!
//! Clonar, escribir, **compilar antes de empujar**, empujar con el sujeto.
//! Una petición que acaba en 422 deja el clon a medias y el clon se tira: el
//! árbol nunca queda a medio escribir. Sobre un directorio (el banco) se
//! restaura lo que había, porque ahí no hay clon que tirar.
//!
//! # La puerta es «el árbol no empeora», no «el árbol compila»
//!
//! Medido el 2026-09-16 (`medida-forge-view-y-table.py` ③): un árbol recién
//! inducido **no compila** hasta que alguien revisa — `owner: cambiame`
//! (`OOS2009`), entidades sin clave (`OOS2010`), la fuente sin `source add`
//! (`OOS2004`). Con la puerta «compila entero», escribir una entidad válida en
//! ese árbol devolvía 422 con cinco errores que no eran suyos, y no había
//! forma de escribir nada hasta arreglar a mano todo lo demás: la puerta
//! bloqueaba justo cuando Forge más sirve, con el árbol a medias.
//!
//! Así que se compila **antes** de tocar nada y **después**, y se rechaza sólo
//! lo que la escritura **añade**: 422 (409 al retirar) con los diagnósticos
//! nuevos, y ninguno de los de siempre. Lo que estaba mal sigue igual de mal y
//! lo dirá `/derivados/diagnosticos` (I3), no la puerta de cada escritura.
//! Un diagnóstico se identifica por `(código, mensaje)`, sin la posición: una
//! línea que se mueve no es un defecto nuevo.
//!
//! # Lo que se midió antes de escribir (2026-09-16, `acme-retail`)
//!
//! | prueba | `ore validate` |
//! |---|---|
//! | `Entity` sin `backedBy` | **pasa** — los bindings de v1alpha7 siguen siendo legales |
//! | `backedBy` a una vista que no existe | `OOS2018` |
//! | una propiedad que la vista no expone | `OOS2022` |
//! | `metadata.displayName` sin prefijo | `OOS1005` |
//! | `relations.*.target` a lo que no está | `OOS2005` |
//!
//! ⇒ `backedBy` lo exige **este verbo**, no el compilador: Forge escribe
//! entidades v1alpha8 y no escribe bindings, así que una entidad sin
//! `backedBy` es una declaración sobre ninguna fila. Se dice con su motivo y
//! sin inventarle un código. El resto de las negativas son las del compilador,
//! tal cual las emite: `diagnosticos: [{codigo, mensaje, donde?, ayuda?}]`.
//!
//! # El árbol se movió: `If-Match`
//!
//! Quien leyó en el commit `A` y escribe puede decirlo con `If-Match: A`. Si el
//! árbol ya no está en `A`, 409 **antes** de escribir nada: la segunda de dos
//! personas que editan la misma ficha no pisa a la primera. Sin la cabecera
//! no se comprueba —es lo que hace hoy `/modelos`— y sobre un directorio sin
//! historia no hay contra qué comprobar.
//!
//! # `DELETE` y quien la nombra
//!
//! Antes de compilar se mira quién la referencia por `relations.*.target`, y
//! se contesta 409 **con los nombres**: `OOS2005` diría «`hr.Department` no
//! existe» apuntando a la que la nombra, que es la misma verdad contada desde
//! el sitio equivocado.

use crate::mando;
use crate::rutas::{Servidor, analizar, de_node, token};
use ore_core::json::Json;
use ore_core::parse::{self, Node, Style};
use ore_entrada::http::Respuesta;
use std::path::{Path, PathBuf};

// 0033: lo que se declara sin decir su version es de la version vigente, que
// es la del dataset. Una View de v1alpha12 no admite `materialized`.
const API: &str = "oos.dev/v1alpha12";

/// Lo que cambia de un `kind` a otro, y es **todo** lo que cambia: dónde se
/// escribe uno nuevo, qué exige el verbo antes de compilar, y con qué
/// artículo se nombra. El recorrido, la ficha, el YAML, `If-Match`, la puerta
/// y el commit son los mismos para todos. Quién nombra a quién está en
/// [`quien_nombra`], porque depende de DOS kinds: el que se retira y el que lo
/// referencia.
///
/// El `kind` de la ruta se resuelve contra esta tabla, y la medida
/// (`medida-forge-contra-serve.py`) la lee de aquí: un kind que no esté no se
/// sirve y no cuenta como servido.
pub(crate) struct Kind {
    pub nombre: &'static str,
    /// `packages/<ns>/<carpeta>/<n>.yaml` cuando el documento es nuevo. Uno
    /// que ya existe se reescribe donde esté: el fichero no se llama como el
    /// documento (medido: `discover` escribe `Clientes__public_clientes.yaml`
    /// con `name: clientes`).
    pub carpeta: &'static str,
    pub articulo: &'static str,
    /// Lo que el verbo exige del `spec` y el compilador todavía no. `Some`
    /// es el motivo del 422.
    pub exige: fn(&Node) -> Option<String>,
    /// `None`: se escribe y se retira por `/documentos`. `Some(verbo)`: se lee
    /// por aquí —la ficha del catálogo, su YAML— pero lo escribe otro verbo,
    /// porque escribirlo es más que el documento (0041: un `Model` es el
    /// documento Y la suscripción de la celda en el gateway, en un acto).
    pub escribe: Option<&'static str>,
}

pub(crate) const KINDS: &[Kind] = &[
    Kind {
        nombre: "Entity",
        carpeta: "entities",
        articulo: "la entidad",
        exige: exige_backed_by,
        escribe: None,
    },
    Kind {
        nombre: "View",
        carpeta: "views",
        articulo: "la vista",
        exige: exige_owner,
        escribe: None,
    },
    Kind {
        nombre: "Table",
        carpeta: "tables",
        articulo: "la tabla",
        exige: sin_exigencias,
        escribe: None,
    },
    // Sin exigencias propias: lo que falta ya es `OOS1004` (`type`).
    Kind {
        nombre: "Concept",
        carpeta: "concepts",
        articulo: "el concepto",
        exige: sin_exigencias,
        escribe: None,
    },
    Kind {
        nombre: "Interface",
        carpeta: "interfaces",
        articulo: "la interfaz",
        exige: sin_exigencias,
        escribe: None,
    },
    // v1alpha11 (0031 W3.7 ②). El modelo entrenado, publicado desde una
    // sesion: el compilador ya exige `owner`, `framework`, `version`,
    // `artifacts` y `digest` (OOS1004) y que `trainedFrom` resuelva (OOS2005).
    Kind {
        nombre: "TrainedModel",
        carpeta: "models",
        articulo: "el modelo entrenado",
        exige: sin_exigencias,
        escribe: None,
    },
    // v1alpha12 (0033). Lo que se tiene: el dataset mantenido (`from`, y el
    // sistema lo cumple) o escrito (`columns` + `changes`). `declare()` desde
    // un puesto lo deja en `packages/<ns>/datasets/`; el compilador exige la
    // forma (OOS1004), que `from` resuelva (OOS2018) y el conducto (OOS4011).
    Kind {
        nombre: "Dataset",
        carpeta: "datasets",
        articulo: "el dataset",
        exige: sin_exigencias,
        escribe: None,
    },
    // v1alpha10 (0034 paso 5). La lógica con contrato y la invocación sin
    // código entran por la misma puerta que los demás: la ficha del catálogo
    // pide el texto por aquí y el workspace lo abre por aquí. Lo que exigen
    // ya lo exige el compilador (`runtime`/`entrypoint`/`over` por OOS1004;
    // `over` y `reads` que resuelvan; un efecto sobre una propiedad con
    // integridad, OOS7005). `GET /funciones` sigue dando la forma con los
    // resultados de sus Jobs, que es otra cosa.
    Kind {
        nombre: "Function",
        carpeta: "functions",
        articulo: "la función",
        exige: sin_exigencias,
        escribe: None,
    },
    Kind {
        nombre: "Action",
        carpeta: "actions",
        articulo: "la acción",
        exige: sin_exigencias,
        escribe: None,
    },
    // v1alpha15 (0041). El modelo desplegado, en su base y su schema como
    // todo lo del catálogo: se lee por aquí, y lo escriben `POST /modelos` y
    // `DELETE /modelos/{ref}` por ESTE motor, más la suscripción.
    Kind {
        nombre: "Model",
        carpeta: "modelos",
        articulo: "el modelo",
        exige: sin_exigencias,
        escribe: Some(
            "lo escriben `POST /modelos` y `DELETE /modelos/{ref}`: el documento y la suscripción de la celda en el gateway van juntos, o nada",
        ),
    },
];

/// El único diagnóstico nuevo que la puerta deja pasar: «el concepto `hr.x`
/// no lo referencia nada del paquete». Es el estado entre escribir un
/// concepto y hablarlo, o entre dejar de hablarlo y retirarlo, y no hay orden
/// que lo evite (ver el módulo).
const SIN_HABLAR: &str = "OOS9004";

/// El concepto que un `OOS9004` nombra: lo que va entre las primeras comillas.
fn concepto_del(mensaje: &str) -> String {
    mensaje.split('`').nth(1).unwrap_or(mensaje).to_string()
}

pub(crate) fn kind_de(nombre: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.nombre == nombre)
}

fn kinds_servidos() -> String {
    KINDS
        .iter()
        .map(|k| k.nombre)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Medido: `ore validate` admite una `Entity` sin `backedBy` porque los
/// bindings de v1alpha7 siguen siendo legales. Forge no escribe bindings.
fn exige_backed_by(spec: &Node) -> Option<String> {
    spec.get("backedBy")
        .and_then(|(_, v)| v.as_str())
        .is_none_or(str::is_empty)
        .then(|| {
            "falta `spec.backedBy`: la entidad no sale de ninguna vista, así que no hay fila que declarar. \
             `ore validate` lo admite todavía por los bindings de v1alpha7, pero Forge no escribe bindings"
                .to_string()
        })
}

/// Medido: una `View` sin `owner` pasa el esquema; `owner` lo exige el
/// emisor (`cambiame` no valida: `OOS2009`). Aquí también.
fn exige_owner(spec: &Node) -> Option<String> {
    spec.get("owner")
        .and_then(|(_, v)| v.as_str())
        .is_none_or(str::is_empty)
        .then(|| {
            "falta `spec.owner`: quien responde de lo que la vista expone y con qué frescura. \
             `ore validate` no lo exige; un `CREATE VIEW` lo pone, y este verbo lo pide"
                .to_string()
        })
}

/// La tabla es un hecho: no tiene dueño, y todo lo demás lo exige el esquema.
fn sin_exigencias(_: &Node) -> Option<String> {
    None
}

/// Un documento tal como está en el árbol: qué es, dónde y qué dice.
struct Documento {
    kind: &'static Kind,
    /// `None` si vive fuera de `packages/` (la raíz, donde `ore init` deja
    /// `interfaces/`).
    paquete: Option<String>,
    fichero: PathBuf,
    texto: String,
    nodo: Node,
}

impl Documento {
    fn nombre(&self) -> String {
        campo(&self.nodo, "metadata", "name")
    }
    fn espacio(&self) -> String {
        campo(&self.nodo, "metadata", "namespace")
    }
    /// Su schema (0038): el que declara, o `default`.
    fn schema(&self) -> String {
        let s = campo(&self.nodo, "metadata", "schema");
        if s.is_empty() {
            ore_core::normalize::SCHEMA_POR_DEFECTO.to_string()
        } else {
            s
        }
    }
    /// En su forma corta: `p.n` en `default`, `p.s.n` en otro schema.
    fn cualificado(&self) -> String {
        ore_core::normalize::corto(&self.espacio(), &self.schema(), &self.nombre())
    }
}

fn campo(n: &Node, padre: &str, k: &str) -> String {
    n.get(padre)
        .and_then(|(_, m)| m.get(k))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Todos los documentos de los kinds servidos, paquete a paquete y después
/// la raíz, y cuántos ficheros no se pudieron leer. Se recorre cada paquete
/// **entero** y el `kind` es el discriminante, como hace el cargador de
/// `ore-core`: el directorio es convención, no regla. Un fichero roto se
/// salta y se cuenta, como en `/esquema`.
///
/// La raíz son los `.yaml` sueltos y los directorios que no son `packages/`
/// ni `vendor/` (ahí van los `.oob`, que no son YAML del árbol) ni ocultos:
/// `ore init` crea `interfaces/` ahí, y el compilador lo lee. Lo que se
/// encuentra en la raíz no tiene paquete.
fn documentos_de(raiz: &Path) -> (Vec<Documento>, usize) {
    let mut lista = Vec::new();
    let mut rotos = 0;
    let mut sitios: Vec<(Option<String>, Vec<PathBuf>)> = Vec::new();
    if let Ok(paquetes) = std::fs::read_dir(raiz.join("packages")) {
        let mut paquetes: Vec<_> = paquetes.flatten().map(|e| e.path()).collect();
        paquetes.sort();
        for p in paquetes.into_iter().filter(|p| p.is_dir()) {
            let nombre = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut ficheros = Vec::new();
            yamls_de(&p, &mut ficheros);
            sitios.push((Some(nombre), ficheros));
        }
    }
    if let Ok(entradas) = std::fs::read_dir(raiz) {
        let mut sueltos = Vec::new();
        for p in entradas.flatten().map(|e| e.path()) {
            let nombre = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if nombre.starts_with('.') || nombre == "packages" || nombre == "vendor" {
                continue;
            }
            if p.is_dir() {
                yamls_de(&p, &mut sueltos);
            } else if p.extension().is_some_and(|x| x == "yaml" || x == "yml") {
                sueltos.push(p);
            }
        }
        sitios.push((None, sueltos));
    }
    for (paquete, mut ficheros) in sitios {
        ficheros.sort();
        for fichero in ficheros {
            let Ok(texto) = std::fs::read_to_string(&fichero) else {
                rotos += 1;
                continue;
            };
            let Ok(nodo) = parse::parse(&texto) else {
                rotos += 1;
                continue;
            };
            let Some(kind) = nodo
                .get("kind")
                .and_then(|(_, k)| k.as_str())
                .and_then(kind_de)
            else {
                continue;
            };
            if campo(&nodo, "metadata", "name").is_empty() {
                rotos += 1;
                continue;
            }
            lista.push(Documento {
                kind,
                paquete: paquete.clone(),
                fichero,
                texto,
                nodo,
            });
        }
    }
    (lista, rotos)
}

/// ⭐ (0041) Un `Model` del árbol tal como el motor lo encuentra, con su
/// referencia: `<base>.<nombre>` o `<base>.<schema>.<nombre>`; uno de antes
/// (v1alpha9–14, en `modelos/` en la raíz, sin paquete), su nombre a secas.
pub(crate) struct ModeloDelArbol {
    pub fichero: PathBuf,
    pub nodo: Node,
    pub referencia: String,
    pub nombre: String,
    pub paquete: Option<String>,
    pub schema: Option<String>,
}

/// Los `Model` del árbol, del mismo recorrido que el resto de `/documentos`.
pub(crate) fn modelos_de(raiz: &Path) -> Vec<ModeloDelArbol> {
    let (lista, _) = documentos_de(raiz);
    let mut v: Vec<ModeloDelArbol> = lista
        .into_iter()
        .filter(|d| d.kind.nombre == "Model")
        .map(|d| {
            let con_sitio = !d.espacio().is_empty();
            ModeloDelArbol {
                referencia: if con_sitio {
                    d.cualificado()
                } else {
                    d.nombre()
                },
                nombre: d.nombre(),
                paquete: con_sitio.then(|| d.espacio()),
                schema: con_sitio.then(|| d.schema()),
                fichero: d.fichero,
                nodo: d.nodo,
            }
        })
        .collect();
    v.sort_by(|a, b| a.referencia.cmp(&b.referencia));
    v
}

/// La fila `Model` de la tabla.
pub(crate) fn kind_modelo() -> &'static Kind {
    kind_de("Model").expect("`Model` está en la tabla")
}

/// Los `.yaml` de un directorio, hacia dentro y sin entrar en los ocultos.
fn yamls_de(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entradas.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                yamls_de(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "yaml" || x == "yml") {
            out.push(p);
        }
    }
}

/// ¿`valor` apunta a `d`? Cualificado, o corto dentro del mismo espacio de
/// nombres (la forma corta es legal ahí: `normalize::qualify`).
fn apunta(valor: Option<&Node>, d: &Documento, mismo_espacio: bool) -> bool {
    let Some(v) = valor.and_then(|v| v.as_str()) else {
        return false;
    };
    v == d.cualificado() || (mismo_espacio && v == d.nombre())
}

/// ¿Alguno de una lista (`requires`, `implements`) apunta a `d`?
fn alguno_apunta(lista: Option<&Node>, d: &Documento, mismo_espacio: bool) -> bool {
    lista
        .map(|l| l.items())
        .unwrap_or(&[])
        .iter()
        .any(|v| apunta(Some(v), d, mismo_espacio))
}

/// Quién referencia a `d`, para decirlo en el 409 de `DELETE`. Medido
/// (`medida-forge-view-y-table.py` ⑥ y `…-concept-e-interface.py`): a una
/// entidad la nombran las `relations` de otra entidad; a una vista,
/// `Entity.backedBy` y `View.from.view`; a una tabla, `View.from.table`; a un
/// concepto, `Entity.properties.*.is` e `Interface.requires`; a una interfaz,
/// `Entity.implements`. Una `Function` escribe por `effects.writes:
/// hr.Employee.estado` —nombra la entidad— y la vista la alcanza por
/// `backedBy`.
fn quien_nombra(todos: &[Documento], d: &Documento) -> Vec<String> {
    let mut quien = Vec::new();
    for o in todos.iter().filter(|o| !std::ptr::eq(*o, d)) {
        let mismo = o.espacio() == d.espacio();
        let spec = o.nodo.get("spec").map(|(_, s)| s);
        let from = |k: &str| {
            spec.and_then(|s| s.get("from"))
                .and_then(|(_, f)| f.get(k))
                .map(|(_, v)| v)
        };
        let lista = |k: &str| spec.and_then(|s| s.get(k)).map(|(_, v)| v);
        match (d.kind.nombre, o.kind.nombre) {
            ("Concept", "Entity") => {
                for (prop, def) in spec
                    .and_then(|s| s.get("properties"))
                    .map(|(_, p)| p.entries())
                    .unwrap_or(&[])
                {
                    if apunta(def.get("is").map(|(_, v)| v), d, mismo)
                        && let Some(prop) = prop.as_str()
                    {
                        quien.push(format!("`{}` (properties.{prop}.is)", o.cualificado()));
                    }
                }
            }
            ("Concept", "Interface") if alguno_apunta(lista("requires"), d, mismo) => {
                quien.push(format!("`{}` (requires)", o.cualificado()));
            }
            ("Interface", "Entity") if alguno_apunta(lista("implements"), d, mismo) => {
                quien.push(format!("`{}` (implements)", o.cualificado()));
            }
            ("Entity", "Entity") => {
                for (k, rel) in spec
                    .and_then(|s| s.get("relations"))
                    .map(|(_, r)| r.entries())
                    .unwrap_or(&[])
                {
                    if apunta(rel.get("target").map(|(_, t)| t), d, mismo)
                        && let Some(k) = k.as_str()
                    {
                        quien.push(format!("`{}` (relations.{k})", o.cualificado()));
                    }
                }
            }
            ("View", "Entity") => {
                if apunta(
                    spec.and_then(|s| s.get("backedBy")).map(|(_, v)| v),
                    d,
                    mismo,
                ) {
                    quien.push(format!("`{}` (backedBy)", o.cualificado()));
                }
            }
            ("View", "View") if apunta(from("view"), d, mismo) => {
                quien.push(format!("`{}` (from.view)", o.cualificado()));
            }
            ("Table", "View") if apunta(from("table"), d, mismo) => {
                quien.push(format!("`{}` (from.table)", o.cualificado()));
            }
            // v1alpha15 §3: `model: modelo/<ref>`, leída por partes desde la
            // función; uno de antes (sin paquete), por su nombre a secas.
            ("Model", "Function") => {
                let Some(m) = spec
                    .and_then(|s| s.get("model"))
                    .and_then(|(_, v)| v.as_str())
                else {
                    continue;
                };
                let m = m.strip_prefix("modelo/").unwrap_or(m);
                let nombra = if d.espacio().is_empty() {
                    m == d.nombre()
                } else {
                    ore_core::normalize::qualify_catalogo(m, Some(&o.espacio()), &o.schema())
                        == d.cualificado()
                };
                if nombra {
                    quien.push(format!("`{}` (model)", o.cualificado()));
                }
            }
            _ => {}
        }
    }
    quien
}

/// La ficha de un documento: `metadata` y `spec` **enteros**, tal como están
/// (labels, aiContext, `x-*`, relations, uniqueKeys, temporal, moved,
/// reserved, implements…), más de qué paquete y de qué fichero.
fn ficha(raiz: &Path, d: &Documento) -> Json {
    let relativo = d
        .fichero
        .strip_prefix(raiz)
        .unwrap_or(&d.fichero)
        .to_string_lossy()
        .replace('\\', "/");
    let parte = |k: &str| {
        d.nodo
            .get(k)
            .map(|(_, v)| de_node(v))
            .unwrap_or(Json::obj([]))
    };
    let mut m = vec![
        ("kind", Json::s(d.kind.nombre)),
        ("apiVersion", Json::s(campo_raiz(&d.nodo, "apiVersion"))),
        ("name", Json::s(d.nombre())),
        ("namespace", Json::s(d.espacio())),
        ("schema", Json::s(d.schema())),
        ("fichero", Json::s(relativo)),
        ("metadata", parte("metadata")),
        ("spec", parte("spec")),
    ];
    // Sin `paquete` si vive en la raíz: la ficha no inventa uno.
    if let Some(p) = &d.paquete {
        m.push(("paquete", Json::s(p.clone())));
    }
    Json::obj(m)
}

fn campo_raiz(n: &Node, k: &str) -> String {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// El kind de la ruta, o el 404 que dice cuáles se sirven.
fn kind_o_404(kind: &str) -> Result<&'static Kind, Respuesta> {
    kind_de(kind).ok_or_else(|| {
        Respuesta::error(
            404,
            format!(
                "no se sirve `{kind}` por `/documentos`. Los que sí: {}",
                kinds_servidos()
            ),
        )
    })
}

/// `GET /documentos/{kind}`.
pub(crate) fn listar(raiz: &Path, kind: &str) -> Respuesta {
    let k = match kind_o_404(kind) {
        Ok(k) => k,
        Err(r) => return r,
    };
    let (lista, rotos) = documentos_de(raiz);
    let mut salida = vec![(
        "documentos",
        Json::Arr(
            lista
                .iter()
                .filter(|d| std::ptr::eq(d.kind, k))
                .map(|d| ficha(raiz, d))
                .collect(),
        ),
    )];
    if rotos > 0 {
        salida.push(("ilegibles", Json::Int(rotos as i64)));
    }
    Respuesta::ok(Json::obj(salida))
}

/// `GET /documentos/{kind}/{ns}[/{schema}]/{n}`: la ficha, su YAML y el commit
/// que la trajo.
pub(crate) fn uno(raiz: &Path, kind: &str, ns: &str, schema: &str, n: &str) -> Respuesta {
    let k = match kind_o_404(kind) {
        Ok(k) => k,
        Err(r) => return r,
    };
    if let Err(r) = nombres(ns, schema, n) {
        return r;
    }
    let (lista, _) = documentos_de(raiz);
    let Some(d) = buscar(&lista, k, ns, schema, n) else {
        return Respuesta::error(
            404,
            format!(
                "no hay {} `{}`",
                k.articulo,
                ore_core::normalize::corto(ns, schema, n)
            ),
        );
    };
    let Json::Obj(mut m) = ficha(raiz, d) else {
        unreachable!("la ficha es un objeto");
    };
    m.insert("yaml".into(), Json::s(d.texto.clone()));
    if let Some(c) = commit_de(raiz, &d.fichero) {
        m.insert("commit".into(), c);
    }
    Respuesta::ok(Json::Obj(m))
}

fn buscar<'a>(
    lista: &'a [Documento],
    k: &Kind,
    ns: &str,
    schema: &str,
    n: &str,
) -> Option<&'a Documento> {
    lista.iter().find(|d| {
        std::ptr::eq(d.kind, k) && d.espacio() == ns && d.schema() == schema && d.nombre() == n
    })
}

fn nombres(ns: &str, schema: &str, n: &str) -> Result<(), Respuesta> {
    token(ns).map_err(|m| Respuesta::error(422, format!("`namespace`: {m}")))?;
    token(schema).map_err(|m| Respuesta::error(422, format!("`schema`: {m}")))?;
    token(n).map_err(|m| Respuesta::error(422, format!("`name`: {m}")))?;
    Ok(())
}

/// Lo que llega en un PUT: el documento en JSON (un formulario), o `{"yaml":
/// "…"}` tal cual (el texto, con sus comentarios). Los dos pasan por la misma
/// puerta; lo que cambia es quién emite el YAML.
fn documento_del_cuerpo(
    k: &Kind,
    ns: &str,
    schema: &str,
    n: &str,
    cuerpo: &str,
) -> Result<(String, Node), Respuesta> {
    let cuerpo = analizar(cuerpo)?;
    // ── el texto tal cual ───────────────────────────────────────────────────
    if let Some((_, y)) = cuerpo.get("yaml") {
        let Some(texto) = y.as_str() else {
            return Err(Respuesta::error(422, "`yaml` tiene que ser una cadena"));
        };
        let doc = parse::parse(texto)
            .map_err(|e| Respuesta::error(422, format!("el `yaml` no analiza: {e:?}")))?;
        comprobar_cabeza(k, ns, schema, n, &doc)?;
        return Ok((texto.to_string(), doc));
    }
    // ── el documento en JSON: se emite ──────────────────────────────────────
    comprobar_cabeza(k, ns, schema, n, &cuerpo)?;
    let Some((_, spec)) = cuerpo.get("spec") else {
        return Err(Respuesta::error(422, "falta `spec`"));
    };
    // En un schema (0038), v1alpha13 y `metadata.schema` (01 §3).
    let en_schema = schema != ore_core::normalize::SCHEMA_POR_DEFECTO;
    let api = cuerpo
        .get("apiVersion")
        .and_then(|(_, v)| v.as_str())
        .unwrap_or(if en_schema { "oos.dev/v1alpha13" } else { API });
    let mut texto = format!(
        "apiVersion: {api}\nkind: {}\nmetadata:\n  name: {n}\n  namespace: {ns}\n",
        k.nombre
    );
    if en_schema {
        texto.push_str(&format!("  schema: {schema}\n"));
    }
    if let Some((_, m)) = cuerpo.get("metadata") {
        for (kk, v) in m.entries() {
            let Some(kk) = kk.as_str() else { continue };
            if kk == "name" || kk == "namespace" || kk == "schema" {
                continue;
            }
            entrada_yaml(kk, v, 1, &mut texto);
        }
    }
    texto.push_str("spec:\n");
    for (kk, v) in spec.entries() {
        if let Some(kk) = kk.as_str() {
            entrada_yaml(kk, v, 1, &mut texto);
        }
    }
    Ok((texto, cuerpo))
}

/// `kind`, `metadata.name` y `metadata.namespace`, si vienen, son los de la
/// ruta; `spec` es un objeto; y lo que el verbo exige, está.
fn comprobar_cabeza(
    k: &Kind,
    ns: &str,
    schema: &str,
    n: &str,
    doc: &Node,
) -> Result<(), Respuesta> {
    if let Some(kd) = doc.get("kind").and_then(|(_, v)| v.as_str())
        && kd != k.nombre
    {
        return Err(Respuesta::error(
            422,
            format!(
                "`kind: {kd}` no es `{}`: esta ruta escribe {}",
                k.nombre, k.articulo
            ),
        ));
    }
    if let Some((_, m)) = doc.get("metadata") {
        // El schema, como el nombre, lo pone la ruta (0038): uno que no lo
        // dice es de `default`.
        let dice = m
            .get("schema")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or(ore_core::normalize::SCHEMA_POR_DEFECTO);
        if dice != schema {
            return Err(Respuesta::error(
                422,
                format!(
                    "`metadata.schema: {dice}` no es el de la ruta (`{schema}`): el nombre lo pone la ruta"
                ),
            ));
        }
        for (campo, sitio) in [("name", n), ("namespace", ns)] {
            if let Some(v) = m.get(campo).and_then(|(_, v)| v.as_str())
                && v != sitio
            {
                return Err(Respuesta::error(
                    422,
                    format!(
                        "`metadata.{campo}: {v}` no es el de la ruta (`{sitio}`): el nombre lo pone la ruta"
                    ),
                ));
            }
        }
    }
    let Some((_, spec)) = doc.get("spec") else {
        return Err(Respuesta::error(422, "falta `spec`"));
    };
    if !matches!(spec, Node::Mapping { .. }) {
        return Err(Respuesta::error(422, "`spec` tiene que ser un objeto"));
    }
    if let Some(motivo) = (k.exige)(spec) {
        return Err(Respuesta::error(422, motivo));
    }
    Ok(())
}

impl Servidor {
    /// `PUT /documentos/{kind}/{ns}/{n}` con el documento en JSON (`metadata`
    /// y `spec`; el nombre y el espacio los pone la ruta) o con `yaml` tal
    /// cual. 201 si es nuevo, 200 si se reescribe; `commit` lo añade
    /// `escribiendo`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn escribir_documento(
        &self,
        raiz: &Path,
        kind: &str,
        ns: &str,
        schema: &str,
        n: &str,
        cuerpo: &str,
        si_commit: Option<&str>,
    ) -> Respuesta {
        let k = match kind_o_404(kind) {
            Ok(k) => k,
            Err(r) => return r,
        };
        if let Some(verbo) = k.escribe {
            return Respuesta::error(
                405,
                format!("{} no se escribe por `/documentos`: {verbo}", k.articulo),
            );
        }
        self.escribir_en_su_sitio(raiz, k, ns, schema, n, cuerpo, si_commit)
    }

    /// La escritura del motor, para un kind ya resuelto: en `packages/<ns>[/<schema>]/<carpeta>/`,
    /// compilando antes de empujar. La usa `/documentos` y, para el `Model`, `/modelos` (0041).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn escribir_en_su_sitio(
        &self,
        raiz: &Path,
        k: &'static Kind,
        ns: &str,
        schema: &str,
        n: &str,
        cuerpo: &str,
        si_commit: Option<&str>,
    ) -> Respuesta {
        if let Err(r) = nombres(ns, schema, n) {
            return r;
        }
        let (texto, _) = match documento_del_cuerpo(k, ns, schema, n, cuerpo) {
            Ok(t) => t,
            Err(r) => return r,
        };
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let paquete = raiz.join("packages").join(ns);
        if !paquete.is_dir() {
            return Respuesta::error(
                404,
                format!(
                    "no hay paquete `{ns}`: el espacio de nombres de un documento es su paquete (OOS2030)"
                ),
            );
        }
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let (lista, _) = documentos_de(raiz);
        let existente =
            buscar(&lista, k, ns, schema, n).map(|d| (d.fichero.clone(), d.texto.clone()));
        // Uno nuevo, en la carpeta de su schema (01 §3): la del paquete en
        // `default`, `<paquete>/<schema>/` en otro.
        let carpeta = if schema == ore_core::normalize::SCHEMA_POR_DEFECTO {
            paquete.join(k.carpeta)
        } else {
            paquete.join(schema).join(k.carpeta)
        };
        let fichero = existente
            .as_ref()
            .map(|(f, _)| f.clone())
            .unwrap_or_else(|| carpeta.join(format!("{n}.yaml")));
        if let Err(e) = std::fs::create_dir_all(fichero.parent().unwrap_or(&carpeta))
            .and_then(|_| std::fs::write(&fichero, &texto))
        {
            return Respuesta::error(
                500,
                format!("no se pudo escribir `{}`: {e}", relativo(raiz, &fichero)),
            );
        }
        // ── compilar antes de empujar: ¿empeora? ────────────────────────────
        let corto = ore_core::normalize::corto(ns, schema, n);
        let sin_hablar = match self.empeora(raiz, &antes, &format!("{} `{corto}`", k.articulo)) {
            Ok(t) => t,
            Err(r) => {
                // (sobre un directorio no hay clon que tirar: se deja como estaba)
                match &existente {
                    Some((f, t)) => {
                        let _ = std::fs::write(f, t);
                    }
                    None => {
                        let _ = std::fs::remove_file(&fichero);
                    }
                }
                return r;
            }
        };
        let mut ficha = vec![
            ("kind", Json::s(k.nombre)),
            ("name", Json::s(n)),
            ("namespace", Json::s(ns)),
            ("schema", Json::s(schema)),
            ("fichero", Json::s(relativo(raiz, &fichero))),
            ("nueva", Json::Bool(existente.is_none())),
        ];
        if fichero.starts_with(raiz.join("packages")) {
            ficha.push(("paquete", Json::s(ns)));
        }
        if !sin_hablar.is_empty() {
            // Conceptos que esta escritura deja sin nadie que los hable: entró,
            // y el árbol lo dirá (OOS9004) hasta que alguien los hable o los retire.
            ficha.push((
                "sinHablar",
                Json::Arr(sin_hablar.into_iter().map(Json::s).collect()),
            ));
        }
        let ficha = Json::obj(ficha);
        if existente.is_none() {
            Respuesta::creado(ficha)
        } else {
            Respuesta::ok(ficha)
        }
    }

    /// `DELETE /documentos/{kind}/{ns}/{n}`: fuera si nadie lo nombra y el
    /// árbol no empeora; 409 con los nombres si alguien lo referencia.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn retirar_documento(
        &self,
        raiz: &Path,
        kind: &str,
        ns: &str,
        schema: &str,
        n: &str,
        si_commit: Option<&str>,
        sujeto: &ore_entrada::identidad::Identidad,
    ) -> Respuesta {
        let k = match kind_o_404(kind) {
            Ok(k) => k,
            Err(r) => return r,
        };
        if let Some(verbo) = k.escribe {
            return Respuesta::error(
                405,
                format!("{} no se retira por `/documentos`: {verbo}", k.articulo),
            );
        }
        self.retirar_de_su_sitio(raiz, k, ns, schema, n, si_commit, sujeto)
    }

    /// La retirada del motor, para un kind ya resuelto: 409 con quién lo nombra, o
    /// fuera si el árbol no empeora. La usa `/documentos` y, para el `Model`, `/modelos`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn retirar_de_su_sitio(
        &self,
        raiz: &Path,
        k: &'static Kind,
        ns: &str,
        schema: &str,
        n: &str,
        si_commit: Option<&str>,
        sujeto: &ore_entrada::identidad::Identidad,
    ) -> Respuesta {
        if let Err(r) = nombres(ns, schema, n) {
            return r;
        }
        let corto = ore_core::normalize::corto(ns, schema, n);
        // Lo escrito es de quien lo escribió (W3.7 gobierno ④): retirar un
        // Dataset con puntero de otra persona es 403 con quién.
        if k.nombre == "Dataset"
            && let Some((_, p)) =
                ore_core::punteros::leer_en(&raiz.join(ore_core::punteros::CARPETA), &corto)
            && let Some(e) = p.get("escrito_por").and_then(|(_, v)| v.as_str())
            && !e.is_empty()
            && e != sujeto.persona
        {
            return Respuesta::error(
                403,
                format!(
                    "el dataset `{corto}` lo escribió `{e}`: retirarlo es suyo; lo tuyo va por una propuesta"
                ),
            );
        }
        let (lista, _) = documentos_de(raiz);
        let Some(d) = buscar(&lista, k, ns, schema, n) else {
            return Respuesta::error(404, format!("no hay {} `{corto}`", k.articulo));
        };
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let cualificado = d.cualificado();
        let mut quien = quien_nombra(&lista, d);
        // Y lo que lee la consulta de una vista SQL (0040 paso 6): no está en
        // un campo, así que se resuelve con el árbol.
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        for v in pkg.docs.iter().filter(|v| ore_core::vistas::es_sql(v)) {
            if ore_core::servir::nombrados(&pkg, v)
                .iter()
                .any(|x| x.doc.path.ends_with(&d.fichero))
            {
                quien.push(format!("`{}` (sql)", v.qname().unwrap_or_default()));
            }
        }
        if !quien.is_empty() {
            return Respuesta::error(
                409,
                format!(
                    "no se retira {} `{cualificado}`: la nombra {}. Quita primero esas referencias",
                    k.articulo,
                    quien.join(", ")
                ),
            );
        }
        let (fichero, texto) = (d.fichero.clone(), d.texto.clone());
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        if let Err(e) = std::fs::remove_file(&fichero) {
            return Respuesta::error(
                500,
                format!("no se pudo retirar `{}`: {e}", relativo(raiz, &fichero)),
            );
        }
        if let Err(mut r) =
            self.empeora(raiz, &antes, &format!("sin {} `{cualificado}`", k.articulo))
        {
            let _ = std::fs::write(&fichero, &texto);
            r.codigo = 409;
            return r;
        }
        // Un Dataset se va con su puntero (0031 W3.7 gobierno ①): medido
        // antes, retirar el documento dejaba el puntero huérfano —un estado
        // sin nada que lo nombre—. Los bytes los expira el mantenimiento
        // (`--recoger`), como siempre. El de su sitio y el de antes (0038 P2).
        let dir = raiz.join(ore_core::punteros::CARPETA);
        let punteros: Vec<std::path::PathBuf> = [
            ore_core::punteros::ruta_en(&dir, &corto),
            ore_core::punteros::legado_en(&dir, &corto),
        ]
        .into_iter()
        .flatten()
        .filter(|p| p.is_file())
        .collect();
        let con_puntero = k.nombre == "Dataset" && !punteros.is_empty();
        if con_puntero {
            for puntero in &punteros {
                if let Err(e) = std::fs::remove_file(puntero) {
                    let _ = std::fs::write(&fichero, &texto);
                    return Respuesta::error(
                        500,
                        format!("no se pudo retirar `{}`: {e}", puntero.display()),
                    );
                }
            }
        }
        Respuesta::ok(Json::obj([
            ("kind", Json::s(k.nombre)),
            ("name", Json::s(n)),
            ("namespace", Json::s(ns)),
            ("retirada", Json::Bool(true)),
            ("puntero", Json::Bool(con_puntero)),
        ]))
    }

    /// `If-Match` contra la cabeza del árbol. `None` si cuadra, si no se dijo,
    /// o si no hay historia contra la que mirar.
    pub(crate) fn arbol_se_movio(&self, raiz: &Path, si_commit: Option<&str>) -> Option<Respuesta> {
        let esperado = si_commit?.trim().trim_matches('"');
        if esperado.is_empty() {
            return None;
        }
        let cabeza = cabeza_de(raiz)?;
        if cabeza.starts_with(esperado) || esperado.starts_with(&cabeza) {
            return None;
        }
        Some(Respuesta::error(
            409,
            format!(
                "el árbol se movió: se leyó en `{}` y ahora está en `{}`. Nada se escribió; hay que volver a leer y decidir sobre lo que hay ahora",
                &esperado[..esperado.len().min(12)],
                &cabeza[..cabeza.len().min(12)]
            ),
        ))
    }

    /// Los diagnósticos del árbol tal como está: la foto de ANTES. Si el
    /// compilador falla sin diagnósticos (no arranca, no es un árbol), se
    /// cuenta la primera línea como uno, para que la foto no salga limpia
    /// sobre algo que no compila.
    pub(crate) fn diagnosticos_de(&self, raiz: &Path) -> Result<Vec<Json>, Respuesta> {
        match mando::correr(&self.binario, raiz, &["validate".into(), ".".into()]) {
            Err(e) => Err(Respuesta::error(500, e.to_string())),
            Ok(s) if !s.bien() => {
                let mut d = diagnosticos(&s.stderr);
                if d.is_empty() {
                    d.push(Json::obj([
                        ("codigo", Json::s("")),
                        (
                            "mensaje",
                            Json::s(crate::rutas::primera_linea(&s.stdout, &s.stderr)),
                        ),
                    ]));
                }
                Ok(d)
            }
            Ok(_) => Ok(Vec::new()),
        }
    }

    /// ¿La escritura **añadió** diagnósticos? `Ok` si el árbol no empeora
    /// —aunque siga sin compilar por lo que ya tenía—, con los conceptos que
    /// los `OOS9004` nuevos dejan sin hablar; la 422 con **sólo los nuevos**
    /// si sí. Dos diagnósticos son el mismo defecto si coinciden en
    /// `(código, mensaje)`: la posición no cuenta.
    pub(crate) fn empeora(
        &self,
        raiz: &Path,
        antes: &[Json],
        que: &str,
    ) -> Result<Vec<String>, Respuesta> {
        self.empeora_salvo(raiz, antes, que, |_| false)
    }

    /// Como [`Self::empeora`], pero un diagnóstico nuevo por el que `destapado`
    /// responde `true` NO cuenta como empeorar. El validador va por fases y
    /// se para en la primera que falla (`validate_package`): quitar del árbol
    /// lo que fallaba en una fase temprana —una base con nombre que no es
    /// espacio de nombres, `OOS2030`— deja llegar a fases que antes no corrían,
    /// y lo que sale de ahí ya estaba, sólo que tapado. Medido en victor el
    /// 2026-09-17: retirar `test-standard` daba 422 por los `OOS2009`/`OOS2010`
    /// de las otras dos bases, que ni nombran a la retirada.
    pub(crate) fn empeora_salvo(
        &self,
        raiz: &Path,
        antes: &[Json],
        que: &str,
        destapado: impl Fn(&Json) -> bool,
    ) -> Result<Vec<String>, Respuesta> {
        let despues = self.diagnosticos_de(raiz)?;
        let habia: std::collections::BTreeSet<(String, String)> =
            antes.iter().map(identidad_de).collect();
        let (sin_hablar, nuevos): (Vec<Json>, Vec<Json>) = despues
            .into_iter()
            .filter(|d| !habia.contains(&identidad_de(d)) && !destapado(d))
            .partition(|d| identidad_de(d).0 == SIN_HABLAR);
        if nuevos.is_empty() {
            return Ok(sin_hablar
                .iter()
                .map(|d| concepto_del(&identidad_de(d).1))
                .collect());
        }
        let resumen = nuevos
            .iter()
            .map(|d| {
                let (c, m) = identidad_de(d);
                format!("{c}: {m}")
            })
            .collect::<Vec<_>>()
            .join(" · ");
        Err(Respuesta {
            codigo: 422,
            cuerpo: Json::obj([
                (
                    "error",
                    Json::s(format!("el árbol empeora con {que}: {resumen}")),
                ),
                ("diagnosticos", Json::Arr(nuevos)),
                // Cuántos había ya y siguen: para que quien lee sepa que el
                // 422 no los cuenta, y que existen.
                ("previos", Json::Int(antes.len() as i64)),
            ]),
        })
    }
}

// ── /conceptos: los del árbol y los importados, con quién los habla ────────

/// `GET /conceptos`. Los `Concept` del árbol (los que `/documentos/Concept`
/// escribe) más los de `vendor/*.oob` (`importado: true`, con el `paquete`
/// que el sobre declara), y para cada uno quién lo habla —`hablado`: las
/// propiedades con `is`, como `hr.Employee.email`— y quién lo exige
/// —`exigido`: las interfaces con `requires`—. Medido: el compilador cuenta
/// las dos cosas como hablar (`OOS9004` no salta si una interfaz lo exige).
pub(crate) fn conceptos(raiz: &Path) -> Respuesta {
    let (lista, mut rotos) = documentos_de(raiz);
    // quién habla y quién exige, por nombre cualificado del concepto
    let mut hablado: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut exigido: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for d in &lista {
        let Some((_, spec)) = d.nodo.get("spec") else {
            continue;
        };
        match d.kind.nombre {
            "Entity" => {
                for (prop, def) in spec
                    .get("properties")
                    .map(|(_, p)| p.entries())
                    .unwrap_or(&[])
                {
                    if let (Some(prop), Some(c)) =
                        (prop.as_str(), def.get("is").and_then(|(_, v)| v.as_str()))
                    {
                        hablado
                            .entry(cualificar(c, &d.espacio()))
                            .or_default()
                            .push(format!("{}.{prop}", d.cualificado()));
                    }
                }
            }
            "Interface" => {
                for c in spec
                    .get("requires")
                    .map(|(_, r)| r.items())
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|c| c.as_str())
                {
                    exigido
                        .entry(cualificar(c, &d.espacio()))
                        .or_default()
                        .push(d.cualificado());
                }
            }
            _ => {}
        }
    }
    let mut salida: Vec<Json> = Vec::new();
    let quien = |m: &mut Json, q: &str| {
        let Json::Obj(m) = m else { return };
        for (clave, tabla) in [("hablado", &hablado), ("exigido", &exigido)] {
            let lista = tabla.get(q).cloned().unwrap_or_default();
            m.insert(
                clave.into(),
                Json::Arr(lista.into_iter().map(Json::s).collect()),
            );
        }
    };
    for d in lista.iter().filter(|d| d.kind.nombre == "Concept") {
        let mut f = ficha(raiz, d);
        if let Json::Obj(m) = &mut f {
            m.insert("importado".into(), Json::Bool(false));
        }
        quien(&mut f, &d.cualificado());
        salida.push(f);
    }
    // ── los importados: cada `.oob` de vendor/ ──────────────────────────────
    let mut sobres: Vec<PathBuf> = std::fs::read_dir(raiz.join("vendor"))
        .map(|e| {
            e.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "oob"))
                .collect()
        })
        .unwrap_or_default();
    sobres.sort();
    for sobre in sobres {
        let Some(nodo) = std::fs::read_to_string(&sobre)
            .ok()
            .and_then(|t| parse::parse(&t).ok())
        else {
            rotos += 1;
            continue;
        };
        let paquete = campo_raiz(&nodo, "package");
        let Some((_, documentos)) = nodo.get("documents") else {
            rotos += 1;
            continue;
        };
        for (_, doc) in documentos.entries() {
            if doc.get("kind").and_then(|(_, k)| k.as_str()) != Some("Concept") {
                continue;
            }
            let (name, namespace) = (
                campo(doc, "metadata", "name"),
                campo(doc, "metadata", "namespace"),
            );
            let parte = |k: &str| doc.get(k).map(|(_, v)| de_node(v)).unwrap_or(Json::obj([]));
            let mut f = Json::obj([
                ("kind", Json::s("Concept")),
                ("apiVersion", Json::s(campo_raiz(doc, "apiVersion"))),
                ("name", Json::s(name.clone())),
                ("namespace", Json::s(namespace.clone())),
                ("paquete", Json::s(paquete.clone())),
                ("fichero", Json::s(relativo(raiz, &sobre))),
                ("importado", Json::Bool(true)),
                ("metadata", parte("metadata")),
                ("spec", parte("spec")),
            ]);
            quien(&mut f, &format!("{namespace}.{name}"));
            salida.push(f);
        }
    }
    let mut cuerpo = vec![("conceptos", Json::Arr(salida))];
    if rotos > 0 {
        cuerpo.push(("ilegibles", Json::Int(rotos as i64)));
    }
    Respuesta::ok(Json::obj(cuerpo))
}

/// La forma corta es legal dentro del mismo espacio de nombres.
fn cualificar(referencia: &str, espacio: &str) -> String {
    if referencia.contains('.') {
        referencia.to_string()
    } else {
        format!("{espacio}.{referencia}")
    }
}

/// Lo que identifica un diagnóstico: el código y el mensaje, sin dónde cayó.
fn identidad_de(d: &Json) -> (String, String) {
    match d {
        Json::Obj(m) => (texto_de(m.get("codigo")), texto_de(m.get("mensaje"))),
        _ => (String::new(), String::new()),
    }
}

fn texto_de(j: Option<&Json>) -> String {
    match j {
        Some(Json::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

pub(crate) fn relativo(raiz: &Path, f: &Path) -> String {
    f.strip_prefix(raiz)
        .unwrap_or(f)
        .to_string_lossy()
        .replace('\\', "/")
}

// ── Lo que dice git ─────────────────────────────────────────────────────────

pub(crate) fn git(raiz: &Path, args: &[&str]) -> Option<String> {
    let s = std::process::Command::new("git")
        .current_dir(raiz)
        .args(args)
        .output()
        .ok()?;
    if !s.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&s.stdout).trim().to_string())
}

/// La cabeza del árbol, o nada si no hay historia (un directorio del banco).
pub(crate) fn cabeza_de(raiz: &Path) -> Option<String> {
    git(raiz, &["rev-parse", "HEAD"]).filter(|s| !s.is_empty())
}

/// El commit que trajo un fichero: `{hash, autor, fecha}`.
pub(crate) fn commit_de(raiz: &Path, fichero: &Path) -> Option<Json> {
    let rel = fichero
        .strip_prefix(raiz)
        .ok()?
        .to_string_lossy()
        .into_owned();
    let s = git(
        raiz,
        &["log", "-1", "--format=%h%x1f%an%x1f%aI", "--", &rel],
    )?;
    let mut partes = s.split('\u{1f}');
    let hash = partes.next()?.to_string();
    if hash.is_empty() {
        return None;
    }
    Some(Json::obj([
        ("hash", Json::s(hash)),
        ("autor", Json::s(partes.next().unwrap_or_default())),
        ("fecha", Json::s(partes.next().unwrap_or_default())),
    ]))
}

// ── Lo que dice el compilador ───────────────────────────────────────────────

/// Los diagnósticos tal como `ore validate` los escribe:
///
/// ```text
/// error[OOS2022]: `hr.empleados` no expone `inventada`, que `hr.Employee` declara
///   → packages/hr/entities/Employee.yaml:111:5
///   ayuda: una entidad sale de UNA vista, …
/// ```
pub(crate) fn diagnosticos(stderr: &str) -> Vec<Json> {
    let mut lista: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();
    for linea in stderr.lines() {
        let recortada = linea.trim_end();
        if let Some(resto) = recortada.strip_prefix("error[").or_else(|| {
            recortada
                .strip_prefix("warning[")
                .or_else(|| recortada.strip_prefix("aviso["))
        }) && let Some((codigo, mensaje)) = resto.split_once("]:")
        {
            lista.push((
                codigo.trim().to_string(),
                mensaje.trim().to_string(),
                None,
                None,
            ));
            continue;
        }
        let Some(ultimo) = lista.last_mut() else {
            continue;
        };
        let t = recortada.trim_start();
        if let Some(d) = t.strip_prefix("→ ") {
            ultimo.2 = Some(d.trim().to_string());
        } else if let Some(a) = t.strip_prefix("ayuda:") {
            ultimo.3 = Some(a.trim().to_string());
        } else if !t.is_empty()
            && recortada.starts_with(' ')
            && let Some(a) = &mut ultimo.3
        {
            a.push(' ');
            a.push_str(t);
        }
    }
    lista
        .into_iter()
        .map(|(codigo, mensaje, donde, ayuda)| {
            let mut m = vec![("codigo", Json::s(codigo)), ("mensaje", Json::s(mensaje))];
            if let Some(d) = donde {
                m.push(("donde", Json::s(d)));
            }
            if let Some(a) = ayuda {
                m.push(("ayuda", Json::s(a)));
            }
            Json::obj(m)
        })
        .collect()
}

// ── Lo que se escribe: YAML, del nodo que llegó ─────────────────────────────
//
// Se emite desde el `Node` y no desde `Json`, para no perder lo que el estilo
// del escalar dice: un `"1"` entrecomillado sigue entrecomillado y un `1`
// llano sigue llano. Es la misma razón por la que `parse.rs` no convierte.

fn sangrar(out: &mut String, nivel: usize) {
    for _ in 0..nivel {
        out.push_str("  ");
    }
}

/// `clave: valor` (o `clave:` y el valor debajo), al nivel dado.
fn entrada_yaml(clave: &str, v: &Node, nivel: usize, out: &mut String) {
    sangrar(out, nivel);
    out.push_str(&escalar_yaml(clave, Style::Plain));
    out.push(':');
    valor_yaml(v, nivel, out);
}

/// Lo que va tras `clave:` o tras `-`: en la misma línea si es un escalar o
/// un contenedor vacío; debajo si no.
fn valor_yaml(v: &Node, nivel: usize, out: &mut String) {
    match v {
        Node::Scalar { raw, style, .. } => {
            out.push(' ');
            out.push_str(&escalar_yaml(raw, *style));
            out.push('\n');
        }
        Node::Mapping { entries, .. } if entries.is_empty() => out.push_str(" {}\n"),
        Node::Sequence { items, .. } if items.is_empty() => out.push_str(" []\n"),
        Node::Mapping { entries, .. } => {
            out.push('\n');
            for (k, v) in entries {
                let k = k.as_str().unwrap_or_default();
                entrada_yaml(k, v, nivel + 1, out);
            }
        }
        Node::Sequence { items, .. } => {
            out.push('\n');
            for it in items {
                sangrar(out, nivel + 1);
                out.push('-');
                match it {
                    Node::Mapping { entries, .. } if !entries.is_empty() => {
                        // el primer par en la línea del guion, el resto debajo
                        let mut primero = true;
                        for (k, v) in entries {
                            let k = k.as_str().unwrap_or_default();
                            if primero {
                                out.push(' ');
                                out.push_str(&escalar_yaml(k, Style::Plain));
                                out.push(':');
                                valor_yaml(v, nivel + 2, out);
                                primero = false;
                            } else {
                                entrada_yaml(k, v, nivel + 2, out);
                            }
                        }
                    }
                    otro => valor_yaml(otro, nivel + 1, out),
                }
            }
        }
    }
}

/// Un escalar llano si puede serlo sin cambiar de tipo ni de valor; si no,
/// entre comillas dobles con los escapes de JSON, que YAML lee igual.
///
/// El cuerpo llega en JSON, donde TODA cadena va entre comillas: si eso se
/// respetara tal cual, el fichero saldría entero entrecomillado. Así que una
/// cadena entrecomillada vuelve llana **salvo que llana se leyera como otra
/// cosa** —`"1"`, `"true"`, `"null"`—, que es lo único que las comillas decían.
fn escalar_yaml(raw: &str, style: Style) -> String {
    let parece_otro_tipo = raw.parse::<f64>().is_ok()
        || matches!(
            raw.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off"
        );
    let llano = !matches!(style, Style::Block)
        && !(matches!(style, Style::Quoted) && parece_otro_tipo)
        && !raw.is_empty()
        && raw == raw.trim()
        && !raw.contains('\n')
        && !raw.contains(": ")
        && !raw.contains(" #")
        && !raw.ends_with(':')
        && !raw.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%',
            '@', '`',
        ]);
    if llano {
        raw.to_string()
    } else {
        Json::s(raw).jcs()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_diagnosticos_se_leen_tal_como_los_escribe_el_compilador() {
        let e = "error[OOS2022]: `hr.empleados` no expone `x`, que `hr.E` declara\n  → packages/hr/entities/E.yaml:11:5\n  ayuda: una entidad sale de UNA vista\n\nerror[OOS1005]: clave desconocida `metadata.displayName`\n  → packages/hr/entities/E.yaml:6:3\n\n2 errores\n";
        let d = diagnosticos(e);
        assert_eq!(d.len(), 2);
        assert_eq!(
            d[0].jcs(),
            r#"{"ayuda":"una entidad sale de UNA vista","codigo":"OOS2022","donde":"packages/hr/entities/E.yaml:11:5","mensaje":"`hr.empleados` no expone `x`, que `hr.E` declara"}"#
        );
        assert!(d[1].jcs().contains(r#""codigo":"OOS1005""#));
    }

    /// El YAML que se escribe vuelve a analizar igual: mismos escalares, mismo
    /// estilo (un `"1"` sigue siendo una cadena), mismo orden.
    #[test]
    fn el_yaml_emitido_vuelve_a_leerse_igual() {
        let n = parse::parse(
            r#"{"nature":"entity","primaryKey":["id"],"backedBy":"v","properties":{"id":{"type":"String"},"n":{"type":"Money<EUR, 2>","enum":["a","b: c"],"description":"con: dos puntos"}},"relations":{"r":{"target":"hr.X","cardinality":"many_to_one","via":["id"]}},"x-rubix-titulo":"1","vacio":{},"lista":[],"moved":[{"from":"a","to":"b","since":"0.3.0"}]}"#,
        )
        .unwrap();
        let mut out = String::from("spec:\n");
        for (k, v) in n.entries() {
            entrada_yaml(k.as_str().unwrap(), v, 1, &mut out);
        }
        let leido = parse::parse(&out).unwrap();
        let spec = leido.get("spec").unwrap().1;
        assert_eq!(de_node(spec).jcs(), de_node(&n).jcs(), "\n{out}");
        assert!(
            out.contains("x-rubix-titulo: \"1\""),
            "una cadena entrecomillada no puede volver llana:\n{out}"
        );
        assert!(
            out.contains("- from: a\n      to: b"),
            "el primer par va en la linea del guion:\n{out}"
        );
    }

    #[test]
    fn el_oos9004_nombra_al_concepto_entre_comillas() {
        assert_eq!(
            concepto_del("el concepto `hr.personalEmail` no lo referencia nada del paquete"),
            "hr.personalEmail"
        );
        assert_eq!(cualificar("personalEmail", "hr"), "hr.personalEmail");
        assert_eq!(cualificar("gdpr.personalEmail", "hr"), "gdpr.personalEmail");
    }

    #[test]
    fn un_escalar_que_yaml_leeria_distinto_va_entre_comillas() {
        assert_eq!(escalar_yaml("hola", Style::Plain), "hola");
        assert_eq!(escalar_yaml("Money<EUR, 2>", Style::Plain), "Money<EUR, 2>");
        assert_eq!(escalar_yaml("a: b", Style::Plain), "\"a: b\"");
        assert_eq!(escalar_yaml("- x", Style::Plain), "\"- x\"");
        assert_eq!(escalar_yaml("", Style::Plain), "\"\"");
        assert_eq!(escalar_yaml("1", Style::Quoted), "\"1\"");
        assert_eq!(
            escalar_yaml("dos\nlineas", Style::Block),
            "\"dos\\nlineas\""
        );
    }
}
