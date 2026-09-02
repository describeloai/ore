//! **Cost Model**: ¿sale más barato incrementar o recomputar?
//!
//! Databricks lo hace explícito: su `Enzyme` elige entre incremental y completo
//! **por coste**. Snowflake documenta la forma de la respuesta: lo incremental
//! gana cuando cambia **menos del 5 %** de la tabla base entre refrescos; por
//! encima, gana recomputar.
//!
//! > **Un motor que siempre incrementa es más lento que uno que sabe cuándo no
//! > hacerlo.**
//!
//! # Esta pieza no inventa ningún número
//!
//! Estuvo bloqueada por medidas, y sigue sin tener las nuestras. Lo que se
//! construye es la **forma** de la decisión, que sí está clara:
//!
//! - **entran medidas** —cuántas filas tiene la base y cuántas trae el delta—
//!   que **mide quien llama**, sobre los Z-sets que ya tiene;
//! - **entra una política declarada**, con sus números a la vista;
//! - **sale un dictamen auditable**: la decisión, **y todo lo que entró en ella**.
//!
//! El 5 % está aquí como [`Politica::documentada_por_snowflake`], **con su
//! procedencia y sin hacerlo nuestro**. Y [`Politica::sin_medir`] es lo que dice
//! ser: coeficientes de uno, para que la forma funcione hasta que haya medidas
//! que los sustituyan. Un `if` con un número escondido dentro de otra pieza es
//! exactamente lo que este módulo existe para que no ocurra.
//!
//! # Lo que sí se sabe sin medir
//!
//! Dos cosas, y las dos vienen de piezas anteriores:
//!
//! - **Si el Refresh Analyzer dice `FULL`, no hay decisión.** Se recomputa, y el
//!   dictamen dice por qué con los motivos de aquél.
//! - **El estado cuesta.** Un plan lineal —sin integradores— aplica un delta al
//!   precio del delta; uno con juntas y agregados toca sus integradores en cada
//!   paso. Los integradores los enumera el Delta Compiler, y entran en el coste.
//!
//! # Sin coma flotante
//!
//! Las razones se comparan como **racionales enteros**: `delta · den < base · num`,
//! en `u128`. Es la regla de M0 una vez más — y aquí además hace que dos
//! ejecuciones con las mismas medidas den el mismo dictamen byte a byte.

use crate::delta_compiler::Zset;
use crate::plan::Nodo;
use crate::refresh_analyzer::{RefreshMode, analizar};

/// Lo que mide quien llama. **Esta pieza no lo mide**: lo recibe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Medida {
    pub filas_base: u64,
    pub filas_delta: u64,
}

impl Medida {
    /// De los Z-sets que ya se tienen. Es la única forma honesta de medir aquí:
    /// contar lo que hay, no estimar lo que habrá.
    pub fn de(base: &Zset, delta: &Zset) -> Medida {
        Medida {
            filas_base: base.filas().count() as u64,
            filas_delta: delta.filas().count() as u64,
        }
    }
}

/// Cómo se decide. **Con los números a la vista.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Politica {
    /// Incremental si `delta / base < numerador / denominador`.
    Umbral { numerador: u64, denominador: u64 },
    /// Coste lineal: incremental cuesta `delta · por_fila_delta · (1 +
    /// integradores · por_integrador)`; recomputar cuesta `base ·
    /// por_fila_recomputo`. Gana el menor; en empate, incremental — también
    /// conserva la frescura.
    Coeficientes {
        por_fila_delta: u64,
        por_fila_recomputo: u64,
        por_integrador: u64,
    },
}

impl Politica {
    /// **La cifra de Snowflake, no la nuestra.** Documentan que lo incremental
    /// gana cuando cambia menos del 5 % de la base. Se ofrece con su procedencia
    /// para que nadie la tome por medida propia.
    pub fn documentada_por_snowflake() -> Politica {
        Politica::Umbral {
            numerador: 5,
            denominador: 100,
        }
    }

    /// Coeficientes de **uno**. Es lo que dice ser: la forma de la decisión
    /// funcionando hasta que haya medidas que la calibren.
    pub fn sin_medir() -> Politica {
        Politica::Coeficientes {
            por_fila_delta: 1,
            por_fila_recomputo: 1,
            por_integrador: 1,
        }
    }

