//! **El manifiesto de un paquete**, escrito por nosotros.
//!
//! Un emisor, y uno solo. Lo escriben `ore package new`, la inducción
//! (`ore discover`) y —desde 0035 ⑦.1— el **sitio de un proyecto**. Si hubiera
//! dos textos habría dos descripciones de la misma forma, y divergirían en el
//! caso que ninguna prueba ejerce: por eso esto vive en el núcleo y no en quien
//! lo escribe.
//!
//! ⛔ `owner` tiene que ser un handle (`team:<h>` o `user:<h>`, `OOS2009`):
//!   quien llame lo sabe ANTES, con [`crate::pertenencia::es_handle`]. Medido
//!   en 0035 ⑦.1 sobre el árbol de victor: con `owner: cambiame` el árbol deja
//!   de compilar (1 error), y con un handle sale **0 y nadie lo nombra**.

/// El `package.yaml` de un paquete: `name`, `version`, `status`, `domain` y
/// `owner`, en la forma canónica y en una línea por clave.
pub fn documento(nombre: &str, owner: &str, estado: &str, dominio: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Package\n\
         metadata: {{ name: {nombre}, version: 0.1.0, status: {estado}, domain: {dominio} }}\n\
         spec: {{ owner: \"{owner}\" }}\n"
    )
}
