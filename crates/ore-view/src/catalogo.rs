//! El **catálogo** y el **expansor**: vista sobre vistas, en un solo plan.
//!
//! Aquí es donde *«un pipeline es una cadena de vistas»* deja de ser una frase.
//! Expandir es **sustituir cada [`Nodo::Referencia`] por el cuerpo de la vista
//! que nombra**, recursivamente, hasta que no queda ninguna. Lo que sale es un
//! plan completo, y eso es comprobable: [`Nodo::expandido`].
//!
//! Y con eso no hace falta un concepto de «transformación» ni de «pipeline».
//! Foundry tiene datasets **y** transforms; Snowflake tiene tablas, vistas,
//! *dynamic tables* **y** tasks. Cognite lo hace con una sola cosa, y de ahí se
//! toma: una vista mapea de **uno o varios** orígenes, y esos orígenes pueden ser
//! otras vistas.
//!
//! # Un ciclo se rechaza nombrando el ciclo
//!
//! No basta con detectarlo. `a → b → c → a` escrito así es un error que se
//! arregla; *«hay una recursión»* es un error que se busca. Es la misma
//! diferencia que hay entre decir *«hay tres políticas y ninguna casó»* y
//! devolver `Deny`.
//!
//! # El camino, no un conjunto de visitados
//!
//! Es el error clásico de este algoritmo y conviene decirlo: si se lleva un
//! `visitados` global, un **rombo** —dos vistas que se apoyan en una tercera—
//! se confunde con un ciclo. Lo que hay que llevar es **el camino actual**, que
//! es lo que distingue *«ya pasé por aquí»* de *«estoy dentro de aquí»*.
//! Tiene su comprobación.
//!
//! # Lo que cuesta expandir, dicho
//!
//! Incorporar duplica: una vista referenciada dos veces se copia dos veces, y
//! una cadena de rombos crece exponencialmente. **Es real y no se guarda de
//! ello**, porque la entrada son las vistas del propio repositorio y no un texto
//! ajeno. Lo que de verdad lo arregla es contestar desde una materialización en
//! vez de recomputar, que es M5.
//!
//! # Y lo que M1 no hace
//!
//! **No hay alias.** Dos referencias a la misma vista dentro de un plan producen
//! dos subárboles idénticos, y juntarlos choca por nombre de columna — que el
//! tipado dice con [`crate::Desajuste::ColisionAlUnir`]. Es una limitación real
//! y sale por su nombre en vez de producir algo raro.

use crate::plan::Nodo;
use std::collections::BTreeMap;

/// Una vista, **en lo que este motor necesita de ella**: un nombre y un cuerpo.
///
/// Ni frescura, ni capacidades, ni dónde se materializa. No es un olvido: no hay
/// todavía quien las consuma, y añadir campos antes de que exista el consumidor
/// es inventar una forma que nadie ha medido. Entran en M4 y con su prueba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vista {
    pub nombre: String,
    /// Un plan cuyas hojas pueden ser lecturas de una fuente **o referencias a
    /// otras vistas**.
    pub cuerpo: Nodo,
}

impl Vista {
    pub fn nueva(nombre: &str, cuerpo: Nodo) -> Vista {
        Vista {
            nombre: nombre.to_string(),
            cuerpo,
        }
    }
}

/// Por qué no se pudo expandir. Los dos son fallos **del catálogo**, no de una
/// consulta: ocurren antes de que nadie pregunte nada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expansion {
    /// Se nombra una vista que no está. `desde` dice **quién la nombró**, que es
    /// lo que convierte el error en algo que se arregla sin buscar.
    NoExiste {
        vista: String,
        desde: Option<String>,
    },
    /// Una vista se alcanza a sí misma. La cadena va **en orden y cerrada**:
    /// `[a, b, c, a]`.
    Ciclo { cadena: Vec<String> },
}

