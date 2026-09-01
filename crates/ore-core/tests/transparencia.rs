//! La prueba de transparencia de un paquete importado — `OOS2017`.
//!
//! Sobre ficheros de verdad, como las de la firma y por lo mismo: lo que puede
//! fallar aquí no es la aritmética —que tiene sus propias pruebas, exhaustivas
//! sobre todos los árboles hasta 16— sino que la prueba **llegue** desde el
//! sobre hasta quien la comprueba.
//!
//! El log se construye aquí en vez de invocar a `ore-log`, para no depender de
//! que otro miembro del espacio de trabajo esté construido. El programa tiene su
//! propia prueba donde `cargo` sí garantiza que exista.

use ore_core::{firma, transparencia as t};
use std::path::{Path, PathBuf};

fn clave_del_paquete() -> ed25519_compact::KeyPair {
    ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([7u8; 32]))
}

fn clave_del_log() -> ed25519_compact::KeyPair {
    // Distinta de la del paquete, y a propósito: son **dos autoridades**. Quien
    // publica afirma «esto es mío»; el log afirma «esto lo he visto y esta es
    // toda mi lista». Con una sola clave, la segunda no diría nada.
    ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([3u8; 32]))
}

fn escribir(p: &Path, texto: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, texto).unwrap();
}

fn vocabulario(dir: &Path) {
    escribir(
        &dir.join("package.yaml"),
        r#"apiVersion: oos.dev/v1alpha1
kind: Package
metadata:
  name: oos.dev/regulatory/gdpr
  version: 0.1.0
  status: draft
  domain: compliance
spec: { owner: "team:compliance" }
"#,
    );
    escribir(
        &dir.join("concepts/dateOfBirth.yaml"),
        r#"apiVersion: oos.dev/v1alpha4
kind: Property
metadata: { name: dateOfBirth, namespace: gdpr }
spec:
  type: Date
  description: La fecha de nacimiento de una persona fisica.
"#,
    );
}

/// Cómo se tuerce un árbol antes de escribirlo. Cada variante es un ataque
/// distinto, y se nombran por lo que hace el atacante y no por el campo.
#[derive(Clone, Copy, PartialEq)]
enum Trampa {
    Ninguna,
    /// Se quita la prueba del sobre. Es la salida barata, y la que el lock cierra.
    SinPrueba,
    /// Se cambia un hash del camino: la hoja deja de caer en ese árbol.
    CaminoTorcido,
    /// Una raíz que nadie firmó, con su árbol hecho a medida.
    RaizInventada,
    /// El log tiene dos historias que **contienen las dos esta entrada**, con
    /// otro vecino: mismo tamaño, otra raíz, firmadas las dos por la clave buena.
    ///
    /// Que la entrada esté en las dos es lo que hace el caso: su prueba de
    /// inclusión es impecable en ambas, y **ninguna comprobación sobre un solo
    /// árbol lo ve**. Una bifurcación donde la entrada faltara la cazaría la
    /// inclusión, y entonces esto no probaría nada.
    LogBifurcado,
}

