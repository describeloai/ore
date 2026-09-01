//! El registro, visto desde fuera.
//!
//! Lo que se mide aquí no es que un servidor funcione —no hay servidor— sino las
//! dos propiedades que hacen que un registro no tenga que ser de confianza:
//! **cualquiera puede recomprobarlo entero**, y **no aporta identidad**.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Los otros binarios, donde `cargo` los deja.
///
/// `CARGO_BIN_EXE_x` solo existe para la crate que lo declara. Si falta se dice
/// en vez de saltarse la prueba: una que se salta sola en CI no prueba nada.
fn vecino(nombre: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_BIN_EXE_ore-registry"))
        .with_file_name(format!("{nombre}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        p.is_file(),
        "falta `{}`. Esta prueba mide entre binarios, así que hacen falta todos: \
         `cargo test --workspace` los construye.",
        p.display()
    );
    p
}

fn salida(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn registro(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ore-registry"))
        .args(args)
        .output()
        .expect("no se pudo invocar `ore-registry`")
}

fn escribir(p: &Path, texto: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, texto).unwrap();
}

/// Un vocabulario publicable, con la versión que se le pida.
fn fuente(dir: &Path, version: &str) {
    escribir(
        &dir.join("package.yaml"),
        &format!(
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata:\n  \
             name: oos.dev/regulatory/gdpr\n  version: {version}\n  status: draft\n  \
             domain: compliance\nspec: {{ owner: \"team:compliance\" }}\n"
        ),
    );
    escribir(
        &dir.join("concepts/dateOfBirth.yaml"),
        "apiVersion: oos.dev/v1alpha4\nkind: Property\n\
         metadata: { name: dateOfBirth, namespace: gdpr }\n\
         spec:\n  type: Date\n  description: La fecha de nacimiento.\n",
    );
}

fn empaquetar(fuente_dir: &Path, destino: &Path) {
    firmando(fuente_dir, destino, None)
}

/// Y con firma, que es como viaja un paquete publicado de verdad.
///
/// Quien lo recompila desde el codigo **no tiene la clave privada**, asi que su
/// `.oob` no puede salir igual byte a byte. Ese es justo el caso que hay que
/// medir: dos ficheros distintos, el mismo paquete.
fn firmando(fuente_dir: &Path, destino: &Path, claves: Option<&Path>) {
    let mut args = vec![
        "pack".to_string(),
        fuente_dir.to_string_lossy().to_string(),
        "-o".to_string(),
        destino.to_string_lossy().to_string(),
    ];
    let mut cmd = Command::new(vecino("ore"));
    if let Some(c) = claves {
        args.extend(["--sign".to_string(), "oos.dev".to_string()]);
        cmd.env("ORE_SIGN_DIR", c).env("PATH", con_binarios());
    }
    let o = cmd.args(&args).output().expect("no se pudo invocar `ore`");
    assert!(o.status.success(), "{}", salida(&o));
}

/// El `PATH` con los binarios delante: es como `ore` encuentra a los delegados.
fn con_binarios() -> std::ffi::OsString {
    let bin = Path::new(env!("CARGO_BIN_EXE_ore-registry"))
        .parent()
        .unwrap();
    match std::env::var_os("PATH") {
        Some(p) => {
            let mut dirs = vec![bin.to_path_buf()];
            dirs.extend(std::env::split_paths(&p));
            std::env::join_paths(dirs).unwrap()
        }
        None => bin.as_os_str().to_owned(),
    }
}

fn escenario(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ore-registry-{nombre}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// El único blob que hay, para poder tocarlo.
fn blob(raiz: &Path) -> PathBuf {
    std::fs::read_dir(raiz.join("blobs/sha256"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .next()
        .expect("no hay blobs")
}

/// Publicar es escribir dos ficheros, y el resultado se sostiene solo.
#[test]
fn publicar_escribe_el_blob_y_el_indice_y_el_registro_se_verifica() {
    let d = escenario("basico");
    fuente(&d.join("fuente"), "0.1.0");
    empaquetar(&d.join("fuente"), &d.join("gdpr.oob"));

    let o = registro(&[
        "publish",
        d.join("reg").to_str().unwrap(),
        d.join("gdpr.oob").to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", salida(&o));
    assert!(
        d.join("reg/index/oos.dev/regulatory/gdpr.json").is_file(),
        "el indice no lleva la coordenada como ruta"
    );
    assert!(blob(&d.join("reg")).is_file());

    let v = registro(&["verify", d.join("reg").to_str().unwrap()]);
    assert!(v.status.success(), "{}", salida(&v));
}

/// **Un blob que no digiere su nombre lo ve cualquiera.**
///
/// Es la primera de las tres comprobaciones y la más simple, y por eso es la que
/// hace que replicar un registro no sea un acto de fe: quien acaba de copiarlo
/// corre lo mismo que quien lo opera.
#[test]
fn un_blob_que_no_digiere_su_nombre_se_ve() {
    let d = escenario("blob-tocado");
    fuente(&d.join("fuente"), "0.1.0");
    empaquetar(&d.join("fuente"), &d.join("gdpr.oob"));
    let raiz = d.join("reg");
    assert!(
        registro(&[
            "publish",
            raiz.to_str().unwrap(),
            d.join("gdpr.oob").to_str().unwrap()
        ])
        .status
        .success()
    );

    let b = blob(&raiz);
    let t = std::fs::read_to_string(&b).unwrap();
    std::fs::write(&b, t.replace("Date", "Strn")).unwrap();

    let v = registro(&["verify", raiz.to_str().unwrap()]);
    assert!(!v.status.success(), "un blob tocado paso la verificacion");
    assert!(salida(&v).contains("digieren"), "{}", salida(&v));
}

/// Y un índice que **miente sobre el digest** también, que es la mentira útil:
/// los bytes son los que dicen ser y lo que se afirma de ellos no.
#[test]
fn un_indice_que_miente_sobre_el_digest_se_ve() {
    let d = escenario("indice-miente");
    fuente(&d.join("fuente"), "0.1.0");
    empaquetar(&d.join("fuente"), &d.join("gdpr.oob"));
    let raiz = d.join("reg");
    assert!(
        registro(&[
            "publish",
            raiz.to_str().unwrap(),
            d.join("gdpr.oob").to_str().unwrap()
        ])
        .status
        .success()
    );

    let idx = raiz.join("index/oos.dev/regulatory/gdpr.json");
    let t = std::fs::read_to_string(&idx).unwrap();
    let i = t.find("sha256:").unwrap() + 7;
    let mut b = t.into_bytes();
    b[i] = if b[i] == b'b' { b'c' } else { b'b' };
    std::fs::write(&idx, b).unwrap();

    let v = registro(&["verify", raiz.to_str().unwrap()]);
    assert!(!v.status.success(), "el indice mintio y nadie lo vio");
    assert!(salida(&v).contains("el blob digiere"), "{}", salida(&v));
}

/// Una versión publicada no cambia de digest. Corregirla es **publicar otra**:
/// quien la tenga vendorizada ya la verificó, y cambiarla debajo sería mentirle
/// sin que su lock se entere.
#[test]
fn una_version_publicada_no_cambia_de_digest() {
    let d = escenario("inmutable");
    let raiz = d.join("reg");
    fuente(&d.join("fuente"), "0.1.0");
    empaquetar(&d.join("fuente"), &d.join("primera.oob"));
    assert!(
        registro(&[
            "publish",
            raiz.to_str().unwrap(),
            d.join("primera.oob").to_str().unwrap()
        ])
        .status
        .success()
    );

    // El mismo número de versión con otro contenido dentro.
    let otra = d.join("otra");
    fuente(&otra, "0.1.0");
    escribir(
        &otra.join("concepts/email.yaml"),
        "apiVersion: oos.dev/v1alpha4\nkind: Property\n\
         metadata: { name: personalEmail, namespace: gdpr }\n\
         spec:\n  type: String\n  description: El correo.\n",
    );
    empaquetar(&otra, &d.join("segunda.oob"));

    let o = registro(&[
        "publish",
        raiz.to_str().unwrap(),
        d.join("segunda.oob").to_str().unwrap(),
    ]);
    assert!(!o.status.success(), "reescribio una version publicada");
    assert!(salida(&o).contains("publicar otra"), "{}", salida(&o));
}

/// **El peldaño 3, y la razón por la que el registro va el último.**
///
/// Dos consumidores que obtienen el mismo paquete de orígenes distintos compilan
/// el mismo bundle. Y «orígenes distintos» no son dos copias del mismo
/// directorio, que solo mediría que `cp` funciona: aquí son
///
/// - **A**, un blob servido por un registro, y
/// - **B**, el `.oob` que un tercero recompila **desde el código fuente**.
///
/// No comparten un solo byte —tienen tamaños distintos y hashes distintos— y aun
/// así dan el mismo digest y el mismo bundle. Es lo que demuestra que el
/// registro no aporta identidad, sino **conveniencia**: exactamente lo que debe
/// aportar, y la propiedad que hace que se pueda prescindir de él.
#[test]
fn dos_origenes_distintos_dan_el_mismo_bundle() {
    let d = escenario("peldano-3");
    fuente(&d.join("fuente"), "0.1.0");

    // Origen A: publicado en un registro, y FIRMADO, que es como viaja de verdad.
    let claves = d.join("claves");
    escribir(&claves.join("oos.dev.key"), &"7".repeat(64));
    firmando(&d.join("fuente"), &d.join("publicado.oob"), Some(&claves));
    let reg = d.join("reg");
    assert!(
        registro(&[
            "publish",
            reg.to_str().unwrap(),
            d.join("publicado.oob").to_str().unwrap()
        ])
        .status
        .success()
    );

    // Origen B: recompilado desde el mismo codigo por un tercero, que no tiene la
    // clave privada y por tanto no puede reproducir el fichero — solo el paquete.
    let plano = d.join("desde-fuente");
    empaquetar(&d.join("fuente"), &plano.join("gdpr-0.1.0.oob"));

    // Y no comparten bytes: si los compartieran, esto mediria `cp`.
    let a = std::fs::read(blob(&reg)).unwrap();
    let b = std::fs::read(plano.join("gdpr-0.1.0.oob")).unwrap();
    assert_ne!(a, b, "los dos origenes sirven los mismos bytes");

    let bundle = |nombre: &str, origen: &Path| {
        let arbol = d.join(nombre);
        escribir(
            &arbol.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
             metadata: { name: rrhh, version: 0.1.0 }\n\
             dependencies:\n  - { package: oos.dev/regulatory/gdpr, version: \"^0.1\" }\n",
        );
        escribir(
            &arbol.join("entities/Empleado.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Entity\n\
             metadata: { name: Empleado, namespace: rrhh }\n\
             spec:\n  nature: entity\n  primaryKey: [id]\n  properties:\n    \
             id: { type: String }\n    nacido: { is: gdpr.dateOfBirth }\n",
        );
        let path = con_binarios();
        let correr = |orden: &str| {
            Command::new(vecino("ore"))
                .args([orden, arbol.to_str().unwrap()])
                .env("ORE_FETCH_DIR", origen)
                .env("PATH", &path)
                .output()
                .expect("no se pudo invocar `ore`")
        };
        let l = correr("lock");
        assert!(l.status.success(), "{}", salida(&l));
        let c = correr("compile");
        assert!(c.status.success(), "{}", salida(&c));
        salida(&c)
            .lines()
            .find(|l| l.contains("\"bundle\""))
            .map(str::trim)
            .expect("no hay digest de bundle")
            .to_string()
    };

    assert_eq!(
        bundle("consumidor-a", &reg),
        bundle("consumidor-b", &plano),
        "dos origenes del mismo paquete dieron bundles distintos"
    );
}
