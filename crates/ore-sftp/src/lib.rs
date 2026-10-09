//! **Un servidor SFTP de un cliente, como origen** (ADR 0061 O4), con la
//! identidad y la verificación de D-O4.
//!
//! ```text
//! sftp://<usuario>@<host>[:<puerto>]/<ruta>?huella=SHA256:<…>[&edad=<s>][&legado=1]
//! ```
//!
//! - **La clave es de la celda**: una Ed25519 que genera ORE, cuya privada lee
//!   el driver de `ORE_SFTP_CLAVE` (un fichero) y cuya pública el cliente pega
//!   en `authorized_keys`. De recurso, una contraseña en la URL
//!   (`usuario:contraseña@`), que entonces vive en el custodio como la clave de
//!   S3.
//! - **La huella del host, siempre fijada** (`huella=SHA256:…`): se compara
//!   **antes** de autenticar, así que a un servidor que no es no le llega ni
//!   la clave. Sin ella no se lee: se dice cuál es la vista, para confirmarla.
//! - **Un SFTP no versiona** (D-O1, sólo colección mantenida): el validador de
//!   un fichero es su tamaño y su `mtime` (en segundos: SFTP v3, medido en
//!   O4·0), se compara al abrir y **otra vez al terminar de leer** —un fichero
//!   reescrito en sitio mientras se lee da una mezcla de los dos (O4·0)—, y no
//!   se copia uno que lleve menos de `edad` segundos sin cambiar (60 por
//!   defecto).
//! - **Los enlaces simbólicos no se siguen**: pueden salir del directorio o
//!   hacer un ciclo.
//! - Algoritmos modernos; `ssh-rsa` con SHA-1 y los KEX viejos sólo con
//!   `legado=1`.

mod origen;

use std::io::{Read, Seek, SeekFrom};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Lo que se espera, en segundos, a conectar y a cada operación.
const ESPERA: Duration = Duration::from_secs(20);
const EDAD: u64 = 60;

static PETICIONES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Cuántas operaciones SFTP (listar un directorio, mirar, abrir), para los
/// avisos del driver; los bytes van en flujo y no se cuentan.
pub fn contadores() -> (usize, usize) {
    (PETICIONES.load(std::sync::atomic::Ordering::Relaxed), 0)
}

fn contar() {
    PETICIONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}
const HOSTKEY: &str = "ssh-ed25519,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,rsa-sha2-512,rsa-sha2-256";
const KEX: &str = "curve25519-sha256,curve25519-sha256@libssh.org,ecdh-sha2-nistp256,\
    ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group-exchange-sha256,\
    diffie-hellman-group16-sha512,diffie-hellman-group18-sha512,diffie-hellman-group14-sha256";

/// La coordenada de una fuente SFTP.
#[derive(Clone, PartialEq, Eq)]
pub struct Fuente {
    pub usuario: String,
    pub host: String,
    pub puerto: u16,
    /// Vacío o acabado en `/`, relativo a la raíz del usuario.
    pub prefijo: String,
    /// `SHA256:<base64 sin relleno>`, como la escribe `ssh-keygen -l`.
    pub huella: Option<String>,
    /// Segundos sin cambiar para que un fichero se copie.
    pub edad: u64,
    /// `ssh-rsa` con SHA-1 y los KEX viejos.
    pub legado: bool,
    contrasena: Option<String>,
}

impl std::fmt::Debug for Fuente {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", publica(self, &self.prefijo))
    }
}

