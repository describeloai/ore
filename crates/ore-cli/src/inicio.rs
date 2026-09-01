//! `ore init` — **el diálogo de creación de un repositorio ontológico.**
//!
//! # Qué es esto, y por qué no es un `mkdir`
//!
//! En una plataforma, un repositorio nace de un formulario: se elige el nombre,
//! el sitio y de qué ontología se parte, y lo que aparece **ya viene con esa
//! respuesta dentro**. La decisión existe; lo que no existe es el paso de abrir
//! un fichero y escribirla a mano.
//!
//! Aquí no hay plataforma, así que **este comando es ese formulario**. Y de ahí
//! sale toda su forma: las decisiones que no puede inventar las toma **como
//! respuestas** —banderas— y emite la plantilla con ellas ya escritas.
//!
//! La versión anterior se quedaba a medias: no inventaba, y tampoco ayudaba.
//! Terminaba diciendo *«tres decisiones te esperan»* y dejaba a quien lo usara
//! delante de un YAML. Eso es lo peor de las dos posturas — la austeridad estaba
//! bien fundada y la conclusión era la equivocada. No era *«no lo escribo»*, era
//! **«pregunto y lo escribo»**.
//!
//! # Lo que sigue sin inventar
//!
//! No escribe un retículo ni un `ConduitPolicy`, y eso no ha cambiado. Los dos
//! son **decisiones de gobierno** y no hay bandera que valga: un retículo es una
//! escala de sensibilidad que alguien tiene que sostener, no un parámetro.
//!
//! Y con el conducto hay una razón más fina: **omitirlo ya significa algo.**
//! `conduit-policy.schema.json` lo dice —*«un conducto NO listado tiene
//! autorización ⊥ y no admite nada: denegación por defecto (P4)»*—. Un
//! repositorio recién creado no sirve nada por ningún conducto, que es la
//! postura correcta, y escribirla no la haría más cierta.
//!
//! **La salida que sí existe es importar.** `--depend` no inventa una
//! clasificación: se acoge a la de otro, que es lo que el esquema llama
//! *transferir autoridad*. Un repositorio que declara `oos.dev/regulatory/gdpr`
//! tiene retículo y conceptos desde el minuto cero sin que nadie de dentro haya
//! decidido nada.
//!
//! # Y lo que emite, que es más que el manifiesto
//!
//! | | Por qué está |
//! |---|---|
//! | `ontology.config.yaml` | las respuestas del diálogo, escritas |
//! | `packages/` | donde el compilador busca, y su ausencia haría sorpresa la convención |
//! | `.gitignore` | el secreto fuera, la caché fuera — y **`vendor/` DENTRO** |
//! | `.github/workflows/` | las garantías **exigidas**, no disponibles |
//! | `AGENTS.md` | qué es este sitio y qué no se puede romper en él |
//!
//! El workflow es el que menos parece y más pesa. Un repositorio que lleva las
//! garantías pero no las corre las tiene *disponibles*; uno que las corre en cada
//! empujón las tiene **exigidas**, y esa es toda la diferencia entre un formato y
//! un contrato.

use std::path::Path;
use std::process::ExitCode;

const CONFIG: &str = "ontology.config.yaml";
const IGNORAR: &str = ".gitignore";
const PAQUETES: &str = "packages";
const FLUJO: &str = ".github/workflows/ontologia.yml";
const AGENTES: &str = "AGENTS.md";

/// Un par `nombre = valor` ya leído: una dependencia o una clave.
type Par = (String, String);

/// Lo que sale de leer una bandera, o el fallo con su ayuda ya escrita.
type Leidos = Result<Vec<Par>, (u8, Vec<String>)>;

/// Las respuestas del diálogo de creación.
///
/// Cada una es algo que este comando **no puede saber** y que tampoco debe
/// inventar. Que lleguen por bandera y no por edición posterior es la diferencia
/// entre un repositorio que nace funcionando y uno que nace con deberes.
#[derive(Default)]
pub struct Respuestas<'a> {
    pub nombre: Option<&'a str>,
    /// `coordenada@rango`, repetible.
    pub depende: &'a [String],
    /// `id=clavePublica`, repetible. Con quién se comprueba una firma.
    pub claves: &'a [String],
    /// `id=clavePublica`, repetible. Con quién se comprueba un log.
    pub logs: &'a [String],
}

