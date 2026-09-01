//! **El retículo fluye por el linaje, y la vista se niega a compilar.**
//!
//! Es el peldaño por el que existe todo lo anterior. M0, M1 y M2 los tienen
//! otros —Calcite planifica, Substrait serializa, OpenLineage registra el
//! linaje—. Esto no lo tiene nadie:
//!
//! > **El linaje se comprueba al compilar, no se observa al ejecutar.**
//!
//! # Y la mitad indirecta cuenta igual
//!
//! Una columna que solo aparece en un `WHERE` no sale en el resultado y decide
//! qué filas salen. En control de flujo de información eso es un **flujo
//! implícito**, y el tratamiento clásico —Denning— es que la etiqueta de la
//! condición se une a todo lo que se computa bajo ella. Aquí es literal: una
//! arista `INDIRECTO` clasifica igual que una `DIRECTO`.
//!
//! Se podría argumentar que un flujo implícito filtra *menos* —si una fila pasó
//! el filtro, no el valor que lo pasó—, y hay literatura que lo cuantifica.
//! **Nosotros no la tenemos**, y aflojar sin el argumento sería aflojar por
//! comodidad justo en la dirección insegura. Lo que hace vivible la regla
//! estricta no es relajarla: es **desclasificar explícitamente**, que es lo que
//! una máscara de `Ruleset` ya hace en OOS y lo que entrará cuando el conducto
//! esté conectado.
//!
//! # El eje decide cómo se combina, y esto no lo habría acertado solo
//!
//! Está en `ore_core::flow::Axis` desde antes que esta pieza, y es lo correcto:
//!
//! | Eje | Pregunta | Combina | Viola si |
//! |---|---|---|---|
//! | **confidencialidad** | *¿cuánto daño si esto se filtra?* | `max` | la salida sale **por encima** de lo autorizado |
//! | **integridad** | *¿cuánto daño si esto es falso?* | `min` | la salida queda **por debajo** de lo exigido |
//!
//! Con `max` en los dos, una vista que junta un dato fiable con uno dudoso
//! parecería fiable. Por eso el retículo se reutiliza entero en vez de
//! redefinirlo aquí: la regla ya estaba escrita y una segunda copia habría
//! divergido en este punto exacto.
//!
//! # Sin etiqueta no hay constraint, y es la convención que ya existe
//!
//! Una raíz sin etiqueta en un eje **no participa** en la combinación de ese
//! eje. No es «el fondo» ni «lo más alto»: es que no está. Es la convención del
//! compilador —una propiedad sin clasificar no está gobernada— y usar otra aquí
//! haría que la misma columna se clasificara distinto según quién preguntase.

use crate::lineage::{Clase, Linaje, Raiz};
use ore_core::flow::{Axis, Lattice};
use std::collections::BTreeMap;

/// Eje → nivel.
pub type Etiquetas = BTreeMap<String, String>;

/// De dónde salen las etiquetas y con qué orden se comparan.
///
/// Se pasa como dato y no se lee de ningún sitio: esta pieza no sabe qué es un
/// paquete OOS. Rellenarlo desde uno es la absorción, y es otro acto.
#[derive(Debug, Clone, Default)]
pub struct Clasificacion {
    /// Eje → su retículo. **El de `ore_core::flow`**, no uno propio: el orden es
    /// normativo y una segunda copia divergiría justo en el eje de integridad.
    pub reticulos: BTreeMap<String, Lattice>,
    /// Qué lleva puesto cada columna de origen.
    pub de_raiz: BTreeMap<Raiz, Etiquetas>,
}

/// Por qué una vista no compila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fuga {
    /// La salida queda fuera de lo autorizado. `culpables` son **todas** las
    /// raíces que alcanzan ese nivel: arreglar una y dejar las otras no arregla
    /// nada, así que se dicen todas.
    FueraDeLoAutorizado {
        salida: String,
        eje: String,
        nivel: String,
        autorizado: String,
        culpables: Vec<(Raiz, Clase)>,
    },
    /// Una etiqueta nombra un nivel que su retículo no tiene. **No es lo mismo
    /// que estar por debajo**: confundirlos convertiría una etiqueta mal escrita
    /// en una columna que parece limpia.
    NivelDesconocido {
        eje: String,
        nivel: String,
        niveles: Vec<String>,
    },
    /// Una etiqueta nombra un eje sin retículo. Sin orden no hay comparación, y
    /// dejarla pasar sería no comprobar nada en silencio.
    EjeDesconocido { eje: String },
}