/// Un consumidor con un `.oob` firmado, anotado en un log de una sola entrada.
fn consumidor(nombre: &str, trampa: Trampa) -> PathBuf {
    let raiz = std::env::temp_dir().join(format!("ore-log-{nombre}"));
    let _ = std::fs::remove_dir_all(&raiz);
    let fuente = raiz.join("fuente");
    vocabulario(&fuente);
    let arbol = raiz.join("arbol");

    let (pkg, diags) = ore_core::validate::cargar_paquete(&fuente);
    assert!(diags.is_empty(), "{diags:?}");
    let publicables = ore_core::link::publicables(&pkg);
    let digest = ore_core::digest::package(&publicables);
    let enunciado = firma::enunciado("oos.dev/regulatory/gdpr", "0.1.0", &digest);
    let sig = firma::a_hex(
        clave_del_paquete()
            .sk
            .sign(enunciado.as_bytes(), None)
            .as_ref(),
    );

    // El log: una sola entrada, que es la que se acaba de firmar. Y si bifurca,
    // otra lista con otra cosa dentro — el mismo tamaño y otra raíz.
    let entrada = t::entrada(&enunciado, "oos.dev", &sig);
    // Dos historias de dos entradas, con la del paquete en las dos y otra
    // distinta al lado. Es la forma mínima en que un log puede bifurcar sin que
    // se note por inclusión: con un solo elemento, el árbol que lo contiene es
    // uno y su raíz también.
    let honestas = vec![
        t::hoja(b"lo que de verdad se anoto"),
        t::hoja(entrada.as_bytes()),
    ];
    let hojas = if trampa == Trampa::LogBifurcado {
        vec![
            t::hoja(b"lo que le enseno a otro"),
            t::hoja(entrada.as_bytes()),
        ]
    } else {
        honestas.clone()
    };
    let mut raiz_log = t::raiz(&hojas);
    if trampa == Trampa::RaizInventada {
        raiz_log[0] ^= 1;
    }
    // Firmada SIEMPRE por la clave buena del log, incluso al bifurcar: es lo que
    // hace el caso interesante. Un log que bifurca no firma mal, firma dos veces.
    let cabeza = firma::a_hex(
        clave_del_log()
            .sk
            .sign(t::cabeza("oos.dev/log", 2, &raiz_log).as_bytes(), None)
            .as_ref(),
    );
    let mut camino: Vec<String> = t::camino_de_inclusion(&hojas, 1)
        .iter()
        .map(t::a_hex)
        .collect();
    if trampa == Trampa::CaminoTorcido {
        camino[0] = t::a_hex(&t::hoja(b"otro hermano"));
    }

    use ore_core::json::Json;
    let mut campos = vec![
        ("oobVersion", Json::Int(1)),
        ("package", Json::s("oos.dev/regulatory/gdpr")),
        ("version", Json::s("0.1.0")),
        ("oos", Json::s("oos.dev/v1alpha4")),
        (
            "documents",
            Json::Obj(
                ore_core::normalize::package(&publicables)
                    .into_iter()
                    .collect(),
            ),
        ),
        (
            "signatures",
            Json::Arr(vec![Json::obj([
                ("algorithm", Json::s(firma::ED25519)),
                ("keyId", Json::s("oos.dev")),
                ("signature", Json::s(&sig)),
            ])]),
        ),
    ];
    if trampa != Trampa::SinPrueba {
        campos.push((
            "transparency",
            Json::Arr(vec![Json::obj([
                ("index", Json::Int(1)),
                ("inclusion", Json::Arr(camino.iter().map(Json::s).collect())),
                ("keyId", Json::s("oos.dev")),
                ("logId", Json::s("oos.dev/log")),
                ("root", Json::s(t::a_hex(&raiz_log))),
                ("rootSignature", Json::s(&cabeza)),
                ("treeSize", Json::Int(2)),
            ])]),
        ));
    }
    escribir(
        &arbol.join("vendor/gdpr-0.1.0.oob"),
        &Json::obj(campos).jcs(),
    );

    escribir(
        &arbol.join("ontology.config.yaml"),
        &format!(
            "apiVersion: oos.dev/v1alpha1\n\
             kind: OntologyConfig\n\
             metadata: {{ name: rrhh, version: 0.1.0 }}\n\
             dependencies:\n  \
               - {{ package: oos.dev/regulatory/gdpr, version: \"^0.1\" }}\n\
             trustedKeys:\n  \
               - {{ id: oos.dev, algorithm: ed25519, publicKey: \"{}\" }}\n\
             trustedLogs:\n  \
               - {{ id: \"oos.dev/log\", algorithm: ed25519, publicKey: \"{}\" }}\n",
            firma::a_hex(clave_del_paquete().pk.as_ref()),
            firma::a_hex(clave_del_log().pk.as_ref()),
        ),
    );
    // El lock fija la cabeza BUENA, del mismo tamaño que la que trae el paquete.
    // Es el ancla, y es lo único que puede ver una segunda historia sin salir a
    // preguntar: dos raíces para el mismo tamaño. Que las cabezas de tamaños
    // distintos sean consistentes lo comprueba `ore lock`, que sí puede pedir la
    // prueba — aquí no se toca nada de fuera.
    let honesta = t::raiz(&honestas);
    escribir(
        &arbol.join("ontology.lock"),
        &format!(
            "lockfileVersion: 1\n\
             generatedFor: rrhh\n\
             \n\
             packages:\n\
             \n  - name: oos.dev/regulatory/gdpr\n    \
                 version: 0.1.0\n    \
                 resolved: file:vendor/gdpr-0.1.0.oob\n    \
                 digest: {digest}\n    \
                 range: \"^0.1\"\n    \
                 signedBy: [oos.dev]\n    \
                 logged: [\"oos.dev/log\"]\n    \
                 requestedBy: root\n\
             \n\
             logs:\n\
             \n  - id: \"oos.dev/log\"\n    \
                 treeSize: 2\n    \
                 root: {}\n",
            t::a_hex(&honesta)
        ),
    );
    arbol
}

