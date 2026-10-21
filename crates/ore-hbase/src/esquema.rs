//! **La forma de una tabla de HBase**, que HBase no tiene: una tabla son celdas de bytes bajo una clave de fila,
//! sin esquema ni tipos. Lo que aqui se llama «columna» sale de **muestrear** filas (ficha 0072, §7.5):
//!
//! - `_clave`: la clave de fila entera, siempre.
//! - `familia:cualificador`: una por cada celda que la muestra vio. El nombre fisico es ese texto; la familia no
//!   puede llevar `:`, asi que se parte en el primero.
//!
//! **Nada se infiere en silencio.** Hasta que existan las codificaciones (N4), una columna es `String` (todos los
//! valores de la muestra son texto UTF-8 imprimible) u `Opaque` (los bytes, en base64). Nunca un numero: ocho
//! bytes pueden ser un `long`, un `double` o un texto, y adivinarlo es corromper.

use ore_driver::{Codigo, Fallo};

/// El nombre fisico de la clave de fila.
pub const CLAVE: &str = "_clave";
/// El espacio de nombres de las tablas que no dicen otro.
pub const ESPACIO_POR_DEFECTO: &str = "default";

/// Lo que se hace con los bytes de una celda. Cada uno viaja como texto Arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    /// Texto UTF-8: `String` en OOS.
    Texto,
    /// Bytes cualesquiera, en base64: `Opaque` en OOS.
    Opaco,
}

impl Tipo {
    pub fn oos(self) -> &'static str {
        match self {
            Tipo::Texto => "String",
            Tipo::Opaco => "Opaque",
        }
    }

    /// El tipo que la tabla declara para una columna. Cualquier otro necesita una codificacion que N3 no tiene:
    /// es un `esquema` con la columna nombrada, nunca una conversion a escondidas.
    pub fn declarado(declarado: &str, objeto: &str, columna: &str) -> Result<Tipo, Fallo> {
        match declarado.trim() {
            "String" => Ok(Tipo::Texto),
            "Opaque" => Ok(Tipo::Opaco),
            otro => Err(Fallo::new(
                Codigo::Esquema,
                format!(
                    "`{objeto}`, columna `{columna}`: se declara `{otro}`, pero HBase guarda bytes sin tipo y su codificacion \
                     (entero de Java, IEEE, texto…) aun no se puede declarar (ficha 0072, N4). Solo `String` y `Opaque`"
                ),
            )),
        }
    }
}

/// Una columna de una tabla muestreada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columna {
    /// `familia:cualificador`, o [`CLAVE`].
    pub nombre: String,
    pub tipo: Tipo,
    /// En cuantas filas de la muestra tenia valor.
    pub filas_con_valor: usize,
}

/// Una tabla, tal como la muestra la vio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabla {
    pub espacio: String,
    pub nombre: String,
    /// El tipo de la clave de fila.
    pub clave: Tipo,
    /// Las columnas de celdas, por familia y cualificador.
    pub columnas: Vec<Columna>,
    pub filas_muestreadas: usize,
    pub regiones: usize,
    pub avisos: Vec<String>,
}

impl Tabla {
    /// `espacio.tabla`: como se nombra en ORE.
    pub fn objeto(&self) -> String {
        format!("{}.{}", self.espacio, self.nombre)
    }
}

/// `espacio.tabla` o `tabla` (en el espacio por defecto). Un espacio no lleva `.`, una tabla si: se parte en el
/// primero.
pub fn partir_objeto(objeto: &str) -> Result<(String, String), Fallo> {
    match objeto.split_once('.') {
        Some((e, t)) if !e.is_empty() && !t.is_empty() => Ok((e.to_string(), t.to_string())),
        None if !objeto.is_empty() => Ok((ESPACIO_POR_DEFECTO.to_string(), objeto.to_string())),
        _ => Err(Fallo::operador(format!(
            "el objeto `{objeto}` no es `espacio.tabla` ni `tabla`"
        ))),
    }
}

/// El nombre que el cliente pide a `hbase:meta`.
pub fn en_hbase(espacio: &str, tabla: &str) -> String {
    format!("{espacio}:{tabla}")
}

/// `familia:cualificador` en sus dos bytes. La familia no lleva `:` y el cualificador si.
pub fn partir_columna(nombre: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    match nombre.split_once(':') {
        Some((f, q)) if !f.is_empty() => Ok((f.as_bytes().to_vec(), q.as_bytes().to_vec())),
        _ => Err(format!(
            "la columna `{nombre}` no es `{CLAVE}` ni `familia:cualificador`"
        )),
    }
}

/// ¿Es texto que una persona leeria? UTF-8 valido y sin caracteres de control (salvo tabulador y saltos).
pub fn es_texto(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok_and(|s| {
        s.chars()
            .all(|c| !(c.is_control() && !matches!(c, '\t' | '\n' | '\r')))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_objeto_se_parte_en_el_primer_punto_y_una_tabla_puede_llevar_puntos() {
        assert_eq!(
            partir_objeto("ventas.pedidos").unwrap(),
            ("ventas".into(), "pedidos".into())
        );
        assert_eq!(
            partir_objeto("ventas.a.b").unwrap(),
            ("ventas".into(), "a.b".into())
        );
        assert_eq!(
            partir_objeto("pedidos").unwrap(),
            ("default".into(), "pedidos".into())
        );
        for malo in ["", ".t", "e."] {
            assert!(partir_objeto(malo).is_err(), "{malo:?}");
        }
    }

    #[test]
    fn la_columna_se_parte_en_los_dos_puntos_y_el_cualificador_puede_llevarlos() {
        assert_eq!(
            partir_columna("info:nombre").unwrap(),
            (b"info".to_vec(), b"nombre".to_vec())
        );
        assert_eq!(
            partir_columna("f:a:b").unwrap(),
            (b"f".to_vec(), b"a:b".to_vec())
        );
        assert_eq!(partir_columna("f:").unwrap(), (b"f".to_vec(), b"".to_vec()));
        assert!(partir_columna("sin-familia").is_err());
        assert!(partir_columna(":q").is_err());
    }

    #[test]
    fn texto_es_utf8_imprimible_y_lo_demas_no() {
        assert!(es_texto(b"hola"));
        assert!(es_texto("canción\ncon salto\ty tab".as_bytes()));
        assert!(es_texto(b""));
        assert!(
            !es_texto(&[0x00, 0x00, 0x00, 0x2a]),
            "un entero de Java no es texto"
        );
        assert!(!es_texto(&[0xff, 0xfe]));
        assert!(!es_texto(b"con \x07 campana"));
    }

    #[test]
    fn solo_string_y_opaque_se_pueden_declarar_hasta_que_haya_codificaciones() {
        assert_eq!(
            Tipo::declarado("String", "e.t", "f:q").unwrap(),
            Tipo::Texto
        );
        assert_eq!(
            Tipo::declarado("Opaque", "e.t", "f:q").unwrap(),
            Tipo::Opaco
        );
        let e = Tipo::declarado("Integer", "e.t", "f:n").unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(
            e.mensaje.contains("f:n") && e.mensaje.contains("N4"),
            "{}",
            e.mensaje
        );
    }
}
