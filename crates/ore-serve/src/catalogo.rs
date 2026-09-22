//! **El catálogo REST de Iceberg, servido** (W3.6c c3, [0031 §11](../../../docs/decisions/0031-el-puesto.md) ①③):
//! la cara por la que PyIceberg, DuckDB, Spark, y el agente del puesto tras
//! `ore-store escribir`, escriben en el lago del inquilino **con las reglas del
//! árbol**. Medido antes (`medida-w3-escribir.py`): PyIceberg y DuckDB escriben
//! contra estas rutas sin parches; DuckDB va siempre por `transactions/commit`
//! y crea con `stage-create` + `assert-create`; los dos piden
//! `vended-credentials` al cargar la tabla.
//!
//! | ruta | qué | quién decide |
//! |---|---|---|
//! | `GET /v1/config` | los defectos: ninguno | aquí |
//! | `GET /v1/namespaces[/{ns}[/tables]]` | los paquetes del árbol, y las tablas del lago con puntero | el árbol (`packages/*/package.yaml`, `ore datasets`) |
//! | `GET\|HEAD /v1/namespaces/{ns}/tables/{t}` | el `LoadTableResult`: puntero + `metadata.json` + **la credencial acotada a la tabla** si el cliente manda `X-Iceberg-Access-Delegation: vended-credentials` | `ore datasets --cargar [--prestar]` |
//! | `POST /v1/namespaces/{ns}/tables` | la tabla nace (`--crear`), o se esboza sin escribir nada si `stage-create` (`--esbozar`) | `ore datasets` |
//! | `POST /v1/namespaces/{ns}/tables/{t}` | `updateTable`: `requirements` + `updates` → el `metadata.json` siguiente, la `Table`, el puntero, **y el commit lo empuja este proceso** | `ore datasets --commit --tabla` |
//! | `POST /v1/transactions/commit` | `commitTransaction`: N tablas, un commit del árbol | `ore datasets --commit` |
//!
//! # Los códigos, que son los de la spec
//!
//! `409 CommitFailedException` en las dos caras del CAS —la semántica (código
//! 75 de `ore`: el puntero ya no es la base) y la de la forja (`[remote
//! rejected]`)—: el cliente refresca y reintenta (PyIceberg lo hace solo,
//! medido). `404 NoSuchTableException` / `NoSuchNamespaceException`. `400
//! BadRequestException` para lo que `ore` niega (una `View` como destino, una
//! `Table` de otra fuente, un cuerpo que no se entiende). Y **5xx
//! `CommitStateUnknownException`** cuando el commit pudo entrar y no se sabe:
//! el cliente tiene que mirar (`GET …/tables/{t}`) antes de reintentar.
//! El cuerpo de error lleva la forma de la spec: `{"error": {"message", "type",
//! "code"}}`.
//!
//! # Lo que este proceso escribe en el bucket
//!
//! Desde W3.6c, **el `metadata.json`** (`ore-store aplicar`, con la identidad
//! del pod: `objectCreator` + `objectViewer`, aprovisionador ③b) — como un
//! catálogo REST. Los ficheros de datos y los manifiestos los escribe el
//! cliente con la credencial que se le prestó: acotada a `ore/v2/datasets/
//! <ns>_<t>/`, sin borrar ni sobrescribir (medido: 200/403/403/403/403).
//!
//! # La rama
//!
//! `x-ore-rama` (0030 W2) elige en qué rama se lee la tabla y en cuál se
//! mueve el puntero: una sesión en una rama escribe en la suya (§11 ⑦).
//!
//! # Lo que no entra (todavía)
//!
//! Vistas de Iceberg, renombrar, borrar tablas, `register`, métricas, el
//! escaneo por el servidor, `Idempotency-Key` (ningún cliente la manda,
//! medido; la clave de operación del snapshot hace ese trabajo).

use std::path::Path;

use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::Identidad;

use crate::mando;
use crate::rutas::{Servidor, token};

/// La cabecera con la que un cliente pide la credencial prestada.
const DELEGACION: &str = "x-iceberg-access-delegation";

