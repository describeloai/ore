//! **El transporte**: lo único de este crate que abre un socket (ficha 0070).
//!
//! Abrir la sesión, leer el catálogo de `system_schema`, el anillo de tokens y
//! ejecutar un [`Plan`]. El driver es asíncrono; el resto de ORE no, así que la
//! [`Conexion`] lleva su propio runtime y expone funciones síncronas.
//!
//! **M1 deja la lectura en [`Conexion::leer`]**: recoge las filas en memoria, que
//! basta para medir el traductor y el lote contra un servidor de verdad. El flujo
//! Arrow en lotes, con cancelación y tope de memoria, es de `ore-read-cassandra`
//! (M2).
//!
//! # Los errores
//!
//! Los del driver se reducen a los siete códigos del conector (`federation.md`
//! §1.3), medidos en X0 (§3.5): contraseña mala o sin `SELECT` → `credencial`; nadie
//! escucha → `conexion`; la tabla no existe → `objeto`; una columna que no está →
//! `esquema`; un filtro que pide `ALLOW FILTERING` → `operador`; el tiempo del
//! cliente → `tiempo`.

use crate::esquema::{Columna, FilaColumna, FilaIndice, Tabla};
use crate::lote;
use crate::residual::Residual;
use crate::tipos::Cql;
use crate::traductor::{Bind, Modo, Plan};
use crate::url::{self, Destino};
use futures::{StreamExt, TryStreamExt};
use ore_base::tipos::Valor;
use ore_driver::{Codigo, Fallo};
use scylla::client::SelfIdentity;
use scylla::client::execution_profile::ExecutionProfile;
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use scylla::errors::TranslationError;
use scylla::policies::address_translator::{AddressTranslator, UntranslatedPeer};
use scylla::policies::load_balancing::DefaultPolicy;
use scylla::statement::Consistency;
use scylla::value::{CqlDate, CqlTime, CqlTimestamp, CqlTimeuuid, CqlValue, Row};
use std::collections::BTreeSet;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Una fila de `LIST ALL PERMISSIONS`: rol, usuario, recurso y permiso.
type FilaPermiso = (String, String, String, String);

/// Una fila, con una celda por columna de [`Plan::leidas`]; `None` es nulo.
pub type Fila = Vec<Option<CqlValue>>;

/// Filas por ida al servidor. Medido en X0 (§3.3): 5 000 da ~36 % más que 1 000.
pub const PAGINA: i32 = 5000;

/// Hasta cuántos rangos de token se leen a la vez. Medido en X0: con un nodo se
/// satura hacia 4–8.
pub const PARALELO: usize = 4;

/// Filas por trozo que un rango manda al hilo que escribe.
const TROZO: usize = 2000;
/// Trozos que pueden esperar en el canal: con [`TROZO`] y [`PARALELO`] el
/// conector no tiene en vuelo más de unas decenas de miles de filas, pase lo que
/// pase en el servidor (caso 7 del kit: flujo con techo de memoria).
const CANAL: usize = 8;
/// Cada cuánto el hilo que escribe mira la bandera de cancelar y el reloj.
const LATIDO: Duration = Duration::from_millis(50);

/// Cómo se presenta el conector en `system_views.clients`: el kit cuenta con él
/// las sesiones que hay abiertas (caso 11).
pub const APLICACION: &str = "ore-read-cassandra";

pub struct Conexion {
    rt: tokio::runtime::Runtime,
    sesion: Arc<Session>,
    pub destino: Destino,
}

/// **Solo el punto de contacto**: toda dirección que el clúster anuncia se
/// traduce a la del primer punto de contacto. Tras NAT, un balanceador, Docker o
/// Kubernetes, la dirección que el nodo cree tener no se alcanza (medido en X0).
#[derive(Debug)]
struct SoloContacto(SocketAddr);

#[async_trait::async_trait]
impl AddressTranslator for SoloContacto {
    async fn translate_address(
        &self,
        _: &UntranslatedPeer,
    ) -> Result<SocketAddr, TranslationError> {
        Ok(self.0)
    }
}

fn consistencia(c: &str) -> Consistency {
    match c {
        "ONE" => Consistency::One,
        "TWO" => Consistency::Two,
        "THREE" => Consistency::Three,
        "QUORUM" => Consistency::Quorum,
        "ALL" => Consistency::All,
        "LOCAL_ONE" => Consistency::LocalOne,
        "EACH_QUORUM" => Consistency::EachQuorum,
        _ => Consistency::LocalQuorum,
    }
}