impl Expansion {
    pub fn como_texto(&self) -> String {
        match self {
            Expansion::NoExiste { vista, desde: None } => {
                format!("`{vista}` no está en el catálogo")
            }
            Expansion::NoExiste {
                vista,
                desde: Some(d),
            } => format!("`{d}` nombra a `{vista}`, que no está en el catálogo"),
            Expansion::Ciclo { cadena } => {
                format!("ciclo entre vistas · {}", cadena.join(" → "))
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Catalogo {
    vistas: BTreeMap<String, Vista>,
}

impl Catalogo {
    pub fn con(vistas: impl IntoIterator<Item = Vista>) -> Catalogo {
        Catalogo {
            vistas: vistas.into_iter().map(|v| (v.nombre.clone(), v)).collect(),
        }
    }

    pub fn nombres(&self) -> impl Iterator<Item = &str> {
        self.vistas.keys().map(String::as_str)
    }

    pub fn de(&self, nombre: &str) -> Option<&Vista> {
        self.vistas.get(nombre)
    }

    /// **El plan completo de una vista.**
    pub fn expandir(&self, nombre: &str) -> Result<Nodo, Expansion> {
        let v = self.vistas.get(nombre).ok_or(Expansion::NoExiste {
            vista: nombre.to_string(),
            desde: None,
        })?;
        let mut camino = vec![nombre.to_string()];
        self.dentro(&v.cuerpo, &mut camino)
    }

    /// Expande un plan suelto: útil para una consulta que nombra vistas sin ser
    /// una.
    pub fn expandir_plan(&self, n: &Nodo) -> Result<Nodo, Expansion> {
        let mut camino = Vec::new();
        self.dentro(n, &mut camino)
    }

    /// `camino` es **la pila del recorrido**, no un conjunto de visitados. Ver
    /// la cabecera: con un conjunto, un rombo se confundiría con un ciclo.
    fn dentro(&self, n: &Nodo, camino: &mut Vec<String>) -> Result<Nodo, Expansion> {
        Ok(match n {
            Nodo::Referencia(v) => {
                if let Some(i) = camino.iter().position(|x| x == v) {
                    // La cadena se corta desde la primera aparición y se cierra
                    // repitiendo el nombre: así se lee como el ciclo que es.
                    let mut cadena: Vec<String> = camino[i..].to_vec();
                    cadena.push(v.clone());
                    return Err(Expansion::Ciclo { cadena });
                }
                let vista = self.vistas.get(v).ok_or(Expansion::NoExiste {
                    vista: v.clone(),
                    desde: camino.last().cloned(),
                })?;
                camino.push(v.clone());
                let dentro = self.dentro(&vista.cuerpo, camino)?;
                camino.pop();
                dentro
            }

            Nodo::Lee(_) => n.clone(),

            Nodo::Proyecta { entrada, campos } => Nodo::Proyecta {
                entrada: Box::new(self.dentro(entrada, camino)?),
                campos: campos.clone(),
            },
            Nodo::Filtra { entrada, predicado } => Nodo::Filtra {
                entrada: Box::new(self.dentro(entrada, camino)?),
                predicado: predicado.clone(),
            },
            Nodo::Une {
                izquierda,
                derecha,
                tipo,
                sobre,
            } => Nodo::Une {
                izquierda: Box::new(self.dentro(izquierda, camino)?),
                derecha: Box::new(self.dentro(derecha, camino)?),
                tipo: *tipo,
                sobre: sobre.clone(),
            },
            Nodo::Agrupa {
                entrada,
                por,
                agregados,
            } => Nodo::Agrupa {
                entrada: Box::new(self.dentro(entrada, camino)?),
                por: por.clone(),
                agregados: agregados.clone(),
            },
            Nodo::Unifica(v) => Nodo::Unifica(
                v.iter()
                    .map(|r| self.dentro(r, camino))
                    .collect::<Result<_, _>>()?,
            ),
            Nodo::Distingue(e) => Nodo::Distingue(Box::new(self.dentro(e, camino)?)),
            Nodo::Limita { entrada, n } => Nodo::Limita {
                entrada: Box::new(self.dentro(entrada, camino)?),
                n: *n,
            },
        })
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Comparador, Expr, Junta, Lectura, Valor};
    use ore_core::types::parse_type;

    fn hoja(objeto: &str, campos: &[(&str, &str)]) -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: objeto.into(),
            campos: campos
                .iter()
                .map(|(n, x)| ((*n).to_string(), parse_type(x).expect("tipo")))
                .collect(),
        })
    }

    fn pedidos() -> Nodo {
        hoja(
            "ventas.pedidos",
            &[("id", "Integer"), ("total", "Decimal"), ("pais", "String")],
        )
    }

    fn ref_a(v: &str) -> Nodo {
        Nodo::Referencia(v.to_string())
    }

    fn filtra(e: Nodo, campo: &str, v: &str) -> Nodo {
        Nodo::Filtra {
            entrada: Box::new(e),
            predicado: Expr::Compara {
                op: Comparador::Igual,
                izquierda: Box::new(Expr::campo(campo)),
                derecha: Box::new(Expr::Literal(Valor::Cadena(v.into()))),
            },
        }
    }

    /// **El criterio de M1.** Una cadena de N vistas produce **un** plan, y en
    /// él ya no queda ninguna referencia.
    #[test]
    fn una_cadena_de_vistas_produce_un_solo_plan() {
        let c = Catalogo::con([
            Vista::nueva("crudos", pedidos()),
            Vista::nueva("espanoles", filtra(ref_a("crudos"), "pais", "ES")),
            Vista::nueva(
                "resumen",
                Nodo::Proyecta {
                    entrada: Box::new(ref_a("espanoles")),
                    campos: [("cuanto".to_string(), Expr::campo("total"))].into(),
                },
            ),
        ]);

        let p = c.expandir("resumen").expect("se expande");
        assert!(p.expandido(), "quedaron referencias: {:?}", p.referencias());
        assert_eq!(p.lecturas().len(), 1);
        assert_eq!(p.lecturas()[0].objeto, "ventas.pedidos");

        // Y el plan completo **sí** se puede tipar, que es para lo que se
        // expande: la cadena entera vale una columna.
        let e = crate::esquema(&p).expect("cuadra");
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e["cuanto"], parse_type("Decimal").unwrap());
    }

