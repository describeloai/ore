//! **Partial State Store**: se cachea lo que se pide, se desaloja lo que no, y
//! lo que falte se repone yendo a la fuente.
//!
//! Es la **única** de las doce piezas que guarda algo, y por eso es la que más
//! reglas tiene. Los tres términos son de **Noria** (Gjengset et al., OSDI'18):
//! *partial state*, *eviction*, *upquery*. Su modelo, dicho en una línea: cada
//! operador mantiene **solo un subconjunto** de su estado; los avisos de desalojo
//! fluyen hacia delante, y las *upqueries* hacia atrás repueblan lo que falte.
//!
//! # Por qué esta forma, y no las otras dos del sector
//!
//! **Materialize** lo tiene todo en memoria —*arrangements*, índices por clave y
//! tiempo—. **Feldera** lo desborda a disco con *checkpoints*. Las dos poseen sus
//! datos. Nosotros no, y eso es lo que hace que Noria encaje sin forzarlo: en un
//! sistema que posee sus datos la *upquery* va a un operador de más abajo; **en
//! el nuestro, la de más abajo es la fuente del cliente**.
//!
//! > **Una *upquery* es un plan.** Y el Pushdown Planner ya sabe repartirlo.
//!
//! # Lo que esta pieza es, y lo que no
//!
//! Es el **contrato** del almacén: qué claves están calientes, bajo qué
//! identidades, qué se hace con un delta según la clave esté presente, en vuelo
//! o ausente, y cuándo se desaloja. Lo implementa sobre un [`Zset`] en memoria
//! porque es la **implementación de referencia**, igual que el Delta Compiler es
//! la semántica de referencia del circuito. Dónde viven los bytes —el
//! almacenamiento del cliente, decidido— es del ejecutor que lo adapte; **la
//! política es esta y no cambia con el sitio.**
//!
//! No abre nada. Un *miss* **devuelve** un plan; no lo ejecuta.
//!
//! # Las reglas, y de dónde sale cada una
//!
//! | Regla | De dónde |
//! |---|---|
//! | un *miss* produce **una** *upquery*, y leer la misma clave ausente dos veces no produce dos | Noria: las *upqueries* en vuelo para la misma clave se **coalescen** |
//! | un delta para una clave **ausente** se **descarta** | Noria: *«operators drop updates that would affect evicted state entries»* — la próxima lectura repone desde la fuente, que es la verdad |
//! | un delta para una clave **en vuelo** se guarda, y se aplica **solo si es más nuevo** que el relleno | la carrera clásica entre relleno y actualización, resuelta con la marca |
//! | un delta **más viejo** que lo que hay se descarta | marcas monótonas: lo viejo ya está reflejado |
//! | un relleno **no pedido** se rechaza | P4: un almacén que acepta lo que nadie pidió es un almacén que se puede envenenar |
//! | un relleno bajo **otro bundle** se rechaza | la regla de E1 —`ReglaDistinta`— a granularidad de clave |
//! | se desaloja la clave **menos leída** | LRU sobre un contador lógico, no sobre un reloj |
//!
//! # La marca es un ordinal, no una fecha
//!
//! Todos los testigos de la tabla del anexo —LSN, SCN, offset, `snapshot-id`—
//! están **totalmente ordenados**. Se modelan como `u64` y quien adapte el
//! almacén decide cómo mapea el suyo. Sin fechas: no hay reloj que leer ni texto
//! que interpretar, y dos deltas se ordenan sin ambigüedad.

use crate::capabilities as pushdown;
use crate::delta_compiler::Zset;
use crate::plan::{Comparador, Expr, Nodo, Valor};
use crate::schema::{Desajuste, esquema};
use std::collections::{BTreeMap, BTreeSet};

/// Los valores de las columnas de clave, en el orden declarado.
pub type Clave = Vec<Valor>;

/// Bajo qué se computó lo que hay en una clave: las mismas tres que
/// `cache::Entrada`, con la marca como ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identidades {
    pub bundle: String,
    pub topologia: Option<String>,
    pub marca: u64,
}