/// Un error con la forma de la spec REST.
fn error(codigo: u16, tipo: &str, mensaje: impl Into<String>) -> Respuesta {
    Respuesta {
        codigo,
        cuerpo: Json::obj([(
            "error",
            Json::obj([
                ("code", Json::Int(codigo as i64)),
                ("message", Json::s(mensaje.into())),
                ("type", Json::s(tipo)),
            ]),
        )]),
    }
}

/// Lo que `ore` dijo (su primera línea de error) con el código de la spec que
/// le toca a su código de salida.
fn de_ore(codigo: i32, stderr: &str, commit: bool) -> Respuesta {
    let m = primera_de(stderr);
    match codigo {
        75 => error(409, "CommitFailedException", m),
        // Lo escrito es de quien lo escribió (W3.7 gobierno ④).
        77 => error(403, "ForbiddenException", m),
        64 => error(400, "BadRequestException", m),
        65 if m.contains("no hay ningún dataset") || m.contains("no hay ningún paquete") => {
            error(404, "NoSuchTableException", m)
        }
        65 => error(400, "BadRequestException", m),
        _ if commit => error(500, "CommitStateUnknownException", m),
        _ => error(500, "InternalServerError", m),
    }
}

/// Qué tabla toca esta petición del catálogo, como `<ns>.<tabla>`; `None` si
/// no toca ninguna (`config`, `namespaces`) o si es un `commitTransaction`,
/// que las trae en el cuerpo y las mira `ore datasets --commit`.
fn tabla_de(p: &Peticion, seg: &[&str]) -> Option<String> {
    match seg {
        ["namespaces", ns, "tables", t] => Some(format!("{ns}.{t}")),
        // Crear una tabla: el nombre va en el cuerpo.
        ["namespaces", ns, "tables"] if p.metodo == "POST" => ore_core::parse::parse(&p.cuerpo)
            .ok()
            .and_then(|n| {
                n.get("name")
                    .and_then(|(_, v)| v.as_str().map(String::from))
            })
            .map(|t| format!("{ns}.{t}")),
        _ => None,
    }
}

/// Una respuesta que ya venía con `{"error": "…"}` (la forja adelantada, un
/// 401, un 422 de la rama) pasa a la forma de la spec.
fn con_forma(r: Respuesta, commit: bool) -> Respuesta {
    if r.codigo < 400 {
        return r;
    }
    let m = match &r.cuerpo {
        Json::Obj(m) => match m.get("error") {
            Some(Json::Str(s)) => s.clone(),
            Some(Json::Obj(_)) => return r,
            _ => "falló sin decir por qué".into(),
        },
        _ => "falló sin decir por qué".into(),
    };
    match r.codigo {
        409 => error(409, "CommitFailedException", m),
        404 => error(404, "NoSuchTableException", m),
        401 => error(401, "NotAuthorizedException", m),
        403 => error(403, "ForbiddenException", m),
        400 | 422 => error(400, "BadRequestException", m),
        c if commit => error(c, "CommitStateUnknownException", m),
        c => error(c, "InternalServerError", m),
    }
}

fn primera_de(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start_matches("error: ").to_string())
        .unwrap_or_else(|| "falló sin decir por qué".into())
}

/// La última línea JSON de lo que `ore` imprimió, **tal cual** (un
/// `metadata.json` lleva `null` y números: no se reanaliza).
fn ultima_json(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .map(String::from)
}

fn ns_valido(ns: &str) -> Result<(), Respuesta> {
    token(ns).map_err(|m| error(400, "BadRequestException", m))
}

