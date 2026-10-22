//! **Lo incremental de Phoenix** (ficha 0071 §5 y §3.16): una tabla con una columna `ROW_TIMESTAMP` en la clave se
//! lee de forma incremental por esa columna (`witness: field`), y **solo** por ella. Sin ella no hay incremental:
//! no se hace un barrido y se llama incremental.
//!
//! Este módulo es **puro** (sin servidor): qué columna vale de cursor, el `SELECT` del `testigo` y cómo se escribe
//! su valor. Lo que se asegura de él es un aserto y no una promesa.
//!
//! # El testigo
//!
//! `ORDER BY cursor DESC LIMIT 1` sobre las filas **que no pasan de la hora del servidor menos un rezago**:
//!
//! - Con `ORDER BY … DESC LIMIT 1` y el cursor primero en la clave es un `REVERSE RANGE SCAN` de una fila (0,05 s en
//!   10⁶ filas). `MAX(cursor)` es un `FULL SCAN` (4,8 s), así que no se usa.
//! - **El tope**: Phoenix deja escribir una fila con una marca del futuro, y entonces el máximo salta al futuro y
//!   todo lo que se escriba hasta entonces queda por debajo del cursor, **perdido en silencio**. Con el tope, esa fila
//!   se incorpora cuando llega su hora.
//! - **El rezago**: una fila con la marca de ahora puede no haberse confirmado; si el testigo la cubre y se confirma
//!   después, el siguiente refresco empieza por encima de ella. Dejar unos segundos de margen es lo que hace el
//!   sector (`lookback`). Se paga con frescura, y se puede quitar (`rezago=0`).

use crate::esquema::{Columna, Tabla, citar};
use crate::tipos::Tipo;
use crate::traductor;
use ore_avatica::Valor as Dato;
use ore_core::tipos::{Fisico, Valor as Analizado};
use ore_driver::{Codigo, Fallo, Filtro, Valor};

/// La columna por la que se lee `t` de forma incremental, si `cursor` es ella. Si no, `operador` y por qué.
pub fn cursor_de<'a>(t: &'a Tabla, cursor: &str, objeto: &str) -> Result<&'a Columna, Fallo> {
    match (t.cursor(), t.row_timestamp.as_deref()) {
        (Some(c), _) if c.nombre == cursor => Ok(c),
        (Some(c), _) => Err(Fallo::operador(format!(
            "un rango sobre `{cursor}` de `{objeto}`: su columna ROW_TIMESTAMP es `{}`, y sobre otra sería un barrido entero que se llama incremental",
            c.nombre
        ))),
        (None, Some(r)) => Err(Fallo::operador(format!(
            "un rango sobre `{cursor}` de `{objeto}`: su columna ROW_TIMESTAMP `{r}` es de un tipo que no sirve de cursor (TIMESTAMP, DATE, BIGINT, UNSIGNED_LONG)"
        ))),
        (None, None) => Err(Fallo::operador(format!(
            "un rango sobre `{cursor}` de `{objeto}`: la tabla no declara ninguna columna ROW_TIMESTAMP, y sin ella un rango sería un barrido entero que se llama incremental"
        ))),
    }
}

/// El valor de un cursor como lo entiende el filtro de esa columna: un instante en texto, o el entero.
fn texto_de_marca(c: &Columna, ms: i64) -> String {
    match c.tipo {
        Tipo::Instante => Analizado::Instante(ms.saturating_mul(1000)).texto(&Fisico::Instante),
        _ => ms.to_string(),
    }
}

/// **El `SELECT` del testigo**: la última marca que no pasa de `tope_ms` (milisegundos desde la época). El filtro del
/// tope sale de [`traductor::condicion`], el mismo camino que el `end` de una lectura: lo que el testigo dice y lo
/// que `leer` entiende no pueden diferir.
pub fn sql_del_testigo(t: &Tabla, c: &Columna, tope_ms: i64) -> Result<String, Fallo> {
    if !t.cursor_es_primera_de_la_clave() || t.cursor().map(|k| &k.nombre) != Some(&c.nombre) {
        return Err(Fallo::new(
            Codigo::Operador,
            format!(
                "el testigo de `{}` por `{}` costaría un barrido de la tabla: su ROW_TIMESTAMP no es la primera columna de la clave",
                t.objeto(),
                c.nombre
            ),
        ));
    }
    let tope = traductor::condicion(
        &Filtro {
            columna: c.nombre.clone(),
            operador: "le".into(),
            valor: Valor::Uno(texto_de_marca(c, tope_ms)),
        },
        t,
    )?;
    Ok(format!(
        "SELECT {} FROM {} WHERE {tope} ORDER BY {} DESC LIMIT 1",
        t.expresion(c),
        t.objeto_citado(),
        citar(&c.nombre)
    ))
}

