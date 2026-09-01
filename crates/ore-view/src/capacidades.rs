//! **Capacidades y reparto:** qué hace el origen y qué queda de residuo.
//!
//! Es lo que convierte un escaneo en una búsqueda por clave, que es el
//! argumento entero del [ADR 0006](../../docs/decisions/0006-el-artefacto-de-topologia.md):
//! *«el índice convierte escaneos en búsquedas por clave»*. Sin bajar los
//! predicados hasta la hoja, cada travesía terminaría pidiendo la tabla entera.
//!
//! # Por declaración, no por intento
//!
//! Trino negocia **por intento**: el optimizador llama a `applyFilter`,
//! `applyProjection`, `applyAggregation`, `applyJoin`, `applyLimit`, y el
//! conector devuelve `Optional.empty()` si no puede. Es exacto, y exige tener el
//! conector delante.
//!
//! Aquí se declara, y el precio es el opuesto:
//!
//! | | Ventaja | Precio |
//! |---|---|---|
//! | por intento | refleja lo que el conector hace de verdad | no se puede planificar sin conexión |
//! | **por declaración** | **un plan se rechaza sin abrir nada** | la declaración puede mentir |
//!
//! > **Se declara, y se deja que el driver contradiga.** Una capacidad declarada
//! > que el driver rechaza es una divergencia con nombre, no un fallo en
//! > ejecución.
//!
//! # El aviso que hay que dejar por escrito
//!
//! Bajar un predicado y quitarlo del residuo **confía en la declaración**. Para
//! un filtro cualquiera eso es una optimización; para un filtro que restringe
//! **qué puede ver un principal**, confiar es devolver filas de más si el origen
//! lo ignora.
//!
//! Esta pieza no sabe cuál es cuál —eso lo sabe el conducto—, así que hace lo
//! útil por defecto: lo baja y lo quita del residuo, **y deja el predicado
//! escrito en la [`Peticion`]** para que quien sí lo sepa pueda volver a
//! aplicarlo. Cuando esto se absorba, el campo que marca los de autorización ya
//! existe: es el `ambito` de un filtro.
//!
//! # Lo que M4 **no** hace, y no por falta de ganas
//!
//! **No poda columnas.** Bajar predicados es lo que convierte un escaneo en una
//! búsqueda; pedir menos columnas es una segunda optimización, con su propia
//! medida, y hacerla sin medirla sería inventarla.
//!
//! **No baja agregados ni juntas.** Son las dos que más cambian el reparto y las
//! dos que más fácil se hacen mal. Cada una necesita su criterio y su prueba.
//!
//! # Y una trampa que sí está encodada
//!
//! **Un predicado no baja por debajo de un límite.** `LIMIT 10` y luego filtrar
//! no es lo mismo que filtrar y luego `LIMIT 10`, y es de los errores de
//! optimizador que devuelven un resultado plausible. Tiene su comprobación.

use crate::plan::{Comparador, Expr, Nodo};
use std::collections::{BTreeMap, BTreeSet};

/// Si el origen admite que se le pida la tabla entera.
///
/// **La ausencia es una negativa**, no una laguna: `05-ejecutor` §5.1 dice que
/// sin capacidades declaradas un origen sirve la búsqueda por clave y nada más.
/// Es P4 — denegación por defecto — aplicada al reparto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recorrido {
    #[default]
    Prohibido,
    Permitido,
}

/// Qué sabe hacer un origen.
#[derive(Debug, Clone, Default)]
pub struct Capacidades {
    /// Los comparadores que sabe empujar.
    pub predicados: BTreeSet<Comparador>,
    /// `campo IN (…)` de una pieza.
    pub en_conjunto: bool,
    pub es_nulo: bool,
    pub disyuncion: bool,
    pub negacion: bool,
    /// El dialecto que habla, si habla alguno. Es lo que hace **útil** a una
    /// expresión opaca en vez de solo costosa: una opaca escrita en el dialecto
    /// del origen se le puede pasar tal cual.
    pub dialecto: Option<String>,
    pub recorrido: Recorrido,
    /// Columnas que **tienen** que llegar filtradas. Una API con cuota las
    /// declara, y pedirle sin ellas es una petición que va a fallar.
    pub filtros_obligatorios: Vec<String>,
}

