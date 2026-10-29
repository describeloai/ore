#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# EL CORTE: UNA CELDA NUEVA NACE EN EL CATÁLOGO (ADR 0079 ·5g·1c) — de punta a punta, en victor
#
#   bash pruebas-de-fuego/el-corte-nace-en-el-catalogo.sh <celda> <paso>
#
# Contra una celda DE VERDAD: pedida por el camino de producción (`POST
# /organizaciones/{org}/celdas`, la consola), montada por el aprovisionador y retirada al
# acabar. Lo que hace una persona —crear un schema, ingerir, una rama y su propuesta— lo hace una
# persona en la consola: la API de una celda sólo admite a las personas de su organización
# (`acceso.rs`, 0047 A9′), y un atajo aquí probaría otra cosa. Este guion comprueba, después de
# cada paso, lo que tiene que ser verdad por detrás:
#
#   alta          el papel y el esquema `t_<celda>` (suyo), su URL en el almacén, la salida al
#                 catálogo, y `ore-serve` arrancado con «el catálogo · esquema t_<celda>»
#   definicion    (tras crear o renombrar un schema) el último commit con bloque `Punteros:` en
#                 `main` está aplicado en el catálogo (`aplicado(sha)`)
#   ingesta       (tras ingerir) hay punteros vivos en `main` del catálogo, cada uno en el bucket
#                 de la celda, y en git NO hay `datasets/`
#   rama <r>      (tras escribir en la rama `r`, proponer y fusionar) la ref `r` existió con sus
#                 punteros, y el commit de la fusión en `main` está aplicado
#   documentador  la cola de documentos está vacía y hay commits de `agente:documentador` en git
#   mantenimiento lanza el mantenimiento (53) a mano y espera a que acabe bien
#   auditoria     lanza la auditoría (78) a mano: 0 fallos en `t_<celda>`
#   retirada      (tras retirarla) ni papel, ni esquema, ni URL, ni namespace
#
# ⚠️ Lee el catálogo como su superusuario (por `kubectl exec` en la primaria) y git desde el
#   `ore-serve` de la celda (su testigo no sale del pod). No escribe nada en ninguno de los dos.
# ══════════════════════════════════════════════════════════════════════════════
set -uo pipefail
CELDA=${1:?uso: $0 <celda> <paso>}
PASO=${2:?uso: $0 <celda> <paso>}
NS="t-$CELDA"
ESQUEMA="t_${CELDA//-/_}"
FALLOS=0
ok()    { echo "  ✓ $*"; }
falla() { echo "  ✗ $*"; FALLOS=$((FALLOS + 1)); }
fin()   { [ "$FALLOS" = 0 ] && { echo "── $PASO: PASA"; exit 0; } || { echo "── $PASO: FALLA ($FALLOS)"; exit 1; }; }

PRIMARIA=$(kubectl -n catalogo get pod -l cnpg.io/cluster=catalogo,cnpg.io/instanceRole=primary \
  -o jsonpath='{.items[0].metadata.name}')
[ -n "$PRIMARIA" ] || { echo "⛔ no hay primaria del Cluster catalogo"; exit 2; }
Q() { kubectl -n catalogo exec -i "$PRIMARIA" -c postgres -- psql -X -q -t -A -U postgres -d catalogo -c "$1" 2>&1; }
QE() { Q "SET search_path = $ESQUEMA; $1"; }
# git, desde el `ore-serve` de la celda: un espejo en su /tmp, con su testigo, que no sale del pod.
G() {
  kubectl -n "$NS" exec deploy/ore-serve -c ore-serve -- sh -c '
    set -e
    U=$(tr "\0" "\n" < /proc/1/cmdline | grep -A1 -x -- --forja | tail -1)
    export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader \
      GIT_CONFIG_VALUE_0="Authorization: token $(cat /testigo/forja)"
    if [ -d /tmp/e2e.git ]; then git -C /tmp/e2e.git fetch -q --prune origin "+refs/heads/*:refs/heads/*"
    else git clone -q --mirror "$U" /tmp/e2e.git; fi
    cd /tmp/e2e.git && '"$1" 2>&1
}

echo "── $PASO · celda $CELDA · esquema $ESQUEMA"
case "$PASO" in
alta)
  [ "$(Q "SELECT count(*) FROM pg_roles WHERE rolname = '$ESQUEMA'")" = 1 ] && ok "el papel $ESQUEMA" || falla "no hay papel $ESQUEMA"
  D=$(Q "SELECT nspowner::regrole FROM pg_namespace WHERE nspname = '$ESQUEMA'")
  [ "$D" = "$ESQUEMA" ] && ok "el esquema $ESQUEMA, suyo" || falla "el esquema $ESQUEMA: dueño «$D»"
  gcloud secrets describe "$NS-catalogo" --format="value(name)" >/dev/null 2>&1 \
    && ok "su URL en el almacén ($NS-catalogo)" || falla "no hay $NS-catalogo en el almacén"
  kubectl -n "$NS" get networkpolicy salida-al-catalogo >/dev/null 2>&1 \
    && ok "la salida al catálogo" || falla "no hay salida-al-catalogo en $NS"
  kubectl -n "$NS" rollout status deploy/ore-serve --timeout=300s >/dev/null 2>&1 \
    && ok "ore-serve listo" || falla "ore-serve no está listo"
  kubectl -n "$NS" logs deploy/ore-serve -c ore-serve 2>/dev/null | grep -q "el catálogo · esquema $ESQUEMA" \
    && ok "ore-serve arrancó con el catálogo" || falla "ore-serve no dice «el catálogo · esquema $ESQUEMA»"
  V=$(QE "SELECT max(version) FROM esquema_version")
  [ "${V:-0}" -gt 0 ] 2>/dev/null && ok "el esquema del catálogo, versión $V" || falla "sin tablas en $ESQUEMA"
  ;;
