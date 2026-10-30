#!/usr/bin/env bash
# ORE 0081 §7.4 · S6: el ORE Runtime medido en victor. `ore-runtime diagnostico` en un pod del pool
# `pesado-driver` (normal, no spot; nunca el de sistema), con la imagen `ore-runtime:main` fijada por su
# digest. Dos cuotas de CPU: una entera y una fraccionaria, que el runtime tiene que redondear hacia abajo.
# Deja las líneas `clave valor` de cada una y borra el namespace.
#
#   bash pruebas-de-fuego/runtime/s6.sh
set -uo pipefail
NS=${NS:-s6-runtime}
REG=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-runtime
IMAGEN=$REG@$(gcloud artifacts docker images describe "$REG:main" --format='value(image_summary.digest)') \
  || { echo "⛔ sin la imagen ore-runtime:main"; exit 1; }
echo "== $IMAGEN"
kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
trap 'kubectl delete namespace "$NS" --wait=false >/dev/null 2>&1' EXIT

medir() {  # <nombre> <cpu> <memoria>
  local J=$1 CPU=$2 MEM=$3
  cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata: {name: $J, namespace: $NS}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 900
  template:
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532, seccompProfile: {type: RuntimeDefault}}
      nodeSelector: {ore.dev/pool: pesado-driver}
      tolerations: [{key: ore.dev/pesado-driver, operator: Equal, value: "true", effect: NoSchedule}]
      volumes: [{name: derrame, emptyDir: {sizeLimit: 10Gi}}]
      containers:
        - name: runtime
          image: $IMAGEN
          args: ["--registro", "warn", "--opcion", "ore.derrame_dir=/derrame", "diagnostico"]
          volumeMounts: [{name: derrame, mountPath: /derrame}]
          resources:
            requests: {cpu: "$CPU", memory: $MEM}
            limits: {cpu: "$CPU", memory: $MEM}
          securityContext: {allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: {drop: [ALL]}}
YAML
  kubectl -n "$NS" wait --for=condition=complete --timeout=900s "job/$J" >/dev/null \
    || { echo "⛔ $J no terminó"; kubectl -n "$NS" describe "job/$J" | tail -15; kubectl -n "$NS" logs "job/$J" | tail -20; return 1; }
  echo "== $J · cpu $CPU · memoria $MEM · nodo $(kubectl -n "$NS" get pod -l job-name="$J" -o jsonpath='{.items[0].spec.nodeName}')"
  kubectl -n "$NS" logs "job/$J" | sed 's/^/   /'
}

medir entera 3 8Gi
medir fraccionaria 1500m 4Gi
