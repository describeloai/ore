//! **`Scan` y `Get`** sobre una region. Un `Escaner` es un iterador de filas: abre el scanner en el servidor con
//! la primera llamada, sigue con `next_call_seq`, y lo cierra al acabar o al soltarse.
//!
//! Lo que X0 midio contra un RegionServer de verdad y la documentacion no dice (ficha 0072, §3.2):
//! - `next_call_seq` empieza en 0 en la primera continuacion y sube de uno en uno.
//! - Cuando la region se acaba, el servidor **cierra el scanner solo** y lo dice con `more_results_in_region =
//!   false`: pedir otra vez, o cerrarlo, da `OutOfOrderScannerNextException`.
//! - `include_start_row` se pone a mano (el valor por defecto del `.proto` no se fia).

use crate::error::{Error, Resultado};
use crate::proto::{
    Column, Filter, Get, GetRequest, GetResponse, RegionSpecifier, RegionSpecifierType,
    ResultadoPb, Scan, ScanRequest, ScanResponse, TimeRange,
};
use crate::rpc::Conexion;
use std::collections::VecDeque;

/// Una celda: la ultima version, salvo que se pida otra cosa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Celda {
    pub familia: Vec<u8>,
    pub cualificador: Vec<u8>,
    /// La marca de tiempo de la celda, en milisegundos desde la epoca (la pone el servidor o el cliente).
    pub marca: u64,
    pub valor: Vec<u8>,
}

/// Una fila de HBase: su clave y sus celdas, en el orden en que llegan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fila {
    pub clave: Vec<u8>,
    pub celdas: Vec<Celda>,
}

impl Fila {
    /// La fila de un `Result` del protocolo; `None` si no trae celdas (no existe, o el filtro la vacio).
    pub fn de(r: ResultadoPb) -> Option<Fila> {
        let clave = r.cell.first()?.row.clone().unwrap_or_default();
        let celdas = r
            .cell
            .into_iter()
            .map(|c| Celda {
                familia: c.family.unwrap_or_default(),
                cualificador: c.qualifier.unwrap_or_default(),
                marca: c.timestamp.unwrap_or(0),
                valor: c.value.unwrap_or_default(),
            })
            .collect();
        Some(Fila { clave, celdas })
    }

    /// El valor de `familia:cualificador`, si esta.
    pub fn valor(&self, familia: &[u8], cualificador: &[u8]) -> Option<&[u8]> {
        self.celdas
            .iter()
            .find(|c| c.familia == familia && c.cualificador == cualificador)
            .map(|c| c.valor.as_slice())
    }
}

/// Que leer de una region.
#[derive(Debug, Clone)]
pub struct Consulta {
    /// Desde esta clave, **inclusive**. Vacio: desde el principio de la region.
    pub inicio: Vec<u8>,
    /// Hasta esta clave, **exclusiva**. Vacio: hasta el final de la region.
    pub fin: Vec<u8>,
    /// Las columnas: `(familia, cualificadores)`. Un cualificador vacio en la lista es la familia entera; sin
    /// columnas, todas.
    pub columnas: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    /// `[desde, hasta)` en marcas de tiempo de celda.
    pub tiempo: Option<(u64, u64)>,
    /// Cuantas filas pide cada llamada.
    pub filas_por_llamada: u32,
    /// Cuantas versiones de cada celda.
    pub versiones: u32,
    /// Un filtro del servidor, tal como lo espera el protocolo: la clase Java (p. ej.
    /// `org.apache.hadoop.hbase.filter.PrefixFilter`) y su mensaje protobuf ya serializado.
    pub filtro: Option<(String, Vec<u8>)>,
}

impl Default for Consulta {
    fn default() -> Self {
        Consulta {
            inicio: Vec::new(),
            fin: Vec::new(),
            columnas: Vec::new(),
            tiempo: None,
            filas_por_llamada: 1000,
            versiones: 1,
            filtro: None,
        }
    }
}

