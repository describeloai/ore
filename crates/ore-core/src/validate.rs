//! `ore validate` — el nivel **L0**.
//!
//! Completamente hermético: sin red, sin credenciales, sin reloj, sin
//! aleatoriedad. Nada de este módulo puede leer ninguna de esas cosas, y esa
//! restricción es la que hace verdad la frase que vende el producto: *el paso
//! que decide qué significan las cosas es el único que no puede filtrar nada.*
//!
//! # Precedencia
//!
//! `99-errors.md` §2.1 es normativo y aquí se materializa como el **orden** de
//! las comprobaciones: no se puede fallar la validación contra un esquema que
//! no se ha podido seleccionar.
//!
//! ```text
//! OOS1001  analizar          ─┐
//! OOS1002  apiVersion         │ despacho: elegir contra qué validar
//! OOS1003  kind              ─┘
//! OOS1005  claves            ─┐
//! OOS1004  forma             ─┘ validación
//! ```

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{self, Kind};
use crate::parse::{self, Node};
use std::path::{Path, PathBuf};

/// Valida un único documento. Se detiene en el primer fallo: continuar tras un
/// error de despacho produciría ruido, no información.
pub fn validate_document(file: &Path, text: &str) -> Vec<Diagnostic> {
    // ── OOS1001 · análisis ──────────────────────────────────────────────────
    //
    // Un fichero es un FLUJO (`90-canonical-form` §5.3): se validan todos sus
    // documentos, no el primero. Un fichero vacío no es un error — no dice nada,
    // y no decir nada no es decir algo mal.
    let roots = match parse::parse_stream(text) {
        Ok(r) => r,
        Err(e) => {
            return vec![Diagnostic::new(Code::Oos1001, file, e.message).at(e.pos)];
        }
    };
    roots.iter().flat_map(|r| validar_raiz(file, r)).collect()
}

