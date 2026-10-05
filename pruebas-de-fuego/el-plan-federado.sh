#!/usr/bin/env bash
# EL PLAN DE UNA LECTURA EN VIVO (ADR 0053 F4·1) — `ore federate`, sin red.
#
#   ORE=target/debug/ore bash pruebas-de-fuego/el-plan-federado.sh
#
# Un árbol con una fuente y tres tablas (una barata, una `forbidden` y una con
# `requiredFilters`), y la política de main aparte. Cada paso del coordinador
# que es del árbol dice lo suyo: el interruptor de la fuente (de main), la
# tabla, las columnas, el empuje, el coste y el conducto.
set -u
ORE="${ORE:-target/debug/ore}"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
MAL=0
falla() { echo "  ✗ $*"; MAL=1; }
dice()  { echo "  ✓ $*"; }

A="$TMP/rama"; M="$TMP/main"
mkdir -p "$A/packages/pg/public/tables" "$M"
cat > "$A/ontology.config.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: prueba, version: 0.1.0 }
datasources:
  - name: pg
    type: postgres
    connectionEnv: PRUEBA_PG_URL
YAML
cat > "$A/conduits.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: prueba }
spec:
  owner: team:prueba
  conduits:
    contextSurface.workspace: { oos.maturity: DRAFT }
YAML
cat > "$A/packages/pg/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: pg, version: 0.1.0, status: draft, domain: pg }
spec: { owner: "team:prueba", exports: [pg.public.barata, pg.public.prohibida, pg.public.exigente] }
YAML
cat > "$A/packages/pg/public/schema.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: public, namespace: pg }
spec: { owner: team:prueba }
YAML
tabla() { # <nombre> <reads>
cat > "$A/packages/pg/public/tables/$1.yaml" <<YAML
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: $1, namespace: pg, schema: public }
spec:
  datasource: pg
  object: "public.$1"
  columns:
    id: { type: Integer, physicalType: integer, required: true }
    pais: { type: String, physicalType: text }
  reads:
$2
  changes:
    key: [id]
YAML
}
tabla barata "    fullScan: cheap
    predicatePushdown: [eq, in]"
tabla prohibida "    fullScan: forbidden
    predicatePushdown: [eq]"
tabla exigente "    fullScan: expensive
    predicatePushdown: [eq, in, range]
    requiredFilters: [pais]"

# La política de main: al principio, la fuente apagada y sin conducto.
cp "$A/ontology.config.yaml" "$A/conduits.yaml" "$M/"

plan() { "$ORE" federate --path "$A" --policy "$M" "$@" 2>&1; }
espera() { # <qué> <campo=valor…> -- <args>
  local que="$1"; shift
  local conds=()
  while [ "$1" != "--" ]; do conds+=("$1"); shift; done
  shift
  local r; r=$(plan "$@")
  for c in "${conds[@]}"; do
    case "$r" in *"\"${c%%=*}\":${c#*=}"*) ;; *) falla "$que: se esperaba ${c} y salió $r"; return ;; esac
  done
  dice "$que"
}

espera "fuente apagada en main → 403 federacion" 'http=403' 'codigo="federacion"' -- --table pg.public.barata
# Encender en la RAMA no basta: manda main.
"$ORE" source federation pg on --path "$A" >/dev/null
espera "encendida sólo en la rama → sigue 403 (manda main)" 'codigo="federacion"' -- --table pg.public.barata
# Encender en main, sin conducto todavía (la rama sí lo tiene ahora: no cuenta).
sed -i 's/^    connectionEnv: PRUEBA_PG_URL$/    connectionEnv: PRUEBA_PG_URL\n    federation: true/' "$M/ontology.config.yaml"
espera "encendida en main, sin federation.read en main → 403 OOS4011" 'http=403' 'codigo="OOS4011"' -- --table pg.public.barata
"$ORE" source federation pg on --path "$M" >/dev/null
grep -q "federation.read" "$M/conduits.yaml" && dice "encender autoriza federation.read en conduits.yaml" || falla "encender no autorizó el conducto"
espera "todo en main → ok, con su objeto y su proyección" 'ok=true' 'objeto="public.barata"' 'tipo="postgres"' 'env="PRUEBA_PG_URL"' -- --table pg.public.barata --columns id,pais
espera "una tabla que no existe → 404" 'http=404' -- --table pg.public.nada
espera "una columna que no tiene → 422" 'http=422' -- --table pg.public.barata --columns id,edad
espera "un filtro que la tabla no deja empujar → 422 empuje" 'codigo="empuje"' -- --table pg.public.barata --filters '[{"columna":"id","operador":"gt","valor":"3"}]'
espera "un filtro que sí → ok, empujado" 'ok=true' -- --table pg.public.barata --filters '[{"columna":"pais","operador":"eq","valor":"ES"}]'
espera "forbidden sin filtro → 422 OOS2044" 'codigo="OOS2044"' -- --table pg.public.prohibida
espera "forbidden con filtro empujado → ok" 'ok=true' -- --table pg.public.prohibida --filters '[{"columna":"id","operador":"eq","valor":"1"}]'
espera "requiredFilters sin pais → 422 OOS2045" 'codigo="OOS2045"' -- --table pg.public.exigente --filters '[{"columna":"id","operador":"gt","valor":"1"}]'
espera "requiredFilters con pais in → ok, expensive" 'ok=true' 'fullScan="expensive"' -- --table pg.public.exigente --filters '[{"columna":"pais","operador":"in","valor":["ES","FR"]}]'
# Apagar en main la apaga en todas las ramas.
"$ORE" source federation pg off --path "$M" >/dev/null
espera "apagada en main → 403 en la rama" 'codigo="federacion"' -- --table pg.public.barata

echo
[ "$MAL" = 0 ] && echo "✓ el plan federado (0053 F4·1)" || { echo "✗ el plan federado"; exit 1; }
