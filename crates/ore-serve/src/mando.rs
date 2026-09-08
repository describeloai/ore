//! Invocar a `ore`, y **sólo lo que un plano de control puede correr**.
//!
//! # La lista es de PERMITIDOS, y ése es todo el diseño
//!
//! La tentación es una lista de vetados: «`source catalog` no, `materialize`
//! tampoco». Está mal por la razón que este repositorio ya tiene escrita —**P4,
//! omitir es cerrar, no abrir**—: con una lista de vetados, el verbo que se
//! añada mañana llega **permitido**, y nadie tiene que decir nada para que eso
//! pase.
//!
//! Aquí un verbo nuevo llega **negado**, y quien lo abra tiene que escribir al
//! lado por qué es hermético. Es la misma denegación por defecto que
//! `ConduitPolicy`, aplicada al proceso en vez de al dato.
//!
//! # Qué se gana, medido y no prometido
//!
//! 12 de los 14 crates de este árbol no abren una conexión. Los dos que sí son
//! drivers, y viven **fuera** del binario `ore` —`ore-cli/tests/dependencias.rs`
//! lee el `Cargo.lock` y falla si una crate de red entra en su cierre—. Así que
//! los verbos de esta lista no es que prometan no salir: **no pueden**.
//!
//! ⇒ El servidor no necesita una credencial de ningún origen. Lo que toca el
//! mundo se va a un Job, con su identidad y su cuota, que es exactamente el
//! reparto que ya corre en `malla/94-flujo-completo.yaml`.
//!
//! # Y la lista es defensa en profundidad, no la puerta
//!
//! Las rutas construyen los argumentos ellas; **quien llama no elige el verbo**
//! en ningún sitio. Esta comprobación existe para el día en que una ruta nueva
//! se escriba distraída, no porque haya un camino por el que un cliente escoja
//! qué se ejecuta.

use std::path::Path;
use std::process::Command;

/// Lo que el plano de control PUEDE correr, con el motivo de cada permiso.
///
/// Se compara por **prefijo de argumentos**, y gana el más largo: `source add`
/// está y `source` a secas no, porque registrar una fuente y sondearla son dos
/// actos con fronteras de confianza distintas — y eso no lo decide esta lista,
/// lo dice `ore-cli/src/fuente.rs`: *«No abre un socket»*.
pub const HERMETICOS: &[(&[&str], &str)] = &[
    (&["--version"], "no hace nada: dice quién es"),
    (&["init"], "escribe el esqueleto y no abre nada"),
    (
        &["source", "add"],
        "registra la fuente y separa el secreto; NO la sondea",
    ),
    (
        &["discover"],
        "induce desde un catálogo YA LEÍDO, que le llega como fichero",
    ),
    (
        &["review"],
        "aplica decisiones que vienen de fuera; no consulta a nadie",
    ),
    (&["validate"], "lee el árbol y contesta"),
    (&["diff"], "compara dos árboles"),
    (&["view"], "compila una vista; el motor es aritmética"),
    (&["package", "new"], "escribe un manifiesto"),
    (&["package", "move"], "mueve documentos entre paquetes"),
    (
        &["package", "split"],
        "enumera componentes, y con `--to` mueve",
    ),
    (&["package", "merge"], "funde un paquete y deja la lápida"),
];

