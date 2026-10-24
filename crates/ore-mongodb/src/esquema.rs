//! **El esquema de una colección, inferido de una muestra** (ficha 0069 §4.2–4.4).
//!
//! Postgres y BigQuery se *preguntan*; MongoDB no tiene qué preguntar. El
//! catálogo toma documentos ([`crate::transporte`]), los pasa por un
//! [`Observador`] y de ahí sale un [`Esquema`]: una columna por **hoja** del
//! documento, con su tipo de OOS (0032) y los avisos de lo que no se dejó tipar.
//!
//! # Las reglas
//!
//! - **Una columna por hoja.** Los documentos anidados se aplanan hasta una
//!   profundidad ([`PROFUNDIDAD_POR_DEFECTO`]) y la ruta física es el nombre
//!   (`awards.wins`), como manda OOS v1alpha1 §4.4. Los arrays, y lo que pase de
//!   la profundidad, son **una** columna de JSON: ni Fivetran ni Airbyte los
//!   parten en tablas hijas.
//! - **Un campo que es documento en unos y otra cosa en otros no se aplana**:
//!   sería una columna y a la vez el padre de otras. Va entero como JSON.
//! - **El tipo es el que dejan los valores no nulos.** `null` y ausente no
//!   cuentan para el tipo.
//! - **Lo que se mezcla, texto.** Si los valores no caben en un tipo común la
//!   columna es `String`, cada valor con su forma canónica, y el aviso dice qué
//!   clases se vieron (la tercera vía de 0032 §4: ni un nulo inventado ni un
//!   fallo por una fila). Enteros con decimales son `Float`; enteros con
//!   `Decimal128`, `Decimal`.
//! - **Una muestra no ve lo que aparece tarde.** Un valor de una clase que la
//!   muestra no vio no cabe en la columna: [`crate::lote`] lo dice con columna
//!   y fila, no lo inventa.

use crate::decimal;
use mongodb::bson::raw::{RawBsonRef, RawDocument};
use mongodb::bson::spec::BinarySubtype;
use ore_base::tipos::Fisico;
use std::collections::{BTreeMap, BTreeSet};

/// Hasta qué nivel se aplanan los documentos anidados.
pub const PROFUNDIDAD_POR_DEFECTO: usize = 3;

/// El tipo de una columna: el escalar de OOS y cómo vive en el origen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tipo {
    /// `Int32`, `Int64`. `Integer` (int64).
    Entero,
    /// `Double`, o enteros y `Double` juntos. `Float`.
    Real,
    /// `Decimal128` (y enteros): `Decimal<precision, escala>`.
    Decimal {
        precision: u8,
        escala: u8,
    },
    Logico,
    /// `Date`: un instante, `DateTimeTz`.
    Instante,
    /// Solo por una declaración de la tabla (`Date`): el día UTC de un `Date` de BSON.
    Fecha,
    /// Solo por una declaración (`DateTime`, sin zona): la hora de un `Date` de BSON, en UTC.
    FechaHora,
    Texto,
    /// `ObjectId`: `String` con el hex de 24, como Fivetran. Comparable.
    ObjectId,
    /// `Binary` de subtipo 4: `String` con el UUID.
    Uuid,
    /// Cualquier otro `Binary`: `Opaque`, en base64.
    Binario,
    /// Un array, un documento que no se aplana, o un tipo que OOS no tiene
    /// (`Regex`, `Code`, `MinKey`…): `String` con extended JSON.
    Json,
    /// Clases que no caben en un tipo común: `String`.
    Mixto,
}

impl Tipo {
    /// El físico de Arrow que le toca (0032 §1).
    pub fn fisico(&self) -> Fisico {
        match self {
            Tipo::Entero => Fisico::Entero,
            Tipo::Real => Fisico::Real,
            Tipo::Decimal { precision, escala } => Fisico::Decimal {
                precision: *precision,
                escala: *escala,
            },
            Tipo::Logico => Fisico::Logico,
            Tipo::Instante => Fisico::Instante,
            Tipo::Fecha => Fisico::Fecha,
            Tipo::FechaHora => Fisico::FechaHora,
            Tipo::Texto
            | Tipo::ObjectId
            | Tipo::Uuid
            | Tipo::Binario
            | Tipo::Json
            | Tipo::Mixto => Fisico::Texto,
        }
    }

