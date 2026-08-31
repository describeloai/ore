//! La fase ① del plan: **autorizar poda**.
//!
//! Contesta una pregunta y solo una — *¿puede **este** principal hacer **esta**
//! acción sobre **esta propiedad**?*— y la contesta **sin abrir nada**: el
//! almacén de entidades se construye del bundle, porque una propiedad es un
//! `Property` cuyos padres son sus etiquetas efectivas, y el principal llega con
//! la petición. Es lo que `05-ejecutor` §6.1 afirmaba y aquí está medido.
//!
//! # El recurso es una propiedad, no una fila
//!
//! Cedar gobierna **propiedades**. *«Qué filas»* es el otro eje y no se
//! autoriza: se **filtra**, con un ámbito que viaja al origen
//! (`v1alpha3/02-ruleset` §4.2). Por eso `autorizar` no toma una clave: no la
//! necesita, y pedirla sería insinuar una decisión por fila que este motor no
//! toma.
//!
//! # Las dos denegaciones que Cedar no distingue
//!
//! Un `forbid` dice quién fue; una denegación por ausencia de permiso no dice
//! nada, y **las dos son el mismo `Deny`**. Está medido en `tests/terreno.rs`.
//!
//! Dejarlo así llevaría *«el error es el producto»* hasta la puerta de L2 y no
//! más allá, así que se distinguen tres:
//!
//! | | Qué pasó |
//! |---|---|
//! | `Prohibida` | un `forbid` la alcanzó, **y se nombra** |
//! | `SinPolitica` | **ninguna** política alcanza esa propiedad |
//! | `NingunaCaso` | hay políticas que la alcanzan y **ninguna casó** |
//!
//! La tercera es la que le sirve a quien escribe políticas: le dice contra qué
//! comparar. Y la segunda no es un fallo — es **P4 funcionando**.

use crate::motor::Motor;
use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityUid, Request, RestrictedExpression,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

/// Una petición, ya recibida y con sus reclamaciones desempaquetadas.
///
/// Los valores llegan **de fuera**: nada de aquí se resuelve contra un binding.
#[derive(Debug, Clone)]
pub struct Peticion {
    /// Quién dice ser el emisor, y para quién dice estar acuñado el token.
    pub emisor: String,
    pub audiencia: String,
    /// El valor de la reclamación que `subject.claim` nombra.
    pub sujeto: String,
    /// Las pertenencias, de la reclamación que `subject.roles` nombra.
    pub roles: Vec<String>,
    /// Las reclamaciones declaradas en `claims`, por su nombre interno.
    pub claims: BTreeMap<String, String>,
    /// `read` · `aggregate` · `export` · `invoke`.
    pub accion: String,
    /// El nombre cualificado de la propiedad.
    pub propiedad: String,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denegacion {
    /// Un `forbid` la alcanzó. Gana siempre, y se nombra.
    Prohibida,
    /// Ninguna política alcanza esa propiedad. **No es un fallo: es P4.**
    SinPolitica,
    /// Hay políticas que la alcanzan y ninguna casó — por el rol, por la
    /// finalidad, por una condición. Se nombran para que haya contra qué mirar.
    NingunaCaso { candidatas: Vec<String> },
}

#[derive(Debug, Clone)]
pub enum Veredicto {
    Permitido {
        /// Los `@id` **nuestros**, no los `policyN` de Cedar.
        politicas: Vec<String>,
        obligaciones: Vec<String>,
        mascaras: Vec<String>,
        ambitos: Vec<String>,
    },
    Denegado {
        politicas: Vec<String>,
        porque: Denegacion,
    },
    /// La petición **no existe**: no satisface el esquema, o su emisor no es el
    /// declarado. Se rechaza **antes** de ① y no es una decisión de política
    /// (`05-ejecutor` §6.1).
    Invalida(String),
}

impl Motor {
    /// El emisor y la audiencia que el `RequestPolicy` declara.
    fn frontera(&self) -> Option<(String, String)> {
        let rp = self.paquete.request_policy()?;
        let i = rp.section("issuer")?;
        Some((
            i.get("url")?.1.as_str()?.to_string(),
            i.get("audience")?.1.as_str()?.to_string(),
        ))
    }

    /// El tipo Cedar del principal, del `subject.entity` declarado.
    fn tipo_principal(&self) -> Option<String> {
        let rp = self.paquete.request_policy()?;
        let qn = rp.section("subject")?.get("entity")?.1.as_str()?;
        Some(qn.rsplit('.').next().unwrap_or(qn).to_string())
    }

