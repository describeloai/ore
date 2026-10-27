//! **`leer` en Arrow**: MongoDB como [`Origen`] (`ore_driver::origen`).
//!
//! Lo único de este módulo es lo que es de MongoDB: de qué colección se lee, con
//! qué esquema, y cómo se vuelve flujo. Los verbos, `SIGTERM` y las sesiones de
//! `servir` son del esqueleto.
//!
//! # El objeto
//!
//! `base.coleccion`, partido en el **primer** punto: el nombre de una base no
//! puede llevarlo y el de una colección sí.
//!
//! # El tipo de cada columna
//!
//! La petición no trae tipos, y Mongo no tiene catálogo que preguntar. Se
//! **infiere de una muestra** al primer uso de la colección y se guarda en la
//! sesión (`servir` reutiliza la conexión, así que dos lecturas seguidas ven el
//! mismo esquema). Una columna que la muestra no vio se contesta `esquema`, no
//! se inventa. Un valor posterior que no cabe en el tipo inferido es `esquema`
//! también, con la columna y la fila.

use crate::esquema::{Esquema, Observador, PROFUNDIDAD_POR_DEFECTO, Tipo};
use crate::lote::{Constructor, ErrorLote};
use crate::traductor::{OPERADORES, Tipos, traducir};
use crate::transporte::{Cambio, Conexion, testigo_de, testigo_texto};
use arrow_array::builder::StringBuilder;
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use ore_arrow::Emisor;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cuántos documentos se miran para inferir el esquema.
pub const MUESTRA: u64 = 1000;
/// Lo que se espera a la muestra si la petición no trae tiempo.
const TIEMPO_MUESTRA: Duration = Duration::from_secs(30);
/// Filas por lote Arrow.
const FILAS_POR_LOTE: usize = 8192;

pub struct Mongo;

/// Lo que une a quien lee con quien cancela: la bandera y **qué operación
/// corre en el servidor** (el `comment` con que se marcó), para matarla allí.
#[derive(Clone, Default)]
pub struct Token {
    bandera: Arc<AtomicBool>,
    en_curso: Arc<Mutex<Option<(Conexion, String)>>>,
}

impl Token {
    pub fn cancelada(&self) -> bool {
        self.bandera.load(Ordering::Relaxed)
    }
}

/// Deja de marcar la operación en curso al salir de `leer`, haya salido como haya salido.
struct EnCurso<'a>(&'a Token);

impl Drop for EnCurso<'_> {
    fn drop(&mut self) {
        if let Ok(mut c) = self.0.en_curso.lock() {
            *c = None;
        }
    }
}

/// Una marca distinta para cada lectura del proceso.
fn marca() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "ore-read-mongodb:{}:{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

pub struct Sesion {
    conexion: Conexion,
    /// Por `base.coleccion`.
    esquemas: HashMap<String, Esquema>,
}

/// `(base, coleccion)` de un objeto `base.coleccion`.
pub fn partir_objeto(objeto: &str) -> Result<(&str, &str), Fallo> {
    match objeto.split_once('.') {
        Some((b, c)) if !b.is_empty() && !c.is_empty() => Ok((b, c)),
        _ => Err(Fallo::operador(format!(
            "el objeto `{objeto}` no es `base.coleccion`"
        ))),
    }
}

/// El esquema que deja inferir una muestra.
pub fn inferir(documentos: &[mongodb::bson::RawDocumentBuf]) -> Result<Esquema, Fallo> {
    let mut o = Observador::new(PROFUNDIDAD_POR_DEFECTO);
    for d in documentos {
        o.observar(d).map_err(|e| Fallo::new(Codigo::Esquema, e))?;
    }
    Ok(o.esquema())
}

/// **Una colección sin documentos**: el flujo de la proyección, en texto (sin un
/// valor no hay tipo que decir) y sin filas. Es lo que el contrato pide de una
/// tabla vacía (caso 6 del kit): decir sus columnas, no fallar.
fn vacia(p: &Peticion, salida: &mut dyn Write) -> Result<u64, Fallo> {
    let columnas: Vec<(String, String, Tipo)> = p
        .proyeccion
        .iter()
        .map(|(prop, ruta)| (prop.clone(), ruta.clone(), Tipo::Texto))
        .collect();
    let cons = Constructor::nuevo(&columnas).map_err(de_lote)?;
    let mut flujo = Emisor::nuevo(&mut *salida, &cons.schema())?;
    flujo.fin()?;
    eprintln!(
        "ore-read-mongodb: aviso · `{}` no tiene documentos: sus columnas salen como texto",
        p.objeto
    );
    Ok(0)
}

