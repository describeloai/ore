//! **La puerta de los punteros** (0079 §19.1, E1a·5b): lo único por donde se
//! lee o se escribe el puntero de un dataset. Quien llama no sabe dónde vive
//! —un directorio del árbol hoy ([`super::ficheros::Ficheros`]), el catálogo
//! transaccional mañana—, y por eso el corte (E1a·5) cambia la implementación
//! y no a quien llama.
//!
//! Cada puerta está abierta sobre **una ref**: la de `main` o la de una rama,
//! con lo que la rama no tocó leído de `main` al día ([`super::manda`]).
//!
//! | | Qué |
//! |---|---|
//! | [`Punteros::uno`] | el puntero efectivo de un nombre, y de dónde sale |
//! | [`Punteros::todos`] | todos los de la ref |
//! | [`Punteros::historia`] | sus versiones, de la más nueva a la más vieja |
//! | [`Punteros::escribir`] | uno o varios cambios, **todos o ninguno**, con su *compare-and-set* |
//! | [`Punteros::reclamados`] | lo que nombra cualquier ref viva: lo que la recogida no borra |
//! | [`Punteros::version`] | cambia si y solo si algo de la ref cambió: la clave de las cachés |
//!
//! Lo que es del dominio —qué campos pone una copia, qué se conserva de un
//! fallo— lo decide quien llama; la puerta guarda lo que le dan y mantiene lo
//! que vale para cualquier puntero: su `dataset` sale de su `metadata_location`
//! (lo que reclama, 0038 P2), y el de antes se retira al escribir el de su sitio.

use std::collections::{BTreeMap, BTreeSet};

use crate::json::Json;
use crate::parse::Node;

/// Un puntero leído.
#[derive(Debug, Clone)]
pub struct Leido {
    pub puntero: Node,
    /// El texto tal como está guardado: para comparar dos lecturas de la misma
    /// implementación (lo que cambió entre dos lados), no entre dos distintas.
    pub texto: String,
    pub de: De,
}

/// De dónde sale lo leído (0044 C.2 ③).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum De {
    /// De la ref sobre la que está abierta la puerta.
    Aqui,
    /// De `main` al día: la rama no lo tocó.
    Main,
}

/// Una versión de un puntero.
#[derive(Debug, Clone)]
pub struct Version {
    /// Lo que la identifica en su almacén: un commit de git, un commit del catálogo.
    pub id: String,
    /// Cuándo se guardó (RFC 3339).
    pub cuando: String,
    pub quien: Option<String>,
    pub puntero: Node,
    pub texto: String,
}

/// Un cambio de [`Punteros::escribir`].
#[derive(Debug, Clone)]
pub struct Cambio {
    /// La forma corta: `base.n` o `base.schema.n`.
    pub nombre: String,
    pub que: Que,
    pub esperado: Esperado,
}

#[derive(Debug, Clone)]
pub enum Que {
    /// Sobre lo que había: quita `quitar` y pone `campos` encima.
    Poner {
        campos: Vec<(String, Json)>,
        quitar: Vec<String>,
    },
    /// El puntero entero (un objeto JSON).
    Sustituir(Json),
    /// Ya no está: el de su sitio y el de antes.
    Retirar,
}

/// El *compare-and-set*: lo que quien escribe cree que hay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Esperado {
    /// No se comprueba.
    Cualquiera,
    /// Que no haya puntero (un dataset que nace).
    Nada,
    /// Que el de hoy apunte a esta `metadata_location`.
    Ml(String),
}

/// Quién escribe y por qué: lo que la historia guarda de cada cambio.
#[derive(Debug, Clone, Default)]
pub struct Contexto {
    pub autor: String,
    /// `copia`, `commit`, `recoger`, `retirar`…
    pub operacion: String,
    /// La misma clave dos veces es una escritura (ADR 0031).
    pub idempotencia: Option<String>,
}