fn codigos(raiz: &Path) -> Vec<String> {
    ore_core::validate_package(raiz)
        .iter()
        .map(|d| d.code.as_str().to_string())
        .collect()
}

/// Un paquete anotado en un log en el que este árbol confía **se usa**.
#[test]
fn una_prueba_que_verifica_no_estorba() {
    assert!(
        codigos(&consumidor("buena", Trampa::Ninguna)).is_empty(),
        "una prueba correcta rompió el build"
    );
}

/// Un camino que no reconstruye la raíz declarada no prueba nada.
#[test]
fn un_camino_torcido_no_se_usa() {
    assert!(
        codigos(&consumidor("camino", Trampa::CaminoTorcido)).contains(&"OOS2017".to_string()),
        "una prueba torcida pasó"
    );
}

/// **Sin cabeza firmada, una raíz es un número que alguien escribió.**
///
/// Es el agujero que se cierra comprobando la firma del log ANTES que la
/// inclusión: con la raíz a elección, cualquiera construye un árbol que contenga
/// la hoja que quiera y presenta una prueba impecable de algo que ningún log ha
/// visto.
#[test]
fn una_raiz_que_el_log_no_firmo_no_se_usa() {
    assert!(
        codigos(&consumidor("raiz", Trampa::RaizInventada)).contains(&"OOS2017".to_string()),
        "aceptó una raíz que nadie firmó"
    );
}

/// La que hace que lo anterior no se pueda esquivar, igual que con la firma: si
/// el lock dice que está anotado, quitar la prueba tampoco compila.
#[test]
fn quitar_la_prueba_que_el_lock_afirma_no_se_usa() {
    assert!(
        codigos(&consumidor("sin", Trampa::SinPrueba)).contains(&"OOS2017".to_string()),
        "se quitó la prueba y el build siguió en verde"
    );
}

/// **La bifurcación, que es el ataque entero.**
///
/// Un log con dos historias firma las dos con su clave buena, y cada prueba de
/// inclusión cuadra contra la suya: ninguna comprobación local sobre un solo
/// árbol lo ve. Lo único que lo delata es tener **otra raíz para el mismo
/// tamaño**, y por eso el lock la recuerda.
#[test]
fn dos_raices_del_mismo_tamano_no_se_usan() {
    let arbol = consumidor("bifurcado", Trampa::LogBifurcado);
    assert!(
        codigos(&arbol).contains(&"OOS2017".to_string()),
        "aceptó un log con dos historias"
    );
    // Y la prueba de inclusión de la historia torcida es impecable: es lo que
    // hace que haga falta el lock.
    let hojas = [t::hoja(b"lo que le enseno a otro"), t::hoja(b"da igual")];
    let torcida = t::raiz(&hojas);
    assert_eq!(
        t::inclusion(
            &hojas[1],
            1,
            2,
            &t::camino_de_inclusion(&hojas, 1),
            &torcida
        ),
        Ok(()),
        "la inclusion sola no ve nada raro: por eso hace falta el lock"
    );
}
