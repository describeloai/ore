//! **`leer` en Arrow**: Phoenix como [`Origen`] (`ore_driver::origen`).
//!
//! Lo único de este módulo es lo que es de Phoenix: de qué tabla se lee, con qué tipos y cómo se vuelve
//! flujo. Los verbos, `SIGTERM` y las sesiones de `servir` son del esqueleto.
//!
//! # El objeto
//!
//! `ESQUEMA.TABLA`, partido en el **primer** punto, o `TABLA` si no tiene esquema. Los nombres de Phoenix
//! distinguen mayúsculas (un identificador sin comillas se pasa a mayúsculas): se prueba el nombre tal cual
//! y, si no existe y lleva minúsculas, el mismo en mayúsculas.
//!
//! # El tipo de cada columna
//!
//! Phoenix **tiene catálogo** (`getColumns`): los tipos no se infieren. Se leen al primer uso de la tabla y se
//! guardan en la sesión (`servir` reutiliza la conexión). Los tipos que la petición declara (`tipos`) mandan
//! si el dato los puede llevar ([`Tipo::declarado`]); si no, es `esquema`, con la columna.
//!
//! # Qué se empuja
//!
//! **La proyección, los diez operadores, el `orderBy` (con los nulos al final) y el `LIMIT`**, traducidos por
//! [`crate::traductor`]: Phoenix los resuelve con la semántica de SQL, así que no hay residual. Lo que no se sabe
//! escribir exacto (un `like` con barra, un `NaN`, una columna binaria) y lo que no se empuja todavía (el recorte
//! por clave y el rango) se **rechaza** con `operador`, nunca se ignora.
//!
//! # El flujo
//!
//! La cabecera Arrow se escribe **después** de ejecutar la consulta: lo que falla ahí (una tabla que no existe,
//! un tiempo límite) sale como un error limpio y no como un flujo a medias. Después, cada lote de
//! [`FILAS_POR_LOTE`] filas sale en cuanto está, mientras el servidor sigue dando frames de uno en uno: la
//! memoria es un frame y un lote, no la tabla.

use crate::esquema::{Tabla, citar};
use crate::lote::{Constructor, ErrorLote};
use crate::tipos::Tipo;
use crate::traductor;
use crate::transporte::{Conexion, fallo_de};
use arrow_array::RecordBatch;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::SchemaRef;
use ore_avatica::Valor;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Filtro, Peticion};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// Filas por lote Arrow.
pub const FILAS_POR_LOTE: usize = 8192;

pub struct Phoenix;

/// Lo que une a quien lee con quien cancela: la bandera y los statements abiertos. El PQS no tiene `killOp`
/// (X0 §3.8): cancelar es **dejar de pedir frames y cerrar el statement**, para que el servidor suelte su cursor
/// y no lo retenga hasta que caduque (M5, §3.13). Lo cierra quien cancela, no quien lee: con `SIGTERM` el esqueleto
/// llama a [`Origen::cancelar`] y termina con 143 sin esperar a que el hilo que lee vuelva.
#[derive(Clone, Default)]
pub struct Token {
    bandera: Arc<AtomicBool>,
    abiertos: Arc<Mutex<HashMap<u64, ore_avatica::Cierre>>>,
    siguiente: Arc<AtomicU64>,
}

impl Token {
    pub fn cancelada(&self) -> bool {
        self.bandera.load(Ordering::Relaxed)
    }

    /// Anota un statement abierto: si se cancela, se cierra. Devuelve con qué soltarlo.
    fn anotar(&self, cierre: ore_avatica::Cierre) -> u64 {
        let id = self.siguiente.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut a) = self.abiertos.lock() {
            a.insert(id, cierre);
        }
        id
    }

    fn soltar(&self, id: u64) {
        if let Ok(mut a) = self.abiertos.lock() {
            a.remove(&id);
        }
    }

    /// Cierra, en el servidor, todo lo que esta lectura tiene abierto.
    fn cerrar_abiertos(&self) {
        let abiertos: Vec<ore_avatica::Cierre> = match self.abiertos.lock() {
            Ok(mut a) => a.drain().map(|(_, c)| c).collect(),
            Err(_) => return,
        };
        for c in abiertos {
            c.cerrar();
        }
    }
}

pub struct Sesion {
    conexion: Conexion,
    /// Por objeto, tal como lo pidió la petición.
    tablas: HashMap<String, Tabla>,
    /// Los extremos de la clave de cada tabla, y cuándo se miraron: sondearlos cuesta dos consultas, y en un
    /// `servir` con cien peticiones pequeñas eso triplicaba el tiempo (medido: 15 s → 43 s).
    extremos: HashMap<String, (Instant, Option<(i64, i64)>)>,
    /// La columna `ROW_TIMESTAMP` de cada tabla **por la que ya se preguntó** (`None`: no tiene). Solo se pregunta
    /// cuando una petición trae un rango: una lectura entera no paga una consulta a `SYSTEM.CATALOG`.
    marcas: HashMap<String, Option<String>>,
}

/// Cuánto vale un extremo sondeado. Equivocarse solo desequilibra las partes —los rangos de los extremos están
/// abiertos—, nunca pierde ni repite una fila.
const VIGENCIA_DE_LOS_EXTREMOS: Duration = Duration::from_secs(60);

/// Lo máximo que puede esperar la sonda de los extremos de la clave. Sobre una tabla tarda decenas de milisegundos.
const LIMITE_DE_LA_SONDA_MS: u64 = 2_000;

/// `(esquema, tabla)` de un objeto `ESQUEMA.TABLA` o `TABLA`.
pub fn partir_objeto(objeto: &str) -> Result<(Option<&str>, &str), Fallo> {
    match objeto.split_once('.') {
        Some((e, t)) if !e.is_empty() && !t.is_empty() => Ok((Some(e), t)),
        None if !objeto.is_empty() => Ok((None, objeto)),
        _ => Err(Fallo::operador(format!(
            "el objeto `{objeto}` no es `ESQUEMA.TABLA` ni `TABLA`"
        ))),
    }
}

