import duckdb
c = duckdb.connect()
c.execute("install iceberg; load iceberg; install spatial; load spatial")
c.execute("set unsafe_enable_version_guessing=true")
for t in ("nativo_v2", "nativo_v3", "plano_v3"):
    for v, que in (("v2", "A (binary)"), ("v3", "C (geometry, tras el paso)")):
        m = f"/salida/wh/g20/{t}/metadata/{v}.metadata.json"
        try:
            r = c.execute(f"select count(*), typeof(any_value(geom)), any_value(ST_AsText(CASE WHEN typeof(geom) = 'BLOB' THEN ST_GeomFromWKB(geom) ELSE geom::GEOMETRY END)) from iceberg_scan('{m}')").fetchone()
            print(f"RESULTADO | {t} · {que} · DuckDB iceberg_scan | bien | {r}")
        except Exception as e:
            print(f"RESULTADO | {t} · {que} · DuckDB iceberg_scan | falla | {str(e).splitlines()[0][:200]}")
for f in ("geo-nativo", "geo-plano"):
    r = c.execute(f"select typeof(any_value(geom)) from '/salida/{f}.parquet'").fetchone()
    print(f"RESULTADO | {f}.parquet · DuckDB directo | bien | {r}")
