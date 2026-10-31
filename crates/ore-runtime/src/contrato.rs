//! **El contrato: ORE SQL** (0081 §7.5). El lenguaje es el de DataFusion, adoptado entero. ORE sólo fija lo que
//! DataFusion deja en blanco o elige distinto de lo que el lago guarda, y lo publica aquí:
//!
//! - los ajustes del contrato, que van en [`crate::opciones::DESVIOS`] con su porqué (`time_zone = UTC`,
//!   `information_schema = true`);
//! - [`TiposDeOre`], el planificador de tipos: el tiempo del lago es en microsegundos;
//! - [`VERSION`], que sale en el manifiesto.
//!
//! La vara de ORE (`tests/contrato/*.slt`) es su definición ejecutable.

use datafusion::arrow::datatypes::{DataType, Field, FieldRef, TimeUnit};
use datafusion::common::Result;
use datafusion::logical_expr::planner::TypePlanner;
use datafusion::sql::sqlparser::ast;
use std::sync::Arc;

/// La versión del contrato: un major de DataFusion más la capa de ORE (0081 §7.5.2).
pub const VERSION: &str = "ORE SQL v1";

/// La zona de `TIMESTAMPTZ`: la del lago (0081 §7.5.3, decisión 4).
pub const ZONA: &str = "UTC";

/// **Los tipos SQL que ORE planifica distinto de DataFusion** (0081 §7.5.3, decisión 4). DataFusion pone el
/// tiempo sin precisión en nanosegundos; el lago (Iceberg) lo guarda en microsegundos. Así, `TIMESTAMP`,
/// `TIMESTAMPTZ` y `TIME` sin precisión son ya los del lago en DDL, `CAST` y literales tipados. Con precisión
/// explícita (`TIMESTAMP(9)`) manda quien la escribe, y el resto de tipos son los de DataFusion.
#[derive(Debug, Default)]
pub struct TiposDeOre;

impl TypePlanner for TiposDeOre {
    fn plan_type_field(&self, tipo: &ast::DataType) -> Result<Option<FieldRef>> {
        use ast::TimezoneInfo as Zona;
        let t = match tipo {
            ast::DataType::Timestamp(None, Zona::None | Zona::WithoutTimeZone) => {
                DataType::Timestamp(TimeUnit::Microsecond, None)
            }
            ast::DataType::Timestamp(None, Zona::WithTimeZone | Zona::Tz) => {
                DataType::Timestamp(TimeUnit::Microsecond, Some(ZONA.into()))
            }
            ast::DataType::Time(None, Zona::None | Zona::WithoutTimeZone) => {
                DataType::Time64(TimeUnit::Microsecond)
            }
            _ => return Ok(None),
        };
        Ok(Some(Arc::new(Field::new("", t, true))))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::limites::{Fuente, Limites};
    use crate::motor::Motor;
    use crate::opciones::{OreOpciones, Papel};
    use crate::sesion::Contexto;
    use crate::zocalos::Zocalos;

    fn motor(opciones: OreOpciones) -> Motor {
        let limites = Limites {
            memoria: 1 << 30,
            memoria_de: Fuente::Host,
            cpu: 2,
            cpu_de: Fuente::Host,
            disco: None,
        };
        Motor::arrancar_con(Papel::Prueba, opciones, &Zocalos::default(), limites).unwrap()
    }

    /// Los tipos que una consulta planifica, columna a columna.
    fn tipos(m: &Motor, sql: &str) -> Vec<DataType> {
        let mut s = m.sesion(&Contexto::nuevo("tipos")).unwrap();
        let plan = m.es().block_on(s.planificar(sql)).unwrap();
        plan.schema()
            .fields()
            .iter()
            .map(|f| f.data_type().clone())
            .collect()
    }

    const TIEMPOS: &str = "select cast('2020-01-01' as timestamp) a, cast('2020-01-01' as timestamptz) b, \
                           cast('10:00' as time) c, timestamp '2020-01-01 10:00:00' d, \
                           cast('2020-01-01' as timestamp(9)) e, now() f";

    /// El tiempo sin precisión es el del lago, local o como cliente de un clúster: `upgrade_for_ballista`
    /// rehace el estado y el planificador de tipos tiene que sobrevivirle.
    #[test]
    fn el_tiempo_es_el_del_lago_en_local_y_en_el_cliente() {
        let utc = Some(Arc::from(ZONA));
        let esperado = vec![
            DataType::Timestamp(TimeUnit::Microsecond, None),
            DataType::Timestamp(TimeUnit::Microsecond, utc.clone()),
            DataType::Time64(TimeUnit::Microsecond),
            DataType::Timestamp(TimeUnit::Microsecond, None),
            // Con precisión, manda quien la escribe.
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            // `now()` es de DataFusion: nanosegundos, en la zona del contrato.
            DataType::Timestamp(TimeUnit::Nanosecond, utc),
        ];
        assert_eq!(tipos(&motor(OreOpciones::default()), TIEMPOS), esperado);
        let cliente = OreOpciones {
            scheduler: Some("df://scheduler:50050".into()),
            ..OreOpciones::default()
        };
        assert_eq!(tipos(&motor(cliente), TIEMPOS), esperado);
    }
}
