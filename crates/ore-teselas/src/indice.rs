//! **El índice de la capa** (ADR 0072, iteración I): del suelo hacia arriba, una capa se sirve de un
//! índice suyo y no de la tabla del cliente, que no se toca nunca. Es una tabla Iceberg del sistema,
//! derivada de la fuente en un snapshot: las mismas filas (I-1), preparadas para pintar (3857, caja,
//! orden Hilbert, grupos de 100 000) y con la identidad de cada una para el clic y para ponerse al día.
//!
//! Este módulo es **el contrato** (I1·1): la receta, la rejilla, la identidad, las columnas y el
//! manifiesto que dice qué se sirve. Construir (I2), servir (I3) y ponerse al día (I4) sólo lo
//! rellenan. Es código puro: no toca el almacén.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use ore_base::json::Json;
use ore_base::parse::Node;

/// **La receta**: todo lo que decide cómo sale un índice. Cambiar algo es otra receta y otra
/// ubicación ([`prefijo`]), nunca reescribir un índice en su sitio.
#[derive(Debug, Clone, PartialEq)]
pub struct Receta {
    pub nombre: &'static str,
    /// El CRS de la geometría del índice: el del mapa (reproyectar por tesela costaba el doble, T1).
    pub crs: &'static str,
    pub grupo_de_filas: usize,
    /// Bits por eje de la rejilla Hilbert (2 × bits ≤ 64: la clave cabe en un `u64`).
    pub bits: u32,
    /// El margen de la rejilla sobre la extensión de los datos, en tanto por ciento de cada lado.
    pub margen_pct: u32,
    /// Se parten las líneas cuya caja pasa de tantas celdas del suelo (I0·c).
    pub trozo_celdas: u32,
    /// Y los polígonos con área / área de su caja por debajo de esto, en milésimas (I0·c: 0,1).
    pub hueco_milesimas: u32,
    /// La profundidad a partir de la cual se compacta.
    pub profundidad_max: u32,
}

/// La receta `i1` (I0, 2026-10-26).
pub const I1: Receta = Receta {
    nombre: "i1",
    crs: "EPSG:3857",
    grupo_de_filas: 100_000,
    bits: 32,
    margen_pct: 10,
    trozo_celdas: 4,
    hueco_milesimas: 100,
    profundidad_max: 5,
};

/// Las recetas que este binario sabe leer: un manifiesto de otra no es de él.
pub const RECETAS: [&Receta; 1] = [&I1];

/// **Dónde vive un índice**: bajo la fuente, por receta. `ubicacion` es la de la tabla en el lago
/// (`gs://…/tabla`), o la raíz del sistema con el id de la fuente para una foreign database.
pub fn prefijo(ubicacion: &str, receta: &Receta) -> String {
    format!(
        "{}/_ore_capa/{}/",
        ubicacion.trim_end_matches('/'),
        receta.nombre
    )
}

