//! ⭐ **LAS PROMESAS DE UN ACTIVO, DESDE LA CONSOLA** (ADR 0063 S7f, OOS v1alpha31).
//!
//! Las promesas viven en el documento del activo (`spec.promises`), y `main` está
//! protegida: escribirlas es una propuesta, como aceptar un esquema (S5c).
//!
//! | | Qué |
//! |---|---|
//! | `POST /promesas` `{ref, promesas: [...]}` | **la lista entera** del activo —añadir, editar, ablandar y quitar son la misma llamada—: se valida con el compilador (lo que no compila es un 422 con sus diagnósticos, y no se crea nada); si compila, una rama, el documento reescrito (`promesas::reescribir`, a v1alpha31) y la propuesta. Sólo una persona |
//! | `GET /promesas/historia/{ref}` | lo que el juez dijo en los últimos refrescos, del historial del puntero en git: nada que guardar aparte |

use crate::rutas::Servidor;
use ore_core::document::Kind;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::Path;

/// Cuántos refrescos enseña la tendencia.
const HISTORIA: usize = 10;

impl Servidor {
    /// **`POST /promesas`**.
    pub(crate) fn proponer_promesas(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(_) => return Respuesta::error(422, "the body is not JSON"),
        };
        let Some(referencia) = n.get("ref").and_then(|(_, v)| v.as_str()) else {
            return Respuesta::error(
                422,
                "missing `ref`: the asset (`dataset:…` or `collection:…`)",
            );
        };
        let Some((_, lista)) = n.get("promesas") else {
            return Respuesta::error(
                422,
                "missing `promesas`: the whole list of the asset's promises",
            );
        };
        let promesas: Vec<Json> = lista.items().iter().map(Json::de_node_fiel).collect();
        let (kind, qn) = match activo(referencia) {
            Ok(x) => x,
            Err(m) => return Respuesta::error(422, m),
        };

        // ① El documento en `main`, y lo que sería: si no compila, aquí se para.
        let mut hallado: Result<(String, String, String), Respuesta> = Err(Respuesta::error(
            404,
            format!("`{referencia}` is not in the tree"),
        ));
        let _ = self.leyendo(|raiz| {
            hallado = documento(raiz, kind, &qn);
            Respuesta::ok(Json::obj([]))
        });
        let (ruta, texto, id) = match hallado {
            Ok(x) => x,
            Err(r) => return r,
        };
        let nuevo = ore_core::promesas::reescribir(&texto, &promesas);
        if nuevo == texto {
            return Respuesta::error(
                409,
                "these are already the asset's promises: nothing to propose",
            );
        }
        let diags = ore_core::validate::validate_document(Path::new(&ruta), &nuevo);
        if !diags.is_empty() {
            return Respuesta {
                codigo: 422,
                cuerpo: Json::obj([
                    (
                        "error",
                        Json::s("the promises do not compile: nothing was proposed"),
                    ),
                    (
                        "diagnosticos",
                        Json::Arr(
                            diags
                                .iter()
                                .map(|d| {
                                    let mut o = vec![
                                        ("codigo", Json::s(d.code.as_str())),
                                        ("mensaje", Json::s(&d.message)),
                                    ];
                                    if let Some(h) = &d.help {
                                        o.push(("ayuda", Json::s(h)));
                                    }
                                    if let Some(p) = d.pos {
                                        o.push(("linea", Json::Int(p.line as i64)));
                                    }
                                    Json::obj(o)
                                })
                                .collect(),
                        ),
                    ),
                ]),
            };
        }

        // ② La rama.
        let nombre = format!(
            "promesas-{}-{}",
            qn.replace(['.', '_'], "-"),
            crate::funciones::corrida_ahora().to_lowercase()
        );
        let creada = self.crear_rama(sujeto, &Json::obj([("nombre", Json::s(&nombre))]).jcs());
        if creada.codigo >= 300 {
            return creada;
        }
        let Some(rama) = texto_de(&creada.cuerpo, "rama") else {
            return Respuesta::error(502, "the branch was created and did not say its name");
        };

