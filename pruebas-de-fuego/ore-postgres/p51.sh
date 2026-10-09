#!/usr/bin/env bash
# P5·1 · El contrato del proxy (ADR 0058). Hecho cuando: contra el proxy de Neon de verdad, en el clúster
# y sin entrada pública todavía, `psql` por el proxy entra con la contraseña del rol y no con otra; un
# endpoint que no existe, un rol que no existe y un endpoint de otra organización, fuera.
#
# El proxy es un pod de prueba en `ore-pg` (la imagen del almacenamiento, que ya lo lleva), con pata en
# la overlay del lado del proxy y un certificado autofirmado de `*.europe-west1.pg.paladio.io`. El
# cliente es `cliente-overlay`: `host=ep-….europe-west1.pg.paladio.io` (el SNI) y `hostaddr=` la IP del
# pod del proxy. La contraseña sale de la API y no se imprime.
#
#   p51.sh [celda] [otra]      (demo y victor por defecto)
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}; B=${2:-victor}
P="p51-$(date +%s | tail -c 6)"
PRUEBA=p51
DOMINIO=europe-west1.pg.paladio.io
IMAGEN=$(k get sts pageserver -o jsonpath='{.spec.template.spec.containers[0].image}')
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A" "$B"
limpiar_proxy() { k delete pod proxy-prueba --ignore-not-found --grace-period=0 >/dev/null 2>&1; k delete secret proxy-prueba-tls --ignore-not-found >/dev/null; }
trap 'limpiar_proxy; for c in $CELDAS_ABIERTAS; do kubectl -n t-$c delete networkpolicy prueba-$PRUEBA --ignore-not-found >/dev/null; done' EXIT
# entra ROL CLAVE BASE VM → lo que contesta psql (la última línea)
entra() { k exec cliente-overlay -- env PGPASSWORD="$2" PGCONNECT_TIMEOUT=10 \
  psql "host=$4.$DOMINIO hostaddr=$PIP port=4432 user=$1 dbname=$3 sslmode=require" -Atc 'select current_user' 2>&1 | tail -1; }

echo "── el proxy de prueba: certificado autofirmado y pod con pata en la overlay"
T=$(mktemp -d)
MSYS_NO_PATHCONV=1 openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
  -subj "/CN=*.$DOMINIO" -addext "subjectAltName=DNS:*.$DOMINIO" -keyout "$T/tls.key" -out "$T/tls.crt" 2>/dev/null
k create secret tls proxy-prueba-tls --cert="$T/tls.crt" --key="$T/tls.key" >/dev/null; rm -rf "$T"
cat <<YAML | k apply -f - >/dev/null
apiVersion: v1
kind: Pod
metadata:
  name: proxy-prueba
  namespace: ore-pg
  labels: { ore.dev/rol: proxy-postgres }
  annotations: { k8s.v1.cni.cncf.io/networks: ore-pg/overlay-del-proxy }
spec:
  nodeSelector: { ore.dev/pool: neon }
  tolerations: [{ key: ore.dev/neon, operator: Equal, value: "true", effect: NoSchedule }]
  containers:
    - name: proxy
      image: $IMAGEN
      command: [sh, -c]
      args:
        - exec /usr/local/bin/proxy --region europe-west1 --proxy 0.0.0.0:4432 --mgmt 127.0.0.1:7000 --http 0.0.0.0:7001
          --auth-backend control-plane --auth-endpoint http://ore-postgres.ore-pg.svc.cluster.local.:8100/proxy/
          --control-plane-token="\$(cat /token/token)" --tls-cert /tls/tls.crt --tls-key /tls/tls.key
      volumeMounts:
        - { name: tls, mountPath: /tls, readOnly: true }
        - { name: token, mountPath: /token, readOnly: true }
  volumes:
    - { name: tls, secret: { secretName: proxy-prueba-tls } }
    - { name: token, secret: { secretName: ore-postgres-proxy } }
