//! **Los errores tipados del conector v2** (`docs/federation.md` §1.3), y la
//! regla que va con ellos: **ni la URL ni la credencial salen nunca**.
//!
//! Un error de v1 era una frase por stderr, y quien la leía sólo podía
//! mostrarla. La pasarela necesita decidir con ella —reintentar o no, contar
//! un fallo del origen o uno de quien pidió— y eso no se decide leyendo prosa:
//! seis códigos, cada uno con su `reintentable` por defecto.
//!
//! Las líneas JSON se escriben con el mismo `Json` de `ore-core` que el resto
//! del protocolo, en su forma canónica.

use ore_core::json::Json;
use std::collections::BTreeMap;

/// Por qué falló una lectura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codigo {
    /// Un operador o un campo que el conector no sabe expresar. Es un defecto
    /// de quien pidió: sólo debía mandar lo que el conector declaró.
    Operador,
    /// El objeto o una columna no existen en el origen.
    Objeto,
    /// El origen rechaza la credencial.
    Credencial,
    /// Se agotó `timeoutMs`.
    Tiempo,
    /// No se llega al origen.
    Conexion,
    /// El origen falló por su cuenta.
    Origen,
}

impl Codigo {
    pub const TODOS: [Codigo; 6] = [
        Codigo::Operador,
        Codigo::Objeto,
        Codigo::Credencial,
        Codigo::Tiempo,
        Codigo::Conexion,
        Codigo::Origen,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Codigo::Operador => "operador",
            Codigo::Objeto => "objeto",
            Codigo::Credencial => "credencial",
            Codigo::Tiempo => "tiempo",
            Codigo::Conexion => "conexion",
            Codigo::Origen => "origen",
        }
    }

    pub fn de(s: &str) -> Option<Codigo> {
        Codigo::TODOS.into_iter().find(|c| c.as_str() == s)
    }

    /// Si, por defecto, volver a intentarlo puede salir distinto. `origen`
    /// depende del origen: quien lo sepa lo dice con [`Fallo::reintentable`].
    pub fn reintentable(self) -> bool {
        matches!(self, Codigo::Tiempo | Codigo::Conexion)
    }
}

/// Un error del conector: `{"codigo", "mensaje", "reintentable"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallo {
    pub codigo: Codigo,
    pub mensaje: String,
    pub reintentable: bool,
}

impl Fallo {
    pub fn new(codigo: Codigo, mensaje: impl Into<String>) -> Fallo {
        Fallo {
            codigo,
            mensaje: mensaje.into(),
            reintentable: codigo.reintentable(),
        }
    }

    pub fn operador(mensaje: impl Into<String>) -> Fallo {
        Fallo::new(Codigo::Operador, mensaje)
    }

    pub fn origen(mensaje: impl Into<String>) -> Fallo {
        Fallo::new(Codigo::Origen, mensaje)
    }

    /// Cambia el `reintentable` por defecto de su código.
    pub fn reintentable(mut self, si: bool) -> Fallo {
        self.reintentable = si;
        self
    }

    /// El mensaje, sin nada de `url` ([`tapar`]). Todo fallo que sale de un
    /// conector pasa por aquí.
    pub fn tapado(mut self, url: &str) -> Fallo {
        self.mensaje = tapar(&self.mensaje, url);
        self
    }

    pub fn campos(&self) -> BTreeMap<String, Json> {
        let mut o = BTreeMap::new();
        o.insert("codigo".to_string(), Json::s(self.codigo.as_str()));
        o.insert("mensaje".to_string(), Json::s(self.mensaje.as_str()));
        o.insert("reintentable".to_string(), Json::Bool(self.reintentable));
        o
    }

    /// La línea JSON, sin salto.
    pub fn linea(&self) -> String {
        Json::Obj(self.campos()).jcs()
    }

    /// Lee los campos de un fallo de un objeto JSON.
    pub fn de_nodo(n: &ore_core::parse::Node) -> Option<Fallo> {
        let codigo = Codigo::de(n.get("codigo")?.1.as_str()?)?;
        let mensaje = n.get("mensaje").and_then(|(_, m)| m.as_str()).unwrap_or("");
        let reintentable = n
            .get("reintentable")
            .and_then(|(_, r)| r.as_str())
            .map(|r| r == "true")
            .unwrap_or_else(|| codigo.reintentable());
        Some(Fallo {
            codigo,
            mensaje: mensaje.to_string(),
            reintentable,
        })
    }
}

/// Lo que todavía no tiene código —los `String` de v1— es del origen.
impl From<String> for Fallo {
    fn from(mensaje: String) -> Fallo {
        Fallo::origen(mensaje)
    }
}

impl std::fmt::Display for Fallo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.codigo.as_str(), self.mensaje)
    }
}

/// Los parámetros de una URL cuyo valor es secreto: la credencial de S3 viaja
/// en la consulta (`ore-sigv4::fuente`), y cualquier otra que se le parezca
/// se tapa igual.
fn es_secreto(clave: &str) -> bool {
    let k = clave.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "passwd",
        "pwd",
        "key",
        "credential",
        "sig",
    ]
    .iter()
    .any(|s| k.contains(s))
}

