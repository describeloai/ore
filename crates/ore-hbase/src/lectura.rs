//! **Leer una tabla de HBase** como el conector v2 del esqueleto (`ore_driver::origen::Origen`): una peticion entra,
//! un flujo Arrow IPC sale.
//!
//! ```text
//!   Peticion ─preparar─▶ Preparada { columnas, que pedir, intervalos de clave, residuales }     [puro]
//!   intervalos ─por regiones─▶ trozos ─repartir─▶ hasta `paralelo` grupos                          [puro]
//!   cada grupo ─un hilo─▶ Lector (ore-hbase-rpc: reintentos, reanuda por la ultima clave) ─▶ lotes Arrow
//! ```
//!
//! Lo que se empuja a HBase es el **rango de la clave**; lo demas se evalua aqui, con la semantica de SQL
//! ([`crate::filtros`]). `orderBy` no se declara (HBase solo ordena por clave y el reparto en paralelo no la
//! conserva), `limit` si, y con el la lectura va en un solo hilo. El incremental (`start`/`end`) llega en N5.

use crate::esquema::{CLAVE, Tipo, en_hbase, partir_columna, partir_objeto};
use crate::filtros::{self, Intervalo, Residual};
use crate::lote::{Constructor, ErrorLote};
use crate::transporte::{Conexion, fallo_de};
use arrow_array::RecordBatch;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::SchemaRef;
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

/// Una columna que sale en la fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saliente {
    /// El nombre de la propiedad: lo que sale en la cabecera del flujo.
    pub propiedad: String,
    /// `_clave` o `familia:cualificador`.
    pub columna: String,
    pub tipo: Tipo,
}

/// Una peticion entendida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preparada {
    pub espacio: String,
    pub tabla: String,
    pub salientes: Vec<Saliente>,
    /// Lo que se pide a HBase: `(familia, cualificadores)`. Vacio: solo la clave.
    pub a_pedir: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    pub intervalos: Vec<Intervalo>,
    pub residuales: Vec<Residual>,
    pub limit: Option<u64>,
}

impl Preparada {
    pub fn solo_claves(&self) -> bool {
        self.a_pedir.is_empty()
    }
}

/// Lo que la peticion declara de una columna, o `String`.
fn tipo_declarado(p: &Peticion, columna: &str) -> Result<Tipo, Fallo> {
    match p.tipos.iter().find(|(c, _)| c == columna) {
        Some((_, t)) => Tipo::declarado(t, &p.objeto, columna),
        None => Ok(Tipo::Texto),
    }
}