pub fn init(raiz: &Path, r: &Respuestas) -> ExitCode {
    match intentar(raiz, r) {
        Ok(informe) => {
            print!("{informe}");
            ExitCode::SUCCESS
        }
        Err((codigo, lineas)) => {
            for (i, l) in lineas.iter().enumerate() {
                if i == 0 {
                    eprintln!("error: {l}");
                } else {
                    eprintln!("{l}");
                }
            }
            ExitCode::from(codigo)
        }
    }
}

fn intentar(raiz: &Path, r: &Respuestas) -> Result<String, (u8, Vec<String>)> {
    let config = raiz.join(CONFIG);
    if config.exists() {
        return Err((
            73, // EX_CANTCREAT
            vec![
                format!("ya existe `{}`", config.display()),
                "  Este directorio ya es un repositorio ontológico. `init` no lo toca:".into(),
                "  sobrescribir un manifiesto perdería lo que declara.".into(),
            ],
        ));
    }

    let nombre = match r.nombre {
        Some(n) => n.to_string(),
        None => derivar_nombre(raiz).ok_or_else(|| {
            (
                65, // EX_DATAERR
                vec![
                    "no se puede derivar un nombre de paquete del directorio".into(),
                    "  Un nombre es minúscula, luego minúsculas, dígitos o `-`.".into(),
                    "  Dilo con `--name`.".into(),
                ],
            )
        })?,
    };
    if !nombre_valido(&nombre) {
        return Err((
            65,
            vec![
                format!("`{nombre}` no sirve como nombre de paquete"),
                "  Minúscula, luego minúsculas, dígitos o `-`. Sin puntos ni mayúsculas.".into(),
            ],
        ));
    }

    // Las respuestas se leen ANTES de escribir nada. Un repositorio a medio
    // emitir por una bandera mal escrita es peor que uno no emitido: el error
    // sale al final y lo que hay ya no se puede volver a crear con `init`.
    let dependencias = leer_dependencias(r.depende)?;
    let claves = leer_claves(r.claves, "--trust")?;
    let logs = leer_claves(r.logs, "--trust-log")?;

    let escribir = |ruta: &Path, texto: &str| -> Result<(), (u8, Vec<String>)> {
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d)
                .map_err(|e| (73, vec![format!("no se pudo crear `{}`: {e}", d.display())]))?;
        }
        std::fs::write(ruta, texto).map_err(|e| {
            (
                73,
                vec![format!("no se pudo escribir `{}`: {e}", ruta.display())],
            )
        })
    };

    escribir(&config, &manifiesto(&nombre, &dependencias, &claves, &logs))?;

    // El directorio de paquetes existe aunque esté vacío: `workspace.members`
    // vale `packages/*` por convención y un directorio ausente convierte esa
    // convención en una sorpresa.
    std::fs::create_dir_all(raiz.join(PAQUETES))
        .map_err(|e| (73, vec![format!("no se pudo crear `{PAQUETES}/`: {e}")]))?;
    escribir(&raiz.join(PAQUETES).join(".gitkeep"), "")?;

    escribir(&raiz.join(FLUJO), &flujo())?;
    escribir(&raiz.join(AGENTES), &agentes(&nombre, &dependencias))?;

    let ignorar = raiz.join(IGNORAR);
    let mut s = std::fs::read_to_string(&ignorar).unwrap_or_default();
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    for (linea, por_que) in IGNORADOS {
        if !s.lines().any(|l| l.trim() == *linea) {
            s.push_str(&format!("\n# {por_que}\n{linea}\n"));
        }
    }
    escribir(&ignorar, &s)?;

    Ok(informe(&nombre, &dependencias, &claves, &logs))
}

/// Lo que no entra en git, con el motivo al lado.
///
/// **`vendor/` no está aquí, y esa ausencia es deliberada.** El reflejo es
/// ignorarlo —parece una carpeta de dependencias descargadas— y sería justo lo
/// contrario de lo que hace útil vendorizar: lo traído se compromete al
/// repositorio para que un clon recién hecho compile sin alcanzar a nadie, y
/// para que el digest de cada dependencia se revise en un pull request como todo
/// lo demás.
const IGNORADOS: &[(&str, &str)] = &[
    (
        ".env.local",
        "Los secretos de conexión. `ore source add` los escribe aquí y el manifiesto\n\
         # solo declara de qué variable salen, para que este repositorio sea publicable.",
    ),
    (".ore/", "Caché derivada del compilador. Se regenera sola."),
];

