//! La cola de trabajo — encolar el catálogo de una fuente en el mismo acto del alta.
//!
//! # Por qué esto existe
//!
//! Medido el 2026-09-10: se da de alta una fuente a las 19:24 y el Job que lee el
//! origen no existe hasta las 20:17. No es lentitud — es que quien lo rendía era
//! la convergencia, y a la convergencia sólo la llamaba el cron.
//!
//! El webhook de la forja SÍ dispara al empujar el árbol, y el `Receiver` de Flux
//! reconcilia lo que lleve `ore.dev/rol: agente`. Pero eso apuntaba al
//! **compartimento**, que no cambia cuando se declara una fuente. Flux miraba,
//! veía lo mismo, y no hacía nada.
//!
//! ⇒ Con una cola propia, el alta la escribe, el webhook dispara, el `Receiver`
//!   la reconcilia y Flux crea el Job. Segundos, y ni un actor nuevo.
//!
//! # ⛔⛔ Y por qué NO se escribe en el compartimento
//!
//! Porque contiene el `Deployment` de este mismo proceso, sus `NetworkPolicy` y
//! a qué cuenta corre. Escribir ahí sería **el gobernado escribiendo su
//! gobierno**. La cola es un segundo repositorio con un segundo escritor, y su
//! `Kustomization` corre con una cuenta que sólo puede crear `Job`.
//!
//! # ⭐⭐ Y por qué aquí NO se renderiza de verdad
//!
//! El renderizado del inquilino —namespace, árbol, organización— vive en
//! `gen-inquilino.py` y tiene detrás una comprobación byte a byte. Copiarlo aquí
//! serían dos descripciones del mismo manifiesto, y la que se quedara vieja **no
//! daría error: daría un Job mal**.
//!
//! ⇒ El aprovisionador deja en la cola `plantilla-catalogo.txt`, ya rendida para
//!   ESTE inquilino y con el hueco de la fuente intacto. Aquí sólo se sustituyen
//!   dos cosas: el nombre de la fuente y el resumen del contenido.

use ore_core::digest;

/// Lo que el aprovisionador deja en la cola para que esto pueda encolar.
pub const PLANTILLA: &str = "plantilla-catalogo.txt";

/// La fuente y el resumen que la plantilla trae de fábrica, y que aquí se
/// sustituyen. Son los del fichero modelo de `malla/`, y si allí cambiaran esto
/// dejaría de sustituir nada — por eso la comprobación de más abajo los fija.
const FUENTE_MODELO: &str = "bq";
const RESUMEN_MODELO: &str = "00000000";

/// De un nombre de FUENTE al nombre de un objeto de Kubernetes.
///
/// ⚠️ Es la misma función que `gen-inquilino.py::nombre_de_objeto`, y eso es una
/// duplicación de verdad. Se acepta porque es **cerrada y comprobable**: no
/// depende del manifiesto ni crece con él, y las pruebas de abajo fijan los
/// mismos casos que las de allí. Lo que NO se duplica es el renderizado del
/// inquilino, que es lo que puede envejecer.
///
/// Cortado a 30 por la misma cuenta: `catalogo-` son 9, el resumen añade 9, y un
/// Job pone a sus pods otro sufijo de 6.
pub fn nombre_de_objeto(s: &str) -> String {
    let mut out = String::new();
    let mut guion = false;
    for c in s.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            guion = false;
        } else if c == '-' || !guion {
            // ⚠️ Una tirada de caracteres inválidos da UN guion, no uno por
            //   carácter: `Ventas..2024` es `ventas-2024`, no `ventas--2024`.
            out.push('-');
            guion = true;
        }
    }
    let n: String = out.trim_matches('-').chars().take(30).collect();
    let n = n.trim_matches('-').to_string();
    if n.is_empty() { "sin-nombre".into() } else { n }
}

