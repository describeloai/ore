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
//! P4·4: los roles y las bases de la rama van en `cluster.roles` y
//! `cluster.databases` (el rol, con su verificador SCRAM: Postgres acepta el
//! verificador como contraseña ya cifrada), y lo borrado y aún no aplicado, en
//! `delta_operations`. `cloud_admin` sigue: es el de dentro (compute_ctl).

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
    /// Los roles y las bases de la rama (P4·4).
    pub datos: &'a Datos,
    /// Las CU de las que salen las conexiones ([`conexiones`]): las máximas del
    /// endpoint o, en una réplica, las del escritor de su rama si son más
    /// ([`CU_DE_LAS_CONEXIONES`]).
    pub cu_conexiones: f64,
}

/// Las CU de las que salen las conexiones de un endpoint `e` (con su alias, en
/// SQL): las suyas, y en una réplica, las del escritor vivo de su rama si son
/// más. ⛔ Postgres no deja a una réplica seguir a un primario con más
/// `max_connections` que ella: pausa la recuperación hasta que se reinicie.
pub const CU_DE_LAS_CONEXIONES: &str =
    "case when e.tipo = 'lectura' then greatest(e.cu_max, coalesce(
      (select max(w.cu_max) from plano.endpoint w
        where w.organizacion = e.organizacion and w.proyecto = e.proyecto and w.rama = e.rama
          and w.tipo = 'lectura-escritura' and w.deseado = 'vivo'), 0))
    else e.cu_max end";

/// **Cuántas conexiones admite Postgres** (`max_connections`) con unas CU: unas
/// 450 por CU (1 CU = 4 GiB), la escala de Neon (0,25 → 112, 1 → 450, 8 →
/// 3600), con un suelo de 100 y un techo de 4000. Sale de las CU **máximas**:
/// `max_connections` sólo cambia al reiniciar, y el escalado no reinicia.
pub fn conexiones(cu: f64) -> i64 {
    ((cu * 450.0).floor() as i64).clamp(100, 4000)
}

/// **El pool de pgbouncer por base**: el 90 % de `max_connections` repartido
/// entre las bases de la rama. pgbouncer no tiene un techo global de
/// conexiones a Postgres, sólo por base (`max_db_connections`): repartido así,
/// la suma nunca pasa de lo que Postgres admite —el cliente que no cabe espera
/// en la cola de pgbouncer— y queda un 10 % para las conexiones directas.
/// `postgres` no cuenta: casi nadie la usa por el pool, y contarla le quitaba
/// la mitad del pool a una rama de una sola base.
pub fn pool_por_base(maximas: i64, bases: usize) -> i64 {
    (maximas * 9 / 10 / (bases.max(1) as i64)).max(1)
}

