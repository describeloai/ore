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
//! - `200` la respuesta: `{view, copia{de, metadata_location | clave}, plan, compensacion,
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
    pub(crate) fn ejecutar(
        &self,
        raiz: &Path,
        ns: &str,
        schema: &str,
        nombre: &str,
        cuerpo: &str,
    ) -> Respuesta {
        if let Err(m) = token(ns) {
            return Respuesta::error(422, format!("espacio de nombres: {m}"));
        }
        if let Err(m) = token(schema) {
            return Respuesta::error(422, format!("schema: {m}"));
        }
        if let Err(m) = token(nombre) {
            return Respuesta::error(422, format!("nombre: {m}"));
        }
        let limite = match limite_de(cuerpo) {
            Ok(l) => l,
            Err(m) => return Respuesta::error(422, m),
        };
        let qn = ore_core::normalize::corto(ns, schema, nombre);
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
                // Una vista SQL sin copia (ADR 0040 paso 4c): se lee en un
                // puesto, con la identidad de quien la lee; aquí no hay qué leer.
                65 if motivo.contains("se lee en un puesto") => 409,
                65 | 66 => 422,
                _ => 502,
            };
            if codigo == 409 {
                // El 409 dice que la copia no está; esto dice SI ESTÁ EN MARCHA.
                let copia = self.estado_de_la_copia(raiz, &motivo);
                let mut cuerpo = Json::obj([("error", Json::s(&motivo))]);
                if let Json::Obj(m) = &mut cuerpo {
                    m.insert("copia".into(), copia);
                }
                return Respuesta { codigo, cuerpo };
            }
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

    /// **¿La copia está en marcha?** Lo que el 409 no decía, y que hacía que
    /// una base recién creada pareciera rota: medido el 2026-09-18, 435 s de
    /// media por alta con la consola diciendo «not made yet» y un botón de
    /// copiar mientras el Job estaba en la cola o corriendo — y el botón,
    /// pulsado en esa ventana, encoló un `rehacer` que releyó el origen.
    ///
    /// Tres estados, los mismos que `estado` da para una fuente:
    ///
    /// ```text
    /// encolada   la cola tiene un `48-la-copia*.yaml` que la nombra y es MÁS
    ///            NUEVO que su último informe (o no hay informe): la pasada no
    ///            ha terminado. `desde` es el commit de la cola
    /// fallida    su informe dice `error` y ningún Job más nuevo la nombra:
    ///            `motivo` es el del informe. Aquí sí sirve copiar
    /// pendiente  nadie la ha encolado
    /// ```
    ///
    /// Mirar la cola cuesta un clon, así que sólo se hace en el 409.
    fn estado_de_la_copia(&self, raiz: &Path, motivo: &str) -> Json {
        // La vista cuya copia contestaría: la primera «`x` la contesta».
        let partes: Vec<&str> = motivo.split('`').collect();
        let vista = partes
            .windows(2)
            .find(|w| w[1].starts_with(" la contesta"))
            .map(|w| w[0].to_string());
        let Some(vista) = vista else {
            return Json::obj([("estado", Json::s("desconocido"))]);
        };
        // Su puntero: el de su sitio o el de antes (0038 P2).
        let dir = raiz.join(ore_core::punteros::CARPETA);
        let hallado = ore_core::punteros::leer_en(&dir, &vista);
        let fichero_informe = hallado
            .as_ref()
            .and_then(|(r, _)| r.strip_prefix(raiz).ok())
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .or_else(|| ore_core::punteros::ruta(&vista))
            .unwrap_or_default();
        let informe = hallado.map(|(_, n)| n);
        let estado_informe = informe
            .as_ref()
            .and_then(|i| {
                i.get("estado")
                    .and_then(|(_, x)| x.as_str().map(String::from))
            })
            .unwrap_or_default();
        let motivo_informe = informe
            .as_ref()
            .and_then(|i| {
                i.get("motivo")
                    .and_then(|(_, x)| x.as_str().map(String::from))
            })
            .unwrap_or_default();
        let fecha_informe = match &self.arbol {
            crate::rutas::Arbol::Forja(f) => f.fecha_de(raiz, &fichero_informe),
            crate::rutas::Arbol::Directorio(_) => None,
        };

        // La cola: el `48-…` más nuevo que la nombre.
        let mut en_cola: Option<(String, (i64, String))> = None;
        if let Some(cola) = &self.cola
            && let Ok(prestado) = cola.clonar()
            && let Ok(entradas) = std::fs::read_dir(prestado.ruta())
        {
            for e in entradas.flatten() {
                let nombre = e.file_name().to_string_lossy().into_owned();
                if !nombre.starts_with("48-la-copia") || !nombre.ends_with(".yaml") {
                    continue;
                }
                let Ok(texto) = std::fs::read_to_string(e.path()) else {
                    continue;
                };
                let la_nombra = texto.lines().any(|l| {
                    l.contains("name: VISTAS")
                        && l.split('"')
                            .nth(1)
                            .is_some_and(|v| v.split(',').any(|x| x.trim() == vista))
                });
                if !la_nombra {
                    continue;
                }
                let fecha = cola
                    .fecha_de(prestado.ruta(), &nombre)
                    .unwrap_or((0, String::new()));
                if en_cola.as_ref().is_none_or(|(_, (s, _))| fecha.0 > *s) {
                    en_cola = Some((nombre, fecha));
                }
            }
        }

        // Sin fechas (un árbol que es un directorio, sin historia) el informe
        // es la última palabra que se ve: `error` es `fallida`.
        let mas_nueva_que_el_informe =
            |(s, _): &(i64, String)| fecha_informe.as_ref().is_some_and(|(si, _)| *s > *si);
        match en_cola {
            Some((fichero, fecha))
                if informe.is_none()
                    || estado_informe != "error"
                    || mas_nueva_que_el_informe(&fecha) =>
            {
                Json::obj([
                    ("vista", Json::s(&vista)),
                    ("estado", Json::s("encolada")),
                    ("fichero", Json::s(&fichero)),
                    ("desde", Json::s(&fecha.1)),
                    (
                        "dice",
                        Json::s("se está copiando: el Job está en la cola o corriendo"),
                    ),
                ])
            }
            _ if estado_informe == "error" => Json::obj([
                ("vista", Json::s(&vista)),
                ("estado", Json::s("fallida")),
                ("motivo", Json::s(&motivo_informe)),
                (
                    "desde",
                    Json::s(fecha_informe.map(|(_, f)| f).unwrap_or_default()),
                ),
                ("dice", Json::s("la última pasada no pudo copiarla")),
            ]),
            _ => Json::obj([
                ("vista", Json::s(&vista)),
                ("estado", Json::s("pendiente")),
                ("dice", Json::s("nadie ha encolado su copia")),
            ]),
        }
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