/// El Job de catálogo de una fuente: cómo se llama el fichero y qué lleva dentro.
///
/// ⛔ El resumen va EL ÚLTIMO, sobre todo lo demás ya sustituido. Un Job es
///   inmutable: con el nombre derivado del contenido, «mismo nombre» implica
///   «mismo contenido» y el conflicto no puede darse. Es la misma regla que
///   `gen-inquilino.py`, y por eso los dos producen **el mismo nombre** para la
///   misma fuente — encolar dos veces no crea dos Jobs.
pub fn rendir(plantilla: &str, fuente: &str) -> Result<(String, String), String> {
    if !plantilla.contains(&format!("catalogo-{FUENTE_MODELO}-{RESUMEN_MODELO}")) {
        // ⛔ Se niega en vez de escribir algo que no sustituye nada. Un Job
        //   llamado `catalogo-bq-00000000` en el namespace de un cliente sería
        //   silencioso y estaría mal.
        return Err(format!(
            "`{PLANTILLA}` no trae el hueco `catalogo-{FUENTE_MODELO}-{RESUMEN_MODELO}`: \
             o no es la plantilla, o `malla/44-el-catalogo.yaml` cambió sin que esto se \
             enterara"
        ));
    }
    let obj = nombre_de_objeto(fuente);
    let t = plantilla.replace(
        &format!("value: \"{FUENTE_MODELO}\""),
        &format!("value: \"{fuente}\""),
    );
    let h = digest::de_bytes(t.as_bytes());
    // `de_bytes` devuelve `sha256:<64 hex>`; se toman los ocho primeros, que es
    // lo mismo que hace el renderizador con `hexdigest()[:8]`.
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let t = t.replace(
        &format!("catalogo-{FUENTE_MODELO}-{RESUMEN_MODELO}"),
        &format!("catalogo-{obj}-{h}"),
    );
    Ok((format!("44-el-catalogo-{obj}.yaml"), t))
}

#[cfg(test)]
mod prueba {
    use super::*;

    /// Los MISMOS casos que fija `gen-inquilino.py`. Si los dos dejan de
    /// coincidir, el aprovisionador y el alta encolarían Jobs con nombres
    /// distintos para la misma fuente — y habría dos.
    #[test]
    fn el_nombre_de_objeto_coincide_con_el_renderizador() {
        assert_eq!(nombre_de_objeto("bq"), "bq");
        assert_eq!(
            nombre_de_objeto("postgresql_20260910_074550"),
            "postgresql-20260910-074550"
        );
        assert_eq!(nombre_de_objeto("Ventas.2024"), "ventas-2024");
        assert_eq!(nombre_de_objeto("---"), "sin-nombre");
        assert_eq!(nombre_de_objeto(""), "sin-nombre");
        // Cortado a 30, y sin dejar un guion colgando al final.
        assert_eq!(nombre_de_objeto(&"a".repeat(60)).len(), 30);
    }

    /// ⛔ Una plantilla que no trae el hueco NO se rinde a medias.
    #[test]
    fn sin_hueco_se_niega() {
        assert!(rendir("apiVersion: batch/v1\nkind: Job\n", "bq").is_err());
    }

    #[test]
    fn el_nombre_lleva_el_resumen_y_es_determinista() {
        let p = "name: catalogo-bq-00000000\nenv:\n  - { name: FUENTE, value: \"bq\" }\n";
        let (f, a) = rendir(p, "ventas").unwrap();
        let (_, b) = rendir(p, "ventas").unwrap();
        assert_eq!(f, "44-el-catalogo-ventas.yaml");
        assert_eq!(a, b, "el mismo contenido tiene que dar el mismo nombre");
        assert!(a.contains("value: \"ventas\""));
        let n = a.lines().next().unwrap();
        assert!(n.starts_with("name: catalogo-ventas-"), "{n}");
        assert_eq!(n.len(), "name: catalogo-ventas-".len() + 8, "{n}");
        // Y una fuente distinta da un nombre distinto.
        let (_, c) = rendir(p, "compras").unwrap();
        assert!(c.contains("catalogo-compras-"));
    }
}
