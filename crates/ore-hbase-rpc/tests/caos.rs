//! El lector contra un **HBase de verdad al que se le hace dano mientras lee**: se parte la tabla, se reinicia
//! el servidor, se deshabilita, se deja caducar un scanner. Lo que se exige es lo de siempre: ni una fila perdida,
//! ni una repetida, y en orden de clave.
//!
//! Solo corre con `HBASE_LAB_ZK=127.0.0.1:2181` y `HBASE_LAB_CAOS=hb` (el nombre del contenedor del laboratorio,
//! `pruebas-de-fuego/hbase/`, con `TestTable`): usa `docker exec` y `docker restart`. Tardan: una lectura de 10^6
//! filas frenada para que dure lo que dura el dano. Se corren una a una (`--test-threads=1`).

use std::process::Command;
use std::time::{Duration, Instant};

use ore_hbase_rpc::{Cancelacion, Cliente, Consulta, Error, Opciones};

fn entorno() -> Option<(Cliente, String)> {
    let zk = std::env::var("HBASE_LAB_ZK")
        .ok()
        .filter(|u| !u.is_empty())?;
    let contenedor = std::env::var("HBASE_LAB_CAOS")
        .ok()
        .filter(|u| !u.is_empty())?;
    let o = Opciones::default()
        .con_traduccion(&std::env::var("HBASE_LAB_TRADUCCION").unwrap_or_default())
        .expect("la traduccion");
    Some((
        Cliente::nuevo(&zk, "/hbase", o).expect("el quorum"),
        contenedor,
    ))
}

