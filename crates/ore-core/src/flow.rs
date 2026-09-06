//! Flujo de información — la familia `OOS4xxx`.
//!
//! Es la fase que define el producto, y toda ella se apoya en **una regla**:
//!
//! > La información con etiqueta `L` no debe alcanzar un conducto con
//! > autorización `C` salvo que `L ⊑ C`, o que atraviese un desclasificador
//! > autorizado.
//!
//! Todo lo que sigue —retículos, propagación, conductos, desclasificadores— es
//! maquinaria para poder comprobar esa frase. Y se comprueba **sin red, sin
//! credenciales y sin tocar un solo dato**: es lo que hace que un auditor externo
//! pueda verificar la gobernanza clonando el repositorio.
//!
//! El recorrido de `derivedFrom` es el mismo que estrenó `OOS3004` con las
//! unidades. Aquí transporta etiquetas.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package};
use crate::parse::Node;
use std::collections::{BTreeMap, BTreeSet};

// ── Retículos ───────────────────────────────────────────────────────────────

/// El eje de un retículo. Decide qué se compara y con qué combinador.
///
/// Confidencialidad pregunta *cuánto daño si esto se filtra*; integridad,
/// *cuánto daño si esto es falso*. Son ortogonales: un dato puede ser público y
/// crítico a la vez —el estado de un pedido no es secreto y escribirlo mal
/// cuesta dinero— y por eso hacen falta los dos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// Gobierna lecturas. Combina por `join` = máximo.
    #[default]
    Confidentiality,
    /// Gobierna escrituras. Combina por `meet` = mínimo.
    Integrity,
}

impl Axis {
    /// El combinador que el eje implica. **No se declara** — es derivable, y un
    /// campo derivable no es declarable (P2).
    pub const fn combinador(self) -> &'static str {
        match self {
            Axis::Confidentiality => "max",
            Axis::Integrity => "min",
        }
    }
}

/// Un retículo: etiquetas y su orden parcial. El orden **es** la secuencia.
#[derive(Debug, Clone)]
pub struct Lattice {
    pub qname: String,
    pub levels: Vec<String>,
    pub axis: Axis,
    /// Qué exige la clasificación, por nivel: `nivel → naturalezas`.
    ///
    /// Vive en el retículo y no en la regla porque es lo que hace que
    /// **importar la clasificación importe su exigencia**. Y nombra **clases**
    /// de regla, no solo un nivel: sin eso una comprobación de nulos
    /// descargaría lo que un paquete de protección de datos pedía como
    /// política, que es el error de categoría más frecuente
    /// (`v1alpha3/01-gobierno` §6.1).
    pub requires_governance: BTreeMap<String, Vec<String>>,
}

impl Lattice {
    pub fn index(&self, level: &str) -> Option<usize> {
        self.levels.iter().position(|l| l == level)
    }

    /// ¿Está `nivel` en `piso` o por encima?
    ///
    /// `None` si alguno de los dos no pertenece al retículo — que no es lo
    /// mismo que `false`, y confundirlos convertiría una etiqueta mal escrita
    /// en una propiedad que parece no seleccionada.
    pub fn ge(&self, nivel: &str, piso: &str) -> Option<bool> {
        Some(self.index(nivel)? >= self.index(piso)?)
    }
}

