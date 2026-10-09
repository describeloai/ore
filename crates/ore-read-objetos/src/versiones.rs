//! **El listado de versiones de un `ObjectTable`** (0046 E8·1b): lo que una
//! colección necesita para su transacción.
//!
//! Lo que se lista es lo que hay **ahora** —la versión vigente de cada clave,
//! sin marca de borrado— con su versión, su ETag, su tamaño y su huella de
//! contenido. Medido en E7 contra el bucket de F1:
//!
//! - `ListObjectVersions` trae la versión en la misma petición que el
//!   listado (98 ms para 37): fijar cada ítem a su versión no cuesta nada;
//! - el checksum (`CRC64NVME`, `FULL_OBJECT`) **no** viene en el listado: es un
//!   `HEAD` por objeto. Por eso la petición trae lo que la colección ya
//!   conoce, y se pide **sólo para lo nuevo**: un segundo pase sin cambios no
//!   pide ninguna huella;
//! - una versión que dejó de ser la vigente —sobrescrita, o tapada por una
//!   marca de borrado— se sigue leyendo por su `VersionId` (206). Así que, de
//!   lo conocido, se dice qué **sigue existiendo** como versión: lo retirado
//!   que existe es `retirado` y se puede servir; lo que no, `perdido`. No hace
//!   falta preguntar si el bucket versiona (otro permiso): lo dice cada versión.
//!
//! # La petición y la respuesta
//!
//! Entra `{url, objeto, patrones, conocidos}` —el prefijo del `ObjectTable`,
//! los *glob* de su `match` y del de la colección (todos tienen que casar), y
//! `[[clave, version, huella], …]`—. Sale `{items, existen, testigo,
//! huellas}`: los ítems vigentes con su huella, qué conocidos siguen
//! existiendo, la huella del listado (clave, versión y ETag, ordenados) y
//! cuántas huellas se pidieron.

use crate::filas::casa;
use ore_core::json::Json;
use ore_objetos::Origen;
use std::collections::{BTreeMap, BTreeSet};