/// **La rejilla Hilbert, congelada al nacer el índice**: la extensión de los datos con su margen.
/// Fija sobre el mundo leía 1,5× más en una ciudad (H0·B2); congelada por índice, las claves no
/// cambian entre una puesta al día y la siguiente, y lo que cae fuera se pega al borde.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rejilla {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rejilla {
    /// La rejilla de una extensión `[x0, y0, x1, y1]` con el margen de la receta. Un lado sin
    /// anchura (un solo punto, una línea recta) se abre un metro.
    pub fn congelar(ext: [f64; 4], receta: &Receta) -> Result<Rejilla, String> {
        if ext.iter().any(|v| !v.is_finite()) || ext[0] > ext[2] || ext[1] > ext[3] {
            return Err(format!("la extensión {ext:?} no es una caja"));
        }
        let m = receta.margen_pct as f64 / 100.0;
        let lado = |a: f64, b: f64| {
            let w = (b - a).max(1.0);
            (a - w * m, b + w * m)
        };
        let (x0, x1) = lado(ext[0], ext[2]);
        let (y0, y1) = lado(ext[1], ext[3]);
        Ok(Rejilla { x0, y0, x1, y1 })
    }

    /// La celda de un punto, pegada al borde si cae fuera.
    pub fn celda(&self, x: f64, y: f64, bits: u32) -> (u32, u32) {
        let lado = ((1u64 << bits) - 1) as f64;
        let c = |v: f64, a: f64, b: f64| ((v - a) / (b - a) * lado).clamp(0.0, lado) as u32;
        (c(x, self.x0, self.x1), c(y, self.y0, self.y1))
    }

    /// **La clave de orden** del centro de una caja.
    pub fn clave(&self, caja: [f64; 4], bits: u32) -> u64 {
        let (cx, cy) = self.celda((caja[0] + caja[2]) / 2.0, (caja[1] + caja[3]) / 2.0, bits);
        hilbert(cx, cy, bits)
    }

    fn a_json(&self) -> Json {
        Json::Arr(
            [self.x0, self.y0, self.x1, self.y1]
                .into_iter()
                .map(decimal)
                .collect(),
        )
    }

    fn de_node(n: &Node) -> Result<Rejilla, String> {
        let v: Vec<f64> = n
            .items()
            .iter()
            .map(|x| leer_decimal(x, "rejilla"))
            .collect::<Result<_, _>>()?;
        match v[..] {
            [x0, y0, x1, y1] => Ok(Rejilla { x0, y0, x1, y1 }),
            _ => Err("`rejilla` lleva cuatro números".into()),
        }
    }
}

/// El índice de Hilbert de la celda `(x, y)` en una rejilla de 2^bits × 2^bits (`xy2d`).
pub fn hilbert(x: u32, y: u32, bits: u32) -> u64 {
    debug_assert!((1..=32).contains(&bits));
    let ultimo = (1u64 << bits) - 1;
    let (mut x, mut y) = (x as u64 & ultimo, y as u64 & ultimo);
    let mut d = 0u64;
    let mut s = 1u64 << (bits - 1);
    while s > 0 {
        let rx = u64::from(x & s > 0);
        let ry = u64::from(y & s > 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = ultimo - x;
                y = ultimo - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s >>= 1;
    }
    d
}

/// **Cómo se nombra una fila de la fuente** en este índice (I0·a). Lo decide la fuente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TipoDeIdentidad {
    /// En el lago: `(fichero, fila)`, la de Iceberg. Un borrado por posición la nombra tal cual, y
    /// un `merge` o un `overwrite` retiran ficheros enteros: ponerse al día es exacto sin clave.
    Posicion,
    /// Fuera del lago, con clave (PK, `_id`, clave de partición), por sus columnas y en su orden.
    Clave { columnas: Vec<String> },
    /// Sin nada estable: la posición dentro del snapshot. Al cambiar la fuente, se rehace.
    Instantanea,
}

impl TipoDeIdentidad {
    fn a_json(&self) -> Json {
        match self {
            TipoDeIdentidad::Posicion => Json::obj([("tipo", Json::s("posicion"))]),
            TipoDeIdentidad::Instantanea => Json::obj([("tipo", Json::s("instantanea"))]),
            TipoDeIdentidad::Clave { columnas } => Json::obj([
                ("tipo", Json::s("clave")),
                (
                    "columnas",
                    Json::Arr(columnas.iter().map(Json::s).collect()),
                ),
            ]),
        }
    }

    fn de_node(n: &Node) -> Result<TipoDeIdentidad, String> {
        match texto(n, "tipo")?.as_str() {
            "posicion" => Ok(TipoDeIdentidad::Posicion),
            "instantanea" => Ok(TipoDeIdentidad::Instantanea),
            "clave" => {
                let columnas: Vec<String> = n
                    .get("columnas")
                    .map(|(_, v)| v.items())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect();
                if columnas.is_empty() {
                    return Err("una identidad `clave` necesita sus columnas".into());
                }
                Ok(TipoDeIdentidad::Clave { columnas })
            }
            o => Err(format!(
                "identidad `{o}`: se conocen posicion, clave e instantanea"
            )),
        }
    }
}