impl Servidor {
    /// Todas las rutas bajo `/v1/`.
    pub(crate) fn catalogo(
        &self,
        p: &Peticion,
        sujeto: &Identidad,
        rama: Option<&str>,
        seg: &[&str],
    ) -> Respuesta {
        let prestar = p
            .cabeceras
            .get(DELEGACION)
            .is_some_and(|v| v.contains("vended-credentials"));
        // Desde un puesto: quien escribe es la persona, no el agente.
        let (sujeto, rama) = match self.sujeto_del_puesto(p, sujeto, rama) {
            Ok(x) => x,
            Err(r) => return con_forma(r, false),
        };
        let sujeto = &sujeto;
        let rama = rama.as_deref();
        // **Lo declarado manda** (0031 W3.7 gobierno ⑤): mientras un transform
        // corre en este puesto, el catálogo sólo carga, esboza o confirma su
        // `output`. Lo demás —cargar otra tabla para escribirla, crearla,
        // commitear sobre ella— es 403 con lo declarado, en el servidor y no
        // en el SDK. Leer va por `datos`, que lo acota igual.
        if let Some(id) = p.cabeceras.get(crate::puestos::PUESTO)
            && let Some(t) = self.transform_de(id.trim())
            && let Some(tabla) = tabla_de(p, seg)
            && tabla != t.output
        {
            return con_forma(
                Respuesta::error(
                    403,
                    format!(
                        "`{tabla}` no es el output de `{}` (`{}`): un transform sólo escribe lo que declara",
                        t.nombre, t.output
                    ),
                ),
                false,
            );
        }
        match (p.metodo.as_str(), seg) {
            ("GET", ["config"]) => Respuesta::ok(Json::obj([
                ("defaults", Json::obj([])),
                ("overrides", Json::obj([])),
            ])),
            ("GET", ["namespaces"]) => self.leyendo_en(rama, |raiz| {
                Respuesta::ok(Json::obj([(
                    "namespaces",
                    Json::Arr(
                        paquetes(raiz)
                            .into_iter()
                            .map(|n| Json::Arr(vec![Json::s(n)]))
                            .collect(),
                    ),
                )]))
            }),
            ("GET" | "HEAD", ["namespaces", ns]) => {
                if let Err(r) = ns_valido(ns) {
                    return r;
                }
                let ns = ns.to_string();
                con_forma(
                    self.leyendo_en(rama, move |raiz| {
                        if paquetes(raiz).contains(&ns) {
                            Respuesta::ok(Json::obj([
                                ("namespace", Json::Arr(vec![Json::s(&ns)])),
                                ("properties", Json::obj([])),
                            ]))
                        } else {
                            error(
                                404,
                                "NoSuchNamespaceException",
                                format!("no hay ningún paquete `{ns}`"),
                            )
                        }
                    }),
                    false,
                )
            }
            ("GET", ["namespaces", ns, "tables"]) => {
                if let Err(r) = ns_valido(ns) {
                    return r;
                }
                let ns = ns.to_string();
                con_forma(
                    self.leyendo_en(rama, move |raiz| self.tablas(raiz, &ns)),
                    false,
                )
            }
            ("GET" | "HEAD", ["namespaces", ns, "tables", t]) => {
                if let Err(r) = ns_valido(ns).and_then(|_| ns_valido(t)) {
                    return r;
                }
                let nombre = format!("{ns}.{t}");
                let cabeza = p.metodo == "HEAD";
                let sujeto_s = sujeto.persona.clone();
                con_forma(
                    self.leyendo_en(rama, move |raiz| {
                        let r = self.cargar(raiz, &nombre, prestar, &sujeto_s);
                        if cabeza && r.codigo == 200 {
                            Respuesta::sin_contenido()
                        } else {
                            r
                        }
                    }),
                    false,
                )
            }
            ("POST", ["namespaces", ns, "tables"]) => {
                if let Err(r) = ns_valido(ns) {
                    return r;
                }
                let cuerpo = match ore_core::parse::parse(&p.cuerpo) {
                    Ok(c) if !p.cuerpo.trim().is_empty() => c,
                    _ => return error(400, "BadRequestException", "el cuerpo no es JSON"),
                };
                let Some(nombre) = cuerpo
                    .get("name")
                    .and_then(|(_, v)| v.as_str())
                    .filter(|s| !s.is_empty())
                else {
                    return error(400, "BadRequestException", "falta `name`");
                };
                if let Err(r) = ns_valido(nombre) {
                    return r;
                }
                let nombre = format!("{ns}.{nombre}");
                let esbozo = cuerpo
                    .get("stage-create")
                    .and_then(|(_, v)| v.as_str())
                    .is_some_and(|v| v == "true");
                let peticion = p.cuerpo.clone();
                if esbozo {
                    // Sin escribir nada: la tabla nace en el commit que siga.
                    con_forma(
                        self.leyendo_en(rama, move |raiz| {
                            let mut args = vec![
                                "datasets",
                                ".",
                                "--esbozar",
                                &nombre,
                                "--peticion",
                                &peticion,
                            ];
                            if prestar {
                                args.push("--prestar");
                            }
                            self.ore_crudo(raiz, &args, false)
                        }),
                        false,
                    )
                } else {
                    let sujeto_s = sujeto.persona.clone();
                    con_forma(
                        self.escribiendo_en(
                            rama,
                            sujeto,
                            &format!("crear `{nombre}`"),
                            move |raiz| {
                                let defecto = self.retencion_defecto();
                                let mut args = vec![
                                    "datasets",
                                    ".",
                                    "--crear",
                                    &nombre,
                                    "--peticion",
                                    &peticion,
                                    "--sujeto",
                                    &sujeto_s,
                                    "--json",
                                ];
                                if let Some(d) = &defecto {
                                    args.push("--retencion-defecto");
                                    args.push(d);
                                }
                                let r = self.ore_crudo(raiz, &args, true);
                                if r.codigo >= 300 {
                                    return r;
                                }
                                self.cargar(raiz, &nombre, prestar, &sujeto_s)
                            },
                        ),
                        true,
                    )
                }
            }
            ("POST", ["namespaces", ns, "tables", t]) => {
                if let Err(r) = ns_valido(ns).and_then(|_| ns_valido(t)) {
                    return r;
                }
                if p.cuerpo.trim().is_empty() || ore_core::parse::parse(&p.cuerpo).is_err() {
                    return error(400, "BadRequestException", "el cuerpo no es JSON");
                }
                let nombre = format!("{ns}.{t}");
                let peticion = p.cuerpo.clone();
                let sujeto_s = sujeto.persona.clone();
                con_forma(
                    self.escribiendo_en(
                        rama,
                        sujeto,
                        &format!("escribir `{nombre}`"),
                        move |raiz| {
                            let defecto = self.retencion_defecto();
                            let mut args = vec![
                                "datasets",
                                ".",
                                "--commit",
                                "--tabla",
                                &nombre,
                                "--peticion",
                                &peticion,
                                "--sujeto",
                                &sujeto_s,
                                "--json",
                            ];
                            if let Some(d) = &defecto {
                                args.push("--retencion-defecto");
                                args.push(d);
                            }
                            let r = self.ore_crudo(raiz, &args, true);
                            if r.codigo >= 300 {
                                return r;
                            }
                            // `{"metadata-location", "metadata"}`: lo que la spec
                            // devuelve tras un commit, sin credencial.
                            self.cargar(raiz, &nombre, false, "")
                        },
                    ),
                    true,
                )
            }
            ("POST", ["transactions", "commit"]) => {
                if p.cuerpo.trim().is_empty() || ore_core::parse::parse(&p.cuerpo).is_err() {
                    return error(400, "BadRequestException", "el cuerpo no es JSON");
                }
                let peticion = p.cuerpo.clone();
                let sujeto_s = sujeto.persona.clone();
                con_forma(
                    self.escribiendo_en(rama, sujeto, "escribir varias tablas", move |raiz| {
                        let defecto = self.retencion_defecto();
                        let mut args = vec![
                            "datasets",
                            ".",
                            "--commit",
                            "--peticion",
                            &peticion,
                            "--sujeto",
                            &sujeto_s,
                            "--json",
                        ];
                        if let Some(d) = &defecto {
                            args.push("--retencion-defecto");
                            args.push(d);
                        }
                        let r = self.ore_crudo(raiz, &args, true);
                        if r.codigo >= 300 {
                            return r;
                        }
                        Respuesta::sin_contenido()
                    }),
                    true,
                )
            }
            _ => error(404, "NotFoundException", format!("no hay `{}`", p.ruta)),
        }
    }

