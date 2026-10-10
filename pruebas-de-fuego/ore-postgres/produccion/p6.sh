#!/usr/bin/env bash
# P6 en producción (ADR 0058, «la ventana 1»): dormir, despertar y el pool con VMs de verdad, entrando
# desde internet. Cada paso dice qué tiene que estar encendido en la malla 86.
#
#   p6.sh dormir          [--dormir] duerme a su hora (dormir_tras 60); dormido, la VM desaparece y
#                         el nodo `pg` se libera: cuánto tarda (dormido cuesta 0); el LSN de /terminate
#   p6.sh http-corto      [--dormir] consultas cortas por HTTP cada 20 s, 4 min: ¿cuentan como
#                         actividad (sigue despierto) o se duerme con clientes? (el hallazgo de P6·7)
#   p6.sh despertar [N]   [--dormir, pool 0] N despertares en frío desde internet: p50/p95
#   p6.sh pool [N]        [--dormir, --pool-computos 2] N despertares desde el pool: p50/p95; el pool
#                         repuesto; la VM del pool, con los límites del endpoint que la recibe
#   p6.sh limpiar
PRUEBA=p6
source "$(dirname "$0")/comun.sh"
A=demo

proyecto() {   # carga P ROL CLAVE HOST (lo crea la primera vez, con dormir_tras 60)
  local f="$ORE_PG_TRABAJO/p6-$A"
  if [ ! -s "$f" ]; then
    local p="p6-$A-$(date +%s | tail -c 5)"
    local r; r=$(crea "$A" "$p") || exit 1
    case "$(ajusta "$A" "$p" '{"dormir_tras":"60"}')" in 2*) ;; *) echo "✗ ajustes" >&2; exit 1;; esac
    echo "$p $r" > "$f"
  fi
  read -r P ROL CLAVE HOST _ < "$f"
}
args86() { k get deploy ore-postgres -o jsonpath='{.spec.template.spec.containers[0].args}'; }
pide_flag() { case "$(args86)" in *"$1"*) ;; *) echo "✗ falta $1 en ore-postgres (su parche de la malla 86)" >&2; exit 1;; esac; }

dormir() {
  pide_flag --dormir
  echo "── dormir a su hora, y dormido cuesta 0"
  proyecto
  local vm; vm=$(computo_de "$P")
  [ "$(pc_psql "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c "create table if not exists t as select g from generate_series(1, 10000) g; select count(*) from t")" = 10000 ] \
    && bien "10 000 filas, desde internet" || mal "escribir"
  local fin; fin=$(date +%s)
  dormido_ep "$P" 300 && numero p6_dormido_tras_s $(( $(date +%s) - fin )) || mal "no se durmió ($(observado "$P"))"
  local t0; t0=$(date +%s)
  until ! kc get neonvm "$vm" >/dev/null 2>&1 || [ $(( $(date +%s) - t0 )) -ge 120 ]; do sleep 2; done
  kc get neonvm "$vm" >/dev/null 2>&1 && mal "la VM $vm sigue" || bien "la VM $vm, fuera"
  numero p6_lsn_al_dormir "\"$(plano_sql "select lsn_al_dormir from plano.endpoint where proyecto = '$P'")\""
  # El nodo: sólo se libera si no queda NINGUNA VM en el pool (otros endpoints, el pool precalentado).
  local otras; otras=$(kc get neonvm --no-headers 2>/dev/null | grep -c .)
  if [ "$otras" = 0 ]; then
    t0=$(date +%s)
    until [ "$(nodos_pg)" = 0 ] || [ $(( $(date +%s) - t0 )) -ge 1800 ]; do sleep 15; done
    [ "$(nodos_pg)" = 0 ] && numero p6_nodo_pg_libre_tras_s $(( $(date +%s) - t0 )) || mal "el nodo pg sigue a los 30 min"
  else
    echo "  · quedan $otras VMs en ore-pg-computo: el nodo no se libera por este endpoint (se mide en P7 de noche)"
  fi
}

