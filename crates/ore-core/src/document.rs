//! Despacho de documentos y comprobaciones de forma.
//!
//! # Por qué no se usa un validador de JSON Schema
//!
//! Los esquemas publicados en `schemas/v1alpha1/` son **la mitad sintáctica de
//! L0**, y su destinatario son los consumidores externos: editores, linters en
//! otros lenguajes, acciones de CI. Ese es su trabajo y lo hacen bien.
//!
//! Para ORE serían maquinaria desproporcionada — los validadores 2020-12
//! disponibles pesan entre 73 y 192 crates, arrastran FFI de plataforma y
//! producen mensajes como `/spec/primaryKey: minItems`. La tesis del proyecto es
//! que **el error es el producto**, y ese es exactamente el error que criticamos.
//!
//! Aquí las siete formas se comprueban de forma nativa y tipada, con posición y
//! con un mensaje que dice qué hacer. La deriva contra los esquemas la atrapa la
//! suite de conformidad, que es el árbitro de ambos.
//!
//! Registro: `docs/decisions/0002-sin-validador-de-json-schema.md`

use crate::parse::Node;

/// Qué falla una regla de forma: el mensaje y, si la hay, qué hacer al respecto.
pub type ShapeFailure = (String, Option<String>);

pub const API_VERSION: &str = "oos.dev/v1alpha1";

/// Las versiones de la especificación que esta implementación entiende.
///
/// Deja de ser una constante en el momento en que hay dos, y el cambio no es
/// cosmético: **el documento elige su conjunto de reglas**. Un `kind` que no
/// existía en v1alpha1 no es un error de escritura, es un documento de otra
/// versión, y decirlo así es la diferencia entre «no reconozco esto» y «esto
/// existe, en otra versión».
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApiVersion {
    V1Alpha1,
    V1Alpha2,
    V1Alpha3,
    V1Alpha4,
    /// v1alpha5 y v1alpha6 no estan, y no faltan: no anadieron gramatica, asi
    /// que ningun documento puede declararlas. **Un `apiVersion` es
    /// consecuencia de haber anadido un `kind`**, igual que un directorio de
    /// esquemas. v1alpha7 anade uno —la vista— y por eso existe aqui.
    V1Alpha7,
    /// v1alpha8. Anade `Table`, adelgaza `View` y **retira** `Binding`.
    ///
    /// Es la primera version que quita algo, y por eso `Kind` necesita `hasta`
    /// al lado de `since`: un `Binding` que declare esta version no es un kind
    /// desconocido ni uno del futuro — es uno del pasado, y el error tiene que
    /// decir eso y decir por que dos documentos.
    V1Alpha8,
}

impl ApiVersion {
    pub const ALL: &'static [ApiVersion] = &[
        ApiVersion::V1Alpha1,
        ApiVersion::V1Alpha2,
        ApiVersion::V1Alpha3,
        ApiVersion::V1Alpha4,
        ApiVersion::V1Alpha7,
        ApiVersion::V1Alpha8,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ApiVersion::V1Alpha1 => "oos.dev/v1alpha1",
            ApiVersion::V1Alpha2 => "oos.dev/v1alpha2",
            ApiVersion::V1Alpha3 => "oos.dev/v1alpha3",
            ApiVersion::V1Alpha4 => "oos.dev/v1alpha4",
            ApiVersion::V1Alpha7 => "oos.dev/v1alpha7",
            ApiVersion::V1Alpha8 => "oos.dev/v1alpha8",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_str() == s)
    }
}

/// Los cinco documentos de v1alpha1, más el manifiesto raíz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    OntologyConfig,
    Package,
    Entity,
    Binding,
    Lattice,
    ConduitPolicy,
    /// v1alpha2. La superficie de efecto.
    Function,
    /// v1alpha2. El efecto sobre la identidad.
    Resolution,
    /// v1alpha3. La regla que apunta.
    Ruleset,
    /// v1alpha4. **El concepto: qué ES un dato**, no una propiedad de una
    /// entidad.
    ///
    /// Se llamó `Property` hasta que la pregunta llegó de fuera: *«¿nuestro
    /// `properties` no cubre ya el tema propiedades?»*. No lo cubre —son las dos
    /// puntas de `is:`— pero el nombre invitaba a creerlo, y el directorio
    /// `concepts/` ya delataba la duda.
    ///
    /// Peor: **`Property` era dos cosas a la vez.** En Cedar es el tipo de
    /// entidad de un CAMPO —`Property::"hr.Employee.nationalId"`— y aquí era el
    /// concepto. Renombrar no crea una colisión: deshace la que había.
    Concept,
    /// v1alpha4. La forma: un conjunto de entidades nombrado por lo que tienen.
    Interface,
    /// v1alpha1. La frontera que faltaba: que entra con una peticion y quien
    /// responde de que sea cierto.
    ///
    /// Las otras dos fronteras estaban declaradas —`datasources` la entrada de
    /// datos, `ConduitPolicy` la salida— y la de identidad no. Y es la unica
    /// entrada que DECIDE en vez de ser gobernada.
    RequestPolicy,
    /// v1alpha7. **La vista: que existe fisicamente y como se llama.**
    ///
    /// Absorbe al `Binding` entero —fuente, mapeo, capacidades, copia— y le
    /// invierte la flecha: el binding nombraba a la entidad, y ahora la entidad
    /// la nombra a ella con `backedBy`. Asi una vista existe antes de que nadie
    /// modele nada, varias entidades pueden respaldarse de la misma, y lo
    /// fisico deja de saber de significado. Y puede salir de OTRA vista, que es
    /// lo que hace que un pipeline sea una estructura y no una frase.
    ///
    /// Convivieron mientras duro la migracion, y el dia que `Binding` se
    /// retiro este comentario se quedo. `Kind::Binding` sigue ahi con su
    /// `hasta`, porque un documento v1alpha1 no caduca.
    View,
    /// v1alpha8. **La tabla: el puntero a un objeto fisico, registrado una vez,
    /// con sus dos caras.**
    ///
    /// v1alpha7 metio el puntero DENTRO de la vista porque el binding lo tenia
    /// asi. Era el puente correcto y la forma equivocada para quedarse: el
    /// contrato es del OBJETO, no de quien lo consulta. Cada vista sobre una
    /// fuente repetia el contrato fisico, una vista sobre otra vista no tenia
    /// ninguno, y las columnas de la fuente no se declaraban en ninguna parte —
    /// por eso `OOS2018` sobre una vista de fuente no era comprobable: no habia
    /// contra que.
    ///
    /// Las dos caras son `reads` —que se le puede pedir, el `capabilities`
    /// mudado— y `changes` —que cambios emite y como los codifica, que es lo
    /// que `version.witness` decia a medias: aquel decia que FECHA el cambio y
    /// no decia que LLEGA.
    ///
    /// **No hay `kind: Stream`**, y no es un olvido: un stream es el nombre
    /// corriente de una tabla cuya cara de lectura es `none`.
    Table,
}

