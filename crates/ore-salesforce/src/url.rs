//! **La URL de una fuente Salesforce**:
//!
//! ```text
//! salesforce://<dominio de la org>/?cliente=<Consumer Key>&usuario=<usuario>[&version=68.0][&objetos=Account,Contact]
//! ```
//!
//! La *Consumer Key* y el usuario no son secretos: la Consumer Key identifica la
//! app y no da acceso por sí sola.
//!
//! **La clave privada del JWT** sí lo es, y llega de uno de dos sitios:
//!
//! - `&clave=<PEM, con %XX>` **en la URL**, que es como la guarda el custodio
//!   (la `conexion` de un Job): como la contraseña de una cadena de Postgres o de
//!   Mongo. Por eso `ore_driver::fallo` la tapa en cualquier mensaje (`clave` está
//!   en la lista de parámetros secretos) y esta URL **no viaja en claro por la
//!   cola** (`ore-serve::cola::url_sin_secreto` no la admite);
//! - o por el entorno, `ORE_SALESFORCE_CLAVE` (el PEM) o
//!   `ORE_SALESFORCE_CLAVE_FICHERO` (su ruta): para el laboratorio y los tests.

use ore_driver::url::decodificar;

/// La versión de la API con que se midió el X0.
pub const VERSION: &str = "68.0";

/// Para pruebas y para el simulador: sustituye el sitio al que se llama
/// (`https://host`), y con él se salta la lista de dominios.
pub const ENV_ENDPOINT: &str = "ORE_SALESFORCE_ENDPOINT";

#[derive(Clone, PartialEq, Eq)]
pub struct Fuente {
    /// El dominio de la org (My Domain) o `login.salesforce.com`.
    pub host: String,
    /// La *Consumer Key* de la External Client App.
    pub cliente: String,
    /// El usuario con el que se firma (`sub` del JWT).
    pub usuario: String,
    pub version: String,
    /// Los objetos que `catalogo` recorre. Vacío: el núcleo de CRM y los
    /// objetos propios (ver [`crate::verbos`]).
    pub objetos: Vec<String>,
    /// La clave privada (PEM) si viaja en la URL, que es como la guarda el
    /// custodio (como la contraseña de una cadena de Postgres o de Mongo). Si
    /// no, llega por el entorno ([`crate::identidad::clave_del_entorno`]).
    pub clave: Option<String>,
}

/// **Sin la clave**: un `{:?}` de una `Fuente` en un log o en un `panic` no la
/// puede sacar.
impl std::fmt::Debug for Fuente {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fuente")
            .field("host", &self.host)
            .field("cliente", &self.cliente)
            .field("usuario", &self.usuario)
            .field("version", &self.version)
            .field("objetos", &self.objetos)
            .field("clave", &self.clave.as_ref().map(|_| "<oculta>"))
            .finish()
    }
}

impl Fuente {
    /// Adónde se llama: `https://<host>`, o el de [`ENV_ENDPOINT`].
    pub fn endpoint(&self) -> String {
        match std::env::var(ENV_ENDPOINT) {
            Ok(e) if !e.is_empty() => e.trim_end_matches('/').to_string(),
            _ => format!("https://{}", self.host),
        }
    }

    /// El `aud` del JWT: el sitio al que se canjea.
    pub fn audiencia(&self) -> String {
        self.endpoint()
    }
}

/// Los dominios a los que se manda el JWT. Un JWT firmado es una credencial
/// de una hora hacia **quien lo reciba**: una URL con otro dominio no se sigue.
fn dominio_valido(h: &str) -> bool {
    let h = h.to_ascii_lowercase();
    let bien_formado = !h.is_empty()
        && h.split('.')
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    bien_formado
        && [".salesforce.com", ".force.com", ".salesforce-setup.com"]
            .iter()
            .any(|suf| h.ends_with(suf))
}

