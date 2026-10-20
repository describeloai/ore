//! **Contra un MongoDB de verdad** (ficha 0069 M1). Se saltan solos si no hay
//! uno: `ORE_MONGO_URL` apunta a un servidor **en el que se puede escribir**
//! (un `mongo:7` en Docker; el test crea y borra su propia base,
//! `ore_m1_<pid>`), y `ORE_MONGO_URL_ATLAS` a un Atlas con la base de pruebas
//! `ore_prueba` (solo se lee).
//!
//! ```text
//! docker run -d --name mongo-x0 -p 27117:27017 mongo:7 --replSet rs0
//! ORE_MONGO_URL='mongodb://localhost:27117/?directConnection=true' cargo test -p ore-mongodb --test laboratorio
//! ```
//!
//! Lo que prueba es lo que las pruebas sin servidor no pueden: que **cada
//! operador, escrito por el traductor, da las filas que daría SQL** sobre datos
//! con `null`, ausentes, arrays y fechas (los mismos casos medidos en X0), y que
//! el esquema inferido y el lote de Arrow salen bien de una muestra real.

use mongodb::bson::{Bson, DateTime, Decimal128, Document, doc, oid::ObjectId};
use mongodb::sync::Client;
use ore_driver::{Filtro, Orden, Peticion, Valor};
use ore_mongodb::lote::valor_en_ruta;
use ore_mongodb::transporte::{Conexion, TipoColeccion};
use ore_mongodb::{Constructor, Observador, Plan, Tipo, Tipos, traducir};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn url() -> Option<String> {
    std::env::var("ORE_MONGO_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

struct Base {
    cliente: Client,
    nombre: String,
    conexion: Conexion,
}

impl Base {
    fn nueva(url: &str) -> Base {
        // Una base por prueba: corren en paralelo y no se pisan.
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let nombre = format!(
            "ore_m1_{}_{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let cliente = Client::with_uri_str(url).expect("cliente");
        cliente.database(&nombre).drop().run().expect("limpiar");
        Base {
            cliente,
            nombre,
            conexion: Conexion::abrir(url).expect("abrir"),
        }
    }

    fn sembrar(&self, coleccion: &str, docs: Vec<Document>) {
        self.cliente
            .database(&self.nombre)
            .collection::<Document>(coleccion)
            .insert_many(docs)
            .run()
            .expect("sembrar");
    }

    /// Las `_id` que devuelve una petición, en el orden en que llegan.
    fn ids(&self, coleccion: &str, p: &Peticion, tipos: &Tipos) -> Vec<i64> {
        let t = traducir(p, tipos).expect("traducir");
        let mut ids = Vec::new();
        let no = AtomicBool::new(false);
        self.conexion
            .ejecutar(&self.nombre, coleccion, &t.plan, &no, None, &mut |d| {
                let v = valor_en_ruta(d, "_id").unwrap().expect("_id");
                ids.push(match v {
                    mongodb::bson::raw::RawBsonRef::Int32(n) => i64::from(n),
                    mongodb::bson::raw::RawBsonRef::Int64(n) => n,
                    otro => panic!("_id inesperado: {otro:?}"),
                });
                Ok(())
            })
            .expect("ejecutar");
        ids
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = self.cliente.database(&self.nombre).drop().run();
    }
}

fn pet(cols: &[&str]) -> Peticion {
    Peticion {
        proyeccion: cols
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        ..Default::default()
    }
}

fn con(mut p: Peticion, f: Filtro) -> Peticion {
    p.filtros.push(f);
    p
}

fn lista(col: &str, op: &str, vs: &[&str]) -> Filtro {
    Filtro {
        columna: col.into(),
        operador: op.into(),
        valor: Valor::Lista(vs.iter().map(|s| s.to_string()).collect()),
    }
}

fn sin_valor(col: &str, op: &str) -> Filtro {
    Filtro {
        columna: col.into(),
        operador: op.into(),
        valor: Valor::Ninguno,
    }
}

fn ordenados(mut v: Vec<i64>) -> Vec<i64> {
    v.sort();
    v
}

#[test]
fn cada_operador_da_las_filas_que_daria_sql() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    // c: 5, 7, null, ausente, [5, 6], [null], 10 — los casos medidos en X0.
    b.sembrar(
        "sem",
        vec![
            doc! {"_id": 1, "c": 5},
            doc! {"_id": 2, "c": 7_i64},
            doc! {"_id": 3, "c": Bson::Null},
            doc! {"_id": 4},
            doc! {"_id": 5, "c": [5, 6]},
            doc! {"_id": 6, "c": [Bson::Null]},
            doc! {"_id": 7, "c": 10},
        ],
    );
    let tipos = Tipos::new([
        ("c".to_string(), Tipo::Entero),
        ("_id".to_string(), Tipo::Entero),
    ]);
    let ids = |f: Filtro| ordenados(b.ids("sem", &con(pet(&["_id"]), f), &tipos));

    // eq: el array [5, 6] NO casa (el `{c: 5}` ingenuo lo casaría).
    assert_eq!(ids(Filtro::uno("c", "eq", "5")), [1]);
    // in
    assert_eq!(ids(lista("c", "in", &["5", "10"])), [1, 7]);
    assert_eq!(ids(lista("c", "in", &[])), Vec::<i64>::new());
    // neq: ni null, ni ausente, ni arrays.
    assert_eq!(ids(Filtro::uno("c", "neq", "5")), [2, 7]);
    // orden
    assert_eq!(ids(Filtro::uno("c", "lt", "8")), [1, 2]);
    assert_eq!(ids(Filtro::uno("c", "le", "7")), [1, 2]);
    assert_eq!(ids(Filtro::uno("c", "gt", "7")), [7]);
    assert_eq!(ids(Filtro::uno("c", "ge", "7")), [2, 7]);
    // isNull: el `null` y el ausente; el array con null NO (`{c: null}` lo casaría).
    assert_eq!(ids(sin_valor("c", "isNull")), [3, 4]);
    assert_eq!(ids(sin_valor("c", "isNotNull")), [1, 2, 5, 6, 7]);
    // Dos filtros: conjunción.
    let p = con(
        con(pet(&["_id"]), Filtro::uno("c", "ge", "5")),
        Filtro::uno("c", "lt", "10"),
    );
    assert_eq!(ordenados(b.ids("sem", &p, &tipos)), [1, 2]);
    // El recorte por clave.
    let mut p = pet(&["_id"]);
    p.clave_columnas = vec!["_id".into()];
    p.claves = vec![vec!["2".into()], vec!["7".into()], vec!["99".into()]];
    assert_eq!(ordenados(b.ids("sem", &p, &tipos)), [2, 7]);
    // limit
    let mut p = pet(&["_id"]);
    p.limit = Some(3);
    assert_eq!(b.ids("sem", &p, &tipos).len(), 3);
}

#[test]
fn like_es_el_de_sql() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar(
        "txt",
        vec![
            doc! {"_id": 1, "s": "axb"},
            doc! {"_id": 2, "s": "a_b"},
            doc! {"_id": 3, "s": "ab"},
            doc! {"_id": 4, "s": "A"},
            doc! {"_id": 5, "s": "anb\nx"},
            doc! {"_id": 6, "s": "a\nb"},
            doc! {"_id": 7, "s": "100%_seguro"},
            doc! {"_id": 8, "s": "1.5"},
            doc! {"_id": 9, "s": "105"},
            doc! {"_id": 10, "s": Bson::Null},
        ],
    );
    let tipos = Tipos::new([
        ("s".to_string(), Tipo::Texto),
        ("_id".to_string(), Tipo::Entero),
    ]);
    let ids = |pat: &str| {
        let p = con(pet(&["_id", "s"]), Filtro::uno("s", "like", pat));
        ordenados(b.ids("txt", &p, &tipos))
    };
    // `a_b`: un carácter cualquiera, también un salto de línea (la bandera `s`),
    // y no cero caracteres.
    assert_eq!(ids("a_b"), [1, 2, 6]);
    // Anclado: `%n%` no casa «A», y `A%` es sensible a mayúsculas.
    assert_eq!(ids("%n%"), [5]);
    assert_eq!(ids("A%"), [4]);
    // Los comodines de dentro del texto son datos; la barra no escapa nada
    // (el `like` del protocolo no tiene escape) y el punto es un punto.
    assert_eq!(
        ids(r"100\%_seguro"),
        Vec::<i64>::new(),
        "sin escape: la barra es literal"
    );
    assert_eq!(ids("100%_seguro"), [7]);
    assert_eq!(ids("1.5"), [8], "el punto no es «cualquiera»");
    assert_eq!(ids("1_5"), [8, 9]);
    // El nulo no casa con nada, ni con `%`.
    assert_eq!(ids("%"), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn el_orden_pone_los_nulos_al_final_en_los_dos_sentidos() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar(
        "ord",
        vec![
            doc! {"_id": 1, "c": 3},
            doc! {"_id": 2, "c": Bson::Null},
            doc! {"_id": 3, "c": 1},
            doc! {"_id": 4},
            doc! {"_id": 5, "c": 2},
        ],
    );
    let tipos = Tipos::new([
        ("c".to_string(), Tipo::Entero),
        ("_id".to_string(), Tipo::Entero),
    ]);
    let con_orden = |desc: bool, limit: Option<u64>| {
        let mut p = pet(&["_id", "c"]);
        p.orden = vec![Orden {
            columna: "c".into(),
            descendente: desc,
        }];
        p.limit = limit;
        b.ids("ord", &p, &tipos)
    };
    // ASC: 1, 2, 3 y luego los nulos y los ausentes (por `_id`). DESC: 3, 2, 1 y los nulos.
    assert_eq!(con_orden(false, None), [3, 5, 1, 2, 4]);
    assert_eq!(con_orden(true, None), [1, 5, 3, 2, 4]);
    // Un `limit` empujado da las mismas n filas que daría el motor.
    assert_eq!(con_orden(false, Some(2)), [3, 5]);
    assert_eq!(con_orden(true, Some(4)), [1, 5, 3, 2]);
}

#[test]
fn los_instantes_y_los_decimales_comparan_con_su_tipo() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    let base = 1_767_225_600_000_i64; // 2026-01-01T00:00:00Z
    b.sembrar("fec", vec![
        doc! {"_id": 1, "f": DateTime::from_millis(base + 1), "d": Decimal128::from_str("10.50").unwrap()},
        doc! {"_id": 2, "f": DateTime::from_millis(base + 2), "d": Decimal128::from_str("10.5").unwrap()},
        doc! {"_id": 3, "f": DateTime::from_millis(base + 3), "d": Decimal128::from_str("3.25").unwrap()},
    ]);
    let tipos = Tipos::new([
        ("_id".to_string(), Tipo::Entero),
        ("f".to_string(), Tipo::Instante),
        (
            "d".to_string(),
            Tipo::Decimal {
                precision: 38,
                escala: 2,
            },
        ),
    ]);
    let ids = |f: Filtro| ordenados(b.ids("fec", &con(pet(&["_id"]), f), &tipos));
    // 1,5 ms: ninguna fecha de BSON es igual, y el redondeo respeta el sentido.
    let t = "2026-01-01T00:00:00.001500+00:00";
    assert_eq!(ids(Filtro::uno("f", "eq", t)), Vec::<i64>::new());
    assert_eq!(ids(Filtro::uno("f", "lt", t)), [1]);
    assert_eq!(ids(Filtro::uno("f", "le", t)), [1]);
    assert_eq!(ids(Filtro::uno("f", "gt", t)), [2, 3]);
    assert_eq!(ids(Filtro::uno("f", "ge", t)), [2, 3]);
    assert_eq!(ids(Filtro::uno("f", "neq", t)), [1, 2, 3]);
    assert_eq!(
        ids(Filtro::uno("f", "eq", "2026-01-01T00:00:00.002000+00:00")),
        [2]
    );
    // `10.50` y `10.5` son el mismo número.
    assert_eq!(ids(Filtro::uno("d", "eq", "10.5")), [1, 2]);
    assert_eq!(ids(Filtro::uno("d", "gt", "10.5")), Vec::<i64>::new());
    assert_eq!(ids(Filtro::uno("d", "lt", "10.5")), [3]);
}

#[test]
fn una_proyeccion_cerrada_devuelve_solo_lo_pedido() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar(
        "pro",
        vec![doc! {"_id": 1, "a": {"b": 5, "c": 6}, "z": "no se pide", "edad": 3}],
    );
    let tipos = Tipos::new([("edad".to_string(), Tipo::Entero)]);
    // `a` y `a.b`: se pide `a`, y `b` se saca de dentro.
    let mut p = pet(&["a", "a.b"]);
    p.filtros = vec![Filtro::uno("edad", "gt", "1")]; // filtra por `edad`, que no se pide
    let t = traducir(&p, &tipos).unwrap();
    let mut claves = Vec::new();
    b.conexion
        .ejecutar(
            &b.nombre,
            "pro",
            &t.plan,
            &AtomicBool::new(false),
            None,
            &mut |d| {
                claves = d.iter().map(|x| x.unwrap().0.to_string()).collect();
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(claves, ["a"], "ni `_id`, ni `z`, ni `edad`");
}

#[test]
fn el_esquema_y_el_lote_de_una_muestra_real() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    let mut docs = Vec::new();
    for i in 1..=300_i32 {
        let mut d = doc! {
            "_id": i, "nombre": format!("n{i}"), "edad": i % 90,
            "precio": Decimal128::from_str(&format!("{i}.25")).unwrap(),
            "alta": DateTime::from_millis(1_700_000_000_000 + i64::from(i)),
            "oid": ObjectId::new(), "cliente": {"ciudad": "Madrid", "geo": {"lat": 40.4}},
            "lineas": [{"sku": "A"}],
        };
        if i % 7 == 0 {
            d.insert("edad", "siete");
        } // tipos mezclados
        if i % 5 == 0 {
            d.remove("precio");
        } // ausente
        docs.push(d);
    }
    b.sembrar("m", docs);

    // El catálogo: la colección sale, el esquema se infiere.
    let cols = b.conexion.colecciones(&b.nombre).unwrap();
    assert_eq!(
        cols.iter().map(|c| c.nombre.as_str()).collect::<Vec<_>>(),
        ["m"]
    );
    assert_eq!(cols[0].tipo, TipoColeccion::Coleccion);
    let muestra = b
        .conexion
        .muestra(&b.nombre, "m", 1000, Duration::from_secs(30), None)
        .unwrap();
    assert_eq!(
        muestra.documentos.len(),
        300,
        "menos que lo pedido: se lee entera"
    );
    let mut o = Observador::new(3);
    for d in &muestra.documentos {
        o.observar(d).unwrap();
    }
    let e = o.esquema();
    let tipo = |r: &str| {
        e.columna(r)
            .unwrap_or_else(|| panic!("sin {r}: {:?}", e.columnas))
            .tipo
            .clone()
    };
    assert_eq!(tipo("_id"), Tipo::Entero);
    assert_eq!(tipo("edad"), Tipo::Mixto, "números y texto");
    assert_eq!(
        tipo("precio"),
        Tipo::Decimal {
            precision: 38,
            escala: 2
        }
    );
    assert_eq!(tipo("alta"), Tipo::Instante);
    assert_eq!(tipo("oid"), Tipo::ObjectId);
    assert_eq!(tipo("cliente.ciudad"), Tipo::Texto);
    // `cliente` es el nivel 1, `geo` el 2 y `lat` el 3: con profundidad 3 se aplana hasta la hoja.
    assert_eq!(tipo("cliente.geo.lat"), Tipo::Real);
    assert!(
        e.columna("cliente.geo").is_none(),
        "un documento aplanado no es una columna"
    );
    assert_eq!(tipo("lineas"), Tipo::Json);
    assert!(
        e.avisos
            .iter()
            .any(|a| a.ruta == "edad" && a.mensaje.contains("tipos mezclados"))
    );

    // Leer toda la colección en Arrow por el esquema.
    let rutas: Vec<(String, String, Tipo)> = e
        .columnas
        .iter()
        .map(|c| (c.ruta.clone(), c.ruta.clone(), c.tipo.clone()))
        .collect();
    let mut cons = Constructor::nuevo(&rutas).unwrap();
    let p = Peticion {
        proyeccion: e
            .columnas
            .iter()
            .map(|c| (c.ruta.clone(), c.ruta.clone()))
            .collect(),
        ..Peticion::default()
    };
    let t = traducir(&p, &Tipos::de_esquema(&e)).unwrap();
    let mut filas = 0;
    b.conexion
        .ejecutar(
            &b.nombre,
            "m",
            &t.plan,
            &AtomicBool::new(false),
            None,
            &mut |d| {
                cons.push(d)
                    .map_err(|e| ore_driver::Fallo::origen(e.to_string()))?;
                filas += 1;
                Ok(())
            },
        )
        .unwrap();
    let lote = cons.finish().unwrap();
    assert_eq!(
        (filas, lote.num_rows(), lote.num_columns()),
        (300, 300, e.columnas.len())
    );
}

#[test]
fn el_testigo_la_cancelacion_y_los_errores() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    assert!(b.conexion.ping().unwrap() < Duration::from_secs(5));
    let (version, conjunto) = b.conexion.servidor().unwrap();
    assert!(
        version.starts_with(|c: char| c.is_ascii_digit()),
        "{version}"
    );
    // El testigo es `segundos:incremento` si hay replica set; si no, `None`.
    match (conjunto, b.conexion.testigo().unwrap()) {
        (Some(_), Some(t)) => {
            let (a, c) = t.split_once(':').expect("t:i");
            assert!(a.parse::<u64>().is_ok() && c.parse::<u64>().is_ok(), "{t}");
        }
        (None, None) => {}
        otro => panic!("{otro:?}"),
    }
    // Cancelar: con la marca activada se para sin leer.
    b.sembrar("c", (1..=2000).map(|i| doc! {"_id": i, "x": i}).collect());
    let t = traducir(&pet(&["x"]), &Tipos::default()).unwrap();
    let cancelar = AtomicBool::new(true);
    let s = b
        .conexion
        .ejecutar(&b.nombre, "c", &t.plan, &cancelar, None, &mut |_| {
            panic!("no debería leer")
        })
        .unwrap();
    assert_eq!((s.filas, s.cancelada), (0, true));
    // Un timeout en el origen es `tiempo`; una colección sin permiso o sin ser, otra cosa.
    let Plan::Find {
        filtro, proyeccion, ..
    } = t.plan
    else {
        panic!()
    };
    let lento = Plan::Find {
        filtro: doc! {"$where": "sleep(100) || true"},
        proyeccion,
        limit: None,
        max_time: Some(Duration::from_millis(300)),
    };
    let _ = filtro;
    let e = b
        .conexion
        .ejecutar(
            &b.nombre,
            "c",
            &lento,
            &AtomicBool::new(false),
            None,
            &mut |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(e.codigo, ore_driver::Codigo::Tiempo, "{e}");
    assert!(e.codigo.reintentable());
}

/// Contra el Atlas de desarrollo, solo lectura: el esquema de `ore_prueba`.
#[test]
fn el_esquema_de_ore_prueba_en_atlas() {
    let Some(u) = std::env::var("ORE_MONGO_URL_ATLAS")
        .ok()
        .filter(|u| !u.is_empty())
    else {
        return eprintln!("sin ORE_MONGO_URL_ATLAS: se salta");
    };
    let c = Conexion::abrir(&u).unwrap();
    let bases = c.bases().unwrap();
    assert!(bases.contains(&"ore_prueba".to_string()), "{bases:?}");
    let cols = c.colecciones("ore_prueba").unwrap();
    let nombres: Vec<&str> = cols.iter().map(|c| c.nombre.as_str()).collect();
    for esperada in [
        "tipos",
        "polimorfica",
        "anidada",
        "ids_variados",
        "vacia",
        "con_validador",
        "vista_tipos",
        "serie",
    ] {
        assert!(nombres.contains(&esperada), "falta {esperada}: {nombres:?}");
    }
    assert!(
        !nombres.iter().any(|n| n.starts_with("system.")),
        "sin system.*"
    );
    let tipo_de = |n: &str| cols.iter().find(|c| c.nombre == n).unwrap().tipo;
    assert_eq!(tipo_de("vista_tipos"), TipoColeccion::Vista);
    assert_eq!(tipo_de("serie"), TipoColeccion::SerieTemporal);
    assert!(
        cols.iter()
            .find(|c| c.nombre == "con_validador")
            .unwrap()
            .validador
            .is_some()
    );

    let m = c
        .muestra(
            "ore_prueba",
            "polimorfica",
            1000,
            Duration::from_secs(30),
            None,
        )
        .unwrap();
    assert_eq!(m.documentos.len(), 1000);
    assert_eq!(m.total, 2000);
    let mut o = Observador::new(3);
    for d in &m.documentos {
        o.observar(d).unwrap();
    }
    let e = o.esquema();
    // `edad` es número, texto, null y ausente: mezclado. `etiquetas`, array y objeto: JSON.
    assert_eq!(e.columna("edad").unwrap().tipo, Tipo::Mixto);
    assert_eq!(e.columna("etiquetas").unwrap().tipo, Tipo::Json);
    assert_eq!(e.columna("nombre").unwrap().tipo, Tipo::Texto);

    let a = c
        .muestra("ore_prueba", "anidada", 1000, Duration::from_secs(30), None)
        .unwrap();
    let mut o = Observador::new(3);
    for d in &a.documentos {
        o.observar(d).unwrap();
    }
    let e = o.esquema();
    assert_eq!(e.columna("cliente.nombre").unwrap().tipo, Tipo::Texto);
    assert_eq!(
        e.columna("cliente.direccion.ciudad").unwrap().tipo,
        Tipo::Texto
    );
    assert_eq!(e.columna("lineas").unwrap().tipo, Tipo::Json);
    assert_eq!(e.columna("meta.creado").unwrap().tipo, Tipo::Instante);
    assert_eq!(
        a.documentos.len(),
        100,
        "una colección pequeña se lee entera"
    );

    // Las pruebas de escritura no se pueden hacer con este usuario.
    assert!(c.testigo().unwrap().is_some());
}

// ── M4 · los cambios ────────────────────────────────────────────────────────

/// Lo que devuelve una lectura por posición: `(_id, _ore_op)` de cada fila, en el
/// orden del changelog.
fn filas_de_cambios(bytes: Vec<u8>) -> Vec<(i64, String)> {
    use arrow_array::cast::AsArray;
    use arrow_array::types::Int64Type;
    let lector = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None)
        .expect("un flujo Arrow");
    let mut v = Vec::new();
    for lote in lector {
        let lote = lote.unwrap();
        let ids = lote
            .column_by_name("id")
            .unwrap()
            .as_primitive::<Int64Type>()
            .clone();
        let ops = lote
            .column_by_name("_ore_op")
            .unwrap()
            .as_string::<i32>()
            .clone();
        for i in 0..lote.num_rows() {
            v.push((ids.value(i), ops.value(i).to_string()));
        }
    }
    v
}

