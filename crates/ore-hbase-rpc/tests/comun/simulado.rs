//! **Un cluster de HBase simulado**, en el propio proceso: un ZooKeeper, `hbase:meta` y un RegionServer que
//! hablan el protocolo de verdad (los mismos mensajes, generados del jar de HBase) sobre una tabla en memoria,
//! y en los que una prueba puede inyectar los fallos que un cluster real produce: partir una region, caducar un
//! scanner, cortar la conexion, caer el servidor, saturarlo, dormirlo.
//!
//! No sustituye al laboratorio ni a los bytes grabados (`trazas.rs`): lo que simula es **el comportamiento**
//! que esas pruebas fijan (que excepcion devuelve un scanner perdido, que `more_results_in_region` cierra el
//! scanner…), para poder ejercitar el reintento y la reanudacion de forma exacta y en el CI. Las clases de
//! excepcion que devuelve son las grabadas en `pruebas-de-fuego/hbase/trazas/excepciones.json`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ore_hbase_rpc::proto::{
    Cell, CellType, ExceptionResponse, GetRequest, GetResponse, MetaRegionServer, RegionInfo,
    RequestHeader, ResponseHeader, ResultadoPb, ScanRequest, ScanResponse, ServerName, TableName,
    TableState, TableStateState,
};
use prost::Message;

pub const TABLA: &str = "t";
/// Lo que el cluster simulado anuncia como su RegionServer; el cliente lo traduce a donde escucha de verdad.
pub const ANUNCIADO: &str = "rs.sim:16020";

/// El valor de la celda de una fila.
pub fn valor(clave: &[u8]) -> Vec<u8> {
    [b"v".as_slice(), clave].concat()
}

/// La clave de la fila numero `n` (ocho cifras: el orden de bytes es el numerico).
pub fn clave(n: usize) -> Vec<u8> {
    format!("{n:08}").into_bytes()
}

#[derive(Clone, Debug)]
pub struct RegionSim {
    pub nombre: Vec<u8>,
    pub inicio: Vec<u8>,
    pub fin: Vec<u8>,
    /// Sin servidor asignado (una region que se mueve): `hbase:meta` no trae `info:server`.
    pub sin_servidor: bool,
}

struct ScannerSim {
    region: Vec<u8>,
    siguiente: Vec<u8>,
    stop: Vec<u8>,
    seq: u64,
    /// Las columnas pedidas `(familia, cualificadores)`; vacio: todas. **Una fila sin ninguna de ellas no se devuelve**,
    /// como en HBase (medido en el kit: el defecto que hace desaparecer las filas con nulos).
    columnas: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    /// `FirstKeyOnlyFilter`: de cada fila, solo la primera celda.
    solo_primera: bool,
}

/// Lo que una llamada le dice al guion para que decida.
#[derive(Debug, Clone)]
pub struct Llamada {
    /// Numero de llamada Scan/Get desde el principio (la primera es la 1).
    pub n: usize,
    pub metodo: String,
    /// El nombre de la region si abre un scanner.
    pub region: Option<Vec<u8>>,
    /// `true` si abre un scanner, `false` si lo sigue o lo cierra.
    pub abre: bool,
    /// Cuantos scanners ha abierto el cliente en regiones de datos hasta ahora (esta llamada incluida).
    pub aperturas: usize,
    /// Cuantas continuaciones lleva el scanner actual.
    pub continuaciones: usize,
}

/// Lo que el guion hace con una llamada.
pub enum Accion {
    Normal,
    Excepcion {
        clase: &'static str,
        mensaje: &'static str,
        no_reintentar: bool,
    },
    CortarConexion,
    Dormir(Duration),
    /// Contesta sin filas y con `heartbeat_message`: el servidor sigue trabajando.
    Latido,
}

/// Lo que decide, llamada a llamada, que hace el servidor.
type Guion = Box<dyn FnMut(&Llamada, &mut Mundo) -> Accion + Send>;

/// Las celdas de una fila, dada su clave: `(familia, cualificador, valor)`.
pub type Celdas = Arc<dyn Fn(&[u8]) -> Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> + Send + Sync>;

/// La celda de siempre: `f:q` = `v` + la clave.
pub fn celdas_de_texto() -> Celdas {
    Arc::new(|k| vec![(b"f".to_vec(), b"q".to_vec(), valor(k))])
}

