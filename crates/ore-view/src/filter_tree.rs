//! **Filter Tree**: de N materializaciones a las candidatas de este plan, sin
//! mirar ninguna por dentro.
//!
//! El nombre es de Goldstein–Larson (SIGMOD'01) y es la aportación suya que se
//! cita menos, aunque es la que decide si esto sirve en producción: un índice
//! sobre las vistas para que buscar candidatas no sea recorrerlas todas. Calcite
//! reconoce por escrito que no lo tiene —*«la regla intenta emparejar todas las
//! vistas contra cada consulta. Planeamos implementar técnicas de filtrado más
//! refinadas»*— y con mil vistas eso son mil intentos por plan.
//!
//! # La firma ya existía
//!
//! Una materialización solo puede contestar a un plan si **todo lo que lee está
//! entre lo que el plan lee**: sus hojas `(datasource, objeto)` contenidas en las
//! de él. Es un test de subconjunto, y `Nodo::lecturas()` da los dos lados desde
//! M0. Un índice invertido —hoja → materializaciones que la tocan— lo convierte
//! en una búsqueda por clave.
//!
//! # Lo que este índice es, y lo que no
//!
//! Es **el primer nivel** del *filter tree* de Goldstein–Larson: el de las tablas
//! de origen. Los suyos siguen —columnas de salida, predicados— y aquí esos
//! filtros son los *checks* del View Matcher, que se hacen sobre las candidatas
//! y no en el índice. Meterlos en el índice es una optimización con su medida,
//! y hacerla sin medir sería inventarla.
//!
//! Y **no mira dentro de ninguna materialización**: solo su firma. Por eso la
//! prueba que importa no mide tiempo — **cuenta cuántas se cotejan**.

use crate::plan::{Lectura, Nodo};
use crate::schema::{Desajuste, esquema};
use std::collections::{BTreeMap, BTreeSet};

/// Una hoja, por su coordenada: de qué fuente y qué objeto.
pub type Hoja = (String, String);

/// Lo que un plan lee, como conjunto de hojas. Es **la firma**, y es todo lo
/// que el índice sabe de un plan o de una materialización.
pub fn firma(n: &Nodo) -> BTreeSet<Hoja> {
    n.lecturas()
        .into_iter()
        .map(|l| (l.datasource.clone(), l.objeto.clone()))
        .collect()
}

/// Con qué se fecha una copia.
///
/// Es el vocabulario de `changes.witness` de OOS —`none`, `snapshot`, `log`,
/// `field`— dicho aquí, y no se inventa otro. Los cuatro son **ordinales**: el
/// motor los compara, no los interpreta ni los convierte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Marca {
    /// Nada la fecha. Legal, y tiene precio: sin marca, lo copiado no puede
    /// decir hasta cuándo era cierto.
    #[default]
    Ninguna,
    /// La versión nativa de un formato de tabla: el *snapshot-id* de Iceberg,
    /// la versión de Delta.
    Instantanea,
    /// Una posición en un flujo de cambios: LSN, SCN, *offset*.
    Registro,
    /// Una columna de la propia tabla que ordena el avance.
    Campo(String),
}

impl Marca {
    pub fn como_texto(&self) -> String {
        match self {
            Marca::Ninguna => "sin marca".to_string(),
            Marca::Instantanea => "instantánea".to_string(),
            Marca::Registro => "registro".to_string(),
            Marca::Campo(c) => format!("campo `{c}`"),
        }
    }
}

/// **Hasta cuándo fue cierta**: la tercera cara de una copia.
///
/// Entra vacía y se queda vacía varias iteraciones, y aun así entra ahora: la
/// usa la frescura para **degradar en vez de mentir**, y añadirla después
/// significa convencer a quienes ya asumieron que no estaba.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Testigo {
    pub marca: Marca,
    /// Lo leído la última vez que se pobló. `None` es **nunca**, y no es un
    /// caso raro: hoy lo son todas.
    pub valor: Option<String>,
}

impl Testigo {
    /// Registrada y sin poblar. El caso de hoy.
    pub fn vacio() -> Self {
        Self::default()
    }

    /// Si la copia puede decir hasta cuándo fue cierta.
    pub fn fechada(&self) -> bool {
        self.valor.is_some()
    }

    pub fn como_texto(&self) -> String {
        match (&self.marca, &self.valor) {
            (Marca::Ninguna, None) => "sin poblar".to_string(),
            (m, None) => format!("{} · sin poblar", m.como_texto()),
            (m, Some(v)) => format!("{} · {v}", m.como_texto()),
        }
    }
}

