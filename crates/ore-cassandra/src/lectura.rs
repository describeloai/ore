//! **`leer` en Arrow**: Cassandra como [`Origen`] (`ore_driver::origen`).
//!
//! Lo único de este módulo es lo que es de Cassandra: de qué tabla se lee, con qué
//! tipos y cómo se vuelve flujo. Los verbos, `SIGTERM` y las sesiones de `servir`
//! son del esqueleto.
//!
//! # El objeto
//!
//! `keyspace.tabla`, partido en el **primer** punto: ni el nombre de un keyspace ni
//! el de una tabla pueden llevarlo.
//!
//! # El tipo de cada columna
//!
//! Cassandra **tiene catálogo** (`system_schema`): los tipos no se infieren. Se leen
//! al primer uso de la tabla y se guardan en la sesión (`servir` reutiliza la
//! conexión). Los tipos que la petición declara (`tipos`) mandan si el dato los
//! puede llevar ([`Tipo::declarado`]); si no, es `esquema`, con la columna.
//!
//! # El flujo
//!
//! La cabecera Arrow se escribe **después** de preparar la lectura (la sentencia,
//! el anillo de tokens): lo que falla ahí sale como un error limpio y no como un
//! flujo a medias. Después, cada lote de [`FILAS_POR_LOTE`] filas sale en cuanto
//! está, mientras los rangos siguen leyendo.

use crate::esquema::Tabla;
use crate::lote::{Constructor, ErrorLote};
use crate::tipos::Tipo;
use crate::traductor::{OPERADORES, Opciones, traducir};
use crate::transporte::{Conexion, PARALELO};
use arrow_ipc::writer::StreamWriter;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use scylla::value::CqlValue;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Filas por lote Arrow.
pub const FILAS_POR_LOTE: usize = 8192;

pub struct Cassandra;

/// Lo que une a quien lee con quien cancela: la bandera. Cassandra no tiene `killOp`
/// (X0 §3.5): cancelar es **dejar de pedir páginas**, y el servidor acaba solo la
/// que tuviera en curso.
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
    /// Por `keyspace.tabla`, tal como las dijo `system_schema`.
    tablas: HashMap<String, Tabla>,
}

/// `(keyspace, tabla)` de un objeto `keyspace.tabla`.
pub fn partir_objeto(objeto: &str) -> Result<(&str, &str), Fallo> {
    match objeto.split_once('.') {
        Some((k, t)) if !k.is_empty() && !t.is_empty() => Ok((k, t)),
        _ => Err(Fallo::operador(format!(
            "el objeto `{objeto}` no es `keyspace.tabla`"
        ))),
    }
}

/// **La tabla con lo que la petición declara por encima** (`tipos`). Lo declarado
/// manda si el dato lo puede llevar; si no, `esquema` con la columna nombrada, nunca
/// una conversión a escondidas. Una declaración de una columna que la tabla no
/// tiene se ignora: no es de esta lectura.
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
    /// La tabla, de la caché de la sesión. Si la petición pide una columna que la
    /// copia guardada no tiene, la tabla pudo cambiar: se vuelve a leer una vez.
    fn tabla(&mut self, p: &Peticion) -> Result<Tabla, Fallo> {
        let (ks, nombre) = partir_objeto(&p.objeto)?;
        let falta = |t: &Tabla| p.proyeccion.iter().any(|(_, c)| t.columna(c).is_none());
        if let Some(t) = self.tablas.get(&p.objeto)
            && !falta(t)
        {
            return Ok(t.clone());
        }
        let t = self.conexion.tabla(ks, nombre)?;
        self.tablas.insert(p.objeto.clone(), t.clone());
        Ok(t)
    }
}

impl Origen for Cassandra {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-cassandra",
        version: env!("CARGO_PKG_VERSION"),
        // Los diez: lo que el servidor no sabe filtrar lo filtra el conector.
        operadores: OPERADORES,
        limit: true,
        // El servidor solo ordena dentro de una partición.
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
        // El token es de todo el proceso: el de la lectura anterior puede venir
        // cancelado.
        token.bandera.store(false, Ordering::SeqCst);
        let tabla = s.tabla(p)?;
        let tabla = con_declarados(&tabla, &p.tipos, &p.objeto)?;
        let t = traducir(p, &tabla, &Opciones::default())?;
        let prep = s.conexion.preparar(&t.plan, &tabla, PARALELO)?;

