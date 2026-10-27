"""ORE 0072 F2·1·3 · El arnés del laboratorio de punta a punta. En la red de `f2-banco` (ore-serve en
127.0.0.1:8080 con `ore-teselas` detrás, el S3 de mentira), como ana:

  1. escribe `geo.edificios` con `ore.write` del SDK (polígonos sintéticos de Madrid, EPSG:4326): lo que
     pinta la capa, con su cobertura `geom_bbox` (el `ore-store-r2` de este árbol);
  2. pide el descriptor (`GET /v1/layers/geo.edificios/descriptor`, con sujeto) y lo comprueba (TileJSON);
  3. pide teselas por las URLs firmadas **sin sujeto**, como el navegador: 200 con `immutable`, `ETag`
     estable y CORS; 204 en el mar; 403 con la firma tocada; 410 con otra clave; 404 bajo el suelo;
  4. (F2·1·6) escribe `geo.viajes_puntos` y `geo.viajes_lineas` y comprueba que sus teselas traen sólo
     su tipo (`x-ore-cuenta`: puntos, lineas, poligonos, descartados) con el `ETag` estable;
  5. deja `mapa.html`: MapLibre con los tres descriptores incrustados (lo que la consola trae con el
     `Bearer`), cada tipo en su capa filtrada por `geometry-type`, y las teselas directas a ore-serve.

Una línea por caso y `n/m casos`.
"""
import json
import math
import os
import sys
import time
import urllib.error
import urllib.request

os.environ.update({"ORE_SERVE": "http://127.0.0.1:8080"})
sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402

ANA = {"x-ore-sujeto": "persona:ana"}
ore.session._cabeceras = dict(ANA)
ore.session.id = ""
casos = []


def caso(nombre, f, espera=None):
    try:
        r = f()
        ok = espera is None or espera(r)
    except Exception as e:  # noqa: BLE001
        r, ok = "%s: %s" % (type(e).__name__, str(e).splitlines()[0][:300]), False
    casos.append(ok)
    print("%s %s · %s" % ("OK   " if ok else "FALLA", nombre, r), flush=True)
    return r


def pedir(url, cabeceras=None):
    """(código, cabeceras, cuerpo, ms) de un GET."""
    t0 = time.time()
    try:
        r = urllib.request.urlopen(urllib.request.Request(url, headers=cabeceras or {}), timeout=60)
        return r.status, {k.lower(): v for k, v in r.headers.items()}, r.read(), round((time.time() - t0) * 1000)
    except urllib.error.HTTPError as e:
        return e.code, {k.lower(): v for k, v in e.headers.items()}, e.read(), round((time.time() - t0) * 1000)


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


# ── 1 · los edificios ────────────────────────────────────────────────────────────────────────────
N = int(os.environ.get("F2_N", "200000"))
con = ore._duckdb()
con.execute("load spatial")
con.execute("set geometry_always_xy = true")
edificios = con.execute(f"""
  with p as (
    select range id,
           -3.7038 + (random() - 0.5) * 0.25 * pow(random(), 1.5) * 4 x,
           40.4168 + (random() - 0.5) * 0.18 * pow(random(), 1.5) * 4 y,
           0.00008 + random() * 0.0003 w, 0.00006 + random() * 0.00025 h
    from range({N}))
  select id, ST_SetCRS(ST_MakeEnvelope(x, y, x + w, y + h), 'EPSG:4326') geom from p
""").fetch_arrow_table()
caso("1 · ore.write de geo.edificios", lambda: {k: v for k, v in dict(ore.write("geo.edificios", edificios)).items()
                                               if k in ("rows", "snapshot", "geo")},
     lambda r: r.get("rows") == N)

# ── 2 · el descriptor ────────────────────────────────────────────────────────────────────────────
D = "http://localhost:8080/v1/layers/geo.edificios/descriptor"
codigo, cab, cuerpo, ms = pedir(D, {**ANA, "host": "localhost:8080"})
desc = json.loads(cuerpo) if codigo == 200 else {}
caso("2 · el descriptor (TileJSON 3.0)", lambda: (codigo, ms, desc.get("tilejson"), desc.get("minzoom"), desc.get("maxzoom"),
                                                   desc.get("vector_layers", [{}])[0].get("id")),
     lambda r: r[0] == 200 and r[2] == "3.0.0" and r[3] == 13 and r[5] == "geo.edificios")
caso("2b · sin sujeto no hay descriptor", lambda: pedir(D)[0], lambda c: c == 401)
caso("2c · minzoom a petición", lambda: json.loads(pedir(D + "?minzoom=12", {**ANA, "host": "localhost:8080"})[2])["minzoom"],
     lambda z: z == 12)
plantilla = (desc.get("tiles") or [""])[0]
caso("2d · la URL va firmada y por la entrada", lambda: plantilla,
     lambda u: u.startswith("http://localhost:8080/v1/tiles/geo.edificios/") and "e=" in u and "&f=" in u)


