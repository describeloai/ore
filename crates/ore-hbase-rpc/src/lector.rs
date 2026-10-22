//! **El lector**: las filas de una tabla en un rango de claves, de una region a la siguiente, aguantando lo que un
//! HBase de verdad hace durante una lectura larga: una region que se mueve o se parte, un RegionServer que se
//! reinicia, un scanner cuyo arriendo caduca, un servidor saturado.
//!
//! El truco que lo hace sencillo y exacto es que **las filas llegan ordenadas por clave**: ante cualquier fallo
//! se descarta el scanner, se espera lo que toque, se vuelve a preguntar a `hbase:meta` si hace falta y se abre
//! otro **desde la ultima clave entregada, exclusiva**. No hay que recordar nada mas, y no se entrega nada dos
//! veces ni se pierde nada (una fila es atomica: el servidor la entrega entera o no la entrega). Un lector de
//! SQL, como el de Phoenix, solo puede reiniciar.
//!
//! Que se hace con cada fallo lo dice [`Error::clase`], con los nombres de excepcion grabados del laboratorio
//! (`pruebas-de-fuego/hbase/trazas/excepciones.json`).

use crate::cancelacion::Cancelacion;
use crate::error::{Clase, Error, Resultado};
use crate::meta::{Cliente, Region, nombre_en_meta};
use crate::proto::{ScanResponse, TableStateState};
use crate::rpc::{Conexion, Servidor};
use crate::scan::{Consulta, Fila, abrir_scanner, cerrar_scanner, seguir_scanner};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

/// Un scanner abierto en un RegionServer.
struct Abierto {
    servidor: Servidor,
    id: u64,
    /// El `next_call_seq` de la siguiente continuacion.
    seq: u64,
    /// El fin (exclusivo) de la region que recorre; vacio en la ultima.
    fin_de_region: Vec<u8>,
}

/// Las filas de una tabla entre dos claves. Es un iterador: cada elemento es una fila o el fallo que paro la
/// lectura (despues de un fallo no entrega mas).
pub struct Lector<'a> {
    cli: &'a Cliente,
    tabla: String,
    consulta: Consulta,
    cancel: Cancelacion,
    /// La siguiente clave que se lee, inclusive.
    desde: Vec<u8>,
    /// El final que pidio quien lee, exclusivo; vacio: hasta el final de la tabla.
    hasta: Vec<u8>,
    ultima: Option<Vec<u8>>,
    /// El fin de la region cuyo scanner acabo: cuando se entregue lo que queda, se sigue por aqui.
    region_hecha: Option<Vec<u8>>,
    scanner: Option<Abierto>,
    conexiones: HashMap<Servidor, Conexion>,
    pendientes: VecDeque<Fila>,
    /// Fallos seguidos sin entregar una fila.
    intentos: u32,
    acabado: bool,
    cancelacion_avisada: bool,
    llamadas: u64,
    latidos: u64,
}

impl Cliente {
    /// Las filas de `tabla` en `[consulta.inicio, consulta.fin)`, en orden de clave y de una region a la siguiente.
    /// `cancelacion` corta la lectura incluso si esta esperando al servidor.
    pub fn leer<'a>(
        &'a self,
        tabla: &str,
        consulta: &Consulta,
        cancelacion: &Cancelacion,
    ) -> Resultado<Lector<'a>> {
        Ok(Lector {
            cli: self,
            tabla: nombre_en_meta(tabla)?,
            consulta: consulta.clone(),
            cancel: cancelacion.clone(),
            desde: consulta.inicio.clone(),
            hasta: consulta.fin.clone(),
            ultima: None,
            region_hecha: None,
            scanner: None,
            conexiones: HashMap::new(),
            pendientes: VecDeque::new(),
            intentos: 0,
            acabado: false,
            cancelacion_avisada: false,
            llamadas: 0,
            latidos: 0,
        })
    }
}

/// La clave inmediatamente siguiente en orden de bytes.
fn siguiente(clave: &[u8]) -> Vec<u8> {
    let mut s = clave.to_vec();
    s.push(0);
    s
}

