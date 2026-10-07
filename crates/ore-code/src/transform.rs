//! **El productor de un dataset** (ORE 0055, OOS v1alpha25 `01`): lo que un
//! `@transform` de Python —o una sentencia SQL que escribe— declara leer y
//! escribir, y el documento `Transform` que se deriva de ello.
//!
//! Lo de Python se lee aquí (`python::derivar` llena [`Derivacion::transforms`]);
//! lo de SQL, en `ore-core`, que es quien analiza un guion: los dos dan una
//! [`Produccion`], y de ella sale el mismo documento.
//!
//! El documento es **determinista e idempotente**, como el de una función
//! ([`crate::emitir`]): la misma producción da los mismos bytes, siempre. La
//! coherencia (`OOS2013`) no compara bytes: compara campo a campo.
//!
//! [`Derivacion::transforms`]: crate::Derivacion::transforms

use crate::emitir::escalar;
use crate::firma::{Fallo, Rango};
use std::fmt::Write;

/// La versión que deriva un `Transform`: la que trae el `kind`.
pub const API_VERSION: &str = "oos.dev/v1alpha25";

/// La primera línea de un documento derivado. Es la marca que deja a la
/// herramienta borrar el de un transform que ya no existe sin tocar nunca otro
/// fichero (v1alpha25 `01` §9).
pub const MARCA: &str = "# derivado por ore desde";

/// Lo que un transform declara, tal como el código lo da (v1alpha25 `01` §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Produccion {
    /// `python` o `sql`.
    pub runtime: &'static str,
    /// `<ruta>.py:<def>` o `<ruta>.sql:<n>`, desde la carpeta del paquete.
    pub entrypoint: String,
    /// Python: la primera línea no vacía de la docstring. SQL: nunca.
    pub descripcion: Option<String>,
    /// Lo que lee, en su orden y sin repetir.
    pub inputs: Vec<String>,
    /// Lo que escribe, como lo escribe el código.
    pub output: String,
}

impl Produccion {
    /// `metadata.name`: la salida con cada `.` cambiado por `__` (§5.3). No es
    /// un nombre que nadie elija.
    pub fn nombre(&self) -> String {
        self.output.replace('.', "__")
    }
}

/// La forma corta de un nombre del árbol (v1alpha13): `p.default.n` es `p.n`;
/// lo demás no cambia. Es la que va al documento (`01` §4).
pub fn corto(nombre: &str) -> String {
    let partes: Vec<&str> = nombre.split('.').collect();
    match partes.as_slice() {
        [p, "default", n] => format!("{p}.{n}"),
        _ => nombre.to_string(),
    }
}

/// Un `@transform` del nivel superior: lo que declara, o todo lo que impide
/// leerlo sin ejecutar (`OOS2043`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transform {
    /// El nombre del `def`.
    pub nombre: String,
    /// El del `def`, para señalarlo.
    pub rango: Rango,
    pub resultado: Result<Produccion, Vec<Fallo>>,
    /// Dónde dice el código lo que declara (0055): un diagnóstico de lo que
    /// resuelve —una entrada que no es nada, una salida sin base— apunta aquí
    /// y no al documento derivado, que nadie escribe.
    pub sitios: Sitios,
}

/// Dónde está, en el fuente, cada cosa que un `@transform` declara.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sitios {
    /// El decorador entero (`@transform(...)`).
    pub decorador: Rango,
    /// Cada entrada, en forma corta, con el sitio de su argumento.
    pub inputs: Vec<(String, Rango)>,
    /// El argumento `output=`.
    pub output: Option<Rango>,
}

/// Dónde lo escribe la herramienta (§9, no normativo): `<repositorio>/pipeline/
/// <salida>.yaml`, con `<repositorio>` la primera carpeta del `entrypoint`. Un
/// validador lo encuentra por su `entrypoint`, viva donde viva.
pub fn ruta_del_documento(p: &Produccion) -> String {
    let ruta = p
        .entrypoint
        .rsplit_once(':')
        .map_or(p.entrypoint.as_str(), |(r, _)| r);
    match ruta.split_once('/') {
        Some((repositorio, _)) => format!("{repositorio}/pipeline/{}.yaml", p.output),
        None => format!("pipeline/{}.yaml", p.output),
    }
}