/// `%XX` → el byte, para encontrar también la forma descodificada de un
/// secreto: un mensaje de error puede llevar cualquiera de las dos.
fn descodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Los trozos de `url` que no pueden salir: la URL entera, la contraseña de
/// su `usuario:contraseña@` y el valor de cada parámetro secreto, en su forma
/// escrita y en la descodificada. Los cortos no cuentan: tapar `a` taparía
/// media frase sin proteger nada.
pub fn secretos(url: &str) -> Vec<String> {
    let mut s: Vec<String> = Vec::new();
    if url.is_empty() {
        return s;
    }
    s.push(url.to_string());
    if let Some((_, resto)) = url.split_once("://") {
        let autoridad = resto.split(['/', '?', '#']).next().unwrap_or("");
        if let Some((info, _)) = autoridad.rsplit_once('@')
            && let Some((_, clave)) = info.split_once(':')
        {
            s.push(clave.to_string());
            s.push(descodificar(clave));
        }
    }
    if let Some((_, consulta)) = url.split_once('?') {
        for par in consulta.split('&') {
            if let Some((k, v)) = par.split_once('=')
                && es_secreto(&descodificar(k))
            {
                s.push(v.to_string());
                s.push(descodificar(v));
            }
        }
    }
    s.retain(|x| x.chars().count() >= 4);
    // Los largos primero: la URL entera antes que la contraseña que contiene.
    s.sort_by_key(|x| std::cmp::Reverse(x.len()));
    s.dedup();
    s
}

/// **Un texto sin nada de `url`**, y sin ninguna contraseña de ninguna otra
/// URL que lleve dentro (`esquema://usuario:contraseña@`).
///
/// La regla de `docs/federation.md` §1.3 —ni la URL ni la credencial salen por
/// stdout, stderr ni un error— escrita una vez: un mensaje del origen puede
/// repetir la cadena de conexión (`libpq` lo hace con algunos errores), y el
/// conector no siempre sabe cuándo.
pub fn tapar(texto: &str, url: &str) -> String {
    let mut t = texto.to_string();
    for s in secretos(url) {
        let sustituto = if s == url { "<url>" } else { "***" };
        t = t.replace(&s, sustituto);
    }
    // Y cualquier `://usuario:contraseña@` que quede, sea de quien sea.
    let mut out = String::with_capacity(t.len());
    let mut resto = t.as_str();
    while let Some(i) = resto.find("://") {
        let (antes, despues) = resto.split_at(i + 3);
        out.push_str(antes);
        let fin = despues
            .find(|c: char| c.is_whitespace() || "/?#\"'`".contains(c))
            .unwrap_or(despues.len());
        let autoridad = &despues[..fin];
        match autoridad.rsplit_once('@') {
            Some((info, host)) if info.contains(':') => {
                let usuario = info.split(':').next().unwrap_or("");
                out.push_str(&format!("{usuario}:***@{host}"));
            }
            _ => out.push_str(autoridad),
        }
        resto = &despues[fin..];
    }
    out.push_str(resto);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **La URL de Postgres no sale, ni entera ni su contraseña suelta.**
    #[test]
    fn la_url_y_su_contrasena_se_tapan() {
        let url = "postgres://ore:s3cr%2Fto@ep-1.neon.tech/db?sslmode=require";
        let t = tapar(
            &format!("no se pudo conectar a {url}: la clave s3cr/to no vale (s3cr%2Fto)"),
            url,
        );
        assert!(!t.contains("s3cr"), "{t}");
        assert!(t.contains("<url>"), "{t}");
    }

    /// **La credencial de S3 viaja en la consulta**: sus valores se tapan, y
    /// lo que no es secreto (`region`) se queda.
    #[test]
    fn los_parametros_secretos_se_tapan() {
        let url = "s3://b/p?region=eu-west-1&access_key_id=AKIAEJEMPLO1&secret_access_key=ab%2Bcd%2Fef&session_token=TOKENLARGO";
        let t = tapar(
            "403 para AKIAEJEMPLO1 con ab+cd/ef y TOKENLARGO en eu-west-1",
            url,
        );
        for s in ["AKIAEJEMPLO1", "ab+cd/ef", "TOKENLARGO"] {
            assert!(!t.contains(s), "{s} en {t}");
        }
        assert!(t.contains("eu-west-1"), "{t}");
    }

    /// **Y la de otra URL que el origen repita**, aunque no sea la pedida.
    #[test]
    fn cualquier_usuario_y_contrasena_se_tapa() {
        let t = tapar(
            "redirigido a postgresql://admin:otra@h:5432/x y a https://h/y",
            "",
        );
        assert_eq!(
            t,
            "redirigido a postgresql://admin:***@h:5432/x y a https://h/y"
        );
    }

    /// El fallo sale como una línea JSON, y se vuelve a leer igual.
    #[test]
    fn el_fallo_ida_y_vuelta() {
        let f = Fallo::new(Codigo::Tiempo, "se agotaron 30000 ms");
        assert!(f.reintentable);
        let l = f.linea();
        assert_eq!(
            l,
            r#"{"codigo":"tiempo","mensaje":"se agotaron 30000 ms","reintentable":true}"#
        );
        let n = ore_core::parse::parse(&l).expect("json");
        assert_eq!(Fallo::de_nodo(&n), Some(f));
        assert!(!Fallo::operador("x").reintentable);
        assert!(Fallo::origen("x").reintentable(true).reintentable);
    }
}