/// Lo que la rama tiene dentro: roles (nombre, verificador SCRAM) y bases
/// (nombre, dueño), y lo borrado que aún no ha aplicado ningún cómputo.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Datos {
    pub roles: Vec<(String, String)>,
    pub bases: Vec<(String, String)>,
    pub roles_borrados: Vec<String>,
    pub bases_borradas: Vec<String>,
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
    let maximas = conexiones(c.cu_conexiones);
    let pool = pool_por_base(maximas, c.datos.bases.len()).to_string();
    let ajustes = vec![
        ajuste("fsync", "on", "bool"),
        ajuste("wal_level", "logical", "enum"),
        ajuste("wal_log_hints", "on", "bool"),
        ajuste("log_connections", "on", "bool"),
        ajuste("port", "5432", "integer"),
        ajuste("shared_buffers", "128MB", "string"),
        ajuste("max_connections", &maximas.to_string(), "integer"),
        ajuste("listen_addresses", "0.0.0.0", "string"),
        ajuste("max_wal_senders", "10", "integer"),
        ajuste("max_replication_slots", "10", "integer"),
        ajuste("wal_sender_timeout", "5s", "string"),
        ajuste("wal_keep_size", "0", "integer"),
        ajuste("password_encryption", "scram-sha-256", "enum"),
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
    // md5 de cloud_admin = md5('cloud_admin' + 'cloud_admin'), la del compose de Neon: el rol de
    // dentro, el de compute_ctl. Los de los usuarios, abajo, con SCRAM.
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
                (
                    "roles",
                    Json::Arr(
                        std::iter::once(cloud_admin)
                            .chain(c.datos.roles.iter().map(|(n, v)| {
                                Json::obj([
                                    ("name", Json::s(n)),
                                    ("encrypted_password", Json::s(v)),
                                ])
                            }))
                            .collect(),
                    ),
                ),
                (
                    "databases",
                    Json::Arr(
                        c.datos
                            .bases
                            .iter()
                            .map(|(n, d)| Json::obj([("name", Json::s(n)), ("owner", Json::s(d))]))
                            .collect(),
                    ),
                ),
                ("settings", Json::Arr(ajustes)),
            ]),
        ),
        (
            "delta_operations",
            Json::Arr(
                // Las bases antes que los roles: un rol con bases no se puede borrar.
                c.datos
                    .bases_borradas
                    .iter()
                    .map(|n| Json::obj([("action", Json::s("delete_db")), ("name", Json::s(n))]))
                    .chain(c.datos.roles_borrados.iter().map(|n| {
                        Json::obj([("action", Json::s("delete_role")), ("name", Json::s(n))])
                    }))
                    .collect(),
            ),
        ),
    ];
    if let Some(a) = almacen {
        spec.push(("storage_auth_token", Json::s(token_de_tenant(a, c.tenant))));
    }
    if c.replica {
        spec.push(("mode", Json::s("Replica")));
    }
    // P5·5: el pgbouncer de la VM, a la medida (compute_ctl lo escribe en su
    // pgbouncer.ini y hace RELOAD, también en un `/configure`: sin reiniciar).
    spec.push((
        "pgbouncer_settings",
        Json::obj([
            ("default_pool_size", Json::s(pool.clone())),
            ("max_db_connections", Json::s(pool)),
        ]),
    ));
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
                datos: &Datos {
                    roles: vec![("ana".into(), "SCRAM-SHA-256$4096:x$y:z".into())],
                    bases: vec![("ventas".into(), "ana".into())],
                    roles_borrados: vec!["viejo".into()],
                    bases_borradas: vec![],
                },
                cu_conexiones: 1.0,
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
            r#"{"encrypted_password":"SCRAM-SHA-256$4096:x$y:z","name":"ana"}"#,
            r#""databases":[{"name":"ventas","owner":"ana"}]"#,
            r#""delta_operations":[{"action":"delete_role","name":"viejo"}]"#,
            // 1 CU: 450 conexiones; el 90 % para `ventas`, la única base.
            r#""name":"max_connections","value":"450""#,
            r#""pgbouncer_settings":{"default_pool_size":"405","max_db_connections":"405"}"#,
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
                datos: &Datos::default(),
                cu_conexiones: 0.25,
            },
            &l,
            None,
        )
        .jcs();
        assert!(!sin.contains("storage_auth_token"));
        assert!(
            sin.contains(r#""name":"max_connections","value":"112""#),
            "{sin}"
        );
    }

    #[test]
    fn las_conexiones_crecen_con_las_cu_y_el_pool_no_pasa_de_ellas() {
        assert_eq!(conexiones(0.25), 112);
        assert_eq!(conexiones(1.0), 450);
        assert_eq!(conexiones(8.0), 3600);
        assert_eq!(conexiones(16.0), 4000);
        assert_eq!(conexiones(0.1), 100);
        for (maximas, bases) in [(112, 0), (112, 1), (112, 5), (450, 2), (4000, 30)] {
            let pool = pool_por_base(maximas, bases);
            assert!(pool >= 1);
            // Todas las bases a la vez, llenas, caben en el 90 %.
            assert!(
                pool * (bases.max(1) as i64) <= maximas * 9 / 10,
                "{maximas} {bases}"
            );
        }
        assert_eq!(pool_por_base(112, 0), 100);
        assert_eq!(pool_por_base(112, 1), 100);
        assert_eq!(pool_por_base(112, 2), 50);
    }
}
