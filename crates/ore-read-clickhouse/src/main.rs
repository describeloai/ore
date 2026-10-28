//! `ore-read-clickhouse`: los verbos v2 del esqueleto (`capacidades`, `leer`, `estimar`, `servir`) y los de ClickHouse.

use ore_clickhouse::{ClickHouse, verbos};

fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<ClickHouse>(verbos::otros)
}