/// **La tabla con lo que la petición declara por encima** (`tipos`). Lo declarado manda si el dato lo puede
/// llevar; si no, `esquema` con la columna nombrada, nunca una conversión a escondidas. Una declaración de una
/// columna que la tabla no tiene se ignora: no es de esta lectura.
pub fn con_declarados(t: &Tabla, tipos: &[(String, String)], objeto: &str) -> Result<Tabla, Fallo> {
    let mut t = t.clone();
    for (columna, declarado) in tipos {
        if let Some(c) = t.columnas.iter_mut().find(|c| &c.nombre == columna) {
            c.tipo = c.tipo.declarado(declarado, c.es_date).map_err(|m| {
                Fallo::new(
                    Codigo::Esquema,
                    format!("`{objeto}`, columna `{columna}`: {m}"),
                )
            })?;
        }
    }
    Ok(t)
}

/// Lo que no se empuja todavía, rechazado con `operador`. `Capacidades::admite` ya rechaza lo que no se declara;
/// esto es lo demás de la petición: el recorte por clave y el rango (M7).
pub fn rechazar_lo_que_no_se_empuja(p: &Peticion) -> Result<(), Fallo> {
    Phoenix::CAPACIDADES.admite(p)?;
    if !p.claves.is_empty() {
        return Err(Fallo::operador(
            "el recorte por clave todavía no se empuja a Phoenix (ficha 0071)",
        ));
    }
    // Un rango sin columna es sobre la posición del origen, y Phoenix no tiene changelog que leer (§3.16).
    if let Some(porque) = ore_driver::rango_servible(p, true, false) {
        return Err(Fallo::operador(porque));
    }
    Ok(())
}

/// **El rango de la petición, como dos filtros más** (`cursor > start` y `cursor <= end`: `start` exclusivo y `end`
/// inclusivo, ADR 0016). Solo vale sobre la columna `ROW_TIMESTAMP` de la tabla (§5): sobre cualquier otra sería un
/// barrido entero que se llama incremental, y se rechaza con `operador`. Los valores pasan por la misma
/// [`traductor::condicion`] que el resto de filtros, así que un `start` que no es de la columna no llega al
/// servidor. Pura (sin servidor).
pub fn con_rango(p: &Peticion, t: &Tabla) -> Result<Peticion, Fallo> {
    let Some(cursor) = p.cursor.as_deref() else {
        return Ok(p.clone());
    };
    let col = crate::incremental::cursor_de(t, cursor, &p.objeto)?;
    let mut q = p.clone();
    for (operador, valor) in [("gt", &p.start), ("le", &p.end)] {
        if let Some(v) = valor {
            q.filtros.push(Filtro {
                columna: col.nombre.clone(),
                operador: operador.into(),
                valor: ore_driver::Valor::Uno(v.clone()),
            });
        }
    }
    Ok(q)
}

/// **La consulta de una petición**: las columnas distintas de la proyección, en su orden, y el `SELECT`. Pura
/// (sin servidor): lo que se asegura de ella es un aserto y no una promesa.
///
/// Dos propiedades pueden salir de la misma columna: se pide **una vez** (un resultado con el nombre repetido
/// se leería mal) y la fila se arma después por posición.
pub fn consulta(p: &Peticion, t: &Tabla) -> Result<(String, Vec<String>), Fallo> {
    consulta_en(p, t, &[])
}

/// Lo mismo, con unas condiciones más en el `WHERE` (el rango de una parte de la lectura, [`rangos`]).
pub fn consulta_en(
    p: &Peticion,
    t: &Tabla,
    extra: &[String],
) -> Result<(String, Vec<String>), Fallo> {
    if p.proyeccion.is_empty() {
        return Err(Fallo::operador("la petición no pide ninguna columna"));
    }
    let mut leidas: Vec<String> = Vec::new();
    for (_, c) in &p.proyeccion {
        if !leidas.contains(c) {
            leidas.push(c.clone());
        }
    }
    let mut expresiones = Vec::new();
    for c in &leidas {
        let col = t.columna(c).ok_or_else(|| {
            Fallo::new(
                Codigo::Esquema,
                format!(
                    "`{}` no tiene la columna `{c}` (tiene {})",
                    p.objeto,
                    nombres(t)
                ),
            )
        })?;
        if let Tipo::Otro(n) = &col.tipo {
            return Err(Fallo::new(
                Codigo::Esquema,
                format!(
                    "`{}`, columna `{c}`: el tipo `{n}` de Phoenix no lo lee este origen todavía",
                    p.objeto
                ),
            ));
        }
        expresiones.push(t.expresion(col));
    }
    let mut sql = format!(
        "SELECT {} FROM {}",
        expresiones.join(", "),
        t.objeto_citado()
    );
    sql.push_str(&traductor::donde_con(&p.filtros, t, extra)?);
    sql.push_str(&traductor::orden(&p.orden, t)?);
    if let Some(n) = p.limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }
    Ok((sql, leidas))
}

/// Con menos claves que esto, partir la lectura cuesta más (abrir conexiones) que lo que ahorra: una tabla de
/// 10⁵ filas se lee en una fracción de segundo.
pub const UMBRAL_PARTIR: i128 = 100_000;

/// Un rango de la clave, `[desde, hasta)`; `None` es «sin cota» en ese extremo.
pub type Rango = (Option<i64>, Option<i64>);

/// **Los rangos de la clave en que se parte una lectura**, de `min` a `max` (inclusive), en `n` partes de ancho
/// parecido. Disjuntos y sin hueco: el primero no tiene cota inferior y el último no tiene cota superior, así que
/// una fila que llegara fuera de `min..=max` mientras se lee tampoco se pierde. Cada rango es `[desde, hasta)`.
pub fn rangos(min: i64, max: i64, n: usize) -> Vec<Rango> {
    let ancho = i128::from(max) - i128::from(min) + 1;
    let n = (n.max(1) as i128).min(ancho.max(1));
    let mut cortes: Vec<i64> = Vec::new();
    for j in 1..n {
        let c = i128::from(min) + ancho * j / n;
        let c = c as i64;
        if cortes.last() != Some(&c) {
            cortes.push(c);
        }
    }
    let mut r = Vec::new();
    let mut desde = None;
    for c in cortes {
        r.push((desde, Some(c)));
        desde = Some(c);
    }
    r.push((desde, None));
    r
}

