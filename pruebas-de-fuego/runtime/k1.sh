#!/usr/bin/env bash
# ORE 0081 §7.6 · K1: lo que cuesta planificar, en victor, contra el lago y el `/v1` reales.
#
# Siembra en `<base>.<schema>` tablas de 1, 100 y 1.000 ficheros (un `insert` por fichero: cada uno es un snapshot y
# un manifiesto, como un lago que se anexa sin compactar), y mide con `ore-runtime local --medir` las fases de cada
# sentencia: planificar (el `loadTable` y el plan lógico), el plan físico (donde el escaneo planifica sus ficheros) y
# correr, con sus peticiones a `/v1`, sus lecturas del lago y sus bytes. Al final, las tablas se retiran.
#
# Como C10: cada paso es un Job del pool `pesado-driver` con la cuenta `puesto` y el token de la apertura de un
# puesto abierto, y la imagen fijada por su digest.
#
#   bash pruebas-de-fuego/runtime/k1.sh <id-del-puesto> <base>.<schema> [ficheros...]
#
# Con `SOLO_MEDIR=1`, mide lo ya sembrado sin volver a sembrar.
set -uo pipefail
PUESTO=${1:?el id del puesto abierto}
DONDE=${2:?<base>.<schema> donde sembrar}
shift 2
TAMANOS=${*:-1 100 1000}
NS=${NS:-t-victor}
BASE=${DONDE%%.*}
REG=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-runtime
IMAGEN=$REG@$(gcloud artifacts docker images describe "$REG:${ETIQUETA:-main}" --format='value(image_summary.digest)') \
  || { echo "⛔ sin la imagen ore-runtime:${ETIQUETA:-main}"; exit 1; }
AUD=$(kubectl -n "$NS" get jobs -o json \
  | python -c "import json,sys
for j in json.load(sys.stdin)['items']:
    for v in j['spec']['template']['spec'].get('volumes', []):
        for s in v.get('projected', {}).get('sources', []):
            a = s.get('serviceAccountToken', {}).get('audience', '')
            if a.startswith('ore-serve/puestos/$PUESTO/'): print(a)" | head -1)
[ -n "$AUD" ] || { echo "⛔ no hay ningún Job de \`$PUESTO\` abierto en $NS"; exit 1; }
echo "== $IMAGEN"
echo "== $AUD · $DONDE · $TAMANOS ficheros"
trap 'kubectl -n "$NS" delete jobs -l ore.dev/k1=si --wait=false >/dev/null 2>&1' EXIT

# corre <qué> <sql>...: las sentencias en un Job, en orden, y sus líneas `medida`.
corre() {
  local QUE=$1 J="k1-$RANDOM"
  shift
  python - "$J" "$IMAGEN" "$AUD" "$BASE" "$NS" "$@" <<'PY' | kubectl create -f - >/dev/null
import json, sys
j, imagen, aud, base, ns, *sql = sys.argv[1:]
print(json.dumps({
  "apiVersion": "batch/v1", "kind": "Job",
  "metadata": {"name": j, "namespace": ns, "labels": {"ore.dev/k1": "si"}},
  "spec": {"backoffLimit": 0, "activeDeadlineSeconds": 7200, "template": {
    "metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": ns[2:], "ore.dev/k1": "si"}},
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
                 "local", "--medir", "--base", base, *sql],
        "volumeMounts": [{"name": "derrame", "mountPath": "/derrame"},
                         {"name": "token-del-pod", "mountPath": "/var/run/ore/pod", "readOnly": True}],
        "resources": {"requests": {"cpu": "3", "memory": "8Gi"}, "limits": {"cpu": "3", "memory": "8Gi"}},
        "securityContext": {"allowPrivilegeEscalation": False, "readOnlyRootFilesystem": True,
                            "capabilities": {"drop": ["ALL"]}}}]}}}}))
PY
  for _ in $(seq 1 1440); do
    [ -n "$(kubectl -n "$NS" get job "$J" -o jsonpath='{.status.succeeded}{.status.failed}' 2>/dev/null)" ] && break
    sleep 5
  done
  local SALIDA
  SALIDA=$(kubectl -n "$NS" logs "job/$J" 2>&1)
  echo "── $QUE"
  if grep -q "^ore-runtime:" <<<"$SALIDA"; then
    grep -A5 "^ore-runtime:" <<<"$SALIDA" | sed 's/^/    /'
  fi
  grep "^medida" <<<"$SALIDA" | sed 's/^/    /'
  kubectl -n "$NS" delete job "$J" --wait=false >/dev/null 2>&1
}

for N in $TAMANOS; do
  T="$DONDE.k1_f$N"
  # La siembra: un fichero de 1.000 filas, y uno más por cada `insert`.
  SIEMBRA=("create or replace dataset $T as select v as id, v % 10 as k from generate_series(1, 1000) g(v)")
  for i in $(seq 2 "$N"); do
    SIEMBRA+=("insert into $T select v + $((i * 1000)) as id, v % 10 as k from generate_series(1, 1000) g(v)")
  done
  [ -n "${SOLO_MEDIR:-}" ] || corre "sembrar $T ($N ficheros)" "${SIEMBRA[@]}" | tail -3
  # Lo medido: tres veces lo mismo (cada sentencia, su sesión: sin caché entre consultas, se repite todo), un filtro
  # que podaría si llegara al escaneo, y un join de la tabla consigo misma por dos nombres.
  corre "medir $T ($N ficheros)" \
    "select count(*) from $T" \
    "select count(*) from $T" \
    "select count(*) from $T" \
    "select count(*) from $T where id = 5" \
    "select count(*) from $T a join $T b on a.id = b.id where a.k = 1"
  corre "retirar $T" "drop dataset $T" >/dev/null
done