impl Kind {
    pub const ALL: &'static [Kind] = &[
        Kind::OntologyConfig,
        Kind::Package,
        Kind::Entity,
        Kind::Binding,
        Kind::Lattice,
        Kind::ConduitPolicy,
        Kind::Function,
        Kind::Resolution,
        Kind::Ruleset,
        Kind::Concept,
        Kind::Interface,
        Kind::RequestPolicy,
        Kind::View,
        Kind::Table,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::OntologyConfig => "OntologyConfig",
            Kind::Package => "Package",
            Kind::Entity => "Entity",
            Kind::Binding => "Binding",
            Kind::Lattice => "Lattice",
            Kind::ConduitPolicy => "ConduitPolicy",
            Kind::Function => "Function",
            Kind::Resolution => "Resolution",
            Kind::Ruleset => "Ruleset",
            Kind::Concept => "Concept",
            Kind::Interface => "Interface",
            Kind::RequestPolicy => "RequestPolicy",
            Kind::View => "View",
            Kind::Table => "Table",
        }
    }

    /// La versión en la que este documento aparece.
    ///
    /// Un `Function` en un paquete de v1alpha1 no es un `kind` desconocido: es
    /// un documento del futuro, y el error tiene que decir eso.
    pub const fn since(self) -> ApiVersion {
        match self {
            Kind::Function | Kind::Resolution => ApiVersion::V1Alpha2,
            Kind::Ruleset => ApiVersion::V1Alpha3,
            Kind::Concept | Kind::Interface => ApiVersion::V1Alpha4,
            Kind::View => ApiVersion::V1Alpha7,
            Kind::Table => ApiVersion::V1Alpha8,
            _ => ApiVersion::V1Alpha1,
        }
    }

    /// La version en la que este documento **se retira**, si se retira.
    ///
    /// El gemelo de `since`, y hasta v1alpha8 no hacia falta porque ninguna
    /// version habia quitado nada. Se retira NO ES SE BORRA: un `Binding` que
    /// declare v1alpha1 sigue compilando, porque v1alpha1 es normativo y sigue
    /// diciendo lo que decia. Lo que no cabe es declarar la version nueva y
    /// usar la forma vieja.
    ///
    /// Que sea un dato del `kind` y no un `if` en el validador es lo que hace
    /// que el mensaje pueda decir **en que version** y **por que**: un
    /// `Binding` en v1alpha8 son una `Table` y una `View`, y decirlo en dos
    /// documentos es lo que permite que dos vistas compartan un objeto sin
    /// repetir su contrato.
    pub const fn hasta(self) -> Option<ApiVersion> {
        match self {
            Kind::Binding => Some(ApiVersion::V1Alpha8),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// Claves admitidas en la raíz del documento.
    pub const fn root_keys(self) -> &'static [&'static str] {
        &["apiVersion", "kind", "metadata", "spec"]
    }

    /// Claves admitidas bajo `metadata`.
    pub const fn metadata_keys(self) -> &'static [&'static str] {
        match self {
            Kind::OntologyConfig => &["name", "version", "description"],
            Kind::Package => &[
                "name",
                "version",
                "status",
                "domain",
                "id",
                "tenant",
                "tags",
                "description",
            ],
            Kind::Entity => &["name", "namespace", "labels", "description", "aiContext"],
            // `Function` no admite `labels`, y la ausencia es normativa: su
            // integridad SE COMPUTA de sus endosos (`02-function` §6). Admitir
            // una etiqueta dejaría que una función escribiera `attested` sobre
            // sí misma sin que exista atestación, y una afirmación sobre uno
            // mismo no es una garantía. Al no existir el campo, el error es
            // estructural — `OOS1005` — en vez de necesitar un código propio.
            // `Resolution` tampoco admite `labels`, y por lo mismo: la
            // integridad que puede producir se deriva de sus estrategias.
            // `Ruleset` tampoco, y por una razón distinta: **no porta datos**,
            // luego no tiene clasificación. Un campo del que nada se computa
            // acaba adquiriendo un significado que nadie escribió.
            Kind::Binding
            | Kind::Lattice
            | Kind::ConduitPolicy
            | Kind::Function
            | Kind::Resolution
            | Kind::Ruleset
            | Kind::Interface
            // `RequestPolicy` es como `ConduitPolicy`: uno por paquete, sin
            // espacio de nombres. No porta datos, luego no tiene clasificacion.
            | Kind::RequestPolicy
            // Una vista tampoco admite `labels`, y es la decision de forma que
            // la define: NO LLEVA SIGNIFICADO. Las etiquetas viven en la
            // entidad y en el datasource; si la vista pudiera declararlas habria
            // dos sitios diciendo que es una columna, y el dia que discrepen
            // ninguno diria cual manda.
            | Kind::View
            // Y la tabla tampoco, por lo mismo que la vista y con mas razon:
            // es el objeto tal cual esta. Su ubicacion la etiqueta el
            // `datasource`, y lo que significa una columna lo dice la entidad.
            | Kind::Table => &["name", "namespace", "description"],
            // `Property` es el único documento con `labels` en LOS DOS SITIOS,
            // y la primera versión de este `match` se lo negó por miedo a la
            // duplicación. Era un error, y lo destapó `confidence`: un concepto
            // acuñado por inferencia tiene que poder declararse `DRAFT`, y la
            // madurez de un documento se declara donde se declara siempre.
            //
            // No son dos superficies para lo mismo porque **el sujeto es
            // distinto**, y así hay que leerlas:
            //
            // - `metadata.labels` clasifica ESTE DOCUMENTO — su madurez.
            // - `spec.labels` clasifica EL DATO que lleve este concepto, y es
            //   lo que hereda una propiedad que declare `is`.
            //
            // Es la misma distinción que en `Entity`, que lleva `labels` en
            // `metadata` y otras dentro de cada propiedad sin que nadie las
            // confunda.
            Kind::Concept => &["name", "namespace", "labels", "description"],
        }
    }

    /// Claves admitidas bajo `spec`.
    pub const fn spec_keys(self) -> &'static [&'static str] {
        match self {
            Kind::OntologyConfig => &[
                "workspace",
                "dependencies",
                "datasources",
                // Las claves públicas en las que este árbol confía. Van aquí y
                // no dentro de un `.oob` porque una clave que viniera con el
                // paquete que firma cerraría el círculo: quien sustituye el
                // paquete sustituye la clave, firma con la suya, y todo
                // verifica. La confianza la declara quien consume o no la
                // declara nadie.
                "trustedKeys",
                // Y los logs en los que confía, que es lo mismo un piso más
                // arriba: una firma dice de quién es un paquete, y un log dice
                // que esa clave no le ha dicho dos cosas distintas a dos
                // personas. La clave del log la declara quien consume por la
                // misma razón, y con más motivo — es la que convierte una raíz
                // en la afirmación de alguien.
                "trustedLogs",
            ],
            Kind::Package => &[
                "owner",
                "team",
                "roles",
                "support",
                "sla",
                "authoritativeDefinitions",
                "dependencies",
            ],
            Kind::Entity => &[
                "nature",
                "principal",
                "primaryKey",
                "timeKey",
                "uniqueKeys",
                "temporal",
                "properties",
                "relations",
                "moved",
                "reserved",
                // v1alpha4. La forma que la entidad declara satisfacer.
                "implements",
                // v1alpha7. La vista que la respalda. Es `Binding.targetEntity`
                // con la flecha al reves: lo fisico existe antes y no sabe de
                // esto; la entidad elige de que vista salir.
                "backedBy",
            ],
            Kind::Binding => &[
                "targetEntity",
                "datasourceRef",
                "profile",
                "source",
                "selector",
                "properties",
                "capabilities",
                "materialization",
            ],
            // `axis` es lo único que v1alpha2 añade al retículo, y de él sale
            // el combinador. `join` queda obsoleto: derivable, luego no
            // declarable (P2), y si aparece debe coincidir — `OOS7007`.
            // `requiresGovernance` es lo que v1alpha3 añade, y va aquí y no en
            // el `Ruleset` a propósito: importar el paquete de clasificación
            // importa **su exigencia**, y eso es lo que hace que «GDPR como
            // dependencia» deje de ser una metáfora.
            Kind::Lattice => &[
                "levels",
                "levelDescriptions",
                "join",
                "axis",
                "requiresGovernance",
            ],
            Kind::ConduitPolicy => &["owner", "conduits"],
            Kind::Function => &[
                "runtime",
                "entrypoint",
                "source",
                "limits",
                "input",
                "output",
                "preconditions",
                "effects",
                "endorsements",
                "authorization",
                "idempotency",
            ],
            Kind::Resolution => &["entity", "sources", "strategies", "endorsements"],
            Kind::RequestPolicy => &["owner", "issuer", "subject", "claims", "purposes"],
            Kind::Ruleset => &[
                "owner",
                "targets",
                "assertions",
                "masks",
                "scopes",
                "duties",
            ],
            // La línea que decide qué cabe en un concepto: **declara lo que es
            // cierto de él en todas partes**. `required`, `unique` y `temporal`
            // no están porque dependen de la tabla, no del significado; `enum`
            // y `aiContext` sí, porque un código de moneda es ISO 4217 en los
            // quince sistemas y los sinónimos de un concepto son los mismos en
            // todos. `derivedFrom` y `expression` tampoco: un correo personal
            // significa lo mismo se calcule como se calcule.
            Kind::Concept => &[
                "type",
                "labels",
                "description",
                "enum",
                "aiContext",
                // v1alpha4 · la exigencia CATEGÓRICA. Un retículo exige por
                // nivel; un concepto, por ser lo que es. Mismo nombre que en el
                // retículo a propósito: es la misma noción sobre otro sujeto, y
                // otro nombre habría sido el error de los dos nombres para un
                // concepto.
                "requiresGovernance",
                "confidence",
            ],
            Kind::Interface => &["requires", "description"],
            // v1alpha7. Lo que era del binding, mudado, y dos cosas nuevas: de
            // que vista se sale (`from.view`) y con que testigo se versiona.
            // `where` es el `selector`, con la misma gramatica cerrada y por lo
            // mismo. `materialized` y `freshness` son la `materialization`
            // partida en dos: donde vive la copia, y cuanto retraso se tolera.
            Kind::View => &[
                "owner",
                "from",
                "version",
                "freshness",
                "fields",
                "where",
                "capabilities",
                "materialized",
            ],
            // v1alpha8. Las dos caras y lo que hay entre ellas: `columns`, que
            // es lo unico nuevo de verdad — hasta aqui ningun documento decia
            // que columnas tenia el objeto, y por eso `OOS2018` sobre una vista
            // de fuente no era comprobable.
            // `profile` es opcional y llego con la migracion: `Binding.profile`
            // no tenia donde ir, y v1alpha7 ya lo habia perdido sin que nadie
            // lo notara porque nada se habia migrado todavia.
            Kind::Table => &[
                "datasource",
                "object",
                "profile",
                "columns",
                "reads",
                "changes",
                // La cara `W`. A diferencia de las otras dos NO es obligatoria,
                // y esa asimetria es la doctrina: la ausencia es una negativa.
                // Callarse sobre lo que se PUEDE PEDIR dejaria al planificador
                // sin con que rechazar un plan; callarse sobre lo que se
                // ACEPTA ya rechaza.
                "writes",
            ],
        }
    }

    /// Las claves de `spec` **en una version**.
    ///
    /// Casi todas son las mismas en todas: un `kind` nace con su forma y la
    /// conserva. La `View` es la excepcion, y la excepcion es el asunto de
    /// v1alpha8: pierde `capabilities` y `version`, que pasan a ser `reads` y
    /// `changes.witness` de la tabla.
    ///
    /// Comprobar contra la union habria dejado compilar una vista v1alpha8 con
    /// el contrato fisico dentro — que es exactamente lo que esta version viene
    /// a impedir, y el defecto no produciria ningun sintoma: la vista compila,
    /// nadie lee ese `capabilities`, y el planificador usa el de la tabla. Un
    /// campo que nadie lee es peor que uno que no existe, porque promete algo.
    pub fn spec_keys_en(self, version: ApiVersion) -> &'static [&'static str] {
        match self {
            Kind::View if version >= ApiVersion::V1Alpha8 => &[
                "owner",
                "from",
                "freshness",
                "fields",
                "where",
                "materialized",
            ],
            _ => self.spec_keys(),
        }
    }

    /// Claves admitidas dentro de una propiedad de `Entity`.
    ///
    /// Se comprueban por lo mismo que las de `spec`: con `additionalProperties:
    /// true` una errata como `qualtiy:` se aceptaría en silencio y la propiedad
    /// quedaría sin gobernar. Aquí eso no es una molestia — es un hueco de
    /// gobierno que no produce ningún síntoma.
    ///
    /// `expression` no es nueva de v1alpha2: existe desde v1alpha1 como prosa
    /// documental. Lo que v1alpha2 cambia es su ESTATUTO —pasa a ser CEL y a
    /// comprobarse—, no su nombre. Un `expr` al lado habrían sido dos nombres
    /// para un concepto.
    ///
    /// Y `quality` NO está: el cuerpo de una aserción es `quality` de ODCS y su
    /// destino de emisión también, pero escribirla aquí sería una segunda
    /// superficie de autoría **sin dueño propio**. Vive en un `Ruleset`, que
    /// admite objetivos por nombre además de por predicado.
    pub const fn property_keys(self) -> &'static [&'static str] {
        &[
            "type",
            "labels",
            "description",
            "required",
            "unique",
            "temporal",
            "enum",
            "derivedFrom",
            "expression",
            "examples",
            "aiContext",
            // v1alpha4. El nombre es mío, el significado es del concepto.
            "is",
            "confidence",
        ]
    }

    /// `OntologyConfig` no lleva `spec`: sus secciones cuelgan de la raíz.
    pub const fn sections_at_root(self) -> bool {
        matches!(self, Kind::OntologyConfig)
    }
}