// ── Las respuestas ──────────────────────────────────────────────────────────

/// `coordenada@rango`. El rango **no tiene valor por defecto**: una dependencia
/// sin rango es una decisión a medias, y adivinarlo aquí sería fijar en silencio
/// qué actualizaciones entran solas.
fn leer_dependencias(crudas: &[String]) -> Leidos {
    crudas
        .iter()
        .map(|d| {
            d.rsplit_once('@')
                .filter(|(c, r)| !c.is_empty() && !r.is_empty())
                .map(|(c, r)| (c.to_string(), r.to_string()))
                .ok_or_else(|| {
                    (
                        64, // EX_USAGE
                        vec![
                            format!("`--depend {d}` no dice qué rango se acepta"),
                            "  La forma es `<coordenada>@<rango>`, y el rango no se puede".into(),
                            "  adivinar: es lo que decide qué actualizaciones entran solas.".into(),
                            "  Por ejemplo: `--depend oos.dev/regulatory/gdpr@^0.1`".into(),
                        ],
                    )
                })
        })
        .collect()
}

/// `id=clavePublica`, con la clave comprobada.
///
/// Se valida la forma **aquí y no al usarla**, porque una clave con un carácter
/// de más no da un error: da un repositorio donde ninguna firma casa nunca, o
/// —peor— uno donde nadie mira. Un dedo puesto donde no toca no puede acabar en
/// silencio en el sitio donde se decide en quién se confía.
fn leer_claves(crudas: &[String], bandera: &str) -> Leidos {
    crudas
        .iter()
        .map(|c| {
            let (id, clave) = c.split_once('=').ok_or_else(|| {
                (
                    64,
                    vec![
                        format!("`{bandera} {c}` no tiene la forma `<id>=<clavePublica>`"),
                        format!("  Por ejemplo: `{bandera} oos.dev=c853ad0f…`"),
                    ],
                )
            })?;
            let bien = clave.len() == 64
                && clave
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !bien {
                return Err((
                    65,
                    vec![
                        format!("la clave de `{id}` no es 32 bytes en hexadecimal minúsculo"),
                        "  Son 64 caracteres de `0-9a-f`. Una clave mal copiada no da un".into(),
                        "  error más tarde: da un repositorio donde ninguna firma casa, o".into(),
                        "  uno donde nadie mira.".into(),
                    ],
                ));
            }
            Ok((id.to_string(), clave.to_string()))
        })
        .collect()
}

// ── Lo que se emite ─────────────────────────────────────────────────────────

fn manifiesto(nombre: &str, deps: &[Par], claves: &[Par], logs: &[Par]) -> String {
    let mut s = format!(
        "apiVersion: oos.dev/v1alpha1\n\
         kind: OntologyConfig\n\
         metadata: {{ name: {nombre}, version: 0.1.0 }}\n\
         \n\
         # `workspace.members` vale `packages/*` por convención, así que no se\n\
         # declara: lo derivable no se declara (P2). Solo hace falta escribirlo\n\
         # con una disposición no estándar.\n\
         #\n\
         # `datasources` lo escribe `ore source add`, que separa el secreto de la\n\
         # conexión para que este fichero siga siendo publicable.\n"
    );
    if !deps.is_empty() {
        s.push_str(
            "\n# Declarar una dependencia es TRANSFERIR AUTORIDAD: al escribir esto, esta\n\
             # organización afirma que la definición de lo que aquí se importa NO ES SUYA,\n\
             # y se acoge a un enunciado concreto y auditable de esa autoridad.\n\
             #\n\
             # `ore lock` la resuelve y fija su digest. A partir de ahí, que lo que hay\n\
             # sea lo que se aceptó deja de ser una promesa.\n\
             dependencies:\n",
        );
        for (c, r) in deps {
            s.push_str(&format!("  - {{ package: {c}, version: \"{r}\" }}\n"));
        }
    }
    if !claves.is_empty() {
        s.push_str(
            "\n# Con qué claves se comprueba la firma de lo que se importa.\n\
             #\n\
             # Viven AQUÍ, en el manifiesto de quien consume, y no dentro del paquete que\n\
             # firman: una clave que viajara con su propio paquete cerraría el círculo —\n\
             # quien sustituye el paquete sustituye la clave, firma con la suya, y todo\n\
             # verifica.\n\
             trustedKeys:\n",
        );
        for (id, k) in claves {
            s.push_str(&format!(
                "  - {{ id: \"{id}\", algorithm: ed25519, publicKey: \"{k}\" }}\n"
            ));
        }
    }
    if !logs.is_empty() {
        s.push_str(
            "\n# Y con qué logs de transparencia. Una firma dice DE QUIÉN es un paquete;\n\
             # un log dice que esa clave no le ha dicho algo distinto a otro en privado.\n\
             trustedLogs:\n",
        );
        for (id, k) in logs {
            s.push_str(&format!(
                "  - {{ id: \"{id}\", algorithm: ed25519, publicKey: \"{k}\" }}\n"
            ));
        }
    }
    s
}

