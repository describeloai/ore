"""ORE 0072 F2·3 · El banco en victor: `ore-teselas` (la imagen de `main`, con los límites de la malla: 1 CPU,
2 GiB) como contenedor de al lado en el mismo pod, y esta carga hablándole por localhost, C3 v1 tal cual
(lo que `ore-serve` le manda), contra los datos REALES del lago de victor en GCS:

  · `query.default.trip_3m_puntos` y `trip_3m_lineas` (3 M cada uno, NYC), por `/v1` con credencial prestada;
  · los edificios del banco en la comunidad de Madrid (~1,33 M polígonos), escritos al lago SIN catálogo como
    F1·3 (`stage-create` + `ore-store aplicar`: el agente no crea datasets, 0052); se borran después a mano.

Dos rondas por dataset y zoom (13–16):
  · **aislada**: las teselas de una vista de una en una con plazo de 60 s: lo que cuesta cada una y en qué fase
    (`x-ore-cuenta`), sin cola;
  · **mapa**: las teselas de una vista de 1600×900 (tesela de 512 px, como MapLibre) TODAS A LA VEZ, con el plazo
    de producción (10 s): lo que vive quien mira (cuántas salen, p50/p95, cuánto tarda la vista entera).

Con `BANCO_PIRAMIDE=1` (F2·3·2·1), las mismas rondas sobre **la pirámide**: los PMTiles de la prueba de concepto
en el bucket del banco (`piramide/*.pmtiles`), con la credencial del pod, en z9–12 (`BANCO_ZOOMS_PIRAMIDE`).

Con `BANCO_INDICE=1` (0072 I3·4), **el índice de la capa**: `trip_30m` como la sirve el descriptor con índice —el
motor encuentra el manifiesto (`/v1/indice`) y la tesela lee el índice (`geom` en 3857, `_ore_id`)—, en z13–16 con
la credencial que presta el catálogo para la tabla (el índice vive debajo); el clic (`/v1/fila`) de unas filas; y,
para comparar, unas teselas z16 aisladas del crudo (la tabla tal cual).

Emite «BANCO|…». Lo lanza `banco.py` (el Job con el motor de al lado).
"""
import json
import math
import os
import subprocess
import sys
import threading
import time
import traceback
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402

MOTOR = "http://127.0.0.1:8099"
BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
ZOOMS = [int(z) for z in os.environ.get("BANCO_ZOOMS", "13,14,15,16").split(",")]
# las líneas leen casi todo el fichero en cualquier zoom (sus grupos cubren NYC entera): dos zooms bastan
ZOOMS_LINEAS = [int(z) for z in os.environ.get("BANCO_ZOOMS_LINEAS", "13,16").split(",")]
DELEGAR = {"x-iceberg-access-delegation": "vended-credentials"}
ZOOMS_PIRAMIDE = [int(z) for z in os.environ.get("BANCO_ZOOMS_PIRAMIDE", "9,10,11,12").split(",")]


def di(que, **kw):
    print("BANCO|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


def testigo():
    d = os.environ["DIRECCION"].rstrip("/")
    realm = os.environ.get("REALM", "rubix")
    cli = open("/puesto/agente-cliente").read().strip()
    sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % realm, data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}


ore.session._cabeceras = testigo()
ore.session.id = ""


# ── las fuentes ──────────────────────────────────────────────────────────────────────────────────
def del_catalogo(base, schema, nombre):
    """`(metadata_location, credencial)` de un dataset del catálogo, como un cliente Iceberg."""
    ore.session._cabeceras = testigo()
    c, r = ore.session.pedir("GET", "/v1/%s/namespaces/%s/tables/%s" % (base, schema, nombre), cabeceras=DELEGAR)
    if c != 200:
        raise RuntimeError("loadTable %s.%s.%s: %s %s" % (base, schema, nombre, c, str(r)[:300]))
    return r["metadata-location"], r.get("config", {})


_pedir = ore.session.pedir
LOCAL, STAGED, LOCS = {}, {}, []


