//! **El TLS del cliente, contra un servidor de verdad** (ficha 0071, §3.14). **No necesita Phoenix**: lo que importa
//! es si el cliente ACEPTA el certificado, y qué le manda al servidor cuando no lo acepta.
//!
//! Un servidor TLS **en el propio proceso** (`SslAcceptor`) presenta el certificado que cada caso quiera, y los
//! certificados los genera el test con el crate `openssl`: una CA en la que el cliente confía (`Opciones::
//! confiar_ademas`, que es lo único que añade confianza y nunca quita verificación), otra en la que no, un nombre
//! que no es el de la URL, un certificado caducado, un servidor que no habla TLS y uno que solo habla TLS 1.1.
//! Cada caso afirma **lo que hizo el servidor** —si el *handshake* terminó— y **lo que recibió**: si el cliente
//! rechaza el certificado, la cabecera `Authorization` no ha salido, y en ningún caso aparece la contraseña en un
//! error.
//!
//! Corre en el CI y en cualquier máquina (con el SChannel de Windows o con OpenSSL).

mod comun;

use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::ssl::{Ssl, SslContext, SslContextBuilder, SslMethod, SslVersion};
use openssl::x509::extension::{BasicConstraints, KeyUsage, SubjectAlternativeName};
use openssl::x509::{X509, X509NameBuilder};

use comun::{Recibida, Respuesta};
use ore_avatica::proto::OpenConnectionResponse;
use ore_avatica::{Conexion, Error, Opciones};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// La contraseña del cliente: no puede aparecer en NINGÚN error, y solo llega al servidor tras un *handshake* bueno.
const CLAVE: &str = "CLAVE-SECRETA-m6";
const USUARIO: &str = "ore";

// ── Certificados ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Par {
    cert: X509,
    clave: PKey<Private>,
}

fn clave() -> PKey<Private> {
    let grupo = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    PKey::from_ec_key(EcKey::generate(&grupo).unwrap()).unwrap()
}

fn ahora() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Un certificado. `firmante` es quien lo firma (o él mismo, si es `None`); `sans`, sus nombres (`dns:x`, `ip:y`);
/// `desde` y `hasta`, en días respecto a hoy.
fn emitir(
    cn: &str,
    es_ca: bool,
    sans: &[&str],
    firmante: Option<&Par>,
    desde: i64,
    hasta: i64,
) -> Par {
    let k = clave();
    let mut nombre = X509NameBuilder::new().unwrap();
    nombre.append_entry_by_nid(Nid::COMMONNAME, cn).unwrap();
    let nombre = nombre.build();
    let mut b = X509::builder().unwrap();
    b.set_version(2).unwrap();
    let mut serie = BigNum::new().unwrap();
    serie.rand(64, MsbOption::MAYBE_ZERO, false).unwrap();
    b.set_serial_number(&serie.to_asn1_integer().unwrap())
        .unwrap();
    b.set_subject_name(&nombre).unwrap();
    b.set_pubkey(&k).unwrap();
    b.set_not_before(&Asn1Time::from_unix(ahora() + desde * 86_400).unwrap())
        .unwrap();
    b.set_not_after(&Asn1Time::from_unix(ahora() + hasta * 86_400).unwrap())
        .unwrap();
    match firmante {
        Some(f) => b.set_issuer_name(f.cert.subject_name()).unwrap(),
        None => b.set_issuer_name(&nombre).unwrap(),
    }
    if es_ca {
        b.append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        b.append_extension(
            KeyUsage::new()
                .critical()
                .key_cert_sign()
                .crl_sign()
                .build()
                .unwrap(),
        )
        .unwrap();
    } else {
        b.append_extension(BasicConstraints::new().critical().build().unwrap())
            .unwrap();
        b.append_extension(
            KeyUsage::new()
                .critical()
                .digital_signature()
                .build()
                .unwrap(),
        )
        .unwrap();
    }
    if !sans.is_empty() {
        let mut s = SubjectAlternativeName::new();
        for n in sans {
            match n.split_once(':').unwrap() {
                ("dns", v) => s.dns(v),
                ("ip", v) => s.ip(v),
                _ => panic!("un SAN que no es dns: ni ip:"),
            };
        }
        let ext = s
            .build(&b.x509v3_context(firmante.map(|f| f.cert.as_ref()), None))
            .unwrap();
        b.append_extension(ext).unwrap();
    }
    let firma = firmante.map_or(&k, |f| &f.clave);
    b.sign(firma, MessageDigest::sha256()).unwrap();
    Par {
        cert: b.build(),
        clave: k,
    }
}