/// El workflow, con **la versión del motor fijada**.
///
/// No es prudencia de más: `--version` lleva el commit porque `G1` promete que
/// el mismo árbol da el mismo digest **con el mismo motor**. Un CI que instalara
/// «la última» convertiría esa garantía en una coincidencia.
///
/// Y comprueba el binario que descarga. Un repositorio cuya tesis es *no te
/// creas lo que te llega* no puede traerse un ejecutable sin mirarlo.
fn flujo() -> String {
    let v = env!("CARGO_PKG_VERSION");
    format!(
        "# Generado por `ore init`.\n\
         #\n\
         # Las dos comprobaciones que hacen que las garantías estén EXIGIDAS y no solo\n\
         # disponibles. Un repositorio que las lleva y no las corre las tiene apuntadas.\n\
         name: ontologia\n\
         \n\
         on: [push, pull_request]\n\
         \n\
         env:\n\
         \x20 # Fijada, no «la última». El digest es función del árbol Y DEL MOTOR: `G1`\n\
         \x20 # promete que el mismo commit da el mismo digest con el mismo `ore`, y por\n\
         \x20 # eso `ore --version` lleva el commit dentro. Con «latest» la garantía sería\n\
         \x20 # una coincidencia que se rompe el día que salga una versión.\n\
         \x20 ORE_VERSION: \"{v}\"\n\
         \n\
         jobs:\n\
         \x20 comprobar:\n\
         \x20   runs-on: ubuntu-latest\n\
         \x20   steps:\n\
         \x20     - uses: actions/checkout@v4\n\
         \n\
         \x20     - name: instalar ore\n\
         \x20       run: |\n\
         \x20         base=https://github.com/describeloai/ore/releases/download/v$ORE_VERSION\n\
         \x20         archivo=ore-$ORE_VERSION-x86_64-unknown-linux-musl\n\
         \x20         curl -sSLo \"$archivo\" \"$base/$archivo\"\n\
         \x20         curl -sSLO \"$base/SHA256SUMS\"\n\
         \x20         # Se comprueba lo que se descarga. Un repositorio cuya tesis es que\n\
         \x20         # el origen no tiene que ser de confianza no puede traerse un\n\
         \x20         # ejecutable sin mirarlo.\n\
         \x20         grep \" $archivo$\" SHA256SUMS | sha256sum -c -\n\
         \x20         chmod +x \"$archivo\"\n\
         \x20         sudo mv \"$archivo\" /usr/local/bin/ore\n\
         \n\
         \x20     - name: el lock está al día\n\
         \x20       # `--check` y no `ore lock`: en CI hace falta saber que el lock quedó\n\
         \x20       # atrás SIN tocar el árbol. Uno que se arregla solo al mirarlo no se\n\
         \x20       # distingue de uno al día.\n\
         \x20       run: ore lock --check .\n\
         \n\
         \x20     - name: la ontología valida\n\
         \x20       run: ore validate .\n"
    )
}

