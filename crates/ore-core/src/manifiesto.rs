//! **El encabezado de un manifiesto** — lo común entre el proyecto (0035 ①) y
//! el repositorio (0035 ⑥, 0036).
//!
//! Las dos cosas se dicen igual: un `README.md` con un encabezado entre rayas,
//! y prosa debajo. El editor lo ve, el compilador lo ignora (`ore validate`
//! sale 0 y no lo nombra) y git lo atribuye. Aquí está el trozo que comparten,
//! que es **leerlo**, y nada más: qué significa cada clave lo decide quien
//! llama.
//!
//! ```text
//! ---
//! nombre: Customer Churn
//! ---
//! Lo que esto hace, en prosa.
//! ```
//!
//! Se analiza con el analizador del árbol (`parse.rs`): ni dependencia nueva ni
//! formato nuevo. Y **no falla nunca hacia fuera**: devuelve por qué no se
//! entiende, para que quien llame lo liste igual con su motivo.
use crate::parse;

/// Lo que hay entre la primera raya y la segunda, analizado. La prosa se ignora.
pub fn encabezado(texto: &str) -> Result<parse::Node, String> {
    let mut lineas = texto.lines();
    if lineas.next().map(str::trim) != Some("---") {
        return Err("sin encabezado".into());
    }
    let mut dentro = String::new();
    let mut cerro = false;
    for l in lineas {
        if l.trim() == "---" {
            cerro = true;
            break;
        }
        dentro.push_str(l);
        dentro.push('\n');
    }
    if !cerro {
        return Err("el encabezado no cierra".into());
    }
    let n =
        parse::parse(&dentro).map_err(|e| format!("el encabezado no se analiza: {}", e.message))?;
    if n.entries().is_empty() {
        return Err("el encabezado no es un mapa".into());
    }
    Ok(n)
}

/// Un escalar del encabezado, sin espacios y sin vacíos.
pub fn campo(n: &parse::Node, clave: &str) -> Option<String> {
    n.get(clave)
        .and_then(|(_, v)| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Una lista de escalares del encabezado (`contiene: [a, b]`).
pub fn lista(n: &parse::Node, clave: &str) -> Vec<String> {
    n.get(clave)
        .map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|i| i.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
