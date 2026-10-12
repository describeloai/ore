//! **El SQL de la consola** (ADR 0058, P8·1, D-P8a): `POST …/ramas/{r}/sql`.
//!
//! Corre como el rol que se elige, con su contraseña (guardada cifrada, [`crate::secreto`]),
//! contra el cómputo de escritura de la rama, al que se conecta directamente
//! (`ip_pod:5432`, por la red de pods, como `/configure` al 3080). Si duerme, se le
//! despierta como lo hace el proxy, y la consulta cuenta como actividad.
//!
//! ⭐ El SQL lo ejecuta un **proceso hijo** (`ore-postgres ejecutar-sql`), con un tope de
//!   memoria propio (`RLIMIT_DATA`): el cliente de Postgres guarda en memoria todo el
//!   resultado antes de devolverlo, y un `select` sin límite sobre una tabla grande
//!   tumbaría al plano entero (su límite es de 128 MiB). Así revienta el hijo, y el
//!   plano contesta que no cabe.
//!
//! Lo que el hijo lee por la entrada estándar y escribe por la salida:
//!
//!   → {"host", "puerto", "base", "rol", "contrasena", "sql", "solo_lectura", "limite_filas"}
//!   ← una línea de resumen  {"filas": N, "error": "<sqlstate>"|null}
//!     y otra con la respuesta {"ok": true, "sentencias": […]} | {"ok": false, "error": {…}}
use crate::celda::Celda;
use crate::secreto::Secreto;
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::Respuesta;
use postgres::Client;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Lo más que se espera a que despierte el cómputo (como el proxy, P6·4).
const PLAZO_DESPERTAR: Duration = Duration::from_secs(120);
/// Lo más que puede durar una sentencia (`statement_timeout`).
const PLAZO_SENTENCIA_MS: u64 = 60_000;
/// Lo más que se espera al hijo: la sentencia, conectar y escribir.
const PLAZO_HIJO: Duration = Duration::from_secs(75);
/// Las filas y los bytes que se devuelven, como mucho, entre todas las sentencias.
pub const LIMITE_FILAS: usize = 1_000;
const LIMITE_BYTES: usize = 5 * 1024 * 1024;
/// El tope de memoria del hijo. Con el plano (unos MB) cabe en los 128 MiB del pod.
const MEMORIA_HIJO: u64 = 64 * 1024 * 1024;
/// Lo más largo que puede ser el SQL que llega.
const LIMITE_SQL: usize = 1024 * 1024;

pub struct Consola {
    /// La clave de las contraseñas (`--clave-contrasenas`).
    pub secreto: Secreto,
    /// El binario que hace de hijo: el propio `ore-postgres`.
    pub ejecutable: std::path::PathBuf,
    /// El puerto de Postgres en el cómputo (5432; las pruebas, el suyo).
    pub puerto: u16,
}

/// Lo que se pide, del cuerpo.
struct Pedido {
    base: String,
    rol: String,
    sql: String,
    solo_lectura: bool,
    limite_filas: usize,
}

fn pedido(cuerpo: &Node) -> Result<Pedido, Respuesta> {
    let texto = |k: &str| {
        cuerpo
            .get(k)
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
    };
    let falta = |k: &str| Respuesta::error(400, format!("falta `{k}`"));
    let base = texto("base").ok_or_else(|| falta("base"))?;
    let rol = texto("rol").ok_or_else(|| falta("rol"))?;
    let sql = texto("sql").ok_or_else(|| falta("sql"))?;
    if sql.trim().is_empty() {
        return Err(Respuesta::error(400, "`sql` está vacío"));
    }
    if sql.len() > LIMITE_SQL {
        return Err(Respuesta::error(413, "el SQL pasa de 1 MiB"));
    }
    let solo_lectura = texto("solo_lectura").as_deref() == Some("true");
    let limite_filas = match texto("limite_filas") {
        None => LIMITE_FILAS,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if (1..=LIMITE_FILAS).contains(&n) => n,
            _ => {
                return Err(Respuesta::error(
                    400,
                    format!("`limite_filas` va de 1 a {LIMITE_FILAS}"),
                ));
            }
        },
    };
    Ok(Pedido {
        base,
        rol,
        sql,
        solo_lectura,
        limite_filas,
    })
}

