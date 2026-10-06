//! **El kit de la pasarela** (ADR 0053 F3·2): `ore-federation` de verdad,
//! delante del conector y del banco, y los ocho casos que la hacen merecer su
//! sitio.
//!
//! | caso | qué |
//! |---|---|
//! | 1 | el mismo flujo que el conector directo, lote a lote |
//! | 2 | caliente: varias lecturas, un proceso |
//! | 3 | el presupuesto: corta en la fila exacta, por bytes y por tiempo, y el origen queda limpio |
//! | 4 | la cola: muchas a la vez, nunca más de `concurrencia` en el origen; llena, `503` |
//! | 5 | cancelar: por `DELETE` y por desconexión, también en el origen |
//! | 6 | la credencial no sale: ni registro, ni `argv`, ni entorno, ni respuestas |
//! | 7 | un conector ocioso se cierra, y su conexión con él |
//! | 8 | lo que falla antes del primer byte dice su estado HTTP y su código |
//!
//! La pasarela corre con cotas cortas para que el kit sea rápido
//! (`ORE_FED_ESPERA_MS=2000`, `ORE_FED_OCIOSA_MS=1500`); las demás, las de
//! verdad.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arrow_array::RecordBatch;
use ore_core::json::Json;

use crate::bancos::Banco;
use crate::casos::{Estado, Resultado};
use crate::conector::Conector;
use crate::semilla::Tabla;

pub const CASOS: [(u8, &str); 10] = [
    (1, "el mismo flujo que el conector"),
    (2, "caliente"),
    (3, "el presupuesto"),
    (4, "la cola por origen"),
    (5, "cancelar"),
    (6, "la credencial no sale"),
    (7, "el ocioso se cierra"),
    (8, "errores antes del primer byte"),
    (9, "una vía: los otros verbos y la copia"),
    (10, "las colecciones por la pasarela"),
];

const CONCURRENCIA: u64 = 4;
const OCIOSA_MS: u64 = 1500;

/// Una respuesta HTTP entera: estado, cabeceras, cuerpo y *trailers*.
#[derive(Debug, Default)]
pub struct Resp {
    pub codigo: u16,
    pub cabeceras: BTreeMap<String, String>,
    pub cuerpo: Vec<u8>,
    pub finales: BTreeMap<String, String>,
    pub ms: u128,
}

impl Resp {
    fn texto(&self) -> String {
        String::from_utf8_lossy(&self.cuerpo).into_owned()
    }
    fn campo(&self, k: &str) -> Option<String> {
        let n = ore_core::parse::parse(self.texto().trim()).ok()?;
        n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
    }
    fn fin(&self, k: &str) -> &str {
        self.finales.get(k).map(String::as_str).unwrap_or("")
    }
}

/// La pasarela lanzada, con lo que escribe por su salida de error.
pub struct Pasarela {
    hijo: Child,
    pub puerto: u16,
    pub registro: Arc<Mutex<String>>,
}

impl Drop for Pasarela {
    fn drop(&mut self) {
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

impl Pasarela {
    pub fn lanzar(binario: &Path, conectores: &Path, tipo: &str) -> Result<Pasarela, String> {
        let puerto = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| e.to_string())?
            .port();
        let mut hijo = Command::new(binario)
            .args([
                "--escucha",
                &format!("127.0.0.1:{puerto}"),
                "--conectores",
                &conectores.to_string_lossy(),
                "--tipos",
                tipo,
            ])
            .env("ORE_FED_CONCURRENCIA", CONCURRENCIA.to_string())
            .env("ORE_FED_COLA", "16")
            .env("ORE_FED_ESPERA_MS", "2000")
            .env("ORE_FED_OCIOSA_MS", OCIOSA_MS.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("no arranca `{}`: {e}", binario.display()))?;
        let registro = Arc::new(Mutex::new(String::new()));
        if let Some(e) = hijo.stderr.take() {
            let r = registro.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(e).lines().map_while(Result::ok) {
                    if let Ok(mut r) = r.lock() {
                        r.push_str(&l);
                        r.push('\n');
                    }
                }
            });
        }
        let p = Pasarela {
            hijo,
            puerto,
            registro,
        };
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(20) {
            if let Ok(r) = p.pedir("GET", "/v1/health", None)
                && r.codigo == 200
            {
                return Ok(p);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!(
            "la pasarela no quedó sana en 20 s: {}",
            p.registro.lock().map(|r| r.clone()).unwrap_or_default()
        ))
    }

    pub fn pid(&self) -> u32 {
        self.hijo.id()
    }

    /// Una petición entera.
    pub fn pedir(&self, metodo: &str, ruta: &str, cuerpo: Option<&str>) -> Result<Resp, String> {
        let t = Instant::now();
        let mut c = self.abrir(metodo, ruta, cuerpo)?;
        let mut r = leer_cabeza(&mut c)?;
        leer_cuerpo(&mut c, &mut r)?;
        r.ms = t.elapsed().as_millis();
        Ok(r)
    }

    fn abrir(
        &self,
        metodo: &str,
        ruta: &str,
        cuerpo: Option<&str>,
    ) -> Result<BufReader<TcpStream>, String> {
        let mut s = TcpStream::connect(("127.0.0.1", self.puerto)).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(60))).ok();
        let cuerpo = cuerpo.unwrap_or("");
        let req = format!(
            "{metodo} {ruta} HTTP/1.1\r\nhost: kit\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
            cuerpo.len()
        );
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        Ok(BufReader::new(s))
    }

    fn origen(&self, nombre: &str) -> Option<ore_core::parse::Node> {
        let r = self.pedir("GET", "/v1/origins", None).ok()?;
        let n = ore_core::parse::parse(r.texto().trim()).ok()?;
        let (_, os) = n.get("origenes")?;
        os.items()
            .iter()
            .find(|o| o.get("origen").and_then(|(_, v)| v.as_str()) == Some(nombre))
            .cloned()
    }
}

