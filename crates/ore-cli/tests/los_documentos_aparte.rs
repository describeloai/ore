//! **Los documentos aparte** (0079 §19.2 ·5c·5a y ·5c·5b): con
//! `--documentos-aparte`, los verbos que escriben (`--crear`, `--commit`,
//! `--confirmar`, `--publicar-retenida`) escriben **el mismo puntero** por la
//! puerta, **no tocan ningún documento del árbol**, y devuelven en
//! `documentos: [{ruta, texto}]` exactamente lo que sin la bandera escribirían.
//!
//! Dos árboles gemelos: `a` sin la bandera (lo de hoy) y `b` con ella. Tras cada
//! paso, lo que `b` devolvió se le escribe a mano (lo que hará la fase 2 de
//! ore-serve), así los dos siguen siendo el mismo árbol. El lago es un
//! `ore-store-r2` de mentira en el `PATH` del hijo. Con `ORE_PUNTEROS` puesto
//! (la suite por la puerta de proceso) corre igual: los punteros siguen en el
//! árbol, por eso la foto de los documentos deja fuera `datasets/` (la carpeta
//! de los punteros) y los punteros se comparan aparte.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ore_core::json::Json;

fn tmp(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "ore-aparte-{nombre}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Un árbol con el paquete `ventas` (y una vista, que el árbol compile algo).
fn arbol(d: &Path) {
    std::fs::create_dir_all(d.join("packages/ventas")).unwrap();
    std::fs::write(
        d.join("ontology.config.yaml"),
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: x, version: 0.1.0 }\n",
    )
    .unwrap();
    std::fs::write(
        d.join("packages/ventas/package.yaml"),
        "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 0.1.0, status: active, domain: ventas }\nspec: { owner: team:ventas }\n",
    )
    .unwrap();
}

/// **El lago de mentira**: `ore-store-r2` que contesta a `aplicar` con las
/// columnas de `columnas.json` (al lado) y un `metadata.json` nuevo cada vez.
fn lago(d: &Path) -> PathBuf {
    let bin = d.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let guion = format!(
        r#"#!/bin/bash
cat > /dev/null
D='{d}'
case "$1" in
  aplicar)
    n=$(cat "$D/n" 2>/dev/null || echo 0); n=$((n+1)); echo "$n" > "$D/n"
    printf '{{"metadata_location":"gs://b/ore/v2/catalogo/ventas/default/t/metadata/%05d-x.metadata.json","snapshot":"%s","filas":"3","uuid":"u-1","operacion":"anexar","columnas_oos":%s}}\n' "$n" "$n" "$(cat "$D/columnas.json")"
    ;;
  buscar) echo '{{"existe":"true"}}' ;;
  historia) echo '{{"snapshots":[]}}' ;;
  metadatos) echo '{{}}' ;;
  *) echo "no sé $1" >&2; exit 2 ;;
esac
"#,
        d = d.display()
    );
    let p = bin.join("ore-store-r2");
    std::fs::write(&p, guion).unwrap();
    Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    bin
}

struct Gemelos {
    raiz: PathBuf,
    a: PathBuf,
    b: PathBuf,
    bin: PathBuf,
}

impl Gemelos {
    fn nuevos(nombre: &str) -> Gemelos {
        let raiz = tmp(nombre);
        let (a, b) = (raiz.join("a"), raiz.join("b"));
        arbol(&a);
        arbol(&b);
        let bin = lago(&raiz);
        Gemelos { raiz, a, b, bin }
    }

    fn columnas(&self, c: &str) {
        std::fs::write(self.raiz.join("columnas.json"), c).unwrap();
    }

