//! **¿Puede ESTA cuenta leer ESTA carpeta?** Se prueba, paso a paso, y cada
//! paso sólo si pasó el anterior: lo que falla se dice con su arreglo.
//!
//! | permiso | qué se prueba | lo que se concede | sobre |
//! |---|---|---|---|
//! | `identidad` | el token de Drive: la cuenta de la celda se suplanta a sí misma con `drive.readonly` | `roles/iam.serviceAccountTokenCreator` sobre sí misma | la cuenta |
//! | `carpeta` | la carpeta por su ID | compartirla con la cuenta, como lector | la carpeta |
//! | `listar` | los hijos del prefijo | (la misma) | la carpeta |
//! | `leer` | un byte de hasta cinco ficheros | (la misma) | la carpeta |
//! | `versiones` | que un fichero dé su revisión y su `md5` | (la misma) | la carpeta |
//!
//! Drive contesta `404 File not found` igual a una carpeta que no existe y a una
//! que no se ha compartido con la cuenta: no se puede distinguir, y se dicen las
//! dos cosas. Una API de Drive apagada en el proyecto (`accessNotConfigured`) no
//! es un permiso de la carpeta, y se dice como lo que es.

use ore_base::json::Json;
use ore_gdrive::{Clase, Fallo, Gdrive, Item};

/// Cuántos ficheros se prueban a leer.
const A_PROBAR: usize = 5;

struct Permiso {
    nombre: &'static str,
    rol: String,
    sobre: String,
    ok: Option<bool>,
    porque: Option<String>,
}

fn cuenta(g: &Gdrive) -> String {
    g.cuenta()
        .unwrap_or_else(|_| "la cuenta de esta celda".into())
}

/// Lo que Drive contestó, dicho para quien da de alta.
fn explicar(g: &Gdrive, f: &Fallo, donde: &str) -> String {
    if f.motivo_api == "accessNotConfigured" || f.cuerpo.contains("has not been used in project") {
        return format!(
            "la API de Google Drive no está activada en el proyecto de la cuenta `{}` ({}): `gcloud services enable drive.googleapis.com`",
            cuenta(g),
            f.motivo()
        );
    }
    match (f.estado, donde) {
        (404, "carpeta") => format!(
            "Drive no encuentra la carpeta `{}` (404): o no existe, o no está compartida con `{}` — Drive contesta igual a las dos. Compártela con ese correo como lector",
            g.fuente.raiz,
            cuenta(g)
        ),
        (404, "listar") => format!(
            "la carpeta `{}` no está en la carpeta compartida (404)",
            g.fuente.prefijo
        ),
        (403 | 401, _) => format!(
            "la cuenta `{}` no puede leer `{}`: compártela como lector ({})",
            cuenta(g),
            g.fuente.raiz,
            f.motivo()
        ),
        _ => f.motivo(),
    }
}

/// Los motivos que no son un permiso: se dicen tal cual.
fn especial(m: &str) -> bool {
    m.contains("no encuentra la carpeta")
        || m.contains("no está en la carpeta")
        || m.contains("no está activada")
}

fn drive(g: &Gdrive) -> String {
    format!("Drive:{}/{}", g.fuente.raiz, g.fuente.prefijo)
}

