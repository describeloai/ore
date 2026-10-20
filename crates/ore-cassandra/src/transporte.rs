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
use ore_core::tipos::Valor;
use ore_driver::{Codigo, Fallo};
use scylla::client::execution_profile::ExecutionProfile;
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use scylla::errors::TranslationError;
use scylla::policies::address_translator::{AddressTranslator, UntranslatedPeer};
use scylla::policies::load_balancing::DefaultPolicy;
use scylla::statement::Consistency;
use scylla::value::{CqlDate, CqlTime, CqlTimestamp, CqlTimeuuid, CqlValue, Row};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

/// Una fila, con una celda por columna de [`Plan::leidas`]; `None` es nulo.
pub type Fila = Vec<Option<CqlValue>>;

/// Filas por ida al servidor. Medido en X0 (§3.3): 5 000 da ~36 % más que 1 000.
pub const PAGINA: i32 = 5000;

/// Hasta cuántos rangos de token se leen a la vez. Medido en X0: con un nodo se
/// satura hacia 4–8.
pub const PARALELO: usize = 4;

pub struct Conexion {
    rt: tokio::runtime::Runtime,
    sesion: Session,
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
        let destino = url::analizar(url).map_err(Fallo::operador)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| Fallo::origen(format!("no se pudo crear el runtime: {e}")))?;
        let sesion = rt.block_on(conectar(&destino))?;
        Ok(Conexion {
            rt,
            sesion,
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

    /// Ejecuta el plan y devuelve **exactamente** lo pedido: las filas del
    /// servidor, sin las solo-estáticas, filtradas por lo que el servidor no
    /// sabe y cortadas en el `limit`. `paralelo` rangos a la vez (o juegos).
    pub fn leer(&self, plan: &Plan, tabla: &Tabla, paralelo: usize) -> Result<Vec<Fila>, Fallo> {
        let residual = Residual::de(plan, tabla)?;
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
        let lectura = self.rt.block_on(async {
            let mut prep = self
                .sesion
                .prepare(plan.cql.as_str())
                .await
                .map_err(fallo)?;
            prep.set_page_size(PAGINA);
            let prep = Arc::new(prep);
            let sesion = &self.sesion;
            let leer_todo = futures::stream::iter(trabajos)
                .map(|valores| {
                    let p = prep.clone();
                    async move {
                        let filas: Vec<Row> = sesion
                            .execute_iter(p.as_ref().clone(), valores)
                            .await
                            .map_err(fallo)?
                            .rows_stream::<Row>()
                            .map_err(fallo)?
                            .try_collect()
                            .await
                            .map_err(fallo)?;
                        Ok::<Vec<Row>, Fallo>(filas)
                    }
                })
                .buffer_unordered(paralelo.max(1))
                .try_fold(Vec::<Row>::new(), |mut a, mut f| async move {
                    a.append(&mut f);
                    Ok(a)
                });
            match plan.timeout {
                Some(t) => tokio::time::timeout(t, leer_todo).await.map_err(|_| {
                    Fallo::new(
                        Codigo::Tiempo,
                        format!("se agotó `timeoutMs` ({} ms) en el cliente", t.as_millis()),
                    )
                })?,
                None => leer_todo.await,
            }
        })?;
        filtrar(
            plan,
            tabla,
            &residual,
            lectura.into_iter().map(|r| r.columns).collect(),
        )
    }
}

async fn conectar(d: &Destino) -> Result<Session, Fallo> {
    let nodos: Vec<String> = d.hosts.iter().map(|(h, p)| format!("{h}:{p}")).collect();
    let mut b = SessionBuilder::new()
        .known_nodes(&nodos)
        .connection_timeout(Duration::from_secs(10));
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
    if d.tls {
        let mut ctx = openssl::ssl::SslContextBuilder::new(openssl::ssl::SslMethod::tls())
            .map_err(|e| Fallo::origen(format!("TLS: {e}")))?;
        ctx.set_default_verify_paths()
            .map_err(|e| Fallo::origen(format!("TLS: {e}")))?;
        // La verificación del certificado y del nombre está siempre encendida.
        ctx.set_verify(openssl::ssl::SslVerifyMode::PEER);
        b = b.tls_context(Some(ctx.build()));
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
    b.build().await.map_err(fallo)
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

/// Quita las filas solo-estáticas, aplica lo que el servidor no sabe filtrar y
/// corta en el `limit`.
fn filtrar(
    plan: &Plan,
    tabla: &Tabla,
    residual: &Residual,
    filas: Vec<Fila>,
) -> Result<Vec<Fila>, Fallo> {
    let pos = |c: &str| plan.leidas.iter().position(|x| x == c);
    let sin_clustering = plan.sin_clustering.as_deref().and_then(pos);
    let mut salida: Vec<Fila> = Vec::new();
    let tope = plan
        .limit
        .map_or(usize::MAX, |n| usize::try_from(n).unwrap_or(usize::MAX));
    for fila in filas {
        if salida.len() >= tope {
            break;
        }
        // Una partición solo con estáticas devuelve una fila con la clave de
        // clustering a `null` (X0 §3.1): no es una fila de la tabla.
        if let Some(i) = sin_clustering
            && fila.get(i).is_none_or(Option::is_none)
        {
            continue;
        }
        if !residual.vacio() {
            let celdas = celdas_residuales(plan, tabla, residual, &fila)?;
            if !residual.cumple(|c| {
                celdas
                    .iter()
                    .find(|(n, _)| n == c)
                    .and_then(|(_, v)| v.as_ref())
            }) {
                continue;
            }
        }
        salida.push(fila);
    }
    Ok(salida)
}

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
