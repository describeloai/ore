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
//! # El sitio (0035 ⑦.1)
//!
//! ⭐⭐ **Un proyecto nace con sitio propio**: `POST /proyectos` escribe, en el
//! mismo commit que su manifiesto, `packages/<id>/package.yaml` — y `contiene`
//! arranca nombrándolo. La lente se queda (sigue nombrando lo de otros, sigue
//! solapándose, sigue sin gobernar); lo que deja de pasar es que un proyecto
//! recién creado **no tenga suelo**: medido en ⑦, lo primero que se guardaba en
//! él sólo podía ir a un paquete prestado, y la consola acababa ofreciendo los
//! paquetes de la celda como si fueran carpetas suyas.
//!
//! ⛔ Y no se **adopta** el paquete de otro: si `packages/<id>` ya está, es 409
//! con el porqué. Nacer dentro de algo que ya existía sería fingir que el
//! proyecto lo creó.
//!
//! ⛔ El `owner` del paquete no se inventa: es `team:<organización>` —quien
//! RESPONDE, como en el alta de una fuente (rutas.rs `dueno_del_arbol`)— y, si
//! este servidor no sabe de quién es el árbol, `user:<persona>`. Si ninguno de
//! los dos da un handle, **el proyecto se crea igual y sin sitio**, y la
//! respuesta lo dice (`sitio: null`): más vale un proyecto sin suelo que un
//! `owner: cambiame` que no compila (medido en ⑦.1: OOS2009, 1 error).
//!
//! Y la escritura es la de siempre: el commit lo firma el sujeto y va a **su**
//! rama (`escribiendo_en`), como todo lo demás desde 0031 W3.7 ④.
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
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
///
/// ⭐ Mira los **paquetes y sus carpetas**, no los ítems (0035 ⑦): un paquete
///   vacío EXISTE —el sitio de un proyecto recién nacido lo es—, y decir que no
///   resuelve sería decir que no está.
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
    let Some(Json::Arr(paquetes)) = m.get("paquetes") else {
        return Vec::new();
    };
    let mut sitios: Vec<(String, String)> = Vec::new();
    for p in paquetes {
        let Json::Obj(p) = p else { continue };
        let Some(Json::Str(nombre)) = p.get("name") else {
            continue;
        };
        sitios.push((nombre.clone(), String::new()));
        if let Some(Json::Arr(cs)) = p.get("carpetas") {
            for c in cs {
                if let Json::Str(c) = c {
                    sitios.push((nombre.clone(), c.clone()));
                }
            }
        }
    }
    contiene
        .iter()
        .filter(|c| {
            let uno = [(*c).clone()];
            !sitios
                .iter()
                .any(|(pq, ca)| ore_core::proyectos::alcanza_en(&uno, pq, ca))
        })
        .cloned()
        .collect()
}

