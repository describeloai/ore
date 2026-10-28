//! Las pruebas del transporte: el vigilante con cada forma de error de ClickHouse (sin red), y el cliente contra un servidor
//! falso local (para saber qué hace `ureq` con un *chunked* cortado, que es lo que decide si el diseño se sostiene).

use super::*;
use std::collections::VecDeque;
use std::io::Write;
use std::net::TcpListener;
use std::sync::Mutex;

// ───────────────────────────── ayudas ─────────────────────────────

/// Un lector que entrega los trozos tal cual (uno por lectura, o partido si `out` es menor) y termina con `final_`.
struct Trozos {
    trozos: VecDeque<Vec<u8>>,
    final_: Option<io::Error>,
}

impl Trozos {
    fn nuevo(trozos: Vec<Vec<u8>>, final_: Option<io::Error>) -> Trozos {
        Trozos {
            trozos: trozos.into(),
            final_,
        }
    }
}

impl Read for Trozos {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        match self.trozos.pop_front() {
            Some(mut t) => {
                let n = t.len().min(out.len());
                out[..n].copy_from_slice(&t[..n]);
                if n < t.len() {
                    self.trozos.push_front(t.split_off(n));
                }
                Ok(n)
            }
            None => match self.final_.take() {
                Some(e) => Err(e),
                None => Ok(0),
            },
        }
    }
}

fn en_trozos(datos: &[u8], n: usize) -> Vec<Vec<u8>> {
    datos.chunks(n).map(<[u8]>::to_vec).collect()
}

/// Todo lo que entrega un lector, y el error con que termina si lo hay.
fn leer_todo<R: Read>(mut r: R) -> (Vec<u8>, Option<io::Error>) {
    let mut v = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match r.read(&mut buf) {
            Ok(0) => return (v, None),
            Ok(n) => v.extend_from_slice(&buf[..n]),
            Err(e) => return (v, Some(e)),
        }
    }
}

/// Bytes sin estructura (no contienen la marca): una congruencia lineal.
fn datos(n: usize) -> Vec<u8> {
    let mut x: u32 = 12345;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 24) as u8
        })
        .collect()
}

fn servidor_de(e: &io::Error) -> ErrorServidor {
    e.get_ref()
        .and_then(|x| x.downcast_ref::<ErrorServidor>())
        .cloned()
        .unwrap_or_else(|| panic!("no es un error del servidor: {e:?}"))
}

const ETIQ: &[u8] = b"jpbnqooquidsxfsa";

fn bloque_26_4() -> Vec<u8> {
    [
        &b"__exception__\r\n"[..],
        ETIQ,
        // Capturado de un 26.4: el mensaje acaba en `\n` (no en `\r\n`) y sigue `<longitud> <ETIQUETA>\r\n__exception__\r\n`.
        b"\r\nCode: 395. DB::Exception: boom a mitad: while executing 'FUNCTION throwIf'. (FUNCTION_THROW_IF_VALUE_IS_NON_ZERO) (version 26.4.5.143 (official build))\n287 ",
        ETIQ,
        b"\r\n__exception__\r\n",
    ]
    .concat()
}

fn bloque_25_8() -> Vec<u8> {
    b"__exception__\r\nCode: 395. DB::Exception: boom a mitad. (FUNCTION_THROW_IF_VALUE_IS_NON_ZERO) (version 25.8.33.6 (official build))\n".to_vec()
}

fn texto_24_8() -> Vec<u8> {
    b"Code: 395. DB::Exception: boom a mitad: while executing 'FUNCTION throwIf'. (FUNCTION_THROW_IF_VALUE_IS_NON_ZERO) (version 24.8.14.39 (official build))\n".to_vec()
}

// ───────────────────────────── el vigilante ─────────────────────────────

#[test]
fn un_flujo_limpio_pasa_entero_con_cualquier_troceado() {
    let d = datos(300_000);
    for n in [7, 100, 4096, 70_000, 300_000] {
        let (v, e) = leer_todo(Vigia::nuevo(Trozos::nuevo(en_trozos(&d, n), None), None));
        assert!(e.is_none(), "troceado {n}: {e:?}");
        assert!(v == d, "troceado {n}: el flujo cambió");
    }
    // Con etiqueta también.
    let (v, e) = leer_todo(Vigia::nuevo(
        Trozos::nuevo(en_trozos(&d, 999), None),
        Some(ETIQ.to_vec()),
    ));
    assert!(e.is_none() && v == d);
}

