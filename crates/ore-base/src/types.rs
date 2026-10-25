//! **La gramática de tipos de OOS** (`Type`, `parse_type`, los supertipos, la nulabilidad):
//! cómo se escribe un tipo y qué significa. Es vocabulario (CI C4): la usan los drivers y el
//! lago. La comprobación de tipos sobre el árbol (la familia `OOS3xxx`) vive en
//! `ore_core::types`, que re-exporta esto.

/// Los escalares del conjunto cerrado. Los nombres se alinean con el enum de
/// Apache Ossie aunque no lo perfilemos: no cuesta nada y convierte la emisión
/// en un mapeo sin renombrados.
pub const ESCALARES: &[&str] = &[
    "String",
    "Integer",
    "Decimal",
    "Float",
    "Boolean",
    "Date",
    "Time",
    "DateTime",
    "DateTimeTz",
    // `Opaque` es la salida prevista para lo que OOS no modela: un blob existe
    // en la fuente, se puede etiquetar y gobernar, y el sistema de tipos no
    // necesita saber qué hay dentro.
    "Opaque",
];

/// **Si pasar de `de` a `a` ENSANCHA el conjunto de valores aceptados.**
///
/// # Por qué existe, y por qué es normativa
///
/// `diff` trataba cualquier cambio de tipo como `OOS5002`, cuyo texto es *«tipo
/// **estrechado**»*. El veredicto no siempre era falso, pero **la atribución
/// sí**: `Integer → Decimal` no estrecha nada, y un código que dice algo que no
/// pasó es lo que este árbol persigue.
///
/// Y la dirección ya estaba decidida un piso más abajo: la rama de los `enum`
/// dice *«retirar valores de un enum. **Añadirlos no rompe a quien lee**»*.
/// Esto es la misma frase sobre el escalar, y por eso ensanchar **no emite**.
///
/// # El único par, y eso es el hallazgo
///
/// Normativa: `02-entity` §3.4.
///
/// Sobre los diez escalares del conjunto cerrado la relación tiene **un
/// elemento**: `Integer → Decimal`. Todo entero cabe exacto en un decimal, y
/// aquí el decimal es exacto porque no hay coma flotante en ninguna parte.
///
/// Lo que vale de esta función es **lo que deja fuera**, porque son los pares
/// que alguien va a querer añadir «obviamente» algún día:
///
/// | par | por qué NO ensancha |
/// |---|---|
/// | `Integer`/`Decimal` → `Float` | pierde exactitud. `68400.50` no tiene representación exacta en binario, y es la regla más dura de este árbol |
/// | `Date` → `DateTime` | una fecha no es un instante. Ponerle una hora es **inventarla** |
/// | `DateTime` → `DateTimeTz` | ídem con la zona |
/// | cualquiera → `String` | una cadena **representa** el valor, no lo contiene: el contrato de lectura cambia entero |
/// | cualquiera → `Opaque` | `Opaque` es *«no lo modelamos»*. Ir ahí no ensancha el dominio: **retira el gobierno** |
/// | `Boolean` → `Integer` | eso es elegir una codificación, no ampliar un dominio |
///
/// Y **entre decimales con precisión** (02-entity §3.4, 2026-09-26):
/// `Decimal<p₁, s₁>` ensancha a `Decimal<p₂, s₂>` cuando no pierde ninguna
/// cifra por ningún lado —`s₂ ≥ s₁` y `p₂ − s₂ ≥ p₁ − s₁`—, e `Integer` a
/// `Decimal<p, s>` solo con `p − s ≥ 19`, las cifras del entero de 64 bits.
/// Declarar o retirar la precisión (`Decimal` ↔ `Decimal<p, s>`) no ensancha
/// en ninguna dirección.
///
/// Un paramétrico de unidad —`Money<EUR,2>`— lo clasifica `OOS5010`, que es
/// más específico y va antes.
pub fn ensancha(de: &str, a: &str) -> bool {
    match (parse_type(de), parse_type(a)) {
        (
            Ok(Type::Decimal {
                precision: p1,
                escala: s1,
            }),
            Ok(Type::Decimal {
                precision: p2,
                escala: s2,
            }),
        ) => s2 >= s1 && p2 - s2 >= p1 - s1,
        (Ok(Type::Scalar(i)), Ok(Type::Decimal { precision, escala })) if i == "Integer" => {
            precision - escala >= CIFRAS_DEL_ENTERO
        }
        _ => matches!((de, a), ("Integer", "Decimal")),
    }
}