fn pet_cambios(url: &str, coleccion: &str, base: &str, start: &str, end: Option<&str>) -> Peticion {
    Peticion {
        url: url.into(),
        objeto: format!("{base}.{coleccion}"),
        proyeccion: vec![("id".into(), "_id".into()), ("n".into(), "n".into())],
        start: Some(start.into()),
        end: end.map(String::from),
        ..Default::default()
    }
}

/// **Un alta, una actualización y un borrado entre dos posiciones**, y qué se
/// salta: la actualización de un documento que ya no existe (su borrado viene
/// después y lo dice).
#[test]
fn los_cambios_entre_dos_posiciones() {
    use ore_driver::origen::Origen;
    use ore_mongodb::lectura::{Mongo, Token};
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar(
        "c",
        vec![
            doc! {"_id": 1, "n": "a"},
            doc! {"_id": 2, "n": "b"},
            doc! {"_id": 3, "n": "c"},
        ],
    );
    let col = b.cliente.database(&b.nombre).collection::<Document>("c");
    let t0 = b
        .conexion
        .testigo()
        .unwrap()
        .expect("un replica set tiene testigo");
    col.insert_one(doc! {"_id": 4, "n": "d"}).run().unwrap();
    col.update_one(doc! {"_id": 1}, doc! {"$set": {"n": "a2"}})
        .run()
        .unwrap();
    col.delete_one(doc! {"_id": 2}).run().unwrap();
    col.update_one(doc! {"_id": 3}, doc! {"$set": {"n": "c2"}})
        .run()
        .unwrap();
    col.delete_one(doc! {"_id": 3}).run().unwrap();
    let t1 = b.conexion.testigo().unwrap().unwrap();
    assert!(t1 > t0, "el testigo es ordinal: {t0} < {t1}");

    let mut s = Mongo::conectar(&u).unwrap();
    // Una prueba rápida de datos: la muestra se toma de lo que hay ahora.
    let mut fuera = Vec::new();
    let n = Mongo::leer(
        &mut s,
        &Token::default(),
        &pet_cambios(&u, "c", &b.nombre, &t0, Some(&t1)),
        &mut fuera,
    )
    .unwrap();
    let filas = filas_de_cambios(fuera);
    assert_eq!(n, filas.len() as u64);
    assert_eq!(
        filas,
        [
            (4, "upsert".to_string()),
            (1, "upsert".to_string()),
            (2, "delete".to_string()),
            // la actualización de 3 se salta (ya no existe) y su borrado sale
            (3, "delete".to_string()),
        ]
    );

    // `start` es exclusivo: desde `t1` no hay nada, y `end` por defecto es ahora.
    let mut fuera = Vec::new();
    Mongo::leer(
        &mut s,
        &Token::default(),
        &pet_cambios(&u, "c", &b.nombre, &t1, None),
        &mut fuera,
    )
    .unwrap();
    assert!(filas_de_cambios(fuera).is_empty());
}

