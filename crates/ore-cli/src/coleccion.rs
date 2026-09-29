//! **La transacción de una colección** (0046 E8·1c): lo que `ore materialize`
//! hace con una `MediaCollection` mantenida, como con un `Dataset`.
//!
//! # El manifiesto es una tabla de sus ítems (E8·1a, medido)
//!
//! Una fila por **(clave, versión)** del origen, con su huella de contenido,
//! su formato, su tamaño, su **estado** —`actual`, `retirado`, `perdido`— y la
//! transacción en que entró y en la que salió. Se sella en el lago como un
//! dataset —Iceberg, `sellar` con fusión por la clave—, así que hereda el
//! puntero con CAS, la historia, `volcar`, `/v1` y SQL. Medido con 100.000
//! ítems: una transacción de tres cambios, 0,64 s; leer lo actual, 0,18 s.
//!
//! # La transacción es la diferencia de dos listados de versiones (E7)
//!
//! - lo vigente que no estaba **entra**;
//! - lo actual que ya no es vigente **se retira**: sale de la vista actual y
//!   se queda en la tabla (spec `02` §5). Sobrescribir es retirar y entrar:
//!   la clave es la misma, la versión no;
//! - lo retirado cuya versión ya no existe en el origen está **perdido**: una
//!   colección virtual no puede servirlo, y se dice en vez de fingirlo. Lo que
//!   sigue existiendo como versión vieja (una marca de borrado encima, o una
//!   versión nueva) se sigue pudiendo servir por su `VersionId`;
//! - si el testigo del listado no cambió, **no se lee ni se escribe nada**.
//!
//! Sólo la **virtual** en E8·1: la que copia los bytes llega con E8·2, y hasta
//! entonces se dice y se deja.

use crate::lector;
use crate::materializar::{almacen, cabecera, campo_de, leer_puntero, programa_del_almacen};
use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::link::{Loaded, Package};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Las columnas del manifiesto, con su tipo de OOS.
const COLUMNAS: &[(&str, &str)] = &[
    ("clave", "String"),
    ("camino", "String"),
    ("version", "String"),
    ("etag", "String"),
    ("huella", "String"),
    ("formato", "String"),
    ("tamano", "Integer"),
    ("modificado", "String"),
    ("estado", "String"),
    ("entro", "Integer"),
    ("retirado", "Integer"),
];

/// Un ítem del manifiesto: una fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub clave: String,
    pub camino: String,
    pub version: String,
    pub etag: String,
    pub huella: String,
    pub formato: String,
    pub tamano: i64,
    pub modificado: String,
    pub estado: String,
    pub entro: i64,
    pub retirado: Option<i64>,
}

impl Item {
    fn id(&self) -> (String, String) {
        (self.clave.clone(), self.version.clone())
    }

    fn fila(&self) -> String {
        let mut m: BTreeMap<String, Json> = BTreeMap::new();
        for (k, v) in [
            ("clave", &self.clave),
            ("camino", &self.camino),
            ("version", &self.version),
            ("etag", &self.etag),
            ("huella", &self.huella),
            ("formato", &self.formato),
            ("modificado", &self.modificado),
            ("estado", &self.estado),
        ] {
            m.insert(k.into(), Json::s(v));
        }
        m.insert("tamano".into(), Json::s(self.tamano.to_string()));
        m.insert("entro".into(), Json::s(self.entro.to_string()));
        if let Some(r) = self.retirado {
            m.insert("retirado".into(), Json::s(r.to_string()));
        }
        Json::Obj(m).jcs()
    }

    fn de_fila(n: &ore_core::parse::Node) -> Option<Item> {
        let c = |k: &str| campo_de(n, k).unwrap_or_default();
        let e = |k: &str| campo_de(n, k).and_then(|v| v.parse::<i64>().ok());
        Some(Item {
            clave: campo_de(n, "clave")?,
            camino: c("camino"),
            version: campo_de(n, "version")?,
            etag: c("etag"),
            huella: c("huella"),
            formato: c("formato"),
            tamano: e("tamano").unwrap_or(0),
            modificado: c("modificado"),
            estado: campo_de(n, "estado")?,
            entro: e("entro").unwrap_or(0),
            retirado: e("retirado"),
        })
    }
}

