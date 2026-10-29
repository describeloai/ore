//! **La grabadora** (0079 §19.2, E1a·5c·5c·2): una puerta que envuelve a otra, deja pasar las
//! lecturas y **apunta** las escrituras en vez de hacerlas.
//!
//! Es lo que hace baratas las escrituras de definición (retirar un `Dataset`, retirar una base,
//! renombrar un schema) con el catálogo: el código que escribiría los punteros no cambia, lo que
//! escribiría sale grabado en la [`Cinta`], y eso es el plan. Quien graba lo lleva en el commit de
//! git ([`bloque`]) y lo aplica después, con `idempotencia = git:<sha>`: el puntero cambia sólo si
//! git dijo que sí.
//!
//! - **Lo grabado no se lee.** Una lectura tras una escritura grabada ve lo de antes: las rutas de
//!   definición escriben al final (retirar) o deshacen al fallar (renombrar), y lo que se graba
//!   cuando la ruta falla no se aplica nunca.
//! - **Lo que nunca podría escribirse se dice ya**, no al aplicar: un nombre que no es
//!   `base.nombre` ni `base.schema.nombre` ([`Fallo::Nombre`]), o un `Sustituir` que no es un
//!   objeto. El *compare-and-set* se comprueba al aplicar, como siempre.
//! - **Varias escrituras se aplican como una**, en su orden: lo grabado es [`Cinta::grabado`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::json::Json;

use super::protocolo::{cambio_a_json, cambio_de_json};
use super::{Cambio, Contexto, Fallo, Leido, Punteros, Que, Ramas, Reclamados, Version, partes};

/// Lo grabado: los cambios de cada `escribir`, en orden, con su contexto. Se comparte entre la
/// grabadora y quien la abrió (y entre hilos: la puerta heredada sirve desde el suyo).
#[derive(Debug, Clone, Default)]
pub struct Cinta(Arc<Mutex<Vec<Grabada>>>);

/// Una escritura grabada: sus cambios y su contexto.
type Grabada = (Vec<Cambio>, Contexto);

impl Cinta {
    pub fn nueva() -> Cinta {
        Cinta::default()
    }

    /// Todo lo grabado como una escritura, en su orden, y el contexto de la primera (`None`: no se
    /// grabó nada).
    pub fn grabado(&self) -> Option<(Vec<Cambio>, Contexto)> {
        let g = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ctx = g.first()?.1.clone();
        Some((g.iter().flat_map(|(c, _)| c.iter().cloned()).collect(), ctx))
    }

    fn grabar(&self, cambios: &[Cambio], ctx: &Contexto) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((cambios.to_vec(), ctx.clone()));
    }
}

/// **La grabadora** sobre la puerta `P`. Ver el módulo.
pub struct Grabadora<P> {
    dentro: P,
    cinta: Cinta,
}

impl<P> Grabadora<P> {
    pub fn sobre(dentro: P, cinta: Cinta) -> Grabadora<P> {
        Grabadora { dentro, cinta }
    }
}

impl<P: Punteros> Punteros for Grabadora<P> {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo> {
        self.dentro.uno(nombre)
    }
    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        self.dentro.todos()
    }
    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo> {
        self.dentro.historia(nombre, hasta)
    }
    fn escribir(&self, cambios: &[Cambio], ctx: &Contexto) -> Result<(), Fallo> {
        for c in cambios {
            if partes(&c.nombre).is_none() {
                return Err(Fallo::Nombre(c.nombre.clone()));
            }
            if let Que::Sustituir(j) = &c.que
                && !matches!(j, Json::Obj(_))
            {
                return Err(Fallo::Almacen(format!(
                    "el puntero de `{}` no es un objeto",
                    c.nombre
                )));
            }
        }
        if !cambios.is_empty() {
            self.cinta.grabar(cambios, ctx);
        }
        Ok(())
    }
    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        self.dentro.reclamados()
    }
    fn version(&self) -> Result<String, Fallo> {
        self.dentro.version()
    }
}

