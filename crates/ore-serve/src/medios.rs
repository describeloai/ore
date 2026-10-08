//! **La media de una celda, por su puerta** (ADR 0049 B2·2): las rutas del
//! contrato (`docs/media.md`) que `ore-serve` atiende y `ore-medios` sirve.
//!
//! | ruta | operación | qué hace aquí |
//! |---|---|---|
//! | `GET /media/{b}/{s}/{c}/items?prefix=&cursor=&limit=&estado=` | `list` | |
//! | `GET /media/{b}/{s}/{c}/item?path=&version=` o `?digest=` | `stat` | |
//! | `POST /media/{b}/{s}/{c}/urls` `{items, ttl_s}` | `url` | y lo anota en la actividad |
//! | `GET /media/{b}/{s}/{c}/content?path=&version=` o `?digest=` | `open` | 307 a donde están los bytes (B3·3) |
//!
//! **`content`** (0049 B3·3): este proceso no pasa bytes, **dice dónde están**.
//! Una mantenida, en el lago: `ore-medios` firma su blob y se redirige a esa
//! URL (el puesto llega a Google). Una virtual, en el origen: se trae la
//! credencial de su fuente del custodio (la caché de 0046 E9b), `ore-medios`
//! guarda el ítem fijado con ella y da un permiso, y se redirige a
//! `ore-medios/contenido?permiso=…`. En los dos casos, `307` con `Location`
//! y un cuerpo JSON (`url`, `version`, `item`, `expires_ms`) para quien no
//! sigue redirecciones —el SDK—, y la apertura queda en la actividad.
//!
//! Este proceso **decide y no sirve**: autentica (antes de llegar aquí), lee la
//! colección y su puntero en el árbol de la rama —el mismo `leyendo_en` de
//! siempre, con su `fetch` de 0,1 s—, y le pasa a `ore-medios` la colección, su
//! clase y el `metadata_location` de esa transacción. `ore-medios` guarda el
//! índice por ese `metadata_location` y firma con una cuenta viva. Así
//! `ore-serve` sigue sin TLS, sin Iceberg y sin poder leer un origen (lo
//! comprueba `ore-cli/tests/dependencias.rs`).
//!
//! **Lo declarado manda** (0049 B4·2): desde el puesto de un transform, una
//! colección que no está en sus `inputs` es `403 media/no-declarada`, y una que
//! está se lee de la transacción que se fijó al declararla
//! ([`Servidor::fijar_colecciones`]), no de la del puntero. Sin transform se
//! lee libre y queda anotado en el puesto (`colecciones_leidas`).
//!
//! Las rutas de 0046 (`/colecciones/…/items`, `…/items/{huella}`,
//! `…/items/resolver`) siguen hasta el relevo (0049 B6).

use crate::puestos::{Fijada, MediaDelPuesto};
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::Identidad;
use std::collections::BTreeMap;
use std::path::Path;

/// Dónde escucha `ore-medios` en la celda (`host:puerto`). Sin él, las rutas
/// de la media dicen que el servicio no está desplegado.
pub const ENTORNO: &str = "ORE_MEDIOS_DIRECCION";

/// Lo que `ore-medios` tarda como mucho: cargar el índice de una colección de
/// un millón de ítems son ~2 s medidos (B2·0) más la descarga de su listado.
const PLAZO: std::time::Duration = std::time::Duration::from_secs(60);

pub(crate) fn problema(status: u16, tipo: &str, detalle: impl Into<String>) -> Respuesta {
    Respuesta {
        codigo: status,
        cuerpo: Json::obj([
            ("type", Json::s(tipo)),
            ("title", Json::s(tipo)),
            ("status", Json::Int(status as i64)),
            ("detail", Json::s(detalle)),
        ]),
    }
}

/// La colección en el árbol: si existe y si es virtual. Se busca por el
/// documento —`kind: MediaCollection`, su nombre y su schema— entre los YAML
/// del paquete, sin cargar el árbol entero: el directorio es convención.
pub(crate) fn clase_de_la_coleccion(raiz: &Path, b: &str, s: &str, c: &str) -> Option<bool> {
    coleccion_en(raiz, b, s, c).map(|(v, _)| v)
}

/// Un documento de `kind` con ese nombre y schema, entre los YAML de
/// `packages/<b>`.
fn documento_en(raiz: &Path, b: &str, kind: &str, s: &str, nombre: &str) -> Option<Node> {
    fichero_y_documento(raiz, b, kind, s, nombre).map(|(_, n)| n)
}