fn leer_cabeza(c: &mut BufReader<TcpStream>) -> Result<Resp, String> {
    let mut l = String::new();
    c.read_line(&mut l).map_err(|e| e.to_string())?;
    let codigo = l
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("una línea de estado rara: {l:?}"))?;
    let mut r = Resp {
        codigo,
        ..Resp::default()
    };
    loop {
        let mut l = String::new();
        c.read_line(&mut l).map_err(|e| e.to_string())?;
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            r.cabeceras
                .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Ok(r)
}

fn leer_cuerpo(c: &mut BufReader<TcpStream>, r: &mut Resp) -> Result<(), String> {
    if r.cabeceras.get("transfer-encoding").map(String::as_str) != Some("chunked") {
        let n: usize = r
            .cabeceras
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        r.cuerpo = vec![0; n];
        return c.read_exact(&mut r.cuerpo).map_err(|e| e.to_string());
    }
    loop {
        let mut l = String::new();
        c.read_line(&mut l).map_err(|e| e.to_string())?;
        let n = usize::from_str_radix(l.trim(), 16).map_err(|_| format!("un trozo raro: {l:?}"))?;
        if n == 0 {
            break;
        }
        let mut t = vec![0; n + 2];
        c.read_exact(&mut t)
            .map_err(|e| format!("cuerpo cortado: {e}"))?;
        r.cuerpo.extend_from_slice(&t[..n]);
    }
    loop {
        let mut l = String::new();
        if c.read_line(&mut l).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            r.finales
                .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Ok(())
}

fn lotes(bytes: &[u8]) -> Result<Vec<RecordBatch>, String> {
    let r = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None)
        .map_err(|e| format!("no es un flujo Arrow: {e}"))?;
    r.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("un lote roto: {e}"))
}

fn filas(bytes: &[u8]) -> Result<u64, String> {
    Ok(lotes(bytes)?.iter().map(|b| b.num_rows() as u64).sum())
}

static SIGUIENTE: AtomicU64 = AtomicU64::new(1);

fn nuevo_id() -> String {
    let n = SIGUIENTE.fetch_add(1, Ordering::SeqCst);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("kit-{n}-{t}")
}

struct Kit<'a> {
    p: &'a Pasarela,
    c: &'a Conector,
    b: &'a mut dyn Banco,
}

/// La parte de la petición que va al conector (sin `url` ni `id`).
type Pet = BTreeMap<String, Json>;

