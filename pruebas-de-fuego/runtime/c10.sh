#!/usr/bin/env bash
# ORE 0081 §7.5 · C10: el contrato (ORE SQL) en victor, contra el lago y el `/v1` reales, en UN nodo.
#
# Cada sentencia corre en un pod del pool `pesado-driver` (normal, no spot; nunca el de sistema) con
# `ore-runtime local --v1`, la imagen `ore-runtime:main` fijada por su digest, la cuenta `puesto` y el token
# proyectado de la apertura de un puesto abierto: `ore-serve` lo trata como ese puesto y escribe a nombre de la
# persona que lo abrió (`x-ore-pod`). Los verbos del contrato sobre un dataset de prueba, con lo que tiene que
# dar cada uno, y lo que tarda cada sentencia en su contenedor. Al final, el dataset se retira.
#
#   bash pruebas-de-fuego/runtime/c10.sh <id-del-puesto> <base>.<schema> [filas]
#
# El puesto tiene que estar abierto (su apertura viva) y su persona poder crear datasets en `<base>.<schema>`.
set -uo pipefail
PUESTO=${1:?el id del puesto abierto (p. ej. puesto-ana-python)}
DONDE=${2:?<base>.<schema> donde crear el dataset de prueba}
FILAS=${3:-10000000}
NS=${NS:-t-victor}
BASE=${DONDE%%.*}
T="$DONDE.c10_ids"
REG=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-runtime
IMAGEN=$REG@$(gcloud artifacts docker images describe "$REG:main" --format='value(image_summary.digest)') \
  || { echo "⛔ sin la imagen ore-runtime:main"; exit 1; }
# La audiencia de su token, de su Job: `ore-serve/puestos/<id>/<apertura>`.
AUD=$(kubectl -n "$NS" get jobs -o json \
  | python -c "import json,sys
for j in json.load(sys.stdin)['items']:
    for v in j['spec']['template']['spec'].get('volumes', []):
        for s in v.get('projected', {}).get('sources', []):
            a = s.get('serviceAccountToken', {}).get('audience', '')
            if a.startswith('ore-serve/puestos/$PUESTO/'): print(a)" | head -1)
[ -n "$AUD" ] || { echo "⛔ no hay ningún Job de \`$PUESTO\` abierto en $NS"; exit 1; }
echo "== $IMAGEN"
echo "== $AUD · $T · $FILAS filas"
N=0
FALLOS=0
trap 'kubectl -n "$NS" delete jobs -l ore.dev/c10=si --wait=false >/dev/null 2>&1' EXIT

# corre <qué> <sql> [lo que tiene que salir]: una sentencia, su salida y lo que tardó su contenedor.
corre() {
  N=$((N + 1))
  local J="c10-$N-$RANDOM" QUE=$1 SQL=$2 ESPERA=${3:-}
  python - "$J" "$IMAGEN" "$AUD" "$BASE" "$SQL" "$NS" <<'PY' | kubectl create -f - >/dev/null
import json, sys
j, imagen, aud, base, sql, ns = sys.argv[1:]
print(json.dumps({
  "apiVersion": "batch/v1", "kind": "Job",
  "metadata": {"name": j, "namespace": ns, "labels": {"ore.dev/c10": "si"}},
  "spec": {"backoffLimit": 0, "activeDeadlineSeconds": 1800, "template": {
    "metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": ns[2:], "ore.dev/c10": "si"}},
    "spec": {
      "restartPolicy": "Never", "serviceAccountName": "puesto",
      "securityContext": {"runAsNonRoot": True, "runAsUser": 65532, "runAsGroup": 65532,
                          "seccompProfile": {"type": "RuntimeDefault"}},
      "nodeSelector": {"ore.dev/pool": "pesado-driver"},
      "tolerations": [{"key": "ore.dev/pesado-driver", "operator": "Equal", "value": "true", "effect": "NoSchedule"}],
      "volumes": [{"name": "derrame", "emptyDir": {"sizeLimit": "10Gi"}},
                  {"name": "token-del-pod", "projected": {"sources": [{"serviceAccountToken": {
                      "audience": aud, "expirationSeconds": 3600, "path": "token"}}]}}],
      "containers": [{
        "name": "runtime", "image": imagen,
        "args": ["--registro", "warn", "--opcion", "ore.derrame_dir=/derrame",
                 "--v1", f"http://ore-serve.{ns}.svc.cluster.local:8080",
                 "--token-del-pod", "/var/run/ore/pod/token",
                 "local", "--base", base, sql],
        "volumeMounts": [{"name": "derrame", "mountPath": "/derrame"},
                         {"name": "token-del-pod", "mountPath": "/var/run/ore/pod", "readOnly": True}],
        "resources": {"requests": {"cpu": "3", "memory": "8Gi"}, "limits": {"cpu": "3", "memory": "8Gi"}},
        "securityContext": {"allowPrivilegeEscalation": False, "readOnlyRootFilesystem": True,
                            "capabilities": {"drop": ["ALL"]}}}]}}}}))
