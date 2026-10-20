#!/usr/bin/env bash
# ORE 0068 C3·d · SpatialBench a escala 100 (600 M viajes, ~56 GB de Parquet) en el bucket del banco,
# `spatialbench/sf100/`, generado en el pool `pesado` de victor (C0 la generó en un pool que ya no existe y
# los datos se borraron con él). Un Job en el namespace de medida `c3d-medida`, sin Kueue; su cuenta de
# Kubernetes (`sb`) recibe un permiso TEMPORAL de escritura sobre el bucket por Workload Identity, que
# `BORRAR=1` retira junto con el namespace.
#   bash pruebas-de-fuego/geo/c3/d/generar.sh            # genera y sube
#   BORRAR=1 bash pruebas-de-fuego/geo/c3/d/generar.sh   # retira el permiso y el namespace (no los datos)
set -uo pipefail
NS=c3d-medida
P=project-8853a180-450d-47be-b83
B=$P-banco-geo
NUM=$(gcloud projects describe $P --format='value(projectNumber)')
QUIEN="principal://iam.googleapis.com/projects/$NUM/locations/global/workloadIdentityPools/$P.svc.id.goog/subject/ns/$NS/sa/sb"
if [ "${BORRAR:-0}" = 1 ]; then
  gcloud storage buckets remove-iam-policy-binding gs://$B --member="$QUIEN" --role=roles/storage.objectAdmin >/dev/null && echo "== permiso retirado"
  kubectl delete ns $NS --ignore-not-found --wait=false; exit 0
fi
kubectl get ns $NS >/dev/null 2>&1 || kubectl create ns $NS >/dev/null
kubectl -n $NS create serviceaccount sb --dry-run=client -o yaml | kubectl apply -f - >/dev/null
gcloud storage buckets add-iam-policy-binding gs://$B --member="$QUIEN" --role=roles/storage.objectAdmin >/dev/null \
  && echo "== permiso temporal: $NS/sb escribe en gs://$B"
J=sf100-$(date +%H%M%S)
cat <<YAML | kubectl create -f - >/dev/null
apiVersion: batch/v1
kind: Job
metadata: {name: $J, namespace: $NS}
spec:
  backoffLimit: 0
  activeDeadlineSeconds: 7200
  ttlSecondsAfterFinished: 3600
  template:
    spec:
      restartPolicy: Never
      serviceAccountName: sb
      nodeSelector: {ore.dev/pool: pesado}
      tolerations: [{key: ore.dev/pesado, operator: Equal, value: "true", effect: NoSchedule}]
      volumes: [{name: t, emptyDir: {}}]
      containers:
        - name: sb
          image: rust:1
          command: ["bash", "-c"]
          args:
            - |
              set -euo pipefail
              T=/t; cd \$T
              echo "== \$(date -u +%T) el generador"
              cargo install -q spatialbench-cli@0.2.0 --root \$T/cargo
              echo "== \$(date -u +%T) la escala 100"
              time \$T/cargo/bin/spatialbench-cli -s 100 --format=parquet --output-dir \$T/sf100
              du -sh \$T/sf100; find \$T/sf100 -type f | head -50
              echo "== \$(date -u +%T) al bucket"
              tok() { curl -s -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p'; }
              cd \$T/sf100
              find . -type f | sed 's|^\./||' | while read -r f; do
                n=\$(printf 'spatialbench/sf100/%s' "\$f" | sed 's|/|%2F|g')
                for i in 1 2 3; do
                  c=\$(curl -s -o /dev/null -w '%{http_code}' -X POST -H "Authorization: Bearer \$(tok)" \
                    -H "Content-Type: application/octet-stream" -T "\$f" \
                    "https://storage.googleapis.com/upload/storage/v1/b/$B/o?uploadType=media&name=\$n")
                  [ "\$c" = 200 ] && break; echo "   reintento \$f (\$c)"; sleep 5
                done
                echo "SUBIDO|\$f|\$c|\$(stat -c %s "\$f")"
              done
              echo "== \$(date -u +%T) hecho"
          volumeMounts: [{name: t, mountPath: /t}]
          resources:
            requests: {cpu: "7", memory: 48Gi}
            limits: {memory: 56Gi}
YAML
echo "== $J creado $(date -u +%T): kubectl -n $NS logs -f job/$J"
