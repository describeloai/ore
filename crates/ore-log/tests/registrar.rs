//! El contrato del log, visto desde fuera.
//!
//! Vive aquí porque es donde `cargo` garantiza que su binario exista, igual que
//! la del obtenedor y la del firmador. Lo que **comprueba** una prueba se prueba
//! en `ore-core` y sin binarios: es la mitad que tiene que funcionar aunque
//! nadie tenga un log a mano.

use ore_core::transparencia as t;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SEMILLA: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const ID: &str = "oos.dev/log";

fn log(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ore-log-dir-{nombre}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("log.key"), SEMILLA).unwrap();
    d
}

fn correr(dir: &PathBuf, args: &[&str], entrada: Option<&str>) -> Output {
    let mut hijo = Command::new(env!("CARGO_BIN_EXE_ore-log"))
        .args(args)
        .env("ORE_LOG_DIR", dir)
        .env("ORE_LOG_ID", ID)
        .stdin(if entrada.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("no se pudo invocar `ore-log`");
    if let Some(x) = entrada {
        hijo.stdin.take().unwrap().write_all(x.as_bytes()).unwrap();
    }
    hijo.wait_with_output().unwrap()
}

fn salida(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// El campo `k` de una respuesta, sin analizar JSON a mano.
fn campo(o: &Output, k: &str) -> String {
    let r = ore_core::parse::parse(&String::from_utf8_lossy(&o.stdout))
        .unwrap_or_else(|e| panic!("no devolvió JSON: {e:?}\n{}", salida(o)));
    r.get(k)
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("falta `{k}` en {}", salida(o)))
        .to_string()
}

fn hashes(o: &Output, k: &str) -> Vec<t::Hash> {
    let r = ore_core::parse::parse(&String::from_utf8_lossy(&o.stdout)).unwrap();
    r.get(k)
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|h| h.as_str().and_then(t::de_hex))
        .collect()
}

fn anotar(dir: &PathBuf, texto: &str) -> Output {
    let p = format!(
        "{{\"entry\":{},\"op\":\"append\"}}",
        ore_core::json::Json::s(texto).jcs()
    );
    let o = correr(dir, &[], Some(&p));
    assert!(o.status.success(), "{}", salida(&o));
    o
}

