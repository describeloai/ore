//! **El banco de HBase**: el espacio `kit` en un HBase de pruebas (el laboratorio de `pruebas-de-fuego/hbase/`, o
//! cualquiera con la pasarela REST), cargado **por la pasarela REST**, que escribe los bytes que se le dan.
//!
//! `HBASE_KIT_URL` es la URL del conector (`hbase://zk:2181?simple=acepto…`) y `HBASE_KIT_REST` la pasarela REST de
//! escritura (`http://127.0.0.1:8080`). El laboratorio no tiene autenticacion, ni ACL, ni consultas que se puedan
//! contar desde fuera: `url_alternativa`, `url_mala`, `sesiones` y `consultas_vivas` salen «no aplica». Tampoco hay
//! como escribir desde una lectura (el conector no tiene camino de escritura) ni un objeto lento (HBase no tiene
//! vistas): los casos 8, 9 y 10 se prueban contra el cluster simulado de `ore-hbase`.
//!
//! **Los bytes los escribe este banco con las codificaciones de `ore-hbase`**, que las pruebas del laboratorio
//! comprueban contra los que escribe Java (`Bytes.toBytes`). Cada tipo de la semilla va con la suya:
//!
//! | tipo | codificacion |
//! |---|---|
//! | `Integer` | `i64be` |
//! | `Decimal<12, 2>` | `decimal` (`BigDecimal` de Java) |
//! | `Float` | `f64be` |
//! | `String` | `utf8` |
//! | `Date`, `DateTime`, `DateTimeTz` | `epoch-ms` |
//! | `Boolean` | `bool` |
//!
//! **Lo que la semilla pierde en HBase, y es lo que el kit mide:**
//!
//! - **un instante son milisegundos** ([`Banco::milisegundos`]): se cargan truncados a ms;
//! - **una fila sin celdas no existe**: la fila 7 de la semilla es toda nula salvo el `id`, y en HBase la clave
//!   sola no es una fila. El banco le pone una celda marcadora (`f:id`, el mismo entero), que el conector no lee.
//!
//! Los nombres de las columnas son los de la semilla (`id`, `texto`…): la URL lleva `familia=f&clave=id`, que el
//! conector lleva a `f:texto` y `_clave`. `grande` (10⁶ filas) se carga en una sola region; una marca dice que esta
//! entera y las siguientes ejecuciones no la rehacen.
//!
//! Detras de la feature `hbase`: arrastra `ore-hbase` (el conector), que no es cosa de quien solo prueba Postgres.

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, GRANDE_COLUMNAS, TIPOS, Tabla};
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use ore_hbase::codificacion::{Codificacion, codificar};
use ore_hbase::lote::base64;
use std::collections::BTreeMap;

const ESPACIO: &str = "kit";
const FAMILIA: &str = "f";
/// Filas por peticion de escritura a la pasarela REST.
const LOTE: usize = 5_000;

/// Una fila para escribir: su clave y sus celdas `(cualificador, valor)`.
type FilaDeBytes = (Vec<u8>, Vec<(String, Vec<u8>)>);

pub struct Hbase {
    url: String,
    rest: String,
}

impl Hbase {
    /// `url`: la del conector. `rest`: la pasarela REST con la que se siembra.
    pub fn new(url: &str, rest: &str) -> Result<Hbase, String> {
        let separador = if url.contains('?') { '&' } else { '?' };
        // Los nombres de la semilla son los de las columnas: `id` es la clave y lo demas esta en la familia `f`.
        let url = format!("{url}{separador}familia={FAMILIA}&clave=id");
        let h = Hbase {
            url,
            rest: rest.trim_end_matches('/').to_string(),
        };
        h.pedir("GET", "/version/cluster", None)
            .map_err(|e| format!("no se llega a la pasarela REST de HBase ({rest}): {e}"))?;
        Ok(h)
    }