def url(z, x, y, t=plantilla):
    return t.replace("{z}", str(z)).replace("{x}", str(x)).replace("{y}", str(y))


# ── 3 · las teselas, sin sujeto ──────────────────────────────────────────────────────────────────
x, y = tesela_de(-3.7038, 40.4168, 15)
c1 = pedir(url(15, x, y))
caso("3 · una tesela de Madrid (z15)", lambda: (c1[0], round(len(c1[2]) / 1024, 1), c1[1].get("cache-control"),
                                                c1[1].get("access-control-allow-origin"), c1[3], c1[1].get("x-ore-cuenta")),
     lambda r: r[0] == 200 and r[1] > 0 and "immutable" in (r[2] or "") and r[3] == "*")
c2 = pedir(url(15, x, y))
caso("3b · la misma tesela, los mismos bytes (ETag)", lambda: (c1[1].get("etag"), c2[1].get("etag"), c1[2] == c2[2]),
     lambda r: r[0] and r[0] == r[1] and r[2])
xm, ym = tesela_de(-20.0, 40.0, 14)
caso("3c · en el mar: 204", lambda: pedir(url(14, xm, ym))[0], lambda c: c == 204)
mala = plantilla[:-1] + ("0" if plantilla[-1] != "0" else "1")
caso("3d · la firma tocada: 403", lambda: pedir(url(15, x, y, mala))[0], lambda c: c == 403)
clave = plantilla.split("/v1/tiles/geo.edificios/")[1].split("/")[0]
otra = plantilla.replace(clave, "0" * 32)
caso("3e · otra clave (firma de otra): 403", lambda: pedir(url(15, x, y, otra))[0], lambda c: c == 403)
xb, yb = tesela_de(-3.7038, 40.4168, 11)
caso("3f · bajo el suelo (z11): 404", lambda: pedir(url(11, xb, yb))[0], lambda c: c == 404)

