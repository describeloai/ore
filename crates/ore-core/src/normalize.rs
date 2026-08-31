//! Normalización semántica — N1 a N8 de `90-canonical-form`.
//!
//! Convierte el árbol YAML de un documento en el valor JSON del que sale su
//! digest. Se aplica **antes** de serializar, y cada regla existe porque sin
//! ella dos autores que dicen lo mismo producirían identidades distintas.
//!
//! | | Regla | Qué deja de importar |
//! |---|---|---|
//! | N1 | cualificación de nombres | escribir `Employee` o `hr.Employee` |
//! | N2 | defaults materializados | omitir lo que ya era el valor por defecto |
//! | N3 | ausencia, nunca nulo | escribir `null` en vez de no escribir |
//! | N4 | conjuntos ordenados | el orden, **solo** donde no es semántico |
//! | N5 | Unicode NFC | cómo compuso el editor una `ñ` |
//! | N6 | identificadores | *nada* — aquí las mayúsculas sí importan |
//! | N7 | comentarios y formato | todo lo que no es significado |
//! | N8 | lo derivado no se serializa | lo que calculó el compilador |
//!
//! # N8 es la única que se cumple no haciendo nada
//!
//! Las otras siete transforman. N8 **prohíbe**: la etiqueta que `flow` computa
//! para una propiedad derivada, el linaje, el grafo de consumidores —todo eso
//! existe ya en memoria cuando se llega aquí, y sería trivial y tentador
//! inyectarlo. No se inyecta. Son salida del compilador y viven en el bundle,
//! no en el repositorio (principio P2).

use crate::json::Json;
use crate::link::{Loaded, Package};
use crate::parse::{Node, Style};
use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

/// N4 · Los campos lista cuyo orden **no** es semántico.
///
/// El resto son secuencias y conservan su orden. El defecto es deliberado:
/// ordenar algo cuyo orden significa —`primaryKey`, `enum`, los niveles de un
/// retículo, `moved`— corrompe el documento en silencio, mientras que no ordenar
/// un conjunto solo hace que dos escrituras equivalentes no converjan. De los
/// dos errores posibles, este módulo comete el reversible.
const CONJUNTOS: &[&str] = &[
    "predicatePushdown",
    "aggregatePushdown",
    "requiredFilters",
    "datasources",
    "dependencies",
    // `reviewers` estuvo aquí desde v1alpha1 y **no es un campo de OOS**:
    // `01-package` §5 lo rechaza con todas las letras —ODCS ya tiene
    // `roles[].firstLevelApprovers`, y aplicarlo es de CODEOWNERS—. Salió de
    // copiar un ejemplo de la prosa de `90-canonical-form` a código. Lo
    // encontró el test de abajo en su primera ejecución.
    "roles",
    "tags",
    "authoritativeDefinitions",
    // v1alpha3. Los cuatro son conjuntos y la especificación lo dice de los dos
    // primeros con todas las letras: `targets` porque la unión es conmutativa,
    // `assertions` porque **todas** se sostienen y no hay nada que desempatar.
    // `masks` y `duties` por lo mismo — todas se aplican, ninguna gana.
    //
    // Faltaban, y no era cosmético: dos `Ruleset` que decían lo mismo con las
    // aserciones en otro orden producían **dos digests**. Eso es G1 —el mismo
    // commit produce el mismo digest— rota en el plano nuevo, y la
    // especificación afirmaba lo contrario.
    //
    // Contrástese con `Resolution.strategies`, que NO está aquí y no debe
    // estarlo: allí la primera que casa gana, así que reordenarla cambia qué
    // registros se fusionan. La misma regla trata los dos casos distinto porque
    // los dos casos son distintos.
    "targets",
    "named",
    "assertions",
    "masks",
    "duties",
    // v1alpha2, y el hueco era el mismo: esta lista no había crecido desde
    // v1alpha1, así que **todo campo lista añadido después quedó sin
    // clasificar**. Reordenar los endosos de una función daba otro digest.
    //
    // Los cinco son conjuntos por la misma razón: se cumplen todos y ninguno
    // gana. `endosada()` los recorre con `any`, las precondiciones se exigen
    // todas, los efectos se comprueban todos.
    "effects",
    "endorsements",
    "preconditions",
    "sources",
    "weights",
    // v1alpha4, y la tercera vez que pasa lo mismo: **esta lista no crece con
    // la versión que introduce los campos**, así que cada plano nuevo llega con
    // G1 rota y nadie lo ve hasta que alguien compara dos digests.
    //
    // `requires` es un conjunto porque una forma no depende del orden en que se
    // enumeran sus conceptos, y la especificación lo dice con todas las letras;
    // `implements` porque implementar dos formas no tiene primera ni segunda;
    // `requiresGovernance` porque exigir dos naturalezas no las ordena.
    //
    // Contrástese con `enum`, que NO está aquí y no debe estarlo: retirar un
    // valor o reordenarlos es un cambio observable, igual que en
    // `Resolution.strategies`. La misma regla trata los dos casos distinto
    // porque los dos casos son distintos.
    "requires",
    "implements",
    "requiresGovernance",
    // v1alpha1, y esto es lo que había que ver: **la lista nunca estuvo
    // completa, ni siquiera para la versión con la que se escribió**. Tres de
    // estos se midieron dando dos digests para el mismo contenido —`reserved`,
    // `uniqueKeys` y `support`— en la versión cerrada, la que
    // se venía usando como prueba de que ese número significa algo.
    //
    // `derivedFrom` es el que más pesa: es lo que propaga las etiquetas, y el
    // `join` es conmutativo. Que el orden en que se escriben dos orígenes
    // cambiara el digest de la entidad era G1 rota en el corazón del régimen
    // de flujo.
    "derivedFrom",
    "moved",
    "reserved",
    "uniqueKeys",
    "support",
    "members",
    "exclude",
    "synonyms",
    "examples",
    "values",
    "match",
    "customProperties",
    "packages",
    "nameList",
    "conflicts",
    "requestedRanges",
    // El de `materialization`: **qué propiedades se copian**. No confundir con
    // el `properties` de una entidad, que es un mapa y no llega aquí.
    "properties",
];

