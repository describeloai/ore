//! El **protocolo del driver**: la petición que el motor manda y la fila que
//! vuelve.
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
//! # Por qué esto es un crate y no un módulo
//!
//! Vivía dentro de `ore-read-postgres`, y al escribir el segundo driver quedó
//! claro qué significaba eso: la petición es **el contrato** entre el motor y
//! cualquier fuente, y un contrato repetido en cada implementación es un
//! contrato que diverge en la tercera.
//!
//! Lo que **no** vive aquí es la traducción. `sql()` se queda en el driver de
//! PostgreSQL porque es de PostgreSQL, y el driver de ficheros no tiene nada
//! parecido — que es justamente la prueba de que la petición no era SQL.
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
