//! **Leer una tabla de HBase** como el conector v2 del esqueleto (`ore_driver::origen::Origen`): una peticion entra,
//! un flujo Arrow IPC sale.
//!
//! ```text
//!   Peticion ─preparar─▶ Preparada { columnas con su tipo y codificacion, que pedir, intervalos, residuales }  [puro]
//!   intervalos ─por regiones─▶ trozos ─repartir─▶ hasta `paralelo` grupos                                       [puro]
//!   cada grupo ─un hilo─▶ Lector (ore-hbase-rpc: reintentos, reanuda por la ultima clave)
//!        └─ por fila: decodificar cada celda UNA vez ─▶ residuales ─▶ lote Arrow
//! ```
//!
//! Lo que se empuja a HBase es el **rango de la clave** (exacto solo si la codificacion de la clave conserva el
//! orden: [`crate::filtros`]); lo demas se evalua aqui sobre el valor ya decodificado, con la semantica de SQL.
//! `orderBy` no se declara (HBase solo ordena por clave y el reparto en paralelo no la conserva), `limit` si, y con
//! el la lectura va en un solo hilo. El incremental (`start`/`end`) llega en N5.

use crate::codificacion::decodificar;
use crate::completa::Completa;
use crate::esquema::{
    CLAVE, PREFIJO_PARTE, ParteClave, Tipo, en_hbase, es_texto, indice_de_parte, partes_de,
    partir_columna, partir_objeto, trocear,
};
use crate::filtros::{self, Intervalo, Residual};
use crate::lote::{Constructor, ErrorLote};
use crate::transporte::{Conexion, fallo_de};
use arrow_array::RecordBatch;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::SchemaRef;
use ore_core::tipos::Valor;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use ore_hbase_rpc::{Cancelacion, Consulta};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

/// Filas por lote Arrow.
pub const FILAS_POR_LOTE: usize = 8192;
/// Filas que se piden al servidor en cada llamada.
pub const FILAS_POR_LLAMADA: u32 = 5_000;
/// `FirstKeyOnlyFilter`: de cada fila, solo su primera celda. Lo que se pide cuando solo hace falta la clave.
const SOLO_PRIMERA_CELDA: &str = "org.apache.hadoop.hbase.filter.FirstKeyOnlyFilter";

pub struct Hbase;

impl Hbase {
    /// Los operadores que declara, para quien los enumera (la cara `I` del catalogo).
    pub const CAPACIDADES_OPERADORES: &'static [&'static str] = &[
        "eq",
        "neq",
        "in",
        "lt",
        "le",
        "gt",
        "ge",
        "isNull",
        "isNotNull",
    ];
}

/// La cancelacion de todo el proceso: la lectura anterior puede venir cancelada y esta la rearma.
pub type Token = Cancelacion;

pub struct Sesion {
    conexion: Conexion,
}

/// De donde sale el valor de una columna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fuente {
    /// La clave de fila entera.
    Clave,
    /// Una parte de la clave compuesta (desde 0).
    Parte(usize),
    /// Una celda: `(familia, cualificador)`.
    Celda(Vec<u8>, Vec<u8>),
}

/// Una columna que la lectura necesita (sale en la fila, o la mira un filtro).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usada {
    /// `_clave` o `familia:cualificador`.
    pub columna: String,
    pub tipo: Tipo,
    partida: Fuente,
}

/// Una peticion entendida.
#[derive(Debug, Clone, PartialEq)]
pub struct Preparada {
    pub espacio: String,
    pub tabla: String,
    /// El tipo de la clave de fila (la decodifican los filtros que no se empujan y las columnas que la piden).
    pub clave: Tipo,
    /// Las partes de la clave si es compuesta (`_clave#1`, `_clave#2`…); vacio si no.
    pub partes: Vec<ParteClave>,
    /// Las columnas que salen: `(propiedad, posicion en `usadas`)`, en el orden de la proyeccion.
    pub salientes: Vec<(String, usize)>,
    /// Las columnas de la proyeccion y de los filtros, cada una una vez.
    pub usadas: Vec<Usada>,
    /// Lo que se pide a HBase: `(familia, cualificadores)`. Vacio: solo la clave.
    pub a_pedir: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    pub intervalos: Vec<Intervalo>,
    /// Los filtros que se evaluan aqui, con la posicion de su columna en `usadas`.
    pub residuales: Vec<(usize, Residual)>,
    pub limit: Option<u64>,
}