PY
  kubectl -n "$NS" wait --for=condition=complete --timeout=1800s "job/$J" >/dev/null 2>&1
  local SALIDA TIEMPO
  SALIDA=$(kubectl -n "$NS" logs "job/$J" 2>&1)
  TIEMPO=$(kubectl -n "$NS" get pod -l job-name="$J" -o json | python -c "import json,sys,datetime as d
s=json.load(sys.stdin)['items'][0]['status']['containerStatuses'][0]['state'].get('terminated',{})
f=lambda x: d.datetime.fromisoformat(x.replace('Z','+00:00'))
print(int((f(s['finishedAt'])-f(s['startedAt'])).total_seconds()) if s else '?')")
  if [ -n "$ESPERA" ] && ! grep -q -- "$ESPERA" <<<"$SALIDA"; then
    FALLOS=$((FALLOS + 1))
    echo "✗ $QUE · ${TIEMPO}s · se esperaba «$ESPERA»"
  else
    echo "✓ $QUE · ${TIEMPO}s"
  fi
  sed 's/^/    /' <<<"$SALIDA" | head -20
  kubectl -n "$NS" delete job "$J" --wait=false >/dev/null 2>&1
}

corre "crear ($FILAS filas)" \
  "create or replace dataset $T as select v as id, v % 100 as k, 'x' || cast(v as varchar) as t from generate_series(1, $FILAS) g(v)" \
  "$FILAS"
ANTES=$(date -u +%Y-%m-%dT%H:%M:%SZ)
sleep 2
corre "leer: contar y sumar" "select count(*) as n, sum(id) as s from $T" "$FILAS"
corre "leer: agrupar" "select k, count(*) as n from $T group by k order by k limit 3" "$((FILAS / 100))"
corre "insert" "insert into $T values (0, 0, 'cero')" "1"
corre "update" "update $T set t = 'uno' where id = 1" "1"
corre "delete" "delete from $T where k = 7" "$((FILAS / 100))"
corre "merge" \
  "merge into $T d using (select * from (values (2, 'dos'), (-1, 'nuevo')) f(id, t)) f on d.id = f.id when matched then update set t = f.t when not matched then insert values (f.id, 0, f.t)" \
  "2"
corre "lo de hoy" "select count(*) from $T" "$((FILAS - FILAS / 100 + 2))"
corre "at(): antes de escribir" "select count(*) from at('$T', timestamp => '$ANTES')" "$FILAS"
corre "information_schema" \
  "select table_name from information_schema.tables where table_name = 'c10_ids'" "c10_ids"
corre "retirar" "drop dataset $T"
echo
[ "$FALLOS" -eq 0 ] && echo "C10: $N/$N" || { echo "C10: $FALLOS de $N fallan"; exit 1; }
