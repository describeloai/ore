//! Traducción a y desde ODCS v3.1.0.
//!
//! # Por qué la ida y vuelta, y no solo la ida
//!
//! > Un perfil que solo restringe es una limitación. **Uno que hace ida y
//! > vuelta es interoperabilidad**, y esa es la razón de perfilar en lugar de
//! > inventar. — `01-package` §4
//!
//! Emitir es fácil y no demuestra nada: cualquier formato se puede escribir
//! perdiendo la mitad. Lo que certifica el perfil es que **`OOS → ODCS → OOS`
//! sea la identidad**, porque solo entonces adoptar OOS no es una puerta de
//! salida cerrada.
//!
//! # Lo que no se interpreta se conserva
//!
//! `price`, `contractCreatedTs`, References, Custom Properties ajenas: el perfil
//! no las entiende y **no debe fingir que sí**. Viajan literalmente por una
//! extensión, sin validarse (§4.3). `contractCreatedTs` es el caso
//! interesante — OOS **nunca** lo escribe, porque el invariante III prohíbe el
//! reloj; pero si viene, se conserva.

use crate::document::Kind;
use crate::json::Json;
use crate::link::Package;
use crate::normalize;
use std::collections::BTreeMap;

const ODCS_VERSION: &str = "v3.1.0";

/// La clave de extensión donde viaja lo que el perfil no interpreta. El
/// mecanismo es el de OOS (`x-<proveedor>-<lo que sea>`), y el proveedor es
/// `odcs` porque el contenido es de ODCS y de nadie más.
const PASSTHROUGH: &str = "x-odcs-passthrough";

/// Los documentos que ODCS **sí** modela. El resto viaja sin traducir.
const PROYECTADOS: &[&str] = &["Package:", "Entity:", "Binding:"];

/// Los campos de un DataContract que **sí** están en el perfil `Package`.
/// Cualquier otro es material que se conserva sin tocar.
const PERFIL: &[&str] = &[
    "apiVersion",
    "kind",
    "id",
    "name",
    "version",
    "status",
    "domain",
    "tenant",
    "tags",
    "description",
    "team",
    "roles",
    "support",
    "slaProperties",
    "authoritativeDefinitions",
    "customProperties",
    "servers",
    "schema",
];

fn obj(j: &Json) -> Option<&BTreeMap<String, Json>> {
    match j {
        Json::Obj(m) => Some(m),
        _ => None,
    }
}

