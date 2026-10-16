# vara-celda.sh (ADR 0067 G0·3): los casos de la vara geo que corren con la imagen de drivers, DENTRO
# de la celda (las URLs salen del cofre con la credencial del agente; ni se imprimen ni tocan el disco
# fuera de /tmp). Emite líneas «V<n>|…» que lee vara.py.
#   V7 · cómo llega la geometría de cada origen del banco por su driver: el tipo que dice el catálogo,
#        una muestra en texto y una lectura entera en Arrow (filas/s).
#   V2 · la inducción (`ore discover --from`) sobre esos catálogos: qué tipo propone a cada columna
#        geo de la verdad del banco.
#   V1 · declarar una columna geo: el paquete inducido de PostGIS con `geom` como
#        `Geometry<MultiPolygon, EPSG:25830>`, y `ore validate`.
set +e
P=project-8853a180-450d-47be-b83
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
COFRE="http://ore-cofre.$NS.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos"
url_de() {  # la URL de una fuente del banco; PostGIS, si aún no está dada de alta, es la de pg_quality con la base banco_geo
  C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "$COFRE/fuente-$1")
  if [ "$C" = 200 ]; then python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])'; rm -f /tmp/s.json; return; fi
  case "$1" in
    banco_geo_pg) curl -s -o /tmp/s.json -H "authorization: Bearer $TOK" "$COFRE/fuente-pg_quality"
      python3 -c 'import json,urllib.parse as p;u=p.urlsplit(json.load(open("/tmp/s.json"))["valor"]);print(p.urlunsplit(u._replace(path="/banco_geo")))'; rm -f /tmp/s.json ;;
    banco_geo_bq) echo "bigquery://$P/banco_geo" ;;
    banco_geo_gcs) echo "gs://$P-banco-geo/ficheros" ;;
  esac
}
mkdir -p /tmp/v
for F in banco_geo_pg banco_geo_bq banco_geo_gcs; do
  url_de $F >/tmp/v/$F.url
  case "$(head -c 5 /tmp/v/$F.url)" in post*) D=ore-read-postgres ;; bigq*) D=ore-read-bigquery ;; gs:/*) D=ore-read-gcs ;; esac
  D=/opt/ore/conectores/$D
  T0=$(date +%s%N); timeout 300 $D catalogo "$F" </tmp/v/$F.url >/tmp/v/$F.json 2>/tmp/v/$F.err; rc=$?
  echo "V7|$F|catalogo|-|rc=$rc en $(( ($(date +%s%N)-T0)/1000000 )) ms $(head -c 200 /tmp/v/$F.err | tr '\n' ' ')"
  F="$F" D="$D" python3 - <<'PY'
import json, os, subprocess, time
f, d = os.environ["F"], os.environ["D"]
url = open(f"/tmp/v/{f}.url").read().strip()
s = open(f"/tmp/v/{f}.json").read()
cat = json.loads(s[s.index("{"):]) if "{" in s else {}
json.dump(cat, open(f"/tmp/v/{f}.cat.json", "w"))
GEO = ("geom", "geog", "geo", "wkt", "lat", "lon", "the_geom", "shape", "geometry", "wkb_geometry")
for o in cat.get("objects", []):
    print(f"V7|{f}|objetos:{o.get('name')}|-|" + json.dumps({k: v for k, v in o.items() if k not in ('name', 'reads', 'changes')}, ensure_ascii=False)[:300])
for t in cat.get("tables", []):
    cols = t.get("columns", [])
    geo = [c for c in cols if any(c.get("name", "").lower() == g or c.get("name", "").lower().startswith(g) for g in GEO)]
    print(f"V7|{f}|{t['name']}|-|tabla: {len(cols)} columnas, filas {t.get('rows') or t.get('rowsEstimate')}, formato {json.dumps(t.get('format'))[:120]}")
    for c in geo:
        print(f"V7|{f}|{t['name']}|{c['name']}|catálogo: " + json.dumps({k: v for k, v in c.items() if k != 'name'}, ensure_ascii=False))
    if not geo:
        continue
    obj = t.get("object") or t["name"]
    col = geo[0]["name"]
    # Una tabla de ficheros lleva su `format` y sus tipos en la petición (`fichero`, ore-driver).
    extra = {"fichero": {"format": t["format"], "tipos": [[c["name"], c["type"]] for c in cols if c.get("type")]}} if t.get("format") else {}
    r = subprocess.run([d, "leer"], input=json.dumps({"url": url, "objeto": obj, "proyeccion": {col: col}, "limit": 2, **extra}),
                       capture_output=True, text=True, timeout=180)
    if "contesta en Arrow" in r.stderr:
        print(f"V7|{f}|{t['name']}|{col}|texto: este driver sólo contesta en Arrow")
    else:
        print(f"V7|{f}|{t['name']}|{col}|texto: " + (r.stdout or r.stderr).strip().replace("\n", " ⏎ ")[:240])
    t0 = time.time()
    r = subprocess.run([d, "leer"], input=json.dumps({"url": url, "objeto": obj, "proyeccion": {c["name"]: c["name"] for c in cols}, "formato": "arrow", **extra}).encode(),
                       capture_output=True, timeout=900)
    sg = time.time() - t0
    n = t.get("rows") or t.get("rowsEstimate") or 0
    print(f"V7|{f}|{t['name']}|*|arrow: rc={r.returncode}, {len(r.stdout) / 1e6:.1f} MB en {sg:.1f} s, {n / sg:,.0f} filas/s {r.stderr.decode(errors='replace')[:160]}")
PY
done
rm -f /tmp/v/*.url

# ── V2 · la inducción ──
rm -rf /tmp/v/ws && mkdir -p /tmp/v/ws && cd /tmp/v/ws && ore init >/dev/null 2>&1
for F in banco_geo_pg banco_geo_bq banco_geo_gcs; do
  cd /tmp/v/ws && timeout 300 ore discover --from /tmp/v/$F.cat.json --out packages/$F --name $F --owner team:geo >/tmp/v/d.out 2>&1
  echo "V2|$F|discover|-|rc=$? $(tail -c 160 /tmp/v/d.out | tr '\n' ' ')"
done
python3 - <<'PY'
import glob, json, os, re
# La verdad del banco: (fuente, tabla, columna) → lo que debería proponerse.
VERDAD = [
    ("banco_geo_pg", "municipios", "geom", "Geometry<MultiPolygon, EPSG:25830>"),
    ("banco_geo_pg", "lugares", "geom", "Geometry<Point>"),
    ("banco_geo_pg", "lugares_geog", "geog", "Geography<Point>"),
    ("banco_geo_pg", "carreteras", "geom", "Geometry"),
    ("banco_geo_bq", "lugares", "geo", "Geography"),
    ("banco_geo_bq", "municipios", "geo", "Geography"),
    ("banco_geo_gcs", "lugares_madrid_latlon", "lat+lon", "par lat/lon → Geometry<Point>"),
    ("banco_geo_gcs", "lugares_madrid_wkt", "wkt", "WKT → Geometry"),
    ("banco_geo_gcs", "lugares_madrid_direcciones", "direccion", "geocodificable"),
    ("banco_geo_gcs", "municipios_madrid", "geom", "GeoParquet → Geometry<MultiPolygon>"),
] + [("banco_geo_gcs", f"municipios_madrid.{e}", "geom", "Geometry<MultiPolygon>") for e in ("geojson", "gpkg", "kml", "fgb", "gdb", "shp.zip")]
cats = {f: json.load(open(f"/tmp/v/{f}.cat.json")) for f in ("banco_geo_pg", "banco_geo_bq", "banco_geo_gcs")}
aciertos = 0
for f, t, c, quiero in VERDAD:
    if "." in t:
        ext = t.split(".", 1)[1].split(".")[0]
        objs = [(o["name"], o.get("media")) for o in cats[f].get("objects", []) if ext in o.get("extensions", []) or ext in o["name"]]
        print(f"V2|{f}|{t}|{c}|quiero: {quiero} · propone: nada, es un objeto de medios {objs} · fallo")
        continue
    texto = ""
    for p in glob.glob(f"/tmp/v/ws/packages/{f}/**/*", recursive=True):
        if os.path.isfile(p) and t in os.path.basename(p):
            texto += open(p, encoding="utf-8", errors="replace").read()
    tipo = "—"
    for col in c.split("+"):
        m = re.search(rf"^\s*{re.escape(col)}\s*:\s*(.+)$", texto, re.M)
        if m:
            tipo = m.group(1).strip()[:90]
            break
    m = re.search(r"\btype\s*:\s*\"?([A-Za-z]+)", tipo)
    geo = bool(m and m.group(1) in ("Geometry", "Geography")) or (c not in ("geom", "geog", "geo") and re.search(r"geo/|geocodific", texto) is not None)
    aciertos += geo
    print(f"V2|{f}|{t}|{c}|quiero: {quiero} · propone: {tipo} · {'acierto' if geo else 'fallo'}")