/// El código de conector que le toca a un error del driver. Se lee del texto
/// porque el driver anida sus errores en muchos tipos y lo que importa es la
/// causa, que cada uno dice (medido en X0 §3.5).
pub fn fallo_de(error: &dyn std::fmt::Display, depurado: &dyn std::fmt::Debug) -> Fallo {
    let d = format!("{depurado:?}");
    let msg = error.to_string();
    let ya = |s: &str| d.contains(s) || msg.contains(s);
    let codigo = if ya("AuthenticationError") || ya("MissingAuthentication") || ya("Unauthorized") {
        Codigo::Credencial
    } else if ya("RequestTimeout") || ya("ReadTimeout") || ya("Timed out") {
        Codigo::Tiempo
    } else if ya("ConnectTimeout")
        || ya("ConnectionPoolError")
        || ya("BrokenConnection")
        || ya("IoError")
        || ya("Connection refused")
        || ya("NoConnections")
    {
        Codigo::Conexion
    } else if ya("Undefined column name") {
        Codigo::Esquema
    } else if ya("does not exist") || ya("unconfigured table") || ya("Keyspace") && ya("not exist")
    {
        Codigo::Objeto
    } else if ya("ALLOW FILTERING") || ya("SyntaxError") {
        Codigo::Operador
    } else {
        Codigo::Origen
    };
    Fallo::new(codigo, msg)
}

fn fallo<E: std::fmt::Display + std::fmt::Debug>(e: E) -> Fallo {
    fallo_de(&e, &e)
}

impl Conexion {
    /// Abre la sesión con lo que dice la URL. La credencial no sale en ningún
    /// error: quien lo muestra pasa por [`Fallo::tapado`].
    pub fn abrir(url: &str) -> Result<Conexion, Fallo> {
        Conexion::abrir_como(url, APLICACION)
    }