/// Lo que **no** es un conjunto, y por qué.
///
/// Existe para que no haya una tercera categoría —*«no lo he mirado»*—, que es
/// la que ha producido las cuatro roturas de G1 de este proyecto. Un campo
/// lista que no esté en ninguna de las dos listas **rompe un test**, así que la
/// única forma de añadir uno es decidiendo qué es.
///
/// Es la misma ley que `OOS8002` y `OOS9004`, aplicada al compilador en vez de
/// a un documento: **un campo sin clasificar tiene exactamente el mismo aspecto
/// que uno clasificado bien.**
const SECUENCIAS: &[&str] = &[
    // El orden ES el significado.
    "levels",     // ascendente por restrictividad: el retículo entero
    "primaryKey", // en una clave compuesta el orden es significativo
    // `via` se empareja POSICION A POSICION con la `primaryKey` del destino, asi
    // que ordenarla enlazaria por pares distintos: `[codPais, id]` contra
    // `[id, codPais]` no es la misma relacion, y el documento se veria igual.
    "via",
    "enum",             // retirar un valor o reordenarlos es observable
    "strategies",       // la primera que casa gana
    "normalize",        // una tubería de transformaciones
    "mustBeBetween",    // `[min, max]`
    "mustNotBeBetween", // igual
];

/// Todo campo lista de los esquemas está en una de las dos listas.
///
/// Se expone para que el arnés lo compruebe contra los esquemas publicados en
/// vez de contra la memoria de nadie.
pub fn clasificacion_de_listas() -> (&'static [&'static str], &'static [&'static str]) {
    (CONJUNTOS, SECUENCIAS)
}

/// Mapas **cuyos valores** son conjuntos.
///
/// `CONJUNTOS` mira la clave bajo la que cuelga una secuencia, y eso no alcanza
/// a `Lattice.requiresGovernance`: allí las listas cuelgan del **nombre de un
/// nivel** —`high`, `critical`— que es un nombre arbitrario y no puede estar en
/// ninguna lista fija.
///
/// El resultado era que el mismo campo se comportaba distinto en dos
/// documentos: ordenado en un `Property`, sin ordenar en un `Lattice`. **Dos
/// semánticas para un nombre**, que es exactamente lo que este proyecto
/// persigue — y una G1 rota que llevaba desde v1alpha3 sin que nadie mirase.
const MAPAS_DE_CONJUNTOS: &[&str] = &["requiresGovernance"];

