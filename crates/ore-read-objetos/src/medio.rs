//! **Qué es un objeto**: por su extensión, y confirmado por sus bytes.
//!
//! Medido en F1: el `Content-Type` que da S3 no es de fiar (la consola subió
//! Parquet y JSONL como `application/x-www-form-urlencoded`), así que no se
//! mira. La extensión propone y los primeros bytes deciden; lo que no se
//! reconoce es `binary` —«todavía no se sabe qué es»—, nunca una conjetura.

/// Lo que un objeto es para el catálogo: filas de un formato, o un medio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Clase {
    Filas(Formato),
    Medio(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Formato {
    Parquet,
    Csv,
    Jsonl,
}

impl Formato {
    pub fn nombre(self) -> &'static str {
        match self {
            Formato::Parquet => "parquet",
            Formato::Csv => "csv",
            Formato::Jsonl => "jsonl",
        }
    }
}

/// La extensión, en minúscula, de la última parte de la clave.
pub fn extension(clave: &str) -> String {
    let base = clave.rsplit('/').next().unwrap_or(clave);
    match base.rsplit_once('.') {
        Some((antes, e)) if !antes.is_empty() => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

// La tabla de extensión a familia vive en `ore-core` (0063 S7d: la usa también la
// promesa `invalidFormats` de una colección, en `ore-cli`): una sola tabla.
use ore_core::medios::FAMILIAS_POR_EXTENSION as FAMILIAS;

/// Lo que la extensión propone. `None`: no dice nada, y deciden los bytes.
pub fn por_extension(ext: &str) -> Option<Clase> {
    use Clase::*;
    match ext {
        "parquet" => return Some(Filas(Formato::Parquet)),
        "csv" | "tsv" => return Some(Filas(Formato::Csv)),
        "jsonl" | "ndjson" => return Some(Filas(Formato::Jsonl)),
        _ => {}
    }
    FAMILIAS
        .iter()
        .find(|(_, exts)| exts.contains(&ext))
        .map(|(m, _)| Medio(m))
}

/// Cuántos bytes del principio se leen para confirmar (el DICOM dice quién es
/// en el 128, el NIfTI en el 344).
pub const CABEZA: usize = 512;

/// Lo que dicen los primeros bytes, cuando dicen algo **específico**. Un
/// contenedor genérico —zip, XML, OLE2, SQLite— lo dice [`generico`].
pub fn por_bytes(b: &[u8]) -> Option<Clase> {
    use Clase::*;
    let empieza = |m: &[u8]| b.starts_with(m);
    let en = |i: usize, m: &[u8]| b.len() >= i + m.len() && &b[i..i + m.len()] == m;
    let x = if empieza(b"PAR1") {
        Filas(Formato::Parquet)
    } else if empieza(b"%PDF") {
        Medio("document")
    } else if empieza(b"\x89PNG")
        || empieza(b"\xff\xd8\xff")
        || empieza(b"GIF8")
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP")
        || (b.len() >= 12
            && &b[4..8] == b"ftyp"
            && matches!(&b[8..12], b"avif" | b"heic" | b"heix" | b"mif1"))
    {
        Medio("image")
    } else if empieza(b"ID3")
        || empieza(b"fLaC")
        || empieza(b"OggS")
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE")
        || (b.len() >= 12 && &b[0..4] == b"FORM" && matches!(&b[8..12], b"AIFF" | b"AIFC"))
        || (b.len() >= 12 && &b[4..8] == b"ftyp" && matches!(&b[8..12], b"M4A " | b"M4B "))
    {
        Medio("audio")
    } else if (b.len() >= 8 && &b[4..8] == b"ftyp")
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"AVI ")
        || empieza(b"\x1a\x45\xdf\xa3")
    {
        Medio("video")
    } else if en(128, b"DICM") || en(344, b"n+1\0") || en(344, b"ni1\0") || empieza(b"MSH|") {
        Medio("medical")
    } else if empieza(b"\x89HDF\r\n\x1a\n")
        || empieza(b"CDF\x01")
        || empieza(b"CDF\x02")
        || empieza(b"\x93NUMPY")
        || empieza(b"SIMPLE  =")
    {
        Medio("scientific")
    } else if empieza(b"glTF") || empieza(b"ISO-10303-21") || empieza(b"AC10") {
        Medio("model3d")
    } else if empieza(b"LASF") || empieza(b"\x00\x00\x27\x0a") {
        Medio("geospatial")
    } else if empieza(b"8BPS") {
        Medio("design")
    } else if empieza(b"\x1f\x8b")
        || empieza(b"7z\xbc\xaf")
        || empieza(b"Rar!")
        || empieza(b"BZh")
        || empieza(b"\xfd7zXZ\0")
        || empieza(b"\x28\xb5\x2f\xfd")
        || en(257, b"ustar")
    {
        Medio("archive")
    } else {
        return None;
    };
    Some(x)
}

/// ⭐ v1alpha29 `01` §1 · **Un contenedor genérico**: lo que es por dentro un
/// zip (DOCX, XLSX, KMZ, USDZ), un XML (KML, GPX, SVG), un OLE2 (DOC, XLS,
/// MSG) o un SQLite (GeoPackage). No desmiente a la extensión: ella dice cuál.
/// Sin extensión que proponga, es lo que es: un zip es `archive`, un XML
/// `structured`.
fn generico(b: &[u8]) -> Option<Clase> {
    let empieza = |m: &[u8]| b.starts_with(m);
    if empieza(b"PK\x03\x04") {
        Some(Clase::Medio("archive"))
    } else if empieza(b"<?xml") {
        Some(Clase::Medio("structured"))
    } else if empieza(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1") || empieza(b"SQLite format 3\0") {
        Some(Clase::Medio("binary"))
    } else {
        None
    }
}

/// Texto: UTF-8 sin bytes de control (salvo los de espacio) en la muestra. Un
/// trozo cortado a mitad de un carácter cuenta como texto.
fn es_texto(b: &[u8]) -> bool {
    if b.is_empty() {
        return false;
    }
    let valido = match std::str::from_utf8(b) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > b.len(),
    };
    valido
        && !b
            .iter()
            .any(|&c| c < 0x20 && !matches!(c, b'\n' | b'\r' | b'\t' | 0x0c))
}

