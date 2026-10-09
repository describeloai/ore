//! **Un servidor SFTP como origen, contra `atmoz/sftp`** (ADR 0061 O4·1):
//! OpenSSH de verdad, con la clave de la "celda" autorizada (`ORE_SFTP_CLAVE`)
//! y la huella del host fijada en la URL (`ORE_LAB_SFTP`, con `edad=0`). La
//! muestra la siembra el laboratorio: `datos/docs/a.pdf`, `datos/grande.bin`
//! (8 MiB), un enlace simbólico y un fichero sin permiso de lectura. Sin la
//! URL no hay laboratorio, y el test lo dice.

use ore_objetos::{Origen, Rechazo, etag_de};
use ore_sftp::{Sftp, Tipo};
use std::io::{Read, Write};

#[test]
fn un_servidor_sftp_como_origen() {
    let Ok(url) = std::env::var("ORE_LAB_SFTP") else {
        eprintln!("sin ORE_LAB_SFTP no hay laboratorio: nada que probar (o4-sftp.sh)");
        return;
    };
    let s = Sftp::de_url(&url).expect("la URL");
    let huella = s.fuente.huella.clone().expect("la huella fijada");

    // la huella: la vista es la fijada; sin fijar, se dice cuál es; otra, se niega
    assert_eq!(s.huella_del_host().unwrap(), huella);
    let sin = Sftp::de_url(url.split('?').next().unwrap()).unwrap();
    match sin.entrar() {
        Err(f) => assert_eq!(
            f.tipo,
            Tipo::HuellaFalta {
                vista: huella.clone()
            },
            "{f}"
        ),
        Ok(()) => panic!("entró sin huella fijada"),
    }
    let otra = Sftp::de_url(&url.replace(&huella, "SHA256:otraotraotraotraotraotraotra")).unwrap();
    match otra.entrar() {
        Err(f) => assert!(matches!(f.tipo, Tipo::HuellaDistinta { .. }), "{f}"),
        Ok(()) => panic!("entró con otra huella"),
    }
    s.entrar().expect("la clave de la celda entra");

    // listar: los ficheros; el enlace, marcado y fuera del Origen
    let crudo = s.listar("datos/").unwrap();
    assert!(
        crudo
            .iter()
            .any(|e| e.clave == "datos/enlace.pdf" && e.enlace),
        "{crudo:?}"
    );
    let o: &dyn Origen = &s;
    let objs = o.listar("datos/").unwrap();
    let claves: Vec<&str> = objs.iter().map(|x| x.clave.as_str()).collect();
    assert!(
        claves.contains(&"datos/docs/a.pdf") && claves.contains(&"datos/docs/Nueva carpeta/b.pdf")
    );
    assert!(!claves.contains(&"datos/enlace.pdf"), "{claves:?}");
    // por prefijo, no sólo por carpeta
    assert_eq!(o.listar("datos/docs/a").unwrap().len(), 1);

    // versiones: el validador como versión (D-O1)
    let vs = o.listar_versiones("datos/docs/").unwrap();
    let a = vs
        .iter()
        .find(|v| v.clave == "datos/docs/a.pdf")
        .expect("a.pdf");
    assert!(etag_de(&a.version).is_some(), "{a:?}");
    // con una edad mínima larga, todo es demasiado joven para copiarse
    let paciente = Sftp::de_url(&url.replace("edad=0", "edad=3600")).unwrap();
    assert!(
        Origen::listar_versiones(&paciente, "datos/")
            .unwrap()
            .is_empty()
    );

    // leer: entero, un rango, y por el final (el pie de un Parquet)
    let (mut r, ab) = o.abrir_version(&a.clave, &a.version).unwrap();
    let mut b = Vec::new();
    r.read_to_end(&mut b).unwrap();
    assert_eq!(
        (b.as_slice(), ab.tamano),
        (&b"%PDF-1.4\n% uno\n%%EOF\n"[..], Some(21))
    );
    assert_eq!(o.rango(&a.clave, "0-3").unwrap(), b"%PDF");
    assert_eq!(o.rango_de(&a.clave, "-6", &a.etag).unwrap(), b"%%EOF\n");
    // un validador que ya no es: no se abre
    assert!(o.abrir(&a.clave, "1-21").is_err());
    // sin permiso de lectura, el error lo dice
    match s.leer("datos/cerrado.pdf", None) {
        Err(f) => assert_eq!(f.tipo, Tipo::Permiso, "{f}"),
        Ok(_) => panic!("leyó un fichero sin permiso"),
    }
    // fijado no se sirve: sólo mantenida (D-O1)
    match o.leer_fijado(&a.clave, &a.version, "", None) {
        Err(Rechazo::Origen(m)) => assert!(m.contains("D-O1"), "{m}"),
        _ => panic!("un SFTP no se sirve fijado"),
    }

    // ⭐ un fichero reescrito en sitio mientras se lee: la lectura falla, no
    //   da la mezcla de los dos (medido en O4·0)
    let g = o
        .listar_versiones("datos/grande.bin")
        .unwrap()
        .pop()
        .expect("grande.bin");
    let (mut r, _) = o.abrir_version(&g.clave, &g.version).unwrap();
    let mut primero = vec![0u8; 1 << 20];
    r.read_exact(&mut primero).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    reescribir(&s, "/datos/grande.bin", b'B', g.tamano as usize);
    let mut resto = Vec::new();
    let e = r
        .read_to_end(&mut resto)
        .expect_err("cambió mientras se leía");
    assert!(e.to_string().contains("cambió"), "{e}");
    eprintln!(
        "sftp: huella (fijada, sin fijar, otra), listar sin enlaces, versiones, edad, rangos, permiso, D-O1, cambio a medias"
    );
}

/// Lo que usa la prueba de fuego (`o4-sftp.sh`, paso 6): reescribe
/// `datos/docs/a.pdf` por fuera con el MISMO tamaño (`% uno` → `% UNO`),
/// entre `versiones` y `bajar`. Ignorado salvo que se pida.
#[test]
#[ignore]
fn reescribir_a_pdf() {
    let Ok(url) = std::env::var("ORE_LAB_SFTP") else {
        return;
    };
    let s = Sftp::de_url(&url).expect("la URL");
    escribir(&s, "/datos/docs/a.pdf", b"%PDF-1.4\n% UNO\n%%EOF\n");
}

/// Reescribe un fichero en sitio con otra conexión (el laboratorio deja
/// escribir; un origen de ORE nunca escribe).
fn reescribir(s: &Sftp, ruta: &str, letra: u8, tamano: usize) {
    escribir(s, ruta, &vec![letra; tamano]);
}

fn escribir(s: &Sftp, ruta: &str, datos: &[u8]) {
    let f = &s.fuente;
    let tcp = std::net::TcpStream::connect((f.host.as_str(), f.puerto)).unwrap();
    let mut ses = ssh2::Session::new().unwrap();
    ses.set_tcp_stream(tcp);
    ses.handshake().unwrap();
    ses.userauth_pubkey_file(
        &f.usuario,
        None,
        std::path::Path::new(&std::env::var("ORE_SFTP_CLAVE").unwrap()),
        None,
    )
    .unwrap();
    let sftp = ses.sftp().unwrap();
    let mut w = sftp
        .open_mode(
            std::path::Path::new(ruta),
            ssh2::OpenFlags::WRITE | ssh2::OpenFlags::TRUNCATE,
            0o644,
            ssh2::OpenType::File,
        )
        .unwrap();
    w.write_all(datos).unwrap();
}
