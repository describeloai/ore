#!/usr/bin/env bash
# P2·6 · la aceptación del almacenamiento de producción (ADR 0058). Cada prueba por separado:
#   p26.sh escrituras | safekeepers | c4 | controlador | borrar
# Montaje: entorno.sh (ns ore-pg, autenticación), cliente.yaml, tenant.sh y vm.sh (cómputo en pod).
source "$(dirname "$0")/entorno.sh"
IP=$(ip_pod); [ -n "$IP" ] || { echo "✗ sin cómputo (vm.sh)"; exit 1; }
E="k exec cliente -- env PGPASSWORD=cloud_admin"
sql() { q "$IP" "$1"; }

# escritor: inserta una fila cada 0,2 s durante <s> segundos por UNA sesión y apunta la hora de
# cada commit confirmado; si la conexión cae, se reconecta (lo que vería una aplicación)
escritor() {
  sql "create table if not exists latido(n int primary key, t timestamptz default clock_timestamp())" >/dev/null
  k exec cliente -- bash -c "
    i=\$(( \$(PGPASSWORD=cloud_admin psql -h $IP -p 55433 -U cloud_admin -d postgres -Atc 'select coalesce(max(n),0) from latido') ))
    fin=\$(( \$(date +%s) + $1 )); : > /tmp/ok.log
    while [ \$(date +%s) -lt \$fin ]; do i=\$((i+1))
      if PGPASSWORD=cloud_admin PGCONNECT_TIMEOUT=3 psql -h $IP -p 55433 -U cloud_admin -d postgres -qAtc \"insert into latido(n) values (\$i)\" >/dev/null 2>&1
      then echo \"\$i \$(date +%s.%N)\" >> /tmp/ok.log; fi
      sleep 0.2
    done" &
}
# huecos: el mayor intervalo entre commits confirmados, y si todo lo confirmado está en la base
balance() {
  k exec cliente -- cat /tmp/ok.log > "$ORE_PG_TRABAJO/ok.log"
  [ -s "$ORE_PG_TRABAJO/ok.log" ] || { echo "   ✗ el escritor no confirmó nada"; return 1; }
  local conf=$(wc -l < "$ORE_PG_TRABAJO/ok.log") maxn=$(tail -1 "$ORE_PG_TRABAJO/ok.log" | cut -d' ' -f1) minn=$(head -1 "$ORE_PG_TRABAJO/ok.log" | cut -d' ' -f1)
  local en_base=$(sql "select count(*) from latido where n between $minn and $maxn")
  echo "   confirmados $conf · en la base $en_base · perdidos $(( conf - en_base > 0 ? conf - en_base : 0 ))"
  python - "$ORE_PG_TRABAJO/ok.log" <<'PY'
import sys
t=[float(l.split()[1]) for l in open(sys.argv[1])]
g=max((b-a, a) for a,b in zip(t,t[1:]))
print(f"   mayor hueco entre commits: {g[0]:.2f} s (a los {g[1]-t[0]:.0f} s)")
PY
}
corta() {  # corta <pods…> : deja a esos safekeepers sin red de entrada (NetworkPolicy, Dataplane V2)
  # ⚠️ Cilium NO usa statefulset.kubernetes.io/pod-name para las identidades: una política que
  #   selecciona por ella no hace nada, en silencio (medido en P2·6). Se etiqueta el pod y se
  #   selecciona por la etiqueta; y aplicar tarda 5–15 s.
  for p in "$@"; do k label pod "$p" p26=corta --overwrite >/dev/null; done
  cat <<EOF | kubectl apply -f - >/dev/null
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: p26-corta, namespace: $ORE_PG_NS }
spec:
  podSelector: { matchLabels: { p26: corta } }
  policyTypes: [Ingress]
  ingress: []
EOF
  for p in "$@"; do
    until ! k exec cliente -- bash -c "timeout 2 bash -c '</dev/tcp/$p.safekeeper.$ORE_PG_NS.svc.cluster.local/5454'" 2>/dev/null; do sleep 1; done
  done
  echo "$(ts)   cortados de verdad: $*"
}
descorta() { k delete networkpolicy p26-corta >/dev/null 2>&1; k label pod -l ore.dev/rol=safekeeper p26- >/dev/null 2>&1; }

case "${1:-}" in
escrituras)
  echo "== ① escrituras reales con autenticación"
  $E pgbench -h "$IP" -p 55433 -U cloud_admin -i -s 10 -q postgres 2>&1 | tail -1
  for c in 1 4; do echo "   TPC-B $c cliente(s) 30 s: $($E pgbench -h "$IP" -p 55433 -U cloud_admin -c $c -j $c -T 30 postgres 2>&1 | grep -E '^tps|failed' | tr '\n' ' ')"; done
  echo "   tenant en el pageserver: $(pageserver GET /v1/tenant/$(leer tenant) | python -c "import sys,json;d=json.load(sys.stdin);print(d['state']['slug'],'gen',d.get('generation'))")" ;;
