//! **¿Puede ESTA app leer ESTE contenedor?** Se prueba, paso a paso.
//!
//! Azure no tiene en el plano de datos a quién preguntar los permisos (GCS sí:
//! `testIamPermissions`), así que `check` hace lo que hará el driver y dice, de
//! lo que falla, **qué falta y dónde**. Nada de esto baja un byte de un blob.
//!
//! | permiso | qué se prueba | lo que se concede | sobre |
//! |---|---|---|---|
//! | `identidad` | el canje en Entra: el token de Google de la celda por uno de Storage | una *federated identity credential* para la cuenta de la celda | la app del cliente |
//! | `listar` | una página bajo el prefijo | `Storage Blob Data Reader` | el contenedor |
//! | `leer` | un `HEAD` del primer blob | `Storage Blob Data Reader` | el contenedor |
//! | `versiones` | una página con `include=versions` | (el mismo) | el contenedor |
//! | `firmar` | la clave de delegación | `Storage Blob Delegator` | **la cuenta** |
//!
//! `versiones` no decide el `ok`: dice cómo se fija (`version`, o `etag` en una
//! cuenta sin versionado, como ADLS Gen2). `firmar` tampoco: sin ella se
//! cataloga y se copia igual, y lo que se pierde es dar URLs firmadas de los
//! ítems (se abren por `content`). Dos cosas no son un rol que falta y se dicen
//! como lo que son: un contenedor (o una cuenta) que no existe, y el firewall
//! de red de la cuenta (`AuthorizationFailure`).

use ore_azure::{Azure, Fallo, Meta};
use ore_core::json::Json;

const LECTOR: &str = "Storage Blob Data Reader";
const DELEGADOR: &str = "Storage Blob Delegator";
/// Los motivos que no son un rol: se dicen tal cual.
const NO_ESTA: &str = "no existe";
const RED: &str = "la cuenta no acepta";

struct Permiso {
    nombre: &'static str,
    rol: String,
    sobre: String,
    ok: Option<bool>,
    porque: Option<String>,
    decide: bool,
}

/// El primer blob (no una «carpeta») de un listado.
fn primero(metas: &[Meta]) -> Option<String> {
    metas
        .iter()
        .find(|m| !m.nombre.ends_with('/'))
        .map(|m| m.nombre.clone())
}

/// Lo que Azure contestó, dicho para quien da de alta.
fn explicar(a: &Azure, f: &Fallo) -> String {
    let (cuenta, contenedor) = (&a.fuente.cuenta, &a.fuente.contenedor);
    if f.estado == 404 && f.motivo().contains("ContainerNotFound") {
        return format!(
            "el contenedor `{contenedor}` {NO_ESTA} en la cuenta `{cuenta}` (404): ¿está bien escrito?"
        );
    }
    if f.estado == 0 && (f.cuerpo.contains("dns") || f.cuerpo.contains("Dns")) {
        return format!(
            "la cuenta `{cuenta}` {NO_ESTA} (su nombre no resuelve): ¿está bien escrito?"
        );
    }
    if f.estado == 403 && f.motivo().contains("AuthorizationFailure") {
        return format!(
            "{RED} peticiones desde aquí ({}): su firewall de red (o un private endpoint) no deja \
             entrar a esta celda; no es un rol que falte",
            f.motivo()
        );
    }
    f.motivo()
}

fn especial(m: &str) -> bool {
    m.contains(&format!(" {NO_ESTA} ")) || m.starts_with(RED)
}

