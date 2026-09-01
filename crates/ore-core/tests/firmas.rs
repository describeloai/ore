//! La firma de un paquete importado — `OOS2016`.
//!
//! Se prueba entero y sobre ficheros de verdad: el cargador abre el `.oob` y
//! guarda su sobre, y `sync` lo contrasta con el lock y con las claves que el
//! consumidor declara. Probar solo la aritmética habría dejado sin cubrir la
//! parte donde de verdad puede fallar esto — que el sobre llegue.
//!
//! Y se firma aquí mismo en vez de invocar a `ore-sign`, para que la prueba no
//! dependa de que otro miembro del espacio de trabajo esté construido: lo que se
//! mide es la verificación, y el firmador tiene su propia prueba donde `cargo` sí
//! garantiza que exista.

use ore_core::firma;
use std::path::{Path, PathBuf};

/// La misma semilla siempre: la prueba no necesita una clave secreta, necesita
/// una clave **fija**. Una aleatoria haría que un fallo no se pudiera repetir.
fn par() -> ed25519_compact::KeyPair {
    ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new([7u8; 32]))
}

fn escribir(p: &Path, texto: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, texto).unwrap();
}

/// El paquete que se publica: un vocabulario con un concepto y nada más.
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

/// Empaqueta el vocabulario en un `.oob`, firmado o no.
///
/// Es lo mismo que hace `ore pack`, y se rehace aquí porque esta prueba es de
/// `ore-core`: invocar al binario habría metido en la medición si otro miembro
/// del espacio de trabajo está construido, que no es lo que se está midiendo.
fn empaquetar(fuente: &Path, destino: &Path, firmar: bool) -> String {
    let (pkg, diags) = ore_core::validate::cargar_paquete(fuente);
    assert!(diags.is_empty(), "{diags:?}");
    let publicables = ore_core::link::publicables(&pkg);
    let digest = ore_core::digest::package(&publicables);
    let canonica = ore_core::normalize::package(&publicables);

    use ore_core::json::Json;
    let mut campos = vec![
        ("oobVersion", Json::Int(1)),
        ("package", Json::s("oos.dev/regulatory/gdpr")),
        ("version", Json::s("0.1.0")),
        ("oos", Json::s("oos.dev/v1alpha4")),
        ("documents", Json::Obj(canonica.into_iter().collect())),
    ];
    if firmar {
        let e = firma::enunciado("oos.dev/regulatory/gdpr", "0.1.0", &digest);
        let s = firma::a_hex(par().sk.sign(e.as_bytes(), None).as_ref());
        campos.push((
            "signatures",
            Json::Arr(vec![Json::obj([
                ("algorithm", Json::s(firma::ED25519)),
                ("keyId", Json::s("oos.dev")),
                ("signature", Json::s(&s)),
            ])]),
        ));
    }
    escribir(destino, &Json::obj(campos).jcs());
    digest
}

