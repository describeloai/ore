//! **Quien escribe el flujo IPC de un lector** (ORE 0075, fase 5): la cabecera, un lote tras otro (cada uno vaciado, para
//! que el que lee lo vea al momento) y el cierre. Lo repetían siete lectores con los mismos tres mensajes de error.
//!
//! Los fallos salen ya como [`Fallo`] del protocolo: no poder empezar es del origen (`Codigo::Origen`); no poder
//! escribir o cerrar es de la conexión con quien lee.

use arrow_array::RecordBatch;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::Schema;
use ore_driver::fallo::{Codigo, Fallo};
use std::io::Write;

/// El flujo IPC de un lector sobre su salida.
pub struct Emisor<W: Write> {
    flujo: StreamWriter<W>,
}

impl<W: Write> Emisor<W> {
    /// Escribe la cabecera con el esquema.
    pub fn nuevo(salida: W, esquema: &Schema) -> Result<Emisor<W>, Fallo> {
        StreamWriter::try_new(salida, esquema)
            .map(|flujo| Emisor { flujo })
            .map_err(|e| Fallo::origen(format!("no se pudo empezar el flujo: {e}")))
    }

    /// Escribe un lote y lo vacía.
    pub fn lote(&mut self, lote: &RecordBatch) -> Result<(), Fallo> {
        self.flujo
            .write(lote)
            .and_then(|_| self.flujo.flush())
            .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo escribir: {e}")))
    }

    /// Cierra el flujo (la marca de fin).
    pub fn fin(&mut self) -> Result<(), Fallo> {
        self.flujo
            .finish()
            .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo cerrar el flujo: {e}")))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use arrow_array::{ArrayRef, Int64Array};
    use arrow_ipc::reader::StreamReader;
    use arrow_schema::{DataType, Field};
    use std::sync::Arc;

    #[test]
    fn un_flujo_con_dos_lotes_se_lee_entero() {
        let esquema = Schema::new(vec![Field::new("a", DataType::Int64, true)]);
        let lote = |v: Vec<i64>| {
            let col: ArrayRef = Arc::new(Int64Array::from(v));
            RecordBatch::try_new(Arc::new(esquema.clone()), vec![col]).unwrap()
        };
        let mut buf = Vec::new();
        let mut e = Emisor::nuevo(&mut buf as &mut dyn Write, &esquema).unwrap();
        e.lote(&lote(vec![1, 2])).unwrap();
        e.lote(&lote(vec![3])).unwrap();
        e.fin().unwrap();
        let filas: usize = StreamReader::try_new(std::io::Cursor::new(buf), None)
            .unwrap()
            .map(|l| l.unwrap().num_rows())
            .sum();
        assert_eq!(filas, 3);
    }

    #[test]
    fn una_salida_rota_es_un_fallo_de_conexion() {
        struct Rota;
        impl Write for Rota {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("tubería rota"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let esquema = Schema::new(vec![Field::new("a", DataType::Int64, true)]);
        // La cabecera ya es una escritura: sin salida no se empieza, y eso es del origen.
        let Err(f) = Emisor::nuevo(Rota, &esquema) else {
            panic!("debía fallar")
        };
        assert!(f.mensaje.starts_with("no se pudo empezar el flujo"));
    }
}