#[test]
fn un_flujo_pequeno_a_un_byte_por_lectura_pasa_entero() {
    let d = datos(3000);
    let (v, e) = leer_todo(Vigia::nuevo(Trozos::nuevo(en_trozos(&d, 1), None), None));
    assert!(e.is_none() && v == d);
}

#[test]
fn un_flujo_vacio_es_vacio() {
    let (v, e) = leer_todo(Vigia::nuevo(Trozos::nuevo(vec![], None), None));
    assert!(v.is_empty() && e.is_none());
}

#[test]
fn error_26_4_con_etiqueta_entrega_los_datos_y_luego_el_error() {
    let prefijo = datos(100_000);
    let flujo = [prefijo.clone(), bloque_26_4()].concat();
    for n in [1000, 4096, 16 * 1024, 200_000] {
        let (v, e) = leer_todo(Vigia::nuevo(
            Trozos::nuevo(
                en_trozos(&flujo, n),
                Some(io::ErrorKind::UnexpectedEof.into()),
            ),
            Some(ETIQ.to_vec()),
        ));
        assert!(
            v == prefijo,
            "troceado {n}: los datos de antes del error no llegaron enteros"
        );
        let s = servidor_de(&e.expect("tiene que haber error"));
        assert_eq!(s.codigo, 395, "{s:?}");
        assert!(
            s.mensaje
                .starts_with("Code: 395. DB::Exception: boom a mitad"),
            "{}",
            s.mensaje
        );
        assert!(
            !s.mensaje.contains("__exception__")
                && !s.mensaje.contains("287")
                && !s.mensaje.contains("jpbn"),
            "el marco no se quita: {}",
            s.mensaje
        );
    }
}

#[test]
fn la_marca_partida_en_cualquier_punto_se_encuentra() {
    let prefijo = datos(40_000);
    let flujo = [prefijo.clone(), bloque_26_4()].concat();
    let p = prefijo.len();
    // Corta el flujo en dos en cada punto desde antes de la marca hasta dentro del mensaje.
    for corte in (p - 3)..(p + 60) {
        let trozos = vec![flujo[..corte].to_vec(), flujo[corte..].to_vec()];
        let (v, e) = leer_todo(Vigia::nuevo(
            Trozos::nuevo(trozos, None),
            Some(ETIQ.to_vec()),
        ));
        assert!(v == prefijo, "corte {corte}: se perdieron o sobraron datos");
        assert_eq!(servidor_de(&e.expect("error")).codigo, 395, "corte {corte}");
    }
}

#[test]
fn error_25_x_sin_etiqueta_y_corte_posterior_es_el_error_del_servidor() {
    let prefijo = datos(80_000);
    let flujo = [prefijo.clone(), bloque_25_8()].concat();
    // El *chunked* sin cerrar que sigue al error (lo que hace 25.x) no tapa el error del servidor.
    let (v, e) = leer_todo(Vigia::nuevo(
        Trozos::nuevo(
            en_trozos(&flujo, 5000),
            Some(io::ErrorKind::UnexpectedEof.into()),
        ),
        None,
    ));
    assert!(v == prefijo);
    let s = servidor_de(&e.expect("error"));
    assert_eq!(s.codigo, 395);
    assert!(!s.mensaje.contains("__exception__"), "{}", s.mensaje);
}

#[test]
fn mensaje_pegado_de_24_8_sin_marca_ni_corte() {
    let prefijo = datos(80_000);
    let flujo = [prefijo.clone(), texto_24_8()].concat();
    for n in [100, 5000, 1 << 20] {
        let (v, e) = leer_todo(Vigia::nuevo(
            Trozos::nuevo(en_trozos(&flujo, n), None),
            None,
        ));
        assert!(v == prefijo, "troceado {n}");
        assert_eq!(servidor_de(&e.expect("error")).codigo, 395, "troceado {n}");
    }
}

#[test]
fn un_corte_sin_marca_es_un_corte_y_no_un_error_del_servidor() {
    let prefijo = datos(50_000);
    let (v, e) = leer_todo(Vigia::nuevo(
        Trozos::nuevo(
            en_trozos(&prefijo, 4096),
            Some(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "chunk cortado",
            )),
        ),
        None,
    ));
    assert!(v == prefijo, "lo recibido antes del corte se entrega");
    let e = e.expect("el corte llega");
    assert!(
        e.get_ref()
            .and_then(|x| x.downcast_ref::<ErrorServidor>())
            .is_none()
    );
    assert_eq!(fallo_de_io(&e).codigo, Codigo::Conexion);
}