/// Una vista materializada: **su definición, y dónde vive el resultado.**
///
/// La tabla es una [`Lectura`] a propósito: lo materializado es una hoja más,
/// que se lee por la puerta que ya existe. Es lo que E4 demostró para la caché —
/// *«servirse de la caché es cambiarle a una lectura la fuente y el objeto»*—
/// dicho una vez más.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Materializacion {
    pub nombre: String,
    /// El plan que la define. **Expandido**: una referencia sin resolver no tiene
    /// firma, y sin firma no hay dónde indexarla.
    pub plan: Nodo,
    /// La tabla donde vive el resultado. Sus `campos` tienen que ser **lo que el
    /// plan produce**, y se comprueba al registrar.
    pub tabla: Lectura,
    /// Hasta cuándo fue cierta. Vacío mientras nadie la pueble.
    pub testigo: Testigo,
}

impl Materializacion {
    /// Registrada y sin poblar, que es como nacen todas.
    pub fn nueva(nombre: impl Into<String>, plan: Nodo, tabla: Lectura) -> Self {
        Self {
            nombre: nombre.into(),
            plan,
            tabla,
            testigo: Testigo::vacio(),
        }
    }

    pub fn con_testigo(mut self, t: Testigo) -> Self {
        self.testigo = t;
        self
    }
}

/// Por qué una materialización no entra en el índice. Los dos son defectos del
/// **registro**, no de ninguna consulta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registro {
    /// El plan no cuadra, o todavía nombra una vista.
    ///
    /// El motivo va en caja porque un `Desajuste` lleva dos tipos dentro y el
    /// `Err` se pasaba del tamaño que clippy tolera: un `Result` grande se copia
    /// en cada retorno, también en el camino bueno.
    PlanNoCuadra {
        nombre: String,
        porque: Box<Desajuste>,
    },
    /// La tabla dice producir algo distinto de lo que el plan produce. Es el
    /// registro que **parece bueno y no lo es**: el índice la ofrecería, el View
    /// Matcher razonaría sobre el plan, y la tabla devolvería otras columnas.
    TablaNoCorresponde {
        nombre: String,
        plan_produce: Vec<String>,
        tabla_tiene: Vec<String>,
    },
    /// Dos materializaciones con el mismo nombre. La segunda no reemplaza a la
    /// primera en silencio.
    NombreRepetido { nombre: String },
}

impl Registro {
    pub fn como_texto(&self) -> String {
        match self {
            Registro::PlanNoCuadra { nombre, porque } => {
                format!("`{nombre}` no se registra: {}", porque.como_texto())
            }
            Registro::TablaNoCorresponde {
                nombre,
                plan_produce,
                tabla_tiene,
            } => format!(
                "`{nombre}` no se registra: su plan produce [{}] y su tabla tiene [{}]. El \
                 índice la ofrecería, el cotejo razonaría sobre el plan y la tabla devolvería \
                 otras columnas",
                plan_produce.join(", "),
                tabla_tiene.join(", ")
            ),
            Registro::NombreRepetido { nombre } => {
                format!("`{nombre}` ya está registrada, y la segunda no reemplaza a la primera")
            }
        }
    }
}

/// El índice.
#[derive(Debug, Clone, Default)]
pub struct FilterTree {
    materializaciones: BTreeMap<String, Materializacion>,
    firmas: BTreeMap<String, BTreeSet<Hoja>>,
    /// hoja → quién la toca. Es lo que convierte el subconjunto en una búsqueda.
    por_hoja: BTreeMap<Hoja, BTreeSet<String>>,
}

impl FilterTree {
    pub fn registrar(&mut self, m: Materializacion) -> Result<(), Registro> {
        if self.materializaciones.contains_key(&m.nombre) {
            return Err(Registro::NombreRepetido { nombre: m.nombre });
        }
        // El plan tiene que cuadrar: sin esquema no se puede comprobar la tabla,
        // y un plan sin expandir no tiene firma.
        let produce = esquema(&m.plan).map_err(|porque| Registro::PlanNoCuadra {
            nombre: m.nombre.clone(),
            porque: Box::new(porque),
        })?;
        if produce != m.tabla.campos {
            return Err(Registro::TablaNoCorresponde {
                nombre: m.nombre,
                plan_produce: produce.keys().cloned().collect(),
                tabla_tiene: m.tabla.campos.keys().cloned().collect(),
            });
        }
        let f = firma(&m.plan);
        for h in &f {
            self.por_hoja
                .entry(h.clone())
                .or_default()
                .insert(m.nombre.clone());
        }
        self.firmas.insert(m.nombre.clone(), f);
        self.materializaciones.insert(m.nombre.clone(), m);
        Ok(())
    }

