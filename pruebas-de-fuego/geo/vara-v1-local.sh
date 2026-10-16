# vara-v1-local.sh (ADR 0067 G0·3, caso V1, en local): lo mismo que V1 en vara-celda.sh —la Table de
# municipios que la inducción saca del catálogo de banco_geo_pg, con `geom` declarada como
# Geometry<MultiPolygon, EPSG:25830>, y `ore validate`— con el `ore` de la rama, antes de desplegarlo.
#   ORE=/target/debug/ore CAT=/datos/cat-pg.json sh vara-v1-local.sh
set -e
rm -rf /tmp/v && mkdir -p /tmp/v/ws /tmp/v/v1 && cd /tmp/v/ws && $ORE init >/dev/null
$ORE discover --from "$CAT" --out packages/banco_geo_pg --name banco_geo_pg --owner team:geo >/dev/null 2>&1
cd /tmp/v/v1 && $ORE init >/dev/null && mkdir -p packages/geo/tables
T=$(ls /tmp/v/ws/packages/banco_geo_pg/*/tables/*unicipios*.yaml /tmp/v/ws/packages/banco_geo_pg/tables/*unicipios*.yaml 2>/dev/null | head -1)
sed -E 's/^( *)geom: *\{.*\} *$/\1geom: { type: "Geometry<MultiPolygon, EPSG:25830>", physicalType: "geometry(MultiPolygon,25830)" }/' "$T" \
  | sed -E 's#apiVersion: oos.dev/v1alpha[0-9]+#apiVersion: oos.dev/v1alpha32#' > packages/geo/tables/municipios.yaml
grep -n "geom\|apiVersion" packages/geo/tables/municipios.yaml
$ORE source add --name banco_geo_pg postgres://banco/banco_geo >/dev/null 2>&1
set +e
$ORE validate . ; echo "V1 rc=$?"
# Y la misma columna en un documento de v1alpha31 (antes de la geometría): OOS3001.
sed -i 's#oos.dev/v1alpha32#oos.dev/v1alpha31#' packages/geo/tables/municipios.yaml
$ORE validate . 2>&1 | grep -E "OOS3001" | head -2
