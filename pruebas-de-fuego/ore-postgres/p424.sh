#!/usr/bin/env bash
# P4·2·4 · borrar no deja huella (ADR 0058). Hecho cuando: crear un proyecto con una rama y datos, y
# borrarlo todo por el API de `ore-postgres`, deja el bucket y los safekeepers COMO ESTABAN (medido).
#
# Una foto entera del almacenamiento antes y después —todos los objetos del bucket, los directorios de
# cada safekeeper, los tenants del pageserver— y entre medias, lo bastante escrito (~64 MB por rama)
# para que los safekeepers descarguen WAL a GCS y el pageserver suba capas. Lo que cuenta es la
# diferencia entre las dos fotos: tiene que ser vacía.
#
#   p424.sh [celda] [MB por rama]      (demo, 64)
export ORE_PG_COMPUTO=pod
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}; MB=${2:-64}
P="p424-$(date +%s | tail -c 6)"
PRUEBA=p424
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
BUCKET=gs://ore-pg-almacen-euw1
R="/v1/postgres/proyectos/$P/ramas"
computo() { ORE_PG_VM=$1 bash "$AQUI/vm.sh" "$2" >/dev/null || { echo "✗ el cómputo $1 no arranca" >&2; return 1; }; leer "pod-$1"; }
limpiar() { for v in p424-main p424-dev; do k delete pod "$v" --ignore-not-found --wait=true >/dev/null; \
  k delete configmap "$v-config" --ignore-not-found >/dev/null; done; }
trap 'limpiar; for c in $CELDAS_ABIERTAS; do kubectl -n t-$c delete networkpolicy prueba-$PRUEBA --ignore-not-found >/dev/null; done' EXIT

# ⚠️ `gcloud` en Git Bash se rompe con MSYS_NO_PATHCONV=1 (que entorno.sh pone para kubectl): se le
#   quita. Medido: con ella, el listado salía vacío sin error y la foto no veía el bucket.
gcs_ls() { env -u MSYS_NO_PATHCONV gcloud storage ls -r "$1"; }
foto() {   # foto FICHERO → una línea por cosa que hay
  local g; g=$(gcs_ls "$BUCKET/**") || { echo "✗ no se pudo listar el bucket" >&2; exit 1; }
  [ -n "$g" ] || { echo "✗ el bucket sale vacío: una foto que no lo ve no mide nada" >&2; exit 1; }
  { echo "$g" | grep '^gs://' | sed 's/^/gcs /'
    for i in 0 1 2; do k exec safekeeper-$i -- sh -c 'ls -1 /data' 2>/dev/null | grep -E '^[0-9a-f]{32}$' | sed "s/^/sk$i /"; done
    pageserver GET /v1/tenant | python -c 'import json,sys; [print("ps", t["id"]) for t in json.load(sys.stdin)]'
  } | sort > "$1"
}
del_tenant() { grep -c "$TENANT" "$1"; }

echo "── la foto de antes"
foto "$ORE_PG_TRABAJO/p424-antes"; echo "  · $(wc -l < "$ORE_PG_TRABAJO/p424-antes") cosas"

echo "── $A crea $P y una rama dev; ~$MB MB en cada una"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'")
espera 202 "crear" "${L[0]}"; OP=$(campo "${L[0]}" operacion id); TENANT=$(campo "${L[0]}" proyecto tenant)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide POST $R '{\"id\":\"dev\"}'")
OP=$(campo "${L[1]}" operacion id); DEV=$(campo "${L[1]}" rama timeline)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $R/main")
MAIN=$(campo "${L[1]}" timeline)
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ tenant $TENANT · main $MAIN · dev $DEV" || { echo "  ✗ ${L[0]:0:300}"; exit 1; }
guardar tenant "$TENANT"
FILAS=$(( MB * 1024 * 1024 / 120 ))   # ~120 B por fila con su cabecera
for r in main dev; do
  TL=$MAIN; [ $r = dev ] && TL=$DEV
  IP=$(computo "p424-$r" "$TL") || exit 1
  q "$IP" "create table p424 (n int, relleno text); insert into p424 select g, repeat('x', 80) from generate_series(1, $FILAS) g" >/dev/null
  echo "  · $r: $(q "$IP" "select pg_size_pretty(pg_total_relation_size('p424'))") escritos"
done
echo "  · se espera a que el WAL baje a GCS (hasta 3 min)"
for i in $(seq 1 18); do [ "$(gcs_ls "$BUCKET/safekeeper/$TENANT/**" 2>/dev/null | grep -c '^gs://')" -gt 0 ] && break; sleep 10; done
foto "$ORE_PG_TRABAJO/p424-durante"
echo "  · durante: $(del_tenant "$ORE_PG_TRABAJO/p424-durante") cosas del tenant ($(grep -c "^gcs .*/safekeeper/$TENANT" "$ORE_PG_TRABAJO/p424-durante") de WAL en GCS, $(grep -c "^gcs .*/pageserver/.*$TENANT" "$ORE_PG_TRABAJO/p424-durante") del pageserver en GCS, $(grep -c "^sk. $TENANT" "$ORE_PG_TRABAJO/p424-durante") safekeepers con su directorio)"

echo "── se borra todo por el API: dev, y el proyecto"
limpiar
T0=$(date +%s)
mapfile -t L < <(en "$A" "pide DELETE $R/dev")
OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide DELETE /v1/postgres/proyectos/$P")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ dev, borrada" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }
OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ el proyecto, borrado a los $(( $(date +%s)-T0 )) s" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }

echo "── la foto de después"
foto "$ORE_PG_TRABAJO/p424-despues"
SOBRA=$(comm -13 "$ORE_PG_TRABAJO/p424-antes" "$ORE_PG_TRABAJO/p424-despues")
FALTA=$(comm -23 "$ORE_PG_TRABAJO/p424-antes" "$ORE_PG_TRABAJO/p424-despues")
if [ -z "$SOBRA" ] && [ -z "$FALTA" ]; then echo "  ✓ igual que antes: $(wc -l < "$ORE_PG_TRABAJO/p424-despues") cosas, ni una más ni una menos"
else
  [ -n "$SOBRA" ] && { echo "  ✗ sobra ($(echo "$SOBRA" | wc -l)):"; echo "$SOBRA" | head -15 | sed 's/^/      /'; fallos=$((fallos+1)); }
  [ -n "$FALTA" ] && { echo "  ✗ falta ($(echo "$FALTA" | wc -l)) — algo de otro se borró:"; echo "$FALTA" | head -15 | sed 's/^/      /'; fallos=$((fallos+1)); }
fi

echo
[ $fallos = 0 ] && echo "P4·2·4 ✓ todo" || { echo "P4·2·4 ✗ $fallos fallos"; exit 1; }
