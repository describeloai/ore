"""ORE 0068 C1·1 · El texto exacto de los errores geo de DuckDB 1.5.6, para decirlos como ORE."""
import duckdb
c = duckdb.connect(); c.execute("set extension_directory = '/opt/ore/duckdb'"); c.execute("load spatial")
A = "ST_SetCRS(ST_Point(1, 2), 'EPSG:4326')"
B = "ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830')"
for sql in [f"select ST_Intersects({A}, {B})", f"select ST_Distance({A}, {B})", f"select {A}::GEOMETRY('EPSG:25830')",
            "select ST_SetCRS(ST_Point(1, 2), 'EPSG:999999')", "select ST_Point(1,2)::GEOMETRY('EPSG:4326') = 1"]:
    try:
        c.sql(sql).fetchall(); print("VA:", sql)
    except Exception as e:
        print(type(e).__name__, "|", str(e).replace("\n", " ⏎ ")[:300])