definicion)
  SHA=$(G 'git log main --format=%H -n1 --grep="^Punteros: "')
  [ -n "$SHA" ] || { falla "no hay en main ningún commit con bloque Punteros:"; fin; }
  ok "el último commit con bloque: ${SHA:0:12} — $(G "git log -n1 --format=%s $SHA")"
  [ "$(QE "SELECT aplicado('$SHA')")" = t ] && ok "aplicado en el catálogo" || falla "${SHA:0:12} NO está aplicado"
  ;;
ingesta)
  N=$(QE "SELECT count(*) FROM estado WHERE ref = 'main' AND NOT borrado AND metadata_location IS NOT NULL")
  [ "${N:-0}" -gt 0 ] 2>/dev/null && ok "$N punteros vivos en main del catálogo" || falla "ningún puntero vivo en main ($N)"
  FUERA=$(QE "SELECT count(*) FROM estado WHERE ref = 'main' AND NOT borrado AND metadata_location IS NOT NULL
              AND metadata_location NOT LIKE 'gs://%-$NS-copia/%'")
  [ "$FUERA" = 0 ] && ok "todos en gs://…-$NS-copia" || falla "$FUERA punteros fuera del bucket de la celda"
  QE "SELECT '    · ' || activo || ' → ' || right(metadata_location, 60) FROM estado
      WHERE ref = 'main' AND NOT borrado AND metadata_location IS NOT NULL ORDER BY activo LIMIT 5"
  F=$(G 'git ls-tree -r --name-only main | grep -c "^datasets/" || true')
  [ "$F" = 0 ] && ok "en git, 0 ficheros bajo datasets/" || falla "en git hay $F ficheros bajo datasets/"
  ;;
rama)
  R=${3:?rama: falta el nombre de la rama}
  N=$(QE "SELECT count(*) FROM historia WHERE ref = '$R'")
  [ "${N:-0}" -gt 0 ] 2>/dev/null && ok "la rama $R escribió $N cambios en el catálogo" || falla "nada de la rama $R en el catálogo ($N)"
  SHA=$(G 'git log main --merges --format=%H -n1')
  [ -n "$SHA" ] || { falla "no hay ningún merge en main"; fin; }
  ok "el último merge: ${SHA:0:12} — $(G "git log -n1 --format=%s $SHA")"
  G "git log -n1 --format=%B $SHA" | grep -q "^Punteros: " && ok "lleva el plan (bloque Punteros:)" \
    || falla "el merge no lleva bloque Punteros: (¿una propuesta sin datos?)"
  [ "$(QE "SELECT aplicado('$SHA')")" = t ] && ok "aplicado en el catálogo" || falla "el merge ${SHA:0:12} NO está aplicado"
  ;;
documentador)
  N=$(QE "SELECT count(*) FROM documentos")
  [ "$N" = 0 ] && ok "la cola de documentos, vacía" || falla "quedan $N documentos en la cola"
  D=$(G 'git log main --format="%an %s" | grep -c documentador || true')
  [ "${D:-0}" -gt 0 ] 2>/dev/null && ok "$D commits del documentador en git" || falla "ningún commit del documentador"
  ;;
mantenimiento)
  J="e2e-mantenimiento-$(date +%s)"
  kubectl -n "$NS" create job "$J" --from=cronjob/mantenimiento >/dev/null || { falla "no se pudo lanzar el mantenimiento"; fin; }
  kubectl -n "$NS" wait --for=condition=complete "job/$J" --timeout=900s >/dev/null 2>&1 \
    && ok "el mantenimiento acabó bien" || falla "el mantenimiento no acabó bien: kubectl -n $NS logs job/$J"
  kubectl -n "$NS" logs "job/$J" -c mantener --tail=6 2>/dev/null | sed 's/^/    /'
  ;;
auditoria)
  J="e2e-auditoria-$(date +%s)"
  kubectl -n catalogo create job "$J" --from=cronjob/auditar-el-catalogo >/dev/null || { falla "no se pudo lanzar la auditoría"; fin; }
  kubectl -n catalogo wait --for=condition=complete "job/$J" --timeout=900s >/dev/null 2>&1 || true
  L=$(kubectl -n catalogo logs "job/$J" 2>/dev/null | grep " $ESQUEMA ")
  echo "$L" | sed 's/^/    /'
  echo "$L" | grep -qE "^PASA +$ESQUEMA " && ! echo "$L" | grep -q FALLA && ok "la auditoría pasa en $ESQUEMA" \
    || falla "la auditoría no pasa en $ESQUEMA"
  ;;
retirada)
  [ "$(Q "SELECT count(*) FROM pg_roles WHERE rolname = '$ESQUEMA'")" = 0 ] && ok "sin papel" || falla "el papel $ESQUEMA sigue"
  [ "$(Q "SELECT count(*) FROM pg_namespace WHERE nspname = '$ESQUEMA'")" = 0 ] && ok "sin esquema" || falla "el esquema $ESQUEMA sigue"
  gcloud secrets describe "$NS-catalogo" >/dev/null 2>&1 && falla "$NS-catalogo sigue en el almacén" || ok "sin URL en el almacén"
  kubectl get ns "$NS" >/dev/null 2>&1 && falla "el namespace $NS sigue (Flux lo poda tras el enganche)" || ok "sin namespace"
  ;;
*) echo "paso desconocido: $PASO"; exit 64 ;;
esac
fin
