//! ⭐ **EL DOCUMENTADOR** (0079 §19.3, E1a·5e): el `Dataset` escrito por máquina que no llegó a git,
//! llevado a git.
//!
//! Una escritura mixta (`crate::dos_fases`) escribe el puntero en el catálogo y el documento en git
//! después. Antes de intentar git, el documento se **encola en el catálogo** ([`encolar`], con lo
//! que había en git al calcularlo: `antes`); si git va bien, se quita ([`quitar`]); si no, se queda
//! y la respuesta dice `pendiente`. Un documento no se puede regenerar del puntero (sale de lo que
//! trajo la escritura: columnas, clave, linaje, dueño), así que se guarda tal cual.
//!
//! Una **pasada** ([`pasada`], al arrancar y cada [`CADENCIA`], sólo con el catálogo): por cada ref
//! con documentos en cola, un commit «Documentar: …» en su rama con lo último de cada ruta, y lo
//! publicado sale de la cola (sólo si no llegó uno más nuevo entretanto). Por cada documento,
//! según lo que hay en git ([`decidir`]):
//!
//! | En git | |
//! |---|---|
//! | lo que había al calcularlo (`antes`) | se escribe |
//! | ya es el documento | nada que escribir; sale de la cola |
//! | otra cosa: alguien lo cambió entretanto | **no se pisa**; sale de la cola y se dice. La siguiente escritura de esa tabla lo regenera sobre lo nuevo |
//!
//! Una rama que ya no está en la forja: sus documentos salen de la cola.
//!
//! ⚠️ Lo que queda abierto: si ore-serve cae entre escribir el puntero y encolar (milisegundos), ese
//!   documento se pierde, como antes de ·5e. Cerrarlo pide que el documento viaje en la misma
//!   transacción que el puntero (el protocolo de la puerta); anotado en el ADR.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use ore_catalogo::almacen::{Documento, EnCola};
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;

use crate::rutas::{Arbol, Servidor};

/// Cada cuánto se pasa.
pub(crate) const CADENCIA: Duration = Duration::from_secs(60);

/// **Encola** los documentos de una escritura en `rama` (`None`: `main`), con lo que había en el
/// árbol `raiz` (el de leer, del que se calcularon). `(ruta, n)` de cada uno, lo que pide
/// [`quitar`]; `None` sin catálogo, o si el catálogo no contesta (se dice: el documento queda como
/// antes de ·5e, a cargo de la fase 2).
pub(crate) fn encolar(
    rama: Option<&str>,
    raiz: &Path,
    docs: &[crate::dos_fases::Documento],
) -> Option<Vec<(String, i64)>> {
    let docs: Vec<Documento> = docs
        .iter()
        .map(|d| Documento {
            ruta: d.ruta.clone(),
            texto: d.texto.clone(),
            antes: std::fs::read_to_string(raiz.join(&d.ruta)).ok(),
        })
        .collect();
    match crate::lugar::en_el_catalogo(rama, |c| c.encolar_documentos(&docs))? {
        Ok(Ok(n)) => Some(n),
        Ok(Err(e)) | Err(e) => {
            eprintln!("⚠ 0079 ·5e: no se pudo encolar el documento: {e}");
            None
        }
    }
}

/// **Quita** de la cola de `rama` lo que ya está en git.
pub(crate) fn quitar(rama: Option<&str>, hechos: &[(String, i64)]) {
    if hechos.is_empty() {
        return;
    }
    let r = rama.unwrap_or("main").to_string();
    if let Some(Ok(Err(e)) | Err(e)) =
        crate::lugar::en_el_catalogo(None, |c| c.quitar_documentos(&r, hechos))
    {
        eprintln!("⚠ 0079 ·5e: no se pudo quitar de la cola: {e}");
    }
}

/// Qué hacer con un documento de la cola, según lo que hay en git.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Accion {
    Escribir,
    YaEsta,
    /// Alguien lo cambió desde que se calculó: no se pisa.
    Cambiado,
}

pub(crate) fn decidir(en_git: Option<&str>, doc: &Documento) -> Accion {
    if en_git == Some(doc.texto.as_str()) {
        Accion::YaEsta
    } else if en_git == doc.antes.as_deref() {
        Accion::Escribir
    } else {
        Accion::Cambiado
    }
}

/// Lo que hizo una pasada.
#[derive(Debug, Default)]
pub(crate) struct Informe {
    pub(crate) escritos: usize,
    pub(crate) ya: usize,
    pub(crate) cambiados: Vec<String>,
    pub(crate) sin_rama: usize,
    pub(crate) fallos: Vec<String>,
    /// Lo que más esperaba al empezar, en segundos.
    pub(crate) espera_max: f64,
}

impl Informe {
    /// Una línea para el registro, o nada si no hubo nada.
    pub(crate) fn linea(&self) -> Option<String> {
        if self.escritos + self.ya + self.sin_rama == 0
            && self.cambiados.is_empty()
            && self.fallos.is_empty()
        {
            return None;
        }
        Some(format!(
            "  documentador: {} escrito(s), {} ya estaban, {} de ramas que no están, {} cambiado(s) a mano {:?}, {} fallo(s) {:?} · el más viejo esperaba {:.0} s",
            self.escritos,
            self.ya,
            self.sin_rama,
            self.cambiados.len(),
            self.cambiados,
            self.fallos.len(),
            self.fallos,
            self.espera_max
        ))
    }
}

