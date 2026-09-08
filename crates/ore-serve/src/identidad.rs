//! De dónde sale el sujeto — **un puerto sin defecto**.
//!
//! # El agujero que este fichero existe para no abrir
//!
//! Un servidor que se cree la identidad que le llega en una cabecera no tiene
//! puerta: tiene un formulario donde el cliente escribe quién es. Y no se abre
//! por descuido, se abre por comodidad — se cablea «para probar» y nadie lo
//! quita.
//!
//! La forma es prestada, y se dice de dónde: `auth/src/identidad.ts` de la
//! plataforma resolvió esto mismo antes y su conclusión es la que se copia.
//!
//! # La regla
//!
//! > **Sin proveedor configurado, las rutas de datos NO SE MONTAN.**
//!
//! El defecto no es «cualquiera»; es «no hay identidad» ⇒ no hay superficie. Es
//! la misma figura que ORE ya usa con `ConduitPolicy`, donde *omitir no deja
//! nada abierto: lo CIERRA*. Un defecto permisivo aquí es una fuga esperando al
//! día en que alguien despliegue sin poner la variable.
//!
//! # Y el modo de prueba pide DOS interruptores
//!
//! [`por_cabecera`] existe para poder ejercitar el camino entero sin haber
//! montado OAuth. Para encenderlo hay que pedirlo **por su nombre** y además
//! **declarar que esto no es producción**. Dos, no uno: un solo interruptor se
//! pulsa sin querer, y el modo que se enciende sin querer es el que se queda
//! encendido.
//!
//! # Lo que este módulo NO hace
//!
//! **No autoriza.** Dice quién pregunta y nada más. Quién puede qué se decide
//! en otro sitio, y mezclarlo aquí es exactamente cómo la frontera entre
//! identidad y permiso se dibuja torcida.

use std::collections::BTreeMap;

/// Lo que una petición trae de identidad.
///
/// `agente` ausente ⇒ sin delegación. La forma es la de RFC 8693 —`sub` más
/// `act`— porque el día que un Job de ORE actúe por una persona, la frase «esto
/// lo hizo Fulano a través de este trabajo» tiene que caber sin rehacer el tipo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identidad {
    pub persona: String,
    pub agente: Option<String>,
}

/// Por qué no hay sujeto.
///
/// **`Ausente` y `Invalida` no son lo mismo y ningún modo puede colapsarlos.**
/// `Ausente` es *«no traes identidad»*; `Invalida` es *«traes una y está mal»*,
/// y tiene que decir por qué — si no, un despliegue con la audiencia mal puesta
/// se lee como «nadie manda credencial» y nadie mira el sitio correcto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinIdentidad {
    Ausente,
    Invalida(String),
}

/// El puerto: de las cabeceras a una identidad.
pub type Proveedor =
    Box<dyn Fn(&BTreeMap<String, String>) -> Result<Identidad, SinIdentidad> + Send + Sync>;

/// La cabecera del modo de prueba. Se llama así, con `x-`, para que se vea en
/// cualquier traza que esto **no** es un token.
pub const CABECERA_SUJETO: &str = "x-ore-sujeto";
pub const CABECERA_AGENTE: &str = "x-ore-agente";

/// ⚠️ **El modo de banco** — el sujeto por cabecera.
///
/// Valida igual de duro que uno de verdad: el sujeto tiene que estar, no puede
/// venir vacío y no puede traer espacios ni saltos de línea —una identidad con
/// un salto dentro es lo que convierte un registro de acceso en dos—. Lo que
/// **no** comprueba, y es todo lo que importa, es que quien lo manda sea quien
/// dice.
pub fn por_cabecera() -> Proveedor {
    Box::new(|cabeceras| {
        let persona = match cabeceras.get(CABECERA_SUJETO) {
            None => return Err(SinIdentidad::Ausente),
            Some(v) => v.trim(),
        };
        if persona.is_empty() {
            return Err(SinIdentidad::Ausente);
        }
        comprobar(persona)?;
        let agente = match cabeceras.get(CABECERA_AGENTE) {
            None => None,
            Some(v) => {
                let v = v.trim();
                if v.is_empty() {
                    None
                } else {
                    comprobar(v)?;
                    Some(v.to_string())
                }
            }
        };
        Ok(Identidad {
            persona: persona.to_string(),
            agente,
        })
    })
}