/// Las cifras del mayor entero de 64 bits, `9 223 372 036 854 775 807`: el
/// `Integer` de las copias (0032) visto como decimal es `Decimal<19, 0>`.
pub const CIFRAS_DEL_ENTERO: u8 = 19;

/// El techo de la precisión: el de Iceberg y Parquet (`decimal128`).
pub const PRECISION_MAXIMA: u8 = 38;

/// **El supertipo de dos decimales** (02-entity §3.5): la mayor escala y la
/// mayor parte entera. Una unión, las dos ramas de un `CASE`, los dos lados de
/// una comparación. `None` si no cabe en [`PRECISION_MAXIMA`]: no hay decimal
/// exacto donde quepan los dos, y redondear sería elegir en silencio qué
/// cifras se pierden.
pub fn supertipo_decimal(a: (u8, u8), b: (u8, u8)) -> Option<(u8, u8)> {
    let escala = a.1.max(b.1);
    let enteras = (a.0 - a.1).max(b.0 - b.1);
    let precision = enteras.checked_add(escala)?;
    (precision <= PRECISION_MAXIMA).then_some((precision, escala))
}

/// Lo que da juntar dos tipos donde tiene que salir uno: los dos lados de una
/// comparación, las ramas de una unión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Supertipo {
    Es(Type),
    /// No hay: tipos distintos que no se mezclan sin decirlo.
    Ninguno,
    /// Dos decimales cuyo supertipo pasaría de 38 cifras (02-entity §3.5).
    Desborda {
        izquierda: Type,
        derecha: Type,
    },
}

/// **El supertipo de dos tipos** (02-entity §3.5). Iguales, él mismo; dos
/// `Decimal<p, s>`, la mayor escala y la mayor parte entera; un `Decimal<p, s>`
/// y un `Decimal` sin precisión —un literal, que se escribe `"100"`—, `Decimal`,
/// porque la precisión deja de saberse. Todo lo demás, ninguno: un `Integer` no
/// se mezcla con un decimal sin decirlo, con precisión o sin ella.
pub fn supertipo(a: &Type, b: &Type) -> Supertipo {
    let decimal_suelto = |t: &Type| matches!(t, Type::Scalar(s) if s == "Decimal");
    match (a, b) {
        _ if a == b => Supertipo::Es(a.clone()),
        (
            Type::Decimal {
                precision: p1,
                escala: s1,
            },
            Type::Decimal {
                precision: p2,
                escala: s2,
            },
        ) => match supertipo_decimal((*p1, *s1), (*p2, *s2)) {
            Some((precision, escala)) => Supertipo::Es(Type::Decimal { precision, escala }),
            None => Supertipo::Desborda {
                izquierda: a.clone(),
                derecha: b.clone(),
            },
        },
        (Type::Decimal { .. }, o) | (o, Type::Decimal { .. }) if decimal_suelto(o) => {
            Supertipo::Es(Type::Scalar("Decimal".into()))
        }
        _ => Supertipo::Ninguno,
    }
}

/// `sum(Decimal<p, s>)` → `Decimal<38, s>` (02-entity §3.5).
pub fn suma_decimal(t: (u8, u8)) -> (u8, u8) {
    (PRECISION_MAXIMA, t.1)
}

/// `avg(Decimal<p, s>)` → `Decimal<38, max(s, 9)>` (02-entity §3.5): la de
/// BigQuery. La de DuckDB, `DOUBLE`, cambia un exacto por un binario.
pub fn media_decimal(t: (u8, u8)) -> (u8, u8) {
    (PRECISION_MAXIMA, t.1.max(9))
}

