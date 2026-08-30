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
//! diagnósticos**: `ore validate` ya es la cola de revisión, dicha en la voz del
//! compilador, y `ore review` será su cara interactiva.
//!
//! # Lo que NO hace, y no por falta de tiempo
//!
//! **No acuña conceptos.** Un nombre de columna repetido en tres tablas es una
//! *candidata* a concepto, no un concepto: acuñar uno por columna repetida es la
//! inflación que `02-property` §6.2 nombra —*cuatro mil columnas producen cuatro
//! mil conceptos, que es igual que no tener vocabulario*—. Se reporta para que la
//! unificación se decida **una vez y no quince**.
//!
//! **No singulariza ni convierte a camelCase.** `pedidos` da `Pedidos`, no
//! `Pedido`: singularizar es adivinar un idioma. Y `id_pedido` se queda como
//! está, porque renombrarlo rompería la correspondencia con el nombre físico a
//! cambio de estética. Renombrar es de `review`, donde hay un humano.

use ore_core::json::Json;
use ore_core::parse::{self, Node};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

// ── El catálogo ─────────────────────────────────────────────────────────────

/// Una columna, ya traducida al sistema de tipos de OOS por el lector.
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

/// Una tabla del origen. `nombre` es **opaco**: sus reglas son del sistema de
/// origen y por eso viaja tal cual al `Binding`.
struct Tabla {
    nombre: String,
    columnas: Vec<Columna>,
    clave: Vec<String>,
    /// `(columnas locales, tabla destino)` — solo lo que el catálogo DECLARA.
    foraneas: Vec<(Vec<String>, String)>,
    filas: Option<u64>,
    /// `table`, `view` o `materializedView`, tal y como lo dijo el origen.
    clase: String,
}

/// Lo que el lector entrega.
pub struct Catalogo {
    fuente: String,
    tablas: Vec<Tabla>,
}

impl Catalogo {
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
            let columnas = t
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
            tablas.push(Tabla {
                nombre: nombre.to_string(),
                columnas,
                clave: lista(t, "primaryKey"),
                foraneas: t
                    .get("foreignKeys")
                    .map(|(_, v)| v.items())
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|f| {
                        Some((
                            lista(f, "columns"),
                            f.get("references")?.1.as_str()?.to_string(),
                        ))
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
            });
        }
        Ok(Catalogo { fuente, tablas })
    }
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

// ── Lo inducido ─────────────────────────────────────────────────────────────

/// Una decisión que el inductor **no toma**.
pub struct Pendiente {
    pub sujeto: String,
    pub que: String,
    pub porque: String,
}

pub struct Induccion {
    pub ficheros: BTreeMap<String, String>,
    pub pendientes: Vec<Pendiente>,
}

