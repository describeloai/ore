//! El cliente contra un **HBase de verdad** (el laboratorio de `pruebas-de-fuego/hbase/`, con `TestTable`: 10^6
//! filas en 4 regiones, que escribe `hbase pe --presplit=4`). Solo corre con `HBASE_LAB_ZK=127.0.0.1:2181` (y, si
//! el laboratorio esta en otros puertos, `HBASE_LAB_TRADUCCION`); sin ella, no hace nada.

use std::time::Instant;

mod comun;

use ore_hbase_rpc::{Cliente, Consulta, Error, Opciones};

fn cliente() -> Option<Cliente> {
    let zk = std::env::var("HBASE_LAB_ZK")
        .ok()
        .filter(|u| !u.is_empty())?;
    // Con el laboratorio en puertos desplazados: `HBASE_LAB_TRADUCCION=127.0.0.1:16020=127.0.0.1:26020`
    let o = Opciones::default()
        .con_traduccion(&std::env::var("HBASE_LAB_TRADUCCION").unwrap_or_default())
        .expect("la traduccion");
    Some(Cliente::nuevo(&zk, "/hbase", o).expect("el quorum"))
}

/// `TestTable` lleva claves de 26 cifras.
fn clave(n: u64) -> Vec<u8> {
    format!("{n:026}").into_bytes()
}

#[test]
fn zookeeper_dice_donde_esta_meta() {
    let Some(c) = cliente() else { return };
    let s = c.servidor_de_meta().unwrap();
    assert_eq!(s.puerto, 16020, "{s}");
}

#[test]
fn las_regiones_de_la_tabla_son_cuatro_contiguas_y_cubren_todo_el_espacio() {
    let Some(c) = cliente() else { return };
    let r = c.regiones("TestTable").unwrap();
    // El defecto que X0 encontro: empezar el scan de meta en `tabla,` perdia la primera region.
    assert_eq!(r.len(), 4, "{r:#?}");
    assert!(r[0].inicio.is_empty(), "la primera region empieza vacia");
    assert!(r[3].fin.is_empty(), "la ultima region acaba vacia");
    for p in r.windows(2) {
        assert_eq!(
            p[0].fin, p[1].inicio,
            "las regiones no dejan huecos ni se solapan"
        );
    }
    assert_eq!(
        c.regiones("default:TestTable").unwrap(),
        r,
        "default: es la misma tabla"
    );
}

#[test]
fn una_tabla_que_no_existe_lo_dice() {
    let Some(c) = cliente() else { return };
    let e = c.regiones("NoExiste").unwrap_err();
    assert!(matches!(e, Error::TablaSinRegiones(_)), "{e}");
}

#[test]
fn un_barrido_completo_llega_entero_y_en_orden() {
    let Some(c) = cliente() else { return };
    let regiones = c.regiones("TestTable").unwrap();
    let t = Instant::now();
    let (mut filas, mut anterior) = (0u64, Vec::new());
    for r in &regiones {
        let mut con = c.conectar(&r.servidor).unwrap();
        let consulta = Consulta {
            inicio: r.inicio.clone(),
            fin: r.fin.clone(),
            ..Consulta::default()
        };
        for f in con.escanear(&r.nombre, &consulta) {
            let f = f.unwrap();
            assert!(f.clave > anterior, "las claves salen ordenadas");
            assert_eq!(f.celdas.len(), 1, "TestTable lleva una celda por fila");
            anterior = f.clave;
            filas += 1;
        }
    }
    eprintln!("barrido de {filas} filas en {:?}", t.elapsed());
    assert_eq!(filas, 1_000_000);
}

#[test]
fn un_rango_de_claves_trae_exactamente_las_filas_de_dentro() {
    let Some(c) = cliente() else { return };
    let regiones = c.regiones("TestTable").unwrap();
    // [250000, 260000) cruza el limite entre la primera y la segunda region
    let (ini, fin) = (clave(245_000), clave(255_000));
    let mut total = 0;
    for r in &regiones {
        let mut con = c.conectar(&r.servidor).unwrap();
        let consulta = Consulta {
            inicio: ini.clone().max(r.inicio.clone()),
            fin: fin.clone(),
            ..Consulta::default()
        };
        if !r.fin.is_empty() && r.fin <= consulta.inicio {
            continue;
        }
        let consulta = Consulta {
            fin: if r.fin.is_empty() || fin < r.fin {
                fin.clone()
            } else {
                r.fin.clone()
            },
            ..consulta
        };
        if consulta.inicio >= consulta.fin {
            continue;
        }
        total += con
            .escanear(&r.nombre, &consulta)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len();
    }
    assert_eq!(total, 10_000);
}

