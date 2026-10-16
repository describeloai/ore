# pg1-censo.sh (ADR 0065 PG1): antes de desplegar el driver que niega el texto plano, ¿qué fuentes
# Postgres de este inquilino dejarían de conectar? Dentro de la celda: la lista de fuentes de ore-serve,
# y por cada una que sea Postgres en el cofre prueba el driver de HOY con la URL tal cual y con
# `sslmode=require` forzado (con el conector de hoy, require ya verifica cadena y nombre: es lo que
# hará el nuevo sin `sslmode`). Nunca imprime una URL: sólo el host y los nombres de los parámetros.
set -e
COFRE="http://ore-cofre.$NS.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos"
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
set +e
# El agente no puede listar el cofre (secreto:listar): la lista sale de ore-serve.
C=$(curl -s -o /tmp/l.json -w '%{http_code}' -H "authorization: Bearer $TOK" "http://ore-serve.$NS.svc.cluster.local:8080/fuentes")
[ "$C" = 200 ] || { echo "## falla: GET /fuentes contestó $C: $(head -c 200 /tmp/l.json)"; exit 1; }
D=/opt/ore/conectores/ore-read-postgres
python3 -c '
import json
def todas(n):
    if isinstance(n, dict):
        x = n.get("nombre") or n.get("name")
        if isinstance(x, str):
            yield x
        for v in n.values():
            if isinstance(v, (list, dict)):
                yield from todas(v)
    elif isinstance(n, list):
        for v in n:
            yield from todas(v)
for x in sorted(set(todas(json.load(open("/tmp/l.json"))))):
    print(x)
' >/tmp/fuentes
echo "fuentes del inquilino: $(wc -l </tmp/fuentes)"
while read -r F; do
  C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "$COFRE/fuente-$F")
  [ "$C" = 200 ] || { echo "$F · el cofre contestó $C"; continue; }
  python3 - >/tmp/u <<'PY'
import json
print(json.load(open("/tmp/s.json"))["valor"])
PY
  rm -f /tmp/s.json
  case "$(head -c 12 /tmp/u)" in postgres*) ;; *) continue ;; esac
  python3 - /tmp/u >/tmp/info <<'PY'
import sys, urllib.parse as u
url = open(sys.argv[1]).read().strip()
p = u.urlsplit(url)
q = u.parse_qsl(p.query)
print("host=%s puerto=%s parametros=%s" % (p.hostname, p.port or 5432,
      ",".join(f"{k}={v}" if k == "sslmode" else k for k, v in q) or "-"))
f = [(k, v) for k, v in q if k != "sslmode"] + [("sslmode", "require")]
open("/tmp/ur", "w").write(u.urlunsplit((p.scheme, p.netloc, p.path, u.urlencode(f), "")))
PY
  tal=$(python3 -c 'import json;print(json.dumps({"url":open("/tmp/u").read().strip()}))' | timeout 30 $D check 2>&1 | tr '\n' ' ' | cut -c1-160)
  req=$(python3 -c 'import json;print(json.dumps({"url":open("/tmp/ur").read()}))' | timeout 30 $D check 2>&1 | tr '\n' ' ' | cut -c1-160)
  echo "$F · $(cat /tmp/info) · tal cual: $tal · require: $req"
  rm -f /tmp/u /tmp/ur
done </tmp/fuentes
echo "## ok"
