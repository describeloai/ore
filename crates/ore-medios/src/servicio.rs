//! **Las rutas de `ore-medios`**, para `ore-serve` y para nadie más (la red de
//! la celda lo impone, B2·4). `ore-serve` ya autenticó, comprobó la concesión
//! y leyó el puntero de la rama: aquí llega la colección, su clase y el
//! `metadata_location` de la transacción que se sirve.
//!
//! | ruta | operación (`docs/media.md`) |
//! |---|---|
//! | `POST /indice/items` | `list` |
//! | `POST /indice/item` | `stat` |
//! | `POST /indice/urls` | `url` |
//! | `GET /salud` | si vive, y cuántos índices tiene |
//!
//! Los errores van en la forma de RFC 9457 (`type`, `title`, `status`,
//! `detail`) con los tipos del contrato; `ore-serve` los pasa tal cual.

use crate::firma::{self, Pedida};
use crate::indice::{Estado, Indice, Indices, Item};
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::{Peticion, Respuesta};
use ore_store::almacen::Almacen;
use ore_store::lago::Lago;
use std::sync::Arc;

/// La vida de una URL, en segundos (`docs/media.md` §2 `url`).
pub const TTL_POR_DEFECTO: u64 = 300;
pub const TTL_MINIMO: u64 = 30;
pub const TTL_MAXIMO: u64 = 3600;
/// Cuántos ítems por página y por lote, como mucho.
pub const LIMITE: usize = 1000;

/// De dónde salen los lotes de un listado: el lago de verdad o, en las
/// pruebas, uno en memoria.
pub trait Listados: Send + Sync {
    fn lotes(
        &self,
        dataset: &str,
        metadata_location: &str,
    ) -> Result<Vec<arrow_array::RecordBatch>, String>;
}

impl Listados for Lago {
    fn lotes(
        &self,
        dataset: &str,
        metadata_location: &str,
    ) -> Result<Vec<arrow_array::RecordBatch>, String> {
        let t = self.abrir(metadata_location, dataset)?;
        Lago::lotes(self, &t)
    }
}

pub struct Servicio {
    pub listados: Box<dyn Listados>,
    pub cuenta: Arc<dyn Almacen>,
    pub indices: Indices,
}

/// Un error del contrato (RFC 9457).
pub fn problema(status: u16, tipo: &str, detalle: impl Into<String>) -> Respuesta {
    Respuesta {
        codigo: status,
        cuerpo: problema_json(status, tipo, detalle),
    }
}

fn problema_json(status: u16, tipo: &str, detalle: impl Into<String>) -> Json {
    let titulo = match tipo {
        "media/no-existe" => "No existe",
        "media/limite" => "Fuera de los límites",
        "media/origen" => "El origen falló",
        _ => "Petición no válida",
    };
    Json::obj([
        ("type", Json::s(tipo)),
        ("title", Json::s(titulo)),
        ("status", Json::Int(status as i64)),
        ("detail", Json::s(detalle)),
    ])
}

/// Lo común de las tres: la colección y su transacción.
struct Pedido<'a> {
    coleccion: &'a str,
    virtual_: bool,
    metadata_location: &'a str,
    transaccion: &'a str,
}

fn texto<'a>(n: &'a Node, k: &str) -> Option<&'a str> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
}

fn pedido(n: &Node) -> Result<Pedido<'_>, Respuesta> {
    let coleccion = texto(n, "coleccion")
        .ok_or_else(|| problema(400, "media/peticion", "falta `coleccion` (`b.s.n`)"))?;
    if coleccion.split('.').count() != 3 {
        return Err(problema(
            400,
            "media/peticion",
            format!("`{coleccion}` no es `b.s.n`"),
        ));
    }
    let metadata_location = texto(n, "metadata_location").ok_or_else(|| {
        problema(
            404,
            "media/no-existe",
            format!("`{coleccion}` no tiene todavía ninguna transacción"),
        )
    })?;
    Ok(Pedido {
        coleccion,
        virtual_: texto(n, "virtual") == Some("true"),
        metadata_location,
        transaccion: texto(n, "transaccion").unwrap_or(""),
    })
}