/// La CA en la que el cliente confía, y otra en la que NO.
fn ca_de_confianza() -> Par {
    emitir("CA de confianza", true, &[], None, -1, 30)
}

fn otra_ca() -> Par {
    emitir("CA que no es de confianza", true, &[], None, -1, 30)
}

fn hoja(sans: &[&str], firmante: &Par) -> Par {
    emitir("servidor", false, sans, Some(firmante), -1, 30)
}

// ── El servidor ─────────────────────────────────────────────────────────────

/// Lo que el servidor vio de una conexión.
#[derive(Debug, Clone)]
struct Visto {
    /// Si el *handshake* TLS terminó.
    handshake: bool,
    /// Los bytes que llegaron (en un servidor sin TLS, todos los que llegaron).
    bytes: Vec<u8>,
}

enum Modo {
    /// Presenta `hoja` y, si se dice, solo esta versión de TLS.
    Tls {
        hoja: Par,
        solo_version: Option<SslVersion>,
    },
    /// No habla TLS: lee lo que le llegue.
    Plano,
}

struct EnMarcha {
    puerto: u16,
    parar: Arc<AtomicBool>,
    hilo: JoinHandle<()>,
    vistos: Arc<Mutex<Vec<Visto>>>,
    /// Las peticiones de Avatica que llegaron a una conexión con el *handshake* hecho.
    recibidas: Arc<Mutex<Vec<Recibida>>>,
}

impl EnMarcha {
    fn termina(self) -> (Vec<Visto>, Vec<Recibida>) {
        self.parar.store(true, Ordering::SeqCst);
        let _ = self.hilo.join();
        let vistos = std::mem::take(&mut *self.vistos.lock().unwrap());
        let recibidas = std::mem::take(&mut *self.recibidas.lock().unwrap());
        (vistos, recibidas)
    }
}

fn contexto(hoja: &Par, solo: Option<SslVersion>) -> SslContext {
    let mut c = SslContextBuilder::new(SslMethod::tls_server()).unwrap();
    c.set_certificate(&hoja.cert).unwrap();
    c.set_private_key(&hoja.clave).unwrap();
    match solo {
        Some(v) => {
            c.set_min_proto_version(Some(v)).unwrap();
            c.set_max_proto_version(Some(v)).unwrap();
            // TLS 1.1 solo existe con el nivel de seguridad 0.
            c.set_cipher_list("ALL:@SECLEVEL=0").unwrap();
        }
        None => c.set_min_proto_version(Some(SslVersion::TLS1_2)).unwrap(),
    }
    c.build()
}

fn contestar_siempre_bien(_: &Recibida) -> Option<Respuesta> {
    Some(Respuesta::avatica(
        200,
        "OpenConnectionResponse",
        &OpenConnectionResponse::default(),
    ))
}

fn arrancar(modo: Modo) -> EnMarcha {
    let v4 = TcpListener::bind("127.0.0.1:0").unwrap();
    let puerto = v4.local_addr().unwrap().port();
    v4.set_nonblocking(true).unwrap();
    let ctx = match &modo {
        Modo::Tls { hoja, solo_version } => Some(contexto(hoja, *solo_version)),
        Modo::Plano => None,
    };
    let parar = Arc::new(AtomicBool::new(false));
    let vistos: Arc<Mutex<Vec<Visto>>> = Arc::default();
    let recibidas: Arc<Mutex<Vec<Recibida>>> = Arc::default();
    let (p, v, r) = (parar.clone(), vistos.clone(), recibidas.clone());
    let hilo = std::thread::spawn(move || {
        while !p.load(Ordering::SeqCst) {
            let Ok((flujo, _)) = v4.accept() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            flujo.set_nonblocking(false).unwrap();
            flujo
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            match ctx.as_ref() {
                None => {
                    let mut f = flujo;
                    let mut b = vec![0u8; 8192];
                    let n = f.read(&mut b).unwrap_or(0);
                    b.truncate(n);
                    // Lo que haría un servidor HTTP sin TLS al que le llega un ClientHello.
                    let _ = f.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
                    v.lock().unwrap().push(Visto {
                        handshake: false,
                        bytes: b,
                    });
                }
                Some(ctx) => match Ssl::new(ctx).unwrap().accept(flujo) {
                    Ok(s) => {
                        v.lock().unwrap().push(Visto {
                            handshake: true,
                            bytes: Vec::new(),
                        });
                        comun::atender(s, &contestar_siempre_bien, &r);
                    }
                    Err(_) => v.lock().unwrap().push(Visto {
                        handshake: false,
                        bytes: Vec::new(),
                    }),
                },
            }
        }
    });
    EnMarcha {
        puerto,
        parar,
        hilo,
        vistos,
        recibidas,
    }
}

