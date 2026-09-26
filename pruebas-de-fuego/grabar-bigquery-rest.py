"""Graba las respuestas de la API REST de BigQuery que los tests del driver leen.

Los tests de `ore-read-bigquery` (Fase A: A2 transporte, A3 catalogo, A4 tipos)
no pueden llamar a BigQuery: CI no tiene red ni credencial. Leen estas
respuestas, grabadas de verdad, y no unas escritas a mano: una respuesta
inventada afirma lo que creemos que Google contesta, y lo que contesta se midio
distinto al menos dos veces (el TIMESTAMP por defecto es un float en notacion
cientifica; el esquema de un resultado dice NULLABLE aunque la tabla diga
REQUIRED).

Solo lee, y cada consulta lleva `maximumBytesBilled`. Necesita la semilla de
`semilla/bigquery-ventas.sql` cargada y un token:

    ORE_GCP_TOKEN=$(gcloud auth print-access-token) \\
      python pruebas-de-fuego/grabar-bigquery-rest.py <proyecto> [dataset=ventas]

Escribe en `crates/ore-read-bigquery/tests/rest/`. El token no se escribe en
ningun sitio: va en la cabecera y en ninguna respuesta.
"""
import json, os, sys, urllib.request, urllib.error, urllib.parse, pathlib

if len(sys.argv) < 2 or not os.environ.get("ORE_GCP_TOKEN"):
    sys.exit(__doc__)
P = sys.argv[1]
D = sys.argv[2] if len(sys.argv) > 2 else "ventas"
API = f"https://bigquery.googleapis.com/bigquery/v2/projects/{P}"
TOPE = str(10**9)
DESTINO = pathlib.Path(__file__).resolve().parent.parent / "crates/ore-read-bigquery/tests/rest"
DESTINO.mkdir(parents=True, exist_ok=True)


def llamar(metodo, url, cuerpo=None):
    req = urllib.request.Request(url, method=metodo, data=json.dumps(cuerpo).encode() if cuerpo else None,
                                 headers={"Authorization": "Bearer " + os.environ["ORE_GCP_TOKEN"],
                                          "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.load(e)


def guardar(nombre, estado, cuerpo, que):
    (DESTINO / f"{nombre}.json").write_text(
        json.dumps({"_que": que, "_estado": estado, "respuesta": cuerpo}, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8", newline="\n")
    print(f"  {nombre:28} {estado}")


def consulta(sql, **extra):
    return {"query": sql, "useLegacySql": False, "maximumBytesBilled": TOPE, "useQueryCache": False, **extra}


INT64 = {"formatOptions": {"useInt64Timestamp": True}}

TIPOS = r"""SELECT
 DATETIME '2026-09-26 10:11:12.123456' AS dt, TIME '23:59:59.999999' AS t, b'\x00\xffhola' AS byt,
 BIGNUMERIC '123456789012345678901234567890123456789.12345678901234567890123456789012345678' AS bn,
 NUMERIC '-99999999999999999999999999999.999999999' AS nmin, JSON '{"a":[1,2,{"b":null}],"c":"null"}' AS j,
 ST_GEOGPOINT(-8.4, 43.37) AS g, CAST('NaN' AS FLOAT64) AS nan, CAST('inf' AS FLOAT64) AS inf,
 CAST('-inf' AS FLOAT64) AS ninf, 1.0e-300 AS tiny, 0.1 AS decima, 9223372036854775807 AS i64max, TRUE AS b,
 STRUCT(1 AS a, STRUCT('x' AS c, [1,2] AS d) AS b) AS st, [STRUCT(1 AS k,'v' AS v), STRUCT(2,'w')] AS arr,
 INTERVAL 1 DAY AS iv, TIMESTAMP '0001-01-01 00:00:00+00' AS tsmin, TIMESTAMP '9999-12-31 23:59:59.999999+00' AS tsmax,
 DATE '0001-01-01' AS dmin, RANGE(DATE '2024-01-01', DATE '2024-02-01') AS rg, ARRAY<INT64>[] AS emptyarr,
 [1, 2] AS ints, '' AS vacio, 'null' AS textonull, CAST(NULL AS STRING) AS nulo"""

# La misma consulta que `lector::consulta` (ore-cli/src/lector.rs) emite hoy:
# A3 la muda al driver y tiene que seguir leyendo esta respuesta igual.
CATALOGO = pathlib.Path(__file__).with_name("grabar-bigquery-catalogo.sql").read_text(encoding="utf-8").replace("{d}", D)

print(f"grabando en {DESTINO}")
e, r = llamar("POST", f"{API}/queries", consulta(TIPOS, location="EU", **INT64))
guardar("tipos-int64", e, r, "los 26 tipos medidos, con useInt64Timestamp")
e, r = llamar("POST", f"{API}/queries", consulta(TIPOS, location="EU"))
guardar("tipos-por-defecto", e, r, "los mismos, SIN useInt64Timestamp: el TIMESTAMP como float (lo que no hay que pedir)")
for t in ("pedidos", "clientes"):
    e, r = llamar("POST", f"{API}/queries", consulta(f"SELECT * FROM {D}.{t} WHERE id LIKE 'ore-e2e-%' ORDER BY id", **INT64))
    guardar(f"{t}-query", e, r, f"la semilla de {t} por jobs.query")
    e, r = llamar("GET", f"{API}/datasets/{D}/tables/{t}")
    guardar(f"{t}-tables-get", e, r, f"tables.get de {t}: el esquema con REQUIRED, sin job")
e, r = llamar("POST", f"{API}/queries", consulta(f"SELECT id FROM {D}.pedidos WHERE id LIKE 'ore-e2e-%' ORDER BY id", maxResults=3))
guardar("pagina-1", e, r, "primera pagina de 3 filas: trae pageToken y jobReference")
job, loc = r["jobReference"]["jobId"], r["jobReference"]["location"]
e, r2 = llamar("GET", f"{API}/queries/{job}?location={loc}&maxResults=3&pageToken={urllib.parse.quote(r['pageToken'])}")
guardar("pagina-2", e, r2, "segunda pagina por getQueryResults")
e, r3 = llamar("GET", f"{API}/queries/{job}?location={loc}&maxResults=3&pageToken={urllib.parse.quote(r2['pageToken'])}")
guardar("pagina-3", e, r3, "ultima pagina: sin pageToken")
LARGA = ("SELECT COUNT(DISTINCT FARM_FINGERPRINT(CAST(x*y AS STRING))) n "
         "FROM UNNEST(GENERATE_ARRAY(1,20000)) x, UNNEST(GENERATE_ARRAY(1,3000)) y")
e, r = llamar("POST", f"{API}/queries", consulta(LARGA, location="EU", timeoutMs=1000))
guardar("larga-incompleta", e, r, "timeoutMs=1000 sobre una consulta de ~25 s: jobComplete=false")
e, r = llamar("POST", f"{API}/queries", consulta("SELEC 1", location="EU"))
guardar("error-sintaxis", e, r, "un 400 de BigQuery, con su mensaje")
e, r = llamar("GET", f"{API}/datasets?all=true")
guardar("datasets-list", e, r, "explorar: los datasets del proyecto")
e, r = llamar("POST", f"{API}/queries", consulta(CATALOGO))
guardar("catalogo", e, r, f"la receta de catalogo de lector.rs sobre {D}")
