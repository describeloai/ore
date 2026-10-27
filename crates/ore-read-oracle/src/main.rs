//! `ore-read-oracle`: los verbos v2 del esqueleto (`capacidades`, `leer`, `servir`) y los de Oracle
//! (`catalogo`, `check`, `explorar`, `testigo`, `cambio`).

use ore_oracle::{lectura::Oracle, verbos};

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<Oracle>(verbos::otros)
}
