//! Las pruebas de `ore-piramide`, sin red: la geometría de la rejilla, y una construcción entera sobre un
//! dataset escrito como en el lago (en memoria) con un tippecanoe falso que guarda lo que le llega.

use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Memoria(Mutex<BTreeMap<String, Vec<u8>>>);

impl Almacen for Memoria {
    fn base(&self) -> String {
        "memory://pruebas".into()
    }
    fn leer(&self, clave: &str) -> Result<Option<String>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(clave)
            .map(|b| String::from_utf8_lossy(b).into_owned()))
    }
    fn existe(&self, clave: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().contains_key(clave))
    }
    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
        let mut m = self.0.lock().unwrap();
        if m.contains_key(clave) {
            return Ok(false);
        }
        m.insert(clave.to_string(), cuerpo.to_vec());
        Ok(true)
    }
    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefijo))
            .cloned()
            .collect())
    }
    fn borrar(&self, clave: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(clave);
        Ok(())
    }
    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.0.lock().unwrap().get(clave).cloned())
    }
}

fn wkb_poligono(anillo: &[(f64, f64)]) -> Vec<u8> {
    let mut w = vec![1u8, 3, 0, 0, 0, 1, 0, 0, 0];
    w.extend_from_slice(&(anillo.len() as u32).to_le_bytes());
    for (a, b) in anillo {
        w.extend_from_slice(&a.to_le_bytes());
        w.extend_from_slice(&b.to_le_bytes());
    }
    w
}

fn wkb_linea(l: &[(f64, f64)]) -> Vec<u8> {
    let mut w = vec![1u8, 2, 0, 0, 0];
    w.extend_from_slice(&(l.len() as u32).to_le_bytes());
    for (a, b) in l {
        w.extend_from_slice(&a.to_le_bytes());
        w.extend_from_slice(&b.to_le_bytes());
    }
    w
}

fn wkb_punto(x: f64, y: f64) -> Vec<u8> {
    let mut w = vec![1u8, 1, 0, 0, 0];
    w.extend_from_slice(&x.to_le_bytes());
    w.extend_from_slice(&y.to_le_bytes());
    w
}

fn cuadrado(x: f64, y: f64, l: f64) -> Vec<(f64, f64)> {
    vec![(x, y), (x + l, y), (x + l, y + l), (x, y + l), (x, y)]
}

/// Un dataset geo (`id`, `geom` WKB en EPSG:4326), escrito como en el lago.
fn dataset_de(filas: Vec<(i64, Vec<u8>)>) -> (Arc<dyn Almacen>, String) {
    use arrow_array::{BinaryArray, Int64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    let cuenta: Arc<dyn Almacen> = Arc::new(Memoria::default());
    let lago = Lago::nuevo(cuenta.clone());
    let (ids, geoms): (Vec<i64>, Vec<Vec<u8>>) = filas.into_iter().unzip();
    let meta = HashMap::from([
        (
            "ARROW:extension:name".to_string(),
            "geoarrow.wkb".to_string(),
        ),
        (
            "ARROW:extension:metadata".to_string(),
            "{\"crs\": \"EPSG:4326\"}".to_string(),
        ),
    ]);
    let esquema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("geom", DataType::Binary, true).with_metadata(meta),
    ]));
    let lote = RecordBatch::try_new(
        esquema.clone(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(BinaryArray::from_iter_values(
                geoms.iter().map(|g| g.as_slice()),
            )),
        ],
    )
    .unwrap();
    let mut ipc = Vec::new();
    {
        let mut w = arrow_ipc::writer::StreamWriter::try_new(&mut ipc, &esquema).unwrap();
        w.write(&lote).unwrap();
        w.finish().unwrap();
    }
    let r = ore_store::ciclo::escribir(&lago, r#"{"dataset":"datasets/geo"}"#, ipc.as_slice())
        .expect("escribe");
    let v: serde_json::Value = serde_json::from_str(&r).unwrap();
    let pet = serde_json::json!({"dataset": "datasets/geo", "requirements": v["requirements"], "updates": v["updates"]});
    let r = ore_store::ciclo::aplicar(&lago, &pet.to_string()).expect("aplica");
    let v: serde_json::Value = serde_json::from_str(&r).unwrap();
    (cuenta, v["metadata_location"].as_str().unwrap().to_string())
}