/// Lo vigente en el origen, como lo da `ore-read-<tipo> versiones`.
#[derive(Debug, Clone)]
pub struct Vigente {
    pub clave: String,
    pub camino: String,
    pub version: String,
    pub etag: String,
    pub tamano: i64,
    pub modificado: String,
    pub huella: String,
}

/// El formato de un camino: su extensión, en minúsculas, con los dos nombres
/// de siempre juntados (`jpeg` es `jpg`, `tif` es `tiff`).
pub fn formato_de(camino: &str) -> String {
    let base = camino.rsplit('/').next().unwrap_or(camino);
    let ext = match base.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    };
    match ext.as_str() {
        "jpeg" => "jpg".into(),
        "tif" => "tiff".into(),
        _ => ext,
    }
}

/// Lo que cambia en una transacción: las filas que se escriben y las cuentas.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cambios {
    pub filas: Vec<Item>,
    pub entran: usize,
    pub retiran: usize,
    pub pierden: usize,
    pub fuera_de_formato: usize,
}

/// **La transacción, pura**: de lo que había (`antes`, todas sus filas), lo
/// vigente, qué versiones conocidas siguen existiendo y los formatos de la
/// colección, a las filas que cambian. Determinista: mismo estado, mismas
/// filas.
pub fn transaccion(
    antes: &[Item],
    vigentes: &[Vigente],
    existen: &BTreeSet<(String, String)>,
    formatos: &[String],
    tx: i64,
) -> Cambios {
    let mut c = Cambios::default();
    let admitido = |camino: &str| {
        let f = formato_de(camino);
        formatos.is_empty() || formatos.iter().any(|x| formato_de(&format!("x.{x}")) == f)
    };
    let vivos: BTreeMap<(String, String), &Vigente> = vigentes
        .iter()
        .filter(|v| {
            let ok = admitido(&v.camino);
            if !ok {
                c.fuera_de_formato += 1;
            }
            ok
        })
        .map(|v| ((v.clave.clone(), v.version.clone()), v))
        .collect();
    let previos: BTreeMap<(String, String), &Item> = antes.iter().map(|i| (i.id(), i)).collect();

    for (id, i) in &previos {
        match (i.estado.as_str(), vivos.contains_key(id)) {
            // Sigue: nada que escribir.
            ("actual", true) => {}
            // Ya no es vigente: se retira, o se pierde si su versión no está.
            ("actual", false) => {
                let perdido = !existen.contains(id);
                if perdido {
                    c.pierden += 1;
                } else {
                    c.retiran += 1;
                }
                c.filas.push(Item {
                    estado: if perdido { "perdido" } else { "retirado" }.into(),
                    retirado: Some(tx),
                    ..(*i).clone()
                });
            }
            // Vuelve a ser vigente (se quitó la marca de borrado): entra otra vez.
            ("retirado" | "perdido", true) => {
                c.entran += 1;
                c.filas.push(Item {
                    estado: "actual".into(),
                    entro: tx,
                    retirado: None,
                    ..(*i).clone()
                });
            }
            // Retirado cuya versión ya no existe: perdido.
            ("retirado", false) if !existen.contains(id) => {
                c.pierden += 1;
                c.filas.push(Item {
                    estado: "perdido".into(),
                    ..(*i).clone()
                });
            }
            _ => {}
        }
    }
    for (id, v) in &vivos {
        if previos.contains_key(id) {
            continue;
        }
        c.entran += 1;
        c.filas.push(Item {
            clave: v.clave.clone(),
            camino: v.camino.clone(),
            version: v.version.clone(),
            etag: v.etag.clone(),
            huella: v.huella.clone(),
            formato: formato_de(&v.camino),
            tamano: v.tamano,
            modificado: v.modificado.clone(),
            estado: "actual".into(),
            entro: tx,
            retirado: None,
        });
    }
    c.filas.sort_by_key(Item::id);
    c
}

/// Las colecciones mantenidas del árbol: las que tienen `from`.
pub fn mantenidas(pkg: &Package) -> Vec<&Loaded> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::MediaCollection && d.section("from").is_some())
        .collect()
}