/// Lo que devuelve una lectura. **Nunca bloquea y nunca abre nada.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lectura {
    Presente {
        filas: Zset,
        marca: u64,
    },
    /// La clave no está. `upquery` es el plan que la repone: el de la vista,
    /// filtrado a esa clave. Quien lo ejecute llama a [`StateStore::rellenar`].
    Ausente {
        upquery: Nodo,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rechazo {
    /// Una columna de clave que el plan no produce.
    ClaveNoEsSalida { columna: String },
    /// El plan no cuadra.
    PlanNoCuadra(Box<Desajuste>),
    /// Una fila sin alguna columna de clave: no se sabe a qué clave pertenece.
    SinClave { columna: String },
    /// Una fila con otras columnas que las que el plan produce.
    FilaNoCuadra {
        esperadas: Vec<String>,
        tiene: Vec<String>,
    },
    /// Un relleno para una clave que nadie pidió.
    NoPedida { clave: Clave },
    /// Un relleno computado bajo otro bundle. Las filas pueden estar
    /// enmascaradas según una clasificación que ya no rige.
    ReglaDistinta { bajo: String },
    /// Un relleno computado con otra topología.
    CorrespondenciaDistinta { bajo: Option<String> },
}

impl Rechazo {
    pub fn como_texto(&self) -> String {
        match self {
            Rechazo::ClaveNoEsSalida { columna } => {
                format!("`{columna}` es columna de clave y el plan no la produce")
            }
            Rechazo::PlanNoCuadra(d) => format!("el plan no cuadra: {}", d.como_texto()),
            Rechazo::SinClave { columna } => {
                format!("una fila no trae `{columna}`: no se sabe a qué clave pertenece")
            }
            Rechazo::FilaNoCuadra { esperadas, tiene } => format!(
                "una fila trae [{}] y el plan produce [{}]",
                tiene.join(", "),
                esperadas.join(", ")
            ),
            Rechazo::NoPedida { clave } => format!(
                "relleno para {clave:?} que nadie pidió: un almacén que acepta lo que no pidió \
                 se puede envenenar"
            ),
            Rechazo::ReglaDistinta { bajo } => format!(
                "relleno computado bajo `{bajo}`: las filas pueden estar enmascaradas según una \
                 clasificación que ya no rige"
            ),
            Rechazo::CorrespondenciaDistinta { bajo } => format!(
                "relleno computado con la topología `{}`: las claves pueden apuntar a otras cosas",
                bajo.as_deref().unwrap_or("—")
            ),
        }
    }
}

/// Qué pasó con un delta, clave a clave. Se devuelve para que quien lo aplique
/// **vea** lo que se descartó: un descarte silencioso es la forma de fallo que
/// este almacén no tiene.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Aplicacion {
    pub aplicadas: usize,
    pub guardadas_en_vuelo: usize,
    pub descartadas_ausentes: usize,
    pub descartadas_viejas: usize,
}

/// Contadores para quien tenga que medir. El Cost Model está bloqueado por
/// medidas, y estas son las suyas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Estadisticas {
    pub aciertos: u64,
    pub fallos: u64,
    pub desalojos: u64,
    pub rellenos: u64,
}

struct Entrada {
    filas: Zset,
    marca: u64,
    ultimo_acceso: u64,
}

pub struct StateStore {
    plan: Nodo,
    clave: Vec<String>,
    columnas: BTreeSet<String>,
    bundle: String,
    topologia: Option<String>,
    capacidad: usize,
    presentes: BTreeMap<Clave, Entrada>,
    /// Claves con *upquery* en vuelo, y los deltas que llegaron mientras tanto.
    en_vuelo: BTreeMap<Clave, Vec<(u64, Zset)>>,
    /// Un contador lógico. **No un reloj**: el orden de los accesos es lo único
    /// que el LRU necesita, y un reloj lo haría depender de cuándo corre la
    /// prueba.
    tic: u64,
    stats: Estadisticas,
}

impl StateStore {
    /// Un almacén para una vista. La clave tiene que ser **salida del plan**:
    /// una clave que el plan no produce no se puede leer de una fila.
    pub fn nuevo(
        plan: Nodo,
        clave: Vec<String>,
        bundle: String,
        topologia: Option<String>,
        capacidad: usize,
    ) -> Result<StateStore, Rechazo> {
        let columnas: BTreeSet<String> = esquema(&plan)
            .map_err(|d| Rechazo::PlanNoCuadra(Box::new(d)))?
            .into_keys()
            .collect();
        for c in &clave {
            if !columnas.contains(c) {
                return Err(Rechazo::ClaveNoEsSalida { columna: c.clone() });
            }
        }
        Ok(StateStore {
            plan,
            clave,
            columnas,
            bundle,
            topologia,
            capacidad: capacidad.max(1),
            presentes: BTreeMap::new(),
            en_vuelo: BTreeMap::new(),
            tic: 0,
            stats: Estadisticas::default(),
        })
    }

