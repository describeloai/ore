"""ORE 0068 · cierre de C1 · Los ejes por la API del SDK de Python (`ore.sql`), sin servidor: un punto
de EPSG:25830 a 4326 sale lon/lat, y la distancia Sol–Barcelona desde lon/lat es la de verdad."""
import sys

sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402

p = ore.sql("select ST_AsText(ST_Transform(ST_Point(440000, 4474000), 'EPSG:25830', 'EPSG:4326')) p").iloc[0, 0]
d = ore.sql("select ST_Distance_Spheroid(ST_Point(-3.7072, 40.4145), ST_Point(2.1734, 41.3851)) / 1000 d").iloc[0, 0]
ok = p.startswith("POINT (-3.70") and 500 < d < 515
print("%s · python · %s · Sol–Barcelona %.1f km" % ("OK   " if ok else "FALLA", p, d))