impl Preparada {
    pub fn solo_claves(&self) -> bool {
        self.a_pedir.is_empty()
    }
}

/// Lo que la peticion dice de una columna: el tipo y la codificacion, sin adivinar.
fn tipo_declarado(p: &Peticion, columna: &str) -> Result<Tipo, Fallo> {
    fn de<'a>(v: &'a [(String, String)], columna: &str) -> Option<&'a str> {
        v.iter()
            .find(|(c, _)| c == columna)
            .map(|(_, x)| x.as_str())
    }
    Tipo::nuevo(
        de(&p.tipos, columna),
        de(&p.codificaciones, columna),
        &p.objeto,
        columna,
    )
}

/// **Los nombres cortos de la URL** (`familia=`, `clave=`) llevados a los de HBase: `_clave` y `familia:cualificador`.
/// Se aplican a todo lo que nombra una columna: la proyeccion, los filtros, los tipos y las codificaciones.
pub fn normalizar(p: &Peticion) -> Result<Peticion, Fallo> {
    let d = crate::url::analizar(&p.url).map_err(Fallo::operador)?;
    if d.familia.is_none() && d.clave.is_none() {
        return Ok(p.clone());
    }
    let n = |c: &str| -> String {
        if d.clave.as_deref() == Some(c) {
            CLAVE.to_string()
        } else if c != CLAVE && !c.starts_with(PREFIJO_PARTE) && !c.contains(':') {
            d.familia
                .as_ref()
                .map_or_else(|| c.to_string(), |f| format!("{f}:{c}"))
        } else {
            c.to_string()
        }
    };
    let mut q = p.clone();
    q.proyeccion.iter_mut().for_each(|(_, c)| *c = n(c));
    q.filtros.iter_mut().for_each(|f| f.columna = n(&f.columna));
    q.tipos.iter_mut().for_each(|(c, _)| *c = n(c));
    q.codificaciones.iter_mut().for_each(|(c, _)| *c = n(c));
    Ok(q)
}

