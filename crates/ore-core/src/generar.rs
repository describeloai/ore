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

/// El documento de una función en la rama principal: su texto y, si es de la
/// forma de antes (del paquete), la versión de su paquete allí.
#[derive(Debug, Clone)]
pub struct Anterior {
    pub texto: String,
    pub version_del_paquete: Option<String>,
}

/// Un documento de código del árbol: dónde está, qué dice y, si es de la forma
/// de antes (del paquete), la versión de su paquete —con la que nace al
/// migrar (v1alpha26 `01` §7)—.
struct Existente {
    ruta: PathBuf,
    texto: String,
    version_del_paquete: Option<String>,
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
/// transferir—. Sin quien crea, el dueño del paquete del código: una función
/// propia no nace sin dueño (v1alpha26 `01` §2).
pub fn plan_con_dueno(
    pkg: &Package,
    solo: Option<&BTreeSet<PathBuf>>,
    dueno: Option<&str>,
) -> Plan {
    plan_con_anteriores(pkg, solo, dueno, None)
}

/// **El plan de las funciones propias** (ORE 0056 V2, OOS v1alpha26): cada
/// función del código va a `functions/<nombre>.yaml` de la raíz, con
/// `codeDigest` y con su versión **calculada** contra `anteriores` —nombre →
/// su documento en la rama principal— por las reglas de
/// [`crate::diff::salto_de_funcion`]. Sin `anteriores`, contra el documento
/// que el árbol tiene. Un documento de la forma de antes (del paquete) se
/// mueve aquí y nace con la versión de su paquete.
pub fn plan_con_anteriores(
    pkg: &Package,
    solo: Option<&BTreeSet<PathBuf>>,
    dueno: Option<&str>,
    anteriores: Option<&BTreeMap<String, Anterior>>,
) -> Plan {
    let mut p = Plan::default();
    let raiz = pkg.root.as_path();
    // Los documentos de código del árbol, por su `entrypoint` desde la raíz.
    let mut existentes: BTreeMap<String, Existente> = BTreeMap::new();
    for f in pkg.of(Kind::Function) {
        if !matches!(
            f.section("runtime").and_then(Node::as_str),
            Some("python" | "node")
        ) {
            continue;
        }
        let Some(e) = f.section("entrypoint").and_then(Node::as_str) else {
            continue;
        };
        let (desde_raiz, version_del_paquete) = if crate::funcion_propia::es_propia(f) {
            (e.to_string(), None)
        } else {
            let carpeta = carpeta_del_paquete(&f.path, raiz);
            let rel = crate::funcion_propia::relativa(raiz, &carpeta);
            let version = pkg
                .of(Kind::Package)
                .find(|d| d.path.parent() == Some(carpeta.as_path()))
                .and_then(|d| d.meta("version").and_then(Node::as_str))
                .map(str::to_string);
            let e = if rel.is_empty() {
                e.to_string()
            } else {
                format!("{rel}/{e}")
            };
            (e, version)
        };
        existentes.insert(
            desde_raiz,
            Existente {
                ruta: f.path.clone(),
                texto: std::fs::read_to_string(&f.path).unwrap_or_default(),
                version_del_paquete,
            },
        );
    }

    let entra = |py: &Path| solo.is_none_or(|s| s.contains(py));
    let mut vivas: BTreeSet<String> = BTreeSet::new();
    // nombre en minúsculas → entrypoint: un nombre, una función (`OOS2035`).
    let mut nombres: BTreeMap<String, String> = BTreeMap::new();
    for (carpeta, _paquete) in paquetes_publicables(pkg) {
        let owner_del_paquete = pkg
            .of(Kind::Package)
            .find(|d| d.path.parent() == Some(carpeta.as_path()))
            .and_then(|d| d.section("owner").and_then(Node::as_str))
            .map(str::to_string);
        let mut pys = Vec::new();
        ficheros_de_codigo(&carpeta, &mut pys);
        pys.sort();
        for py in pys {
            if carpeta_del_paquete(&py, raiz) != carpeta || !entra(&py) {
                continue; // de un paquete de dentro, que lo planea él; o no se pidió
            }
            let Ok(fuente) = std::fs::read_to_string(&py) else {
                continue;
            };
            let ruta = crate::funcion_propia::relativa(raiz, &py);
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
                if let Some(otro) = nombres.insert(f.nombre.to_lowercase(), entrypoint.clone()) {
                    p.diagnosticos.push(
                        Diagnostic::new(
                            Code::Oos2035,
                            &py,
                            format!(
                                "dos funciones se llamarían `{}.{}`: `{otro}` y `{entrypoint}`",
                                crate::funcion_propia::ESPACIO,
                                f.nombre
                            ),
                        )
                        .help(
                            "el nombre de una función es único en el espacio de trabajo, sin \
                             mirar mayúsculas: renombra uno de los dos",
                        ),
                    );
                    continue;
                }
                let destino = raiz.join(crate::funcion_propia::ruta_del_documento(&f.nombre));
                let previo = existentes.get(&entrypoint);
                let en_destino = std::fs::read_to_string(&destino).ok();
                // El dueño: el que ya tenía; si nace, el de quien lo crea; si
                // nadie lo dice, el del paquete del código.
                let owner = previo
                    .and_then(|e| owner_de(&e.texto))
                    .or_else(|| {
                        en_destino
                            .as_deref()
                            .filter(|t| emitir::es_generado(t))
                            .and_then(owner_de)
                    })
                    .or_else(|| dueno.map(str::to_string))
                    .or_else(|| owner_del_paquete.clone());
                let Some(owner) = owner else {
                    p.diagnosticos.push(
                        Diagnostic::new(
                            Code::Oos1004,
                            &py,
                            format!("`{entrypoint}` no tiene de quién ser: nadie da `owner`"),
                        )
                        .help(
                            "una función propia nace con dueño, y el paquete del código no lo dice",
                        ),
                    );
                    continue;
                };
                let Some(huella) = crate::funcion_propia::huella(raiz, &ruta, firma.runtime())
                else {
                    continue;
                };
                // La versión: contra la de la rama principal si se da; si no,
                // contra lo que el árbol tiene.
                let (anterior, base) = match anteriores {
                    // Lo que sabe la rama principal manda: lo que no tiene, nace.
                    Some(a) => match a.get(&f.nombre) {
                        Some(x) => (Some(x.texto.clone()), x.version_del_paquete.clone()),
                        None => (None, None),
                    },
                    None => match previo {
                        Some(e) => (Some(e.texto.clone()), e.version_del_paquete.clone()),
                        None => (en_destino.clone().filter(|t| emitir::es_generado(t)), None),
                    },
                };
                let version = version_de(
                    firma,
                    &owner,
                    &huella,
                    &destino,
                    anterior.as_deref(),
                    base.as_deref(),
                );
                let contenido = emitir::documento_propio(firma, &owner, &version, &huella);
                let movido = previo.filter(|e| e.ruta != destino);
                let movido_a_mano = movido.is_some_and(|e| !emitir::es_generado(&e.texto));
                let es_suyo = previo.is_some_and(|e| e.ruta == destino);
                let borrar_el_de_antes = |p: &mut Plan| {
                    if let Some(e) = movido {
                        p.cambios.push(Cambio {
                            ruta: e.ruta.clone(),
                            entrypoint: entrypoint.clone(),
                            accion: Accion::Borrar,
                        });
                    }
                };
                let accion = match en_destino {
                    None if movido_a_mano => Accion::Reescribir {
                        contenido,
                        a_mano: true,
                    },
                    None => Accion::Crear(contenido),
                    Some(t) if t.replace("\r\n", "\n") == contenido => {
                        if movido.is_some() {
                            borrar_el_de_antes(&mut p);
                        } else {
                            p.al_dia += 1;
                        }
                        continue;
                    }
                    Some(t) if !es_suyo && !emitir::es_generado(&t) => {
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
                borrar_el_de_antes(&mut p);
                p.cambios.push(Cambio {
                    ruta: destino,
                    entrypoint,
                    accion,
                });
            }
        }
    }

    // ── lo que sobra: generado, y su `def` ya no es una función ─────────────
    for (entrypoint, e) in &existentes {
        let fichero = raiz.join(
            entrypoint
                .rsplit_once(':')
                .map(|(r, _)| r)
                .unwrap_or(entrypoint),
        );
        if !vivas.contains(entrypoint)
            && emitir::es_generado(&e.texto)
            && entra(&fichero)
            && !p.cambios.iter().any(|c| c.ruta == e.ruta)
        {
            p.cambios.push(Cambio {
                ruta: e.ruta.clone(),
                entrypoint: entrypoint.clone(),
                accion: Accion::Borrar,
            });
        }
    }
    p.cambios.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    p
}

/// La versión de una función que se genera (v1alpha26 `01` §5): nace en
/// `0.1.0`; si tenía documento, la de antes —o la de su paquete, si era de la
/// forma de antes— subida lo que el cambio exige.
fn version_de(
    firma: &ore_code::Firma,
    owner: &str,
    huella: &str,
    destino: &Path,
    anterior: Option<&str>,
    base: Option<&str>,
) -> String {
    let inicial = crate::funcion_propia::VERSION_INICIAL.to_string();
    let Some(texto) = anterior else {
        return inicial;
    };
    let cargar = |t: &str| {
        crate::parse::parse(t).ok().map(|root| crate::link::Loaded {
            path: destino.to_path_buf(),
            kind: Kind::Function,
            root,
        })
    };
    let Some(antes) = cargar(texto) else {
        return inicial;
    };
    let de_antes = base
        .map(str::to_string)
        .or_else(|| {
            antes
                .meta("version")
                .and_then(Node::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| inicial.clone());
    // El documento nuevo con la versión de antes: lo que cambia es el resto.
    let Some(despues) = cargar(&emitir::documento_propio(firma, owner, &de_antes, huella)) else {
        return de_antes;
    };
    let (salto, _) = crate::diff::salto_de_funcion(&antes, &despues);
    // La forma de antes no tenía huella: ganarla al migrar no es un cambio.
    let salto = if base.is_some() && salto == Some(crate::diff::Bump::Patch) {
        None
    } else {
        salto
    };
    crate::diff::version_tras(&de_antes, salto).unwrap_or(de_antes)
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
