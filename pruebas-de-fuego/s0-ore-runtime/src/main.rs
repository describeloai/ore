//! S0: el trío compila, qué versión dice cada pieza, qué paralelismo ve el proceso y cuánto
//! cuesta clonar una sesión.
use datafusion::execution::SessionStateBuilder;
use datafusion::prelude::*;
use std::time::Instant;

#[tokio::main]
async fn main() -> datafusion::error::Result<()> {
    println!("datafusion {}", datafusion::DATAFUSION_VERSION);
    println!("available_parallelism {:?}", std::thread::available_parallelism());
    let config = SessionConfig::new();
    println!("target_partitions por defecto {}", config.target_partitions());

    let plantilla = SessionStateBuilder::new()
        .with_config(config)
        .with_default_features()
        .build();
    let n = 10_000;
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(plantilla.clone());
    }
    println!("clonar la sesión: {:?} por clon", t.elapsed() / n);

    let t = Instant::now();
    for _ in 0..100 {
        std::hint::black_box(SessionStateBuilder::new().with_default_features().build());
    }
    println!("construir la sesión: {:?} por sesión", t.elapsed() / 100);

    let ctx = SessionContext::new_with_state(plantilla.clone());
    ctx.sql("select 1 + 1 as dos").await?.show().await?;
    Ok(())
}