/// `oos.maturity` es estándar de la especificación y está siempre activo, lo
/// declare el paquete o no.
///
/// El orden es ASCENDENTE POR RESTRICTIVIDAD, igual que todo retículo, y por
/// eso `STABLE` es el fondo: es lo que puede servirse a cualquier consumidor.
/// Tres partes normativas lo fijan en esa dirección y no en la contraria:
///
/// - `ore promote` es un **desclasificador** y BAJA `DRAFT` a `REVIEWED` a
///   `STABLE` (`04-flow.md` §3, §5). Desclasificar es bajar; luego
///   `STABLE ⊑ REVIEWED ⊑ DRAFT`.
/// - La suite —normativa— razona en `diff/downgrade-maturity` que `DRAFT` es
///   **invisible para los consumidores de producción**. Un `contextSurface`
///   que admite `STABLE` y rechaza `DRAFT` solo es expresable con este orden.
/// - Las autorizaciones de ejemplo de `04-flow.md` §4 solo son coherentes así:
///   `cache: STABLE` admite únicamente lo estable, y `log: DEPRECATED` —el
///   techo— lo admite todo. Con el orden inverso, `cache` aceptaría un
///   borrador y `contextSurface` rechazaría lo estable.
fn maturity() -> Lattice {
    Lattice {
        qname: "oos.maturity".into(),
        levels: ["STABLE", "REVIEWED", "DRAFT", "DEPRECATED"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        axis: Axis::Confidentiality,
        // `oos.maturity` no exige gobierno: es el ciclo de vida de un
        // documento, no una clasificación de sensibilidad. Obligar a cubrir
        // todo lo que no sea STABLE convertiría un borrador en un error.
        requires_governance: BTreeMap::new(),
    }
}

pub fn lattices(pkg: &Package) -> BTreeMap<String, Lattice> {
    let mut out = BTreeMap::new();
    let m = maturity();
    out.insert(m.qname.clone(), m);
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Lattice) {
        let Some(q) = d.qname() else { continue };
        let levels: Vec<String> = d
            .section("levels")
            .map(|n| {
                n.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // Sin `axis`, confidencialidad: es lo que hace que todo retículo de
        // v1alpha1 siga significando lo mismo sin tocar un fichero.
        let axis = match d.section("axis").and_then(|n| n.as_str()) {
            Some("integrity") => Axis::Integrity,
            _ => Axis::Confidentiality,
        };
        let requires_governance: BTreeMap<String, Vec<String>> = d
            .section("requiresGovernance")
            .map(|n| {
                n.entries()
                    .iter()
                    .filter_map(|(k, v)| {
                        let nivel = k.as_str()?.to_string();
                        let naturalezas = v
                            .items()
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect();
                        Some((nivel, naturalezas))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.insert(
            q.clone(),
            Lattice {
                qname: q,
                levels,
                axis,
                requires_governance,
            },
        );
    }
    out
}

// ── Etiquetas ───────────────────────────────────────────────────────────────

/// De dónde salió una etiqueta. Es lo único que distingue `OOS4002` de
/// `OOS4001`, y la distinción no es cosmética: la directa la detecta cualquier
/// linter, la computada no la hace nadie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Escrita en la propia propiedad.
    Declared,
    /// Heredada de la entidad, o del datasource al que se enlaza.
    Inherited,
    /// **Computada** por el compilador propagando `join` desde los orígenes de
    /// una derivación. Nadie la escribió en ninguna parte.
    Computed,
}

/// Etiquetas efectivas de **una propiedad**: retículo → (nivel, de dónde salió).
pub type Labels = BTreeMap<String, (String, Origin)>;

/// Etiquetas efectivas de **una entidad**: propiedad → sus etiquetas.
type EntityLabels = BTreeMap<String, Labels>;

/// Etiquetas efectivas de todo el paquete: `entidad.propiedad` → retículo →
/// nivel.
///
/// Se expone para [`governance`](crate::governance), que necesita exactamente
/// esto y **no debe recalcularlo**: un objetivo que viera solo las etiquetas
/// declaradas dejaría fuera las heredadas de la entidad y las computadas por
/// propagación, que son las dos que nadie escribió y por tanto las que más
/// falta hace gobernar.
pub fn efectivas(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        for (prop, etiquetas) in propagar_solo(pkg, e, lat) {
            out.insert(
                format!("{qn}.{prop}"),
                etiquetas
                    .into_iter()
                    .map(|(ret, (nivel, _))| (ret, nivel))
                    .collect(),
            );
        }
    }
    out
}

fn read_labels(n: &Node) -> Vec<(String, String, crate::diag::Pos)> {
    n.get("labels")
        .map(|(_, l)| {
            l.entries()
                .iter()
                .filter_map(|(k, v)| {
                    Some((k.as_str()?.to_string(), v.as_str()?.to_string(), v.pos()))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── El chequeo completo ─────────────────────────────────────────────────────

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let lat = lattices(pkg);

    // 1 · Toda etiqueta escrita debe pertenecer a un retículo. Sin esto, el
    //     resto de la fase compararía contra la nada.
    etiquetas_conocidas(pkg, &lat, &mut out);
    if !out.is_empty() {
        return out;
    }

    // 2 · Herencia y propagación.
    let mut efectivas: BTreeMap<String, EntityLabels> = BTreeMap::new();
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        efectivas.insert(qn, propagar(pkg, e, &lat, &mut out));
    }
    if !out.is_empty() {
        return out;
    }

    // 3 · Los conductos y la regla de flujo.
    let conductos = clearances(pkg, &lat);
    vistas_materializadas(pkg, &lat, &efectivas, &conductos, &mut out);
    indices_de_topologia(pkg, &lat, &efectivas, &conductos, &mut out);

    // 4 · Desclasificadores y valores de ejemplo.
    desclasificadores(pkg, &mut out);
    ejemplos(pkg, &lat, &efectivas, &mut out);

    out
}

// ── OOS4003 ─────────────────────────────────────────────────────────────────

fn etiquetas_conocidas(pkg: &Package, lat: &BTreeMap<String, Lattice>, out: &mut Vec<Diagnostic>) {
    let mut revisar = |d: &Loaded, n: &Node| {
        for (ret, nivel, pos) in read_labels(n) {
            // Una etiqueta de un retículo de integridad no es asunto de esta
            // fase: la comprueba `effect` con `OOS7003`. Emitir `OOS4003` aquí
            // sería contestar en el eje equivocado.
            if lat.get(&ret).is_some_and(|l| l.axis == Axis::Integrity) {
                continue;
            }
            let malo = match lat.get(&ret) {
                None => Some(format!(
                    "no hay ningún retículo `{ret}` declarado ni importado"
                )),
                Some(l) if l.index(&nivel).is_none() => Some(format!(
                    "`{nivel}` no es un nivel de `{ret}`; sus niveles son {}",
                    l.levels.join(" ⊑ ")
                )),
                Some(_) => None,
            };
            if let Some(m) = malo {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &d.path,
                        format!("etiqueta `{ret}:{nivel}`: {m}"),
                    )
                    .at(pos)
                    .help(
                        "el esquema comprueba que la clave es un nombre cualificado y el \
                             valor un identificador, y ahí se acaba lo que puede saber: que ese \
                             nivel exista en ese retículo es una relación entre documentos",
                    ),
                );
            }
        }
    };

    for e in pkg.entities() {
        if let Some((_, m)) = e.root.get("metadata") {
            revisar(e, m);
        }
        if let Some(ps) = e.section("properties") {
            for (_, v) in ps.entries() {
                revisar(e, v);
            }
        }
    }
    // Y la vista, desde que admite `oos.maturity`. Solo su `metadata`: no
    // tiene etiquetas en ningún otro sitio, y `validate` ya rechazó cualquier
    // clave que no sea esa. Lo que queda por comprobar es lo que un esquema no
    // puede — que `DRFAT` no es un nivel de `oos.maturity` — y es exactamente
    // la pregunta que esta función existe para contestar.
    for v in pkg.of(Kind::View) {
        if let Some((_, m)) = v.root.get("metadata") {
            revisar(v, m);
        }
    }
    for c in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
        for ds in c.section("datasources").map(|n| n.items()).unwrap_or(&[]) {
            revisar(c, ds);
        }
    }
}

// ── OOS4008 · OOS4012 · propagación ─────────────────────────────────────────

fn propagar(
    pkg: &Package,
    e: &Loaded,
    lat: &BTreeMap<String, Lattice>,
    out: &mut Vec<Diagnostic>,
) -> EntityLabels {
    let qn = e.qname().unwrap_or_default();

    // Heredadas de la entidad: lo cierto del conjunto se declara una vez.
    let mut heredadas: Labels = BTreeMap::new();
    if let Some((_, m)) = e.root.get("metadata") {
        for (r, n, _) in read_labels(m) {
            heredadas.insert(r, (n, Origin::Inherited));
        }
    }

    // Heredadas del datasource: la ubicación física es un hecho del mundo, no
    // una decisión de modelado, así que se computa.
    //
    // Por los bindings y por la vista, y la vista **atraviesa la cadena**: una
    // entidad respaldada por una vista sobre otra vista hereda del datasource
    // en el que la cadena termina, porque de ahí es de donde salen los bytes.
    // `vistas::datasources_de` es quien sabe llegar; aquí solo se etiqueta.
    for dsref in crate::vistas::datasources_de(pkg, e) {
        for c in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
            for ds in c.section("datasources").map(|n| n.items()).unwrap_or(&[]) {
                if ds.get("name").and_then(|(_, v)| v.as_str()) != Some(dsref.as_str()) {
                    continue;
                }
                for (r, n, _) in read_labels(ds) {
                    let subir = match (heredadas.get(&r), lat.get(&r)) {
                        (Some((actual, _)), Some(l)) => l.index(&n) > l.index(actual),
                        (None, _) => true,
                        _ => false,
                    };
                    if subir {
                        heredadas.insert(r, (n, Origin::Inherited));
                    }
                }
            }
        }
    }

    let conceptos = crate::significado::conceptos(pkg);

    let mut efectivas: EntityLabels = BTreeMap::new();
    let Some(ps) = e.section("properties") else {
        return BTreeMap::new();
    };

    // Primera pasada: declaradas y heredadas.
    for (k, v) in ps.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let derivada = v.get("derivedFrom").is_some();
        let propias = read_labels(v);

        // OOS4008 · una derivada no declara etiqueta. Falla AUNQUE el valor
        // declarado sea el correcto: si se admitiera cuando coincide, el día
        // que alguien rebaje un origen la etiqueta seguiría mintiendo.
        if derivada && !propias.is_empty() {
            out.push(
                Diagnostic::new(
                    Code::Oos4008,
                    &e.path,
                    format!("`{qn}.{nombre}` es derivada y declara etiqueta"),
                )
                .at(propias[0].2)
                .help(
                    "la etiqueta de una derivada la computa el compilador con `join` sobre sus \
                     orígenes. Declararla es un error aunque el valor sea el correcto hoy: una \
                     etiqueta que un humano puede desincronizar del código acaba mintiendo, y \
                     firmarla criptográficamente lo empeora — parece verificada",
                ),
            );
            continue;
        }

        let mut ls = heredadas.clone();

        // v1alpha4 · el concepto es la TERCERA fuente de herencia, y entra aquí
        // —dentro de la propagación que ya existía— en vez de en un módulo
        // propio. No es comodidad: si viviera aparte habría dos sitios que
        // computan la etiqueta de una propiedad, y la regla de cobertura vería
        // uno de los dos. **Lo único nuevo es el nivel al que se aplica lo que
        // ya estaba** — `OOS4012`, se puede elevar y no rebajar, sin cambiar
        // una letra.
        if let Some(c) = crate::significado::mapeo(v).and_then(|q| conceptos.get(&q)) {
            for (r, n) in &c.labels {
                let subir = match (ls.get(r), lat.get(r)) {
                    (Some((actual, _)), Some(l)) => l.index(n) > l.index(actual),
                    (None, _) => true,
                    _ => false,
                };
                if subir {
                    ls.insert(r.clone(), (n.clone(), Origin::Inherited));
                }
            }
        }

        for (r, n, pos) in propias {
            // OOS4012 · elevar es legítimo; rebajar, no.
            //
            // Se compara contra `ls` y no contra `heredadas`, y el cambio lo
            // forzó un caso: `ls` es todo lo heredado —la entidad, el
            // `datasource` y, desde v1alpha4, el concepto— y `heredadas` es
            // solo lo primero. Mientras el concepto no existía las dos eran lo
            // mismo, y la distinción no se veía. Con `is`, comparar contra
            // `heredadas` dejaba que una propiedad **rebajara la clasificación
            // de su concepto en silencio**, que es exactamente lo que esta
            // regla existe para impedir.
            let heredado = ls.get(&r).map(|(nivel, _)| nivel.clone());
            if let (Some(heredado), Some(l)) = (heredado, lat.get(&r))
                && l.index(&n) < l.index(&heredado)
            {
                out.push(
                    Diagnostic::new(
                        Code::Oos4012,
                        &e.path,
                        format!("`{qn}.{nombre}` rebaja `{r}` de `{heredado}` a `{n}`"),
                    )
                    .at(pos)
                    .help(
                        "restringir siempre se puede; relajar es una decisión que exige \
                             tomarse donde se declaró la restricción, no en una propiedad suelta",
                    ),
                );
                continue;
            }
            ls.insert(r, (n, Origin::Declared));
        }
        efectivas.insert(nombre.to_string(), ls);
    }

    // Segunda pasada: propagación por derivación. `join` = la más restrictiva.
    for (k, v) in ps.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let Some((_, from)) = v.get("derivedFrom") else {
            continue;
        };
        let mut ls = heredadas.clone();
        for r in from.items() {
            let Some(q) = r.as_str() else { continue };
            let Some((ent, prop)) = q.rsplit_once('.') else {
                continue;
            };
            let origen = if ent == qn {
                efectivas.get(prop).cloned()
            } else {
                pkg.entity(ent).map(|o| {
                    propagar_solo(pkg, o, lat)
                        .get(prop)
                        .cloned()
                        .unwrap_or_default()
                })
            };
            for (ret, (nivel, _)) in origen.unwrap_or_default() {
                let subir = match (ls.get(&ret), lat.get(&ret)) {
                    (Some((actual, _)), Some(l)) => l.index(&nivel) > l.index(actual),
                    (None, _) => true,
                    _ => false,
                };
                if subir {
                    ls.insert(ret, (nivel, Origin::Computed));
                }
            }
        }

        // v1alpha4 · el concepto **también entra en el join**, y no entrarlo
        // era un agujero: esta pasada empieza de cero con `heredadas` y pisaba
        // lo que la primera había heredado del concepto, así que **añadir
        // `derivedFrom` a una propiedad mapeada le borraba la clasificación en
        // silencio.** Compilaba sin una sola regla lo que sin `derivedFrom`
        // rompía con `OOS8001`.
        //
        // Entra como un origen más y con la misma dirección —solo sube—, que
        // es lo coherente con lo que el concepto es: **un suelo de
        // clasificación, no un valor**. Si fijara un valor, importar
        // vocabulario ajeno obligaría a aceptar su laxitud.
        if let Some(c) = crate::significado::mapeo(v).and_then(|q| conceptos.get(&q)) {
            for (ret, nivel) in &c.labels {
                let subir = match (ls.get(ret), lat.get(ret)) {
                    (Some((actual, _)), Some(l)) => l.index(nivel) > l.index(actual),
                    (None, _) => true,
                    _ => false,
                };
                if subir {
                    ls.insert(ret.clone(), (nivel.clone(), Origin::Inherited));
                }
            }
        }

        efectivas.insert(nombre.to_string(), ls);
    }

    efectivas
}

/// Propagación sin emitir diagnósticos, para resolver derivaciones que cruzan
/// entidades sin duplicar los errores de la otra.
fn propagar_solo(pkg: &Package, e: &Loaded, lat: &BTreeMap<String, Lattice>) -> EntityLabels {
    let mut descartar = Vec::new();
    propagar(pkg, e, lat, &mut descartar)
}

// ── OOS4001 · OOS4002 · la vista materializada ──────────────────────────────

/// Una vista con `materialized` **copia datos**, y una copia instancia un
/// conducto: `materialization.payload`, el mismo que el eje `payload` del
/// binding, porque es la misma cosa con otro dueño.
///
/// Lo que fluye por él es **cada campo de la vista**, y lo que lleva puesto
/// cada campo se sabe por dos vías:
///
/// - la del datasource raíz, que etiqueta a todo lo que sale de él;
/// - la de **cada entidad cuya cadena pasa por esta vista**: la entidad
///   etiqueta sus propiedades, las propiedades nombran campos de SU vista, y
///   `vistas::proyectar` baja esos nombres hasta la que se copia.
///
/// La segunda vía es la que vale: una entidad puede declarar `nationalId: high`
/// sobre una vista de tres eslabones, y **la de abajo, que es la que se
/// materializa, no lo sabe**. Sin esto se copiaría en claro un dato que la
/// entidad clasificó — y compilaría.
fn vistas_materializadas(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    efectivas: &BTreeMap<String, EntityLabels>,
    conductos: &BTreeMap<String, Labels>,
    out: &mut Vec<Diagnostic>,
) {
    let conducto = "materialization.payload";
    for v in pkg.of(Kind::View) {
        let Some(mat) = v.section("materialized") else {
            continue;
        };
        let vqn = v.qname().unwrap_or_default();

        // OOS4011 · omitir un conducto no es dejarlo abierto: es cerrarlo.
        let Some(autorizacion) = conductos.get(conducto) else {
            out.push(
                Diagnostic::new(
                    Code::Oos4011,
                    &v.path,
                    format!("el conducto `{conducto}` no tiene autorización declarada"),
                )
                .at(mat.pos())
                .help(format!(
                    "un conducto sin autorización es ⊥ y no admite nada. Declara `{conducto}`                      en la política de conductos, o quita `materialized` de la vista"
                )),
            );
            continue;
        };

        // Qué lleva puesto cada campo. Se acumula por `join` —el más
        // restrictivo— porque dos entidades pueden nombrar el mismo campo con
        // clasificaciones distintas, y la copia es una.
        let mut por_campo: BTreeMap<String, Labels> = BTreeMap::new();
        let subir = |ls: &mut Labels, ret: &str, nivel: &str, origen: Origin| {
            let sube = match (ls.get(ret), lat.get(ret)) {
                (Some((actual, _)), Some(l)) => l.index(nivel) > l.index(actual),
                (None, _) => true,
                _ => false,
            };
            if sube {
                ls.insert(ret.to_string(), (nivel.to_string(), origen));
            }
        };

        // Vía 1 · el datasource raíz.
        if let Ok(raiz) = crate::vistas::raiz(pkg, v) {
            for c in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
                for ds in c.section("datasources").map(|n| n.items()).unwrap_or(&[]) {
                    if ds.get("name").and_then(|(_, x)| x.as_str())
                        != Some(raiz.datasource.as_str())
                    {
                        continue;
                    }
                    for (r, n, _) in read_labels(ds) {
                        for campo in raiz.columnas.keys() {
                            subir(
                                por_campo.entry(campo.clone()).or_default(),
                                &r,
                                &n,
                                Origin::Inherited,
                            );
                        }
                    }
                }
            }
        }

        // Vía 2 · cada entidad **de la misma cadena**, esté arriba o abajo.
        //
        // # Las dos direcciones, y por qué las dos
        //
        // La copia y la vista de la entidad están en la misma cadena, y da
        // igual quién derive de quién: **se copia el mismo dato**. Hacia abajo,
        // porque un eslabón inferior contiene las mismas columnas; hacia
        // arriba, porque una vista derivada es una proyección de esas mismas
        // columnas. La etiqueta es del dato, no del eslabón.
        //
        // Esto **solo miraba hacia abajo**, y el agujero se midió: `nationalId`
        // declarada `critical` se copiaba por un conducto de `high` y
        // compilaba — exactamente lo que el comentario de esta función dice que
        // no puede pasar. Y en silencio, porque el `else { continue }` de una
        // entidad que no toca esta vista es indistinguible del de una que sí la
        // toca por el otro lado. `pruebas-de-fuego/medida-el-sello-no-sube.py`.
        //
        // # Por qué no hace falta invertir nada
        //
        // `proyectar` resuelve **hacia abajo desde donde se le pida**, así que
        // para una copia por encima basta llamarla al revés: da campo de la
        // copia → campo de la vista de la entidad. Se recorren los campos de la
        // copia buscando su origen, en vez de las propiedades buscando su
        // destino. Un mapa invertido no haría falta ni sería seguro — dos
        // campos pueden venir del mismo, y entonces **los dos** llevan lo suyo,
        // que es lo que sale solo al recorrer en esta dirección.
        for e in pkg.entities() {
            let Some(suya) = crate::vistas::respaldo(pkg, e) else {
                continue;
            };
            let sqn = suya.qname().unwrap_or_default();
            // `prop -> campo de la copia`, resuelto por el lado que exista.
            let mapa: BTreeMap<String, String> =
                if let Some(abajo) = crate::vistas::proyectar(pkg, suya, &vqn) {
                    abajo
                } else if let Some(arriba) = crate::vistas::proyectar(pkg, v, &sqn) {
                    // Al revés: `campo de la copia -> campo de la vista`, que es
                    // el nombre de la propiedad. Se da la vuelta al leerlo, y un
                    // origen repetido reparte la etiqueta a sus dos campos.
                    let mut m: BTreeMap<String, String> = BTreeMap::new();
                    for (campo, propiedad) in arriba {
                        m.insert(propiedad, campo);
                    }
                    m
                } else {
                    continue;
                };
            let eqn = e.qname().unwrap_or_default();
            let Some(props) = efectivas.get(&eqn) else {
                continue;
            };
            for (prop, labels) in props {
                let Some(campo) = mapa.get(prop) else {
                    continue;
                };
                for (ret, (nivel, origen)) in labels {
                    subir(
                        por_campo.entry(campo.clone()).or_default(),
                        ret,
                        nivel,
                        *origen,
                    );
                }
            }
        }

        for (campo, labels) in por_campo {
            for (ret, (nivel, origen)) in labels {
                let Some(l) = lat.get(&ret) else { continue };
                let permitido = autorizacion
                    .get(&ret)
                    .and_then(|(n, _)| l.index(n))
                    .unwrap_or(0);
                let Some(tiene) = l.index(&nivel) else {
                    continue;
                };
                if tiene <= permitido {
                    continue;
                }
                let (code, como) = match origen {
                    Origin::Computed => (Code::Oos4001, "computada por join"),
                    Origin::Declared => (Code::Oos4002, "declarada"),
                    Origin::Inherited => (Code::Oos4002, "heredada"),
                };
                let permitido_txt = autorizacion
                    .get(&ret)
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| l.levels[0].clone());
                out.push(
                    Diagnostic::new(
                        code,
                        &v.path,
                        format!(
                            "`{vqn}.{campo}` lleva `{ret}:{nivel}` ({como}) y `{conducto}`                              solo admite `{ret}:{permitido_txt}`"
                        ),
                    )
                    .at(mat.pos())
                    .help(
                        "una vista materializada es una copia, y la copia lleva lo que llevan                          sus campos aunque quien los clasificó sea una entidad tres vistas                          más arriba. Quita el campo de la vista, eleva la autorización del                          conducto donde se decide eso, o no materialices",
                    ),
                );
            }
        }
    }
}

