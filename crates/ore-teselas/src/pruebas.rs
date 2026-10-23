//! Las pruebas del motor, sin red: un dataset geo escrito con el `escribir` del SDK en un
//! almacén en memoria (con su cobertura y su orden, como en el lago), y sus teselas.

use super::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

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

/// Un cuadrado WKB (lon/lat) de lado `l` grados con su esquina en (x, y).
fn cuadrado(x: f64, y: f64, l: f64) -> Vec<u8> {
    let mut w = vec![1u8, 3, 0, 0, 0, 1, 0, 0, 0, 5, 0, 0, 0];
    for (a, b) in [(x, y), (x + l, y), (x + l, y + l), (x, y + l), (x, y)] {
        w.extend_from_slice(&a.to_le_bytes());
        w.extend_from_slice(&b.to_le_bytes());
    }
    w
}

/// Madrid: una rejilla de `n`×`n` edificios de ~20 m, más uno lejos (Cádiz) que no debe salir.
fn dataset(n: usize) -> (Arc<dyn Almacen>, String) {
    use arrow_schema::{DataType, Field, Schema};
    let cuenta: Arc<dyn Almacen> = Arc::new(Memoria::default());
    let lago = Lago::nuevo(cuenta.clone());
    let mut ids = vec![];
    let mut geoms: Vec<Vec<u8>> = vec![];
    for i in 0..n {
        for j in 0..n {
            ids.push((i * n + j) as i64);
            geoms.push(cuadrado(
                -3.71 + i as f64 * 0.0005,
                40.41 + j as f64 * 0.0005,
                0.0002,
            ));
        }
    }
    ids.push(999_999);
    geoms.push(cuadrado(-6.29, 36.53, 0.0002));
    let meta = std::collections::HashMap::from([
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
    let r = ore_store::ciclo::escribir(
        &lago,
        r#"{"dataset":"datasets/geo_edificios"}"#,
        ipc.as_slice(),
    )
    .expect("escribe");
    // `escribir` deja el cambio preparado; el commit es `aplicar` (lo que hace ore-serve)
    let v: serde_json::Value = serde_json::from_str(&r).unwrap();
    let pet = serde_json::json!({"dataset": "datasets/geo_edificios", "requirements": v["requirements"], "updates": v["updates"]});
    let r = ore_store::ciclo::aplicar(&lago, &pet.to_string()).expect("aplica");
    let n = ore_core::parse::parse(&r).unwrap();
    let ml = n
        .get("metadata_location")
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("{r}"))
        .to_string();
    (cuenta, ml)
}

/// La petición de C3 para el dataset de la prueba (su `metadata_location` es `memory://`,
/// que la forma de C3 no admite: se pone después).
fn pet(ml: &str, z: u32, x: u32, y: u32) -> Peticion {
    let mut p = peticion(&pide("s3://pruebas/t/metadata/0.json", z, x, y)).unwrap();
    p.metadata_location = ml.to_string();
    p
}

fn pide(ml: &str, z: u32, x: u32, y: u32) -> String {
    format!(
        r#"{{"protocolo":"1","tms":"WebMercatorQuad","z":"{z}","x":"{x}","y":"{y}","formato":"mvt","capa":"edificios",
            "fuente":{{"metadata_location":"{ml}","geometria":"geom","crs":"EPSG:4326","id":"id"}},
            "limites":{{"plazo_ms":"10000","max_bytes":"2000000"}},
            "credencial":{{"s3.endpoint":"http://nadie","s3.access-key-id":"k","s3.secret-access-key":"s"}}}}"#
    )
}

