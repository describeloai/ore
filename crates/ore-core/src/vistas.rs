//! Las vistas: la pieza que absorbe al `Binding`.
//!
//! Una vista dice **qué existe físicamente y cómo se llama**: de dónde sale
//! —una fuente declarada, u **otra vista**—, qué campos expone, qué filas son
//! suyas, qué sabe hacer el origen y, si se copia, dónde. No lleva significado:
//! `is:`, los conceptos y las etiquetas siguen en la entidad (`v1alpha7/01-view`
//! §2). Y la flecha se invierte: el binding nombraba a la entidad, y ahora **la
//! entidad nombra a la vista** con `backedBy`. Así una vista existe antes de que
//! nadie modele nada, y varias entidades pueden respaldarse de la misma sin
//! duplicarla.
//!
//! Este módulo es lo que el resto del núcleo necesita saber de una vista sin
//! abrir su documento: **su raíz** —a qué fuente y objeto llega una cadena de
//! vistas, y con qué nombres de columna— y **sus comprobaciones**. Lo que no
//! está aquí es el álgebra: el IR, el linaje por columna y el reescritor viven
//! en `ore-view`, que depende de este crate y no al revés.
//!
//! # Lo que se comprueba, y con qué código
//!
//! | | |
//! |---|---|
//! | `from.datasource` o `materialized.datasource` sin declarar | `OOS2004` — el mismo que para `datasourceRef`, porque es el mismo defecto |
//! | `from.view`, `backedBy`, un campo o un filtro que nombran lo que la vista de abajo no expone | `OOS2018` |
//! | una cadena de vistas que vuelve sobre sí misma | `OOS2019` |
//! | la vista que respalda una entidad no expone su clave o sus `via` | `OOS2011` — lo que necesita columna, dicho de la vista |
//!
//! El flujo de etiquetas atraviesa la cadena en `flow`: la entidad hereda del
//! datasource **raíz** de su vista, y una vista con `materialized` instancia el
//! conducto `materialization.payload` como lo hacía el eje `payload` del binding.

use std::collections::{BTreeMap, BTreeSet};

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package};
use crate::normalize::qualify;
use crate::parse::Node;

/// De dónde sale una vista: de una fuente declarada, o de otra vista.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fuente {
    Datasource { datasource: String, objeto: String },
    Vista(String),
}

/// La hoja de una cadena de vistas, ya compuesta: **a qué fuente y objeto se
/// llega**, y con qué nombre físico se pide cada campo de la vista de arriba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raiz {
    pub datasource: String,
    pub objeto: String,
    /// Campo de la vista → columna física en la raíz. La composición de los
    /// renombres de toda la cadena.
    pub columnas: BTreeMap<String, String>,
    /// Los filtros de **toda** la cadena, ya en columnas físicas de la raíz:
    /// `(columna, valores)`. Una vista sobre otra hereda las filas que la de
    /// abajo ya recortó — lo que no está en la de abajo no está en ninguna.
    pub filtros: Vec<(String, Vec<String>)>,
}

/// Por qué una cadena no llega a una raíz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinRaiz {
    /// `from.view` nombra una vista que no existe. Lleva la que la nombró.
    NoExiste { vista: String, desde: String },
    /// La cadena vuelve sobre sí misma. La cadena entera, para que el mensaje
    /// la enseñe.
    Ciclo(Vec<String>),
    /// Una vista sin `from` que resuelva. El esquema lo impide; esto es lo que
    /// pasa si se llega aquí sin haberlo validado.
    SinFrom(String),
}

impl Package {
    /// La vista con este nombre cualificado.
    pub fn view(&self, qname: &str) -> Option<&Loaded> {
        self.of(Kind::View)
            .find(|d| d.qname().as_deref() == Some(qname))
    }

    /// Resuelve una referencia a vista **tal como la escribió el autor**: la
    /// forma corta vale dentro del mismo espacio de nombres (N1), igual que
    /// para una entidad.
    pub fn resolve_view(&self, referencia: &str, desde: &Loaded) -> Option<&Loaded> {
        let ns = desde.meta("namespace").and_then(|n| n.as_str());
        self.view(&qualify(referencia, ns))
    }
}

