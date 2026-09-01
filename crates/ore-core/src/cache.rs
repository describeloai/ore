//! El **manifiesto de caché**: la mitad del tercer plano que sí es nuestra.
//!
//! El [ADR 0006](../../docs/decisions/0006-el-artefacto-de-topologia.md) decidió
//! que la carga útil se materializa como **una tabla Iceberg en el lago del
//! cliente**, y que esa tabla no es nuestra. La decisión es buena y deja una
//! pregunta sin dueño:
//!
//! > **¿Esta caché puede servir esta consulta?**
//!
//! No la puede contestar la tabla, porque una tabla con filas dentro tiene el
//! mismo aspecto sirva o no sirva. La contesta el manifiesto, que es pequeño, es
//! metadato y es exactamente lo que la tesis del proyecto dice que se posee:
//!
//! > *Las superficies ontológicas se construyen **federando el dato** y
//! > **almacenando el contexto y la topología**.*
//!
//! Los bytes de la caché son del cliente. **La afirmación sobre bajo qué se
//! escribieron es nuestra**, y sin ella la caché es un acelerador sin gobierno.
//!
//! # Cuatro motivos para no servir, y no se arreglan igual
//!
//! Es toda la razón de que esto sea un enum y no un `bool`:
//!
//! | Veredicto | Qué pasó | Se arregla |
//! |---|---|---|
//! | [`Veredicto::ReglaDistinta`] | se materializó bajo **otro bundle** | **reconstruyendo** |
//! | [`Veredicto::CorrespondenciaDistinta`] | otra versión de topología | reconstruyendo |
//! | [`Veredicto::Incompleta`] | no tiene esa propiedad | ampliando la materialización |
//! | [`Veredicto::Rancia`] | la marca no llega al SLA | **refrescando** |
//!
//! La primera fila es la que justifica el módulo entero. El ADR 0006 la nombra
//! al final y nada la comprobaba:
//!
//! > *Refrescar responde a que el dato cambió; reconstruir, a que la REGLA
//! > cambió. Un efecto computado bajo una regla nueva sobre datos enmascarados
//! > con la vieja es la clase de fallo que no tiene aspecto de fallo.*
//!
//! Una caché escrita cuando `nif` no llevaba `gdpr.sensitivity: critical`
//! contiene el `nif` **en claro**. Recompilar el paquete con la etiqueta puesta
//! cambia lo que el conducto autoriza y **no cambia una sola fila de la tabla**.
//! Servirla es exportar el dato que la política nueva acaba de prohibir, con
//! todos los indicadores en verde. Por eso `ReglaDistinta` se comprueba antes
//! que nada y por eso *«refresca»* es el consejo equivocado: refrescar reescribe
//! filas bajo la misma pregunta, y la pregunta es la que cambió.
//!
//! # Y por qué esto vive dentro del compilador
//!
//! Porque no necesita nada de lo que está vetado. No abre la tabla, no lee el
//! reloj —el instante llega con la consulta, como en todo lo demás— y no
//! necesita credenciales. Es aritmética sobre un fichero que ya está en el
//! árbol, igual que verificar una firma. Leer la tabla es el otro acto y es de
//! quien tiene el driver.

use crate::frescura::retraso;
use crate::json::Json;
use crate::parse::Node;

/// El marcador de formato. La misma doctrina que `ORETOPO1`: un fichero dice qué
/// es antes de que nadie lo interprete.
pub const FORMATO: i64 = 1;

/// Una materialización: qué se guardó, dónde, y **bajo qué**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrada {
    /// La entidad cualificada cuya carga útil está materializada.
    pub entidad: String,
    /// Las propiedades que la tabla contiene. **Ordenadas**: el manifiesto es un
    /// artefacto y dos escrituras del mismo estado tienen que dar los mismos
    /// bytes, como todo lo demás aquí.
    pub propiedades: Vec<String>,
    /// El digest del bundle **bajo el que se escribieron estas filas**. Es el
    /// campo del que depende todo el módulo.
    pub bundle: String,
    /// La versión de topología que resolvió las claves, si intervino una
    /// travesía. `None` significa *«no hizo falta»*, no *«se desconoce»*.
    pub topologia: Option<String>,
    /// Hasta cuándo era cierto lo que hay dentro.
    pub marca: String,
    /// Dónde está la tabla. Es una coordenada en el catálogo del cliente y
    /// **este campo es lo único que no gobernamos**: se transporta para que
    /// quien tenga el driver sepa qué abrir.
    pub tabla: String,
}