/// El conjunto cerrado, para quien tenga que OFRECERLO.
///
/// `ore review` pregunta por el tipo de una columna que el lector no supo
/// traducir, y las opciones que ofrece son estas. Se exponen en vez de copiarse
/// porque una lista escrita a mano en otro fichero envejece en silencio la
/// primera vez que el conjunto crezca — que es la figura que este repositorio
/// lleva encontrando.
pub fn escalares() -> &'static [&'static str] {
    ESCALARES
}

/// Un tipo ya analizado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Scalar(String),
    /// `Money<EUR, 2>` · `Quantity<km, 1>`. La unidad es parte del tipo, no una
    /// anotación: sin ella, sumar euros y dólares no falla — solo da cifras
    /// incorrectas.
    Parametric {
        ctor: String,
        unit: String,
        precision: u32,
    },
    /// `list<T>`. Hasta v1alpha16, `T` es un escalar; v1alpha17 admite además
    /// `Struct<…>` y `Anchor` (`01-los-tipos` §2).
    List(Box<Type>),
    /// v1alpha17. `Struct<a: T, b: U>`: campos con nombre y tipo, **en orden**
    /// —el orden es parte del tipo, como en Iceberg, Parquet y DuckDB—
    /// (`01-los-tipos` §1).
    Struct(Vec<(String, Type)>),
    /// v1alpha17. `Vector<n>`: exactamente `n` valores `Float32`, `1 ≤ n ≤
    /// 16000`. No es `list<Float>` porque la dimensión es del contrato: un
    /// índice de vecinos y un modelo de embeddings solo se entienden con ella
    /// (`01-los-tipos` §2).
    Vector(u16),
    /// v1alpha17. La parte de un medio a la que se refiere un resultado: el
    /// selector de W3C Web Annotation, tipado (`02-el-ancla`).
    Anchor,
    /// `Decimal<p, s>` (02-entity §3.2): el decimal con su precisión y su
    /// escala, `1 ≤ p ≤ 38` y `0 ≤ s ≤ p`. `Decimal` a secas es
    /// `Scalar("Decimal")` y dice otra cosa: la precisión no se declaró.
    Decimal {
        precision: u8,
        escala: u8,
    },
    /// `iso.CountryAlpha2`. Su resolución es trabajo de dependencias.
    Imported(String),
    /// v1alpha16. `Media<legal.archivo.contratos>`: **la referencia a un ítem
    /// de una colección** (`03` §2). El valor es la huella del ítem, no el
    /// fichero; lleva el tipo de medio y la clasificación de su colección. La
    /// referencia se guarda tal como se escribió y la resuelve el enlazado
    /// (`OOS2018`), como `backedBy`.
    Media(String),
    /// v1alpha32. `Geometry<Polygon, EPSG:25830>`: geometría **plana** con su
    /// subtipo (`Any`, `Point`, …, con `Z`/`M`/`ZM`) y su CRS, fiel al origen
    /// (`01-la-geometria` §1). Por defecto `Any` y `OGC:CRS84`.
    Geometry {
        subtipo: String,
        crs: String,
    },
    /// v1alpha32. `Geography<LineString>`: sobre el elipsoide, aristas
    /// geodésicas, CRS siempre `OGC:CRS84` (`01-la-geometria` §2).
    Geography {
        subtipo: String,
    },
}

impl Type {
    /// La unidad, si el tipo la tiene. Es lo único que `OOS3004` necesita.
    pub fn unit(&self) -> Option<&str> {
        match self {
            Type::Parametric { unit, .. } => Some(unit),
            _ => None,
        }
    }
}

