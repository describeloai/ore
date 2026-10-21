//! **De los filtros y el orden de una `Peticion` a SQL de Phoenix** (ficha 0071 §3.12). Una función pura: sin red
//! y sin servidor, porque «el valor no se sale de su literal» y «lo que no se sabe escribir se rechaza» tienen que
//! ser un aserto y no una promesa (0008).
//!
//! Phoenix admite todo el vocabulario (`=`, `<>`, `<`…, `IN`, `LIKE`, `IS NULL`), con la semántica de SQL, así que
//! **no hay residual**: lo que se pide se empuja entero o se rechaza con `operador`. Lo que Phoenix hace distinto
//! y lo que este módulo hace por ello (medido en §3.12):
//!
//! - **La barra invertida es un escape** dentro de un literal de texto: se duplica, y con ella las comillas. Con
//!   solo las comillas, un valor que acabe en `\` dejaría el literal sin cerrar.
//! - **La cadena vacía es nulo**, también como literal: `<> ''` no devuelve nada. Se normaliza.
//! - **`LIKE` trata `\_` y `\%` como escapes** y no admite `ESCAPE`: un patrón con barra se rechaza.
//! - **Un instante se escribe `TIMESTAMP '…'`** y un día `TO_DATE(…, 'GMT')`; `TO_TIMESTAMP` con `SSSSSS` lee
//!   microsegundos como milisegundos.
//! - **`NaN` e `Infinity` no tienen literal**: se rechazan.
//! - **`IN ()` no existe**: la lista vacía es `1 = 0`.
//! - **Los nulos van al final en los dos sentidos** (`NULLS LAST`), que es lo que hace DuckDB.

use crate::esquema::{Columna, Tabla, citar};
use crate::tipos::Tipo;
use ore_core::tipos::{Fisico, Valor as Analizado};
use ore_driver::{Codigo, Fallo, Filtro, Orden, Valor};

/// Lo que no se cumple nunca (una lista `IN` vacía, un `= ''`).
const NADA: &str = "1 = 0";

/// Un literal de texto de Phoenix: entre comillas simples, con las comillas **y las barras** duplicadas.
pub fn literal_texto(s: &str) -> String {
    let mut r = String::with_capacity(s.len() + 2);
    r.push('\'');
    for c in s.chars() {
        match c {
            '\'' => r.push_str("''"),
            '\\' => r.push_str("\\\\"),
            c => r.push(c),
        }
    }
    r.push('\'');
    r
}

fn operador(m: String) -> Fallo {
    Fallo::new(Codigo::Operador, m)
}

/// Si la columna se puede comparar como SQL, o por qué no.
fn comparable(c: &Columna, op: &str) -> Result<(), Fallo> {
    let motivo = match &c.tipo {
        Tipo::Binario => "es binaria",
        Tipo::Lista => "es una lista (`ARRAY`)",
        Tipo::DecimalLibre => {
            "es un `DECIMAL` sin precisión, que sale como texto y no se compara como número"
        }
        Tipo::Hora => "es un `TIME`, que este origen todavía no filtra",
        Tipo::Otro(_) => "es de un tipo que este origen no lee",
        _ => return Ok(()),
    };
    Err(operador(format!(
        "`{op}` sobre `{}`: la columna {motivo}; no se filtra",
        c.nombre
    )))
}

