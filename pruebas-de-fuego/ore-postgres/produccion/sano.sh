#!/usr/bin/env bash
# Ventana 1, paso 1 (ADR 0058): tras `malla/despertar.sh --con-postgres`, ¿está todo como debe?
#   · este PC: Docker contesta y la imagen de psql trae la raíz de Let's Encrypt (sslrootcert=system);
#   · ore-pg: todos los pods listos (el almacenamiento, el controller, ore-postgres);
#   · ore-postgres corre la imagen de `main` y tiene las migraciones 005–008 (P5·6 … P6·5);
#   · `--dormir` y el pool, APAGADOS (se encienden en sus pasos);
#   · los endpoints que ya había (postgre, el de la consola) siguen `listo`.
# Solo lee.
PRUEBA=sano
source "$(dirname "$0")/comun.sh"

echo "── este PC"
docker info >/dev/null 2>&1 && bien "Docker contesta" || { mal "Docker no contesta: arráncalo antes de seguir"; exit 1; }
n=$(docker run --rm "$PG_IMAGEN" sh -c 'ls /etc/ssl/certs 2>/dev/null | grep -ci isrg_root')
[ "${n:-0}" -ge 1 ] && bien "$PG_IMAGEN trae la raíz de Let's Encrypt ($n)" || mal "$PG_IMAGEN sin ISRG en /etc/ssl/certs: sslrootcert=system no valdrá"

echo "── ore-pg"
noListos=$(k get pods --no-headers 2>/dev/null | awk '$3 != "Completed" && ($2 !~ /^([0-9]+)\/\1$/ || $3 != "Running") {print $1" "$2" "$3}')
[ -z "$noListos" ] && bien "todos los pods de ore-pg listos ($(k get pods --no-headers | grep -c Running) corriendo)" || mal "sin listo: $noListos"
img=$(k get deploy ore-postgres -o jsonpath='{.spec.template.spec.containers[0].image}')
digest=$(k get pods -l ore.dev/rol=plano-postgres -o jsonpath='{.items[0].status.containerStatuses[0].imageID}')
echo "  · ore-postgres: $img (${digest##*@})"
args=$(k get deploy ore-postgres -o jsonpath='{.spec.template.spec.containers[0].args}')
case "$args" in *--dormir*) mal "--dormir ya está puesto (va en su paso, después del proxy)";; *) bien "--dormir apagado";; esac
case "$args" in *--pool-computos*) mal "el pool ya está puesto";; *) bien "el pool apagado";; esac
m=$(plano_sql "select string_agg(nombre, ' ' order by nombre) from public.migracion where nombre >= '005'")
for x in 005-quien-entra 006-los-limites 007-dormir 008-el-pool; do
  case " $m " in *" $x "*) bien "migración $x aplicada";; *) mal "falta la migración $x (¿corre el binario nuevo?)";; esac
done

echo "── los endpoints que ya había"
plano_sql "select organizacion || '/' || proyecto || '/' || id || ' ' || observado || ' ' || coalesce(computo, vm) || ' dormir_tras=' || dormir_tras from plano.endpoint where deseado = 'vivo' order by 1" | sed 's/^/  · /'
malos=$(plano_sql "select count(*) from plano.endpoint where deseado = 'vivo' and observado <> 'listo'")
[ "$malos" = 0 ] && bien "todos los vivos, listos" || mal "$malos endpoints vivos que no están listos"
echo
[ $fallos = 0 ] && echo "sano ✓" || { echo "sano ✗ $fallos"; exit 1; }
