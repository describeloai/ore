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