/// Las ramas no se graban: su ciclo de vida no es una escritura de definición.
impl<P: Ramas> Ramas for Grabadora<P> {
    fn crear(&self, rama: &str, desde: &str) -> Result<(), Fallo> {
        self.dentro.crear(rama, desde)
    }
    fn borrar(&self, rama: &str) -> Result<(), Fallo> {
        self.dentro.borrar(rama)
    }
    fn traer(&self, rama: &str) -> Result<(), Fallo> {
        self.dentro.traer(rama)
    }
}

/// Una puerta en una caja es una puerta: lo que deja grabar sobre la de ore-serve
/// (`Box<dyn Puerta>`).
impl<P: Punteros + ?Sized> Punteros for Box<P> {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo> {
        (**self).uno(nombre)
    }
    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        (**self).todos()
    }
    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo> {
        (**self).historia(nombre, hasta)
    }
    fn escribir(&self, cambios: &[Cambio], ctx: &Contexto) -> Result<(), Fallo> {
        (**self).escribir(cambios, ctx)
    }
    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        (**self).reclamados()
    }
    fn version(&self) -> Result<String, Fallo> {
        (**self).version()
    }
}

impl<P: Ramas + ?Sized> Ramas for Box<P> {
    fn crear(&self, rama: &str, desde: &str) -> Result<(), Fallo> {
        (**self).crear(rama, desde)
    }
    fn borrar(&self, rama: &str) -> Result<(), Fallo> {
        (**self).borrar(rama)
    }
    fn traer(&self, rama: &str) -> Result<(), Fallo> {
        (**self).traer(rama)
    }
}

// ── El bloque `Punteros:` del commit ───────────────────────────────────────────────────────────

/// Cómo empieza la línea del commit de git que lleva la intención sobre los punteros.
pub const PREFIJO: &str = "Punteros: ";

/// **Lo que un commit de git lleva para el catálogo** (0079 §19.2, «git primero, y git lleva la
/// intención»): lo que el catálogo aplica tras el commit, y lo que el reconciliador (·5c·5c·4)
/// aplica si eso no llegó a pasar. Idempotente por el sha del commit.
#[derive(Debug, Clone)]
pub enum Bloque {
    /// Retirar o renombrar: los cambios grabados, en `r` (`main` o una rama).
    Definicion {
        r: String,
        operacion: String,
        cambios: Vec<Cambio>,
    },
    /// Una fusión de `rama` en `main`: el plan del catálogo (`plan_fusion`), tal cual.
    Fusion { rama: String, plan: Json },
}

/// `mensaje` con el bloque al final, en su línea (JSON en una línea, JCS).
pub fn bloque(mensaje: &str, b: &Bloque) -> String {
    let j = match b {
        Bloque::Definicion {
            r,
            operacion,
            cambios,
        } => Json::obj([
            ("ref", Json::s(r)),
            ("operacion", Json::s(operacion)),
            (
                "cambios",
                Json::Arr(cambios.iter().map(cambio_a_json).collect()),
            ),
        ]),
        Bloque::Fusion { rama, plan } => {
            Json::obj([("fusion", Json::s(rama)), ("plan", plan.clone())])
        }
    };
    format!("{}\n\n{PREFIJO}{}", mensaje.trim_end(), j.jcs())
}

