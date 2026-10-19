"""ORE 0068 · cierre de C1 · El previo de `geometry_always_xy` en el DuckDB de los puestos (1.5.6): qué
cambia y qué no, antes de ponerlo en los tres SDK. Sin red, en `ore-puesto-python:local`:
    bash pruebas-de-fuego/geo/c1/ejes/medir.sh
Una línea por caso: lo que da sin el ajuste y con él.
"""
import duckdb

SOL_UTM = "POINT (440000 4474000)"          # la Puerta del Sol, EPSG:25830
SOL_LONLAT = "POINT (-3.7072 40.4145)"      # la misma, x = longitud (GeoParquet, Sedona)
BCN_LONLAT = "POINT (2.1734 41.3851)"       # Barcelona, x = longitud


def con():
    c = duckdb.connect()
    c.execute("set extension_directory = '/opt/ore/duckdb'")
    c.execute("set autoinstall_known_extensions = false")
    return c


def uno(c, q):
    try:
        v = c.execute(q).fetchone()[0]
        return round(v, 1) if isinstance(v, float) else v
    except Exception as e:  # noqa: BLE001
        return "FALLA " + str(e).splitlines()[0][:140]


print("duckdb", duckdb.__version__)
c = con()
print("0 · el ajuste antes de `load spatial`:", uno(c, "set geometry_always_xy = true") or "va")
c.execute("load spatial")

CASOS = [
    ("1 · ST_Transform 25830 → 4326", f"select ST_AsText(ST_Transform(ST_GeomFromText('{SOL_UTM}'), 'EPSG:25830', 'EPSG:4326'))"),
    ("2 · ST_Transform 4326 → 25830 (un lon/lat del lago)", f"select ST_AsText(ST_Transform(ST_GeomFromText('{SOL_LONLAT}'), 'EPSG:4326', 'EPSG:25830'))"),
    ("3 · ida y vuelta 25830 → 4326 → 25830", f"select ST_AsText(ST_Transform(ST_Transform(ST_GeomFromText('{SOL_UTM}'), 'EPSG:25830', 'EPSG:4326'), 'EPSG:4326', 'EPSG:25830'))"),
    ("4 · always_xy := false explícito", f"select ST_AsText(ST_Transform(ST_GeomFromText('{SOL_UTM}'), 'EPSG:25830', 'EPSG:4326', always_xy := false))"),
    ("5 · ST_Distance_Spheroid Sol–Barcelona (lon/lat), km", f"select ST_Distance_Spheroid(ST_GeomFromText('{SOL_LONLAT}'), ST_GeomFromText('{BCN_LONLAT}')) / 1000"),
    ("6 · ST_Distance_Sphere Sol–Barcelona (lon/lat), km", f"select ST_Distance_Sphere(ST_GeomFromText('{SOL_LONLAT}'), ST_GeomFromText('{BCN_LONLAT}')) / 1000"),
    ("7 · ST_X de un punto 4326 leído de WKB", f"select ST_X(ST_GeomFromWKB(ST_AsWKB(ST_GeomFromText('{SOL_LONLAT}'))))"),
    ("8 · ST_AsWKB no cambia (hex)", f"select hex(ST_AsWKB(ST_GeomFromText('{SOL_LONLAT}')))"),
    ("9 · GEOMETRY('EPSG:4326') y ST_Transform a 25830", f"select ST_AsText(ST_Transform(ST_GeomFromText('{SOL_LONLAT}')::GEOMETRY('EPSG:4326'), 'EPSG:25830'))"),
    ("10 · ST_Area_Spheroid de un cuadrado de 0,01° (lon/lat), km²", "select ST_Area_Spheroid(ST_GeomFromText('POLYGON ((-3.71 40.41, -3.70 40.41, -3.70 40.42, -3.71 40.42, -3.71 40.41))')) / 1e6"),
]
sin = {n: uno(c, q) for n, q in CASOS}
print("ajuste:", uno(c, "set geometry_always_xy = true") or "va", "· current_setting:", uno(c, "select current_setting('geometry_always_xy')"))
con_ajuste = {n: uno(c, q) for n, q in CASOS}
for n, _ in CASOS:
    print("%-58s sin: %-42s con: %s%s" % (n, sin[n], con_ajuste[n], "" if sin[n] != con_ajuste[n] else "   (igual)"))

# 11 · ¿por conexión o por base? Un cursor de la misma base, y otra base.
cur = c.cursor()
print("11 · un cursor de la misma base:", uno(cur, "select current_setting('geometry_always_xy')"))
otra = con()
otra.execute("load spatial")
print("11b · otra base:", uno(otra, "select current_setting('geometry_always_xy')"))
print("12 · reset:", uno(c, "reset geometry_always_xy") or "va", uno(c, "select current_setting('geometry_always_xy')"))
