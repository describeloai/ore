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

/// Lo que la extensión propone. `None`: no dice nada, y deciden los bytes.
pub fn por_extension(ext: &str) -> Option<Clase> {
    use Clase::*;
    Some(match ext {
        "parquet" => Filas(Formato::Parquet),
        "csv" | "tsv" => Filas(Formato::Csv),
        "jsonl" | "ndjson" => Filas(Formato::Jsonl),
        "pdf" | "doc" | "docx" | "ppt" | "pptx" | "odt" | "odp" | "rtf" | "txt" | "md" => {
            Medio("document")
        }
        "jpg" | "jpeg" | "png" | "gif" | "tif" | "tiff" | "webp" | "bmp" | "heic" | "dcm" => {
            Medio("image")
        }
        "wav" | "mp3" | "flac" | "ogg" | "m4a" | "aac" => Medio("audio"),
        "mp4" | "mov" | "avi" | "mkv" | "webm" => Medio("video"),
        "xlsx" | "xls" | "ods" => Medio("spreadsheet"),
        "eml" | "msg" => Medio("email"),
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" => Medio("archive"),
        _ => return None,
    })
}

/// Lo que dicen los primeros bytes, cuando dicen algo.
pub fn por_bytes(b: &[u8]) -> Option<Clase> {
    use Clase::*;
    let empieza = |m: &[u8]| b.starts_with(m);
    let x = if empieza(b"PAR1") {
        Filas(Formato::Parquet)
    } else if empieza(b"%PDF") {
        Medio("document")
    } else if empieza(b"\x89PNG")
        || empieza(b"\xff\xd8\xff")
        || empieza(b"GIF8")
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP")
    {
        Medio("image")
    } else if empieza(b"ID3")
        || empieza(b"fLaC")
        || empieza(b"OggS")
        || (b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE")
    {
        Medio("audio")
    } else if b.len() >= 8 && &b[4..8] == b"ftyp" {
        Medio("video")
    } else if empieza(b"PK\x03\x04") {
        // Un zip… o un docx, un xlsx: los de Office son zips. Lo decide la
        // extensión (`confirmar`); aquí solo se sabe que es un contenedor.
        Medio("archive")
    } else if empieza(b"\x1f\x8b") || empieza(b"7z\xbc\xaf") || empieza(b"Rar!") {
        Medio("archive")
    } else {
        return None;
    };
    Some(x)
}

/// La extensión y los bytes, juntos: qué es de verdad, y si discrepan, por qué.
pub fn confirmar(ext: &str, cabeza: &[u8]) -> (Clase, Option<String>) {
    let propuesta = por_extension(ext);
    let bytes = por_bytes(cabeza);
    match (propuesta, bytes) {
        // Los de Office son zips por dentro: la extensión manda.
        (Some(Clase::Medio(m)), Some(Clase::Medio("archive")))
            if matches!(ext, "docx" | "pptx" | "xlsx" | "odt" | "odp" | "ods") =>
        {
            (Clase::Medio(m), None)
        }
        (Some(p), Some(b)) if p == b => (p, None),
        (Some(p), Some(b)) => (
            b,
            Some(format!(
                "la extensión `.{ext}` dice {p:?} y los bytes dicen {b:?}: se toma lo de los bytes"
            )),
        ),
        // CSV y JSONL no tienen número mágico: la extensión y que empiece por
        // texto. Un `.csv` que empieza por un PDF lo habría cogido arriba.
        (Some(p), None) => (p, None),
        (None, Some(b)) => (b, None),
        (None, None) => (Clase::Medio("binary"), None),
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
        let (c, aviso) = confirmar("csv", b"%PDF-1.7");
        assert_eq!(c, Clase::Medio("document"));
        assert!(aviso.is_some());
    }
}