impl Fuga {
    pub fn como_texto(&self) -> String {
        match self {
            Fuga::FueraDeLoAutorizado {
                salida,
                eje,
                nivel,
                autorizado,
                culpables,
            } => {
                let mut s = format!("`{salida}` no compila\n");
                for (r, c) in culpables {
                    s.push_str(&format!(
                        "  ← {}·{}.{}  {}\n",
                        r.datasource,
                        r.objeto,
                        r.campo,
                        match c {
                            Clase::Indirecto(i) => format!("por INFLUENCIA ({i:?})"),
                            Clase::Directo(d) => format!("por derivación ({d:?})"),
                        }
                    ));
                }
                s.push_str(&format!("  {eje} del origen    : {nivel}\n"));
                s.push_str(&format!("  {eje} de esta vista : {autorizado}"));
                s
            }
            Fuga::NivelDesconocido {
                eje,
                nivel,
                niveles,
            } => format!(
                "`{nivel}` no es un nivel de `{eje}`; sus niveles son {}",
                niveles.join(" ⊑ ")
            ),
            Fuga::EjeDesconocido { eje } => format!(
                "`{eje}` no tiene retículo: sin orden no hay comparación, y dejarlo pasar \
                 sería no comprobar nada en silencio"
            ),
        }
    }
}

/// Lo que sale de comprobar: **la clasificación efectiva de cada columna** y las
/// fugas.
///
/// Las dos cosas, y no solo la segunda: la clasificación calculada es lo que una
/// vista de más arriba hereda, y devolverla evita que quien la necesite la
/// recalcule con otra regla.
#[derive(Debug, Clone, Default)]
pub struct Veredicto {
    pub efectivas: BTreeMap<String, Etiquetas>,
    pub fugas: Vec<Fuga>,
}

impl Veredicto {
    pub fn compila(&self) -> bool {
        self.fugas.is_empty()
    }
}

/// A qué nivel ha quedado un eje, y **quién lo puso ahí**.
///
/// Los culpables van al lado del nivel y no se recalculan después: quien
/// determina el nivel es el mismo recorrido que lo combina, y volver a buscarlo
/// sería una segunda regla que podría discrepar de la primera.
type Acumulado = (usize, Vec<(Raiz, Clase)>);

/// Cómo combina un eje. **Confidencialidad une por arriba; integridad, por
/// abajo.** Con `max` en los dos, juntar un dato fiable con uno dudoso daría un
/// resultado que parece fiable.
fn combinar(eje: Axis, a: usize, b: usize) -> usize {
    match eje {
        Axis::Confidentiality => a.max(b),
        Axis::Integrity => a.min(b),
    }
}

/// Y cómo se viola. Es el mismo desdoble, en el otro sentido.
fn viola(eje: Axis, nivel: usize, autorizado: usize) -> bool {
    match eje {
        Axis::Confidentiality => nivel > autorizado,
        Axis::Integrity => nivel < autorizado,
    }
}

