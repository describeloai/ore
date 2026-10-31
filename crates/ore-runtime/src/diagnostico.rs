//! **El runtime se describe y se mide en su sitio** (0081 §7.4, S6): qué es, qué lee de su pod y lo que cuesta,
//! medido por el propio binario. Es `ore-runtime diagnostico`, y lo que S6 corre en victor: la imagen es
//! `scratch` y no lleva más herramientas que ésta.
//!
//! Tres medidas, las del cierre del suelo (§7.4):
//! - **abrir una sesión** y **abrir y planificar `select 1`**: lo que cuesta cada consulta antes de calcular;
//! - **las particiones**: `target_partitions` sale de la cuota del cgroup, no de los núcleos del nodo;
//! - **el latido**: con la CPU llena (una consulta pesada por hilo de CPU), lo que se retrasa un latido de
//!   20 ms en el runtime de E/S, que es donde vive la salud.
//!
//! Sale una línea por medida, `clave valor`, para leerla con `grep` o un programa.

use crate::fallo::Fallo;
use crate::motor::Motor;
use crate::opciones::Papel;
use crate::sesion::Contexto;
use std::time::{Duration, Instant};

/// Una consulta que tarda: la CPU de un hilo, llena, durante segundos.
const PESADA: &str = "select sum((v * 7919) % 1000003) from generate_series(1, 4000000000) t(v)";
/// Cuántas veces se abre una sesión para medirlo.
const SESIONES: u32 = 20_000;
/// Cuántas veces se abre y se planifica `select 1`.
const PLANES: u32 = 2_000;
/// Los latidos, y cada cuánto.
const LATIDOS: u32 = 50;
const CADA: Duration = Duration::from_millis(20);

/// Mide un motor de papel local y devuelve sus medidas en orden.
pub fn medir(motor: &Motor) -> Result<Vec<(String, String)>, Fallo> {
    if motor.papel() != Papel::Local {
        return Err(Fallo::Config(
            "el diagnóstico mide un motor de papel local".into(),
        ));
    }
    let l = motor.limites();
    let mut m: Vec<(String, String)> = vec![
        ("manifiesto".into(), motor.manifiesto().to_string()),
        (
            "limites.memoria".into(),
            format!("{} MiB ({:?})", l.memoria >> 20, l.memoria_de),
        ),
        ("limites.cpu".into(), format!("{} ({:?})", l.cpu, l.cpu_de)),
        (
            "limites.disco".into(),
            l.disco
                .map_or("sin medir".into(), |d| format!("{} MiB", d >> 20)),
        ),
        (
            "target_partitions".into(),
            motor.config().target_partitions().to_string(),
        ),
        (
            "raiz_de_memoria".into(),
            format!("{} MiB", motor.raiz_de_memoria() >> 20),
        ),
        (
            "nucleos_del_nodo".into(),
            // Lo que vería quien no lee el cgroup: los del nodo, sin la cuota.
            unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) }.to_string(),
        ),
    ];

    // Abrir una sesión: un clon de la plantilla.
    for i in 0..100 {
        std::hint::black_box(motor.sesion(&Contexto::nuevo(i.to_string()))?);
    }
    let t = Instant::now();
    for i in 0..SESIONES {
        std::hint::black_box(motor.sesion(&Contexto::nuevo(i.to_string()))?);
    }
    m.push((
        "abrir_una_sesion".into(),
        format!("{:?}", t.elapsed() / SESIONES),
    ));

    // Abrir y planificar `select 1`.
    let t = Instant::now();
    for i in 0..PLANES {
        let mut s = motor.sesion(&Contexto::nuevo(i.to_string()))?;
        std::hint::black_box(motor.es().block_on(s.planificar("select 1"))?.logico()?);
    }
    m.push((
        "abrir_y_planificar_select_1".into(),
        format!("{:?}", t.elapsed() / PLANES),
    ));

    // El latido con la CPU llena: una consulta pesada por hilo de CPU.
    let mut pesadas = Vec::new();
    for i in 0..l.cpu {
        let mut s = motor.sesion(&Contexto::nuevo(format!("pesada-{i}")))?;
        let plan = motor.es().block_on(s.planificar(PESADA))?.logico()?;
        pesadas.push(motor.correr(&s, plan)?);
    }
    std::thread::sleep(Duration::from_millis(500));
    let (peor, media) = motor.es().block_on(async {
        let (mut peor, mut total) = (Duration::ZERO, Duration::ZERO);
        for _ in 0..LATIDOS {
            let t = Instant::now();
            tokio::time::sleep(CADA).await;
            let tarde = t.elapsed().saturating_sub(CADA);
            peor = peor.max(tarde);
            total += tarde;
        }
        (peor, total / LATIDOS)
    });
    drop(pesadas);
    m.push((
        "latido_con_la_cpu_llena".into(),
        format!(
            "peor {peor:?}, media {media:?} ({} consultas pesadas, latido de {CADA:?})",
            l.cpu
        ),
    ));
    Ok(m)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::limites::{Fuente, Limites};
    use crate::opciones::OreOpciones;
    use crate::zocalos::Zocalos;

    #[test]
    fn el_diagnostico_sale_entero_y_solo_en_local() {
        let limites = |cpu| Limites {
            memoria: 1 << 30,
            memoria_de: Fuente::Host,
            cpu,
            cpu_de: Fuente::Host,
            disco: None,
        };
        let m = Motor::arrancar_con(
            Papel::Local,
            OreOpciones::default(),
            &Zocalos::default(),
            limites(2),
        )
        .unwrap();
        let medidas = medir(&m).unwrap();
        let claves: Vec<&str> = medidas.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            claves,
            [
                "manifiesto",
                "limites.memoria",
                "limites.cpu",
                "limites.disco",
                "target_partitions",
                "raiz_de_memoria",
                "nucleos_del_nodo",
                "abrir_una_sesion",
                "abrir_y_planificar_select_1",
                "latido_con_la_cpu_llena",
            ]
        );
        assert_eq!(medidas[4].1, "2");
        let s = Motor::arrancar_con(
            Papel::Scheduler,
            OreOpciones::default(),
            &Zocalos::default(),
            limites(1),
        )
        .unwrap();
        assert!(medir(&s).is_err());
    }
}
