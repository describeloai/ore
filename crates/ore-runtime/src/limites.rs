//! **Lo que el proceso tiene** (0081 §7.4, propiedad 7): memoria, CPU y disco, leídos del cgroup y no del
//! nodo. Con eso se dimensionan la raíz de memoria, el paralelismo y el derrame.
//!
//! Los analizadores son funciones puras, probadas con el texto exacto de cada fichero; [`Limites::leer`]
//! sólo los alimenta. Medido en S0 (C4): `std::thread::available_parallelism` ya respeta `cpu.max` y
//! redondea hacia abajo (1,5 CPU → 1). Aquí se hace lo mismo, **pero dicho**: la CPU sale entera y nunca 0.

use std::path::Path;

/// De dónde salió una cifra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fuente {
    /// cgroup v2 (`memory.max`, `cpu.max`).
    CgroupV2,
    /// cgroup v1 (`memory.limit_in_bytes`, `cpu.cfs_quota_us`).
    CgroupV1,
    /// Sin límite propio: lo del nodo.
    Host,
}

/// Lo que el proceso tiene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limites {
    /// Bytes de memoria.
    pub memoria: u64,
    /// De dónde salió la memoria.
    pub memoria_de: Fuente,
    /// CPU enteras, nunca 0.
    pub cpu: usize,
    /// De dónde salió la CPU.
    pub cpu_de: Fuente,
    /// Bytes libres en el directorio de derrame, si se dio uno y se pudo medir.
    pub disco: Option<u64>,
}

const V2_MEMORIA: &str = "/sys/fs/cgroup/memory.max";
const V2_CPU: &str = "/sys/fs/cgroup/cpu.max";
const V1_MEMORIA: &str = "/sys/fs/cgroup/memory/memory.limit_in_bytes";
const V1_CUOTA: &str = "/sys/fs/cgroup/cpu/cpu.cfs_quota_us";
const V1_PERIODO: &str = "/sys/fs/cgroup/cpu/cpu.cfs_period_us";
const MEMINFO: &str = "/proc/meminfo";

impl Limites {
    /// Lee los límites de este proceso. `derrame` es el directorio de derrame, si lo hay.
    pub fn leer(derrame: Option<&Path>) -> Limites {
        let leer = |p: &str| std::fs::read_to_string(p).ok();
        let host_memoria = leer(MEMINFO)
            .as_deref()
            .and_then(memoria_del_host)
            .unwrap_or(u64::MAX);
        let host_cpu = std::thread::available_parallelism().map_or(1, |n| n.get());

        let (memoria, memoria_de) =
            if let Some(m) = leer(V2_MEMORIA).as_deref().and_then(memoria_v2) {
                (m.min(host_memoria), Fuente::CgroupV2)
            } else if let Some(m) = leer(V1_MEMORIA)
                .as_deref()
                .and_then(|t| memoria_v1(t, host_memoria))
            {
                (m, Fuente::CgroupV1)
            } else {
                (host_memoria, Fuente::Host)
            };

        let (cpu, cpu_de) = if let Some(c) = leer(V2_CPU).as_deref().and_then(cpu_v2) {
            (c.min(host_cpu), Fuente::CgroupV2)
        } else if let Some(c) = leer(V1_CUOTA)
            .zip(leer(V1_PERIODO))
            .and_then(|(q, p)| cpu_v1(&q, &p))
        {
            (c.min(host_cpu), Fuente::CgroupV1)
        } else {
            (host_cpu, Fuente::Host)
        };

        Limites {
            memoria,
            memoria_de,
            cpu,
            cpu_de,
            disco: derrame.and_then(disco_libre),
        }
    }
}

/// `memory.max` de cgroup v2: un número de bytes, o `max` (sin límite).
pub fn memoria_v2(texto: &str) -> Option<u64> {
    match texto.trim() {
        "max" => None,
        n => n.parse().ok(),
    }
}

/// `cpu.max` de cgroup v2: `<cuota> <periodo>`, o `max <periodo>` (sin límite). Entera hacia abajo y nunca 0.
pub fn cpu_v2(texto: &str) -> Option<usize> {
    let mut partes = texto.split_whitespace();
    let cuota = partes.next()?;
    let periodo: u64 = partes.next()?.parse().ok()?;
    if cuota == "max" || periodo == 0 {
        return None;
    }
    let cuota: u64 = cuota.parse().ok()?;
    Some(((cuota / periodo) as usize).max(1))
}

/// `memory.limit_in_bytes` de cgroup v1: sin límite es un número enorme (cercano a `i64::MAX`), así que lo
/// que no queda por debajo de la memoria del nodo no es un límite.
pub fn memoria_v1(texto: &str, host: u64) -> Option<u64> {
    let n: u64 = texto.trim().parse().ok()?;
    (n < host).then_some(n)
}

