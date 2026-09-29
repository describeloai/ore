//! **En qué se diferencia una rama de la de por defecto** — ramas globales,
//! fase 2. `GET /ramas/{rama}/cambios`.
//!
//! Git sabe qué ficheros cambiaron; esto dice qué **activos** cambiaron y qué
//! significa. Por activo (kind y nombre, no fichero): nuevo, modificado,
//! borrado o movido de sitio, comparando el documento en su forma canónica —
//! reformatear no es un cambio—. De cada uno, qué cambió: las claves de su
//! `spec` y `metadata`, las columnas (añadidas, quitadas, con otro tipo) y el
//! SQL antes y después. Lo que ROMPE sale de `ore diff` y se ata a su activo;
//! y a quién alcanza, del linaje: quién lee lo roto, hasta el final. Con los
//! diagnósticos de la rama y la distancia a la base.
//!
//! ⭐ Contra el punto del que salió la rama (`merge-base`), no contra la base
//!   de hoy: lo que la base hizo después no es de la rama.
//!
//! ⭐ **En conflicto con la base**: un activo que la rama cambió y que la base
//!   TAMBIÉN cambió desde que la rama salió (`conflicto` y `enBase`). Al fusionar
//!   pueden chocar, y quien mira la rama está viendo una versión que ya no es la
//!   de la base. Se compara el punto de partida con la base de hoy.
//!
//! ⭐ **Un `Package` es también sus `discover.*`** (lo que lo hace una base: de
//!   qué origen sale y qué entró). No son documentos, pero son del paquete: si
//!   cambian, el `Package` cambia, y viajan con él (`ficheros`). Sin eso, una
//!   propuesta de activos llevaba la base a `main` sin `discover.scope.json`, y en
//!   `main` no era una database (la #9 de t-victor, 2026-09-29).
//!
//! ⭐ De memoria por `(cabeza de la base, cabeza de la rama)`: con las dos
//!   cabezas iguales el resultado es el mismo, y se saben sin clonar.

use crate::rutas::{Arbol, Servidor};
use ore_core::json::Json;
use ore_core::link::{Loaded, Package};
use ore_entrada::http::Respuesta;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

/// Un activo del árbol, por su identidad (`Kind:nombre`).
struct Activo<'a> {
    doc: &'a Loaded,
    canonico: String,
    ruta: String,
    /// De un `Package`, sus [`DE_LA_BASE`] que existen: `ruta → contenido`.
    anexos: BTreeMap<String, String>,
}

/// Lo que el alta de una base escribe junto a su `package.yaml` (`ore discover`,
/// `ore-cli/src/alcance.rs`): del paquete, aunque no sean documentos.
pub(crate) const DE_LA_BASE: [&str; 4] = [
    "discover.scope.json",
    "discover.catalog.json",
    "discover.answers.json",
    "discover.pending.json",
];

/// Los [`DE_LA_BASE`] de un `Package` que existen, por su ruta en el árbol.
fn anexos(d: &Loaded, ruta: &str) -> BTreeMap<String, String> {
    let (Some(sitio), Some(dir)) = (ruta.rsplit_once('/').map(|(s, _)| s), d.path.parent()) else {
        return BTreeMap::new();
    };
    if d.kind != ore_core::document::Kind::Package {
        return BTreeMap::new();
    }
    DE_LA_BASE
        .iter()
        .filter_map(|f| {
            let c = std::fs::read_to_string(dir.join(f)).ok()?;
            Some((format!("{sitio}/{f}"), c))
        })
        .collect()
}

/// Los anexos que cambian entre dos lados (nuevos, quitados, distintos o
/// movidos: de un movimiento, las dos rutas).
fn anexos_que_cambian(
    antes: &BTreeMap<String, String>,
    despues: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut out: Vec<String> = antes
        .keys()
        .chain(despues.keys())
        .filter(|r| antes.get(*r) != despues.get(*r))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

fn activos<'a>(pkg: &'a Package, raiz: &Path) -> BTreeMap<String, Activo<'a>> {
    pkg.docs
        .iter()
        .filter(|d| d.qname().is_some())
        .map(|d| {
            let ruta = d
                .path
                .strip_prefix(raiz)
                .unwrap_or(&d.path)
                .to_string_lossy()
                .replace('\\', "/");
            (
                ore_core::normalize::doc_id(d),
                Activo {
                    doc: d,
                    canonico: ore_core::normalize::document(d).jcs(),
                    anexos: anexos(d, &ruta),
                    ruta,
                },
            )
        })
        .collect()
}

