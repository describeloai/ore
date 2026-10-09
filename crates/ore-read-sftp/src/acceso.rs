//! **¿Se llega a ESTE servidor, es él, y deja leer?** Paso a paso.
//!
//! | permiso | qué se prueba | lo que hace falta |
//! |---|---|---|
//! | `conexion` | el saludo SSH | el puerto abierto a la IP de salida de esta celda |
//! | `huella` | la clave del host frente a la fijada | confirmarla en el alta (D-O4); si cambió, puede ser otro |
//! | `identidad` | entrar con la clave de la celda | su pública en `~/.ssh/authorized_keys` del usuario |
//! | `listar` | un nivel bajo la ruta | permiso de lectura del usuario en ella |
//! | `leer` | un byte del primer fichero | lo mismo, en el fichero |
//!
//! Cada paso sólo se prueba si el anterior pasó: sin la huella confirmada no
//! se autentica (a un servidor que no es no le llega ni la clave). La huella
//! vista sale siempre en `huella`, para que el alta la enseñe y el usuario la
//! confirme. Nada de esto baja un fichero.

use ore_core::json::Json;
use ore_sftp::{Entrada, Fallo, Sftp, Tipo};

struct Paso {
    nombre: &'static str,
    rol: String,
    donde: String,
    ok: Option<bool>,
    porque: Option<String>,
}

/// Lo que el servidor contestó de una ruta, para quien da de alta.
fn explicar(s: &Sftp, f: &Fallo) -> String {
    match f.tipo {
        Tipo::NoEsta => format!(
            "la ruta `/{}` no existe para `{}` (la raíz es la de su usuario: ¿un chroot?)",
            s.fuente.prefijo, s.fuente.usuario
        ),
        _ => f.mensaje.clone(),
    }
}

/// El primer fichero de verdad (no un enlace).
fn primero(fs: &[Entrada]) -> Option<String> {
    fs.iter().find(|e| !e.enlace).map(|e| e.clave.clone())
}

