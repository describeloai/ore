//! **La huella de contenido de un objeto**, como la escribe la colección:
//! `<algoritmo>:<valor>`. Es la `checksum` de un `ObjectTable` (spec
//! `v1alpha16/02` §5), y la colección mantenida la coteja **mientras baja**
//! los bytes (0046 E8·2): lo que llega es lo que el manifiesto dice, o no
//! entra. Cada origen da la suya —S3 el CRC64NVME, GCS el CRC32C, Azure a veces
//! un MD5—; [`para`] da el cálculo que casa con una huella dada.
//!
//! **CRC64NVME** (S3, `FULL_OBJECT`, por defecto desde 2025): base64 del valor
//! en big-endian. Medido contra el bucket de F1: el calculado aquí coincide con
//! el de S3 en los 17 objetos y versiones del experimento. Polinomio
//! `0xAD93D23594C93659`, reflejado (`0x9A6C9329AC4BC9B5`), con registro inicial
//! y salida a unos.

const TABLA: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u64;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0x9A6C_9329_AC4B_C9B5 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// El CRC64NVME en flujo.
pub struct Crc64Nvme(u64);

impl Default for Crc64Nvme {
    fn default() -> Self {
        Crc64Nvme(!0)
    }
}

impl Crc64Nvme {
    pub fn sumar(&mut self, datos: &[u8]) {
        let mut c = self.0;
        for &b in datos {
            c = TABLA[((c ^ u64::from(b)) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }

    pub fn valor(&self) -> u64 {
        !self.0
    }

    /// Como lo dice S3: base64 de los 8 bytes en big-endian.
    pub fn base64(&self) -> String {
        base64(&self.valor().to_be_bytes())
    }
}

/// La huella de unos bytes, como la escribe la colección: `crc64nvme:<b64>`.
pub fn de(bytes: &[u8]) -> String {
    let mut c = Crc64Nvme::default();
    c.sumar(bytes);
    format!("crc64nvme:{}", c.base64())
}

/// **Una huella que se calcula mientras se baja**: la de un algoritmo.
pub trait Calculo {
    fn sumar(&mut self, datos: &[u8]);
    /// Como la escribe la colección: `<algoritmo>:<valor>`.
    fn texto(&self) -> String;
}

impl Calculo for Crc64Nvme {
    fn sumar(&mut self, datos: &[u8]) {
        Crc64Nvme::sumar(self, datos)
    }
    fn texto(&self) -> String {
        format!("crc64nvme:{}", self.base64())
    }
}

/// El cálculo que casa con `huella` (por su algoritmo), si se sabe hacer. Una
/// huella de un algoritmo que no, o un `etag:` —un validador, no una huella—,
/// no se coteja: `None`.
pub fn para(huella: &str) -> Option<Box<dyn Calculo>> {
    match huella.split_once(':') {
        Some(("crc64nvme", _)) => Some(Box::new(Crc64Nvme::default())),
        _ => None,
    }
}

/// Base64 estándar, como los orígenes dan sus checksums.
pub fn base64(b: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for t in b.chunks(3) {
        let n = ((t[0] as u32) << 16)
            | ((*t.get(1).unwrap_or(&0) as u32) << 8)
            | (*t.get(2).unwrap_or(&0) as u32);
        for i in 0..4 {
            if i <= t.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_crc64nvme_es_el_de_s3() {
        // El valor de comprobación del catálogo de CRC (CRC-64/NVME).
        let mut c = Crc64Nvme::default();
        c.sumar(b"1234");
        c.sumar(b"56789");
        assert_eq!(c.valor(), 0xAE8B_1486_0A79_9888);
        assert_eq!(de(b""), "crc64nvme:AAAAAAAAAAA=");
    }

    #[test]
    fn el_calculo_casa_con_la_huella_y_un_etag_no_se_coteja() {
        let mut c = para("crc64nvme:loquesea").expect("se sabe");
        c.sumar(b"123456789");
        assert_eq!(c.texto(), de(b"123456789"));
        assert!(para("etag:\"abc\"").is_none());
        assert!(para("crc32c:AAAA").is_none(), "todavía no");
    }
}