/// El menor de dos finales exclusivos, donde vacio es «sin limite».
fn menor_fin(a: &[u8], b: &[u8]) -> Vec<u8> {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_vec(),
        (_, true) => a.to_vec(),
        _ => a.min(b).to_vec(),
    }
}

impl Lector<'_> {
    /// Cuantas llamadas `Scan` ha hecho (las que fallaron y se repitieron cuentan).
    pub fn llamadas(&self) -> u64 {
        self.llamadas
    }

    /// Cuantas respuestas han sido un latido: el servidor sigue buscando y aun no tiene una fila que dar.
    pub fn latidos(&self) -> u64 {
        self.latidos
    }

    fn region_para(&self, clave: &[u8]) -> Resultado<Region> {
        // Dos vueltas: la de la cache y, si la cache no cubre la clave, la de `hbase:meta` recien preguntada.
        for _ in 0..2 {
            let regiones = self.cli.regiones_en_cache(&self.tabla)?;
            if let Some(r) = regiones.iter().find(|r| {
                r.inicio.as_slice() <= clave && (r.fin.is_empty() || clave < r.fin.as_slice())
            }) {
                return Ok(r.clone());
            }
            self.cli.invalidar(&self.tabla);
        }
        Err(Error::RegionSinServidor(format!(
            "{}: ninguna region cubre la clave",
            self.tabla
        )))
    }

    /// Una llamada `Scan`, que para en cuanto se cancela la lectura.
    fn llamar(
        &mut self,
        servidor: &Servidor,
        peticion: &crate::proto::ScanRequest,
    ) -> Resultado<ScanResponse> {
        if !self.conexiones.contains_key(servidor) {
            let c = self.cli.conectar(servidor)?;
            self.conexiones.insert(servidor.clone(), c);
        }
        let con = self.conexiones.get_mut(servidor).expect("recien insertada");
        self.llamadas += 1;
        con.llamar_cancelable("Scan", peticion, Some(&self.cancel))
    }

    fn recibir(&mut self, r: ScanResponse) {
        if r.heartbeat_message == Some(true) {
            self.latidos += 1;
        }
        for res in r.results {
            if let Some(f) = Fila::de(res) {
                self.pendientes.push_back(f);
            }
        }
        // El servidor cierra solo el scanner cuando la region se acaba (§3.2 de la ficha): no hay que cerrarlo.
        if (r.more_results_in_region == Some(false) || r.more_results == Some(false))
            && let Some(s) = self.scanner.take()
        {
            self.region_hecha = Some(s.fin_de_region);
        }
    }

    /// Una llamada al servidor: abre el scanner de la region de `desde` o sigue el abierto.
    fn paso(&mut self) -> Resultado<()> {
        let Some((servidor, id, seq)) = self.scanner.as_mut().map(|s| {
            let seq = s.seq;
            s.seq += 1;
            (s.servidor.clone(), s.id, seq)
        }) else {
            if !self.hasta.is_empty() && self.desde >= self.hasta {
                self.acabado = true;
                return Ok(());
            }
            let region = self.region_para(&self.desde.clone())?;
            let consulta = Consulta {
                inicio: self.desde.clone(),
                fin: menor_fin(&self.hasta, &region.fin),
                ..self.consulta.clone()
            };
            let r = self.llamar(&region.servidor, &abrir_scanner(&region.nombre, &consulta))?;
            let id = r
                .scanner_id
                .ok_or_else(|| Error::Protocolo("el scanner no devolvio identificador".into()))?;
            self.scanner = Some(Abierto {
                servidor: region.servidor,
                id,
                seq: 0,
                fin_de_region: region.fin,
            });
            self.recibir(r);
            return Ok(());
        };
        let r = self.llamar(
            &servidor,
            &seguir_scanner(id, seq, self.consulta.filas_por_llamada),
        )?;
        self.recibir(r);
        Ok(())
    }

    fn dormir(&self, total: Duration) {
        let mut falta = total;
        while !falta.is_zero() && !self.cancel.cancelada() {
            let t = falta.min(Duration::from_millis(25));
            std::thread::sleep(t);
            falta -= t;
        }
    }

    /// Decide si el fallo se supera esperando y, si si, deja el lector listo para reabrir desde `desde`.
    fn recuperar(&mut self, e: Error) -> Resultado<()> {
        if self.cancel.cancelada() {
            return Err(Error::Cancelada);
        }
        let clase = e.clase();
        if clase == Clase::Fatal {
            return Err(e);
        }
        self.intentos += 1;
        let politica = self.cli.opciones().politica;
        if self.intentos > politica.max_intentos {
            return Err(Error::Agotado {
                intentos: self.intentos - 1,
                ultimo: Box::new(e),
            });
        }
        // El scanner (si lo habia) ya no vale: lo que dejo abierto en el servidor caduca solo.
        self.scanner = None;
        match clase {
            Clase::Region => {
                self.cli.invalidar(&self.tabla);
                // Una region que no aparece dos veces seguidas puede ser una tabla deshabilitada, que no se
                // arregla esperando.
                if self.intentos >= 2
                    && matches!(
                        self.cli.estado_de_tabla(&self.tabla),
                        Ok(Some(TableStateState::Disabled | TableStateState::Disabling))
                    )
                {
                    return Err(Error::TablaDeshabilitada(self.tabla.clone()));
                }
            }
            Clase::Red => {
                self.conexiones.clear();
                self.cli.invalidar(&self.tabla);
            }
            Clase::Scanner | Clase::Ocupado | Clase::Fatal => {}
        }
        // Un scanner perdido se reabre al momento: el servidor esta bien, solo olvido el scanner.
        if clase != Clase::Scanner {
            self.dormir(politica.espera(self.intentos));
        }
        Ok(())
    }
}

