//! `ore source add` — registrar una fuente física **separando el secreto de la
//! conexión**.
//!
//! # Por qué esto no vive en `ore-core`
//!
//! `ore-core` tiene una invariante escrita en su cabecera: la compilación es
//! pura —sin red, sin credenciales, sin reloj, sin variables de entorno—, y es
//! lo que hace verdad la frase que sostiene el producto: *el paso que decide qué
//! significan las cosas es el único que no puede filtrar nada.*
//!
//! Este comando es el **primero de ORE que toca una credencial**. Por eso vive
//! del lado del andamiaje y no del compilador, y por eso `ore-core` no expone
//! nada que lo ayude: la frontera se mantiene por lo que *no* está ahí.
//!
//! # Qué fija la especificación, y qué queda para aquí
//!
//! La forma de destino es normativa —`ontology-config.schema.json`, sección
//! `datasources`— y su descripción de `connectionEnv` nombra este comando:
//!
//! > *«El secreto NUNCA aparece en un documento OOS. Este campo declara dónde
//! > buscarlo, no qué es. Es lo que hace publicable un repositorio ontológico, y
//! > `ore source add` lo separa automáticamente para que nadie tenga que
//! > acordarse.»*
//!
//! De ahí salen las tres decisiones de este módulo:
//!
//! 1. **Lo derivable no se pregunta** (P2). El `type` sale del esquema de la URL
//!    y el nombre de la variable, del manifiesto más el nombre de la fuente. Un
//!    campo que se puede computar y aun así se pide es una oportunidad de
//!    escribirlo mal.
//! 2. **Lo que no se sabe se marca, no se inventa.** `01-package.md` §5 lo dice
//!    para la importación y vale igual aquí: *la decisión pendiente se marca;
//!    **NO DEBE** inventarse*. Un `host` que contiene `eu-west-1` **no** produce
//!    `residency: eu_only`. La ubicación física es un hecho del mundo, pero
//!    quién lo afirma es una decisión de gobierno, y esa la firma una persona.
//! 3. **El manifiesto se edita como texto.** Reescribirlo desde el árbol
//!    analizado perdería comentarios y orden — y el orden de un documento OOS es
//!    contenido: `90-canonical-form` §N4 conserva secuencias. Se inserta una
//!    entrada y el resto del fichero conserva sus bytes.
//!
//! # Lo que este comando NO hace todavía
//!
//! No abre un socket. La línea `✓ conectado · PostgreSQL 16.2 · 47 tablas` de la
//! documentación es una **sonda**, y una sonda es introspección: pertenece a
//! `discover`, con el driver que eso arrastra. Registrar la fuente y comprobar
//! que responde son dos actos con fronteras de confianza distintas, y el primero
//! está decidido de punta a punta mientras el segundo aún no.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const CONFIG: &str = "ontology.config.yaml";
const SECRETOS: &str = ".env.local";
const IGNORAR: &str = ".gitignore";

/// Lo que se pide en la línea de órdenes, antes de derivar nada.
pub struct Alta<'a> {
    pub raiz: &'a Path,
    pub nombre: &'a str,
    pub url: &'a str,
    pub tipo: Option<&'a str>,
    pub env: Option<&'a str>,
    pub etiquetas: &'a [String],
    pub descripcion: Option<&'a str>,
}

/// La entrada que acabará en `datasources`, ya derivada y comprobada.
struct Fuente {
    nombre: String,
    tipo: String,
    env: String,
    etiquetas: Vec<(String, String)>,
    descripcion: Option<String>,
}

pub fn add(a: &Alta) -> ExitCode {
    match intentar(a) {
        Ok(informe) => {
            print!("{informe}");
            ExitCode::SUCCESS
        }
        Err(fallo) => {
            eprintln!("error: {}", fallo.mensaje);
            for linea in &fallo.ayuda {
                eprintln!("{linea}");
            }
            ExitCode::from(fallo.codigo)
        }
    }
}

#[derive(Debug)]
struct Fallo {
    mensaje: String,
    ayuda: Vec<String>,
    codigo: u8,
}