pub struct Mundo {
    /// Las claves, **ordenadas por bytes**.
    pub filas: Vec<Vec<u8>>,
    /// Las celdas de cada fila.
    pub celdas_de: Celdas,
    pub regiones: Vec<RegionSim>,
    pub estado: TableStateState,
    /// Mientras sea `true`, toda conexion nueva se cierra y toda llamada corta la conexion.
    pub caido: bool,
    /// La proxima apertura de scanner ignora `start_row` y arranca en el principio de la region (un servidor
    /// que repite filas: el lector no debe entregarlas dos veces).
    pub repetir_desde_el_principio: bool,
    pub registro: Vec<String>,
    scanners: HashMap<u64, ScannerSim>,
    id_scanner: u64,
    generacion: u64,
    llamadas: usize,
    aperturas: usize,
    continuaciones: usize,
    guion: Guion,
}

impl Mundo {
    fn nombre_de_region(&mut self, inicio: &[u8]) -> Vec<u8> {
        self.generacion += 1;
        format!(
            "{TABLA},{},{}.enc{}.",
            String::from_utf8_lossy(inicio),
            1000 + self.generacion,
            self.generacion
        )
        .into_bytes()
    }

    /// Parte la region que contiene `clave` en dos: las dos nuevas tienen nombre nuevo y la madre deja de
    /// servirse. Los scanners abiertos se pierden, como cuando un RegionServer cierra la region madre.
    pub fn partir(&mut self, clave: &[u8]) {
        let i = self
            .regiones
            .iter()
            .position(|r| {
                r.inicio.as_slice() <= clave && (r.fin.is_empty() || clave < r.fin.as_slice())
            })
            .expect("una region que contenga la clave");
        let madre = self.regiones.remove(i);
        let (n1, n2) = (
            self.nombre_de_region(&madre.inicio),
            self.nombre_de_region(clave),
        );
        self.regiones.insert(
            i,
            RegionSim {
                nombre: n2,
                inicio: clave.to_vec(),
                fin: madre.fin.clone(),
                sin_servidor: false,
            },
        );
        self.regiones.insert(
            i,
            RegionSim {
                nombre: n1,
                inicio: madre.inicio,
                fin: clave.to_vec(),
                sin_servidor: false,
            },
        );
        self.scanners.clear();
        self.registro
            .push(format!("partida en {}", String::from_utf8_lossy(clave)));
    }

    /// El servidor pierde todos los scanners (se reinicio, o caducaron).
    pub fn olvidar_scanners(&mut self) {
        self.scanners.clear();
    }

    fn resultado(clave: &[u8], celdas: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>) -> ResultadoPb {
        ResultadoPb {
            cell: celdas
                .into_iter()
                .map(|(familia, cualificador, valor)| Cell {
                    row: Some(clave.to_vec()),
                    family: Some(familia),
                    qualifier: Some(cualificador),
                    timestamp: Some(1000),
                    cell_type: Some(CellType::Put as i32),
                    value: Some(valor),
                    tags: None,
                })
                .collect(),
            associated_cell_count: None,
            exists: None,
            stale: None,
            partial: Some(false),
        }
    }

    fn fila_de_meta(&self, r: &RegionSim, padre_partido: bool) -> ResultadoPb {
        let info = RegionInfo {
            region_id: 1,
            table_name: TableName {
                namespace: b"default".to_vec(),
                qualifier: TABLA.as_bytes().to_vec(),
            },
            start_key: Some(r.inicio.clone()),
            end_key: Some(r.fin.clone()),
            offline: Some(padre_partido),
            split: Some(padre_partido),
            replica_id: None,
        };
        let celda = |q: &[u8], v: Vec<u8>| Cell {
            row: Some(r.nombre.clone()),
            family: Some(b"info".to_vec()),
            qualifier: Some(q.to_vec()),
            timestamp: Some(1),
            cell_type: Some(CellType::Put as i32),
            value: Some(v),
            tags: None,
        };
        let mut cell = vec![celda(
            b"regioninfo",
            [b"PBUF".as_slice(), &info.encode_to_vec()].concat(),
        )];
        if !r.sin_servidor {
            cell.push(celda(b"server", ANUNCIADO.as_bytes().to_vec()));
        }
        ResultadoPb {
            cell,
            associated_cell_count: None,
            exists: None,
            stale: None,
            partial: Some(false),
        }
    }

