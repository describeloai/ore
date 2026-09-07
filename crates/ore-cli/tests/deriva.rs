//! `ore drift-detect`, ejercido contra el catálogo capturado y contra mutaciones suyas.
//!
//! # El aserto que va primero, y por qué
//!
//! **El catálogo del que salió un paquete no puede derivar de él.** Si diera
//! cualquier otra cosa, lo que sobrara sería exactamente lo que hay que mover a
//! la columna de gobierno — y hasta que no dé eso, el detector no se puede usar
//! dos veces. Es la fatiga de alertas del sector, pero por construcción y no por
//! un umbral mal puesto.
//!
//! Ya cobró en su primera ejecución: la comparación nacía comparando **las dos
//! serializaciones como texto**, y daba doce derivas falsas porque el documento
//! escribe `[eq]` y el JSON `["eq"]`. La misma lista con dos comillas de más.
//!
//! # Lo que sí sale, y es verdad
//!
//! Dos objetos: `rubix_demo_ventas.Pedidos` y `.pedidos`, el par que colisiona
//! por mayúsculas. Están en el origen y **no están declarados**, porque la
//! inducción paró en ellos y los dejó en la cola. Eso no es un falso positivo:
//! es el detector contando lo que hay.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/catalogos/bigquery-rubix-demo-ventas.json")
}

fn ore(dir: &Path, args: &[&str]) -> (Option<i32>, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.code(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&s.stdout),
            String::from_utf8_lossy(&s.stderr)
        ),
    )
}

/// Un paquete recién inducido del catálogo, y el catálogo al lado.
fn taller(nombre: &str, catalogo: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-deriva-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("no se pudo crear el taller");
    let cat = dir.join("cat.json");
    std::fs::write(&cat, catalogo).expect("no se pudo escribir el catálogo");
    let (_, dicho) = ore(
        &dir,
        &["discover", "--from", "cat.json", "--out", "packages/ventas"],
    );
    assert!(
        dir.join("packages/ventas/tables").is_dir(),
        "el taller no tiene tablas:\n{dicho}"
    );
    dir
}

#[test]
fn el_catalogo_del_que_salio_un_paquete_no_deriva_de_el() {
    let cat = std::fs::read_to_string(fixture()).unwrap();
    let dir = taller("cero", &cat);
    let (codigo, dicho) = ore(&dir, &["drift-detect", "--from", "cat.json"]);

    // Ni una sola deriva que ESTRECHE, ni una columna, ni una cara. Lo único
    // que sale son los dos objetos que la cola retiene, y salen como `+`.
    assert!(
        !dicho.contains("\n  - ") && !dicho.contains("\n  ~ "),
        "el catálogo de origen produce deriva que duele:\n{dicho}"
    );
    assert!(!dicho.contains("la columna"), "deriva de columna:\n{dicho}");
    assert!(!dicho.contains("la cara"), "deriva de cara:\n{dicho}");

    // Y los dos que sí, que son ciertos: la colisión por mayúsculas.
    assert!(dicho.contains("rubix_demo_ventas.Pedidos"), "{dicho}");
    assert!(dicho.contains("rubix_demo_ventas.pedidos"), "{dicho}");
    assert_eq!(codigo, Some(2), "hay deriva, así que 2:\n{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **La rejilla, ejercida.** Se muta el catálogo capturado en cuatro sitios y se
/// afirma la clase y la dirección de cada uno, que son dos preguntas distintas:
/// una columna que se va y una que llega no son el mismo hecho con el signo
/// cambiado — a una le duele alguien y a la otra no.
#[test]
fn cada_clase_de_deriva_sale_con_su_direccion() {
    let cat = std::fs::read_to_string(fixture()).unwrap();
    let dir = taller("rejilla", &cat);

    // UN renombre en el origen produce **las dos** derivas a la vez —la columna
    // vieja se fue y la nueva llegó—, que es exactamente como se ve un renombre
    // desde fuera: el catálogo no dice «renombré», dice qué hay.
    let mutado = cat
        .replace("\"name\": \"nom\"", "\"name\": \"nom_del_origen\"")
        // y el recorrido se encarece: bajar la escalera que el esquema publica
        .replace("\"fullScan\": \"expensive\"", "\"fullScan\": \"forbidden\"");
    std::fs::write(dir.join("cat.json"), &mutado).unwrap();

    let (codigo, dicho) = ore(&dir, &["drift-detect", "--from", "cat.json"]);
    assert_eq!(codigo, Some(2), "{dicho}");

    // Lo que se va: estrecha.
    assert!(
        dicho.contains("- rubix_demo_ventas.clientes.nom ·"),
        "una columna que desaparece tiene que estrechar:\n{dicho}"
    );
    // Lo que llega: ensancha, y no le duele a nadie.
    assert!(
        dicho.contains("+ rubix_demo_ventas.clientes.nom_del_origen"),
        "una columna nueva tiene que ensanchar:\n{dicho}"
    );
    // Y la cara, por la escalera que el propio esquema publica.
    assert!(
        dicho.contains("- rubix_demo_ventas.clientes.reads.fullScan"),
        "bajar en la escalera de `fullScan` estrecha:\n{dicho}"
    );
    assert!(
        dicho.contains("expensive → forbidden"),
        "se dice de qué a qué:\n{dicho}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A quién le duele**, que es lo que el sector aproxima aprendiendo del uso y
/// aquí se sabe: la vista que proyecta esa columna está escrita en un fichero.
#[test]
fn una_columna_que_se_va_dice_que_vistas_la_proyectan() {
    let cat = std::fs::read_to_string(fixture()).unwrap();
    let dir = taller("linaje", &cat);
    let mutado = cat.replace("\"name\": \"nom\"", "\"name\": \"nom_del_origen\"");
    std::fs::write(dir.join("cat.json"), &mutado).unwrap();

    let (_, dicho) = ore(&dir, &["drift-detect", "--from", "cat.json"]);
    assert!(
        dicho.contains("lo proyecta:"),
        "no dice a quién le duele:\n{dicho}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Los códigos de salida, que son la mitad del contrato con un pipeline.
///
/// `2` es deriva y `0` es limpio —la convención de `terraform plan
/// -detailed-exitcode`—, y **no pueden confundirse con un error**: pedir un
/// catálogo que no existe sale con un `sysexit`, para que «no pude preguntar» y
/// «el origen cambió» sean dos respuestas distintas.
#[test]
fn no_poder_preguntar_no_se_confunde_con_haber_derivado() {
    let dir = taller("codigos", &std::fs::read_to_string(fixture()).unwrap());

    let (c, _) = ore(&dir, &["drift-detect", "--from", "no-existe.json"]);
    assert_eq!(c, Some(66), "un catálogo que no está es EX_NOINPUT");

    let (c, dicho) = ore(&dir, &["drift-detect"]);
    assert_eq!(c, Some(64), "sin origen es EX_USAGE:\n{dicho}");
    assert!(dicho.contains("fallan por separado"), "{dicho}");

    // Y el caso limpio: un paquete vacío de esa fuente no tiene nada que derivar.
    let vacio = dir.join("vacio");
    std::fs::create_dir_all(&vacio).unwrap();
    std::fs::write(
        vacio.join("cat.json"),
        r#"{"source":"otra_fuente","tables":[]}"#,
    )
    .unwrap();
    let (c, dicho) = ore(&vacio, &["drift-detect", "--from", "cat.json"]);
    assert_eq!(c, Some(0), "sin deriva es 0:\n{dicho}");
    assert!(dicho.contains("sin deriva"), "{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}
