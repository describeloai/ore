//! Cuánta pila pide derivar un `.ts` hostil, por byte de fuente (0050 R3).
//! `cargo run --release --example hostil_ts -- <caso> <n>`: prueba con 1, 2,
//! 4… MiB en un proceso hijo hasta que no desborda.

fn fuente(caso: &str, n: usize) -> String {
    let f = "\nexport default function repeat(): string { return \"\"; }\n";
    match caso {
        "c" => format!("const x = {}1{};{f}", "[".repeat(n), "]".repeat(n)),
        "m" => format!("const x = {}1;{f}", "-".repeat(n)),
        "s" => format!("const x = 1{};{f}", "+1".repeat(n)),
        "p" => format!("type X = {}string{};{f}", "(".repeat(n), ")".repeat(n)),
        "a" => format!("type X = string{};{f}", "[]".repeat(n)),
        "o" => format!("type X = {}string{};{f}", "{a:".repeat(n), "}".repeat(n)),
        "u" => format!("type X = {}string;{f}", "string|".repeat(n)),
        _ => format!("const x = {}1;{f}", "()=>".repeat(n)),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "--hijo" {
        let f = fuente(&a[2], a[3].parse().unwrap());
        let mb: usize = a[4].parse().unwrap();
        stacker::grow(mb << 20, || {
            drop(ore_code::typescript::derivar_en_esta_pila(
                &f,
                "x/functions/repeat.ts",
            ))
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