    fn scan(&mut self, p: ScanRequest) -> Result<ScanResponse, (&'static str, String, bool)> {
        if let Some(id) = p.scanner_id {
            let Some(s) = self.scanners.get_mut(&id) else {
                return Err((
                    "org.apache.hadoop.hbase.UnknownScannerException",
                    format!("Unknown scanner '{id}'"),
                    true,
                ));
            };
            if p.close_scanner == Some(true) {
                self.registro.push("cierra".to_string());
                self.scanners.remove(&id);
                return Ok(ScanResponse::default());
            }
            if p.next_call_seq.unwrap_or(0) != s.seq {
                return Err((
                    "org.apache.hadoop.hbase.exceptions.OutOfOrderScannerNextException",
                    format!(
                        "Expected nextCallSeq: {} But the nextCallSeq got from client: {:?}",
                        s.seq, p.next_call_seq
                    ),
                    true,
                ));
            }
            s.seq += 1;
            return Ok(self.lote(id, p.number_of_rows.unwrap_or(100) as usize, false));
        }
        let region = p.region.expect("region").value;
        if region == b"hbase:meta,,1" {
            let mut results: Vec<ResultadoPb> = self
                .regiones
                .iter()
                .map(|r| self.fila_de_meta(r, false))
                .collect();
            // y una madre ya partida, que el cliente debe ignorar
            results.push(self.fila_de_meta(
                &RegionSim {
                    nombre: format!("{TABLA},,1.madre.").into_bytes(),
                    inicio: Vec::new(),
                    fin: Vec::new(),
                    sin_servidor: false,
                },
                true,
            ));
            return Ok(ScanResponse {
                scanner_id: Some(0),
                more_results: Some(false),
                more_results_in_region: Some(false),
                results,
                ..ScanResponse::default()
            });
        }
        let Some(r) = self.regiones.iter().find(|r| r.nombre == region).cloned() else {
            return Err((
                "org.apache.hadoop.hbase.NotServingRegionException",
                format!("{} is not online on sim", String::from_utf8_lossy(&region)),
                false,
            ));
        };
        let scan = p.scan.expect("scan");
        let mut inicio = scan.start_row.unwrap_or_default();
        if inicio.as_slice() < r.inicio.as_slice() {
            inicio = r.inicio.clone();
        }
        if std::mem::take(&mut self.repetir_desde_el_principio) {
            inicio = r.inicio.clone();
        }
        let stop = {
            let s = scan.stop_row.unwrap_or_default();
            match (s.is_empty(), r.fin.is_empty()) {
                (true, _) => r.fin.clone(),
                (_, true) => s,
                _ => s.min(r.fin.clone()),
            }
        };
        self.id_scanner += 1;
        let id = self.id_scanner;
        let columnas = scan
            .column
            .iter()
            .map(|c| (c.family.clone(), c.qualifier.clone()))
            .collect();
        let solo_primera = scan
            .filter
            .as_ref()
            .is_some_and(|f| f.name.ends_with("FirstKeyOnlyFilter"));
        self.scanners.insert(
            id,
            ScannerSim {
                region,
                siguiente: inicio,
                stop,
                seq: 0,
                columnas,
                solo_primera,
            },
        );
        Ok(self.lote(id, p.number_of_rows.unwrap_or(100) as usize, true))
    }

    fn lote(&mut self, id: u64, n: usize, abre: bool) -> ScanResponse {
        let celdas = self.celdas_de.clone();
        let s = self.scanners.get_mut(&id).expect("scanner");
        let mut results = Vec::new();
        let mut siguiente = None;
        for f in self.filas.iter().filter(|f| {
            f.as_slice() >= s.siguiente.as_slice()
                && (s.stop.is_empty() || f.as_slice() < s.stop.as_slice())
        }) {
            // las celdas que el cliente pidio; una fila que se queda sin ninguna no existe para el scan
            let mut propias: Vec<_> = celdas(f)
                .into_iter()
                .filter(|(fam, q, _)| {
                    s.columnas.is_empty()
                        || s.columnas
                            .iter()
                            .any(|(f2, qs)| f2 == fam && (qs.is_empty() || qs.contains(q)))
                })
                .collect();
            if s.solo_primera {
                propias.truncate(1);
            }
            if propias.is_empty() {
                continue;
            }
            if results.len() == n {
                siguiente = Some(f.clone());
                break;
            }
            results.push(Self::resultado(f, propias));
        }
        let hay_mas = siguiente.is_some();
        if let Some(sig) = siguiente {
            s.siguiente = sig;
        }
        let region_hecha = !hay_mas;
        if region_hecha {
            self.scanners.remove(&id);
        }
        let _ = abre;
        ScanResponse {
            scanner_id: Some(id),
            more_results: Some(true),
            more_results_in_region: Some(!region_hecha),
            results,
            ..ScanResponse::default()
        }
    }