/// El fichero de un documento del árbol y lo que dice (B4·4: el linaje se
/// escribe en él).
pub(crate) fn fichero_y_documento(
    raiz: &Path,
    b: &str,
    kind: &str,
    s: &str,
    nombre: &str,
) -> Option<(std::path::PathBuf, Node)> {
    let mut ficheros = Vec::new();
    crate::documentos::yamls_de(&raiz.join("packages").join(b), &mut ficheros);
    for f in ficheros {
        let Ok(texto) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(n) = ore_core::parse::parse(&texto) else {
            continue;
        };
        if campo(&n, "kind").as_deref() != Some(kind) {
            continue;
        }
        let Some((_, m)) = n.get("metadata") else {
            continue;
        };
        let schema = campo(m, "schema")
            .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
        if campo(m, "name").as_deref() == Some(nombre) && schema == s {
            return Some((f, n));
        }
    }
    None
}

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
}

/// **Si la colección es escrita** (0049 B4b·2): sin `from`, la llena el
/// código. `None`: no existe.
pub(crate) fn es_escrita(raiz: &Path, b: &str, s: &str, c: &str) -> Option<bool> {
    let n = documento_en(raiz, b, "MediaCollection", s, c)?;
    Some(n.get("spec").is_none_or(|(_, sp)| sp.get("from").is_none()))
}

/// Una petición a `ore-medios` (el puerto de `ore-serve`): su código y su
/// texto. Sin `ORE_MEDIOS_DIRECCION`, o sin respuesta, `503 media/no-desplegado`.
pub(crate) fn pedir_a_medios(ruta: &str, cuerpo: &Json) -> Result<(u16, String), Respuesta> {
    let Some(direccion) = std::env::var(ENTORNO).ok().filter(|d| !d.is_empty()) else {
        return Err(problema(
            503,
            "media/no-desplegado",
            "esta celda no tiene `ore-medios` todavía (0049 B2)",
        ));
    };
    ore_entrada::http::pedir_con(
        "POST",
        &direccion,
        ruta,
        &[],
        Some(cuerpo),
        ore_entrada::http::Plazos {
            conectar: std::time::Duration::from_secs(5),
            responder: PLAZO,
        },
    )
    .map_err(|e| {
        problema(
            503,
            "media/no-desplegado",
            format!("`ore-medios` no contesta: {e}"),
        )
    })
}

/// La colección: si es virtual, y el `objectTable` del que sale.
fn coleccion_en(raiz: &Path, b: &str, s: &str, c: &str) -> Option<(bool, Option<String>)> {
    let n = documento_en(raiz, b, "MediaCollection", s, c)?;
    let spec = n.get("spec").map(|(_, sp)| sp);
    let virtual_ = spec
        .and_then(|sp| campo(sp, "virtual"))
        .is_some_and(|v| v == "true");
    let objeto = spec
        .and_then(|sp| sp.get("from"))
        .and_then(|(_, f)| campo(f, "objectTable"));
    Some((virtual_, objeto))
}

/// **De qué fuente sale una virtual, y en qué variable vive su credencial**
/// (B3·3): la colección nombra su `objectTable` (`b.s.n`, `s.n` o `n`, desde
/// la colección), el `ObjectTable` su `datasource`, y `ontology.config.yaml`
/// su `connectionEnv`. Lo mismo que `ore collections --servir` (`fuente_de`),
/// leyendo sólo esos documentos.
pub(crate) fn fuente_de_la_coleccion(
    raiz: &Path,
    b: &str,
    s: &str,
    c: &str,
) -> Result<(String, String), String> {
    let (_, objeto) = coleccion_en(raiz, b, s, c).ok_or("la colección no está en el árbol")?;
    let objeto = objeto.ok_or("la colección no dice de qué `objectTable` sale")?;
    let partes: Vec<&str> = objeto.split('.').collect();
    let (ob, os, on) = match partes.as_slice() {
        [ob, os, on] => (*ob, *os, *on),
        [os, on] => (b, *os, *on),
        [on] => (b, s, *on),
        _ => return Err(format!("`{objeto}` no es `b.s.n`")),
    };
    let o = documento_en(raiz, ob, "ObjectTable", os, on)
        .ok_or_else(|| format!("`{objeto}` no es un `ObjectTable` del árbol"))?;
    let ds = o
        .get("spec")
        .and_then(|(_, sp)| campo(sp, "datasource"))
        .ok_or_else(|| format!("`{objeto}` no dice su `datasource`"))?;
    let config = std::fs::read_to_string(raiz.join("ontology.config.yaml"))
        .map_err(|e| format!("no se pudo leer `ontology.config.yaml`: {e}"))?;
    let config = ore_core::parse::parse(&config)
        .map_err(|_| "`ontology.config.yaml` no analiza".to_string())?;
    let env = config
        .get("datasources")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .find(|d| campo(d, "name").as_deref() == Some(ds.as_str()))
        .and_then(|d| campo(d, "connectionEnv"))
        .ok_or_else(|| format!("la fuente `{ds}` no está declarada, o no dice `connectionEnv`"))?;
    Ok((ds, env))
}

