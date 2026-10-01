//! **Generar** los documentos de las funciones de código (ORE 0050 G1d): el
//! cliente escribe Python, y cada `@function` del paquete tiene su `Function`
//! porque esto lo escribe, no porque nadie lo copie a mano.
//!
//! Aquí solo se **planea**: qué documento se crea, cuál se reescribe y cuál
//! sobra, con el contenido exacto. Aplicarlo es de quien llama —`ore functions
//! generate` lo escribe en disco; el commit de un repositorio (G2) lo hará en
//! su árbol— y por eso el plan es una función pura del árbol.
//!
//! # Las reglas
//!
//! - **El contenido** es el de `ore_code::emitir`: los mismos bytes para la
//!   misma firma, así que generar dos veces no cambia nada.
//! - **Dónde**: si un documento del paquete ya nombra ese `entrypoint`, ahí se
//!   queda (§4.8: el sitio no es parte de la regla, y quien lo movió sabía por
//!   qué). Si no, en `functions/<def>.yaml` del **repositorio** del código —la
//!   carpeta más cercana, subiendo, con `pyproject.toml`— o del paquete.
//! - **Lo que sobra**: un documento generado (empieza por la marca de
//!   procedencia) cuyo `def` ya no es un `@function` se borra. Uno escrito a
//!   mano nunca: si su `def` ya no está, eso lo dice `ore validate`.
//! - **Lo que no se deriva** no se escribe ni se borra: su documento se queda
//!   como estaba y el diagnóstico (`OOS2043`) dice por qué.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::Package;
use crate::parse::Node;
use crate::promover::{carpeta_del_paquete, ficheros_py, no_se_deriva, paquetes_publicables, roto};
use ore_code::{emitir, python};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accion {
    /// No había documento.
    Crear(String),
    /// Había uno y no es el que el código da. `a_mano`: no llevaba la marca de
    /// procedencia —alguien lo escribió—, y se dice.
    Reescribir { contenido: String, a_mano: bool },
    /// Era generado y su `def` ya no es un `@function`.
    Borrar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cambio {
    pub ruta: PathBuf,
    /// `<ruta del .py>:<def>`, desde la carpeta del paquete.
    pub entrypoint: String,
    pub accion: Accion,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub cambios: Vec<Cambio>,
    /// Lo que impide generar algo: un `def` que no se deriva, un fichero que
    /// no es Python del puesto, dos funciones que se llamarían igual.
    pub diagnosticos: Vec<Diagnostic>,
    /// Los documentos que ya eran el que el código da.
    pub al_dia: usize,
}

/// Un documento `runtime: python` del paquete: dónde está y qué dice.
struct Existente {
    ruta: PathBuf,
    texto: String,
}

pub fn plan(pkg: &Package) -> Plan {
    let mut p = Plan::default();
    for (carpeta, paquete) in paquetes_publicables(pkg) {
        plan_del_paquete(pkg, &carpeta, &paquete, &mut p);
    }
    p.cambios.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    p
}

fn plan_del_paquete(pkg: &Package, carpeta: &Path, paquete: &str, p: &mut Plan) {
    // Los documentos de código del paquete, por su `entrypoint`.
    let mut existentes: BTreeMap<String, Existente> = BTreeMap::new();
    for f in pkg.of(Kind::Function) {
        if carpeta_del_paquete(&f.path, &pkg.root) != carpeta
            || f.section("runtime").and_then(Node::as_str) != Some("python")
        {
            continue;
        }
        let Some(e) = f.section("entrypoint").and_then(Node::as_str) else {
            continue;
        };
        let texto = std::fs::read_to_string(&f.path).unwrap_or_default();
        existentes.insert(
            e.to_string(),
            Existente {
                ruta: f.path.clone(),
                texto,
            },
        );
    }

    let mut pys = Vec::new();
    ficheros_py(carpeta, &mut pys);
    pys.sort();
    // Lo que el código pide, y dónde: para no generar dos veces el mismo sitio.
    let mut vivas: BTreeSet<String> = BTreeSet::new();
    let mut destinos: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut nombres: BTreeMap<String, String> = BTreeMap::new();
    for py in pys {
        if carpeta_del_paquete(&py, &pkg.root) != carpeta {
            continue; // de un paquete de dentro: lo planea él
        }
        let Ok(fuente) = std::fs::read_to_string(&py) else {
            continue;
        };
        if !python::puede_tener_funciones(&fuente) {
            continue;
        }
        let Ok(rel) = py.strip_prefix(carpeta) else {
            continue;
        };
        let ruta = rel.to_string_lossy().replace('\\', "/");
        let d = python::derivar(&fuente, &ruta);
        if d.funciones.is_empty() {
            continue;
        }
        // Un fichero roto no dice qué funciones tiene: sus documentos se quedan.
        if let Some(diag) = roto(&py, &fuente, &d) {
            for f in &d.funciones {
                vivas.insert(format!("{ruta}:{}", f.nombre));
            }
            p.diagnosticos.push(diag);
            continue;
        }
        for f in &d.funciones {
            let entrypoint = format!("{ruta}:{}", f.nombre);
            vivas.insert(entrypoint.clone());
            let firma = match &f.resultado {
                Ok(x) => x,
                Err(fallos) => {
                    no_se_deriva(&py, &fuente, &f.nombre, fallos, &mut p.diagnosticos);
                    continue;
                }
            };
            if let Some(otro) = nombres.insert(f.nombre.clone(), entrypoint.clone()) {
                p.diagnosticos.push(
                    Diagnostic::new(
                        Code::Oos2013,
                        &py,
                        format!(
                            "dos `@function` se llamarían `{paquete}.{}`: `{otro}` y `{entrypoint}`",
                            f.nombre
                        ),
                    )
                    .help("una función es un nombre del paquete: renombra uno de los dos `def`"),
                );
                continue;
            }
            let contenido = emitir::documento(firma, paquete);
            let destino = match existentes.get(&entrypoint) {
                Some(e) => e.ruta.clone(),
                None => repositorio(&py, carpeta).join(emitir::ruta_del_documento(firma)),
            };
            if let Some(otro) = destinos.insert(destino.clone(), entrypoint.clone()) {
                p.diagnosticos.push(
                    Diagnostic::new(
                        Code::Oos2013,
                        &destino,
                        format!("`{otro}` y `{entrypoint}` irían al mismo documento"),
                    )
                    .help("mueve uno de los dos documentos: se le encuentra por su `entrypoint`"),
                );
                continue;
            }
            let actual = match existentes.get(&entrypoint) {
                Some(e) => Some(e.texto.clone()),
                None => std::fs::read_to_string(&destino).ok(),
            };
            let accion = match actual {
                None => Accion::Crear(contenido),
                Some(t) if t.replace("\r\n", "\n") == contenido => {
                    p.al_dia += 1;
                    continue;
                }
                Some(t) if !existentes.contains_key(&entrypoint) && !emitir::es_generado(&t) => {
                    // El sitio lo ocupa un documento que no es de esta función.
                    p.diagnosticos.push(
                        Diagnostic::new(
                            Code::Oos2013,
                            &destino,
                            format!(
                                "el documento de `{entrypoint}` iría aquí, y aquí hay otro escrito a mano"
                            ),
                        )
                        .help("muévelo o renómbralo; los generados se reescriben, los escritos a mano no"),
                    );
                    continue;
                }
                Some(t) => Accion::Reescribir {
                    contenido,
                    a_mano: !emitir::es_generado(&t),
                },
            };
            p.cambios.push(Cambio {
                ruta: destino,
                entrypoint,
                accion,
            });
        }
    }

    // ── lo que sobra: generado, y su `def` ya no es un `@function` ──────────
    for (entrypoint, e) in &existentes {
        if !vivas.contains(entrypoint) && emitir::es_generado(&e.texto) {
            p.cambios.push(Cambio {
                ruta: e.ruta.clone(),
                entrypoint: entrypoint.clone(),
                accion: Accion::Borrar,
            });
        }
    }
}

/// El repositorio de un `.py`: la carpeta más cercana, subiendo y sin salir
/// del paquete, con `pyproject.toml`; o la del paquete.
fn repositorio(py: &Path, carpeta: &Path) -> PathBuf {
    let mut d = py.parent();
    while let Some(c) = d {
        if !c.starts_with(carpeta) || c == carpeta {
            break;
        }
        if c.join("pyproject.toml").is_file() {
            return c.to_path_buf();
        }
        d = c.parent();
    }
    carpeta.to_path_buf()
}

/// Escribe el plan en disco.
pub fn aplicar(plan: &Plan) -> std::io::Result<()> {
    for c in &plan.cambios {
        match &c.accion {
            Accion::Crear(t) | Accion::Reescribir { contenido: t, .. } => {
                if let Some(d) = c.ruta.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(&c.ruta, t)?;
            }
            Accion::Borrar => std::fs::remove_file(&c.ruta)?,
        }
    }
    Ok(())
}