fn publica(dir: &PathBuf) -> String {
    let o = correr(dir, &["--public"], None);
    assert!(o.status.success(), "{}", salida(&o));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Anotar y probar son **un solo hecho**, y por eso vienen en el mismo viaje:
/// quien anota necesita la prueba contra el árbol en el que acaba de entrar, y
/// pedirla aparte dejaría una ventana en la que el árbol ya creció.
#[test]
fn anotar_devuelve_la_prueba_del_arbol_en_el_que_entro() {
    let d = log("anota");
    for i in 0..5u32 {
        let o = anotar(&d, &format!("entrada {i}"));
        let indice: u64 = campo(&o, "index").parse().unwrap();
        let tamano: u64 = campo(&o, "treeSize").parse().unwrap();
        assert_eq!((indice, tamano), (i as u64, i as u64 + 1));

        let raiz = t::de_hex(&campo(&o, "root")).unwrap();
        // La cabeza, firmada por el log y nombrando su tamaño: sin ella una raíz
        // es un número que alguien escribió.
        assert_eq!(
            ore_core::firma::verificar(
                ore_core::firma::ED25519,
                &publica(&d),
                &campo(&o, "rootSignature"),
                &t::cabeza(ID, tamano, &raiz),
            ),
            Ok(())
        );
        assert_eq!(
            t::inclusion(
                &t::hoja(format!("entrada {i}").as_bytes()),
                indice,
                tamano,
                &hashes(&o, "inclusion"),
                &raiz,
            ),
            Ok(())
        );
    }
}

/// **Solo crece.** Es lo único que un log garantiza, y lo demuestra para cada
/// par de tamaños por los que ha pasado — no solo hasta el actual, porque quien
/// consume compara con la cabeza que anotó y esa puede ser cualquiera.
#[test]
fn prueba_que_extiende_cualquier_tamano_anterior() {
    let d = log("crece");
    let mut raices = vec![];
    for i in 0..8u32 {
        let o = anotar(&d, &format!("entrada {i}"));
        raices.push(t::de_hex(&campo(&o, "root")).unwrap());
    }
    for antes in 1..=8u64 {
        for ahora in antes..=8u64 {
            let p = format!("{{\"from\":{antes},\"op\":\"consistency\",\"to\":{ahora}}}");
            let o = correr(&d, &[], Some(&p));
            assert!(o.status.success(), "{}", salida(&o));
            assert_eq!(
                t::consistencia(
                    antes,
                    &raices[antes as usize - 1],
                    ahora,
                    &raices[ahora as usize - 1],
                    &hashes(&o, "consistency"),
                ),
                Ok(()),
                "{antes} → {ahora}"
            );
        }
    }
}

/// Una entrada anotada se puede volver a probar más tarde, contra el árbol de
/// entonces y no contra el de cuando entró.
#[test]
fn una_entrada_vieja_se_prueba_contra_el_arbol_de_ahora() {
    let d = log("vieja");
    anotar(&d, "la primera");
    for i in 0..4 {
        anotar(&d, &format!("relleno {i}"));
    }
    let p = format!(
        "{{\"entry\":{},\"op\":\"inclusion\"}}",
        ore_core::json::Json::s("la primera").jcs()
    );
    let o = correr(&d, &[], Some(&p));
    assert!(o.status.success(), "{}", salida(&o));
    assert_eq!(campo(&o, "treeSize"), "5");
    assert_eq!(
        t::inclusion(
            &t::hoja(b"la primera"),
            campo(&o, "index").parse().unwrap(),
            5,
            &hashes(&o, "inclusion"),
            &t::de_hex(&campo(&o, "root")).unwrap(),
        ),
        Ok(())
    );
}

/// **Dos veces lo mismo son dos entradas**, y eso es correcto: que alguien
/// publique dos veces la misma versión es justo la clase de hecho que un log
/// existe para dejar por escrito. Deduplicar lo borraría.
#[test]
fn anotar_dos_veces_lo_mismo_deja_dos_entradas() {
    let d = log("repetida");
    anotar(&d, "la misma");
    let o = anotar(&d, "la misma");
    assert_eq!(campo(&o, "treeSize"), "2");
    assert_eq!(campo(&o, "index"), "1");
}

/// Lo que no está no se prueba, y se dice en vez de devolver una prueba vacía —
/// que verificaría contra un árbol de una hoja y sería la peor respuesta
/// posible.
#[test]
fn lo_que_no_esta_en_el_log_no_se_prueba() {
    let d = log("ausente");
    anotar(&d, "algo");
    let p = format!(
        "{{\"entry\":{},\"op\":\"inclusion\"}}",
        ore_core::json::Json::s("otra cosa").jcs()
    );
    let o = correr(&d, &[], Some(&p));
    assert!(!o.status.success(), "probó algo que no anotó");
    assert!(salida(&o).contains("no esta en el log"), "{}", salida(&o));
}

/// Un log no encoge, y pedirle el árbol de un tamaño que nunca tuvo es una
/// pregunta sin respuesta — no una respuesta vacía.
#[test]
fn no_se_puede_pedir_un_arbol_que_el_log_no_ha_tenido() {
    let d = log("encoge");
    anotar(&d, "una");
    let o = correr(
        &d,
        &[],
        Some("{\"from\":1,\"op\":\"consistency\",\"to\":9}"),
    );
    assert!(!o.status.success(), "{}", salida(&o));
}

/// Por **stdin** y no por `argv`, igual que el obtenedor y el firmador.
#[test]
fn sin_peticion_por_stdin_no_hace_nada() {
    let o = correr(&log("vacio"), &[], Some(""));
    assert!(!o.status.success());
    assert!(salida(&o).contains("stdin"), "{}", salida(&o));
}

/// Y sin `ORE_LOG_DIR` lo dice, en vez de escribir un log donde le parezca.
#[test]
fn sin_directorio_lo_dice() {
    let o = Command::new(env!("CARGO_BIN_EXE_ore-log"))
        .arg("--public")
        .env_remove("ORE_LOG_DIR")
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(salida(&o).contains("ORE_LOG_DIR"), "{}", salida(&o));
}

// ── Y la mitad delegada: `ore lock` avanzando la cabeza ─────────────────────

/// Los tres binarios, donde `cargo` los deja.
///
/// `CARGO_BIN_EXE_x` solo existe para la crate que declara el binario, así que
/// los otros dos se buscan al lado. Si faltan, se dice en vez de saltarse la
/// prueba: una que se salta sola en CI no prueba nada.
fn vecino(nombre: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_BIN_EXE_ore-log"))
        .with_file_name(format!("{nombre}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        p.is_file(),
        "falta `{}`. Esta prueba mide la delegación entre binarios, así que hacen falta \
         todos: `cargo test --workspace` los construye.",
        p.display()
    );
    p
}

/// **La mitad de la transparencia que no cabe en un paquete.**
///
/// Quien publica no sabe qué cabeza viste tú la última vez, así que la prueba de
/// que el log **extiende** lo que ya viste no puede viajar dentro del `.oob`. Se
/// pide, como se pide un paquete — y por eso vive en `lock` y no en `validate`:
/// aquí se delega, allí no se toca nada de fuera.
///
/// Sin esto la garantía sería *«el log dijo esto»*, que es lo que dice cualquier
/// firma. Lo que lo convierte en *«y no ha dicho nunca otra cosa»* es esto.
#[test]
fn subir_de_version_avanza_la_cabeza_con_prueba_de_consistencia() {
    let dir = std::env::temp_dir().join("ore-log-delegacion");
    let _ = std::fs::remove_dir_all(&dir);
    let (claves, registro, arbol) = (dir.join("claves"), dir.join("registro"), dir.join("arbol"));
    let bitacora = dir.join("log");
    for d in [&claves, &registro, &arbol, &bitacora] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(claves.join("oos.dev.key"), "7".repeat(64)).unwrap();
    std::fs::write(bitacora.join("log.key"), SEMILLA).unwrap();

    let escribir = |p: PathBuf, t: &str| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    let publicar = |version: &str, fichero: &str| {
        let fuente = dir.join(format!("fuente-{version}"));
        escribir(
            fuente.join("package.yaml"),
            &format!(
                "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata:\n  \
                 name: oos.dev/regulatory/gdpr\n  version: {version}\n  status: draft\n  \
                 domain: compliance\nspec: {{ owner: \"team:compliance\" }}\n"
            ),
        );
        escribir(
            fuente.join("concepts/dateOfBirth.yaml"),
            "apiVersion: oos.dev/v1alpha4\nkind: Property\n\
             metadata: { name: dateOfBirth, namespace: gdpr }\n\
             spec:\n  type: Date\n  description: La fecha de nacimiento.\n",
        );
        let o = Command::new(vecino("ore"))
            .args([
                "pack",
                fuente.to_str().unwrap(),
                "-o",
                &registro.join(fichero).to_string_lossy(),
                "--sign",
                "oos.dev",
                "--log",
                ID,
            ])
            .env("ORE_SIGN_DIR", &claves)
            .env("ORE_LOG_DIR", &bitacora)
            .env("ORE_LOG_ID", ID)
            .env("PATH", con_binarios())
            .output()
            .expect("no se pudo invocar `ore`");
        assert!(o.status.success(), "{}", salida(&o));
    };

    let firmante = {
        let o = Command::new(vecino("ore-sign"))
            .args(["--public", "oos.dev"])
            .env("ORE_SIGN_DIR", &claves)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    escribir(
        arbol.join("ontology.config.yaml"),
        &format!(
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
             metadata: {{ name: rrhh, version: 0.1.0 }}\n\
             dependencies:\n  - {{ package: oos.dev/regulatory/gdpr, version: \"^0.1\" }}\n\
             trustedKeys:\n  - {{ id: oos.dev, algorithm: ed25519, publicKey: \"{firmante}\" }}\n\
             trustedLogs:\n  - {{ id: \"{ID}\", algorithm: ed25519, publicKey: \"{}\" }}\n",
            publica(&bitacora)
        ),
    );

    let resolver = || {
        Command::new(vecino("ore"))
            .args(["lock", arbol.to_str().unwrap()])
            .env("ORE_FETCH_DIR", &registro)
            .env("ORE_LOG_DIR", &bitacora)
            .env("ORE_LOG_ID", ID)
            .env("PATH", con_binarios())
            .output()
            .expect("no se pudo invocar `ore`")
    };

    publicar("0.1.0", "gdpr-0.1.0.oob");
    let o = resolver();
    assert!(o.status.success(), "{}", salida(&o));
    let lock = std::fs::read_to_string(arbol.join("ontology.lock")).unwrap();
    assert!(lock.contains("treeSize: 1"), "{lock}");
    assert!(lock.contains(&format!("logged: [{ID}]")), "{lock}");

    // El log crece —dos entradas más, una de ellas la 0.2.0— y el consumidor
    // sube. La cabeza vieja solo se puede abandonar demostrando que la nueva la
    // extiende.
    anotar(&bitacora, "otro paquete cualquiera");
    publicar("0.2.0", "gdpr-0.2.0.oob");
    let c = arbol.join("ontology.config.yaml");
    let t = std::fs::read_to_string(&c).unwrap().replace("^0.1", "^0.2");
    std::fs::write(&c, t).unwrap();

    let o = resolver();
    assert!(o.status.success(), "{}", salida(&o));
    let lock = std::fs::read_to_string(arbol.join("ontology.lock")).unwrap();
    assert!(lock.contains("version: 0.2.0"), "{lock}");
    assert!(
        lock.contains("treeSize: 3"),
        "la cabeza no avanzó al árbol de 3:\n{lock}"
    );

    // Y con la cabeza ya avanzada, compilar es hermético: no se vuelve a
    // preguntar a nadie.
    let v = Command::new(vecino("ore"))
        .args(["validate", arbol.to_str().unwrap()])
        .env_remove("ORE_LOG_DIR")
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(v.status.success(), "{}", salida(&v));
}

/// El `PATH` con los binarios delante: es como `ore` encuentra a los delegados,
/// y el contrato dice *«un programa del usuario en el PATH»*.
fn con_binarios() -> std::ffi::OsString {
    let bin = Path::new(env!("CARGO_BIN_EXE_ore-log")).parent().unwrap();
    match std::env::var_os("PATH") {
        Some(p) => {
            let mut dirs = vec![bin.to_path_buf()];
            dirs.extend(std::env::split_paths(&p));
            std::env::join_paths(dirs).unwrap()
        }
        None => bin.as_os_str().to_owned(),
    }
}