/// La columna por la que se puede partir esta lectura: la primera de la clave primaria, si es un entero. Sin
/// partir si se pide orden (las partes llegan en el orden en que acaban) o límite (las primeras `n` filas de
/// una lectura partida no serían las de la sin partir), o si la URL pide una sola conexión.
pub fn columna_de_particion<'a>(
    p: &Peticion,
    t: &'a Tabla,
    paralelo: usize,
) -> Option<&'a crate::esquema::Columna> {
    // Un incremental es pequeño por construcción, y partirlo abriría conexiones para nada.
    if paralelo < 2
        || !p.orden.is_empty()
        || p.limit.is_some()
        || p.start.is_some()
        || p.end.is_some()
    {
        return None;
    }
    let primera = t.clave_primaria().into_iter().next()?;
    t.columna(&primera).filter(|c| c.tipo == Tipo::Entero)
}

/// La condición de un rango sobre la columna de la clave.
fn condicion_de_rango(columna: &str, (desde, hasta): Rango) -> Vec<String> {
    let q = citar(columna);
    let mut c = Vec::new();
    if let Some(d) = desde {
        c.push(format!("{q} >= {d}"));
    }
    if let Some(h) = hasta {
        c.push(format!("{q} < {h}"));
    }
    c
}

fn nombres(t: &Tabla) -> String {
    t.columnas
        .iter()
        .map(|c| citar(&c.nombre))
        .collect::<Vec<_>>()
        .join(", ")
}

fn de_lote(e: ErrorLote) -> Fallo {
    e.into()
}

impl Sesion {
    /// **Cómo se parte una lectura**: los rangos de la clave, si se puede y si merece la pena. Dos consultas baratas
    /// (`ORDER BY clave LIMIT 1`, ascendente y descendente: un `RANGE SCAN` de una fila, 0,03 s en 262 144 filas)
    /// dan los extremos, que se recuerdan un minuto; con menos de [`UMBRAL_PARTIR`] claves de ancho no se parte.
    fn plan_de_partes(
        &mut self,
        t: &Tabla,
        col: &str,
        n: usize,
        limite_ms: Option<u64>,
    ) -> Result<Option<Vec<Rango>>, Fallo> {
        let clave = t.objeto();
        let vigente = match self.extremos.get(&clave) {
            Some((cuando, e)) if cuando.elapsed() < VIGENCIA_DE_LOS_EXTREMOS => Some(*e),
            _ => None,
        };
        let extremos = match vigente {
            Some(e) => e,
            None => {
                let q = citar(col);
                // La sonda tiene **su** límite, y nunca el de la sesión (que no tiene ninguno): sobre una tabla
                // son dos lecturas de una fila, pero sobre una vista que filtra y casi no da filas cada una es un
                // barrido (medido en M7: 3,8 s la ascendente y más de 60 s la descendente en 262 144 filas).
                // Partir es una optimización; no puede retener la lectura más de lo que ella misma admite.
                let limite =
                    limite_ms.map_or(LIMITE_DE_LA_SONDA_MS, |l| l.min(LIMITE_DE_LA_SONDA_MS));
                let sonda = self.conexion.con_tiempo(limite)?;
                let extremo = |sentido: &str| -> Result<Option<i64>, Fallo> {
                    let filas = sonda.consulta_entera(&format!(
                        "SELECT {q} FROM {} ORDER BY {q} {sentido} LIMIT 1",
                        t.objeto_citado()
                    ))?;
                    Ok(match filas.first().and_then(|f| f.first()) {
                        Some(Valor::Entero(n)) => Some(*n),
                        _ => None,
                    })
                };
                // Una sonda que no contesta a tiempo no es un fallo de la lectura: es que no se sabe de dónde a dónde
                // va la clave, y se lee sin partir. Cualquier otro error sí es de la lectura. En cadena: si la
                // primera no contesta, la segunda no se espera.
                let sin_tiempo = |r: Result<Option<i64>, Fallo>| match r {
                    Ok(v) => Ok(v),
                    Err(f) if f.codigo == Codigo::Tiempo => Ok(None),
                    Err(f) => Err(f),
                };
                let e = match sin_tiempo(extremo("ASC"))? {
                    Some(min) => sin_tiempo(extremo("DESC"))?.map(|max| (min, max)),
                    None => None,
                };
                self.extremos.insert(clave, (Instant::now(), e));
                e
            }
        };
        Ok(extremos.and_then(|(min, max)| {
            (i128::from(max) - i128::from(min) + 1 >= UMBRAL_PARTIR).then(|| rangos(min, max, n))
        }))
    }

    /// La tabla con su columna `ROW_TIMESTAMP`, preguntada una vez por sesión.
    fn con_marca(&mut self, objeto: &str, mut t: Tabla) -> Tabla {
        let marca = match self.marcas.get(objeto) {
            Some(m) => m.clone(),
            None => {
                let m = self.conexion.con_row_timestamp(t.clone()).row_timestamp;
                self.marcas.insert(objeto.to_string(), m.clone());
                m
            }
        };
        t.row_timestamp = marca;
        t
    }

    /// La tabla, de la caché de la sesión. Si la petición pide una columna que la copia guardada no tiene, la
    /// tabla pudo cambiar: se vuelve a leer una vez.
    fn tabla(&mut self, p: &Peticion) -> Result<Tabla, Fallo> {
        let falta = |t: &Tabla| p.proyeccion.iter().any(|(_, c)| t.columna(c).is_none());
        if let Some(t) = self.tablas.get(&p.objeto)
            && !falta(t)
        {
            return Ok(t.clone());
        }
        let (esquema, nombre) = partir_objeto(&p.objeto)?;
        let t = match self.conexion.tabla(esquema, nombre) {
            Err(e) if e.codigo == Codigo::Objeto && p.objeto.chars().any(|c| c.is_lowercase()) => {
                // Phoenix pasa a mayúsculas lo que no va entre comillas: se prueba como lo haría él.
                let (esq, nom) = (esquema.map(str::to_uppercase), nombre.to_uppercase());
                self.conexion.tabla(esq.as_deref(), &nom).map_err(|_| e)?
            }
            otro => otro?,
        };
        self.tablas.insert(p.objeto.clone(), t.clone());
        Ok(t)
    }
}

