//! **Un reintento nunca escribe dos veces** (0081 §4 y §7.5.3, decisión 5; C5). La clave de idempotencia se fija al
//! planificar y va en el plan: si la tarea del commit se ejecuta otra vez (un reintento de Ballista, de la red), la
//! puerta contesta lo de la primera y no hay un segundo snapshot. Y si alguien escribió después de la foto, el
//! commit no entra: nada se publica.

use datafusion::arrow::array::{ArrayRef, Int64Array};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::physical_plan::collect;
use ore_runtime::{Contexto, Motor, OreOpciones, Papel, Zocalos};
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::{CatalogoOre, Nombre, ejecutable};
use std::sync::Arc;

fn ids(desde: i64, n: i64) -> Vec<RecordBatch> {
    let a: ArrayRef = Arc::new(Int64Array::from((desde..desde + n).collect::<Vec<_>>()));
    vec![RecordBatch::try_from_iter_with_nullable([("id", a, true)]).unwrap()]
}

fn motor(lago: &LagoEnMemoria) -> Motor {
    let z = Zocalos {
        catalogo: Arc::new(
            CatalogoOre::new(lago.fuente(), lago.lago().clone()).con_puerta(lago.puerta()),
        ),
        ..Zocalos::default()
    };
    Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).unwrap()
}

