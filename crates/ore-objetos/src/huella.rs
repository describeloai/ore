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

/// **CRC-32C** (Castagnoli), el que GCS da de cada objeto (`crc32c`, base64
/// del valor en big-endian), siempre, también de los compuestos (ADR 0061 O2).
/// Polinomio reflejado `0x82F63B78`, registro inicial y salida a unos.
pub struct Crc32c(u32);

const TABLA_32C: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0x82F6_3B78 ^ (c >> 1)
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

impl Default for Crc32c {
    fn default() -> Self {
        Crc32c(!0)
    }
}

impl Crc32c {
    pub fn valor(&self) -> u32 {
        !self.0
    }
}

impl Calculo for Crc32c {
    fn sumar(&mut self, datos: &[u8]) {
        let mut c = self.0;
        for &b in datos {
            c = TABLA_32C[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }
    fn texto(&self) -> String {
        format!("crc32c:{}", base64(&self.valor().to_be_bytes()))
    }
}

/// **MD5**, el `Content-MD5` que Azure guarda de un blob subido de una vez
/// (Put Blob lo calcula siempre; Put Block List sólo si el cliente lo dio):
/// base64 de los 16 bytes (ADR 0061 O3). No es una defensa contra quien
/// fabrique una colisión —eso lo hace el sha256 de la copia—, sino el cotejo
/// de que lo que llega es lo que el origen guardó.
#[derive(Default)]
pub struct Md5(md5::Md5);

impl Calculo for Md5 {
    fn sumar(&mut self, datos: &[u8]) {
        md5::Digest::update(&mut self.0, datos);
    }
    fn texto(&self) -> String {
        format!("md5:{}", base64(&md5::Digest::finalize(self.0.clone())))
    }
}

/// **quickXorHash**, la única huella que SharePoint y OneDrive dan siempre
/// (`file.hashes.quickXorHash`, ADR 0061 O5): un registro circular de 160
/// bits en el que el byte `i` se suma (XOR) desplazado `11·i` bits, y al final
/// la longitud (64 bits, little-endian) sobre los últimos 8 bytes; base64 de
/// los 20. Del algoritmo de Microsoft (`code-snippets/quickxorhash`),
/// cotejado con los vectores de rclone (`quickxor-vectores.txt`).
#[derive(Default, Clone)]
pub struct QuickXor {
    registro: [u8; 20],
    /// Bytes sumados: la posición del siguiente y la longitud del final.
    largo: u64,
}

impl Calculo for QuickXor {
    fn sumar(&mut self, datos: &[u8]) {
        // El desplazamiento de un byte se repite cada 160 bytes (11·160 es
        // múltiplo de 160): se lleva en módulo para no multiplicar en u64.
        let mut bit = ((self.largo % 160) * 11 % 160) as usize;
        for &b in datos {
            let (i, d) = (bit / 8, bit % 8);
            self.registro[i] ^= b << d;
            if d != 0 {
                self.registro[(i + 1) % 20] ^= b >> (8 - d);
            }
            bit = (bit + 11) % 160;
        }
        self.largo += datos.len() as u64;
    }
    fn texto(&self) -> String {
        let mut r = self.registro;
        for (i, b) in self.largo.to_le_bytes().iter().enumerate() {
            r[12 + i] ^= b;
        }
        format!("quickxor:{}", base64(&r))
    }
}

/// El cálculo que casa con `huella` (por su algoritmo), si se sabe hacer. Una
/// huella de un algoritmo que no, o un `etag:` —un validador, no una huella—,
/// no se coteja: `None`.
pub fn para(huella: &str) -> Option<Box<dyn Calculo>> {
    match huella.split_once(':') {
        Some(("crc64nvme", _)) => Some(Box::new(Crc64Nvme::default())),
        Some(("crc32c", _)) => Some(Box::new(Crc32c::default())),
        Some(("md5", _)) => Some(Box::new(Md5::default())),
        Some(("quickxor", _)) => Some(Box::new(QuickXor::default())),
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

    /// El valor de comprobación del catálogo de CRC (CRC-32C) y el de GCS.
    #[test]
    fn el_crc32c_es_el_de_gcs() {
        let mut c = Crc32c::default();
        Calculo::sumar(&mut c, b"1234");
        Calculo::sumar(&mut c, b"56789");
        assert_eq!(c.valor(), 0xE306_9283);
        assert_eq!(c.texto(), "crc32c:4waSgw==");
        let mut z = para("crc32c:x").expect("se sabe");
        z.sumar(&[0u8; 32]);
        assert_eq!(
            z.texto(),
            format!("crc32c:{}", base64(&0x8A91_36AAu32.to_be_bytes()))
        );
    }

    /// El MD5 de RFC 1321 y el `Content-MD5` que Azurite dio (O3·0).
    #[test]
    fn el_md5_es_el_de_azure() {
        let mut c = para("md5:x").expect("se sabe");
        c.sumar(
            b"%PDF-1.4
% UNO",
        );
        c.sumar(
            b"
%%EOF
",
        );
        assert_eq!(c.texto(), "md5:CInfjbZ21DfOIWDhgYr6dw==");
        let mut z = Md5::default();
        z.sumar(b"abc");
        assert_eq!(
            z.texto(),
            format!(
                "md5:{}",
                base64(&[
                    0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28,
                    0xe1, 0x7f, 0x72
                ])
            )
        );
    }

    /// Los 70 vectores de rclone, enteros y a trozos (que el desplazamiento
    /// siga entre llamadas, y pasado el registro de 160 bytes).
    #[test]
    fn el_quickxor_es_el_de_sharepoint() {
        let vectores = include_str!("quickxor-vectores.txt");
        let mut n = 0;
        for l in vectores.lines().filter(|l| !l.starts_with('#')) {
            let c: Vec<&str> = l.split(' ').collect();
            let datos: Vec<u8> = if c[1] == "-" {
                vec![]
            } else {
                (0..c[1].len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&c[1][i..i + 2], 16).unwrap())
                    .collect()
            };
            assert_eq!(datos.len(), c[0].parse::<usize>().unwrap());
            let quiere = format!("quickxor:{}", c[2]);
            for trozo in [1, 7, 64, 161, 1000] {
                let mut q = para("quickxor:x").expect("se sabe");
                for t in datos.chunks(trozo) {
                    q.sumar(t);
                }
                assert_eq!(q.texto(), quiere, "{} bytes a trozos de {trozo}", c[0]);
            }
            n += 1;
        }
        assert_eq!(n, 70);
    }

    #[test]
    fn el_calculo_casa_con_la_huella_y_un_etag_no_se_coteja() {
        let mut c = para("crc64nvme:loquesea").expect("se sabe");
        c.sumar(b"123456789");
        assert_eq!(c.texto(), de(b"123456789"));
        assert!(para("etag:\"abc\"").is_none());
        assert!(para("md5:AAAA").is_some(), "Azure, desde O3·1");
        assert!(para("sha1:AAAA").is_none());
    }
}