/// El valor del testigo, de lo que Phoenix devolvió. `None` si la tabla no tiene filas por debajo del tope: no es
/// un fallo, es que todavía no hay por dónde avanzar.
pub fn ordinal(c: &Columna, fila: &[Dato]) -> Option<String> {
    match fila.first() {
        Some(Dato::Entero(ms)) => Some(texto_de_marca(c, *ms)),
        _ => None,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn col(nombre: &str, tipo: &str, clave: Option<u32>) -> Columna {
        Columna {
            nombre: nombre.into(),
            tipo_phoenix: tipo.into(),
            tipo: Tipo::de(tipo, 0, 0),
            es_date: tipo == "DATE",
            precision: 0,
            escala: 0,
            anulable: clave.is_none(),
            clave,
        }
    }

    fn tabla(cursor: &str, tipo: &str, posicion: u32) -> Tabla {
        let (a, b) = if posicion == 1 {
            (Some(1), Some(2))
        } else {
            (Some(2), Some(1))
        };
        Tabla {
            esquema: Some("ORE_X0".into()),
            nombre: "EVENTOS".into(),
            columnas: vec![
                col(cursor, tipo, a),
                col("ID", "BIGINT", b),
                col("V", "VARCHAR", None),
            ],
            row_timestamp: Some(cursor.into()),
        }
    }

    // 2026-10-21 08:39:19.253 UTC
    const MS: i64 = 1_792_571_959_253;

    #[test]
    fn el_testigo_de_un_timestamp_pide_la_ultima_marca_que_no_pasa_del_tope() {
        let t = tabla("TS", "TIMESTAMP", 1);
        let c = t.cursor().unwrap();
        let sql = sql_del_testigo(&t, c, MS).unwrap();
        assert_eq!(
            sql,
            "SELECT \"TS\" FROM \"ORE_X0\".\"EVENTOS\" WHERE \"TS\" <= TIMESTAMP '2026-10-21 08:39:19.253' ORDER BY \"TS\" DESC LIMIT 1"
        );
    }

    #[test]
    fn el_testigo_de_un_date_lo_lee_como_instante() {
        let t = tabla("D", "DATE", 1);
        let sql = sql_del_testigo(&t, t.cursor().unwrap(), MS).unwrap();
        assert!(
            sql.starts_with("SELECT CAST(\"D\" AS TIMESTAMP) FROM"),
            "{sql}"
        );
        // Un `DATE` se compara con `TO_DATE`: con un literal `TIMESTAMP` no es exacto al milisegundo (§3.16).
        assert!(
            sql.contains(
                "\"D\" <= TO_DATE('2026-10-21 08:39:19.253', 'yyyy-MM-dd HH:mm:ss.SSS', 'GMT')"
            ),
            "{sql}"
        );
        // Y se ordena por la columna, no por la expresión: es lo que recorre la clave.
        assert!(sql.ends_with("ORDER BY \"D\" DESC LIMIT 1"), "{sql}");
    }

    #[test]
    fn el_testigo_de_un_bigint_compara_con_el_entero() {
        let t = tabla("T", "BIGINT", 1);
        let sql = sql_del_testigo(&t, t.cursor().unwrap(), MS).unwrap();
        assert_eq!(
            sql,
            format!(
                "SELECT \"T\" FROM \"ORE_X0\".\"EVENTOS\" WHERE \"T\" <= {MS} ORDER BY \"T\" DESC LIMIT 1"
            )
        );
    }

    #[test]
    fn el_ordinal_que_sale_es_el_que_el_filtro_entiende_de_vuelta() {
        // Lo que `testigo` devuelve se pasa luego como `start`: tiene que ser un valor de la columna.
        let t = tabla("TS", "TIMESTAMP", 1);
        let c = t.cursor().unwrap();
        let o = ordinal(c, &[Dato::Entero(MS)]).unwrap();
        let f = Filtro {
            columna: "TS".into(),
            operador: "gt".into(),
            valor: Valor::Uno(o.clone()),
        };
        assert_eq!(
            traductor::condicion(&f, &t).unwrap(),
            "\"TS\" > TIMESTAMP '2026-10-21 08:39:19.253'",
            "{o}"
        );
        let t = tabla("T", "BIGINT", 1);
        assert_eq!(
            ordinal(t.cursor().unwrap(), &[Dato::Entero(MS)]).unwrap(),
            MS.to_string()
        );
    }

    #[test]
    fn una_tabla_sin_filas_por_debajo_del_tope_no_tiene_ordinal() {
        let t = tabla("TS", "TIMESTAMP", 1);
        assert_eq!(ordinal(t.cursor().unwrap(), &[]), None);
        assert_eq!(ordinal(t.cursor().unwrap(), &[Dato::Nulo]), None);
    }

    #[test]
    fn si_el_cursor_no_es_la_primera_de_la_clave_el_testigo_se_niega() {
        let t = tabla("TS", "TIMESTAMP", 2);
        let e = sql_del_testigo(&t, t.cursor().unwrap(), MS).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("barrido"), "{}", e.mensaje);
    }

    #[test]
    fn el_cursor_es_la_columna_row_timestamp_y_ninguna_otra() {
        let t = tabla("TS", "TIMESTAMP", 1);
        assert_eq!(cursor_de(&t, "TS", "ORE_X0.EVENTOS").unwrap().nombre, "TS");
        let e = cursor_de(&t, "V", "ORE_X0.EVENTOS").unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("barrido entero"), "{}", e.mensaje);
        assert!(e.mensaje.contains("`TS`"), "{}", e.mensaje);

        let mut sin = tabla("TS", "TIMESTAMP", 1);
        sin.row_timestamp = None;
        let e = cursor_de(&sin, "TS", "ORE_X0.EVENTOS").unwrap_err();
        assert!(
            e.mensaje
                .contains("no declara ninguna columna ROW_TIMESTAMP"),
            "{}",
            e.mensaje
        );

        // Un tipo que no sirve de cursor: la clave de HBase lo admite, este origen no lo filtra.
        let t = tabla("H", "TIME", 1);
        let e = cursor_de(&t, "H", "ORE_X0.EVENTOS").unwrap_err();
        assert!(e.mensaje.contains("no sirve de cursor"), "{}", e.mensaje);
    }
}
