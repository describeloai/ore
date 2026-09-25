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
//! | `GET /v1/config` | los defectos: ninguno; los `endpoints` que se sirven (con los de vistas: sin ellos Spark no las pide) | aquí |
//! | `GET /v1/namespaces[/{ns}[/tables]]` | los paquetes del árbol, y las tablas del lago con puntero | el árbol (`packages/*/package.yaml`, `ore datasets`) |
//! | `GET\|HEAD /v1/namespaces/{ns}/tables/{t}` | el `LoadTableResult`: puntero + `metadata.json` + **la credencial acotada a la tabla** si el cliente manda `X-Iceberg-Access-Delegation: vended-credentials` | `ore datasets --cargar [--prestar]` |
//! | `POST /v1/namespaces/{ns}/tables` | la tabla nace (`--crear`), o se esboza sin escribir nada si `stage-create` (`--esbozar`) | `ore datasets` |
//! | `POST /v1/namespaces/{ns}/tables/{t}` | `updateTable`: `requirements` + `updates` → el `metadata.json` siguiente, la `Table`, el puntero, **y el commit lo empuja este proceso** | `ore datasets --commit --tabla` |
//! | `POST /v1/transactions/commit` | `commitTransaction`: N tablas, un commit del árbol | `ore datasets --commit` |
//! | `GET /v1/namespaces/{ns}/views` | las Views del paquete que no se llaman como un dataset | el árbol |
//! | `GET\|HEAD /v1/namespaces/{ns}/views/{v}` | el `LoadViewResult`: la View como SQL sobre sus datasets por su nombre del catálogo, su esquema de Iceberg, **con el conducto** | `ore ask --sql --catalogo` |
//! | `POST …/tables/{t}/metrics` | el informe de un escaneo (Spark lo manda tras cada lectura): 204, no se guarda | aquí |
//!
//! # Las Views (medido con Spark: `medida-spark-por-el-catalogo.py`)
//!
//! Spark resuelve un nombre pidiendo PRIMERO la tabla, y sólo si es 404
//! `NoSuchTableException` prueba `loadView`: por eso una View pedida como tabla
//! es 404 al leer (y no el 400 de «una consulta no se escribe», que le para).
//! Dentro de la SQL de la View cada dataset va como `"p"."n"`, sin catálogo
//! delante, y el motor lo pide por `loadTable`: su conducto, su credencial y
//! lo declarado se deciden ahí, dataset a dataset, como siempre. Una View que
//! se llama como un dataset no se sirve como vista: el nombre es la tabla (y
//! su SQL, que nombra al dataset, se leería a sí misma).
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
//! Renombrar, borrar tablas, `register`, métricas, el
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
fn tabla_de(p: &Peticion, base: Option<&str>, seg: &[&str]) -> Option<String> {
    match seg {
        ["namespaces", ns, "tables", t] => Some(nombre_de(base, ns, t)),
        // Crear una tabla: el nombre va en el cuerpo.
        ["namespaces", ns, "tables"] if p.metodo == "POST" => ore_core::parse::parse(&p.cuerpo)
            .ok()
            .and_then(|n| {
                n.get("name")
                    .and_then(|(_, v)| v.as_str().map(String::from))
            })
            .map(|t| nombre_de(base, ns, &t)),
        _ => None,
    }
}

/// **El nombre del árbol de lo que una ruta nombra** (0038 P4), en su forma
/// corta. Sin `prefix`, el namespace es la base y lo que hay es de `default`
/// —lo de siempre—; con `prefix` (el `warehouse` que el cliente pidió), el
/// `prefix` es la base y el namespace su schema, como en Unity Catalog.
fn nombre_de(base: Option<&str>, ns: &str, t: &str) -> String {
    match base {
        None => format!("{ns}.{t}"),
        Some(b) => ore_core::normalize::corto(b, ns, t),
    }
}

