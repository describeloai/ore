//! La conexión y la lectura. Aquí viven los **cuatro invariantes** que X0 midió (0071 §3.3):
//!
//! 1. se pide siempre el tamaño del primer frame (sin él llega vacío);
//! 2. se leen `missing_statement` y `missing_results` (sin ellos, un statement perdido es un truncado
//!    silencioso);
//! 3. no se confía en el `offset` del `Fetch` (el servidor lo ignora: es un cursor hacia delante), así que
//!    un `Fetch` perdido **no se reintenta**: la lectura se reinicia entera;
//! 4. un frame sin filas y sin `done` es un error (`Truncado`), no el final.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::Error;
use crate::proto::{
    CloseConnectionRequest, CloseConnectionResponse, CloseStatementRequest, CloseStatementResponse,
    ColumnsRequest, CreateStatementRequest, CreateStatementResponse, ExecuteResponse, FetchRequest,
    FetchResponse, Frame, OpenConnectionRequest, OpenConnectionResponse, PrepareAndExecuteRequest,
    ResultSetResponse, SchemasRequest, TablesRequest,
};
use crate::transporte::{Opciones, Transporte};
use crate::valor::{Columna, Fila, valor_de};

/// Un resultado entero en memoria: el de los metadatos (esquemas, tablas, columnas).
#[derive(Debug, Clone, PartialEq)]
pub struct Tabla {
    pub columnas: Vec<Columna>,
    pub filas: Vec<Fila>,
}

impl Tabla {
    /// La posición de una columna por su nombre (sin mirar mayúsculas), o `None`.
    pub fn indice(&self, nombre: &str) -> Option<usize> {
        self.columnas
            .iter()
            .position(|c| c.nombre.eq_ignore_ascii_case(nombre))
    }
}

/// La conexión abierta. Se cierra sola al soltarla (mejor esfuerzo): el PQS guarda una conexión por
/// cliente hasta que caduca.
pub struct Conexion {
    sesion: Arc<Sesion>,
}

/// Lo que comparten la conexión y sus lecturas, para que cerrar la conexión no ocurra con una lectura viva.
struct Sesion {
    t: Transporte,
    id: String,
    filas_por_frame: i32,
}

impl Drop for Sesion {
    fn drop(&mut self) {
        let _ = self.t.llamar::<_, CloseConnectionResponse>(
            "CloseConnectionRequest",
            &CloseConnectionRequest {
                connection_id: self.id.clone(),
            },
            "CloseConnectionResponse",
        );
    }
}

static CONTADOR: AtomicU64 = AtomicU64::new(0);

/// El identificador de conexión lo inventa el cliente (medido: cualquier cadena vale). Único por proceso y
/// por arranque, sin necesitar un generador de UUID.
fn identificador() -> String {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "ore-{:x}-{:x}-{:x}",
        std::process::id(),
        ns,
        CONTADOR.fetch_add(1, Ordering::Relaxed)
    )
}

impl Conexion {
    /// Abre la conexión con las propiedades de `Opciones` (`TenantId`, `CurrentSCN`,
    /// `phoenix.query.timeoutMs`…). No ejecuta nada.
    pub fn abrir(o: &Opciones) -> Result<Conexion, Error> {
        if o.filas_por_frame <= 0 {
            return Err(Error::protocolo(
                "filas_por_frame tiene que ser positivo: sin él el primer frame llega vacío",
            ));
        }
        let t = Transporte::nuevo(o)?;
        let id = identificador();
        let _: OpenConnectionResponse = t.llamar(
            "OpenConnectionRequest",
            &OpenConnectionRequest {
                connection_id: id.clone(),
                info: o.propiedades.clone(),
            },
            "OpenConnectionResponse",
        )?;
        Ok(Conexion {
            sesion: Arc::new(Sesion {
                t,
                id,
                filas_por_frame: o.filas_por_frame,
            }),
        })
    }