/// Cómo se escribe un tipo ya analizado.
///
/// Existía `parse_type` y no existía la vuelta, y eso es un agujero en cuanto
/// alguien tiene que **enseñar** un tipo o meterlo en una forma canónica. La
/// invariante es que `parse_type(t.to_string()) == t` para todo tipo que se
/// haya analizado, y hay una prueba que la ejerce sobre el conjunto cerrado
/// entero — si no, habría dos escrituras del mismo tipo y ninguna diría cuál
/// manda.
impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Scalar(s) | Type::Imported(s) => f.write_str(s),
            Type::Media(c) => write!(f, "Media<{c}>"),
            Type::List(t) => write!(f, "list<{t}>"),
            // La forma canónica: un espacio tras cada `:` y cada `,`
            // (`01-los-tipos` §1). `parse_type` acepta sin espacios y esto
            // escribe con ellos, y así el digest del paquete no depende de cómo
            // se tecleó.
            Type::Struct(campos) => {
                f.write_str("Struct<")?;
                for (i, (n, t)) in campos.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{n}: {t}")?;
                }
                f.write_str(">")
            }
            Type::Vector(n) => write!(f, "Vector<{n}>"),
            Type::Anchor => f.write_str("Anchor"),
            // v1alpha32 · la forma canónica omite lo que vale por defecto: el
            // subtipo `Any` y el CRS `OGC:CRS84` (`01-la-geometria` §1).
            Type::Geometry { subtipo, crs } => match (subtipo.as_str(), crs.as_str()) {
                (GEO_SUBTIPO_DEFECTO, CRS_DEFECTO) => f.write_str("Geometry"),
                (s, CRS_DEFECTO) => write!(f, "Geometry<{s}>"),
                (s, c) => write!(f, "Geometry<{s}, {c}>"),
            },
            Type::Geography { subtipo } if subtipo == GEO_SUBTIPO_DEFECTO => {
                f.write_str("Geography")
            }
            Type::Geography { subtipo } => write!(f, "Geography<{subtipo}>"),
            Type::Decimal { precision, escala } => write!(f, "Decimal<{precision}, {escala}>"),
            Type::Parametric {
                ctor,
                unit,
                precision,
            } => write!(f, "{ctor}<{unit}, {precision}>"),
        }
    }
}

/// **Si una columna puede ser nula** (ORE 0051, «ORE Null Contract»).
///
/// Va pegada al tipo de cada columna y es la misma en todas las capas: lo que
/// el origen **declara** (`Table.columns.<c>.required`, v1alpha22), lo que una
/// vista **deriva** de lo que lee, y lo que no se sabe. Ninguna capa lo decide
/// con un `bool` suyo.
///
/// El orden importa: `Nulable < Derivada < Garantizada`, y combinar dos ramas
/// toma la menor ([`Nulabilidad::y`]). Ante la duda, nulable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Nulabilidad {
    /// Puede ser nula. Lo de siempre, y lo que vale sin una garantía.
    #[default]
    Nulable,
    /// Nunca nula porque se deriva de lo que la consulta lee (v1alpha22
    /// `01-nunca-nula` §4): una columna leída tal cual de una garantizada, un
    /// `COUNT`.
    Derivada,
    /// Nunca nula porque **el origen lo garantiza**: un `NOT NULL`, un
    /// `mode: REQUIRED`, un campo `required` de Parquet o de Iceberg.
    Garantizada,
}

impl Nulabilidad {
    /// ¿Nunca es nula?
    pub const fn nunca_nula(self) -> bool {
        !matches!(self, Nulabilidad::Nulable)
    }

    /// Dos ramas que dan la misma columna (un `CASE`, un `UNION`): nunca nula
    /// sólo si las dos lo son.
    pub fn y(self, otra: Nulabilidad) -> Nulabilidad {
        self.min(otra)
    }

    /// Lo que se lee tal cual aguas abajo: la garantía del origen llega como
    /// derivada, porque quien la afirma ya no es el origen sino la consulta.
    pub const fn aguas_abajo(self) -> Nulabilidad {
        match self {
            Nulabilidad::Garantizada => Nulabilidad::Derivada,
            otra => otra,
        }
    }

