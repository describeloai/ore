//! El **esquema de un plan**, derivado del plan.
//!
//! Es lo que convierte el IR en un álgebra *tipada* y no en un árbol de datos.
//! Y no es un lujo: sin esquema no hay linaje a nivel de columna —M2— y sin
//! linaje no hay retículo que fluya —M3—, que es el peldaño por el que existe
//! todo esto.
//!
//! # Se calcula, no se declara
//!
//! Solo las hojas declaran su esquema, porque son las únicas que miran algo que
//! no controlamos. Todo lo demás **se deriva**. Un plan que llevara su esquema
//! escrito tendría dos verdades y ninguna diría cuál manda — que es P2: *lo
//! derivable no se declara*.
//!
//! # Cuatro cosas que aquí se cazan y que en SQL no fallan hasta ejecutar
//!
//! | | Qué pasa en un almacén |
//! |---|---|
//! | juntar dos tablas con una columna del mismo nombre | una gana en silencio, o hay que cualificar |
//! | unir dos ramas con columnas distintas | error, pero al ejecutar |
//! | comparar tipos que no se comparan | conversión implícita, y cifras incorrectas |
//! | una expresión opaca que lee una columna que no existe | error, y al ejecutar |
//!
//! Las cuatro fallan **aquí**, sin abrir nada.

use crate::plan::{Agregado, Expr, Nodo, Opaca};
use ore_core::types::Type;
use std::collections::BTreeMap;

/// Campo → tipo. Ordenado, porque de aquí sale la forma canónica de lo que un
/// plan produce.
pub type Esquema = BTreeMap<String, Type>;

fn escalar(s: &str) -> Type {
    Type::Scalar(s.to_string())
}

