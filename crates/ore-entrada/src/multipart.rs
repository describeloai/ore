//! **Un `multipart/form-data` leído en flujo** (ADR 0074 AR2·1).
//!
//! La subida de PyPI (`uv publish`, `twine upload`) es un `POST` multipart por
//! fichero: unos campos pequeños (`name`, `version`, `sha256_digest`…) y el
//! fichero. Lo medido en AR2·0: **los dos mandan todos los campos antes y el
//! fichero el último**, con `Content-Length`. Así que quien recibe decide con
//! los campos antes de tocar un byte del fichero, y el fichero pasa en flujo a
//! donde vaya, con la memoria constante.
//!
//! ```text
//! let mut m = Multipart::nuevo(subida, tipo_de_contenido, largo)?;
//! while let Some(parte) = m.siguiente()? {
//!     match parte {
//!         Parte::Campo { nombre, valor } => …,
//!         Parte::Fichero { .. } => { let largo = m.largo_del_fichero(); m.fichero() … }
//!     }
//! }
//! ```
//!
//! **El fichero tiene que ser la última parte.** Su largo es lo que queda del
//! cuerpo menos el cierre (`\r\n--<límite>--\r\n`): así se le puede dar un
//! `Content-Length` a quien lo recibe sin guardarlo antes. El lector del
//! fichero **comprueba el cierre antes de entregar su último byte**; si no
//! está —había otra parte detrás, o el cuerpo no termina así—, falla, y quien
//! recibía no ve un fichero entero.

use std::io::Read;

/// Lo que se guarda de un campo: los de PyPI son cortos; lo demás (la
/// descripción larga que manda `twine`) se lee y se tira.
pub const CAMPO_MAXIMO: usize = 64 * 1024;

/// Las cabeceras de una parte.
const CABEZA_MAXIMA: usize = 16 * 1024;

const TROZO: usize = 64 * 1024;

/// Una parte del formulario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parte {
    /// Un campo: su valor, como texto (hasta [`CAMPO_MAXIMO`] bytes).
    Campo { nombre: String, valor: String },
    /// El fichero: su nombre de campo y el de fichero. Sus bytes se leen con
    /// [`Multipart::fichero`].
    Fichero {
        nombre: String,
        fichero: String,
        tipo: Option<String>,
    },
}

pub struct Multipart<R: Read> {
    lector: R,
    /// Lo que falta por leer de `lector`.
    queda: u64,
    /// `\r\n--<límite>`: lo que separa una parte de la siguiente.
    separador: Vec<u8>,
    buf: Vec<u8>,
    pos: usize,
    /// Donde se lee cada trozo (una vez reservado, se reusa).
    trozo: Vec<u8>,
    empezado: bool,
    terminado: bool,
    en_fichero: bool,
}

/// El `boundary` de un `Content-Type: multipart/form-data; boundary=…`.
pub fn limite_de(tipo_de_contenido: &str) -> Option<String> {
    let mut trozos = tipo_de_contenido.split(';');
    if !trozos
        .next()?
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return None;
    }
    trozos.find_map(|t| {
        let (k, v) = t.split_once('=')?;
        if !k.trim().eq_ignore_ascii_case("boundary") {
            return None;
        }
        let v = v.trim().trim_matches('"');
        (!v.is_empty() && v.len() <= 200).then(|| v.to_string())
    })
}

fn buscar(pajar: &[u8], aguja: &[u8]) -> Option<usize> {
    pajar.windows(aguja.len()).position(|w| w == aguja)
}

/// Un parámetro (`name="…"`) de una cabecera `Content-Disposition`.
fn parametro(cabecera: &str, clave: &str) -> Option<String> {
    let mut resto = cabecera;
    while let Some(i) = resto.find(';') {
        resto = resto[i + 1..].trim_start();
        let Some((k, v)) = resto.split_once('=') else {
            break;
        };
        if k.trim().eq_ignore_ascii_case(clave) {
            let v = v.trim_start();
            return Some(if let Some(c) = v.strip_prefix('"') {
                c.split('"').next().unwrap_or_default().to_string()
            } else {
                v.split(';').next().unwrap_or_default().trim().to_string()
            });
        }
    }
    None
}

