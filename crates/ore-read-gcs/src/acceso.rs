//! **¿Puede ESTA identidad leer ESTE bucket?** Se le pregunta a GCS.
//!
//! S3 no tiene a quién preguntar y prueba a ciegas (`ore-read-s3`, `acceso.rs`).
//! GCS sí: `testIamPermissions` sobre el bucket dice, de los permisos que se le
//! piden, cuáles tiene quien pregunta —la cuenta de la celda o la suplantada—,
//! sin bajar nada y sin depender de que haya un objeto con el que probar. Aun
//! así se lista una página: es lo que enseña las carpetas, y un permiso que IAM
//! concede puede negarlo otra cosa (un perímetro de VPC Service Controls). Si
//! el servidor no sabe contestar (el emulador), se prueba listando y leyendo.
//!
//! | permiso | qué | lo que se concede | sobre |
//! |---|---|---|---|
//! | `identidad` | con `suplantar`: el token de la cuenta del cliente | `roles/iam.serviceAccountTokenCreator` a la cuenta de la celda | la cuenta del cliente |
//! | `listar` | `storage.objects.list` | `roles/storage.objectViewer` | el bucket |
//! | `leer` | `storage.objects.get` | `roles/storage.objectViewer` | el bucket |
//!
//! Las generaciones viejas se listan y se leen con los mismos dos permisos: no
//! hay un tercero que pueda faltar, y cada objeto se fija siempre por su
//! generación (`fija: version`).

use ore_core::json::Json;
use ore_gcs::Gcs;

const LISTAR: &str = "storage.objects.list";
const LEER: &str = "storage.objects.get";
const LECTOR: &str = "roles/storage.objectViewer";
const SUPLANTAR: &str = "roles/iam.serviceAccountTokenCreator";
/// El prefijo del motivo cuando no es un permiso sino el bucket, que no está:
/// el mensaje no puede decir que falta un rol que nadie ha negado.
const NO_ESTA: &str = "el bucket no existe";

struct Permiso {
    nombre: &'static str,
    permiso: Option<&'static str>,
    rol: &'static str,
    sobre: String,
    ok: Option<bool>,
    porque: Option<String>,
}

/// El primer objeto (no una «carpeta») de un listado.
fn primero(metas: &[ore_gcs::Meta]) -> Option<String> {
    metas
        .iter()
        .find(|m| !m.nombre.ends_with('/'))
        .map(|m| m.nombre.clone())
}

/// Lo que se sabe de un permiso: lo que la prueba dio, si se hizo —lo que
/// pasa de verdad, que IAM puede no ver entero (un perímetro que niega, la ACL
/// de un objeto en un bucket sin acceso uniforme que concede)—; si no, lo que
/// IAM dice.
fn veredicto(
    iam: Option<bool>,
    prueba: Option<Result<(), String>>,
) -> (Option<bool>, Option<String>) {
    match (iam, prueba) {
        (_, Some(Err(m))) => (Some(false), Some(m)),
        (_, Some(Ok(()))) | (Some(true), None) => (Some(true), None),
        (Some(false), None) => (
            Some(false),
            Some("`testIamPermissions`: esta identidad no lo tiene".into()),
        ),
        (None, None) => (
            None,
            Some("no hay un objeto visible bajo el prefijo con el que probar".into()),
        ),
    }
}

