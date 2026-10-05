//! **Catalogar y comprobar sin Job** (ADR 0053 F8·2, «una vía»).
//!
//! Hasta F8 mirar un origen era un Job: `malla/44-el-catalogo.yaml` para el
//! catálogo y `malla/54-la-comprobacion.yaml` para «comprobar acceso», cada uno
//! con su conector lanzado por su cuenta, fuera de la cola del origen. Ahora lo
//! hace `ore-serve` con el binario `ore` y `ORE_PASARELA`: el verbo lo ejecuta
//! la pasarela, en la cola del origen, con su identidad. Los pasos son los
//! mismos que los del Job: el paquete si no está, `ore source catalog --out`
//! (que induce la fuente entera), y al árbol; si falla, el diagnóstico en
//! `.fallos/<fuente>.catalogo.txt`, que es lo que `/fuentes/{n}/catalogo` lee.
//!
//! ⛔ La credencial la pone quien llama (decisión 3): ésta la saca del custodio
//!   como el agente de la celda (`credencial_de_la_fuente`) y va al proceso
//!   `ore` por su variable (`connectionEnv`), nunca por `argv` ni al disco.
//!
//! Sin custodio o sin agente (una máquina, una prueba) no hay de dónde sacar
//! la credencial: entonces, el Job de siempre.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;

use crate::git;
use crate::rutas::{Arbol, Servidor};

/// Las fuentes que se están catalogando ahora: `/fuentes/{n}/catalogo` dice
/// `encolada` mientras tanto.
static CATALOGANDO: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Las comprobaciones hechas, 15 min, para `GET /fuentes/comprobaciones/{id}`.
static COMPROBADAS: Mutex<VecDeque<(String, Instant, String)>> = Mutex::new(VecDeque::new());
const VIVE: Duration = Duration::from_secs(15 * 60);

pub(crate) fn catalogando(fuente: &str) -> bool {
    CATALOGANDO.lock().expect("catalogando").contains(fuente)
}

/// Sin la credencial: ni la `url` entera ni su clave (`esquema://u:clave@…`).
fn tapar(texto: &str, url: &str) -> String {
    let mut t = texto.replace(url, "***");
    if let Some(clave) = url
        .split_once("://")
        .and_then(|(_, r)| r.split_once('@'))
        .and_then(|(u, _)| u.split_once(':'))
        .map(|(_, c)| c)
        .filter(|c| c.len() >= 4)
    {
        t = t.replace(clave, "***");
    }
    t
}

/// Lo que dice una `Respuesta` de error, en una línea.
fn motivo(r: &Respuesta) -> String {
    match &r.cuerpo {
        Json::Obj(m) => match m.get("error") {
            Some(Json::Str(s)) => s.clone(),
            _ => r.cuerpo.jcs(),
        },
        o => o.jcs(),
    }
}

/// `(tipo, connectionEnv)` de `fuente` en `ontology.config.yaml`.
fn declaracion(raiz: &Path, fuente: &str) -> Result<(String, String), String> {
    let config = std::fs::read_to_string(raiz.join("ontology.config.yaml"))
        .map_err(|e| format!("no se pudo leer `ontology.config.yaml`: {e}"))?;
    let config = ore_core::parse::parse(&config)
        .map_err(|_| "`ontology.config.yaml` no analiza".to_string())?;
    let d = config
        .get("datasources")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .find(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(fuente))
        .ok_or_else(|| format!("`{fuente}` no está declarada"))?;
    let campo = |k: &str| d.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    match (campo("type"), campo("connectionEnv")) {
        (Some(t), Some(e)) => Ok((t, e)),
        _ => Err(format!("`{fuente}` no dice su `type` o su `connectionEnv`")),
    }
}

impl Servidor {
    /// ¿Se puede mirar un origen sin Job? Hace falta de dónde sacar la credencial.
    pub(crate) fn por_la_pasarela(&self) -> bool {
        self.cofre.is_some() && self.agente.is_some() && self.organizacion.is_some()
    }

