//! **El banco de S3**: un bucket `ore-kit` en un S3 compatible —el S3 de
//! mentira de `pruebas-de-fuego/de-mentira.py` en el CI, o un MinIO—, con la
//! semilla en Parquet bajo `tipos/`, `vacia/` y `grande/`.
//!
//! `S3_KIT_ENDPOINT`, `S3_KIT_CLAVE` y `S3_KIT_SECRETO` dicen dónde y con qué.
//! Es un S3 de pruebas: **nunca el bucket de un cliente** —la credencial de
//! éste es de sólo lectura y el kit escribe—.
//!
//! Un bucket no tiene sesiones, ni consultas que cancelar, ni nada que una
//! lectura pueda escribir: esos casos salen «no aplica».

use super::Banco;
use crate::respuesta::tipo_arrow;
use crate::semilla::{self, Columna, FILAS, GRANDE, GRANDE_COLUMNAS, TIPOS, Tabla};
use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{Field, Schema};
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use ore_sigv4::{Bucket, Credencial, firma, hex, sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

const BUCKET: &str = "ore-kit";
const REGION: &str = "us-east-1";
/// `grande` en tantos ficheros: un listado de varios, como una tabla de verdad.
const PARTES: u64 = 10;

pub struct S3 {
    endpoint: String,
    clave: String,
    secreto: String,
    /// Si el S3 de pruebas comprueba la firma: MinIO sí; el S3 de mentira de
    /// `pruebas-de-fuego/de-mentira.py`, no.
    firma: bool,
}

impl S3 {
    pub fn new(endpoint: &str, clave: &str, secreto: &str, firma: bool) -> S3 {
        S3 {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            clave: clave.to_string(),
            secreto: secreto.to_string(),
            firma,
        }
    }

    fn bucket(&self) -> Bucket {
        Bucket {
            endpoint: self.endpoint.clone(),
            bucket: BUCKET.to_string(),
            region: REGION.to_string(),
            en_ruta: true,
            credencial: Credencial {
                clave: self.clave.clone(),
                secreto: self.secreto.clone(),
                token: None,
            },
        }
    }

    /// Un `PUT` firmado: el bucket (`clave: None`) o un objeto.
    fn poner(&self, clave: Option<&str>, cuerpo: &[u8]) -> Result<u16, String> {
        let b = self.bucket();
        let ruta = b.ruta(clave);
        let cabeceras = firma::firmar(
            &b.credencial,
            &b.region,
            &b.host(),
            "PUT",
            &ruta,
            "",
            Vec::new(),
            &hex(&sha256(cuerpo)),
        );
        let mut r = ureq::put(&format!("{}{ruta}", b.endpoint));
        for (k, v) in &cabeceras {
            if k != "host" {
                r = r.set(k, v);
            }
        }
        match r.send_bytes(cuerpo) {
            Ok(x) => Ok(x.status()),
            Err(ureq::Error::Status(s, _)) => Ok(s),
            Err(e) => Err(format!("no se llega al S3 de pruebas: {e}")),
        }
    }

    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<(), String> {
        match self.poner(Some(clave), cuerpo)? {
            200 => Ok(()),
            s => Err(format!("`{clave}` no se sube: {s}")),
        }
    }
}

/// Un lote de Arrow con estas columnas y estas filas en texto canónico.
pub(super) fn lote(
    columnas: &[Columna],
    filas: &[Vec<Option<String>>],
) -> Result<RecordBatch, String> {
    let mut arrays: Vec<ArrayRef> = Vec::new();
    for (i, c) in columnas.iter().enumerate() {
        let f = c.fisico();
        let vs: Vec<Option<Valor>> = filas
            .iter()
            .map(|fila| fila[i].as_deref().and_then(|t| f.analizar(t)))
            .collect();
        let a: ArrayRef = match f {
            Fisico::Entero => {
                let mut b = Int64Builder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::Entero(n)) => Some(*n),
                        _ => None,
                    })
                });
                Arc::new(b.finish())
            }
            Fisico::Real => {
                let mut b = Float64Builder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::Real(n)) => Some(*n),
                        _ => None,
                    })
                });
                Arc::new(b.finish())
            }
            Fisico::Logico => {
                let mut b = BooleanBuilder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::Logico(n)) => Some(*n),
                        _ => None,
                    })
                });
                Arc::new(b.finish())
            }
            Fisico::Decimal { precision, escala } => {
                let mut b = Decimal128Builder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::Decimal(n)) => Some(*n),
                        _ => None,
                    })
                });
                Arc::new(
                    b.finish()
                        .with_precision_and_scale(precision, escala as i8)
                        .map_err(|e| e.to_string())?,
                )
            }
            Fisico::Fecha => {
                let mut b = Date32Builder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::Fecha(n)) => Some(*n),
                        _ => None,
                    })
                });
                Arc::new(b.finish())
            }
            Fisico::FechaHora => {
                let mut b = TimestampMicrosecondBuilder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::FechaHora(n)) => Some(*n),
                        _ => None,
                    })
                });
                Arc::new(b.finish())
            }
            Fisico::Instante => {
                let mut b = TimestampMicrosecondBuilder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::Instante(n)) => Some(*n),
                        _ => None,
                    })
                });
                Arc::new(b.finish().with_timezone("+00:00"))
            }
            Fisico::Texto | Fisico::Hora => {
                let mut b = StringBuilder::new();
                vs.iter().for_each(|v| {
                    b.append_option(match v {
                        Some(Valor::Texto(n)) => Some(n.as_str()),
                        _ => None,
                    })
                });
                Arc::new(b.finish())
            }
        };
        arrays.push(a);
    }
    let esquema = Schema::new(
        columnas
            .iter()
            .map(|c| Field::new(c.nombre, tipo_arrow(&c.fisico()), c.nombre != "id"))
            .collect::<Vec<_>>(),
    );
    RecordBatch::try_new(Arc::new(esquema), arrays).map_err(|e| e.to_string())
}