/// Induce un paquete de un catálogo.
///
/// `paquete` es el espacio de nombres y el nombre del `Package`. El resultado no
/// se escribe: se devuelve, para que quien llama decida —y para que esto se
/// pueda comprobar sin tocar el disco.
pub fn inducir(cat: &Catalogo, paquete: &str) -> Induccion {
    let mut ficheros = BTreeMap::new();
    let mut pendientes = Vec::new();

    // Los nombres de entidad, antes de emitir nada: dos tablas pueden dar el
    // mismo, y entonces ninguna de las dos se puede emitir sin decidir cuál es.
    let mut por_nombre: BTreeMap<String, Vec<&Tabla>> = BTreeMap::new();
    for t in &cat.tablas {
        por_nombre.entry(entidad(&t.nombre)).or_default().push(t);
    }

    for (nombre, tablas) in &por_nombre {
        if tablas.len() > 1 {
            pendientes.push(Pendiente {
                sujeto: tablas
                    .iter()
                    .map(|t| t.nombre.clone())
                    .collect::<Vec<_>>()
                    .join(" · "),
                que: format!("colisionan en `{nombre}`"),
                porque: "dos tablas dan el mismo identificador de OOS. Elegir una \
                         automáticamente decidiría cuál de las dos existe"
                    .into(),
            });
            continue;
        }
        let t = tablas[0];

        if t.clave.is_empty() {
            pendientes.push(Pendiente {
                sujeto: t.nombre.clone(),
                que: "sin clave primaria".into(),
                porque: "el origen no la declara. `01-package` §5: NO DEBE inferirse — \
                         sin clave no hay identidad, y una identidad inventada es peor \
                         que ninguna"
                    .into(),
            });
        }

        // Una columna cuyo tipo el lector no supo traducir no se emite: no hay
        // tipo que poner, y `Opaque` afirmaria «no hay estructura dentro» de algo
        // que el origen acaba de enumerar. Se nombra, con lo que dijo el origen.
        for c in t.columnas.iter().filter(|c| c.tipo.is_none()) {
            pendientes.push(Pendiente {
                sujeto: format!("{}.{}", t.nombre, c.nombre),
                que: "sin tipo de OOS".into(),
                porque: format!(
                    "el origen dice `{}`. Puede ser un objeto embebido o una entidad \
                     aparte, y las dos lecturas son modelos distintos: elegir una es \
                     modelar, no traducir",
                    c.origen
                        .as_deref()
                        .unwrap_or("un tipo que no se sabe traducir")
                ),
            });
        }
        // Una vista es una PROYECCION de algo. Puede ser la entidad, o puede ser
        // un informe sobre ella: emitirla sin mas duplicaria el concepto.
        if t.clase != "table" {
            pendientes.push(Pendiente {
                sujeto: t.nombre.clone(),
                que: format!("el origen la declara `{}`", t.clase),
                porque: "una vista es una proyeccion, y una proyeccion puede ser la \
                         entidad o puede ser un informe sobre ella. Emitirla como \
                         entidad sin mas duplicaria el concepto"
                    .into(),
            });
        }
        // Y si NINGUNA columna se pudo tipar, no hay entidad que escribir: un
        // `properties` vacio no valida, y llenarlo seria inventarlo.
        if t.columnas.iter().all(|c| c.tipo.is_none()) {
            pendientes.push(Pendiente {
                sujeto: t.nombre.clone(),
                que: "ninguna columna tiene tipo de OOS".into(),
                porque: "no queda nada que emitir. `properties` exige al menos una, y \
                         rellenarla seria inventar el modelo entero"
                    .into(),
            });
            continue;
        }

        ficheros.insert(
            format!("entities/{nombre}.yaml"),
            entidad_yaml(nombre, paquete, t),
        );
        ficheros.insert(
            format!("bindings/{}.yaml", identificador(&t.nombre)),
            binding_yaml(nombre, paquete, &cat.fuente, t),
        );

        if t.filas == Some(0) {
            pendientes.push(Pendiente {
                sujeto: t.nombre.clone(),
                que: "cero filas".into(),
                porque: "puede ser una tabla viva y vacía o un resto. El inductor no \
                         distingue una cosa de la otra, y borrarla sería decidirlo"
                    .into(),
            });
        }
    }

    ficheros.insert("package.yaml".into(), paquete_yaml(paquete));
    pendientes.extend(fragmentadas(&por_nombre));
    pendientes.extend(candidatas_a_concepto(&cat.tablas));
    pendientes.extend(relaciones_no_declaradas(&cat.tablas, &por_nombre));

    Induccion {
        ficheros,
        pendientes,
    }
}

// ── Las conjeturas, que se reportan y no se escriben ────────────────────────

/// Tablas cuyo nombre solo difiere en un sufijo de dígitos: `evento_20190101`,
/// `_02`, `_03`. Es **una** entidad con eje temporal, no tres — pero decir cuál
/// es la columna de tiempo es del humano.
fn fragmentadas(por_nombre: &BTreeMap<String, Vec<&Tabla>>) -> Vec<Pendiente> {
    let mut familias: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for nombre in por_nombre.keys() {
        let raiz = nombre.trim_end_matches(|c: char| c.is_ascii_digit());
        if raiz.len() < nombre.len() && raiz.len() > 1 {
            familias
                .entry(raiz.trim_end_matches('_').to_string())
                .or_default()
                .push(nombre.clone());
        }
    }
    familias
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(raiz, v)| Pendiente {
            sujeto: v.join(" · "),
            que: format!("¿una sola entidad `{raiz}` con eje temporal?"),
            porque: "el sufijo numérico es el patrón de una tabla fragmentada por fecha. \
                     Unirlas exige nombrar la columna de tiempo, y eso no está en el catálogo"
                .into(),
        })
        .collect()
}

/// Un nombre de columna que se repite en varias tablas **con el mismo tipo** es
/// una candidata a concepto. No se acuña: se muestran juntas para que la
/// unificación se decida una vez.
fn candidatas_a_concepto(tablas: &[Tabla]) -> Vec<Pendiente> {
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
    donde
        .into_iter()
        .filter(|((n, _), v)| v.len() > 1 && !estructural(n))
        .map(|((n, tipo), v)| Pendiente {
            sujeto: format!("`{n}: {tipo}` en {}", v.join(", ")),
            que: "¿el mismo concepto?".into(),
            porque: "acuñar uno por columna repetida es la inflación que produce cuatro mil \
                     conceptos. Decidirlo una vez vale por todas las apariciones"
                .into(),
        })
        .collect()
}

