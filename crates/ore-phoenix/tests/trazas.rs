//! **`leer` de punta a punta con los bytes de un PQS de verdad** (`pruebas-de-fuego/phoenix/trazas/`, grabados
//! en X0): el conector real, el cliente Avatica real y un servidor falso que reproduce lo grabado. Lo que sale
//! es un flujo Arrow IPC, que aquí se vuelve a leer. Corre en el CI sin laboratorio.

#[path = "../../ore-avatica/tests/comun/mod.rs"]
mod comun;

use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::sync::Mutex;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Decimal128Type, Float64Type, Int64Type, Time64MicrosecondType,
    TimestampMicrosecondType,
};
use arrow_array::{Array, RecordBatch};
use arrow_ipc::reader::StreamReader;
use arrow_schema::{DataType, TimeUnit};
use base64::Engine;
use comun::{PREFIJO_PETICION, Respuesta, Servidor};
use ore_avatica::proto::{
    CloseConnectionResponse, CloseStatementResponse, OpenConnectionRequest, WireMessage,
};
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Peticion};
use ore_phoenix::lectura::{Phoenix, Token};
use prost::Message;

const CARPETA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../pruebas-de-fuego/phoenix/trazas"
);

/// Un servidor que contesta con lo grabado, por nombre de petición y en orden, juntando varias trazas.
fn reproducir(trazas: &[&str]) -> Servidor {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut cola: BTreeMap<String, VecDeque<(u16, Vec<u8>)>> = BTreeMap::new();
    for traza in trazas {
        let texto =
            std::fs::read_to_string(format!("{CARPETA}/{traza}.json")).expect("la traza grabada");
        let json: serde_json::Value = serde_json::from_str(&texto).unwrap();
        for e in json.as_array().unwrap() {
            let (Some(p), Some(r)) = (e.get("peticion"), e.get("respuesta")) else {
                continue;
            };
            let p =
                WireMessage::decode(b64.decode(p.as_str().unwrap()).unwrap().as_slice()).unwrap();
            let nombre = p.name.trim_start_matches(PREFIJO_PETICION).to_string();
            cola.entry(nombre).or_default().push_back((
                e["estado"].as_u64().unwrap() as u16,
                b64.decode(r.as_str().unwrap()).unwrap(),
            ));
        }
    }
    let cola = Mutex::new(cola);
    Servidor::arrancar(move |r| {
        if let Some((estado, cuerpo)) = cola
            .lock()
            .unwrap()
            .get_mut(&r.nombre)
            .and_then(|q| q.pop_front())
        {
            return Some(Respuesta { estado, cuerpo });
        }
        match r.nombre.as_str() {
            "CloseStatementRequest" => Some(Respuesta::avatica(
                200,
                "CloseStatementResponse",
                &CloseStatementResponse::default(),
            )),
            "CloseConnectionRequest" => Some(Respuesta::avatica(
                200,
                "CloseConnectionResponse",
                &CloseConnectionResponse::default(),
            )),
            otro => Some(Respuesta {
                estado: 599,
                cuerpo: format!("sin traza para {otro}").into_bytes(),
            }),
        }
    })
}

fn url(s: &Servidor) -> String {
    format!("phoenix://{}", s.direccion)
}

/// Las 23 columnas de `ORE_X0.TIPOS`, en el orden de la tabla.
const TIPOS: [&str; 23] = [
    "K1", "K2", "T_TINY", "T_SMALL", "T_INT", "T_BIG", "T_UINT", "T_ULONG", "T_FLOAT", "T_DOUBLE",
    "T_DEC", "T_DECSIN", "T_BOOL", "T_VAR", "T_CHAR", "T_BIN", "T_VARBIN", "T_DATE", "T_TIME",
    "T_TS", "T_UTS", "T_AINT", "T_AVAR",
];

fn peticion_de_tipos(s: &Servidor) -> Peticion {
    Peticion {
        url: url(s),
        objeto: "ORE_X0.TIPOS".into(),
        proyeccion: TIPOS
            .iter()
            .map(|c| (c.to_lowercase(), c.to_string()))
            .collect(),
        // La traza grabó el `DATE` como días (sin `CAST`): se declara `Date` para pedirlo tal cual.
        tipos: vec![("T_DATE".into(), "Date".into())],
        ..Default::default()
    }
}

fn leer(p: &Peticion) -> Result<Vec<RecordBatch>, ore_driver::Fallo> {
    let mut s = Phoenix::conectar(&p.url)?;
    let mut salida = Vec::new();
    Phoenix::leer(&mut s, &Token::default(), p, &mut salida)?;
    let lector = StreamReader::try_new(Cursor::new(salida), None).expect("un flujo Arrow IPC");
    Ok(lector.map(|b| b.expect("lote")).collect())
}