// ── El cliente ──────────────────────────────────────────────────────────────

struct Resultado {
    cliente: Result<(), Error>,
    vistos: Vec<Visto>,
    recibidas: Vec<Recibida>,
}

impl Resultado {
    fn handshake(&self) -> bool {
        self.vistos.iter().any(|v| v.handshake)
    }
}

/// Abre una conexión a `https://<host>:<puerto>/` con la CA de confianza en `ca`.
fn caso(host: &str, ca: &Par, modo: Modo) -> Resultado {
    caso_con(host, Some(ca), modo)
}

fn caso_con(host: &str, ca: Option<&Par>, modo: Modo) -> Resultado {
    let s = arrancar(modo);
    let mut o = Opciones::nuevas(format!("https://{host}:{}/", s.puerto));
    o.credencial = Some((USUARIO.into(), CLAVE.into()));
    o.tiempo_conexion = Duration::from_secs(5);
    o.tiempo_peticion = Duration::from_secs(5);
    if let Some(ca) = ca {
        o.confiar_ademas.push(ca.cert.to_pem().unwrap());
    }
    let cliente = Conexion::abrir(&o).map(drop);
    let (vistos, recibidas) = s.termina();
    if let Err(e) = &cliente {
        // La regla de todos los casos: la contraseña no sale en un error, ni en su `Debug`.
        for t in [e.to_string(), format!("{e:?}")] {
            assert!(!t.contains(CLAVE), "el error repite la contraseña: {t}");
        }
    }
    Resultado {
        cliente,
        vistos,
        recibidas,
    }
}

/// El cliente rechazó el servidor: el servidor no recibió NI una petición (con su cabecera `Authorization`), y el
/// error es `Tls` —que no se reintenta— y dice que la credencial no salió. **No se mira si el servidor dio el
/// *handshake* por terminado**: en TLS 1.3 lo da antes de que el cliente verifique el certificado y aborte, y eso
/// no quiere decir que el cliente aceptara.
fn rechazado(r: &Resultado) {
    assert!(
        r.recibidas.is_empty(),
        "el cliente mandó peticiones a un servidor que rechazaba: {:?}",
        r.recibidas
    );
    let e = r.cliente.as_ref().expect_err("conectó");
    assert!(matches!(e, Error::Tls(_)), "se esperaba Error::Tls: {e:?}");
    assert!(!e.reintentable(), "un certificado malo no se reintenta");
    assert!(e.to_string().contains("no se envió la credencial"), "{e}");
}

/// El cliente aceptó: el *handshake* terminó, la conexión se abrió y **la credencial llegó, por el canal cifrado**.
fn aceptado(r: &Resultado) {
    assert!(
        r.handshake(),
        "el servidor no terminó el handshake: el cliente RECHAZÓ el certificado: {:?}",
        r.cliente
    );
    r.cliente.as_ref().expect("la conexión no se abrió");
    let a = r
        .recibidas
        .iter()
        .find(|p| p.nombre == "OpenConnectionRequest")
        .and_then(|p| p.autorizacion.clone())
        .expect("el servidor no recibió la cabecera Authorization");
    assert!(a.starts_with("Basic "), "{a}");
}

// ── Los casos ───────────────────────────────────────────────────────────────

#[test]
fn un_certificado_de_la_ca_con_el_nombre_de_la_url_se_acepta_y_la_credencial_llega_cifrada() {
    let ca = ca_de_confianza();
    aceptado(&caso(
        "localhost",
        &ca,
        Modo::Tls {
            hoja: hoja(&["dns:localhost"], &ca),
            solo_version: None,
        },
    ));
}

#[test]
fn un_certificado_con_la_ip_se_acepta_si_la_url_es_la_ip() {
    let ca = ca_de_confianza();
    aceptado(&caso(
        "127.0.0.1",
        &ca,
        Modo::Tls {
            hoja: hoja(&["ip:127.0.0.1"], &ca),
            solo_version: None,
        },
    ));
}

#[test]
fn el_nombre_de_la_url_se_compara_sin_mirar_mayusculas() {
    let ca = ca_de_confianza();
    aceptado(&caso(
        "LOCALHOST",
        &ca,
        Modo::Tls {
            hoja: hoja(&["dns:localhost"], &ca),
            solo_version: None,
        },
    ));
}

