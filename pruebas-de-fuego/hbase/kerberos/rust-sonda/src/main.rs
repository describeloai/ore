//! SASL/GSSAPI contra un HBase con Kerberos, en Rust, y una llamada `Get` a mano para probar el envoltorio.
//!   sonda-kerberos <host> <puerto>
use libgssapi::context::{ClientCtx, CtxFlags, SecurityContext};
use libgssapi::credential::{Cred, CredUsage};
use libgssapi::name::Name;
use libgssapi::oid::{OidSet, GSS_MECH_KRB5, GSS_NT_HOSTBASED_SERVICE};
use std::io::{Read, Write};
use std::net::TcpStream;

fn leer(s: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    s.read_exact(&mut b).expect("leer");
    b
}
fn u32be(s: &mut TcpStream) -> usize {
    u32::from_be_bytes(leer(s, 4).try_into().unwrap()) as usize
}
fn marco(s: &mut TcpStream, cuerpo: &[u8]) {
    let mut m = (cuerpo.len() as u32).to_be_bytes().to_vec();
    m.extend_from_slice(cuerpo);
    s.write_all(&m).unwrap();
}
fn varint(mut n: usize, v: &mut Vec<u8>) {
    while n >= 0x80 {
        v.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    v.push(n as u8);
}
fn campo(num: u8, dato: &[u8], v: &mut Vec<u8>) {
    v.push(num << 3 | 2);
    varint(dato.len(), v);
    v.extend_from_slice(dato);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (host, puerto) = (&args[1], &args[2]);
    let mut s = TcpStream::connect(format!("{host}:{puerto}")).unwrap();
    s.write_all(b"HBas\x00\x51").unwrap(); // preámbulo: versión 0, autenticación SASL (81)

    let mecanismos = {
        let mut m = OidSet::new().unwrap();
        m.add(&GSS_MECH_KRB5).unwrap();
        m
    };
    let nombre = Name::new(format!("hbase@{host}").as_bytes(), Some(&GSS_NT_HOSTBASED_SERVICE))
        .unwrap()
        .canonicalize(Some(&GSS_MECH_KRB5))
        .unwrap();
    let cred = Cred::acquire(None, None, CredUsage::Initiate, Some(&mecanismos)).unwrap();
    let flags = CtxFlags::GSS_C_MUTUAL_FLAG | CtxFlags::GSS_C_SEQUENCE_FLAG | CtxFlags::GSS_C_INTEG_FLAG | CtxFlags::GSS_C_CONF_FLAG;
    let mut ctx = ClientCtx::new(Some(cred), nombre, flags, Some(&GSS_MECH_KRB5));

    let mut token: Vec<u8> = ctx.step(None, None).unwrap().expect("sin primer token").to_vec();
    let capa;
    loop {
        marco(&mut s, &token);
        let estado = u32be(&mut s);
        assert_eq!(estado, 0, "SASL rechazado");
        let n = u32be(&mut s);
        let reto = leer(&mut s, n);
        if !ctx.is_complete() {
            match ctx.step(Some(&reto), None).unwrap() {
                Some(t) => token = t.to_vec(),
                None => token = Vec::new(),
            }
            if !ctx.is_complete() {
                continue;
            }
            // el contexto se completó con este reto: sigue la oferta de capas (otro mensaje del servidor)
            continue;
        }
        let oferta = ctx.unwrap(&reto).unwrap();
        let ofrecidas = oferta[0];
        capa = if ofrecidas & 4 != 0 { 4 } else if ofrecidas & 2 != 0 { 2 } else { 1 };
        println!("contexto Kerberos completo; capas ofrecidas={ofrecidas:#04x}, elegida={capa}");
        let mut resp = vec![capa];
        resp.extend_from_slice(&oferta[1..4]);
        let w = ctx.wrap(false, &resp).unwrap();
        marco(&mut s, &w);
        break;
    }

    // ConnectionHeader {user_info{effective_user}, service_name}, y luego un Get de hbase:meta, todo por la capa elegida
    let mut ui = Vec::new();
    campo(1, b"lector", &mut ui);
    let mut ch = Vec::new();
    campo(1, &ui, &mut ch);
    campo(2, b"ClientService", &mut ch);
    let enviar = |s: &mut TcpStream, ctx: &mut ClientCtx, cuerpo: &[u8]| {
        if capa > 1 {
            let mut plano = (cuerpo.len() as u32).to_be_bytes().to_vec();
            plano.extend_from_slice(cuerpo);
            let w = ctx.wrap(capa == 4, &plano).unwrap();
            marco(s, &w);
        } else {
            marco(s, cuerpo);
        }
    };
    enviar(&mut s, &mut ctx, &ch);

    let mut cab = vec![0x08, 0x01]; // call_id = 1
    campo(3, b"Get", &mut cab); // method_name
    cab.extend_from_slice(&[0x20, 0x01]); // request_param = true
    let mut region = vec![0x08, 0x01]; // type = REGION_NAME
    campo(2, b"hbase:meta,,1", &mut region);
    let mut get = Vec::new();
    campo(1, b"TestTable,,", &mut get); // row
    let mut peticion = Vec::new();
    campo(1, &region, &mut peticion);
    campo(2, &get, &mut peticion);
    let mut msg = Vec::new();
    varint(cab.len(), &mut msg);
    msg.extend_from_slice(&cab);
    varint(peticion.len(), &mut msg);
    msg.extend_from_slice(&peticion);
    enviar(&mut s, &mut ctx, &msg);

    let n = u32be(&mut s);
    let mut dat = leer(&mut s, n);
    if capa > 1 {
        dat = ctx.unwrap(&dat).unwrap()[4..].to_vec();
    }
    // ResponseHeader: call_id (campo 1) y, si hay fallo, exception (campo 2)
    let lc = dat[0] as usize;
    let cabecera = &dat[1..1 + lc];
    let con_excepcion = cabecera.windows(1).any(|w| w[0] == 0x12);
    println!("respuesta de {} bytes; cabecera {:02x?}; excepcion={}", dat.len(), cabecera, con_excepcion);
}