/// **Entiende la peticion** (pura): rechaza lo que no sabe hacer, resuelve tipos, y separa lo que va a HBase de lo
/// que se evalua aqui.
pub fn preparar(p: &Peticion) -> Result<Preparada, Fallo> {
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
    let validar = |c: &str| -> Result<(), Fallo> {
        if c == CLAVE {
            return Ok(());
        }
        partir_columna(c)
            .map(|_| ())
            .map_err(|m| Fallo::new(Codigo::Esquema, format!("`{}`: {m}", p.objeto)))
    };
    let mut salientes = Vec::with_capacity(p.proyeccion.len());
    for (propiedad, columna) in &p.proyeccion {
        validar(columna)?;
        salientes.push(Saliente {
            propiedad: propiedad.clone(),
            columna: columna.clone(),
            tipo: tipo_declarado(p, columna)?,
        });
    }
    // Los tipos de las columnas que solo aparecen en un filtro tambien se miran.
    let mut tipos_de_filtro: BTreeMap<String, Tipo> = BTreeMap::new();
    for f in &p.filtros {
        validar(&f.columna)?;
        tipos_de_filtro.insert(f.columna.clone(), tipo_declarado(p, &f.columna)?);
    }
    let clave = tipos_de_filtro.get(CLAVE).copied().unwrap_or(Tipo::Texto);
    let plan = filtros::plan(
        &p.filtros,
        clave,
        &|c| tipos_de_filtro.get(c).copied().unwrap_or(Tipo::Texto),
        &p.objeto,
    )?;

    // Las celdas que hay que traer: las de la proyeccion y las de los residuales, cada una una vez.
    let mut por_familia: BTreeMap<Vec<u8>, Vec<Vec<u8>>> = BTreeMap::new();
    let columnas = salientes
        .iter()
        .map(|s| s.columna.as_str())
        .chain(plan.residuales.iter().map(|r| r.columna.as_str()));
    for c in columnas.filter(|c| *c != CLAVE) {
        let (f, q) = partir_columna(c).expect("validada");
        let qs = por_familia.entry(f).or_default();
        if !qs.contains(&q) {
            qs.push(q);
        }
    }
    Ok(Preparada {
        espacio,
        tabla,
        salientes,
        a_pedir: por_familia.into_iter().collect(),
        intervalos: plan.intervalos,
        residuales: plan.residuales,
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

/// Lo que comparten las partes de una lectura.
struct Compartido<'a> {
    cliente: &'a ore_hbase_rpc::Cliente,
    tabla_hbase: &'a str,
    prep: &'a Preparada,
    token: &'a Token,
    parar: &'a AtomicBool,
}

/// Una columna ya partida en `(familia, cualificador)`; `None` es la clave de fila.
type Particion = Option<(Vec<u8>, Vec<u8>)>;

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
        .map(|s| (s.propiedad.clone(), s.tipo))
        .collect();
    let mut cons = Constructor::nuevo(&columnas);
    // Cada columna, ya partida: no se parte por fila.
    let partidas: Vec<Particion> = prep
        .salientes
        .iter()
        .map(|s| (s.columna != CLAVE).then(|| partir_columna(&s.columna).expect("validada")))
        .collect();
    let residuales: Vec<(Particion, &Residual)> = prep
        .residuales
        .iter()
        .map(|r| {
            (
                (r.columna != CLAVE).then(|| partir_columna(&r.columna).expect("validada")),
                r,
            )
        })
        .collect();
    let mut filas = 0u64;
    for (inicio, fin) in intervalos {
        let consulta = Consulta {
            inicio: inicio.clone(),
            fin: fin.clone(),
            columnas: prep.a_pedir.clone(),
            filas_por_llamada: FILAS_POR_LLAMADA,
            filtro: prep
                .solo_claves()
                .then(|| (SOLO_PRIMERA_CELDA.to_string(), Vec::new())),
            ..Consulta::default()
        };
        let lector = c
            .cliente
            .leer(c.tabla_hbase, &consulta, c.token)
            .map_err(fallo_de)?;
        for fila in lector {
            let fila = match fila {
                Ok(f) => f,
                Err(_) if c.token.cancelada() => return Err(cancelada(filas)),
                Err(e) => return Err(fallo_de(e)),
            };
            // los residuales, sobre la fila entera
            let valor = |q: &Particion| -> Option<&[u8]> {
                match q {
                    None => Some(fila.clave.as_slice()),
                    Some((f, q)) => fila.valor(f, q),
                }
            };
            let cumple = residuales.iter().all(|(q, r)| {
                filtros::cumple(std::slice::from_ref(*r), &|_| valor(q).map(<[u8]>::to_vec))
            });
            if !cumple {
                continue;
            }
            let celdas: Vec<Option<&[u8]>> = partidas.iter().map(valor).collect();
            cons.push(&celdas).map_err(de_lote)?;
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
            .map(|s| (s.propiedad.clone(), s.tipo))
            .collect();
        let mut escritor = Escritor {
            salida: Some(salida),
            flujo: None,
            schema: Constructor::nuevo(&columnas).schema(),
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
    fn dos_propiedades_de_la_misma_celda_se_piden_una_vez() {
        let p = preparar(&peticion(&[("a", "f:q"), ("b", "f:q")])).unwrap();
        assert_eq!(p.a_pedir, vec![(b"f".to_vec(), vec![b"q".to_vec()])]);
        assert_eq!(p.salientes.len(), 2, "y salen las dos");
    }

    #[test]
    fn solo_la_clave_no_pide_celdas() {
        let p = preparar(&peticion(&[("id", "_clave")])).unwrap();
        assert!(p.solo_claves());
    }

    #[test]
    fn un_filtro_sobre_una_celda_que_no_sale_tambien_se_pide() {
        let mut q = peticion(&[("id", "_clave")]);
        q.filtros = vec![Filtro::uno("info:estado", "eq", "ok")];
        let p = preparar(&q).unwrap();
        assert_eq!(
            p.a_pedir,
            vec![(b"info".to_vec(), vec![b"estado".to_vec()])]
        );
        assert_eq!(p.residuales.len(), 1);
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
    fn lo_declarado_que_no_es_string_ni_opaque_es_un_error_de_esquema_con_la_columna() {
        let mut q = peticion(&[("n", "info:edad")]);
        q.tipos = vec![("info:edad".into(), "Integer".into())];
        let e = preparar(&q).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("info:edad"), "{}", e.mensaje);
        // una declaracion de una columna que no se usa no estorba
        let mut q = peticion(&[("id", "_clave")]);
        q.tipos = vec![("info:edad".into(), "Integer".into())];
        assert!(preparar(&q).is_ok());
    }

    #[test]
    fn una_columna_que_no_es_clave_ni_familia_cualificador_es_un_error_de_esquema() {
        let e = preparar(&peticion(&[("x", "sinfamilia")])).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
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