YAML
k wait --for=condition=Ready pod/proxy-prueba --timeout=180s >/dev/null || { k logs proxy-prueba | tail -20; exit 1; }
PIP=$(k get pod proxy-prueba -o jsonpath='{.status.podIP}')
echo "  ✓ el proxy escucha en $PIP:4432"

echo "── $A crea $P con dueño user:p51; $B crea el suyo"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\",\"dueno\":\"user:p51\"}'")
OP=$(campo "${L[0]}" operacion id); ROL=$(campo "${L[0]}" rol nombre); CLAVE=$(campo "${L[0]}" rol contrasena)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET /v1/postgres/proyectos/$P/ramas/main/endpoints/principal")
VM=$(campo "${L[1]}" vm)
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ $P listo · endpoint $VM" || { echo "  ✗ ${L[0]:0:300}"; exit 1; }
mapfile -t L < <(en "$B" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\",\"dueno\":\"user:p51\"}'")
OPB=$(campo "${L[0]}" operacion id); CLAVE_B=$(campo "${L[0]}" rol contrasena)
mapfile -t L < <(en "$B" "hasta_hecha /v1/postgres/operaciones/$OPB" "pide GET /v1/postgres/proyectos/$P/ramas/main/endpoints/principal")
VM_B=$(campo "${L[1]}" vm)
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ el de $B listo · endpoint $VM_B" || { echo "  ✗ ${L[0]:0:300}"; exit 1; }

echo "── por el proxy"
[ "$(entra "$ROL" "$CLAVE" "$P" "$VM")" = "$ROL" ] && echo "  ✓ $ROL entra en su $P por ep-….$DOMINIO" \
  || { echo "  ✗ no entra: $(entra "$ROL" "$CLAVE" "$P" "$VM")"; fallos=$((fallos+1)); }
[ "$(entra "$ROL" "$CLAVE" "$P" "$VM-pooler")" = "$ROL" ] && echo "  ✓ y por -pooler el proxy resuelve el mismo cómputo" \
  || echo "  · por -pooler: $(entra "$ROL" "$CLAVE" "$P" "$VM-pooler" | cut -c1-120) (el pool es P5·5)"
case "$(entra "$ROL" "mala" "$P" "$VM")" in "$ROL") echo "  ✗ entra con otra contraseña"; fallos=$((fallos+1));;
  *) echo "  ✓ con otra contraseña, no: $(entra "$ROL" "mala" "$P" "$VM" | cut -c1-100)";; esac
case "$(entra nadie "$CLAVE" "$P" "$VM")" in nadie) echo "  ✗ entra un rol que no existe"; fallos=$((fallos+1));;
  *) echo "  ✓ un rol que no existe, no";; esac
case "$(entra "$ROL" "$CLAVE" "$P" ep-00000000000000000000)" in "$ROL") echo "  ✗ entra en un endpoint que no existe"; fallos=$((fallos+1));;
  *) echo "  ✓ un endpoint que no existe, no";; esac
# La contraseña de A en el endpoint de B (mismo nombre de rol, otra organización): no.
case "$(entra "$ROL" "$CLAVE" "$P" "$VM_B")" in "$ROL") echo "  ✗ la contraseña de $A entra en el de $B"; fallos=$((fallos+1));;
  *) echo "  ✓ la contraseña de $A en el endpoint de $B, no";; esac
[ "$(entra "$ROL" "$CLAVE_B" "$P" "$VM_B")" = "$ROL" ] && echo "  ✓ y el de $B entra en el suyo con la suya" \
  || { echo "  ✗ el de $B no entra en el suyo"; fallos=$((fallos+1)); }

echo "── se borra"
for c in "$A" "$B"; do
  mapfile -t L < <(en "$c" "pide DELETE /v1/postgres/proyectos/$P"); OP=$(campo "${L[0]}" operacion id)
  mapfile -t L < <(en "$c" "hasta_hecha /v1/postgres/operaciones/$OP")
  [ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ borrado en $c" || { echo "  ✗ ${L[0]:0:200}"; fallos=$((fallos+1)); }
done

echo
[ $fallos = 0 ] && echo "P5·1 ✓ todo" || { echo "P5·1 ✗ $fallos fallos"; exit 1; }
