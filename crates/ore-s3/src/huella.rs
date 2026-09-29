//! **La huella de contenido de S3**: el CRC64NVME (`FULL_OBJECT`) que S3
//! calcula de cada objeto desde 2025 y devuelve con `x-amz-checksum-mode`, en
//! base64 del valor en big-endian. Es la `checksum` de un `ObjectTable` (spec
//! `v1alpha16/02` §5), y la colección mantenida la coteja **mientras baja**
//! los bytes (0046 E8·2): lo que llega es lo que el manifiesto dice, o no
//! entra. Medido contra el bucket de F1: el calculado aquí coincide con el de
//! S3 en los 17 objetos y versiones del experimento.
//!
//! CRC-64/NVME: polinomio `0xAD93D23594C93659`, reflejado (`0x9A6C9329AC4BC9B5`),
//! con registro inicial y salida a unos.

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
        crate::base64(&self.valor().to_be_bytes())
    }
}

/// La huella de unos bytes, como la escribe la colección: `crc64nvme:<b64>`.
pub fn de(bytes: &[u8]) -> String {
    let mut c = Crc64Nvme::default();
    c.sumar(bytes);
    format!("crc64nvme:{}", c.base64())
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
}