/// Ordenes del shell de HBase, en un hilo aparte (arrancar la JVM tarda varios segundos).
fn shell(contenedor: &str, ordenes: &str) -> std::thread::JoinHandle<()> {
    let (c, o) = (contenedor.to_string(), ordenes.to_string());
    std::thread::spawn(move || {
        let mut p = Command::new("docker")
            .args([
                "exec",
                "-i",
                &c,
                "bash",
                "-c",
                "$HBASE_HOME/bin/hbase shell -n",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("docker");
        use std::io::Write;
        p.stdin
            .take()
            .unwrap()
            .write_all(format!("{o}\nexit\n").as_bytes())
            .unwrap();
        p.wait().unwrap();
    })
}

fn clave(n: u64) -> Vec<u8> {
    format!("{n:026}").into_bytes()
}

/// Lee `TestTable` entera frenando para que la lectura dure, mientras `durante` hace dano. Devuelve cuantas filas
/// llegaron, si fue en orden y sin repetir, y el fallo que la paro si lo hubo.
fn leer_frenado(
    c: &Cliente,
    freno_por_mil_filas: Duration,
    durante: impl FnOnce() + Send + 'static,
) -> (u64, Option<Error>, Duration) {
    let q = Consulta {
        filas_por_llamada: 1000,
        ..Consulta::default()
    };
    let hilo = std::thread::spawn(durante);
    let t = Instant::now();
    let (mut n, mut anterior, mut fallo) = (0u64, Vec::new(), None);
    for f in c.leer("TestTable", &q, &Cancelacion::nueva()).unwrap() {
        match f {
            Ok(f) => {
                assert!(
                    f.clave > anterior,
                    "orden roto o fila repetida en la numero {n}: {:?}",
                    String::from_utf8_lossy(&f.clave)
                );
                anterior = f.clave;
                n += 1;
                if n % 1000 == 0 {
                    std::thread::sleep(freno_por_mil_filas);
                }
            }
            Err(e) => fallo = Some(e),
        }
    }
    hilo.join().unwrap();
    (n, fallo, t.elapsed())
}

#[test]
fn partir_todas_las_regiones_a_mitad_de_lectura_no_pierde_ni_repite_filas() {
    let Some((c, cont)) = entorno() else { return };
    let antes = c.regiones("TestTable").unwrap().len();
    let (n, fallo, dt) = leer_frenado(&c, Duration::from_millis(30), move || {
        shell(&cont, "split 'TestTable'").join().unwrap();
    });
    let despues = c.regiones("TestTable").unwrap().len();
    eprintln!("regiones {antes} -> {despues}; {n} filas en {dt:?}; fallo: {fallo:?}");
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(n, 1_000_000);
    assert!(
        despues > antes,
        "la tabla no llego a partirse ({antes} -> {despues}): la prueba no probo nada"
    );
}

#[test]
fn reiniciar_el_servidor_a_mitad_de_lectura_se_espera_y_se_sigue_donde_se_estaba() {
    let Some((c, cont)) = entorno() else { return };
    let nombre = cont.clone();
    let (n, fallo, dt) = leer_frenado(&c, Duration::from_millis(30), move || {
        std::thread::sleep(Duration::from_secs(4));
        let r = Command::new("docker")
            .args(["restart", &nombre])
            .output()
            .expect("docker");
        assert!(r.status.success());
    });
    eprintln!("{n} filas en {dt:?}; fallo: {fallo:?}");
    assert!(fallo.is_none(), "{fallo:?}");
    assert_eq!(n, 1_000_000);
    assert!(
        dt > Duration::from_secs(10),
        "el reinicio no se noto: {dt:?}"
    );
}

#[test]
fn un_scanner_cuyo_arriendo_caduca_de_verdad_se_reabre_solo() {
    let Some((c, _)) = entorno() else { return };
    // el laboratorio caduca los arriendos a los 10 s (hbase.client.scanner.timeout.period)
    let q = Consulta {
        inicio: clave(300_000),
        fin: clave(305_000),
        filas_por_llamada: 100,
        ..Consulta::default()
    };
    let mut n = 0u64;
    let mut anterior = Vec::new();
    for f in c.leer("TestTable", &q, &Cancelacion::nueva()).unwrap() {
        let f = f.unwrap();
        assert!(f.clave > anterior);
        anterior = f.clave;
        n += 1;
        if n == 150 {
            std::thread::sleep(Duration::from_secs(13));
        }
    }
    assert_eq!(n, 5_000);
}

#[test]
fn deshabilitar_la_tabla_a_mitad_de_lectura_lo_dice_y_no_se_queda_esperando() {
    let Some((c, cont)) = entorno() else { return };
    let (n, fallo, dt) = leer_frenado(&c, Duration::from_millis(30), {
        let cont = cont.clone();
        move || {
            std::thread::sleep(Duration::from_secs(2));
            shell(&cont, "disable 'TestTable'").join().unwrap();
        }
    });
    // se deja la tabla como estaba
    shell(&cont, "enable 'TestTable'").join().unwrap();
    eprintln!("{n} filas en {dt:?}; fallo: {fallo:?}");
    assert!(
        matches!(fallo, Some(Error::TablaDeshabilitada(_))),
        "{fallo:?}"
    );
    assert!(n < 1_000_000);
    assert!(dt < Duration::from_secs(60), "{dt:?}");
}

#[test]
fn el_estado_de_la_tabla_se_lee_de_hbase_meta_habilitada_y_deshabilitada() {
    let Some((c, cont)) = entorno() else { return };
    let antes = c.estado_de_tabla("TestTable");
    shell(&cont, "disable 'TestTable'").join().unwrap();
    let durante = c.estado_de_tabla("TestTable");
    shell(&cont, "enable 'TestTable'").join().unwrap();
    let despues = c.estado_de_tabla("TestTable");
    eprintln!("antes: {antes:?}\ndurante: {durante:?}\ndespues: {despues:?}");
    use ore_hbase_rpc::proto::TableStateState::{Disabled, Enabled};
    assert_eq!(antes.unwrap(), Some(Enabled));
    assert_eq!(durante.unwrap(), Some(Disabled));
    assert_eq!(despues.unwrap(), Some(Enabled));
}

/// El filtro `PrefixFilter` serializado: el campo 1 (`prefix`) como bytes.
fn filtro_de_prefijo(prefijo: &[u8]) -> (String, Vec<u8>) {
    let mut m = vec![0x0a, prefijo.len() as u8];
    m.extend_from_slice(prefijo);
    ("org.apache.hadoop.hbase.filter.PrefixFilter".to_string(), m)
}

/// Un filtro del servidor que lo descarta todo se lee sin fallo. **Los latidos no se pudieron provocar**: el
/// servidor recorre cada region en unos 100 ms sin darle nada al cliente, y con un plazo de llamada corto (250 y
/// 60 ms) el cliente agota el tiempo antes de que el servidor mande un latido. Los latidos los decide el
/// arriendo del scanner del servidor (10 s en el laboratorio) y se revisan cada 10 000 celdas, no el plazo de la
/// llamada. Que el lector los sigue lo prueba el simulador (`los_latidos_no_son_el_final`); contra un servidor
/// de verdad **no esta observado** (ficha 0072, N2).
#[test]
fn un_filtro_que_lo_descarta_todo_se_lee_sin_fallo() {
    let Some((c, _)) = entorno() else { return };
    let q = Consulta {
        filtro: Some(filtro_de_prefijo(b"zzz")),
        filas_por_llamada: 1000,
        ..Consulta::default()
    };
    let t = Instant::now();
    let mut l = c.leer("TestTable", &q, &Cancelacion::nueva()).unwrap();
    let resultado: Result<Vec<_>, _> = l.by_ref().collect();
    eprintln!(
        "llamadas: {}; latidos: {}; en {:?}",
        l.llamadas(),
        l.latidos(),
        t.elapsed()
    );
    assert!(
        resultado.unwrap().is_empty(),
        "ninguna fila empieza por zzz"
    );
}
