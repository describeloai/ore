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
    ///
    /// ⭐ Con `x-ore-rama`, el puntero se mueve **en esa rama** (0044 C D2), como
    ///   en `/v1`: antes iba a `main` dijera lo que dijera la cabecera (medido,
    ///   D0 M5). Sin ella, lo de siempre.
    pub(crate) fn confirmar_dataset(
        &self,
        rama: Option<&str>,
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
        let mensaje = format!("confirmar dataset `{nombre}`");
        let hacer = |raiz: &std::path::Path| {
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
        };
        match rama {
            None => self.escribiendo(sujeto, &mensaje, hacer),
            Some(_) => self.escribiendo_en(rama, sujeto, &mensaje, hacer),
        }
    }

    /// `GET /colecciones` (0046 E8·1d): cada `MediaCollection` del árbol, con
    /// su forma, su origen y el estado de su puntero.
    pub(crate) fn colecciones(&self, rama: Option<&str>) -> Respuesta {
        self.leyendo_en(rama, |raiz| {
            self.ore_json(raiz, &["collections".into(), ".".into(), "--json".into()])
        })
    }

    /// `GET /colecciones/{b}[/{schema}]/{n}`: la colección y la historia de sus
    /// transacciones.
    pub(crate) fn ficha_de_la_coleccion(
        &self,
        rama: Option<&str>,
        b: &str,
        schema: &str,
        n: &str,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let nombre = ore_core::normalize::corto(b, schema, n);
        self.leyendo_en(rama, move |raiz| {
            self.ore_json(
                raiz,
                &[
                    "collections".into(),
                    ".".into(),
                    "--ficha".into(),
                    nombre,
                    "--json".into(),
                ],
            )
        })
    }

    /// `GET /colecciones/{b}/{s}/{n}/items?estado=&desde=&limite=`: sus ítems,
    /// por estado (`actual` por defecto) y en páginas de hasta mil.
    pub(crate) fn items_de_la_coleccion(
        &self,
        rama: Option<&str>,
        b: &str,
        schema: &str,
        n: &str,
        consulta: &std::collections::BTreeMap<String, String>,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let estado = consulta
            .get("estado")
            .map(String::as_str)
            .unwrap_or("actual");
        if !["actual", "retirado", "perdido", "todos"].contains(&estado) {
            return Respuesta::error(422, "`estado` es `actual`, `retirado`, `perdido` o `todos`");
        }
        let numero = |k: &str, defecto: usize| -> Result<usize, Respuesta> {
            match consulta.get(k) {
                None => Ok(defecto),
                Some(v) => v
                    .parse::<usize>()
                    .map_err(|_| Respuesta::error(422, format!("`{k}` es un número"))),
            }
        };
        let (desde, limite) = match (numero("desde", 0), numero("limite", 100)) {
            (Ok(d), Ok(l)) => (d, l.clamp(1, 1000)),
            (Err(r), _) | (_, Err(r)) => return r,
        };
        let nombre = ore_core::normalize::corto(b, schema, n);
        let estado = estado.to_string();
        self.leyendo_en(rama, move |raiz| {
            self.ore_json(
                raiz,
                &[
                    "collections".into(),
                    ".".into(),
                    "--items".into(),
                    nombre,
                    "--estado".into(),
                    estado,
                    "--desde".into(),
                    desde.to_string(),
                    "--limite".into(),
                    limite.to_string(),
                    "--json".into(),
                ],
            )
        })
    }

    /// **Servir** (0046 E9·2): de cada huella, su ítem y una URL firmada a sus
    /// bytes, que vive 5 minutos (`ttl`, de 30 s a 1 h). `ore-serve` no pasa
    /// bytes: autoriza y firma, y el lago sirve —con rangos, a la velocidad
    /// del lago (103 MB/s medidos en el clúster)—. La URL es un portador: se
    /// anota **quién la pidió, de qué, y cuánto vive** en la actividad de la
    /// organización (`coleccion:servir`), porque el lago no sabe quién lee.
    ///
    /// Quién puede: quien lee los ítems de la colección —la pertenencia a la
    /// organización (0047 A9′)—. La decisión por recurso (`coleccion:leer`
    /// sobre esta colección) llega con 0047 A8: el motor de `ore-iam` sólo
    /// decide sobre la organización todavía.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn servir_items(
        &self,
        rama: Option<&str>,
        p: &ore_entrada::http::Peticion,
        b: &str,
        schema: &str,
        n: &str,
        huellas: &[String],
        ttl: Option<u64>,
        una: bool,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let nombre = ore_core::normalize::corto(b, schema, n);
        let mut args: Vec<String> = vec![
            "collections".into(),
            ".".into(),
            "--servir".into(),
            nombre,
            "--json".into(),
        ];
        for h in huellas {
            args.push(format!("--huella={h}"));
        }
        if let Some(t) = ttl {
            args.push("--ttl".into());
            args.push(t.to_string());
        }
        let r = self.leyendo_en(rama, |raiz| {
            let s = match mando::correr(&self.binario, raiz, &args) {
                Ok(s) => s,
                Err(e) => return Respuesta::error(500, e.to_string()),
            };
            let codigo = match s.codigo {
                0 => 200,
                64 => 422,
                65 => 404,
                66 => 409,
                _ => 502,
            };
            if codigo != 200 {
                return Respuesta::error(codigo, primera_de(&s.stderr));
            }
            match s
                .stdout
                .lines()
                .rev()
                .find(|l| l.trim_start().starts_with('{'))
                .and_then(|l| ore_core::parse::parse(l).ok())
            {
                Some(n) => Respuesta::ok(Json::de_node(&n)),
                None => Respuesta::error(502, "`ore collections --servir` no devolvió JSON"),
            }
        });
        if r.codigo != 200 {
            return r;
        }
        let Json::Obj(m) = &r.cuerpo else { return r };
        let items = match m.get("items") {
            Some(Json::Arr(a)) => a.clone(),
            _ => Vec::new(),
        };
        if !items.is_empty() {
            self.contar_lo_servido(p, rama, m, &items);
        }
        if !una {
            return r;
        }
        match items.into_iter().next() {
            Some(Json::Obj(mut item)) => {
                for k in ["coleccion", "segundos", "caduca_ms"] {
                    if let Some(v) = m.get(k) {
                        item.insert(k.into(), v.clone());
                    }
                }
                Respuesta::ok(Json::Obj(item))
            }
            _ => Respuesta::error(404, "ningún ítem de la colección lleva esa huella"),
        }
    }

    /// Lo servido, a la actividad: quién, qué colección, qué huellas y blobs,
    /// y cuánto viven las URLs. Por el buzón: la respuesta no lo espera.
    fn contar_lo_servido(
        &self,
        p: &ore_entrada::http::Peticion,
        rama: Option<&str>,
        m: &std::collections::BTreeMap<String, Json>,
        items: &[Json],
    ) {
        let Some(buzon) = self.buzon.as_ref() else {
            return;
        };
        let de = |k: &str| -> Vec<Json> {
            items
                .iter()
                .filter_map(|i| match i {
                    Json::Obj(o) => o.get(k).cloned(),
                    _ => None,
                })
                .collect()
        };
        let mut detalle = vec![
            ("huellas", Json::Arr(de("huella"))),
            ("blobs", Json::Arr(de("blob"))),
            (
                "segundos",
                m.get("segundos").cloned().unwrap_or(Json::Int(0)),
            ),
        ];
        if let Some(r) = rama.filter(|r| !r.trim().is_empty()) {
            detalle.push(("rama", Json::s(r.trim())));
        }
        let coleccion = match m.get("coleccion") {
            Some(Json::Str(c)) => c.clone(),
            _ => String::new(),
        };
        let mut e = crate::acceso::evento(
            "coleccion:servir",
            &format!("colecciones/{}", coleccion.replace('.', "/")),
            "hecho",
            None,
            None,
        );
        e.detalle = Some(Json::obj(detalle));
        let token = p
            .cabeceras
            .get("authorization")
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })
            .map(|v| v.trim().to_string());
        buzon.echar(token, e);
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

/// Un segmento de la ruta con sus `%XX` resueltos: una huella lleva `/`, `+`,
/// `=` y `:` (`crc64nvme:<base64>`), y viaja codificada. `None` si no es UTF-8.
pub(crate) fn sin_porcentajes(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn primera_de(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start_matches("error: ").to_string())
        .unwrap_or_else(|| "falló sin decir por qué".into())
}

#[cfg(test)]
mod pruebas {
    use super::sin_porcentajes;

    /// Una huella de S3 viaja en la ruta con su `/`, `+`, `=` y `:` codificados.
    #[test]
    fn la_huella_vuelve_de_la_ruta() {
        assert_eq!(
            sin_porcentajes("crc64nvme%3Aab%2Fc%2Bd%3D").as_deref(),
            Some("crc64nvme:ab/c+d=")
        );
        assert_eq!(sin_porcentajes("sin-nada").as_deref(), Some("sin-nada"));
        assert_eq!(sin_porcentajes("%zz"), None);
        assert_eq!(sin_porcentajes("%FF"), None, "no es UTF-8");
        assert_eq!(sin_porcentajes("a%2").as_deref(), Some("a%2"));
    }
}