    fn tic(&mut self) -> u64 {
        self.tic += 1;
        self.tic
    }

    /// La clave de una fila, o qué columna le falta.
    fn clave_de(&self, fila: &BTreeMap<String, Valor>) -> Result<Clave, Rechazo> {
        self.clave
            .iter()
            .map(|c| {
                fila.get(c)
                    .cloned()
                    .ok_or(Rechazo::SinClave { columna: c.clone() })
            })
            .collect()
    }

    fn cuadra(&self, fila: &BTreeMap<String, Valor>) -> Result<(), Rechazo> {
        let tiene: BTreeSet<&String> = fila.keys().collect();
        let esperadas: BTreeSet<&String> = self.columnas.iter().collect();
        if tiene != esperadas {
            return Err(Rechazo::FilaNoCuadra {
                esperadas: self.columnas.iter().cloned().collect(),
                tiene: fila.keys().cloned().collect(),
            });
        }
        Ok(())
    }

    /// **La *upquery*** de una clave: el plan de la vista, filtrado a ella. Es un
    /// plan como cualquier otro, y el Pushdown Planner lo baja hasta la hoja —
    /// que es lo que convierte el *miss* en una búsqueda por clave.
    pub fn upquery(&self, clave: &[Valor]) -> Nodo {
        let conyuntos: Vec<Expr> = self
            .clave
            .iter()
            .zip(clave)
            .map(|(c, v)| Expr::Compara {
                op: Comparador::Igual,
                izquierda: Box::new(Expr::campo(c)),
                derecha: Box::new(Expr::Literal(v.clone())),
            })
            .collect();
        Nodo::Filtra {
            entrada: Box::new(self.plan.clone()),
            predicado: if conyuntos.len() == 1 {
                conyuntos.into_iter().next().expect("uno")
            } else {
                Expr::Y(conyuntos)
            },
        }
    }

    /// **Leer.** Presente → las filas, y la clave se calienta. Ausente → la
    /// *upquery*, y la clave queda **en vuelo**: leerla otra vez no produce otra.
    pub fn leer(&mut self, clave: &[Valor]) -> Lectura {
        let clave: Clave = clave.iter().cloned().map(Valor::normalizado).collect();
        let ahora = self.tic();
        if let Some(e) = self.presentes.get_mut(&clave) {
            e.ultimo_acceso = ahora;
            self.stats.aciertos += 1;
            return Lectura::Presente {
                filas: e.filas.clone(),
                marca: e.marca,
            };
        }
        self.stats.fallos += 1;
        self.en_vuelo.entry(clave.clone()).or_default();
        Lectura::Ausente {
            upquery: self.upquery(&clave),
        }
    }

    /// Las *upqueries* pendientes, con su plan. Una por clave.
    pub fn pendientes(&self) -> Vec<(Clave, Nodo)> {
        self.en_vuelo
            .keys()
            .map(|k| (k.clone(), self.upquery(k)))
            .collect()
    }

    /// **Rellenar** una clave que se pidió, con lo que la fuente devolvió y bajo
    /// qué identidades. Los deltas que llegaron mientras tanto se aplican **solo
    /// si son más nuevos** que el relleno; los demás ya están dentro.
    pub fn rellenar(
        &mut self,
        clave: &[Valor],
        filas: Zset,
        identidades: Identidades,
    ) -> Result<(), Rechazo> {
        let clave: Clave = clave.iter().cloned().map(Valor::normalizado).collect();
        if identidades.bundle != self.bundle {
            return Err(Rechazo::ReglaDistinta {
                bajo: identidades.bundle,
            });
        }
        if identidades.topologia != self.topologia {
            return Err(Rechazo::CorrespondenciaDistinta {
                bajo: identidades.topologia,
            });
        }
        let Some(en_vuelo) = self.en_vuelo.remove(&clave) else {
            return Err(Rechazo::NoPedida { clave });
        };
        for (f, _) in filas.filas() {
            self.cuadra(f)?;
            if self.clave_de(f)? != clave {
                return Err(Rechazo::NoPedida {
                    clave: self.clave_de(f)?,
                });
            }
        }
        let mut entrada = Entrada {
            filas,
            marca: identidades.marca,
            ultimo_acceso: self.tic(),
        };
        for (marca, delta) in en_vuelo {
            if marca > entrada.marca {
                entrada.filas.sumar(&delta);
                entrada.marca = marca;
            }
        }
        self.presentes.insert(clave, entrada);
        self.stats.rellenos += 1;
        self.desalojar_hasta_capacidad();
        Ok(())
    }

