//! El segundo verbo: **devolver filas**.
//!
//! La petición es un **fragmento del plan**, no SQL
//! (`docs/decisions/0008-el-protocolo-del-driver.md`): la misma para todos los
//! drivers, y **traducir es del driver**. Si viajara SQL, el ejecutor tendría
//! que conocer el dialecto de cada origen, y añadir una familia de fuentes
//! dejaría de ser un binario nuevo para ser un cambio en el planificador.
//!
//! # Por qué el SQL se construye en una función pura
//!
//! Porque *«el SQL emitido contiene solo las columnas proyectadas»* tiene que
//! ser un **aserto** y no una promesa, y un aserto que exigiera un servidor no
//! se ejecutaría nunca en la suite.
//!
//! Y ahí es donde la máscara se hace efectiva: una propiedad `redact` no está en
//! el plan, luego no está en la petición, luego **no puede estar en el SQL**. La
//! salvaguarda es estructural — no hay ningún punto donde alguien pueda
//! olvidarse de aplicarla, porque no hay nada que aplicar.
//!
//! # El analizador que no hizo falta
//!
//! La petición es JSON, y **JSON es un subconjunto de YAML**: la lee el mismo
//! `ore_core::parse` que lee los documentos. Añadir un analizador de JSON para
//! esto habría sido la segunda gramática para la misma forma.


/// Lo que el motor pide. Nombres físicos ya resueltos: el driver no conoce el
/// modelo, solo el objeto y sus columnas.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Peticion {
    pub url: String,
    pub objeto: String,
    /// propiedad → columna. La clave es lo que sale en la fila; el valor, lo que
    /// va en el `SELECT`.
    pub proyeccion: Vec<(String, String)>,
    pub clave_columnas: Vec<String>,
    pub claves: Vec<Vec<String>>,
    pub filtros: Vec<(String, String)>,
}

pub fn leer_peticion(texto: &str) -> Result<Peticion, String> {
    let n = ore_core::parse::parse(texto).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    let cadena = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).unwrap_or("").to_string();
    let lista = |k: &str| -> Vec<String> {
        n.get(k)
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };

    let proyeccion: Vec<(String, String)> = n
        .get("proyeccion")
        .map(|(_, v)| {
            v.entries()
                .iter()
                .filter_map(|(k, c)| Some((k.as_str()?.to_string(), c.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let claves: Vec<Vec<String>> = n
        .get("claves")
        .map(|(_, v)| {
            v.items()
                .iter()
                .map(|t| t.items().iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .collect()
        })
        .unwrap_or_default();

    let filtros: Vec<(String, String)> = n
        .get("filtros")
        .map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|f| {
                    // Solo `eq`: es el único operador que un ámbito produce, y
                    // aceptar otros aquí sería admitir un predicado que nadie
                    // declaró.
                    match f.get("operador").and_then(|(_, o)| o.as_str()) {
                        Some("eq") | None => Some((
                            f.get("columna")?.1.as_str()?.to_string(),
                            f.get("valor")?.1.as_str()?.to_string(),
                        )),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let p = Peticion {
        url: cadena("url"),
        objeto: cadena("objeto"),
        proyeccion,
        clave_columnas: lista("claveColumnas"),
        claves,
        filtros,
    };
    if p.objeto.is_empty() {
        return Err("la petición no nombra ningún objeto".into());
    }
    if p.proyeccion.is_empty() {
        // Un plan con proyección vacía **no llega a lanzar el driver**, así que
        // si llega es que alguien construyó la petición a mano.
        return Err("la proyección está vacía: no hay nada que pedir".into());
    }
    Ok(p)
}

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
        condiciones.push(format!(
            "({}) IN ({})",
            cols.join(", "),
            tuplas.join(", ")
        ));
    }

    for (col, valor) in &p.filtros {
        params.push(valor.clone());
        condiciones.push(format!("{} = ${}", ident(col), params.len()));
    }

    if !condiciones.is_empty() {
        q.push_str(" WHERE ");
        q.push_str(&condiciones.join(" AND "));
    }
    (q, params)
}

/// Una fila, como objeto JSON con las **propiedades** como claves.
///
/// No las columnas físicas: el nombre físico es del binding y no tiene por qué
/// salir del driver.
pub fn fila(p: &Peticion, valores: &[Option<String>]) -> String {
    let obj: std::collections::BTreeMap<String, ore_core::json::Json> = p
        .proyeccion
        .iter()
        .zip(valores)
        .map(|((prop, _), v)| {
            (
                prop.clone(),
                match v {
                    Some(x) => ore_core::json::Json::s(x.as_str()),
                    // `null` y la cadena vacía no son lo mismo, y un driver que
                    // los confundiera haría indistinguible «no hay dato» de
                    // «hay dato y está vacío».
                    None => ore_core::json::Json::Str(String::new()),
                },
            )
        })
        .collect();
    ore_core::json::Json::Obj(obj).jcs()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            filtros: vec![("cost_center".into(), "finanzas".into())],
        }
    }

    /// El aserto de M2: **solo las columnas proyectadas**.
    #[test]
    fn el_sql_no_pide_una_columna_que_no_este_en_la_proyeccion() {
        let (q, _) = sql(&peticion());
        assert!(q.starts_with("SELECT \"base_pay\", \"employee_id\" FROM"), "{q}");
        assert!(!q.contains('*'), "un `SELECT *` traería la columna redactada: {q}");
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

    /// La petición es JSON, y la lee el mismo analizador que los documentos.
    #[test]
    fn la_peticion_se_lee_con_el_analizador_de_siempre() {
        let texto = r#"{"claveColumnas":["employee_id"],"claves":[["emp-7"]],
            "filtros":[{"columna":"cost_center","operador":"eq","valor":"finanzas"}],
            "objeto":"public.employees","proyeccion":{"baseSalary":"base_pay"},
            "url":"postgres://x"}"#;
        let p = leer_peticion(texto).expect("analiza");
        assert_eq!(p.objeto, "public.employees");
        assert_eq!(p.proyeccion, vec![("baseSalary".to_string(), "base_pay".to_string())]);
        assert_eq!(p.claves, vec![vec!["emp-7".to_string()]]);
        assert_eq!(p.filtros, vec![("cost_center".to_string(), "finanzas".to_string())]);
    }

    /// Una proyección vacía no es una petición: el plan que la produjera no
    /// habría llegado a lanzar el driver.
    #[test]
    fn una_proyeccion_vacia_se_rechaza() {
        let texto = r#"{"objeto":"t","proyeccion":{},"url":"x"}"#;
        assert!(leer_peticion(texto).is_err());
    }

    /// La fila sale con **propiedades**, no con columnas físicas.
    #[test]
    fn la_fila_habla_el_vocabulario_del_modelo() {
        let p = peticion();
        let f = fila(&p, &[Some("1000".into()), Some("emp-7".into())]);
        assert_eq!(f, r#"{"baseSalary":"1000","employeeId":"emp-7"}"#);
        assert!(!f.contains("base_pay"), "{f}");
    }
}
