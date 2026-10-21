//! **La forma de una tabla de HBase**, que HBase no tiene: una tabla son celdas de bytes bajo una clave de fila,
//! sin esquema ni tipos. Lo que aqui se llama «columna» sale de **muestrear** filas (ficha 0072, §7.5):
//!
//! - `_clave`: la clave de fila entera, siempre.
//! - `familia:cualificador`: una por cada celda que la muestra vio. El nombre fisico es ese texto; la familia no
//!   puede llevar `:`, asi que se parte en el primero.
//!
//! **Nada se infiere en silencio.** El catalogo propone `String` (todo lo visto es texto UTF-8 imprimible) u
//! `Opaque` (los bytes, en base64) y **nunca un numero**: ocho bytes pueden ser un `long`, un `double` o un texto,
//! y adivinarlo es corromper. Un tipo numerico, una fecha o un decimal los declara la tabla, con el tipo de OOS en
//! `type` y la **codificacion** de los bytes en `physicalType` (`i32be`, `f64be`, `epoch-ms`…): ver
//! [`crate::codificacion`]. Un tipo que no es `String` ni `Opaque` sin codificacion es un error de `esquema` que
//! nombra la columna.

use crate::codificacion::Codificacion;
use ore_core::tipos::Fisico;
use ore_core::types::{Type, parse_type};
use ore_driver::{Codigo, Fallo};

/// El nombre fisico de la clave de fila.
pub const CLAVE: &str = "_clave";
/// El espacio de nombres de las tablas que no dicen otro.
pub const ESPACIO_POR_DEFECTO: &str = "default";

/// Lo que es una columna: su tipo de OOS, su fisico (en que tipo de Arrow vive) y como estan escritos sus bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tipo {
    /// El tipo de OOS tal como la tabla lo declara (`Integer`, `Decimal<12, 2>`…): sale en los metadatos del campo.
    pub oos: String,
    pub fisico: Fisico,
    /// `Opaque`: bytes cualesquiera. Como fisico es texto (base64), por eso se lleva aparte.
    pub opaco: bool,
    pub cod: Codificacion,
}

impl Tipo {
    pub fn texto() -> Tipo {
        Tipo {
            oos: "String".into(),
            fisico: Fisico::Texto,
            opaco: false,
            cod: Codificacion::Utf8,
        }
    }

    pub fn opaco() -> Tipo {
        Tipo {
            oos: "Opaque".into(),
            fisico: Fisico::Texto,
            opaco: true,
            cod: Codificacion::Bytes,
        }
    }