/// `cpu.cfs_quota_us` y `cpu.cfs_period_us` de cgroup v1: una cuota de `-1` es sin límite.
pub fn cpu_v1(cuota: &str, periodo: &str) -> Option<usize> {
    let cuota: i64 = cuota.trim().parse().ok()?;
    let periodo: i64 = periodo.trim().parse().ok()?;
    if cuota <= 0 || periodo <= 0 {
        return None;
    }
    Some(((cuota / periodo) as usize).max(1))
}

/// `MemTotal` de `/proc/meminfo`, en bytes.
pub fn memoria_del_host(meminfo: &str) -> Option<u64> {
    let linea = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = linea.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// Los bytes libres para el proceso en el sistema de ficheros de `dir`.
#[cfg(unix)]
pub fn disco_libre(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let ruta = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `ruta` es una cadena C válida durante la llamada y `s` es memoria del tamaño de `statvfs`, que
    // `statvfs` rellena entera cuando devuelve 0; sólo entonces se lee.
    let s = unsafe {
        if libc::statvfs(ruta.as_ptr(), s.as_mut_ptr()) != 0 {
            return None;
        }
        s.assume_init()
    };
    Some((s.f_bavail as u64).saturating_mul(s.f_frsize as u64))
}

/// Fuera de Unix el disco no se mide: el tope lo dan las opciones o DataFusion.
#[cfg(not(unix))]
pub fn disco_libre(_dir: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_memoria_de_cgroup_v2() {
        assert_eq!(memoria_v2("1073741824\n"), Some(1 << 30));
        assert_eq!(memoria_v2("max\n"), None);
        assert_eq!(memoria_v2("basura"), None);
    }

    #[test]
    fn la_cpu_de_cgroup_v2_entera_hacia_abajo_y_nunca_cero() {
        assert_eq!(cpu_v2("200000 100000\n"), Some(2));
        assert_eq!(
            cpu_v2("150000 100000\n"),
            Some(1),
            "1,5 CPU → 1, como std (S0 C4)"
        );
        assert_eq!(cpu_v2("50000 100000\n"), Some(1), "media CPU no es 0");
        assert_eq!(cpu_v2("max 100000\n"), None);
        assert_eq!(cpu_v2("100000 0"), None);
        assert_eq!(cpu_v2(""), None);
    }

    #[test]
    fn la_memoria_de_cgroup_v1_sin_limite_es_un_numero_enorme() {
        let host = 64 << 30;
        assert_eq!(memoria_v1("9223372036854771712\n", host), None);
        assert_eq!(memoria_v1("4294967296\n", host), Some(4 << 30));
    }

    #[test]
    fn la_cpu_de_cgroup_v1() {
        assert_eq!(cpu_v1("400000\n", "100000\n"), Some(4));
        assert_eq!(cpu_v1("-1\n", "100000\n"), None);
        assert_eq!(cpu_v1("30000", "100000"), Some(1));
    }

    #[test]
    fn la_memoria_del_nodo() {
        let meminfo = "MemTotal:       65802940 kB\nMemFree:         1234 kB\n";
        assert_eq!(memoria_del_host(meminfo), Some(65802940 * 1024));
        assert_eq!(memoria_del_host("MemFree: 1 kB"), None);
    }

    #[test]
    fn leer_da_algo_sensato_aqui() {
        let dir = tempfile::tempdir().unwrap();
        let l = Limites::leer(Some(dir.path()));
        assert!(l.cpu >= 1);
        assert!(l.memoria > 0);
        #[cfg(unix)]
        assert!(l.disco.is_some_and(|d| d > 0), "{l:?}");
    }

    /// En un contenedor con límites conocidos: `ORE_ESPERADA_CPU` y `ORE_ESPERADA_MEMORIA` (bytes). Se corre
    /// a mano con `docker run --cpus N -m M` (0081 S2·2).
    #[test]
    #[ignore = "necesita un contenedor con límites conocidos"]
    fn lee_los_limites_de_este_contenedor() {
        let l = Limites::leer(None);
        let esperada = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
        println!("{l:?}");
        if let Some(c) = esperada("ORE_ESPERADA_CPU") {
            assert_eq!(l.cpu as u64, c, "{l:?}");
            assert_ne!(l.cpu_de, Fuente::Host);
        }
        if let Some(m) = esperada("ORE_ESPERADA_MEMORIA") {
            assert_eq!(l.memoria, m, "{l:?}");
            assert_ne!(l.memoria_de, Fuente::Host);
        }
    }
}
