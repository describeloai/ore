//! **El TLS, contra un servidor de verdad** (ficha 0070, §TLS real). **No necesita Cassandra**: lo único que
//! interesa es si el cliente ACEPTA el certificado y qué le manda al servidor cuando no lo acepta.
//!
//! Un servidor TLS **en el propio proceso** (`SslAcceptor`) presenta el certificado que cada caso quiera, y los
//! certificados los genera el test con el crate `openssl`: una CA de confianza, otra que no lo es, certificados
//! con la IP, solo con el nombre, de otra IP, caducados, con la cadena incompleta. Cada caso afirma **lo que
//! hizo el servidor** —si el *handshake* terminó— y **lo que recibió**: si el cliente rechaza un certificado,
//! tiene que no haber mandado ni un byte de aplicación, y en ningún caso aparece la contraseña.
//!
//! Corre en el CI y en cualquier máquina. El servidor no habla CQL: si el cliente acepta el certificado,
//! luego falla porque aquello no es Cassandra, y eso es lo esperado.

use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::ssl::{Ssl, SslContextBuilder, SslMethod, SslVersion};
use openssl::x509::extension::{BasicConstraints, KeyUsage, SubjectAlternativeName};
use openssl::x509::{X509, X509NameBuilder};
use ore_cassandra::tls::confiar_ademas_en;
use ore_cassandra::transporte::Conexion;
use ore_driver::{Codigo, Fallo};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// La contraseña del cliente: no puede aparecer en NINGÚN byte que reciba ningún servidor.
const CLAVE: &str = "CLAVE-SECRETA-m6";

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

/// Un certificado. `firmante` es quien lo firma (o él mismo, si es `None`); `sans`, sus nombres
/// (`dns:x`, `ip:y`); `desde` y `hasta`, en días respecto a hoy.
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
    let (firma, emisor) = match firmante {
        Some(f) => (&f.clave, f.cert.clone()),
        None => (&k, X509::builder().unwrap().build()),
    };
    let _ = emisor;
    b.sign(firma, MessageDigest::sha256()).unwrap();
    Par {
        cert: b.build(),
        clave: k,
    }
}

/// La CA en la que el cliente confía (una sola, para todo el proceso), y otra en la que NO.
fn ca_de_confianza() -> &'static Par {
    static CA: OnceLock<Par> = OnceLock::new();
    CA.get_or_init(|| {
        let ca = emitir("CA de confianza", true, &[], None, -1, 30);
        confiar_ademas_en(ca.cert.clone());
        ca
    })
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
    /// Los bytes de aplicación (o, en un servidor sin TLS, todos los que llegaron).
    bytes: Vec<u8>,
}

enum Servidor {
    /// Presenta `cadena` (la hoja y, quizá, intermedias) y, si se dice, solo esta versión de TLS.
    Tls {
        hoja: Par,
        intermedias: Vec<X509>,
        solo_version: Option<SslVersion>,
    },
    /// No habla TLS: lee lo que le llegue.
    Plano,
}

struct EnMarcha {
    puerto: u16,
    parar: Arc<AtomicBool>,
    hilos: Vec<JoinHandle<()>>,
    vistos: Arc<Mutex<Vec<Visto>>>,
}

impl EnMarcha {
    fn termina(self) -> Vec<Visto> {
        self.parar.store(true, Ordering::SeqCst);
        for h in self.hilos {
            let _ = h.join();
        }
        std::mem::take(&mut *self.vistos.lock().unwrap())
    }
}

