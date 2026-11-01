//! **Donde las filas se vuelven un flujo Arrow** (ADR 0083 §1.5 y §1.6).
//!
//! Las dos vías de lectura (REST y Bulk) entregan **filas de texto** en el orden
//! de [`Traduccion::columnas`]; esto hace lo que es igual para las dos:
//!
//! ```text
//!   celdas (texto) ──residuales──▶ ──limit──▶ ──convertir──▶ RecordBatchBuilder ──▶ Emisor (IPC)
//! ```
//!
//! 1. **Reevalúa los residuales** (`residual`): lo que el servidor aplicó de más.
//! 2. **Corta en `limit`**, ya reevaluado: el servidor no pudo cortar (con un
//!    residual su `LIMIT` daría de menos) ni a través de varias consultas.
//! 3. **Convierte** cada celda proyectada a su tipo (`valores`); un valor que no
//!    cabe es un error con columna y fila, nunca un nulo.
//! 4. Vacía un lote cada [`FILAS_POR_LOTE`] filas.
//!
//! Las columnas que solo se trajeron para un residual se leen y se tiran: no
//! salen en el lote (0008: la proyección es una lista cerrada).

use crate::esquema::{Campo, Objeto};
use crate::residual;
use crate::traductor::{COLUMNA_OPERACION, OP_DELETE, OP_UPSERT, Traduccion};
use crate::valores;
use arrow_array::RecordBatch;
use ore_arrow::{Columna, Emisor, RecordBatchBuilder};
use ore_base::tipos::{Fisico, Valor};
use ore_driver::{Codigo, Fallo, Filtro};
use std::io::Write;

/// Filas por lote Arrow.
pub const FILAS_POR_LOTE: usize = 8192;

/// Una columna del lote: una celda leída, o la operación (virtual).
enum Col {
    /// `(posición en la fila leída, campo)`.
    Celda(usize, Campo),
    /// [`COLUMNA_OPERACION`]: `upsert` en una fila leída, `delete` en un borrado.
    Operacion,
}

pub struct Salida<'a> {
    emisor: Emisor<&'a mut dyn Write>,
    lote: RecordBatchBuilder,
    /// Cada columna proyectada, en orden.
    proyectadas: Vec<Col>,
    residuales: Vec<(usize, Filtro)>,
    limit: Option<u64>,
    emitidas: u64,
    vistas: u64,
    fila: Vec<Option<Valor>>,
}

