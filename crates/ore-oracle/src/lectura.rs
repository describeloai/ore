//! **`leer` en Arrow**: Oracle como [`Origen`] (`ore_driver::origen`).
//!
//! Lo único de este módulo es lo que es de Oracle: de qué tabla se lee, con qué tipos y cómo se vuelve flujo.
//! Los verbos, `SIGTERM` y las sesiones de `servir` son del esqueleto.
//!
//! # El objeto
//!
//! `ESQUEMA.TABLA`, partido en el **primer** punto. Los nombres son los del catálogo (mayúsculas salvo que se
//! hayan creado entre comillas) y se citan siempre.
//!
//! # El tipo de cada columna
//!
//! Oracle **tiene catálogo** (`ALL_TAB_COLUMNS`): los tipos no se infieren. Se leen al primer uso de la tabla y
//! se guardan en la sesión (`servir` reutiliza la conexión). Los que la petición declara (`tipos`) mandan si el
//! dato los puede llevar ([`Tipo::declarado`]); si no, es `esquema`, con la columna.
//!
//! # El flujo
//!
//! La cabecera Arrow se escribe **después** de preparar la lectura (la sentencia): lo que falla ahí sale como un
//! error limpio y no como un flujo a medias. Después, cada lote de [`FILAS_POR_LOTE`] filas sale en cuanto está,
//! mientras el servidor sigue mandando.
//!
//! # Cancelar
//!
//! Medido el 2026-10-25: `oracledb` no tiene una llamada para interrumpir una sentencia. Se hace con lo que hay,
//! en tres escalones: la **bandera** (se mira en cada fila, y mientras las filas fluyen corta en milisegundos);
//! `ALTER SYSTEM KILL SESSION` desde otra conexión **si el usuario puede** (corta también una sentencia que aún
//! no ha devuelto la primera fila; muere en 0,08 s); y `timeoutMs`, que cierra la conexión al vencer.

use crate::atado::atar;
use crate::lote::{Constructor, ErrorLote};
use crate::tipos::Tipo;
use crate::transporte::{Conexion, Tabla, celda, fallo_de, fallo_leyendo};
use oracledb::ToDbValue;
use ore_arrow::Emisor;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, OPERADORES, Peticion};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Filas por lote Arrow.
pub const FILAS_POR_LOTE: usize = 8192;

pub struct Oracle;

/// Lo que une a quien lee con quien cancela.
#[derive(Clone, Default)]
pub struct Token {
    bandera: Arc<AtomicBool>,
    /// La sesión que lee, para matarla desde otra: `(url, "sid,serial#")`. Solo si el usuario puede verla.
    victima: Arc<Mutex<Option<(String, String)>>>,
}

impl Token {
    pub fn cancelada(&self) -> bool {
        self.bandera.load(Ordering::Relaxed)
    }
}

pub struct Sesion {
    url: String,
    conexion: Conexion,
    /// Por `ESQUEMA.TABLA`, tal como las dijo el catálogo.
    tablas: HashMap<String, Tabla>,
    /// `"sid,serial#"` de esta sesión, o `None` si el usuario no puede verlo (se pregunta una vez).
    identidad: Option<Option<String>>,
}