impl Kit<'_> {
    fn pet(&self, tabla: Tabla, proy: &[(&str, &str)]) -> Pet {
        let mut m = Pet::new();
        m.insert("objeto".into(), Json::s(self.b.objeto(tabla)));
        m.insert(
            "proyeccion".into(),
            Json::Obj(
                proy.iter()
                    .map(|(p, c)| (p.to_string(), Json::s(*c)))
                    .collect(),
            ),
        );
        self.b.completar(tabla, &mut m);
        m.remove("url");
        m
    }

    fn cuerpo(
        &self,
        id: &str,
        url: &str,
        pet: &Pet,
        presupuesto: Option<(u64, u64, u64)>,
    ) -> String {
        let mut o = BTreeMap::new();
        o.insert("id".to_string(), Json::s(id));
        o.insert("origen".to_string(), Json::s("kit"));
        o.insert("tipo".to_string(), Json::s(self.b.familia()));
        o.insert("url".to_string(), Json::s(url));
        o.insert("peticion".to_string(), Json::Obj(pet.clone()));
        if let Some((f, b, ms)) = presupuesto {
            o.insert(
                "presupuesto".to_string(),
                Json::obj([
                    ("filas", Json::Int(f as i64)),
                    ("bytes", Json::Int(b as i64)),
                    ("ms", Json::Int(ms as i64)),
                ]),
            );
        }
        Json::Obj(o).jcs()
    }

    fn leer(
        &self,
        pet: &Pet,
        presupuesto: Option<(u64, u64, u64)>,
    ) -> Result<(String, Resp), String> {
        let id = nuevo_id();
        let r = self.p.pedir(
            "POST",
            "/v1/read",
            Some(&self.cuerpo(&id, &self.b.url(), pet, presupuesto)),
        )?;
        Ok((id, r))
    }

    fn origen_limpio(&mut self, objeto: &str) -> Option<u128> {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            match self.b.consultas_vivas(objeto) {
                None => return Some(0),
                Some(0) => return Some(t.elapsed().as_millis()),
                Some(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        None
    }

    fn estado_de(&self, id: &str) -> Option<(String, String)> {
        let r = self.p.pedir("GET", &format!("/v1/read/{id}"), None).ok()?;
        Some((r.campo("estado")?, r.campo("motivo").unwrap_or_default()))
    }

    // ── 10 ─────────────────────────────────────────────────────────────────
    /// 0053 F9·3 · `versiones` y `bajar` (las colecciones de medios) por la
    /// pasarela: lo vigente bajo un prefijo, y los bytes de un ítem en flujo,
    /// con su final en los *trailers*. Sólo para S3: es la familia que los tiene.
    fn colecciones(&mut self) -> (Estado, String) {
        if self.b.familia() != "s3" {
            return (Estado::NoAplica, "sólo S3 tiene colecciones".into());
        }
        let url = self.b.url();
        let prefijo = self.b.objeto(Tabla::Tipos);
        let cuerpo = Json::obj([
            ("origen", Json::s("kit")),
            ("tipo", Json::s("s3")),
            ("url", Json::s(url.as_str())),
            (
                "peticion",
                Json::obj([
                    ("objeto", Json::s(prefijo.as_str())),
                    ("patrones", Json::Arr(vec![])),
                    ("conocidos", Json::Arr(vec![])),
                ]),
            ),
        ])
        .jcs();
        let r = match self.p.pedir("POST", "/v1/versions", Some(&cuerpo)) {
            Ok(r) => r,
            Err(e) => return (Estado::Falla, format!("versions: {e}")),
        };
        let texto = r.texto();
        let Ok(n) = ore_core::parse::parse(texto.trim()) else {
            return (Estado::Falla, format!("versions: {} · {texto}", r.codigo));
        };
        let items = n
            .get("items")
            .map(|(_, v)| v.items().to_vec())
            .unwrap_or_default();
        let Some(i) = items.first() else {
            return (
                Estado::Falla,
                format!("versions: sin ítems bajo `{prefijo}` · {texto}"),
            );
        };
        let campo = |k: &str| {
            i.get(k)
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let tamano: u64 = campo("tamano").parse().unwrap_or(0);
        let pedido = Json::obj([
            ("origen", Json::s("kit")),
            ("tipo", Json::s("s3")),
            ("url", Json::s(url.as_str())),
            (
                "peticion",
                Json::obj([(
                    "items",
                    Json::Arr(vec![Json::obj([
                        ("clave", Json::s(campo("clave"))),
                        ("version", Json::s(campo("version"))),
                        ("huella", Json::s(campo("huella"))),
                        ("tamano", Json::Int(tamano as i64)),
                        ("tipo", Json::s("text/csv")),
                    ])]),
                )]),
            ),
        ])
        .jcs();
        let r = match self.p.pedir("POST", "/v1/fetch", Some(&pedido)) {
            Ok(r) => r,
            Err(e) => return (Estado::Falla, format!("fetch: {e}")),
        };
        if r.codigo != 200 || r.fin("ore-estado") != "completo" || (r.cuerpo.len() as u64) < tamano
        {
            return (
                Estado::Falla,
                format!(
                    "fetch: {} · {} {} · {} B (el ítem pesa {tamano})",
                    r.codigo,
                    r.fin("ore-estado"),
                    r.fin("ore-motivo"),
                    r.cuerpo.len()
                ),
            );
        }
        let o = match self.p.pedir("GET", "/v1/origins", None) {
            Ok(r) => r.texto(),
            Err(e) => return (Estado::Falla, e),
        };
        if !o.contains(r#""fetch":1"#) || !o.contains(r#""versions":1"#) {
            return (Estado::Falla, format!("/v1/origins no los cuenta: {o}"));
        }
        (
            Estado::Pasa,
            format!(
                "versions: {} ítems · fetch: `{}` en {} B de flujo, completo · contados",
                items.len(),
                campo("clave"),
                r.cuerpo.len()
            ),
        )
    }

    // ── 9 ──────────────────────────────────────────────────────────────────
    /// 0053 F8 · comprobar y catalogar por la pasarela (de un tiro, en la cola
    /// del origen, sin la credencial en la respuesta) y una lectura de copia.
    fn una_via(&mut self) -> (Estado, String) {
        let url = self.b.url();
        let clave = clave_de(&url);
        let cuerpo = Json::obj([
            ("origen", Json::s("kit")),
            ("tipo", Json::s(self.b.familia())),
            ("url", Json::s(url.as_str())),
        ])
        .jcs();
        let mut dicho = Vec::new();
        for ruta in ["check", "catalog"] {
            let r = match self.p.pedir("POST", &format!("/v1/{ruta}"), Some(&cuerpo)) {
                Ok(r) => r,
                Err(e) => return (Estado::Falla, format!("{ruta}: {e}")),
            };
            let t = r.texto();
            if r.codigo != 200 || ore_core::parse::parse(t.trim()).is_err() {
                return (Estado::Falla, format!("{ruta}: {} · {}", r.codigo, t));
            }
            if clave.as_deref().is_some_and(|c| t.contains(c)) {
                return (
                    Estado::Falla,
                    format!("{ruta}: la respuesta lleva la credencial"),
                );
            }
            dicho.push(format!("{ruta} 200 en {} ms", r.ms));
        }
        // Un verbo que no existe y un perfil que no existe: 404/405 y 400.
        match self.p.pedir("POST", "/v1/borrar", Some(&cuerpo)) {
            Ok(r) if r.codigo == 404 => {}
            Ok(r) => return (Estado::Falla, format!("/v1/borrar: {}", r.codigo)),
            Err(e) => return (Estado::Falla, e),
        }
        // La copia: sin presupuesto, entera; y se cuenta como copia.
        let pet = self.pet(Tabla::Tipos, &[("id", "id")]);
        let id = nuevo_id();
        let mut c = match ore_core::parse::parse(&self.cuerpo(&id, &url, &pet, None)) {
            Ok(n) => match Json::de_node(&n) {
                Json::Obj(m) => m,
                _ => return (Estado::Falla, "cuerpo".into()),
            },
            Err(e) => return (Estado::Falla, format!("{e:?}")),
        };
        c.insert("perfil".into(), Json::s("copia"));
        let r = match self
            .p
            .pedir("POST", "/v1/read", Some(&Json::Obj(c.clone()).jcs()))
        {
            Ok(r) => r,
            Err(e) => return (Estado::Falla, e),
        };
        if r.codigo != 200 || r.fin("ore-estado") != "completo" {
            return (
                Estado::Falla,
                format!(
                    "copia: {} · {} · {}",
                    r.codigo,
                    r.fin("ore-estado"),
                    r.texto()
                ),
            );
        }
        dicho.push(format!("copia completa, {} filas", r.fin("ore-filas")));
        c.insert("id".into(), Json::s(nuevo_id()));
        c.insert("perfil".into(), Json::s("todo"));
        match self.p.pedir("POST", "/v1/read", Some(&Json::Obj(c).jcs())) {
            Ok(r) if r.codigo == 400 => {}
            Ok(r) => return (Estado::Falla, format!("perfil inventado: {}", r.codigo)),
            Err(e) => return (Estado::Falla, e),
        }
        // Y `/v1/origins` lo cuenta todo.
        let o = match self.p.pedir("GET", "/v1/origins", None) {
            Ok(r) => r.texto(),
            Err(e) => return (Estado::Falla, e),
        };
        for k in [r#""check":1"#, r#""catalog":1"#] {
            if !o.contains(k) {
                return (Estado::Falla, format!("/v1/origins no cuenta {k}: {o}"));
            }
        }
        if o.contains(r#""copias":0"#) {
            return (
                Estado::Falla,
                format!("/v1/origins no cuenta la copia: {o}"),
            );
        }
        dicho.push("/v1/origins lo cuenta".into());
        (Estado::Pasa, dicho.join(" · "))
    }

    // ── 1 ──────────────────────────────────────────────────────────────────
    fn igual(&mut self) -> (Estado, String) {
        let proy = [("id", "id"), ("texto", "texto"), ("entero", "entero")];
        let pet = self.pet(Tabla::Tipos, &proy);
        let (_, r) = match self.leer(&pet, None) {
            Ok(x) => x,
            Err(e) => return (Estado::Falla, e),
        };
        if r.codigo != 200 || r.fin("ore-estado") != "completo" {
            return (
                Estado::Falla,
                format!("{} · {} · {}", r.codigo, r.fin("ore-estado"), r.texto()),
            );
        }
        let mut directa = pet.clone();
        directa.insert("url".into(), Json::s(self.b.url()));
        directa.insert("formato".into(), Json::s("arrow"));
        let s = self.c.correr(
            "leer",
            &Json::Obj(directa).jcs(),
            Duration::from_secs(60),
            None,
        );
        if !s.ok {
            return (
                Estado::Falla,
                format!("el conector directo: {}", s.resumen()),
            );
        }
        match (lotes(&r.cuerpo), lotes(&s.stdout)) {
            (Ok(a), Ok(b)) if a == b => (
                Estado::Pasa,
                format!(
                    "{} lotes, {} filas, iguales; trailers {} filas · {} B",
                    a.len(),
                    a.iter().map(|x| x.num_rows()).sum::<usize>(),
                    r.fin("ore-filas"),
                    r.fin("ore-bytes")
                ),
            ),
            (Ok(a), Ok(b)) => (
                Estado::Falla,
                format!(
                    "distintos: {} lotes por la pasarela, {} directos",
                    a.len(),
                    b.len()
                ),
            ),
            (a, b) => (Estado::Falla, format!("{:?} / {:?}", a.err(), b.err())),
        }
    }

    // ── 2 ──────────────────────────────────────────────────────────────────
    fn caliente(&mut self) -> (Estado, String) {
        let pet = self.pet(Tabla::Tipos, &[("id", "id")]);
        let antes = self
            .p
            .origen("kit")
            .and_then(|o| {
                o.get("procesosLanzados")
                    .and_then(|(_, v)| v.as_str())
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0);
        let mut ms = Vec::new();
        for _ in 0..6 {
            match self.leer(&pet, None) {
                Ok((_, r)) if r.codigo == 200 && r.fin("ore-estado") == "completo" => ms.push(r.ms),
                Ok((_, r)) => return (Estado::Falla, format!("{} · {}", r.codigo, r.texto())),
                Err(e) => return (Estado::Falla, e),
            }
        }
        let despues = self
            .p
            .origen("kit")
            .and_then(|o| {
                o.get("procesosLanzados")
                    .and_then(|(_, v)| v.as_str())
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0);
        let mut resto = ms[1..].to_vec();
        resto.sort_unstable();
        let mediana = resto[resto.len() / 2];
        let lanzados = despues - antes;
        let detalle = format!(
            "6 lecturas seguidas, {lanzados} proceso(s) nuevo(s); la 1ª {} ms, la mediana de las demás {mediana} ms",
            ms[0]
        );
        if lanzados <= 1 {
            (Estado::Pasa, detalle)
        } else {
            (Estado::Falla, detalle)
        }
    }

    // ── 3 ──────────────────────────────────────────────────────────────────
    fn presupuesto(&mut self) -> (Estado, String) {
        let mut dicho = Vec::new();
        let grande = self.b.objeto(Tabla::Grande);
        let pet = self.pet(Tabla::Grande, &[("id", "id"), ("nota", "nota")]);

        // Por filas: la fila exacta.
        match self.leer(&pet, Some((1000, 64 << 20, 30_000))) {
            Ok((_, r)) => {
                let n = filas(&r.cuerpo);
                if r.codigo != 200
                    || n != Ok(1000)
                    || r.fin("ore-estado") != "cortado"
                    || r.fin("ore-motivo") != "filas"
                {
                    return (
                        Estado::Falla,
                        format!(
                            "filas: {} · {n:?} · {} {}",
                            r.codigo,
                            r.fin("ore-estado"),
                            r.fin("ore-motivo")
                        ),
                    );
                }
                match self.origen_limpio(&grande) {
                    Some(ms) => dicho.push(format!(
                        "filas: 1000 justas, cortado, origen limpio en {ms} ms"
                    )),
                    None => {
                        return (
                            Estado::Falla,
                            "filas: la consulta sigue viva en el origen 3 s después".into(),
                        );
                    }
                }
            }
            Err(e) => return (Estado::Falla, e),
        }
        // Por bytes: no se pasa del lote que lo rebasaría.
        let tope = 256 * 1024;
        match self.leer(&pet, Some((10_000_000, tope, 30_000))) {
            Ok((_, r)) => {
                let n = filas(&r.cuerpo).unwrap_or(0);
                if r.codigo != 200 || r.fin("ore-motivo") != "bytes" || n == 0 || n >= 1_000_000 {
                    return (
                        Estado::Falla,
                        format!(
                            "bytes: {} · {n} filas · {} {}",
                            r.codigo,
                            r.fin("ore-estado"),
                            r.fin("ore-motivo")
                        ),
                    );
                }
                dicho.push(format!(
                    "bytes: {n} filas en {} B (tope {tope})",
                    r.cuerpo.len()
                ));
            }
            Err(e) => return (Estado::Falla, e),
        }
        // Por tiempo, con algo lento de verdad.
        if let Some(lenta) = self.b.lenta() {
            let mut p = self.pet(Tabla::Tipos, &[("id", "id")]);
            p.insert("objeto".into(), Json::s(lenta.as_str()));
            let t = Instant::now();
            match self.leer(&p, Some((1000, 64 << 20, 1500))) {
                Ok((id, r)) => {
                    let ms = t.elapsed().as_millis();
                    let por_tiempo = (r.codigo == 504
                        && r.campo("codigo").as_deref() == Some("tiempo"))
                        || (r.codigo == 200 && r.fin("ore-motivo") == "tiempo");
                    if !por_tiempo || ms > 6000 {
                        return (
                            Estado::Falla,
                            format!(
                                "tiempo: {} en {ms} ms · {:?} · {}",
                                r.codigo,
                                r.finales,
                                r.campo("mensaje").unwrap_or_default()
                            ),
                        );
                    }
                    let e = self.estado_de(&id);
                    match self.origen_limpio(&lenta) {
                        Some(l) => dicho.push(format!(
                            "tiempo: {} a los {ms} ms ({e:?}), origen limpio en {l} ms",
                            r.codigo
                        )),
                        None => {
                            return (
                                Estado::Falla,
                                "tiempo: la consulta sigue viva en el origen 3 s después".into(),
                            );
                        }
                    }
                }
                Err(e) => return (Estado::Falla, e),
            }
            self.b.limpiar();
        } else {
            dicho.push("tiempo: no hay nada lento en este banco".into());
        }
        (Estado::Pasa, dicho.join("; "))
    }

    // ── 4 ──────────────────────────────────────────────────────────────────
    fn cola(&mut self) -> (Estado, String) {
        let pet = self.pet(Tabla::Tipos, &[("id", "id"), ("texto", "texto")]);
        let url = self.b.url();
        let n = 40;
        let pico = Arc::new(AtomicU64::new(0));
        let sigue = Arc::new(std::sync::atomic::AtomicBool::new(true));
        // Las sesiones del origen, mientras tanto (si el banco las cuenta).
        let mide = self.b.sesiones().is_some();
        let hilos: Vec<_> = (0..n)
            .map(|_| {
                let cuerpo = self.cuerpo(&nuevo_id(), &url, &pet, None);
                let puerto = self.p.puerto;
                std::thread::spawn(move || {
                    let p = PasarelaRemota { puerto };
                    p.pedir(&cuerpo)
                })
            })
            .collect();
        while hilos.iter().any(|h| !h.is_finished()) && mide {
            if let Some(s) = self.b.sesiones() {
                pico.fetch_max(s, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        sigue.store(false, Ordering::SeqCst);
        let mut ok = 0;
        let mut saturadas = 0;
        let mut otras = Vec::new();
        for h in hilos {
            match h.join() {
                Ok(Ok(r)) if r.codigo == 200 && r.fin("ore-estado") == "completo" => ok += 1,
                Ok(Ok(r)) if r.codigo == 503 && r.cabeceras.contains_key("retry-after") => {
                    saturadas += 1
                }
                Ok(Ok(r)) => otras.push(format!("{} {}", r.codigo, r.texto())),
                Ok(Err(e)) => otras.push(e),
                Err(_) => otras.push("un hilo murió".into()),
            }
        }
        let pico = pico.load(Ordering::SeqCst);
        let mut detalle = format!(
            "{n} a la vez: {ok} completas, {saturadas} saturadas (503 con Retry-After), {} otras; pico de sesiones en el origen {}",
            otras.len(),
            if mide {
                pico.to_string()
            } else {
                "sin medir".into()
            }
        );
        if !otras.is_empty() {
            return (Estado::Falla, format!("{detalle}: {}", otras[0]));
        }
        if ok == 0 || (mide && pico > CONCURRENCIA) {
            return (Estado::Falla, detalle);
        }
        // Y llena: con lecturas lentas, la que no cabe ni en la cola es 503 ya.
        if let Some(lenta) = self.b.lenta() {
            let mut p = self.pet(Tabla::Tipos, &[("id", "id")]);
            p.insert("objeto".into(), Json::s(lenta.as_str()));
            let ids: Vec<String> = (0..(CONCURRENCIA + 16)).map(|_| nuevo_id()).collect();
            let hilos: Vec<_> = ids
                .iter()
                .map(|id| {
                    let cuerpo = self.cuerpo(id, &url, &p, Some((10, 1 << 20, 5000)));
                    let puerto = self.p.puerto;
                    std::thread::spawn(move || PasarelaRemota { puerto }.pedir(&cuerpo))
                })
                .collect();
            std::thread::sleep(Duration::from_millis(600));
            let t = Instant::now();
            let sobra = self.leer(&p, Some((10, 1 << 20, 5000)));
            let ms = t.elapsed().as_millis();
            for id in &ids {
                let _ = self.p.pedir("DELETE", &format!("/v1/read/{id}"), None);
            }
            for h in hilos {
                let _ = h.join();
            }
            self.b.limpiar();
            match sobra {
                Ok((_, r))
                    if r.codigo == 503
                        && r.campo("codigo").as_deref() == Some("saturado")
                        && ms < 1000 =>
                {
                    detalle.push_str(&format!(
                        "; llena ({} en curso o en cola), la siguiente 503 en {ms} ms",
                        CONCURRENCIA + 16
                    ));
                }
                Ok((_, r)) => {
                    return (
                        Estado::Falla,
                        format!("{detalle}; llena, la siguiente dio {} en {ms} ms", r.codigo),
                    );
                }
                Err(e) => return (Estado::Falla, e),
            }
        }
        (Estado::Pasa, detalle)
    }

    // ── 5 ──────────────────────────────────────────────────────────────────
    fn cancelar(&mut self) -> (Estado, String) {
        let mut dicho = Vec::new();
        // Por DELETE, mientras el origen trabaja.
        if let Some(lenta) = self.b.lenta() {
            let mut p = self.pet(Tabla::Tipos, &[("id", "id")]);
            p.insert("objeto".into(), Json::s(lenta.as_str()));
            let id = nuevo_id();
            let cuerpo = self.cuerpo(&id, &self.b.url(), &p, Some((10, 1 << 20, 30_000)));
            let puerto = self.p.puerto;
            let h = std::thread::spawn(move || PasarelaRemota { puerto }.pedir(&cuerpo));
            std::thread::sleep(Duration::from_millis(1000));
            let d = self.p.pedir("DELETE", &format!("/v1/read/{id}"), None);
            let r = h.join().ok().and_then(Result::ok);
            let limpio = self.origen_limpio(&lenta);
            let e = self.estado_de(&id);
            self.b.limpiar();
            match (d.map(|d| d.codigo), limpio, &e) {
                (Ok(202), Some(ms), Some((est, mot))) if est == "cortado" && mot == "cancelada" => {
                    dicho.push(format!(
                        "DELETE: {} y origen limpio en {ms} ms",
                        r.map(|r| r.codigo).unwrap_or(0)
                    ))
                }
                (d, l, e) => {
                    return (
                        Estado::Falla,
                        format!("DELETE: {d:?} · limpio {l:?} · {e:?}"),
                    );
                }
            }
        }
        // Por desconexión, a mitad de un flujo.
        let grande = self.b.objeto(Tabla::Grande);
        let pet = self.pet(Tabla::Grande, &[("id", "id"), ("nota", "nota")]);
        let id = nuevo_id();
        let cuerpo = self.cuerpo(
            &id,
            &self.b.url(),
            &pet,
            Some((10_000_000, 1 << 30, 60_000)),
        );
        match self.p.abrir("POST", "/v1/read", Some(&cuerpo)) {
            Ok(mut c) => {
                let r = leer_cabeza(&mut c);
                let mut algo = [0u8; 4096];
                let _ = c.read(&mut algo);
                drop(c);
                if !matches!(r, Ok(ref r) if r.codigo == 200) {
                    return (
                        Estado::Falla,
                        format!("desconexión: no empezó ({:?})", r.map(|r| r.codigo)),
                    );
                }
            }
            Err(e) => return (Estado::Falla, e),
        }
        let t = Instant::now();
        let mut e = None;
        while t.elapsed() < Duration::from_secs(10) {
            e = self.estado_de(&id);
            if matches!(&e, Some((est, _)) if est != "en-curso") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let ms = t.elapsed().as_millis();
        let limpio = self.origen_limpio(&grande);
        self.b.limpiar();
        match (&e, limpio) {
            (Some((est, mot)), Some(l)) if est == "cortado" && mot == "desconexion" => dicho.push(
                format!("desconexión: cortada a los {ms} ms, origen limpio en {l} ms"),
            ),
            _ => {
                return (
                    Estado::Falla,
                    format!("desconexión: {e:?}, limpio {limpio:?}"),
                );
            }
        }
        (Estado::Pasa, dicho.join("; "))
    }

    // ── 6 ──────────────────────────────────────────────────────────────────
    fn credencial(&mut self) -> (Estado, String) {
        let url = self.b.url();
        let Some(clave) = clave_de(&url) else {
            return (
                Estado::NoAplica,
                "la url de este banco no lleva clave".into(),
            );
        };
        // Una lectura buena y una mala, para que haya registro de las dos.
        let pet = self.pet(Tabla::Tipos, &[("id", "id")]);
        let _ = self.leer(&pet, None);
        let mut mala = None;
        if let Some(u) = self.b.url_mala() {
            let id = nuevo_id();
            let _ = self
                .p
                .pedir("POST", "/v1/read", Some(&self.cuerpo(&id, &u, &pet, None)));
            mala = Some(id);
        }
        let mut donde = Vec::new();
        std::thread::sleep(Duration::from_millis(300));
        let registro = self
            .p
            .registro
            .lock()
            .map(|r| r.clone())
            .unwrap_or_default();
        if registro.contains(&clave) {
            donde.push("el registro de la pasarela".to_string());
        }
        for ruta in ["/v1/origins", "/v1/connectors", "/v1/health"] {
            if let Ok(r) = self.p.pedir("GET", ruta, None)
                && r.texto().contains(&clave)
            {
                donde.push(ruta.to_string());
            }
        }
        if let Some(id) = &mala
            && let Ok(r) = self.p.pedir("GET", &format!("/v1/read/{id}"), None)
            && r.texto().contains(&clave)
        {
            donde.push("el estado de una lectura con la credencial mala".into());
        }
        let procesos = procesos_de(self.p.pid());
        if procesos.is_empty() {
            return if donde.is_empty() {
                (
                    Estado::Pasa,
                    "ni en el registro ni en las respuestas (sin /proc: argv y entorno sin mirar)"
                        .into(),
                )
            } else {
                (
                    Estado::Falla,
                    format!("la clave sale en: {}", donde.join(", ")),
                )
            };
        }
        for pid in &procesos {
            for f in ["cmdline", "environ"] {
                if let Ok(b) = std::fs::read(format!("/proc/{pid}/{f}"))
                    && String::from_utf8_lossy(&b).contains(&clave)
                {
                    donde.push(format!("/proc/{pid}/{f}"));
                }
            }
        }
        if donde.is_empty() {
            (
                Estado::Pasa,
                format!(
                    "ni en el registro, ni en las respuestas, ni en argv/entorno de {} procesos (la pasarela y sus conectores)",
                    procesos.len()
                ),
            )
        } else {
            (
                Estado::Falla,
                format!("la clave sale en: {}", donde.join(", ")),
            )
        }
    }

    // ── 7 ──────────────────────────────────────────────────────────────────
    fn ocioso(&mut self) -> (Estado, String) {
        let pet = self.pet(Tabla::Tipos, &[("id", "id")]);
        if let Err(e) = self.leer(&pet, None) {
            return (Estado::Falla, e);
        }
        let calientes = |k: &Kit| {
            k.p.origen("kit")
                .and_then(|o| {
                    o.get("calientes")
                        .and_then(|(_, v)| v.as_str())
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .unwrap_or(0)
        };
        let antes = calientes(self);
        let sesiones_antes = self.b.sesiones();
        std::thread::sleep(Duration::from_millis(OCIOSA_MS * 2 + 1000));
        let despues = calientes(self);
        let sesiones = self.b.sesiones();
        let detalle = format!(
            "calientes {antes} → {despues} a los {} ms; sesiones en el origen {sesiones_antes:?} → {sesiones:?}",
            OCIOSA_MS * 2 + 1000
        );
        if antes >= 1 && despues == 0 && sesiones.unwrap_or(0) == 0 {
            (Estado::Pasa, detalle)
        } else {
            (Estado::Falla, detalle)
        }
    }

    // ── 8 ──────────────────────────────────────────────────────────────────
    fn errores(&mut self) -> (Estado, String) {
        let mut dicho = Vec::new();
        let mut mal = Vec::new();
        let esperar = |dicho: &mut Vec<String>,
                       mal: &mut Vec<String>,
                       nombre: &str,
                       r: Result<Resp, String>,
                       codigo: u16,
                       cod: &str| match r {
            Ok(r) if r.codigo == codigo && r.campo("codigo").as_deref() == Some(cod) => {
                dicho.push(format!("{nombre} → {codigo} {cod}"))
            }
            Ok(r) => mal.push(format!("{nombre} → {} {}", r.codigo, r.texto().trim())),
            Err(e) => mal.push(format!("{nombre}: {e}")),
        };
        // Un objeto que no existe: lo que diga el conector directo (en S3 un
        // prefijo vacío es una tabla vacía, no un error), la pasarela igual.
        let mut pet = self.pet(Tabla::Tipos, &[("id", "id")]);
        pet.insert(
            "objeto".into(),
            Json::s(format!("{}_no_existe", self.b.objeto(Tabla::Tipos))),
        );
        let mut directa = pet.clone();
        directa.insert("url".into(), Json::s(self.b.url()));
        directa.insert("formato".into(), Json::s("arrow"));
        let s = self.c.correr(
            "leer",
            &Json::Obj(directa).jcs(),
            Duration::from_secs(60),
            None,
        );
        let r = self.leer(&pet, None).map(|x| x.1);
        match s.fallo() {
            Some(f) if !s.ok => esperar(
                &mut dicho,
                &mut mal,
                "un objeto que no existe",
                r,
                crate_http(f.codigo),
                f.codigo.as_str(),
            ),
            _ => match r {
                Ok(r) if r.codigo == 200 => {
                    dicho.push("un objeto que no existe → 200, como el conector directo".into())
                }
                Ok(r) => mal.push(format!(
                    "un objeto que no existe → {}, y el conector directo lo da por bueno",
                    r.codigo
                )),
                Err(e) => mal.push(e),
            },
        }
        let mut pet = self.pet(Tabla::Tipos, &[("id", "id")]);
        pet.insert(
            "filtros".into(),
            Json::Arr(vec![Json::obj([
                ("columna", Json::s("id")),
                ("operador", Json::s("regex")),
                ("valor", Json::s("x")),
            ])]),
        );
        esperar(
            &mut dicho,
            &mut mal,
            "un operador que no existe",
            self.leer(&pet, None).map(|x| x.1),
            400,
            "operador",
        );
        if let Some(u) = self.b.url_mala() {
            let pet = self.pet(Tabla::Tipos, &[("id", "id")]);
            let r = self.p.pedir(
                "POST",
                "/v1/read",
                Some(&self.cuerpo(&nuevo_id(), &u, &pet, None)),
            );
            esperar(
                &mut dicho,
                &mut mal,
                "una credencial mala",
                r,
                502,
                "credencial",
            );
        }
        esperar(
            &mut dicho,
            &mut mal,
            "un cuerpo que no es JSON",
            self.p.pedir("POST", "/v1/read", Some("{esto no")),
            400,
            "operador",
        );
        esperar(
            &mut dicho,
            &mut mal,
            "sin id",
            self.p
                .pedir("POST", "/v1/read", Some(r#"{"origen":"kit"}"#)),
            400,
            "operador",
        );
        let pet = self.pet(Tabla::Tipos, &[("id", "id")]);
        let id = nuevo_id();
        let cuerpo = self.cuerpo(&id, &self.b.url(), &pet, None);
        let _ = self.p.pedir("POST", "/v1/read", Some(&cuerpo));
        esperar(
            &mut dicho,
            &mut mal,
            "un id repetido",
            self.p.pedir("POST", "/v1/read", Some(&cuerpo)),
            409,
            "operador",
        );
        if mal.is_empty() {
            (Estado::Pasa, dicho.join("; "))
        } else {
            (Estado::Falla, mal.join("; "))
        }
    }
}

/// El estado HTTP que `docs/federation.md` §3 pone a cada código.
fn crate_http(c: ore_driver::Codigo) -> u16 {
    use ore_driver::Codigo::*;
    match c {
        Operador => 400,
        Objeto => 404,
        Tiempo => 504,
        Credencial | Conexion | Origen => 502,
    }
}

/// Un cliente que sólo sabe el puerto (para los hilos).
struct PasarelaRemota {
    puerto: u16,
}

impl PasarelaRemota {
    fn pedir(&self, cuerpo: &str) -> Result<Resp, String> {
        let mut s = TcpStream::connect(("127.0.0.1", self.puerto)).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(60))).ok();
        let req = format!(
            "POST /v1/read HTTP/1.1\r\nhost: kit\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
            cuerpo.len()
        );
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut c = BufReader::new(s);
        let mut r = leer_cabeza(&mut c)?;
        leer_cuerpo(&mut c, &mut r)?;
        Ok(r)
    }
}

/// La clave de una `url` (`esquema://usuario:clave@…`), si la lleva.
fn clave_de(url: &str) -> Option<String> {
    let resto = url.split_once("://")?.1;
    let usuario = resto.split_once('@')?.0;
    let clave = usuario.split_once(':')?.1;
    (clave.len() >= 4).then(|| clave.to_string())
}

/// La pasarela y sus hijos, por `/proc` (vacío si no hay `/proc`).
fn procesos_de(pid: u32) -> Vec<u32> {
    let mut v = vec![];
    if !Path::new(&format!("/proc/{pid}")).exists() {
        return v;
    }
    v.push(pid);
    if let Ok(tareas) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for t in tareas.flatten() {
            if let Ok(h) = std::fs::read_to_string(t.path().join("children")) {
                v.extend(h.split_whitespace().filter_map(|x| x.parse::<u32>().ok()));
            }
        }
    }
    v
}

/// **Corre los ocho** contra la pasarela de `binario`, con el conector de
/// `conector` (la pasarela busca los conectores en su directorio).
pub fn correr(
    binario: &Path,
    conector: &Conector,
    ruta_conector: &Path,
    b: &mut dyn Banco,
    solo: &[u8],
) -> Result<Vec<Resultado>, String> {
    let dir: PathBuf = ruta_conector
        .parent()
        .map(PathBuf::from)
        .ok_or("el conector no tiene directorio")?;
    let esperado = format!("ore-read-{}", b.familia());
    if ruta_conector
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        != Some(esperado.clone())
    {
        return Err(format!(
            "la pasarela busca `{esperado}` junto a los demás; el conector se llama `{}`",
            ruta_conector.display()
        ));
    }
    let p = Pasarela::lanzar(binario, &dir, b.familia())?;
    let mut kit = Kit {
        p: &p,
        c: conector,
        b,
    };
    let quiere = |n: u8| solo.is_empty() || solo.contains(&n);
    let mut hechos = Vec::new();
    for (n, nombre) in CASOS {
        if !quiere(n) {
            continue;
        }
        let (estado, detalle) = match n {
            1 => kit.igual(),
            2 => kit.caliente(),
            3 => kit.presupuesto(),
            4 => kit.cola(),
            5 => kit.cancelar(),
            6 => kit.credencial(),
            7 => kit.ocioso(),
            8 => kit.errores(),
            9 => kit.una_via(),
            10 => kit.colecciones(),
            _ => unreachable!(),
        };
        eprintln!("  P{n} · {nombre}: {}", estado.as_str());
        hechos.push(Resultado {
            caso: n,
            nombre,
            estado,
            detalle,
        });
    }
    Ok(hechos)
}
