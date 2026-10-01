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

/// Los tipos de una firma (OOS v1alpha18 01 §4.6).
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
    /// `<ruta desde la carpeta del paquete>:<def>`.
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
