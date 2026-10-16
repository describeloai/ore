//! ORE 0067 G0·3, caso V3 · **Escribir N geometrías al lago**, por el camino de
//! la copia (texto del driver → `carga::lote` → el escritor geo de G2·1 →
//! tabla Iceberg v3), en un almacén en memoria: lo que mide es el formato, no
//! la red. Deja el fichero de datos en `<salida>/ore-geo.parquet` para la
//! matriz de adopción (V5).
//!
//!     cargo run --release -p ore-store --example vara-v3-escribir -- 1000000 /salida
use ore_store::almacen::{Almacen, Listado};
use ore_store::ciclo::sellar;
use ore_store::lago::Lago;
use ore_store::sobre::{Cabecera, Testigo};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Default)]
struct Memoria(Mutex<BTreeMap<String, Vec<u8>>>);

impl Almacen for Memoria {
    fn base(&self) -> String {
        "memory://vara".into()
    }
    fn leer(&self, c: &str) -> Result<Option<String>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(c)
            .map(|b| String::from_utf8_lossy(b).into_owned()))
    }
    fn existe(&self, c: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().contains_key(c))
    }
    fn subir(&self, c: &str, b: &[u8]) -> Result<bool, String> {
        let mut m = self.0.lock().unwrap();
        if m.contains_key(c) {
            return Ok(false);
        }
        m.insert(c.into(), b.to_vec());
        Ok(true)
    }
    fn listar(&self, p: &str) -> Result<Vec<String>, String> {
        Ok(self
            .0
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
        self.0.lock().unwrap().remove(c);
        Ok(())
    }
    fn leer_bytes(&self, c: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.0.lock().unwrap().get(c).cloned())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let salida = args.get(2).cloned().unwrap_or_else(|| ".".into());
    // Puntos en la península, como EWKB en hexadecimal (lo que entrega PostGIS).
    let mut s: u64 = 42;
    let mut azar = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let t0 = Instant::now();
    let filas: Vec<String> = (0..n)
        .map(|i| {
            let mut w = vec![1u8, 1, 0, 0, 0x20, 0xE6, 0x10, 0, 0];
            w.extend_from_slice(&(-9.3 + azar() * 12.6).to_le_bytes());
            w.extend_from_slice(&(36.0 + azar() * 7.8).to_le_bytes());
            format!(
                "{{\"id\":\"{i}\",\"geom\":\"{}\"}}",
                ore_core::geo::a_hex(&w)
            )
        })
        .collect();
    let preparar = t0.elapsed().as_secs_f64();
    let almacen = Arc::new(Memoria::default());
    let lago = Lago::nuevo(almacen.clone());
    let cab = Cabecera {
        plan: "sha256:vara-v3".into(),
        esquema: [
            ("id".to_string(), "Integer".to_string()),
            ("geom".to_string(), "Geometry<Point>".to_string()),
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
    let t1 = Instant::now();
    let r = sellar(
        &lago,
        &cab,
        "copias/vara_v3",
        None,
        false,
        filas.iter().map(String::as_str),
    )
    .expect("sella");
    let sellado = t1.elapsed().as_secs_f64();
    let datos: Vec<(String, Vec<u8>)> = almacen
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(k, _)| k.contains("/data/") && k.ends_with(".parquet"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let bytes: usize = datos.iter().map(|(_, v)| v.len()).sum();
    if let Some((_, v)) = datos.first() {
        std::fs::write(format!("{salida}/ore-geo.parquet"), v).expect("escribe el fichero");
    }
    let sin_estrechar = r.contains(r#""sin_estrechar":{}"#);
    // El reparto: tipar (hex → WKB validado → Arrow) y escribir (Parquet geo + Iceberg), por separado.
    let filas_mapa: Vec<ore_store::carga::Fila> = filas
        .iter()
        .map(|f| {
            let v: serde_json::Value = serde_json::from_str(f).unwrap();
            v.as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                .collect()
        })
        .collect();
    let t2 = Instant::now();
    let lote = ore_store::carga::lote(&cab.esquema, &filas_mapa)
        .expect("tipa")
        .lote;
    let tipar = t2.elapsed().as_secs_f64();
    let lago2 = Lago::nuevo(Arc::new(Memoria::default()));
    let esquema = ore_store::lago::esquema_deseado(
        &lote
            .schema()
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.data_type().clone()))
            .collect::<Vec<_>>(),
        &Default::default(),
        None,
        false,
    )
    .expect("esquema");
    let t = lago2
        .crear("copias/vara_v3b", esquema.clone(), Default::default())
        .expect("crea");
    let t3 = Instant::now();
    lago2
        .instantanea(
            &t,
            esquema,
            vec![lote],
            ore_store::lago::Operacion::Anexar,
            Default::default(),
        )
        .expect("escribe");
    let escribir = t3.elapsed().as_secs_f64();
    println!(
        "V3|local|reparto|geom|tipar {tipar:.2} s ({:.0} filas/s) · escribir {escribir:.2} s ({:.0} filas/s) · el resto de sellar, el texto de la copia",
        n as f64 / tipar,
        n as f64 / escribir
    );
    println!(
        "V3|local|copias/vara_v3|geom|{n} puntos · sellar {sellado:.2} s ({:.0} filas/s) · {} ficheros, {:.1} MB · \
         generar el texto {preparar:.2} s · sin_estrechar vacío: {sin_estrechar}",
        n as f64 / sellado,
        datos.len(),
        bytes as f64 / 1e6
    );
}