/// **Entiende la peticion** (pura): rechaza lo que no sabe hacer, resuelve tipos y codificaciones, y separa lo que
/// va a HBase de lo que se evalua aqui.
pub fn preparar(p: &Peticion) -> Result<Preparada, Fallo> {
    let p = &normalizar(p)?;
    Hbase::CAPACIDADES.admite(p)?;
    if !p.claves.is_empty() {
        return Err(Fallo::operador(
            "el recorte por clave todavia no se empuja a HBase (ficha 0072)",
        ));
    }
    // El incremental (`start`/`end` sobre una marca de tiempo de celda) es N5.
    if let Some(porque) = ore_driver::rango_servible(p, false, false) {
        return Err(Fallo::operador(porque));
    }
    let (espacio, tabla) = partir_objeto(&p.objeto)?;
    let partes = partes_de(&p.tipos, &p.codificaciones, &p.objeto)?;
    let validar = |c: &str| -> Result<(), Fallo> {
        if c == CLAVE {
            return Ok(());
        }
        if c.starts_with(PREFIJO_PARTE) {
            return match indice_de_parte(c) {
                Some(n) if n <= partes.len() => Ok(()),
                _ => Err(Fallo::new(
                    Codigo::Esquema,
                    format!(
                        "`{}`: `{c}` no es una parte de la clave que la tabla declare (hay {})",
                        p.objeto,
                        partes.len()
                    ),
                )),
            };
        }
        partir_columna(c)
            .map(|_| ())
            .map_err(|m| Fallo::new(Codigo::Esquema, format!("`{}`: {m}", p.objeto)))
    };
    // Los tipos de todas las columnas que la peticion toca, una vez cada una.
    let mut tipos: BTreeMap<String, Tipo> = BTreeMap::new();
    let mut tocar = |c: &str| -> Result<(), Fallo> {
        validar(c)?;
        if !tipos.contains_key(c) {
            let t = match indice_de_parte(c) {
                Some(n) => partes[n - 1].tipo.clone(),
                None => tipo_declarado(p, c)?,
            };
            tipos.insert(c.to_string(), t);
        }
        Ok(())
    };
    tocar(CLAVE)?;
    for (_, c) in &p.proyeccion {
        tocar(c)?;
    }
    for f in &p.filtros {
        tocar(&f.columna)?;
    }
    let clave = tipos[CLAVE].clone();
    let plan = filtros::plan(
        &p.filtros,
        &clave,
        &partes,
        &|c| tipos.get(c).cloned().unwrap_or_else(Tipo::texto),
        &p.objeto,
    )?;

    // Las columnas usadas: las de la proyeccion y las de los residuales, cada una una vez.
    let mut usadas: Vec<Usada> = Vec::new();
    let mut posicion = |c: &str| -> usize {
        if let Some(i) = usadas.iter().position(|u| u.columna == c) {
            return i;
        }
        usadas.push(Usada {
            columna: c.to_string(),
            tipo: tipos[c].clone(),
            partida: if c == CLAVE {
                Fuente::Clave
            } else if let Some(n) = indice_de_parte(c) {
                Fuente::Parte(n - 1)
            } else {
                let (f, q) = partir_columna(c).expect("validada");
                Fuente::Celda(f, q)
            },
        });
        usadas.len() - 1
    };
    let salientes: Vec<(String, usize)> = p
        .proyeccion
        .iter()
        .map(|(prop, c)| (prop.clone(), posicion(c)))
        .collect();
    let residuales: Vec<(usize, Residual)> = plan
        .residuales
        .into_iter()
        .map(|r| (posicion(&r.columna), r))
        .collect();

    // Las celdas que hay que traer.
    let mut por_familia: BTreeMap<Vec<u8>, Vec<Vec<u8>>> = BTreeMap::new();
    for (f, q) in usadas.iter().filter_map(|u| match &u.partida {
        Fuente::Celda(f, q) => Some((f.clone(), q.clone())),
        _ => None,
    }) {
        let qs = por_familia.entry(f).or_default();
        if !qs.contains(&q) {
            qs.push(q);
        }
    }
    Ok(Preparada {
        espacio,
        tabla,
        clave,
        partes,
        salientes,
        usadas,
        a_pedir: por_familia.into_iter().collect(),
        intervalos: plan.intervalos,
        residuales,
        limit: p.limit,
    })
}

/// Lo que una parte de la lectura cuenta a quien escribe el flujo.
enum Msg {
    Abierta,
    Lote(RecordBatch),
    Fin(u64),
    Error(Fallo),
}

/// Quien escribe el flujo Arrow. La cabecera sale cuando la tabla se ha localizado ([`Msg::Abierta`]): lo que falla
/// antes (una tabla que no existe) es un error limpio y no un flujo a medias.
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

fn de_lote(e: ErrorLote) -> Fallo {
    Fallo::new(Codigo::Esquema, e.to_string())
}

fn cancelada(filas: u64) -> Fallo {
    Fallo::origen(format!("cancelada con {filas} filas leidas"))
}

/// La clave de una fila para un mensaje de error: el texto si lo es, y si no, los bytes en hexadecimal.
fn clave_legible(clave: &[u8]) -> String {
    if es_texto(clave) {
        String::from_utf8_lossy(clave).into_owned()
    } else {
        format!(
            "0x{}",
            clave.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )
    }
}

/// Lo que comparten las partes de una lectura.
struct Compartido<'a> {
    cliente: &'a ore_hbase_rpc::Cliente,
    tabla_hbase: &'a str,
    prep: &'a Preparada,
    token: &'a Token,
    parar: &'a AtomicBool,
}

/// Las celdas de una fila, **decodificadas una vez** cada una. Un valor que no cabe en su tipo es un error que dice
/// columna y clave (0032 §4): ni nulo ni arreglo.
fn decodificar_fila(
    prep: &Preparada,
    fila: &ore_hbase_rpc::Fila,
) -> Result<Vec<Option<Valor>>, Fallo> {
    // La clave compuesta se trocea una vez por fila, y solo si alguna parte se usa.
    let trozos = if prep
        .usadas
        .iter()
        .any(|u| matches!(u.partida, Fuente::Parte(_)))
    {
        Some(trocear(&prep.partes, &fila.clave).map_err(|detalle| {
            de_lote(ErrorLote::NoCabe {
                columna: "_clave".to_string(),
                clave: clave_legible(&fila.clave),
                detalle,
            })
        })?)
    } else {
        None
    };
    prep.usadas
        .iter()
        .map(|u| {
            let bytes: Option<&[u8]> = match &u.partida {
                Fuente::Clave => Some(&fila.clave),
                Fuente::Parte(i) => trozos.as_ref().map(|t| t[*i]),
                Fuente::Celda(f, q) => fila.valor(f, q),
            };
            bytes
                .map(|b| {
                    decodificar(u.tipo.cod, &u.tipo.fisico, b).map_err(|detalle| {
                        de_lote(ErrorLote::NoCabe {
                            columna: u.columna.clone(),
                            clave: clave_legible(&fila.clave),
                            detalle,
                        })
                    })
                })
                .transpose()
        })
        .collect()
}

