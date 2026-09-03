//! **El sobre.** Lo decidió el
//! [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md):
//!
//! > Una copia es un artefacto: **un sobre nuestro alrededor de una carga en
//! > Parquet, nombrado por su digest**, inmutable.
//!
//! ```text
//! "ORECOPY1"        8 bytes
//! cabecera          JSON canónico, longitud + bytes
//!   plan            el digest del plan que esta copia contesta
//!   esquema         qué columnas produce, y de qué tipo
//!   testigo         { modo, valor } — hasta cuándo fue cierta
//!   conducto        cuál autorizó la copia
//!   bundle          contra qué compilación se construyó
//! carga             Parquet
//! ```
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
/// distinta sobre **la copia**, no sobre quien la consulta.
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
    /// Contra qué compilación se construyó.
    pub bundle: String,
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
            ("bundle", Json::s(&self.bundle)),
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

fn u32le(v: u32, out: &mut Vec<u8>) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// **El artefacto entero.** Determinista: la misma cabecera y la misma carga
/// dan los mismos bytes, y por tanto el mismo nombre.
pub fn sellar(c: &Cabecera, carga: &[u8]) -> Vec<u8> {
    let cab = c.jcs();
    let mut out = Vec::with_capacity(MAGIA.len() + 4 + cab.len() + carga.len());
    out.extend_from_slice(MAGIA);
    u32le(cab.len() as u32, &mut out);
    out.extend_from_slice(cab.as_bytes());
    out.extend_from_slice(carga);
    out
}

/// **El nombre es el contenido.**
///
/// De ahí salen tres cosas que no hay que programar: no hay carrera —dos
/// escritores que lleguen a la vez escriben los mismos bytes—, re-materializar
/// es idempotente, y ramificar sale gratis porque una rama nombra otro digest.
pub fn clave(artefacto: &[u8]) -> String {
    let d = ore_core::digest::de_bytes(artefacto);
    format!("ore/v1/{}", d.trim_start_matches("sha256:"))
}

/// **El recibo**, y por qué hace falta uno.
///
/// El [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md) dice
/// del paso 4 del ciclo: *«calcula `digest(plan, testigo)` y hace un `HEAD`. Si
/// está, termina aquí»*, y promete con ello **saber si hay que copiar sin copiar
/// nada**.
///
/// Eso **no se puede hacer** con el nombre del artefacto y nada más: el nombre
/// es el digest del artefacto **entero**, carga incluida, y para calcularlo hay
/// que haber leído ya todas las filas. El paso 4, tal y como estaba escrito,
/// ahorraba la subida y no la lectura — que es el trabajo caro.
///
/// El recibo lo arregla con un objeto minúsculo:
///
/// ```text
/// ore/v1/plan/<sha256 de la cabecera>   →   contiene la clave del artefacto
/// ```
///
/// La cabecera se conoce **antes** de leer una fila —plan, esquema, testigo,
/// conducto y bundle salen todos de la compilación— así que el `HEAD` de verdad
/// se hace aquí. Y **sigue sin haber puntero mutable**: el nombre del recibo
/// también es su contenido, y se escribe con `If-None-Match: *`.
///
/// # Lo que el recibo mide sin proponérselo
///
/// > **Es donde la promesa del testigo se pone a prueba.**
///
/// La cabecera determina el artefacto **solo si el origen es determinista dado
/// el testigo**. Si un testigo no fija de verdad el estado del origen, dos
/// materializaciones con la misma cabecera producen cargas distintas — y
/// entonces el mismo recibo apuntaría a dos artefactos. `If-None-Match` deja
/// ganar al primero, así que la discrepancia queda **detectable** en vez de
/// silenciosa: el segundo ve que el recibo apunta a otro sitio.
/// # Y por qué el plan va en la ruta, y no solo en el digest
///
/// Porque **agrupa**. `ore/v1/plan/<plan>/<cabecera>` permite enumerar por
/// prefijo todas las copias de un mismo plan, y sin eso no se puede contestar la
/// única pregunta que la recogida de basura necesita: *¿cuál de estas es la
/// vigente y cuáles quedaron atrás?*
///
/// No introduce ningún puntero mutable, que era la propiedad a no perder: los
/// dos segmentos siguen siendo contenido. Lo que añade es **un sitio donde
/// mirar**, que es exactamente lo que el registro hizo un piso más arriba.
pub fn recibo(c: &Cabecera) -> String {
    let d = ore_core::digest::de_bytes(c.jcs().as_bytes());
    format!(
        "{}/{}",
        prefijo_de_plan(&c.plan),
        d.trim_start_matches("sha256:")
    )
}

/// Dónde viven todos los recibos de un plan. Es lo que se enumera para recoger.
pub fn prefijo_de_plan(plan: &str) -> String {
    format!("ore/v1/plan/{}", plan.trim_start_matches("sha256:"))
}

/// Vuelve a abrirlo. Existe para que la prueba de ida y vuelta sea una prueba y
/// no una inspección a ojo, y para que quien lea una copia no tenga que adivinar
/// dónde acaba la cabecera.
///
/// **La mitad lectora del formato, y todavía no la llama nadie.** Se escribe
/// junto con la escritora a propósito: un formato cuyo lector se escribe meses
/// después se descubre ilegible meses después. Quien lea copias —el ejecutor,
/// cuando las sirva— entra por aquí.
#[cfg_attr(not(test), allow(dead_code))]
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
            bundle: "sha256:bbbb".into(),
        }
    }

    /// **G1, aquí.** Dos sellados de lo mismo dan los mismos bytes — y por tanto
    /// el mismo nombre. Sin esto, el almacén acumularía una copia por intento.
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
        c.bundle = "sha256:dddd".into();
        variantes.push(("bundle", c));

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
