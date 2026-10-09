//! **¿Puede ESTA app leer ESTA biblioteca?** Se prueba, paso a paso, y cada
//! paso sólo si pasó el anterior: lo que falla se dice con su arreglo.
//!
//! | permiso | qué se prueba | lo que se concede | sobre |
//! |---|---|---|---|
//! | `identidad` | el canje en Entra: el token de Google de la celda por uno de Graph | una *federated identity credential* para la cuenta de la celda | la app del cliente |
//! | `sitio` | el sitio por su ruta | `Sites.Selected` (con consentimiento de un administrador) **y** la concesión `read` en ese sitio | el sitio |
//! | `biblioteca` | la biblioteca por su nombre | — | el sitio |
//! | `listar` | la carpeta del prefijo | (la concesión `read`) | la biblioteca |
//! | `leer` | un byte de hasta cinco ficheros | (la concesión `read`) | la biblioteca |
//! | `versiones` | las versiones de uno | (la concesión `read`) | la biblioteca |
//!
//! Graph no dice qué le falta a un `403`: que la app no tenga `Sites.Selected`
//! o que no tenga la concesión del sitio dan lo mismo, y se dicen las dos con
//! el comando que las arregla. Un sitio, una biblioteca o una carpeta que no
//! existe no es un permiso que falte, y se dice como lo que es.

use ore_core::json::Json;
use ore_graph::{Clase, Fallo, Graph, Item};

/// Los motivos que no son un permiso: se dicen tal cual.
const NO_ESTA: &str = "no existe";
/// Cuántos ficheros se prueban a leer, y cuántas carpetas se miran buscándolos.
const A_PROBAR: usize = 5;
const CARPETAS: usize = 10;

struct Permiso {
    nombre: &'static str,
    rol: String,
    sobre: String,
    ok: Option<bool>,
    porque: Option<String>,
}

fn web(g: &Graph) -> String {
    let f = &g.fuente;
    if f.sitio.is_empty() {
        format!("https://{}", f.host)
    } else {
        format!("https://{}/{}", f.host, f.sitio)
    }
}

/// Lo que concede un administrador del cliente para que la app lea el sitio.
fn concesion(g: &Graph) -> String {
    format!(
        "la app `{}` necesita el permiso de aplicación `Sites.Selected` de Microsoft Graph, con el \
         consentimiento de un administrador, y la concesión `read` sobre el sitio, que da un \
         administrador de SharePoint: `Grant-PnPEntraIDAppSitePermission -AppId {} -DisplayName \
         ORE -Site {} -Permissions Read` (o `POST /sites/{{id}}/permissions` con `roles: [\"read\"]`)",
        g.fuente.cliente,
        g.fuente.cliente,
        web(g)
    )
}

/// Lo que Graph contestó, dicho para quien da de alta.
fn explicar(g: &Graph, f: &Fallo, donde: &str) -> String {
    match (f.estado, donde) {
        (404, "sitio") => format!(
            "el sitio `{}` {NO_ESTA} (404): ¿está bien escrito? (`sites/<nombre>`, `teams/<nombre>`, \
             o nada para el sitio raíz)",
            web(g)
        ),
        (404, "listar") => format!(
            "la carpeta `{}` {NO_ESTA} en la biblioteca `{}` (404)",
            g.fuente.prefijo, g.fuente.biblioteca
        ),
        (404, _) if f.codigo == "bibliotecaNoEsta" => f.cuerpo.clone(),
        (403, _) => format!("{} ({})", concesion(g), f.motivo()),
        (401, _) => format!(
            "Graph no acepta el token de la app `{}` ({}): ¿tiene el permiso `Sites.Selected` \
             concedido por un administrador?",
            g.fuente.cliente,
            f.motivo()
        ),
        _ => f.motivo(),
    }
}

fn especial(m: &str) -> bool {
    m.contains(&format!(" {NO_ESTA} ")) || m.starts_with("no hay una biblioteca")
}