    /// El tipo de una columna a partir de lo que la tabla declara de ella: el tipo de OOS (`tipos`) y la
    /// codificacion de sus bytes (`codificaciones`). Cualquiera puede faltar:
    ///
    /// - ninguno: `String`, texto UTF-8 (lo que dice el catalogo);
    /// - solo la codificacion: `utf8` es `String` y `bytes` es `Opaque`; las demas necesitan el tipo;
    /// - el tipo sin codificacion: `String` es `utf8` y `Opaque` es `bytes`; los demas necesitan la codificacion.
    ///
    /// Cada falta, y cada pareja que no se puede (un `Integer` leido como `f64be`), es un error de `esquema` que
    /// nombra la columna y dice que si.
    pub fn nuevo(
        tipo: Option<&str>,
        cod: Option<&str>,
        objeto: &str,
        columna: &str,
    ) -> Result<Tipo, Fallo> {
        let falla = |m: String| {
            Fallo::new(
                Codigo::Esquema,
                format!("`{objeto}`, columna `{columna}`: {m}"),
            )
        };
        let cod_declarada = match cod {
            None => None,
            Some(n) => Some(Codificacion::analizar(n).ok_or_else(|| {
                falla(format!(
                    "`{n}` no es una codificacion de bytes de HBase. Las hay: {}",
                    Codificacion::lista()
                ))
            })?),
        };
        let Some(t) = tipo else {
            return match cod_declarada {
                None | Some(Codificacion::Utf8) => Ok(Tipo::texto()),
                Some(Codificacion::Bytes) => Ok(Tipo::opaco()),
                Some(c) => Err(falla(format!(
                    "declara la codificacion `{}` pero no el tipo (`type`): no se adivina",
                    c.nombre()
                ))),
            };
        };
        let t = t.trim();
        let parseado = parse_type(t).map_err(|_| falla(format!("`{t}` no es un tipo de OOS")))?;
        let (fisico, opaco) = match &parseado {
            Type::Scalar(s) if s == "Opaque" => (Fisico::Texto, true),
            Type::Scalar(_) | Type::Decimal { .. } | Type::Parametric { .. } => {
                (Fisico::de(&parseado), false)
            }
            _ => {
                return Err(falla(format!(
                    "HBase no sabe leer `{t}`: solo escalares (String, Opaque, Integer, Float, Boolean, Decimal, Date, DateTime, DateTimeTz)"
                )));
            }
        };
        // `String` y los escalares que no tienen su propio fisico caen en `Texto`: solo `String` y `Opaque` son texto.
        if fisico == Fisico::Texto
            && !opaco
            && !matches!(&parseado, Type::Scalar(s) if s == "String")
        {
            return Err(falla(format!(
                "HBase no sabe leer `{t}` (es texto, pero no `String` ni `Opaque`)"
            )));
        }
        let cod = match cod_declarada {
            Some(c) => c,
            None if opaco => Codificacion::Bytes,
            None if fisico == Fisico::Texto => Codificacion::Utf8,
            None => {
                return Err(falla(format!(
                    "`{t}` necesita decir como estan escritos sus bytes: pon su codificacion en `physicalType` \
                     ({}). HBase guarda bytes sin tipo y no se adivina",
                    Codificacion::lista()
                )));
            }
        };
        if !cod.sirve_para(&fisico, opaco) {
            return Err(falla(format!(
                "`{t}` no se puede leer con la codificacion `{}`",
                cod.nombre()
            )));
        }
        Ok(Tipo {
            oos: t.to_string(),
            fisico,
            opaco,
            cod,
        })
    }
}

// ── La clave compuesta ──────────────────────────────────────────────────────────────────────────────────────────

/// El prefijo de las columnas que son una parte de la clave de fila: `_clave#1`, `_clave#2`…
pub const PREFIJO_PARTE: &str = "_clave#";

/// El numero (desde 1) de una columna `_clave#N`.
pub fn indice_de_parte(columna: &str) -> Option<usize> {
    columna
        .strip_prefix(PREFIJO_PARTE)?
        .parse()
        .ok()
        .filter(|n| *n >= 1)
}

/// Donde acaba una parte de la clave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Largo {
    /// Un ancho fijo en bytes (`i32be` son 4; `utf8:10`, 10).
    Fijo(usize),
    /// Hasta un separador, que no es de la parte (`utf8:fin=23`: hasta el primer `#`).
    Hasta(Vec<u8>),
    /// El resto de la clave. Solo la ultima parte.
    Resto,
}

/// Una parte de una clave compuesta: su columna, su tipo y donde acaba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParteClave {
    /// `_clave#N`.
    pub columna: String,
    pub tipo: Tipo,
    pub largo: Largo,
}

