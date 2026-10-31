//! **La poda al planificar** (0081 §7.6, K2): un filtro que se traduce llega a iceberg-rust y el escaneo no lee los
//! ficheros que no pueden casar, con el mismo resultado que sin él.

use datafusion::arrow::array::Int64Array;
use ore_runtime::{Contexto, Motor, OreOpciones, Papel, Zocalos};
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::{CatalogoOre, ejecutable};
use std::sync::Arc;

fn motor(lago: &LagoEnMemoria) -> Motor {
    let catalogo = CatalogoOre::new(lago.fuente(), lago.lago().clone()).con_puerta(lago.puerta());
    let z = Zocalos {
        catalogo: Arc::new(catalogo),
        ..Zocalos::default()
    };
    Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).unwrap()
}

/// Una sentencia y su único entero.
fn uno(m: &Motor, sql: &str) -> i64 {
    let mut s = m
        .sesion(&Contexto::nuevo("poda").en_base("v").que_escribe())
        .unwrap();
    let lotes = m.es().block_on(async {
        let p = s.planificar(sql).await.unwrap();
        let p = ejecutable(&s, p).await.unwrap();
        m.correr(&s, p).unwrap().recoger().await.unwrap()
    });
    lotes[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .map(|a| a.value(0))
        .unwrap_or_else(|| {
            datafusion::arrow::compute::cast(
                lotes[0].column(0),
                &datafusion::arrow::datatypes::DataType::Int64,
            )
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0)
        })
}

/// Tres ficheros de ids que no se pisan: 1–10, 11–20 y 21–30.
fn tres_ficheros() -> (LagoEnMemoria, Motor) {
    let lago = LagoEnMemoria::default();
    let m = motor(&lago);
    uno(
        &m,
        "create dataset a as select v as id from generate_series(1, 10) g(v)",
    );
    uno(
        &m,
        "insert into a select v from generate_series(11, 20) g(v)",
    );
    uno(
        &m,
        "insert into a select v from generate_series(21, 30) g(v)",
    );
    (lago, m)
}

/// Lo que lee una sentencia del almacén, y su resultado.
fn leido(lago: &LagoEnMemoria, m: &Motor, sql: &str) -> (usize, i64) {
    let antes = lago.objetos_leidos();
    let r = uno(m, sql);
    (lago.objetos_leidos() - antes, r)
}

#[test]
fn un_filtro_que_se_traduce_no_lee_los_ficheros_que_no_casan() {
    let (lago, m) = tres_ficheros();
    let (todo, n) = leido(&lago, &m, "select count(*) from a");
    assert_eq!(n, 30);
    let (podado, n) = leido(&lago, &m, "select count(*) from a where id = 15");
    assert_eq!(n, 1);
    // Los mismos metadatos, y un Parquet en vez de tres.
    assert_eq!(todo - podado, 2, "sin poda {todo}, con poda {podado}");
    let (_, n) = leido(
        &lago,
        &m,
        "select count(*) from a where id between 5 and 25",
    );
    assert_eq!(n, 21);
    let (_, n) = leido(
        &lago,
        &m,
        "select count(*) from a where id in (1, 30) or id > 28",
    );
    assert_eq!(n, 3);
}

#[test]
fn un_filtro_que_no_se_traduce_lee_todo_y_da_lo_mismo() {
    let (lago, m) = tres_ficheros();
    let (todo, _) = leido(&lago, &m, "select count(*) from a");
    // Una función sobre la columna: no se traduce, se lee todo y DataFusion filtra.
    let (leidos, n) = leido(&lago, &m, "select count(*) from a where id + 0 = 15");
    assert_eq!(n, 1);
    assert_eq!(leidos, todo);
}

#[test]
fn ningun_fichero_casa_y_el_resultado_es_vacio_sin_error() {
    let (lago, m) = tres_ficheros();
    let (_, n) = leido(&lago, &m, "select count(*) from a where id > 1000");
    assert_eq!(n, 0);
}