/// `spec.from` de una vista.
pub fn fuente(v: &Loaded) -> Option<Fuente> {
    let from = v.section("from")?;
    if let Some((_, vista)) = from.get("view") {
        let nombre = vista.as_str()?;
        let ns = v.meta("namespace").and_then(|n| n.as_str());
        return Some(Fuente::Vista(qualify(nombre, ns)));
    }
    let datasource = from.get("datasource")?.1.as_str()?.to_string();
    let objeto = from
        .get("object")
        .and_then(|(_, o)| o.as_str())
        .unwrap_or("")
        .to_string();
    Some(Fuente::Datasource { datasource, objeto })
}

/// Campo → nombre en la fuente. Admite la forma breve y la expandida, como el
/// mapeo del binding: la canónica es la expandida.
pub fn campos(v: &Loaded) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(fs) = v.section("fields") else {
        return out;
    };
    for (k, val) in fs.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let col = val.as_str().map(str::to_string).or_else(|| {
            val.get("column")
                .and_then(|(_, c)| c.as_str())
                .map(str::to_string)
        });
        if let Some(col) = col {
            out.insert(nombre.to_string(), col);
        }
    }
    out
}

/// `spec.where` de una vista: `(nombre en la fuente, valores)`. La gramática es
/// la del `selector` del binding —igualdad, pertenencia, ausencia— y por lo
/// mismo: un predicado sobre una columna clasificada es un canal lateral, y
/// una partición solo revela pertenencia.
pub fn filtros(v: &Loaded) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let Some(w) = v.section("where") else {
        return out;
    };
    for (k, val) in w.entries() {
        let Some(col) = k.as_str() else { continue };
        let valores: Vec<String> = match val {
            Node::Sequence { items, .. } => items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect(),
            _ => val.as_str().map(str::to_string).into_iter().collect(),
        };
        out.push((col.to_string(), valores));
    }
    out
}

/// La entidad nombra a su vista: `spec.backedBy`, resuelta.
pub fn respaldo<'a>(pkg: &'a Package, e: &Loaded) -> Option<&'a Loaded> {
    let r = e.section("backedBy")?.as_str()?;
    pkg.resolve_view(r, e)
}

/// La cadena de una vista hasta su raíz, en orden: ella primero.
///
/// Es la operación que hace que *«un pipeline es una cadena de vistas»* sea
/// una estructura: componer renombres y filtros no necesita un concepto nuevo,
/// solo seguir `from.view` hasta que deje de haberlo.
pub fn cadena<'a>(pkg: &'a Package, v: &'a Loaded) -> Result<Vec<&'a Loaded>, SinRaiz> {
    let mut vistos: Vec<String> = Vec::new();
    let mut fila: Vec<&Loaded> = Vec::new();
    let mut actual = v;
    loop {
        let qn = actual.qname().unwrap_or_default();
        if vistos.contains(&qn) {
            vistos.push(qn);
            return Err(SinRaiz::Ciclo(vistos));
        }
        vistos.push(qn.clone());
        fila.push(actual);
        match fuente(actual) {
            None => return Err(SinRaiz::SinFrom(qn)),
            Some(Fuente::Datasource { .. }) => return Ok(fila),
            Some(Fuente::Vista(otra)) => match pkg.view(&otra) {
                Some(n) => actual = n,
                None => {
                    return Err(SinRaiz::NoExiste {
                        vista: otra,
                        desde: qn,
                    });
                }
            },
        }
    }
}

/// La raíz de una vista: fuente, objeto, columnas compuestas y filtros.
///
/// Un campo que en algún eslabón no resuelve **no aparece** en `columnas`: la
/// comprobación de que resuelva es de `comprobar`, con `OOS2018`, y aquí no se
/// inventa una columna para lo que no tiene.
pub fn raiz(pkg: &Package, v: &Loaded) -> Result<Raiz, SinRaiz> {
    let fila = cadena(pkg, v)?;
    let hoja = fila.last().expect("una cadena tiene al menos un eslabón");
    let Some(Fuente::Datasource { datasource, objeto }) = fuente(hoja) else {
        unreachable!("la cadena termina en una fuente por construcción")
    };

    // De abajo arriba: la hoja nombra columnas físicas; cada eslabón de encima
    // nombra campos del de abajo, y se sustituyen.
    let mut columnas: BTreeMap<String, String> = campos(hoja);
    let mut filtros_fisicos: Vec<(String, Vec<String>)> = filtros(hoja);
    for eslabon in fila.iter().rev().skip(1) {
        let de_abajo = columnas;
        columnas = campos(eslabon)
            .into_iter()
            .filter_map(|(campo, en_fuente)| de_abajo.get(&en_fuente).map(|c| (campo, c.clone())))
            .collect();
        for (campo, valores) in filtros(eslabon) {
            if let Some(c) = de_abajo.get(&campo) {
                filtros_fisicos.push((c.clone(), valores));
            }
        }
    }
    Ok(Raiz {
        datasource,
        objeto,
        columnas,
        filtros: filtros_fisicos,
    })
}