impl Fallo {
    fn nueva(codigo: u8, mensaje: impl Into<String>) -> Self {
        Fallo {
            mensaje: mensaje.into(),
            ayuda: Vec::new(),
            codigo,
        }
    }
    fn ayuda(mut self, linea: impl Into<String>) -> Self {
        self.ayuda.push(linea.into());
        self
    }
}

/// 66 `EX_NOINPUT` · 65 `EX_DATAERR` · 73 `EX_CANTCREAT`, como el resto de la CLI.
fn intentar(a: &Alta) -> Result<String, Fallo> {
    let config = a.raiz.join(CONFIG);
    let texto = leer_config(&config)?;

    let arbol = ore_core::parse::parse(&texto).map_err(|e| {
        Fallo::nueva(
            65,
            format!("`{}` no analiza: {}", config.display(), e.message),
        )
    })?;

    let fuente = derivar(a, &arbol)?;
    let nuevo = insertar(&texto, &bloque(&fuente)).map_err(|m| {
        Fallo::nueva(65, m).ayuda("  Añade la entrada a mano; la forma está arriba.")
    })?;

    // Se valida ANTES de escribir. Un comando de andamiaje que deja el
    // repositorio sin compilar ha hecho más daño que trabajo.
    let diags = ore_core::validate_document(&config, &nuevo);
    if let Some(d) = diags.first() {
        return Err(
            Fallo::nueva(65, "la entrada resultante no valida contra OOS")
                .ayuda(d.render(a.raiz))
                .ayuda("  No se ha escrito nada."),
        );
    }

    // El manifiesto se escribe el ULTIMO, y el orden importa. Si algo falla por
    // el camino, sobra una linea en `.env.local` —inerte— en vez de faltar el
    // secreto de una fuente ya declarada, que es lo que rompe a `discover`.
    let secreto = escribir_secreto(a.raiz, &fuente.env, a.url)?;
    let ignorado = asegurar_ignorado(a.raiz)?;
    escribir(&config, &nuevo)?;

    Ok(informe(a, &fuente, &secreto, ignorado))
}

// ── Derivación ──────────────────────────────────────────────────────────────

fn derivar(a: &Alta, arbol: &ore_core::parse::Node) -> Result<Fuente, Fallo> {
    if !identificador(a.nombre) {
        return Err(
            Fallo::nueva(65, format!("`{}` no es un identificador válido", a.nombre))
                .ayuda("  Los bindings lo referencian con `datasourceRef`: letra, luego letras,")
                .ayuda("  dígitos o `_`. Sin puntos ni guiones."),
        );
    }

    if ya_existe(arbol, a.nombre) {
        return Err(Fallo::nueva(
            65,
            format!("`{}` ya está declarado en el manifiesto", a.nombre),
        )
        .ayuda("  Una fuente registrada dos veces con el mismo nombre haría ambiguo un")
        .ayuda("  `datasourceRef`. Elige otro nombre o edita la entrada existente."));
    }

    let tipo = match a.tipo {
        Some(t) => t.to_string(),
        None => esquema(a.url).ok_or_else(|| {
            Fallo::nueva(65, "no se puede derivar el tipo de driver de esa URL")
                .ayuda("  Se toma del esquema —`postgres://…` da `postgres`—. Si la cadena de")
                .ayuda("  conexión no lo lleva delante, dilo con `--type`.")
        })?,
    };
    if !tipo_valido(&tipo) {
        return Err(
            Fallo::nueva(65, format!("`{tipo}` no sirve como tipo de driver"))
                .ayuda("  Minúscula, luego minúsculas, dígitos o `_`."),
        );
    }

    let env = match a.env {
        Some(e) => e.to_string(),
        None => variable(nombre_del_manifiesto(arbol).as_deref(), a.nombre),
    };
    if !nombre_de_variable(&env) {
        return Err(
            Fallo::nueva(65, format!("`{env}` no sirve como nombre de variable"))
                .ayuda("  Mayúscula, luego mayúsculas, dígitos o `_`."),
        );
    }

    let mut etiquetas = Vec::new();
    for cruda in a.etiquetas {
        let (k, v) = cruda.split_once('=').ok_or_else(|| {
            Fallo::nueva(65, format!("`{cruda}` no tiene la forma `clave=valor`"))
        })?;
        if !nombre_cualificado(k) {
            return Err(
                Fallo::nueva(65, format!("`{k}` no es un nombre cualificado"))
                    .ayuda("  Una etiqueta se nombra `espacio.nombre`, como `acme.residency`."),
            );
        }
        if !identificador(v) {
            return Err(Fallo::nueva(65, format!("`{v}` no es un identificador")));
        }
        etiquetas.push((k.to_string(), v.to_string()));
    }
    etiquetas.sort();

    Ok(Fuente {
        nombre: a.nombre.to_string(),
        tipo,
        env,
        etiquetas,
        descripcion: a.descripcion.map(str::to_string),
    })
}

