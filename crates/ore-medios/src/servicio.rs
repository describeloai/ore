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
//! | `POST /indice/abrir` | el permiso de leer un ítem (B3·2) |
//! | `POST /indice/derivaciones` | el registro de lo que `apply()` derivó (B9) |
//! | `GET /contenido?permiso=…` | `open` / `read_range`: los bytes (B3·2) |
//! | `GET /salud` | si vive, y cuántos índices tiene |
//!
//! ⭐ `/contenido` es la única que **no** viene de `ore-serve`: la pide el
//!   puesto, con el permiso que `ore-serve` le pidió aquí (la red lo deja
//!   entrar sólo a esa, B3·4). Sin permiso válido no hay nada que leer.
//!
//! Los errores van en la forma de RFC 9457 (`type`, `title`, `status`,
//! `detail`) con los tipos del contrato; `ore-serve` los pasa tal cual.

use crate::contenido::{Origen, Pieza};
use crate::firma::{self, Pedida};
use crate::indice::{Estado, Indice, Indices, Item};
use crate::permisos::{self, Permisos};
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::{Peticion, Respuesta, Salida};
use ore_store::almacen::Almacen;
use ore_store::lago::Lago;
use std::sync::Arc;

/// La vida de una URL, en segundos (`docs/media.md` §2 `url`).
pub const TTL_POR_DEFECTO: u64 = 300;
pub const TTL_MINIMO: u64 = 30;
pub const TTL_MAXIMO: u64 = 3600;
/// Cuántos ítems por página y por lote, como mucho.
pub const LIMITE: usize = 1000;
/// Cuántos ítems verifica una petición de `verify`, como mucho: lee los bytes
/// enteros (0049 H4; para auditar, no para el camino caliente).
pub const LIMITE_VERIFY: usize = 100;

/// De dónde salen los lotes de un listado: el lago de verdad o, en las
/// pruebas, uno en memoria.
pub trait Listados: Send + Sync {
    fn lotes(
        &self,
        dataset: &str,
        metadata_location: &str,
    ) -> Result<Vec<arrow_array::RecordBatch>, String>;

    /// **Sella un manifiesto** (B4b·1): las filas, sobre `base` si la hay
    /// (fundidas por la clave de la cabecera). Lo que `ore-store sellar` dice,
    /// una línea de JSON. Por defecto, este lago no escribe.
    fn sellar(
        &self,
        cabecera: &ore_store::sobre::Cabecera,
        dataset: &str,
        base: Option<&str>,
        filas: &[String],
    ) -> Result<String, String> {
        let _ = (cabecera, dataset, base, filas);
        Err("este lago no escribe".into())
    }

    /// **Si una transacción de antes sigue en la tabla** (0049 H1): el snapshot
    /// de `historica` entre los de `actual`. `Ok(false)`: la retención lo
    /// recogió; `Err`: el lago no contesta. Por defecto, sigue.
    fn sigue(&self, dataset: &str, historica: &str, actual: &str) -> Result<bool, String> {
        let _ = (dataset, historica, actual);
        Ok(true)
    }
}

impl Listados for Lago {
    fn sellar(
        &self,
        cabecera: &ore_store::sobre::Cabecera,
        dataset: &str,
        base: Option<&str>,
        filas: &[String],
    ) -> Result<String, String> {
        ore_store::ciclo::sellar(
            self,
            cabecera,
            dataset,
            base,
            base.is_some(),
            filas.iter().map(String::as_str),
        )
    }

    fn lotes(
        &self,
        dataset: &str,
        metadata_location: &str,
    ) -> Result<Vec<arrow_array::RecordBatch>, String> {
        let t = self.abrir(metadata_location, dataset)?;
        Lago::lotes(self, &t)
    }

    fn sigue(&self, dataset: &str, historica: &str, actual: &str) -> Result<bool, String> {
        let hoy = self.abrir(actual, dataset)?;
        let meta = hoy.metadata();
        match self.abrir(historica, dataset) {
            // Su `metadata.json` está: ¿su snapshot sigue entre los de hoy?
            Ok(t) => Ok(t
                .metadata()
                .current_snapshot_id()
                .is_none_or(|id| meta.snapshots().any(|s| s.snapshot_id() == id))),
            // No se abre: recogido si la tabla ya no lo nombra; si lo nombra, el
            // lago falla.
            Err(e) => {
                if meta
                    .metadata_log()
                    .iter()
                    .any(|l| l.metadata_file == historica)
                {
                    Err(e)
                } else {
                    Ok(false)
                }
            }
        }
    }
}

pub struct Servicio {
    pub listados: Box<dyn Listados>,
    pub cuenta: Arc<dyn Almacen>,
    pub indices: Indices,
    /// Los `sha256` que la puerta de lectura calculó al paso (B3·1).
    pub vistos: Arc<crate::contenido::Vistos>,
    /// Los permisos vivos de leer un ítem (B3·2).
    pub permisos: Permisos,
    /// Las transacciones abiertas de las colecciones escritas (B4b·1).
    pub escrituras: crate::escritura::Escrituras,
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
        "media/cambiado" => "La versión fijada cambió",
        "media/permiso" => "Sin permiso válido",
        "media/rango" => "El rango no cabe",
        "media/transaccion" => "La transacción no está abierta",
        "media/digest-no-casa" => "El digest no es el de los bytes",
        "media/corrupto" => "Los bytes no casan con el ítem",
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
pub(crate) struct Pedido<'a> {
    pub coleccion: &'a str,
    pub virtual_: bool,
    pub metadata_location: &'a str,
    pub transaccion: &'a str,
}

