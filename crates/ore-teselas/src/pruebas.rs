//! Las pruebas del motor, sin red: un dataset geo escrito con el `escribir` del SDK en un
//! almacén en memoria (con su cobertura y su orden, como en el lago), y sus teselas.

use super::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

// Las pruebas, con el asignador del binario (lo que corre en la imagen).
#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    let mut filas = vec![];
    for i in 0..n {
        for j in 0..n {
            filas.push((
                (i * n + j) as i64,
                cuadrado(-3.71 + i as f64 * 0.0005, 40.41 + j as f64 * 0.0005, 0.0002),
            ));
        }
    }
    filas.push((999_999, cuadrado(-6.29, 36.53, 0.0002)));
    dataset_de(filas)
}

/// Un dataset geo (`id`, `geom` WKB en EPSG:4326) con estas filas, escrito como en el lago.
fn dataset_de(filas: Vec<(i64, Vec<u8>)>) -> (Arc<dyn Almacen>, String) {
    use arrow_schema::{DataType, Field, Schema};
    let cuenta: Arc<dyn Almacen> = Arc::new(Memoria::default());
    let lago = Lago::nuevo(cuenta.clone());
    let (ids, geoms): (Vec<i64>, Vec<Vec<u8>>) = filas.into_iter().unzip();
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
    let n = ore_base::parse::parse(&r).unwrap();
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

// ── F2·1·6: puntos, líneas y colecciones ────────────────────────────────────────────────────────

/// La cabecera WKB little-endian de un tipo (ISO: +1000 Z, +2000 M, +3000 ZM).
fn cabecera(tipo: u32) -> Vec<u8> {
    let mut w = vec![1u8];
    w.extend_from_slice(&tipo.to_le_bytes());
    w
}

fn mas_coords(w: &mut Vec<u8>, cs: &[f64]) {
    for c in cs {
        w.extend_from_slice(&c.to_le_bytes());
    }
}

/// Un punto WKB (2D).
fn punto(x: f64, y: f64) -> Vec<u8> {
    let mut w = cabecera(1);
    mas_coords(&mut w, &[x, y]);
    w
}

/// Una línea WKB (2D).
fn linea(l: &[(f64, f64)]) -> Vec<u8> {
    let mut w = cabecera(2);
    w.extend_from_slice(&(l.len() as u32).to_le_bytes());
    for q in l {
        mas_coords(&mut w, &[q.0, q.1]);
    }
    w
}

/// Un contenedor WKB (4 multipunto, 5 multilínea, 6 multipolígono, 7 colección) de sus partes.
fn de_partes(tipo: u32, partes: &[Vec<u8>]) -> Vec<u8> {
    let mut w = cabecera(tipo);
    w.extend_from_slice(&(partes.len() as u32).to_le_bytes());
    for p in partes {
        w.extend_from_slice(p);
    }
    w
}

/// Una curva (CIRCULARSTRING, tipo 8): no se pinta.
fn curva(x: f64, y: f64) -> Vec<u8> {
    let mut c = cabecera(8);
    c.extend_from_slice(&3u32.to_le_bytes());
    mas_coords(&mut c, &[x, y, x + 0.0001, y + 0.0001, x + 0.0002, y]);
    c
}

fn ident(x: f64, y: f64) -> (f64, f64) {
    (x, y)
}

#[test]
fn se_leen_los_siete_tipos_y_la_coleccion_se_descompone() {
    use Geometria::*;
    assert_eq!(
        geometrias(&punto(1.0, 2.0), &ident),
        Some(vec![Puntos(vec![(1.0, 2.0)])])
    );
    assert_eq!(
        geometrias(&linea(&[(0.0, 0.0), (5.0, 5.0)]), &ident),
        Some(vec![Lineas(vec![vec![(0.0, 0.0), (5.0, 5.0)]])])
    );
    // multipunto y multilínea: un elemento con todas sus partes
    let mp = de_partes(4, &[punto(1.0, 1.0), punto(2.0, 2.0)]);
    assert_eq!(
        geometrias(&mp, &ident),
        Some(vec![Puntos(vec![(1.0, 1.0), (2.0, 2.0)])])
    );
    let ml = de_partes(
        5,
        &[
            linea(&[(0.0, 0.0), (1.0, 0.0)]),
            linea(&[(2.0, 0.0), (3.0, 0.0)]),
        ],
    );
    assert!(matches!(&geometrias(&ml, &ident).unwrap()[..], [Lineas(v)] if v.len() == 2));
    // la colección: un elemento por tipo, en orden punto, línea, polígono; anidada también
    let c = de_partes(
        7,
        &[
            cuadrado(0.0, 0.0, 1.0),
            punto(9.0, 9.0),
            de_partes(7, &[linea(&[(0.0, 0.0), (4.0, 4.0)]), punto(8.0, 8.0)]),
        ],
    );
    let g = geometrias(&c, &ident).unwrap();
    assert!(
        matches!(&g[..], [Puntos(p), Lineas(_), Poligonos(_)] if p.len() == 2),
        "{g:?}"
    );
    // vacíos: no se pinta nada, pero no es un descarte
    assert_eq!(geometrias(&punto(f64::NAN, f64::NAN), &ident), Some(vec![]));
    assert_eq!(geometrias(&de_partes(7, &[]), &ident), Some(vec![]));
}

#[test]
fn z_y_m_se_descartan_y_el_ewkb_con_srid_se_lee() {
    // ISO: POINT Z (1001), POINT ZM (3001), LINESTRING M (2002)
    let mut z = cabecera(1001);
    mas_coords(&mut z, &[1.0, 2.0, 99.0]);
    let mut zm = cabecera(3001);
    mas_coords(&mut zm, &[1.0, 2.0, 99.0, 7.0]);
    let mut lm = cabecera(2002);
    lm.extend_from_slice(&2u32.to_le_bytes());
    mas_coords(&mut lm, &[0.0, 0.0, 5.0, 3.0, 3.0, 6.0]);
    for w in [&z, &zm] {
        assert_eq!(
            geometrias(w, &ident),
            Some(vec![Geometria::Puntos(vec![(1.0, 2.0)])])
        );
    }
    assert_eq!(
        geometrias(&lm, &ident),
        Some(vec![Geometria::Lineas(vec![vec![(0.0, 0.0), (3.0, 3.0)]])])
    );
    // EWKB: POINT Z con SRID (0xA0000001) y su SRID 4326
    let mut ew = cabecera(0xA000_0001);
    ew.extend_from_slice(&4326u32.to_le_bytes());
    mas_coords(&mut ew, &[1.0, 2.0, 99.0]);
    assert_eq!(
        geometrias(&ew, &ident),
        Some(vec![Geometria::Puntos(vec![(1.0, 2.0)])])
    );
}

#[test]
fn las_curvas_y_el_wkb_roto_se_descartan() {
    // una curva, y una colección que la lleva: no se pinta la fila
    assert_eq!(geometrias(&curva(0.0, 0.0), &ident), None);
    assert_eq!(
        geometrias(&de_partes(7, &[punto(1.0, 1.0), curva(0.0, 0.0)]), &ident),
        None
    );
    // truncado
    let l = linea(&[(0.0, 0.0), (5.0, 5.0)]);
    assert_eq!(geometrias(&l[..l.len() - 3], &ident), None);
    assert_eq!(geometrias(&[], &ident), None);
}

#[test]
fn la_linea_que_sale_y_vuelve_a_entrar_se_parte_en_dos_trozos() {
    let l = [
        (-500.0, 100.0),
        (2000.0, 100.0),
        (5000.0, 100.0),
        (5000.0, 3000.0),
        (2000.0, 3000.0),
    ];
    let t = recortar_linea(&l);
    assert_eq!(t.len(), 2, "{t:?}");
    assert_eq!(
        t[0],
        vec![(-BUFFER, 100.0), (2000.0, 100.0), (EXTENT + BUFFER, 100.0)]
    );
    assert_eq!(t[1], vec![(EXTENT + BUFFER, 3000.0), (2000.0, 3000.0)]);
    // fuera del todo: nada; dentro del todo: tal cual
    assert!(recortar_linea(&[(-900.0, -900.0), (-800.0, 9000.0)]).is_empty());
    let dentro = [(10.0, 10.0), (20.0, 30.0)];
    assert_eq!(recortar_linea(&dentro), vec![dentro.to_vec()]);
    // RDP abierto: conserva los extremos y quita lo que no se ve
    let casi = [(0.0, 0.0), (100.0, 0.3), (200.0, 0.0), (300.0, 50.0)];
    assert_eq!(
        rdp_abierto(&casi, 1.0),
        vec![(0.0, 0.0), (200.0, 0.0), (300.0, 50.0)]
    );
    // a la rejilla: un trozo que se aplasta en un punto no es una línea
    assert_eq!(a_rejilla_linea(&[(1.2, 1.2), (0.9, 1.1)]), None);
}

#[test]
fn los_puntos_entran_con_su_margen_y_no_se_funden_entre_filas() {
    let mut dim = 0;
    let g = Geometria::Puntos(vec![
        (-60.0, 10.0),
        (-70.0, 10.0),
        (4150.0, 4150.0),
        (5.2, 5.4),
        (5.0, 5.0),
    ]);
    let (tipo, partes, _) = a_la_tesela(g, &mut dim).unwrap();
    assert_eq!(tipo, PUNTO);
    // el de −70 cae fuera del margen; dos de un multipunto en la misma celda, uno
    assert_eq!(partes, vec![vec![(-60, 10), (4150, 4150), (5, 5)]]);
    assert!(a_la_tesela(Geometria::Puntos(vec![(-900.0, 0.0)]), &mut dim).is_none());
}

/// Lo justo de un lector de MVT para las pruebas: `(tipo, id, comandos)` de cada feature.
fn features(t: &[u8]) -> Vec<(u64, Option<u64>, Vec<u64>)> {
    fn var(b: &[u8], i: &mut usize) -> u64 {
        let (mut v, mut s) = (0u64, 0);
        loop {
            let x = b[*i];
            *i += 1;
            v |= ((x & 0x7f) as u64) << s;
            if x < 0x80 {
                return v;
            }
            s += 7;
        }
    }
    type Campo = (u64, Vec<u8>, u64);
    fn campos(b: &[u8]) -> Vec<Campo> {
        let (mut i, mut out) = (0, vec![]);
        while i < b.len() {
            let k = var(b, &mut i);
            match k & 7 {
                0 => out.push((k >> 3, vec![], var(b, &mut i))),
                2 => {
                    let n = var(b, &mut i) as usize;
                    out.push((k >> 3, b[i..i + n].to_vec(), 0));
                    i += n;
                }
                w => panic!("tipo de cable {w}"),
            }
        }
        out
    }
    let mut out = vec![];
    for (c, capa, _) in campos(t) {
        assert_eq!(c, 3);
        for (c, f, _) in campos(&capa) {
            if c != 2 {
                continue;
            }
            let (mut tipo, mut id, mut geom) = (0, None, vec![]);
            for (c, b, v) in campos(&f) {
                match c {
                    1 => id = Some(v),
                    3 => tipo = v,
                    4 => {
                        let mut i = 0;
                        while i < b.len() {
                            geom.push(var(&b, &mut i));
                        }
                    }
                    _ => {}
                }
            }
            out.push((tipo, id, geom));
        }
    }
    out
}

#[test]
fn una_tesela_con_los_tres_tipos_los_codifica_como_la_spec() {
    let (lon, lat) = (-3.708, 40.412);
    let filas = vec![
        (1, punto(lon, lat)),
        // una línea que cruza la tesela de lado a lado: se recorta
        (2, linea(&[(lon - 0.01, lat), (lon + 0.01, lat + 0.0001)])),
        (3, cuadrado(lon, lat - 0.0005, 0.0002)),
        // la colección: un punto, una línea y un polígono, los tres con el id 4
        (
            4,
            de_partes(
                7,
                &[
                    punto(lon + 0.0003, lat),
                    linea(&[(lon, lat + 0.0003), (lon + 0.0004, lat + 0.0003)]),
                    cuadrado(lon + 0.0004, lat - 0.0004, 0.0001),
                ],
            ),
        ),
        // (una curva no llega: el lago la rechaza al escribir, y el motor la descarta)
    ];
    let (cuenta, ml) = dataset_de(filas);
    let (x, y) = tesela_de(lon, lat, 15);
    let p = pet(&ml, 15, x, y);
    let (b, c) = Motor::nuevo().tesela_con(&p, cuenta.clone()).unwrap();
    let b = b.expect("no vacía");
    assert_eq!(
        (c.puntos, c.lineas, c.poligonos, c.descartados),
        (2, 2, 2, 0),
        "{c:?}"
    );
    assert_eq!(c.elementos, 6);
    let fs = features(&b);
    let de = |id: u64| {
        let mut t: Vec<u64> = fs.iter().filter(|f| f.1 == Some(id)).map(|f| f.0).collect();
        t.sort();
        t
    };
    assert_eq!(
        (de(1), de(2), de(3), de(4)),
        (vec![1], vec![2], vec![3], vec![1, 2, 3])
    );
    for (tipo, id, g) in &fs {
        match tipo {
            // MoveTo(1) y sus dos coordenadas: un punto, sin LineTo
            1 => assert_eq!((g[0] & 7, g[0] >> 3, g.len()), (1, 1, 3)),
            // MoveTo 1 + LineTo n−1, sin ClosePath
            2 => {
                assert_eq!(g[0], (1 << 3) | 1);
                assert_eq!(g[3] & 7, 2);
                assert_ne!(*g.last().unwrap(), (1 << 3) | 7);
                if *id == Some(2) {
                    // recortada al marco con su margen: ningún vértice fuera
                    let (mut x, mut ok) = (0i64, true);
                    let dz = |v: u64| ((v >> 1) as i64) ^ -((v & 1) as i64);
                    for v in [g[1], g[4]] {
                        x += dz(v);
                        ok &= (-BUFFER as i64..=(EXTENT + BUFFER) as i64).contains(&x);
                    }
                    assert!(ok, "{g:?}");
                }
            }
            // el polígono acaba en ClosePath
            3 => assert_eq!(*g.last().unwrap(), (1 << 3) | 7),
            t => panic!("tipo {t}"),
        }
    }
    // la misma petición da los mismos bytes (D-F2-6)
    let (b2, _) = Motor::nuevo().tesela_con(&p, cuenta).unwrap();
    assert_eq!(Some(b), b2);
}

// ── F2·3: el filtro por fila y los turnos ───────────────────────────────────────────────────────

#[test]
fn la_criba_por_fila_descodifica_solo_lo_que_toca_la_tesela() {
    // 10×10 edificios y Cádiz en un solo grupo: el grupo se lee, pero `geom` sólo de lo que toca
    let (cuenta, ml) = dataset(10);
    let (x, y) = tesela_de(-3.708, 40.412, 17);
    let (b, c) = Motor::nuevo()
        .tesela_con(&pet(&ml, 17, x, y), cuenta)
        .unwrap();
    assert!(b.is_some(), "{c:?}");
    assert_eq!((c.grupos_leidos, c.filas_en_grupos), (1, 101), "{c:?}");
    assert!(c.filas > 0 && c.filas < 101, "la criba no cribó: {c:?}");
    // lo que pasa la criba es lo que se pinta (la caja de la fila toca la tesela con su margen)
    assert!(c.elementos <= c.filas, "{c:?}");
}

#[test]
fn los_turnos_ponen_tope_y_se_devuelven_al_soltarlos() {
    let t = Turnos::nuevos(1);
    let a = t
        .entrar(Duration::from_millis(10))
        .expect("el primero entra");
    assert!(
        t.entrar(Duration::from_millis(30)).is_none(),
        "el segundo espera y no entra"
    );
    drop(a);
    assert!(
        t.entrar(Duration::from_millis(10)).is_some(),
        "soltado, vuelve a haber"
    );
    // y uno que espera entra en cuanto otro suelta
    let t = std::sync::Arc::new(Turnos::nuevos(1));
    let b = t.entrar(Duration::ZERO).unwrap();
    let t2 = t.clone();
    let h = std::thread::spawn(move || t2.entrar(Duration::from_secs(5)).is_some());
    std::thread::sleep(Duration::from_millis(50));
    drop(b);
    assert!(h.join().unwrap());
}

// ── F2·3·1: las oleadas, la caché de páginas y lo compartido ───────────────────────────────────

/// Un almacén que tarda en cada rango (como GCS) y cuenta los que le piden.
struct Lento {
    dentro: Arc<dyn Almacen>,
    rangos: std::sync::atomic::AtomicUsize,
}

impl Almacen for Lento {
    fn base(&self) -> String {
        self.dentro.base()
    }
    fn leer(&self, clave: &str) -> Result<Option<String>, String> {
        self.dentro.leer(clave)
    }
    fn existe(&self, clave: &str) -> Result<bool, String> {
        self.dentro.existe(clave)
    }
    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
        self.dentro.subir(clave, cuerpo)
    }
    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
        self.dentro.listar(prefijo)
    }
    fn borrar(&self, clave: &str) -> Result<(), String> {
        self.dentro.borrar(clave)
    }
    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
        self.dentro.leer_bytes(clave)
    }
    fn leer_rango(
        &self,
        clave: &str,
        rango: Option<(u64, u64)>,
    ) -> Result<Option<Vec<u8>>, String> {
        self.rangos
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(150));
        self.dentro.leer_rango(clave, rango)
    }
}

