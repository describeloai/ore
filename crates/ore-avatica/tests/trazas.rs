//! El cliente contra **los bytes de un PQS de verdad** (`pruebas-de-fuego/phoenix/trazas/`, grabados en X0
//! con `trazas.py`). El servidor falso reproduce cada respuesta en su sitio; el cliente es el de producción.
//! Corre en el CI sin laboratorio.

mod comun;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use base64::Engine;
use comun::{PREFIJO_PETICION, Respuesta, Servidor};
use ore_avatica::proto::{
    CloseConnectionResponse, CloseStatementResponse, FetchRequest, PrepareAndExecuteRequest,
    WireMessage,
};
use ore_avatica::{Anulable, Conexion, Error, Opciones, Valor};
use prost::Message;

const CARPETA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../pruebas-de-fuego/phoenix/trazas"
);

/// Un servidor que contesta con lo grabado, por nombre de petición y en orden.
fn reproducir(traza: &str) -> Servidor {
    let texto =
        std::fs::read_to_string(format!("{CARPETA}/{traza}.json")).expect("la traza grabada");
    let json: serde_json::Value = serde_json::from_str(&texto).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut cola: BTreeMap<String, VecDeque<(u16, Vec<u8>)>> = BTreeMap::new();
    for e in json.as_array().unwrap() {
        let (Some(p), Some(r)) = (e.get("peticion"), e.get("respuesta")) else {
            continue;
        };
        let p = WireMessage::decode(b64.decode(p.as_str().unwrap()).unwrap().as_slice()).unwrap();
        let nombre = p.name.trim_start_matches(PREFIJO_PETICION).to_string();
        cola.entry(nombre).or_default().push_back((
            e["estado"].as_u64().unwrap() as u16,
            b64.decode(r.as_str().unwrap()).unwrap(),
        ));
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
        // El cierre de lo que se abrió no está siempre grabado (el cliente de Python lo hace al recolectar).
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

fn conectar(s: &Servidor) -> Conexion {
    let mut o = Opciones::nuevas(s.url());
    o.filas_por_frame = 3;
    Conexion::abrir(&o).expect("abrir")
}

#[test]
fn una_lectura_paginada_del_pqs_real() {
    let s = reproducir("lectura_paginada");
    let c = conectar(&s);
    let mut l = c
        .ejecutar("SELECT id, cliente, importe FROM ore_x0.pedidos ORDER BY id")
        .expect("ejecutar");

    let nombres: Vec<&str> = l.columnas().iter().map(|c| c.nombre.as_str()).collect();
    assert_eq!(nombres, ["ID", "CLIENTE", "IMPORTE"]);
    assert_eq!(l.columnas()[0].tipo, "BIGINT");
    assert_eq!(l.columnas()[2].tipo, "DECIMAL");
    assert_eq!((l.columnas()[2].precision, l.columnas()[2].escala), (12, 2));

    let f1 = l.siguiente_frame().unwrap().expect("primer frame");
    assert_eq!(f1.len(), 3);
    assert_eq!(
        f1[0],
        vec![
            Valor::Entero(1),
            Valor::Texto("cli001".into()),
            Valor::Decimal("1.25".into())
        ]
    );
    let f2 = l.siguiente_frame().unwrap().expect("segundo frame");
    let ids: Vec<&Valor> = f2.iter().map(|f| &f[0]).collect();
    assert_eq!(
        ids,
        [&Valor::Entero(4), &Valor::Entero(5), &Valor::Entero(6)]
    );
    assert_eq!(l.leidas(), 6);
    drop(l);
    drop(c);

    // Invariante 1: el primer frame se pide con tamaño. Invariante 3: el Fetch lleva las filas leídas.
    let ejecutar: Vec<PrepareAndExecuteRequest> = s.de("PrepareAndExecuteRequest");
    assert_eq!(ejecutar.len(), 1);
    assert_eq!(ejecutar[0].first_frame_max_size, 3);
    let fetch: Vec<FetchRequest> = s.de("FetchRequest");
    assert_eq!(
        (fetch.len(), fetch[0].offset, fetch[0].frame_max_size),
        (1, 3, 3)
    );
    // El orden de lo que se hizo: abrir, crear, ejecutar, traer, y cerrar lo abierto.
    assert_eq!(
        s.nombres(),
        [
            "OpenConnectionRequest",
            "CreateStatementRequest",
            "PrepareAndExecuteRequest",
            "FetchRequest",
            "CloseStatementRequest",
            "CloseConnectionRequest"
        ]
    );
    // El tipo de contenido que el PQS espera (medido en X0).
    assert!(
        s.recibidas
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.tipo_contenido.as_deref() == Some("application/x-google-protobuf"))
    );
}

#[test]
fn todos_los_tipos_de_phoenix_como_llegan() {
    let s = reproducir("todos_los_tipos");
    let c = conectar(&s);
    let mut l = c
        .ejecutar("SELECT * FROM ore_x0.tipos ORDER BY k1")
        .expect("ejecutar");
    let cols: Vec<(String, String)> = l
        .columnas()
        .iter()
        .map(|c| (c.nombre.clone(), c.tipo.clone()))
        .collect();
    let tipo = |n: &str| cols.iter().find(|(c, _)| c == n).unwrap().1.clone();
    // Lo que distingue un tipo sin signo es el nombre, no `rep` (0071 §3.4).
    assert_eq!(tipo("T_UINT"), "UNSIGNED_INT");
    assert_eq!(tipo("T_ULONG"), "UNSIGNED_LONG");
    assert_eq!(tipo("T_UTS"), "UNSIGNED_TIMESTAMP");
    assert_eq!(tipo("T_AINT"), "INTEGER ARRAY");
    assert_eq!(cols.len(), 23);

    let t_dec = l.columnas().iter().find(|c| c.nombre == "T_DEC").unwrap();
    assert_eq!((t_dec.precision, t_dec.escala), (12, 3));
    let t_aint = l.columnas().iter().find(|c| c.nombre == "T_AINT").unwrap();
    assert_eq!(t_aint.componente.as_ref().unwrap().tipo, "INTEGER");
    assert_eq!(l.columnas()[0].anulable, Anulable::No); // k1 es de la clave
    assert_eq!(l.columnas()[2].anulable, Anulable::Si);

    let filas = l.siguiente_frame().unwrap().expect("filas");
    assert_eq!(filas.len(), 2);
    let a: BTreeMap<&str, &Valor> = l
        .columnas()
        .iter()
        .map(|c| c.nombre.as_str())
        .zip(filas[0].iter())
        .collect();
    assert_eq!(a["K1"], &Valor::Texto("a".into()));
    assert_eq!(a["T_TINY"], &Valor::Entero(-5));
    assert_eq!(a["T_BIG"], &Valor::Entero(-9_000_000_000));
    assert_eq!(a["T_ULONG"], &Valor::Entero(9_000_000_000_000_000_000));
    assert_eq!(a["T_FLOAT"], &Valor::Real(1.5));
    // El decimal exacto, como texto: ni un dígito menos.
    assert_eq!(a["T_DEC"], &Valor::Decimal("12345.678".into()));
    assert_eq!(
        a["T_DECSIN"],
        &Valor::Decimal("3.14159265358979323846".into())
    );
    assert_eq!(a["T_BOOL"], &Valor::Bool(true));
    assert_eq!(a["T_VAR"], &Valor::Texto("héllo ñ 日本".into()));
    assert_eq!(a["T_BIN"], &Valor::Bytes(b"ab\0\0".to_vec()));
    // Lo que se pierde en el cable: DATE en días, TIMESTAMP en milisegundos (0071 §3.4).
    assert_eq!(a["T_DATE"], &Valor::Entero(20746));
    assert_eq!(a["T_TIME"], &Valor::Entero(49_559_000));
    assert_eq!(a["T_TS"], &Valor::Entero(1_792_503_959_123));
    assert_eq!(a["T_UTS"], &Valor::Entero(1_792_503_959_500));
    assert_eq!(
        a["T_AINT"],
        &Valor::Lista(vec![Valor::Entero(1), Valor::Entero(2), Valor::Entero(3)])
    );
    assert_eq!(
        a["T_AVAR"],
        &Valor::Lista(vec![Valor::Texto("x".into()), Valor::Texto("y".into())])
    );

    // La fila `b`: todo nulo salvo la clave.
    let b: BTreeMap<&str, &Valor> = l
        .columnas()
        .iter()
        .map(|c| c.nombre.as_str())
        .zip(filas[1].iter())
        .collect();
    assert_eq!(b["K1"], &Valor::Texto("b".into()));
    assert_eq!(b["K2"], &Valor::Entero(2));
    for n in [
        "T_TINY", "T_INT", "T_DEC", "T_BOOL", "T_VAR", "T_BIN", "T_DATE", "T_TS", "T_AINT",
    ] {
        assert_eq!(b[n], &Valor::Nulo, "{n}");
    }
    assert!(l.siguiente_frame().unwrap().is_none());
}

#[test]
fn el_catalogo_del_pqs_real() {
    let s = reproducir("catalogo");
    let c = conectar(&s);
    let esquemas = c.esquemas(None).unwrap();
    let i = esquemas.indice("TABLE_SCHEM").unwrap();
    let nombres: Vec<String> = esquemas
        .filas
        .iter()
        .filter_map(|f| {
            if let Valor::Texto(t) = &f[i] {
                Some(t.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(
        nombres.contains(&"ORE_X0".to_string()) && nombres.contains(&"SYSTEM".to_string()),
        "{nombres:?}"
    );

    let tablas = c.tablas(Some("ORE_X0"), None, &[]).unwrap();
    let (n, t) = (
        tablas.indice("TABLE_NAME").unwrap(),
        tablas.indice("TABLE_TYPE").unwrap(),
    );
    let por_nombre: BTreeMap<String, String> = tablas
        .filas
        .iter()
        .map(|f| {
            let txt = |v: &Valor| {
                if let Valor::Texto(s) = v {
                    s.clone()
                } else {
                    String::new()
                }
            };
            (txt(&f[n]), txt(&f[t]))
        })
        .collect();
    assert_eq!(por_nombre["PEDIDOS"], "TABLE");
    assert_eq!(por_nombre["PEDIDOS_CERRADOS"], "VIEW");
    assert!(tablas.indice("SALT_BUCKETS").is_some() && tablas.indice("MULTI_TENANT").is_some());

    let columnas = c.columnas(Some("ORE_X0"), Some("TIPOS")).unwrap();
    assert_eq!(columnas.filas.len(), 23);
    let (nc, tn) = (
        columnas.indice("COLUMN_NAME").unwrap(),
        columnas.indice("TYPE_NAME").unwrap(),
    );
    assert_eq!(columnas.filas[0][nc], Valor::Texto("K1".into()));
    assert_eq!(columnas.filas[0][tn], Valor::Texto("VARCHAR".into()));
    assert!(columnas.indice("KEY_SEQ").is_some());
}

#[test]
fn una_tabla_que_no_existe_es_un_error_del_servidor_con_su_codigo() {
    let s = reproducir("error_tabla");
    let c = conectar(&s);
    let Err(e) = c.ejecutar("SELECT * FROM no_existe") else {
        panic!("tenía que fallar")
    };
    let Error::Servidor {
        codigo,
        sql_state,
        mensaje,
        excepciones,
    } = &e
    else {
        panic!("{e:?}")
    };
    // El PQS entrega **todo** error SQL con código −1 y estado `00000`: el de Phoenix (`ERROR 1012
    // (42M03)`) va dentro del texto, y lo extrae `ore-phoenix` (M2). Medido contra los bytes reales.
    assert_eq!((*codigo, sql_state.as_str()), (u32::MAX, "00000"));
    assert!(
        mensaje.contains("ERROR 1012 (42M03): Table undefined. tableName=NO_EXISTE"),
        "{mensaje}"
    );
    assert!(!excepciones.is_empty(), "el PQS manda la pila de Java");
    assert!(!e.es_tiempo_limite());
    // El statement que se creó se cierra aunque el SELECT fallara.
    assert!(s.nombres().contains(&"CloseStatementRequest".to_string()));
}

#[test]
fn un_statement_perdido_es_un_error_y_no_cero_filas() {
    let s = reproducir("statement_perdido");
    let c = conectar(&s);
    let mut l = c
        .ejecutar("SELECT id FROM ore_x0.pedidos ORDER BY id")
        .unwrap();
    assert_eq!(l.siguiente_frame().unwrap().unwrap().len(), 3);
    // La traza trae la respuesta real de un `Fetch` tras cerrar el statement: `missing_statement`.
    assert_eq!(l.siguiente_frame().unwrap_err(), Error::StatementPerdido);
    // Y la lectura no sigue: un error no se convierte en un final.
    assert_eq!(l.siguiente_frame().unwrap(), None);
}

#[test]
fn el_tiempo_limite_del_servidor_se_reconoce_por_el_texto() {
    let s = reproducir("tiempo_limite");
    let mut o = Opciones::nuevas(s.url());
    o.propiedades
        .insert("phoenix.query.timeoutMs".into(), "300".into());
    let c = Conexion::abrir(&o).unwrap();
    let Err(e) = c.ejecutar(
        "SELECT COUNT(*) FROM ore_x0.pedidos a, ore_x0.pedidos b WHERE a.importe = b.importe",
    ) else {
        panic!()
    };
    // Sin código (−1) ni estado (`00000`); dentro de una unión el texto es `TimeoutException` a secas.
    let Error::Servidor {
        codigo, sql_state, ..
    } = &e
    else {
        panic!("{e:?}")
    };
    assert_eq!((*codigo, sql_state.as_str()), (u32::MAX, "00000"));
    assert!(e.es_tiempo_limite());
    // Las propiedades de la conexión llegaron al servidor.
    let abrir: Vec<ore_avatica::proto::OpenConnectionRequest> = s.de("OpenConnectionRequest");
    assert_eq!(
        abrir[0]
            .info
            .get("phoenix.query.timeoutMs")
            .map(String::as_str),
        Some("300")
    );
}
