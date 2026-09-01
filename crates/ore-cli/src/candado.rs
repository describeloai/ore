//! `ore lock` — el resolutor que se podía escribir sin registro.
//!
//! # Lo que faltaba, y de qué mitad se ocupa esto
//!
//! `ontology.lock` existe, `dependencies` existe, `OOS2013` comprueba que estén
//! sincronizados y **nada podía escribirlo**: la ayuda de ese diagnóstico decía
//! literalmente *«el resolutor que escribe el lock todavía no existe: hoy la
//! entrada se añade a mano»*. La cabecera del lock del ejemplo de referencia
//! anuncia desde el principio `ore lock` y `ore lock --check`, dos comandos que
//! no existían.
//!
//! El alcance que `v1alpha2/00-scope` §4 le pone a la resolución completa son
//! tres cosas —**un protocolo de registro, el formato del paquete publicable y
//! el resolutor**— y dos de las tres no son un comando. Pero hay una mitad que
//! no necesita ninguna: **resolver contra lo que ya está en el árbol.**
//!
//! Es el caso que se usa hoy y el único que se puede usar hoy. Un vocabulario se
//! consume copiándolo como un miembro más del workspace, y hasta ahora eso no se
//! podía **declarar**: `dependencies` quedaba escrito sin que nada lo
//! comprobara, o directamente sin escribir, y el árbol no registraba de dónde
//! venía su clasificación.
//!
//! # Por qué una coordenada casa con un paquete del árbol
//!
//! Porque el nombre de un paquete **es** la coordenada con la que otro lo
//! importa. Estaba en la descripción de `packageName` desde el principio —*«los
//! paquetes publicados llevan espacio de nombres, p. ej.
//! `oos.dev/regulatory/gdpr`»*— y el patrón del esquema la rechazaba, así que
//! había dos ideas de qué es un nombre de paquete y ninguna implementada.
//! Arreglado eso, casar `{package: oos.dev/regulatory/gdpr}` con el paquete que
//! se llama así **no es una convención de este motor**: es leer el nombre.
//!
//! # Y lo que este resolutor NO hace, que es la otra mitad
//!
//! **No trae nada.** Si la coordenada no está en el árbol, falla diciéndolo — no
//! la busca, no la descarga y no inventa una entrada. `ore` no sabe hablar por la
//! red y esa es una propiedad comprobada, no una promesa: el día que exista un
//! registro, traer un paquete se delegará como se delega leer una fuente.
//!
//! **No firma nada.** El `digest` que escribe es el de los documentos que hay en
//! el árbol, computado con el mismo `digest::package` que usa el bundle. No es el
//! digest de un `.oob` publicado, porque ese formato no existe todavía — y decir
//! que lo es sería exactamente la clase de afirmación que este proyecto desmonta.

use ore_core::document::Kind;
use ore_core::link::Package;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const CONFIG: &str = "ontology.config.yaml";
const LOCK: &str = "ontology.lock";

#[derive(Debug)]
pub struct Fallo {
    pub codigo: u8,
    pub mensaje: String,
    pub ayuda: Vec<String>,
}

fn fallo(codigo: u8, mensaje: impl Into<String>, ayuda: &[&str]) -> Fallo {
    Fallo {
        codigo,
        mensaje: mensaje.into(),
        ayuda: ayuda.iter().map(|s| (*s).to_string()).collect(),
    }
}

pub fn lock(raiz: &Path, comprobar: bool) -> ExitCode {
    match intentar(raiz, comprobar) {
        Ok(informe) => {
            print!("{informe}");
            ExitCode::SUCCESS
        }
        Err(f) => {
            eprintln!("error: {}", f.mensaje);
            for l in &f.ayuda {
                eprintln!("{l}");
            }
            ExitCode::from(f.codigo)
        }
    }
}

