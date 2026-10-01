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
