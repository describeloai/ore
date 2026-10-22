"""ORE 0068 C2·3 pieza 2 · El SDK en Heavyweight, de punta a punta contra el /v1 del banco: los mismos
nombres (`ore.write`, `ore.over`, `ore.sql`) con `ORE_MOTOR=heavyweight`, como los deja el arnés de un
build. Corre en `ore-puesto-spark:local` (`medir.sh sdk.py`), como ana. Una línea por caso, y al
final `n/m casos`.
"""
import json
import os
import sys
import urllib.request

os.environ.update({
    "ORE_MOTOR": "heavyweight", "ORE_SERVE": "http://127.0.0.1:8080", "ORE_SUJETO": "persona:ana",
    "ORE_MEMORIA_MB": "4096",
    # lo que el arnés de un build pone (builds.rs `arnes`): la procedencia sale de aquí
    "ORE_BUILD": json.dumps({"transform": "packages/geo/pipeline/zonas.yaml", "entrypoint": "zonas.py:zonas",
                             "output": "geo.zonas", "id": "trabajo-ana-1a2b3c4d", "commit": "abc1234"}),
})
sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402

casos = []


def caso(nombre, f, espera=None):
    try:
        r = f()
        ok = espera is None or espera(r)
    except Exception as e:  # noqa: BLE001
        r, ok = "%s: %s" % (type(e).__name__, str(e).splitlines()[0][:220]), espera == "falla"
    if espera == "falla" and not isinstance(r, str):
        ok = False
    casos.append(ok)
    print("%s %s · %s" % ("OK   " if ok else "FALLA", nombre, r), flush=True)


def v1(t):
    q = urllib.request.Request("http://127.0.0.1:8080/v1/geo/namespaces/default/tables/" + t,
                               headers={"x-ore-sujeto": "persona:ana"})
    return json.load(urllib.request.urlopen(q))["metadata"]


def vigente(m):
    """El snapshot vigente (`current-snapshot-id`): el orden de `snapshots` no lo garantiza nadie."""
    return next(s for s in m["snapshots"] if s["snapshot-id"] == m["current-snapshot-id"])


from ore import spark as pesado  # noqa: E402

# La identidad de la sesión del SDK la pone el agente en un puesto; aquí, la de ana.
ore.session._cabeceras = {"x-ore-sujeto": "persona:ana"}
s = pesado.sesion()
caso("0 · el motor", lambda: (pesado.activo(), s.version, s.conf.get("spark.driver.memory")),
     lambda r: r[0] and r[2] == "2048m")

# 1 · escribir puntos de Sedona (EPSG:25830): WKB en la tabla, y ore.geo sacado de los datos
puntos = s.sql("select id, concat('p', id) nombre, ST_SetSRID(ST_Point(440000 + id * 10, 4474000), 25830) geom "
               "from range(5)")
caso("1 · write(): puntos con SRID", lambda: dict(ore.write("geo.lugares", puntos)),
     lambda r: r["rows"] == 5 and r["geo"]["geom"]["crs"] == "EPSG:25830")
caso("1b · lo que ORE ve: binary + ore.geo.geom", lambda: (
    [(f["name"], f["type"]) for f in v1("lugares")["schemas"][-1]["fields"]],
    v1("lugares")["properties"].get("ore.geo.geom")),
    lambda r: ("geom", "binary") in r[0] and json.loads(r[1]) == {"covering": "geom_bbox", "crs": "EPSG:25830",
                                                                     "type": "Geometry<Point, EPSG:25830>"})
# ORE 0072 F1·2 · la cobertura, como la escribe ore-store: la caja de cada valor, y grupos de ~100 000 filas
caso("1c · geom_bbox: la caja de cada punto", lambda: ore.over("geo.lugares").selectExpr(
    "id", "geom_bbox.xmin a", "geom_bbox.xmax b", "geom_bbox.ymin c", "geom_bbox.ymax d").orderBy("id").collect(),
    lambda r: [(x.a, x.b, x.c, x.d) for x in r] == [(440000.0 + 10 * i, 440000.0 + 10 * i, 4474000.0, 4474000.0)
                                                     for i in range(5)])
caso("1d · grupos de ~100 000 filas", lambda: v1("lugares")["properties"].get("write.parquet.row-group-size-bytes"),
     lambda r: r == str(4 * 1024 * 1024))

# 2 · leer: over() da geometría de Sedona con su SRID
caso("2 · over(): geometría con SRID", lambda: ore.over("geo.lugares").selectExpr(
    "ST_SRID(geom) s", "ST_GeometryType(geom) t").distinct().collect(),
    lambda r: len(r) == 1 and r[0].s == 25830 and r[0].t == "ST_Point")

# 3 · un transform: buffers sobre lo leído, escritos con su procedencia
def transform():
    zonas = ore.over("geo.lugares").selectExpr("id", "ST_Buffer(geom, 100) zona")
    return dict(ore.write("geo.zonas", zonas))