/// Lo que se le pide a una hoja.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peticion {
    pub datasource: String,
    pub objeto: String,
    /// Los predicados que el origen aplica. **Se quedan escritos aquí** aunque
    /// salgan del residuo: ver el aviso de la cabecera.
    pub filtros: Vec<Expr>,
}

/// El reparto: qué hace cada origen y qué queda por hacer encima.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reparto {
    pub peticiones: Vec<Peticion>,
    /// El plan que queda. Los predicados bajados **ya no están** aquí.
    pub residuo: Nodo,
}

/// Por qué un plan no se puede repartir. Los dos ocurren **antes de abrir
/// ninguna conexión**, que es toda la gracia de declarar en vez de intentar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rechazo {
    /// La hoja quedaría sin ningún filtro y el origen no admite recorrido
    /// completo — o no declaró capacidades, que significa lo mismo.
    RecorridoCompleto {
        datasource: String,
        objeto: String,
        porque: &'static str,
    },
    /// El origen exige que una columna llegue filtrada y no llega.
    FiltroObligatorioAusente {
        datasource: String,
        objeto: String,
        columna: String,
    },
    /// Se reparte un plan que todavía nombra una vista.
    SinExpandir { vista: String },
}

impl Rechazo {
    pub fn como_texto(&self) -> String {
        match self {
            Rechazo::RecorridoCompleto {
                datasource,
                objeto,
                porque,
            } => format!("recorrido completo · `{datasource}`·`{objeto}` — {porque}"),
            Rechazo::FiltroObligatorioAusente {
                datasource,
                objeto,
                columna,
            } => format!(
                "filtro obligatorio ausente · `{datasource}`·`{objeto}` exige que `{columna}` \
                 llegue filtrada, y este plan no la filtra"
            ),
            Rechazo::SinExpandir { vista } => {
                format!(
                    "el plan todavía nombra a `{vista}`: hay que expandirlo antes de repartirlo"
                )
            }
        }
    }
}

/// **El reparto.** `caps` es por nombre de fuente; una fuente que no aparezca no
/// declara nada, y no declarar nada es una negativa.
pub fn repartir(n: &Nodo, caps: &BTreeMap<String, Capacidades>) -> Result<Reparto, Rechazo> {
    let mut peticiones = Vec::new();
    let residuo = bajar(n, Vec::new(), caps, &mut peticiones)?;
    Ok(Reparto {
        peticiones,
        residuo,
    })
}

/// Los conyuntos de un predicado: `Y(a, Y(b, c))` son tres, no uno.
///
/// Aplanar es lo que permite bajar **parte** de una condición. Sin esto, un
/// `pais = 'ES' AND algo_raro(total)` no bajaría nada porque el conjunto entero
/// no es empujable, y se pediría la tabla entera por culpa de un trozo.
fn conyuntos(x: &Expr) -> Vec<Expr> {
    match x {
        Expr::Y(v) => v.iter().flat_map(conyuntos).collect(),
        otro => vec![otro.clone()],
    }
}

/// Vuelve a juntar lo que queda, o nada si no queda.
fn conjuncion(mut v: Vec<Expr>) -> Option<Expr> {
    match v.len() {
        0 => None,
        1 => Some(v.remove(0)),
        _ => Some(Expr::Y(v)),
    }
}

fn filtrando(entrada: Nodo, pendientes: Vec<Expr>) -> Nodo {
    match conjuncion(pendientes) {
        None => entrada,
        Some(p) => Nodo::Filtra {
            entrada: Box::new(entrada),
            predicado: p,
        },
    }
}

/// ¿Sabe el origen aplicar esta expresión entera?
fn empujable(x: &Expr, c: &Capacidades, campos: &BTreeSet<&str>) -> bool {
    match x {
        Expr::Campo(f) => campos.contains(f.as_str()),
        Expr::Literal(_) => true,
        Expr::Compara {
            op,
            izquierda,
            derecha,
        } => {
            c.predicados.contains(op)
                && empujable(izquierda, c, campos)
                && empujable(derecha, c, campos)
        }
        Expr::EnConjunto { campo, .. } => c.en_conjunto && campos.contains(campo.as_str()),
        Expr::EsNulo(e) => c.es_nulo && empujable(e, c, campos),
        Expr::Y(v) => v.iter().all(|e| empujable(e, c, campos)),
        Expr::O(v) => c.disyuncion && v.iter().all(|e| empujable(e, c, campos)),
        Expr::No(e) => c.negacion && empujable(e, c, campos),
        // Una opaca se puede empujar **si el origen habla ese dialecto**. Es lo
        // que la hace útil en vez de solo cara.
        Expr::Opaca(o) => {
            c.dialecto.as_deref() == Some(o.dialecto.as_str())
                && o.lee.iter().all(|f| campos.contains(f.as_str()))
        }
    }
}

