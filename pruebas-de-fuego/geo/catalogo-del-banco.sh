# catalogo-del-banco.sh (ADR 0067): imprime el catálogo de una fuente del banco ($FUENTE), por su driver,
# DENTRO de la celda. El catálogo no lleva la URL ni secretos; la URL sale del cofre y no se imprime.
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
curl -s -o /tmp/s.json -H "authorization: Bearer $TOK" "http://ore-cofre.$NS.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos/fuente-$FUENTE"
python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])' >/tmp/u; rm -f /tmp/s.json
case "$(head -c 5 /tmp/u)" in post*) D=ore-read-postgres ;; bigq*) D=ore-read-bigquery ;; gs:/*) D=ore-read-gcs ;; esac
echo "### catalogo"
timeout 300 /opt/ore/conectores/$D catalogo "$FUENTE" </tmp/u 2>/dev/null; rm -f /tmp/u
echo "### fin"
echo "## ok"
