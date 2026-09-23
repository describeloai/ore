//! **Los repositorios, escritos** (0035 ⑥, 0036 ②): `POST` y `PUT /repositorios`.
//!
//! Leerlos **no tiene ruta**: `GET /assets` ya los trae (0036 ①) — un
//! repositorio es una carpeta del mismo árbol, no un segundo registro.
//! Borrarlos **tampoco estrena verbo**: es el `DELETE /arbol/<carpeta>` de
//! 0035 ③b, que se lleva la carpeta entera **en un commit** y dice qué ficheros
//! —y el manifiesto se va con ella, que es exactamente lo que debe pasar—.
//!
//! Aquí está lo que el árbol desnudo no sabe hacer:
//!
//! 1. **Nacer entero.** El manifiesto y **la semilla de la clase** se escriben
//!    en **un solo commit**: una plantilla que deja los ficheros a medias no es
//!    una plantilla. Y si se dice el proyecto, su `contiene` se actualiza **en
//!    ese mismo commit**, porque «creado pero no nombrado» es un estado que
//!    nadie pidió.
//! 2. **Decir que el sitio ya está cogido.** 409 si esa carpeta ya es un
//!    repositorio. Dos instancias sobre la misma carpeta serían dos sesiones,
//!    dos ramas y dos «lo mío» sobre los mismos ficheros.
//! 3. **Comprobar la clase.** `plantilla` tiene que estar en la tabla del
//!    producto (`ore_core::clases`), y si no, 422 **con las que sí están**: es
//!    lo que hace que la consola no pueda ofrecer algo que el servidor
//!    rechazaría.
//!
//! Lo que **no** hace, y es de 0036: no guarda configuración. El manifiesto
//! lleva `nombre`, `plantilla` y `plantillaVersion`; las dependencias van en el
//! `pyproject.toml` del repositorio, donde el ecosistema las pone.
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::path::Path;

/// Un segmento de ruta del árbol: sin `/`, sin `..`, y del alfabeto de siempre.
fn segmento_valido(s: &str, que: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 64 {
        return Err(format!("{que} tiene entre 1 y 64 letras"));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "`{s}` no vale como {que}: letras, números, `_` y `-`"
        ));
    }
    Ok(())
}

/// `<carpeta>` o `<carpeta>/<otra>`: la hondura vale, salir del árbol no.
fn carpeta_valida(c: &str) -> Result<(), String> {
    if c.is_empty() {
        return Err("falta `carpeta`: un repositorio vive DENTRO de un paquete".into());
    }
    for parte in c.split('/') {
        segmento_valido(parte, "una carpeta")?;
    }
    Ok(())
}

struct Cuerpo {
    nombre: String,
    plantilla: &'static ore_core::clases::Clase,
}

fn del_cuerpo(texto: &str) -> Result<Cuerpo, Respuesta> {
    let n = ore_core::parse::parse(texto)
        .map_err(|e| Respuesta::error(400, format!("el cuerpo no se analiza: {}", e.message)))?;
    let nombre = ore_core::manifiesto::campo(&n, "nombre").ok_or_else(|| {
        Respuesta::error(
            422,
            "falta `nombre`: un repositorio se llama de alguna manera para las personas",
        )
    })?;
    let id = ore_core::manifiesto::campo(&n, "plantilla").ok_or_else(|| {
        Respuesta::error(
            422,
            format!(
                "falta `plantilla`: la clase del repositorio. Las que hay: {}",
                ore_core::clases::nombres()
            ),
        )
    })?;
    let plantilla = ore_core::clases::de(&id).ok_or_else(|| {
        Respuesta::error(
            422,
            format!(
                "`{id}` no es una clase de repositorio. Las que hay: {}",
                ore_core::clases::nombres()
            ),
        )
    })?;
    Ok(Cuerpo { nombre, plantilla })
}

