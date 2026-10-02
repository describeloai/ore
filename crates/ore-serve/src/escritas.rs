//! **Escribir una colección, por su puerta** (ADR 0049 B4b·2): las rutas del
//! contrato (`docs/media.md` §2, `put`) que `ore-serve` decide y `ore-medios`
//! hace.
//!
//! | ruta | qué hace aquí |
//! |---|---|
//! | `POST /media/{b}/{s}/{c}/transactions {ttl_s?}` | decide si se puede y abre la transacción en `ore-medios`: `201 {transaction, upload, ttl_s, expires_ms}` |
//! | `POST /media/{b}/{s}/{c}/transactions/{t}/commit` | sella en `ore-medios` sobre el puntero de ahora y **escribe el puntero** en el árbol, con su procedencia, en un commit |
//! | `POST /media/{b}/{s}/{c}/transactions/{t}/abort` | `204`, y no queda nada |
//!
//! **Los bytes no pasan por aquí.** `upload` es la dirección de la subida en
//! `ore-medios` (`…:8098/subida?permiso=…`, el puerto al que llega el puesto):
//! el código sube cada ítem ahí con `PUT …&path=<camino>`. Es un **portador**
//! —quien la tenga sube a esa transacción mientras viva— y no se anota en
//! ningún sitio. Este servidor recibe cuerpos de texto de hasta 1 MiB, y un
//! PDF no lo es.
//!
//! **Quién puede.** Sólo una colección **escrita** (sin `from`): una mantenida
//! o una virtual es `409 media/no-escribible`. Desde un puesto —reconocido por
//! su agente, no por la cabecera, como al leer (B4·2)— la clase de su
//! repositorio puede quitar (`media/sin-permiso`), y dentro de un transform
//! sólo se escribe su `output` (`media/no-declarada`). Se comprueba al abrir y
//! otra vez al confirmar. Confirmar o abortar sólo lo hace quien abrió.
//!
//! **El puntero** (`datasets/<b>/<s>/<c>.json`, como el de una mantenida) lo
//! escribe este proceso, en la rama del puesto si la tiene, con:
//! `transaccion`, `metadata_location`, `items`, `cambios` y la
//! **procedencia** —el transform, sus `inputs` y las transacciones que fijó
//! (B4·2); o, en una sesión, las colecciones que leyó—. Dos confirmaciones a
//! la vez: la forja deja ganar a una y la otra recibe `409`; su transacción
//! **sigue abierta** (`ore-medios` no la cierra hasta que el puntero está
//! escrito) y se vuelve a confirmar, sobre la base nueva.
//!
//! ⚠️ En memoria, como en `ore-medios`: un reinicio olvida las abiertas.

use crate::medios::{direccion_del_contenido, es_escrita, pedir_a_medios, problema};
use crate::rutas::{Servidor, token};
use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::Identidad;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Una transacción abierta, y de quién es.
#[derive(Debug, Clone)]
pub(crate) struct Abierta {
    /// `b.s.c`, completa.
    pub coleccion: String,
    pub rama: Option<String>,
    /// El sujeto que la abrió: sólo él la confirma o la aborta.
    pub quien: String,
    /// Quien firma el commit del puntero: la persona del puesto, con su agente.
    pub escritor: Identidad,
    pub procedencia: Json,
    pub caduca: Instant,
}

/// Las transacciones abiertas por esta celda.
#[derive(Default)]
pub(crate) struct Escritas(Mutex<HashMap<String, Abierta>>);

impl Escritas {
    fn guardar(&self, t: &str, a: Abierta) {
        let ahora = Instant::now();
        let mut m = self.0.lock().unwrap();
        m.retain(|_, x| x.caduca > ahora);
        m.insert(t.to_string(), a);
    }

    /// La abierta `t`, si vive, es de `quien` y de `coleccion`.
    pub(crate) fn de(&self, t: &str, quien: &str, coleccion: &str) -> Option<Abierta> {
        let ahora = Instant::now();
        let m = self.0.lock().unwrap();
        m.get(t)
            .filter(|a| a.caduca > ahora && a.quien == quien && a.coleccion == coleccion)
            .cloned()
    }

