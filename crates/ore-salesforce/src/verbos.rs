//! **Los verbos que son de Salesforce** y no del esqueleto: `catalogo`, `check`
//! y `explorar`. Los de lectura (`capacidades`, `leer`, `estimar`, `servir`)
//! están en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por
//! stdout, o el motivo del fallo (que va a stderr, literal).

use crate::esquema::Objeto;
use crate::lectura::{abrir, describir};
use crate::transporte::Cliente;
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Tabla, escribir};
use ore_driver::{Codigo, Fallo};
use serde_json::Value;
use std::process::ExitCode;

/// Los objetos de CRM que `catalogo` recorre si la URL no trae `objetos=`,
/// además de los propios de la org. Una org tiene ~1.700 objetos: describirlos
/// todos serían ~1.700 peticiones de las 15.000 del día (X0 A1, F3).
pub const NUCLEO: &[&str] = &[
    "Account",
    "Contact",
    "Lead",
    "Opportunity",
    "Case",
    "Task",
    "Event",
    "User",
    "Campaign",
    "Product2",
    "Pricebook2",
    "Contract",
    "Order",
];

pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-salesforce: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(
            entrada.trim(),
            args.first().map_or("salesforce", String::as_str),
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
            eprintln!("ore-read-salesforce: {m}");
            ExitCode::FAILURE
        }
    }
}

fn texto(f: Fallo, url: &str) -> String {
    f.tapado(url).mensaje
}

/// Todos los objetos consultables de la org, de `describeGlobal` (X0 A1).
fn globales(c: &Cliente) -> Result<Vec<(String, bool)>, Fallo> {
    let t = c.get(&c.api("/sobjects"))?;
    let v: Value = serde_json::from_str(&t)
        .map_err(|e| Fallo::new(Codigo::Esquema, format!("`describeGlobal` no es JSON: {e}")))?;
    Ok(v["sobjects"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|o| o["queryable"].as_bool().unwrap_or(false))
                .filter_map(|o| {
                    Some((
                        o["name"].as_str()?.to_string(),
                        o["custom"].as_bool().unwrap_or(false),
                    ))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// La cara `I`: lo que se puede empujar y cuánto cuesta recorrerlo entero.
/// **Medido** (X0 B): proyección y filtros sí; recorrerlo todo gasta cuota.
fn lee() -> Json {
    Json::obj([
        (
            "predicatePushdown",
            Json::Arr(
                ["eq", "neq", "in", "range", "like", "isNull"]
                    .iter()
                    .map(|o| Json::s(*o))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s("expensive")),
        ("maxRowsPerRequest", Json::Int(2000)),
    ])
}

/// Los objetos cuyos borrados **no se leen con fiabilidad**: se borran sin pasar
/// por la Papelera (`OpportunityLineItem`: `getDeleted` no los ve, X0 §2 Fivetran)
/// o dependen de otro objeto. Fivetran los **reimporta enteros**; aquí no
/// declaran cambios y la copia los refresca completos.
pub const SIN_CAMBIOS: &[&str] = &[
    "OpportunityLineItem",
    "OpportunitySplit",
    "OpportunityLineItemSchedule",
    "QuoteLineItem",
    "AccountTeamMember",
    "OpportunityTeamMember",
];

/// ¿Sabe este objeto decir sus cambios? Con `SystemModstamp` (o
/// `LastModifiedDate`) para las altas y actualizaciones, `IsDeleted` para sus
/// borrados (de `queryAll`), replicable, y que no sea de [`SIN_CAMBIOS`].
pub fn con_cambios(o: &Objeto) -> Option<&str> {
    let cursor = o.cursor()?;
    let borrados = o.campo("IsDeleted").is_some_and(|c| c.filtrable);
    (o.replicable && borrados && !SIN_CAMBIOS.contains(&o.nombre.as_str())).then_some(cursor)
}

/// La cara `D`, **declarada por lo que el objeto sabe de verdad** (ADR 0083
/// §1.8). `upsert` con clave `Id` y marca de agua: `append` con `field` no
/// compila y no mantiene una entidad (OOS2021, OOS2023), y con clave la
/// relectura solapada es idempotente. Lo demás, `none`: no se inventa.
///
/// ⚠️ Un `upsert` promete borrados, y se entregan con la columna `_ore_op`
/// (provisional, la de `ore-mongodb`) **cuando la petición la pide**.
fn cambia_de(o: &Objeto) -> Json {
    match con_cambios(o) {
        Some(cursor) => Json::obj([
            ("mode", Json::s("upsert")),
            ("key", Json::Arr(vec![Json::s("Id")])),
            ("witness", Json::s("field")),
            ("field", Json::s(cursor)),
        ]),
        None => Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))]),
    }
}

fn tabla_de(o: &Objeto) -> Tabla {
    Tabla {
        nombre: o.nombre.clone(),
        columnas: o.columnas().map(Objeto::columna_del_catalogo).collect(),
        clave: vec!["Id".to_string()],
        clase: "table".to_string(),
        lee: Some(lee()),
        cambia: Some(cambia_de(o)),
        ..Tabla::default()
    }
}

/// **`catalogo`**: los objetos pedidos (`objetos=` en la URL), o el núcleo de
/// CRM y los propios, cada uno con las columnas de su `describe`. Un objeto
/// que no existe en la org (`Order` sin la función activada) se salta con su
/// aviso: no tumba el catálogo.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err(
            "no llegó nada por stdin. `catalogo` espera la URL de Salesforce por ahí".into(),
        );
    }
    let c = abrir(url).map_err(|f| texto(f, url))?;
    let objetos: Vec<String> = if c.fuente.objetos.is_empty() {
        let globales = globales(&c).map_err(|f| texto(f, url))?;
        let mut v: Vec<String> = NUCLEO
            .iter()
            .filter(|n| globales.iter().any(|(g, _)| g == *n))
            .map(|n| n.to_string())
            .collect();
        v.extend(
            globales
                .into_iter()
                .filter(|(_, propio)| *propio)
                .map(|(n, _)| n),
        );
        v
    } else {
        c.fuente.objetos.clone()
    };
    let (mut tablas, mut saltadas) = (Vec::new(), Vec::new());
    for n in &objetos {
        match describir(&c, n) {
            Ok(o) if o.consultable => tablas.push(tabla_de(&o)),
            Ok(_) => saltadas.push(format!("`{n}` no es consultable")),
            Err(f) => {
                let m = texto(f, url);
                eprintln!("ore-read-salesforce: aviso · `{n}` no se cataloga: {m}");
                saltadas.push(m);
            }
        }
    }
    if tablas.is_empty()
        && let Some(m) = saltadas.first()
    {
        return Err(format!(
            "ninguno de los {} objetos se pudo catalogar; el primero: {m}",
            objetos.len()
        ));
    }
    Ok(escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
        objetos: Vec::new(),
    }))
}