impl Consola {
    /// `POST …/proyectos/{p}/ramas/{r}/sql`. Toma la base sólo cuando la necesita: el
    /// despertar espera, y mientras espera la suelta (la API entera la comparte).
    pub fn ejecutar(
        &self,
        base: &Mutex<Client>,
        url: Option<&str>,
        celda: &Celda,
        (p, r): (&str, &str),
        cuerpo: &Node,
        quien: Option<&str>,
    ) -> Respuesta {
        let pd = match pedido(cuerpo) {
            Ok(x) => x,
            Err(e) => return e,
        };
        // ① El cómputo de escritura de la rama y la contraseña del rol.
        let datos = crate::proxy::con_base(base, url, |c| -> Result<(String, String), Respuesta> {
            let fallo = |e: postgres::Error| Respuesta::error(500, crate::base::mal(e));
            if c.query_opt(
                "select 1 from plano.proyecto where organizacion = $1 and id = $2 and deseado = 'vivo'",
                &[&celda.organizacion, &p],
            )
            .map_err(fallo)?
            .is_none()
            {
                return Err(Respuesta::error(404, format!("no hay ningún proyecto `{p}`")));
            }
            let Some(vm) = c
                .query_opt(
                    "select vm from plano.endpoint
                      where organizacion = $1 and proyecto = $2 and rama = $3
                        and tipo = 'lectura-escritura' and deseado = 'vivo'",
                    &[&celda.organizacion, &p, &r],
                )
                .map_err(fallo)?
            else {
                return Err(Respuesta::error(
                    409,
                    format!(
                        "la rama `{r}` no tiene un cómputo de escritura: créalo para ejecutar SQL"
                    ),
                ));
            };
            let Some(rol) = c
                .query_opt(
                    "select contrasena_cifrada from plano.rol
                      where organizacion = $1 and proyecto = $2 and rama = $3 and nombre = $4
                        and deseado = 'vivo'",
                    &[&celda.organizacion, &p, &r, &pd.rol],
                )
                .map_err(fallo)?
            else {
                return Err(Respuesta::error(
                    404,
                    format!("no hay ningún rol `{}` en `{r}`", pd.rol),
                ));
            };
            let Some(cifrada) = rol.get::<_, Option<String>>(0) else {
                return Err(sin_contrasena(&pd.rol));
            };
            Ok((vm.get(0), cifrada))
        });
        let (vm, cifrada) = match datos {
            Ok(Ok(x)) => x,
            Ok(Err(r)) | Err(r) => return r,
        };
        let contrasena = match self.secreto.descifrar(
            &cifrada,
            &crate::secreto::asociado(&celda.organizacion, p, &pd.rol),
        ) {
            Ok(c) => c,
            Err(e) => return Respuesta::error(500, e),
        };
        // ② Despierto, como lo despierta el proxy.
        let t0 = Instant::now();
        let ip = match self.despierto(base, url, &vm) {
            Ok(ip) => ip,
            Err(r) => return r,
        };
        let despertar_ms = t0.elapsed().as_millis() as i64;
        // ③ El hijo.
        let t1 = Instant::now();
        let salida = self.hijo(&ip, &pd, &contrasena);
        let ms = t1.elapsed().as_millis() as i64;
        let (filas, error, cuerpo) = match salida {
            Ok(x) => x,
            Err(m) => (0, Some("hijo".to_string()), Err(m)),
        };
        // ④ Apuntado, sin el texto.
        let _ = crate::proxy::con_base(base, url, |c| {
            c.execute(
                "insert into plano.sql_ejecutado
                   (quien, organizacion, proyecto, rama, rol, base, ms, filas, error)
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[
                    &quien,
                    &celda.organizacion,
                    &p,
                    &r,
                    &pd.rol,
                    &pd.base,
                    &(ms as i32),
                    &(filas as i32),
                    &error,
                ],
            )
        });
        match cuerpo {
            Ok(json) => Respuesta {
                codigo: 200,
                cuerpo: Json::Crudo(con_tiempos(&json, ms, despertar_ms)),
            },
            Err(m) => Respuesta::error(502, m),
        }
    }

