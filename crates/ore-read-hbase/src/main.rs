//! `ore-read-hbase`: los verbos v2 del esqueleto (`capacidades`, `leer`, `servir`) y los de HBase (`catalogo`,
//! `check`, `explorar`, `testigo`, `cambio`).

use ore_hbase::{lectura::Hbase, verbos};

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<Hbase>(verbos::otros)
}
