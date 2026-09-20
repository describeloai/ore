//! **La cabecera de la copia, y el sobre heredado.**
//!
//! [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md)
//! decidió: *«una copia es un artefacto: un sobre nuestro alrededor de una carga
//! en Parquet, nombrado por su digest, inmutable»*. Desde W3.6a (0031 §10,
//! 2026-09-20) **la copia es un dataset** —una tabla Iceberg con su historia—
//! y el sobre ya no se escribe: lo que sigue vivo de aquí es **la cabecera**
//! ([`Cabecera`]: plan, esquema, testigo, clave, conducto), que ahora viaja
//! como propiedad del snapshot de la tabla, y [`abrir`], que sigue leyendo los
//! sobres `ORECOPY1` que queden en `ore/v1/` hasta que `recoger-huerfanas` los
//! retire.
//!
//! ```text
//! "ORECOPY1"        8 bytes
//! cabecera          JSON canónico, longitud + bytes
//!   plan            el digest del plan que esta copia contesta
//!   esquema         qué columnas produce, y de qué tipo
//!   testigo         { modo, valor } — hasta cuándo fue cierta
//!   conducto        cuál autorizó la copia
//! carga             Parquet
//! ```
//!
//! # Lo que NO va en la cabecera: el bundle (desde el 2026-09-18)
//!
//! Iba: *«contra qué compilación se construyó»*. Y era el digest del **árbol
//! entero** (‖ versión OOS ‖ lock), así que cualquier commit en cualquier
//! paquete —un alta, un catálogo, una retirada, hasta `ontology.config.yaml`—
//! cambiaba la cabecera de TODAS las copias y dejaba sin recibo a todas las
//! vistas: medido en `victor`, 16 de 19 commits, y `postgre_standard` (10
//! tablas) releída del origen en las tres pasadas del día sin que nada suyo
//! cambiara (`medida-lo-que-parece-roto.py` §4). Y la versión de OOS dentro
//! significaba que cada release releía todos los orígenes de todos los
//! inquilinos.
//!
//! La cabecera dice **qué contiene** la copia —plan, esquema, clave, testigo,
//! conducto— y eso ya nombra la compilación que importa: el plan ES la vista
//! compilada hasta su tabla. De qué árbol salió es procedencia, y la
//! procedencia va en el informe (`copias/<vista>.json`, campo `bundle`), que
//! vive en el árbol y se lee sin abrir el almacén.
//!
//! # Por qué el sobre es nuestro y la carga no
//!
//! El sobre lleva **las tres cosas que no lleva ningún formato**: qué plan
//! contesta, hasta cuándo fue cierta y quién la autorizó. Eso no cabe en un pie
//! de página de Parquet sin inventarse un convenio, y un convenio inventado es
//! un formato propio con peor prensa.
//!
//! La carga es Parquet porque el sobre no tiene por qué saber leer columnas — y
//! porque deja abierta la puerta a que algún día la escriba el origen.
//!
//! # La misma figura que `.oretopo`, con otra carga dentro
//!
//! Magia, cabeceras fuera del cuerpo, longitud + bytes, todo determinista. Es
//! [ADR 0006](../../../docs/decisions/0006-el-artefacto-de-topologia.md) otra
//! vez: **el mismo artefacto con dos cargas** — aristas en CSR allí, filas en
//! Parquet aquí.

use ore_core::json::Json;
use std::collections::BTreeMap;

pub const MAGIA: &[u8; 8] = b"ORECOPY1";

/// El JSON canónico de la cabecera, en una línea: lo que `ore` sella y lo que
/// `leer` devuelve. Es un tipo de texto porque va a un sitio de texto (la
/// propiedad `ore.cabecera` del snapshot).
pub type CabeceraJcs = String;

/// Hasta cuándo fue cierta. El vocabulario es el de `changes.witness` de OOS y
/// no se inventa otro: `none`, `snapshot`, `log`, `field`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Testigo {
    pub modo: String,
    /// El ordinal leído. `None` es una copia que no puede decir hasta cuándo fue
    /// cierta, y eso es legal y tiene precio: su frescura no se comprueba.
    pub valor: Option<String>,
}