fn de_hex(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

impl ParteClave {
    /// De lo que la tabla declara de la columna `_clave#N`: el tipo y la especificacion de sus bytes
    /// `<codificacion>[:<ancho>|:fin=<hex>]` (`i32be`, `utf8:10`, `utf8:fin=23`, `bytes:4`).
    pub fn nueva(
        columna: &str,
        tipo: Option<&str>,
        spec: &str,
        objeto: &str,
    ) -> Result<ParteClave, Fallo> {
        let falla = |m: String| {
            Fallo::new(
                Codigo::Esquema,
                format!("`{objeto}`, parte de la clave `{columna}`: {m}"),
            )
        };
        let (cod, suf) = match spec.split_once(':') {
            Some((c, s)) => (c, Some(s)),
            None => (spec, None),
        };
        let t = Tipo::nuevo(tipo, Some(cod), objeto, columna)?;
        let largo = match (suf, t.cod.ancho()) {
            (None, Some(n)) => Largo::Fijo(n),
            (None, None) => Largo::Resto,
            (Some(s), ancho) => {
                if let Some(h) = s.strip_prefix("fin=") {
                    Largo::Hasta(de_hex(h).ok_or_else(|| {
                        falla(format!(
                            "`fin={h}` no son bytes en hexadecimal (`fin=23` es `#`)"
                        ))
                    })?)
                } else {
                    let n: usize = s.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                        falla(format!("`{s}` no es un ancho (`:10`) ni `fin=<hex>`"))
                    })?;
                    if ancho.is_some_and(|a| a != n) {
                        return Err(falla(format!(
                            "`{cod}` ocupa {} byte(s), no {n}",
                            ancho.unwrap_or(0)
                        )));
                    }
                    Largo::Fijo(n)
                }
            }
        };
        Ok(ParteClave {
            columna: columna.to_string(),
            tipo: t,
            largo,
        })
    }
}

/// Las partes de la clave que la peticion declara (columnas `_clave#N` en `tipos` o en `codificaciones`), en orden.
/// Vacio: la clave no es compuesta. Tienen que ser `1..=k` sin huecos, y todas menos la ultima tener fin.
pub fn partes_de(
    tipos: &[(String, String)],
    cods: &[(String, String)],
    objeto: &str,
) -> Result<Vec<ParteClave>, Fallo> {
    let mut indices: Vec<usize> = tipos
        .iter()
        .chain(cods)
        .filter_map(|(c, _)| indice_de_parte(c))
        .collect();
    indices.sort_unstable();
    indices.dedup();
    let Some(&k) = indices.last() else {
        return Ok(Vec::new());
    };
    if indices != (1..=k).collect::<Vec<_>>() {
        return Err(Fallo::new(
            Codigo::Esquema,
            format!(
                "`{objeto}`: las partes de la clave son {indices:?} y tienen que ser 1..={k} sin huecos"
            ),
        ));
    }
    let mut partes = Vec::new();
    for i in 1..=k {
        let col = format!("{PREFIJO_PARTE}{i}");
        let de = |v: &[(String, String)]| v.iter().find(|(c, _)| *c == col).map(|(_, x)| x.clone());
        let spec = de(cods).ok_or_else(|| {
            Fallo::new(
                Codigo::Esquema,
                format!("`{objeto}`: la parte `{col}` de la clave necesita su codificacion en `physicalType`"),
            )
        })?;
        partes.push(ParteClave::nueva(
            &col,
            de(tipos).as_deref(),
            &spec,
            objeto,
        )?);
    }
    for p in &partes[..partes.len() - 1] {
        if p.largo == Largo::Resto {
            return Err(Fallo::new(
                Codigo::Esquema,
                format!(
                    "`{objeto}`: `{}` no es la ultima parte y no dice donde acaba: pon un ancho (`utf8:10`) o un fin (`utf8:fin=23`)",
                    p.columna
                ),
            ));
        }
    }
    Ok(partes)
}

