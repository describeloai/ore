//! **Los nodos de ORE, de ida y vuelta por el cable** (0081 §7.5, C8·2 y C8·3). Lo que el cliente planifica y lo que
//! el scheduler planifica en físico se codifica como lo hace Ballista (`datafusion-proto` con el codec de ORE,
//! envuelto por el suelo) y se decodifica en otro sitio: sin leer nada del lago, y con la misma foto, el mismo
//! snapshot y la misma clave.

use datafusion::arrow::array::{ArrayRef, Int64Array};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::LogicalPlan;
use datafusion::physical_plan::{ExecutionPlan, collect, displayable};
use datafusion_proto::bytes::{
    logical_plan_from_bytes_with_extension_codec, logical_plan_to_bytes_with_extension_codec,
    physical_plan_from_bytes_with_extension_codec, physical_plan_to_bytes_with_extension_codec,
};
use ore_runtime::{Contexto, Motor, OreOpciones, Papel, Sesion, Zocalos, codecs};
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::{CatalogoOre, Nombre, ejecutable};
use std::sync::Arc;

fn ids(n: i64) -> Vec<RecordBatch> {
    let a: ArrayRef = Arc::new(Int64Array::from((1..=n).collect::<Vec<_>>()));
    vec![RecordBatch::try_from_iter_with_nullable([("id", a, true)]).unwrap()]
}

/// Un lago con `v.default.a` en tres versiones (1, 2 y 3 filas), y un motor que escribe en él.
fn lago_y_motor() -> (LagoEnMemoria, Motor, Vec<i64>) {
    let lago = LagoEnMemoria::default();
    let a = Nombre::new("v", "default", "a");
    let mut snapshots = Vec::new();
    for n in 1..=3 {
        let ml = lago.escribir(&a, ids(n)).unwrap();
        let t = lago.lago().abrir(&ml, "a").unwrap();
        snapshots.push(t.metadata().current_snapshot().unwrap().snapshot_id());
    }
    let catalogo = CatalogoOre::new(lago.fuente(), lago.lago().clone()).con_puerta(lago.puerta());
    let z = Zocalos {
        catalogo: Arc::new(catalogo),
        ..Zocalos::default()
    };
    let m = Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).unwrap();
    (lago, m, snapshots)
}

/// El codec de un proceso que lee y escribe en `lago`, como lo pone el suelo: envuelto con el de Ballista.
fn codec_de(lago: &LagoEnMemoria) -> codecs::Codecs {
    let ore = CatalogoOre::new(lago.fuente(), lago.lago().clone())
        .con_puerta(lago.puerta())
        .codec()
        .codecs();
    let (logico, fisico) = codecs::del_papel(Some(ore));
    codecs::Codecs { logico, fisico }
}

fn planificar(m: &Motor, sql: &str) -> (Sesion, LogicalPlan) {
    let mut s = m
        .sesion(&Contexto::nuevo("codec").en_base("v").que_escribe())
        .unwrap();
    let plan = m.es().block_on(async {
        let p = s.planificar(sql).await.unwrap();
        ejecutable(&s, p).await.unwrap()
    });
    (s, plan)
}

