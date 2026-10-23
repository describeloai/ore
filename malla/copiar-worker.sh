#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# LA PASADA DEL COPIADOR RESIDENTE (`ore-copiador`)
#
#   copiar-worker.sh <pasada> <dataset,dataset,…>
#
# Es la pasada ACOTADA de `malla/48-la-copia.yaml` (`PROGRAMADA`) sin lo que el Job
# paga por nacer: no hay pod, ni init containers, ni espera de Kueue. Hace lo mismo,
# en el mismo orden y con el mismo `ore materialize`:
#
#   1. el testigo de la forja y el del agente (en un tmpfs, renovados cada 5 min);
#   2. un clon del árbol, propio de esta pasada;
#   3. la credencial de cada fuente que lee un dataset, del cofre, SOLO en el
#      entorno de este proceso (nunca a disco);
#   4. para cada dataset: `ore materialize --vista`, y su puntero a la forja en
#      cuanto está (`publicar`, con rebase si la forja avanzó);
#   5. una línea `@@ <dataset> listo|fallo <motivo>` por dataset: es lo que
#      `ore-copiador` cuenta como su estado.
#
# Lo que NO hace: copias por consulta (necesitan DuckDB, que vive en el puesto), ramas,
# `rehacer` ni refrescos programados. Esas siguen por Job.
# ═══════════════════════════════════════════════════════════════════════════
set -u

PASADA=${1:?falta la pasada}
VISTAS=${2:?faltan los datasets}
PUESTO=${PUESTO:-/puesto}
TRABAJO=${TRABAJO:-/trabajo}
DIR="$TRABAJO/p-$PASADA"

: "${FORJA_URL:?falta FORJA_URL}" "${COFRE_URL:?falta COFRE_URL}" "${SECRETOS:?falta SECRETOS}"
: "${ORGANIZACION:?}" "${EMISOR:?}" "${DIRECCION:?}" "${REALM:?}"

T0=$(date +%s)
paso() { echo "### $(( $(date +%s) - T0 )) s · $*"; }
limpiar() { rm -rf "$DIR"; }
trap limpiar EXIT

fallan() { # todos los datasets, con el mismo motivo
  for v in $(echo "$VISTAS" | tr ',' ' '); do echo "@@ $v fallo $1"; done
  exit 1
}

# ── 1 · los testigos, en memoria, renovados si tienen más de 5 minutos ───────
mkdir -p "$PUESTO"
if ! find "$PUESTO/forja" -mmin -5 2>/dev/null | grep -q .; then
  paso "renovando los testigos"
  gcloud secrets versions access latest --secret="$SECRETOS-forja-token" --out-file="$PUESTO/forja.nuevo" 2>/dev/null \
    && chmod 0400 "$PUESTO/forja.nuevo" || fallan "no se pudo leer el testigo de la forja"
  for p in cliente secreto; do
    gcloud secrets versions access latest --secret="$SECRETOS-agente-$p" --out-file="$PUESTO/agente-$p.nuevo" 2>/dev/null \
      && chmod 0400 "$PUESTO/agente-$p.nuevo" || fallan "no se pudo leer la credencial del agente"
  done
  for f in agente-cliente agente-secreto forja; do mv -f "$PUESTO/$f.nuevo" "$PUESTO/$f"; done
fi

# ── 2 · el clon de esta pasada ──────────────────────────────────────────────
export GIT_TERMINAL_PROMPT=0 GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader \
  GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*'
export GIT_CONFIG_VALUE_0="Authorization: token $(cat "$PUESTO/forja")"
mkdir -p "$TRABAJO"
git clone --quiet "$FORJA_URL" "$DIR" 2>&1 || fallan "no se pudo clonar el árbol"
cd "$DIR" || fallan "no hay clon"
mkdir -p "$DIR/calculo"
paso "árbol clonado"

# ── 3 · las fuentes que se leen, y su credencial del cofre ──────────────────
ore view . > "$DIR/vista.txt" 2>&1 || echo "  ⚠️ \`ore view\` dice que algo no compila; se sigue con lo que sí"
FUENTES=""
for QV in $(echo "$VISTAS" | tr ',' ' '); do
  F=$(awk -v n="$QV" '$0==n{m=1;next} m&&/^  ra/{s=$0;sub(/^  [^ ]+ +/,"",s);sub(/ .*/,"",s);print s;exit}' "$DIR/vista.txt")
  for G in $(echo "$F" | tr ',' ' '); do
    [ -z "$G" ] || [ "$G" = lago ] && continue
    case ",$FUENTES," in *",$G,"*) ;; *) FUENTES="${FUENTES:+$FUENTES,}$G" ;; esac
  done
