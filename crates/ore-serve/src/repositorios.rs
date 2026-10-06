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
//! 4. **Actualizar la plantilla** (⑧b): `POST /repositorios/{ruta}/actualizar`
//!    escribe la semilla de la versión de hoy **en una rama** y abre una
//!    **propuesta con su diff**. No es un `PUT`: esos ficheros los ha editado
//!    alguien, y pisarlos sin enseñar qué cambia sería borrar trabajo. Es lo
//!    que Foundry hace con sus upgrade PRs, y aquí ya estaba todo —ramas,
//!    propuestas y diff (0030 W2), y propuestas acotadas por ficheros (④)—:
//!    faltaba el verbo que las junta.
//!
//! Lo que **no** hace, y es de 0036: no guarda configuración. El manifiesto
//! lleva `nombre`, `plantilla` y `plantillaVersion`; las dependencias van en el
//! `pyproject.toml` del repositorio, donde el ecosistema las pone — y desde ⑧a
//! **la plantilla lo siembra**, que sin ese fichero un repositorio no podía
//! declarar nada.
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
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
        prosa.unwrap_or(PROSA)
    )
}

/// La prosa de un manifiesto que nadie ha escrito.
const PROSA: &str = "Lo que este repositorio hace, en prosa.";

/// La prosa con que se reescribe un manifiesto: la de quien la escribió, o la
/// guía de la plantilla (L1) si no hay más que la frase de siempre.
fn prosa_o_guia(
    antes: Option<&str>,
    clase: &ore_core::clases::Clase,
    paquete: &str,
    carpeta: &str,
) -> Option<String> {
    match antes {
        Some(p) if p != PROSA => Some(p.to_string()),
        _ => clase
            .guia
            .map(|g| ore_core::clases::sembrar(g, paquete, carpeta)),
    }
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

/// La semilla de `plantilla` en `packages/<paquete>/<carpeta>`, con sus huecos
/// rellenos, y lo que derive de su código (0050 G2, 0055 T1·4: el `Function`
/// de cada `@function` y el `Transform` de cada transform), como si se hubiera
/// guardado. Devuelve las rutas escritas, desde la raíz.
pub(crate) fn sembrar_en(
    raiz: &Path,
    plantilla: &ore_core::clases::Clase,
    paquete: &str,
    carpeta: &str,
    dueno: Option<&str>,
) -> Result<Vec<String>, Respuesta> {
    let ruta = format!("packages/{paquete}/{carpeta}");
    let dir = raiz.join("packages").join(paquete).join(carpeta);
    let mut semilla = Vec::new();
    for (rel, contenido) in plantilla.semilla {
        if !ore_core::clases::se_siembra(rel, paquete) {
            continue;
        }
        // R3 T6: la ruta también lleva huecos (en TypeScript el nombre de la
        // función es el del fichero).
        let rel = &ore_core::clases::sembrar(rel, paquete, carpeta);
        let f = dir.join(rel);
        if let Some(padre) = f.parent()
            && let Err(e) = std::fs::create_dir_all(padre)
        {
            return Err(Respuesta::error(
                500,
                format!("no se pudo sembrar `{ruta}/{rel}`: {e}"),
            ));
        }
        // 0050 P5: los huecos de la semilla (el paquete, la carpeta, un nombre
        // de función único en el paquete; 0055 T1·5, la base de ejemplos y un
        // dataset único en el inquilino).
        let contenido = ore_core::clases::sembrar(contenido, paquete, carpeta);
        if let Err(e) = std::fs::write(&f, contenido) {
            return Err(Respuesta::error(
                500,
                format!("no se pudo sembrar `{ruta}/{rel}`: {e}"),
            ));
        }
        semilla.push(format!("{ruta}/{rel}"));
    }
    // 0050 G2: el documento de cada `@function` de la semilla, en ESTE commit,
    // como si se hubiera guardado el código; y desde 0055 T1·4, el `Transform`.
    let sembrados: Vec<&str> = semilla.iter().map(String::as_str).collect();
    for g in crate::arbol::generar_funciones(raiz, &sembrados, dueno) {
        let r = crate::arbol::ruta_de(raiz, &g);
        if !semilla.contains(&r) {
            semilla.push(r);
        }
    }
    Ok(semilla)
}

/// Si la semilla de `plantilla` escribe en la base de ejemplos (`{{base}}`).
pub(crate) fn nombra_la_base(plantilla: &ore_core::clases::Clase) -> bool {
    plantilla
        .semilla
        .iter()
        .any(|(_, t)| t.contains("{{base}}"))
}

impl Servidor {
    /// **La base de ejemplos** (0055 T1·5, `clases::BASE_DE_EJEMPLOS`): la
    /// standard database vacía donde escribe el transform de una plantilla, que
    /// se crea en el mismo commit que el repositorio si no existe —como `create
    /// standard database` (`base_vacia`), de quien la crea—. El árbol valida
    /// sin ella (la salida está por nacer); escribir, no: `write()` pide su
    /// paquete. `Ok(Some(ruta))` si la creó.
    pub(crate) fn asegurar_base(
        &self,
        raiz: &Path,
        plantilla: &ore_core::clases::Clase,
        dueno: Option<&str>,
    ) -> Result<Option<String>, Respuesta> {
        let base = ore_core::clases::BASE_DE_EJEMPLOS;
        let manifiesto = format!("packages/{base}/package.yaml");
        if !nombra_la_base(plantilla) || raiz.join(&manifiesto).is_file() {
            return Ok(None);
        }
        // Sin quien la crea no hay dueño que darle: el repositorio nace igual,
        // y la base la crea quien la necesite (`create standard database`).
        let Some(dueno) = dueno else {
            return Ok(None);
        };
        let args: Vec<String> = vec![
            "package".into(),
            "new".into(),
            base.into(),
            "--path".into(),
            raiz.to_string_lossy().into_owned(),
            "--owner".into(),
            dueno.into(),
        ];
        match crate::mando::correr(&self.binario, raiz, &args) {
            Err(e) => Err(Respuesta::error(500, e.to_string())),
            Ok(s) if !s.bien() => Err(Respuesta::error(
                500,
                format!(
                    "la base de ejemplos `{base}` no se pudo crear: {}",
                    crate::rutas::primera_linea(&s.stdout, &s.stderr)
                ),
            )),
            Ok(_) => Ok(Some(manifiesto)),
        }
    }

    /// `POST /repositorios {paquete, carpeta, nombre, plantilla, proyecto?}`:
    /// 201 con la ruta; 409 si esa carpeta ya es uno; 404 si no hay paquete.
    pub(crate) fn crear_repositorio(
        &self,
        raiz: &Path,
        cuerpo: &str,
        dueno: Option<&str>,
    ) -> Respuesta {
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
        // ⛔ Y no dentro de un paquete de DATOS (⑧b): ahí vive lo que trajo una
        //   fuente —`discover.*` lo dice— y meter código dentro es lo que hacía
        //   que la consola ofreciera guardar en una ingesta. Un repositorio va
        //   en el paquete de un proyecto, que nace con él (0035 ⑦.1).
        if ["discover.scope.json", "discover.catalog.json"]
            .iter()
            .any(|f| dir_paquete.join(f).is_file())
        {
            return Respuesta::error(
                422,
                format!(
                    "`{paquete}` es un paquete de datos: ahí vive lo que trajo una fuente. Un repositorio va en el paquete de un proyecto"
                ),
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
        // L1: con la guía de la plantilla, si la tiene, en la prosa.
        let guia = prosa_o_guia(None, c.plantilla, &paquete, &carpeta);
        let texto = manifiesto(
            &c.nombre,
            c.plantilla.id,
            c.plantilla.version,
            guia.as_deref(),
        );
        if let Err(e) = std::fs::write(dir.join("README.md"), texto) {
            return Respuesta::error(500, format!("no se pudo escribir `{ruta}/README.md`: {e}"));
        }
        let mut semilla = match sembrar_en(raiz, c.plantilla, &paquete, &carpeta, dueno) {
            Ok(s) => s,
            Err(r) => return r,
        };
        // 0055 T1·5: la base de ejemplos donde escribe su transform, si no está.
        match self.asegurar_base(raiz, c.plantilla, dueno) {
            Ok(Some(b)) => semilla.push(b),
            Ok(None) => {}
            Err(r) => return r,
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

    /// `POST /repositorios/{ruta}/actualizar`: **la plantilla de hoy, en una
    /// rama y como propuesta** (⑧b).
    ///
    /// ⭐⭐ No se aplica: se PROPONE. Los ficheros de la semilla los ha podido
    ///   editar quien trabaja ahí, así que lo que se revisa es el **diff** —qué
    ///   trae la versión nueva y qué pisa de lo suyo—, y fusionar es aceptar.
    ///   Antes de esto, «Upgrade to v2» reescribía el número del manifiesto y
    ///   nada más: el repositorio DECÍA v2 y ERA v1.
    ///
    /// 404 si esa carpeta no es un repositorio; 409 si ya está en la versión
    /// del producto o si el producto no conoce su clase (no se inventa a qué
    /// actualizar); 422 si este árbol no tiene forja, porque sin forja no hay
    /// rama ni propuesta que abrir.
    pub(crate) fn actualizar_plantilla(&self, sujeto: &Identidad, ruta: &str) -> Respuesta {
        let ruta = ruta.trim_matches('/').to_string();
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        // Qué clase es y en qué versión está, leído en la rama por defecto.
        let mut clase: Option<&'static ore_core::clases::Clase> = None;
        let mut tenia: Option<i64> = None;
        let mut nombre = String::new();
        let mut prosa: Option<String> = None;
        let leido = self.leyendo(|r| {
            let Some(rep) = ore_core::repositorios::leer(r)
                .into_iter()
                .find(|x| x.ruta == ruta)
            else {
                return Respuesta::error(404, format!("`{ruta}` no es un repositorio"));
            };
            clase = rep.plantilla.as_deref().and_then(ore_core::clases::de);
            tenia = rep.plantilla_version;
            nombre = rep.nombre.clone().unwrap_or_else(|| rep.carpeta.clone());
            prosa = std::fs::read_to_string(r.join(&rep.manifiesto))
                .ok()
                .and_then(|t| prosa_de(&t));
            Respuesta::ok(Json::obj([("ruta", Json::s(&ruta))]))
        });
        if leido.codigo >= 300 {
            return leido;
        }
        let Some(clase) = clase else {
            return Respuesta::error(
                409,
                format!(
                    "este producto no conoce la clase de `{ruta}`: no hay a qué actualizarlo. Las que trae: {}",
                    ore_core::clases::nombres()
                ),
            );
        };
        if tenia.unwrap_or(0) >= clase.version {
            return Respuesta::error(
                409,
                format!(
                    "`{ruta}` ya está en la v{} de `{}`: no hay nada que traer",
                    clase.version, clase.id
                ),
            );
        }
        // La rama: de quien la pide, y dice lo que es.
        let base = api.rama_por_defecto().unwrap_or_else(|_| "main".into());
        let tramo = ruta.replace('/', "-");
        let rama = format!(
            "{}/plantilla-{tramo}-v{}",
            crate::propuestas::prefijo_de(&sujeto.persona),
            clase.version
        );
        if let Err(e) = api.crear_rama(&rama, &base) {
            return crate::propuestas::de_la_forja(e);
        }
        // La semilla de hoy y el manifiesto con su versión, en ESA rama.
        let (id, version) = (clase.id, clase.version);
        let semilla = clase.semilla;
        let nombre_del_commit = nombre.clone();
        let prosa_de_antes = prosa.clone();
        let ruta_r = ruta.clone();
        let mut escrito: Vec<String> = Vec::new();
        // 0050 P3: lo que se fusionó en una declaración, y lo que no se tocó.
        let mut notas: Vec<String> = Vec::new();
        let resp = self.escribiendo_en(
            Some(&rama),
            sujeto,
            &format!("actualizar `{ruta}` a la v{version} de `{id}`"),
            |r| {
                let dir = r.join(ruta_r.replace('/', std::path::MAIN_SEPARATOR_STR));
                for (rel, contenido) in semilla {
                    // Los mismos huecos que al crear: `packages/<paquete>/<carpeta>`,
                    // también en la ruta (R3 T6).
                    let (paquete, carpeta) = ruta_r
                        .strip_prefix("packages/")
                        .and_then(|x| x.split_once('/'))
                        .unwrap_or(("", ""));
                    let rel = &ore_core::clases::sembrar(rel, paquete, carpeta);
                    let f = dir.join(rel);
                    if let Some(padre) = f.parent()
                        && let Err(e) = std::fs::create_dir_all(padre)
                    {
                        return Respuesta::error(500, format!("no se pudo escribir `{rel}`: {e}"));
                    }
                    if !ore_core::clases::se_siembra(rel, paquete) {
                        continue;
                    }
                    let mut contenido = ore_core::clases::sembrar(contenido, paquete, carpeta);
                    // ⭐ 0050 P3: la declaración del repositorio es SUYA. No se
                    //   sustituye: se le AÑADE lo que la semilla declara y no
                    //   tiene, y lo demás se queda —sus librerías, sus versiones,
                    //   sus comentarios—. Lo que no se entiende no se toca.
                    if ore_core::declaracion::es_declaracion(rel)
                        && let Ok(actual) = std::fs::read_to_string(&f)
                    {
                        use ore_core::declaracion::Fusion;
                        match ore_core::declaracion::fusionar(rel, &actual, &contenido) {
                            Fusion::Igual => continue,
                            Fusion::Nueva { texto, anadido } => {
                                notas.push(format!(
                                    "`{rel}`: se añade {}; lo tuyo se queda",
                                    anadido.join(", ")
                                ));
                                contenido = texto;
                            }
                            Fusion::NoSeEntiende(m) => {
                                notas.push(format!("`{rel}` no se ha tocado: {m}"));
                                continue;
                            }
                        }
                    }
                    if let Err(e) = std::fs::write(&f, contenido) {
                        return Respuesta::error(500, format!("no se pudo escribir `{rel}`: {e}"));
                    }
                    escrito.push(format!("{ruta_r}/{rel}"));
                }
                // La prosa de quien la escribió; si no hay más que la frase de
                // siempre, la guía de la plantilla (L1).
                let (paquete, carpeta) = ruta_r
                    .strip_prefix("packages/")
                    .and_then(|x| x.split_once('/'))
                    .unwrap_or(("", ""));
                let prosa = prosa_o_guia(prosa_de_antes.as_deref(), clase, paquete, carpeta);
                let texto = manifiesto(&nombre_del_commit, id, version, prosa.as_deref());
                if let Err(e) = std::fs::write(dir.join("README.md"), texto) {
                    return Respuesta::error(
                        500,
                        format!("no se pudo escribir el manifiesto: {e}"),
                    );
                }
                escrito.push(format!("{ruta_r}/README.md"));
                // 0050 G2: lo que derive del código que trae la plantilla.
                let traidos: Vec<&str> = escrito.iter().map(String::as_str).collect();
                // Lo que nace de la plantilla es de quien la actualiza (v1alpha21).
                let dueno = self.dueno_de_quien_crea(sujeto).ok();
                for g in crate::arbol::generar_funciones(r, &traidos, dueno.as_deref()) {
                    escrito.push(crate::arbol::ruta_de(r, &g));
                }
                // 0055 T1·5: la base de ejemplos que la semilla de hoy nombra.
                match self.asegurar_base(r, clase, dueno.as_deref()) {
                    Ok(Some(b)) => escrito.push(b),
                    Ok(None) => {}
                    Err(e) => return e,
                }
                Respuesta::ok(Json::obj([("ruta", Json::s(&ruta_r))]))
            },
        );
        if resp.codigo >= 300 {
            return resp;
        }
        // Y la propuesta, que es lo que se revisa.
        let titulo = format!("Actualizar `{ruta}` a la v{version} de `{id}`");
        let mut cuerpo = format!(
            "sub: {}\n\nLa plantilla `{id}` del producto va por la v{version} y este repositorio estaba en la v{}.\nEsto trae sus ficheros tal como los trae hoy: lo que hayas cambiado sale en el diff, y fusionar es aceptarlo.",
            sujeto.persona,
            tenia.unwrap_or(0)
        );
        if !notas.is_empty() {
            cuerpo.push_str("\n\nLas declaraciones se fusionan, no se sustituyen:\n");
            for n in &notas {
                cuerpo.push_str(&format!("- {n}\n"));
            }
        }
        match api.abrir_pull(&rama, &base, &titulo, &cuerpo) {
            Ok(pr) => {
                let mut ficha = crate::propuestas::propuesta_de(&pr);
                if let Json::Obj(m) = &mut ficha {
                    m.insert("ruta".into(), Json::s(&ruta));
                    m.insert("rama".into(), Json::s(&rama));
                    m.insert("plantilla".into(), Json::s(id));
                    m.insert("plantillaVersion".into(), Json::Int(version));
                    m.insert(
                        "ficheros".into(),
                        Json::Arr(escrito.iter().map(Json::s).collect()),
                    );
                }
                Respuesta::creado(ficha)
            }
            Err(e) => crate::propuestas::de_la_forja(e),
        }
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

    /// L1: un repositorio de TypeScript nace con la guía en su manifiesto; al
    /// actualizar, la guía entra donde no había más que la frase de siempre, y
    /// la prosa que alguien escribió no se pisa.
    #[test]
    fn la_guia_nace_en_el_manifiesto_y_no_pisa_la_prosa() {
        let ts = ore_core::clases::de("functions-typescript").unwrap();
        let guia = prosa_o_guia(None, ts, "ventas", "riesgo").unwrap();
        assert!(guia.starts_with("# TypeScript functions"), "{guia}");
        assert!(guia.contains("riesgoInvoiceStatus.ts") && !guia.contains("{{"));
        let m = manifiesto("Riesgo", ts.id, ts.version, Some(&guia));
        assert_eq!(prosa_de(&m).as_deref(), Some(guia.as_str()));
        assert_eq!(
            prosa_o_guia(Some(PROSA), ts, "ventas", "riesgo"),
            Some(guia)
        );
        assert_eq!(
            prosa_o_guia(Some("Lo mío."), ts, "ventas", "riesgo").as_deref(),
            Some("Lo mío.")
        );
        // 0050 P3: la de Python también nace con su guía.
        let py = ore_core::clases::de("functions-python").unwrap();
        let g = prosa_o_guia(None, py, "ventas", "riesgo").unwrap();
        assert!(
            g.starts_with("# Python functions") && !g.contains("{{"),
            "{g}"
        );
        // 0055 T1·5: y la de transforms de Python.
        let tr = ore_core::clases::de("transforms-python").unwrap();
        let g = prosa_o_guia(None, tr, "ventas", "riesgo").unwrap();
        assert!(
            g.starts_with("# Python transforms") && !g.contains("{{"),
            "{g}"
        );
        // Una plantilla sin guía: la frase de siempre.
        let sql = ore_core::clases::de("transforms-sql").unwrap();
        assert_eq!(prosa_o_guia(None, sql, "ventas", "riesgo"), None);
    }

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

/// 0055 T1·5: crear un repositorio de transforms lo deja en verde, con su
/// `Transform` en `pipeline/` del mismo commit.
#[cfg(test)]
mod semilla_de_transforms {
    use super::*;

    #[test]
    fn crear_cada_repositorio_de_transforms_nace_en_verde() {
        for id in ["transforms-python", "transforms-sql"] {
            let c = ore_core::clases::de(id).unwrap();
            assert!(nombra_la_base(c), "{id}");
            let raiz =
                std::env::temp_dir().join(format!("ore-serve-semilla-{id}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&raiz);
            std::fs::create_dir_all(&raiz).unwrap();
            std::fs::write(
                raiz.join("ontology.config.yaml"),
                "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n",
            )
            .unwrap();
            // El proyecto y la base de ejemplos (la que `asegurar_base` crea
            // con `ore package new`), como `package.yaml`.
            for p in ["ventas", ore_core::clases::BASE_DE_EJEMPLOS] {
                std::fs::create_dir_all(raiz.join("packages").join(p)).unwrap();
                std::fs::write(
                    raiz.join("packages").join(p).join("package.yaml"),
                    format!(
                        "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {p}, version: 0.1.0, status: draft, domain: {p} }}\nspec: {{ owner: \"user:ana\" }}\n"
                    ),
                )
                .unwrap();
            }
            let Ok(semilla) = sembrar_en(&raiz, c, "ventas", "etl", Some("user:ana")) else {
                panic!("{id}: no se siembra")
            };
            let doc = "packages/ventas/etl/pipeline/sandbox.ventas_etl_example.yaml";
            assert!(semilla.iter().any(|r| r == doc), "{id}: {semilla:?}");
            let texto = std::fs::read_to_string(raiz.join(doc)).unwrap();
            assert!(texto.contains("  inputs: []\n") && texto.contains("owner: user:ana"));
            let d = ore_core::validate::validate_package(&raiz);
            assert!(
                d.is_empty(),
                "{id}: {:?}",
                d.iter()
                    .map(|x| format!("{} {}", x.code.as_str(), x.message))
                    .collect::<Vec<_>>()
            );
            let _ = std::fs::remove_dir_all(&raiz);
        }
        assert!(!nombra_la_base(
            ore_core::clases::de("functions-python").unwrap()
        ));
    }
}
