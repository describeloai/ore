use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};

fn fuente(caso: &str, n: usize) -> String {
    match caso {
        "c" => format!("x = {}1{}\n", "[".repeat(n), "]".repeat(n)),
        "m" => format!("x = {}1\n", "-".repeat(n)),
        "s" => format!("x = {}1\n", "1+".repeat(n)),
        "p" => format!("x = {}1\n", "2**".repeat(n)),
        _ => format!("x = {}1\n", "lambda: ".repeat(n)),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "--hijo" {
        let f = fuente(&a[2], a[3].parse().unwrap());
        let mb: usize = a[4].parse().unwrap();
        stacker::grow(mb << 20, || {
            drop(parse_unchecked(&f, ParseOptions::from(Mode::Module)))
        });
        return;
    }
    let (caso, n) = (&a[1], a[2].parse::<usize>().unwrap());
    let largo = fuente(caso, n).len();
    for mb in [1usize, 2, 4, 8, 16, 32, 64, 128, 256, 512] {
        let ok = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--hijo", caso, &n.to_string(), &mb.to_string()])
            .output()
            .unwrap()
            .status
            .success();
        if ok {
            println!(
                "{caso} n={n} ({largo} B): {mb} MiB → {:.0} B de pila por byte",
                (mb << 20) as f64 / largo as f64
            );
            return;
        }
    }
    println!("{caso} n={n}: ni con 512 MiB");
}