safekeepers)
  echo "== ② safekeepers: 1 caído (las escrituras siguen) y 2 caídos (se paran sin perder nada)"
  ev() { echo "$1 $(date +%s.%N)" >> "$ORE_PG_TRABAJO/eventos"; }; : > "$ORE_PG_TRABAJO/eventos"
  escritor 200; W=$!; sleep 10; ev inicio
  echo "$(ts) corta safekeeper-0"; corta safekeeper-0; ev uno; sleep 30
  echo "$(ts) corta también safekeeper-1 (sin quórum)"; corta safekeeper-0 safekeeper-1; ev dos; sleep 30
  echo "$(ts) devuelve la red"; descorta; ev vuelta; wait $W 2>/dev/null; sleep 2
  balance
  python - "$ORE_PG_TRABAJO/ok.log" "$ORE_PG_TRABAJO/eventos" <<'PY'
import sys
t=[float(l.split()[1]) for l in open(sys.argv[1])]
e={l.split()[0]: float(l.split()[1]) for l in open(sys.argv[2])}
tramos=[("3 vivos", e["inicio"], e["uno"]), ("1 caído", e["uno"], e["dos"]), ("2 caídos", e["dos"], e["vuelta"]), ("de vuelta", e["vuelta"], t[-1])]
for txt,a,b in tramos:
    n=sum(1 for x in t if a<=x<b); print(f"   {txt:9s} {b-a:5.1f} s: {n:4d} commits ({n/(b-a) if b>a else 0:.1f}/s)")
PY
  ;;
c4)
  echo "== ③ C4 sin mano: se borra el pageserver CON su disco"
  T=$(leer tenant); sql "create table if not exists marca(q text)" >/dev/null
  sql "insert into marca values ('antes del desastre')" >/dev/null
  escritor 90; W=$!; sleep 5
  T0=$(ms); echo "$(ts) borrar pvc + pod"
  k delete pvc datos-pageserver-0 --wait=false >/dev/null; k delete pod pageserver-0 --grace-period=0 --force >/dev/null 2>&1
  until [ "$(k get pod pageserver-0 -o jsonpath='{.status.containerStatuses[0].ready}' 2>/dev/null)" = true ]; do sleep 1; done
  T1=$(ms); echo "$(ts) pageserver nuevo listo: $(( T1-T0 )) ms"
  until [ "$(pageserver GET /v1/tenant/$T | python -c "import sys,json;print(json.load(sys.stdin)['state']['slug'])" 2>/dev/null)" = Active ]; do sleep 0.5; done
  T2=$(ms); echo "$(ts) tenant Active SIN intervención: $(( T2-T0 )) ms desde el desastre (gen $(pageserver GET /v1/tenant/$T | python -c "import sys,json;print(json.load(sys.stdin).get('generation'))"))"
  # una lectura que no esté en la caché del cómputo: pide páginas al pageserver nuevo
  echo "   lectura fría: $(sql "select count(*) from pgbench_accounts") filas"
  wait $W 2>/dev/null; balance
  echo "   marca: $(sql "select string_agg(q, ' | ') from marca")" ;;
controlador)
  echo "== ④ reiniciar el controller con escrituras en curso"
  escritor 40; W=$!; sleep 5
  echo "$(ts) borrar el pod del controller"; k delete pod -l ore.dev/rol=storage-controller --wait=false >/dev/null
  k rollout status deploy/storage-controller --timeout=120s >/dev/null; echo "$(ts) controller de vuelta"
  wait $W 2>/dev/null; balance
  echo "   tenants que ve: $(controlador GET /debug/v1/tenant | python -c "import sys,json;print([(t['tenant_shard_id'][:8], t['generation']) for t in json.load(sys.stdin)])")" ;;
borrar)
  echo "== ⑤ borrar el tenant: ¿y el WAL de los safekeepers?"
  T=$(leer tenant)
  n() { gcloud storage ls -r "gs://ore-pg-almacen-euw1/$1/tenants/$T/**" 2>/dev/null | grep -c "^gs://" ; }
  sk() { for i in 0 1 2; do k exec safekeeper-$i -- sh -c "du -sk /data/$T 2>/dev/null | cut -f1" ; done | tr '\n' ' '; }
  gcloud storage ls "gs://ore-pg-almacen-euw1/safekeeper/" 2>/dev/null | head -3
  echo "   antes: pageserver/ $(n pageserver) objetos · safekeeper/ $(gcloud storage ls -r "gs://ore-pg-almacen-euw1/safekeeper/$T/**" 2>/dev/null | grep -c '^gs://') objetos · disco de los safekeepers (kB): $(sk)"
  k delete pod "$ORE_PG_VM" --wait=true >/dev/null
  "$AQUI/tenant.sh" borrar
  sleep 30
  echo "   después: pageserver/ $(n pageserver) objetos · safekeeper/ $(gcloud storage ls -r "gs://ore-pg-almacen-euw1/safekeeper/$T/**" 2>/dev/null | grep -c '^gs://') objetos · disco de los safekeepers (kB): $(sk)" ;;
*) echo "uso: $0 escrituras | safekeepers | c4 | controlador | borrar"; exit 1 ;;
esac
