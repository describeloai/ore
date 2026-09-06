//! `ore view add` — **autorar una pregunta nueva sobre un hecho**.
//!
//! Es el tercer acto, y hasta hoy no lo hacía nadie: `discover` espeja lo que
//! hay, `review` decide lo que la inducción no pudo decidir, y escribir una
//! vista que nadie propuso era abrir un editor de texto. Eso convierte a quien
//! lo haga —una persona, una interfaz, un backend— en un **segundo emisor**.
//!
//! Por eso esto **no escribe YAML**: llama al emisor del inductor.
//! [`crate::inductor::documento_vista`] es el único que sabe escribir una
//! `View`, y una vista autorada y una inducida son el mismo texto.
//!
//! # Lo que deriva y lo que pregunta
//!
//! P2 decide casi todo —*lo derivable no se declara*—, y `source add` ya dejó
//! escrito el corolario: *«un campo que se puede computar y aun así se pide es
//! una oportunidad de escribirlo mal»*.
//!
//! **`fields` empieza con TODAS las columnas.** Es el defecto de Foundry al
//! elegir un datasource —*«mapea cada columna a una propiedad, y puedes
//! descartar las que no quieras»*— y es el correcto en la dirección que
//! importa: quitar una columna es una decisión visible, y olvidarse de añadir
//! una no lo es.
//!
//! **El nombre se pide, y no se deriva.** El derivado ya está cogido: el
//! inductor propuso una vista por objeto y la llamó por él. Una pregunta nueva
//! sobre el mismo hecho necesita un nombre propio, y elegirlo por quien la hace
//! sería nombrar su pregunta.
//!
//! **`owner` se pregunta.** No se deriva de nada: es quién responde. Si no se
//! pasa se escribe `cambiame`, que **no valida** — la misma figura que el
//! inductor, y por el mismo motivo: un handle inventado deja el documento sin
//! nadie que responda aparentando lo contrario.
//!
//! **`freshness` y `materialized` no se proponen.** Son decisiones de operación
//! con coste, y la segunda además instancia un conducto que se sella.
//!
//! # Por qué no exige un paquete válido
//!
//! Porque el caso típico es justo el contrario: se añade una vista a un paquete
//! recién descubierto, que tiene una cola de decisiones abierta y **no compila
//! a propósito**. Exigir validez aquí haría que autorar solo fuera posible
//! después de contestarlo todo.
//!
//! Lo que sí hace es **validar al terminar y enseñar lo que salga**. Seis
//! códigos pueden dispararse el mismo día —`OOS2018`, `OOS2020`, `OOS2029`,
//! `OOS2004`, `OOS4011`, `OOS4002`— y ninguno lo puede evitar este mando:
//! son del paquete, no del documento. Escribir un fichero que rompe el árbol y
//! callarlo es media herramienta.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ore_core::document::Kind;
use ore_core::link::Loaded;

use crate::inductor::{Origen, documento_vista};

