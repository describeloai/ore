#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# CORTAR UNA CELDA · una celda que vivía en git pasa al catálogo transaccional (ADR 0079 ·5g·2)
#
#   bash malla/80-cortar-una-celda.sh <celda>             # en seco: dice lo que haría
#   APLICAR=1 bash malla/80-cortar-una-celda.sh <celda>   # lo hace
#
# La celda queda sin servicio unos minutos. El orden es lo que importa: el catálogo tiene que tener
# la historia de git ANTES de que nadie escriba en él. En demo (2026-10-29) el render se aplicó entero
# a la vez, el copiador nuevo escribió antes que `ponerse-al-dia` de ore-serve, y la carga se saltó.
#
#   ① la foto de antes: cada puntero de git, por rama
#   ② Flux en pausa (malla, enganches y los de la celda: el padre reaplica a los hijos)
#   ③ ore-serve y el copiador a 0, y ningún Job a medias: nadie escribe en git
#   ④ `iam.celda.punteros = 'catalogo'` y la pasada del aprovisionador de la celda: el alta (papel,
#      esquema, URL) y el render, empujado pero SIN aplicar
#   ⑤ la carga: `79-cargar-el-catalogo.sh` (la historia entera y su auditoría, 0 diferencias)
#   ⑤b la foto de después, ANTES de que nada escriba: el catálogo frente a la de antes
#   ⑥ Flux en marcha: el render se aplica; `ponerse-al-dia` encuentra la historia y arranca
#
# Si ④, ⑤ o ⑤b fallan: `punteros = 'ficheros'`, la pasada del aprovisionador y Flux en marcha, y la
# celda vuelve a git como estaba (nadie ha escrito en el catálogo). Después de ⑥, no hay vuelta:
# lo que se escriba vive sólo en el catálogo.
# ═══════════════════════════════════════════════════════════════════════════
set -uo pipefail
CELDA=${1:?uso: $0 <celda>}
NS="t-$CELDA"
ESQUEMA="t_${CELDA//-/_}"
AQUI=$(cd "$(dirname "$0")" && pwd)
FOTOS=${FOTOS:-${TMPDIR:-/tmp}/corte-$CELDA}
mkdir -p "$FOTOS"
paso() { printf '\n── %s\n' "$*"; }
ok() { echo "  ✓ $*"; }
para() { echo "  ⛔ $*"; exit 1; }
haz() { if [ -n "${APLICAR:-}" ]; then "$@"; else echo "  ~ haría: $*"; fi; }
iam() { kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc "$1" 2>/dev/null | tr -d '\r'; }
flux() { # <true|false> — malla, enganches y los de la celda
  for k in malla enganches "inquilino-$CELDA" "trabajo-$CELDA"; do
    haz kubectl -n flux-system patch kustomization "$k" --type merge -p "{\"spec\":{\"suspend\":$1}}"
  done
}
# git de la celda, desde su ore-serve: lo de `$1` dentro de un espejo recién traído.
git_de_la_celda() {
  kubectl -n "$NS" exec deploy/ore-serve -c ore-serve -- sh -c '
    set -e
    U=$(tr "\0" "\n" < /proc/1/cmdline | grep -A1 -x -- --forja | tail -1)
    export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader \
      GIT_CONFIG_VALUE_0="Authorization: token $(cat /testigo/forja)"
    rm -rf /tmp/corte && git clone -q --mirror "$U" /tmp/corte && cd /tmp/corte && '"$1"'
    rm -rf /tmp/corte' 2>&1 | grep -v '^warning'
}

echo "── el corte de $CELDA ($NS, esquema $ESQUEMA) · ${APLICAR:+DE VERDAD}${APLICAR:-EN SECO}"

paso "⓪ ANTES DE EMPEZAR"
[ "$(iam "select punteros from iam.celda where nombre = '$CELDA'")" = ficheros ] \
  || para "$CELDA no está en \`ficheros\` (o no existe)"
