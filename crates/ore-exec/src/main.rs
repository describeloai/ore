fn main() {
    let raiz = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    match ore_exec::Motor::cargar(std::path::Path::new(&raiz)) {
        Err(e) => {
            eprintln!("{e:?}");
            std::process::exit(65);
        }
        Ok(m) => {
            let errores = m.validar();
            let avisos = m.avisos();
            for e in &errores {
                println!("error: {e}");
            }
            for a in &avisos {
                println!("aviso: {a}");
            }
            println!("\n{} errores, {} avisos", errores.len(), avisos.len());
            if !errores.is_empty() {
                std::process::exit(65);
            }
        }
    }
}