/// Lo que una parte de la lectura cuenta a quien escribe el flujo.
enum Msg {
    /// La consulta se ejecutó y el servidor la aceptó: ya se puede escribir la cabecera del flujo.
    Abierta,
    Lote(RecordBatch),
    Fin(u64),
    Error(Fallo),
}

/// Quien escribe el flujo Arrow. La cabecera sale **después** de que el servidor aceptó la consulta
/// ([`Msg::Abierta`]): lo que falla antes (una tabla que no existe, un tiempo límite) es un error limpio y no un
/// flujo a medias.
struct Escritor<'a> {
    salida: Option<&'a mut dyn Write>,
    flujo: Option<StreamWriter<&'a mut dyn Write>>,
    schema: SchemaRef,
}

impl<'a> Escritor<'a> {
    fn abrir(&mut self) -> Result<(), Fallo> {
        if let Some(w) = self.salida.take() {
            self.flujo = Some(
                StreamWriter::try_new(w, &self.schema)
                    .map_err(|e| Fallo::origen(format!("no se pudo empezar el flujo: {e}")))?,
            );
        }
        Ok(())
    }

    fn recibir(&mut self, m: Msg) -> Result<(), Fallo> {
        match m {
            Msg::Abierta => self.abrir(),
            Msg::Lote(l) => {
                self.abrir()?;
                let f = self.flujo.as_mut().expect("abierto");
                f.write(&l)
                    .and_then(|_| f.flush())
                    .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo escribir: {e}")))
            }
            Msg::Fin(_) | Msg::Error(_) => Ok(()),
        }
    }

    fn terminar(&mut self) -> Result<(), Fallo> {
        self.abrir()?;
        self.flujo
            .as_mut()
            .expect("abierto")
            .finish()
            .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo cerrar el flujo: {e}")))
    }
}

fn cancelada(filas: u64) -> Fallo {
    Fallo::origen(format!("cancelada con {filas} filas leídas"))
}

/// **Una consulta, de principio a fin**: la ejecuta, lee sus frames de uno en uno y entrega un lote Arrow cada
/// [`FILAS_POR_LOTE`] filas. La memoria es un frame y un lote. Se detiene si se cancela (`token`) o si otra parte
/// de la misma lectura falló (`parar`). Su statement queda anotado en el `token` mientras dura, para que quien
/// cancele lo cierre en el servidor.
#[allow(clippy::too_many_arguments)]
fn leer_flujo(
    conexion: &Conexion,
    sql: &str,
    esperadas: usize,
    columnas: &[(String, Tipo)],
    posiciones: &[usize],
    token: &Token,
    parar: &AtomicBool,
    sink: &mut dyn FnMut(Msg) -> Result<(), Fallo>,
) -> Result<u64, Fallo> {
    let mut lectura = conexion.consulta(sql)?;
    let anotado = token.anotar(lectura.cierre());
    let r = (|| {
        if lectura.columnas().len() != esperadas {
            return Err(Fallo::origen(format!(
                "Phoenix devolvió {} columnas y se pidieron {esperadas}",
                lectura.columnas().len(),
            )));
        }
        let mut cons = Constructor::nuevo(columnas).map_err(de_lote)?;
        sink(Msg::Abierta)?;
        let mut filas = 0u64;
        loop {
            let frame = match lectura.siguiente_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                // Quien cancela cierra el statement: el `Fetch` que estaba en curso falla, y eso no es un fallo.
                Err(_) if token.cancelada() => return Err(cancelada(filas)),
                Err(e) => return Err(fallo_de(e)),
            };
            for fila in &frame {
                let celdas: Vec<Valor> = posiciones.iter().map(|i| fila[*i].clone()).collect();
                cons.push(&celdas).map_err(de_lote)?;
                filas += 1;
                if cons.filas() >= FILAS_POR_LOTE {
                    sink(Msg::Lote(cons.finish().map_err(de_lote)?))?;
                }
            }
            if token.cancelada() {
                return Err(cancelada(filas));
            }
            if parar.load(Ordering::Relaxed) {
                return Err(Fallo::origen("otra parte de la lectura falló"));
            }
        }
        // Cancelada mientras llegaba el último frame: no es una lectura entera, y quien cancela la espera cancelada.
        if token.cancelada() {
            return Err(cancelada(filas));
        }
        if cons.filas() > 0 {
            sink(Msg::Lote(cons.finish().map_err(de_lote)?))?;
        }
        Ok(filas)
    })();
    token.soltar(anotado);
    r
}