    /// **Aplicar un delta de la salida de la vista**, con su marca. Clave a
    /// clave: presente y más nuevo → se aplica; en vuelo → se guarda; ausente →
    /// **se descarta**, porque la próxima lectura repone desde la fuente, que es
    /// la verdad. Se devuelve qué pasó con cada trozo.
    pub fn aplicar(&mut self, delta: &Zset, marca: u64) -> Result<Aplicacion, Rechazo> {
        let mut por_clave: BTreeMap<Clave, Zset> = BTreeMap::new();
        for (f, w) in delta.filas() {
            self.cuadra(f)?;
            por_clave
                .entry(self.clave_de(f)?)
                .or_default()
                .insertar(f.clone(), w);
        }
        let mut a = Aplicacion::default();
        for (clave, trozo) in por_clave {
            if let Some(e) = self.presentes.get_mut(&clave) {
                if marca > e.marca {
                    e.filas.sumar(&trozo);
                    e.marca = marca;
                    a.aplicadas += 1;
                } else {
                    a.descartadas_viejas += 1;
                }
            } else if let Some(cola) = self.en_vuelo.get_mut(&clave) {
                cola.push((marca, trozo));
                a.guardadas_en_vuelo += 1;
            } else {
                a.descartadas_ausentes += 1;
            }
        }
        Ok(a)
    }

    /// Desalojar una clave. Desde ahora sus deltas se descartan **sin que nadie
    /// tenga que acordarse de ella**: no hay lista de desalojadas, hay ausencia.
    pub fn desalojar(&mut self, clave: &[Valor]) -> bool {
        let clave: Clave = clave.iter().cloned().map(Valor::normalizado).collect();
        let habia = self.presentes.remove(&clave).is_some();
        if habia {
            self.stats.desalojos += 1;
        }
        habia
    }

    /// LRU: la menos leída se va. Sobre el contador lógico, y en caso de empate
    /// la menor clave, para que dos ejecuciones desalojen lo mismo.
    fn desalojar_hasta_capacidad(&mut self) {
        while self.presentes.len() > self.capacidad {
            let victima = self
                .presentes
                .iter()
                .min_by_key(|(k, e)| (e.ultimo_acceso, (*k).clone()))
                .map(|(k, _)| k.clone())
                .expect("hay más de cero");
            self.presentes.remove(&victima);
            self.stats.desalojos += 1;
        }
    }

    pub fn presente(&self, clave: &[Valor]) -> bool {
        let clave: Clave = clave.iter().cloned().map(Valor::normalizado).collect();
        self.presentes.contains_key(&clave)
    }

    pub fn claves(&self) -> Vec<Clave> {
        self.presentes.keys().cloned().collect()
    }

    pub fn estadisticas(&self) -> &Estadisticas {
        &self.stats
    }