    /// El tipo de OOS, como lo escribe el catálogo (`type`).
    pub fn oos(&self) -> String {
        match self {
            Tipo::Entero => "Integer".into(),
            Tipo::Real => "Float".into(),
            Tipo::Decimal { precision, escala } => format!("Decimal<{precision}, {escala}>"),
            Tipo::Logico => "Boolean".into(),
            Tipo::Instante => "DateTimeTz".into(),
            Tipo::Fecha => "Date".into(),
            Tipo::FechaHora => "DateTime".into(),
            Tipo::Binario => "Opaque".into(),
            Tipo::Texto | Tipo::ObjectId | Tipo::Uuid | Tipo::Json | Tipo::Mixto => "String".into(),
        }
    }

    /// El tipo tal como lo dice el origen (`physicalType`).
    pub fn fisico_origen(&self) -> &'static str {
        match self {
            Tipo::Entero => "int/long",
            Tipo::Real => "double",
            Tipo::Decimal { .. } => "decimal",
            Tipo::Logico => "bool",
            Tipo::Instante | Tipo::Fecha | Tipo::FechaHora => "date",
            Tipo::Texto => "string",
            Tipo::ObjectId => "objectId",
            Tipo::Uuid => "uuid",
            Tipo::Binario => "binData",
            Tipo::Json => "array/object",
            Tipo::Mixto => "mixed",
        }
    }

    /// Si un filtro y un orden de SQL significan lo mismo sobre esta columna.
    /// Un tipo mezclado o compuesto **no se empuja**: `{v: x}` buscaría dentro
    /// de un array y compararía solo la clase que casa (medido en X0).
    pub fn empujable(&self) -> bool {
        !matches!(self, Tipo::Binario | Tipo::Json | Tipo::Mixto)
    }
}

impl Tipo {
    /// **El tipo que sale de juntar lo que la tabla declara con lo que el dato
    /// es.** Lo declarado manda cuando el dato lo puede llevar; si no, es un
    /// fallo del esquema —la tabla dice una cosa y la colección otra—, no una
    /// conversión a escondidas.
    ///
    /// | declarado | el dato debe ser | sale |
    /// |---|---|---|
    /// | `String` | lo que sea | texto |
    /// | `Integer`, `Float`, `Boolean` | lo mismo | lo mismo |
    /// | `Decimal<p, s>` | decimal (o entero) | decimal con **esa** precisión y escala |
    /// | `Date`, `DateTime` | una fecha de BSON | el día / la hora de pared, en UTC |
    /// | `DateTimeTz` | una fecha de BSON | instante |
    ///
    /// Un tipo que no está aquí (`Time`, lo compuesto…) se rechaza: no se
    /// inventa una lectura que nadie probó.
    pub fn declarado(&self, declarado: &str) -> Result<Tipo, String> {
        use ore_base::types::{Type, parse_type};
        let t = parse_type(declarado)
            .map_err(|e| format!("`{declarado}` no es un tipo de OOS: {e:?}"))?;
        let no = |dato: &str| {
            Err(format!(
                "la tabla declara `{declarado}` y el dato es {dato}"
            ))
        };
        match (Fisico::de(&t), &t, self) {
            (Fisico::Texto, Type::Scalar(s), _) if s == "String" => Ok(Tipo::Texto),
            (Fisico::Entero, _, Tipo::Entero) => Ok(Tipo::Entero),
            (Fisico::Real, _, Tipo::Real | Tipo::Entero) => Ok(Tipo::Real),
            (Fisico::Logico, _, Tipo::Logico) => Ok(Tipo::Logico),
            (Fisico::Decimal { precision, escala }, _, Tipo::Decimal { .. } | Tipo::Entero) => {
                Ok(Tipo::Decimal { precision, escala })
            }
            (Fisico::Fecha, _, Tipo::Instante) => Ok(Tipo::Fecha),
            (Fisico::FechaHora, _, Tipo::Instante) => Ok(Tipo::FechaHora),
            (Fisico::Instante, _, Tipo::Instante) => Ok(Tipo::Instante),
            (Fisico::Texto, Type::Scalar(s), _) => Err(format!(
                "la tabla declara `{declarado}` ({s}) y este origen no lo lee: usa `String`"
            )),
            (_, _, otro) => no(otro.fisico_origen()),
        }
    }
}