/// Una columna `X_id` cuando existe una entidad que casa con `X`. En un origen
/// sin claves foráneas —BigQuery no las tiene— es la única pista que hay, y es
/// una pista, no un hecho.
fn relaciones_no_declaradas(
    tablas: &[Tabla],
    por_nombre: &BTreeMap<String, Vec<&Tabla>>,
) -> Vec<Pendiente> {
    let mut out = Vec::new();
    for t in tablas {
        let declaradas: Vec<&String> = t.foraneas.iter().flat_map(|(c, _)| c).collect();
        let propia = entidad(&t.nombre);
        for c in &t.columnas {
            // Ya declarada como foránea, o parte de la clave: `pedidos.id_pedido`
            // es la identidad de `pedidos`, no una arista de la tabla a sí misma.
            // El sufijo `_id` dice «esto identifica algo», y la mitad de las veces
            // ese algo es la propia fila.
            if declaradas.contains(&&c.nombre) || t.clave.contains(&c.nombre) {
                continue;
            }
            let Some(raiz) = c
                .nombre
                .strip_suffix("_id")
                .or_else(|| c.nombre.strip_prefix("id_"))
            else {
                continue;
            };
            if raiz.is_empty() {
                continue;
            }
            let destino = por_nombre.keys().find(|n| {
                n.to_ascii_lowercase()
                    .starts_with(&raiz.to_ascii_lowercase())
            });
            if let Some(d) = destino.filter(|d| **d != propia) {
                out.push(Pendiente {
                    sujeto: format!("{}.{}", t.nombre, c.nombre),
                    que: format!("¿una relación hacia `{d}`?"),
                    porque: "el origen no declara la clave foránea, así que esto es un \
                             parecido de nombres. Emitir la arista convertiría una \
                             coincidencia en una afirmación sobre el grafo"
                        .into(),
                });
            }
        }
    }
    out
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

fn paquete_yaml(paquete: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Package\n\
         metadata: {{ name: {paquete}, version: 0.1.0, status: active, domain: {paquete} }}\n\
         spec: {{ owner: \"cambiame\" }}\n"
    )
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

fn entidad_yaml(nombre: &str, paquete: &str, t: &Tabla) -> String {
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
    if t.clave.is_empty() {
        s.push_str(
            "  # Sin clave primaria: el origen no la declara y NO DEBE inferirse.\n\
             \x20 # `ore validate` lo dirá con OOS2010, que es esta decisión escrita\n\
             \x20 # en la voz del compilador.\n",
        );
    } else {
        let _ = writeln!(s, "  primaryKey: [{}]", t.clave.join(", "));
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
        match &c.descripcion {
            None => {
                let _ = writeln!(
                    s,
                    "    {}: {{ type: {tipo} }}{obligatoria}",
                    identificador(&c.nombre)
                );
            }
            Some(d) => {
                let _ = writeln!(
                    s,
                    "    {}:{obligatoria}\n      type: {tipo}\n      description: {}",
                    identificador(&c.nombre),
                    entrecomillar(d)
                );
            }
        }
    }
    if !t.foraneas.is_empty() {
        s.push_str("  relations:\n");
        for (columnas, destino) in &t.foraneas {
            let _ = write!(
                s,
                "    {}:\n      target: {paquete}.{}\n      cardinality: many_to_one\n      via: {}\n      required: false\n",
                identificador(&entidad(destino)).to_lowercase(),
                entidad(destino),
                identificador(&columnas[0])
            );
        }
    }
    s
}

fn binding_yaml(nombre: &str, paquete: &str, fuente: &str, t: &Tabla) -> String {
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
        identificador(&t.nombre),
        t.nombre
    );
    for c in &t.columnas {
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
    let id = identificador(ultimo);
    let mut c = id.chars();
    match c.next() {
        Some(p) => p.to_uppercase().collect::<String>() + c.as_str(),
        None => id,
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
        let _ = writeln!(s, "    {}\n", p.porque);
    }
    s.push_str("  ore validate <ruta>   ·   las que el compilador ya sabe decir\n");
    s
}

/// El informe también en JSON, porque `ore review` va a leerlo y una persona no
/// es el único consumidor de esto.
pub fn informe_json(ind: &Induccion) -> Json {
    Json::obj([(
        "pending",
        Json::Arr(
            ind.pendientes
                .iter()
                .map(|p| {
                    Json::obj([
                        ("subject", Json::s(&p.sujeto)),
                        ("decision", Json::s(&p.que)),
                        ("because", Json::s(&p.porque)),
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
        assert!(p.sujeto.contains("Evento_20190101") && p.sujeto.contains("Evento_20190102"));
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

    #[test]
    fn los_nombres_no_se_singularizan_ni_se_inventan() {
        assert_eq!(entidad("rubix_demo_ventas.pedidos"), "Pedidos");
        assert_eq!(entidad("public.tb_order"), "Tb_order");
        assert_eq!(identificador("2024_total"), "t_2024_total");
        assert_eq!(identificador("first.name"), "first_name");
    }
}