    /// Lo que un reparto de la *upquery* le pediría a cada hoja. Es una
    /// comodidad para quien ejecute: la misma llamada al Pushdown Planner que
    /// haría a mano.
    pub fn repartir_upquery(
        &self,
        clave: &[Valor],
        caps: &BTreeMap<String, pushdown::Capacidades>,
    ) -> Result<pushdown::Reparto, pushdown::Rechazo> {
        pushdown::repartir(&self.upquery(clave), caps)
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{Capacidades, Recorrido};
    use crate::plan::Lectura as Hoja;
    use ore_core::types::parse_type;

    fn pedidos() -> Nodo {
        Nodo::Lee(Hoja {
            datasource: "lago".into(),
            objeto: "ventas.pedidos".into(),
            campos: [
                ("id".to_string(), parse_type("Integer").unwrap()),
                ("pais".to_string(), parse_type("String").unwrap()),
                ("total".to_string(), parse_type("Decimal").unwrap()),
            ]
            .into(),
        })
    }
    /// La vista: pedidos proyectados a `pais`, `total`. Clave: `pais`.
    fn vista() -> Nodo {
        Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [
                ("pais".to_string(), Expr::campo("pais")),
                ("total".to_string(), Expr::campo("total")),
            ]
            .into(),
        }
    }
    fn store(capacidad: usize) -> StateStore {
        StateStore::nuevo(
            vista(),
            vec!["pais".into()],
            "sha256:aaa".into(),
            None,
            capacidad,
        )
        .expect("se crea")
    }
    fn s(x: &str) -> Valor {
        Valor::Cadena(x.into())
    }
    fn fila(pais: &str, total: &str) -> BTreeMap<String, Valor> {
        [
            ("pais".to_string(), s(pais)),
            ("total".to_string(), Valor::Decimal(total.into())),
        ]
        .into()
    }
    fn filas(pais: &str, totales: &[&str]) -> Zset {
        Zset::de(totales.iter().map(|t| (fila(pais, t), 1)))
    }
    fn ident(marca: u64) -> Identidades {
        Identidades {
            bundle: "sha256:aaa".into(),
            topologia: None,
            marca,
        }
    }
    fn caps() -> BTreeMap<String, Capacidades> {
        [(
            "lago".to_string(),
            Capacidades {
                predicados: [Comparador::Igual].into(),
                recorrido: Recorrido::Prohibido,
                ..Default::default()
            },
        )]
        .into()
    }

    /// **EL CRITERIO, mitad primera.** Un *miss* produce **una** *upquery*, que
    /// es un plan que el Pushdown Planner acepta — y que baja la clave hasta la
    /// hoja, con un origen que **prohíbe** el recorrido completo. Y leer la misma
    /// clave ausente dos veces no produce dos.
    #[test]
    fn un_miss_produce_una_upquery_que_el_pushdown_planner_acepta() {
        let mut st = store(10);
        let Lectura::Ausente { upquery } = st.leer(&[s("ES")]) else {
            panic!("tenía que faltar");
        };
        // Es un plan, y el reparto lo baja a la hoja como búsqueda por clave.
        let r = pushdown::repartir(&upquery, &caps()).expect("el planner lo acepta");
        assert_eq!(r.peticiones.len(), 1);
        assert_eq!(r.peticiones[0].filtros.len(), 1, "{:?}", r.peticiones[0]);
        assert!(esquema(&upquery).is_ok());

        // Segunda lectura: sigue ausente, y sigue habiendo UNA pendiente.
        assert!(matches!(st.leer(&[s("ES")]), Lectura::Ausente { .. }));
        assert_eq!(st.pendientes().len(), 1);
        assert_eq!(st.estadisticas().fallos, 2);
        assert_eq!(
            st.repartir_upquery(&[s("ES")], &caps())
                .map(|r| r.peticiones.len()),
            Ok(1)
        );
    }

    /// El relleno completa la *upquery*: la clave pasa a presente, la siguiente
    /// lectura acierta, y la pendiente desaparece.
    #[test]
    fn un_relleno_completa_la_upquery() {
        let mut st = store(10);
        let _ = st.leer(&[s("ES")]);
        st.rellenar(&[s("ES")], filas("ES", &["10", "20"]), ident(5))
            .expect("se rellena");
        assert!(st.pendientes().is_empty());
        let Lectura::Presente { filas, marca } = st.leer(&[s("ES")]) else {
            panic!()
        };
        assert_eq!(filas.filas().count(), 2);
        assert_eq!(marca, 5);
        assert_eq!(st.estadisticas().aciertos, 1);
    }

    /// **P4.** Un relleno que nadie pidió se rechaza: un almacén que acepta lo
    /// que no pidió es un almacén que se puede envenenar.
    #[test]
    fn un_relleno_no_pedido_se_rechaza() {
        let mut st = store(10);
        assert_eq!(
            st.rellenar(&[s("ES")], filas("ES", &["10"]), ident(1)),
            Err(Rechazo::NoPedida {
                clave: vec![s("ES")]
            })
        );
        // Y un relleno pedido para `ES` que trae filas de `PT` tampoco entra.
        let _ = st.leer(&[s("ES")]);
        assert_eq!(
            st.rellenar(&[s("ES")], filas("PT", &["10"]), ident(1)),
            Err(Rechazo::NoPedida {
                clave: vec![s("PT")]
            })
        );
    }