#[test]
fn otro_nombre_se_rechaza_aunque_la_ca_sea_de_confianza() {
    let ca = ca_de_confianza();
    rechazado(&caso(
        "localhost",
        &ca,
        Modo::Tls {
            hoja: hoja(&["dns:otro.host"], &ca),
            solo_version: None,
        },
    ));
}

#[test]
fn un_certificado_solo_con_el_nombre_no_vale_si_la_url_es_una_ip() {
    // Es la verificación normal de HTTPS: la IP no está en el certificado y no se inventa una verificación.
    let ca = ca_de_confianza();
    rechazado(&caso(
        "127.0.0.1",
        &ca,
        Modo::Tls {
            hoja: hoja(&["dns:localhost"], &ca),
            solo_version: None,
        },
    ));
}

#[test]
fn un_comodin_de_otro_dominio_no_vale() {
    let ca = ca_de_confianza();
    rechazado(&caso(
        "localhost",
        &ca,
        Modo::Tls {
            hoja: hoja(&["dns:*.otro.host"], &ca),
            solo_version: None,
        },
    ));
}

#[test]
fn una_ca_que_no_es_de_confianza_se_rechaza_aunque_el_nombre_case() {
    let ca = ca_de_confianza();
    let otra = otra_ca();
    rechazado(&caso(
        "localhost",
        &ca,
        Modo::Tls {
            hoja: hoja(&["dns:localhost"], &otra),
            solo_version: None,
        },
    ));
}

#[test]
fn sin_confiar_en_la_ca_de_la_prueba_se_rechaza_incluso_un_certificado_perfecto() {
    // El caso base de «el cliente no confía en nada que no sea del sistema o declarado».
    let ca = ca_de_confianza();
    rechazado(&caso_con(
        "localhost",
        None,
        Modo::Tls {
            hoja: hoja(&["dns:localhost"], &ca),
            solo_version: None,
        },
    ));
}

#[test]
fn un_certificado_caducado_se_rechaza() {
    let ca = ca_de_confianza();
    let caducado = emitir("servidor", false, &["dns:localhost"], Some(&ca), -30, -1);
    rechazado(&caso(
        "localhost",
        &ca,
        Modo::Tls {
            hoja: caducado,
            solo_version: None,
        },
    ));
}

#[test]
fn un_certificado_autofirmado_se_rechaza() {
    let ca = ca_de_confianza();
    let propio = emitir("servidor", false, &["dns:localhost"], None, -1, 30);
    rechazado(&caso(
        "localhost",
        &ca,
        Modo::Tls {
            hoja: propio,
            solo_version: None,
        },
    ));
}

#[test]
fn un_servidor_que_solo_habla_tls_1_1_se_rechaza() {
    // El mínimo es 1.2 y está en el contexto del cliente, no en la política de la máquina.
    let ca = ca_de_confianza();
    let r = caso(
        "localhost",
        &ca,
        Modo::Tls {
            hoja: hoja(&["dns:localhost"], &ca),
            solo_version: Some(SslVersion::TLS1_1),
        },
    );
    rechazado(&r);
}

#[test]
fn un_servidor_sin_tls_detras_de_una_url_https_no_recibe_la_credencial() {
    let ca = ca_de_confianza();
    let r = caso("localhost", &ca, Modo::Plano);
    assert!(r.cliente.is_err(), "conectó a un servidor sin TLS");
    for v in &r.vistos {
        assert!(
            !v.bytes.windows(CLAVE.len()).any(|w| w == CLAVE.as_bytes()),
            "la contraseña llegó a un servidor sin TLS"
        );
        assert!(
            !v.bytes
                .windows("Authorization".len())
                .any(|w| w.eq_ignore_ascii_case(b"Authorization")),
            "la cabecera Authorization llegó en claro"
        );
    }
}

