//! El **lector** frente a lo que un HBase de verdad hace durante una lectura larga, en un cluster simulado
//! (`comun/simulado.rs`) en el que cada fallo se inyecta a voluntad. En cada caso se exige lo mismo: **ni una
//! fila perdida, ni una repetida, y en orden de clave**. No necesita laboratorio.

mod comun;

use std::time::{Duration, Instant};

use comun::simulado::{ANUNCIADO, Accion, Llamada, Mundo, Sim, clave};
use ore_hbase_rpc::proto::TableStateState;
use ore_hbase_rpc::{Cancelacion, Clase, Cliente, Consulta, Error, Opciones, Politica};

fn cliente(sim: &Sim, max_intentos: u32) -> Cliente {
    let o = Opciones {
        tiempo: Duration::from_secs(5),
        politica: Politica {
            pausa: Duration::from_millis(1),
            max_intentos,
        },
        ..Opciones::default()
    }
    .con_traduccion(&format!("{ANUNCIADO}={}", sim.rpc))
    .unwrap();
    Cliente::nuevo(&sim.zk.to_string(), "/hbase", o).unwrap()
}

fn consulta() -> Consulta {
    Consulta {
        filas_por_llamada: 50,
        ..Consulta::default()
    }
}

/// Todas las claves que el lector entrega, o las que entrego antes del fallo que lo paro.
fn leer(c: &Cliente, q: &Consulta) -> (Vec<Vec<u8>>, Option<Error>) {
    let lector = c.leer("t", q, &Cancelacion::nueva()).unwrap();
    let (mut claves, mut fallo) = (Vec::new(), None);
    for f in lector {
        match f {
            Ok(f) => claves.push(f.clave),
            Err(e) => fallo = Some(e),
        }
    }
    (claves, fallo)
}

fn esperadas(desde: usize, hasta: usize) -> Vec<Vec<u8>> {
    (desde..hasta).map(clave).collect()
}

fn nada(_: &Llamada, _: &mut Mundo) -> Accion {
    Accion::Normal
}

#[test]
fn lee_toda_la_tabla_en_orden_de_una_region_a_la_siguiente() {
    let sim = Sim::nuevo(400, 4, nada);
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
    assert_eq!(sim.n_registros("abre"), 4, "un scanner por region");
    assert_eq!(sim.n_registros("meta"), 1, "hbase:meta se pregunta una vez");
}

#[test]
fn un_rango_solo_toca_las_regiones_que_cruza_y_se_recorta_en_los_dos_extremos() {
    let sim = Sim::nuevo(400, 4, nada);
    let q = Consulta {
        inicio: clave(150),
        fin: clave(250),
        ..consulta()
    };
    let (claves, fallo) = leer(&cliente(&sim, 15), &q);
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(150, 250));
    assert_eq!(
        sim.n_registros("abre"),
        2,
        "las regiones 1 y 2, ninguna mas: {:?}",
        sim.registro()
    );
}

#[test]
fn un_rango_vacio_o_al_reves_no_abre_ningun_scanner() {
    let sim = Sim::nuevo(400, 4, nada);
    let q = Consulta {
        inicio: clave(300),
        fin: clave(300),
        ..consulta()
    };
    assert!(leer(&cliente(&sim, 15), &q).0.is_empty());
    assert_eq!(sim.n_registros("abre"), 0);
}

#[test]
fn una_region_que_se_parte_a_mitad_de_lectura_se_refresca_y_sigue_sin_perder_ni_repetir() {
    // en la llamada 4 la region madre se parte: sus scanners se pierden y su nombre ya no se sirve
    let sim = Sim::nuevo(400, 2, |l, m| {
        if l.n == 4 {
            m.partir(&clave(170));
        }
        Accion::Normal
    });
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
    let r = sim.registro();
    assert!(
        r.iter().any(|l| l.contains("NotServingRegionException")),
        "{r:?}"
    );
    assert!(
        sim.n_registros("meta") >= 2,
        "tuvo que volver a preguntar a hbase:meta: {r:?}"
    );
}

#[test]
fn un_scanner_cuyo_arriendo_caduca_se_reabre_desde_la_ultima_clave_entregada_exclusiva() {
    let sim = Sim::nuevo(400, 2, |l, m| {
        if l.n == 3 {
            m.olvidar_scanners();
        }
        Accion::Normal
    });
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
    // la ultima entregada antes de perder el scanner es la 99; se reabre en la 99 + un byte cero
    let reabre = format!("desde {}\0", String::from_utf8(clave(99)).unwrap());
    assert!(
        sim.registro().iter().any(|l| l.ends_with(&reabre)),
        "{:?}",
        sim.registro()
    );
    assert_eq!(
        sim.n_registros("meta"),
        1,
        "un scanner perdido no obliga a preguntar a hbase:meta"
    );
}

#[test]
fn una_conexion_cortada_se_reabre_y_sigue() {
    let sim = Sim::nuevo(400, 2, |l, _| {
        if l.n == 3 {
            Accion::CortarConexion
        } else {
            Accion::Normal
        }
    });
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
}

#[test]
fn un_regionserver_que_tarda_en_volver_se_espera_con_pausas_crecientes() {
    let sim = Sim::nuevo(400, 2, |l, m| {
        if l.n == 3 {
            m.caido = true;
            Accion::CortarConexion
        } else {
            Accion::Normal
        }
    });
    let mundo = sim.mundo.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        mundo.lock().unwrap().caido = false;
    });
    let t = Instant::now();
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
    assert!(
        t.elapsed() >= Duration::from_millis(350),
        "esperó a que volviera: {:?}",
        t.elapsed()
    );
}

