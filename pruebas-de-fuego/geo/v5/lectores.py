"""ADR 0067 G2·5 · Los lectores de Python de la matriz de adopción, los mismos en local y en victor.

    python lectores.py <lector> <fijos.json> <artefacto>=<ruta> [...]

Artefactos:
  A=<fichero.parquet>             el fichero de datos que escribe ORE
  B=<raíz de la tabla>@<versión>  la tabla v3 de hoy (`binary` + `ore.geo.<col>`)
  C=<raíz de la tabla>@<versión>  la misma, promovida solo en metadatos a `geometry`
  X=<raíz de la tabla>@<versión>  una tabla v3 ajena con `geometry` (Iceberg Java 1.12)

Cada lector deja una línea `V5R|<lector>|<artefacto>|<json>` con lo que vio (`geo`: si la columna llega
como geometría; `crs`; `wkb`: el hex por `id`), y `juzgar.py` la compara con lo esperado. Un lector que
no aplica a un artefacto no escribe nada; uno que falla escribe `{"error": …}`.
"""
import json
import sys
import traceback


def salida(lector, art, d):
    print("V5R|%s|%s|%s" % (lector, art, json.dumps(d, ensure_ascii=False, default=str)), flush=True)


def con_duckdb(spatial):
    import duckdb, os
    # En la imagen del puesto las extensiones están preinstaladas (y no hay red).
    d = "/opt/ore/duckdb" if os.path.isdir("/opt/ore/duckdb") else os.path.expanduser("~/.duckdb/extensions")
    c = duckdb.connect(config={"extension_directory": d, "home_directory": "/tmp"})
    for e in (["spatial"] if spatial else []) + ["iceberg"]:
        try:
            c.execute("load " + e)
        except Exception:
            c.execute("install " + e)
            c.execute("load " + e)
    return c


def fuente_duck(art, ruta):
    if art == "A":
        return "read_parquet('%s')" % ruta
    raiz, version = ruta.rsplit("@", 1)
    return "iceberg_scan('%s', version='%s', allow_moved_paths=true)" % (raiz, version)


def duckdb_lector(spatial):
    def leer(art, ruta):
        c = con_duckdb(spatial)
        f = fuente_duck(art, ruta)
        tipo = c.execute("select typeof(geom) from %s limit 1" % f).fetchone()[0]
        geo = tipo.upper().startswith("GEOMETRY")
        if geo and spatial:
            q = "select id, case when geom is null then null else hex(ST_AsWKB(geom)) end from %s" % f
        elif geo:
            # Sin `spatial`, DuckDB 1.5 tiene el tipo pero no lo convierte a WKB.
            n = c.execute("select count(*) from %s" % f).fetchone()[0]
            crs = tipo[tipo.index("(") + 2:-2] if "(" in tipo else None
            return {"geo": True, "tipo": tipo, "crs": crs, "wkb": None, "filas": n,
                    "nota": "sin `spatial` no hay ST_AsWKB: el valor no se puede sacar"}
        else:
            q = "select id, case when geom is null then null else hex(geom) end from %s" % f
        wkb = {str(i): w for i, w in c.execute(q).fetchall()}
        crs = tipo[tipo.index("(") + 2:-2] if "(" in tipo else None
        return {"geo": geo, "tipo": tipo, "crs": crs, "wkb": wkb}
    return leer


def pyarrow_lector(art, ruta):
    if art != "A":
        return None
    import pyarrow.parquet as pq
    pf = pq.ParquetFile(ruta)
    t = pf.read()
    campo = t.schema.field("geom")
    meta = {k.decode(): v.decode() for k, v in (campo.metadata or {}).items()}
    logico = str(pf.metadata.schema.column(t.schema.get_field_index("geom")).logical_type)
    crs = None
    ext = meta.get("ARROW:extension:metadata")
    if ext:
        crs = json.loads(ext).get("crs")
        if isinstance(crs, dict):
            i = crs.get("id") or {}
            crs = "%s:%s" % (i.get("authority"), i.get("code")) if i else "PROJJSON sin id"
    ids = t.column("id").to_pylist()
    wkb = {str(i): (None if w is None else w.hex().upper()) for i, w in zip(ids, t.column("geom").to_pylist())}
    geo = meta.get("ARROW:extension:name") == "geoarrow.wkb" or "Geometry" in logico
    return {"geo": geo, "tipo": "%s · %s · %s" % (campo.type, meta.get("ARROW:extension:name"), logico), "crs": crs, "wkb": wkb}


def geopandas_lector(art, ruta):
    if art != "A":
        return None
    import geopandas
    g = geopandas.read_parquet(ruta)
    crs = g.crs.to_string() if g.crs is not None else None
    wkb = {str(i): (None if v is None else v.wkb_hex) for i, v in zip(g["id"], g.geometry)}
    return {"geo": isinstance(g, geopandas.GeoDataFrame) and g.geometry.name == "geom", "tipo": type(g).__name__, "crs": crs, "wkb": wkb}


def gdal_lector(art, ruta):
    if art != "A":
        return None
    from osgeo import gdal, ogr
    gdal.UseExceptions()
    ds = ogr.Open(ruta)
    capa = ds.GetLayer(0)
    srs = capa.GetSpatialRef()
    crs = None
    if srs is not None:
        srs.AutoIdentifyEPSG()
        a, c = srs.GetAuthorityName(None), srs.GetAuthorityCode(None)
        crs = "%s:%s" % (a, c) if a else srs.GetName()
    wkb = {}
    for f in capa:
        g = f.GetGeometryRef()
        wkb[str(f.GetField("id"))] = None if g is None else bytes(g.ExportToIsoWkb()).hex().upper()
    return {"geo": capa.GetGeomType() != ogr.wkbNone, "tipo": ogr.GeometryTypeToName(capa.GetGeomType()), "crs": crs, "wkb": wkb}


LECTORES = {
    "duckdb": duckdb_lector(False),
    "duckdb-spatial": duckdb_lector(True),
    "pyarrow": pyarrow_lector,
    "geopandas": geopandas_lector,
    "gdal": gdal_lector,
}

if __name__ == "__main__":
    lector = sys.argv[1]
    for a in sys.argv[3:]:
        art, ruta = a.split("=", 1)
        try:
            d = LECTORES[lector](art, ruta)
        except Exception as e:
            d = {"error": "%s: %s" % (type(e).__name__, str(e).replace("\n", " ")[:300]),
                 "donde": traceback.format_exc().strip().splitlines()[-2][:200]}
        if d is not None:
            salida(lector, art, d)