    fn get(&self, p: GetRequest) -> Result<GetResponse, (&'static str, String, bool)> {
        if p.region.value == b"hbase:meta,,1" {
            // el estado de la tabla: fila `t`, familia `table`, cualificador `state`
            let pb = TableState {
                state: self.estado as i32,
            };
            let cell = Cell {
                row: Some(TABLA.as_bytes().to_vec()),
                family: Some(b"table".to_vec()),
                qualifier: Some(b"state".to_vec()),
                timestamp: Some(1),
                cell_type: Some(CellType::Put as i32),
                // como en HBase 2.4.18: el protobuf pelado, sin la marca `PBUF` (medido en el laboratorio)
                value: Some(pb.encode_to_vec()),
                tags: None,
            };
            return Ok(GetResponse {
                result: Some(ResultadoPb {
                    cell: vec![cell],
                    associated_cell_count: None,
                    exists: None,
                    stale: None,
                    partial: Some(false),
                }),
            });
        }
        Ok(GetResponse::default())
    }
}

/// El cluster simulado.
pub struct Sim {
    pub zk: SocketAddr,
    pub rpc: SocketAddr,
    pub mundo: Arc<Mutex<Mundo>>,
}

impl Sim {
    /// `n_filas` filas repartidas en `regiones` regiones de igual tamano, y un guion que decide, llamada a
    /// llamada, que hace el servidor.
    pub fn nuevo(
        n_filas: usize,
        regiones: usize,
        guion: impl FnMut(&Llamada, &mut Mundo) -> Accion + Send + 'static,
    ) -> Sim {
        Sim::con_filas(
            (0..n_filas).map(clave).collect(),
            regiones,
            celdas_de_texto(),
            guion,
        )
    }

    /// Lo mismo con las claves y las celdas que se den: `filas` **ordenadas por bytes** (las de una clave entera
    /// codificada, los negativos van despues de los positivos), partidas en `regiones` regiones por su numero.
    pub fn con_filas(
        filas: Vec<Vec<u8>>,
        regiones: usize,
        celdas_de: Celdas,
        guion: impl FnMut(&Llamada, &mut Mundo) -> Accion + Send + 'static,
    ) -> Sim {
        assert!(
            filas.windows(2).all(|w| w[0] < w[1]),
            "las claves, ordenadas por bytes y distintas"
        );
        let n_filas = filas.len();
        let mut mundo = Mundo {
            filas,
            celdas_de,
            regiones: Vec::new(),
            estado: TableStateState::Enabled,
            caido: false,
            repetir_desde_el_principio: false,
            registro: Vec::new(),
            scanners: HashMap::new(),
            id_scanner: 0,
            generacion: 0,
            llamadas: 0,
            aperturas: 0,
            continuaciones: 0,
            guion: Box::new(guion),
        };
        let por_region = n_filas / regiones;
        for i in 0..regiones {
            let inicio = if i == 0 {
                Vec::new()
            } else {
                mundo.filas[i * por_region].clone()
            };
            let fin = if i + 1 == regiones {
                Vec::new()
            } else {
                mundo.filas[(i + 1) * por_region].clone()
            };
            let nombre = mundo.nombre_de_region(&inicio);
            mundo.regiones.push(RegionSim {
                nombre,
                inicio,
                fin,
                sin_servidor: false,
            });
        }
        let mundo = Arc::new(Mutex::new(mundo));
        let zk = TcpListener::bind("127.0.0.1:0").unwrap();
        let rpc = TcpListener::bind("127.0.0.1:0").unwrap();
        let sim = Sim {
            zk: zk.local_addr().unwrap(),
            rpc: rpc.local_addr().unwrap(),
            mundo: mundo.clone(),
        };
        std::thread::spawn(move || {
            for s in zk.incoming().flatten() {
                std::thread::spawn(move || atender_zk(s));
            }
        });
        std::thread::spawn(move || {
            for s in rpc.incoming().flatten() {
                let m = mundo.clone();
                std::thread::spawn(move || atender_rpc(s, m));
            }
        });
        sim
    }

