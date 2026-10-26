//! **Los verbos que son de Oracle** y no del esqueleto: `catalogo`, `check`, `explorar`, `testigo` y `cambio`.
//! Los cuatro de lectura (`capacidades`, `leer`, `estimar`, `servir`) están en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por stdout, o el motivo del fallo (que va
//! a stderr, literal, sin la credencial).

use crate::lectura::partir_objeto;
use crate::tipos::Tipo;
use crate::transporte::{Conexion, Objeto, Tabla as TablaOracle};
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Tabla, escribir};
use ore_driver::{Codigo, Fallo};
use std::collections::BTreeMap;
use std::process::ExitCode;

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-oracle: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(
            entrada.trim(),
            args.first().map_or("oracle", String::as_str),
        ),
        "check" => check(&entrada),
        "explorar" => explorar(&entrada),
        "testigo" => testigo(&entrada),
        "cambio" => cambio(&entrada),
        otro => Err(format!(
            "verbo desconocido `{otro}` (capacidades, leer, servir, catalogo, check, explorar, testigo, cambio)"
        )),
    };
    match hecho {
        Ok(salida) => {
            println!("{salida}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-read-oracle: {m}");
            ExitCode::FAILURE
        }
    }
}

/// Un fallo del conector, como el texto que `ore` enseña: sin la credencial.
fn texto(f: Fallo, url: &str) -> String {
    f.tapado(url).mensaje
}

/// **`check`**: ¿responde esta fuente? Paso a paso, y el motivo del fallo va **literal** (es lo único
/// accionable): abre (red, TLS, credencial, servicio), pregunta la versión y lista lo que la URL deja ver. Si
/// hay algo, **lee una fila de una tabla** (el permiso de lectura).
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = (|| -> Result<(), Fallo> {
        let c = Conexion::abrir(&url)?;
        c.version()?;
        let objetos = c.objetos()?;
        if let Some(e) = &c.destino.esquema
            && objetos.is_empty()
        {
            return Err(Fallo::new(
                Codigo::Objeto,
                format!(
                    "el esquema `{e}` no existe o no tiene tablas que se vean (los nombres van en mayúsculas)"
                ),
            ));
        }
        if let Some(o) = objetos.iter().find(|o| !o.vista) {
            c.c.query_row(
                &format!(
                    "SELECT 1 FROM \"{}\".\"{}\" FETCH FIRST 1 ROWS ONLY",
                    o.esquema.replace('"', "\"\""),
                    o.nombre.replace('"', "\"\"")
                ),
                &[],
            )
            .map(|_| ())
            // Una tabla vacía no es un fallo: `query_row` la llama `NoDataFound`.
            .or_else(|e| match e.kind() {
                oracledb::ErrorKind::NoDataFound => Ok(()),
                _ => Err(crate::transporte::fallo_de(e)),
            })?;
        }
        Ok(())
    })();
    Ok(match hecho {
        Ok(()) => ore_driver::comprobacion(true, None),
        Err(f) => ore_driver::comprobacion(false, Some(&texto(f, &url))),
    })
}

/// **`catalogo`**: cada tabla y vista que la URL deja ver, con los tipos de `ALL_TAB_COLUMNS` (no se infiere
/// nada), su clave primaria, sus claves únicas y los comentarios de sus columnas. Una tabla que no se deja
/// catalogar no tumba el catálogo: se salta con su aviso y su motivo; solo falla si no se pudo con ninguna.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err("no llegó nada por stdin. `catalogo` espera la URL de Oracle por ahí".into());
    }
    let c = Conexion::abrir(url).map_err(|f| texto(f, url))?;
    let mut tablas = Vec::new();
    let mut saltadas: Vec<String> = Vec::new();
    for o in c.objetos().map_err(|f| texto(f, url))? {
        match c.tabla(&o.esquema, &o.nombre) {
            Ok(t) => {
                for a in avisos(&t) {
                    eprintln!("ore-read-oracle: aviso · `{}`: {a}", t.objeto());
                }
                tablas.push(tabla_de(&t, &o));
            }
            Err(f) => {
                let m = texto(f, url);
                eprintln!(
                    "ore-read-oracle: aviso · `{}.{}` no se cataloga: {m}",
                    o.esquema, o.nombre
                );
                saltadas.push(m);
            }
        }
    }
    if tablas.is_empty()
        && let Some(m) = saltadas.first()
    {
        return Err(format!(
            "ninguna de las {} tablas se pudo catalogar; la primera: {m}",
            saltadas.len()
        ));
    }
    Ok(escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
        objetos: Vec::new(),
    }))
}

