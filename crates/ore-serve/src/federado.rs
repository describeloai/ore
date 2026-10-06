//! **El coordinador del Federation Engine** (ADR 0053 F4, `docs/federation.md`
//! §4): `POST /federation/read` y `PUT /fuentes/{n}` (el interruptor).
//!
//! `ore-serve` **decide** y la pasarela **lee**. Los pasos, en este orden; el
//! primero que falla corta y no se conecta nada:
//!
//! | # | paso | quién |
//! |---|---|---|
//! | 0–2, 4 | el interruptor de la fuente y el conducto (de `main`), la tabla, sus columnas, el empuje y el coste (de la rama) | `ore federate` |
//! | 3 | el acceso: ser de la organización (ya, en `quien`) y el enganche de A8 | aquí |
//! | 5 | el presupuesto: 100 000 filas o 64 MB, 30 s; `expensive`, 10⁶ filas | aquí |
//! | 6 | la credencial, del custodio, como el agente de la celda | `credencial_del_cofre` |
//! | 7 | leer: la pasarela de la celda, y el flujo pasa sin juntarse | aquí |
//! | 8 | anotar `federation:read`, también si se negó | el buzón de `ore-iam` |
//!
//! ⛔ La credencial no sale de aquí más que hacia la pasarela, y el evento
//!   lleva los predicados **sin sus valores**: un valor de filtro puede ser un
//!   dato personal.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ore_core::json::Json;
use ore_entrada::http::{Bytes, Finales, Peticion, Respuesta, Salida};
use ore_entrada::identidad::Identidad;

use crate::rutas::Servidor;

/// Donde está la pasarela de la celda: el Service de la plantilla
/// (`malla/58-la-pasarela.yaml`), o `ORE_PASARELA`.
pub(crate) fn pasarela() -> String {
    std::env::var("ORE_PASARELA").unwrap_or_else(|_| "ore-federation:8099".into())
}

const FINALES: [&str; 5] = [
    "ore-estado",
    "ore-motivo",
    "ore-filas",
    "ore-bytes",
    "ore-ms",
];

/// Lo que el evento lleva de una lectura, sin valores.
#[derive(Clone)]
struct Anotacion {
    persona: String,
    agente: Option<String>,
    rama: Option<String>,
    tabla: String,
    origen: String,
    columnas: Vec<String>,
    predicados: Vec<(String, String)>,
    limit: Option<u64>,
    token: Option<String>,
    /// La de la apertura del puesto, si la lectura sale de uno (0053 F4·3).
    decision: Option<String>,
    empezo: Instant,
}