print(f"V2|resumen|-|-|exhaustividad {aciertos}/{len(VERDAD)}")
PY

# ── V1 · declarar una columna geo ──
# Un árbol con una sola Table (la de municipios que indujo PostGIS) y `geom` declarada como geometría:
# lo que `ore validate` diga es de ella.
rm -rf /tmp/v/v1 && mkdir -p /tmp/v/v1 && cd /tmp/v/v1 && ore init >/dev/null 2>&1
python3 - <<'PY'
import glob, os, re, shutil
p = next((p for p in glob.glob("/tmp/v/ws/packages/banco_geo_pg/**/tables/*.yaml", recursive=True)
          if "unicipios" in os.path.basename(p)), None)
if not p:
    print("V1|banco_geo_pg|-|-|sin Table de municipios en la inducción")
else:
    s = open(p).read()
    s2 = re.sub(r"^(\s*)geom\s*:\s*\{.*\}\s*$", r'\1geom: { type: "Geometry<MultiPolygon, EPSG:25830>", physicalType: "geometry(MultiPolygon,25830)" }', s, count=1, flags=re.M)
    # Una columna geo es de v1alpha32 (`01-la-geometria` §6): el documento sube de versión.
    s2 = re.sub(r"apiVersion: oos.dev/v1alpha[0-9]+", "apiVersion: oos.dev/v1alpha32", s2, count=1)
    os.makedirs("/tmp/v/v1/packages/geo/tables", exist_ok=True)
    open("/tmp/v/v1/packages/geo/tables/municipios.yaml", "w").write(s2)
    print("V1|banco_geo_pg|municipios|geom|declarada: " + ("Geometry<MultiPolygon, EPSG:25830>" if s2 != s else "no se encontró la columna"))
PY
ore source add --name banco_geo_pg postgres://banco/banco_geo >/dev/null 2>&1
ore validate . >/tmp/v/val.out 2>&1; rc=$?
echo "V1|banco_geo_pg|validate|-|rc=$rc $(grep -E '^(error|warning)' /tmp/v/val.out | head -4 | tr '\n' ' ' | cut -c1-500)"
echo "## ok"