#[test]
fn la_marca_sin_codigo_no_es_un_error_sin_etiqueta() {
    let flujo = [
        datos(1000),
        b"__exception__\r\nhola, soy un dato".to_vec(),
        datos(1000),
    ]
    .concat();
    let (v, e) = leer_todo(Vigia::nuevo(
        Trozos::nuevo(en_trozos(&flujo, 333), None),
        None,
    ));
    assert!(e.is_none() && v == flujo);
}

#[test]
fn con_etiqueta_la_marca_de_otra_etiqueta_es_un_dato() {
    let falso = [
        &b"__exception__\r\n"[..],
        b"otraetiquetaxxxx",
        b"\r\nCode: 1. DB::Exception: x\r\n",
    ]
    .concat();
    let flujo = [datos(500), falso, datos(500)].concat();
    let (v, e) = leer_todo(Vigia::nuevo(
        Trozos::nuevo(en_trozos(&flujo, 64), None),
        Some(ETIQ.to_vec()),
    ));
    assert!(e.is_none() && v == flujo);
}

#[test]
fn un_dato_que_acaba_en_texto_de_codigo_sin_version_pasa() {
    let flujo = [
        datos(500),
        b"Code: 7. DB::Exception: esto es un dato\n".to_vec(),
    ]
    .concat();
    let (v, e) = leer_todo(Vigia::nuevo(
        Trozos::nuevo(en_trozos(&flujo, 64), None),
        None,
    ));
    assert!(e.is_none() && v == flujo);
}

// ───────────────────────────── los códigos ─────────────────────────────

#[test]
fn cada_codigo_de_clickhouse_va_al_codigo_del_protocolo() {
    let c = |n| fallo_de_codigo(n, String::new()).codigo;
    assert_eq!(c(516), Codigo::Credencial);
    assert_eq!(c(497), Codigo::Credencial);
    assert_eq!(c(60), Codigo::Objeto);
    assert_eq!(c(81), Codigo::Objeto);
    assert_eq!(c(47), Codigo::Esquema);
    assert_eq!(c(159), Codigo::Tiempo);
    assert_eq!(c(62), Codigo::Operador);
    assert_eq!(c(457), Codigo::Operador);
    assert_eq!(c(115), Codigo::Operador);
    assert_eq!(c(241), Codigo::Origen);
    assert_eq!(c(0), Codigo::Origen);
}

#[test]
fn el_codigo_se_lee_del_texto() {
    assert_eq!(
        ErrorServidor::de_texto("Code: 60. DB::Exception: Unknown table").codigo,
        60
    );
    assert_eq!(ErrorServidor::de_texto("sin código").codigo, 0);
    assert_eq!(
        ErrorServidor::de_texto("  Code: 516. DB::Exception: x \n").mensaje,
        "Code: 516. DB::Exception: x"
    );
}

#[test]
fn base64_de_los_vectores_conocidos() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"ana:s"), "YW5hOnM=");
}

#[test]
fn con_clave_la_autorizacion_es_basic_y_sin_clave_no_hay() {
    let c = Cliente::abrir("clickhouse://ana:s@h.ejemplo.com?tls=true").unwrap();
    assert_eq!(c.autorizacion.as_deref(), Some("Basic YW5hOnM="));
    let c = Cliente::abrir("clickhouse://kit@localhost").unwrap();
    assert!(c.autorizacion.is_none());
    // Una URL que no se admite es un fallo del que pidió, con el motivo.
    let f = Cliente::abrir("clickhouse://ana:secreta@h")
        .err()
        .expect("falla");
    assert_eq!(f.codigo, Codigo::Operador);
    assert!(!f.mensaje.contains("secreta"));
}

// ───────────────────────────── el cliente contra un servidor falso ─────────────────────────────

