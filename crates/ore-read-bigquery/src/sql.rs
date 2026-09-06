//! La traducción a **GoogleSQL**. Es de BigQuery y por eso vive aquí.
//!
//! El protocolo —la petición y la fila— está en `ore-driver`, compartido. Lo
//! que no se comparte es esto, igual que `ore-read-postgres` no comparte el
//! suyo: *traducir es del driver*.
//!
//! # Por qué esto necesita los tipos de las columnas y el de PostgreSQL no
//!
//! Porque los parámetros de PostgreSQL llegan como texto y el servidor los
//! **coacciona** al tipo de la columna. GoogleSQL no: comparar un `STRING` con
//! un `INT64` es un error de tipos, no una conversión.
//!
//! Hay dos salidas y una es falsa. La falsa es `CAST(col AS STRING) = @p`:
//! compila, y a cambio **rompe el orden** —`"10" < "9"`, así que un rango sobre
//! un cursor entero devolvería otras filas sin fallar— y deja fuera el recorte
//! por partición, que en una tabla con `fullScan: forbidden` es la diferencia
//! entre leer y que BigQuery se niegue.
//!
//! La buena es preguntar. El driver conoce el sistema de tipos de su fuente
//! —eso ya lo dice `inductor.rs` de la otra mitad de la costura— así que este
//! programa hace **una consulta más** a `INFORMATION_SCHEMA.COLUMNS` y emite
//! cada parámetro con el tipo de su columna. Una columna cuyo tipo no llega no
//! se adivina: se rechaza la petición entera.

use std::collections::BTreeMap;

use ore_driver::Peticion;

/// Lo que hay que ejecutar.
#[derive(Debug)]
pub struct Invocacion {
    /// El texto. Viaja por **stdin**, como en la receta del catálogo: `argv`
    /// tiene un límite de ~32 kB en Windows y Rust se niega a pasarle saltos de
    /// línea a un `.cmd` desde CVE-2024-24576 — y `bq` en Windows es un `.cmd`.
    pub consulta: String,
    /// `nombre:TIPO:valor`, la forma que `bq --parameter` espera.
    ///
    /// # El precio, dicho y no escondido
    ///
    /// Esto **sí** viaja por `argv`, porque `bq` no admite parámetros por otro
    /// sitio, y `argv` lo lee cualquier proceso de la máquina. El protocolo
    /// evita `argv` para la petición por esa misma razón, así que aquí se está
    /// pagando algo: los **valores** de los filtros y de las claves quedan
    /// expuestos localmente mientras dura la consulta. No los nombres de
    /// columna —eso ya iba en el texto— sino los datos.
    ///
    /// La alternativa es interpolarlos en el texto, y esa es peor por dos: es
    /// una inyección esperando a un apellido con apóstrofo, y además borra la
    /// distinción entre dato y consulta que hace comprobable lo de arriba.
    pub parametros: Vec<String>,
}

/// Un identificador de BigQuery, citado con acentos graves.
///
/// **No hay escape para un acento grave dentro de un identificador citado**, así
/// que uno que lo contenga se rechaza en vez de escaparse. Inventar un escape
/// que el dialecto no tiene produciría una consulta que compila y pregunta otra
/// cosa, que es exactamente el fallo que este proyecto persigue.
fn ident(s: &str) -> Result<String, String> {
    if s.contains('`') {
        return Err(format!(
            "`{s}` lleva un acento grave, y GoogleSQL no tiene forma de escaparlo dentro de un \
             identificador citado. No se traduce"
        ));
    }
    Ok(format!("`{s}`"))
}

/// El nombre cualificado: `proyecto.dataset.tabla`.
///
/// El objeto llega como `dataset.tabla` —es lo que emite la receta del
/// catálogo— y el proyecto sale de la URL. Se cita **entero y de una vez**,
/// que es como BigQuery cita una referencia de tabla; citar por partes también
/// vale y esto es lo que sale en su documentación.
fn tabla(proyecto: &str, objeto: &str) -> Result<String, String> {
    ident(&format!("{proyecto}.{objeto}"))
}

/// El tipo de un parámetro, del tipo de su columna.
///
/// Se queda con el nombre base: `NUMERIC(10, 2)` es `NUMERIC` para
/// `--parameter`, que no admite la parametrización del tipo.
fn tipo_de(columna: &str, tipos: &BTreeMap<String, String>) -> Result<String, String> {
    let t = tipos.get(columna).ok_or_else(|| {
        format!(
            "no se sabe el tipo de `{columna}`, y sin él el parámetro se emitiría como `STRING`. \
             Comparar un `STRING` con un `INT64` es un error de tipos en GoogleSQL, y compararlo \
             como texto rompería el orden. No se adivina"
        )
    })?;
    Ok(t.split_once('(').map_or(t.as_str(), |(b, _)| b).trim().to_string())
}