    /// Lo mismo, presentándose en `system_views.clients` con otro nombre: quien no
    /// es el conector (el banco del kit, una herramienta) no se cuenta entre sus
    /// sesiones.
    pub fn abrir_como(url: &str, aplicacion: &str) -> Result<Conexion, Fallo> {
        let destino = url::analizar(url).map_err(Fallo::operador)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| Fallo::origen(format!("no se pudo crear el runtime: {e}")))?;
        let sesion = rt.block_on(conectar(&destino, aplicacion))?;
        Ok(Conexion {
            rt,
            sesion: Arc::new(sesion),
            destino,
        })
    }

    /// La versión del servidor y su partidor: lo primero que `check` pregunta.
    pub fn servidor(&self) -> Result<(String, String), Fallo> {
        self.rt.block_on(async {
            let r = self
                .sesion
                .query_unpaged("SELECT release_version, partitioner FROM system.local", &[])
                .await
                .map_err(fallo)?
                .into_rows_result()
                .map_err(fallo)?;
            r.single_row::<(String, String)>().map_err(fallo)
        })
    }

    /// Las tablas de un keyspace. **No filtra por permiso** (M2): `system_schema`
    /// lista también las de otros keyspaces a los que el rol no llega (X0 §3.5).
    pub fn tablas(&self, keyspace: &str) -> Result<Vec<String>, Fallo> {
        self.rt.block_on(async {
            let mut v: Vec<String> = self
                .sesion
                .query_iter(
                    "SELECT table_name FROM system_schema.tables WHERE keyspace_name = ?",
                    (keyspace,),
                )
                .await
                .map_err(fallo)?
                .rows_stream::<(String,)>()
                .map_err(fallo)?
                .map_ok(|(n,)| n)
                .try_collect()
                .await
                .map_err(fallo)?;
            v.sort();
            Ok(v)
        })
    }

    /// La tabla, de `system_schema`. `objeto` si no existe.
    pub fn tabla(&self, keyspace: &str, nombre: &str) -> Result<Tabla, Fallo> {
        let (columnas, indices) = self.rt.block_on(async {
            let columnas: Vec<FilaColumna> = self
                .sesion
                .query_iter(
                    "SELECT column_name, kind, position, clustering_order, type \
                     FROM system_schema.columns WHERE keyspace_name = ? AND table_name = ?",
                    (keyspace, nombre),
                )
                .await
                .map_err(fallo)?
                .rows_stream::<(String, String, i32, String, String)>()
                .map_err(fallo)?
                .map_ok(|(nombre, kind, posicion, orden, tipo)| FilaColumna {
                    nombre,
                    kind,
                    posicion,
                    orden,
                    tipo,
                })
                .try_collect()
                .await
                .map_err(fallo)?;
            let indices: Vec<FilaIndice> = self
                .sesion
                .query_iter(
                    "SELECT index_name, kind, options \
                     FROM system_schema.indexes WHERE keyspace_name = ? AND table_name = ?",
                    (keyspace, nombre),
                )
                .await
                .map_err(fallo)?
                .rows_stream::<(String, String, std::collections::BTreeMap<String, String>)>()
                .map_err(fallo)?
                .map_ok(|(nombre, kind, opciones)| FilaIndice {
                    nombre,
                    kind,
                    clase: opciones.get("class_name").cloned(),
                    objetivo: opciones.get("target").cloned(),
                })
                .try_collect()
                .await
                .map_err(fallo)?;
            Ok::<_, Fallo>((columnas, indices))
        })?;
        if columnas.is_empty() {
            return Err(Fallo::new(
                Codigo::Objeto,
                format!("`{keyspace}.{nombre}` no existe (o el rol no la ve)"),
            ));
        }
        Tabla::de_system_schema(keyspace, nombre, &columnas, &indices)
            .map_err(|m| Fallo::new(Codigo::Esquema, m))
    }

    /// **Los rangos de token del anillo**, `(inicio exclusivo, fin inclusivo)`,
    /// de todos los nodos. Solo con `Murmur3Partitioner`: el de Cassandra desde
    /// 1.2 y el único que mide el recorrido (otro es un `origen`).
    pub fn rangos(&self) -> Result<Vec<(i64, i64)>, Fallo> {
        self.rt.block_on(async {
            let (partidor, locales) = self
                .sesion
                .query_unpaged("SELECT partitioner, tokens FROM system.local", &[])
                .await
                .map_err(fallo)?
                .into_rows_result()
                .map_err(fallo)?
                .single_row::<(String, std::collections::HashSet<String>)>()
                .map_err(fallo)?;
            if !partidor.ends_with("Murmur3Partitioner") {
                return Err(Fallo::origen(format!(
                    "el recorrido por rangos de token es de `Murmur3Partitioner` y este clúster usa `{partidor}`"
                )));
            }
            let mut fichas: Vec<i64> = locales.iter().filter_map(|t| t.parse().ok()).collect();
            let pares: Vec<(Option<std::collections::HashSet<String>>,)> = self
                .sesion
                .query_iter("SELECT tokens FROM system.peers_v2", &[])
                .await
                .map_err(fallo)?
                .rows_stream::<(Option<std::collections::HashSet<String>>,)>()
                .map_err(fallo)?
                .try_collect()
                .await
                .map_err(fallo)?;
            for (t,) in pares {
                fichas.extend(t.into_iter().flatten().filter_map(|t| t.parse::<i64>().ok()));
            }
            fichas.sort_unstable();
            fichas.dedup();
            let mut rangos = Vec::with_capacity(fichas.len() + 1);
            let mut previo = i64::MIN;
            for f in fichas {
                rangos.push((previo, f));
                previo = f;
            }
            rangos.push((previo, i64::MAX));
            Ok(rangos)
        })
    }

    /// Una sentencia sin resultado ni enlaces (DDL, `INSERT`): para quien prepara
    /// un laboratorio. El conector no la usa: un origen se lee.
    pub fn ejecutar(&self, cql: &str) -> Result<(), Fallo> {
        self.rt
            .block_on(self.sesion.query_unpaged(cql, &[]))
            .map(|_| ())
            .map_err(fallo)
    }

    /// **Carga filas con una sentencia preparada** (`INSERT … VALUES (?, ?)`), `paralelo`
    /// a la vez: para quien prepara un laboratorio o la semilla del kit. Medido en X0:
    /// ~7 000 filas/s contra un nodo en Docker. El conector no lo usa: un origen se lee.
    pub fn cargar(
        &self,
        cql: &str,
        filas: Vec<Vec<CqlValue>>,
        paralelo: usize,
    ) -> Result<(), Fallo> {
        self.rt.block_on(async {
            let prep = Arc::new(self.sesion.prepare(cql).await.map_err(fallo)?);
            futures::stream::iter(filas)
                .map(|valores| {
                    let (sesion, prep) = (self.sesion.clone(), prep.clone());
                    async move {
                        sesion
                            .execute_unpaged(prep.as_ref(), valores)
                            .await
                            .map(|_| ())
                            .map_err(fallo)
                    }
                })
                .buffer_unordered(paralelo.max(1))
                .try_collect::<Vec<()>>()
                .await
                .map(|_| ())
        })
    }

    /// Una consulta cualquiera, con todas sus filas, sin interpretar: para el
    /// banco del kit (`system_views.clients`, un `count(*)`).
    pub fn consulta(&self, cql: &str) -> Result<Vec<Fila>, Fallo> {
        self.rt.block_on(async {
            let filas: Vec<Row> = self
                .sesion
                .query_iter(cql, &[])
                .await
                .map_err(fallo)?
                .rows_stream::<Row>()
                .map_err(fallo)?
                .try_collect()
                .await
                .map_err(fallo)?;
            Ok(filas.into_iter().map(|r| r.columns).collect())
        })
    }

    /// **Lo que el rol puede leer.** `system_schema` enseña todas las tablas del
    /// clúster aunque el rol no llegue a ellas (X0 §3.5); esto dice cuáles lee de
    /// verdad, con `LIST ALL PERMISSIONS OF <rol>` (funciona sobre el propio rol).
    ///
    /// Sin usuario, o con un servidor sin autorizador (`AllowAllAuthorizer`), se
    /// puede todo. Si no se pudo preguntar es un `Err`: quien cataloga decide.
    pub fn permisos(&self) -> Result<Permisos, Fallo> {
        let Some(rol) = &self.destino.usuario else {
            return Ok(Permisos::todo());
        };
        let cql = format!("LIST ALL PERMISSIONS OF {}", crate::esquema::citar(rol));
        let r: Result<Vec<FilaPermiso>, Box<dyn std::error::Error>> = self.rt.block_on(async {
            Ok(self
                .sesion
                .query_iter(cql.as_str(), &[])
                .await?
                .rows_stream::<FilaPermiso>()?
                .try_collect::<Vec<_>>()
                .await?)
        });
        match r {
            Ok(filas) => {
                Ok(permisos_de(filas.iter().map(|(_, _, recurso, permiso)| {
                    (recurso.as_str(), permiso.as_str())
                })))
            }
            Err(e) => {
                let m = e.to_string();
                // Sin autorizador no hay permisos que listar: se puede todo.
                if m.contains("AllowAllAuthorizer") || m.contains("not supported") {
                    Ok(Permisos::todo())
                } else {
                    Err(fallo(e))
                }
            }
        }
    }

    /// Prepara la lectura de un plan: la sentencia, los rangos de token o los
    /// juegos de enlaces. Lo que falla aquí (la sentencia, el anillo) falla
    /// **antes del primer byte** del flujo.
    pub fn preparar(
        &self,
        plan: &Plan,
        tabla: &Tabla,
        paralelo: usize,
    ) -> Result<Preparada, Fallo> {
        let trabajos: Vec<Vec<CqlValue>> = match plan.modo {
            Modo::Recorrido => self
                .rangos()?
                .into_iter()
                .map(|(a, b)| vec![CqlValue::BigInt(a), CqlValue::BigInt(b)])
                .collect(),
            _ => plan
                .juegos
                .iter()
                .map(|j| enlazar(plan, tabla, j))
                .collect::<Result<_, _>>()?,
        };
        let mut prep = self
            .rt
            .block_on(self.sesion.prepare(plan.cql.as_str()))
            .map_err(fallo)?;
        prep.set_page_size(PAGINA);
        Ok(Preparada {
            prep: Arc::new(prep),
            trabajos,
            paralelo: paralelo.max(1),
        })
    }

    /// **Ejecuta lo preparado, en flujo y en orden.** Cada rango (o juego) lee en una
    /// tarea y manda trozos de [`TROZO`] filas por **su propio** canal de [`CANAL`]
    /// plazas; quien llama los recibe aquí, en su hilo, **un rango tras otro**, y `fila`
    /// los va escribiendo.
    ///
    /// Leen a la vez como mucho `paralelo` rangos (el resto espera su turno), y los que
    /// van por delante del que se está entregando se paran cuando su canal se llena: la
    /// memoria no pasa de `paralelo × CANAL × TROZO` filas, pase lo que pase en el
    /// servidor. A cambio **la salida es determinista**: los rangos en el orden del
    /// anillo (o los juegos en el de la petición) y, dentro de cada uno, en el del
    /// servidor. Una lectura que sale distinta según qué rango contestó antes no se
    /// puede comparar, repetir ni sellar.
    ///
    /// Devuelve **exactamente** lo pedido: sin las filas solo-estáticas, filtrado
    /// por lo que el servidor no sabe, y cortado en el `limit`. Si `bandera` se
    /// levanta, para y lo dice (`cancelada`): no hay `killOp` en Cassandra, y dejar
    /// de pedir páginas es cancelar.
    pub fn correr(
        &self,
        plan: &Plan,
        tabla: &Tabla,
        p: &Preparada,
        bandera: &AtomicBool,
        fila: &mut dyn FnMut(Fila) -> Result<(), Fallo>,
    ) -> Result<Fin, Fallo> {
        let residual = Residual::de(plan, tabla)?;
        let sin_clustering = plan
            .sin_clustering
            .as_deref()
            .and_then(|c| plan.leidas.iter().position(|x| x == c));
        let tope = plan.limit.unwrap_or(u64::MAX);
        let (sesion, prep, trabajos, paralelo) = (
            self.sesion.clone(),
            p.prep.clone(),
            p.trabajos.clone(),
            p.paralelo,
        );
        // Un canal por rango: el que se entrega y los que van por delante.
        let mut emisores = Vec::with_capacity(trabajos.len());
        let mut receptores = Vec::with_capacity(trabajos.len());
        for _ in 0..trabajos.len() {
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<Row>, Fallo>>(CANAL);
            emisores.push(tx);
            receptores.push(rx);
        }
        let tarea = self.rt.spawn(async move {
            let turnos = Arc::new(tokio::sync::Semaphore::new(paralelo));
            let mut tareas = tokio::task::JoinSet::new();
            for (valores, tx) in trabajos.into_iter().zip(emisores) {
                // Un turno por rango en curso: el rango `n + paralelo` espera a que
                // acabe alguno de los que van delante.
                let Ok(turno) = turnos.clone().acquire_owned().await else {
                    break;
                };
                let (sesion, prep) = (sesion.clone(), prep.clone());
                tareas.spawn(async move {
                    let _turno = turno;
                    let r: Result<(), Fallo> = async {
                        let mut it = sesion
                            .execute_iter(prep.as_ref().clone(), valores)
                            .await
                            .map_err(fallo)?
                            .rows_stream::<Row>()
                            .map_err(fallo)?;
                        let mut trozo: Vec<Row> = Vec::with_capacity(TROZO);
                        while let Some(r) = it.try_next().await.map_err(fallo)? {
                            trozo.push(r);
                            if trozo.len() >= TROZO
                                && tx.send(Ok(std::mem::take(&mut trozo))).await.is_err()
                            {
                                return Ok(()); // nadie escucha: se acabó
                            }
                        }
                        if !trozo.is_empty() {
                            let _ = tx.send(Ok(trozo)).await;
                        }
                        Ok(())
                    }
                    .await;
                    if let Err(e) = r {
                        let _ = tx.send(Err(e)).await;
                    }
                });
            }
            while tareas.join_next().await.is_some() {}
        });

        let inicio = Instant::now();
        let mut fin = Fin::default();
        let mut actual = 0usize;
        let resultado = loop {
            if bandera.load(Ordering::Relaxed) {
                fin.cancelada = true;
                break Ok(());
            }
            if let Some(t) = plan.timeout
                && inicio.elapsed() >= t
            {
                break Err(Fallo::new(
                    Codigo::Tiempo,
                    format!("se agotó `timeoutMs` ({} ms) en el cliente", t.as_millis()),
                ));
            }
            let Some(rx) = receptores.get_mut(actual) else {
                break Ok(()); // todos los rangos entregados
            };
            let recibido = self.rt.block_on(async {
                tokio::select! {
                    m = rx.recv() => Some(m),
                    () = tokio::time::sleep(LATIDO) => None,
                }
            });
            let trozo = match recibido {
                None => continue,
                Some(None) => {
                    actual += 1; // este rango acabó: el siguiente
                    continue;
                }
                Some(Some(Err(e))) => break Err(e),
                Some(Some(Ok(t))) => t,
            };
            let mut hecho = false;
            for r in trozo {
                if fin.filas >= tope {
                    hecho = true;
                    break;
                }
                let celdas = r.columns;
                // Una partición solo con estáticas devuelve una fila con la clave
                // de clustering a `null` (X0 §3.1): no es una fila de la tabla.
                if let Some(i) = sin_clustering
                    && celdas.get(i).is_none_or(Option::is_none)
                {
                    continue;
                }
                if !residual.vacio() {
                    match celdas_residuales(plan, tabla, &residual, &celdas) {
                        Ok(v) => {
                            if !residual.cumple(|c| {
                                v.iter().find(|(n, _)| n == c).and_then(|(_, x)| x.as_ref())
                            }) {
                                continue;
                            }
                        }
                        Err(e) => {
                            tarea.abort();
                            return Err(e);
                        }
                    }
                }
                if let Err(e) = fila(celdas) {
                    tarea.abort();
                    return Err(e);
                }
                fin.filas += 1;
            }
            if hecho || fin.filas >= tope {
                break Ok(());
            }
        };
        // Dejar de pedir páginas es lo que cancela; el servidor termina solo la
        // que tuviera en curso.
        tarea.abort();
        resultado.map(|()| fin)
    }

    /// Ejecuta el plan y recoge las filas en memoria: para las pruebas y quien
    /// lea poco. El flujo con tope de memoria es [`Conexion::correr`].
    pub fn leer(&self, plan: &Plan, tabla: &Tabla, paralelo: usize) -> Result<Vec<Fila>, Fallo> {
        let p = self.preparar(plan, tabla, paralelo)?;
        let mut v = Vec::new();
        self.correr(plan, tabla, &p, &AtomicBool::new(false), &mut |f| {
            v.push(f);
            Ok(())
        })?;
        Ok(v)
    }
}