    /// Lo que se añade a la forma de texto del tipo: `!` sólo cuando nunca es
    /// nula. Así un tipo sin garantía se escribe como siempre, y nada que se
    /// calcule de su texto —la cabecera de una copia, el digest de un plan—
    /// cambia en un árbol que no declara `required`.
    pub const fn sufijo(self) -> &'static str {
        if self.nunca_nula() { "!" } else { "" }
    }
}

/// **El tipo de una columna con su nulabilidad.** Se escribe como su tipo, más
/// `!` si nunca es nula ([`Nulabilidad::sufijo`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tipado {
    pub tipo: Type,
    pub nulabilidad: Nulabilidad,
}

impl Tipado {
    /// Un tipo sin garantía: lo de siempre.
    pub fn nulable(tipo: Type) -> Tipado {
        Tipado {
            tipo,
            nulabilidad: Nulabilidad::Nulable,
        }
    }
}

impl std::fmt::Display for Tipado {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.tipo, self.nulabilidad.sufijo())
    }
}

/// Por qué un tipo no es válido.
#[derive(Debug)]
pub enum TypeError {
    Desconocido,
    /// Un paramétrico al que le falta la unidad o la precisión.
    Incompleto(String),
    /// `Decimal<p, s>` con los números fuera de rango (OOS3002).
    DecimalFueraDeRango(String),
    /// v1alpha17. Un tipo compuesto mal formado (OOS3007), con el porqué.
    Compuesto(String),
    /// v1alpha32. Un tipo geo mal formado (OOS3008), con el porqué.
    Geo(String),
}

/// v1alpha17. La dimensión máxima de un `Vector<n>`: la de pgvector, que es la
/// más baja de los que lo guardan con tipo (`01-los-tipos` §2).
pub const DIMENSION_MAXIMA: u16 = 16000;

impl Type {
    /// v1alpha17. **Si el tipo es de v1alpha17**: lleva, a cualquier
    /// profundidad, un `Struct`, un `Vector` o un `Anchor`. En un documento de
    /// antes es un tipo que no existía (`OOS3001`), como `Media<…>` antes de
    /// v1alpha16.
    pub fn es_de_v1alpha17(&self) -> bool {
        match self {
            Type::Struct(_) | Type::Vector(_) | Type::Anchor => true,
            Type::List(t) => t.es_de_v1alpha17(),
            _ => false,
        }
    }

    /// v1alpha32. **Si el tipo es de v1alpha32**: lleva, a cualquier
    /// profundidad, una `Geometry` o una `Geography`. En un documento de antes
    /// es un tipo que no existía (`OOS3001`).
    pub fn es_de_v1alpha32(&self) -> bool {
        match self {
            Type::Geometry { .. } | Type::Geography { .. } => true,
            Type::List(t) => t.es_de_v1alpha32(),
            Type::Struct(campos) => campos.iter().any(|(_, t)| t.es_de_v1alpha32()),
            _ => false,
        }
    }

    /// v1alpha32. ¿Es una geometría (plana o sobre el elipsoide)?
    pub fn es_geo(&self) -> bool {
        matches!(self, Type::Geometry { .. } | Type::Geography { .. })
    }
}

/// v1alpha32. Los subtipos de `Geometry` y `Geography` (`01-la-geometria` §1).
pub const GEO_SUBTIPOS: &[&str] = &[
    "Any",
    "Point",
    "LineString",
    "Polygon",
    "MultiPoint",
    "MultiLineString",
    "MultiPolygon",
    "GeometryCollection",
];

/// v1alpha32. El subtipo y el CRS por defecto: cualquier geometría, en WGS 84
/// con los ejes longitud/latitud (el de GeoJSON, GeoParquet e Iceberg).
pub const GEO_SUBTIPO_DEFECTO: &str = "Any";
pub const CRS_DEFECTO: &str = "OGC:CRS84";