// Lo que NO entra, y conviene que se vea la ausencia:
//
// - `strategies` de `Resolution`: la primera que casa gana. Ordenarla cambiaría
//   qué registros se fusionan, que es el fallo silencioso que N4 existe para
//   impedir.
// - `normalize` dentro de una estrategia: son transformaciones encadenadas y no
//   todas conmutan. Ante la duda, secuencia — de los dos errores posibles este
//   módulo comete el reversible.
// - `levels` de un retículo: su orden **es** el orden parcial.

/// N1 · Campos cuyo valor es una **referencia** a otro documento y por tanto se
/// expande al nombre cualificado. Un nombre corto es azúcar del autor, no una
/// identidad distinta.
/// La clave con la que se recurre a una lista dentro de una lista: no está en
/// `CONJUNTOS`, no está en `SECUENCIAS` y no es una referencia.
const ANIDADA: &str = "";

fn es_referencia(clave: &str) -> bool {
    matches!(clave, "targetEntity" | "target")
}

/// N1 · Expande un nombre corto con el espacio de nombres de quien lo escribe.
///
/// Pública porque **la fase de enlazado tiene que resolver con esta misma
/// regla**. Si `link` exigiera el nombre largo y `normalize` aceptara el corto,
/// habría documentos que compilan y no resuelven, o peor: que resuelven a una
/// cosa y cuyo digest describe otra.
pub fn qualify(nombre: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(ns) if !nombre.contains('.') => format!("{ns}.{nombre}"),
        _ => nombre.to_string(),
    }
}

/// N3 · Las formas en que YAML escribe «nada».
fn es_nulo(raw: &str, style: Style) -> bool {
    style == Style::Plain && matches!(raw, "" | "~" | "null" | "Null" | "NULL")
}

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

struct Ctx {
    /// El espacio de nombres del documento, para N1.
    namespace: Option<String>,
}

fn escalar(raw: &str, style: Style, clave: &str, ctx: &Ctx) -> Json {
    if style != Style::Plain {
        // Entrecomillado es siempre una cadena. Es lo que hace que `"68400.50"`
        // sobreviva intacto: §4.1 y `OOS6003` conspiran para que aquí no haya
        // ninguna decisión que tomar.
        return Json::Str(nfc(raw));
    }
    match raw {
        "true" => return Json::Bool(true),
        "false" => return Json::Bool(false),
        _ => {}
    }
    if let Ok(n) = raw.parse::<i64>() {
        return Json::Int(n);
    }

    // N1 · un nombre sin punto en un campo de referencia es la forma corta.
    if es_referencia(clave) {
        return Json::Str(nfc(&qualify(raw, ctx.namespace.as_deref())));
    }
    Json::Str(nfc(raw))
}

fn valor(n: &Node, clave: &str, ctx: &Ctx) -> Option<Json> {
    match n {
        Node::Scalar { raw, style, .. } => {
            (!es_nulo(raw, *style)).then(|| escalar(raw, *style, clave, ctx))
        }
        Node::Mapping { entries, .. } => {
            let mut m: BTreeMap<String, Json> = BTreeMap::new();
            // N4, para el caso que la clave inmediata no alcanza: aquí el que
            // sabe que sus valores son conjuntos es el mapa, no ellos.
            let valores_son_conjuntos = MAPAS_DE_CONJUNTOS.contains(&clave);
            for (k, v) in entries {
                let Some(nombre) = k.as_str() else { continue };
                // N7 · el comentario ya se descartó al analizar; aquí se
                // descarta el resto del formato, que es todo lo que no es una
                // clave o un valor.
                if let Some(mut j) = valor(v, nombre, ctx) {
                    // Ordenar después equivale a ordenar durante: los elementos
                    // ya están en forma canónica, que es la única ordenación que
                    // no depende de cómo estén escritos.
                    if valores_son_conjuntos && let Json::Arr(ref mut xs) = j {
                        xs.sort_by_key(|x| x.jcs());
                    }
                    m.insert(nfc(nombre), j);
                }
            }
            Some(Json::Obj(m))
        }
        Node::Sequence { items, .. } => {
            // Una lista dentro de una lista NO hereda la clasificación de la de
            // fuera, y `uniqueKeys` es por lo que hace falta decirlo: es un
            // **conjunto de claves**, y cada clave es una **secuencia**.
            // Ordenar las de dentro convertiría la clave compuesta `[a, b]` en
            // otra clave.
            //
            // Pasar una clave neutra es seguro porque la única cosa para la que
            // `clave` sirve en un escalar es `es_referencia`, y ninguna de las
            // dos referencias que existen —`target`, `targetEntity`— es una
            // lista de listas.
            let mut xs: Vec<Json> = items
                .iter()
                .map(|i| match i {
                    Node::Sequence { .. } => (i, ANIDADA),
                    _ => (i, clave),
                })
                .filter_map(|(i, k)| valor(i, k, ctx))
                .collect();
            // N4 · ordenar por la forma canónica serializada, que es la única
            // ordenación que no depende de cómo esté escrito el elemento.
            if CONJUNTOS.contains(&clave) {
                xs.sort_by_key(|x| x.jcs());
            }
            Some(Json::Arr(xs))
        }
    }
}