/// Lo que [`Conexion::preparar`] deja listo para [`Conexion::correr`].
pub struct Preparada {
    prep: Arc<scylla::statement::prepared::PreparedStatement>,
    trabajos: Vec<Vec<CqlValue>>,
    paralelo: usize,
}

impl Preparada {
    /// Cuántas ejecuciones hay (rangos de token, o juegos de enlaces).
    pub fn ejecuciones(&self) -> usize {
        self.trabajos.len()
    }
}

/// Cómo terminó una lectura.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Fin {
    pub filas: u64,
    /// La bandera se levantó: lo que se leyó es lo que había hasta entonces.
    pub cancelada: bool,
}

// ── Lo que el rol puede leer ────────────────────────────────────────────────

/// Los `SELECT` que el rol tiene, por clúster, keyspace o tabla.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Permisos {
    todo: bool,
    keyspaces: BTreeSet<String>,
    tablas: BTreeSet<(String, String)>,
}

impl Permisos {
    pub fn todo() -> Permisos {
        Permisos {
            todo: true,
            ..Default::default()
        }
    }

    /// Si el rol lee **esta** tabla.
    pub fn puede_leer(&self, keyspace: &str, tabla: &str) -> bool {
        self.todo
            || self.keyspaces.contains(keyspace)
            || self
                .tablas
                .contains(&(keyspace.to_string(), tabla.to_string()))
    }