fn un_entero(lotes: &[RecordBatch]) -> i64 {
    lotes[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

/// Ida y vuelta del plan lógico, sin leer nada del lago al decodificar.
fn ida_y_vuelta_logica(
    lago: &LagoEnMemoria,
    s: &Sesion,
    plan: &LogicalPlan,
    c: &codecs::Codecs,
) -> LogicalPlan {
    let bytes = logical_plan_to_bytes_with_extension_codec(plan, c.logico.as_ref()).unwrap();
    let antes = lago.objetos_leidos();
    let vuelta = logical_plan_from_bytes_with_extension_codec(
        &bytes,
        s.estado().task_ctx().as_ref(),
        c.logico.as_ref(),
    )
    .unwrap();
    assert_eq!(lago.objetos_leidos(), antes, "decodificar leyó del lago");
    vuelta
}

fn ida_y_vuelta_fisica(
    lago: &LagoEnMemoria,
    s: &Sesion,
    plan: Arc<dyn ExecutionPlan>,
    c: &codecs::Codecs,
) -> Arc<dyn ExecutionPlan> {
    let bytes = physical_plan_to_bytes_with_extension_codec(plan, c.fisico.as_ref()).unwrap();
    let antes = lago.objetos_leidos();
    let vuelta = physical_plan_from_bytes_with_extension_codec(
        &bytes,
        s.estado().task_ctx().as_ref(),
        c.fisico.as_ref(),
    )
    .unwrap();
    assert_eq!(lago.objetos_leidos(), antes, "decodificar leyó del lago");
    vuelta
}

fn texto(p: &Arc<dyn ExecutionPlan>) -> String {
    displayable(p.as_ref()).indent(true).to_string()
}

/// C8·2: una lectura viaja con su foto. Lo que se escriba después de planificar no se ve al otro lado, y `at(…)` lleva
/// su snapshot.
#[test]
fn una_lectura_viaja_con_su_foto_y_su_snapshot() {
    let (lago, m, snapshots) = lago_y_motor();
    let c = codec_de(&lago);
    let (s, hoy) = planificar(&m, "select count(*) from a");
    let (s2, antes) = planificar(
        &m,
        &format!("select count(*) from at('a', snapshot => {})", snapshots[0]),
    );
    let hoy = ida_y_vuelta_logica(&lago, &s, &hoy, &c);
    let antes = ida_y_vuelta_logica(&lago, &s2, &antes, &c);
    // Una escritura después de planificar: no está en la foto que viajó.
    lago.escribir(&Nombre::new("v", "default", "a"), ids(9))
        .unwrap();
    let correr = |s: &Sesion, p: LogicalPlan| {
        m.es()
            .block_on(async { m.correr(s, p).unwrap().recoger().await })
            .unwrap()
    };
    assert_eq!(un_entero(&correr(&s, hoy)), 3);
    assert_eq!(un_entero(&correr(&s2, antes)), 1);
}

/// C8·2: el destino de una escritura viaja con su nombre, y al otro lado se confirma por la puerta de ese proceso.
#[test]
fn una_escritura_viaja_y_se_confirma_al_otro_lado() {
    let (lago, m, _) = lago_y_motor();
    let c = codec_de(&lago);
    let (s, plan) = planificar(&m, "insert into a values (10), (11)");
    let plan = ida_y_vuelta_logica(&lago, &s, &plan, &c);
    let commits = lago.commits();
    let lotes = m
        .es()
        .block_on(async { m.correr(&s, plan).unwrap().recoger().await })
        .unwrap();
    let n = lotes[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::UInt64Array>()
        .unwrap()
        .value(0);
    assert_eq!(n, 2);
    assert_eq!(lago.commits(), commits + 1);
}

/// C8·3: el escaneo viaja con sus ficheros ya planificados, y lee lo mismo.
#[test]
fn el_escaneo_viaja_con_sus_ficheros() {
    let (lago, m, _) = lago_y_motor();
    let c = codec_de(&lago);
    let (s, plan) = planificar(&m, "select id from a order by id");
    let fisico = m
        .es()
        .block_on(s.estado().create_physical_plan(&plan))
        .unwrap();
    assert!(texto(&fisico).contains("EscaneoOre"), "{}", texto(&fisico));
    let vuelta = ida_y_vuelta_fisica(&lago, &s, fisico.clone(), &c);
    assert_eq!(texto(&vuelta), texto(&fisico));
    let leer =
        |p: Arc<dyn ExecutionPlan>| m.es().block_on(collect(p, s.estado().task_ctx())).unwrap();
    let (a, b) = (leer(fisico), leer(vuelta));
    assert_eq!(
        datafusion::arrow::util::pretty::pretty_format_batches(&a)
            .unwrap()
            .to_string(),
        datafusion::arrow::util::pretty::pretty_format_batches(&b)
            .unwrap()
            .to_string()
    );
}

/// C8·3: el sink viaja con su operación, sus marcas y **su clave**: correr el plan de ida y el de vuelta es un solo
/// commit.
#[test]
fn el_sink_viaja_con_su_clave() {
    let (lago, m, _) = lago_y_motor();
    let c = codec_de(&lago);
    for sql in [
        "insert into a values (10)",
        "insert overwrite a values (10), (2)",
        "delete from a where id = 10",
        "update a set id = id + 1 where id = 2",
    ] {
        let (s, plan) = planificar(&m, sql);
        let fisico = m
            .es()
            .block_on(s.estado().create_physical_plan(&plan))
            .unwrap();
        assert!(
            texto(&fisico).contains("OreSink"),
            "{sql}: {}",
            texto(&fisico)
        );
        let vuelta = ida_y_vuelta_fisica(&lago, &s, fisico.clone(), &c);
        assert_eq!(texto(&vuelta), texto(&fisico), "{sql}");
        let commits = lago.commits();
        for p in [fisico, vuelta] {
            m.es().block_on(collect(p, s.estado().task_ctx())).unwrap();
        }
        assert_eq!(lago.commits(), commits + 1, "{sql}: dos commits");
    }
}