/// N2 · Materialización de valores por defecto.
///
/// La forma canónica no contiene valores implícitos: un binding que omite
/// `materialization` y otro que escribe `mode: passthrough` **dicen lo mismo**, y
/// tienen que producir el mismo digest. Omitir es un atajo de escritura, no una
/// afirmación distinta.
fn defaults(kind: crate::document::Kind, doc: &mut Json) {
    use crate::document::Kind;
    let Json::Obj(raiz) = doc else { return };
    let Some(Json::Obj(spec)) = raiz.get_mut("spec") else {
        return;
    };
    if kind == Kind::Binding {
        let mat = spec
            .entry("materialization".to_string())
            .or_insert_with(|| Json::Obj(BTreeMap::new()));
        if let Json::Obj(m) = mat {
            m.entry("mode".to_string())
                .or_insert_with(|| Json::s("passthrough"));
        }
    }
}

/// La identidad de un documento: `kind:nombreCualificado`.
///
/// **Nunca la ruta.** El nombre del fichero es incidental, igual que los
/// comentarios y la indentación: renombrar `Employee.yaml` a `emp.yaml` no
/// cambia lo que el documento dice, y por tanto no debe cambiar su digest ni
/// invalidar una firma (`90-canonical-form` §5.2).
pub fn doc_id(d: &Loaded) -> String {
    format!(
        "{}:{}",
        d.kind.as_str(),
        d.qname().unwrap_or_else(|| "<sin nombre>".into())
    )
}

/// La forma canónica de un documento.
pub fn document(d: &Loaded) -> Json {
    let ctx = Ctx {
        namespace: d
            .meta("namespace")
            .and_then(|n| n.as_str())
            .map(str::to_string),
    };
    let mut j = valor(&d.root, "", &ctx).unwrap_or(Json::Obj(BTreeMap::new()));
    defaults(d.kind, &mut j);
    j
}

/// La forma canónica de un documento **ajeno**: sin N1 ni N2.
///
/// Un contrato ODCS no tiene espacio de nombres OOS que expandir ni defaults de
/// OOS que materializar, y aplicárselos sería interpretarlo — que es justo lo
/// que `01-package` §4.3 prohíbe. Lo que sí se aplica es lo que no depende del
/// perfil: nulos fuera, NFC, comentarios y formato descartados.
pub fn foreign(root: &Node) -> Json {
    let ctx = Ctx { namespace: None };
    valor(root, "", &ctx).unwrap_or(Json::Obj(BTreeMap::new()))
}

/// Entidades del paquete que ningún binding enlaza.
pub fn sin_binding(pkg: &Package) -> Vec<String> {
    let enlazadas: std::collections::BTreeSet<String> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == crate::document::Kind::Binding)
        .filter_map(|b| {
            let t = b.section("targetEntity")?.as_str()?;
            Some(qualify(t, b.meta("namespace").and_then(|n| n.as_str())))
        })
        .collect();
    pkg.docs
        .iter()
        .filter(|d| d.kind == crate::document::Kind::Entity)
        .filter_map(|e| e.qname())
        .filter(|qn| !enlazadas.contains(qn))
        .collect()
}

/// ¿Es este documento el lock? Es un artefacto generado, no fuente: entra en el
/// digest del **bundle**, no en el del paquete (§5.3).
pub fn es_lock(d: &Loaded) -> bool {
    d.path.file_name().is_some_and(|n| n == "ontology.lock")
}

