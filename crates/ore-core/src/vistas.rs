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
//! | `from.datasource`, `materialized.datasource` o el `datasource` de una tabla sin declarar | `OOS2004` — el mismo que para `datasourceRef`, porque es el mismo defecto |
//! | `from.view`, `from.table`, `backedBy`, un campo o un filtro que nombran lo que no existe | `OOS2018` |
//! | una cadena de vistas que vuelve sobre sí misma | `OOS2019` |
//! | la vista que respalda una entidad no expone su clave o sus `via` | `OOS2011` — lo que necesita columna, dicho de la vista |
//! | una vista cuya **raíz de lectura** no se deja leer y no lleva `materialized` | `OOS2020` — v1alpha8 |
//! | una copia de un flujo que solo anexa respaldando una entidad **mutable** | `OOS2021` — v1alpha8 |
//! | una propiedad de una entidad que su vista no expone, sin `derivedFrom` | `OOS2022` — v1alpha8, y la otra cara de haber retirado la federación |
//!
//! # v1alpha8 · la tabla, y por qué `OOS2018` llega ahora hasta el suelo
//!
//! Hasta v1alpha8 el puntero físico vivía **dentro** de la vista, y con él el
//! límite de lo comprobable: ningún documento decía qué columnas tenía la
//! fuente, así que la comprobación de nombres cubría el eslabón vista→vista y
//! **creía** el último tramo — el que toca el mundo. `kind: Table` declara las
//! columnas, y por eso la misma regla, con el mismo código, alcanza ahora la
//! columna física.
//!
//! Las dos versiones conviven sin condicionales repartidos: la diferencia está
//! en `Fuente`, que ahora tiene tres variantes, y en `raiz()`, que sabe llegar
//! por los dos caminos. **Todo lo de encima —`flow`, `governance`, el ejecutor,
//! `ore view`— llama a `raiz()` y no se entera**, que es lo que la absorción
//! V0–V3 compró y aquí se cobra.
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

/// De dónde sale una vista: de una tabla, de otra vista, o —v1alpha7— del
/// puntero físico que la vista llevaba dentro.
///
/// Las dos primeras variantes dicen lo mismo del mundo y no son la misma cosa
/// para quien las escribe: `Datasource` es el contrato físico **repetido en
/// cada vista que toca la fuente**, y `Tabla` es el mismo contrato **nombrado
/// una vez**. Por eso la primera no se borra —un documento v1alpha7 sigue
/// compilando— y por eso no se fusionan: fusionarlas sería volver a tener un
/// sitio donde el objeto se describe y otro donde se describe otra vez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fuente {
    /// v1alpha7: `from: {datasource, object}`, el puntero dentro de la vista.
    Datasource {
        datasource: String,
        objeto: String,
    },
    /// v1alpha8: `from: {table}`, el nombre cualificado de un `kind: Table`.
    Tabla(String),
    Vista(String),
}

/// Cómo codifica una tabla los cambios que emite: la cara `D`.
///
/// Son exactamente las tres formas que Flink documenta de convertir una tabla
/// dinámica en un flujo, más la ausencia. El vocabulario es **cerrado** por lo
/// mismo que `predicatePushdown`: cada modo dice qué **pesos** son legales en
/// un delta, y si un perfil pudiera inventar una codificación el mantenedor no
/// podría razonar sobre los que le llegan — un delta con un peso ilegal
/// entraría sin que nadie lo notara.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// No emite cambios, o no se sabe si los emite. No se inventa.
    Ninguno,
    /// Solo altas: solo `+1`. Una marca de agua no ve borrados.
    Anexa,
    /// Un borrado retracta; una actualización retracta la vieja y añade la
    /// nueva. `-1` y `+1`. El *Change Data Feed* de Delta es esto con cuatro
    /// nombres.
    Retracta,
    /// `+1` por clave, `-1` por *tombstone*. Exige clave única.
    Upsert,
}

impl Modo {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Modo::Ninguno,
            "append" => Modo::Anexa,
            "retract" => Modo::Retracta,
            "upsert" => Modo::Upsert,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Modo::Ninguno => "none",
            Modo::Anexa => "append",
            Modo::Retracta => "retract",
            Modo::Upsert => "upsert",
        }
    }
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
    /// El nombre cualificado de la `Table` de la que sale, si sale de una.
    ///
    /// `None` en una cadena v1alpha7, donde el objeto no es un documento y no
    /// tiene nombre que dar. Quien necesite las dos caras —el planificador, el
    /// mantenedor— pregunta por aquí; quien solo necesite dónde vive el dato
    /// tiene `datasource` y `objeto` en los dos casos, y por eso no se entera.
    pub tabla: Option<String>,
}

/// Por qué una cadena no llega a una raíz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinRaiz {
    /// `from.view` nombra una vista que no existe. Lleva la que la nombró.
    NoExiste { vista: String, desde: String },
    /// `from.table` nombra una tabla que no existe. Es un caso aparte y no el
    /// mismo con otro nombre: una cadena que no llega a la tabla no llega al
    /// suelo, y el mensaje tiene que decir qué se buscaba para que se entienda
    /// que falta un documento, no un eslabón más.
    TablaNoExiste { tabla: String, desde: String },
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

    /// La tabla con este nombre cualificado.
    pub fn table(&self, qname: &str) -> Option<&Loaded> {
        self.of(Kind::Table)
            .find(|d| d.qname().as_deref() == Some(qname))
    }

    /// Resuelve una referencia a tabla con la misma regla que a una vista: la
    /// forma corta vale dentro del mismo espacio de nombres (N1).
    pub fn resolve_table(&self, referencia: &str, desde: &Loaded) -> Option<&Loaded> {
        let ns = desde.meta("namespace").and_then(|n| n.as_str());
        self.table(&qualify(referencia, ns))
    }
}

/// `spec.from` de una vista.
pub fn fuente(v: &Loaded) -> Option<Fuente> {
    let from = v.section("from")?;
    let ns = v.meta("namespace").and_then(|n| n.as_str());
    if let Some((_, vista)) = from.get("view") {
        return Some(Fuente::Vista(qualify(vista.as_str()?, ns)));
    }
    if let Some((_, tabla)) = from.get("table") {
        return Some(Fuente::Tabla(qualify(tabla.as_str()?, ns)));
    }
    let datasource = from.get("datasource")?.1.as_str()?.to_string();
    let objeto = from
        .get("object")
        .and_then(|(_, o)| o.as_str())
        .unwrap_or("")
        .to_string();
    Some(Fuente::Datasource { datasource, objeto })
}

