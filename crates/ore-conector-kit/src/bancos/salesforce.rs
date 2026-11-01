//! **El banco de Salesforce** (ADR 0083 paso 5): la org de mentira
//! (`pruebas-de-fuego/salesforce-de-mentira.py`) —no hay emulador de Salesforce—, con las
//! mismas tres tablas de la semilla como objetos propios (`Tipos__c`, `Vacia__c`, `Grande__c`) y
//! uno lento (`Lenta__c`).
//!
//! `SALESFORCE_KIT_ENDPOINT` dice dónde (`http://127.0.0.1:8794`). El conector sólo habla con
//! él si `ORE_SALESFORCE_ENDPOINT` apunta ahí (lo que el script del CI exporta) y lleva su clave
//! en `ORE_SALESFORCE_CLAVE_FICHERO`. La org de mentira aprueba a `kit@lab.ore` y `otro@lab.ore`
//! (`url_alternativa`); la credencial mala es un usuario que no aprobó la app (`invalid_grant`).
//!
//! # Lo que la semilla pierde en Salesforce, y es lo que el kit mide
//!
//! - **La cadena vacía es nulo**: Salesforce guarda `''` como `NULL` ([`Banco::vacio_es_nulo`]).
//! - **Segundos**: un `datetime` guarda segundos —escribes `…:00.500` y devuelve `…:00.000`, medido en una org
//!   real— ([`Banco::segundos`]).
//! - **No hay hora local**: todo `datetime` es un instante UTC. `local` no se compara
//!   ([`Banco::sin_columnas`]): el kit lo dice en el caso 5, y se carga de todos modos.
//! - **Los nombres**: un objeto propio acaba en `__c`, y `Id` ya es el de registro. La columna `id`
//!   de la semilla es `Id__c`, `entero` es `Entero__c`… ([`Salesforce::adaptar`]).
//! - **Los tipos son los que el kit necesita**, no los de un `Number(18,0)` (que un `describe`
//!   real da como `double` con precisión 18, y el conector entrega como `Decimal128(18, 0)`):
//!   `id` es `int`, `entero` es `long`, `real` es un `double` sin precisión. Una org real tiene
//!   campos así en sus objetos estándar. Los tipos que un Salesforce de verdad da a un campo
//!   personalizado los prueba `o7-salesforce.sh` con otros objetos.
//!
//! `sesiones` no aplica: la API REST no tiene sesión que contar.

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use ore_base::json::Json;
use std::collections::BTreeMap;

const CLIENTE: &str = "3MVGkitconsumerkey";
const USUARIO: &str = "kit%40lab.ore";
const OTRO: &str = "otro%40lab.ore";
const NADIE: &str = "nadie%40lab.ore";

pub struct Salesforce {
    endpoint: String,
}

impl Salesforce {
    pub fn new(endpoint: &str) -> Salesforce {
        Salesforce {
            endpoint: endpoint.trim_end_matches('/').to_string(),
        }
    }

    fn url_de(&self, usuario: &str) -> String {
        format!("salesforce://kit.my.salesforce.com/?cliente={CLIENTE}&usuario={usuario}")
    }

    fn post(&self, ruta: &str, cuerpo: &serde_json::Value) -> Result<(), String> {
        let url = format!("{}/_mentira/{ruta}", self.endpoint);
        ureq::post(&url)
            .set("content-type", "application/json")
            .send_string(&cuerpo.to_string())
            .map(|_| ())
            .map_err(|e| match e {
                ureq::Error::Status(c, r) => format!(
                    "la org de mentira rechaza `{ruta}` ({c}): {}",
                    r.into_string().unwrap_or_default()
                ),
                e => format!("no se llega a la org de mentira en {}: {e}", self.endpoint),
            })
    }

    fn get(&self, ruta: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}/_mentira/{ruta}", self.endpoint);
        let t = ureq::get(&url)
            .call()
            .map_err(|e| format!("la org de mentira no contesta `{ruta}`: {e}"))?
            .into_string()
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&t).map_err(|e| format!("`{ruta}` no es JSON: {e}"))
    }
}

/// El nombre en la org de una columna de la semilla.
fn campo(columna: &str) -> String {
    match columna {
        "id" => "Id__c",
        "entero" => "Entero__c",
        "importe" => "Importe__c",
        "real" => "Real__c",
        "texto" => "Texto__c",
        "fecha" => "Fecha__c",
        "local" => "Local__c",
        "instante" => "Instante__c",
        "logico" => "Logico__c",
        "grupo" => "Grupo__c",
        "nota" => "Nota__c",
        otro => otro,
    }
    .to_string()
}

/// El campo de Salesforce con que se guarda un tipo de la semilla.
fn campo_json(nombre: &str, tipo: &str) -> Result<serde_json::Value, String> {
    let n = campo(nombre);
    Ok(match (nombre, tipo) {
        ("id", _) => serde_json::json!({"nombre": n, "tipo": "int", "nulable": false}),
        ("entero", _) => serde_json::json!({"nombre": n, "tipo": "long"}),
        (_, "Integer") => serde_json::json!({"nombre": n, "tipo": "int"}),
        (_, "Float") => serde_json::json!({"nombre": n, "tipo": "double"}),
        (_, "Boolean") => serde_json::json!({"nombre": n, "tipo": "boolean"}),
        (_, "String") => serde_json::json!({"nombre": n, "tipo": "string", "largo": 255}),
        (_, "Decimal<12, 2>") => {
            serde_json::json!({"nombre": n, "tipo": "currency", "precision": 12, "escala": 2})
        }
        (_, "Date") => serde_json::json!({"nombre": n, "tipo": "date"}),
        (_, "DateTime") | (_, "DateTimeTz") => serde_json::json!({"nombre": n, "tipo": "datetime"}),
        (_, otro) => return Err(format!("la semilla no usa `{otro}`")),
    })
}