/// Las fuentes físicas de una entidad: las de sus bindings y la raíz de su
/// vista. Es lo que `governance` necesita para `OOS8005` y lo que `flow`
/// necesita para heredar la ubicación — y las dos deben verlo igual.
pub fn datasources_de(pkg: &Package, e: &Loaded) -> BTreeSet<String> {
    let qn = e.qname().unwrap_or_default();
    let mut out: BTreeSet<String> = pkg
        .of(Kind::Binding)
        .filter(|b| {
            b.section("targetEntity")
                .and_then(|t| t.as_str())
                .map(|t| qualify(t, b.meta("namespace").and_then(|n| n.as_str())))
                .as_deref()
                == Some(qn.as_str())
        })
        .filter_map(|b| b.section("datasourceRef").and_then(|d| d.as_str()))
        .map(String::from)
        .collect();
    if let Some(v) = respaldo(pkg, e)
        && let Ok(r) = raiz(pkg, v)
    {
        out.insert(r.datasource);
    }
    out
}

/// A qué campo de `objetivo` llega cada campo de `desde`, siguiendo la cadena
/// hacia abajo. `None` si `objetivo` no está en la cadena de `desde`.
///
/// Es lo que hace que una etiqueta puesta en una entidad **viaje hasta la
/// vista que se materializa**: la entidad nombra campos de su vista, la vista
/// los renombra de la de abajo, y la de abajo es la que se copia.
pub fn proyectar(
    pkg: &Package,
    desde: &Loaded,
    objetivo: &str,
) -> Option<BTreeMap<String, String>> {
    let fila = cadena(pkg, desde).ok()?;
    let pos = fila
        .iter()
        .position(|v| v.qname().as_deref() == Some(objetivo))?;
    // Identidad en `desde`, y se compone bajando hasta `objetivo`.
    let mut mapa: BTreeMap<String, String> = campos(desde)
        .keys()
        .map(|k| (k.clone(), k.clone()))
        .collect();
    for eslabon in &fila[..pos] {
        let renombres = campos(eslabon);
        mapa = mapa
            .into_iter()
            .filter_map(|(origen, actual)| {
                renombres.get(&actual).map(|abajo| (origen, abajo.clone()))
            })
            .collect();
    }
    Some(mapa)
}

// ── Enlazado ────────────────────────────────────────────────────────────────

fn datasources_declarados(pkg: &Package) -> BTreeSet<String> {
    pkg.of(Kind::OntologyConfig)
        .filter_map(|c| c.section("datasources"))
        .flat_map(|n| n.items())
        .filter_map(|it| {
            it.get("name")
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        })
        .collect()
}

fn no_declarado(v: &Loaded, nodo: &Node, campo: &str, declarados: &BTreeSet<String>) -> Diagnostic {
    let r = nodo.as_str().unwrap_or("");
    Diagnostic::new(
        Code::Oos2004,
        &v.path,
        format!("`{campo}: {r}` no está declarado en el manifiesto raíz"),
    )
    .at(nodo.pos())
    .help(if declarados.is_empty() {
        "el manifiesto no declara ningún datasource".to_string()
    } else {
        format!(
            "declarados: {}",
            declarados.iter().cloned().collect::<Vec<_>>().join(" · ")
        )
    })
}

