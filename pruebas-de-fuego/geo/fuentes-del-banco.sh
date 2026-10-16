# fuentes-del-banco.sh (ADR 0067 G0·2): ¿están dadas de alta las fuentes del banco? En la celda, sin URLs.
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
curl -s -H "authorization: Bearer $TOK" "http://ore-serve.$NS.svc.cluster.local:8080/fuentes" | python3 -c '
import json,sys
s=json.dumps(json.load(sys.stdin))
for f in ("banco_geo_pg","banco_geo_bq","banco_geo_gcs"): print(f, "dada de alta" if f in s else "FALTA")'
# Y que cada una se lee: el catálogo por su driver, con la URL del cofre (no se imprime).
COFRE="http://ore-cofre.$NS.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos"
for F in banco_geo_pg banco_geo_bq banco_geo_gcs; do
  C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "$COFRE/fuente-$F")
  [ "$C" = 200 ] || { echo "$F · cofre $C"; continue; }
  python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])' >/tmp/u; rm -f /tmp/s.json
  case "$(head -c 5 /tmp/u)" in post*) D=ore-read-postgres ;; bigq*) D=ore-read-bigquery ;; gs:/*) D=ore-read-gcs ;; *) echo "$F · esquema raro: $(head -c 12 /tmp/u)"; continue ;; esac
  F="$F" python3 -c 'import os,urllib.parse as p;u=p.urlsplit(open("/tmp/u").read().strip());print(os.environ["F"],"· url:",u.scheme+"://"+(u.hostname or "")+u.path)'
  timeout 300 /opt/ore/conectores/$D catalogo "$F" </tmp/u >/tmp/c.json 2>/tmp/c.err; rc=$?; rm -f /tmp/u
  python3 - "$F" "$rc" <<'PY'
import json, sys
f, rc = sys.argv[1], sys.argv[2]
s = open("/tmp/c.json").read()
if "{" not in s:
    print(f, "· catálogo rc", rc, open("/tmp/c.err").read()[:300]); sys.exit()
c = json.loads(s[s.index("{"):])
print(f, "· catálogo rc", rc, "·", len(c.get("tables", [])), "tablas:", ", ".join(t["name"] for t in c.get("tables", [])),
      "·", len(c.get("objects", [])), "conjuntos de objetos")
PY
done
echo "## ok"
