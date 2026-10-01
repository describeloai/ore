//! **Lo que se escribe, a la actividad de la organización** (0047 A6.3).
//!
//! `ore-serve` escribe por 56 rutas (M1) y, hasta A6, sólo tres lo contaban (las
//! de A5). Esto lo cuenta desde un sitio, tras atender (`recuento::atendiendo`):
//! toda petición que escribe, con sujeto, y que la ruta no contó ya, deja un
//! `hizo` en el buzón. La respuesta no lo espera.
//!
//! - `2xx` es `hecho`, con el `commit` de la respuesta si lo trae (H13: se apunta,
//!   no se copia).
//! - `423` (la rama protegida) es `negado`, y también un `403` que el módulo dijo
//!   por su cuenta. El `403` de una potestad lo anota `ore-iam` al decidir
//!   (`acceso:negado`), y no se repite.
//! - Lo demás (`4xx` de forma, `5xx`) no es un acto: no se cuenta.
//!
//! ⛔ **Fuera, lo de los datos** (H12, A8): ejecutar en un puesto, su SQL, su LSP y
//!   sus salidas, y ejecutar una vista. Son las rutas que leen o computan datos, y
//!   registrarlas es de la organización que lo encienda.
//!
//! ⭐ **La operación se declara aquí, ruta a ruta, y una prueba exige que toda ruta
//!   que escribe tenga la suya** (M1 § 7: lo que se declara aparte de la ruta se
//!   desincroniza, y sin la prueba una ruta nueva escribiría sin dejar rastro).

use crate::rutas::Servidor;
use ore_entrada::http::{Peticion, Respuesta, Salida};

/// Qué es una ruta que escribe, para la actividad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Es {
    /// Un acto de la organización, con el nombre de su operación.
    Acto(&'static str),
    /// Leer o computar datos: no se cuenta (A8).
    Datos,
}