/// v1alpha32. Un subtipo con su dimensión, canónico (`Point Z`), o el porqué.
fn geo_subtipo(s: &str) -> Result<String, TypeError> {
    let mut partes = s.split_whitespace();
    let base = partes.next().unwrap_or("");
    let dim = partes.next();
    if partes.next().is_some() {
        return Err(TypeError::Geo(format!(
            "`{s}`: un subtipo es un nombre y, si acaso, `Z`, `M` o `ZM`"
        )));
    }
    if !GEO_SUBTIPOS.contains(&base) {
        return Err(TypeError::Geo(format!(
            "`{base}` no es un subtipo de geometría: {}",
            GEO_SUBTIPOS.join(" · ")
        )));
    }
    match dim {
        None => Ok(base.to_string()),
        // `Any` admite cualquier subtipo, con cualquier dimensión: no lleva la
        // suya (`01-la-geometria` §1).
        Some(d) if base == GEO_SUBTIPO_DEFECTO => Err(TypeError::Geo(format!(
            "`Any {d}`: `Any` admite cualquier subtipo y no lleva dimensiones"
        ))),
        Some(d @ ("Z" | "M" | "ZM")) => Ok(format!("{base} {d}")),
        Some(d) => Err(TypeError::Geo(format!(
            "`{d}` no es una dimensión: `Z`, `M` o `ZM`, como en WKT"
        ))),
    }
}

/// v1alpha32. Un CRS como código de autoridad, `AUTORIDAD:CÓDIGO`.
fn geo_crs(s: &str) -> Result<String, TypeError> {
    let valido = s.split_once(':').is_some_and(|(a, c)| {
        !a.is_empty()
            && !c.is_empty()
            && a.chars()
                .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
            && c.chars().all(|ch| ch.is_ascii_alphanumeric())
    });
    if valido {
        Ok(s.to_string())
    } else {
        Err(TypeError::Geo(format!(
            "`{s}` no es un CRS: se escribe `AUTORIDAD:CÓDIGO` (`EPSG:25830`, `OGC:CRS84`)"
        )))
    }
}

/// v1alpha32. Lo que sigue a `Geometry` o a `Geography` (§1, §2, §5).
fn geo(ctor: &str, resto: &str) -> Result<Type, TypeError> {
    let args = if resto.is_empty() {
        Vec::new()
    } else {
        let Some(dentro) = resto.strip_prefix('<').and_then(|r| r.strip_suffix('>')) else {
            return Err(TypeError::Geo(format!(
                "`{ctor}{resto}` no se cierra: `{ctor}<subtipo, crs>`"
            )));
        };
        if dentro.trim().is_empty() {
            return Err(TypeError::Geo(format!(
                "`{ctor}<>` no dice nada: sin argumentos se escribe `{ctor}`"
            )));
        }
        partir_por_comas(dentro)
    };
    if args.len() > 2 {
        return Err(TypeError::Geo(format!(
            "`{ctor}` lleva como mucho subtipo y CRS, y aquí hay {}",
            args.len()
        )));
    }
    let subtipo = match args.first() {
        Some(s) => geo_subtipo(s)?,
        None => GEO_SUBTIPO_DEFECTO.to_string(),
    };
    if ctor == "Geography" {
        if let Some(c) = args.get(1) {
            return Err(TypeError::Geo(format!(
                "una `Geography` está siempre en `OGC:CRS84` y no lleva CRS (`{c}`): para otro CRS, `Geometry<{subtipo}, {c}>`"
            )));
        }
        return Ok(Type::Geography { subtipo });
    }
    let crs = match args.get(1) {
        Some(c) => geo_crs(c)?,
        None => CRS_DEFECTO.to_string(),
    };
    Ok(Type::Geometry { subtipo, crs })
}

/// Parte `a, b<c, d>, e` por las comas de primer nivel.
fn partir_por_comas(s: &str) -> Vec<&str> {
    let (mut partes, mut prof, mut desde) = (Vec::new(), 0i32, 0usize);
    for (i, ch) in s.char_indices() {
        match ch {
            '<' => prof += 1,
            '>' => prof -= 1,
            ',' if prof == 0 => {
                partes.push(s[desde..i].trim());
                desde = i + 1;
            }
            _ => {}
        }
    }
    partes.push(s[desde..].trim());
    partes
}