fn columnas_del_protocolo(cols: &[(Vec<u8>, Vec<Vec<u8>>)]) -> Vec<Column> {
    cols.iter()
        .map(|(f, qs)| Column {
            family: f.clone(),
            qualifier: qs.clone(),
        })
        .collect()
}

fn region(nombre: &[u8]) -> RegionSpecifier {
    RegionSpecifier {
        r#type: RegionSpecifierType::RegionName as i32,
        value: nombre.to_vec(),
    }
}

fn tiempo_del_protocolo(t: Option<(u64, u64)>) -> Option<TimeRange> {
    t.map(|(d, h)| TimeRange {
        from: Some(d),
        to: Some(h),
    })
}

/// La peticion que **abre** un scanner.
pub fn abrir_scanner(nombre_de_region: &[u8], c: &Consulta) -> ScanRequest {
    ScanRequest {
        region: Some(region(nombre_de_region)),
        scan: Some(Scan {
            column: columnas_del_protocolo(&c.columnas),
            start_row: Some(c.inicio.clone()),
            stop_row: if c.fin.is_empty() {
                None
            } else {
                Some(c.fin.clone())
            },
            time_range: tiempo_del_protocolo(c.tiempo),
            filter: c.filtro.as_ref().map(|(nombre, datos)| Filter {
                name: nombre.clone(),
                serialized_filter: Some(datos.clone()),
            }),
            max_versions: Some(c.versiones),
            caching: Some(c.filas_por_llamada),
            include_start_row: Some(true),
            ..Scan::default()
        }),
        number_of_rows: Some(c.filas_por_llamada),
        // El servidor entrega la fila entera aunque sea grande; los latidos si, para que un filtro que descarta
        // casi todo no agote el tiempo de la llamada.
        client_handles_partials: Some(false),
        client_handles_heartbeats: Some(true),
        ..ScanRequest::default()
    }
}

/// La peticion que **sigue** un scanner abierto.
pub fn seguir_scanner(id: u64, seq: u64, filas: u32) -> ScanRequest {
    ScanRequest {
        scanner_id: Some(id),
        number_of_rows: Some(filas),
        next_call_seq: Some(seq),
        client_handles_partials: Some(false),
        client_handles_heartbeats: Some(true),
        ..ScanRequest::default()
    }
}

/// La peticion que **cierra** un scanner abierto.
pub fn cerrar_scanner(id: u64, seq: u64) -> ScanRequest {
    ScanRequest {
        scanner_id: Some(id),
        number_of_rows: Some(0),
        next_call_seq: Some(seq),
        close_scanner: Some(true),
        ..ScanRequest::default()
    }
}

/// Un iterador de las filas de una region. Cierra el scanner del servidor al soltarse.
pub struct Escaner<'a> {
    con: &'a mut Conexion,
    nombre: Vec<u8>,
    consulta: Consulta,
    id: Option<u64>,
    seq: u64,
    pendientes: VecDeque<Fila>,
    /// El servidor ya no tiene scanner (la region se acabo, o fallo): no hay que cerrarlo.
    cerrado: bool,
    acabado: bool,
}

impl Conexion {
    /// Las filas de la region `nombre` que la consulta pide, en orden de clave.
    pub fn escanear<'a>(&'a mut self, nombre: &[u8], consulta: &Consulta) -> Escaner<'a> {
        Escaner {
            con: self,
            nombre: nombre.to_vec(),
            consulta: consulta.clone(),
            id: None,
            seq: 0,
            pendientes: VecDeque::new(),
            cerrado: false,
            acabado: false,
        }
    }

    /// Una fila por su clave; `None` si no existe.
    pub fn leer_fila(
        &mut self,
        nombre_de_region: &[u8],
        clave: &[u8],
        columnas: &[(Vec<u8>, Vec<Vec<u8>>)],
    ) -> Resultado<Option<Fila>> {
        let peticion = GetRequest {
            region: region(nombre_de_region),
            get: Get {
                row: clave.to_vec(),
                column: columnas_del_protocolo(columnas),
                max_versions: Some(1),
                ..Get::default()
            },
        };
        let r: GetResponse = self.llamar("Get", &peticion)?;
        Ok(r.result.and_then(Fila::de))
    }
}

