# vara-pg.sh: una sentencia en el Postgres de la vara (Neon, la fuente postgresql_20260918_1920),
# DENTRO de la celda: la URL sale del cofre con la credencial del agente y no toca el disco ni la
# salida. La sentencia llega en $SQL; se imprime lo que devuelve, con «## ok» o «## falla».
set -e
DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])')
[ "$DICE" = "$EMISOR/realms/$REALM" ] || { echo "## falla: emisor inesperado"; exit 1; }
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "http://ore-cofre.t-victor.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos/fuente-postgresql_20260918_1920")
[ "$C" = 200 ] || { echo "## falla: el custodio contesto $C"; exit 1; }
export U="$(python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])')"; rm -f /tmp/s.json
export PGCONNECT_TIMEOUT=15
if psql "$U" -v ON_ERROR_STOP=1 -XAtq -F ' | ' -c "set statement_timeout=60000" -c "$SQL" 2>&1; then echo "## ok"; else echo "## falla"; fi