pub fn versiones(o: &dyn Origen, peticion: &str) -> Result<String, String> {
    let n: serde_json::Value =
        serde_json::from_str(peticion).map_err(|e| format!("la petición no es JSON: {e}"))?;
    let objeto = n.get("objeto").and_then(|v| v.as_str()).unwrap_or("");
    let patrones: Vec<&str> = n
        .get("patrones")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default();
    // (clave, versión) → huella
    let conocidos: BTreeMap<(String, String), String> = n
        .get("conocidos")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let x = x.as_array()?;
                    Some((
                        (
                            x.first()?.as_str()?.to_string(),
                            x.get(1)?.as_str()?.to_string(),
                        ),
                        x.get(2).and_then(|h| h.as_str()).unwrap_or("").to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    let todas = o.listar_versiones(objeto)?;
    let existentes: BTreeSet<(&str, &str)> = todas
        .iter()
        .filter(|v| !v.marca)
        .map(|v| (v.clave.as_str(), v.version.as_str()))
        .collect();
    let relativa =
        |clave: &str| -> String { clave.strip_prefix(objeto).unwrap_or(clave).to_string() };
    let mut vigentes: Vec<&ore_objetos::Version> = todas
        .iter()
        .filter(|v| v.actual && !v.marca && !v.clave.ends_with('/'))
        .filter(|v| v.clave.starts_with(objeto))
        .filter(|v| {
            let rel = relativa(&v.clave);
            patrones.iter().all(|p| casa(p, &rel))
        })
        .collect();
    vigentes.sort_by(|a, b| a.clave.cmp(&b.clave));

    let mut pedidas = 0usize;
    let mut items = Vec::with_capacity(vigentes.len());
    let mut firma = String::new();
    for v in vigentes {
        let huella = match conocidos.get(&(v.clave.clone(), v.version.clone())) {
            Some(h) if !h.is_empty() => h.clone(),
            _ => {
                pedidas += 1;
                o.huella_de(&v.clave, &v.version)?
                    .unwrap_or_else(|| format!("etag:{}", v.etag.trim_matches('"')))
            }
        };
        firma.push_str(&format!("{}\t{}\t{}\n", v.clave, v.version, v.etag));
        items.push(Json::obj([
            ("clave", Json::s(&v.clave)),
            ("camino", Json::s(relativa(&v.clave))),
            ("version", Json::s(&v.version)),
            ("etag", Json::s(v.etag.trim_matches('"'))),
            ("tamano", Json::Int(v.tamano as i64)),
            ("modificado", Json::s(&v.modificado)),
            ("huella", Json::s(huella)),
        ]));
    }
    let existen: Vec<Json> = conocidos
        .keys()
        .filter(|(c, v)| existentes.contains(&(c.as_str(), v.as_str())))
        .map(|(c, v)| Json::Arr(vec![Json::s(c), Json::s(v)]))
        .collect();
    Ok(Json::obj([
        ("items", Json::Arr(items)),
        ("existen", Json::Arr(existen)),
        (
            "testigo",
            Json::s(format!(
                "sha256:{}",
                ore_objetos::hex(&ore_objetos::sha256(firma.as_bytes()))
            )),
        ),
        ("huellas", Json::Int(pedidas as i64)),
    ])
    .jcs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_objetos::Version;
    use ore_objetos::memoria::EnMemoria;

    fn v(clave: &str, version: &str, actual: bool, marca: bool, etag: &str) -> Version {
        Version {
            clave: clave.into(),
            version: version.into(),
            actual,
            marca,
            tamano: if marca { 0 } else { 100 },
            etag: format!("\"{etag}\""),
            modificado: "2026-09-29T11:00:00.000Z".into(),
        }
    }

    /// **El experimento de E7, en memoria**: `a.pdf` sobrescrito, `b.pdf`
    /// borrado, `c.jpg` renombrado a `c2.jpg`, `d.pdf` repetido. Con un
    /// `.txt` que el patrón deja fuera.
    fn despues() -> EnMemoria {
        let mut o = EnMemoria {
            historia: vec![
                v("r/a.pdf", "a2", true, false, "e2"),
                v("r/a.pdf", "a1", false, false, "e1"),
                v("r/b.pdf", "bm", true, true, ""),
                v("r/b.pdf", "b1", false, false, "e2"),
                v("r/c2.jpg", "c2", true, false, "e3"),
                v("r/c.jpg", "cm", true, true, ""),
                v("r/c.jpg", "c1", false, false, "e3"),
                v("r/d.pdf", "d1", true, false, "e2"),
                v("r/nota.txt", "t1", true, false, "e4"),
            ],
            ..Default::default()
        };
        for (c, ver, cont) in [
            ("r/a.pdf", "a2", "RECIBO"),
            ("r/a.pdf", "a1", "FACTURA"),
            ("r/b.pdf", "b1", "RECIBO"),
            ("r/c2.jpg", "c2", "FOTO"),
            ("r/c.jpg", "c1", "FOTO"),
            ("r/d.pdf", "d1", "RECIBO"),
        ] {
            o.por_version
                .insert((c.into(), ver.into()), cont.as_bytes().to_vec());
        }
        o
    }

    fn pedir(
        o: &EnMemoria,
        patrones: &[&str],
        conocidos: &[(&str, &str, &str)],
    ) -> serde_json::Value {
        let p = serde_json::json!({
            "url": "s3://b", "objeto": "r/", "patrones": patrones,
            "conocidos": conocidos.iter().map(|(c, v, h)| serde_json::json!([c, v, h])).collect::<Vec<_>>(),
        });
        serde_json::from_str(&versiones(o, &p.to_string()).expect("lista")).unwrap()
    }

    /// Lo vigente con su versión y su huella; lo borrado no está; lo que el
    /// patrón no nombra, tampoco. Y el mismo contenido en tres claves da la
    /// misma huella.
    #[test]
    fn lo_vigente_con_su_version_y_su_huella() {
        let o = despues();
        let r = pedir(&o, &["*.pdf"], &[]);
        let items = r["items"].as_array().unwrap();
        let claves: Vec<&str> = items.iter().map(|i| i["clave"].as_str().unwrap()).collect();
        assert_eq!(
            claves,
            ["r/a.pdf", "r/d.pdf"],
            "b.pdf está borrado; c2 no es pdf"
        );
        assert_eq!(items[0]["version"], "a2");
        assert_eq!(items[0]["camino"], "a.pdf");
        assert_eq!(
            items[0]["huella"], items[1]["huella"],
            "a y d son un contenido"
        );
        assert_eq!(r["huellas"], 2);
    }

    /// **Un segundo pase sin cambios no pide ninguna huella**: lo conocido
    /// trae la suya. Y de lo conocido que ya no es vigente se dice si sigue
    /// existiendo como versión (`b1`, tapada por una marca) o no (`zz`).
    #[test]
    fn lo_conocido_no_se_vuelve_a_mirar_y_se_dice_si_sigue() {
        let o = despues();
        let r = pedir(
            &o,
            &["*.pdf"],
            &[
                ("r/a.pdf", "a2", "crc64nvme:x"),
                ("r/d.pdf", "d1", "crc64nvme:y"),
                ("r/b.pdf", "b1", "crc64nvme:z"),
                ("r/borrada.pdf", "zz", "crc64nvme:w"),
            ],
        );
        assert_eq!(r["huellas"], 0);
        assert_eq!(o.huellas.get(), 0);
        assert_eq!(r["items"][0]["huella"], "crc64nvme:x");
        let existen: Vec<String> = r["existen"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| format!("{}@{}", x[0].as_str().unwrap(), x[1].as_str().unwrap()))
            .collect();
        assert!(existen.contains(&"r/b.pdf@b1".to_string()), "{existen:?}");
        assert!(!existen.contains(&"r/borrada.pdf@zz".to_string()));
    }

    /// El testigo es la huella del listado: igual si nada cambió, distinto si
    /// una versión cambia aunque la clave sea la misma.
    #[test]
    fn el_testigo_cambia_con_la_version() {
        let o = despues();
        let a = pedir(&o, &["*.pdf"], &[]);
        assert_eq!(a["testigo"], pedir(&o, &["*.pdf"], &[])["testigo"]);
        let mut o2 = despues();
        o2.historia[0].version = "a3".into();
        o2.por_version
            .insert(("r/a.pdf".into(), "a3".into()), b"OTRO".to_vec());
        assert_ne!(a["testigo"], pedir(&o2, &["*.pdf"], &[])["testigo"]);
    }
}
