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

impl Forja {
    /// Los ajustes que van por el ENTORNO y no por la línea de órdenes.
    fn entorno(&self) -> Vec<(String, String)> {
        vec![
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_CONFIG_COUNT".into(), "2".into()),
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
        ]
    }

    fn git(&self, dir: Option<&Path>, args: &[&str]) -> Result<String, Fallo> {
        let mut c = Command::new("git");
        if let Some(d) = dir {
            c.current_dir(d);
        }
        for (k, v) in self.entorno() {
            c.env(k, v);
        }
        let s = c
            .args(args)
            .output()
            .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
        let err = String::from_utf8_lossy(&s.stderr).into_owned();
        if s.status.success() {
            return Ok(String::from_utf8_lossy(&s.stdout).into_owned());
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
            .args(["merge", "--no-ff", "--no-edit", "-m", mensaje, "FETCH_HEAD"])
            .output()
            .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
        let mut salida = String::from_utf8_lossy(&s.stdout).into_owned();
        salida.push('\n');
        salida.push_str(&String::from_utf8_lossy(&s.stderr));
        if s.status.success() {
            return Ok(!salida.contains("Already up to date"));
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
            .args(["commit", "--quiet", "-m", mensaje])
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
    pub fn derivar(
        &self,
        base: &str,
        rama: &str,
        alcance: &Alcance,
        sujeto: &Identidad,
        mensaje: &str,
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
        if ficheros.is_empty() {
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
        if let Err(e) = self.git(Some(dir), &["apply", "--3way", "--index", &fichero_s]) {
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
        self.confirmar(
            dir,
            sujeto,
            &format!(
                "{mensaje}\n\nrama: {rama}\nalcance: {}\nhuella: {huella}",
                alcance.describe()
            ),
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
    pub fn huellas(
        &self,
        base: &str,
        rama: &str,
        derivada: &str,
        alcance: &Alcance,
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
            Ok(_) => "-".into(),
            // Lo que ya no se puede proponer tampoco está «al día».
            Err(_) => "✗".into(),
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
const ARBOLES: usize = 8;

struct Espejo {
    /// El repositorio `--bare`.
    dir: PathBuf,
    /// El `fetch`, los `worktree` y su lista, de uno en uno.
    estado: std::sync::Mutex<Arboles>,
}

#[derive(Default)]
struct Arboles {
    /// commit → su árbol, del más viejo al más nuevo.
    hechos: Vec<(String, std::sync::Arc<PathBuf>)>,
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

impl Forja {
    /// El espejo de esta forja, creado la primera vez. Por proceso: dos
    /// servidores en la misma máquina (las pruebas) no comparten directorio.
    fn espejo(&self) -> Result<std::sync::Arc<Espejo>, Fallo> {
        let mut todos = espejos().lock().unwrap_or_else(|e| e.into_inner());
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
        });
        todos.insert(self.url.clone(), e.clone());
        Ok(e)
    }

    /// El espejo, al día: todas las ramas de la forja, y las borradas fuera.
    fn al_dia(&self, e: &Espejo) -> Result<(), Fallo> {
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
        )
        .map(|_| ())
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

    /// **El árbol de una rama, para leer** (`None`: la de por defecto). Lo
    /// último de la forja, sin clonarlo: un `fetch` y el `worktree` de su
    /// commit, que se comparte. Si el espejo falla, un clon como antes.
    pub fn para_leer(&self, rama: Option<&str>) -> Result<ParaLeer, Fallo> {
        match self.leer_del_espejo(rama) {
            Ok(a) => Ok(ParaLeer::Compartido(a)),
            Err(Fallo::SinRama(r)) => Err(Fallo::SinRama(r)),
            Err(_) => self.clonar_de_la_forja(rama).map(ParaLeer::Propio),
        }
    }

    fn leer_del_espejo(&self, rama: Option<&str>) -> Result<std::sync::Arc<PathBuf>, Fallo> {
        let e = self.espejo()?;
        let mut a = e.estado.lock().unwrap_or_else(|x| x.into_inner());
        self.al_dia(&e)?;
        let c = self.commit_de(&e, rama)?;
        if let Some(i) = a.hechos.iter().position(|(h, _)| *h == c) {
            let (_, dir) = &a.hechos[i];
            // Limpio, o se rehace: un árbol compartido que alguien tocó ya no
            // es el commit que dice ser.
            let limpio = dir.is_dir()
                && self
                    .git(Some(dir), &["status", "--porcelain", "--ignored"])
                    .is_ok_and(|s| s.trim().is_empty());
            if limpio {
                let dir = dir.clone();
                let par = a.hechos.remove(i);
                a.hechos.push(par);
                return Ok(dir);
            }
            let (_, viejo) = a.hechos.remove(i);
            self.quitar_arbol(&e, &viejo);
        }
        a.creados += 1;
        let dir = e.dir.with_extension(format!("arbol-{}", a.creados));
        let _ = std::fs::remove_dir_all(&dir);
        let (d, dir_s) = (
            e.dir.to_string_lossy().into_owned(),
            dir.to_string_lossy().into_owned(),
        );
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
                &c,
            ],
        )?;
        let dir = std::sync::Arc::new(dir);
        a.hechos.push((c, dir.clone()));
        // Los viejos que nadie lee, fuera.
        while a.hechos.len() > ARBOLES {
            let Some(i) = a
                .hechos
                .iter()
                .position(|(_, p)| std::sync::Arc::strong_count(p) == 1)
            else {
                break;
            };
            let (_, viejo) = a.hechos.remove(i);
            self.quitar_arbol(&e, &viejo);
        }
        Ok(dir)
    }

    fn quitar_arbol(&self, e: &Espejo, dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
        let d = e.dir.to_string_lossy().into_owned();
        let _ = self.git(None, &["--git-dir", &d, "worktree", "prune"]);
    }

    /// Un clon para escribir, del espejo al día (sin red: enlaces duros) y con
    /// `origin` en la forja. `None` si el espejo no se pudo usar.
    fn clonar_del_espejo(&self, rama: Option<&str>) -> Result<Prestado, Fallo> {
        let e = self.espejo()?;
        {
            let _a = e.estado.lock().unwrap_or_else(|x| x.into_inner());
            self.al_dia(&e)?;
            self.commit_de(&e, rama)?;
        }
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

#[cfg(test)]
mod pruebas {
    use super::*;

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
}
