//! **La puerta de `/v1`** contra un `ore-serve` de mentira (C5): las rutas y el cuerpo del SDK, el reintento de un
//! 5xx con la misma `Idempotency-Key`, el 409 y el esbozo de una tabla que nace. Contra el `ore-serve` de verdad se
//! prueba en victor (C10).

use datafusion::arrow::array::{ArrayRef, Int64Array};
use datafusion::arrow::record_batch::RecordBatch;
use ore_store::lago::{Lago, Operacion};
use ore_tabla::Nombre;
use ore_tabla::escritura::{FalloCommit, Puerta};
use ore_tabla::memoria::Memoria;
use ore_tabla::v1::{Credencial, PuertaV1};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Lo que llegó: método, ruta, `Idempotency-Key`, `Authorization` y cuerpo.
#[derive(Debug, Clone)]
struct Peticion {
    metodo: String,
    ruta: String,
    clave: Option<String>,
    token: Option<String>,
    pod: Option<String>,
    cuerpo: Value,
}

/// Un servidor que contesta, en orden, lo que se le da, y apunta lo que le piden.
fn servidor(respuestas: Vec<(u16, Value)>) -> (String, Arc<Mutex<Vec<Peticion>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let vistas = Arc::new(Mutex::new(Vec::new()));
    let v = vistas.clone();
    std::thread::spawn(move || {
        for (codigo, cuerpo) in respuestas {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut linea = String::new();
            r.read_line(&mut linea).unwrap();
            let mut partes = linea.split_whitespace();
            let (metodo, ruta) = (
                partes.next().unwrap().to_string(),
                partes.next().unwrap().to_string(),
            );
            let mut cabeceras = HashMap::new();
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                let (k, v) = h.split_once(':').unwrap();
                cabeceras.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
            let largo: usize = cabeceras
                .get("content-length")
                .map_or(0, |n| n.parse().unwrap());
            let mut b = vec![0; largo];
            r.read_exact(&mut b).unwrap();
            v.lock().unwrap().push(Peticion {
                metodo,
                ruta,
                clave: cabeceras.get("idempotency-key").cloned(),
                token: cabeceras.get("authorization").cloned(),
                pod: cabeceras.get("x-ore-pod").cloned(),
                cuerpo: serde_json::from_slice(&b).unwrap_or(Value::Null),
            });
            let texto = cuerpo.to_string();
            let mut s = s;
            write!(
                s,
                "HTTP/1.1 {codigo} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{texto}",
                texto.len()
            )
            .unwrap();
        }
    });
    (url, vistas)
}

fn lago() -> Lago {
    Lago::nuevo(Arc::new(Memoria::default()))
}