/// Lo que se le pide a una URL firmada con una credencial temporal: la vida
/// de un permiso (5 min), y nunca más de lo que le queda a la credencial
/// menos 30 s (0046 E9b).
/// [`Servidor::fijar_colecciones`] en un árbol ya abierto.
pub(crate) fn fijar_en(raiz: &Path, inputs: &[String]) -> BTreeMap<String, Fijada> {
    let mut fijadas = BTreeMap::new();
    for i in inputs {
        let completo = ore_core::normalize::completo(i);
        let [b, s, c] = completo.split('.').collect::<Vec<_>>()[..] else {
            continue;
        };
        if clase_de_la_coleccion(raiz, b, s, c).is_none() {
            continue;
        }
        let puntero =
            ore_core::punteros::leer_en(&raiz.join("datasets"), &completo).map(|(_, n)| n);
        let de = |k: &str| {
            puntero
                .as_ref()
                .and_then(|n| campo(n, k))
                .unwrap_or_default()
        };
        fijadas.insert(
            i.clone(),
            Fijada {
                metadata_location: de("metadata_location"),
                transaccion: de("transaccion"),
            },
        );
    }
    fijadas
}

fn vida_hasta(caduca_ms: Option<u64>, ahora_ms: u64) -> u64 {
    let vida = VIDA_DE_UN_PERMISO;
    match caduca_ms {
        Some(c) => (c.saturating_sub(ahora_ms) / 1000)
            .saturating_sub(30)
            .clamp(30, vida),
        None => vida,
    }
}

/// La vida de lo que se pide al abrir un ítem.
pub const VIDA_DE_UN_PERMISO: u64 = 300;

/// `307` con `Location`: el cuerpo de `content` (`{url, …}`) como redirección,
/// para quien la sigue sola (un navegador, `curl -L`). `Respuesta` no lleva
/// cabeceras; esto es la salida en bytes, con el mismo cuerpo.
pub(crate) fn redireccion(r: &Respuesta) -> Option<ore_entrada::http::Salida> {
    let Json::Crudo(texto) = &r.cuerpo else {
        return None;
    };
    let n = ore_core::parse::parse(texto).ok()?;
    let url = campo(&n, "url")?;
    let mut cabeceras = vec![
        ("location".to_string(), url),
        ("content-type".to_string(), "application/json".to_string()),
    ];
    if let Some(v) = campo(&n, "version") {
        cabeceras.push(("ore-media-version".to_string(), v));
    }
    let cuerpo = texto.as_bytes().to_vec();
    Some(ore_entrada::http::Salida::Bytes(ore_entrada::http::Bytes {
        codigo: 307,
        cabeceras,
        largo: Some(cuerpo.len() as u64),
        lector: Box::new(std::io::Cursor::new(cuerpo)),
        finales: None,
    }))
}

impl Servidor {
    /// **Fija las colecciones de `inputs`** (0049 B4·2): de cada nombre que es
    /// una `MediaCollection` en esa rama, su `metadata_location` y su
    /// `transaccion` de ahora, del puntero. Lo demás (vistas, datasets) no se
    /// fija aquí. Una colección sin puntero todavía se fija vacía: el trabajo
    /// la ve vacía hasta el final, aunque otro la llene mientras corre.
    pub(crate) fn fijar_colecciones(
        &self,
        rama: Option<&str>,
        inputs: &[String],
    ) -> BTreeMap<String, Fijada> {
        let mut fijadas = BTreeMap::new();
        self.leyendo_en(rama, |raiz| {
            fijadas = fijar_en(raiz, inputs);
            Respuesta::sin_contenido()
        });
        fijadas
    }

