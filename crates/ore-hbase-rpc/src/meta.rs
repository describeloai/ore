//! **Donde esta cada cosa**: el servidor de `hbase:meta` (que dice ZooKeeper) y, leyendo `hbase:meta`, las
//! regiones de una tabla y el RegionServer que sirve cada una.
//!
//! Lo que X0 midio y nadie avisa (ficha 0072, §3.2): `hbase:meta` **no se ordena por bytes**. Un `Scan` que
//! empieza en `tabla,` se salta en silencio la region de clave vacia (devolvio 3 regiones de 4: la lectura
//! perdia el 25 % de los datos sin ningun error). Hay que empezar en `tabla,,`.

use crate::error::{Error, Resultado};
use crate::proto::{MetaRegionServer, RegionInfo, TableState, TableStateState};
use crate::rpc::{Conexion, Opciones, Servidor};
use crate::scan::{Consulta, Fila};
use crate::zk::{self, Direccion};
use prost::Message;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// El nombre de la region de `hbase:meta`.
pub const REGION_DE_META: &[u8] = b"hbase:meta,,1";
/// `PBUF`: los datos de ZooKeeper y de `hbase:meta` van en protobuf precedidos de esta marca.
const MAGIA: &[u8] = b"PBUF";

/// Una region de una tabla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// El nombre completo (`tabla,inicio,marca.codificado.`): lo que el protocolo pide.
    pub nombre: Vec<u8>,
    /// Primera clave de la region, inclusive. Vacia en la primera.
    pub inicio: Vec<u8>,
    /// Primera clave de la region siguiente, exclusiva. Vacia en la ultima.
    pub fin: Vec<u8>,
    /// Quien la sirve.
    pub servidor: Servidor,
}

/// Lo que dice el nodo `meta-region-server` de ZooKeeper: `0xFF`, la longitud de un identificador, el
/// identificador, `PBUF` y el protobuf.
pub fn servidor_de_meta_desde(datos: &[u8]) -> Resultado<Servidor> {
    if datos.first() != Some(&0xFF) || datos.len() < 5 {
        return Err(Error::Protocolo(
            "el nodo de meta no empieza por la marca 0xFF".into(),
        ));
    }
    let n = u32::from_be_bytes([datos[1], datos[2], datos[3], datos[4]]) as usize;
    let resto = datos
        .get(5 + n..)
        .ok_or_else(|| Error::Protocolo("el nodo de meta es mas corto de lo que anuncia".into()))?;
    let pb = resto
        .strip_prefix(MAGIA)
        .ok_or_else(|| Error::Protocolo("el nodo de meta no lleva la marca PBUF".into()))?;
    let m = MetaRegionServer::decode(pb)
        .map_err(|e| Error::Protocolo(format!("MetaRegionServer: {e}")))?;
    Ok(Servidor {
        host: m.server.host_name,
        puerto: u16::try_from(m.server.port.unwrap_or(0)).unwrap_or(0),
    })
}

/// `default:t` y `t` son la misma tabla; en `hbase:meta` la del espacio por defecto va sin prefijo.
pub fn nombre_en_meta(tabla: &str) -> Resultado<String> {
    let t = tabla.trim();
    if t.is_empty() {
        return Err(Error::Peticion("un nombre de tabla vacio".into()));
    }
    Ok(t.strip_prefix("default:").unwrap_or(t).to_string())
}

/// La region de una fila de `hbase:meta`; `None` si esa fila no es una region (el estado de la tabla, una
/// region cortada en dos que ya no se sirve).
fn region_de(fila: &Fila, esperada: &str) -> Resultado<Option<Region>> {
    let valor = |q: &[u8]| fila.valor(b"info", q);
    let Some(ri) = valor(b"regioninfo") else {
        return Ok(None);
    };
    let pb = ri
        .strip_prefix(MAGIA)
        .ok_or_else(|| Error::Protocolo("info:regioninfo sin la marca PBUF".into()))?;
    let info = RegionInfo::decode(pb).map_err(|e| Error::Protocolo(format!("RegionInfo: {e}")))?;
    let tabla = info.table_name;
    let ns = String::from_utf8_lossy(&tabla.namespace);
    let q = String::from_utf8_lossy(&tabla.qualifier);
    let completo = if ns == "default" {
        q.to_string()
    } else {
        format!("{ns}:{q}")
    };
    if completo != esperada || info.offline == Some(true) || info.split == Some(true) {
        return Ok(None);
    }
    let nombre = fila.clave.clone();
    let Some(s) = valor(b"server") else {
        return Err(Error::RegionSinServidor(
            String::from_utf8_lossy(&nombre).into_owned(),
        ));
    };
    let s = String::from_utf8_lossy(s);
    let (host, puerto) = s
        .rsplit_once(':')
        .and_then(|(h, p)| Some((h, p.parse::<u16>().ok()?)))
        .ok_or_else(|| Error::Protocolo(format!("info:server `{s}` no es host:puerto")))?;
    Ok(Some(Region {
        nombre,
        inicio: info.start_key.unwrap_or_default(),
        fin: info.end_key.unwrap_or_default(),
        servidor: Servidor {
            host: host.to_string(),
            puerto,
        },
    }))
}