/// Las columnas que una tabla declara.
///
/// Es lo único verdaderamente nuevo de v1alpha8, y lo que hace comprobable lo
/// que antes no lo era. El nombre es **opaco** —puede llevar puntos si el
/// origen es anidado— y por eso no es un identificador.
pub fn columnas(t: &Loaded) -> BTreeSet<String> {
    t.section("columns")
        .map(|c| {
            c.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// La cara `I`: si a esta tabla se le puede pedir algo.
///
/// `reads: none` significa *no se le puede pedir nada* — un tema de Kafka se
/// escribe, no se pregunta. Es lo único que hace falta saber para `OOS2020`;
/// **qué** se le puede pedir lo lee el planificador, y eso es de `ore-cli`.
pub fn se_lee(t: &Loaded) -> bool {
    t.section("reads")
        .is_none_or(|r| r.as_str() != Some("none"))
}

/// La cara `D`: cómo codifica sus cambios. Un modo fuera del vocabulario se lee
/// como `Ninguno` **aquí** y lo rechaza la forma con `OOS1004`: esta función no
/// es el sitio donde se decide qué es legal.
pub fn modo(t: &Loaded) -> Modo {
    t.section("changes")
        .and_then(|c| c.get("mode"))
        .and_then(|(_, m)| m.as_str())
        .and_then(Modo::parse)
        .unwrap_or(Modo::Ninguno)
}

/// Campo → nombre en la fuente. Admite la forma breve y la expandida, como el
/// mapeo del binding: la canónica es la expandida.
///
/// La forma expandida se retira en v1alpha8 —existía para llevar
/// `physicalType`, y el tipo físico lo dice ahora `columns`— y **esta función
/// sigue leyéndola**, porque sigue habiendo documentos v1alpha7 que la usan.
/// Lo que decide qué se admite es `spec_keys_en`, no esto.
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
            // Las dos formas de tocar el suelo. Una vista NO sale de una tabla
            // y de otra vista a la vez: `from` es exactamente una de dos, y por
            // eso aquí no hay que decidir nada.
            Some(Fuente::Datasource { .. }) | Some(Fuente::Tabla(_)) => return Ok(fila),
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

/// Por qué una vista no se puede invertir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoInvertible {
    /// Un campo no sale de una columna: sale de **calcularla**. Escribir por
    /// ahí exigiría deshacer el cálculo, y no todos se deshacen.
    CampoCalculado { vista: String, campo: String },
    /// La vista declara algo que esta guarda no sabe clasificar.
    ///
    /// **Y por eso el defecto es «no».** Una clave nueva en el vocabulario que
    /// nadie clasifique llega aquí, en vez de colarse como invertible por no
    /// haberla mirado.
    ConstruccionDesconocida { vista: String, clave: String },
}

/// Las claves de una `View` que **no cambian qué filas ni qué columnas salen**,
/// y por eso no afectan a la invertibilidad.
const NEUTRAS: &[&str] = &["owner", "freshness", "materialized"];

/// Las que sí, y son invertibles las tres.
///
/// | | por qué |
/// |---|---|
/// | `from` | una sola entrada. Es la primera condición que PostgreSQL exige a una vista auto-actualizable |
/// | `fields` | renombrar es una biyección; proyectar pierde columnas, así que la escritura es **parcial**, no ambigua |
/// | `where` | recortar es invertible: la fila escrita cumple el predicado, o se cae de la vista |
const INVERTIBLES: &[&str] = &["from", "fields", "where"];

/// Si se puede escribir a través de esta vista.
///
/// # Por qué hoy no puede fallar, y aun así está
///
/// El vocabulario de `View` en v1alpha8 es exactamente el fragmento invertible
/// —`00-scope` §6.1 lo dice, y no se buscó: se descubrió al migrar—. No hay
/// junta, ni agregado, ni `distinct`, ni límite, así que **ningún documento OOS
/// puede violar esto hoy**, y se dice aquí en vez de dejar que alguien lo
/// deduzca de que ningún caso lo ejerce.
///
/// Lo que esta guarda hace es que el día que la gramática crezca, el
/// constructor nuevo tenga que **decidir** si es invertible, en vez de heredar
/// un «sí» que nadie escribió. El defecto es `ConstruccionDesconocida`, y el
/// censo de `crate::document` lo ata al vocabulario para que no se pueda
/// añadir una clave sin pasar por aquí.
///
/// Es la misma forma que el IR de `ore-view`, que tiene `Agrupa` y `Une` con
/// sus reglas medidas y ningún documento que los produzca: la máquina está
/// lista antes que el vocabulario, a propósito.
pub fn invertible(v: &Loaded) -> Result<(), NoInvertible> {
    let qn = v.qname().unwrap_or_default();
    for (k, _) in v.root.get("spec").map(|(_, s)| s.entries()).unwrap_or(&[]) {
        let Some(clave) = k.as_str() else { continue };
        if NEUTRAS.contains(&clave)
            || INVERTIBLES.contains(&clave)
            || crate::document::is_extension(clave)
        {
            continue;
        }
        return Err(NoInvertible::ConstruccionDesconocida {
            vista: qn,
            clave: clave.to_string(),
        });
    }
    // Un campo tiene que salir de UNA columna, nombrada. Hoy `OOS2018` ya exige
    // que sea columna de la raíz, así que esto tampoco puede fallar; el día que
    // `fields` admita una expresión, falla aquí antes que en ningún sitio.
    for (campo, origen) in campos(v) {
        if !es_nombre_de_columna(&origen) {
            return Err(NoInvertible::CampoCalculado { vista: qn, campo });
        }
    }
    Ok(())
}

#[cfg(test)]
mod censo {
    use super::*;
    use crate::document::{ApiVersion, Kind};

    /// **El vocabulario de `View` está clasificado entero.**
    ///
    /// Esta es la prueba que le da dientes a [`invertible`]. Sin ella la guarda
    /// sería decorativa: alguien añade `groupBy` al vocabulario, nadie lo
    /// clasifica, y la guarda empieza a rechazarlo TODO en silencio —el defecto
    /// es «no»— o, peor, si el defecto fuera «sí», lo aceptaría todo.
    ///
    /// Con esto, añadir una clave a `View` sin decir si se invierte **no
    /// compila la suite**. Es el mismo mecanismo que el censo del registro de
    /// códigos, y por la misma razón: una lista que se puede ampliar sin mirar
    /// deja de significar algo.
    #[test]
    fn el_vocabulario_de_view_esta_clasificado_entero() {
        let mut sin_clasificar: Vec<&str> = Kind::View
            .spec_keys_en(ApiVersion::V1Alpha8)
            .iter()
            .copied()
            .filter(|k| !NEUTRAS.contains(k) && !INVERTIBLES.contains(k))
            .collect();
        sin_clasificar.sort_unstable();
        assert!(
            sin_clasificar.is_empty(),
            "estas claves de `View` no están clasificadas en `vistas.rs`: {sin_clasificar:?}.\n\
             Di si cada una cambia qué filas o qué columnas salen —y entonces si eso se \
             invierte— antes de que un efecto pase por encima de ella sin mirar."
        );
    }

    /// Y la simétrica: nada clasificado que ya no exista.
    #[test]
    fn no_se_clasifica_lo_que_ya_no_esta_en_el_vocabulario() {
        let vocabulario = Kind::View.spec_keys_en(ApiVersion::V1Alpha8);
        let huerfanas: Vec<&&str> = NEUTRAS
            .iter()
            .chain(INVERTIBLES)
            .filter(|k| !vocabulario.contains(k))
            .collect();
        assert!(
            huerfanas.is_empty(),
            "clasificadas y ya inexistentes: {huerfanas:?}"
        );
    }
}

/// Un nombre de columna, no una expresión.
///
/// Deliberadamente permisivo con lo que un origen puede llamar columna
/// —`"Worker_Reference.ID"` es un nombre legal, y `01-table` lo dice—; lo que
/// descarta es lo que **solo** puede ser cómputo: operadores, llamadas, comas.
fn es_nombre_de_columna(s: &str) -> bool {
    !s.is_empty()
        && !s.chars().any(|c| {
            matches!(c, '+' | '-' | '*' | '/' | '(' | ')' | ',' | '|' | '<' | '>' | '=' | '\'')
        })
}

/// La raíz de una vista: fuente, objeto, columnas compuestas y filtros.
///
/// Un campo que en algún eslabón no resuelve **no aparece** en `columnas`: la
/// comprobación de que resuelva es de `comprobar`, con `OOS2018`, y aquí no se
/// inventa una columna para lo que no tiene.
pub fn raiz(pkg: &Package, v: &Loaded) -> Result<Raiz, SinRaiz> {
    let fila = cadena(pkg, v)?;
    let hoja = fila.last().expect("una cadena tiene al menos un eslabón");
    // Los dos caminos al suelo, y el único sitio del núcleo donde se distinguen.
    // Todo lo que llama a `raiz()` recibe la misma forma y no se entera de por
    // cuál vino: es lo que permite que un paquete tenga vistas de las dos
    // versiones a la vez sin que nadie más lleve un condicional.
    let (datasource, objeto, tabla) = match fuente(hoja) {
        Some(Fuente::Datasource { datasource, objeto }) => (datasource, objeto, None),
        Some(Fuente::Tabla(qn)) => {
            let Some(t) = pkg.table(&qn) else {
                return Err(SinRaiz::TablaNoExiste {
                    tabla: qn,
                    desde: hoja.qname().unwrap_or_default(),
                });
            };
            (
                t.section("datasource")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
                t.section("object")
                    .and_then(|o| o.as_str())
                    .unwrap_or_default()
                    .to_string(),
                Some(qn),
            )
        }
        _ => unreachable!("la cadena termina en una tabla o en una fuente por construcción"),
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
        tabla,
    })
}

/// La **raíz de lectura**: de dónde salen de verdad las filas.
///
/// Es la vista `materialized` más cercana bajando por la cadena —ella misma
/// incluida—, y `None` cuando no hay ninguna y por tanto se lee del objeto.
///
/// La distinción con la raíz es todo el asunto de `OOS2020`, y no es un
/// tecnicismo: si la regla mirara la raíz, una vista virtual sobre una
/// materializada sobre un flujo fallaría, y obligaría a materializar dos veces
/// lo mismo. Hay dónde preguntar; está un eslabón más abajo.
pub fn raiz_de_lectura<'a>(pkg: &'a Package, v: &'a Loaded) -> Option<&'a Loaded> {
    cadena(pkg, v)
        .ok()?
        .into_iter()
        .find(|e| e.section("materialized").is_some())
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

/// El gemelo de `no_expone` con el sujeto de v1alpha8: lo que se nombra no es
/// una columna de la tabla.
///
/// Es el mismo código y no el mismo mensaje, y la diferencia importa: *«la
/// vista de abajo no lo expone»* invita a mirar otra vista, y aquí no hay otra
/// vista — hay un objeto que no tiene esa columna, y la ayuda tiene que
/// enseñar las que sí tiene.
fn no_es_columna(
    path: &std::path::Path,
    nodo: &Node,
    que: String,
    tabla: &str,
    cols: &BTreeSet<String>,
) -> Diagnostic {
    Diagnostic::new(Code::Oos2018, path, que)
        .at(nodo.pos())
        .help(if cols.is_empty() {
            format!("`{tabla}` no declara ninguna columna")
        } else {
            format!(
                "`{tabla}` tiene: {}",
                cols.iter().cloned().collect::<Vec<_>>().join(" · ")
            )
        })
}

/// Las comprobaciones de enlazado de las tablas, las vistas y `backedBy`.
pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let declarados = datasources_declarados(pkg);

    // ── Las tablas ──────────────────────────────────────────────────────────
    //
    // Una tabla se sostiene sola: es el puntero a un objeto que existe, y
    // existe lo consulte alguien o no. Lo único que se le puede preguntar aquí
    // es si la fuente está declarada y si lo que sus dos caras nombran son
    // columnas suyas — que es exactamente lo que un esquema JSON no alcanza,
    // porque exige que un campo ESTÉ y no puede saber si lo que dice EXISTE.
    for tabla in pkg.of(Kind::Table) {
        let tqn = tabla.qname().unwrap_or_default();
        let cols = columnas(tabla);

        // OOS2004 · el mismo código que `datasourceRef` y que `from.datasource`.
        // Que el sujeto haya cambiado tres veces y el código sea el mismo es la
        // afirmación de que la tabla no cambia la regla, cambia el sujeto.
        if let Some(ds) = tabla.section("datasource")
            && !declarados.contains(ds.as_str().unwrap_or(""))
        {
            out.push(no_declarado(tabla, ds, "datasource", &declarados));
        }

        if let Some(cambios) = tabla.section("changes") {
            // OOS2018 · la clave del upsert. Sin clave real, un tombstone no
            // dice qué fila retira y el mantenedor aplicaría un `-1` a nada.
            if let Some((_, k)) = cambios.get("key") {
                for i in k.items() {
                    let Some(c) = i.as_str() else { continue };
                    if !cols.contains(c) {
                        out.push(no_es_columna(
                            &tabla.path,
                            i,
                            format!("`{tqn}` declara `changes.key: {c}`, que no es columna suya"),
                            &tqn,
                            &cols,
                        ));
                    }
                }
            }
            // OOS2018 · la marca de agua. Una que no es columna no la lee nadie,
            // y el refresco incremental no tendría por dónde empezar.
            if let Some((_, f)) = cambios.get("field") {
                let c = f.as_str().unwrap_or("");
                if !cols.contains(c) {
                    out.push(no_es_columna(
                        &tabla.path,
                        f,
                        format!("`{tqn}` declara `changes.field: {c}`, que no es columna suya"),
                        &tqn,
                        &cols,
                    ));
                }
            }
        }

        // OOS2024 · la cara `W` exige con qué identificar la fila.
        //
        // `insert` no la necesita: no señala a nada que ya esté. `update` y
        // `delete` sí, y sin ella *«actualiza esta fila»* no nombra ninguna —
        // es un `UPDATE` sin `WHERE`, que no es una escritura parcial sino un
        // accidente. La clave es `changes.key` y no una propia: un segundo
        // sitio diciendo lo mismo es el defecto que la tabla vino a corregir.
        let ops = crate::document::escrituras(tabla.section("writes"));
        let necesita: Vec<&str> = ops
            .iter()
            .filter(|o| *o == "update" || *o == "delete")
            .map(String::as_str)
            .collect();
        if !necesita.is_empty()
            && tabla
                .section("changes")
                .and_then(|c| c.get("key"))
                .is_none()
        {
            out.push(
                Diagnostic::new(
                    Code::Oos2024,
                    &tabla.path,
                    format!(
                        "`{tqn}` acepta `{}` y no declara `changes.key`",
                        necesita.join("` y `")
                    ),
                )
                .at(tabla
                    .section("writes")
                    .map_or_else(|| tabla.root.pos(), |w| w.pos()))
                .help(
                    "sin clave, `update` no dice qué fila cambia y `delete` no dice cuál \
                     retira. Declara `changes.key` con las columnas que identifican una fila \
                     —es la misma que hace fundible un incremento, y por eso no hay una \
                     segunda—, o deja en `writes` solo `insert`, que no señala a nada que ya \
                     esté",
                ),
            );
        }

        // OOS2018 · un filtro exigido que no es columna no lo puede poner nadie.
        // Cambia de sujeto respecto al binding, donde eran PROPIEDADES: lo exige
        // el origen, y el origen habla de columnas.
        if let Some((_, rf)) = tabla
            .section("reads")
            .and_then(|r| r.get("requiredFilters"))
        {
            for i in rf.items() {
                let Some(c) = i.as_str() else { continue };
                if !cols.contains(c) {
                    out.push(no_es_columna(
                        &tabla.path,
                        i,
                        format!("`{tqn}` exige filtrar por `{c}`, que no es columna suya"),
                        &tqn,
                        &cols,
                    ));
                }
            }
        }
    }

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

        // OOS2018 · v1alpha8 · la tabla existe, y tiene las columnas que esta
        // vista nombra. Es la misma regla que para `from.view` con el sujeto
        // cambiado, y es **la primera vez que llega hasta la columna física**:
        // hasta que la tabla no declaró `columns` no había contra qué comprobar,
        // así que el último tramo —el que toca el mundo— se creía.
        if let Some((_, nodo)) = from.get("table") {
            let referencia = nodo.as_str().unwrap_or("");
            match pkg.resolve_table(referencia, v) {
                None => out.push(
                    Diagnostic::new(
                        Code::Oos2018,
                        &v.path,
                        format!("`from.table: {referencia}` no existe"),
                    )
                    .at(nodo.pos())
                    .help(
                        "una vista sale de una tabla del paquete o de una dependencia. Una                          cadena que no llega al suelo no tiene raíz, y sin raíz no hay de dónde                          heredar etiquetas ni de dónde leer",
                    ),
                ),
                Some(tabla) => {
                    let cols = columnas(tabla);
                    let tqn = tabla.qname().unwrap_or_default();
                    let mios = campos(v);
                    if let Some(fs) = v.section("fields") {
                        for (k, val) in fs.entries() {
                            let Some(campo) = k.as_str() else { continue };
                            let col = mios.get(campo).cloned().unwrap_or_default();
                            if !cols.contains(&col) {
                                out.push(no_es_columna(
                                    &v.path,
                                    val,
                                    format!("`{qn}.{campo}` lee `{col}`, que `{tqn}` no tiene"),
                                    &tqn,
                                    &cols,
                                ));
                            }
                        }
                    }
                    if let Some(w) = v.section("where") {
                        for (k, _) in w.entries() {
                            let Some(col) = k.as_str() else { continue };
                            if !cols.contains(col) {
                                out.push(no_es_columna(
                                    &v.path,
                                    k,
                                    format!("`{qn}` filtra por `{col}`, que `{tqn}` no tiene"),
                                    &tqn,
                                    &cols,
                                ));
                            }
                        }
                    }
                }
            }
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

    // ── OOS2020 · lo que no se puede leer se debe materializar ──────────────
    //
    // En su propio recorrido y no dentro del de arriba: aquel tiene `continue`
    // en cada rama de error, y una regla que solo se comprueba cuando ninguna
    // otra falló es una regla que un día deja de comprobarse sin que se note.
    for v in pkg.of(Kind::View) {
        // Hay copia en la cadena —ella misma incluida—: se lee de ahí.
        if raiz_de_lectura(pkg, v).is_some() {
            continue;
        }
        let Ok(fila) = cadena(pkg, v) else { continue };
        let hoja = fila.last().copied().expect("una cadena tiene un eslabón");
        let Some(Fuente::Tabla(tqn)) = fuente(hoja) else {
            continue;
        };
        let Some(tabla) = pkg.table(&tqn) else {
            continue;
        };
        if se_lee(tabla) {
            continue;
        }
        let qn = v.qname().unwrap_or_default();
        let pos = v.section("from").map(|f| f.pos());
        let mut d = Diagnostic::new(
            Code::Oos2020,
            &v.path,
            format!("`{qn}` es virtual y `{tqn}` declara `reads: none`: no hay dónde preguntar"),
        )
        .help(
            "una tabla con `reads: none` no responde consultas, solo emite cambios — un tema se              escribe, no se pregunta. Esta vista promete un sitio donde preguntar que no existe,              y lo promete al compilar para fallar al consultar. Ponle `materialized`, o sácala de              una vista de abajo que ya lo lleve",
        );
        if let Some(p) = pos {
            d = d.at(p);
        }
        out.push(d);
    }

    // ── OOS2023 · la pareja decide la garantía ──────────────────────────────
    //
    // `witness: field` fecha por una columna, y eso es **at-least-once por
    // construcción**: la columna es siempre mayor o igual que sí misma, así que
    // el solape se re-entrega en cada refresco. Airbyte lo documenta con esas
    // palabras para su *cursor field*, que es el mismo mecanismo, y admite
    // además que se pueden **perder** filas si la columna no se mantiene al
    // modificar una.
    //
    // Con `upsert` o `retract` hay clave y re-entregar es idempotente. Con
    // `append` no hay con qué deduplicar: **cada refresco suma el solape, para
    // siempre**, y nadie lo ve hasta que llega la factura.
    //
    // Es la tercera regla que mira las dos caras a la vez, con `OOS2020` y
    // `OOS2021`, y como ellas **es sobre la copia y no sobre la tabla**: un log
    // de eventos fechado por una columna de tiempo es legítimo y existe. Lo que
    // no se puede es **mantener una copia suya**.
    for v in pkg.of(Kind::View) {
        // Solo si esta vista es la que se copia. Una virtual encima de una copia
        // no declara nada, y la de abajo ya se comprueba por su cuenta.
        if v.section("materialized").is_none() {
            continue;
        }
        let Ok(r) = raiz(pkg, v) else { continue };
        let Some(tabla) = r.tabla.as_deref().and_then(|qn| pkg.table(qn)) else {
            continue;
        };
        let por_columna = tabla
            .section("changes")
            .and_then(|c| c.get("witness"))
            .and_then(|(_, w)| w.as_str())
            == Some("field");
        if !(por_columna && modo(tabla) == Modo::Anexa) {
            continue;
        }
        let qn = v.qname().unwrap_or_default();
        let tqn = r.tabla.as_deref().unwrap_or_default();
        let mut d = Diagnostic::new(
            Code::Oos2023,
            &v.path,
            format!(
                "`{qn}` se copia de `{tqn}`, que declara `{{ mode: append, witness: field }}`: no \
                 hay clave con la que deduplicar lo que se re-entrega"
            ),
        )
        .help(
            "fechar por una columna es at-least-once: la columna es mayor o IGUAL que sí misma, \
             así que cada refresco vuelve a traer el borde y sin clave no hay forma de quitarlo — \
             el solape se acumula para siempre y no da ningún síntoma hasta la factura. La tabla \
             es legítima; lo que no se puede es mantener una copia suya. Declara `key` con \
             `mode: upsert`, o fecha por `witness: log` o `snapshot`, que nombran una posición \
             replayable",
        );
        if let Some(p) = v.section("materialized").map(|m| m.pos()) {
            d = d.at(p);
        }
        out.push(d);
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

        // ── OOS2022 · una propiedad sin campo no tiene de dónde salir ────────
        //
        // **La otra cara de haber retirado la federación.** `03-binding` §2.1
        // admitía que una entidad tuviera varios bindings, «cada uno cubre un
        // subconjunto de sus propiedades»: con eso, una cobertura parcial no
        // solo era legal, era el mecanismo, y preguntar de dónde sale una
        // propiedad no tenía respuesta local.
        //
        // v1alpha8 retira eso (`00-scope` §6): una entidad sale de UNA vista. Y
        // en cuanto no hay otro documento donde mirar, una propiedad sin campo
        // pasa de «la cubre otro» a «no la cubre nadie».
        //
        // Sin esto, la migración que esta versión pide produce el fallo que este
        // proyecto persigue: se escribe la vista con la mitad de los campos, la
        // entidad sigue declarando el doble, COMPILA EN VERDE, y las propiedades
        // huérfanas responden vacío para siempre. Se midió sobre un paquete de
        // tres propiedades y dos sin campo: `ok · sin errores`.
        //
        // **Y solo de v1alpha8.** Un documento anterior declaró su versión, y esa
        // versión sí admitía cobertura parcial: v1alpha1 porque otro binding la
        // cubría, y v1alpha7 porque el binding seguía en la gramática.
        // Aplicársela cambiaría lo que significa un documento ya escrito, y el
        // invariante que esta versión sostuvo en cinco peldaños es que **no
        // cambia un solo resultado de v1alpha1 a v1alpha7**.
        //
        // Se midió sin la puerta: `conformance/v1alpha7` caía de 13/13 a 12/13
        // —`valid/entity-backed-by-view`—, `acme-retail` dejaba de validar y
        // tres pruebas de `cache.rs` caían detrás de ella.
        if e.version()
            .is_some_and(|ver| ver >= crate::document::ApiVersion::V1Alpha8)
            && let Some(props) = e.section("properties")
        {
            for (k, cuerpo) in props.entries() {
                let Some(prop) = k.as_str() else { continue };
                // `derivedFrom` es la excepción, y es la única: una propiedad
                // derivada declara de qué otras sale, y eso ES su origen.
                // Exigirle además una columna sería exigirle que esté calculada
                // en la fuente — justo lo que la migración de
                // `Binding.properties.<x>.expression` deja de poder hacer.
                if cuerpo.get("derivedFrom").is_some() || expone.contains_key(prop) {
                    continue;
                }
                out.push(
                    Diagnostic::new(
                        Code::Oos2022,
                        &e.path,
                        format!("`{vista_qn}` no expone `{prop}`, que `{qn}` declara"),
                    )
                    .at(k.pos())
                    .help(
                        "una entidad sale de UNA vista, así que una propiedad que la vista no \
                         da no tiene de dónde salir: responde vacía y nada lo dice. Añade el \
                         campo a la vista, declara `derivedFrom` si de verdad se computa, o \
                         quita la propiedad. Con bindings esto era legal porque otro binding \
                         podía cubrirla; en v1alpha8 no hay otro",
                    ),
                );
            }
        }

        // ── OOS2021 · sin retractación no se mantiene lo mutable ─────────────
        //
        // El peor modo de fallo del motor, porque **no produce ningún síntoma**:
        // la vista se materializa, la consulta responde, los números salen — y
        // son los de antes. Sin este código se derivaría en silencio, y por eso
        // Foundry lo documenta como una limitación en vez de rechazarlo.
        //
        // Exige las tres cosas a la vez, y ninguna sobra: la entidad es MUTABLE
        // —un hecho ocurrido no se retira, y por eso un `nature: event` sí se
        // respalda de un `append`—; hay una COPIA en la cadena —una vista
        // virtual lee del origen, que sí tiene el estado presente—; y la raíz
        // SOLO ANEXA.
        if e.section("nature").and_then(|n| n.as_str()) == Some("entity")
            && raiz_de_lectura(pkg, v).is_some()
            && let Ok(r) = raiz(pkg, v)
            && let Some(tqn) = r.tabla.as_deref()
            && let Some(tabla) = pkg.table(tqn)
            && modo(tabla) == Modo::Anexa
        {
            out.push(
                Diagnostic::new(
                    Code::Oos2021,
                    &e.path,
                    format!(
                        "`{qn}` es `nature: entity` y se respalda de una copia de `{tqn}`, que \
                         solo anexa"
                    ),
                )
                .at(b.pos())
                .help(
                    "una entidad es una cosa que cambia y sigue siendo la misma, así que \
                     mantener su estado presente exige poder QUITAR lo que dejó de ser cierto. \
                     Un `changes.mode: append` no puede: lo que se copia no es el estado \
                     presente, es el histórico con las filas viejas dentro. La consulta \
                     responde, los números salen, y son los de antes. Un `nature: event` sí se \
                     respalda de un `append`",
                ),
            );
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

    /// **La guarda de invertibilidad, ejercida por las dos ramas.**
    ///
    /// Ningún documento OOS puede disparar esto hoy: el vocabulario de `View`
    /// en v1alpha8 es exactamente el fragmento invertible, así que un
    /// `groupBy` ni siquiera pasa de `OOS1005`. Se construye a mano, aquí,
    /// **por el mismo motivo por el que el IR de `ore-view` tiene `Agrupa`
    /// probado sin que ningún documento lo produzca**: la máquina se deja
    /// lista antes que el vocabulario, y una máquina que nadie ejerce no está
    /// lista, está escrita.
    ///
    /// La rama que importa es la segunda: el defecto es **«no invertible»**.
    /// Si algún día alguien amplía el vocabulario y se olvida de clasificar lo
    /// nuevo, esto niega la escritura en vez de concederla por descuido — y el
    /// censo de arriba hace que además no compile.
    #[test]
    fn la_guarda_de_invertibilidad_niega_lo_que_no_sabe_clasificar() {
        // Lo que hoy se puede escribir: renombra, recorta y proyecta. Invertible.
        let buena = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: empleados, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               freshness: 15m\n  \
               fields: { id: employee_id, pais: country }\n  \
               where: { deleted: \"false\" }\n",
        );
        assert_eq!(invertible(&buena), Ok(()));

        // Un constructor que la gramática todavía no tiene. El defecto es «no».
        let futura = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: por_pais, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               fields: { pais: country }\n  \
               groupBy: [country]\n",
        );
        assert_eq!(
            invertible(&futura),
            Err(NoInvertible::ConstruccionDesconocida {
                vista: "hr.por_pais".to_string(),
                clave: "groupBy".to_string(),
            })
        );

        // Y un campo que sale de calcularlo, que es lo primero que aparecería
        // el día que `fields` admita algo más que un nombre.
        let calculada = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: importes, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               fields: { total: \"precio * cantidad\" }\n",
        );
        assert_eq!(
            invertible(&calculada),
            Err(NoInvertible::CampoCalculado {
                vista: "hr.importes".to_string(),
                campo: "total".to_string(),
            })
        );

        // Una extensión de proveedor no decide nada sobre las filas, así que no
        // niega: `x-` es el mecanismo declarado para lo que no es del estándar.
        let con_extension = doc(
            Kind::View,
            "apiVersion: oos.dev/v1alpha8\n\
             kind: View\n\
             metadata: { name: empleados, namespace: hr }\n\
             spec:\n  \
               owner: team:rrhh\n  \
               from: { table: erp.employees }\n  \
               fields: { id: employee_id }\n  \
               x-acme-nota: \"la que usa nominas\"\n",
        );
        assert_eq!(invertible(&con_extension), Ok(()));
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

    // ── v1alpha8 · la tabla ─────────────────────────────────────────────────

    fn tabla(nombre: &str, spec: &str) -> Loaded {
        doc(
            Kind::Table,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
                 metadata: {{ name: {nombre}, namespace: erp }}\nspec:\n{spec}"
            ),
        )
    }

    /// La tabla de referencia: las dos caras puestas, y se deja leer.
    fn employees() -> Loaded {
        tabla(
            "employees",
            "  datasource: erp\n  object: public.employees\n  \
             columns:\n    employee_id: {}\n    national_id: {}\n    country: {}\n    deleted: {}\n  \
             reads: { predicatePushdown: [eq, in], fullScan: cheap }\n  \
             changes: { mode: retract, witness: log }\n",
        )
    }

    /// Un tema: se escribe, no se pregunta. Y solo anexa, para `OOS2021`.
    fn topico(modo: &str) -> Loaded {
        tabla(
            "orders",
            &format!(
                "  datasource: erp\n  object: orders.v2\n  \
                 columns:\n    order_id: {{}}\n    total: {{}}\n  \
                 reads: none\n  changes: {{ mode: {modo}, witness: log }}\n"
            ),
        )
    }

    fn vista8(nombre: &str, spec: &str) -> Loaded {
        doc(
            Kind::View,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: {{ name: {nombre}, namespace: hr }}\nspec:\n  owner: team:hr\n{spec}"
            ),
        )
    }

    fn codigos(pkg: &Package) -> Vec<Code> {
        let mut out = Vec::new();
        comprobar(pkg, &mut out);
        out.into_iter().map(|d| d.code).collect()
    }

    /// La cadena llega al suelo por el camino nuevo, y **lo que llega es lo
    /// mismo**: quien llama a `raiz()` no se entera de por cuál de los dos vino.
    #[test]
    fn la_raiz_atraviesa_una_tabla_y_da_la_misma_forma() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employees }\n  fields:\n    id: employee_id\n    dni: national_id\n  \
             where: { deleted: 'false' }\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        let r = raiz(&pkg, pkg.view("hr.empleados").unwrap()).unwrap();
        assert_eq!(r.datasource, "erp");
        assert_eq!(r.objeto, "public.employees");
        assert_eq!(
            r.columnas.get("dni").map(String::as_str),
            Some("national_id")
        );
        assert_eq!(
            r.filtros,
            vec![("deleted".to_string(), vec!["false".to_string()])]
        );
        // Lo único que cambia: ahora hay un documento que nombrar.
        assert_eq!(r.tabla.as_deref(), Some("erp.employees"));
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// **OOS2018 llega hasta el suelo.** En v1alpha7 esto compilaba, y no por
    /// indulgencia: no había ningún documento contra el que comprobarlo.
    #[test]
    fn un_campo_que_no_es_columna_de_la_tabla_no_compila() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employees }\n  fields:\n    dni: nif\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2018]);
    }

    #[test]
    fn un_filtro_que_no_es_columna_de_la_tabla_no_compila() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employees }\n  fields:\n    id: employee_id\n  where: { borrado: 'false' }\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2018]);
    }

    #[test]
    fn una_tabla_que_no_existe_no_es_una_raiz() {
        let v = vista8(
            "empleados",
            "  from: { table: erp.employes }\n  fields:\n    id: employee_id\n",
        );
        let pkg = paquete(vec![config(), employees(), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2018]);
    }

    /// Las dos caras nombran columnas suyas, y eso un esquema no lo puede
    /// mirar: exige que el campo ESTÉ, no puede saber si lo que dice EXISTE.
    #[test]
    fn las_dos_caras_nombran_columnas_de_la_tabla() {
        let mala = tabla(
            "orders",
            "  datasource: erp\n  object: orders.v2\n  columns:\n    order_id: {}\n  \
             reads: { requiredFilters: [tenant_id] }\n  \
             changes: { mode: upsert, key: [order_key], witness: field, field: updated_at }\n",
        );
        let pkg = paquete(vec![config(), mala]);
        // Tres nombres inventados, tres diagnósticos, un solo código.
        assert_eq!(
            codigos(&pkg),
            vec![Code::Oos2018, Code::Oos2018, Code::Oos2018]
        );
    }

    #[test]
    fn la_fuente_de_una_tabla_se_declara_en_el_manifiesto() {
        let t = doc(
            Kind::Table,
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: a, namespace: crm }\nspec:\n  \
             datasource: salesforce\n  object: Account\n  columns:\n    Id: {}\n  \
             reads: none\n  changes: { mode: none, witness: none }\n",
        );
        let pkg = paquete(vec![config(), t]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2004]);
    }

    /// **OOS2020 · lo que no se puede leer se debe materializar.**
    #[test]
    fn una_vista_virtual_sobre_algo_que_no_se_lee_no_compila() {
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n",
        );
        let pkg = paquete(vec![config(), topico("upsert"), v]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2020]);
    }

    #[test]
    fn con_la_copia_puesta_la_misma_vista_compila() {
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n  \
             materialized: { datasource: erp, table: cache.pedidos }\n",
        );
        let pkg = paquete(vec![config(), topico("upsert"), v]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    // ── OOS2023 · la pareja decide la garantía ─────────────────────────────

    /// Una tabla fechada por columna, con el modo que se le pase.
    fn por_columna(modo: &str, clave: &str) -> Loaded {
        tabla(
            "clicks",
            &format!(
                "  datasource: erp\n  object: public.clicks\n  \
                 columns:\n    click_id: {{}}\n    ocurrio_en: {{}}\n  \
                 reads: {{ fullScan: cheap }}\n  \
                 changes: {{ mode: {modo}, {clave}witness: field, field: ocurrio_en }}\n"
            ),
        )
    }

    fn copia_de_clicks() -> Loaded {
        vista8(
            "clics",
            "  from: { table: erp.clicks }\n  fields:\n    id: click_id\n    \
             cuando: ocurrio_en\n  materialized: { datasource: erp, table: cache.clics }\n",
        )
    }

    /// **`{ witness: field, mode: append }` no se puede mantener.**
    ///
    /// Fechar por una columna es at-least-once —la columna es mayor o IGUAL que
    /// sí misma, así que el borde se re-entrega— y sin clave no hay con qué
    /// quitarlo. El solape se acumula **para siempre**, y no da ningún síntoma:
    /// la copia responde, los números salen, y son de más.
    ///
    /// Antes de este código el árbol no solo lo aceptaba: `ore view`
    /// **recomendaba `INCREMENTAL`** sobre esta pareja exacta.
    #[test]
    fn una_copia_fechada_por_columna_y_solo_anexa_no_compila() {
        let pkg = paquete(vec![config(), por_columna("append", ""), copia_de_clicks()]);
        assert_eq!(codigos(&pkg), vec![Code::Oos2023]);
    }

    /// **Y el rechazo es de la COMBINACIÓN, no del modo ni del testigo.**
    ///
    /// Sin esta prueba, `OOS2023` podría estar mirando solo `witness: field` y
    /// nadie lo notaría: prohibiría fechar por columna, que es legítimo y es lo
    /// único que muchos orígenes saben hacer. Con clave, re-entregar es
    /// idempotente y no pasa nada.
    #[test]
    fn la_misma_tabla_con_clave_si_se_copia() {
        let pkg = paquete(vec![
            config(),
            por_columna("upsert", "key: [click_id], "),
            copia_de_clicks(),
        ]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// **Y es sobre la copia, no sobre la tabla.** Un log de eventos fechado por
    /// una columna de tiempo es legítimo y existe; lo que no se puede es
    /// mantener una copia suya. Sin `materialized`, no hay nada que rechazar.
    #[test]
    fn la_tabla_que_solo_anexa_es_legitima_mientras_nadie_la_copie() {
        let virtual_ = vista8(
            "clics",
            "  from: { table: erp.clicks }\n  fields:\n    id: click_id\n",
        );
        let pkg = paquete(vec![config(), por_columna("append", ""), virtual_]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// La distinción raíz / raíz de lectura, que es todo el asunto: si la regla
    /// mirara la raíz, esto fallaría y obligaría a copiar dos veces lo mismo.
    #[test]
    fn una_virtual_sobre_una_copia_sobre_un_topico_lee_de_la_copia() {
        let abajo = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n    total: total\n  \
             materialized: { datasource: erp, table: cache.pedidos }\n",
        );
        let arriba = vista8(
            "iberia",
            "  from: { view: pedidos }\n  fields:\n    id: id\n",
        );
        let pkg = paquete(vec![config(), topico("upsert"), abajo, arriba]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
        // Y la raíz de lectura de la de arriba es la copia, no el tópico.
        let a = pkg.view("hr.iberia").unwrap();
        assert_eq!(
            raiz_de_lectura(&pkg, a).and_then(|v| v.qname()).as_deref(),
            Some("hr.pedidos")
        );
    }

    fn entidad(naturaleza: &str, extra: &str) -> Loaded {
        doc(
            Kind::Entity,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: {{ name: Pedido, namespace: hr }}\nspec:\n  nature: {naturaleza}\n\
                 {extra}  backedBy: pedidos\n  properties:\n    id: {{ type: String }}\n"
            ),
        )
    }

    fn copia_de(modo: &str) -> Vec<Loaded> {
        vec![
            config(),
            topico(modo),
            vista8(
                "pedidos",
                "  from: { table: erp.orders }\n  fields:\n    id: order_id\n  \
                 materialized: { datasource: erp, table: cache.pedidos }\n",
            ),
        ]
    }

    /// **OOS2021 · sin retractación no se mantiene lo mutable.** El peor modo
    /// de fallo del motor: sin este código la copia se deriva en silencio y los
    /// números salen — los de antes.
    #[test]
    fn una_copia_de_lo_que_solo_anexa_no_respalda_una_entidad_mutable() {
        let mut docs = copia_de("append");
        docs.push(entidad("entity", "  primaryKey: [id]\n"));
        let pkg = paquete(docs);
        assert_eq!(codigos(&pkg), vec![Code::Oos2021]);
    }

    /// Y la mitad que **sí** compila: un hecho ocurrido no se retira.
    #[test]
    fn una_copia_de_lo_que_solo_anexa_si_respalda_un_evento() {
        let mut docs = copia_de("append");
        docs.push(entidad("event", "  timeKey: id\n"));
        let pkg = paquete(docs);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// Y con retractación, lo mutable se mantiene: la regla es sobre el modo,
    /// no sobre materializar.
    #[test]
    fn una_copia_de_lo_que_retracta_si_respalda_una_entidad() {
        let mut docs = copia_de("retract");
        docs.push(entidad("entity", "  primaryKey: [id]\n"));
        let pkg = paquete(docs);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// Una vista VIRTUAL sobre un `append` que se deja leer compila: se lee del
    /// origen, que sí tiene el estado presente. La regla es sobre la COPIA.
    #[test]
    fn sin_copia_un_append_si_respalda_una_entidad() {
        let plana = tabla(
            "orders",
            "  datasource: erp\n  object: orders.v2\n  columns:\n    order_id: {}\n  \
             reads: { fullScan: cheap }\n  changes: { mode: append, witness: log }\n",
        );
        let v = vista8(
            "pedidos",
            "  from: { table: erp.orders }\n  fields:\n    id: order_id\n",
        );
        let pkg = paquete(vec![
            config(),
            plana,
            v,
            entidad("entity", "  primaryKey: [id]\n"),
        ]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
    }

    /// Las dos versiones en el mismo paquete. Si esto fallara, la migración
    /// sería un salto, y un salto sobre un árbol grande no se da.
    #[test]
    fn una_vista_v1alpha7_y_una_tabla_v1alpha8_conviven() {
        let pkg = paquete(vec![config(), base(), employees(), {
            vista8(
                "nuevos",
                "  from: { table: erp.employees }\n  fields:\n    id: employee_id\n",
            )
        }]);
        assert!(codigos(&pkg).is_empty(), "{:?}", codigos(&pkg));
        // Y las dos llegan al mismo suelo por caminos distintos.
        let vieja = raiz(&pkg, pkg.view("hr.empleados").unwrap()).unwrap();
        let nueva = raiz(&pkg, pkg.view("hr.nuevos").unwrap()).unwrap();
        assert_eq!(vieja.datasource, nueva.datasource);
        assert_eq!(vieja.objeto, nueva.objeto);
        assert_eq!(vieja.tabla, None);
        assert_eq!(nueva.tabla.as_deref(), Some("erp.employees"));
    }
}