/// Un servidor que contesta, una conexión por respuesta y en orden, lo que se le dé, y apunta lo que recibió.
fn servidor(respuestas: Vec<Vec<u8>>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let puerto = l.local_addr().unwrap().port();
    let recibido = Arc::new(Mutex::new(Vec::new()));
    let apuntar = recibido.clone();
    std::thread::spawn(move || {
        for r in respuestas {
            let (mut s, _) = match l.accept() {
                Ok(x) => x,
                Err(_) => return,
            };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            // Cabecera.
            let fin_cabecera = loop {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break buf.len();
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = buscar(&buf, b"\r\n\r\n") {
                    break i + 4;
                }
            };
            // Cuerpo, por `Content-Length`.
            let cabecera = String::from_utf8_lossy(&buf[..fin_cabecera.min(buf.len())]).to_string();
            let largo = cabecera
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            while buf.len() < fin_cabecera + largo {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            apuntar
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf).to_string());
            let _ = s.write_all(&r);
            let _ = s.flush();
            // Se cierra aquí: sin `0\r\n\r\n` si la respuesta no lo trae, que es justo lo que se quiere ver.
        }
    });
    (puerto, recibido)
}

fn cliente(puerto: u16) -> Cliente {
    Cliente::abrir(&format!("clickhouse://kit@127.0.0.1:{puerto}/kit")).unwrap()
}

fn trozo(datos: &[u8]) -> Vec<u8> {
    [format!("{:x}\r\n", datos.len()).as_bytes(), datos, b"\r\n"].concat()
}

const CAB_CHUNKED: &str =
    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/octet-stream\r\n";

#[test]
fn un_chunked_completo_se_lee() {
    let r = [
        CAB_CHUNKED.as_bytes(),
        b"\r\n",
        &trozo(b"hello"),
        b"0\r\n\r\n",
    ]
    .concat();
    let (p, _) = servidor(vec![r]);
    assert_eq!(cliente(p).texto("SELECT 1").unwrap(), "hello");
}

#[test]
fn un_chunked_cortado_sin_terminador_es_un_error_y_no_un_eof_silencioso() {
    // Lo que 25.x y 26.x hacen tras un error a mitad: cierran sin el `0\r\n\r\n`. Si `ureq` lo devolviera como un fin normal,
    // el vigilante no tendría con qué distinguirlo de una lectura completa.
    let r = [CAB_CHUNKED.as_bytes(), b"\r\n", &trozo(b"hello")].concat();
    let (p, _) = servidor(vec![r]);
    let e = cliente(p)
        .texto("SELECT 1")
        .expect_err("un cuerpo cortado tiene que fallar");
    assert_eq!(e.codigo, Codigo::Conexion, "{e:?}");
}

#[test]
fn error_a_mitad_26_4_llega_con_los_datos_de_antes_y_el_codigo() {
    let prefijo = datos(50_000);
    let cuerpo = [prefijo.clone(), bloque_26_4()].concat();
    let r = [
        CAB_CHUNKED.as_bytes(),
        b"X-ClickHouse-Exception-Tag: jpbnqooquidsxfsa\r\n\r\n",
        &trozo(&cuerpo),
        // sin `0\r\n\r\n`: cortado
    ]
    .concat();
    let (p, _) = servidor(vec![r]);
    let mut f = cliente(p)
        .flujo("SELECT 1", "RowBinary", &[], &[], None)
        .unwrap();
    let (v, e) = leer_todo(&mut f);
    assert!(v == prefijo, "los datos de antes del error llegan enteros");
    let fallo = fallo_de_io(&e.expect("error"));
    assert_eq!(fallo.codigo, Codigo::Origen);
    assert!(fallo.mensaje.contains("Code: 395"), "{}", fallo.mensaje);
}

#[test]
fn error_a_mitad_25_x_sin_etiqueta_y_cortado() {
    let prefijo = datos(50_000);
    let r = [
        CAB_CHUNKED.as_bytes(),
        b"\r\n",
        &trozo(&[prefijo.clone(), bloque_25_8()].concat()),
    ]
    .concat();
    let (p, _) = servidor(vec![r]);
    let mut f = cliente(p)
        .flujo("SELECT 1", "RowBinary", &[], &[], None)
        .unwrap();
    let (v, e) = leer_todo(&mut f);
    assert!(v == prefijo);
    assert!(
        fallo_de_io(&e.expect("error"))
            .mensaje
            .contains("Code: 395")
    );
}

#[test]
fn error_a_mitad_24_8_con_el_chunked_cerrado_limpio() {
    let prefijo = datos(50_000);
    let r = [
        CAB_CHUNKED.as_bytes(),
        b"\r\n",
        &trozo(&[prefijo.clone(), texto_24_8()].concat()),
        b"0\r\n\r\n",
    ]
    .concat();
    let (p, _) = servidor(vec![r]);
    let mut f = cliente(p)
        .flujo("SELECT 1", "RowBinary", &[], &[], None)
        .unwrap();
    let (v, e) = leer_todo(&mut f);
    assert!(v == prefijo);
    assert!(
        fallo_de_io(&e.expect("error"))
            .mensaje
            .contains("Code: 395")
    );
}