fn lento(cuenta: Arc<dyn Almacen>) -> Arc<Lento> {
    Arc::new(Lento {
        dentro: cuenta,
        rangos: Default::default(),
    })
}

#[test]
fn una_tesela_en_frio_son_pocas_oleadas_y_la_segunda_sale_de_la_cache() {
    let (cuenta, ml) = dataset(10);
    let (x, y) = tesela_de(-3.708, 40.412, 15);
    let m = Motor::nuevo();
    let (b1, c1) = m.tesela_con(&pet(&ml, 15, x, y), cuenta.clone()).unwrap();
    assert!(b1.is_some(), "{c1:?}");
    // el pie (cola + índice de páginas), la cobertura y `geom`+`id`: nada en serie por página
    assert!(c1.oleadas <= 3 && c1.peticiones <= 5, "en frío: {c1:?}");
    assert!(c1.bytes > 0, "{c1:?}");
    // la misma: de la caché de teselas, sin leer nada
    let (b2, c2) = m.tesela_con(&pet(&ml, 15, x, y), cuenta.clone()).unwrap();
    assert_eq!(b1, b2, "misma tesela, mismos bytes");
    assert_eq!(
        (c2.tesela_en_cache, c2.peticiones, c2.bytes),
        (1, 0, 0),
        "{c2:?}"
    );
    assert_eq!(c2.elementos, c1.elementos, "la cuenta dice lo que tiene");
    // la de al lado (al oeste: dentro de la caja del grupo, que llega a Cádiz) comparte sus páginas
    let (_, c3) = m.tesela_con(&pet(&ml, 15, x - 1, y), cuenta).unwrap();
    assert_eq!((c3.tesela_en_cache, c3.peticiones), (0, 0), "{c3:?}");
    assert!(c3.en_cache > 0, "{c3:?}");
}