    /// Si el rol lee **algo** de este keyspace (para no enseñar los que no).
    pub fn ve_keyspace(&self, keyspace: &str) -> bool {
        self.todo
            || self.keyspaces.contains(keyspace)
            || self.tablas.iter().any(|(k, _)| k == keyspace)
    }
}

/// Lee las filas de `LIST ALL PERMISSIONS`: `(recurso, permiso)`. Solo cuenta
/// `SELECT`. El recurso es `<all keyspaces>`, `<keyspace k>`, `<table k.t>` o
/// `<all tables in keyspace k>`.
pub fn permisos_de<'a>(filas: impl IntoIterator<Item = (&'a str, &'a str)>) -> Permisos {
    let mut p = Permisos::default();
    for (recurso, permiso) in filas {
        if !permiso.eq_ignore_ascii_case("SELECT") {
            continue;
        }
        let r = recurso
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim();
        if r.eq_ignore_ascii_case("all keyspaces") {
            p.todo = true;
        } else if let Some(k) = r.strip_prefix("keyspace ") {
            p.keyspaces.insert(k.trim().to_string());
        } else if let Some(k) = r
            .strip_prefix("all tables in keyspace ")
            .or_else(|| r.strip_prefix("all tables in "))
        {
            p.keyspaces.insert(k.trim().to_string());
        } else if let Some(t) = r.strip_prefix("table ")
            && let Some((k, n)) = t.trim().split_once('.')
        {
            p.tablas.insert((k.to_string(), n.to_string()));
        }
    }
    p
}