    pub fn de(&self, nombre: &str) -> Option<&Materializacion> {
        self.materializaciones.get(nombre)
    }

    pub fn cuantas(&self) -> usize {
        self.materializaciones.len()
    }

    /// **Las candidatas de un plan**, de más a menos específica.
    ///
    /// Candidata es la que tiene **toda** su firma dentro de la del plan. Una que
    /// toque una hoja del plan y además otra que el plan no lee **no** lo es:
    /// leería algo que la consulta no pidió, y eso no es una reescritura, es
    /// otra consulta.
    ///
    /// El orden es por tamaño de firma descendente —la que cubre más hojas del
    /// plan se prueba antes— y por nombre para que dos ejecuciones den la misma
    /// lista. Cuando el View Matcher sepa contestar **parte** de un plan y unir
    /// el resto, el orden importará más; hoy es el que hace determinista la
    /// salida.
    /// **Las que leen todo lo que el plan lee, y quizá más.**
    ///
    /// Es la otra consulta, y es la que hace posible el *check 1* del View
    /// Matcher: una materialización con una junta **de más** solo vale si esa
    /// junta es sin pérdida, y decidirlo es del matcher — pero encontrarla es de
    /// aquí. Se calcula intersecando, hoja a hoja del plan, quién la toca; lo que
    /// queda toca todas. Orden: menos hojas de más primero, y nombre.
    pub fn candidatas_superconjunto(&self, plan: &Nodo) -> Vec<&Materializacion> {
        let del_plan = firma(plan);
        let mut hojas = del_plan.iter();
        let Some(primera) = hojas.next() else {
            return Vec::new();
        };
        let mut comunes: BTreeSet<&String> = match self.por_hoja.get(primera) {
            Some(s) => s.iter().collect(),
            None => return Vec::new(),
        };
        for h in hojas {
            match self.por_hoja.get(h) {
                Some(s) => comunes.retain(|n| s.contains(*n)),
                None => return Vec::new(),
            }
        }
        let mut out: Vec<&Materializacion> = comunes
            .into_iter()
            .map(|n| &self.materializaciones[n])
            .collect();
        out.sort_by(|a, b| {
            self.firmas[&a.nombre]
                .len()
                .cmp(&self.firmas[&b.nombre].len())
                .then_with(|| a.nombre.cmp(&b.nombre))
        });
        out
    }

    pub fn candidatas(&self, plan: &Nodo) -> Vec<&Materializacion> {
        let del_plan = firma(plan);
        // Todas las que tocan alguna hoja del plan…
        let tocan: BTreeSet<&String> = del_plan
            .iter()
            .filter_map(|h| self.por_hoja.get(h))
            .flatten()
            .collect();
        // …y de ellas, solo las que no tocan nada más.
        let mut out: Vec<&Materializacion> = tocan
            .into_iter()
            .filter(|n| self.firmas[*n].is_subset(&del_plan))
            .map(|n| &self.materializaciones[n])
            .collect();
        out.sort_by(|a, b| {
            self.firmas[&b.nombre]
                .len()
                .cmp(&self.firmas[&a.nombre].len())
                .then_with(|| a.nombre.cmp(&b.nombre))
        });
        out
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Comparador, Expr, Junta, Valor};
    use ore_core::types::parse_type;

    fn hoja(ds: &str, objeto: &str, campos: &[&str]) -> Nodo {
        Nodo::Lee(lectura(ds, objeto, campos))
    }

    fn lectura(ds: &str, objeto: &str, campos: &[&str]) -> Lectura {
        Lectura {
            datasource: ds.into(),
            objeto: objeto.into(),
            campos: campos
                .iter()
                .map(|c| ((*c).to_string(), parse_type("String").unwrap()))
                .collect(),
        }
    }

    fn pedidos() -> Nodo {
        hoja("lago", "ventas.pedidos", &["id", "pais", "total"])
    }

    fn lineas() -> Nodo {
        hoja("sap", "ventas.lineas", &["id_pedido", "sku"])
    }

