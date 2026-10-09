//! `ore-read-sftp` — el lector de un servidor SFTP de un cliente, **la familia
//! de los objetos** (ADR 0061 O4·2).
//!
//! Lo de todos los almacenes de objetos vive en `ore-read-objetos`; aquí queda
//! lo de SFTP: leer su URL (`sftp://usuario@host/ruta?huella=SHA256:…`, con la
//! clave de la celda en `ORE_SFTP_CLAVE`, D-O4), comprobar paso a paso que se
//! llega y se puede leer (`check`, `acceso.rs`), explorar sus carpetas, y
//! clasificar lo que contesta. El servidor es su [`ore_objetos::Origen`] por
//! `ore_sftp::Sftp`, y sólo para copiar: un SFTP no versiona (D-O1).
//!
//! La URL va por stdin como la de cualquier lector: puede llevar una
//! contraseña (el recurso de D-O4), que este programa no imprime.

mod acceso;

use ore_objetos::Origen;
use ore_read_objetos::driver::{self, Proveedor};
use ore_sftp::Sftp;

struct Ssh;

impl Proveedor for Ssh {
    const NOMBRE: &'static str = "ore-read-sftp";
    /// Un SFTP no fija nada (la versión es la copia en el lago), no firma, no
    /// da huella sin bajar el fichero, y la clave de la celda no caduca.
    const OBJETOS: ore_objetos::Capacidades = ore_objetos::Capacidades {
        fija: ore_objetos::Fija::Nada,
        firma: false,
        huella: None,
        credencial_corta: false,
    };
    /// La conexión vive lo que la fuente: `servir` la guarda entre peticiones.
    type Fuente = Sftp;

    fn leer(url: &str) -> Result<Sftp, String> {
        Sftp::de_url(url)
    }

    fn prefijo(s: &Sftp) -> &str {
        &s.fuente.prefijo
    }

    fn origen(s: &Sftp) -> Box<dyn Origen + Sync + '_> {
        Box::new(s)
    }

    fn comprobar(s: &Sftp) -> String {
        acceso::comprobar(s)
    }

    fn explorar(s: &Sftp) -> Result<String, String> {
        acceso::explorar(s)
    }

    /// Lo que el servidor dice (`SFTP 2` no está, `SFTP 3` sin permiso), y lo
    /// de entrar: la huella y la clave.
    fn fallo(m: String) -> ore_driver::Fallo {
        driver::clasificar(
            m,
            &["no deja entrar", "ORE_SFTP_CLAVE", "huella", "SFTP 3"],
            &["SFTP 2", "no está", "ya no está"],
        )
    }

    fn contadores() -> (usize, usize) {
        ore_sftp::contadores()
    }
}

fn main() -> std::process::ExitCode {
    driver::main::<Ssh>()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_driver::Codigo;

    #[test]
    fn lo_que_el_servidor_contesta_se_dice_en_el_codigo_del_contrato() {
        let c = |m: &str| Ssh::fallo(m.to_string()).codigo;
        assert_eq!(
            c("`ore@h` no deja entrar ([Session(-18)] Username/PublicKey combination invalid): …"),
            Codigo::Credencial
        );
        assert_eq!(
            c("la huella del host `h` es `SHA256:a` y la fijada `SHA256:b`: …"),
            Codigo::Credencial
        );
        assert_eq!(c("`datos/x.csv` no está (SFTP 2)"), Codigo::Objeto);
        assert_eq!(
            c("`h:22` no contesta (Connection refused (os error 111)): …"),
            Codigo::Conexion
        );
    }

    #[test]
    fn sus_capacidades_dicen_que_no_fija_nada() {
        assert_eq!(
            Ssh::OBJETOS.json(),
            r#"{"credencial_corta":false,"fija":"ninguna","firma":false,"huella":null}"#
        );
    }
}
