//! **El CSV de Bulk API 2.0** (ADR 0083 §1.6, X0 C1 y C11): un lector de RFC 4180
//! sin dependencias.
//!
//! Lo que X0 midió que el CSV de Salesforce hace:
//!
//! - cada página trae **su propia cabecera** (el `locator` pagina, y cada trozo
//!   empieza por los nombres de columna);
//! - **nulo y vacío son la misma celda vacía** (C11): aquí, una celda vacía es
//!   `None`. REST ya devolvía `null` para un texto vacío guardado (B15), así que
//!   no se pierde nada que la otra vía conservara;
//! - comas, comillas dobles (`""`), **saltos de línea** y Unicode van dentro de
//!   un campo entre comillas y llegan intactos;
//! - `NA`, `None` y `null` llegan como texto: son nombres, no nulos.

/// Las filas del CSV, la primera es la cabecera. Cada celda es `None` si está
/// vacía. Falla si una comilla queda abierta.
pub fn leer(texto: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    let mut filas: Vec<Vec<Option<String>>> = Vec::new();
    let mut fila: Vec<Option<String>> = Vec::new();
    let mut celda = String::new();
    let mut entre_comillas = false;
    // Una celda `""` está vacía pero existe: cuenta para cerrar la última fila.
    let mut hubo_comillas = false;
    let mut c = texto.chars().peekable();
    let cerrar = |celda: &mut String, fila: &mut Vec<Option<String>>, hubo: &mut bool| {
        // Nulo y vacío son lo mismo, entrecomillado o no (X0 C11): Bulk escribe un
        // nulo como `""`.
        fila.push(if celda.is_empty() {
            None
        } else {
            Some(std::mem::take(celda))
        });
        *hubo = false;
    };
    while let Some(x) = c.next() {
        if entre_comillas {
            match x {
                '"' if c.peek() == Some(&'"') => {
                    celda.push('"');
                    c.next();
                }
                '"' => entre_comillas = false,
                otro => celda.push(otro),
            }
            continue;
        }
        match x {
            '"' if celda.is_empty() => {
                entre_comillas = true;
                hubo_comillas = true;
            }
            ',' => cerrar(&mut celda, &mut fila, &mut hubo_comillas),
            '\r' if c.peek() == Some(&'\n') => {}
            '\n' => {
                cerrar(&mut celda, &mut fila, &mut hubo_comillas);
                filas.push(std::mem::take(&mut fila));
            }
            otro => celda.push(otro),
        }
    }
    if entre_comillas {
        return Err("el CSV termina con una comilla sin cerrar".into());
    }
    if !celda.is_empty() || hubo_comillas || !fila.is_empty() {
        cerrar(&mut celda, &mut fila, &mut hubo_comillas);
        filas.push(fila);
    }
    Ok(filas)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn s(x: &str) -> Option<String> {
        Some(x.to_string())
    }

    #[test]
    fn una_pagina_con_cabecera_y_nulos() {
        let f = leer("Id,Name,Industry\n001A,\"NA\",\n001B,None,Banking\n").unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!(f[0], [s("Id"), s("Name"), s("Industry")]);
        assert_eq!(
            f[1],
            [s("001A"), s("NA"), None],
            "vacío es nulo; `NA` es un texto"
        );
        assert_eq!(f[2], [s("001B"), s("None"), s("Banking")]);
    }

    /// X0 C11: la descripción de la siembra, con coma, comillas, salto y Unicode.
    #[test]
    fn un_campo_con_todo_dentro_llega_intacto() {
        let d = "fila 0, con coma, \"comillas\"\nsalto de línea · ñ 日本 😀";
        let csv = format!("Description\n\"{}\"\n", d.replace('"', "\"\""));
        let f = leer(&csv).unwrap();
        assert_eq!(f[1], [s(d)]);
    }

    #[test]
    fn el_final_de_linea_crlf_y_la_ultima_fila_sin_salto() {
        let f = leer("a,b\r\n1,2").unwrap();
        assert_eq!(f, [[s("a"), s("b")], [s("1"), s("2")]]);
    }

    /// Lo que Salesforce hace de verdad (cazado por `lectura_real`): Bulk escribe
    /// el nulo como `""`. Es un nulo, no un texto vacío.
    #[test]
    fn una_celda_entrecomillada_vacia_es_un_nulo_como_la_de_bulk() {
        let f = leer("a,b\n\"\",\n").unwrap();
        assert_eq!(f[1], [None, None]);
        let g = leer("Id,NumberOfEmployees\n\"001A\",\"\"\n").unwrap();
        assert_eq!(g[1], [s("001A"), None]);
    }

    #[test]
    fn una_comilla_abierta_se_dice() {
        assert!(leer("a\n\"sin cerrar").unwrap_err().contains("sin cerrar"));
    }

    #[test]
    fn un_csv_vacio_no_tiene_filas() {
        assert!(leer("").unwrap().is_empty());
    }
}