impl Servicio {
    pub fn atender(&self, p: &Peticion) -> Respuesta {
        match (p.metodo.as_str(), p.ruta.as_str()) {
            ("GET", "/salud") => Respuesta::ok(Json::obj([(
                "indices",
                Json::Int(self.indices.cuantos() as i64),
            )])),
            ("POST", ruta @ ("/indice/items" | "/indice/item" | "/indice/urls")) => {
                let n = match ore_core::parse::parse(&p.cuerpo) {
                    Ok(n) => n,
                    Err(_) => return problema(400, "media/peticion", "el cuerpo no es JSON"),
                };
                let ped = match pedido(&n) {
                    Ok(p) => p,
                    Err(r) => return r,
                };
                let ix = match self.indice(&ped) {
                    Ok(i) => i,
                    Err(e) => return problema(502, "media/origen", e),
                };
                match ruta {
                    "/indice/items" => self.items(&ix, &n),
                    "/indice/item" => self.item(&ix, &n),
                    _ => self.urls(&ix, &n),
                }
            }
            _ => problema(
                404,
                "media/no-existe",
                format!("{} {} no es una ruta", p.metodo, p.ruta),
            ),
        }
    }

    fn indice(&self, p: &Pedido<'_>) -> Result<Arc<Indice>, String> {
        let dataset = format!("colecciones/{}", p.coleccion.replace('.', "/"));
        self.indices.obtener(p.metadata_location, || {
            let lotes = self.listados.lotes(&dataset, p.metadata_location)?;
            Indice::de_lotes(p.coleccion, p.virtual_, p.transaccion, &lotes)
        })
    }

    /// `list`.
    fn items(&self, ix: &Indice, n: &Node) -> Respuesta {
        let estado = match texto(n, "estado").map(Estado::de) {
            None => Estado::Actual,
            Some(Some(e)) => e,
            Some(None) => {
                return problema(
                    400,
                    "media/peticion",
                    "`estado` es `actual`, `retirado`, `perdido` o `todos`",
                );
            }
        };
        let limite = texto(n, "limit")
            .and_then(|l| l.parse::<usize>().ok())
            .unwrap_or(LIMITE)
            .clamp(1, LIMITE);
        match ix.pagina(texto(n, "prefix"), texto(n, "cursor"), limite, estado) {
            Ok((pagina, cursor)) => Respuesta::ok(Json::obj([
                ("as_of", Json::s(&ix.transaccion)),
                (
                    "items",
                    Json::Arr(pagina.iter().map(|it| ix.referencia(it)).collect()),
                ),
                (
                    "cursor",
                    cursor.map(Json::s).unwrap_or(Json::Crudo("null".into())),
                ),
            ])),
            Err(e) => problema(400, "media/peticion", e),
        }
    }