#[test]
fn un_error_antes_de_empezar_se_traduce_por_su_codigo() {
    let t = "Code: 60. DB::Exception: Unknown table expression identifier 'kit.nada'. (UNKNOWN_TABLE) (version 25.8.33.6)";
    let r404 = format!(
        "HTTP/1.1 404 Not Found\r\nX-ClickHouse-Exception-Code: 60\r\nContent-Length: {}\r\n\r\n{t}",
        t.len()
    );
    let t2 = "Code: 516. DB::Exception: kit: Authentication failed. (AUTHENTICATION_FAILED)";
    let r403 = format!(
        "HTTP/1.1 403 Forbidden\r\nX-ClickHouse-Exception-Code: 516\r\nContent-Length: {}\r\n\r\n{t2}",
        t2.len()
    );
    let (p, _) = servidor(vec![r404.into_bytes(), r403.into_bytes()]);
    let f = cliente(p).texto("SELECT * FROM nada").expect_err("404");
    assert_eq!(f.codigo, Codigo::Objeto, "{f:?}");
    assert!(f.mensaje.contains("Unknown table"));
    let f = cliente(p).texto("SELECT 1").expect_err("403");
    assert_eq!(f.codigo, Codigo::Credencial, "{f:?}");
}

#[test]
fn una_conexion_rechazada_es_conexion_y_reintentable() {
    let p = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }; // cerrado: nadie escucha
    let f = cliente(p).texto("SELECT 1").expect_err("rechazada");
    assert_eq!(f.codigo, Codigo::Conexion, "{f:?}");
    assert!(f.reintentable);
    assert!(f.mensaje.contains(&p.to_string()));
}

#[test]
fn la_peticion_lleva_la_base_el_formato_los_parametros_y_el_usuario() {
    let r = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
        .as_bytes()
        .to_vec();
    let (p, recibido) = servidor(vec![r]);
    let c = cliente(p);
    let mut f = c
        .flujo(
            "SELECT {x:String}",
            "RowBinaryWithNamesAndTypes",
            &[("x", Param::Texto("a b&c=\\N"))],
            &[("max_threads", "2")],
            Some("consulta-1"),
        )
        .unwrap();
    let _ = leer_todo(&mut f);
    let req = recibido.lock().unwrap()[0].clone();
    let primera = req.lines().next().unwrap().to_string();
    assert!(primera.starts_with("POST /?"), "{primera}");
    for esperado in [
        "database=kit",
        "default_format=RowBinaryWithNamesAndTypes",
        "query_id=consulta-1",
        "max_threads=2",
    ] {
        assert!(
            primera.contains(esperado),
            "falta `{esperado}` en `{primera}`"
        );
    }
    // El valor del parámetro va codificado: ni `&` ni `=` rompen la consulta. `ureq` escribe el espacio como `+`, y las
    // cuatro versiones del laboratorio (24.8, 25.3, 25.8, 26.4) lo leen como espacio; `%20` valdría igual.
    assert!(
        primera.contains("param_x=a+b%26c%3D%5C%5CN")
            || primera.contains("param_x=a%20b%26c%3D%5C%5CN"),
        "{primera}"
    );
    let bajo = req.to_ascii_lowercase();
    assert!(bajo.contains("x-clickhouse-user: kit"), "{req}");
    assert!(
        !bajo.contains("authorization:"),
        "sin clave no hay Authorization: {req}"
    );
    assert!(
        req.ends_with("SELECT {x:String}"),
        "el SQL va en el cuerpo: {req}"
    );
}

#[test]
fn los_parametros_se_escapan_como_texto_del_servidor() {
    assert_eq!(escapar_texto("a"), "a");
    // La barra se duplica, así que `\N` (dos caracteres) no se confunde con el nulo.
    assert_eq!(escapar_texto("\\N"), "\\\\N");
    assert_eq!(escapar_texto("a\tb\nc\rd\0e"), "a\\tb\\nc\\rd\\0e");
    assert_eq!(escapar_texto("日本 'x' \"y\""), "日本 'x' \"y\"");
    assert_eq!(Param::Nulo.en_el_cable(), "\\N");
    assert_eq!(Param::Texto("").en_el_cable(), "");
}
