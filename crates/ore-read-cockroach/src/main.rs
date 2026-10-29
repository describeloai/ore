//! `ore-read-cockroach`: los verbos v2 del esqueleto (`capacidades`, `leer`, `estimar`, `servir`) y los de CockroachDB.

use ore_cockroach::{Cockroach, verbos};

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<Cockroach>(verbos::otros)
}