    /// `GET|POST /media/{b}/{s}/{c}/…`: la operación, en la rama que se lee.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn media(
        &self,
        rama: Option<&str>,
        p: &Peticion,
        sujeto: &Identidad,
        b: &str,
        s: &str,
        c: &str,
        operacion: &str,
    ) -> Respuesta {
        if let Err(m) = crate::rutas::token(b)
            .and(crate::rutas::token(s))
            .and(crate::rutas::token(c))
        {
            return problema(422, "media/peticion", m);
        }
        let Some(direccion) = std::env::var(ENTORNO).ok().filter(|d| !d.is_empty()) else {
            return problema(
                503,
                "media/no-desplegado",
                "esta celda no tiene `ore-medios` todavía (0049 B2): las colecciones se sirven por \
                 `/colecciones/…` hasta que lo tenga",
            );
        };
        let coleccion = format!("{b}.{s}.{c}");
        // 0049 B4·2: lo declarado manda, antes de pedir nada a `ore-medios`.
        let corta = ore_core::normalize::a_corto(&coleccion).into_owned();
        let fijada = match self.media_del_puesto(sujeto, &corta) {
            MediaDelPuesto::Libre => None,
            MediaDelPuesto::Declarada(f) => f,
            MediaDelPuesto::NoDeclarada { transform, inputs } => {
                return problema(
                    403,
                    "media/no-declarada",
                    format!(
                        "`{corta}` no está en los inputs de `{transform}` ({}): un transform sólo lee \
                         lo que declara",
                        inputs.join(", ")
                    ),
                );
            }
        };
        let cuerpo_pedido = match operacion {
            "urls" => match ore_core::parse::parse(&p.cuerpo) {
                Ok(n) if !p.cuerpo.trim().is_empty() => Some(n),
                _ => {
                    return problema(
                        400,
                        "media/peticion",
                        "el cuerpo no es JSON: `{items: [...], ttl_s?}`",
                    );
                }
            },
            _ => None,
        };
        let r = self.leyendo_en(rama, |raiz| {
            let Some(virtual_) = clase_de_la_coleccion(raiz, b, s, c) else {
                return problema(
                    404,
                    "media/no-existe",
                    format!("no hay ninguna colección `{coleccion}`"),
                );
            };
            let puntero =
                ore_core::punteros::leer_en(&raiz.join("datasets"), &coleccion).map(|(_, n)| n);
            let de_puntero = |k: &str| {
                puntero
                    .as_ref()
                    .and_then(|n| n.get(k))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let mut pedido: BTreeMap<String, Json> = BTreeMap::new();
            pedido.insert("coleccion".into(), Json::s(&coleccion));
            pedido.insert("virtual".into(), Json::s(virtual_.to_string()));
            // Dentro de un transform, la transacción fijada; si no, la de ahora.
            let (ml, tx) = match &fijada {
                Some(f) => (f.metadata_location.clone(), f.transaccion.clone()),
                None => (de_puntero("metadata_location"), de_puntero("transaccion")),
            };
            pedido.insert("metadata_location".into(), Json::s(ml));
            pedido.insert("transaccion".into(), Json::s(tx));
            match operacion {
                "items" => {
                    for (k, a) in [
                        ("prefix", "prefix"),
                        ("cursor", "cursor"),
                        ("limit", "limit"),
                        ("estado", "estado"),
                    ] {
                        if let Some(v) = p.consulta.get(k) {
                            pedido.insert(a.into(), Json::s(v));
                        }
                    }
                }
                "derivations" => {
                    for k in ["cursor", "limit"] {
                        if let Some(v) = p.consulta.get(k) {
                            pedido.insert(k.into(), Json::s(v));
                        }
                    }
                }
                "item" => {
                    for k in ["path", "version", "digest"] {
                        if let Some(v) = p.consulta.get(k) {
                            pedido.insert(k.into(), Json::s(v));
                        }
                    }
                }
                "content" => {
                    for k in ["path", "version", "digest"] {
                        if let Some(v) = p.consulta.get(k) {
                            pedido.insert(k.into(), Json::s(v));
                        }
                    }
                    let mut caduca = None;
                    // ⭐ 0049 B8·3: una mantenida cuya copia no está completa
                    //   —la última transacción fue virtual, o dejó ítems sin
                    //   blob (`por_copiar`)— sirve esos ítems de su origen:
                    //   lleva la credencial, si la hay. Si no, lo que ya está
                    //   en el lago se sirve igual y lo demás lo dice `ore-medios`.
                    let copia_incompleta = !virtual_
                        && (de_puntero("virtual") != "false"
                            || !matches!(de_puntero("por_copiar").as_str(), "" | "0"));
                    if copia_incompleta
                        && let Ok((fuente, env)) = fuente_de_la_coleccion(raiz, b, s, c)
                        && let Ok((valor, c_ms)) = self.credencial_de_la_fuente(&fuente, &env)
                    {
                        pedido.insert("fuente".into(), Json::s(valor));
                        caduca = c_ms;
                    }
                    if virtual_ {
                        let (fuente, env) = match fuente_de_la_coleccion(raiz, b, s, c) {
                            Ok(f) => f,
                            Err(e) => return problema(502, "media/origen", e),
                        };
                        let (valor, c_ms) = match self.credencial_de_la_fuente(&fuente, &env) {
                            Ok(v) => v,
                            Err(r) => return r,
                        };
                        pedido.insert("fuente".into(), Json::s(valor));
                        caduca = c_ms;
                    }
                    let vida = vida_hasta(caduca, crate::datasets::ahora_ms());
                    pedido.insert("ttl_s".into(), Json::Int(vida as i64));
                }
                _ => {
                    let n = cuerpo_pedido.as_ref().expect("analizado arriba");
                    pedido.insert(
                        "items".into(),
                        n.get("items")
                            .map(|(_, v)| Json::de_node(v))
                            .unwrap_or(Json::Arr(vec![])),
                    );
                    if let Some((_, t)) = n.get("ttl_s") {
                        pedido.insert("ttl_s".into(), Json::de_node(t));
                    }
                }
            }
            let ruta = match operacion {
                "items" => "/indice/items",
                "item" => "/indice/item",
                "derivations" => "/indice/derivaciones",
                "content" => "/indice/abrir",
                _ => "/indice/urls",
            };
            match ore_entrada::http::pedir_con(
                "POST",
                &direccion,
                ruta,
                &[],
                Some(&Json::Obj(pedido)),
                ore_entrada::http::Plazos {
                    conectar: std::time::Duration::from_secs(5),
                    responder: PLAZO,
                },
            ) {
                // Tal cual: `de_node` volvería `null` la cadena `"null"`
                // (medido: el `digest` de un ítem virtual). Se comprueba que
                // es JSON y se pasa como vino.
                Ok((200, texto)) if operacion == "content" => a_donde(&direccion, texto.trim()),
                Ok((codigo, texto)) => match ore_core::parse::parse(texto.trim()) {
                    Ok(_) => Respuesta {
                        codigo,
                        cuerpo: Json::Crudo(texto.trim().to_string()),
                    },
                    Err(_) => problema(502, "media/origen", "`ore-medios` no contestó JSON"),
                },
                Err(e) => problema(
                    503,
                    "media/no-desplegado",
                    format!("`ore-medios` no contesta: {e}"),
                ),
            }
        });
        if operacion == "urls" && r.codigo == 200 {
            self.anotar_las_urls(p, rama, &coleccion, &r.cuerpo);
        }
        // Lo abierto, a la actividad, como una URL servida (un permiso también
        // es un portador): con la forma de `urls`, una entrada.
        if operacion == "content"
            && r.codigo == 307
            && let Json::Crudo(t) = &r.cuerpo
        {
            let lote = Json::Crudo(format!("{{\"urls\":[{t}]}}"));
            self.anotar_las_urls(p, rama, &coleccion, &lote);
        }
        r
    }

    /// Lo firmado, a la actividad de la organización (`coleccion:servir`), como
    /// lo de 0046: quién, de qué colección, qué ítems y cuánto viven. La URL es
    /// un portador y el lago no sabe quién lee.
    fn anotar_las_urls(&self, p: &Peticion, rama: Option<&str>, coleccion: &str, cuerpo: &Json) {
        let Json::Crudo(texto) = cuerpo else { return };
        let Ok(n) = ore_core::parse::parse(texto) else {
            return;
        };
        let Json::Obj(m) = Json::de_node(&n) else {
            return;
        };
        let Some(Json::Arr(urls)) = m.get("urls") else {
            return;
        };
        let mut segundos = 0;
        let mut virtual_ = false;
        let mut items = Vec::new();
        for u in urls {
            let Json::Obj(u) = u else { continue };
            if u.contains_key("error") {
                continue;
            }
            if let Some(Json::Int(t)) = u.get("ttl_s") {
                segundos = *t;
            }
            let Some(Json::Obj(r)) = u.get("item") else {
                continue;
            };
            let s = |k: &str| match r.get(k) {
                Some(Json::Str(v)) => Json::s(v),
                _ => Json::s(""),
            };
            let blob = match r.get("digest") {
                Some(Json::Str(d)) if d.starts_with("sha256:") => {
                    Json::s(d.trim_start_matches("sha256:"))
                }
                _ => {
                    virtual_ = true;
                    Json::s("")
                }
            };
            items.push(Json::obj([
                ("huella", s("checksum")),
                ("blob", blob),
                ("clave", s("path")),
                ("version", s("version")),
            ]));
        }
        if items.is_empty() {
            return;
        }
        let m: BTreeMap<String, Json> = [
            ("coleccion".to_string(), Json::s(coleccion)),
            ("virtual".to_string(), Json::Bool(virtual_)),
            ("segundos".to_string(), Json::Int(segundos)),
        ]
        .into_iter()
        .collect();
        self.contar_lo_servido(p, rama, &m, &items);
    }
}

