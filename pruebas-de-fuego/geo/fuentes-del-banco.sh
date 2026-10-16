# fuentes-del-banco.sh (ADR 0067 G0·2): ¿están dadas de alta las fuentes del banco? En la celda, sin URLs.
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
curl -s -H "authorization: Bearer $TOK" "http://ore-serve.$NS.svc.cluster.local:8080/fuentes" | python3 -c '
import json,sys
s=json.dumps(json.load(sys.stdin))
for f in ("banco_geo_pg","banco_geo_bq","banco_geo_gcs"): print(f, "dada de alta" if f in s else "FALTA")'
echo "## ok"