// ── OOS4001 · OOS4002 · OOS4011 · el índice de topología ────────────────────

/// Una relación con `via` **se atraviesa**, y atravesar es una búsqueda por
/// clave sobre una copia de dos columnas: la clave de la entidad y el enlace.
/// Esa copia instancia `materialization.topology`, igual que la declaraba el
/// eje del binding — porque es la misma cosa con otro dueño.
///
/// # Por qué esto no es un `OOS2026` que prohíbe
///
/// Se midió la regla contraria —*lo que se atraviesa se debe materializar*,
/// entendida como «declara `materialized`»— y no se puede pagar: `materialized`
/// es el conducto de **la carga**, y una vista cuya carga no puede salir del
/// origen tiene aristas que sí pueden. Son dos decisiones, y la política ya las
/// separa con dos autorizaciones. `pruebas-de-fuego/medida-b0-impagable.py`.
///
/// Lo que faltaba no era prohibir la travesía: era mirar la copia que ya se
/// hace. `registro::topologia` la construye —plan, destino y refresco propios—
/// y hasta aquí no pasaba por ningún conducto.
///
/// # Por qué solo las derivadas
///
/// Un binding **declara** su `materialization.topology`, y [`materializaciones`]
/// ya sella esa declaración. Una vista no declara nada —lo derivable no se
/// declara (P2)— así que la única forma de sellar su copia es derivarla. Sellar
/// las dos aquí sería contar dos veces el mismo camino viejo, y **cambiaría un
/// resultado de v1alpha1**: un binding sin eje declarado no copia aristas hoy, y
/// pasaría a fallar. Por eso `Arista::derivada`, y por eso no hace falta acotar
/// por versión — el sujeto nuevo solo existe donde hay `backedBy`.
///
/// # De dónde salen las etiquetas
///
/// De la entidad, y directamente: lo que viaja son **dos propiedades suyas**,
/// no campos de una vista tres eslabones más abajo. Así que no hay que
/// proyectar nada — `efectivas` ya las tiene, con su herencia resuelta.
fn indices_de_topologia(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    efectivas: &BTreeMap<String, EntityLabels>,
    conductos: &BTreeMap<String, Labels>,
    out: &mut Vec<Diagnostic>,
) {
    let conducto = "materialization.topology";
    // Una misma arista sale una vez por fuente física, y la copia es una. Se
    // sella por NOMBRE, que es como el registro la identifica.
    let mut vistas: BTreeSet<String> = BTreeSet::new();
    for a in crate::aristas::aristas(pkg).into_iter().filter(|a| a.derivada) {
        if !vistas.insert(a.nombre.clone()) {
            continue;
        }
        let Some(e) = pkg.entity(&a.entidad) else {
            continue;
        };
        // Se ancla en la relación, que es lo que hay que quitar o repensar.
        let pos = e
            .section("relations")
            .and_then(|r| r.get(&a.relacion))
            .map(|(_, n)| n.pos())
            .or_else(|| e.section("relations").map(|r| r.pos()));

        // OOS4011 · omitir un conducto no es dejarlo abierto: es cerrarlo.
        let Some(autorizacion) = conductos.get(conducto) else {
            let mut d = Diagnostic::new(
                Code::Oos4011,
                &e.path,
                format!(
                    "`{}` se atraviesa por `{}`, y eso copia dos columnas por `{conducto}`, que no tiene autorización declarada",
                    a.entidad, a.relacion
                ),
            )
            .help(
                "recorrer una relación es una búsqueda por clave sobre una copia de la clave y el enlace: se materializa, aunque no lo declare nadie. Un conducto sin autorización es ⊥ y no admite nada, así que declara `materialization.topology` en la política de conductos, o quita la `via` de la relación",
            );
            if let Some(p) = pos {
                d = d.at(p);
            }
            out.push(d);
            continue;
        };

        // Y lo que viaja: exactamente dos propiedades, ni una más. Que sean dos
        // y no doce es toda la tesis — por eso una entidad con campos críticos
        // puede atravesarse sin que salga nada crítico.
        let Some(props) = efectivas.get(&a.entidad) else {
            continue;
        };
        for prop in [&a.clave, &a.via] {
            let Some(labels) = props.get(prop) else {
                continue;
            };
            for (ret, (nivel, origen)) in labels {
                let Some(l) = lat.get(ret) else { continue };
                let permitido = autorizacion
                    .get(ret)
                    .and_then(|(n, _)| l.index(n))
                    .unwrap_or(0);
                let Some(tiene) = l.index(nivel) else { continue };
                if tiene <= permitido {
                    continue;
                }
                let (code, como) = match origen {
                    Origin::Computed => (Code::Oos4001, "computada por join"),
                    Origin::Declared => (Code::Oos4002, "declarada"),
                    Origin::Inherited => (Code::Oos4002, "heredada"),
                };
                let permitido_txt = autorizacion
                    .get(ret)
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| l.levels[0].clone());
                let mut d = Diagnostic::new(
                    code,
                    &e.path,
                    format!(
                        "atravesar `{}` copia `{}.{prop}`, que lleva `{ret}:{nivel}` ({como}), y `{conducto}` solo admite `{ret}:{permitido_txt}`",
                        a.relacion, a.entidad
                    ),
                )
                .help(
                    "el índice de topología copia la clave y el enlace, y nada más: por eso una entidad con campos críticos se puede atravesar. Pero estas dos columnas también llevan lo suyo. Eleva la autorización del conducto donde se decide eso, quita la `via`, o relaja el suelo del datasource si la etiqueta es heredada",
                );
                if let Some(p) = pos {
                    d = d.at(p);
                }
                out.push(d);
            }
        }
    }
}