    fn como_texto(&self) -> String {
        match self {
            Politica::Umbral {
                numerador,
                denominador,
            } => format!("umbral {numerador}/{denominador}"),
            Politica::Coeficientes {
                por_fila_delta,
                por_fila_recomputo,
                por_integrador,
            } => format!(
                "coeficientes delta={por_fila_delta} recómputo={por_fila_recomputo} \
                 integrador={por_integrador}"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Incremental,
    Recomputar,
}

/// La decisión **y todo lo que entró en ella**. Un dictamen que solo dijera
/// «incremental» no se podría discutir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictamen {
    pub decision: Decision,
    pub porque: String,
    pub medida: Medida,
    pub politica: Politica,
    pub integradores: usize,
    pub modo: RefreshMode,
}

impl Dictamen {
    pub fn como_texto(&self, vista: &str) -> String {
        let d = match self.decision {
            Decision::Incremental => "INCREMENTAL",
            Decision::Recomputar => "RECOMPUTAR",
        };
        format!(
            "{vista}  →  {d}\n  porque: {}\n  medida: base={} delta={} integradores={}\n  \
             política: {}\n",
            self.porque,
            self.medida.filas_base,
            self.medida.filas_delta,
            self.integradores,
            self.politica.como_texto()
        )
    }
}

/// **El dictamen.**
pub fn decidir(plan: &Nodo, medida: Medida, politica: &Politica) -> Dictamen {
    let modo = analizar(plan);
    let integradores = match &modo {
        RefreshMode::Incremental { state } => state.len(),
        RefreshMode::Full { .. } => 0,
    };
    let (decision, porque) = match &modo {
        // Sin regla no hay decisión: se recomputa, y se dice por qué.
        RefreshMode::Full { porque } => (
            Decision::Recomputar,
            format!(
                "no se puede mantener incrementalmente: {}",
                porque
                    .iter()
                    .map(|m| m.como_texto())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ),
        // Sin base, aplicar el delta ES el recómputo.
        RefreshMode::Incremental { .. } if medida.filas_base == 0 => (
            Decision::Incremental,
            "la base está vacía: aplicar el delta es recomputar".into(),
        ),
        RefreshMode::Incremental { .. } => match politica {
            Politica::Umbral {
                numerador,
                denominador,
            } => {
                let izq = u128::from(medida.filas_delta) * u128::from(*denominador);
                let der = u128::from(medida.filas_base) * u128::from(*numerador);
                if izq < der {
                    (
                        Decision::Incremental,
                        format!(
                            "cambia {}/{} de la base, por debajo de {numerador}/{denominador}",
                            medida.filas_delta, medida.filas_base
                        ),
                    )
                } else {
                    (
                        Decision::Recomputar,
                        format!(
                            "cambia {}/{} de la base, no por debajo de {numerador}/{denominador}",
                            medida.filas_delta, medida.filas_base
                        ),
                    )
                }
            }
            Politica::Coeficientes {
                por_fila_delta,
                por_fila_recomputo,
                por_integrador,
            } => {
                let incremental = u128::from(medida.filas_delta)
                    * u128::from(*por_fila_delta)
                    * (1 + integradores as u128 * u128::from(*por_integrador));
                let recomputar = u128::from(medida.filas_base) * u128::from(*por_fila_recomputo);
                if incremental <= recomputar {
                    (
                        Decision::Incremental,
                        format!("incrementar cuesta {incremental} y recomputar {recomputar}"),
                    )
                } else {
                    (
                        Decision::Recomputar,
                        format!("incrementar cuesta {incremental} y recomputar {recomputar}"),
                    )
                }
            }
        },
    };
    Dictamen {
        decision,
        porque,
        medida,
        politica: politica.clone(),
        integradores,
        modo,
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Agregacion, Agregado, Expr, Junta, Lectura, Valor};
    use ore_core::types::parse_type;
    use std::collections::BTreeMap;

    fn pedidos() -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: "ventas.pedidos".into(),
            campos: [
                ("id".to_string(), parse_type("Integer").unwrap()),
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
    fn lineal() -> Nodo {
        Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("id".to_string(), Expr::campo("id"))].into(),
        }
    }
    fn junta() -> Nodo {
        Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(lineas()),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "id_pedido".into())],
        }
    }
    fn m(base: u64, delta: u64) -> Medida {
        Medida {
            filas_base: base,
            filas_delta: delta,
        }
    }

    /// **Sin regla no hay decisión.** Un plan `FULL` se recomputa sea cual sea
    /// la medida, y el dictamen dice por qué con los motivos del Refresh
    /// Analyzer.
    #[test]
    fn un_plan_full_se_recomputa_diga_lo_que_diga_la_medida() {
        let full = Nodo::Agrupa {
            entrada: Box::new(pedidos()),
            por: BTreeMap::<String, ()>::new().into_keys().collect(),
            agregados: [(
                "media".to_string(),
                Agregacion {
                    funcion: Agregado::Promedio,
                    sobre: Some("total".into()),
                },
            )]
            .into(),
        };
        let d = decidir(
            &full,
            m(1_000_000, 1),
            &Politica::documentada_por_snowflake(),
        );
        assert_eq!(d.decision, Decision::Recomputar);
        assert!(d.porque.contains("división"), "{}", d.porque);
    }

    /// **El umbral, con la cifra de Snowflake y su procedencia.** 4 de 100 está
    /// por debajo de 5/100; 5 de 100 no lo está. Racionales enteros, sin coma.
    #[test]
    fn el_umbral_compara_racionales_sin_coma_flotante() {
        let p = Politica::documentada_por_snowflake();
        assert_eq!(
            decidir(&lineal(), m(100, 4), &p).decision,
            Decision::Incremental
        );
        assert_eq!(
            decidir(&lineal(), m(100, 5), &p).decision,
            Decision::Recomputar
        );
        // Y no se desborda con bases grandes: u128 por dentro.
        assert_eq!(
            decidir(&lineal(), m(u64::MAX, 1), &p).decision,
            Decision::Incremental
        );
    }

    /// **El estado cuesta.** Con los mismos coeficientes y la misma medida, un
    /// plan lineal incrementa y una junta —dos integradores— recomputa.
    #[test]
    fn el_estado_entra_en_el_coste() {
        let p = Politica::sin_medir();
        let l = decidir(&lineal(), m(100, 40), &p);
        assert_eq!(l.integradores, 0);
        assert_eq!(l.decision, Decision::Incremental, "{}", l.porque);

        let j = decidir(&junta(), m(100, 40), &p);
        assert_eq!(j.integradores, 2);
        // 40 · 1 · (1 + 2) = 120 > 100.
        assert_eq!(j.decision, Decision::Recomputar, "{}", j.porque);

        // Y con menos delta, la misma junta incrementa: 30 · 3 = 90 ≤ 100.
        assert_eq!(
            decidir(&junta(), m(100, 30), &p).decision,
            Decision::Incremental
        );
    }

    /// Sin base, aplicar el delta ES el recómputo: incremental, y se dice.
    #[test]
    fn sin_base_se_incrementa() {
        let d = decidir(&junta(), m(0, 10), &Politica::sin_medir());
        assert_eq!(d.decision, Decision::Incremental);
        assert!(d.porque.contains("vacía"), "{}", d.porque);
    }

    /// **Auditable.** El dictamen lleva todo lo que entró: la medida, la política
    /// con sus números, los integradores y el modo. Y dos ejecuciones con lo
    /// mismo dan lo mismo.
    #[test]
    fn el_dictamen_lleva_todo_lo_que_entro_y_es_determinista() {
        let p = Politica::documentada_por_snowflake();
        let a = decidir(&junta(), m(100, 3), &p);
        let b = decidir(&junta(), m(100, 3), &p);
        assert_eq!(a, b);
        let t = a.como_texto("ventas.pedidos_con_lineas");
        assert!(t.contains("INCREMENTAL"), "{t}");
        assert!(t.contains("base=100 delta=3 integradores=2"), "{t}");
        assert!(t.contains("umbral 5/100"), "{t}");
        assert!(t.contains("3/100"), "{t}");
    }

    /// La medida sale de los Z-sets que ya se tienen: contar, no estimar.
    #[test]
    fn la_medida_se_cuenta_de_los_zsets() {
        let fila = |i: i64| {
            [("id".to_string(), Valor::Entero(i))]
                .into_iter()
                .collect::<BTreeMap<_, _>>()
        };
        let base = Zset::de((0..10).map(|i| (fila(i), 1)));
        let delta = Zset::de([(fila(3), -1), (fila(11), 1)]);
        assert_eq!(Medida::de(&base, &delta), m(10, 2));
    }
}
