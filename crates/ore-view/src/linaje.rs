//! El **linaje a nivel de columna**, derivado del plan.
//!
//! Qué columna raíz produce qué columna de salida, y **por qué arista**. No se
//! observa de una ejecución: se calcula del IR, así que existe antes de que
//! nadie abra nada y es tan cierto como el plan.
//!
//! # `DIRECTO` e `INDIRECTO`, y por qué la segunda es la que importa
//!
//! El vocabulario es el de OpenLineage, y no se inventa nada: `DIRECT` cuando el
//! valor de salida **se deriva** del de entrada, `INDIRECT` cuando **está
//! influido** por él sin derivarse.
//!
//! Léase con ojos de flujo de información:
//!
//! > **`INDIRECTO` es un flujo implícito.** Una columna que solo aparece en un
//! > `WHERE` **no sale en el resultado y decide qué filas salen**.
//!
//! Si esa columna es `gdpr.sensitivity: critical`, filtrar por ella filtra
//! información crítica hacia un resultado que nadie clasificó. OpenLineage lo
//! **registra**; Foundry propaga sus *markings* por dataset y los aplica al
//! acceder; dbt no sabe qué es una etiqueta. **Nadie se niega a compilar.** Que
//! aquí se calcule es lo que hace posible M3, y sin la mitad indirecta M3 sería
//! decorativo.
//!
//! # Cómo se componen dos pasos
//!
//! Raíz `r` → intermedia `m` → salida `o`. La regla es una y es la que hace que
//! la influencia no se pierda por el camino:
//!
//! > **Derivar es transitivo solo a través de derivaciones. Si cualquiera de los
//! > dos pasos es influencia, el resultado es influencia.**
//!
//! El subtipo lo pone **el paso más reciente que la hizo indirecta**, porque a
//! quien pregunta *«¿por qué depende de esto?»* le sirve la razón más cercana. Y
//! entre dos pasos directos gana el más fuerte: identidad < transformación <
//! agregación.
//!
//! # Lo que aquí NO se decide
//!
//! El campo `masking` del facet de OpenLineage no se emite, y no por olvido:
//! saber si algo enmascara es un juicio de gobierno y aquí no hay ni retículo ni
//! conducto. Es de M3, donde las máscaras existen.
//!
//! Y el array `dataset` del facet —dependencias indirectas de todo el conjunto
//! en vez de por columna— tampoco: es una compactación, y tener el mismo hecho
//! en dos sitios es exactamente lo que este proyecto no hace. Se emite por
//! columna, que es completo y es la forma que el retículo necesita.

use crate::esquema::{Desajuste, esquema};
use crate::plan::{Agregado, Expr, Nodo};
use ore_core::json::Json;
use std::collections::{BTreeMap, BTreeSet};

/// Una columna de una hoja: de dónde sale un dato de verdad.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Raiz {
    pub datasource: String,
    pub objeto: String,
    pub campo: String,
}

/// El valor de salida **se deriva** del de entrada. Ordenados de menos a más
/// fuerte: entre dos pasos directos gana el mayor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Directa {
    Identidad,
    Transformacion,
    Agregacion,
}

/// El valor de salida **está influido** por el de entrada sin derivarse de él.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Indirecta {
    /// La clave de una junta decide qué filas se emparejan.
    Junta,
    /// La clave de un grupo decide qué filas se agregan juntas. **También es lo
    /// que produce un `Distingue`**: quitar duplicados es agrupar por todas las
    /// columnas, que es como lo reescribe cualquier planificador.
    Agrupacion,
    /// Un predicado decide qué filas salen.
    Filtro,
}

/// Que las dos familias no se puedan mezclar es a propósito: `DIRECT` con
/// subtipo `FILTER` no significa nada, y un `enum` plano lo dejaría escribir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Clase {
    Directo(Directa),
    Indirecto(Indirecta),
}

impl Clase {
    /// La composición de dos pasos. Está desarrollada en la cabecera.
    fn componer(dentro: Clase, paso: Clase) -> Clase {
        match (dentro, paso) {
            // El paso más reciente manda cuando es el que introduce influencia:
            // a quien pregunta le sirve la razón más cercana.
            (_, Clase::Indirecto(b)) => Clase::Indirecto(b),
            (Clase::Indirecto(a), _) => Clase::Indirecto(a),
            (Clase::Directo(a), Clase::Directo(b)) => Clase::Directo(a.max(b)),
        }
    }