ok "$CELDA vive en git"
H=$(date -u +%H%M); H=${H#0}
[ "$H" -ge 345 ] && [ "$H" -le 415 ] && para "es la hora del mantenimiento (04:00 UTC): otro rato"
ACTIVOS=$(kubectl -n "$NS" get jobs -o jsonpath='{range .items[?(@.status.active)]}{.metadata.name} {end}')
[ -z "$ACTIVOS" ] || para "Jobs a medias en $NS: $ACTIVOS"
ok "ningún Job a medias"

paso "① LA FOTO DE ANTES"
git_de_la_celda 'for r in $(git for-each-ref refs/heads --format="%(refname:short)"); do
  for p in $(git ls-tree -r --name-only "$r" -- datasets/); do
    echo "$r|$p|$(git show "$r:$p" | tr -d "\n " | grep -o "\"metadata_location\":\"[^\"]*\"" | cut -d\" -f4)"
  done; done' | sort > "$FOTOS/antes.txt"
[ -s "$FOTOS/antes.txt" ] || para "la foto de antes salió vacía"
ok "$(wc -l < "$FOTOS/antes.txt") punteros en $(cut -d'|' -f1 "$FOTOS/antes.txt" | sort -u | wc -l) ramas → $FOTOS/antes.txt"

paso "② FLUX EN PAUSA"
flux true

paso "③ NADIE ESCRIBE EN GIT"
haz kubectl -n "$NS" scale deploy ore-serve ore-copiador --replicas=0
if [ -n "${APLICAR:-}" ]; then
  for i in $(seq 1 60); do
    kubectl -n "$NS" get pods --no-headers 2>/dev/null | grep -qE '^(ore-serve|ore-copiador)-' || break
    sleep 2
  done
  kubectl -n "$NS" get pods --no-headers | grep -E '^(ore-serve|ore-copiador)-' && para "siguen vivos"
  ok "ore-serve y el copiador, parados"
fi

paso "④ EL ALTA Y EL RENDER, SIN APLICAR"
haz kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc \
  "update iam.celda set punteros = 'catalogo' where nombre = '$CELDA'"
JOB="corte-$CELDA-$(date -u +%H%M%S)"
if [ -n "${APLICAR:-}" ]; then
  kubectl -n ore-system create job "$JOB" --from=cronjob/aprovisionador --dry-run=client -o json \
    | python3 -c "import json,sys; j=json.load(sys.stdin); c=j['spec']['template']['spec']['containers'][0]; c['command']+=['$CELDA']; print(json.dumps(j))" \
    | kubectl apply -f - >/dev/null
  kubectl -n ore-system wait --for=condition=complete "job/$JOB" --timeout=900s >/dev/null 2>&1 \
    || para "la pasada del aprovisionador no acabó: kubectl -n ore-system logs job/$JOB (vuelta atrás: arriba)"
  kubectl -n ore-system logs "job/$JOB" --all-containers | grep -E "el papel $ESQUEMA|el esquema $ESQUEMA|empujado a $NS/compartimento" | sed 's/^/ /'
  gcloud secrets describe "$NS-catalogo" --format="value(name)" >/dev/null 2>&1 || para "no hay $NS-catalogo"
  ok "el alta hecha y el render empujado (Flux en pausa: sin aplicar)"
else
  echo "  ~ haría: la pasada del aprovisionador sólo para $CELDA (Job $JOB)"
fi

paso "⑤ LA CARGA"
if [ -n "${APLICAR:-}" ]; then
  bash "$AQUI/79-cargar-el-catalogo.sh" "$CELDA" || para "la carga no cuadró (vuelta atrás: arriba)"
  ok "la historia en $ESQUEMA, 0 diferencias"
else
  echo "  ~ haría: bash malla/79-cargar-el-catalogo.sh $CELDA"
fi

paso "⑤b LA FOTO DE DESPUÉS, antes de que nadie escriba"
if [ -n "${APLICAR:-}" ]; then
  PRIMARIA=$(kubectl -n catalogo get pod -l cnpg.io/cluster=catalogo,cnpg.io/instanceRole=primary -o name)
  kubectl -n catalogo exec "$PRIMARIA" -c postgres -- psql -X -q -t -A -U postgres -d catalogo -c \
    "SELECT ref || '|' || activo || '|' || coalesce(metadata_location, '') FROM $ESQUEMA.estado
     WHERE ref = 'main' AND NOT borrado" | tr -d '\r' | sort > "$FOTOS/despues.txt"
  # Sólo main: en el catálogo una rama guarda lo suyo y hereda lo demás de main, y en git cada rama
  # lo lleva todo. Las ramas las compara la auditoría de ⑤ (`ore-catalogo`, que sabe heredar).
  grep '^main|' "$FOTOS/antes.txt" > "$FOTOS/antes-main.txt"
  if diff "$FOTOS/antes-main.txt" "$FOTOS/despues.txt" > "$FOTOS/diferencias.txt"; then
    ok "el catálogo dice lo mismo que git: $(wc -l < "$FOTOS/despues.txt") punteros"
  else
    echo "  ✗ diferencias ($(grep -c '^[<>]' "$FOTOS/diferencias.txt")): $FOTOS/diferencias.txt"
    head -10 "$FOTOS/diferencias.txt" | sed 's/^/     /'
    exit 1
  fi
fi
paso "⑥ FLUX EN MARCHA"
flux false
haz kubectl -n flux-system annotate kustomization malla enganches "inquilino-$CELDA" "trabajo-$CELDA" \
  "reconcile.fluxcd.io/requestedAt=$(date +%s)" --overwrite
if [ -n "${APLICAR:-}" ]; then
  for i in $(seq 1 60); do
    case "$(kubectl -n "$NS" get deploy ore-serve -o jsonpath='{.spec.template.spec.initContainers[*].name}')" in
      *ponerse-al-dia*) break ;;
    esac
    sleep 5
  done
  kubectl -n "$NS" rollout status deploy/ore-serve --timeout=600s || para "ore-serve no arranca"
  P=$(kubectl -n "$NS" get pod -o name | grep ore-serve | head -1)
  kubectl -n "$NS" logs "$P" -c ponerse-al-dia | tail -1 | cut -c1-300 | sed 's/^/   /'
  kubectl -n "$NS" logs "$P" -c ore-serve | grep -m1 punteros | sed 's/^/   /'
  ok "ore-serve en el catálogo"
fi

echo; echo "── $CELDA: ${APLICAR:+cortada}${APLICAR:-en seco, nada tocado}"