/// Quien firma los commits del documentador: este servidor, a nombre de nadie.
fn documentador() -> Identidad {
    Identidad {
        persona: "agente:documentador".into(),
        agente: Some("ore-serve".into()),
        correo: None,
        nombre: Some("documentador".into()),
        tipo: None,
        usuario: None,
    }
}

/// **Una pasada** sobre la cola.
pub(crate) fn pasada(s: &Servidor) -> Informe {
    let mut inf = Informe::default();
    let Arbol::Forja(forja) = &s.arbol else {
        return inf;
    };
    let cola = match crate::lugar::en_el_catalogo(None, |c| c.documentos_en_cola()) {
        None => return inf,
        Some(Ok(Ok(c))) => c,
        Some(Ok(Err(e)) | Err(e)) => {
            inf.fallos.push(format!("la cola: {e}"));
            return inf;
        }
    };
    inf.espera_max = cola.iter().map(|c| c.espera_s).fold(0.0, f64::max);
    let mut por_ref: BTreeMap<String, Vec<EnCola>> = BTreeMap::new();
    for c in cola {
        por_ref.entry(c.r.clone()).or_default().push(c);
    }
    for (r, docs) in por_ref {
        let rama = (r != "main").then_some(r.as_str());
        let hechos: Vec<(String, i64)> = docs.iter().map(|d| (d.doc.ruta.clone(), d.n)).collect();
        if let Some(b) = rama
            && forja.cabeza_de(b).is_none()
        {
            inf.sin_rama += docs.len();
            quitar(rama, &hechos);
            continue;
        }
        let mut escritos = Vec::new();
        let mut ya = 0;
        let mut cambiados = Vec::new();
        let escribir = |raiz: &Path| {
            for d in &docs {
                let f = raiz.join(&d.doc.ruta);
                let hoy = std::fs::read_to_string(&f).ok();
                match decidir(hoy.as_deref(), &d.doc) {
                    Accion::YaEsta => ya += 1,
                    Accion::Cambiado => cambiados.push(d.doc.ruta.clone()),
                    Accion::Escribir => {
                        let uno = crate::dos_fases::Documento {
                            ruta: d.doc.ruta.clone(),
                            texto: d.doc.texto.clone(),
                        };
                        if let Err(m) =
                            crate::dos_fases::escribir_documentos(raiz, std::slice::from_ref(&uno))
                        {
                            return Respuesta::error(500, m);
                        }
                        escritos.push(d.doc.ruta.clone());
                    }
                }
            }
            Respuesta::ok(Json::obj([("documentados", Json::Int(0))]))
        };
        let rutas: Vec<&str> = docs.iter().map(|d| d.doc.ruta.as_str()).collect();
        let mensaje = format!(
            "Documentar: {} (0079 ·5e, lo que el catálogo ya tenía)",
            rutas.join(", ")
        );
        let quien = documentador();
        let hecho = match rama {
            // `main`: lo de la celda (como una escritura mixta en `main`), sin la regla de
            // protegida: el documento es de máquina y su dato ya está.
            None => s.escribiendo(&quien, &mensaje, escribir),
            Some(b) => s.escribiendo_en(Some(b), &quien, &mensaje, escribir),
        };
        if hecho.codigo >= 300 {
            inf.fallos.push(format!(
                "{r}: {}",
                hecho.cuerpo.jcs().chars().take(200).collect::<String>()
            ));
            continue;
        }
        inf.escritos += escritos.len();
        inf.ya += ya;
        inf.cambiados.extend(cambiados);
        quitar(rama, &hechos);
    }
    inf
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn doc(texto: &str, antes: Option<&str>) -> Documento {
        Documento {
            ruta: "packages/v/datasets/a.yaml".into(),
            texto: texto.into(),
            antes: antes.map(String::from),
        }
    }

    /// No se pisa lo que alguien cambió; lo que ya está, no se escribe.
    #[test]
    fn se_escribe_solo_sobre_lo_que_habia() {
        assert_eq!(decidir(None, &doc("nuevo", None)), Accion::Escribir);
        assert_eq!(
            decidir(Some("viejo"), &doc("nuevo", Some("viejo"))),
            Accion::Escribir
        );
        assert_eq!(
            decidir(Some("nuevo"), &doc("nuevo", Some("viejo"))),
            Accion::YaEsta
        );
        assert_eq!(
            decidir(Some("a mano"), &doc("nuevo", Some("viejo"))),
            Accion::Cambiado
        );
        assert_eq!(
            decidir(Some("alguien"), &doc("nuevo", None)),
            Accion::Cambiado
        );
        assert_eq!(
            decidir(None, &doc("nuevo", Some("viejo"))),
            Accion::Cambiado,
            "lo borraron"
        );
    }
}