http_corto() {
  pide_flag --dormir
  echo "── consultas cortas por HTTP cada 20 s durante 4 min (dormir_tras 60)"
  proyecto
  pc_psql "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c "select 1" >/dev/null   # despierto al empezar
  docker volume create p5prod-npm >/dev/null
  docker run --rm -v p5prod-npm:/w/node_modules -w /w -e URL="postgresql://$ROL:$CLAVE@$HOST/$P" "$NODE_IMAGEN" sh -c '
    [ -d node_modules/@neondatabase/serverless ] || npm i -s --no-save --no-package-lock @neondatabase/serverless@1.1.0 >/dev/null 2>&1
    node -e "
      const { neon } = require(\"@neondatabase/serverless\"); const sql = neon(process.env.URL);
      (async () => { for (let i = 0; i < 12; i++) { const t = Date.now(); await sql\`select 1\`; console.log(\"  · consulta \" + i + \": \" + (Date.now() - t) + \" ms\"); await new Promise(r => setTimeout(r, 20000)); } })();
    "' | tail -3
  local o; o=$(observado "$P")
  case "$o" in
    listo) bien "sigue despierto: las consultas cortas por HTTP cuentan como actividad"; numero p6_http_corto_mantiene_despierto true;;
    *) mal "se durmió ($o) con clientes por HTTP: la actividad hay que apuntarla también desde el proxy (P6·7)"; numero p6_http_corto_mantiene_despierto false;;
  esac
}

despiertos() {   # despiertos N CLAVE_NUMERO → N ciclos de dormir y despertar desde internet
  local n=$1 clave=$2 v=() x
  proyecto
  for i in $(seq 1 "$n"); do
    dormido_ep "$P" 300 || { mal "ciclo $i: no se durmió"; continue; }
    x=$(pc_ms "$CLAVE" "$(conn "$HOST" "$ROL" "$P" 120)")
    case "$x" in fallo*) mal "ciclo $i: $x";; *) v+=("$x"); echo "  · despertar $i: $x ms ($(computo_de "$P"))";; esac
  done
  [ ${#v[@]} -gt 0 ] || return
  numero "${clave}_p50_ms" "$(percentil 0.5 "${v[@]}")"
  numero "${clave}_p95_ms" "$(percentil 0.95 "${v[@]}")"
  numero "${clave}_n" "${#v[@]}"
}

despertar() {
  pide_flag --dormir
  case "$(args86)" in *--pool-computos*) echo "✗ el pool está encendido: el frío se mide antes" >&2; exit 1;; esac
  echo "── despertar en frío desde internet (VM de verdad), ${1:-10} veces"
  despiertos "${1:-10}" p6_despertar_frio
}

pool() {
  pide_flag --pool-computos
  echo "── el pool"
  local t0; t0=$(date +%s)
  until [ "$(plano_sql "select count(*) from plano.pool_computo where estado = 'libre'")" -ge 2 ] || [ $(( $(date +%s) - t0 )) -ge 900 ]; do sleep 5; done
  numero p6_pool_libres_tras_s $(( $(date +%s) - t0 ))
  echo "── despertar desde el pool, ${1:-10} veces"
  despiertos "${1:-10}" p6_despertar_pool
  # La VM que sirve ahora salió del pool: ¿tiene los límites del endpoint o los genéricos (0,25–2)?
  proyecto
  local vm; vm=$(computo_de "$P")
  echo "  · la VM de ahora ($vm): cpus $(kc get neonvm "$vm" -o jsonpath='{.spec.guest.cpus}') · memoria $(kc get neonvm "$vm" -o jsonpath='{.spec.guest.memorySlots}') · el endpoint pide CU $(plano_sql "select cu_min || '–' || cu_max from plano.endpoint where proyecto = '$P'")"
  echo "  · VMs en ore-pg-computo: $(kc get neonvm --no-headers | grep -c .) ($(kc get neonvm --no-headers | grep -c '^pool-') del pool) · nodos pg: $(nodos_pg)"
}

limpiar() {
  [ -s "$ORE_PG_TRABAJO/p6-$A" ] && { read -r p _ < "$ORE_PG_TRABAJO/p6-$A"; borra "$A" "$p"; rm -f "$ORE_PG_TRABAJO/p6-$A"; echo "  · $p borrado"; }
}

case "${1:-}" in
  dormir|despertar|pool|limpiar) "$@";;
  http-corto) http_corto;;
  *) sed -n '2,15p' "$0"; exit 64;;
esac
echo
[ $fallos = 0 ] && echo "P6 (producción) ✓ ${1}" || { echo "P6 (producción) ✗ $fallos fallos"; exit 1; }