    fn quitar(&self, t: &str) {
        self.0.lock().unwrap().remove(t);
    }
}

/// **El puntero de una escrita**: el de antes, con lo que el sello dijo encima
/// y la procedencia de esta transacción. Lo que es de una mantenida (testigo,
/// blobs bajados, lo no copiado) no va.
pub(crate) fn puntero_de_la_escrita(
    previo: Option<Json>,
    sellado: &BTreeMap<String, Json>,
    procedencia: Json,
    vista: &str,
) -> Json {
    let mut m = match previo {
        Some(Json::Obj(m)) => m,
        _ => BTreeMap::new(),
    };
    for k in ["motivo", "no_copiados", "testigo", "blobs", "techo"] {
        m.remove(k);
    }
    m.insert("kind".into(), Json::s("MediaCollection"));
    m.insert("virtual".into(), Json::Bool(false));
    m.insert("escrita".into(), Json::Bool(true));
    m.insert("estado".into(), Json::s("transaccion"));
    for k in [
        "transaccion",
        "metadata_location",
        "snapshot",
        "bytes",
        "ficheros",
        "items",
        "cambios",
        "dataset",
    ] {
        if let Some(v) = sellado.get(k) {
            m.insert(k.into(), v.clone());
        }
    }
    // El nombre en el lago, el de donde están los bytes (lo que se reclama).
    if let Some(Json::Str(ml)) = m.get("metadata_location")
        && let Some(d) = ore_core::punteros::dataset_de_ubicacion(ml)
    {
        m.insert("dataset".into(), Json::s(d));
    }
    m.insert("vista".into(), Json::s(vista));
    m.insert("procedencia".into(), procedencia);
    Json::Obj(m)
}

/// Un campo del puntero como texto (la transacción se guarda como número).
fn texto_de(m: &Option<Json>, k: &str) -> Option<String> {
    match m {
        Some(Json::Obj(m)) => match m.get(k) {
            Some(Json::Str(s)) if !s.is_empty() => Some(s.clone()),
            Some(Json::Int(i)) => Some(i.to_string()),
            _ => None,
        },
        _ => None,
    }
}

impl Servidor {
    /// Lo que deja escribir el puesto de quien pide, como un problema si no.
    fn quien_escribe(
        &self,
        sujeto: &Identidad,
        corta: &str,
    ) -> Result<Option<crate::puestos::EscrituraDelPuesto>, Respuesta> {
        self.escritura_del_puesto(sujeto, corta)
            .map_err(|(tipo, m)| problema(403, tipo, m))
    }