/// Lo que se le pregunta a la caché: bajo qué se compiló el plan y qué pide.
///
/// Se pasa explícito en vez de recibir un `Plan` porque el plan vive en el
/// ejecutor y esto vive en el compilador — y porque los cinco campos son
/// exactamente lo que hace falta. Un tipo que arrastrara el plan entero
/// escondería que la decisión no mira nada más.
#[derive(Debug, Clone, Copy)]
pub struct Pregunta<'a> {
    /// El bundle **actual**, el que compiló el plan.
    pub bundle: &'a str,
    /// La versión de topología que el plan usó, si hubo travesía.
    pub topologia: Option<&'a str>,
    pub entidad: &'a str,
    pub propiedades: &'a [String],
    /// Cuándo se pregunta. Llega de fuera: aquí no hay reloj.
    pub instante: Option<&'a str>,
    /// El `freshnessSLA` que aplique.
    pub sla: Option<&'a str>,
}

/// Si la caché sirve, y si no, por qué no. Cada variante tiene un remedio
/// distinto, y esa es la razón de que no sea un `bool`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Veredicto {
    /// Sirve. Se dice qué tabla abrir para no obligar a buscarla otra vez.
    Sirve { tabla: String },
    /// No hay entrada para esa entidad. **No es un fallo**: es que hay que ir a
    /// la fuente, que es lo que se hacía antes de que existiera la caché.
    NoMaterializada,
    /// Se materializó bajo otro bundle. El dato de dentro puede estar
    /// enmascarado según una regla que ya no rige — **en cualquiera de las dos
    /// direcciones**, y la peligrosa es la que devuelve de más.
    ReglaDistinta { bajo: String },
    /// Las claves se resolvieron con otra correspondencia, así que las filas
    /// pueden ser de otras cosas.
    CorrespondenciaDistinta { bajo: Option<String> },
    /// Falta alguna propiedad de las que se piden.
    Incompleta { faltan: Vec<String> },
    /// Está dentro del bundle y de la topología, y **vieja**. Es la única que se
    /// arregla refrescando.
    Rancia { marca: String, retraso: i64 },
}

impl Veredicto {
    pub fn sirve(&self) -> bool {
        matches!(self, Veredicto::Sirve { .. })
    }

    /// Qué hay que hacer. Existe para que la diferencia entre reconstruir y
    /// refrescar salga por la boca de la herramienta y no se quede en el enum.
    pub const fn remedio(&self) -> &'static str {
        match self {
            Veredicto::Sirve { .. } => "nada",
            Veredicto::NoMaterializada => "leer de la fuente",
            Veredicto::ReglaDistinta { .. } | Veredicto::CorrespondenciaDistinta { .. } => {
                "reconstruir: refrescar reescribiría las filas bajo la misma pregunta, y \
                 lo que cambió es la pregunta"
            }
            Veredicto::Incompleta { .. } => "ampliar la materialización, o leer de la fuente",
            Veredicto::Rancia { .. } => "refrescar",
        }
    }

    pub fn como_texto(&self) -> String {
        match self {
            Veredicto::Sirve { tabla } => format!("sirve · {tabla}"),
            Veredicto::NoMaterializada => "no materializada".into(),
            Veredicto::ReglaDistinta { bajo } => format!(
                "regla distinta · se materializó bajo `{bajo}`: las filas pueden estar \
                 enmascaradas según una clasificación que ya no rige"
            ),
            Veredicto::CorrespondenciaDistinta { bajo } => format!(
                "correspondencia distinta · se materializó con la topología `{}`: las claves \
                 pueden apuntar a otras cosas",
                bajo.as_deref().unwrap_or("—")
            ),
            Veredicto::Incompleta { faltan } => {
                format!("incompleta · no tiene {}", faltan.join(", "))
            }
            Veredicto::Rancia { marca, retraso } => format!(
                "rancia · la marca es `{marca}` y lleva {retraso} s de retraso sobre el SLA"
            ),
        }
    }
}

/// Lo que hay materializado. Un fichero, no un servicio.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifiesto {
    pub entradas: Vec<Entrada>,
}