fn tesela_de(lon: f64, lat: f64, z: u32) -> (u32, u32) {
    let n = 2f64.powi(z as i32);
    (
        ((lon + 180.0) / 360.0 * n) as u32,
        ((1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n) as u32,
    )
}

#[test]
fn una_tesela_de_madrid_trae_sus_edificios_y_sale_identica() {
    let (cuenta, ml) = dataset(10);
    let (x, y) = tesela_de(-3.708, 40.412, 15);
    let p = pet(&ml, 15, x, y);
    let m = Motor::nuevo();
    let (b1, c) = m.tesela_con(&p, cuenta.clone()).unwrap();
    let b1 = b1.expect("no vacía");
    assert!(c.elementos > 0 && c.elementos <= 100, "{c:?}");
    assert_eq!(c.grupos_leidos, c.grupos, "un grupo pequeño: se lee");
    // la misma petición, otro motor (sin cachés): los mismos bytes (D-F2-6)
    let (b2, _) = Motor::nuevo().tesela_con(&p, cuenta).unwrap();
    assert_eq!(Some(b1.clone()), b2);
    // la capa se llama como pide el descriptor
    assert!(b1.windows(9).any(|w| w == b"edificios"));
}

#[test]
fn lejos_de_los_datos_no_se_lee_nada_y_la_tesela_esta_vacia() {
    let (cuenta, ml) = dataset(10);
    // el Atlántico: la cobertura dice que ningún grupo toca
    let (x, y) = tesela_de(-30.0, 40.0, 12);
    let (b, c) = Motor::nuevo()
        .tesela_con(&pet(&ml, 12, x, y), cuenta)
        .unwrap();
    assert_eq!(b, None);
    assert_eq!(c.grupos_leidos, 0, "{c:?}");
}

#[test]
fn lo_que_no_cabe_es_un_503_que_lo_dice() {
    let (cuenta, ml) = dataset(30);
    let (x, y) = tesela_de(-3.70, 40.415, 14);
    let mut p = pet(&ml, 14, x, y);
    p.max_bytes = 100;
    let e = Motor::nuevo().tesela_con(&p, cuenta).unwrap_err();
    assert_eq!(e.codigo(), 503, "{e:?}");
    assert!(e.motivo().contains("agregado"));
}

#[test]
fn la_peticion_mal_formada_es_un_422_que_dice_que_falta() {
    let casos = [
        ("{}", "protocolo"),
        (r#"{"protocolo":"7"}"#, "protocolo 7"),
        (r#"{"protocolo":"1","tms":"WorldCRS84Quad"}"#, "tms"),
        (
            r#"{"protocolo":"1","tms":"WebMercatorQuad","formato":"mvt","z":"3","x":"9","y":"0"}"#,
            "no existe",
        ),
    ];
    for (c, dice) in casos {
        let e = peticion(c).unwrap_err();
        assert_eq!(e.codigo(), 422);
        assert!(e.motivo().contains(dice), "{c} → {}", e.motivo());
    }
    let otra = pide("s3://b/t/metadata/x.json", 1, 0, 0).replace("EPSG:4326", "EPSG:25830");
    assert!(peticion(&otra).unwrap_err().motivo().contains("25830"));
}

#[test]
fn la_cadena_orienta_los_anillos_como_la_spec() {
    // exterior en sentido antihorario en pantalla: se le da la vuelta
    let l = vec![
        (0.0, 0.0),
        (0.0, 10.0),
        (10.0, 10.0),
        (10.0, 0.0),
        (0.0, 0.0),
    ];
    let ext = a_rejilla(&l, true).unwrap();
    assert!(area(&ext) > 0.0);
    let int = a_rejilla(&l, false).unwrap();
    assert!(area(&int) < 0.0);
    // lo que queda por debajo de 4 u² no sale
    let d = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)];
    assert_eq!(a_rejilla(&d, true), None);
}

#[test]
fn recortar_deja_el_anillo_dentro_del_marco_y_rdp_quita_lo_que_no_se_ve() {
    // un cuadrado que se sale por la derecha: queda dentro del margen
    let r = vec![
        (4000.0, 100.0),
        (5000.0, 100.0),
        (5000.0, 200.0),
        (4000.0, 200.0),
        (4000.0, 100.0),
    ];
    let c = recortar(&r);
    assert!(c.iter().all(|q| q.0 <= EXTENT + BUFFER + 1e-9), "{c:?}");
    assert_eq!(c.first(), c.last());
    // fuera del todo: no queda nada
    assert!(recortar(&[(9000.0, 0.0), (9100.0, 0.0), (9100.0, 50.0), (9000.0, 0.0)]).len() <= 1);
    // un vértice a medio píxel de la recta se va; uno a 5 u se queda
    let z = vec![
        (0.0, 0.0),
        (5.0, 0.4),
        (10.0, 0.0),
        (10.0, 10.0),
        (5.0, 15.0),
        (0.0, 10.0),
        (0.0, 0.0),
    ];
    let s = rdp(&z, 1.0);
    assert!(
        !s.contains(&(5.0, 0.4)) && s.contains(&(5.0, 15.0)),
        "{s:?}"
    );
}