/// Un tippecanoe falso: guarda su entrada en `<salida>.entrada` y deja un PMTiles de mentira.
fn tippecanoe_falso(dir: &Path) -> String {
    let s = dir.join("tippecanoe");
    std::fs::write(
        &s,
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do [ \"$1\" = -o ] && OUT=$2; shift; done\n\
         cat > \"$OUT.entrada\"\n\
         printf PMTiles > \"$OUT\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    s.to_string_lossy().into_owned()
}

fn temporal(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ore-piramide-{nombre}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn pet(ml: &str, dir: &Path, tipo: Option<Tipo>, suelo: u32) -> Peticion {
    let mut p = peticion(&format!(
        r#"{{"fuente":{{"metadata_location":"gs://lago/x/metadata/0.json","geometria":"geom","id":"id"}},
            "capa":"c","suelo":"{suelo}","destino":{{"ubicacion":"gs://ore/piramides/x.pmtiles"}},
            "trabajo":"{}"}}"#,
        dir.display()
    ))
    .unwrap();
    p.metadata_location = ml.to_string();
    p.tipo = tipo;
    p.tippecanoe = tippecanoe_falso(dir);
    p
}

#[test]
fn la_celda_y_su_caja_se_corresponden() {
    for (lon, lat) in [
        (-3.7038, 40.4168),
        (-73.98, 40.75),
        (139.69, 35.68),
        (0.0, 0.0),
    ] {
        for l in [0, 5, 12, 18] {
            let (x, y) = celda(lon, lat, l);
            let (x0, y0, x1, y1) = caja(x, y, l);
            // (el borde de abajo de una celda es el de arriba de la siguiente: 0° cae en los dos)
            assert!(
                x0 <= lon && lon < x1 && y0 <= lat && lat <= y1,
                "{lon},{lat} en l{l}"
            );
        }
    }
}

#[test]
fn el_area_y_el_centro_de_un_edificio() {
    // un cuadrado de 0,0002° en Madrid: 0,0002·111 320·cos(40,41°) × 0,0002·110 540 ≈ 377 m²
    let (a, c) = area_y_centro(&[cuadrado(-3.71, 40.41, 0.0002)]).unwrap();
    assert!((a - 377.0).abs() < 4.0, "{a}");
    assert!(
        (c.0 + 3.7099).abs() < 1e-9 && (c.1 - 40.4101).abs() < 1e-9,
        "{c:?}"
    );
    // con un agujero de la cuarta parte, tres cuartos
    let (a2, _) = area_y_centro(&[
        cuadrado(-3.71, 40.41, 0.0002),
        cuadrado(-3.70995, 40.41005, 0.0001),
    ])
    .unwrap();
    assert!((a2 / a - 0.75).abs() < 1e-6, "{a2} {a}");
}

#[test]
fn todos_los_zooms_cuentan_lo_mismo() {
    let mut r = Rejilla::nueva(12, 1_000_000);
    for i in 0..400 {
        let (x, y) = (-3.9 + (i % 20) as f64 * 0.02, 40.2 + (i / 20) as f64 * 0.02);
        r.agregar(&Geometria::Poligonos(vec![vec![cuadrado(x, y, 0.0002)]]))
            .unwrap();
    }
    let total: (u64, f64) = r
        .celdas
        .values()
        .fold((0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
    assert_eq!(total.0, 400);
    for (z, m) in r.por_zoom() {
        let t: (u64, f64) = m.values().fold((0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
        assert_eq!(t.0, 400, "z{z}");
        assert!((t.1 - total.1).abs() < 1e-6 * total.1, "z{z}");
    }
}

#[test]
fn una_linea_reparte_sus_metros_y_cuenta_una_vez_por_celda() {
    let mut r = Rejilla::nueva(12, 1_000_000);
    // 0,05° hacia el este a 40,75° de latitud (Manhattan): ~4 216 m
    let l = vec![(-74.0, 40.75), (-73.95, 40.75)];
    r.agregar(&Geometria::Lineas(vec![l])).unwrap();
    let m: f64 = r.celdas.values().map(|v| v.1).sum();
    let esperado = 0.05 * R_X * 40.75f64.to_radians().cos();
    assert!((m - esperado).abs() < 1e-6 * esperado, "{m} {esperado}");
    assert!(r.celdas.values().all(|v| v.0 == 1), "una vez por celda");
    // a lo largo: cruza de 0,05° / (360° / 2^17) ≈ 18 celdas
    assert!((17..=20).contains(&r.celdas.len()), "{}", r.celdas.len());
}

#[test]
fn el_tope_de_celdas_para_la_rejilla() {
    let mut r = Rejilla::nueva(16, 10);
    let e = (0..100)
        .map(|i| r.agregar(&Geometria::Puntos(vec![(-3.7 + i as f64 * 0.01, 40.4)])))
        .find(|x| x.is_err())
        .unwrap()
        .unwrap_err();
    assert!(e.contains("10 celdas"), "{e}");
}

#[test]
fn una_celda_llena_cubre_uno_y_sale_solo_en_su_zoom() {
    let l = 3 + CELDAS;
    let (x, y) = celda(-3.7, 40.4, l);
    let (x0, y0, x1, y1) = caja(x, y, l);
    let lado = 40_075_016.7 * ((y0 + y1) / 2.0).to_radians().cos() / (1u64 << l) as f64;
    let g = celda_geojson(3, (x, y), (5, lado * lado * 2.0), false);
    let v: serde_json::Value = serde_json::from_str(&g).unwrap();
    assert_eq!(v["tippecanoe"]["minzoom"], 3);
    assert_eq!(v["tippecanoe"]["maxzoom"], 3);
    assert_eq!(
        v["properties"]["cobertura"].as_f64(),
        Some(1.0),
        "con tope 1"
    );
    assert_eq!(
        v["geometry"]["coordinates"][0][0][0].as_f64().unwrap(),
        (x0 * 1e7).round() / 1e7
    );
    let _ = (x1, y1);
}

#[test]
fn los_puntos_van_a_tippecanoe_tal_cual_con_su_id() {
    let filas: Vec<(i64, Vec<u8>)> = (0..50)
        .map(|i| (i, wkb_punto(-73.98 + i as f64 * 0.001, 40.75)))
        .collect();
    let (cuenta, ml) = dataset_de(filas);
    let dir = temporal("puntos");
    let p = pet(&ml, &dir, None, 13);
    let salida = dir.join("p.pmtiles");
    let mut dicho = vec![];
    let h = construir(&cuenta, &p, &salida, &mut |q, j| {
        dicho.push((q.to_string(), j.jcs()))
    })
    .unwrap();
    assert_eq!(h.tipo, Tipo::Puntos, "lo decide la primera geometría");
    assert_eq!((h.leido.filas, h.leido.puntos), (50, 50));
    assert!(
        h.orden.contains("-z12")
            && h.orden.contains("-B16")
            && h.orden.contains("--drop-densest-as-needed"),
        "{}",
        h.orden
    );
    let entrada = std::fs::read_to_string(dir.join("p.pmtiles.entrada")).unwrap();
    let lineas: Vec<serde_json::Value> = entrada
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lineas.len(), 50);
    assert_eq!(lineas[7]["properties"]["id"], 7);
    assert_eq!(
        lineas[7]["geometry"]["coordinates"][0].as_f64(),
        Some(-73.973)
    );
    assert!(dicho.iter().any(|(q, _)| q == "leido"));
}

#[test]
fn los_edificios_se_agregan_y_cada_zoom_trae_todos() {
    let filas: Vec<(i64, Vec<u8>)> = (0..300)
        .map(|i| {
            let (x, y) = (
                -3.75 + (i % 30) as f64 * 0.003,
                40.38 + (i / 30) as f64 * 0.003,
            );
            (i, wkb_poligono(&cuadrado(x, y, 0.0002)))
        })
        .collect();
    let (cuenta, ml) = dataset_de(filas);
    let dir = temporal("edificios");
    let p = pet(&ml, &dir, None, 13);
    let salida = dir.join("e.pmtiles");
    let h = construir(&cuenta, &p, &salida, &mut |_, _| {}).unwrap();
    assert_eq!(h.tipo, Tipo::Agregado);
    assert_eq!((h.leido.filas, h.leido.poligonos), (300, 300));
    assert!(h.orden.contains("--no-tile-size-limit"), "{}", h.orden);
    let entrada = std::fs::read_to_string(dir.join("e.pmtiles.entrada")).unwrap();
    let mut por_zoom: BTreeMap<i64, i64> = BTreeMap::new();
    for l in entrada.lines() {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        *por_zoom
            .entry(v["tippecanoe"]["minzoom"].as_i64().unwrap())
            .or_default() += v["properties"]["n"].as_i64().unwrap();
    }
    assert_eq!(por_zoom.len(), 13, "z0..=z12");
    assert!(por_zoom.values().all(|n| *n == 300), "{por_zoom:?}");
}

#[test]
fn las_lineas_se_agregan_por_km_de_linea() {
    let filas: Vec<(i64, Vec<u8>)> = (0..20)
        .map(|i| {
            (
                i,
                wkb_linea(&[(-74.0, 40.70 + i as f64 * 0.001), (-73.9, 40.75)]),
            )
        })
        .collect();
    let (cuenta, ml) = dataset_de(filas);
    let dir = temporal("lineas");
    let p = pet(&ml, &dir, None, 14);
    let h = construir(&cuenta, &p, &dir.join("l.pmtiles"), &mut |_, _| {}).unwrap();
    assert_eq!((h.tipo, h.leido.lineas), (Tipo::Agregado, 20));
    let entrada = std::fs::read_to_string(dir.join("l.pmtiles.entrada")).unwrap();
    let z0: Vec<serde_json::Value> = entrada
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["tippecanoe"]["minzoom"] == 0)
        .collect();
    assert_eq!(z0.len(), 1, "en z0, una celda");
    // 20 líneas de ~9,6 km: ~192 km en una celda de z0
    let km = z0[0]["properties"]["a"].as_f64().unwrap() / 1000.0;
    assert!((185.0..200.0).contains(&km), "{km}");
}

/// Sin `suelo` en la petición (la de `ore-serve`), lo decide la primera geometría: 14 para las
/// líneas (en z13 su crudo no cabe), 13 para los puntos; el manifiesto dice el que se usó.
#[test]
fn sin_suelo_lo_decide_el_tipo_y_lo_dice_el_manifiesto() {
    let lineas: Vec<(i64, Vec<u8>)> = (0..5)
        .map(|i| {
            (
                i,
                wkb_linea(&[(-74.0, 40.70 + i as f64 * 0.001), (-73.9, 40.75)]),
            )
        })
        .collect();
    let puntos: Vec<(i64, Vec<u8>)> = (0..5).map(|i| (i, wkb_punto(-3.7, 40.4))).collect();
    for (nombre, filas, suelo, z) in [("l", lineas, 14, "-z13"), ("p", puntos, 13, "-z12")] {
        let (cuenta, ml) = dataset_de(filas);
        let dir = temporal(&format!("suelo-{nombre}"));
        let mut p = pet(&ml, &dir, None, 13);
        p.suelo = None;
        let h = construir(&cuenta, &p, &dir.join("x.pmtiles"), &mut |_, _| {}).unwrap();
        assert_eq!(h.suelo, suelo, "{nombre}");
        assert!(h.orden.contains(z), "{}", h.orden);
        let m = manifiesto(&p, &h, "tippecanoe v0", 0.0).jcs();
        assert!(
            m.contains(&format!("\"suelo\":{suelo}"))
                && m.contains(&format!("\"maxzoom\":{}", suelo - 1)),
            "{m}"
        );
    }
}

#[test]
fn la_peticion_pide_lo_que_hace_falta() {
    assert!(
        peticion("{}")
            .unwrap_err()
            .contains("fuente.metadata_location")
    );
    let e = peticion(r#"{"fuente":{"metadata_location":"gs://a/b","crs":"EPSG:25830"},"destino":{"ubicacion":"gs://o/p"}}"#)
        .unwrap_err();
    assert!(e.contains("25830"), "{e}");
    assert!(partir("s3://a/b").is_err() && partir("gs://a").is_err());
    assert_eq!(
        partir("gs://ore/piramides/x.pmtiles").unwrap(),
        ("ore", "piramides/x.pmtiles")
    );
}