/// La clase BSON de un valor, en el vocabulario del inferidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Clase {
    Nulo,
    Int32,
    Int64,
    Double,
    Decimal,
    Texto,
    Logico,
    Fecha,
    ObjectId,
    Uuid,
    Binario,
    Timestamp,
    Array,
    Documento,
    /// Lo que OOS no tiene escalar para: `Regex`, `Code`, `Symbol`…
    SinTraducir(&'static str),
}

impl Clase {
    pub fn nombre(&self) -> &'static str {
        match self {
            Clase::Nulo => "null",
            Clase::Int32 => "int",
            Clase::Int64 => "long",
            Clase::Double => "double",
            Clase::Decimal => "decimal",
            Clase::Texto => "string",
            Clase::Logico => "bool",
            Clase::Fecha => "date",
            Clase::ObjectId => "objectId",
            Clase::Uuid => "uuid",
            Clase::Binario => "binData",
            Clase::Timestamp => "timestamp",
            Clase::Array => "array",
            Clase::Documento => "object",
            Clase::SinTraducir(n) => n,
        }
    }
}

pub fn clase_de(v: &RawBsonRef<'_>) -> Clase {
    match v {
        RawBsonRef::Null => Clase::Nulo,
        RawBsonRef::Int32(_) => Clase::Int32,
        RawBsonRef::Int64(_) => Clase::Int64,
        RawBsonRef::Double(_) => Clase::Double,
        RawBsonRef::Decimal128(_) => Clase::Decimal,
        RawBsonRef::String(_) => Clase::Texto,
        RawBsonRef::Boolean(_) => Clase::Logico,
        RawBsonRef::DateTime(_) => Clase::Fecha,
        RawBsonRef::ObjectId(_) => Clase::ObjectId,
        RawBsonRef::Binary(b) if b.subtype == BinarySubtype::Uuid && b.bytes.len() == 16 => {
            Clase::Uuid
        }
        RawBsonRef::Binary(_) => Clase::Binario,
        RawBsonRef::Timestamp(_) => Clase::Timestamp,
        RawBsonRef::Array(_) => Clase::Array,
        RawBsonRef::Document(_) => Clase::Documento,
        RawBsonRef::RegularExpression(_) => Clase::SinTraducir("regex"),
        RawBsonRef::JavaScriptCode(_) | RawBsonRef::JavaScriptCodeWithScope(_) => {
            Clase::SinTraducir("javascript")
        }
        RawBsonRef::Symbol(_) => Clase::SinTraducir("symbol"),
        RawBsonRef::Undefined => Clase::SinTraducir("undefined"),
        RawBsonRef::MaxKey => Clase::SinTraducir("maxKey"),
        RawBsonRef::MinKey => Clase::SinTraducir("minKey"),
        RawBsonRef::DbPointer(_) => Clase::SinTraducir("dbPointer"),
    }
}

/// Lo que se vio de una ruta.
#[derive(Debug, Default, Clone)]
struct Estadistica {
    clases: BTreeMap<Clase, u64>,
    /// De los `Decimal128` y los enteros: la escala mayor y las cifras enteras
    /// mayores (para saber si caben en `Decimal<38, s>`).
    escala_max: u32,
    enteras_max: u32,
    /// `NaN` o `±Infinity`: no caben en un decimal.
    decimal_especial: bool,
    hijos: BTreeSet<String>,
}

/// Recoge lo que se ve de cada ruta de una muestra de documentos.
#[derive(Debug, Clone)]
pub struct Observador {
    profundidad: usize,
    rutas: BTreeMap<String, Estadistica>,
    documentos: u64,
}

impl Observador {
    pub fn new(profundidad: usize) -> Observador {
        Observador {
            profundidad,
            rutas: BTreeMap::new(),
            documentos: 0,
        }
    }

    pub fn documentos(&self) -> u64 {
        self.documentos
    }

    /// Mira un documento. Un documento mal formado es un error, no se salta.
    pub fn observar(&mut self, doc: &RawDocument) -> Result<(), String> {
        self.documentos += 1;
        self.recorrer(doc, "", 1)
    }