// ── Conductos ───────────────────────────────────────────────────────────────

/// Autorización efectiva de cada conducto. Varias políticas se combinan
/// tomando la **más restrictiva**: una local nunca afloja lo que una importada
/// restringe.
///
/// Se expone por la misma razón que [`efectivas`]: el emisor de GraphQL
/// necesita **este** techo y no debe recalcularlo. Si lo recalculara, el
/// contrato emitido y el chequeo de flujo podrían discrepar — y entonces
/// `ore validate` diría que un dato no puede salir por `contextSurface`
/// mientras el esquema lo declara.
pub fn clearances(pkg: &Package, lat: &BTreeMap<String, Lattice>) -> BTreeMap<String, Labels> {
    let mut out: BTreeMap<String, Labels> = BTreeMap::new();
    for cp in pkg.docs.iter().filter(|d| d.kind == Kind::ConduitPolicy) {
        let Some(cs) = cp.section("conduits") else {
            continue;
        };
        for (ck, cv) in cs.entries() {
            let Some(nombre) = ck.as_str() else { continue };
            let entrada = out.entry(nombre.to_string()).or_default();
            for (rk, rv) in cv.entries() {
                let (Some(ret), Some(nivel)) = (rk.as_str(), rv.as_str()) else {
                    continue;
                };
                let bajar = match (entrada.get(ret), lat.get(ret)) {
                    (Some((actual, _)), Some(l)) => l.index(nivel) < l.index(actual),
                    (None, _) => true,
                    _ => false,
                };
                if bajar {
                    entrada.insert(ret.to_string(), (nivel.to_string(), Origin::Declared));
                }
            }
        }
    }
    out
}