        // ③ En la rama: el documento de la rama, reescrito (puede no ser el de
        //   `main` si la rama nació después de que algo cambiara).
        let escrito = self.escribiendo_en(
            Some(&rama),
            sujeto,
            &format!("Promises of `{qn}` (0063 S7f)"),
            |raiz| match documento(raiz, kind, &qn) {
                Ok((ruta, t, _)) => {
                    let nuevo = ore_core::promesas::reescribir(&t, &promesas);
                    match std::fs::write(raiz.join(&ruta), nuevo) {
                        Ok(()) => Respuesta::ok(Json::obj([("ruta", Json::s(&ruta))])),
                        Err(e) => Respuesta::error(500, format!("could not write `{ruta}`: {e}")),
                    }
                }
                Err(r) => r,
            },
        );
        if escrito.codigo >= 300 || texto_de(&escrito.cuerpo, "commit").is_none() {
            let _ = self.retirar_rama(&rama);
            return if escrito.codigo >= 300 {
                escrito
            } else {
                Respuesta::error(409, "nothing changed in the branch: nothing to propose")
            };
        }

        // ④ La propuesta.
        let propuesta = self.proponer(
            sujeto,
            &Json::obj([
                ("rama", Json::s(&rama)),
                ("titulo", Json::s(format!("Promises of `{qn}`"))),
                ("descripcion", Json::s(descripcion(&qn, &promesas))),
                // Con alcance de ese activo (0044 E2): la propuesta lleva sólo
                // él, y *Assets changed* dice qué le cambia (sus promesas).
                ("activos", Json::Arr(vec![Json::s(&id)])),
            ])
            .jcs(),
        );
        if propuesta.codigo >= 300 {
            return propuesta;
        }
        Respuesta::creado(Json::obj([
            ("rama", Json::s(&rama)),
            ("ruta", Json::s(&ruta)),
            ("propuesta", propuesta.cuerpo),
        ]))
    }

    /// **`GET /promesas/historia/{ref}`**: por refresco, cuándo, cómo acabó y
    /// lo que el juez dijo de cada promesa, del más nuevo al más viejo.
    pub(crate) fn historia_de_promesas(&self, raiz: &Path, referencia: &str) -> Respuesta {
        let (_, qn) = match activo(referencia) {
            Ok(x) => x,
            Err(m) => return Respuesta::error(422, m),
        };
        // Por la puerta (0079 ·5b·3): lo heredado de `main` tiene la historia
        // de `main`, y la ruta de antes cuenta. Sin historia, ninguna.
        let corto = ore_core::normalize::a_corto(&qn);
        let historia = match crate::lugar::Lugar::de(raiz)
            .punteros()
            .historia(&corto, HISTORIA * 3)
        {
            Ok(h) => h,
            Err(ore_core::punteros::Fallo::Nombre(_)) => {
                return Respuesta::error(422, format!("`{qn}` is not `base.schema.name`"));
            }
            Err(_) => Vec::new(),
        };
        let mut refrescos: Vec<Json> = Vec::new();
        let mut visto: Option<String> = None;
        for v in &historia {
            if refrescos.len() >= HISTORIA {
                break;
            }
            let p = &v.puntero;
            let campo = |n: &ore_core::parse::Node, k: &str| {
                n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
            };
            let intento = p.get("intento").map(|(_, i)| i);
            let cuando = intento.and_then(|i| campo(i, "cuando"));
            // Un mismo intento puede quedar en varios commits (la recogida, una
            // comprobación): cuenta una vez.
            if cuando.is_none() || cuando == visto {
                continue;
            }
            visto = cuando.clone();
            let Some((_, ps)) = p.get("promesas") else {
                continue;
            };
            refrescos.push(Json::obj([
                ("cuando", Json::s(cuando.unwrap_or_default())),
                (
                    "resultado",
                    Json::s(
                        intento
                            .and_then(|i| campo(i, "resultado"))
                            .unwrap_or_default(),
                    ),
                ),
                ("promesas", Json::de_node_fiel(ps)),
            ]));
        }
        Respuesta::ok(Json::obj([
            ("ref", Json::s(referencia)),
            ("refrescos", Json::Arr(refrescos)),
        ]))
    }
}