def pedir(metodo, ruta, cuerpo=None, *a, **kw):
    """Como F1·3: `f2b_*` no pasa por el catálogo; `stage-create` presta la credencial y `ore-store aplicar`
    escribe el `metadata.json`."""
    if "/tables/f2b_" in ruta:
        if metodo == "GET":
            if ruta in LOCAL:
                l = LOCAL[ruta]
                return 200, {"metadata-location": l["ml"], "config": l["config"], "metadata": {"location": l["loc"]}}
            return 404, {"error": {"message": "sin crear"}}
        if metodo == "POST":
            cfg, loc = (LOCAL[ruta]["config"], LOCAL[ruta]["loc"]) if ruta in LOCAL else STAGED[ruta]
            binario, env = ore._ore_store(cfg, loc)
            pet = {"dataset": "medida/" + ruta.rsplit("/", 1)[1], "requirements": cuerpo["requirements"],
                   "updates": cuerpo["updates"]}
            if ruta in LOCAL:
                pet["metadata_location"] = LOCAL[ruta]["ml"]
            p = subprocess.run([binario, "aplicar"], input=json.dumps(pet).encode() + b"\n", capture_output=True, env=env)
            if p.returncode != 0:
                return 400, {"error": {"message": "aplicar: " + p.stderr.decode("utf-8", "replace")[-400:]}}
            out = json.loads(p.stdout.decode("utf-8"))
            ml = out.get("metadata_location") or out.get("metadata-location")
            LOCAL[ruta] = {"ml": ml, "config": cfg, "loc": loc}
            return 200, {"metadata-location": ml, "metadata": {"current-snapshot-id": out.get("snapshot")}}
    if metodo == "POST" and ruta.endswith("/tables") and isinstance(cuerpo, dict) and cuerpo.get("stage-create") \
            and str(cuerpo.get("name", "")).startswith("f2b_"):
        c, r = _pedir(metodo, ruta, cuerpo, *a, **kw)
        if c == 200:
            STAGED[ruta + "/" + cuerpo["name"]] = (r.get("config", {}), r["metadata"]["location"])
            LOCS.append(r["metadata"]["location"])
        return c, r
    return _pedir(metodo, ruta, cuerpo, *a, **kw)


def edificios():
    """Los edificios del banco en la comunidad de Madrid, al lago sin catálogo."""
    ore.session.pedir = pedir
    try:
        _, r = ore.session.pedir("GET", "/v1/namespaces")
        ns = sorted(n[0] for n in r["namespaces"])[0]
        con = ore._duckdb()
        for e in ("spatial", "httpfs"):
            con.execute("load " + e)
        con.execute("set geometry_always_xy = true")
        con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (
            ore._token_de_google().replace("'", "''"), BANCO))
        t0 = time.time()
        t = con.execute(f"""select row_number() over () id, ST_SetCRS(geom::GEOMETRY, 'EPSG:4326') geom
                            from read_parquet('{BANCO}/g22/a_tal_cual.parquet')
                            where ST_Intersects(geom, ST_MakeEnvelope(-4.6, 39.8, -2.8, 41.0))""").fetch_arrow_table()
        leer = time.time() - t0
        ore.session._cabeceras = testigo()
        e = ore.write(ns + ".f2b_edificios", t)
        ruta = [k for k in LOCAL if k.endswith("/f2b_edificios")][0]
        di("edificios", filas=t.num_rows, leer_s=round(leer, 1), escribir_s=round(time.time() - t0 - leer, 1),
           ml=e["metadata_location"], borrar=LOCS)
        return e["metadata_location"], LOCAL[ruta]["config"]
    finally:
        ore.session.pedir = _pedir


