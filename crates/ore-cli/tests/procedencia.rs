//! **El primer artefacto con procedencia real**, y lo que sostiene.
//!
//! Hasta hoy el árbol tenía casi dos mil ficheros de fixture y **ninguno venía
//! de un origen**: los datasources se llamaban `erp_snowflake` y `acme.example`,
//! los escribimos nosotros, con la forma que creemos que tienen. No era un
//! defecto de las pruebas —hacen falta y son exactas—; era que todas eran del
//! mismo tipo.
//!
//! Y `descubrimiento.rs` lo dice de sí mismo: *«esto empieza donde acaba el
//! driver, con un catálogo escrito a mano»*. Está bien razonado —lo que necesita
//! servidor es CONSEGUIR el catálogo, y lo que pasa después no—, pero de ahí
//! salía una consecuencia sin escribir: **nadie comprobaba si el catálogo que
//! escribimos se parece al que emite un driver**. Es la costura exacta por donde
//! se han caído las cosas esta semana.
//!
//! # Qué es este fichero
//!
//! `catalogos/bigquery-rubix-demo-ventas.json` es lo que `ore source catalog`
//! sacó de un dataset de BigQuery de verdad. **Doce objetos, treinta y seis
//! columnas**, y ni una fila: un catálogo dice qué columnas hay y de qué tipo, y
//! qué sabe hacer el origen. Por eso puede viajar — la regla que el árbol ya
//! tenía sobre esto, `*.oretopo` en `.gitignore`, es para lo que **contiene
//! datos**, y esto no.
//!
//! # Qué trae que ningún fixture escrito a mano traía
//!
//! - `fullScan: expensive` derivado de un hecho del origen, no elegido;
//! - `rows`, que solo emite esta familia;
//! - cuatro clases de objeto —tabla, vista, vista materializada y una tabla de
//!   respaldo— en el mismo catálogo;
//! - y **la colisión por mayúsculas**: `rubix_demo_ventas.Pedidos` y
//!   `rubix_demo_ventas.pedidos` son dos tablas distintas en BigQuery y **el
//!   mismo fichero** en Windows y en macOS. Nadie la habría escrito a mano
//!   porque nadie la habría imaginado; salió sola en el primer dataset real que
//!   se miró.
//!
//! # Cómo se recaptura
//!
//! ```text
//! ore source add bigquery://<proyecto>/<dataset> --name bq_ventas
//! ore source catalog bq_ventas --out crates/ore-cli/tests/catalogos/<…>.json
//! ```
//!
//! Y **ese diff es la detección de deriva**: si el origen cambia una columna,
//! aparece aquí. Es el verbo que `--help` declara y no está implementado, y
//! resulta que su mitad barata ya existe.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/catalogos/bigquery-rubix-demo-ventas.json")
}

fn texto() -> String {
    std::fs::read_to_string(fixture()).expect("el catálogo capturado no está")
}

/// **El ancla del emisor.**
///
/// Es la prueba que hace que este fichero valga más que como muestra: si
/// `ore_driver::catalogo::escribir` cambia de criterio —una clave que se deja,
/// un orden distinto, un tipo que se pierde— este aserto se rompe y obliga a
/// mirar. Un catálogo capturado que se reescribiera distinto cada vez sería
/// ruido en el diff que detecta la deriva, y entonces el diff no detectaría
/// nada.
#[test]
fn lo_capturado_se_relee_y_se_reescribe_igual() {
    let original = texto();
    let cat = ore_driver::catalogo::Catalogo::leer(&original).expect("el capturado analiza");
    assert_eq!(
        ore_driver::catalogo::escribir(&cat),
        original.trim_end(),
        "el emisor ya no produce lo que produjo al capturarlo"
    );
}

/// Lo que este origen **sí** dice, contado sobre el fichero y no de memoria.
#[test]
fn el_catalogo_real_trae_lo_que_ningun_fixture_escrito_a_mano_traia() {
    let cat = ore_driver::catalogo::Catalogo::leer(&texto()).unwrap();
    assert_eq!(cat.fuente(), "bq_ventas");
    assert_eq!(cat.tablas.len(), 12, "doce objetos");
    assert_eq!(
        cat.tablas.iter().map(|t| t.columnas.len()).sum::<usize>(),
        36,
        "treinta y seis columnas"
    );

    // `rows`: solo lo emite esta familia, y aquí viene de verdad.
    assert!(
        cat.tablas.iter().any(|t| t.filas.is_some()),
        "ningún objeto trae `rows`"
    );

    // Más de una clase de objeto en el mismo catálogo.
    let mut clases: Vec<&str> = cat.tablas.iter().map(|t| t.clase.as_str()).collect();
    clases.sort_unstable();
    clases.dedup();
    assert!(
        clases.len() >= 2,
        "un catálogo real trae más de una clase de objeto: {clases:?}"
    );

    // Y la colisión que nadie habría escrito a mano.
    assert!(
        cat.tablas.iter().any(|t| t.nombre.ends_with(".Pedidos"))
            && cat.tablas.iter().any(|t| t.nombre.ends_with(".pedidos")),
        "el par que colisiona por mayúsculas ya no está: era la mitad del valor de este fixture"
    );
}

/// Y lo que **no** dice, que es la otra mitad de la respuesta.
///
/// La ausencia es una respuesta —P4— y este aserto la fija: si mañana la receta
/// de BigQuery empezara a emitir `uniqueKeys`, esto se rompe y hay que decidir
/// si es que el origen lo dice o que alguien lo inventó.
#[test]
fn lo_que_este_origen_no_dice_sigue_sin_decirse() {
    let t = texto();
    for k in [
        "uniqueKeys",
        "foreignKeys",
        "references",
        "toColumns",
        "enum",
    ] {
        assert!(
            !t.contains(&format!("\"{k}\"")),
            "`{k}` aparece ahora en un catálogo de BigQuery, y antes no"
        );
    }
}

/// **El pase entero, desde un catálogo de verdad.** Es lo que
/// `descubrimiento.rs` hace con uno escrito a mano, y esto lo hace con el otro.
///
/// Lo que se afirma no son los nombres —eso es del dataset y puede cambiar—:
/// es que la inducción **para en la colisión** en vez de perder un documento.
/// Salió midiendo: los dos objetos daban dos ficheros que en Windows son el
/// mismo, el segundo pisaba al primero, y `ore validate` salía en verde porque
/// una entidad sin fuente es legal en DRAFT.
#[test]
fn induce_desde_el_catalogo_real_y_para_en_la_colision() {
    let dir = std::env::temp_dir().join(format!("ore-procedencia-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("no se pudo crear el directorio de trabajo");

    let salida = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["discover", "--from"])
        .arg(fixture())
        .args(["--out", "packages/ventas"])
        .current_dir(&dir)
        .output()
        .expect("no se pudo invocar `ore`");
    let dicho = format!(
        "{}{}",
        String::from_utf8_lossy(&salida.stdout),
        String::from_utf8_lossy(&salida.stderr)
    );

    let tablas = dir.join("packages/ventas/tables");
    let n = std::fs::read_dir(&tablas)
        .map(|d| d.flatten().count())
        .unwrap_or(0);
    assert!(n > 0, "no indujo ninguna tabla:\n{dicho}");
    assert!(
        n < 12,
        "indujo {n} de 12 sin preguntar: la colisión por mayúsculas se resolvió sola"
    );
    assert!(
        dicho.to_lowercase().contains("pedidos"),
        "la cola no menciona el par que colisiona:\n{dicho}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