fn campos_de_tipos() -> Result<Vec<serde_json::Value>, String> {
    TIPOS.iter().map(|c| campo_json(c.nombre, c.tipo)).collect()
}

fn fila_json(f: &[Option<&str>; 9]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (c, v) in TIPOS.iter().zip(f.iter()) {
        m.insert(
            campo(c.nombre),
            v.map_or(serde_json::Value::Null, |t| serde_json::json!(t)),
        );
    }
    serde_json::Value::Object(m)
}

impl Banco for Salesforce {
    fn familia(&self) -> &'static str {
        "salesforce"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let filas: Vec<_> = FILAS.iter().map(fila_json).collect();
        self.post(
            "objeto",
            &serde_json::json!({"nombre": "Tipos__c", "campos": campos_de_tipos()?, "filas": filas}),
        )?;
        self.post(
            "objeto",
            &serde_json::json!({"nombre": "Vacia__c", "campos": campos_de_tipos()?}),
        )?;
        // `Lenta__c`: tarda en dar su primera fila (los casos 8 y 10).
        self.post(
            "objeto",
            &serde_json::json!({"nombre": "Lenta__c", "campos": campos_de_tipos()?,
                "filas": FILAS.iter().take(3).map(fila_json).collect::<Vec<_>>(), "lenta": true}),
        )?;
        // `Escribe__c`: un objeto que la API no deja consultar (los hay: `queryable: false`). Leerlo no puede
        // servirse ni, por supuesto, escribir (caso 9).
        self.post(
            "objeto",
            &serde_json::json!({"nombre": "Escribe__c", "campos": campos_de_tipos()?, "consultable": false}),
        )?;
        // `Grande__c` no se sube: la org la calcula (`id` de 1 a 10⁶, `grupo = id % 100`,
        // `importe = id / 100`, `nota = 'fila-<id>'`), como ClickHouse con `numbers()`.
        let campos = semilla::GRANDE_COLUMNAS
            .iter()
            .map(|c| campo_json(c.nombre, c.tipo))
            .collect::<Result<Vec<_>, _>>()?;
        self.post(
            "objeto",
            &serde_json::json!({"nombre": "Grande__c", "campos": campos,
                "generar": {"n": GRANDE, "id": campo("id"), "grupo": campo("grupo"),
                            "importe": campo("importe"), "nota": campo("nota")}}),
        )
    }

    fn url(&self) -> String {
        self.url_de(USUARIO)
    }

    /// Otro usuario que la app aprobó: el caso 11 (servir) lee con dos credenciales.
    fn url_alternativa(&self) -> Option<String> {
        Some(self.url_de(OTRO))
    }

    /// Un usuario que no aprobó la app: Salesforce contesta `invalid_grant`.
    fn url_mala(&self) -> Option<String> {
        Some(self.url_de(NADIE))
    }

    fn objeto(&self, tabla: Tabla) -> String {
        match tabla {
            Tabla::Tipos => "Tipos__c",
            Tabla::Vacia => "Vacia__c",
            Tabla::Grande => "Grande__c",
        }
        .to_string()
    }

    fn lenta(&self) -> Option<String> {
        Some("Lenta__c".into())
    }

    /// SOQL no escribe: no hay objeto cuya lectura lo haga. El caso 9 comprueba entonces que un objeto que la API
    /// no deja consultar (`Escribe__c`) se rechaza, que los nombres con `;` también, y que el conector no manda
    /// jamás una escritura (`escrituras`).
    fn que_escribe(&self) -> Option<String> {
        Some("Escribe__c".into())
    }

    fn escrituras(&mut self) -> Option<Result<u64, String>> {
        Some(
            self.get("escrituras")
                .map(|v| v["escrituras"].as_u64().unwrap_or(0)),
        )
    }

    fn consultas_vivas(&mut self, objeto: &str) -> Option<u64> {
        let o = objeto.split(['/', ';']).next().unwrap_or(objeto);
        self.get(&format!("vivas?objeto={o}"))
            .ok()
            .map(|v| v["vivas"].as_u64().unwrap_or(0))
    }

    /// Un `datetime` guarda segundos: le escribes `…:00.500` y devuelve `…:00.000` (medido en una org real).
    fn segundos(&self) -> bool {
        true
    }

    /// No hay hora local: todo `datetime` es un instante.
    fn sin_columnas(&self) -> &'static [&'static str] {
        &["local"]
    }

    fn vacio_es_nulo(&self) -> bool {
        true
    }

    /// Los nombres de la semilla (`entero`) son los de la org (`Entero__c`). Lo que no es de la semilla
    /// (un nombre con `;` del caso 9) pasa tal cual: es lo que el conector tiene que rechazar.
    fn adaptar(&self, peticion: &mut BTreeMap<String, Json>) {
        if let Some(Json::Obj(p)) = peticion.get_mut("proyeccion") {
            for v in p.values_mut() {
                if let Json::Str(s) = v {
                    *s = campo(s);
                }
            }
        }
        for clave in ["filtros", "orderBy"] {
            if let Some(Json::Arr(a)) = peticion.get_mut(clave) {
                for f in a {
                    if let Json::Obj(o) = f
                        && let Some(Json::Str(c)) = o.get_mut("columna")
                    {
                        *c = campo(c);
                    }
                }
            }
        }
    }
}