    /// Y sin expandir **no se puede tipar**, en vez de dar un esquema parcial
    /// que parece bueno.
    #[test]
    fn un_plan_sin_expandir_no_se_tipa() {
        assert_eq!(
            crate::esquema(&filtra(ref_a("crudos"), "pais", "ES")),
            Err(crate::Desajuste::SinExpandir {
                vista: "crudos".into()
            })
        );
    }

    /// **El criterio de M1.** Un ciclo se rechaza **nombrando el ciclo**, en
    /// orden y cerrado. *«Hay una recursión»* es un error que se busca; `a → b →
    /// c → a` es un error que se arregla.
    #[test]
    fn un_ciclo_se_rechaza_nombrando_el_ciclo() {
        let c = Catalogo::con([
            Vista::nueva("a", filtra(ref_a("b"), "pais", "ES")),
            Vista::nueva("b", filtra(ref_a("c"), "pais", "PT")),
            Vista::nueva("c", filtra(ref_a("a"), "pais", "FR")),
        ]);
        assert_eq!(
            c.expandir("a"),
            Err(Expansion::Ciclo {
                cadena: vec!["a".into(), "b".into(), "c".into(), "a".into()]
            })
        );
        assert_eq!(
            c.expandir("a").unwrap_err().como_texto(),
            "ciclo entre vistas · a → b → c → a"
        );
    }