/// Hasta [`A_PROBAR`] ficheros bajo el prefijo, mirando como mucho
/// [`CARPETAS`] carpetas (la primera, la del prefijo), y sus subcarpetas
/// directas (lo que `explorar` enseña).
fn buscar(g: &Graph) -> Result<(Vec<Item>, Vec<String>), Fallo> {
    let mut pendientes = vec![g.fuente.prefijo.clone()];
    let mut ficheros = Vec::new();
    let mut carpetas = Vec::new();
    let mut mirado = 0;
    while let Some(c) = pendientes.pop() {
        let hijos = g.hijos(&c)?;
        for i in hijos {
            match i.clase {
                Clase::Carpeta => {
                    if mirado == 0 {
                        carpetas.push(format!("{}/", i.ruta));
                    }
                    pendientes.insert(0, i.ruta);
                }
                Clase::Fichero if ficheros.len() < A_PROBAR => ficheros.push(i),
                _ => {}
            }
        }
        mirado += 1;
        if ficheros.len() >= A_PROBAR || mirado >= CARPETAS {
            break;
        }
    }
    Ok((ficheros, carpetas))
}

pub fn comprobar(g: &Graph) -> String {
    let f = &g.fuente;
    let mut permisos = Vec::new();
    let mut carpetas: Vec<String> = Vec::new();
    let biblioteca = format!("{}/{}", web(g), f.biblioteca);
    let lector = "la concesión read (Sites.Selected)".to_string();

    let identidad = g.token();
    permisos.push(Permiso {
        nombre: "identidad",
        rol: "federated identity credential (issuer https://accounts.google.com, subject = la \
              cuenta de esta celda)"
            .into(),
        sobre: format!("app {} del tenant {}", f.cliente, f.tenant),
        ok: Some(identidad.is_ok()),
        porque: identidad.as_ref().err().cloned(),
    });

    let sitio = identidad.is_ok().then(|| g.sitio());
    if let Some(s) = &sitio {
        permisos.push(Permiso {
            nombre: "sitio",
            rol: "Sites.Selected y la concesión read".into(),
            sobre: web(g),
            ok: Some(s.is_ok()),
            porque: s.as_ref().err().map(|e| explicar(g, e, "sitio")),
        });
    }

    if let Some(Ok(_)) = &sitio {
        let b = g.biblioteca();
        permisos.push(Permiso {
            nombre: "biblioteca",
            rol: "la biblioteca, por su nombre".into(),
            sobre: biblioteca.clone(),
            ok: Some(b.is_ok()),
            porque: b.as_ref().err().map(|e| explicar(g, e, "biblioteca")),
        });

        if b.is_ok() {
            let busqueda = buscar(g);
            permisos.push(Permiso {
                nombre: "listar",
                rol: lector.clone(),
                sobre: biblioteca.clone(),
                ok: Some(busqueda.is_ok()),
                porque: busqueda.as_ref().err().map(|e| explicar(g, e, "listar")),
            });

            if let Ok((ficheros, cs)) = busqueda {
                carpetas = cs;
                let (ok, porque, uno) = leer_alguno(g, &ficheros);
                permisos.push(Permiso {
                    nombre: "leer",
                    rol: lector.clone(),
                    sobre: biblioteca.clone(),
                    ok,
                    porque,
                });
                let (ok, porque) = match uno {
                    None => (
                        None,
                        Some("sin un fichero leído no hay con qué probar".into()),
                    ),
                    Some(i) => match g.versiones(&i.id) {
                        Ok(v) if !v.is_empty() => (Some(true), None),
                        Ok(_) => (
                            Some(false),
                            Some(format!("SharePoint no da ninguna versión de `{}`", i.ruta)),
                        ),
                        Err(e) => (Some(false), Some(explicar(g, &e, "versiones"))),
                    },
                };
                permisos.push(Permiso {
                    nombre: "versiones",
                    rol: lector.clone(),
                    sobre: biblioteca.clone(),
                    ok,
                    porque,
                });
            }
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

/// Un byte de cada fichero hasta que uno se lea: pasa si uno se deja, y dice
/// cuáles no.
fn leer_alguno(g: &Graph, ficheros: &[Item]) -> (Option<bool>, Option<String>, Option<Item>) {
    if ficheros.is_empty() {
        return (
            None,
            Some("no hay un fichero visible bajo el prefijo con el que probar".into()),
            None,
        );
    }
    let mut no = Vec::new();
    for i in ficheros {
        let leido = g.bajar(&i.id, None, Some("bytes=0-0")).and_then(|mut l| {
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut l.lector, &mut b)
                .map(|_| ())
                .map_err(|e| Fallo {
                    estado: 0,
                    codigo: String::new(),
                    cuerpo: e.to_string(),
                })
        });
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

/// **Qué hay**: las carpetas bajo el prefijo, cada una con su URL, los
/// ficheros sueltos, y las bibliotecas del sitio (otra biblioteca es otra URL).
pub fn explorar(g: &Graph) -> Result<String, String> {
    let f = &g.fuente;
    let (_, drives) = g
        .sitio()
        .map_err(|e| format!("no se pudo mirar el sitio: {}", explicar(g, &e, "sitio")))?;
    let hijos = g
        .hijos(&f.prefijo)
        .map_err(|e| format!("no se pudo listar: {}", explicar(g, &e, "listar")))?;
    let mut contiene: Vec<Json> = hijos
        .iter()
        .filter(|i| i.clase == Clase::Carpeta)
        .map(|i| {
            let p = format!("{}/", i.ruta);
            Json::obj([
                ("nombre", Json::s(&p)),
                ("url", Json::s(ore_graph::publica(f, &p))),
            ])
        })
        .collect();
    let sueltos = hijos.iter().filter(|i| i.clase == Clase::Fichero).count();
    let saltados = hijos
        .iter()
        .filter(|i| matches!(i.clase, Clase::Cuaderno | Clase::Acceso))
        .count();
    if sueltos > 0 {
        contiene.push(Json::obj([
            ("nombre", Json::s(&f.prefijo)),
            ("url", Json::s(ore_graph::publica(f, &f.prefijo))),
            ("objetos", Json::Int(sueltos as i64)),
        ]));
    }
    let bibliotecas = drives
        .iter()
        .map(|(n, _)| {
            let otra = ore_graph::Fuente {
                biblioteca: n.clone(),
                prefijo: String::new(),
                ..f.clone()
            };
            Json::obj([
                ("nombre", Json::s(n)),
                ("url", Json::s(ore_graph::publica(&otra, ""))),
            ])
        })
        .collect();
    if contiene.is_empty() && saltados == 0 {
        return Err(format!(
            "`{}` no tiene nada visible con esta identidad. Una lista vacía tendría el mismo \
             aspecto que una biblioteca a la que no se llega, así que se dice",
            ore_graph::publica(f, &f.prefijo)
        ));
    }
    let mut o = vec![
        ("contiene", Json::Arr(contiene)),
        ("bibliotecas", Json::Arr(bibliotecas)),
    ];
    if saltados > 0 {
        o.push((
            "saltados",
            Json::s(format!(
                "{saltados} cuadernos de OneNote o accesos directos: no son ficheros de esta biblioteca"
            )),
        ));
    }
    Ok(Json::obj(o).pretty())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn lo_que_no_es_un_permiso_se_dice_como_lo_que_es() {
        let g = Graph::de_url(
            "sharepoint://contoso.sharepoint.com/sites/Finanzas/Documentos/2026?tenant=t&cliente=app",
        )
        .unwrap();
        let f = |estado, codigo: &str, cuerpo: &str| Fallo {
            estado,
            codigo: codigo.into(),
            cuerpo: cuerpo.into(),
        };
        let m = explicar(&g, &f(404, "itemNotFound", "{}"), "sitio");
        assert!(especial(&m) && m.contains("sites/Finanzas"), "{m}");
        let m = explicar(&g, &f(404, "itemNotFound", "{}"), "listar");
        assert!(especial(&m) && m.contains("`2026/`"), "{m}");
        let m = explicar(
            &g,
            &f(
                404,
                "bibliotecaNoEsta",
                "no hay una biblioteca `X` en el sitio; hay: `Documentos`",
            ),
            "biblioteca",
        );
        assert!(especial(&m), "{m}");
        let m = explicar(
            &g,
            &f(
                403,
                "accessDenied",
                r#"{"error":{"code":"accessDenied","message":"Access denied"}}"#,
            ),
            "sitio",
        );
        assert!(
            !especial(&m)
                && m.contains("Sites.Selected")
                && m.contains("Grant-PnPEntraIDAppSitePermission -AppId app")
                && m.contains("-Site https://contoso.sharepoint.com/sites/Finanzas"),
            "{m}"
        );
    }
}