/// Lo que va en la cabecera. Cinco campos, y los cinco contestan una pregunta
/// distinta sobre **la copia**, no sobre quien la consulta ni sobre el árbol
/// del que salió.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cabecera {
    /// El digest del plan que esta copia contesta. Es lo que el View Matcher
    /// necesita para decidir si sirve.
    pub plan: String,
    /// Columna → tipo, en el vocabulario de OOS. Ordenado, porque va al digest.
    pub esquema: BTreeMap<String, String>,
    pub testigo: Testigo,
    /// Qué columnas identifican una fila, si alguna. Es `changes.key` de la
    /// tabla, y va en la cabecera por dos motivos: hace la copia
    /// **autodescriptiva** —quien la lea sabe por qué se identifican sus filas—
    /// y es lo único que permite **fundir** un incremento con ella.
    ///
    /// Vacía significa que no hay con qué deduplicar, y entonces la copia solo
    /// se puede rehacer entera. Es la otra cara de `OOS2023`.
    pub clave: Vec<String>,
    /// El conducto que la autorizó. Sin él no se sabría bajo qué permiso
    /// existen estas filas fuera de su origen.
    pub conducto: String,
}

impl Cabecera {
    /// La cabecera como JSON canónico. **De estos bytes sale el digest**, así
    /// que aquí vive G1 igual que en el resto del proyecto.
    pub fn jcs(&self) -> String {
        let testigo = match &self.testigo.valor {
            Some(v) => Json::obj([("modo", Json::s(&self.testigo.modo)), ("valor", Json::s(v))]),
            // Se omite en vez de escribirse `null`: la forma canónica de este
            // proyecto no tiene nulos, y «sin poblar» ya lo dice la ausencia.
            None => Json::obj([("modo", Json::s(&self.testigo.modo))]),
        };
        Json::obj([
            ("clave", Json::Arr(self.clave.iter().map(Json::s).collect())),
            ("conducto", Json::s(&self.conducto)),
            (
                "esquema",
                Json::Obj(
                    self.esquema
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::s(v)))
                        .collect(),
                ),
            ),
            ("plan", Json::s(&self.plan)),
            ("testigo", testigo),
        ])
        .jcs()
    }
}