/// `(método, patrón, qué es)`. En el patrón, `{}` es un segmento cualquiera y `**`
/// cero o más (un camino dentro del árbol, una rama con `/`, el `prefix` de
/// Iceberg). Se busca en orden: el primero que casa.
pub const ESCRITURAS: &[(&str, &str, Es)] = &[
    // la celda: fuentes, modelos, copias
    (
        "POST",
        "/fuentes/comprobaciones",
        Es::Acto("fuente:comprobar"),
    ),
    (
        "POST",
        "/fuentes/{}/catalogar",
        Es::Acto("fuente:catalogar"),
    ),
    ("POST", "/fuentes", Es::Acto("fuente:crear")),
    ("DELETE", "/fuentes/{}", Es::Acto("fuente:retirar")),
    ("POST", "/modelos", Es::Acto("modelo:crear")),
    ("DELETE", "/modelos/{}", Es::Acto("modelo:retirar")),
    (
        "POST",
        "/paquetes/{}/tablas/{}/copiar",
        Es::Acto("copia:crear"),
    ),
    (
        "POST",
        "/paquetes/{}/copia/rehacer",
        Es::Acto("copia:rehacer"),
    ),
    ("POST", "/paquetes/{}/copia", Es::Acto("copia:crear")),
    ("POST", "/paquetes/{}/decisiones", Es::Acto("copia:decidir")),
    (
        "POST",
        "/datasets/{}/{}/confirmar",
        Es::Acto("dataset:confirmar"),
    ),
    (
        "POST",
        "/datasets/{}/{}/{}/confirmar",
        Es::Acto("dataset:confirmar"),
    ),
    // el árbol
    ("POST", "/paquetes", Es::Acto("base:crear")),
    ("DELETE", "/paquetes/{}", Es::Acto("base:retirar")),
    ("POST", "/paquetes/{}/schemas", Es::Acto("schema:crear")),
    (
        "POST",
        "/paquetes/{}/schemas/{}/renombrar",
        Es::Acto("schema:renombrar"),
    ),
    (
        "POST",
        "/paquetes/{}/tablas/{}/modelar",
        Es::Acto("tabla:modelar"),
    ),
    ("POST", "/arbol/commit", Es::Acto("arbol:escribir")),
    ("PUT", "/arbol/**", Es::Acto("arbol:escribir")),
    ("DELETE", "/arbol/**", Es::Acto("arbol:borrar")),
    ("PUT", "/documentos/**", Es::Acto("documento:escribir")),
    ("DELETE", "/documentos/**", Es::Acto("documento:borrar")),
    ("POST", "/proyectos", Es::Acto("proyecto:crear")),
    ("PUT", "/proyectos/{}", Es::Acto("proyecto:editar")),
    ("DELETE", "/proyectos/{}", Es::Acto("proyecto:retirar")),
    ("POST", "/repositorios", Es::Acto("repositorio:crear")),
    (
        "POST",
        "/repositorios/**/actualizar",
        Es::Acto("repositorio:actualizar"),
    ),
    ("PUT", "/repositorios/**", Es::Acto("repositorio:escribir")),
    // el catálogo Iceberg (`/v1`, con `prefix` o sin él): crear y cambiar tablas
    ("POST", "/v1/**/namespaces", Es::Acto("schema:crear")),
    (
        "POST",
        "/v1/**/namespaces/{}/tables",
        Es::Acto("tabla:crear"),
    ),
    (
        "POST",
        "/v1/**/namespaces/{}/tables/{}",
        Es::Acto("tabla:actualizar"),
    ),
    (
        "POST",
        "/v1/**/transactions/commit",
        Es::Acto("tabla:actualizar"),
    ),
    // ramas y propuestas
    ("POST", "/ramas", Es::Acto("rama:crear")),
    ("PUT", "/ramas/**/proteccion", Es::Acto("rama:proteger")),
    ("POST", "/ramas/**/fusionar", Es::Acto("rama:fusionar")),
    ("DELETE", "/ramas/**", Es::Acto("rama:retirar")),
    ("POST", "/propuestas", Es::Acto("propuesta:abrir")),
    (
        "POST",
        "/propuestas/{}/revisar",
        Es::Acto("propuesta:revisar"),
    ),
    (
        "POST",
        "/propuestas/{}/fusionar",
        Es::Acto("propuesta:fusionar"),
    ),
    ("DELETE", "/propuestas/{}", Es::Acto("propuesta:cerrar")),
    // la cola: trabajos, entornos, funciones
    ("POST", "/trabajos", Es::Acto("trabajo:abrir")),
    ("POST", "/entorno", Es::Acto("entorno:resolver")),
    ("POST", "/entorno/{}", Es::Acto("entorno:resolver")),
    (
        "POST",
        "/funciones/{}/{}/invocar",
        Es::Acto("funcion:invocar"),
    ),
    (
        "POST",
        "/funciones/{}/{}/{}/invocar",
        Es::Acto("funcion:invocar"),
    ),
    // el puesto: abrirlo y cerrarlo son actos; lo de dentro, datos
    ("POST", "/puestos", Es::Acto("puesto:abrir")),
    ("DELETE", "/puestos/{}", Es::Acto("puesto:cerrar")),
    (
        "DELETE",
        "/puestos/{}/transform",
        Es::Acto("transform:retirar"),
    ),
    ("POST", "/puestos/{}/transform", Es::Datos),
    ("POST", "/puestos/{}/ejecutar", Es::Datos),
    ("POST", "/puestos/{}/sql", Es::Datos),
    ("POST", "/puestos/{}/lsp", Es::Datos),
    ("POST", "/puestos/{}/lsp/salida", Es::Datos),
    // 0049 B2·3: el latido del agente no cambia nada del árbol ni de los datos.
    ("POST", "/puestos/{}/latido", Es::Datos),
    ("POST", "/puestos/{}/celdas/{}/salida", Es::Datos),
    ("POST", "/vistas/**/ejecutar", Es::Datos),
    // 0046 E9·2: resolver huellas a URLs firmadas lo cuenta la ruta, con sus
    // huellas y sus blobs (`coleccion:servir`), en `GET` y en `POST` por igual.
    ("POST", "/colecciones/{}/{}/{}/items/resolver", Es::Datos),
    // 0049 B2·2: firmar las URLs de unos ítems por su puerta es leer; lo firmado
    // lo cuenta la ruta (`coleccion:servir`), como el resolver de 0046.
    ("POST", "/media/{}/{}/{}/urls", Es::Datos),
];