impl Servidor {
    /// **`POST /federation/read`**.
    pub(crate) fn leer_federado(&self, p: &Peticion, sujeto: &Identidad) -> Salida {
        let rama = p
            .cabeceras
            .get(crate::propuestas::CABECERA_RAMA)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let desde_puesto = self.puesto_que_llama(p, sujeto).is_some();
        let (quien, rama) = match self.sujeto_del_puesto(p, sujeto, rama) {
            Ok(x) => x,
            Err(r) => return Salida::Una(r),
        };
        let n = match ore_core::parse::parse(&p.cuerpo) {
            Ok(n) => n,
            Err(_) => return Salida::Una(error(400, "operador", "el cuerpo no es JSON")),
        };
        let Some(tabla) = n
            .get("tabla")
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
        else {
            return Salida::Una(error(
                400,
                "operador",
                "falta `tabla` (`base.schema.nombre`)",
            ));
        };
        let columnas: Vec<String> = n
            .get("columnas")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|c| c.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let filtros = n.get("filtros").map(|(_, v)| Json::de_node(v));
        let predicados: Vec<(String, String)> = n
            .get("filtros")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .map(|f| {
                        let k = |c: &str| {
                            f.get(c)
                                .and_then(|(_, x)| x.as_str())
                                .unwrap_or("")
                                .to_string()
                        };
                        (
                            k("columna"),
                            if k("operador").is_empty() {
                                "eq".into()
                            } else {
                                k("operador")
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let limit = n
            .get("limit")
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse::<u64>().ok());
        let orden = n.get("orderBy").map(|(_, v)| Json::de_node(v));
        let token = p
            .cabeceras
            .get("authorization")
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })
            .map(|v| v.trim().to_string());
        let mut nota = Anotacion {
            persona: quien.persona.clone(),
            agente: quien.agente.clone(),
            rama: rama.clone(),
            tabla: tabla.clone(),
            origen: String::new(),
            columnas: columnas.clone(),
            predicados,
            limit,
            token,
            decision: self.decision_de_quien_llama(p),
            empezo: Instant::now(),
        };

        // ⓪–④ El plan: la política de main, el resto de la rama.
        let plan = match self.plan_federado(
            &tabla,
            &columnas,
            filtros.as_ref(),
            rama.as_deref(),
            desde_puesto,
        ) {
            Ok(plan) => plan,
            Err(r) => {
                self.anotar(&nota, "negado", &motivo_de(&r), 0, 0);
                return Salida::Una(r);
            }
        };
        let texto = |k: &str| {
            plan.get(k)
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        if texto("ok") != "true" {
            let http = texto("http").parse::<u16>().unwrap_or(422);
            let r = error(http, &texto("codigo"), &texto("mensaje"));
            self.anotar(
                &nota,
                "negado",
                &format!("{}: {}", texto("codigo"), texto("mensaje")),
                0,
                0,
            );
            return Salida::Una(r);
        }
        let tabla = texto("tabla");
        nota.tabla = tabla.clone();
        let fuente = texto("fuente");
        nota.origen = fuente.clone();
        if nota.columnas.is_empty() {
            nota.columnas = plan
                .get("proyeccion")
                .map(|(_, v)| {
                    v.entries()
                        .iter()
                        .filter_map(|(k, _)| k.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
        }

        // ③ El acceso a la tabla (A8): ser de la organización ya se exigió.
        if let Err(r) = acceso_a_la_tabla(&quien, &tabla) {
            self.anotar(&nota, "negado", &motivo_de(&r), 0, 0);
            return Salida::Una(r);
        }

        // ⑤ El presupuesto.
        let (filas, bytes, ms) = if texto("fullScan") == "expensive" {
            (1_000_000u64, 1u64 << 30, 30_000u64)
        } else {
            (100_000, 64 << 20, 30_000)
        };

        // ⑥ La credencial, del custodio como el agente de la celda. Sin
        //   custodio (una máquina, una prueba), la del entorno: como `ore`.
        let env = texto("env");
        let url = if self.cofre.is_some() {
            match self.credencial_de_la_fuente(&fuente, &env) {
                Ok((u, _)) => u,
                Err(r) => {
                    self.anotar(&nota, "fallido", &motivo_de(&r), 0, 0);
                    return Salida::Una(r);
                }
            }
        } else {
            match std::env::var(&env) {
                Ok(u) if !u.is_empty() => u,
                _ => {
                    let r = error(
                        503,
                        "credencial",
                        &format!("sin custodio y sin `{env}` en el entorno"),
                    );
                    self.anotar(&nota, "fallido", "credencial: sin custodio", 0, 0);
                    return Salida::Una(r);
                }
            }
        };

        // ⑦ La petición a la pasarela.
        let mut peticion = vec![
            ("objeto", Json::s(texto("objeto"))),
            (
                "proyeccion",
                plan.get("proyeccion")
                    .map(|(_, v)| Json::de_node(v))
                    .unwrap_or(Json::obj([])),
            ),
        ];
        if let Some(f) = filtros {
            peticion.push(("filtros", f));
        }
        if let Some(l) = limit {
            peticion.push(("limit", Json::Int(l as i64)));
        }
        if let Some(o) = orden {
            peticion.push(("orderBy", o));
        }
        if let Some((_, f)) = plan.get("fichero") {
            peticion.push(("fichero", Json::de_node(f)));
        }
        // 0053 F9·1: el listado de un `ObjectTable`.
        if let Some((_, l)) = plan.get("listado") {
            peticion.push(("listado", Json::de_node(l)));
        }
        let id = format!("fed-{}", ore_acceso::nuevo_id().trim_start_matches("ev-"));
        let cuerpo = Json::obj([
            ("id", Json::s(id.as_str())),
            ("origen", Json::s(fuente.as_str())),
            ("tipo", Json::s(texto("tipo"))),
            ("url", Json::s(url.as_str())),
            ("peticion", Json::obj(peticion)),
            (
                "presupuesto",
                Json::obj([
                    ("filas", Json::Int(filas as i64)),
                    ("bytes", Json::Int(bytes as i64)),
                    ("ms", Json::Int(ms as i64)),
                ]),
            ),
        ])
        .jcs();
        drop(url);
        let (codigo, cabeceras, lector) = match abrir(&pasarela(), &cuerpo) {
            Ok(x) => x,
            Err(e) => {
                let r = error(
                    503,
                    "pasarela",
                    &format!("la pasarela de la celda no contesta: {e}"),
                );
                self.anotar(&nota, "fallido", "pasarela: no contesta", 0, 0);
                return Salida::Una(r);
            }
        };
        drop(cuerpo);
        if codigo != 200 {
            // Un error antes del primer byte: su JSON, tal cual (ya tapado).
            let mut b = String::new();
            let mut lector = lector;
            let _ = lector.read_to_string(&mut b);
            let n = ore_core::parse::parse(b.trim()).ok();
            let c = n
                .as_ref()
                .and_then(|n| {
                    n.get("codigo")
                        .and_then(|(_, v)| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            let m = n
                .as_ref()
                .and_then(|n| {
                    n.get("mensaje")
                        .and_then(|(_, v)| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            self.anotar(
                &nota,
                if codigo == 503 { "fallido" } else { "negado" },
                &format!("{c}: {m}"),
                0,
                0,
            );
            return Salida::Una(Respuesta {
                codigo,
                cuerpo: Json::Crudo(b.trim().to_string()),
            });
        }
        let _ = cabeceras;
        let finales: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let leido = Desenmarcado {
            de: lector,
            queda: 0,
            fin: false,
            finales: finales.clone(),
            al_soltar: Some(Box::new({
                let buzon = self.buzon.clone();
                let finales = finales.clone();
                let id = id.clone();
                move || {
                    let f = finales.lock().map(|f| f.clone()).unwrap_or_default();
                    let campo = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
                    let (estado, motivo) = match campo("ore-estado") {
                        Some(e) => (e, campo("ore-motivo").unwrap_or_default()),
                        None => ("cortado".into(), "desconexion".into()),
                    };
                    let num = |k: &str| campo(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                    guardar_final(
                        &id,
                        &nota.persona,
                        Json::obj([
                            ("id", Json::s(id.as_str())),
                            ("tabla", Json::s(nota.tabla.as_str())),
                            ("estado", Json::s(estado.as_str())),
                            ("motivo", Json::s(motivo.as_str())),
                            ("filas", Json::Int(num("ore-filas") as i64)),
                            ("bytes", Json::Int(num("ore-bytes") as i64)),
                        ]),
                    );
                    let resultado = if estado == "error" {
                        "fallido"
                    } else {
                        "hecho"
                    };
                    emitir(
                        buzon.as_ref(),
                        &nota,
                        resultado,
                        &estado,
                        &motivo,
                        num("ore-filas"),
                        num("ore-bytes"),
                    );
                }
            })),
        };
        Salida::Bytes(Bytes {
            codigo: 200,
            cabeceras: vec![
                (
                    "content-type".into(),
                    "application/vnd.apache.arrow.stream".into(),
                ),
                ("ore-lectura".into(), id),
            ],
            largo: None,
            lector: Box::new(leido),
            finales: Some(Finales {
                nombres: FINALES.iter().map(|s| s.to_string()).collect(),
                valores: Box::new(move || finales.lock().map(|f| f.clone()).unwrap_or_default()),
            }),
        })
    }

    /// El plan, de `ore federate`: la política de `main` a un directorio
    /// aparte, y el árbol de la rama.
    fn plan_federado(
        &self,
        tabla: &str,
        columnas: &[String],
        filtros: Option<&Json>,
        rama: Option<&str>,
        desde_puesto: bool,
    ) -> Result<ore_core::parse::Node, Respuesta> {
        let dir = self.politica_federada()?;
        let mut args = vec![
            "federate".to_string(),
            "--table".into(),
            tabla.to_string(),
            "--policy".into(),
            dir.to_string_lossy().into_owned(),
        ];
        if !columnas.is_empty() {
            args.push("--columns".into());
            args.push(columnas.join(","));
        }
        if let Some(f) = filtros {
            args.push("--filters".into());
            args.push(f.jcs());
        }
        if desde_puesto {
            args.push("--from-workspace".into());
        }
        let mut salida: Option<String> = None;
        let r = self.leyendo_en(rama, |raiz| {
            args.push("--path".into());
            args.push(raiz.to_string_lossy().into_owned());
            match crate::mando::correr(&self.binario, raiz, &args) {
                Ok(s) if s.bien() => {
                    salida = s
                        .stdout
                        .lines()
                        .rev()
                        .find(|l| l.trim_start().starts_with('{'))
                        .map(String::from);
                    Respuesta::ok(Json::obj([]))
                }
                Ok(s) => Respuesta::error(
                    502,
                    format!("`ore federate` devolvió {}: {}", s.codigo, s.stderr.trim()),
                ),
                Err(e) => Respuesta::error(500, e.to_string()),
            }
        });
        let _ = std::fs::remove_dir_all(&dir);
        let Some(linea) = salida else {
            return Err(r);
        };
        ore_core::parse::parse(&linea)
            .map_err(|_| Respuesta::error(502, "`ore federate` no devolvió JSON"))
    }

    /// La política que manda —`ontology.config.yaml` y `conduits.yaml` de
    /// main— en un directorio aparte, para `--policy`. Quien llama lo borra.
    fn politica_federada(&self) -> Result<std::path::PathBuf, Respuesta> {
        let mut politica: Option<(String, Option<String>)> = None;
        let r =
            self.leyendo(
                |raiz| match std::fs::read_to_string(raiz.join("ontology.config.yaml")) {
                    Ok(m) => {
                        politica =
                            Some((m, std::fs::read_to_string(raiz.join("conduits.yaml")).ok()));
                        Respuesta::ok(Json::obj([]))
                    }
                    Err(e) => {
                        Respuesta::error(500, format!("main no tiene `ontology.config.yaml`: {e}"))
                    }
                },
            );
        let Some((manifiesto, conductos)) = politica else {
            return Err(r);
        };
        let dir = std::env::temp_dir().join(format!("ore-politica-{}", ore_acceso::nuevo_id()));
        let escrito = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(dir.join("ontology.config.yaml"), &manifiesto))
            .and_then(|_| match &conductos {
                Some(c) => std::fs::write(dir.join("conduits.yaml"), c),
                None => Ok(()),
            });
        if let Err(e) = escrito {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Respuesta::error(
                500,
                format!("no se pudo dejar la política de main: {e}"),
            ));
        }
        Ok(dir)
    }

    /// **El reparto de una sentencia del puesto** (0053 F6·1): `ore explain`
    /// en la rama, con la política de main y como lectura que acaba en un
    /// puesto. `json`: la línea del plan (`{"ok": …}`); si no, el texto —o el
    /// `error[CÓDIGO]` que dio—.
    pub(crate) fn explicar(
        &self,
        rama: Option<&str>,
        texto: &str,
        json: bool,
    ) -> Result<String, Respuesta> {
        let dir = self.politica_federada()?;
        let fichero = dir.join("consulta.sql");
        if let Err(e) = std::fs::write(&fichero, texto) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Respuesta::error(
                500,
                format!("no se pudo dejar la consulta: {e}"),
            ));
        }
        let mut args = vec![
            "explain".to_string(),
            "--policy".into(),
            dir.to_string_lossy().into_owned(),
            "--from-workspace".into(),
            "--file".into(),
            fichero.to_string_lossy().into_owned(),
        ];
        if json {
            args.push("--json".into());
        }
        let mut salida: Option<String> = None;
        let r = self.leyendo_en(rama, |raiz| {
            args.push("--path".into());
            args.push(raiz.to_string_lossy().into_owned());
            match crate::mando::correr(&self.binario, raiz, &args) {
                Ok(s) if json && s.bien() => {
                    salida = s
                        .stdout
                        .lines()
                        .rev()
                        .find(|l| l.trim_start().starts_with('{'))
                        .map(String::from);
                    Respuesta::ok(Json::obj([]))
                }
                Ok(s) if !json => {
                    salida = Some(if s.bien() {
                        s.stdout
                    } else {
                        s.stderr.trim().to_string()
                    });
                    Respuesta::ok(Json::obj([]))
                }
                Ok(s) => Respuesta::error(
                    502,
                    format!("`ore explain` devolvió {}: {}", s.codigo, s.stderr.trim()),
                ),
                Err(e) => Respuesta::error(500, e.to_string()),
            }
        });
        let _ = std::fs::remove_dir_all(&dir);
        salida.ok_or(r)
    }

    /// `GET /federation/read/{id}` (0053 F6·1): **cómo acabó una lectura** —su
    /// estado, por qué y cuántas filas—: lo que va en los *trailers* y ningún
    /// SDK sabe leer (ni `urllib`, ni `fetch`, ni el cliente de Java). Sólo a
    /// quien la hizo; vive unos minutos.
    pub(crate) fn final_de_lectura(&self, p: &Peticion, sujeto: &Identidad, id: &str) -> Respuesta {
        let quien = match self.sujeto_del_puesto(p, sujeto, None) {
            Ok((q, _)) => q.persona,
            Err(r) => return r,
        };
        let mut g = RECIENTES.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|(_, _, t, _)| t.elapsed() < VIDA_DE_UN_FINAL);
        match g
            .iter()
            .find(|(i, persona, _, _)| i == id && *persona == quien)
        {
            Some((_, _, _, j)) => Respuesta::ok(j.clone()),
            None => Respuesta::error(
                404,
                format!("no hay una lectura `{id}` tuya terminada en los últimos minutos"),
            ),
        }
    }

    fn anotar(&self, nota: &Anotacion, resultado: &str, motivo: &str, filas: u64, bytes: u64) {
        let estado = if resultado == "negado" {
            "negado"
        } else {
            "error"
        };
        emitir(
            self.buzon.as_ref(),
            nota,
            resultado,
            estado,
            motivo,
            filas,
            bytes,
        );
    }

    /// **`PUT /fuentes/{n}`** `{"federation": true|false}`: el interruptor, en
    /// `main` (las fuentes son de la celda).
    pub(crate) fn federacion_de_fuente(
        &self,
        raiz: &std::path::Path,
        nombre: &str,
        cuerpo: &str,
    ) -> Respuesta {
        let si = match ore_core::parse::parse(cuerpo)
            .ok()
            .and_then(|n| {
                n.get("federation")
                    .and_then(|(_, v)| v.as_str())
                    .map(String::from)
            })
            .as_deref()
        {
            Some("true") => true,
            Some("false") => false,
            _ => return Respuesta::error(422, "falta `federation` (true o false)"),
        };
        let args = vec![
            "source".to_string(),
            "federation".into(),
            nombre.to_string(),
            if si { "on".into() } else { "off".into() },
            "--path".into(),
            raiz.to_string_lossy().into_owned(),
        ];
        match crate::mando::correr(&self.binario, raiz, &args) {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => Respuesta::error(
                if s.stderr.contains("no está declarada") {
                    404
                } else {
                    422
                },
                s.stderr.trim().trim_start_matches("error: ").to_string(),
            ),
            Ok(s) => match s.stdout.lines().rev().find(|l| l.starts_with('{')) {
                Some(l) => Respuesta::ok(Json::Crudo(l.to_string())),
                None => Respuesta::ok(Json::obj([
                    ("name", Json::s(nombre)),
                    ("federation", Json::Bool(si)),
                ])),
            },
        }
    }
}

/// **El enganche de A8** (0047): la acción `dato:leer` sobre la tabla. `ore-iam`
/// todavía sólo decide sobre la organización («este motor sólo decide sobre la
/// organización todavía»), y ser de ella ya se exigió en `quien`. Cuando A8
/// llegue, la pregunta va aquí; hasta entonces deja pasar, y se dice.
fn acceso_a_la_tabla(_quien: &Identidad, _tabla: &str) -> Result<(), Respuesta> {
    Ok(())
}

fn emitir(
    buzon: Option<&ore_acceso::Buzon>,
    nota: &Anotacion,
    resultado: &str,
    estado: &str,
    motivo: &str,
    filas: u64,
    bytes: u64,
) {
    let Some(buzon) = buzon else {
        eprintln!(
            "federation:read · {} · {} · {estado} {motivo} · {filas} filas",
            nota.persona, nota.tabla
        );
        return;
    };
    let mut e = crate::acceso::evento(
        "federation:read",
        &format!("tablas/{}", nota.tabla.replace('.', "/")),
        resultado,
        nota.decision.clone(),
        None,
    );
    let mut d = vec![
        ("persona", Json::s(nota.persona.as_str())),
        ("tabla", Json::s(nota.tabla.as_str())),
        ("origen", Json::s(nota.origen.as_str())),
        (
            "columnas",
            Json::Arr(nota.columnas.iter().map(|c| Json::s(c.as_str())).collect()),
        ),
        (
            "predicados",
            Json::Arr(
                nota.predicados
                    .iter()
                    .map(|(c, o)| {
                        Json::obj([
                            ("columna", Json::s(c.as_str())),
                            ("operador", Json::s(o.as_str())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("estado", Json::s(estado)),
        ("motivo", Json::s(motivo)),
        ("filas", Json::Int(filas as i64)),
        ("bytes", Json::Int(bytes as i64)),
        ("ms", Json::Int(nota.empezo.elapsed().as_millis() as i64)),
    ];
    if let Some(a) = &nota.agente {
        d.push(("act", Json::s(a.as_str())));
    }
    if let Some(r) = &nota.rama {
        d.push(("rama", Json::s(r.as_str())));
    }
    if let Some(l) = nota.limit {
        d.push(("limit", Json::Int(l as i64)));
    }
    e.detalle = Some(Json::obj(d));
    buzon.echar(nota.token.clone(), e);
}

fn motivo_de(r: &Respuesta) -> String {
    match &r.cuerpo {
        Json::Obj(m) => m
            .get("mensaje")
            .or_else(|| m.get("error"))
            .map(|v| match v {
                Json::Str(s) => s.clone(),
                o => o.jcs(),
            })
            .unwrap_or_default(),
        o => o.jcs(),
    }
}

fn error(codigo: u16, cod: &str, mensaje: &str) -> Respuesta {
    Respuesta {
        codigo,
        cuerpo: Json::obj([
            ("codigo", Json::s(cod)),
            ("mensaje", Json::s(mensaje)),
            ("error", Json::s(mensaje)),
        ]),
    }
}

/// Los finales de las lecturas recientes: `(id, persona, cuándo, final)`.
type Final = (String, String, Instant, Json);
static RECIENTES: Mutex<std::collections::VecDeque<Final>> =
    Mutex::new(std::collections::VecDeque::new());
const VIDA_DE_UN_FINAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);
const RECIENTES_COMO_MUCHO: usize = 4096;

fn guardar_final(id: &str, persona: &str, f: Json) {
    let mut g = RECIENTES.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|(_, _, t, _)| t.elapsed() < VIDA_DE_UN_FINAL);
    while g.len() >= RECIENTES_COMO_MUCHO {
        g.pop_front();
    }
    g.push_back((id.to_string(), persona.to_string(), Instant::now(), f));
}

/// El estado, las cabeceras y el cuerpo de una lectura abierta en la pasarela.
type Abierta = (u16, Vec<(String, String)>, BufReader<TcpStream>);

/// Abre la lectura en la pasarela: su estado, sus cabeceras y el cuerpo.
fn abrir(destino: &str, cuerpo: &str) -> Result<Abierta, String> {
    use std::net::ToSocketAddrs;
    let dir = destino
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("sin dirección")?;
    let mut s =
        TcpStream::connect_timeout(&dir, Duration::from_secs(5)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let req = format!(
        "POST /v1/read HTTP/1.1\r\nhost: pasarela\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        cuerpo.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    s.write_all(cuerpo.as_bytes()).map_err(|e| e.to_string())?;
    let mut c = BufReader::new(s);
    let mut l = String::new();
    c.read_line(&mut l).map_err(|e| e.to_string())?;
    let codigo = l
        .split_whitespace()
        .nth(1)
        .and_then(|x| x.parse().ok())
        .ok_or("una línea de estado rara")?;
    let mut cabeceras = Vec::new();
    loop {
        let mut l = String::new();
        c.read_line(&mut l).map_err(|e| e.to_string())?;
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            cabeceras.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    Ok((codigo, cabeceras, c))
}

/// **El cuerpo troceado de la pasarela, desenmarcado al paso**: lo que llega
/// sale hacia quien pidió sin juntarse, y los *trailers* se guardan para los
/// suyos. Al soltarse —terminado o porque quien leía se fue— anota la lectura.
struct Desenmarcado {
    de: BufReader<TcpStream>,
    queda: usize,
    fin: bool,
    finales: Arc<Mutex<Vec<(String, String)>>>,
    al_soltar: Option<Box<dyn FnOnce() + Send>>,
}

impl Read for Desenmarcado {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.fin {
            return Ok(0);
        }
        if self.queda == 0 {
            let mut l = String::new();
            self.de.read_line(&mut l)?;
            let n = usize::from_str_radix(l.trim(), 16)
                .map_err(|_| std::io::Error::other("un trozo raro de la pasarela"))?;
            if n == 0 {
                let mut f = Vec::new();
                loop {
                    let mut l = String::new();
                    if self.de.read_line(&mut l)? == 0 {
                        break;
                    }
                    let l = l.trim_end();
                    if l.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = l.split_once(':') {
                        f.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                    }
                }
                if let Ok(mut g) = self.finales.lock() {
                    *g = f;
                }
                self.fin = true;
                return Ok(0);
            }
            self.queda = n;
        }
        let k = buf.len().min(self.queda);
        let leidos = self.de.read(&mut buf[..k])?;
        if leidos == 0 {
            return Err(std::io::Error::other("la pasarela cortó a mitad"));
        }
        self.queda -= leidos;
        if self.queda == 0 {
            let mut crlf = [0u8; 2];
            self.de.read_exact(&mut crlf)?;
        }
        Ok(leidos)
    }
}

impl Drop for Desenmarcado {
    fn drop(&mut self) {
        if let Some(f) = self.al_soltar.take() {
            f();
        }
    }
}
