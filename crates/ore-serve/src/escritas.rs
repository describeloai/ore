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
            // ⭐ B4·4 · Y EL LINAJE, en el documento y en el mismo commit
            //   (v1alpha19 `01` §2): lo que esta transacción leyó —los `inputs`
            //   del transform, o lo que leyó la sesión— es su `derivedFrom`. Por
            //   ahí le baja la clasificación, que no se lee del puntero. Cada
            //   transacción lo reescribe; si no leyó nada, no lo lleva.
            let derivado = linaje_de(&procedencia, &corta, &completa);
            if let Some((f, _)) =
                crate::medios::fichero_y_documento(raiz, b, "MediaCollection", s, c)
                && let Ok(texto) = std::fs::read_to_string(&f)
                && let Some(nuevo) = con_linaje(&texto, &derivado)
                && nuevo != texto
                && let Err(e) = std::fs::write(&f, nuevo)
            {
                return problema(
                    500,
                    "media/origen",
                    format!("el linaje de la colección no se escribió: {e}"),
                );
            }
            let mut m = sellado;
            m.insert(
                "derivedFrom".into(),
                Json::Arr(derivado.iter().map(Json::s).collect()),
            );
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

/// **Lo que una transacción leyó** (B4·4): los `inputs` del transform que la
/// escribió, o lo que leyó la sesión (`leidas`). Sin repetidos, en orden, y sin
/// la colección misma (OOS2019).
fn linaje_de(procedencia: &Json, corta: &str, completa: &str) -> Vec<String> {
    let Json::Obj(m) = procedencia else {
        return Vec::new();
    };
    let lista = m.get("inputs").or_else(|| m.get("leidas"));
    let mut v: Vec<String> = match lista {
        Some(Json::Arr(a)) => a
            .iter()
            .filter_map(|x| match x {
                Json::Str(s) => Some(s.trim().to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty() && s != corta && s != completa)
            .collect(),
        _ => Vec::new(),
    };
    v.sort();
    v.dedup();
    v
}

/// **El documento con su `derivedFrom`**, sin reescribir lo demás: quita el
/// que tuviera (en bloque o en flujo), pone `derivado` como primera clave de
/// `spec` si no está vacío, y sube un v1alpha16–18 a v1alpha19 (la clave es de
/// v1alpha19: OOS1005). `None` si no hay un `spec:` de primer nivel.
fn con_linaje(texto: &str, derivado: &[String]) -> Option<String> {
    let lista = format!("[{}]", derivado.join(", "));
    let mut fuera = String::with_capacity(texto.len() + lista.len() + 24);
    let mut hecho = false;
    let mut saltando: Option<usize> = None; // la sangría de un `derivedFrom:` en bloque
    let lineas: Vec<&str> = texto.split_inclusive('\n').collect();
    for (i, l) in lineas.iter().enumerate() {
        let sin = l.trim_end_matches(['\n', '\r']);
        let sangria = sin.len() - sin.trim_start().len();
        // lo que quedaba de un `derivedFrom:` en bloque (`- x` más adentro)
        if let Some(s0) = saltando {
            if sin.trim().is_empty() || (sangria > s0 && sin.trim_start().starts_with('-')) {
                continue;
            }
            saltando = None;
        }
        let mut sin = sin.to_string();
        for v in ["v1alpha16", "v1alpha17", "v1alpha18"] {
            if sangria == 0
                && sin.starts_with("apiVersion:")
                && sin.contains(&format!("oos.dev/{v}"))
            {
                sin = sin.replace(&format!("oos.dev/{v}"), "oos.dev/v1alpha19");
            }
        }
        if sangria > 0 && sin.trim_start().starts_with("derivedFrom:") {
            if sin.trim_start()["derivedFrom:".len()..].trim().is_empty() {
                saltando = Some(sangria);
            }
            continue;
        }
        let Some(resto) = sin.strip_prefix("spec:").filter(|_| !hecho) else {
            fuera.push_str(&sin);
            fuera.push('\n');
            continue;
        };
        let resto = resto.trim();
        if resto.is_empty() || resto.starts_with('#') {
            let s = lineas[i + 1..]
                .iter()
                .map(|x| x.trim_end_matches(['\n', '\r']))
                .find(|x| !x.trim().is_empty())
                .map(|x| x.len() - x.trim_start().len())
                .filter(|n| *n > 0)
                .unwrap_or(2);
            fuera.push_str(&sin);
            fuera.push('\n');
            if !derivado.is_empty() {
                fuera.push_str(&format!("{}derivedFrom: {lista}\n", " ".repeat(s)));
            }
        } else {
            let dentro = resto.strip_prefix('{')?.trim_start();
            let dentro = sin_clave_en_flujo(dentro, "derivedFrom");
            let dentro = dentro.trim_start();
            match (derivado.is_empty(), dentro.strip_prefix('}')) {
                (true, _) => fuera.push_str(&format!("spec: {{ {dentro}\n")),
                (false, Some(tras)) => {
                    fuera.push_str(&format!("spec: {{ derivedFrom: {lista} }}{tras}\n"))
                }
                (false, None) => {
                    fuera.push_str(&format!("spec: {{ derivedFrom: {lista}, {dentro}\n"))
                }
            }
        }
        hecho = true;
    }
    if !texto.ends_with('\n') {
        fuera.pop();
    }
    hecho.then_some(fuera)
}

/// `k: [ … ]` (y su coma) fuera de un mapa en flujo, si está.
fn sin_clave_en_flujo(dentro: &str, k: &str) -> String {
    let Some(i) = dentro.find(&format!("{k}:")) else {
        return dentro.to_string();
    };
    let tras = &dentro[i..];
    let Some(abre) = tras.find('[') else {
        return dentro.to_string();
    };
    let Some(cierra) = tras[abre..].find(']') else {
        return dentro.to_string();
    };
    let mut fin = i + abre + cierra + 1;
    let resto = &dentro[fin..];
    let quitar_coma = resto.trim_start().starts_with(',');
    if quitar_coma {
        fin += resto.find(',').unwrap() + 1;
    }
    let mut antes = dentro[..i].to_string();
    if !quitar_coma {
        // era la última: la coma de antes sobra
        let t = antes.trim_end();
        if let Some(sin) = t.strip_suffix(',') {
            antes = sin.to_string() + " ";
        }
    }
    format!("{antes}{}", dentro[fin..].trim_start())
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

    /// B4·4 · El linaje: de los inputs o de lo leído, sin la colección misma;
    /// en bloque y en flujo; se reescribe y se quita; y v1alpha16 sube a 19.
    #[test]
    fn el_linaje_se_escribe_en_el_documento() {
        let p = Json::obj([
            ("transform", Json::s("t")),
            (
                "inputs",
                Json::Arr(vec![
                    Json::s("legal.archivo.contratos"),
                    Json::s("legal.archivo.paginas"),
                    Json::s("legal.archivo.contratos"),
                ]),
            ),
        ]);
        let d = linaje_de(&p, "legal.archivo.paginas", "legal.archivo.paginas");
        assert_eq!(d, ["legal.archivo.contratos"]);
        let sesion = Json::obj([("leidas", Json::Arr(vec![Json::s("b.s.x")]))]);
        assert_eq!(linaje_de(&sesion, "b.s.c", "b.s.c"), ["b.s.x"]);
        assert!(linaje_de(&Json::obj([]), "b.s.c", "b.s.c").is_empty());

        let bloque = "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: { name: paginas, namespace: legal, schema: archivo }\nspec:\n  owner: user:ana\n  media: image\n  formats: [png]\n";
        let a = con_linaje(bloque, &d).unwrap();
        assert!(a.starts_with("apiVersion: oos.dev/v1alpha19\n"), "{a}");
        assert!(
            a.contains("spec:\n  derivedFrom: [legal.archivo.contratos]\n  owner: user:ana\n"),
            "{a}"
        );
        // se reescribe: el de antes se va
        let b = con_linaje(&a, &["x.y.z".to_string()]).unwrap();
        assert!(
            b.contains("  derivedFrom: [x.y.z]\n") && !b.contains("contratos"),
            "{b}"
        );
        // en bloque como lista de guiones, también
        let guiones = "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nspec:\n  owner: user:ana\n  derivedFrom:\n    - a.b\n    - c.d\n  media: image\n";
        let g = con_linaje(guiones, &[]).unwrap();
        assert_eq!(
            g,
            "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nspec:\n  owner: user:ana\n  media: image\n"
        );
        // sin lecturas: se quita
        let c = con_linaje(&a, &[]).unwrap();
        assert!(!c.contains("derivedFrom"), "{c}");
        // en flujo
        let flujo = "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nspec: { owner: team:legal, media: image, formats: [png], derivedFrom: [legal.archivo.viejo] }\n";
        let f = con_linaje(flujo, &d).unwrap();
        assert_eq!(
            f,
            "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nspec: { derivedFrom: [legal.archivo.contratos], owner: team:legal, media: image, formats: [png] }\n"
        );
        let f0 = con_linaje(flujo, &[]).unwrap();
        assert_eq!(
            f0,
            "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nspec: { owner: team:legal, media: image, formats: [png] }\n"
        );
        assert!(con_linaje("kind: X\n", &d).is_none());
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
