//! Cada `View` de un árbol como su consulta (`ore_core::linaje::como_sql`),
//! para las medidas de ADR 0040 (`medida-servir-la-vista-como-sql.py`).
//!
//! Uso: `como_sql <árbol>`. Salida: una línea por vista, `<qname>\t<schema>\t<sql>`
//! con los saltos de línea del SQL escritos como `\n`.

use std::path::Path;

use ore_core::document::Kind;

fn main() {
    let raiz = std::env::args().nth(1).expect("uso: como_sql <árbol>");
    let (pkg, _) = ore_core::validate::cargar_paquete(Path::new(&raiz));
    for v in pkg.docs.iter().filter(|d| d.kind == Kind::View) {
        let Some(sql) = ore_core::linaje::como_sql(v) else {
            continue;
        };
        println!(
            "{}\t{}\t{}",
            v.qname().unwrap_or_default(),
            v.schema().unwrap_or("default"),
            sql.replace('\n', "\\n")
        );
    }
}
