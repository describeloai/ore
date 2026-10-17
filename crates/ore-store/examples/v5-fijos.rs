//! ORE 0067 G2·5 · **Los fijos de la matriz de adopción, sellados por el
//! escritor de ORE**, en disco, para iterar los lectores en local (el veredicto
//! es en victor, sobre lo que escribe el ORE desplegado). Lee
//! `pruebas-de-fuego/geo/v5/fijos.json` (todos los subtipos, en EPSG:25830 y en
//! OGC:CRS84) y deja, por CRS, `<salida>/<crs>/tabla/…` (la tabla Iceberg v3,
//! artefacto B, con rutas `file://<raiz>/<crs>/tabla/…`) y
//! `<salida>/<crs>/ore-geo.parquet` (su fichero de datos, artefacto A).
//!
//!     cargo run -p ore-store --example v5-fijos -- fijos.json /salida file:///v5
use ore_store::almacen::{Almacen, Listado};
use ore_store::ciclo::sellar;
use ore_store::lago::Lago;
use ore_store::sobre::{Cabecera, Testigo};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

struct Memoria(String, Mutex<BTreeMap<String, Vec<u8>>>);

impl Almacen for Memoria {
    fn base(&self) -> String {
        self.0.clone()
    }
    fn leer(&self, c: &str) -> Result<Option<String>, String> {
        Ok(self
            .1
            .lock()
            .unwrap()
            .get(c)
            .map(|b| String::from_utf8_lossy(b).into_owned()))
    }
    fn existe(&self, c: &str) -> Result<bool, String> {
        Ok(self.1.lock().unwrap().contains_key(c))
    }
    fn subir(&self, c: &str, b: &[u8]) -> Result<bool, String> {
        let mut m = self.1.lock().unwrap();
        if m.contains_key(c) {
            return Ok(false);
        }
        m.insert(c.into(), b.to_vec());
        Ok(true)
    }
    fn listar(&self, p: &str) -> Result<Vec<String>, String> {
        Ok(self
            .1
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(p))
            .cloned()
            .collect())
    }
    fn listar_con_fecha(&self, p: &str) -> Result<Vec<Listado>, String> {
        Ok(self
            .listar(p)?
            .into_iter()
            .map(|clave| Listado {
                clave,
                tamano: 0,
                tocado_ms: 0,
            })
            .collect())
    }
    fn borrar(&self, c: &str) -> Result<(), String> {
        self.1.lock().unwrap().remove(c);
        Ok(())
    }
    fn leer_bytes(&self, c: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.1.lock().unwrap().get(c).cloned())
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let fijos: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&a[1]).expect("lee los fijos"))
            .expect("JSON");
    let salida = &a[2];
    let raiz = a
        .get(3)
        .cloned()
        .unwrap_or_else(|| format!("file://{salida}"));
    for (crs, f) in fijos.as_object().expect("un objeto por CRS") {
        let carpeta = crs.replace(':', "-");
        let base = format!("{raiz}/{carpeta}/tabla");
        let almacen = Arc::new(Memoria(base.clone(), Mutex::default()));
        let lago = Lago::nuevo(almacen.clone());
        let tipo = if crs == "OGC:CRS84" {
            "Geometry".to_string()
        } else {
            format!("Geometry<Any, {crs}>")
        };
        let cab = Cabecera {
            plan: "sha256:v5-fijos".into(),
            esquema: [
                ("id".to_string(), "Integer".to_string()),
                ("subtipo".to_string(), "String".to_string()),
                ("geom".to_string(), tipo),
            ]
            .into(),
            testigo: Testigo {
                modo: "log".into(),
                valor: Some("1".into()),
            },
            clave: vec![],
            conducto: "materialization.payload".into(),
            obligatorias: Default::default(),
        };
        let filas: Vec<String> = f["filas"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let mut o = serde_json::Map::new();
                o.insert("id".into(), r["id"].to_string().into());
                o.insert("subtipo".into(), r["subtipo"].clone());
                if let Some(w) = r["wkb"].as_str() {
                    o.insert("geom".into(), w.into());
                }
                serde_json::Value::Object(o).to_string()
            })
            .collect();
        let r = sellar(
            &lago,
            &cab,
            "v5",
            None,
            false,
            filas.iter().map(String::as_str),
        )
        .unwrap_or_else(|e| panic!("{crs}: {e}"));
        let m = almacen.1.lock().unwrap();
        for (k, v) in m.iter() {
            let rel = k.trim_start_matches(&format!("{base}/"));
            let ruta = format!("{salida}/{carpeta}/tabla/{rel}");
            std::fs::create_dir_all(std::path::Path::new(&ruta).parent().unwrap()).unwrap();
            std::fs::write(&ruta, v).unwrap();
            if k.contains("/data/") && k.ends_with(".parquet") {
                std::fs::write(format!("{salida}/{carpeta}/ore-geo.parquet"), v).unwrap();
            }
        }
        let ml = m
            .keys()
            .filter(|k| k.ends_with(".metadata.json"))
            .max()
            .cloned()
            .unwrap_or_default();
        println!("V5F|{crs}|{} filas|{ml}|{}", filas.len(), r.len());
    }
}
