#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# CARGAR EL CATÁLOGO · la mudanza de los punteros de un inquilino (ADR 0079 §17.8, E1a·4c·6)
#
# Un Job de una vez en la celda, t-<org>, con `ore-catalogo` (imagen `ore-serve:main`) en modo
# `seguir --una-vez`: trae el árbol de la forja, replica en el esquema t_<org> cada commit de `main`
# con sus punteros (la historia entera, verificada paso a paso) y las ramas, y audita el resultado
# contra git con la regla `manda`. Sale 0 si todo cuadra.
#
# Es incremental: la segunda vez sólo trae lo que falte. Así se usa dos veces:
#   el ensayo   ahora, con el inquilino escribiendo en git como siempre: se carga y se audita, y los
#               datos se quedan (nadie los lee todavía);
#   el corte    (E1a·5) con las escrituras paradas unos segundos: trae lo que falte desde el ensayo,
#               audita, y se conmuta.
#
# Corre como el plano de control de la celda (`rol: control`, cuenta `ore-serve`): llega a la forja y
# al catálogo (76: `salida-al-catalogo`), trae el testigo de la forja como ore-serve, y entra con el
# papel del inquilino (78: el Secret `catalogo`), nunca con el superusuario.
#
#   bash malla/79-cargar-el-catalogo.sh victor
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
ORG=${1:?uso: $0 <org>}
case "$ORG" in *[!a-z0-9]*|'') echo "⛔ la organización va en minúsculas y cifras: \`$ORG\`"; exit 64 ;; esac
NS="t-$ORG"
JOB=cargar-el-catalogo-$(date -u +%Y%m%d%H%M%S)
R=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore

kubectl -n "$NS" get secret catalogo >/dev/null 2>&1 || { echo "⛔ $NS no tiene el Secret catalogo: antes, 78-el-catalogo-del-inquilino.sh $ORG"; exit 1; }

kubectl -n "$NS" apply -f - >/dev/null <<EOF
apiVersion: batch/v1
kind: Job
metadata:
  name: $JOB
  namespace: $NS
  labels: { ore.dev/rol: control, ore.dev/tenant: $ORG, ore.dev/pieza: mudanza-del-catalogo }
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 86400
  template:
    metadata:
      labels: { ore.dev/rol: control, ore.dev/tenant: $ORG, ore.dev/pieza: mudanza-del-catalogo }
    spec:
      restartPolicy: Never
      serviceAccountName: ore-serve
      nodeSelector: { ore.dev/pool: jobs }
      tolerations:
        - { key: ore.dev/jobs, operator: Equal, value: "true", effect: NoSchedule }
      volumes:
        - { name: testigo, emptyDir: { medium: Memory, sizeLimit: 1Mi } }
        - { name: espejo, emptyDir: {} }
      initContainers:
        - name: traer-el-testigo
          image: $R/ore-drivers:main
          volumeMounts: [{ name: testigo, mountPath: /testigo }]
          env:
            - { name: HOME, value: /tmp }
            - { name: CLOUDSDK_CONFIG, value: /tmp/.gcloud }
          command: ["/bin/sh", "-c"]
          args:
            - |
              set -e
              gcloud secrets versions access latest --secret=$NS-forja-token --out-file=/testigo/forja
              chmod 0444 /testigo/forja
              echo "testigo puesto · \$(wc -c < /testigo/forja) bytes"
      containers:
        - name: cargar
          image: $R/ore-serve:main
          imagePullPolicy: Always
          volumeMounts:
            - { name: testigo, mountPath: /testigo }
            - { name: espejo, mountPath: /espejo }
          env:
            - name: CATALOGO_URL
              valueFrom: { secretKeyRef: { name: catalogo, key: url } }
          command:
            - /usr/local/bin/ore-catalogo
            - seguir
            - --esquema
            - t_$ORG
            - --forja
            - http://forja.$NS.svc.cluster.local:3000/$NS/ontologia.git
            - --espejo
            - /espejo/arbol.git
            - --testigo-fichero
            - /testigo/forja
            - --una-vez
            - --verificar-pasos
          # ⛔ El Service ore-serve elige por \`rol: control\`: sin esto, este pod entraría en sus
          #   endpoints y recibiría peticiones de la consola que no sabe contestar. Nunca listo =
          #   nunca en el Service; un Job termina igual.
          readinessProbe:
            exec: { command: ["false"] }
            periodSeconds: 3600
          resources:
            requests: { cpu: 250m, memory: 256Mi }
            limits: { memory: 1Gi }
EOF
echo "── $JOB en $NS"
fin=$(( $(date +%s) + 900 ))
until [ -n "$(kubectl -n "$NS" get job/"$JOB" -o jsonpath='{.status.succeeded}{.status.failed}')" ]; do
  [ "$(date +%s)" -lt "$fin" ] || { echo "⛔ $JOB no terminó en 900 s"; break; }
  sleep 5
done
kubectl -n "$NS" logs job/"$JOB" --all-containers --prefix | sed 's/^/   /'
[ "$(kubectl -n "$NS" get job/"$JOB" -o jsonpath='{.status.succeeded}')" = 1 ]
