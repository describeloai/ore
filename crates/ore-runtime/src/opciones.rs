//! **La configuración como contrato** (0081 §7.4, propiedad 5).
//!
//! - Las opciones de ORE son una `ConfigExtension` con el prefijo `ore` ([`OreOpciones`]).
//! - Lo que se aparta de DataFusion está en [`DESVIOS`], cada valor con su porqué, y una prueba comprueba
//!   que cada clave sigue existiendo: si una actualización la renombra, se sabe aquí.
//! - La configuración es una **función pura** del papel, los límites y las opciones ([`config_de`]). Una
//!   consulta distribuida la tiene **igual** en los tres procesos. Así lo exige Ballista, que en el scheduler
//!   y el ejecutor pierde **en silencio** las claves que no tiene registradas (0081 S0 C2): `ore.*` y
//!   `ballista.*` van registradas en todos los papeles.
//! - Lo que Ballista restringe de DataFusion ([`DESVIOS_DISTRIBUIDOS`]) sólo se aplica a lo distribuido: una
//!   consulta local conserva el hash join y los filtros dinámicos.
//! - Una clave pasa siempre por `options_mut().set()`, que devuelve error. Nunca por `set_str`, que hace
//!   pánico con una clave desconocida.

use crate::fallo::Fallo;
use crate::limites::Limites;
use ballista_core::config::BallistaConfig;
use datafusion::common::config::ConfigExtension;
use datafusion::common::extensions_options;
use datafusion::prelude::SessionConfig;

extensions_options! {
    /// Las opciones de ORE (`ore.*`).
    pub struct OreOpciones {
        /// La parte de la memoria del proceso que no se da a la raíz de memoria: DataFusion sólo cuenta a los
        /// consumidores grandes (ni el escaneo ni lo intermedio), y pide al menos un 10 % de holgura.
        pub holgura_memoria: f64, default = 0.15
        /// El directorio de derrame. Sin él, el temporal del sistema.
        pub derrame_dir: Option<String>, default = None
        /// El tope de derrame en bytes. Sin él, el 90 % del disco libre del directorio, si se puede medir.
        pub derrame_tope: Option<u64>, default = None
        /// El scheduler de Ballista (`df://host:puerto`). Con él, las consultas de un proceso local se
        /// distribuyen: distribuir es una opción, no un papel (0081 D-S7).
        pub scheduler: Option<String>, default = None
    }
}

impl ConfigExtension for OreOpciones {
    const PREFIX: &'static str = "ore";
}

/// La mayor holgura que tiene sentido: con más, la raíz se queda sin memoria que repartir.
pub const HOLGURA_MAXIMA: f64 = 0.9;

/// Los papeles de un proceso del ORE Runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Papel {
    /// Una consulta en un solo proceso.
    Local,
    /// El scheduler de Ballista.
    Scheduler,
    /// Un ejecutor de Ballista.
    Ejecutor,
    /// Las pruebas: planes estables, sin depender de la máquina.
    Prueba,
}

/// Las particiones del papel [`Papel::Prueba`]: fijas, para que un plan no dependa de la máquina que lo
/// prueba. Es el valor con que DataFusion corre su propio corpus de `sqllogictest`.
pub const PARTICIONES_DE_PRUEBA: usize = 4;

/// Un valor que se aparta de DataFusion, y por qué.
#[derive(Debug, Clone, Copy)]
pub struct Desvio {
    pub clave: &'static str,
    pub valor: &'static str,
    pub porque: &'static str,
}

/// **Lo que el ORE Runtime cambia de DataFusion.** `target_partitions` no está aquí porque no es un valor
/// fijo: sale de los límites ([`config_de`]).
pub const DESVIOS: &[Desvio] = &[Desvio {
    clave: "datafusion.execution.spill_compression",
    valor: "lz4_frame",
    porque: "derramar sin comprimir llena el disco del pod; lz4 cuesta poca CPU (0081 §6.6)",
}];