/// **Una parte de la lectura**: sus intervalos, uno tras otro, con su lector y sus lotes.
fn leer_parte(
    c: &Compartido,
    intervalos: &[Intervalo],
    sink: &mut dyn FnMut(Msg) -> Result<(), Fallo>,
) -> Result<u64, Fallo> {
    let prep = c.prep;
    let columnas: Vec<(String, Tipo)> = prep
        .salientes
        .iter()
        .map(|(p, i)| (p.clone(), prep.usadas[*i].tipo.clone()))
        .collect();
    let mut cons = Constructor::nuevo(&columnas).map_err(de_lote)?;
    let mut filas = 0u64;
    for (inicio, fin) in intervalos {
        // Las claves de TODAS las filas (con `FirstKeyOnlyFilter`: barato) ...
        let todas = Consulta {
            inicio: inicio.clone(),
            fin: fin.clone(),
            filas_por_llamada: FILAS_POR_LLAMADA,
            filtro: Some((SOLO_PRIMERA_CELDA.to_string(), Vec::new())),
            ..Consulta::default()
        };
        let claves = c
            .cliente
            .leer(c.tabla_hbase, &todas, c.token)
            .map_err(fallo_de)?;
        // ... y, si hay celdas que leer, las de las filas que las tienen. Un `Scan` con columnas concretas no devuelve
        // las filas que no tienen ninguna: [`Completa`] las pone, con todo nulo.
        let filas_del_intervalo: Box<
            dyn Iterator<Item = ore_hbase_rpc::Resultado<ore_hbase_rpc::Fila>> + '_,
        > = if prep.solo_claves() {
            Box::new(claves)
        } else {
            let con_celdas = Consulta {
                columnas: prep.a_pedir.clone(),
                filtro: None,
                ..todas
            };
            let celdas = c
                .cliente
                .leer(c.tabla_hbase, &con_celdas, c.token)
                .map_err(fallo_de)?;
            Box::new(Completa::nuevo(claves, celdas))
        };
        for fila in filas_del_intervalo {
            let fila = match fila {
                Ok(f) => f,
                Err(_) if c.token.cancelada() => return Err(cancelada(filas)),
                Err(e) => return Err(fallo_de(e)),
            };
            let valores = decodificar_fila(prep, &fila)?;
            if !prep
                .residuales
                .iter()
                .all(|(i, r)| filtros::cumple_uno(r, valores[*i].as_ref()))
            {
                continue;
            }
            let salida: Vec<Option<Valor>> = prep
                .salientes
                .iter()
                .map(|(_, i)| valores[*i].clone())
                .collect();
            cons.push(&salida).map_err(de_lote)?;
            filas += 1;
            if cons.filas() >= FILAS_POR_LOTE {
                sink(Msg::Lote(cons.finish().map_err(de_lote)?))?;
            }
            if prep.limit.is_some_and(|l| filas >= l) {
                break;
            }
            if c.token.cancelada() {
                return Err(cancelada(filas));
            }
            if c.parar.load(Ordering::Relaxed) {
                return Err(Fallo::origen("otra parte de la lectura fallo"));
            }
        }
        if prep.limit.is_some_and(|l| filas >= l) {
            break;
        }
    }
    if cons.filas() > 0 {
        sink(Msg::Lote(cons.finish().map_err(de_lote)?))?;
    }
    Ok(filas)
}