/// `postgres://u:p@h/db` → `postgres`. `postgresql` es el mismo driver con otro
/// nombre y se normaliza; cualquier otro esquema pasa tal cual, porque `type` es
/// un conjunto **abierto** y el compilador no razona sobre sus valores.
fn esquema(url: &str) -> Option<String> {
    let s = url.split("://").next()?;
    if s.is_empty() || s.len() == url.len() {
        return None;
    }
    let s = s.to_ascii_lowercase().replace(['-', '+'], "_");
    Some(if s == "postgresql" {
        "postgres".into()
    } else {
        s
    })
}

/// `<manifiesto>_<fuente>_URL`. Derivable, y por eso no se pregunta.
fn variable(manifiesto: Option<&str>, fuente: &str) -> String {
    let mut partes = Vec::new();
    if let Some(m) = manifiesto {
        partes.push(mayusculas(m));
    }
    partes.push(mayusculas(fuente));
    partes.push("URL".into());
    partes.retain(|p: &String| !p.is_empty());
    partes.join("_")
}

fn mayusculas(s: &str) -> String {
    let mut out = String::new();
    let mut previo_guion = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
            previo_guion = false;
        } else if !out.is_empty() && !previo_guion {
            out.push('_');
            previo_guion = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    // Una variable de entorno no puede empezar por dígito.
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'X');
    }
    out
}

fn nombre_del_manifiesto(arbol: &ore_core::parse::Node) -> Option<String> {
    arbol
        .get("metadata")
        .and_then(|(_, m)| m.get("name"))
        .and_then(|(_, v)| v.as_str())
        .map(String::from)
}

fn ya_existe(arbol: &ore_core::parse::Node, nombre: &str) -> bool {
    arbol
        .get("datasources")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .any(|it| it.get("name").and_then(|(_, v)| v.as_str()) == Some(nombre))
}

// ── Formas ──────────────────────────────────────────────────────────────────