caso("3 · transform: buffer y write()", transform,
     lambda r: r["rows"] == 5 and r["geo"]["zona"]["type"] == "Geometry<Polygon, EPSG:25830>")
caso("3b · procedencia en el snapshot", lambda: json.loads(vigente(v1("zonas"))["summary"]["ore.procedencia"]),
     lambda r: isinstance(r, dict) and "abc1234" in json.dumps(r))
caso("3c · el área, como DuckDB (31 214 m²)", lambda: ore.over("geo.zonas").selectExpr(
    "round(ST_Area(zona)) a").distinct().collect(), lambda r: [x.a for x in r] == [31214.0])

# 4 · append: suma filas y no pierde ore.geo
caso("4 · append", lambda: dict(ore.write("geo.lugares", puntos, mode="append")),
     lambda r: r["rows"] == 10 and r["mode"] == "append")
caso("4b · ore.geo sigue", lambda: v1("lugares")["properties"].get("ore.geo.geom"), lambda r: r and "25830" in r)
# F1·2 · anexar a una tabla con cobertura la rellena también en las filas nuevas
caso("4d · append: la cobertura, también en lo anexado", lambda: ore.over("geo.lugares").where(
    "geom_bbox is null").count(), lambda r: r == 0)
# ore.modo, la palabra de ore-store: lo que ORE mira para saber si se reemplazó lo que había
caso("4c · ore.modo de cada escritura", lambda: (vigente(v1("zonas"))["summary"].get("ore.modo"),
                                                  vigente(v1("lugares"))["summary"].get("ore.modo")),
     lambda r: r == ("sobrescribir", "anexar"))

# 5 · lo que se niega
caso("5 · geometría sin CRS", lambda: ore.write("geo.mal", s.sql("select id, ST_Point(id, id) g from range(2)")), "falla")
caso("5b · dos CRS en una columna", lambda: ore.write("geo.mal", s.sql(
    "select id, ST_SetSRID(ST_Point(id, id), if(id = 0, 4326, 25830)) g from range(2)")), "falla")
caso("5c · upsert", lambda: ore.write("geo.lugares", puntos, mode="upsert", key=["id"]), "falla")
caso("5d · un pandas en Heavyweight", lambda: ore.write("geo.mal", __import__("pandas").DataFrame({"a": [1]})), "falla")


# 6 · el techo: un transform sólo escribe lo que declara (lo de siempre, antes del motor)
def fuera_del_techo():
    ore._transform = ore._Transform("zonas", ["geo.lugares"], "geo.zonas") if hasattr(ore, "_Transform") else None
    try:
        return ore.write("geo.otra", puntos)
    finally:
        ore._transform = None


caso("6 · fuera del techo", fuera_del_techo, "falla")

# 7 · sql(): Spark SQL con el catálogo ore por defecto
caso("7 · sql()", lambda: ore.sql("select count(*) n from geo.zonas").first().n, lambda r: r == 5)

# 8 · table(): la muestra del informe de un build (6c: con Spark daba None y el informe fallaba)
caso("8 · table() de un DataFrame de Spark", lambda: (lambda t: (t["total"], len(t["filas"]),
     [c["name"] for c in t["columnas"]], isinstance(t["filas"][0][1], str) and t["filas"][0][1][:2]))(
     ore.table(ore.over("geo.lugares").select("id", "geom").orderBy("id"), 3)),
     lambda r: r[0] == 10 and r[1] == 3 and r[2] == ["id", "geom"] and r[3] in ("01", "00"))
def plan_del_join(umbral):
    """C3·a · El join espacial de `geo.zonas` (5 polígonos, geo) con `geo.lugares` por `over()`: qué hace
    Spark con el umbral de filas `umbral` (sin tocar el de bytes de Spark, que difundiría igual)."""
    os.environ["ORE_DIFUNDIR_FILAS"] = str(umbral)
    s.conf.set("spark.sql.autoBroadcastJoinThreshold", "-1")
    try:
        z, p = ore.over("geo.zonas").alias("z"), ore.over("geo.lugares").alias("p")
        r = z.join(p, __import__("pyspark.sql.functions", fromlist=["expr"]).expr("ST_Contains(z.zona, p.geom)"))
        plan = r._jdf.queryExecution().executedPlan().toString()
        return ("BroadcastIndexJoin" in plan, r.count())
    finally:
        os.environ.pop("ORE_DIFUNDIR_FILAS", None)
        s.conf.unset("spark.sql.autoBroadcastJoinThreshold")


caso("9 · over() difunde un dataset geo pequeño", lambda: plan_del_join(1000), lambda r: r[0] and r[1] >= 5)
caso("9b · y uno por encima del umbral, no", lambda: plan_del_join(1), lambda r: not r[0] and r[1] >= 5)
caso("8b · table() que cabe no cuenta aparte", lambda: ore.table(ore.sql("select id from geo.zonas"), 200)["total"],
     lambda r: r == 5)

print("\n%d/%d casos" % (sum(casos), len(casos)))