impl<R: Read> Multipart<R> {
    /// `largo`: lo que mide el cuerpo entero (su `Content-Length`).
    pub fn nuevo(lector: R, tipo_de_contenido: &str, largo: u64) -> Result<Self, String> {
        let limite = limite_de(tipo_de_contenido)
            .ok_or("the body is not `multipart/form-data` with a `boundary`")?;
        let mut separador = b"\r\n--".to_vec();
        separador.extend_from_slice(limite.as_bytes());
        Ok(Multipart {
            lector,
            queda: largo,
            separador,
            // El primer límite no lleva `\r\n` delante: se le pone, y todos se
            // buscan igual.
            buf: b"\r\n".to_vec(),
            pos: 0,
            trozo: Vec::new(),
            empezado: false,
            terminado: false,
            en_fichero: false,
        })
    }

    fn pendiente(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    /// Trae otro trozo; `false` si el cuerpo se acabó.
    fn llenar(&mut self) -> Result<bool, String> {
        if self.queda == 0 {
            return Ok(false);
        }
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        let quiero = (TROZO as u64).min(self.queda) as usize;
        if self.trozo.len() < quiero {
            self.trozo.resize(quiero, 0);
        }
        let n = self
            .lector
            .read(&mut self.trozo[..quiero])
            .map_err(|e| format!("the upload was cut short: {e}"))?;
        if n == 0 {
            return Err("the upload was cut short".into());
        }
        self.buf.extend_from_slice(&self.trozo[..n]);
        self.queda -= n as u64;
        Ok(true)
    }

    /// Lee hasta el siguiente separador y lo consume; devuelve lo de antes
    /// (hasta `guardar` bytes: el resto se tira).
    fn hasta_el_separador(&mut self, guardar: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        loop {
            if let Some(i) = buscar(self.pendiente(), &self.separador) {
                let cabe = guardar.saturating_sub(out.len()).min(i);
                out.extend_from_slice(&self.pendiente()[..cabe]);
                self.pos += i + self.separador.len();
                return Ok(out);
            }
            // Lo que ya no puede ser el principio de un separador, fuera.
            let seguro = self
                .pendiente()
                .len()
                .saturating_sub(self.separador.len() - 1);
            let cabe = guardar.saturating_sub(out.len()).min(seguro);
            out.extend_from_slice(&self.pendiente()[..cabe]);
            self.pos += seguro;
            if !self.llenar()? {
                return Err("the form ends without its closing boundary".into());
            }
        }
    }

    /// Asegura `n` bytes pendientes (o los que queden).
    fn asegurar(&mut self, n: usize) -> Result<(), String> {
        while self.pendiente().len() < n {
            if !self.llenar()? {
                break;
            }
        }
        Ok(())
    }

    /// La siguiente parte, o `None` al cierre. Tras un [`Parte::Fichero`] no
    /// hay siguiente: el fichero es lo último.
    pub fn siguiente(&mut self) -> Result<Option<Parte>, String> {
        if self.terminado {
            return Ok(None);
        }
        if self.en_fichero {
            return Err("the file is the last part of the form".into());
        }
        // La primera vez, saltar lo que hubiera antes del primer límite.
        if !self.empezado {
            self.empezado = true;
            self.hasta_el_separador(0)?;
        }
        // Tras el separador: `--` (el cierre) o `\r\n` (otra parte).
        self.asegurar(2)?;
        if self.pendiente().starts_with(b"--") {
            self.terminado = true;
            return Ok(None);
        }
        if !self.pendiente().starts_with(b"\r\n") {
            return Err("a boundary is not followed by a line break".into());
        }
        self.pos += 2;
        // Las cabeceras de la parte, hasta la línea vacía.
        let fin = loop {
            if let Some(i) = buscar(self.pendiente(), b"\r\n\r\n") {
                break i;
            }
            if self.pendiente().len() > CABEZA_MAXIMA {
                return Err("the headers of a part are too long".into());
            }
            if !self.llenar()? {
                return Err("a part ends inside its headers".into());
            }
        };
        let cabeza = String::from_utf8_lossy(&self.pendiente()[..fin]).into_owned();
        self.pos += fin + 4;
        let (mut disposicion, mut tipo) = (None, None);
        for l in cabeza.split("\r\n") {
            if let Some((k, v)) = l.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "content-disposition" => disposicion = Some(v.trim().to_string()),
                    "content-type" => tipo = Some(v.trim().to_string()),
                    _ => {}
                }
            }
        }
        let disposicion = disposicion.ok_or("a part without `Content-Disposition`")?;
        let nombre = parametro(&disposicion, "name").ok_or("a part without a name")?;
        if let Some(fichero) = parametro(&disposicion, "filename") {
            self.en_fichero = true;
            return Ok(Some(Parte::Fichero {
                nombre,
                fichero,
                tipo,
            }));
        }
        let valor = self.hasta_el_separador(CAMPO_MAXIMO)?;
        Ok(Some(Parte::Campo {
            nombre,
            valor: String::from_utf8_lossy(&valor).into_owned(),
        }))
    }

    /// El cierre que va detrás del fichero: `\r\n--<límite>--\r\n`.
    fn cierre(&self) -> Vec<u8> {
        let mut c = self.separador.clone();
        c.extend_from_slice(b"--\r\n");
        c
    }

    /// Lo que mide el fichero (tras un [`Parte::Fichero`]): lo que queda del
    /// cuerpo menos el cierre. `None` si no cabe ni el cierre.
    pub fn largo_del_fichero(&self) -> Option<u64> {
        (self.pendiente().len() as u64 + self.queda).checked_sub(self.cierre().len() as u64)
    }

    /// Los bytes del fichero, en flujo. Antes del último comprueba el cierre.
    pub fn fichero(&mut self) -> Result<Fichero<'_, R>, String> {
        if !self.en_fichero {
            return Err("no file part has been reached".into());
        }
        let restante = self
            .largo_del_fichero()
            .ok_or("the form ends before its file")?;
        Ok(Fichero {
            m: self,
            restante,
            comprobado: false,
        })
    }
}

