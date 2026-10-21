//! **La GSSAPI del sistema** como [`Contexto`] (ficha 0072, N6): `libgssapi` sobre `libgssapi_krb5`. Medido en X0b (§3.5)
//! con Debian y con Alpine/musl; en Alpine el binario se enlaza dinamicamente (`-C target-feature=-crt-static`).
//!
//! Detras del *feature* `kerberos`: sin el, este modulo no existe y el crate no pide ninguna biblioteca del sistema.

use crate::error::{Error, Resultado};
use crate::sasl::{Contexto, Kerberos};
use libgssapi::context::{ClientCtx, CtxFlags, SecurityContext};
use libgssapi::credential::{Cred, CredUsage};
use libgssapi::name::Name;
use libgssapi::oid::{GSS_MECH_KRB5, GSS_NT_HOSTBASED_SERVICE, GSS_NT_KRB5_PRINCIPAL, OidSet};
use std::sync::OnceLock;

fn de_gss(que: &str, e: libgssapi::error::Error) -> Error {
    Error::Autenticacion(format!("Kerberos: {que}: {e}"))
}

/// El *keytab* del proceso. La GSSAPI lo lee de `KRB5_CLIENT_KTNAME`, que es del proceso entero: **uno por proceso**.
/// Se fija una vez, antes de pedir ninguna credencial; otro distinto despues es un error y no un cambio silencioso.
static KEYTAB: OnceLock<String> = OnceLock::new();

fn fijar_keytab(ruta: &str) -> Resultado<()> {
    let puesto = KEYTAB.get_or_init(|| {
        let valor = if ruta.contains(':') {
            ruta.to_string()
        } else {
            format!("FILE:{ruta}")
        };
        // SAFETY: se ejecuta una sola vez (OnceLock), antes de que la GSSAPI lea el entorno por primera vez en este
        // proceso; ningun otro hilo de este crate toca el entorno.
        unsafe { std::env::set_var("KRB5_CLIENT_KTNAME", &valor) };
        ruta.to_string()
    });
    if puesto == ruta {
        Ok(())
    } else {
        Err(Error::Peticion(format!(
            "Kerberos: este proceso ya usa el keytab `{puesto}`; un proceso lee con un solo keytab (pedido: `{ruta}`)"
        )))
    }
}

struct Gss {
    ctx: ClientCtx,
}

/// El contexto de Kerberos contra `host`, con el servicio `servicio/host`.
pub fn nuevo(host: &str, k: &Kerberos) -> Resultado<Box<dyn Contexto>> {
    if let Some(kt) = &k.keytab {
        fijar_keytab(kt)?;
    }
    let mut mecanismos = OidSet::new().map_err(|e| de_gss("mecanismos", e))?;
    mecanismos
        .add(&GSS_MECH_KRB5)
        .map_err(|e| de_gss("mecanismos", e))?;
    let servicio = Name::new(
        format!("{}@{host}", k.servicio).as_bytes(),
        Some(&GSS_NT_HOSTBASED_SERVICE),
    )
    .and_then(|n| n.canonicalize(Some(&GSS_MECH_KRB5)))
    .map_err(|e| de_gss(&format!("el servicio `{}@{host}`", k.servicio), e))?;
    let yo = match &k.principal {
        Some(p) => Some(
            Name::new(p.as_bytes(), Some(&GSS_NT_KRB5_PRINCIPAL))
                .and_then(|n| n.canonicalize(Some(&GSS_MECH_KRB5)))
                .map_err(|e| de_gss(&format!("el principal `{p}`"), e))?,
        ),
        None => None,
    };
    let cred =
        Cred::acquire(yo.as_ref(), None, CredUsage::Initiate, Some(&mecanismos)).map_err(|e| {
            de_gss(
                "no hay credenciales (¿`kinit`, o `keytab=` y `principal=` en la URL?)",
                e,
            )
        })?;
    let flags = CtxFlags::GSS_C_MUTUAL_FLAG
        | CtxFlags::GSS_C_SEQUENCE_FLAG
        | CtxFlags::GSS_C_INTEG_FLAG
        | CtxFlags::GSS_C_CONF_FLAG;
    Ok(Box::new(Gss {
        ctx: ClientCtx::new(Some(cred), servicio, flags, Some(&GSS_MECH_KRB5)),
    }))
}

impl Contexto for Gss {
    fn paso(&mut self, reto: Option<&[u8]>) -> Resultado<Option<Vec<u8>>> {
        self.ctx
            .step(reto, None)
            .map(|t| t.map(|b| b.to_vec()))
            .map_err(|e| de_gss("el apreton con el servidor", e))
    }

    fn completo(&self) -> bool {
        self.ctx.is_complete()
    }

    fn envolver(&mut self, confidencial: bool, datos: &[u8]) -> Resultado<Vec<u8>> {
        self.ctx
            .wrap(confidencial, datos)
            .map(|b| b.to_vec())
            .map_err(|e| de_gss("envolver un mensaje", e))
    }

    fn desenvolver(&mut self, datos: &[u8]) -> Resultado<Vec<u8>> {
        self.ctx
            .unwrap(datos)
            .map(|b| b.to_vec())
            .map_err(|e| de_gss("desenvolver un mensaje", e))
    }
}