/// Qué columnas de la hoja toca un predicado, para las comprobaciones de la
/// hoja.
fn toca(x: &Expr) -> BTreeSet<String> {
    x.campos_leidos()
}

fn bajar(
    n: &Nodo,
    pendientes: Vec<Expr>,
    caps: &BTreeMap<String, Capacidades>,
    out: &mut Vec<Peticion>,
) -> Result<Nodo, Rechazo> {
    Ok(match n {
        Nodo::Referencia(v) => return Err(Rechazo::SinExpandir { vista: v.clone() }),

        Nodo::Lee(l) => {
            let vacias = Capacidades::default();
            let c = caps.get(&l.datasource).unwrap_or(&vacias);
            let campos: BTreeSet<&str> = l.campos.keys().map(String::as_str).collect();

            let (empujados, resto): (Vec<Expr>, Vec<Expr>) = pendientes
                .into_iter()
                .partition(|x| empujable(x, c, &campos));

            // ── La puerta del recorrido completo ────────────────────────────
            if empujados.is_empty() && c.recorrido == Recorrido::Prohibido {
                return Err(Rechazo::RecorridoCompleto {
                    datasource: l.datasource.clone(),
                    objeto: l.objeto.clone(),
                    porque: if caps.contains_key(&l.datasource) {
                        "`fullScan: forbidden` es la negativa, y este plan no le baja \
                         ningún filtro"
                    } else {
                        "sin capacidades declaradas un origen sirve la búsqueda por clave y \
                         nada más"
                    },
                });
            }

            // ── Y los filtros que el origen exige ───────────────────────────
            let filtrados: BTreeSet<String> = empujados.iter().flat_map(toca).collect();
            for columna in &c.filtros_obligatorios {
                if !filtrados.contains(columna) {
                    return Err(Rechazo::FiltroObligatorioAusente {
                        datasource: l.datasource.clone(),
                        objeto: l.objeto.clone(),
                        columna: columna.clone(),
                    });
                }
            }

            out.push(Peticion {
                datasource: l.datasource.clone(),
                objeto: l.objeto.clone(),
                filtros: empujados,
            });
            filtrando(n.clone(), resto)
        }

        // Un filtro se disuelve en la bajada: sus conyuntos se suman a los que
        // ya bajaban, y lo que no llegue abajo se vuelve a poner arriba.
        Nodo::Filtra { entrada, predicado } => {
            let mut p = pendientes;
            p.extend(conyuntos(predicado));
            bajar(entrada, p, caps, out)?
        }

        // Por debajo de una proyección los nombres son otros. Un conyunto solo
        // baja si **cada** campo que nombra viene de una columna copiada tal
        // cual: si viniera de una expresión, bajarlo exigiría reescribirlo, y
        // reescribir predicados es donde un optimizador se equivoca en silencio.
        Nodo::Proyecta { entrada, campos } => {
            let mut abajo = Vec::new();
            let mut aqui = Vec::new();
            for x in pendientes {
                match traducir(&x, campos) {
                    Some(t) => abajo.push(t),
                    None => aqui.push(x),
                }
            }
            let dentro = bajar(entrada, abajo, caps, out)?;
            filtrando(
                Nodo::Proyecta {
                    entrada: Box::new(dentro),
                    campos: campos.clone(),
                },
                aqui,
            )
        }

        // Cada conyunto va al lado que tiene sus columnas. Los que cruzan los dos
        // se quedan arriba: bajarlos a un lado cambiaría el resultado.
        Nodo::Une {
            izquierda,
            derecha,
            tipo,
            sobre,
        } => {
            let (ci, cd) = (columnas(izquierda), columnas(derecha));
            let (mut a_i, mut a_d, mut aqui) = (Vec::new(), Vec::new(), Vec::new());
            for x in pendientes {
                let t = toca(&x);
                if t.iter().all(|f| ci.contains(f)) {
                    a_i.push(x);
                } else if t.iter().all(|f| cd.contains(f)) {
                    a_d.push(x);
                } else {
                    aqui.push(x);
                }
            }
            let i = bajar(izquierda, a_i, caps, out)?;
            let d = bajar(derecha, a_d, caps, out)?;
            filtrando(
                Nodo::Une {
                    izquierda: Box::new(i),
                    derecha: Box::new(d),
                    tipo: *tipo,
                    sobre: sobre.clone(),
                },
                aqui,
            )
        }

        // Solo bajan los que hablan de claves de grupo. Filtrar por un agregado
        // es un `HAVING`, y un `HAVING` por debajo del grupo no significa nada.
        Nodo::Agrupa {
            entrada,
            por,
            agregados,
        } => {
            let (mut abajo, mut aqui) = (Vec::new(), Vec::new());
            for x in pendientes {
                if toca(&x).iter().all(|f| por.contains(f)) {
                    abajo.push(x);
                } else {
                    aqui.push(x);
                }
            }
            let dentro = bajar(entrada, abajo, caps, out)?;
            filtrando(
                Nodo::Agrupa {
                    entrada: Box::new(dentro),
                    por: por.clone(),
                    agregados: agregados.clone(),
                },
                aqui,
            )
        }

        // Un filtro se reparte a todas las ramas de una unión: es la propiedad
        // distributiva, y es lo que evita traer entera la rama que no aporta.
        Nodo::Unifica(ramas) => Nodo::Unifica(
            ramas
                .iter()
                .map(|r| bajar(r, pendientes.clone(), caps, out))
                .collect::<Result<_, _>>()?,
        ),

        Nodo::Distingue(e) => Nodo::Distingue(Box::new(bajar(e, pendientes, caps, out)?)),

        // **La trampa.** Filtrar y luego limitar no es limitar y luego filtrar,
        // y el resultado de equivocarse es plausible: salen filas, menos de las
        // que debían. Nada baja de aquí.
        Nodo::Limita { entrada, n: k } => filtrando(
            Nodo::Limita {
                entrada: Box::new(bajar(entrada, Vec::new(), caps, out)?),
                n: *k,
            },
            pendientes,
        ),
    })
}

