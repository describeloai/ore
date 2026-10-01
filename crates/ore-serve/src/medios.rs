//! **La media de una celda, por su puerta** (ADR 0049 B2·2): las rutas del
//! contrato (`docs/media.md`) que `ore-serve` atiende y `ore-medios` sirve.
//!
//! | ruta | operación | qué hace aquí |
//! |---|---|---|
//! | `GET /media/{b}/{s}/{c}/items?prefix=&cursor=&limit=&estado=` | `list` | |
//! | `GET /media/{b}/{s}/{c}/item?path=&version=` o `?digest=` | `stat` | |
//! | `POST /media/{b}/{s}/{c}/urls` `{items, ttl_s}` | `url` | y lo anota en la actividad |
//!
//! Este proceso **decide y no sirve**: autentica (antes de llegar aquí), lee la
//! colección y su puntero en el árbol de la rama —el mismo `leyendo_en` de
//! siempre, con su `fetch` de 0,1 s—, y le pasa a `ore-medios` la colección, su
//! clase y el `metadata_location` de esa transacción. `ore-medios` guarda el
//! índice por ese `metadata_location` y firma con una cuenta viva. Así
//! `ore-serve` sigue sin TLS, sin Iceberg y sin poder leer un origen (lo
//! comprueba `ore-cli/tests/dependencias.rs`).
//!
//! Las rutas de 0046 (`/colecciones/…/items`, `…/items/{huella}`,
//! `…/items/resolver`) siguen hasta el relevo (0049 B6).

use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::{Peticion, Respuesta};
use std::collections::BTreeMap;
use std::path::Path;

/// Dónde escucha `ore-medios` en la celda (`host:puerto`). Sin él, las rutas
/// de la media dicen que el servicio no está desplegado.
pub const ENTORNO: &str = "ORE_MEDIOS_DIRECCION";

/// Lo que `ore-medios` tarda como mucho: cargar el índice de una colección de
/// un millón de ítems son ~2 s medidos (B2·0) más la descarga de su listado.
const PLAZO: std::time::Duration = std::time::Duration::from_secs(60);

fn problema(status: u16, tipo: &str, detalle: impl Into<String>) -> Respuesta {
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
    let mut ficheros = Vec::new();
    crate::documentos::yamls_de(&raiz.join("packages").join(b), &mut ficheros);
    for f in ficheros {
        let Ok(texto) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Ok(n) = ore_core::parse::parse(&texto) else {
            continue;
        };
        let campo = |n: &Node, k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        if campo(&n, "kind").as_deref() != Some("MediaCollection") {
            continue;
        }
        let Some((_, m)) = n.get("metadata") else {
            continue;
        };
        let schema = campo(m, "schema")
            .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
        if campo(m, "name").as_deref() == Some(c) && schema == s {
            let virtual_ = n
                .get("spec")
                .and_then(|(_, sp)| campo(sp, "virtual"))
                .is_some_and(|v| v == "true");
            return Some(virtual_);
        }
    }
    None
}

impl Servidor {
    /// `GET|POST /media/{b}/{s}/{c}/…`: la operación, en la rama que se lee.
    pub(crate) fn media(
        &self,
        rama: Option<&str>,
        p: &Peticion,
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
            pedido.insert(
                "metadata_location".into(),
                Json::s(de_puntero("metadata_location")),
            );
            pedido.insert("transaccion".into(), Json::s(de_puntero("transaccion")));
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
                "item" => {
                    for k in ["path", "version", "digest"] {
                        if let Some(v) = p.consulta.get(k) {
                            pedido.insert(k.into(), Json::s(v));
                        }
                    }
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

#[cfg(test)]
mod pruebas {
    use super::*;

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
