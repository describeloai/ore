//! La traducción a SQL. **Es de PostgreSQL y por eso vive aquí.**
//!
//! El protocolo —la petición y la fila— está en `ore-driver`, compartido. Lo que
//! no se comparte es esto: **traducir es del driver**, y el lector de ficheros no
//! tiene nada parecido, que es justamente la prueba de que la petición no era
//! SQL.

use ore_driver::Peticion;

/// Un identificador de PostgreSQL, entrecomillado por partes: `public.tabla` son
/// dos, y una comilla dentro se duplica.
fn ident(s: &str) -> String {
    s.split('.')
        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}

/// El SQL y sus parámetros. **Función pura**: se comprueba sin servidor.
///
/// Nunca `SELECT *`, y nunca una columna que no esté en la proyección. Los
/// valores van **siempre** como parámetros — un valor interpolado en el texto
/// sería una inyección esperando a un apellido con apóstrofo.
pub fn sql(p: &Peticion) -> (String, Vec<String>) {
    let columnas: Vec<String> = p.proyeccion.iter().map(|(_, c)| ident(c)).collect();
    let mut q = format!("SELECT {} FROM {}", columnas.join(", "), ident(&p.objeto));

    let mut params: Vec<String> = Vec::new();
    let mut condiciones: Vec<String> = Vec::new();

    if !p.claves.is_empty() && !p.clave_columnas.is_empty() {
        let cols: Vec<String> = p.clave_columnas.iter().map(|c| ident(c)).collect();
        let mut tuplas: Vec<String> = Vec::new();
        for t in &p.claves {
            let marcas: Vec<String> = t
                .iter()
                .map(|v| {
                    params.push(v.clone());
                    format!("${}", params.len())
                })
                .collect();
            tuplas.push(format!("({})", marcas.join(", ")));
        }
        condiciones.push(format!("({}) IN ({})", cols.join(", "), tuplas.join(", ")));
    }

    for (col, op, valor) in &p.filtros {
        params.push(valor.clone());
        let simbolo = match op.as_str() {
            "gt" => ">",
            _ => "=",
        };
        condiciones.push(format!("{} {simbolo} ${}", ident(col), params.len()));
    }

    // **El rango, cuando va sobre una columna.** `start` es exclusivo y `end`
    // inclusivo, que es la convención que Iceberg usa y la que hace que dos
    // refrescos encadenados no repitan ni se salten el borde.
    //
    // Sale como dos condiciones más y no como un mecanismo aparte, y eso es lo
    // que dice que la petición estaba cortada por el sitio correcto: para el
    // SQL, un rango es un `WHERE`.
    if let Some(cursor) = p.cursor.as_deref() {
        if let Some(s) = &p.start {
            params.push(s.clone());
            condiciones.push(format!("{} > ${}", ident(cursor), params.len()));
        }
        if let Some(e) = &p.end {
            params.push(e.clone());
            condiciones.push(format!("{} <= ${}", ident(cursor), params.len()));
        }
    }

    if !condiciones.is_empty() {
        q.push_str(" WHERE ");
        q.push_str(&condiciones.join(" AND "));
    }
    (q, params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_driver::Peticion;

    fn peticion() -> Peticion {
        Peticion {
            url: "postgres://x".into(),
            objeto: "public.employees".into(),
            proyeccion: vec![
                ("baseSalary".into(), "base_pay".into()),
                ("employeeId".into(), "employee_id".into()),
            ],
            clave_columnas: vec!["employee_id".into()],
            claves: vec![vec!["emp-7".into()], vec!["emp-9".into()]],
            filtros: vec![("cost_center".into(), "eq".into(), "finanzas".into())],
            ..Default::default()
        }
    }

    /// El aserto de M2: **solo las columnas proyectadas**.
    #[test]
    fn el_sql_no_pide_una_columna_que_no_este_en_la_proyeccion() {
        let (q, _) = sql(&peticion());
        assert!(
            q.starts_with("SELECT \"base_pay\", \"employee_id\" FROM"),
            "{q}"
        );
        assert!(
            !q.contains('*'),
            "un `SELECT *` traería la columna redactada: {q}"
        );
        assert!(!q.contains("national_id"), "{q}");
    }

    /// Y los valores van como parámetros, siempre.
    #[test]
    fn ningun_valor_se_interpola_en_el_texto() {
        let (q, params) = sql(&peticion());
        assert!(q.contains("(\"employee_id\") IN (($1), ($2))"), "{q}");
        assert!(q.contains("\"cost_center\" = $3"), "{q}");
        assert_eq!(params, vec!["emp-7", "emp-9", "finanzas"]);
        assert!(!q.contains("emp-7"), "{q}");
    }

    /// Una clave compuesta es una tupla, y sigue siéndolo en el SQL.
    #[test]
    fn una_clave_compuesta_sigue_siendo_una_tupla() {
        let mut p = peticion();
        p.clave_columnas = vec!["id".into(), "cod_pais".into()];
        p.claves = vec![vec!["7".into(), "ES".into()]];
        let (q, params) = sql(&p);
        assert!(q.contains("(\"id\", \"cod_pais\") IN (($1, $2))"), "{q}");
        assert_eq!(params[..2], ["7".to_string(), "ES".to_string()]);
    }
}

#[cfg(test)]
mod rango {
    use super::*;
    use ore_driver::Peticion;

    fn con_rango(start: Option<&str>, end: Option<&str>, cursor: Option<&str>) -> Peticion {
        Peticion {
            url: "postgres://x".into(),
            objeto: "public.clicks".into(),
            proyeccion: vec![("id".into(), "click_id".into())],
            start: start.map(String::from),
            end: end.map(String::from),
            cursor: cursor.map(String::from),
            ..Default::default()
        }
    }

    /// **`start` exclusivo y `end` inclusivo.**
    ///
    /// Es la convención de Iceberg, y la que hace que dos refrescos encadenados
    /// no repitan ni se salten el borde: lo que fue `end` de uno es `start` del
    /// siguiente, y una fila cae en exactamente uno de los dos.
    #[test]
    fn el_rango_sale_como_dos_condiciones_mas() {
        let (q, params) = sql(&con_rango(Some("100"), Some("200"), Some("ocurrio_en")));
        assert!(q.contains("\"ocurrio_en\" > $1"), "{q}");
        assert!(q.contains("\"ocurrio_en\" <= $2"), "{q}");
        assert_eq!(params, vec!["100", "200"]);
    }

    /// Sin `end` se lee hasta donde esté el origen, que es lo que hacen Iceberg y
    /// Delta al omitirlo.
    #[test]
    fn sin_end_se_lee_hasta_donde_este() {
        let (q, _) = sql(&con_rango(Some("100"), None, Some("ocurrio_en")));
        assert!(q.contains("> $1") && !q.contains("<="), "{q}");
    }

    /// **Y sin `cursor` no se toca el SQL.** Ese rango va sobre la posición del
    /// changelog, y este driver no lo sirve: se rechaza antes de llegar aquí, en
    /// `rango_servible`. Traducirlo a un `WHERE` sobre alguna columna sería
    /// inventarse cuál.
    #[test]
    fn sin_cursor_el_rango_no_se_traduce_a_un_where() {
        let (q, params) = sql(&con_rango(Some("0/1A2B"), None, None));
        assert!(!q.contains("WHERE"), "{q}");
        assert!(params.is_empty(), "{params:?}");
    }
}