fn identificador(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn tipo_valido(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn nombre_de_variable(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.starts_with(|c: char| c.is_ascii_uppercase())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn nombre_cualificado(s: &str) -> bool {
    !s.is_empty() && s.len() <= 512 && s.split('.').all(identificador)
}

// ── Escritura del manifiesto ────────────────────────────────────────────────

fn bloque(f: &Fuente) -> String {
    // El orden es el del esquema. Que sea siempre el mismo es lo que hace que
    // dos personas ejecutando este comando produzcan el mismo texto.
    let mut s = format!(
        "  - name: {}\n    type: {}\n    connectionEnv: {}\n",
        f.nombre, f.tipo, f.env
    );
    if !f.etiquetas.is_empty() {
        s.push_str("    labels:\n");
        for (k, v) in &f.etiquetas {
            s.push_str(&format!("      {k}: {v}\n"));
        }
    }
    if let Some(d) = &f.descripcion {
        s.push_str(&format!("    description: {}\n", entrecomillar(d)));
    }
    s
}

fn entrecomillar(s: &str) -> String {
    let escapado = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escapado}\"")
}

/// Inserta el bloque al final de `datasources:`, o crea la sección si no está.
/// Todo lo demás conserva sus bytes: comentarios incluidos.
fn insertar(texto: &str, bloque: &str) -> Result<String, String> {
    let lineas: Vec<&str> = texto.lines().collect();

    let cabecera = lineas.iter().position(|l| {
        l.starts_with("datasources:") && l["datasources:".len()..].trim_start().is_empty()
    });

    let Some(i) = cabecera else {
        // Estilo de flujo: `datasources: [ … ]`. Editarlo a ciegas sería
        // adivinar dónde acaba, y adivinar es lo que este comando no hace.
        if lineas.iter().any(|l| l.starts_with("datasources:")) {
            return Err(
                "`datasources` está escrito en estilo de flujo y este comando \
                        solo sabe editar la forma de bloque"
                    .into(),
            );
        }
        let mut s = texto.trim_end().to_string();
        s.push_str("\n\ndatasources:\n");
        s.push_str(bloque);
        return Ok(s);
    };

    // El bloque acaba en la siguiente clave de primer nivel; las líneas en
    // blanco que la preceden son separación y se conservan detrás.
    let mut fin = lineas.len();
    for (j, l) in lineas.iter().enumerate().skip(i + 1) {
        if !l.is_empty() && !l.starts_with([' ', '\t']) {
            fin = j;
            break;
        }
    }
    while fin > i + 1 && lineas[fin - 1].trim().is_empty() {
        fin -= 1;
    }

    let mut s = String::new();
    for l in &lineas[..fin] {
        s.push_str(l);
        s.push('\n');
    }
    s.push_str(bloque);
    for l in &lineas[fin..] {
        s.push_str(l);
        s.push('\n');
    }
    Ok(s)
}

// ── Escritura del secreto ───────────────────────────────────────────────────

fn escribir_secreto(raiz: &Path, env: &str, url: &str) -> Result<PathBuf, Fallo> {
    let ruta = raiz.join(SECRETOS);
    let previo = std::fs::read_to_string(&ruta).unwrap_or_default();

    if previo
        .lines()
        .any(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == env))
    {
        return Err(Fallo::nueva(73, format!("`{env}` ya está en `{SECRETOS}`"))
            .ayuda("  No se sobrescribe una credencial existente: si es la que quieres,")
            .ayuda("  bórrala a mano primero. No se ha escrito nada."));
    }

    let valor = valor_env(url)?;
    let mut s = previo;
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s.push_str(&format!("{env}={valor}\n"));
    escribir(&ruta, &s)?;
    Ok(ruta)
}

/// Una cadena de conexión va entrecomillada si lleva algo que un lector de
/// `.env` podría cortar. Lo que no se puede representar sin ambigüedad se
/// rechaza en vez de escribirse a medias.
fn valor_env(url: &str) -> Result<String, Fallo> {
    if url.contains(['\n', '\r']) {
        return Err(Fallo::nueva(
            65,
            "la cadena de conexión contiene un salto de línea",
        ));
    }
    if url.contains('\'') {
        return Err(
            Fallo::nueva(65, "la cadena de conexión contiene una comilla simple")
                .ayuda("  Los lectores de `.env` no la escapan dentro de comillas. Codifícala")
                .ayuda("  en porcentaje (`%27`) o define la variable a mano."),
        );
    }
    Ok(
        if url.contains([' ', '\t', '#', '"', '$']) || url.is_empty() {
            format!("'{url}'")
        } else {
            url.to_string()
        },
    )
}

fn asegurar_ignorado(raiz: &Path) -> Result<bool, Fallo> {
    let ruta = raiz.join(IGNORAR);
    let previo = std::fs::read_to_string(&ruta).unwrap_or_default();
    if previo
        .lines()
        .any(|l| matches!(l.trim(), SECRETOS | "/.env.local" | ".env*" | ".env.*"))
    {
        return Ok(false);
    }
    let mut s = previo;
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s.push_str(&format!("{SECRETOS}\n"));
    escribir(&ruta, &s)?;
    Ok(true)
}