pub fn comprobar(a: &Azure) -> String {
    let f = &a.fuente;
    let contenedor = format!("{}/{}", f.cuenta, f.contenedor);
    let mut permisos = Vec::new();
    let mut prefijos: Vec<String> = Vec::new();
    let mut fija = "version";

    let identidad = a.token();
    permisos.push(Permiso {
        nombre: "identidad",
        rol: "federated identity credential (issuer https://accounts.google.com, subject = la \
              cuenta de esta celda)"
            .into(),
        sobre: format!("app {} del tenant {}", f.cliente, f.tenant),
        ok: Some(identidad.is_ok()),
        porque: identidad.as_ref().err().cloned(),
        decide: true,
    });

    if identidad.is_ok() {
        let (lista, primera) = match a.listar(&f.prefijo, false, Some("/"), Some(50)) {
            Ok((metas, ps)) => {
                prefijos = ps;
                let mut primera = primero(&metas);
                if primera.is_none() {
                    // En este nivel solo hay carpetas: se baja a buscar uno.
                    primera = a
                        .listar(&f.prefijo, false, None, Some(50))
                        .ok()
                        .and_then(|(m, _)| primero(&m));
                }
                (Ok(()), primera)
            }
            Err(e) => (Err(explicar(a, &e)), None),
        };
        let listo = lista.is_ok();
        permisos.push(Permiso {
            nombre: "listar",
            rol: LECTOR.into(),
            sobre: contenedor.clone(),
            ok: Some(listo),
            porque: lista.err(),
            decide: true,
        });

        let (ok, porque) = match &primera {
            None if listo => (
                None,
                Some("no hay un blob visible bajo el prefijo con el que probar".to_string()),
            ),
            None => (None, Some("sin listar no hay con qué probar".to_string())),
            Some(k) => match a.meta(k, None) {
                Ok(_) => (Some(true), None),
                Err(e) => (Some(false), Some(explicar(a, &e))),
            },
        };
        permisos.push(Permiso {
            nombre: "leer",
            rol: LECTOR.into(),
            sobre: contenedor.clone(),
            ok,
            porque,
            decide: true,
        });

        let (ok, porque) = match a.listar(&f.prefijo, true, None, Some(50)) {
            Ok((metas, _)) if metas.iter().any(|m| m.version.is_some()) => (Some(true), None),
            Ok(_) => {
                fija = "etag";
                (
                    Some(true),
                    Some(
                        "la cuenta no versiona (o es ADLS Gen2, que no puede): cada blob se fija \
                         por su ETag, lo que cambió entre listar y leer es un 412, y sus ítems no \
                         dan URL firmada (se abren por `content`)"
                            .to_string(),
                    ),
                )
            }
            Err(e) => (Some(false), Some(explicar(a, &e))),
        };
        permisos.push(Permiso {
            nombre: "versiones",
            rol: LECTOR.into(),
            sobre: contenedor.clone(),
            ok,
            porque,
            decide: false,
        });

        let (ok, porque) = match a.clave_de_delegacion(60) {
            Ok(_) => (Some(true), None),
            Err(e) => (
                Some(false),
                Some(format!(
                    "{}: sin la clave de delegación no hay URLs firmadas de los ítems (se abren \
                     por `content`); se cataloga y se copia igual",
                    explicar(a, &e)
                )),
            ),
        };
        permisos.push(Permiso {
            nombre: "firmar",
            rol: DELEGADOR.into(),
            sobre: f.cuenta.clone(),
            ok,
            porque,
            decide: false,
        });
    }

    let fallo = permisos.iter().find(|p| p.decide && p.ok == Some(false));
    let ok = fallo.is_none();
    let porque = fallo.map(|p| match p.porque.as_deref() {
        Some(m) if especial(m) => m.to_string(),
        Some(m) if p.nombre == "identidad" => format!("no hay identidad con la que leer: {m}"),
        m => format!(
            "falta `{}` sobre `{}`: {}",
            p.rol,
            p.sobre,
            m.unwrap_or("denegado")
        ),
    });
    let permisos = Json::Obj(
        permisos
            .iter()
            .map(|p| {
                let mut c = vec![("donde", Json::s(&p.sobre)), ("rol", Json::s(&p.rol))];
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
        ("fija", Json::s(fija)),
        ("como", Json::s("probando")),
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

/// **Qué contiene el contenedor**, un nivel: las carpetas bajo el prefijo,
/// cada una con su URL. Es lo que el alta ofrece para elegir.
pub fn explorar(a: &Azure) -> Result<String, String> {
    let f = &a.fuente;
    let (metas, prefijos) = a
        .listar(&f.prefijo, false, Some("/"), Some(1000))
        .map_err(|e| format!("no se pudo listar: {}", explicar(a, &e)))?;
    let mut contiene: Vec<Json> = prefijos
        .iter()
        .map(|pr| {
            Json::obj([
                ("nombre", Json::s(pr)),
                ("url", Json::s(ore_azure::publica(f, pr))),
            ])
        })
        .collect();
    let sueltos = metas.iter().filter(|m| !m.nombre.ends_with('/')).count();
    if contiene.is_empty() && sueltos == 0 {
        return Err(format!(
            "`{}` no tiene nada visible con esta identidad. Una lista vacía tendría el mismo \
             aspecto que un contenedor al que no se llega, así que se dice",
            ore_azure::publica(f, &f.prefijo)
        ));
    }
    if sueltos > 0 {
        contiene.push(Json::obj([
            ("nombre", Json::s(&f.prefijo)),
            ("url", Json::s(ore_azure::publica(f, &f.prefijo))),
            ("objetos", Json::Int(sueltos as i64)),
        ]));
    }
    Ok(Json::obj([("contiene", Json::Arr(contiene))]).pretty())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn lo_que_no_es_un_rol_se_dice_como_lo_que_es() {
        let a = Azure::de_url("az://cuenta/cubo?tenant=t&cliente=c").unwrap();
        let f = |estado, cuerpo: &str| Fallo {
            estado,
            codigo: String::new(),
            cuerpo: cuerpo.into(),
        };
        let m = explicar(&a, &f(404, "<Error><Code>ContainerNotFound</Code></Error>"));
        assert!(especial(&m) && m.contains("`cubo`"), "{m}");
        let m = explicar(&a, &f(0, "Azure no contesta: Dns Failed: resolve dns name"));
        assert!(especial(&m) && m.contains("`cuenta`"), "{m}");
        let m = explicar(
            &a,
            &f(403, "<Error><Code>AuthorizationFailure</Code></Error>"),
        );
        assert!(especial(&m) && m.contains("firewall"), "{m}");
        let m = explicar(
            &a,
            &f(
                403,
                "<Error><Code>AuthorizationPermissionMismatch</Code></Error>",
            ),
        );
        assert!(
            !especial(&m) && m.contains("AuthorizationPermissionMismatch"),
            "{m}"
        );
    }
}