/// `sftp://usuario@host:puerto/ruta?huella=…` → la coordenada.
pub fn leer(url: &str) -> Result<Fuente, String> {
    let resto = url.trim().strip_prefix("sftp://").ok_or(
        "la URL de un SFTP es `sftp://<usuario>@<host>[:<puerto>]/<ruta>?huella=SHA256:…`",
    )?;
    let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
    let (autoridad, ruta) = camino.split_once('/').unwrap_or((camino, ""));
    let (quien, donde) = autoridad
        .rsplit_once('@')
        .ok_or("falta el usuario: `sftp://<usuario>@<host>`")?;
    let (usuario, contrasena) = match quien.split_once(':') {
        Some((u, c)) => (descodificar(u), Some(descodificar(c))),
        None => (descodificar(quien), None),
    };
    let (host, puerto) = match donde.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (
            h,
            p.parse::<u16>()
                .map_err(|_| format!("`{p}` no es un puerto"))?,
        ),
        _ => (donde, 22),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if usuario.is_empty()
        || host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:".contains(&b))
    {
        return Err(format!("`{usuario}@{host}` no es un usuario y un host"));
    }
    let (mut huella, mut edad, mut legado) = (None, EDAD, false);
    for par in consulta.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = par.split_once('=').unwrap_or((par, ""));
        let v = descodificar(v);
        match k {
            "huella" => {
                if !v.starts_with("SHA256:") || v.len() < 20 {
                    return Err(format!(
                        "`huella={v}` no es una huella SHA-256 (`SHA256:…`, la de `ssh-keygen -l`)"
                    ));
                }
                huella = Some(v.trim_end_matches('=').to_string());
            }
            "edad" => {
                edad = v
                    .parse()
                    .map_err(|_| format!("`edad={v}` no son segundos"))?
            }
            "legado" => legado = v == "1",
            otro => return Err(format!("`{otro}` no es un parámetro de una URL `sftp://`")),
        }
    }
    let ruta = descodificar(ruta);
    let ruta = ruta.trim_start_matches('/');
    let prefijo = if ruta.is_empty() || ruta.ends_with('/') {
        ruta.to_string()
    } else {
        format!("{ruta}/")
    };
    Ok(Fuente {
        usuario,
        host: host.to_string(),
        puerto,
        prefijo,
        huella,
        edad,
        legado,
        contrasena,
    })
}

/// La URL de un prefijo, que se puede enseñar: sin la contraseña, si la hay.
pub fn publica(f: &Fuente, prefijo: &str) -> String {
    let host = if f.host.contains(':') {
        format!("[{}]", f.host)
    } else {
        f.host.clone()
    };
    let puerto = if f.puerto == 22 {
        String::new()
    } else {
        format!(":{}", f.puerto)
    };
    let mut q = Vec::new();
    if let Some(h) = &f.huella {
        q.push(format!("huella={h}"));
    }
    if f.edad != EDAD {
        q.push(format!("edad={}", f.edad));
    }
    if f.legado {
        q.push("legado=1".into());
    }
    let mut u = format!("sftp://{}@{host}{puerto}/{prefijo}", f.usuario);
    if !q.is_empty() {
        u.push('?');
        u.push_str(&q.join("&"));
    }
    u
}

/// `%xx` deshecho; un `+` se queda (una huella en base64 los lleva).
fn descodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── Los fallos ──────────────────────────────────────────────────────────────

/// De qué es un fallo: lo que `check` y el driver dicen distinto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tipo {
    /// El host no contesta (o no resuelve).
    Conexion,
    /// La URL no fija la huella: `vista` es la del servidor, para confirmar.
    HuellaFalta {
        vista: String,
    },
    /// La huella no es la fijada: posible impostor; no se autenticó.
    HuellaDistinta {
        vista: String,
    },
    /// La clave (o la contraseña) no entra.
    Autenticacion,
    NoEsta,
    Permiso,
    /// El fichero no es el que se listó, o cambió mientras se leía.
    Cambio,
    Otro,
}

#[derive(Debug, Clone)]
pub struct Fallo {
    pub tipo: Tipo,
    pub mensaje: String,
}

impl std::fmt::Display for Fallo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.mensaje)
    }
}

impl Fallo {
    fn de(tipo: Tipo, mensaje: impl Into<String>) -> Fallo {
        Fallo {
            tipo,
            mensaje: mensaje.into(),
        }
    }

    /// Un error de SFTP sobre una ruta: `2` no está, `3` sin permiso.
    fn de_sftp(ruta: &str, e: &ssh2::Error) -> Fallo {
        match e.code() {
            ssh2::ErrorCode::SFTP(2) => {
                Fallo::de(Tipo::NoEsta, format!("`{ruta}` no está (SFTP 2)"))
            }
            ssh2::ErrorCode::SFTP(3) => Fallo::de(
                Tipo::Permiso,
                format!("`{ruta}`: sin permiso de lectura para este usuario (SFTP 3)"),
            ),
            _ => Fallo::de(Tipo::Otro, format!("`{ruta}`: {e}")),
        }
    }