pub fn comprobar(g: &Gdrive) -> String {
    let mut permisos = Vec::new();
    let mut carpetas: Vec<String> = Vec::new();
    let donde = drive(g);
    let lector = "lector de la carpeta compartida".to_string();

    let identidad = g.token();
    permisos.push(Permiso {
        nombre: "identidad",
        rol: "roles/iam.serviceAccountTokenCreator sobre la propia cuenta (se suplanta a sí misma con \
              drive.readonly)"
            .into(),
        sobre: cuenta(g),
        ok: Some(identidad.is_ok()),
        porque: identidad.as_ref().err().cloned(),
    });

    let raiz = identidad.is_ok().then(|| g.raiz());
    if let Some(r) = &raiz {
        let porque = match r {
            Err(e) => Some(explicar(g, e, "carpeta")),
            Ok(i) if i.clase != Clase::Carpeta => Some(format!(
                "`{}` no es una carpeta sino un fichero `{}` ({}): la fuente es una carpeta",
                g.fuente.raiz, i.nombre, i.tipo
            )),
            Ok(_) => None,
        };
        permisos.push(Permiso {
            nombre: "carpeta",
            rol: lector.clone(),
            sobre: donde.clone(),
            ok: Some(porque.is_none()),
            porque,
        });
    }

    if permisos
        .last()
        .is_some_and(|p| p.nombre == "carpeta" && p.ok == Some(true))
    {
        let busqueda = buscar(g);
        permisos.push(Permiso {
            nombre: "listar",
            rol: lector.clone(),
            sobre: donde.clone(),
            ok: Some(busqueda.is_ok()),
            porque: busqueda.as_ref().err().map(|e| explicar(g, e, "listar")),
        });

        if let Ok((ficheros, cs)) = busqueda {
            carpetas = cs;
            let (ok, porque, uno) = leer_alguno(g, &ficheros);
            permisos.push(Permiso {
                nombre: "leer",
                rol: lector.clone(),
                sobre: donde.clone(),
                ok,
                porque,
            });
            let (ok, porque) = match uno {
                None => (
                    None,
                    Some("sin un fichero leído no hay con qué probar".into()),
                ),
                Some(i) if matches!(i.clase, Clase::Nativo(_)) => (Some(true), None),
                Some(i) if i.revision.is_some() && i.md5.is_some() => (Some(true), None),
                Some(i) => (
                    Some(false),
                    Some(format!("Drive no da la revisión o el md5 de `{}`", i.ruta)),
                ),
            };
            permisos.push(Permiso {
                nombre: "versiones",
                rol: lector.clone(),
                sobre: donde.clone(),
                ok,
                porque,
            });
        }
    }

    let fallo = permisos.iter().find(|p| p.ok == Some(false));
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
        ("fija", Json::s("version")),
        ("como", Json::s("probando")),
        ("permisos", permisos),
        (
            "prefijos",
            Json::Arr(carpetas.iter().map(Json::s).collect()),
        ),
    ];
    if let Some(p) = porque {
        o.push(("porque", Json::s(p)));
    }
    Json::obj(o).jcs()
}

/// Hasta [`A_PROBAR`] ficheros en la carpeta del prefijo, y sus subcarpetas
/// directas (lo que `explorar` enseña).
fn buscar(g: &Gdrive) -> Result<(Vec<Item>, Vec<String>), Fallo> {
    let id = g.carpeta_de(&g.fuente.prefijo)?;
    let mut ficheros = Vec::new();
    let mut carpetas = Vec::new();
    for h in g.hijos(&id, &g.fuente.prefijo)? {
        match h.clase {
            Clase::Carpeta => carpetas.push(format!("{}/", h.ruta)),
            Clase::Fichero | Clase::Nativo(_) if ficheros.len() < A_PROBAR => ficheros.push(h),
            _ => {}
        }
    }
    Ok((ficheros, carpetas))
}

/// Un byte de cada fichero hasta que uno se lea: pasa si uno se deja, y dice
/// cuáles no.
fn leer_alguno(g: &Gdrive, ficheros: &[Item]) -> (Option<bool>, Option<String>, Option<Item>) {
    if ficheros.is_empty() {
        return (
            None,
            Some("no hay un fichero visible bajo el prefijo con el que probar".into()),
            None,
        );
    }
    let mut no = Vec::new();
    for i in ficheros {
        // Un nativo se prueba exportándolo (X0 D1): no tiene revisión que pedir.
        let leido = match i.clase {
            Clase::Nativo(_) => g.exportar(i).map(|_| ()),
            _ => g
                .bajar(&i.id, i.revision.as_deref(), Some("bytes=0-0"))
                .and_then(|mut l| {
                    let mut b = Vec::new();
                    std::io::Read::read_to_end(&mut l.lector, &mut b)
                        .map(|_| ())
                        .map_err(|e| Fallo::propio(e.to_string()))
                }),
        };
        match leido {
            Ok(()) => {
                let porque =
                    (!no.is_empty()).then(|| format!("no se dejaron leer: {}", no.join("; ")));
                return (Some(true), porque, Some(i.clone()));
            }
            Err(e) => no.push(format!("`{}`: {}", i.ruta, explicar(g, &e, "leer"))),
        }
    }
    (Some(false), Some(no.join("; ")), None)
}