/// Lo que un changelog no admite se rechaza diciéndolo, y no se sirve otra cosa.
#[test]
fn un_rango_por_posicion_rechaza_lo_que_no_sabe() {
    use ore_driver::origen::Origen;
    use ore_mongodb::lectura::{Mongo, Token};
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar("c", vec![doc! {"_id": 1, "n": "a"}]);
    let t = b.conexion.testigo().unwrap().unwrap();
    let mut s = Mongo::conectar(&u).unwrap();
    let mut leer = |p: Peticion| Mongo::leer(&mut s, &Token::default(), &p, &mut Vec::new());

    let mut con_filtro = pet_cambios(&u, "c", &b.nombre, &t, None);
    con_filtro.filtros = vec![Filtro::uno("n", "eq", "a")];
    let e = leer(con_filtro).unwrap_err();
    assert_eq!(e.codigo, ore_driver::Codigo::Operador);
    assert!(e.mensaje.contains("sin filtrar"), "{}", e.mensaje);

    let mut con_limit = pet_cambios(&u, "c", &b.nombre, &t, None);
    con_limit.limit = Some(1);
    assert_eq!(
        leer(con_limit).unwrap_err().codigo,
        ore_driver::Codigo::Operador
    );

    // «Desde el principio» no es una posición del oplog.
    let mut sin_start = pet_cambios(&u, "c", &b.nombre, &t, None);
    sin_start.start = None;
    sin_start.end = Some(t.clone());
    let e = leer(sin_start).unwrap_err();
    assert!(e.mensaje.contains("necesita `start`"), "{}", e.mensaje);

    // Una posición que no es de este origen.
    let e = leer(pet_cambios(&u, "c", &b.nombre, "LSN 0/16B3748", None)).unwrap_err();
    assert_eq!(e.codigo, ore_driver::Codigo::Operador);
}

