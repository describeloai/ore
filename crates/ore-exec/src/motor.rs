//! Cargar un paquete y dejarlo listo para autorizar.
//!
//! Tres pasos, y los tres pueden fallar por motivos distintos que hay que
//! distinguir — un rechazo que no dice cuál de los tres fue no sirve para
//! arreglarlo.

use cedar_policy::{PolicySet, Schema, ValidationMode, Validator};
use ore_core::link::Package;
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

pub struct Motor {
    pub esquema: Schema,
    pub politicas: PolicySet,
    pub paquete: Package,
    /// Las políticas leídas estructuralmente, indexadas por **nuestro** `@id`.
    ///
    /// Hace falta porque Cedar nombra las políticas por POSICIÓN —`policy0`,
    /// `policy1`…— que es exactamente la identidad que el ADR 0003 rechazó por
    /// escrito: *«sin `@id`, mover una política de línea parecería borrarla y
    /// crear otra»*. Un veredicto que dijera `policy2` sería inauditable en
    /// cuanto alguien reordenase el fichero.
    pub leidas: BTreeMap<String, ore_core::cedar::Policy>,
    /// Qué propiedades alcanza cada política. Es lo que permite distinguir *«no
    /// hay ninguna política»* de *«hay tres y ninguna casó»*, que Cedar devuelve
    /// como el mismo `Deny`.
    pub alcance: BTreeMap<String, Vec<String>>,
    /// El `@id` que corresponde a un identificador de Cedar.
    nombres: BTreeMap<String, String>,
}

impl Motor {
    /// Traduce un identificador de Cedar al `@id` del documento.
    pub(crate) fn nombre_de(&self, cedar: &str) -> String {
        self.nombres.get(cedar).cloned().unwrap_or_else(|| cedar.to_string())
    }

    /// Propiedad → sus etiquetas efectivas, en la forma `<retículo>:<nivel>`.
    pub(crate) fn etiquetas_por_propiedad(&self) -> BTreeMap<String, Vec<String>> {
        let lat = ore_core::flow::lattices(&self.paquete);
        ore_core::flow::efectivas(&self.paquete, &lat)
            .into_iter()
            .map(|(prop, etiquetas)| {
                (
                    prop,
                    etiquetas
                        .iter()
                        .map(|(ret, nivel)| format!("{ret}:{nivel}"))
                        .collect(),
                )
            })
            .collect()
    }
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

        let nombres: BTreeMap<String, String> = politicas
            .policies()
            .map(|p| {
                let cedar = p.id().to_string();
                let nuestro = p
                    .annotation("id")
                    .map(str::to_string)
                    .unwrap_or_else(|| cedar.clone());
                (cedar, nuestro)
            })
            .collect();
        let leidas: BTreeMap<String, ore_core::cedar::Policy> = paquete
            .cedar
            .iter()
            .flat_map(|(_, t)| ore_core::cedar::read(t))
            .map(|p| (p.id.clone(), p))
            .collect();
        let alcance = ore_core::politica::alcance(&paquete);

        Ok(Motor {
            esquema,
            politicas,
            paquete,
            leidas,
            alcance,
            nombres,
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