    fn ore(&self, dir: &Path, args: &[&str]) -> (i32, String, String) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let s = Command::new(env!("CARGO_BIN_EXE_ore"))
            .args(args)
            .current_dir(dir)
            .env("PATH", path)
            .env_remove("ORE_STORE")
            .output()
            .unwrap();
        (
            s.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&s.stdout).into_owned(),
            String::from_utf8_lossy(&s.stderr).into_owned(),
        )
    }

    /// ⭐ **Un paso**: el verbo en `a` (lo de hoy) y en `b` con la bandera. Lo
    /// comprueba todo y devuelve las rutas de lo que `b` devolvió.
    fn paso(&self, args: &[&str], tabla: &str) -> Vec<String> {
        let antes = foto(&self.b);
        // El lago contesta lo mismo a los dos: el contador va por paso.
        let n = std::fs::read_to_string(self.raiz.join("n")).unwrap_or_default();
        let (ca, sa, ea) = self.ore(&self.a, args);
        std::fs::write(self.raiz.join("n"), &n).unwrap();
        let mut con: Vec<&str> = args.to_vec();
        con.push("--documentos-aparte");
        let (cb, sb, eb) = self.ore(&self.b, &con);
        assert_eq!(ca, 0, "{args:?}\n{sa}\n{ea}");
        assert_eq!(cb, 0, "{con:?}\n{sb}\n{eb}");
        // ⛔ El árbol de `b`, sin un documento escrito ni un fichero de más.
        assert_eq!(
            foto(&self.b),
            antes,
            "--documentos-aparte tocó el árbol: {con:?}"
        );
        // El mismo puntero (sin la hora del intento).
        assert_eq!(puntero(&self.a, tabla), puntero(&self.b, tabla), "{args:?}");
        assert!(puntero(&self.b, tabla).is_some(), "{args:?}: sin puntero");
        // Lo devuelto, exactamente lo que `a` escribió de nuevo o cambió.
        let j = ultima(&sb);
        let docs: Vec<(String, String)> = match j.get("documentos") {
            Some((_, d)) => d
                .items()
                .iter()
                .map(|x| {
                    let c = |k: &str| x.get(k).and_then(|(_, v)| v.as_str()).unwrap().to_string();
                    (c("ruta"), c("texto"))
                })
                .collect(),
            None => panic!("sin `documentos` con la bandera: {sb}"),
        };
        let cambiados: Vec<(String, String)> = foto(&self.a)
            .into_iter()
            .filter(|(r, t)| antes.get(r) != Some(t))
            .collect();
        assert_eq!(docs, cambiados, "{args:?}");
        // Y sin la bandera, la salida no dice `documentos`.
        assert!(ultima(&sa).get("documentos").is_none(), "{sa}");
        // La fase 2, a mano: `b` vuelve a ser el gemelo de `a`.
        for (r, t) in &docs {
            let p = self.b.join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        assert_eq!(foto(&self.a), foto(&self.b));
        docs.into_iter().map(|(r, _)| r).collect()
    }
}

impl Drop for Gemelos {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.raiz);
    }
}