/// Lo que quien cataloga debe saber de esta tabla y el catálogo no puede llevar.
fn avisos(t: &TablaOracle) -> Vec<String> {
    let mut a = Vec::new();
    for c in &t.columnas {
        match &c.tipo {
            Tipo::Otro(n) => a.push(format!(
                "la columna `{}` es {n}: se cataloga y no se lee (`{n}` pide envolver la columna en el SELECT)",
                c.nombre
            )),
            Tipo::FechaHora if c.tipo_oracle == "DATE" => a.push(format!(
                "la columna `{}` es un `DATE`: lleva hora (segundos) y se lee como `DateTime`",
                c.nombre
            )),
            Tipo::FechaHora if c.escala.unwrap_or(0) > 6 => a.push(format!(
                "la columna `{}` guarda nanosegundos y Arrow los guarda en microsegundos: se pierden los tres últimos dígitos",
                c.nombre
            )),
            Tipo::DecimalLibre if c.tipo_oracle == "NUMBER" => a.push(format!(
                "la columna `{}` es un `NUMBER` sin precisión: viaja como texto exacto (`String`); decláralo con `type: Decimal<p, s>` para tiparlo",
                c.nombre
            )),
            Tipo::Intervalo | Tipo::Documento => a.push(format!(
                "la columna `{}` es {}: sale como `Opaque` (texto)",
                c.nombre, c.tipo_oracle
            )),
            _ => {}
        }
    }
    a
}

/// Una tabla o vista de Oracle, como la tabla del catálogo.
fn tabla_de(t: &TablaOracle, o: &Objeto) -> Tabla {
    let columnas = t
        .columnas
        .iter()
        .map(|k| Columna {
            nombre: k.nombre.clone(),
            // `None`: este conector no supo traducirlo (no es un hueco a rellenar).
            tipo: k.tipo.se_lee().then(|| k.tipo.oos()),
            // El tipo de Oracle tal cual (`NUMBER(12,2)`, `TIMESTAMP(6) WITH TIME ZONE`): con él, el motor sabe
            // lo que el escalar no lleva.
            origen: Some(k.origen()),
            obligatoria: !k.anulable || t.clave.contains(&k.nombre),
            valores: Vec::new(),
            descripcion: k.descripcion.clone(),
        })
        .collect();
    Tabla {
        nombre: t.objeto(),
        columnas,
        clave: t.clave.clone(),
        unicas: t.unicas.clone(),
        filas_estimadas: o.filas_estimadas,
        clase: if o.vista { "view" } else { "table" }.to_string(),
        lee: Some(lee()),
        cambia: Some(cambios()),
        ..Tabla::default()
    }
}

/// La cara `I`: lo que la inducción declara que se puede empujar (lo que el conector v2 pone, menos `like`), y
/// el recorrido completo.
fn lee() -> Json {
    Json::obj([
        (
            "predicatePushdown",
            Json::Arr(
                ore_driver::EMPUJE_INDUCIDO
                    .iter()
                    .map(|o| Json::s(*o))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s("cheap")),
    ])
}

/// La cara `D`. Oracle tiene redo log (LogMiner), pero este conector todavía no lo lee: **no se declara lo que
/// no se hace**. Cuando llegue el CDC será una capa nueva, y este verbo cambiará con ella.
fn cambios() -> Json {
    Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))])
}

/// **`explorar`**: los esquemas con tablas o vistas que la URL deja ver. Lo que `catalogo` recorre si la URL no
/// nombra uno.
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    let objetos = c.objetos().map_err(|f| texto(f, &url))?;
    let mut por_esquema: BTreeMap<&str, usize> = BTreeMap::new();
    for o in &objetos {
        *por_esquema.entry(o.esquema.as_str()).or_default() += 1;
    }
    let contiene: Vec<Json> = por_esquema
        .iter()
        .map(|(e, n)| Json::obj([("nombre", Json::s(*e)), ("objetos", Json::Int(*n as i64))]))
        .collect();
    let nota = match &c.destino.esquema {
        Some(e) => format!("la URL nombra el esquema `{e}`: el catálogo recorre solo ese"),
        None => "la URL no nombra un esquema: el catálogo recorre todos los que ve el usuario, menos los que mantiene Oracle".to_string(),
    };
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
}