/// Lo que `ore-medios` contestó a `/indice/abrir`, como la respuesta de
/// `content`: `307`, y en el cuerpo `url` —la del lago tal cual, o la de
/// `ore-medios` con el permiso— más lo que vino (`version`, `item`…), sin
/// reanalizarlo (`de_node` volvería `null` la cadena `"null"`).
/// Dónde lee el puesto los bytes (B3·4): `ORE_MEDIOS_CONTENIDO` si se dice; si
/// no, el host de `ORE_MEDIOS_DIRECCION` en el puerto 8098, el único de
/// `ore-medios` al que la red deja llegar al puesto.
pub(crate) fn direccion_del_contenido(direccion: &str) -> String {
    if let Ok(d) = std::env::var("ORE_MEDIOS_CONTENIDO")
        && !d.trim().is_empty()
    {
        return d.trim().to_string();
    }
    let host = direccion.rsplit_once(':').map_or(direccion, |(h, _)| h);
    format!("{host}:8098")
}

fn a_donde(direccion: &str, texto: &str) -> Respuesta {
    let Ok(n) = ore_core::parse::parse(texto) else {
        return problema(502, "media/origen", "`ore-medios` no contestó JSON");
    };
    if campo(&n, "url").is_some() {
        return Respuesta {
            codigo: 307,
            cuerpo: Json::Crudo(texto.to_string()),
        };
    }
    let Some(permiso) = campo(&n, "permiso") else {
        return problema(502, "media/origen", "`ore-medios` no dio ni URL ni permiso");
    };
    let url = format!(
        "http://{}/contenido?permiso={permiso}",
        direccion_del_contenido(direccion)
    );
    let resto = texto.trim_start().strip_prefix('{').unwrap_or("}");
    Respuesta {
        codigo: 307,
        cuerpo: Json::Crudo(format!("{{\"url\":{},{resto}", Json::s(url).jcs())),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_fuente_de_una_virtual_sale_de_su_objecttable_y_de_la_config() {
        let d = std::env::temp_dir().join(format!("ore-medios-fuente-{}", std::process::id()));
        let col = d.join("packages/legal/archivo");
        let obj = d.join("packages/s3_ventas/docs");
        std::fs::create_dir_all(&col).unwrap();
        std::fs::create_dir_all(&obj).unwrap();
        std::fs::write(
            col.join("contratos.yaml"),
            "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: { name: contratos, namespace: legal, schema: archivo }\nspec: { owner: team:legal, media: document, formats: [pdf], from: { objectTable: s3_ventas.docs.contratos }, virtual: true }\n",
        )
        .unwrap();
        std::fs::write(
            obj.join("contratos.yaml"),
            "apiVersion: oos.dev/v1alpha16\nkind: ObjectTable\nmetadata: { name: contratos, namespace: s3_ventas, schema: docs }\nspec: { datasource: s3_ventas, prefix: \"Nueva carpeta/contratos/\", media: document }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: s3_ventas, type: s3, connectionEnv: S3_VENTAS_URL }\n",
        )
        .unwrap();
        assert_eq!(
            fuente_de_la_coleccion(&d, "legal", "archivo", "contratos"),
            Ok(("s3_ventas".to_string(), "S3_VENTAS_URL".to_string()))
        );
        assert!(fuente_de_la_coleccion(&d, "legal", "archivo", "nada").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn content_dice_a_donde_y_la_vida_no_pasa_de_la_credencial() {
        let r = a_donde(
            "ore-medios:8097",
            r#"{"permiso":"ab12","version":"null","item":{"digest":null}}"#,
        );
        assert_eq!(r.codigo, 307);
        let t = r.cuerpo.jcs();
        assert!(
            t.contains("\"url\":\"http://ore-medios:8098/contenido?permiso=ab12\""),
            "{t}"
        );
        assert!(
            t.contains("\"digest\":null"),
            "el null sigue siendo null: {t}"
        );
        let r = a_donde("x", r#"{"url":"https://lago/firmada","desde":"lago"}"#);
        assert!(r.cuerpo.jcs().contains("https://lago/firmada"));
        let s = match redireccion(&r) {
            Some(ore_entrada::http::Salida::Bytes(b)) => b,
            _ => panic!("una redirección"),
        };
        assert_eq!(s.codigo, 307);
        assert!(
            s.cabeceras
                .iter()
                .any(|(k, v)| k == "location" && v == "https://lago/firmada")
        );
        assert_eq!(vida_hasta(None, 0), 300);
        assert_eq!(vida_hasta(Some(200_000), 0), 170);
        assert_eq!(vida_hasta(Some(10_000), 0), 30);
    }

    #[test]
    fn al_declarar_se_fija_la_transaccion_de_cada_coleccion() {
        let d = std::env::temp_dir().join(format!("ore-medios-fijar-{}", std::process::id()));
        let dir = d.join("packages/legal/archivo/collections");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(d.join("datasets/legal/archivo")).unwrap();
        let coleccion = |n: &str| {
            format!(
                "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: {{ name: {n}, namespace: legal, schema: archivo }}\nspec: {{ owner: team:legal, media: document, formats: [pdf] }}\n"
            )
        };
        std::fs::write(dir.join("contratos.yaml"), coleccion("contratos")).unwrap();
        std::fs::write(dir.join("vacia.yaml"), coleccion("vacia")).unwrap();
        std::fs::write(
            d.join("datasets/legal/archivo/contratos.json"),
            "{\"metadata_location\":\"gs://lago/m/v3.json\",\"transaccion\":\"tx-3\"}",
        )
        .unwrap();
        let f = fijar_en(
            &d,
            &[
                "legal.archivo.contratos".into(),
                "legal.archivo.vacia".into(),
                "legal.registro".into(),
            ],
        );
        assert_eq!(
            f.get("legal.archivo.contratos"),
            Some(&Fijada {
                metadata_location: "gs://lago/m/v3.json".into(),
                transaccion: "tx-3".into(),
            })
        );
        assert_eq!(
            f.get("legal.archivo.vacia"),
            Some(&Fijada::default()),
            "sin puntero, se fija vacía"
        );
        assert!(
            !f.contains_key("legal.registro"),
            "lo que no es una colección no se fija aquí"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn la_clase_sale_del_documento_y_su_schema() {
        let d = std::env::temp_dir().join(format!("ore-medios-clase-{}", std::process::id()));
        let dir = d.join("packages/legal/archivo/collections");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("contratos.yaml"),
            "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: { name: contratos, namespace: legal, schema: archivo }\nspec: { owner: team:legal, media: document, formats: [pdf], virtual: true }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/legal/fotos.yaml"),
            "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: { name: fotos, namespace: legal }\nspec: { owner: team:legal, media: image, formats: [jpg] }\n",
        )
        .unwrap();
        assert_eq!(
            clase_de_la_coleccion(&d, "legal", "archivo", "contratos"),
            Some(true)
        );
        assert_eq!(
            clase_de_la_coleccion(&d, "legal", "default", "fotos"),
            Some(false)
        );
        assert_eq!(
            clase_de_la_coleccion(&d, "legal", "default", "contratos"),
            None,
            "otro schema"
        );
        assert_eq!(clase_de_la_coleccion(&d, "legal", "archivo", "nada"), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