fn no_expone(
    path: &std::path::Path,
    nodo: &Node,
    que: String,
    vista: &str,
    expone: &BTreeMap<String, String>,
) -> Diagnostic {
    Diagnostic::new(Code::Oos2018, path, que)
        .at(nodo.pos())
        .help(if expone.is_empty() {
            format!("`{vista}` no expone ningún campo")
        } else {
            format!(
                "`{vista}` expone: {}",
                expone.keys().cloned().collect::<Vec<_>>().join(" · ")
            )
        })
}

/// Las comprobaciones de enlazado de las vistas y de `backedBy`.
pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let declarados = datasources_declarados(pkg);

    for v in pkg.of(Kind::View) {
        let qn = v.qname().unwrap_or_default();
        let Some(from) = v.section("from") else {
            continue;
        };

        // OOS2004 · la fuente, declarada. El mismo código que `datasourceRef`
        // porque es exactamente el mismo defecto con otro nombre de campo.
        if let Some((_, ds)) = from.get("datasource")
            && !declarados.contains(ds.as_str().unwrap_or(""))
        {
            out.push(no_declarado(v, ds, "from.datasource", &declarados));
        }
        if let Some((_, ds)) = v.section("materialized").and_then(|m| m.get("datasource"))
            && !declarados.contains(ds.as_str().unwrap_or(""))
        {
            out.push(no_declarado(v, ds, "materialized.datasource", &declarados));
        }

        // OOS2018 · la vista de abajo existe, y expone lo que esta le pide.
        // OOS2019 · y la cadena no vuelve sobre sí misma.
        if let Some((_, nodo)) = from.get("view") {
            let referencia = nodo.as_str().unwrap_or("");
            let Some(abajo) = pkg.resolve_view(referencia, v) else {
                out.push(
                    Diagnostic::new(
                        Code::Oos2018,
                        &v.path,
                        format!("`from.view: {referencia}` no existe"),
                    )
                    .at(nodo.pos())
                    .help(
                        "una vista sobre otra necesita que la otra esté en el paquete o en \
                         una dependencia. Resolver un nombre exige el paquete entero: es lo \
                         que un esquema JSON no alcanza",
                    ),
                );
                continue;
            };
            match cadena(pkg, v) {
                Err(SinRaiz::Ciclo(c)) => {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2019,
                            &v.path,
                            format!(
                                "la cadena de vistas vuelve sobre sí misma: {}",
                                c.join(" → ")
                            ),
                        )
                        .at(nodo.pos())
                        .help(
                            "una vista se define por lo que tiene debajo, y una que se tiene a \
                             sí misma debajo no se define. Ninguna de las de la cadena tiene \
                             raíz, así que ninguna se puede leer",
                        ),
                    );
                    continue;
                }
                Err(_) => continue,
                Ok(_) => {}
            }
            let expone = campos(abajo);
            let abajo_qn = abajo.qname().unwrap_or_default();
            if let Some(fs) = v.section("fields") {
                for (k, val) in fs.entries() {
                    let Some(campo) = k.as_str() else { continue };
                    let en_fuente = campos(v).get(campo).cloned().unwrap_or_default();
                    if !expone.contains_key(&en_fuente) {
                        out.push(no_expone(
                            &v.path,
                            val,
                            format!("`{qn}.{campo}` lee `{en_fuente}`, que `{abajo_qn}` no expone"),
                            &abajo_qn,
                            &expone,
                        ));
                    }
                }
            }
            if let Some(w) = v.section("where") {
                for (k, _) in w.entries() {
                    let Some(campo) = k.as_str() else { continue };
                    if !expone.contains_key(campo) {
                        out.push(no_expone(
                            &v.path,
                            k,
                            format!("`{qn}` filtra por `{campo}`, que `{abajo_qn}` no expone"),
                            &abajo_qn,
                            &expone,
                        ));
                    }
                }
            }
        }

        // OOS2018 · el testigo por campo nombra un campo de la vista.
        if let Some(ver) = v.section("version")
            && let Some((_, f)) = ver.get("field")
        {
            let campo = f.as_str().unwrap_or("");
            let expone = campos(v);
            if !expone.contains_key(campo) {
                out.push(no_expone(
                    &v.path,
                    f,
                    format!("`{qn}` declara `version.field: {campo}`, que no está en `fields`"),
                    &qn,
                    &expone,
                ));
            }
        }
    }

    // `backedBy` · la entidad nombra a su vista.
    for e in pkg.entities() {
        let Some(b) = e.section("backedBy") else {
            continue;
        };
        let referencia = b.as_str().unwrap_or("");
        let qn = e.qname().unwrap_or_default();
        let Some(v) = pkg.resolve_view(referencia, e) else {
            out.push(
                Diagnostic::new(
                    Code::Oos2018,
                    &e.path,
                    format!("`backedBy: {referencia}` no existe"),
                )
                .at(b.pos())
                .help(
                    "la entidad nombra a la vista que la respalda, y no al revés: la vista \
                     tiene que existir antes. Es lo que permite descubrir y exponer una \
                     fuente antes de modelar nada sobre ella",
                ),
            );
            continue;
        };
        let expone = campos(v);
        let vista_qn = v.qname().unwrap_or_default();

        // OOS2011 · lo que necesita columna: la clave y los `via`. La misma
        // regla del binding, dicha de la vista.
        let mut exigidas: Vec<(String, &Node)> = Vec::new();
        if let Some(k) = e.section("primaryKey") {
            for i in k.items() {
                if let Some(p) = i.as_str() {
                    exigidas.push((p.to_string(), i));
                }
            }
        }
        if let Some(rels) = e.section("relations") {
            for (_, rv) in rels.entries() {
                if let Some((_, via)) = rv.get("via") {
                    for i in via.items() {
                        if let Some(p) = i.as_str() {
                            exigidas.push((p.to_string(), i));
                        }
                    }
                }
            }
        }
        for (p, nodo) in exigidas {
            if !expone.contains_key(&p) {
                out.push(
                    Diagnostic::new(
                        Code::Oos2011,
                        &e.path,
                        format!("`{vista_qn}` no expone `{p}`, que `{qn}` necesita como columna"),
                    )
                    .at(nodo.pos())
                    .help(
                        "sin la clave no hay resolución de instancia, ni índice de topología, \
                         ni recurso identificable en una política; sin la columna de un enlace, \
                         la relación se declara y no se puede recorrer. Añade el campo a la \
                         vista o quítalo de la entidad",
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use std::path::PathBuf;

    fn doc(kind: Kind, texto: &str) -> Loaded {
        Loaded {
            path: PathBuf::from(format!("{}.yaml", kind.as_str())),
            kind,
            root: parse(texto).expect("yaml"),
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

    fn config() -> Loaded {
        doc(
            Kind::OntologyConfig,
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: x, version: 0.1.0 }\n\
             datasources:\n  - { name: erp, type: postgres, connectionEnv: ERP_URL }\n",
        )
    }

    fn vista(nombre: &str, from: &str, fields: &str, extra: &str) -> Loaded {
        doc(
            Kind::View,
            &format!(
                "apiVersion: oos.dev/v1alpha7\nkind: View\nmetadata: {{ name: {nombre}, namespace: hr }}\n\
                 spec:\n  owner: team:hr\n  from: {from}\n  version: {{ witness: none }}\n  fields:\n{fields}{extra}"
            ),
        )
    }

    fn base() -> Loaded {
        vista(
            "empleados",
            "{ datasource: erp, object: public.employees }",
            "    employeeId: employee_id\n    nationalId: { column: national_id, physicalType: varchar(16) }\n    pais: country\n",
            "  where: { deleted: 'false', country: [ES, PT] }\n",
        )
    }

    #[test]
    fn la_raiz_compone_renombres_y_filtros() {
        let iberia = vista(
            "iberia",
            "{ view: empleados }",
            "    id: employeeId\n    dni: nationalId\n",
            "  where: { pais: ES }\n",
        );
        let pkg = paquete(vec![config(), base(), iberia]);
        let r = raiz(&pkg, pkg.view("hr.iberia").unwrap()).unwrap();
        assert_eq!(r.datasource, "erp");
        assert_eq!(r.objeto, "public.employees");
        assert_eq!(
            r.columnas.get("id").map(String::as_str),
            Some("employee_id")
        );
        assert_eq!(
            r.columnas.get("dni").map(String::as_str),
            Some("national_id")
        );
        // `pais` no lo expone `iberia`: no aparece.
        assert!(!r.columnas.contains_key("pais"));
        // Los filtros de abajo se heredan y el de arriba llega en columna física.
        assert_eq!(
            r.filtros,
            vec![
                ("deleted".to_string(), vec!["false".to_string()]),
                (
                    "country".to_string(),
                    vec!["ES".to_string(), "PT".to_string()]
                ),
                ("country".to_string(), vec!["ES".to_string()]),
            ]
        );
    }

    #[test]
    fn proyectar_baja_los_nombres_hasta_la_vista_que_se_copia() {
        let iberia = vista(
            "iberia",
            "{ view: empleados }",
            "    id: employeeId\n    dni: nationalId\n",
            "",
        );
        let pkg = paquete(vec![config(), base(), iberia]);
        let m = proyectar(&pkg, pkg.view("hr.iberia").unwrap(), "hr.empleados").unwrap();
        assert_eq!(m.get("dni").map(String::as_str), Some("nationalId"));
        assert_eq!(m.get("id").map(String::as_str), Some("employeeId"));
        assert!(proyectar(&pkg, pkg.view("hr.empleados").unwrap(), "hr.iberia").is_none());
    }

    #[test]
    fn un_ciclo_es_oos2019_y_una_vista_ausente_oos2018() {
        let a = vista("a", "{ view: b }", "    x: x\n", "");
        let b = vista("b", "{ view: a }", "    x: x\n", "");
        let pkg = paquete(vec![config(), a, b]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert!(out.iter().any(|d| d.code == Code::Oos2019), "{out:?}");

        let suelta = vista("suelta", "{ view: nadie }", "    x: x\n", "");
        let pkg = paquete(vec![config(), suelta]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::Oos2018);
    }

    #[test]
    fn lo_que_la_de_abajo_no_expone_es_oos2018() {
        let iberia = vista(
            "iberia",
            "{ view: empleados }",
            "    salario: baseSalary\n",
            "  where: { ciudad: Vigo }\n",
        );
        let pkg = paquete(vec![config(), base(), iberia]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        let codigos: Vec<Code> = out.iter().map(|d| d.code).collect();
        assert_eq!(codigos, vec![Code::Oos2018, Code::Oos2018], "{out:?}");
    }

    #[test]
    fn la_fuente_sin_declarar_es_oos2004_con_las_dos_caras() {
        let v = vista(
            "v",
            "{ datasource: lago, object: t }",
            "    x: x\n",
            "  materialized: { datasource: otro, table: t2 }\n",
        );
        let pkg = paquete(vec![config(), v]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.code == Code::Oos2004));
    }

    #[test]
    fn backed_by_exige_la_clave_y_resuelve_en_corto() {
        let e = doc(
            Kind::Entity,
            "apiVersion: oos.dev/v1alpha7\nkind: Entity\nmetadata: { name: Employee, namespace: hr }\n\
             spec:\n  nature: entity\n  primaryKey: [employeeId]\n  backedBy: empleados\n\
             properties:\n    employeeId: { type: String }\n",
        );
        let pkg = paquete(vec![config(), base(), e]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert!(out.is_empty(), "{out:?}");

        let e2 = doc(
            Kind::Entity,
            "apiVersion: oos.dev/v1alpha7\nkind: Entity\nmetadata: { name: Employee, namespace: hr }\n\
             spec:\n  nature: entity\n  primaryKey: [id]\n  backedBy: empleados\n\
             properties:\n    id: { type: String }\n",
        );
        let pkg = paquete(vec![config(), base(), e2]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::Oos2011);
        assert_eq!(
            datasources_de(&pkg, pkg.entity("hr.Employee").unwrap()),
            BTreeSet::from(["erp".to_string()])
        );
    }

    #[test]
    fn el_testigo_por_campo_nombra_un_campo() {
        // A mano y no con el helper, que fija `witness: none`.
        let v = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha7\nkind: View\nmetadata: { name: v, namespace: hr }\n\
             spec:\n  owner: team:hr\n  from: { datasource: erp, object: t }\n  \
             version: { witness: field, field: updatedAt }\n  fields:\n    x: x\n",
        );
        let pkg = paquete(vec![config(), v]);
        let mut out = Vec::new();
        comprobar(&pkg, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].code, Code::Oos2018);
    }
}