/// El manifiesto, escrito por el servidor: escalares de una línea y entre
/// comillas, para que un nombre con `---` dentro no cierre el encabezado.
fn manifiesto(nombre: &str, plantilla: &str, version: i64, prosa: Option<&str>) -> String {
    let escalar = |v: &str| {
        let limpio: String = v
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        format!(
            "\"{}\"",
            limpio.trim().replace('\\', "\\\\").replace('"', "\\\"")
        )
    };
    format!(
        "---\nnombre: {}\nplantilla: {}\nplantillaVersion: {}\n---\n\n{}\n",
        escalar(nombre),
        plantilla,
        version,
        prosa.unwrap_or("Lo que este repositorio hace, en prosa.")
    )
}

fn ficha(r: &ore_core::repositorios::Repositorio, semilla: &[String], nueva: bool) -> Json {
    Json::obj([
        ("ruta", Json::s(&r.ruta)),
        (
            "nombre",
            r.nombre
                .as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
        (
            "plantilla",
            r.plantilla
                .as_deref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
        (
            "plantillaVersion",
            r.plantilla_version
                .map(Json::Int)
                .unwrap_or(Json::Crudo("null".into())),
        ),
        ("paquete", Json::s(&r.paquete)),
        ("carpeta", Json::s(&r.carpeta)),
        ("manifiesto", Json::s(&r.manifiesto)),
        ("nueva", Json::Bool(nueva)),
        ("semilla", Json::Arr(semilla.iter().map(Json::s).collect())),
    ])
}

/// La prosa de un manifiesto: lo que va debajo del encabezado, para no perderla
/// al reescribirlo. Lo que una persona escribió no lo borra un `PUT`.
fn prosa_de(texto: &str) -> Option<String> {
    let mut lineas = texto.lines();
    if lineas.next().map(str::trim) != Some("---") {
        return None;
    }
    let mut vistas = false;
    let resto: Vec<&str> = lineas
        .skip_while(|l| {
            if vistas {
                return false;
            }
            if l.trim() == "---" {
                vistas = true;
            }
            true
        })
        .collect();
    let p = resto.join("\n").trim().to_string();
    (!p.is_empty()).then_some(p)
}

impl Servidor {
    /// `POST /repositorios {paquete, carpeta, nombre, plantilla, proyecto?}`:
    /// 201 con la ruta; 409 si esa carpeta ya es uno; 404 si no hay paquete.
    pub(crate) fn crear_repositorio(&self, raiz: &Path, cuerpo: &str) -> Respuesta {
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(e) => {
                return Respuesta::error(400, format!("el cuerpo no se analiza: {}", e.message));
            }
        };
        let c = match del_cuerpo(cuerpo) {
            Ok(c) => c,
            Err(r) => return r,
        };
        let paquete = ore_core::manifiesto::campo(&n, "paquete").unwrap_or_default();
        let carpeta = ore_core::manifiesto::campo(&n, "carpeta").unwrap_or_default();
        if let Err(m) = segmento_valido(&paquete, "un paquete") {
            return Respuesta::error(422, m);
        }
        if let Err(m) = carpeta_valida(&carpeta) {
            return Respuesta::error(422, m);
        }
        let dir_paquete = raiz.join("packages").join(&paquete);
        if !dir_paquete.is_dir() {
            return Respuesta::error(
                404,
                format!("no hay paquete `{paquete}`: un repositorio vive dentro de uno"),
            );
        }
        let ruta = format!("packages/{paquete}/{carpeta}");
        let dir = raiz.join("packages").join(&paquete).join(&carpeta);
        if dir.join("README.md").is_file()
            && ore_core::repositorios::leer(raiz)
                .iter()
                .any(|r| r.ruta == ruta)
        {
            return Respuesta::error(
                409,
                format!(
                    "`{ruta}` ya es un repositorio: dos instancias sobre la misma carpeta serían dos sesiones sobre los mismos ficheros"
                ),
            );
        }
        // El manifiesto y la semilla, en el mismo acto (el commit lo hace
        // `escribiendo_en` con todo lo que quede escrito en el clon).
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return Respuesta::error(500, format!("no se pudo crear `{ruta}`: {e}"));
        }
        let texto = manifiesto(&c.nombre, c.plantilla.id, c.plantilla.version, None);
        if let Err(e) = std::fs::write(dir.join("README.md"), texto) {
            return Respuesta::error(500, format!("no se pudo escribir `{ruta}/README.md`: {e}"));
        }
        let mut semilla = Vec::new();
        for (rel, contenido) in c.plantilla.semilla {
            let f = dir.join(rel);
            if let Some(padre) = f.parent()
                && let Err(e) = std::fs::create_dir_all(padre)
            {
                return Respuesta::error(500, format!("no se pudo sembrar `{ruta}/{rel}`: {e}"));
            }
            if let Err(e) = std::fs::write(&f, contenido) {
                return Respuesta::error(500, format!("no se pudo sembrar `{ruta}/{rel}`: {e}"));
            }
            semilla.push(format!("{ruta}/{rel}"));
        }
        // Y si se dijo el proyecto, que lo nombre — en ESTE commit.
        let mut en_proyecto = Json::Crudo("null".into());
        if let Some(p) = ore_core::manifiesto::campo(&n, "proyecto") {
            // ⭐ Lo que se añade es ESTE repositorio y no su paquete (0035
            //   ⑦.3): `contiene` es lo que el proyecto ATRIBUYE, y nombrar el
            //   paquete entero le colgaría los ítems de los vecinos. Y si el
            //   proyecto ya alcanza este sitio —lo normal, porque nace con su
            //   paquete (⑦.1) y ahí es donde cae—, no se añade nada.
            match self.nombrar_en_proyecto(raiz, &p, &format!("{paquete}/{carpeta}")) {
                Err(r) => return r,
                Ok(()) => en_proyecto = Json::s(&p),
            }
        }
        let leidos = ore_core::repositorios::leer(raiz);
        let Some(r) = leidos.iter().find(|r| r.ruta == ruta) else {
            return Respuesta::error(500, format!("`{ruta}` no se relee como repositorio"));
        };
        let mut resp = Respuesta::creado(ficha(r, &semilla, true));
        if let Json::Obj(m) = &mut resp.cuerpo {
            m.insert("proyecto".into(), en_proyecto);
        }
        resp
    }

    /// Añade `<paquete>/<carpeta>` al `contiene` de un proyecto, reescribiendo
    /// su manifiesto y **conservando su prosa**.
    fn nombrar_en_proyecto(&self, raiz: &Path, proyecto: &str, que: &str) -> Result<(), Respuesta> {
        let ruta = raiz.join("proyectos").join(proyecto).join("README.md");
        let Ok(texto) = std::fs::read_to_string(&ruta) else {
            return Err(Respuesta::error(
                404,
                format!("no hay proyecto `{proyecto}`"),
            ));
        };
        let Ok(n) = ore_core::manifiesto::encabezado(&texto) else {
            return Err(Respuesta::error(
                422,
                format!("el manifiesto del proyecto `{proyecto}` no se entiende"),
            ));
        };
        let mut contiene = ore_core::manifiesto::lista(&n, "contiene");
        // ⭐ Y no se dice dos veces lo mismo (⑦.3): si el proyecto ya alcanza
        //   este sitio —porque es su paquete, o una carpeta de arriba—, no hay
        //   nada que añadir. Un `contiene` con `p` y `p/x` dentro dice lo mismo
        //   dos veces y la segunda sobra.
        let (pq, ca) = que.split_once('/').unwrap_or((que, ""));
        if ore_core::proyectos::alcanza_en(&contiene, pq, ca) {
            return Ok(());
        }
        contiene.push(que.to_string());
        let nombre = ore_core::manifiesto::campo(&n, "nombre").unwrap_or_else(|| proyecto.into());
        let descripcion = ore_core::manifiesto::campo(&n, "descripcion");
        let mut nuevo = String::from("---\n");
        nuevo.push_str(&format!("nombre: \"{}\"\n", nombre.replace('"', "\\\"")));
        if let Some(d) = &descripcion {
            nuevo.push_str(&format!("descripcion: \"{}\"\n", d.replace('"', "\\\"")));
        }
        nuevo.push_str(&format!("contiene: [{}]\n---\n", contiene.join(", ")));
        nuevo.push_str(&format!(
            "\n{}\n",
            prosa_de(&texto).unwrap_or_else(|| "Lo que este proyecto hace, en prosa.".into())
        ));
        std::fs::write(&ruta, nuevo).map_err(|e| {
            Respuesta::error(
                500,
                format!("no se pudo escribir el proyecto `{proyecto}`: {e}"),
            )
        })
    }

    /// `PUT /repositorios/{ruta}`: el manifiesto entero —`nombre`, `plantilla`
    /// y `plantillaVersion`—, **conservando la prosa**. 404 si esa carpeta no
    /// es un repositorio: crear es `POST`.
    pub(crate) fn escribir_repositorio(&self, raiz: &Path, ruta: &str, cuerpo: &str) -> Respuesta {
        let ruta = ruta.trim_matches('/');
        let leidos = ore_core::repositorios::leer(raiz);
        if !leidos.iter().any(|r| r.ruta == ruta) {
            return Respuesta::error(404, format!("`{ruta}` no es un repositorio"));
        }
        let c = match del_cuerpo(cuerpo) {
            Ok(c) => c,
            Err(r) => return r,
        };
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(e) => {
                return Respuesta::error(400, format!("el cuerpo no se analiza: {}", e.message));
            }
        };
        let version = ore_core::manifiesto::campo(&n, "plantillaVersion")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(c.plantilla.version);
        let f = raiz.join(ruta).join("README.md");
        let antes = std::fs::read_to_string(&f).unwrap_or_default();
        let texto = manifiesto(
            &c.nombre,
            c.plantilla.id,
            version,
            prosa_de(&antes).as_deref(),
        );
        if let Err(e) = std::fs::write(&f, texto) {
            return Respuesta::error(500, format!("no se pudo escribir `{ruta}/README.md`: {e}"));
        }
        let leidos = ore_core::repositorios::leer(raiz);
        let Some(r) = leidos.iter().find(|r| r.ruta == ruta) else {
            return Respuesta::error(500, format!("`{ruta}` no se relee como repositorio"));
        };
        Respuesta::ok(ficha(r, &[], false))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_manifiesto_no_lo_puede_cerrar_un_nombre() {
        let m = manifiesto("Churn\n---\nplantilla: models", "transforms", 1, None);
        assert_eq!(m.matches("\n---\n").count(), 1, "{m}");
        assert!(m.contains("plantillaVersion: 1"), "{m}");
    }

    #[test]
    fn la_prosa_se_conserva() {
        let viejo = "---\nnombre: X\nplantilla: transforms\n---\n\nLo mío, escrito a mano.\n";
        assert_eq!(prosa_de(viejo).as_deref(), Some("Lo mío, escrito a mano."));
        let m = manifiesto("Y", "models", 2, prosa_de(viejo).as_deref());
        assert!(m.ends_with("Lo mío, escrito a mano.\n"), "{m}");
    }

    #[test]
    fn una_carpeta_puede_ser_honda_pero_no_salir_del_arbol() {
        assert!(carpeta_valida("raw").is_ok());
        assert!(carpeta_valida("raw/2026").is_ok());
        assert!(carpeta_valida("").is_err());
        assert!(carpeta_valida("../fuera").is_err());
        assert!(carpeta_valida("raw/").is_err());
    }

    #[test]
    fn la_clase_tiene_que_estar_en_la_tabla() {
        assert!(del_cuerpo(r#"{"nombre":"X","plantilla":"transforms"}"#).is_ok());
        let Err(r) = del_cuerpo(r#"{"nombre":"X","plantilla":"lo-que-sea"}"#) else {
            panic!("una clase inventada tenía que ser 422")
        };
        assert_eq!(r.codigo, 422);
        assert!(
            del_cuerpo(r#"{"plantilla":"transforms"}"#).is_err(),
            "sin nombre"
        );
    }
}