/// Lo que salió de correr un mando.
pub struct Salida {
    pub codigo: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Salida {
    pub fn bien(&self) -> bool {
        self.codigo == 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Negado {
    /// El verbo no está en la lista. **No se dice si existe**: que un verbo
    /// exista o no es información del binario, y esto contesta por la lista.
    FueraDeLaLista(String),
    /// El argumento trae algo que no puede viajar por un `argv`.
    ArgumentoImposible(String),
}

impl std::fmt::Display for Negado {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Negado::FueraDeLaLista(v) => write!(
                f,
                "`{v}` no está en la lista de verbos herméticos: si toca un origen, \
                 su sitio es un Job con la imagen de drivers"
            ),
            Negado::ArgumentoImposible(a) => {
                write!(f, "argumento con un carácter que no puede viajar: `{a}`")
            }
        }
    }
}

/// ¿Puede correr esto aquí? Devuelve el motivo del permiso, para poder decirlo.
pub fn permitido(args: &[String]) -> Result<&'static str, Negado> {
    for a in args {
        if a.contains('\0') || a.contains('\n') || a.contains('\r') {
            return Err(Negado::ArgumentoImposible(a.clone()));
        }
    }
    let mut mejor: Option<(usize, &'static str)> = None;
    for (prefijo, porque) in HERMETICOS {
        if args.len() >= prefijo.len()
            && args[..prefijo.len()]
                .iter()
                .zip(prefijo.iter())
                .all(|(a, p)| a == p)
        {
            let largo = prefijo.len();
            if mejor.is_none_or(|(l, _)| largo > l) {
                mejor = Some((largo, porque));
            }
        }
    }
    match mejor {
        Some((_, porque)) => Ok(porque),
        None => Err(Negado::FueraDeLaLista(
            args.first().cloned().unwrap_or_default(),
        )),
    }
}

/// Corre `ore` con estos argumentos, si la lista lo permite.
///
/// `raiz` es el directorio de trabajo, y va como tal y no como bandera: cada
/// verbo nombra su raíz de una manera —`--path`, un posicional, dos posicionales
/// en `diff`— y elegir por él aquí sería reimplementar su interfaz.
pub fn correr(binario: &Path, raiz: &Path, args: &[String]) -> Result<Salida, Negado> {
    permitido(args)?;
    let salida = Command::new(binario)
        .args(args)
        .current_dir(raiz)
        // El entorno se hereda a propósito: es de donde `connectionEnv` saca el
        // secreto de una fuente. Y da igual para los verbos de la lista, que no
        // lo usan — el que lo usaría es el driver, que corre en otra parte.
        .output()
        .map_err(|e| Negado::ArgumentoImposible(format!("no se pudo ejecutar `ore`: {e}")))?;
    Ok(Salida {
        codigo: salida.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&salida.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&salida.stderr).into_owned(),
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lo_hermetico_pasa() {
        assert!(permitido(&v(&["discover", "--from", "c.json"])).is_ok());
        assert!(permitido(&v(&["source", "add", "--name", "bq", "url"])).is_ok());
    }

    /// El corte fino: la fuente se registra aquí y se sondea en otra parte.
    #[test]
    fn sondear_un_origen_no_pasa() {
        assert!(permitido(&v(&["source", "catalog", "bq"])).is_err());
        assert!(permitido(&v(&["source", "check", "bq"])).is_err());
        assert!(permitido(&v(&["source", "explore", "bq"])).is_err());
    }

    /// P4: lo que no está, está negado. Un verbo que existe y no se listó
    /// **no pasa**, y ése es justo el caso que una lista de vetados fallaría.
    #[test]
    fn lo_que_no_esta_en_la_lista_se_niega() {
        for verbo in [
            "materialize",
            "drift-detect",
            "dev",
            "serve",
            "lock",
            "pack",
        ] {
            assert!(
                permitido(&v(&[verbo])).is_err(),
                "`{verbo}` no está en la lista y aun así pasó"
            );
        }
    }

    #[test]
    fn gana_el_prefijo_mas_largo() {
        // `package` a secas no está; `package new` sí.
        assert!(permitido(&v(&["package"])).is_err());
        assert!(permitido(&v(&["package", "new", "ventas"])).is_ok());
        assert!(permitido(&v(&["package", "publish"])).is_err());
    }

    #[test]
    fn un_argumento_con_salto_de_linea_no_viaja() {
        assert!(matches!(
            permitido(&v(&["discover", "--name", "a\nb"])),
            Err(Negado::ArgumentoImposible(_))
        ));
    }
}
