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

/// La única dependencia que se emite sin que nadie la pida.
///
/// # Por qué esta sí, cuando el retículo no
///
/// Porque **no clasifica nada**. `iso` da nombre a `countryCode`, `currencyCode`
/// y `languageTag`, y ninguno lleva etiquetas: no puede gobernar, no puede
/// clasificar mal un dato, y lo peor que hace es ofrecer un nombre. Un retículo
/// del RGPD sí decide —qué es sensible y cuánto— y por eso no se emite.
///
/// La otra mitad del argumento la dio un dataset de verdad: la columna
/// `cod_pais` aparecía en tres tablas y la cola solo sabía decir *«acúñalo»*,
/// cuando `iso.countryCode` **la lleva de sinónimo**. La respuesta correcta
/// existía y el repositorio no la tenía a mano.
///
/// **Y tiene un precio que conviene decir:** hasta que exista un registro que la
/// sirva, `ore lock` de un repositorio recién creado no la resuelve — dirá que no
/// está en el árbol y cómo traerla. `ore validate` sí pasa, porque `01-package`
/// §3.1 admite una dependencia declarada y sin resolver. Cualquier `--depend`
/// explícito sustituye esto entero.
const POR_DEFECTO: &[&str] = &["oos.dev/types/iso@^0.1"];

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
    let depende: Vec<String> = if r.depende.is_empty() {
        POR_DEFECTO.iter().map(|s| (*s).to_string()).collect()
    } else {
        r.depende.to_vec()
    };
    let dependencias = leer_dependencias(&depende)?;
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

    // Y el mapa: un directorio por cada cosa que esta organización escribe con su
    // propia mano, con un README que dice qué decide.
    for (dir, titulo, desde, cuerpo) in MAPA {
        escribir(
            &raiz.join(dir).join("README.md"),
            &format!(
                "# `{dir}` · {titulo}

Introducido por **{desde}**.

{cuerpo}"
            ),
        )?;
    }

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