/// Los bits de la fila en el id de una [`Identidad::Posicion`]; el resto, el ordinal del fichero.
const BITS_FILA: u32 = 40;

/// **La identidad de una fila**, la que el índice guarda y el clic usa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identidad {
    /// `fichero` es el ordinal en la tabla de ficheros del índice.
    Posicion {
        fichero: u32,
        fila: u64,
    },
    /// Los valores de la clave en forma canónica, en el orden de sus columnas.
    Clave(Vec<String>),
    Instantanea {
        fila: u64,
    },
}

impl Identidad {
    /// **El id del elemento en la tesela** (`u64`, el de MVT). Exacto y reversible para una
    /// posición (`ordinal << 40 | fila`: hasta 16 M ficheros de 10¹² filas); para una clave, los
    /// 64 primeros bits del SHA-256 de su forma canónica (con 20 000 M filas se esperan ~10
    /// colisiones: sólo tocan el resaltado; el clic va por la clave guardada).
    pub fn id(&self) -> Result<u64, String> {
        match self {
            Identidad::Posicion { fichero, fila } => {
                if *fichero >= 1 << (64 - BITS_FILA) || *fila >= 1 << BITS_FILA {
                    return Err(format!(
                        "la posición ({fichero}, {fila}) no cabe en un id: hasta 2^24 ficheros de 2^40 filas"
                    ));
                }
                Ok(((*fichero as u64) << BITS_FILA) | fila)
            }
            Identidad::Clave(_) => {
                let h =
                    ore_base::digest::sha256(self.clave_canonica().unwrap_or_default().as_bytes());
                Ok(u64::from_be_bytes(h[..8].try_into().expect("8 bytes")))
            }
            Identidad::Instantanea { fila } => Ok(*fila),
        }
    }

    /// La posición de un id de [`Identidad::Posicion`].
    pub fn de_id_de_posicion(id: u64) -> Identidad {
        Identidad::Posicion {
            fichero: (id >> BITS_FILA) as u32,
            fila: id & ((1 << BITS_FILA) - 1),
        }
    }

    /// **La forma canónica de una clave** (`_ore_clave`): el JSON canónico de sus valores, que es
    /// legible y no confunde `["a,b"]` con `["a", "b"]`.
    pub fn clave_canonica(&self) -> Option<String> {
        match self {
            Identidad::Clave(v) => Some(Json::Arr(v.iter().map(Json::s).collect()).jcs()),
            _ => None,
        }
    }
}

/// Las columnas que el índice añade, con el prefijo que una columna de la fuente no puede llevar.
pub const PREFIJO: &str = "_ore_";
pub const COL_ID: &str = "_ore_id";
pub const COL_FICHERO: &str = "_ore_fichero";
pub const COL_FILA: &str = "_ore_fila";
pub const COL_CLAVE: &str = "_ore_clave";
pub const COL_TROZO: &str = "_ore_trozo";
pub const COL_GEOM: &str = "geom";
pub const COL_CAJA: &str = "geom_bbox";

/// **El esquema del índice**: las columnas fijas de la receta y, detrás, las que trae de la fuente
/// (las de la capa, las del filtro de quien mira y la de tiempo) con su nombre en la fuente.
pub fn esquema(receta: &Receta, de_la_fuente: &[Field]) -> Result<Schema, String> {
    let mut campos = vec![
        Field::new(COL_ID, DataType::UInt64, false),
        Field::new(COL_FICHERO, DataType::UInt32, true),
        Field::new(COL_FILA, DataType::UInt64, true),
        Field::new(COL_CLAVE, DataType::Utf8, true),
        Field::new(COL_TROZO, DataType::UInt16, false),
        ore_store::carga::campo_del_oos(COL_GEOM, &format!("Geometry<Any, {}>", receta.crs)),
        Field::new(
            COL_CAJA,
            DataType::Struct(
                ["xmin", "ymin", "xmax", "ymax"]
                    .iter()
                    .map(|n| Field::new(*n, DataType::Float64, true))
                    .collect(),
            ),
            true,
        ),
    ];
    for f in de_la_fuente {
        let n = f.name();
        if n.starts_with(PREFIJO) || n == COL_GEOM || n == COL_CAJA {
            return Err(format!(
                "la columna `{n}` de la fuente choca con las del índice (`{PREFIJO}*`, `{COL_GEOM}`, `{COL_CAJA}`)"
            ));
        }
        if campos.iter().any(|c| c.name() == n) {
            return Err(format!("la columna `{n}` llega dos veces"));
        }
        campos.push(f.clone());
    }
    Ok(Schema::new(
        campos.into_iter().map(Arc::new).collect::<Vec<_>>(),
    ))
}