/// **El esquema inferido con lo que la tabla declara por encima** (`tipos` de la
/// petición). Lo declarado manda si el dato lo puede llevar ([`Tipo::declarado`]);
/// si no, `esquema`, nombrando la columna. Una declaración de una columna que la
/// muestra no vio se ignora: no es de esta lectura.
pub fn con_declarados(
    e: &Esquema,
    tipos: &[(String, String)],
    objeto: &str,
) -> Result<Esquema, Fallo> {
    let mut e = e.clone();
    for (columna, declarado) in tipos {
        if let Some(c) = e.columnas.iter_mut().find(|c| &c.ruta == columna) {
            c.tipo = c.tipo.declarado(declarado).map_err(|m| {
                Fallo::new(
                    Codigo::Esquema,
                    format!("`{objeto}`, columna `{columna}`: {m}"),
                )
            })?;
        }
    }
    Ok(e)
}

fn de_lote(e: ErrorLote) -> Fallo {
    match e {
        ErrorLote::Arrow(m) => Fallo::origen(m),
        otro => Fallo::new(Codigo::Esquema, otro.to_string()),
    }
}

impl Origen for Mongo {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-mongodb",
        version: env!("CARGO_PKG_VERSION"),
        operadores: OPERADORES,
        limit: true,
        order_by: true,
        // `estimar` es `explain`, y un `explain` honesto con filtros es de M2·2.
        estimar: false,
        servir: true,
    };

    fn conectar(url: &str) -> Result<Sesion, Fallo> {
        Ok(Sesion {
            conexion: Conexion::abrir(url)?,
            esquemas: HashMap::new(),
        })
    }

    fn leer(
        s: &mut Sesion,
        token: &Token,
        p: &Peticion,
        salida: &mut dyn Write,
    ) -> Result<u64, Fallo> {
        token.bandera.store(false, Ordering::SeqCst);
        let comment = marca();
        if let Ok(mut c) = token.en_curso.lock() {
            *c = Some((s.conexion.clone(), comment.clone()));
        }
        let _en_curso = EnCurso(token);
        let hecho = leer_marcada(s, token, p, salida, &comment);
        // Matar la operación hace que el servidor conteste «interrumpida»: aquí
        // eso es lo que se pidió, no un fallo del origen.
        match hecho {
            Err(_) if token.cancelada() => Err(Fallo::origen("cancelada en el origen")),
            otro => otro,
        }
    }

    fn estimar(_: &mut Sesion, _: &Peticion) -> Result<String, Fallo> {
        Err(Fallo::operador("este origen todavía no estima"))
    }

    fn cancelar(token: &Token) {
        token.bandera.store(true, Ordering::SeqCst);
        let en_curso = token.en_curso.lock().ok().and_then(|c| c.clone());
        if let Some((conexion, comment)) = en_curso {
            let _ = conexion.matar(&comment);
        }
    }
}

/// La columna que dice qué fue cada fila de una lectura por posición.
///
/// ⚠️ **Provisional.** El contrato (0016) define el rango por posición pero no
/// la forma de las filas de un changelog, y el motor todavía no consume uno
/// (0063: «el driver no sabe leer el changelog»). Esta es la forma más corta que
/// deja distinguir un alta de un borrado; cuando el motor fije la suya, se
/// cambia aquí y en nada más.
pub const COLUMNA_OPERACION: &str = "_ore_op";
/// Un documento tal como está ahora (alta, actualización o reemplazo).
pub const OP_UPSERT: &str = "upsert";
/// Un borrado: solo la clave, el resto nulo.
pub const OP_DELETE: &str = "delete";

/// Si la petición pide un rango por la posición del propio origen (sin columna).
fn por_posicion(p: &Peticion) -> bool {
    (p.start.is_some() || p.end.is_some()) && p.cursor.is_none()
}