/// Una clave desconocida es un error salvo que declare ser una extensión de
/// proveedor: `x-<proveedor>-<lo que sea>`.
///
/// La estrictez es deliberada: con `additionalProperties: true` una errata como
/// `propertis:` se aceptaría en silencio y el campo real quedaría sin declarar.
pub fn is_extension(key: &str) -> bool {
    let Some(rest) = key.strip_prefix("x-") else {
        return false;
    };
    let Some((vendor, _)) = rest.split_once('-') else {
        return false;
    };
    !vendor.is_empty()
        && vendor
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Una regla de forma que no se puede expresar como «esta clave existe».
pub struct ShapeRule {
    pub kind: Kind,
    /// Ruta desde la raíz, p. ej. `["spec", "primaryKey"]`.
    pub path: &'static [&'static str],
    pub check: fn(&Node) -> Option<ShapeFailure>,
}

/// Las operaciones que la cara `W` declara, tal cual estan escritas.
///
/// **El unico lector de `writes` del arbol.** Lo llaman tres —la regla de forma
/// de aqui, `OOS2024` en `vistas` y `OOS7012` en `effect`— y una segunda
/// derivacion divergiria en la que ninguna prueba ejerce, que es la leccion de
/// `aristas`.
///
/// No valida: devuelve lo que hay. `none`, la ausencia y una lista vacia dan lo
/// mismo —nada—, porque para quien pregunta *«¿acepta update?»* las tres
/// significan lo mismo. Que una lista vacia ademas sea un defecto de forma lo
/// dice la regla, no esto.
pub fn escrituras(writes: Option<&Node>) -> Vec<String> {
    let Some(n) = writes else { return Vec::new() };
    if n.as_str().is_some() {
        return Vec::new();
    }
    n.items()
        .iter()
        .filter_map(|i| i.as_str())
        .map(String::from)
        .collect()
}