pub fn es_virtual(d: &Loaded) -> bool {
    d.section("virtual").and_then(|v| v.as_str()) == Some("true")
}

/// El `ObjectTable` del que sale una colección.
fn objeto_de<'a>(pkg: &'a Package, d: &Loaded) -> Result<&'a Loaded, String> {
    let r = d
        .section("from")
        .and_then(|f| f.get("objectTable"))
        .and_then(|(_, v)| v.as_str())
        .ok_or("la colección no dice de qué `objectTable` sale")?;
    let q = ore_core::normalize::a_corto(&ore_core::link::cualificar(r, d)).into_owned();
    pkg.docs
        .iter()
        .find(|o| o.kind == Kind::ObjectTable && o.qname().as_deref() == Some(q.as_str()))
        .ok_or_else(|| format!("`{r}` no es un `ObjectTable` del árbol"))
}

/// Las filas del manifiesto vigente, leídas del lago.
fn manifiesto(dataset: &str, ml: &str) -> Result<Vec<Item>, String> {
    let peticion = Json::obj([
        ("dataset", Json::s(dataset)),
        ("metadata_location", Json::s(ml)),
    ])
    .jcs();
    let texto = lector::ejecutar(&programa_del_almacen()?, &["leer".into()], Some(&peticion))
        .map_err(|f| format!("no se pudo leer el manifiesto: {}", f.mensaje))?;
    Ok(texto
        .lines()
        .filter_map(|l| ore_core::parse::parse(l.trim()).ok())
        .filter_map(|n| Item::de_fila(&n))
        .collect())
}

