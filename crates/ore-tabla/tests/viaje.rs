//! **El viaje en el tiempo** (0081 §7.5.3, decisión 3; C9) con ids e instantes de verdad: cada escritura deja un
//! snapshot en los metadatos de la tabla, y `at(…)` lee cualquiera de ellos con su esquema de entonces.

use datafusion::arrow::array::{ArrayRef, Int64Array};
use datafusion::arrow::record_batch::RecordBatch;
use ore_runtime::{Contexto, Motor, OreOpciones, Papel, Zocalos};
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::{CatalogoOre, Nombre};
use std::sync::Arc;

fn ids(n: i64) -> Vec<RecordBatch> {
    let a: ArrayRef = Arc::new(Int64Array::from((1..=n).collect::<Vec<_>>()));
    vec![RecordBatch::try_from_iter_with_nullable([("id", a, true)]).unwrap()]
}

fn un_entero(m: &Motor, sql: &str) -> Result<i64, String> {
    let mut s = m.sesion(&Contexto::nuevo("q").en_base("v")).unwrap();
    let lotes = m
        .es()
        .block_on(async {
            let plan = s.planificar(sql).await?.logico()?;
            m.correr(&s, plan)?.recoger().await
        })
        .map_err(|e| e.to_string())?;
    Ok(lotes[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0))
}

#[test]
fn at_lee_cada_snapshot_y_cada_momento() {
    let lago = LagoEnMemoria::default();
    let a = Nombre::new("v", "default", "a");
    // Tres versiones: 1, 2 y 3 filas, cada una su snapshot.
    let mut snapshots = Vec::new();
    for n in 1..=3 {
        let ml = lago.escribir(&a, ids(n)).unwrap();
        let tabla = lago.lago().abrir(&ml, "a").unwrap();
        let s = tabla.metadata().current_snapshot().unwrap();
        snapshots.push((s.snapshot_id(), s.timestamp_ms()));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let z = Zocalos {
        catalogo: Arc::new(CatalogoOre::new(lago.fuente(), lago.lago().clone())),
        ..Zocalos::default()
    };
    let m = Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).unwrap();
    for (i, (id, ms)) in snapshots.iter().enumerate() {
        let n = i as i64 + 1;
        assert_eq!(
            un_entero(
                &m,
                &format!("select count(*) from at('a', snapshot => {id})")
            )
            .unwrap(),
            n
        );
        let instante = datafusion::arrow::temporal_conversions::timestamp_ms_to_datetime(*ms)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        assert_eq!(
            un_entero(
                &m,
                &format!("select count(*) from at('v.default.a', timestamp => '{instante}')")
            )
            .unwrap(),
            n,
            "{instante}"
        );
    }
    // Lo de hoy y lo de antes, en la misma sentencia.
    let (primero, _) = snapshots[0];
    assert_eq!(
        un_entero(
            &m,
            &format!(
                "select (select count(*) from a) * 10 + (select count(*) from at('a', snapshot => {primero}))"
            )
        )
        .unwrap(),
        31
    );
}
