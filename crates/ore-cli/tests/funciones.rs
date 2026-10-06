//! `ore functions generate` de punta a punta (ORE 0050 G1d): el cliente escribe
//! Python, la orden escribe los documentos, y `ore validate` los da por buenos.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn ore(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("ore")
}

fn salida(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn arbol(nombre: &str) -> PathBuf {
    let raiz = std::env::temp_dir().join(format!("ore-g1d-{nombre}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&raiz);
    let repo = raiz.join("packages/ventas/riesgo");
    std::fs::create_dir_all(repo.join("funciones")).unwrap();
    std::fs::write(
        raiz.join("ontology.config.yaml"),
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n",
    )
    .unwrap();
    std::fs::write(
        raiz.join("packages/ventas/package.yaml"),
        "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 0.1.0, status: draft, domain: ventas }\nspec: { owner: \"team:x\" }\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("pyproject.toml"),
        "[project]\nname = \"riesgo\"\n",
    )
    .unwrap();
    raiz
}

const CODIGO: &str = "\
from dataclasses import dataclass
from decimal import Decimal

from ore import function


@dataclass
class Nivel:
    nivel: str
    puntos: int | None = None


@function
def nivel(importe: Decimal, moneda: str = \"EUR\") -> Nivel:
    \"\"\"El nivel de riesgo de un importe.\"\"\"
    return Nivel(\"alto\" if importe > 100 else \"bajo\")


@function(timeout=\"30s\")
def saludo(nombre: str) -> str:
    return \"hola \" + nombre
";

fn escribir(raiz: &Path, codigo: &str) {
    std::fs::write(
        raiz.join("packages/ventas/riesgo/funciones/riesgo.py"),
        codigo,
    )
    .unwrap();
}

#[test]
fn el_codigo_escribe_sus_documentos_y_validan() {
    let raiz = arbol("ciclo");
    escribir(&raiz, CODIGO);
    // v1alpha26: las funciones son propias, de la raíz (ORE 0056).
    let docs = raiz.join("functions");

    // Sin documentos, `validate` dice qué falta y cómo se arregla.
    let o = ore(&raiz, &["validate", "."]);
    assert!(!o.status.success());
    assert!(salida(&o).contains("OOS2013"), "{}", salida(&o));
    assert!(
        salida(&o).contains("ore functions generate"),
        "{}",
        salida(&o)
    );

    // Generar los escribe en `functions/` de la RAÍZ, fuera de todo paquete:
    // una función publicada es `functions.<nombre>` (v1alpha26, ORE 0056).
    let o = ore(&raiz, &["functions", "generate", "."]);
    assert!(o.status.success(), "{}", salida(&o));
    let nivel = std::fs::read_to_string(docs.join("nivel.yaml")).unwrap();
    assert!(
        nivel.starts_with(
            "# generado por ore desde packages/ventas/riesgo/funciones/riesgo.py:nivel"
        ),
        "{nivel}"
    );
    assert!(
        nivel.contains("description: El nivel de riesgo de un importe."),
        "{nivel}"
    );
    assert!(
        nivel.contains("importe: { type: Decimal, required: true }"),
        "{nivel}"
    );
    assert!(nivel.contains("moneda: { type: String }"), "{nivel}");
    assert!(nivel.contains("puntos: { type: Integer }"), "{nivel}");
    let saludo = std::fs::read_to_string(docs.join("saludo.yaml")).unwrap();
    assert!(saludo.contains("output: { type: String }"), "{saludo}");
    assert!(
        saludo.contains("limits: { timeout: 30s }") || saludo.contains("timeout: '30s'"),
        "{saludo}"
    );

    let o = ore(&raiz, &["validate", "."]);
    assert!(o.status.success(), "{}", salida(&o));

    // Otra vez: nada que hacer, y `--check` lo confirma.
    let o = ore(&raiz, &["functions", "generate", "."]);
    assert!(
        salida(&o).contains("al día · 2"),
        "{}",
        salida(&o)
    );
    assert!(
        ore(&raiz, &["functions", "generate", "--check", "."])
            .status
            .success()
    );

    // El código cambia: `--check` lo ve y no escribe; generar lo pone al día.
    escribir(
        &raiz,
        &CODIGO.replace(
            "def saludo(nombre: str)",
            "def saludo(nombre: str, veces: int)",
        ),
    );
    let o = ore(&raiz, &["functions", "generate", "--check", "."]);
    assert_eq!(o.status.code(), Some(1), "{}", salida(&o));
    assert_eq!(
        std::fs::read_to_string(docs.join("saludo.yaml")).unwrap(),
        saludo
    );
    assert!(!ore(&raiz, &["validate", "."]).status.success());
    let o = ore(&raiz, &["functions", "generate", "."]);
    assert!(
        salida(&o).contains("~ functions/saludo.yaml"),
        "{}",
        salida(&o)
    );
    assert!(
        std::fs::read_to_string(docs.join("saludo.yaml"))
            .unwrap()
            .contains("veces: { type: Integer, required: true }")
    );
    assert!(ore(&raiz, &["validate", "."]).status.success());

    // Un `@function` que desaparece se lleva su documento generado.
    let sin_saludo = &CODIGO[..CODIGO.find("@function(timeout").unwrap()];
    escribir(&raiz, sin_saludo);
    let o = ore(&raiz, &["functions", "generate", "."]);
    assert!(
        salida(&o).contains("- functions/saludo.yaml"),
        "{}",
        salida(&o)
    );
    assert!(!docs.join("saludo.yaml").exists());
    assert!(
        ore(&raiz, &["validate", "."]).status.success(),
        "{}",
        salida(&ore(&raiz, &["validate", "."]))
    );
    let _ = std::fs::remove_dir_all(&raiz);
}

#[test]
fn lo_escrito_a_mano_se_dice_y_lo_que_no_se_deriva_no_se_escribe() {
    let raiz = arbol("a-mano");
    escribir(&raiz, CODIGO);
    // v1alpha26: las funciones son propias, de la raíz (ORE 0056).
    let docs = raiz.join("functions");
    std::fs::create_dir_all(raiz.join("packages/ventas/riesgo/functions")).unwrap();
    // Un documento escrito a mano para `nivel`, en otro sitio —junto al
    // repositorio, como en la v6 de la plantilla— y con un tipo mal.
    let a_mano = raiz.join("packages/ventas/riesgo/functions/el-nivel.yaml");
    std::fs::write(
        &a_mano,
        "apiVersion: oos.dev/v1alpha18\nkind: Function\nmetadata: { name: nivel, namespace: ventas }\nspec:\n  runtime: python\n  entrypoint: riesgo/funciones/riesgo.py:nivel\n  input:\n    importe: { type: Float, required: true }\n    moneda: { type: String }\n  output:\n    nivel: { type: String, required: true }\n    puntos: { type: Integer }\n",
    )
    .unwrap();
    let o = ore(&raiz, &["validate", "."]);
    assert!(
        salida(&o).contains("`input.importe` es `Float` y en el código, `Decimal`"),
        "{}",
        salida(&o)
    );

    // Se mueve a `functions/` del paquete, reescrito, y se dice que era a mano.
    let o = ore(&raiz, &["functions", "generate", "."]);
    assert!(o.status.success(), "{}", salida(&o));
    assert!(
        salida(&o).contains("estaba escrito a mano"),
        "{}",
        salida(&o)
    );
    assert!(
        salida(&o).contains("- packages/ventas/riesgo/functions/el-nivel.yaml"),
        "{}",
        salida(&o)
    );
    assert!(!a_mano.exists(), "el de antes, fuera");
    assert!(
        std::fs::read_to_string(docs.join("nivel.yaml"))
            .unwrap()
            .contains("type: Decimal")
    );
    assert!(ore(&raiz, &["validate", "."]).status.success());
    // Y otra vez, nada que mover.
    assert!(
        ore(&raiz, &["functions", "generate", "--check", "."])
            .status
            .success()
    );

    // Un `@function` que no se deriva: el error en su sitio, y nada escrito.
    escribir(
        &raiz,
        &format!("{CODIGO}\n\n@function\ndef rota(x) -> int:\n    return 1\n"),
    );
    let o = ore(&raiz, &["functions", "generate", "--json", "."]);
    assert_eq!(o.status.code(), Some(1));
    let j = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(j.contains("\"code\":\"OOS2043\""), "{j}");
    assert!(
        j.contains("\"fichero\":\"packages/ventas/riesgo/funciones/riesgo.py\""),
        "{j}"
    );
    assert!(j.contains("\"linea\":25"), "{j}");
    assert!(!docs.join("rota.yaml").exists());
    let _ = std::fs::remove_dir_all(&raiz);
}