pub fn anadir(
    raiz: &Path,
    nombre: &str,
    de: &str,
    campos: &[String],
    recorte: &[String],
    owner: Option<&str>,
) -> ExitCode {
    // **Sin exigir validez**: ver la cabecera. Se carga y se mira, no se juzga.
    let pkg = ore_core::validate::cargar_paquete(raiz).0;

    let Some(origen) = buscar(&pkg.docs, de) else {
        eprintln!("error: `{de}` no es una tabla ni una vista de este paquete");
        let mut hay: Vec<String> = pkg
            .docs
            .iter()
            .filter(|d| matches!(d.kind, Kind::Table | Kind::View))
            .filter_map(|d| d.qname())
            .collect();
        hay.sort();
        if hay.is_empty() {
            eprintln!("  El paquete no tiene ninguna. `ore discover` espeja una fuente.");
        } else {
            eprintln!("  Hay: {}", hay.join(", "));
        }
        return ExitCode::from(65); // EX_DATAERR
    };

    let (fuente, disponibles) = match origen.kind {
        Kind::Table => (
            Origen::Tabla(corto(origen)),
            columnas_de(origen, "columns"),
        ),
        _ => (Origen::Vista(corto(origen)), campos_de(origen)),
    };
    if disponibles.is_empty() {
        eprintln!(
            "error: `{de}` no expone ninguna columna, así que no hay nada que preguntarle"
        );
        return ExitCode::from(65);
    }

    // **Todas, y se restan.** Sin `--field` van las de la fuente en su orden;
    // con ellos, solo esos y en el orden en que se pidieron.
    let elegidos: Vec<(String, String)> = if campos.is_empty() {
        disponibles
            .iter()
            .map(|c| (crate::inductor::identificador_publico(c), c.clone()))
            .collect()
    } else {
        let mut out = Vec::new();
        for f in campos {
            let (prop, col) = match f.split_once('=') {
                Some((p, c)) => (p.to_string(), c.to_string()),
                None => (crate::inductor::identificador_publico(f), f.clone()),
            };
            if !disponibles.contains(&col) {
                eprintln!("error: `{de}` no tiene `{col}`");
                eprintln!("  Tiene: {}", disponibles.join(", "));
                return ExitCode::from(65);
            }
            out.push((prop, col));
        }
        out
    };

    // El recorte. Un mismo nombre repetido es una **lista**, que es lo que el
    // vocabulario admite: una igualdad o un conjunto, y nada más.
    let mut donde: Vec<(String, Vec<String>)> = Vec::new();
    for w in recorte {
        let Some((col, valor)) = w.split_once('=') else {
            eprintln!("error: `{w}` no tiene la forma `columna=valor`");
            return ExitCode::from(64); // EX_USAGE
        };
        if !disponibles.contains(&col.to_string()) {
            eprintln!("error: `{de}` no tiene `{col}`, así que no se puede recortar por él");
            return ExitCode::from(65);
        }
        match donde.iter_mut().find(|(c, _)| c == col) {
            Some((_, vs)) => vs.push(valor.to_string()),
            None => donde.push((col.to_string(), vec![valor.to_string()])),
        }
    }

    let paquete = origen
        .qname()
        .and_then(|q| q.split_once('.').map(|(ns, _)| ns.to_string()))
        .unwrap_or_else(|| "paquete".to_string());
    let texto = documento_vista(
        nombre,
        &paquete,
        owner.unwrap_or("cambiame"),
        &fuente,
        &elegidos,
        &donde,
    );

    // Al lado de las demás: `views/` hermano del directorio del origen. El
    // reparto en directorios no lo decide este mando —lo decide el árbol que ya
    // hay— así que se escribe donde ya viven, y no en uno nuevo.
    let destino = vistas_de(&origen.path).join(format!("{nombre}.yaml"));
    if destino.exists() {
        eprintln!("error: `{}` ya existe", destino.display());
        eprintln!("  Sobrescribirla sería perder lo que dijera, y este mando no lo decide.");
        return ExitCode::from(65);
    }
    if let Some(d) = destino.parent()
        && let Err(e) = std::fs::create_dir_all(d)
    {
        eprintln!("error: no se pudo crear `{}`: {e}", d.display());
        return ExitCode::from(73); // EX_CANTCREAT
    }
    if let Err(e) = std::fs::write(&destino, &texto) {
        eprintln!("error: no se pudo escribir `{}`: {e}", destino.display());
        return ExitCode::from(73);
    }

    println!("  ✓ {}", destino.display());
    println!(
        "  ✓ {} campo(s){} · en DRAFT: no es verdad todavía",
        elegidos.len(),
        if donde.is_empty() {
            String::new()
        } else {
            format!(", recortada por {}", donde.len())
        }
    );
    if owner.is_none() {
        println!();
        println!("  · `owner: cambiame` — NO valida, y es a propósito");
        println!("    De él heredan las políticas. Un handle inventado dejaría la vista");
        println!("    sin nadie que responda aparentando lo contrario.");
    }

    // Y lo que salga, sale. Los seis códigos que puede disparar son del
    // paquete y no del documento, y callarlos dejaría un árbol roto en
    // silencio.
    let diags = ore_core::validate_package(raiz);
    if diags.is_empty() {
        println!();
        println!("  ok · sin errores");
        return ExitCode::SUCCESS;
    }
    println!();
    println!("  {} diagnóstico(s) en el paquete:", diags.len());
    for d in diags.iter().take(5) {
        for l in d.render(raiz).lines().take(2) {
            println!("    {l}");
        }
    }
    if diags.len() > 5 {
        println!("    … y {} más. `ore validate`", diags.len() - 5);
    }
    ExitCode::SUCCESS
}

/// Busca por nombre cualificado o por el corto. Los dos, porque quien escribe
/// desde una interfaz tiene el cualificado y quien escribe a mano tiene el otro.
fn buscar<'a>(docs: &'a [Loaded], de: &str) -> Option<&'a Loaded> {
    docs.iter()
        .filter(|d| matches!(d.kind, Kind::Table | Kind::View))
        .find(|d| d.qname().as_deref() == Some(de) || corto(d) == de)
}

fn corto(d: &Loaded) -> String {
    d.root
        .get("metadata")
        .and_then(|(_, m)| m.get("name"))
        .and_then(|(_, n)| n.as_str())
        .unwrap_or_default()
        .to_string()
}

fn columnas_de(d: &Loaded, seccion: &str) -> Vec<String> {
    d.section(seccion)
        .map(|n| {
            n.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// De una vista se pregunta por sus **campos**, no por sus columnas: lo que la
/// de arriba puede pedir es lo que la de abajo expone, y no lo que hay debajo
/// de ella. Es la misma frase que sostiene `OOS2019`.
fn campos_de(d: &Loaded) -> Vec<String> {
    columnas_de(d, "fields")
}

/// El directorio `views/` hermano del que contiene al origen.
fn vistas_de(origen: &Path) -> PathBuf {
    origen
        .parent()
        .and_then(|d| d.parent())
        .map(|p| p.join("views"))
        .unwrap_or_else(|| PathBuf::from("views"))
}
