//! v1alpha13 · **El schema**: el segundo nivel del nombre de lo que un paquete
//! tiene (`<paquete>.<schema>.<nombre>`), y la carpeta que se ata a él.
//!
//! La identidad nunca es la ruta (`90-canonical-form` §5.2): el schema lo
//! DECLARA el documento —`metadata.schema`, o `default`— y existe porque un
//! `kind: Schema` lo declara. Lo que esta fase comprueba es que la carpeta
//! esté de acuerdo, exactamente como `OOS2030` comprueba que el `namespace`
//! es el del paquete que contiene al documento:
//!
//! - `OOS2037`: el schema que un documento nombra está declarado en su paquete
//!   (o es `default`, que existe sin declararse).
//! - `OOS2036`: el documento vive dentro de la carpeta de su schema; uno en
//!   `default`, fuera de toda carpeta de schema; y un `Schema`, directamente
//!   en la carpeta que nombra.
//! - `OOS1004`: un `Schema` no se llama `default` ni `information_schema`.
//! - `OOS2009`: su `owner`, si lo lleva, es un handle.
//!
//! Solo mira documentos de v1alpha13 en adelante: en uno anterior el fichero
//! está en `default` diga lo que diga su carpeta, que es lo que significaba.
//! Y no mira lo importado de un `.oob`: su ruta es sintética (`<el
//! .oob>/<identidad>`), no hay carpeta con la que discrepar.
//!
//! Registro: `docs/decisions/0038-los-tres-niveles.md` · spec:
//! `vendor/oos/spec/v1alpha13/01-el-schema.md`.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{ApiVersion, Kind};
use crate::link::{Loaded, Package};
use crate::normalize::SCHEMA_POR_DEFECTO;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Los nombres que un `Schema` no puede tener: `default` existe sin
/// declararse, e `information_schema` es de SQL.
pub const RESERVADOS: &[&str] = &[SCHEMA_POR_DEFECTO, "information_schema"];

/// Los schemas declarados de cada miembro: (miembro, nombre) → su carpeta.
pub fn declarados<'a>(
    pkg: &'a Package,
    miembros: &'a [PathBuf],
) -> BTreeMap<(&'a Path, &'a str), PathBuf> {
    pkg.of(Kind::Schema)
        .filter_map(|s| {
            let m = crate::link::miembro_de(miembros, &s.path)?;
            let nombre = s.meta("name")?.as_str()?;
            Some(((m, nombre), m.join(nombre)))
        })
        .collect()
}

/// Un miembro importado de un `.oob`: el fichero ES su directorio, y lo de
/// dentro no tiene carpetas.
fn importado(m: &Path) -> bool {
    m.extension().is_some_and(|e| e == "oob")
}