done
paso "fuentes que hay que abrir: ${FUENTES:-ninguna}"
if [ -n "$FUENTES" ]; then
  DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])') \
    || fallan "no se alcanza el proveedor de identidad"
  [ "$DICE" = "$EMISOR/realms/$REALM" ] || fallan "el proveedor de identidad afirma ser $DICE y se esperaba $EMISOR/realms/$REALM"
  TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" \
    -d grant_type=client_credentials -d "client_id=$(cat "$PUESTO/agente-cliente")" \
    -d "client_secret=$(cat "$PUESTO/agente-secreto")" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])') \
    || fallan "el agente no obtuvo testigo"
  for F in $(echo "$FUENTES" | tr ',' ' '); do
    VAR=$(awk -v n="$F" '/^datasources:/{d=1;next} d&&/^[^ ]/{exit} d{if(match($0,/name: *[A-Za-z0-9_]+/)){s=substr($0,RSTART,RLENGTH);sub(/name: */,"",s);c=s} if(match($0,/connectionEnv: *[A-Za-z0-9_]+/)&&c==n){v=substr($0,RSTART,RLENGTH);sub(/connectionEnv: */,"",v);print v;exit}}' ontology.config.yaml)
    [ -n "$VAR" ] || { echo "  ⚠️ el manifiesto no dice el \`connectionEnv\` de \`$F\`"; continue; }
    C=$(curl -s -o "$DIR/s.json" -w '%{http_code}' -H "authorization: Bearer $TOK" \
      "$COFRE_URL/organizaciones/$ORGANIZACION/secretos/fuente-$F")
    if [ "$C" != "200" ]; then
      echo "  ⚠️ el custodio contestó $C para \`fuente-$F\`: sus datasets no se copiarán"
      rm -f "$DIR/s.json"
      continue
    fi
    export "$VAR"="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["valor"])' "$DIR/s.json")"
    rm -f "$DIR/s.json"
    echo "  ✓ credencial de \`$F\` puesta en \`$VAR\` — y no toca el disco"
  done
fi

# ── lo que reclaman las demás ramas (para que --recoger no retire lo suyo) ───
rm -rf "$DIR/ramas"; mkdir -p "$DIR/ramas"
for R in $(git for-each-ref refs/remotes/origin --format='%(refname)' | sed 's#^refs/remotes/origin/##'); do
  [ "$R" = HEAD ] || [ "$R" = main ] && continue
  B=$(git merge-base origin/main "origin/$R") || continue
  git diff --name-only --diff-filter=AM "$B" "origin/$R" -- datasets copias resultados | while read -r F; do
    case "$F" in *.json) mkdir -p "$DIR/ramas/$R/$(dirname "$F")"; git show "origin/$R:$F" > "$DIR/ramas/$R/$F" ;; esac
  done
done

# ── publicar: el commit de los punteros y su push, con rebase si la forja avanzó ──
publicar() {
  export GIT_AUTHOR_NAME=copiador GIT_AUTHOR_EMAIL=copiador@invalido
  export GIT_COMMITTER_NAME=copiador GIT_COMMITTER_EMAIL=copiador@invalido
  git add -A datasets
  if ! git diff --cached --quiet; then
    git commit -q -m "Copia: ${1:-$VISTAS}" || return 1
  fi
  for _ in 1 2 3 4 5; do
    git fetch -q origin main || return 1
    [ -n "$(git rev-list HEAD "^origin/main" | head -1)" ] || return 0
    git push -q origin HEAD:main && return 0
    git rebase -q origin/main || { git rebase --abort 2>/dev/null; return 1; }
  done
  return 1
}

# ── 4 · cada dataset, y su puntero en cuanto está ───────────────────────────
for v in $(echo "$VISTAS" | tr ',' ' '); do
  paso "copiando \`$v\`"
  ore materialize . --vista "$v" --recoger --reclaman "$DIR/ramas" --informe datasets --calculado "$DIR/calculo" 2>&1 \
    || echo "⚠️ \`$v\` no se copió: su informe en datasets/ lo dice"
  if ! publicar "$v"; then
    echo "@@ $v fallo el puntero no se pudo publicar en la forja"
    continue
  fi
  P="datasets/$(echo "$v" | tr '.' '/').json"
  if [ ! -f "$P" ]; then
    # Un dataset en `default` vive sin su schema en el nombre.
    P=$(find datasets -name "$(echo "$v" | awk -F. '{print $NF}').json" 2>/dev/null | head -1)
  fi
  if [ -n "$P" ] && [ -f "$P" ] && grep -q '"estado": *"error"' "$P"; then
    MOTIVO=$(sed -n 's/.*"motivo": *"\(.*\)".*/\1/p' "$P" | head -1)
    echo "@@ $v fallo ${MOTIVO:-la copia terminó en error}"
  else
    paso "\`$v\` publicado"
    echo "@@ $v listo"
  fi
done