/// **La comprobación.**
///
/// `autoriza` es lo que la vista se compromete a exponer, por eje. Un eje que no
/// nombre no se comprueba — declarar la autorización es lo que la hace
/// comprobable, y suponerla sería inventar un compromiso que nadie asumió.
///
/// Devuelve **todas** las fugas, no la primera: un compilador que informa de un
/// error cada vez es un compilador que se ejecuta diez veces.
pub fn comprobar(l: &Linaje, c: &Clasificacion, autoriza: &Etiquetas) -> Veredicto {
    let mut v = Veredicto::default();

    for (salida, aristas) in l {
        // Eje → (índice combinado, raíces que están en ese nivel).
        let mut por_eje: BTreeMap<&str, Acumulado> = BTreeMap::new();

        for a in aristas {
            let Some(etiquetas) = c.de_raiz.get(&a.raiz) else {
                continue;
            };
            for (eje, nivel) in etiquetas {
                let Some(ret) = c.reticulos.get(eje) else {
                    empujar(&mut v.fugas, Fuga::EjeDesconocido { eje: eje.clone() });
                    continue;
                };
                let Some(i) = ret.index(nivel) else {
                    empujar(
                        &mut v.fugas,
                        Fuga::NivelDesconocido {
                            eje: eje.clone(),
                            nivel: nivel.clone(),
                            niveles: ret.levels.clone(),
                        },
                    );
                    continue;
                };
                // **La arista indirecta entra igual que la directa.** Es la
                // línea de la que depende que M2 no sea decorativo.
                match por_eje.get_mut(eje.as_str()) {
                    None => {
                        por_eje.insert(eje.as_str(), (i, vec![(a.raiz.clone(), a.clase)]));
                    }
                    Some((actual, culpables)) => {
                        let nuevo = combinar(ret.axis, *actual, i);
                        if nuevo != *actual {
                            *actual = nuevo;
                            culpables.clear();
                        }
                        if i == *actual {
                            culpables.push((a.raiz.clone(), a.clase));
                        }
                    }
                }
            }
        }

        let mut efectivas = Etiquetas::new();
        for (eje, (i, culpables)) in por_eje {
            let ret = &c.reticulos[eje];
            let nivel = ret.levels[i].clone();
            efectivas.insert(eje.to_string(), nivel.clone());

            let Some(techo) = autoriza.get(eje) else {
                continue;
            };
            let Some(t) = ret.index(techo) else {
                empujar(
                    &mut v.fugas,
                    Fuga::NivelDesconocido {
                        eje: eje.to_string(),
                        nivel: techo.clone(),
                        niveles: ret.levels.clone(),
                    },
                );
                continue;
            };
            if viola(ret.axis, i, t) {
                v.fugas.push(Fuga::FueraDeLoAutorizado {
                    salida: salida.clone(),
                    eje: eje.to_string(),
                    nivel,
                    autorizado: techo.clone(),
                    culpables,
                });
            }
        }
        v.efectivas.insert(salida.clone(), efectivas);
    }

    v
}