pub fn comprobar(s: &Sftp) -> String {
    let f = &s.fuente;
    let mut pasos: Vec<Paso> = Vec::new();
    let mut prefijos: Vec<String> = Vec::new();
    let mut vista: Option<String> = None;
    let paso = |nombre, rol: String, donde: String| Paso {
        nombre,
        rol,
        donde,
        ok: None,
        porque: Some("no se prueba: falló el paso anterior".into()),
    };
    let mut conexion = paso(
        "conexion",
        format!(
            "el puerto {} abierto a la IP de salida de esta celda",
            f.puerto
        ),
        format!("{}:{}", f.host, f.puerto),
    );
    let mut huella = paso(
        "huella",
        "la huella del host, confirmada en el alta".into(),
        f.host.clone(),
    );
    let mut identidad = paso(
        "identidad",
        "la clave pública de la celda en ~/.ssh/authorized_keys".into(),
        format!("{}@{}", f.usuario, f.host),
    );
    let mut listar = paso(
        "listar",
        "permiso de lectura del usuario".into(),
        format!("/{}", f.prefijo),
    );
    let mut leer = paso(
        "leer",
        "permiso de lectura del usuario".into(),
        format!("/{}", f.prefijo),
    );

    match s.huella_del_host() {
        Err(e) => (conexion.ok, conexion.porque) = (Some(false), Some(e.mensaje)),
        Ok(v) => {
            (conexion.ok, conexion.porque) = (Some(true), None);
            vista = Some(v.clone());
            match &f.huella {
                None => {
                    huella.ok = Some(false);
                    huella.porque = Some(format!(
                        "la URL no fija la huella: la del servidor es `{v}`; si es la suya, \
                         confírmala (`huella={v}`)"
                    ));
                }
                Some(h) if *h != v => {
                    huella.ok = Some(false);
                    huella.porque = Some(format!(
                        "la huella del servidor es `{v}` y la fijada `{h}`: puede ser otro \
                         haciéndose pasar por él; si el cliente la cambió, se vuelve a fijar a propósito"
                    ));
                }
                Some(_) => (huella.ok, huella.porque) = (Some(true), None),
            }
        }
    }
    if huella.ok == Some(true) {
        match s.entrar() {
            Err(e) => (identidad.ok, identidad.porque) = (Some(false), Some(e.mensaje)),
            Ok(()) => (identidad.ok, identidad.porque) = (Some(true), None),
        }
    }
    if identidad.ok == Some(true) {
        match s.nivel(&f.prefijo) {
            Err(e) => (listar.ok, listar.porque) = (Some(false), Some(explicar(s, &e))),
            Ok((carpetas, ficheros)) => {
                (listar.ok, listar.porque) = (Some(true), None);
                let mut uno = primero(&ficheros);
                if uno.is_none()
                    && let Some(c) = carpetas.first()
                {
                    // En este nivel solo hay carpetas: se baja a buscar uno.
                    uno = s.nivel(c).ok().and_then(|(_, fs)| primero(&fs));
                }
                prefijos = carpetas;
                match uno {
                    None => {
                        leer.ok = None;
                        leer.porque =
                            Some("no hay un fichero visible bajo la ruta con el que probar".into());
                    }
                    Some(k) => match s.rango(&k, "0-0", None) {
                        Ok(_) => (leer.ok, leer.porque) = (Some(true), None),
                        Err(e) => (leer.ok, leer.porque) = (Some(false), Some(explicar(s, &e))),
                    },
                }
            }
        }
    }
    pasos.extend([conexion, huella, identidad, listar, leer]);

    let fallo = pasos.iter().find(|p| p.ok == Some(false));
    let ok = fallo.is_none();
    let porque = fallo.and_then(|p| p.porque.clone());
    let permisos = Json::Obj(
        pasos
            .iter()
            .map(|p| {
                let mut c = vec![("donde", Json::s(&p.donde)), ("rol", Json::s(&p.rol))];
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
        ("fija", Json::s("ninguna")),
        ("como", Json::s("probando")),
        ("edad", Json::Int(f.edad as i64)),
        ("permisos", permisos),
        (
            "prefijos",
            Json::Arr(prefijos.iter().map(Json::s).collect()),
        ),
    ];
    if let Some(v) = vista {
        o.push(("huella", Json::s(v)));
    }
    if let Some(p) = porque {
        o.push(("porque", Json::s(p)));
    }
    Json::obj(o).jcs()
}

/// **Qué contiene la ruta**, un nivel: las carpetas, cada una con su URL (sin
/// contraseña). Es lo que el alta ofrece para elegir.
pub fn explorar(s: &Sftp) -> Result<String, String> {
    let f = &s.fuente;
    let (carpetas, ficheros) = s
        .nivel(&f.prefijo)
        .map_err(|e| format!("no se pudo listar: {}", explicar(s, &e)))?;
    let mut contiene: Vec<Json> = carpetas
        .iter()
        .map(|c| {
            Json::obj([
                ("nombre", Json::s(c)),
                ("url", Json::s(ore_sftp::publica(f, c))),
            ])
        })
        .collect();
    let sueltos = ficheros.iter().filter(|e| !e.enlace).count();
    if contiene.is_empty() && sueltos == 0 {
        return Err(format!(
            "`{}` no tiene nada visible para este usuario. Una lista vacía tendría el mismo \
             aspecto que una ruta a la que no se llega, así que se dice",
            ore_sftp::publica(f, &f.prefijo)
        ));
    }
    if sueltos > 0 {
        contiene.push(Json::obj([
            ("nombre", Json::s(&f.prefijo)),
            ("url", Json::s(ore_sftp::publica(f, &f.prefijo))),
            ("objetos", Json::Int(sueltos as i64)),
        ]));
    }
    Ok(Json::obj([("contiene", Json::Arr(contiene))]).pretty())
}
