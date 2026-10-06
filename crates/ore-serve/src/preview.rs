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
//! | `view` | el lago | lo mismo sobre la copia de la vista SQL; sin copia, **409**: se lee en un puesto (sin motor aquí) |
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
                self.leyendo_en(rama, |raiz| {
                    self.del_lago(raiz, &nombre, desde, limite, snapshot.as_deref())
                })
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
