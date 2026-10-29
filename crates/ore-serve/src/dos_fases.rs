//! ⭐ **LAS ESCRITURAS POR CLASE, EN DOS FASES** (0079 §19.2 ·5c·5a y ·5c·5b).
//!
//! Con el catálogo, el puntero va a Postgres por la puerta y git sólo guarda
//! documentos. Así que una escritura del lago ya no necesita **el árbol de
//! escribir** (poner el espejo al día, clonar, superponer `main` en una rama…),
//! que existía sólo para que `ore` pudiera escribir ficheros.
//!
//! | | Fase 1 · siempre | Fase 2 · sólo si hay documentos |
//! |---|---|---|
//! | **Dónde** | el árbol de LEER (`leyendo_en`), el compartido del `loadTable` | un árbol de escribir (`escribiendo`, `escribiendo_en`), como hoy |
//! | **Qué** | el 423 de `main` protegida (la misma regla, antes de escribir nada); `ore … --documentos-aparte`: el puntero en una transacción del catálogo, y los documentos **en la respuesta**, no en el árbol | escribir esos ficheros, y nada más; lo que la ruta contesta (`cargar`) y `publicar` |
//! | **Git** | **nada** (·5c·5a, solo puntero) | un commit con sólo el documento, **después** del catálogo (·5c·5b, la mixta) |
//! | **Si falla** | lo que diga el catálogo (409 por el CAS, 503) y nada escrito | ver «pendiente» |
//!
//! # Pendiente (la decisión (b) de §11.3)
//!
//! Si la fase 2 no publica (`Adelantado` o 502), se vuelve a intentar con un
//! árbol de escribir nuevo —clonado de lo último: la rebase de siempre— y el
//! mismo documento, que es de máquina; hasta [`INTENTOS`] veces. Si aun así no:
//! **el dato ya está en el catálogo**, así que la respuesta es la buena (la de
//! la ruta, con el árbol de leer), con [`PENDIENTE`] en el cuerpo (qué rutas y
//! por qué; un `204` pasa a `200` para poder decirlo) y la cabecera
//! `x-ore-documento: pendiente` ([`salida`]), y una línea en el registro. **Lo
//! arregla el documentador (·5e)**, que confirma a git el `Dataset` de lo que el
//! catálogo tiene. Lo que falla en la ruta misma (un `cargar` que no contesta)
//! se dice como hoy.
//!
//! # Con `Ficheros`, lo de siempre
//!
//! Sin el catálogo ([`activas`] es `false`), [`Servidor::por_clase`] es
//! exactamente la costura de hoy (`escribiendo_en`, o `escribiendo` en `main`
//! para lo que es de la celda), con los mismos argumentos de `ore`: sin
//! `--documentos-aparte`. Una sola función decide, y cada ruta le da dos cierres:
//! `escribir(raiz, aparte)` (el verbo, y sus documentos si `aparte`) y
//! `responder(raiz, hecho)` (lo que la ruta contesta tras escribir).
//!
//! # Cuáles
//!
//! `/v1` `createTable`, `updateTable` y `transactions/commit` (costura de la
//! rama, [`Costura::Rama`]); `/datasets/…/confirmar`, `/datasets/…/publicar-retenida`
//! y el commit de una colección (`escritas.rs`, que escribe su `derivedFrom` él
//! mismo) (costura de la celda, [`Costura::Celda`]).

use std::path::{Component, Path};

use ore_core::json::Json;
use ore_entrada::http::{Bytes, Peticion, Respuesta, Salida};
use ore_entrada::identidad::Identidad;

use crate::rutas::Servidor;

/// La bandera de `ore datasets` (0079 ·5c·5).
pub(crate) const BANDERA: &str = "--documentos-aparte";

/// La cabecera de una respuesta cuyo documento no llegó a git.
pub(crate) const CABECERA: &str = "x-ore-documento";

/// La clave, en el cuerpo, de un documento pendiente: `{estado, rutas, motivo}`.
pub(crate) const PENDIENTE: &str = "ore.documento";

/// Cuántas veces se intenta publicar el documento (§19.2: «×3»).
pub(crate) const INTENTOS: usize = 3;