/// De dónde sale un índice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fuente {
    /// `lago` o `foranea`.
    pub tipo: String,
    /// El nombre por el que se pide la capa (`<namespace>.<tabla>`).
    pub nombre: String,
    /// La de la tabla en el lago, o la raíz del sistema con el id de la fuente ([`prefijo`]).
    pub ubicacion: String,
}

/// La pirámide emparejada con un índice: la del mismo `snapshot_fuente` (I-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiramideDeLaCapa {
    pub clave: String,
    pub snapshot_fuente: String,
}

/// La versión del formato del manifiesto (no la de la receta: ésa va en `receta`).
pub const FORMATO: u32 = 1;

/// **El manifiesto de la capa**: la única verdad sobre qué se sirve. Cambiar de snapshot es cambiar
/// de manifiesto de golpe (I1·3).
#[derive(Debug, Clone, PartialEq)]
pub struct Manifiesto {
    pub receta: String,
    pub generacion: u64,
    pub fuente: Fuente,
    /// El snapshot de la fuente que refleja: su `metadata_location` en el lago, su versión fuera.
    pub snapshot_fuente: String,
    pub identidad: TipoDeIdentidad,
    /// El `metadata_location` de la tabla del índice.
    pub indice: String,
    /// La clave del objeto con la tabla de ordinales de ficheros (sólo con [`TipoDeIdentidad::Posicion`]).
    pub ficheros: Option<String>,
    pub piramide: Option<PiramideDeLaCapa>,
    pub suelo: u32,
    pub rejilla: Rejilla,
    pub profundidad: f64,
    pub filas: u64,
    /// La suma de los ids con desbordamiento: vale para cualquier identidad y con duplicados (I-1).
    pub suma_ids: u64,
    pub creado_ms: i64,
}

impl Manifiesto {
    pub fn a_json(&self) -> Json {
        let mut m = Json::obj([
            ("formato", Json::Int(FORMATO as i64)),
            ("receta", Json::s(&self.receta)),
            ("generacion", Json::s(self.generacion.to_string())),
            (
                "fuente",
                Json::obj([
                    ("tipo", Json::s(&self.fuente.tipo)),
                    ("nombre", Json::s(&self.fuente.nombre)),
                    ("ubicacion", Json::s(&self.fuente.ubicacion)),
                ]),
            ),
            ("snapshot_fuente", Json::s(&self.snapshot_fuente)),
            ("identidad", self.identidad.a_json()),
            ("indice", Json::s(&self.indice)),
            ("suelo", Json::Int(self.suelo as i64)),
            ("rejilla", self.rejilla.a_json()),
            ("profundidad", decimal(self.profundidad)),
            ("filas", Json::s(self.filas.to_string())),
            ("suma_ids", Json::s(self.suma_ids.to_string())),
            ("creado_ms", Json::Int(self.creado_ms)),
        ]);
        if let Json::Obj(o) = &mut m {
            if let Some(f) = &self.ficheros {
                o.insert("ficheros".into(), Json::s(f));
            }
            if let Some(p) = &self.piramide {
                o.insert(
                    "piramide".into(),
                    Json::obj([
                        ("clave", Json::s(&p.clave)),
                        ("snapshot_fuente", Json::s(&p.snapshot_fuente)),
                    ]),
                );
            }
        }
        m
    }

