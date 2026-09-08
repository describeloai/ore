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
    /// El correo que el emisor afirma, si lo afirma.
    ///
    /// ⚠️ Es una COPIA de lo que dijo el emisor, no una verdad nuestra — igual
    /// que `iam.persona.correo`. Sirve para una cosa concreta: comprobar que
    /// un vale de invitación es para quien lo presenta. Una invitación a `a@x`
    /// redimida por `b@y` es un traspaso que nadie autorizó.
    pub correo: Option<String>,
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
            // El modo de banco no trae correo: es lo que hace que `admitir` no
            // se pueda ejercitar con el, y eso es correcto.
            correo: None,
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

/// Los dos modos, y la lista es donde se ve que son cosas distintas.
///
/// `cabecera` **afirma** quién pide; `oidc` lo **demuestra**. Que compartan un
/// puerto no los hace equivalentes, y por eso uno pide dos interruptores para
/// encenderse y el otro no pide ninguno.
pub const MODOS: &[&str] = &["cabecera", "oidc"];

/// Lo que hace falta para resolver un modo.
pub struct Ajustes<'a> {
    pub modo: Option<&'a str>,
    pub no_es_produccion: bool,
    /// El emisor esperado: `https://…/realms/<realm>`.
    pub emisor: Option<&'a str>,
    /// **Nuestra** audiencia. Un token del mismo realm para otro servicio no
    /// vale aquí, y sin esto no habría forma de decirlo.
    pub audiencia: Option<&'a str>,
    /// El fichero con el juego de llaves. **No una URL**: este proceso no va a
    /// buscarlas. Ver la cabecera de `oidc.rs`.
    pub jwks: Option<&'a std::path::Path>,
}

/// Resuelve el modo pedido. `None` **no es un error**: es el estado por defecto,
/// y quien lo reciba tiene que dejar las rutas de datos sin montar.
///
/// `cabecera` exige el segundo interruptor, y se niega sin él con el motivo
/// escrito: no es una comprobación de higiene, es la que impide que el modo de
/// prueba llegue a producción por omisión.
///
/// `oidc` exige las tres piezas —emisor, audiencia y llaves— y se niega si
/// falta una. **Ninguna tiene defecto**: un emisor por defecto sería confiar en
/// alguien que nadie eligió, y una audiencia por defecto sería aceptar
/// cualquier token del realm.
///
/// Devuelve el proveedor **y una línea que lo describe**. La línea la imprime
/// quien llama: una biblioteca que escribe en `stderr` decide por su
/// consumidor dónde va su salida, y eso no es suyo.
pub fn resolver(a: &Ajustes) -> Result<Option<(Proveedor, String)>, String> {
    match a.modo {
        None => Ok(None),
        Some("cabecera") if a.no_es_produccion => Ok(Some((
            por_cabecera(),
            "cabecera  ⚠️  MODO DE BANCO: el sujeto lo escribe quien llama".to_string(),
        ))),
        Some("cabecera") => Err(
            "`--identidad cabecera` es el modo de banco: el sujeto lo escribe quien llama.\n\
             Para usarlo hay que declararlo además con `--no-es-produccion`."
                .into(),
        ),
        Some("oidc") => {
            let emisor = a.emisor.ok_or("`--identidad oidc` necesita `--emisor`")?;
            let audiencia = a.audiencia.ok_or(
                "`--identidad oidc` necesita `--audiencia`: sin ella valdría cualquier token del realm",
            )?;
            let jwks = a
                .jwks
                .ok_or("`--identidad oidc` necesita `--jwks <fichero>`")?;
            let texto = std::fs::read_to_string(jwks)
                .map_err(|e| format!("no se pudo leer `{}`: {e}", jwks.display()))?;
            let llaves = crate::oidc::Llaves::leer(&texto)?;
            let dicho = format!(
                "oidc · emisor {emisor} · audiencia {audiencia} · {} llaves de `{}`",
                llaves.cuantas(),
                jwks.display()
            );
            let emisor = crate::oidc::Emisor {
                iss: emisor.to_string(),
                aud: audiencia.to_string(),
                llaves,
            };
            let proveedor: Proveedor = Box::new(move |cabeceras| {
                let cabecera = cabeceras
                    .get("authorization")
                    .ok_or(SinIdentidad::Ausente)?;
                emisor.verificar(cabecera, ahora())
            });
            Ok(Some((proveedor, dicho)))
        }
        Some(otro) => Err(format!(
            "modo de identidad `{otro}` desconocido; los que hay: {}",
            MODOS.join(", ")
        )),
    }
}

