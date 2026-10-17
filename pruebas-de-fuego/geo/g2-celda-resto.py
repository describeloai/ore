# ADR 0067 · Lo que faltó de g2-celda.py en la segunda pasada: H3 y V5. Usa las tablas que ya dejó.
PAQUETE = "test_project"

import glob, json, os, subprocess, sys, time
import ore
from google.cloud import storage

BANCO = "project-8853a180-450d-47be-b83-banco-geo"
os.makedirs("/tmp/g2/m", exist_ok=True)
for b in storage.Client().list_blobs(BANCO, prefix="g2/"):
    b.download_to_filename("/tmp/g2/m/" + b.name.split("/")[-1])
def di(que, **kw): print("G2|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)

# H3 (con `load h3` a mano: el SDK de esta imagen todavía no lo carga solo).
try:
    ore._duckdb().execute("load h3")
    h = ore.sql("select h3_latlng_to_cell(ST_Y(p), ST_X(p), 8) celda, count(*) n from "
                "(select ST_Transform(geom, 'EPSG:25830', 'EPSG:4326', true) p from %s.medida_g2_puntos) group by 1" % PAQUETE)
    di("g235_h3", ok=True, celdas=len(h), filas=int(h["n"].sum()))
except Exception as e:
    di("g235_h3", ok=False, error=str(e)[:300])

# V5: bajar cada tabla de fijos (con la credencial prestada, por https), promoverla y leer A, B y C.
def correr(*a, entrada=None):
    p = subprocess.run([sys.executable, *a], input=entrada, capture_output=True, text=True)
    return p.stdout + (("\n" + p.stderr[-300:]) if p.stderr.strip() else "")
for crs in ("EPSG:25830", "OGC:CRS84"):
    d = crs.replace(":", "-")
    vista = PAQUETE + ".medida_g2_v5_" + d.lower().replace("-", "_")
    raiz = "/tmp/g2/v5/" + d
    try:
        _, r = ore._fuente_de(vista)
        url = r["metadata_location"].rsplit("/metadata/", 1)[0].replace("gs://", "https://storage.googleapis.com/")
        n = 0
        for ruta, contenido in ore._duckdb().execute("select filename, content from read_blob('%s/**')" % url).fetchall():
            p = raiz + "/" + ruta[len(url) + 1:]
            os.makedirs(os.path.dirname(p), exist_ok=True); open(p, "wb").write(contenido); n += 1
        di("v5_bajar_" + d, ok=True, ficheros=n, geo_del_puntero=r.get("geo"))
    except Exception as e:
        di("v5_bajar_" + d, ok=False, error=str(e)[:300]); continue
    v0 = os.path.basename(sorted(glob.glob(raiz + "/metadata/*.metadata.json"))[-1])[: -len(".metadata.json")]
    correr("/tmp/g2/m/promover.py", raiz)
    v1 = os.path.basename(sorted(glob.glob(raiz + "/metadata/*.metadata.json"))[-1])[: -len(".metadata.json")]
    a = (glob.glob(raiz + "/data/*.parquet") or [""])[0]
    args = ["A=" + a, "B=%s@%s" % (raiz, v0), "C=%s@%s" % (raiz, v1)]
    lineas = "".join(correr("/tmp/g2/m/lectores.py", l, "/tmp/g2/m/fijos.json", *args)
                     for l in ("duckdb", "duckdb-spatial", "pyarrow", "geopandas"))
    print(correr("/tmp/g2/m/juzgar.py", "/tmp/g2/m/fijos.json", crs, entrada=lineas))
di("fin")