fn cadena(m: &BTreeMap<String, Json>, k: &str) -> Option<String> {
    match m.get(k) {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

// ── OOS → ODCS ──────────────────────────────────────────────────────────────

/// Emite el paquete como contrato ODCS v3.1.0.
pub fn emit(pkg: &Package) -> Json {
    emit_canonical(&normalize::package(pkg))
}

/// `ODCS → OOS → ODCS`. Pasa **por la representación OOS**, que es exactamente
/// lo que la afirmación de fidelidad exige comprobar: si el perfil perdiera algo
/// al entrar, no habría forma de recuperarlo al salir.
pub fn reemit(contrato: &Json) -> Json {
    emit_canonical(&import(contrato))
}

/// Emite desde la forma canónica de un paquete. Las dos entradas —un paquete
/// leído del disco y uno importado de ODCS— llegan aquí indistinguibles, y esa
/// es la razón de que la ida y vuelta pueda ser la identidad.
pub fn emit_canonical(canonica: &BTreeMap<String, Json>) -> Json {
    let mut out: BTreeMap<String, Json> = BTreeMap::new();
    out.insert("apiVersion".into(), Json::s(ODCS_VERSION));
    out.insert("kind".into(), Json::s("DataContract"));

    let paquete = canonica
        .iter()
        .find(|(id, _)| id.starts_with("Package:"))
        .map(|(_, j)| j);

    if let Some(p) = paquete.and_then(obj) {
        let meta = p.get("metadata").and_then(obj);
        let spec = p.get("spec").and_then(obj);

        if let Some(m) = meta {
            for k in ["name", "version", "status", "domain", "tenant", "id"] {
                if let Some(v) = m.get(k) {
                    out.insert(k.to_string(), v.clone());
                }
            }
            for k in ["tags", "description"] {
                if let Some(v) = m.get(k) {
                    out.insert(k.to_string(), v.clone());
                }
            }
        }
        // `id` derivado si no era explícito: un contrato ODCS sin identidad no
        // se puede referenciar, y derivarla de nombre y versión la hace estable.
        //
        // Pero **derivar no es declarar**, y la diferencia hay que anotarla: sin
        // ella, la vuelta restauraría un `id` que el paquete original no tenía y
        // la ida y vuelta dejaría de ser la identidad por un campo que ORE se
        // inventó.
        let declarado = out.contains_key("id");
        if !declarado {
            let derivado = Json::s(format!(
                "{}:{}",
                cadena(&out, "name").unwrap_or_default(),
                cadena(&out, "version").unwrap_or_default()
            ));
            out.insert("id".into(), derivado);
        }

        if let Some(s) = spec {
            for k in ["team", "roles", "support", "authoritativeDefinitions"] {
                if let Some(v) = s.get(k) {
                    out.insert(k.to_string(), v.clone());
                }
            }
            if let Some(sla) = s.get("sla").and_then(obj) {
                out.insert("slaProperties".into(), sla_a_odcs(sla));
            }
            let mut custom = Vec::new();
            if !declarado {
                custom.push(propiedad("x-oos-idDerived", Json::Bool(true)));
            }
            if let Some(owner) = s.get("owner") {
                custom.push(propiedad("x-oos-owner", owner.clone()));
            }
            if let Some(deps) = s.get("dependencies") {
                custom.push(propiedad("x-oos-dependencies", deps.clone()));
            }
            if !custom.is_empty() {
                out.insert("customProperties".into(), Json::Arr(custom));
            }
        }
    }

    // ODCS modela tres de nuestros seis documentos. Los otros tres —el
    // manifiesto de workspace, los retículos, las políticas de conducto— no
    // tienen contrapartida, y perderlos rompería la fidelidad de formas nada
    // sutiles: `datasourceRef: hr_db` quedaría colgando y la clasificación
    // entera desaparecería. Viajan enteros por el mismo mecanismo que las
    // dependencias, sin traducir.
    let ajenos: Vec<Json> = canonica
        .iter()
        .filter(|(id, _)| !PROYECTADOS.iter().any(|k| id.starts_with(k)))
        .map(|(id, v)| Json::obj([("id", Json::s(id.as_str())), ("value", v.clone())]))
        .collect();
    if !ajenos.is_empty() {
        let cps = out
            .entry("customProperties".into())
            .or_insert_with(|| Json::Arr(Vec::new()));
        if let Json::Arr(xs) = cps {
            xs.push(propiedad("x-oos-documents", Json::Arr(ajenos)));
        }
    }

    // Los bindings son la sección Servers, y el mapeo físico de cada propiedad
    // viaja con su servidor: sin él, `physicalName` no diría de qué origen.
    let servers: Vec<Json> = canonica
        .iter()
        .filter(|(id, _)| id.starts_with("Binding:"))
        .filter_map(|(_, j)| binding_a_server(j))
        .collect();
    if !servers.is_empty() {
        out.insert("servers".into(), Json::Arr(servers));
    }

    let schema: Vec<Json> = canonica
        .iter()
        .filter(|(id, _)| id.starts_with("Entity:"))
        .filter_map(|(id, j)| entidad_a_schema(id, j))
        .collect();
    if !schema.is_empty() {
        out.insert("schema".into(), Json::Arr(schema));
    }

    // Lo que ya venía de ODCS y el perfil no interpreta, de vuelta a su sitio.
    if let Some(p) = paquete.and_then(obj)
        && let Some(guardado) = p
            .get("metadata")
            .and_then(obj)
            .and_then(|m| m.get(PASSTHROUGH))
        && let Some(g) = obj(guardado)
    {
        for (k, v) in g {
            out.insert(k.clone(), v.clone());
        }
    }

    Json::Obj(out)
}

fn propiedad(nombre: &str, valor: Json) -> Json {
    Json::obj([("property", Json::s(nombre)), ("value", valor)])
}

/// `sla.breakingChangePolicy` se traduce a una `slaProperty`; `sla.properties`
/// salen tal cual. Es el único campo del SLA que OOS tipa, y por eso el único
/// que necesita traducción en lugar de tránsito.
fn sla_a_odcs(sla: &BTreeMap<String, Json>) -> Json {
    let mut props: Vec<Json> = Vec::new();
    if let Some(bcp) = sla.get("breakingChangePolicy").and_then(obj)
        && let Some(np) = bcp.get("noticePeriod")
    {
        props.push(propiedad("x-oos-breakingChangeNoticePeriod", np.clone()));
    }
    if let Some(Json::Arr(xs)) = sla.get("properties") {
        props.extend(xs.iter().cloned());
    }
    Json::Arr(props)
}

fn binding_a_server(b: &Json) -> Option<Json> {
    let spec = obj(b)?.get("spec").and_then(obj)?;
    let meta = obj(b)?.get("metadata").and_then(obj)?;
    let mut s: BTreeMap<String, Json> = BTreeMap::new();
    s.insert("server".into(), meta.get("name")?.clone());
    for (de, a) in [("datasourceRef", "type"), ("source", "dataset")] {
        if let Some(v) = spec.get(de) {
            s.insert(a.to_string(), v.clone());
        }
    }
    s.insert(
        "x-oos-targetEntity".into(),
        spec.get("targetEntity")?.clone(),
    );
    if let Some(ns) = meta.get("namespace") {
        s.insert("x-oos-namespace".into(), ns.clone());
    }
    // §5.1 · `materialization`, `freshnessSLA` y `profile` bajo `x-oos-`.
    for k in ["materialization", "profile", "properties"] {
        if let Some(v) = spec.get(k) {
            s.insert(format!("x-oos-{k}"), v.clone());
        }
    }
    Some(Json::Obj(s))
}

fn entidad_a_schema(id: &str, e: &Json) -> Option<Json> {
    let spec = obj(e)?.get("spec").and_then(obj)?;
    let meta = obj(e)?.get("metadata").and_then(obj)?;

    // Lo que entro de un contrato ODCS nativo y el perfil no consumio
    // —physicalName, descripciones, cuanto ODCS defina y OOS no interprete—
    // vuelve a su sitio antes que nada, para que lo mapeado lo sobrescriba.
    let mut s: BTreeMap<String, Json> = meta
        .get(PASSTHROUGH)
        .and_then(obj)
        .cloned()
        .unwrap_or_default();
    s.insert("name".into(), meta.get("name")?.clone());

    // Un espacio de nombres es lo que distingue una entidad OOS de una
    // importacion a medio terminar (4.2: sin el, el paquete entra en DRAFT y el
    // campo queda como decision pendiente). Sin el no se emiten extensiones:
    // anadirlas inventaria una identidad que nadie ha decidido todavia.
    let oos_nativa = meta.contains_key("namespace");
    if oos_nativa {
        s.insert(
            "x-oos-qualifiedName".into(),
            Json::s(id.trim_start_matches("Entity:")),
        );
        if let Some(ns) = meta.get("namespace") {
            s.insert("x-oos-namespace".into(), ns.clone());
        }
    }

    let clave: Vec<&str> = match spec.get("primaryKey") {
        Some(Json::Arr(xs)) => xs
            .iter()
            .filter_map(|x| match x {
                Json::Str(s) => Some(s.as_str()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    if let Some(props) = spec.get("properties").and_then(obj) {
        let ps: Vec<Json> = props
            .iter()
            .map(|(nombre, def)| {
                let mut p: BTreeMap<String, Json> = BTreeMap::new();
                p.insert("name".into(), Json::s(nombre));
                if let Some(d) = obj(def) {
                    // Lo que trajo el contrato y el perfil no consumió
                    // —`logicalType`, `physicalType`— vuelve primero, para que
                    // lo mapeado pueda sobrescribirlo.
                    if let Some(pt) = d.get(PASSTHROUGH).and_then(obj) {
                        p.extend(pt.iter().map(|(k, v)| (k.clone(), v.clone())));
                    }
                    if oos_nativa {
                        // El tipo paramétrico no tiene equivalente en ODCS:
                        // viaja entero por extensión, y con él la unidad y la
                        // precisión. Perderlas convertiría `Money<EUR, 2>` en
                        // «un número».
                        if let Some(t) = d.get("type") {
                            p.insert("x-oos-type".into(), t.clone());
                        }
                        for k in ["labels", "derivedFrom", "examples", "required", "enum"] {
                            if let Some(v) = d.get(k) {
                                p.insert(format!("x-oos-{k}"), v.clone());
                            }
                        }
                    }
                }
                if clave.contains(&nombre.as_str()) {
                    p.insert("primaryKey".into(), Json::Bool(true));
                }
                Json::Obj(p)
            })
            .collect();
        s.insert("properties".into(), Json::Arr(ps));
    }
    if oos_nativa {
        for k in [
            "nature",
            "relations",
            "timeKey",
            "temporal",
            "moved",
            "reserved",
            "uniqueKeys",
        ] {
            if let Some(v) = spec.get(k) {
                s.insert(format!("x-oos-{k}"), v.clone());
            }
        }
        if let Some(l) = meta.get("labels") {
            s.insert("x-oos-labels".into(), l.clone());
        }
        // El orden de una clave compuesta es semantico (N4) y ODCS solo tiene un
        // booleano por propiedad, que lo pierde. Por eso la clave viaja ademas
        // por extension.
        if let Some(pk) = spec.get("primaryKey") {
            s.insert("x-oos-primaryKey".into(), pk.clone());
        }
    }
    Some(Json::Obj(s))
}

// ── ODCS → OOS ──────────────────────────────────────────────────────────────

/// Importa un contrato ODCS y devuelve la forma canónica del paquete OOS
/// resultante: las mismas identidades y los mismos valores que produciría
/// `normalize::package`.
///
/// Devolver la forma canónica y no un árbol de ficheros es deliberado: lo que
/// la conformidad afirma es que **la ida y vuelta es la identidad**, y esa
/// afirmación se hace sobre significado, no sobre en qué fichero acabó cada
/// cosa.
pub fn import(contrato: &Json) -> BTreeMap<String, Json> {
    let Some(c) = obj(contrato) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();

    let nombre = cadena(c, "name").unwrap_or_default();
    let mut meta: BTreeMap<String, Json> = BTreeMap::new();
    for k in [
        "name",
        "version",
        "status",
        "domain",
        "tenant",
        "tags",
        "description",
        "id",
    ] {
        if let Some(v) = c.get(k) {
            meta.insert(k.to_string(), v.clone());
        }
    }

    // §4.3 · lo que el perfil no reconoce se guarda literalmente. No se
    // interpreta, no se valida, y sale intacto al emitir de vuelta.
    let ajeno: BTreeMap<String, Json> = c
        .iter()
        .filter(|(k, _)| !PERFIL.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if !ajeno.is_empty() {
        meta.insert(PASSTHROUGH.into(), Json::Obj(ajeno));
    }

    let mut spec: BTreeMap<String, Json> = BTreeMap::new();
    let mut extra: Vec<(String, Json)> = Vec::new();
    for k in ["team", "roles", "support", "authoritativeDefinitions"] {
        if let Some(v) = c.get(k) {
            spec.insert(k.to_string(), v.clone());
        }
    }
    if let Some(Json::Arr(cps)) = c.get("customProperties") {
        for cp in cps {
            let Some(m) = obj(cp) else { continue };
            match cadena(m, "property").as_deref() {
                Some("x-oos-owner") => {
                    if let Some(v) = m.get("value") {
                        spec.insert("owner".into(), v.clone());
                    }
                }
                Some("x-oos-dependencies") => {
                    if let Some(v) = m.get("value") {
                        spec.insert("dependencies".into(), v.clone());
                    }
                }
                // El `id` que ORE derivó al emitir no se restaura: no estaba.
                Some("x-oos-idDerived") => {
                    meta.remove("id");
                }
                Some("x-oos-documents") => {
                    if let Some(Json::Arr(xs)) = m.get("value") {
                        for d in xs {
                            if let Some(e) = obj(d)
                                && let (Some(id), Some(v)) = (cadena(e, "id"), e.get("value"))
                            {
                                extra.push((id, v.clone()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(sla) = odcs_a_sla(c) {
        spec.insert("sla".into(), sla);
    }

    let mut paquete: BTreeMap<String, Json> = BTreeMap::new();
    paquete.insert("apiVersion".into(), Json::s(crate::document::API_VERSION));
    paquete.insert("kind".into(), Json::s(Kind::Package.as_str()));
    paquete.insert("metadata".into(), Json::Obj(meta));
    paquete.insert("spec".into(), Json::Obj(spec));
    out.insert(format!("Package:{nombre}"), Json::Obj(paquete));
    out.extend(extra);

    if let Some(Json::Arr(xs)) = c.get("schema") {
        for s in xs {
            if let Some((id, e)) = schema_a_entidad(s) {
                out.insert(id, e);
            }
        }
    }
    if let Some(Json::Arr(xs)) = c.get("servers") {
        for s in xs {
            if let Some((id, b)) = server_a_binding(s) {
                out.insert(id, b);
            }
        }
    }
    out
}

fn odcs_a_sla(c: &BTreeMap<String, Json>) -> Option<Json> {
    let Json::Arr(props) = c.get("slaProperties")? else {
        return None;
    };
    let mut sla: BTreeMap<String, Json> = BTreeMap::new();
    let mut resto = Vec::new();
    for p in props {
        let Some(m) = obj(p) else { continue };
        if cadena(m, "property").as_deref() == Some("x-oos-breakingChangeNoticePeriod") {
            if let Some(v) = m.get("value") {
                sla.insert(
                    "breakingChangePolicy".into(),
                    Json::obj([("noticePeriod", v.clone())]),
                );
            }
        } else {
            resto.push(p.clone());
        }
    }
    if !resto.is_empty() {
        sla.insert("properties".into(), Json::Arr(resto));
    }
    (!sla.is_empty()).then_some(Json::Obj(sla))
}

/// Deshace `x-oos-<campo>` sobre un mapa, devolviendo lo desnudo.
fn desprefijar(m: &BTreeMap<String, Json>, campos: &[&str], out: &mut BTreeMap<String, Json>) {
    for k in campos {
        if let Some(v) = m.get(&format!("x-oos-{k}")) {
            out.insert((*k).to_string(), v.clone());
        }
    }
}

/// Lo que el perfil consume de un objeto `schema` de ODCS.
const SCHEMA_PERFIL: &[&str] = &["name", "properties"];
/// Lo que el perfil consume de una propiedad de ODCS.
const PROP_PERFIL: &[&str] = &["name", "primaryKey"];

/// Lo que queda de un objeto ODCS tras quitarle lo que el perfil consume. Es lo
/// que hay que conservar literalmente para que la vuelta sea la identidad.
fn sobrante(m: &BTreeMap<String, Json>, perfil: &[&str]) -> BTreeMap<String, Json> {
    m.iter()
        .filter(|(k, _)| !perfil.contains(&k.as_str()) && !k.starts_with("x-oos-"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn schema_a_entidad(s: &Json) -> Option<(String, Json)> {
    let m = obj(s)?;
    let mut meta: BTreeMap<String, Json> = BTreeMap::new();
    meta.insert("name".into(), m.get("name")?.clone());
    desprefijar(m, &["namespace", "labels"], &mut meta);
    let sobra = sobrante(m, SCHEMA_PERFIL);
    if !sobra.is_empty() {
        meta.insert(PASSTHROUGH.into(), Json::Obj(sobra));
    }

    let mut spec: BTreeMap<String, Json> = BTreeMap::new();
    desprefijar(
        m,
        &[
            "nature",
            "primaryKey",
            "relations",
            "timeKey",
            "temporal",
            "moved",
            "reserved",
            "uniqueKeys",
        ],
        &mut spec,
    );

    let mut clave_por_banderas: Vec<String> = Vec::new();
    if let Some(Json::Arr(ps)) = m.get("properties") {
        let mut props: BTreeMap<String, Json> = BTreeMap::new();
        for p in ps {
            let Some(pm) = obj(p) else { continue };
            let Some(nombre) = cadena(pm, "name") else {
                continue;
            };
            let mut def: BTreeMap<String, Json> = BTreeMap::new();
            desprefijar(
                pm,
                &[
                    "type",
                    "labels",
                    "derivedFrom",
                    "examples",
                    "required",
                    "enum",
                ],
                &mut def,
            );
            let sobra = sobrante(pm, PROP_PERFIL);
            if !sobra.is_empty() {
                def.insert(PASSTHROUGH.into(), Json::Obj(sobra));
            }
            // La clave, cuando no vino por extension: ODCS la marca propiedad a
            // propiedad. Se reconstruye, y NO se inventa si no hay ninguna.
            if matches!(pm.get("primaryKey"), Some(Json::Bool(true))) {
                clave_por_banderas.push(nombre.clone());
            }
            props.insert(nombre, Json::Obj(def));
        }
        spec.insert("properties".into(), Json::Obj(props));
    }
    if !spec.contains_key("primaryKey") && !clave_por_banderas.is_empty() {
        spec.insert(
            "primaryKey".into(),
            Json::Arr(clave_por_banderas.into_iter().map(Json::Str).collect()),
        );
    }

    let mut e: BTreeMap<String, Json> = BTreeMap::new();
    e.insert("apiVersion".into(), Json::s(crate::document::API_VERSION));
    e.insert("kind".into(), Json::s(Kind::Entity.as_str()));
    e.insert("metadata".into(), Json::Obj(meta));
    e.insert("spec".into(), Json::Obj(spec));
    // Sin nombre cualificado, la identidad es el nombre a secas: es una
    // importacion a la que le falta decidir el espacio de nombres.
    let qn = cadena(m, "x-oos-qualifiedName").or_else(|| cadena(m, "name"))?;
    Some((format!("Entity:{qn}"), Json::Obj(e)))
}

fn server_a_binding(s: &Json) -> Option<(String, Json)> {
    let m = obj(s)?;
    let mut meta: BTreeMap<String, Json> = BTreeMap::new();
    meta.insert("name".into(), m.get("server")?.clone());
    desprefijar(m, &["namespace"], &mut meta);

    let mut spec: BTreeMap<String, Json> = BTreeMap::new();
    desprefijar(
        m,
        &["targetEntity", "materialization", "profile", "properties"],
        &mut spec,
    );
    for (de, a) in [("type", "datasourceRef"), ("dataset", "source")] {
        if let Some(v) = m.get(de) {
            spec.insert(a.to_string(), v.clone());
        }
    }

    let mut b: BTreeMap<String, Json> = BTreeMap::new();
    b.insert("apiVersion".into(), Json::s(crate::document::API_VERSION));
    b.insert("kind".into(), Json::s(Kind::Binding.as_str()));
    b.insert("metadata".into(), Json::Obj(meta));
    b.insert("spec".into(), Json::Obj(spec));

    let ns = cadena(&meta_de(&b), "namespace");
    let nombre = cadena(&meta_de(&b), "name")?;
    let id = match ns {
        Some(ns) => format!("Binding:{ns}.{nombre}"),
        None => format!("Binding:{nombre}"),
    };
    Some((id, Json::Obj(b)))
}

fn meta_de(b: &BTreeMap<String, Json>) -> BTreeMap<String, Json> {
    b.get("metadata").and_then(obj).cloned().unwrap_or_default()
}