/// **El mapa: un directorio por cada cosa que se escribe aquí a mano.**
///
/// La pregunta que contesta es la que se hace cualquiera al abrir un repositorio
/// recién creado y ver una carpeta: *¿qué puedo poner aquí?*. Un `.gitkeep` no
/// la contesta, y la alternativa —leer cuatro documentos de la especificación
/// para averiguar dónde va un `Ruleset`— es peor.
///
/// # La línea de qué entra y qué no
///
/// **Directorios para lo que vas a escribir; nada para lo que el silencio ya
/// decide.** Por eso no hay `conduits.yaml` ni `request.yaml`: los dos son
/// ficheros cuya AUSENCIA significa denegación por defecto, y crearlos vacíos
/// cambiaría una postura por un formulario a medio rellenar.
///
/// Tampoco están `entities/`, `bindings/` ni `concepts/`: esos los escribe
/// `discover` dentro de un paquete, y adelantarlos aquí insinuaría que van en la
/// raíz.
const MAPA: &[(&str, &str, &str, &str)] = &[
    (
        "lattices",
        "las escalas de clasificación",
        "v1alpha1",
        "Un `Lattice` es un orden: `none ⊑ low ⊑ medium ⊑ high`. Es lo que hace comparable
         una clasificación, y sin uno no hay nada que gobernar — una etiqueta suelta no dice
         si algo es más sensible que otra cosa.
         
         **Puede estar vacío.** Si este repositorio depende de un vocabulario, su retículo
         viene con él y la autoridad sobre el orden no es de aquí. Escribir uno propio es
         decir que sí lo es.
",
    ),
    (
        "rulesets",
        "el gobierno, que apunta por clasificación",
        "v1alpha3",
        "Un `Ruleset` dice qué debe sostenerse y **quién responde**. Apunta por
         clasificación —`atLeast: { gdpr.sensitivity: high }`— y no por nombre, que es la
         diferencia entre una regla que se pudre y una que no: lo que alguien clasifique
         mañana queda gobernado el mismo día.
         
         Vive en su propio documento, con su propio `owner`, porque quien responde del
         cumplimiento tiene que poder restringir el modelo SIN poder editarlo.
",
    ),
    (
        "policies",
        "la autorización, en Cedar",
        "v1alpha1",
        "OOS no define un lenguaje de autorización: las políticas **son** Cedar. Aquí van
         los `.cedar` y el `.cedarschema` que el compilador compara con lo que el paquete
         implica.
         
         El esquema es un artefacto **generado** que se compromete a git para que el tooling
         de Cedar funcione sin compilar. Puede quedar obsoleto, y `OOS2013` lo cobra.
",
    ),
    (
        "interfaces",
        "las formas: conjuntos nombrados por lo que tienen",
        "v1alpha4",
        "Una `Interface` nombra un conjunto de entidades por los conceptos que llevan, no
         por su nombre. Es lo que permite que una regla alcance a algo que todavía no
         existe cuando se escribe.
",
    ),
    (
        "functions",
        "la superficie de efecto",
        "v1alpha2",
        "Una `Function` es lo que se puede **causar**, el dual de lo que se puede saber. Su
         integridad SE COMPUTA de sus endosos: no admite `labels`, porque una afirmación
         sobre uno mismo no es una garantía.
",
    ),
    (
        "resolutions",
        "el efecto sobre la identidad",
        "v1alpha2",
        "Una `Resolution` dice cuándo dos registros son el mismo. Sus `strategies` son una
         **secuencia** y no un conjunto: la primera que casa gana, así que reordenarlas
         cambia qué se fusiona.
",
    ),
];

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
         ## Dónde va cada documento\n\
         \n\
         | `kind` | Dónde | Desde |\n\
         |---|---|---|\n\
         | `Lattice` | `lattices/` | v1alpha1 |\n\
         | `Entity` | `packages/<x>/entities/` | v1alpha1 |\n\
         | `Binding` | `packages/<x>/bindings/` | v1alpha1 |\n\
         | `ConduitPolicy` | `conduits.yaml` **en la raíz** | v1alpha1 |\n\
         | `RequestPolicy` | `request.yaml` **en la raíz** | v1alpha1 |\n\
         | `Function` | `functions/` | v1alpha2 |\n\
         | `Resolution` | `resolutions/` | v1alpha2 |\n\
         | `Ruleset` | `rulesets/` | v1alpha3 |\n\
         | `Property` (concepto) | `packages/<x>/concepts/` | v1alpha4 |\n\
         | `Interface` | `interfaces/` | v1alpha4 |\n\
         \n\
         Cada directorio lleva un `README.md` con qué decide lo que va dentro.\n\
         \n\
         `entities/`, `bindings/` y `concepts/` los escribe `ore discover` dentro del\n\
         paquete que se le pase en `--out`; los demás se escriben a mano.\n\
         \n\
         **`conduits.yaml` y `request.yaml` no existen todavía a propósito.** Los dos son\n\
         ficheros cuya ausencia significa denegación por defecto, así que crearlos vacíos\n\
         cambiaría una postura por un formulario a medio rellenar.\n\
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
    s.push_str("\n  Y un sitio para cada cosa que se escribe aquí a mano:\n\n");
    for (dir, titulo, desde, _) in MAPA {
        s.push_str(&format!("    {dir:<12} {titulo}  ·  {desde}\n"));
    }
    s.push_str(
        "\n  No hay `conduits.yaml` ni `request.yaml`, y es deliberado: su AUSENCIA ya\n\
         \x20 significa denegación por defecto. Crearlos vacíos cambiaría una postura\n\
         \x20 por un formulario a medio rellenar.\n",
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

    /// Y lo que NO escribe importa igual: un reticulo o un `ConduitPolicy`
    /// inventados saldrian con aspecto de valor por defecto sensato, que es la
    /// peor forma de inventar una decision de gobierno. `--depend` no es una
    /// excepcion: acogerse a la clasificacion de otro no es inventar una.
    ///
    /// Se mide por **documentos y no por nombres de fichero**. La version
    /// anterior buscaba «lattice» en los nombres, y empezo a fallar el dia que
    /// la plantilla gano un directorio `lattices/` con un README dentro — que no
    /// inventa nada: dice que iria ahi si alguien lo escribiera. Un guardian que
    /// confunde un mapa con una decision no guarda lo que dice guardar.
    #[test]
    fn no_inventa_gobierno() {
        let d = en("gobierno");
        intentar(
            &d,
            &con("pedidos", &["oos.dev/regulatory/gdpr@^0.1".into()]),
        )
        .unwrap();

        let mut pila = vec![d.clone()];
        let mut culpables = Vec::new();
        while let Some(dir) = pila.pop() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    pila.push(p);
                    continue;
                }
                let t = std::fs::read_to_string(&p).unwrap_or_default();
                if t.lines().any(|l| {
                    let l = l.trim();
                    l == "kind: Lattice" || l == "kind: ConduitPolicy"
                }) {
                    culpables.push(p.display().to_string());
                }
            }
        }
        assert!(
            culpables.is_empty(),
            "invento una decision de gobierno: {culpables:?}"
        );
    }

    /// Sin `--depend`, sale la de por defecto — y es la unica que se emite sin
    /// que nadie la pida, porque **no clasifica nada**.
    #[test]
    fn sin_banderas_importa_los_tipos_iso_y_nada_mas() {
        let d = en("por-defecto");
        intentar(
            &d,
            &Respuestas {
                nombre: Some("pedidos"),
                ..Default::default()
            },
        )
        .unwrap();
        let t = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        assert!(t.contains("oos.dev/types/iso"), "{t}");
        // Y NO un reticulo: eso si decide.
        assert!(!t.contains("regulatory/gdpr"), "{t}");
    }

    /// Un `--depend` explicito sustituye lo de por defecto entero. Quien nombra
    /// sus dependencias no quiere que le añadan una.
    #[test]
    fn una_dependencia_explicita_sustituye_la_de_por_defecto() {
        let d = en("sustituye");
        intentar(
            &d,
            &con("pedidos", &["oos.dev/regulatory/gdpr@^0.1".into()]),
        )
        .unwrap();
        let t = std::fs::read_to_string(d.join(CONFIG)).unwrap();
        assert!(t.contains("regulatory/gdpr"), "{t}");
        assert!(
            !t.contains("types/iso"),
            "añadio una que nadie pidio:
{t}"
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

    /// **La pregunta que contesta el mapa**: quien abre un repositorio recien
    /// creado ve carpetas y pregunta que puede poner en cada una. Un `.gitkeep`
    /// no lo contesta, y leer cuatro documentos de la especificacion para saber
    /// donde va un `Ruleset` es peor.
    #[test]
    fn cada_directorio_dice_que_decide_lo_que_va_dentro() {
        let d = en("mapa");
        intentar(&d, &con("pedidos", &[])).unwrap();
        for (dir, _, desde, _) in MAPA {
            let readme = d.join(dir).join("README.md");
            let t = std::fs::read_to_string(&readme)
                .unwrap_or_else(|_| panic!("falta `{dir}/README.md`"));
            assert!(t.contains(desde), "`{dir}` no dice desde que version: {t}");
        }
    }

    /// Y lo que NO se crea: `conduits.yaml` y `request.yaml` son ficheros cuya
    /// AUSENCIA significa denegacion por defecto. Crearlos vacios cambiaria una
    /// postura por un formulario a medio rellenar.
    #[test]
    fn no_crea_los_ficheros_cuya_ausencia_ya_decide() {
        let d = en("ausencia");
        intentar(&d, &con("pedidos", &[])).unwrap();
        assert!(!d.join("conduits.yaml").exists());
        assert!(!d.join("request.yaml").exists());
        // Y tampoco los que escribe `discover`, que van DENTRO de un paquete:
        // adelantarlos aqui insinuaria que van en la raiz.
        for x in ["entities", "bindings", "concepts"] {
            assert!(!d.join(x).exists(), "creo `{x}/` en la raiz");
        }
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