#[test]
fn soltar_el_escaner_a_medias_deja_la_conexion_usable() {
    let Some(c) = cliente() else { return };
    let r = &c.regiones("TestTable").unwrap()[1];
    let mut con = c.conectar(&r.servidor).unwrap();
    let consulta = Consulta {
        inicio: r.inicio.clone(),
        fin: r.fin.clone(),
        filas_por_llamada: 50,
        ..Consulta::default()
    };
    let diez: Vec<_> = con
        .escanear(&r.nombre, &consulta)
        .take(10)
        .map(|f| f.unwrap())
        .collect();
    assert_eq!(diez.len(), 10);
    // Si el scanner quedara abierto, o se cerrara mal, la siguiente llamada fallaria con OutOfOrderScannerNextException
    let otra = con
        .escanear(&r.nombre, &consulta)
        .take(3)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(otra.len(), 3);
}

#[test]
fn get_de_una_fila_y_de_una_que_no_existe() {
    let Some(c) = cliente() else { return };
    let r = &c.regiones("TestTable").unwrap()[1];
    let mut con = c.conectar(&r.servidor).unwrap();
    let primera = con
        .escanear(
            &r.nombre,
            &Consulta {
                inicio: r.inicio.clone(),
                fin: r.fin.clone(),
                ..Consulta::default()
            },
        )
        .next()
        .unwrap()
        .unwrap();
    let f = con
        .leer_fila(&r.nombre, &primera.clave, &[])
        .unwrap()
        .expect("la fila existe");
    assert_eq!(f, primera);
    // una clave DENTRO de la region que no existe: no hay fila
    let mut sin = primera.clave.clone();
    sin.push(b'x');
    assert!(con.leer_fila(&r.nombre, &sin, &[]).unwrap().is_none());
    // una clave FUERA de la region: el servidor lo dice con su excepcion (nombre medido en el laboratorio)
    let e = con.leer_fila(&r.nombre, b"zzz", &[]).unwrap_err();
    assert!(
        matches!(&e, Error::Servidor { clase, .. } if clase.ends_with("WrongRegionException")),
        "{e}"
    );
}

#[test]
fn el_rango_de_tiempo_se_empuja_al_servidor() {
    let Some(c) = cliente() else { return };
    let r = &c.regiones("TestTable").unwrap()[0];
    let mut con = c.conectar(&r.servidor).unwrap();
    let vacia = Consulta {
        inicio: r.inicio.clone(),
        fin: r.fin.clone(),
        tiempo: Some((0, 1_000)),
        ..Consulta::default()
    };
    let ninguna = con
        .escanear(&r.nombre, &vacia)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(ninguna.is_empty(), "ninguna celda es de 1970");
    let todo = Consulta {
        tiempo: Some((0, u64::MAX / 2)),
        filas_por_llamada: 1000,
        ..vacia
    };
    let cinco = con
        .escanear(&r.nombre, &todo)
        .take(5)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(cinco.len(), 5);
}

/// El escenario que se graba (`pruebas-de-fuego/hbase/grabar_trazas.py`) y que `tests/trazas.rs` reproduce.
/// Con `HBASE_LAB_RESUMEN=ruta` deja alli lo que vio, para guardarlo junto a los bytes.
#[test]
fn escenario_grabado() {
    let Some(c) = cliente() else { return };
    let visto = comun::escenario(&c);
    assert_eq!(visto["regiones"].as_array().unwrap().len(), 4);
    assert_eq!(visto["rango"]["filas"], 120);
    if let Ok(ruta) = std::env::var("HBASE_LAB_RESUMEN") {
        std::fs::write(ruta, serde_json::to_string_pretty(&visto).unwrap()).unwrap();
    }
}
