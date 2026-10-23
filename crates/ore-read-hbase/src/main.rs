//! `ore-read-hbase`: los verbos v2 del esqueleto (`capacidades`, `leer`, `servir`) y los de HBase (`catalogo`,
//! `check`, `explorar`, `testigo`, `cambio`).

use ore_hbase::{lectura::Hbase, verbos};

// El `malloc` de musl tiene un candado global y la lectura en paralelo reserva memoria por celda: ver el `Cargo.toml`.
#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<Hbase>(verbos::otros)
}