/// Una tabla tal como la cuenta `hbase:meta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablaEnMeta {
    /// El espacio de nombres (`default`, `ventas`…).
    pub espacio: String,
    pub nombre: String,
    /// Cuantas regiones sirve (las partidas y fuera de linea no cuentan).
    pub regiones: usize,
    /// `None` si `hbase:meta` no lleva su estado.
    pub estado: Option<TableStateState>,
}

/// El punto de entrada: ZooKeeper dice donde esta `hbase:meta`, y `hbase:meta` donde esta todo lo demas.
#[derive(Debug, Clone)]
pub struct Cliente {
    quorum: Vec<Direccion>,
    znode: String,
    opciones: Opciones,
    /// Las regiones de cada tabla, tal como `hbase:meta` las dijo la ultima vez. Se comparte entre los clones
    /// (los lectores en paralelo) y se tira cuando un servidor dice que ya no es asi.
    cache: Arc<Mutex<HashMap<String, Arc<Vec<Region>>>>>,
}

impl Cliente {
    /// `zk`: `a:2181,b:2181`. `znode`: la raiz de HBase en ZooKeeper (`/hbase` por defecto).
    pub fn nuevo(zk: &str, znode: &str, opciones: Opciones) -> Resultado<Cliente> {
        Ok(Cliente {
            quorum: zk::quorum(zk)?,
            znode: znode.trim_end_matches('/').to_string(),
            opciones,
            cache: Arc::default(),
        })
    }

    pub fn opciones(&self) -> &Opciones {
        &self.opciones
    }

    /// El servidor que sirve `hbase:meta` ahora.
    pub fn servidor_de_meta(&self) -> Resultado<Servidor> {
        let ruta = format!("{}/meta-region-server", self.znode);
        servidor_de_meta_desde(&zk::leer_nodo(&self.quorum, &ruta, self.opciones.tiempo)?)
    }

    /// Abre una conexion RPC con `s`.
    pub fn conectar(&self, s: &Servidor) -> Resultado<Conexion> {
        Conexion::abrir(s, &self.opciones)
    }

    /// Las regiones de la tabla **desde la cache**; si no estan, de `hbase:meta`.
    pub fn regiones_en_cache(&self, tabla: &str) -> Resultado<Arc<Vec<Region>>> {
        let nombre = nombre_en_meta(tabla)?;
        if let Some(r) = self.cache.lock().unwrap().get(&nombre) {
            return Ok(r.clone());
        }
        self.regiones(tabla)?;
        Ok(self
            .cache
            .lock()
            .unwrap()
            .get(&nombre)
            .cloned()
            .unwrap_or_default())
    }

    /// **Todas las tablas**, leidas de `hbase:meta` (una pasada entera: sus regiones y los estados de las tablas), sin
    /// pasar por el maestro. Incluye las del sistema (`hbase:meta`, `hbase:namespace`…): quien las quiera fuera,
    /// que las filtre por espacio.
    pub fn tablas(&self) -> Resultado<Vec<TablaEnMeta>> {
        let mut meta = self.conectar(&self.servidor_de_meta()?)?;
        let consulta = Consulta {
            columnas: vec![
                (b"info".to_vec(), vec![b"regioninfo".to_vec()]),
                (b"table".to_vec(), vec![b"state".to_vec()]),
            ],
            filas_por_llamada: 500,
            ..Consulta::default()
        };
        let mut regiones: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut estados: BTreeMap<(String, String), TableStateState> = BTreeMap::new();
        for fila in meta.escanear(REGION_DE_META, &consulta) {
            let fila = fila?;
            if let Some(ri) = fila.valor(b"info", b"regioninfo") {
                let pb = ri.strip_prefix(MAGIA).unwrap_or(ri);
                let info = RegionInfo::decode(pb)
                    .map_err(|e| Error::Protocolo(format!("RegionInfo: {e}")))?;
                if info.offline != Some(true) && info.split != Some(true) {
                    let t = info.table_name;
                    let clave = (
                        String::from_utf8_lossy(&t.namespace).into_owned(),
                        String::from_utf8_lossy(&t.qualifier).into_owned(),
                    );
                    *regiones.entry(clave).or_default() += 1;
                }
            } else if let Some(st) = fila.valor(b"table", b"state") {
                // la fila de una tabla se llama como ella: `tabla` en el espacio por defecto, `espacio:tabla` en otro
                let nombre = String::from_utf8_lossy(&fila.clave).into_owned();
                let clave = match nombre.split_once(':') {
                    Some((e, t)) => (e.to_string(), t.to_string()),
                    None => ("default".to_string(), nombre),
                };
                let pb = st.strip_prefix(MAGIA).unwrap_or(st);
                if let Ok(e) = TableState::decode(pb)
                    && let Ok(e) = TableStateState::try_from(e.state)
                {
                    estados.insert(clave, e);
                }
            }
        }
        Ok(regiones
            .into_iter()
            .map(|((espacio, nombre), n)| TablaEnMeta {
                estado: estados.get(&(espacio.clone(), nombre.clone())).copied(),
                espacio,
                nombre,
                regiones: n,
            })
            .collect())
    }

