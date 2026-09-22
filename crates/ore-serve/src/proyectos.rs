//! **Los proyectos, escritos** (0035 ②): `POST`, `PUT` y `DELETE /proyectos`.
//!
//! Leerlos **no** tiene ruta propia: `GET /assets` ya los trae (0034 ⑤, 0035
//! ①) — una llamada, la que la consola ya hace. Aquí está sólo lo que el árbol
//! desnudo no sabe hacer: escribir el manifiesto con su forma, decir que el
//! nombre ya está cogido, y borrar **el manifiesto y no lo que nombra**.
//!
//! Tres cosas que este módulo no hace, y cada una es la decisión de 0035:
//!
//! 1. **No compila antes de escribir.** Un documento pasa por `empeora` porque
//!    puede romper el árbol; un proyecto **no puede** —medido en ⓪: con el
//!    manifiesto dentro `ore validate` sale 0 y no lo nombra en demo ni en
//!    victor—. Escribir un proyecto no puede dejar el árbol peor que estaba.
//! 2. **No comprueba que `contiene` resuelva.** Un proyecto puede nombrar lo
//!    que todavía no existe: es un propósito, no un inventario. Lo que no
//!    resuelve se **dice** en la respuesta (`sinResolver`), como `sinHablar` en
//!    los documentos, y no impide nada.
//! 3. **No decide quién lo ve.** Eso es `ore-iam`, que es plano de control. El
//!    árbol no lleva colaboradores.
//!
//! Y la escritura es la de siempre: el commit lo firma el sujeto y va a **su**
//! rama (`escribiendo_en`), como todo lo demás desde 0031 W3.7 ④.
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::path::Path;

/// El identificador de un proyecto: el nombre de su carpeta.
fn id_valido(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        return Err("el identificador de un proyecto tiene entre 1 y 64 letras".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "`{id}` no vale como identificador: minúsculas, números y guiones (es el nombre de una carpeta)"
        ));
    }
    if id.starts_with('-') || id.ends_with('-') {
        return Err(format!("`{id}` no puede empezar ni acabar en guion"));
    }
    Ok(())
}

/// De un título a un identificador: lo que la consola enseña, hecho carpeta.
pub(crate) fn id_de(titulo: &str) -> String {
    let mut out = String::new();
    for c in titulo.trim().to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(64).collect()
}