#[test]
fn los_23_tipos_de_phoenix_llegan_a_arrow_con_su_tipo() {
    let s = reproducir(&["catalogo", "todos_los_tipos"]);
    let lotes = leer(&peticion_de_tipos(&s)).expect("leer");
    assert_eq!(lotes.len(), 1);
    let b = &lotes[0];
    assert_eq!(b.num_rows(), 2);
    let schema = b.schema();
    let tipo = |n: &str| schema.field_with_name(n).unwrap().data_type().clone();
    let oos = |n: &str| {
        schema
            .field_with_name(n)
            .unwrap()
            .metadata()
            .get("ore:tipo")
            .cloned()
            .unwrap()
    };

    // Lo que X0 midió, ahora en Arrow.
    assert_eq!(tipo("t_dec"), DataType::Decimal128(12, 3));
    assert_eq!(oos("t_dec"), "Decimal<12, 3>");
    assert_eq!(tipo("t_decsin"), DataType::Utf8); // `DECIMAL` sin precisión: texto exacto
    assert_eq!(
        tipo("t_ts"),
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(oos("t_ts"), "DateTimeTz");
    assert_eq!(tipo("t_date"), DataType::Date32);
    assert_eq!(tipo("t_time"), DataType::Time64(TimeUnit::Microsecond));
    assert_eq!(tipo("t_bin"), DataType::Utf8);
    assert_eq!(oos("t_bin"), "Opaque");
    assert_eq!(tipo("t_aint"), DataType::Utf8);
    for entero in [
        "k2", "t_tiny", "t_small", "t_int", "t_big", "t_uint", "t_ulong",
    ] {
        assert_eq!(tipo(entero), DataType::Int64, "{entero}");
    }

    let col = |n: &str| b.column_by_name(n).unwrap();
    // Fila `a`, todo lleno.
    assert_eq!(col("k1").as_string::<i32>().value(0), "a");
    assert_eq!(col("t_tiny").as_primitive::<Int64Type>().value(0), -5);
    assert_eq!(
        col("t_uint").as_primitive::<Int64Type>().value(0),
        2_000_000_000
    );
    assert_eq!(
        col("t_ulong").as_primitive::<Int64Type>().value(0),
        9_000_000_000_000_000_000
    );
    assert_eq!(col("t_float").as_primitive::<Float64Type>().value(0), 1.5);
    assert_eq!(
        col("t_dec").as_primitive::<Decimal128Type>().value(0),
        12_345_678
    );
    assert_eq!(
        col("t_decsin").as_string::<i32>().value(0),
        "3.14159265358979323846"
    );
    assert!(col("t_bool").as_boolean().value(0));
    assert_eq!(col("t_var").as_string::<i32>().value(0), "héllo ñ 日本");
    assert_eq!(col("t_char").as_string::<i32>().value(0), "abc");
    assert_eq!(col("t_bin").as_string::<i32>().value(0), "YWIAAA==");
    assert_eq!(col("t_varbin").as_string::<i32>().value(0), "Ynl0ZXM=");
    assert_eq!(col("t_date").as_primitive::<Date32Type>().value(0), 20_746);
    assert_eq!(
        col("t_time")
            .as_primitive::<Time64MicrosecondType>()
            .value(0),
        49_559_000_000
    );
    // Milisegundos → microsegundos: los nanosegundos no estaban.
    assert_eq!(
        col("t_ts")
            .as_primitive::<TimestampMicrosecondType>()
            .value(0),
        1_792_503_959_123_000
    );
    assert_eq!(
        col("t_uts")
            .as_primitive::<TimestampMicrosecondType>()
            .value(0),
        1_792_503_959_500_000
    );
    assert_eq!(col("t_aint").as_string::<i32>().value(0), "[1,2,3]");
    assert_eq!(col("t_avar").as_string::<i32>().value(0), "[\"x\",\"y\"]");
    // Fila `b`: todo nulo salvo la clave.
    assert_eq!(col("k1").as_string::<i32>().value(1), "b");
    assert_eq!(col("k2").as_primitive::<Int64Type>().value(1), 2);
    for n in TIPOS.iter().skip(2) {
        assert!(col(&n.to_lowercase()).is_null(1), "{n}");
    }
}

#[test]
fn un_date_sin_declarar_pide_la_hora_con_cast() {
    // La traza grabó `SELECT *`: da igual lo que se pida, el servidor falso contesta lo grabado. Lo que se
    // comprueba es **qué SQL salió**: un `DATE` de Phoenix lleva fecha y hora y llega como días (§3.4).
    let s = reproducir(&["catalogo", "todos_los_tipos"]);
    let mut p = peticion_de_tipos(&s);
    p.tipos.clear();
    p.proyeccion = vec![("d".into(), "T_DATE".into())];
    let _ = leer(&p); // la respuesta grabada no es la de esta consulta: no importa
    let sql: Vec<ore_avatica::proto::PrepareAndExecuteRequest> = s.de("PrepareAndExecuteRequest");
    assert_eq!(
        sql[0].sql,
        "SELECT CAST(\"T_DATE\" AS TIMESTAMP) AS \"T_DATE\" FROM \"ORE_X0\".\"TIPOS\""
    );
    assert!(sql[0].first_frame_max_size > 0);
}

#[test]
fn una_tabla_que_no_existe_es_objeto() {
    // El catálogo grabado es el de `TIPOS`: pedir otra tabla no encuentra columnas.
    let s = reproducir(&["catalogo"]);
    let mut p = peticion_de_tipos(&s);
    p.objeto = "ORE_X0.NO_EXISTE".into();
    let e = leer(&p).unwrap_err();
    assert_eq!(e.codigo, Codigo::Objeto);
    assert!(e.mensaje.contains("ORE_X0.NO_EXISTE"), "{}", e.mensaje);
    assert!(!e.reintentable);
}

#[test]
fn un_error_de_phoenix_en_la_consulta_llega_limpio_y_en_su_codigo() {
    // El catálogo dice que la tabla está; la consulta grabada es la que falló con `ERROR 1012 (42M03)`.
    let s = reproducir(&["catalogo", "error_tabla"]);
    let e = leer(&peticion_de_tipos(&s)).unwrap_err();
    assert_eq!(e.codigo, Codigo::Objeto);
    assert!(
        e.mensaje
            .starts_with("Phoenix 1012 (42M03): Table undefined"),
        "{}",
        e.mensaje
    );
    // Ni la pila de Java ni el nombre de la excepción salen.
    assert!(
        !e.mensaje.contains("RuntimeException") && !e.mensaje.contains("\tat "),
        "{}",
        e.mensaje
    );
}

#[test]
fn el_tiempo_limite_del_servidor_es_tiempo_y_reintentable() {
    let s = reproducir(&["catalogo", "tiempo_limite"]);
    let e = leer(&peticion_de_tipos(&s)).unwrap_err();
    assert_eq!(e.codigo, Codigo::Tiempo);
    assert!(e.reintentable);
}

#[test]
fn un_statement_perdido_a_mitad_es_un_fallo_de_conexion_reintentable() {
    // Tres filas y, después, el `Fetch` que contesta `missing_statement`: la lectura no se da por terminada.
    let s = reproducir(&["catalogo", "statement_perdido"]);
    let mut p = peticion_de_tipos(&s);
    p.proyeccion = vec![("id".into(), "K2".into())]; // da igual: lo grabado devuelve 1 columna
    let e = leer(&p).unwrap_err();
    assert_eq!(e.codigo, Codigo::Conexion, "{e:?}");
    assert!(e.reintentable);
    assert!(e.mensaje.contains("statement"), "{}", e.mensaje);
}

#[test]
fn un_tiempo_limite_de_la_peticion_abre_su_propia_conexion() {
    let s = reproducir(&["catalogo", "todos_los_tipos"]);
    let mut p = peticion_de_tipos(&s);
    p.timeout_ms = Some(2500);
    leer(&p).expect("leer");
    let abiertas: Vec<OpenConnectionRequest> = s.de("OpenConnectionRequest");
    assert_eq!(abiertas.len(), 2, "la de la sesión y la de esta lectura");
    assert!(
        !abiertas[0].info.contains_key("phoenix.query.timeoutMs"),
        "la sesión no lleva tiempo"
    );
    assert_eq!(
        abiertas[1]
            .info
            .get("phoenix.query.timeoutMs")
            .map(String::as_str),
        Some("2500")
    );
}

#[test]
fn lo_que_no_se_empuja_se_rechaza_antes_de_hablar_con_el_servidor() {
    let s = reproducir(&["catalogo", "todos_los_tipos"]);
    let mut p = peticion_de_tipos(&s);
    p.filtros = vec![ore_driver::Filtro::uno("K1", "eq", "a")];
    let e = leer(&p).unwrap_err();
    assert_eq!(e.codigo, Codigo::Operador);
    // Ni siquiera se preguntó por la tabla: un filtro no se ignora, ni se paga una consulta para rechazarlo.
    assert!(
        !s.nombres()
            .contains(&"PrepareAndExecuteRequest".to_string()),
        "{:?}",
        s.nombres()
    );
    assert!(
        !s.nombres().contains(&"ColumnsRequest".to_string()),
        "{:?}",
        s.nombres()
    );
}

#[test]
fn la_contrasena_no_sale_en_el_error_de_una_url_mala() {
    let e = Phoenix::conectar("phoenix://ana:secreta-9876@127.0.0.1:1?tls=true&tlsInsecure=true")
        .err()
        .expect("falla");
    assert!(!e.mensaje.contains("secreta-9876"), "{}", e.mensaje);
    assert_eq!(e.codigo, Codigo::Operador);
    // Y un puerto sin nadie es `conexion`, reintentable.
    let e = Phoenix::conectar("phoenix://127.0.0.1:1")
        .err()
        .expect("falla");
    assert_eq!((e.codigo, e.reintentable), (Codigo::Conexion, true));
}

#[test]
fn las_capacidades_que_se_declaran() {
    let c: Capacidades = Phoenix::CAPACIDADES;
    assert_eq!(c.conector, "ore-read-phoenix");
}