fn contexto_servidor(
    hoja: &Par,
    intermedias: &[X509],
    solo: Option<SslVersion>,
) -> Result<openssl::ssl::SslContext, String> {
    let e = |e: openssl::error::ErrorStack| e.to_string();
    let mut c = SslContextBuilder::new(SslMethod::tls_server()).map_err(e)?;
    c.set_certificate(&hoja.cert).map_err(e)?;
    c.set_private_key(&hoja.clave).map_err(e)?;
    for i in intermedias {
        c.add_extra_chain_cert(i.clone()).map_err(e)?;
    }
    match solo {
        Some(v) => {
            c.set_min_proto_version(Some(v)).map_err(e)?;
            c.set_max_proto_version(Some(v)).map_err(e)?;
            // TLS 1.1 solo existe con el nivel de seguridad 0.
            c.set_cipher_list("ALL:@SECLEVEL=0").map_err(e)?;
        }
        None => c
            .set_min_proto_version(Some(SslVersion::TLS1_2))
            .map_err(e)?,
    }
    Ok(c.build())
}

fn arrancar(modo: Servidor) -> Result<EnMarcha, String> {
    let v4 = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let puerto = v4.local_addr().unwrap().port();
    // `localhost` puede resolver a `::1` antes que a `127.0.0.1`: se escucha en los dos, si se puede.
    let v6 = TcpListener::bind(("::1", puerto)).ok();
    let ctx = match &modo {
        Servidor::Tls {
            hoja,
            intermedias,
            solo_version,
        } => Some(contexto_servidor(hoja, intermedias, *solo_version)?),
        Servidor::Plano => None,
    };
    let parar = Arc::new(AtomicBool::new(false));
    let vistos = Arc::new(Mutex::new(Vec::new()));
    let mut hilos = Vec::new();
    let escuchas: Vec<TcpListener> = std::iter::once(v4).chain(v6).collect();
    for l in escuchas {
        l.set_nonblocking(true).unwrap();
        let (parar, vistos, ctx) = (parar.clone(), vistos.clone(), ctx.clone());
        hilos.push(std::thread::spawn(move || {
            while !parar.load(Ordering::SeqCst) {
                let Ok((flujo, _)) = l.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                flujo.set_nonblocking(false).unwrap();
                flujo
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let visto = atender(flujo, ctx.as_ref());
                vistos.lock().unwrap().push(visto);
            }
        }));
    }
    Ok(EnMarcha {
        puerto,
        parar,
        hilos,
        vistos,
    })
}

fn atender(flujo: TcpStream, ctx: Option<&openssl::ssl::SslContext>) -> Visto {
    let leer = |s: &mut dyn std::io::Read| {
        let mut b = vec![0u8; 8192];
        let n = s.read(&mut b).unwrap_or(0);
        b.truncate(n);
        b
    };
    match ctx {
        None => {
            let mut f = flujo;
            let bytes = leer(&mut f);
            // Lo que hace un Cassandra sin TLS cuando le llega un ClientHello: contesta con una trama de
            // error del protocolo nativo (versión 0x84, opcode ERROR), que para un cliente TLS no es TLS.
            let texto = b"Invalid or unsupported protocol version";
            let mut cuerpo = vec![0, 0, 0, 0x0A, 0, texto.len() as u8];
            cuerpo.extend_from_slice(texto);
            let mut trama = vec![0x84, 0, 0, 0, 0];
            trama.extend_from_slice(&(cuerpo.len() as u32).to_be_bytes());
            trama.extend(cuerpo);
            let _ = std::io::Write::write_all(&mut f, &trama);
            Visto {
                handshake: false,
                bytes,
            }
        }
        Some(ctx) => match Ssl::new(ctx).unwrap().accept(flujo) {
            Ok(mut s) => Visto {
                handshake: true,
                bytes: leer(&mut s),
            },
            Err(_) => Visto {
                handshake: false,
                bytes: Vec::new(),
            },
        },
    }
}

// ── El cliente ──────────────────────────────────────────────────────────────

struct Resultado {
    cliente: Fallo,
    servidor: Vec<Visto>,
}

impl Resultado {
    fn handshake(&self) -> bool {
        self.servidor.iter().any(|v| v.handshake)
    }
}

