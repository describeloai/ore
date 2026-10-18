"""ORE 0068 C2·3 pieza 3 · `/v1` comprueba: contra el banco de C2·4 (ore-serve con lo de la pieza 3),
desde `ore-puesto-spark:local` en su red, como ana. Una línea `OK|FALLA · caso · detalle`, y al final
`n/m casos`.
    bash pruebas-de-fuego/geo/c2/v1-comprueba/medir.sh
"""
import json
import os
import sys
import urllib.request

os.environ.update({"ORE_MOTOR": "heavyweight", "ORE_SERVE": "http://127.0.0.1:8080",
                   "ORE_SUJETO": "persona:ana", "ORE_MEMORIA_MB": "4096"})
sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402
from ore import spark as pesado  # noqa: E402

ore.session._cabeceras = {"x-ore-sujeto": "persona:ana"}
s = pesado.sesion()
BASE = "http://127.0.0.1:8080"
casos = []


def caso(nombre, f, ok):
    try:
        r = f()
    except Exception as e:  # noqa: BLE001
        l = [x.strip() for x in str(e).splitlines() if x.strip() and not x.strip().startswith("at ")]
        r = "EXC " + (([x for x in l if "Exception" in x] or l or ["?"])[0])[:300]
    bien = bool(ok(r))
    casos.append(bien)
    print("%s · %s · %s" % ("OK   " if bien else "FALLA", nombre, r), flush=True)


def pedir(ruta):
    q = urllib.request.Request(BASE + ruta, headers={"x-ore-sujeto": "persona:ana",
                                                     "X-Iceberg-Access-Delegation": "vended-credentials"})
    try:
        with urllib.request.urlopen(q) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


ore.write("geo.lugares", s.sql("select id, ST_SetSRID(ST_Point(440000 + id, 4474000), 25830) geom from range(3)"))

# ── 1 · S6: el GEOMETRY nativo de Spark (Iceberg v3) → 400 que dice cómo ─────────
s.sql("set spark.sql.geospatial.enabled=true")
caso("1 · CREATE TABLE … GEOMETRY(4326) v3", lambda: s.sql(
    "create table ore.geo.nativa (id bigint, g geometry(4326)) using iceberg tblproperties ('format-version'='3')").collect(),
    lambda r: isinstance(r, str) and "BadRequest" in r and "ST_AsBinary" in r)
caso("1b · y no queda nada", lambda: pedir("/v1/namespaces/geo/tables/nativa")[0], lambda r: r == 404)

# ── 2 · ore.geo comprobada en la puerta ──────────────────────────────────────────
for que, valor, espera in [
    ("2 · ore.geo que no es JSON", "no es json", "no es JSON"),
    ("2b · ore.geo sin crs", '{"type":"Geometry<Point, EPSG:25830>"}', "le falta `crs`"),
    ("2c · ore.geo con el CRS contradicho", '{"crs":"EPSG:4326","type":"Geometry<Point, EPSG:25830>"}', "una columna, un CRS"),
]:
    caso(que, lambda v=valor: s.sql("create table ore.geo.mala (id bigint, g binary) using iceberg "
                                    "tblproperties ('ore.geo.g' = '%s')" % v.replace("'", "\\'")).collect(),
         lambda r, e=espera: isinstance(r, str) and "BadRequest" in r and e in r)
caso("2d · ore.geo que se sostiene", lambda: (s.sql(
    "create table ore.geo.buena (id bigint, g binary) using iceberg tblproperties "
    "('ore.geo.g' = '{\"crs\":\"EPSG:25830\",\"type\":\"Geometry<Point, EPSG:25830>\"}')").collect(), "creada")[1],
    lambda r: r == "creada")
caso("2e · un ALTER que la estropea", lambda: s.sql(
    "alter table ore.geo.buena set tblproperties ('ore.geo.g' = '{\"crs\":\"25830\"}')").collect(),
    lambda r: isinstance(r, str) and "no es un CRS" in r)

# ── 3 · las tablas de metadatos de Spark (un namespace de dos niveles → 404) ─────
caso("3 · select … from ….snapshots", lambda: s.sql(
    "select count(*) n from ore_geo.default.lugares.snapshots").first().n if pesado._tabla("geo.lugares") else None,
    lambda r: isinstance(r, int) and r >= 1)
caso("3b · el 404 de dos niveles, dicho", lambda: pedir("/v1/namespaces/default%1Flugares/tables/snapshots"),
     lambda r: r[0] == 404 and r[1]["error"]["type"] == "NoSuchTableException")

# ── 4 · el refresco de la credencial ─────────────────────────────────────────────
c, lt = pedir("/v1/geo/namespaces/default/tables/lugares")
cfg = lt.get("config", {}) if c == 200 else {}
# Las claves fijas del S3 del banco (las de R2) no caducan: no se anuncia refresco (y S3FileIO, con él,
# exigiría un `session-token` que no tienen). El de GCS, que caduca, sólo se ve en victor.
caso("4 · loadTable no anuncia refresco para lo que no caduca", lambda: {k: v for k, v in cfg.items() if "refresh" in k},
     lambda r: r == {})
caso("4b · GET …/credentials (loadCredentials)", lambda: pedir("/v1/geo/namespaces/default/tables/lugares/credentials"),
     lambda r: r[0] == 200 and r[1]["storage-credentials"][0]["config"].get("s3.access-key-id")
     == cfg.get("s3.access-key-id") and "metadata" not in r[1])


def proveedor():
    """El VendedCredentialsProvider de Iceberg (el que S3FileIO usa para refrescar), con las
    propiedades que tendría su FileIO: las del catálogo (su `uri` y su identidad) y el endpoint
    (`credentials.uri`, a la que S3FileIO traduce la anunciada). Pide el endpoint de verdad; con las
    claves fijas del banco, lo que devuelve no le vale como temporal (sin `session-token`), y eso
    —y no un 401, 403 o 404— es lo que tiene que decir."""
    jvm = s._jvm
    m = jvm.java.util.HashMap()
    for k, v in {"uri": BASE, "header.x-ore-sujeto": "persona:ana",
                 "credentials.uri": "v1/geo/namespaces/default/tables/lugares/credentials"}.items():
        m.put(k, v)
    p = jvm.org.apache.iceberg.aws.s3.VendedCredentialsProvider.create(m)
    return p.resolveCredentials().accessKeyId()


caso("4c · Iceberg pide el endpoint con la identidad del catálogo, y lo lee", proveedor,
     lambda r: isinstance(r, str) and "session-token" in r)
print("\n%d/%d casos" % (sum(casos), len(casos)))