/// Por qué un plan no cuadra. Cada variante es un fallo distinto y se cuentan
/// aparte porque no se arreglan igual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Desajuste {
    /// Se nombra un campo que la entrada no tiene.
    CampoDesconocido { donde: &'static str, campo: String },
    /// Dos lados de una junta traen un campo con el mismo nombre. **No se
    /// resuelve por precedencia**: una de las dos columnas desaparecería sin
    /// decirlo, y quien lea el resultado creería estar mirando la otra.
    ColisionAlUnir { campo: String },
    /// Las ramas de una unión no producen lo mismo.
    RamasDesiguales { campo: String, porque: String },
    /// Comparar dos tipos que no son el mismo. **No hay ensanchamiento
    /// implícito**: comparar un `Decimal` con un `String` no falla en un almacén
    /// — da cifras incorrectas, que es peor.
    TiposIncomparables { izquierda: Type, derecha: Type },
    /// Donde hacía falta una condición hay otra cosa.
    NoEsBooleano { donde: &'static str, tipo: Type },
    /// Un agregado que necesita una columna y no la tiene. Solo `cuenta` puede
    /// no tenerla: sumar la nada no significa nada.
    AgregadoSinCampo { nombre: String },
    /// Se pide el esquema de un plan que todavía nombra una vista.
    ///
    /// **Lo señaló el compilador al añadir la referencia**, y la respuesta
    /// correcta no era «trátala como una hoja vacía»: un plan sin expandir no se
    /// puede tipar porque le faltan trozos, y devolver un esquema parcial daría
    /// un análisis que parece bueno sobre medio plan.
    SinExpandir { vista: String },
    /// Una expresión opaca que dice leer un campo que no existe. **Es el único
    /// control que tiene**, y por eso no se puede saltar: si su superficie
    /// declarada miente, las etiquetas dejan de fluir por donde de verdad pasan.
    OpacaLeeLoQueNoHay { campo: String },
}

impl Desajuste {
    pub fn como_texto(&self) -> String {
        match self {
            Desajuste::CampoDesconocido { donde, campo } => {
                format!("`{campo}` no está en la entrada de `{donde}`")
            }
            Desajuste::ColisionAlUnir { campo } => format!(
                "los dos lados de la junta traen `{campo}`: una de las dos columnas \
                 desaparecería sin decirlo, y quien lea el resultado creería estar mirando \
                 la otra"
            ),
            Desajuste::RamasDesiguales { campo, porque } => {
                format!("las ramas de la unión no coinciden en `{campo}`: {porque}")
            }
            Desajuste::TiposIncomparables { izquierda, derecha } => format!(
                "no se comparan `{izquierda}` y `{derecha}`: no hay ensanchamiento implícito, \
                 porque una conversión en silencio no da un error — da cifras incorrectas"
            ),
            Desajuste::NoEsBooleano { donde, tipo } => {
                format!("`{donde}` necesita una condición y esto es `{tipo}`")
            }
            Desajuste::AgregadoSinCampo { nombre } => {
                format!("`{nombre}` agrega sin decir sobre qué, y solo `cuenta` puede")
            }
            Desajuste::SinExpandir { vista } => format!(
                "el plan todavía nombra a `{vista}`: hay que expandirlo antes de tiparlo,                  porque un esquema sobre medio plan parece bueno"
            ),
            Desajuste::OpacaLeeLoQueNoHay { campo } => format!(
                "una expresión opaca declara leer `{campo}` y no está: su superficie declarada \
                 es lo único que se puede comprobar de ella"
            ),
        }
    }
}

/// El esquema que produce un plan.
pub fn esquema(n: &Nodo) -> Result<Esquema, Desajuste> {
    match n {
        Nodo::Lee(l) => Ok(l.campos.clone()),

        Nodo::Referencia(v) => Err(Desajuste::SinExpandir { vista: v.clone() }),

        Nodo::Proyecta { entrada, campos } => {
            let dentro = esquema(entrada)?;
            campos
                .iter()
                .map(|(nombre, e)| Ok((nombre.clone(), tipo_de(e, &dentro, "proyecta")?)))
                .collect()
        }

        Nodo::Filtra { entrada, predicado } => {
            let dentro = esquema(entrada)?;
            exige_condicion(predicado, &dentro, "filtra")?;
            Ok(dentro)
        }

        Nodo::Une {
            izquierda,
            derecha,
            sobre,
            ..
        } => {
            let (i, d) = (esquema(izquierda)?, esquema(derecha)?);
            // La colisión primero: sin esto, el resto del análisis razonaría
            // sobre un esquema que no describe lo que sale.
            if let Some(campo) = i.keys().find(|k| d.contains_key(*k)) {
                return Err(Desajuste::ColisionAlUnir {
                    campo: campo.clone(),
                });
            }
            for (a, b) in sobre {
                exige(&i, a, "une · izquierda")?;
                exige(&d, b, "une · derecha")?;
            }
            let mut out = i;
            out.extend(d);
            Ok(out)
        }

        Nodo::Agrupa {
            entrada,
            por,
            agregados,
        } => {
            let dentro = esquema(entrada)?;
            let mut out = Esquema::new();
            for c in por {
                out.insert(c.clone(), exige(&dentro, c, "agrupa")?);
            }
            for (nombre, a) in agregados {
                let t = match (a.funcion, &a.sobre) {
                    // `cuenta` cuenta filas: no necesita columna, y lo que
                    // devuelve no depende de ninguna.
                    (Agregado::Cuenta, _) => escalar("Integer"),
                    (_, None) => {
                        return Err(Desajuste::AgregadoSinCampo {
                            nombre: nombre.clone(),
                        });
                    }
                    // El promedio de enteros no es un entero, y decir que sí lo
                    // es sería el primer sitio por donde se pierde un decimal.
                    (Agregado::Promedio, Some(_)) => escalar("Decimal"),
                    (_, Some(c)) => exige(&dentro, c, "agrupa")?,
                };
                out.insert(nombre.clone(), t);
            }
            Ok(out)
        }

        Nodo::Unifica(ramas) => {
            let mut iter = ramas.iter();
            let Some(primera) = iter.next() else {
                return Ok(Esquema::new());
            };
            let base = esquema(primera)?;
            for r in iter {
                let otra = esquema(r)?;
                for (campo, t) in &base {
                    match otra.get(campo) {
                        None => {
                            return Err(Desajuste::RamasDesiguales {
                                campo: campo.clone(),
                                porque: "una rama no lo produce".into(),
                            });
                        }
                        Some(o) if o != t => {
                            return Err(Desajuste::RamasDesiguales {
                                campo: campo.clone(),
                                porque: format!("una rama lo da `{t}` y otra `{o}`"),
                            });
                        }
                        _ => {}
                    }
                }
                if let Some(sobra) = otra.keys().find(|k| !base.contains_key(*k)) {
                    return Err(Desajuste::RamasDesiguales {
                        campo: sobra.clone(),
                        porque: "una rama lo produce y otra no".into(),
                    });
                }
            }
            Ok(base)
        }

        Nodo::Distingue(e) => esquema(e),
        Nodo::Limita { entrada, .. } => esquema(entrada),
    }
}

fn exige(e: &Esquema, campo: &str, donde: &'static str) -> Result<Type, Desajuste> {
    e.get(campo).cloned().ok_or(Desajuste::CampoDesconocido {
        donde,
        campo: campo.to_string(),
    })
}

fn exige_condicion(x: &Expr, e: &Esquema, donde: &'static str) -> Result<(), Desajuste> {
    let t = tipo_de(x, e, donde)?;
    if t == escalar("Boolean") {
        Ok(())
    } else {
        Err(Desajuste::NoEsBooleano { donde, tipo: t })
    }
}

/// La superficie declarada de una opaca **se comprueba**, aunque su cuerpo no.
fn comprueba_opaca(o: &Opaca, e: &Esquema) -> Result<(), Desajuste> {
    for c in &o.lee {
        if !e.contains_key(c) {
            return Err(Desajuste::OpacaLeeLoQueNoHay { campo: c.clone() });
        }
    }
    Ok(())
}

pub fn tipo_de(x: &Expr, e: &Esquema, donde: &'static str) -> Result<Type, Desajuste> {
    Ok(match x {
        Expr::Campo(c) => exige(e, c, donde)?,
        Expr::Literal(v) => v.tipo(),

        // El comparador no se mira: los seis dan `Boolean`. Lo que sí se mira
        // son los dos lados.
        Expr::Compara {
            izquierda, derecha, ..
        } => {
            let (i, d) = (tipo_de(izquierda, e, donde)?, tipo_de(derecha, e, donde)?);
            if i != d {
                return Err(Desajuste::TiposIncomparables {
                    izquierda: i,
                    derecha: d,
                });
            }
            escalar("Boolean")
        }

        Expr::EnConjunto { campo, valores } => {
            let t = exige(e, campo, donde)?;
            if let Some(v) = valores.iter().find(|v| v.tipo() != t) {
                return Err(Desajuste::TiposIncomparables {
                    izquierda: t,
                    derecha: v.tipo(),
                });
            }
            escalar("Boolean")
        }

        // Cualquier cosa puede ser nula, así que solo hay que comprobar que la
        // cosa exista.
        Expr::EsNulo(x) => {
            tipo_de(x, e, donde)?;
            escalar("Boolean")
        }

        Expr::Y(v) | Expr::O(v) => {
            for x in v {
                exige_condicion(x, e, donde)?;
            }
            escalar("Boolean")
        }
        Expr::No(x) => {
            exige_condicion(x, e, donde)?;
            escalar("Boolean")
        }

        Expr::Opaca(o) => {
            comprueba_opaca(o, e)?;
            // El tipo se cree. Es el precio de la escapatoria, y está dicho.
            o.tipo.clone()
        }
    })
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Agregacion, Comparador, Junta, Lectura, Valor};
    use ore_core::types::parse_type;

    fn t(s: &str) -> Type {
        parse_type(s).expect("un tipo de OOS")
    }

    fn hoja(objeto: &str, campos: &[(&str, &str)]) -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: objeto.into(),
            campos: campos
                .iter()
                .map(|(n, x)| ((*n).to_string(), t(x)))
                .collect(),
        })
    }

    fn pedidos() -> Nodo {
        hoja(
            "ventas.pedidos",
            &[("id", "Integer"), ("total", "Decimal"), ("pais", "String")],
        )
    }

    fn lit(v: Valor) -> Expr {
        Expr::Literal(v)
    }

    /// El esquema se **deriva**: solo las hojas lo declaran, porque son las
    /// únicas que miran algo que no controlamos.
    #[test]
    fn el_esquema_sale_del_plan_y_no_de_una_declaracion() {
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [
                ("donde".to_string(), Expr::campo("pais")),
                ("cuanto".to_string(), Expr::campo("total")),
            ]
            .into(),
        };
        let e = esquema(&p).expect("cuadra");
        assert_eq!(e.len(), 2, "{e:?}");
        assert_eq!(e["donde"], t("String"));
        assert_eq!(e["cuanto"], t("Decimal"));
        // Y lo que la proyección no nombra **no sale**: es la misma regla que
        // hace que una máscara `redact` se aplique no pidiendo la columna.
        assert!(!e.contains_key("id"));
    }

    /// **Una junta con una columna del mismo nombre a los dos lados.** En un
    /// almacén una gana en silencio o hay que cualificar; aquí es un error, y
    /// dice cuál es la columna.
    #[test]
    fn dos_lados_con_la_misma_columna_no_se_juntan_en_silencio() {
        let p = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(hoja(
                "ventas.lineas",
                &[("id", "Integer"), ("unidades", "Integer")],
            )),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "id".into())],
        };
        assert_eq!(
            esquema(&p),
            Err(Desajuste::ColisionAlUnir { campo: "id".into() })
        );
        assert!(
            esquema(&p)
                .unwrap_err()
                .como_texto()
                .contains("sin decirlo"),
            "el mensaje tiene que decir por qué importa"
        );
    }

    /// Y una clave de junta que no está a su lado se dice **por su lado**: sin
    /// eso, buscarla es mirar dos esquemas a mano.
    #[test]
    fn una_clave_de_junta_que_no_esta_se_dice_de_que_lado() {
        let p = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(hoja("ventas.lineas", &[("id_pedido", "Integer")])),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "no_existe".into())],
        };
        assert_eq!(
            esquema(&p),
            Err(Desajuste::CampoDesconocido {
                donde: "une · derecha",
                campo: "no_existe".into()
            })
        );
    }

    /// **Dos ramas de una unión que no producen lo mismo.**
    #[test]
    fn las_ramas_de_una_union_tienen_que_producir_lo_mismo() {
        let falta = Nodo::Unifica(vec![
            pedidos(),
            hoja("ventas.viejos", &[("id", "Integer"), ("total", "Decimal")]),
        ]);
        assert_eq!(
            esquema(&falta),
            Err(Desajuste::RamasDesiguales {
                campo: "pais".into(),
                porque: "una rama no lo produce".into()
            })
        );

        // Y el caso que de verdad se escapa: los mismos nombres con OTRO tipo.
        let tipo = Nodo::Unifica(vec![
            pedidos(),
            hoja(
                "ventas.viejos",
                &[("id", "Integer"), ("total", "String"), ("pais", "String")],
            ),
        ]);
        let Err(Desajuste::RamasDesiguales { campo, porque }) = esquema(&tipo) else {
            panic!("tenía que discrepar en el tipo");
        };
        assert_eq!(campo, "total");
        assert!(
            porque.contains("Decimal") && porque.contains("String"),
            "{porque}"
        );
    }

    /// **Comparar tipos que no se comparan.** En un almacén hay conversión
    /// implícita y no falla: da cifras incorrectas, que es peor.
    #[test]
    fn no_hay_ensanchamiento_implicito() {
        let p = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Compara {
                op: Comparador::Mayor,
                izquierda: Box::new(Expr::campo("total")),
                derecha: Box::new(lit(Valor::Entero(100))),
            },
        };
        assert_eq!(
            esquema(&p),
            Err(Desajuste::TiposIncomparables {
                izquierda: t("Decimal"),
                derecha: t("Integer")
            })
        );
        // Escrito como decimal, cuadra. La salida existe y es una línea.
        let bien = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Compara {
                op: Comparador::Mayor,
                izquierda: Box::new(Expr::campo("total")),
                derecha: Box::new(lit(Valor::Decimal("100".into()))),
            },
        };
        assert!(esquema(&bien).is_ok());
    }

    /// Un conjunto con un valor de otro tipo es el mismo fallo por otra puerta.
    #[test]
    fn un_conjunto_mezclado_tampoco_pasa() {
        let p = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::EnConjunto {
                campo: "pais".into(),
                valores: vec![Valor::Cadena("ES".into()), Valor::Entero(34)],
            },
        };
        assert!(matches!(
            esquema(&p),
            Err(Desajuste::TiposIncomparables { .. })
        ));
    }

    /// Donde hace falta una condición, una columna no vale.
    #[test]
    fn un_filtro_necesita_una_condicion() {
        let p = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::campo("pais"),
        };
        assert_eq!(
            esquema(&p),
            Err(Desajuste::NoEsBooleano {
                donde: "filtra",
                tipo: t("String")
            })
        );
    }

    /// **La superficie declarada de una opaca se comprueba, aunque su cuerpo
    /// no.** Es el único control que tiene: si `lee` miente, las etiquetas
    /// dejan de fluir por donde de verdad pasan.
    #[test]
    fn una_opaca_que_dice_leer_lo_que_no_hay_no_pasa() {
        let opaca = |lee: &str| {
            Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "algo".into(),
                lee: vec![lee.to_string()],
                tipo: t("Date"),
                determinista: true,
            })
        };
        let malo = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("cuando".to_string(), opaca("fecha_cruda"))].into(),
        };
        assert_eq!(
            esquema(&malo),
            Err(Desajuste::OpacaLeeLoQueNoHay {
                campo: "fecha_cruda".into()
            })
        );

        // Y con una columna que sí está, pasa — y el tipo declarado se cree,
        // que es el precio de la escapatoria y está dicho.
        let bueno = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("cuando".to_string(), opaca("pais"))].into(),
        };
        assert_eq!(esquema(&bueno).expect("cuadra")["cuando"], t("Date"));
    }

    /// Los agregados: `cuenta` sin columna vale, los demás no. Y el promedio de
    /// enteros no es un entero.
    #[test]
    fn los_agregados_dicen_de_que_tipo_salen() {
        let agrupa = |nombre: &str, a: Agregacion| Nodo::Agrupa {
            entrada: Box::new(pedidos()),
            por: ["pais".to_string()].into(),
            agregados: [(nombre.to_string(), a)].into(),
        };

        let e = esquema(&agrupa(
            "n",
            Agregacion {
                funcion: Agregado::Cuenta,
                sobre: None,
            },
        ))
        .expect("cuenta no necesita columna");
        assert_eq!(e["n"], t("Integer"));
        assert_eq!(e["pais"], t("String"), "la clave de grupo sale");

        assert_eq!(
            esquema(&agrupa(
                "suma",
                Agregacion {
                    funcion: Agregado::Suma,
                    sobre: None
                }
            )),
            Err(Desajuste::AgregadoSinCampo {
                nombre: "suma".into()
            })
        );

        // El promedio de enteros no es un entero: decir que sí lo es sería el
        // primer sitio por donde se pierde un decimal.
        let e = esquema(&agrupa(
            "media",
            Agregacion {
                funcion: Agregado::Promedio,
                sobre: Some("id".into()),
            },
        ))
        .expect("cuadra");
        assert_eq!(e["media"], t("Decimal"));
    }

    /// Y un campo que no existe se dice **diciendo dónde**: un error que no
    /// nombra el operador obliga a buscarlo a mano en un plan anidado.
    #[test]
    fn un_campo_que_no_existe_dice_en_que_operador() {
        let p = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("x".to_string(), Expr::campo("no_existe"))].into(),
        };
        assert_eq!(
            esquema(&p),
            Err(Desajuste::CampoDesconocido {
                donde: "proyecta",
                campo: "no_existe".into()
            })
        );
    }
}
