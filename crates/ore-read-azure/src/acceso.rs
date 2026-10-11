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
//!
//! ⭐ D-O6 · **Un contenedor público** (`?publico=1`) no tiene identidad que
//! probar: su primer paso es `publico`, el nivel de acceso que el contenedor
//! dice sin identidad (`container` deja listar; `blob`, sólo leer blobs
//! sueltos, y ORE necesita listar). Un contenedor privado y uno que no existe
//! contestan lo mismo sin identidad (`401 NoAuthenticationInformation`), y así
//! se dice. `firmar` no se prueba: sin identidad no hay clave de delegación.

use ore_azure::{Azure, Fallo, Meta};
use ore_core::json::Json;

const LECTOR: &str = "Storage Blob Data Reader";
const DELEGADOR: &str = "Storage Blob Delegator";
const PUBLICO: &str = "public access level `container` (anonymous read and list)";
/// Los motivos que no son un rol: se dicen tal cual.
const NO_ESTA: &str = "does not exist";
const RED: &str = "the account does not accept";
const NO_PUBLICO: &str = "is not public";

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
            "the container `{contenedor}` {NO_ESTA} in the account `{cuenta}` (404): is it spelled right?"
        );
    }
    if f.estado == 0 && (f.cuerpo.contains("dns") || f.cuerpo.contains("Dns")) {
        return format!(
            "the account `{cuenta}` {NO_ESTA} (its name does not resolve): is it spelled right?"
        );
    }
    // D-O6: sin identidad, Azure no distingue un contenedor privado de uno que
    // no existe (`401`); Azurite contesta `403 AuthorizationFailure`, el mismo
    // código que el firewall, así que sin identidad se dicen las dos. Una
    // cuenta con el acceso público apagado contesta `409`.
    if a.es_publico() {
        let m = f.motivo();
        if f.estado == 409 && m.contains("PublicAccessNotPermitted") {
            return format!(
                "the account `{cuenta}` {NO_PUBLICO}: it does not allow public access \
                 (AllowBlobPublicAccess=false), so its containers are read through an Entra app \
                 (`tenant` and `cliente`)"
            );
        }
        if f.estado == 401 && m.contains("NoAuthenticationInformation") {
            return format!(
                "the container `{contenedor}` of `{cuenta}` {NO_PUBLICO}, or does not exist \
                 (without identity Azure answers the same to both): a private container is read \
                 through an Entra app (`tenant` and `cliente`)"
            );
        }
        if f.estado == 403 && m.contains("AuthorizationFailure") {
            return format!(
                "the container `{contenedor}` of `{cuenta}` {NO_PUBLICO}, or the account's network \
                 firewall does not let this cell in ({m})"
            );
        }
    }
    if f.estado == 403 && f.motivo().contains("AuthorizationFailure") {
        return format!(
            "{RED} requests from here ({}): its network firewall (or a private endpoint) does not \
             let this cell in; it is not a missing role",
            f.motivo()
        );
    }
    f.motivo()
}

fn especial(m: &str) -> bool {
    m.contains(&format!(" {NO_ESTA} ")) || m.starts_with(RED) || m.contains(NO_PUBLICO)
}

/// D-O6 · El primer paso de un contenedor público: su nivel de acceso.
fn publico(a: &Azure) -> Permiso {
    let f = &a.fuente;
    let (ok, porque) = match a.nivel_publico() {
        Ok(Some(n)) if n == "container" => (true, None),
        Ok(Some(n)) if n == "blob" => (
            false,
            Some(format!(
                "the container `{}` {NO_PUBLICO} for listing: its level is `blob` (only individual \
                 blobs can be read anonymously), and ORE needs to list it",
                f.contenedor
            )),
        ),
        Ok(n) => (
            false,
            Some(format!(
                "the container `{}` {NO_PUBLICO} (public access: `{}`)",
                f.contenedor,
                n.as_deref().unwrap_or("none")
            )),
        ),
        Err(e) => (false, Some(explicar(a, &e))),
    };
    Permiso {
        nombre: "publico",
        rol: PUBLICO.into(),
        sobre: format!("{}/{}", f.cuenta, f.contenedor),
        ok: Some(ok),
        porque,
        decide: true,
    }
}

