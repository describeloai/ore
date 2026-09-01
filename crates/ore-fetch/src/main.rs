//! `ore-fetch` — el obtenedor de referencia, **fuera** del compilador.
//!
//! Normativo: [`spec/v1alpha6/01-distribucion.md`] §4. El contrato es corto y
//! todo él está pensado para que **el origen no tenga que ser de confianza**:
//!
//! - la petición entra por **stdin**, nunca por `argv` — lo lee cualquier
//!   proceso de la máquina, y una coordenada privada dice de qué depende una
//!   organización;
//! - el `.oob` sale por **stdout** y nada más;
//! - lo que haya que contar sale por **stderr**, y `ore` lo muestra literal.
//!
//! # Por qué un directorio
//!
//! Por lo mismo que el segundo lector de fuentes es un fichero y no otra base de
//! datos: **si el mismo protocolo sirve a un directorio y a un registro, la
//! costura está cortada por el sitio correcto.** Y se puede escribir hoy, que es
//! lo que permite probar la delegación entera sin que exista un registro.
//!
//! # Lo que este programa NO garantiza, y está bien
//!
//! Devuelve la versión más alta que encuentra y **no interpreta el rango**. No
//! es dejadez: quien pide **no debe creerse** lo que le devuelvan, así que `ore`
//! comprueba de todos modos que el `.oob` diga el paquete que se pidió, que su
//! versión satisfaga el rango y que su digest sea el que el lock fija. Un
//! obtenedor que se esmerara en eso solo conseguiría que la comprobación de
//! verdad pareciera redundante.

use std::io::Read as _;
use std::process::ExitCode;

const DIR: &str = "ORE_FETCH_DIR";

fn main() -> ExitCode {
    match intentar() {
        Ok(bytes) => {
            print!("{bytes}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            // `ore` muestra esto literal, sin resumirlo: es lo único accionable
            // que va a ver quien lo ejecute.
            eprintln!("ore-fetch: {m}");
            ExitCode::FAILURE
        }
    }
}

fn intentar() -> Result<String, String> {
    let mut entrada = String::new();
    std::io::stdin()
        .read_to_string(&mut entrada)
        .map_err(|e| format!("no se pudo leer stdin: {e}"))?;
    if entrada.trim().is_empty() {
        return Err(
            "no llegó nada por stdin. La petición va por ahí y no por la línea \
                    de órdenes, porque `argv` lo lee cualquier proceso de la máquina"
                .into(),
        );
    }
    let peticion =
        ore_core::parse::parse(&entrada).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    let coordenada = peticion
        .get("package")
        .and_then(|(_, v)| v.as_str())
        .ok_or("la petición no dice qué `package` se quiere")?;

    let dir = std::env::var(DIR).map_err(|_| {
        format!(
            "`{DIR}` no está definida. Este obtenedor trae paquetes de un directorio: \
             es el caso que se puede escribir sin que exista un registro, y el que \
             demuestra que el contrato no depende de uno"
        )
    })?;
    let corto = coordenada.rsplit('/').next().unwrap_or(coordenada);

    // La más alta que haya. NO se interpreta el rango: quien pide comprueba de
    // todos modos que lo devuelto sea lo que pidió, y hacerlo aquí solo haría
    // que esa comprobación pareciera de más.
    let mut candidatos: Vec<(Vec<u64>, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .map_err(|e| format!("no se pudo leer `{dir}`: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "oob"))
        .filter_map(|p| {
            let n = p.file_stem()?.to_string_lossy().to_string();
            let (base, v) = n.rsplit_once('-')?;
            (base == corto).then(|| {
                (
                    v.split('.').map(|x| x.parse().unwrap_or(0)).collect(),
                    p.clone(),
                )
            })
        })
        .collect();
    candidatos.sort();

    let Some((_, ruta)) = candidatos.pop() else {
        return Err(format!(
            "no hay ningún `{corto}-<version>.oob` en `{dir}`.\n  \
             Un obtenedor que devolviera otra cosa no engañaría a nadie —`ore` \
             comprueba el digest— pero tampoco serviría de nada"
        ));
    };
    std::fs::read_to_string(&ruta).map_err(|e| format!("no se pudo leer `{}`: {e}", ruta.display()))
}