/// **Lo que Ballista restringe de DataFusion** para distribuir, cada valor con el porqué que da el upstream
/// (`ballista_restricted_configuration`, Ballista 55.0.0). Una prueba compara su **efecto** con el de
/// `upgrade_for_ballista`: si una versión de Ballista añade o cambia una restricción, se sabe aquí. Es el
/// efecto y no la lista porque una clave puede mandar sobre otras: `enable_dynamic_filter_pushdown` apaga
/// también los filtros dinámicos de join, agregado y top-k.
pub const DESVIOS_DISTRIBUIDOS: &[Desvio] = &[
    Desvio {
        clave: "datafusion.optimizer.enable_round_robin_repartition",
        valor: "false",
        porque: "Ballista no lo soporta bien, y su scheduler lo impone de todos modos",
    },
    Desvio {
        clave: "datafusion.execution.parquet.schema_force_view_types",
        valor: "false",
        porque: "con `Utf8View`, el IPC de Arrow genera tramas demasiado grandes para Flight",
    },
    Desvio {
        clave: "datafusion.sql_parser.map_string_types_to_utf8view",
        valor: "false",
        porque: "la misma causa: nada de `Utf8View` cruzando entre procesos",
    },
    Desvio {
        clave: "datafusion.optimizer.hash_join_single_partition_threshold",
        valor: "134217728",
        porque: "el umbral de difusión de Ballista (128 MiB), para que DataFusion y la AQE elijan igual",
    },
    Desvio {
        clave: "datafusion.optimizer.hash_join_single_partition_threshold_rows",
        valor: "1000000",
        porque: "el umbral de difusión de Ballista en filas, por la misma razón",
    },
    Desvio {
        clave: "datafusion.optimizer.prefer_hash_join",
        valor: "false",
        porque: "el hash join no derrama y cada tarea guarda entero el lado que construye; sort-merge sí \
                 derrama (Ballista #1648)",
    },
    Desvio {
        clave: "datafusion.optimizer.enable_physical_uncorrelated_scalar_subquery",
        valor: "false",
        porque: "su resultado compartido no cruza etapas ni se deserializa fuera de su nodo; reescrita como \
                 join, se distribuye (Ballista #1909)",
    },
    Desvio {
        clave: "datafusion.optimizer.enable_dynamic_filter_pushdown",
        valor: "false",
        porque: "un filtro dinámico producido en otra etapa nunca llega y el escaneo espera para siempre \
                 (Ballista #1375)",
    },
];

/// Si una consulta de este papel es distribuida: lo es en el scheduler y el ejecutor, y en un proceso local
/// con `ore.scheduler`.
pub fn distribuida(papel: Papel, opciones: &OreOpciones) -> bool {
    match papel {
        Papel::Scheduler | Papel::Ejecutor => true,
        Papel::Local | Papel::Prueba => opciones.scheduler.is_some(),
    }
}

/// **La configuración de un proceso**: función pura del papel, los límites y las opciones.
pub fn config_de(
    papel: Papel,
    limites: &Limites,
    opciones: &OreOpciones,
) -> Result<SessionConfig, Fallo> {
    if !(0.0..=HOLGURA_MAXIMA).contains(&opciones.holgura_memoria) {
        return Err(Fallo::Config(format!(
            "`ore.holgura_memoria` = {} no está entre 0 y {HOLGURA_MAXIMA}",
            opciones.holgura_memoria
        )));
    }
    let mut config = SessionConfig::new()
        .with_option_extension(opciones.clone())
        .with_option_extension(BallistaConfig::default());
    for d in DESVIOS {
        fijar(&mut config, d.clave, d.valor)?;
    }
    if distribuida(papel, opciones) {
        for d in DESVIOS_DISTRIBUIDOS {
            fijar(&mut config, d.clave, d.valor)?;
        }
    }
    let particiones = match papel {
        Papel::Prueba => PARTICIONES_DE_PRUEBA,
        Papel::Local | Papel::Scheduler | Papel::Ejecutor => limites.cpu,
    };
    Ok(config.with_target_partitions(particiones))
}

/// Fija una clave. Una clave que no existe es un error que la nombra, nunca un pánico.
pub fn fijar(config: &mut SessionConfig, clave: &str, valor: &str) -> Result<(), Fallo> {
    config
        .options_mut()
        .set(clave, valor)
        .map_err(|e| Fallo::Config(format!("`{clave}` = `{valor}`: {e}")))
}

/// Las opciones de ORE de una configuración.
pub fn de(config: &SessionConfig) -> Option<&OreOpciones> {
    config.options().extensions.get::<OreOpciones>()
}