/// **La posición caduca y no se inventa continuidad**: una posición que el oplog
/// ya no tiene falla diciendo que hay que copiar entera, y una colección que se
/// borra a mitad de rango también.
#[test]
fn una_posicion_caducada_no_se_disfraza() {
    use ore_driver::origen::Origen;
    use ore_mongodb::lectura::{Mongo, Token};
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar("c", vec![doc! {"_id": 1, "n": "a"}]);
    let mut s = Mongo::conectar(&u).unwrap();

    // Una posición que el oplog **de verdad** ya no tiene: hay que haberlo truncado
    // (`pruebas-de-fuego/mongodb/caduca-el-oplog.py` da una), porque un servidor
    // que nunca perdió historia acepta cualquier posición anterior (medido: una de
    // 1970 no falla). Sin `ORE_MONGO_CADUCADA` se salta.
    if let Ok(vieja) = std::env::var("ORE_MONGO_CADUCADA") {
        let e = Mongo::leer(
            &mut s,
            &Token::default(),
            &pet_cambios(&u, "c", &b.nombre, &vieja, None),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(e.mensaje.contains("posición caducada"), "{e:?}");
        assert!(!e.reintentable, "caducar no se arregla reintentando");
    }

    // La colección se borra después de la posición.
    let t = b.conexion.testigo().unwrap().unwrap();
    b.cliente
        .database(&b.nombre)
        .collection::<Document>("c")
        .drop()
        .run()
        .unwrap();
    let e = Mongo::leer(
        &mut s,
        &Token::default(),
        &pet_cambios(&u, "c", &b.nombre, &t, None),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        e.mensaje.contains("no hay continuidad") || e.mensaje.contains("no tiene documentos"),
        "{e:?}"
    );
}

/// La cara `D` de una colección: sondeada, no declarada a ojo.
#[test]
fn el_catalogo_declara_los_cambios_que_el_servidor_da() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar("c", vec![doc! {"_id": 1, "n": "a"}]);
    let con_base = format!(
        "{}{}?directConnection=true",
        u.split('?').next().unwrap(),
        b.nombre
    );
    let cat = ore_mongodb::verbos::catalogo(&con_base, "m").unwrap();
    let con_cambios = b.conexion.admite_cambios().unwrap();
    let esperado = if con_cambios { "upsert" } else { "none" };
    assert!(cat.contains(&format!("\"mode\": \"{esperado}\"")), "{cat}");
    if con_cambios {
        assert!(cat.contains("\"witness\": \"log\""), "{cat}");
    }
}