    /// **Catalogar `fuente` por la pasarela**, en un hilo: lo que contesta es si
    /// empezó y por qué no. Cómo acaba lo dice `/fuentes/{n}/catalogo`.
    pub(crate) fn catalogar_por_la_pasarela(
        &self,
        fuente: &str,
        sujeto: &Identidad,
        dueno: Option<&str>,
    ) -> String {
        // ① El manifiesto: la familia y la variable de la credencial; y si ya
        //    tiene paquete, no se toca (como el Job: un catálogo que reescribe un
        //    paquete ya inducido pierde decisiones ya contestadas).
        let mut leido = Err("el árbol no se pudo leer".to_string());
        let _ = self.leyendo(|raiz| {
            leido =
                declaracion(raiz, fuente).map(|d| (d, raiz.join("packages").join(fuente).is_dir()));
            Respuesta::ok(Json::obj([]))
        });
        let ((_tipo, env), ya) = match leido {
            Ok(x) => x,
            Err(m) => return format!("NO catalogado: {m}"),
        };
        if ya {
            return format!("`{fuente}` ya tiene paquete: no se toca");
        }
        // ② La credencial, del custodio.
        let url = match self.credencial_de_la_fuente(fuente, &env) {
            Ok((u, _)) => u,
            Err(r) => return format!("NO catalogado: {}", motivo(&r)),
        };
        // ③ Una vez a la vez por fuente.
        if !CATALOGANDO
            .lock()
            .expect("catalogando")
            .insert(fuente.to_string())
        {
            return format!("`{fuente}` ya se está catalogando");
        }
        let donde = match &self.arbol {
            Arbol::Forja(f) => Donde::Forja(git::Forja {
                url: f.url.clone(),
                testigo: f.testigo.clone(),
            }),
            Arbol::Directorio(d) => Donde::Directorio(d.clone()),
        };
        let t = Trabajo {
            binario: self.binario.clone(),
            fuente: fuente.to_string(),
            env,
            url,
            dueno: dueno.map(String::from).unwrap_or_else(|| {
                format!("team:{}", self.organizacion.clone().unwrap_or_default())
            }),
            sujeto: sujeto.clone(),
        };
        std::thread::spawn(move || {
            let r = (0..2)
                .map(|_| t.hacer(&donde))
                .find(|r| !matches!(r, Err(Fin::Adelantado)))
                .unwrap_or(Err(Fin::Adelantado));
            match r {
                Ok(c) => eprintln!("catálogo de `{}` · por la pasarela · {c}", t.fuente),
                Err(Fin::Adelantado) => eprintln!(
                    "catálogo de `{}` · por la pasarela · ✗ el árbol se movió dos veces",
                    t.fuente
                ),
                Err(Fin::Fallo(f)) => {
                    eprintln!("catálogo de `{}` · por la pasarela · ✗ {f}", t.fuente)
                }
            }
            CATALOGANDO.lock().expect("catalogando").remove(&t.fuente);
        });
        format!("catalogando `{fuente}` por la pasarela (sin Job)")
    }

    /// **`POST /fuentes/comprobaciones`** por la pasarela: la respuesta ya
    /// lleva el resultado (`resultado`), y `GET /fuentes/comprobaciones/{job}`
    /// lo repite 15 min (la consola lo pregunta así).
    pub(crate) fn comprobar_por_la_pasarela(&self, tipo: &str, url: &str) -> Respuesta {
        let origen = format!("comprobar-{tipo}");
        let cuerpo = Json::obj([
            ("origen", Json::s(origen.as_str())),
            ("tipo", Json::s(tipo)),
            ("url", Json::s(url)),
        ]);
        let r = ore_entrada::http::pedir_con(
            "POST",
            &crate::federado::pasarela(),
            "/v1/check",
            &[],
            Some(&cuerpo),
            ore_entrada::http::Plazos {
                conectar: Duration::from_secs(5),
                responder: Duration::from_secs(660),
            },
        );
        let (c, b) = match r {
            Ok(x) => x,
            Err(e) => {
                return Respuesta::error(503, tapar(&format!("la pasarela no contesta: {e}"), url));
            }
        };
        let Ok(n) = ore_core::parse::parse(b.trim()) else {
            return Respuesta::error(502, "la pasarela contestó algo que no es JSON");
        };
        if c != 200 {
            let m = n
                .get("mensaje")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("")
                .to_string();
            return Respuesta::error(502, tapar(&format!("comprobar: {m}"), url));
        }
        let job = format!("comprobar-{}", crate::funciones::corrida_ahora());
        let resultado = b.trim().to_string();
        {
            let mut h = COMPROBADAS.lock().expect("comprobadas");
            h.retain(|(_, t, _)| t.elapsed() < VIVE);
            h.push_back((job.clone(), Instant::now(), resultado.clone()));
        }
        Respuesta::ok(Json::obj([
            ("job", Json::s(job.as_str())),
            ("estado", Json::s("ok")),
            ("resultado", Json::Crudo(resultado)),
            (
                "dice",
                Json::s("comprobado por la pasarela, con la identidad que leerá de verdad"),
            ),
        ]))
    }
}

/// `GET /fuentes/comprobaciones/{job}`: el resultado de una comprobación de
/// los últimos 15 min, o 404 (la de un Job de antes: su log, en `/celdas`).
pub(crate) fn comprobacion(job: &str) -> Respuesta {
    let h = COMPROBADAS.lock().expect("comprobadas");
    match h.iter().find(|(j, t, _)| j == job && t.elapsed() < VIVE) {
        Some((_, _, r)) => Respuesta::ok(Json::obj([
            ("job", Json::s(job)),
            ("estado", Json::s("ok")),
            ("resultado", Json::Crudo(r.clone())),
        ])),
        None => Respuesta::error(404, "no hay una comprobación reciente con ese nombre"),
    }
}

