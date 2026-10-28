//! **De la `Peticion` al `SELECT`** (ADR 0077 §6, paso 5): la proyección, los filtros, el orden y el límite.
//!
//! Los valores **nunca van en el texto del SQL**: cada uno es un parámetro del servidor (`{p0:Int64}`), con el tipo de la columna, y
//! se escapa en el transporte ([`crate::transporte::Param`]). Los nombres sí van en el SQL, entre comillas, y los que podrían
//! romperlas se rechazan.
//!
//! **Un valor que no cabe en el tipo de la columna es un error, no un valor distinto.** ClickHouse desborda en silencio
//! (`UInt8` ← 300 da 44) y recorta la fecha fuera de rango; aquí se comprueba el rango antes de atar.

use crate::rowbinary::{Tipo, analizar};
use crate::transporte::{Cliente, Param};
use ore_base::tipos::Fisico;
use ore_driver::{Codigo, Fallo, Filtro, Peticion, Valor};
use std::collections::HashMap;

/// `Date32` de ClickHouse: de 1900-01-01 a 2299-12-31, en días desde 1970.
const DIAS_MIN: i32 = -25_567;
const DIAS_MAX: i32 = 120_529;

/// Un identificador entre comillas de ClickHouse. Un nombre con comilla invertida, barra o control no se admite: no hay
/// nada legítimo que escapar y sí mucho que inyectar.
pub fn identificador(nombre: &str) -> Result<String, Fallo> {
    if nombre.is_empty()
        || nombre
            .chars()
            .any(|c| c == '`' || c == '\\' || c.is_control())
    {
        return Err(Fallo::operador(format!(
            "`{nombre}` no es un nombre que ORE ponga en un SELECT de ClickHouse"
        )));
    }
    Ok(format!("`{nombre}`"))
}

/// El objeto de la petición como `(base, tabla)`: `base.tabla`, o `tabla` en la base de la URL.
pub fn partes_del_objeto(objeto: &str, base: Option<&str>) -> Result<(String, String), Fallo> {
    match (objeto.split_once('.'), base) {
        (Some((b, t)), _) => Ok((b.to_string(), t.to_string())),
        (None, Some(b)) => Ok((b.to_string(), objeto.to_string())),
        (None, None) => Err(Fallo::operador(format!(
            "`{objeto}` no dice su base: `base.tabla`, o una base en la URL (`clickhouse://…/base`)"
        ))),
    }
}

/// Los valores atados de una consulta: `(nombre, valor)`, para `param_<nombre>`.
#[derive(Default, Debug)]
pub struct Atados(pub Vec<(String, String)>);

impl Atados {
    fn nuevo(&mut self, valor: String) -> String {
        let nombre = format!("p{}", self.0.len());
        self.0.push((nombre.clone(), valor));
        nombre
    }

    /// Para pasarlos al transporte.
    pub fn como_params(&self) -> Vec<(&str, Param<'_>)> {
        self.0
            .iter()
            .map(|(n, v)| (n.as_str(), Param::Texto(v)))
            .collect()
    }
}

/// Las columnas de una tabla y sus tipos, tal como el servidor los dice (`system.columns`: sólo las que el usuario puede ver).
pub fn tipos_de_la_tabla(
    c: &Cliente,
    base: &str,
    tabla: &str,
) -> Result<HashMap<String, String>, Fallo> {
    let hoja = c.texto_con(
        "SELECT name, type FROM system.columns WHERE database = {d:String} AND table = {t:String}",
        &[("d", Param::Texto(base)), ("t", Param::Texto(tabla))],
    )?;
    let mut m = HashMap::new();
    for linea in hoja.lines() {
        if let Some((n, t)) = linea.split_once('\t') {
            m.insert(desescapar(n), desescapar(t));
        }
    }
    if m.is_empty() {
        return Err(Fallo::new(
            Codigo::Objeto,
            format!("`{base}.{tabla}` no existe, o el usuario no puede verla"),
        ));
    }
    Ok(m)
}

/// El inverso de `escapar_texto`, para lo que el servidor devuelve en `TabSeparated`.
fn desescapar(t: &str) -> String {
    let mut s = String::with_capacity(t.len());
    let mut c = t.chars();
    while let Some(x) = c.next() {
        if x != '\\' {
            s.push(x);
            continue;
        }
        match c.next() {
            Some('t') => s.push('\t'),
            Some('n') => s.push('\n'),
            Some('r') => s.push('\r'),
            Some('0') => s.push('\0'),
            Some(otro) => s.push(otro),
            None => s.push('\\'),
        }
    }
    s
}

