//! Las consultas que **son de BigQuery** y no de la forma.
//!
//! La forma —`SELECT` proyección, `FROM` objeto, `WHERE` claves ∧ filtros ∧
//! rango— se fue a [`ore_sql`], y el dialecto con ella:
//! `ore_sql::dialectos::BIGQUERY`. Lo que queda aquí es lo que ningún otro
//! driver hace igual: partir el objeto en dataset y tabla, y **el máximo del
//! cursor**, que es el testigo cuando la tabla se fecha por una columna.
//!
//! Los tipos de las columnas —que GoogleSQL obliga a saber antes de traducir,
//! porque no coacciona `STRING` a `INT64`— ya no son una consulta: los da
//! `tables.get`, sin job y sin coste (A2).
//!
//! Las consultas citan identificadores con la `Cita` del dialecto en vez de una
//! suya: si esta crate tuviera su propio `ident`, habría dos formas de citar una
//! tabla de BigQuery, y divergirían en la que ninguna prueba ejerce.

use ore_sql::dialectos::BIGQUERY;

/// El nombre cualificado: `proyecto.dataset.tabla`.
///
/// El objeto llega como `dataset.tabla` —es lo que emite la receta del
/// catálogo— y el proyecto sale de la URL.
pub fn cualificado(proyecto: &str, objeto: &str) -> String {
    format!("{proyecto}.{objeto}")
}

/// `dataset.tabla` → los dos trozos. Sin dataset no se sabe a qué tabla
/// preguntar, y completarlo a ojo sería elegir una.
pub fn partes(objeto: &str) -> Result<(&str, &str), String> {
    objeto.split_once('.').ok_or_else(|| {
        format!(
            "`{objeto}` no tiene la forma `dataset.tabla`, que es la que emite el catálogo de \
             BigQuery. Sin dataset no se sabe a qué tabla preguntar"
        )
    })
}

/// La consulta del testigo por columna: el máximo del *cursor field*.
///
/// El mismo modelo que `ore-read-jsonl` —*«el máximo de la columna ES el
/// testigo»*— y el que medio sector llama *cursor field*.
pub fn maximo(proyecto: &str, objeto: &str, cursor: &str) -> Result<String, String> {
    Ok(format!(
        "SELECT CAST(MAX({}) AS STRING) AS m FROM {}",
        BIGQUERY.cita.ident(cursor)?,
        BIGQUERY.cita.ident(&cualificado(proyecto, objeto))?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un objeto sin dataset se rechaza en vez de suponer uno.
    #[test]
    fn un_objeto_sin_dataset_no_se_completa_a_ojo() {
        assert_eq!(partes("hr.employees"), Ok(("hr", "employees")));
        let e = partes("employees").expect_err("se niega");
        assert!(e.contains("dataset.tabla"), "{e}");
    }

    /// Y el máximo se pide sobre la tabla cualificada con el proyecto.
    #[test]
    fn el_maximo_se_pide_sobre_la_tabla_cualificada() {
        assert_eq!(
            maximo("acme", "hr.employees", "actualizado").expect("traduce"),
            "SELECT CAST(MAX(`actualizado`) AS STRING) AS m FROM `acme.hr.employees`"
        );
    }

    /// La cita es la del dialecto, no una propia: un acento grave se rechaza
    /// aquí igual que en la forma.
    #[test]
    fn la_cita_es_la_del_dialecto_y_rechaza_igual() {
        let e = maximo("acme", "hr.employees", "ma`la").expect_err("se niega");
        assert!(e.contains("no tiene forma de escaparlo"), "{e}");
    }
}