/// La extensión y los bytes, juntos: qué es de verdad, y si discrepan, por qué.
pub fn confirmar(ext: &str, cabeza: &[u8]) -> (Clase, Option<String>) {
    let propuesta = por_extension(ext);
    // `.ts`: MPEG-TS empieza por el byte de sincronía; TypeScript es texto.
    if ext == "ts" && cabeza.first() != Some(&0x47) && es_texto(cabeza) {
        return (Clase::Medio("text"), None);
    }
    let bytes = por_bytes(cabeza);
    match (propuesta, bytes) {
        (Some(p), Some(b)) if p == b => (p, None),
        (Some(p), Some(b)) => (
            b,
            Some(format!(
                "la extensión `.{ext}` dice {p:?} y los bytes dicen {b:?}: se toma lo de los bytes"
            )),
        ),
        // Lo genérico (zip, XML, OLE2) no desmiente: es lo que la extensión
        // dice por dentro. CSV y JSONL no tienen número mágico.
        (Some(p), None) => (p, None),
        (None, Some(b)) => (b, None),
        (None, None) => match generico(cabeza) {
            Some(g) => (g, None),
            // Sin extensión que proponga ni firma: un `{` o un `[` es
            // `structured`; otro texto, `text`; lo demás, no se sabe.
            None if es_texto(cabeza) => {
                let t = String::from_utf8_lossy(cabeza);
                let t = t.trim_start();
                if t.starts_with('{') || t.starts_with('[') {
                    (Clase::Medio("structured"), None)
                } else {
                    (Clase::Medio("text"), None)
                }
            }
            None => (Clase::Medio("binary"), None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lo medido en F1: los nombres de verdad, y los bytes de verdad.
    #[test]
    fn la_extension_propone_y_los_bytes_deciden() {
        assert_eq!(
            extension("Nueva carpeta/fotos/Foto Portada 2026.JPG"),
            "jpg"
        );
        assert_eq!(extension("bloc anucios.txt"), "txt");
        assert_eq!(extension(".oculto"), "");
        assert_eq!(
            confirmar("parquet", b"PAR1\x15\x04").0,
            Clase::Filas(Formato::Parquet)
        );
        assert_eq!(confirmar("pdf", b"%PDF-1.4").0, Clase::Medio("document"));
        assert_eq!(
            confirmar("jpg", b"\xff\xd8\xff\xe0").0,
            Clase::Medio("image")
        );
        assert_eq!(confirmar("zip", b"PK\x03\x04").0, Clase::Medio("archive"));
        assert_eq!(confirmar("docx", b"PK\x03\x04").0, Clase::Medio("document"));
        assert_eq!(
            confirmar("csv", b"id,nombre\n").0,
            Clase::Filas(Formato::Csv)
        );
        assert_eq!(confirmar("", b"\x00\x01rar").0, Clase::Medio("binary"));

        // ── v1alpha29: las familias nuevas ──
        let m = |e: &str, b: &[u8]| confirmar(e, b).0;
        assert_eq!(m("json", b"{\"a\": 1}"), Clase::Medio("structured"));
        assert_eq!(
            m("sha256", b"9f86d081884c7d65  train.csv\n"),
            Clase::Medio("text")
        );
        assert_eq!(
            m("geojson", b"{\"type\": \"FeatureCollection\""),
            Clase::Medio("geospatial")
        );
        // Un KML es un XML por dentro, y un KMZ un zip: la extensión dice cuál.
        assert_eq!(
            m("kml", b"<?xml version=\"1.0\"?><kml"),
            Clase::Medio("geospatial")
        );
        assert_eq!(m("kmz", b"PK\x03\x04"), Clase::Medio("geospatial"));
        assert_eq!(
            m("xls", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"),
            Clase::Medio("spreadsheet")
        );
        assert_eq!(m("stl", b"solid pieza\n"), Clase::Medio("model3d"));
        assert_eq!(m("glb", b"glTF\x02\0\0\0"), Clase::Medio("model3d"));
        let mut dicom = vec![0u8; 128];
        dicom.extend_from_slice(b"DICM");
        assert_eq!(m("dcm", &dicom), Clase::Medio("medical"));
        assert_eq!(
            m("", &dicom),
            Clase::Medio("medical"),
            "sin extensión, los bytes"
        );
        assert_eq!(m("h5", b"\x89HDF\r\n\x1a\n"), Clase::Medio("scientific"));
        assert_eq!(m("psd", b"8BPS\0\x01"), Clase::Medio("design"));
        assert_eq!(m("gz", b"\x1f\x8b\x08"), Clase::Medio("archive"));
        // `.ts`: MPEG-TS por su byte de sincronía; si no, TypeScript.
        assert_eq!(m("ts", b"\x47\x40\x00\x10"), Clase::Medio("video"));
        assert_eq!(m("ts", b"export const a = 1;\n"), Clase::Medio("text"));
        // Sin extensión: un zip es `archive`, un XML `structured`, un `{` también, un texto `text`.
        assert_eq!(m("", b"PK\x03\x04"), Clase::Medio("archive"));
        assert_eq!(m("", b"<?xml version"), Clase::Medio("structured"));
        assert_eq!(m("", b"[{\"a\": 1}]"), Clase::Medio("structured"));
        assert_eq!(m("", b"LICENSE\nMIT\n"), Clase::Medio("text"));
        // Una extensión es de una sola familia.
        let mut vistas = std::collections::BTreeSet::new();
        for (_, exts) in FAMILIAS {
            for e in *exts {
                assert!(vistas.insert(*e), "`{e}` está en dos familias");
            }
        }
        // Y toda familia es de la spec.
        for (f, _) in FAMILIAS {
            assert!(
                ore_core::document::medios_de_objetos(ore_core::document::ApiVersion::V1Alpha29)
                    .contains(f),
                "`{f}` no es de la spec"
            );
        }
        let (c, aviso) = confirmar("csv", b"%PDF-1.7");
        assert_eq!(c, Clase::Medio("document"));
        assert!(aviso.is_some());
    }
}