fn intentar(raiz: &Path, comprobar: bool) -> Result<String, Fallo> {
    if !raiz.join(CONFIG).is_file() {
        return Err(fallo(
            66, // EX_NOINPUT
            format!("no hay `{CONFIG}` en `{}`", raiz.display()),
            &["  Un lock es del workspace, y el manifiesto es lo que lo delimita."],
        ));
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);

    let declaradas = declaradas(&pkg);
    if declaradas.is_empty() {
        // No se escribe un lock vacío: un artefacto generado que no resuelve
        // nada es un fichero que hay que mantener sin que diga nada.
        return Ok(format!(
            "  Sin dependencias declaradas en `{CONFIG}`. No hay nada que resolver.\n"
        ));
    }

    let disponibles = miembros(&pkg);
    let mut entradas = Vec::new();
    for (coordenada, raiz_de_quien) in &declaradas {
        let Some((dir, version)) = disponibles.get(coordenada) else {
            return Err(no_encontrada(coordenada, &disponibles));
        };
        let rango = rango_de(&pkg, coordenada)?;
        if !satisface(version, &rango)? {
            return Err(fallo(
                65, // EX_DATAERR
                format!("`{coordenada}` está en el árbol como `{version}`, y se pidió `{rango}`"),
                &[
                    "  Un lock que resolviera un rango con una versión que no lo satisface",
                    "  diría que se cumple algo que no se cumple. O se cambia el rango, o se",
                    "  vendoriza la versión que pide.",
                ],
            ));
        }
        entradas.push(Entrada {
            nombre: coordenada.clone(),
            version: version.clone(),
            ruta: relativa(raiz, dir),
            digest: digest_de(dir),
            rango,
            raiz: *raiz_de_quien,
            provides: provides_de(dir),
        });
    }
    entradas.sort_by(|a, b| a.nombre.cmp(&b.nombre));

    let texto = escribir(&nombre_del_workspace(&pkg), &entradas);
    let ruta = raiz.join(LOCK);

    if comprobar {
        let actual = std::fs::read_to_string(&ruta).unwrap_or_default();
        if actual == texto {
            return Ok(format!(
                "  ✓ `{LOCK}` está al día · {} resueltas\n",
                entradas.len()
            ));
        }
        return Err(fallo(
            65,
            format!("`{LOCK}` no corresponde a `{CONFIG}`"),
            &[
                "  Es un artefacto generado y este quedó atrás. `ore lock` lo reescribe.",
                "  Se comprueba aparte de regenerarlo porque en CI hace falta saberlo sin",
                "  tocar el árbol: un artefacto obsoleto que se arregla solo al mirarlo no",
                "  se distingue de uno al día.",
            ],
        ));
    }

    std::fs::write(&ruta, &texto).map_err(|e| {
        fallo(
            73,
            format!("no se pudo escribir `{}`: {e}", ruta.display()),
            &[],
        )
    })?;
    Ok(informe(&entradas, &ruta))
}

// ── Lo que se declara y lo que hay ──────────────────────────────────────────

struct Entrada {
    nombre: String,
    version: String,
    ruta: String,
    digest: String,
    rango: String,
    raiz: bool,
    provides: BTreeMap<String, Vec<String>>,
}

/// Las dependencias declaradas, y si las pide el manifiesto o un paquete.
///
/// `requestedBy: root` es la del manifiesto; una declarada por un miembro es
/// transitiva desde el punto de vista del workspace, y el lock lo dice porque es
/// lo que permite saber **por qué** está ahí lo que está.
fn declaradas(pkg: &Package) -> Vec<(String, bool)> {
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    for d in &pkg.docs {
        let raiz = d.kind == Kind::OntologyConfig;
        if !raiz && d.kind != Kind::Package {
            continue;
        }
        for it in d.section("dependencies").map(|s| s.items()).unwrap_or(&[]) {
            if let Some(n) = it.get("package").and_then(|(_, v)| v.as_str()) {
                // Si la pide la raíz, la raíz manda: es la declaración que
                // alguien firma en el manifiesto publicable.
                let e = out.entry(n.to_string()).or_insert(raiz);
                *e = *e || raiz;
            }
        }
    }
    out.into_iter().collect()
}

/// El rango que se pidió para una coordenada. La primera declaración gana, y son
/// la misma porque `OOS2003` ya rechaza declararla dos veces.
fn rango_de(pkg: &Package, coordenada: &str) -> Result<String, Fallo> {
    for d in &pkg.docs {
        for it in d.section("dependencies").map(|s| s.items()).unwrap_or(&[]) {
            if it.get("package").and_then(|(_, v)| v.as_str()) == Some(coordenada)
                && let Some(r) = it.get("version").and_then(|(_, v)| v.as_str())
            {
                return Ok(r.to_string());
            }
        }
    }
    Err(fallo(
        65,
        format!("`{coordenada}` se declara sin `version`"),
        &["  Un rango es obligatorio: es lo único que dice a qué enunciado te acoges."],
    ))
}

/// Los paquetes que hay en el árbol, por su nombre — que es su coordenada.
fn miembros(pkg: &Package) -> BTreeMap<String, (PathBuf, String)> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|d| {
            let nombre = d.meta("name")?.as_str()?.to_string();
            let version = d.meta("version")?.as_str()?.to_string();
            Some((nombre, (d.path.parent()?.to_path_buf(), version)))
        })
        .collect()
}