/// `(esquema, tabla)` de un objeto `ESQUEMA.TABLA`.
pub fn partir_objeto(objeto: &str) -> Result<(&str, &str), Fallo> {
    match objeto.split_once('.') {
        Some((e, t)) if !e.is_empty() && !t.is_empty() => Ok((e, t)),
        _ => Err(Fallo::operador(format!(
            "el objeto `{objeto}` no es `ESQUEMA.TABLA`"
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
            c.tipo = c.tipo.declarado(declarado).map_err(|m| {
                Fallo::new(
                    Codigo::Esquema,
                    format!("`{objeto}`, columna `{columna}`: {m}"),
                )
            })?;
        }
    }
    Ok(t)
}

fn de_lote(e: ErrorLote) -> Fallo {
    e.into()
}

impl Sesion {
    /// La tabla, de la caché de la sesión. Si la petición pide una columna que la copia guardada no tiene, la
    /// tabla pudo cambiar: se vuelve a leer una vez.
    fn tabla(&mut self, p: &Peticion) -> Result<Tabla, Fallo> {
        let (esquema, nombre) = partir_objeto(&p.objeto)?;
        let falta = |t: &Tabla| p.proyeccion.iter().any(|(_, c)| t.columna(c).is_none());
        if let Some(t) = self.tablas.get(&p.objeto)
            && !falta(t)
        {
            return Ok(t.clone());
        }
        let t = self.conexion.tabla(esquema, nombre)?;
        self.tablas.insert(p.objeto.clone(), t.clone());
        Ok(t)
    }

    /// El `sid,serial#` de esta sesión, si el usuario puede leer `V$SESSION`.
    fn identidad(&mut self) -> Option<String> {
        if self.identidad.is_none() {
            let r = self.conexion.c.query_row(
                "SELECT sid || ',' || serial# FROM v$session \
                 WHERE audsid = SYS_CONTEXT('USERENV', 'SESSIONID') AND ROWNUM = 1",
                &[],
            );
            self.identidad = Some(r.ok().and_then(|f| f.get::<String>(0).ok()));
        }
        self.identidad.clone().flatten()
    }
}

impl Origen for Oracle {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-oracle",
        version: env!("CARGO_PKG_VERSION"),
        // Los diez: Oracle filtra lo que se le pide.
        operadores: OPERADORES,
        limit: true,
        order_by: true,
        estimar: false,
        servir: true,
    };

    fn conectar(url: &str) -> Result<Sesion, Fallo> {
        Ok(Sesion {
            url: url.to_string(),
            conexion: Conexion::abrir(url)?,
            tablas: HashMap::new(),
            identidad: None,
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
        let tabla = s.tabla(p)?;
        let tabla = con_declarados(&tabla, &p.tipos, &p.objeto)?;

        // La consulta: la forma común con el dialecto de Oracle. No hay tipos que traer (`:1` se coacciona).
        let consulta = ore_sql::preparar(p, &ore_sql::dialectos::ORACLE, &p.objeto, || {
            Ok(BTreeMap::new())
        })
        .map_err(Fallo::operador)?;

        // Cada propiedad pedida: su columna del catálogo (que tiene que existir y poderse leer) y dónde está
        // entre las que el `SELECT` trae.
        let mut columnas = Vec::new();
        let mut leidas = Vec::new();
        for (prop, col) in &p.proyeccion {
            let c = tabla.columna(col).ok_or_else(|| {
                Fallo::new(
                    Codigo::Esquema,
                    format!("`{}` no tiene la columna `{col}`", p.objeto),
                )
            })?;
            if !c.tipo.se_lee() {
                return Err(Fallo::new(
                    Codigo::Esquema,
                    format!(
                        "la columna `{col}` de `{}` es {}, que este origen no lee",
                        p.objeto, c.tipo_oracle
                    ),
                ));
            }
            let pos = consulta
                .columnas
                .iter()
                .position(|x| x == col)
                .expect("el `SELECT` pide la proyección");
            columnas.push((prop.clone(), c.tipo.clone()));
            leidas.push((pos, c.clone()));
        }

        // Para poder matarla desde otra sesión (al cancelar, o al vencer `timeoutMs`).
        let id_sesion = s.identidad();
        let url_sesion = s.url.clone();
        if let Some(id) = &id_sesion {
            *token.victima.lock().unwrap() = Some((url_sesion.clone(), id.clone()));
        }
        // Al vencer el tiempo el driver cierra SU lado; la consulta sigue en el servidor hasta acabar. Si el
        // usuario puede, se mata: una consulta abandonada no debe seguir gastando la base de otro.
        let tras_el_tiempo = |f: Fallo| {
            if f.codigo == Codigo::Tiempo
                && let Some(id) = &id_sesion
            {
                matar(&url_sesion, id);
            }
            f
        };
        if let Some(ms) = p.timeout_ms {
            s.conexion
                .c
                .set_call_timeout(Some(Duration::from_millis(ms)))
                .map_err(fallo_de)?;
        }

        let lote = s.conexion.destino.filas_por_viaje;
        let inicio = Instant::now();
        let sentencia = s
            .conexion
            .c
            .statement(&consulta.texto)
            .map_err(fallo_de)?
            .fetch_array_size(lote)
            .prefetch_rows(lote)
            .build()
            .map_err(fallo_de)?;
        // Cada parámetro, atado con el tipo de la columna con que se compara (no como el texto que la sesión
        // sepa leer).
        let atados = atar(
            &tabla,
            &consulta.parametros,
            &consulta.columnas_de_parametros,
        )?;
        let binds: Vec<&dyn ToDbValue> = atados.iter().map(|b| b.as_ref()).collect();
        let cursor = sentencia
            .query(&binds)
            .map_err(|e| tras_el_tiempo(fallo_leyendo(e, p.timeout_ms, inicio)))?;

        let mut cons = Constructor::nuevo(&columnas).map_err(de_lote)?;
        let mut flujo = Emisor::nuevo(&mut *salida, &cons.schema())?;
        let escribir = |cons: &mut Constructor, flujo: &mut Emisor<&mut dyn Write>| {
            let lote = cons.finish().map_err(de_lote)?;
            flujo.lote(&lote)
        };

        let mut filas = 0u64;
        let mut cancelada = false;
        let mut texto: Vec<Option<String>> = Vec::with_capacity(leidas.len());
        for fila in cursor {
            if token.cancelada() {
                cancelada = true;
                break;
            }
            let fila = match fila {
                Ok(f) => f,
                // Si nos mataron la sesión, el error es nuestro y se dice.
                Err(_) if token.cancelada() => {
                    cancelada = true;
                    break;
                }
                Err(e) => return Err(tras_el_tiempo(fallo_leyendo(e, p.timeout_ms, inicio))),
            };
            texto.clear();
            for (pos, c) in &leidas {
                texto.push(celda(&fila, *pos, c)?);
            }
            cons.push(&texto).map_err(de_lote)?;
            filas += 1;
            if cons.filas() >= FILAS_POR_LOTE {
                escribir(&mut cons, &mut flujo)?;
            }
        }
        if cancelada {
            return Err(Fallo::origen(format!("cancelada con {filas} filas leídas")));
        }
        if cons.filas() > 0 {
            escribir(&mut cons, &mut flujo)?;
        }
        flujo.fin()?;
        for c in &tabla.columnas {
            if c.tipo == Tipo::FechaHora && c.tipo_oracle.starts_with("TIMESTAMP") {
                // Arrow guarda microsegundos: los nanosegundos de un `TIMESTAMP(7..9)` se pierden.
                if c.escala.unwrap_or(0) > 6 {
                    eprintln!(
                        "ore-read-oracle: aviso · `{}`.`{}` guarda nanosegundos y se lee a microsegundos",
                        p.objeto, c.nombre
                    );
                }
            }
        }
        Ok(filas)
    }

    fn estimar(_: &mut Sesion, _: &Peticion) -> Result<String, Fallo> {
        Err(Fallo::operador("este origen todavía no estima"))
    }

    fn cancelar(token: &Token) {
        token.bandera.store(true, Ordering::SeqCst);
        // El segundo escalón, si el usuario puede: mata la sesión que lee. Sin el permiso (`ALTER SYSTEM`) no
        // pasa nada: la bandera y `timeoutMs` siguen ahí.
        let victima = token.victima.lock().unwrap().take();
        if let Some((url, id)) = victima {
            matar(&url, &id);
        }
    }
}

/// Mata la sesión `sid,serial#` desde otra conexión. Sin el permiso (`ALTER SYSTEM`) no pasa nada.
fn matar(url: &str, id: &str) {
    if let Ok(otra) = Conexion::abrir(url) {
        let _ = otra
            .c
            .execute(&format!("ALTER SYSTEM KILL SESSION '{id}' IMMEDIATE"), &[]);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::transporte::Columna;

    fn tabla() -> Tabla {
        let c = |n: &str, t: &str, p, s| Columna {
            nombre: n.into(),
            tipo: Tipo::de(t, p, s),
            tipo_oracle: t.into(),
            precision: p,
            escala: s,
            anulable: true,
            descripcion: None,
        };
        Tabla {
            esquema: "VENTAS".into(),
            nombre: "PEDIDOS".into(),
            columnas: vec![
                c("ID", "NUMBER", Some(10), Some(0)),
                c("TOTAL", "NUMBER", None, None),
                c("NOTA", "VARCHAR2", None, None),
                c("HORA", "TIMESTAMP(6)", None, Some(6)),
            ],
            clave: vec!["ID".into()],
            unicas: vec![],
        }
    }

    #[test]
    fn el_objeto_se_parte_en_el_primer_punto() {
        assert_eq!(
            partir_objeto("VENTAS.PEDIDOS").unwrap(),
            ("VENTAS", "PEDIDOS")
        );
        assert_eq!(partir_objeto("A.b.c").unwrap(), ("A", "b.c"));
        for malo in ["", "solo", ".t", "e."] {
            assert!(partir_objeto(malo).is_err(), "{malo}");
        }
    }

    #[test]
    fn lo_declarado_manda_si_el_dato_lo_lleva() {
        let t = tabla();
        let d = con_declarados(
            &t,
            &[
                ("TOTAL".into(), "Decimal<12, 2>".into()),
                ("FANTASMA".into(), "Integer".into()),
            ],
            "VENTAS.PEDIDOS",
        )
        .unwrap();
        assert_eq!(
            d.columna("TOTAL").unwrap().tipo,
            Tipo::Decimal {
                precision: 12,
                escala: 2
            }
        );
        // La tabla original no cambia: la caché de la sesión sigue siendo la del servidor.
        assert_eq!(t.columna("TOTAL").unwrap().tipo, Tipo::DecimalLibre);
        // Un tipo que el dato no lleva nombra la columna.
        let e =
            con_declarados(&t, &[("NOTA".into(), "Integer".into())], "VENTAS.PEDIDOS").unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("columna `NOTA`"), "{}", e.mensaje);
    }

    #[test]
    fn el_conector_declara_lo_que_sabe() {
        let c = Oracle::CAPACIDADES;
        assert_eq!(c.conector, "ore-read-oracle");
        assert_eq!(c.operadores.len(), OPERADORES.len());
        assert!(c.limit && c.order_by && c.servir && !c.estimar);
    }
}