#[test]
fn el_indice_de_paginas_descarta_paginas_de_la_cobertura() {
    // 150×150 = 22 500 edificios: un grupo de varias páginas (20 000 filas por página), en Hilbert
    let (cuenta, ml) = dataset(150);
    let (x, y) = tesela_de(-3.7099, 40.4101, 17);
    let (b, c) = Motor::nuevo()
        .tesela_con(&pet(&ml, 17, x, y), cuenta.clone())
        .unwrap();
    assert!(b.is_some(), "{c:?}");
    assert!(
        c.filas_en_paginas < c.filas_en_grupos,
        "ninguna página descartada: {c:?}"
    );
    assert!(c.filas <= c.filas_en_paginas && c.elementos > 0, "{c:?}");
    // descartar páginas no cambia la tesela: la misma que con todo el grupo (sin índice) no hay
    // forma de pedirla, pero sí la de un motor nuevo, que es determinista
    let (b2, _) = Motor::nuevo()
        .tesela_con(&pet(&ml, 17, x, y), cuenta)
        .unwrap();
    assert_eq!(b, b2);
}

#[test]
fn dos_teselas_a_la_vez_no_piden_dos_veces_lo_mismo() {
    let (cuenta, ml) = dataset(10);
    let (x, y) = tesela_de(-3.708, 40.412, 15);
    // lo que pide una sola, con un motor nuevo
    let solo = lento(cuenta.clone());
    Motor::nuevo()
        .tesela_con(&pet(&ml, 15, x, y), solo.clone())
        .unwrap();
    let una = solo.rangos.load(std::sync::atomic::Ordering::SeqCst);
    // dos a la vez sobre otro motor: la segunda espera lo que trae la primera
    let juntas = lento(cuenta);
    let m = Arc::new(Motor::nuevo());
    let barrera = Arc::new(std::sync::Barrier::new(2));
    let hilos: Vec<_> = (0..2)
        .map(|_| {
            let (m, j, b, ml) = (m.clone(), juntas.clone(), barrera.clone(), ml.clone());
            std::thread::spawn(move || {
                b.wait();
                m.tesela_con(&pet(&ml, 15, x, y), j).unwrap()
            })
        })
        .collect();
    let rs: Vec<_> = hilos.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(rs[0].0, rs[1].0);
    let compartidas: usize = rs.iter().map(|r| r.1.compartidas + r.1.en_cache).sum();
    assert!(compartidas > 0, "{:?} {:?}", rs[0].1, rs[1].1);
    // la lista de ficheros y los pies los puede pedir cada una; los datos, una vez
    assert!(
        juntas.rangos.load(std::sync::atomic::Ordering::SeqCst) < 2 * una,
        "pidieron {} (una sola: {una})",
        juntas.rangos.load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[test]
fn la_cache_de_paginas_no_pasa_de_su_tope() {
    let (cuenta, ml) = dataset(30);
    let m = Motor::con(lector::Paginas::nuevas(4 << 10, 8), Turnos::nuevos(1));
    for (lon, lat) in [(-3.71, 40.41), (-3.70, 40.42), (-3.705, 40.415)] {
        let (x, y) = tesela_de(lon, lat, 16);
        m.tesela_con(&pet(&ml, 16, x, y), cuenta.clone()).unwrap();
    }
    assert!(
        m.paginas_guardadas() <= 4 << 10,
        "{}",
        m.paginas_guardadas()
    );
}

/// Lo que cuesta la CPU de una tesela con caché (a mano: `cargo test --release -- --ignored`).
#[test]
#[ignore]
fn cuanto_cuesta_la_cpu_con_cache() {
    let (cuenta, ml) = dataset(300);
    let (x, y) = tesela_de(-3.70, 40.45, 14);
    let m = Motor::nuevo();
    let (_, frio) = m.tesela_con(&pet(&ml, 14, x, y), cuenta.clone()).unwrap();
    let t = Instant::now();
    let (_, c) = m.tesela_con(&pet(&ml, 14, x, y + 1), cuenta).unwrap();
    eprintln!("frío {frio:?}\ncaché {} ms {c:?}", t.elapsed().as_millis());
}

// ── F2·3·2·1: la pirámide ───────────────────────────────────────────────────────────────────────

/// Un MVT de una capa `nombre` sin elementos (lo que importa aquí es la capa, no lo que lleva).
fn mvt_vacio(nombre: &str, relleno: u8) -> Vec<u8> {
    let mut capa = vec![];
    piramide_prueba::campo_texto(1, nombre, &mut capa);
    // version = 2, extent = 4096; y un valor de relleno (para que dos teselas no sean iguales)
    capa.extend_from_slice(&[15 << 3, 2, 5 << 3]);
    piramide_prueba::varint(4096, &mut capa);
    piramide_prueba::campo_texto(3, &format!("k{relleno}"), &mut capa);
    let mut t = vec![3 << 3 | 2];
    piramide_prueba::varint(capa.len() as u64, &mut t);
    t.extend_from_slice(&capa);
    t
}

mod piramide_prueba {
    //! Un escritor de PMTiles v3 mínimo (sólo para las pruebas): cabecera, raíz, hojas y datos,
    //! directorios y teselas en gzip, como los escribe tippecanoe.
    use std::io::Write;

    pub fn varint(mut v: u64, o: &mut Vec<u8>) {
        while v >= 0x80 {
            o.push((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
        o.push(v as u8);
    }

    pub fn campo_texto(n: u64, s: &str, o: &mut Vec<u8>) {
        varint(n << 3 | 2, o);
        varint(s.len() as u64, o);
        o.extend_from_slice(s.as_bytes());
    }

    pub fn gzip(b: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
        e.write_all(b).unwrap();
        e.finish().unwrap()
    }

    /// (id, corrida, largo, desplaz)
    fn dir(e: &[(u64, u32, u32, u64)]) -> Vec<u8> {
        let mut o = vec![];
        varint(e.len() as u64, &mut o);
        let mut ultimo = 0;
        for x in e {
            varint(x.0 - ultimo, &mut o);
            ultimo = x.0;
        }
        for x in e {
            varint(x.1 as u64, &mut o);
        }
        for x in e {
            varint(x.2 as u64, &mut o);
        }
        for (k, x) in e.iter().enumerate() {
            let seguido = k > 0 && x.3 == e[k - 1].3 + e[k - 1].2 as u64;
            varint(if seguido { 0 } else { x.3 + 1 }, &mut o);
        }
        gzip(&o)
    }

    /// Un PMTiles con estas teselas (z, x, y, MVT). `por_hoja` > 0: la raíz apunta a hojas de
    /// ese tamaño. Dos teselas seguidas con los mismos bytes van en una corrida.
    pub fn escribir(teselas: &[(u32, u32, u32, Vec<u8>)], por_hoja: usize) -> Vec<u8> {
        let mut t: Vec<(u64, Vec<u8>)> = teselas
            .iter()
            .map(|(z, x, y, b)| (crate::piramide::tile_id(*z, *x, *y), gzip(b)))
            .collect();
        t.sort_by_key(|e| e.0);
        let mut datos: Vec<u8> = vec![];
        let mut entradas: Vec<(u64, u32, u32, u64)> = vec![];
        for (id, b) in &t {
            if let Some(u) = entradas.last_mut() {
                let (a, l) = (u.3 as usize, u.2 as usize);
                if u.0 + u.1 as u64 == *id && datos[a..a + l] == b[..] {
                    u.1 += 1;
                    continue;
                }
            }
            entradas.push((*id, 1, b.len() as u32, datos.len() as u64));
            datos.extend_from_slice(b);
        }
        let (raiz, hojas) = if por_hoja == 0 {
            (dir(&entradas), vec![])
        } else {
            let (mut hojas, mut raiz) = (vec![], vec![]);
            for trozo in entradas.chunks(por_hoja) {
                let h = dir(trozo);
                raiz.push((trozo[0].0, 0, h.len() as u32, hojas.len() as u64));
                hojas.extend_from_slice(&h);
            }
            (dir(&raiz), hojas)
        };
        let (zmin, zmax) = (
            teselas.iter().map(|t| t.0).min().unwrap(),
            teselas.iter().map(|t| t.0).max().unwrap(),
        );
        let mut c = vec![0u8; 127];
        c[..7].copy_from_slice(b"PMTiles");
        c[7] = 3;
        let pon = |c: &mut Vec<u8>, i: usize, v: u64| c[i..i + 8].copy_from_slice(&v.to_le_bytes());
        let r0 = 127u64;
        pon(&mut c, 8, r0);
        pon(&mut c, 16, raiz.len() as u64);
        pon(&mut c, 40, r0 + raiz.len() as u64);
        pon(&mut c, 48, hojas.len() as u64);
        pon(&mut c, 56, r0 + raiz.len() as u64 + hojas.len() as u64);
        pon(&mut c, 64, datos.len() as u64);
        (c[96], c[97], c[98], c[99], c[100], c[101]) = (1, 2, 2, 1, zmin as u8, zmax as u8);
        [c, raiz, hojas, datos].concat()
    }
}

/// La tesela que sigue a (x, y) en la curva de Hilbert (siempre una vecina) y otra vecina que no.
fn siguiente_y_otra(z: u32, x: u32, y: u32) -> ((u32, u32), (u32, u32)) {
    let id = piramide::tile_id(z, x, y);
    let vecinas = [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)];
    let s = *vecinas
        .iter()
        .find(|(a, b)| piramide::tile_id(z, *a, *b) == id + 1)
        .expect("la siguiente en Hilbert es vecina");
    let o = *vecinas
        .iter()
        .find(|(a, b)| (*a, *b) != s && piramide::tile_id(z, *a, *b) != id - 1)
        .unwrap();
    (s, o)
}

fn pet_piramide(ml: &str, ubicacion: &str, z: u32, x: u32, y: u32) -> Peticion {
    let mut p = pet(ml, z, x, y);
    p.piramide = Some(FuentePiramide {
        ubicacion: ubicacion.into(),
        credencial: p.credencial.clone(),
    });
    p
}

#[test]
fn el_id_de_pmtiles_es_el_de_la_spec() {
    // la spec: z0 es 0; z1 en orden de Hilbert (0,0) (0,1) (1,1) (1,0); z2 empieza en 5
    assert_eq!(piramide::tile_id(0, 0, 0), 0);
    assert_eq!(
        [(0, 0), (0, 1), (1, 1), (1, 0)].map(|(x, y)| piramide::tile_id(1, x, y)),
        [1, 2, 3, 4]
    );
    assert_eq!(piramide::tile_id(2, 0, 0), 5);
    assert_eq!(piramide::tile_id(12, 3423, 1763), 19078479);
}

#[test]
fn la_capa_se_renombra_y_lo_demas_no_cambia() {
    let t = mvt_vacio("trip_3m_puntos", 7);
    let r = piramide::renombrar(&t, "query.default.trip_3m_puntos").unwrap();
    assert_eq!(r, mvt_vacio("query.default.trip_3m_puntos", 7));
    // renombrar con el mismo nombre deja los mismos bytes
    assert_eq!(piramide::renombrar(&t, "trip_3m_puntos").unwrap(), t);
}

#[test]
fn una_tesela_de_la_piramide_es_un_get_y_las_de_encima_las_sirve_el_crudo() {
    let (cuenta, ml) = dataset(10);
    let (x, y) = tesela_de(-3.708, 40.412, 12);
    let teselas: Vec<(u32, u32, u32, Vec<u8>)> = (9..=12)
        .flat_map(|z| {
            let (x, y) = tesela_de(-3.708, 40.412, z);
            // dos vecinas con los mismos bytes (una corrida) y una distinta
            // la siguiente en Hilbert con los mismos bytes (una corrida) y una distinta
            let ((xs, ys), otra) = siguiente_y_otra(z, x, y);
            [
                (z, x, y, mvt_vacio("pc", z as u8)),
                (z, xs, ys, mvt_vacio("pc", z as u8)),
                (z, otra.0, otra.1, mvt_vacio("pc", 100 + z as u8)),
            ]
        })
        .collect();
    for por_hoja in [0, 2] {
        let clave = format!("piramides/pc-{por_hoja}.pmtiles");
        cuenta
            .subir(&clave, &piramide_prueba::escribir(&teselas, por_hoja))
            .unwrap();
        let ub = format!("s3://pruebas/{clave}");
        let m = Motor::nuevo();
        // la primera: abrir (cabecera; raíz y hojas) y la tesela, tres oleadas
        let (b, c) = m
            .tesela_con(&pet_piramide(&ml, &ub, 12, x, y), cuenta.clone())
            .unwrap();
        assert_eq!(b.unwrap(), mvt_vacio("edificios", 12), "hojas {por_hoja}");
        assert_eq!((c.piramide, c.oleadas), (1, 3), "{c:?}");
        // la siguiente en Hilbert (la corrida: mismos bytes, el mismo rango): nada que pedir
        let ((xs, ys), _) = siguiente_y_otra(12, x, y);
        let (b, c) = m
            .tesela_con(&pet_piramide(&ml, &ub, 12, xs, ys), cuenta.clone())
            .unwrap();
        assert_eq!(b.unwrap(), mvt_vacio("edificios", 12));
        assert_eq!(
            (c.oleadas, c.peticiones),
            (0, 0),
            "la corrida: mismos bytes, en caché {c:?}"
        );
        let (b, c) = m
            .tesela_con(
                &pet_piramide(&ml, &ub, 11, x / 2 + 3, y / 2 + 3),
                cuenta.clone(),
            )
            .unwrap();
        assert!(
            b.is_none(),
            "una tesela que la pirámide no tiene está vacía"
        );
        assert_eq!((c.piramide, c.peticiones), (1, 0), "{c:?}");
        let (x9, y9) = tesela_de(-3.708, 40.412, 9);
        let (_, (xo, yo)) = siguiente_y_otra(9, x9, y9);
        let (b, c) = m
            .tesela_con(&pet_piramide(&ml, &ub, 9, xo, yo), cuenta.clone())
            .unwrap();
        assert_eq!(b.unwrap(), mvt_vacio("edificios", 109));
        assert_eq!((c.oleadas, c.peticiones), (1, 1), "{c:?}");
        // por encima de la pirámide (z15), el crudo, con los mismos edificios que sin pirámide
        let (x15, y15) = tesela_de(-3.708, 40.412, 15);
        let (b, c) = m
            .tesela_con(&pet_piramide(&ml, &ub, 15, x15, y15), cuenta.clone())
            .unwrap();
        assert_eq!(c.piramide, 0);
        assert!(c.elementos > 0, "{c:?}");
        let (sin, _) = Motor::nuevo()
            .tesela_con(&pet(&ml, 15, x15, y15), cuenta.clone())
            .unwrap();
        assert_eq!(b, sin);
    }
}

#[test]
fn una_piramide_que_no_lo_es_falla_y_no_se_sirve_el_crudo_en_su_lugar() {
    let (cuenta, ml) = dataset(3);
    cuenta
        .subir("piramides/rota.pmtiles", b"no soy un pmtiles")
        .unwrap();
    let (x, y) = tesela_de(-3.708, 40.412, 10);
    let e = Motor::nuevo()
        .tesela_con(
            &pet_piramide(&ml, "s3://pruebas/piramides/rota.pmtiles", 10, x, y),
            cuenta,
        )
        .unwrap_err();
    assert!(e.motivo().contains("pirámide"), "{e:?}");
}

#[test]
fn la_peticion_lleva_la_piramide_con_su_credencial() {
    // la del dataset (s3 en `pide`) no vale para gs://: la de gs
    let t = pide("gs://lago/t/metadata/0.json", 10, 1, 1).replace(
        r#""credencial":{"s3.endpoint":"http://nadie","s3.access-key-id":"k","s3.secret-access-key":"s"}"#,
        r#""credencial":{"gcs.oauth2.token":"t1"}"#,
    );
    // sin credencial de la pirámide: inválida
    let t = t.replace(
        r#""id":"id"}"#,
        r#""id":"id","piramide":{"ubicacion":"gs://ore/piramides/a.pmtiles","credencial":{}}}"#,
    );
    let e = peticion(&t).unwrap_err();
    assert!(
        e.motivo()
            .contains("fuente.piramide.credencial.gcs.oauth2.token"),
        "{e:?}"
    );
    let t = t.replace(
        r#""credencial":{}}"#,
        r#""credencial":{"gcs.oauth2.token":"t2"}}"#,
    );
    let p = peticion(&t).unwrap();
    let f = p.piramide.as_ref().unwrap();
    assert_eq!(f.ubicacion, "gs://ore/piramides/a.pmtiles");
    assert!(matches!(&f.credencial, Credencial::Gcs { token } if token == "t2"));
    // la pirámide entra en la clave de la caché: otra pirámide es otra tesela
    let mut q = p.clone();
    q.piramide.as_mut().unwrap().ubicacion = "gs://ore/piramides/b.pmtiles".into();
    assert_ne!(clave_de(&p), clave_de(&q));
}

/// El lector contra un PMTiles de tippecanoe (a mano: `ORE_PMTILES=<fichero>
/// ORE_PMTILES_MUESTRA=<json> cargo test -p ore-teselas -- --ignored de_tippecanoe`): las mismas
/// teselas que la librería oficial de PMTiles (la muestra la escribe `piramide/muestra.py`).
#[test]
#[ignore]
fn un_pmtiles_de_tippecanoe_da_las_teselas_de_la_libreria_oficial() {
    let fichero = std::env::var("ORE_PMTILES").expect("ORE_PMTILES");
    let muestra =
        std::fs::read_to_string(std::env::var("ORE_PMTILES_MUESTRA").expect("ORE_PMTILES_MUESTRA"))
            .unwrap();
    let cuenta: Arc<dyn Almacen> = Arc::new(Memoria::default());
    cuenta
        .subir("p.pmtiles", &std::fs::read(&fichero).unwrap())
        .unwrap();
    let paginas = lector::Paginas::nuevas(64 << 20, 8);
    let mut cu = Cuenta::default();
    let fin = Instant::now() + Duration::from_secs(60);
    let pi = piramide::Piramide::abrir(&paginas, &cuenta, "p.pmtiles", fin, &mut cu).unwrap();
    let todo = cuenta.leer_bytes("p.pmtiles").unwrap().unwrap();
    let n: serde_json::Value = serde_json::from_str(&muestra).unwrap();
    let (mut iguales, mut vistas) = (0, 0);
    for t in n.as_array().unwrap() {
        let g = |k: &str| t[k].as_i64().unwrap();
        let (z, x, y, largo) = (g("z") as u32, g("x") as u32, g("y") as u32, g("largo"));
        vistas += 1;
        let r = pi.donde(z, x, y);
        let nuestro = r.map_or(-1, |r| (r.end - r.start) as i64);
        if nuestro == largo {
            iguales += 1;
            if let Some(r) = pi.donde(z, x, y) {
                let b = &todo[r.start as usize..r.end as usize];
                pi.mvt(b, "x").expect("descomprime y renombra");
            }
        }
    }
    eprintln!("{iguales} de {vistas} iguales; abrir: {cu:?}");
    assert_eq!(iguales, vistas);
}

#[test]
fn el_resumen_dice_si_hay_piramide_y_que_cubre() {
    let (cuenta, _) = dataset(3);
    let (x, y) = tesela_de(-3.708, 40.412, 10);
    cuenta
        .subir(
            "t/_ore_piramides/0-r1.pmtiles",
            &piramide_prueba::escribir(
                &[
                    (10, x, y, mvt_vacio("c", 1)),
                    (12, x * 4, y * 4, mvt_vacio("c", 2)),
                ],
                0,
            ),
        )
        .unwrap();
    cuenta
        .subir(
            "t/_ore_piramides/0-r1.pmtiles.json",
            br#"{"tipo":"agregado","suelo":13}"#,
        )
        .unwrap();
    let m = Motor::nuevo();
    let f = |u: &str| FuentePiramide {
        ubicacion: u.into(),
        credencial: Credencial::Gcs { token: "t".into() },
    };
    assert!(
        m.resumen_en(&f("s3://pruebas/t/_ore_piramides/otra.pmtiles"), &cuenta)
            .unwrap()
            .is_none()
    );
    let r = m
        .resumen_en(&f("s3://pruebas/t/_ore_piramides/0-r1.pmtiles"), &cuenta)
        .unwrap()
        .unwrap()
        .jcs();
    let v: serde_json::Value = serde_json::from_str(&r).unwrap();
    assert_eq!(
        (v["minzoom"].as_i64(), v["maxzoom"].as_i64()),
        (Some(10), Some(12)),
        "{r}"
    );
    assert_eq!(v["manifiesto"]["tipo"], "agregado", "{r}");
}

#[test]
fn sin_credencial_la_piramide_lee_con_la_del_dataset() {
    let t = pide("gs://lago/t/metadata/0.json", 10, 1, 1)
        .replace(
            r#""credencial":{"s3.endpoint":"http://nadie","s3.access-key-id":"k","s3.secret-access-key":"s"}"#,
            r#""credencial":{"gcs.oauth2.token":"t1"}"#,
        )
        .replace(r#""id":"id"}"#, r#""id":"id","piramide":{"ubicacion":"gs://lago/t/_ore_piramides/0-r1.pmtiles"}}"#);
    let p = peticion(&t).unwrap();
    assert!(matches!(&p.piramide.unwrap().credencial, Credencial::Gcs { token } if token == "t1"));
}