    /// Ejecuta una consulta y devuelve su lectura, **sin traer más que el primer frame**.
    pub fn ejecutar(&self, sql: &str) -> Result<Lectura, Error> {
        let s = &self.sesion;
        let c: CreateStatementResponse = s.t.llamar(
            "CreateStatementRequest",
            &CreateStatementRequest {
                connection_id: s.id.clone(),
            },
            "CreateStatementResponse",
        )?;
        let r: Result<ExecuteResponse, Error> = s.t.llamar(
            "PrepareAndExecuteRequest",
            &PrepareAndExecuteRequest {
                connection_id: s.id.clone(),
                sql: sql.to_string(),
                statement_id: c.statement_id,
                max_row_count: 0,
                max_rows_total: -1,
                // Invariante 1: sin esto el primer frame llega con 0 filas.
                first_frame_max_size: s.filas_por_frame,
            },
            "ExecuteResponse",
        );
        let e = match r {
            Ok(e) => e,
            Err(err) => {
                cerrar_statement(s, c.statement_id);
                return Err(err);
            }
        };
        if e.missing_statement {
            return Err(Error::StatementPerdido);
        }
        if e.results.len() != 1 {
            cerrar_statement(s, c.statement_id);
            return Err(Error::protocolo(format!(
                "se esperaba un resultado y llegaron {}",
                e.results.len()
            )));
        }
        let rs = e.results.into_iter().next().expect("uno");
        Lectura::de(self.sesion.clone(), c.statement_id, rs)
    }

    /// Los esquemas (`SchemasRequest`). `patron` es un patrón JDBC (`%`, `_`); `None` = todos.
    pub fn esquemas(&self, patron: Option<&str>) -> Result<Tabla, Error> {
        let s = &self.sesion;
        let rs: ResultSetResponse = s.t.llamar(
            "SchemasRequest",
            &SchemasRequest {
                connection_id: s.id.clone(),
                schema_pattern: patron.unwrap_or_default().to_string(),
                has_schema_pattern: patron.is_some(),
                ..Default::default()
            },
            "ResultSetResponse",
        )?;
        self.todo(rs)
    }

    /// Las tablas y vistas (`TablesRequest`); `tipos` vacío = todos los tipos.
    pub fn tablas(
        &self,
        esquema: Option<&str>,
        tabla: Option<&str>,
        tipos: &[&str],
    ) -> Result<Tabla, Error> {
        let s = &self.sesion;
        let rs: ResultSetResponse = s.t.llamar(
            "TablesRequest",
            &TablesRequest {
                connection_id: s.id.clone(),
                schema_pattern: esquema.unwrap_or_default().to_string(),
                has_schema_pattern: esquema.is_some(),
                table_name_pattern: tabla.unwrap_or_default().to_string(),
                has_table_name_pattern: tabla.is_some(),
                type_list: tipos.iter().map(|t| t.to_string()).collect(),
                has_type_list: !tipos.is_empty(),
                ..Default::default()
            },
            "ResultSetResponse",
        )?;
        self.todo(rs)
    }

    /// Las columnas (`ColumnsRequest`).
    pub fn columnas(&self, esquema: Option<&str>, tabla: Option<&str>) -> Result<Tabla, Error> {
        let s = &self.sesion;
        let rs: ResultSetResponse = s.t.llamar(
            "ColumnsRequest",
            &ColumnsRequest {
                connection_id: s.id.clone(),
                schema_pattern: esquema.unwrap_or_default().to_string(),
                has_schema_pattern: esquema.is_some(),
                table_name_pattern: tabla.unwrap_or_default().to_string(),
                has_table_name_pattern: tabla.is_some(),
                ..Default::default()
            },
            "ResultSetResponse",
        )?;
        self.todo(rs)
    }

    /// Lee un resultado de metadatos hasta el final.
    fn todo(&self, rs: ResultSetResponse) -> Result<Tabla, Error> {
        let mut l = Lectura::de(self.sesion.clone(), rs.statement_id, rs)?;
        let columnas = l.columnas().to_vec();
        let mut filas = Vec::new();
        while let Some(f) = l.siguiente_frame()? {
            filas.extend(f);
        }
        Ok(Tabla { columnas, filas })
    }
}

