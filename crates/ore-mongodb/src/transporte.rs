//! **El transporte**: abrir, preguntar, muestrear, leer, cancelar (ficha 0069
//! M1). Lo único de este crate que abre un socket.
//!
//! Medido en X0 (§3 de la ficha), y lo que se hace con ello:
//!
//! - **Solo lectura la da el rol**, no la sesión: no hay `SET … READ ONLY`. El
//!   conector no escribe porque no tiene código que escriba; `Conexion` solo
//!   expone lecturas.
//! - **`$sample` choca con los 32 MB de ordenación de un M0** si los documentos
//!   son grandes (10 000 de ~24 KB dan el error 292, también con
//!   `allowDiskUse`): el tamaño de la muestra se limita por
//!   `32 MB / tamaño medio` ([`TOPE_ORDENACION`]).
//! - **Cancelar**: cerrar el cursor libera el del servidor; `maxTimeMS` corta en
//!   el origen. Medido contra un `mongo:7`: `killOp` termina la consulta con el
//!   error 11601 en 20 ms.
//! - **Los errores** se mapean al vocabulario del conector ([`fallo_de`]): 50 es
//!   `tiempo`, 13 y 18 `credencial`, 26 `objeto`, una caída de red `conexion`.
//!   Nada de lo que dicen lleva la URL: quien los recibe la tapa
//!   ([`ore_driver::Fallo::tapado`]).

