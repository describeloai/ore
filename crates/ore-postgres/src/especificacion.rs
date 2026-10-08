//! **La especificación de un cómputo** (0058 P4·3·1): lo que `compute_ctl` lee
//! al arrancar (`config.json`). Sustituye a `especificacion.py` de las pruebas,
//! con lo medido en P2 y P3 dentro:
//!
//! - los safekeepers por su Service ClusterIP de cada pod, que da una IP estable
//!   (P2·6: el DNS de pod de un headless dejaba un SYN colgado ~127 s);
//! - el pageserver **del tenant**, preguntado al controller, no uno fijo;
//! - el token de scope `tenant` firmado con la llave del almacenamiento (P2·4);
//! - el JWKS de la llave propia de `ore-postgres`, para que `compute_ctl` acepte
//!   sus órdenes (`/configure`, `/status`), y `compute_id` igual al nombre.
//!
//! `cloud_admin` con la contraseña de las pruebas es de P4·3: los roles de
//! verdad, con SCRAM, son P4·4.

use crate::llaves::{Llave, token_de_tenant};
use ore_core::json::Json;

/// Lo que hace distinto a un cómputo de otro.
pub struct Computo<'a> {
    /// El nombre de la VM; es también su `compute_id`.
    pub nombre: &'a str,
    pub tenant: &'a str,
    pub timeline: &'a str,
    /// `host=… port=…` del pageserver que lleva el tenant ([`crate::almacen::Almacen::pageserver_de`]).
    pub pageserver: &'a str,
    /// `host:5454` de cada safekeeper.
    pub safekeepers: &'a [String],
    /// A qué agrupa (en Neon, el proyecto): va en `cluster_id`.
    pub grupo: &'a str,
    /// RFC 3339 del momento en que se escribe.
    pub ahora: &'a str,
    /// Un endpoint de sólo lectura: una réplica en caliente que sigue la rama
    /// (`"mode": "Replica"` de Neon), sin votar en los safekeepers.
    pub replica: bool,
}

fn ajuste(nombre: &str, valor: &str, tipo: &str) -> Json {
    Json::obj([
        ("name", Json::s(nombre)),
        ("value", Json::s(valor)),
        ("vartype", Json::s(tipo)),
    ])
}