#[test]
fn si_no_vuelve_se_agotan_los_reintentos_y_el_fallo_dice_cuantos_y_cual_fue_el_ultimo() {
    let sim = Sim::nuevo(400, 2, |l, m| {
        if l.n == 2 {
            m.caido = true;
            Accion::CortarConexion
        } else {
            Accion::Normal
        }
    });
    let t = Instant::now();
    let (claves, fallo) = leer(&cliente(&sim, 4), &consulta());
    assert_eq!(
        claves,
        esperadas(0, 50),
        "lo entregado antes del fallo llega"
    );
    let Some(Error::Agotado { intentos, ultimo }) = fallo else {
        panic!("se esperaba Agotado: {fallo:?}")
    };
    assert_eq!(intentos, 4);
    assert_eq!(ultimo.clase(), Clase::Red, "{ultimo}");
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "no se cuelga: {:?}",
        t.elapsed()
    );
}

#[test]
fn un_error_fatal_para_a_la_primera_sin_reintentar() {
    let sim = Sim::nuevo(400, 2, |l, _| {
        if l.n == 2 {
            Accion::Excepcion {
                clase: "org.apache.hadoop.hbase.security.AccessDeniedException",
                mensaje: "Insufficient permissions",
                no_reintentar: true,
            }
        } else {
            Accion::Normal
        }
    });
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert_eq!(claves, esperadas(0, 50));
    let Some(e @ Error::Servidor { .. }) = fallo else {
        panic!("se esperaba el error del servidor: {fallo:?}")
    };
    assert_eq!(e.clase(), Clase::Fatal);
    assert_eq!(
        sim.n_registros("excepcion"),
        1,
        "una sola, sin reintentos: {:?}",
        sim.registro()
    );
}

#[test]
fn un_servidor_saturado_se_espera_sin_volver_a_preguntar_a_meta() {
    let sim = Sim::nuevo(400, 2, |l, _| {
        if l.n == 2 || l.n == 3 {
            Accion::Excepcion {
                clase: "org.apache.hadoop.hbase.RegionTooBusyException",
                mensaje: "Above memstore limit",
                no_reintentar: false,
            }
        } else {
            Accion::Normal
        }
    });
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
    assert_eq!(sim.n_registros("meta"), 1);
}

#[test]
fn los_latidos_no_son_el_final() {
    // el servidor contesta tres veces «sigo trabajando» sin una fila
    let sim = Sim::nuevo(400, 2, |l, _| {
        if (2..=4).contains(&l.n) {
            Accion::Latido
        } else {
            Accion::Normal
        }
    });
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
    assert_eq!(sim.n_registros("latido"), 3);
}

#[test]
fn una_tabla_deshabilitada_se_dice_y_no_se_espera_a_que_vuelva() {
    let sim = Sim::nuevo(400, 2, |_, m| {
        m.estado = TableStateState::Disabled;
        Accion::Excepcion {
            clase: "org.apache.hadoop.hbase.NotServingRegionException",
            mensaje: "is not online",
            no_reintentar: false,
        }
    });
    let t = Instant::now();
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(claves.is_empty());
    assert!(
        matches!(fallo, Some(Error::TablaDeshabilitada(_))),
        "{fallo:?}"
    );
    assert!(sim.n_registros("excepcion") <= 3, "{:?}", sim.registro());
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

#[test]
fn un_servidor_que_repite_filas_tras_reabrir_no_las_entrega_dos_veces() {
    let sim = Sim::nuevo(400, 2, |l, m| {
        if l.n == 3 {
            m.olvidar_scanners();
            // el scanner nuevo ignorara el inicio pedido y arrancara en el principio de la region
            m.repetir_desde_el_principio = true;
        }
        Accion::Normal
    });
    let (claves, fallo) = leer(&cliente(&sim, 15), &consulta());
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(claves, esperadas(0, 400));
}

#[test]
fn cancelar_corta_una_llamada_que_el_servidor_tiene_dormida_y_no_espera_a_que_acabe() {
    let sim = Sim::nuevo(400, 2, |l, _| {
        if l.n == 2 {
            Accion::Dormir(Duration::from_secs(5))
        } else {
            Accion::Normal
        }
    });
    let c = cliente(&sim, 15);
    let cancelacion = Cancelacion::nueva();
    let otra = cancelacion.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        otra.cancelar();
    });
    let t = Instant::now();
    let mut entregadas = 0;
    let mut fallo = None;
    for f in c.leer("t", &consulta(), &cancelacion).unwrap() {
        match f {
            Ok(_) => entregadas += 1,
            Err(e) => fallo = Some(e),
        }
    }
    assert_eq!(entregadas, 50, "lo que llego antes de cancelar");
    assert!(matches!(fallo, Some(Error::Cancelada)), "{fallo:?}");
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "tardo {:?}",
        t.elapsed()
    );
}

#[test]
fn soltar_el_lector_a_medias_cierra_el_scanner_en_el_servidor() {
    let sim = Sim::nuevo(400, 2, nada);
    let c = cliente(&sim, 15);
    {
        let l = c.leer("t", &consulta(), &Cancelacion::nueva()).unwrap();
        let diez: Vec<_> = l.take(10).map(|f| f.unwrap().clave).collect();
        assert_eq!(diez, esperadas(0, 10));
    }
    assert_eq!(sim.n_registros("cierra"), 1, "{:?}", sim.registro());
}

#[test]
fn una_region_sin_servidor_es_un_fallo_de_region_y_no_un_panico() {
    let sim = Sim::nuevo(400, 2, nada);
    sim.mundo.lock().unwrap().regiones[1].sin_servidor = true;
    let c = cliente(&sim, 3);
    let e = c.regiones("t").unwrap_err();
    assert!(matches!(e, Error::RegionSinServidor(_)), "{e}");
    assert_eq!(e.clase(), Clase::Region);
}