/// **La foto de una configuración**: cada clave con su valor, y cada una con el nombre con que se fija.
///
/// DataFusion 55 lista las claves de una `ConfigExtension` **sin su prefijo** (`holgura_memoria`, no
/// `ore.holgura_memoria`), aunque para fijarlas haya que escribirlo (medido en S2·3). Una foto así no se
/// puede volver a aplicar ni distinguir de otra extensión: aquí van las de DataFusion tal cual, las de ORE
/// con `ore.` delante y las de Ballista, que ya lo llevan, en orden (las guarda en un `HashMap`).
pub fn entradas(config: &SessionConfig) -> Vec<(String, Option<String>)> {
    use datafusion::common::config::ExtensionOptions;
    let mut foto: Vec<(String, Option<String>)> = config
        .options()
        .entries()
        .into_iter()
        .filter(|e| e.key.starts_with("datafusion."))
        .map(|e| (e.key, e.value))
        .collect();
    if let Some(o) = de(config) {
        foto.extend(
            o.entries()
                .into_iter()
                .map(|e| (format!("{}.{}", OreOpciones::PREFIX, e.key), e.value)),
        );
    }
    if let Some(b) = config.options().extensions.get::<BallistaConfig>() {
        let mut ballista: Vec<_> = b.entries().into_iter().map(|e| (e.key, e.value)).collect();
        ballista.sort();
        foto.extend(ballista);
    }
    foto
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::limites::Fuente;

    fn limites(cpu: usize) -> Limites {
        Limites {
            memoria: 4 << 30,
            memoria_de: Fuente::Host,
            cpu,
            cpu_de: Fuente::Host,
            disco: None,
        }
    }

    #[test]
    fn cada_desvio_existe_en_datafusion() {
        let mut c = SessionConfig::new();
        for d in DESVIOS {
            fijar(&mut c, d.clave, d.valor).unwrap_or_else(|e| {
                panic!("el desvío `{}` ya no vale en DataFusion: {e}", d.clave)
            });
            assert!(!d.porque.is_empty(), "`{}` sin porqué", d.clave);
            let leido = c
                .options()
                .entries()
                .into_iter()
                .find(|e| e.key == d.clave)
                .and_then(|e| e.value)
                .unwrap_or_default();
            assert_eq!(leido, d.valor, "`{}`", d.clave);
        }
    }

    #[test]
    fn una_clave_desconocida_es_un_error_y_no_un_panico() {
        let mut c = config_de(Papel::Local, &limites(2), &OreOpciones::default()).unwrap();
        let e = fijar(&mut c, "datafusion.execution.no_existe", "1").unwrap_err();
        assert!(matches!(e, Fallo::Config(_)), "{e}");
        assert!(e.to_string().contains("no_existe"), "{e}");
        let e = fijar(&mut c, "ore.no_existe", "1").unwrap_err();
        assert!(e.to_string().contains("ore.no_existe"), "{e}");
        assert!(fijar(&mut c, "sin_espacio", "1").is_err());
    }

    #[test]
    fn las_opciones_de_ore_son_claves_de_la_sesion() {
        let mut c = config_de(Papel::Local, &limites(2), &OreOpciones::default()).unwrap();
        fijar(&mut c, "ore.holgura_memoria", "0.2").unwrap();
        fijar(&mut c, "ore.derrame_dir", "/trabajo/derrame").unwrap();
        let o = de(&c).unwrap();
        assert_eq!(o.holgura_memoria, 0.2);
        assert_eq!(o.derrame_dir.as_deref(), Some("/trabajo/derrame"));
        let foto = entradas(&c);
        let valor = |k: &str| {
            foto.iter()
                .find(|(c, _)| c == k)
                .and_then(|(_, v)| v.clone())
        };
        assert_eq!(valor("ore.holgura_memoria").as_deref(), Some("0.2"));
        assert_eq!(
            valor("ore.derrame_dir").as_deref(),
            Some("/trabajo/derrame")
        );
        // Y la foto se puede volver a aplicar, clave a clave, en una configuración nueva.
        let mut otra = config_de(Papel::Local, &limites(2), &OreOpciones::default()).unwrap();
        for (k, v) in foto.iter().filter(|(_, v)| v.is_some()) {
            fijar(&mut otra, k, v.as_deref().unwrap()).unwrap();
        }
        assert_eq!(entradas(&otra), foto);
    }

    /// Las claves en que dos configuraciones difieren.
    fn diferencias(a: &SessionConfig, b: &SessionConfig) -> Vec<String> {
        let (a, b) = (entradas(a), entradas(b));
        assert_eq!(a.len(), b.len());
        a.into_iter()
            .zip(b)
            .filter(|(x, y)| x != y)
            .map(|(x, _)| x.0)
            .collect()
    }

    #[test]
    fn los_papeles_difieren_donde_deben_y_solo_ahi() {
        let l = limites(6);
        let local = OreOpciones::default();
        let cliente = OreOpciones {
            scheduler: Some("df://scheduler:50050".into()),
            ..OreOpciones::default()
        };
        let c = |p, o: &OreOpciones| config_de(p, &l, o).unwrap();
        let (sola, distribuida, scheduler, ejecutor, prueba) = (
            c(Papel::Local, &local),
            c(Papel::Local, &cliente),
            c(Papel::Scheduler, &local),
            c(Papel::Ejecutor, &local),
            c(Papel::Prueba, &local),
        );
        assert_eq!(sola.target_partitions(), 6);
        assert_eq!(prueba.target_partitions(), PARTICIONES_DE_PRUEBA);
        // Una consulta distribuida: la MISMA configuración en los tres procesos (S0 C2), salvo el scheduler al
        // que llama el cliente, que viaja con la consulta.
        assert_eq!(entradas(&scheduler), entradas(&ejecutor));
        assert_eq!(diferencias(&distribuida, &scheduler), vec!["ore.scheduler"]);
        // Una consulta local no paga las restricciones de Ballista, y sólo ésas.
        let mut restringida = sola.clone();
        for d in DESVIOS_DISTRIBUIDOS {
            fijar(&mut restringida, d.clave, d.valor).unwrap();
        }
        assert_eq!(entradas(&restringida), entradas(&scheduler));
        assert!(!diferencias(&sola, &scheduler).is_empty());
        // Y la de prueba sólo cambia las particiones.
        assert_eq!(
            diferencias(&sola, &prueba),
            vec!["datafusion.execution.target_partitions"]
        );
    }

    #[test]
    fn las_claves_de_ballista_estan_registradas_en_todos_los_papeles() {
        for p in [
            Papel::Local,
            Papel::Scheduler,
            Papel::Ejecutor,
            Papel::Prueba,
        ] {
            let mut c = config_de(p, &limites(2), &OreOpciones::default()).unwrap();
            fijar(&mut c, "ballista.job.name", "suelo").unwrap();
            assert!(fijar(&mut c, "ballista.no_existe", "1").is_err(), "{p:?}");
            assert!(
                entradas(&c).contains(&("ballista.job.name".into(), Some("suelo".into()))),
                "{p:?}"
            );
        }
    }

    /// `DESVIOS_DISTRIBUIDOS` hace **exactamente** lo que Ballista restringe: aplicados sobre la configuración
    /// de DataFusion, dan la misma que `upgrade_for_ballista`, clave a clave.
    #[test]
    fn las_restricciones_de_ballista_son_las_escritas() {
        use ballista_core::extension::SessionConfigExt;
        let datafusion = |c: &SessionConfig| -> Vec<(String, Option<String>)> {
            c.options()
                .entries()
                .into_iter()
                .filter(|e| e.key.starts_with("datafusion."))
                .map(|e| (e.key, e.value))
                .collect()
        };
        let ballista = SessionConfig::new().upgrade_for_ballista();
        let mut escrita = SessionConfig::new();
        for d in DESVIOS_DISTRIBUIDOS {
            fijar(&mut escrita, d.clave, d.valor).unwrap();
            assert!(!d.porque.is_empty(), "`{}` sin porqué", d.clave);
        }
        let distintas: Vec<_> = datafusion(&ballista)
            .into_iter()
            .zip(datafusion(&escrita))
            .filter(|(a, b)| a != b)
            .collect();
        assert!(
            distintas.is_empty(),
            "Ballista restringe otra cosa que lo escrito en `DESVIOS_DISTRIBUIDOS` (Ballista, lo escrito): \
             {distintas:?}"
        );
    }

    #[test]
    fn la_holgura_tiene_que_tener_sentido() {
        for h in [-0.1, 0.95, f64::NAN] {
            let o = OreOpciones {
                holgura_memoria: h,
                ..OreOpciones::default()
            };
            assert!(config_de(Papel::Local, &limites(1), &o).is_err(), "{h}");
        }
    }
}