    pub fn registro(&self) -> Vec<String> {
        self.mundo.lock().unwrap().registro.clone()
    }

    pub fn n_registros(&self, que: &str) -> usize {
        self.registro()
            .iter()
            .filter(|l| l.starts_with(que))
            .count()
    }
}

fn leer(s: &mut TcpStream, n: usize) -> std::io::Result<Vec<u8>> {
    let mut b = vec![0; n];
    s.read_exact(&mut b)?;
    Ok(b)
}

fn trama(cuerpo: &[u8]) -> Vec<u8> {
    [(cuerpo.len() as u32).to_be_bytes().as_slice(), cuerpo].concat()
}

/// ZooKeeper: una sesion, `getData` del nodo de `meta`, y cierre.
fn atender_zk(mut s: TcpStream) {
    let nodo = {
        let m = MetaRegionServer {
            server: ServerName {
                host_name: "rs.sim".into(),
                port: Some(16020),
                start_code: None,
            },
            rpc_version: None,
            state: None,
        };
        [
            &[0xFFu8, 0, 0, 0, 3][..],
            b"sim",
            b"PBUF",
            &m.encode_to_vec(),
        ]
        .concat()
    };
    let leer_trama = |s: &mut TcpStream| -> std::io::Result<Vec<u8>> {
        let l = leer(s, 4)?;
        leer(s, u32::from_be_bytes([l[0], l[1], l[2], l[3]]) as usize)
    };
    if leer_trama(&mut s).is_err() {
        return;
    }
    let mut conexion = Vec::new();
    conexion.extend_from_slice(&0i32.to_be_bytes());
    conexion.extend_from_slice(&30_000i32.to_be_bytes());
    conexion.extend_from_slice(&1i64.to_be_bytes());
    conexion.extend_from_slice(&16i32.to_be_bytes());
    conexion.extend_from_slice(&[0; 16]);
    conexion.push(0);
    if s.write_all(&trama(&conexion)).is_err() {
        return;
    }
    while let Ok(p) = leer_trama(&mut s) {
        let tipo = i32::from_be_bytes([p[4], p[5], p[6], p[7]]);
        if tipo != 4 {
            return;
        }
        let mut r = Vec::new();
        r.extend_from_slice(&p[0..4]); // xid
        r.extend_from_slice(&0i64.to_be_bytes());
        r.extend_from_slice(&0i32.to_be_bytes());
        r.extend_from_slice(&(nodo.len() as i32).to_be_bytes());
        r.extend_from_slice(&nodo);
        r.extend_from_slice(&[0; 68]);
        if s.write_all(&trama(&r)).is_err() {
            return;
        }
    }
}

fn excepcion(id: u32, clase: &str, mensaje: &str, no_reintentar: bool) -> Vec<u8> {
    let cab = ResponseHeader {
        call_id: Some(id),
        exception: Some(ExceptionResponse {
            exception_class_name: Some(clase.into()),
            stack_trace: Some(format!("{clase}: {mensaje}\n\tat sim")),
            hostname: None,
            port: None,
            do_not_retry: Some(no_reintentar),
        }),
        cell_block_meta: None,
    };
    trama(&cab.encode_length_delimited_to_vec())
}

fn respuesta(id: u32, m: &impl Message) -> Vec<u8> {
    let cab = ResponseHeader {
        call_id: Some(id),
        exception: None,
        cell_block_meta: None,
    };
    let mut cuerpo = cab.encode_length_delimited_to_vec();
    cuerpo.extend_from_slice(&m.encode_length_delimited_to_vec());
    trama(&cuerpo)
}