/// El instante, en segundos desde la época.
///
/// Aquí SÍ se lee el reloj, y conviene decir por qué eso no contradice la
/// invariante del compilador: `ore-core` es puro porque compilar el mismo
/// documento dos veces tiene que dar lo mismo. Comprobar si un token caducó es
/// justo lo contrario — la respuesta correcta **depende de cuándo se pregunta**.
fn ahora() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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

    fn ajustes(modo: Option<&'static str>, no_es_produccion: bool) -> Ajustes<'static> {
        Ajustes {
            modo,
            no_es_produccion,
            emisor: None,
            audiencia: None,
            jwks: None,
        }
    }

    #[test]
    fn sin_modo_no_hay_proveedor() {
        assert!(resolver(&ajustes(None, true)).unwrap().is_none());
        assert!(resolver(&ajustes(None, false)).unwrap().is_none());
    }

    /// El segundo interruptor no es decorativo.
    #[test]
    fn el_modo_de_banco_exige_los_dos_interruptores() {
        assert!(resolver(&ajustes(Some("cabecera"), false)).is_err());
        assert!(
            resolver(&ajustes(Some("cabecera"), true))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn un_modo_que_no_existe_se_niega() {
        assert!(resolver(&ajustes(Some("inventado"), true)).is_err());
    }

    /// Las tres piezas de `oidc` no tienen defecto, y cada una falta por su
    /// cuenta: un emisor por defecto sería confiar en quien nadie eligió, y una
    /// audiencia por defecto sería aceptar cualquier token del realm.
    #[test]
    fn oidc_exige_sus_tres_piezas() {
        let sin_nada = Ajustes {
            modo: Some("oidc"),
            no_es_produccion: false,
            emisor: None,
            audiencia: None,
            jwks: None,
        };
        assert!(resolver(&sin_nada).is_err());

        let solo_emisor = Ajustes {
            emisor: Some("https://x/realms/y"),
            ..sin_nada
        };
        assert!(resolver(&solo_emisor).is_err());

        let sin_llaves = Ajustes {
            audiencia: Some("ore-serve"),
            ..solo_emisor
        };
        assert!(
            resolver(&sin_llaves).is_err(),
            "sin `--jwks` no puede haber proveedor"
        );
    }

    /// Y `oidc` NO pide `--no-es-produccion`: es el modo que se sostiene solo.
    #[test]
    fn oidc_no_es_el_modo_de_banco() {
        let d = std::env::temp_dir().join(format!("ore-serve-jwks-{}.json", std::process::id()));
        std::fs::write(
            &d,
            // Una llave de verdad —la misma de las pruebas de `oidc`—, porque
            // `Llaves::leer` se niega si el módulo no es utilizable, y con
            // razón: un juego de llaves que no sirve no debe dejar arrancar.
            concat!(
                r#"{"keys":[{"kty":"RSA","use":"sig","kid":"k1","e":"AQAB","n":""#,
                "i7YpoTYP_5CNa4i2r4ESwFtZiXv3tRa8PLqNX7M-cdxWE5dWNA9HHsWtq2_V6NbbfnDLj8jeVT2CBssrH-Fr4yr2Huc9ang_ZdPMpdOa7QKDDRWOzUGD8dAaoMvcJwogBt-heJmBTJ2eFcg-BbStNgzgxasI9uyvXnvwqpf3WJc",
                r#""}]}"#
            ),
        )
        .unwrap();
        let r = resolver(&Ajustes {
            modo: Some("oidc"),
            no_es_produccion: false,
            emisor: Some("https://login.paladio.io/realms/rubix"),
            audiencia: Some("ore-serve"),
            jwks: Some(&d),
        });
        let _ = std::fs::remove_file(&d);
        assert!(r.unwrap().is_some());
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