async fn conectar(d: &Destino, aplicacion: &str) -> Result<Session, Fallo> {
    let nodos: Vec<String> = d.hosts.iter().map(|(h, p)| format!("{h}:{p}")).collect();
    let mut b = SessionBuilder::new()
        .known_nodes(&nodos)
        .connection_timeout(Duration::from_secs(10))
        .custom_identity(
            SelfIdentity::new()
                .with_application_name(aplicacion.to_string())
                .with_client_id(uuid::Uuid::new_v4().to_string()),
        );
    if let (Some(u), Some(c)) = (&d.usuario, &d.clave) {
        b = b.user(u, c);
    }
    if d.solo_contacto {
        let primero = nodos[0]
            .to_socket_addrs()
            .map_err(|e| Fallo::new(Codigo::Conexion, format!("`{}`: {e}", nodos[0])))?
            .next()
            .ok_or_else(|| Fallo::new(Codigo::Conexion, format!("`{}` no resuelve", nodos[0])))?;
        b = b.address_translator(Arc::new(SoloContacto(primero)));
    }
    // El TLS: la cadena contra las CA de confianza y el certificado final contra la IP del nodo
    // **o** el nombre de la URL (`tls.rs`). Y se guarda por qué se rechazó, para contarlo.
    let tls_pedido = d.tls;
    let mut tls_ctx = None;
    if d.tls {
        let nombres: Vec<String> = d.hosts.iter().map(|(h, _)| h.clone()).collect();
        let c = crate::tls::contexto(&nombres)?;
        b = b.tls_context(Some(c.ctx.clone()));
        tls_ctx = Some(c);
    }
    let mut perfil = ExecutionProfile::builder().consistency(consistencia(&d.consistencia));
    if let Some(dc) = &d.dc_local {
        perfil = perfil.load_balancing_policy(
            DefaultPolicy::builder()
                .prefer_datacenter(dc.clone())
                .build(),
        );
    }
    b = b.default_execution_profile_handle(perfil.build().into_handle());
    b.build().await.map_err(|e| {
        let (texto, depurado) = (e.to_string(), format!("{e:?}"));
        // Un certificado que no se acepta, o un servidor que no habla TLS, se dicen como lo que son.
        let razon = tls_ctx.as_ref().and_then(crate::tls::Contexto::razon);
        crate::tls::explicar(&format!("{texto} {depurado}"), razon.as_deref()).unwrap_or_else(
            || {
                let mut f = fallo_de(&texto, &depurado);
                // Sin TLS contra un servidor que lo exige: el driver solo dice que la conexión se cortó.
                if !tls_pedido && let Some(pista) = crate::tls::pista_sin_tls(&texto) {
                    f.mensaje.push_str(pista);
                }
                f
            },
        )
    })
}