/// El valor de un filtro como literal de Phoenix, según el tipo de la columna.
fn literal(c: &Columna, op: &str, v: &str) -> Result<String, Fallo> {
    let fisico = c.tipo.fisico();
    let analizado = fisico.analizar(v).ok_or_else(|| {
        operador(format!(
            "`{op}` sobre `{}`: `{v}` no es un valor de {}",
            c.nombre,
            c.tipo.oos()
        ))
    })?;
    Ok(match (&analizado, &fisico) {
        (Analizado::Texto(s), _) => {
            if s.contains('\0') {
                return Err(operador(format!(
                    "`{op}` sobre `{}`: el valor lleva un carácter nulo, que Phoenix no admite",
                    c.nombre
                )));
            }
            literal_texto(s)
        }
        (Analizado::Entero(n), _) => n.to_string(),
        (Analizado::Real(f), _) => {
            if !f.is_finite() {
                return Err(operador(format!(
                    "`{op}` sobre `{}`: `{v}` no tiene literal en Phoenix (`NaN` e `Infinity` no se escriben)",
                    c.nombre
                )));
            }
            format!("{f:?}")
        }
        (Analizado::Logico(b), _) => if *b { "TRUE" } else { "FALSE" }.into(),
        // Exacto: lo que `analizar` aceptó cabe en la precisión y la escala de la columna.
        (Analizado::Decimal(_), _) => v.to_string(),
        (Analizado::Fecha(d), _) => format!(
            "TO_DATE('{}', 'yyyy-MM-dd', 'GMT')",
            Analizado::Fecha(*d).texto(&fisico)
        ),
        (Analizado::FechaHora(us), _) | (Analizado::Instante(us), _) => format!(
            "TIMESTAMP '{}'",
            Analizado::FechaHora(*us).texto(&Fisico::FechaHora)
        ),
        (Analizado::Hora(_), _) => unreachable!("`comparable` rechaza el `TIME`"),
    })
}

/// Una condición de `WHERE` por filtro. Cada una es **exacta** o es un error `operador`/`esquema`.
pub fn condicion(f: &Filtro, t: &Tabla) -> Result<String, Fallo> {
    let c = t.columna(&f.columna).ok_or_else(|| {
        Fallo::new(
            Codigo::Esquema,
            format!("la columna `{}` no está en `{}`", f.columna, t.objeto()),
        )
    })?;
    let q = citar(&c.nombre);
    let op = f.operador.as_str();
    let es_texto = c.tipo == Tipo::Texto;
    match (op, &f.valor) {
        ("isNull", Valor::Ninguno) => Ok(format!("{q} IS NULL")),
        ("isNotNull", Valor::Ninguno) => Ok(format!("{q} IS NOT NULL")),
        ("isNull" | "isNotNull", _) => Err(operador(format!(
            "`{op}` sobre `{}` no lleva valor",
            c.nombre
        ))),
        ("in", Valor::Lista(lista)) => {
            comparable(c, op)?;
            let mut valores = Vec::new();
            for v in lista {
                if es_texto && v.is_empty() {
                    continue;
                }
                valores.push(literal(c, op, v)?);
            }
            if valores.is_empty() {
                return Ok(NADA.into());
            }
            Ok(format!("{q} IN ({})", valores.join(", ")))
        }
        ("like", Valor::Uno(v)) => {
            if c.tipo != Tipo::Texto {
                return Err(operador(format!(
                    "`like` sobre `{}`: la columna es {}, no texto",
                    c.nombre,
                    c.tipo.oos()
                )));
            }
            if v.contains('\\') {
                return Err(operador(format!(
                    "`like` sobre `{}`: el patrón lleva `\\`, que Phoenix lee como escape de `_` y `%` y ORE como un carácter; no se empuja",
                    c.nombre
                )));
            }
            if v.is_empty() {
                return Ok(NADA.into());
            }
            Ok(format!("{q} LIKE {}", literal(c, op, v)?))
        }
        ("eq" | "neq" | "lt" | "le" | "gt" | "ge", Valor::Uno(v)) => {
            comparable(c, op)?;
            if es_texto && v.is_empty() {
                // Phoenix guarda `''` como nulo y lee el literal `''` como nulo (§3.12): «las filas con el texto
                // vacío» no existen, y «las que no son vacías» son todas las no nulas.
                return Ok(match op {
                    "neq" | "gt" | "ge" => format!("{q} IS NOT NULL"),
                    _ => NADA.into(),
                });
            }
            let simbolo = match op {
                "eq" => "=",
                "neq" => "<>",
                "lt" => "<",
                "le" => "<=",
                "gt" => ">",
                _ => ">=",
            };
            Ok(format!("{q} {simbolo} {}", literal(c, op, v)?))
        }
        (otro, _) => Err(operador(format!(
            "`{otro}` sobre `{}`: no es un operador del vocabulario, o el valor no tiene la forma del operador",
            c.nombre
        ))),
    }
}