/// `Nullable(LowCardinality(T))` → `T`: el tipo con el que se ata un parámetro.
fn tipo_base(texto: &str) -> &str {
    let mut t = texto.trim();
    loop {
        let dentro = ["Nullable", "LowCardinality"].iter().find_map(|n| {
            t.strip_prefix(n)
                .and_then(|r| r.strip_prefix('('))
                .and_then(|r| r.strip_suffix(')'))
        });
        match dentro {
            Some(d) => t = d.trim(),
            None => return t,
        }
    }
}

fn sin_nulo(t: Tipo) -> Tipo {
    match t {
        Tipo::Nulable(i) => sin_nulo(*i),
        otro => otro,
    }
}

fn no_cabe(columna: &str, valor: &str, que: &str) -> Fallo {
    Fallo::operador(format!(
        "el valor `{valor}` no es {que}: no se compara con la columna `{columna}`"
    ))
}

/// El valor de un filtro como expresión de SQL con su parámetro, validado contra el tipo de la columna.
fn literal(
    columna: &str,
    texto_tipo: &str,
    tipo: &Tipo,
    valor: &str,
    at: &mut Atados,
) -> Result<String, Fallo> {
    let base = tipo_base(texto_tipo);
    match tipo {
        Tipo::Entero { bytes, signo } => {
            let n: i128 = valor
                .parse()
                .map_err(|_| no_cabe(columna, valor, &format!("un entero ({base})")))?;
            let bits = u32::from(*bytes) * 8;
            let (min, max) = if *signo {
                (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
            } else {
                (0, (1i128 << bits) - 1)
            };
            if n < min || n > max {
                return Err(no_cabe(
                    columna,
                    valor,
                    &format!("un {base} ({min} a {max}); ClickHouse lo desbordaría en silencio"),
                ));
            }
            Ok(format!("{{{}:{base}}}", at.nuevo(n.to_string())))
        }
        Tipo::Entero64SinSigno => {
            let n: u64 = valor
                .parse()
                .map_err(|_| no_cabe(columna, valor, "un UInt64"))?;
            Ok(format!("{{{}:UInt64}}", at.nuevo(n.to_string())))
        }
        Tipo::Real { .. } => {
            let f = match Fisico::Real.analizar(valor) {
                Some(ore_base::tipos::Valor::Real(f)) => f,
                _ => return Err(no_cabe(columna, valor, "un real")),
            };
            // ClickHouse lee `nan`, `inf` y `-inf`; no `NaN` ni `Infinity`.
            let t = if f.is_nan() {
                "nan".to_string()
            } else if f.is_infinite() {
                if f > 0.0 { "inf" } else { "-inf" }.to_string()
            } else {
                valor.to_string()
            };
            Ok(format!("{{{}:{base}}}", at.nuevo(t)))
        }
        Tipo::Logico => match valor {
            "true" | "false" => Ok(format!("{{{}:Bool}}", at.nuevo(valor.to_string()))),
            _ => Err(no_cabe(columna, valor, "un booleano (true o false)")),
        },
        Tipo::Decimal { .. } => {
            // Con la escala **del valor**, no la de la columna: `< 0.005` sobre `Decimal(12, 2)` no se redondea a `< 0.01`.
            let escala = valor.split_once('.').map_or(0, |(_, d)| d.len());
            let ok = escala <= 38
                && Fisico::Decimal {
                    precision: 38,
                    escala: escala as u8,
                }
                .analizar(valor)
                .is_some();
            if !ok {
                return Err(no_cabe(columna, valor, "un decimal de hasta 38 dígitos"));
            }
            Ok(format!(
                "{{{}:Decimal(38, {escala})}}",
                at.nuevo(valor.to_string())
            ))
        }
        Tipo::Fecha { .. } => match Fisico::Fecha.analizar(valor) {
            Some(ore_base::tipos::Valor::Fecha(d)) if (DIAS_MIN..=DIAS_MAX).contains(&d) => {
                Ok(format!("toDate32({{{}:Int32}})", at.nuevo(d.to_string())))
            }
            Some(_) => Err(no_cabe(
                columna,
                valor,
                "una fecha entre 1900 y 2299 (el rango de ClickHouse)",
            )),
            None => Err(no_cabe(columna, valor, "una fecha")),
        },
        Tipo::Instante { pared, .. } => {
            let fisico = if *pared {
                Fisico::FechaHora
            } else {
                Fisico::Instante
            };
            match fisico.analizar(valor) {
                Some(
                    ore_base::tipos::Valor::Instante(us) | ore_base::tipos::Valor::FechaHora(us),
                ) => Ok(format!(
                    "fromUnixTimestamp64Micro({{{}:Int64}}, 'UTC')",
                    at.nuevo(us.to_string())
                )),
                _ => Err(no_cabe(
                    columna,
                    valor,
                    if *pared {
                        "una fecha y hora sin zona"
                    } else {
                        "un instante (con zona)"
                    },
                )),
            }
        }
        Tipo::Texto | Tipo::TextoFijo(_) | Tipo::Enumerado { .. } => {
            Ok(format!("{{{}:String}}", at.nuevo(valor.to_string())))
        }
        Tipo::Uuid | Tipo::Ipv4 | Tipo::Ipv6 => {
            Ok(format!("{{{}:{base}}}", at.nuevo(valor.to_string())))
        }
        Tipo::Nulable(_) => unreachable!("se quita antes"),
    }
}

/// La condición de un filtro, con sus valores atados.
pub fn condicion(
    f: &Filtro,
    texto_tipo: &str,
    declarado: Option<&str>,
    at: &mut Atados,
) -> Result<String, Fallo> {
    let col = identificador(&f.columna)?;
    let tipo = analizar(texto_tipo)
        .map_err(|m| Fallo::new(Codigo::Esquema, format!("columna `{}`: {m}", f.columna)))?;
    let tipo = match declarado {
        Some(d) => tipo.declarado(d),
        None => tipo,
    };
    let tipo = sin_nulo(tipo);
    let simbolo = match f.operador.as_str() {
        "eq" => "=",
        "neq" => "!=",
        "lt" => "<",
        "le" => "<=",
        "gt" => ">",
        "ge" => ">=",
        _ => "",
    };
    match (f.operador.as_str(), &f.valor) {
        ("isNull", _) => Ok(format!("{col} IS NULL")),
        ("isNotNull", _) => Ok(format!("{col} IS NOT NULL")),
        ("in", Valor::Lista(v)) if v.is_empty() => Ok("0".into()),
        ("in", Valor::Lista(v)) => {
            let l = v
                .iter()
                .map(|x| literal(&f.columna, texto_tipo, &tipo, x, at))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            Ok(format!("{col} IN ({l})"))
        }
        ("like", Valor::Uno(v)) => match tipo {
            Tipo::Texto | Tipo::TextoFijo(_) => Ok(format!(
                "{col} LIKE {}",
                literal(&f.columna, texto_tipo, &tipo, v, at)?
            )),
            _ => Err(Fallo::operador(format!(
                "`like` sobre `{}`: sólo se empuja sobre un String",
                f.columna
            ))),
        },
        (_, Valor::Uno(v)) if !simbolo.is_empty() => Ok(format!(
            "{col} {simbolo} {}",
            literal(&f.columna, texto_tipo, &tipo, v, at)?
        )),
        (op, _) => Err(Fallo::operador(format!(
            "`{op}`: el valor no es el de ese operador"
        ))),
    }
}

/// El `SELECT` completo y sus valores atados. Las columnas se **nombran siempre**, en el orden de la proyección: `SELECT *` omite
/// las `MATERIALIZED` y las `ALIAS`. `tipos` son los de la tabla, y sólo hacen falta si hay filtros.
pub fn consulta(
    p: &Peticion,
    base: Option<&str>,
    tipos: &HashMap<String, String>,
    con_final: bool,
) -> Result<(String, Atados), Fallo> {
    if p.proyeccion.is_empty() {
        return Err(Fallo::operador("la proyección está vacía"));
    }
    let columnas = p
        .proyeccion
        .iter()
        .map(|(_, c)| identificador(c))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let (b, t) = partes_del_objeto(&p.objeto, base)?;
    let mut sql = format!(
        "SELECT {columnas} FROM {}.{}{}",
        identificador(&b)?,
        identificador(&t)?,
        // Las tablas que fusionan por clave dejan repetidos hasta que se fusionan: sin `FINAL` la lectura no es la tabla (§3.3).
        if con_final { " FINAL" } else { "" }
    );
    let mut at = Atados::default();
    let mut condiciones = Vec::new();
    for f in &p.filtros {
        let texto = tipos.get(&f.columna).ok_or_else(|| {
            Fallo::new(
                Codigo::Esquema,
                format!("`{}` no tiene la columna `{}`", p.objeto, f.columna),
            )
        })?;
        let declarado = p
            .tipos
            .iter()
            .find(|(c, _)| c == &f.columna)
            .map(|(_, d)| d.as_str());
        condiciones.push(condicion(f, texto, declarado, &mut at)?);
    }
    if !condiciones.is_empty() {
        sql.push_str(&format!(" WHERE {}", condiciones.join(" AND ")));
    }
    if !p.orden.is_empty() {
        let o = p
            .orden
            .iter()
            .map(|o| {
                // Los nulos al final, también en `DESC` (v1alpha24 §3).
                Ok(format!(
                    "{} {} NULLS LAST",
                    identificador(&o.columna)?,
                    if o.descendente { "DESC" } else { "ASC" }
                ))
            })
            .collect::<Result<Vec<_>, Fallo>>()?
            .join(", ");
        sql.push_str(&format!(" ORDER BY {o}"));
    }
    if let Some(n) = p.limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }
    Ok((sql, at))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn tipos() -> HashMap<String, String> {
        [
            ("id", "Int64"),
            ("peq", "UInt8"),
            ("texto", "Nullable(String)"),
            ("importe", "Nullable(Decimal(12, 2))"),
            ("fecha", "Nullable(Date32)"),
            ("t", "Nullable(DateTime64(6, 'UTC'))"),
            ("n", "UInt64"),
            ("logico", "Nullable(Bool)"),
            ("real", "Nullable(Float64)"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    fn con(json: &str) -> Result<(String, Atados), Fallo> {
        let p = ore_driver::leer_peticion(&format!(
            r#"{{"url":"clickhouse://x","objeto":"kit.t","proyeccion":{{"a":"id"}},{json}}}"#
        ))
        .expect("petición");
        consulta(&p, None, &tipos(), false)
    }

    fn filtro(c: &str, op: &str, v: &str) -> String {
        format!(r#""filtros":[{{"columna":"{c}","operador":"{op}","valor":{v}}}]"#)
    }

    #[test]
    fn sin_filtros_es_el_select_de_siempre() {
        let (sql, _) = con(r#""limit":5"#).unwrap();
        assert_eq!(sql, "SELECT `id` FROM `kit`.`t` LIMIT 5");
        let (sql, at) = con(r#""formato":"arrow""#).unwrap();
        assert_eq!(sql, "SELECT `id` FROM `kit`.`t`");
        assert!(at.0.is_empty());
    }

    #[test]
    fn los_valores_van_atados_y_con_el_tipo_de_la_columna() {
        let (sql, at) = con(&filtro("id", "ge", r#""3""#)).unwrap();
        assert_eq!(sql, "SELECT `id` FROM `kit`.`t` WHERE `id` >= {p0:Int64}");
        assert_eq!(at.0, vec![("p0".to_string(), "3".to_string())]);
        // Una comilla en el valor no llega al SQL.
        let (sql, at) = con(&filtro("texto", "eq", r#""a' OR 1=1 --""#)).unwrap();
        assert!(!sql.contains("OR 1=1"), "{sql}");
        assert_eq!(at.0[0].1, "a' OR 1=1 --");
    }

    #[test]
    fn el_entero_que_no_cabe_es_un_error_y_no_un_valor_distinto() {
        for (v, ok) in [("255", true), ("256", false), ("-1", false), ("300", false)] {
            let r = con(&filtro("peq", "eq", &format!(r#""{v}""#)));
            assert_eq!(r.is_ok(), ok, "UInt8 ← {v}");
        }
        let e = con(&filtro("peq", "eq", r#""300""#)).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("UInt8"), "{e:?}");
        assert!(con(&filtro("id", "eq", r#""9223372036854775808""#)).is_err());
        assert!(con(&filtro("id", "eq", r#""9223372036854775807""#)).is_ok());
        assert!(con(&filtro("n", "eq", r#""18446744073709551615""#)).is_ok());
        assert!(con(&filtro("n", "eq", r#""18446744073709551616""#)).is_err());
        assert!(con(&filtro("id", "eq", r#""1.5""#)).is_err());
    }

    #[test]
    fn la_fecha_fuera_del_rango_de_clickhouse_se_rechaza() {
        assert!(con(&filtro("fecha", "eq", r#""2299-12-31""#)).is_ok());
        assert!(con(&filtro("fecha", "eq", r#""1900-01-01""#)).is_ok());
        assert!(con(&filtro("fecha", "eq", r#""2300-01-01""#)).is_err());
        assert!(con(&filtro("fecha", "eq", r#""1899-12-31""#)).is_err());
        assert!(con(&filtro("fecha", "eq", r#""no es fecha""#)).is_err());
    }

    #[test]
    fn el_decimal_lleva_la_escala_de_su_valor_y_no_la_de_la_columna() {
        let (sql, at) = con(&filtro("importe", "lt", r#""0.005""#)).unwrap();
        assert!(sql.contains("{p0:Decimal(38, 3)}"), "{sql}");
        assert_eq!(at.0[0].1, "0.005");
    }

    #[test]
    fn el_instante_va_como_microsegundos() {
        let (sql, at) = con(&filtro("t", "lt", r#""2026-10-04T10:00:00Z""#)).unwrap();
        assert!(
            sql.contains("fromUnixTimestamp64Micro({p0:Int64}, 'UTC')"),
            "{sql}"
        );
        assert_eq!(at.0[0].1, "1791108000000000");
    }

    #[test]
    fn in_vacio_no_deja_pasar_nada_y_los_nulos_no_llevan_valor() {
        let (sql, _) = con(&filtro("id", "in", "[]")).unwrap();
        assert!(sql.ends_with("WHERE 0"), "{sql}");
        let (sql, at) = con(&filtro("id", "in", r#"["1","2"]"#)).unwrap();
        assert!(sql.ends_with("`id` IN ({p0:Int64}, {p1:Int64})"), "{sql}");
        assert_eq!(at.0.len(), 2);
        let (sql, _) = con(r#""filtros":[{"columna":"texto","operador":"isNull"}]"#).unwrap();
        assert!(sql.ends_with("`texto` IS NULL"), "{sql}");
    }

    #[test]
    fn like_solo_sobre_texto() {
        assert!(con(&filtro("texto", "like", r#""a_b""#)).is_ok());
        assert!(con(&filtro("id", "like", r#""1%""#)).is_err());
    }

    #[test]
    fn el_orden_pone_los_nulos_al_final_y_el_limite_cierra() {
        let p = ore_driver::leer_peticion(
            r#"{"url":"clickhouse://x","objeto":"kit.t","proyeccion":{"a":"id"},"orderBy":[{"columna":"importe","direccion":"desc"},{"columna":"id"}],"limit":3}"#,
        )
        .unwrap();
        let (sql, _) = consulta(&p, None, &tipos(), false).unwrap();
        assert_eq!(
            sql,
            "SELECT `id` FROM `kit`.`t` ORDER BY `importe` DESC NULLS LAST, `id` ASC NULLS LAST LIMIT 3"
        );
    }

    #[test]
    fn una_columna_que_la_tabla_no_tiene_es_un_cambio_de_esquema() {
        let e = con(&filtro("no_existe", "eq", r#""1""#)).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
    }

    #[test]
    fn el_nombre_con_comilla_no_llega_al_sql() {
        let p = ore_driver::leer_peticion(
            r#"{"url":"clickhouse://x","objeto":"kit.t","proyeccion":{"a":"id` FROM system.users --"}}"#,
        )
        .unwrap();
        assert_eq!(
            consulta(&p, None, &tipos(), false).unwrap_err().codigo,
            Codigo::Operador
        );
    }

    #[test]
    fn el_objeto_sin_base_usa_la_de_la_url() {
        assert_eq!(
            partes_del_objeto("t", Some("kit")).unwrap(),
            ("kit".into(), "t".into())
        );
        assert_eq!(
            partes_del_objeto("a.t", Some("kit")).unwrap(),
            ("a".into(), "t".into())
        );
        assert!(partes_del_objeto("t", None).is_err());
    }

    #[test]
    fn final_va_detras_de_la_tabla_y_antes_del_where() {
        let p = ore_driver::leer_peticion(
            r#"{"url":"clickhouse://x","objeto":"kit.t","proyeccion":{"a":"id"},"filtros":[{"columna":"id","operador":"eq","valor":"1"}]}"#,
        )
        .unwrap();
        let (sql, _) = consulta(&p, None, &tipos(), true).unwrap();
        assert_eq!(
            sql,
            "SELECT `id` FROM `kit`.`t` FINAL WHERE `id` = {p0:Int64}"
        );
    }
}