/// Si un documento lo derivó `ore` (empieza por [`MARCA`]).
pub fn es_derivado(texto: &str) -> bool {
    texto.trim_start_matches('\u{feff}').starts_with(MARCA)
}

/// El documento `Transform` de una producción, en el paquete `paquete`.
pub fn documento(p: &Produccion, paquete: &str) -> String {
    documento_con_dueno(p, paquete, None)
}

/// [`documento`] con quien responde (v1alpha21 `01`): no sale del código, lo
/// da quien guarda el documento, y al regenerarlo se conserva. Sin él, los
/// mismos bytes.
pub fn documento_con_dueno(p: &Produccion, paquete: &str, owner: Option<&str>) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{MARCA} {} · se edita el código, no este fichero",
        p.entrypoint
    );
    let _ = writeln!(s, "apiVersion: {API_VERSION}");
    s.push_str("kind: Transform\nmetadata:\n");
    let _ = writeln!(s, "  name: {}", escalar(&p.nombre()));
    let _ = writeln!(s, "  namespace: {}", escalar(paquete));
    if let Some(d) = &p.descripcion {
        let _ = writeln!(s, "  description: {}", escalar(d));
    }
    s.push_str("spec:\n");
    let _ = writeln!(s, "  runtime: {}", p.runtime);
    let _ = writeln!(s, "  entrypoint: {}", escalar(&p.entrypoint));
    let inputs: Vec<String> = p.inputs.iter().map(|x| escalar(x)).collect();
    let _ = writeln!(s, "  inputs: [{}]", inputs.join(", "));
    let _ = writeln!(s, "  output: {}", escalar(&p.output));
    if let Some(o) = owner {
        let _ = writeln!(s, "  owner: {}", escalar(o));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resumen() -> Produccion {
        Produccion {
            runtime: "python",
            entrypoint: "etl/transforms/resumen.py:resumen".into(),
            descripcion: Some("El total por país.".into()),
            inputs: vec!["ventas.pedidos".into(), "ventas.clientes".into()],
            output: "ventas.resumen".into(),
        }
    }

    /// Los bytes de `conformance/v1alpha25/valid/a-python-transform`.
    #[test]
    fn el_documento_tiene_la_forma_de_los_casos() {
        assert_eq!(
            documento(&resumen(), "ventas"),
            "# derivado por ore desde etl/transforms/resumen.py:resumen · se edita el código, \
             no este fichero\n\
             apiVersion: oos.dev/v1alpha25\n\
             kind: Transform\n\
             metadata:\n  name: ventas__resumen\n  namespace: ventas\n  \
             description: El total por país.\n\
             spec:\n  runtime: python\n  entrypoint: etl/transforms/resumen.py:resumen\n  \
             inputs: [ventas.pedidos, ventas.clientes]\n  output: ventas.resumen\n"
        );
        assert!(es_derivado(&documento(&resumen(), "ventas")));
        assert_eq!(
            ruta_del_documento(&resumen()),
            "etl/pipeline/ventas.resumen.yaml"
        );
    }

    #[test]
    fn sin_entradas_ni_descripcion() {
        let p = Produccion {
            runtime: "sql",
            entrypoint: "c.sql:2".into(),
            descripcion: None,
            inputs: vec![],
            output: "ventas.default.x".into(),
        };
        let d = documento(&p, "ventas");
        assert!(d.contains("  name: ventas__default__x\n"), "{d}");
        assert!(d.contains("  inputs: []\n"), "{d}");
        assert!(d.contains("  entrypoint: c.sql:2\n"), "{d}");
        assert!(!d.contains("description"), "{d}");
        assert_eq!(corto("ventas.default.x"), "ventas.x");
        assert_eq!(corto("ventas.etl.x"), "ventas.etl.x");
        assert_eq!(ruta_del_documento(&p), "pipeline/ventas.default.x.yaml");
    }
}