// ── De los enlaces del plan a lo que el servidor espera ─────────────────────

fn enlazar(plan: &Plan, t: &Tabla, juego: &[Bind]) -> Result<Vec<CqlValue>, Fallo> {
    if plan.enlaces.len() != juego.len() {
        return Err(Fallo::origen(
            "el plan lleva enlaces de más o de menos: es un defecto del traductor",
        ));
    }
    plan.enlaces
        .iter()
        .zip(juego)
        .map(|(c, b)| {
            let col = t.columna(c).ok_or_else(|| {
                Fallo::new(Codigo::Esquema, format!("la columna `{c}` ya no está"))
            })?;
            bind_a_cql(b, col)
        })
        .collect()
}

/// El enlace con el tipo **exacto** de su columna: un `int` no admite un `bigint`.
fn bind_a_cql(b: &Bind, c: &Columna) -> Result<CqlValue, Fallo> {
    let raro = || Fallo::origen(format!("un enlace que no es de la columna `{}`", c.nombre));
    Ok(match b {
        Bind::Entero(n) => match c.cql.sin_frozen() {
            Cql::TinyInt => CqlValue::TinyInt(i8::try_from(*n).map_err(|_| raro())?),
            Cql::SmallInt => CqlValue::SmallInt(i16::try_from(*n).map_err(|_| raro())?),
            Cql::Int => CqlValue::Int(i32::try_from(*n).map_err(|_| raro())?),
            Cql::BigInt => CqlValue::BigInt(*n),
            _ => return Err(raro()),
        },
        Bind::Texto(s) => match c.cql.sin_frozen() {
            Cql::Ascii => CqlValue::Ascii(s.clone()),
            _ => CqlValue::Text(s.clone()),
        },
        Bind::Logico(x) => CqlValue::Boolean(*x),
        Bind::Instante(ms) => CqlValue::Timestamp(CqlTimestamp(*ms)),
        Bind::Fecha(d) => CqlValue::Date(CqlDate((i64::from(*d) + (1i64 << 31)) as u32)),
        Bind::Hora(ns) => CqlValue::Time(CqlTime(*ns)),
        Bind::Uuid(s) => {
            let u = uuid::Uuid::parse_str(s).map_err(|_| raro())?;
            match c.cql.sin_frozen() {
                Cql::TimeUuid => CqlValue::Timeuuid(CqlTimeuuid::from(u)),
                _ => CqlValue::Uuid(u),
            }
        }
    })
}