    /// Hasta que el cómputo esté `listo`; su IP de pod.
    fn despierto(
        &self,
        base: &Mutex<Client>,
        url: Option<&str>,
        vm: &str,
    ) -> Result<String, Respuesta> {
        let desde = Instant::now();
        loop {
            match crate::proxy::con_base(base, url, |c| crate::proxy::mirar(c, vm, self.puerto)) {
                Ok(crate::proxy::Paso::Contestar(r)) if r.codigo == 200 => break,
                Ok(crate::proxy::Paso::Contestar(r)) | Err(r) => return Err(r),
                Ok(crate::proxy::Paso::Esperar) if desde.elapsed() >= PLAZO_DESPERTAR => {
                    return Err(Respuesta::error(
                        503,
                        "el cómputo aún está despertando: reintenta en unos segundos",
                    ));
                }
                Ok(crate::proxy::Paso::Esperar) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let ip = crate::proxy::con_base(base, url, |c| {
            c.query_opt(
                "select ip_pod from plano.endpoint where vm = $1 and deseado = 'vivo'",
                &[&vm],
            )
        });
        match ip {
            Ok(Ok(Some(f))) => f
                .get::<_, Option<String>>(0)
                .ok_or_else(|| Respuesta::error(503, "el cómputo aún no dice su dirección")),
            Ok(Ok(None)) => Err(Respuesta::error(404, "el endpoint ya no está")),
            Ok(Err(e)) => Err(Respuesta::error(500, crate::base::mal(e))),
            Err(r) => Err(r),
        }
    }

    /// Lanza el hijo, le da el pedido y lee lo que contesta: (filas, error, respuesta).
    #[allow(clippy::type_complexity)]
    fn hijo(
        &self,
        ip: &str,
        pd: &Pedido,
        contrasena: &str,
    ) -> Result<(i64, Option<String>, Result<String, String>), String> {
        let entrada = Json::obj([
            ("host", Json::s(ip)),
            ("puerto", Json::Int(self.puerto as i64)),
            ("base", Json::s(&pd.base)),
            ("rol", Json::s(&pd.rol)),
            ("contrasena", Json::s(contrasena)),
            ("sql", Json::s(&pd.sql)),
            ("solo_lectura", Json::Bool(pd.solo_lectura)),
            ("limite_filas", Json::Int(pd.limite_filas as i64)),
        ])
        .jcs();
        let mut hijo = std::process::Command::new(&self.ejecutable)
            .arg("ejecutar-sql")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo lanzar el hijo: {e}"))?;
        if let Some(mut e) = hijo.stdin.take() {
            let _ = e.write_all(entrada.as_bytes());
        }
        let mut leer = hijo.stdout.take().expect("salida en tubería");
        let lector = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = (&mut leer)
                .take((LIMITE_BYTES * 3) as u64)
                .read_to_end(&mut b);
            b
        });
        let desde = Instant::now();
        let estado = loop {
            match hijo.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if desde.elapsed() >= PLAZO_HIJO => {
                    let _ = hijo.kill();
                    let _ = hijo.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(format!("esperando al hijo: {e}")),
            }
        };
        let b = lector.join().unwrap_or_default();
        let Some(estado) = estado else {
            return Ok((
                0,
                Some("plazo".into()),
                Err("la consulta pasó de 75 s".into()),
            ));
        };
        let texto = String::from_utf8_lossy(&b);
        let mut lineas = texto.splitn(2, '\n');
        let (Some(resumen), Some(respuesta)) = (lineas.next(), lineas.next()) else {
            // Sin salida: murió. Lo normal es el tope de memoria (un resultado enorme).
            return Ok((
                0,
                Some("memoria".into()),
                Err(format!(
                    "el resultado no cabe en memoria ({} MiB) o la ejecución se cortó ({estado}): acótalo, p. ej. con LIMIT",
                    MEMORIA_HIJO / 1024 / 1024
                )),
            ));
        };
        let r = ore_core::parse::parse(resumen).map_err(|_| "el resumen del hijo no es JSON")?;
        let filas = r
            .get("filas")
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let error = r
            .get("error")
            .and_then(|(_, v)| v.as_str())
            .filter(|v| *v != "null" && !v.is_empty())
            .map(str::to_string);
        Ok((filas, error, Ok(respuesta.trim_end().to_string())))
    }
}

