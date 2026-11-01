//! El árbol vive en la forja, y este proceso **no se lo queda**.
//!
//! # La forma: lo último de la forja en cada petición
//!
//! Cada petición que toca el árbol lee lo último de la forja, opera, y —si
//! escribió— empuja. Hasta 0046 E5b·2 eso era clonar entero cada vez, y medido
//! en victor el clon era casi toda la petición (1,5 s de 1,7). Ahora hay un
//! espejo en este proceso que se pone al día con un `fetch` (0,1 s) antes de
//! responder: ver [`ParaLeer`] y «El espejo», abajo. El espejo es una caché:
//!
//! lo que compra seguir así es que **el servidor no tiene estado**. Se puede matar, se
//! puede replicar, y dos réplicas no divergen porque no hay nada que diverja:
//! el sistema de registro es la forja.
//!
//! # Y la carrera se resuelve sola, que es todo el motivo
//!
//! Dos personas contestan la misma cola de decisiones a la vez. Las dos clonan
//! el mismo commit, las dos deciden, las dos empujan. **La segunda es
//! rechazada** —no avanza la referencia— y eso llega aquí como un `409`, no
//! como una ontología con las dos respuestas mezcladas.
//!
//! Esto no lo hemos escrito nosotros: es lo que hace git, y es la razón por la
//! que el árbol vive aquí y no en un volumen. Un fichero compartido habría
//! aceptado las dos escrituras y ninguna prueba lo habría notado.
//!
//! # El testigo no viaja en `argv`
//!
//! Un `git push http://usuario:token@host/…` deja la credencial en la línea de
//! órdenes, que lee cualquier proceso de la máquina. Va por `GIT_CONFIG_*`, que
//! git lee del **entorno** del hijo. Es la misma frontera que `source add`
//! traza con `connectionEnv`: el secreto se dice dónde está, no se escribe.

use ore_entrada::identidad::Identidad;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A dónde empujar, y con qué.
pub struct Forja {
    /// La URL del repositorio, con `.git`.
    pub url: String,
    /// El testigo. Sale de una variable de entorno y **nunca** de `argv`.
    ///
    /// No hay campo de usuario, y no es un olvido: la cabecera `Authorization:
    /// token …` no lleva ninguno. Un `usuario` aquí sería un dato que nadie
    /// mira y que el día que alguien lo cambiara no cambiaría nada.
    pub testigo: String,
}

#[derive(Debug)]
pub enum Fallo {
    /// No se pudo hablar con la forja.
    NoResponde(String),
    /// El empujón fue rechazado: alguien escribió antes.
    Adelantado(String),
    /// Cualquier otra cosa que git dijo.
    Git(String),
    /// La rama pedida no está en la forja (0030 W2).
    SinRama(String),
    /// Un merge con conflictos: los ficheros que chocan (0030 W2).
    Conflicto(Vec<String>),
    /// Un fichero movido de dentro a fuera de un alcance, o al revés (0044 A.2).
    Cruza(Vec<String>),
}

impl std::fmt::Display for Fallo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fallo::NoResponde(m) => write!(f, "la forja no responde: {m}"),
            Fallo::Adelantado(m) => write!(
                f,
                "alguien escribió en el árbol mientras se decidía esto, así que \
                 el empujón se rechazó y NADA se perdió. Hay que volver a leer y \
                 volver a decidir sobre lo que hay ahora: {m}"
            ),
            Fallo::Git(m) => write!(f, "git: {m}"),
            Fallo::SinRama(r) => write!(f, "no hay ninguna rama `{r}` en la forja"),
            Fallo::Cruza(fs) => write!(
                f,
                "se mueve a través del borde del repositorio, y eso no es sólo del repositorio: {}",
                fs.join(", ")
            ),
            Fallo::Conflicto(fs) => write!(
                f,
                "las dos ramas cambian lo mismo y git no sabe cuál vale: {}",
                fs.join(", ")
            ),
        }
    }
}

/// Una rama frente a otra ([`Forja::frente_a`]).
#[derive(Debug, Clone)]
pub struct Frente {
    /// El commit del que salió (`merge-base`).
    pub desde: String,
    pub cabeza: String,
    /// Commits de la rama que la base no tiene.
    pub adelante: u64,
    /// Commits de la base desde que la rama salió.
    pub atras: u64,
}

/// Un directorio temporal que se borra al soltarlo.
///
/// Sin esto, un servidor que atiende mil peticiones deja mil clones en el
/// disco. Y no se limpia «al final»: se limpia cuando el valor muere, incluidos
/// los caminos de error, que son justo los que se olvidan.
pub struct Prestado(PathBuf);

impl Prestado {
    pub fn ruta(&self) -> &Path {
        &self.0
    }
}