    /// Una peticion a la pasarela. `Ok` con el cuerpo, o `Err` con el estado HTTP y lo que dijo.
    fn pedir(&self, metodo: &str, ruta: &str, cuerpo: Option<&str>) -> Result<String, String> {
        let r = ureq::request(metodo, &format!("{}{ruta}", self.rest))
            .set("Accept", "application/json")
            .set("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(300));
        let r = match cuerpo {
            Some(c) => r.send_string(c),
            None => r.call(),
        };
        match r {
            Ok(r) => Ok(r.into_string().unwrap_or_default()),
            Err(ureq::Error::Status(c, r)) => Err(format!(
                "{metodo} {ruta}: {c} {}",
                r.into_string().unwrap_or_default()
            )),
            Err(e) => Err(format!("{metodo} {ruta}: {e}")),
        }
    }

    /// Borra la tabla si existe y la crea con la familia `f`.
    fn crear(&self, tabla: &str) -> Result<(), String> {
        // 200 si estaba; 404 si no: las dos valen
        match self.pedir("DELETE", &format!("/{ESPACIO}:{tabla}/schema"), None) {
            Ok(_) => {}
            Err(e) if e.contains(" 404 ") || e.contains(" 503 ") => {}
            Err(e) => return Err(format!("no se pudo borrar `{ESPACIO}:{tabla}`: {e}")),
        }
        let esquema = format!(
            "{{\"name\":\"{ESPACIO}:{tabla}\",\"ColumnSchema\":[{{\"name\":\"{FAMILIA}\"}}]}}"
        );
        self.pedir("PUT", &format!("/{ESPACIO}:{tabla}/schema"), Some(&esquema))
            .map(|_| ())
            .map_err(|e| format!("no se pudo crear `{ESPACIO}:{tabla}`: {e}"))
    }

    /// Escribe un lote de filas: `(clave, [(cualificador, valor)])`.
    fn escribir(&self, tabla: &str, filas: &[FilaDeBytes]) -> Result<(), String> {
        let mut j = String::with_capacity(filas.len() * 160);
        j.push_str("{\"Row\":[");
        for (i, (clave, celdas)) in filas.iter().enumerate() {
            if i > 0 {
                j.push(',');
            }
            j.push_str(&format!("{{\"key\":\"{}\",\"Cell\":[", base64(clave)));
            for (k, (q, v)) in celdas.iter().enumerate() {
                if k > 0 {
                    j.push(',');
                }
                j.push_str(&format!(
                    "{{\"column\":\"{}\",\"$\":\"{}\"}}",
                    base64(format!("{FAMILIA}:{q}").as_bytes()),
                    base64(v)
                ));
            }
            j.push_str("]}");
        }
        j.push_str("]}");
        self.pedir("PUT", &format!("/{ESPACIO}:{tabla}/fakerow"), Some(&j))
            .map(|_| ())
            .map_err(|e| format!("la semilla no se carga en `{ESPACIO}:{tabla}`: {e}"))
    }

    /// El valor de la clave `marca` de `kit:marcas`, si esta.
    fn marca(&self, clave: &str) -> Option<u64> {
        let r = self
            .pedir(
                "GET",
                &format!("/{ESPACIO}:marcas/{clave}/{FAMILIA}:v"),
                None,
            )
            .ok()?;
        let n = ore_core::parse::parse(&r).ok()?;
        let celda = n
            .get("Row")?
            .1
            .items()
            .first()?
            .get("Cell")?
            .1
            .items()
            .first()?
            .get("$")?
            .1
            .as_str()?
            .to_string();
        let bytes = de_base64(&celda)?;
        Some(u64::from_be_bytes(bytes.try_into().ok()?))
    }
}

/// Base64 de vuelta.
fn de_base64(s: &str) -> Option<Vec<u8>> {
    ore_hbase::lote::de_base64(s)
}

/// La codificacion con que se guarda cada tipo de la semilla.
fn codificacion_de(tipo: &str) -> Result<&'static str, String> {
    Ok(match tipo {
        "Integer" => "i64be",
        "Decimal<12, 2>" => "decimal",
        "Float" => "f64be",
        "String" => "utf8",
        "Date" | "DateTime" | "DateTimeTz" => "epoch-ms",
        "Boolean" => "bool",
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

/// Los bytes de un valor de la semilla (texto canonico de OOS) en la codificacion de su tipo.
fn bytes_de(tipo: &str, texto: &str) -> Result<Vec<u8>, String> {
    let t =
        ore_core::types::parse_type(tipo).map_err(|_| format!("`{tipo}` no es un tipo de OOS"))?;
    let fisico = Fisico::de(&t);
    let valor = fisico
        .analizar(texto)
        .ok_or_else(|| format!("`{texto}` no es `{tipo}`"))?;
    // HBase guarda milisegundos: lo que la semilla trae en microsegundos no llega a existir.
    let valor = match valor {
        Valor::FechaHora(us) => Valor::FechaHora(us.div_euclid(1000) * 1000),
        Valor::Instante(us) => Valor::Instante(us.div_euclid(1000) * 1000),
        otro => otro,
    };
    let cod = Codificacion::analizar(codificacion_de(tipo)?).expect("una codificacion que existe");
    codificar(cod, &fisico, &valor)
}

/// Una fila de la semilla como celdas: las que no son nulas, mas la marcadora que hace que la fila exista.
fn celdas_de(
    columnas: &[semilla::Columna],
    valores: &[Option<String>],
) -> Result<FilaDeBytes, String> {
    let mut clave = Vec::new();
    let mut celdas = Vec::new();
    for (c, v) in columnas.iter().zip(valores) {
        let Some(v) = v else { continue };
        let b = bytes_de(c.tipo, v)?;
        if c.nombre == "id" {
            clave = b.clone();
            // en HBase la clave sola no es una fila
            celdas.push(("id".to_string(), b));
        } else {
            celdas.push((c.nombre.to_string(), b));
        }
    }
    Ok((clave, celdas))
}

impl Banco for Hbase {
    fn familia(&self) -> &'static str {
        "hbase"
    }

    fn cargar(&mut self) -> Result<(), String> {
        // El espacio de nombres (201 si lo crea; 403 si ya estaba).
        match self.pedir("POST", &format!("/namespaces/{ESPACIO}"), None) {
            Ok(_) => {}
            Err(e) if e.contains(" 403 ") || e.contains(" 409 ") => {}
            Err(e) => return Err(format!("no se pudo crear el espacio `{ESPACIO}`: {e}")),
        }
        for nombre in ["tipos", "vacia"] {
            self.crear(nombre)?;
        }
        let filas: Vec<FilaDeBytes> = FILAS
            .iter()
            .map(|f| {
                let v: Vec<Option<String>> = f.iter().map(|x| x.map(String::from)).collect();
                celdas_de(TIPOS, &v)
            })
            .collect::<Result<_, _>>()?;
        self.escribir("tipos", &filas)?;

        // `grande` son 10⁶ filas: solo se rehace si no esta entera, y lo dice una marca.
        let hecha = self.marca("grande") == Some(GRANDE);
        if !hecha {
            self.crear("grande")?;
            self.crear("marcas")?;
            let mut lote = Vec::with_capacity(LOTE);
            for id in 1..=GRANDE {
                let f = semilla::fila_grande(id);
                let v: Vec<Option<String>> = f.iter().map(|x| Some(x.clone())).collect();
                lote.push(celdas_de(GRANDE_COLUMNAS, &v)?);
                if lote.len() == LOTE {
                    self.escribir("grande", &lote)?;
                    lote.clear();
                }
            }
            if !lote.is_empty() {
                self.escribir("grande", &lote)?;
            }
            let marca = vec![(
                b"grande".to_vec(),
                vec![("v".to_string(), GRANDE.to_be_bytes().to_vec())],
            )];
            self.escribir("marcas", &marca)?;
        }
        Ok(())
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    /// El laboratorio no tiene autenticacion: no hay otra credencial que probar.
    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Ni una que se rechace: con SIMPLE, cualquier usuario entra.
    fn url_mala(&self) -> Option<String> {
        None
    }

    /// Lo que el motor manda y el dato no lleva: el tipo de cada columna y la codificacion de sus bytes. Sin ellos,
    /// HBase es texto.
    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        let columnas = tabla.columnas();
        peticion.insert(
            "tipos".into(),
            Json::Arr(
                columnas
                    .iter()
                    .map(|c| Json::Arr(vec![Json::s(c.nombre), Json::s(c.tipo)]))
                    .collect(),
            ),
        );
        peticion.insert(
            "codificaciones".into(),
            Json::Arr(
                columnas
                    .iter()
                    .map(|c| {
                        Json::Arr(vec![
                            Json::s(c.nombre),
                            Json::s(codificacion_de(c.tipo).unwrap_or("utf8")),
                        ])
                    })
                    .collect(),
            ),
        );
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{ESPACIO}.{}", tabla.nombre())
    }

    fn milisegundos(&self) -> bool {
        true
    }
}