/// ¿Casa `ruta` con `patron`?
fn casa(patron: &[&str], ruta: &[&str]) -> bool {
    match (patron.first(), ruta.first()) {
        (None, None) => true,
        (Some(&"**"), _) => (0..=ruta.len()).any(|i| casa(&patron[1..], &ruta[i..])),
        (Some(p), Some(r)) => (*p == "{}" || p == r) && casa(&patron[1..], &ruta[1..]),
        _ => false,
    }
}

/// Qué es `metodo ruta`, si escribe. `None`: no está en la tabla (un `GET`, o una
/// ruta que escribe sin declarar, que la prueba no deja pasar).
pub fn que_es(metodo: &str, ruta: &str) -> Option<Es> {
    let segs: Vec<&str> = ruta.split('/').filter(|s| !s.is_empty()).collect();
    ESCRITURAS
        .iter()
        .find(|(m, p, _)| {
            *m == metodo && {
                let pat: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
                casa(&pat, &segs)
            }
        })
        .map(|(_, _, es)| *es)
}

/// Lo que se cuenta de una respuesta: `hecho`, `negado`, o nada.
fn resultado(codigo: u16, preguntado: bool) -> Option<&'static str> {
    match codigo {
        200..=299 => Some("hecho"),
        423 => Some("negado"),
        403 if !preguntado => Some("negado"),
        _ => None,
    }
}

impl Servidor {
    /// Tras atender: si escribió, a la actividad. `contado` es que la ruta ya lo
    /// dijo (A5); `preguntado`, que `ore-iam` decidió (y ya anotó si negó).
    pub(crate) fn a_la_actividad(
        &self,
        p: &Peticion,
        salida: &Salida,
        token: Option<String>,
        contado: bool,
        preguntado: bool,
    ) {
        let Some(buzon) = self.buzon.as_ref() else {
            return;
        };
        if contado || matches!(p.metodo.as_str(), "GET" | "HEAD" | "OPTIONS") {
            return;
        }
        let Salida::Una(r) = salida else {
            return;
        };
        let Some(Es::Acto(operacion)) = que_es(&p.metodo, &p.ruta) else {
            return;
        };
        let Some(resultado) = resultado(r.codigo, preguntado) else {
            return;
        };
        let mut e = crate::acceso::evento(
            operacion,
            p.ruta.trim_start_matches('/'),
            resultado,
            None,
            crate::acceso::commit_de(r),
        );
        e.detalle = Some(detalle(p, r));
        buzon.echar(token, e);
    }
}

