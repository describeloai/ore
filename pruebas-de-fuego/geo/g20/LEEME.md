# G2·0 · el paso de `binary` a `geometry` en Iceberg v3 (ADR 0067)

Laboratorio local, repetible:

1. `src/main.rs` escribe dos ficheros de datos con los `field_id` de Iceberg: `geo-nativo.parquet` (tipo
   lógico GEOMETRY de Parquet, lo que escribirá G2·1) y `geo-plano.parquet` (el mismo WKB como
   `BYTE_ARRAY`, lo que escribe hoy iceberg-rust). `cargo run --release` en `rust:1.90`, salida en `/salida`.
2. `g20.scala`, con Spark 4.1 + Iceberg 1.11 (catálogo `hadoop` en `/salida/wh`): tres tablas (v2 y v3
   sobre el nativo, v3 sobre el plano) con `geom: binary`. Mide quién lee hoy, si la API acepta el paso
   y el paso solo en metadatos, y quién lee después (también el snapshot de antes).
3. `g20_duckdb.py`, con DuckDB 1.5.6 y su extensión `iceberg`, sobre los mismos `metadata.json`.

Los resultados y la decisión están en la bitácora de ADR 0067 (G2·0).
