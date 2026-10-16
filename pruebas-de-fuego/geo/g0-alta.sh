# g0-alta.sh (ADR 0067 G0·2): da de alta las fuentes del banco geo que no llevan secreto, DENTRO de
# la celda y con la credencial del agente: BigQuery (`banco_geo_bq`) y los ficheros en GCS
# (`banco_geo_gcs`, gs://…-banco-geo/ficheros). PostGIS lo da de alta g0-postgis.sh. «## ok» / «## falla».
set -e
P=project-8853a180-450d-47be-b83
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
mal=0
for par in "banco_geo_bq bigquery://$P/banco_geo" "banco_geo_gcs gs://$P-banco-geo/ficheros"; do
  set -- $par
  C=$(curl -s -o /tmp/r.json -w '%{http_code}' -X POST -H "authorization: Bearer $TOK" -H 'content-type: application/json' \
    --data-binary "{\"name\":\"$1\",\"url\":\"$2\"}" "http://ore-serve.$NS.svc.cluster.local:8080/fuentes")
  echo "$1: POST /fuentes $C $(head -c 300 /tmp/r.json)"
  case "$C" in 2*|409) ;; *) mal=1 ;; esac
done
[ $mal = 0 ] && echo "## ok" || echo "## falla"