    pub(crate) fn cambio(clave: &str) -> Fallo {
        Fallo::de(Tipo::Cambio, ore_objetos::memoria::cambio(clave))
    }
}

// ── La conexión ─────────────────────────────────────────────────────────────

struct Conexion {
    _sesion: ssh2::Session,
    sftp: ssh2::Sftp,
}

/// La huella de la clave del host, como `ssh-keygen -l`: `SHA256:<b64>`.
fn huella(s: &ssh2::Session) -> Option<String> {
    s.host_key_hash(ssh2::HashType::Sha256).map(|h| {
        format!(
            "SHA256:{}",
            ore_objetos::huella::base64(h).trim_end_matches('=')
        )
    })
}

/// El handshake, sin autenticar: la sesión y la huella vista.
fn saludar(f: &Fuente) -> Result<(ssh2::Session, String), Fallo> {
    let direccion = (f.host.as_str(), f.puerto)
        .to_socket_addrs()
        .map_err(|e| Fallo::de(Tipo::Conexion, format!("`{}` no resuelve: {e}", f.host)))?
        .next()
        .ok_or_else(|| Fallo::de(Tipo::Conexion, format!("`{}` no resuelve", f.host)))?;
    let tcp = TcpStream::connect_timeout(&direccion, ESPERA).map_err(|e| {
        Fallo::de(
            Tipo::Conexion,
            format!(
                "`{}:{}` no contesta ({e}): ¿está abierto a la IP de salida de esta celda?",
                f.host, f.puerto
            ),
        )
    })?;
    let mut s = ssh2::Session::new().map_err(|e| Fallo::de(Tipo::Otro, e.to_string()))?;
    s.set_tcp_stream(tcp);
    s.set_timeout(ESPERA.as_millis() as u32);
    let (hk, kex) = if f.legado {
        (
            format!("{HOSTKEY},ssh-rsa"),
            format!("{KEX},diffie-hellman-group14-sha1,diffie-hellman-group-exchange-sha1"),
        )
    } else {
        (HOSTKEY.to_string(), KEX.to_string())
    };
    // libssh2 se queda con los que sabe de cada lista.
    let _ = s.method_pref(ssh2::MethodType::HostKey, &hk);
    let _ = s.method_pref(ssh2::MethodType::Kex, &kex);
    s.handshake().map_err(|e| {
        Fallo::de(
            Tipo::Conexion,
            format!(
                "el saludo SSH con `{}` falla ({e}){}",
                f.host,
                if f.legado {
                    ""
                } else {
                    ": si el servidor es viejo (sólo `ssh-rsa`), `legado=1` en la URL"
                }
            ),
        )
    })?;
    let vista = huella(&s).ok_or_else(|| Fallo::de(Tipo::Otro, "el servidor no da su clave"))?;
    Ok((s, vista))
}

fn conectar(f: &Fuente) -> Result<Conexion, Fallo> {
    let (s, vista) = saludar(f)?;
    match &f.huella {
        None => {
            return Err(Fallo::de(
                Tipo::HuellaFalta {
                    vista: vista.clone(),
                },
                format!(
                    "la URL no fija la huella del host: la de `{}` es `{vista}`; si es la suya, \
                     `huella={vista}` en la URL",
                    f.host
                ),
            ));
        }
        Some(h) if *h != vista => {
            return Err(Fallo::de(
                Tipo::HuellaDistinta {
                    vista: vista.clone(),
                },
                format!(
                    "la huella del host `{}` es `{vista}` y la fijada `{h}`: puede ser otro servidor \
                     haciéndose pasar por él, así que no se autentica ni se lee. Si el cliente la \
                     cambió, se vuelve a fijar a propósito",
                    f.host
                ),
            ));
        }
        Some(_) => {}
    }
    match &f.contrasena {
        Some(c) => s.userauth_password(&f.usuario, c),
        None => {
            let clave = std::env::var("ORE_SFTP_CLAVE").map_err(|_| {
                Fallo::de(
                    Tipo::Autenticacion,
                    "falta la clave de la celda (`ORE_SFTP_CLAVE`, el fichero de su privada)",
                )
            })?;
            s.userauth_pubkey_file(&f.usuario, None, Path::new(&clave), None)
        }
    }
    .map_err(|e| {
        Fallo::de(
            Tipo::Autenticacion,
            format!(
                "`{}@{}` no deja entrar ({e}): {}",
                f.usuario,
                f.host,
                if f.contrasena.is_some() {
                    "la contraseña no vale"
                } else {
                    "la clave pública de la celda tiene que estar en su `~/.ssh/authorized_keys`"
                }
            ),
        )
    })?;
    let sftp = s
        .sftp()
        .map_err(|e| Fallo::de(Tipo::Otro, format!("el subsistema SFTP no se abre: {e}")))?;
    Ok(Conexion { _sesion: s, sftp })
}