// ── OOS4006 · OOS4007 ───────────────────────────────────────────────────────

/// El vocabulario **cerrado** de desclasificadores. Cerrarlo es lo que lo hace
/// analizable: con un conjunto abierto, el compilador tendría que elegir entre
/// suponer que una obligación desconocida desclasifica —inseguro— o suponer que
/// no —lo que haría inútil la extensibilidad que supuestamente ganaba.
const DESCLASIFICADORES: &[&str] = &["mask", "tokenize", "redact", "aggregate", "promote"];

fn desclasificadores(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for (path, texto) in &pkg.cedar {
        for (i, linea) in texto.lines().enumerate() {
            let Some(inicio) = linea.find("@obligation(") else {
                continue;
            };
            let resto = &linea[inicio + 12..];
            let Some(fin) = resto.find(')') else { continue };
            let arg = resto[..fin].trim().trim_matches('"');
            let (nombre, param) = match arg.split_once(':') {
                Some((n, p)) => (n, Some(p)),
                None => (arg, None),
            };
            let pos = crate::diag::Pos {
                line: i + 1,
                col: inicio + 1,
            };

            if !DESCLASIFICADORES.contains(&nombre) {
                out.push(
                    Diagnostic::new(
                        Code::Oos4006,
                        path,
                        format!("`{nombre}` no es un desclasificador de OOS"),
                    )
                    .at(pos)
                    .help(format!(
                        "el vocabulario es cerrado: {}. Cerrarlo es lo que permite demostrar \
                         que ningún dato etiquetado llega sin transformar a un conducto, y que \
                         un regulador pueda leer la lista completa de transformaciones posibles",
                        DESCLASIFICADORES.join(" · ")
                    )),
                );
                continue;
            }

            if nombre == "aggregate" {
                let umbral = param
                    .and_then(|p| p.split_once('='))
                    .filter(|(k, _)| k.trim() == "minGroupSize")
                    .and_then(|(_, v)| v.trim().parse::<u32>().ok());
                if umbral.is_none() {
                    out.push(
                        Diagnostic::new(Code::Oos4007, path, "`aggregate` sin `minGroupSize`")
                            .at(pos)
                            .help(
                                "sin umbral no desclasifica nada: el agregado de un grupo de una \
                             persona es esa persona. Aceptarlo sería peor que rechazarlo — el \
                             paquete compilaría y todo el mundo creería que hay una garantía \
                             de k-anonimato donde solo hay una palabra",
                            ),
                    );
                    continue;
                }
            }
        }
    }
}