use crate::traductor::Plan;
use mongodb::bson::raw::RawDocument;
use mongodb::bson::{Bson, Document, RawDocumentBuf, Timestamp, doc};
use mongodb::change_stream::event::OperationType;
use mongodb::error::{Error, ErrorKind};
use mongodb::options::FullDocumentType;
use mongodb::sync::Client;
use ore_driver::{Codigo, Fallo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// El límite de memoria de un `$sort`/`$sample` en un M0 (32 MiB).
pub const TOPE_ORDENACION: u64 = 32 * 1024 * 1024;

/// Documentos por lote que se piden al servidor.
pub const LOTE: u32 = 8192;

/// Lo que `listCollections` dice de una colección.
#[derive(Debug, Clone, PartialEq)]
pub struct Coleccion {
    pub nombre: String,
    pub tipo: TipoColeccion,
    /// El `$jsonSchema` del validador, si lo tiene: manda sobre el muestreo.
    pub validador: Option<Document>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoColeccion {
    Coleccion,
    Vista,
    SerieTemporal,
}

/// Una muestra de una colección.
#[derive(Debug)]
pub struct Muestra {
    pub documentos: Vec<RawDocumentBuf>,
    /// Cuántos documentos tiene la colección (estimado) y cuánto pesa uno.
    pub total: u64,
    pub tamano_medio: u64,
    /// Cuántos se pidieron, y cuántos se habrían pedido sin el tope de memoria.
    pub pedidos: u64,
    pub recortada_por_memoria: bool,
}

/// Cómo terminó una lectura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Salida {
    pub filas: u64,
    pub cancelada: bool,
}

#[derive(Clone)]
pub struct Conexion {
    cliente: Client,
}

impl Conexion {
    /// Abre el cliente. No conecta hasta la primera operación: `ping` es la que
    /// dice si se llega.
    pub fn abrir(url: &str) -> Result<Conexion, Fallo> {
        crate::url::validar(url).map_err(|m| Fallo::new(Codigo::Credencial, m))?;
        let mut opciones = mongodb::options::ClientOptions::parse(url)
            .run()
            .map_err(|e| fallo_de(&e))?;
        if opciones.app_name.is_none() {
            // Distinto a cada cliente: el origen cuenta así las sesiones del conector
            // (un cliente abre varias conexiones).
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            opciones.app_name = Some(format!(
                "ore-read-mongodb/{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
        }
        let cliente = Client::with_options(opciones).map_err(|e| fallo_de(&e))?;
        Ok(Conexion { cliente })
    }

    fn comando(&self, base: &str, c: Document) -> Result<Document, Fallo> {
        self.cliente
            .database(base)
            .run_command(c)
            .run()
            .map_err(|e| fallo_de(&e))
    }

    /// `ping` al servidor: cuánto tarda, o por qué no se llega.
    pub fn ping(&self) -> Result<Duration, Fallo> {
        let t = Instant::now();
        self.comando("admin", doc! {"ping": 1})?;
        Ok(t.elapsed())
    }

    /// La versión del servidor y si es un replica set (si hay change streams).
    pub fn servidor(&self) -> Result<(String, Option<String>), Fallo> {
        let b = self.comando("admin", doc! {"buildInfo": 1})?;
        let version = b.get_str("version").unwrap_or("?").to_string();
        let h = self.comando("admin", doc! {"hello": 1})?;
        Ok((version, h.get_str("setName").ok().map(String::from)))
    }

    /// Las bases de usuario (sin `admin`, `local` ni `config`).
    pub fn bases(&self) -> Result<Vec<String>, Fallo> {
        let r = self.comando("admin", doc! {"listDatabases": 1, "nameOnly": true})?;
        let mut v: Vec<String> = r
            .get_array("databases")
            .map(|a| {
                a.iter()
                    .filter_map(|d| {
                        d.as_document()
                            .and_then(|d| d.get_str("name").ok())
                            .map(String::from)
                    })
                    .filter(|n| !matches!(n.as_str(), "admin" | "local" | "config"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        Ok(v)
    }

    /// Las colecciones y vistas de una base, sin `system.*` (ficha 0069 §4.8).
    pub fn colecciones(&self, base: &str) -> Result<Vec<Coleccion>, Fallo> {
        let r = self.comando(base, doc! {"listCollections": 1})?;
        let lote = r
            .get_document("cursor")
            .and_then(|c| c.get_array("firstBatch"))
            .map_err(|_| Fallo::origen("`listCollections` contestó sin `cursor.firstBatch`"))?;
        let mut v = Vec::new();
        for d in lote.iter().filter_map(Bson::as_document) {
            let nombre = d.get_str("name").unwrap_or_default().to_string();
            if nombre.is_empty() || nombre.starts_with("system.") {
                continue;
            }
            let tipo = match d.get_str("type").unwrap_or("collection") {
                "view" => TipoColeccion::Vista,
                "timeseries" => TipoColeccion::SerieTemporal,
                _ => TipoColeccion::Coleccion,
            };
            let validador = d
                .get_document("options")
                .and_then(|o| o.get_document("validator"))
                .and_then(|v| v.get_document("$jsonSchema"))
                .ok()
                .cloned();
            v.push(Coleccion {
                nombre,
                tipo,
                validador,
            });
        }
        v.sort_by(|a, b| a.nombre.cmp(&b.nombre));
        Ok(v)
    }

    /// Una muestra de `n` documentos. Con pocos documentos (o menos que `n`) se
    /// lee con `find`; si no, con `$sample`, limitada por la memoria.
    pub fn muestra(
        &self,
        base: &str,
        coleccion: &str,
        n: u64,
        max_time: Duration,
        comment: Option<&str>,
    ) -> Result<Muestra, Fallo> {
        let col = self
            .cliente
            .database(base)
            .collection::<RawDocumentBuf>(coleccion);
        // Una vista no tiene `collStats`: se queda con el tamaño por defecto.
        let (total, medio) = match self.comando(base, doc! {"collStats": coleccion}) {
            Ok(s) => (num(&s, "count"), num(&s, "avgObjSize").max(1)),
            Err(_) => (0, 1024),
        };
        let tope = (TOPE_ORDENACION * 6 / 10 / medio).max(1);
        let pedidos = n.min(tope);
        let recortada = pedidos < n;
        let mut documentos = Vec::new();
        if total != 0 && total <= pedidos {
            let mut b = col.find(doc! {}).limit(pedidos as i64).max_time(max_time);
            if let Some(c) = comment {
                b = b.comment(c);
            }
            let cur = b.run().map_err(|e| fallo_de(&e))?;
            for d in cur {
                documentos.push(d.map_err(|e| fallo_de(&e))?);
            }
        } else {
            let tubo = vec![doc! {"$sample": {"size": pedidos as i64}}];
            let mut b = col
                .aggregate(tubo)
                .max_time(max_time)
                .allow_disk_use(true)
                .with_type::<RawDocumentBuf>();
            if let Some(c) = comment {
                b = b.comment(c);
            }
            let cur = b.run().map_err(|e| fallo_de(&e))?;
            for d in cur {
                documentos.push(d.map_err(|e| fallo_de(&e))?);
            }
        }
        Ok(Muestra {
            documentos,
            total,
            tamano_medio: medio,
            pedidos,
            recortada_por_memoria: recortada,
        })
    }

    /// Lee lo que dice el plan y entrega cada documento a `f`, de uno en uno
    /// (el cursor trae `LOTE` por viaje). Si `cancelar` se activa, cierra el
    /// cursor —lo que libera el del servidor— y termina con `cancelada`.
    pub fn ejecutar(
        &self,
        base: &str,
        coleccion: &str,
        plan: &Plan,
        cancelar: &AtomicBool,
        comment: Option<&str>,
        f: &mut dyn FnMut(&RawDocument) -> Result<(), Fallo>,
    ) -> Result<Salida, Fallo> {
        let col = self
            .cliente
            .database(base)
            .collection::<RawDocumentBuf>(coleccion);
        let mut filas = 0_u64;
        let mut consumir = |cur: &mut dyn Iterator<Item = Result<RawDocumentBuf, Error>>| -> Result<Salida, Fallo> {
            for d in cur {
                if cancelar.load(Ordering::Relaxed) {
                    return Ok(Salida { filas, cancelada: true });
                }
                let d = d.map_err(|e| fallo_de(&e))?;
                f(&d)?;
                filas += 1;
            }
            Ok(Salida { filas, cancelada: false })
        };
        match plan {
            Plan::Find {
                filtro,
                proyeccion,
                limit,
                max_time,
            } => {
                let mut b = col
                    .find(filtro.clone())
                    .projection(proyeccion.clone())
                    .batch_size(LOTE);
                if let Some(n) = limit {
                    b = b.limit(*n);
                }
                if let Some(t) = max_time {
                    b = b.max_time(*t);
                }
                if let Some(c) = comment {
                    b = b.comment(c);
                }
                let mut cur = b.run().map_err(|e| fallo_de(&e))?;
                consumir(&mut cur)
            }
            Plan::Agregado { tubo, max_time } => {
                let mut b = col
                    .aggregate(tubo.clone())
                    .batch_size(LOTE)
                    .with_type::<RawDocumentBuf>();
                if let Some(t) = max_time {
                    b = b.max_time(*t);
                }
                if let Some(c) = comment {
                    b = b.comment(c);
                }
                let mut cur = b.run().map_err(|e| fallo_de(&e))?;
                consumir(&mut cur)
            }
        }
    }

    /// **Cancelar en el origen**: busca con `currentOp` las operaciones **propias**
    /// marcadas con `comment` y les manda `killOp`. Devuelve cuántas mató. Medido:
    /// contra un `mongo:7` la consulta termina con el error 11601 en ~20 ms, y
    /// contra un Atlas con un usuario **de solo lectura** también (0,1 s): un
    /// usuario puede matar sus propias operaciones sin el privilegio `killOp`.
    ///
    /// Dos cosas medidas en Atlas que no se ven en un servidor local sin
    /// autenticación:
    ///
    /// - **`$ownOps: true`**: sin él, un usuario sin el privilegio `inprog` no ve
    ///   ni sus operaciones.
    /// - **El comentario se compara aquí, no en el filtro del comando**: la
    ///   igualdad sobre `command.comment` no casa (`$exists` sí), y en la fase de
    ///   `getMore` el comentario puede estar en `originatingCommand`.
    pub fn matar(&self, comment: &str) -> Result<usize, Fallo> {
        let r = self.comando(
            "admin",
            doc! {"currentOp": 1, "$ownOps": true, "active": true},
        )?;
        let marcada = |op: &Document| {
            ["command", "originatingCommand"].iter().any(|k| {
                op.get_document(k)
                    .is_ok_and(|c| c.get_str("comment") == Ok(comment))
            })
        };
        let mut muertas = 0;
        for op in r.get_array("inprog").map(|a| a.as_slice()).unwrap_or(&[]) {
            let Some(op) = op.as_document().filter(|d| marcada(d)) else {
                continue;
            };
            if let Some(id) = op.get("opid") {
                self.comando("admin", doc! {"killOp": 1, "op": id.clone()})?;
                muertas += 1;
            }
        }
        Ok(muertas)
    }

    /// El testigo del origen (0008, el verbo `testigo`): el `operationTime` del
    /// replica set, que es ordinal y no se empata, escrito con ancho fijo
    /// ([`testigo_texto`]) para que comparar el texto sea comparar el instante.
    /// Un servidor suelto no lo tiene y contesta `None`: «no sé fecharme» es una
    /// respuesta válida.
    pub fn testigo(&self) -> Result<Option<String>, Fallo> {
        Ok(self.ahora()?.map(testigo_texto))
    }

    /// El instante del cluster ahora (`operationTime` de `hello`), si lo tiene.
    pub fn ahora(&self) -> Result<Option<Timestamp>, Fallo> {
        let h = self.comando("admin", doc! {"hello": 1})?;
        Ok(match h.get("operationTime") {
            Some(Bson::Timestamp(t)) => Some(*t),
            _ => None,
        })
    }

    /// Si este servidor tiene change streams: un replica set o un clúster con
    /// shards (`mongos`). Un servidor suelto no los tiene.
    pub fn admite_cambios(&self) -> Result<bool, Fallo> {
        let h = self.comando("admin", doc! {"hello": 1})?;
        Ok(h.get_str("setName").is_ok() || h.get_str("msg") == Ok("isdbgrid"))
    }

    /// **Abre los cambios de una colección desde una posición** (ficha 0069 M4): el
    /// change stream abierto en `desde`, que es **exclusivo** (un cambio en
    /// `desde` ya se entregó). Es aparte de leerlo para que lo que falla al abrir
    /// —una posición que el oplog ya no tiene, un permiso— falle **antes del
    /// primer byte**, y no con la cabecera del flujo ya escrita.
    ///
    /// Un servidor solo da `ChangeStreamHistoryLost` si su oplog **se truncó** de
    /// verdad (medido: en un servidor nuevo, una posición de 1970 no falla porque
    /// no se perdió nada). Cuando lo da, no se inventa continuidad: el error dice
    /// que hay que copiar entera (la regla de Debezium y Snowflake, ADR 0016).
    pub fn abrir_cambios(
        &self,
        base: &str,
        coleccion: &str,
        desde: Timestamp,
    ) -> Result<Cambios, Fallo> {
        let col = self
            .cliente
            .database(base)
            .collection::<RawDocumentBuf>(coleccion);
        let cs = col
            .watch()
            .full_document(FullDocumentType::UpdateLookup)
            .start_at_operation_time(desde)
            .max_await_time(Duration::from_millis(500))
            .run()
            .map_err(|e| fallo_de(&e))?;
        Ok(Cambios {
            cs,
            desde,
            objeto: format!("{base}.{coleccion}"),
        })
    }
}

/// Los cambios de una colección, abiertos ([`Conexion::abrir_cambios`]).
pub struct Cambios {
    cs: mongodb::sync::ChangeStream<
        mongodb::change_stream::event::ChangeStreamEvent<RawDocumentBuf>,
    >,
    desde: Timestamp,
    objeto: String,
}

impl Cambios {
    /// Lee hasta `hasta` inclusive o hasta alcanzar el presente, lo que ocurra
    /// antes.
    ///
    /// Un alta, una actualización o un reemplazo entregan el documento **tal como
    /// está ahora** (`updateLookup`); un borrado, solo su `_id`. Si el documento
    /// ya no existe cuando se pide, la actualización se salta: su borrado viene
    /// después y lo dirá. Si la colección se borra o se renombra a mitad de rango
    /// no hay continuidad, y falla diciéndolo.
    pub fn leer(
        &mut self,
        hasta: Timestamp,
        cancelar: &AtomicBool,
        f: &mut dyn FnMut(Cambio) -> Result<(), Fallo>,
    ) -> Result<Salida, Fallo> {
        let mut filas = 0_u64;
        loop {
            if cancelar.load(Ordering::Relaxed) {
                return Ok(Salida {
                    filas,
                    cancelada: true,
                });
            }
            // `None`: el lote vino vacío tras esperar, es decir, ya no hay más
            // hasta ahora. `hasta` se tomó antes, así que ahora ya lo cubre.
            let Some(ev) = self.cs.next_if_any().map_err(|e| fallo_de(&e))? else {
                break;
            };
            let Some(ct) = ev.cluster_time else {
                return Err(Fallo::origen(
                    "un evento del change stream sin `clusterTime`",
                ));
            };
            if !posterior(&ct, &self.desde) {
                continue;
            }
            if posterior(&ct, &hasta) {
                break;
            }
            match ev.operation_type {
                OperationType::Insert | OperationType::Update | OperationType::Replace => {
                    if let Some(d) = ev.full_document {
                        f(Cambio::Alta(d))?;
                        filas += 1;
                    }
                }
                OperationType::Delete => {
                    let clave = ev
                        .document_key
                        .ok_or_else(|| Fallo::origen("un borrado sin `documentKey`"))?;
                    let d = RawDocumentBuf::from_document(&clave)
                        .map_err(|e| Fallo::origen(e.to_string()))?;
                    f(Cambio::Baja(d))?;
                    filas += 1;
                }
                OperationType::Drop
                | OperationType::Rename
                | OperationType::DropDatabase
                | OperationType::Invalidate => {
                    return Err(Fallo::new(
                        Codigo::Objeto,
                        format!(
                            "`{}` se borró o se renombró en la posición {}: no hay continuidad, hay que copiarla entera",
                            self.objeto,
                            testigo_texto(ct)
                        ),
                    ));
                }
                _ => {}
            }
        }
        Ok(Salida {
            filas,
            cancelada: false,
        })
    }
}

/// Un cambio de una colección: el documento como está, o la clave del borrado.
pub enum Cambio {
    Alta(RawDocumentBuf),
    /// `{_id: …}` y nada más.
    Baja(RawDocumentBuf),
}

/// El instante `a` es estrictamente posterior a `b`.
fn posterior(a: &Timestamp, b: &Timestamp) -> bool {
    (a.time, a.increment) > (b.time, b.increment)
}

/// Una posición, con **ancho fijo** (`segundos:incremento`, diez cifras cada
/// uno): así comparar los textos es comparar los instantes, que es lo que el
/// testigo promete ser (un ordinal que se compara, no se interpreta).
pub fn testigo_texto(t: Timestamp) -> String {
    format!("{:010}:{:010}", t.time, t.increment)
}

/// Lo contrario de [`testigo_texto`]. Una posición que no es de este origen es un
/// error del que pide (`operador`), no un «desde el principio».
pub fn testigo_de(texto: &str) -> Result<Timestamp, Fallo> {
    let mal = || {
        Fallo::operador(format!(
            "`{texto}` no es una posición de MongoDB (`segundos:incremento`, la que da el verbo `testigo`)"
        ))
    };
    let (t, i) = texto.split_once(':').ok_or_else(mal)?;
    Ok(Timestamp {
        time: t.parse().map_err(|_| mal())?,
        increment: i.parse().map_err(|_| mal())?,
    })
}

fn num(d: &Document, k: &str) -> u64 {
    match d.get(k) {
        Some(Bson::Int32(n)) => (*n).max(0) as u64,
        Some(Bson::Int64(n)) => (*n).max(0) as u64,
        Some(Bson::Double(x)) => x.max(0.0) as u64,
        _ => 0,
    }
}

/// Un error del cliente, en el vocabulario del conector (`docs/federation.md`
/// §1.3). El mensaje no lleva la URL: la tapa quien lo emite.
pub fn fallo_de(e: &Error) -> Fallo {
    match &*e.kind {
        ErrorKind::Authentication { message, .. } => Fallo::new(
            Codigo::Credencial,
            format!("MongoDB rechazó la credencial: {message}"),
        ),
        ErrorKind::ServerSelection { message, .. } => Fallo::new(
            Codigo::Conexion,
            format!("no se llega al servidor de MongoDB: {message}"),
        ),
        ErrorKind::DnsResolve { message, .. } => Fallo::new(
            Codigo::Conexion,
            format!("no se resolvió el nombre del servidor de MongoDB: {message}"),
        ),
        ErrorKind::Io(io) => Fallo::new(
            Codigo::Conexion,
            format!("la conexión con MongoDB falló: {io}"),
        ),
        ErrorKind::Command(c) => fallo_de_comando(c.code, &c.code_name, &c.message),
        _ => Fallo::new(Codigo::Origen, e.to_string()),
    }
}

/// El error de un comando, por su código. Aparte de [`fallo_de`] para poder
/// probarlo sin un servidor que lo provoque.
pub fn fallo_de_comando(code: i32, code_name: &str, message: &str) -> Fallo {
    let m = format!("{message} (código {code}, {code_name})");
    match code {
        // ExecutionTimeout: `maxTimeMS` se agotó en el origen.
        50 => Fallo::new(Codigo::Tiempo, m),
        // Unauthorized, AuthenticationFailed, y el 8000 de Atlas.
        13 | 18 | 8000 => Fallo::new(Codigo::Credencial, m),
        // NamespaceNotFound.
        26 => Fallo::new(Codigo::Objeto, m),
        // ChangeStreamHistoryLost, ChangeStreamFatalError y CappedPositionLost: la
        // posición pedida ya no está en el oplog. No se inventa continuidad.
        286 | 280 | 136 => Fallo::new(
            Codigo::Origen,
            format!(
                "posición caducada: ya no está en el oplog del origen y hay que copiar entera ({m})"
            ),
        ),
        // Interrupted: alguien hizo `killOp`.
        11601 => Fallo::new(Codigo::Origen, format!("la consulta fue interrumpida: {m}")),
        _ => Fallo::new(Codigo::Origen, m),
    }
}
