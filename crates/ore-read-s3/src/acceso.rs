//! **¿Puede ESTA credencial leer ESTE bucket?** Las acciones, una a una.
//!
//! Medido en F1, y es el primer fallo de cualquier cliente: la política dejaba
//! leer la configuración del bucket y **no listar**, porque `s3:ListBucket` va
//! sobre el ARN del bucket y no sobre `bucket/*`. Y sin listar, S3 contesta
//! `403` también a una clave que no existe, así que ni siquiera se sabe si
//! `GetObject` está concedido. Lo que la consola necesita para decirle al
//! cliente qué conceder es **qué acción falta y sobre qué ARN**, como el `check`
//! de BigQuery dice qué rol y dónde.
//!
//! | permiso | qué se prueba | acción | sobre |
//! |---|---|---|---|
//! | `listar` | una página de un objeto bajo el prefijo | `s3:ListBucket` | `arn:aws:s3:::<bucket>` |
//! | `leer` | un `HEAD` del primer objeto | `s3:GetObject` | `arn:aws:s3:::<bucket>/*` |
//! | `versiones` | una página de versiones | `s3:ListBucketVersions` | `arn:aws:s3:::<bucket>` |
//!
//! `versiones` no decide el `ok`: sin ella se cataloga igual, y lo que se
//! pierde es poder seguir a un objeto borrado (la iteración de borrados, E7).
//! Nada de esto baja un byte de un objeto.

use crate::fuente::Fuente;
use ore_core::json::Json;
use ore_s3::Respuesta;

struct Permiso {
    nombre: &'static str,
    accion: &'static str,
    sobre: String,
    ok: Option<bool>,
    porque: Option<String>,
    decide: bool,
}

/// El prefijo del motivo cuando no es un permiso sino la región: el mensaje no
/// puede decir que falta una acción que nadie ha negado.
const REGION: &str = "el bucket está en";

/// El motivo de un fallo, con lo que S3 dice de la región si es eso.
fn motivo(f: &Fuente, r: &Respuesta) -> String {
    if let Some(region) = r.cabecera("x-amz-bucket-region")
        && region != f.bucket.region
    {
        return format!(
            "{REGION} `{region}`, no en `{}`: pon `region={region}` en la URL",
            f.bucket.region
        );
    }
    r.motivo()
}

pub fn comprobar(f: &Fuente) -> String {
    let b = &f.bucket;
    let mut permisos = Vec::new();
    let mut primera: Option<String> = None;
    let mut prefijos: Vec<String> = Vec::new();

    let (ok, porque) = match ore_s3::listar(b, &f.prefijo, Some("/"), None, Some(50)) {
        Ok(p) => {
            primera = p
                .objetos
                .iter()
                .find(|o| !o.clave.ends_with('/'))
                .map(|o| o.clave.clone());
            prefijos = p.prefijos;
            if primera.is_none() {
                // En este nivel solo hay carpetas: se baja a buscar uno.
                if let Ok(q) = ore_s3::listar(b, &f.prefijo, None, None, Some(50)) {
                    primera = q
                        .objetos
                        .iter()
                        .find(|o| !o.clave.ends_with('/'))
                        .map(|o| o.clave.clone());
                }
            }
            (Some(true), None)
        }
        Err(Ok(r)) => (Some(false), Some(motivo(f, &r))),
        Err(Err(t)) => (Some(false), Some(t)),
    };
    permisos.push(Permiso {
        nombre: "listar",
        accion: "s3:ListBucket",
        sobre: b.arn(),
        ok,
        porque,
        decide: true,
    });

    let (ok, porque) = match &primera {
        None => (
            None,
            Some("no hay un objeto visible bajo el prefijo con el que probar".to_string()),
        ),
        Some(k) => match ore_s3::cabeza(b, k) {
            Ok(r) if r.ok() => (Some(true), None),
            Ok(r) => (Some(false), Some(motivo(f, &r))),
            Err(t) => (Some(false), Some(t)),
        },
    };
    permisos.push(Permiso {
        nombre: "leer",
        accion: "s3:GetObject",
        sobre: format!("{}/*", b.arn()),
        ok,
        porque,
        decide: true,
    });

    let v = ore_s3::pedir(
        b,
        "GET",
        None,
        &[
            ("versions", String::new()),
            ("max-keys", "1".into()),
            ("prefix", f.prefijo.clone()),
        ],
        Vec::new(),
    );
    let (ok, porque) = match v {
        Ok(r) if r.ok() => (Some(true), None),
        Ok(r) => (Some(false), Some(motivo(f, &r))),
        Err(t) => (Some(false), Some(t)),
    };
    permisos.push(Permiso {
        nombre: "versiones",
        accion: "s3:ListBucketVersions",
        sobre: b.arn(),
        ok,
        porque,
        decide: false,
    });

    let fallo = permisos.iter().find(|p| p.decide && p.ok == Some(false));
    let ok = fallo.is_none();
    let porque = fallo.map(|p| match p.porque.as_deref() {
        Some(m) if m.starts_with(REGION) => m.to_string(),
        m => format!(
            "falta `{}` sobre `{}`: {}",
            p.accion,
            p.sobre,
            m.unwrap_or("denegado")
        ),
    });
    let permisos = Json::Obj(
        permisos
            .iter()
            .map(|p| {
                let mut c = vec![("donde", Json::s(&p.sobre)), ("rol", Json::s(p.accion))];
                match p.ok {
                    Some(x) => c.push(("ok", Json::Bool(x))),
                    None => c.push(("probado", Json::Bool(false))),
                }
                if let Some(m) = &p.porque {
                    c.push(("porque", Json::s(m)));
                }
                (p.nombre.to_string(), Json::obj(c))
            })
            .collect(),
    );
    let mut o = vec![
        ("ok", Json::Bool(ok)),
        ("permisos", permisos),
        (
            "prefijos",
            Json::Arr(prefijos.iter().map(Json::s).collect()),
        ),
    ];
    if let Some(p) = porque {
        o.push(("porque", Json::s(p)));
    }
    Json::obj(o).jcs()
}

/// **Qué contiene el bucket**, un nivel: las carpetas bajo el prefijo, cada
/// una con su URL (sin credencial). Es lo que el alta ofrece para elegir.
pub fn explorar(f: &Fuente) -> Result<String, String> {
    let p = ore_s3::listar(&f.bucket, &f.prefijo, Some("/"), None, Some(1000)).map_err(
        |e| match e {
            Ok(r) => format!("no se pudo listar: {}", motivo(f, &r)),
            Err(t) => t,
        },
    )?;
    let mut contiene: Vec<Json> = p
        .prefijos
        .iter()
        .map(|pr| {
            Json::obj([
                ("nombre", Json::s(pr)),
                ("url", Json::s(crate::fuente::publica(f, pr))),
            ])
        })
        .collect();
    let sueltos = p.objetos.iter().filter(|o| !o.clave.ends_with('/')).count();
    if contiene.is_empty() && sueltos == 0 {
        return Err(format!(
            "`{}` no tiene nada visible con esta credencial. Una lista vacía tendría el mismo \
             aspecto que un bucket al que no se llega, así que se dice",
            crate::fuente::publica(f, &f.prefijo)
        ));
    }
    if sueltos > 0 {
        contiene.push(Json::obj([
            ("nombre", Json::s(&f.prefijo)),
            ("url", Json::s(crate::fuente::publica(f, &f.prefijo))),
            ("objetos", Json::Int(sueltos as i64)),
        ]));
    }
    Ok(Json::obj([("contiene", Json::Arr(contiene))]).pretty())
}