/// Un eje sin retículo o un nivel mal escrito se dicen **una vez**, no una por
/// cada columna que los toque: repetir el mismo defecto veinte veces entierra
/// los otros diecinueve.
fn empujar(fugas: &mut Vec<Fuga>, f: Fuga) {
    if !fugas.contains(&f) {
        fugas.push(f);
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lineage::{Directa, Indirecta, linaje};
    use crate::plan::{Comparador, Expr, Junta, Lectura, Nodo, Valor};
    use ore_core::types::parse_type;
    use std::collections::BTreeMap;

    fn reticulo(qname: &str, niveles: &[&str], axis: Axis) -> Lattice {
        Lattice {
            qname: qname.into(),
            levels: niveles.iter().map(|s| (*s).to_string()).collect(),
            axis,
            requires_governance: BTreeMap::new(),
        }
    }

    fn gdpr() -> BTreeMap<String, Lattice> {
        [(
            "gdpr.sensitivity".to_string(),
            reticulo(
                "gdpr.sensitivity",
                &["low", "medium", "high", "critical"],
                Axis::Confidentiality,
            ),
        )]
        .into()
    }

    fn raiz(campo: &str) -> Raiz {
        Raiz {
            datasource: "lago".into(),
            objeto: "ventas.pedidos".into(),
            campo: campo.into(),
        }
    }

    fn etiqueta(eje: &str, nivel: &str) -> Etiquetas {
        [(eje.to_string(), nivel.to_string())].into()
    }

    /// `nif` es crítico, `total` no lleva nada.
    fn clasificacion() -> Clasificacion {
        Clasificacion {
            reticulos: gdpr(),
            de_raiz: [(raiz("nif"), etiqueta("gdpr.sensitivity", "critical"))].into(),
        }
    }

    fn pedidos() -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: "ventas.pedidos".into(),
            campos: [
                ("id".to_string(), parse_type("Integer").unwrap()),
                ("total".to_string(), parse_type("Decimal").unwrap()),
                ("nif".to_string(), parse_type("String").unwrap()),
            ]
            .into(),
        })
    }

    fn eq(campo: &str, v: &str) -> Expr {
        Expr::Compara {
            op: Comparador::Igual,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(Valor::Cadena(v.into()))),
        }
    }

    fn ver(p: &Nodo, autoriza: &Etiquetas) -> Veredicto {
        comprobar(&linaje(p).expect("cuadra"), &clasificacion(), autoriza)
    }

    /// Exponer una columna crítica por encima de lo autorizado no compila. Es la
    /// mitad fácil, y sin ella la otra no significaría nada.
    #[test]
    fn exponer_una_columna_por_encima_de_lo_autorizado_no_compila() {
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("documento".to_string(), Expr::campo("nif"))].into(),
        };
        let v = ver(&p, &etiqueta("gdpr.sensitivity", "medium"));
        assert!(!v.compila(), "{:?}", v.fugas);
        assert_eq!(v.efectivas["documento"]["gdpr.sensitivity"], "critical");

        // Y con autorización suficiente, compila.
        assert!(ver(&p, &etiqueta("gdpr.sensitivity", "critical")).compila());
    }

    /// **EL CRITERIO DE M3, y lo que decide si M2 era decorativo.**
    ///
    /// `nif` **no sale** en el resultado: solo se filtra por él. Y filtrar por
    /// una columna crítica filtra información crítica hacia un resultado que
    /// nadie clasificó. Si esto compilase, la mitad indirecta del linaje sería
    /// un adorno.
    #[test]
    fn filtrar_por_una_columna_critica_tampoco_compila() {
        let p = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: eq("nif", "12345678Z"),
            }),
            campos: [("cuanto".to_string(), Expr::campo("total"))].into(),
        };

        // La salida no contiene `nif` por ningún lado.
        assert!(!crate::esquema(&p).unwrap().contains_key("nif"));

        let v = ver(&p, &etiqueta("gdpr.sensitivity", "medium"));
        assert!(!v.compila(), "el flujo implícito pasó sin más");

        let [
            Fuga::FueraDeLoAutorizado {
                salida,
                nivel,
                autorizado,
                culpables,
                ..
            },
        ] = &v.fugas[..]
        else {
            panic!("{:?}", v.fugas);
        };
        assert_eq!(salida, "cuanto");
        assert_eq!(nivel, "critical");
        assert_eq!(autorizado, "medium");
        // Y el mensaje dice que llega por influencia, no por derivación: sin
        // eso, quien lo lea buscará una columna que no está en la proyección.
        assert_eq!(
            culpables,
            &[(raiz("nif"), Clase::Indirecto(Indirecta::Filtro))]
        );
        assert!(
            v.fugas[0].como_texto().contains("por INFLUENCIA"),
            "{}",
            v.fugas[0].como_texto()
        );
    }

    /// Y la clave de una junta hace lo mismo: emparejar por una columna crítica
    /// decide qué filas salen.
    #[test]
    fn juntar_por_una_columna_critica_tampoco() {
        let otra = Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: "ventas.contactos".into(),
            campos: [
                ("doc".to_string(), parse_type("String").unwrap()),
                ("email".to_string(), parse_type("String").unwrap()),
            ]
            .into(),
        });
        let p = Nodo::Proyecta {
            entrada: Box::new(Nodo::Une {
                izquierda: Box::new(pedidos()),
                derecha: Box::new(otra),
                tipo: Junta::Interna,
                sobre: vec![("nif".into(), "doc".into())],
            }),
            campos: [("correo".to_string(), Expr::campo("email"))].into(),
        };
        let v = ver(&p, &etiqueta("gdpr.sensitivity", "medium"));
        assert!(!v.compila(), "{:?}", v.efectivas);
        assert!(matches!(v.fugas[0], Fuga::FueraDeLoAutorizado { .. }));
    }

    /// **El eje decide cómo se combina.** En integridad se une por ABAJO, así
    /// que juntar un dato fiable con uno dudoso da un resultado dudoso — y con
    /// `max` en los dos ejes esto pasaría por fiable.
    #[test]
    fn en_integridad_se_une_por_abajo() {
        let c = Clasificacion {
            reticulos: [(
                "oos.integrity".to_string(),
                reticulo(
                    "oos.integrity",
                    &["untrusted", "reviewed", "certified"],
                    Axis::Integrity,
                ),
            )]
            .into(),
            de_raiz: [
                (raiz("total"), etiqueta("oos.integrity", "certified")),
                (raiz("id"), etiqueta("oos.integrity", "untrusted")),
            ]
            .into(),
        };
        // Una columna que deriva de las dos.
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [(
                "mezcla".to_string(),
                // Lee las dos columnas sin compararlas: el tipador de M0
                // rechaza `Decimal > Integer`, y tiene razon.
                Expr::Y(vec![
                    Expr::EsNulo(Box::new(Expr::campo("total"))),
                    Expr::EsNulo(Box::new(Expr::campo("id"))),
                ]),
            )]
            .into(),
        };
        let v = comprobar(
            &linaje(&p).expect("cuadra"),
            &c,
            &etiqueta("oos.integrity", "reviewed"),
        );
        assert_eq!(
            v.efectivas["mezcla"]["oos.integrity"], "untrusted",
            "en integridad se une por abajo"
        );
        assert!(!v.compila(), "queda por DEBAJO de lo exigido");
    }

    /// Y ese mismo caso, con las mismas etiquetas pero en un eje de
    /// confidencialidad, se une por arriba. Es la prueba de que el
    /// desdoblamiento hace algo.
    #[test]
    fn el_mismo_par_de_etiquetas_se_combina_al_reves_segun_el_eje() {
        let de_raiz: BTreeMap<Raiz, Etiquetas> = [
            (raiz("total"), etiqueta("x", "alto")),
            (raiz("id"), etiqueta("x", "bajo")),
        ]
        .into();
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [(
                "mezcla".to_string(),
                // Lee las dos columnas sin compararlas: el tipador de M0
                // rechaza `Decimal > Integer`, y tiene razon.
                Expr::Y(vec![
                    Expr::EsNulo(Box::new(Expr::campo("total"))),
                    Expr::EsNulo(Box::new(Expr::campo("id"))),
                ]),
            )]
            .into(),
        };
        let l = linaje(&p).expect("cuadra");

        let con_eje = |axis| {
            comprobar(
                &l,
                &Clasificacion {
                    reticulos: [("x".to_string(), reticulo("x", &["bajo", "alto"], axis))].into(),
                    de_raiz: de_raiz.clone(),
                },
                &Etiquetas::new(),
            )
            .efectivas["mezcla"]["x"]
                .clone()
        };
        assert_eq!(con_eje(Axis::Confidentiality), "alto");
        assert_eq!(con_eje(Axis::Integrity), "bajo");
    }

    /// Una raíz sin etiqueta en un eje **no participa**: no es el fondo ni lo
    /// más alto, es que no está. Sin esta regla, `total` arrastraría el eje a
    /// `low` y taparía el `critical` de `nif`.
    #[test]
    fn una_raiz_sin_etiqueta_no_participa() {
        let p = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: eq("nif", "X"),
            }),
            campos: [("cuanto".to_string(), Expr::campo("total"))].into(),
        };
        let v = ver(&p, &Etiquetas::new());
        // `total` no lleva nada; `nif` sí. Gana el que hay.
        assert_eq!(v.efectivas["cuanto"]["gdpr.sensitivity"], "critical");

        // Y una columna cuyas raíces no llevan nada no tiene etiqueta: no está
        // gobernada, que es la convención del compilador.
        let limpia = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("n".to_string(), Expr::campo("id"))].into(),
        };
        assert!(ver(&limpia, &Etiquetas::new()).efectivas["n"].is_empty());
    }

    /// Un eje que la vista no autoriza **no se comprueba**: declarar la
    /// autorización es lo que la hace comprobable, y suponerla sería inventar un
    /// compromiso que nadie asumió.
    #[test]
    fn un_eje_sin_autorizacion_declarada_no_se_comprueba() {
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("documento".to_string(), Expr::campo("nif"))].into(),
        };
        let v = ver(&p, &Etiquetas::new());
        assert!(v.compila());
        // Pero la clasificación se calcula igual, que es lo que hereda quien
        // esté por encima.
        assert_eq!(v.efectivas["documento"]["gdpr.sensitivity"], "critical");
    }

    /// Un nivel mal escrito **no es lo mismo que estar por debajo**:
    /// confundirlos convertiría una etiqueta con una errata en una columna que
    /// parece limpia.
    #[test]
    fn una_etiqueta_mal_escrita_no_pasa_por_limpia() {
        let c = Clasificacion {
            reticulos: gdpr(),
            de_raiz: [(raiz("nif"), etiqueta("gdpr.sensitivity", "criticaal"))].into(),
        };
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("documento".to_string(), Expr::campo("nif"))].into(),
        };
        let v = comprobar(
            &linaje(&p).expect("cuadra"),
            &c,
            &etiqueta("gdpr.sensitivity", "medium"),
        );
        assert!(!v.compila());
        assert!(
            matches!(v.fugas[0], Fuga::NivelDesconocido { .. }),
            "{:?}",
            v.fugas
        );
    }

    /// Y un eje sin retículo tampoco: sin orden no hay comparación, y dejarlo
    /// pasar sería no comprobar nada en silencio.
    #[test]
    fn un_eje_sin_reticulo_no_pasa_por_limpio() {
        let c = Clasificacion {
            reticulos: BTreeMap::new(),
            de_raiz: [(raiz("nif"), etiqueta("inventado.eje", "alto"))].into(),
        };
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("documento".to_string(), Expr::campo("nif"))].into(),
        };
        let v = comprobar(&linaje(&p).expect("cuadra"), &c, &Etiquetas::new());
        assert_eq!(
            v.fugas,
            vec![Fuga::EjeDesconocido {
                eje: "inventado.eje".into()
            }]
        );
    }

    /// El mismo defecto se dice **una vez**, no una por columna: repetirlo
    /// veinte veces entierra los otros diecinueve.
    #[test]
    fn un_defecto_del_reticulo_no_se_repite_por_columna() {
        let c = Clasificacion {
            reticulos: BTreeMap::new(),
            de_raiz: [(raiz("nif"), etiqueta("inventado.eje", "alto"))].into(),
        };
        // El filtro hace que `nif` influya en las tres columnas de salida.
        let p = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: eq("nif", "X"),
        };
        let v = comprobar(&linaje(&p).expect("cuadra"), &c, &Etiquetas::new());
        assert_eq!(v.fugas.len(), 1, "{:?}", v.fugas);
    }

    /// Y se dicen **todas** las fugas, no la primera: un compilador que informa
    /// de un error cada vez es un compilador que se ejecuta diez veces.
    #[test]
    fn se_dicen_todas_las_fugas() {
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [
                ("a".to_string(), Expr::campo("nif")),
                ("b".to_string(), Expr::campo("nif")),
            ]
            .into(),
        };
        let v = ver(&p, &etiqueta("gdpr.sensitivity", "low"));
        assert_eq!(v.fugas.len(), 2, "{:?}", v.fugas);
    }

    /// Una columna con **dos** raíces al nivel que rompe las nombra a las dos:
    /// arreglar una y dejar la otra no arregla nada.
    #[test]
    fn se_nombran_todos_los_culpables() {
        let c = Clasificacion {
            reticulos: gdpr(),
            de_raiz: [
                (raiz("nif"), etiqueta("gdpr.sensitivity", "critical")),
                (raiz("total"), etiqueta("gdpr.sensitivity", "critical")),
            ]
            .into(),
        };
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [(
                "mezcla".to_string(),
                Expr::Y(vec![
                    Expr::EsNulo(Box::new(Expr::campo("nif"))),
                    Expr::EsNulo(Box::new(Expr::campo("total"))),
                ]),
            )]
            .into(),
        };
        let v = comprobar(
            &linaje(&p).expect("cuadra"),
            &c,
            &etiqueta("gdpr.sensitivity", "low"),
        );
        let [Fuga::FueraDeLoAutorizado { culpables, .. }] = &v.fugas[..] else {
            panic!("{:?}", v.fugas);
        };
        assert_eq!(culpables.len(), 2, "{culpables:?}");
    }

    /// La clasificación se calcula aunque no haya nada que comprobar, y es lo
    /// que una vista de más arriba hereda. Y una identidad conserva el nivel.
    #[test]
    fn la_clasificacion_efectiva_se_devuelve_para_quien_este_encima() {
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("documento".to_string(), Expr::campo("nif"))].into(),
        };
        let v = ver(&p, &Etiquetas::new());
        assert_eq!(
            v.efectivas["documento"],
            etiqueta("gdpr.sensitivity", "critical")
        );
        // Y una derivación tampoco la baja: transformar no desclasifica.
        let derivada = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [(
                "hay".to_string(),
                Expr::EsNulo(Box::new(Expr::campo("nif"))),
            )]
            .into(),
        };
        let v = ver(&derivada, &Etiquetas::new());
        assert_eq!(v.efectivas["hay"]["gdpr.sensitivity"], "critical");
        assert_eq!(
            linaje(&derivada).unwrap()["hay"]
                .iter()
                .next()
                .unwrap()
                .clase,
            Clase::Directo(Directa::Transformacion)
        );
    }
}