/// La rama en la que se escribió (si no es la de por defecto) y el código.
fn detalle(p: &Peticion, r: &Respuesta) -> ore_core::json::Json {
    use ore_core::json::Json;
    let mut d = vec![("codigo", Json::Int(r.codigo as i64))];
    if let Some(rama) = p
        .cabeceras
        .get("x-ore-rama")
        .filter(|r| !r.trim().is_empty())
    {
        d.push(("rama", Json::s(rama.trim())));
    }
    Json::obj(d)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Los brazos de un enrutador con su método: `("POST", [...])`, `("PUT" | "DELETE",
    /// [...])`, y las alternativas `[...] | [...]`. Cada uno, con sus variables
    /// rellenas, es un camino concreto.
    fn brazos_que_escriben(fuente: &str, delante: &str) -> Vec<(String, String)> {
        let fuente = &fuente[..fuente.find("#[cfg(test)]\nmod ").unwrap_or(fuente.len())];
        let mut out = Vec::new();
        let mut desde = 0;
        while let Some(i) = fuente[desde..].find("(\"") {
            let abre = desde + i + 1;
            desde = abre;
            // Los métodos: "A" | "B" …, hasta la coma.
            let Some(coma) = fuente[abre..].find(',') else {
                break;
            };
            let metodos: Vec<&str> = fuente[abre..abre + coma]
                .split('|')
                .map(|m| m.trim().trim_matches('"'))
                .collect();
            if !metodos
                .iter()
                .all(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_uppercase()))
            {
                continue;
            }
            let mut resto = fuente[abre + coma + 1..].trim_start();
            while let Some(r) = resto.strip_prefix('[') {
                let Some(cierra) = r.find(']') else {
                    break;
                };
                let camino: Vec<String> = r[..cierra]
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(
                        |s| match s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                            Some(l) => l.to_string(),
                            None if s.ends_with("..") => "x9/x9".to_string(),
                            None => "x9".to_string(),
                        },
                    )
                    .collect();
                for m in &metodos {
                    if !matches!(*m, "GET" | "HEAD") {
                        out.push((m.to_string(), format!("{delante}/{}", camino.join("/"))));
                    }
                }
                resto = r[cierra + 1..].trim_start();
                match resto.strip_prefix('|') {
                    Some(r) => resto = r.trim_start(),
                    None => break,
                }
            }
        }
        out
    }

    /// ⛔ **Toda ruta que escribe tiene su operación** (o se declara de datos). Una
    ///   ruta nueva que no esté en `ESCRITURAS` escribiría sin dejar rastro.
    #[test]
    fn toda_ruta_que_escribe_se_declara() {
        let mut brazos = brazos_que_escriben(include_str!("rutas.rs"), "");
        let n = brazos.len();
        brazos.extend(brazos_que_escriben(include_str!("catalogo.rs"), "/v1"));
        brazos.extend(brazos_que_escriben(include_str!("documentos.rs"), ""));
        assert!(
            n > 50,
            "sólo {n} brazos que escriben en rutas.rs: el lector se ha perdido"
        );
        let sin: Vec<String> = brazos
            .iter()
            .filter(|(m, r)| que_es(m, r).is_none())
            .map(|(m, r)| format!("{m} {r}"))
            .collect();
        assert!(
            sin.is_empty(),
            "rutas que escriben sin operación en `ESCRITURAS` (actividad.rs):\n  {}",
            sin.join("\n  ")
        );
    }

    #[test]
    fn cada_camino_su_operacion() {
        for (m, r, esperado) in [
            ("POST", "/fuentes", Es::Acto("fuente:crear")),
            (
                "POST",
                "/fuentes/comprobaciones",
                Es::Acto("fuente:comprobar"),
            ),
            (
                "PUT",
                "/arbol/packages/ventas/datasets/x.yaml",
                Es::Acto("arbol:escribir"),
            ),
            (
                "PUT",
                "/ramas/ana/prueba/proteccion",
                Es::Acto("rama:proteger"),
            ),
            ("DELETE", "/ramas/ana/prueba", Es::Acto("rama:retirar")),
            (
                "POST",
                "/ramas/ana/prueba/fusionar",
                Es::Acto("rama:fusionar"),
            ),
            ("POST", "/v1/namespaces", Es::Acto("schema:crear")),
            (
                "POST",
                "/v1/ventas/namespaces/public/tables",
                Es::Acto("tabla:crear"),
            ),
            (
                "POST",
                "/v1/ventas/namespaces/public/tables/x",
                Es::Acto("tabla:actualizar"),
            ),
            ("POST", "/puestos/p1/sql", Es::Datos),
            ("POST", "/vistas/ventas/v/ejecutar", Es::Datos),
            ("POST", "/vistas/ventas/public/v/ejecutar", Es::Datos),
            ("DELETE", "/puestos/p1", Es::Acto("puesto:cerrar")),
        ] {
            assert_eq!(que_es(m, r), Some(esperado), "{m} {r}");
        }
        assert_eq!(que_es("GET", "/fuentes"), None);
    }

    #[test]
    fn lo_que_cuenta_y_lo_que_no() {
        assert_eq!(resultado(201, false), Some("hecho"));
        assert_eq!(resultado(423, false), Some("negado"));
        assert_eq!(resultado(403, false), Some("negado"));
        assert_eq!(
            resultado(403, true),
            None,
            "el 403 de una potestad ya lo anotó ore-iam"
        );
        assert_eq!(resultado(422, false), None);
        assert_eq!(resultado(503, false), None);
    }
}