/// La consulta y sus parámetros. **Función pura**: se comprueba sin servidor,
/// que es lo que hace que *«el SQL emitido contiene solo las columnas
/// proyectadas»* sea un aserto y no una promesa.
pub fn consulta(
    p: &Peticion,
    proyecto: &str,
    tipos: &BTreeMap<String, String>,
) -> Result<Invocacion, String> {
    // **Sin repetir.** Dos propiedades pueden salir de la misma columna, y
    // BigQuery nombra cada campo del resultado por su columna: pedirla dos
    // veces devolvería un objeto JSON con la clave repetida, y la fila se
    // leería mal. Se piden las columnas distintas y la fila se arma después por
    // nombre, así que las dos propiedades reciben el mismo valor, que es lo
    // correcto.
    let mut vistas: Vec<&str> = Vec::new();
    let mut columnas: Vec<String> = Vec::new();
    for (_, c) in &p.proyeccion {
        if vistas.contains(&c.as_str()) {
            continue;
        }
        vistas.push(c);
        columnas.push(ident(c)?);
    }
    let mut q = format!(
        "SELECT {} FROM {}",
        columnas.join(", "),
        tabla(proyecto, &p.objeto)?
    );

    let mut parametros: Vec<String> = Vec::new();
    let mut condiciones: Vec<String> = Vec::new();
    let mut marca = |col: &str, valor: &str, tipos: &BTreeMap<String, String>| {
        let t = tipo_de(col, tipos)?;
        let n = format!("p{}", parametros.len());
        parametros.push(format!("{n}:{t}:{valor}"));
        Ok::<String, String>(format!("@{n}"))
    };

    // **El recorte por clave, como disyunción de conjunciones.**
    //
    // GoogleSQL admite formas más cortas con `STRUCT` y con `IN UNNEST`, y esta
    // es la que se lee igual en cualquier dialecto y no depende de cómo se
    // comparan dos structs. Una clave compuesta sigue siendo una tupla: se
    // comparan sus columnas, no una concatenación —que exigiría un separador
    // que nadie declaró.
    //
    // Crece con el número de claves, y eso está acotado por dónde viaja: la
    // consulta va por stdin, así que no la limita `argv` sino el millón de
    // caracteres que BigQuery admite por consulta.
    if !p.claves.is_empty() && !p.clave_columnas.is_empty() {
        let mut tuplas: Vec<String> = Vec::new();
        for t in &p.claves {
            let mut iguales: Vec<String> = Vec::new();
            for (col, valor) in p.clave_columnas.iter().zip(t) {
                iguales.push(format!("{} = {}", ident(col)?, marca(col, valor, tipos)?));
            }
            tuplas.push(format!("({})", iguales.join(" AND ")));
        }
        condiciones.push(format!("({})", tuplas.join(" OR ")));
    }

    for (col, op, valor) in &p.filtros {
        let simbolo = match op.as_str() {
            "gt" => ">",
            _ => "=",
        };
        condiciones.push(format!(
            "{} {simbolo} {}",
            ident(col)?,
            marca(col, valor, tipos)?
        ));
    }

    // **El rango, cuando va sobre una columna.** `start` exclusivo y `end`
    // inclusivo, la convención de Iceberg: dos refrescos encadenados no repiten
    // ni se saltan el borde. Sale como dos condiciones más, que es lo que dice
    // que la petición estaba cortada por el sitio correcto — para el SQL, un
    // rango es un `WHERE`.
    if let Some(cursor) = p.cursor.as_deref() {
        if let Some(s) = &p.start {
            condiciones.push(format!("{} > {}", ident(cursor)?, marca(cursor, s, tipos)?));
        }
        if let Some(e) = &p.end {
            condiciones.push(format!("{} <= {}", ident(cursor)?, marca(cursor, e, tipos)?));
        }
    }

    if !condiciones.is_empty() {
        q.push_str(" WHERE ");
        q.push_str(&condiciones.join(" AND "));
    }
    Ok(Invocacion {
        consulta: q,
        parametros,
    })
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
            ident(&format!("{proyecto}.{dataset}.INFORMATION_SCHEMA.COLUMNS"))?
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
            ident(cursor)?,
            tabla(proyecto, objeto)?
        ),
        parametros: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tipos_de_prueba() -> BTreeMap<String, String> {
        [
            ("employee_id", "INT64"),
            ("base_pay", "NUMERIC(10, 2)"),
            ("cost_center", "STRING"),
            ("actualizado", "TIMESTAMP"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    fn peticion() -> Peticion {
        Peticion {
            url: "bigquery://acme/hr".into(),
            objeto: "hr.employees".into(),
            proyeccion: vec![
                ("baseSalary".into(), "base_pay".into()),
                ("employeeId".into(), "employee_id".into()),
            ],
            clave_columnas: vec!["employee_id".into()],
            claves: vec![vec!["7".into()], vec!["9".into()]],
            filtros: vec![("cost_center".into(), "eq".into(), "finanzas".into())],
            ..Default::default()
        }
    }

    /// El mismo aserto que en el driver de PostgreSQL: **solo las columnas
    /// proyectadas**. Una propiedad enmascarada no está en el plan, luego no
    /// está en la petición, luego no puede estar en el SQL.
    #[test]
    fn el_sql_no_pide_una_columna_que_no_este_en_la_proyeccion() {
        let i = consulta(&peticion(), "acme", &tipos_de_prueba()).expect("traduce");
        assert!(
            i.consulta
                .starts_with("SELECT `base_pay`, `employee_id` FROM `acme.hr.employees`"),
            "{}",
            i.consulta
        );
        assert!(!i.consulta.contains('*'), "{}", i.consulta);
        assert!(!i.consulta.contains("national_id"), "{}", i.consulta);
    }

    /// Ningún valor se interpola: todos salen como parámetros con nombre, y con
    /// **el tipo de su columna**.
    #[test]
    fn los_valores_van_como_parametros_y_con_el_tipo_de_su_columna() {
        let i = consulta(&peticion(), "acme", &tipos_de_prueba()).expect("traduce");
        assert!(!i.consulta.contains("finanzas"), "{}", i.consulta);
        assert!(!i.consulta.contains('7'), "{}", i.consulta);
        assert_eq!(
            i.parametros,
            vec!["p0:INT64:7", "p1:INT64:9", "p2:STRING:finanzas"]
        );
    }

    /// Y una clave compuesta se compara por columnas, no concatenada: la tupla
    /// sigue siendo una tupla.
    #[test]
    fn una_clave_compuesta_se_compara_columna_a_columna() {
        let mut p = peticion();
        p.clave_columnas = vec!["employee_id".into(), "cost_center".into()];
        p.claves = vec![vec!["7".into(), "finanzas".into()]];
        p.filtros.clear();
        let i = consulta(&p, "acme", &tipos_de_prueba()).expect("traduce");
        assert!(
            i.consulta
                .contains("((`employee_id` = @p0 AND `cost_center` = @p1))"),
            "{}",
            i.consulta
        );
        assert_eq!(i.parametros, vec!["p0:INT64:7", "p1:STRING:finanzas"]);
    }

    /// El rango: `start` exclusivo, `end` inclusivo, y con el tipo del cursor
    /// —que es lo que impide que un rango entero se compare como texto.
    #[test]
    fn el_rango_sale_como_dos_condiciones_con_el_tipo_del_cursor() {
        let mut p = peticion();
        p.claves.clear();
        p.filtros.clear();
        p.cursor = Some("actualizado".into());
        p.start = Some("2026-01-01T00:00:00Z".into());
        p.end = Some("2026-02-01T00:00:00Z".into());
        let i = consulta(&p, "acme", &tipos_de_prueba()).expect("traduce");
        assert!(
            i.consulta
                .contains("WHERE `actualizado` > @p0 AND `actualizado` <= @p1"),
            "{}",
            i.consulta
        );
        assert!(i.parametros[0].starts_with("p0:TIMESTAMP:"), "{:?}", i.parametros);
    }

    /// Un tipo que no llega **no se sustituye por `STRING`**: se rechaza la
    /// petición. Emitirlo como texto compilaría y rompería el orden de un
    /// cursor entero sin fallar, que es la dirección insegura.
    #[test]
    fn una_columna_sin_tipo_rechaza_la_peticion_en_vez_de_suponer_string() {
        let mut tipos = tipos_de_prueba();
        tipos.remove("cost_center");
        let e = consulta(&peticion(), "acme", &tipos).expect_err("se niega");
        assert!(e.contains("cost_center"), "{e}");
        assert!(e.contains("No se adivina"), "{e}");
    }

    /// Un acento grave en un identificador se rechaza: GoogleSQL no tiene
    /// escape para él dentro de un identificador citado.
    #[test]
    fn un_identificador_con_acento_grave_se_rechaza_en_vez_de_escaparse() {
        let mut p = peticion();
        p.proyeccion = vec![("x".into(), "ma`la".into())];
        let e = consulta(&p, "acme", &tipos_de_prueba()).expect_err("se niega");
        assert!(e.contains("acento grave"), "{e}");
    }

    /// La consulta de tipos pregunta al `INFORMATION_SCHEMA` del dataset del
    /// objeto, y filtra por parámetro.
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
}