    fn recorrer(&mut self, doc: &RawDocument, base: &str, nivel: usize) -> Result<(), String> {
        for par in doc.iter() {
            let (clave, valor) = par.map_err(|e| format!("documento mal formado: {e}"))?;
            let ruta = if base.is_empty() {
                clave.to_string()
            } else {
                format!("{base}.{clave}")
            };
            let clase = clase_de(&valor);
            if !base.is_empty() {
                self.rutas
                    .entry(base.to_string())
                    .or_default()
                    .hijos
                    .insert(ruta.clone());
            }
            let e = self.rutas.entry(ruta.clone()).or_default();
            *e.clases.entry(clase).or_insert(0) += 1;
            match &valor {
                RawBsonRef::Int32(n) => {
                    e.enteras_max = e.enteras_max.max(cifras(i64::from(*n)));
                }
                RawBsonRef::Int64(n) => {
                    e.enteras_max = e.enteras_max.max(cifras(*n));
                }
                RawBsonRef::Decimal128(d) => match decimal::de(d) {
                    Ok(dec) => {
                        let (escala, enteras) = dec.forma();
                        e.escala_max = e.escala_max.max(escala);
                        e.enteras_max = e.enteras_max.max(enteras);
                    }
                    Err(_) => e.decimal_especial = true,
                },
                RawBsonRef::Document(sub) if nivel < self.profundidad => {
                    self.recorrer(sub, &ruta, nivel + 1)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// El esquema que dejan las observaciones.
    pub fn esquema(&self) -> Esquema {
        let mut columnas = Vec::new();
        let mut avisos = Vec::new();
        let raices: Vec<&String> = self.rutas.keys().filter(|r| !r.contains('.')).collect();
        // `_id` primero, el resto por orden alfabético: determinista.
        let mut raices = raices;
        raices.sort_by(|a, b| (a.as_str() != "_id", *a).cmp(&(b.as_str() != "_id", *b)));
        for r in raices {
            self.hoja(r, &mut columnas, &mut avisos);
        }
        Esquema {
            columnas,
            documentos: self.documentos,
            avisos,
        }
    }

    fn hoja(&self, ruta: &str, columnas: &mut Vec<Columna>, avisos: &mut Vec<Aviso>) {
        let e = &self.rutas[ruta];
        let no_nulas: Vec<&Clase> = e.clases.keys().filter(|c| **c != Clase::Nulo).collect();
        // Un documento en todos los casos, y con hijos: no es una columna, son sus campos.
        if !no_nulas.is_empty()
            && no_nulas.iter().all(|c| **c == Clase::Documento)
            && !e.hijos.is_empty()
        {
            for h in &e.hijos {
                self.hoja(h, columnas, avisos);
            }
            return;
        }
        let presentes: u64 = e.clases.values().sum();
        let con_valor: u64 = e
            .clases
            .iter()
            .filter(|(c, _)| **c != Clase::Nulo)
            .map(|(_, n)| *n)
            .sum();
        let (tipo, aviso) = resolver(e, &no_nulas);
        if let Some(mensaje) = aviso {
            avisos.push(Aviso {
                ruta: ruta.to_string(),
                mensaje,
            });
        }
        columnas.push(Columna {
            ruta: ruta.to_string(),
            tipo,
            presentes,
            con_valor,
        });
    }
}

/// Cuántas cifras tiene la parte entera de un `i64`.
fn cifras(n: i64) -> u32 {
    n.unsigned_abs().to_string().len() as u32
}

/// El tipo de una ruta a partir de las clases que se vieron, y el aviso si hubo
/// que ceder.
fn resolver(e: &Estadistica, clases: &[&Clase]) -> (Tipo, Option<String>) {
    use Clase::*;
    let tiene = |c: Clase| clases.iter().any(|x| **x == c);
    let solo = |permitidas: &[Clase]| clases.iter().all(|c| permitidas.contains(c));
    let nombres = || {
        clases
            .iter()
            .map(|c| c.nombre())
            .collect::<Vec<_>>()
            .join(", ")
    };

    if clases.is_empty() {
        // Siempre null: no hay valor que tipar.
        return (
            Tipo::Texto,
            Some("todos los valores vistos son null".into()),
        );
    }
    if solo(&[Int32, Int64]) {
        return (Tipo::Entero, None);
    }
    if solo(&[Double]) || (tiene(Double) && solo(&[Double, Int32, Int64])) {
        return (Tipo::Real, None);
    }
    if tiene(Decimal) && solo(&[Decimal, Int32, Int64]) {
        if e.decimal_especial {
            return (
                Tipo::Mixto,
                Some(
                    "hay `NaN` o `Infinity` en un `Decimal128` y no caben en un decimal: texto"
                        .into(),
                ),
            );
        }
        let escala = e.escala_max;
        if escala + e.enteras_max > 38 {
            return (
                Tipo::Mixto,
                Some(format!(
                    "`Decimal128` con {} cifras enteras y {} decimales no cabe en Decimal(38): texto exacto",
                    e.enteras_max, escala
                )),
            );
        }
        return (
            Tipo::Decimal {
                precision: 38,
                escala: escala as u8,
            },
            None,
        );
    }
    if solo(&[Texto]) {
        return (Tipo::Texto, None);
    }
    if solo(&[Logico]) {
        return (Tipo::Logico, None);
    }
    if solo(&[Fecha]) {
        return (Tipo::Instante, None);
    }
    if solo(&[ObjectId]) {
        return (Tipo::ObjectId, None);
    }
    if solo(&[Uuid]) {
        return (Tipo::Uuid, None);
    }
    if solo(&[Binario]) {
        return (Tipo::Binario, None);
    }
    if solo(&[Array, Documento]) {
        return (Tipo::Json, None);
    }
    if clases.len() == 1 {
        return match *clases[0] {
            Timestamp => (
                Tipo::Texto,
                Some("`Timestamp` interno de MongoDB: texto `segundos:incremento`".into()),
            ),
            SinTraducir(n) => (
                Tipo::Json,
                Some(format!(
                    "`{n}` no tiene tipo en OOS: se cita como extended JSON, no se traduce"
                )),
            ),
            _ => (
                Tipo::Mixto,
                Some(format!("tipos no resueltos: {}", nombres())),
            ),
        };
    }
    (
        Tipo::Mixto,
        Some(format!("tipos mezclados ({}): texto", nombres())),
    )
}

/// Una columna del esquema inferido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columna {
    /// La ruta física: `awards.wins`.
    pub ruta: String,
    pub tipo: Tipo,
    /// En cuántos documentos de la muestra estaba (no nulo o nulo).
    pub presentes: u64,
    /// En cuántos tenía un valor que no era `null`. Con `con_valor == documentos`
    /// la columna es obligatoria; `presentes` solo no basta (`{x: null}` está).
    pub con_valor: u64,
}

/// Algo que el catálogo tiene que decir de una columna.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aviso {
    pub ruta: String,
    pub mensaje: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Esquema {
    pub columnas: Vec<Columna>,
    pub documentos: u64,
    pub avisos: Vec<Aviso>,
}

impl Esquema {
    pub fn columna(&self, ruta: &str) -> Option<&Columna> {
        self.columnas.iter().find(|c| c.ruta == ruta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{
        Binary, DateTime, Decimal128, RawDocumentBuf, Timestamp, doc, oid::ObjectId,
    };
    use std::str::FromStr;

    fn esquema_de(docs: Vec<mongodb::bson::Document>, prof: usize) -> Esquema {
        let mut o = Observador::new(prof);
        for d in docs {
            o.observar(&RawDocumentBuf::from_document(&d).unwrap())
                .unwrap();
        }
        o.esquema()
    }

    fn tipo(e: &Esquema, r: &str) -> Tipo {
        e.columna(r)
            .unwrap_or_else(|| panic!("sin columna {r}: {:?}", e.columnas))
            .tipo
            .clone()
    }

    #[test]
    fn los_escalares_de_siempre() {
        let e = esquema_de(
            vec![
                doc! {"_id": 1_i32, "n": 5_i64, "x": 1.5, "s": "a", "b": true, "f": DateTime::now()},
            ],
            3,
        );
        assert_eq!(tipo(&e, "_id"), Tipo::Entero);
        assert_eq!(tipo(&e, "n"), Tipo::Entero);
        assert_eq!(tipo(&e, "x"), Tipo::Real);
        assert_eq!(tipo(&e, "s"), Tipo::Texto);
        assert_eq!(tipo(&e, "b"), Tipo::Logico);
        assert_eq!(tipo(&e, "f"), Tipo::Instante);
        assert!(e.avisos.is_empty(), "{:?}", e.avisos);
        // `_id` primero.
        assert_eq!(e.columnas[0].ruta, "_id");
    }

    /// Enteros y decimales juntos son `Float`; lo demás mezclado, texto con aviso.
    #[test]
    fn lo_que_se_mezcla_se_cede_y_se_dice() {
        let e = esquema_de(
            vec![
                doc! {"a": 1_i32, "b": 1_i32, "c": 1_i32},
                doc! {"a": 2.5, "b": "dos", "c": true},
            ],
            3,
        );
        assert_eq!(tipo(&e, "a"), Tipo::Real);
        assert_eq!(tipo(&e, "b"), Tipo::Mixto);
        assert_eq!(tipo(&e, "c"), Tipo::Mixto);
        let b = e.avisos.iter().find(|a| a.ruta == "b").unwrap();
        assert!(
            b.mensaje.contains("tipos mezclados")
                && b.mensaje.contains("int")
                && b.mensaje.contains("string"),
            "{b:?}"
        );
        // El aviso de `a` no existe: enteros y double es un tipo común.
        assert!(e.avisos.iter().all(|a| a.ruta != "a"));
    }

    /// La muestra de `ore_prueba.polimorfica`: número, texto, null y ausente.
    #[test]
    fn null_y_ausente_no_cuentan_para_el_tipo() {
        let e = esquema_de(
            vec![
                doc! {"edad": 3_i32},
                doc! {"edad": mongodb::bson::Bson::Null},
                doc! {"otro": 1_i32},
                doc! {"edad": 7_i64},
            ],
            3,
        );
        assert_eq!(tipo(&e, "edad"), Tipo::Entero);
        // Una ruta que solo fue null no tiene tipo: texto, y lo dice.
        let e = esquema_de(vec![doc! {"x": mongodb::bson::Bson::Null}], 3);
        assert_eq!(tipo(&e, "x"), Tipo::Texto);
        assert!(e.avisos[0].mensaje.contains("null"));
    }

    #[test]
    fn decimal_con_su_escala_y_los_enteros_que_caben() {
        let d = |t: &str| Decimal128::from_str(t).unwrap();
        let e = esquema_de(
            vec![
                doc! {"p": d("12345678901234567890123456789.123")},
                doc! {"p": d("0.5")},
                doc! {"p": 7_i32},
            ],
            3,
        );
        assert_eq!(
            tipo(&e, "p"),
            Tipo::Decimal {
                precision: 38,
                escala: 3
            }
        );
        // 34 cifras enteras y 5 decimales: 39, no caben.
        let e = esquema_de(
            vec![
                doc! {"p": d("1234567890123456789012345678901234")},
                doc! {"p": d("0.12345")},
            ],
            3,
        );
        assert_eq!(tipo(&e, "p"), Tipo::Mixto);
        assert!(
            e.avisos[0].mensaje.contains("no cabe en Decimal(38)"),
            "{:?}",
            e.avisos
        );
        // NaN: no es un número decimal.
        let e = esquema_de(vec![doc! {"p": d("NaN")}], 3);
        assert_eq!(tipo(&e, "p"), Tipo::Mixto);
    }

    #[test]
    fn objectid_uuid_binario_y_lo_que_no_tiene_tipo() {
        let e = esquema_de(
            vec![doc! {
                "o": ObjectId::new(),
                "u": Binary { subtype: mongodb::bson::spec::BinarySubtype::Uuid, bytes: vec![1; 16] },
                "b": Binary { subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: vec![1, 2] },
                "t": Timestamp { time: 1, increment: 2 },
                "r": mongodb::bson::Regex { pattern: "a".into(), options: "i".into() },
                "k": mongodb::bson::Bson::MinKey,
            }],
            3,
        );
        assert_eq!(tipo(&e, "o"), Tipo::ObjectId);
        assert_eq!(tipo(&e, "u"), Tipo::Uuid);
        assert_eq!(tipo(&e, "b"), Tipo::Binario);
        assert_eq!(tipo(&e, "t"), Tipo::Texto);
        assert_eq!(tipo(&e, "r"), Tipo::Json);
        assert_eq!(tipo(&e, "k"), Tipo::Json);
        let r = e.avisos.iter().find(|a| a.ruta == "r").unwrap();
        assert!(
            r.mensaje.contains("regex") && r.mensaje.contains("no se traduce"),
            "{r:?}"
        );
    }

    /// OOS manda aplanar (v1alpha1 §4.4): una columna por hoja, con la ruta física.
    #[test]
    fn los_documentos_se_aplanan_hasta_la_profundidad() {
        let d = doc! {"_id": 1_i32, "awards": {"wins": 3_i32, "texto": "x"}, "imdb": {"rating": 6.5, "extra": {"a": {"b": 1_i32}}}};
        let e = esquema_de(vec![d.clone()], 3);
        assert_eq!(tipo(&e, "awards.wins"), Tipo::Entero);
        assert_eq!(tipo(&e, "awards.texto"), Tipo::Texto);
        assert_eq!(tipo(&e, "imdb.rating"), Tipo::Real);
        // imdb.extra.a es el nivel 3: se aplana; su hijo `b` está en el nivel 4: no.
        assert_eq!(tipo(&e, "imdb.extra.a"), Tipo::Json);
        assert!(e.columna("awards").is_none(), "el padre no es una columna");
        // Con profundidad 1 no se aplana nada.
        let e = esquema_de(vec![d], 1);
        assert_eq!(tipo(&e, "awards"), Tipo::Json);
        assert!(e.columna("awards.wins").is_none());
    }

    /// Un campo que es documento en unos y array en otros no se parte.
    #[test]
    fn documento_y_otra_cosa_van_enteros_como_json() {
        let e = esquema_de(
            vec![
                doc! {"etiquetas": {"unica": "x"}},
                doc! {"etiquetas": ["a", "b"]},
            ],
            3,
        );
        assert_eq!(tipo(&e, "etiquetas"), Tipo::Json);
        assert!(e.columna("etiquetas.unica").is_none());
        // Un array solo es una columna de JSON, no tablas hijas.
        let e = esquema_de(vec![doc! {"lineas": [{"sku": "A"}, {"sku": "B"}]}], 3);
        assert_eq!(tipo(&e, "lineas"), Tipo::Json);
    }

    /// El orden es determinista: `_id` y luego alfabético.
    #[test]
    fn el_orden_de_las_columnas_es_determinista() {
        let e = esquema_de(vec![doc! {"z": 1_i32, "_id": 1_i32, "a": 1_i32}], 3);
        let rutas: Vec<&str> = e.columnas.iter().map(|c| c.ruta.as_str()).collect();
        assert_eq!(rutas, ["_id", "a", "z"]);
    }

    #[test]
    fn los_nombres_de_oos() {
        assert_eq!(
            Tipo::Decimal {
                precision: 38,
                escala: 3
            }
            .oos(),
            "Decimal<38, 3>"
        );
        assert_eq!(Tipo::Instante.oos(), "DateTimeTz");
        assert_eq!(Tipo::ObjectId.oos(), "String");
        assert_eq!(Tipo::Binario.oos(), "Opaque");
        assert!(Tipo::Entero.empujable() && Tipo::ObjectId.empujable());
        assert!(!Tipo::Mixto.empujable() && !Tipo::Json.empujable() && !Tipo::Binario.empujable());
    }

    /// Lo que la tabla declara manda si el dato lo puede llevar; si no, es un
    /// fallo del esquema y no una conversión a escondidas.
    #[test]
    fn lo_declarado_manda_si_el_dato_lo_lleva() {
        assert_eq!(Tipo::Instante.declarado("Date"), Ok(Tipo::Fecha));
        assert_eq!(Tipo::Instante.declarado("DateTime"), Ok(Tipo::FechaHora));
        assert_eq!(Tipo::Instante.declarado("DateTimeTz"), Ok(Tipo::Instante));
        // La precisión sale de la declaración: `Decimal128` no la lleva.
        let d = Tipo::Decimal {
            precision: 38,
            escala: 2,
        };
        assert_eq!(
            d.declarado("Decimal<12, 2>"),
            Ok(Tipo::Decimal {
                precision: 12,
                escala: 2
            })
        );
        // Texto lo lleva todo, también lo mezclado y lo compuesto.
        assert_eq!(Tipo::Mixto.declarado("String"), Ok(Tipo::Texto));
        assert_eq!(Tipo::Json.declarado("String"), Ok(Tipo::Texto));
        // Un entero puede ser real o decimal.
        assert_eq!(Tipo::Entero.declarado("Float"), Ok(Tipo::Real));
        // Lo que el dato no lleva.
        let e = Tipo::Texto.declarado("Integer").unwrap_err();
        assert!(e.contains("la tabla declara `Integer`"), "{e}");
        assert!(Tipo::Instante.declarado("Time").is_err());
        assert!(Tipo::Instante.declarado("Integer").is_err());
        assert!(Tipo::Entero.declarado("Nada<<").is_err());
    }
}