/// El dataset del listado de una colección, en el lago.
fn dataset_de(coleccion: &str) -> String {
    format!("colecciones/{}", coleccion.replace('.', "/"))
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
    /// Lo que el servidor llama: los bytes de `/contenido` en flujo, y lo
    /// demás como una respuesta.
    pub fn atender_flujo(&self, p: &Peticion) -> Salida {
        if p.metodo == "GET" && p.ruta == "/contenido" {
            return match self.contenido(p) {
                Ok(b) => Salida::Bytes(b),
                Err(r) => Salida::Una(r),
            };
        }
        Salida::Una(self.atender(p))
    }

    /// **El puerto del puesto** (B3·4): sólo `GET /contenido`. Lo demás —el
    /// índice, la firma, los permisos— confía en quien llama porque sólo llama
    /// `ore-serve`, y por eso vive en el otro puerto, al que el puesto no llega
    /// (la red lo impone; esto, además, lo dice).
    /// El puerto del puesto, con la subida (B4b·1): `PUT /subida` recibe el
    /// cuerpo en flujo; lo demás, [`Servicio::atender_contenido`].
    pub fn atender_del_puesto(
        &self,
        p: &Peticion,
        subida: Option<&mut ore_entrada::http::Subida>,
    ) -> Salida {
        match subida {
            Some(s) if es_subida(p) => {
                let largo = s.largo;
                Salida::Una(self.subir(p, s, largo))
            }
            _ => self.atender_contenido(p),
        }
    }

    pub fn atender_contenido(&self, p: &Peticion) -> Salida {
        if p.metodo == "GET" && p.ruta == "/contenido" {
            return self.atender_flujo(p);
        }
        Salida::Una(problema(
            404,
            "media/no-existe",
            format!(
                "{} {} no es una ruta de este puerto: sólo GET /contenido",
                p.metodo, p.ruta
            ),
        ))
    }

    /// `GET /contenido?permiso=…` (con `Range`, si se quiere).
    fn contenido(&self, p: &Peticion) -> Result<ore_entrada::http::Bytes, Respuesta> {
        let sin = || {
            problema(
                401,
                "media/permiso",
                "el permiso falta, caducó o no es de esta celda: pídelo otra vez a la puerta",
            )
        };
        let id = p.consulta.get("permiso").ok_or_else(sin)?;
        let permiso = self.permisos.de(id).ok_or_else(sin)?;
        crate::contenido::abrir(
            self.cuenta.as_ref(),
            &permiso.pieza,
            p.cabeceras.get("range").map(String::as_str),
            self.vistos.clone(),
        )
    }

    pub fn atender(&self, p: &Peticion) -> Respuesta {
        match (p.metodo.as_str(), p.ruta.as_str()) {
            ("GET", "/salud") => Respuesta::ok(Json::obj([
                ("indices", Json::Int(self.indices.cuantos() as i64)),
                ("permisos", Json::Int(self.permisos.cuantos() as i64)),
                ("transacciones", Json::Int(self.escrituras.cuantas() as i64)),
            ])),
            (
                "POST",
                ruta @ ("/escritura/abrir" | "/escritura/confirmar" | "/escritura/abortar"),
            ) => {
                let n = match ore_core::parse::parse(&p.cuerpo) {
                    Ok(n) => n,
                    Err(_) => return problema(400, "media/peticion", "el cuerpo no es JSON"),
                };
                match ruta {
                    "/escritura/abrir" => self.escritura_abrir(&n),
                    "/escritura/confirmar" => self.escritura_confirmar(&n),
                    _ => self.escritura_abortar(&n),
                }
            }
            (
                "POST",
                ruta @ ("/indice/items"
                | "/indice/item"
                | "/indice/urls"
                | "/indice/abrir"
                | "/indice/verificar"
                | "/indice/derivaciones"),
            ) => {
                let n = match ore_core::parse::parse(&p.cuerpo) {
                    Ok(n) => n,
                    Err(_) => return problema(400, "media/peticion", "el cuerpo no es JSON"),
                };
                // 0049 B9 · Una escrita recién creada no tiene transacción:
                //   su registro está vacío, no «no existe» —la primera pasada
                //   de `apply()` lo lee antes de escribir nada—.
                if ruta == "/indice/derivaciones" && texto(&n, "metadata_location").is_none() {
                    return Respuesta::ok(Json::obj([
                        ("as_of", Json::Crudo("null".into())),
                        ("derivations", Json::Arr(Vec::new())),
                        ("cursor", Json::Crudo("null".into())),
                    ]));
                }
                let ped = match pedido(&n) {
                    Ok(p) => p,
                    Err(r) => return r,
                };
                let ix = match self.indice(&ped) {
                    Ok(i) => i,
                    Err(e) => return self.no_se_carga(&n, &ped, e),
                };
                match ruta {
                    "/indice/items" => self.items(&ix, &n),
                    "/indice/item" => self.item(&ix, &n),
                    "/indice/abrir" => self.abrir(&ix, &n),
                    "/indice/verificar" => self.verificar(&ix, &n),
                    "/indice/derivaciones" => self.derivaciones(&ix, &n),
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

    /// Un índice que no se carga: el lago que falla (`502`) o, si se pidió una
    /// transacción de antes (`ore-serve` manda la de hoy en `actual`), la
    /// retención que ya la recogió (`404`, 0049 H1).
    fn no_se_carga(&self, n: &Node, p: &Pedido<'_>, e: String) -> Respuesta {
        if let Some(actual) = texto(n, "actual")
            && actual != p.metadata_location
            && let Ok(false) =
                self.listados
                    .sigue(&dataset_de(p.coleccion), p.metadata_location, actual)
        {
            return problema(
                404,
                "media/no-existe",
                format!(
                    "la transacción `{}` de `{}` ya no se puede leer: la retención de la colección la recogió",
                    p.transaccion, p.coleccion
                ),
            );
        }
        problema(502, "media/origen", e)
    }

    pub(crate) fn indice(&self, p: &Pedido<'_>) -> Result<Arc<Indice>, String> {
        let dataset = dataset_de(p.coleccion);
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

    /// 0049 B9 · **El registro** de una colección escrita por `apply()`: una
    /// entrada por origen, por cursor.
    fn derivaciones(&self, ix: &Indice, n: &Node) -> Respuesta {
        let limite = texto(n, "limit")
            .and_then(|l| l.parse::<usize>().ok())
            .unwrap_or(LIMITE)
            .clamp(1, LIMITE);
        match ix.derivaciones(texto(n, "cursor"), limite) {
            Ok((pagina, cursor)) => Respuesta::ok(Json::obj([
                ("as_of", Json::s(&ix.transaccion)),
                ("derivations", Json::Arr(pagina)),
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
                    // B3·1: el sha256 que una lectura entera calculó al paso
                    // (`open-007`), si el ítem no lo traía.
                    let visto = it
                        .blob
                        .is_none()
                        .then(|| self.vistos.de(&ix.coleccion, &it.camino, &it.version))
                        .flatten();
                    if let Some(h) = visto {
                        m.insert("digest".into(), Json::s(format!("sha256:{h}")));
                    }
                }
                Respuesta::ok(r)
            }
            Err(p) => Respuesta {
                codigo: 404,
                cuerpo: p,
            },
        }
    }

    /// **El permiso de leer un ítem** (B3·2): el ítem, resuelto en el índice
    /// de esta transacción y fijado a su versión, se guarda con lo que hace
    /// falta para leerlo —de una virtual, la clave, la versión, el ETag y la
    /// credencial temporal que `ore-serve` trajo en `fuente`—, y vuelve un
    /// permiso opaco. `ttl_s` acota su vida (`ore-serve` no pide más de lo que
    /// le queda a la credencial).
    fn abrir(&self, ix: &Indice, n: &Node) -> Respuesta {
        let it = match self.buscar(ix, n) {
            Ok(it) => it,
            Err(p) => {
                return Respuesta {
                    codigo: 404,
                    cuerpo: p,
                };
            }
        };
        let pieza = match self.pieza(ix, it, texto(n, "fuente")) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let segundos = texto(n, "ttl_s")
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(permisos::VIDA_POR_DEFECTO);
        // B3·3: de una mantenida, los bytes los sirve el lago —el puesto llega a
        // Google— con una URL firmada, y no pasan por la celda. Si el almacén
        // no sabe firmar (uno local), por el permiso, como una virtual.
        if let Origen::Lago { sha256 } = &pieza.origen {
            let segundos = segundos.clamp(TTL_MINIMO, TTL_MAXIMO);
            let tipo = pieza
                .tipo
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into());
            let nombre = it.camino.rsplit('/').next().unwrap_or(&it.camino);
            let (_, disposicion) = ore_core::medios::disposicion(&tipo, nombre);
            let firmada = firma::firmar(
                &self.cuenta,
                &[Pedida {
                    blob: sha256.clone(),
                    tipo: Some(tipo),
                    disposicion: Some(disposicion),
                }],
                segundos,
            )
            .pop();
            if let Some(Ok(url)) = firmada {
                return Respuesta::ok(Json::obj([
                    ("url", Json::s(url)),
                    ("desde", Json::s("lago")),
                    ("ttl_s", Json::Int(segundos as i64)),
                    (
                        "expires_ms",
                        Json::Int(ahora_ms() + (segundos as i64) * 1000),
                    ),
                    ("version", Json::s(&it.version)),
                    ("item", ix.referencia(it)),
                ]));
            }
        }
        let Some((permiso, segundos)) = self.permisos.emitir(pieza, segundos) else {
            return problema(
                429,
                "media/limite",
                "demasiados permisos vivos en la celda: prueba en unos minutos",
            );
        };
        Respuesta::ok(Json::obj([
            ("permiso", Json::s(permiso)),
            ("desde", Json::s("medios")),
            ("ttl_s", Json::Int(segundos as i64)),
            (
                "expires_ms",
                Json::Int(ahora_ms() + (segundos as i64) * 1000),
            ),
            ("version", Json::s(&it.version)),
            ("item", ix.referencia(it)),
        ]))
    }

    /// **Lo que hay que leer de un ítem** (0049 B8·3): de una mantenida, lo que
    /// ya tiene su blob, del lago; lo que todavía no (una virtual que acaba de
    /// pasar a mantenida, y su copia está en camino), de su origen como una
    /// virtual, si `ore-serve` trajo la credencial. Mantenida no deja un ítem
    /// sin servir.
    fn pieza(&self, ix: &Indice, it: &Item, fuente: Option<&str>) -> Result<Pieza, Respuesta> {
        let del_origen = |fuente: &str| Origen::Objeto {
            fuente: fuente.to_string(),
            clave: it.clave.clone().unwrap_or_else(|| it.camino.clone()),
            version: it.version.clone(),
            etag: it.etag.as_deref().map(entre_comillas).unwrap_or_default(),
        };
        let origen = match (&it.blob, fuente) {
            (Some(b), _) if !ix.virtual_ => Origen::Lago { sha256: b.clone() },
            (_, Some(fuente)) => del_origen(fuente),
            (_, None) if ix.virtual_ => {
                return Err(problema(
                    400,
                    "media/peticion",
                    "una virtual se lee con la credencial de su fuente: falta `fuente`",
                ));
            }
            (_, None) => {
                return Err(problema(
                    404,
                    "media/no-existe",
                    "el ítem no tiene blob en el lago todavía (su copia está en camino) y no llegó la credencial de su origen",
                ));
            }
        };
        let sha256 = it
            .blob
            .clone()
            .or_else(|| self.vistos.de(&ix.coleccion, &it.camino, &it.version));
        Ok(Pieza {
            coleccion: ix.coleccion.clone(),
            camino: it.camino.clone(),
            version: it.version.clone(),
            origen,
            tamano: it.tamano.and_then(|t| u64::try_from(t).ok()),
            sha256,
            tipo: it.tipo.clone(),
        })
    }

    /// **`verify`** (0049 H4, `docs/media.md` §2): lee cada ítem entero, del lago
    /// o de su origen, calcula su `sha256` y lo compara con su digest; uno por
    /// posición, el error de uno en la suya. Sin digest conocido (una virtual
    /// que nadie leyó entera), se compara el tamaño y el digest calculado queda
    /// visto: el siguiente `stat` lo da (D-H4). Hasta [`LIMITE_VERIFY`] ítems.
    fn verificar(&self, ix: &Indice, n: &Node) -> Respuesta {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let pedidos = n.get("items").map(|(_, v)| v.items()).unwrap_or(&[]);
        if pedidos.is_empty() || pedidos.len() > LIMITE_VERIFY {
            return problema(
                413,
                "media/limite",
                format!(
                    "de 1 a {LIMITE_VERIFY} ítems por petición: `verify` lee los bytes enteros"
                ),
            );
        }
        let error = |r: Respuesta| Json::obj([("error", r.cuerpo)]);
        let mut resultados = Vec::with_capacity(pedidos.len());
        for p in pedidos {
            let it = match self.buscar(ix, p) {
                Ok(it) => it,
                Err(e) => {
                    resultados.push(Json::obj([("error", e)]));
                    continue;
                }
            };
            let pieza = match self.pieza(ix, it, texto(n, "fuente")) {
                Ok(p) => p,
                Err(r) => {
                    resultados.push(error(r));
                    continue;
                }
            };
            let comparado = if pieza.sha256.is_some() {
                "digest"
            } else if pieza.tamano.is_some() {
                "size"
            } else {
                "nada"
            };
            let mut abierto = match crate::contenido::abrir(
                self.cuenta.as_ref(),
                &pieza,
                None,
                self.vistos.clone(),
            ) {
                Ok(b) => b,
                Err(r) => {
                    resultados.push(error(r));
                    continue;
                }
            };
            // El lector ya comprueba al final (`Verificado`); aquí se calcula
            // también, para decir qué digest tienen los bytes que hay.
            let mut hash = Sha256::new();
            let mut buf = vec![0u8; 1 << 16];
            let fallo = loop {
                match abierto.lector.read(&mut buf) {
                    Ok(0) => break None,
                    Ok(k) => hash.update(&buf[..k]),
                    Err(e) => break Some(e),
                }
            };
            let visto = format!("sha256:{}", ore_s3::hex(&hash.finalize()));
            resultados.push(match fallo {
                None => Json::obj([
                    ("item", ix.referencia(it)),
                    ("ok", Json::Bool(true)),
                    ("digest", Json::s(&visto)),
                    ("comparado", Json::s(comparado)),
                ]),
                Some(e) if e.kind() == std::io::ErrorKind::InvalidData => Json::obj([
                    ("item", ix.referencia(it)),
                    ("ok", Json::Bool(false)),
                    ("digest", Json::s(&visto)),
                    ("error", problema_json(502, "media/corrupto", e.to_string())),
                ]),
                Some(e) => Json::obj([(
                    "error",
                    problema_json(502, "media/origen", format!("la lectura se cortó: {e}")),
                )]),
            });
        }
        Respuesta::ok(Json::obj([("results", Json::Arr(resultados))]))
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
        // ⭐ 0049 H3: lo que no está en el lago —una virtual, o una mantenida a
        //   medio copiar— se firma en su origen, con la credencial que trajo
        //   `ore-serve` (que ya recortó `ttl_s` a lo que le queda).
        let origen = texto(n, "fuente").map(crate::origenes::de);
        // Cada pedido, a su ítem o a su error; los que se firman, juntos.
        let mut salida: Vec<Option<Json>> = vec![None; pedidos.len()];
        let mut firmar: Vec<(usize, Pedida)> = Vec::new();
        // La referencia de cada uno va con su URL: quien pide sabe qué firmó.
        let mut firmar_ref: Vec<Option<Json>> = vec![None; pedidos.len()];
        for (i, p) in pedidos.iter().enumerate() {
            match self.buscar(ix, p) {
                Err(e) => salida[i] = Some(Json::obj([("error", e)])),
                Ok(it) if ix.virtual_ || it.blob.is_none() => {
                    salida[i] = Some(match &origen {
                        Some(Ok(o)) => firmada_en_el_origen(&**o, ix, it, segundos),
                        // Sin repetir la URL: lleva la credencial.
                        Some(Err(r)) => Json::obj([("error", r.cuerpo.clone())]),
                        None if ix.virtual_ => Json::obj([(
                            "error",
                            problema_json(
                                502,
                                "media/origen",
                                "un ítem de una virtual se firma en su origen, y no llegó la credencial de su fuente",
                            ),
                        )]),
                        None => Json::obj([(
                            "error",
                            problema_json(
                                404,
                                "media/no-existe",
                                "el ítem no tiene blob en el lago todavía (su copia está en camino) y no llegó la credencial de su origen",
                            ),
                        )]),
                    });
                }
                Ok(it) => {
                    let Some(blob) = it.blob.clone() else {
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

/// **La URL de un ítem en su origen** (0049 H3, ADR 0061 O0·3): la que firma
/// su proveedor, sin red, fijada a su versión, con el tipo y la disposición
/// dentro de la firma. Quien la tenga lee ese objeto en esa versión hasta que
/// caduque. Un origen que no firma lo dice en su posición (`501`).
fn firmada_en_el_origen(
    o: &dyn ore_objetos::Origen,
    ix: &Indice,
    it: &Item,
    segundos: u64,
) -> Json {
    let clave = it.clave.clone().unwrap_or_else(|| it.camino.clone());
    let tipo = it
        .tipo
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    let nombre = it.camino.rsplit('/').next().unwrap_or(&it.camino);
    let (_, disposicion) = ore_core::medios::disposicion(&tipo, nombre);
    let url = match o.firmar(&clave, &it.version, &tipo, &disposicion, segundos) {
        Ok(Some(u)) => u,
        Err(e) => {
            return Json::obj([(
                "error",
                problema_json(
                    502,
                    "media/origen",
                    format!("el origen no firmó la URL: {e}"),
                ),
            )]);
        }
        Ok(None) => {
            return Json::obj([(
                "error",
                problema_json(
                    501,
                    "media/origen",
                    "este ítem no tiene URL firmada —su origen no la da, o sólo se fija por su ETag, que una URL no lleva—: se abre por `content`",
                ),
            )]);
        }
    };
    Json::obj([
        ("item", ix.referencia(it)),
        ("url", Json::s(url)),
        (
            "expires_ms",
            Json::Int(ahora_ms() + (segundos as i64) * 1000),
        ),
        ("ttl_s", Json::Int(segundos as i64)),
    ])
}

/// Un ETag va entre comillas (RFC 9110 §8.8.3); el manifiesto puede no
/// guardarlas.
/// Lo que el puerto del puesto recibe en flujo: `PUT /subida` (B4b·1).
pub fn es_subida(p: &Peticion) -> bool {
    p.metodo == "PUT" && p.ruta == "/subida"
}

fn entre_comillas(e: &str) -> String {
    if e.starts_with('"') || e.starts_with("W/") {
        e.to_string()
    } else {
        format!("\"{e}\"")
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
            if ml == "rota" || ml == "recogida" {
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

        fn sigue(&self, _: &str, historica: &str, _: &str) -> Result<bool, String> {
            Ok(historica != "recogida")
        }
    }

    /// 0049 H1: una transacción de antes que la retención recogió es `404`; un
    /// lago que no contesta sigue siendo `502`, con o sin `actual`.
    #[test]
    fn lo_recogido_por_la_retencion_es_404_y_un_lago_roto_502() {
        let (s, _) = servicio();
        let pide = |ml: &str, actual: &str| {
            pedir(
                &s,
                "/indice/items",
                &format!(
                    r#"{{"coleccion":"legal.archivo.contratos","metadata_location":"{ml}","transaccion":"2"{actual}}}"#
                ),
            )
        };
        let (c, b) = pide("recogida", r#","actual":"m7""#);
        assert_eq!(c, 404, "{b}");
        assert!(
            b.contains("media/no-existe") && b.contains("retención"),
            "{b}"
        );
        let (c, b) = pide("rota", r#","actual":"m7""#);
        assert_eq!(c, 502, "{b}");
        let (c, b) = pide("recogida", "");
        assert_eq!(c, 502, "sin `actual` no es una de antes: {b}");
    }

    fn servicio() -> (Servicio, Arc<AtomicUsize>) {
        let n = Arc::new(AtomicUsize::new(0));
        (
            Servicio {
                listados: Box::new(Contador(n.clone())),
                cuenta: Arc::new(Firmante),
                indices: Indices::new_para_pruebas(),
                vistos: Arc::default(),
                permisos: Permisos::default(),
                escrituras: Default::default(),
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

    fn bytes(s: &Servicio, permiso: &str, rango: Option<&str>) -> (u16, Vec<u8>, String) {
        let mut cabeceras = BTreeMap::new();
        if let Some(r) = rango {
            cabeceras.insert("range".to_string(), r.to_string());
        }
        let mut consulta = BTreeMap::new();
        consulta.insert("permiso".to_string(), permiso.to_string());
        match s.atender_flujo(&Peticion {
            metodo: "GET".into(),
            ruta: "/contenido".into(),
            cabeceras,
            cuerpo: String::new(),
            consulta,
        }) {
            Salida::Bytes(mut b) => {
                let mut v = Vec::new();
                // Un cuerpo que no casa con su digest se corta: aquí, código 0.
                if std::io::Read::read_to_end(&mut b.lector, &mut v).is_err() {
                    return (0, v, String::new());
                }
                let cr = b
                    .cabeceras
                    .iter()
                    .find(|(k, _)| k == "content-range")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                (b.codigo, v, cr)
            }
            Salida::Una(r) => (r.codigo, r.cuerpo.jcs().into_bytes(), String::new()),
            Salida::Flujo(_) => panic!("un flujo de eventos"),
        }
    }

    fn permiso_de(b: &str) -> String {
        ore_core::parse::parse(b)
            .unwrap()
            .get("permiso")
            .and_then(|(_, v)| v.as_str())
            .unwrap()
            .to_string()
    }

    /// Una mantenida: el permiso guarda el blob, y `/contenido` da sus bytes
    /// enteros o por rango. Sin permiso, o con uno que no es, 401.
    #[test]
    fn el_permiso_abre_los_bytes_de_un_item_y_solo_los_suyos() {
        let (mut s, _) = servicio();
        let lago = crate::contenido::pruebas::Memoria::default();
        ore_store::almacen::Almacen::subir(&lago, &ore_store::blobs::clave_de("bb"), b"%PDF-c2")
            .unwrap();
        s.cuenta = Arc::new(lago);
        let (c, b) = pedir(
            &s,
            "/indice/abrir",
            &format!("{{{BASE},\"path\":\"docs/c.pdf\",\"ttl_s\":60}}"),
        );
        assert_eq!(c, 200, "{b}");
        assert!(
            b.contains("\"version\":\"v2\"") && b.contains("\"ttl_s\":60"),
            "la actual: {b}"
        );
        let p = permiso_de(&b);
        // El blob de prueba se llama `bb` y sus bytes no son ese sha256: la
        // lectura entera lo descubre al final y corta (B3·1).
        let (c, _, _) = bytes(&s, &p, None);
        assert_eq!(c, 0, "el digest no casa: se corta");
        let (c, v, cr) = bytes(&s, &p, Some("bytes=0-3"));
        assert_eq!((c, v.as_slice()), (206, &b"%PDF"[..]));
        // El total, el que dice el índice.
        assert!(cr.starts_with("bytes 0-3/"), "{cr}");
        let (c, v, _) = bytes(&s, "0123", None);
        assert_eq!(c, 401);
        assert!(String::from_utf8_lossy(&v).contains("media/permiso"));
        let (c, _) = pedir(
            &s,
            "/indice/abrir",
            &format!("{{{BASE},\"path\":\"docs/sin.pdf\"}}"),
        );
        assert_eq!(c, 404, "sin blob no hay qué abrir");
    }

    /// Una mantenida en un lago que sabe firmar: la URL del blob, sin permiso.
    #[test]
    fn una_mantenida_se_abre_con_la_url_de_su_blob() {
        let (s, _) = servicio();
        let (c, b) = pedir(
            &s,
            "/indice/abrir",
            &format!("{{{BASE},\"path\":\"docs/a.pdf\",\"ttl_s\":120}}"),
        );
        assert_eq!(c, 200, "{b}");
        assert!(
            b.contains("blobs/sha256/aa?vida=120") && b.contains("\"desde\":\"lago\""),
            "{b}"
        );
        assert!(!b.contains("permiso"), "{b}");
    }

    /// El puerto del puesto no sirve más que `/contenido`: ni el índice, ni la
    /// firma, ni los permisos.
    #[test]
    fn el_puerto_del_puesto_solo_sirve_contenido() {
        let (s, _) = servicio();
        for (m, r) in [
            ("POST", "/indice/items"),
            ("POST", "/indice/urls"),
            ("POST", "/indice/abrir"),
            ("POST", "/indice/verificar"),
            ("POST", "/indice/derivaciones"),
            ("GET", "/salud"),
        ] {
            let salida = s.atender_contenido(&Peticion {
                metodo: m.into(),
                ruta: r.into(),
                cabeceras: BTreeMap::new(),
                cuerpo: format!("{{{BASE},\"path\":\"docs/a.pdf\"}}"),
                consulta: BTreeMap::new(),
            });
            match salida {
                Salida::Una(x) => assert_eq!(x.codigo, 404, "{m} {r}"),
                _ => panic!("{m} {r} no debía servir nada"),
            }
        }
        let salida = s.atender_contenido(&Peticion {
            metodo: "GET".into(),
            ruta: "/contenido".into(),
            cabeceras: BTreeMap::new(),
            cuerpo: String::new(),
            consulta: BTreeMap::new(),
        });
        match salida {
            Salida::Una(x) => assert_eq!(x.codigo, 401, "sin permiso"),
            _ => panic!("sin permiso no hay bytes"),
        }
    }

    /// Una virtual sin la credencial de su fuente no se abre.
    #[test]
    fn una_virtual_pide_su_fuente() {
        let (s, _) = servicio();
        let (c, b) = pedir(
            &s,
            "/indice/abrir",
            &format!("{{{BASE},\"virtual\":\"true\",\"path\":\"docs/a.pdf\"}}"),
        );
        assert_eq!(c, 400, "{b}");
        assert!(b.contains("`fuente`"), "{b}");
        let (c, b) = pedir(
            &s,
            "/indice/abrir",
            &format!(
                "{{{BASE},\"virtual\":\"true\",\"path\":\"docs/a.pdf\",\"fuente\":\"s3://cubo?region=x&access_key_id=a&secret_access_key=b\"}}"
            ),
        );
        assert_eq!(c, 200, "{b}");
        assert!(!b.contains("secret"), "la credencial no vuelve: {b}");
    }

    /// 0049 B8·3: el ítem de una mantenida que todavía no tiene su blob (una
    /// virtual que acaba de pasar a mantenida) se abre de su origen, si
    /// llega la credencial; sin ella, 404 que dice por qué.
    #[test]
    fn lo_que_aun_no_esta_en_el_lago_se_abre_de_su_origen() {
        let (s, _) = servicio();
        let (c, b) = pedir(
            &s,
            "/indice/abrir",
            &format!("{{{BASE},\"path\":\"docs/sin.pdf\"}}"),
        );
        assert_eq!(c, 404, "{b}");
        assert!(b.contains("todavía"), "{b}");
        let (c, b) = pedir(
            &s,
            "/indice/abrir",
            &format!(
                "{{{BASE},\"path\":\"docs/sin.pdf\",\"fuente\":\"s3://cubo?region=x&access_key_id=a&secret_access_key=b\"}}"
            ),
        );
        assert_eq!(c, 200, "{b}");
        assert!(!b.contains("secret"), "la credencial no vuelve: {b}");
        assert!(!b.contains("\"desde\":\"lago\""), "{b}");
    }

    #[test]
    fn stat_da_el_sha256_visto_al_paso_si_no_lo_traia() {
        let (s, _) = servicio();
        let cuerpo = format!("{{{BASE},\"path\":\"docs/sin.pdf\"}}");
        let (_, b) = pedir(&s, "/indice/item", &cuerpo);
        assert!(b.contains("\"digest\":null"), "{b}");
        s.vistos
            .anotar("legal.archivo.contratos", "docs/sin.pdf", "v1", "ab12");
        let (_, b) = pedir(&s, "/indice/item", &cuerpo);
        assert!(b.contains("\"digest\":\"sha256:ab12\""), "{b}");
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

    /// 0049 H3: la URL de una virtual —y la de lo que una mantenida no tiene
    /// todavía en el lago— se firma en su origen: fijada a su versión, sin el
    /// secreto, con su vida; una fuente que no es S3 lo dice en su posición.
    #[test]
    fn la_url_de_lo_que_no_esta_en_el_lago_se_firma_en_su_origen() {
        const FUENTE: &str =
            "s3://cubo/?region=eu-north-1&access_key_id=AKIAEJEMPLO&secret_access_key=secreto";
        let pide = |virtual_: &str, fuente: &str, items: &str| {
            // Uno por pedido: el índice se guarda por `metadata_location`, con su clase.
            let (s, _) = servicio();
            let (c, b) = pedir(
                &s,
                "/indice/urls",
                &format!(
                    "{{{BASE},\"virtual\":\"{virtual_}\",\"fuente\":\"{fuente}\",\"ttl_s\":\"120\",\"items\":[{items}]}}"
                ),
            );
            assert_eq!(c, 200, "{b}");
            assert!(!b.contains("secreto"), "el secreto no sale: {b}");
            let j = ore_core::parse::parse(&b).unwrap();
            j.get("urls").unwrap().1.items().to_vec()
        };
        let url = |n: &ore_core::parse::Node| n.get("url").unwrap().1.as_str().unwrap().to_string();

        // una virtual: las tres, en su origen; la que no existe, su error en su sitio
        let u = pide(
            "true",
            FUENTE,
            r#"{"path":"docs/a.pdf"},{"path":"docs/no.pdf"},{"path":"docs/sin.pdf"}"#,
        );
        let a = url(&u[0]);
        assert!(
            a.starts_with("https://cubo.s3.eu-north-1.amazonaws.com/docs/a.pdf?"),
            "{a}"
        );
        for p in [
            "versionId=v1",
            "X-Amz-Expires=120",
            "X-Amz-Credential=AKIAEJEMPLO%2F",
            "response-content-type=",
            "X-Amz-Signature=",
        ] {
            assert!(a.contains(p), "{p} en {a}");
        }
        assert_eq!(u[0].get("ttl_s").unwrap().1.as_str(), Some("120"));
        assert_eq!(
            u[0].get("item").unwrap().1.get("path").unwrap().1.as_str(),
            Some("docs/a.pdf")
        );
        assert!(u[1].get("error").is_some());
        assert!(
            url(&u[2]).contains("docs/sin.pdf?"),
            "sin blob, también: es del origen"
        );

        // una mantenida: lo del lago, del lago; lo que no tiene blob, del origen
        let u = pide(
            "false",
            FUENTE,
            r#"{"path":"docs/a.pdf"},{"path":"docs/sin.pdf"}"#,
        );
        assert!(url(&u[0]).contains("blobs/sha256/aa"), "{}", url(&u[0]));
        assert!(url(&u[1]).starts_with("https://cubo.s3."), "{}", url(&u[1]));

        // una fuente que no se sabe servir: el error en su posición, sin
        // repetir su URL
        let u = pide("true", "ftp://u:secreto@otro/x", r#"{"path":"docs/a.pdf"}"#);
        let e = u[0].get("error").unwrap().1;
        assert_eq!(e.get("status").unwrap().1.as_str(), Some("501"));
    }

    /// 0049 H4: `verify` lee cada ítem entero y lo compara con su digest; el
    /// corrupto dice el digest que tienen sus bytes, el que no tiene blob su
    /// error, todo en su posición; más de 100, 413.
    #[test]
    fn verify_lee_entero_y_compara_con_el_digest() {
        use sha2::{Digest, Sha256};
        let bien = vec![b'a'; 77];
        let sha = |b: &[u8]| ore_s3::hex(&Sha256::digest(b));
        let (h_bien, h_malo) = (sha(&bien), sha(b"otro"));
        struct Uno(String, String);
        impl Listados for Uno {
            fn lotes(&self, _: &str, _: &str) -> Result<Vec<arrow_array::RecordBatch>, String> {
                Ok(vec![listado(&[
                    (
                        "docs/bien.pdf",
                        "v1",
                        "crc64nvme:A",
                        "actual",
                        Some(&self.0),
                    ),
                    (
                        "docs/malo.pdf",
                        "v1",
                        "crc64nvme:B",
                        "actual",
                        Some(&self.1),
                    ),
                    ("docs/ido.pdf", "v1", "crc64nvme:C", "actual", Some("ffff")),
                ])])
            }
        }
        let lago = crate::contenido::pruebas::Memoria::default();
        lago.subir(&ore_store::blobs::clave_de(&h_bien), &bien)
            .unwrap();
        // los bytes del blob de `malo`, otros (del mismo tamaño)
        lago.subir(&ore_store::blobs::clave_de(&h_malo), &[b'b'; 77])
            .unwrap();
        let s = Servicio {
            listados: Box::new(Uno(h_bien.clone(), h_malo.clone())),
            cuenta: Arc::new(lago),
            indices: Indices::new_para_pruebas(),
            vistos: Arc::default(),
            permisos: Permisos::default(),
            escrituras: Default::default(),
        };
        let (c, b) = pedir(
            &s,
            "/indice/verificar",
            &format!(
                "{{{BASE},\"items\":[{{\"path\":\"docs/bien.pdf\"}},{{\"path\":\"docs/malo.pdf\"}},{{\"path\":\"docs/ido.pdf\"}},{{\"path\":\"docs/no.pdf\"}}]}}"
            ),
        );
        assert_eq!(c, 200, "{b}");
        let j = ore_core::parse::parse(&b).unwrap();
        let r = j.get("results").unwrap().1.items();
        let campo = |i: usize, k: &str| {
            r[i].get(k)
                .map(|(_, v)| v.as_str().unwrap_or("").to_string())
        };
        assert_eq!(campo(0, "ok").as_deref(), Some("true"), "{b}");
        assert_eq!(campo(0, "digest"), Some(format!("sha256:{h_bien}")));
        assert_eq!(campo(0, "comparado").as_deref(), Some("digest"));
        assert_eq!(campo(1, "ok").as_deref(), Some("false"), "{b}");
        assert_eq!(
            campo(1, "digest"),
            Some(format!("sha256:{}", sha(&[b'b'; 77])))
        );
        assert!(b.contains("media/corrupto"), "{b}");
        assert!(
            r[2].get("error").is_some() && r[2].get("ok").is_none(),
            "{b}"
        );
        assert!(r[3].get("error").is_some(), "{b}");

        let muchos = vec![r#"{"path":"docs/bien.pdf"}"#; 101].join(",");
        let (c, _) = pedir(
            &s,
            "/indice/verificar",
            &format!("{{{BASE},\"items\":[{muchos}]}}"),
        );
        assert_eq!(c, 413);
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
    fn una_virtual_sin_su_credencial_no_firma_y_un_lago_roto_es_502() {
        let (s, _) = servicio();
        let (_, b) = pedir(
            &s,
            "/indice/urls",
            &format!("{{{BASE},\"virtual\":\"true\",\"items\":[{{\"path\":\"docs/a.pdf\"}}]}}"),
        );
        assert!(b.contains("no llegó la credencial"), "{b}");
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