/// **`testigo`**: hasta dónde está el origen. Oracle sabe fecharse por posición (el SCN), pero este conector no
/// lee el redo todavía, así que `none`, que es una respuesta cierta (ADR 0016): devolver «ahora» inventaría una
/// marca que la lectura no respalda.
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    // Se abre para decir que el origen responde antes de decir que no sabe fecharse.
    Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    Ok(ore_driver::testigo("none", None))
}

/// **`cambio`** (ADR 0063 S3): ¿cambió ESTA tabla desde la última copia? Oracle no lleva contadores por tabla
/// que cuenten escrituras sin un permiso especial (`ORA_ROWSCN` por bloque no es una huella), así que **no hay
/// huella que respalde**: `respalda: false`, y la copia no se fía. Si la tabla no está es un error, no «no
/// respalda».
pub fn cambio(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let (esquema, nombre) = partir_objeto(&objeto).map_err(|f| texto(f, &url))?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    c.tabla(esquema, nombre).map_err(|f| texto(f, &url))?;
    Ok(ore_driver::huella("sin-contadores", None, false))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::transporte::Columna as ColOracle;

    fn col(n: &str, t: &str, p: Option<i32>, s: Option<i32>) -> ColOracle {
        ColOracle {
            nombre: n.into(),
            tipo: Tipo::de(t, p, s),
            tipo_oracle: t.into(),
            precision: p,
            escala: s,
            anulable: n != "ID",
            descripcion: (n == "TOTAL").then(|| "Importe con IVA".to_string()),
        }
    }

    fn tabla() -> TablaOracle {
        TablaOracle {
            esquema: "VENTAS".into(),
            nombre: "PEDIDOS".into(),
            columnas: vec![
                col("ID", "NUMBER", Some(10), Some(0)),
                col("TOTAL", "NUMBER", Some(12), Some(2)),
                col("LIBRE", "NUMBER", None, None),
                col("ALTA", "DATE", None, None),
                col("XML", "XMLTYPE", None, None),
            ],
            clave: vec!["ID".into()],
            unicas: vec![],
        }
    }

    #[test]
    fn la_tabla_del_catalogo_lleva_tipos_clave_y_comentarios() {
        let o = Objeto {
            esquema: "VENTAS".into(),
            nombre: "PEDIDOS".into(),
            vista: false,
            filas_estimadas: Some(15000),
        };
        let t = tabla_de(&tabla(), &o);
        assert_eq!(t.nombre, "VENTAS.PEDIDOS");
        assert_eq!(t.clase, "table");
        assert_eq!(t.clave, vec!["ID"]);
        assert_eq!(t.filas_estimadas, Some(15000));
        let tipos: Vec<_> = t.columnas.iter().map(|c| c.tipo.clone()).collect();
        assert_eq!(
            tipos,
            vec![
                Some("Integer".into()),
                Some("Decimal<12, 2>".into()),
                Some("String".into()),
                Some("DateTime".into()),
                // Lo que no se sabe leer no inventa un tipo.
                None,
            ]
        );
        assert_eq!(t.columnas[1].origen.as_deref(), Some("NUMBER(12,2)"));
        assert_eq!(
            t.columnas[1].descripcion.as_deref(),
            Some("Importe con IVA")
        );
        // La clave es obligatoria aunque Oracle la diga anulable (aquí `ID` ya es NOT NULL).
        assert!(t.columnas[0].obligatoria && !t.columnas[1].obligatoria);
    }

    #[test]
    fn los_avisos_dicen_lo_que_el_catalogo_no_puede_llevar() {
        let a = avisos(&tabla()).join("\n");
        assert!(a.contains("`LIBRE` es un `NUMBER` sin precisión"), "{a}");
        assert!(a.contains("`ALTA` es un `DATE`: lleva hora"), "{a}");
        assert!(a.contains("`XML` es XMLTYPE"), "{a}");
        assert!(!a.contains("`ID`") && !a.contains("`TOTAL`"), "{a}");
    }

    #[test]
    fn el_origen_no_declara_cambios_que_no_lee() {
        assert_eq!(cambios().jcs(), r#"{"mode":"none","witness":"none"}"#);
        assert!(lee().jcs().contains("\"fullScan\":\"cheap\""));
    }
}