fn no_vacio(nombre: &'static str, ayuda: &'static str) -> impl Fn(&Node) -> Option<ShapeFailure> {
    move |n: &Node| {
        matches!(n, Node::Sequence { items, .. } if items.is_empty())
            .then(|| (format!("`{nombre}` está vacío"), Some(ayuda.to_string())))
    }
}

/// Reglas de forma comprobadas hoy. Crece con las fases; la suite de
/// conformidad dice cuáles faltan.
const AYUDA_NATURALEZA: &str = "el vocabulario es cerrado: `constraint`, `authorization`, \
     `obligation` y `transformation`. `derivation` no está, y no por olvido — produce \
     contenido, no lo gobierna. Y una naturaleza que no se reconoce no exige nada EN \
     SILENCIO, que es el peor de los dos fallos posibles";

fn naturaleza_desconocida(n: &crate::parse::Node) -> Option<String> {
    n.items()
        .iter()
        .filter_map(|i| i.as_str())
        .find(|s| !crate::governance::NATURALEZAS.contains(s))
        .map(String::from)
}

pub fn shape_rules() -> Vec<ShapeRule> {
    vec![
        // ── v1alpha8 · la tabla ─────────────────────────────────────────────
        //
        // Lo que un puntero necesita para serlo. Sin `columns` no es un puntero
        // a nada: es un nombre — y ademas es contra lo que se comprueba todo lo
        // demas, asi que su ausencia no da un error, da SILENCIO.
        ShapeRule {
            kind: Kind::Table,
            path: &["spec"],
            check: |n| {
                for (clave, ayuda) in [
                    (
                        "datasource",
                        "una tabla es el puntero a un objeto de una fuente declarada, y sin \
                         fuente no apunta a ninguna parte",
                    ),
                    (
                        "object",
                        "el nombre del objeto en el origen. Es opaco —sus reglas son del \
                         origen— pero tiene que estar",
                    ),
                    (
                        "reads",
                        "la cara `I`: que se le puede pedir. `none` es una respuesta legal y \
                         tiene consecuencias (OOS2020); no declararla no lo es, porque el \
                         planificador se quedaria sin con que rechazar un plan",
                    ),
                    (
                        "changes",
                        "la cara `D`: que cambios emite. `{ mode: none, witness: none }` es \
                         una respuesta legal —no se sabe, y no se inventa—; callarse no lo \
                         es, porque el mantenedor tendria que adivinar que pesos son legales",
                    ),
                ] {
                    if n.get(clave).is_none() {
                        return Some((format!("una tabla sin `{clave}`"), Some(ayuda.to_string())));
                    }
                }
                if n.get("columns").is_none_or(|(_, v)| v.entries().is_empty()) {
                    return Some((
                        "una tabla sin `columns`".to_string(),
                        Some(
                            "las columnas que HAY, no las que usa una vista. Es contra lo que se \
                             comprueba que un campo exista, y sin ellas esa comprobacion no \
                             falla: no se hace"
                                .to_string(),
                        ),
                    ));
                }
                None
            },
        },
        // La cara `W`, y la clave que ahora pueden pedir dos.
        //
        // Vive en `spec` y no en `spec.writes` porque la regla mira DOS
        // secciones: que `changes.key` este donde significa algo depende de
        // `changes` **y** de `writes`, y desde dentro de una no se ve la otra.
        ShapeRule {
            kind: Kind::Table,
            path: &["spec"],
            check: |n| {
                let w = n.get("writes").map(|(_, v)| v);
                if let Some(w) = w {
                    if let Some(lit) = w.as_str() {
                        if lit != "none" {
                            return Some((
                                format!("`writes: {lit}`"),
                                Some(
                                    "la cara `W` es o una lista de operaciones o el literal \
                                     `none`. El vocabulario es cerrado: `insert`, `update`, \
                                     `delete`"
                                        .to_string(),
                                ),
                            ));
                        }
                    } else if w.items().is_empty() {
                        return Some((
                            "`writes` esta vacio".to_string(),
                            Some(
                                "una lista vacia no dice nada que la ausencia no diga ya, y \
                                 obliga a leerla para descubrirlo. Si no acepta nada, quitala \
                                 o escribe `none`"
                                    .to_string(),
                            ),
                        ));
                    }
                }
                let ops = escrituras(w);
                let mut vistas: Vec<&str> = Vec::new();
                for op in &ops {
                    if !matches!(op.as_str(), "insert" | "update" | "delete") {
                        return Some((
                            format!("`{op}` no es una operacion de escritura"),
                            Some(
                                "el vocabulario es cerrado: `insert`, `update`, `delete`. No \
                                 hay `upsert` — es `insert` mas `update`, y el conjunto ya lo \
                                 dice sin una cuarta palabra"
                                    .to_string(),
                            ),
                        ));
                    }
                    if vistas.contains(&op.as_str()) {
                        return Some((
                            format!("`{op}` repetido en `writes`"),
                            Some(
                                "es un conjunto: repetirlo no acepta mas, solo deja dos sitios \
                                 donde puede haber uno"
                                    .to_string(),
                            ),
                        ));
                    }
                    vistas.push(op.as_str());
                }
                // Y la clave donde no significa nada. La disyuncion es de
                // v1alpha8: la puede pedir el modo, o la puede pedir la cara `W`.
                let modo = n
                    .get("changes")
                    .and_then(|(_, c)| c.get("mode"))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tiene_clave = n.get("changes").and_then(|(_, c)| c.get("key")).is_some();
                let la_pide_w = ops.iter().any(|o| o == "update" || o == "delete");
                if tiene_clave && modo != "upsert" && !la_pide_w {
                    return Some((
                        format!("`changes.key` con `mode: {modo}` y sin `writes` que la use"),
                        Some(
                            "la clave la pide el upsert —para saber que retira— o la cara `W` \
                             —para saber que fila actualiza—. Sin ninguno de los dos no la lee \
                             nadie, y un campo que nadie lee es peor que uno que no existe"
                                .to_string(),
                        ),
                    ));
                }
                None
            },
        },
        // `key` y `field` solo significan algo con su modo y su testigo. Que
        // FALTEN donde hacen falta deja al mantenedor sin saber que retirar;
        // que SOBREN donde no, promete algo que nadie lee — y un campo que
        // nadie lee es peor que uno que no existe.
        ShapeRule {
            kind: Kind::Table,
            path: &["spec", "changes"],
            check: |n| {
                let modo = n.get("mode").and_then(|(_, v)| v.as_str()).unwrap_or("");
                let testigo = n.get("witness").and_then(|(_, v)| v.as_str()).unwrap_or("");
                if modo == "upsert" && n.get("key").is_none() {
                    return Some((
                        "`changes.mode: upsert` sin `key`".to_string(),
                        Some(
                            "un upsert retira por clave: sin ella un tombstone no dice que fila \
                             quita, y el mantenedor aplicaria un -1 a nada"
                                .to_string(),
                        ),
                    ));
                }
                if testigo == "field" && n.get("field").is_none() {
                    return Some((
                        "`changes.witness: field` sin `field`".to_string(),
                        Some(
                            "el testigo por campo dice que una columna de la tabla ordena el \
                             avance. Cual, es lo unico que hace falta y lo unico que falta"
                                .to_string(),
                        ),
                    ));
                }
                if testigo != "field" && n.get("field").is_some() {
                    return Some((
                        format!("`changes.field` con `witness: {testigo}`"),
                        Some(
                            "la columna que ordena el avance solo la lee `witness: field`. Con \
                             otro testigo se ignora, y un campo que se ignora promete algo"
                                .to_string(),
                        ),
                    ));
                }
                None
            },
        },
        // Una cache ES UNA COPIA DE DATOS, y por eso el esquema le exige dos
        // cosas que a `topology` no: QUE copia y CUANTO tolera que envejezca.
        // Ninguna de las dos se comprobaba — un `payload: {}` validaba limpio,
        // que es justo la regla que justifica que el campo exista.
        ShapeRule {
            kind: Kind::Binding,
            path: &["spec", "materialization", "payload"],
            check: |n| {
                if n.get("properties")
                    .is_none_or(|(_, v)| v.items().is_empty())
                {
                    return Some((
                        "`payload` no declara `properties`".to_string(),
                        Some(
                            "una caché es una copia de datos, y quien la declara tiene que \
                             decir exactamente de qué. En `topology` no hace falta porque lo \
                             materializado es derivable; aquí no lo es"
                                .to_string(),
                        ),
                    ));
                }
                if n.get("freshnessSLA").is_none() {
                    return Some((
                        "`payload` no declara `freshnessSLA`".to_string(),
                        Some(
                            "una copia sin cota declarada es una copia que nadie va a notar \
                             ponerse mala. Y ese número es lo que acota la respuesta a una \
                             solicitud de supresión: un borrado en el origen tarda hasta ese \
                             plazo en propagarse"
                                .to_string(),
                        ),
                    ));
                }
                None
            },
        },
        // Un `event` no tiene identidad estable por registro, asi que no hay
        // sujeto que nombrar: `principal` sobre un evento declara algo que no
        // puede existir.
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec"],
            check: |n| {
                let principal = n.get("principal").and_then(|(_, v)| v.as_str()) == Some("true");
                let evento = n.get("nature").and_then(|(_, v)| v.as_str()) == Some("event");
                (principal && evento).then(|| {
                    (
                        "`principal: true` sobre `nature: event`".to_string(),
                        Some(
                            "un evento no tiene identidad estable por registro, asi que no                              hay sujeto que nombrar. Un principal es alguien, y `event`                              situa hechos en el tiempo en vez de identificar sujetos"
                                .to_string(),
                        ),
                    )
                })
            },
        },
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec", "primaryKey"],
            check: |n| {
                no_vacio(
                    "primaryKey",
                    "una clave vacía no identifica nada; declara al menos una propiedad, \
                     o usa `nature: event` con `timeKey` si los registros no tienen \
                     identidad estable",
                )(n)
            },
        },
        // v1alpha4 · el guardarraíl, y va aquí y no en una familia nueva
        // porque **el esquema lo expresa entero**: es un `oneOf` entre declarar
        // localmente y referenciar un concepto. El borrador de v1alpha4 reservó
        // `OOS9002` para esto y sobra — un incumplimiento de forma ya tiene
        // código, y es este. Inflar una familia por simetría con una tabla es
        // lo contrario de lo que P7 pide.
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec", "properties"],
            check: |n| {
                for (k, v) in n.entries() {
                    let Some(nombre) = k.as_str() else { continue };
                    let referencia = v.get("is").is_some();
                    // El guardarraíl alcanza a `type` y **no** a `labels**, y
                    // la asimetría no es un descuido: la primera versión
                    // prohibió las dos y se contradecía con `OOS4012`, que
                    // permite elevar la clasificación heredada. Elevarla exige
                    // escribirla, luego prohibir `labels` dejaba la elevación
                    // sin sintaxis.
                    //
                    // Los dos campos no son la misma clase de cosa:
                    //
                    // - `type` es una IGUALDAD. Redeclararlo solo puede coincidir
                    //   o contradecir, y en el segundo caso no hay nada a lo que
                    //   apelar para decidir quién gana. Se prohíbe.
                    // - `labels` es un ORDEN. Redeclararlas tiene un significado
                    //   definido —elevar— y un error definido —rebajar—, y la
                    //   regla que los separa existe desde v1alpha1.
                    //
                    // Por eso `OOS4012` «sube un nivel sin cambiar una letra»:
                    // porque aquí no hace falta ninguna.
                    if referencia && v.get("type").is_some() {
                        return Some((
                            format!(
                                "`{nombre}` declara `is` y también `type`: una propiedad declara                                  localmente o referencia un concepto, nunca las dos"
                            ),
                            Some(
                                "el tipo lo pone el concepto, y no hay orden al que apelar si la                                  copia deja de coincidir. La clasificación es otra cosa: esa se                                  puede escribir para ELEVARLA —`OOS4012`— y nunca para rebajarla"
                                    .into(),
                            ),
                        ));
                    }
                    // La otra mitad del `oneOf`, y estaba sin implementar. El
                    // esquema exige DECLARAR LOCALMENTE o REFERENCIAR UN
                    // CONCEPTO —v1alpha1 con `required: [type]`, v1alpha4 con el
                    // `oneOf` entero—, y aquí solo se comprobaba que no fueran
                    // las dos. Una propiedad sin ninguna de las dos validaba, y
                    // el emisor de GraphQL le inventaba `String` porque no tenía
                    // otra cosa que poner: una columna sin tipo salía en el
                    // contrato **tipada**, que es peor que no salir.
                    if !referencia && v.get("type").is_none() {
                        return Some((
                            format!("`{nombre}` no declara `type` ni `is`"),
                            Some(
                                "una propiedad declara su tipo localmente o referencia un \
                                 concepto que lo pone. Sin ninguna de las dos no hay tipo, y \
                                 lo que emite el contrato tendría que inventarlo"
                                    .into(),
                            ),
                        ));
                    }
                    if !referencia && v.get("confidence").is_some() {
                        return Some((
                            format!("`{nombre}` declara `confidence` sin `is`"),
                            Some(
                                "`confidence` es la confianza de UNA INFERENCIA, y sin mapeo no                                  hay nada de lo que dudar. Una propiedad escrita a mano no es una                                  inferencia: es una decisión, y nadie declara cuánta confianza                                  tiene en algo que acaba de decidir"
                                    .into(),
                            ),
                        ));
                    }
                }
                None
            },
        },
        // El vocabulario de naturalezas **no lo validaba nadie**, y es un
        // agujero que v1alpha3 ya tenía: `cobertura` filtra por la lista
        // cerrada, así que un `autorization` mal escrito no exigía nada **en
        // silencio**. Es `OOS8002` un piso más arriba, otra vez — una exigencia
        // que no exige nada tiene el mismo aspecto que una que sí.
        //
        // Se arregla en los dos sitios a la vez, que es lo que obliga a añadir
        // el segundo: si solo se validara el nuevo, el viejo quedaría peor por
        // comparación.
        ShapeRule {
            kind: Kind::Lattice,
            path: &["spec", "requiresGovernance"],
            check: |n| {
                for (nivel, v) in n.entries() {
                    let nivel = nivel.as_str().unwrap_or("?");
                    if let Some(mala) = naturaleza_desconocida(v) {
                        return Some((
                            format!("`{mala}` no es una naturaleza de regla, en `{nivel}`"),
                            Some(AYUDA_NATURALEZA.into()),
                        ));
                    }
                }
                None
            },
        },
        // El dueño de la superficie de SEGURIDAD.
        //
        // Era el único de los cuatro documentos que gobiernan sin declararlo, y
        // no porque no lo tuviera: el ejemplo de referencia lo llevaba escrito
        // **en un comentario** —*«CODEOWNERS exige revisión de @acme/security
        // para cualquier cambio en este fichero»*—. Un dueño en prosa no viaja
        // en el bundle, y lo que va a una auditoría es el bundle.
        //
        // Y es lo que da dueño a las políticas de Cedar, que son la única
        // superficie de gobierno que no lo tenía: quien eleva la autorización de
        // un conducto y quien escribe un `permit` son la misma persona.
        ShapeRule {
            kind: Kind::ConduitPolicy,
            path: &["spec"],
            check: |n| {
                n.get("owner").is_none().then(|| {
                    (
                        "un `ConduitPolicy` DEBE declarar `owner`".into(),
                        Some(
                            "elevar la autorización de un conducto es LA decisión de \
                             seguridad de este modelo, y un techo del que nadie responde es \
                             el hueco que este campo cierra. Usa `team:<handle>`, que es lo \
                             que se alinea con CODEOWNERS — y de él heredan las políticas de \
                             Cedar, que son la otra superficie sin dueño propio"
                                .into(),
                        ),
                    )
                })
            },
        },
        // La tercera frontera. Los cuatro campos son obligatorios y ninguno es
        // decorativo:
        //
        //   `owner`     — es OTRO equipo. Quien opera la identidad no modela el
        //                 dominio ni escribe las políticas.
        //   `issuer`    — contra quién se verifica la firma. `05-ejecutor` §6.1
        //                 ya exigía verificarla y no había contra qué.
        //   `subject`   — qué tipo es el sujeto, y qué reclamación lo nombra.
        //   `purposes`  — sin esto, `OOS4005` no tiene contra qué comprobar, y
        //                 una finalidad mal escrita deja de casar en silencio.
        ShapeRule {
            kind: Kind::RequestPolicy,
            path: &["spec"],
            check: |n| {
                for (k, ayuda) in [
                    (
                        "owner",
                        "quien opera la identidad no es quien modela el dominio",
                    ),
                    (
                        "issuer",
                        "sin emisor, «los atributos llegan firmados» no es comprobable",
                    ),
                    (
                        "subject",
                        "hace falta saber qué tipo es el sujeto para responder `resource in \
                         principal`",
                    ),
                    (
                        "purposes",
                        "una finalidad que nadie declara no falla: deja de casar, y el dato \
                         queda sin gobernar en silencio",
                    ),
                ] {
                    if n.get(k).is_none() {
                        return Some((
                            format!("un `RequestPolicy` DEBE declarar `{k}`"),
                            Some(ayuda.to_string()),
                        ));
                    }
                }
                None
            },
        },
        ShapeRule {
            kind: Kind::RequestPolicy,
            path: &["spec", "issuer"],
            check: |n| {
                // La audiencia importa tanto como el emisor: un token acuñado
                // para otro destinatario es un token robado, aunque lo firme
                // quien debe. Es `bound_audiences`, y el `aud` de un ID token.
                for k in ["url", "audience"] {
                    if n.get(k).is_none() {
                        return Some((
                            format!("`issuer` DEBE declarar `{k}`"),
                            Some(
                                "un token acuñado para otro destinatario es un token robado, \
                                 aunque lo firme quien debe"
                                    .into(),
                            ),
                        ));
                    }
                }
                None
            },
        },
        ShapeRule {
            kind: Kind::RequestPolicy,
            path: &["spec", "subject"],
            check: |n| {
                for k in ["entity", "claim"] {
                    if n.get(k).is_none() {
                        return Some((
                            format!("`subject` DEBE declarar `{k}`"),
                            Some(
                                "`entity` es el tipo del sujeto y `claim` la reclamación que lo \
                                 identifica: sin las dos no se sabe a quién se refiere el token"
                                    .into(),
                            ),
                        ));
                    }
                }
                None
            },
        },
        // Un ámbito de fila declara `id`, `property` y `matches`, y NADA más.
        //
        // No lleva operador, y eso no es una simplificación de la primera
        // versión: la igualdad es la única comparación que cierra el canal
        // lateral de `02-ruleset` §4.2.2 —el lado derecho es un atributo que el
        // principal ya traía, así que la presencia de una fila no le revela
        // nada nuevo—. Un `operator: greaterThan` volvería a abrirlo, y ofrecer
        // una elección que no existe es peor que no ofrecerla.
        ShapeRule {
            kind: Kind::Ruleset,
            path: &["spec", "scopes"],
            check: |n| {
                for s in n.items() {
                    for k in ["id", "property", "matches"] {
                        if s.get(k).is_none() {
                            return Some((
                                format!("un ámbito de fila DEBE declarar `{k}`"),
                                Some(
                                    "`property` es la columna que se recorta y `matches` el \
                                     NOMBRE del atributo del principal contra el que se compara. \
                                     Sin uno de los dos no hay filtro que construir"
                                        .into(),
                                ),
                            ));
                        }
                    }
                    for (k, _) in s.entries() {
                        let k = k.as_str().unwrap_or("?");
                        if !["id", "property", "matches"].contains(&k) {
                            return Some((
                                format!("`{k}` no es un campo de un ámbito de fila"),
                                Some(
                                    "un ámbito solo compara una columna con un atributo del \
                                     principal por igualdad. Cualquier otra cosa es un predicado, \
                                     y un predicado no filtra: lee"
                                        .into(),
                                ),
                            ));
                        }
                    }
                }
                None
            },
        },
        ShapeRule {
            kind: Kind::Concept,
            path: &["spec", "requiresGovernance"],
            check: |n| {
                if n.items().is_empty() {
                    return Some((
                        "`requiresGovernance` vacío".into(),
                        Some("omite el campo en lugar de exigir la nada".into()),
                    ));
                }
                naturaleza_desconocida(n).map(|mala| {
                    (
                        format!("`{mala}` no es una naturaleza de regla"),
                        Some(AYUDA_NATURALEZA.into()),
                    )
                })
            },
        },
        ShapeRule {
            kind: Kind::Concept,
            path: &["spec"],
            check: |n| {
                n.get("type").is_none().then(|| {
                    (
                        "un `Property` DEBE declarar `type`".into(),
                        Some(
                            "es la mitad de lo que el concepto declara —la otra es `labels`— y es                              lo que hereda toda propiedad que lo referencie"
                                .into(),
                        ),
                    )
                })
            },
        },
        ShapeRule {
            kind: Kind::Interface,
            path: &["spec"],
            check: |n| {
                n.get("requires").is_none().then(|| {
                    (
                        "un `Interface` DEBE declarar `requires`".into(),
                        Some(
                            "una forma sin exigencias la satisface cualquier cosa, y entonces no                              nombra ningún conjunto. Es `OOS8002` visto desde el otro lado"
                                .into(),
                        ),
                    )
                })
            },
        },
        ShapeRule {
            kind: Kind::Interface,
            path: &["spec", "requires"],
            check: |n| {
                no_vacio(
                    "requires",
                    "omite el documento en lugar de declarar una forma que no exige nada",
                )(n)
            },
        },
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec", "uniqueKeys"],
            check: |n| no_vacio("uniqueKeys", "omite el campo en lugar de declararlo vacío")(n),
        },
        // Las claves obligatorias de un `Ruleset`, y la disyunción que el
        // esquema expresa con `anyOf`. Van sobre `spec` entero y no sobre cada
        // clave porque una regla sobre una clave ausente no llega a correr:
        // una clave que falta no tiene nodo donde mirarla.
        ShapeRule {
            kind: Kind::Ruleset,
            path: &["spec"],
            check: |n| {
                if n.get("owner").is_none() {
                    return Some((
                        "un `Ruleset` DEBE declarar `owner`".into(),
                        Some(
                            "y es independiente del dueño de los paquetes a los que apunta: ahí \
                             está la razón de que esto sea un documento y no un bloque dentro de \
                             `Entity`. En un entorno regulado, quien responde del cumplimiento \
                             tiene que poder restringir la ontología sin poder editarla"
                                .into(),
                        ),
                    ));
                }
                if n.get("targets").is_none() {
                    return Some((
                        "un `Ruleset` DEBE declarar `targets`".into(),
                        Some(
                            "una regla sin objetivo es una regla que enumera, y para eso ya está \
                             `quality` de ODCS colgando de la propiedad"
                                .into(),
                        ),
                    ));
                }
                if ["assertions", "masks", "scopes", "duties"]
                    .iter()
                    .all(|k| n.get(k).is_none())
                {
                    return Some((
                        "este `Ruleset` no declara ninguna regla".into(),
                        Some(
                            "necesita al menos `assertions`, `masks`, `scopes` o `duties`: un \
                             objetivo sin nada que sostener selecciona propiedades y no las \
                             gobierna"
                                .into(),
                        ),
                    ));
                }
                None
            },
        },
        ShapeRule {
            kind: Kind::Lattice,
            path: &["spec", "levels"],
            check: |n| match n {
                Node::Sequence { items, .. } if items.len() < 2 => Some((
                    "`levels` necesita al menos dos niveles".into(),
                    Some("un retículo con un solo nivel no ordena nada".into()),
                )),
                _ => None,
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconoce_extensiones_de_proveedor() {
        assert!(is_extension("x-acme-owner"));
        assert!(is_extension("x-oos-dependencies"));
        // Sin proveedor, sin sufijo, o mayúsculas: no es una extensión válida.
        assert!(!is_extension("x-owner"));
        assert!(!is_extension("acmeOwner"));
        assert!(!is_extension("x-ACME-owner"));
    }

    #[test]
    fn los_kinds_se_resuelven() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(*k));
        }
        assert_eq!(Kind::parse("Ontology"), None);
    }

    #[test]
    fn function_es_de_v1alpha2_y_no_admite_etiquetas() {
        assert_eq!(Kind::Function.since(), ApiVersion::V1Alpha2);
        assert_eq!(Kind::Entity.since(), ApiVersion::V1Alpha1);
        // La ausencia que impide que una función se atestigüe a sí misma.
        assert!(!Kind::Function.metadata_keys().contains(&"labels"));
        assert!(Kind::Entity.metadata_keys().contains(&"labels"));
    }
}