    pub fn autorizar(&self, p: &Peticion) -> Veredicto {
        // ── La frontera, antes que nada ──────────────────────────────────────
        //
        // No es una denegación: es una petición que no existe. Y la firma en sí
        // NO se comprueba aquí — verificarla exige la red (JWKS), que es una
        // capacidad que este crate no tiene y que se decide con `serve`.
        let Some((emisor, audiencia)) = self.frontera() else {
            return Veredicto::Invalida(
                "el paquete no declara ningún `RequestPolicy`: no hay frontera de identidad \
                 contra la que verificar nada"
                    .into(),
            );
        };
        if p.emisor != emisor {
            return Veredicto::Invalida(format!(
                "emisor `{}`, y el declarado es `{emisor}`",
                p.emisor
            ));
        }
        if p.audiencia != audiencia {
            return Veredicto::Invalida(format!(
                "audiencia `{}`, y la declarada es `{audiencia}` — un token acuñado para otro \
                 destinatario es un token robado, aunque lo firme quien debe",
                p.audiencia
            ));
        }

        let Some(tipo) = self.tipo_principal() else {
            return Veredicto::Invalida("`subject.entity` no declara el tipo del principal".into());
        };

        // ── El almacén: del bundle, sin abrir nada ───────────────────────────
        let mut entidades: Vec<Entity> = self
            .etiquetas_por_propiedad()
            .into_iter()
            .filter_map(|(prop, etiquetas)| {
                let uid = EntityUid::from_str(&format!("Property::{:?}", prop)).ok()?;
                let padres: HashSet<EntityUid> = etiquetas
                    .iter()
                    .filter_map(|e| EntityUid::from_str(&format!("Label::{e:?}")).ok())
                    .collect();
                Some(Entity::new_no_attrs(uid, padres))
            })
            .collect();

        let attrs: HashMap<String, RestrictedExpression> = p
            .claims
            .iter()
            .map(|(k, v)| (k.clone(), RestrictedExpression::new_string(v.clone())))
            .collect();
        let padres: HashSet<EntityUid> = p
            .roles
            .iter()
            .filter_map(|r| EntityUid::from_str(&format!("Role::{r:?}")).ok())
            .collect();
        let Ok(uid_principal) = EntityUid::from_str(&format!("{tipo}::{:?}", p.sujeto)) else {
            return Veredicto::Invalida(format!("`{}` no es un identificador válido", p.sujeto));
        };
        match Entity::new(uid_principal.clone(), attrs, padres) {
            Ok(e) => entidades.push(e),
            Err(err) => return Veredicto::Invalida(err.to_string()),
        }

        // Aquí se rechaza la reclamación que el esquema no declara: el almacén
        // no conforma, y eso es P4 en la entrada.
        let entidades = match Entities::from_entities(entidades, Some(&self.esquema)) {
            Ok(e) => e,
            Err(err) => return Veredicto::Invalida(err.to_string()),
        };

        // ── La petición ──────────────────────────────────────────────────────
        let Ok(contexto) = Context::from_pairs([(
            "purpose".to_string(),
            RestrictedExpression::new_string(p.purpose.clone()),
        )]) else {
            return Veredicto::Invalida("no se pudo construir el contexto".into());
        };
        let (Ok(accion), Ok(recurso)) = (
            EntityUid::from_str(&format!("Action::{:?}", p.accion)),
            EntityUid::from_str(&format!("Property::{:?}", p.propiedad)),
        ) else {
            return Veredicto::Invalida(format!(
                "`{}` o `{}` no son identificadores válidos",
                p.accion, p.propiedad
            ));
        };
        let peticion = match Request::new(
            uid_principal,
            accion,
            recurso,
            contexto,
            Some(&self.esquema),
        ) {
            Ok(r) => r,
            Err(err) => return Veredicto::Invalida(err.to_string()),
        };

        // ── ① ────────────────────────────────────────────────────────────────
        let respuesta = Authorizer::new().is_authorized(&peticion, &self.politicas, &entidades);
        let decidieron: Vec<String> = respuesta
            .diagnostics()
            .reason()
            .map(|id| self.nombre_de(&id.to_string()))
            .collect();

        match respuesta.decision() {
            Decision::Allow => {
                let (mut obligaciones, mut mascaras, mut ambitos) =
                    (Vec::new(), Vec::new(), Vec::new());
                for id in &decidieron {
                    if let Some(pol) = self.leidas.get(id) {
                        obligaciones.extend(pol.obligations.iter().cloned());
                        mascaras.extend(pol.masks.iter().cloned());
                        ambitos.extend(pol.scopes.iter().cloned());
                    }
                }
                Veredicto::Permitido {
                    politicas: decidieron,
                    obligaciones,
                    mascaras,
                    ambitos,
                }
            }
            Decision::Deny => {
                let porque = if !decidieron.is_empty() {
                    Denegacion::Prohibida
                } else {
                    let candidatas: Vec<String> = self
                        .alcance
                        .iter()
                        .filter(|(_, props)| props.iter().any(|x| *x == p.propiedad))
                        .map(|(id, _)| id.clone())
                        .collect();
                    if candidatas.is_empty() {
                        Denegacion::SinPolitica
                    } else {
                        Denegacion::NingunaCaso { candidatas }
                    }
                };
                Veredicto::Denegado {
                    politicas: decidieron,
                    porque,
                }
            }
        }
    }
}