/// El bloque de un mensaje de commit, si lo lleva. Un bloque que no se entiende es `Err`: el
/// reconciliador lo dice, no lo salta en silencio.
pub fn leer_bloque(mensaje: &str) -> Option<Result<Bloque, Fallo>> {
    let linea = mensaje
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(PREFIJO))?;
    let mal = |m: &str| Fallo::Almacen(format!("el bloque `Punteros:` del commit: {m}"));
    Some((|| {
        let n = crate::parse::parse(linea).map_err(|e| mal(&format!("{e:?}")))?;
        let texto = |k: &str| {
            n.get(k)
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
                .ok_or_else(|| mal(&format!("falta `{k}`")))
        };
        if let Some((_, plan)) = n.get("plan") {
            return Ok(Bloque::Fusion {
                rama: texto("fusion")?,
                plan: Json::de_node_fiel(plan),
            });
        }
        let cambios = n
            .get("cambios")
            .map(|(_, c)| {
                c.items()
                    .iter()
                    .map(cambio_de_json)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .ok_or_else(|| mal("falta `cambios`"))?;
        Ok(Bloque::Definicion {
            r: texto("ref")?,
            operacion: texto("operacion")?,
            cambios,
        })
    })())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::punteros::{Esperado, Ficheros};

    fn retirar(n: &str) -> Cambio {
        Cambio {
            nombre: n.into(),
            que: Que::Retirar,
            esperado: Esperado::Cualquiera,
        }
    }

    #[test]
    fn graba_en_vez_de_escribir_y_deja_pasar_las_lecturas() {
        let d = std::env::temp_dir().join(format!("ore-grabadora-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("datasets/ventas/default")).unwrap();
        std::fs::write(
            d.join("datasets/ventas/default/pedidos.json"),
            "{\"estado\": \"copiada\"}\n",
        )
        .unwrap();
        let cinta = Cinta::nueva();
        let g = Grabadora::sobre(Ficheros::en(&d), cinta.clone());
        assert!(cinta.grabado().is_none());
        let ctx = Contexto {
            autor: "ana".into(),
            operacion: "retirar".into(),
            idempotencia: None,
        };
        g.escribir(&[retirar("ventas.pedidos")], &ctx).unwrap();
        g.escribir(&[], &ctx).unwrap();
        g.escribir(&[retirar("ventas.otro")], &ctx).unwrap();
        // Nada se escribió: la lectura ve lo de antes.
        assert!(g.uno("ventas.pedidos").unwrap().is_some());
        assert!(d.join("datasets/ventas/default/pedidos.json").is_file());
        let (cambios, c) = cinta.grabado().unwrap();
        assert_eq!(
            cambios
                .iter()
                .map(|c| c.nombre.as_str())
                .collect::<Vec<_>>(),
            ["ventas.pedidos", "ventas.otro"]
        );
        assert_eq!(c.operacion, "retirar");
        // Lo que nunca podría escribirse, ya.
        assert!(matches!(
            g.escribir(&[retirar("sin_base")], &ctx),
            Err(Fallo::Nombre(_))
        ));
        let mal = Cambio {
            que: Que::Sustituir(Json::s("x")),
            ..retirar("ventas.x")
        };
        assert!(g.escribir(&[mal], &ctx).is_err());
        assert_eq!(cinta.grabado().unwrap().0.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_bloque_va_y_vuelve_por_el_mensaje() {
        let b = Bloque::Definicion {
            r: "main".into(),
            operacion: "renombrar-schema".into(),
            cambios: vec![
                Cambio {
                    nombre: "ventas.eu.p".into(),
                    que: Que::Sustituir(Json::obj([("filas", Json::Int(3))])),
                    esperado: Esperado::Nada,
                },
                retirar("ventas.es.p"),
            ],
        };
        let m = bloque("`ventas`: el schema `es` pasa a llamarse `eu`\n", &b);
        assert!(m.starts_with("`ventas`: el schema"));
        assert_eq!(m.lines().filter(|l| l.starts_with(PREFIJO)).count(), 1);
        let Some(Ok(Bloque::Definicion {
            r,
            operacion,
            cambios,
        })) = leer_bloque(&m)
        else {
            panic!("{m}")
        };
        assert_eq!(
            (r.as_str(), operacion.as_str()),
            ("main", "renombrar-schema")
        );
        assert_eq!(cambios.len(), 2);
        assert!(matches!(cambios[0].esperado, Esperado::Nada));
        let f = bloque(
            "Fusionar #3",
            &Bloque::Fusion {
                rama: "ana/x".into(),
                plan: Json::obj([("estado", Json::s("plan"))]),
            },
        );
        assert!(
            matches!(leer_bloque(&f), Some(Ok(Bloque::Fusion { rama, .. })) if rama == "ana/x")
        );
        assert!(leer_bloque("un commit cualquiera").is_none());
        assert!(matches!(
            leer_bloque("x\n\nPunteros: {\"ref\": \"main\"}"),
            Some(Err(_))
        ));
    }
}