fn agentes(nombre: &str, deps: &[Par]) -> String {
    let importa = if deps.is_empty() {
        "Este repositorio no importa ningún vocabulario todavía, así que **nada de lo que\n\
         hay está clasificado**. Sin un retículo —propio o importado— no hay escala con la\n\
         que decir que un dato es más sensible que otro, y `ore validate` no puede\n\
         gobernar lo que no tiene con qué comparar.\n"
            .to_string()
    } else {
        let lista: Vec<String> = deps.iter().map(|(c, r)| format!("- `{c}` `{r}`")).collect();
        format!(
            "Este repositorio **importa su clasificación** en vez de inventarla:\n\n{}\n\n\
             Eso significa que la definición de lo que aquí se importa **no es de esta\n\
             organización**. Cambiarla no se hace editando lo importado: se hace subiendo\n\
             de versión, y `ore lock` dirá qué propiedades tuyas se mueven con ella.\n",
            lista.join("\n")
        )
    };
    format!(
        "# `{nombre}` · repositorio ontológico\n\
         \n\
         Generado por `ore init`. Se compila con [`ore`](https://github.com/describeloai/ore),\n\
         que lee este árbol y no habla con nadie: sin red, sin reloj y sin credenciales.\n\
         \n\
         ## Qué es cada cosa\n\
         \n\
         | | |\n\
         |---|---|\n\
         | `ontology.config.yaml` | el manifiesto: qué fuentes hay y de qué se depende |\n\
         | `packages/` | los paquetes de este repositorio, uno por dominio |\n\
         | `vendor/` | lo importado, **comprometido a git a propósito** |\n\
         | `ontology.lock` | generado. Fija el digest de cada dependencia |\n\
         \n\
         ## Las órdenes\n\
         \n\
         ```bash\n\
         ore source add --name <fuente> <url>   # declara una fuente; el secreto va a .env.local\n\
         ore discover --source <fuente> --out packages/<x>   # induce entidades y bindings\n\
         ore review packages/<x>                # contesta lo que la inducción no supo decidir\n\
         ore lock .                             # resuelve las dependencias y fija sus digests\n\
         ore validate .                         # la ontología entera\n\
         ore diff <antes> <despues>             # clasifica un cambio por eje\n\
         ```\n\
         \n\
         ## Lo que no se puede romper\n\
         \n\
         **Un secreto no entra en un documento.** `ore source add` escribe la conexión en\n\
         `.env.local` y el manifiesto solo declara de qué variable sale. Es lo que hace\n\
         publicable este repositorio, y `OOS2012` lo comprueba.\n\
         \n\
         **`ontology.lock` no se edita a mano.** Es un artefacto generado, y lo que fija\n\
         —el digest de cada dependencia, quién la firmó, en qué log está— deja de\n\
         significar algo en cuanto alguien lo escribe en vez de computarlo.\n\
         \n\
         **`vendor/` no se ignora.** El reflejo es tratarlo como una carpeta de descargas.\n\
         Lo traído se compromete para que un clon recién hecho compile sin alcanzar a\n\
         nadie, y para que el digest de cada dependencia se revise en un pull request.\n\
         \n\
         **Omitir un `ConduitPolicy` no deja nada abierto: lo cierra.** Un conducto no\n\
         listado tiene autorización ⊥ y no admite nada. Hoy este repositorio no sirve nada\n\
         por ningún conducto, que es la postura correcta para uno recién creado.\n\
         \n\
         ## De dónde sale la clasificación\n\
         \n\
         {importa}"
    )
}

fn informe(nombre: &str, deps: &[Par], claves: &[Par], logs: &[Par]) -> String {
    let mut s = format!(
        "  ✓ {CONFIG} · {nombre} 0.1.0\n\
         \x20 ✓ {PAQUETES}/ · donde el compilador busca los paquetes\n\
         \x20 ✓ {IGNORAR} · el secreto y la caché fuera; `vendor/` NO\n\
         \x20 ✓ {FLUJO} · `lock --check` y `validate` en cada empujón\n\
         \x20 ✓ {AGENTES} · qué es este sitio y qué no se puede romper\n"
    );
    if deps.is_empty() {
        s.push_str(
            "\n  Sin clasificación todavía, y **no se puede inventar**:\n\
             \n\
             \x20   un retículo         una escala de sensibilidad que alguien sostiene\n\
             \x20   o `--depend`        acogerse a la de otro, que es lo normal para el RGPD\n\
             \n\
             \x20 Sin una de las dos, `ore validate` no tiene con qué comparar.\n",
        );
    } else {
        s.push_str("\n  Clasificación importada, no inventada:\n\n");
        for (c, r) in deps {
            s.push_str(&format!("    {c} {r}\n"));
        }
        let comprobado = match (claves.is_empty(), logs.is_empty()) {
            (true, _) => {
                "  Sin `--trust`, una firma que traiga no se comprueba: no hay con qué.\n\
                 \x20 No es un fallo —un paquete sin firmar se usa igual— pero conviene\n\
                 \x20 saberlo antes de que importe.\n"
            }
            (false, true) => {
                "  Su firma se comprobará. Sin `--trust-log`, lo que no se comprueba es\n\
                 \x20 que quien firma no le haya dicho otra cosa a otro en privado.\n"
            }
            (false, false) => {
                "  Se comprobarán su firma y su presencia en el log: de quién es, y que\n\
                 \x20 esa clave no ha dicho dos cosas distintas.\n"
            }
        };
        s.push_str(&format!("\n{comprobado}"));
        s.push_str("\n  ore lock .\n");
    }
    s.push_str("\n  ore validate .\n");
    s
}

