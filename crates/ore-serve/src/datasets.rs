//! **Los datasets, servidos** (W3.6b, [0031 §10](../../../docs/decisions/0031-el-puesto.md)):
//! la lista, la ficha con la historia de la tabla, y **el swap** del puntero de
//! un dataset del lago por quien no puede empujar al árbol.
//!
//! | ruta | qué | quién decide |
//! |---|---|---|
//! | `GET /datasets` | los punteros de `copias/` y `datasets/`, con su estado | `ore datasets . --json` |
//! | `GET /datasets/{ns}/{n}` | el puntero y los snapshots de la tabla (`ore-store historia`, con la identidad del pod y su `objectViewer`) | `ore datasets . --ficha ns.n --json` |
//! | `POST /datasets/{ns}/{n}/confirmar` | el puntero de una `Table` del lago pasa a `metadata_location`, con `esperado` como compare-and-set semántico; la `Table` nace con `columnas` si no existe; y **el commit lo empuja este proceso** | `ore datasets . --confirmar ns.n …`, y después `git push` |
//!
//! # Las dos caras del 409
//!
//! `confirmar` puede perder dos carreras y las dos se dicen igual, porque para
//! quien escribe son la misma: *«vuelve a leer y escribe sobre lo que hay
//! ahora»*. La **semántica** —el puntero ya no es `esperado`— la decide `ore`
//! (código 75) sin empujar nada; la de **la forja** —dos empujones a la vez, y
//! el que pierde recibe `[remote rejected]`— la decide git, y `escribiendo` la
//! traduce. Medido (`medida-w3-swap.py`): ocho escritores sobre el mismo
//! puntero dan exactamente uno que gana, y los siete reintentando se
//! serializan en siete rondas.
//!
//! # Lo que este proceso NO hace
//!
//! No escribe en el bucket: el `metadata.json` que se apunta lo escribió el
//! puesto con su identidad, y aquí sólo se comprueba que está (`ore datasets`
//! le pide un HEAD a `ore-store`). `ore-serve` tiene `objectViewer` y le basta.

use std::path::Path;

use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;

use crate::mando;
use crate::rutas::{Servidor, token};

impl Servidor {
    /// `GET /datasets`.
    pub(crate) fn datasets(&self, rama: Option<&str>) -> Respuesta {
        self.leyendo_en(rama, |raiz| {
            self.ore_json(raiz, &["datasets".into(), ".".into(), "--json".into()])
        })
    }

    /// `GET /datasets/{ns}[/{schema}]/{n}`.
    pub(crate) fn ficha_del_dataset(
        &self,
        rama: Option<&str>,
        ns: &str,
        schema: &str,
        n: &str,
    ) -> Respuesta {
        if let Err(m) = token(ns).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let nombre = ore_core::normalize::corto(ns, schema, n);
        self.leyendo_en(rama, move |raiz| {
            self.ore_json(
                raiz,
                &[
                    "datasets".into(),
                    ".".into(),
                    "--ficha".into(),
                    nombre,
                    "--json".into(),
                ],
            )
        })
    }

    /// `POST /datasets/{ns}/{n}/confirmar` con
    /// `{metadata_location, esperado?, snapshot?, filas?, columnas?}`.
    pub(crate) fn confirmar_dataset(
        &self,
        sujeto: &Identidad,
        ns: &str,
        schema: &str,
        n: &str,
        cuerpo: &str,
    ) -> Respuesta {
        if let Err(m) = token(ns).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let c = match ore_core::parse::parse(cuerpo) {
            Ok(c) if !cuerpo.trim().is_empty() => c,
            _ => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        let campo = |k: &str| {
            c.get(k)
                .and_then(|(_, v)| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        let Some(ml) = campo("metadata_location") else {
            return Respuesta::error(
                422,
                "falta `metadata_location`: el `metadata.json` que se escribió en el bucket",
            );
        };
        let nombre = ore_core::normalize::corto(ns, schema, n);
        let mut args: Vec<String> = vec![
            "datasets".into(),
            ".".into(),
            "--confirmar".into(),
            nombre.clone(),
            "--json".into(),
            "--metadata-location".into(),
            ml,
            "--sujeto".into(),
            sujeto.persona.clone(),
        ];
        if let Some(e) = campo("esperado") {
            args.push("--esperado".into());
            args.push(e);
        }
        if let Some(s) = campo("snapshot") {
            args.push("--snapshot".into());
            args.push(s);
        }
        if let Some(f) = campo("filas") {
            args.push("--filas".into());
            args.push(f);
        }
        if let Some((_, cols)) = c.get("columnas") {
            // Las columnas viajan como el JSON que llegó: `ore` las analiza.
            args.push("--columnas".into());
            args.push(Json::de_node(cols).jcs());
        }
        self.escribiendo(sujeto, &format!("confirmar dataset `{nombre}`"), |raiz| {
            let s = match mando::correr(&self.binario, raiz, &args) {
                Ok(s) => s,
                Err(e) => return Respuesta::error(500, e.to_string()),
            };
            let cuerpo = s
                .stdout
                .lines()
                .rev()
                .find(|l| l.trim_start().starts_with('{'))
                .and_then(|l| ore_core::parse::parse(l).ok())
                .map(|n| Json::de_node(&n));
            match s.codigo {
                0 => {
                    let nuevo = matches!(&cuerpo, Some(Json::Obj(m)) if m.get("puntero_nuevo") == Some(&Json::Bool(true)));
                    let j = cuerpo.unwrap_or_else(|| Json::obj([("tabla", Json::s(&nombre))]));
                    if nuevo {
                        Respuesta::creado(j)
                    } else {
                        Respuesta::ok(j)
                    }
                }
                75 => {
                    let mut r = Respuesta::error(
                        409,
                        s.stderr
                            .lines()
                            .find_map(|l| l.strip_prefix("error: "))
                            .unwrap_or("el puntero ya no es el esperado")
                            .to_string(),
                    );
                    if let (Json::Obj(m), Some(Json::Obj(c))) = (&mut r.cuerpo, cuerpo) {
                        for (k, v) in c {
                            m.entry(k).or_insert(v);
                        }
                    }
                    r
                }
                64 => Respuesta::error(400, primera_de(&s.stderr)),
                65 => Respuesta::error(422, primera_de(&s.stderr)),
                _ => Respuesta::error(502, primera_de(&s.stderr)),
            }
        })
    }

    /// Corre `ore` y devuelve la última línea JSON de su salida tal cual; lo
    /// que no es 0 es 502 con lo que dijo.
    fn ore_json(&self, raiz: &Path, args: &[String]) -> Respuesta {
        let s = match mando::correr(&self.binario, raiz, args) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(500, e.to_string()),
        };
        if !s.bien() {
            return Respuesta::error(
                if s.codigo == 65 { 404 } else { 502 },
                primera_de(&s.stderr),
            );
        }
        match s
            .stdout
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .and_then(|l| ore_core::parse::parse(l).ok())
        {
            Some(n) => Respuesta::ok(Json::de_node(&n)),
            None => Respuesta::error(502, "`ore datasets` no devolvió JSON"),
        }
    }
}

fn primera_de(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start_matches("error: ").to_string())
        .unwrap_or_else(|| "falló sin decir por qué".into())
}
