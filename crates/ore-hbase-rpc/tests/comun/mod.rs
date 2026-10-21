//! Lo que comparten las pruebas contra el laboratorio y las que reproducen sus bytes grabados:
//! el **escenario** (la misma secuencia de operaciones) y el **servidor falso** que repite una conversacion
//! grabada y comprueba, trama a trama, que el cliente dice exactamente lo que dijo cuando se grabo.
#![allow(dead_code)]

pub mod simulado;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use ore_hbase_rpc::{Cliente, Consulta, Error};
use serde_json::{Value, json};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn clave(n: u64) -> Vec<u8> {
    format!("{n:026}").into_bytes()
}

/// La secuencia de operaciones que se graba y que se reproduce. Devuelve un resumen de lo que vio: el mismo
/// contra el HBase de verdad y contra los bytes grabados.
pub fn escenario(c: &Cliente) -> Value {
    let regiones = c.regiones("TestTable").unwrap();
    let r = &regiones[1];
    let mut con = c.conectar(&r.servidor).unwrap();

    // 120 filas dentro de la segunda region, en tres llamadas de 50
    let rango = Consulta {
        inicio: clave(250_000),
        fin: clave(250_120),
        filas_por_llamada: 50,
        ..Consulta::default()
    };
    let filas: Vec<_> = con
        .escanear(&r.nombre, &rango)
        .map(|f| f.unwrap())
        .collect();

    // soltar a medias cierra el scanner en el servidor
    let diez: Vec<_> = con
        .escanear(
            &r.nombre,
            &Consulta {
                inicio: r.inicio.clone(),
                filas_por_llamada: 50,
                ..Consulta::default()
            },
        )
        .take(10)
        .map(|f| f.unwrap().clave)
        .collect();

    let primera = filas[0].clone();
    let existente = con.leer_fila(&r.nombre, &primera.clave, &[]).unwrap();
    let mut sin = primera.clave.clone();
    sin.push(b'x');
    let ausente = con.leer_fila(&r.nombre, &sin, &[]).unwrap();
    let fuera = match con.leer_fila(&r.nombre, b"zzz", &[]) {
        Err(Error::Servidor { clase, .. }) => clase,
        otra => panic!("se esperaba WrongRegionException, llego {otra:?}"),
    };
    let sin_tabla = c.regiones("NoExiste").unwrap_err().to_string();
    let en_1970 = con
        .escanear(
            &r.nombre,
            &Consulta {
                inicio: clave(250_000),
                fin: clave(250_100),
                tiempo: Some((0, 1_000)),
                ..Consulta::default()
            },
        )
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .len();

    // el lector cruza el limite entre la primera y la segunda region (250000): dos scanners, dos regiones
    let cruce: Vec<_> = c
        .leer(
            "TestTable",
            &Consulta {
                inicio: clave(249_990),
                fin: clave(250_010),
                filas_por_llamada: 8,
                ..Consulta::default()
            },
            &ore_hbase_rpc::Cancelacion::nueva(),
        )
        .unwrap()
        .map(|f| f.unwrap().clave)
        .collect();

    json!({
        "lector_cruzando_regiones": {
            "filas": cruce.len(),
            "primera": hex(&cruce[0]),
            "ultima": hex(&cruce[cruce.len() - 1]),
        },
        "regiones": regiones.iter().map(|r| json!([hex(&r.inicio), hex(&r.fin), r.servidor.to_string()])).collect::<Vec<_>>(),
        "rango": {
            "filas": filas.len(),
            "primera": hex(&filas[0].clave),
            "ultima": hex(&filas[filas.len() - 1].clave),
            "celdas_de_la_primera": filas[0].celdas.len(),
            "marca_de_la_primera": filas[0].celdas[0].marca,
            "valor_de_la_primera": hex(&filas[0].celdas[0].valor),
        },
        "soltado": diez.iter().map(|k| hex(k)).collect::<Vec<_>>(),
        "get_existente": existente.map(|f| hex(&f.clave)),
        "get_ausente": ausente.map(|f| hex(&f.clave)),
        "get_fuera_de_la_region": fuera,
        "tabla_sin_regiones": sin_tabla,
        "en_1970": en_1970,
    })
}

/// Una conexion grabada: cada trama con su lado (`c` cliente, `s` servidor).
#[derive(Clone)]
struct Guion {
    tramas: Vec<(char, Vec<u8>)>,
}

pub struct Traza {
    pub resumen: Value,
    zk: Vec<Guion>,
    rpc: Vec<Guion>,
}