/// Los ficheros del árbol (ruta → texto), sin `datasets/` (los punteros).
fn foto(d: &Path) -> BTreeMap<String, String> {
    fn ir(raiz: &Path, d: &Path, out: &mut BTreeMap<String, String>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            let rel = p
                .strip_prefix(raiz)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                if rel != "datasets" {
                    ir(raiz, &p, out);
                }
            } else {
                out.insert(rel, std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    ir(d, d, &mut out);
    out
}

/// El puntero de `ventas.<t>`, sin `intento` (lleva la hora).
fn puntero(d: &Path, t: &str) -> Option<Json> {
    let texto =
        std::fs::read_to_string(d.join(format!("datasets/ventas/default/{t}.json"))).ok()?;
    match Json::de_node_fiel(&ore_core::parse::parse(&texto).unwrap()) {
        Json::Obj(mut m) => {
            m.remove("intento");
            Some(Json::Obj(m))
        }
        otro => Some(otro),
    }
}

fn ultima(stdout: &str) -> ore_core::parse::Node {
    let l = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("sin JSON: {stdout}"));
    ore_core::parse::parse(l).unwrap()
}

const SUJETO: &str = "user:ana";

fn commit(t: &str) -> String {
    format!(
        r#"{{"identifier":{{"namespace":["ventas"],"name":"{t}"}},"requirements":[],"updates":[]}}"#
    )
}

/// ⭐ Cada verbo, con y sin la bandera: el mismo puntero, el árbol intacto, y el
/// documento devuelto exactamente cuando hoy se escribiría.
#[test]
fn cada_verbo_escribe_el_mismo_puntero_y_devuelve_el_documento_en_vez_de_escribirlo() {
    let g = Gemelos::nuevos("verbos");
    let doc = "packages/ventas/datasets/salida.yaml".to_string();

    // `--crear`: la tabla nace, y su `Dataset` con ella.
    g.columnas(r#"{"id":"Integer"}"#);
    let r = g.paso(
        &[
            "datasets",
            ".",
            "--crear",
            "ventas.salida",
            "--peticion",
            r#"{"name":"salida"}"#,
            "--sujeto",
            SUJETO,
            "--owner",
            SUJETO,
            "--json",
        ],
        "salida",
    );
    assert_eq!(r, vec![doc.clone()], "nace la tabla: su documento");

    // `--commit` con las mismas columnas: solo puntero.
    let p = commit("salida");
    let args_commit = [
        "datasets",
        ".",
        "--commit",
        "--tabla",
        "ventas.salida",
        "--peticion",
        &p,
        "--sujeto",
        SUJETO,
        "--json",
    ];
    let r = g.paso(&args_commit, "salida");
    assert!(r.is_empty(), "las mismas columnas no regeneran: {r:?}");

    // `--commit` con una columna nueva: la mixta.
    g.columnas(r#"{"id":"Integer","pais":"String"}"#);
    let r = g.paso(&args_commit, "salida");
    assert_eq!(r, vec![doc.clone()], "el esquema evolucionó");

    // `--commitTransaction` de dos tablas, una que nace y otra que no cambia.
    g.columnas(r#"{"id":"Integer","pais":"String"}"#);
    let trans = format!(
        r#"{{"table-changes":[{},{{"identifier":{{"namespace":["ventas"],"name":"nueva"}},"requirements":[{{"type":"assert-create"}}],"updates":[]}}]}}"#,
        commit("salida")
    );
    let r = g.paso(
        &[
            "datasets",
            ".",
            "--commit",
            "--peticion",
            &trans,
            "--sujeto",
            SUJETO,
            "--owner",
            SUJETO,
            "--json",
        ],
        "nueva",
    );
    assert_eq!(r, vec!["packages/ventas/datasets/nueva.yaml".to_string()]);

    // `--confirmar`: nace con columnas (mixta) y luego se mueve (solo puntero).
    g.columnas(r#"{"id":"Integer"}"#);
    let ml1 = "gs://b/ore/v2/catalogo/ventas/default/swap/metadata/00001-x.metadata.json";
    let r = g.paso(
        &[
            "datasets",
            ".",
            "--confirmar",
            "ventas.swap",
            "--metadata-location",
            ml1,
            "--columnas",
            r#"{"id":"Integer"}"#,
            "--sujeto",
            SUJETO,
            "--owner",
            SUJETO,
            "--json",
        ],
        "swap",
    );
    assert_eq!(r, vec!["packages/ventas/datasets/swap.yaml".to_string()]);
    let r = g.paso(
        &[
            "datasets",
            ".",
            "--confirmar",
            "ventas.swap",
            "--metadata-location",
            "gs://b/ore/v2/catalogo/ventas/default/swap/metadata/00002-x.metadata.json",
            "--esperado",
            ml1,
            "--sujeto",
            SUJETO,
            "--json",
        ],
        "swap",
    );
    assert!(r.is_empty(), "mover el puntero no toca el documento: {r:?}");

    // `--publicar-retenida`: lo retenido trae otra columna, y el documento la sigue.
    for d in [&g.a, &g.b] {
        let f = d.join("datasets/ventas/default/salida.json");
        let mut m = match Json::de_node_fiel(
            &ore_core::parse::parse(&std::fs::read_to_string(&f).unwrap()).unwrap(),
        ) {
            Json::Obj(m) => m,
            _ => unreachable!(),
        };
        m.insert(
            "version_retenida".into(),
            Json::obj([
                (
                    "metadata_location",
                    Json::s("gs://b/ore/v2/catalogo/ventas/default/salida/metadata/00009-x.metadata.json"),
                ),
                ("snapshot", Json::s("9")),
                ("filas", Json::Int(0)),
                ("operacion", Json::s("sobrescribir")),
                ("escrito_por", Json::s(SUJETO)),
                (
                    "columnas",
                    Json::obj([
                        ("id", Json::s("Integer")),
                        ("pais", Json::s("String")),
                        ("total", Json::s("Float")),
                    ]),
                ),
            ]),
        );
        std::fs::write(&f, Json::Obj(m).jcs()).unwrap();
    }
    let r = g.paso(
        &[
            "datasets",
            ".",
            "--publicar-retenida",
            "ventas.salida",
            "--sujeto",
            SUJETO,
            "--json",
        ],
        "salida",
    );
    assert_eq!(r, vec![doc]);
    assert!(
        std::fs::read_to_string(g.b.join("packages/ventas/datasets/salida.yaml"))
            .unwrap()
            .contains("total"),
        "el documento sigue lo publicado"
    );
}

/// Lo que no compila no se devuelve: el error de siempre, y el árbol intacto.
#[test]
fn un_documento_que_no_compila_es_el_error_de_siempre_y_no_toca_nada() {
    let g = Gemelos::nuevos("roto");
    // Una columna con un tipo que OOS no conoce.
    g.columnas(r#"{"id":"Integer","x":"NoEsUnTipo"}"#);
    let antes = foto(&g.b);
    let args = [
        "datasets",
        ".",
        "--crear",
        "ventas.salida",
        "--peticion",
        r#"{"name":"salida"}"#,
        "--sujeto",
        SUJETO,
        "--owner",
        SUJETO,
        "--json",
    ];
    let (ca, _, _) = g.ore(&g.a, &args);
    let mut con = args.to_vec();
    con.push("--documentos-aparte");
    let (cb, sb, _) = g.ore(&g.b, &con);
    assert_eq!(ca, cb, "el mismo código con y sin la bandera");
    assert_ne!(cb, 0);
    assert!(!sb.contains("documentos"), "{sb}");
    assert_eq!(foto(&g.b), antes);
}

/// La bandera sólo va con un verbo que escribe y con `--json`.
#[test]
fn la_bandera_va_con_un_verbo_que_escribe_y_json() {
    let g = Gemelos::nuevos("bandera");
    for args in [
        vec!["datasets", ".", "--json", "--documentos-aparte"],
        vec![
            "datasets",
            ".",
            "--cargar",
            "ventas.x",
            "--json",
            "--documentos-aparte",
        ],
        vec![
            "datasets",
            ".",
            "--crear",
            "ventas.x",
            "--peticion",
            "{}",
            "--documentos-aparte",
        ],
    ] {
        let (c, _, e) = g.ore(&g.b, &args);
        assert_eq!(c, 64, "{args:?}: {e}");
        assert!(e.contains("--documentos-aparte"), "{e}");
    }
}

/// ⭐ **La clave de idempotencia llega a la puerta** (ADR 0031): lo que viaja por
/// el protocolo (una puerta de proceso con un espía delante) lleva
/// `contexto.idempotencia`, en el commit y en el crear.
#[test]
fn la_clave_de_idempotencia_llega_al_contexto_de_la_puerta() {
    let g = Gemelos::nuevos("idem");
    let log = g.raiz.join("protocolo.log");
    let espia = g.raiz.join("espia.sh");
    std::fs::write(
        &espia,
        format!(
            "#!/bin/bash\ntee -a '{}' | '{}' punteros-servir\n",
            log.display(),
            env!("CARGO_BIN_EXE_ore")
        ),
    )
    .unwrap();
    let path = format!(
        "{}:{}",
        g.bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let correr = |args: &[&str]| {
        let s = Command::new(env!("CARGO_BIN_EXE_ore"))
            .args(args)
            .current_dir(&g.b)
            .env("PATH", &path)
            .env("ORE_PUNTEROS", format!("/bin/bash {}", espia.display()))
            .env_remove("ORE_STORE")
            .output()
            .unwrap();
        assert!(
            s.status.success(),
            "{args:?}\n{}",
            String::from_utf8_lossy(&s.stderr)
        );
    };
    g.columnas(r#"{"id":"Integer"}"#);
    let k1 = "0192f0c4-7b2a-7c3e-9d41-5a6b7c8d9e01";
    let k2 = "0192f0c4-7b2a-7c3e-9d41-5a6b7c8d9e02";
    correr(&[
        "datasets",
        ".",
        "--crear",
        "ventas.salida",
        "--peticion",
        r#"{"name":"salida"}"#,
        "--sujeto",
        SUJETO,
        "--owner",
        SUJETO,
        "--json",
        "--documentos-aparte",
        &format!("--idempotencia={k1}"),
    ]);
    let p = commit("salida");
    correr(&[
        "datasets",
        ".",
        "--commit",
        "--tabla",
        "ventas.salida",
        "--peticion",
        &p,
        "--sujeto",
        SUJETO,
        "--json",
        "--documentos-aparte",
        &format!("--idempotencia={k2}"),
    ]);
    let oido = std::fs::read_to_string(&log).unwrap();
    let escrituras: Vec<&str> = oido
        .lines()
        .filter(|l| l.contains(r#""op":"escribir""#))
        .collect();
    assert_eq!(escrituras.len(), 2, "{oido}");
    assert!(
        escrituras[0].contains(&format!(r#""idempotencia":"{k1}""#))
            && escrituras[0].contains(r#""operacion":"crear""#),
        "{}",
        escrituras[0]
    );
    assert!(
        escrituras[1].contains(&format!(r#""idempotencia":"{k2}""#))
            && escrituras[1].contains(r#""operacion":"commit""#),
        "{}",
        escrituras[1]
    );
}
