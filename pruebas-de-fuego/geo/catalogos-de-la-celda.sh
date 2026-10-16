# catalogos-de-la-celda.sh (ADR 0067 G1·4): el catálogo de cada fuente Postgres y BigQuery del inquilino,
# DENTRO de la celda, para pasarles la inducción nueva fuera (los falsos positivos de V2). Solo estructura:
# las URLs salen del cofre y no se imprimen. Cada catálogo va entre «### <fuente>» y «### fin».
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
COFRE="http://ore-cofre.$NS.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos"
curl -s -H "authorization: Bearer $TOK" "http://ore-serve.$NS.svc.cluster.local:8080/fuentes" | python3 -c '
import json,sys
def todas(n):
    if isinstance(n, dict):
        x = n.get("nombre") or n.get("name")
        if isinstance(x, str): yield x
        for v in n.values():
            if isinstance(v, (list, dict)): yield from todas(v)
    elif isinstance(n, list):
        for v in n: yield from todas(v)
print("\n".join(sorted(set(todas(json.load(sys.stdin))))))' >/tmp/fuentes
while read -r F; do
  case "$F" in banco_geo_*) continue ;; esac
  C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "$COFRE/fuente-$F")
  [ "$C" = 200 ] || continue
  python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])' >/tmp/u; rm -f /tmp/s.json
  case "$(head -c 5 /tmp/u)" in post*) D=ore-read-postgres ;; bigq*) D=ore-read-bigquery ;; *) rm -f /tmp/u; continue ;; esac
  echo "### $F"
  timeout 180 /opt/ore/conectores/$D catalogo "$F" </tmp/u 2>/dev/null; rm -f /tmp/u
  echo "### fin"
done </tmp/fuentes
echo "## ok"
