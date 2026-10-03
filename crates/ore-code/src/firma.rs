//! Lo que se deriva de un fichero de código, sin el lenguaje en que se
//! escribió.

use std::fmt;

/// Un trozo del fuente, en bytes: `[inicio, fin)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Rango {
    pub inicio: u32,
    pub fin: u32,
}

/// Algo del fuente que impide derivar, o que no es lo que parece: dónde,
/// qué pasa y, cuando se sabe, cómo se arregla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallo {
    pub rango: Rango,
    pub mensaje: String,
    pub ayuda: Option<String>,
}

impl Fallo {
    pub fn new(rango: Rango, mensaje: impl Into<String>) -> Self {
        Fallo {
            rango,
            mensaje: mensaje.into(),
            ayuda: None,
        }
    }
    pub fn ayuda(mut self, a: impl Into<String>) -> Self {
        self.ayuda = Some(a.into());
        self
    }
}

/// Los tipos de una firma (OOS v1alpha18 01 §4.6, y v1alpha20 `01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tipo {
    Integer,
    Float,
    String,
    Boolean,
    Date,
    DateTime,
    Decimal,
    /// `list<T>`, sin listas dentro.
    Lista(Box<Tipo>),
    // ── v1alpha20 · la firma habla OOS ──────────────────────────────────
    /// `datetime.time`.
    Time,
    /// `ore.tipos.DateTimeTz`: un instante, con su zona.
    DateTimeTz,
    /// `bytes`: lo que OOS no modela.
    Opaque,
    /// `Annotated[Decimal, Precision(p, s)]`.
    DecimalPs {
        precision: u8,
        escala: u8,
    },
    /// `Money["EUR", 2]` y `Quantity["km", 1]`: la unidad es parte del tipo.
    Unidad {
        ctor: &'static str,
        unidad: String,
        precision: u32,
    },
    /// Una `@dataclass` del fichero que no es la vuelta: sus campos, en orden.
    Struct(Vec<(String, Tipo)>),
    /// `ore.tipos.Media["base.schema.coleccion"]`.
    Media(String),
}

impl Tipo {
    /// Si es de v1alpha20, a cualquier profundidad: lo que la tabla de
    /// v1alpha18 no derivaba.
    pub fn de_v1alpha20(&self) -> bool {
        match self {
            Tipo::Integer
            | Tipo::Float
            | Tipo::String
            | Tipo::Boolean
            | Tipo::Date
            | Tipo::DateTime
            | Tipo::Decimal => false,
            Tipo::Lista(t) => t.de_v1alpha20(),
            _ => true,
        }
    }
}

impl fmt::Display for Tipo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tipo::Integer => f.write_str("Integer"),
            Tipo::Float => f.write_str("Float"),
            Tipo::String => f.write_str("String"),
            Tipo::Boolean => f.write_str("Boolean"),
            Tipo::Date => f.write_str("Date"),
            Tipo::DateTime => f.write_str("DateTime"),
            Tipo::Decimal => f.write_str("Decimal"),
            Tipo::Lista(t) => write!(f, "list<{t}>"),
            // Como los escribe `ore_core::types` (`Display` de `Type`): la
            // forma canónica, un espacio tras cada `:` y cada `,`.
            Tipo::Time => f.write_str("Time"),
            Tipo::DateTimeTz => f.write_str("DateTimeTz"),
            Tipo::Opaque => f.write_str("Opaque"),
            Tipo::DecimalPs { precision, escala } => write!(f, "Decimal<{precision}, {escala}>"),
            Tipo::Unidad {
                ctor,
                unidad,
                precision,
            } => write!(f, "{ctor}<{unidad}, {precision}>"),
            Tipo::Struct(campos) => {
                f.write_str("Struct<")?;
                for (i, (n, t)) in campos.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{n}: {t}")?;
                }
                f.write_str(">")
            }
            Tipo::Media(c) => write!(f, "Media<{c}>"),
        }
    }
}

/// Un parámetro de `input`, o un campo de `output`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Campo {
    pub nombre: String,
    pub tipo: Tipo,
    /// Sin valor por defecto y sin ser opcional. En el documento, `required:
    /// true`; lo demás no lo escribe (`required` ausente es `false`).
    pub requerido: bool,
}

/// Lo que devuelve: un valor sin nombre (`output: {type: T}`) o un registro
/// (`output` como mapa de campos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Salida {
    Valor(Tipo),
    Campos(Vec<Campo>),
}

/// El contrato de una función, tal como el código lo da.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Firma {
    pub nombre: String,
    /// `<ruta desde la carpeta del paquete>:<def>` (Python) o `<ruta>.ts`
    /// (TypeScript, v1alpha23).
    pub entrypoint: String,
    /// La primera línea no vacía de la docstring.
    pub descripcion: Option<String>,
    pub over: Option<String>,
    pub reads: Option<Vec<String>>,
    /// Cada una como `modelo/<referencia>`.
    pub models: Option<Vec<String>>,
    pub timeout: Option<String>,
    pub entrada: Vec<Campo>,
    pub salida: Salida,
}

impl Firma {
    /// La versión más baja cuya tabla deriva esta firma (v1alpha20 `01` §6):
    /// v1alpha18, salvo que use algo de v1alpha20. Un árbol que no usa nada
    /// nuevo no cambia ni un byte.
    pub fn api_version(&self) -> &'static str {
        // Una función de TypeScript nace en v1alpha23: antes no había `node`.
        if self.runtime() == "node" {
            return "oos.dev/v1alpha23";
        }
        let nuevo = self.entrada.iter().any(|c| c.tipo.de_v1alpha20())
            || match &self.salida {
                Salida::Valor(t) => t.de_v1alpha20(),
                Salida::Campos(cs) => cs.iter().any(|c| c.tipo.de_v1alpha20()),
            };
        if nuevo {
            "oos.dev/v1alpha20"
        } else {
            "oos.dev/v1alpha18"
        }
    }
}

impl Firma {
    /// `python` o `node`: lo dice el `entrypoint` —`<ruta>.py:<def>` o
    /// `<ruta>.ts` (OOS v1alpha23 `01` §2)—, que es de donde se derivó.
    pub fn runtime(&self) -> &'static str {
        if self.entrypoint.ends_with(".ts") {
            "node"
        } else {
            "python"
        }
    }
}

/// Un `@function` del nivel superior: su firma, o todo lo que impide sacarla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Funcion {
    pub nombre: String,
    /// El nombre del `def`, para señalarlo.
    pub rango: Rango,
    pub resultado: Result<Firma, Vec<Fallo>>,
}

/// Un `def` del nivel superior, lleve `@function` o no: lo que un `entrypoint`
/// puede nombrar (`OOS2042` si no está, o es `async`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Def {
    pub nombre: String,
    pub rango: Rango,
    pub asincrona: bool,
    pub decorada: bool,
}

/// Todo lo que un fichero de código dice de sí mismo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Derivacion {
    /// Lo que no es Python. Con un error de sintaxis el analizador se recupera
    /// y sigue, así que lo demás puede venir lleno; quien lo use decide.
    pub sintaxis: Vec<Fallo>,
    /// Python válido que la versión del puesto no entiende (p. ej. una
    /// t-string de 3.14 con el puesto en 3.12).
    pub version: Vec<Fallo>,
    pub defs: Vec<Def>,
    pub funciones: Vec<Funcion>,
    /// Un `@function` que no es función: dentro de una clase o de otro `def`.
    pub avisos: Vec<Fallo>,
}