fn leer_config(config: &Path) -> Result<String, Fallo> {
    std::fs::read_to_string(config).map_err(|_| {
        Fallo::nueva(66, format!("no existe `{}`", config.display()))
            .ayuda("")
            .ayuda("  Una fuente se declara en el manifiesto raíz, y el manifiesto no se")
            .ayuda("  puede inventar: su nombre y su versión son decisiones. Escríbelo:")
            .ayuda("")
            .ayuda("    apiVersion: oos.dev/v1alpha1")
            .ayuda("    kind: OntologyConfig")
            .ayuda("    metadata: { name: mi-ontologia, version: 0.1.0 }")
    })
}

fn escribir(ruta: &Path, texto: &str) -> Result<(), Fallo> {
    std::fs::write(ruta, texto)
        .map_err(|e| Fallo::nueva(73, format!("no se pudo escribir `{}`: {e}", ruta.display())))
}

// ── Informe ─────────────────────────────────────────────────────────────────

/// Nunca imprime el secreto. La contraseña de la autoridad y el valor de todo
/// parámetro que parezca una credencial se sustituyen antes de mostrar nada.
fn redactar(url: &str) -> String {
    let (esquema, resto) = match url.split_once("://") {
        Some((e, r)) => (format!("{e}://"), r.to_string()),
        None => (String::new(), url.to_string()),
    };

    let (autoridad, cola) = match resto.find(['/', '?']) {
        Some(i) => (resto[..i].to_string(), resto[i..].to_string()),
        None => (resto, String::new()),
    };

    // Solo se tapa lo que hay. Pintar un antifaz donde no había contraseña sería
    // mentir sobre la entrada, y el informe existe para que se reconozca.
    let autoridad = match autoridad.rsplit_once('@') {
        Some((cred, host)) => match cred.split_once(':') {
            Some((usuario, _)) => format!("{usuario}:••••@{host}"),
            None => format!("{cred}@{host}"),
        },
        None => autoridad,
    };

    let cola = match cola.split_once('?') {
        Some((ruta, consulta)) => {
            let partes: Vec<String> = consulta
                .split('&')
                .map(|p| match p.split_once('=') {
                    Some((k, _)) if sensible(k) => format!("{k}=••••"),
                    _ => p.to_string(),
                })
                .collect();
            format!("{ruta}?{}", partes.join("&"))
        }
        None => cola,
    };

    format!("{esquema}{autoridad}{cola}")
}

fn sensible(clave: &str) -> bool {
    let k = clave.to_ascii_lowercase();
    ["pass", "pwd", "secret", "token", "key", "credential"]
        .iter()
        .any(|s| k.contains(s))
}