pub fn leer(url: &str) -> Result<Fuente, String> {
    let resto = url.strip_prefix("salesforce://").ok_or_else(|| {
        format!(
            "`{}` no es una URL `salesforce://`",
            ore_driver::url::publica(url, "salesforce")
        )
    })?;
    let (host, consulta) = match resto.split_once('?') {
        Some((h, c)) => (h.trim_end_matches('/'), c),
        None => (resto.trim_end_matches('/'), ""),
    };
    let host = host.split('/').next().unwrap_or("");
    let permitido = std::env::var(ENV_ENDPOINT).is_ok_and(|e| !e.is_empty());
    if !permitido && !dominio_valido(host) {
        return Err(format!(
            "`{host}` no es un dominio de Salesforce (`*.salesforce.com`, `*.force.com`): no se le manda un JWT firmado"
        ));
    }
    let (mut cliente, mut usuario, mut version, mut objetos, mut clave) =
        (None, None, VERSION.to_string(), Vec::new(), None);
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        let v = decodificar(v)?;
        match k {
            "cliente" => cliente = Some(v),
            "clave" => clave = Some(v).filter(|c| !c.trim().is_empty()),
            "usuario" => usuario = Some(v),
            "version" => {
                let ok = v.split_once('.').is_some_and(|(a, b)| {
                    !a.is_empty()
                        && !b.is_empty()
                        && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
                });
                if !ok {
                    return Err(format!("`version={v}`: se espera `68.0`"));
                }
                version = v;
            }
            "objetos" => {
                objetos = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            otro => {
                return Err(format!(
                    "la URL no entiende `{otro}` (cliente, usuario, clave, version, objetos)"
                ));
            }
        }
    }
    Ok(Fuente {
        host: host.to_string(),
        cliente: cliente
            .filter(|c| !c.is_empty())
            .ok_or("a la URL le falta `cliente` (la Consumer Key)")?,
        usuario: usuario
            .filter(|c| !c.is_empty())
            .ok_or("a la URL le falta `usuario`")?,
        version,
        objetos,
        clave,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const BUENA: &str = "salesforce://orgfarm-1-dev-ed.develop.my.salesforce.com/?cliente=3MVG9abc&usuario=victor%40x.com";

    #[test]
    fn lee_lo_que_hay() {
        let f = leer(BUENA).unwrap();
        assert_eq!(f.host, "orgfarm-1-dev-ed.develop.my.salesforce.com");
        assert_eq!(f.cliente, "3MVG9abc");
        assert_eq!(f.usuario, "victor@x.com");
        assert_eq!(f.version, VERSION);
        assert!(f.objetos.is_empty());
    }

    /// El PEM viaja en la URL con `%XX` (es como lo guarda el custodio) y se
    /// descodifica entero, con sus saltos de línea.
    #[test]
    fn la_clave_privada_viaja_en_la_url_y_no_sale_en_el_debug() {
        let pem =
            "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0B\n-----END PRIVATE KEY-----\n";
        let cod: String = pem
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect();
        let f = leer(&format!("{BUENA}&clave={cod}")).unwrap();
        assert_eq!(f.clave.as_deref(), Some(pem));
        let d = format!("{f:?}");
        assert!(!d.contains("MIIEv") && d.contains("<oculta>"), "{d}");
        // Sin clave en la URL, no hay clave: la pone el entorno.
        assert_eq!(leer(BUENA).unwrap().clave, None);
        assert_eq!(leer(&format!("{BUENA}&clave=")).unwrap().clave, None);
    }

    #[test]
    fn los_objetos_y_la_version_se_leen() {
        let f = leer(&format!("{BUENA}&version=67.0&objetos=Account, Contact")).unwrap();
        assert_eq!(f.version, "67.0");
        assert_eq!(f.objetos, ["Account", "Contact"]);
    }

    /// **Un JWT firmado no se manda a cualquier sitio.**
    #[test]
    fn un_dominio_ajeno_no_recibe_el_jwt() {
        for malo in [
            "salesforce://evil.example.com/?cliente=a&usuario=b",
            "salesforce://salesforce.com.evil.io/?cliente=a&usuario=b",
            "salesforce://x.salesforce.com@evil.io/?cliente=a&usuario=b",
            "salesforce://localhost/?cliente=a&usuario=b",
        ] {
            let e = leer(malo).unwrap_err();
            assert!(e.contains("no es un dominio de Salesforce"), "{malo}: {e}");
        }
    }

    #[test]
    fn faltan_los_datos_se_dice_cuales() {
        assert!(
            leer("salesforce://a.my.salesforce.com/?usuario=u")
                .unwrap_err()
                .contains("`cliente`")
        );
        assert!(
            leer("salesforce://a.my.salesforce.com/?cliente=c")
                .unwrap_err()
                .contains("`usuario`")
        );
        assert!(
            leer("https://a.my.salesforce.com/?cliente=c&usuario=u")
                .unwrap_err()
                .contains("salesforce://")
        );
        assert!(
            leer(&format!("{BUENA}&nada=1"))
                .unwrap_err()
                .contains("`nada`")
        );
        assert!(
            leer(&format!("{BUENA}&version=abc"))
                .unwrap_err()
                .contains("68.0")
        );
    }
}