    const fn tipo(self) -> &'static str {
        match self {
            Clase::Directo(_) => "DIRECT",
            Clase::Indirecto(_) => "INDIRECT",
        }
    }

    const fn subtipo(self) -> &'static str {
        match self {
            Clase::Directo(Directa::Identidad) => "IDENTITY",
            Clase::Directo(Directa::Transformacion) => "TRANSFORMATION",
            Clase::Directo(Directa::Agregacion) => "AGGREGATION",
            Clase::Indirecto(Indirecta::Junta) => "JOIN",
            Clase::Indirecto(Indirecta::Agrupacion) => "GROUP_BY",
            Clase::Indirecto(Indirecta::Filtro) => "FILTER",
        }
    }

    pub const fn es_indirecta(self) -> bool {
        matches!(self, Clase::Indirecto(_))
    }
}

/// Una arista del linaje: **de qué raíz, y por qué**.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Arista {
    pub raiz: Raiz,
    pub clase: Clase,
}

/// Columna de salida → de dónde sale.
pub type Linaje = BTreeMap<String, BTreeSet<Arista>>;

/// **El linaje de un plan.**
///
/// Exige que el plan cuadre antes de mirarlo: un linaje sobre un plan que no
/// tipa nombraría columnas que no existen, y sería exactamente el análisis que
/// parece bueno sobre medio plan que `Desajuste::SinExpandir` existe para
/// impedir.
pub fn linaje(n: &Nodo) -> Result<Linaje, Desajuste> {
    esquema(n)?;
    dentro(n)
}

/// Compone un conjunto de aristas con un paso más.
fn con(aristas: &BTreeSet<Arista>, paso: Clase) -> BTreeSet<Arista> {
    aristas
        .iter()
        .map(|a| Arista {
            raiz: a.raiz.clone(),
            clase: Clase::componer(a.clase, paso),
        })
        .collect()
}

/// Las aristas de los campos que una expresión lee, ya compuestas con el paso.
fn de_la_expresion(
    x: &Expr,
    dentro: &Linaje,
    paso: Clase,
    donde: &'static str,
) -> Result<BTreeSet<Arista>, Desajuste> {
    let mut out = BTreeSet::new();
    for c in x.campos_leidos() {
        let a = dentro
            .get(&c)
            .ok_or(Desajuste::CampoDesconocido { donde, campo: c })?;
        out.extend(con(a, paso));
    }
    Ok(out)
}