# ── las teselas ──────────────────────────────────────────────────────────────────────────────────
def tesela_de(lon, lat, z):
    n = 2 ** z
    return ((lon + 180) / 360 * n, (1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def vista(lon, lat, z, ancho=1600, alto=900):
    """Las teselas que MapLibre pide para una vista (tesela de 512 px): las que cubren el rectángulo."""
    cx, cy = tesela_de(lon, lat, z)
    mx, my = ancho / 512 / 2, alto / 512 / 2
    return [(z, x, y) for x in range(math.floor(cx - mx), math.floor(cx + mx) + 1)
            for y in range(math.floor(cy - my), math.floor(cy + my) + 1)]


def pedir_tesela(fuente, z, x, y, plazo_ms):
    """`fuente`: `(metadata_location, credencial[, pirámide[, lo que cambia de la fuente]])`."""
    ml, cred = fuente[:2]
    cuerpo = {"protocolo": "1", "tms": "WebMercatorQuad", "z": str(z), "x": str(x), "y": str(y), "formato": "mvt",
              "capa": "banco", "fuente": {"metadata_location": ml, "geometria": "geom", "crs": "EPSG:4326", "id": "id"},
              "limites": {"plazo_ms": str(plazo_ms), "max_bytes": "2000000"},
              "credencial": {k: str(v) for k, v in cred.items()}}
    if len(fuente) > 2 and fuente[2]:
        cuerpo["fuente"]["piramide"] = fuente[2]
    if len(fuente) > 3:
        cuerpo["fuente"].update(fuente[3])
    pet = urllib.request.Request(MOTOR + "/v1/teselas", data=json.dumps(cuerpo).encode(), method="POST",
                                 headers={"content-type": "application/json"})
    t0 = time.time()
    try:
        r = urllib.request.urlopen(pet, timeout=plazo_ms / 1000 + 40)
        b = r.read()
        return {"codigo": r.status, "ms": round((time.time() - t0) * 1000), "kb": round(len(b) / 1024, 1),
                "cuenta": json.loads(r.headers.get("x-ore-cuenta") or "{}")}
    except urllib.error.HTTPError as e:
        cuerpo = e.read().decode("utf-8", "replace")
        return {"codigo": e.code, "ms": round((time.time() - t0) * 1000), "motivo": cuerpo[:160]}
    except Exception as e:  # noqa: BLE001
        return {"codigo": 0, "ms": round((time.time() - t0) * 1000), "motivo": "%s: %s" % (type(e).__name__, e)}


def pct(v, p):
    v = sorted(v)
    return v[min(len(v) - 1, int(round(p * (len(v) - 1))))] if v else None


def resumen(rs):
    ok = [r for r in rs if r["codigo"] in (200, 204)]
    cu = [r["cuenta"] for r in ok if r.get("cuenta")]
    med = lambda k: pct([c.get(k, 0) for c in cu], 0.5)  # noqa: E731
    return {"teselas": len(rs), "ok": len(ok),
            "fallos": sorted({"%s %s" % (r["codigo"], (r.get("motivo") or "")[:60]) for r in rs if r not in ok}),
            "p50_ms": pct([r["ms"] for r in ok], 0.5), "p95_ms": pct([r["ms"] for r in ok], 0.95),
            "max_ms": max([r["ms"] for r in ok], default=None), "kb_max": max([r.get("kb", 0) for r in ok], default=None),
            "mediana": {k: med(k) for k in ("listar_ms", "leer_ms", "turno_ms", "geom_ms", "codificar_ms", "grupos",
                                            "grupos_leidos", "filas_en_grupos", "filas_en_paginas", "filas",
                                            "elementos", "peticiones", "oleadas", "bytes", "en_cache",
                                            "compartidas", "piramide")}}


def ronda(nombre, fuente, centro, zooms=None):
    for z in zooms or (ZOOMS_LINEAS if nombre == "trip_3m_lineas" else ZOOMS):
        ts = vista(centro[0], centro[1], z)
        # aislada: de una en una, plazo largo (cuánto cuesta cada tesela, sin cola)
        rs = [pedir_tesela(fuente, *t, 60_000) for t in ts]
        di("aislada", dataset=nombre, z=z, **resumen(rs))
        # con caché: las mismas otra vez (lo que vive quien vuelve a una vista)
        rs = [pedir_tesela(fuente, *t, 60_000) for t in ts]
        di("cache", dataset=nombre, z=z, **resumen(rs))
        # mapa: todas a la vez con el plazo de producción (lo que vive quien mira); la vista de al lado,
        # para no repetir teselas ya calientes en el motor
        ts = vista(centro[0] + 0.05 * 2 ** (13 - z), centro[1], z)
        rs, hilos = [None] * len(ts), []
        t0 = time.time()
        for i, t in enumerate(ts):
            h = threading.Thread(target=lambda i=i, t=t: rs.__setitem__(i, pedir_tesela(fuente, *t, 10_000)))
            h.start()
            hilos.append(h)
        for h in hilos:
            h.join()
        di("mapa", dataset=nombre, z=z, vista_ms=round((time.time() - t0) * 1000), **resumen(rs))


def main():
    for _ in range(300):
        try:
            urllib.request.urlopen(MOTOR + "/salud", timeout=2).read()
            break
        except Exception:  # noqa: BLE001
            time.sleep(0.5)
    di("inicio", zooms=ZOOMS, cpus=os.cpu_count())
    if os.environ.get("BANCO_PIRAMIDE") == "1":
        return piramides()
    if os.environ.get("BANCO_INDICE") == "1":
        return indice()
    fuentes = []
    for n in ("trip_3m_puntos", "trip_3m_lineas"):
        try:
            fuentes.append((n, del_catalogo("query", "default", n), (-73.98, 40.70)))
        except Exception as e:  # noqa: BLE001
            di("falla", dataset=n, error=str(e)[:300])
    if os.environ.get("BANCO_EDIFICIOS", "1") == "1":
        try:
            fuentes.append(("edificios_madrid", edificios(), (-3.7038, 40.4168)))
        except Exception as e:  # noqa: BLE001
            di("falla", dataset="edificios", error=str(e)[:300], donde=traceback.format_exc().splitlines()[-3][:200])
    for nombre, fuente, centro in fuentes:
        ronda(nombre, fuente, centro)
    di("fin", borrar=LOCS)


def al_motor(ruta, cuerpo):
    pet = urllib.request.Request(MOTOR + ruta, data=json.dumps(cuerpo).encode(), method="POST",
                                 headers={"content-type": "application/json"})
    t0 = time.time()
    try:
        with urllib.request.urlopen(pet, timeout=60) as r:
            return r.status, json.load(r), round((time.time() - t0) * 1000)
    except urllib.error.HTTPError as e:
        return e.code, {"error": e.read().decode("utf-8", "replace")[:300]}, round((time.time() - t0) * 1000)


def indice():
    """0072 I3·4: `trip_30m` servida de su índice, como la sirve el descriptor; el clic; el crudo, para comparar."""
    ml, cred = del_catalogo("query", "default", "trip_30m")
    tabla = ml.rsplit("/metadata/", 1)[0]
    credencial = {k: str(v) for k, v in cred.items()}
    pide = {"ubicacion": tabla, "metadata_location": ml, "credencial": credencial}
    for vez in ("frio", "caliente"):
        c, r, ms = al_motor("/v1/indice", pide)
        di("indice_al_motor", vez=vez, codigo=c, ms=ms, fuente=r.get("fuente"),
           instantanea_ms=r.get("instantanea_ms"), hay=("indice" in r), error=r.get("error"))
    m = r.get("indice")
    if c != 200 or not m:
        di("falla", motivo="no hay índice de trip_30m", respuesta=str(r)[:300])
        return
    di("manifiesto", generacion=m.get("generacion"), al_dia=(m.get("snapshot_fuente") == ml),
       filas=m.get("filas"), profundidad=m.get("profundidad"), indice=m.get("indice"))
    centro = (-73.98, 40.75)
    del_indice = (m["indice"], cred, None, {"geometria": "geom", "crs": "EPSG:3857", "id": "_ore_id"})
    ronda("trip_30m_indice", del_indice, centro, ZOOMS)
    # el clic: unas filas de la fuente por su posición (fichero 0), en frío y otra vez
    for fila in (0, 1_234_567, 29_999_999):
        for vez in ("frio", "caliente"):
            c, r, ms = al_motor("/v1/fila", {"metadata_location": m["snapshot_fuente"], "ficheros": m["ficheros"],
                                              "ordinal": "0", "fila": str(fila), "credencial": credencial})
            di("clic", fila=fila, vez=vez, codigo=c, ms=ms,
               propiedades=sorted((r.get("propiedades") or {}).keys()), error=r.get("error"))
    # el crudo, para comparar: la tabla tal cual, unas teselas z16 de una en una (al final: puede tumbar al motor)
    if os.environ.get("BANCO_CRUDO", "1") == "1":
        crudo = (ml, cred, None, {"geometria": "t_pickup", "crs": "EPSG:4326", "id": "t_tripkey"})
        ts = vista(centro[0], centro[1], 16)[:4]
        rs = [pedir_tesela(crudo, *t, 60_000) for t in ts]
        di("aislada", dataset="trip_30m_crudo", z=16, **resumen(rs))
    di("fin", borrar=[])


def piramides():
    """F2·3·2·1: las pirámides de la prueba, servidas por el motor de al lado (z9–12)."""
    tok = ore._token_de_google()
    cubo = BANCO.rsplit("/", 1)[1]

    def pir(f):
        return {"ubicacion": "gs://%s/piramide/%s" % (cubo, f), "credencial": {"gcs.oauth2.token": tok}}

    # sin crudo detrás (z9–12 nunca llega a él): si el catálogo no presta, la pirámide sola igual
    nada = ("gs://%s/piramide/sin-crudo/metadata/0.metadata.json" % cubo, {"gcs.oauth2.token": tok})
    fuentes = []
    for n, f in (("trip_3m_puntos", "trip_3m_puntos-0.pmtiles"), ("trip_3m_lineas", "trip_3m_lineas_densidad-0.pmtiles")):
        try:
            ml, cred = del_catalogo("query", "default", n)
        except Exception as e:  # noqa: BLE001
            di("falla", dataset=n, error=str(e)[:300], sigue="sin crudo")
            ml, cred = nada
        fuentes.append(("piramide_" + n, (ml, cred, pir(f)), (-73.98, 40.70)))
    fuentes.append(("piramide_edificios", nada + (pir("edificios_densidad-0.pmtiles"),), (-3.7038, 40.4168)))
    for nombre, fuente, centro in fuentes:
        ronda(nombre, fuente, centro, ZOOMS_PIRAMIDE)
    di("fin", borrar=[])


main()