/// Una carpeta como la escribe quien lee el diagnóstico: relativa al árbol y
/// con `/`, sea cual sea el sistema.
fn rel(pkg: &Package, p: &Path) -> String {
    p.strip_prefix(&pkg.root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn pos_de(d: &Loaded, clave: &str) -> crate::diag::Pos {
    d.root
        .get("metadata")
        .and_then(|(_, m)| m.get(clave))
        .map(|(k, _)| k.pos())
        .unwrap_or_else(|| d.root.pos())
}

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let miembros = crate::link::miembros(pkg);
    let mut out = Vec::new();
    let schemas = declarados(pkg, &miembros);

    // ── El documento `Schema` ──────────────────────────────────────────────
    for s in pkg.of(Kind::Schema) {
        let Some(nombre) = s.meta("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if RESERVADOS.contains(&nombre) {
            out.push(
                Diagnostic::new(
                    Code::Oos1004,
                    &s.path,
                    format!("un schema no puede llamarse `{nombre}`"),
                )
                .at(pos_de(s, "name"))
                .help(if nombre == SCHEMA_POR_DEFECTO {
                    "`default` existe en todo paquete sin declararse: es donde está lo que no dice \
                     otro schema. Para darle dueño a lo de `default`, lo lleva el paquete"
                } else {
                    "`information_schema` es el schema de metadatos de SQL, y un motor que lo \
                     lea no distinguiría el tuyo del suyo"
                }),
            );
        }
        if let Some(v) = s.section("owner")
            && !crate::pertenencia::es_handle(v.as_str().unwrap_or(""))
        {
            out.push(
                Diagnostic::new(
                    Code::Oos2009,
                    &s.path,
                    format!("`owner: {}` no es un handle", v.as_str().unwrap_or("")),
                )
                .at(v.pos())
                .help(
                    "usa `team:<handle>` o `user:<handle>`, como el paquete; sin `owner`, del \
                     schema responde el dueño del paquete",
                ),
            );
        }
        let Some(m) = crate::link::miembro_de(&miembros, &s.path) else {
            continue;
        };
        if importado(m) {
            continue;
        }
        let suya = m.join(nombre);
        if s.path.parent() != Some(suya.as_path()) {
            out.push(
                Diagnostic::new(
                    Code::Oos2036,
                    &s.path,
                    format!("el schema `{nombre}` no está en su carpeta"),
                )
                .at(pos_de(s, "name"))
                .help(format!(
                    "un `Schema` vive directamente en la carpeta que nombra —`{}/`—, y esa \
                     carpeta ES el schema: lo que haya dentro está en él",
                    rel(pkg, &suya)
                )),
            );
        }
    }

    // ── Lo del catálogo: su schema existe, y su carpeta es la suya ─────────
    for d in pkg.docs.iter().filter(|d| d.kind.con_schema()) {
        if d.version().is_none_or(|v| v < ApiVersion::V1Alpha13) {
            continue;
        }
        let Some(m) = crate::link::miembro_de(&miembros, &d.path) else {
            continue; // en la raíz: no hay paquete que tenga schemas
        };
        if importado(m) {
            continue;
        }
        let schema = d.schema().unwrap_or(SCHEMA_POR_DEFECTO);
        if schema == SCHEMA_POR_DEFECTO {
            // `default` es el paquete: fuera de toda carpeta de schema.
            if let Some(((_, otro), _)) = schemas
                .iter()
                .find(|((mm, _), carpeta)| *mm == m && d.path.starts_with(carpeta))
            {
                out.push(
                    Diagnostic::new(
                        Code::Oos2036,
                        &d.path,
                        format!("este documento vive en la carpeta del schema `{otro}` y está en `default`"),
                    )
                    .at(pos_de(d, "name"))
                    .help(format!(
                        "o declara `metadata.schema: {otro}` —y entonces se llama \
                         `<paquete>.{otro}.<nombre>`—, o se saca de esa carpeta. La carpeta y \
                         el nombre tienen que decir lo mismo"
                    )),
                );
            }
            continue;
        }
        let Some(carpeta) = schemas.get(&(m, schema)) else {
            out.push(
                Diagnostic::new(
                    Code::Oos2037,
                    &d.path,
                    format!("el paquete no declara el schema `{schema}`"),
                )
                .at(pos_de(d, "schema"))
                .help(format!(
                    "un schema existe porque un `kind: Schema` lo declara, en su carpeta \
                     (`{}/schema.yaml`). Sin `metadata.schema` el documento está en `default`",
                    rel(pkg, &m.join(schema))
                )),
            );
            continue;
        };
        if !d.path.starts_with(carpeta) {
            out.push(
                Diagnostic::new(
                    Code::Oos2036,
                    &d.path,
                    format!("este documento está en el schema `{schema}` y no vive en su carpeta"),
                )
                .at(pos_de(d, "schema"))
                .help(format!(
                    "lo de un schema vive dentro de su carpeta —`{}/`, a cualquier \
                     profundidad—: el nombre lo dice el documento y la carpeta tiene que estar \
                     de acuerdo, como con el paquete (`OOS2030`)",
                    rel(pkg, carpeta)
                )),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(ruta: &str, kind: Kind, texto: &str) -> Loaded {
        Loaded {
            path: PathBuf::from(ruta),
            kind,
            root: crate::parse::parse(texto).expect("analiza"),
        }
    }

    fn paquete(docs: Vec<Loaded>) -> Package {
        Package {
            root: PathBuf::from("."),
            docs,
            cedar: Vec::new(),
            generated: Vec::new(),
            sobres: Vec::new(),
        }
    }

    const PKG: &str = "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 1.0.0, status: active, domain: v }\nspec: { owner: team:v }\n";
    const ESPANA: &str = "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: espana, namespace: ventas }\n";

    fn ds(schema: Option<&str>, version: &str) -> String {
        format!(
            "apiVersion: oos.dev/{version}\nkind: Dataset\nmetadata: {{ name: pedidos, namespace: ventas{} }}\nspec: {{ owner: team:v, columns: {{ id: {{ type: Integer }} }}, changes: {{ mode: append }} }}\n",
            schema.map(|s| format!(", schema: {s}")).unwrap_or_default()
        )
    }

    fn codigos(docs: Vec<Loaded>) -> Vec<Code> {
        check(&paquete(docs)).into_iter().map(|d| d.code).collect()
    }

    #[test]
    fn lo_de_un_schema_vive_en_su_carpeta_y_lo_de_default_fuera() {
        let base = || {
            vec![
                doc("packages/ventas/package.yaml", Kind::Package, PKG),
                doc("packages/ventas/espana/schema.yaml", Kind::Schema, ESPANA),
            ]
        };
        // en su carpeta, a cualquier profundidad: bien
        let mut d = base();
        d.push(doc(
            "packages/ventas/espana/datasets/pedidos.yaml",
            Kind::Dataset,
            &ds(Some("espana"), "v1alpha13"),
        ));
        assert!(codigos(d).is_empty());
        // fuera de su carpeta: OOS2036
        let mut d = base();
        d.push(doc(
            "packages/ventas/datasets/pedidos.yaml",
            Kind::Dataset,
            &ds(Some("espana"), "v1alpha13"),
        ));
        assert_eq!(codigos(d), vec![Code::Oos2036]);
        // un schema que nadie declara: OOS2037
        let mut d = base();
        d.push(doc(
            "packages/ventas/francia/pedidos.yaml",
            Kind::Dataset,
            &ds(Some("francia"), "v1alpha13"),
        ));
        assert_eq!(codigos(d), vec![Code::Oos2037]);
        // en `default` dentro de la carpeta de un schema: OOS2036
        let mut d = base();
        d.push(doc(
            "packages/ventas/espana/pedidos.yaml",
            Kind::Dataset,
            &ds(None, "v1alpha13"),
        ));
        assert_eq!(codigos(d), vec![Code::Oos2036]);
        // en `default` en el paquete: bien
        let mut d = base();
        d.push(doc(
            "packages/ventas/datasets/pedidos.yaml",
            Kind::Dataset,
            &ds(None, "v1alpha13"),
        ));
        assert!(codigos(d).is_empty());
        // uno de v1alpha12 en la carpeta de un schema: la carpeta no significa nada
        let mut d = base();
        d.push(doc(
            "packages/ventas/espana/pedidos.yaml",
            Kind::Dataset,
            &ds(None, "v1alpha12"),
        ));
        assert!(codigos(d).is_empty());
    }

    #[test]
    fn el_schema_vive_en_la_carpeta_que_nombra_y_no_se_llama_default() {
        let pkg = || doc("packages/ventas/package.yaml", Kind::Package, PKG);
        assert_eq!(
            codigos(vec![
                pkg(),
                doc("packages/ventas/otra/schema.yaml", Kind::Schema, ESPANA)
            ]),
            vec![Code::Oos2036]
        );
        assert_eq!(
            codigos(vec![
                pkg(),
                doc("packages/ventas/espana/x/schema.yaml", Kind::Schema, ESPANA)
            ]),
            vec![Code::Oos2036]
        );
        let default = "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: default, namespace: ventas }\n";
        assert_eq!(
            codigos(vec![
                pkg(),
                doc("packages/ventas/default/schema.yaml", Kind::Schema, default)
            ]),
            vec![Code::Oos1004]
        );
        let dueno = "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: espana, namespace: ventas }\nspec: { owner: cambiame }\n";
        assert_eq!(
            codigos(vec![
                pkg(),
                doc("packages/ventas/espana/schema.yaml", Kind::Schema, dueno)
            ]),
            vec![Code::Oos2009]
        );
    }
}
