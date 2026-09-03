//! **Refresh Analyzer**: `INCREMENTAL` o `FULL`. Y si es `FULL`, **por qué**.
//!
//! El vocabulario es el de Snowflake —`REFRESH_MODE` es literalmente la columna
//! de su catálogo— y la pieza hace lo que su motor hace, **un paso antes**.
//! Snowflake descubre que una vista no se puede mantener **al refrescarla**: si
//! el `SELECT` tiene una UDF volátil, un `INTERSECT`, un `PERCENTILE_CONT`, cae a
//! `FULL` y uno se entera por la factura. Aquí se sabe **antes de escribir la
//! vista**, y con la lista entera de motivos, no el primero.
//!
//! # Vale por sí sola
//!
//! No necesita ni el Partial State Store ni el Cost Model. Un motor que dice
//! *«esta vista no se mantiene porque tiene un `PROMEDIO` y una opaca sin
//! declaración de pureza»* es un motor con el que se **diseñan** vistas
//! mantenibles, en vez de descubrirlo tarde. Es la pieza más subestimada de M6.
//!
//! # Una regla, escrita una vez
//!
//! Los motivos de `FULL` son **exactamente** los refusals del Delta Compiler:
//! [`crate::delta_compiler::motivos_con`] los recoge todos, y
//! `Circuito::compilar_con` consulta la misma función antes de construir nada.
//! Si esta pieza dijera `INCREMENTAL` y el compilador refusara —o al revés—
//! habría dos definiciones de «mantenible», y la primera vez que alguien las
//! comparara tendría razón en desconfiar. Hay una prueba que las compara.
//!
//! La invariante vale **para el mismo conocimiento de los orígenes**, y por eso
//! los dos tienen su `_con`: saber que una hoja no emite cambios tiene que mover
//! a los dos o a ninguno.
//!
//! # Dos preguntas, y hacían falta las dos
//!
//! *¿Se puede incrementalizar este plan?* es una pregunta sobre el **álgebra**.
//! *¿Se va a mantener esta vista?* es además una pregunta sobre el **origen**:
//! un plan lineal sobre una tabla que declara `changes: { mode: none }` es
//! impecable como álgebra y no se va a refrescar nunca, porque no va a llegar
//! ningún Δ. Esta pieza contestaba solo la primera y decía `INCREMENTAL` de la
//! segunda; el compilador de OOS ya lo sabía y el motor no se lo preguntaba.
//!
//! # Lo que se corrigió al construirla
//!
//! El plano decía que una vista con un `MIN` saldría como `FULL`. **No**: el
//! Delta Compiler la mantiene, recomputando el grupo que el Δ toca — lo que
//! `MIN` y `MAX` cuestan es **estado**, el multiconjunto del grupo, porque no
//! son invertibles bajo baja. Salen `INCREMENTAL`, y el estado lo dice. Lo que
//! sí es `FULL` es lo que no tiene regla: `Limita`, `PROMEDIO`, una opaca
//! volátil, una junta externa. Lo medido manda sobre lo planeado.

use crate::delta_compiler::{Circuito, Emite, Estado, NoIncrementalizable, motivos_con};
use crate::plan::Nodo;
use std::collections::BTreeMap;

/// Cómo se refresca una vista, y qué cuesta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshMode {
    /// Se mantiene con deltas. `state` es lo que un almacén tendría que
    /// sostener — vacío para un plan lineal, que es lo que significa que
    /// mantenerlo no cuesta nada.
    Incremental { state: Vec<Estado> },
    /// Hay que recomputar entero, y estos son **todos** los motivos.
    Full { porque: Vec<NoIncrementalizable> },
}

impl RefreshMode {
    pub fn es_incremental(&self) -> bool {
        matches!(self, RefreshMode::Incremental { .. })
    }