impl Origen for Hbase {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-hbase",
        version: env!("CARGO_PKG_VERSION"),
        // Sobre la clave de fila se empujan como rangos; sobre las celdas se evaluan aqui. `like` no: no hay
        // como empujarlo y evaluarlo con la semantica de SQL es trabajo de otro hito.
        operadores: Hbase::CAPACIDADES_OPERADORES,
        limit: true,
        // HBase solo ordena por clave, y el reparto en paralelo no la conserva.
        order_by: false,
        estimar: false,
        servir: true,
    };

    fn conectar(url: &str) -> Result<Sesion, Fallo> {
        Ok(Sesion {
            conexion: Conexion::abrir(url)?,
        })
    }

    fn leer(
        s: &mut Sesion,
        token: &Token,
        p: &Peticion,
        salida: &mut dyn Write,
    ) -> Result<u64, Fallo> {
        // El token es de todo el proceso: el de la lectura anterior puede venir cancelado.
        token.rearmar();
        let prep = preparar(p)?;
        // Un tiempo limite de la peticion es de **esta** lectura: otro cliente, sin tocar el de la sesion.
        let propia;
        let conexion = match p.timeout_ms {
            Some(ms) => {
                propia = s.conexion.con_tiempo(ms)?;
                &propia
            }
            None => &s.conexion,
        };
        let tabla_hbase = en_hbase(&prep.espacio, &prep.tabla);
        // Las regiones de la tabla: de ellas sale el reparto, y si la tabla no existe se dice aqui, antes de abrir el
        // flujo. Si la cache estaba vieja, el lector re-pregunta region a region: solo se desequilibra el reparto.
        let regiones = conexion
            .cliente
            .regiones_en_cache(&tabla_hbase)
            .map_err(fallo_de)?;
        let limites: Vec<Intervalo> = regiones
            .iter()
            .map(|r| (r.inicio.clone(), r.fin.clone()))
            .collect();
        let trozos = filtros::por_regiones(&prep.intervalos, &limites);
        // Con `limit`, un solo hilo: las partes no se coordinan para parar a las n filas.
        let paralelo = if prep.limit.is_some() {
            1
        } else {
            conexion.destino.paralelo
        };
        let partes = filtros::repartir(&trozos, paralelo);

        let columnas: Vec<(String, Tipo)> = prep
            .salientes
            .iter()
            .map(|(p, i)| (p.clone(), prep.usadas[*i].tipo.clone()))
            .collect();
        let mut escritor = Escritor {
            salida: Some(salida),
            flujo: None,
            schema: Constructor::nuevo(&columnas).map_err(de_lote)?.schema(),
        };
        let parar = AtomicBool::new(false);
        // La cancelacion de **esta** lectura: la levanta quien cancela el proceso (`token`) o el plazo. Los reintentos
        // del lector la miran, asi que un servidor colgado no puede alargar la lectura mas alla de su plazo.
        let lectura = Cancelacion::nueva();
        let terminada = AtomicBool::new(false);
        let vencio = AtomicBool::new(false);
        let limite = p
            .timeout_ms
            .map(|ms| Instant::now() + std::time::Duration::from_millis(ms));
        let comun = Compartido {
            cliente: &conexion.cliente,
            tabla_hbase: &tabla_hbase,
            prep: &prep,
            token: &lectura,
            parar: &parar,
        };

        let resultado = std::thread::scope(|sc| -> Result<u64, Fallo> {
            sc.spawn(|| {
                while !terminada.load(Ordering::Relaxed) {
                    if token.cancelada() {
                        lectura.cancelar();
                    }
                    if limite.is_some_and(|l| Instant::now() > l) {
                        vencio.store(true, Ordering::Relaxed);
                        lectura.cancelar();
                    }
                    if lectura.cancelada() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            });
            let hecho = (|| -> Result<u64, Fallo> {
                escritor.recibir(Msg::Abierta)?;
                if partes.len() <= 1 {
                    return match partes.first() {
                        Some(t) => leer_parte(&comun, t, &mut |m| escritor.recibir(m)),
                        None => Ok(0),
                    };
                }
                let (tx, rx) = mpsc::sync_channel::<Msg>(partes.len() * 2);
                std::thread::scope(|sc| -> Result<u64, Fallo> {
                    for parte in &partes {
                        let tx = tx.clone();
                        let comun = &comun;
                        sc.spawn(move || {
                            let hecho = leer_parte(comun, parte, &mut |m| {
                                tx.send(m).map_err(|_| Fallo::origen("la lectura termino"))
                            });
                            let _ = tx.send(match hecho {
                                Ok(n) => Msg::Fin(n),
                                Err(f) => Msg::Error(f),
                            });
                        });
                    }
                    drop(tx);
                    let (mut total, mut error) = (0u64, None::<Fallo>);
                    // Se sigue vaciando el canal aunque haya un error: un hilo bloqueado en `send` no acabaria.
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
                })
            })();
            terminada.store(true, Ordering::Relaxed);
            hecho
        });
        let filas = match resultado {
            // Lo que corto la lectura fue el plazo, no quien cancela.
            Err(_) if vencio.load(Ordering::Relaxed) && !token.cancelada() => {
                return Err(Fallo::new(
                    Codigo::Tiempo,
                    format!(
                        "se agoto el tiempo de la lectura ({} ms)",
                        p.timeout_ms.unwrap_or(0)
                    ),
                ));
            }
            otro => otro?,
        };
        escritor.terminar()?;
        Ok(filas)
    }

    fn estimar(_: &mut Sesion, _: &Peticion) -> Result<String, Fallo> {
        Err(Fallo::operador("este origen todavia no estima"))
    }

    fn cancelar(token: &Token) {
        token.cancelar();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_driver::Filtro;

    fn peticion(cols: &[(&str, &str)]) -> Peticion {
        Peticion {
            url: "hbase://zk?simple=acepto".into(),
            objeto: "ventas.pedidos".into(),
            proyeccion: cols
                .iter()
                .map(|(p, c)| (p.to_string(), c.to_string()))
                .collect(),
            ..Peticion::default()
        }
    }

    fn con_tipos(mut p: Peticion, v: &[(&str, &str, &str)]) -> Peticion {
        p.tipos = v
            .iter()
            .map(|(c, t, _)| (c.to_string(), t.to_string()))
            .collect();
        p.codificaciones = v
            .iter()
            .map(|(c, _, e)| (c.to_string(), e.to_string()))
            .collect();
        p
    }

    #[test]
    fn los_nombres_cortos_de_la_url_se_llevan_a_los_de_hbase_en_todo_lo_que_nombra_una_columna() {
        let mut q = peticion(&[("a", "id"), ("b", "nombre"), ("c", "g:x")]);
        q.url = "hbase://zk?simple=acepto&familia=f&clave=id".into();
        q.filtros = vec![
            Filtro::uno("id", "ge", "5"),
            Filtro::uno("nombre", "eq", "x"),
        ];
        q.tipos = vec![("id".into(), "Integer".into())];
        q.codificaciones = vec![("id".into(), "i64be".into())];
        let n = normalizar(&q).unwrap();
        assert_eq!(
            n.proyeccion
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>(),
            ["_clave", "f:nombre", "g:x"],
            "la familia explicita no se toca"
        );
        assert_eq!(n.filtros[0].columna, "_clave");
        assert_eq!(n.filtros[1].columna, "f:nombre");
        assert_eq!(n.tipos[0].0, "_clave");
        assert_eq!(n.codificaciones[0].0, "_clave");
        // y la peticion se prepara con esos nombres: la clave es entera, y se empuja
        let p = preparar(&q).unwrap();
        assert_eq!(p.clave.oos, "Integer");
        assert_eq!(p.intervalos.len(), 1);
        // sin las opciones, nada cambia
        let mut sin = peticion(&[("a", "id")]);
        sin.url = "hbase://zk?simple=acepto".into();
        assert_eq!(normalizar(&sin).unwrap(), sin);
    }

    #[test]
    fn la_proyeccion_pide_solo_las_celdas_que_salen_agrupadas_por_familia() {
        let p = preparar(&peticion(&[
            ("id", "_clave"),
            ("n", "info:nombre"),
            ("c", "info:ciudad"),
            ("t", "pago:total"),
        ]))
        .unwrap();
        assert_eq!(
            (p.espacio.as_str(), p.tabla.as_str()),
            ("ventas", "pedidos")
        );
        assert_eq!(
            p.a_pedir,
            vec![
                (
                    b"info".to_vec(),
                    vec![b"nombre".to_vec(), b"ciudad".to_vec()]
                ),
                (b"pago".to_vec(), vec![b"total".to_vec()])
            ]
        );
        assert!(!p.solo_claves());
        assert_eq!(p.salientes.len(), 4);
    }

    #[test]
    fn dos_propiedades_de_la_misma_celda_se_piden_y_se_decodifican_una_vez() {
        let p = preparar(&peticion(&[("a", "f:q"), ("b", "f:q")])).unwrap();
        assert_eq!(p.a_pedir, vec![(b"f".to_vec(), vec![b"q".to_vec()])]);
        assert_eq!(p.usadas.len(), 1);
        assert_eq!(
            p.salientes,
            vec![("a".to_string(), 0), ("b".to_string(), 0)],
            "y salen las dos"
        );
    }

    #[test]
    fn solo_la_clave_no_pide_celdas() {
        let p = preparar(&peticion(&[("id", "_clave")])).unwrap();
        assert!(p.solo_claves());
        assert_eq!(p.clave, Tipo::texto());
    }

    #[test]
    fn un_filtro_sobre_una_celda_que_no_sale_tambien_se_pide_y_se_decodifica() {
        let mut q = peticion(&[("id", "_clave")]);
        q.filtros = vec![Filtro::uno("info:estado", "eq", "ok")];
        let p = preparar(&q).unwrap();
        assert_eq!(
            p.a_pedir,
            vec![(b"info".to_vec(), vec![b"estado".to_vec()])]
        );
        assert_eq!(p.residuales.len(), 1);
        assert_eq!(p.usadas[p.residuales[0].0].columna, "info:estado");
    }

    #[test]
    fn un_filtro_sobre_la_clave_recorta_y_no_pide_nada_mas() {
        let mut q = peticion(&[("id", "_clave")]);
        q.filtros = vec![
            Filtro::uno("_clave", "ge", "c"),
            Filtro::uno("_clave", "lt", "m"),
        ];
        let p = preparar(&q).unwrap();
        assert_eq!(p.intervalos, vec![(b"c".to_vec(), b"m".to_vec())]);
        assert!(p.residuales.is_empty() && p.solo_claves());
    }

    #[test]
    fn los_tipos_y_las_codificaciones_de_la_peticion_llegan_a_cada_columna() {
        let q = con_tipos(
            peticion(&[
                ("id", "_clave"),
                ("edad", "info:edad"),
                ("total", "pago:total"),
                ("n", "info:nota"),
            ]),
            &[
                ("_clave", "Integer", "i64be"),
                ("info:edad", "Integer", "i32be"),
                ("pago:total", "Decimal<12, 2>", "decimal"),
            ],
        );
        let p = preparar(&q).unwrap();
        let por_nombre = |c: &str| {
            p.usadas
                .iter()
                .find(|u| u.columna == c)
                .unwrap()
                .tipo
                .clone()
        };
        assert_eq!(por_nombre("_clave").oos, "Integer");
        assert_eq!(p.clave.oos, "Integer");
        assert_eq!(por_nombre("info:edad").cod.nombre(), "i32be");
        assert_eq!(por_nombre("pago:total").oos, "Decimal<12, 2>");
        assert_eq!(
            por_nombre("info:nota"),
            Tipo::texto(),
            "sin declarar es texto"
        );
    }

    #[test]
    fn un_tipo_declarado_sin_su_codificacion_es_un_error_de_esquema_con_la_columna() {
        let mut q = peticion(&[("n", "info:edad")]);
        q.tipos = vec![("info:edad".into(), "Integer".into())];
        let e = preparar(&q).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(
            e.mensaje.contains("info:edad") && e.mensaje.contains("i32be"),
            "{}",
            e.mensaje
        );
        // una declaracion de una columna que no se usa no estorba
        let mut q = peticion(&[("id", "_clave")]);
        q.tipos = vec![("info:edad".into(), "Integer".into())];
        assert!(preparar(&q).is_ok());
    }

    #[test]
    fn un_filtro_sobre_una_clave_entera_se_empuja_y_sobre_una_decimal_se_evalua() {
        let mut q = con_tipos(
            peticion(&[("id", "_clave")]),
            &[("_clave", "Integer", "i32be")],
        );
        q.filtros = vec![Filtro::uno("_clave", "lt", "5")];
        let p = preparar(&q).unwrap();
        assert_eq!(
            p.intervalos.len(),
            2,
            "los no negativos y los negativos: {:?}",
            p.intervalos
        );
        assert!(p.residuales.is_empty());

        let mut q = con_tipos(
            peticion(&[("id", "_clave")]),
            &[("_clave", "Decimal<12, 2>", "decimal")],
        );
        q.filtros = vec![Filtro::uno("_clave", "lt", "5.00")];
        let p = preparar(&q).unwrap();
        assert_eq!(p.intervalos, vec![(vec![], vec![])], "no se empuja");
        assert_eq!(p.residuales.len(), 1);
    }

    #[test]
    fn lo_que_no_se_sabe_hacer_se_rechaza_con_operador_y_nunca_se_ignora() {
        let con = |f: fn(&mut Peticion)| {
            let mut q = peticion(&[("id", "_clave")]);
            f(&mut q);
            preparar(&q).unwrap_err()
        };
        assert_eq!(
            con(|q| q.filtros = vec![Filtro::uno("f:q", "like", "a%")]).codigo,
            Codigo::Operador
        );
        assert_eq!(
            con(|q| q.orden = vec![ore_driver::Orden {
                columna: "_clave".into(),
                descendente: false
            }])
            .codigo,
            Codigo::Operador
        );
        assert_eq!(
            con(|q| q.claves = vec![vec!["a".into()]]).codigo,
            Codigo::Operador
        );
        let e = con(|q| {
            q.cursor = Some("f:ts".into());
            q.start = Some("1".into());
        });
        assert_eq!(e.codigo, Codigo::Operador);
    }

    #[test]
    fn una_columna_que_no_es_clave_ni_familia_cualificador_es_un_error_de_esquema() {
        let e = preparar(&peticion(&[("x", "sinfamilia")])).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
    }

    #[test]
    fn una_fila_se_decodifica_una_vez_por_columna_y_un_valor_malo_dice_columna_y_clave() {
        use ore_hbase_rpc::{Celda, Fila};
        let q = con_tipos(
            peticion(&[("edad", "info:edad"), ("t", "info:t")]),
            &[("info:edad", "Integer", "i32be")],
        );
        let p = preparar(&q).unwrap();
        let fila = |edad: &[u8]| Fila {
            clave: b"r1".to_vec(),
            celdas: vec![
                Celda {
                    familia: b"info".to_vec(),
                    cualificador: b"edad".to_vec(),
                    marca: 1,
                    valor: edad.to_vec(),
                },
                Celda {
                    familia: b"info".to_vec(),
                    cualificador: b"t".to_vec(),
                    marca: 1,
                    valor: b"hola".to_vec(),
                },
            ],
        };
        let v = decodificar_fila(&p, &fila(&[0, 0, 0, 42])).unwrap();
        assert_eq!(
            v,
            vec![Some(Valor::Entero(42)), Some(Valor::Texto("hola".into()))]
        );
        // 3 bytes donde van 4
        let e = decodificar_fila(&p, &fila(&[0, 0, 42])).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(
            e.mensaje.contains("info:edad")
                && e.mensaje.contains("r1")
                && e.mensaje.contains("4 byte"),
            "{}",
            e.mensaje
        );
        // una celda que falta es nula
        let sin = Fila {
            clave: b"r2".to_vec(),
            celdas: vec![],
        };
        assert_eq!(decodificar_fila(&p, &sin).unwrap(), vec![None, None]);
    }

    #[test]
    fn la_clave_de_un_error_es_texto_si_lo_es_y_hexadecimal_si_no() {
        assert_eq!(clave_legible(b"pedido-7"), "pedido-7");
        assert_eq!(clave_legible(&[0, 0, 0, 42]), "0x0000002a");
    }

    #[test]
    fn el_conector_declara_lo_que_sabe() {
        let c = Hbase::CAPACIDADES;
        assert_eq!(c.conector, "ore-read-hbase");
        assert!(c.limit && c.servir && !c.order_by && !c.estimar);
        assert!(
            c.operadores
                .iter()
                .all(|o| ore_driver::OPERADORES.contains(o))
        );
        assert!(!c.operadores.contains(&"like"));
    }

    #[test]
    fn cancelar_levanta_la_bandera_y_leer_la_rearma() {
        let t = Token::default();
        Hbase::cancelar(&t);
        assert!(t.cancelada());
        t.rearmar();
        assert!(!t.cancelada());
    }
}
