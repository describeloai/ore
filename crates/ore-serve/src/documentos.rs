//! Los documentos del árbol, por `kind` (Ontology Forge I1): hoy `Entity`.
//!
//! `GET /documentos/Entity` · `GET|PUT|DELETE /documentos/Entity/{ns}/{n}`.
//!
//! # Por qué `/documentos/Entity` y no `/documentos?kind=Entity`
//!
//! La puerta **descarta la cadena de consulta a propósito** (`http.rs`: *ningún
//! dato entra por la URL*). El `kind` no es un dato, pero tampoco hace falta
//! abrir la consulta para decirlo: es un segmento, como `{n}` en `/modelos/{n}`.
//! Y va **literal** —`"Entity"`, no `{kind}`— para que la medida
//! (`medida-forge-contra-serve.py`) no cuente como servido lo que no lo está:
//! `View`, `Concept` y los demás entran en I2 con su segmento cada uno.
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

const API: &str = "oos.dev/v1alpha8";

/// Una entidad tal como está en el árbol: dónde y qué.
struct Documento {
    paquete: String,
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
    fn cualificado(&self) -> String {
        format!("{}.{}", self.espacio(), self.nombre())
    }
}

fn campo(n: &Node, padre: &str, k: &str) -> String {
    n.get(padre)
        .and_then(|(_, m)| m.get(k))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Todas las entidades del árbol, paquete a paquete, y cuántos ficheros no se
/// pudieron leer. Un fichero roto se salta y se cuenta, como en `/esquema`.
fn entidades_de(raiz: &Path) -> (Vec<Documento>, usize) {
    let mut lista = Vec::new();
    let mut rotos = 0;
    let Ok(paquetes) = std::fs::read_dir(raiz.join("packages")) else {
        return (lista, rotos);
    };
    let mut paquetes: Vec<_> = paquetes.flatten().map(|e| e.path()).collect();
    paquetes.sort();
    for p in paquetes.into_iter().filter(|p| p.is_dir()) {
        let Ok(ficheros) = std::fs::read_dir(p.join("entities")) else {
            continue;
        };
        let mut ficheros: Vec<_> = ficheros
            .flatten()
            .map(|e| e.path())
            .filter(|f| f.extension().is_some_and(|x| x == "yaml"))
            .collect();
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
            if nodo.get("kind").and_then(|(_, k)| k.as_str()) != Some("Entity") {
                continue;
            }
            if campo(&nodo, "metadata", "name").is_empty() {
                rotos += 1;
                continue;
            }
            lista.push(Documento {
                paquete: p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                fichero,
                texto,
                nodo,
            });
        }
    }
    (lista, rotos)
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
    Json::obj([
        ("kind", Json::s("Entity")),
        ("apiVersion", Json::s(campo_raiz(&d.nodo, "apiVersion"))),
        ("name", Json::s(d.nombre())),
        ("namespace", Json::s(d.espacio())),
        ("paquete", Json::s(d.paquete.clone())),
        ("fichero", Json::s(relativo)),
        ("metadata", parte("metadata")),
        ("spec", parte("spec")),
    ])
}

