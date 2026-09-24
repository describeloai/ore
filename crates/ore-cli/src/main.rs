//! La CLI de ORE.
//!
//! Tres caras bajo un solo binario, con fronteras de confianza distintas. La
//! columna que importa no es qué hace cada comando, sino **qué toca**.
//!
//! De los trece comandos implementados, **diez no invocan a nada**. Los tres que
//! delegan son `discover --source`, `lock` y `pack --sign/--log` — y ninguno de
//! los tres abre el socket: lo abre el programa que llaman.

mod activos;
mod alcance;
mod autoria;
mod cache;
mod candado;
mod datasets;
mod deriva;
mod empaquetar;
mod fuente;
mod inductor;
mod inicio;
mod invocar;
mod lector;
mod materializar;
mod mcp;
mod migrar;
mod paquete;
mod preguntar;
mod registro;
mod revision;
mod unidad_sql;
mod verificar;
mod vista;
mod vocabulario;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// La identidad del motor, y es mas que un numero de version.
///
/// Un bundle lleva `sha256:...` y **G1** promete que el mismo commit produce el
/// mismo digest. Quien audite ese bundle tiene que poder contestar *cual motor
/// lo produjo*, y `ore 0.1.0` contesta *alguna compilacion de la 0.1.0* — que
/// para una garantia de determinismo no vale.
///
/// El commit entra por **variable de entorno al compilar**, no por un `build.rs`
/// que invoque a git. La diferencia no es de comodidad: asi una compilacion
/// local dice honestamente que **no viene de un commit conocido**, en vez de
/// sellar el hash de un arbol que puede estar sucio. Un binario que miente sobre
/// su procedencia tiene exactamente el mismo aspecto que uno que no.
///
/// Y las versiones de OOS **se derivan** de `ApiVersion::ALL` (P2): una lista
/// escrita a mano aqui envejeceria en silencio la primera vez que el motor
/// aprendiera una version nueva.
fn version() -> String {
    let commit = option_env!("ORE_COMMIT").unwrap_or("sin sellar");
    let oos: Vec<&str> = ore_core::document::ApiVersion::ALL
        .iter()
        .map(|v| v.as_str())
        .collect();
    format!(
        "{} ({commit})
OOS: {}",
        env!("CARGO_PKG_VERSION"),
        oos.join(" · ")
    )
}