/// `cambio` no tiene huella que respalde, y una colección ausente es un error.
#[test]
fn cambio_no_respalda_y_una_ausente_es_un_error() {
    let Some(u) = url() else {
        return eprintln!("sin ORE_MONGO_URL: se salta");
    };
    let b = Base::nueva(&u);
    b.sembrar("c", vec![doc! {"_id": 1}]);
    let pide = |objeto: &str| format!(r#"{{"url":"{u}","objeto":"{objeto}"}}"#);
    let r = ore_mongodb::verbos::cambio(&pide(&format!("{}.c", b.nombre))).unwrap();
    assert!(r.contains("\"respalda\":false"), "{r}");
    let e = ore_mongodb::verbos::cambio(&pide(&format!("{}.no_esta", b.nombre))).unwrap_err();
    assert!(e.contains("no está en el origen"), "{e}");
}

/// El error de la posición caducada, por su código, sin un servidor que lo provoque
/// (el caso real, medido con el oplog truncado, es `ChangeStreamHistoryLost`, 286).
#[test]
fn los_codigos_de_posicion_caducada_dicen_que_hay_que_copiar_entera() {
    for (code, nombre) in [
        (286, "ChangeStreamHistoryLost"),
        (280, "ChangeStreamFatalError"),
        (136, "CappedPositionLost"),
    ] {
        let f = ore_mongodb::transporte::fallo_de_comando(code, nombre, "ya no está");
        assert_eq!(f.codigo, ore_driver::Codigo::Origen, "{nombre}");
        assert!(
            f.mensaje.contains("posición caducada"),
            "{nombre}: {}",
            f.mensaje
        );
        assert!(
            f.mensaje.contains("copiar entera"),
            "{nombre}: {}",
            f.mensaje
        );
        assert!(!f.reintentable, "{nombre}");
    }
    // Los demás siguen como estaban.
    assert_eq!(
        ore_mongodb::transporte::fallo_de_comando(50, "MaxTimeMSExpired", "x").codigo,
        ore_driver::Codigo::Tiempo
    );
}

/// El testigo es un ordinal: con ancho fijo, comparar el texto es comparar el
/// instante (con `9:10` frente a `10:2` el orden de los textos sin relleno miente).
#[test]
fn el_testigo_se_ordena_como_el_instante() {
    use mongodb::bson::Timestamp;
    use ore_mongodb::transporte::{testigo_de, testigo_texto};
    let a = Timestamp {
        time: 9,
        increment: 10,
    };
    let b = Timestamp {
        time: 10,
        increment: 2,
    };
    assert!(testigo_texto(a) < testigo_texto(b));
    assert!(
        testigo_texto(Timestamp {
            time: 10,
            increment: 2
        }) < testigo_texto(Timestamp {
            time: 10,
            increment: 10
        })
    );
    assert_eq!(testigo_de(&testigo_texto(b)).unwrap(), b);
    assert!(testigo_de("LSN 0/16B3748").is_err());
    assert!(testigo_de("1:x").is_err());
}

/// **Cancelar con el usuario de solo lectura de un Atlas**: la operación propia se
/// ve (`$ownOps`) y se mata sin el privilegio `killOp` (medido en X0 como pendiente,
/// y en M4 como cierto). La consulta es un `$lookup` sin índice, que tarda segundos.
#[test]
fn se_cancela_en_atlas_con_un_usuario_de_solo_lectura() {
    use std::sync::atomic::AtomicBool;
    let Ok(u) = std::env::var("ORE_MONGO_URL_ATLAS") else {
        return eprintln!("sin ORE_MONGO_URL_ATLAS: se salta");
    };
    let c = Conexion::abrir(&u).unwrap();
    let marca = format!("ore-prueba-cancelar-{}", std::process::id());
    let plan = Plan::Agregado {
        tubo: vec![doc! {"$lookup": {
            "from": "comments", "localField": "text", "foreignField": "text", "as": "x"
        }}],
        max_time: Some(Duration::from_secs(25)),
    };
    let c2 = c.clone();
    let m = marca.clone();
    let hilo = std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let r = c2.ejecutar(
            "sample_mflix",
            "comments",
            &plan,
            &AtomicBool::new(false),
            Some(&m),
            &mut |_| Ok(()),
        );
        (r, t.elapsed())
    });
    std::thread::sleep(Duration::from_secs(3));
    let t = std::time::Instant::now();
    let muertas = c.matar(&marca).unwrap();
    let (r, total) = hilo.join().unwrap();
    assert_eq!(muertas, 1, "la operación propia se ve y se mata");
    assert!(r.is_err(), "la lectura termina interrumpida");
    assert!(
        total < Duration::from_secs(15),
        "se cortó en {total:?}, no a los 25 s del maxTimeMS"
    );
    eprintln!("cancelada en Atlas: {:?} tras el killOp", t.elapsed());
}
