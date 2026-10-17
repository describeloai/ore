fn main() {
    let s = std::fs::read_to_string(std::env::args().nth(1).unwrap().as_str()).unwrap();
    match serde_json::from_str::<iceberg::spec::TableMetadata>(&s) {
        Ok(m) => {
            println!("SONDA ok v{:?}", m.format_version());
            for f in m.current_schema().as_struct().fields() { println!("SONDA campo {} {:?}", f.name, f.field_type); }
        }
        Err(e) => println!("SONDA error: {e}"),
    }
}
