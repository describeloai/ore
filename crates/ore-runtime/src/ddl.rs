//! **Los cambios del catálogo, planificados** (0081 §7.5.2, C2b): el DDL de ORE ([`crate::sentencia::Ddl`]) con los
//! nombres resueltos en la base y el schema de la sesión, las consultas de dentro planificadas por DataFusion y las
//! columnas de un dataset vacío tipadas por DataFusion (con el planificador de tipos de ORE). Quién los cumple:
//! los del lago (base, schema, dataset, vista), el catálogo de ORE por `/v1`; los de otros productos (la foreign
//! database y su origen, la colección de medios, la copia de la vista materializada), su producto. El contrato fija
//! su forma y su plan.
//!
//! [`fmt::Display`] los dice de una forma canónica: es lo que enseña la vara y lo que queda en la observación.

pub use crate::sentencia::{Clase, ColumnaDeVista, DeOrigen, Retirable};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::common::ResolvedTableReference;
use datafusion::logical_expr::LogicalPlan;
use std::fmt;

/// Qué se retira.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Objetivo {
    Base(String),
    Schema { base: String, schema: String },
    Objeto(ResolvedTableReference),
}

impl fmt::Display for Objetivo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Objetivo::Base(b) => f.write_str(b),
            Objetivo::Schema { base, schema } => write!(f, "{base}.{schema}"),
            Objetivo::Objeto(n) => write!(f, "{n}"),
        }
    }
}

/// **Un cambio del catálogo, planificado.**
#[derive(Debug, Clone)]
pub enum CambioDeCatalogo {
    CrearBase {
        base: String,
        clase: Clase,
        si_no_existe: bool,
        origen: Option<DeOrigen>,
    },
    CrearSchema {
        base: String,
        schema: String,
        si_no_existe: bool,
    },
    /// Un dataset vacío: su esquema, tipado por DataFusion; `not null` es una columna no nulable.
    CrearDatasetVacio {
        nombre: ResolvedTableReference,
        si_no_existe: bool,
        esquema: SchemaRef,
    },
    CrearVista {
        nombre: ResolvedTableReference,
        reemplazar: bool,
        si_no_existe: bool,
        materializada: bool,
        columnas: Vec<ColumnaDeVista>,
        comentario: Option<String>,
        evolucion: bool,
        /// La consulta tal como se guarda (`dialect: datafusion`).
        sql: String,
        consulta: LogicalPlan,
    },
    CrearColeccion {
        nombre: ResolvedTableReference,
        reemplazar: bool,
        si_no_existe: bool,
        medio: String,
        formatos: Vec<String>,
        comentario: Option<String>,
        consulta: Option<(String, LogicalPlan)>,
    },
    Retirar {
        que: Retirable,
        objetivo: Objetivo,
        si_existe: bool,
        cascada: bool,
    },
}

fn columnas_de(plan: &LogicalPlan) -> String {
    plan.schema()
        .fields()
        .iter()
        .map(|c| c.name().as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for CambioDeCatalogo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let crear = |f: &mut fmt::Formatter<'_>, reemplazar: bool| {
            f.write_str(if reemplazar {
                "create or replace "
            } else {
                "create "
            })
        };
        let si_no = |f: &mut fmt::Formatter<'_>, si: bool| {
            if si {
                f.write_str("if not exists ")
            } else {
                Ok(())
            }
        };
        match self {
            CambioDeCatalogo::CrearBase {
                base,
                clase,
                si_no_existe,
                origen,
            } => {
                crear(f, false)?;
                f.write_str(match clase {
                    Clase::Standard => "standard database ",
                    Clase::Foreign => "foreign database ",
                })?;
                si_no(f, *si_no_existe)?;
                f.write_str(base)?;
                if let Some(o) = origen {
                    write!(
                        f,
                        " from origin {} include ({})",
                        o.origen,
                        o.incluye.join(", ")
                    )?;
                }
                Ok(())
            }
            CambioDeCatalogo::CrearSchema {
                base,
                schema,
                si_no_existe,
            } => {
                crear(f, false)?;
                f.write_str("schema ")?;
                si_no(f, *si_no_existe)?;
                write!(f, "{base}.{schema}")
            }
            CambioDeCatalogo::CrearDatasetVacio {
                nombre,
                si_no_existe,
                esquema,
            } => {
                crear(f, false)?;
                f.write_str("dataset ")?;
                si_no(f, *si_no_existe)?;
                let columnas: Vec<String> = esquema
                    .fields()
                    .iter()
                    .map(|c| {
                        let nulo = if c.is_nullable() { "" } else { " not null" };
                        format!("{} {}{nulo}", c.name(), c.data_type())
                    })
                    .collect();
                write!(f, "{nombre} ({})", columnas.join(", "))
            }
            CambioDeCatalogo::CrearVista {
                nombre,
                reemplazar,
                si_no_existe,
                materializada,
                columnas,
                comentario,
                evolucion,
                consulta,
                ..
            } => {
                crear(f, *reemplazar)?;
                if *materializada {
                    f.write_str("materialized ")?;
                }
                f.write_str("view ")?;
                si_no(f, *si_no_existe)?;
                write!(f, "{nombre}")?;
                if !columnas.is_empty() {
                    let c: Vec<String> = columnas
                        .iter()
                        .map(|c| match &c.comentario {
                            Some(t) => format!("{} comment '{t}'", c.nombre),
                            None => c.nombre.to_string(),
                        })
                        .collect();
                    write!(f, " ({})", c.join(", "))?;
                }
                if let Some(t) = comentario {
                    write!(f, " comment '{t}'")?;
                }
                if *evolucion {
                    f.write_str(" with schema evolution")?;
                }
                write!(f, " as a query of ({})", columnas_de(consulta))
            }
            CambioDeCatalogo::CrearColeccion {
                nombre,
                reemplazar,
                si_no_existe,
                medio,
                formatos,
                comentario,
                consulta,
            } => {
                crear(f, *reemplazar)?;
                f.write_str("media collection ")?;
                si_no(f, *si_no_existe)?;
                write!(
                    f,
                    "{nombre} media {medio} formats ({})",
                    formatos.join(", ")
                )?;
                if let Some(t) = comentario {
                    write!(f, " comment '{t}'")?;
                }
                if let Some((_, plan)) = consulta {
                    write!(f, " as a query of ({})", columnas_de(plan))?;
                }
                Ok(())
            }
            CambioDeCatalogo::Retirar {
                que,
                objetivo,
                si_existe,
                cascada,
            } => {
                write!(f, "drop {que} ")?;
                if *si_existe {
                    f.write_str("if exists ")?;
                }
                write!(f, "{objetivo}")?;
                if *cascada {
                    f.write_str(" cascade")?;
                }
                Ok(())
            }
        }
    }
}