/// Conecta a un servidor con `modo`, por el host que se diga, y devuelve lo que pasó de cada lado.
/// El cliente falla SIEMPRE (el servidor no es Cassandra): lo que importa es **por qué**.
fn caso(host: &str, modo: Servidor) -> Resultado {
    ca_de_confianza();
    let s = arrancar(modo).expect("el servidor de prueba no arranca");
    let url = format!(
        "cassandra://u:{CLAVE}@{host}:{}/?tls=true&contacto=solo",
        s.puerto
    );
    let cliente = match Conexion::abrir(&url) {
        Ok(_) => panic!("conectó a algo que no es Cassandra"),
        Err(f) => f,
    };
    let servidor = s.termina();
    // La regla de todos los casos: la contraseña no sale, ni en claro ni en nada.
    for v in &servidor {
        assert!(
            !v.bytes.windows(CLAVE.len()).any(|w| w == CLAVE.as_bytes()),
            "la contraseña llegó al servidor: {} bytes",
            v.bytes.len()
        );
    }
    assert!(
        !cliente.mensaje.contains(CLAVE),
        "el error repite la contraseña: {}",
        cliente.mensaje
    );
    Resultado { cliente, servidor }
}

fn tls(hoja: Par) -> Servidor {
    Servidor::Tls {
        hoja,
        intermedias: Vec::new(),
        solo_version: None,
    }
}

/// El cliente rechazó el certificado: el servidor no terminó el *handshake*, no recibió NI UN byte de
/// aplicación, y el error lo cuenta y no es reintentable.
fn rechazado(r: &Resultado, razon: &str) {
    assert!(
        !r.handshake(),
        "el servidor terminó el handshake: el cliente ACEPTÓ el certificado. {:?}",
        r.cliente
    );
    assert!(
        r.servidor.iter().all(|v| v.bytes.is_empty()),
        "el cliente mandó bytes a un servidor cuyo certificado rechazaba: {:?}",
        r.servidor
    );
    assert_eq!(r.cliente.codigo, Codigo::Conexion, "{:?}", r.cliente);
    assert!(
        !r.cliente.reintentable,
        "un certificado malo no se reintenta"
    );
    let m = r.cliente.mensaje.to_lowercase();
    assert!(
        m.contains("no se pudo verificar") && m.contains("no se envió la credencial"),
        "{}",
        r.cliente.mensaje
    );
    assert!(
        m.contains(&razon.to_lowercase()),
        "se esperaba «{razon}» en: {}",
        r.cliente.mensaje
    );
}

/// El cliente aceptó el certificado: el *handshake* terminó. (Luego falla, porque no es Cassandra.)
fn aceptado(r: &Resultado) {
    assert!(
        r.handshake(),
        "el servidor no terminó el handshake: el cliente RECHAZÓ el certificado: {}",
        r.cliente.mensaje
    );
}

// ── Los casos ───────────────────────────────────────────────────────────────

#[test]
fn un_certificado_con_la_ip_se_acepta_por_ip_y_por_nombre() {
    let ca = ca_de_confianza();
    let h = hoja(&["ip:127.0.0.1", "ip:::1", "dns:localhost"], ca);
    aceptado(&caso("127.0.0.1", tls(h.clone())));
    aceptado(&caso("localhost", tls(h)));
}

#[test]
fn un_certificado_solo_con_el_nombre_se_acepta_por_el_nombre_de_la_url() {
    // Lo que el driver solo, que mira la IP, rechazaba: el certificado normal de un clúster.
    let ca = ca_de_confianza();
    aceptado(&caso("localhost", tls(hoja(&["dns:localhost"], ca))));
}

#[test]
fn el_nombre_de_la_url_se_compara_sin_mirar_mayusculas() {
    let ca = ca_de_confianza();
    aceptado(&caso("LOCALHOST", tls(hoja(&["dns:LocalHost"], ca))));
}

#[test]
fn solo_con_el_nombre_pero_la_url_es_una_ip_se_rechaza() {
    // No hay nombre en la URL con que casar, y la IP no está: no se inventa una verificación.
    let ca = ca_de_confianza();
    rechazado(
        &caso("127.0.0.1", tls(hoja(&["dns:localhost"], ca))),
        "mismatch",
    );
}