/// Un documento que `ore` habría escrito: su ruta (relativa a la raíz, con `/`)
/// y su texto entero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Documento {
    pub(crate) ruta: String,
    pub(crate) texto: String,
}

/// **Por qué costura escribía la ruta hasta hoy**, que decide el 423.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Costura {
    /// `escribiendo_en(rama)` siempre (`/v1`): en `main`, el 423 si está protegida.
    Rama,
    /// Lo de la celda: sin rama, `escribiendo` (sin 423); con una,
    /// `escribiendo_en` (y `x-ore-rama: main` sí da 423).
    Celda,
}

#[cfg(test)]
thread_local! {
    /// Las pruebas fuerzan las dos fases con `Ficheros` (ver `pruebas`).
    static FORZADAS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Cuántas veces se abrió un árbol de escribir en la fase 2.
    static FASES_DOS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// **¿Dos fases?** Sólo con el catálogo (`--punteros catalogo`).
pub(crate) fn activas() -> bool {
    #[cfg(test)]
    if FORZADAS.with(|f| f.get()) {
        return true;
    }
    crate::lugar::con_catalogo()
}

/// La `Idempotency-Key` de la petición como argumento de `ore`
/// (`--idempotencia=<uuid>`), si la trae bien formada: llega a
/// `Contexto.idempotencia` de la escritura del puntero (ADR 0031). Una mal
/// formada no se pasa (fuera de `/v1`, como hasta hoy, no se mira).
pub(crate) fn clave_de(p: &Peticion) -> Option<String> {
    p.cabeceras
        .get("idempotency-key")
        .map(|v| v.trim())
        .filter(|k| ore_core::idempotencia::bien_formada(k))
        .map(|k| format!("--idempotencia={k}"))
}

/// **Los `documentos` de lo que contestó `ore`**, fuera de su cuerpo: el
/// cuerpo sigue siendo el de siempre (un `Crudo` sin ellos pasa a objeto, con
/// sus números y sus `null` intactos). Sin la clave, la respuesta tal cual.
pub(crate) fn separar(r: Respuesta) -> (Respuesta, Vec<Documento>) {
    let m = match r.cuerpo {
        Json::Obj(m) if m.contains_key("documentos") => m,
        Json::Crudo(ref t) if t.contains("\"documentos\"") => {
            match ore_core::parse::parse(t).map(|n| Json::de_node_fiel(&n)) {
                Ok(Json::Obj(m)) if m.contains_key("documentos") => m,
                _ => return (r, Vec::new()),
            }
        }
        _ => return (r, Vec::new()),
    };
    let (cuerpo, docs) = sin_documentos(m);
    (
        Respuesta {
            codigo: r.codigo,
            cuerpo,
        },
        docs,
    )
}

/// Un objeto sin su clave `documentos`, y lo que decía.
fn sin_documentos(mut m: std::collections::BTreeMap<String, Json>) -> (Json, Vec<Documento>) {
    let docs = match m.remove("documentos") {
        Some(Json::Arr(l)) => l
            .iter()
            .filter_map(|d| {
                let Json::Obj(d) = d else { return None };
                let c = |k: &str| match d.get(k) {
                    Some(Json::Str(s)) => Some(s.clone()),
                    _ => None,
                };
                Some(Documento {
                    ruta: c("ruta")?,
                    texto: c("texto")?,
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    (Json::Obj(m), docs)
}

/// Una ruta de documento que se puede escribir: relativa, sin `..`, dentro del árbol.
fn ruta_del_arbol(ruta: &str) -> bool {
    let p = Path::new(ruta);
    !ruta.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// **La fase 2, lo que escribe**: esos ficheros y nada más.
pub(crate) fn escribir_documentos(raiz: &Path, docs: &[Documento]) -> Result<(), String> {
    for d in docs {
        if !ruta_del_arbol(&d.ruta) {
            return Err(format!("`{}` no es una ruta del árbol", d.ruta));
        }
        let f = raiz.join(&d.ruta);
        if let Some(padre) = f.parent() {
            std::fs::create_dir_all(padre).map_err(|e| format!("`{}`: {e}", d.ruta))?;
        }
        std::fs::write(&f, &d.texto).map_err(|e| format!("`{}`: {e}", d.ruta))?;
    }
    Ok(())
}

/// Una respuesta otra vez (`Respuesta` no es `Clone`).
fn otra(r: &Respuesta) -> Respuesta {
    Respuesta {
        codigo: r.codigo,
        cuerpo: r.cuerpo.clone(),
    }
}

/// El mensaje de una respuesta de error (`{"error": "…"}` o la forma de la spec).
fn motivo_de(r: &Respuesta) -> String {
    match &r.cuerpo {
        Json::Obj(m) => match m.get("error") {
            Some(Json::Str(s)) => s.clone(),
            Some(Json::Obj(e)) => match e.get("message") {
                Some(Json::Str(s)) => s.clone(),
                _ => r.cuerpo.jcs(),
            },
            _ => r.cuerpo.jcs(),
        },
        otro => otro.jcs(),
    }
}

/// **La respuesta buena, con el documento pendiente** (ver el módulo).
pub(crate) fn con_pendiente(r: Respuesta, docs: &[Documento], motivo: &str) -> Respuesta {
    let mut m = match r.cuerpo {
        Json::Obj(m) => m,
        Json::Crudo(t) => match ore_core::parse::parse(&t).map(|n| Json::de_node_fiel(&n)) {
            Ok(Json::Obj(m)) => m,
            _ => return r_crudo_con_pendiente(r.codigo, &t, docs, motivo),
        },
        _ => Default::default(),
    };
    m.insert(PENDIENTE.into(), pendiente(docs, motivo));
    Respuesta {
        codigo: if r.codigo == 204 { 200 } else { r.codigo },
        cuerpo: Json::Obj(m),
    }
}

/// Un cuerpo crudo que no es un objeto no debería llegar; si llega, va dentro.
fn r_crudo_con_pendiente(codigo: u16, t: &str, docs: &[Documento], motivo: &str) -> Respuesta {
    Respuesta {
        codigo,
        cuerpo: Json::obj([
            ("respuesta", Json::Crudo(t.to_string())),
            (PENDIENTE, pendiente(docs, motivo)),
        ]),
    }
}

fn pendiente(docs: &[Documento], motivo: &str) -> Json {
    Json::obj([
        ("estado", Json::s("pendiente")),
        (
            "rutas",
            Json::Arr(docs.iter().map(|d| Json::s(&d.ruta)).collect()),
        ),
        ("motivo", Json::s(motivo)),
        (
            "arregla",
            Json::s("el documentador (0079 ·5e): el dato ya está en el catálogo"),
        ),
    ])
}

/// ¿Dice este cuerpo que su documento quedó pendiente?
pub(crate) fn es_pendiente(r: &Respuesta) -> bool {
    matches!(&r.cuerpo, Json::Obj(m) if matches!(
        m.get(PENDIENTE),
        Some(Json::Obj(p)) if p.get("estado") == Some(&Json::s("pendiente"))
    ))
}

/// **La salida de una respuesta**: con su documento pendiente, con la cabecera
/// `x-ore-documento: pendiente` (una [`Bytes`]: es lo que lleva cabeceras); si
/// no, la de siempre.
pub(crate) fn salida(r: Respuesta) -> Salida {
    if !es_pendiente(&r) {
        return Salida::Una(r);
    }
    let cuerpo = r.cuerpo.jcs().into_bytes();
    Salida::Bytes(Bytes {
        codigo: r.codigo,
        cabeceras: vec![
            ("content-type".into(), "application/json".into()),
            ("cache-control".into(), "no-store".into()),
            ("x-content-type-options".into(), "nosniff".into()),
            (CABECERA.into(), "pendiente".into()),
        ],
        largo: Some(cuerpo.len() as u64),
        lector: Box::new(std::io::Cursor::new(cuerpo)),
        finales: None,
    })
}

impl Servidor {
    /// ⭐⭐ **UNA ESCRITURA DEL LAGO, POR CLASE** (ver el módulo).
    ///
    /// - `escribir(raiz, aparte)`: corre el verbo (con [`BANDERA`] si `aparte`)
    ///   y devuelve lo que contestó y sus documentos (vacíos si no `aparte`).
    /// - `responder(raiz, hecho)`: lo que la ruta contesta tras escribir (un
    ///   `loadTable`, o `hecho` tal cual). No se llama si `hecho` es un error.
    pub(crate) fn por_clase(
        &self,
        rama: Option<&str>,
        costura: Costura,
        sujeto: &Identidad,
        mensaje: &str,
        escribir: impl FnOnce(&Path, bool) -> (Respuesta, Vec<Documento>),
        responder: impl Fn(&Path, Respuesta) -> Respuesta,
    ) -> Respuesta {
        if !activas() {
            // Lo de hoy, sin tocar: la costura de siempre y `ore` sin la bandera.
            let hoy = |raiz: &Path| {
                let (r, _) = escribir(raiz, false);
                if r.codigo >= 300 {
                    r
                } else {
                    responder(raiz, r)
                }
            };
            return match (costura, rama) {
                (Costura::Celda, None) => self.escribiendo(sujeto, mensaje, hoy),
                _ => self.escribiendo_en(rama, sujeto, mensaje, hoy),
            };
        }
        // Adónde se escribe: la rama por defecto, dicha, es `main`
        // (`escribiendo_en`, 2026-10-02).
        let destino = rama.filter(|r| !self.es_la_rama_por_defecto(r));
        let proteger = match costura {
            Costura::Rama => true,
            Costura::Celda => rama.is_some(),
        };

        // ── Fase 1: en el árbol de leer ─────────────────────────────────────
        let mut mixta: Option<(Respuesta, Vec<Documento>)> = None;
        // 0079 ·5e: los documentos, en la cola del documentador antes de intentar git.
        let mut cola: Option<Vec<(String, i64)>> = None;
        let r = self.leyendo_en(destino, |raiz| {
            if destino.is_none() && proteger && crate::politica::Politica::de_raiz(raiz).protegida {
                return Respuesta::error(
                    423,
                    "`main` está protegida: nada se escribió. Se trabaja en una rama y se propone",
                );
            }
            let (hecho, docs) = escribir(raiz, true);
            if hecho.codigo >= 300 {
                return hecho;
            }
            if docs.is_empty() {
                // ·5c·5a: solo puntero. Ni árbol de escribir, ni commit, ni push.
                return responder(raiz, hecho);
            }
            cola = crate::documentador::encolar(destino, raiz, &docs);
            mixta = Some((hecho, docs));
            Respuesta::sin_contenido()
        });
        let Some((hecho, docs)) = mixta else {
            return r;
        };

        // ── Fase 2 (·5c·5b): el documento a git, después del catálogo ───────
        let mut motivo = String::new();
        for _ in 0..INTENTOS {
            #[cfg(test)]
            FASES_DOS.with(|n| n.set(n.get() + 1));
            // Qué falló, si falla: los documentos (1) o la ruta misma (2); si
            // no, fue publicar.
            let propio = std::cell::Cell::new(0u8);
            let hacer = |raiz: &Path| {
                if let Err(m) = escribir_documentos(raiz, &docs) {
                    propio.set(1);
                    return Respuesta::error(500, m);
                }
                let r = responder(raiz, otra(&hecho));
                if r.codigo >= 300 {
                    propio.set(2);
                }
                r
            };
            let e = match destino {
                None => self.escribiendo(sujeto, mensaje, hacer),
                Some(b) => self.escribiendo_en(Some(b), sujeto, mensaje, hacer),
            };
            if e.codigo < 300 {
                // En git: fuera de la cola (si no llegó uno más nuevo entretanto).
                crate::documentador::quitar(destino, cola.as_deref().unwrap_or_default());
                return e;
            }
            if propio.get() == 2 {
                return e;
            }
            motivo = motivo_de(&e);
            if propio.get() == 1 || !matches!(e.codigo, 409 | 502) {
                break;
            }
        }
        let rutas: Vec<&str> = docs.iter().map(|d| d.ruta.as_str()).collect();
        eprintln!(
            "ore-serve · documento pendiente ({}): {motivo} · el dato ya está en el catálogo; {}",
            rutas.join(", "),
            if cola.is_some() {
                "en la cola del documentador (0079 ·5e)"
            } else {
                "⛔ y NO está en la cola del documentador: nadie lo llevará a git"
            }
        );
        let r = self.leyendo_en(destino, |raiz| responder(raiz, hecho));
        if r.codigo >= 300 {
            return r;
        }
        con_pendiente(r, &docs, &motivo)
    }
}

#[cfg(test)]
pub(crate) mod pruebas {
    //! Las dos fases, con `Ficheros` detrás y **las dos fases forzadas**
    //! ([`FORZADAS`]): probarlas contra el catálogo de verdad pediría a ore-serve
    //! un cliente de Postgres para las pruebas (crear el esquema), y eso es una
    //! dependencia más en el `Cargo.lock`. Lo que aquí se prueba es la
    //! orquestación: la forja es una de verdad (`file://`, pelada), y `ore`, uno
    //! de mentira que no escribe nada en el árbol (con `Ficheros` de verdad la
    //! fase 1 escribiría el puntero en el árbol de leer: con el catálogo, va a
    //! Postgres). Que `ore` con la bandera escribe el mismo puntero y devuelve
    //! justo el documento de hoy lo prueba `ore-cli/tests/los_documentos_aparte.rs`.
    use super::*;
    use crate::rutas::Arbol;
    use std::path::PathBuf;
    use std::process::Command;

    /// Las dos fases forzadas mientras viva (en este hilo).
    pub(crate) struct Forzadas;

    impl Forzadas {
        pub(crate) fn poner() -> Forzadas {
            FORZADAS.with(|f| f.set(true));
            FASES_DOS.with(|n| n.set(0));
            Forzadas
        }
        pub(crate) fn fases_dos(&self) -> usize {
            FASES_DOS.with(|n| n.get())
        }
    }

    impl Drop for Forzadas {
        fn drop(&mut self) {
            FORZADAS.with(|f| f.set(false));
        }
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let s = Command::new("git")
            .args(args)
            .current_dir(dir)
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
        String::from_utf8_lossy(&s.stdout).into_owned()
    }

    /// Una forja pelada con un árbol (`ficheros`), y su directorio de prueba.
    pub(crate) struct Banco {
        pub(crate) dir: PathBuf,
        pub(crate) pelada: PathBuf,
        pub(crate) servidor: Servidor,
        pub(crate) log: PathBuf,
    }

    impl Drop for Banco {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Banco {
        pub(crate) fn nuevo(nombre: &str, ficheros: &[(&str, &str)]) -> Banco {
            let dir = std::env::temp_dir().join(format!(
                "ore-serve-dos-fases-{nombre}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let pelada = dir.join("arbol.git");
            let semilla = dir.join("semilla");
            std::fs::create_dir_all(&semilla).unwrap();
            git(
                &dir,
                &[
                    "init",
                    "-q",
                    "--bare",
                    "-b",
                    "main",
                    pelada.to_str().unwrap(),
                ],
            );
            git(&semilla, &["init", "-q", "-b", "main"]);
            for (r, t) in ficheros {
                let f = semilla.join(r);
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(f, t).unwrap();
            }
            git(&semilla, &["add", "-A"]);
            git(&semilla, &["commit", "-qm", "semilla"]);
            git(
                &semilla,
                &["push", "-q", pelada.to_str().unwrap(), "HEAD:main"],
            );
            let log = dir.join("ore.log");
            let ore = dir.join("ore");
            // `ore` de mentira: apunta sus argumentos, y con la bandera devuelve
            // lo que diga `documentos.json` (si no está, ninguno). No escribe
            // nada en el árbol (como `ore` con el catálogo detrás).
            std::fs::write(
                &ore,
                format!(
                    r#"#!/bin/bash
echo "$*" >> '{log}'
case " $* " in
  *" --cargar "*)
    echo '{{"metadata-location":"gs://b/m.json","metadata":{{"format-version":2}}}}' ;;
  *" --documentos-aparte "*)
    d=$(cat '{dir}/documentos.json' 2>/dev/null || echo '[]')
    echo "{{\"tabla\":\"ventas.t\",\"publicada\":true,\"documentos\":$d}}" ;;
  *) echo '{{"tabla":"ventas.t","publicada":true}}' ;;
esac
"#,
                    log = log.display(),
                    dir = dir.display()
                ),
            )
            .unwrap();
            Command::new("chmod").arg("+x").arg(&ore).status().unwrap();
            let servidor = Servidor {
                binario: ore,
                arbol: Arbol::Forja(crate::git::Forja {
                    url: format!("file://{}", pelada.to_string_lossy()),
                    testigo: String::new(),
                }),
                identidad: None,
                cola: None,
                cofre: None,
                organizacion: None,
                cuenta_driver: None,
                cuenta_medios: None,
                modelos: None,
                perfiles: None,
                forja_api: None,
                puestos: Default::default(),
                assets_cache: Default::default(),
                cambios_cache: Default::default(),
                acceso: None,
                buzon: None,
                pertenencias: Default::default(),
                agente: None,
                escritas: Default::default(),
                refresco: None,
                builds_elasticos: None,
                query_en_g: false,
                session_pool: false,
            };
            Banco {
                dir,
                pelada,
                servidor,
                log,
            }
        }

        pub(crate) fn documentos(&self, docs: &[(&str, &str)]) {
            let j = Json::Arr(
                docs.iter()
                    .map(|(r, t)| Json::obj([("ruta", Json::s(*r)), ("texto", Json::s(*t))]))
                    .collect(),
            );
            std::fs::write(self.dir.join("documentos.json"), j.jcs()).unwrap();
        }

        /// La punta de `main` en la forja.
        pub(crate) fn main(&self) -> String {
            git(&self.pelada, &["rev-parse", "main"]).trim().to_string()
        }

        /// Las veces que se corrió `ore`, y sus argumentos.
        pub(crate) fn llamadas(&self) -> Vec<String> {
            std::fs::read_to_string(&self.log)
                .unwrap_or_default()
                .lines()
                .map(String::from)
                .collect()
        }

        /// ⛔ El árbol de leer compartido, limpio: ni un fichero de más.
        pub(crate) fn leer_limpio(&self) {
            let Arbol::Forja(f) = &self.servidor.arbol else {
                unreachable!()
            };
            let a = f.para_leer(None).unwrap();
            assert_eq!(
                git(a.ruta(), &["status", "--porcelain", "--ignored"]).trim(),
                "",
                "la fase 1 ensució el árbol de leer"
            );
        }

        /// Que la forja rechace todo empujón (un `pre-receive` que dice que no).
        pub(crate) fn forja_que_rechaza(&self) {
            let h = self.pelada.join("hooks/pre-receive");
            std::fs::write(&h, "#!/bin/sh\necho 'no, hoy no' >&2\nexit 1\n").unwrap();
            Command::new("chmod").arg("+x").arg(&h).status().unwrap();
        }
    }

    pub(crate) fn ana() -> Identidad {
        Identidad {
            persona: "user:ana".into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: None,
            usuario: None,
        }
    }

    const ARBOL: &[(&str, &str)] = &[
        ("ontology.config.yaml", "apiVersion: oos.dev/v1alpha1\n"),
        ("packages/ventas/package.yaml", "kind: Package\n"),
    ];

    const DOC: &str = "packages/ventas/datasets/t.yaml";

    /// ⭐ **·5c·5a · Solo puntero: ni un `git` de escritura.** `ore` corre una vez,
    /// con la bandera, en el árbol de leer; `main` no se mueve, no se abre ningún
    /// árbol de escribir, y el árbol de leer sigue limpio.
    #[test]
    fn solo_puntero_no_escribe_git_y_deja_limpio_el_arbol_de_leer() {
        let b = Banco::nuevo("solo", ARBOL);
        let f = Forzadas::poner();
        let antes = b.main();
        let r = b
            .servidor
            .publicar_retenida(None, &ana(), "ventas", "default", "t", None);
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        assert!(!r.cuerpo.jcs().contains("documentos"), "{}", r.cuerpo.jcs());
        assert_eq!(b.main(), antes, "solo puntero: ni commit ni push");
        assert_eq!(f.fases_dos(), 0, "ni un árbol de escribir");
        let l = b.llamadas();
        assert_eq!(l.len(), 1, "{l:?}");
        assert!(l[0].contains(BANDERA), "{l:?}");
        b.leer_limpio();
    }

    /// ⭐ **·5c·5b · La mixta: un commit con sólo el documento**, después del
    /// catálogo; `ore` corre una vez (no se repite la escritura en la fase 2).
    #[test]
    fn la_mixta_confirma_solo_el_documento() {
        let b = Banco::nuevo("mixta", ARBOL);
        let f = Forzadas::poner();
        let texto = "apiVersion: oos.dev/v1alpha12\nkind: Dataset\n# Un dataset escrito (0033)\n";
        b.documentos(&[(DOC, texto)]);
        let antes = b.main();
        let r = b
            .servidor
            .publicar_retenida(None, &ana(), "ventas", "default", "t", None);
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        assert!(!es_pendiente(&r));
        let despues = b.main();
        assert_ne!(despues, antes);
        assert_eq!(
            git(
                &b.pelada,
                &["rev-list", "--count", &format!("{antes}..main")]
            )
            .trim(),
            "1"
        );
        assert_eq!(
            git(
                &b.pelada,
                &["diff", "--name-only", &format!("{antes}..main")]
            )
            .trim(),
            DOC,
            "el commit lleva sólo el documento"
        );
        assert_eq!(
            git(&b.pelada, &["show", &format!("main:{DOC}")]),
            texto,
            "y exactamente su texto"
        );
        assert_eq!(f.fases_dos(), 1);
        assert_eq!(b.llamadas().len(), 1, "`ore` no se repite");
        b.leer_limpio();
    }

    /// ⭐ **Pendiente:** con la forja que rechaza, tres intentos y un 200 que lo
    /// dice (cuerpo y cabecera); `main` no se mueve y el árbol de leer, limpio.
    #[test]
    fn si_git_no_llega_la_respuesta_es_buena_y_el_documento_pendiente() {
        let b = Banco::nuevo("pendiente", ARBOL);
        let f = Forzadas::poner();
        b.documentos(&[(DOC, "kind: Dataset\n")]);
        b.forja_que_rechaza();
        let antes = b.main();
        let r = b
            .servidor
            .publicar_retenida(None, &ana(), "ventas", "default", "t", None);
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        assert!(es_pendiente(&r), "{}", r.cuerpo.jcs());
        let c = r.cuerpo.jcs();
        assert!(c.contains(DOC) && c.contains("\"publicada\":true"), "{c}");
        assert_eq!(f.fases_dos(), INTENTOS, "se intenta {INTENTOS} veces");
        assert_eq!(b.main(), antes);
        assert_eq!(b.llamadas().len(), 1, "el puntero, una vez");
        b.leer_limpio();
        // Y sale con su cabecera.
        match salida(r) {
            Salida::Bytes(x) => {
                assert_eq!(x.codigo, 200);
                assert!(
                    x.cabeceras
                        .iter()
                        .any(|(k, v)| k == CABECERA && v == "pendiente"),
                    "{:?}",
                    x.cabeceras
                );
            }
            _ => panic!("sin la cabecera"),
        }
        // Un 204 pasa a 200, para poder decirlo.
        let r = con_pendiente(Respuesta::sin_contenido(), &[], "x");
        assert_eq!(r.codigo, 200);
        assert!(matches!(salida(Respuesta::sin_contenido()), Salida::Una(_)));
    }

    /// El 423 de `main` protegida, antes de escribir nada: `ore` ni corre.
    #[test]
    fn main_protegida_es_423_antes_de_escribir_nada() {
        let mut arbol = ARBOL.to_vec();
        arbol.push((crate::politica::RUTA, "main:\n  protegida: true\n"));
        let b = Banco::nuevo("protegida", &arbol);
        let f = Forzadas::poner();
        // `x-ore-rama: main` en una ruta de la celda: `escribiendo_en` → 423.
        let r = b
            .servidor
            .publicar_retenida(Some("main"), &ana(), "ventas", "default", "t", None);
        assert_eq!(r.codigo, 423, "{}", r.cuerpo.jcs());
        assert!(b.llamadas().is_empty(), "nada se escribió");
        // Sin rama, lo de la celda escribe en `main` como hoy (P6).
        let r = b
            .servidor
            .publicar_retenida(None, &ana(), "ventas", "default", "t", None);
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        assert_eq!(f.fases_dos(), 0);
    }

    /// Sin forzar (`Ficheros`, lo de hoy): una fase, `ore` sin la bandera.
    #[test]
    fn con_ficheros_lo_de_hoy() {
        let b = Banco::nuevo("hoy", ARBOL);
        let r = b
            .servidor
            .publicar_retenida(None, &ana(), "ventas", "default", "t", None);
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        let l = b.llamadas();
        assert_eq!(l.len(), 1);
        assert!(!l[0].contains(BANDERA), "{l:?}");
    }

    fn v1(b: &Banco, cabeceras: &[(&str, &str)]) -> Respuesta {
        let p = Peticion {
            metodo: "POST".into(),
            ruta: String::new(),
            cabeceras: cabeceras
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            cuerpo: r#"{"requirements":[],"updates":[]}"#.into(),
            consulta: Default::default(),
            consulta_cruda: String::new(),
        };
        b.servidor
            .catalogo(&p, &ana(), None, &["namespaces", "ventas", "tables", "t"])
    }

    /// ⭐ **`/v1` `updateTable`, por clase:** solo puntero contesta el `loadTable`
    /// sin tocar git; con un documento, lo confirma y contesta lo mismo; la
    /// `Idempotency-Key` llega a `ore`.
    #[test]
    fn el_commit_de_v1_por_clase() {
        let b = Banco::nuevo("v1", ARBOL);
        let f = Forzadas::poner();
        let antes = b.main();
        let k = "0192f0c4-7b2a-7c3e-9d41-5a6b7c8d9e0f";
        let r = v1(&b, &[("idempotency-key", k)]);
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        assert!(
            r.cuerpo.jcs().contains("gs://b/m.json"),
            "{}",
            r.cuerpo.jcs()
        );
        assert_eq!(b.main(), antes);
        assert_eq!(f.fases_dos(), 0);
        let l = b.llamadas();
        assert!(
            l[0].contains("--commit") && l[0].contains(BANDERA) && l[0].contains(k),
            "{l:?}"
        );
        assert!(l[1].contains("--cargar"), "{l:?}");
        b.leer_limpio();

        b.documentos(&[(DOC, "kind: Dataset\n")]);
        let r = v1(&b, &[]);
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        assert!(
            r.cuerpo.jcs().contains("gs://b/m.json"),
            "{}",
            r.cuerpo.jcs()
        );
        assert_eq!(
            git(
                &b.pelada,
                &["diff", "--name-only", &format!("{antes}..main")]
            )
            .trim(),
            DOC
        );
        assert_eq!(f.fases_dos(), 1);
        b.leer_limpio();
    }

    /// `/v1` en `main` protegida: 423 sin rama (la costura de la rama), y `ore`
    /// ni corre.
    #[test]
    fn el_commit_de_v1_en_main_protegida_es_423() {
        let mut arbol = ARBOL.to_vec();
        arbol.push((crate::politica::RUTA, "main:\n  protegida: true\n"));
        let b = Banco::nuevo("v1-423", &arbol);
        let _f = Forzadas::poner();
        let r = v1(&b, &[]);
        assert_eq!(r.codigo, 423, "{}", r.cuerpo.jcs());
        assert!(b.llamadas().is_empty());
    }

    /// Los `documentos`, fuera del cuerpo: el resto, intacto (números y `null`).
    #[test]
    fn los_documentos_salen_del_cuerpo_y_el_resto_queda() {
        let (r, d) = separar(Respuesta::ok(Json::Crudo(
            r#"{"tablas":[{"filas":3,"x":null}],"documentos":[{"ruta":"a/b.yaml","texto":"t\n"}]}"#
                .into(),
        )));
        assert_eq!(
            d,
            vec![Documento {
                ruta: "a/b.yaml".into(),
                texto: "t\n".into()
            }]
        );
        assert_eq!(r.cuerpo.jcs(), r#"{"tablas":[{"filas":3,"x":null}]}"#);
        let (r, d) = separar(Respuesta::ok(Json::Crudo(r#"{"a":1}"#.into())));
        assert!(d.is_empty());
        assert!(matches!(r.cuerpo, Json::Crudo(_)), "sin la clave, tal cual");
        assert!(!ruta_del_arbol("../x.yaml"));
        assert!(!ruta_del_arbol("/etc/x"));
        assert!(ruta_del_arbol("packages/v/datasets/t.yaml"));
    }
}