    /// El texto que se guarda: JSON indentado, que lo lea una persona.
    pub fn texto(&self) -> String {
        self.a_json().pretty() + "\n"
    }

    pub fn de_texto(cuerpo: &str) -> Result<Manifiesto, String> {
        let n = ore_base::parse::parse(cuerpo)
            .map_err(|e| format!("el manifiesto no es JSON: {e:?}"))?;
        let formato = entero(&n, "formato")?;
        if formato != FORMATO as u64 {
            return Err(format!(
                "manifiesto de formato {formato}: este binario lee el {FORMATO}"
            ));
        }
        let receta = texto_de(&n, "receta")?;
        if !RECETAS.iter().any(|r| r.nombre == receta) {
            return Err(format!("receta `{receta}`: este binario no la conoce"));
        }
        let sub = |k: &str| {
            n.get(k)
                .map(|(_, v)| v)
                .ok_or_else(|| format!("al manifiesto le falta `{k}`"))
        };
        let f = sub("fuente")?;
        let piramide = match n.get("piramide").map(|(_, v)| v) {
            None => None,
            Some(p) => Some(PiramideDeLaCapa {
                clave: texto(p, "clave")?,
                snapshot_fuente: texto(p, "snapshot_fuente")?,
            }),
        };
        Ok(Manifiesto {
            receta,
            generacion: entero(&n, "generacion")?,
            fuente: Fuente {
                tipo: texto(f, "tipo")?,
                nombre: texto(f, "nombre")?,
                ubicacion: texto(f, "ubicacion")?,
            },
            snapshot_fuente: texto_de(&n, "snapshot_fuente")?,
            identidad: TipoDeIdentidad::de_node(sub("identidad")?)?,
            indice: texto_de(&n, "indice")?,
            ficheros: n
                .get("ficheros")
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string),
            piramide,
            suelo: entero(&n, "suelo")? as u32,
            rejilla: Rejilla::de_node(sub("rejilla")?)?,
            profundidad: leer_decimal(sub("profundidad")?, "profundidad")?,
            filas: entero(&n, "filas")?,
            suma_ids: entero(&n, "suma_ids")?,
            creado_ms: entero(&n, "creado_ms")? as i64,
        })
    }
}

/// Un decimal como cadena (la forma canónica no lleva números con decimales, `OOS6003`), con la
/// escritura más corta que vuelve al mismo `f64`.
fn decimal(v: f64) -> Json {
    Json::s(format!("{v}"))
}

fn leer_decimal(n: &Node, que: &str) -> Result<f64, String> {
    n.as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("`{que}` no es un número"))
}

fn texto(n: &Node, k: &str) -> Result<String, String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("falta `{k}`"))
}

fn texto_de(n: &Node, k: &str) -> Result<String, String> {
    texto(n, k).map_err(|e| format!("al manifiesto le {e}"))
}