impl Manifiesto {
    /// **La decisión.** Es pura y el orden de las comprobaciones importa.
    ///
    /// `ReglaDistinta` va primero aunque la entrada esté además rancia e
    /// incompleta, porque es la única cuyo remedio no es el obvio: quien lea
    /// *«rancia»* refrescará, y refrescar sobre una regla nueva reescribe las
    /// filas correctas dejando el problema exactamente donde estaba.
    pub fn consultar(&self, p: &Pregunta) -> Veredicto {
        let Some(e) = self.entradas.iter().find(|e| e.entidad == p.entidad) else {
            return Veredicto::NoMaterializada;
        };

        if e.bundle != p.bundle {
            return Veredicto::ReglaDistinta {
                bajo: e.bundle.clone(),
            };
        }

        // Solo si el plan resolvió claves con una travesía. Sin travesía, la
        // topología de la caché es irrelevante: no se usó para nada.
        if let Some(v) = p.topologia
            && e.topologia.as_deref() != Some(v)
        {
            return Veredicto::CorrespondenciaDistinta {
                bajo: e.topologia.clone(),
            };
        }

        let faltan: Vec<String> = p
            .propiedades
            .iter()
            .filter(|prop| !e.propiedades.contains(prop))
            .cloned()
            .collect();
        if !faltan.is_empty() {
            return Veredicto::Incompleta { faltan };
        }

        // Sin instante o sin SLA no hay nada que comparar, y eso NO se cuenta
        // como rancio: nadie declaró cuánto se tolera.
        if let (Some(t), Some(sla)) = (p.instante, p.sla)
            && let Some(r) = retraso(&e.marca, t, sla)
        {
            return Veredicto::Rancia {
                marca: e.marca.clone(),
                retraso: r,
            };
        }

        Veredicto::Sirve {
            tabla: e.tabla.clone(),
        }
    }

    /// La forma canónica. El manifiesto es un artefacto: se firma y se compara,
    /// así que dos escrituras del mismo estado tienen que dar los mismos bytes.
    pub fn jcs(&self) -> String {
        let mut entradas: Vec<&Entrada> = self.entradas.iter().collect();
        entradas.sort_by(|a, b| a.entidad.cmp(&b.entidad));
        Json::obj([
            ("oreCache", Json::Int(FORMATO)),
            (
                "entries",
                Json::Arr(
                    entradas
                        .into_iter()
                        .map(|e| {
                            let mut props: Vec<String> = e.propiedades.clone();
                            props.sort();
                            props.dedup();
                            let mut campos = vec![
                                ("bundle", Json::s(&e.bundle)),
                                ("entity", Json::s(&e.entidad)),
                                ("properties", Json::Arr(props.iter().map(Json::s).collect())),
                                ("table", Json::s(&e.tabla)),
                                ("watermark", Json::s(&e.marca)),
                            ];
                            // Ausente y vacío no son lo mismo: `None` dice «no
                            // hizo falta travesía», y una cadena vacía diría
                            // «hubo una y no se sabe cuál».
                            if let Some(t) = &e.topologia {
                                campos.push(("topology", Json::s(t)));
                            }
                            Json::obj(campos)
                        })
                        .collect(),
                ),
            ),
        ])
        .jcs()
    }

    /// Lee un manifiesto. Se apoya en el analizador que ya existe —JSON es YAML—
    /// en vez de traer un segundo, que sería un segundo sitio donde envejecer.
    pub fn leer(texto: &str) -> Result<Manifiesto, String> {
        let n = crate::parse::parse(texto).map_err(|e| format!("no se puede leer: {e:?}"))?;
        match n.get("oreCache").and_then(|(_, v)| v.as_str()) {
            Some(v) if v == FORMATO.to_string() => {}
            Some(v) => return Err(format!("`oreCache: {v}` no es un formato que se sepa leer")),
            None => return Err("esto no es un manifiesto de caché: no dice `oreCache`".into()),
        }
        let entradas = n
            .get("entries")
            .map(|(_, v)| v.items())
            .unwrap_or_default()
            .iter()
            .map(entrada)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Manifiesto { entradas })
    }
}

