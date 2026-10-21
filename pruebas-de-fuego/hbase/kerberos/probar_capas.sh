#!/bin/bash
# Las tres protecciones del RPC (N6 de 0072): levanta el laboratorio con cada una, crea `kt` y corre los tests de Kerberos.
#   probar_capas.sh [integrity authentication privacy]
# Hace falta haber compilado antes (volumenes cargoreg y krbtarget): ver la ficha.
export MSYS_NO_PATHCONV=1
FALLO=0
AQUI="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
RAIZ="${ORE_RAIZ:-C:/ore-hbase}"
for P in "${@:-integrity authentication privacy}"; do
  echo "=== $P"
  bash "$AQUI/kerberos.sh" "$P" >/dev/null 2>&1
  # el arranque puede salir antes de que el maestro exista (carga de la maquina): se repite una vez
  docker ps --format '{{.Names}}' | grep -q '^hbk$' || { docker start hbk >/dev/null; sleep 45; }
  docker cp "$AQUI/kt.rb" hbk:/tmp/kt.rb
  docker exec hbk bash -c '/opt/hbase/bin/hbase shell -n /tmp/kt.rb 2>&1 | tail -2'
  docker run --rm --network krb -v "$RAIZ:/src" -v cargoreg:/usr/local/cargo/registry -v krbtarget:/target -v krbkeys:/keytabs \
    -v "$AQUI/krb5.conf:/etc/krb5.conf:ro" -e CARGO_TARGET_DIR=/target \
    -e "HBASE_LAB_KERBEROS_URL=hbase://hb.ore.test:2181?auth=kerberos&principal=lector@ORE.TEST&keytab=/keytabs/lector.keytab&paralelo=4" \
    -w /src rust:1-bookworm bash -c "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libkrb5-dev clang libclang-dev krb5-user >/dev/null 2>&1; cargo test -p ore-hbase --features kerberos --test laboratorio kerberos -- --test-threads=1 2>&1 | grep -E '^test |test result|^error'" | tee /tmp/capa.txt
  grep -q 'test result: ok. 4 passed' /tmp/capa.txt || { echo "⛔ $P: no pasan los cuatro"; FALLO=1; }
done
exit $FALLO