/// 409 con lo que hay que hacer: los roles de antes de P8·1 no tienen su contraseña.
pub fn sin_contrasena(rol: &str) -> Respuesta {
    Respuesta::error(
        409,
        format!(
            "la contraseña de `{rol}` no está guardada (se creó antes de la consola): regenérala una vez para usarlo aquí"
        ),
    )
}

/// `{"ms":…,"despertar_ms":…,` delante de lo que contestó el hijo (un objeto).
fn con_tiempos(json: &str, ms: i64, despertar_ms: i64) -> String {
    let resto = json.trim_start().strip_prefix('{').unwrap_or("}");
    let coma = if resto.trim_start().starts_with('}') {
        ""
    } else {
        ","
    };
    format!("{{\"despertar_ms\":{despertar_ms},\"ms\":{ms}{coma}{resto}")
}

// ── el hijo ────────────────────────────────────────────────────────────────

/// `ore-postgres ejecutar-sql`: lee el pedido, se pone su tope de memoria, ejecuta y
/// escribe el resumen y la respuesta. Siempre sale con 0 si llegó a escribir.
pub fn hijo_principal() -> std::process::ExitCode {
    tope_de_memoria(MEMORIA_HIJO);
    let mut entrada = String::new();
    if std::io::stdin().read_to_string(&mut entrada).is_err() {
        return std::process::ExitCode::from(66);
    }
    let (filas, error, respuesta) = match ore_core::parse::parse(&entrada) {
        Ok(n) => ejecutar_en_el_hijo(&n),
        Err(_) => (
            0,
            None,
            error_json(None, "el pedido no es JSON", None, None, None),
        ),
    };
    let resumen = Json::obj([
        ("filas", Json::Int(filas as i64)),
        (
            "error",
            error.map(Json::s).unwrap_or(Json::Crudo("null".into())),
        ),
    ]);
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", resumen.jcs());
    let _ = writeln!(out, "{}", respuesta.jcs());
    std::process::ExitCode::SUCCESS
}

#[cfg(unix)]
fn tope_de_memoria(bytes: u64) {
    let l = libc::rlimit {
        rlim_cur: bytes as libc::rlim_t,
        rlim_max: bytes as libc::rlim_t,
    };
    // SAFETY: setrlimit sólo lee la estructura que se le pasa.
    unsafe {
        libc::setrlimit(libc::RLIMIT_DATA, &l);
    }
}

#[cfg(not(unix))]
fn tope_de_memoria(_: u64) {}

fn error_json(
    sqlstate: Option<&str>,
    mensaje: &str,
    detalle: Option<&str>,
    pista: Option<&str>,
    posicion: Option<u32>,
) -> Json {
    let nulo = || Json::Crudo("null".into());
    Json::obj([
        ("ok", Json::Bool(false)),
        (
            "error",
            Json::obj([
                ("sqlstate", sqlstate.map(Json::s).unwrap_or_else(nulo)),
                ("mensaje", Json::s(mensaje)),
                ("detalle", detalle.map(Json::s).unwrap_or_else(nulo)),
                ("pista", pista.map(Json::s).unwrap_or_else(nulo)),
                (
                    "posicion",
                    posicion.map(|p| Json::Int(p as i64)).unwrap_or_else(nulo),
                ),
            ]),
        ),
    ])
}