/// **Una colección, una pasada**: la línea del informe y el puntero nuevo, o
/// `None` si no hay nada que hacer todavía (la que copia bytes, E8·2).
#[allow(clippy::too_many_arguments)]
pub fn una(
    pkg: &Package,
    raiz: &Path,
    d: &Loaded,
    qn: &str,
    punteros: &Path,
    seco: bool,
    rehacer: bool,
    bundle: &str,
) -> Result<Option<(String, Json)>, String> {
    if !es_virtual(d) {
        return Ok(Some((
            "la colección que copia los ficheros llega con 0046 E8·2; hoy se mantiene la virtual"
                .into(),
            Json::obj([
                ("kind", Json::s("MediaCollection")),
                ("estado", Json::s("pendiente")),
                ("motivo", Json::s("copiar los ficheros llega con 0046 E8·2")),
            ]),
        )));
    }
    let o = objeto_de(pkg, d)?;
    let datasource = o
        .section("datasource")
        .and_then(|v| v.as_str())
        .ok_or("el `ObjectTable` no dice su `datasource`")?
        .to_string();
    let prefijo = o
        .section("prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let patrones: Vec<String> = [
        o.section("match").and_then(|v| v.as_str()),
        d.section("from")
            .and_then(|f| f.get("match"))
            .and_then(|(_, v)| v.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(String::from)
    .collect();
    let formatos: Vec<String> = d
        .section("formats")
        .map(|f| {
            f.items()
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    let puntero = leer_puntero(punteros, qn);
    let ml = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "metadata_location"));
    let dataset = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "dataset"))
        .unwrap_or_else(|| {
            ore_core::punteros::dataset_nuevo(qn).replacen("catalogo/", "colecciones/", 1)
        });
    let antes: Vec<Item> = match &ml {
        Some(m) if !rehacer => manifiesto(&dataset, m)?,
        _ => Vec::new(),
    };
    let tx = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "transaccion"))
        .and_then(|t| t.parse::<i64>().ok())
        .filter(|_| ml.is_some() && !rehacer)
        .map(|t| t + 1)
        .unwrap_or(1);

    // El listado de versiones, del lector de la fuente.
    let (tipo, env) = lector::declaracion(raiz, &datasource)
        .map_err(|f| format!("la fuente `{datasource}` · {}", f.mensaje))?;
    let url = lector::url(raiz, &env, &datasource)
        .map_err(|f| format!("la fuente `{datasource}` · {}", f.mensaje))?;
    let peticion = Json::obj([
        ("url", Json::s(&url)),
        ("objeto", Json::s(&prefijo)),
        (
            "patrones",
            Json::Arr(patrones.iter().map(Json::s).collect()),
        ),
        (
            "conocidos",
            Json::Arr(
                antes
                    .iter()
                    .map(|i| {
                        Json::Arr(vec![
                            Json::s(&i.clave),
                            Json::s(&i.version),
                            Json::s(&i.huella),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
    .jcs();
    let salida = lector::ejecutar(
        &format!("ore-read-{tipo}"),
        &["versiones".into()],
        Some(&peticion),
    )
    .map_err(|f| format!("`ore-read-{tipo} versiones`: {}", f.mensaje))?;
    let r = ore_core::parse::parse(salida.trim())
        .map_err(|e| format!("lo que devolvió `ore-read-{tipo} versiones` no analiza: {e:?}"))?;
    let testigo = campo_de(&r, "testigo").unwrap_or_default();
    let huellas = campo_de(&r, "huellas").unwrap_or_else(|| "0".into());

    // ④ El listado no cambió: nada que leer ni escribir.
    let testigo_antes = puntero
        .as_ref()
        .and_then(|p| p.get("testigo"))
        .and_then(|(_, t)| campo_de(t, "valor"));
    if !rehacer && ml.is_some() && testigo_antes.as_deref() == Some(testigo.as_str()) {
        let mut m = match puntero.as_ref().map(Json::de_node) {
            Some(Json::Obj(m)) => m,
            _ => Default::default(),
        };
        m.insert("estado".into(), Json::s("al-dia"));
        m.insert("bundle".into(), Json::s(bundle));
        return Ok(Some((
            format!("al día · el listado no cambió (transacción {})", tx - 1),
            Json::Obj(m),
        )));
    }

    let vigentes: Vec<Vigente> = r
        .get("items")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|i| {
            Some(Vigente {
                clave: campo_de(i, "clave")?,
                camino: campo_de(i, "camino").unwrap_or_default(),
                version: campo_de(i, "version")?,
                etag: campo_de(i, "etag").unwrap_or_default(),
                tamano: campo_de(i, "tamano")
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0),
                modificado: campo_de(i, "modificado").unwrap_or_default(),
                huella: campo_de(i, "huella").unwrap_or_default(),
            })
        })
        .collect();
    let existen: BTreeSet<(String, String)> = r
        .get("existen")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| match x.items() {
            [c, v] => Some((c.as_str()?.to_string(), v.as_str()?.to_string())),
            _ => None,
        })
        .collect();
    let c = transaccion(&antes, &vigentes, &existen, &formatos, tx);
    let cuenta = |estado: &str| -> i64 {
        let mut vivos: BTreeMap<(String, String), &str> =
            antes.iter().map(|i| (i.id(), i.estado.as_str())).collect();
        for f in &c.filas {
            vivos.insert(f.id(), f.estado.as_str());
        }
        vivos.values().filter(|e| **e == estado).count() as i64
    };
    let (actuales, retirados, perdidos) = (cuenta("actual"), cuenta("retirado"), cuenta("perdido"));
    let linea = format!(
        "transacción {tx} · {} entran, {} se retiran, {} se pierden · {actuales} actuales, \
         {retirados} retirados, {perdidos} perdidos · {huellas} huellas pedidas{}",
        c.entran,
        c.retiran,
        c.pierden,
        if c.fuera_de_formato > 0 {
            format!(" · {} fuera de sus formatos", c.fuera_de_formato)
        } else {
            String::new()
        }
    );
    if seco {
        return Ok(Some((format!("haría la {linea}"), Json::obj([]))));
    }

    let esquema: BTreeMap<String, ore_core::types::Type> = COLUMNAS
        .iter()
        .filter_map(|(c, t)| Some((c.to_string(), ore_core::types::parse_type(t).ok()?)))
        .collect();
    let plan = ore_core::digest::de_bytes(format!("coleccion:{qn}").as_bytes());
    let t = ("listing".to_string(), Some(testigo.clone()));
    let clave = vec!["clave".to_string(), "version".to_string()];
    // Sobre el manifiesto que hay, siempre (rehacer es sobrescribir: un
    // snapshot nuevo, y la historia se queda); fundir, si había filas.
    let base = ml.as_deref();
    let fundir = base.is_some() && !rehacer && !antes.is_empty();
    let mut extra = format!("{{\"dataset\":\"{dataset}\",\"fundir\":{fundir},");
    if let Some(b) = base {
        extra.push_str(&format!("\"base\":\"{b}\","));
    }
    let peticion = cabecera(&plan, &esquema, &t, &clave).replacen('{', &extra, 1);
    let filas: String = c.filas.iter().map(|f| f.fila() + "\n").collect();
    let s = if c.filas.is_empty() && fundir {
        // El listado cambió y la colección no (lo nuevo está fuera de sus
        // formatos): el manifiesto se queda; sólo el testigo se mueve.
        None
    } else {
        Some(almacen("sellar", &peticion, Some(&filas))?)
    };
    let mut m: BTreeMap<String, Json> = match puntero.as_ref().map(Json::de_node) {
        Some(Json::Obj(m)) => m,
        _ => Default::default(),
    };
    m.remove("motivo");
    m.insert("kind".into(), Json::s("MediaCollection"));
    m.insert("virtual".into(), Json::Bool(true));
    m.insert("estado".into(), Json::s("transaccion"));
    m.insert("transaccion".into(), Json::Int(tx));
    m.insert("bundle".into(), Json::s(bundle));
    m.insert(
        "testigo".into(),
        Json::obj([("modo", Json::s("listing")), ("valor", Json::s(&testigo))]),
    );
    m.insert(
        "items".into(),
        Json::obj([
            ("actuales", Json::Int(actuales)),
            ("retirados", Json::Int(retirados)),
            ("perdidos", Json::Int(perdidos)),
        ]),
    );
    m.insert(
        "cambios".into(),
        Json::obj([
            ("entran", Json::Int(c.entran as i64)),
            ("retiran", Json::Int(c.retiran as i64)),
            ("pierden", Json::Int(c.pierden as i64)),
            ("fuera_de_formato", Json::Int(c.fuera_de_formato as i64)),
        ]),
    );
    if let Some(s) = s {
        for k in ["metadata_location", "snapshot", "bytes", "ficheros"] {
            if let Some(v) = campo_de(&s, k) {
                m.insert(
                    k.into(),
                    v.parse::<i64>()
                        .ok()
                        .filter(|_| k != "snapshot")
                        .map(Json::Int)
                        .unwrap_or(Json::s(v)),
                );
            }
        }
    }
    Ok(Some((linea, Json::Obj(m))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vig(clave: &str, version: &str, huella: &str) -> Vigente {
        Vigente {
            clave: clave.into(),
            camino: clave.into(),
            version: version.into(),
            etag: String::new(),
            tamano: 10,
            modificado: String::new(),
            huella: huella.into(),
        }
    }

    fn fijar(antes: &[Item], c: &Cambios) -> Vec<Item> {
        let mut m: BTreeMap<(String, String), Item> =
            antes.iter().map(|i| (i.id(), i.clone())).collect();
        for f in &c.filas {
            m.insert(f.id(), f.clone());
        }
        m.into_values().collect()
    }

    fn estado<'a>(items: &'a [Item], clave: &str, version: &str) -> &'a str {
        items
            .iter()
            .find(|i| i.clave == clave && i.version == version)
            .map(|i| i.estado.as_str())
            .unwrap_or("-")
    }

    /// **El experimento de E7, en dos transacciones.** Antes: `a.pdf` (a1),
    /// `b.pdf` y `c.jpg`. Después: `a.pdf` sobrescrito (a2), `b.pdf` borrado
    /// (su versión sigue), `c.jpg` renombrado a `c2.jpg` (su versión sigue),
    /// `d.pdf` nuevo con el contenido de otro; y un `.txt` que no es de la
    /// colección.
    #[test]
    fn el_experimento_de_e7_en_dos_transacciones() {
        let formatos = vec!["pdf".to_string(), "jpg".to_string()];
        let t1 = transaccion(
            &[],
            &[
                vig("a.pdf", "a1", "F"),
                vig("b.pdf", "b1", "R"),
                vig("c.jpg", "c1", "P"),
            ],
            &BTreeSet::new(),
            &formatos,
            1,
        );
        assert_eq!((t1.entran, t1.retiran, t1.pierden), (3, 0, 0));
        let antes = fijar(&[], &t1);

        let existen: BTreeSet<(String, String)> =
            [("a.pdf", "a1"), ("b.pdf", "b1"), ("c.jpg", "c1")]
                .iter()
                .map(|(c, v)| (c.to_string(), v.to_string()))
                .collect();
        let t2 = transaccion(
            &antes,
            &[
                vig("a.pdf", "a2", "R"),
                vig("c2.jpg", "c2", "P"),
                vig("d.pdf", "d1", "R"),
                vig("nota.txt", "t1", "T"),
            ],
            &existen,
            &formatos,
            2,
        );
        assert_eq!((t2.entran, t2.retiran, t2.pierden), (3, 3, 0));
        assert_eq!(t2.fuera_de_formato, 1);
        let despues = fijar(&antes, &t2);
        assert_eq!(
            estado(&despues, "a.pdf", "a1"),
            "retirado",
            "sobrescrito: la vieja se retira"
        );
        assert_eq!(estado(&despues, "a.pdf", "a2"), "actual");
        assert_eq!(
            estado(&despues, "b.pdf", "b1"),
            "retirado",
            "borrado: su versión sigue"
        );
        assert_eq!(estado(&despues, "c.jpg", "c1"), "retirado");
        assert_eq!(estado(&despues, "c2.jpg", "c2"), "actual");
        assert_eq!(estado(&despues, "d.pdf", "d1"), "actual");
        let a1 = despues.iter().find(|i| i.version == "a1").unwrap();
        assert_eq!((a1.entro, a1.retirado), (1, Some(2)));
        // El contenido repetido se ve por la huella: tres ítems, un contenido.
        let r: Vec<&str> = despues
            .iter()
            .filter(|i| i.huella == "R" && i.estado == "actual")
            .map(|i| i.clave.as_str())
            .collect();
        assert_eq!(r, ["a.pdf", "d.pdf"]);

        // Sin cambios: ninguna fila.
        let vivos: Vec<Vigente> = despues
            .iter()
            .filter(|i| i.estado == "actual")
            .map(|i| vig(&i.clave, &i.version, &i.huella))
            .collect();
        let t3 = transaccion(&despues, &vivos, &existen, &formatos, 3);
        assert!(t3.filas.is_empty(), "{:?}", t3.filas);
    }

    /// **Perdido**: lo que deja de ser vigente y cuya versión ya no existe (un
    /// origen sin versionado, o una versión purgada por el ciclo de vida del
    /// cliente). Y un retirado que se pierde después, también.
    #[test]
    fn lo_que_ya_no_existe_se_pierde_y_se_dice() {
        let t1 = transaccion(
            &[],
            &[vig("x.pdf", "null", "H"), vig("y.pdf", "null", "I")],
            &BTreeSet::new(),
            &[],
            1,
        );
        let antes = fijar(&[], &t1);
        let solo_y: BTreeSet<(String, String)> = [("y.pdf".to_string(), "null".to_string())].into();
        let t2 = transaccion(&antes, &[], &solo_y, &[], 2);
        assert_eq!((t2.retiran, t2.pierden), (1, 1));
        let d = fijar(&antes, &t2);
        assert_eq!(estado(&d, "x.pdf", "null"), "perdido");
        assert_eq!(estado(&d, "y.pdf", "null"), "retirado");
        let t3 = transaccion(&d, &[], &BTreeSet::new(), &[], 3);
        assert_eq!(t3.pierden, 1, "y.pdf ya no existe como versión");
        assert_eq!(estado(&fijar(&d, &t3), "y.pdf", "null"), "perdido");
    }

    #[test]
    fn el_formato_por_la_extension() {
        assert_eq!(formato_de("Nueva carpeta/Foto Portada 2026.JPG"), "jpg");
        assert_eq!(formato_de("x/y.jpeg"), "jpg");
        assert_eq!(formato_de("sin-extension"), "");
    }
}