fn dentro(n: &Nodo) -> Result<Linaje, Desajuste> {
    Ok(match n {
        Nodo::Referencia(v) => return Err(Desajuste::SinExpandir { vista: v.clone() }),

        Nodo::Lee(l) => l
            .campos
            .keys()
            .map(|c| {
                (
                    c.clone(),
                    [Arista {
                        raiz: Raiz {
                            datasource: l.datasource.clone(),
                            objeto: l.objeto.clone(),
                            campo: c.clone(),
                        },
                        clase: Clase::Directo(Directa::Identidad),
                    }]
                    .into(),
                )
            })
            .collect(),

        Nodo::Proyecta { entrada, campos } => {
            let d = dentro(entrada)?;
            let mut out = Linaje::new();
            for (nombre, x) in campos {
                // Copiar una columna es identidad; hacerle algo es
                // transformación. La distinción no es cosmética: una identidad
                // conserva la clasificación tal cual, y una transformación
                // también — pero solo la primera permite reconocer que la
                // columna de salida ES la de entrada.
                let paso = match x {
                    Expr::Campo(_) => Clase::Directo(Directa::Identidad),
                    _ => Clase::Directo(Directa::Transformacion),
                };
                out.insert(nombre.clone(), de_la_expresion(x, &d, paso, "proyecta")?);
            }
            out
        }

        // **La mitad que nadie calcula.** Las columnas del predicado no salen y
        // deciden qué filas salen, así que influyen en TODAS las de salida.
        Nodo::Filtra { entrada, predicado } => {
            let d = dentro(entrada)?;
            let influye =
                de_la_expresion(predicado, &d, Clase::Indirecto(Indirecta::Filtro), "filtra")?;
            d.into_iter()
                .map(|(c, a)| {
                    let mut a = a;
                    a.extend(influye.iter().cloned());
                    (c, a)
                })
                .collect()
        }

        // Las claves de la junta deciden qué filas se emparejan, así que
        // influyen en todo lo que sale — de los dos lados.
        Nodo::Une {
            izquierda,
            derecha,
            sobre,
            ..
        } => {
            let (i, d) = (dentro(izquierda)?, dentro(derecha)?);
            let mut influye = BTreeSet::new();
            for (a, b) in sobre {
                for (lado, campo, donde) in [(&i, a, "une · izquierda"), (&d, b, "une · derecha")]
                {
                    let aristas = lado.get(campo).ok_or(Desajuste::CampoDesconocido {
                        donde,
                        campo: campo.clone(),
                    })?;
                    influye.extend(con(aristas, Clase::Indirecto(Indirecta::Junta)));
                }
            }
            i.into_iter()
                .chain(d)
                .map(|(c, a)| {
                    let mut a = a;
                    a.extend(influye.iter().cloned());
                    (c, a)
                })
                .collect()
        }

        Nodo::Agrupa {
            entrada,
            por,
            agregados,
        } => {
            let d = dentro(entrada)?;
            // Las claves de grupo deciden qué filas se agregan juntas: influyen
            // en cada agregado.
            let mut influye = BTreeSet::new();
            for c in por {
                let a = d.get(c).ok_or(Desajuste::CampoDesconocido {
                    donde: "agrupa",
                    campo: c.clone(),
                })?;
                influye.extend(con(a, Clase::Indirecto(Indirecta::Agrupacion)));
            }

            let mut out = Linaje::new();
            for c in por {
                out.insert(c.clone(), d[c].clone());
            }
            for (nombre, a) in agregados {
                let mut aristas = match (&a.sobre, a.funcion) {
                    (Some(c), _) => {
                        let e = d.get(c).ok_or(Desajuste::CampoDesconocido {
                            donde: "agrupa",
                            campo: c.clone(),
                        })?;
                        con(e, Clase::Directo(Directa::Agregacion))
                    }
                    // `cuenta` sin columna no deriva de ninguna: lo único que
                    // decide su valor es qué filas caen en el grupo, y eso ya lo
                    // dicen las aristas indirectas de abajo.
                    (None, Agregado::Cuenta) => BTreeSet::new(),
                    (None, _) => {
                        return Err(Desajuste::AgregadoSinCampo {
                            nombre: nombre.clone(),
                        });
                    }
                };
                aristas.extend(influye.iter().cloned());
                out.insert(nombre.clone(), aristas);
            }
            out
        }

        Nodo::Unifica(ramas) => {
            let mut out = Linaje::new();
            for r in ramas {
                for (c, a) in dentro(r)? {
                    out.entry(c).or_default().extend(a);
                }
            }
            out
        }

        // **Quitar duplicados es agrupar por todas las columnas**, que es como
        // lo reescribe cualquier planificador. Así que cada columna influye en
        // todas — incluida ella misma, que es lo que de verdad pasa.
        Nodo::Distingue(e) => {
            let d = dentro(e)?;
            let influye: BTreeSet<Arista> = d
                .values()
                .flat_map(|a| con(a, Clase::Indirecto(Indirecta::Agrupacion)))
                .collect();
            d.into_iter()
                .map(|(c, a)| {
                    let mut a = a;
                    a.extend(influye.iter().cloned());
                    (c, a)
                })
                .collect()
        }

        // Sin orden, qué filas sobreviven a un límite no lo decide ninguna
        // columna. El día que haya un nodo de orden, sus columnas producirán
        // `SORT` — y hasta que lo haya, inventar la arista sería mentir.
        Nodo::Limita { entrada, .. } => dentro(entrada)?,
    })
}