#[test]
fn un_servidor_http_en_claro_recibe_lo_que_el_conector_no_permite_y_aqui_solo_se_comprueba_que_no_se_sigue_una_redireccion()
 {
    // La URL con credencial por HTTP la rechaza `ore-phoenix` (url.rs); aquí se comprueba el otro hueco: un
    // servidor que redirige no se lleva la cabecera `Authorization` a otra parte.
    let destino = TcpListener::bind("127.0.0.1:0").unwrap();
    destino.set_nonblocking(true).unwrap();
    let puerto_destino = destino.local_addr().unwrap().port();

    let origen = TcpListener::bind("127.0.0.1:0").unwrap();
    let puerto_origen = origen.local_addr().unwrap().port();
    let hilo = std::thread::spawn(move || {
        let (mut f, _) = origen.accept().unwrap();
        f.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        // Se lee la petición ENTERA (cabecera y cuerpo) antes de contestar: si se cierra con bytes sin leer, el
        // sistema manda un RST, el cliente lo toma por un fallo de red y reintenta contra un puerto ya cerrado.
        let mut recibido = Vec::new();
        let mut b = [0u8; 4096];
        let (cabecera, largo) = loop {
            let n = f.read(&mut b).unwrap_or(0);
            if n == 0 {
                break (recibido.len(), 0);
            }
            recibido.extend_from_slice(&b[..n]);
            if let Some(i) = recibido.windows(4).position(|w| w == b"\r\n\r\n") {
                let texto = String::from_utf8_lossy(&recibido[..i]).to_lowercase();
                let largo = texto
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                break (i + 4, largo);
            }
        };
        while recibido.len() < cabecera + largo {
            let n = f.read(&mut b).unwrap_or(0);
            if n == 0 {
                break;
            }
            recibido.extend_from_slice(&b[..n]);
        }
        let r = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{puerto_destino}/\r\nContent-Length: 0\r\n\r\n"
        );
        let _ = f.write_all(r.as_bytes());
    });

    let mut o = Opciones::nuevas(format!("http://127.0.0.1:{puerto_origen}/"));
    o.credencial = Some((USUARIO.into(), CLAVE.into()));
    o.tiempo_peticion = Duration::from_secs(5);
    let e = Conexion::abrir(&o)
        .map(drop)
        .expect_err("siguió la redirección");
    hilo.join().unwrap();

    assert!(
        matches!(e, Error::Http { estado: 302, .. }),
        "una redirección es un error: {e:?}"
    );
    assert!(
        destino.accept().is_err(),
        "el cliente fue a donde le mandó la redirección"
    );
}

// ── El PQS de verdad, con TLS ───────────────────────────────────────────────

/// `PHOENIX_TLS_URL=https://localhost:8766/` y `PHOENIX_TLS_CA=<ca.pem>` (los deja `tls-lab.sh`). Sin ellas, no hace
/// nada: es el único test que necesita el laboratorio.
fn pqs_con_tls() -> Option<(String, Vec<u8>)> {
    let url = std::env::var("PHOENIX_TLS_URL").ok()?;
    let ca = std::env::var("PHOENIX_TLS_CA").ok()?;
    Some((
        url,
        std::fs::read(ca).expect("no se lee la CA del laboratorio"),
    ))
}

#[test]
fn el_pqs_real_por_https_se_lee_con_su_ca_y_se_rechaza_sin_ella() {
    let Some((url, ca)) = pqs_con_tls() else {
        return;
    };
    // Con la CA del laboratorio: una conexión, una consulta y el dato.
    let mut o = Opciones::nuevas(&url);
    o.confiar_ademas.push(ca);
    let c = Conexion::abrir(&o).expect("abrir contra el PQS con TLS");
    let mut l = c
        .ejecutar("SELECT 41 + 1 FROM SYSTEM.CATALOG LIMIT 1")
        .unwrap();
    let f = l.siguiente_frame().unwrap().unwrap();
    assert_eq!(f[0][0], ore_avatica::Valor::Entero(42), "{f:?}");
    drop(l);
    drop(c);

    // Sin ella, con una credencial: el certificado no es de nadie en quien se confíe, y la credencial no sale.
    let mut o = Opciones::nuevas(&url);
    o.credencial = Some((USUARIO.into(), CLAVE.into()));
    let e = Conexion::abrir(&o)
        .map(drop)
        .expect_err("conectó sin confiar en la CA");
    assert!(matches!(e, Error::Tls(_)), "{e:?}");
    assert!(!e.to_string().contains(CLAVE), "{e}");
}

#[test]
fn el_pqs_real_con_tls_no_contesta_a_http_en_claro() {
    let Some((url, _)) = pqs_con_tls() else {
        return;
    };
    let claro = url.replacen("https://", "http://", 1);
    let mut o = Opciones::nuevas(claro);
    o.tiempo_peticion = Duration::from_secs(10);
    // Lo que sea que conteste Jetty a un POST en claro en un puerto TLS, no es una conexión abierta.
    assert!(
        Conexion::abrir(&o).is_err(),
        "abrió una conexión en claro contra un puerto TLS"
    );
}
