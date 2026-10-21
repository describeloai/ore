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
use comun::{PREFIJO_PETICION, Recibida, Respuesta, Servidor};
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

/// Cómo se casa una petición con lo grabado: por nombre y en orden, salvo `getColumns`, que se casa por
/// tabla (el catálogo puede pedir un subconjunto de las que se grabaron).
fn clave(nombre: &str, mensaje: &[u8]) -> String {
    if nombre == "ColumnsRequest" {
        let c = ore_avatica::proto::ColumnsRequest::decode(mensaje).unwrap();
        return format!(
            "ColumnsRequest:{}:{}",
            c.schema_pattern, c.table_name_pattern
        );
    }
    nombre.to_string()
}

/// Un servidor que contesta con lo grabado, por nombre de petición y en orden, juntando varias trazas.
fn reproducir(trazas: &[&str]) -> Servidor {
    reproducir_con(trazas, |_| None)
}

/// Lo mismo, con un `gancho` que ve cada petición antes: si contesta, manda; si devuelve `None`, contesta lo grabado.
fn reproducir_con(
    trazas: &[&str],
    gancho: impl Fn(&Recibida) -> Option<Respuesta> + Send + Sync + 'static,
) -> Servidor {
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
            let nombre = clave(
                p.name.trim_start_matches(PREFIJO_PETICION),
                &p.wrapped_message,
            );
            cola.entry(nombre).or_default().push_back((
                e["estado"].as_u64().unwrap() as u16,
                b64.decode(r.as_str().unwrap()).unwrap(),
            ));
        }
    }
    // Una tabla que no se grabó: el servidor contesta con las columnas de otra, y quien lee filtra por
    // nombre exacto y no la encuentra (lo que haría un `getColumns` real de una tabla que no existe).
    let ajena = cola
        .iter()
        .find(|(k, _)| k.starts_with("ColumnsRequest:"))
        .and_then(|(_, q)| q.front().cloned());
    let cola = Mutex::new(cola);
    Servidor::arrancar(move |r| {
        if let Some(propia) = gancho(r) {
            return Some(propia);
        }
        if let Some((estado, cuerpo)) = cola
            .lock()
            .unwrap()
            .get_mut(&clave(&r.nombre, &r.mensaje))
            .and_then(|q| q.pop_front())
        {
            return Some(Respuesta { estado, cuerpo });
        }
        match r.nombre.as_str() {
            "ColumnsRequest" if ajena.is_some() => ajena
                .clone()
                .map(|(estado, cuerpo)| Respuesta { estado, cuerpo }),
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
    // Un operador fuera del vocabulario: los diez se empujan, este no existe.
    p.filtros = vec![ore_driver::Filtro::uno("K1", "regex", "a")];
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

// ── M3 · el catálogo, con la traza `catalogo_completo` (getTables de todo y getColumns de cada una) ──

fn catalogo_de(url: &str) -> Result<ore_driver::catalogo::Catalogo, String> {
    let texto = ore_phoenix::verbos::catalogo(url, "phoenix")?;
    Ok(ore_driver::catalogo::Catalogo::leer(&texto).expect("el lector del catálogo lo entiende"))
}

fn nombres(c: &ore_driver::catalogo::Catalogo) -> Vec<&str> {
    c.tablas.iter().map(|t| t.nombre.as_str()).collect()
}

#[test]
fn el_catalogo_trae_tablas_y_vistas_sin_el_sistema_ni_indices_ni_vistas_de_inquilino() {
    let s = reproducir(&["catalogo_completo"]);
    let c = catalogo_de(&url(&s)).expect("catálogo");
    let n = nombres(&c);
    for esperado in [
        "SUELTA",
        "ORE_X0.PEDIDOS",
        "ORE_X0.PEDIDOS_CERRADOS",
        "ORE_X0.SALADA",
        "ORE_X0.INQUILINOS",
        "ORE_X0.TIPOS",
        "ORE_X0.minusculas",
        "ORE_X0_AJENO.SECRETOS",
    ] {
        assert!(n.contains(&esperado), "falta {esperado}: {n:?}");
    }
    // Ni el catálogo de Phoenix, ni un índice, ni las vistas de inquilino que `getColumns` no ve.
    assert!(
        !n.iter()
            .any(|x| x.starts_with("SYSTEM") || x.contains("IDX_") || x.contains("VISTA_T")),
        "{n:?}"
    );
    // Las sin esquema primero, luego por esquema y nombre.
    let con_esquema = n.iter().position(|x| x.contains('.')).unwrap();
    assert!(n[..con_esquema].contains(&"SUELTA"), "{n:?}");
    assert!(n[con_esquema..].iter().all(|x| x.contains('.')), "{n:?}");
    let resto = n[con_esquema..].to_vec();
    let mut ordenado = resto.clone();
    ordenado.sort();
    assert_eq!(resto, ordenado);
}

#[test]
fn la_tabla_del_catalogo_lleva_tipos_clave_y_las_dos_caras() {
    let s = reproducir(&["catalogo_completo"]);
    let c = catalogo_de(&url(&s)).unwrap();
    let t = |n: &str| c.tablas.iter().find(|t| t.nombre == n).unwrap();
    let pedidos = t("ORE_X0.PEDIDOS");
    assert_eq!(pedidos.clave, ["ID"]);
    assert_eq!(pedidos.clase, "table");
    let col = |t: &ore_driver::catalogo::Tabla, n: &str| {
        t.columnas.iter().find(|k| k.nombre == n).unwrap().clone()
    };
    assert_eq!(
        col(pedidos, "IMPORTE").tipo.as_deref(),
        Some("Decimal<12, 2>")
    );
    assert_eq!(col(pedidos, "CREADO").tipo.as_deref(), Some("DateTimeTz"));
    assert!(col(pedidos, "ID").obligatoria && !col(pedidos, "CLIENTE").obligatoria);
    // El tipo de Phoenix, tal cual.
    let tipos = t("ORE_X0.TIPOS");
    assert_eq!(col(tipos, "T_UINT").origen.as_deref(), Some("UNSIGNED_INT"));
    assert_eq!(col(tipos, "T_UINT").tipo.as_deref(), Some("Integer"));
    assert_eq!(
        col(tipos, "T_AINT").origen.as_deref(),
        Some("INTEGER ARRAY")
    );
    assert_eq!(tipos.clave, ["K1", "K2"]);
    // Una vista es una vista; el nombre en minúsculas citado se conserva tal cual.
    assert_eq!(t("ORE_X0.PEDIDOS_CERRADOS").clase, "view");
    assert_eq!(
        col(t("ORE_X0.minusculas"), "Mixta").origen.as_deref(),
        Some("VARCHAR")
    );
    // Las dos caras: no empuja nada (M4) y leer entero es caro; sin cambios.
    let lee = pedidos.lee.clone().unwrap().jcs();
    assert!(
        lee.contains("\"predicatePushdown\":[]") && lee.contains("\"fullScan\":\"expensive\""),
        "{lee}"
    );
    assert_eq!(
        pedidos.cambia.clone().unwrap().jcs(),
        r#"{"mode":"none","witness":"none"}"#
    );
    // No se inventa un número que el `EXPLAIN` no sabe dar (X0 §3.9).
    assert!(c.tablas.iter().all(|t| t.filas_estimadas.is_none()));
}

#[test]
fn si_la_url_nombra_un_esquema_solo_se_cataloga_ese() {
    let s = reproducir(&["catalogo_completo"]);
    let c = catalogo_de(&format!("{}/ORE_X0", url(&s))).unwrap();
    let n = nombres(&c);
    assert!(n.contains(&"ORE_X0.PEDIDOS"));
    assert!(
        n.iter().all(|x| x.starts_with("ORE_X0.")),
        "ni `SUELTA` ni `ORE_X0_AJENO`: {n:?}"
    );
}

#[test]
fn explorar_lista_los_esquemas_y_dice_lo_que_recorre() {
    let s = reproducir(&["catalogo_completo"]);
    let out =
        ore_phoenix::verbos::explorar(&format!(r#"{{"url":"{}"}}"#, url(&s))).expect("explorar");
    assert!(
        out.contains("\"ORE_X0\"") && out.contains("\"ORE_X0_AJENO\""),
        "{out}"
    );
    assert!(!out.contains("\"nombre\": \"SYSTEM\""), "{out}");
    assert!(out.contains("sin esquema"), "{out}");
}

#[test]
fn si_ninguna_tabla_se_deja_catalogar_es_un_error_que_lo_dice() {
    // Todo lo que `getTables` lista, `getColumns` lo niega (como una base de solo vistas de inquilino).
    let b64 = base64::engine::general_purpose::STANDARD;
    let texto = std::fs::read_to_string(format!("{CARPETA}/catalogo_completo.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&texto).unwrap();
    let mut tablas = None;
    let mut negada = None;
    for e in json.as_array().unwrap() {
        let (Some(p), Some(r)) = (e.get("peticion"), e.get("respuesta")) else {
            continue;
        };
        let p = WireMessage::decode(b64.decode(p.as_str().unwrap()).unwrap().as_slice()).unwrap();
        let r = b64.decode(r.as_str().unwrap()).unwrap();
        let estado = e["estado"].as_u64().unwrap() as u16;
        if p.name.ends_with("TablesRequest") && tablas.is_none() {
            tablas = Some((estado, r));
        } else if p.name.ends_with("ColumnsRequest") && estado == 500 && negada.is_none() {
            negada = Some((estado, r));
        }
    }
    let (tablas, negada) = (tablas.unwrap(), negada.unwrap());
    let s = Servidor::arrancar(move |r| match r.nombre.as_str() {
        "TablesRequest" => Some(Respuesta {
            estado: tablas.0,
            cuerpo: tablas.1.clone(),
        }),
        "ColumnsRequest" => Some(Respuesta {
            estado: negada.0,
            cuerpo: negada.1.clone(),
        }),
        otro => match otro {
            "OpenConnectionRequest" => Some(Respuesta::avatica(
                200,
                "OpenConnectionResponse",
                &ore_avatica::proto::OpenConnectionResponse::default(),
            )),
            _ => Some(Respuesta::avatica(
                200,
                "CloseConnectionResponse",
                &CloseConnectionResponse::default(),
            )),
        },
    });
    let e = catalogo_de(&url(&s)).unwrap_err();
    assert!(e.contains("ninguna de las") && e.contains("tenant="), "{e}");
    assert!(e.contains("Phoenix 1012"), "el motivo literal: {e}");
}

#[test]
fn el_catalogo_no_enseña_la_contrasena_si_la_conexion_falla() {
    // Puerto 1: nadie escucha.
    let e =
        ore_phoenix::verbos::catalogo("phoenix://ana:secreto-9@127.0.0.1:1?tls=true", "phoenix")
            .unwrap_err();
    assert!(!e.contains("secreto-9"), "{e}");
    let e =
        ore_phoenix::verbos::explorar(r#"{"url":"phoenix://ana:secreto-9@127.0.0.1:1?tls=true"}"#)
            .unwrap_err();
    assert!(!e.contains("secreto-9"), "{e}");
}

#[test]
fn catalogo_sin_url_se_dice() {
    let e = ore_phoenix::verbos::catalogo("", "phoenix").unwrap_err();
    assert!(e.contains("stdin"), "{e}");
}

#[test]
fn los_filtros_el_orden_y_el_limit_salen_en_el_sql_que_recibe_el_servidor() {
    let s = reproducir(&["catalogo", "todos_los_tipos"]);
    let mut p = peticion_de_tipos(&s);
    p.proyeccion = vec![("k".into(), "K1".into())];
    p.tipos.clear();
    p.filtros = vec![
        ore_driver::Filtro::uno("K1", "neq", r"a\' OR 1=1 --"),
        ore_driver::Filtro::uno("K1", "gt", ""),
        ore_driver::Filtro::uno("K2", "ge", "2"),
    ];
    p.orden = vec![ore_driver::Orden {
        columna: "K2".into(),
        descendente: true,
    }];
    p.limit = Some(10);
    let _ = leer(&p); // lo grabado no es la respuesta de esta consulta: no importa
    let sql: Vec<ore_avatica::proto::PrepareAndExecuteRequest> = s.de("PrepareAndExecuteRequest");
    assert_eq!(
        sql[0].sql,
        r#"SELECT "K1" FROM "ORE_X0"."TIPOS" WHERE "K1" <> 'a\\'' OR 1=1 --' AND "K1" IS NOT NULL AND "K2" >= 2 ORDER BY "K2" DESC NULLS LAST LIMIT 10"#
    );
}

#[test]
fn un_filtro_que_no_se_sabe_escribir_no_llega_al_servidor() {
    let s = reproducir(&["catalogo", "todos_los_tipos"]);
    let mut p = peticion_de_tipos(&s);
    p.filtros = vec![ore_driver::Filtro::uno("T_DOUBLE", "eq", "NaN")];
    let e = leer(&p).unwrap_err();
    assert_eq!(e.codigo, Codigo::Operador);
    assert!(
        !s.nombres()
            .contains(&"PrepareAndExecuteRequest".to_string()),
        "{:?}",
        s.nombres()
    );
}

// ── M5 · cancelar ──────────────────────────────────────────────────────────────────────────────────────

/// El `ExecuteResponse` que se grabó de una consulta, para cambiarlo.
fn ejecucion_grabada(traza: &str) -> ore_avatica::proto::ExecuteResponse {
    let b64 = base64::engine::general_purpose::STANDARD;
    let texto = std::fs::read_to_string(format!("{CARPETA}/{traza}.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&texto).unwrap();
    for e in json.as_array().unwrap() {
        let Some(p) = e.get("peticion") else { continue };
        let p = WireMessage::decode(b64.decode(p.as_str().unwrap()).unwrap().as_slice()).unwrap();
        if p.name.ends_with("PrepareAndExecuteRequest") {
            let r = b64.decode(e["respuesta"].as_str().unwrap()).unwrap();
            let w = WireMessage::decode(r.as_slice()).unwrap();
            return ore_avatica::proto::ExecuteResponse::decode(w.wrapped_message.as_slice())
                .unwrap();
        }
    }
    panic!("la traza `{traza}` no trae un PrepareAndExecute");
}

#[test]
fn cancelar_cierra_el_statement_en_el_servidor_aunque_el_hilo_que_lee_siga_esperando_un_fetch() {
    use ore_avatica::proto::{FetchResponse, Frame};
    use std::time::{Duration, Instant};
    // El primer frame no es el último, y el siguiente `Fetch` tarda 3 s en contestar.
    let mut ejecucion = ejecucion_grabada("todos_los_tipos");
    ejecucion.results[0].first_frame.as_mut().unwrap().done = false;
    let cierre = std::sync::Arc::new(Mutex::new(None::<Instant>));
    let c2 = cierre.clone();
    let s = reproducir_con(&["catalogo", "todos_los_tipos"], move |r| {
        match r.nombre.as_str() {
            "PrepareAndExecuteRequest" => {
                Some(Respuesta::avatica(200, "ExecuteResponse", &ejecucion))
            }
            "FetchRequest" => {
                std::thread::sleep(Duration::from_secs(3));
                Some(Respuesta::avatica(
                    200,
                    "FetchResponse",
                    &FetchResponse {
                        frame: Some(Frame {
                            done: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ))
            }
            "CloseStatementRequest" => {
                c2.lock().unwrap().get_or_insert_with(Instant::now);
                None
            }
            _ => None,
        }
    });
    let p = peticion_de_tipos(&s);
    let token = Token::default();
    let inicio = Instant::now();
    let hilo = {
        let (token, p) = (token.clone(), p.clone());
        std::thread::spawn(move || {
            let mut sesion = Phoenix::conectar(&p.url).unwrap();
            Phoenix::leer(&mut sesion, &token, &p, &mut Vec::new())
        })
    };
    std::thread::sleep(Duration::from_millis(700));
    // Lo que hace el esqueleto con `SIGTERM`: llama a `cancelar` y termina, sin esperar al hilo que lee.
    Phoenix::cancelar(&token);
    let cerrado_a = cierre
        .lock()
        .unwrap()
        .map(|t| t.duration_since(inicio))
        .expect("el servidor recibió un CloseStatement");
    assert!(
        cerrado_a < Duration::from_secs(2),
        "el statement se cerró a los {cerrado_a:?}, con el Fetch aún colgado (3 s)"
    );
    let e = hilo.join().unwrap().unwrap_err();
    assert!(e.mensaje.contains("cancelada"), "{}", e.mensaje);
    assert!(
        inicio.elapsed() >= Duration::from_secs(3),
        "el Fetch seguía pendiente"
    );
}