/// El fichero de un formulario: [`Read`] de exactamente
/// [`Multipart::largo_del_fichero`] bytes, que falla si detrás no está el cierre.
pub struct Fichero<'a, R: Read> {
    m: &'a mut Multipart<R>,
    restante: u64,
    comprobado: bool,
}

impl<R: Read> Fichero<'_, R> {
    fn comprobar_el_cierre(&mut self) -> std::io::Result<()> {
        let cierre = self.m.cierre();
        let mal = |m: String| std::io::Error::new(std::io::ErrorKind::InvalidData, m);
        self.m.asegurar(cierre.len()).map_err(mal)?;
        if self.m.pendiente() != cierre.as_slice() {
            return Err(mal(
                "the file is not the last part of the form (or the form does not end with its closing boundary)"
                    .into(),
            ));
        }
        self.m.pos = self.m.buf.len();
        self.m.terminado = true;
        self.comprobado = true;
        Ok(())
    }
}

impl<R: Read> Read for Fichero<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.restante == 0 {
            if !self.comprobado {
                self.comprobar_el_cierre()?;
            }
            return Ok(0);
        }
        let corto = |e: String| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, e);
        let sep = self.m.separador.len();
        // Lo que se puede dar: lo pendiente que es del fichero, sin un
        // separador dentro, y sin sus últimos `sep - 1` bytes salvo que sean el
        // final del fichero (podrían ser el principio de un separador partido).
        let n = loop {
            let del_fichero = (self.m.pendiente().len() as u64).min(self.restante) as usize;
            if buscar(&self.m.pendiente()[..del_fichero], &self.m.separador).is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the file is not the last part of the form",
                ));
            }
            let dable = if del_fichero as u64 == self.restante {
                del_fichero
            } else {
                del_fichero.saturating_sub(sep - 1)
            };
            if dable > 0 {
                break dable.min(out.len());
            }
            if !self.m.llenar().map_err(corto)? {
                return Err(corto("the upload was cut short".into()));
            }
        };
        out[..n].copy_from_slice(&self.m.pendiente()[..n]);
        self.m.pos += n;
        self.restante -= n as u64;
        // ⭐ El último byte no sale sin comprobar el cierre: quien recibe el
        //   fichero (con su largo exacto) no lo da por entero si esto falla.
        if self.restante == 0 {
            self.comprobar_el_cierre()?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const TIPO: &str = "multipart/form-data; boundary=XyZ";

    fn formulario(campos: &[(&str, &str)], fichero: &[u8], cola: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        for (k, v) in campos {
            b.extend_from_slice(
                format!("--XyZ\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                    .as_bytes(),
            );
        }
        b.extend_from_slice(
            b"--XyZ\r\nContent-Disposition: form-data; name=\"content\"; filename=\"a-1.0-py3-none-any.whl\"\r\nContent-Type: application/octet-stream\r\n\r\n",
        );
        b.extend_from_slice(fichero);
        b.extend_from_slice(cola);
        b
    }

    /// Un lector que da de `n` en `n` bytes: los límites caen partidos.
    struct Goteo(std::io::Cursor<Vec<u8>>, usize);
    impl Read for Goteo {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.1);
            self.0.read(&mut buf[..n])
        }
    }

    fn leer(cuerpo: Vec<u8>, goteo: usize) -> Result<(Vec<Parte>, Vec<u8>), String> {
        let largo = cuerpo.len() as u64;
        let mut m = Multipart::nuevo(Goteo(std::io::Cursor::new(cuerpo), goteo), TIPO, largo)?;
        let mut partes = Vec::new();
        let mut bytes = Vec::new();
        while let Some(p) = m.siguiente()? {
            let es_fichero = matches!(p, Parte::Fichero { .. });
            partes.push(p);
            if es_fichero {
                let esperado = m.largo_del_fichero().unwrap();
                m.fichero()?
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                assert_eq!(bytes.len() as u64, esperado);
            }
        }
        Ok((partes, bytes))
    }

    #[test]
    fn los_campos_y_el_fichero_salen_por_orden_con_los_limites_partidos() {
        // Bytes que se parecen a un límite sin serlo, dentro del fichero (el
        // límite entero no aparece nunca: el cliente lo elige así).
        let fichero: Vec<u8> = b"PK\x03\x04\r\n--Xy\r\n--XyY no es el limite\r\n-XyZ\r\n"
            .iter()
            .copied()
            .chain((0..=255u8).cycle().take(200_000))
            .collect();
        let cuerpo = formulario(
            &[
                (":action", "file_upload"),
                ("name", "Banco_Riesgo"),
                ("version", "1.0"),
            ],
            &fichero,
            b"\r\n--XyZ--\r\n",
        );
        for goteo in [1, 3, 7, 4096, 1 << 20] {
            let (partes, bytes) = leer(cuerpo.clone(), goteo).unwrap();
            assert_eq!(
                partes[..3],
                [
                    Parte::Campo {
                        nombre: ":action".into(),
                        valor: "file_upload".into()
                    },
                    Parte::Campo {
                        nombre: "name".into(),
                        valor: "Banco_Riesgo".into()
                    },
                    Parte::Campo {
                        nombre: "version".into(),
                        valor: "1.0".into()
                    },
                ]
            );
            assert!(
                matches!(&partes[3], Parte::Fichero { fichero, .. } if fichero == "a-1.0-py3-none-any.whl")
            );
            assert_eq!(bytes, fichero, "goteo {goteo}");
        }
    }

    #[test]
    fn un_campo_largo_se_corta_y_lo_demas_sigue() {
        let largo = "x".repeat(CAMPO_MAXIMO * 3);
        let cuerpo = formulario(
            &[("description", &largo), ("name", "a")],
            b"abc",
            b"\r\n--XyZ--\r\n",
        );
        let (partes, bytes) = leer(cuerpo, 999).unwrap();
        assert!(matches!(&partes[0], Parte::Campo { valor, .. } if valor.len() == CAMPO_MAXIMO));
        assert_eq!(
            partes[1],
            Parte::Campo {
                nombre: "name".into(),
                valor: "a".into()
            }
        );
        assert_eq!(bytes, b"abc");
    }

    #[test]
    fn si_detras_del_fichero_hay_otra_parte_el_fichero_no_sale_entero() {
        let cuerpo = formulario(
            &[("name", "a")],
            b"contenido",
            b"\r\n--XyZ\r\nContent-Disposition: form-data; name=\"tarde\"\r\n\r\nx\r\n--XyZ--\r\n",
        );
        let e = leer(cuerpo, 5).unwrap_err();
        assert!(e.contains("last part"), "{e}");
    }

    #[test]
    fn un_cuerpo_cortado_falla() {
        let mut cuerpo = formulario(&[("name", "a")], b"contenido", b"\r\n--XyZ--\r\n");
        let largo = cuerpo.len() as u64;
        cuerpo.truncate(cuerpo.len() - 6);
        let mut m = Multipart::nuevo(std::io::Cursor::new(cuerpo), TIPO, largo).unwrap();
        m.siguiente().unwrap();
        m.siguiente().unwrap();
        let mut v = Vec::new();
        assert!(m.fichero().unwrap().read_to_end(&mut v).is_err());
    }

    #[test]
    fn sin_boundary_no_es_multipart() {
        assert_eq!(
            limite_de("multipart/form-data; boundary=\"a b\""),
            Some("a b".into())
        );
        assert_eq!(limite_de("application/json"), None);
        assert_eq!(limite_de("multipart/form-data"), None);
        assert!(Multipart::nuevo(std::io::empty(), "text/plain", 0).is_err());
    }
}