/// Los frames de una consulta, uno a uno. Se puede soltar a medias: cierra el statement.
pub struct Lectura {
    sesion: Arc<Sesion>,
    statement_id: u32,
    columnas: Vec<Columna>,
    /// El primer frame, que ya vino con el `ExecuteResponse`.
    primero: Option<Frame>,
    leidas: u64,
    hecha: bool,
}

impl Lectura {
    fn de(sesion: Arc<Sesion>, statement_id: u32, rs: ResultSetResponse) -> Result<Lectura, Error> {
        let Some(firma) = rs.signature else {
            cerrar_statement(&sesion, statement_id);
            return Err(Error::protocolo(
                "el resultado no trae firma: no es una consulta",
            ));
        };
        let Some(frame) = rs.first_frame else {
            cerrar_statement(&sesion, statement_id);
            return Err(Error::protocolo("el resultado no trae el primer frame"));
        };
        Ok(Lectura {
            sesion,
            statement_id,
            columnas: firma.columns.iter().map(Columna::de).collect(),
            primero: Some(frame),
            leidas: 0,
            hecha: false,
        })
    }

    pub fn columnas(&self) -> &[Columna] {
        &self.columnas
    }

    /// Las filas leídas hasta ahora.
    pub fn leidas(&self) -> u64 {
        self.leidas
    }

    /// El siguiente frame de filas, o `None` si ya no hay más.
    pub fn siguiente_frame(&mut self) -> Result<Option<Vec<Fila>>, Error> {
        loop {
            if self.hecha {
                return Ok(None);
            }
            let frame = match self.primero.take() {
                Some(f) => f,
                None => self.traer()?,
            };
            // Invariante 4: ni filas ni `done` es que el cursor no avanza, no que se acabó.
            if frame.rows.is_empty() && !frame.done {
                self.hecha = true;
                cerrar_statement(&self.sesion, self.statement_id);
                return Err(Error::Truncado {
                    leidas: self.leidas,
                });
            }
            if frame.done {
                self.hecha = true;
                cerrar_statement(&self.sesion, self.statement_id);
            }
            if frame.rows.is_empty() {
                continue; // `done` sin filas: el final limpio
            }
            self.leidas += frame.rows.len() as u64;
            return Ok(Some(
                frame
                    .rows
                    .iter()
                    .map(|r| r.value.iter().map(valor_de).collect())
                    .collect(),
            ));
        }
    }

    /// Un `Fetch`. El `offset` que se manda es el de las filas leídas, aunque el servidor lo ignore
    /// (invariante 3): así, si algún día lo honra, no cambia nada.
    fn traer(&mut self) -> Result<Frame, Error> {
        let s = &self.sesion;
        let r: FetchResponse = s.t.llamar(
            "FetchRequest",
            &FetchRequest {
                connection_id: s.id.clone(),
                statement_id: self.statement_id,
                offset: self.leidas,
                fetch_max_row_count: 0,
                frame_max_size: s.filas_por_frame,
            },
            "FetchResponse",
        )?;
        // Invariante 2.
        if r.missing_statement || r.missing_results {
            self.hecha = true;
            return Err(Error::StatementPerdido);
        }
        r.frame
            .ok_or_else(|| Error::protocolo("el Fetch no trae frame"))
    }
}

impl Drop for Lectura {
    fn drop(&mut self) {
        if !self.hecha {
            cerrar_statement(&self.sesion, self.statement_id);
        }
    }
}

/// Mejor esfuerzo: un statement que no se pudo cerrar caduca solo en el servidor.
fn cerrar_statement(s: &Sesion, statement_id: u32) {
    let _ = s.t.llamar::<_, CloseStatementResponse>(
        "CloseStatementRequest",
        &CloseStatementRequest {
            connection_id: s.id.clone(),
            statement_id,
        },
        "CloseStatementResponse",
    );
}
