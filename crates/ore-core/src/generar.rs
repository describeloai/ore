//! **Generar** los documentos de las funciones de código (ORE 0050 G1d): el
//! cliente escribe Python o TypeScript, y cada `@function` —o exportación por
//! defecto de un `.ts` de `functions/` (R3, OOS v1alpha23)— del paquete tiene
//! su `Function` porque esto lo escribe, no porque nadie lo copie a mano.
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
//! - **Dónde**: siempre en `functions/<def>.yaml` **del paquete**, nunca junto
//!   al código. Una función publicada es un nombre del paquete
//!   (`<paquete>.<def>`) y no del repositorio donde se escribe: es lo que se ve
//!   en Assets → Functions, sin base ni schema que elegir. Un documento de esa
//!   función que esté en otro sitio —el de antes, junto al repositorio— se
//!   mueve: se borra allí y se escribe aquí.
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
use crate::promover::{
    carpeta_del_paquete, ficheros_de_codigo, no_se_deriva, paquetes_publicables, roto,
};
use ore_code::emitir;
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
    /// `<ruta del .py>:<def>` o `<ruta del .ts>`, desde la carpeta del paquete.
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
    plan_de(pkg, None)
}

/// El plan, **solo** para el código de `solo` (rutas de `.py` o `.ts`, existan o no):
/// lo que deriva de esos ficheros y los documentos generados que los nombran.
/// Es lo que hace el commit de un repositorio (G2): genera lo que ese commit
/// toca, y nada de lo que la rama no tocó.
pub fn plan_de(pkg: &Package, solo: Option<&BTreeSet<PathBuf>>) -> Plan {
    plan_con_dueno(pkg, solo, None)
}

/// [`plan_de`] sabiendo quién crea (v1alpha21 `01` §4): un documento que NACE
/// lleva su `owner`; uno que ya estaba conserva el suyo —regenerar no es
/// transferir—, y uno sin él sigue sin él.
pub fn plan_con_dueno(
    pkg: &Package,
    solo: Option<&BTreeSet<PathBuf>>,
    dueno: Option<&str>,
) -> Plan {
    let mut p = Plan::default();
    for (carpeta, paquete) in paquetes_publicables(pkg) {
        plan_del_paquete(pkg, &carpeta, &paquete, solo, dueno, &mut p);
    }
    p.cambios.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    p
}

fn plan_del_paquete(
    pkg: &Package,
    carpeta: &Path,
    paquete: &str,
    solo: Option<&BTreeSet<PathBuf>>,
    dueno: Option<&str>,
    p: &mut Plan,
) {
    let entra = |py: &Path| solo.is_none_or(|s| s.contains(py));
    // Los documentos de código del paquete, por su `entrypoint`.
    let mut existentes: BTreeMap<String, Existente> = BTreeMap::new();
    for f in pkg.of(Kind::Function) {
        if carpeta_del_paquete(&f.path, &pkg.root) != carpeta
            || !matches!(
                f.section("runtime").and_then(Node::as_str),
                Some("python" | "node")
            )
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
    ficheros_de_codigo(carpeta, &mut pys);
    pys.sort();
    // Lo que el código pide, y dónde: para no generar dos veces el mismo sitio.
    let mut vivas: BTreeSet<String> = BTreeSet::new();
    let mut destinos: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut nombres: BTreeMap<String, String> = BTreeMap::new();
    for py in pys {
        if carpeta_del_paquete(&py, &pkg.root) != carpeta || !entra(&py) {
            continue; // de un paquete de dentro, que lo planea él; o no se pidió
        }
        let Ok(fuente) = std::fs::read_to_string(&py) else {
            continue;
        };
        let Ok(rel) = py.strip_prefix(carpeta) else {
            continue;
        };
        let ruta = rel.to_string_lossy().replace('\\', "/");
        if !ore_code::puede_tener_funciones(&ruta, &fuente) {
            continue;
        }
        let Some(d) = ore_code::derivar(&fuente, &ruta) else {
            continue;
        };
        if d.funciones.is_empty() {
            continue;
        }
        // Un fichero roto no dice qué funciones tiene: sus documentos se quedan.
        if let Some(diag) = roto(&py, &fuente, &d) {
            for f in &d.funciones {
                vivas.insert(ore_code::entrypoint_de(&ruta, &f.nombre));
            }
            p.diagnosticos.push(diag);
            continue;
        }
        for f in &d.funciones {
            let entrypoint = ore_code::entrypoint_de(&ruta, &f.nombre);
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
            let destino = carpeta.join(emitir::ruta_del_documento(firma));
            let previo = existentes.get(&entrypoint);
            // El dueño: el que ya tenía su documento (aunque se mueva); si nace,
            // el de quien lo crea.
            let owner = match previo {
                Some(e) => owner_de(&e.texto),
                None => match std::fs::read_to_string(&destino) {
                    Ok(t) if emitir::es_generado(&t) => owner_de(&t),
                    Ok(_) => None,
                    Err(_) => dueno.map(str::to_string),
                },
            };
            let contenido = emitir::documento_con_dueno(firma, paquete, owner.as_deref());
            // El suyo, en otro sitio: se mueve aquí.
            let movido = previo.filter(|e| e.ruta != destino);
            let movido_a_mano = movido.is_some_and(|e| !emitir::es_generado(&e.texto));
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
            // Lo que hay en el destino, y si es el suyo.
            let (actual, es_suyo) = match previo {
                Some(e) if movido.is_none() => (Some(e.texto.clone()), true),
                _ => (std::fs::read_to_string(&destino).ok(), false),
            };
            let borrar_el_de_antes = |p: &mut Plan| {
                if let Some(e) = movido {
                    p.cambios.push(Cambio {
                        ruta: e.ruta.clone(),
                        entrypoint: entrypoint.clone(),
                        accion: Accion::Borrar,
                    });
                }
            };
            let accion = match actual {
                None if movido_a_mano => Accion::Reescribir {
                    contenido,
                    a_mano: true,
                },
                None => Accion::Crear(contenido),
                Some(t) if t.replace("\r\n", "\n") == contenido => {
                    if movido.is_some() {
                        borrar_el_de_antes(p);
                    } else {
                        p.al_dia += 1;
                    }
                    continue;
                }
                Some(t) if !es_suyo && !emitir::es_generado(&t) => {
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
                    a_mano: !emitir::es_generado(&t) || movido_a_mano,
                },
            };
            borrar_el_de_antes(p);
            p.cambios.push(Cambio {
                ruta: destino,
                entrypoint,
                accion,
            });
        }
    }

    // ── lo que sobra: generado, y su `def` ya no es un `@function` ──────────
    for (entrypoint, e) in &existentes {
        // `<ruta>.py:<def>`, o `<ruta>.ts`: el fichero es la función.
        let fichero = carpeta.join(
            entrypoint
                .rsplit_once(':')
                .map(|(r, _)| r)
                .unwrap_or(entrypoint),
        );
        if !vivas.contains(entrypoint) && emitir::es_generado(&e.texto) && entra(&fichero) {
            p.cambios.push(Cambio {
                ruta: e.ruta.clone(),
                entrypoint: entrypoint.clone(),
                accion: Accion::Borrar,
            });
        }
    }
}

/// Escribe el plan en disco.
/// El `spec.owner` de un documento, si lo dice.
fn owner_de(texto: &str) -> Option<String> {
    crate::parse::parse(texto)
        .ok()?
        .get("spec")
        .and_then(|(_, s)| s.get("owner"))
        .and_then(|(_, v)| v.as_str())
        .filter(|o| !o.is_empty())
        .map(str::to_string)
}

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
