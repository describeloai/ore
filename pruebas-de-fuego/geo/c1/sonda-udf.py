"""ORE 0068 C1·1 · ¿Cómo se tipa una UDF de Python con GEOMETRY en DuckDB 1.5.6?"""
import duckdb
c = duckdb.connect(); c.execute("set extension_directory = '/opt/ore/duckdb'"); c.execute("load spatial")
P = "ST_SetCRS(ST_Point(1, 2), 'EPSG:25830')"
vistos = []


def f(g):
    vistos.append(type(g).__name__ + ":" + (bytes(g).hex()[:14] if isinstance(g, (bytes, bytearray, memoryview)) else str(g)[:30]))
    return g


def prueba(nombre, hacer_tipo):
    vistos.clear()
    try:
        tipo = hacer_tipo()
        n = "f_" + "".join(ch for ch in nombre if ch.isalnum())
        c.create_function(n, f, [tipo], tipo)
        r = c.sql(f"select typeof({n}({P})), ST_AsText({n}({P}))").fetchone()
        print(nombre, "→", r, vistos[:1])
    except Exception as e:
        print(nombre, "FALLA", type(e).__name__, str(e).splitlines()[0][:200])


prueba("texto GEOMETRY", lambda: "GEOMETRY")
prueba("c.sqltype GEOMETRY", lambda: c.sqltype("GEOMETRY"))
prueba("c.sqltype GEOMETRY crs", lambda: c.sqltype("GEOMETRY('EPSG:25830')"))
prueba("texto GEOMETRY crs", lambda: "GEOMETRY('EPSG:25830')")