/// Lo que un listado dice de un fichero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrada {
    /// La ruta relativa a la raíz del usuario (`datos/docs/a.pdf`).
    pub clave: String,
    pub tamano: u64,
    /// Segundos desde 1970.
    pub mtime: u64,
    /// Un enlace simbólico: se lista y no se sigue.
    pub enlace: bool,
}

/// El validador de un fichero: lo que cambia si cambia (`<mtime>-<tamaño>`).
pub fn validador(tamano: u64, mtime: u64) -> String {
    format!("{mtime}-{tamano}")
}

/// **Un servidor SFTP, listo para leer**: su coordenada y una conexión que se
/// abre al primer uso y se reabre si se cae.
pub struct Sftp {
    pub fuente: Fuente,
    conexion: Mutex<Option<Arc<Conexion>>>,
}

impl Sftp {
    pub fn de(fuente: Fuente) -> Sftp {
        Sftp {
            fuente,
            conexion: Mutex::new(None),
        }
    }

    pub fn de_url(url: &str) -> Result<Sftp, String> {
        Ok(Sftp::de(leer(url)?))
    }

    /// **La huella del host**, sin autenticar: lo que el alta enseña para
    /// confirmarla.
    pub fn huella_del_host(&self) -> Result<String, Fallo> {
        saludar(&self.fuente).map(|(_, v)| v)
    }

    fn conexion(&self) -> Result<Arc<Conexion>, Fallo> {
        let mut g = self.conexion.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = g.as_ref() {
            return Ok(c.clone());
        }
        let c = Arc::new(conectar(&self.fuente)?);
        *g = Some(c.clone());
        Ok(c)
    }

    /// Conectar y autenticar (lo que `check` prueba antes de listar).
    pub fn entrar(&self) -> Result<(), Fallo> {
        self.conexion().map(|_| ())
    }

    fn ruta(clave: &str) -> PathBuf {
        PathBuf::from(format!("/{}", clave.trim_start_matches('/')))
    }