/// La forma canónica de un paquete: sus documentos fuente por identidad.
///
/// Ordenados por `docId`, que es lo que hace que **mover un paquete de la forma
/// plana a `packages/hr/` no cambie el artefacto**. La estructura de carpetas es
/// una preferencia de quien organiza el repositorio; la ontología es la misma.
pub fn package(pkg: &Package) -> BTreeMap<String, Json> {
    pkg.docs
        .iter()
        .filter(|d| !es_lock(d))
        .map(|d| (doc_id(d), document(d)))
        .collect()
}

#[cfg(test)]
mod tests {
    /// **Todo campo lista de los esquemas publicados está clasificado.**
    ///
    /// Este test existe porque el proyecto rompió `G1` cuatro veces por la
    /// misma causa, y ninguna se descubrió leyendo: siempre comparando dos
    /// digests a mano. `CONJUNTOS` no creció con v1alpha2, ni con v1alpha3, ni
    /// con v1alpha4 — y al medirlo del todo resultó que **tampoco estaba
    /// completa para v1alpha1**, la versión cerrada.
    ///
    /// > Una lista que hay que acordarse de actualizar es una lista de la que
    /// > nadie se acuerda.
    ///
    /// Así que deja de haber que acordarse. Un campo lista nuevo en un esquema
    /// **rompe este test** hasta que alguien diga si es un conjunto o una
    /// secuencia. Es la misma ley que `OOS8002` y `OOS9004` aplicada al
    /// compilador: *lo que no se decide, no compila*.
    ///
    /// Se lee de los esquemas **publicados** y no de una lista paralela: si la
    /// fuente de verdad fuera otro fichero de este repositorio, volvería a
    /// poder desincronizarse.
    #[test]
    fn todo_campo_lista_de_los_esquemas_esta_clasificado() {
        use crate::parse::Node;
        use std::collections::BTreeSet;
        use std::path::{Path, PathBuf};

        let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/oos/schemas");
        let mut ficheros = Vec::new();
        fn recorrer(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(es) = std::fs::read_dir(dir) else {
                return;
            };
            for e in es.flatten() {
                let p = e.path();
                if p.is_dir() {
                    recorrer(&p, out);
                } else if p.extension().is_some_and(|x| x == "json") {
                    out.push(p);
                }
            }
        }
        recorrer(&raiz, &mut ficheros);
        assert!(!ficheros.is_empty(), "submódulo sin inicializar");

        // JSON es un subconjunto de YAML, así que el analizador del motor lo
        // lee sin que haga falta un segundo. Es la misma vía por la que `ore
        // export` acepta un contrato ODCS.
        fn buscar(n: &Node, nombre: Option<&str>, out: &mut BTreeSet<String>) {
            let Node::Mapping { entries, .. } = n else {
                return;
            };
            let es_lista = n
                .get("type")
                .and_then(|(_, v)| v.as_str())
                .is_some_and(|t| t == "array");
            if es_lista && let Some(nombre) = nombre {
                out.insert(nombre.to_string());
            }
            for (k, v) in entries {
                let clave = k.as_str().unwrap_or_default();
                if matches!(clave, "properties" | "$defs" | "definitions")
                    && let Node::Mapping { entries: hijos, .. } = v
                {
                    for (hk, hv) in hijos {
                        buscar(hv, hk.as_str(), out);
                    }
                } else {
                    buscar(v, nombre, out);
                }
            }
        }

        let mut listas: BTreeSet<String> = BTreeSet::new();
        for f in &ficheros {
            let texto = std::fs::read_to_string(f).expect("esquema ilegible");
            let arbol = crate::parse::parse(&texto).expect("esquema que no analiza");
            buscar(&arbol, None, &mut listas);
        }

        let clasificados: BTreeSet<String> = CONJUNTOS
            .iter()
            .chain(SECUENCIAS.iter())
            .map(|s| s.to_string())
            .collect();
        let huerfanos: Vec<&String> = listas.difference(&clasificados).collect();
        assert!(
            huerfanos.is_empty(),
            "campos lista sin clasificar como conjunto o secuencia: {huerfanos:?}"
        );

        // Y al revés: nada clasificado que ya no exista. Una entrada muerta no
        // rompe nada hoy y despista mañana.
        let vivos: Vec<&str> = clasificados
            .iter()
            .filter(|c| !listas.contains(*c))
            .map(|s| s.as_str())
            .collect();
        assert!(
            vivos.is_empty(),
            "clasificados y ya inexistentes: {vivos:?}"
        );

        // Y ninguno en las dos.
        let ambos: Vec<&&str> = CONJUNTOS
            .iter()
            .filter(|c| SECUENCIAS.contains(c))
            .collect();
        assert!(ambos.is_empty(), "clasificados dos veces: {ambos:?}");
    }