/// Lo que va dentro de `list<…>` o de un campo de `Struct<…>` (`01-los-tipos`
/// §4): lo que no puede ir ahí es `OOS3007`, no `OOS3001`, porque el tipo
/// existe y lo que está mal es dónde se puso.
fn dentro(s: &str, donde: &str) -> Result<Type, TypeError> {
    let t = parse_type(s)?;
    let fuera =
        |que: &str, porque: &str| Err(TypeError::Compuesto(format!("{que} {donde}: {porque}")));
    match (&t, donde) {
        (Type::Media(_), _) => fuera(
            "una referencia `Media<…>`",
            "una referencia es una columna, y el ítem se referencia una vez por fila",
        ),
        (Type::Scalar(e), "en un campo de `Struct`") if e == "Opaque" => {
            fuera("`Opaque`", "lo que no se modela no se anida en lo que sí")
        }
        (Type::Anchor, "en un campo de `Struct`") => fuera(
            "un `Anchor`",
            "un ancla es una columna o un elemento de `list<Anchor>`",
        ),
        (Type::List(_), "en una lista") => fuera(
            "una lista",
            "una lista de listas se escribe como lista de structs",
        ),
        (Type::Vector(_), "en una lista") => fuera(
            "un `Vector`",
            "una lista de vectores se escribe como lista de structs con su vector",
        ),
        _ => Ok(t),
    }
}

fn estructura(args: &str) -> Result<Type, TypeError> {
    if args.trim().is_empty() {
        return Err(TypeError::Compuesto(
            "un `Struct` sin campos no tiene forma".into(),
        ));
    }
    let mut campos: Vec<(String, Type)> = Vec::new();
    for parte in partir_por_comas(args) {
        let Some((nombre, tipo)) = parte.split_once(':') else {
            return Err(TypeError::Compuesto(format!(
                "`{parte}` no es un campo: se escribe `nombre: Tipo`"
            )));
        };
        let nombre = nombre.trim();
        let valido = nombre
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
            && nombre
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valido {
            return Err(TypeError::Compuesto(format!(
                "`{nombre}` no es un nombre de campo (`[a-z_][a-z0-9_]*`)"
            )));
        }
        if campos.iter().any(|(n, _)| n == nombre) {
            return Err(TypeError::Compuesto(format!(
                "el campo `{nombre}` está dos veces"
            )));
        }
        campos.push((
            nombre.to_string(),
            dentro(tipo.trim(), "en un campo de `Struct`")?,
        ));
    }
    Ok(Type::Struct(campos))
}

