"""ADR 0067 G2·5 · La celda de BigQuery de la matriz: carga el fichero de datos que escribió el ORE de
victor (A) en `banco_geo` como Parquet, y lee lo que BigQuery entiende (tipo, filas, WKB por `id`).
Va por la API REST con el token de `gcloud` de quien lo corre (el `bq` de este PC no arranca).

    python pruebas-de-fuego/geo/v5/bigquery.py <crs> <gs://…/data/x.parquet>
Escribe la línea `V5R|bigquery|A|…` para `juzgar.py`.
"""
import json
import subprocess
import sys
import time
import urllib.request

PROYECTO = "project-8853a180-450d-47be-b83"
DATASET = "banco_geo"


def token():
    return subprocess.run("gcloud auth print-access-token", shell=True, capture_output=True, text=True).stdout.strip()


def api(metodo, ruta, cuerpo=None):
    req = urllib.request.Request("https://bigquery.googleapis.com/bigquery/v2/projects/%s/%s" % (PROYECTO, ruta),
                                 data=None if cuerpo is None else json.dumps(cuerpo).encode(), method=metodo,
                                 headers={"authorization": "Bearer " + token(), "content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        return {"error": json.loads(e.read() or b"{}").get("error", {}).get("message", str(e))}


def esperar(job):
    while True:
        j = api("GET", "jobs/%s?location=europe-west1" % job["jobReference"]["jobId"]) if "jobReference" in job else job
        if "error" in j or j.get("status", {}).get("state") == "DONE":
            return j
        time.sleep(2)


def consulta(sql):
    r = api("POST", "queries", {"query": sql, "useLegacySql": False, "timeoutMs": 60000})
    if "error" in r:
        raise RuntimeError(r["error"])
    return [[c["v"] for c in f["f"]] for f in r.get("rows", [])], r.get("schema")


if __name__ == "__main__":
    crs, uri = sys.argv[1], sys.argv[2]
    tabla = "v5_ore_" + crs.replace(":", "_").lower()
    try:
        j = esperar(api("POST", "jobs", {"configuration": {"load": {
            "sourceUris": [uri], "sourceFormat": "PARQUET", "writeDisposition": "WRITE_TRUNCATE",
            "destinationTable": {"projectId": PROYECTO, "datasetId": DATASET, "tableId": tabla},
            "parquetOptions": {"enableListInference": True}}}}))
        err = j.get("error") or j.get("status", {}).get("errorResult", {}).get("message")
        if err:
            raise RuntimeError("load: %s" % err)
        filas, esquema = consulta("select cast(id as string), to_hex(%s) from `%s.%s` order by id" % (
            "st_asbinary(geom)" if True else "geom", DATASET, tabla))
        t = api("GET", "datasets/%s/tables/%s" % (DATASET, tabla))
        tipo = [c["type"] for c in t["schema"]["fields"] if c["name"] == "geom"][0]
        # GEOGRAPHY pasa por S2: las coordenadas vuelven con deriva de ulps. Se mide cuánto, y si
        # cada geometría es la misma con 1e-9 grados (ST_EQUALS no basta: compara en la esfera).
        fijos = json.load(open(__file__.rsplit("bigquery.py", 1)[0] + "fijos.json", encoding="utf-8"))[crs]["filas"]
        valores = ", ".join("(%d, %s)" % (f["id"], "null" if f["wkb"] is None else "from_hex('%s')" % f["wkb"]) for f in fijos)
        dist, _ = consulta("with e as (select * from unnest([struct<id int64, w bytes> %s])) "
                           "select countif(e.w is not null and not st_dwithin(t.geom, st_geogfromwkb(e.w), 0.001)), "
                           "max(st_distance(t.geom, st_geogfromwkb(e.w))) from `%s.%s` t join e using (id)"
                           % (valores, DATASET, tabla))
        d = {"geo": tipo == "GEOGRAPHY", "tipo": tipo, "crs": "OGC:CRS84" if tipo == "GEOGRAPHY" else None,
             "wkb": {i: (w.upper() if w else None) for i, w in filas},
             "nota": "GEOGRAPHY (S2): %s geometrías a más de 1 mm, distancia máxima %s m" % (dist[0][0], dist[0][1])}
    except Exception as e:
        # Si la columna no es GEOGRAPHY, `st_asbinary` no aplica: el binario tal cual.
        try:
            filas, esquema = consulta("select cast(id as string), to_hex(geom) from `%s.%s` order by id" % (DATASET, tabla))
            tipo = [c["type"] for c in esquema["fields"] if c["name"] == "geom"][0]
            d = {"geo": False, "tipo": tipo, "crs": None, "wkb": {i: (w.upper() if w else None) for i, w in filas},
                 "nota": str(e)[:200]}
        except Exception as e2:
            d = {"error": "%s / %s" % (str(e)[:250], str(e2)[:150])}
    print("V5R|bigquery|A|" + json.dumps(d, ensure_ascii=False), flush=True)