#[test]
fn otro_nombre_y_otra_ip_se_rechaza() {
    let ca = ca_de_confianza();
    rechazado(
        &caso(
            "localhost",
            tls(hoja(&["dns:otro.host", "ip:10.9.9.9"], ca)),
        ),
        "mismatch",
    );
}

#[test]
fn un_comodin_de_otro_dominio_no_vale() {
    let ca = ca_de_confianza();
    rechazado(
        &caso("localhost", tls(hoja(&["dns:*.otro.host"], ca))),
        "mismatch",
    );
}

#[test]
fn una_ca_que_no_es_de_confianza_se_rechaza_aunque_los_nombres_casen() {
    ca_de_confianza();
    let otra = otra_ca();
    rechazado(
        &caso(
            "localhost",
            tls(hoja(&["ip:127.0.0.1", "ip:::1", "dns:localhost"], &otra)),
        ),
        "issuer",
    );
}

#[test]
fn un_certificado_caducado_se_rechaza_aunque_el_nombre_case() {
    // El callback solo perdona la IP que falta: nunca un certificado caducado.
    let ca = ca_de_confianza();
    let caducado = emitir("servidor", false, &["dns:localhost"], Some(ca), -10, -1);
    rechazado(&caso("localhost", tls(caducado)), "expired");
}

#[test]
fn una_cadena_incompleta_se_rechaza_y_completa_se_acepta() {
    let ca = ca_de_confianza();
    let intermedia = emitir("intermedia", true, &[], Some(ca), -1, 30);
    let h = emitir(
        "servidor",
        false,
        &["dns:localhost"],
        Some(&intermedia),
        -1,
        30,
    );
    // El servidor presenta solo la hoja: falta la intermedia.
    rechazado(&caso("localhost", tls(h.clone())), "issuer");
    // Con la intermedia, la cadena llega a la CA de confianza.
    aceptado(&caso(
        "localhost",
        Servidor::Tls {
            hoja: h,
            intermedias: vec![intermedia.cert.clone()],
            solo_version: None,
        },
    ));
}

#[test]
fn un_servidor_sin_tls_se_dice_y_no_recibe_la_credencial() {
    let r = caso("127.0.0.1", Servidor::Plano);
    assert!(!r.handshake());
    assert_eq!(r.cliente.codigo, Codigo::Conexion, "{:?}", r.cliente);
    assert!(!r.cliente.reintentable);
    assert!(
        r.cliente.mensaje.contains("no habla TLS") && r.cliente.mensaje.contains("UseSSL"),
        "{}",
        r.cliente.mensaje
    );
    // Lo que llega a un servidor sin TLS es el principio de un handshake (un ClientHello), no una credencial:
    // `caso` ya comprobó que la contraseña no está en ningún byte.
}

#[test]
fn tls_1_1_no_se_admite() {
    let ca = ca_de_confianza();
    let h = hoja(&["ip:127.0.0.1", "dns:localhost"], ca);
    let servidor = Servidor::Tls {
        hoja: h,
        intermedias: Vec::new(),
        solo_version: Some(SslVersion::TLS1_1),
    };
    // Algunas compilaciones de OpenSSL ya no saben hablar TLS 1.1: no es culpa del cliente, y se dice.
    if arrancar(match &servidor {
        Servidor::Tls {
            hoja, solo_version, ..
        } => Servidor::Tls {
            hoja: hoja.clone(),
            intermedias: Vec::new(),
            solo_version: *solo_version,
        },
        Servidor::Plano => unreachable!(),
    })
    .map(|s| s.termina())
    .is_err()
    {
        eprintln!("este OpenSSL no habla TLS 1.1: el caso no se puede montar, se salta");
        return;
    }
    let r = caso("127.0.0.1", servidor);
    assert!(!r.handshake(), "el cliente habló TLS 1.1");
    assert!(r.servidor.iter().all(|v| v.bytes.is_empty()));
    assert_eq!(r.cliente.codigo, Codigo::Conexion, "{:?}", r.cliente);
}
