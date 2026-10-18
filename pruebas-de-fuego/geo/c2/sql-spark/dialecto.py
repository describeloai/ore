"""ORE 0068 C2·3 2b·0 · El SQL como se escribe en ORE (DuckDB), corrido por Spark sobre las vistas que
registra `over()`: lo que la reescritura de 2b dará. Corre en `ore-puesto-spark:local` en la red del
banco (`medir.sh`), como ana. Primero escribe dos tablas geo con el SDK pesado; después, cada caso es
una sentencia escrita como en DuckDB, ya reescrita (`geo.lugares` → `ore__geo__default__lugares`).
Una línea `va|NO · caso · resultado`: lo que no va es la tabla de diferencias del ADR, no un fallo.
"""
import json
import os
import sys

os.environ.update({"ORE_MOTOR": "heavyweight", "ORE_SERVE": "http://127.0.0.1:8080",
                   "ORE_SUJETO": "persona:ana", "ORE_MEMORIA_MB": "4096"})
sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402
from ore import spark as pesado  # noqa: E402

ore.session._cabeceras = {"x-ore-sujeto": "persona:ana"}
s = pesado.sesion()

# las dos tablas (lo que haría un build anterior)
ore.write("geo.lugares", s.sql("select id, concat('p', id) nombre, "
                               "ST_SetSRID(ST_Point(440000 + id * 40, 4474000 + (id % 3) * 40), 25830) geom from range(30)"))
ore.write("geo.zonas", s.sql("select id, ST_SetSRID(ST_Buffer(ST_Point(440000 + id * 400, 4474000), 150), 25830) zona "
                             "from range(4)"))
# ⭐ lo que la celda de 2b·2 hará con cada nombre que la sentencia lee
for n in ("geo.lugares", "geo.zonas"):
    b, sch, t = ore._partes(ore._corto(n))
    ore.over(n).createOrReplaceTempView("ore__%s__%s__%s" % (b, sch, t))

L, Z = "ore__geo__default__lugares", "ore__geo__default__zonas"
CASOS = [
    ("join espacial (ST_Contains) con group by",
     f"select z.id, count(*) n from {Z} z join {L} l on ST_Contains(z.zona, l.geom) group by z.id order by z.id"),
    ("el SRID llega por la vista", f"select distinct ST_SRID(geom) s from {L}"),
    ("ST_DWithin", f"select count(*) n from {L} a join {L} b on a.id < b.id and ST_DWithin(a.geom, b.geom, 50)"),
    ("ST_Distance", f"select round(ST_Distance(a.geom, b.geom), 1) d from {L} a, {L} b where a.id = 0 and b.id = 1"),
    ("ST_Transform con 3 argumentos (DuckDB)", f"select ST_AsText(ST_Transform(geom, 'EPSG:25830', 'EPSG:4326')) t from {L} where id = 0"),
    ("ST_Transform con 2 (el SRID ya está)", f"select ST_AsText(ST_Transform(geom, 'EPSG:4326')) t from {L} where id = 0"),
    ("ST_Area, ST_Buffer, ST_Intersects", f"select round(sum(ST_Area(ST_Buffer(geom, 10)))) a from {L} where ST_Intersects(geom, geom)"),
    ("ST_Union_Agg (DuckDB) ", f"select ST_GeometryType(ST_Union_Agg(zona)) t from {Z}"),
    ("ST_Union_Aggr (Sedona)", f"select ST_GeometryType(ST_Union_Aggr(zona)) t from {Z}"),
    ("ST_X / ST_Y / ST_AsText / ST_GeomFromText", f"select ST_X(geom) x, ST_Y(geom) y from {L} where id = 1"),
    ("cast con :: ", f"select id::double d from {L} where id = 2"),
    ("ilike", f"select count(*) n from {L} where nombre ilike 'P1%'"),
    ("count(*) filter (where …)", f"select count(*) filter (where id > 10) n from {L}"),
    ("qualify", f"select id from {L} qualify row_number() over (order by id desc) = 1"),
    ("select * exclude (…) (DuckDB)", f"select * exclude (geom) from {L} limit 1"),
    ("select * except (…) (Spark)", f"select * except (geom) from {L} limit 1"),
    ("list(…) (DuckDB)", f"select size(list(id)) n from {L}"),
    ("array_agg", f"select size(array_agg(id)) n from {L}"),
    ("h3 de DuckDB: h3_latlng_to_cell", f"select h3_latlng_to_cell(40.4, -3.7, 9) c"),
    ("h3 de Sedona: ST_H3CellIDs", f"select ST_H3CellIDs(ST_Transform(geom, 'EPSG:4326'), 9, false)[0] c from {L} where id = 0"),
    ("string_agg (DuckDB)", f"select string_agg(nombre, ',') s from {L} where id < 3"),
    ("date_trunc / current_date", "select date_trunc('month', current_date) m"),
]
va = 0
for que, q in CASOS:
    try:
        r = s.sql(q).collect()
        texto, ok = str([tuple(x) for x in r][:4]), True
    except Exception as e:  # noqa: BLE001
        l = [x.strip() for x in str(e).splitlines() if x.strip()]
        texto, ok = (l[0] if l else "?")[:200], False
    va += ok
    print("%s · %s · %s" % ("va" if ok else "NO", que, texto), flush=True)
print("\n%d/%d van tal cual" % (va, len(CASOS)))
print(json.dumps({"dialecto": "fin"}))
