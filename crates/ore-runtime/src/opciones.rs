//! **La configuración como contrato** (0081 §7.4, propiedad 5).
//!
//! - Las opciones de ORE son una `ConfigExtension` con el prefijo `ore` ([`OreOpciones`]).
//! - Lo que se aparta de DataFusion está en [`DESVIOS`], cada valor con su porqué, y una prueba comprueba
//!   que cada clave sigue existiendo: si una actualización la renombra, se sabe aquí.
//! - La configuración es una **función pura** del papel, los límites y las opciones ([`config_de`]): la
//!   misma en los tres papeles. Así lo exige Ballista, que en el scheduler y el ejecutor pierde **en
//!   silencio** las claves que no tiene registradas (0081 S0 C2).
//! - Una clave pasa siempre por `options_mut().set()`, que devuelve error. Nunca por `set_str`, que hace
//!   pánico con una clave desconocida.

use crate::fallo::Fallo;
use crate::limites::Limites;
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
    let mut config = SessionConfig::new().with_option_extension(opciones.clone());
    for d in DESVIOS {
        fijar(&mut config, d.clave, d.valor)?;
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
/// puede volver a aplicar ni distinguir de otra extensión: aquí van las de DataFusion tal cual y las de ORE
/// con `ore.` delante.
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

    #[test]
    fn los_papeles_difieren_donde_deben_y_solo_ahi() {
        let o = OreOpciones::default();
        let l = limites(6);
        let c = |p| config_de(p, &l, &o).unwrap();
        let (local, scheduler, ejecutor, prueba) = (
            c(Papel::Local),
            c(Papel::Scheduler),
            c(Papel::Ejecutor),
            c(Papel::Prueba),
        );
        assert_eq!(local.target_partitions(), 6);
        assert_eq!(prueba.target_partitions(), PARTICIONES_DE_PRUEBA);
        let claves = entradas;
        // Los tres papeles de una consulta de verdad: la MISMA configuración (S0 C2).
        assert_eq!(claves(&local), claves(&scheduler));
        assert_eq!(claves(&local), claves(&ejecutor));
        // Y la de prueba sólo cambia las particiones.
        let distintas: Vec<_> = claves(&local)
            .into_iter()
            .zip(claves(&prueba))
            .filter(|(a, b)| a != b)
            .map(|(a, _)| a.0)
            .collect();
        assert_eq!(
            distintas,
            vec!["datafusion.execution.target_partitions".to_string()]
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