# z13–z16 por Madrid: tiempos y tamaños por la ruta entera (ore-serve → ore-teselas → S3)
ms_z = {}
for z in (13, 14, 15, 16):
    ts, kb = [], []
    for dx in (-1, 0, 1):
        for dy in (-1, 0, 1):
            tx, ty = tesela_de(-3.7038, 40.4168, z)
            c = pedir(url(z, tx + dx, ty + dy))
            if c[0] in (200, 204):
                ts.append(c[3])
                kb.append(len(c[2]) / 1024)
    ms_z[z] = {"teselas": len(ts), "p50_ms": sorted(ts)[len(ts) // 2] if ts else None,
               "max_ms": max(ts) if ts else None, "kb_max": round(max(kb), 1) if kb else None}
caso("3g · Madrid z13–z16 (3×3 por zoom)", lambda: ms_z, lambda r: all(v["teselas"] == 9 for v in r.values()))

# ── 4 · puntos y líneas (F2·1·6) ─────────────────────────────────────────────────────────────────
# Viajes sintéticos por Madrid: la recogida como punto y el trayecto como línea de tres vértices.
NP = int(os.environ.get("F2_NP", "200000"))
NL = int(os.environ.get("F2_NL", "50000"))
viajes = """
  select range id,
         -3.7038 + (random() - 0.5) * 0.25 * pow(random(), 1.5) * 4 x,
         40.4168 + (random() - 0.5) * 0.18 * pow(random(), 1.5) * 4 y,
         (random() - 0.5) * 0.02 dx, (random() - 0.5) * 0.02 dy
  from range({n})"""
puntos = con.execute(f"""
  with v as ({viajes.format(n=NP)})
  select id, ST_SetCRS(ST_Point(x, y), 'EPSG:4326') geom from v
""").fetch_arrow_table()
lineas = con.execute(f"""
  with v as ({viajes.format(n=NL)})
  select id, ST_SetCRS(ST_MakeLine([ST_Point(x, y), ST_Point(x + dx / 2, y + dy / 3), ST_Point(x + dx, y + dy)]),
                       'EPSG:4326') geom from v
""").fetch_arrow_table()
for nombre, tabla, n in (("geo.viajes_puntos", puntos, NP), ("geo.viajes_lineas", lineas, NL)):
    caso("4 · ore.write de " + nombre, lambda: {k: v for k, v in dict(ore.write(nombre, tabla)).items()
                                               if k in ("rows", "snapshot")},
         lambda r, n=n: r.get("rows") == n)


def descriptor_de(capa):
    c = pedir("http://localhost:8080/v1/layers/%s/descriptor" % capa, {**ANA, "host": "localhost:8080"})
    return json.loads(c[2]) if c[0] == 200 else {"codigo": c[0], "cuerpo": c[2][:300]}


DESC = {"geo.edificios": desc}
for capa, tipo in (("geo.viajes_puntos", "puntos"), ("geo.viajes_lineas", "lineas")):
    d = DESC[capa] = descriptor_de(capa)
    caso("4b · el descriptor de " + capa, lambda d=d: (d.get("tilejson"), d.get("minzoom"), d.get("codigo")),
         lambda r: r[0] == "3.0.0")
    t = (d.get("tiles") or [""])[0]
    a = pedir(url(15, x, y, t))
    b = pedir(url(15, x, y, t))
    cuenta = json.loads(a[1].get("x-ore-cuenta") or "{}")
    caso("4c · una tesela de %s (z15): %s > 0, el mismo ETag" % (capa, tipo),
         lambda a=a, b=b, cuenta=cuenta, tipo=tipo: (a[0], round(len(a[2]) / 1024, 1), a[3], cuenta.get(tipo),
                                                     cuenta.get("descartados"), a[1].get("etag") == b[1].get("etag")),
         lambda r: r[0] == 200 and (r[3] or 0) > 0 and r[4] == 0 and r[5])
    # y los otros dos tipos no aparecen en ella
    caso("4d · en %s sólo hay %s" % (capa, tipo),
         lambda cuenta=cuenta, tipo=tipo: {k: cuenta.get(k) for k in ("puntos", "lineas", "poligonos")},
         lambda r, tipo=tipo: all(v == 0 for k, v in r.items() if k != tipo))
# los edificios siguen siendo polígonos, y sólo polígonos
cuenta_e = json.loads(c1[1].get("x-ore-cuenta") or "{}")
caso("4e · geo.edificios: sólo polígonos", lambda: {k: cuenta_e.get(k) for k in ("puntos", "lineas", "poligonos")},
     lambda r: r["poligonos"] > 0 and r["puntos"] == 0 and r["lineas"] == 0)

# ── 5 · la página ─────────────────────────────────────────────────────────────────────────────────
pagina = """<!doctype html>
<!-- ORE 0072 F2·1·3 y F2·1·6 · MapLibre con los descriptores incrustados (lo que la consola trae con el
     Bearer): edificios (polígonos), viajes (puntos) y trayectos (líneas); las teselas, directas a ore-serve
     con su firma, y cada tipo con su capa filtrada por `geometry-type`, como en la consola. La genera arnes.py. -->
<html lang="es"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>F2·1 laboratorio</title>
<link rel="stylesheet" href="https://unpkg.com/maplibre-gl@5.24.0/dist/maplibre-gl.css">
<script src="https://unpkg.com/maplibre-gl@5.24.0/dist/maplibre-gl.js"></script>
<style>html,body{margin:0;height:100%;background:#fff}#mapa{position:absolute;inset:0}</style></head>
<body><div id="mapa"></div><script>
const DESCRIPTORES = __DESCRIPTORES__;
const COLOR = { "geo.edificios": "#0f62fe", "geo.viajes_puntos": "#da1e28", "geo.viajes_lineas": "#198038" };
window.f2 = { errores: [], cargadas: 0 };
const mapa = new maplibregl.Map({ container: "mapa", style: "https://basemaps.cartocdn.com/gl/positron-gl-style/style.json",
                                   center: [-3.7038, 40.4168], zoom: 15 });
const es = t => ["==", ["geometry-type"], t];
mapa.on("load", () => {
  for (const [capa, d] of Object.entries(DESCRIPTORES)) {
    if (!d.tiles) continue;
    const c = COLOR[capa], fuente = { source: capa, "source-layer": d.vector_layers[0].id };
    mapa.addSource(capa, { type: "vector", tiles: d.tiles, minzoom: d.minzoom, maxzoom: d.maxzoom });
    mapa.addLayer({ id: capa + ":relleno", type: "fill", ...fuente, filter: es("Polygon"),
                    paint: { "fill-color": c, "fill-opacity": 0.45, "fill-outline-color": c } });
    mapa.addLayer({ id: capa + ":linea", type: "line", ...fuente, filter: es("LineString"),
                    paint: { "line-color": c, "line-width": 1.2 } });
    mapa.addLayer({ id: capa + ":punto", type: "circle", ...fuente, filter: es("Point"),
                    paint: { "circle-color": c, "circle-radius": 2.5, "circle-opacity": 0.8 } });
  }
});
mapa.on("sourcedata", e => { if (e.tile) window.f2.cargadas++; });
mapa.on("error", e => window.f2.errores.push(String(e.error && e.error.message)));
</script></body></html>
"""
open("/l/mapa.html", "w", encoding="utf-8").write(pagina.replace("__DESCRIPTORES__", json.dumps(DESC)))
caso("5 · mapa.html con los tres descriptores", lambda: os.path.getsize("/l/mapa.html"), lambda n: n > 1000)

print("\n%d/%d casos" % (sum(casos), len(casos)))