    /// `POST /media/{b}/{s}/{c}/transactions`.
    pub(crate) fn abrir_transaccion(
        &self,
        rama: Option<&str>,
        p: &Peticion,
        sujeto: &Identidad,
        b: &str,
        s: &str,
        c: &str,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(s)).and(token(c)) {
            return problema(422, "media/peticion", m);
        }
        let completa = format!("{b}.{s}.{c}");
        let corta = ore_core::normalize::a_corto(&completa).into_owned();
        let del_puesto = match self.quien_escribe(sujeto, &corta) {
            Ok(d) => d,
            Err(r) => return r,
        };
        // La rama del puesto, si tiene; si no, la que se pidió.
        let rama = del_puesto
            .as_ref()
            .and_then(|d| d.rama.clone())
            .or_else(|| rama.map(String::from));
        let r = self.leyendo_en(rama.as_deref(), |raiz| se_escribe(raiz, b, s, c));
        if r.codigo != 204 {
            return r;
        }
        let ttl = ore_core::parse::parse(&p.cuerpo)
            .ok()
            .and_then(|n| {
                n.get("ttl_s")
                    .and_then(|(_, v)| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| "3600".into());
        let (codigo, texto) = match pedir_a_medios(
            "/escritura/abrir",
            &Json::obj([("coleccion", Json::s(&completa)), ("ttl_s", Json::s(ttl))]),
        ) {
            Ok(x) => x,
            Err(r) => return r,
        };
        let n = match ore_core::parse::parse(texto.trim()) {
            Ok(n) if codigo == 201 => n,
            Ok(_) => {
                return Respuesta {
                    codigo,
                    cuerpo: Json::Crudo(texto.trim().to_string()),
                };
            }
            Err(_) => return problema(502, "media/origen", "`ore-medios` no contestó JSON"),
        };
        let campo = |k: &str| {
            n.get(k)
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let (t, permiso) = (campo("transaccion"), campo("permiso"));
        let segundos: u64 = campo("ttl_s").parse().unwrap_or(3600);
        let escritor = match &del_puesto {
            Some(d) => Identidad {
                persona: d.persona.clone(),
                agente: Some(sujeto.persona.clone()),
                correo: None,
                nombre: None,
                tipo: None,
                usuario: None,
            },
            None => sujeto.clone(),
        };
        self.escritas.guardar(
            &t,
            Abierta {
                coleccion: completa.clone(),
                rama,
                quien: sujeto.persona.clone(),
                escritor,
                procedencia: del_puesto
                    .map(|d| d.procedencia)
                    .unwrap_or_else(|| Json::obj([])),
                caduca: Instant::now() + Duration::from_secs(segundos),
            },
        );
        let direccion = std::env::var(crate::medios::ENTORNO).unwrap_or_default();
        Respuesta {
            codigo: 201,
            cuerpo: Json::obj([
                ("transaction", Json::s(&t)),
                ("collection", Json::s(&completa)),
                (
                    "upload",
                    Json::s(format!(
                        "http://{}/subida?permiso={permiso}",
                        direccion_del_contenido(&direccion)
                    )),
                ),
                ("ttl_s", Json::Int(segundos as i64)),
                (
                    "expires_ms",
                    Json::Int(crate::datasets::ahora_ms() as i64 + (segundos as i64) * 1000),
                ),
            ]),
        }
    }

    /// `POST /media/{b}/{s}/{c}/transactions/{t}/commit | abort`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cerrar_transaccion(
        &self,
        sujeto: &Identidad,
        b: &str,
        s: &str,
        c: &str,
        t: &str,
        confirmar: bool,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(s)).and(token(c)).and(token(t)) {
            return problema(422, "media/peticion", m);
        }
        let completa = format!("{b}.{s}.{c}");
        let corta = ore_core::normalize::a_corto(&completa).into_owned();
        let Some(abierta) = self.escritas.de(t, &sujeto.persona, &completa) else {
            return problema(
                404,
                "media/transaccion",
                format!(
                    "no hay ninguna transacción abierta `{t}` de `{completa}` que sea tuya (caducó, se cerró, o un reinicio)"
                ),
            );
        };
        if !confirmar {
            let _ = pedir_a_medios(
                "/escritura/abortar",
                &Json::obj([("transaccion", Json::s(t))]),
            );
            self.escritas.quitar(t);
            return Respuesta::sin_contenido();
        }
        // Lo declarado manda también al confirmar; y en una sesión, lo leído
        // hasta ahora.
        let procedencia = match self.quien_escribe(sujeto, &corta) {
            Ok(Some(d)) => d.procedencia,
            Ok(None) => abierta.procedencia.clone(),
            Err(r) => return r,
        };
        let procedencia = match procedencia {
            Json::Obj(mut m) => {
                m.insert("transaccion".into(), Json::s(t));
                m.insert("por".into(), Json::s(&abierta.escritor.persona));
                Json::Obj(m)
            }
            otra => otra,
        };
        let mensaje = format!("confirmar coleccion `{corta}` ({t})");
        let hacer = |raiz: &Path| {
            let r = se_escribe(raiz, b, s, c);
            if r.codigo != 204 {
                return r;
            }
            let dir = raiz.join("datasets");
            let previo =
                ore_core::punteros::leer_en(&dir, &completa).map(|(_, n)| Json::de_node(&n));
            let mut pedido: BTreeMap<String, Json> = BTreeMap::new();
            pedido.insert("transaccion".into(), Json::s(t));
            pedido.insert("coleccion".into(), Json::s(&completa));
            pedido.insert("cerrar".into(), Json::s("false"));
            for (k, a) in [
                ("metadata_location", "metadata_location"),
                ("transaccion", "base"),
                ("dataset", "dataset"),
            ] {
                if let Some(v) = texto_de(&previo, k) {
                    pedido.insert(a.into(), Json::s(v));
                }
            }
            let (codigo, texto) = match pedir_a_medios("/escritura/confirmar", &Json::Obj(pedido)) {
                Ok(x) => x,
                Err(r) => return r,
            };
            let sellado = match ore_core::parse::parse(texto.trim()).map(|n| Json::de_node(&n)) {
                Ok(Json::Obj(m)) if codigo == 200 => m,
                Ok(_) => {
                    return Respuesta {
                        codigo,
                        cuerpo: Json::Crudo(texto.trim().to_string()),
                    };
                }
                Err(_) => {
                    return problema(502, "media/origen", "`ore-medios` no contestó JSON");
                }
            };
            if sellado.get("sin_cambios") == Some(&Json::Bool(true)) {
                return Respuesta::ok(Json::Obj(sellado));
            }
            let puntero = puntero_de_la_escrita(previo, &sellado, procedencia.clone(), &corta);
            let Some(ruta) = ore_core::punteros::ruta_en(&dir, &completa) else {
                return problema(
                    422,
                    "media/peticion",
                    format!("`{completa}` no es un nombre"),
                );
            };
            if let Some(padre) = ruta.parent()
                && let Err(e) = std::fs::create_dir_all(padre)
            {
                return problema(
                    500,
                    "media/origen",
                    format!("el puntero no se escribió: {e}"),
                );
            }
            if let Err(e) = std::fs::write(&ruta, puntero.pretty() + "\n") {
                return problema(
                    500,
                    "media/origen",
                    format!("el puntero no se escribió: {e}"),
                );
            }
            ore_core::punteros::retirar_legado(&dir, &completa, &ruta);
            let mut m = sellado;
            m.insert("procedencia".into(), procedencia.clone());
            Respuesta::ok(Json::Obj(m))
        };
        let r = match abierta.rama.as_deref() {
            None => self.escribiendo(&abierta.escritor, &mensaje, hacer),
            Some(_) => {
                self.escribiendo_en(abierta.rama.as_deref(), &abierta.escritor, &mensaje, hacer)
            }
        };
        // Escrito el puntero (o nada que escribir), se cierra; si la forja
        // perdió la carrera, sigue abierta para volver a confirmar.
        if r.codigo < 300 {
            let _ = pedir_a_medios(
                "/escritura/abortar",
                &Json::obj([("transaccion", Json::s(t))]),
            );
            self.escritas.quitar(t);
        }
        r
    }
}