fn entrada(n: &Node) -> Result<Entrada, String> {
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let obligatorio = |k: &str| campo(k).ok_or_else(|| format!("una entrada no dice `{k}`"));
    Ok(Entrada {
        entidad: obligatorio("entity")?,
        propiedades: n
            .get("properties")
            .map(|(_, v)| v.items())
            .unwrap_or_default()
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect(),
        bundle: obligatorio("bundle")?,
        topologia: campo("topology"),
        marca: obligatorio("watermark")?,
        tabla: obligatorio("table")?,
    })
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE: &str = "sha256:aaa";
    const TOPO: &str = "sha256:ttt";

    fn manifiesto() -> Manifiesto {
        Manifiesto {
            entradas: vec![Entrada {
                entidad: "ventas.Pedidos".into(),
                propiedades: vec!["id".into(), "total".into(), "fecha".into()],
                bundle: BUNDLE.into(),
                topologia: Some(TOPO.into()),
                marca: "2026-08-31T10:00:00Z".into(),
                tabla: "lago.cache.ventas_pedidos".into(),
            }],
        }
    }

    fn pide(props: &[&str]) -> Vec<String> {
        props.iter().map(|s| (*s).to_string()).collect()
    }

    fn pregunta<'a>(bundle: &'a str, propiedades: &'a [String]) -> Pregunta<'a> {
        Pregunta {
            bundle,
            topologia: Some(TOPO),
            entidad: "ventas.Pedidos",
            propiedades,
            instante: Some("2026-08-31T10:30:00Z"),
            sla: Some("1h"),
        }
    }

    #[test]
    fn dentro_del_bundle_de_la_topologia_y_del_sla_sirve() {
        let props = pide(&["id", "total"]);
        assert_eq!(
            manifiesto().consultar(&pregunta(BUNDLE, &props)),
            Veredicto::Sirve {
                tabla: "lago.cache.ventas_pedidos".into()
            }
        );
    }

    /// **El motivo de este módulo.**
    ///
    /// La caché se escribió cuando `nif` no llevaba `gdpr.sensitivity: critical`,
    /// así que contiene el `nif` en claro. Recompilar con la etiqueta puesta
    /// cambia lo que el conducto autoriza y **no toca una sola fila**. Servirla
    /// sería exportar justo lo que la política nueva prohíbe, con la tabla llena
    /// y todo verde.
    #[test]
    fn una_cache_escrita_bajo_otra_regla_no_sirve() {
        let props = pide(&["id"]);
        let v = manifiesto().consultar(&pregunta("sha256:bbb", &props));
        assert_eq!(
            v,
            Veredicto::ReglaDistinta {
                bajo: BUNDLE.into()
            }
        );
        // Y el remedio NO es refrescar: refrescar reescribe las filas bajo la
        // misma pregunta, y lo que cambió es la pregunta.
        assert!(v.remedio().contains("reconstruir"));
    }

    /// Y gana a los otros tres motivos aunque también se den. Quien lea
    /// «rancia» refrescará, y refrescar deja el problema donde estaba.
    #[test]
    fn la_regla_distinta_se_dice_antes_que_lo_demas() {
        let props = pide(&["id", "no_existe"]);
        let v = manifiesto().consultar(&Pregunta {
            bundle: "sha256:bbb",
            topologia: Some("sha256:otra"),
            entidad: "ventas.Pedidos",
            propiedades: &props,
            instante: Some("2027-01-01T00:00:00Z"),
            sla: Some("1h"),
        });
        assert!(matches!(v, Veredicto::ReglaDistinta { .. }), "{v:?}");
    }

    #[test]
    fn otra_correspondencia_no_sirve_y_tampoco_se_refresca() {
        let props = pide(&["id"]);
        let v = manifiesto().consultar(&Pregunta {
            topologia: Some("sha256:otra"),
            ..pregunta(BUNDLE, &props)
        });
        assert_eq!(
            v,
            Veredicto::CorrespondenciaDistinta {
                bajo: Some(TOPO.into())
            }
        );
        assert!(v.remedio().contains("reconstruir"));
    }

    /// Un plan que no hizo travesía no resolvió ninguna clave con la topología,
    /// así que la de la caché no le concierne. Exigirla igual dejaría la caché
    /// inservible para la mitad de los planes por una razón inventada.
    #[test]
    fn un_plan_sin_travesia_no_mira_la_topologia() {
        let props = pide(&["id"]);
        let v = manifiesto().consultar(&Pregunta {
            topologia: None,
            ..pregunta(BUNDLE, &props)
        });
        assert!(v.sirve(), "{v:?}");
    }

    #[test]
    fn lo_que_no_tiene_dentro_se_dice_por_su_nombre() {
        let props = pide(&["id", "iva", "descuento"]);
        assert_eq!(
            manifiesto().consultar(&pregunta(BUNDLE, &props)),
            Veredicto::Incompleta {
                faltan: vec!["iva".into(), "descuento".into()]
            }
        );
    }

    #[test]
    fn pasado_el_sla_es_rancia_y_se_refresca() {
        let props = pide(&["id"]);
        let v = manifiesto().consultar(&Pregunta {
            instante: Some("2026-08-31T12:00:00Z"),
            ..pregunta(BUNDLE, &props)
        });
        assert_eq!(
            v,
            Veredicto::Rancia {
                marca: "2026-08-31T10:00:00Z".into(),
                retraso: 3_600
            }
        );
        assert_eq!(v.remedio(), "refrescar");
    }

    /// Sin SLA declarado no hay rancio: nadie dijo cuánto se tolera, y
    /// inventarlo fallaría en una de las dos direcciones sin decirlo.
    #[test]
    fn sin_sla_no_se_inventa_un_umbral() {
        let props = pide(&["id"]);
        let v = manifiesto().consultar(&Pregunta {
            instante: Some("2030-01-01T00:00:00Z"),
            sla: None,
            ..pregunta(BUNDLE, &props)
        });
        assert!(v.sirve(), "{v:?}");
    }

    /// Que no haya entrada no es un fallo: es ir a la fuente, que es lo que se
    /// hacía antes de que existiera la caché.
    #[test]
    fn una_entidad_sin_materializar_manda_a_la_fuente() {
        let props = pide(&["id"]);
        let v = manifiesto().consultar(&Pregunta {
            entidad: "ventas.Clientes",
            ..pregunta(BUNDLE, &props)
        });
        assert_eq!(v, Veredicto::NoMaterializada);
        assert_eq!(v.remedio(), "leer de la fuente");
    }

    #[test]
    fn el_manifiesto_va_y_vuelve() {
        let m = manifiesto();
        let leido = Manifiesto::leer(&m.jcs()).expect("se lee");
        // Las propiedades salen ordenadas de la forma canónica, así que la
        // igualdad se compara sobre la forma, no sobre el orden de escritura.
        assert_eq!(leido.jcs(), m.jcs());
        assert_eq!(leido.entradas[0].bundle, BUNDLE);
        assert_eq!(leido.entradas[0].topologia.as_deref(), Some(TOPO));
    }

    /// Dos escrituras del mismo estado, los mismos bytes — aunque las entradas
    /// y las propiedades lleguen en otro orden. Es G1 aplicado a este artefacto.
    #[test]
    fn el_orden_de_llegada_no_cambia_los_bytes() {
        let a = manifiesto();
        let mut b = manifiesto();
        b.entradas[0].propiedades.reverse();
        b.entradas.push(Entrada {
            entidad: "ventas.Clientes".into(),
            propiedades: vec!["id".into()],
            bundle: BUNDLE.into(),
            topologia: None,
            marca: "2026-08-31T10:00:00Z".into(),
            tabla: "lago.cache.ventas_clientes".into(),
        });
        let mut c = b.clone();
        c.entradas.reverse();
        assert_eq!(a.jcs(), manifiesto().jcs());
        assert_eq!(b.jcs(), c.jcs());
    }

    /// Y una entrada sin travesía no escribe `topology`: ausente y vacío no son
    /// lo mismo — uno dice «no hizo falta» y el otro «hubo una y no se sabe».
    #[test]
    fn sin_travesia_el_campo_no_se_escribe() {
        let m = Manifiesto {
            entradas: vec![Entrada {
                topologia: None,
                ..manifiesto().entradas.remove(0)
            }],
        };
        assert!(!m.jcs().contains("topology"), "{}", m.jcs());
        assert!(
            Manifiesto::leer(&m.jcs()).expect("se lee").entradas[0]
                .topologia
                .is_none()
        );
    }

    /// Lo que no es esto se dice, en vez de leerse a medias.
    #[test]
    fn lo_que_no_es_un_manifiesto_no_se_intenta_interpretar() {
        assert!(Manifiesto::leer("{}").is_err());
        assert!(Manifiesto::leer(r#"{"oreCache":9,"entries":[]}"#).is_err());
        // Y una entrada a la que le falta el campo del que depende todo.
        assert!(
            Manifiesto::leer(r#"{"oreCache":1,"entries":[{"entity":"a","table":"t"}]}"#).is_err()
        );
    }
}
