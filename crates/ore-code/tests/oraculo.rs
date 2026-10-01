//! La prueba **diferencial**: `ore-code` (Ruff) contra `oraculo.py` (el `ast`
//! de CPython), la misma regla escrita dos veces sobre dos analizadores.
//!
//! - `corpus/esperado.json` lo escribe el oráculo y se compromete; aquí no hace
//!   falta Python. Regenerarlo (con Docker, si no hay 3.14 a mano):
//!   `docker run --rm -v "$PWD/crates/ore-code/tests:/t" -w /t python:3.14 python oraculo.py`
//! - La biblioteca estándar entera, si `ORE_CODE_STDLIB` apunta al veredicto de
//!   CPython (`python oraculo.py --stdlib stdlib.json`): lo que CPython analiza,
//!   Ruff lo analiza, y lo que rechaza, lo rechaza. Sin la variable se salta.

use ore_code::{Salida, python};
use serde_json::{Value, json};
use std::path::Path;

fn corpus() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus"))
}

/// Lo que `ore-code` deriva, con la forma de `esperado.json`.
fn dicho(texto: &str, ruta: &str) -> Value {
    let d = python::derivar(texto, ruta);
    if !d.sintaxis.is_empty() {
        return json!({ "sintaxis": d.sintaxis[0].mensaje });
    }
    let campos = |cs: &[ore_code::Campo]| -> Vec<Value> {
        cs.iter()
            .map(|c| json!([c.nombre, c.tipo.to_string(), c.requerido]))
            .collect()
    };
    Value::Array(
        d.funciones
            .iter()
            .map(|f| match &f.resultado {
                Err(_) => json!({ "name": f.nombre, "error": "OOS2043" }),
                Ok(x) => {
                    let mut o = serde_json::Map::new();
                    o.insert("name".into(), json!(x.nombre));
                    o.insert("entrypoint".into(), json!(x.entrypoint));
                    if let Some(d) = &x.descripcion {
                        o.insert("description".into(), json!(d));
                    }
                    if let Some(v) = &x.over {
                        o.insert("over".into(), json!(v));
                    }
                    if let Some(v) = &x.reads {
                        o.insert("reads".into(), json!(v));
                    }
                    if let Some(v) = &x.models {
                        o.insert("models".into(), json!(v));
                    }
                    if let Some(v) = &x.timeout {
                        o.insert("timeout".into(), json!(v));
                    }
                    o.insert("input".into(), Value::Array(campos(&x.entrada)));
                    o.insert(
                        "output".into(),
                        match &x.salida {
                            Salida::Valor(t) => json!({ "valor": t.to_string() }),
                            Salida::Campos(cs) => json!({ "campos": campos(cs) }),
                        },
                    );
                    Value::Object(o)
                }
            })
            .collect(),
    )
}

/// El oráculo explica `por` qué no se deriva; la comparación es si se deriva
/// y qué, no con qué palabras.
fn sin_por(v: &Value) -> Value {
    let mut v = v.clone();
    if let Value::Array(a) = &mut v {
        for x in a.iter_mut() {
            if let Value::Object(o) = x {
                o.remove("por");
            }
        }
    }
    v
}

#[test]
fn deriva_lo_mismo_que_cpython() {
    let esperado: Value =
        serde_json::from_str(&std::fs::read_to_string(corpus().join("esperado.json")).unwrap())
            .unwrap();
    let mut mal = Vec::new();
    let mut ficheros = 0;
    for entrada in std::fs::read_dir(corpus().join("funciones")).unwrap() {
        let p = entrada.unwrap().path();
        let nombre = p.file_name().unwrap().to_str().unwrap().to_string();
        ficheros += 1;
        let Some(e) = esperado.get(&nombre) else {
            mal.push(format!(
                "  {nombre}: no está en esperado.json; regenera el oráculo"
            ));
            continue;
        };
        let d = dicho(
            &std::fs::read_to_string(&p).unwrap(),
            &format!("funciones/{nombre}"),
        );
        let los_dos_rotos = e.get("sintaxis").is_some() && d.get("sintaxis").is_some();
        if !los_dos_rotos && sin_por(e) != d {
            mal.push(format!(
                "  {nombre}\n    CPython  {}\n    ore-code {d}",
                sin_por(e)
            ));
        }
    }
    assert_eq!(
        ficheros,
        esperado.as_object().unwrap().len(),
        "el corpus y esperado.json no tienen los mismos ficheros"
    );
    assert!(
        mal.is_empty(),
        "ore-code no deriva lo que deriva CPython:\n{}",
        mal.join("\n")
    );
}

#[test]
fn lo_roto_se_rechaza_y_no_tumba_nada() {
    for entrada in std::fs::read_dir(corpus().join("rotos")).unwrap() {
        let p = entrada.unwrap().path();
        let d = python::derivar(&std::fs::read_to_string(&p).unwrap(), "x.py");
        assert!(
            !d.sintaxis.is_empty(),
            "{} es Python roto y se ha leído como bueno",
            p.display()
        );
    }
}

#[test]
fn un_anidamiento_hostil_no_desborda_la_pila() {
    // `ore-serve` lee código ajeno: un fichero hecho para tumbarlo tiene que
    // ser un error, no un desbordamiento de pila que se lleve el proceso.
    for (abre, cierra) in [("(", ")"), ("[", "]"), ("not ", ""), ("-", "")] {
        let fuente = format!("x = {}1{}\n", abre.repeat(50_000), cierra.repeat(50_000));
        let _ = python::derivar(&fuente, "hostil.py");
    }
    // …y una anotación de mil niveles no es una firma, ni la forma de bajar
    // por la pila de la derivación.
    let fuente = format!(
        "from ore import function
@function
def f(x: {}int{}) -> int: ...
",
        "list[".repeat(2_000),
        "]".repeat(2_000)
    );
    let d = python::derivar(&fuente, "hostil.py");
    assert!(d.funciones[0].resultado.is_err());
}

#[test]
fn la_biblioteca_estandar_como_cpython() {
    let Ok(veredicto) = std::env::var("ORE_CODE_STDLIB") else {
        eprintln!("ORE_CODE_STDLIB sin definir: se salta la stdlib");
        return;
    };
    let v: Value = serde_json::from_str(&std::fs::read_to_string(veredicto).unwrap()).unwrap();
    let (mut n, mut bytes, mut mal) = (0, 0usize, Vec::new());
    let t0 = std::time::Instant::now();
    for (p, ok) in v.as_object().unwrap() {
        let Ok(texto) = std::fs::read_to_string(p) else {
            continue;
        };
        n += 1;
        bytes += texto.len();
        let d = python::derivar(&texto, "x.py");
        if d.sintaxis.is_empty() != ok.as_bool().unwrap() {
            mal.push(p.clone());
        }
    }
    eprintln!(
        "stdlib: {} de {n} como CPython · {:.1} MB en {} ms",
        n - mal.len(),
        bytes as f64 / 1e6,
        t0.elapsed().as_millis()
    );
    assert!(
        n > 500,
        "el veredicto apunta a {n} ficheros legibles: ¿es la stdlib?"
    );
    assert!(
        mal.is_empty(),
        "Ruff y CPython no coinciden en:\n  {}",
        mal.join("\n  ")
    );
}
