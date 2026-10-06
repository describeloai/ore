//! **El preview de un activo del catálogo** — `GET /preview/{kind}/{b}/{s}/{n}`.
//!
//! Lo que `select * from b.s.n limit L offset D` daría en el editor SQL, en
//! la misma rama (`x-ore-rama`), **sin abrir un puesto**: la consola lo pide
//! al enseñar un activo y lo pagina con `desde`/`limite`, sin cargar la tabla
//! de golpe. La forma es la de una salida de tabla del editor —columnas con
//! su escalar OOS en orden, filas de texto, un nulo es la columna que falta—,
//! para que se pinte con la misma rejilla.
//!
//! | kind | de dónde | qué se lee |
//! |---|---|---|
//! | `dataset` | el lago | `ore datasets --muestra` → `ore-store muestra`: los ficheros anteriores a `desde` ni se abren, y del que toca sólo sus páginas (índice de páginas, por rangos) |
//! | `view` | el lago, o el motor | lo mismo sobre la copia de la vista SQL; **sin copia, `ore-motor`** (0057 B4·3·2): su `select` con las fuentes resueltas aquí —datasets con su credencial, lo leído en vivo de la pasarela— y DuckDB allí. Sin motor en la celda, 409 |
//! | `table`, `objecttable` | el origen, en vivo | `POST /federation/read` con `limit = desde + limite` —la pasarela no sabe `offset`—, el flujo Arrow a filas por `ore-store arrow-a-filas` |
//!
//! La página de un dataset dice qué `snapshot` leyó; la consola pide las
//! siguientes sobre el mismo y no se mueven aunque alguien escriba entre medias.
//! Una `Table` no tiene snapshot: es el origen tal como está, como en el editor.
//!
//! El acceso es el de las demás lecturas: ser de la organización (`quien`), el
//! conducto en vivo y el evento `federation:read` los pone `leer_federado`, y
//! el lago es nuestra copia (`ore-store` con la identidad del pod).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta, Salida};
use ore_entrada::identidad::Identidad;

use crate::rutas::{Servidor, token};

const LIMITE_POR_DEFECTO: u64 = 100;
/// El de `ore-store` (`muestra::LIMITE_MAXIMO`): una página para mirar.
const LIMITE_MAXIMO: u64 = 1_000;
/// Lo que se junta de la pasarela antes de pasarlo a filas: el presupuesto
/// de una lectura en vivo (64 MB) y nada más.
const BYTES_MAXIMOS: u64 = 64 << 20;

/// Los kinds que tienen preview de filas. Una colección de medios se enseña
/// por `/media/{b}/{s}/{c}/items` (su relación: ítem, ruta, huella, tamaño…).
pub(crate) const KINDS: [&str; 4] = ["dataset", "view", "table", "objecttable"];

/// `desde`, `limite` y `snapshot` de la cadena de consulta, comprobados.
fn pagina(p: &Peticion) -> Result<(u64, u64, Option<String>), String> {
    let numero = |k: &str| -> Result<Option<u64>, String> {
        p.consulta
            .get(k)
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|_| format!("`{k}` tiene que ser un entero positivo"))
            })
            .transpose()
    };
    let desde = numero("desde")?.unwrap_or(0);
    let limite = numero("limite")?.unwrap_or(LIMITE_POR_DEFECTO);
    if limite == 0 || limite > LIMITE_MAXIMO {
        return Err(format!("`limite` va de 1 a {LIMITE_MAXIMO}"));
    }
    let snapshot = match p.consulta.get("snapshot") {
        Some(s) if s.parse::<i64>().is_ok() => Some(s.clone()),
        Some(_) => return Err("`snapshot` es el id de un snapshot".into()),
        None => None,
    };
    Ok((desde, limite, snapshot))
}

/// La consulta que este preview es: lo que la consola enseña y abre en el
/// editor tal cual.
fn sql(b: &str, s: &str, n: &str, desde: u64, limite: u64) -> String {
    let mut q = format!("select * from {b}.{s}.{n} limit {limite}");
    if desde > 0 {
        q.push_str(&format!(" offset {desde}"));
    }
    q
}