/// Un RegionServer: apertura (preambulo + cabecera) y llamadas.
fn atender_rpc(mut s: TcpStream, mundo: Arc<Mutex<Mundo>>) {
    if mundo.lock().unwrap().caido {
        return; // se cierra al soltar `s`
    }
    let Ok(pre) = leer(&mut s, 10) else { return };
    if leer(
        &mut s,
        u32::from_be_bytes([pre[6], pre[7], pre[8], pre[9]]) as usize,
    )
    .is_err()
    {
        return;
    }
    loop {
        let Ok(l) = leer(&mut s, 4) else { return };
        let Ok(cuerpo) = leer(
            &mut s,
            u32::from_be_bytes([l[0], l[1], l[2], l[3]]) as usize,
        ) else {
            return;
        };
        let mut b = cuerpo.as_slice();
        let Ok(cab) = RequestHeader::decode_length_delimited(&mut b) else {
            return;
        };
        let id = cab.call_id.unwrap_or(0);
        let metodo = cab.method_name.unwrap_or_default();

        let mut m = mundo.lock().unwrap();
        if m.caido {
            return;
        }
        // lo que dice la llamada, para el guion
        let (region, abre, scanner) = match metodo.as_str() {
            "Scan" => {
                let p = ScanRequest::decode_length_delimited(&mut b.to_vec().as_slice()).unwrap();
                let datos = p.scanner_id.is_none()
                    && p.region
                        .as_ref()
                        .is_some_and(|r| r.value != b"hbase:meta,,1");
                (p.region.map(|r| r.value), datos, p.scanner_id)
            }
            _ => (None, false, None),
        };
        let es_de_datos = metodo == "Scan" && region.as_ref().is_none_or(|r| r != b"hbase:meta,,1");
        if es_de_datos {
            m.llamadas += 1;
            if abre {
                m.aperturas += 1;
                m.continuaciones = 0;
            } else if scanner.is_some() {
                m.continuaciones += 1;
            }
        } else if metodo == "Scan" {
            m.registro.push("meta".to_string());
        }
        let llamada = Llamada {
            n: m.llamadas,
            metodo: metodo.clone(),
            region: region.clone(),
            abre,
            aperturas: m.aperturas,
            continuaciones: m.continuaciones,
        };
        let accion = if es_de_datos {
            let mut guion = std::mem::replace(&mut m.guion, Box::new(|_, _| Accion::Normal));
            let a = guion(&llamada, &mut m);
            m.guion = guion;
            a
        } else {
            Accion::Normal
        };
        match accion {
            Accion::CortarConexion => {
                m.registro
                    .push(format!("conexion cortada en la llamada {}", llamada.n));
                return;
            }
            Accion::Dormir(d) => {
                drop(m);
                std::thread::sleep(d);
                continue_con(&mundo, &mut s, id, &metodo, b);
                continue;
            }
            Accion::Excepcion {
                clase,
                mensaje,
                no_reintentar,
            } => {
                m.registro
                    .push(format!("excepcion {clase} en la llamada {}", llamada.n));
                if s.write_all(&excepcion(id, clase, mensaje, no_reintentar))
                    .is_err()
                {
                    return;
                }
                continue;
            }
            Accion::Latido => {
                m.registro
                    .push(format!("latido en la llamada {}", llamada.n));
                let r = ScanResponse {
                    scanner_id: scanner,
                    more_results: Some(true),
                    more_results_in_region: Some(true),
                    heartbeat_message: Some(true),
                    ..ScanResponse::default()
                };
                if s.write_all(&respuesta(id, &r)).is_err() {
                    return;
                }
                continue;
            }
            Accion::Normal => {}
        }
        drop(m);
        continue_con(&mundo, &mut s, id, &metodo, b);
    }
}

/// Ejecuta la llamada tal cual y la contesta.
fn continue_con(mundo: &Arc<Mutex<Mundo>>, s: &mut TcpStream, id: u32, metodo: &str, mut b: &[u8]) {
    let mut m = mundo.lock().unwrap();
    let r = match metodo {
        "Scan" => {
            let p = ScanRequest::decode_length_delimited(&mut b).unwrap();
            if let Some(reg) = &p.region
                && reg.value != b"hbase:meta,,1"
            {
                m.registro.push(format!(
                    "abre {} desde {}",
                    String::from_utf8_lossy(&reg.value),
                    p.scan
                        .as_ref()
                        .and_then(|s| s.start_row.clone())
                        .map_or_else(String::new, |k| String::from_utf8_lossy(&k).into_owned())
                ));
            }
            m.scan(p).map(|r| respuesta(id, &r))
        }
        "Get" => {
            let p = GetRequest::decode_length_delimited(&mut b).unwrap();
            m.get(p).map(|r| respuesta(id, &r))
        }
        otro => Err((
            "org.apache.hadoop.hbase.exceptions.UnknownProtocolException",
            format!("metodo {otro}"),
            true,
        )),
    };
    let bytes = match r {
        Ok(b) => b,
        Err((clase, mensaje, no_reintentar)) => {
            m.registro.push(format!("excepcion {clase}"));
            excepcion(id, clase, &mensaje, no_reintentar)
        }
    };
    let _ = s.write_all(&bytes);
}
