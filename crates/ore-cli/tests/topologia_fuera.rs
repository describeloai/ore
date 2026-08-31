//! **El artefacto de topología no viaja con la configuración.**
//!
//! El [ADR 0006](../../../docs/decisions/0006-el-artefacto-de-topologia.md) §4
//! dice la parte que no estaba escrita en ningún otro sitio:
//!
//! > **El artefacto de topología contiene datos. El plano de contexto no.**
//!
//! Una clave primaria es un valor, y *saber que el paciente X está enlazado con
//! la clínica Y **es** el diagnóstico*. De ahí una separación que hay que
//! respetar aunque las dos cosas se mapeen igual: el bundle viaja en la imagen;
//! **el artefacto de topología no**. Meterlo en una imagen OCI sería publicar las
//! aristas de un cliente en un registro.
//!
//! `.gitignore` lo excluye, pero un `.gitignore` es una intención — este fichero
//! la convierte en una comprobación. Y busca por **magia**, no por extensión:
//! renombrar el fichero es exactamente lo que haría alguien que quiere meterlo.

use std::path::{Path, PathBuf};

const MAGIA: &[u8; 8] = b"ORETOPO1";

fn recorrer(dir: &Path, out: &mut Vec<PathBuf>) {
    // `target/` y `.git/` no viajan a ninguna imagen, y `vendor/` es el
    // submódulo de la especificación.
    let saltar = ["target", ".git", "vendor", "node_modules"];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let nombre = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if saltar.contains(&nombre) {
            continue;
        }
        if p.is_dir() {
            recorrer(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn ningun_artefacto_de_topologia_esta_en_el_arbol() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut ficheros = Vec::new();
    recorrer(&raiz, &mut ficheros);

    let culpables: Vec<String> = ficheros
        .iter()
        .filter(|p| {
            std::fs::read(p)
                .map(|b| b.len() >= 8 && &b[..8] == MAGIA)
                .unwrap_or(false)
        })
        .map(|p| p.display().to_string())
        .collect();

    assert!(
        culpables.is_empty(),
        "hay un artefacto de topología dentro del árbol del repositorio:\n  {}\n\n\
         Contiene DATOS del cliente —una clave primaria es un valor, y las aristas son \
         el hecho más sensible del modelo—, así que no viaja con la configuración. Se \
         construye contra las fuentes del cliente y vive en su almacenamiento.",
        culpables.join("\n  ")
    );
}