    use super::*;
    use crate::document::Kind;

    fn doc(kind: Kind, yaml: &str) -> Loaded {
        Loaded {
            path: "x.yaml".into(),
            kind,
            root: crate::parse::parse(yaml).unwrap(),
        }
    }

    #[test]
    fn n1_expande_la_forma_corta() {
        let corto = document(&doc(
            Kind::Binding,
            "metadata: { name: erp, namespace: hr }\nspec: { targetEntity: Employee }\n",
        ));
        let largo = document(&doc(
            Kind::Binding,
            "metadata: { name: erp, namespace: hr }\nspec: { targetEntity: hr.Employee }\n",
        ));
        assert_eq!(corto.jcs(), largo.jcs());
    }

    #[test]
    fn n2_materializa_el_modo_por_defecto() {
        let j = document(&doc(Kind::Binding, "spec: { targetEntity: hr.E }\n"));
        assert!(j.jcs().contains(r#""mode":"passthrough""#), "{}", j.jcs());
    }

    #[test]
    fn n3_omite_los_nulos() {
        let j = document(&doc(Kind::Entity, "spec: { a: ~, b: 1 }\n"));
        assert_eq!(j.jcs(), r#"{"spec":{"b":1}}"#);
    }

    /// N4 en sus dos direcciones, que es donde está el contenido de la regla.
    #[test]
    fn n4_ordena_conjuntos_y_respeta_secuencias() {
        let uno = document(&doc(
            Kind::Binding,
            "spec: { capabilities: { predicatePushdown: [range, eq, in] } }\n",
        ));
        let otro = document(&doc(
            Kind::Binding,
            "spec: { capabilities: { predicatePushdown: [in, range, eq] } }\n",
        ));
        assert_eq!(
            uno.jcs(),
            otro.jcs(),
            "un conjunto no debe depender del orden"
        );

        let a = document(&doc(Kind::Entity, "spec: { primaryKey: [a, b] }\n"));
        let b = document(&doc(Kind::Entity, "spec: { primaryKey: [b, a] }\n"));
        assert_ne!(a.jcs(), b.jcs(), "una clave compuesta SÍ depende del orden");
    }

    #[test]
    fn n5_converge_nfc_y_nfd() {
        let compuesto = document(&doc(
            Kind::Entity,
            "metadata: { description: \"compañía\" }\n",
        ));
        let descompuesto = document(&doc(
            Kind::Entity,
            "metadata: { description: \"compan\u{303}i\u{301}a\" }\n",
        ));
        assert_eq!(compuesto.jcs(), descompuesto.jcs());
    }

    #[test]
    fn n6_las_mayusculas_distinguen() {
        let a = document(&doc(Kind::Entity, "spec: { primaryKey: [employeeId] }\n"));
        let b = document(&doc(Kind::Entity, "spec: { primaryKey: [employeeid] }\n"));
        assert_ne!(a.jcs(), b.jcs());
    }

    #[test]
    fn n7_el_formato_no_llega_al_digest() {
        let plano = document(&doc(Kind::Entity, "spec: { a: 1, b: 2 }\n"));
        let bloque = document(&doc(
            Kind::Entity,
            "# un comentario\nspec:\n  a: 1\n  b: 2\n",
        ));
        assert_eq!(plano.jcs(), bloque.jcs());
    }

    /// La identidad vive dentro del documento, no en dónde alguien lo guardó.
    #[test]
    fn la_identidad_no_es_la_ruta() {
        let mut a = doc(
            Kind::Entity,
            "metadata: { name: Employee, namespace: hr }\n",
        );
        let mut b = doc(
            Kind::Entity,
            "metadata: { name: Employee, namespace: hr }\n",
        );
        a.path = "entities/Employee.yaml".into();
        b.path = "packages/hr/entities/emp_v2_final.yaml".into();
        assert_eq!(doc_id(&a), doc_id(&b));
        assert_eq!(doc_id(&a), "Entity:hr.Employee");
    }
}