impl Servidor {
    /// **`GET /preview/{kind}/{b}/{s}/{n}?desde=&limite=&snapshot=`**.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preview(
        &self,
        rama: Option<&str>,
        p: &Peticion,
        sujeto: &Identidad,
        kind: &str,
        b: &str,
        s: &str,
        n: &str,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(s)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        if !KINDS.contains(&kind) {
            return Respuesta::error(
                404,
                format!(
                    "`{kind}` no tiene preview de filas: {} (una colección, por `/media/…/items`)",
                    KINDS.join(", ")
                ),
            );
        }
        let (desde, limite, snapshot) = match pagina(p) {
            Ok(x) => x,
            Err(m) => return Respuesta::error(422, m),
        };
        let consulta = sql(b, s, n, desde, limite);
        let r = match kind {
            "dataset" | "view" => {
                let nombre = ore_core::normalize::corto(b, s, n);
                let r = self.leyendo_en(rama, |raiz| {
                    self.del_lago(raiz, &nombre, desde, limite, snapshot.as_deref())
                });
                // ⭐ 0057 B4·3·2: una vista sin copia —foreign view, o sobre
                //   datasets— se calcula en `ore-motor`.
                if kind == "view" && r.codigo == 409 && sin_copia(&r) {
                    match motor() {
                        Some(m) => self.por_el_motor(&m, rama, p, sujeto, &consulta, limite),
                        None => r,
                    }
                } else {
                    r
                }
            }
            _ => self.en_vivo(p, sujeto, b, s, n, desde, limite),
        };
        con_consulta(r, &consulta)
    }

    /// Un dataset, o la copia de una vista SQL: `ore datasets --muestra`.
    fn del_lago(
        &self,
        raiz: &Path,
        nombre: &str,
        desde: u64,
        limite: u64,
        snapshot: Option<&str>,
    ) -> Respuesta {
        let mut args: Vec<String> = vec![
            "datasets".into(),
            ".".into(),
            "--muestra".into(),
            nombre.into(),
            "--desde".into(),
            desde.to_string(),
            "--limite".into(),
            limite.to_string(),
            "--json".into(),
        ];
        if let Some(s) = snapshot {
            args.push("--snapshot".into());
            args.push(s.into());
        }
        let s = match crate::mando::correr(&self.binario, raiz, &args) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(500, e.to_string()),
        };
        let motivo = || {
            s.stderr
                .lines()
                .find_map(|l| l.strip_prefix("error: "))
                .unwrap_or("`ore datasets --muestra` falló")
                .to_string()
        };
        match s.codigo {
            0 => match s
                .stdout
                .lines()
                .rev()
                .find(|l| l.trim_start().starts_with('{'))
                .and_then(|l| ore_core::parse::parse(l).ok())
            {
                Some(n) => {
                    let mut j = Json::de_node(&n);
                    if let Json::Obj(m) = &mut j {
                        m.insert("origen".into(), Json::s("lago"));
                    }
                    Respuesta::ok(j)
                }
                None => Respuesta::error(502, "`ore datasets --muestra` no devolvió JSON"),
            },
            65 => Respuesta::error(404, motivo()),
            // Una vista viva, sin copia: sus filas las calcula un motor, y el
            // de la plataforma es el puesto.
            70 => Respuesta {
                codigo: 409,
                cuerpo: Json::obj([
                    ("error", Json::s(motivo())),
                    ("codigo", Json::s("preview/se-lee-en-un-puesto")),
                ]),
            },
            _ => Respuesta::error(502, motivo()),
        }
    }

    /// Una `Table` o un `ObjectTable`: la lectura en vivo de siempre
    /// (`leer_federado`, con su conducto, su presupuesto y su evento), cortada
    /// en `desde + limite`, y sus filas desde `desde`.
    #[allow(clippy::too_many_arguments)]
    fn en_vivo(
        &self,
        p: &Peticion,
        sujeto: &Identidad,
        b: &str,
        s: &str,
        n: &str,
        desde: u64,
        limite: u64,
    ) -> Respuesta {
        let lectura = Peticion {
            metodo: "POST".into(),
            ruta: "/federation/read".into(),
            cabeceras: p.cabeceras.clone(),
            cuerpo: Json::obj([
                ("tabla", Json::s(format!("{b}.{s}.{n}"))),
                ("limit", Json::s((desde + limite).to_string())),
            ])
            .jcs(),
            consulta: Default::default(),
        };
        let mut bytes = match self.leer_federado(&lectura, sujeto) {
            Salida::Bytes(b) => b,
            Salida::Una(r) => return r,
            Salida::Flujo(_) => return Respuesta::error(502, "la lectura en vivo no dio bytes"),
        };
        let mut arrow = Vec::new();
        if let Err(e) = (&mut bytes.lector)
            .take(BYTES_MAXIMOS + 1)
            .read_to_end(&mut arrow)
        {
            return Respuesta::error(502, format!("la lectura en vivo se cortó: {e}"));
        }
        // Soltarlo anota la lectura (`federation:read`) con cómo acabó.
        drop(bytes);
        if arrow.len() as u64 > BYTES_MAXIMOS {
            return Respuesta::error(
                413,
                format!(
                    "{} filas de `{b}.{s}.{n}` pasan de {} MB: pide una página más cerca del principio",
                    desde + limite,
                    BYTES_MAXIMOS >> 20
                ),
            );
        }
        let peticion = Json::obj([
            ("desde", Json::s(desde.to_string())),
            ("limite", Json::s(limite.to_string())),
        ])
        .jcs();
        let salida = match a_filas(&self.binario, &peticion, &arrow) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(502, e),
        };
        match ore_core::parse::parse(salida.trim()) {
            Ok(nodo) => {
                let mut j = Json::de_node(&nodo);
                if let Json::Obj(m) = &mut j {
                    m.insert("origen".into(), Json::s("en-vivo"));
                    m.insert("de".into(), Json::s(format!("{b}.{s}.{n}")));
                }
                Respuesta::ok(j)
            }
            Err(_) => Respuesta::error(502, "`ore-store arrow-a-filas` no devolvió JSON"),
        }
    }
}