/// (filas devueltas, sqlstate del error, la respuesta).
fn ejecutar_en_el_hijo(n: &Node) -> (usize, Option<String>, Json) {
    let t = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let puerto: u16 = t("puerto").parse().unwrap_or(5432);
    let limite: usize = t("limite_filas").parse().unwrap_or(LIMITE_FILAS);
    let mut opciones = format!(
        "-c statement_timeout={PLAZO_SENTENCIA_MS} -c idle_in_transaction_session_timeout={PLAZO_SENTENCIA_MS} -c application_name=ore-consola"
    );
    if t("solo_lectura") == "true" {
        opciones.push_str(" -c default_transaction_read_only=on");
    }
    let conectado = postgres::Config::new()
        .host(&t("host"))
        .port(puerto)
        .user(&t("rol"))
        .password(t("contrasena"))
        .dbname(&t("base"))
        .options(&opciones)
        .connect_timeout(Duration::from_secs(10))
        .connect(postgres::NoTls);
    let mut c = match conectado {
        Ok(c) => c,
        Err(e) => return de_error(&e, "no se pudo conectar"),
    };
    let mensajes = match c.simple_query(&t("sql")) {
        Ok(m) => m,
        Err(e) => return de_error(&e, "falló"),
    };
    let (mut filas_total, mut bytes) = (0usize, 0usize);
    let mut sentencias = Vec::new();
    let mut columnas: Option<Vec<String>> = None;
    let mut filas: Vec<Json> = Vec::new();
    let mut truncada = false;
    for m in mensajes {
        match m {
            postgres::SimpleQueryMessage::RowDescription(cols) => {
                columnas = Some(cols.iter().map(|c| c.name().to_string()).collect());
                filas.clear();
                truncada = false;
            }
            postgres::SimpleQueryMessage::Row(f) => {
                let ancho: usize = (0..f.len()).map(|i| f.get(i).map_or(4, str::len)).sum();
                if filas_total >= limite || bytes + ancho > LIMITE_BYTES {
                    truncada = true;
                    continue;
                }
                filas_total += 1;
                bytes += ancho;
                filas.push(Json::Arr(
                    (0..f.len())
                        .map(|i| f.get(i).map(Json::s).unwrap_or(Json::Crudo("null".into())))
                        .collect(),
                ));
            }
            postgres::SimpleQueryMessage::CommandComplete(n) => {
                let mut s = vec![("filas_afectadas", Json::Int(n as i64))];
                if let Some(cols) = columnas.take() {
                    s.push((
                        "columnas",
                        Json::Arr(cols.into_iter().map(Json::s).collect()),
                    ));
                    s.push(("filas", Json::Arr(std::mem::take(&mut filas))));
                    s.push(("truncada", Json::Bool(truncada)));
                }
                sentencias.push(Json::obj(s));
                truncada = false;
            }
            _ => {}
        }
    }
    (
        filas_total,
        None,
        Json::obj([
            ("ok", Json::Bool(true)),
            ("sentencias", Json::Arr(sentencias)),
        ]),
    )
}

fn de_error(e: &postgres::Error, que: &str) -> (usize, Option<String>, Json) {
    match e.as_db_error() {
        Some(d) => {
            let posicion = match d.position() {
                Some(postgres::error::ErrorPosition::Original(p)) => Some(*p),
                _ => None,
            };
            (
                0,
                Some(d.code().code().to_string()),
                error_json(
                    Some(d.code().code()),
                    d.message(),
                    d.detail(),
                    d.hint(),
                    posicion,
                ),
            )
        }
        None => (
            0,
            Some("conexion".into()),
            error_json(None, &format!("{que}: {e}"), None, None, None),
        ),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_tiempos_van_delante() {
        assert_eq!(
            con_tiempos(r#"{"ok":true,"sentencias":[]}"#, 12, 0),
            r#"{"despertar_ms":0,"ms":12,"ok":true,"sentencias":[]}"#
        );
        assert_eq!(con_tiempos("{}", 1, 2), r#"{"despertar_ms":2,"ms":1}"#);
    }

    #[test]
    fn el_pedido_se_valida() {
        let n = |s: &str| ore_core::parse::parse(s).unwrap();
        assert!(pedido(&n(r#"{"base":"b","rol":"r"}"#)).is_err());
        assert!(pedido(&n(r#"{"base":"b","rol":"r","sql":"  "}"#)).is_err());
        assert!(
            pedido(&n(
                r#"{"base":"b","rol":"r","sql":"select 1","limite_filas":5000}"#
            ))
            .is_err()
        );
        let p = pedido(&n(
            r#"{"base":"b","rol":"r","sql":"select 1","solo_lectura":true}"#,
        ))
        .ok()
        .unwrap();
        assert!(p.solo_lectura);
        assert_eq!(p.limite_filas, LIMITE_FILAS);
    }
}
