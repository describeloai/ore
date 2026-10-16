# geo-t7-censo.sh (ADR 0066 T7): ¿qué geografía hay de verdad en las fuentes de este inquilino?
# Dentro de la celda: la lista de fuentes de ore-serve; por cada una con URL en el cofre, el catálogo de
# su driver, y de él las columnas con tipo geo del origen (geometry, geography, point…) y las que por
# su nombre parecen coordenadas o direcciones. En un Postgres, además, si tiene PostGIS. Nunca imprime
# una URL.
set -e
COFRE="http://ore-cofre.$NS.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos"
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
set +e
curl -s -o /tmp/l.json -H "authorization: Bearer $TOK" "http://ore-serve.$NS.svc.cluster.local:8080/fuentes"
python3 -c '
import json
def todas(n):
    if isinstance(n, dict):
        x = n.get("nombre") or n.get("name")
        if isinstance(x, str): yield x
        for v in n.values():
            if isinstance(v, (list, dict)): yield from todas(v)
    elif isinstance(n, list):
        for v in n: yield from todas(v)
for x in sorted(set(todas(json.load(open("/tmp/l.json"))))): print(x)
' >/tmp/fuentes
echo "fuentes del inquilino: $(wc -l </tmp/fuentes)"
while read -r F; do
  C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "$COFRE/fuente-$F")
  [ "$C" = 200 ] || continue
  python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])' >/tmp/u; rm -f /tmp/s.json
  ESQ=$(head -c 12 /tmp/u | sed 's/:.*//')
  case "$ESQ" in
    postgres|postgresql) D=/opt/ore/conectores/ore-read-postgres
      EXT=$(psql "$(cat /tmp/u)" -XAtq -c "select string_agg(extname||' '||extversion, ', ') from pg_extension where extname in ('postgis','h3','pgvector','vector','timescaledb')" 2>&1 | tr '\n' ' ' | cut -c1-120) ;;
    bigquery) D=/opt/ore/conectores/ore-read-bigquery; EXT="-" ;;
    *) echo "$F · $ESQ · (no tabular, se salta)"; rm -f /tmp/u; continue ;;
  esac
  timeout 180 $D catalogo "$F" </tmp/u >/tmp/c.json 2>/tmp/c.err; rc=$?; rm -f /tmp/u
  python3 - "$F" "$ESQ" "$rc" "$EXT" <<'PY'
import json, re, sys
f, esq, rc, ext = sys.argv[1:]
try:
    s = open("/tmp/c.json").read(); ts = json.loads(s[s.index("{"):])["tables"]
except Exception as e:
    print(f"{f} · {esq} · catálogo rc={rc}: {open('/tmp/c.err').read()[:160]}"); sys.exit()
GEO = re.compile(r"geometry|geography|point|polygon|linestring|box2d|raster", re.I)
COORD = re.compile(r"(^|_)(lat|latitude|latitud|lng|lon|long|longitude|longitud|x_coord|y_coord|geohash|h3|wkt|geojson|geom|coordenadas?)($|_)", re.I)
DIR = re.compile(r"(^|_)(address|direccion|dirección|street|calle|city|ciudad|municipio|zip|postal|cp|province|provincia|state|country|pais|país)($|_)", re.I)
geo, coord, dirs = [], [], []
for t in ts:
    for c in t.get("columns", []):
        st = c.get("sourceType") or ""
        n = c["name"]
        if GEO.search(st): geo.append(f"{t['name']}.{n}:{st}")
        elif COORD.search(n): coord.append(f"{t['name']}.{n}:{st}")
        elif DIR.search(n): dirs.append(f"{t['name']}.{n}")
filas = sum(t.get("rowsEstimate") or t.get("rows") or 0 for t in ts)
print(f"{f} · {esq} · {len(ts)} tablas · ~{filas} filas · extensiones: {ext}")
print(f"   tipo geo del origen ({len(geo)}): {geo[:8]}")
print(f"   coordenadas por nombre ({len(coord)}): {coord[:10]}")
print(f"   direcciones por nombre ({len(dirs)}): {dirs[:10]}")
PY
done </tmp/fuentes
echo "## ok"