    /// **La regla de E1, a granularidad de clave.** Un relleno computado bajo
    /// otro bundle no entra: sus filas pueden estar enmascaradas según una
    /// clasificación que ya no rige. Y con otra topología, tampoco.
    #[test]
    fn un_relleno_bajo_otro_bundle_o_topologia_se_rechaza() {
        let mut st = store(10);
        let _ = st.leer(&[s("ES")]);
        assert_eq!(
            st.rellenar(
                &[s("ES")],
                filas("ES", &["10"]),
                Identidades {
                    bundle: "sha256:bbb".into(),
                    topologia: None,
                    marca: 1
                }
            ),
            Err(Rechazo::ReglaDistinta {
                bajo: "sha256:bbb".into()
            })
        );
        assert_eq!(
            st.rellenar(
                &[s("ES")],
                filas("ES", &["10"]),
                Identidades {
                    bundle: "sha256:aaa".into(),
                    topologia: Some("sha256:t".into()),
                    marca: 1
                }
            ),
            Err(Rechazo::CorrespondenciaDistinta {
                bajo: Some("sha256:t".into())
            })
        );
        // Y la clave sigue en vuelo: el rechazo no la pierde.
        assert_eq!(st.pendientes().len(), 1);
    }

    /// **EL CRITERIO, mitad segunda.** Una clave desalojada deja de recibir
    /// deltas **sin que nadie tenga que acordarse de ella**: no hay lista de
    /// desalojadas, hay ausencia. Y la siguiente lectura repone desde la fuente.
    #[test]
    fn una_clave_desalojada_deja_de_recibir_deltas() {
        let mut st = store(10);
        let _ = st.leer(&[s("ES")]);
        st.rellenar(&[s("ES")], filas("ES", &["10"]), ident(1))
            .unwrap();
        assert!(st.desalojar(&[s("ES")]));
        assert!(!st.presente(&[s("ES")]));

        let a = st.aplicar(&filas("ES", &["99"]), 7).expect("se aplica");
        assert_eq!(a.descartadas_ausentes, 1, "{a:?}");
        assert_eq!(a.aplicadas, 0);
        assert!(!st.presente(&[s("ES")]), "el delta no la resucitó");

        // Y la próxima lectura es una upquery otra vez.
        assert!(matches!(st.leer(&[s("ES")]), Lectura::Ausente { .. }));
        assert_eq!(st.estadisticas().desalojos, 1);
    }

    /// **Marcas monótonas.** Un delta más viejo que el relleno ya está dentro y
    /// se descarta; uno más nuevo se aplica y avanza la marca.
    #[test]
    fn un_delta_mas_viejo_que_lo_que_hay_se_descarta() {
        let mut st = store(10);
        let _ = st.leer(&[s("ES")]);
        st.rellenar(&[s("ES")], filas("ES", &["10"]), ident(10))
            .unwrap();

        let viejo = st.aplicar(&filas("ES", &["20"]), 5).unwrap();
        assert_eq!(viejo.descartadas_viejas, 1, "{viejo:?}");
        let Lectura::Presente { filas: f, marca } = st.leer(&[s("ES")]) else {
            panic!()
        };
        assert_eq!(f.filas().count(), 1);
        assert_eq!(marca, 10);

        let nuevo = st.aplicar(&filas("ES", &["20"]), 11).unwrap();
        assert_eq!(nuevo.aplicadas, 1);
        let Lectura::Presente { filas: f, marca } = st.leer(&[s("ES")]) else {
            panic!()
        };
        assert_eq!(f.filas().count(), 2);
        assert_eq!(marca, 11);

        // Y una baja también es un delta: quitar el `10` lo quita.
        let baja = Zset::de([(fila("ES", "10"), -1)]);
        st.aplicar(&baja, 12).unwrap();
        let Lectura::Presente { filas: f, .. } = st.leer(&[s("ES")]) else {
            panic!()
        };
        assert_eq!(f.filas().count(), 1);
        assert_eq!(f.peso(&fila("ES", "20")), 1);
    }

