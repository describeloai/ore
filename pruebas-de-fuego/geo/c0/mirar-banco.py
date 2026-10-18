"""ORE 0068 C0·1 · Mirar el banco local antes de definir los casos: esquemas, filas y subtipos."""
import duckdb

c = duckdb.connect()
c.execute("set extension_directory = '/opt/ore/duckdb'")
c.execute("load spatial")
B = "/banco"
for t in ("edificios_es", "lugares_es", "divisiones_es", "carreteras_es"):
    print("==", t)
    print(c.sql(f"select column_name, column_type from (describe select * from '{B}/{t}.parquet')").fetchall())
    print(c.sql(f"select count(*) from '{B}/{t}.parquet'").fetchall())
print(c.sql(f"select subtype, count(*) from '{B}/divisiones_es.parquet' group by 1 order by 2 desc").fetchall())
print(c.sql(f"select st_geometrytype(geometry) g, count(*) from '{B}/divisiones_es.parquet' group by 1").fetchall())
