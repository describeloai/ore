//! **Los schemas de una base** (0038 P6): crear y renombrar, por
//! `ore package schema`.
//!
//! La consola tenía «Create schema» y el doble clic de renombrar desde el
//! principio, y los dos se quedaban en el estado del navegador: un schema
//! creado desaparecía al recargar, porque nunca se había creado. Ahora van al
//! árbol, por el mismo camino que `model` y `copy`: un clon, `ore`, un commit.
//!
//! Lo que cada verbo hace —y por qué renombrar son seis cosas y no una— está
//! en `ore-cli/src/schemas.rs`. Aquí sólo se traduce: la salida de `ore` a su
//! código HTTP (65 → 422, 66 → 404, 73 → 409) y su JSON a la respuesta. La
//! puerta («el árbol no empeora») la pasa `ore`, y deshace si no: en un árbol
//! que es un directorio no hay clon que tirar.
//!
//! - `POST /paquetes/{p}/schemas` `{ name, description?, owner? }` → 201
//! - `POST /paquetes/{p}/schemas/{s}/renombrar` `{ to, since? }` → 200

use crate::mando;
use crate::rutas::{Servidor, analizar, de_node, primera_linea, token};
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::path::Path;

/// El código de `ore package schema` a HTTP.
fn http(codigo: i32) -> u16 {
    match codigo {
        66 => 404,
        73 => 409,
        65 => 422,
        _ => 500,
    }
}

/// Lo que `ore` contestó, como respuesta.
fn responder(salida: Result<mando::Salida, mando::Negado>, bien: u16) -> Respuesta {
    match salida {
        Err(e) => Respuesta::error(500, e.to_string()),
        Ok(s) if !s.bien() => {
            let motivo = primera_linea(&s.stdout, &s.stderr);
            let motivo = motivo
                .strip_prefix("error: ")
                .unwrap_or(&motivo)
                .to_string();
            // Los diagnósticos nuevos, si la puerta dijo que no: cada `OOSxxxx: …`.
            let nuevos: Vec<Json> = s
                .stderr
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with("OOS"))
                .map(Json::s)
                .collect();
            let mut r = Respuesta::error(http(s.codigo), motivo);
            if !nuevos.is_empty()
                && let Json::Obj(m) = &mut r.cuerpo
            {
                m.insert("diagnosticos".into(), Json::Arr(nuevos));
            }
            r
        }
        Ok(s) => match ore_core::parse::parse(&s.stdout) {
            Ok(n) => Respuesta {
                codigo: bien,
                cuerpo: de_node(&n),
            },
            Err(_) => Respuesta::error(500, format!("`ore` no contestó JSON: {}", s.stdout.trim())),
        },
    }
}

impl Servidor {
    /// `POST /paquetes/{p}/schemas`.
    pub(crate) fn crear_schema(&self, raiz: &Path, paquete: &str, cuerpo: &str) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        let cuerpo = match analizar(cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let campo = |k: &str| {
            cuerpo
                .get(k)
                .and_then(|(_, v)| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let Some(nombre) = campo("name") else {
            return Respuesta::error(422, "falta `name`");
        };
        let mut args: Vec<String> = vec![
            "package".into(),
            "schema".into(),
            "new".into(),
            paquete.into(),
            nombre,
            "--json".into(),
            "--path".into(),
            raiz.to_string_lossy().into_owned(),
        ];
        // Un argumento no lleva saltos de línea (`mando::permitido`): una
        // descripción de varias líneas llega en una.
        if let Some(d) = campo("description") {
            let d = d.split_whitespace().collect::<Vec<_>>().join(" ");
            args.extend(["--description".into(), d]);
        }
        if let Some(o) = campo("owner") {
            args.extend(["--owner".into(), o]);
        }
        responder(mando::correr(&self.binario, raiz, &args), 201)
    }

    /// `POST /paquetes/{p}/schemas/{s}/renombrar`.
    pub(crate) fn renombrar_schema(
        &self,
        raiz: &Path,
        paquete: &str,
        viejo: &str,
        cuerpo: &str,
    ) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        let cuerpo = match analizar(cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let campo = |k: &str| {
            cuerpo
                .get(k)
                .and_then(|(_, v)| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let Some(nuevo) = campo("to") else {
            return Respuesta::error(422, "falta `to`: cómo se va a llamar");
        };
        let mut args: Vec<String> = vec![
            "package".into(),
            "schema".into(),
            "rename".into(),
            paquete.into(),
            viejo.into(),
            nuevo,
            "--json".into(),
            "--path".into(),
            raiz.to_string_lossy().into_owned(),
        ];
        if let Some(s) = campo("since") {
            args.extend(["--since".into(), s]);
        }
        responder(mando::correr(&self.binario, raiz, &args), 200)
    }
}