/// El **facet `columnLineage` de OpenLineage**, tal cual lo define su esquema.
///
/// Se emite el vocabulario ajeno y no uno propio por lo mismo que emitimos a
/// Cedar y a ODCS: un linaje que solo entienda esta herramienta no vale para
/// nada en un catálogo que ya existe.
pub fn facet(l: &Linaje) -> Json {
    Json::obj([(
        "fields",
        Json::Obj(
            l.iter()
                .map(|(salida, aristas)| {
                    // Una raíz puede llegar por varias vías —directa y por un
                    // filtro—: OpenLineage lo modela como UN `inputField` con
                    // varias `transformations`, no como dos entradas.
                    let mut por_raiz: BTreeMap<&Raiz, Vec<Clase>> = BTreeMap::new();
                    for a in aristas {
                        por_raiz.entry(&a.raiz).or_default().push(a.clase);
                    }
                    (
                        salida.clone(),
                        Json::obj([(
                            "inputFields",
                            Json::Arr(
                                por_raiz
                                    .into_iter()
                                    .map(|(r, clases)| {
                                        Json::obj([
                                            ("namespace", Json::s(r.datasource.as_str())),
                                            ("name", Json::s(r.objeto.as_str())),
                                            ("field", Json::s(r.campo.as_str())),
                                            (
                                                "transformations",
                                                Json::Arr(
                                                    clases
                                                        .into_iter()
                                                        .map(|c| {
                                                            Json::obj([
                                                                ("type", Json::s(c.tipo())),
                                                                ("subtype", Json::s(c.subtipo())),
                                                            ])
                                                        })
                                                        .collect(),
                                                ),
                                            ),
                                        ])
                                    })
                                    .collect(),
                            ),
                        )]),
                    )
                })
                .collect(),
        ),
    )])
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Agregacion, Comparador, Junta, Lectura, Opaca, Valor};
    use ore_core::types::parse_type;

    fn hoja(objeto: &str, campos: &[(&str, &str)]) -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: objeto.into(),
            campos: campos
                .iter()
                .map(|(n, x)| ((*n).to_string(), parse_type(x).expect("tipo")))
                .collect(),
        })
    }

    fn pedidos() -> Nodo {
        hoja(
            "ventas.pedidos",
            &[("id", "Integer"), ("total", "Decimal"), ("nif", "String")],
        )
    }

    fn eq_str(campo: &str, v: &str) -> Expr {
        Expr::Compara {
            op: Comparador::Igual,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(Valor::Cadena(v.into()))),
        }
    }

    /// Las raíces de una columna de salida, con su clase.
    fn de(l: &Linaje, salida: &str) -> Vec<(String, Clase)> {
        l[salida]
            .iter()
            .map(|a| (a.raiz.campo.clone(), a.clase))
            .collect()
    }

    /// El linaje **cubre exactamente lo que el esquema dice que sale**. Si
    /// divergieran, una columna quedaría sin gobernar sin que nadie lo notase.
    #[test]
    fn el_linaje_tiene_las_mismas_columnas_que_el_esquema() {
        let planes = [
            pedidos(),
            Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: eq_str("nif", "X"),
            },
            Nodo::Distingue(Box::new(pedidos())),
            Nodo::Agrupa {
                entrada: Box::new(pedidos()),
                por: ["nif".to_string()].into(),
                agregados: [(
                    "n".to_string(),
                    Agregacion {
                        funcion: Agregado::Cuenta,
                        sobre: None,
                    },
                )]
                .into(),
            },
        ];
        for p in planes {
            let esq = crate::esquema(&p).expect("cuadra");
            let lin = linaje(&p).expect("cuadra");
            let (e, k): (Vec<&String>, Vec<&String>) = (esq.keys().collect(), lin.keys().collect());
            assert_eq!(e, k, "el linaje y el esquema discrepan en {p:?}");
        }
    }

    /// Copiar una columna es identidad; hacerle algo es transformación.
    #[test]
    fn una_proyeccion_distingue_copiar_de_transformar() {
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [
                ("tal_cual".to_string(), Expr::campo("total")),
                (
                    "tocada".to_string(),
                    Expr::Opaca(Opaca {
                        dialecto: "bigquery".into(),
                        texto: "redondea".into(),
                        lee: vec!["total".into()],
                        tipo: parse_type("Decimal").unwrap(),
                        determinista: true,
                    }),
                ),
            ]
            .into(),
        };
        let l = linaje(&p).expect("cuadra");
        assert_eq!(
            de(&l, "tal_cual"),
            [("total".to_string(), Clase::Directo(Directa::Identidad))]
        );
        // Y la opaca aporta su superficie declarada: un trozo que nadie entiende
        // sigue dejando fluir el linaje.
        assert_eq!(
            de(&l, "tocada"),
            [("total".to_string(), Clase::Directo(Directa::Transformacion))]
        );
    }

    /// **EL CRITERIO DE M2, y la mitad que nadie calcula.**
    ///
    /// `nif` no aparece en la salida: solo está en el `WHERE`. Y decide qué
    /// filas salen, así que influye en **todas** las columnas de salida. Si esa
    /// columna es `gdpr.sensitivity: critical`, filtrar por ella filtra
    /// información crítica hacia un resultado que nadie clasificó.
    #[test]
    fn una_columna_que_solo_esta_en_un_filtro_sale_como_indirecta() {
        let p = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: eq_str("nif", "12345678Z"),
            }),
            campos: [("cuanto".to_string(), Expr::campo("total"))].into(),
        };

        // `nif` NO está en el esquema de salida.
        assert!(!crate::esquema(&p).unwrap().contains_key("nif"));

        // Y sin embargo está en el linaje de `cuanto`, como influencia.
        let l = linaje(&p).expect("cuadra");
        let raices = de(&l, "cuanto");
        assert!(
            raices.contains(&("nif".to_string(), Clase::Indirecto(Indirecta::Filtro))),
            "el filtro implícito se perdió: {raices:?}"
        );
        assert!(raices.contains(&("total".to_string(), Clase::Directo(Directa::Identidad))));
        assert_eq!(raices.len(), 2, "{raices:?}");
    }

    /// **La regla de composición.** Derivar es transitivo solo a través de
    /// derivaciones: una columna que llegó como influencia sigue siendo
    /// influencia por muchos pasos directos que se le apilen encima.
    ///
    /// Sin esto, proyectar después de filtrar borraría el flujo implícito — y el
    /// análisis parecería más limpio justo donde deja de ser cierto.
    #[test]
    fn una_influencia_no_se_convierte_en_derivacion_al_proyectarla() {
        let filtrado = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: eq_str("nif", "X"),
        };
        // Cuatro proyecciones encadenadas por encima del filtro.
        let mut p = filtrado;
        for _ in 0..4 {
            p = Nodo::Proyecta {
                entrada: Box::new(p),
                campos: [("total".to_string(), Expr::campo("total"))].into(),
            };
        }
        let raices = de(&linaje(&p).expect("cuadra"), "total");
        assert!(
            raices.contains(&("nif".to_string(), Clase::Indirecto(Indirecta::Filtro))),
            "{raices:?}"
        );
    }

    /// Las claves de una junta deciden qué filas se emparejan: influyen en todo
    /// lo que sale, **de los dos lados**.
    #[test]
    fn las_claves_de_una_junta_influyen_en_las_dos_mitades() {
        let p = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(hoja(
                "ventas.lineas",
                &[("id_pedido", "Integer"), ("unidades", "Integer")],
            )),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        let l = linaje(&p).expect("cuadra");
        for salida in ["total", "unidades"] {
            let r = de(&l, salida);
            assert!(
                r.contains(&("id".to_string(), Clase::Indirecto(Indirecta::Junta))),
                "{salida}: {r:?}"
            );
            assert!(
                r.contains(&("id_pedido".to_string(), Clase::Indirecto(Indirecta::Junta))),
                "{salida}: {r:?}"
            );
        }
    }

    /// Un agregado deriva de su columna, y la clave de grupo influye en él. Y
    /// `cuenta` sin columna **no deriva de ninguna**: lo único que decide su
    /// valor es qué filas caen en el grupo.
    #[test]
    fn un_agregado_deriva_de_su_columna_y_el_grupo_influye() {
        let p = Nodo::Agrupa {
            entrada: Box::new(pedidos()),
            por: ["nif".to_string()].into(),
            agregados: [
                (
                    "suma".to_string(),
                    Agregacion {
                        funcion: Agregado::Suma,
                        sobre: Some("total".into()),
                    },
                ),
                (
                    "n".to_string(),
                    Agregacion {
                        funcion: Agregado::Cuenta,
                        sobre: None,
                    },
                ),
            ]
            .into(),
        };
        let l = linaje(&p).expect("cuadra");

        let suma = de(&l, "suma");
        assert!(suma.contains(&("total".to_string(), Clase::Directo(Directa::Agregacion))));
        assert!(suma.contains(&("nif".to_string(), Clase::Indirecto(Indirecta::Agrupacion))));

        // `cuenta` no tiene raíz directa, y aun así **no está huérfana**.
        let n = de(&l, "n");
        assert_eq!(
            n,
            [("nif".to_string(), Clase::Indirecto(Indirecta::Agrupacion))]
        );

        // La clave de grupo sale como ella misma.
        assert_eq!(
            de(&l, "nif"),
            [("nif".to_string(), Clase::Directo(Directa::Identidad))]
        );
    }

    /// **Quitar duplicados es agrupar por todas las columnas.** Si `nif` decide
    /// qué filas sobreviven a un `DISTINCT`, influye en `total` aunque no lo
    /// toque.
    #[test]
    fn distinguir_hace_que_toda_columna_influya_en_todas() {
        let p = Nodo::Proyecta {
            entrada: Box::new(Nodo::Distingue(Box::new(pedidos()))),
            campos: [("cuanto".to_string(), Expr::campo("total"))].into(),
        };
        let r = de(&linaje(&p).expect("cuadra"), "cuanto");
        assert!(
            r.contains(&("nif".to_string(), Clase::Indirecto(Indirecta::Agrupacion))),
            "{r:?}"
        );
        assert!(
            r.contains(&("id".to_string(), Clase::Indirecto(Indirecta::Agrupacion))),
            "{r:?}"
        );
    }

    /// Un límite **sin orden** no lo decide ninguna columna, así que no inventa
    /// aristas. El día que haya un nodo de orden, sus columnas darán `SORT`.
    #[test]
    fn un_limite_sin_orden_no_inventa_influencias() {
        let p = Nodo::Limita {
            entrada: Box::new(pedidos()),
            n: 10,
        };
        assert_eq!(
            de(&linaje(&p).expect("cuadra"), "total"),
            [("total".to_string(), Clase::Directo(Directa::Identidad))]
        );
    }

    /// Una unión junta las raíces de las dos ramas bajo la misma columna: es lo
    /// que hace que una columna con dos orígenes se gobierne por los dos.
    #[test]
    fn una_union_acumula_las_raices_de_todas_las_ramas() {
        let p = Nodo::Unifica(vec![
            pedidos(),
            hoja(
                "ventas.pedidos_viejos",
                &[("id", "Integer"), ("total", "Decimal"), ("nif", "String")],
            ),
        ]);
        let l = linaje(&p).expect("cuadra");
        let objetos: Vec<&str> = l["total"].iter().map(|a| a.raiz.objeto.as_str()).collect();
        assert_eq!(objetos, ["ventas.pedidos", "ventas.pedidos_viejos"]);
    }

    /// Un plan sin expandir no tiene linaje, por lo mismo que no tiene esquema.
    #[test]
    fn un_plan_sin_expandir_no_tiene_linaje() {
        assert_eq!(
            linaje(&Nodo::Referencia("otra".into())),
            Err(Desajuste::SinExpandir {
                vista: "otra".into()
            })
        );
    }

    /// **El facet de OpenLineage**, con su vocabulario y no con uno propio: un
    /// linaje que solo entienda esta herramienta no vale en un catálogo que ya
    /// existe.
    #[test]
    fn se_emite_el_facet_de_openlineage() {
        let p = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: eq_str("nif", "X"),
            }),
            campos: [("cuanto".to_string(), Expr::campo("total"))].into(),
        };
        let j = facet(&linaje(&p).expect("cuadra")).jcs();

        // La forma: `fields` → columna → `inputFields` → namespace/name/field.
        assert!(j.contains(r#""fields":{"cuanto":{"inputFields":["#), "{j}");
        assert!(j.contains(r#""namespace":"lago""#), "{j}");
        assert!(j.contains(r#""name":"ventas.pedidos""#), "{j}");
        // Y las dos clases, con el vocabulario ajeno tal cual.
        assert!(
            j.contains(r#"{"subtype":"IDENTITY","type":"DIRECT"}"#),
            "{j}"
        );
        assert!(
            j.contains(r#"{"subtype":"FILTER","type":"INDIRECT"}"#),
            "{j}"
        );
    }

    /// Y una raíz que llega por dos vías es **un** `inputField` con dos
    /// transformaciones, que es como OpenLineage lo modela — no dos entradas
    /// para la misma columna.
    #[test]
    fn una_raiz_que_llega_por_dos_vias_es_una_entrada_con_dos_transformaciones() {
        // `total` sale de sí misma Y filtra por sí misma.
        let p = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Compara {
                op: Comparador::Mayor,
                izquierda: Box::new(Expr::campo("total")),
                derecha: Box::new(Expr::Literal(Valor::Decimal("0".into()))),
            },
        };
        let l = linaje(&p).expect("cuadra");
        assert_eq!(l["total"].len(), 2, "{:?}", l["total"]);

        let j = facet(&l).jcs();
        // Una sola entrada para `total` bajo la columna `total`, con las dos.
        let campo_total = j
            .split(r#""total":{"inputFields":["#)
            .nth(1)
            .expect("hay columna total");
        let entrada = campo_total.split(']').next().unwrap();
        assert_eq!(
            entrada.matches(r#""field":"total""#).count(),
            1,
            "la raíz se duplicó: {entrada}"
        );
        assert!(
            entrada.contains("IDENTITY") && entrada.contains("FILTER"),
            "{entrada}"
        );
    }
}