/// Parte una clave en los trozos de cada parte. `Err` con el motivo si no cabe en la forma declarada.
pub fn trocear<'a>(partes: &[ParteClave], clave: &'a [u8]) -> Result<Vec<&'a [u8]>, String> {
    let mut sal = Vec::with_capacity(partes.len());
    let mut resto = clave;
    for p in partes {
        match &p.largo {
            Largo::Fijo(n) => {
                if resto.len() < *n {
                    return Err(format!(
                        "a la clave le faltan bytes para `{}` ({n} y quedan {})",
                        p.columna,
                        resto.len()
                    ));
                }
                let (a, b) = resto.split_at(*n);
                sal.push(a);
                resto = b;
            }
            Largo::Hasta(sep) => {
                let i = resto
                    .windows(sep.len())
                    .position(|w| w == sep.as_slice())
                    .ok_or_else(|| format!("en la clave no aparece el fin de `{}`", p.columna))?;
                sal.push(&resto[..i]);
                resto = &resto[i + sep.len()..];
            }
            Largo::Resto => {
                sal.push(resto);
                resto = &[];
            }
        }
    }
    if !resto.is_empty() {
        return Err(format!(
            "a la clave le sobran {} byte(s) tras la ultima parte",
            resto.len()
        ));
    }
    Ok(sal)
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

    fn partes(v: &[(&str, &str, &str)]) -> Result<Vec<ParteClave>, Fallo> {
        let tipos: Vec<_> = v
            .iter()
            .map(|(c, t, _)| (c.to_string(), t.to_string()))
            .collect();
        let cods: Vec<_> = v
            .iter()
            .map(|(c, _, e)| (c.to_string(), e.to_string()))
            .collect();
        partes_de(&tipos, &cods, "e.t")
    }

    #[test]
    fn las_partes_de_una_clave_compuesta_se_leen_de_las_columnas_clave_n() {
        let p = partes(&[
            ("_clave#1", "String", "utf8:fin=23"),
            ("_clave#2", "Integer", "i32be"),
            ("_clave#3", "String", "utf8"),
        ])
        .unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].largo, Largo::Hasta(vec![0x23]));
        assert_eq!(p[1].largo, Largo::Fijo(4));
        assert_eq!(p[2].largo, Largo::Resto);
        assert!(
            partes(&[]).unwrap().is_empty(),
            "sin columnas `_clave#N` la clave no es compuesta"
        );
        assert_eq!(indice_de_parte("_clave#12"), Some(12));
        assert_eq!(indice_de_parte("_clave"), None);
        assert_eq!(indice_de_parte("_clave#0"), None);
    }

    #[test]
    fn una_declaracion_de_partes_mal_hecha_dice_cual_es_el_problema() {
        let e = partes(&[
            ("_clave#1", "String", "utf8"),
            ("_clave#2", "Integer", "i32be"),
        ])
        .unwrap_err();
        assert!(
            e.mensaje.contains("_clave#1") && e.mensaje.contains("donde acaba"),
            "{}",
            e.mensaje
        );
        let e = partes(&[("_clave#2", "Integer", "i32be")]).unwrap_err();
        assert!(e.mensaje.contains("sin huecos"), "{}", e.mensaje);
        let e = partes(&[("_clave#1", "Integer", "i32be:8")]).unwrap_err();
        assert!(e.mensaje.contains("4 byte"), "{}", e.mensaje);
        for mala in ["utf8:0", "utf8:abc", "utf8:fin=", "utf8:fin=2"] {
            assert!(partes(&[("_clave#1", "String", mala)]).is_err(), "{mala}");
        }
        assert!(ParteClave::nueva("_clave#1", Some("Integer"), "texto", "e.t").is_ok());
    }

    #[test]
    fn la_clave_se_trocea_por_ancho_por_fin_y_por_resto() {
        let p = partes(&[
            ("_clave#1", "String", "utf8:fin=23"),
            ("_clave#2", "Integer", "i32be"),
            ("_clave#3", "String", "utf8"),
        ])
        .unwrap();
        let mut k = b"ana#".to_vec();
        k.extend(7i32.to_be_bytes());
        k.extend(b"resto");
        assert_eq!(
            trocear(&p, &k).unwrap(),
            vec![&b"ana"[..], &[0, 0, 0, 7][..], &b"resto"[..]]
        );
        assert!(trocear(&p, b"ana").unwrap_err().contains("fin"));
        assert!(trocear(&p, b"ana#\0\0").unwrap_err().contains("faltan"));
        let solo_ancho = partes(&[
            ("_clave#1", "String", "utf8:3"),
            ("_clave#2", "Integer", "i16be"),
        ])
        .unwrap();
        assert!(trocear(&solo_ancho, b"abc\0\x07").is_ok());
        assert!(
            trocear(&solo_ancho, b"abc\0\x07x")
                .unwrap_err()
                .contains("sobran")
        );
        // un separador de varios bytes
        let dos = partes(&[
            ("_clave#1", "String", "utf8:fin=3a3a"),
            ("_clave#2", "String", "utf8"),
        ])
        .unwrap();
        assert_eq!(trocear(&dos, b"a::b").unwrap(), vec![&b"a"[..], &b"b"[..]]);
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

    fn nuevo(t: Option<&str>, c: Option<&str>) -> Result<Tipo, Fallo> {
        Tipo::nuevo(t, c, "e.t", "f:q")
    }

    #[test]
    fn sin_declarar_nada_es_string_utf8_y_con_solo_la_codificacion_de_texto_tambien() {
        assert_eq!(nuevo(None, None).unwrap(), Tipo::texto());
        assert_eq!(nuevo(None, Some("utf8")).unwrap(), Tipo::texto());
        assert_eq!(nuevo(None, Some("bytes")).unwrap(), Tipo::opaco());
        assert_eq!(nuevo(Some("String"), None).unwrap(), Tipo::texto());
        assert_eq!(nuevo(Some("Opaque"), None).unwrap(), Tipo::opaco());
        assert_eq!(nuevo(Some("Opaque"), Some("bytes")).unwrap(), Tipo::opaco());
    }

    #[test]
    fn un_tipo_con_su_codificacion_se_entiende() {
        let t = nuevo(Some("Integer"), Some("i32be")).unwrap();
        assert_eq!(
            (t.oos.as_str(), t.fisico, t.cod),
            ("Integer", Fisico::Entero, Codificacion::I32Be)
        );
        let t = nuevo(Some("Decimal<12, 2>"), Some("decimal")).unwrap();
        assert_eq!(
            t.fisico,
            Fisico::Decimal {
                precision: 12,
                escala: 2
            }
        );
        assert_eq!(
            nuevo(Some("DateTimeTz"), Some("epoch-ms")).unwrap().fisico,
            Fisico::Instante
        );
        assert_eq!(
            nuevo(Some("Boolean"), Some("BOOL")).unwrap().cod,
            Codificacion::Bool,
            "mayusculas valen"
        );
        assert_eq!(
            nuevo(Some("Integer"), Some("texto")).unwrap().cod,
            Codificacion::Texto
        );
    }

    #[test]
    fn un_tipo_que_no_es_string_ni_opaque_sin_codificacion_es_esquema_con_la_columna_y_las_que_hay()
    {
        let e = nuevo(Some("Integer"), None).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(
            e.mensaje.contains("f:q")
                && e.mensaje.contains("physicalType")
                && e.mensaje.contains("i32be"),
            "{}",
            e.mensaje
        );
    }

    #[test]
    fn una_codificacion_que_no_existe_o_que_no_sirve_para_el_tipo_se_dice() {
        let e = nuevo(Some("Integer"), Some("numeric(10,2)")).unwrap_err();
        assert!(
            e.mensaje.contains("numeric(10,2)") && e.mensaje.contains("i64be"),
            "{}",
            e.mensaje
        );
        let e = nuevo(Some("Integer"), Some("f64be")).unwrap_err();
        assert!(e.mensaje.contains("f64be"), "{}", e.mensaje);
        assert!(nuevo(Some("String"), Some("i32be")).is_err());
        assert!(
            nuevo(Some("Opaque"), Some("utf8")).is_err(),
            "Opaque es bytes"
        );
        assert!(nuevo(Some("Float"), Some("i64be")).is_err());
    }

    #[test]
    fn la_codificacion_sin_tipo_solo_vale_si_es_de_texto_y_lo_que_no_es_escalar_no_se_lee() {
        let e = nuevo(None, Some("i32be")).unwrap_err();
        assert!(e.mensaje.contains("no el tipo"), "{}", e.mensaje);
        for t in ["list<String>", "Time", "Geometry", "Json"] {
            assert!(nuevo(Some(t), Some("utf8")).is_err(), "{t}");
        }
        assert!(nuevo(Some("Cosa que no es un tipo"), None).is_err());
    }
}
