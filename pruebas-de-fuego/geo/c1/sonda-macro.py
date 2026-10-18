"""ORE 0068 C1·1 · Una Function con geometría desde SQL: UDF sobre WKB + macro que pone tipo y CRS."""
import duckdb
c = duckdb.connect(); c.execute("set extension_directory = '/opt/ore/duckdb'"); c.execute("load spatial")
vistos = []


def buffer100(wkb):  # lo que haría el contrato: WKB → shapely → WKB
    import shapely
    g = shapely.from_wkb(bytes(wkb)); vistos.append(g.geom_type)
    return shapely.to_wkb(g.buffer(100))


c.create_function("__ore_fn_1_wkb", buffer100, ["BLOB"], "BLOB")
c.execute("""create macro __ore_fn_1(g) as
  ST_SetCRS(ST_GeomFromWKB(__ore_fn_1_wkb(ST_AsWKB(g::GEOMETRY('EPSG:25830')))), 'EPSG:25830')""")
P = "ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830')"
Q = "ST_SetCRS(ST_Point(-3.7, 40.4), 'EPSG:4326')"


def caso(n, sql):
    try:
        print(n, "→", c.sql(sql).fetchall(), vistos[-1:])
    except Exception as e:
        print(n, "FALLA", type(e).__name__, str(e).splitlines()[0][:200])


caso("bien", f"select typeof(__ore_fn_1({P})), round(ST_Area(__ore_fn_1({P})))")
caso("describe", f"describe select __ore_fn_1({P}) as z")
caso("otro CRS", f"select __ore_fn_1({Q})")
caso("sin CRS", "select __ore_fn_1(ST_Point(1, 2))")
caso("cast entre CRS", f"select {Q}::GEOMETRY('EPSG:25830')")
caso("null", "select __ore_fn_1(null::GEOMETRY('EPSG:25830'))")
