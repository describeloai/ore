//! El cliente contra los **bytes de un HBase de verdad**, grabados con `pruebas-de-fuego/hbase/grabar_trazas.py`
//! (`pruebas-de-fuego/hbase/trazas/nativo.json`). No necesita laboratorio: un servidor falso repite lo que
//! ZooKeeper y el RegionServer contestaron, y comprueba que el cliente dice, byte a byte, lo mismo que dijo
//! cuando se grabo. Si cambia una peticion (un campo, un orden), esta prueba lo dice.

mod comun;

use comun::Traza;
use ore_hbase_rpc::{Cliente, Opciones};

const TRAZA: &str = include_str!("../../../pruebas-de-fuego/hbase/trazas/nativo.json");

#[test]
fn el_escenario_grabado_se_reproduce_byte_a_byte_y_da_lo_que_dio() {
    let traza = Traza::cargar(TRAZA);
    let falso = traza.servir();
    // El RegionServer se anuncio como 127.0.0.1:16020 al grabar: la traduccion lo lleva al falso.
    let o = Opciones::default()
        .con_traduccion(&format!("127.0.0.1:16020={}", falso.rpc))
        .unwrap();
    let c = Cliente::nuevo(&falso.zk.to_string(), "/hbase", o).unwrap();

    let visto = comun::escenario(&c);
    falso.comprobar();
    assert_eq!(
        visto, traza.resumen,
        "lo que el cliente ve de los bytes grabados"
    );

    // las cifras que importan, dichas aparte para que un cambio salte a la vista
    assert_eq!(visto["regiones"].as_array().unwrap().len(), 4);
    assert_eq!(visto["rango"]["filas"], 120);
    assert_eq!(visto["en_1970"], 0);
    assert!(
        visto["get_fuera_de_la_region"]
            .as_str()
            .unwrap()
            .ends_with("WrongRegionException")
    );
}