/// `packages/<id>`, si ese paquete está: el sitio propio del proyecto (⑦.1).
fn sitio_de(raiz: &Path, id: &str) -> Option<String> {
    raiz.join("packages")
        .join(id)
        .join("package.yaml")
        .is_file()
        .then(|| format!("packages/{id}"))
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
        // ⭐ Dónde vive (⑦.1): su paquete, si lo tiene. La consola lo enseña
        //   como LA RAÍZ del proyecto, y no como una cosa más que nombra.
        (
            "sitio",
            sitio_de(raiz, id)
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
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
    pub(crate) fn crear_proyecto(
        &self,
        raiz: &Path,
        sujeto: &Identidad,
        cuerpo: &str,
    ) -> Respuesta {
        let mut c = match del_cuerpo(cuerpo) {
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
        // ⭐⭐ EL SITIO (⑦.1): su paquete, en ESTE commit, y `contiene` lo
        //   nombra el primero. Sin esto un proyecto nace sin suelo y lo primero
        //   que se guarde en él tiene que ir a un paquete prestado.
        let suyo = raiz.join("packages").join(&id);
        if suyo.join("package.yaml").is_file() {
            return Respuesta::error(
                409,
                format!(
                    "ya hay un paquete `{id}` en el árbol: el sitio de un proyecto es suyo, y adoptar el de otro sería fingir que nació aquí. Dale otro nombre"
                ),
            );
        }
        if let Err(r) = self.dar_sitio(raiz, &id, sujeto) {
            return r;
        }
        if sitio_de(raiz, &id).is_some() && !c.contiene.iter().any(|x| x == &id) {
            c.contiene.insert(0, id.clone());
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

    /// Le da a un proyecto su sitio: `packages/<id>/package.yaml`, si no lo
    /// tiene ya y hay de quién sea. No falla si no hay dueño: el proyecto se
    /// queda sin suelo y la ficha lo dice (`sitio: null`).
    fn dar_sitio(&self, raiz: &Path, id: &str, sujeto: &Identidad) -> Result<(), Respuesta> {
        let suyo = raiz.join("packages").join(id);
        if suyo.join("package.yaml").is_file() {
            return Ok(());
        }
        let Some(dueno) = self.dueno_de_un_sitio(sujeto) else {
            return Ok(());
        };
        std::fs::create_dir_all(&suyo)
            .and_then(|_| {
                std::fs::write(
                    suyo.join("package.yaml"),
                    ore_core::paquetes::documento(id, &dueno, "draft", id),
                )
            })
            .map_err(|e| {
                Respuesta::error(
                    500,
                    format!("no se pudo escribir `packages/{id}/package.yaml`: {e}"),
                )
            })
    }

    /// De quién es el paquete de un proyecto: `team:<organización>` —quien
    /// RESPONDE, igual que en el alta de una fuente— o, si este servidor no
    /// sabe de quién es el árbol, `user:<persona>`.
    ///
    /// ⛔ `None` antes que `cambiame`: un `owner` que no es handle es `OOS2009`
    ///   y el commit no entraría (medido en ⑦.1). Sin dueño, el proyecto nace
    ///   sin sitio y la respuesta lo dice, que es peor pero es verdad.
    fn dueno_de_un_sitio(&self, sujeto: &Identidad) -> Option<String> {
        self.dueno_del_arbol().or_else(|| {
            // El sujeto viene con su clase delante (`persona:ana`); el handle
            // es quien es, no de qué clase es.
            let quien = sujeto.persona.rsplit(':').next().unwrap_or(&sujeto.persona);
            let h: String = quien
                .to_lowercase()
                .chars()
                .map(|x| {
                    if x.is_ascii_lowercase() || x.is_ascii_digit() {
                        x
                    } else {
                        '-'
                    }
                })
                .collect();
            let h = format!("user:{}", h.trim_matches('-'));
            ore_core::pertenencia::es_handle(&h).then_some(h)
        })
    }

    /// `PUT /proyectos/{id}`: el manifiesto **entero**, como se escribe un
    /// documento. 404 si no está: crear es `POST`, y así el verbo dice cuál de
    /// las dos cosas pasó.
    pub(crate) fn escribir_proyecto(
        &self,
        raiz: &Path,
        sujeto: &Identidad,
        id: &str,
        cuerpo: &str,
    ) -> Respuesta {
        if let Err(m) = id_valido(id) {
            return Respuesta::error(422, m);
        }
        let dir = raiz.join("proyectos").join(id);
        if !dir.join("README.md").is_file() {
            return Respuesta::error(404, format!("no hay proyecto `{id}`"));
        }
        let mut c = match del_cuerpo(cuerpo) {
            Ok(c) => c,
            Err(r) => return r,
        };
        // ⭐ Y si no lo tiene —un proyecto creado ANTES de ⑦.1—, se le da aquí:
        //   editarlo es el sitio natural para arreglarlo, y sin suelo no se le
        //   puede crear nada dentro. Es la única forma de recuperar los que ya
        //   estaban escritos cuando un proyecto era sólo un nombre.
        if let Err(r) = self.dar_sitio(raiz, id, sujeto) {
            return r;
        }
        // ⛔ Y el sitio no se quita con un PUT (⑦.1): un proyecto puede dejar de
        //   nombrar lo de otros, pero no el suelo donde nacen sus cosas —
        //   quitárselo dejaría sus repositorios fuera de él sin moverlos.
        if sitio_de(raiz, id).is_some() && !c.contiene.iter().any(|x| x == id) {
            c.contiene.insert(0, id.to_string());
        }
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
            // ⛔ Su sitio TAMBIÉN se queda (⑦.1): quitar la lente no borra el
            //   suelo ni lo que hay encima. Va aparte de lo demás porque es lo
            //   único que el proyecto creó, y quien lo lea tiene que verlo.
            (
                "sitio",
                sitio_de(raiz, id)
                    .map(Json::s)
                    .unwrap_or(Json::Crudo("null".into())),
            ),
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