    /// Tira lo que se sabe de la tabla: la siguiente lectura vuelve a preguntar a `hbase:meta`.
    pub fn invalidar(&self, tabla: &str) {
        if let Ok(n) = nombre_en_meta(tabla) {
            self.cache.lock().unwrap().remove(&n);
        }
    }

    /// El estado de la tabla en `hbase:meta` (fila de la tabla, familia `table`, cualificador `state`); `None` si no
    /// figura. Sirve para distinguir una tabla deshabilitada (no se arregla esperando) de una region que se mueve.
    pub fn estado_de_tabla(&self, tabla: &str) -> Resultado<Option<TableStateState>> {
        let nombre = nombre_en_meta(tabla)?;
        let mut meta = self.conectar(&self.servidor_de_meta()?)?;
        let col = [(b"table".to_vec(), vec![b"state".to_vec()])];
        let Some(f) = meta.leer_fila(REGION_DE_META, nombre.as_bytes(), &col)? else {
            return Ok(None);
        };
        let Some(v) = f.valor(b"table", b"state") else {
            return Ok(None);
        };
        // Medido contra HBase 2.4.18: a diferencia de `info:regioninfo`, el valor de `table:state` es el protobuf
        // pelado, sin la marca `PBUF`. Se admite con ella por si otra version la pone.
        let pb = v.strip_prefix(MAGIA).unwrap_or(v);
        let e = TableState::decode(pb).map_err(|e| Error::Protocolo(format!("TableState: {e}")))?;
        Ok(TableStateState::try_from(e.state).ok())
    }

    /// Las regiones de la tabla, en orden de clave, **preguntando a `hbase:meta`** (y renovando la cache).
    pub fn regiones(&self, tabla: &str) -> Resultado<Vec<Region>> {
        let nombre = nombre_en_meta(tabla)?;
        let mut meta = self.conectar(&self.servidor_de_meta()?)?;
        // `tabla,,` y no `tabla,`: ver la cabecera. El final, `tabla-`, es `tabla,` con el ultimo byte + 1.
        let consulta = Consulta {
            inicio: format!("{nombre},,").into_bytes(),
            fin: format!("{nombre}-").into_bytes(),
            ..Consulta::default()
        };
        let mut regiones = Vec::new();
        for fila in meta.escanear(REGION_DE_META, &consulta) {
            if let Some(r) = region_de(&fila?, &nombre)? {
                regiones.push(r);
            }
        }
        if regiones.is_empty() {
            return Err(Error::TablaSinRegiones(nombre));
        }
        regiones.sort_by(|a, b| a.inicio.cmp(&b.inicio));
        self.cache
            .lock()
            .unwrap()
            .insert(nombre, Arc::new(regiones.clone()));
        Ok(regiones)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ServerName;

    fn nodo(host: &str, puerto: u32) -> Vec<u8> {
        let m = MetaRegionServer {
            server: ServerName {
                host_name: host.into(),
                port: Some(puerto),
                start_code: None,
            },
            rpc_version: None,
            state: None,
        };
        let mut v = vec![0xFF, 0, 0, 0, 3];
        v.extend_from_slice(b"abc");
        v.extend_from_slice(MAGIA);
        v.extend_from_slice(&m.encode_to_vec());
        v
    }

    #[test]
    fn el_nodo_de_meta_dice_host_y_puerto() {
        let s = servidor_de_meta_desde(&nodo("rs1.hadoop.local", 16020)).unwrap();
        assert_eq!(
            s,
            Servidor {
                host: "rs1.hadoop.local".into(),
                puerto: 16020
            }
        );
    }

    #[test]
    fn un_nodo_que_no_es_de_hbase_se_rechaza_sin_panico() {
        assert!(servidor_de_meta_desde(&[]).is_err());
        assert!(servidor_de_meta_desde(&[0xFF, 0, 0, 0, 99, 1]).is_err());
        let mut sin_magia = nodo("h", 1);
        sin_magia[8] = b'X';
        assert!(servidor_de_meta_desde(&sin_magia).is_err());
    }

    #[test]
    fn la_tabla_del_espacio_por_defecto_va_sin_prefijo() {
        assert_eq!(nombre_en_meta("default:t").unwrap(), "t");
        assert_eq!(nombre_en_meta("t").unwrap(), "t");
        assert_eq!(nombre_en_meta("ventas:t").unwrap(), "ventas:t");
        assert!(nombre_en_meta(" ").is_err());
    }
}