    /// Y una vista que se nombra a sí misma es el mismo caso con longitud uno.
    #[test]
    fn una_vista_que_se_nombra_a_si_misma_tambien() {
        let c = Catalogo::con([Vista::nueva("sola", filtra(ref_a("sola"), "pais", "ES"))]);
        assert_eq!(
            c.expandir("sola"),
            Err(Expansion::Ciclo {
                cadena: vec!["sola".into(), "sola".into()]
            })
        );
    }

    /// **El error clásico de este algoritmo.** Un rombo —dos vistas que se
    /// apoyan en la misma tercera— **no es un ciclo**, y con un conjunto de
    /// visitados en vez del camino se confundiría con uno.
    ///
    /// Sin esta comprobación, la primera vez que alguien reutilizara una vista
    /// limpia en dos sitios se encontraría un «ciclo» que no existe.
    #[test]
    fn un_rombo_no_es_un_ciclo() {
        let c = Catalogo::con([
            Vista::nueva("base", pedidos()),
            Vista::nueva("es", filtra(ref_a("base"), "pais", "ES")),
            Vista::nueva(
                "solo_id",
                Nodo::Proyecta {
                    entrada: Box::new(ref_a("base")),
                    campos: [("id_pedido".to_string(), Expr::campo("id"))].into(),
                },
            ),
            Vista::nueva(
                "junta",
                Nodo::Une {
                    izquierda: Box::new(ref_a("es")),
                    derecha: Box::new(ref_a("solo_id")),
                    tipo: Junta::Interna,
                    sobre: vec![("id".into(), "id_pedido".into())],
                },
            ),
        ]);

        let p = c.expandir("junta").expect("un rombo no es un ciclo");
        assert!(p.expandido());
        // Incorporar duplica: `base` sale dos veces. Es el coste real de
        // expandir, y lo que de verdad lo arregla es M5.
        assert_eq!(p.lecturas().len(), 2);
        assert!(crate::esquema(&p).is_ok());
    }

    /// Una vista que no está se dice **con quién la nombró**: sin eso, el error
    /// obliga a buscar quién la mencionaba entre todas las del catálogo.
    #[test]
    fn una_vista_que_falta_dice_quien_la_nombro() {
        let c = Catalogo::con([Vista::nueva("a", filtra(ref_a("fantasma"), "pais", "ES"))]);
        assert_eq!(
            c.expandir("a"),
            Err(Expansion::NoExiste {
                vista: "fantasma".into(),
                desde: Some("a".into())
            })
        );
        // Y pedir una que no existe, sin nadie que la nombrara, no inventa un
        // culpable.
        assert_eq!(
            c.expandir("tampoco"),
            Err(Expansion::NoExiste {
                vista: "tampoco".into(),
                desde: None
            })
        );
    }

    /// Expandir es **determinista**: es la condición para que el digest de un
    /// plan expandido sirva de identidad, que es de lo que vive M5.
    #[test]
    fn expandir_dos_veces_da_el_mismo_digest() {
        let c = Catalogo::con([
            Vista::nueva("base", pedidos()),
            Vista::nueva("es", filtra(ref_a("base"), "pais", "ES")),
        ]);
        assert_eq!(
            c.expandir("es").unwrap().digest(),
            c.expandir("es").unwrap().digest()
        );
        // Y expandir una vista es lo mismo que expandir su cuerpo suelto.
        assert_eq!(
            c.expandir("es").unwrap().digest(),
            c.expandir_plan(&c.de("es").unwrap().cuerpo)
                .unwrap()
                .digest()
        );
    }

    /// Un plan sin referencias sale de la expansión igual que entró: expandir no
    /// es una oportunidad de reescribir nada.
    #[test]
    fn expandir_lo_que_ya_esta_completo_no_lo_toca() {
        let c = Catalogo::default();
        let p = filtra(pedidos(), "pais", "ES");
        assert_eq!(c.expandir_plan(&p).expect("nada que expandir"), p);
    }
}