/// **Los cambios de una colección entre dos posiciones**, en Arrow: las
/// columnas pedidas más [`COLUMNA_OPERACION`]. `start` es exclusivo, `end`
/// inclusivo y, si falta, el presente (que se toma **antes** de empezar, así el
/// testigo y las filas son el mismo instante).
///
/// Un changelog no se filtra, ordena ni recorta en el origen: un borrado no trae
/// el documento, así que no hay con qué filtrarlo. Se rechaza diciéndolo.
fn leer_posicion(
    conexion: &Conexion,
    token: &Token,
    p: &Peticion,
    esquema: &Esquema,
    salida: &mut dyn Write,
) -> Result<u64, Fallo> {
    if !p.filtros.is_empty() || !p.orden.is_empty() || p.limit.is_some() {
        return Err(Fallo::operador(
            "un rango por posición devuelve el changelog sin filtrar, ordenar ni recortar (un borrado no trae el documento): eso lo hace el motor",
        ));
    }
    let (base, coleccion) = partir_objeto(&p.objeto)?;
    if !conexion.admite_cambios()? {
        return Err(Fallo::operador(format!(
            "`{}`: este servidor no tiene change streams (no es un replica set), no hay changelog que leer",
            p.objeto
        )));
    }
    let Some(start) = p.start.as_deref() else {
        return Err(Fallo::operador(
            "un rango por posición necesita `start`: «desde el principio» no es una posición del oplog",
        ));
    };
    let desde = testigo_de(start)?;
    let hasta = match p.end.as_deref() {
        Some(e) => testigo_de(e)?,
        None => conexion
            .ahora()?
            .ok_or_else(|| Fallo::operador("este servidor no sabe fecharse"))?,
    };
    let columnas: Vec<(String, String, Tipo)> = p
        .proyeccion
        .iter()
        .map(|(prop, ruta)| {
            let tipo = esquema
                .columna(ruta)
                .expect("comprobada en leer_marcada")
                .tipo
                .clone();
            (prop.clone(), ruta.clone(), tipo)
        })
        .collect();
    let mut cons = Constructor::nuevo(&columnas).map_err(de_lote)?;
    let mut campos: Vec<Field> = cons
        .schema()
        .fields()
        .iter()
        .map(|f| f.as_ref().clone())
        .collect();
    campos.push(
        Field::new(COLUMNA_OPERACION, DataType::Utf8, false)
            .with_metadata([("ore:tipo".to_string(), "String".to_string())].into()),
    );
    let esquema_arrow = std::sync::Arc::new(Schema::new(campos));
    // Abrir primero: lo que falla al abrir (una posición caducada) falla antes del
    // primer byte y no con la cabecera ya escrita.
    let mut abiertos = conexion.abrir_cambios(base, coleccion, desde)?;
    let mut flujo = Emisor::nuevo(&mut *salida, &esquema_arrow)?;
    let mut ops: Vec<&'static str> = Vec::new();
    let vaciar = |cons: &mut Constructor,
                  ops: &mut Vec<&'static str>,
                  flujo: &mut Emisor<&mut dyn Write>|
     -> Result<(), Fallo> {
        let lote = cons.finish().map_err(de_lote)?;
        let mut b = StringBuilder::new();
        for o in ops.drain(..) {
            b.append_value(o);
        }
        let mut columnas: Vec<ArrayRef> = lote.columns().to_vec();
        columnas.push(std::sync::Arc::new(b.finish()));
        let lote = RecordBatch::try_new(esquema_arrow.clone(), columnas)
            .map_err(|e| Fallo::origen(e.to_string()))?;
        flujo.lote(&lote)
    };
    let fin = abiertos.leer(hasta, &token.bandera, &mut |c| {
        match c {
            Cambio::Alta(d) => {
                cons.push(&d).map_err(de_lote)?;
                ops.push(OP_UPSERT);
            }
            Cambio::Baja(d) => {
                cons.push(&d).map_err(de_lote)?;
                ops.push(OP_DELETE);
            }
        }
        if cons.filas() >= FILAS_POR_LOTE {
            vaciar(&mut cons, &mut ops, &mut flujo)?;
        }
        Ok(())
    })?;
    if fin.cancelada {
        return Err(Fallo::origen(format!(
            "cancelada con {} cambios leídos",
            fin.filas
        )));
    }
    if cons.filas() > 0 {
        vaciar(&mut cons, &mut ops, &mut flujo)?;
    }
    flujo.fin()?;
    eprintln!(
        "ore-read-mongodb: cambios de `{}` de {} (exclusiva) a {}: {} filas",
        p.objeto,
        testigo_texto(desde),
        testigo_texto(hasta),
        fin.filas
    );
    Ok(fin.filas)
}