    /// La retención con la que nace una tabla que no la trae: `ORE_RETENCION`
    /// (`7d`, `30d`, `0`); sin ella, ninguna (y nada expira, §11 ⑥).
    fn retencion_defecto(&self) -> Option<String> {
        std::env::var("ORE_RETENCION")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    /// Corre `ore datasets …` y devuelve su última línea JSON **tal cual**.
    fn ore_crudo(&self, raiz: &Path, args: &[&str], commit: bool) -> Respuesta {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let s = match mando::correr(&self.binario, raiz, &args) {
            Ok(s) => s,
            Err(e) => return error(500, "InternalServerError", e.to_string()),
        };
        if s.codigo != 0 {
            let mut r = de_ore(s.codigo, &s.stderr, commit);
            // El 75 trae `actual`: se conserva junto al error.
            if s.codigo == 75
                && let Some(j) = ultima_json(&s.stdout)
                && let Json::Obj(m) = &mut r.cuerpo
            {
                m.insert("actual".into(), Json::Crudo(j));
            }
            return r;
        }
        match ultima_json(&s.stdout) {
            Some(j) => Respuesta::ok(Json::Crudo(j)),
            None => error(
                500,
                "InternalServerError",
                "`ore datasets` no devolvió JSON",
            ),
        }
    }

    /// El `LoadTableResult` de una tabla.
    fn cargar(&self, raiz: &Path, nombre: &str, prestar: bool, sujeto: &str) -> Respuesta {
        let mut args = vec!["datasets", ".", "--cargar", nombre];
        if prestar {
            // La credencial para escribir es de quien escribió: `ore` decide
            // con el sujeto (W3.7 gobierno ④).
            args.extend(["--prestar", "--sujeto", sujeto]);
        }
        self.ore_crudo(raiz, &args, false)
    }

    /// Las tablas del lago con puntero, en un paquete.
    fn tablas(&self, raiz: &Path, ns: &str) -> Respuesta {
        let args: Vec<String> = ["datasets", ".", "--json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = match mando::correr(&self.binario, raiz, &args) {
            Ok(s) => s,
            Err(e) => return error(500, "InternalServerError", e.to_string()),
        };
        let Some(j) = ultima_json(&s.stdout).and_then(|l| ore_core::parse::parse(&l).ok()) else {
            return error(
                500,
                "InternalServerError",
                "`ore datasets` no devolvió JSON",
            );
        };
        let ids: Vec<Json> = j
            .get("datasets")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter(|d| d.get("clase").and_then(|(_, v)| v.as_str()) == Some("dataset"))
            .filter_map(|d| d.get("nombre").and_then(|(_, v)| v.as_str()))
            .filter_map(|n| n.split_once('.'))
            .filter(|(p, _)| *p == ns)
            .map(|(p, t)| {
                Json::obj([
                    ("name", Json::s(t)),
                    ("namespace", Json::Arr(vec![Json::s(p)])),
                ])
            })
            .collect();
        Respuesta::ok(Json::obj([("identifiers", Json::Arr(ids))]))
    }
}

/// Los paquetes del árbol: los directorios de `packages/` con `package.yaml`.
fn paquetes(raiz: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(raiz.join("packages"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().join("package.yaml").is_file())
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los códigos de `ore` → los de la spec, con su tipo.
    #[test]
    fn los_codigos_de_ore_son_los_de_la_spec() {
        let r = de_ore(75, "error: el puntero de `v.t` ya no es la base\n", true);
        assert_eq!(r.codigo, 409);
        assert!(
            r.cuerpo.jcs().contains("CommitFailedException"),
            "{}",
            r.cuerpo.jcs()
        );
        let r = de_ore(65, "error: no hay ningún dataset `v.t`\n", false);
        assert_eq!(r.codigo, 404);
        assert!(r.cuerpo.jcs().contains("NoSuchTableException"));
        let r = de_ore(
            65,
            "error: `v.t` es una View: una consulta no se escribe\n",
            true,
        );
        assert_eq!(r.codigo, 400);
        let r = de_ore(69, "error: `ore-store-r2` falló\n", true);
        assert_eq!(r.codigo, 500);
        assert!(r.cuerpo.jcs().contains("CommitStateUnknownException"));
        // y lo que ya venía como error plano toma la forma
        let r = con_forma(Respuesta::error(409, "[remote rejected] main"), true);
        assert!(r.cuerpo.jcs().contains("CommitFailedException"));
        let r = con_forma(Respuesta::ok(Json::Crudo("{\"a\":null}".into())), true);
        assert_eq!(r.cuerpo.jcs(), "{\"a\":null}");
    }
}