pub fn comprobar(a: &Azure) -> String {
    let f = &a.fuente;
    let contenedor = format!("{}/{}", f.cuenta, f.contenedor);
    let mut permisos = Vec::new();
    let mut prefijos: Vec<String> = Vec::new();
    let mut fija = "version";
    // Un contenedor público no tiene rol que conceder: lo que se prueba es su
    // nivel de acceso.
    let lector = if a.es_publico() { PUBLICO } else { LECTOR };

    let entrada = match &f.app {
        None => publico(a),
        Some(app) => {
            let identidad = a.token();
            Permiso {
                nombre: "identidad",
                rol: "federated identity credential (issuer https://accounts.google.com, subject \
                      = this cell's account)"
                    .into(),
                sobre: format!("app {} of tenant {}", app.cliente, app.tenant),
                ok: Some(identidad.is_ok()),
                porque: identidad.err(),
                decide: true,
            }
        }
    };
    let dentro = entrada.ok == Some(true);
    permisos.push(entrada);

    if dentro {
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
            rol: lector.into(),
            sobre: contenedor.clone(),
            ok: Some(listo),
            porque: lista.err(),
            decide: true,
        });

        let (ok, porque) = match &primera {
            None if listo => (
                None,
                Some("there is no visible blob under the prefix to test with".to_string()),
            ),
            None => (
                None,
                Some("without listing there is nothing to test with".to_string()),
            ),
            Some(k) => match a.meta(k, None) {
                Ok(_) => (Some(true), None),
                Err(e) => (Some(false), Some(explicar(a, &e))),
            },
        };
        permisos.push(Permiso {
            nombre: "leer",
            rol: lector.into(),
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
                        "the account does not version (or is ADLS Gen2, which cannot): each blob \
                         is pinned by its ETag, a change between listing and reading is a 412, \
                         and its items give no signed URL (they open through `content`)"
                            .to_string(),
                    ),
                )
            }
            Err(e) => (Some(false), Some(explicar(a, &e))),
        };
        permisos.push(Permiso {
            nombre: "versiones",
            rol: lector.into(),
            sobre: contenedor.clone(),
            ok,
            porque,
            decide: false,
        });

        let (ok, porque) = if a.es_publico() {
            (
                None,
                Some(
                    "a public container is read without identity: there is no user delegation \
                     key, so its items give no signed URL (they open through `content`); it is \
                     catalogued and copied the same"
                        .to_string(),
                ),
            )
        } else {
            match a.clave_de_delegacion(60) {
                Ok(_) => (Some(true), None),
                Err(e) => (
                    Some(false),
                    Some(format!(
                        "{}: without the user delegation key the items give no signed URL (they \
                         open through `content`); it is catalogued and copied the same",
                        explicar(a, &e)
                    )),
                ),
            }
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
        Some(m) if p.nombre == "identidad" => format!("there is no identity to read with: {m}"),
        m => format!(
            "missing `{}` on `{}`: {}",
            p.rol,
            p.sobre,
            m.unwrap_or("denied")
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
    let como = if a.es_publico() {
        "publico"
    } else {
        "probando"
    };
    let mut o = vec![
        ("ok", Json::Bool(ok)),
        ("fija", Json::s(fija)),
        ("como", Json::s(como)),
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
        .map_err(|e| format!("could not list: {}", explicar(a, &e)))?;
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
            "`{}` has nothing visible with this identity. An empty list would look the same as a \
             container that cannot be reached, so it is said",
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
        let m = explicar(
            &a,
            &f(0, "Azure is not responding: Dns Failed: resolve dns name"),
        );
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
        // D-O6: sin identidad, un contenedor privado y uno que no existe son
        // lo mismo, y se dice; con identidad, un 401 es lo que es.
        let no = f(
            401,
            "<Error><Code>NoAuthenticationInformation</Code></Error>",
        );
        let p = Azure::de_url("az://cuenta/cubo?publico=1").unwrap();
        let m = explicar(&p, &no);
        assert!(
            especial(&m) && m.contains("is not public") && m.contains("`cubo`"),
            "{m}"
        );
        assert!(!especial(&explicar(&a, &no)));
        let m = explicar(
            &p,
            &f(403, "<Error><Code>AuthorizationFailure</Code></Error>"),
        );
        assert!(m.contains("is not public") && m.contains("firewall"), "{m}");
        let m = explicar(
            &p,
            &f(409, "<Error><Code>PublicAccessNotPermitted</Code></Error>"),
        );
        assert!(especial(&m) && m.contains("AllowBlobPublicAccess"), "{m}");
    }
}
