fn main() -> std::process::ExitCode {
    ore_driver::origen::principal::<ore_salesforce::lectura::Salesforce>(
        ore_salesforce::verbos::otros,
    )
}