/// Abre un sobre heredado: la cabecera y la carga. Es la mitad lectora de un
/// formato que ya no se escribe, y se queda hasta que no quede ningún sobre en
/// ningún bucket (`recoger-huerfanas` los retira cuando ningún puntero los
/// nombra).
pub fn abrir(b: &[u8]) -> Result<(String, &[u8]), String> {
    if b.len() < MAGIA.len() + 4 || &b[..8] != MAGIA {
        return Err("no empieza por `ORECOPY1`: no es una copia de ORE".into());
    }
    let n = u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize;
    let fin = 12 + n;
    if b.len() < fin {
        return Err(format!(
            "la cabecera dice {n} bytes y el fichero solo tiene {}",
            b.len() - 12
        ));
    }
    let cab = String::from_utf8(b[12..fin].to_vec()).map_err(|_| "la cabecera no es UTF-8")?;
    Ok((cab, &b[fin..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32le(v: u32, out: &mut Vec<u8>) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    /// El sobre, tal como se escribía hasta W3.6a: para probar que `abrir`
    /// sigue leyéndolo.
    fn sellar(c: &Cabecera, carga: &[u8]) -> Vec<u8> {
        let cab = c.jcs();
        let mut out = Vec::with_capacity(MAGIA.len() + 4 + cab.len() + carga.len());
        out.extend_from_slice(MAGIA);
        u32le(cab.len() as u32, &mut out);
        out.extend_from_slice(cab.as_bytes());
        out.extend_from_slice(carga);
        out
    }

    fn clave(artefacto: &[u8]) -> String {
        let d = ore_core::digest::de_bytes(artefacto);
        format!("ore/v1/{}", d.trim_start_matches("sha256:"))
    }

    fn cabecera() -> Cabecera {
        Cabecera {
            plan: "sha256:aaaa".into(),
            esquema: [
                ("id".to_string(), "String".to_string()),
                ("total".to_string(), "Decimal".to_string()),
            ]
            .into(),
            testigo: Testigo {
                modo: "log".into(),
                valor: Some("1234".into()),
            },
            clave: vec!["id".into()],
            conducto: "materialization.payload".into(),
        }
    }

    /// **G1, aquí.** Dos cabeceras de lo mismo dan los mismos bytes: es lo que
    /// hace que `ore` pueda comparar la del puntero con la que construye y
    /// decir «ya está» sin leer una fila.
    #[test]
    fn dos_sellados_de_lo_mismo_dan_los_mismos_bytes() {
        let a = sellar(&cabecera(), b"carga");
        let b = sellar(&cabecera(), b"carga");
        assert_eq!(a, b);
        assert_eq!(clave(&a), clave(&b));
    }

    /// Y cambiar **cualquiera** de las cinco cosas de la cabecera cambia el
    /// nombre. Es lo que hace que re-materializar con el mismo testigo no suba
    /// nada y con otro testigo suba otra copia.
    #[test]
    fn cambiar_una_cosa_de_la_cabecera_cambia_el_nombre() {
        let base = clave(&sellar(&cabecera(), b"carga"));
        let mut variantes = Vec::new();

        let mut c = cabecera();
        c.plan = "sha256:cccc".into();
        variantes.push(("plan", c));

        let mut c = cabecera();
        c.esquema.insert("pais".into(), "String".into());
        variantes.push(("esquema", c));

        let mut c = cabecera();
        c.testigo.valor = Some("1235".into());
        variantes.push(("testigo", c));

        let mut c = cabecera();
        c.conducto = "otro".into();
        variantes.push(("conducto", c));

        let mut c = cabecera();
        c.clave = vec!["id".into(), "pais".into()];
        variantes.push(("clave", c));

        for (que, c) in variantes {
            assert_ne!(
                clave(&sellar(&c, b"carga")),
                base,
                "cambiar `{que}` tiene que cambiar el nombre"
            );
        }
        // Y la carga, obviamente.
        assert_ne!(clave(&sellar(&cabecera(), b"otra")), base);
    }

    /// Un testigo sin valor **se omite** en vez de escribirse `null`. La forma
    /// canónica de este proyecto no tiene nulos, y una copia sin poblar y una
    /// copia con el valor vacío no pueden llamarse igual.
    #[test]
    fn el_testigo_sin_valor_se_omite_y_no_es_el_valor_vacio() {
        let mut sin = cabecera();
        sin.testigo.valor = None;
        assert!(!sin.jcs().contains("valor"), "{}", sin.jcs());

        let mut vacio = cabecera();
        vacio.testigo.valor = Some(String::new());
        assert_ne!(clave(&sellar(&sin, b"c")), clave(&sellar(&vacio, b"c")));
    }

    /// Ida y vuelta: lo que se sella se vuelve a abrir, y la carga sale entera.
    #[test]
    fn se_abre_lo_que_se_sella() {
        let carga = b"parquet ira aqui".to_vec();
        let bytes = sellar(&cabecera(), &carga);
        let (cab, salida) = abrir(&bytes).expect("abre");
        assert_eq!(cab, cabecera().jcs());
        assert_eq!(salida, &carga[..]);
    }

    /// Y lo que no es una copia se rechaza por la magia, no por el nombre del
    /// fichero: renombrar es exactamente lo que haría quien se equivoca.
    #[test]
    fn lo_que_no_lleva_la_magia_no_se_abre() {
        assert!(abrir(b"ORETOPO1xxxxxxxx").is_err());
        assert!(abrir(b"corto").is_err());
    }
}
