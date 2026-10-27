//! **Cómo se sirve un ítem de una colección** (0046 E9): lo que va dentro de la
//! firma de su URL. Lo usan `ore collections --servir` y `ore-medios` (0049
//! B2), y vive aquí para que los dos digan lo mismo.

/// Lo que un navegador enseña **sin ejecutar nada**: se sirve `inline`. Todo lo
/// demás —HTML, SVG, XML, texto que un navegador podría olfatear— como
/// descarga, aunque el tipo lo diga el origen (0046 E9, lo que los grandes
/// hacen con el contenido de sus usuarios: nunca activo en línea).
pub const EN_LINEA: &[&str] = &[
    "application/pdf",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
    "video/mp4",
    "video/webm",
    "video/quicktime",
    "audio/mpeg",
    "audio/mp4",
    "audio/ogg",
    "audio/wav",
    "audio/webm",
];

/// **El tipo por los bytes** (0049 B4b·1): lo que dicen los primeros bytes de
/// un fichero, cuando dicen algo, como `Content-Type`. Es lo que la celda
/// guarda de un ítem escrito: el tipo que el código declara puede mentir (y
/// `EN_LINEA` decide por el tipo cómo se sirve), los bytes no. Los mismos
/// números mágicos que `ore-read-s3` mira para clasificar un origen. Un zip
/// puede ser un `docx`: aquí es un zip, y la extensión lo afina.
pub fn tipo_por_bytes(b: &[u8]) -> Option<&'static str> {
    let empieza = |m: &[u8]| b.starts_with(m);
    let riff = |t: &[u8]| b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == t;
    Some(if empieza(b"%PDF") {
        "application/pdf"
    } else if empieza(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if empieza(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if empieza(b"GIF87a") || empieza(b"GIF89a") {
        "image/gif"
    } else if riff(b"WEBP") {
        "image/webp"
    } else if empieza(b"II*\0") || empieza(b"MM\0*") {
        "image/tiff"
    } else if empieza(b"BM") && b.len() >= 14 {
        "image/bmp"
    } else if b.len() >= 12 && &b[4..8] == b"ftyp" {
        match &b[8..12] {
            b"avif" | b"avis" => "image/avif",
            b"heic" | b"heix" | b"mif1" => "image/heic",
            b"qt  " => "video/quicktime",
            b"M4A " => "audio/mp4",
            _ => "video/mp4",
        }
    } else if empieza(b"\x1a\x45\xdf\xa3") {
        "video/webm"
    } else if riff(b"WAVE") {
        "audio/wav"
    } else if empieza(b"ID3") || (b.len() >= 2 && b[0] == 0xff && b[1] & 0xe0 == 0xe0) {
        "audio/mpeg"
    } else if empieza(b"fLaC") {
        "audio/flac"
    } else if empieza(b"OggS") {
        "audio/ogg"
    } else if empieza(b"PAR1") {
        "application/vnd.apache.parquet"
    } else if empieza(b"PK\x03\x04") {
        "application/zip"
    } else if empieza(b"\x1f\x8b") {
        "application/gzip"
    } else if empieza(b"7z\xbc\xaf") {
        "application/x-7z-compressed"
    // ── v1alpha29 · las familias nuevas: todas se sirven como descarga ──────
    } else if b.len() >= 132 && &b[128..132] == b"DICM" {
        "application/dicom"
    } else if empieza(b"\x89HDF\r\n\x1a\n") {
        "application/x-hdf5"
    } else if empieza(b"CDF\x01") || empieza(b"CDF\x02") {
        "application/x-netcdf"
    } else if empieza(b"\x93NUMPY") {
        "application/x-npy"
    } else if empieza(b"SIMPLE  =") {
        "application/fits"
    } else if empieza(b"glTF") {
        "model/gltf-binary"
    } else if empieza(b"ISO-10303-21") {
        "model/step"
    } else if empieza(b"AC10") {
        "image/vnd.dwg"
    } else if empieza(b"LASF") {
        "application/vnd.las"
    } else if empieza(b"8BPS") {
        "image/vnd.adobe.photoshop"
    } else if empieza(b"%!PS-Adobe") {
        "application/postscript"
    } else if empieza(b"Rar!") {
        "application/vnd.rar"
    } else if empieza(b"BZh") {
        "application/x-bzip2"
    } else if empieza(b"\xfd7zXZ\0") {
        "application/x-xz"
    } else if empieza(b"\x28\xb5\x2f\xfd") {
        "application/zstd"
    } else if empieza(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1") {
        "application/x-ole-storage"
    } else if empieza(b"<?xml") {
        "application/xml"
    } else {
        return None;
    })
}

/// `Content-Disposition` de un ítem: el modo y su nombre, en ASCII (`filename`)
/// y en UTF-8 (`filename*`, RFC 6266), sin nada que rompa la cabecera.
pub fn disposicion(tipo: &str, nombre: &str) -> (&'static str, String) {
    let modo = if EN_LINEA.contains(&tipo) {
        "inline"
    } else {
        "attachment"
    };
    let ascii: String = nombre
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let utf8: String = nombre
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    (
        modo,
        format!("{modo}; filename=\"{ascii}\"; filename*=UTF-8''{utf8}"),
    )
}

/// ⭐ OOS v1alpha29 (`01-las-familias` §1) · **La familia que propone cada
/// extensión.** Una extensión es de una sola familia; lo que no está aquí no
/// propone nada, y deciden los bytes.
pub const FAMILIAS_POR_EXTENSION: &[(&str, &[&str])] = &[
    (
        "document",
        &[
            "pdf", "doc", "docx", "ppt", "pptx", "odt", "odp", "rtf", "txt", "md", "html", "htm",
            "epub", "xps", "pages", "key",
        ],
    ),
    (
        "image",
        &[
            "jpg", "jpeg", "png", "gif", "tif", "tiff", "webp", "bmp", "heic", "heif", "avif",
            "svg", "ico", "jp2", "dng", "cr2", "nef", "arw",
        ],
    ),
    (
        "audio",
        &[
            "wav", "mp3", "flac", "ogg", "oga", "opus", "m4a", "aac", "aiff", "aif", "wma", "sph",
        ],
    ),
    (
        "video",
        &[
            "mp4", "m4v", "mov", "avi", "mkv", "webm", "mpg", "mpeg", "ts", "m2ts", "wmv", "flv",
            "3gp",
        ],
    ),
    (
        "spreadsheet",
        &["xlsx", "xlsm", "xlsb", "xls", "ods", "numbers"],
    ),
    ("email", &["eml", "msg", "mbox"]),
    ("structured", &["json", "xml", "yaml", "yml", "toml"]),
    (
        "text",
        &[
            "log", "sha256", "sha1", "md5", "ini", "conf", "cfg", "py", "js", "java", "scala",
            "kt", "rs", "go", "c", "h", "cpp", "cs", "rb", "php", "sh", "ps1", "sql", "r", "ipynb",
        ],
    ),
    (
        "geospatial",
        &[
            "geojson", "kml", "kmz", "gpx", "gml", "gpkg", "shp", "shx", "dbf", "prj", "cpg",
            "las", "laz",
        ],
    ),
    (
        "model3d",
        &[
            "stl", "obj", "ply", "glb", "gltf", "fbx", "3ds", "dwg", "dxf", "step", "stp", "iges",
            "igs", "usdz",
        ],
    ),
    ("medical", &["dcm", "dicom", "nii", "hl7", "edf"]),
    (
        "scientific",
        &[
            "h5", "hdf5", "he5", "nc", "nc4", "netcdf", "npy", "npz", "fits", "fit", "mat",
        ],
    ),
    (
        "design",
        &["psd", "ai", "eps", "indd", "sketch", "fig", "xd"],
    ),
    (
        "archive",
        &["zip", "tar", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar"],
    ),
];

/// La familia de medios de una extensión (en minúscula, sin el punto), o `None`.
pub fn familia_de_extension(ext: &str) -> Option<&'static str> {
    FAMILIAS_POR_EXTENSION
        .iter()
        .find(|(_, exts)| exts.contains(&ext))
        .map(|(f, _)| *f)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_tipo_sale_de_los_bytes() {
        assert_eq!(tipo_por_bytes(b"%PDF-1.7\n"), Some("application/pdf"));
        assert_eq!(tipo_por_bytes(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(tipo_por_bytes(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(tipo_por_bytes(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(tipo_por_bytes(b"\0\0\0\x18ftypmp42"), Some("video/mp4"));
        assert_eq!(tipo_por_bytes(b"\0\0\0\x18ftypavif"), Some("image/avif"));
        assert_eq!(tipo_por_bytes(b"PK\x03\x04"), Some("application/zip"));
        // v1alpha29: las familias nuevas.
        let mut dicom = vec![0u8; 128];
        dicom.extend_from_slice(b"DICM\x02\0");
        assert_eq!(tipo_por_bytes(&dicom), Some("application/dicom"));
        assert_eq!(
            tipo_por_bytes(b"\x89HDF\r\n\x1a\n\0"),
            Some("application/x-hdf5")
        );
        assert_eq!(tipo_por_bytes(b"glTF\x02\0\0\0"), Some("model/gltf-binary"));
        assert_eq!(
            tipo_por_bytes(b"8BPS\0\x01"),
            Some("image/vnd.adobe.photoshop")
        );
        assert_eq!(tipo_por_bytes(b"<?xml version"), Some("application/xml"));
        assert!(
            !EN_LINEA.contains(&"application/xml"),
            "lo que un navegador podría ejecutar, como descarga"
        );
        assert_eq!(tipo_por_bytes(b"<html>"), None, "lo activo no se adivina");
        assert_eq!(tipo_por_bytes(b""), None);
    }
}
