//! **La pregunta, servida** — `POST /vistas/{ns}/{n}/ejecutar` (ADR 0030 W1 ④).
//!
//! Es `ore ask` detrás de la puerta: el servidor clona el árbol, corre
//! `ore ask . --vista <ns>.<n> --limite N` y devuelve la cabecera con las filas
//! dentro. **Síncrono**, y no por la cola: medido (medida W1 §D), un Job tarda
//! 100–160 s y un *Run* en el editor tiene que contestar en segundos; el clon
//! cuesta un segundo, traer la copia menos, y el residuo, milisegundos.
//!
//! # Lo que este verbo abre, y por qué es hermético igual
//!
//! `mando::HERMETICOS` dice que el plano de control no necesita la credencial
//! de ningún origen, y sigue siendo verdad: `ask` **no abre un origen**. Lee
//! **la copia del inquilino**, que es nuestra —el bucket que el aprovisionador
//! creó para la celda— con la identidad del pod, que tiene `objectViewer` y
//! nada más («`ore-serve-<n>` lee la copia, y no escribe», aprovisionador ③b,
//! puesto para esto). Quien lee es `ore-store-gcs`, un programa aparte con su
//! cierre de dependencias; `ore` sigue sin abrir un socket. Y si la copia no
//! está, se niega: no hay camino por el que esta ruta llegue a un origen.
//!
//! # Los códigos
//!
//! - `200` la respuesta: `{view, copia{de, clave}, plan, compensacion,
//!   columnas, limite, filas, leidas, trabajo, datos}`;
//! - `404` la vista no está en el árbol;
//! - `409` una copia la contesta **pero no está hecha**: es lo accionable
//!   —copiar— y el mismo código que `invocar` da por lo mismo;
//! - `422` no compila donde vive, o ninguna copia contesta (con los motivos);
//! - `502` el almacén o el motor fallaron: lo que dijeron, tal cual.
//!
//! # El límite
//!
//! Por defecto 200 filas y como mucho 5 000: es una respuesta para mirar, no
//! una copia. El plan se ejecuta entero —el límite recorta la respuesta, no lo
//! leído (`ore ask`)— así que un agregado sale bien aunque se pidan dos filas.

use crate::mando;
use crate::rutas::{Servidor, token};
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::path::Path;

const LIMITE_POR_DEFECTO: u64 = 200;
const LIMITE_MAXIMO: u64 = 5_000;

/// Lo que el cuerpo puede pedir: `{"limite": N}`. Vacío es el defecto.
pub(crate) fn limite_de(cuerpo: &str) -> Result<u64, String> {
    if cuerpo.trim().is_empty() {
        return Ok(LIMITE_POR_DEFECTO);
    }
    let n = ore_core::parse::parse(cuerpo).map_err(|_| "el cuerpo no es JSON".to_string())?;
    let Some((_, l)) = n.get("limite") else {
        return Ok(LIMITE_POR_DEFECTO);
    };
    let l: u64 = l
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "`limite` tiene que ser un entero positivo".to_string())?;
    if l == 0 || l > LIMITE_MAXIMO {
        return Err(format!("`limite` va de 1 a {LIMITE_MAXIMO}"));
    }
    Ok(l)
}

impl Servidor {
    /// `POST /vistas/{ns}/{n}/ejecutar`: la pregunta, contestada sobre la copia.
    pub(crate) fn ejecutar(&self, raiz: &Path, ns: &str, nombre: &str, cuerpo: &str) -> Respuesta {
        if let Err(m) = token(ns) {
            return Respuesta::error(422, format!("espacio de nombres: {m}"));
        }
        if let Err(m) = token(nombre) {
            return Respuesta::error(422, format!("nombre: {m}"));
        }
        let limite = match limite_de(cuerpo) {
            Ok(l) => l,
            Err(m) => return Respuesta::error(422, m),
        };
        let qn = format!("{ns}.{nombre}");
        let salida = match mando::correr(
            &self.binario,
            raiz,
            &[
                "ask".into(),
                ".".into(),
                "--vista".into(),
                qn.clone(),
                "--limite".into(),
                limite.to_string(),
            ],
        ) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(500, e.to_string()),
        };
        if salida.codigo != 0 {
            let motivo = salida
                .stderr
                .trim()
                .strip_prefix("error: ")
                .unwrap_or(salida.stderr.trim())
                .to_string();
            let codigo = match salida.codigo {
                65 if motivo.starts_with("no hay ninguna `View`") => 404,
                65 if motivo.contains("no está hecha") || motivo.contains("no está:") => 409,
                65 | 66 => 422,
                _ => 502,
            };
            return Respuesta::error(codigo, motivo);
        }
        // La cabecera en la primera línea, las filas debajo: se devuelven juntas.
        let mut lineas = salida.stdout.lines().filter(|l| !l.trim().is_empty());
        let Some(cab) = lineas.next() else {
            return Respuesta::error(502, "`ore ask` no devolvió la cabecera");
        };
        let mut cabecera = match ore_core::parse::parse(cab) {
            Ok(n) => crate::rutas::de_node(&n),
            Err(e) => {
                return Respuesta::error(
                    502,
                    format!("la cabecera de `ore ask` no analiza: {e:?}"),
                );
            }
        };
        let mut datos = Vec::new();
        for l in lineas {
            match ore_core::parse::parse(l) {
                Ok(n) => datos.push(crate::rutas::de_node(&n)),
                Err(e) => {
                    return Respuesta::error(
                        502,
                        format!("una fila de `ore ask` no analiza: {e:?}"),
                    );
                }
            }
        }
        if let Json::Obj(m) = &mut cabecera {
            m.insert("view".into(), Json::s(&qn));
            m.remove("vista");
            m.insert("datos".into(), Json::Arr(datos));
        }
        Respuesta::ok(cabecera)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_limite_tiene_defecto_y_techo() {
        assert_eq!(limite_de(""), Ok(LIMITE_POR_DEFECTO));
        assert_eq!(limite_de("{}"), Ok(LIMITE_POR_DEFECTO));
        assert_eq!(limite_de(r#"{"limite": 7}"#), Ok(7));
        assert!(limite_de(r#"{"limite": 0}"#).is_err());
        assert!(limite_de(r#"{"limite": 999999}"#).is_err());
        assert!(limite_de(r#"{"limite": "muchas"}"#).is_err());
    }
}