impl Origen for Phoenix {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-phoenix",
        version: env!("CARGO_PKG_VERSION"),
        // Los diez, sin residual: Phoenix los resuelve con la semántica de SQL (§3.12).
        operadores: ore_driver::OPERADORES,
        limit: true,
        order_by: true,
        // `EXPLAIN` da el número de la tabla y no el del filtro (§3.12): no se declara lo que no se sabe.
        estimar: false,
        servir: true,
    };

    fn conectar(url: &str) -> Result<Sesion, Fallo> {
        Ok(Sesion {
            conexion: Conexion::abrir(url)?,
            tablas: HashMap::new(),
            extremos: HashMap::new(),
            marcas: HashMap::new(),
        })
    }

    fn leer(
        s: &mut Sesion,
        token: &Token,
        p: &Peticion,
        salida: &mut dyn Write,
    ) -> Result<u64, Fallo> {
        // El token es de todo el proceso: el de la lectura anterior puede venir cancelado.
        token.bandera.store(false, Ordering::SeqCst);
        rechazar_lo_que_no_se_empuja(p)?;
        let tabla = s.tabla(p)?;
        let tabla = if p.cursor.is_some() {
            s.con_marca(&p.objeto, tabla)
        } else {
            tabla
        };
        let tabla = con_declarados(&tabla, &p.tipos, &p.objeto)?;
        // El rango, si lo hay, son dos filtros más: de aquí en adelante `p` los lleva.
        let con_el_rango = con_rango(p, &tabla)?;
        let p = &con_el_rango;
        let (sql, leidas) = consulta(p, &tabla)?;

        let columnas: Vec<(String, Tipo)> = p
            .proyeccion
            .iter()
            .map(|(prop, col)| {
                (
                    prop.clone(),
                    tabla
                        .columna(col)
                        .expect("comprobada en `consulta`")
                        .tipo
                        .clone(),
                )
            })
            .collect();
        // Dónde está cada columna pedida entre las que el `SELECT` trae.
        let posiciones: Vec<usize> = p
            .proyeccion
            .iter()
            .map(|(_, col)| {
                leidas
                    .iter()
                    .position(|c| c == col)
                    .expect("el `SELECT` pide la proyección")
            })
            .collect();

        let paralelo = s.conexion.destino.paralelo;
        let partes = match columna_de_particion(p, &tabla, paralelo) {
            Some(c) => s
                .plan_de_partes(&tabla, &c.nombre, paralelo, p.timeout_ms)?
                .map(|r| (c.nombre.clone(), r)),
            None => None,
        };

        // Un tiempo límite de la petición es de **esta** lectura: Phoenix lo lee al abrir la conexión, así que
        // se abre una propia y no se toca la sesión que `servir` reutiliza.
        let propia;
        let conexion = match p.timeout_ms {
            Some(ms) => {
                propia = s.conexion.con_tiempo(ms)?;
                &propia
            }
            None => &s.conexion,
        };

        let mut escritor = Escritor {
            salida: Some(salida),
            flujo: None,
            schema: Constructor::nuevo(&columnas).map_err(de_lote)?.schema(),
        };
        let parar = AtomicBool::new(false);
        let destino = &s.conexion.destino;

        let filas = match partes {
            None => leer_flujo(
                conexion,
                &sql,
                leidas.len(),
                &columnas,
                &posiciones,
                token,
                &parar,
                &mut |m| escritor.recibir(m),
            )?,
            Some((col, rangos)) => {
                let (tx, rx) = mpsc::sync_channel::<Msg>(rangos.len() * 2);
                std::thread::scope(|sc| -> Result<u64, Fallo> {
                    for rango in rangos {
                        let tx = tx.clone();
                        let (columnas, posiciones, parar) = (&columnas, &posiciones, &parar);
                        let (col, destino, leidas) = (&col, destino, &leidas);
                        let (tabla, token) = (&tabla, token);
                        sc.spawn(move || {
                            let hecho = (|| {
                                let (sql, _) =
                                    consulta_en(p, tabla, &condicion_de_rango(col, rango))?;
                                // Cada parte, su conexión: una conexión de Avatica lee un cursor a la vez.
                                let c = Conexion::abrir_destino(destino.clone(), p.timeout_ms)?;
                                leer_flujo(
                                    &c,
                                    &sql,
                                    leidas.len(),
                                    columnas,
                                    posiciones,
                                    token,
                                    parar,
                                    &mut |m| {
                                        tx.send(m).map_err(|_| Fallo::origen("la lectura terminó"))
                                    },
                                )
                            })();
                            let _ = tx.send(match hecho {
                                Ok(n) => Msg::Fin(n),
                                Err(f) => Msg::Error(f),
                            });
                        });
                    }
                    drop(tx);
                    let (mut total, mut error) = (0u64, None::<Fallo>);
                    // Se sigue vaciando el canal aunque haya un error: un hilo bloqueado en `send` no acabaría.
                    for m in rx {
                        match m {
                            Msg::Fin(n) => total += n,
                            Msg::Error(f) => {
                                if error.is_none() {
                                    error = Some(f);
                                    parar.store(true, Ordering::Relaxed);
                                }
                            }
                            otro if error.is_none() => {
                                if let Err(f) = escritor.recibir(otro) {
                                    error = Some(f);
                                    parar.store(true, Ordering::Relaxed);
                                }
                            }
                            _ => {}
                        }
                    }
                    match error {
                        Some(f) => Err(f),
                        None => Ok(total),
                    }
                })?
            }
        };
        escritor.terminar()?;
        for a in avisos(&tabla, &leidas) {
            eprintln!("ore-read-phoenix: aviso · `{}`: {a}", p.objeto);
        }
        Ok(filas)
    }

    fn estimar(_: &mut Sesion, _: &Peticion) -> Result<String, Fallo> {
        Err(Fallo::operador("este origen todavía no estima"))
    }

    fn cancelar(token: &Token) {
        token.bandera.store(true, Ordering::SeqCst);
        token.cerrar_abiertos();
    }
}

