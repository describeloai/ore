"""ORE 0068 C1·1 · El previo de SQL: cuatro preguntas, contra DuckDB 1.5.6 y el SDK del árbol.

    docker run --rm -v <árbol>/puesto/python:/sdk -v <aquí>:/c1 --entrypoint python g235-puesto-python:latest /c1/sonda-sql.py

S1 · Una Function del árbol desde SQL con geometría: ¿cómo llega una GEOMETRY('CRS') a una UDF de
     Python (nativa y arrow) y cómo vuelve?
S2 · La vista con copia (`calcular.py`): una consulta con ST_* sobre Arrow con `geoarrow.wkb`.
S3 · Un Transform SQL: lo que sale de DuckDB, ¿lleva la marca y el CRS que `write()` reconoce?
S4 · El editor: ¿los diagnósticos dicen la mezcla de CRS y la geometría sin CRS?
"""
import json, sys, traceback
sys.path.insert(0, "/sdk")
import duckdb, pyarrow as pa

EXT = "/opt/ore/duckdb"


def con(geo=True):
    c = duckdb.connect()
    c.execute(f"set extension_directory = '{EXT}'")
    if geo:
        c.execute("load spatial")
    return c


def caso(nombre, f):
    try:
        print(f"{nombre}: {f()}")
    except Exception as e:
        print(f"{nombre}: FALLA {type(e).__name__}: {str(e).splitlines()[0][:220]}")


# ── S1 ───────────────────────────────────────────────────────────────────────────────────
def s1_nativa():
    c = con()
    vistos = []

    def f(g):
        vistos.append(type(g).__name__)
        return g
    c.create_function("f_nat", f, ["GEOMETRY('EPSG:25830')"], "GEOMETRY('EPSG:25830')")
    r = c.sql("select f_nat(ST_SetCRS(ST_Point(1, 2), 'EPSG:25830')) as g").fetchall()
    t = c.sql("select typeof(f_nat(ST_SetCRS(ST_Point(1, 2), 'EPSG:25830')))").fetchone()[0]
    return f"llega como {vistos[:1]}, vuelve {type(r[0][0]).__name__}, tipo {t}"


def s1_arrow():
    c = con()
    vistos = []

    def f(a):
        vistos.append((str(a.type), (a.type.__class__.__name__)))
        return a
    c.create_function("f_arr", f, ["GEOMETRY('EPSG:25830')"], "GEOMETRY('EPSG:25830')", type="arrow")
    t = c.sql("select typeof(f_arr(ST_SetCRS(ST_Point(1, 2), 'EPSG:25830')))").fetchone()[0]
    return f"llega como {vistos[:1]}, tipo {t}"


def s1_blob():
    # Lo que haría duckdb_type si Geometry fuera BLOB: ¿se puede envolver?
    c = con()
    c.create_function("f_wkb", lambda b: b, ["BLOB"], "BLOB")
    t = c.sql("select typeof(ST_GeomFromWKB(f_wkb(ST_AsWKB(ST_SetCRS(ST_Point(1, 2), 'EPSG:25830')))))").fetchone()[0]
    return f"BLOB y vuelta con ST_GeomFromWKB: {t} (el CRS se pierde: hay que reponerlo)"


def s1_sdk():
    from ore import sql_functions
    return sql_functions.duckdb_type("Geometry<Point, EPSG:25830>")


# ── S2 ───────────────────────────────────────────────────────────────────────────────────
def s2():
    c0 = con()
    tabla = c0.sql("select 1 as id, ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830') as geom").arrow()
    campo = tabla.schema.field("geom")
    c = con(geo=False)  # como calcular.py: sin spatial
    c.register("t", tabla)
    tipo = c.sql("select typeof(geom) from t").fetchone()[0]
    try:
        c.sql("select ST_Buffer(geom, 10) from t").fetchall()
        sin = "ST_Buffer sin spatial: va"
    except Exception as e:
        sin = "ST_Buffer sin spatial: " + str(e).splitlines()[0][:120]
    return f"marca {dict(campo.metadata or {}).get(b'ARROW:extension:name')}; tipo leído {tipo}; {sin}"


# ── S3 ───────────────────────────────────────────────────────────────────────────────────
def s3():
    import ore
    c = con()
    t = c.sql("select ST_Buffer(ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830'), 100) as zona").arrow()
    f = t.schema.field("zona")
    md = {k.decode(): v.decode()[:120] for k, v in (f.metadata or {}).items()}
    return f"_es_geo={ore._es_geo(f)}; metadatos {md}"


# ── S4 ───────────────────────────────────────────────────────────────────────────────────
def s4(sql):
    from ore import lsp_sql
    indice = {"items": {"a": {"kind": "Dataset", "paquete": "geo", "schema": "default", "name": "lugares",
                              "expone": [{"name": "geom", "type": "Geometry<Point, EPSG:4326>"},
                                         {"name": "utm", "type": "Geometry<Point, EPSG:25830>"}]}}}
    cat = lsp_sql.Catalogo(indice)
    d = lsp_sql.diagnosticar(sql, cat)
    return [x.get("message", x)[:160] if isinstance(x, dict) else str(x)[:160] for x in d] or "sin diagnósticos"


caso("S1 nativa", s1_nativa)
caso("S1 arrow", s1_arrow)
caso("S1 BLOB", s1_blob)
caso("S1 duckdb_type del SDK", s1_sdk)
caso("S2 vista con copia", s2)
caso("S3 Transform SQL", s3)
caso("S4 mezcla de CRS", lambda: s4("select ST_Intersects(geom, utm) from geo.lugares"))
caso("S4 sin CRS", lambda: s4("select ST_Point(1, 2) as p from geo.lugares"))
caso("S4 bien", lambda: s4("select ST_Buffer(utm, 100) as z from geo.lugares"))