/// Las columnas de un activo con su tipo si lo dice (`""` si no): `columns`,
/// o `fields` en una vista de antes de v1alpha14. Una columna de una `Table`
/// puede no llevar tipo (`id: {}`), y sigue siendo una columna.
fn columnas(d: &Loaded) -> BTreeMap<String, String> {
    let seccion = d.section("columns").or_else(|| d.section("fields"));
    seccion
        .map(|c| {
            c.entries()
                .iter()
                .filter_map(|(k, v)| {
                    let tipo = v
                        .get("type")
                        .and_then(|(_, t)| t.as_str())
                        .unwrap_or_default();
                    Some((k.as_str()?.to_string(), tipo.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Qué claves de `metadata` y `spec` cambian entre dos formas canónicas.
fn claves_que_cambian(antes: &Json, despues: &Json) -> Vec<String> {
    let mut out = Vec::new();
    for seccion in ["metadata", "spec"] {
        let (a, b) = (hijo(antes, seccion), hijo(despues, seccion));
        let claves: BTreeSet<&String> = a.keys().chain(b.keys()).copied().collect();
        for k in claves {
            if a.get(k).map(|j| j.jcs()) != b.get(k).map(|j| j.jcs()) {
                out.push(format!("{seccion}.{k}"));
            }
        }
    }
    out
}

fn hijo<'a>(j: &'a Json, k: &str) -> BTreeMap<&'a String, &'a Json> {
    match j {
        Json::Obj(m) => match m.get(k) {
            Some(Json::Obj(h)) => h.iter().collect(),
            _ => BTreeMap::new(),
        },
        _ => BTreeMap::new(),
    }
}

fn sql(d: &Loaded) -> Option<String> {
    d.section("sql")
        .and_then(|s| s.as_str())
        .map(str::to_string)
}

/// Quién lee a quién, por identidad: `leido → lectores` (lo que una vista o un
/// dataset lee directamente, y lo que respalda a una entidad).
fn lectores(pkg: &Package) -> BTreeMap<String, BTreeSet<String>> {
    let mut g: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for d in &pkg.docs {
        let yo = ore_core::normalize::doc_id(d);
        let mut lee = ore_core::vistas::lee_directo(pkg, d);
        if let Some(r) = ore_core::vistas::respaldo(pkg, d) {
            lee.push(r);
        }
        for f in lee {
            g.entry(ore_core::normalize::doc_id(f))
                .or_default()
                .insert(yo.clone());
        }
    }
    g
}

/// Lo que lee cada activo, directamente: `activo → leídos` (lo mismo que
/// [`lectores`], del revés). Es lo que una propuesta de activos tiene que
/// llevar consigo si la rama también lo cambió (`propuestas.rs`, la expansión).
///
/// Y un `Package`, lo que EXPORTA: su `exports` nombra documentos suyos, y un
/// `package.yaml` que exporta un puntero que no va no compila (OOS2027, medido
/// 2026-09-28 proponiendo uno de dos datasets de una base standard nueva).
fn lee(pkg: &Package) -> BTreeMap<String, Vec<String>> {
    let mut g: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in &pkg.docs {
        let mut lee = ore_core::vistas::lee_directo(pkg, d);
        if let Some(r) = ore_core::vistas::respaldo(pkg, d) {
            lee.push(r);
        }
        lee.extend(exporta(pkg, d));
        if lee.is_empty() {
            continue;
        }
        let mut ids: Vec<String> = lee.iter().map(|f| ore_core::normalize::doc_id(f)).collect();
        ids.sort();
        ids.dedup();
        g.insert(ore_core::normalize::doc_id(d), ids);
    }
    g
}

/// Lo que un `Package` exporta: los documentos de su sitio que su `exports`
/// nombra (la forma corta, cualificada con su namespace como en `exporta.rs`).
fn exporta<'a>(pkg: &'a Package, p: &'a Loaded) -> Vec<&'a Loaded> {
    if p.kind != ore_core::document::Kind::Package {
        return Vec::new();
    }
    let (Some(sitio), Some(v)) = (p.path.parent(), p.section("exports")) else {
        return Vec::new();
    };
    let ns = p.meta("namespace").and_then(|n| n.as_str());
    let nombres: BTreeSet<String> = v
        .items()
        .iter()
        .filter_map(|i| i.as_str())
        .map(|s| {
            ore_core::normalize::qualify_catalogo(s, ns, ore_core::normalize::SCHEMA_POR_DEFECTO)
        })
        .collect();
    pkg.docs
        .iter()
        .filter(|d| d.kind != ore_core::document::Kind::Package && d.path.starts_with(sitio))
        .filter(|d| d.qname().is_some_and(|q| nombres.contains(&q)))
        .collect()
}

/// Todo lo que lee `id`, directa o indirectamente, sin él.
fn alcanza(g: &BTreeMap<String, BTreeSet<String>>, id: &str) -> Vec<String> {
    let mut visto = BTreeSet::new();
    let mut cola: VecDeque<&str> = VecDeque::from([id]);
    while let Some(x) = cola.pop_front() {
        for l in g.get(x).into_iter().flatten() {
            if visto.insert(l.clone()) {
                cola.push_back(l);
            }
        }
    }
    visto.remove(id);
    visto.into_iter().collect()
}

/// `ore diff <antes> <despues>`, tal cual: `changes`, `requiredBump`, `verdicts`.
fn semantico(binario: &Path, antes: &Path, despues: &Path) -> Option<Json> {
    let s = crate::mando::correr(
        binario,
        despues,
        &[
            "diff".into(),
            antes.to_string_lossy().into_owned(),
            despues.to_string_lossy().into_owned(),
        ],
    )
    .ok()?;
    // `ore diff` sale 1 cuando hay cambios, como `diff`: no es un fallo.
    if s.codigo != 0 && s.codigo != 1 {
        return None;
    }
    ore_core::parse::parse(&s.stdout)
        .ok()
        .map(|n| crate::rutas::de_node(&n))
}

fn campo<'a>(j: &'a Json, k: &str) -> Option<&'a Json> {
    match j {
        Json::Obj(m) => m.get(k),
        _ => None,
    }
}

fn texto(j: Option<&Json>) -> Option<&str> {
    match j {
        Some(Json::Str(s)) => Some(s),
        _ => None,
    }
}

impl Servidor {
    /// `GET /ramas/{rama}/cambios`.
    pub(crate) fn cambios(&self, rama: &str) -> Respuesta {
        if let Err(m) = crate::propuestas::nombre_de_rama_valido(rama) {
            return Respuesta::error(422, m);
        }
        let Arbol::Forja(forja) = &self.arbol else {
            return Respuesta::error(
                422,
                "este árbol es un directorio, no una forja: no hay ramas que comparar",
            );
        };
        let base = self
            .api()
            .ok()
            .and_then(|a| a.rama_por_defecto().ok())
            .unwrap_or_else(|| "main".into());
        if rama == base {
            return Respuesta::error(
                422,
                format!("`{rama}` es la rama por defecto: no tiene cambios frente a sí misma"),
            );
        }
        // De memoria, por las dos cabezas (sin clonar).
        let clave = match (forja.cabeza_de(&base), forja.cabeza_de(rama)) {
            (Some(b), Some(r)) => Some((format!("cambios:{rama}"), format!("{b}..{r}"))),
            _ => None,
        };
        if let Some(k) = &clave
            && let Some(j) = self.cambios_cache.lee(k)
        {
            let mut j = (*j).clone();
            if let Json::Obj(m) = &mut j {
                m.insert("desde_cache".into(), Json::Bool(true));
            }
            return Respuesta::ok(j);
        }
        let clon = match forja.clonar_rama(Some(rama)) {
            Err(crate::git::Fallo::SinRama(r)) => {
                return Respuesta::error(404, format!("no hay ninguna rama `{r}`"));
            }
            Err(e) => return Respuesta::error(502, e.to_string()),
            Ok(p) => p,
        };
        let frente = match forja.frente_a(clon.ruta(), &base) {
            Ok(f) => f,
            Err(e) => return Respuesta::error(502, e.to_string()),
        };
        let antes = match forja.extraer(clon.ruta(), &frente.desde) {
            Ok(p) => p,
            Err(e) => return Respuesta::error(502, e.to_string()),
        };
        // La base de hoy, sólo si se movió desde el punto de partida: sin eso no
        // hay nada que pueda chocar. `frente_a` la dejó en `FETCH_HEAD`.
        let hoy = if frente.atras > 0 {
            match forja.extraer(clon.ruta(), "FETCH_HEAD") {
                Ok(p) => Some(p),
                Err(e) => return Respuesta::error(502, e.to_string()),
            }
        } else {
            None
        };
        let j = self.comparar(
            rama,
            &base,
            &frente,
            antes.ruta(),
            clon.ruta(),
            hoy.as_ref().map(|p| p.ruta()),
        );
        if let Some(k) = clave {
            self.cambios_cache.guarda(k, Arc::new(j.clone()));
        }
        let mut j = j;
        if let Json::Obj(m) = &mut j {
            m.insert("desde_cache".into(), Json::Bool(false));
        }
        Respuesta::ok(j)
    }

    fn comparar(
        &self,
        rama: &str,
        base: &str,
        frente: &crate::git::Frente,
        antes: &Path,
        despues: &Path,
        base_hoy: Option<&Path>,
    ) -> Json {
        let (pkg_a, _) = ore_core::validate::cargar_paquete(antes);
        let (pkg_d, _) = ore_core::validate::cargar_paquete(despues);
        let (a, d) = (activos(&pkg_a, antes), activos(&pkg_d, despues));
        // La base de hoy, si se movió: sus activos, para ver cuáles tocó también.
        let pkg_h = base_hoy.map(|h| ore_core::validate::cargar_paquete(h).0);
        let h = match (&pkg_h, base_hoy) {
            (Some(p), Some(r)) => Some(activos(p, r)),
            _ => None,
        };
        let (g_a, g_d) = (lectores(&pkg_a), lectores(&pkg_d));
        let lee_d = lee(&pkg_d);

        // Lo que rompe, por activo: el sujeto de un cambio de `ore diff` es
        // `<nombre del activo>.<lo de dentro>`; se ata al nombre más largo.
        let sem = semantico(&self.binario, antes, despues);
        let mut por_activo: BTreeMap<String, Vec<Json>> = BTreeMap::new();
        let mut sueltos = Vec::new();
        let nombres: Vec<(String, String)> = d
            .iter()
            .chain(a.iter())
            .filter_map(|(id, x)| x.doc.qname().map(|q| (q, id.clone())))
            .collect();
        if let Some(Json::Arr(cs)) = sem.as_ref().and_then(|s| campo(s, "changes")) {
            for c in cs {
                let dueno = texto(campo(c, "subject")).and_then(|s| {
                    nombres
                        .iter()
                        .filter(|(q, _)| s == q || s.starts_with(&format!("{q}.")))
                        .max_by_key(|(q, _)| q.len())
                        .map(|(_, id)| id.clone())
                });
                match dueno {
                    Some(id) => por_activo.entry(id).or_default().push(c.clone()),
                    None => sueltos.push(c.clone()),
                }
            }
        }

        let ids: BTreeSet<&String> = a.keys().chain(d.keys()).collect();
        let (mut nuevos, mut modificados, mut borrados, mut rompen) = (0, 0, 0, 0);
        let mut conflictos = 0;
        let mut cambios = Vec::new();
        for id in ids {
            let (x, y) = (a.get(id), d.get(id));
            let vacio = BTreeMap::new();
            let ficheros = anexos_que_cambian(
                x.map(|x| &x.anexos).unwrap_or(&vacio),
                y.map(|y| &y.anexos).unwrap_or(&vacio),
            );
            let estado = match (x, y) {
                (None, Some(_)) => "nuevo",
                (Some(_), None) => "borrado",
                (Some(x), Some(y)) if x.canonico != y.canonico => "modificado",
                (Some(_), Some(_)) if !ficheros.is_empty() => "modificado",
                (Some(x), Some(y)) if x.ruta != y.ruta => "movido",
                _ => continue,
            };
            match estado {
                "nuevo" => nuevos += 1,
                "borrado" => borrados += 1,
                _ => modificados += 1,
            }
            let doc = y.or(x).unwrap().doc;
            let mut m: Vec<(&'static str, Json)> = vec![
                ("id", Json::s(id.as_str())),
                // La dirección del activo en el índice del catálogo (`GET
                // /assets`): con ella la consola casa el cambio con su hoja.
                ("ref", Json::s(ore_core::assets::ref_doc(doc))),
                ("kind", Json::s(doc.kind.as_str())),
                ("nombre", Json::s(doc.qname().unwrap_or_default())),
                ("estado", Json::s(estado)),
                ("ruta", Json::s(y.or(x).unwrap().ruta.as_str())),
            ];
            // Lo que viaja con él sin ser documento: los `discover.*` de su base.
            if !ficheros.is_empty() {
                m.push((
                    "ficheros",
                    Json::Arr(ficheros.iter().map(Json::s).collect()),
                ));
            }
            // ¿La base también lo cambió desde el punto de partida? Y qué le hizo.
            let en_base = h.as_ref().and_then(|h| {
                match (x.map(|x| &x.canonico), h.get(id).map(|z| &z.canonico)) {
                    (None, Some(_)) => Some("nuevo"),
                    (Some(_), None) => Some("borrado"),
                    (Some(a), Some(z)) if a != z => Some("modificado"),
                    _ => None,
                }
            });
            // Lo que lee en la rama (si sigue en ella): lo que una propuesta
            // suya tiene que llevar si la rama también lo cambió.
            if y.is_some() {
                m.push((
                    "lee",
                    Json::Arr(
                        lee_d
                            .get(id.as_str())
                            .into_iter()
                            .flatten()
                            .map(Json::s)
                            .collect(),
                    ),
                ));
            }
            m.push(("conflicto", Json::Bool(en_base.is_some())));
            if let Some(e) = en_base {
                conflictos += 1;
                m.push(("enBase", Json::s(e)));
            }
            if let (Some(x), Some(y)) = (x, y) {
                if x.ruta != y.ruta {
                    m.push(("rutaAntes", Json::s(x.ruta.as_str())));
                }
                let (ca, cd) = (
                    ore_core::normalize::document(x.doc),
                    ore_core::normalize::document(y.doc),
                );
                m.push((
                    "campos",
                    Json::Arr(
                        claves_que_cambian(&ca, &cd)
                            .into_iter()
                            .map(Json::s)
                            .collect(),
                    ),
                ));
            }
            // Las columnas: las de antes frente a las de ahora.
            let (col_a, col_d) = (
                x.map(|x| columnas(x.doc)).unwrap_or_default(),
                y.map(|y| columnas(y.doc)).unwrap_or_default(),
            );
            if !(col_a.is_empty() && col_d.is_empty()) && col_a != col_d {
                let anadidas: Vec<Json> = col_d
                    .keys()
                    .filter(|k| !col_a.contains_key(*k))
                    .map(|k| Json::s(k.as_str()))
                    .collect();
                let quitadas: Vec<Json> = col_a
                    .keys()
                    .filter(|k| !col_d.contains_key(*k))
                    .map(|k| Json::s(k.as_str()))
                    .collect();
                let cambiadas: Vec<Json> = col_a
                    .iter()
                    .filter_map(|(k, t)| {
                        let u = col_d.get(k)?;
                        (u != t).then(|| {
                            Json::obj([
                                ("columna", Json::s(k.as_str())),
                                ("antes", Json::s(t.as_str())),
                                ("despues", Json::s(u.as_str())),
                            ])
                        })
                    })
                    .collect();
                m.push((
                    "columnas",
                    Json::obj([
                        ("anadidas", Json::Arr(anadidas)),
                        ("quitadas", Json::Arr(quitadas)),
                        ("cambiadas", Json::Arr(cambiadas)),
                    ]),
                ));
            }
            let (sa, sd) = (x.and_then(|x| sql(x.doc)), y.and_then(|y| sql(y.doc)));
            if sa != sd {
                m.push((
                    "sql",
                    Json::obj([
                        ("antes", sa.map(Json::s).unwrap_or(Json::Bool(false))),
                        ("despues", sd.map(Json::s).unwrap_or(Json::Bool(false))),
                    ]),
                ));
            }
            let suyos = por_activo.remove(id.as_str()).unwrap_or_default();
            // `ore diff` sólo emite lo que rompe (un eje está roto si tiene
            // algún cambio): un cambio suyo, o haberlo borrado, rompe.
            let rompe = estado == "borrado" || !suyos.is_empty();
            if rompe {
                rompen += 1;
            }
            // A quién alcanza: si rompe, quién lo leía (en la base, si se
            // borró; en la rama, si sigue).
            let afecta = if rompe {
                if estado == "borrado" {
                    alcanza(&g_a, id)
                } else {
                    alcanza(&g_d, id)
                }
            } else {
                Vec::new()
            };
            m.push(("semantico", Json::Arr(suyos)));
            m.push(("rompe", Json::Bool(rompe)));
            m.push((
                "afecta",
                Json::Arr(afecta.into_iter().map(Json::s).collect()),
            ));
            cambios.push(Json::obj(m));
        }
        // Lo que `ore diff` ató a un activo que no cambió de documento (p. ej.
        // la versión del paquete) va con lo suelto.
        for (_, v) in por_activo {
            sueltos.extend(v);
        }

        let diagnosticos = self
            .diagnosticos_de(despues)
            .map(|ds| Json::Arr(ds.iter().map(crate::arbol::con_posicion).collect()))
            .unwrap_or(Json::Arr(vec![]));
        let corto = |c: &str| c.chars().take(12).collect::<String>();
        Json::obj([
            ("rama", Json::s(rama)),
            ("base", Json::s(base)),
            ("desde", Json::s(corto(&frente.desde))),
            ("cabeza", Json::s(corto(&frente.cabeza))),
            ("adelante", Json::Int(frente.adelante as i64)),
            ("atras", Json::Int(frente.atras as i64)),
            (
                "resumen",
                Json::obj([
                    ("nuevos", Json::Int(nuevos)),
                    ("modificados", Json::Int(modificados)),
                    ("borrados", Json::Int(borrados)),
                    ("rompen", Json::Int(rompen)),
                    ("conflictos", Json::Int(conflictos)),
                ]),
            ),
            ("cambios", Json::Arr(cambios)),
            (
                "semantico",
                Json::obj([
                    (
                        "requiredBump",
                        sem.as_ref()
                            .and_then(|s| campo(s, "requiredBump"))
                            .cloned()
                            .unwrap_or(Json::Bool(false)),
                    ),
                    (
                        "verdicts",
                        sem.as_ref()
                            .and_then(|s| campo(s, "verdicts"))
                            .cloned()
                            .unwrap_or(Json::Bool(false)),
                    ),
                    ("otros", Json::Arr(sueltos)),
                ]),
            ),
            ("diagnosticos", diagnosticos),
        ])
    }
}

#[cfg(test)]
mod pruebas {
    use std::collections::BTreeMap;

    /// La #9 de t-victor: el paquete ya estaba en `main`, igual, pero sin sus
    /// `discover.*`; la rama los tiene. Eso es un cambio del `Package`.
    #[test]
    fn los_discover_que_faltan_en_la_base_son_un_cambio() {
        let r = |f: &str| format!("packages/s3/{f}");
        let rama: BTreeMap<String, String> = super::DE_LA_BASE
            .iter()
            .map(|f| (r(f), "{}".to_string()))
            .collect();
        let main = BTreeMap::new();
        assert_eq!(super::anexos_que_cambian(&main, &rama).len(), 4);
        assert!(super::anexos_que_cambian(&rama, &rama).is_empty());
        let mut otra = rama.clone();
        otra.insert(r("discover.answers.json"), "{\"a\":1}".into());
        assert_eq!(
            super::anexos_que_cambian(&rama, &otra),
            vec![r("discover.answers.json")]
        );
    }
}