fn comprobar(v: &str) -> Result<(), SinIdentidad> {
    if v.len() > 256 {
        return Err(SinIdentidad::Invalida(
            "el sujeto es demasiado largo".into(),
        ));
    }
    if v.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(SinIdentidad::Invalida(
            "el sujeto no puede llevar espacios ni caracteres de control".into(),
        ));
    }
    Ok(())
}

/// Los modos que se pueden pedir por su nombre. Hoy hay uno, y el día que haya
/// un `oidc` esta lista es donde se ve que son dos cosas distintas.
pub const MODOS: &[&str] = &["cabecera"];

/// Resuelve el modo pedido. `None` **no es un error**: es el estado por defecto,
/// y quien lo reciba tiene que dejar las rutas de datos sin montar.
///
/// `cabecera` exige el segundo interruptor, y se niega sin él con el motivo
/// escrito: no es una comprobación de higiene, es la que impide que el modo de
/// prueba llegue a producción por omisión.
pub fn resolver(modo: Option<&str>, no_es_produccion: bool) -> Result<Option<Proveedor>, String> {
    match modo {
        None => Ok(None),
        Some("cabecera") if no_es_produccion => Ok(Some(por_cabecera())),
        Some("cabecera") => Err(
            "`--identidad cabecera` es el modo de banco: el sujeto lo escribe quien llama.\n\
             Para usarlo hay que declararlo además con `--no-es-produccion`."
                .into(),
        ),
        Some(otro) => Err(format!(
            "modo de identidad `{otro}` desconocido; los que hay: {}",
            MODOS.join(", ")
        )),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn cabeceras(pares: &[(&str, &str)]) -> BTreeMap<String, String> {
        pares
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn sin_modo_no_hay_proveedor() {
        assert!(resolver(None, true).unwrap().is_none());
        assert!(resolver(None, false).unwrap().is_none());
    }

    /// El segundo interruptor no es decorativo.
    #[test]
    fn el_modo_de_banco_exige_los_dos_interruptores() {
        assert!(resolver(Some("cabecera"), false).is_err());
        assert!(resolver(Some("cabecera"), true).unwrap().is_some());
    }

    #[test]
    fn un_modo_que_no_existe_se_niega() {
        assert!(resolver(Some("oidc"), true).is_err());
    }

    #[test]
    fn ausente_y_invalida_no_son_lo_mismo() {
        let p = por_cabecera();
        assert_eq!(p(&cabeceras(&[])), Err(SinIdentidad::Ausente));
        assert_eq!(
            p(&cabeceras(&[(CABECERA_SUJETO, "  ")])),
            Err(SinIdentidad::Ausente)
        );
        assert!(matches!(
            p(&cabeceras(&[(CABECERA_SUJETO, "con espacio")])),
            Err(SinIdentidad::Invalida(_))
        ));
    }

    #[test]
    fn la_delegacion_es_opcional_y_se_valida_igual() {
        let p = por_cabecera();
        let sola = p(&cabeceras(&[(CABECERA_SUJETO, "persona:ana")])).unwrap();
        assert_eq!(sola.agente, None);

        let con = p(&cabeceras(&[
            (CABECERA_SUJETO, "persona:ana"),
            (CABECERA_AGENTE, "agente:job-7"),
        ]))
        .unwrap();
        assert_eq!(con.agente.as_deref(), Some("agente:job-7"));

        assert!(matches!(
            p(&cabeceras(&[
                (CABECERA_SUJETO, "persona:ana"),
                (CABECERA_AGENTE, "mal agente"),
            ])),
            Err(SinIdentidad::Invalida(_))
        ));
    }
}