impl<'a> Salida<'a> {
    pub fn nueva(
        salida: &'a mut dyn Write,
        t: &Traduccion,
        o: &Objeto,
    ) -> Result<Salida<'a>, Fallo> {
        let pos = |col: &str| {
            t.columnas.iter().position(|c| c == col).ok_or_else(|| {
                Fallo::origen(format!("la columna `{col}` no está entre las que se piden"))
            })
        };
        let mut proyectadas = Vec::new();
        let mut cols = Vec::new();
        for (prop, col) in &t.rutas {
            if col == COLUMNA_OPERACION {
                cols.push(Columna::new(prop, Fisico::Texto, "String"));
                proyectadas.push(Col::Operacion);
                continue;
            }
            let c = o.campo(col).ok_or_else(|| {
                Fallo::new(
                    Codigo::Esquema,
                    format!("`{}.{col}`: el objeto no tiene esa columna", o.nombre),
                )
            })?;
            cols.push(valores::columna(prop, c));
            proyectadas.push(Col::Celda(pos(col)?, c.clone()));
        }
        let residuales = t
            .residuales
            .iter()
            .map(|f| Ok((pos(&f.columna)?, f.clone())))
            .collect::<Result<Vec<_>, Fallo>>()?;
        let lote = RecordBatchBuilder::con_capacidad(&cols, FILAS_POR_LOTE)
            .map_err(|e| Fallo::origen(e.0))?;
        let emisor = Emisor::nuevo(salida, &lote.schema())?;
        Ok(Salida {
            emisor,
            lote,
            proyectadas,
            residuales,
            limit: t.limit,
            emitidas: 0,
            vistas: 0,
            fila: Vec::new(),
        })
    }

    /// Una fila leída, con las celdas en el orden de `Traduccion::columnas`.
    /// **`false` es «para»: se llegó al `limit`.**
    pub fn fila(&mut self, celdas: &[Option<String>]) -> Result<bool, Fallo> {
        self.vistas += 1;
        for (i, f) in &self.residuales {
            if !residual::cumple(f, celdas[*i].as_deref()) {
                return Ok(true);
            }
        }
        if self.limit.is_some_and(|n| self.emitidas >= n) {
            return Ok(false);
        }
        self.fila.clear();
        for col in &self.proyectadas {
            let v = match col {
                Col::Celda(i, c) => valores::convertir(c, celdas[*i].as_deref()).map_err(|m| {
                    Fallo::new(
                        Codigo::Esquema,
                        format!("columna `{}`, fila {}: {m}", c.nombre, self.vistas),
                    )
                })?,
                Col::Operacion => Some(Valor::Texto(OP_UPSERT.into())),
            };
            self.fila.push(v);
        }
        self.lote.push(&self.fila).map_err(|e| Fallo::origen(e.0))?;
        self.emitidas += 1;
        if self.lote.filas() >= FILAS_POR_LOTE {
            self.vaciar()?;
        }
        Ok(self.limit.is_none_or(|n| self.emitidas < n))
    }

    /// **Un borrado**: solo la clave y la operación; el resto, nulo (un borrado no
    /// trae el documento). `id` es el `Id` del registro retirado.
    pub fn baja(&mut self, id: &str) -> Result<(), Fallo> {
        self.vistas += 1;
        self.fila.clear();
        for col in &self.proyectadas {
            self.fila.push(match col {
                Col::Operacion => Some(Valor::Texto(OP_DELETE.into())),
                Col::Celda(_, c) if c.nombre == "Id" => Some(Valor::Texto(id.to_string())),
                Col::Celda(..) => None,
            });
        }
        self.lote.push(&self.fila).map_err(|e| Fallo::origen(e.0))?;
        self.emitidas += 1;
        if self.lote.filas() >= FILAS_POR_LOTE {
            self.vaciar()?;
        }
        Ok(())
    }

    fn vaciar(&mut self) -> Result<(), Fallo> {
        let b: RecordBatch = self.lote.finish().map_err(|e| Fallo::origen(e.0))?;
        self.emisor.lote(&b)
    }

    /// Las filas emitidas hasta ahora.
    pub fn emitidas(&self) -> u64 {
        self.emitidas
    }

    /// Cierra el flujo y devuelve las filas emitidas.
    pub fn terminar(mut self) -> Result<u64, Fallo> {
        if self.lote.filas() > 0 {
            self.vaciar()?;
        }
        self.emisor.fin()?;
        Ok(self.emitidas)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::esquema::pruebas::cuenta;
    use crate::traductor::traducir;
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Decimal128Type, Int64Type, TimestampMicrosecondType};
    use arrow_ipc::reader::StreamReader;
    use arrow_schema::DataType;
    use ore_driver::Peticion;

    fn pet(cols: &[&str], filtros: Vec<Filtro>, limit: Option<u64>) -> Peticion {
        Peticion {
            objeto: "Account".into(),
            proyeccion: cols
                .iter()
                .map(|c| (c.to_lowercase(), c.to_string()))
                .collect(),
            filtros,
            limit,
            ..Default::default()
        }
    }

    fn s(x: &str) -> Option<String> {
        Some(x.to_string())
    }

    /// Escribe las filas por la `Salida` y lee el IPC de vuelta.
    fn correr(
        p: &Peticion,
        filas: &[Vec<Option<String>>],
    ) -> Result<(Vec<RecordBatch>, u64), Fallo> {
        let o = cuenta();
        let t = traducir(p, &o)?;
        let mut buf: Vec<u8> = Vec::new();
        let n = {
            let mut sal = Salida::nueva(&mut buf, &t, &o)?;
            for f in filas {
                if !sal.fila(f)? {
                    break;
                }
            }
            sal.terminar()?
        };
        let lotes = StreamReader::try_new(std::io::Cursor::new(buf), None)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        Ok((lotes, n))
    }

    /// Lo medido en X0: tipos, formato de REST y de Bulk en el mismo lote.
    #[test]
    fn las_celdas_se_vuelven_arrow_con_su_tipo() {
        let p = pet(
            &[
                "Id",
                "NumberOfEmployees",
                "X0_Formula__c",
                "SystemModstamp",
                "IsDeleted",
            ],
            vec![],
            None,
        );
        let (lotes, n) = correr(
            &p,
            &[
                // REST: fecha `+0000`, entero `10`, fórmula `20.0`.
                vec![
                    s("001A"),
                    s("10"),
                    s("20.0"),
                    s("2026-10-31T20:39:17.000+0000"),
                    s("false"),
                ],
                // Bulk: la misma fecha con `Z`, y nulos en celdas vacías.
                vec![
                    s("001B"),
                    None,
                    None,
                    s("2026-10-31T20:39:17.000Z"),
                    s("true"),
                ],
            ],
        )
        .unwrap();
        assert_eq!(n, 2);
        let b = &lotes[0];
        assert_eq!(b.num_rows(), 2);
        assert_eq!(b.schema().field(1).data_type(), &DataType::Int64);
        let ent = b.column(1).as_primitive::<Int64Type>();
        assert_eq!((ent.value(0), ent.is_null(1)), (10, true));
        let dec = b.column(2).as_primitive::<Decimal128Type>();
        assert_eq!((dec.value(0), dec.is_null(1)), (20, true));
        let t = b.column(3).as_primitive::<TimestampMicrosecondType>();
        assert_eq!(t.value(0), t.value(1), "REST y Bulk: el mismo instante");
        assert_eq!(t.value(0), 1_793_479_157_000_000);
        let l = b.column(4).as_boolean();
        assert_eq!((l.value(0), l.value(1)), (false, true));
    }

    /// Un residual sin su columna en la proyección: se lee, se reevalúa y se tira.
    #[test]
    fn un_residual_reevalua_y_su_columna_no_sale() {
        let p = pet(
            &["Id"],
            vec![Filtro::uno("Name", "eq", "X0-Cuenta-001")],
            None,
        );
        let (lotes, n) = correr(
            &p,
            &[
                vec![s("001A"), s("X0-Cuenta-001")],
                // SOQL lo devolvió por no distinguir mayúsculas; SQL, no.
                vec![s("001B"), s("x0-cuenta-001")],
                vec![s("001C"), None],
            ],
        )
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            lotes[0].num_columns(),
            1,
            "`Name` solo estaba para el residual"
        );
        assert_eq!(lotes[0].column(0).as_string::<i32>().value(0), "001A");
    }

    /// El `LIMIT` no bajó al servidor (había residual): corta aquí, **ya reevaluado**.
    #[test]
    fn el_limit_corta_despues_de_reevaluar() {
        let p = pet(&["Id"], vec![Filtro::uno("Name", "like", "X0-%")], Some(2));
        let filas: Vec<_> = (0..10)
            .map(|i| {
                vec![
                    s(&format!("00{i}")),
                    s(if i % 2 == 0 { "X0-a" } else { "otro" }),
                ]
            })
            .collect();
        let (lotes, n) = correr(&p, &filas).unwrap();
        // Las pares cumplen: 000, 002, 004…; con limit 2, 000 y 002.
        assert_eq!(n, 2);
        let ids = lotes[0].column(0).as_string::<i32>();
        assert_eq!((ids.value(0), ids.value(1)), ("000", "002"));
    }

    #[test]
    fn la_fila_que_alcanza_el_limit_dice_que_pare() {
        let o = cuenta();
        let t = traducir(&pet(&["Id"], vec![], Some(1)), &o).unwrap();
        let mut buf: Vec<u8> = Vec::new();
        let mut sal = Salida::nueva(&mut buf, &t, &o).unwrap();
        assert!(
            !sal.fila(&[s("001A")]).unwrap(),
            "con la primera ya está el límite"
        );
        assert_eq!(sal.emitidas(), 1);
    }

    /// Un valor que no cabe es un error con columna y fila: nunca un nulo.
    #[test]
    fn un_valor_que_no_cabe_nombra_columna_y_fila() {
        let p = pet(&["Id", "NumberOfEmployees"], vec![], None);
        let e = correr(&p, &[vec![s("a"), s("1")], vec![s("b"), s("muchos")]]).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(
            e.mensaje.contains("`NumberOfEmployees`") && e.mensaje.contains("fila 2"),
            "{}",
            e.mensaje
        );
    }

    /// ADR 0083 §1.8: una lectura de cambios marca cada fila, y un borrado lleva
    /// solo la clave (un borrado no trae el documento).
    #[test]
    fn la_operacion_marca_altas_y_bajas_y_la_baja_solo_lleva_la_clave() {
        let o = cuenta();
        let t = traducir(&pet(&["Id", "Name", "_ore_op"], vec![], None), &o).unwrap();
        let mut buf: Vec<u8> = Vec::new();
        let n = {
            let mut sal = Salida::nueva(&mut buf, &t, &o).unwrap();
            assert!(sal.fila(&[s("001A"), s("Acme")]).unwrap());
            sal.baja("001B").unwrap();
            sal.terminar().unwrap()
        };
        assert_eq!(n, 2);
        let l = StreamReader::try_new(std::io::Cursor::new(buf), None)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let b = &l[0];
        let nombres: Vec<_> = b
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert_eq!(nombres, ["id", "name", "_ore_op"]);
        let (id, nombre, op) = (
            b.column(0).as_string::<i32>(),
            b.column(1).as_string::<i32>(),
            b.column(2).as_string::<i32>(),
        );
        assert_eq!(
            (id.value(0), nombre.value(0), op.value(0)),
            ("001A", "Acme", "upsert")
        );
        assert_eq!(
            (id.value(1), nombre.is_null(1), op.value(1)),
            ("001B", true, "delete")
        );
    }

    /// El resultado vacío también es un flujo, con su esquema (el contrato del kit).
    #[test]
    fn sin_filas_el_flujo_lleva_el_esquema() {
        let (lotes, n) = correr(&pet(&["Id", "Name"], vec![], None), &[]).unwrap();
        assert_eq!((n, lotes.len()), (0, 0));
        let o = cuenta();
        let t = traducir(&pet(&["Id", "Name"], vec![], None), &o).unwrap();
        let mut buf: Vec<u8> = Vec::new();
        Salida::nueva(&mut buf, &t, &o).unwrap().terminar().unwrap();
        let r = StreamReader::try_new(std::io::Cursor::new(buf), None).unwrap();
        assert_eq!(r.schema().fields().len(), 2);
    }

    /// Más de un lote: 8192 filas por lote.
    #[test]
    fn muchas_filas_salen_en_varios_lotes() {
        let filas: Vec<_> = (0..FILAS_POR_LOTE + 5)
            .map(|i| vec![s(&format!("{i}"))])
            .collect();
        let (lotes, n) = correr(&pet(&["Id"], vec![], None), &filas).unwrap();
        assert_eq!(n as usize, FILAS_POR_LOTE + 5);
        assert_eq!(
            lotes.iter().map(RecordBatch::num_rows).collect::<Vec<_>>(),
            [FILAS_POR_LOTE, 5]
        );
    }
}