impl Drop for Prestado {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// **El plazo duro de un `git` de la forja** (F2). Lo que `http.lowSpeedLimit`
/// no ve —una conexión que no llega a abrirse: 135 s medidos en L3— y cualquier
/// otro cuelgue se cortan aquí, para que nadie retenga el candado del espejo
/// indefinidamente. Holgado: un clon entero de victor son 3,2 s en el pod (y
/// 5–12 s con la CPU disputada).
pub(crate) const PLAZO_GIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Corre `c` como mucho `plazo`; `None` si se pasó (y se mató). La salida se lee
/// en dos hilos —si no, una salida grande llenaría la tubería y el hijo se
/// pararía—, y al matarlo **no se espera a esos hilos**: el hijo de `git` que
/// habla HTTP puede seguir vivo con la tubería abierta hasta que el `lowSpeed`
/// o el kernel lo cierren, y esperarlo sería volver a colgarse.
fn salida_con_plazo(
    mut c: Command,
    plazo: std::time::Duration,
    entrada: Option<Vec<u8>>,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let mut h = c
        .stdin(if entrada.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // La entrada, en su hilo: escribirla aquí entera antes de leer la salida
    // puede atascar a los dos si las tuberías se llenan.
    if let (Some(e), Some(mut stdin)) = (entrada, h.stdin.take()) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&e);
        });
    }
    let leer = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = r.read_to_end(&mut b);
            b
        })
    };
    let salida = leer(Box::new(h.stdout.take().expect("stdout con tubería")));
    let error = leer(Box::new(h.stderr.take().expect("stderr con tubería")));
    let t = std::time::Instant::now();
    loop {
        if let Some(status) = h.try_wait()? {
            return Ok(Some(std::process::Output {
                status,
                stdout: salida.join().unwrap_or_default(),
                stderr: error.join().unwrap_or_default(),
            }));
        }
        if t.elapsed() >= plazo {
            let _ = h.kill();
            let _ = h.wait();
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

impl Forja {
    /// Los ajustes que van por el ENTORNO y no por la línea de órdenes.
    fn entorno(&self) -> Vec<(String, String)> {
        vec![
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_CONFIG_COUNT".into(), "4".into()),
            ("GIT_CONFIG_KEY_0".into(), "http.extraheader".into()),
            (
                "GIT_CONFIG_VALUE_0".into(),
                format!("Authorization: token {}", self.testigo),
            ),
            // El clon lo crea este proceso, así que el dueño coincide y esto no
            // haría falta. Se pone porque el día que el directorio venga de un
            // volumen el fallo es «dubious ownership», que no se parece en nada
            // a su causa y cuesta media hora encontrar.
            ("GIT_CONFIG_KEY_1".into(), "safe.directory".into()),
            ("GIT_CONFIG_VALUE_1".into(), "*".into()),
            // ⭐ F2 (2026-10-15): una forja que deja de mandar datos se abandona a
            //   los 10 s. Sin esto, medido en L3 (git 2.49.1): una forja que acepta
            //   y calla, o que se queda a medias, tiene a `git` esperando MÁS DE 6
            //   MINUTOS sin rendirse —con el candado del espejo tomado y todo
            //   ore-serve detrás: el atasco del 14 de octubre—. Con esto, 10 s.
            //   No cubre una conexión que no llega a abrirse (135 s, lo que dicta
            //   el kernel): eso lo corta [`PLAZO_GIT`].
            ("GIT_CONFIG_KEY_2".into(), "http.lowSpeedLimit".into()),
            ("GIT_CONFIG_VALUE_2".into(), "1000".into()),
            ("GIT_CONFIG_KEY_3".into(), "http.lowSpeedTime".into()),
            ("GIT_CONFIG_VALUE_3".into(), "10".into()),
        ]
    }

    fn git(&self, dir: Option<&Path>, args: &[&str]) -> Result<String, Fallo> {
        self.ejecutar(dir, args, None)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// **Muchos blobs en UN proceso** (F3a): `git cat-file --batch`, con los
    /// hashes por la entrada. Lo que no está, no viene. Antes era un
    /// `git cat-file blob <h>` por blob: con la CPU del pod, ~25 ms cada uno.
    fn blobs_por_lote(
        &self,
        pre: &[&str],
        hashes: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<u8>>, Fallo> {
        let mut fuera = std::collections::HashMap::new();
        if hashes.is_empty() {
            return Ok(fuera);
        }
        let mut args: Vec<&str> = pre.to_vec();
        args.extend(["cat-file", "--batch"]);
        let mut entrada = hashes.join("\n").into_bytes();
        entrada.push(b'\n');
        let b = self.ejecutar(None, &args, Some(entrada))?;
        // `<hash> <tipo> <tamaño>\n<contenido>\n`, o `<pedido> missing\n`.
        let mut i = 0;
        while i < b.len() {
            let Some(fin) = b[i..].iter().position(|x| *x == b'\n') else {
                break;
            };
            let cab = String::from_utf8_lossy(&b[i..i + fin]).into_owned();
            i += fin + 1;
            let partes: Vec<&str> = cab.split(' ').collect();
            let [h, _, tam] = partes.as_slice() else {
                continue; // `missing`
            };
            let Ok(n) = tam.parse::<usize>() else {
                continue;
            };
            let hasta = (i + n).min(b.len());
            fuera.insert(h.to_string(), b[i..hasta].to_vec());
            i = hasta + 1;
        }
        Ok(fuera)
    }

    /// Un `git` de la forja, con su plazo (F2), y la salida en bytes.
    fn ejecutar(
        &self,
        dir: Option<&Path>,
        args: &[&str],
        entrada: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Fallo> {
        let mut c = Command::new("git");
        if let Some(d) = dir {
            c.current_dir(d);
        }
        for (k, v) in self.entorno() {
            c.env(k, v);
        }
        let _turno = crate::fases::turno_de_proceso();
        let t = std::time::Instant::now();
        c.args(args);
        let s = salida_con_plazo(c, PLAZO_GIT, entrada)
            .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
        crate::fases::git(args, t.elapsed());
        let Some(s) = s else {
            eprintln!(
                "plazo · git {} · matado a los {} s",
                args.iter().take(4).copied().collect::<Vec<_>>().join(" "),
                PLAZO_GIT.as_secs()
            );
            return Err(Fallo::NoResponde(format!(
                "no contestó en {} s (`git {}`)",
                PLAZO_GIT.as_secs(),
                args.first().copied().unwrap_or_default()
            )));
        };
        let err = String::from_utf8_lossy(&s.stderr).into_owned();
        if s.status.success() {
            return Ok(s.stdout);
        }
        let bajo = err.to_ascii_lowercase();
        Err(if adelantado(&bajo) {
            Fallo::Adelantado(primera(&err))
        } else if bajo.contains("could not resolve host") || bajo.contains("connection refused") {
            Fallo::NoResponde(primera(&err))
        } else {
            Fallo::Git(primera(&err))
        })
    }

    /// La cabeza de una rama **sin clonar**: `git ls-remote`, milisegundos. Es
    /// lo que decide si `GET /assets` se sirve de memoria (0034 ⑤). `None` si
    /// la forja no contesta o la rama no está: entonces se clona y se calcula.
    pub fn cabeza_de(&self, rama: &str) -> Option<String> {
        let s = self
            .git(None, &["ls-remote", "--heads", &self.url, rama])
            .ok()?;
        let linea = s
            .lines()
            .find(|l| l.trim_end().ends_with(&format!("refs/heads/{rama}")))?;
        let hash = linea.split_whitespace().next()?.to_string();
        (!hash.is_empty()).then_some(hash)
    }

    /// **La rama existe, o nace de `main`** (0031 W3.7 gobierno ④): un puesto
    /// sin rama escribe en `<persona>/puesto`, y esa rama la crea el servidor
    /// por git —`push main:refs/heads/<rama>`— sin pedirle nada a la API de la
    /// forja, que en local no está. Idempotente: si `ls-remote` la ve, nada.
    /// Devuelve `true` si la creó.
    pub fn asegurar_rama(&self, rama: &str) -> Result<bool, Fallo> {
        if self.cabeza_de(rama).is_some() {
            return Ok(false);
        }
        let clon = self.clonar()?;
        let destino = format!("HEAD:refs/heads/{rama}");
        self.git(Some(clon.ruta()), &["push", "--quiet", "origin", &destino])?;
        Ok(true)
    }

    /// Un clon fresco de la rama por defecto, en un directorio que se borra solo.
    pub fn clonar(&self) -> Result<Prestado, Fallo> {
        self.clonar_rama(None)
    }

    /// Un clon fresco de UNA rama (0030 W2). `None` es la rama por defecto —
    /// `main`—, que es lo que todo hacía hasta hoy. Con rama, `HEAD` del clon
    /// es esa rama, así que `publicar` empuja a ella y no a `main`: una
    /// propuesta no toca lo que Flux mira.
    ///
    /// ⭐ 0046 E5b·2: del espejo al día, sin red; si el espejo no se puede
    ///   usar, de la forja como antes.
    pub fn clonar_rama(&self, rama: Option<&str>) -> Result<Prestado, Fallo> {
        match self.clonar_del_espejo(rama) {
            Ok(p) => Ok(p),
            Err(Fallo::SinRama(r)) => Err(Fallo::SinRama(r)),
            Err(_) => self.clonar_de_la_forja(rama),
        }
    }

    /// El clon de siempre, bajado de la forja.
    fn clonar_de_la_forja(&self, rama: Option<&str>) -> Result<Prestado, Fallo> {
        let destino = temporal();
        std::fs::create_dir_all(&destino)
            .map_err(|e| Fallo::Git(format!("no se pudo crear el directorio: {e}")))?;
        let prestado = Prestado(destino.clone());
        // ⛔ Sin `autocrlf`: el árbol es LF y el clon tiene que ser el árbol,
        //   byte a byte, en cualquier máquina. Medido en Windows (0030 W0):
        //   un Git de sistema con `autocrlf=true` dejaba CRLF en el clon,
        //   «reescribir lo mismo» parecía un cambio, y el commit vacío daba 502.
        let destino_s = destino.to_string_lossy().into_owned();
        let mut args = vec!["-c", "core.autocrlf=false", "clone", "--quiet"];
        if let Some(r) = rama {
            args.extend(["--branch", r]);
        }
        args.extend([self.url.as_str(), destino_s.as_str()]);
        self.git(None, &args).map_err(|e| match (rama, e) {
            (Some(r), Fallo::Git(m)) if m.contains("not found") || m.contains("Remote branch") => {
                Fallo::SinRama(r.to_string())
            }
            (_, e) => e,
        })?;
        Ok(prestado)
    }

    /// **Dónde está la rama de este clon frente a `base`** (ramas globales, fase
    /// 2): de qué commit salió —el `merge-base`, no la `base` de hoy: comparar
    /// contra lo que `base` hizo después pondría sus cambios en la rama, al
    /// revés—, su cabeza, y cuántos commits lleva cada una desde entonces.
    pub fn frente_a(&self, dir: &Path, base: &str) -> Result<Frente, Fallo> {
        self.git(Some(dir), &["fetch", "--quiet", "origin", base])
            .map_err(|_| Fallo::SinRama(base.to_string()))?;
        let desde = self
            .git(Some(dir), &["merge-base", "HEAD", "FETCH_HEAD"])?
            .trim()
            .to_string();
        let cabeza = self
            .git(Some(dir), &["rev-parse", "HEAD"])?
            .trim()
            .to_string();
        let cuentas = self.git(
            Some(dir),
            &["rev-list", "--left-right", "--count", "FETCH_HEAD...HEAD"],
        )?;
        let mut n = cuentas
            .split_whitespace()
            .map(|x| x.parse::<u64>().unwrap_or(0));
        let (atras, adelante) = (n.next().unwrap_or(0), n.next().unwrap_or(0));
        Ok(Frente {
            desde,
            cabeza,
            adelante,
            atras,
        })
    }

    /// El árbol del commit `c` de este clon, en un directorio aparte que se
    /// borra solo (un `worktree`: no se vuelve a clonar).
    pub fn extraer(&self, dir: &Path, c: &str) -> Result<Prestado, Fallo> {
        let destino = temporal();
        let prestado = Prestado(destino.clone());
        let destino_s = destino.to_string_lossy().into_owned();
        self.git(
            Some(dir),
            &[
                "-c",
                "core.autocrlf=false",
                "worktree",
                "add",
                "--quiet",
                "--detach",
                &destino_s,
                c,
            ],
        )?;
        Ok(prestado)
    }

    /// Cuándo se tocó `fichero` por última vez en este clon: `(segundos, ISO
    /// 8601)` del commit, o `None` si no tiene historia. Es lo que permite
    /// decir «encolada desde las 18:46» sin inventar un reloj.
    pub fn fecha_de(&self, dir: &Path, fichero: &str) -> Option<(i64, String)> {
        let s = self
            .git(
                Some(dir),
                &["log", "-1", "--format=%ct%n%cI", "--", fichero],
            )
            .ok()?;
        let mut l = s.lines();
        let seg = l.next()?.trim().parse().ok()?;
        Some((seg, l.next()?.trim().to_string()))
    }

    /// **Traer otra rama a este clon** (0030 W2, el «Merge» del menú): `git
    /// merge --no-ff` de `desde`, con la persona de autor y este servidor de
    /// committer, SIN empujar — el gate decide después si se empuja. `Ok(false)`
    /// es «ya estaba al día»; un conflicto deshace el merge y dice qué choca.
    pub fn traer(
        &self,
        dir: &Path,
        desde: &str,
        sujeto: &Identidad,
        mensaje: &str,
    ) -> Result<bool, Fallo> {
        if self
            .git(Some(dir), &["fetch", "--quiet", "origin", desde])
            .is_err()
        {
            return Err(Fallo::SinRama(desde.to_string()));
        }
        let mut c = Command::new("git");
        c.current_dir(dir);
        for (k, v) in self.entorno() {
            c.env(k, v);
        }
        // ⭐ El autor lleva el NOMBRE de la persona cuando el emisor lo afirma
        //   (claim `name`; 0030 W2, *Version history*), y el sujeto —la verdad—
        //   va en el correo: `<sub>@sujeto.invalid`. Sin nombre, el sujeto.
        c.env(
            "GIT_AUTHOR_NAME",
            sujeto.nombre.as_deref().unwrap_or(&sujeto.persona),
        )
        .env("GIT_AUTHOR_EMAIL", correo(&sujeto.persona))
        .env(
            "GIT_COMMITTER_NAME",
            sujeto.agente.clone().unwrap_or_else(|| "ore-serve".into()),
        )
        .env("GIT_COMMITTER_EMAIL", "ore-serve@ore.dev");
        let s = c
            .args(["merge", "--no-ff", "--no-commit", "FETCH_HEAD"])
            .output()
            .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
        let mut salida = String::from_utf8_lossy(&s.stdout).into_owned();
        salida.push('\n');
        salida.push_str(&String::from_utf8_lossy(&s.stderr));
        if salida.contains("Already up to date") {
            return Ok(false);
        }
        if s.status.success() {
            let mut c = Command::new("git");
            c.current_dir(dir);
            for (k, v) in self.entorno() {
                c.env(k, v);
            }
            c.env(
                "GIT_AUTHOR_NAME",
                sujeto.nombre.as_deref().unwrap_or(&sujeto.persona),
            )
            .env("GIT_AUTHOR_EMAIL", correo(&sujeto.persona))
            .env(
                "GIT_COMMITTER_NAME",
                sujeto.agente.clone().unwrap_or_else(|| "ore-serve".into()),
            )
            .env("GIT_COMMITTER_EMAIL", "ore-serve@ore.dev");
            let h = c
                .args(["commit", "-q", "--no-edit", "-m", mensaje])
                .output()
                .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
            if h.status.success() {
                return Ok(true);
            }
            salida.push_str(&String::from_utf8_lossy(&h.stderr));
        }
        let _ = self.git(Some(dir), &["merge", "--abort"]);
        let chocan: Vec<String> = salida
            .lines()
            .filter(|l| l.starts_with("CONFLICT"))
            .filter_map(|l| l.rsplit(' ').next())
            .map(|f| f.trim_end_matches('.').to_string())
            .collect();
        if chocan.is_empty() {
            Err(Fallo::Git(primera(&salida)))
        } else {
            Err(Fallo::Conflicto(chocan))
        }
    }

    /// Empuja lo que el clon ya tiene commiteado (un merge). Devuelve el commit.
    pub fn empujar(&self, dir: &Path) -> Result<String, Fallo> {
        self.git(Some(dir), &["push", "--quiet", "origin", "HEAD"])?;
        Ok(self
            .git(Some(dir), &["rev-parse", "--short", "HEAD"])?
            .trim()
            .to_string())
    }

    /// El sha **entero** de la cabeza del clon: la clave con la que el catálogo aplica lo que un
    /// commit lleva (`git:<sha>`, 0079 ·5c·5c), la misma que leerá el reconciliador.
    pub fn sha_de_la_cabeza(&self, dir: &Path) -> Result<String, Fallo> {
        Ok(self
            .git(Some(dir), &["rev-parse", "HEAD"])?
            .trim()
            .to_string())
    }

    /// ¿Cambió algo? Un `commit` vacío es ruido en la historia, y la historia
    /// **es** la auditoría: un commit por petición que no cambió nada convierte
    /// «quién cambió qué» en una lista de quién pasó por aquí.
    pub fn hay_cambios(&self, dir: &Path) -> bool {
        self.git(Some(dir), &["status", "--porcelain"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }

    /// Confirma lo que haya y lo empuja. Devuelve el commit.
    ///
    /// **El autor es el sujeto de la petición y el committer es el servidor.**
    /// No es un detalle de estilo: es la forma de RFC 8693 —`sub` más `act`—
    /// escrita donde se puede leer sin un sistema aparte. Quien decidió y qué
    /// lo ejecutó son dos cosas, y un registro que las funde no sirve para
    /// contestar ninguna de las dos.
    pub fn publicar(&self, dir: &Path, sujeto: &Identidad, mensaje: &str) -> Result<String, Fallo> {
        self.git(Some(dir), &["add", "-A"])?;
        self.confirmar(dir, sujeto, mensaje)?;
        self.git(Some(dir), &["push", "--quiet", "origin", "HEAD"])?;
        Ok(self
            .git(Some(dir), &["rev-parse", "--short", "HEAD"])?
            .trim()
            .to_string())
    }

    /// El commit de lo que ya está en el índice, con la persona de autor y este
    /// servidor de committer (lo de `publicar`), sin empujar.
    fn confirmar(&self, dir: &Path, sujeto: &Identidad, mensaje: &str) -> Result<(), Fallo> {
        self.confirmar_vacio(dir, sujeto, mensaje, false)
    }

    /// `confirmar`, y con `vacio` aunque no haya nada en el índice (la derivada que sólo lleva
    /// datos del catálogo).
    fn confirmar_vacio(
        &self,
        dir: &Path,
        sujeto: &Identidad,
        mensaje: &str,
        vacio: bool,
    ) -> Result<(), Fallo> {
        let mut c = Command::new("git");
        c.current_dir(dir);
        for (k, v) in self.entorno() {
            c.env(k, v);
        }
        // ⭐ El autor lleva el NOMBRE de la persona cuando el emisor lo afirma
        //   (claim `name`; 0030 W2, *Version history*), y el sujeto —la verdad—
        //   va en el correo: `<sub>@sujeto.invalid`. Sin nombre, el sujeto.
        c.env(
            "GIT_AUTHOR_NAME",
            sujeto.nombre.as_deref().unwrap_or(&sujeto.persona),
        )
        .env("GIT_AUTHOR_EMAIL", correo(&sujeto.persona))
        .env(
            "GIT_COMMITTER_NAME",
            sujeto.agente.clone().unwrap_or_else(|| "ore-serve".into()),
        )
        .env("GIT_COMMITTER_EMAIL", "ore-serve@ore.dev");
        let mut args = vec!["commit", "--quiet", "-m", mensaje];
        if vacio {
            args.push("--allow-empty");
        }
        let s = c
            .args(&args)
            .output()
            .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
        if !s.status.success() {
            return Err(Fallo::Git(primera(&String::from_utf8_lossy(&s.stderr))));
        }
        Ok(())
    }

    /// **Qué lleva el alcance de un repositorio** (0044 A.2 ①), en un clon
    /// que tiene las dos puntas: lo que la rama cambia bajo `prefijo` desde
    /// `desde`, **sin los documentos del catálogo** —un `.yaml`, un `.oob`, una
    /// política—, que son activos y se proponen desde el catálogo.
    /// Un fichero movido **a través del borde** del repositorio no se parte:
    /// `Fallo::Cruza`. Devuelve `(ficheros, documentos que se quedan, parche)`.
    fn alcance_en(
        &self,
        dir: &Path,
        desde: &str,
        cabeza: &str,
        alcance: &Alcance,
    ) -> Result<(Vec<String>, Vec<String>, String), Fallo> {
        let prefijo = match alcance {
            Alcance::Carpeta(p) => p.as_str(),
            Alcance::Rutas(rutas) => return self.rutas_en(dir, desde, cabeza, rutas),
        };
        let dentro = |r: &str| r == prefijo || r.starts_with(&format!("{prefijo}/"));
        let estados = self.git(Some(dir), &["diff", "--name-status", "-M", desde, cabeza])?;
        let mut ficheros = Vec::new();
        let mut fuera = Vec::new();
        let mut cruzan = Vec::new();
        for l in estados.lines() {
            let partes: Vec<&str> = l.split('\t').collect();
            let (antes, despues) = match partes.as_slice() {
                [e, a, d] if e.starts_with('R') || e.starts_with('C') => (*a, *d),
                [_, r] => (*r, *r),
                _ => continue,
            };
            if !dentro(antes) && !dentro(despues) {
                continue;
            }
            // lo que es del catálogo, a un lado u otro del movimiento, no va
            let documento = Self::es_documento(antes) || Self::es_documento(despues);
            if documento {
                for r in [antes, despues] {
                    if dentro(r) && !fuera.iter().any(|x| x == r) {
                        fuera.push(r.to_string());
                    }
                }
                continue;
            }
            if dentro(antes) != dentro(despues) {
                cruzan.push(format!("{antes} → {despues}"));
                continue;
            }
            ficheros.push(despues.to_string());
        }
        if !cruzan.is_empty() {
            return Err(Fallo::Cruza(cruzan));
        }
        if ficheros.is_empty() {
            return Ok((ficheros, fuera, String::new()));
        }
        let mut args: Vec<String> = ["diff", "--binary", "-M", desde, cabeza, "--", prefijo]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.extend(fuera.iter().map(|f| format!(":(exclude){f}")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let parche = self.git(Some(dir), &args)?;
        Ok((ficheros, fuera, parche))
    }

    /// **Qué lleva un alcance de activos** (0044 A.2, E2): sus ficheros, tal
    /// cual —de antes y de después si se movieron: un movimiento viaja entero—.
    /// Sin exclusiones: los ficheros ya son los de los activos elegidos.
    fn rutas_en(
        &self,
        dir: &Path,
        desde: &str,
        cabeza: &str,
        rutas: &[String],
    ) -> Result<(Vec<String>, Vec<String>, String), Fallo> {
        if rutas.is_empty() {
            return Ok((Vec::new(), Vec::new(), String::new()));
        }
        let mut args: Vec<&str> = vec!["diff", "--name-only", "-M", desde, cabeza, "--"];
        args.extend(rutas.iter().map(String::as_str));
        let ficheros: Vec<String> = self
            .git(Some(dir), &args)?
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        if ficheros.is_empty() {
            return Ok((ficheros, Vec::new(), String::new()));
        }
        let mut args: Vec<&str> = vec!["diff", "--binary", "-M", desde, cabeza, "--"];
        args.extend(rutas.iter().map(String::as_str));
        let parche = self.git(Some(dir), &args)?;
        Ok((ficheros, Vec::new(), parche))
    }

    /// ¿`ruta` es un documento del catálogo? Lo que el compilador carga
    /// (`validate.rs`): todo `.yaml`/`.yml` —uno sin `kind` no es «del
    /// repositorio»: el árbol no compila (`OOS1002`, medido)—, `.oob`, `.cedar`,
    /// `.cedarschema` y `ontology.lock`.
    fn es_documento(ruta: &str) -> bool {
        let bajo = ruta.to_ascii_lowercase();
        [".yaml", ".yml", ".oob", ".cedar", ".cedarschema"]
            .iter()
            .any(|e| bajo.ends_with(e))
            || bajo.ends_with("ontology.lock")
    }

    /// **La derivada de una propuesta con alcance** (0044 A.2 ③): el `base` de
    /// hoy más lo que `rama` cambia bajo `prefijo` desde que salió de `base`
    /// (`alcance_en`: sin los documentos del catálogo), aplicado a tres bandas.
    /// Copiar los ficheros de la rama NO vale: pisaría lo que `base` cambió
    /// después en esos mismos ficheros (medido).
    ///
    /// `Ok(None)` si la rama no cambia nada del repositorio ahí. Un choque con
    /// lo que `base` hizo después es `Fallo::Conflicto` con los ficheros. El
    /// commit lleva la **huella** —el parche del alcance—: es lo que se revisó.
    ///
    /// ⭐ **Los punteros de datos están en el catálogo** (0079), no en git: `datos` es la huella
    ///   de los del alcance (`fusion_catalogo::huella_de_datos`), que se suma a la del parche, y su
    ///   plan es del catálogo. Un alcance que sólo lleva datos hace una derivada sin cambios de
    ///   ficheros (un commit vacío con su huella). Los `datasets/**` que una forja aún guarde de
    ///   antes del catálogo no los lee nadie: no viajan (0079 ·5g·3, se borran en R4).
    #[allow(clippy::too_many_arguments)]
    pub fn derivar(
        &self,
        base: &str,
        rama: &str,
        alcance: &Alcance,
        sujeto: &Identidad,
        mensaje: &str,
        datos: Option<&str>,
    ) -> Result<Option<Derivada>, Fallo> {
        let clon = self.clonar_rama(Some(base))?;
        let dir = clon.ruta();
        self.git(Some(dir), &["fetch", "--quiet", "origin", rama])
            .map_err(|_| Fallo::SinRama(rama.to_string()))?;
        let desde = self
            .git(Some(dir), &["merge-base", "HEAD", "FETCH_HEAD"])?
            .trim()
            .to_string();
        let (ficheros, fuera, parche) = self.alcance_en(dir, &desde, "FETCH_HEAD", alcance)?;
        let (punteros, ficheros): (Vec<String>, Vec<String>) =
            ficheros.into_iter().partition(|f| es_puntero(f));
        if ficheros.is_empty() && datos.is_none() {
            return Ok(None);
        }
        let fichero = dir.join(".git").join("alcance.patch");
        std::fs::write(&fichero, &parche)
            .map_err(|e| Fallo::Git(format!("no se pudo escribir el parche: {e}")))?;
        let fichero_s = fichero.to_string_lossy().into_owned();
        let huella = self
            .git(Some(dir), &["hash-object", &fichero_s])?
            .trim()
            .to_string();
        let huella = match datos {
            Some(d) => format!("{huella}+{d}"),
            None => huella,
        };
        let mut aplicar: Vec<String> = vec!["apply".into(), "--3way".into(), "--index".into()];
        aplicar.extend(punteros.iter().map(|p| format!("--exclude={p}")));
        aplicar.push(fichero_s.clone());
        let aplicar: Vec<&str> = aplicar.iter().map(String::as_str).collect();
        if !ficheros.is_empty()
            && let Err(e) = self.git(Some(dir), &aplicar)
        {
            let chocan: Vec<String> = self
                .git(Some(dir), &["diff", "--name-only", "--diff-filter=U"])
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .filter(|l| !l.is_empty())
                .collect();
            return Err(if chocan.is_empty() {
                e
            } else {
                Fallo::Conflicto(chocan)
            });
        }
        // Sólo datos (con el catálogo): la derivada lleva su huella y ningún fichero.
        self.confirmar_vacio(
            dir,
            sujeto,
            &format!(
                "{mensaje}\n\nrama: {rama}\nalcance: {}\nhuella: {huella}",
                alcance.describe()
            ),
            ficheros.is_empty(),
        )?;
        Ok(Some(Derivada {
            clon,
            ficheros,
            fuera,
        }))
    }

    /// Empuja la cabeza de este clon a `destino`, **forzando**: la derivada se
    /// regenera, no se edita (0044 A.2 ③), y sólo la escribe este servidor.
    pub fn empujar_a(&self, dir: &Path, destino: &str) -> Result<(), Fallo> {
        let r = format!("HEAD:refs/heads/{destino}");
        self.git(Some(dir), &["push", "--quiet", "--force", "origin", &r])
            .map(|_| ())
    }

    /// `(la huella de hoy del alcance, la huella con la que se hizo la
    /// derivada)`. Distintas ⇒ la rama cambió lo propuesto desde entonces.
    ///
    /// `datos`: la huella de los datos del alcance con el catálogo (ver [`Forja::derivar`]).
    pub fn huellas(
        &self,
        base: &str,
        rama: &str,
        derivada: &str,
        alcance: &Alcance,
        datos: Option<&str>,
    ) -> Result<(String, String), Fallo> {
        let clon = self.clonar_rama(Some(rama))?;
        let dir = clon.ruta();
        self.git(Some(dir), &["fetch", "--quiet", "origin", base])
            .map_err(|_| Fallo::SinRama(base.to_string()))?;
        let desde = self
            .git(Some(dir), &["merge-base", "HEAD", "FETCH_HEAD"])?
            .trim()
            .to_string();
        let hoy = match self.alcance_en(dir, &desde, "HEAD", alcance) {
            Ok((fs, _, parche)) if !fs.is_empty() => {
                let fichero = dir.join(".git").join("alcance.patch");
                std::fs::write(&fichero, &parche)
                    .map_err(|e| Fallo::Git(format!("no se pudo escribir el parche: {e}")))?;
                self.git(Some(dir), &["hash-object", &fichero.to_string_lossy()])?
                    .trim()
                    .to_string()
            }
            Ok(_) => match datos {
                // Sólo datos: la huella del parche vacío, como la hizo `derivar`.
                Some(_) => {
                    let fichero = dir.join(".git").join("alcance.patch");
                    std::fs::write(&fichero, "")
                        .map_err(|e| Fallo::Git(format!("no se pudo escribir el parche: {e}")))?;
                    self.git(Some(dir), &["hash-object", &fichero.to_string_lossy()])?
                        .trim()
                        .to_string()
                }
                None => "-".into(),
            },
            // Lo que ya no se puede proponer tampoco está «al día».
            Err(_) => "✗".into(),
        };
        let hoy = match datos {
            Some(d) if hoy != "✗" => format!("{hoy}+{d}"),
            _ => hoy,
        };
        self.git(Some(dir), &["fetch", "--quiet", "origin", derivada])
            .map_err(|_| Fallo::SinRama(derivada.to_string()))?;
        let mensaje = self.git(Some(dir), &["log", "-1", "--format=%B", "FETCH_HEAD"])?;
        let hecha = mensaje
            .lines()
            .find_map(|l| l.strip_prefix("huella: "))
            .unwrap_or("")
            .trim()
            .to_string();
        Ok((hoy, hecha))
    }
}

/// **Lo que lleva una propuesta con alcance**, para git (0044 A.2): lo que la
/// rama cambia bajo la carpeta de un repositorio (sin documentos del catálogo),
/// o los ficheros de unos activos.
pub enum Alcance {
    Carpeta(String),
    Rutas(Vec<String>),
}

impl Alcance {
    pub fn describe(&self) -> String {
        match self {
            Alcance::Carpeta(p) => p.clone(),
            Alcance::Rutas(rs) => format!("{} ficheros de activos", rs.len()),
        }
    }
}

/// La derivada recién hecha: el clon con su commit (sin empujar; la huella va
/// en el mensaje) y los ficheros del alcance.
pub struct Derivada {
    pub clon: Prestado,
    pub ficheros: Vec<String>,
    /// Documentos del catálogo bajo la carpeta que la rama cambia y NO van.
    pub fuera: Vec<String>,
}

/// ¿Es un puntero de datos? (`datasets/**` y `copias/**`, `.json`.)
pub fn es_puntero(ruta: &str) -> bool {
    ruta.ends_with(".json")
        && ore_core::punteros::crudo::CARPETAS_AL_DIA
            .iter()
            .any(|c| ruta.starts_with(&format!("{c}/")))
}

/// El correo de un sujeto que no tiene correo.
///
/// Git exige uno y no hay ninguno que sea cierto: el sujeto es un identificador,
/// no una dirección. Se compone uno **que se ve que no es una dirección**, en
/// vez de inventar algo que parezca real y acabe en un `mailto:`.
fn correo(persona: &str) -> String {
    let limpio: String = persona
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '.'
            }
        })
        .collect();
    format!("{limpio}@sujeto.invalid")
}

/// ⭐ Las TRES caras de «alguien escribió antes» (medida W3.6b, 2026-09-20):
/// `non-fast-forward` / `fetch first` cuando el clon iba por detrás al empujar;
/// y `[remote rejected] … (incorrect old value provided)`, `failed to update
/// ref` o `cannot lock ref` cuando dos empujones llegan A LA VEZ y la forja
/// decide en su cerrojo. Ocho hilos sobre el mismo puntero daban 1 × 200 y
/// 7 × 502 «git: To file://…»: el CAS funcionaba y la respuesta mentía. Las
/// tres son 409.
fn adelantado(bajo: &str) -> bool {
    bajo.contains("non-fast-forward")
        || bajo.contains("fetch first")
        || bajo.contains("incorrect old value")
        || bajo.contains("failed to update ref")
        || bajo.contains("cannot lock ref")
        || bajo.contains("[remote rejected]")
}

/// La línea que dice algo: la del rechazo o el error si la hay («To file://…»
/// a secas, que es la primera de un `push` fallido, no le dice nada a nadie).
fn primera(s: &str) -> String {
    let lineas: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lineas
        .iter()
        .find(|l| l.contains("rejected") || l.starts_with("error:") || l.starts_with("fatal:"))
        .or(lineas.first())
        .map(|l| l.trim_start_matches("! ").to_string())
        .unwrap_or_default()
}

// ── El espejo: el árbol vivo (0046 E5b·2) ───────────────────────────────────
//
// Medido en victor, dentro del pod (0,5 CPU): clonar el árbol costaba 1,5–1,6 s
// y era casi toda la petición —`GET /paquetes` 1,7 s con 168 ficheros—; un
// `fetch` sobre un clon vivo, 0,07–0,10 s; un `worktree`, 0,08 s; cargar el
// árbol, 0,12 s. Con 2.000 tablas el clon crece y el `worktree` son 0,77 s.
//
// ⇒ Un espejo `--bare` por forja, en este proceso, que se pone al día con un
//   `fetch` en cada petición: se lee siempre lo último, como antes. Lo que ya
//   no se hace es bajarlo entero cada vez:
//   · para LEER, un `worktree` por commit, compartido por las peticiones que
//     lean ese commit mientras alguna lo use; si alguien lo ensucia, se rehace;
//   · para ESCRIBIR, un clon local del espejo (enlaces duros, sin red) con
//     `origin` en la forja: publicar, la carrera y el `409` no cambian.
//
// ⭐ El sistema de registro sigue siendo la forja. El espejo es una caché que
//   se reconstruye sola: matar el proceso no pierde nada, y dos réplicas no
//   divergen porque cada una hace `fetch` antes de responder. Si el espejo
//   falla, se clona como antes.

/// Cuántos árboles por commit se guardan (los que nadie esté leyendo).
const ARBOLES: usize = 16;

// ⭐ F3b (2026-10-15): antes, UN candado por espejo para todo —el `fetch`, montar
//   el `worktree`, superponer `main` y el `git status`—: con 50 ramas a la vez,
//   cada petición esperaba a que se montaran las de delante (L5, 1.000 activos y
//   50 usuarios: `candado 2×246 s`). Ahora son tres cosas:
//   · el `fetch`, uno a la vez y COMPARTIDO: quien llega mientras otro trae, o
//     antes de que empiece el siguiente, se sirve de ese —trae lo que había
//     cuando llegó, que es lo que prometía—;
//   · la lista de árboles, un candado corto (buscar, apuntar, soltar);
//   · montar un árbol, fuera del candado, y una sola vez por clave: quien pide
//     la misma rama mientras se monta, espera a ESE montaje.
struct Espejo {
    /// El repositorio `--bare`.
    dir: PathBuf,
    /// La lista de árboles, y qué se está montando.
    estado: std::sync::Mutex<Arboles>,
    /// Avisa cuando acaba un montaje.
    montado: std::sync::Condvar,
    /// Cuándo EMPEZÓ el último `fetch` que acabó bien.
    traido: std::sync::Mutex<Option<std::time::Instant>>,
}

#[derive(Default)]
struct Arboles {
    /// clave (el commit; en una rama por detrás de `main`, `rama+main`) → su
    /// árbol y el `git status` que tiene que tener, del más viejo al más nuevo.
    hechos: Vec<(String, std::sync::Arc<PathBuf>, String)>,
    /// Las claves que alguien está montando ahora.
    montando: std::collections::BTreeSet<String>,
    creados: u64,
}

/// El árbol para leer: compartido (un `worktree` del espejo) o propio (un clon
/// fresco, si el espejo no se pudo usar). Se suelta al salir de la petición.
pub enum ParaLeer {
    Compartido(std::sync::Arc<PathBuf>),
    Propio(Prestado),
}

impl ParaLeer {
    pub fn ruta(&self) -> &Path {
        match self {
            ParaLeer::Compartido(p) => p,
            ParaLeer::Propio(p) => p.ruta(),
        }
    }
}

fn espejos() -> &'static std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Espejo>>>
{
    static E: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Espejo>>>,
    > = std::sync::OnceLock::new();
    E.get_or_init(Default::default)
}

/// El candado del espejo, contando lo que se esperó (L1: [`crate::fases`]).
/// Todo lector del árbol pasa por aquí, y quien lo tiene puede estar hablando
/// con la forja: lo que espera el resto es lo que hay que ver.
fn bloquear<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    let t = std::time::Instant::now();
    let g = m.lock().unwrap_or_else(|x| x.into_inner());
    crate::fases::candado(t.elapsed());
    g
}

impl Forja {
    /// El espejo de esta forja, creado la primera vez. Por proceso: dos
    /// servidores en la misma máquina (las pruebas) no comparten directorio.
    fn espejo(&self) -> Result<std::sync::Arc<Espejo>, Fallo> {
        let t = std::time::Instant::now();
        let mut todos = espejos().lock().unwrap_or_else(|e| e.into_inner());
        crate::fases::candado(t.elapsed());
        if let Some(e) = todos.get(&self.url) {
            return Ok(e.clone());
        }
        let huella = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.url.hash(&mut h);
            h.finish()
        };
        let dir = std::env::temp_dir().join(format!(
            "ore-serve-espejo-{}-{huella:016x}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_s = dir.to_string_lossy().into_owned();
        self.git(
            None,
            &[
                "-c",
                "core.autocrlf=false",
                "clone",
                "--quiet",
                "--bare",
                &self.url,
                &dir_s,
            ],
        )?;
        let e = std::sync::Arc::new(Espejo {
            dir,
            estado: Default::default(),
            montado: Default::default(),
            traido: Default::default(),
        });
        todos.insert(self.url.clone(), e.clone());
        Ok(e)
    }

    /// El espejo, al día: todas las ramas de la forja, y las borradas fuera.
    /// **Compartido** (F3b): si otro `fetch` empezó después de que esta
    /// petición llegara, ya trae lo que había al llegar, y se usa ese.
    fn al_dia(&self, e: &Espejo) -> Result<(), Fallo> {
        let llegada = std::time::Instant::now();
        let mut traido = bloquear(&e.traido);
        if traido.is_some_and(|t| t >= llegada) {
            return Ok(());
        }
        let empieza = std::time::Instant::now();
        let d = e.dir.to_string_lossy().into_owned();
        self.git(
            None,
            &[
                "--git-dir",
                &d,
                "fetch",
                "--quiet",
                "--prune",
                "origin",
                "+refs/heads/*:refs/heads/*",
            ],
        )?;
        *traido = Some(empieza);
        Ok(())
    }

    /// El commit de una rama del espejo (`None`: la de por defecto).
    fn commit_de(&self, e: &Espejo, rama: Option<&str>) -> Result<String, Fallo> {
        let d = e.dir.to_string_lossy().into_owned();
        let r = match rama {
            Some(r) => format!("refs/heads/{r}^{{commit}}"),
            None => "HEAD^{commit}".to_string(),
        };
        self.git(
            None,
            &["--git-dir", &d, "rev-parse", "--verify", "--quiet", &r],
        )
        .map(|s| s.trim().to_string())
        .map_err(|_| Fallo::SinRama(rama.unwrap_or("HEAD").to_string()))
    }

    /// **Los ficheros de una carpeta en TODAS las ramas** (0050 F4): los que
    /// están directamente en `dir` y empiezan por `prefijo`, con la rama y su
    /// texto (`None`: la rama por defecto). Del espejo, al día con la forja.
    /// Un fichero que está en varias ramas sale una vez, de la primera que lo
    /// tiene (la de por defecto primero).
    ///
    /// Lo que pide: los informes de una función. Una invocación corre como un
    /// trabajo del puesto y su informe se confirma en la rama del puesto
    /// (`<persona>/puesto`), no en `main`, que está protegida: leer solo
    /// `main` las escondía todas (medido en t-victor el 2026-10-02).
    pub fn en_las_ramas(
        &self,
        dir: &str,
        prefijo: &str,
    ) -> Result<Vec<(Option<String>, String, String)>, Fallo> {
        let e = self.espejo()?;
        self.al_dia(&e)?;
        let d = e.dir.to_string_lossy().into_owned();
        let principal = self
            .git(None, &["--git-dir", &d, "symbolic-ref", "--short", "HEAD"])
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let mut ramas: Vec<String> = self
            .git(
                None,
                &[
                    "--git-dir",
                    &d,
                    "for-each-ref",
                    "--format=%(refname:short)",
                    "refs/heads",
                ],
            )?
            .lines()
            .map(str::to_string)
            .filter(|r| !r.is_empty())
            .collect();
        ramas.sort_by_key(|r| (*r != principal, r.clone()));
        let carpeta = format!("{}/", dir.trim_end_matches('/'));
        let mut vistos = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for r in ramas {
            let Ok(lista) = self.git(
                None,
                &[
                    "--git-dir",
                    &d,
                    "ls-tree",
                    "--name-only",
                    &format!("refs/heads/{r}"),
                    "--",
                    &carpeta,
                ],
            ) else {
                continue;
            };
            for ruta in lista.lines() {
                let nombre = ruta.rsplit('/').next().unwrap_or(ruta);
                if !nombre.starts_with(prefijo) || !vistos.insert(nombre.to_string()) {
                    continue;
                }
                if let Ok(t) = self.git(
                    None,
                    &["--git-dir", &d, "show", &format!("refs/heads/{r}:{ruta}")],
                ) {
                    out.push(((r != principal).then(|| r.clone()), ruta.to_string(), t));
                }
            }
        }
        Ok(out)
    }

    /// **Los commits recientes que llevan un bloque `Punteros:`** (0079 ·5c·5c·4, el
    /// reconciliador): de cada rama del espejo al día, los últimos `por_rama`, del más viejo al más
    /// nuevo y cada uno una vez (un merge está en varias ramas). `(sha, autor, mensaje)`.
    pub fn con_bloque(&self, por_rama: usize) -> Result<Vec<(String, String, String)>, Fallo> {
        let e = self.espejo()?;
        self.al_dia(&e)?;
        let d = e.dir.to_string_lossy().into_owned();
        let ramas: Vec<String> = self
            .git(
                None,
                &[
                    "--git-dir",
                    &d,
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/heads",
                ],
            )?
            .lines()
            .map(str::to_string)
            .filter(|r| !r.is_empty())
            .collect();
        let n = format!("-n{por_rama}");
        let mut vistos = std::collections::BTreeSet::new();
        let mut out: Vec<(i64, String, String, String)> = Vec::new();
        for r in ramas {
            let Ok(log) = self.git(
                None,
                &[
                    "--git-dir",
                    &d,
                    "log",
                    &n,
                    "--format=%H%x1f%ct%x1f%ae%x1f%B%x1e",
                    &r,
                ],
            ) else {
                continue;
            };
            for c in log.split('\x1e') {
                let partes: Vec<&str> = c.trim_start_matches('\n').splitn(4, '\x1f').collect();
                let [sha, cuando, autor, mensaje] = partes.as_slice() else {
                    continue;
                };
                if !mensaje.contains(ore_core::punteros::grabadora::PREFIJO)
                    || !vistos.insert(sha.to_string())
                {
                    continue;
                }
                out.push((
                    cuando.parse().unwrap_or(0),
                    sha.to_string(),
                    autor.to_string(),
                    mensaje.to_string(),
                ));
            }
        }
        out.sort();
        Ok(out.into_iter().map(|(_, s, a, m)| (s, a, m)).collect())
    }

    /// **El árbol de una rama, para leer** (`None`: la de por defecto). Lo
    /// último de la forja, sin clonarlo: un `fetch` y el `worktree` de su
    /// commit, que se comparte. Si el espejo falla, un clon como antes.
    pub fn para_leer(&self, rama: Option<&str>) -> Result<ParaLeer, Fallo> {
        self.para_leer_con_clave(rama, true).map(|(a, _)| a)
    }

    /// **Qué árbol se leería, sin montarlo** (F4): el espejo al día y la clave
    /// del árbol de lectura —el commit; en una rama por detrás de `main`,
    /// `rama+main`—. Es lo que identifica el índice de `/assets`: un `fetch`
    /// compartido y dos `rev-parse`, en vez de preguntar a la forja dos veces.
    pub fn clave_para_leer(&self, rama: Option<&str>) -> Result<String, Fallo> {
        let e = self.espejo()?;
        self.al_dia(&e)?;
        Ok(self.clave_de(&e, rama)?.0)
    }

    /// Como [`Forja::para_leer`], y la clave del árbol (`None`: no vino del
    /// espejo). Con `traer = false`, el espejo como está: quien acaba de pedir
    /// [`Forja::clave_para_leer`] no trae dos veces.
    pub fn para_leer_con_clave(
        &self,
        rama: Option<&str>,
        traer: bool,
    ) -> Result<(ParaLeer, Option<String>), Fallo> {
        match self.leer_del_espejo(rama, traer) {
            Ok((a, clave)) => Ok((ParaLeer::Compartido(a), Some(clave))),
            Err(Fallo::SinRama(r)) => Err(Fallo::SinRama(r)),
            Err(_) => Ok((ParaLeer::Propio(self.clonar_de_la_forja(rama)?), None)),
        }
    }

    /// `(clave, commit de la rama, commit de main)`.
    ///
    /// Es la clave de `ARBOLES` y de `/assets`, sin montar el árbol. Los punteros no entran: son
    /// del catálogo, con su `Punteros::version()`.
    fn clave_de(&self, e: &Espejo, rama: Option<&str>) -> Result<(String, String, String), Fallo> {
        let c = self.commit_de(e, rama)?;
        // ⭐ 0044: en una rama, los orígenes que no tocó se leen de `main` al día
        //   (`heredar_origenes`), así que el árbol depende también de la cabeza de `main`.
        let m = self.commit_de(e, None)?;
        let clave = if rama.is_some() && m != c {
            format!("{c}+{m}")
        } else {
            c.clone()
        };
        Ok((clave, c, m))
    }

    fn leer_del_espejo(
        &self,
        rama: Option<&str>,
        traer: bool,
    ) -> Result<(std::sync::Arc<PathBuf>, String), Fallo> {
        let e = self.espejo()?;
        if traer {
            self.al_dia(&e)?;
        }
        let (clave, c, m) = self.clave_de(&e, rama)?;
        loop {
            let mut a = bloquear(&e.estado);
            if let Some(i) = a.hechos.iter().position(|(h, _, _)| *h == clave) {
                let par = a.hechos.remove(i);
                let (dir, huella) = (par.1.clone(), par.2.clone());
                a.hechos.push(par);
                drop(a);
                // Como quedó, o se rehace: un árbol compartido que alguien tocó
                // ya no es el commit que dice ser. Fuera del candado (F3b): un
                // `status` de un árbol grande no hace esperar a las demás ramas.
                let limpio = dir.is_dir()
                    && self
                        .git(Some(&dir), &["status", "--porcelain", "--ignored"])
                        .is_ok_and(|s| s.trim() == huella.trim());
                if limpio {
                    return Ok((dir, clave));
                }
                let mut a = bloquear(&e.estado);
                if let Some(i) = a
                    .hechos
                    .iter()
                    .position(|(_, p, _)| std::sync::Arc::ptr_eq(p, &dir))
                {
                    a.hechos.remove(i);
                }
                drop(a);
                self.quitar_arbol(&e, &dir);
                continue;
            }
            if a.montando.contains(&clave) {
                // Otro lo está montando: se espera a ese montaje, y se vuelve a
                // mirar (si falló, se monta aquí).
                let t = std::time::Instant::now();
                drop(e.montado.wait(a).unwrap_or_else(|x| x.into_inner()));
                crate::fases::candado(t.elapsed());
                continue;
            }
            a.montando.insert(clave.clone());
            a.creados += 1;
            let n = a.creados;
            drop(a);
            let hecho = self.montar(&e, n, &c, (clave != c).then_some(m.as_str()));
            let mut a = bloquear(&e.estado);
            a.montando.remove(&clave);
            e.montado.notify_all();
            let (dir, huella) = hecho?;
            let dir = std::sync::Arc::new(dir);
            a.hechos.push((clave.clone(), dir.clone(), huella));
            // Los viejos que nadie lee, fuera (después, sin el candado).
            let mut fuera = Vec::new();
            while a.hechos.len() > ARBOLES {
                let Some(i) = a
                    .hechos
                    .iter()
                    .position(|(_, p, _)| std::sync::Arc::strong_count(p) == 1)
                else {
                    break;
                };
                fuera.push(a.hechos.remove(i).1);
            }
            drop(a);
            for viejo in fuera {
                self.quitar_arbol(&e, &viejo);
            }
            return Ok((dir, clave));
        }
    }

    /// Monta el árbol `n` del commit `c` —con los orígenes de `main` al día si
    /// viene `m`, ver [`Forja::heredar_origenes`]—
    /// y devuelve su `git status`, la huella de cómo tiene que seguir. Sin el
    /// candado: dos `worktree add` a la vez son seguros (git marca el que nace
    /// como `locked` y `worktree prune` lo respeta), y leer objetos también.
    fn montar(
        &self,
        e: &Espejo,
        n: u64,
        c: &str,
        m: Option<&str>,
    ) -> Result<(PathBuf, String), Fallo> {
        let dir = e.dir.with_extension(format!("arbol-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        let (d, dir_s) = (
            e.dir.to_string_lossy().into_owned(),
            dir.to_string_lossy().into_owned(),
        );
        let hecho = (|| {
            self.git(
                None,
                &[
                    "--git-dir",
                    &d,
                    "-c",
                    "core.autocrlf=false",
                    "worktree",
                    "add",
                    "--quiet",
                    "--detach",
                    &dir_s,
                    c,
                ],
            )?;
            if let Some(m) = m {
                let pre = ["--git-dir", d.as_str()];
                let base = self
                    .git(None, &["--git-dir", &d, "merge-base", m, c])
                    .map(|s| s.trim().to_string())
                    .ok();
                self.heredar_origenes(&pre, &dir, c, m, base.as_deref())?;
            }
            Ok(self
                .git(Some(&dir), &["status", "--porcelain", "--ignored"])
                .unwrap_or_default())
        })();
        match hecho {
            Ok(huella) => Ok((dir, huella)),
            Err(x) => {
                self.quitar_arbol(e, &dir);
                Err(x)
            }
        }
    }

    fn quitar_arbol(&self, e: &Espejo, dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
        let d = e.dir.to_string_lossy().into_owned();
        let _ = self.git(None, &["--git-dir", &d, "worktree", "prune"]);
    }

    /// **Los orígenes de la celda, en el árbol de una rama** (0044: las fuentes
    /// son «de la celda, no del árbol»). Lo mismo que [`ore_core::punteros::manda`]
    /// hace con cada puntero, con dos unidades más grandes:
    ///
    /// - un **paquete** (`packages/<n>/`) entero, por la huella de su árbol de
    ///   git: 17 ficheros que van juntos no se mezclan fichero a fichero;
    /// - una entrada de **`datasources`** de `ontology.config.yaml`, por nombre.
    ///
    /// Lo que la rama cambió o creó es suyo; lo que no tocó se lee de `main` al
    /// día —lo que `main` dio de alta después, también; lo que retiró,
    /// tampoco—. Sin esto, una rama anterior al alta de un origen lo listaba y no
    /// encontraba su esquema (`404`), y la consola se quedaba «leyendo».
    fn heredar_origenes(
        &self,
        pre: &[&str],
        dir: &Path,
        c: &str,
        m: &str,
        base: Option<&str>,
    ) -> Result<(), Fallo> {
        use ore_core::punteros::{Manda, manda};
        let git = |args: &[&str]| {
            let mut v: Vec<&str> = pre.to_vec();
            v.extend_from_slice(args);
            self.git(None, &v)
        };
        let paquetes = |commit: &str| -> std::collections::BTreeMap<String, String> {
            git(&["ls-tree", commit, "packages/"])
                .unwrap_or_default()
                .lines()
                .filter_map(|l| {
                    let (izq, ruta) = l.split_once('\t')?;
                    let mut p = izq.split_whitespace();
                    (p.nth(1)? == "tree").then_some(())?;
                    Some((
                        ruta.strip_prefix("packages/")?.to_string(),
                        p.next()?.to_string(),
                    ))
                })
                .collect()
        };
        let (en_rama, en_main) = (paquetes(c), paquetes(m));
        let en_base = base.map(paquetes).unwrap_or_default();
        let mut nombres: std::collections::BTreeSet<&String> = en_rama.keys().collect();
        nombres.extend(en_main.keys());
        nombres.extend(en_base.keys());
        for n in nombres {
            let r = en_rama.get(n).map(String::as_str);
            let ruta = dir.join("packages").join(n);
            match manda(
                r,
                en_base.get(n).map(String::as_str),
                en_main.get(n).map(String::as_str),
            ) {
                Manda::Rama => {}
                Manda::Main(h) if Some(h) == r => {}
                Manda::Main(_) => {
                    let _ = std::fs::remove_dir_all(&ruta);
                    let raiz = format!("packages/{n}");
                    let ficheros: Vec<(String, String)> = git(&["ls-tree", "-r", m, "--", &raiz])?
                        .lines()
                        .filter_map(|l| {
                            let (izq, f) = l.split_once('\t')?;
                            Some((f.to_string(), izq.split_whitespace().nth(2)?.to_string()))
                        })
                        .collect();
                    let hashes: Vec<&str> = ficheros.iter().map(|(_, h)| h.as_str()).collect();
                    let blobs = self.blobs_por_lote(pre, &hashes)?;
                    for (f, h) in &ficheros {
                        let bytes = blobs.get(h).ok_or_else(|| {
                            Fallo::Git(format!("`{h}` ({f}) no está en el espejo"))
                        })?;
                        let destino = dir.join(f);
                        if let Some(p) = destino.parent() {
                            let _ = std::fs::create_dir_all(p);
                        }
                        std::fs::write(&destino, bytes)
                            .map_err(|e| Fallo::Git(format!("no se pudo escribir `{f}`: {e}")))?;
                    }
                }
                Manda::Ninguno if r.is_some() => {
                    let _ = std::fs::remove_dir_all(&ruta);
                }
                Manda::Ninguno => {}
            }
        }
        // Y la lista de orígenes: lo que `GET /fuentes` lee del manifiesto.
        let manifiesto =
            |commit: &str| git(&["show", &format!("{commit}:ontology.config.yaml")]).ok();
        if let (Some(r), Some(mn)) = (manifiesto(c), manifiesto(m)) {
            let b = base.and_then(manifiesto);
            if let Some((nuevo, _)) = fuentes_al_dia(&r, b.as_deref(), &mn) {
                std::fs::write(dir.join("ontology.config.yaml"), nuevo).map_err(|e| {
                    Fallo::Git(format!("no se pudo escribir `ontology.config.yaml`: {e}"))
                })?;
            }
        }
        Ok(())
    }

    /// Un clon para escribir, del espejo al día (sin red: enlaces duros) y con
    /// `origin` en la forja. `None` si el espejo no se pudo usar.
    fn clonar_del_espejo(&self, rama: Option<&str>) -> Result<Prestado, Fallo> {
        let e = self.espejo()?;
        self.al_dia(&e)?;
        self.commit_de(&e, rama)?;
        let destino = temporal();
        let prestado = Prestado(destino.clone());
        let (d, destino_s) = (
            e.dir.to_string_lossy().into_owned(),
            destino.to_string_lossy().into_owned(),
        );
        let mut args = vec!["-c", "core.autocrlf=false", "clone", "--quiet"];
        if let Some(r) = rama {
            args.extend(["--branch", r]);
        }
        args.extend([d.as_str(), destino_s.as_str()]);
        self.git(None, &args)?;
        self.git(
            Some(&destino),
            &["remote", "set-url", "origin", self.url.as_str()],
        )?;
        Ok(prestado)
    }
}

fn temporal() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "ore-serve-arbol-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// La sección `datasources:` de un `ontology.config.yaml`, en trozos: lo de antes,
/// cada entrada con su texto (y su nombre), y lo de después. A mano y no con el
/// intérprete porque lo que se escribe de vuelta es el texto de `main`, tal cual:
/// con sus comillas y sus comentarios.
struct Fuentes {
    antes: Vec<String>,
    entradas: Vec<(String, String)>,
    despues: Vec<String>,
}

fn nombre_de_entrada(bloque: &str) -> Option<String> {
    let t = bloque.trim_start().strip_prefix('-')?.trim_start();
    let mut resto = t.strip_prefix('{').map_or(t, str::trim_start);
    // `name:` puede venir primero (`- name: x`) o en medio de la forma corta.
    if !resto.starts_with("name:") {
        resto = &resto[resto.find(" name:")? + 1..];
    }
    let v = resto["name:".len()..].lines().next()?;
    let v = v.split([',', '}']).next()?.trim().trim_matches(['"', '\'']);
    (!v.is_empty()).then(|| v.to_string())
}

fn trocear_fuentes(texto: &str) -> Option<Fuentes> {
    let lineas: Vec<&str> = texto.lines().collect();
    let h = lineas.iter().position(|l| l.trim_end() == "datasources:")?;
    let fin = (h + 1..lineas.len())
        .find(|&i| {
            let l = lineas[i];
            !l.is_empty() && !l.starts_with([' ', '#', '-'])
        })
        .unwrap_or(lineas.len());
    let sangria = |l: &str| l.len() - l.trim_start().len();
    let es_entrada = |l: &str| l.trim_start().starts_with("- ");
    let primera = (h + 1..fin).find(|&i| es_entrada(lineas[i]));
    let mut f = Fuentes {
        antes: lineas[..=h].iter().map(|l| l.to_string()).collect(),
        entradas: Vec::new(),
        despues: lineas[fin..].iter().map(|l| l.to_string()).collect(),
    };
    let Some(p) = primera else {
        f.antes
            .extend(lineas[h + 1..fin].iter().map(|l| l.to_string()));
        return Some(f);
    };
    f.antes
        .extend(lineas[h + 1..p].iter().map(|l| l.to_string()));
    let nivel = sangria(lineas[p]);
    let mut inicio = p;
    let mut cortes: Vec<usize> = (p + 1..fin)
        .filter(|&i| es_entrada(lineas[i]) && sangria(lineas[i]) == nivel)
        .collect();
    cortes.push(fin);
    for corte in cortes {
        let mut hasta = corte;
        if corte == fin {
            // Lo que sobra al final (líneas en blanco y comentarios) no es de la entrada.
            while hasta > inicio + 1 && {
                let l = lineas[hasta - 1].trim();
                l.is_empty() || l.starts_with('#')
            } {
                hasta -= 1;
            }
            let mut resto: Vec<String> = lineas[hasta..fin].iter().map(|l| l.to_string()).collect();
            resto.append(&mut f.despues);
            f.despues = resto;
        }
        let bloque = lineas[inicio..hasta].join("\n");
        let nombre = nombre_de_entrada(&bloque)?;
        f.entradas.push((nombre, bloque));
        inicio = corte;
    }
    Some(f)
}

/// **Las fuentes de la rama, al día con `main`** (ver [`Forja::heredar_origenes`]):
/// por nombre, con [`ore_core::punteros::manda`] sobre el texto de la entrada.
/// `None` si no cambia nada. Devuelve el manifiesto nuevo y qué entradas tocó.
fn fuentes_al_dia(
    rama: &str,
    base: Option<&str>,
    main: &str,
) -> Option<(String, Vec<(String, &'static str)>)> {
    use ore_core::punteros::{Manda, manda};
    let m = trocear_fuentes(main)?;
    let r = trocear_fuentes(rama).unwrap_or_else(|| Fuentes {
        antes: rama
            .lines()
            .map(String::from)
            .chain(["datasources:".to_string()])
            .collect(),
        entradas: Vec::new(),
        despues: Vec::new(),
    });
    let b = base.and_then(trocear_fuentes);
    let de = |f: &Fuentes, n: &str| {
        f.entradas
            .iter()
            .find(|(x, _)| x == n)
            .map(|(_, t)| t.trim_end().to_string())
    };
    let mut nombres: Vec<&String> = r.entradas.iter().map(|(n, _)| n).collect();
    for (n, _) in m
        .entradas
        .iter()
        .chain(b.iter().flat_map(|b| b.entradas.iter()))
    {
        if !nombres.contains(&n) {
            nombres.push(n);
        }
    }
    let mut entradas: Vec<(String, String)> = r.entradas.clone();
    let mut tocadas = Vec::new();
    for n in nombres {
        let (en_r, en_m) = (de(&r, n), de(&m, n));
        let en_b = b.as_ref().and_then(|b| de(b, n));
        match manda(en_r.as_deref(), en_b.as_deref(), en_m.as_deref()) {
            Manda::Rama => {}
            Manda::Main(h) if Some(h) == en_r.as_deref() => {}
            Manda::Main(h) => {
                match entradas.iter_mut().find(|(x, _)| x == n) {
                    Some(e) => e.1 = h.to_string(),
                    None => entradas.push((n.clone(), h.to_string())),
                }
                tocadas.push((n.clone(), "main"));
            }
            Manda::Ninguno if en_r.is_some() => {
                entradas.retain(|(x, _)| x != n);
                tocadas.push((n.clone(), "ninguno"));
            }
            Manda::Ninguno => {}
        }
    }
    if tocadas.is_empty() {
        return None;
    }
    let mut lineas = r.antes;
    lineas.extend(
        entradas
            .into_iter()
            .flat_map(|(_, t)| t.lines().map(String::from).collect::<Vec<_>>()),
    );
    lineas.extend(r.despues);
    Some((lineas.join("\n") + "\n", tocadas))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// F2: lo que se pasa del plazo se mata y se dice (`None`), sin esperarlo.
    #[cfg(unix)]
    #[test]
    fn lo_que_se_pasa_del_plazo_se_mata() {
        let t = std::time::Instant::now();
        let mut c = Command::new("sleep");
        c.arg("30");
        let s = salida_con_plazo(c, std::time::Duration::from_millis(500), None).unwrap();
        assert!(s.is_none());
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        // y lo que acaba a tiempo trae su salida
        let mut c = Command::new("git");
        c.arg("--version");
        let s = salida_con_plazo(c, std::time::Duration::from_secs(10), None)
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&s.stdout).starts_with("git version"));
    }

    /// F2: una forja que acepta la conexión y calla —el caso B de L3: más de 6
    /// minutos sin el plazo— se abandona en ~10 s (`http.lowSpeedTime`).
    #[test]
    fn una_forja_que_calla_se_abandona_en_segundos() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let puerto = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut quietas = Vec::new();
            for c in l.incoming().flatten() {
                quietas.push(c); // acepta y no dice nada
            }
        });
        let f = Forja {
            url: format!("http://127.0.0.1:{puerto}/r.git"),
            testigo: String::new(),
        };
        let t = std::time::Instant::now();
        let e = f
            .git(None, &["ls-remote", "--heads", &f.url])
            .expect_err("una forja que calla no contesta");
        let d = t.elapsed();
        // Es el plazo de `lowSpeed` lo que corta, no otra cosa que falle antes.
        assert!(e.to_string().contains("too slow"), "{e} ({d:?})");
        assert!(d < std::time::Duration::from_secs(30), "tardó {d:?}");
    }

    #[test]
    fn el_testigo_no_aparece_en_los_argumentos() {
        let f = Forja {
            url: "http://forja/x.git".into(),
            testigo: "SECRETO".into(),
        };
        // Va en el entorno, y sólo ahí.
        let e = f.entorno();
        assert!(e.iter().any(|(_, v)| v.contains("SECRETO")));
        assert!(!f.url.contains("SECRETO"));
    }

    /// Un correo compuesto tiene que verse compuesto. `.invalid` está reservado
    /// por el RFC 2606 justo para esto.
    #[test]
    fn el_correo_del_sujeto_no_finge_ser_uno() {
        assert_eq!(correo("persona:ana"), "persona.ana@sujeto.invalid");
        assert!(correo("cualquiera").ends_with("@sujeto.invalid"));
    }

    #[test]
    fn el_prestado_se_borra_solo() {
        let d = temporal();
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), "x").unwrap();
        let camino = d.clone();
        {
            let _p = Prestado(d);
            assert!(camino.exists());
        }
        assert!(!camino.exists(), "el clon sobrevivió a quien lo pidió");
    }

    /// **La rama del puesto nace de `main` por git** (W3.7 gobierno ④): una
    /// forja pelada sin API; la primera vez se crea, la segunda ya está.
    #[test]
    fn la_rama_del_puesto_nace_de_main_y_una_vez() {
        let d = temporal();
        let pelada = d.join("arbol.git");
        let semilla = d.join("semilla");
        std::fs::create_dir_all(&semilla).unwrap();
        let corre = |args: &[&str], cwd: &Path| {
            let s = Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "s")
                .env("GIT_AUTHOR_EMAIL", "s@x")
                .env("GIT_COMMITTER_NAME", "s")
                .env("GIT_COMMITTER_EMAIL", "s@x")
                .output()
                .unwrap();
            assert!(
                s.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&s.stderr)
            );
        };
        corre(
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                pelada.to_str().unwrap(),
            ],
            &d,
        );
        corre(&["init", "-q", "-b", "main"], &semilla);
        std::fs::write(
            semilla.join("README.md"),
            "hola
",
        )
        .unwrap();
        corre(&["add", "-A"], &semilla);
        corre(&["commit", "-qm", "semilla"], &semilla);
        corre(
            &["push", "-q", pelada.to_str().unwrap(), "HEAD:main"],
            &semilla,
        );
        let f = Forja {
            url: format!("file://{}", pelada.to_string_lossy().replace('\\', "/")),
            testigo: String::new(),
        };
        assert!(f.cabeza_de("ana/puesto").is_none());
        assert!(
            f.asegurar_rama("ana/puesto").unwrap(),
            "la primera vez se crea"
        );
        assert_eq!(f.cabeza_de("ana/puesto"), f.cabeza_de("main"));
        assert!(
            !f.asegurar_rama("ana/puesto").unwrap(),
            "la segunda ya está"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⭐ 0046 E5b·2 · **El espejo lee siempre lo último, y no se deja
    /// ensuciar.** Una forja pelada: lo que otro empuja se ve en la lectura
    /// siguiente (el `fetch` de cada petición); dos lecturas del mismo commit
    /// comparten árbol; uno ensuciado se rehace; una rama que no está es
    /// `SinRama`; y escribir clona del espejo y empuja a la forja.
    #[test]
    fn el_espejo_lee_lo_ultimo_y_no_se_ensucia() {
        let d = temporal();
        let pelada = d.join("arbol.git");
        let otro = d.join("otro");
        std::fs::create_dir_all(&otro).unwrap();
        let corre = |args: &[&str], cwd: &Path| {
            let s = Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "s")
                .env("GIT_AUTHOR_EMAIL", "s@x")
                .env("GIT_COMMITTER_NAME", "s")
                .env("GIT_COMMITTER_EMAIL", "s@x")
                .output()
                .unwrap();
            assert!(
                s.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&s.stderr)
            );
        };
        corre(
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                pelada.to_str().unwrap(),
            ],
            &d,
        );
        corre(&["init", "-q", "-b", "main"], &otro);
        std::fs::write(otro.join("a.txt"), "uno\n").unwrap();
        corre(&["add", "-A"], &otro);
        corre(&["commit", "-qm", "uno"], &otro);
        corre(
            &["push", "-q", pelada.to_str().unwrap(), "HEAD:main"],
            &otro,
        );
        let f = Forja {
            url: format!("file://{}", pelada.to_string_lossy().replace('\\', "/")),
            testigo: String::new(),
        };
        let leer = |f: &Forja| {
            let a = f.para_leer(None).unwrap();
            assert!(matches!(a, ParaLeer::Compartido(_)), "no vino del espejo");
            (
                a.ruta().to_path_buf(),
                std::fs::read_to_string(a.ruta().join("a.txt")).unwrap(),
            )
        };
        let (r1, t1) = leer(&f);
        assert_eq!(t1, "uno\n");
        let (r2, _) = leer(&f);
        assert_eq!(r1, r2, "el mismo commit comparte árbol");

        // Otro empuja: la lectura siguiente lo ve.
        std::fs::write(otro.join("a.txt"), "dos\n").unwrap();
        corre(&["commit", "-qam", "dos"], &otro);
        corre(
            &["push", "-q", pelada.to_str().unwrap(), "HEAD:main"],
            &otro,
        );
        let (r3, t3) = leer(&f);
        assert_eq!(t3, "dos\n", "el espejo no se puso al día");
        assert_ne!(r3, r1);

        // Alguien ensucia el árbol compartido: se rehace, limpio.
        std::fs::write(r3.join("a.txt"), "roto\n").unwrap();
        let (_, t4) = leer(&f);
        assert_eq!(t4, "dos\n", "se sirvió un árbol ensuciado");

        assert!(matches!(
            f.para_leer(Some("no-esta")),
            Err(Fallo::SinRama(_))
        ));

        // Escribir: un clon del espejo que empuja a la forja.
        let sujeto = Identidad {
            persona: "persona:ana".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: None,
            usuario: None,
        };
        let clon = f.clonar().unwrap();
        std::fs::write(clon.ruta().join("b.txt"), "nuevo\n").unwrap();
        f.publicar(clon.ruta(), &sujeto, "b").unwrap();
        drop(clon);
        let a = f.para_leer(None).unwrap();
        assert_eq!(
            std::fs::read_to_string(a.ruta().join("b.txt")).unwrap(),
            "nuevo\n",
            "lo escrito no llegó a la forja, o el espejo no lo trajo"
        );
        drop(a);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// F3b · **Muchas ramas a la vez**: cada lectura ve SU rama (el `fetch`
    /// compartido y los montajes fuera del candado no las cruzan), y quienes
    /// piden la misma rama a la vez comparten un solo árbol.
    #[test]
    fn muchas_ramas_a_la_vez_cada_una_la_suya() {
        let d = temporal();
        let pelada = d.join("arbol.git");
        let otro = d.join("otro");
        std::fs::create_dir_all(&otro).unwrap();
        let corre = |args: &[&str], cwd: &Path| {
            let s = Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "s")
                .env("GIT_AUTHOR_EMAIL", "s@x")
                .env("GIT_COMMITTER_NAME", "s")
                .env("GIT_COMMITTER_EMAIL", "s@x")
                .output()
                .unwrap();
            assert!(
                s.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&s.stderr)
            );
        };
        let pel = pelada.to_str().unwrap().to_string();
        corre(&["init", "-q", "--bare", "-b", "main", &pel], &d);
        corre(&["init", "-q", "-b", "main"], &otro);
        std::fs::write(otro.join("a.txt"), "main\n").unwrap();
        corre(&["add", "-A"], &otro);
        corre(&["commit", "-qm", "main"], &otro);
        corre(&["push", "-q", &pel, "HEAD:main"], &otro);
        const RAMAS: usize = 6;
        for i in 0..RAMAS {
            corre(&["checkout", "-q", "-b", &format!("r{i}"), "main"], &otro);
            std::fs::write(otro.join("a.txt"), format!("r{i}\n")).unwrap();
            corre(&["commit", "-qam", "r"], &otro);
            corre(&["push", "-q", &pel, &format!("r{i}")], &otro);
        }
        let f = std::sync::Arc::new(Forja {
            url: format!("file://{}", pelada.to_string_lossy().replace('\\', "/")),
            testigo: String::new(),
        });
        let hilos: Vec<_> = (0..RAMAS * 3)
            .map(|k| {
                let f = f.clone();
                std::thread::spawn(move || {
                    let r = format!("r{}", k % RAMAS);
                    let a = f.para_leer(Some(&r)).unwrap();
                    assert!(matches!(a, ParaLeer::Compartido(_)), "no vino del espejo");
                    let t = std::fs::read_to_string(a.ruta().join("a.txt")).unwrap();
                    (r, t, a.ruta().to_path_buf())
                })
            })
            .collect();
        let mut dirs: std::collections::BTreeMap<String, std::collections::BTreeSet<PathBuf>> =
            Default::default();
        for h in hilos {
            let (r, t, dir) = h.join().unwrap();
            assert_eq!(t, format!("{r}\n"), "la rama `{r}` leyó otra");
            dirs.entry(r).or_default().insert(dir);
        }
        for (r, ds) in &dirs {
            assert_eq!(ds.len(), 1, "`{r}` se montó más de una vez: {ds:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⭐ 0050 F4 · **Los informes de una función, en todas las ramas.** Una
    /// invocación confirma su informe en la rama del puesto (`ana/puesto`) y
    /// no en `main`: se leen los de las dos, sin repetir los que la rama trae
    /// de `main`, y cada uno dice de qué rama es (`None`: la de por defecto).
    #[test]
    fn los_ficheros_de_una_carpeta_en_todas_las_ramas() {
        let d = temporal();
        let pelada = d.join("arbol.git");
        let otro = d.join("otro");
        std::fs::create_dir_all(&otro).unwrap();
        let corre = |args: &[&str], cwd: &Path| {
            let s = Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "s")
                .env("GIT_AUTHOR_EMAIL", "s@x")
                .env("GIT_COMMITTER_NAME", "s")
                .env("GIT_COMMITTER_EMAIL", "s@x")
                .output()
                .unwrap();
            assert!(
                s.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&s.stderr)
            );
        };
        let p = pelada.to_str().unwrap();
        corre(&["init", "-q", "--bare", "-b", "main", p], &d);
        corre(&["init", "-q", "-b", "main"], &otro);
        std::fs::create_dir_all(otro.join("resultados")).unwrap();
        let (de_main, de_ana) = ("{\"de\": \"main\"}", "{\"de\": \"ana\"}");
        std::fs::write(otro.join("resultados/v_f_20261001T000000Z.json"), de_main).unwrap();
        std::fs::write(otro.join("resultados/v_fx_20261001T000000Z.json"), "{}").unwrap();
        corre(&["add", "-A"], &otro);
        corre(&["commit", "-qm", "main"], &otro);
        corre(&["push", "-q", p, "HEAD:main"], &otro);
        std::fs::write(otro.join("resultados/v_f_20261002T065806Z.json"), de_ana).unwrap();
        corre(&["add", "-A"], &otro);
        corre(&["commit", "-qm", "invocar"], &otro);
        corre(&["push", "-q", p, "HEAD:refs/heads/ana/puesto"], &otro);
        let f = Forja {
            url: format!("file://{}", pelada.to_string_lossy().replace('\\', "/")),
            testigo: String::new(),
        };
        let mut v = f.en_las_ramas("resultados", "v_f_").unwrap();
        v.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            v,
            vec![
                (
                    None,
                    "resultados/v_f_20261001T000000Z.json".to_string(),
                    de_main.to_string()
                ),
                (
                    Some("ana/puesto".to_string()),
                    "resultados/v_f_20261002T065806Z.json".to_string(),
                    de_ana.to_string()
                ),
            ]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n\n# lo escribe `ore source add`\ndatasources:\n  - name: pg\n    type: postgres\n    connectionEnv: PG_URL\n  - { name: s3, type: s3, connectionEnv: S3_URL }\n";

    fn con(fuentes: &[&str]) -> String {
        let mut t = CONFIG.to_string();
        for f in fuentes {
            t.push_str(f);
            t.push('\n');
        }
        t
    }

    const MONGO: &str = "  - name: mongov2\n    type: mongodb\n    connectionEnv: V_MONGOV2_URL\n    description: \"mongov2\"";

    /// Las entradas de `datasources` se trocean con su nombre, en las dos formas.
    #[test]
    fn las_fuentes_del_manifiesto_se_trocean_por_nombre() {
        let f = trocear_fuentes(&con(&[MONGO])).unwrap();
        let nombres: Vec<&str> = f.entradas.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(nombres, ["pg", "s3", "mongov2"]);
        assert!(f.antes.last().unwrap() == "datasources:");
        assert!(trocear_fuentes("kind: OntologyConfig\n").is_none());
    }

    /// Una fuente que `main` dio de alta después de nacer la rama entra en ella,
    /// con su texto tal cual y sin tocar lo demás.
    #[test]
    fn la_fuente_nueva_de_main_entra_en_la_rama() {
        let (nuevo, tocadas) = fuentes_al_dia(CONFIG, Some(CONFIG), &con(&[MONGO])).unwrap();
        assert_eq!(nuevo, con(&[MONGO]));
        assert_eq!(tocadas, vec![("mongov2".to_string(), "main")]);
        // Sin cambios no hay nada que escribir.
        assert!(fuentes_al_dia(CONFIG, Some(CONFIG), CONFIG).is_none());
    }

    /// Lo que la rama cambió es suyo; lo que borró, no vuelve; lo que `main`
    /// retiró y ella no tocó, se va.
    #[test]
    fn la_rama_manda_en_lo_que_toco() {
        // La rama editó `pg`: gana la rama aunque `main` lo haya movido.
        let rama = CONFIG.replace("PG_URL", "OTRA_URL");
        let main = CONFIG.replace("PG_URL", "MAIN_URL");
        assert!(fuentes_al_dia(&rama, Some(CONFIG), &main).is_none());
        // La rama borró `s3`: no vuelve.
        let sin_s3 = CONFIG.replace("  - { name: s3, type: s3, connectionEnv: S3_URL }\n", "");
        assert!(fuentes_al_dia(&sin_s3, Some(CONFIG), CONFIG).is_none());
        // `main` retiró `s3` y la rama no lo tocó: se oculta también en ella.
        let (nuevo, tocadas) = fuentes_al_dia(CONFIG, Some(CONFIG), &sin_s3).unwrap();
        assert_eq!(nuevo, sin_s3);
        assert_eq!(tocadas, vec![("s3".to_string(), "ninguno")]);
    }

    /// El caso de victor (30-oct): una rama del 11-oct, `main` con `mongov2` dado
    /// de alta y catalogado después. La lectura de la rama trae el paquete entero
    /// y la fuente; lo que la rama tiene suyo, se queda.
    #[test]
    fn la_rama_anterior_al_alta_ve_el_origen_y_su_esquema() {
        let d = temporal();
        let pelada = d.join("arbol.git");
        let otro = d.join("otro");
        std::fs::create_dir_all(&otro).unwrap();
        let corre = |args: &[&str], cwd: &Path| {
            let s = Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "s")
                .env("GIT_AUTHOR_EMAIL", "s@x")
                .env("GIT_COMMITTER_NAME", "s")
                .env("GIT_COMMITTER_EMAIL", "s@x")
                .output()
                .unwrap();
            assert!(
                s.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&s.stderr)
            );
        };
        let escribe = |ruta: &str, texto: &str| {
            let f = otro.join(ruta);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, texto).unwrap();
        };
        let p = pelada.to_str().unwrap();
        corre(&["init", "-q", "--bare", "-b", "main", p], &d);
        corre(&["init", "-q", "-b", "main"], &otro);
        escribe("ontology.config.yaml", CONFIG);
        escribe("packages/pg/package.yaml", "name: pg\n");
        escribe("packages/vieja/package.yaml", "name: vieja\n");
        corre(&["add", "-A"], &otro);
        corre(&["commit", "-qm", "base"], &otro);
        corre(&["push", "-q", p, "HEAD:main"], &otro);
        // La rama nace aquí, y borra `vieja` y crea `suya`.
        corre(&["checkout", "-q", "-b", "rama"], &otro);
        std::fs::remove_dir_all(otro.join("packages/vieja")).unwrap();
        escribe("packages/suya/package.yaml", "name: suya\n");
        corre(&["add", "-A"], &otro);
        corre(&["commit", "-qm", "rama"], &otro);
        corre(&["push", "-q", p, "HEAD:refs/heads/rama"], &otro);
        // Después, `main` da de alta y cataloga `mongov2`, y mueve `pg`.
        corre(&["checkout", "-q", "main"], &otro);
        escribe("ontology.config.yaml", &con(&[MONGO]));
        escribe("packages/mongov2/package.yaml", "name: mongov2\n");
        escribe(
            "packages/mongov2/ore_prueba/tables/anidada.yaml",
            "kind: Table\n",
        );
        escribe("packages/pg/package.yaml", "name: pg\nversion: 2\n");
        corre(&["add", "-A"], &otro);
        corre(&["commit", "-qm", "alta y catalogo"], &otro);
        corre(&["push", "-q", p, "HEAD:main"], &otro);

        let f = Forja {
            url: format!("file://{}", pelada.to_string_lossy().replace('\\', "/")),
            testigo: String::new(),
        };
        let a = f.para_leer(Some("rama")).unwrap();
        let r = a.ruta();
        let lee = |x: &str| std::fs::read_to_string(r.join(x)).ok();
        assert_eq!(
            lee("packages/mongov2/package.yaml").as_deref(),
            Some("name: mongov2\n")
        );
        assert_eq!(
            lee("packages/mongov2/ore_prueba/tables/anidada.yaml").as_deref(),
            Some("kind: Table\n"),
            "el paquete viene entero"
        );
        assert_eq!(lee("ontology.config.yaml"), Some(con(&[MONGO])));
        // Lo no tocado por la rama sigue a `main` al día; lo suyo, suyo; lo que borró, fuera.
        assert_eq!(
            lee("packages/pg/package.yaml").as_deref(),
            Some("name: pg\nversion: 2\n")
        );
        assert_eq!(
            lee("packages/suya/package.yaml").as_deref(),
            Some("name: suya\n")
        );
        assert!(
            lee("packages/vieja/package.yaml").is_none(),
            "la rama la borró"
        );
        // Un clon de escritura de la rama NO lo lleva: no acaba en un commit suyo.
        let clon = f.clonar_rama(Some("rama")).unwrap();
        assert!(!clon.ruta().join("packages/mongov2").exists());
        drop(clon);
        drop(a);
        let _ = std::fs::remove_dir_all(&d);
    }
}