/// **`check`**: ¿responde esta fuente? Por pasos, y cada fallo dice **qué
/// tocar** (F1, F2: lo que de verdad falló al montar la app).
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = (|| -> Result<String, Fallo> {
        let c = abrir(&url)?;
        // El canje es lo primero que se prueba: identidad, ámbitos y política.
        let limites = c.get(&c.api("/limits"))?;
        let v: Value = serde_json::from_str(&limites).unwrap_or_default();
        let dia = &v["DailyApiRequests"];
        let n = globales(&c)?.len();
        Ok(format!(
            "identidad ok · cuota diaria {} de {} · {n} objetos consultables",
            dia["Max"].as_i64().unwrap_or(0) - dia["Remaining"].as_i64().unwrap_or(0),
            dia["Max"].as_i64().unwrap_or(0),
        ))
    })();
    Ok(match hecho {
        Ok(resumen) => {
            eprintln!("ore-read-salesforce: {resumen}");
            ore_driver::comprobacion(true, None)
        }
        Err(f) => ore_driver::comprobacion(false, Some(&texto(f, &url))),
    })
}

/// **`testigo`** (ADR 0016): hasta dónde está la org **ahora**, para fijar el
/// final de una lectura por cursor. Es la hora **de la org** (la cabecera `Date`
/// de Salesforce) como instante UTC en texto: ordinal y comparable con
/// `SystemModstamp`. Un objeto que no dice sus cambios contesta `none`, que es
/// una respuesta cierta: devolver «ahora» inventaría una marca que no respalda.
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    if objeto.is_empty() {
        return Err("`testigo` pide el objeto: la marca es de un objeto y no de la org".into());
    }
    let c = abrir(&url).map_err(|f| texto(f, &url))?;
    let o = describir(&c, &objeto).map_err(|f| texto(f, &url))?;
    if con_cambios(&o).is_none() {
        return Ok(ore_driver::testigo("none", None));
    }
    let ahora = c.ahora().map_err(|f| texto(f, &url))?;
    Ok(ore_driver::testigo(
        "field",
        Some(&crate::valores::texto_de_instante(ahora)),
    ))
}