/// Reescribe un predicado en los nombres de **debajo** de una proyección.
///
/// `None` si alguno de sus campos no viene de una columna copiada tal cual.
fn traducir(x: &Expr, campos: &BTreeMap<String, Expr>) -> Option<Expr> {
    let de = |f: &str| -> Option<String> {
        match campos.get(f) {
            Some(Expr::Campo(c)) => Some(c.clone()),
            // Una columna que no está en la proyección no existe arriba, así que
            // un predicado que la nombre no puede haber llegado hasta aquí.
            _ => None,
        }
    };
    Some(match x {
        Expr::Campo(f) => Expr::Campo(de(f)?),
        Expr::Literal(v) => Expr::Literal(v.clone()),
        Expr::Compara {
            op,
            izquierda,
            derecha,
        } => Expr::Compara {
            op: *op,
            izquierda: Box::new(traducir(izquierda, campos)?),
            derecha: Box::new(traducir(derecha, campos)?),
        },
        Expr::EnConjunto { campo, valores } => Expr::EnConjunto {
            campo: de(campo)?,
            valores: valores.clone(),
        },
        Expr::EsNulo(e) => Expr::EsNulo(Box::new(traducir(e, campos)?)),
        Expr::Y(v) => Expr::Y(
            v.iter()
                .map(|e| traducir(e, campos))
                .collect::<Option<_>>()?,
        ),
        Expr::O(v) => Expr::O(
            v.iter()
                .map(|e| traducir(e, campos))
                .collect::<Option<_>>()?,
        ),
        Expr::No(e) => Expr::No(Box::new(traducir(e, campos)?)),
        // Una opaca no se reescribe: su texto nombra columnas por dentro y este
        // motor no lo lee. Traducir solo su `lee` dejaría el texto apuntando a
        // nombres que ya no existen.
        Expr::Opaca(_) => return None,
    })
}