// ── Lo que el servidor devolvió, lo que se pidió ────────────────────────────

/// Las celdas que el residual mira, como `Valor` de OOS.
fn celdas_residuales(
    plan: &Plan,
    tabla: &Tabla,
    residual: &Residual,
    fila: &Fila,
) -> Result<Vec<(String, Option<Valor>)>, Fallo> {
    let mut v = Vec::new();
    for c in residual.columnas() {
        let i = plan.leidas.iter().position(|x| x == c).ok_or_else(|| {
            Fallo::origen(format!("el filtro mira `{c}` y el `SELECT` no la pide"))
        })?;
        let tipo = &tabla
            .columna(c)
            .ok_or_else(|| Fallo::new(Codigo::Esquema, format!("la columna `{c}` ya no está")))?
            .tipo;
        let valor = match fila.get(i).and_then(Option::as_ref) {
            None | Some(CqlValue::Empty) => None,
            Some(x) => Some(
                lote::celda(x, tipo)
                    .map_err(|e| Fallo::new(Codigo::Esquema, format!("columna `{c}`: {e:?}")))?,
            ),
        };
        v.push((c.to_string(), valor));
    }
    Ok(v)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_permisos_de_un_rol_de_solo_lectura() {
        let p = permisos_de([
            ("<keyspace ore_prueba>", "SELECT"),
            ("<keyspace otro>", "MODIFY"),
            ("<table k.t>", "SELECT"),
        ]);
        assert!(p.puede_leer("ore_prueba", "lo_que_sea"));
        assert!(p.puede_leer("k", "t"));
        assert!(
            !p.puede_leer("k", "otra"),
            "una tabla suelta no abre el keyspace"
        );
        assert!(!p.puede_leer("otro", "t"), "MODIFY no es leer");
        assert!(p.ve_keyspace("k") && p.ve_keyspace("ore_prueba") && !p.ve_keyspace("otro"));
    }

    #[test]
    fn todos_los_keyspaces_y_todas_las_tablas_de_uno() {
        assert!(permisos_de([("<all keyspaces>", "SELECT")]).puede_leer("x", "y"));
        let p = permisos_de([("<all tables in keyspace k>", "SELECT")]);
        assert!(p.puede_leer("k", "t") && !p.puede_leer("j", "t"));
        assert!(!permisos_de([("<all keyspaces>", "MODIFY")]).puede_leer("x", "y"));
        assert!(Permisos::todo().puede_leer("a", "b"));
        assert!(!Permisos::default().puede_leer("a", "b"));
    }
}