    /// **Todo lo que hay bajo un prefijo**, recorriendo los directorios desde
    /// el último `/` del prefijo; los enlaces, marcados y sin seguir.
    pub fn listar(&self, prefijo: &str) -> Result<Vec<Entrada>, Fallo> {
        let c = self.conexion()?;
        let dir = match prefijo.rfind('/') {
            Some(i) => &prefijo[..=i],
            None => "",
        };
        let mut out = Vec::new();
        let mut pila = vec![dir.trim_end_matches('/').to_string()];
        while let Some(d) = pila.pop() {
            let ruta = Sftp::ruta(&d);
            contar();
            let hijos = c
                .sftp
                .readdir(&ruta)
                .map_err(|e| Fallo::de_sftp(&format!("/{d}"), &e))?;
            for (p, st) in hijos {
                let nombre = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let clave = if d.is_empty() {
                    nombre
                } else {
                    format!("{d}/{nombre}")
                };
                let tipo = st.file_type();
                if tipo.is_dir() {
                    if clave.starts_with(prefijo) || prefijo.starts_with(&format!("{clave}/")) {
                        pila.push(clave);
                    }
                } else if (tipo.is_file() || tipo.is_symlink()) && clave.starts_with(prefijo) {
                    out.push(Entrada {
                        clave,
                        tamano: st.size.unwrap_or(0),
                        mtime: st.mtime.unwrap_or(0),
                        enlace: tipo.is_symlink(),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.clave.cmp(&b.clave));
        Ok(out)
    }

    /// **Un nivel**: las carpetas (acabadas en `/`) y los ficheros de `dir`
    /// (vacío o acabado en `/`), sin bajar. Lo que miran `check` y `explorar`
    /// sin recorrer un servidor entero.
    pub fn nivel(&self, dir: &str) -> Result<(Vec<String>, Vec<Entrada>), Fallo> {
        let c = self.conexion()?;
        let d = dir.trim_end_matches('/');
        contar();
        let hijos = c
            .sftp
            .readdir(Sftp::ruta(d))
            .map_err(|e| Fallo::de_sftp(&format!("/{d}"), &e))?;
        let (mut carpetas, mut ficheros) = (Vec::new(), Vec::new());
        for (p, st) in hijos {
            let nombre = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let clave = format!("{dir}{nombre}");
            let tipo = st.file_type();
            if tipo.is_dir() {
                carpetas.push(format!("{clave}/"));
            } else if tipo.is_file() || tipo.is_symlink() {
                ficheros.push(Entrada {
                    clave,
                    tamano: st.size.unwrap_or(0),
                    mtime: st.mtime.unwrap_or(0),
                    enlace: tipo.is_symlink(),
                });
            }
        }
        carpetas.sort();
        ficheros.sort_by(|a, b| a.clave.cmp(&b.clave));
        Ok((carpetas, ficheros))
    }

    /// El tamaño y el `mtime` de un fichero (`lstat`: un enlace no se sigue).
    pub fn mirar(&self, clave: &str) -> Result<(u64, u64), Fallo> {
        let c = self.conexion()?;
        contar();
        let st = c
            .sftp
            .lstat(&Sftp::ruta(clave))
            .map_err(|e| Fallo::de_sftp(clave, &e))?;
        if st.file_type().is_symlink() {
            return Err(Fallo::de(
                Tipo::Otro,
                format!("`{clave}` es un enlace simbólico: no se sigue"),
            ));
        }
        Ok((st.size.unwrap_or(0), st.mtime.unwrap_or(0)))
    }

    /// Si un fichero con este `mtime` es demasiado joven para copiarlo.
    pub fn joven(&self, mtime: u64) -> bool {
        ahora_s().saturating_sub(mtime) < self.fuente.edad
    }

    /// **El fichero entero, vigilado**: si `esperado` (un validador) no casa
    /// al abrir, o el fichero cambia mientras se lee, el lector falla.
    pub fn leer(&self, clave: &str, esperado: Option<&str>) -> Result<Vigilado<'_>, Fallo> {
        let (tamano, mtime) = self.mirar(clave)?;
        let v = validador(tamano, mtime);
        if let Some(e) = esperado
            && e.trim_matches('"') != v
        {
            return Err(Fallo::cambio(clave));
        }
        let c = self.conexion()?;
        contar();
        let fichero = c
            .sftp
            .open(Sftp::ruta(clave))
            .map_err(|e| Fallo::de_sftp(clave, &e))?;
        Ok(Vigilado {
            fichero,
            sftp: self,
            clave: clave.to_string(),
            validador: v,
            tamano,
        })
    }

    /// **Un rango** (`a-b`, `a-` o el sufijo `-n`), vigilado igual.
    pub fn rango(
        &self,
        clave: &str,
        rango: &str,
        esperado: Option<&str>,
    ) -> Result<Vec<u8>, Fallo> {
        let mut l = self.leer(clave, esperado)?;
        let (desde, hasta) = limites(rango, l.tamano).ok_or_else(|| {
            Fallo::de(Tipo::Otro, format!("`{rango}` no es un rango de `{clave}`"))
        })?;
        l.fichero
            .seek(SeekFrom::Start(desde))
            .map_err(|e| Fallo::de(Tipo::Otro, format!("`{clave}`: {e}")))?;
        let mut b = vec![0u8; (hasta - desde) as usize];
        l.fichero
            .read_exact(&mut b)
            .map_err(|e| Fallo::de(Tipo::Otro, format!("`{clave}`: {e}")))?;
        l.comprobar().map_err(|_| Fallo::cambio(clave))?;
        Ok(b)
    }
}

/// `[desde, hasta)` de un rango sobre un fichero de `tamano` bytes.
fn limites(rango: &str, tamano: u64) -> Option<(u64, u64)> {
    let (a, b) = rango.split_once('-')?;
    let (desde, hasta) = match (a.parse::<u64>().ok(), b.parse::<u64>().ok()) {
        (Some(a), Some(b)) => (a, b.saturating_add(1).min(tamano)),
        (Some(a), None) if b.is_empty() => (a, tamano),
        (None, Some(n)) if a.is_empty() => (tamano.saturating_sub(n), tamano),
        _ => return None,
    };
    (desde <= hasta && desde <= tamano).then_some((desde, hasta))
}

/// **Un fichero que se lee vigilado**: al llegar al final se vuelve a mirar,
/// y si el tamaño o el `mtime` ya no son los de al abrir, la lectura falla —un
/// fichero reescrito en sitio mientras se lee da una mezcla (O4·0)—.
pub struct Vigilado<'a> {
    fichero: ssh2::File,
    sftp: &'a Sftp,
    clave: String,
    validador: String,
    pub tamano: u64,
}

impl Vigilado<'_> {
    fn comprobar(&self) -> std::io::Result<()> {
        let (t, m) = self
            .sftp
            .mirar(&self.clave)
            .map_err(|f| std::io::Error::other(f.mensaje))?;
        if validador(t, m) != self.validador {
            return Err(std::io::Error::other(ore_objetos::memoria::cambio(
                &self.clave,
            )));
        }
        Ok(())
    }
}

impl Read for Vigilado<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.fichero.read(buf)?;
        if n == 0 && !buf.is_empty() {
            self.comprobar()?;
        }
        Ok(n)
    }
}