/// Un consumidor con el `.oob` vendorizado, su lock y —si se le da— la clave.
///
/// `confia` y `lock_firmado` se dan por separado a propósito: son las dos mitades
/// de la comprobación, y hay casos donde solo está una.
fn consumidor(nombre: &str, firmar: bool, confia: bool, lock_firmado: bool) -> PathBuf {
    let raiz = std::env::temp_dir().join(format!("ore-firma-{nombre}"));
    let _ = std::fs::remove_dir_all(&raiz);
    let fuente = raiz.join("fuente");
    vocabulario(&fuente);
    let arbol = raiz.join("arbol");
    let digest = empaquetar(&fuente, &arbol.join("vendor/gdpr-0.1.0.oob"), firmar);

    let claves = if confia {
        format!(
            "trustedKeys:\n  - {{ id: oos.dev, algorithm: ed25519, publicKey: \"{}\" }}\n",
            firma::a_hex(par().pk.as_ref())
        )
    } else {
        String::new()
    };
    escribir(
        &arbol.join("ontology.config.yaml"),
        &format!(
            "apiVersion: oos.dev/v1alpha1\n\
             kind: OntologyConfig\n\
             metadata: {{ name: rrhh, version: 0.1.0 }}\n\
             dependencies:\n  \
               - {{ package: oos.dev/regulatory/gdpr, version: \"^0.1\" }}\n\
             {claves}"
        ),
    );
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
                 requestedBy: root\n{}",
            if lock_firmado {
                "    signedBy: [oos.dev]\n"
            } else {
                ""
            }
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

/// Un `.oob` firmado con la clave que se declara **se usa**.
#[test]
fn una_firma_que_verifica_no_estorba() {
    assert!(
        codigos(&consumidor("buena", true, true, true)).is_empty(),
        "una firma correcta rompió el build"
    );
}

/// Y una manipulada **para el build**, con el mismo trato que un digest que no
/// case: lo que hay no es lo que se aceptó.
#[test]
fn una_firma_manipulada_no_se_usa() {
    let arbol = consumidor("manipulada", true, true, true);
    let oob = arbol.join("vendor/gdpr-0.1.0.oob");
    let t = std::fs::read_to_string(&oob).unwrap();
    // Un solo carácter de la firma. El paquete sigue siendo byte a byte el
    // mismo, así que el digest cuadra y solo la firma delata el cambio.
    let i = t.find("\"signature\":\"").unwrap() + 13;
    let mut b = t.into_bytes();
    b[i] = if b[i] == b'b' { b'c' } else { b'b' };
    std::fs::write(&oob, b).unwrap();

    assert!(
        codigos(&arbol).contains(&"OOS2016".to_string()),
        "una firma manipulada pasó"
    );
}

/// **La que hace que lo anterior no se pueda esquivar.** Borrar el campo era la
/// forma barata de saltarse la comprobación, y una comprobación evitable no es
/// una comprobación: el ancla es el lock, que ya afirmó que la firma estaba.
#[test]
fn quitar_la_firma_que_el_lock_afirma_no_se_usa() {
    let arbol = consumidor("quitada", false, true, true);
    assert!(
        codigos(&arbol).contains(&"OOS2016".to_string()),
        "se quitó la firma y el build siguió en verde"
    );
}

/// Un paquete sin firmar se usa como se usaba ayer. Hacerla obligatoria hoy
/// rompería todo árbol existente por un cambio de política que nadie pidió.
#[test]
fn un_paquete_sin_firmar_se_sigue_usando() {
    assert!(
        codigos(&consumidor("sin-firma", false, true, false)).is_empty(),
        "exigió una firma que nadie prometió"
    );
}

/// Sin la clave no hay con qué comprobar, y rechazar por no poder mirar
/// convertiría la firma de un tercero en un fallo de quien no lo conoce.
#[test]
fn una_firma_de_una_clave_desconocida_se_ignora() {
    assert!(
        codigos(&consumidor("desconocida", true, false, false)).is_empty(),
        "rechazó una firma que no tenía cómo comprobar"
    );
}

/// **El contenedor no cambia la identidad, y la firma tampoco.**
///
/// Es la misma propiedad que decidió el formato, aplicada a lo que se le añade:
/// si firmar moviera el digest, un lock resuelto contra el paquete sin firmar
/// dejaría de valer el día que alguien lo firmara — y firmar habría sido
/// indistinguible de cambiar el paquete.
#[test]
fn firmar_no_cambia_el_digest() {
    let raiz = std::env::temp_dir().join("ore-firma-identidad");
    let _ = std::fs::remove_dir_all(&raiz);
    let fuente = raiz.join("fuente");
    vocabulario(&fuente);
    assert_eq!(
        empaquetar(&fuente, &raiz.join("sin.oob"), false),
        empaquetar(&fuente, &raiz.join("con.oob"), true),
    );
}