/// Un escalar del encabezado, en una línea y entre comillas: el manifiesto lo
/// escribe el servidor, así que nada de lo que venga puede abrir otra clave ni
/// cerrar el encabezado.
fn escalar(v: &str) -> String {
    let limpio: String = v
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    format!(
        "\"{}\"",
        limpio.trim().replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Lo que nombra un proyecto: `<paquete>` o `<paquete>/<carpeta…>`.
fn contenido_valido(c: &str) -> Result<(), String> {
    if c.is_empty() || c.starts_with('/') || c.ends_with('/') || c.contains("..") {
        return Err(format!(
            "`{c}` no vale en `contiene`: es `<paquete>` o `<paquete>/<carpeta>`"
        ));
    }
    if !c
        .chars()
        .all(|x| x.is_ascii_alphanumeric() || x == '_' || x == '-' || x == '/')
    {
        return Err(format!(
            "`{c}` no vale en `contiene`: letras, números, `_`, `-` y `/`"
        ));
    }
    Ok(())
}

/// Lo que viene en el cuerpo: `{nombre, descripcion?, contiene?}`.
struct Cuerpo {
    titulo: String,
    descripcion: Option<String>,
    contiene: Vec<String>,
}

fn del_cuerpo(texto: &str) -> Result<Cuerpo, Respuesta> {
    let n = ore_core::parse::parse(texto)
        .map_err(|e| Respuesta::error(400, format!("el cuerpo no se analiza: {}", e.message)))?;
    let titulo = n
        .get("nombre")
        .and_then(|(_, v)| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Respuesta::error(
                422,
                "falta `nombre`: un proyecto es un propósito con nombre",
            )
        })?
        .to_string();
    let descripcion = n
        .get("descripcion")
        .and_then(|(_, v)| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut contiene = Vec::new();
    if let Some((_, v)) = n.get("contiene") {
        for i in v.items() {
            let Some(s) = i.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            contenido_valido(s).map_err(|m| Respuesta::error(422, m))?;
            if !contiene.iter().any(|x| x == s) {
                contiene.push(s.to_string());
            }
        }
    }
    Ok(Cuerpo {
        titulo,
        descripcion,
        contiene,
    })
}

fn manifiesto(c: &Cuerpo) -> String {
    let mut s = String::from("---\n");
    s.push_str(&format!("nombre: {}\n", escalar(&c.titulo)));
    if let Some(d) = &c.descripcion {
        s.push_str(&format!("descripcion: {}\n", escalar(d)));
    }
    if !c.contiene.is_empty() {
        s.push_str(&format!("contiene: [{}]\n", c.contiene.join(", ")));
    }
    s.push_str("---\n");
    s.push_str(&format!(
        "\n{}\n",
        c.descripcion
            .as_deref()
            .unwrap_or("Lo que este proyecto hace, en prosa.")
    ));
    s
}

/// Los paquetes y carpetas del árbol, para decir qué de `contiene` no resuelve
/// **todavía**. No impide nada: el proyecto es un propósito.
fn sin_resolver(raiz: &Path, contiene: &[String]) -> Vec<String> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let indice = ore_core::assets::indice(
        &pkg,
        &std::collections::BTreeMap::new(),
        &ore_core::assets::Cabeza::default(),
    );
    let Json::Obj(m) = &indice else {
        return Vec::new();
    };
    let Some(Json::Obj(items)) = m.get("items") else {
        return Vec::new();
    };
    let sitios: Vec<(String, String)> = items
        .values()
        .filter_map(|it| {
            let Json::Obj(it) = it else { return None };
            let p = match it.get("paquete") {
                Some(Json::Str(p)) => p.clone(),
                _ => return None,
            };
            let c = match it.get("carpeta") {
                Some(Json::Str(c)) => c.clone(),
                _ => String::new(),
            };
            Some((p, c))
        })
        .collect();
    contiene
        .iter()
        .filter(|c| {
            let p = ore_core::proyectos::Proyecto {
                nombre: String::new(),
                titulo: None,
                descripcion: None,
                contiene: vec![(*c).clone()],
                ruta: String::new(),
                roto: None,
            };
            !sitios.iter().any(|(pq, ca)| p.alcanza(pq, ca))
        })
        .cloned()
        .collect()
}

fn ficha(raiz: &Path, id: &str, c: &Cuerpo, nueva: bool) -> Json {
    let mut m = vec![
        ("id", Json::s(id)),
        ("nombre", Json::s(&c.titulo)),
        (
            "descripcion",
            c.descripcion
                .as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
        (
            "contiene",
            Json::Arr(c.contiene.iter().map(Json::s).collect()),
        ),
        ("ruta", Json::s(format!("proyectos/{id}/README.md"))),
        ("nueva", Json::Bool(nueva)),
    ];
    let falta = sin_resolver(raiz, &c.contiene);
    if !falta.is_empty() {
        m.push((
            "sinResolver",
            Json::Arr(falta.into_iter().map(Json::s).collect()),
        ));
    }
    Json::obj(m)
}

impl Servidor {
    /// `POST /proyectos {nombre, descripcion?, contiene?}`: 201 con el `id` (el
    /// nombre de la carpeta, del título), o 409 si ese nombre ya está cogido.
    pub(crate) fn crear_proyecto(&self, raiz: &Path, cuerpo: &str) -> Respuesta {
        let c = match del_cuerpo(cuerpo) {
            Ok(c) => c,
            Err(r) => return r,
        };
        let id = id_de(&c.titulo);
        if let Err(m) = id_valido(&id) {
            return Respuesta::error(422, m);
        }
        let dir = raiz.join("proyectos").join(&id);
        if dir.join("README.md").is_file() {
            return Respuesta::error(
                409,
                format!("ya hay un proyecto `{id}`: los proyectos se nombran una vez"),
            );
        }
        if let Err(e) = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(dir.join("README.md"), manifiesto(&c)))
        {
            return Respuesta::error(
                500,
                format!("no se pudo escribir `proyectos/{id}/README.md`: {e}"),
            );
        }
        Respuesta::creado(ficha(raiz, &id, &c, true))
    }

    /// `PUT /proyectos/{id}`: el manifiesto **entero**, como se escribe un
    /// documento. 404 si no está: crear es `POST`, y así el verbo dice cuál de
    /// las dos cosas pasó.
    pub(crate) fn escribir_proyecto(&self, raiz: &Path, id: &str, cuerpo: &str) -> Respuesta {
        if let Err(m) = id_valido(id) {
            return Respuesta::error(422, m);
        }
        let dir = raiz.join("proyectos").join(id);
        if !dir.join("README.md").is_file() {
            return Respuesta::error(404, format!("no hay proyecto `{id}`"));
        }
        let c = match del_cuerpo(cuerpo) {
            Ok(c) => c,
            Err(r) => return r,
        };
        if let Err(e) = std::fs::write(dir.join("README.md"), manifiesto(&c)) {
            return Respuesta::error(
                500,
                format!("no se pudo escribir `proyectos/{id}/README.md`: {e}"),
            );
        }
        Respuesta::ok(ficha(raiz, id, &c, false))
    }

    /// `DELETE /proyectos/{id}`: se va **el manifiesto, no lo que nombra**. Un
    /// proyecto es una lente: quitarla no quita lo que se veía a través de
    /// ella, y la respuesta lo dice (`siguenEnElArbol`).
    pub(crate) fn retirar_proyecto(&self, raiz: &Path, id: &str) -> Respuesta {
        if let Err(m) = id_valido(id) {
            return Respuesta::error(422, m);
        }
        let dir = raiz.join("proyectos").join(id);
        if !dir.join("README.md").is_file() {
            return Respuesta::error(404, format!("no hay proyecto `{id}`"));
        }
        let nombraba: Vec<String> = ore_core::proyectos::leer(raiz)
            .into_iter()
            .find(|p| p.nombre == id)
            .map(|p| p.contiene)
            .unwrap_or_default();
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            return Respuesta::error(500, format!("no se pudo retirar `proyectos/{id}`: {e}"));
        }
        Respuesta::ok(Json::obj([
            ("id", Json::s(id)),
            ("retirado", Json::Bool(true)),
            (
                "siguenEnElArbol",
                Json::Arr(nombraba.iter().map(Json::s).collect()),
            ),
        ]))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_id_sale_del_titulo() {
        assert_eq!(id_de("Customer Churn"), "customer-churn");
        assert_eq!(id_de("  Nómina 2026 "), "n-mina-2026");
        assert_eq!(id_de("A//B"), "a-b");
        assert_eq!(id_de("···"), "");
    }

    #[test]
    fn el_manifiesto_se_escribe_con_su_forma_y_nada_se_escapa() {
        let m = manifiesto(&Cuerpo {
            titulo: "Churn\n---\nkind: Project".into(),
            descripcion: Some("Con \"comillas\"".into()),
            contiene: vec!["ventas/churn".into(), "rrhh".into()],
        });
        assert!(m.starts_with("---\n"), "{m}");
        assert_eq!(m.matches("\n---\n").count(), 1, "sólo cierra una vez: {m}");
        assert!(m.contains("contiene: [ventas/churn, rrhh]"), "{m}");
        assert!(m.contains(r#"descripcion: "Con \"comillas\"""#), "{m}");
    }

    #[test]
    fn lo_escrito_se_vuelve_a_leer_igual() {
        let d = std::env::temp_dir().join(format!("ore-serve-proyectos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dir = d.join("proyectos").join("churn");
        std::fs::create_dir_all(&dir).unwrap();
        let c = Cuerpo {
            titulo: "Customer Churn".into(),
            descripcion: Some("Abandono.".into()),
            contiene: vec!["ventas/churn".into()],
        };
        std::fs::write(dir.join("README.md"), manifiesto(&c)).unwrap();
        let ps = ore_core::proyectos::leer(&d);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].titulo.as_deref(), Some("Customer Churn"));
        assert_eq!(ps[0].descripcion.as_deref(), Some("Abandono."));
        assert_eq!(ps[0].contiene, ["ventas/churn"]);
        assert!(ps[0].roto.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_cuerpo_exige_nombre_y_mira_lo_que_nombra() {
        assert!(del_cuerpo("{}").is_err(), "sin `nombre`, 422");
        let Ok(c) = del_cuerpo(r#"{"nombre":"X","contiene":["ventas","ventas"]}"#) else {
            panic!("tenía que analizarse")
        };
        assert_eq!(c.contiene, ["ventas"], "lo repetido se dice una vez");
        assert!(
            del_cuerpo(r#"{"nombre":"X","contiene":["../etc"]}"#).is_err(),
            "salir del árbol, 422"
        );
    }
}
