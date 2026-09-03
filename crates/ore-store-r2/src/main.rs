//! `ore-store-r2` — **el almacén delegado**, fuera del compilador.
//!
//! Normativo: [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md).
//! Es la **tercera** vez que este árbol delega, y por la misma razón que las dos
//! anteriores: `ore` no puede abrir un socket, y no por promesa —
//! `ore-cli/tests/dependencias.rs` lee el `Cargo.lock` y falla si aparece una
//! crate de red, de TLS o de FFI en su cierre.
//!
//! | | qué delega | ADR |
//! |---|---|---|
//! | `ore-read-<tipo>` | leer filas de un origen | 0008 |
//! | `ore-maintain` | correr el circuito Δ | 0013 |
//! | **`ore-store-<tipo>`** | **sellar y subir el artefacto** | **0015** |
//!
//! # El protocolo
//!
//! Hereda la línea de 0008 —*«la petición es un fragmento del plan, no SQL»*—
//! llevada a su sitio:
//!
//! > **Lo que viaja no son llamadas al almacén: es el artefacto.**
//!
//! - **stdin**: la cabecera del sobre en JSON canónico, **una línea**, y después
//!   las filas, **una por línea**, como objetos JSON de cadenas. Por stdin y no
//!   por `argv` por lo mismo de siempre: `argv` lo lee cualquier proceso, y una
//!   fila es un dato;
//! - **stdout**: una línea JSON con el nombre, el digest, el tamaño y **si hizo
//!   falta subir**;
//! - **stderr**: lo que haya que contar.
//!
//! Este programa **no sabe qué es una entidad, ni un conducto, ni una vista.**
//! Recibe una cabecera y un flujo de filas.
//!
//! # Y lo que decide si sube
//!
//! El nombre **es** el contenido, así que:
//!
//! - re-materializar con el mismo testigo da el mismo nombre y **no sube ni un
//!   byte** — lo dice `subido: false`;
//! - dos escritores que lleguen a la vez escriben los mismos bytes, así que la
//!   carrera es inofensiva.

mod carga;
mod r2;
mod sobre;

use std::collections::BTreeMap;
use std::io::Read;

fn main() -> std::process::ExitCode {
    match correr() {
        Ok(linea) => {
            println!("{linea}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn correr() -> Result<String, String> {
    let mut texto = String::new();
    std::io::stdin()
        .read_to_string(&mut texto)
        .map_err(|e| format!("no se pudo leer la entrada: {e}"))?;

    let mut lineas = texto.lines().filter(|l| !l.trim().is_empty());
    let cabecera = lineas
        .next()
        .ok_or("la entrada está vacía: se esperaba la cabecera en la primera línea")?;
    let cab = leer_cabecera(cabecera)?;

    let filas: Vec<carga::Fila> = lineas
        .map(objeto_plano)
        .collect::<Result<Vec<_>, String>>()?;

    let parquet = carga::escribir(&cab.esquema, &filas)?;
    let artefacto = sobre::sellar(&cab, &parquet);
    let clave = sobre::clave(&artefacto);
    let digest = ore_core::digest::de_bytes(&artefacto);

    // El paso 4: se sabe si hay que copiar **sin copiar nada**.
    let cuenta = r2::Cuenta::del_entorno()?;
    let subido = if r2::existe(&cuenta, &clave)? {
        false
    } else {
        r2::subir(&cuenta, &clave, &artefacto)?
    };

    Ok(ore_core::json::Json::obj([
        ("bytes", ore_core::json::Json::Int(artefacto.len() as i64)),
        ("clave", ore_core::json::Json::s(&clave)),
        ("digest", ore_core::json::Json::s(&digest)),
        ("filas", ore_core::json::Json::Int(filas.len() as i64)),
        ("subido", ore_core::json::Json::Bool(subido)),
    ])
    .jcs())
}

/// La cabecera, leída con el analizador del núcleo — el mismo que lee YAML, que
/// es un superconjunto de JSON. No entra un analizador más para esto.
fn leer_cabecera(linea: &str) -> Result<sobre::Cabecera, String> {
    let n = ore_core::parse::parse(linea).map_err(|e| format!("la cabecera no analiza: {e:?}"))?;
    let s = |k: &str| -> Result<String, String> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
            .ok_or_else(|| format!("a la cabecera le falta `{k}`"))
    };
    let esquema = n
        .get("esquema")
        .map(|(_, e)| {
            e.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    if esquema.is_empty() {
        return Err("la cabecera no declara `esquema`: sin él no hay Parquet que escribir".into());
    }
    let testigo = n.get("testigo").map(|(_, t)| sobre::Testigo {
        modo: t
            .get("modo")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("none")
            .to_string(),
        valor: t
            .get("valor")
            .and_then(|(_, v)| v.as_str())
            .map(String::from),
    });
    Ok(sobre::Cabecera {
        plan: s("plan")?,
        esquema,
        testigo: testigo.unwrap_or_default(),
        conducto: s("conducto")?,
        bundle: s("bundle")?,
    })
}

/// Una fila: un objeto de cadenas y nada más. Un valor que no sea escalar es un
/// defecto de quien la produjo — y se dice, en vez de aplanarlo.
fn objeto_plano(linea: &str) -> Result<carga::Fila, String> {
    let n = ore_core::parse::parse(linea).map_err(|e| format!("una fila no analiza: {e:?}"))?;
    let mut out = BTreeMap::new();
    for (k, v) in n.entries() {
        let Some(nombre) = k.as_str() else { continue };
        match v.as_str() {
            Some(x) => {
                out.insert(nombre.to_string(), x.to_string());
            }
            None => {
                return Err(format!(
                    "`{nombre}` no es un escalar: una fila es un objeto plano, y aplanarlo aquí \
                     inventaría una codificación que nadie declaró"
                ));
            }
        }
    }
    Ok(out)
}