/// Los schemas de una base: `default` y los que declara (v1alpha13, `kind:
/// Schema`), ordenados.
fn schemas_de(pkg: &ore_core::link::Package, base: &str) -> Vec<String> {
    let mut v: Vec<String> = pkg
        .docs
        .iter()
        .filter(|d| {
            d.kind == ore_core::document::Kind::Schema
                && d.meta("namespace").and_then(|x| x.as_str()) == Some(base)
        })
        .filter_map(|d| d.meta("name").and_then(|x| x.as_str()).map(String::from))
        .collect();
    v.push(ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
    v.sort();
    v.dedup();
    v
}

/// ¿Es `nombre` (forma corta) de este namespace? Sin base: de la base `ns`, en
/// `default`. Con base: de esa base y del schema `ns`.
fn del_namespace(base: Option<&str>, ns: &str, nombre: &str) -> Option<String> {
    let (b, s, t) = ore_core::punteros::partes(nombre)?;
    let es = match base {
        None => b == ns && s == ore_core::normalize::SCHEMA_POR_DEFECTO,
        Some(base) => b == base && s == ns,
    };
    es.then(|| t.to_string())
}

/// El cuerpo de un `commitTransaction` con la base delante del namespace de
/// cada `identifier` (`["espana"]` → `["ventas", "espana"]`).
fn con_la_base(n: &ore_core::parse::Node, base: &str) -> String {
    let mut j = Json::de_node(n);
    if let Json::Obj(m) = &mut j
        && let Some(Json::Arr(cambios)) = m.get_mut("table-changes")
    {
        for c in cambios {
            if let Json::Obj(c) = c
                && let Some(Json::Obj(id)) = c.get_mut("identifier")
                && let Some(Json::Arr(ns)) = id.get_mut("namespace")
            {
                ns.insert(0, Json::s(base));
            }
        }
    }
    j.jcs()
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
        // **El `prefix`** (0038 P4): lo que el cliente recibió de `config`
        // cuando pidió un `warehouse` —la base—. Lo que no es una de las rutas
        // de la spec sin él (`config`, `namespaces`, `transactions`) es él.
        let (base, seg) = match seg {
            [b, resto @ ..] if !["config", "namespaces", "transactions"].contains(b) => {
                if let Err(r) = ns_valido(b) {
                    return r;
                }
                (Some(b.to_string()), resto)
            }
            _ => (None, seg),
        };
        let base = base.as_deref();
        // El informe de un escaneo: Spark lo manda tras cada lectura. No se
        // guarda, y un 404 aquí sólo ensucia su registro.
        if p.metodo == "POST" && matches!(seg, ["namespaces", _, "tables", _, "metrics"]) {
            return Respuesta::sin_contenido();
        }
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
        // **El techo de la clase** (0036 ⑤): un repositorio de una clase que no
        // escribe datos —`analytics`, `functions`— no escribe **aunque su
        // código lo declare**. Va aquí, donde ya se decide quién escribe, y no
        // en el SDK: lo que se comprueba en el cliente se rodea pidiendo a
        // pelo (medido en W3.7 ⑤). Y **sólo quita**: una clase nunca concede.
        if p.metodo != "GET"
            && p.metodo != "HEAD"
            && let Some(id) = p.cabeceras.get(crate::puestos::PUESTO)
            && let Some(c) = self.clase_de(id.trim())
            && !c.escribe
        {
            return con_forma(
                Respuesta::error(
                    403,
                    format!(
                        "este puesto vive en un repositorio `{}`, y esa clase no escribe datos: para escribir, un repositorio `transforms` o `models`",
                        c.id
                    ),
                ),
                false,
            );
        }
        // **Lo declarado manda** (0031 W3.7 gobierno ⑤): mientras un transform
        // corre en este puesto, el catálogo sólo carga, esboza o confirma su
        // `output`. Lo demás —cargar otra tabla para escribirla, crearla,
        // commitear sobre ella— es 403 con lo declarado, en el servidor y no
        // en el SDK. Leer va por `datos`, que lo acota igual.
        if let Some(id) = p.cabeceras.get(crate::puestos::PUESTO)
            && let Some(t) = self.transform_de(id.trim())
            && let Some(tabla) = tabla_de(p, base, seg)
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
        // Desde un puesto —o un agente sin él—: lo que se lee pasa por el
        // conducto (0031 W3.7 gobierno ②), sea una tabla o una View.
        let desde_puesto = p
            .cabeceras
            .get(crate::puestos::PUESTO)
            .is_some_and(|s| !s.trim().is_empty())
            || crate::puestos::es_agente(sujeto);
        match (p.metodo.as_str(), seg) {
            ("GET", ["config"]) => {
                // `?warehouse=<base>`: la base es el `prefix` de lo que siga
                // (medido: PyIceberg y DuckDB lo usan, `medida-v1-como-unity.py`).
                let overrides = match p.consulta.get("warehouse") {
                    None => Json::obj([]),
                    Some(w) => {
                        let w = w.clone();
                        let hay = self.leyendo_en(rama, |raiz| {
                            if paquetes(raiz).contains(&w) {
                                Respuesta::ok(Json::obj([]))
                            } else {
                                error(
                                    404,
                                    "NoSuchWarehouseException",
                                    format!("no hay ninguna base `{w}`"),
                                )
                            }
                        });
                        if hay.codigo != 200 {
                            return con_forma(hay, false);
                        }
                        Json::obj([("prefix", Json::s(w))])
                    }
                };
                Respuesta::ok(Json::obj([
                    ("defaults", Json::obj([])),
                    ("overrides", overrides),
                    (
                        "endpoints",
                        Json::Arr(ENDPOINTS.iter().map(|e| Json::s(*e)).collect()),
                    ),
                ]))
            }
            ("GET", ["namespaces"]) => con_forma(
                self.leyendo_en(rama, |raiz| {
                    // Sin base, las bases; con ella, sus schemas.
                    let nombres = match base {
                        None => paquetes(raiz),
                        Some(b) if !paquetes(raiz).iter().any(|p| p == b) => {
                            return error(
                                404,
                                "NoSuchWarehouseException",
                                format!("no hay ninguna base `{b}`"),
                            );
                        }
                        Some(b) => schemas_de(&ore_core::validate::cargar_paquete(raiz).0, b),
                    };
                    Respuesta::ok(Json::obj([(
                        "namespaces",
                        Json::Arr(
                            nombres
                                .into_iter()
                                .map(|n| Json::Arr(vec![Json::s(n)]))
                                .collect(),
                        ),
                    )]))
                }),
                false,
            ),
            ("GET" | "HEAD", ["namespaces", ns]) => {
                if let Err(r) = ns_valido(ns) {
                    return r;
                }
                let ns = ns.to_string();
                con_forma(
                    self.leyendo_en(rama, move |raiz| {
                        let existe = match base {
                            None => paquetes(raiz).contains(&ns),
                            Some(b) => schemas_de(&ore_core::validate::cargar_paquete(raiz).0, b)
                                .contains(&ns),
                        };
                        if existe {
                            Respuesta::ok(Json::obj([
                                ("namespace", Json::Arr(vec![Json::s(&ns)])),
                                ("properties", Json::obj([])),
                            ]))
                        } else {
                            error(
                                404,
                                "NoSuchNamespaceException",
                                match base {
                                    None => format!("no hay ningún paquete `{ns}`"),
                                    Some(b) => {
                                        format!("no hay ningún schema `{ns}` en la base `{b}`")
                                    }
                                },
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
                    self.leyendo_en(rama, move |raiz| {
                        // Lo que no está es 404, no una lista vacía (medido:
                        // `namespaces/espana/tables` daba 200 sin nada).
                        let existe = match base {
                            None => paquetes(raiz).contains(&ns),
                            Some(b) => schemas_de(&ore_core::validate::cargar_paquete(raiz).0, b)
                                .contains(&ns),
                        };
                        if !existe {
                            return error(
                                404,
                                "NoSuchNamespaceException",
                                match base {
                                    None => format!("no hay ningún paquete `{ns}`"),
                                    Some(b) => {
                                        format!("no hay ningún schema `{ns}` en la base `{b}`")
                                    }
                                },
                            );
                        }
                        self.tablas(raiz, base, &ns)
                    }),
                    false,
                )
            }
            ("GET" | "HEAD", ["namespaces", ns, "tables", t]) => {
                if let Err(r) = ns_valido(ns).and_then(|_| ns_valido(t)) {
                    return r;
                }
                let nombre = nombre_de(base, ns, t);
                let cabeza = p.metodo == "HEAD";
                let sujeto_s = sujeto.persona.clone();
                // **El conducto de la lectura, también aquí** (0031 W3.7
                // gobierno ②). `loadTable` devuelve dónde están los ficheros y,
                // con `vended-credentials`, con qué leerlos: es una puerta de
                // lectura tanto como `datos`, y hasta ahora no preguntaba.
                // Medido (`medida-el-catalogo-como-resolutor.py` §4): con el
                // conducto en `low`, `datos` era 403 OOS4002 y `loadTable` 200
                // con credencial, y DuckDB leía la columna `high` entera. Desde
                // un puesto —o un agente sin él— decide lo mismo que `datos`,
                // con el mismo código. Y es para leer Y para escribir: DuckDB y
                // `write()` hacen un solo `loadTable`, y la credencial que da
                // lee; no se escribe encima de lo que no se puede leer.
                con_forma(
                    self.leyendo_en(rama, move |raiz| {
                        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
                        // Una View no es una tabla: 404, para que el motor
                        // pruebe `loadView` (Spark sólo lo hace tras un 404).
                        if solo_vista(&pkg, &nombre) {
                            return error(
                                404,
                                "NoSuchTableException",
                                format!(
                                    "`{nombre}` es una View, no una tabla: se lee por `loadView`"
                                ),
                            );
                        }
                        if desde_puesto
                            && let Err(n) = ore_core::flow::lectura_desde_puesto(&pkg, &nombre)
                        {
                            return error(
                                403,
                                "ForbiddenException",
                                format!("{}: {}", n.codigo, n.mensaje),
                            );
                        }
                        // Desde un puesto, lo de otra persona se lee: la
                        // credencial de leer, no el 403 de escribir.
                        let r = self.cargar(raiz, &nombre, prestar, &sujeto_s, desde_puesto);
                        if cabeza && r.codigo == 200 {
                            Respuesta::sin_contenido()
                        } else {
                            r
                        }
                    }),
                    false,
                )
            }
            ("GET", ["namespaces", ns, "views"]) => {
                if let Err(r) = ns_valido(ns) {
                    return r;
                }
                let ns = ns.to_string();
                con_forma(
                    self.leyendo_en(rama, move |raiz| {
                        if !paquetes(raiz).contains(&base.unwrap_or(&ns).to_string()) {
                            return error(
                                404,
                                "NoSuchNamespaceException",
                                format!("no hay ningún paquete `{}`", base.unwrap_or(&ns)),
                            );
                        }
                        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
                        let mut ids: Vec<Json> = pkg
                            .docs
                            .iter()
                            .filter(|d| d.kind == ore_core::document::Kind::View)
                            .filter_map(|d| d.qname())
                            .filter(|q| solo_vista(&pkg, q))
                            .filter_map(|q| {
                                let v = del_namespace(base, &ns, &q)?;
                                Some(Json::obj([
                                    ("name", Json::s(v)),
                                    ("namespace", Json::Arr(vec![Json::s(&ns)])),
                                ]))
                            })
                            .collect();
                        ids.sort_by_key(|j| j.jcs());
                        Respuesta::ok(Json::obj([("identifiers", Json::Arr(ids))]))
                    }),
                    false,
                )
            }
            ("GET" | "HEAD", ["namespaces", ns, "views", v]) => {
                if let Err(r) = ns_valido(ns).and_then(|_| ns_valido(v)) {
                    return r;
                }
                let nombre = nombre_de(base, ns, v);
                let cabeza = p.metodo == "HEAD";
                let ns = ns.to_string();
                con_forma(
                    self.leyendo_en(rama, move |raiz| {
                        self.cargar_vista(raiz, &nombre, base, &ns, desde_puesto, cabeza)
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
                let nombre = nombre_de(base, ns, nombre);
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
                                self.cargar(raiz, &nombre, prestar, &sujeto_s, false)
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
                let nombre = nombre_de(base, ns, t);
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
                            self.cargar(raiz, &nombre, false, "", false)
                        },
                    ),
                    true,
                )
            }
            ("POST", ["transactions", "commit"]) => {
                let Ok(n) = ore_core::parse::parse(&p.cuerpo) else {
                    return error(400, "BadRequestException", "el cuerpo no es JSON");
                };
                if p.cuerpo.trim().is_empty() {
                    return error(400, "BadRequestException", "el cuerpo no es JSON");
                }
                // Con `prefix`, el namespace de cada tabla es su schema: se le
                // pone delante la base, y `ore datasets` lee `[base, schema]`.
                let peticion = match base {
                    None => p.cuerpo.clone(),
                    Some(b) => con_la_base(&n, b),
                };
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

    /// **El `LoadViewResult` de una View** (spec REST de Iceberg, `loadView`):
    /// su SQL —en DuckDB y en Spark— sobre sus datasets por su nombre del
    /// catálogo y su esquema, de
    /// `ore ask --sql --catalogo`. No hay `metadata.json` de la vista en el
    /// bucket: la View vive en el árbol, y lo que se sirve es su versión de
    /// ahora (una, la 1). El `view-uuid` sale del nombre: el mismo siempre.
    fn cargar_vista(
        &self,
        raiz: &Path,
        nombre: &str,
        base: Option<&str>,
        ns: &str,
        desde_puesto: bool,
        cabeza: bool,
    ) -> Respuesta {
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        if !solo_vista(&pkg, nombre) {
            let m = if pkg.dataset(nombre).is_some() {
                format!("`{nombre}` es un dataset: se lee por `loadTable`")
            } else {
                format!("no hay ninguna View `{nombre}`")
            };
            return error(404, "NoSuchViewException", m);
        }
        if desde_puesto && let Err(n) = ore_core::flow::lectura_desde_puesto(&pkg, nombre) {
            return error(
                403,
                "ForbiddenException",
                format!("{}: {}", n.codigo, n.mensaje),
            );
        }
        let mut args: Vec<String> = ["ask", ".", "--vista", nombre, "--sql", "--catalogo"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Con `prefix`, el namespace es el schema: el SQL nombra desde él (0038 P4).
        if let Some(b) = base {
            args.extend(["--base".to_string(), b.to_string()]);
        }
        let s = match mando::correr(&self.binario, raiz, &args) {
            Ok(s) => s,
            Err(e) => return error(500, "InternalServerError", e.to_string()),
        };
        let json = ultima_json(&s.stdout).and_then(|l| ore_core::parse::parse(&l).ok());
        let Some(j) = json.filter(|_| s.bien()) else {
            // Una View virtual (sobre una Table) o una que el traductor no
            // sabe escribir: existe, y se dice por qué no se sirve.
            return error(
                400,
                "BadRequestException",
                format!(
                    "`{nombre}` no se sirve como vista del catálogo: {}",
                    primera_de(&s.stderr)
                ),
            );
        };
        if cabeza {
            return Respuesta::sin_contenido();
        }
        // El namespace, el que el cliente usa (la base sin `prefix`, el
        // schema con él); el lugar, por la forma corta.
        let v = nombre.rsplit('.').next().unwrap_or(nombre);
        let ns = ns.to_string();
        // Una representación por dialecto (DuckDB y Spark): el motor elige la
        // suya (medido: Spark toma `spark` aunque vaya detrás).
        let representaciones = j
            .get("representaciones")
            .map(|(_, r)| Json::de_node(r))
            .unwrap_or(Json::Arr(vec![]));
        let esquema = j
            .get("esquema")
            .map(|(_, e)| Json::de_node(e))
            .unwrap_or(Json::obj([]));
        let ahora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let lugar = format!("ore://arbol/{}", nombre.replace('.', "/"));
        let _ = v;
        let version = Json::obj([
            ("version-id", Json::Int(1)),
            ("timestamp-ms", Json::Int(ahora)),
            ("schema-id", Json::Int(0)),
            ("summary", Json::obj([("operation", Json::s("create"))])),
            ("default-namespace", Json::Arr(vec![Json::s(&ns)])),
            ("representations", representaciones),
        ]);
        Respuesta::ok(Json::obj([
            (
                "metadata-location",
                Json::s(format!("{lugar}/v1.metadata.json")),
            ),
            (
                "metadata",
                Json::obj([
                    ("view-uuid", Json::s(uuid_de(nombre))),
                    ("format-version", Json::Int(1)),
                    ("location", Json::s(lugar)),
                    ("current-version-id", Json::Int(1)),
                    ("versions", Json::Arr(vec![version])),
                    (
                        "version-log",
                        Json::Arr(vec![Json::obj([
                            ("version-id", Json::Int(1)),
                            ("timestamp-ms", Json::Int(ahora)),
                        ])]),
                    ),
                    ("schemas", Json::Arr(vec![esquema])),
                    ("properties", Json::obj([])),
                ]),
            ),
            ("config", Json::obj([])),
        ]))
    }

    /// El `LoadTableResult` de una tabla.
    fn cargar(
        &self,
        raiz: &Path,
        nombre: &str,
        prestar: bool,
        sujeto: &str,
        o_leer: bool,
    ) -> Respuesta {
        let mut args = vec!["datasets", ".", "--cargar", nombre];
        if prestar {
            // La credencial para escribir es de quien escribió: `ore` decide
            // con el sujeto (W3.7 gobierno ④). Con `o_leer` (desde un puesto,
            // tras el conducto) a quien no lo escribió le da la de leer.
            args.extend(["--prestar", "--sujeto", sujeto]);
            if o_leer {
                args.push("--o-leer");
            }
        }
        self.ore_crudo(raiz, &args, false)
    }

    /// Las tablas del lago con puntero, en un namespace (una base sin
    /// `prefix`, un schema de la base con él).
    fn tablas(&self, raiz: &Path, base: Option<&str>, ns: &str) -> Respuesta {
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
            // Lo que es una tabla de Iceberg: escrito o mantenido, con su
            // `metadata.json` (una copia heredada no lo es). Filtraba por una
            // `clase` que ya no existe, y salía vacío (medido con Spark).
            .filter(|d| {
                d.get("metadata_location")
                    .and_then(|(_, v)| v.as_str())
                    .is_some_and(|m| !m.is_empty())
            })
            .filter_map(|d| d.get("nombre").and_then(|(_, v)| v.as_str()))
            .filter_map(|n| del_namespace(base, ns, n))
            .map(|t| {
                Json::obj([
                    ("name", Json::s(t)),
                    ("namespace", Json::Arr(vec![Json::s(ns)])),
                ])
            })
            .collect();
        Respuesta::ok(Json::obj([("identifiers", Json::Arr(ids))]))
    }
}

/// Los paquetes del árbol: los directorios de `packages/` con `package.yaml`.
/// Lo que `/v1` sirve, con la forma de la spec (`GET /v1/config`). Sin la
/// lista, un cliente de la biblioteca de Iceberg supone las rutas de tablas y
/// NO pide vistas (medido con Spark); con ella, sólo pide lo que está.
const ENDPOINTS: &[&str] = &[
    "GET /v1/{prefix}/namespaces",
    "GET /v1/{prefix}/namespaces/{namespace}",
    "HEAD /v1/{prefix}/namespaces/{namespace}",
    "GET /v1/{prefix}/namespaces/{namespace}/tables",
    "POST /v1/{prefix}/namespaces/{namespace}/tables",
    "GET /v1/{prefix}/namespaces/{namespace}/tables/{table}",
    "HEAD /v1/{prefix}/namespaces/{namespace}/tables/{table}",
    "POST /v1/{prefix}/namespaces/{namespace}/tables/{table}",
    "POST /v1/{prefix}/namespaces/{namespace}/tables/{table}/metrics",
    "POST /v1/{prefix}/transactions/commit",
    "GET /v1/{prefix}/namespaces/{namespace}/views",
    "GET /v1/{prefix}/namespaces/{namespace}/views/{view}",
    "HEAD /v1/{prefix}/namespaces/{namespace}/views/{view}",
];

/// Una View que no se llama como un dataset: la que el catálogo sirve como
/// vista. Con el mismo nombre (v1alpha12) el nombre es la tabla.
fn solo_vista(pkg: &ore_core::link::Package, nombre: &str) -> bool {
    pkg.view(nombre).is_some() && pkg.dataset(nombre).is_none()
}

/// Un UUID estable por nombre (la forma de un v5: versión 5, variante RFC).
fn uuid_de(nombre: &str) -> String {
    let h = ore_core::digest::de_bytes(format!("ore-view:{nombre}").as_bytes());
    let x = h.trim_start_matches("sha256:");
    format!(
        "{}-{}-5{}-8{}-{}",
        &x[0..8],
        &x[8..12],
        &x[13..16],
        &x[17..20],
        &x[20..32]
    )
}

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