fn ahora_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `2026-10-09T17:20:52.000Z` de unos segundos (Howard Hinnant).
pub(crate) fn iso(s: u64) -> String {
    let dias = (s / 86_400) as i64;
    let z = dias + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let r = s % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        r / 3600,
        r / 60 % 60,
        r % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_url_dice_usuario_host_ruta_y_huella() {
        let f = leer("sftp://ore@sftp.cliente.com:2222/datos/diario?huella=SHA256:UrJeF35yPhfwvjVpV5aIbUf5uweUTmPR/XKLLXay4Do&edad=300").unwrap();
        assert_eq!(
            (
                f.usuario.as_str(),
                f.host.as_str(),
                f.puerto,
                f.prefijo.as_str()
            ),
            ("ore", "sftp.cliente.com", 2222, "datos/diario/")
        );
        assert_eq!(
            f.huella.as_deref(),
            Some("SHA256:UrJeF35yPhfwvjVpV5aIbUf5uweUTmPR/XKLLXay4Do")
        );
        assert_eq!((f.edad, f.legado), (300, false));
        assert_eq!(
            publica(&f, "datos/"),
            "sftp://ore@sftp.cliente.com:2222/datos/?huella=SHA256:UrJeF35yPhfwvjVpV5aIbUf5uweUTmPR/XKLLXay4Do&edad=300"
        );
        // una huella con `+` no se rompe, y la contraseña no se enseña
        let f = leer("sftp://ore:s%40creto@h/?huella=SHA256:ab+cd/efghijklmnopq").unwrap();
        assert_eq!(f.huella.as_deref(), Some("SHA256:ab+cd/efghijklmnopq"));
        assert_eq!(f.contrasena.as_deref(), Some("s@creto"));
        assert!(!publica(&f, "").contains("creto") && !format!("{f:?}").contains("creto"));
        assert_eq!((f.puerto, f.prefijo.as_str()), (22, ""));
        for mala in [
            "s3://c/",
            "sftp://host/x",
            "sftp://ore@/x",
            "sftp://ore@h:xx/",
            "sftp://ore@h/?huella=MD5:aa",
            "sftp://ore@h/?clave=x",
        ] {
            assert!(leer(mala).is_err(), "{mala}");
        }
    }

    #[test]
    fn los_rangos_y_el_validador() {
        assert_eq!(limites("0-3", 21), Some((0, 4)));
        assert_eq!(limites("-6", 21), Some((15, 21)));
        assert_eq!(limites("15-", 21), Some((15, 21)));
        assert_eq!(limites("10-999", 21), Some((10, 21)));
        assert_eq!(limites("30-", 21), None);
        assert_eq!(limites("x", 21), None);
        assert_eq!(validador(21, 1_791_571_760), "1791571760-21");
        assert_eq!(iso(1_791_566_452), "2026-10-09T17:20:52.000Z");
    }
}
