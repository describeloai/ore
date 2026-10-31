//! **La foto** (0081 §7.5.3, decisión 2): una sentencia ve una sola foto del catálogo para todas sus tablas, fijada
//! al planificar; la siguiente lee otra y ve lo que se escribió.

use datafusion::arrow::array::{ArrayRef, Int64Array};
use datafusion::arrow::record_batch::RecordBatch;
use ore_core::punteros::Ficheros;
use ore_runtime::{Contexto, Motor, OreOpciones, Papel, Zocalos};
use ore_tabla::foto::{DePunteros, Fuente, Objeto};
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::{CatalogoOre, Nombre};
use std::sync::Arc;

fn ids(n: i64) -> Vec<RecordBatch> {
    let a: ArrayRef = Arc::new(Int64Array::from((1..=n).collect::<Vec<_>>()));
    vec![RecordBatch::try_from_iter([("id", a)]).unwrap()]
}

fn motor(lago: &LagoEnMemoria) -> Motor {
    let z = Zocalos {
        catalogo: Arc::new(CatalogoOre::new(lago.fuente(), lago.lago().clone())),
        ..Zocalos::default()
    };
    Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).unwrap()
}

fn un_entero(lotes: &[RecordBatch]) -> i64 {
    lotes[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

#[test]
fn una_sentencia_una_foto_y_la_siguiente_ve_lo_nuevo() {
    let lago = LagoEnMemoria::default();
    let (a, b) = (
        Nombre::new("v", "default", "a"),
        Nombre::new("v", "default", "b"),
    );
    lago.escribir(&a, ids(3)).unwrap();
    lago.escribir(&b, ids(3)).unwrap();
    lago.vista(&Nombre::new("v", "default", "va"), "select id from a");
    let m = motor(&lago);
    let sql =
        "select (select count(*) from a) + (select count(*) from b) + (select count(*) from va)";

    // Dos tablas y una vista: una sola lectura de la foto, al planificar.
    let mut s = m.sesion(&Contexto::nuevo("q1").en_base("v")).unwrap();
    let antes = lago.lecturas();
    let plan = m
        .es()
        .block_on(s.planificar(sql))
        .unwrap()
        .logico()
        .unwrap();
    assert_eq!(lago.lecturas(), antes + 1);

    // Se escribe entre planificar y correr: la sentencia sigue en su foto.
    lago.escribir(&a, ids(5)).unwrap();
    lago.escribir(&b, ids(5)).unwrap();
    let lotes = m
        .es()
        .block_on(m.correr(&s, plan).unwrap().recoger())
        .unwrap();
    assert_eq!(un_entero(&lotes), 9);

    // La siguiente lee otra foto y ve lo nuevo, en todas sus tablas.
    let mut s = m.sesion(&Contexto::nuevo("q2").en_base("v")).unwrap();
    let lotes = m
        .es()
        .block_on(async {
            let plan = s.planificar(sql).await?.logico()?;
            m.correr(&s, plan)?.recoger().await
        })
        .unwrap();
    assert_eq!(un_entero(&lotes), 15);
}

/// La foto de los punteros: cada uno con su `metadata_location` es un dataset, con su nombre entero (el de una
/// parte de schema, en `default`). El que no la tiene no es una tabla que leer.
#[test]
fn la_foto_de_los_punteros() {
    let raiz = std::env::temp_dir().join(format!("ore-tabla-foto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&raiz);
    let escribe = |ruta: &str, cuerpo: &str| {
        let f = raiz.join("datasets").join(ruta);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, cuerpo).unwrap();
    };
    escribe(
        "ventas/default/pedidos.json",
        r#"{"metadata_location":"gs://c/ore/v2/catalogo/ventas/default/pedidos/metadata/1.metadata.json"}"#,
    );
    escribe(
        "ventas/eu/clientes.json",
        r#"{"metadata_location":"gs://c/ore/v2/catalogo/ventas/eu/clientes/metadata/1.metadata.json"}"#,
    );
    escribe("ventas/default/a_medias.json", r#"{"dataset":"x"}"#);
    let foto = DePunteros::new(Arc::new(Ficheros::en(&raiz)))
        .foto()
        .unwrap();
    let _ = std::fs::remove_dir_all(&raiz);
    let nombres: Vec<String> = foto.objetos.keys().map(|n| n.to_string()).collect();
    assert_eq!(nombres, ["ventas.default.pedidos", "ventas.eu.clientes"]);
    assert!(matches!(
        &foto.objetos[&Nombre::new("ventas", "eu", "clientes")],
        Objeto::Dataset { metadata_location } if metadata_location.ends_with("clientes/metadata/1.metadata.json")
    ));
}