fn campo_raiz(n: &Node, k: &str) -> String {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// `GET /documentos/Entity`.
pub(crate) fn entidades(raiz: &Path) -> Respuesta {
    let (lista, rotos) = entidades_de(raiz);
    let mut salida = vec![(
        "documentos",
        Json::Arr(lista.iter().map(|d| ficha(raiz, d)).collect()),
    )];
    if rotos > 0 {
        salida.push(("ilegibles", Json::Int(rotos as i64)));
    }
    Respuesta::ok(Json::obj(salida))
}

/// `GET /documentos/Entity/{ns}/{n}`: la ficha, su YAML y el commit que la trajo.
pub(crate) fn entidad(raiz: &Path, ns: &str, n: &str) -> Respuesta {
    if let Err(r) = nombres(ns, n) {
        return r;
    }
    let (lista, _) = entidades_de(raiz);
    let Some(d) = lista.iter().find(|d| d.espacio() == ns && d.nombre() == n) else {
        return Respuesta::error(404, format!("no hay ninguna entidad `{ns}.{n}`"));
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

fn nombres(ns: &str, n: &str) -> Result<(), Respuesta> {
    token(ns).map_err(|m| Respuesta::error(422, format!("`namespace`: {m}")))?;
    token(n).map_err(|m| Respuesta::error(422, format!("`name`: {m}")))?;
    Ok(())
}

impl Servidor {
    /// `PUT /documentos/Entity/{ns}/{n}` con el documento en JSON: `metadata`
    /// (el nombre y el espacio los pone la ruta) y `spec`. 201 si es nueva,
    /// 200 si se reescribe; `commit` lo añade `escribiendo`.
    pub(crate) fn escribir_entidad(
        &self,
        raiz: &Path,
        ns: &str,
        n: &str,
        cuerpo: &str,
        si_commit: Option<&str>,
    ) -> Respuesta {
        if let Err(r) = nombres(ns, n) {
            return r;
        }
        let cuerpo = match analizar(cuerpo) {
            Ok(c) => c,
            Err(r) => return r,
        };
        if let Some(k) = cuerpo.get("kind").and_then(|(_, k)| k.as_str())
            && k != "Entity"
        {
            return Respuesta::error(
                422,
                format!("`kind: {k}` no es `Entity`: esta ruta escribe entidades"),
            );
        }
        let Some((_, spec)) = cuerpo.get("spec") else {
            return Respuesta::error(422, "falta `spec`");
        };
        if !matches!(spec, Node::Mapping { .. }) {
            return Respuesta::error(422, "`spec` tiene que ser un objeto");
        }
        // ── lo que este verbo exige y el compilador todavía no ─────────────
        if spec
            .get("backedBy")
            .and_then(|(_, v)| v.as_str())
            .is_none_or(str::is_empty)
        {
            return Respuesta::error(
                422,
                "falta `spec.backedBy`: la entidad no sale de ninguna vista, así que no hay fila que declarar. \
                 `ore validate` lo admite todavía por los bindings de v1alpha7, pero Forge no escribe bindings",
            );
        }
        if let Some(m) = cuerpo.get("metadata").map(|(_, m)| m) {
            for (k, sitio) in [("name", n), ("namespace", ns)] {
                if let Some(v) = m.get(k).and_then(|(_, v)| v.as_str())
                    && v != sitio
                {
                    return Respuesta::error(
                        422,
                        format!(
                            "`metadata.{k}: {v}` no es el de la ruta (`{sitio}`): el nombre lo pone la ruta"
                        ),
                    );
                }
            }
        }
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let paquete = raiz.join("packages").join(ns);
        if !paquete.is_dir() {
            return Respuesta::error(
                404,
                format!(
                    "no hay paquete `{ns}`: el espacio de nombres de una entidad es su paquete (OOS2030)"
                ),
            );
        }

        // ── el documento, en YAML y con la cabeza en su sitio ───────────────
        let api = cuerpo
            .get("apiVersion")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or(API);
        let mut texto =
            format!("apiVersion: {api}\nkind: Entity\nmetadata:\n  name: {n}\n  namespace: {ns}\n");
        if let Some((_, m)) = cuerpo.get("metadata") {
            for (k, v) in m.entries() {
                let Some(k) = k.as_str() else { continue };
                if k == "name" || k == "namespace" {
                    continue;
                }
                entrada_yaml(k, v, 1, &mut texto);
            }
        }
        texto.push_str("spec:\n");
        for (k, v) in spec.entries() {
            if let Some(k) = k.as_str() {
                entrada_yaml(k, v, 1, &mut texto);
            }
        }

        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let (lista, _) = entidades_de(raiz);
        let existente = lista
            .iter()
            .find(|d| d.espacio() == ns && d.nombre() == n)
            .map(|d| (d.fichero.clone(), d.texto.clone()));
        let fichero = existente
            .as_ref()
            .map(|(f, _)| f.clone())
            .unwrap_or_else(|| paquete.join("entities").join(format!("{n}.yaml")));
        if let Err(e) = std::fs::create_dir_all(paquete.join("entities"))
            .and_then(|_| std::fs::write(&fichero, &texto))
        {
            return Respuesta::error(
                500,
                format!("no se pudo escribir `{}`: {e}", relativo(raiz, &fichero)),
            );
        }
        // ── compilar antes de empujar: ¿empeora? ────────────────────────────
        if let Some(r) = self.empeora(raiz, &antes, &format!("la entidad `{ns}.{n}`")) {
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
        let ficha = Json::obj([
            ("kind", Json::s("Entity")),
            ("name", Json::s(n)),
            ("namespace", Json::s(ns)),
            ("paquete", Json::s(ns)),
            ("fichero", Json::s(relativo(raiz, &fichero))),
            ("nueva", Json::Bool(existente.is_none())),
        ]);
        if existente.is_none() {
            Respuesta::creado(ficha)
        } else {
            Respuesta::ok(ficha)
        }
    }

    /// `DELETE /documentos/Entity/{ns}/{n}`: fuera si nadie la nombra y el
    /// árbol sigue compilando; 409 con los nombres si alguien la referencia.
    pub(crate) fn retirar_entidad(
        &self,
        raiz: &Path,
        ns: &str,
        n: &str,
        si_commit: Option<&str>,
    ) -> Respuesta {
        if let Err(r) = nombres(ns, n) {
            return r;
        }
        let (lista, _) = entidades_de(raiz);
        let Some(d) = lista.iter().find(|d| d.espacio() == ns && d.nombre() == n) else {
            return Respuesta::error(404, format!("no hay ninguna entidad `{ns}.{n}`"));
        };
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let cualificado = d.cualificado();
        let quien: Vec<String> = lista
            .iter()
            .filter(|o| o.cualificado() != cualificado)
            .flat_map(|o| {
                let mismo_paquete = o.paquete == d.paquete;
                o.nodo
                    .get("spec")
                    .and_then(|(_, s)| s.get("relations"))
                    .map(|(_, r)| r.entries())
                    .unwrap_or(&[])
                    .iter()
                    .filter(|(_, rel)| {
                        let t = rel
                            .get("target")
                            .and_then(|(_, t)| t.as_str())
                            .unwrap_or_default();
                        t == cualificado || (mismo_paquete && t == n)
                    })
                    .filter_map(|(k, _)| {
                        k.as_str()
                            .map(|k| format!("`{}` (relations.{k})", o.cualificado()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        if !quien.is_empty() {
            return Respuesta::error(
                409,
                format!(
                    "no se retira `{cualificado}`: la nombra {}. Quita primero esas relaciones",
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
        if let Some(mut r) = self.empeora(raiz, &antes, &format!("sin la entidad `{cualificado}`"))
        {
            let _ = std::fs::write(&fichero, &texto);
            r.codigo = 409;
            return r;
        }
        Respuesta::ok(Json::obj([
            ("kind", Json::s("Entity")),
            ("name", Json::s(n)),
            ("namespace", Json::s(ns)),
            ("retirada", Json::Bool(true)),
        ]))
    }

    /// `If-Match` contra la cabeza del árbol. `None` si cuadra, si no se dijo,
    /// o si no hay historia contra la que mirar.
    fn arbol_se_movio(&self, raiz: &Path, si_commit: Option<&str>) -> Option<Respuesta> {
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
    fn diagnosticos_de(&self, raiz: &Path) -> Result<Vec<Json>, Respuesta> {
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

    /// ¿La escritura **añadió** diagnósticos? `None` si el árbol no empeora
    /// —aunque siga sin compilar por lo que ya tenía—; la 422 con **sólo los
    /// nuevos** si sí. Dos diagnósticos son el mismo defecto si coinciden en
    /// `(código, mensaje)`: la posición no cuenta.
    fn empeora(&self, raiz: &Path, antes: &[Json], que: &str) -> Option<Respuesta> {
        let despues = match self.diagnosticos_de(raiz) {
            Ok(d) => d,
            Err(r) => return Some(r),
        };
        let habia: std::collections::BTreeSet<(String, String)> =
            antes.iter().map(identidad_de).collect();
        let nuevos: Vec<Json> = despues
            .into_iter()
            .filter(|d| !habia.contains(&identidad_de(d)))
            .collect();
        if nuevos.is_empty() {
            return None;
        }
        let resumen = nuevos
            .iter()
            .map(|d| {
                let (c, m) = identidad_de(d);
                format!("{c}: {m}")
            })
            .collect::<Vec<_>>()
            .join(" · ");
        Some(Respuesta {
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

fn relativo(raiz: &Path, f: &Path) -> String {
    f.strip_prefix(raiz)
        .unwrap_or(f)
        .to_string_lossy()
        .replace('\\', "/")
}

// ── Lo que dice git ─────────────────────────────────────────────────────────

fn git(raiz: &Path, args: &[&str]) -> Option<String> {
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
fn cabeza_de(raiz: &Path) -> Option<String> {
    git(raiz, &["rev-parse", "HEAD"]).filter(|s| !s.is_empty())
}

/// El commit que trajo un fichero: `{hash, autor, fecha}`.
fn commit_de(raiz: &Path, fichero: &Path) -> Option<Json> {
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