fn cuenta(m: &Motor, sql: &str) -> i64 {
    let mut s = m.sesion(&Contexto::nuevo("cuenta").en_base("v")).unwrap();
    let lotes = m
        .es()
        .block_on(async {
            let plan = s.planificar(sql).await?.logico()?;
            m.correr(&s, plan)?.recoger().await
        })
        .unwrap();
    lotes[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

/// El plan físico de una escritura, planificado una vez: lo que Ballista mandaría a una tarea.
fn plan_fisico(
    m: &Motor,
    sql: &str,
) -> (
    Arc<dyn datafusion::physical_plan::ExecutionPlan>,
    Arc<datafusion::execution::TaskContext>,
) {
    let mut s = m
        .sesion(&Contexto::nuevo("escribe").en_base("v").que_escribe())
        .unwrap();
    m.es()
        .block_on(async {
            let plan = s.planificar(sql).await?;
            let plan = ejecutable(&s, plan).await?;
            let fisico = s.estado().create_physical_plan(&plan).await?;
            Ok::<_, ore_runtime::Fallo>((fisico, s.estado().task_ctx()))
        })
        .unwrap()
}

#[test]
fn la_misma_tarea_dos_veces_es_un_solo_commit() {
    let lago = LagoEnMemoria::default();
    lago.escribir(&Nombre::new("v", "default", "a"), ids(1, 3))
        .unwrap();
    let m = motor(&lago);
    let (fisico, ctx) = plan_fisico(&m, "insert into a values (10), (11)");
    for _ in 0..2 {
        m.es()
            .block_on(collect(fisico.clone(), ctx.clone()))
            .unwrap();
    }
    assert_eq!(
        lago.commits(),
        1,
        "la segunda vez contesta lo de la primera"
    );
    assert_eq!(cuenta(&m, "select count(*) from a"), 5);
}

#[test]
fn si_alguien_escribio_despues_de_la_foto_nada_se_publica() {
    let lago = LagoEnMemoria::default();
    let a = Nombre::new("v", "default", "a");
    lago.escribir(&a, ids(1, 3)).unwrap();
    let m = motor(&lago);
    let (fisico, ctx) = plan_fisico(&m, "insert into a values (10)");
    // Otro escribe entre la foto de la sentencia y su commit.
    lago.escribir(&a, ids(100, 7)).unwrap();
    let e = m
        .es()
        .block_on(collect(fisico, ctx))
        .unwrap_err()
        .to_string();
    assert!(e.contains("alguien escribió después"), "{e}");
    assert_eq!(lago.commits(), 0);
    assert_eq!(cuenta(&m, "select count(*) from a"), 7);
}

#[test]
fn una_sesion_que_solo_lee_no_escribe_aunque_el_catalogo_tenga_puerta() {
    let lago = LagoEnMemoria::default();
    lago.escribir(&Nombre::new("v", "default", "a"), ids(1, 3))
        .unwrap();
    let m = motor(&lago);
    let mut s = m.sesion(&Contexto::nuevo("lee").en_base("v")).unwrap();
    let e = m
        .es()
        .block_on(s.planificar("insert into a values (4)"))
        .unwrap_err()
        .to_string();
    assert!(e.contains("esta superficie sólo lee"), "{e}");
    assert_eq!(lago.commits(), 0);
}

/// `DELETE` y `UPDATE` (C6): lo que no toca ninguna fila no hace commit; lo que toca, uno.
#[test]
fn lo_que_no_toca_nada_no_hace_commit() {
    let lago = LagoEnMemoria::default();
    lago.escribir(&Nombre::new("v", "default", "a"), ids(1, 3))
        .unwrap();
    let m = motor(&lago);
    let escribe = |sql: &str| {
        let (fisico, ctx) = plan_fisico(&m, sql);
        let lotes = m.es().block_on(collect(fisico, ctx)).unwrap();
        lotes[0]
            .column(0)
            .as_any()
            .downcast_ref::<datafusion::arrow::array::UInt64Array>()
            .unwrap()
            .value(0)
    };
    assert_eq!(escribe("delete from a where id > 100"), 0);
    assert_eq!(escribe("update a set id = 0 where id > 100"), 0);
    assert_eq!(lago.commits(), 0);
    assert_eq!(escribe("delete from a where id = 1"), 1);
    assert_eq!(escribe("update a set id = id + 10"), 2);
    assert_eq!(lago.commits(), 2);
    assert_eq!(cuenta(&m, "select sum(id) from a"), 25);
}

/// Una vista no tiene filas propias: `DELETE` y `UPDATE` sobre ella no son de ORE SQL.
#[test]
fn una_vista_no_se_borra_ni_se_cambia() {
    let lago = LagoEnMemoria::default();
    lago.escribir(&Nombre::new("v", "default", "a"), ids(1, 3))
        .unwrap();
    lago.vista(&Nombre::new("v", "default", "va"), "select id from a");
    let m = motor(&lago);
    for sql in ["delete from va where id = 1", "update va set id = 0"] {
        let mut s = m
            .sesion(&Contexto::nuevo("vista").en_base("v").que_escribe())
            .unwrap();
        let e = m
            .es()
            .block_on(async {
                let plan = s.planificar(sql).await?;
                let plan = ejecutable(&s, plan).await?;
                m.correr(&s, plan)?.recoger().await
            })
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("`va` es una vista: no tiene filas propias"),
            "{sql}: {e}"
        );
    }
    assert_eq!(lago.commits(), 0);
}

/// `MERGE INTO` (C7): lo que no casa no hace commit; una fila del destino que casa con dos de la fuente falla y no
/// publica nada; el upsert, un commit.
#[test]
fn merge_un_commit_o_ninguno() {
    let lago = LagoEnMemoria::default();
    lago.escribir(&Nombre::new("v", "default", "a"), ids(1, 3))
        .unwrap();
    let m = motor(&lago);
    let correr = |sql: &str| {
        let (fisico, ctx) = plan_fisico(&m, sql);
        m.es().block_on(collect(fisico, ctx))
    };
    correr("merge into a using (values (99)) s(id) on a.id = s.id when matched then delete")
        .unwrap();
    assert!(
        correr("merge into a using (values (1), (1)) s(id) on a.id = s.id when matched then update set id = 7")
            .unwrap_err()
            .to_string()
            .contains("casa con más de una")
    );
    assert_eq!(lago.commits(), 0);
    correr(
        "merge into a using (values (3), (4)) s(id) on a.id = s.id \
         when matched then update set id = s.id * 10 when not matched then insert values (s.id)",
    )
    .unwrap();
    assert_eq!(lago.commits(), 1);
    assert_eq!(cuenta(&m, "select sum(id) from a"), 1 + 2 + 30 + 4);
}