#[derive(Parser)]
#[command(
    name = "ore",
    about = "Ontology Runtime Engine — compila, coteja y sirve un repositorio ontológico",
    long_about = None,
    version = version()
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Lo que se puede hacer con una fuente. Hoy solo darla de alta; `list` y
/// `remove` esperan a tener más de una cosa que decir que la que ya dice el
/// manifiesto, que se lee.
/// Lo que se puede hacer con una vista mas alla de mirarla.
#[derive(Subcommand)]
enum AccionVista {
    /// **Autora una pregunta nueva sobre un hecho.**
    ///
    /// El tercer acto: `discover` espeja, `review` decide lo que la induccion
    /// no supo decidir, y esto escribe una vista que nadie propuso. Usa el
    /// MISMO emisor que el inductor, para que una vista autorada y una
    /// inducida sean el mismo texto.
    ///
    /// `fields` empieza con TODAS las columnas del origen y se restan: quitar
    /// una es una decision visible y olvidarse de anadir una no lo es.
    Add {
        /// Como se llama esta pregunta. **No se deriva**: el nombre derivado ya
        /// lo cogio la vista que el inductor propuso por el objeto.
        nombre: String,
        /// La tabla o la vista de la que sale. Cualificado o corto.
        #[arg(long = "from", value_name = "TABLA|VISTA")]
        de: String,
        /// `propiedad=columna`, o solo `columna`. Repetible. Sin ninguno, van
        /// todas las del origen.
        #[arg(long = "field", value_name = "PROP=COL")]
        campos: Vec<String>,
        /// `columna=valor`. Repetible; el mismo nombre dos veces es una lista.
        #[arg(long = "where", value_name = "COL=VALOR")]
        recorte: Vec<String>,
        /// Quien responde. Sin el se escribe `cambiame`, que NO valida.
        #[arg(long)]
        owner: Option<String>,
        /// Raiz del paquete donde vive el origen.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum AccionFuente {
    /// Da de alta una fuente: el secreto va a `.env.local` y el manifiesto solo
    /// declara de qué variable sale.
    Add {
        /// Nombre con el que las tablas la referenciarán (`spec.datasource`).
        #[arg(long)]
        name: String,
        /// Cadena de conexión completa. **No** se escribe en ningún documento OOS.
        url: String,
        /// Driver. Por defecto se deriva del esquema de la URL.
        #[arg(long = "type", value_name = "DRIVER")]
        tipo: Option<String>,
        /// Variable de entorno. Por defecto, del manifiesto más el nombre.
        #[arg(long, value_name = "VAR")]
        env: Option<String>,
        /// Etiqueta que hereda todo lo enlazado a esta fuente. Repetible.
        #[arg(long = "label", value_name = "CLAVE=VALOR")]
        label: Vec<String>,
        /// Para qué es esta fuente.
        #[arg(long, value_name = "TEXTO")]
        description: Option<String>,
        /// Raíz del repositorio ontológico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// **Retirar una fuente del árbol.** El inverso de `add`, que hasta hoy no
    /// existía.
    ///
    /// Un día de pruebas dejó veintiuna fuentes declaradas en un árbol y
    /// dieciocho eran intentos fallidos. Quitarlas era editar a mano el
    /// manifiesto de un cliente — justo lo que `add` existe para evitar.
    ///
    /// ⭐ Y no es simetría por simetría: el reconciliador rinde un Job de
    /// catálogo por cada fuente SIN paquete, así que una fuente basura no es una
    /// línea de más — es trabajo que se encola una y otra vez.
    ///
    /// ⛔ NO borra la credencial del custodio ni el paquete. La primera porque
    /// esta CLI no habla con el cofre y dárselo para esto sería pagar con la
    /// propiedad más cara del binario; el segundo porque es trabajo hecho.
    Remove {
        /// La fuente, tal como la declara el manifiesto.
        name: String,
        /// Raíz del repositorio ontológico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// No tocar `.env.local`. Para el caso raro de que la variable la
        /// comparta otra fuente declarada a mano.
        #[arg(long = "keep-secret")]
        keep_secret: bool,
    },
    /// **¿Qué contiene esta fuente?** Lo que habría que declarar, antes de
    /// declararlo.
    ///
    /// Existe por una asimetría real y no por simetría: una URL de BigQuery
    /// nombra **un dataset**, así que hasta hoy había que sabérselo de
    /// antemano y la única forma de comprobar que existía era fallar al
    /// descubrirlo. Las otras dos familias abarcan su fuente entera, y lo
    /// contestan diciendo justamente eso.
    Explore {
        /// La fuente, tal como la declara el manifiesto. Puede no tener
        /// contenedor: `bigquery://<proyecto>` se explora y no se descubre.
        name: String,
        /// Raíz del repositorio ontológico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// **¿Responde esta fuente?** Y nada más: no lee el catálogo, no propone
    /// nada y no toca un fichero.
    ///
    /// Los otros mandos preguntan por la fuente **haciendo un trabajo**, así
    /// que una credencial caducada se descubría diciendo «el catálogo no
    /// analiza». Son dos preguntas y fallan por separado, así que se piden por
    /// separado — la misma figura que `source add` con el secreto y `discover`
    /// con leer y proponer.
    Check {
        /// La fuente, tal como la declara el manifiesto.
        name: String,
        /// Raíz del repositorio ontológico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// **¿Qué tiene dentro esta fuente?** El catálogo, y para.
    ///
    /// Es el primer acto de `discover` sin el segundo. `discover --from` acepta
    /// «un catálogo ya leído, venga de donde venga» y hasta ahora ningún mando
    /// emitía uno, así que el artefacto de la frontera no se podía tener en la
    /// mano.
    ///
    /// Sin `--out` sale por stdout y nada más sale por stdout, para poder
    /// redirigirlo y dárselo a `--from` tal cual.
    Catalog {
        /// La fuente, tal como la declara el manifiesto.
        name: String,
        /// Dónde escribirlo. Sin esto, a stdout.
        #[arg(long, value_name = "FICHERO")]
        out: Option<PathBuf>,
        /// Raíz del repositorio ontológico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

/// Lo que se puede hacer con un paquete.
///
/// Hasta hoy un paquete **solo nacia descubriendo una fuente**: `ore init` deja
/// `packages/` vacio y el unico que escribia un `package.yaml` era el inductor.
#[derive(Subcommand)]
enum AccionPaquete {
    /// **Crea un paquete**: el manifiesto, y nada mas.
    ///
    /// Es un acto de gobierno —necesita dueno, version y estado— y por eso es un
    /// verbo aparte de mover documentos entre paquetes, que no lo es.
    ///
    /// Escribe `status: draft` y no `active`: `01-package` §2.3 deriva de ahi la
    /// madurez POR DEFECTO de lo que el paquete contenga, y uno recien creado no
    /// contiene nada. Llamarlo `active` seria afirmar STABLE sobre lo que no
    /// existe.
    ///
    /// Y no crea `views/` ni `tables/`: un directorio vacio no viaja en git.
    New {
        /// El nombre, que **es el espacio de nombres** de todo lo que contenga.
        /// Un nombre con puntos o guiones es legal como coordenada de
        /// importacion y no puede ser un `namespace`: se rechaza.
        name: String,
        /// Quien responde. Sin el se escribe `cambiame`, que NO valida.
        #[arg(long)]
        owner: Option<String>,
        /// El dominio de negocio. Por defecto, el nombre.
        #[arg(long)]
        domain: Option<String>,
        /// Raiz del repositorio ontologico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// **Mueve un documento a otro paquete**: las tres cosas a la vez.
    ///
    /// Mueve el fichero, reescribe su `namespace` y ANUNCIA el movimiento en el
    /// manifiesto de origen —sin ese anuncio, el nombre que desaparece es un
    /// `OOS5007`—. Y reapunta lo que lo nombraba, incluida la forma corta: un
    /// documento que compartia espacio con el lo llamaba a secas.
    ///
    /// Lo que NO toca es `exports`. Es «esto lo expongo a proposito», y un
    /// mando que ensancha la superficie publica por su cuenta contradice la
    /// frase para la que esa lista existe: dice que hace falta y no lo decide.
    Move {
        /// El nombre cualificado del documento: `<paquete>.<nombre>`.
        qname: String,
        /// El paquete al que va. Tiene que existir — `ore package new`.
        #[arg(long = "to", value_name = "PAQUETE")]
        a: String,
        /// La version desde la que el nombre viejo deja de estar. Por defecto,
        /// la que el paquete de origen declara hoy.
        #[arg(long)]
        since: Option<String>,
        /// Raiz del repositorio ontologico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// **¿En que piezas se parte este paquete?** Y, con `--to`, las mueve.
    ///
    /// SIN `--to` no mueve nada: enumera las COMPONENTES —los grupos de
    /// documentos que se nombran entre si— y dice si el corte sale gratis. Es
    /// el mismo reparto que `drift-detect`: ensenar y parar es un acto, mover
    /// es otro, y fallan por separado.
    ///
    /// Y la respuesta tiene dos mitades, medidas. Sobre un paquete RECIEN
    /// DESCUBIERTO la clausura es exacta y sale gratis —30 documentos en 10
    /// componentes de 3, y mover una entera deja CERO referencias cruzando—.
    /// Sobre uno MODELADO no hay corte gratis: el mas barato cuesta un cruce,
    /// y entonces esto dice el precio en vez de buscarlo.
    Split {
        /// El paquete que se parte.
        paquete: String,
        /// Que documento se lleva. Repetible.
        #[arg(long = "con", value_name = "QNAME")]
        con: Vec<String>,
        /// A donde. Sin esto, solo enumera.
        #[arg(long = "to", value_name = "PAQUETE")]
        a: Option<String>,
        /// La version desde la que los nombres viejos dejan de estar.
        #[arg(long)]
        since: Option<String>,
        /// Raiz del repositorio ontologico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// **Funde un paquete en otro**, dejando una LAPIDA.
    ///
    /// El paquete de origen NO desaparece: se queda `status: retired`, sin
    /// documentos, y con un `moved` por cada uno de los que se fueron. Se midio:
    /// asi `ore diff` lo llama compatible, y sin el anuncio da `OOS5007`.
    ///
    /// Las colisiones de nombre SE NIEGAN: fundir encima perderia uno de los
    /// dos, y cual se pierde no lo decide una herramienta.
    Merge {
        /// El paquete que se funde y queda como lapida.
        origen: String,
        /// En cual.
        #[arg(long = "into", value_name = "PAQUETE")]
        a: String,
        /// La version desde la que los nombres viejos dejan de estar.
        #[arg(long)]
        since: Option<String>,
        /// Raiz del repositorio ontologico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

/// Lo que se puede hacer con la cache. Hoy solo preguntarle si sirve:
/// **escribirla no es nuestro**, porque las filas viven en una tabla del lago
/// del cliente y quien las escribe es quien tiene el driver (ADR 0006).
#[derive(Subcommand)]
enum AccionCache {
    /// Contesta si lo materializado puede servir una consulta, y si no, por que.
    Check {
        /// El manifiesto de cache.
        #[arg(long, value_name = "FICHERO")]
        manifest: std::path::PathBuf,
        /// La entidad cualificada por la que se pregunta.
        #[arg(long, value_name = "QNAME")]
        entity: String,
        /// Las propiedades que la consulta necesita.
        #[arg(long, value_name = "A,B", value_delimiter = ',')]
        props: Vec<String>,
        /// La version de topologia con la que el plan resolvio las claves.
        ///
        /// Se teclea porque leer un artefacto es de otro binario y `ore` no
        /// enlaza contra ninguno. Sin esta bandera se entiende que el plan no
        /// hizo travesia, y entonces la topologia de la cache no le concierne.
        ///
        /// **Hoy nadie produce esa cadena**: el artefacto `ORETOPO1` lo escribia
        /// `ore-exec`, que se retiro. Ver la cabecera de `cache.rs`.
        #[arg(long, value_name = "SHA256")]
        topology: Option<String>,
        /// Cuando se pregunta. **El motor no lee el reloj**: el instante llega
        /// de fuera, igual que en una respuesta.
        #[arg(long, value_name = "ISO8601")]
        at: Option<String>,
        /// El `freshnessSLA` que aplique: `30m`, `2h`, `7d`.
        #[arg(long, value_name = "DURACION")]
        sla: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

// Un `Command` se construye una vez al arrancar y se consume: que `Datasets`
// (los verbos del catálogo, W3.6c) pese 460 bytes y `Init` 40 no cuesta nada,
// y meter sus opciones en una caja sólo por el lint las alejaría de clap.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Command {
    // ── Scaffolder ──────── autoría · toca metadatos de producción y, si se pide, un LLM
    /// Crea el esqueleto de un repositorio ontológico.
    ///
    /// No escribe un retículo ni un `ConduitPolicy`: son decisiones de gobierno
    /// y este comando no las tiene. Omitir el conducto además YA significa algo
    /// —autorización ⊥, denegación por defecto—, así que escribirlo no lo haría
    /// más cierto.
    Init {
        /// Nombre del repositorio. Por defecto, el del directorio.
        #[arg(long)]
        name: Option<String>,
        /// Vocabulario del que depender: `<coordenada>@<rango>`. Repetible.
        ///
        /// Es la salida que no inventa: en vez de escribir un retículo propio,
        /// el repositorio **se acoge al de otro**. Declarar una dependencia es
        /// transferir autoridad, y con ella el repo tiene clasificación desde el
        /// minuto cero sin que nadie de dentro haya decidido nada.
        #[arg(long, value_name = "COORDENADA@RANGO")]
        depend: Vec<String>,
        /// Clave con la que comprobar una firma: `<id>=<clavePublica>`. Repetible.
        #[arg(long, value_name = "ID=CLAVE")]
        trust: Vec<String>,
        /// Clave de un log de transparencia: `<id>=<clavePublica>`. Repetible.
        #[arg(long = "trust-log", value_name = "ID=CLAVE")]
        trust_log: Vec<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Registra una fuente física, separando la credencial de la conexión.
    #[command(name = "source", subcommand)]
    Source(AccionFuente),
    /// Crea y organiza paquetes: el manifiesto, y lo que contiene.
    #[command(name = "package", subcommand)]
    Package(AccionPaquete),
    /// Espeja una fuente en tablas y propone entidades y vistas en DRAFT.
    ///
    /// Son dos actos: **leer** un catálogo y **proponer** una ontología, y se
    /// piden por separado porque fallan por separado. `--source` lee de una
    /// fuente declarada; `--from` toma un catálogo ya leído, venga de donde
    /// venga. Lo que produce el primero es exactamente lo que acepta el segundo.
    Discover {
        /// Un catálogo en JSON: columnas, tipos y claves de un origen.
        #[arg(long, conflicts_with = "source", required_unless_present = "source")]
        from: Option<PathBuf>,
        /// El nombre de una fuente declarada en `ontology.config.yaml`.
        #[arg(long)]
        source: Option<String>,
        /// Dónde se escribe el paquete inducido.
        #[arg(long)]
        out: PathBuf,
        /// Nombre y espacio de nombres del paquete. Por defecto, el del directorio.
        #[arg(long)]
        name: Option<String>,
        /// **Solo este objeto del origen.** Repetible. Sin ninguno, entra todo.
        ///
        /// El nombre es el que el catalogo le da, cualificado como lo cualifique
        /// el lector: `public.clientes` en PostgreSQL. No se normaliza —lo que
        /// se compara es lo que el origen dijo—, y un nombre que el catalogo no
        /// tiene se dice en vez de tragarse: o es una errata o la tabla ya no
        /// esta, y las dos piden que alguien mire.
        #[arg(long = "only", value_name = "OBJETO")]
        solo: Vec<String>,
        /// Lo mismo, de un fichero: un objeto por linea, `#` para anotar.
        ///
        /// Existe porque **cien tablas no caben en una linea de ordenes**, y
        /// menos en la de un `Job` que la lleva escrita en un manifiesto.
        #[arg(long = "only-file", value_name = "FICHERO")]
        solo_de: Option<PathBuf>,
        /// **La clase de la base** (ORE 0027 P1 I4): `standard` copia a la celda
        /// todo lo que entra —cada tabla sale con su `Dataset` (0033)—;
        /// `foreign` (lo de siempre, y lo que se entiende si
        /// falta) es un espejo. Pide un alcance (`--only`): una base es lo que
        /// se elige.
        #[arg(long = "type", value_name = "standard|foreign")]
        tipo: Option<String>,
        /// **El catálogo no modela** (ORE 0027 P1 C1): con esto, ninguna tabla
        /// del alcance lleva `Entity` ni cola de modelado — sólo `Table` y
        /// `View`. Se modela después, una a una, con `ore model`.
        #[arg(long = "no-model", requires = "tipo")]
        sin_modelar: bool,
        /// Modela sólo estas tablas del alcance. Repetible. Sin esto ni
        /// `--no-model`, se modelan todas (lo de siempre).
        #[arg(long = "model", value_name = "OBJETO", conflicts_with = "sin_modelar")]
        modelar: Vec<String>,
        /// Quien responde del paquete (`team:<handle>` | `user:<handle>`): es la
        /// decision `dueno`, contestada por quien llama. Sin el se escribe
        /// `cambiame`, que NO valida, y la decision queda en la cola.
        #[arg(long, value_name = "HANDLE")]
        owner: Option<String>,
    },
    /// **Modelar una tabla de una base**: la añade a `entities` del alcance y
    /// vuelve a inducir. Es lo que un catálogo de activos llama *promote to
    /// object type*: la tabla gana su `Entity` y sus decisiones (la clave, las
    /// relaciones, los conceptos), y su copia, si la base es estándar, pasa a
    /// esperar la clave.
    Model {
        /// El paquete de la base: donde `discover --only` escribió.
        path: PathBuf,
        /// El objeto físico, como lo nombra el catálogo: `public.pedidos`.
        objeto: String,
    },
    /// **Copiar una tabla de una base foránea** a la celda: la añade a `copies`
    /// del alcance y vuelve a inducir. La excepción a la clase: la base sigue
    /// siendo foránea, esa tabla se copia. En una estándar no hace falta.
    Copy { path: PathBuf, objeto: String },
    /// Escribe el paquete publicable: un `.oob`.
    ///
    /// No es un archivo comprimido, y esa es la decisión: uno lleva marcas de
    /// tiempo y orden de entradas, así que el mismo paquete daría bytes
    /// distintos. Un `.oob` es **la forma canónica escrita en un fichero**, y su
    /// digest es el del paquete — el contenedor no cambia la identidad.
    Pack {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Dónde se escribe. Sin esto, el `.oob` sale por stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Firma el paquete con esta clave, delegando en `ore-sign`.
        ///
        /// `ore` no toca una clave privada: construye el enunciado —coordenada
        /// y digest— y lo manda a firmar fuera. Verificar sí vive dentro, y esa
        /// asimetría es la que permite parar ante una firma que no case sin
        /// haberle confiado nada a nadie.
        #[arg(long, value_name = "KEY_ID")]
        sign: Option<String>,
        /// Anota el paquete firmado en este log, delegando en `ore-log`.
        ///
        /// Exige `--sign`: lo que se anota es el enunciado **y quién lo firmó**.
        /// Una firma dice de quién es un paquete; el log es lo que impide que
        /// esa clave le diga cosas distintas a dos personas en privado.
        #[arg(long, value_name = "LOG_ID", requires = "sign")]
        log: Option<String>,
    },
    /// Resuelve `dependencies` y escribe `ontology.lock`.
    ///
    /// Contra **el árbol**, no contra un registro: `ore` no sabe hablar por la
    /// red. Un paquete se resuelve si está vendorizado como miembro del
    /// workspace, y si no, esto falla en vez de inventar una entrada.
    Lock {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Comprueba sin escribir. En CI hace falta saber que el lock quedó
        /// atrás **sin** tocar el árbol: uno que se arregla solo al mirarlo no
        /// se distingue de uno al día.
        #[arg(long)]
        check: bool,
    },
    /// Cola interactiva de decisiones para lo que el descubrimiento no supo clasificar.
    ///
    /// No edita lo inducido: **vuelve a inducir** el catálogo que `discover` dejó
    /// al lado, esta vez con las decisiones tomadas. Por eso es puro igual que el
    /// inductor —sin red, sin credenciales, sin driver— y por eso contestar dos
    /// veces lo mismo produce el mismo paquete byte a byte.
    Review {
        /// El paquete inducido: donde `discover` escribió.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Contesta en diferido, sin terminal. Es lo que permite probar esto en
        /// CI, y una cola que solo se contesta a mano no se prueba.
        #[arg(long, value_name = "FICHERO")]
        answers: Option<PathBuf>,
        /// Vuelve a inducir aunque no haya respuesta nueva: para cuando lo que
        /// cambió es la REGLA (la clase del alcance), no las respuestas.
        #[arg(long)]
        reinducir: bool,
    },
    /// **¿Qué se movió en el origen desde que se declaró?** Enseña y para.
    ///
    /// `ore diff` contesta «¿quién se rompe?» —una relación entre dos
    /// versiones— y esto contesta «¿qué se movió?» —una relación entre el mundo
    /// y lo dicho—. Una columna nueva que ninguna vista proyecta es invisible
    /// para `diff`, y está bien que lo sea: es justo la mitad que esto cuenta.
    ///
    /// Compara el catálogo del origen contra las `kind: Table` del paquete, que
    /// son la declaración del plano físico. Lo de gobierno —quién responde, la
    /// madurez, la frescura, las etiquetas— no se mira: compararlo daría deriva
    /// en todos los paquetes gobernados, siempre.
    ///
    /// **Sale con `2` si hay deriva y con `0` si no.** No es un `sysexit`, y es
    /// a propósito: es la convención de `terraform plan -detailed-exitcode`, y
    /// existe para que esto entre en un pipeline sin parsear su salida. Los
    /// errores usan los `sysexits` del resto, para que «no pude preguntar» y
    /// «el origen cambió» no se confundan nunca.
    ///
    /// Y no corrige nada. Escribir la corrección es otro acto.
    #[command(name = "drift-detect")]
    DriftDetect {
        /// La fuente declarada a la que preguntar.
        #[arg(long, conflicts_with = "from")]
        source: Option<String>,
        /// Un catálogo ya leído. **Es el que hace esto probable**: un aserto
        /// que exigiera un servidor no se ejecutaría nunca en la suite.
        #[arg(long, value_name = "FICHERO")]
        from: Option<PathBuf>,
        /// Raíz del repositorio ontológico.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },

    // ── Compilador ──────── CI · hermético: sin red, sin credenciales, sin reloj
    /// Comprueba consistencia de reglas, tipados y políticas.
    Lint {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Valida contra OOS: esquema, integridad referencial y flujo de información.
    Validate {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Ejecuta los casos de prueba semánticos del paquete.
    Test {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Compara dos versiones y clasifica los cambios por eje.
    ///
    /// Es el único comando que toma DOS entradas: la clasificación de un cambio
    /// no es una propiedad de un paquete, es una relación entre dos.
    ///
    /// Cada entrada es un árbol **o** un `.oob`, y se pueden mezclar: comparar
    /// lo que tienes con lo que vendría es la pregunta entera de una
    /// actualización, y lo que vendría se publica empaquetado.
    Diff { before: PathBuf, after: PathBuf },
    /// Muestra el delta semántico antes de aplicarlo.
    Plan {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// El registro de qué gobierna qué, y quién responde.
    ///
    /// No es una lista de incumplimientos y no puede serlo: una propiedad sin
    /// la clase que exige su clasificación no compila. La pregunta que contesta
    /// es la otra — **quién responde, y por qué vía**.
    Report {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Lo que el motor de vistas dice de cada `kind: View` del paquete.
    ///
    /// Plan e identidad, esquema, linaje por columna —con la arista INDIRECT,
    /// la del `where`, que `validate` no mira—, modo de refresco, qué empuja al
    /// origen, y si la copia compila. Todo desde el árbol de ficheros: no
    /// ejecuta, no mide, no abre nada.
    View {
        /// `add` autora una vista nueva; sin subcomando, informa.
        ///
        /// Clap prefiere el subcomando cuando el primer token coincide con
        /// su nombre, asi que `ore view <ruta>` sigue funcionando. Un
        /// directorio que se llamara literalmente `add` seria ambiguo, y es un
        /// precio que se paga para no romper el mando que ya existia.
        #[command(subcommand)]
        accion: Option<AccionVista>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Coteja una **propuesta** contra el paquete que la autoriza.
    ///
    /// Una funcion no aplica, propone: devuelve una `Propuesta` con qué
    /// escribir y **bajo qué lo decidio** —las cinco identidades—, y lo que
    /// devuelve un delegado no se cree. Esto contesta si cae dentro de lo que
    /// `effects:` autorizaba, si el significado sigue vigente, y si se sigue
    /// entrando por la misma vista.
    ///
    /// No ejecuta la funcion, no abre el origen y no escribe. Que sea
    /// contestable sin runtime es lo que hace que el simulacro salga gratis:
    /// **la propuesta ES el simulacro**.
    ///
    /// No es `validate`: aquel juzga si un paquete compila, este si una
    /// propuesta sobre el es aceptable. Un paquete puede compilar y una
    /// propuesta sobre el no aplicarse, que es el caso que existe para atrapar.
    Verify {
        /// El artefacto que devolvio quien invoco la funcion.
        propuesta: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Puebla los datasets mantenidos (0033): el ciclo entero del ADR 0015.
    ///
    /// Compila el plan, comprueba el flujo, pregunta al almacen si la copia ya
    /// esta —y si esta, **no lee ni una fila del origen**—, y si no, canaliza
    /// las filas de `ore-read-<tipo>` a `ore-store-<r2|gcs>` (`ORE_STORE`). `ore` no abre un socket
    /// en ningun momento: esta en medio de dos procesos.
    Materialize {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Dice que haria y no lo hace. No lee el origen ni escribe nada.
        #[arg(long)]
        seco: bool,
        /// Borra las copias del mismo plan que quedaron atras.
        ///
        /// **Explicito y no automatico**: una copia superada sigue siendo cierta
        /// hasta su marca, y alguien puede estar leyendola por su digest.
        #[arg(long)]
        recoger: bool,
        /// Escribe un informe por vista (`<DIR>/<paquete>_<vista>.json`): que
        /// copia hay, cuantas filas, con que testigo. Para quien no alcanza el
        /// almacen (0027 P1 I3).
        #[arg(long, value_name = "DIR")]
        informe: Option<PathBuf>,
        /// No pregunta al recibo: lee el origen entero y deja el recibo
        /// apuntando a la copia nueva (la superada se borra). Para cuando
        /// cambia COMO se lee, o el testigo no se mueve aunque los datos si.
        #[arg(long)]
        rehacer: bool,
        /// Solo estas vistas (`paquete.vista`, repetible). Sin esto, todas
        /// las que declaran copia.
        #[arg(long, value_name = "NS.VISTA")]
        vista: Vec<String>,
    },
    /// Invoca una `Function` de lectura (`runtime: model`, `over`, `output`,
    /// sin `effects`) sobre la COPIA de `over`: `ore-store-<tipo> leer` trae
    /// las filas, `ore-invoke` las lleva al modelo por la puerta con el token
    /// de la celda, y el resultado se sella en el bucket del inquilino con un
    /// informe en el arbol (ADR 0029, F4a). `ore` no abre un socket: esta en
    /// medio de tres procesos.
    Invoke {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// La funcion, cualificada: `<paquete>.<nombre>`.
        #[arg(long, value_name = "NS.NOMBRE")]
        funcion: String,
        /// La puerta del modelo (`GET /modelos/{n}` → `url`); o `MODELO_URL`.
        #[arg(long)]
        puerta: Option<String>,
        /// El id que la puerta sirve (`GET /modelos/{n}` → `model`); o `MODELO_ID`.
        #[arg(long)]
        modelo: Option<String>,
        /// Escribe el informe de la corrida en `<DIR>/<ns>_<f>_<corrida>.json`.
        #[arg(long, value_name = "DIR")]
        informe: Option<PathBuf>,
        /// Solo las N primeras filas de la copia: para probar, y para pagar menos.
        #[arg(long)]
        limite: Option<usize>,
        /// Llamadas a la vez (E0 midio 4 sin degradar).
        #[arg(long, default_value_t = 4)]
        concurrencia: usize,
        /// Trae la copia y dice cuantas filas; no llama al modelo ni sella nada.
        #[arg(long)]
        seco: bool,
    },
    /// Ejecuta la pregunta de una vista sobre su copia, en la celda y sin
    /// abrir el origen (0030 W1).
    ///
    /// Compila el plan, decide que copia lo contesta (la suya, o una que lo
    /// implique, con compensacion), la trae por su nombre con
    /// `ore-store-<r2|gcs> leer`, tipa las filas por la cabecera y ejecuta
    /// el plan reescrito. Cabecera y filas por stdout; no escribe nada.
    Ask {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// La vista, cualificada: `<paquete>.<nombre>`.
        #[arg(long, value_name = "NS.NOMBRE")]
        vista: String,
        /// Solo las N primeras filas DE LA RESPUESTA (el plan se ejecuta entero).
        #[arg(long)]
        limite: Option<u64>,
        /// Decide quien contesta y con que; no trae ni ejecuta.
        #[arg(long)]
        seco: bool,
    },
    /// El indice de assets del arbol (0034): cada documento como un item
    /// (`kind:namespace.name`) con su carpeta, lo que define y expone, su
    /// puntero, sus relaciones en las dos direcciones y su acceso. Es lo que
    /// el catalogo de la consola lee; `--json` es el indice entero.
    Assets {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Lo que un `.sql` del arbol declara: que lee, que escribe y en que modo
    /// (`create or replace table ... as` sobrescribe, `insert into` anexa,
    /// `insert or replace into` hace upsert; un `select` lee y no escribe), o
    /// por que no es una unidad. Dentro de un arbol, ademas, si lo que lee se
    /// puede leer y lo que escribe se puede escribir. `--json` con posiciones.
    Sql {
        fichero: PathBuf,
        /// El arbol contra el que cotejar; sin el, el primero subiendo.
        #[arg(long)]
        arbol: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Un arbol de antes pasa a despues (0033 §4): `ore migrate v1alpha12 .`
    /// convierte cada `View` con `materialized` en un `Dataset` con su plan,
    /// cada `Table` con `datasource: lago` en un `Dataset` escrito, reapunta
    /// `from` a lo que paso a ser dataset y mueve `copias/` a `datasets/`.
    /// Con `--seco` dice que haria y no toca nada.
    Migrate {
        /// La version de destino. Hoy solo `v1alpha12`.
        version: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Dice que haria y no escribe nada.
        #[arg(long)]
        seco: bool,
    },
    /// Los datasets del arbol, por sus punteros (0031 §10, W3.6b): la copia de
    /// cada vista materializada (`copias/`) y la salida de cada `write()`
    /// (`datasets/`), que son la misma cosa — una tabla Iceberg en el bucket.
    /// Sin banderas los lista; `--ficha` trae la historia de una tabla;
    /// `--recoger` es el mantenimiento (expirar lo superado, retirar lo que
    /// nadie nombra, mover los punteros); `--confirmar` es el swap del puntero
    /// de un dataset del lago, con su `Table`, por quien no puede empujar.
    /// Trabaja sobre los punteros: no compila el arbol ni abre un origen.
    Datasets {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// La salida como una linea JSON (lo que `ore-serve` lee).
        #[arg(long)]
        json: bool,
        /// El puntero y la historia de la tabla de `<paquete>.<nombre>`.
        #[arg(long, value_name = "NS.NOMBRE")]
        ficha: Option<String>,
        /// El mantenimiento: expirar, retirar, mover los punteros.
        #[arg(long)]
        recoger: bool,
        /// Cuanta historia conserva `--recoger`: `7d`, `12h`, `30m`, `0`.
        #[arg(long, value_name = "EDAD")]
        edad: Option<String>,
        /// Con `--recoger`: dice que se iria y no toca nada.
        #[arg(long)]
        seco: bool,
        /// El swap: el puntero de la Table del lago `<paquete>.<tabla>`.
        #[arg(long, value_name = "NS.TABLA")]
        confirmar: Option<String>,
        /// Con `--confirmar`: el `metadata.json` que el escritor dejo en el bucket.
        #[arg(long, value_name = "URI")]
        metadata_location: Option<String>,
        /// Con `--confirmar`: el `metadata_location` sobre el que se construyo
        /// (vacio si el dataset nace). Si el puntero ya no es ese: codigo 75.
        #[arg(long, value_name = "URI")]
        esperado: Option<String>,
        /// Con `--confirmar`: el id del snapshot vigente.
        #[arg(long)]
        snapshot: Option<String>,
        /// Con `--confirmar`: cuantas filas tiene.
        #[arg(long)]
        filas: Option<i64>,
        /// Con `--confirmar`: las columnas de la Table del lago como JSON
        /// `{"col": "Tipo", ...}` (obligatorio si la Table no existe).
        #[arg(long, value_name = "JSON")]
        columnas: Option<String>,
        /// Con `--confirmar`: quien escribio (queda en el puntero).
        #[arg(long)]
        sujeto: Option<String>,
        /// Donde viven los punteros de las copias; sin esto, `<arbol>/copias`.
        #[arg(long, value_name = "DIR")]
        informe: Option<PathBuf>,
        /// El commit del catalogo REST de Iceberg (0031 §11): `requirements` +
        /// `updates` de un `updateTable` o un `commitTransaction`, aplicados,
        /// la clave de operacion cotejada, la Table que nace o sigue el esquema,
        /// y el puntero. Con `--peticion`.
        #[arg(long)]
        commit: bool,
        /// Con `--commit`: la tabla `<paquete>.<tabla>` si el cuerpo no trae `identifier`.
        #[arg(long, value_name = "NS.TABLA")]
        tabla: Option<String>,
        /// La tabla nace de un `createTable` (sin stage): metadata.json v0, Table y puntero.
        #[arg(long, value_name = "NS.TABLA")]
        crear: Option<String>,
        /// `stage-create`: los metadatos que la tabla tendria, sin escribir nada.
        #[arg(long, value_name = "NS.TABLA")]
        esbozar: Option<String>,
        /// La retencion declarada en la tabla (`history.expire.*`), con `--edad` y `--minimo`.
        #[arg(long, value_name = "NS.TABLA")]
        retencion: Option<String>,
        /// Con `--retencion`: cuantos snapshots se conservan como minimo.
        #[arg(long)]
        minimo: Option<i64>,
        /// El cuerpo de la peticion: JSON, `@fichero` o `-` (stdin).
        #[arg(long, value_name = "JSON|@FICHERO|-")]
        peticion: Option<String>,
        /// Con `--commit`/`--crear`: la retencion de una tabla que nace y no la trae (`7d`).
        #[arg(long, value_name = "EDAD")]
        retencion_defecto: Option<String>,
        /// `loadTable`: el LoadTableResult de la tabla, tal cual (con `--prestar`, la credencial).
        #[arg(long, value_name = "NS.TABLA")]
        cargar: Option<String>,
        /// Con `--cargar`/`--esbozar`: la credencial acotada a la tabla, prestada por el almacen.
        #[arg(long)]
        prestar: bool,
        /// Con `--prestar`: solo para leer (lo que el puesto usa en over()).
        #[arg(long)]
        leer: bool,
    },
    /// Pregunta a la cache si lo materializado sirve, y si no, por que.
    ///
    /// Es la mitad del tercer plano que si es nuestra. Las filas viven en una
    /// tabla del lago del cliente; **la afirmacion sobre bajo que se escribieron
    /// es de aqui**, y sin ella una cache es un acelerador sin gobierno.
    #[command(name = "cache", subcommand)]
    Cache(AccionCache),
    /// Compila el repositorio a un Ontology Bundle firmado.
    Compile {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Eleva el estado de madurez de una entidad.
    Promote { entity: String },
    /// Emite a ODCS o a esquema Cedar (`cedar` en JSON, `cedarschema` nativo).
    ///
    /// `oos` y `json` dan la forma canónica, con y sin interpretar. Apache Ossie
    /// está declarado y no implementado: falla explicando por qué exige lo físico.
    Export {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        format: String,
    },

    // ── Runtime ─────────── producción · custodia credenciales vivas
    /// Sirve el contrato por MCP sobre stdio. Nivel L1: no toca un dato.
    ///
    /// La frontera con `serve` no es el nivel, es qué custodian: `dev` es un
    /// proceso hijo que muere con su cliente y no abre un puerto; `serve` es un
    /// servicio que sobrevive a sus clientes y por eso les debe autenticación
    /// (ADR 0005).
    Dev {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Sirve la ontología en producción.
    Serve {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Command::Validate { path } => return validar(path),
        Command::Report { path } => return informar(path),
        Command::View {
            accion:
                Some(AccionVista::Add {
                    nombre,
                    de,
                    campos,
                    recorte,
                    owner,
                    path,
                }),
            ..
        } => {
            return autoria::anadir(path, nombre, de, campos, recorte, owner.as_deref());
        }
        Command::View { path, .. } => return vista::ver(path),
        Command::Verify { propuesta, path } => return verificar::verificar(path, propuesta),
        Command::Materialize {
            path,
            seco,
            recoger,
            informe,
            rehacer,
            vista,
        } => {
            return materializar::materializar(
                path,
                &materializar::Opciones {
                    seco: *seco,
                    recoger: *recoger,
                    informe: informe.as_deref(),
                    rehacer: *rehacer,
                    solo: vista,
                },
            );
        }
        Command::Ask {
            path,
            vista,
            limite,
            seco,
        } => {
            return preguntar::preguntar(
                path,
                &preguntar::Opciones {
                    vista,
                    limite: *limite,
                    seco: *seco,
                },
            );
        }
        Command::Assets { path, json } => {
            return activos::assets(path, &activos::Opciones { json: *json });
        }
        Command::Sql {
            fichero,
            arbol,
            json,
        } => {
            return unidad_sql::sql(
                fichero,
                &unidad_sql::Opciones {
                    arbol: arbol.clone(),
                    json: *json,
                },
            );
        }
        Command::Migrate {
            version,
            path,
            seco,
        } => {
            if version != "v1alpha12" {
                eprintln!("ore migrate · solo se migra a `v1alpha12` (pediste `{version}`)");
                return std::process::ExitCode::from(64);
            }
            return migrar::migrar(path, &migrar::Opciones { seco: *seco });
        }
        Command::Datasets {
            path,
            json,
            ficha,
            recoger,
            edad,
            seco,
            confirmar,
            metadata_location,
            esperado,
            snapshot,
            filas,
            columnas,
            sujeto,
            informe,
            commit,
            tabla,
            crear,
            esbozar,
            retencion,
            minimo,
            peticion,
            retencion_defecto,
            cargar,
            prestar,
            leer,
        } => {
            return datasets::datasets(
                path,
                &datasets::Opciones {
                    json: *json,
                    ficha: ficha.as_deref(),
                    recoger: *recoger,
                    edad: edad.as_deref(),
                    seco: *seco,
                    confirmar: confirmar.as_deref(),
                    metadata_location: metadata_location.as_deref(),
                    esperado: esperado.as_deref(),
                    snapshot: snapshot.as_deref(),
                    filas: *filas,
                    columnas: columnas.as_deref(),
                    sujeto: sujeto.as_deref(),
                    informe: informe.as_deref(),
                    commit: *commit,
                    tabla: tabla.as_deref(),
                    crear: crear.as_deref(),
                    esbozar: esbozar.as_deref(),
                    retencion: retencion.as_deref(),
                    minimo: *minimo,
                    peticion: peticion.as_deref(),
                    retencion_defecto: retencion_defecto.as_deref(),
                    cargar: cargar.as_deref(),
                    prestar: *prestar,
                    leer: *leer,
                },
            );
        }
        Command::Invoke {
            path,
            funcion,
            puerta,
            modelo,
            informe,
            limite,
            concurrencia,
            seco,
        } => {
            return invocar::invocar(
                path,
                &invocar::Opciones {
                    funcion,
                    puerta: puerta.as_deref(),
                    modelo: modelo.as_deref(),
                    informe: informe.as_deref(),
                    limite: *limite,
                    concurrencia: *concurrencia,
                    seco: *seco,
                },
            );
        }
        Command::Diff { before, after } => return diferir(before, after),
        Command::Compile { path } => return compilar(path),
        Command::Export { path, format } => return exportar(path, format),
        Command::Dev { path } => return desarrollo(path),
        Command::Init {
            name,
            depend,
            trust,
            trust_log,
            path,
        } => {
            return inicio::init(
                path,
                &inicio::Respuestas {
                    nombre: name.as_deref(),
                    depende: depend,
                    claves: trust,
                    logs: trust_log,
                },
            );
        }
        Command::Discover {
            from,
            source,
            out,
            name,
            solo,
            solo_de,
            tipo,
            sin_modelar,
            modelar,
            owner,
        } => {
            return descubrir(
                from.as_deref(),
                source.as_ref(),
                out,
                name.as_deref(),
                solo,
                solo_de.as_deref(),
                Reglas {
                    tipo: tipo.as_deref(),
                    modeladas: if *sin_modelar {
                        Some(Vec::new())
                    } else if modelar.is_empty() {
                        None
                    } else {
                        Some(modelar.clone())
                    },
                    owner: owner.as_deref(),
                },
            );
        }
        Command::Model { path, objeto } => return revision::modelar(path, objeto),
        Command::Copy { path, objeto } => return revision::copiar(path, objeto),
        Command::Review {
            path,
            answers,
            reinducir,
        } => return revision::review(path, answers.as_deref(), *reinducir),
        Command::Lock { path, check } => return candado::lock(path, *check),
        Command::Pack {
            path,
            out,
            sign,
            log,
        } => {
            return empaquetar::pack(path, out.as_deref(), sign.as_deref(), log.as_deref());
        }
        Command::Cache(AccionCache::Check {
            manifest,
            entity,
            props,
            topology,
            at,
            sla,
            path,
        }) => {
            let pkg = match cargar_valido(path, false) {
                Ok(p) => p,
                Err(c) => return c,
            };
            return cache::check(
                &cache::Consulta {
                    manifiesto: manifest,
                    entidad: entity,
                    propiedades: props.clone(),
                    topologia: topology.as_deref(),
                    instante: at.as_deref(),
                    sla: sla.as_deref(),
                },
                &pkg,
            );
        }
        Command::Source(AccionFuente::Explore { name, path }) => {
            return lector::explorar(path, name);
        }
        Command::Source(AccionFuente::Check { name, path }) => {
            return lector::comprobar(path, name);
        }
        Command::Package(AccionPaquete::Merge {
            origen,
            a,
            since,
            path,
        }) => {
            return paquete::fundir(path, origen, a, since.as_deref());
        }
        Command::Package(AccionPaquete::Split {
            paquete,
            con,
            a,
            since,
            path,
        }) => {
            return paquete::dividir(path, paquete, con, a.as_deref(), since.as_deref());
        }
        Command::Package(AccionPaquete::Move {
            qname,
            a,
            since,
            path,
        }) => {
            return paquete::mover(path, qname, a, since.as_deref());
        }
        Command::Package(AccionPaquete::New {
            name,
            owner,
            domain,
            path,
        }) => {
            return paquete::nuevo(path, name, owner.as_deref(), domain.as_deref());
        }
        Command::Source(AccionFuente::Catalog { name, out, path }) => {
            return lector::emitir_catalogo(path, name, out.as_deref());
        }
        Command::DriftDetect { source, from, path } => {
            let origen = match (source, from) {
                (_, Some(f)) => deriva::Origen::Fichero(f),
                (Some(s), None) => deriva::Origen::Fuente(s),
                (None, None) => {
                    eprintln!("error: hace falta `--source <fuente>` o `--from <fichero>`");
                    eprintln!("  Son dos actos y fallan por separado: preguntarle al origen");
                    eprintln!("  necesita una credencial, y comparar no necesita nada.");
                    return std::process::ExitCode::from(64); // EX_USAGE
                }
            };
            return deriva::detectar(path, origen);
        }
        Command::Source(AccionFuente::Add {
            name,
            url,
            tipo,
            env,
            label,
            description,
            path,
        }) => {
            return fuente::add(&fuente::Alta {
                raiz: path,
                nombre: name,
                url,
                tipo: tipo.as_deref(),
                env: env.as_deref(),
                etiquetas: label,
                descripcion: description.as_deref(),
            });
        }
        Command::Source(AccionFuente::Remove {
            name,
            path,
            keep_secret,
        }) => {
            return fuente::remove(&fuente::Baja {
                raiz: path,
                nombre: name,
                conservar_secreto: *keep_secret,
            });
        }
        _ => {}
    }

    let (nombre, fase) = match cli.command {
        Command::Validate { .. }
        | Command::Diff { .. }
        | Command::Compile { .. }
        | Command::Export { .. }
        | Command::Dev { .. }
        | Command::Init { .. }
        | Command::Discover { .. }
        | Command::Report { .. }
        | Command::View { .. }
        | Command::Verify { .. }
        | Command::Materialize { .. }
        | Command::Invoke { .. }
        | Command::Datasets { .. }
        | Command::Migrate { .. }
        | Command::Assets { .. }
        | Command::Sql { .. }
        | Command::Ask { .. }
        | Command::Review { .. }
        | Command::Model { .. }
        | Command::Copy { .. }
        | Command::Lock { .. }
        | Command::Pack { .. }
        | Command::Source(_)
        | Command::Package(_)
        | Command::DriftDetect { .. }
        | Command::Cache(_) => unreachable!(),
        Command::Lint { .. } => ("lint", "posterior"),
        Command::Test { .. } => ("test", "posterior"),
        Command::Plan { .. } => ("plan", "posterior"),
        Command::Promote { .. } => ("promote", "posterior"),
        Command::Serve { .. } => ("serve", "posterior"),
    };

    eprintln!("ore {nombre}: no implementado todavía (fase {fase})");
    eprintln!();
    eprintln!("  Hoy existen: {}.", implementados().join(", "));
    eprintln!("  Marcador:    cargo test -p ore-cli --test conformance -- --nocapture");

    std::process::ExitCode::from(70) // EX_SOFTWARE
}

/// Los comandos que hoy hacen algo.
///
/// **Se deriva de `clap`**, que es quien sabe qué hay. La lista anterior estaba
/// escrita a mano, era la tercera copia de la misma cosa y le faltaba `report` —
/// que es exactamente lo que le pasa a una cuenta escrita a mano en cuanto la
/// realidad avanza sin ella.
fn implementados() -> Vec<String> {
    use clap::CommandFactory as _;
    Cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .filter(|n| !SIN_IMPLEMENTAR.contains(&n.as_str()))
        .collect()
}

/// Lo que está declarado y todavía no hace nada. Es la lista corta, y es la que
/// encoge: un comando desaparece de aquí el día que existe.
const SIN_IMPLEMENTAR: [&str; 5] = ["lint", "test", "plan", "promote", "serve"];

/// `ore validate` — nivel L0. Hermético: no abre un socket ni lee una credencial.
fn validar(path: &std::path::Path) -> std::process::ExitCode {
    if !path.exists() {
        eprintln!("error: no existe `{}`", path.display());
        return std::process::ExitCode::from(66); // EX_NOINPUT
    }

    let diags = if path.is_dir() {
        ore_core::validate_package(path)
    } else {
        match std::fs::read_to_string(path) {
            Ok(t) => ore_core::validate_document(path, &t),
            Err(e) => {
                eprintln!("error: no se pudo leer `{}`: {e}", path.display());
                return std::process::ExitCode::from(66);
            }
        }
    };

    if diags.is_empty() {
        println!("ok · sin errores");
        // El acuse de recibo. Escribir Cedar es hoy un acto a ciegas: se declara
        // una politica y nada dice si alcanza lo que uno creia. Que una politica
        // no alcance nada NO es un error —`Property in [Label, EntityType]`
        // existe para que una entidad quede gobernada el dia que se etiqueta, sin
        // tocar la politica—, pero verlo es la diferencia entre saberlo y
        // suponerlo.
        let alcance = if path.is_dir() {
            ore_core::politica::alcance(&ore_core::validate::cargar_paquete(path).0)
        } else {
            Default::default()
        };
        if !alcance.is_empty() {
            println!();
            for (id, props) in &alcance {
                if props.is_empty() {
                    println!("  · {id} — no alcanza ninguna propiedad todavia");
                } else {
                    println!("  · {id} — {}", resumir(props));
                }
            }
        }
        return std::process::ExitCode::SUCCESS;
    }

    let raiz = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    for d in &diags {
        eprintln!("{}", d.render(raiz));
        eprintln!();
    }
    let n = diags.len();
    eprintln!("{n} error{}", if n == 1 { "" } else { "es" });
    std::process::ExitCode::FAILURE
}

/// `ore discover` — el inductor.
///
/// Escribe lo que es un hecho y **reporta lo que es una conjetura**. Lo inducido
/// entra en `DRAFT` y probablemente no compile: una entidad sin clave falla con
/// `OOS2010`, y está bien que falle — inventar la clave sería lo único peor.
/// Las primeras propiedades y cuantas quedan. Una politica sobre `critical`
/// puede alcanzar doscientas, y doscientas lineas no informan: abruman.
fn resumir(props: &[String]) -> String {
    const MUESTRA: usize = 4;
    if props.len() <= MUESTRA {
        return props.join(", ");
    }
    format!(
        "{}, y {} mas",
        props[..MUESTRA].join(", "),
        props.len() - MUESTRA
    )
}

/// Lo que el alcance manda (ORE 0027 P1): la clase de la base y qué tablas
/// se modelan. Juntas porque las dos van al mismo fichero.
struct Reglas<'a> {
    tipo: Option<&'a str>,
    /// `None` = todas; `Some(vec![])` = ninguna.
    modeladas: Option<Vec<String>>,
    /// La decisión `dueno`, contestada de antemano por quien llama.
    owner: Option<&'a str>,
}

fn descubrir(
    origen: Option<&std::path::Path>,
    fuente: Option<&String>,
    destino: &std::path::Path,
    nombre: Option<&str>,
    solo: &[String],
    solo_de: Option<&std::path::Path>,
    reglas: Reglas<'_>,
) -> std::process::ExitCode {
    let Reglas {
        tipo,
        modeladas,
        owner,
    } = reglas;
    if let Some(t) = tipo
        && t != "standard"
        && t != "foreign"
    {
        eprintln!("error: `--type` es `standard` o `foreign`, no `{t}`");
        return std::process::ExitCode::from(64); // EX_USAGE
    }
    // ⛔ Un dueño que no es un handle se rechaza AQUI, antes de escribir: el
    //   inductor lo escribiria tal cual y el paquete entero naceria sin
    //   compilar (OOS2009) — lo mismo que `cambiame`, pero aparentando que no.
    if let Some(o) = owner
        && !ore_core::pertenencia::es_handle(o)
    {
        eprintln!(
            "error: `--owner {o}` no es un handle: `team:<handle>` o `user:<handle>`, en minúsculas, dígitos y guiones"
        );
        return std::process::ExitCode::from(64); // EX_USAGE
    }
    // El catálogo se lee de un fichero o de una fuente viva, y a partir de aquí
    // el resto del comando no distingue cuál: es el mismo texto.
    let texto = match (origen, fuente) {
        (Some(o), _) => match std::fs::read_to_string(o) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: no se pudo leer `{}`: {e}", o.display());
                return std::process::ExitCode::from(66); // EX_NOINPUT
            }
        },
        (None, Some(f)) => match lector::catalogo(std::path::Path::new("."), f) {
            Ok(t) => t,
            Err(fallo) => {
                eprintln!("error: {}", fallo.mensaje);
                for l in &fallo.ayuda {
                    eprintln!("{l}");
                }
                return std::process::ExitCode::from(fallo.codigo);
            }
        },
        (None, None) => unreachable!("clap exige --from o --source"),
    };
    let mut catalogo = match inductor::Catalogo::leer(&texto) {
        Ok(c) => c,
        Err(m) => {
            eprintln!("error: {m}");
            return std::process::ExitCode::from(65); // EX_DATAERR
        }
    };

    // El alcance, si lo hay. Se aplica AQUI —sobre el catalogo, antes de nada—
    // porque todo lo de abajo cuenta tablas: `costura` compara contra el
    // manifiesto y avisaria de tablas que nadie pidio.
    let mut objetos: Vec<String> = solo.to_vec();
    if let Some(p) = solo_de {
        match alcance::de_fichero(p) {
            Ok(v) => objetos.extend(v),
            Err(m) => {
                eprintln!("error: {m}");
                return std::process::ExitCode::from(66); // EX_NOINPUT
            }
        }
    }
    let el_alcance = if objetos.is_empty() {
        if tipo.is_some() {
            eprintln!("error: `--type` pide un alcance (`--only`): una base es lo que se elige");
            return std::process::ExitCode::from(64);
        }
        None
    } else {
        let a = alcance::Alcance::nuevo(catalogo.fuente(), objetos)
            .con_tipo(tipo.unwrap_or("foreign"))
            .con_entidades(modeladas.clone());
        // ⚠️ Los nombres se toman ANTES de recortar. Listarlos despues era el
        //    error que tenia esto: con una errata, el recorte deja el catalogo
        //    vacio y la ayuda salia sin una sola linea — justo cuando lo unico
        //    util que se puede decir es como se llaman de verdad.
        let habia: Vec<String> = catalogo.tablas.iter().map(|t| t.nombre.clone()).collect();
        let (recortado, recorte) = a.aplicar(catalogo);
        // Un nombre que el catalogo no tiene PARA el comando: se acaba de
        // escribir en la linea de ordenes, asi que una errata es lo mas
        // probable y seguir produciria un paquete al que le falta una tabla
        // sin que nada lo diga.
        if !recorte.sin_respaldo.is_empty() {
            eprintln!(
                "error: el origen no tiene {} de los objetos pedidos:",
                recorte.sin_respaldo.len()
            );
            for o in &recorte.sin_respaldo {
                eprintln!("  · `{o}`");
            }
            eprintln!("  El catálogo trae {} objetos:", habia.len());
            for n in habia.iter().take(20) {
                eprintln!("  · `{n}`");
            }
            if habia.len() > 20 {
                eprintln!("  · … y {} más", habia.len() - 20);
            }
            return std::process::ExitCode::from(65); // EX_DATAERR
        }
        if recortado.tablas.is_empty() {
            eprintln!("error: el alcance deja el paquete sin ninguna tabla");
            return std::process::ExitCode::from(65);
        }
        catalogo = recortado;
        Some((a, recorte))
    };

    let paquete = nombre.map(String::from).unwrap_or_else(|| {
        destino
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "inducido".into())
    });
    // ⛔ El nombre del paquete ES el espacio de nombres de todo lo que induce
    //   (OOS2030): una letra y luego letras, dígitos y `_`. Con un guion se
    //   escribiría un paquete entero que no compila — medido en `victor`
    //   (`test-standard`, 0027 P1 I5).
    if !ore_core::pertenencia::puede_ser_namespace(&paquete) {
        eprintln!(
            "error: `{paquete}` no puede ser un espacio de nombres: una letra y luego letras, dígitos y `_` (sin guiones ni puntos)"
        );
        return std::process::ExitCode::from(64); // EX_USAGE
    }

    // El vocabulario que el repositorio ya publica. Sin esto la séptima pregunta
    // solo sabe ofrecer «acuña uno», que es la respuesta cara y la que produce
    // cuatro mil conceptos.
    let voc = match raiz_del_repositorio(destino) {
        Some(r) => vocabulario::Vocabulario::leer(&r),
        None => vocabulario::Vocabulario::default(),
    };
    let regla = inductor::Regla {
        estandar: el_alcance.as_ref().is_some_and(|(a, _)| a.estandar()),
        modeladas: el_alcance
            .as_ref()
            .and_then(|(a, _)| a.modeladas().cloned()),
        copiadas: Default::default(),
    };
    // ⭐ El dueño no se deriva: lo contesta quien llama, como cualquier otra
    //   decision — y por eso entra por `Decisiones` y se guarda con las demas
    //   (`discover.answers.json`), para que `review` lo conserve en vez de
    //   devolver el manifiesto a `cambiame`.
    let mut dec = inductor::Decisiones::default();
    if let Some(o) = owner {
        dec.responder(
            format!("dueno/{paquete}"),
            inductor::Respuesta::Palabra(o.to_string()),
        );
    }
    let ind = inductor::inducir_con_regla(&catalogo, &paquete, &dec, &voc, &regla);
    if let Err((codigo, mensaje)) = escribir_paquete(&ind, destino) {
        eprintln!("error: {mensaje}");
        return std::process::ExitCode::from(codigo);
    }
    if !dec.is_empty() {
        let dadas = revision::ruta_respuestas(destino);
        if let Err(e) = std::fs::write(&dadas, dec.json().pretty()) {
            eprintln!("error: no se pudo escribir `{}`: {e}", dadas.display());
            return std::process::ExitCode::from(73);
        }
    }

    // El catálogo, al lado de lo que produjo. No es un caché: es lo que hace que
    // `--source` sea reproducible como `--from`, y lo que permite que `review`
    // vuelva a inducir sin hablar con la fuente ni custodiar una credencial.
    let catalogo_json = destino.join("discover.catalog.json");
    if let Err(e) = std::fs::write(&catalogo_json, &texto) {
        eprintln!(
            "error: no se pudo escribir `{}`: {e}",
            catalogo_json.display()
        );
        return std::process::ExitCode::from(73);
    }
    let _ = std::fs::write(revision::ruta_cola(destino), revision::cola(&ind));

    // El alcance, al lado del catalogo entero y de las respuestas. Sin esto,
    // `drift-detect` denunciaria cada tabla no elegida en cada pasada: «no lo
    // elegi» y «no lo vi» son la misma ausencia hasta que una se escribe.
    if let Some((a, recorte)) = &el_alcance {
        let r = alcance::ruta(destino);
        if let Err(e) = std::fs::write(&r, a.escribir()) {
            eprintln!("error: no se pudo escribir `{}`: {e}", r.display());
            return std::process::ExitCode::from(73);
        }
        println!(
            "  \u{2713} alcance: {} objeto(s) · {} del origen se quedan fuera, y {} lo dice",
            catalogo.tablas.len(),
            recorte.fuera,
            alcance::FICHERO
        );
    }

    print!("{}", inductor::informe(&ind, destino));
    for l in costura(destino, &catalogo) {
        eprintln!("{l}");
    }
    std::process::ExitCode::SUCCESS
}

/// Escribe los ficheros de una inducción bajo `destino`.
///
/// Lo comparten `discover` y `review` porque escriben lo mismo: el inductor dice
/// QUÉ, y quien llama dice DÓNDE. Dos copias de este bucle serían dos sitios
/// donde arreglar el mismo permiso denegado.
fn escribir_paquete(
    ind: &inductor::Induccion,
    destino: &std::path::Path,
) -> Result<(), (u8, String)> {
    for (rel, contenido) in &ind.ficheros {
        let ruta = destino.join(rel);
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d)
                .map_err(|e| (73, format!("no se pudo crear `{}`: {e}", d.display())))?;
        }
        std::fs::write(&ruta, contenido)
            .map_err(|e| (73, format!("no se pudo escribir `{}`: {e}", ruta.display())))?;
    }
    Ok(())
}

/// El directorio que manda sobre un paquete: el primero, subiendo, que tiene un
/// manifiesto.
///
/// Lo necesitan dos cosas por motivos distintos y es el mismo directorio: ahí
/// está la fuente declarada, y ahí están los conceptos publicados que `discover`
/// puede ofrecer. Buscarlo dos veces sería tener dos ideas de dónde empieza el
/// repositorio.
pub fn raiz_del_repositorio(desde: &std::path::Path) -> Option<std::path::PathBuf> {
    // `canonicalize` exige que la ruta exista, y `--out` normalmente **no existe
    // todavía**: se resuelve desde el ancestro más cercano que sí exista. Sin
    // esto, subir desde una ruta relativa acaba en el directorio vacío y el
    // repositorio parece no estar donde está.
    let absoluto = desde
        .ancestors()
        .find_map(|d| std::fs::canonicalize(d).ok())
        .or_else(|| std::env::current_dir().ok())?;
    absoluto
        .ancestors()
        .find(|d| d.join("ontology.config.yaml").is_file())
        .map(std::path::Path::to_path_buf)
}

/// El aviso de la costura: una tabla referencia una fuente, y esa fuente la
/// declara **el manifiesto del repositorio**.
///
/// Salió midiendo, no leyendo. `discover --out <dir fuera de un repo>` escribe
/// tablas con `datasource: crm_prod` y nada declara ese datasource, así que
/// `ore validate` responde `OOS2004` por cada uno. Es coherente —el inductor no
/// inventa un manifiesto— pero significa que **el camino de verdad es dentro de
/// un repositorio**, y eso no lo decía nadie: el comando terminaba en verde y el
/// error aparecía un paso después, lejos de su causa.
fn costura(destino: &std::path::Path, cat: &inductor::Catalogo) -> Vec<String> {
    let fuente = cat.fuente();
    let manifiesto = raiz_del_repositorio(destino).map(|r| r.join("ontology.config.yaml"));

    let Some(m) = manifiesto else {
        return vec![
            format!("aviso: nada declara la fuente `{fuente}`, y las tablas la referencian."),
            format!(
                "  `{}` no está dentro de un repositorio ontológico: no hay",
                destino.display()
            ),
            "  `ontology.config.yaml` en ningún directorio por encima, así que".to_string(),
            "  `ore validate` dirá OOS2004 una vez por tabla.".to_string(),
            "  `ore init` crea uno, y `ore discover --out packages/<nombre>` induce dentro."
                .to_string(),
        ];
    };
    let declarada = std::fs::read_to_string(&m)
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok())
        .map(|a| {
            a.get("datasources")
                .map(|(_, v)| v.items())
                .unwrap_or(&[])
                .iter()
                .any(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(fuente))
        })
        .unwrap_or(false);
    if declarada {
        return Vec::new();
    }
    vec![
        format!("aviso: `{}` no declara la fuente `{fuente}`.", m.display()),
        "  Las tablas la referencian, así que `ore validate` dirá OOS2004 por cada una."
            .to_string(),
        format!("  `ore source add --name {fuente} <url>` la declara sin escribir el secreto."),
    ]
}

/// `ore dev` — el servidor de contexto.
///
/// Compila el repositorio y sirve **el contrato** por MCP sobre stdio. No abre
/// un puerto, no lee una credencial y no toca un dato: es L1, y la mitad que el
/// criterio de la fase 3 daba por supuesta sin nombrarla.
fn desarrollo(path: &std::path::Path) -> std::process::ExitCode {
    let pkg = match cargar_valido(path, false) {
        Ok(p) => p,
        Err(c) => return c,
    };
    match mcp::servir(&pkg) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(motivo) => {
            eprintln!("error: {motivo}");
            std::process::ExitCode::from(70) // EX_SOFTWARE
        }
    }
}

/// Lo que se puede cargar como paquete: un árbol, o un `.oob` que ya lo lleva
/// entero dentro.
fn es_paquete(p: &std::path::Path) -> bool {
    p.is_dir() || (p.is_file() && p.extension().is_some_and(|x| x == "oob"))
}

/// `ore diff` — la familia `OOS5xxx`.
///
/// Igual de hermético que `validate`: compara dos paquetes, y le da igual en qué
/// forma vengan —un árbol o un `.oob`—, porque la comparación es entre sus
/// formas canónicas y esas no saben de contenedores. Que el carácter rompedor de
/// un cambio **se compute** en lugar de afirmarse es exactamente lo que hace que
/// la versión sea una comprobación y no una promesa.
///
/// El código de salida distingue las dos cosas que a un CI le importan por
/// separado: `0` compatible, `1` hay cambios rompedores.
fn diferir(antes: &std::path::Path, despues: &std::path::Path) -> std::process::ExitCode {
    for p in [antes, despues] {
        // Un directorio **o** un `.oob`: un paquete es un paquete, y el `.oob`
        // es la forma en que un paquete viaja. La pregunta entera de una
        // actualización —qué me rompe la versión que vendría— se hace contra lo
        // que se publica, no contra un árbol que quien consume no tiene.
        if !es_paquete(p) {
            eprintln!(
                "error: `{}` no es un paquete: ni un directorio ni un `.oob`",
                p.display()
            );
            return std::process::ExitCode::from(66); // EX_NOINPUT
        }
    }

    // Un paquete que no valida no se puede comparar: la diferencia entre dos
    // formas mal construidas no significa nada.
    for p in [antes, despues] {
        let (_, diags) = ore_core::validate::cargar_paquete(p);
        if let Some(d) = diags.first() {
            eprintln!("{}", d.render(p));
            eprintln!(
                "error: `{}` no analiza; no hay nada que comparar",
                p.display()
            );
            return std::process::ExitCode::from(65); // EX_DATAERR
        }
    }

    let (a, _) = ore_core::validate::cargar_paquete(antes);
    let (b, _) = ore_core::validate::cargar_paquete(despues);
    let informe = ore_core::diff::diff(&a, &b);
    println!("{}", informe.json().pretty());

    if informe.changes.is_empty() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

/// Carga un paquete y lo rechaza si no valida. Compilar algo que no valida
/// produciría un digest de un artefacto que no existe.
fn cargar_valido(
    path: &std::path::Path,
    ignorar_generados: bool,
) -> Result<ore_core::link::Package, std::process::ExitCode> {
    if !path.is_dir() {
        eprintln!("error: `{}` no es un directorio de paquete", path.display());
        return Err(std::process::ExitCode::from(66)); // EX_NOINPUT
    }
    let diags: Vec<_> = ore_core::validate_package(path)
        .into_iter()
        .filter(|d| !(ignorar_generados && d.code == ore_core::Code::Oos2013))
        .collect();
    if let Some(d) = diags.first() {
        eprintln!("{}", d.render(path));
        return Err(std::process::ExitCode::from(65)); // EX_DATAERR
    }
    Ok(ore_core::validate::cargar_paquete(path).0)
}

/// `ore compile` — la forma canónica y los digests.
///
/// Puro por invariante III: sin red, sin credenciales, sin reloj, sin
/// aleatoriedad. Ejecutarlo dos veces sobre el mismo árbol de ficheros produce
/// byte a byte la misma salida, y eso es lo que `digest/deterministic-across-runs`
/// certifica.
fn compilar(path: &std::path::Path) -> std::process::ExitCode {
    let pkg = match cargar_valido(path, false) {
        Ok(p) => p,
        Err(c) => return c,
    };

    let canonica = ore_core::normalize::package(&pkg);
    let salida = ore_core::json::Json::obj([
        (
            "canonical",
            ore_core::json::Json::Obj(canonica.into_iter().collect()),
        ),
        (
            "digest",
            ore_core::json::Json::obj([
                (
                    "package",
                    ore_core::json::Json::s(ore_core::digest::package(&pkg)),
                ),
                (
                    "bundle",
                    ore_core::json::Json::s(ore_core::digest::bundle(&pkg)),
                ),
                (
                    "documents",
                    ore_core::json::Json::Obj(
                        ore_core::digest::documents(&pkg)
                            .into_iter()
                            .map(|(k, v)| (k, ore_core::json::Json::s(v)))
                            .collect(),
                    ),
                ),
            ]),
        ),
        (
            "oosVersion",
            ore_core::json::Json::s(ore_core::digest::OOS_VERSION),
        ),
    ]);
    println!("{}", salida.pretty());
    std::process::ExitCode::SUCCESS
}

/// `ore export` — traducción a un formato externo.
///
/// El argumento puede ser un **directorio de paquete OOS** o un **fichero de
/// otro formato**, y `--format` dice a qué se traduce. Que ambas direcciones
/// vivan en el mismo comando no es economía de subcomandos: es la afirmación de
/// que la traducción es reversible, y ahí es donde un perfil deja de ser una
/// limitación y pasa a ser interoperabilidad.
///
/// La ida y vuelta se compone desde fuera —`export a odcs`, luego `export ese
/// odcs a oos`— y se compara. ORE no se examina a sí mismo.
fn exportar(path: &std::path::Path, formato: &str) -> std::process::ExitCode {
    use ore_core::json::Json;

    // Un fichero suelto no es un paquete OOS: es un documento de otro formato
    // que entra. Se lee sin validarlo — §4.3 prohíbe interpretar lo ajeno.
    if path.is_file() {
        let texto = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: no se pudo leer `{}`: {e}", path.display());
                return std::process::ExitCode::from(66);
            }
        };
        let arbol = match ore_core::parse::parse(&texto) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("error: `{}` no analiza: {}", path.display(), e.message);
                return std::process::ExitCode::from(65);
            }
        };
        let entrada = ore_core::normalize::foreign(&arbol);
        let salida = match formato {
            // Sin traducir: el documento tal cual, en forma canónica. Es la
            // referencia contra la que se mide la fidelidad de la ida y vuelta.
            "json" => entrada,
            "oos" => Json::Obj(ore_core::odcs::import(&entrada).into_iter().collect()),
            "odcs" => ore_core::odcs::reemit(&entrada),
            otro => {
                eprintln!("error: `--format {otro}` no se puede producir desde un fichero suelto");
                return std::process::ExitCode::from(64); // EX_USAGE
            }
        };
        println!("{}", salida.pretty());
        return std::process::ExitCode::SUCCESS;
    }

    // `cedarschema` regenera el artefacto, así que no puede exigir que el
    // artefacto esté al día: sería pedirle a alguien que arregle un fichero
    // usando un comando que ese mismo fichero bloquea.
    let regenera = formato == "cedarschema";
    let pkg = match cargar_valido(path, regenera) {
        Ok(p) => p,
        Err(c) => return c,
    };

    let salida = match formato {
        "odcs" => ore_core::odcs::emit(&pkg),
        "cedar" => ore_core::cedar_schema::emit(&pkg),
        // La sintaxis nativa es la que se compromete al repositorio y la que
        // consume el tooling de Cedar; el JSON es la misma proyeccion en el
        // formato de esquema de Cedar. Emitir las dos desde el mismo sitio es
        // lo que impide que diverjan.
        "cedarschema" => {
            print!("{}", ore_core::cedar_schema::emit_text(&pkg));
            return std::process::ExitCode::SUCCESS;
        }
        "oos" => Json::Obj(ore_core::normalize::package(&pkg).into_iter().collect()),
        // La cuarta superficie. El SDL es texto, no JSON: sale por `stdout` sin
        // pasar por `Json`, igual que `cedarschema`.
        "graphql" => match ore_core::graphql::emit(&pkg) {
            Ok(sdl) => {
                print!("{sdl}");
                return std::process::ExitCode::SUCCESS;
            }
            Err(motivo) => {
                eprintln!("error: no se puede emitir a GraphQL: {motivo}");
                return std::process::ExitCode::from(65); // EX_DATAERR
            }
        },
        // Ossie no es anfitrión de `Entity`: un `Dataset` exige `source` y cada
        // `Field` exige `expression`, y ninguno de los dos está en la entidad —
        // están en el binding. Emitir sin él obligaría a INVENTAR los valores
        // obligatorios, y produciría un documento que valida contra el esquema
        // de Ossie y miente sobre dónde vive el dato.
        "ossie" => {
            let huerfanas: Vec<String> = ore_core::normalize::sin_respaldo(&pkg);
            if !huerfanas.is_empty() {
                eprintln!(
                    "error: no se puede emitir a Ossie: {} sin fuente física (ni binding ni `backedBy`)",
                    huerfanas.join(", ")
                );
                eprintln!();
                eprintln!("  Un `Dataset` de Ossie exige `source`; cada `Field`, `expression`.");
                eprintln!(
                    "  Ninguno de los dos está en la entidad: están en el binding o en la vista."
                );
                eprintln!("  Emitir de todos modos exigiría inventarlos, y el documento");
                eprintln!("  resultante validaría contra Ossie y mentiría sobre dónde vive");
                eprintln!("  el dato. Por eso `Entity` es gramática propia y no perfil.");
                return std::process::ExitCode::from(65); // EX_DATAERR
            }
            eprintln!("ore export --format ossie: emisión no implementada todavía (fase 2)");
            return std::process::ExitCode::from(70);
        }
        otro => {
            eprintln!("error: formato `{otro}` desconocido");
            eprintln!("  formatos: odcs, cedar, cedarschema, graphql, ossie, oos, json");
            return std::process::ExitCode::from(64);
        }
    };
    println!("{}", salida.pretty());
    std::process::ExitCode::SUCCESS
}
/// `ore report` — **el registro de qué gobierna qué, y quién responde**.
///
/// # Lo que NO es, y por qué eso lo define
///
/// No es una lista de incumplimientos. **Aquí no puede haber filas rojas**:
/// una propiedad sin la clase de gobierno que exige su clasificación no
/// compila (`OOS8001`), así que un paquete que llega hasta aquí ya está
/// cubierto entero.
///
/// Eso lo separa del *compliance status report* de GitLab, que es fila por
/// (proyecto, control) con su estado, y existe porque **allí el gobierno se
/// evalúa cada doce horas sobre un objetivo que ya está desplegado**. Aquí se
/// evalúa al compilar, así que la pregunta interesante deja de ser *¿está
/// gobernado?* y pasa a ser:
///
/// > **¿Quién responde, y por qué vía?**
///
/// # Por qué no lista todas las propiedades
///
/// Porque la mayoría no exige nada. Medido sobre la ontología de referencia:
/// **40 propiedades clasificadas, 29 sin ninguna exigencia**. Un informe que
/// las listara sería el 72% de filas diciendo *«nada que gobernar»*, y el ruido
/// esconde exactamente lo que se viene a mirar.
///
/// Lo que exige gobierno lo decide `requiresGovernance`, **no** la
/// clasificación: una propiedad `low` está clasificada y no exige nada.
///
/// # Y lo ámbar no son filas
///
/// Una regla que **existe y no cuenta** —una aserción `severity: warning`, una
/// `type: text` que se transporta sin interpretar— no corresponde a ninguna
/// pareja (propiedad, clase): corresponde a una regla. Va al margen, y va,
/// porque *«lo vimos y no paramos nada»* tiene el mismo aspecto que no haberlo
/// visto.
fn informar(path: &std::path::Path) -> std::process::ExitCode {
    let pkg = match cargar_valido(path, true) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let lat = ore_core::flow::lattices(&pkg);
    let props = ore_core::flow::efectivas(&pkg, &lat);
    let cubierto = ore_core::governance::cobertura_atribuida(&pkg);
    let alcance = ore_core::politica::alcance(&pkg);
    // Cuantos `ConduitPolicy` hay, porque el motivo por el que una politica de
    // Cedar se queda sin dueno depende de eso y **no es el mismo**.
    let conductos = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::ConduitPolicy)
        .count();

    let mut filas = 0usize;
    for (prop, etiquetas) in &props {
        // Lo que EXIGE cada clasificación que alcanza. Es la misma lectura que
        // hace `OOS8001`, en la otra dirección: allí para señalar lo que falta,
        // aquí para nombrar lo que hay.
        let mut exigidas: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for (ret, nivel) in etiquetas {
            let Some(l) = lat.get(ret) else { continue };
            for (piso, naturalezas) in &l.requires_governance {
                if l.ge(nivel, piso) == Some(true) {
                    exigidas.extend(naturalezas.iter().map(String::as_str));
                }
            }
        }
        if exigidas.is_empty() {
            continue;
        }
        if filas == 0 {
            println!("{:<34} {:<15} QUIÉN RESPONDE", "PROPIEDAD", "EXIGE");
        }
        filas += 1;
        for clase in &exigidas {
            let quienes = cubierto
                .get(prop)
                .and_then(|m| m.get(clase))
                .map(|v| {
                    v.iter()
                        .map(|d| match &d.owner {
                            Some(o) => format!("{} ({o})", d.regla),
                            // Sin dueño hay DOS motivos y no eran el mismo. El
                            // comentario que había aquí decía «solo pasa con
                            // varios `ConduitPolicy`», y se midió que también
                            // pasa con NINGUNO — que es el caso de cualquier
                            // repositorio recién creado. Decir «varios» ahí manda
                            // a buscar un segundo conducto que no existe, y un
                            // mensaje que nombra la causa equivocada es peor que
                            // uno que calla.
                            //
                            // La salida es la misma para los dos: `@oosOwner` en
                            // la propia política.
                            None if conductos == 0 => {
                                format!("{} (sin ConduitPolicy del que heredar)", d.regla)
                            }
                            None => format!("{} (varios ConduitPolicy: sin herencia)", d.regla),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("{prop:<34} {clase:<15} {quienes}");
        }
    }

    if filas == 0 {
        println!("Ninguna propiedad de este paquete exige gobierno.");
        println!("No es un vacío: `requiresGovernance` es lo que exige, y ningún retículo");
        println!("declarado lo pide en los niveles que este modelo alcanza.");
        return std::process::ExitCode::SUCCESS;
    }

    // ── El margen ───────────────────────────────────────────────────────────
    let mut margen: Vec<String> = Vec::new();
    for (id, alcanzadas) in &alcance {
        if alcanzadas.is_empty() {
            margen.push(format!(
                "la política `{id}` no alcanza ninguna propiedad — no es un defecto: \
                 `Property in [Label, …]` existe para que una entidad quede gobernada el día \
                 que se etiqueta"
            ));
        }
    }
    for r in pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Ruleset)
    {
        let q = r.qname().unwrap_or_default();
        for a in r.section("assertions").map(|n| n.items()).unwrap_or(&[]) {
            let id = a.get("id").and_then(|(_, v)| v.as_str()).unwrap_or("?");
            let tipo = a
                .get("type")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("library");
            let sev = a
                .get("severity")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("error");
            if sev == "warning" {
                margen.push(format!(
                    "`{q}#{id}` es `severity: warning` y **no cuenta**: un aviso es, por \
                     definición, «lo vimos y no paramos nada»"
                ));
            } else if tipo == "text" || tipo == "custom" {
                margen.push(format!(
                    "`{q}#{id}` es `type: {tipo}` y **no cuenta**: se transporta sin \
                     interpretar, así que el compilador no sabe qué afirma"
                ));
            }
        }
    }
    if !margen.is_empty() {
        println!("\nAl margen — existen y no descargan nada:");
        for m in &margen {
            println!("  · {m}");
        }
    }

    println!(
        "\n{filas} propiedad(es) exigen gobierno, y las {filas} lo tienen: si alguna no lo \
         tuviera, esto no habría compilado."
    );
    std::process::ExitCode::SUCCESS
}