    fn eq(campo: &str, v: &str) -> Expr {
        Expr::Compara {
            op: Comparador::Igual,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(Valor::Cadena(v.into()))),
        }
    }

    /// Una materialización cuyo plan es `plan`, con una tabla que produce lo
    /// mismo — que es la única forma de que se registre.
    fn materializacion(nombre: &str, plan: Nodo) -> Materializacion {
        let produce = esquema(&plan).expect("cuadra");
        Materializacion::nueva(
            nombre,
            plan,
            Lectura {
                datasource: "lago".into(),
                objeto: format!("cache.{nombre}"),
                campos: produce,
            },
        )
    }

    /// **EL CRITERIO DE M5.0.** Con mil materializaciones y un plan de dos hojas,
    /// se cotejan solo las que tocan esas dos hojas — y se **cuenta** cuántas, no
    /// cuánto tarda.
    #[test]
    fn con_mil_materializaciones_solo_se_cotejan_las_que_tocan_las_hojas_del_plan() {
        let mut ft = FilterTree::default();
        // 996 que leen tablas que este plan no lee.
        for i in 0..996 {
            ft.registrar(materializacion(
                &format!("otra_{i:03}"),
                hoja("lago", &format!("otras.tabla_{i}"), &["x"]),
            ))
            .expect("se registra");
        }
        // Y cuatro que sí tocan sus hojas, de distintas maneras.
        ft.registrar(materializacion("solo_pedidos", pedidos()))
            .unwrap();
        ft.registrar(materializacion(
            "pedidos_es",
            Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: eq("pais", "ES"),
            },
        ))
        .unwrap();
        ft.registrar(materializacion("solo_lineas", lineas()))
            .unwrap();
        ft.registrar(materializacion(
            "pedidos_con_lineas",
            Nodo::Une {
                izquierda: Box::new(pedidos()),
                derecha: Box::new(lineas()),
                tipo: Junta::Interna,
                sobre: vec![("id".into(), "id_pedido".into())],
            },
        ))
        .unwrap();
        assert_eq!(ft.cuantas(), 1000);

        let plan = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(lineas()),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        let c = ft.candidatas(&plan);
        let nombres: Vec<&str> = c.iter().map(|m| m.nombre.as_str()).collect();
        assert_eq!(c.len(), 4, "{nombres:?}");
        // La que cubre las dos hojas va primero; las de una hoja, por nombre.
        assert_eq!(
            nombres,
            [
                "pedidos_con_lineas",
                "pedidos_es",
                "solo_lineas",
                "solo_pedidos"
            ]
        );
    }

    /// Una materialización que toca una hoja del plan **y otra que el plan no
    /// lee** no es candidata: leería algo que la consulta no pidió, y eso no es
    /// una reescritura, es otra consulta.
    #[test]
    fn una_materializacion_con_una_hoja_de_mas_no_es_candidata() {
        let mut ft = FilterTree::default();
        ft.registrar(materializacion(
            "pedidos_con_lineas",
            Nodo::Une {
                izquierda: Box::new(pedidos()),
                derecha: Box::new(lineas()),
                tipo: Junta::Interna,
                sobre: vec![("id".into(), "id_pedido".into())],
            },
        ))
        .unwrap();
        ft.registrar(materializacion("solo_pedidos", pedidos()))
            .unwrap();

        // El plan solo lee pedidos: la junta con líneas toca una hoja de más.
        let c = ft.candidatas(&pedidos());
        let nombres: Vec<&str> = c.iter().map(|m| m.nombre.as_str()).collect();
        assert_eq!(nombres, ["solo_pedidos"]);
    }

    /// La firma es **coarse a propósito**: dos planes distintos sobre las mismas
    /// hojas tienen la misma firma. Distinguirlos es del View Matcher; el índice
    /// solo quita lo que seguro no sirve.
    #[test]
    fn la_firma_no_distingue_dos_planes_sobre_las_mismas_hojas() {
        let a = pedidos();
        let b = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: eq("pais", "PT"),
        };
        assert_eq!(firma(&a), firma(&b));
        assert_eq!(
            firma(&a),
            [("lago".to_string(), "ventas.pedidos".to_string())].into()
        );
    }

    /// Y por eso una materialización que **no** sirve por su predicado sigue
    /// siendo candidata aquí: el índice no mira dentro, y decir que sí sirve no
    /// es su trabajo.
    #[test]
    fn el_indice_no_mira_dentro() {
        let mut ft = FilterTree::default();
        ft.registrar(materializacion(
            "pedidos_pt",
            Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: eq("pais", "PT"),
            },
        ))
        .unwrap();
        let plan_es = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: eq("pais", "ES"),
        };
        assert_eq!(
            ft.candidatas(&plan_es).len(),
            1,
            "es candidata aunque no sirva"
        );
    }

    /// Un plan sin expandir no tiene firma, así que no se registra — con el
    /// mismo motivo que ya dan el tipador y el linaje.
    #[test]
    fn una_materializacion_sin_expandir_no_se_registra() {
        let mut ft = FilterTree::default();
        let m = Materializacion::nueva(
            "rota",
            Nodo::Referencia("otra".into()),
            lectura("lago", "cache.rota", &[]),
        );
        assert_eq!(
            ft.registrar(m),
            Err(Registro::PlanNoCuadra {
                nombre: "rota".into(),
                porque: Box::new(Desajuste::SinExpandir {
                    vista: "otra".into()
                })
            })
        );
        assert_eq!(ft.cuantas(), 0);
    }

    /// **El registro que parece bueno y no lo es.** La tabla dice producir otras
    /// columnas que el plan. Si entrara, el índice la ofrecería, el View Matcher
    /// razonaría sobre el plan y la tabla devolvería otra cosa.
    #[test]
    fn una_tabla_que_no_produce_lo_que_su_plan_produce_no_se_registra() {
        let mut ft = FilterTree::default();
        // Le falta `total`, y trae una que el plan no da.
        let m = Materializacion::nueva(
            "desfasada",
            pedidos(),
            lectura("lago", "cache.desfasada", &["id", "pais", "descuento"]),
        );
        let Err(Registro::TablaNoCorresponde {
            plan_produce,
            tabla_tiene,
            ..
        }) = ft.registrar(m)
        else {
            panic!("tenía que rechazarla");
        };
        assert_eq!(plan_produce, ["id", "pais", "total"]);
        assert_eq!(tabla_tiene, ["descuento", "id", "pais"]);
    }

    /// Dos con el mismo nombre: la segunda no reemplaza a la primera en
    /// silencio.
    #[test]
    fn un_nombre_repetido_no_reemplaza_en_silencio() {
        let mut ft = FilterTree::default();
        ft.registrar(materializacion("p", pedidos())).unwrap();
        assert_eq!(
            ft.registrar(materializacion("p", lineas())),
            Err(Registro::NombreRepetido { nombre: "p".into() })
        );
        // Y la primera sigue siendo la que está.
        assert_eq!(ft.de("p").unwrap().plan, pedidos());
    }

    /// Sin materializaciones no hay candidatas, y un plan cuyas hojas nadie toca
    /// tampoco: las dos son listas vacías, no errores.
    #[test]
    fn sin_nada_que_ofrecer_la_lista_esta_vacia() {
        let ft = FilterTree::default();
        assert!(ft.candidatas(&pedidos()).is_empty());

        let mut ft = FilterTree::default();
        ft.registrar(materializacion("l", lineas())).unwrap();
        assert!(ft.candidatas(&pedidos()).is_empty());
    }

    /// **La otra consulta.** Las que leen todo lo que el plan lee y quizá más:
    /// son las candidatas del check 1, donde una junta de más puede ser sin
    /// pérdida. Las de menos hojas de más van primero.
    #[test]
    fn el_superconjunto_encuentra_las_que_leen_de_mas() {
        let mut ft = FilterTree::default();
        let junta = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(lineas()),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        ft.registrar(materializacion("solo_pedidos", pedidos()))
            .unwrap();
        ft.registrar(materializacion("con_lineas", junta)).unwrap();
        ft.registrar(materializacion("solo_lineas", lineas()))
            .unwrap();

        // El plan solo lee pedidos: las dos que lo leen son candidatas, la que
        // no lo toca no. Y la exacta va antes que la que trae una hoja de más.
        let n: Vec<&str> = ft
            .candidatas_superconjunto(&pedidos())
            .iter()
            .map(|m| m.nombre.as_str())
            .collect();
        assert_eq!(n, ["solo_pedidos", "con_lineas"]);

        // Y un plan que lee una hoja que nadie toca no tiene superconjunto.
        let otra = hoja("lago", "ventas.otra", &["x"]);
        assert!(ft.candidatas_superconjunto(&otra).is_empty());
    }

    /// El orden es determinista: dos ejecuciones dan la misma lista, y las de la
    /// misma firma salen por nombre.
    #[test]
    fn el_orden_de_las_candidatas_es_estable() {
        let mut ft = FilterTree::default();
        for n in ["zeta", "alfa", "mu"] {
            ft.registrar(materializacion(
                n,
                Nodo::Filtra {
                    entrada: Box::new(pedidos()),
                    predicado: eq("pais", n),
                },
            ))
            .unwrap();
        }
        let nombres: Vec<String> = ft
            .candidatas(&pedidos())
            .iter()
            .map(|m| m.nombre.clone())
            .collect();
        assert_eq!(nombres, ["alfa", "mu", "zeta"]);
    }
}
