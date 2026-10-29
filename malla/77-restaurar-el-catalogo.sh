#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# RESTAURAR EL CATÁLOGO · la prueba de que la copia es una copia (ADR 0079 §18)
#
# «Una copia que nadie ha restaurado nunca no es una copia: es un fichero» (62). Esto baja el
# último volcado de `gs://…-copias/catalogo/` (o el que se diga), lo restaura en una base NUEVA del
# mismo servidor (`restaurada_<fecha>`, nunca encima de `catalogo`) y compara, esquema a esquema
# de inquilino, el estado de los punteros, la historia, los commits y las ramas con la base viva.
#
#   bash malla/77-restaurar-el-catalogo.sh                    # el último volcado; borra la restaurada
#   VOLCADO=catalogo-20261028T120000Z.dump bash malla/77-…    # uno concreto
#   GUARDAR=1 bash malla/77-…                                 # deja la base restaurada para mirarla
#
# ⚠️ Compara con la base VIVA: si alguien escribió después del volcado, la diferencia es legítima
#   y se ve como tal (más commits en la viva). Para una prueba limpia, volcar y restaurar seguidos.
# Sale 0 si cada esquema cuadra.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
NS=catalogo
VOLCADO=${VOLCADO:-}
GUARDAR=${GUARDAR:-}
JOB=restaurar-$(date -u +%Y%m%d%H%M%S)
DESTINO=restaurada_$(date -u +%Y%m%d%H%M%S)

kubectl -n $NS apply -f - >/dev/null <<EOF
apiVersion: batch/v1
kind: Job
metadata:
  name: $JOB
  namespace: $NS
  labels: { ore.dev/rol: mantenimiento, ore.dev/tenant: system }
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 3600
  template:
    metadata:
      labels: { ore.dev/rol: mantenimiento, ore.dev/tenant: system }
    spec:
      restartPolicy: Never
      serviceAccountName: copias
      nodeSelector: { ore.dev/pool: catalogo }
      tolerations:
        - { key: ore.dev/catalogo, operator: Equal, value: "true", effect: NoSchedule }
      volumes: [{ name: volcado, emptyDir: {} }]
      initContainers:
        - name: bajar
          image: europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-drivers:main
          volumeMounts: [{ name: volcado, mountPath: /volcado }]
          env:
            - { name: HOME, value: /tmp }
            - { name: CLOUDSDK_CONFIG, value: /tmp/.gcloud }
            - { name: BUCKET, value: "gs://project-8853a180-450d-47be-b83-copias" }
            - { name: VOLCADO, value: "$VOLCADO" }
          command: ["/bin/sh", "-c"]
          args:
            - |
              set -e
              [ -n "\$VOLCADO" ] || VOLCADO=\$(gcloud storage ls "\$BUCKET/catalogo/" | sort | tail -1 | sed 's#.*/##')
              gcloud storage cp "\$BUCKET/catalogo/\$VOLCADO" /volcado/catalogo.dump
              echo "bajado \$VOLCADO · \$(wc -c < /volcado/catalogo.dump) bytes"
      containers:
        - name: restaurar
          image: europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/postgres:16
          volumeMounts: [{ name: volcado, mountPath: /volcado }]
          env:
            - { name: PGHOST, value: catalogo-db.catalogo.svc.cluster.local }
            # El superusuario del `Cluster catalogo` (0079 ·5f): crea la base de la prueba y lee
            # los esquemas de los inquilinos.
            - name: PGUSER
              valueFrom: { secretKeyRef: { name: catalogo-superuser, key: username } }
            - name: PGPASSWORD
              valueFrom: { secretKeyRef: { name: catalogo-superuser, key: password } }
            - { name: DESTINO, value: "$DESTINO" }
            - { name: GUARDAR, value: "$GUARDAR" }
          command: ["/bin/sh", "-c"]
          args:
            - |
              set -e
              createdb "\$DESTINO"
              # Las publicaciones no se restauran en otra base del mismo servidor sin chocar de
              # nombre: fuera de la prueba (en una restauración de verdad, se recrean).
              pg_restore -d "\$DESTINO" --no-owner --no-publications /volcado/catalogo.dump
              firma() {
                psql -X -q -t -A -d "\$1" -c "SELECT md5(string_agg(ref||'|'||activo||'|'||coalesce(puntero::text,'-')||'|'||borrado, E'\n' ORDER BY ref, activo)) || ' · ' || (SELECT count(*) FROM \$2.historia) || ' historia · ' || (SELECT count(*) FROM \$2.commits) || ' commits · ' || (SELECT string_agg(nombre||'@'||coalesce(punto_salida::text,'-'), ',' ORDER BY nombre) FROM \$2.refs) FROM \$2.estado" | md5sum | cut -c1-12
              }
              mal=0; n=0
              for s in \$(psql -X -q -t -A -d catalogo -c "SELECT nspname FROM pg_namespace WHERE nspname LIKE 't\_%' ORDER BY 1"); do
                n=\$((n+1)); a=\$(firma catalogo "\$s"); b=\$(firma "\$DESTINO" "\$s")
                if [ "\$a" = "\$b" ]; then echo "PASA  \$s  (\$a)"; else echo "FALLA \$s  viva \$a · restaurada \$b"; mal=\$((mal+1)); fi
              done
              echo "esquemas: \$n · distintos: \$mal"
              [ -n "\$GUARDAR" ] || dropdb "\$DESTINO"
              [ "\$n" -gt 0 ] && [ "\$mal" = 0 ]
EOF
echo "── $JOB (restaura en $DESTINO)"
fin=$(( $(date +%s) + 600 ))
until [ -n "$(kubectl -n $NS get job/$JOB -o jsonpath='{.status.succeeded}{.status.failed}')" ]; do
  [ "$(date +%s)" -lt "$fin" ] || { echo "⛔ $JOB no terminó en 600 s"; break; }
  sleep 5
done
kubectl -n $NS logs job/$JOB --all-containers --prefix | sed 's/^/   /'
[ "$(kubectl -n $NS get job/$JOB -o jsonpath='{.status.succeeded}')" = 1 ]