/// La especificación entera. `almacen` es `None` sólo si el almacenamiento no
/// exige autenticación (las pruebas locales de P2·3).
pub fn especificacion(c: &Computo, propia: &Llave, almacen: Option<&Llave>) -> Json {
    let sk = c.safekeepers.join(",");
    let ajustes = vec![
        ajuste("fsync", "on", "bool"),
        ajuste("wal_level", "logical", "enum"),
        ajuste("wal_log_hints", "on", "bool"),
        ajuste("log_connections", "on", "bool"),
        ajuste("port", "55433", "integer"),
        ajuste("shared_buffers", "128MB", "string"),
        ajuste("max_connections", "100", "integer"),
        ajuste("listen_addresses", "0.0.0.0", "string"),
        ajuste("max_wal_senders", "10", "integer"),
        ajuste("max_replication_slots", "10", "integer"),
        ajuste("wal_sender_timeout", "5s", "string"),
        ajuste("wal_keep_size", "0", "integer"),
        ajuste("password_encryption", "md5", "enum"),
        ajuste("restart_after_crash", "off", "bool"),
        ajuste("synchronous_standby_names", "walproposer", "string"),
        ajuste(
            "shared_preload_libraries",
            "neon,pg_cron,timescaledb,pg_stat_statements",
            "string",
        ),
        ajuste("neon.safekeepers", &sk, "string"),
        ajuste("neon.tenant_id", c.tenant, "string"),
        ajuste("neon.timeline_id", c.timeline, "string"),
        ajuste("neon.pageserver_connstring", c.pageserver, "string"),
        ajuste("max_replication_write_lag", "500MB", "string"),
        ajuste("max_replication_flush_lag", "10GB", "string"),
        ajuste("cron.database", "postgres", "string"),
        ajuste("neon.max_file_cache_size", "1GB", "string"),
        ajuste("neon.file_cache_size_limit", "1GB", "string"),
        ajuste(
            "neon.file_cache_path",
            "/var/db/postgres/compute/file.cache",
            "string",
        ),
    ];
    // md5 de cloud_admin = md5('cloud_admin' + 'cloud_admin'), la del compose de Neon (P4·4 la quita).
    let cloud_admin = Json::obj([
        ("name", Json::s("cloud_admin")),
        (
            "encrypted_password",
            Json::s("b093c0d3b281ba6da1eacc608620abd8"),
        ),
    ]);
    let mut spec = vec![
        ("format_version", Json::Int(1)),
        ("timestamp", Json::s(c.ahora)),
        (
            "operation_uuid",
            Json::s("00000000-0000-0000-0000-000000000000"),
        ),
        ("suspend_timeout_seconds", Json::Int(60)),
        (
            "cluster",
            Json::obj([
                ("cluster_id", Json::s(c.grupo)),
                ("name", Json::s(c.nombre)),
                ("state", Json::s("restarted")),
                ("roles", Json::Arr(vec![cloud_admin])),
                ("databases", Json::Arr(vec![])),
                ("settings", Json::Arr(ajustes)),
            ]),
        ),
        ("delta_operations", Json::Arr(vec![])),
    ];
    if let Some(a) = almacen {
        spec.push(("storage_auth_token", Json::s(token_de_tenant(a, c.tenant))));
    }
    if c.replica {
        spec.push(("mode", Json::s("Replica")));
    }
    Json::obj([
        ("spec", Json::obj(spec)),
        (
            "compute_ctl_config",
            Json::obj([("jwks", Json::obj([("keys", Json::Arr(vec![propia.jwk()]))]))]),
        ),
    ])
}

/// RFC 3339 en UTC, con milisegundos a cero, de unos segundos desde 1970 (el
/// algoritmo civil de Howard Hinnant: sin tablas y sin dependencias).
pub fn iso(seg: i64) -> String {
    let (dias, s) = (seg.div_euclid(86_400), seg.rem_euclid(86_400));
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let a = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{a:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_fecha() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso(1_791_453_600), "2026-10-08T10:00:00.000Z");
    }

    const PEM: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC
-----END PRIVATE KEY-----";

    #[test]
    fn lleva_lo_que_compute_ctl_necesita() {
        let l = Llave::de_pem(PEM).unwrap();
        let sk = vec!["sk-0:5454".to_string(), "sk-1:5454".to_string()];
        let e = especificacion(
            &Computo {
                nombre: "ep-1",
                tenant: "t1",
                timeline: "tl1",
                pageserver: "host=ps port=6400",
                safekeepers: &sk,
                grupo: "ventas",
                ahora: "2026-10-08T10:00:00.000Z",
                replica: false,
            },
            &l,
            Some(&l),
        )
        .jcs();
        for esperado in [
            r#""name":"neon.safekeepers","value":"sk-0:5454,sk-1:5454""#,
            r#""name":"neon.tenant_id","value":"t1""#,
            r#""name":"neon.timeline_id","value":"tl1""#,
            r#""name":"neon.pageserver_connstring","value":"host=ps port=6400""#,
            r#""name":"ep-1""#,
            r#""storage_auth_token":"ey"#,
            &format!(r#""kid":"{}""#, l.kid),
        ] {
            assert!(e.contains(esperado), "falta {esperado} en {e}");
        }
        // Sin autenticación del almacenamiento, sin token.
        let sin = especificacion(
            &Computo {
                nombre: "ep-1",
                tenant: "t1",
                timeline: "tl1",
                pageserver: "host=ps port=6400",
                safekeepers: &sk,
                grupo: "ventas",
                ahora: "2026-10-08T10:00:00.000Z",
                replica: false,
            },
            &l,
            None,
        )
        .jcs();
        assert!(!sin.contains("storage_auth_token"));
    }
}
