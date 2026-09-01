//! `ore-sign` — el firmador de referencia, **fuera** del compilador.
//!
//! El contrato es el mismo que el de `ore-fetch`, y no por simetría: lo que se
//! delega es distinto pero la razón de delegarlo es la misma clase de razón.
//!
//! - la petición entra por **stdin**, nunca por `argv` — lo lee cualquier
//!   proceso de la máquina, y aquí lo que pasaría por ahí es qué clave se está
//!   usando y sobre qué;
//! - la firma sale por **stdout**, en hexadecimal y nada más;
//! - lo que haya que contar sale por **stderr**, y `ore` lo muestra literal.
//!
//! # Por qué esto no puede estar dentro de `ore`
//!
//! Porque necesita una **clave privada**, y `ore` no toca credenciales. No es
//! una regla de estilo: es lo que permite que el binario que compila se pueda
//! ejecutar en cualquier sitio —el CI de un tercero, la máquina de quien audita—
//! sin que nadie tenga que preguntarse a qué tiene acceso. Un compilador que
//! pudiera firmar sería un compilador al que hay que confiarle algo.
//!
//! Verificar sí vive dentro, y la asimetría es el punto entero: comprobar una
//! firma no necesita más que bytes que ya están en el árbol.
//!
//! # Y este guarda la clave en un fichero, que es lo justo para empezar
//!
//! Un directorio con `<keyId>.key` dentro, cada uno con una semilla de 32 bytes
//! en hexadecimal. Es el equivalente de que `ore-fetch` lea un directorio: el
//! caso que se puede escribir hoy y que demuestra que el contrato no depende de
//! nada más. Quien necesite un HSM, un KMS o una tarjeta escribe el suyo — y no
//! tiene que cambiar nada de este lado.

use ore_core::firma;
use std::io::Read as _;
use std::process::ExitCode;

const DIR: &str = "ORE_SIGN_DIR";

fn main() -> ExitCode {
    // `--public <keyId>` imprime la clave publica, que es lo que hay que copiar
    // a `trustedKeys`. Va por `argv` y no por stdin a proposito: una clave
    // publica es publica, y pedirla no revela nada que valga la pena ocultar.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.as_slice() {
        [bandera, id] if bandera == "--public" => publica(id),
        [] => firmar(),
        _ => Err("uso: `ore-sign` (la peticion por stdin) o `ore-sign --public <keyId>`".into()),
    };
    match r {
        Ok(salida) => {
            println!("{salida}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-sign: {m}");
            ExitCode::FAILURE
        }
    }
}

fn firmar() -> Result<String, String> {
    let mut entrada = String::new();
    std::io::stdin()
        .read_to_string(&mut entrada)
        .map_err(|e| format!("no se pudo leer stdin: {e}"))?;
    if entrada.trim().is_empty() {
        return Err(
            "no llego nada por stdin. La peticion va por ahi y no por la linea \
                    de ordenes, porque `argv` lo lee cualquier proceso de la maquina"
                .into(),
        );
    }
    let peticion =
        ore_core::parse::parse(&entrada).map_err(|e| format!("la peticion no analiza: {e:?}"))?;
    let campo = |k: &str| {
        peticion
            .get(k)
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    };
    let id = campo("keyId").ok_or("la peticion no dice con que `keyId` firmar")?;
    // El enunciado llega **hecho**, y eso es deliberado: si este programa lo
    // construyera, habria dos definiciones de que se firma y la de quien
    // verifica es la que manda. Aqui se firman los bytes que se reciben.
    let enunciado = campo("statement").ok_or("la peticion no trae el `statement` que firmar")?;

    let kp = par(&id)?;
    Ok(firma::a_hex(
        kp.sk.sign(enunciado.as_bytes(), None).as_ref(),
    ))
}

fn publica(id: &str) -> Result<String, String> {
    Ok(firma::a_hex(par(id)?.pk.as_ref()))
}

/// La clave, de un fichero `<keyId>.key` con una semilla de 32 bytes en hex.
///
/// El `keyId` se usa como nombre de fichero y por eso **no puede llevar
/// separadores**: sin esto, un `keyId` de `../../otro` leeria una clave que no
/// es la que se pidio, y el error no lo veria nadie porque la firma saldria
/// bien.
fn par(id: &str) -> Result<ed25519_compact::KeyPair, String> {
    if id.is_empty() || id.contains(['/', '\\', ':']) || id.contains("..") {
        return Err(format!(
            "`{id}` no sirve como nombre de clave: se usa como nombre de fichero"
        ));
    }
    let dir = std::env::var(DIR).map_err(|_| {
        format!(
            "`{DIR}` no esta definida. Este firmador lee la clave de un directorio: es \
             el caso que se puede escribir sin un HSM, y el que demuestra que el \
             contrato no depende de uno"
        )
    })?;
    let ruta = std::path::Path::new(&dir).join(format!("{id}.key"));
    let texto = std::fs::read_to_string(&ruta)
        .map_err(|e| format!("no se pudo leer `{}`: {e}", ruta.display()))?;
    let semilla = firma::hex(texto.trim())
        .filter(|s| s.len() == 32)
        .ok_or_else(|| {
            format!(
                "`{}` no es una semilla de 32 bytes en hexadecimal",
                ruta.display()
            )
        })?;
    let mut fija = [0u8; 32];
    fija.copy_from_slice(&semilla);
    Ok(ed25519_compact::KeyPair::from_seed(
        ed25519_compact::Seed::new(fija),
    ))
}