    /// Un ítem por `path` (y `version`) o por `digest`.
    fn buscar<'a>(&self, ix: &'a Indice, n: &Node) -> Result<&'a Item, Json> {
        let it = match (texto(n, "path"), texto(n, "digest")) {
            (Some(p), _) => ix.por_camino(p, texto(n, "version")),
            (None, Some(d)) => ix.por_contenido(d),
            (None, None) => {
                return Err(problema_json(
                    400,
                    "media/peticion",
                    "falta `path` o `digest`",
                ));
            }
        };
        it.ok_or_else(|| {
            problema_json(
                404,
                "media/no-existe",
                format!("`{}` no tiene ese ítem", ix.coleccion),
            )
        })
    }

    /// `stat`.
    fn item(&self, ix: &Indice, n: &Node) -> Respuesta {
        match self.buscar(ix, n) {
            Ok(it) => {
                let mut r = ix.referencia(it);
                if let Json::Obj(m) = &mut r {
                    m.insert("current".into(), Json::Bool(ix.es_actual(it)));
                }
                Respuesta::ok(r)
            }
            Err(p) => Respuesta {
                codigo: 404,
                cuerpo: p,
            },
        }
    }

    /// `url`: una por ítem, en su posición; el error de uno va en la suya.
    fn urls(&self, ix: &Indice, n: &Node) -> Respuesta {
        let pedidos = n.get("items").map(|(_, v)| v.items()).unwrap_or(&[]);
        if pedidos.is_empty() || pedidos.len() > LIMITE {
            return problema(
                413,
                "media/limite",
                format!("de 1 a {LIMITE} ítems por petición"),
            );
        }
        let segundos = texto(n, "ttl_s")
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(TTL_POR_DEFECTO)
            .clamp(TTL_MINIMO, TTL_MAXIMO);
        // Cada pedido, a su ítem o a su error; los que se firman, juntos.
        let mut salida: Vec<Option<Json>> = vec![None; pedidos.len()];
        let mut firmar: Vec<(usize, Pedida)> = Vec::new();
        // La referencia de cada uno va con su URL: quien pide sabe qué firmó.
        let mut firmar_ref: Vec<Option<Json>> = vec![None; pedidos.len()];
        for (i, p) in pedidos.iter().enumerate() {
            match self.buscar(ix, p) {
                Err(e) => salida[i] = Some(Json::obj([("error", e)])),
                Ok(_) if ix.virtual_ => {
                    salida[i] = Some(Json::obj([(
                        "error",
                        problema_json(
                            501,
                            "media/origen",
                            "la URL de un ítem de una colección virtual la firma la puerta de lectura (0049 B3)",
                        ),
                    )]))
                }
                Ok(it) => {
                    let Some(blob) = it.blob.clone() else {
                        salida[i] = Some(Json::obj([(
                            "error",
                            problema_json(
                                404,
                                "media/no-existe",
                                "el ítem no tiene blob en el lago",
                            ),
                        )]));
                        continue;
                    };
                    let tipo = it
                        .tipo
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into());
                    let nombre = it
                        .camino
                        .rsplit('/')
                        .next()
                        .unwrap_or(&it.camino)
                        .to_string();
                    let (_, cabecera) = ore_core::medios::disposicion(&tipo, &nombre);
                    firmar_ref[i] = Some(ix.referencia(it));
                    firmar.push((
                        i,
                        Pedida {
                            blob,
                            tipo: Some(tipo),
                            disposicion: Some(cabecera),
                        },
                    ));
                }
            }
        }
        let caduca_ms = ahora_ms() + (segundos as i64) * 1000;
        let lista: Vec<Pedida> = firmar
            .iter()
            .map(|(_, p)| Pedida {
                blob: p.blob.clone(),
                tipo: p.tipo.clone(),
                disposicion: p.disposicion.clone(),
            })
            .collect();
        for ((i, _), u) in firmar
            .iter()
            .zip(firma::firmar(&self.cuenta, &lista, segundos))
        {
            salida[*i] = Some(match u {
                Ok(url) => Json::obj([
                    (
                        "item",
                        firmar_ref[*i].take().unwrap_or(Json::Crudo("null".into())),
                    ),
                    ("url", Json::s(url)),
                    ("expires_ms", Json::Int(caduca_ms)),
                    ("ttl_s", Json::Int(segundos as i64)),
                ]),
                Err(e) => Json::obj([("error", problema_json(502, "media/origen", e))]),
            });
        }
        Respuesta::ok(Json::obj([(
            "urls",
            Json::Arr(
                salida
                    .into_iter()
                    .map(|s| s.unwrap_or(Json::Crudo("null".into())))
                    .collect(),
            ),
        )]))
    }
}

fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::firma::pruebas::Firmante;
    use crate::indice::pruebas::listado;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Un lago que cuenta cuántas veces se le pide un listado.
    struct Contador(Arc<AtomicUsize>);
    impl Listados for Contador {
        fn lotes(&self, dataset: &str, ml: &str) -> Result<Vec<arrow_array::RecordBatch>, String> {
            assert_eq!(dataset, "colecciones/legal/archivo/contratos");
            if ml == "rota" {
                return Err("no se pudo leer".into());
            }
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(vec![listado(&[
                ("docs/a.pdf", "v1", "crc64nvme:A", "actual", Some("aa")),
                ("docs/c.pdf", "v1", "crc64nvme:C1", "retirado", Some("cc")),
                ("docs/c.pdf", "v2", "crc64nvme:C2", "actual", Some("bb")),
                ("docs/sin.pdf", "v1", "crc64nvme:S", "actual", None),
            ])])
        }
    }

    fn servicio() -> (Servicio, Arc<AtomicUsize>) {
        let n = Arc::new(AtomicUsize::new(0));
        (
            Servicio {
                listados: Box::new(Contador(n.clone())),
                cuenta: Arc::new(Firmante),
                indices: Indices::new_para_pruebas(),
            },
            n,
        )
    }

    impl Indices {
        fn new_para_pruebas() -> Indices {
            Indices::nuevo(1_000_000)
        }
    }

    fn pedir(s: &Servicio, ruta: &str, cuerpo: &str) -> (u16, String) {
        let r = s.atender(&Peticion {
            metodo: "POST".into(),
            ruta: ruta.into(),
            cabeceras: BTreeMap::new(),
            cuerpo: cuerpo.into(),
            consulta: BTreeMap::new(),
        });
        (r.codigo, r.cuerpo.jcs())
    }

    const BASE: &str =
        r#""coleccion":"legal.archivo.contratos","metadata_location":"m7","transaccion":"7""#;

    #[test]
    fn list_da_referencias_sin_urls_y_el_indice_se_carga_una_vez() {
        let (s, cargas) = servicio();
        let (c, b) = pedir(&s, "/indice/items", &format!("{{{BASE},\"limit\":\"1\"}}"));
        assert_eq!(c, 200, "{b}");
        assert!(
            b.contains("\"as_of\":\"7\"") && b.contains("docs/a.pdf") && !b.contains("https://"),
            "{b}"
        );
        let cursor = b
            .split("\"cursor\":\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        let (_, b2) = pedir(
            &s,
            "/indice/items",
            &format!("{{{BASE},\"limit\":\"1\",\"cursor\":\"{cursor}\"}}"),
        );
        assert!(
            b2.contains("docs/c.pdf") && b2.contains("\"version\":\"v2\""),
            "{b2}"
        );
        assert_eq!(cargas.load(Ordering::Relaxed), 1, "dos páginas, una carga");
    }

    #[test]
    fn stat_dice_si_es_la_actual_y_lo_que_no_esta_es_404() {
        let (s, _) = servicio();
        let (c, b) = pedir(
            &s,
            "/indice/item",
            &format!("{{{BASE},\"path\":\"docs/c.pdf\",\"version\":\"v1\"}}"),
        );
        assert_eq!(c, 200);
        assert!(b.contains("\"current\":false"), "{b}");
        let (c, b) = pedir(
            &s,
            "/indice/item",
            &format!("{{{BASE},\"path\":\"docs/no.pdf\"}}"),
        );
        assert_eq!(c, 404);
        assert!(b.contains("\"type\":\"media/no-existe\""), "{b}");
    }

    #[test]
    fn url_por_lote_con_el_error_en_su_posicion_y_el_ttl_recortado() {
        let (s, _) = servicio();
        let (c, b) = pedir(
            &s,
            "/indice/urls",
            &format!(
                "{{{BASE},\"ttl_s\":\"99999\",\"items\":[{{\"path\":\"docs/a.pdf\"}},{{\"path\":\"docs/no.pdf\"}},{{\"digest\":\"sha256:bb\"}},{{\"path\":\"docs/sin.pdf\"}}]}}"
            ),
        );
        assert_eq!(c, 200, "{b}");
        let j = ore_core::parse::parse(&b).unwrap();
        let u = j.get("urls").unwrap().1.items();
        assert!(
            u[0].get("url")
                .unwrap()
                .1
                .as_str()
                .unwrap()
                .contains("blobs/sha256/aa?vida=3600")
        );
        assert_eq!(u[0].get("ttl_s").unwrap().1.as_str(), Some("3600"));
        assert!(u[1].get("error").is_some());
        assert!(
            u[2].get("url")
                .unwrap()
                .1
                .as_str()
                .unwrap()
                .contains("sha256/bb")
        );
        assert!(u[3].get("error").is_some(), "sin blob no se firma");
    }

    /// `ore-serve` reenvía `ttl_s` y `limit` como números JSON.
    #[test]
    fn los_numeros_llegan_como_numeros() {
        let (s, _) = servicio();
        let (_, b) = pedir(
            &s,
            "/indice/urls",
            &format!("{{{BASE},\"ttl_s\":120,\"items\":[{{\"path\":\"docs/a.pdf\"}}]}}"),
        );
        assert!(b.contains("vida=120"), "{b}");
        let (_, b) = pedir(&s, "/indice/items", &format!("{{{BASE},\"limit\":1}}"));
        assert!(
            b.contains("\"cursor\":\""),
            "una página de 1 da cursor: {b}"
        );
    }

    #[test]
    fn una_virtual_todavia_no_firma_y_un_lago_roto_es_502() {
        let (s, _) = servicio();
        let (_, b) = pedir(
            &s,
            "/indice/urls",
            &format!("{{{BASE},\"virtual\":\"true\",\"items\":[{{\"path\":\"docs/a.pdf\"}}]}}"),
        );
        assert!(b.contains("0049 B3"), "{b}");
        let (c, b) = pedir(
            &s,
            "/indice/items",
            r#"{"coleccion":"legal.archivo.contratos","metadata_location":"rota"}"#,
        );
        assert_eq!(c, 502);
        assert!(b.contains("media/origen"), "{b}");
        let (c, _) = pedir(
            &s,
            "/indice/items",
            r#"{"coleccion":"legal.archivo.contratos"}"#,
        );
        assert_eq!(c, 404, "sin transacción todavía");
    }
}