/// El nombre de un paquete es `^[a-z][a-z0-9-]*$`. Del directorio se toma lo que
/// encaje: `Pedidos 2024` da `pedidos-2024`.
fn derivar_nombre(raiz: &Path) -> Option<String> {
    let bruto = raiz
        .canonicalize()
        .ok()
        .as_deref()
        .or(Some(raiz))
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))?;
    let s: String = bruto
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    let s = {
        let mut out = String::new();
        let mut guion = false;
        for c in s.chars() {
            if c == '-' {
                guion = true;
                continue;
            }
            if guion && !out.is_empty() {
                out.push('-');
            }
            guion = false;
            out.push(c);
        }
        out
    };
    nombre_valido(&s).then_some(s)
}

fn nombre_valido(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|p| p.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '-')
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn en(nombre: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ore-init-{nombre}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn con<'a>(nombre: &'a str, depende: &'a [String]) -> Respuestas<'a> {
        Respuestas {
            nombre: Some(nombre),
            depende,
            ..Default::default()
        }
    }

    #[test]
    fn el_nombre_se_deriva_del_directorio() {
        assert_eq!(
            derivar_nombre(Path::new("/tmp/pedidos")).as_deref(),
            Some("pedidos")
        );
        assert_eq!(
            derivar_nombre(Path::new("/tmp/Pedidos 2024")).as_deref(),
            Some("pedidos-2024")
        );
        // Empezar por dígito no se arregla inventando una letra delante.
        assert_eq!(derivar_nombre(Path::new("/tmp/2024")), None);
    }

    #[test]
    fn lo_que_escribe_valida() {
        let d = en("valida");
        assert!(intentar(&d, &con("pedidos", &[])).is_ok());
        let texto = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        let diags = ore_core::validate_document(&d.join(CONFIG), &texto);
        assert!(
            diags.is_empty(),
            "el manifiesto recién creado no valida: {diags:?}"
        );
    }

    /// Y con las respuestas puestas también, que es el caso que importa: un
    /// manifiesto que solo valida vacío no serviría para lo que existe.
    #[test]
    fn lo_que_escribe_con_respuestas_valida() {
        let d = en("valida-con");
        let clave = "c".repeat(64);
        intentar(
            &d,
            &Respuestas {
                nombre: Some("pedidos"),
                depende: &["oos.dev/regulatory/gdpr@^0.1".into()],
                claves: &[format!("oos.dev={clave}")],
                logs: &[format!("oos.dev/log={clave}")],
            },
        )
        .unwrap();
        let texto = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        let diags = ore_core::validate_document(&d.join(CONFIG), &texto);
        assert!(diags.is_empty(), "{diags:?}\n{texto}");
        assert!(texto.contains("oos.dev/regulatory/gdpr"), "{texto}");
        assert!(texto.contains("trustedKeys"), "{texto}");
        assert!(texto.contains("trustedLogs"), "{texto}");
    }

    /// **La razón de ser de este comando.** Quien lo usa no abre un editor: la
    /// decisión entra por bandera y sale escrita.
    #[test]
    fn la_dependencia_entra_por_bandera_y_no_por_edicion() {
        let d = en("dependencia");
        intentar(
            &d,
            &con("pedidos", &["oos.dev/regulatory/gdpr@^0.1".into()]),
        )
        .unwrap();
        let texto = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        assert!(
            texto.contains("dependencies:")
                && texto.contains("{ package: oos.dev/regulatory/gdpr, version: \"^0.1\" }"),
            "{texto}"
        );
    }

    /// Un rango no se adivina: es lo que decide qué actualizaciones entran solas,
    /// y ponerle un valor por defecto sería tomar esa decisión en silencio.
    #[test]
    fn una_dependencia_sin_rango_no_se_acepta() {
        let d = en("sin-rango");
        let err = intentar(&d, &con("pedidos", &["oos.dev/regulatory/gdpr".into()])).unwrap_err();
        assert_eq!(err.0, 64);
        assert!(!d.join(CONFIG).exists(), "escribió el repositorio a medias");
    }

    /// Una clave mal copiada no da un error más tarde: da un repositorio donde
    /// ninguna firma casa, o uno donde nadie mira. Se ve al escribirla.
    #[test]
    fn una_clave_que_no_es_una_clave_no_se_escribe() {
        let d = en("clave-mala");
        let err = intentar(
            &d,
            &Respuestas {
                nombre: Some("pedidos"),
                claves: &["oos.dev=abc".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.0, 65);
        assert!(!d.join(CONFIG).exists(), "escribió el repositorio a medias");
    }

    /// Y lo que NO escribe importa igual: un retículo o un `ConduitPolicy`
    /// inventados saldrían con aspecto de valor por defecto sensato, que es la
    /// peor forma de inventar una decisión de gobierno. `--depend` no es una
    /// excepción: acogerse a la clasificación de otro no es inventar una.
    #[test]
    fn no_inventa_gobierno() {
        let d = en("gobierno");
        intentar(
            &d,
            &con("pedidos", &["oos.dev/regulatory/gdpr@^0.1".into()]),
        )
        .unwrap();
        let ficheros: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !ficheros.iter().any(|f| f.contains("lattice")),
            "{ficheros:?}"
        );
        assert!(
            !ficheros.iter().any(|f| f.contains("conduit")),
            "{ficheros:?}"
        );
    }

    /// El reflejo es ignorar `vendor/`, y sería lo contrario de lo que hace útil
    /// vendorizar: un clon recién hecho dejaría de compilar sin el obtenedor.
    #[test]
    fn el_gitignore_saca_el_secreto_y_deja_vendor_dentro() {
        let d = en("ignorar");
        intentar(&d, &con("pedidos", &[])).unwrap();
        let t = std::fs::read_to_string(d.join(IGNORAR)).unwrap();
        assert!(t.lines().any(|l| l.trim() == ".env.local"), "{t}");
        assert!(t.lines().any(|l| l.trim() == ".ore/"), "{t}");
        assert!(
            !t.lines().any(|l| l.trim().starts_with("vendor")),
            "ignoró `vendor/`:\n{t}"
        );
    }

    /// El workflow fija la versión del motor. Con «la última», `G1` —el mismo
    /// commit da el mismo digest— pasaría a depender de qué día corre el CI.
    #[test]
    fn el_flujo_fija_la_version_del_motor() {
        let d = en("flujo");
        intentar(&d, &con("pedidos", &[])).unwrap();
        let t = std::fs::read_to_string(d.join(FLUJO)).unwrap();
        assert!(
            t.contains(&format!("ORE_VERSION: \"{}\"", env!("CARGO_PKG_VERSION"))),
            "{t}"
        );
        // `releases/latest`, no «latest» a secas: `ubuntu-latest` es otra cosa y
        // hacía saltar esto por un motivo que no tiene que ver con lo que mide.
        assert!(
            !t.contains("releases/latest"),
            "instalaría «la última»:\n{t}"
        );
        // Y comprueba lo que descarga.
        assert!(t.contains("sha256sum -c -"), "{t}");
        assert!(t.contains("ore lock --check"), "{t}");
        assert!(t.contains("ore validate"), "{t}");
    }

    #[test]
    fn no_sobrescribe_un_repositorio_existente() {
        let d = en("existente");
        intentar(&d, &con("pedidos", &[])).unwrap();
        let err = intentar(&d, &con("otro", &[])).unwrap_err();
        assert_eq!(err.0, 73);
        // Y el manifiesto sigue diciendo lo que decía.
        let texto = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        assert!(texto.contains("name: pedidos"), "{texto}");
    }
}
