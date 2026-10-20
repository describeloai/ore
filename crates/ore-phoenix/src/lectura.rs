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
//! # Qué se empuja en M2
//!
//! **La proyección y el `LIMIT`.** Ningún filtro (`operadores` vacío): lo que se pida con un filtro, una clave,
//! un cursor o un orden se **rechaza** con `operador`, nunca se ignora (ficha 0071 §6, M4 los trae). El motor
//! filtra lo que el conector no sabe.
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
use crate::transporte::{Conexion, fallo_de};
use arrow_ipc::writer::StreamWriter;
use ore_avatica::Valor;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Filas por lote Arrow.
pub const FILAS_POR_LOTE: usize = 8192;

pub struct Phoenix;

/// Lo que une a quien lee con quien cancela: la bandera. El PQS no tiene `killOp` (X0 §3.8): cancelar es
/// **dejar de pedir frames**, y el servidor acaba solo el que tuviera en curso.
#[derive(Clone, Default)]
pub struct Token {
    bandera: Arc<AtomicBool>,
}

impl Token {
    pub fn cancelada(&self) -> bool {
        self.bandera.load(Ordering::Relaxed)
    }
}

pub struct Sesion {
    conexion: Conexion,
    /// Por objeto, tal como lo pidió la petición.
    tablas: HashMap<String, Tabla>,
}

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

/// Lo que M2 no sabe empujar, rechazado con `operador` (§6). `Capacidades::admite` ya rechaza los filtros, el
/// `orderBy` y lo que no se declara; esto es lo demás de la petición: el recorte por clave y el rango.
pub fn rechazar_lo_que_no_se_empuja(p: &Peticion) -> Result<(), Fallo> {
    Phoenix::CAPACIDADES.admite(p)?;
    if !p.claves.is_empty() {
        return Err(Fallo::operador(
            "el recorte por clave todavía no se empuja a Phoenix (ficha 0071, M4)",
        ));
    }
    if p.cursor.is_some() || p.start.is_some() || p.end.is_some() {
        return Err(Fallo::operador(
            "el rango (cursor, start, end) todavía no se empuja a Phoenix (ficha 0071, M4/M7)",
        ));
    }
    Ok(())
}

/// **La consulta de una petición**: las columnas distintas de la proyección, en su orden, y el `SELECT`. Pura
/// (sin servidor): lo que se asegura de ella es un aserto y no una promesa.
///
/// Dos propiedades pueden salir de la misma columna: se pide **una vez** (un resultado con el nombre repetido
/// se leería mal) y la fila se arma después por posición.
pub fn consulta(p: &Peticion, t: &Tabla) -> Result<(String, Vec<String>), Fallo> {
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
    if let Some(n) = p.limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }
    Ok((sql, leidas))
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

impl Origen for Phoenix {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-phoenix",
        version: env!("CARGO_PKG_VERSION"),
        // M2 no empuja ningún filtro: lo que se pida se rechaza (M4 los trae).
        operadores: &[],
        limit: true,
        order_by: false,
        estimar: false,
        servir: true,
    };

    fn conectar(url: &str) -> Result<Sesion, Fallo> {
        Ok(Sesion {
            conexion: Conexion::abrir(url)?,
            tablas: HashMap::new(),
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
        let tabla = con_declarados(&tabla, &p.tipos, &p.objeto)?;
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
        let mut lectura = conexion.consulta(&sql)?;
        if lectura.columnas().len() != leidas.len() {
            return Err(Fallo::origen(format!(
                "Phoenix devolvió {} columnas y se pidieron {}",
                lectura.columnas().len(),
                leidas.len()
            )));
        }

        let mut cons = Constructor::nuevo(&columnas).map_err(de_lote)?;
        let mut flujo = StreamWriter::try_new(&mut *salida, &cons.schema())
            .map_err(|e| Fallo::origen(format!("no se pudo empezar el flujo: {e}")))?;
        let escribir = |cons: &mut Constructor, flujo: &mut StreamWriter<&mut dyn Write>| {
            let lote = cons.finish().map_err(de_lote)?;
            flujo
                .write(&lote)
                .and_then(|_| flujo.flush())
                .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo escribir: {e}")))
        };

        let mut filas = 0u64;
        while let Some(frame) = lectura.siguiente_frame().map_err(fallo_de)? {
            for fila in &frame {
                let celdas: Vec<Valor> = posiciones.iter().map(|i| fila[*i].clone()).collect();
                cons.push(&celdas).map_err(de_lote)?;
                filas += 1;
                if cons.filas() >= FILAS_POR_LOTE {
                    escribir(&mut cons, &mut flujo)?;
                }
            }
            if token.cancelada() {
                return Err(Fallo::origen(format!("cancelada con {filas} filas leídas")));
            }
        }
        if cons.filas() > 0 {
            escribir(&mut cons, &mut flujo)?;
        }
        flujo.finish().map_err(|e| {
            Fallo::new(Codigo::Conexion, format!("no se pudo cerrar el flujo: {e}"))
        })?;
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
        assert_eq!(
            sql,
            "SELECT CAST(\"D\" AS TIMESTAMP) AS \"D\", \"TS\" FROM \"T\""
        );
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
        let mut p = base.clone();
        p.filtros = vec![Filtro::uno("ID", "eq", "1")];
        let e = rechazar_lo_que_no_se_empuja(&p).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("`eq` sobre `ID`"), "{}", e.mensaje);
        let mut p = base.clone();
        p.orden = vec![Orden {
            columna: "ID".into(),
            descendente: false,
        }];
        assert_eq!(
            rechazar_lo_que_no_se_empuja(&p).unwrap_err().codigo,
            Codigo::Operador
        );
        let mut p = base.clone();
        p.clave_columnas = vec!["ID".into()];
        p.claves = vec![vec!["1".into()]];
        assert!(
            rechazar_lo_que_no_se_empuja(&p)
                .unwrap_err()
                .mensaje
                .contains("clave")
        );
        for poner in [
            (|p: &mut Peticion| p.cursor = Some("CREADO".into())) as fn(&mut Peticion),
            |p| p.start = Some("1".into()),
            |p| p.end = Some("9".into()),
        ] {
            let mut p = base.clone();
            poner(&mut p);
            assert!(
                rechazar_lo_que_no_se_empuja(&p)
                    .unwrap_err()
                    .mensaje
                    .contains("rango")
            );
        }
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
        assert!(c.operadores.is_empty() && c.limit && c.servir && !c.order_by && !c.estimar);
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
}
