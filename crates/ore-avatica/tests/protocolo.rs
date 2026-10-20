//! Lo que el cliente hace cuando el servidor **no** se porta como en el caso feliz: los cuatro invariantes de
//! la ficha 0071 §3.3, y los fallos de red, de HTTP y de credenciales. Mensajes fabricados con `prost`.

mod comun;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use comun::{Recibida, Respuesta, Servidor};
use ore_avatica::proto::*;
use ore_avatica::{Conexion, Error, Opciones, Valor};

fn fila(n: i64) -> Row {
    Row {
        value: vec![ColumnValue {
            scalar_value: Some(TypedValue {
                r#type: Rep::Long as i32,
                number_value: n,
                ..Default::default()
            }),
            ..Default::default()
        }],
    }
}

fn frame(desde: i64, cuantas: i64, done: bool) -> Frame {
    Frame {
        offset: 0,
        done,
        rows: (desde..desde + cuantas).map(fila).collect(),
    }
}

fn firma() -> Signature {
    Signature {
        columns: vec![ColumnMetaData {
            column_name: "N".into(),
            r#type: Some(AvaticaType {
                id: -5i32 as u32,
                name: "BIGINT".into(),
                rep: Rep::Long as i32,
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Un PQS de mentira cuyos `Fetch` devuelven, en orden, `siguientes`; el primer frame es `primero`.
fn con_frames(primero: Frame, siguientes: Vec<Frame>) -> Servidor {
    let cola = Mutex::new(siguientes.into_iter());
    Servidor::arrancar(move |r: &Recibida| {
        Some(match r.nombre.as_str() {
            "OpenConnectionRequest" => Respuesta::avatica(
                200,
                "OpenConnectionResponse",
                &OpenConnectionResponse::default(),
            ),
            "CreateStatementRequest" => Respuesta::avatica(
                200,
                "CreateStatementResponse",
                &CreateStatementResponse {
                    statement_id: 7,
                    ..Default::default()
                },
            ),
            "PrepareAndExecuteRequest" => Respuesta::avatica(
                200,
                "ExecuteResponse",
                &ExecuteResponse {
                    results: vec![ResultSetResponse {
                        statement_id: 7,
                        signature: Some(firma()),
                        first_frame: Some(primero.clone()),
                        update_count: u64::MAX,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ),
            "FetchRequest" => {
                let f = cola
                    .lock()
                    .unwrap()
                    .next()
                    .expect("el cliente pidió más frames de los previstos");
                Respuesta::avatica(
                    200,
                    "FetchResponse",
                    &FetchResponse {
                        frame: Some(f),
                        ..Default::default()
                    },
                )
            }
            "CloseStatementRequest" => Respuesta::avatica(
                200,
                "CloseStatementResponse",
                &CloseStatementResponse::default(),
            ),
            "CloseConnectionRequest" => Respuesta::avatica(
                200,
                "CloseConnectionResponse",
                &CloseConnectionResponse::default(),
            ),
            otro => panic!("petición inesperada: {otro}"),
        })
    })
}

fn conectar(s: &Servidor) -> Conexion {
    Conexion::abrir(&Opciones::nuevas(s.url())).expect("abrir")
}

fn contar(s: &Servidor, nombre: &str) -> usize {
    s.nombres().iter().filter(|n| *n == nombre).count()
}

fn enteros(filas: &[Vec<Valor>]) -> Vec<i64> {
    filas
        .iter()
        .map(|f| {
            if let Valor::Entero(n) = f[0] {
                n
            } else {
                panic!("{f:?}")
            }
        })
        .collect()
}

// ── los cuatro invariantes ──────────────────────────────────────────────────

#[test]
fn invariante_1_el_primer_frame_se_pide_con_tamano_y_un_cero_no_se_acepta() {
    let s = con_frames(frame(1, 2, true), vec![]);
    let mut o = Opciones::nuevas(s.url());
    o.filas_por_frame = 12_345;
    let c = Conexion::abrir(&o).unwrap();
    c.ejecutar("SELECT n FROM t").unwrap();
    assert_eq!(
        s.de::<PrepareAndExecuteRequest>("PrepareAndExecuteRequest")[0].first_frame_max_size,
        12_345
    );

    // Sin tamaño, el PQS devuelve el primer frame vacío: el cliente ni lo permite.
    for malo in [0, -1] {
        let mut o = Opciones::nuevas(s.url());
        o.filas_por_frame = malo;
        assert!(
            matches!(Conexion::abrir(&o), Err(Error::Protocolo(_))),
            "{malo}"
        );
    }
}

#[test]
fn invariante_2_un_statement_perdido_no_es_un_final() {
    // En el Fetch (`missing_statement`) y en el Fetch con `missing_results`.
    for (statement, resultados) in [(true, false), (false, true)] {
        let s = Servidor::arrancar(move |r| {
            Some(match r.nombre.as_str() {
                "OpenConnectionRequest" => Respuesta::avatica(
                    200,
                    "OpenConnectionResponse",
                    &OpenConnectionResponse::default(),
                ),
                "CreateStatementRequest" => Respuesta::avatica(
                    200,
                    "CreateStatementResponse",
                    &CreateStatementResponse {
                        statement_id: 7,
                        ..Default::default()
                    },
                ),
                "PrepareAndExecuteRequest" => Respuesta::avatica(
                    200,
                    "ExecuteResponse",
                    &ExecuteResponse {
                        results: vec![ResultSetResponse {
                            statement_id: 7,
                            signature: Some(firma()),
                            first_frame: Some(frame(1, 2, false)),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                ),
                // Con las banderas y SIN filas: lo que hace un PQS con un statement que ya no existe.
                "FetchRequest" => Respuesta::avatica(
                    200,
                    "FetchResponse",
                    &FetchResponse {
                        frame: Some(Frame {
                            done: false,
                            ..Default::default()
                        }),
                        missing_statement: statement,
                        missing_results: resultados,
                        ..Default::default()
                    },
                ),
                _ => Respuesta::avatica(
                    200,
                    "CloseStatementResponse",
                    &CloseStatementResponse::default(),
                ),
            })
        });
        let c = conectar(&s);
        let mut l = c.ejecutar("SELECT n FROM t").unwrap();
        assert_eq!(l.siguiente_frame().unwrap().unwrap().len(), 2);
        assert_eq!(
            l.siguiente_frame().unwrap_err(),
            Error::StatementPerdido,
            "{statement}/{resultados}"
        );
        // No se reintenta ni se da por terminada una lectura rota.
        assert_eq!(l.siguiente_frame().unwrap(), None);
        assert_eq!(contar(&s, "FetchRequest"), 1);
    }
}

#[test]
fn invariante_2_un_execute_con_missing_statement_tambien() {
    let s = Servidor::arrancar(|r| {
        Some(match r.nombre.as_str() {
            "OpenConnectionRequest" => Respuesta::avatica(
                200,
                "OpenConnectionResponse",
                &OpenConnectionResponse::default(),
            ),
            "CreateStatementRequest" => Respuesta::avatica(
                200,
                "CreateStatementResponse",
                &CreateStatementResponse {
                    statement_id: 7,
                    ..Default::default()
                },
            ),
            "PrepareAndExecuteRequest" => Respuesta::avatica(
                200,
                "ExecuteResponse",
                &ExecuteResponse {
                    missing_statement: true,
                    ..Default::default()
                },
            ),
            _ => Respuesta::avatica(
                200,
                "CloseConnectionResponse",
                &CloseConnectionResponse::default(),
            ),
        })
    });
    assert!(matches!(
        conectar(&s).ejecutar("SELECT 1"),
        Err(Error::StatementPerdido)
    ));
}

#[test]
fn invariante_3_el_fetch_lleva_las_filas_leidas_y_nunca_se_reintenta() {
    let s = con_frames(
        frame(1, 2, false),
        vec![frame(3, 2, false), frame(5, 1, true)],
    );
    let c = conectar(&s);
    let mut l = c.ejecutar("SELECT n FROM t").unwrap();
    let mut todo = Vec::new();
    while let Some(f) = l.siguiente_frame().unwrap() {
        todo.extend(enteros(&f));
    }
    assert_eq!(todo, [1, 2, 3, 4, 5]);
    assert_eq!(l.leidas(), 5);
    let offsets: Vec<u64> = s
        .de::<FetchRequest>("FetchRequest")
        .iter()
        .map(|f| f.offset)
        .collect();
    assert_eq!(offsets, [2, 4]); // las leídas, no el offset que el servidor echó en el frame
    // El último frame trae `done`: no hay un Fetch de más.
    assert_eq!(contar(&s, "FetchRequest"), 2);
}

#[test]
fn invariante_3_un_fallo_de_red_en_un_fetch_no_se_reintenta() {
    let vistos = Arc::new(Mutex::new(0));
    let v = vistos.clone();
    let s = Servidor::arrancar(move |r| {
        Some(match r.nombre.as_str() {
            "OpenConnectionRequest" => Respuesta::avatica(
                200,
                "OpenConnectionResponse",
                &OpenConnectionResponse::default(),
            ),
            "CreateStatementRequest" => Respuesta::avatica(
                200,
                "CreateStatementResponse",
                &CreateStatementResponse {
                    statement_id: 7,
                    ..Default::default()
                },
            ),
            "PrepareAndExecuteRequest" => Respuesta::avatica(
                200,
                "ExecuteResponse",
                &ExecuteResponse {
                    results: vec![ResultSetResponse {
                        statement_id: 7,
                        signature: Some(firma()),
                        first_frame: Some(frame(1, 2, false)),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ),
            "FetchRequest" => {
                *v.lock().unwrap() += 1;
                Respuesta {
                    estado: 503,
                    cuerpo: b"<html>servicio no disponible</html>".to_vec(),
                }
            }
            _ => Respuesta::avatica(
                200,
                "CloseStatementResponse",
                &CloseStatementResponse::default(),
            ),
        })
    });
    let c = conectar(&s);
    let mut l = c.ejecutar("SELECT n FROM t").unwrap();
    l.siguiente_frame().unwrap();
    let e = l.siguiente_frame().unwrap_err();
    assert!(matches!(e, Error::Http { estado: 503, .. }), "{e:?}");
    // Un Fetch repetido no releería: el trozo ya está consumido (el servidor ignora el offset).
    assert_eq!(*vistos.lock().unwrap(), 1);
}

#[test]
fn invariante_4_ni_filas_ni_done_es_un_truncado_y_no_el_final() {
    // El cursor se queda parado tras 2 filas.
    let s = con_frames(frame(1, 2, false), vec![frame(0, 0, false)]);
    let c = conectar(&s);
    let mut l = c.ejecutar("SELECT n FROM t").unwrap();
    assert_eq!(l.siguiente_frame().unwrap().unwrap().len(), 2);
    assert_eq!(
        l.siguiente_frame().unwrap_err(),
        Error::Truncado { leidas: 2 }
    );
    drop(l);
    assert_eq!(
        contar(&s, "CloseStatementRequest"),
        1,
        "y se cierra el statement"
    );

    // El primer frame vacío y sin done: lo que da un PQS al que no se le pide tamaño.
    let s = con_frames(frame(0, 0, false), vec![]);
    let mut l = conectar(&s).ejecutar("SELECT n FROM t").unwrap();
    assert_eq!(
        l.siguiente_frame().unwrap_err(),
        Error::Truncado { leidas: 0 }
    );
}

#[test]
fn un_done_sin_filas_es_el_final_limpio_y_un_resultado_vacio_no_es_un_error() {
    let s = con_frames(frame(1, 2, false), vec![frame(0, 0, true)]);
    let mut l = conectar(&s).ejecutar("SELECT n FROM t").unwrap();
    assert_eq!(enteros(&l.siguiente_frame().unwrap().unwrap()), [1, 2]);
    assert_eq!(l.siguiente_frame().unwrap(), None);
    assert_eq!(l.siguiente_frame().unwrap(), None);

    let s = con_frames(frame(0, 0, true), vec![]);
    let mut l = conectar(&s)
        .ejecutar("SELECT n FROM t WHERE 1 = 0")
        .unwrap();
    assert_eq!(l.siguiente_frame().unwrap(), None);
    assert_eq!(l.leidas(), 0);
}

// ── limpieza ────────────────────────────────────────────────────────────────

#[test]
fn soltar_una_lectura_a_medias_cierra_su_statement_y_soltar_la_conexion_la_cierra() {
    let s = con_frames(frame(1, 2, false), vec![]);
    let c = conectar(&s);
    let mut l = c.ejecutar("SELECT n FROM t").unwrap();
    l.siguiente_frame().unwrap();
    assert_eq!(contar(&s, "CloseStatementRequest"), 0);
    drop(l);
    assert_eq!(contar(&s, "CloseStatementRequest"), 1);
    assert_eq!(
        s.de::<CloseStatementRequest>("CloseStatementRequest")[0].statement_id,
        7
    );
    drop(c);
    assert_eq!(contar(&s, "CloseConnectionRequest"), 1);
}

#[test]
fn la_conexion_sigue_abierta_mientras_viva_una_lectura() {
    let s = con_frames(frame(1, 2, false), vec![frame(3, 1, true)]);
    let c = conectar(&s);
    let mut l = c.ejecutar("SELECT n FROM t").unwrap();
    drop(c); // la lectura comparte la sesión: no se cierra por debajo
    assert_eq!(contar(&s, "CloseConnectionRequest"), 0);
    assert_eq!(enteros(&l.siguiente_frame().unwrap().unwrap()), [1, 2]);
    assert_eq!(enteros(&l.siguiente_frame().unwrap().unwrap()), [3]);
    drop(l);
    assert_eq!(contar(&s, "CloseConnectionRequest"), 1);
}

// ── lo que no es una consulta, y lo que no es Avatica ───────────────────────

#[test]
fn un_resultado_sin_firma_no_es_una_consulta() {
    let s = Servidor::arrancar(|r| {
        Some(match r.nombre.as_str() {
            "OpenConnectionRequest" => Respuesta::avatica(
                200,
                "OpenConnectionResponse",
                &OpenConnectionResponse::default(),
            ),
            "CreateStatementRequest" => Respuesta::avatica(
                200,
                "CreateStatementResponse",
                &CreateStatementResponse {
                    statement_id: 7,
                    ..Default::default()
                },
            ),
            "PrepareAndExecuteRequest" => Respuesta::avatica(
                200,
                "ExecuteResponse",
                &ExecuteResponse {
                    results: vec![ResultSetResponse {
                        statement_id: 7,
                        update_count: 1,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ),
            _ => Respuesta::avatica(
                200,
                "CloseStatementResponse",
                &CloseStatementResponse::default(),
            ),
        })
    });
    assert!(matches!(
        conectar(&s).ejecutar("UPSERT INTO t VALUES (1)"),
        Err(Error::Protocolo(_))
    ));
    // Y dos resultados tampoco: el cliente ejecuta de uno en uno.
    let s = Servidor::arrancar(|r| {
        Some(match r.nombre.as_str() {
            "OpenConnectionRequest" => Respuesta::avatica(
                200,
                "OpenConnectionResponse",
                &OpenConnectionResponse::default(),
            ),
            "CreateStatementRequest" => Respuesta::avatica(
                200,
                "CreateStatementResponse",
                &CreateStatementResponse::default(),
            ),
            "PrepareAndExecuteRequest" => Respuesta::avatica(
                200,
                "ExecuteResponse",
                &ExecuteResponse {
                    results: vec![Default::default(), Default::default()],
                    ..Default::default()
                },
            ),
            _ => Respuesta::avatica(
                200,
                "CloseStatementResponse",
                &CloseStatementResponse::default(),
            ),
        })
    });
    assert!(matches!(
        conectar(&s).ejecutar("SELECT 1"),
        Err(Error::Protocolo(_))
    ));
}

#[test]
fn una_respuesta_de_otro_tipo_es_un_error_de_protocolo() {
    let s = Servidor::arrancar(|_| {
        Some(Respuesta::avatica(
            200,
            "CloseConnectionResponse",
            &CloseConnectionResponse::default(),
        ))
    });
    let Err(e) = Conexion::abrir(&Opciones::nuevas(s.url())) else {
        panic!()
    };
    assert!(
        matches!(&e, Error::Protocolo(m) if m.contains("OpenConnectionResponse")),
        "{e:?}"
    );
}

#[test]
fn una_pagina_de_un_balanceador_es_un_error_http_y_se_recorta() {
    let s = Servidor::arrancar(|_| {
        Some(Respuesta {
            estado: 502,
            cuerpo: format!("<html>{}</html>", "mala puerta de enlace ".repeat(500)).into_bytes(),
        })
    });
    let Err(Error::Http { estado, cuerpo }) = Conexion::abrir(&Opciones::nuevas(s.url())) else {
        panic!()
    };
    assert_eq!(estado, 502);
    assert!(cuerpo.chars().count() < 600, "{}", cuerpo.len());
}

#[test]
fn un_servidor_que_no_esta_es_un_error_de_red() {
    let direccion = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    }; // el puerto queda libre
    let Err(e) = Conexion::abrir(&Opciones::nuevas(format!("http://{direccion}/"))) else {
        panic!()
    };
    assert!(matches!(e, Error::Red(_)), "{e:?}");
    assert!(!e.es_tiempo_limite());
}

#[test]
fn un_servidor_que_no_contesta_es_un_tiempo_agotado() {
    let s = Servidor::arrancar(|_| None);
    let mut o = Opciones::nuevas(s.url());
    o.tiempo_peticion = Duration::from_millis(300);
    let t = Instant::now();
    let Err(e) = Conexion::abrir(&o) else {
        panic!()
    };
    assert!(matches!(e, Error::TiempoRed(_)), "{e:?}");
    assert!(e.es_tiempo_limite());
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "esperó {:?}",
        t.elapsed()
    );
}

// ── credenciales ────────────────────────────────────────────────────────────

#[test]
fn la_credencial_va_en_cada_peticion_y_en_ningun_mensaje() {
    const CLAVE: &str = "clave-X7q-secreta";
    let s = Servidor::arrancar(|r| {
        // Un servidor que exige la credencial; si no, 401 con una página que no la repite.
        if r.autorizacion.as_deref() != Some("Basic dXN1YXJpbzpjbGF2ZS1YN3Etc2VjcmV0YQ==") {
            return Some(Respuesta {
                estado: 401,
                cuerpo: b"<html>no autorizado</html>".to_vec(),
            });
        }
        Some(match r.nombre.as_str() {
            "OpenConnectionRequest" => Respuesta::avatica(
                200,
                "OpenConnectionResponse",
                &OpenConnectionResponse::default(),
            ),
            _ => Respuesta::avatica(
                200,
                "CloseConnectionResponse",
                &CloseConnectionResponse::default(),
            ),
        })
    });
    // Sin ella: 401, y el mensaje no la tiene.
    let Err(e) = Conexion::abrir(&Opciones::nuevas(s.url())) else {
        panic!()
    };
    assert!(matches!(e, Error::Http { estado: 401, .. }), "{e:?}");

    // Con ella: entra. `usuario:clave-X7q-secreta` en base64 es lo que el servidor espera.
    let mut o = Opciones::nuevas(s.url());
    o.credencial = Some(("usuario".into(), CLAVE.into()));
    let c = Conexion::abrir(&o).expect("con la credencial entra");
    drop(c);
    assert!(
        s.recibidas
            .lock()
            .unwrap()
            .iter()
            .skip(1)
            .all(|r| r.autorizacion.is_some()),
        "también en el cierre"
    );

    // Y con una equivocada, el error no repite ni la correcta ni la mala.
    o.credencial = Some(("usuario".into(), "otra-clave-9".into()));
    let Err(e) = Conexion::abrir(&o) else {
        panic!()
    };
    for texto in [e.to_string(), format!("{e:?}"), format!("{o:?}")] {
        assert!(
            !texto.contains(CLAVE) && !texto.contains("otra-clave-9"),
            "{texto}"
        );
    }
}

// ── metadatos paginados ─────────────────────────────────────────────────────

#[test]
fn un_metadato_con_varios_frames_se_lee_entero() {
    let s = Servidor::arrancar({
        let cola = Mutex::new(vec![frame(3, 2, true)].into_iter());
        move |r| {
            Some(match r.nombre.as_str() {
                "OpenConnectionRequest" => Respuesta::avatica(
                    200,
                    "OpenConnectionResponse",
                    &OpenConnectionResponse::default(),
                ),
                "TablesRequest" => Respuesta::avatica(
                    200,
                    "ResultSetResponse",
                    &ResultSetResponse {
                        statement_id: 3,
                        signature: Some(firma()),
                        first_frame: Some(frame(1, 2, false)),
                        ..Default::default()
                    },
                ),
                "FetchRequest" => Respuesta::avatica(
                    200,
                    "FetchResponse",
                    &FetchResponse {
                        frame: cola.lock().unwrap().next(),
                        ..Default::default()
                    },
                ),
                _ => Respuesta::avatica(
                    200,
                    "CloseConnectionResponse",
                    &CloseConnectionResponse::default(),
                ),
            })
        }
    });
    let t = conectar(&s)
        .tablas(Some("ORE_X0"), None, &["TABLE", "VIEW"])
        .unwrap();
    assert_eq!(enteros(&t.filas), [1, 2, 3, 4]);
    let pedida: Vec<TablesRequest> = s.de("TablesRequest");
    assert!(pedida[0].has_schema_pattern && pedida[0].schema_pattern == "ORE_X0");
    assert!(!pedida[0].has_table_name_pattern && pedida[0].has_type_list);
    assert_eq!(pedida[0].type_list, ["TABLE", "VIEW"]);
}

#[test]
fn el_identificador_de_conexion_es_distinto_en_cada_conexion() {
    let s = con_frames(frame(1, 1, true), vec![]);
    let _a = conectar(&s);
    let _b = conectar(&s);
    let ids: Vec<String> = s
        .de::<OpenConnectionRequest>("OpenConnectionRequest")
        .into_iter()
        .map(|o| o.connection_id)
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert!(ids.iter().all(|i| !i.is_empty()));
}

#[test]
fn que_errores_permiten_empezar_de_nuevo() {
    let servidor = |m: &str| Error::Servidor {
        codigo: u32::MAX,
        sql_state: "00000".into(),
        mensaje: m.into(),
        excepciones: vec![],
    };
    for (e, si) in [
        (Error::Red("x".into()), true),
        (Error::TiempoRed("x".into()), true),
        (Error::StatementPerdido, true),
        (
            Error::Http {
                estado: 502,
                cuerpo: String::new(),
            },
            true,
        ),
        (
            Error::Http {
                estado: 503,
                cuerpo: String::new(),
            },
            true,
        ),
        (
            Error::Http {
                estado: 504,
                cuerpo: String::new(),
            },
            true,
        ),
        (
            Error::Http {
                estado: 401,
                cuerpo: String::new(),
            },
            false,
        ),
        (
            Error::Http {
                estado: 404,
                cuerpo: String::new(),
            },
            false,
        ),
        // Lo que el servidor contestó, volvería a contestarlo: ni un error SQL ni el tiempo límite de Phoenix.
        (servidor("ERROR 1012 (42M03): Table undefined"), false),
        (servidor("TimeoutException"), false),
        (Error::Protocolo("x".into()), false),
        (Error::Truncado { leidas: 3 }, false),
    ] {
        assert_eq!(e.reintentable(), si, "{e:?}");
    }
}