/// Lo que se pierde en el cable y quien lee tiene que saber (§3.4): se dice una vez por lectura.
pub fn avisos(t: &Tabla, leidas: &[String]) -> Vec<String> {
    let con_marca: Vec<&str> = leidas
        .iter()
        .filter_map(|c| t.columna(c))
        .filter(|c| {
            matches!(
                c.tipo_phoenix.to_ascii_uppercase().as_str(),
                "TIMESTAMP" | "UNSIGNED_TIMESTAMP"
            )
        })
        .map(|c| c.nombre.as_str())
        .collect();
    if con_marca.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "las columnas TIMESTAMP ({}) llegan en milisegundos por Avatica: los nanosegundos no se leen",
        con_marca.join(", ")
    )]
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::esquema::desde_metadatos;
    use crate::esquema::pruebas::{fila, metadatos, pedidos};

    fn peticion(cols: &[(&str, &str)]) -> Peticion {
        Peticion {
            objeto: "ORE_X0.PEDIDOS".into(),
            proyeccion: cols
                .iter()
                .map(|(p, c)| (p.to_string(), c.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn el_objeto_se_parte_en_el_primer_punto() {
        assert_eq!(partir_objeto("ESQ.TABLA").unwrap(), (Some("ESQ"), "TABLA"));
        assert_eq!(partir_objeto("TABLA").unwrap(), (None, "TABLA"));
        assert_eq!(partir_objeto("E.a.b").unwrap(), (Some("E"), "a.b"));
        for malo in ["", ".t", "e."] {
            assert!(partir_objeto(malo).is_err(), "`{malo}`");
        }
    }

    #[test]
    fn el_select_pide_solo_las_columnas_de_la_proyeccion_citadas() {
        let t = pedidos();
        let (sql, leidas) =
            consulta(&peticion(&[("id", "ID"), ("cliente", "CLIENTE")]), &t).unwrap();
        assert_eq!(
            sql,
            "SELECT \"ID\", \"CLIENTE\" FROM \"ORE_X0\".\"PEDIDOS\""
        );
        assert_eq!(leidas, ["ID", "CLIENTE"]);
        // El orden es el de la proyección, no el de la tabla.
        let (sql, _) = consulta(&peticion(&[("c", "CLIENTE"), ("i", "ID")]), &t).unwrap();
        assert_eq!(
            sql,
            "SELECT \"CLIENTE\", \"ID\" FROM \"ORE_X0\".\"PEDIDOS\""
        );
        // Un `LIMIT` se empuja.
        let mut p = peticion(&[("id", "ID")]);
        p.limit = Some(5);
        assert!(consulta(&p, &t).unwrap().0.ends_with(" LIMIT 5"));
    }

    #[test]
    fn el_select_lleva_el_where_el_orden_y_el_limit_en_ese_orden() {
        use ore_driver::{Filtro, Orden};
        let t = pedidos();
        let mut p = peticion(&[("id", "ID")]);
        p.filtros = vec![
            Filtro::uno("ESTADO", "eq", "o'neil\\"),
            Filtro::uno("ID", "ge", "10"),
        ];
        p.orden = vec![Orden {
            columna: "ID".into(),
            descendente: true,
        }];
        p.limit = Some(3);
        let (sql, leidas) = consulta(&p, &t).unwrap();
        assert_eq!(
            sql,
            "SELECT \"ID\" FROM \"ORE_X0\".\"PEDIDOS\" WHERE \"ESTADO\" = 'o''neil\\\\' AND \"ID\" >= 10 ORDER BY \"ID\" DESC NULLS LAST LIMIT 3"
        );
        // Una columna filtrada que no se pide no entra en lo que se lee.
        assert_eq!(leidas, ["ID"]);
        // Un filtro sobre una columna que no está es `esquema`, no se ignora.
        p.filtros = vec![Filtro::uno("NO_EXISTE", "eq", "1")];
        assert_eq!(consulta(&p, &t).unwrap_err().codigo, Codigo::Esquema);
    }

    #[test]
    fn dos_propiedades_de_la_misma_columna_la_piden_una_vez() {
        let t = pedidos();
        let (sql, leidas) =
            consulta(&peticion(&[("a", "ID"), ("b", "ID"), ("c", "CLIENTE")]), &t).unwrap();
        assert_eq!(
            sql,
            "SELECT \"ID\", \"CLIENTE\" FROM \"ORE_X0\".\"PEDIDOS\""
        );
        assert_eq!(leidas, ["ID", "CLIENTE"]);
    }

    #[test]
    fn un_nombre_con_comillas_no_sale_de_su_identificador() {
        let m = metadatos(vec![fila(
            None,
            "T\"; DROP",
            "A\" FROM x --",
            "INTEGER",
            0,
            0,
            1,
            1,
            None,
        )]);
        let t = desde_metadatos(None, "T\"; DROP", &m).unwrap().unwrap();
        let p = Peticion {
            objeto: "T".into(),
            proyeccion: vec![("a".into(), "A\" FROM x --".into())],
            ..Default::default()
        };
        let (sql, _) = consulta(&p, &t).unwrap();
        assert_eq!(sql, "SELECT \"A\"\" FROM x --\" FROM \"T\"\"; DROP\"");
    }

    #[test]
    fn un_date_sin_declarar_se_pide_como_timestamp() {
        let m = metadatos(vec![
            fila(None, "T", "D", "DATE", 0, 0, 1, 1, None),
            fila(None, "T", "TS", "TIMESTAMP", 0, 0, 1, 2, None),
        ]);
        let t = desde_metadatos(None, "T", &m).unwrap().unwrap();
        let p = Peticion {
            objeto: "T".into(),
            proyeccion: vec![("d".into(), "D".into()), ("ts".into(), "TS".into())],
            ..Default::default()
        };
        let (sql, _) = consulta(&p, &t).unwrap();
        assert_eq!(sql, "SELECT CAST(\"D\" AS TIMESTAMP), \"TS\" FROM \"T\"");
        // Declarado `Date`, se lee tal cual: los días son lo que se pidió.
        let t = con_declarados(&t, &[("D".into(), "Date".into())], "T").unwrap();
        let (sql, _) = consulta(&p, &t).unwrap();
        assert_eq!(sql, "SELECT \"D\", \"TS\" FROM \"T\"");
    }

    #[test]
    fn lo_que_no_es_de_la_tabla_o_no_se_lee_se_dice() {
        let t = pedidos();
        let e = consulta(&peticion(&[("x", "NO_EXISTE")]), &t).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(
            e.mensaje.contains("`NO_EXISTE`") && e.mensaje.contains("\"ID\""),
            "{}",
            e.mensaje
        );
        let e = consulta(&peticion(&[]), &t).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        let m = metadatos(vec![fila(None, "T", "B", "BSON", 0, 0, 1, 1, None)]);
        let t = desde_metadatos(None, "T", &m).unwrap().unwrap();
        let p = Peticion {
            objeto: "T".into(),
            proyeccion: vec![("b".into(), "B".into())],
            ..Default::default()
        };
        let e = consulta(&p, &t).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("BSON"), "{}", e.mensaje);
    }

    #[test]
    fn lo_que_no_se_empuja_se_rechaza_y_nunca_se_ignora() {
        use ore_driver::{Filtro, Orden};
        let base = peticion(&[("id", "ID")]);
        assert!(rechazar_lo_que_no_se_empuja(&base).is_ok());
        // Los diez operadores y el orden se declaran: ya no se rechazan aquí. Un operador fuera del
        // vocabulario sí.
        let mut p = base.clone();
        p.filtros = vec![Filtro::uno("ID", "eq", "1")];
        p.orden = vec![Orden {
            columna: "ID".into(),
            descendente: false,
        }];
        assert!(rechazar_lo_que_no_se_empuja(&p).is_ok());
        p.filtros = vec![Filtro::uno("ID", "regex", "1")];
        let e = rechazar_lo_que_no_se_empuja(&p).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("`regex` sobre `ID`"), "{}", e.mensaje);
        let mut p = base.clone();
        p.clave_columnas = vec!["ID".into()];
        p.claves = vec![vec!["1".into()]];
        assert!(
            rechazar_lo_que_no_se_empuja(&p)
                .unwrap_err()
                .mensaje
                .contains("clave")
        );
        // Un rango sin columna es sobre la posición del origen, y Phoenix no tiene changelog: se rechaza. Con
        // columna, el que decide es la tabla (`con_rango`).
        for poner in [
            (|p: &mut Peticion| p.start = Some("1".into())) as fn(&mut Peticion),
            |p| p.end = Some("9".into()),
        ] {
            let mut p = base.clone();
            poner(&mut p);
            assert!(
                rechazar_lo_que_no_se_empuja(&p)
                    .unwrap_err()
                    .mensaje
                    .contains("posición del origen")
            );
        }
        let mut p = base.clone();
        p.cursor = Some("CREADO".into());
        p.start = Some("1".into());
        assert!(rechazar_lo_que_no_se_empuja(&p).is_ok());
    }

    fn con_row_timestamp() -> Tabla {
        let m = metadatos(vec![
            fila(None, "T", "CREADO", "TIMESTAMP", 0, 0, 1, 1, Some(1)),
            fila(None, "T", "ID", "BIGINT", 0, 0, 1, 2, Some(2)),
            fila(None, "T", "ESTADO", "VARCHAR", 0, 0, 1, 3, None),
        ]);
        let mut t = desde_metadatos(None, "T", &m).unwrap().unwrap();
        t.row_timestamp = Some("CREADO".into());
        t
    }

    fn rango(cursor: &str, start: Option<&str>, end: Option<&str>) -> Peticion {
        Peticion {
            objeto: "T".into(),
            proyeccion: vec![("id".into(), "ID".into())],
            cursor: Some(cursor.into()),
            start: start.map(String::from),
            end: end.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn el_rango_sobre_la_row_timestamp_es_start_exclusivo_y_end_inclusivo() {
        let t = con_row_timestamp();
        let q = con_rango(
            &rango(
                "CREADO",
                Some("2026-10-21T08:39:19.253Z"),
                Some("2026-10-21T09:00:00Z"),
            ),
            &t,
        )
        .unwrap();
        let (sql, _) = consulta(&q, &t).unwrap();
        assert_eq!(
            sql,
            "SELECT \"ID\" FROM \"T\" WHERE \"CREADO\" > TIMESTAMP '2026-10-21 08:39:19.253' AND \"CREADO\" <= TIMESTAMP '2026-10-21 09:00:00'"
        );
        // Solo `end` (la primera copia, hasta el testigo) y solo `start` (hasta donde estés): una condición.
        let q = con_rango(&rango("CREADO", None, Some("2026-10-21T09:00:00Z")), &t).unwrap();
        assert_eq!(q.filtros.len(), 1);
        assert_eq!(q.filtros[0].operador, "le");
        let q = con_rango(&rango("CREADO", Some("2026-10-21T09:00:00Z"), None), &t).unwrap();
        assert_eq!(q.filtros.len(), 1);
        assert_eq!(q.filtros[0].operador, "gt");
        // Sin cursor no hay nada que añadir.
        let mut sin = rango("CREADO", None, None);
        sin.cursor = None;
        assert!(con_rango(&sin, &t).unwrap().filtros.is_empty());
    }

    #[test]
    fn un_rango_sobre_otra_columna_o_en_una_tabla_sin_row_timestamp_se_rechaza() {
        let t = con_row_timestamp();
        let e = con_rango(&rango("ESTADO", Some("a"), None), &t).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("barrido entero"), "{}", e.mensaje);
        let mut sin = con_row_timestamp();
        sin.row_timestamp = None;
        let e = con_rango(&rango("CREADO", Some("a"), None), &sin).unwrap_err();
        assert!(
            e.mensaje
                .contains("no declara ninguna columna ROW_TIMESTAMP"),
            "{}",
            e.mensaje
        );
    }

    #[test]
    fn un_valor_que_no_es_de_la_columna_no_llega_al_servidor() {
        let t = con_row_timestamp();
        let e = con_rango(&rango("CREADO", Some("ayer"), None), &t).unwrap();
        // Los filtros se traducen al armar la consulta: ahí es donde se rechaza el valor.
        let e = consulta(&e, &t).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("`ayer`"), "{}", e.mensaje);
    }

    #[test]
    fn un_incremental_no_se_parte() {
        let t = con_row_timestamp();
        let mut p = rango("CREADO", Some("2026-10-21T08:39:19.253Z"), None);
        p.proyeccion = vec![("id".into(), "ID".into())];
        // El `ID` es entero y primera clave de otra tabla: aquí se comprueba solo la regla del rango.
        let mut t2 = t.clone();
        t2.columnas[0].clave = Some(2);
        t2.columnas[1].clave = Some(1);
        assert!(columna_de_particion(&rango("CREADO", None, None), &t2, 4).is_some());
        assert!(columna_de_particion(&p, &t2, 4).is_none());
    }

    #[test]
    fn lo_declarado_manda_si_el_dato_lo_lleva() {
        let m = metadatos(vec![
            fila(None, "T", "TOTAL", "DECIMAL", 0, 0, 1, 1, None),
            fila(None, "T", "CREADO", "TIMESTAMP", 0, 0, 1, 2, None),
            fila(None, "T", "ESTADO", "VARCHAR", 0, 0, 1, 3, None),
        ]);
        let t = desde_metadatos(None, "T", &m).unwrap().unwrap();
        let d = con_declarados(
            &t,
            &[
                ("TOTAL".into(), "Decimal<12, 2>".into()),
                ("CREADO".into(), "DateTime".into()),
                ("FANTASMA".into(), "Integer".into()),
            ],
            "T",
        )
        .unwrap();
        assert_eq!(
            d.columna("TOTAL").unwrap().tipo,
            Tipo::Decimal {
                precision: 12,
                escala: 2
            }
        );
        assert_eq!(d.columna("CREADO").unwrap().tipo, Tipo::FechaHora);
        // La tabla original no cambia: la caché de la sesión sigue siendo la del servidor.
        assert_eq!(t.columna("TOTAL").unwrap().tipo, Tipo::DecimalLibre);
        // Un tipo que el dato no lleva nombra la columna.
        let e = con_declarados(&t, &[("ESTADO".into(), "Integer".into())], "T").unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("columna `ESTADO`"), "{}", e.mensaje);
    }

    #[test]
    fn el_conector_declara_lo_que_sabe() {
        let c = Phoenix::CAPACIDADES;
        assert_eq!(c.conector, "ore-read-phoenix");
        assert_eq!(c.operadores, ore_driver::OPERADORES);
        assert!(c.limit && c.servir && c.order_by);
        // `EXPLAIN` no sabe el coste de un filtro (§3.12): no se declara.
        assert!(!c.estimar);
        // Y la declaración es JSON canónico válido.
        assert!(c.json().contains("\"conector\":\"ore-read-phoenix\""));
    }

    #[test]
    fn los_timestamp_avisan_de_los_milisegundos() {
        let t = pedidos();
        let a = avisos(&t, &["ID".into(), "CREADO".into()]);
        assert_eq!(a.len(), 1);
        assert!(
            a[0].contains("CREADO") && a[0].contains("milisegundos"),
            "{a:?}"
        );
        assert!(avisos(&t, &["ID".into()]).is_empty());
    }

    #[test]
    fn los_rangos_son_disjuntos_y_cubren_todas_las_claves() {
        // Sin partir: un solo rango sin cotas.
        assert_eq!(rangos(1, 1_000_000, 1), [(None, None)]);
        let r = rangos(1, 1_000_000, 4);
        assert_eq!(
            r,
            [
                (None, Some(250_001)),
                (Some(250_001), Some(500_001)),
                (Some(500_001), Some(750_001)),
                (Some(750_001), None),
            ]
        );
        // Cada rango empieza donde acaba el anterior: ni hueco ni solape, y los extremos están abiertos para
        // que una fila fuera de `min..=max` tampoco se pierda.
        for (a, b) in r.iter().zip(r.iter().skip(1)) {
            assert_eq!(a.1, b.0);
        }
        assert!(r.first().unwrap().0.is_none() && r.last().unwrap().1.is_none());
        // Más partes que claves: no salen rangos vacíos.
        assert_eq!(rangos(5, 6, 8).len(), 2);
        assert_eq!(rangos(7, 7, 4), [(None, None)]);
        // Los extremos de un `i64` no desbordan.
        let r = rangos(i64::MIN, i64::MAX, 4);
        assert_eq!(r.len(), 4);
        let cortes: Vec<i64> = r.iter().filter_map(|x| x.1).collect();
        assert!(cortes.windows(2).all(|w| w[0] < w[1]), "{cortes:?}");
    }

    #[test]
    fn solo_se_parte_una_lectura_sin_orden_ni_limite_con_clave_entera() {
        use ore_driver::Orden;
        let t = pedidos(); // la clave es `ID`, un `BIGINT`
        let p = peticion(&[("id", "ID")]);
        assert_eq!(columna_de_particion(&p, &t, 4).unwrap().nombre, "ID");
        // Una sola conexión pedida.
        assert!(columna_de_particion(&p, &t, 1).is_none());
        // Con orden o con límite, las partes darían otro resultado.
        let mut con_orden = p.clone();
        con_orden.orden = vec![Orden {
            columna: "ID".into(),
            descendente: false,
        }];
        assert!(columna_de_particion(&con_orden, &t, 4).is_none());
        let mut con_limite = p.clone();
        con_limite.limit = Some(10);
        assert!(columna_de_particion(&con_limite, &t, 4).is_none());
        // Una clave que no es un entero no se parte por rangos.
        let m = metadatos(vec![fila(None, "T", "K", "VARCHAR", 0, 0, 0, 1, Some(1))]);
        let t = desde_metadatos(None, "T", &m).unwrap().unwrap();
        assert!(columna_de_particion(&p, &t, 4).is_none());
        // Ni una tabla sin clave.
        let m = metadatos(vec![fila(None, "T", "K", "BIGINT", 0, 0, 1, 1, None)]);
        let t = desde_metadatos(None, "T", &m).unwrap().unwrap();
        assert!(columna_de_particion(&p, &t, 4).is_none());
    }

    #[test]
    fn cada_parte_lleva_su_rango_junto_a_los_filtros() {
        use ore_driver::Filtro;
        let t = pedidos();
        let mut p = peticion(&[("id", "ID")]);
        p.filtros = vec![Filtro::uno("ESTADO", "eq", "a")];
        let (sql, _) =
            consulta_en(&p, &t, &condicion_de_rango("ID", (Some(10), Some(20)))).unwrap();
        assert_eq!(
            sql,
            "SELECT \"ID\" FROM \"ORE_X0\".\"PEDIDOS\" WHERE \"ESTADO\" = 'a' AND \"ID\" >= 10 AND \"ID\" < 20"
        );
        // Los extremos abiertos no llevan cota; sin filtros, el rango es todo el `WHERE`.
        let p = peticion(&[("id", "ID")]);
        let (sql, _) = consulta_en(&p, &t, &condicion_de_rango("ID", (None, Some(20)))).unwrap();
        assert!(sql.ends_with(" WHERE \"ID\" < 20"), "{sql}");
        let (sql, _) = consulta_en(&p, &t, &condicion_de_rango("ID", (Some(-5), None))).unwrap();
        assert!(sql.ends_with(" WHERE \"ID\" >= -5"), "{sql}");
        assert!(
            consulta_en(&p, &t, &condicion_de_rango("ID", (None, None)))
                .unwrap()
                .0
                .ends_with("\"PEDIDOS\"")
        );
    }

    #[test]
    fn cancelar_cierra_lo_que_la_lectura_tenia_abierto_y_activa_la_bandera() {
        let t = Token::default();
        assert!(!t.cancelada());
        Phoenix::cancelar(&t);
        assert!(t.cancelada());
        // Sin nada abierto no hace nada más, y cancelar dos veces no falla.
        Phoenix::cancelar(&t);
    }
}