    /// El informe, con el nombre de la vista delante.
    pub fn como_texto(&self, vista: &str) -> String {
        match self {
            RefreshMode::Incremental { state } => {
                let mut s = format!("{vista}  →  REFRESH_MODE = INCREMENTAL\n");
                if state.is_empty() {
                    s.push_str("  estado: ninguno\n");
                } else {
                    s.push_str("  estado:\n");
                    for e in state {
                        s.push_str(&format!("    {} · {}\n", e.operador, e.guarda));
                    }
                }
                s
            }
            RefreshMode::Full { porque } => {
                let mut s = format!("{vista}  →  REFRESH_MODE = FULL\n");
                for m in porque {
                    s.push_str(&format!("  ← {}\n", m.como_texto()));
                }
                s
            }
        }
    }
}

/// **El análisis.** Todos los motivos, o el estado.
pub fn analizar(plan: &Nodo) -> RefreshMode {
    analizar_con(plan, &BTreeMap::new())
}

/// Lo mismo, **mirando además la cara `D`**: qué cambios emite cada hoja.
///
/// Sin esto, esta pieza decidía por la forma del plan y nada más — y diría
/// `INCREMENTAL` de una vista sobre una tabla que declara `changes: { mode:
/// none }`. El compilador de OOS ya lo sabía; el motor no se lo preguntaba, y
/// esa es exactamente la clase de discrepancia que hace desconfiar de las dos
/// respuestas.
///
/// Una hoja ausente del mapa **no se supone que no emita**: se supone que no se
/// declaró. Por eso `analizar` sigue contestando lo de siempre.
pub fn analizar_con(plan: &Nodo, emite: &BTreeMap<(String, String), Emite>) -> RefreshMode {
    let porque = motivos_con(plan, emite);
    if !porque.is_empty() {
        return RefreshMode::Full { porque };
    }
    // Sin motivos, el compilador no puede refusar: consulta la misma lista. Si
    // lo hiciera, sería un defecto de esta pieza y se diría como tal.
    match Circuito::compilar_con(plan, emite) {
        Ok(c) => RefreshMode::Incremental { state: c.estado() },
        Err(m) => RefreshMode::Full { porque: vec![m] },
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Agregacion, Agregado, Comparador, Expr, Junta, Lectura, Opaca, Valor};
    use ore_core::types::parse_type;

    fn pedidos() -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: "ventas.pedidos".into(),
            campos: [
                ("id".to_string(), parse_type("Integer").unwrap()),
                ("pais".to_string(), parse_type("String").unwrap()),
                ("total".to_string(), parse_type("Decimal").unwrap()),
            ]
            .into(),
        })
    }
    fn lineas() -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "sap".into(),
            objeto: "ventas.lineas".into(),
            campos: [("id_pedido".to_string(), parse_type("Integer").unwrap())].into(),
        })
    }
    fn agrupa(e: Nodo, ag: &[(&str, Agregado, Option<&str>)]) -> Nodo {
        Nodo::Agrupa {
            entrada: Box::new(e),
            por: ["pais".to_string()].into(),
            agregados: ag
                .iter()
                .map(|(n, f, s)| {
                    (
                        (*n).to_string(),
                        Agregacion {
                            funcion: *f,
                            sobre: s.map(String::from),
                        },
                    )
                })
                .collect(),
        }
    }
    fn opaca(dialecto: &str, determinista: bool) -> Expr {
        Expr::Opaca(Opaca {
            dialecto: dialecto.into(),
            texto: "algo(pais)".into(),
            lee: vec!["pais".into()],
            tipo: parse_type("Boolean").unwrap(),
            determinista,
        })
    }

    /// **El criterio, mitad primera.** Solo proyecciones y filtros: `INCREMENTAL`
    /// **con estado cero**, y el informe lo dice con esas palabras.
    #[test]
    fn solo_proyecciones_y_filtros_es_incremental_con_estado_cero() {
        let plan = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: Expr::Compara {
                    op: Comparador::Igual,
                    izquierda: Box::new(Expr::campo("pais")),
                    derecha: Box::new(Expr::Literal(Valor::Cadena("ES".into()))),
                },
            }),
            campos: [("id".to_string(), Expr::campo("id"))].into(),
        };
        let r = analizar(&plan);
        assert_eq!(r, RefreshMode::Incremental { state: Vec::new() });
        let t = r.como_texto("ventas.es");
        assert!(t.contains("REFRESH_MODE = INCREMENTAL"), "{t}");
        assert!(t.contains("estado: ninguno"), "{t}");
    }

    /// **El criterio, mitad segunda.** Un `PROMEDIO` y una opaca volátil: `FULL`
    /// **nombrando los dos**. Snowflake solo te diría el primero, y al refrescar.
    #[test]
    fn un_promedio_y_una_opaca_volatil_salen_full_nombrando_los_dos() {
        let plan = Nodo::Filtra {
            entrada: Box::new(agrupa(
                pedidos(),
                &[("media", Agregado::Promedio, Some("total"))],
            )),
            predicado: opaca("bigquery", false),
        };
        let RefreshMode::Full { porque } = analizar(&plan) else {
            panic!("tenía que ser FULL");
        };
        assert_eq!(porque.len(), 2, "{porque:?}");
        assert!(porque.contains(&NoIncrementalizable::Promedio {
            nombre: "media".into()
        }));
        assert!(porque.contains(&NoIncrementalizable::OpacaVolatil {
            dialecto: "bigquery".into()
        }));
        let t = analizar(&plan).como_texto("ventas.resumen");
        assert!(t.contains("REFRESH_MODE = FULL"), "{t}");
        assert!(t.contains("división") && t.contains("determinista"), "{t}");
    }

    /// **La corrección.** El plano decía que un `MIN` sería `FULL`. No: se
    /// mantiene, y lo que cuesta es **estado** — el multiconjunto del grupo—, y
    /// el informe lo dice.
    #[test]
    fn un_min_es_incremental_y_cuesta_el_multiconjunto() {
        let plan = agrupa(
            pedidos(),
            &[
                ("menor", Agregado::Minimo, Some("total")),
                ("suma", Agregado::Suma, Some("total")),
            ],
        );
        let r = analizar(&plan);
        let RefreshMode::Incremental { state } = &r else {
            panic!("{r:?}");
        };
        assert!(
            state.iter().any(|e| e.guarda.contains("multiconjunto")),
            "{state:?}"
        );
        assert!(
            state.iter().any(|e| e.guarda.contains("acumulador")),
            "{state:?}"
        );
        let t = r.como_texto("ventas.minimos");
        assert!(t.contains("multiconjunto"), "{t}");
    }

    /// **Una regla, escrita una vez.** Lo que esta pieza llama `INCREMENTAL` es
    /// exactamente lo que el Delta Compiler compila, y al revés. Se comprueba
    /// sobre planes de todas las formas, con y sin motivo.
    #[test]
    fn dice_incremental_exactamente_cuando_el_compilador_compila() {
        let planes = vec![
            pedidos(),
            Nodo::Limita {
                entrada: Box::new(pedidos()),
                n: 3,
            },
            agrupa(pedidos(), &[("media", Agregado::Promedio, Some("total"))]),
            agrupa(pedidos(), &[("n", Agregado::Cuenta, None)]),
            Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: opaca("bigquery", false),
            },
            Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: opaca("bigquery", true),
            },
            Nodo::Une {
                izquierda: Box::new(pedidos()),
                derecha: Box::new(lineas()),
                tipo: Junta::Izquierda,
                sobre: vec![("id".into(), "id_pedido".into())],
            },
            Nodo::Une {
                izquierda: Box::new(pedidos()),
                derecha: Box::new(lineas()),
                tipo: Junta::Interna,
                sobre: vec![("id".into(), "id_pedido".into())],
            },
            Nodo::Distingue(Box::new(pedidos())),
            Nodo::Referencia("otra".into()),
        ];
        // Y con la cara `D` puesta: la invariante vale **para el mismo
        // conocimiento de los orígenes**, que es lo que la hace comprobable. Sin
        // los dos `_con`, saber que una hoja no emite haría decir `FULL` a esta
        // pieza de un plan que el compilador construye sin protestar, y habría
        // otra vez dos definiciones de «mantenible».
        let ninguno: BTreeMap<(String, String), Emite> = [(
            ("lago".to_string(), "ventas.pedidos".to_string()),
            Emite::Nada,
        )]
        .into();
        for mapa in [&BTreeMap::new(), &ninguno] {
            for p in &planes {
                assert_eq!(
                    analizar_con(p, mapa).es_incremental(),
                    Circuito::compilar_con(p, mapa).is_ok(),
                    "discrepan sobre {p:?}"
                );
            }
        }
    }

    /// **La cara `D`, que esta pieza no miraba.**
    ///
    /// Un plan lineal sobre una tabla que declara `changes: { mode: none }` es
    /// perfectamente incrementalizable *como álgebra* — y aun así no se va a
    /// mantener nunca, porque no va a llegar ningún Δ. Decirlo `INCREMENTAL`
    /// sería prometer un refresco que no ocurre; el compilador de OOS ya lo
    /// sabía y el motor no se lo preguntaba.
    ///
    /// Y una hoja **ausente** del mapa sigue siendo `INCREMENTAL`: ausencia es
    /// *no se declaró*, no *no emite*. Sin declaración no se supone ninguna.
    #[test]
    fn una_hoja_que_no_emite_cambios_no_se_mantiene_aunque_el_algebra_pueda() {
        let plan = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Compara {
                op: Comparador::Igual,
                izquierda: Box::new(Expr::campo("pais")),
                derecha: Box::new(Expr::Literal(Valor::Cadena("ES".into()))),
            },
        };
        assert!(
            analizar(&plan).es_incremental(),
            "sin declarar, lo de siempre"
        );

        let hoja = ("lago".to_string(), "ventas.pedidos".to_string());
        for (e, incremental) in [
            (Emite::Nada, false),
            (Emite::Altas, true),
            (Emite::Retracciones, true),
            (Emite::Upserts, true),
        ] {
            let r = analizar_con(&plan, &[(hoja.clone(), e)].into());
            assert_eq!(r.es_incremental(), incremental, "con {e:?}");
        }

        let t = analizar_con(&plan, &[(hoja, Emite::Nada)].into()).como_texto("ventas.es");
        assert!(t.contains("REFRESH_MODE = FULL"), "{t}");
        assert!(t.contains("declara no emitir cambios"), "{t}");
    }

    /// Dos opacas volátiles del **mismo** dialecto son un motivo; de dialectos
    /// distintos, dos. Repetir el mismo defecto veinte veces entierra los otros
    /// diecinueve.
    #[test]
    fn el_mismo_motivo_no_se_repite() {
        let dos_iguales = Nodo::Filtra {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: opaca("bigquery", false),
            }),
            predicado: opaca("bigquery", false),
        };
        let RefreshMode::Full { porque } = analizar(&dos_iguales) else {
            panic!()
        };
        assert_eq!(porque.len(), 1, "{porque:?}");

        let dos_distintas = Nodo::Filtra {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: opaca("bigquery", false),
            }),
            predicado: opaca("snowflake", false),
        };
        let RefreshMode::Full { porque } = analizar(&dos_distintas) else {
            panic!()
        };
        assert_eq!(porque.len(), 2, "{porque:?}");
    }

    /// Y los motivos se recogen **por todo el árbol**, no hasta el primero: un
    /// `Limita` arriba y un `PROMEDIO` abajo salen los dos.
    #[test]
    fn los_motivos_se_recogen_por_todo_el_arbol() {
        let plan = Nodo::Limita {
            entrada: Box::new(agrupa(
                pedidos(),
                &[("media", Agregado::Promedio, Some("total"))],
            )),
            n: 10,
        };
        let RefreshMode::Full { porque } = analizar(&plan) else {
            panic!()
        };
        assert_eq!(porque.len(), 2, "{porque:?}");
        assert!(porque.contains(&NoIncrementalizable::Limita));
    }
}