/// Unos `requirements + updates` de verdad: los de escribir dos filas en una tabla que nace.
fn un_commit(lago: &Lago) -> (iceberg::table::Table, ore_store::lago::Preparado) {
    let a: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    let lote = RecordBatch::try_from_iter_with_nullable([("id", a, true)]).unwrap();
    let esquema = Lago::esquema_de_tabla_nueva(&lote.schema()).unwrap();
    let tabla = lago
        .crear("catalogo/ventas/eu/t", esquema.clone(), HashMap::new())
        .unwrap();
    let p = lago
        .preparar(
            &tabla,
            esquema,
            vec![lote],
            Operacion::Sobrescribir,
            HashMap::new(),
        )
        .unwrap();
    (tabla, p)
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn el_commit_va_como_el_del_sdk_y_un_5xx_se_reintenta_con_la_misma_clave() {
    let (url, vistas) = servidor(vec![
        (503, json!({"error": {"message": "un momento"}})),
        (
            200,
            json!({"metadata-location": "gs://c/ore/v2/catalogo/ventas/eu/t/metadata/1.json"}),
        ),
    ]);
    let lago = lago();
    let (tabla, p) = un_commit(&lago);
    let puerta = PuertaV1::new(url, "el-token", lago).con_esperas(vec![Duration::from_millis(10)]);
    let n = Nombre::new("ventas", "eu", "t");
    let ml = rt()
        .block_on(puerta.confirmar(&n, &tabla, p.requisitos, p.cambios, "la-clave"))
        .unwrap();
    assert_eq!(ml, "gs://c/ore/v2/catalogo/ventas/eu/t/metadata/1.json");
    let v = vistas.lock().unwrap().clone();
    assert_eq!(v.len(), 2);
    for p in &v {
        assert_eq!(p.metodo, "POST");
        assert_eq!(p.ruta, "/v1/ventas/namespaces/eu/tables/t");
        assert_eq!(
            p.clave.as_deref(),
            Some("la-clave"),
            "la misma clave en el reintento"
        );
        assert_eq!(p.token.as_deref(), Some("Bearer el-token"));
        assert_eq!(
            p.cuerpo["identifier"],
            json!({"namespace": ["eu"], "name": "t"})
        );
        assert!(!p.cuerpo["requirements"].as_array().unwrap().is_empty());
        assert!(!p.cuerpo["updates"].as_array().unwrap().is_empty());
    }
}

/// C10·3: desde un pod del puesto, el commit va con el token del pod (`x-ore-pod`), leído en cada petición porque el
/// kubelet lo renueva; nunca como portador. `ore-serve` lo escribe a nombre de la persona del puesto.
#[test]
fn desde_un_puesto_el_commit_va_con_el_token_del_pod() {
    let (url, vistas) = servidor(vec![(
        200,
        json!({"metadata-location": "gs://c/ore/v2/catalogo/ventas/eu/t/metadata/1.json"}),
    )]);
    let ruta = std::env::temp_dir().join(format!("ore-pod-v1-{}", std::process::id()));
    std::fs::write(&ruta, "el-token-del-pod\n").unwrap();
    let lago = lago();
    let (tabla, p) = un_commit(&lago);
    let puerta = PuertaV1::con(url, Credencial::Pod(ruta), Arc::new(lago));
    rt().block_on(puerta.confirmar(
        &Nombre::new("ventas", "eu", "t"),
        &tabla,
        p.requisitos,
        p.cambios,
        "k",
    ))
    .unwrap();
    let v = vistas.lock().unwrap().clone();
    assert_eq!(v[0].pod.as_deref(), Some("el-token-del-pod"));
    assert_eq!(v[0].token, None);
}

#[test]
fn un_409_es_un_conflicto_y_no_se_reintenta() {
    let (url, vistas) = servidor(vec![(
        409,
        json!({"error": {"message": "CommitFailedException"}}),
    )]);
    let lago = lago();
    let (tabla, p) = un_commit(&lago);
    let puerta = PuertaV1::new(url, "t", lago).con_esperas(vec![Duration::from_millis(10)]);
    let e = rt()
        .block_on(puerta.confirmar(
            &Nombre::new("ventas", "eu", "t"),
            &tabla,
            p.requisitos,
            p.cambios,
            "k",
        ))
        .unwrap_err();
    assert_eq!(e, FalloCommit::Conflicto { hay: None });
    assert_eq!(vistas.lock().unwrap().len(), 1);
}

#[test]
fn una_tabla_que_nace_es_un_stage_create() {
    let lago = lago();
    let a: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let lote = RecordBatch::try_from_iter_with_nullable([("id", a, true)]).unwrap();
    let esquema = Lago::esquema_de_tabla_nueva(&lote.schema()).unwrap();
    // Lo que `ore-serve` contesta: los metadatos de la tabla esbozada.
    let esbozo = lago
        .crear("catalogo/ventas/eu/t", esquema.clone(), HashMap::new())
        .unwrap();
    let (url, vistas) = servidor(vec![(
        200,
        json!({"metadata": serde_json::to_value(esbozo.metadata()).unwrap(), "config": {}}),
    )]);
    let puerta = PuertaV1::new(url, "t", lago);
    let tabla = rt()
        .block_on(puerta.esbozar(&Nombre::new("ventas", "eu", "t"), esquema))
        .unwrap();
    assert!(
        tabla.metadata_location().is_none(),
        "un esbozo no está en ningún sitio todavía"
    );
    let v = vistas.lock().unwrap().clone();
    assert_eq!(v[0].metodo, "POST");
    assert_eq!(v[0].ruta, "/v1/ventas/namespaces/eu/tables");
    assert_eq!(v[0].cuerpo["name"], "t");
    assert_eq!(v[0].cuerpo["stage-create"], true);
    assert_eq!(v[0].clave, None);
}

#[test]
fn retirar_es_un_drop_table() {
    let (url, vistas) = servidor(vec![(204, json!({}))]);
    let puerta = PuertaV1::new(url, "t", lago());
    rt().block_on(puerta.retirar(&Nombre::new("ventas", "default", "d"), "gs://x", "k"))
        .unwrap();
    let v = vistas.lock().unwrap().clone();
    assert_eq!(v[0].metodo, "DELETE");
    assert_eq!(v[0].ruta, "/v1/ventas/namespaces/default/tables/d");
}
