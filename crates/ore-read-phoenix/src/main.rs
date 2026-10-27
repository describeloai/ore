//! `ore-read-phoenix`: los verbos v2 del esqueleto (`capacidades`, `leer`, `servir`) y los de
//! Phoenix (`check`, `testigo`, `cambio`; `catalogo` y `explorar` son de M3).

use ore_phoenix::{lectura::Phoenix, verbos};

#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<Phoenix>(verbos::otros)
}