/// Valida **un** documento ya analizado. Se detiene en el primer fallo:
/// continuar tras un error de despacho produciría ruido, no información.
fn validar_raiz(file: &Path, root: &Node) -> Vec<Diagnostic> {
    let Node::Mapping { .. } = root else {
        return vec![
            Diagnostic::new(Code::Oos1004, file, "el documento raíz debe ser un mapa")
                .at(root.pos()),
        ];
    };

    // ── OOS1002 · apiVersion ────────────────────────────────────────────────
    let Some((k, v)) = root.get("apiVersion") else {
        return vec![
            Diagnostic::new(Code::Oos1002, file, "falta `apiVersion`")
                .at(root.pos())
                .help(format!("añade `apiVersion: {}`", document::API_VERSION)),
        ];
    };
    let Some(version) = v.as_str().and_then(document::ApiVersion::parse) else {
        let (msg, pos) = match v.as_str() {
            Some(other) => (format!("`apiVersion: {other}` no está soportada"), k.pos()),
            None => ("`apiVersion` debe ser una cadena".to_string(), v.pos()),
        };
        return vec![
            Diagnostic::new(Code::Oos1002, file, msg)
                .at(pos)
                .help(format!(
                    "esta implementación entiende {}. Hay un esquema por apiVersion: sin \
                     resolver la versión no hay contra qué validar",
                    document::ApiVersion::ALL
                        .iter()
                        .map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
        ];
    };

    // ── OOS1003 · kind ──────────────────────────────────────────────────────
    let Some((kk, kv)) = root.get("kind") else {
        return vec![Diagnostic::new(Code::Oos1003, file, "falta `kind`").at(root.pos())];
    };
    // Un `kind` que existe en otra versión no es desconocido: es del futuro, y
    // el error tiene que distinguirlo de una errata.
    if let Some(k) = kv.as_str().and_then(Kind::parse)
        && k.since() > version
    {
        return vec![
            Diagnostic::new(
                Code::Oos1003,
                file,
                format!("`kind: {}` no existe en {}", k.as_str(), version.as_str()),
            )
            .at(kk.pos())
            .help(format!(
                "es un documento de {}. Cambia el `apiVersion` del documento si es lo que \
                 querías declarar",
                k.since().as_str()
            )),
        ];
    }
    let Some(kind) = kv.as_str().and_then(Kind::parse) else {
        let nombre = kv.as_str().unwrap_or("<no es una cadena>");
        return vec![
            Diagnostic::new(
                Code::Oos1003,
                file,
                format!("`kind: {nombre}` no es un documento de v1alpha1"),
            )
            .at(kk.pos())
            .help(format!(
                "los documentos son: {}",
                Kind::ALL
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        ];
    };

    let mut diags = Vec::new();

    // ── OOS1005 · claves desconocidas ───────────────────────────────────────
    //
    // Se comprueba antes que la forma porque una errata es más probable que un
    // incumplimiento estructural, y decirlo con el nombre del campo ayuda más.
    // En `OntologyConfig` las secciones cuelgan de la raíz, así que el conjunto
    // admitido allí es la unión: comprobarlas por separado rechazaría
    // `datasources` antes de llegar a mirarla.
    let mut raiz_ok: Vec<&str> = kind.root_keys().to_vec();
    if kind.sections_at_root() {
        raiz_ok.extend_from_slice(kind.spec_keys());
    }
    check_keys(file, root, &raiz_ok, "", &mut diags);

    if let Some((_, meta)) = root.get("metadata") {
        check_keys(file, meta, kind.metadata_keys(), "metadata.", &mut diags);
    }
    if !kind.sections_at_root()
        && let Some((_, spec)) = root.get("spec")
    {
        check_keys(file, spec, kind.spec_keys(), "spec.", &mut diags);
        // Y dentro de cada propiedad. Sin esto, un `qualtiy:` mal escrito se
        // acepta en silencio y la propiedad queda sin gobernar — un hueco que
        // no produce ningún síntoma.
        if kind == Kind::Entity
            && let Some((_, props)) = spec.get("properties")
        {
            for (k, v) in props.entries() {
                let Some(nombre) = k.as_str() else { continue };
                check_keys(
                    file,
                    v,
                    kind.property_keys(),
                    &format!("spec.properties.{nombre}."),
                    &mut diags,
                );
            }
        }
    }
    if !diags.is_empty() {
        return diags;
    }

    // ── OOS6003 · forma canónica ────────────────────────────────────────────
    //
    // Antes que `OOS1004`: el esquema JSON acepta un número donde el perfil
    // admite un número, y no tiene forma de saber que ese número no sobrevive
    // a la serialización canónica. Es la regla de precedencia de `99-errors`
    // §2.1 — el código específico gana — aplicada a la familia de bytes.
    diags.extend(crate::canonical::check(file, root, kind));
    if !diags.is_empty() {
        return diags;
    }

    // ── OOS1004 · forma ─────────────────────────────────────────────────────
    for regla in document::shape_rules()
        .into_iter()
        .filter(|r| r.kind == kind)
    {
        let mut nodo = root;
        let mut encontrado = true;
        for seg in regla.path {
            match nodo.get(seg) {
                Some((_, v)) => nodo = v,
                None => {
                    encontrado = false;
                    break;
                }
            }
        }
        if encontrado && let Some((msg, help)) = (regla.check)(nodo) {
            let mut d = Diagnostic::new(Code::Oos1004, file, msg).at(nodo.pos());
            if let Some(h) = help {
                d = d.help(h);
            }
            diags.push(d);
        }
    }

    diags
}

fn check_keys(
    file: &Path,
    map: &Node,
    permitidas: &[&str],
    prefijo: &str,
    out: &mut Vec<Diagnostic>,
) {
    for (k, _) in map.entries() {
        let Some(name) = k.as_str() else { continue };
        if permitidas.contains(&name) || document::is_extension(name) {
            continue;
        }
        // `apiVersion` y `kind` conviven con las secciones cuando estas cuelgan
        // de la raíz; no son desconocidas.
        if prefijo.is_empty() && matches!(name, "apiVersion" | "kind" | "metadata" | "spec") {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos1005,
                file,
                format!("clave desconocida `{prefijo}{name}`"),
            )
            .at(k.pos())
            .help(format!(
                "si es una extensión de proveedor, declárala como `x-<proveedor>-{name}`; \
                 si no, revisa si es una errata"
            )),
        );
    }
}

/// Valida un paquete entero.
///
/// Dos fases, y el orden es normativo: **no se puede enlazar un paquete que no
/// analiza**. Si la fase de documento encuentra algo, la de enlazado no llega a
/// correr — resolver referencias sobre un árbol que no se pudo construir
/// produciría cascadas de errores derivados en lugar de la causa.
pub fn validate_package(root: &Path) -> Vec<Diagnostic> {
    let (pkg, diags) = cargar_paquete(root);
    if !diags.is_empty() {
        return diags;
    }

    // Enlazado antes que tipos: no se puede comprobar el tipo de una referencia
    // que no resuelve. Es la misma disciplina de fases que impide enlazar un
    // paquete que no analiza.
    let refs = crate::link::link(&pkg);
    if !refs.is_empty() {
        return refs;
    }
    // Los artefactos generados, tras el enlazado: un ciclo de dependencias es
    // una razón mejor para fallar que un lock desincronizado, y de hecho la
    // explica. Antes que tipos porque un esquema Cedar obsoleto invalida la
    // gobernanza entera, no una propiedad.
    let generados = crate::sync::check(&pkg);
    if !generados.is_empty() {
        return generados;
    }
    // Tipos antes que flujo: la propagación transporta etiquetas por el mismo
    // grafo de derivación que los tipos acaban de validar.
    let tipos = crate::types::check(&pkg);
    if !tipos.is_empty() {
        return tipos;
    }
    // Las derivaciones antes que flujo, y el orden importa: `OOS4015` dice que
    // `derivedFrom` declara de menos, y la propagación de flujo usa justo eso.
    // Propagar primero daría etiquetas más bajas de las debidas y el
    // diagnóstico saldría en otro sitio, o en ninguno.
    let derivadas = crate::derivacion::check(&pkg);
    if !derivadas.is_empty() {
        return derivadas;
    }
    let flujo = crate::flow::check(&pkg);
    if !flujo.is_empty() {
        return flujo;
    }
    // Efectos antes que gobierno: la regla de integridad razona sobre
    // etiquetas que la fase de flujo acaba de dar por buenas, y sobre un grafo
    // que ya resuelve.
    let efectos = crate::effect::check(&pkg);
    if !efectos.is_empty() {
        return efectos;
    }
    // Significado despues de flujo y antes de gobierno, y las dos mitades del
    // orden son normativas. Despues de flujo porque `OOS9003` compara contra la
    // madurez EFECTIVA, que es lo que la propagacion acaba de computar. Antes de
    // gobierno porque un objetivo `implements` selecciona por una forma, y
    // acreditar cobertura sobre una forma que no se satisface seria acreditar lo
    // que nadie cumple — el sentido inseguro del error.
    let significado = crate::significado::check(&pkg);
    if !significado.is_empty() {
        return significado;
    }
    // Y el gobierno al final, porque es el único que razona sobre el paquete
    // ENTERO en vez de documento a documento: la cobertura es una diferencia de
    // conjuntos, y calcularla sobre etiquetas que aún podrían estar mal daría
    // una ausencia falsa — el peor diagnóstico posible, porque señala algo que
    // nadie escribió.
    let gobierno = crate::governance::check(&pkg);
    if !gobierno.is_empty() {
        return gobierno;
    }
    // Y las etiquetas que MENCIONA una politica, al final del todo. Son
    // referencias como las demas y fallan igual —apuntan a algo que no
    // existe—, pero llegan aqui y no con el enlazado por una razon que salio
    // ejecutando: si una dependencia no resuelve, su reticulo no se carga y
    // TODAS sus etiquetas parecen inexistentes. Este diagnostico seria entonces
    // la CONSECUENCIA del error real, adelantandolo — y `99-errors` §2.1 dice
    // que gana el codigo especifico.
    crate::politica::check(&pkg)
}

/// Lee un directorio y construye el paquete, con lo que falló al hacerlo.
///
/// Se separa de `validate_package` porque `ore diff` necesita **dos** paquetes
/// y no le interesan sus diagnósticos individuales: compara sus formas. Que
/// cargar y validar sean lo mismo era una coincidencia mientras solo existía
/// una operación.
pub fn cargar_paquete(root: &Path) -> (crate::link::Package, Vec<Diagnostic>) {
    let mut ficheros = Vec::new();
    // Un `.oob` es un paquete entero dentro de un fichero, así que también es
    // una **raíz** y no solo un miembro de un árbol. `ore diff` pregunta qué
    // cambia entre lo que tienes y lo que vendría, y lo que vendría se publica
    // como `.oob`: exigir un directorio obligaba a desempaquetarlo a mano antes
    // de poder preguntar, que es justo el paso que la forma canónica evita.
    if root.extension().is_some_and(|x| x == "oob") {
        ficheros.push(root.to_path_buf());
    } else {
        let fuera = excluidos(root);
        recolectar(root, root, &fuera, &mut ficheros);
        ficheros.sort();
    }

    let mut diags = Vec::new();
    let mut cargados = Vec::new();
    let mut cedar = Vec::new();
    let mut generated = Vec::new();
    let mut sobres = Vec::new();

    for f in ficheros {
        // El esquema Cedar es un artefacto generado: no se valida, se compara
        // contra lo que el paquete implica.
        if f.extension().is_some_and(|x| x == "cedarschema") {
            if let Ok(t) = std::fs::read_to_string(&f) {
                generated.push((f.clone(), t));
            }
            continue;
        }

        if f.extension().is_some_and(|x| x == "cedar") {
            if let Ok(t) = std::fs::read_to_string(&f) {
                cedar.push((f.clone(), t));
            }
            continue;
        }

        let text = match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => {
                diags.push(Diagnostic::new(
                    Code::Oos1001,
                    &f,
                    format!("no se pudo leer: {e}"),
                ));
                continue;
            }
        };
        // `ontology.lock` no es un documento de la ontología: es un artefacto
        // generado, versionado por su propio formato y sin `apiVersion` ni
        // `kind` a propósito. No pasa por la fase de documento, pero sí entra en
        // la de enlazado — el grafo de dependencias vive ahí.
        if f.file_name().is_some_and(|n| n == "ontology.lock") {
            cargados.extend(cargar(&f, &text));
            continue;
        }

        // Un `.oob` es un paquete entero dentro de un fichero, y se abre aquí.
        // Sus documentos ya vienen en forma canónica —los escribió `ore pack`—
        // así que no se vuelven a normalizar: se analizan y entran como
        // cualquier otro.
        if f.extension().is_some_and(|x| x == "oob") {
            let (docs, sobre, d) = abrir_oob(&f, &text);
            cargados.extend(docs);
            if let Some(n) = sobre {
                sobres.push((f.clone(), n));
            }
            diags.extend(d);
            continue;
        }

        let d = validate_document(&f, &text);
        if d.is_empty() {
            cargados.extend(cargar(&f, &text));
        }
        diags.extend(d);
    }

    let pkg = crate::link::Package {
        root: root.to_path_buf(),
        docs: cargados,
        cedar,
        generated,
        sobres,
    };
    (pkg, diags)
}

/// Reanaliza un documento ya validado para la fase de enlazado. `ontology.lock`
/// no lleva `kind` —es un artefacto generado, no un documento de la ontología—
/// y entra como `OntologyConfig` para que el grafo de dependencias sea legible.
fn cargar(path: &Path, text: &str) -> Vec<crate::link::Loaded> {
    let Ok(roots) = parse::parse_stream(text) else {
        return Vec::new();
    };
    let es_lock = path.file_name().is_some_and(|n| n == "ontology.lock");
    roots
        .into_iter()
        .filter_map(|root| {
            let kind = match root
                .get("kind")
                .and_then(|(_, v)| v.as_str())
                .and_then(Kind::parse)
            {
                Some(k) => k,
                None if es_lock => Kind::OntologyConfig,
                None => return None,
            };
            Some(crate::link::Loaded {
                path: path.to_path_buf(),
                kind,
                root,
            })
        })
        .collect()
}

/// Lo que `workspace.exclude` saca de la compilación.
///
/// **Excluido del workspace es no compilado, y por tanto no gobernado**
/// (`02-ruleset` §2.5.3). La especificación se apoya en esa frase para explicar
/// por qué un `Ruleset` no necesita un `scope` de miembros — «un concepto en vez
/// de dos»— y el campo se declaraba sin que nadie lo leyera: un directorio
/// excluido compilaba igual. La alternativa que se descartó por innecesaria no
/// estaba cubierta por lo que se puso en su lugar.
///
/// `members` NO se lee, y no es una omisión. Dice **dónde están los paquetes**,
/// y todo lo que hoy pregunta eso —la atribución de un documento a su miembro en
/// `OOS9004`— lo contesta mejor mirando dónde hay un `package.yaml`: funciona con
/// cualquier disposición sin expandir un patrón, que es justo para lo que existe
/// declararlo. Un motor de globs para un campo que no cambia ningún resultado
/// sería código que solo puede estar mal.
fn excluidos(root: &Path) -> Vec<Vec<String>> {
    let Ok(t) = std::fs::read_to_string(root.join("ontology.config.yaml")) else {
        return Vec::new();
    };
    let Ok(arbol) = crate::parse::parse(&t) else {
        return Vec::new();
    };
    arbol
        .get("workspace")
        .and_then(|(_, w)| w.get("exclude"))
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|i| i.as_str())
        .map(|p| {
            p.split(['/', '\\'])
                .filter(|s| !s.is_empty() && *s != ".")
                .map(String::from)
                .collect()
        })
        .filter(|s: &Vec<String>| !s.is_empty())
        .collect()
}

/// Un patrón excluye a una ruta si es **prefijo** de sus segmentos. Un `*` vale
/// por un segmento entero; dentro de un segmento vale por cualquier trozo.
///
/// Prefijo y no igualdad porque excluir un directorio excluye lo que hay dentro:
/// esa es la unidad con la que alguien piensa cuando escribe `exclude`.
fn excluida(patrones: &[Vec<String>], rel: &Path) -> bool {
    let segmentos: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    patrones.iter().any(|p| {
        p.len() <= segmentos.len() && p.iter().zip(&segmentos).all(|(pat, seg)| casa(pat, seg))
    })
}

fn casa(patron: &str, segmento: &str) -> bool {
    match patron.split_once('*') {
        None => patron == segmento,
        Some((antes, despues)) => {
            segmento.len() >= antes.len() + despues.len()
                && segmento.starts_with(antes)
                && segmento.ends_with(despues)
        }
    }
}

/// Abre un `.oob`: un paquete publicado, dentro de un fichero.
///
/// # La ruta sintética, que hace más de lo que parece
///
/// Cada documento entra con la ruta `<el .oob>/<su identidad>`, y esa barra no
/// es cosmética: hace que **el `.oob` sea el directorio de sus documentos**. La
/// atribución de un documento a su miembro del workspace —la que decide a quién
/// se le exige que hable un concepto, `OOS9004`— mira el ancestro que tiene un
/// `Package`, y con esta forma un paquete importado es su propio miembro sin que
/// esa regla se entere de que aquí no hay directorio.
///
/// # Por qué no se verifica aquí
///
/// Comprobar que lo que hay es lo que el lock nombra es de `sync`, no del
/// cargador: cargar es leer, y un fichero que no se pueda analizar ya falla por
/// sí solo. Mezclar las dos cosas daría un cargador que decide qué se puede
/// usar, que es exactamente la clase de decisión que este proyecto pone donde se
/// pueda revisar.
///
/// Por eso el sobre sale de aquí **entero y sin interpretar**, junto a los
/// documentos: la firma se comprueba donde estan el lock y las claves que el
/// consumidor declara, que es el unico sitio donde se puede contrastar con algo
/// en vez de creersela.
fn abrir_oob(
    f: &Path,
    texto: &str,
) -> (
    Vec<crate::link::Loaded>,
    Option<crate::parse::Node>,
    Vec<Diagnostic>,
) {
    let mut docs = Vec::new();
    let Ok(raiz) = crate::parse::parse(texto) else {
        return (
            docs,
            None,
            vec![
                Diagnostic::new(Code::Oos1001, f, "no analiza como JSON".to_string()).help(
                    "un `.oob` es la forma canónica en JCS, tal y como la escribe `ore pack`",
                ),
            ],
        );
    };
    let Some((_, documentos)) = raiz.get("documents") else {
        return (
            docs,
            None,
            vec![
                Diagnostic::new(Code::Oos1002, f, "no hay `documents`".to_string())
                    .help("un `.oob` sin documentos es un fichero que nadie puede importar"),
            ],
        );
    };
    for (id, doc) in documentos.entries() {
        let Some(id) = id.as_str() else { continue };
        let Some(kind) = doc
            .get("kind")
            .and_then(|(_, k)| k.as_str())
            .and_then(Kind::parse)
        else {
            continue;
        };
        docs.push(crate::link::Loaded {
            // La barra se escapa, y hace falta: el `docId` de un `Package`
            // publicado lleva su coordenada dentro —`Package:oos.dev/…/gdpr`— y
            // sin escaparla la ruta sintética se parte en tres componentes. El
            // ancestro dejaba de ser el `.oob`, la atribución de miembro no
            // encontraba el `Package`, y catorce conceptos importados salían
            // como vocabulario muerto. Se midió con el primero que se importó.
            path: f.join(id.replace('/', "%2f")),
            kind,
            root: doc.clone(),
        });
    }
    // El sobre se devuelve ENTERO y sin mirar. Lo que dice de si mismo
    // —incluidas sus firmas— es materia de `sync`, que es donde se puede
    // contrastar con lo que el consumidor declara y con su lock.
    (docs, Some(raiz), Vec::new())
}

fn recolectar(root: &Path, dir: &Path, fuera: &[Vec<String>], out: &mut Vec<PathBuf>) {
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entradas.flatten() {
        let p = e.path();
        if !fuera.is_empty() && p.strip_prefix(root).is_ok_and(|rel| excluida(fuera, rel)) {
            continue;
        }
        if p.is_dir() {
            // Un directorio oculto no forma parte del paquete. La regla es
            // general y no una lista: `.ore/` es caché derivada, `.github/` y
            // `.gitlab/` son maquinaria del repositorio, `.git/` y `.vscode/`
            // no son de nadie. Ninguno contiene documentos de la ontología, y
            // enumerarlos uno a uno solo aplaza el siguiente.
            //
            // Sin esto, un repositorio no puede guardar su propio CI junto a
            // la ontología que valida: `ore validate` entraría en
            // `.github/workflows/*.yml` y exigiría `apiVersion` a un workflow.
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            recolectar(root, &p, fuera, out);
        } else if p.extension().is_some_and(|x| x == "yaml" || x == "yml")
            || p.extension()
                .is_some_and(|x| x == "cedar" || x == "cedarschema" || x == "oob")
            || p.file_name().is_some_and(|n| n == "ontology.lock")
        {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codigos(text: &str) -> Vec<Code> {
        validate_document(Path::new("t.yaml"), text)
            .iter()
            .map(|d| d.code)
            .collect()
    }

    const ENTIDAD_OK: &str = "apiVersion: oos.dev/v1alpha1\nkind: Entity\n\
        metadata: { name: Employee, namespace: hr }\n\
        spec:\n  nature: entity\n  primaryKey: [id]\n  properties:\n    id: { type: String }\n";

    #[test]
    fn un_documento_correcto_no_produce_diagnosticos() {
        assert!(codigos(ENTIDAD_OK).is_empty());
    }

    #[test]
    fn el_despacho_precede_a_la_validacion() {
        // kind desconocido Y clave desconocida: debe ganar el despacho, porque
        // sin kind no hay contra qué validar las claves.
        let t = "apiVersion: oos.dev/v1alpha1\nkind: Ontology\nzzz: 1\n";
        assert_eq!(codigos(t), vec![Code::Oos1003]);
    }

    #[test]
    fn extension_de_proveedor_admitida() {
        let t = ENTIDAD_OK.replace(
            "  nature: entity",
            "  x-acme-owner: plataforma\n  nature: entity",
        );
        assert!(codigos(&t).is_empty(), "{:?}", codigos(&t));
    }
}