/// Dónde está `ore-motor` en esta celda (`ORE_MOTOR`, `host:puerto`).
fn motor() -> Option<String> {
    std::env::var("ORE_MOTOR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Si el 409 es el de una vista sin copia (`preview/se-lee-en-un-puesto`).
fn sin_copia(r: &Respuesta) -> bool {
    matches!(&r.cuerpo, Json::Obj(m) if matches!(m.get("codigo"), Some(Json::Str(c)) if c == "preview/se-lee-en-un-puesto"))
}

impl Servidor {
    /// ⭐ 0057 B4·3·2 · **Una consulta, calculada en `ore-motor`**. Aquí se
    /// decide todo, con la identidad y la rama de quien pregunta: los nombres
    /// y lo que cada uno es (`fuentes_de_sql`, lo mismo que un puesto), y lo
    /// que se lee en vivo, que se trae de la pasarela por `leer_federado` (su
    /// gobierno, su presupuesto, su huella). El motor sólo calcula: no habla
    /// con nadie más.
    pub(crate) fn por_el_motor(
        &self,
        motor: &str,
        rama: Option<&str>,
        p: &Peticion,
        sujeto: &Identidad,
        consulta: &str,
        limite: u64,
    ) -> Respuesta {
        let rama_s = rama.map(String::from);
        let r = self.fuentes_de_sql(rama_s.clone(), consulta, &|n| {
            self.datos_en(rama_s.clone(), n)
        });
        if r.codigo != 200 {
            return r;
        }
        let fuentes = match &r.cuerpo {
            Json::Obj(m) => match m.get("fuentes") {
                Some(Json::Obj(f)) => f.clone(),
                _ => Default::default(),
            },
            _ => Default::default(),
        };
        // Lo que se lee en vivo, una vez por tabla.
        let mut vivas: std::collections::BTreeMap<String, Json> = Default::default();
        for v in fuentes.values() {
            let Json::Obj(m) = v else { continue };
            let Some(Json::Obj(l)) = m.get("federada") else {
                continue;
            };
            let Some(Json::Str(tabla)) = l.get("tabla") else {
                continue;
            };
            if vivas.contains_key(tabla) {
                continue;
            }
            match self.arrow_en_vivo(p, sujeto, l) {
                Ok(bytes) => {
                    vivas.insert(tabla.clone(), Json::s(base64(&bytes)));
                }
                Err(r) => return r,
            }
        }
        let cuerpo = Json::obj([
            ("texto", Json::s(consulta)),
            ("fuentes", Json::Obj(fuentes)),
            ("vivas", Json::Obj(vivas)),
            ("limite", Json::Int(limite as i64)),
        ])
        .jcs();
        let arrow = match al_motor(motor, &cuerpo) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let peticion = Json::obj([
            ("desde", Json::s("0")),
            ("limite", Json::s(limite.to_string())),
        ])
        .jcs();
        let salida = match a_filas(&self.binario, &peticion, &arrow) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(502, e),
        };
        match ore_core::parse::parse(salida.trim()) {
            Ok(nodo) => {
                let mut j = Json::de_node(&nodo);
                if let Json::Obj(m) = &mut j {
                    m.insert("origen".into(), Json::s("motor"));
                }
                Respuesta::ok(j)
            }
            Err(_) => Respuesta::error(502, "`ore-store arrow-a-filas` no devolvió JSON"),
        }
    }

    /// Una lectura en vivo ya repartida (`federada`), en Arrow, por
    /// `leer_federado` en nombre de quien pregunta.
    fn arrow_en_vivo(
        &self,
        p: &Peticion,
        sujeto: &Identidad,
        l: &std::collections::BTreeMap<String, Json>,
    ) -> Result<Vec<u8>, Respuesta> {
        let mut cuerpo: Vec<(&str, Json)> = vec![(
            "tabla",
            l.get("tabla")
                .cloned()
                .unwrap_or(Json::Crudo("null".into())),
        )];
        if let Some(c) = l.get("columnas") {
            cuerpo.push(("columnas", c.clone()));
        }
        if let Some(f) = l.get("empujados") {
            cuerpo.push(("filtros", f.clone()));
        }
        if let Some(x) = l.get("limit")
            && !matches!(x, Json::Crudo(c) if c == "null")
        {
            cuerpo.push(("limit", x.clone()));
        }
        if let Some(Json::Arr(os)) = l.get("orderBy")
            && !os.is_empty()
        {
            let orden = os
                .iter()
                .filter_map(|o| match o {
                    Json::Obj(m) => Some(Json::obj([
                        (
                            "columna",
                            m.get("columna")
                                .cloned()
                                .unwrap_or(Json::Crudo("null".into())),
                        ),
                        (
                            "direccion",
                            Json::s(if matches!(m.get("desc"), Some(Json::Bool(true))) {
                                "desc"
                            } else {
                                "asc"
                            }),
                        ),
                    ])),
                    _ => None,
                })
                .collect();
            cuerpo.push(("orderBy", Json::Arr(orden)));
        }
        let lectura = Peticion {
            metodo: "POST".into(),
            ruta: "/federation/read".into(),
            cabeceras: p.cabeceras.clone(),
            cuerpo: Json::obj(cuerpo).jcs(),
            consulta: Default::default(),
        };
        let mut bytes = match self.leer_federado(&lectura, sujeto) {
            Salida::Bytes(b) => b,
            Salida::Una(r) => return Err(r),
            Salida::Flujo(_) => {
                return Err(Respuesta::error(502, "la lectura en vivo no dio bytes"));
            }
        };
        let mut arrow = Vec::new();
        if let Err(e) = (&mut bytes.lector)
            .take(BYTES_MAXIMOS + 1)
            .read_to_end(&mut arrow)
        {
            return Err(Respuesta::error(
                502,
                format!("la lectura en vivo se cortó: {e}"),
            ));
        }
        drop(bytes);
        if arrow.len() as u64 > BYTES_MAXIMOS {
            return Err(Respuesta::error(
                413,
                format!(
                    "lo leído en vivo pasa de {} MB: la vista pide demasiado del origen",
                    BYTES_MAXIMOS >> 20
                ),
            ));
        }
        Ok(arrow)
    }
}

/// `POST /v1/calcular` a `ore-motor`, por HTTP plano dentro de la celda: el
/// Arrow del resultado, o su 422 tal cual.
fn al_motor(motor: &str, cuerpo: &str) -> Result<Vec<u8>, Respuesta> {
    use std::net::ToSocketAddrs as _;
    let dir = motor
        .to_socket_addrs()
        .ok()
        .and_then(|mut d| d.next())
        .ok_or_else(|| {
            Respuesta::error(502, format!("`ore-motor` (`{motor}`) no tiene dirección"))
        })?;
    let mut s = std::net::TcpStream::connect_timeout(&dir, std::time::Duration::from_secs(5))
        .map_err(|e| Respuesta::error(502, format!("`ore-motor` no contesta: {e}")))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(120)))
        .ok();
    let req = format!(
        "POST /v1/calcular HTTP/1.1\r\nhost: motor\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
        cuerpo.len()
    );
    s.write_all(req.as_bytes())
        .map_err(|e| Respuesta::error(502, format!("`ore-motor`: {e}")))?;
    let mut todo = Vec::new();
    s.read_to_end(&mut todo)
        .map_err(|e| Respuesta::error(502, format!("`ore-motor`: {e}")))?;
    let fin = todo
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| Respuesta::error(502, "`ore-motor` contestó algo que no es HTTP"))?;
    let cabeza = String::from_utf8_lossy(&todo[..fin]).to_string();
    let cuerpo = todo[fin + 4..].to_vec();
    let codigo: u16 = cabeza
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(502);
    if codigo == 200 {
        return Ok(cuerpo);
    }
    let j = ore_core::parse::parse(String::from_utf8_lossy(&cuerpo).trim())
        .map(|n| Json::de_node(&n))
        .unwrap_or_else(|_| {
            Json::obj([("error", Json::s(String::from_utf8_lossy(&cuerpo).trim()))])
        });
    Err(Respuesta {
        codigo: if codigo == 422 { 422 } else { 502 },
        cuerpo: j,
    })
}

