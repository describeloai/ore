//! **La SAS de delegación de usuario** de un blob (ADR 0061 O3): una URL que
//! abre ese blob —y, con `versionid`, esa versión— durante un rato, firmada con
//! la **clave de delegación** que Azure da a quien tiene un token de Entra (no
//! con la clave de la cuenta, que D-O3 no admite y que muchas organizaciones
//! apagan). Sin red: la clave se pide aparte (`Azure::clave_de_delegacion`).
//!
//! La cadena que se firma es la de `sv=2023-11-03` (la disposición de
//! 2020-12-06 en adelante), medida contra Azurite en O3·0: 24 campos, con el
//! tipo y la disposición dentro (`rsct`, `rscd`). Para una versión, `sr=bv` y
//! su id en el hueco de la instantánea; el `versionid` va en la URL y no en el
//! token, como hace el SDK de Azure (`_shared_access_signature.py`).

/// La versión del servicio con la que se firma.
pub const VERSION: &str = "2023-11-03";

/// La clave de delegación, como la da `?restype=service&comp=userdelegationkey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clave {
    /// Los bytes de la clave (`Value`, en base64).
    pub valor: Vec<u8>,
    pub oid: String,
    pub tid: String,
    pub inicio: String,
    pub fin: String,
    pub servicio: String,
    pub version: String,
}

/// Lo que se firma.
pub struct Pedida<'a> {
    pub cuenta: &'a str,
    pub contenedor: &'a str,
    pub blob: &'a str,
    /// La versión (`sr=bv`), o ninguna: el blob vigente (`sr=b`).
    pub version: Option<&'a str>,
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    pub inicio: &'a str,
    pub fin: &'a str,
    pub tipo: &'a str,
    pub disposicion: &'a str,
}

/// La cadena que se firma: 24 campos, uno por línea.
pub fn por_firmar(p: &Pedida, k: &Clave) -> String {
    let recurso = format!("/blob/{}/{}/{}", p.cuenta, p.contenedor, p.blob);
    let sr = if p.version.is_some() { "bv" } else { "b" };
    [
        "r",
        p.inicio,
        p.fin,
        &recurso,
        &k.oid,
        &k.tid,
        &k.inicio,
        &k.fin,
        &k.servicio,
        &k.version,
        "", // saoid
        "", // suoid
        "", // scid
        "", // sip
        "https",
        VERSION,
        sr,
        p.version.unwrap_or(""), // la instantánea, o la versión
        "",                      // ses
        "",                      // rscc
        p.disposicion,
        "", // rsce
        "", // rscl
        p.tipo,
    ]
    .join("\n")
}

/// **La consulta de la URL firmada** (sin el `?`), con el `versionid` si es de
/// una versión.
pub fn consulta(p: &Pedida, k: &Clave) -> String {
    let firma = ore_gcp::base64(&ore_sigv4::firma::hmac(&k.valor, &por_firmar(p, k)));
    let mut q = vec![
        ("sp", "r".to_string()),
        ("st", p.inicio.to_string()),
        ("se", p.fin.to_string()),
        ("skoid", k.oid.clone()),
        ("sktid", k.tid.clone()),
        ("skt", k.inicio.clone()),
        ("ske", k.fin.clone()),
        ("sks", k.servicio.clone()),
        ("skv", k.version.clone()),
        ("spr", "https".into()),
        ("sv", VERSION.into()),
        ("sr", if p.version.is_some() { "bv" } else { "b" }.into()),
        ("rscd", p.disposicion.to_string()),
        ("rsct", p.tipo.to_string()),
        ("sig", firma),
    ];
    if let Some(v) = p.version {
        q.push(("versionid", v.to_string()));
    }
    q.iter()
        .map(|(k, v)| format!("{k}={}", crate::codificar(v, false)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clave() -> Clave {
        Clave {
            valor: b"una clave de delegacion".to_vec(),
            oid: "00000000-0000-0000-0000-000000000001".into(),
            tid: "ab1f708d-50f6-404c-a006-d71b2ac7a606".into(),
            inicio: "2026-10-09T00:00:00Z".into(),
            fin: "2026-10-15T00:00:00Z".into(),
            servicio: "b".into(),
            version: VERSION.into(),
        }
    }

    fn pedida(version: Option<&str>) -> Pedida<'_> {
        Pedida {
            cuenta: "cuenta",
            contenedor: "cubo",
            blob: "docs/a.pdf",
            version,
            inicio: "2026-10-09T10:00:00Z",
            fin: "2026-10-09T11:00:00Z",
            tipo: "application/pdf",
            disposicion: "inline",
        }
    }

    /// La disposición que Azurite aceptó en O3·0: 24 campos, el recurso
    /// canónico, el tipo y la disposición en su sitio.
    #[test]
    fn la_cadena_tiene_los_24_campos_en_su_orden() {
        let s = por_firmar(&pedida(None), &clave());
        let c: Vec<&str> = s.split('\n').collect();
        assert_eq!(c.len(), 24);
        assert_eq!(c[3], "/blob/cuenta/cubo/docs/a.pdf");
        assert_eq!((c[14], c[15], c[16], c[17]), ("https", VERSION, "b", ""));
        assert_eq!((c[20], c[23]), ("inline", "application/pdf"));
    }

    /// Una versión: `sr=bv`, su id firmado en el hueco de la instantánea, y el
    /// `versionid` en la URL (no en el token del SDK, pero sí en la nuestra,
    /// que es la URL entera de la consulta).
    #[test]
    fn una_version_se_firma_con_sr_bv() {
        let v = "2026-10-09T10:00:00.1234567Z";
        let s = por_firmar(&pedida(Some(v)), &clave());
        let c: Vec<&str> = s.split('\n').collect();
        assert_eq!((c[16], c[17]), ("bv", v));
        let q = consulta(&pedida(Some(v)), &clave());
        assert!(
            q.contains("sr=bv") && q.ends_with("&versionid=2026-10-09T10%3A00%3A00.1234567Z"),
            "{q}"
        );
        // la firma es la del HMAC de la cadena, y cambia con la versión
        assert_ne!(q, consulta(&pedida(None), &clave()));
        let sig = q.split("sig=").nth(1).unwrap().split('&').next().unwrap();
        let esperada = ore_gcp::base64(&ore_sigv4::firma::hmac(b"una clave de delegacion", &s));
        assert_eq!(sig, crate::codificar(&esperada, false));
    }
}