// ── OOS4014 ─────────────────────────────────────────────────────────────────

fn ejemplos(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    efectivas: &BTreeMap<String, EntityLabels>,
    out: &mut Vec<Diagnostic>,
) {
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let Some(ps) = e.section("properties") else {
            continue;
        };
        let Some(labels) = efectivas.get(&qn) else {
            continue;
        };

        for (k, v) in ps.entries() {
            let Some(nombre) = k.as_str() else { continue };
            let Some((_, ex)) = v.get("examples") else {
                continue;
            };
            let sintetico = ex.get("synthetic").and_then(|(_, s)| s.as_str()) == Some("true");
            if sintetico {
                continue;
            }
            // Solo importa si la propiedad está etiquetada por encima de ⊥.
            let etiquetada: BTreeSet<&String> = labels
                .get(nombre)
                .map(|ls| {
                    ls.iter()
                        .filter(|(r, (n, _))| lat.get(*r).and_then(|l| l.index(n)).unwrap_or(0) > 0)
                        .map(|(r, _)| r)
                        .collect()
                })
                .unwrap_or_default();
            if etiquetada.is_empty() {
                continue;
            }
            out.push(
                Diagnostic::new(
                    Code::Oos4014,
                    &e.path,
                    format!("`{qn}.{nombre}` está etiquetada y declara `examples` reales"),
                )
                .at(ex.pos())
                .help(
                    "los valores de ejemplo de una columna de salarios son salarios, y este \
                     fichero se revisa en un pull request, se publica y alcanza la superficie \
                     de contexto de cualquier agente. Declara `synthetic: true` si no proceden \
                     de datos reales — obligar a decirlo convierte un descuido silencioso en \
                     una afirmación consciente",
                ),
            );
        }
    }
}