enum Donde {
    Forja(git::Forja),
    Directorio(PathBuf),
}

#[derive(Debug)]
enum Fin {
    /// Alguien escribió en el árbol entre el clon y el empujón: otra vez.
    Adelantado,
    Fallo(String),
}

struct Trabajo {
    binario: PathBuf,
    fuente: String,
    env: String,
    url: String,
    dueno: String,
    sujeto: Identidad,
}

impl Trabajo {
    fn ore(&self, dir: &Path, args: &[&str]) -> (bool, String) {
        match Command::new(&self.binario)
            .args(args)
            .arg("--path")
            .arg(dir)
            .current_dir(dir)
            .env(&self.env, &self.url)
            .env("ORE_PASARELA", crate::federado::pasarela())
            .output()
        {
            Ok(o) => (
                o.status.success(),
                tapar(
                    &format!(
                        "{}{}",
                        String::from_utf8_lossy(&o.stdout),
                        String::from_utf8_lossy(&o.stderr)
                    ),
                    &self.url,
                ),
            ),
            Err(e) => (false, format!("no arranca `ore`: {e}")),
        }
    }

    /// Los pasos del Job de `44`, en un clon del árbol.
    fn hacer(&self, donde: &Donde) -> Result<String, Fin> {
        let prestado;
        let dir: PathBuf = match donde {
            Donde::Forja(f) => {
                prestado = f.clonar().map_err(|e| Fin::Fallo(e.to_string()))?;
                prestado.ruta().to_path_buf()
            }
            Donde::Directorio(d) => d.clone(),
        };
        let f = self.fuente.as_str();
        let paquete = dir.join("packages").join(f);
        if paquete.is_dir() {
            return Ok(format!("`{f}` ya tiene paquete: no se toca"));
        }
        let (ok, mut log) = self.ore(&dir, &["package", "new", f, "--owner", &self.dueno]);
        let salida = format!("packages/{f}/discover.catalog.json");
        let fallos = dir.join(".fallos").join(format!("{f}.catalogo.txt"));
        let (ok, l) = if ok {
            self.ore(&dir, &["source", "catalog", f, "--out", &salida])
        } else {
            (false, String::new())
        };
        log.push_str(&l);
        let mensaje = if ok {
            let _ = std::fs::remove_file(&fallos);
            format!("Catalogo de `{f}`")
        } else {
            log.push_str("### el catalogo fallo; diagnostico:\n");
            log.push_str(&self.ore(&dir, &["source", "check", f]).1);
            // Como el Job: nada de lo que se escribió a medias, sólo el diagnóstico.
            let _ = std::fs::remove_dir_all(&paquete);
            let cola: Vec<&str> = log.lines().collect();
            let cola = cola[cola.len().saturating_sub(60)..].join("\n");
            let ahora = crate::funciones::corrida_ahora();
            let _ = std::fs::create_dir_all(dir.join(".fallos"));
            std::fs::write(
                &fallos,
                format!("job: pasarela-{ahora}\nfin: {ahora}\n---\n{cola}\n"),
            )
            .map_err(|e| Fin::Fallo(e.to_string()))?;
            format!("Catalogo de `{f}`: fallo")
        };
        if let Donde::Forja(forja) = donde {
            if !forja.hay_cambios(&dir) {
                return Ok("nada que empujar".into());
            }
            match forja.publicar(&dir, &self.sujeto, &mensaje) {
                Ok(_) => {}
                Err(git::Fallo::Adelantado(_)) => return Err(Fin::Adelantado),
                Err(e) => return Err(Fin::Fallo(e.to_string())),
            }
        }
        if ok {
            Ok(mensaje)
        } else {
            Err(Fin::Fallo(mensaje))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_credencial_no_sale_en_lo_que_se_dice() {
        let u = "postgres://ana:secreto@h:5432/db";
        let t = tapar("no conecta a postgres://ana:secreto@h:5432/db (secreto)", u);
        assert!(!t.contains("secreto"), "{t}");
    }

    #[test]
    fn la_declaracion_de_una_fuente() {
        let d = std::env::temp_dir().join(format!("f82-decl-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: t, version: 0.1.0 }
datasources:
  - { name: pg, type: postgres, connectionEnv: PG_URL }
",
        )
        .unwrap();
        assert_eq!(
            declaracion(&d, "pg").unwrap(),
            ("postgres".to_string(), "PG_URL".to_string())
        );
        assert!(declaracion(&d, "otra").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn una_comprobacion_se_recuerda() {
        COMPROBADAS.lock().unwrap().push_back((
            "comprobar-x".into(),
            Instant::now(),
            r#"{"ok":true}"#.into(),
        ));
        assert_eq!(comprobacion("comprobar-x").codigo, 200);
        assert_eq!(comprobacion("comprobar-y").codigo, 404);
    }
}
