//! Cargar un paquete y dejarlo listo para autorizar.
//!
//! Tres pasos, y los tres pueden fallar por motivos distintos que hay que
//! distinguir — un rechazo que no dice cuál de los tres fue no sirve para
//! arreglarlo.

use cedar_policy::{PolicySet, Schema, ValidationMode, Validator};
use ore_core::link::Package;
use std::path::Path;
use std::str::FromStr;

pub struct Motor {
    pub esquema: Schema,
    pub politicas: PolicySet,
    pub paquete: Package,
}

/// Por qué no se pudo cargar. Los tres son fallos **del artefacto**, no de la
/// petición: ocurren antes de que exista ninguna.
#[derive(Debug)]
pub enum Carga {
    /// El paquete no valida. Un ejecutor no autoriza contra un documento que el
    /// compilador rechaza: sería servir una política cuyo significado no está
    /// fijado.
    NoValida(Vec<String>),
    /// Nuestra propia proyección a esquema Cedar no la acepta Cedar. Es un
    /// defecto de `ore_core::cedar_schema`, no del paquete.
    EsquemaRechazado(String),
    /// Las políticas del paquete no se analizan.
    PoliticasIlegibles(String),
}

impl Motor {
    pub fn cargar(raiz: &Path) -> Result<Motor, Carga> {
        let diags = ore_core::validate_package(raiz);
        if !diags.is_empty() {
            return Err(Carga::NoValida(
                diags.iter().map(|d| d.render(raiz)).collect(),
            ));
        }
        let paquete = ore_core::validate::cargar_paquete(raiz).0;

        // EL MISMO esquema que emite `ore export`, no uno equivalente.
        let json = ore_core::cedar_schema::emit(&paquete).jcs();
        let esquema =
            Schema::from_json_str(&json).map_err(|e| Carga::EsquemaRechazado(e.to_string()))?;

        // Un solo conjunto con todos los ficheros: Cedar decide sobre el
        // conjunto, y `forbid` gana desde cualquiera de ellos.
        let texto: String = paquete
            .cedar
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let politicas =
            PolicySet::from_str(&texto).map_err(|e| Carga::PoliticasIlegibles(e.to_string()))?;

        Ok(Motor {
            esquema,
            politicas,
            paquete,
        })
    }

    /// La prueba de fuego: **las políticas del paquete contra el esquema que el
    /// propio paquete proyecta.**
    ///
    /// Nadie las había enfrentado nunca. `sync.rs` comprueba que el esquema
    /// comprometido conozca cada nivel de cada retículo, y `politica.rs` que
    /// cada etiqueta mencionada exista — las dos direcciones de *una* de las
    /// proyecciones. Que una política **entera** sea válida contra el esquema
    /// entero es otra pregunta, y solo la contesta un validador de Cedar.
    pub fn validar(&self) -> Vec<String> {
        let r = Validator::new(self.esquema.clone()).validate(&self.politicas, ValidationMode::Strict);
        r.validation_errors().map(|e| e.to_string()).collect()
    }

    /// Lo que el validador no considera un error pero conviene ver: una
    /// condición imposible es una política que no gobierna, y ya sabemos que
    /// tiene el mismo aspecto que una que sí.
    pub fn avisos(&self) -> Vec<String> {
        let r = Validator::new(self.esquema.clone()).validate(&self.politicas, ValidationMode::Strict);
        r.validation_warnings().map(|w| w.to_string()).collect()
    }
}