pub fn parse_type(s: &str) -> Result<Type, TypeError> {
    let s = s.trim();
    // v1alpha17: `list<…>` admite, además de un escalar, lo compuesto
    // (`01-los-tipos` §2). Un escalar desconocido dentro sigue siendo un tipo
    // que no existe (`OOS3001`).
    if let Some(inner) = s.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
        let inner = inner.trim();
        if ESCALARES.contains(&inner) {
            return Ok(Type::List(Box::new(Type::Scalar(inner.to_string()))));
        }
        return match dentro(inner, "en una lista")? {
            // v1alpha32: una lista de geometrías (las posiciones de una
            // trayectoria, `01-la-geometria` §5).
            t @ (Type::Struct(_)
            | Type::Anchor
            | Type::Geometry { .. }
            | Type::Geography { .. }) => Ok(Type::List(Box::new(t))),
            _ => Err(TypeError::Desconocido),
        };
    }
    if let Some(args) = s.strip_prefix("Struct<").and_then(|r| r.strip_suffix('>')) {
        return estructura(args);
    }
    if let Some(n) = s.strip_prefix("Vector<").and_then(|r| r.strip_suffix('>')) {
        return match n.trim().parse::<u32>() {
            Ok(n) if (1..=u32::from(DIMENSION_MAXIMA)).contains(&n) => Ok(Type::Vector(n as u16)),
            Ok(n) => Err(TypeError::Compuesto(format!(
                "`Vector<{n}>`: la dimensión va de 1 a {DIMENSION_MAXIMA}"
            ))),
            Err(_) => Err(TypeError::Compuesto(
                "`Vector` lleva su dimensión, `Vector<768>`: es parte del contrato".into(),
            )),
        };
    }
    if s == "Anchor" {
        return Ok(Type::Anchor);
    }
    // v1alpha32: los tipos geo (`01-la-geometria`).
    for ctor in ["Geometry", "Geography"] {
        if let Some(resto) = s.strip_prefix(ctor)
            && (resto.is_empty() || resto.starts_with('<'))
        {
            return geo(ctor, resto.trim());
        }
    }

    if let Some(resto) = s.strip_prefix("Decimal<") {
        return decimal(resto);
    }
    // v1alpha16. Una colección y solo una: `Media` sin ella no dice a qué
    // apunta, y con dos no es una referencia. Las dos son `OOS3001` —un tipo
    // que no existe—, no una colección que no resuelve.
    if let Some(c) = s.strip_prefix("Media<").and_then(|r| r.strip_suffix('>')) {
        let c = c.trim();
        let valida = !c.is_empty()
            && c.split('.').all(|p| {
                !p.is_empty() && p.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            });
        return if valida {
            Ok(Type::Media(c.to_string()))
        } else {
            Err(TypeError::Desconocido)
        };
    }
    if let Some((ctor, resto)) = s.split_once('<') {
        if !matches!(ctor, "Money" | "Quantity") {
            return Err(TypeError::Desconocido);
        }
        let Some(args) = resto.strip_suffix('>') else {
            return Err(TypeError::Incompleto(ctor.to_string()));
        };
        let partes: Vec<&str> = args.split(',').map(str::trim).collect();
        if partes.len() != 2 || partes[0].is_empty() {
            return Err(TypeError::Incompleto(ctor.to_string()));
        }
        let Ok(precision) = partes[1].parse::<u32>() else {
            return Err(TypeError::Incompleto(ctor.to_string()));
        };
        return Ok(Type::Parametric {
            ctor: ctor.to_string(),
            unit: partes[0].to_string(),
            precision,
        });
    }

    if ESCALARES.contains(&s) {
        return Ok(Type::Scalar(s.to_string()));
    }
    // Un nombre cualificado es un tipo importado de un paquete de tipos.
    if s.contains('.') && s.split('.').all(|p| !p.is_empty()) {
        return Ok(Type::Imported(s.to_string()));
    }
    Err(TypeError::Desconocido)
}

/// Lo que sigue a `Decimal<`. Sin cerrar, o sin los dos números, está
/// incompleto; con los dos pero fuera de rango, también es `OOS3002`, con la
/// causa dicha.
fn decimal(resto: &str) -> Result<Type, TypeError> {
    let Some(args) = resto.strip_suffix('>') else {
        return Err(TypeError::Incompleto("Decimal".into()));
    };
    let partes: Vec<&str> = args.split(',').map(str::trim).collect();
    let [p, e] = partes[..] else {
        return Err(TypeError::Incompleto("Decimal".into()));
    };
    let (Ok(p), Ok(e)) = (p.parse::<u16>(), e.parse::<u16>()) else {
        return Err(TypeError::Incompleto("Decimal".into()));
    };
    if p == 0 || p > u16::from(PRECISION_MAXIMA) {
        return Err(TypeError::DecimalFueraDeRango(format!(
            "la precisión va de 1 a {PRECISION_MAXIMA}, el techo de Iceberg y Parquet \
             (`decimal128`), y es {p}. Lo que no cabe viaja como `String` con su `physicalType`"
        )));
    }
    if e > p {
        return Err(TypeError::DecimalFueraDeRango(format!(
            "la escala ({e}) no puede pasar de la precisión ({p}): son las cifras detrás de la \
             coma, y no hay más que las que hay"
        )));
    }
    Ok(Type::Decimal {
        precision: p as u8,
        escala: e as u8,
    })
}