fn leer_marcada(
    s: &mut Sesion,
    token: &Token,
    p: &Peticion,
    salida: &mut dyn Write,
    comment: &str,
) -> Result<u64, Fallo> {
    {
        let (base, coleccion) = partir_objeto(&p.objeto)?;
        if !s.esquemas.contains_key(&p.objeto) {
            let espera = p.timeout_ms.map_or(TIEMPO_MUESTRA, Duration::from_millis);
            let muestra = s
                .conexion
                .muestra(base, coleccion, MUESTRA, espera, Some(comment))?;
            let e = inferir(&muestra.documentos)
                .map_err(|f| Fallo::new(f.codigo, format!("{}: {}", p.objeto, f.mensaje)))?;
            if e.documentos == 0 {
                if por_posicion(p) {
                    return Err(Fallo::new(
                        Codigo::Esquema,
                        format!(
                            "`{}` no tiene documentos: sin ellos no hay tipos con que leer sus cambios, y hay que copiarla entera",
                            p.objeto
                        ),
                    ));
                }
                // Sin documentos no hay tipo que inferir, y mañana puede haberlos:
                // no se guarda. Se contesta el esquema de la proyección, vacío.
                return vacia(p, salida);
            }
            s.esquemas.insert(p.objeto.clone(), e);
        }
        let esquema = &con_declarados(&s.esquemas[&p.objeto], &p.tipos, &p.objeto)?;
        // Una columna que la muestra no vio es del esquema, no del operador.
        for (_, ruta) in &p.proyeccion {
            if esquema.columna(ruta).is_none() {
                return Err(Fallo::new(
                    Codigo::Esquema,
                    format!(
                        "la columna `{ruta}` no está en lo que se vio de `{}` ({} documentos de muestra)",
                        p.objeto, esquema.documentos
                    ),
                ));
            }
        }
        if por_posicion(p) {
            return leer_posicion(&s.conexion, token, p, esquema, salida);
        }
        let t = traducir(p, &Tipos::de_esquema(esquema))?;
        let columnas: Vec<(String, String, Tipo)> = t
            .rutas
            .iter()
            .map(|(prop, ruta)| {
                let tipo = esquema
                    .columna(ruta)
                    .expect("comprobada arriba")
                    .tipo
                    .clone();
                (prop.clone(), ruta.clone(), tipo)
            })
            .collect();
        let mut cons = Constructor::nuevo(&columnas).map_err(de_lote)?;
        let mut flujo = Emisor::nuevo(&mut *salida, &cons.schema())?;
        let escribir_lote = |cons: &mut Constructor, flujo: &mut Emisor<&mut dyn Write>| {
            let lote = cons.finish().map_err(de_lote)?;
            flujo.lote(&lote)
        };
        let fin = s.conexion.ejecutar(
            base,
            coleccion,
            &t.plan,
            &token.bandera,
            Some(comment),
            &mut |d| {
                cons.push(d).map_err(de_lote)?;
                if cons.filas() >= FILAS_POR_LOTE {
                    escribir_lote(&mut cons, &mut flujo)?;
                }
                Ok(())
            },
        )?;
        if fin.cancelada {
            return Err(Fallo::origen(format!(
                "cancelada con {} filas leídas",
                fin.filas
            )));
        }
        if cons.filas() > 0 {
            escribir_lote(&mut cons, &mut flujo)?;
        }
        flujo.fin()?;
        for a in &esquema.avisos {
            eprintln!("ore-read-mongodb: aviso · `{}`: {}", a.ruta, a.mensaje);
        }
        Ok(fin.filas)
    }
}