impl Escaner<'_> {
    fn traer(&mut self) -> Resultado<()> {
        let peticion = match self.id {
            None => abrir_scanner(&self.nombre, &self.consulta),
            Some(id) => {
                let p = seguir_scanner(id, self.seq, self.consulta.filas_por_llamada);
                self.seq += 1;
                p
            }
        };
        let r: ScanResponse = match self.con.llamar("Scan", &peticion) {
            Ok(r) => r,
            Err(e) => {
                // Un fallo deja el scanner en un estado que no se sabe: no se cierra, se descarta.
                self.cerrado = true;
                self.acabado = true;
                return Err(e);
            }
        };
        if self.id.is_none() {
            self.id =
                Some(r.scanner_id.ok_or_else(|| {
                    Error::Protocolo("el scanner no devolvio identificador".into())
                })?);
        }
        for res in r.results {
            if let Some(f) = Fila::de(res) {
                self.pendientes.push_back(f);
            }
        }
        // El servidor cierra solo el scanner al acabar la region (§3.2).
        if r.more_results_in_region == Some(false) {
            self.cerrado = true;
            self.acabado = true;
        } else if r.more_results == Some(false) {
            self.acabado = true;
        }
        Ok(())
    }
}

impl Iterator for Escaner<'_> {
    type Item = Resultado<Fila>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(f) = self.pendientes.pop_front() {
                return Some(Ok(f));
            }
            if self.acabado {
                return None;
            }
            if let Err(e) = self.traer() {
                return Some(Err(e));
            }
        }
    }
}

impl Drop for Escaner<'_> {
    fn drop(&mut self) {
        // Soltarlo a medias deja un scanner vivo en el servidor hasta que caduque su arriendo: se cierra.
        if let (Some(id), false) = (self.id, self.cerrado) {
            let _: Resultado<ScanResponse> = self.con.llamar("Scan", &cerrar_scanner(id, self.seq));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abrir_pide_el_inicio_inclusivo_y_deja_al_servidor_la_fila_entera() {
        let c = Consulta {
            inicio: b"a".to_vec(),
            fin: b"z".to_vec(),
            tiempo: Some((10, 20)),
            ..Consulta::default()
        };
        let p = abrir_scanner(b"t,,1.abc.", &c);
        let s = p.scan.unwrap();
        assert_eq!(s.start_row.as_deref(), Some(&b"a"[..]));
        assert_eq!(s.stop_row.as_deref(), Some(&b"z"[..]));
        assert_eq!(s.include_start_row, Some(true));
        assert_eq!(s.max_versions, Some(1));
        let t = s.time_range.unwrap();
        assert_eq!((t.from, t.to), (Some(10), Some(20)));
        assert_eq!(p.client_handles_partials, Some(false));
        assert_eq!(p.client_handles_heartbeats, Some(true));
        assert_eq!(p.scanner_id, None);
    }

    #[test]
    fn sin_fin_no_se_manda_stop_row() {
        let p = abrir_scanner(b"r", &Consulta::default());
        assert_eq!(p.scan.unwrap().stop_row, None);
    }

    #[test]
    fn seguir_y_cerrar_llevan_el_scanner_y_la_secuencia() {
        let p = seguir_scanner(9, 4, 500);
        assert_eq!(
            (p.scanner_id, p.next_call_seq, p.number_of_rows),
            (Some(9), Some(4), Some(500))
        );
        assert!(p.region.is_none() && p.scan.is_none());
        let c = cerrar_scanner(9, 5);
        assert_eq!(
            (c.close_scanner, c.next_call_seq, c.number_of_rows),
            (Some(true), Some(5), Some(0))
        );
    }

    #[test]
    fn un_result_sin_celdas_no_es_una_fila() {
        assert!(Fila::de(ResultadoPb::default()).is_none());
    }
}