        let columnas: Vec<(String, Tipo)> = t
            .rutas
            .iter()
            .map(|(prop, col)| {
                (
                    prop.clone(),
                    tabla
                        .columna(col)
                        .expect("comprobada en traducir")
                        .tipo
                        .clone(),
                )
            })
            .collect();
        // Dónde está cada columna pedida entre las que el `SELECT` trae.
        let posiciones: Vec<usize> = t
            .rutas
            .iter()
            .map(|(_, col)| {
                t.plan
                    .leidas
                    .iter()
                    .position(|c| c == col)
                    .expect("el `SELECT` pide la proyección")
            })
            .collect();

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
        let fin = s
            .conexion
            .correr(&t.plan, &tabla, &prep, &token.bandera, &mut |fila| {
                let salida: Vec<Option<CqlValue>> =
                    posiciones.iter().map(|i| fila[*i].clone()).collect();
                cons.push(&salida).map_err(de_lote)?;
                if cons.filas() >= FILAS_POR_LOTE {
                    escribir(&mut cons, &mut flujo)?;
                }
                Ok(())
            })?;
        if fin.cancelada {
            return Err(Fallo::origen(format!(
                "cancelada con {} filas leídas",
                fin.filas
            )));
        }
        if cons.filas() > 0 {
            escribir(&mut cons, &mut flujo)?;
        }
        flujo.finish().map_err(|e| {
            Fallo::new(Codigo::Conexion, format!("no se pudo cerrar el flujo: {e}"))
        })?;
        for a in &tabla.avisos {
            eprintln!("ore-read-cassandra: aviso · `{}`: {a}", p.objeto);
        }
        Ok(fin.filas)
    }

    fn estimar(_: &mut Sesion, _: &Peticion) -> Result<String, Fallo> {
        Err(Fallo::operador("este origen todavía no estima"))
    }

    fn cancelar(token: &Token) {
        token.bandera.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esquema::pruebas::pedidos;

    #[test]
    fn el_objeto_se_parte_en_el_primer_punto() {
        assert_eq!(partir_objeto("ks.tabla").unwrap(), ("ks", "tabla"));
        assert_eq!(partir_objeto("ks.a.b").unwrap(), ("ks", "a.b"));
        for malo in ["", "solo", ".t", "k."] {
            assert!(partir_objeto(malo).is_err(), "{malo}");
        }
    }

    #[test]
    fn lo_declarado_manda_si_el_dato_lo_lleva() {
        let t = pedidos(&[]);
        let d = con_declarados(
            &t,
            &[
                ("total".into(), "Decimal<12, 2>".into()),
                ("hora".into(), "DateTime".into()),
                ("fantasma".into(), "Integer".into()),
            ],
            "k.t",
        )
        .unwrap();
        assert_eq!(
            d.columna("total").unwrap().tipo,
            Tipo::Decimal {
                precision: 12,
                escala: 2
            }
        );
        assert_eq!(d.columna("hora").unwrap().tipo, Tipo::FechaHora);
        // La tabla original no cambia: la caché de la sesión sigue siendo la del servidor.
        assert_eq!(t.columna("total").unwrap().tipo, Tipo::DecimalLibre);
        // Un tipo que el dato no lleva nombra la columna.
        let e = con_declarados(&t, &[("estado".into(), "Integer".into())], "k.t").unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("columna `estado`"), "{}", e.mensaje);
    }

    #[test]
    fn el_conector_declara_lo_que_sabe() {
        let c = Cassandra::CAPACIDADES;
        assert_eq!(c.conector, "ore-read-cassandra");
        assert_eq!(c.operadores.len(), 10);
        assert!(c.limit && c.servir && !c.order_by && !c.estimar);
    }
}