pub fn comprobar(g: &Gcs) -> String {
    let f = &g.fuente;
    let bucket = format!("projects/_/buckets/{}", f.bucket);
    let mut permisos = Vec::new();
    let mut prefijos: Vec<String> = Vec::new();
    let mut como = "testIamPermissions";

    // Sin identidad no hay nada que preguntar.
    let identidad = g.identidad();
    if let Some(cuenta) = &f.suplantar {
        permisos.push(Permiso {
            nombre: "identidad",
            permiso: Some("iam.serviceAccounts.getAccessToken"),
            rol: SUPLANTAR,
            sobre: cuenta.clone(),
            ok: Some(identidad.is_ok()),
            porque: identidad.as_ref().err().cloned(),
        });
    }

    if identidad.is_ok() {
        // Lo que IAM dice; `None`: el servidor no lo sabe, o no contestó.
        let iam = match g.permisos(&[LISTAR, LEER]) {
            Ok(Some(tiene)) => Some(tiene),
            Ok(None) | Err(_) => {
                como = "probando";
                None
            }
        };
        let tiene = |p: &str| iam.as_ref().map(|t| t.iter().any(|x| x == p));

        let (lista, primera) = match g.pagina(&f.prefijo, Some("/"), 50) {
            Ok((metas, ps)) => {
                prefijos = ps;
                let mut primera = primero(&metas);
                if primera.is_none() {
                    // En este nivel solo hay carpetas: se baja a buscar uno.
                    primera = g
                        .pagina(&f.prefijo, None, 50)
                        .ok()
                        .and_then(|(m, _)| primero(&m));
                }
                (Ok(()), primera)
            }
            Err(e) if e.estado == 404 => (
                Err(format!(
                    "{NO_ESTA} (`{}`, 404): ¿está bien escrito el nombre?",
                    f.bucket
                )),
                None,
            ),
            Err(e) => (Err(e.motivo()), None),
        };
        let (ok, porque) = veredicto(tiene(LISTAR), Some(lista));
        permisos.push(Permiso {
            nombre: "listar",
            permiso: Some(LISTAR),
            rol: LECTOR,
            sobre: bucket.clone(),
            ok,
            porque,
        });

        let prueba = primera.map(|k| g.meta(&k, None).map(|_| ()).map_err(|e| e.motivo()));
        let (ok, porque) = veredicto(tiene(LEER), prueba);
        permisos.push(Permiso {
            nombre: "leer",
            permiso: Some(LEER),
            rol: LECTOR,
            sobre: bucket.clone(),
            ok,
            porque,
        });
    }

    let fallo = permisos.iter().find(|p| p.ok == Some(false));
    let ok = identidad.is_ok() && fallo.is_none();
    let porque = match (&identidad, fallo) {
        (Err(m), None) => Some(format!("no hay identidad con la que leer: {m}")),
        (_, Some(p)) if p.porque.as_deref().is_some_and(|m| m.starts_with(NO_ESTA)) => {
            p.porque.clone()
        }
        (_, Some(p)) => Some(format!(
            "falta `{}` sobre `{}`{}: {}",
            p.rol,
            p.sobre,
            p.permiso.map(|x| format!(" ({x})")).unwrap_or_default(),
            p.porque.as_deref().unwrap_or("denegado")
        )),
        _ => None,
    };
    let permisos = Json::Obj(
        permisos
            .iter()
            .map(|p| {
                let mut c = vec![("donde", Json::s(&p.sobre)), ("rol", Json::s(p.rol))];
                if let Some(x) = p.permiso {
                    c.push(("permiso", Json::s(x)));
                }
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
        ("fija", Json::s("version")),
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

/// **Qué contiene el bucket**, un nivel: las carpetas bajo el prefijo, cada
/// una con su URL. Es lo que el alta ofrece para elegir.
pub fn explorar(g: &Gcs) -> Result<String, String> {
    let f = &g.fuente;
    let (metas, prefijos) = g
        .pagina(&f.prefijo, Some("/"), 1000)
        .map_err(|e| format!("no se pudo listar: {}", e.motivo()))?;
    let mut contiene: Vec<Json> = prefijos
        .iter()
        .map(|pr| {
            Json::obj([
                ("nombre", Json::s(pr)),
                ("url", Json::s(ore_gcs::publica(f, pr))),
            ])
        })
        .collect();
    let sueltos = metas.iter().filter(|m| !m.nombre.ends_with('/')).count();
    if contiene.is_empty() && sueltos == 0 {
        return Err(format!(
            "`{}` no tiene nada visible con esta identidad. Una lista vacía tendría el mismo \
             aspecto que un bucket al que no se llega, así que se dice",
            ore_gcs::publica(f, &f.prefijo)
        ));
    }
    if sueltos > 0 {
        contiene.push(Json::obj([
            ("nombre", Json::s(&f.prefijo)),
            ("url", Json::s(ore_gcs::publica(f, &f.prefijo))),
            ("objetos", Json::Int(sueltos as i64)),
        ]));
    }
    Ok(Json::obj([("contiene", Json::Arr(contiene))]).pretty())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn manda_la_prueba_y_sin_ella_lo_que_iam_dice() {
        let err = || Some(Err::<(), _>("403 denegado".to_string()));
        assert_eq!(veredicto(Some(true), err()).0, Some(false));
        assert_eq!(veredicto(None, err()).0, Some(false));
        assert_eq!(veredicto(Some(false), Some(Ok(()))).0, Some(true));
        assert_eq!(veredicto(Some(false), None).0, Some(false));
        assert_eq!(veredicto(Some(true), None), (Some(true), None));
        assert_eq!(veredicto(None, Some(Ok(()))), (Some(true), None));
        // Ni IAM ni objeto con el que probar: no se sabe, y se dice.
        assert_eq!(veredicto(None, None).0, None);
    }
}