/// `204` si la colección existe y es escrita; si no, su problema.
fn se_escribe(raiz: &Path, b: &str, s: &str, c: &str) -> Respuesta {
    match es_escrita(raiz, b, s, c) {
        None => problema(
            404,
            "media/no-existe",
            format!("no hay ninguna colección `{b}.{s}.{c}`"),
        ),
        Some(false) => problema(
            409,
            "media/no-escribible",
            format!(
                "`{b}.{s}.{c}` se mantiene desde su `from`: la llena su origen, no el código. Se escribe una colección sin `from`"
            ),
        ),
        Some(true) => Respuesta::sin_contenido(),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_puntero_de_una_escrita_lleva_lo_sellado_y_su_procedencia() {
        let previo = Json::obj([
            ("kind", Json::s("MediaCollection")),
            ("transaccion", Json::Int(1)),
            ("testigo", Json::obj([("modo", Json::s("listing"))])),
            ("bundle", Json::s("sha256:b")),
        ]);
        let sellado: BTreeMap<String, Json> = [
            ("transaccion".to_string(), Json::Int(2)),
            (
                "metadata_location".to_string(),
                Json::s("gs://lago/ore/v2/colecciones/legal/archivo/paginas/metadata/2.json"),
            ),
            ("items".to_string(), Json::obj([("actuales", Json::Int(3))])),
            ("operacion".to_string(), Json::s("refrescada")),
        ]
        .into();
        let p = puntero_de_la_escrita(
            Some(previo),
            &sellado,
            Json::obj([("transform", Json::s("paginar"))]),
            "legal.archivo.paginas",
        );
        let Json::Obj(m) = p else { panic!() };
        assert_eq!(m.get("transaccion"), Some(&Json::Int(2)));
        assert_eq!(m.get("escrita"), Some(&Json::Bool(true)));
        assert_eq!(m.get("virtual"), Some(&Json::Bool(false)));
        assert!(!m.contains_key("testigo"), "de una mantenida, no");
        assert!(
            !m.contains_key("operacion"),
            "lo del sello que no es del puntero, no"
        );
        assert_eq!(
            m.get("bundle"),
            Some(&Json::s("sha256:b")),
            "lo de antes se queda"
        );
        assert_eq!(
            m.get("procedencia"),
            Some(&Json::obj([("transform", Json::s("paginar"))]))
        );
        assert_eq!(m.get("vista"), Some(&Json::s("legal.archivo.paginas")));
    }

    fn quien(p: &str) -> Identidad {
        Identidad {
            persona: p.into(),
            agente: None,
            correo: None,
            nombre: None,
            tipo: None,
            usuario: None,
        }
    }

    #[test]
    fn una_transaccion_es_de_quien_la_abrio_y_de_su_coleccion() {
        let e = Escritas::default();
        e.guardar(
            "t-1",
            Abierta {
                coleccion: "legal.archivo.paginas".into(),
                rama: None,
                quien: "agente:puesto-ana".into(),
                escritor: quien("persona:ana"),
                procedencia: Json::obj([]),
                caduca: Instant::now() + Duration::from_secs(60),
            },
        );
        assert!(
            e.de("t-1", "agente:puesto-ana", "legal.archivo.paginas")
                .is_some()
        );
        assert!(
            e.de("t-1", "agente:puesto-beto", "legal.archivo.paginas")
                .is_none()
        );
        assert!(
            e.de("t-1", "agente:puesto-ana", "legal.archivo.otra")
                .is_none()
        );
        e.guardar(
            "t-2",
            Abierta {
                coleccion: "legal.archivo.paginas".into(),
                rama: None,
                quien: "agente:puesto-ana".into(),
                escritor: quien("persona:ana"),
                procedencia: Json::obj([]),
                caduca: Instant::now() - Duration::from_secs(1),
            },
        );
        assert!(
            e.de("t-2", "agente:puesto-ana", "legal.archivo.paginas")
                .is_none(),
            "caducada"
        );
        e.quitar("t-1");
        assert!(
            e.de("t-1", "agente:puesto-ana", "legal.archivo.paginas")
                .is_none()
        );
    }

    #[test]
    fn solo_se_escribe_una_coleccion_sin_from() {
        let d = std::env::temp_dir().join(format!("ore-escritas-{}", std::process::id()));
        let dir = d.join("packages/legal/archivo/collections");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("paginas.yaml"),
            "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: paginas, namespace: legal, schema: archivo }\nspec: { owner: team:legal, media: image, formats: [png], derivedFrom: [legal.archivo.contratos] }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("contratos.yaml"),
            "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: { name: contratos, namespace: legal, schema: archivo }\nspec: { owner: team:legal, media: document, formats: [pdf], from: { objectTable: s3.docs.contratos }, virtual: true }\n",
        )
        .unwrap();
        assert_eq!(se_escribe(&d, "legal", "archivo", "paginas").codigo, 204);
        assert_eq!(se_escribe(&d, "legal", "archivo", "contratos").codigo, 409);
        assert_eq!(se_escribe(&d, "legal", "archivo", "nada").codigo, 404);
        let _ = std::fs::remove_dir_all(&d);
    }
}
