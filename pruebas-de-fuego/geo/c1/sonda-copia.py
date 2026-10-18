"""ORE 0068 C1·1 pieza 2 · La vista con copia: Arrow binario con la marca geoarrow.wkb (como lo
dejaría `volcar` con el CRS de `ore.geo.<col>`) registrado en DuckDB, y lo que sale para sellar."""
import json
import duckdb, pyarrow as pa
c = duckdb.connect(); c.execute("set extension_directory = '/opt/ore/duckdb'")
wkb = duckdb.connect()
wkb.execute("set extension_directory = '/opt/ore/duckdb'"); wkb.execute("load spatial")
punto = wkb.sql("select ST_AsWKB(ST_Point(440000, 4474000))").fetchone()[0]
for meta in ({"crs": "EPSG:25830"}, {"crs": "OGC:CRS84"}, {}):
    campo = pa.field("geom", pa.binary(), metadata={b"ARROW:extension:name": b"geoarrow.wkb",
                                                     b"ARROW:extension:metadata": json.dumps(meta).encode()})
    t = pa.table({"id": [1], "geom": pa.array([punto], pa.binary())}, schema=pa.schema([pa.field("id", pa.int64()), campo]))
    for geo in (False, True):
        k = duckdb.connect(); k.execute("set extension_directory = '/opt/ore/duckdb'")
        if geo:
            k.execute("load spatial")
        k.register("t", t)
        try:
            tipo = k.sql("select typeof(geom) from t").fetchone()[0]
        except Exception as e:
            tipo = "FALLA " + str(e).splitlines()[0][:100]
        extra = ""
        if geo:
            try:
                r = k.sql("select ST_Intersects(geom, ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830')) from t").fetchone()
                extra = " · casa con 'EPSG:25830': %s" % r[0]
            except Exception as e:
                extra = " · con 'EPSG:25830': " + str(e).splitlines()[0][:110]
            try:
                out = k.sql("select ST_Buffer(geom, 10) as z from t").arrow().schema.field("z")
                md = dict(out.metadata or {})
                extra += " · salida %s %s" % (md.get(b"ARROW:extension:name"), md.get(b"ARROW:extension:metadata", b"")[:60])
            except Exception as e:
                extra += " · salida: " + str(e).splitlines()[0][:80]
        print(json.dumps(meta), "spatial" if geo else "sin spatial", "→", tipo[:60], extra)