/// `dataset:b.s.n` o `collection:b.s.n`: su `kind` y su nombre.
fn activo(referencia: &str) -> Result<(Kind, String), String> {
    let (k, qn) = referencia
        .split_once(':')
        .ok_or_else(|| format!("`{referencia}` is not `kind:name`"))?;
    let kind = match k {
        "dataset" => Kind::Dataset,
        "collection" => Kind::MediaCollection,
        _ => {
            return Err(format!(
                "a `{k}` has no promises: only a dataset or a collection is checked on every refresh"
            ));
        }
    };
    Ok((kind, qn.to_string()))
}

/// El documento del activo en `raiz`: su ruta relativa, su texto y su id de
/// activo (`Dataset:b.s.n`, el de una propuesta con alcance).
fn documento(raiz: &Path, kind: Kind, qn: &str) -> Result<(String, String, String), Respuesta> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let corto = ore_core::normalize::a_corto(qn);
    let d = pkg
        .docs
        .iter()
        .find(|d| {
            d.kind == kind
                && d.qname()
                    .is_some_and(|q| ore_core::normalize::a_corto(&q) == corto)
        })
        .ok_or_else(|| Respuesta::error(404, format!("`{qn}` is not in the tree")))?;
    let ruta = d
        .path
        .strip_prefix(raiz)
        .unwrap_or(&d.path)
        .to_string_lossy()
        .replace('\\', "/");
    let texto = std::fs::read_to_string(&d.path)
        .map_err(|e| Respuesta::error(500, format!("could not read `{ruta}`: {e}")))?;
    Ok((ruta, texto, ore_core::normalize::doc_id(d)))
}

fn texto_de(j: &Json, k: &str) -> Option<String> {
    match j {
        Json::Obj(m) => match m.get(k) {
            Some(Json::Str(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// La descripción de la propuesta: lo que se promete, una línea por promesa.
fn descripcion(qn: &str, promesas: &[Json]) -> String {
    if promesas.is_empty() {
        return format!(
            "Removes every written promise of `{qn}`. `notEmptied` stays: it is built in."
        );
    }
    let mut s = format!(
        "The promises of `{qn}`, checked on every refresh (OOS v1alpha31). A Blocking promise that fails holds the refresh; a Warning publishes and says it.\n"
    );
    for p in promesas {
        let campo = |k: &str| match p {
            Json::Obj(m) => m.get(k).map(|v| match v {
                Json::Str(x) => x.clone(),
                otro => otro.jcs(),
            }),
            _ => None,
        };
        let sev = if campo("severity").as_deref() == Some("warning") {
            "Warning"
        } else {
            "Blocking"
        };
        s.push_str(&format!(
            "\n- `{}` · {}{} · {sev}",
            campo("id").unwrap_or_default(),
            campo("metric").unwrap_or_else(|| "built in".into()),
            campo("column")
                .map(|c| format!(" of `{c}`"))
                .unwrap_or_default()
        ));
    }
    s
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_activo_de_una_ref() {
        assert_eq!(
            activo("dataset:vara_pg.ore_vara.pedidos").unwrap(),
            (Kind::Dataset, "vara_pg.ore_vara.pedidos".into())
        );
        assert_eq!(
            activo("collection:lago.fotos").unwrap().0,
            Kind::MediaCollection
        );
        assert!(activo("view:a.b").unwrap_err().contains("no promises"));
        assert!(activo("pedidos").is_err());
    }

    #[test]
    fn la_descripcion_dice_cada_promesa() {
        let ps = match Json::de_node_fiel(
            &ore_core::parse::parse(
                r#"[{"id":"volumen","metric":"rowCount","mustBe":1},{"id":"c","metric":"nullValues","column":"cliente","severity":"warning","mustBe":0}]"#,
            )
            .unwrap(),
        ) {
            Json::Arr(a) => a,
            _ => vec![],
        };
        let d = descripcion("vara_pg.ore_vara.pedidos", &ps);
        assert!(d.contains("\n- `volumen` · rowCount · Blocking"), "{d}");
        assert!(
            d.contains("\n- `c` · nullValues of `cliente` · Warning"),
            "{d}"
        );
        assert!(descripcion("x.y", &[]).starts_with("Removes every"));
    }
}