fn entero(n: &Node, k: &str) -> Result<u64, String> {
    texto_de(n, k)?
        .parse()
        .map_err(|_| format!("`{k}` no es un entero"))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn manifiesto() -> Manifiesto {
        Manifiesto {
            receta: "i1".into(),
            generacion: 7,
            fuente: Fuente {
                tipo: "lago".into(),
                nombre: "query.default.trip_30m".into(),
                ubicacion: "gs://cubo/ore/v2/t/trip_30m".into(),
            },
            snapshot_fuente: "gs://cubo/ore/v2/t/trip_30m/metadata/00003-abc.metadata.json".into(),
            identidad: TipoDeIdentidad::Posicion,
            indice:
                "gs://cubo/ore/v2/t/trip_30m/_ore_capa/i1/indice/metadata/00001-x.metadata.json"
                    .into(),
            ficheros: Some("ore/v2/t/trip_30m/_ore_capa/i1/ficheros-7.json".into()),
            piramide: Some(PiramideDeLaCapa {
                clave: "ore/v2/t/trip_30m/_ore_piramides/00003-abc-r1.pmtiles".into(),
                snapshot_fuente: "gs://cubo/ore/v2/t/trip_30m/metadata/00003-abc.metadata.json"
                    .into(),
            }),
            suelo: 13,
            rejilla: Rejilla {
                x0: -8_266_000.125,
                y0: 4_940_000.5,
                x1: -8_210_000.0,
                y1: 4_990_000.75,
            },
            profundidad: 2.4,
            filas: 30_000_000,
            suma_ids: u64::MAX - 12,
            creado_ms: 1_793_000_000_000,
        }
    }

    #[test]
    fn el_manifiesto_va_y_vuelve_igual() {
        let m = manifiesto();
        let t = m.texto();
        let de = Manifiesto::de_texto(&t).unwrap();
        assert_eq!(de, m);
        assert_eq!(de.texto(), t, "byte a byte");
        // sin las opcionales, también
        let m2 = Manifiesto {
            ficheros: None,
            piramide: None,
            identidad: TipoDeIdentidad::Clave {
                columnas: vec!["pais".into(), "id".into()],
            },
            ..manifiesto()
        };
        assert_eq!(Manifiesto::de_texto(&m2.texto()).unwrap(), m2);
    }

    #[test]
    fn un_manifiesto_ajeno_no_se_lee() {
        let t = manifiesto().texto();
        let e = Manifiesto::de_texto(&t.replace("\"i1\"", "\"i9\"")).unwrap_err();
        assert!(e.contains("i9"), "{e}");
        let e = Manifiesto::de_texto(&t.replace("\"formato\": 1", "\"formato\": 2")).unwrap_err();
        assert!(e.contains("formato 2"), "{e}");
        assert!(Manifiesto::de_texto(&t.replace("\"indice\"", "\"otro\"")).is_err());
    }

    #[test]
    fn el_id_de_una_posicion_es_exacto_y_reversible() {
        for (fichero, fila) in [
            (0, 0),
            (1, 0),
            (0, 1),
            (12_345, 99_999),
            ((1 << 24) - 1, (1 << 40) - 1),
        ] {
            let p = Identidad::Posicion { fichero, fila };
            let id = p.id().unwrap();
            assert_eq!(Identidad::de_id_de_posicion(id), p);
        }
        assert_ne!(
            Identidad::Posicion {
                fichero: 1,
                fila: 0
            }
            .id()
            .unwrap(),
            Identidad::Posicion {
                fichero: 0,
                fila: 1
            }
            .id()
            .unwrap()
        );
        assert!(
            Identidad::Posicion {
                fichero: 1 << 24,
                fila: 0
            }
            .id()
            .is_err()
        );
        assert!(
            Identidad::Posicion {
                fichero: 0,
                fila: 1 << 40
            }
            .id()
            .is_err()
        );
    }

    #[test]
    fn el_id_de_una_clave_es_estable_y_no_confunde_valores() {
        let c = |v: &[&str]| Identidad::Clave(v.iter().map(|s| s.to_string()).collect());
        // estable: el mismo valor de hoy y de siempre (si cambia, cambian todos los ids publicados)
        assert_eq!(c(&["42"]).id().unwrap(), c(&["42"]).id().unwrap());
        assert_eq!(c(&["42"]).clave_canonica().unwrap(), r#"["42"]"#);
        assert_eq!(
            c(&["42"]).id().unwrap(),
            0xd646_f2fa_bdd1_3a7e,
            "SHA-256 de [\"42\"], fijado"
        );
        // la forma canónica separa lo que una concatenación juntaría
        assert_ne!(c(&["a,b"]).id().unwrap(), c(&["a", "b"]).id().unwrap());
        assert_ne!(c(&["ab", ""]).id().unwrap(), c(&["a", "b"]).id().unwrap());
        // el orden de la clave es el de sus columnas, y cuenta
        assert_ne!(c(&["1", "2"]).id().unwrap(), c(&["2", "1"]).id().unwrap());
        assert_eq!(Identidad::Instantanea { fila: 9 }.id().unwrap(), 9);
    }

    #[test]
    fn dos_recetas_dos_ubicaciones() {
        let otra = Receta { nombre: "i2", ..I1 };
        let a = prefijo("gs://cubo/t/", &I1);
        assert_eq!(a, "gs://cubo/t/_ore_capa/i1/");
        assert_ne!(a, prefijo("gs://cubo/t", &otra));
        assert!(!prefijo("gs://cubo/t", &otra).starts_with(&a));
    }

    #[test]
    fn la_curva_llena_cada_cuadrante_antes_de_salir() {
        // las 16 celdas del bloque de 4×4 de la esquina son los 16 primeros índices: la curva no
        // sale de un cuadrante hasta llenarlo (lo que hace que lo cercano quede junto)
        let mut v: Vec<u64> = (0..4)
            .flat_map(|x| (0..4).map(move |y| hilbert(x, y, 32)))
            .collect();
        v.sort();
        assert_eq!(v, (0..16).collect::<Vec<_>>());
        assert_eq!(hilbert(0, 0, 32), 0);
        // la esquina opuesta en x es la última celda de la curva
        assert_eq!(hilbert(u32::MAX, 0, 32), u64::MAX);
        // y vecinos en la curva son vecinos en el plano
        assert!(hilbert(1, 0, 32) == 1 || hilbert(0, 1, 32) == 1);
    }

    #[test]
    fn la_rejilla_congelada_pega_lo_de_fuera_al_borde() {
        let r = Rejilla::congelar([0.0, 0.0, 100.0, 50.0], &I1).unwrap();
        assert_eq!((r.x0, r.x1, r.y0, r.y1), (-10.0, 110.0, -5.0, 55.0));
        assert_eq!(r.celda(-1e9, -1e9, 32), (0, 0));
        assert_eq!(r.celda(1e9, 1e9, 32), (u32::MAX, u32::MAX));
        // un solo punto: un lado de un metro
        let p = Rejilla::congelar([5.0, 5.0, 5.0, 5.0], &I1).unwrap();
        assert!(p.x1 > p.x0 && p.y1 > p.y0);
        assert!(Rejilla::congelar([1.0, 0.0, 0.0, 1.0], &I1).is_err());
        assert!(Rejilla::congelar([f64::NAN, 0.0, 1.0, 1.0], &I1).is_err());
        // lo cercano, cerca: dos puntos a un metro tienen claves más próximas que dos lejanos
        let k = |x: f64| r.clave([x, 20.0, x, 20.0], 32);
        // (fuera de las fronteras grandes de la curva: en el centro, x = 50, salta de cuadrante)
        assert!(k(27.3).abs_diff(k(27.301)) < k(27.3).abs_diff(k(95.0)));
    }

    #[test]
    fn el_esquema_lleva_lo_fijo_y_lo_de_la_fuente() {
        let s = esquema(
            &I1,
            &[
                Field::new("tarifa", DataType::Float64, true),
                Field::new("cuando", DataType::Int64, true),
            ],
        )
        .unwrap();
        let nombres: Vec<&str> = s.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            nombres,
            [
                COL_ID,
                COL_FICHERO,
                COL_FILA,
                COL_CLAVE,
                COL_TROZO,
                COL_GEOM,
                COL_CAJA,
                "tarifa",
                "cuando"
            ]
        );
        // la geometría lleva la marca geo con su CRS: el lago la escribe con el escritor geo
        let g = s.field_with_name(COL_GEOM).unwrap();
        assert_eq!(
            g.metadata().get("ARROW:extension:name").map(String::as_str),
            Some("geoarrow.wkb")
        );
        assert!(g.metadata()["ARROW:extension:metadata"].contains("3857"));
        for choca in ["_ore_x", "geom", "geom_bbox"] {
            assert!(
                esquema(&I1, &[Field::new(choca, DataType::Utf8, true)]).is_err(),
                "{choca}"
            );
        }
        let dos = [
            Field::new("a", DataType::Utf8, true),
            Field::new("a", DataType::Utf8, true),
        ];
        assert!(esquema(&I1, &dos).is_err());
    }
}