/// Lo que reclaman las refs vivas: lo que la recogida no puede borrar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reclamados {
    /// Los nombres en el lago (`catalogo/b/s/n`, `datasets/p_n`, `colecciones/…`).
    pub datasets: BTreeSet<String>,
    /// Las claves de las copias de una consulta (`clave`).
    pub claves: BTreeSet<String>,
    /// Por dataset, las `metadata_location` que se nombran.
    pub ubicaciones: BTreeMap<String, BTreeSet<String>>,
    /// Las colecciones, `(dataset, metadata_location)`: sus blobs cuentan vivos.
    pub colecciones: BTreeSet<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallo {
    /// El *compare-and-set* no se cumplió: nada se escribió.
    Conflicto {
        nombre: String,
        esperado: Esperado,
        hay: Option<String>,
    },
    /// No es una forma corta (`base.n` o `base.schema.n`).
    Nombre(String),
    /// La ref no admite escribir (`main` protegida).
    Protegida(String),
    /// El almacén no contestó o no se pudo leer o escribir.
    Almacen(String),
}

impl Fallo {
    /// El código HTTP que le corresponde.
    pub fn codigo(&self) -> u16 {
        match self {
            Fallo::Conflicto { .. } => 409,
            Fallo::Nombre(_) => 422,
            Fallo::Protegida(_) => 423,
            Fallo::Almacen(_) => 503,
        }
    }
}

impl std::fmt::Display for Fallo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fallo::Conflicto {
                nombre,
                esperado,
                hay,
            } => {
                let quiso = match esperado {
                    Esperado::Cualquiera => "cualquiera".to_string(),
                    Esperado::Nada => "ninguno".to_string(),
                    Esperado::Ml(m) => format!("`{m}`"),
                };
                let hay = hay
                    .as_deref()
                    .map_or("ninguno".to_string(), |h| format!("`{h}`"));
                write!(
                    f,
                    "el puntero de `{nombre}` cambió: se esperaba {quiso} y hay {hay}"
                )
            }
            Fallo::Nombre(n) => write!(f, "`{n}` no es `base.nombre` ni `base.schema.nombre`"),
            Fallo::Protegida(r) => write!(f, "`{r}` está protegida: se escribe por una propuesta"),
            Fallo::Almacen(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Fallo {}

/// **La puerta.** Ver el módulo.
pub trait Punteros {
    /// El puntero efectivo de `nombre` en la ref, si lo hay.
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo>;

    /// Todos los de la ref, por su forma corta.
    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo>;

    /// Las versiones de `nombre`, de la más nueva a la más vieja, hasta `hasta`.
    /// Lo heredado de `main` tiene la historia de `main`; un puntero de antes
    /// (`datasets/<p>_<n>.json`), la suya.
    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo>;

    /// **Todos o ninguno**: si un `esperado` no se cumple, [`Fallo::Conflicto`]
    /// y nada cambia. Lo que se escribe sobre algo heredado de `main` pasa a
    /// ser de la ref.
    fn escribir(&self, cambios: &[Cambio], ctx: &Contexto) -> Result<(), Fallo>;

    /// Lo que nombra cualquier ref viva, esta incluida.
    fn reclamados(&self) -> Result<Reclamados, Fallo>;

    /// Cambia si y solo si algo de lo que la puerta ve cambió.
    fn version(&self) -> Result<String, Fallo>;
}

/// **El ciclo de vida de las ramas**, para quien lo gobierna (las propuestas).
/// En git lo hace git: ahí no hace nada. En el catálogo, es su tabla de refs.
pub trait Ramas {
    /// Nace `rama` desde `desde` (su punto de salida).
    fn crear(&self, rama: &str, desde: &str) -> Result<(), Fallo>;
    /// `rama` deja de existir: suelta lo que reclamaba.
    fn borrar(&self, rama: &str) -> Result<(), Fallo>;
    /// `rama` trae `main`: su punto de salida pasa a `main` de hoy.
    fn traer(&self, rama: &str) -> Result<(), Fallo>;
}