impl Iterator for Lector<'_> {
    type Item = Resultado<Fila>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.cancel.cancelada() {
                self.acabado = true;
                self.pendientes.clear();
                return if std::mem::replace(&mut self.cancelacion_avisada, true) {
                    None
                } else {
                    Some(Err(Error::Cancelada))
                };
            }
            if let Some(f) = self.pendientes.pop_front() {
                // Defensa: tras reabrir, una fila que no pasa de la ultima entregada es un duplicado.
                if self.ultima.as_ref().is_some_and(|u| f.clave <= *u) {
                    continue;
                }
                self.desde = siguiente(&f.clave);
                self.ultima = Some(f.clave.clone());
                self.intentos = 0;
                return Some(Ok(f));
            }
            if self.acabado {
                return None;
            }
            if let Some(fin) = self.region_hecha.take() {
                if fin.is_empty() || (!self.hasta.is_empty() && fin >= self.hasta) {
                    self.acabado = true;
                } else {
                    self.desde = fin;
                }
                continue;
            }
            if let Err(e) = self.paso()
                && let Err(e) = self.recuperar(e)
            {
                self.acabado = true;
                return Some(Err(e));
            }
        }
    }
}

impl Drop for Lector<'_> {
    fn drop(&mut self) {
        // Soltarlo a medias deja un scanner vivo en el servidor hasta que caduque su arriendo: se cierra, sin
        // esperar mucho ni quejarse si ya no esta.
        if let Some(s) = self.scanner.take()
            && !self.cancel.cancelada()
            && let Some(con) = self.conexiones.get_mut(&s.servidor)
        {
            let _ = con.llamar::<_, ScanResponse>("Scan", &cerrar_scanner(s.id, s.seq));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_clave_siguiente_es_la_misma_con_un_cero_detras() {
        assert_eq!(siguiente(b"ab"), b"ab\0");
        assert!(siguiente(b"ab").as_slice() > b"ab".as_slice());
        assert!(siguiente(b"ab").as_slice() < b"ab\x01".as_slice());
    }

    #[test]
    fn el_menor_fin_trata_el_vacio_como_sin_limite() {
        assert_eq!(menor_fin(b"", b""), b"");
        assert_eq!(menor_fin(b"", b"m"), b"m");
        assert_eq!(menor_fin(b"c", b""), b"c");
        assert_eq!(menor_fin(b"c", b"m"), b"c");
        assert_eq!(menor_fin(b"z", b"m"), b"m");
    }
}