    /// **La carrera entre relleno y actualización.** Un delta que llega mientras
    /// la *upquery* está en vuelo se guarda; al rellenar se aplica **solo si es
    /// más nuevo** que el relleno. Las dos mitades, porque la segunda es la que
    /// se equivoca: aplicar un delta que el relleno ya contenía lo contaría dos
    /// veces.
    #[test]
    fn un_delta_en_vuelo_se_aplica_solo_si_es_mas_nuevo_que_el_relleno() {
        // Más nuevo: se aplica.
        let mut st = store(10);
        let _ = st.leer(&[s("ES")]);
        let a = st.aplicar(&filas("ES", &["20"]), 7).unwrap();
        assert_eq!(a.guardadas_en_vuelo, 1, "{a:?}");
        st.rellenar(&[s("ES")], filas("ES", &["10"]), ident(5))
            .unwrap();
        let Lectura::Presente { filas: f, marca } = st.leer(&[s("ES")]) else {
            panic!()
        };
        assert_eq!(f.filas().count(), 2, "el 20 tenía que entrar");
        assert_eq!(marca, 7);

        // Más viejo que el relleno: ya estaba dentro, y no se cuenta dos veces.
        let mut st = store(10);
        let _ = st.leer(&[s("PT")]);
        st.aplicar(&filas("PT", &["20"]), 3).unwrap();
        st.rellenar(&[s("PT")], filas("PT", &["10", "20"]), ident(9))
            .unwrap();
        let Lectura::Presente { filas: f, marca } = st.leer(&[s("PT")]) else {
            panic!()
        };
        assert_eq!(f.peso(&fila("PT", "20")), 1, "se habría contado dos veces");
        assert_eq!(marca, 9);
    }

    /// **LRU sobre un contador lógico.** Con capacidad dos, rellenar tres
    /// desaloja la menos leída — y leer una la salva.
    #[test]
    fn la_capacidad_desaloja_a_la_menos_leida() {
        let mut st = store(2);
        for p in ["ES", "PT"] {
            let _ = st.leer(&[s(p)]);
            st.rellenar(&[s(p)], filas(p, &["1"]), ident(1)).unwrap();
        }
        // `ES` se lee: se calienta. `PT` queda como la menos reciente.
        let _ = st.leer(&[s("ES")]);
        let _ = st.leer(&[s("FR")]);
        st.rellenar(&[s("FR")], filas("FR", &["1"]), ident(1))
            .unwrap();
        assert!(st.presente(&[s("ES")]));
        assert!(st.presente(&[s("FR")]));
        assert!(!st.presente(&[s("PT")]), "{:?}", st.claves());
        assert_eq!(st.estadisticas().desalojos, 1);
    }

    /// La clave tiene que ser salida del plan, y una fila sin ella o con otras
    /// columnas no entra.
    #[test]
    fn lo_que_no_cuadra_con_el_plan_no_entra() {
        assert_eq!(
            StateStore::nuevo(vista(), vec!["id".into()], "b".into(), None, 1).err(),
            Some(Rechazo::ClaveNoEsSalida {
                columna: "id".into()
            })
        );
        let mut st = store(10);
        let sin_clave = Zset::de([(
            [("total".to_string(), Valor::Decimal("1".into()))].into(),
            1,
        )]);
        assert!(matches!(
            st.aplicar(&sin_clave, 1),
            Err(Rechazo::FilaNoCuadra { .. })
        ));
        let de_mas: BTreeMap<String, Valor> = [
            ("pais".to_string(), s("ES")),
            ("total".to_string(), Valor::Decimal("1".into())),
            ("extra".to_string(), s("x")),
        ]
        .into();
        assert!(matches!(
            st.aplicar(&Zset::de([(de_mas, 1)]), 1),
            Err(Rechazo::FilaNoCuadra { .. })
        ));
    }

    /// `0.10` y `0.1` en la clave son la misma clave, como en todo el motor.
    #[test]
    fn la_clave_se_normaliza() {
        let mut st =
            StateStore::nuevo(vista(), vec!["total".into()], "sha256:aaa".into(), None, 10)
                .unwrap();
        let _ = st.leer(&[Valor::Decimal("0.10".into())]);
        st.rellenar(
            &[Valor::Decimal("0.1".into())],
            filas("ES", &["0.10"]),
            ident(1),
        )
        .expect("es la misma clave");
        assert!(st.presente(&[Valor::Decimal("0.100".into())]));
    }
}