/// Base64 (el alfabeto estándar, con relleno): el Arrow en vivo, en el JSON
/// que va al motor.
fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for t in b.chunks(3) {
        let n = (u32::from(t[0]) << 16)
            | (u32::from(*t.get(1).unwrap_or(&0)) << 8)
            | u32::from(*t.get(2).unwrap_or(&0));
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if t.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if t.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `sql` en una respuesta buena: la consulta que el preview es.
fn con_consulta(mut r: Respuesta, consulta: &str) -> Respuesta {
    if r.codigo == 200
        && let Json::Obj(m) = &mut r.cuerpo
    {
        m.insert("sql".into(), Json::s(consulta));
    }
    r
}

/// El `ore-store` de esta imagen, junto a `ore`: el mismo que `ore` elige.
fn programa_del_almacen(ore: &Path) -> PathBuf {
    let nombre = match std::env::var("ORE_STORE").as_deref() {
        Ok("gcs") => "ore-store-gcs",
        _ => "ore-store-r2",
    };
    match ore.parent() {
        Some(d) if d.join(nombre).exists() => d.join(nombre),
        _ => PathBuf::from(nombre),
    }
}

/// `ore-store arrow-a-filas`: la petición en la primera línea y el flujo Arrow
/// detrás, como `escribir`.
fn a_filas(ore: &Path, peticion: &str, arrow: &[u8]) -> Result<String, String> {
    let programa = programa_del_almacen(ore);
    let mut hijo = Command::new(&programa)
        .arg("arrow-a-filas")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("no se pudo ejecutar `{}`: {e}", programa.display()))?;
    {
        let mut entrada = hijo.stdin.take().ok_or("no se pudo escribir en el hijo")?;
        entrada
            .write_all(peticion.as_bytes())
            .and_then(|_| entrada.write_all(b"\n"))
            .and_then(|_| entrada.write_all(arrow))
            .map_err(|e| format!("no se pudo escribir en `{}`: {e}", programa.display()))?;
    }
    let s = hijo
        .wait_with_output()
        .map_err(|e| format!("`{}` no terminó: {e}", programa.display()))?;
    if !s.status.success() {
        let e = String::from_utf8_lossy(&s.stderr);
        return Err(e
            .lines()
            .find_map(|l| l.strip_prefix("error: "))
            .unwrap_or("`ore-store arrow-a-filas` falló")
            .to_string());
    }
    Ok(String::from_utf8_lossy(&s.stdout).into_owned())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::collections::BTreeMap;

    fn con(q: &[(&str, &str)]) -> Peticion {
        Peticion {
            metodo: "GET".into(),
            ruta: "/preview/dataset/b/s/n".into(),
            cabeceras: BTreeMap::new(),
            cuerpo: String::new(),
            consulta: q
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn la_pagina_por_defecto_y_sus_topes() {
        assert_eq!(pagina(&con(&[])).unwrap(), (0, 100, None));
        assert_eq!(
            pagina(&con(&[
                ("desde", "200"),
                ("limite", "50"),
                ("snapshot", "-7")
            ]))
            .unwrap(),
            (200, 50, Some("-7".into()))
        );
        assert!(pagina(&con(&[("limite", "0")])).is_err());
        assert!(pagina(&con(&[("limite", "1001")])).is_err());
        assert!(pagina(&con(&[("desde", "x")])).is_err());
        assert!(pagina(&con(&[("snapshot", "abc")])).is_err());
    }

    #[test]
    fn la_consulta_que_el_preview_es() {
        assert_eq!(sql("b", "s", "n", 0, 100), "select * from b.s.n limit 100");
        assert_eq!(
            sql("b", "s", "n", 300, 100),
            "select * from b.s.n limit 100 offset 300"
        );
    }
}