/// **`cambio`** (ADR 0063 S3): ¿cambió ESTE objeto desde la última copia sin leer
/// una fila? Salesforce **no tiene contadores por objeto**: un `COUNT()` más el
/// `MAX(SystemModstamp)` no ve las fórmulas (que no mueven el sello, X0 §2) ni un
/// cambio en el mismo segundo, así que **no se respalda** (`respalda: false`) y la
/// copia no se fía, que es lo seguro. Si el objeto no existe es un error.
pub fn cambio(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let c = abrir(&url).map_err(|f| texto(f, &url))?;
    describir(&c, &objeto).map_err(|f| texto(f, &url))?;
    Ok(ore_driver::huella("sin-contadores", None, false))
}

/// **`explorar`**: los objetos consultables de la org (los que `catalogo`
/// puede recorrer si se piden con `objetos=`).
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = abrir(&url).map_err(|f| texto(f, &url))?;
    let g = globales(&c).map_err(|f| texto(f, &url))?;
    let contiene: Vec<Json> = g
        .iter()
        .map(|(n, propio)| Json::obj([("nombre", Json::s(n)), ("propio", Json::Bool(*propio))]))
        .collect();
    let nota = format!(
        "{} objetos consultables. Sin `objetos=` en la URL, el catálogo recorre el núcleo de CRM ({}) y los propios; con `objetos=A,B`, solo esos",
        g.len(),
        NUCLEO.join(", ")
    );
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::esquema::{Campo, pruebas::cuenta};

    fn json(o: &Objeto) -> String {
        cambia_de(o).jcs()
    }

    /// ADR 0083 §1.8: la cara `D` de una cuenta es `upsert` con clave `Id` y marca de agua.
    #[test]
    fn una_cuenta_declara_upsert_por_id_con_su_marca_de_agua() {
        assert_eq!(
            json(&cuenta()),
            r#"{"field":"SystemModstamp","key":["Id"],"mode":"upsert","witness":"field"}"#
        );
    }

    /// Lo que no sabe decir sus borrados no declara cambios: no se inventa.
    #[test]
    fn lo_que_no_lee_sus_borrados_no_declara_cambios() {
        let none = r#"{"mode":"none","witness":"none"}"#;
        // Los objetos de SIN_CAMBIOS (Fivetran los reimporta).
        let mut o = cuenta();
        o.nombre = "OpportunityLineItem".into();
        assert_eq!(json(&o), none);
        // Sin `IsDeleted` no hay borrados que leer.
        let mut o = cuenta();
        o.campos.retain(|c| c.nombre != "IsDeleted");
        assert_eq!(json(&o), none);
        // Sin `SystemModstamp`, el cursor es `LastModifiedDate`; sin ninguno de los
        // dos, no hay cursor y no hay cambios.
        let mut o = cuenta();
        o.campos.retain(|c| c.nombre != "SystemModstamp");
        o.campos.push(Campo {
            nombre: "LastModifiedDate".into(),
            tipo: "datetime".into(),
            filtrable: true,
            ordenable: true,
            ..o.campo("CreatedDate").unwrap().clone()
        });
        assert_eq!(
            json(&o),
            r#"{"field":"LastModifiedDate","key":["Id"],"mode":"upsert","witness":"field"}"#
        );
        o.campos.retain(|c| c.nombre != "LastModifiedDate");
        assert_eq!(json(&o), none);
        // No replicable (X0 A1: 591 objetos consultables no lo son).
        let mut o = cuenta();
        o.replicable = false;
        assert_eq!(json(&o), none);
    }

    #[test]
    fn la_cara_d_va_en_el_catalogo_por_objeto() {
        let t = tabla_de(&cuenta());
        assert!(t.cambia.unwrap().jcs().contains(r#""mode":"upsert""#));
    }
}
