//! Las dos consultas que **son de BigQuery** y no de la forma.
//!
//! La forma —`SELECT` proyección, `FROM` objeto, `WHERE` claves ∧ filtros ∧
//! rango— se fue a [`ore_sql`], y el dialecto con ella:
//! `ore_sql::dialectos::BIGQUERY`. Lo que queda aquí son dos preguntas que
//! ningún otro driver hace igual, y que no caben en una plantilla:
//!
//! - **los tipos de las columnas**, que GoogleSQL obliga a saber antes de
//!   traducir porque no coacciona `STRING` a `INT64`;
//! - **el máximo del cursor**, que es el testigo cuando la tabla se fecha por
//!   una columna.
//!
//! Las dos citan identificadores, y para eso usan la `Cita` del dialecto en vez
//! de la suya: si esta crate tuviera su propio `ident`, habría dos formas de
//! citar una tabla de BigQuery, y divergirían en la que ninguna prueba ejerce.

use ore_sql::dialectos::BIGQUERY;

/// Lo que hay que ejecutar: el texto —que viaja por stdin— y sus parámetros.
#[derive(Debug)]
pub struct Invocacion {
    pub consulta: String,
    /// `nombre:TIPO:valor`, la forma que `bq --parameter` espera.
    ///
    /// # El precio, dicho y no escondido
    ///
    /// Esto **sí** viaja por `argv`, porque `bq` no admite parámetros por otro
    /// sitio, y `argv` lo lee cualquier proceso de la máquina. El protocolo
    /// evita `argv` para la petición por esa misma razón, así que aquí se está
    /// pagando algo: los **valores** quedan expuestos localmente mientras dura
    /// la consulta.
    ///
    /// La alternativa es interpolarlos en el texto, y esa es peor por dos: es
    /// una inyección esperando a un apellido con apóstrofo, y borra la
    /// distinción entre dato y consulta que hace comprobable todo lo demás.
    pub parametros: Vec<String>,
}

/// El nombre cualificado: `proyecto.dataset.tabla`.
///
/// El objeto llega como `dataset.tabla` —es lo que emite la receta del
/// catálogo— y el proyecto sale de la URL.
pub fn cualificado(proyecto: &str, objeto: &str) -> String {
    format!("{proyecto}.{objeto}")
}

/// La consulta que trae los tipos, para la tabla de esta petición.
///
/// `INFORMATION_SCHEMA.COLUMNS` es por dataset, y el objeto llega
/// `dataset.tabla`: de ahí salen los dos trozos. Se filtra por `table_name` con
/// un parámetro, no interpolando, por lo mismo que todo lo demás.
pub fn tipos(proyecto: &str, objeto: &str) -> Result<Invocacion, String> {
    let (dataset, nombre) = objeto.split_once('.').ok_or_else(|| {
        format!(
            "`{objeto}` no tiene la forma `dataset.tabla`, que es la que emite el catálogo de \
             BigQuery. Sin dataset no se sabe a qué `INFORMATION_SCHEMA` preguntar"
        )
    })?;
    Ok(Invocacion {
        consulta: format!(
            "SELECT column_name, data_type FROM {} WHERE table_name = @t",
            BIGQUERY
                .cita
                .ident(&format!("{proyecto}.{dataset}.INFORMATION_SCHEMA.COLUMNS"))?
        ),
        parametros: vec![format!("t:STRING:{nombre}")],
    })
}

/// La consulta del testigo por columna: el máximo del *cursor field*.
///
/// El mismo modelo que `ore-read-jsonl` —*«el máximo de la columna ES el
/// testigo»*— y el que medio sector llama *cursor field*.
pub fn maximo(proyecto: &str, objeto: &str, cursor: &str) -> Result<Invocacion, String> {
    Ok(Invocacion {
        consulta: format!(
            "SELECT CAST(MAX({}) AS STRING) AS m FROM {}",
            BIGQUERY.cita.ident(cursor)?,
            BIGQUERY.cita.ident(&cualificado(proyecto, objeto))?
        ),
        parametros: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los tipos se piden al `INFORMATION_SCHEMA` **del dataset del objeto**, y
    /// el nombre de la tabla va como parámetro y no en el texto.
    #[test]
    fn los_tipos_se_piden_al_information_schema_del_dataset() {
        let i = tipos("acme", "hr.employees").expect("traduce");
        assert!(
            i.consulta
                .contains("FROM `acme.hr.INFORMATION_SCHEMA.COLUMNS`"),
            "{}",
            i.consulta
        );
        assert!(!i.consulta.contains("employees"), "{}", i.consulta);
        assert_eq!(i.parametros, vec!["t:STRING:employees"]);
    }

    /// Un objeto sin dataset se rechaza en vez de suponer uno.
    #[test]
    fn un_objeto_sin_dataset_no_se_completa_a_ojo() {
        let e = tipos("acme", "employees").expect_err("se niega");
        assert!(e.contains("dataset.tabla"), "{e}");
    }

    /// Y el máximo se pide sobre la tabla cualificada con el proyecto.
    #[test]
    fn el_maximo_se_pide_sobre_la_tabla_cualificada() {
        let i = maximo("acme", "hr.employees", "actualizado").expect("traduce");
        assert_eq!(
            i.consulta,
            "SELECT CAST(MAX(`actualizado`) AS STRING) AS m FROM `acme.hr.employees`"
        );
        assert!(i.parametros.is_empty());
    }

    /// La cita es la del dialecto, no una propia: un acento grave se rechaza
    /// aquí igual que en la forma.
    #[test]
    fn la_cita_es_la_del_dialecto_y_rechaza_igual() {
        let e = maximo("acme", "hr.employees", "ma`la").expect_err("se niega");
        assert!(e.contains("no tiene forma de escaparlo"), "{e}");
    }
}