/// Un Parquet de un lote.
pub(super) fn parquet(l: &RecordBatch) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut w = parquet::arrow::ArrowWriter::try_new(&mut out, l.schema(), None)
        .map_err(|e| e.to_string())?;
    w.write(l).map_err(|e| e.to_string())?;
    w.close().map_err(|e| e.to_string())?;
    Ok(out)
}

impl Banco for S3 {
    fn familia(&self) -> &'static str {
        "s3"
    }

    fn cargar(&mut self) -> Result<(), String> {
        // 200 si se crea; 409 si ya era nuestro.
        match self.poner(None, b"")? {
            200 | 409 => {}
            s => return Err(format!("el bucket `{BUCKET}` no se crea: {s}")),
        }
        sembrar(|clave, cuerpo| self.subir(clave, cuerpo))
    }

    fn url(&self) -> String {
        format!(
            "s3://{BUCKET}?region={REGION}&endpoint={}&access_key_id={}&secret_access_key={}",
            self.endpoint, self.clave, self.secreto
        )
    }

    fn url_alternativa(&self) -> Option<String> {
        None
    }

    fn url_mala(&self) -> Option<String> {
        self.firma.then(|| {
            format!(
                "s3://{BUCKET}?region={REGION}&endpoint={}&access_key_id={}&secret_access_key=una-clave-que-no-es",
                self.endpoint, self.clave
            )
        })
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{}/", tabla.nombre())
    }

    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        completar_fichero(tabla, peticion)
    }
}

/// **La semilla de un almacén de objetos**, en Parquet: `tipos/`, `vacia/` y
/// `grande/` en [`PARTES`] ficheros. `subir` pone un objeto; lo comparten S3 y
/// GCS (ADR 0061 O2·3).
pub(super) fn sembrar(
    mut subir: impl FnMut(&str, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let tipos: Vec<Vec<Option<String>>> = FILAS
        .iter()
        .map(|f| f.iter().map(|v| v.map(String::from)).collect())
        .collect();
    subir("tipos/parte-0.parquet", &parquet(&lote(TIPOS, &tipos)?)?)?;
    subir("vacia/parte-0.parquet", &parquet(&lote(TIPOS, &[])?)?)?;
    let por_parte = GRANDE / PARTES;
    for k in 0..PARTES {
        let filas: Vec<Vec<Option<String>>> = (k * por_parte + 1..=(k + 1) * por_parte)
            .map(|id| semilla::fila_grande(id).into_iter().map(Some).collect())
            .collect();
        subir(
            &format!("grande/parte-{k}.parquet"),
            &parquet(&lote(GRANDE_COLUMNAS, &filas)?)?,
        )?;
    }
    Ok(())
}

/// El `format` de la tabla y sus tipos congelados (v1alpha16 `03` §1), como
/// los manda el coordinador desde el árbol.
pub(super) fn completar_fichero(tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
    let tipos = tabla
        .columnas()
        .iter()
        .map(|c| Json::Arr(vec![Json::s(c.nombre), Json::s(c.tipo)]))
        .collect();
    peticion.insert(
        "fichero".into(),
        Json::obj([
            (
                "format",
                Json::obj([
                    ("type", Json::s("parquet")),
                    ("match", Json::s("*.parquet")),
                ]),
            ),
            ("tipos", Json::Arr(tipos)),
        ]),
    );
}