/// **Qué hay**: las carpetas bajo el prefijo, cada una con su URL, y cuántos
/// ficheros sueltos hay.
pub fn explorar(g: &Gdrive) -> Result<String, String> {
    let f = &g.fuente;
    let id = g
        .carpeta_de(&f.prefijo)
        .map_err(|e| format!("no se pudo listar: {}", explicar(g, &e, "listar")))?;
    let hijos = g
        .hijos(&id, &f.prefijo)
        .map_err(|e| format!("no se pudo listar: {}", explicar(g, &e, "listar")))?;
    let mut contiene: Vec<Json> = hijos
        .iter()
        .filter(|i| i.clase == Clase::Carpeta)
        .map(|i| {
            let p = format!("{}/", i.ruta);
            Json::obj([
                ("nombre", Json::s(&p)),
                ("url", Json::s(ore_gdrive::publica(f, &p))),
            ])
        })
        .collect();
    let sueltos = hijos
        .iter()
        .filter(|i| matches!(i.clase, Clase::Fichero | Clase::Nativo(_)))
        .count();
    let saltados = hijos
        .iter()
        .filter(|i| matches!(i.clase, Clase::Acceso | Clase::Otro))
        .count();
    if sueltos > 0 {
        contiene.push(Json::obj([
            ("nombre", Json::s(&f.prefijo)),
            ("url", Json::s(ore_gdrive::publica(f, &f.prefijo))),
            ("objetos", Json::Int(sueltos as i64)),
        ]));
    }
    if contiene.is_empty() && saltados == 0 {
        return Err(format!(
            "`{}` no tiene nada visible con esta identidad. Una lista vacía tendría el mismo \
             aspecto que una carpeta a la que no se llega, así que se dice",
            ore_gdrive::publica(f, &f.prefijo)
        ));
    }
    let mut o = vec![
        ("contiene", Json::Arr(contiene)),
        ("bibliotecas", Json::Arr(Vec::new())),
    ];
    if saltados > 0 {
        o.push((
            "saltados",
            Json::s(format!(
                "{saltados} accesos directos u otros archivos de Google (formularios, mapas…): no son ficheros que ORE lea"
            )),
        ));
    }
    Ok(Json::obj(o).pretty())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn g() -> Gdrive {
        // Sin red: `cuenta()` cae al mensaje genérico si no hay variable.
        Gdrive::de_url("gdrive://CARPETA/2026").unwrap()
    }

    fn f(estado: u16, motivo_api: &str, cuerpo: &str) -> Fallo {
        Fallo::nuevo(estado, motivo_api, cuerpo)
    }

    #[test]
    fn lo_que_no_es_un_permiso_se_dice_como_lo_que_es() {
        let g = g();
        let m = explicar(
            &g,
            &f(404, "notFound", "File not found: CARPETA"),
            "carpeta",
        );
        assert!(especial(&m) && m.contains("o no está compartida"), "{m}");
        let m = explicar(&g, &f(404, "notFound", "{}"), "listar");
        assert!(especial(&m) && m.contains("`2026/`"), "{m}");
        let m = explicar(
            &g,
            &f(
                403,
                "accessNotConfigured",
                "Google Drive API has not been used in project 1 before",
            ),
            "carpeta",
        );
        assert!(
            especial(&m) && m.contains("gcloud services enable drive.googleapis.com"),
            "{m}"
        );
        let m = explicar(&g, &f(403, "forbidden", "{}"), "leer");
        assert!(!especial(&m) && m.contains("compártela como lector"), "{m}");
    }
}