fn informe(a: &Alta, f: &Fuente, secreto: &Path, ignorado: bool) -> String {
    let mut s = format!("  ✓ {} · {} · {}\n", f.nombre, f.tipo, redactar(a.url));
    s.push_str(&format!(
        "  ✓ credencial en {} como {}{}\n",
        secreto.display(),
        f.env,
        if ignorado {
            format!(" (añadido a {IGNORAR})")
        } else {
            String::new()
        }
    ));
    s.push_str(&format!(
        "  ✓ {CONFIG} declara la fuente; el secreto no aparece en él\n"
    ));

    match f
        .etiquetas
        .iter()
        .find(|(k, _)| k.rsplit('.').next() == Some("residency"))
    {
        Some((k, v)) => s.push_str(&format!("  ✓ {k}: {v}\n")),
        None => {
            s.push_str("  · residency: <sin declarar>              ← decisión pendiente\n");
            s.push_str("    Dónde vive el dato es un hecho del mundo, pero afirmarlo es de\n");
            s.push_str(
                "    quien responde por él. Cuando se sepa:  --label acme.residency=eu_only\n",
            );
        }
    }
    s.push_str("\n  Nada se ha leído de la fuente: registrar y consultar son actos distintos.\n");
    s
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_tipo_se_deriva_del_esquema() {
        assert_eq!(
            esquema("postgres://u:p@h:5432/db").as_deref(),
            Some("postgres")
        );
        assert_eq!(esquema("postgresql://h/db").as_deref(), Some("postgres"));
        assert_eq!(esquema("SNOWFLAKE://acc/db").as_deref(), Some("snowflake"));
        assert_eq!(esquema("db2-luw://h/db").as_deref(), Some("db2_luw"));
        assert_eq!(esquema("host:5432/db"), None);
    }

    #[test]
    fn la_variable_se_deriva_del_manifiesto_y_del_nombre() {
        assert_eq!(
            variable(Some("acme-retail"), "crm_prod"),
            "ACME_RETAIL_CRM_PROD_URL"
        );
        assert_eq!(variable(None, "crm"), "CRM_URL");
        // Una variable de entorno no puede empezar por dígito.
        assert_eq!(variable(Some("360"), "crm"), "X360_CRM_URL");
    }

    #[test]
    fn el_secreto_no_aparece_en_el_informe() {
        assert_eq!(
            redactar("postgres://acme:s3cr3t@db.internal:5432/crm"),
            "postgres://acme:••••@db.internal:5432/crm"
        );
        assert_eq!(
            redactar("snowflake://h/db?password=abc&role=reader"),
            "snowflake://h/db?password=••••&role=reader"
        );
        // Sin credencial en la URL no hay nada que tapar, y tapar de más
        // ocultaría a qué se ha conectado uno.
        assert_eq!(
            redactar("postgres://db.internal/crm"),
            "postgres://db.internal/crm"
        );
        assert_eq!(
            redactar("workday://acme@wd.example.com/hr"),
            "workday://acme@wd.example.com/hr"
        );
    }

    #[test]
    fn la_entrada_se_inserta_al_final_de_la_seccion() {
        let previo = "kind: OntologyConfig\n\
                      datasources:\n  \
                      - { name: pg, type: postgres, connectionEnv: PG_URL }\n\n\
                      dependencies: []\n";
        let s = insertar(previo, "  - name: crm\n").unwrap();
        assert_eq!(
            s,
            "kind: OntologyConfig\n\
             datasources:\n  \
             - { name: pg, type: postgres, connectionEnv: PG_URL }\n  \
             - name: crm\n\n\
             dependencies: []\n"
        );
    }

    #[test]
    fn sin_seccion_se_crea_al_final() {
        let s = insertar("kind: OntologyConfig\n", "  - name: crm\n").unwrap();
        assert_eq!(s, "kind: OntologyConfig\n\ndatasources:\n  - name: crm\n");
    }

    #[test]
    fn el_estilo_de_flujo_se_rechaza_en_vez_de_adivinarse() {
        assert!(insertar("datasources: [{ name: pg }]\n", "  - name: crm\n").is_err());
    }

    #[test]
    fn una_cadena_ambigua_se_rechaza_en_vez_de_escribirse_a_medias() {
        assert!(valor_env("postgres://h/db").is_ok());
        assert_eq!(
            valor_env("postgres://h/db?a=1 2").unwrap(),
            "'postgres://h/db?a=1 2'"
        );
        assert!(valor_env("postgres://u:it's@h/db").is_err());
        assert!(valor_env("uno\ndos").is_err());
    }

    #[test]
    fn el_bloque_es_el_mismo_texto_siempre() {
        let f = Fuente {
            nombre: "crm_prod".into(),
            tipo: "postgres".into(),
            env: "ACME_CRM_PROD_URL".into(),
            etiquetas: vec![
                ("gdpr.sensitivity".into(), "high".into()),
                ("acme.residency".into(), "eu_only".into()),
            ],
            descripcion: Some("CRM de producción".into()),
        };
        let mut ordenadas = f.etiquetas.clone();
        ordenadas.sort();
        let f = Fuente {
            etiquetas: ordenadas,
            ..f
        };
        assert_eq!(
            bloque(&f),
            "  - name: crm_prod\n\
             \x20   type: postgres\n\
             \x20   connectionEnv: ACME_CRM_PROD_URL\n\
             \x20   labels:\n\
             \x20     acme.residency: eu_only\n\
             \x20     gdpr.sensitivity: high\n\
             \x20   description: \"CRM de producción\"\n"
        );
    }
}