/// Las columnas que un subplan produce. Se calcula sin tipar porque aquí solo
/// hacen falta los nombres, y tipar exigiría el esquema entero en cada nodo.
fn columnas(n: &Nodo) -> BTreeSet<String> {
    match n {
        Nodo::Referencia(_) => BTreeSet::new(),
        Nodo::Lee(l) => l.campos.keys().cloned().collect(),
        Nodo::Proyecta { campos, .. } => campos.keys().cloned().collect(),
        Nodo::Agrupa { por, agregados, .. } => por
            .iter()
            .cloned()
            .chain(agregados.keys().cloned())
            .collect(),
        Nodo::Une {
            izquierda, derecha, ..
        } => columnas(izquierda)
            .into_iter()
            .chain(columnas(derecha))
            .collect(),
        Nodo::Filtra { entrada, .. } | Nodo::Limita { entrada, .. } => columnas(entrada),
        Nodo::Distingue(e) => columnas(e),
        Nodo::Unifica(v) => v.first().map(columnas).unwrap_or_default(),
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Junta, Lectura, Valor};
    use ore_core::types::parse_type;

    fn hoja(ds: &str, objeto: &str, campos: &[&str]) -> Nodo {
        Nodo::Lee(Lectura {
            datasource: ds.into(),
            objeto: objeto.into(),
            campos: campos
                .iter()
                .map(|c| ((*c).to_string(), parse_type("String").unwrap()))
                .collect(),
        })
    }

    fn pedidos() -> Nodo {
        hoja("lago", "ventas.pedidos", &["id", "pais", "total"])
    }

    fn eq(campo: &str, v: &str) -> Expr {
        Expr::Compara {
            op: Comparador::Igual,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(Valor::Cadena(v.into()))),
        }
    }

    fn gt(campo: &str, v: &str) -> Expr {
        Expr::Compara {
            op: Comparador::Mayor,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(Valor::Cadena(v.into()))),
        }
    }

    /// Solo sabe `eq`, y admite recorrido completo para que la puerta no tape lo
    /// que se está midiendo.
    fn solo_eq() -> BTreeMap<String, Capacidades> {
        [(
            "lago".to_string(),
            Capacidades {
                predicados: [Comparador::Igual].into(),
                recorrido: Recorrido::Permitido,
                ..Default::default()
            },
        )]
        .into()
    }

    fn filtra(e: Nodo, p: Expr) -> Nodo {
        Nodo::Filtra {
            entrada: Box::new(e),
            predicado: p,
        }
    }

    /// **EL CRITERIO DE M4, mitad primera.** Se empuja el `eq` y el resto queda
    /// de residuo — **diciendo cuál es el residuo**.
    #[test]
    fn se_empuja_lo_que_sabe_hacer_y_el_resto_queda_dicho() {
        let p = filtra(
            pedidos(),
            Expr::Y(vec![eq("pais", "ES"), gt("total", "100")]),
        );
        let r = repartir(&p, &solo_eq()).expect("se reparte");

        assert_eq!(r.peticiones.len(), 1);
        assert_eq!(r.peticiones[0].datasource, "lago");
        assert_eq!(r.peticiones[0].filtros, vec![eq("pais", "ES")]);

        // Y el residuo es exactamente lo que la fuente no sabe hacer.
        let Nodo::Filtra { predicado, entrada } = &r.residuo else {
            panic!("{:?}", r.residuo);
        };
        assert_eq!(*predicado, gt("total", "100"));
        assert!(matches!(**entrada, Nodo::Lee(_)));
    }

    /// Aplanar la conjunción es lo que permite bajar **parte** de una
    /// condición. Sin ello, un trozo raro haría pedir la tabla entera.
    #[test]
    fn un_trozo_que_no_se_puede_empujar_no_arrastra_a_los_demas() {
        let p = filtra(
            pedidos(),
            Expr::Y(vec![
                eq("pais", "ES"),
                Expr::Y(vec![eq("id", "7"), gt("total", "0")]),
            ]),
        );
        let r = repartir(&p, &solo_eq()).expect("se reparte");
        assert_eq!(r.peticiones[0].filtros.len(), 2, "{:?}", r.peticiones[0]);
    }

    /// **EL CRITERIO DE M4, mitad segunda.** Sin filtros y sin recorrido
    /// completo, el plan se rechaza — y **sin abrir nada**, que es toda la
    /// gracia de declarar en vez de intentar.
    #[test]
    fn sin_filtros_y_sin_recorrido_completo_se_rechaza() {
        let caps: BTreeMap<String, Capacidades> = [(
            "lago".to_string(),
            Capacidades {
                predicados: [Comparador::Igual].into(),
                recorrido: Recorrido::Prohibido,
                ..Default::default()
            },
        )]
        .into();
        let r = repartir(&pedidos(), &caps).expect_err("sin claves no");
        let Rechazo::RecorridoCompleto { porque, .. } = &r else {
            panic!("{r:?}");
        };
        assert!(porque.contains("forbidden"), "{porque}");

        // Y con un filtro que sí baja, pasa.
        assert!(repartir(&filtra(pedidos(), eq("id", "7")), &caps).is_ok());
    }

    /// **La ausencia de capacidades es una negativa, no una laguna.** Es P4
    /// aplicada al reparto, y `05-ejecutor` §5.1 dicho en código.
    #[test]
    fn una_fuente_que_no_declara_nada_no_sirve_un_recorrido_completo() {
        let r = repartir(&pedidos(), &BTreeMap::new()).expect_err("sin declarar, no");
        let Rechazo::RecorridoCompleto { porque, .. } = &r else {
            panic!("{r:?}");
        };
        assert!(porque.contains("búsqueda por clave"), "{porque}");
    }

    /// Un origen que exige que una columna llegue filtrada lo dice antes, no
    /// después de que la petición falle.
    #[test]
    fn un_filtro_obligatorio_que_no_llega_se_dice_antes() {
        let caps: BTreeMap<String, Capacidades> = [(
            "lago".to_string(),
            Capacidades {
                predicados: [Comparador::Igual].into(),
                recorrido: Recorrido::Permitido,
                filtros_obligatorios: vec!["id".into()],
                ..Default::default()
            },
        )]
        .into();
        assert_eq!(
            repartir(&filtra(pedidos(), eq("pais", "ES")), &caps),
            Err(Rechazo::FiltroObligatorioAusente {
                datasource: "lago".into(),
                objeto: "ventas.pedidos".into(),
                columna: "id".into()
            })
        );
        // Y filtrando por ella, pasa.
        assert!(repartir(&filtra(pedidos(), eq("id", "7")), &caps).is_ok());
    }

    /// **LA TRAMPA.** Filtrar y luego limitar no es limitar y luego filtrar, y
    /// el resultado de equivocarse es plausible: salen filas, menos de las que
    /// debían. Nada baja por debajo de un límite.
    #[test]
    fn un_predicado_no_baja_por_debajo_de_un_limite() {
        let p = filtra(
            Nodo::Limita {
                entrada: Box::new(pedidos()),
                n: 10,
            },
            eq("pais", "ES"),
        );
        let mut caps = solo_eq();
        caps.get_mut("lago").unwrap().recorrido = Recorrido::Permitido;
        let r = repartir(&p, &caps).expect("se reparte");

        assert!(
            r.peticiones[0].filtros.is_empty(),
            "el filtro cruzó el límite: {:?}",
            r.peticiones[0]
        );
        // Y sigue arriba, donde tiene que estar.
        assert!(matches!(&r.residuo, Nodo::Filtra { .. }), "{:?}", r.residuo);
    }

    /// Por debajo de una proyección los nombres son otros: un predicado baja si
    /// **cada** campo que nombra viene de una columna copiada tal cual.
    #[test]
    fn un_predicado_se_traduce_al_bajar_por_una_proyeccion() {
        let p = filtra(
            Nodo::Proyecta {
                entrada: Box::new(pedidos()),
                campos: [("donde".to_string(), Expr::campo("pais"))].into(),
            },
            eq("donde", "ES"),
        );
        let r = repartir(&p, &solo_eq()).expect("se reparte");
        assert_eq!(
            r.peticiones[0].filtros,
            vec![eq("pais", "ES")],
            "no se tradujo al nombre de abajo"
        );
    }

    /// Y **no** baja si viene de una expresión: reescribir un predicado sobre
    /// algo computado es donde un optimizador se equivoca en silencio.
    #[test]
    fn un_predicado_sobre_una_columna_computada_no_baja() {
        let p = filtra(
            Nodo::Proyecta {
                entrada: Box::new(pedidos()),
                campos: [(
                    "hay".to_string(),
                    Expr::EsNulo(Box::new(Expr::campo("pais"))),
                )]
                .into(),
            },
            Expr::Compara {
                op: Comparador::Igual,
                izquierda: Box::new(Expr::campo("hay")),
                derecha: Box::new(Expr::Literal(Valor::Booleano(true))),
            },
        );
        let mut caps = solo_eq();
        caps.get_mut("lago").unwrap().recorrido = Recorrido::Permitido;
        let r = repartir(&p, &caps).expect("se reparte");
        assert!(r.peticiones[0].filtros.is_empty(), "{:?}", r.peticiones[0]);
        assert!(matches!(&r.residuo, Nodo::Filtra { .. }));
    }

    /// En una junta, cada conyunto va al lado que tiene sus columnas. Los que
    /// cruzan los dos se quedan arriba.
    #[test]
    fn en_una_junta_cada_predicado_va_a_su_lado() {
        let lineas = hoja("sap", "ventas.lineas", &["id_pedido", "sku"]);
        let caps: BTreeMap<String, Capacidades> = ["lago", "sap"]
            .into_iter()
            .map(|d| {
                (
                    d.to_string(),
                    Capacidades {
                        predicados: [Comparador::Igual].into(),
                        recorrido: Recorrido::Permitido,
                        ..Default::default()
                    },
                )
            })
            .collect();

        let p = filtra(
            Nodo::Une {
                izquierda: Box::new(pedidos()),
                derecha: Box::new(lineas),
                tipo: Junta::Interna,
                sobre: vec![("id".into(), "id_pedido".into())],
            },
            Expr::Y(vec![
                eq("pais", "ES"),
                eq("sku", "A-1"),
                // Este cruza los dos lados: se queda arriba.
                Expr::Compara {
                    op: Comparador::Igual,
                    izquierda: Box::new(Expr::campo("pais")),
                    derecha: Box::new(Expr::campo("sku")),
                },
            ]),
        );
        let r = repartir(&p, &caps).expect("se reparte");
        let de = |ds: &str| {
            r.peticiones
                .iter()
                .find(|x| x.datasource == ds)
                .expect("hay petición")
                .filtros
                .clone()
        };
        assert_eq!(de("lago"), vec![eq("pais", "ES")]);
        assert_eq!(de("sap"), vec![eq("sku", "A-1")]);
        // Y el que cruza sigue arriba.
        assert!(matches!(&r.residuo, Nodo::Filtra { .. }), "{:?}", r.residuo);
    }

    /// Un filtro se reparte a **todas** las ramas de una unión: es la propiedad
    /// distributiva, y es lo que evita traer entera la rama que no aporta.
    #[test]
    fn un_filtro_se_reparte_a_todas_las_ramas_de_una_union() {
        let p = filtra(
            Nodo::Unifica(vec![
                pedidos(),
                hoja("lago", "ventas.pedidos_viejos", &["id", "pais", "total"]),
            ]),
            eq("pais", "ES"),
        );
        let r = repartir(&p, &solo_eq()).expect("se reparte");
        assert_eq!(r.peticiones.len(), 2);
        assert!(
            r.peticiones
                .iter()
                .all(|x| x.filtros == vec![eq("pais", "ES")])
        );
    }

    /// Por debajo de un grupo solo bajan los predicados sobre claves de grupo.
    /// Filtrar por un agregado es un `HAVING`, y un `HAVING` por debajo del
    /// grupo no significa nada.
    #[test]
    fn un_having_no_baja_por_debajo_del_grupo() {
        use crate::plan::{Agregacion, Agregado};
        let agrupado = Nodo::Agrupa {
            entrada: Box::new(pedidos()),
            por: ["pais".to_string()].into(),
            agregados: [(
                "n".to_string(),
                Agregacion {
                    funcion: Agregado::Cuenta,
                    sobre: None,
                },
            )]
            .into(),
        };
        let p = filtra(
            agrupado,
            Expr::Y(vec![
                eq("pais", "ES"),
                // Sobre el agregado: se queda arriba.
                Expr::Compara {
                    op: Comparador::Igual,
                    izquierda: Box::new(Expr::campo("n")),
                    derecha: Box::new(Expr::Literal(Valor::Entero(3))),
                },
            ]),
        );
        let r = repartir(&p, &solo_eq()).expect("se reparte");
        assert_eq!(r.peticiones[0].filtros, vec![eq("pais", "ES")]);
        assert!(matches!(&r.residuo, Nodo::Filtra { .. }), "{:?}", r.residuo);
    }

    /// Una opaca se empuja **si el origen habla ese dialecto**. Es lo que la
    /// hace útil en vez de solo cara.
    #[test]
    fn una_opaca_baja_si_el_origen_habla_su_dialecto() {
        use crate::plan::Opaca;
        let opaca = |d: &str| {
            Expr::Opaca(Opaca {
                dialecto: d.into(),
                texto: "REGEXP_CONTAINS(pais, r'^E')".into(),
                lee: vec!["pais".into()],
                tipo: parse_type("Boolean").unwrap(),
                determinista: true,
            })
        };
        let caps = |d: Option<&str>| -> BTreeMap<String, Capacidades> {
            [(
                "lago".to_string(),
                Capacidades {
                    dialecto: d.map(String::from),
                    recorrido: Recorrido::Permitido,
                    ..Default::default()
                },
            )]
            .into()
        };

        let p = filtra(pedidos(), opaca("bigquery"));
        let r = repartir(&p, &caps(Some("bigquery"))).expect("se reparte");
        assert_eq!(r.peticiones[0].filtros.len(), 1, "{:?}", r.peticiones[0]);

        // Y con otro dialecto —o con ninguno— se queda arriba.
        for d in [Some("snowflake"), None] {
            let r = repartir(&p, &caps(d)).expect("se reparte");
            assert!(r.peticiones[0].filtros.is_empty(), "{d:?}");
        }
    }

    /// Y una opaca **no se traduce** al bajar por una proyección: su texto
    /// nombra columnas por dentro y este motor no lo lee, así que traducir solo
    /// su `lee` dejaría el texto apuntando a nombres que ya no existen.
    #[test]
    fn una_opaca_no_cruza_una_proyeccion_que_renombra() {
        use crate::plan::Opaca;
        let p = filtra(
            Nodo::Proyecta {
                entrada: Box::new(pedidos()),
                campos: [("donde".to_string(), Expr::campo("pais"))].into(),
            },
            Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "REGEXP_CONTAINS(donde, r'^E')".into(),
                lee: vec!["donde".into()],
                tipo: parse_type("Boolean").unwrap(),
                determinista: true,
            }),
        );
        let caps: BTreeMap<String, Capacidades> = [(
            "lago".to_string(),
            Capacidades {
                dialecto: Some("bigquery".into()),
                recorrido: Recorrido::Permitido,
                ..Default::default()
            },
        )]
        .into();
        let r = repartir(&p, &caps).expect("se reparte");
        assert!(r.peticiones[0].filtros.is_empty(), "{:?}", r.peticiones[0]);
    }

    /// Un plan sin expandir no se reparte, por lo mismo que no se tipa.
    #[test]
    fn un_plan_sin_expandir_no_se_reparte() {
        assert_eq!(
            repartir(&Nodo::Referencia("otra".into()), &BTreeMap::new()),
            Err(Rechazo::SinExpandir {
                vista: "otra".into()
            })
        );
    }

    /// El residuo **sigue siendo un plan**: se tipa, y produce lo mismo que el
    /// original. Si repartir cambiara el esquema, habría cambiado la respuesta.
    #[test]
    fn repartir_no_cambia_lo_que_el_plan_produce() {
        let p = filtra(pedidos(), Expr::Y(vec![eq("pais", "ES"), gt("total", "1")]));
        let r = repartir(&p, &solo_eq()).expect("se reparte");
        assert_eq!(
            crate::esquema(&r.residuo).expect("el residuo cuadra"),
            crate::esquema(&p).expect("el original cuadra")
        );
    }
}