/// El `ORDER BY`, con los nulos al final en los dos sentidos. Vacío si no se pide orden.
pub fn orden(o: &[Orden], t: &Tabla) -> Result<String, Fallo> {
    if o.is_empty() {
        return Ok(String::new());
    }
    let mut partes = Vec::new();
    for k in o {
        let c = t.columna(&k.columna).ok_or_else(|| {
            Fallo::new(
                Codigo::Esquema,
                format!("la columna `{}` no está en `{}`", k.columna, t.objeto()),
            )
        })?;
        comparable(c, "orderBy")?;
        partes.push(format!(
            "{} {} NULLS LAST",
            citar(&c.nombre),
            if k.descendente { "DESC" } else { "ASC" }
        ));
    }
    Ok(format!(" ORDER BY {}", partes.join(", ")))
}

/// El `WHERE` de todos los filtros, o vacío.
pub fn donde(filtros: &[Filtro], t: &Tabla) -> Result<String, Fallo> {
    donde_con(filtros, t, &[])
}

/// El `WHERE` de todos los filtros **y de unas condiciones ya escritas** (el rango de una parte de la lectura).
pub fn donde_con(filtros: &[Filtro], t: &Tabla, extra: &[String]) -> Result<String, Fallo> {
    let mut cs = filtros
        .iter()
        .map(|f| condicion(f, t))
        .collect::<Result<Vec<_>, _>>()?;
    cs.extend(extra.iter().cloned());
    if cs.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(" WHERE {}", cs.join(" AND ")))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::esquema::desde_metadatos;
    use crate::esquema::pruebas::{fila, metadatos};

    /// Una tabla con un tipo de cada.
    fn tabla() -> Tabla {
        let f = |nombre: &str, tipo: &str, tam: i64, dig: i64, ord: i64| {
            fila(Some("K"), "T", nombre, tipo, tam, dig, 1, ord, None)
        };
        let m = metadatos(vec![
            f("id", "BIGINT", 0, 0, 1),
            f("entero", "BIGINT", 0, 0, 2),
            f("importe", "DECIMAL", 12, 2, 3),
            f("real", "DOUBLE", 0, 0, 4),
            f("texto", "VARCHAR", 0, 0, 5),
            f("dia", "DATE", 0, 0, 6),
            f("instante", "TIMESTAMP", 0, 0, 7),
            f("logico", "BOOLEAN", 0, 0, 8),
            f("bin", "VARBINARY", 0, 0, 9),
            f("lista", "INTEGER ARRAY", 0, 0, 10),
            f("libre", "DECIMAL", 0, 0, 11),
            f("local", "TIMESTAMP", 0, 0, 12),
        ]);
        let mut t = desde_metadatos(Some("K"), "T", &m).unwrap().unwrap();
        // El `DATE` declarado `Date`, como hace el kit.
        let dia = t.columnas.iter_mut().find(|c| c.nombre == "dia").unwrap();
        dia.tipo = Tipo::Fecha;
        let local = t.columnas.iter_mut().find(|c| c.nombre == "local").unwrap();
        local.tipo = Tipo::FechaHora;
        t
    }

    fn uno(c: &str, op: &str, v: &str) -> Result<String, Fallo> {
        condicion(&Filtro::uno(c, op, v), &tabla())
    }

    fn lista(c: &str, v: &[&str]) -> Result<String, Fallo> {
        condicion(
            &Filtro {
                columna: c.into(),
                operador: "in".into(),
                valor: Valor::Lista(v.iter().map(|s| s.to_string()).collect()),
            },
            &tabla(),
        )
    }

    fn sin(c: &str, op: &str) -> Result<String, Fallo> {
        condicion(
            &Filtro {
                columna: c.into(),
                operador: op.into(),
                valor: Valor::Ninguno,
            },
            &tabla(),
        )
    }

    #[test]
    fn el_literal_de_texto_duplica_comillas_y_barras() {
        assert_eq!(literal_texto("ana"), "'ana'");
        assert_eq!(literal_texto("o'neil"), "'o''neil'");
        assert_eq!(literal_texto("a\\b"), "'a\\\\b'");
        // Un valor que acaba en barra no se come la comilla de cierre.
        assert_eq!(literal_texto("x\\"), "'x\\\\'");
        // Lo que sería una inyección queda dentro del literal.
        let v = "x\\' OR 1=1 --";
        assert_eq!(literal_texto(v), "'x\\\\'' OR 1=1 --'");
        assert_eq!(literal_texto("línea\nnueva"), "'línea\nnueva'");
    }

    #[test]
    fn cada_operador_sobre_cada_tipo() {
        assert_eq!(uno("texto", "eq", "ana").unwrap(), "\"texto\" = 'ana'");
        assert_eq!(uno("texto", "neq", "ana").unwrap(), "\"texto\" <> 'ana'");
        assert_eq!(
            uno("entero", "eq", "-9223372036854775808").unwrap(),
            "\"entero\" = -9223372036854775808"
        );
        assert_eq!(uno("entero", "ge", "3").unwrap(), "\"entero\" >= 3");
        assert_eq!(uno("entero", "lt", "1").unwrap(), "\"entero\" < 1");
        assert_eq!(uno("entero", "gt", "3").unwrap(), "\"entero\" > 3");
        assert_eq!(uno("entero", "le", "1").unwrap(), "\"entero\" <= 1");
        assert_eq!(
            uno("importe", "ge", "9999999999.99").unwrap(),
            "\"importe\" >= 9999999999.99"
        );
        assert_eq!(uno("real", "gt", "2.5").unwrap(), "\"real\" > 2.5");
        assert_eq!(uno("logico", "eq", "false").unwrap(), "\"logico\" = FALSE");
        assert_eq!(
            uno("dia", "eq", "2000-02-29").unwrap(),
            "\"dia\" = TO_DATE('2000-02-29', 'yyyy-MM-dd', 'GMT')"
        );
        assert_eq!(
            uno("instante", "ge", "2099-12-31T23:59:59.999999Z").unwrap(),
            "\"instante\" >= TIMESTAMP '2099-12-31 23:59:59.999999'"
        );
        assert_eq!(
            uno("instante", "eq", "2000-02-29T12:30:00.5Z").unwrap(),
            "\"instante\" = TIMESTAMP '2000-02-29 12:30:00.5'"
        );
        assert_eq!(
            uno("local", "ge", "2026-10-03T23:59:59").unwrap(),
            "\"local\" >= TIMESTAMP '2026-10-03 23:59:59'"
        );
        assert_eq!(sin("texto", "isNull").unwrap(), "\"texto\" IS NULL");
        assert_eq!(
            sin("logico", "isNotNull").unwrap(),
            "\"logico\" IS NOT NULL"
        );
        assert_eq!(uno("texto", "like", "%n%").unwrap(), "\"texto\" LIKE '%n%'");
        assert_eq!(
            lista("texto", &["ana", "Ana", "ñandú"]).unwrap(),
            "\"texto\" IN ('ana', 'Ana', 'ñandú')"
        );
        assert_eq!(
            lista("entero", &["1", "2", "99"]).unwrap(),
            "\"entero\" IN (1, 2, 99)"
        );
    }

    #[test]
    fn un_in_vacio_no_deja_pasar_ninguna_fila() {
        // `IN ()` es un error de sintaxis en Phoenix.
        assert_eq!(lista("texto", &[]).unwrap(), NADA);
        assert_eq!(lista("entero", &[]).unwrap(), NADA);
    }

    #[test]
    fn la_cadena_vacia_se_normaliza_porque_en_phoenix_es_nulo() {
        assert_eq!(uno("texto", "eq", "").unwrap(), NADA);
        assert_eq!(uno("texto", "lt", "").unwrap(), NADA);
        assert_eq!(uno("texto", "le", "").unwrap(), NADA);
        assert_eq!(uno("texto", "like", "").unwrap(), NADA);
        // «lo que no es vacío» son todas las filas no nulas.
        for op in ["neq", "gt", "ge"] {
            assert_eq!(
                uno("texto", op, "").unwrap(),
                "\"texto\" IS NOT NULL",
                "{op}"
            );
        }
        // En un `in`, el vacío se descarta; si solo había eso, no pasa nada.
        assert_eq!(
            lista("texto", &["", "ana"]).unwrap(),
            "\"texto\" IN ('ana')"
        );
        assert_eq!(lista("texto", &[""]).unwrap(), NADA);
    }

    #[test]
    fn un_valor_con_forma_de_inyeccion_se_queda_en_su_literal() {
        let c = uno("texto", "eq", "x\\' OR 1=1 --").unwrap();
        assert_eq!(c, "\"texto\" = 'x\\\\'' OR 1=1 --'");
        // Todas las comillas simples del valor están duplicadas: el literal no se cierra antes del final.
        let dentro = &c["\"texto\" = ".len()..];
        let cuerpo = &dentro[1..dentro.len() - 1];
        assert_eq!(cuerpo.replace("''", "").matches('\'').count(), 0);
    }

    #[test]
    fn like_con_barra_se_rechaza_y_sin_ella_va() {
        let e = uno("texto", "like", "a\\_b").unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("escape"), "{}", e.mensaje);
        assert!(uno("texto", "like", "a_b").is_ok());
        // Un `like` sobre algo que no es texto no se inventa.
        assert_eq!(
            uno("entero", "like", "1%").unwrap_err().codigo,
            Codigo::Operador
        );
    }

    #[test]
    fn lo_que_no_se_sabe_escribir_se_rechaza_con_operador() {
        for (c, op, v) in [
            ("real", "eq", "NaN"),
            ("real", "gt", "Infinity"),
            ("entero", "eq", "uno"),
            ("entero", "eq", "1.5"),
            ("importe", "eq", "0.005"),
            ("logico", "eq", "si"),
            ("dia", "eq", "ayer"),
            ("instante", "eq", "2026-10-04"),
            ("texto", "eq", "a\0b"),
        ] {
            let e = uno(c, op, v).unwrap_err();
            assert_eq!(e.codigo, Codigo::Operador, "{c} {op} {v}: {}", e.mensaje);
        }
        // Columnas que no se comparan como SQL.
        for c in ["bin", "lista", "libre"] {
            let e = uno(c, "eq", "1").unwrap_err();
            assert_eq!(e.codigo, Codigo::Operador, "{c}");
            assert!(e.mensaje.contains(c), "{}", e.mensaje);
        }
        // Un operador fuera del vocabulario, y un valor con la forma de otro operador.
        assert_eq!(
            uno("texto", "regex", "a").unwrap_err().codigo,
            Codigo::Operador
        );
        assert_eq!(
            condicion(
                &Filtro {
                    columna: "texto".into(),
                    operador: "eq".into(),
                    valor: Valor::Lista(vec!["a".into()])
                },
                &tabla()
            )
            .unwrap_err()
            .codigo,
            Codigo::Operador
        );
        assert_eq!(
            condicion(&Filtro::uno("texto", "isNull", "x"), &tabla())
                .unwrap_err()
                .codigo,
            Codigo::Operador
        );
    }

    #[test]
    fn una_columna_que_no_esta_es_esquema() {
        let e = uno("fantasma", "eq", "1").unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("fantasma"));
    }

    #[test]
    fn el_orden_pone_los_nulos_al_final_en_los_dos_sentidos() {
        let t = tabla();
        let o = |c: &str, d: bool| Orden {
            columna: c.into(),
            descendente: d,
        };
        assert_eq!(orden(&[], &t).unwrap(), "");
        assert_eq!(
            orden(&[o("entero", false)], &t).unwrap(),
            " ORDER BY \"entero\" ASC NULLS LAST"
        );
        assert_eq!(
            orden(&[o("entero", true), o("id", false)], &t).unwrap(),
            " ORDER BY \"entero\" DESC NULLS LAST, \"id\" ASC NULLS LAST"
        );
        assert_eq!(
            orden(&[o("bin", false)], &t).unwrap_err().codigo,
            Codigo::Operador
        );
        assert_eq!(
            orden(&[o("zz", false)], &t).unwrap_err().codigo,
            Codigo::Esquema
        );
    }

    #[test]
    fn el_where_une_con_and_y_es_vacio_sin_filtros() {
        let t = tabla();
        assert_eq!(donde(&[], &t).unwrap(), "");
        assert_eq!(
            donde(
                &[
                    Filtro::uno("entero", "ge", "3"),
                    Filtro::uno("texto", "eq", "a")
                ],
                &t
            )
            .unwrap(),
            " WHERE \"entero\" >= 3 AND \"texto\" = 'a'"
        );
    }
}
