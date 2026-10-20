#!/usr/bin/env bash
# ORE 0068 C3·e · Q6 de SpatialBench a escala 100 en DuckDB 1.5.6 (la imagen del puesto), con las 19 filas SIN
# redondear, para cotejarlas con las de Spark (`medir.sh CASOS=q6 MUESTRA=20 CRUDO=1`). En C3·d, Spark dio las
# 19 filas de DuckDB pero otra huella, y C0 solo guardó dos filas de DuckDB.
# Un Job en el namespace de medida (`medir.sh` lo prepara: la cuenta `sb` lee el bucket), en un nodo `pesado`:
# baja `trip` y `zone` (45 GB) a su SSD local y corre Q6 allí. Imprime `Q6D|fila|…`.
#   bash pruebas-de-fuego/geo/c3/d/cotejar-q6.sh
set -uo pipefail
export PYTHONUTF8=1
NS=c3d-medida
aqui="$(cd "$(dirname "$0")" && pwd)"
P=project-8853a180-450d-47be-b83
IMAGEN=europe-west1-docker.pkg.dev/$P/ore/puesto-python:1
kubectl get sa sb -n $NS >/dev/null 2>&1 || { echo "⛔ sin $NS/sb: antes, medir.sh (prepara el namespace y el permiso)"; exit 1; }
J=q6d-$(date +%H%M%S)
python3 - "$aqui/consultas.json" > "/tmp/$J.py" <<'PY'
import json, sys
q6 = json.load(open(sys.argv[1]))["q6"]
print('''import datetime, decimal, json, os, time, urllib.request
B = "%s"
def tok():
    r = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                               headers={"Metadata-Flavor": "Google"})
    return json.load(urllib.request.urlopen(r))["access_token"]
for t in ("trip", "zone"):
    t0 = time.time()
    n = urllib.parse.quote("spatialbench/sf100/%%s.parquet" %% t, safe="")
    r = urllib.request.Request("https://storage.googleapis.com/storage/v1/b/%%s/o/%%s?alt=media" %% (B, n),
                               headers={"Authorization": "Bearer " + tok()})
    with urllib.request.urlopen(r) as src, open("/d/%%s.parquet" %% t, "wb") as dst:
        while True:
            b = src.read(16 << 20)
            if not b:
                break
            dst.write(b)
    print("Q6D|bajado|%%s|%%.0f s|%%d bytes" %% (t, time.time() - t0, os.path.getsize("/d/%%s.parquet" %% t)), flush=True)
import duckdb
c = duckdb.connect()
c.execute("set extension_directory = '/opt/ore/duckdb'")
c.execute("load spatial")
c.execute("set temp_directory = '/d/tmp'")
for t in ("trip", "zone"):
    c.execute("create view %%s as select * from read_parquet('/d/%%s.parquet')" %% (t, t))
t0 = time.time()
filas = c.execute(%r).fetchall()
print("Q6D|q6|%%.1f s|%%d filas" %% (time.time() - t0, len(filas)), flush=True)
for f in sorted(repr(tuple(f)) for f in filas):
    print("Q6D|fila|" + f, flush=True)
''' % ("project-8853a180-450d-47be-b83-banco-geo", q6))
PY
kubectl -n $NS create configmap $J --from-file=q6.py="/tmp/$J.py" >/dev/null
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata: {name: $J, namespace: $NS}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 3600
  ttlSecondsAfterFinished: 3600
  template:
    spec:
      restartPolicy: Never
      serviceAccountName: sb
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      nodeSelector: {ore.dev/pool: pesado}
      tolerations: [{key: ore.dev/pesado, operator: Equal, value: "true", effect: NoSchedule}]
      volumes: [{name: m, configMap: {name: $J}}, {name: d, emptyDir: {}}]
      containers:
        - name: duckdb
          image: $IMAGEN
          command: ["python3", "/m/q6.py"]
          workingDir: /d
          env: [{name: HOME, value: /tmp}]
          volumeMounts: [{name: m, mountPath: /m}, {name: d, mountPath: /d}]
          resources:
            requests: {cpu: "7", memory: 48Gi, ephemeral-storage: 250Gi}
            limits: {memory: 48Gi, ephemeral-storage: 250Gi}
          securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}
YAML
echo "== $J creado $(date -u +%T)"
until s=$(kubectl -n $NS get job $J -o jsonpath='{.status.succeeded}{.status.failed}' 2>/dev/null); [ -n "$s" ]; do sleep 15; done
kubectl -n $NS logs job/$J 2>/dev/null | grep -E "^Q6D|Error|Exception" | cut -c1-300
kubectl -n $NS delete job $J --wait=false >/dev/null 2>&1; kubectl -n $NS delete configmap $J >/dev/null 2>&1
rm -f "/tmp/$J.py"
