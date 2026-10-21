//! **SASL/GSSAPI (Kerberos)** para el RPC de HBase (ficha 0072, N6). El protocolo del apreton y del envoltorio esta en
//! `rpc.rs` y no sabe de Kerberos: habla con un [`Contexto`]. El de verdad es la GSSAPI del sistema (`libgssapi`, detras
//! del *feature* `kerberos`: pide `libgssapi_krb5` y se enlaza dinamicamente); el de las pruebas es un cifrado de juguete,
//! y asi los bytes del protocolo se comprueban sin un KDC.

use crate::error::Resultado;

/// La capa de proteccion que el servidor ofrece (`hbase.rpc.protection`) y el cliente acepta (RFC 4752).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capa {
    /// `authentication`: solo el apretón; los mensajes van en claro.
    Nada = 1,
    /// `integrity`: cada mensaje va firmado.
    Integridad = 2,
    /// `privacy`: cada mensaje va cifrado.
    Confidencialidad = 4,
}

impl Capa {
    /// La mejor de las que el servidor ofrece (`oferta` es el byte de capas: 1, 2 y 4 sumados).
    pub fn de_oferta(oferta: u8) -> Option<Capa> {
        [Capa::Confidencialidad, Capa::Integridad, Capa::Nada]
            .into_iter()
            .find(|c| oferta & (*c as u8) != 0)
    }

    /// ¿Hay que envolver los mensajes?
    pub fn envuelve(self) -> bool {
        self != Capa::Nada
    }
}

/// Un contexto de seguridad ya iniciado por el cliente.
pub trait Contexto {
    /// Un paso del apretón: el reto del servidor (o nada, el primero) y lo que se le envía (nada si no hay).
    fn paso(&mut self, reto: Option<&[u8]>) -> Resultado<Option<Vec<u8>>>;
    /// ¿Se completó el contexto?
    fn completo(&self) -> bool;
    /// Firma (`confidencial = false`) o cifra (`true`) un mensaje.
    fn envolver(&mut self, confidencial: bool, datos: &[u8]) -> Resultado<Vec<u8>>;
    /// Lo contrario.
    fn desenvolver(&mut self, datos: &[u8]) -> Resultado<Vec<u8>>;
}

/// Lo que la URL (o quien llama) dice de Kerberos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kerberos {
    /// El principal del cliente; sin él, el del almacén de credenciales por defecto.
    pub principal: Option<String>,
    /// El *keytab* del cliente; sin él, la caché de tickets (`kinit`) o el keytab por defecto de la GSSAPI.
    pub keytab: Option<String>,
    /// El nombre del servicio de HBase en el KDC: `hbase`, de `hbase/<host>@REINO`.
    pub servicio: String,
}

impl Default for Kerberos {
    fn default() -> Self {
        Kerberos {
            principal: None,
            keytab: None,
            servicio: "hbase".into(),
        }
    }
}

impl Kerberos {
    /// El usuario que va en la cabecera de conexion: el principal sin la instancia ni el reino.
    pub fn usuario(&self) -> Option<String> {
        let p = self.principal.as_deref()?;
        Some(p.split(['/', '@']).next().unwrap_or(p).to_string())
    }
}

/// El contexto de Kerberos de `host`. Sin el *feature* `kerberos` es un error que lo dice.
pub fn contexto(host: &str, k: &Kerberos) -> Resultado<Box<dyn Contexto>> {
    #[cfg(feature = "kerberos")]
    {
        crate::gss::nuevo(host, k)
    }
    #[cfg(not(feature = "kerberos"))]
    {
        let _ = (host, k);
        Err(crate::error::Error::Peticion(
            "este binario se compilo sin Kerberos (el feature `kerberos` de ore-hbase-rpc): \
             la imagen de los drivers lo trae; una compilacion local necesita krb5"
                .into(),
        ))
    }
}

#[cfg(test)]
pub mod prueba {
    //! Un cifrado de juguete con la forma de la GSSAPI: un reto, un contexto completo, y mensajes envueltos con una
    //! marca y un OR exclusivo. Basta para comprobar los bytes del protocolo.

    use super::*;
    use crate::error::Error;

    pub const MARCA_FIRMA: u8 = 0xA5;
    pub const MARCA_CIFRADO: u8 = 0x5A;

    pub fn tapar(confidencial: bool, datos: &[u8]) -> Vec<u8> {
        let (m, x) = if confidencial {
            (MARCA_CIFRADO, 0x11)
        } else {
            (MARCA_FIRMA, 0)
        };
        let mut v = vec![m];
        v.extend(datos.iter().map(|b| b ^ x));
        v
    }

    pub fn destapar(datos: &[u8]) -> Option<Vec<u8>> {
        let x = match *datos.first()? {
            MARCA_FIRMA => 0,
            MARCA_CIFRADO => 0x11,
            _ => return None,
        };
        Some(datos[1..].iter().map(|b| b ^ x).collect())
    }

    /// Dos pasos: el primer token y, con el reto, el contexto queda completo sin nada que enviar.
    #[derive(Default)]
    pub struct Juguete {
        pasos: u32,
    }

    impl Contexto for Juguete {
        fn paso(&mut self, reto: Option<&[u8]>) -> Resultado<Option<Vec<u8>>> {
            self.pasos += 1;
            match (self.pasos, reto) {
                (1, None) => Ok(Some(b"TOKEN-1".to_vec())),
                (2, Some(b"RETO")) => Ok(None),
                _ => Err(Error::Protocolo("paso de apretón inesperado".into())),
            }
        }
        fn completo(&self) -> bool {
            self.pasos >= 2
        }
        fn envolver(&mut self, confidencial: bool, datos: &[u8]) -> Resultado<Vec<u8>> {
            Ok(tapar(confidencial, datos))
        }
        fn desenvolver(&mut self, datos: &[u8]) -> Resultado<Vec<u8>> {
            destapar(datos).ok_or_else(|| Error::Protocolo("mensaje mal envuelto".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn de_las_capas_ofrecidas_se_elige_la_mas_fuerte() {
        assert_eq!(Capa::de_oferta(1), Some(Capa::Nada));
        assert_eq!(Capa::de_oferta(2), Some(Capa::Integridad));
        assert_eq!(Capa::de_oferta(4), Some(Capa::Confidencialidad));
        assert_eq!(Capa::de_oferta(7), Some(Capa::Confidencialidad));
        assert_eq!(Capa::de_oferta(3), Some(Capa::Integridad));
        assert_eq!(Capa::de_oferta(0), None);
        assert!(!Capa::Nada.envuelve() && Capa::Integridad.envuelve());
    }

    #[test]
    fn el_usuario_es_el_principal_sin_instancia_ni_reino() {
        let u = |p: &str| {
            Kerberos {
                principal: Some(p.into()),
                ..Kerberos::default()
            }
            .usuario()
        };
        assert_eq!(u("lector@ORE.TEST").as_deref(), Some("lector"));
        assert_eq!(u("etl/host.ore.test@ORE.TEST").as_deref(), Some("etl"));
        assert_eq!(u("solo").as_deref(), Some("solo"));
        assert_eq!(Kerberos::default().usuario(), None);
    }

    #[cfg(not(feature = "kerberos"))]
    #[test]
    fn sin_el_feature_se_dice_que_falta_y_no_se_cae() {
        let e = contexto("hb.ore.test", &Kerberos::default()).err().unwrap();
        assert!(e.to_string().contains("kerberos"), "{e}");
        assert_eq!(e.clase(), crate::Clase::Fatal);
    }
}