impl Traza {
    pub fn cargar(texto: &str) -> Traza {
        let v: Value = serde_json::from_str(texto).unwrap();
        let (mut zk, mut rpc) = (Vec::new(), Vec::new());
        for c in v["conexiones"].as_array().unwrap() {
            let tramas = c["tramas"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    let d = t["d"].as_str().unwrap().chars().next().unwrap();
                    let b = base64::engine::general_purpose::STANDARD
                        .decode(t["b"].as_str().unwrap())
                        .unwrap();
                    (d, b)
                })
                .collect();
            let g = Guion { tramas };
            if c["tipo"] == "zk" {
                zk.push(g);
            } else {
                rpc.push(g);
            }
        }
        Traza {
            resumen: v["resumen"].clone(),
            zk,
            rpc,
        }
    }
}

/// Un servidor falso para ZooKeeper y otro para el RegionServer, en el propio proceso.
pub struct Falso {
    pub zk: SocketAddr,
    pub rpc: SocketAddr,
    fallos: Arc<Mutex<Vec<String>>>,
    atendidas: Arc<Mutex<(usize, usize)>>,
    terminadas: Arc<Mutex<usize>>,
    esperadas: (usize, usize),
}

fn atender(mut s: std::net::TcpStream, g: Guion, fallos: Arc<Mutex<Vec<String>>>, nombre: String) {
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    for (i, (d, b)) in g.tramas.iter().enumerate() {
        if *d == 's' {
            if s.write_all(b).is_err() {
                return;
            }
            continue;
        }
        let mut recibido = vec![0u8; b.len()];
        if let Err(e) = s.read_exact(&mut recibido) {
            fallos.lock().unwrap().push(format!(
                "{nombre}, trama {i}: el cliente no dijo lo esperado ({e})"
            ));
            return;
        }
        if &recibido != b {
            fallos.lock().unwrap().push(format!(
                "{nombre}, trama {i}: el cliente dijo otra cosa\n   grabado: {}\n   recibido: {}",
                hex(b),
                hex(&recibido)
            ));
            return;
        }
    }
    // Despues de la ultima trama el cliente cierra: no manda nada mas.
    let mut resto = Vec::new();
    let _ = s.read_to_end(&mut resto);
    if !resto.is_empty() {
        fallos.lock().unwrap().push(format!(
            "{nombre}: el cliente siguio hablando ({} bytes)",
            resto.len()
        ));
    }
}

impl Traza {
    pub fn servir(&self) -> Falso {
        let fallos = Arc::new(Mutex::new(Vec::new()));
        let atendidas = Arc::new(Mutex::new((0usize, 0usize)));
        let terminadas = Arc::new(Mutex::new(0usize));
        let mut direcciones = Vec::new();
        for (guiones, nombre) in [(self.zk.clone(), "zk"), (self.rpc.clone(), "rpc")] {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            direcciones.push(l.local_addr().unwrap());
            let (fallos, atendidas, terminadas) =
                (fallos.clone(), atendidas.clone(), terminadas.clone());
            std::thread::spawn(move || {
                for (i, g) in guiones.into_iter().enumerate() {
                    let Ok((s, _)) = l.accept() else { return };
                    {
                        let mut a = atendidas.lock().unwrap();
                        if nombre == "zk" { a.0 += 1 } else { a.1 += 1 }
                    }
                    let (fallos, terminadas) = (fallos.clone(), terminadas.clone());
                    let n = format!("{nombre}[{i}]");
                    // Cada conexion en su hilo: el cliente abre la de `hbase:meta` con la del RegionServer viva.
                    // El guion de cada una se decide por el orden de llegada, que es el de la grabacion.
                    std::thread::spawn(move || {
                        atender(s, g, fallos, n);
                        *terminadas.lock().unwrap() += 1;
                    });
                }
            });
        }
        Falso {
            zk: direcciones[0],
            rpc: direcciones[1],
            fallos,
            atendidas,
            terminadas,
            esperadas: (self.zk.len(), self.rpc.len()),
        }
    }
}

impl Falso {
    /// Que el cliente hizo exactamente las conexiones grabadas y dijo exactamente lo grabado.
    pub fn comprobar(&self) {
        // Espera a que cada conexion acabe de comprobar su guion (el cliente ya cerro las suyas).
        let total = self.esperadas.0 + self.esperadas.1;
        for _ in 0..100 {
            if *self.terminadas.lock().unwrap() >= total {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let f = self.fallos.lock().unwrap();
        assert!(f.is_empty(), "{}", f.join("\n"));
        assert_eq!(
            *self.atendidas.lock().unwrap(),
            self.esperadas,
            "conexiones atendidas (zk, rpc)"
        );
    }
}

impl Drop for Falso {
    /// Si el cliente fallo antes de `comprobar`, lo que el servidor falso vio explica por que.
    fn drop(&mut self) {
        if std::thread::panicking() {
            for f in self.fallos.lock().unwrap().iter() {
                eprintln!("servidor falso: {f}");
            }
        }
    }
}
