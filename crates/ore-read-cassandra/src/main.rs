//! `ore-read-cassandra`: los verbos v2 del esqueleto (`capacidades`, `leer`,
//! `servir`) y los de Cassandra (`catalogo`, `check`, `explorar`, `testigo`, `cambio`).

use ore_cassandra::{lectura::Cassandra, verbos};

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<Cassandra>(verbos::otros)
}