fn no_encontrada(coordenada: &str, disponibles: &BTreeMap<String, (PathBuf, String)>) -> Fallo {
    let mut ayuda = vec![
        "  No se busca fuera: `ore` no sabe hablar por la red, y eso es una propiedad".to_string(),
        "  comprobada. Hoy una dependencia se resuelve contra un paquete que esté en el"
            .to_string(),
        "  árbol —vendorizado como un miembro más— y el día que exista un registro,".to_string(),
        "  traerla se delegará como se delega leer una fuente.".to_string(),
    ];
    if !disponibles.is_empty() {
        ayuda.push(format!(
            "  En el árbol hay: {}",
            disponibles.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    Fallo {
        codigo: 69, // EX_UNAVAILABLE
        mensaje: format!("`{coordenada}` no está en el árbol"),
        ayuda,
    }
}

// ── Los rangos ──────────────────────────────────────────────────────────────

fn partes(v: &str) -> Option<(u64, u64, u64)> {
    let limpio = v.split(['-', '+']).next()?;
    let mut it = limpio.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    Some((a, b, c))
}

/// `^`, `~` y la versión exacta.
///
/// Un rango que este resolutor no sepa leer **falla**, no pasa. Aceptarlo
/// escribiría en el lock que se cumple algo que nadie comprobó, que es la
/// dirección insegura: `>=1.0 <2.0` está en la gramática y todavía no aquí.
fn satisface(version: &str, rango: &str) -> Result<bool, Fallo> {
    let r = rango.trim();
    let Some(v) = partes(version) else {
        return Err(fallo(
            65,
            format!("`{version}` no es una versión semver"),
            &[],
        ));
    };
    let (op, resto) = match r.strip_prefix('^') {
        Some(x) => ('^', x),
        None => match r.strip_prefix('~') {
            Some(x) => ('~', x),
            None => ('=', r),
        },
    };
    let Some(p) = partes(resto) else {
        return Err(fallo(
            65,
            format!("no se sabe leer el rango `{rango}`"),
            &[
                "  Se admiten `^X.Y[.Z]`, `~X.Y[.Z]` y una versión exacta. Los rangos con",
                "  `>=` y `<` están en la gramática y no aquí — y aceptarlos sin",
                "  comprobarlos escribiría en el lock que se cumple algo que nadie miró.",
            ],
        ));
    };
    let techo = match op {
        // `^0.1` no es `<1.0.0`: en `0.x` cada minor puede romper, y esa es la
        // convención que usan todos los ecosistemas que la resolvieron antes.
        '^' if p.0 == 0 => (0, p.1 + 1, 0),
        '^' => (p.0 + 1, 0, 0),
        '~' => (p.0, p.1 + 1, 0),
        _ => return Ok(v == p),
    };
    Ok(v >= p && v < techo)
}

// ── Lo que se escribe ───────────────────────────────────────────────────────

fn relativa(raiz: &Path, dir: &Path) -> String {
    dir.strip_prefix(raiz)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/")
}

/// El digest de lo que hay en el árbol, con el mismo `digest::package` del
/// bundle. No es el de un `.oob` publicado, porque ese formato no existe — y
/// llamarlo así sería afirmar una procedencia que nadie puede comprobar.
fn digest_de(dir: &Path) -> String {
    ore_core::digest::package(&ore_core::validate::cargar_paquete(dir).0)
}

/// Qué aporta un paquete, **derivado de lo que tiene dentro**. Lo derivable no
/// se declara (P2), y una lista escrita a mano en un artefacto generado sería
/// dos veces lo mismo.
fn provides_de(dir: &Path) -> BTreeMap<String, Vec<String>> {
    let (pkg, _) = ore_core::validate::cargar_paquete(dir);
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in &pkg.docs {
        let clave = match d.kind {
            Kind::Property => "concepts",
            Kind::Lattice => "lattices",
            Kind::ConduitPolicy => "conduits",
            Kind::Ruleset => "rulesets",
            Kind::Interface => "interfaces",
            Kind::Entity => "entities",
            Kind::Function => "functions",
            Kind::Resolution => "resolutions",
            // `Binding`, `Package` y el manifiesto no se aportan: el primero
            // dice dónde está el dato de QUIEN LO PUBLICA, y los otros dos son
            // el paquete, no algo dentro de él.
            _ => continue,
        };
        if let Some(q) = d.qname() {
            out.entry(clave.to_string()).or_default().push(q);
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

fn nombre_del_workspace(pkg: &Package) -> String {
    pkg.docs
        .iter()
        .find(|d| d.kind == Kind::OntologyConfig)
        .and_then(|d| {
            let n = d.meta("name")?.as_str()?;
            let v = d.meta("version")?.as_str()?;
            Some(format!("{n}@{v}"))
        })
        .unwrap_or_else(|| "desconocido".into())
}

fn escribir(para: &str, entradas: &[Entrada]) -> String {
    let mut s = String::from(
        "# =============================================================================\n\
         # GENERADO POR ORE — NO EDITAR A MANO\n\
         #\n\
         # Resolución determinista de dependencias. Se compromete a Git.\n\
         # Regenerar: `ore lock`   ·   Verificar sin modificar: `ore lock --check`\n\
         #\n\
         # Todo lo de aquí se resolvió CONTRA EL ÁRBOL. `ore` no habla por la red, así\n\
         # que un paquete se resuelve si está vendorizado como un miembro del workspace\n\
         # y si no, esto falla en vez de inventar una entrada. El día que exista un\n\
         # registro, traerlo se delegará como se delega leer una fuente.\n\
         #\n\
         # `digest` es el de los documentos que hay en el árbol, con el mismo cómputo\n\
         # que el del bundle. NO es el digest de un paquete publicado: ese formato no\n\
         # existe todavía, y decir que lo es sería afirmar una procedencia que nadie\n\
         # puede comprobar. Por lo mismo no hay `profiles`: sus digests no se pueden\n\
         # computar de nada, y uno inventado es peor que ninguno.\n\
         # =============================================================================\n\
         \n\
         lockfileVersion: 1\n",
    );
    let _ = writeln!(s, "generatedFor: {para}\n");
    s.push_str("packages:\n");
    for e in entradas {
        let _ = write!(
            s,
            "\n  - name: {}\n    version: {}\n    resolved: file:{}\n    digest: {}\n    range: \"{}\"\n",
            e.nombre, e.version, e.ruta, e.digest, e.rango
        );
        if e.raiz {
            s.push_str("    requestedBy: root\n");
        }
        if !e.provides.is_empty() {
            s.push_str("    provides:\n");
            for (clave, valores) in &e.provides {
                let _ = writeln!(s, "      {clave}: [{}]", valores.join(", "));
            }
        }
    }
    s
}

fn informe(entradas: &[Entrada], ruta: &Path) -> String {
    let mut s = format!("  ✓ {}\n", ruta.display());
    for e in entradas {
        let _ = writeln!(
            s,
            "  · {} {} · {} · {}",
            e.nombre,
            e.version,
            e.ruta,
            &e.digest[..e.digest.len().min(19)]
        );
    }
    s.push_str("\n  Resuelto contra el árbol. Nada se ha traído de fuera.\n");
    s
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// En `0.x` cada minor puede romper, y por eso `^0.1` no llega a `0.2`. Es
    /// la convención de todos los ecosistemas que resolvieron esto antes, y
    /// tratarla como `^1.x` dejaría pasar una versión rompedora.
    #[test]
    fn el_cero_mayor_es_estricto() {
        assert!(satisface("0.1.0", "^0.1").unwrap());
        assert!(satisface("0.1.9", "^0.1").unwrap());
        assert!(!satisface("0.2.0", "^0.1").unwrap());
        assert!(!satisface("0.0.9", "^0.1").unwrap());
    }

    #[test]
    fn el_acento_circunflejo_y_la_tilde_no_son_lo_mismo() {
        assert!(satisface("2.9.0", "^2.1").unwrap());
        assert!(!satisface("3.0.0", "^2.1").unwrap());
        assert!(satisface("2.1.9", "~2.1").unwrap());
        assert!(!satisface("2.2.0", "~2.1").unwrap());
    }

    #[test]
    fn una_version_exacta_es_exacta() {
        assert!(satisface("1.2.3", "1.2.3").unwrap());
        assert!(!satisface("1.2.4", "1.2.3").unwrap());
    }

    /// Un rango que no se sabe leer falla. Aceptarlo escribiría en el lock que
    /// se cumple algo que nadie comprobó, que es la dirección insegura.
    #[test]
    fn un_rango_que_no_se_sabe_leer_no_pasa() {
        assert!(satisface("1.0.0", ">=1.0 <2.0").is_err());
    }
}
