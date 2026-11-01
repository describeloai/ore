#!/usr/bin/env bash
# UNA ORG DE SALESFORCE, DE PUNTA A PUNTA (ADR 0083 paso 5), sin cuenta de Salesforce, contra la org de
# mentira (`salesforce-de-mentira.py`: no hay emulador; está escrita de lo que X0 midió contra una org
# de verdad: un `datetime` guarda segundos, el literal de SOQL ignora la fracción, `!=` incluye los nulos,
# el orden pone los nulos primero, Bulk escribe el nulo como `""`…).
#
#   1  el kit de conformidad: los 14 casos del contrato (1-13 deben pasar; el 14, `estimar`, no aplica),
#      con páginas REST de 500 filas y un `503` cada 13 peticiones
#   2  la pasarela de federación sobre el mismo conector (P1-P9: el flujo, caliente, presupuesto, cola,
#      cancelar, `check`, `catalogo` y la copia completa)
#   3  `cambios_real.rs`: altas, actualizaciones y borrados leídos por el cursor `SystemModstamp` y por
#      `queryAll`, la Papelera, el testigo y el catálogo de cambios (las mismas 7 pruebas que corren contra
#      una org de verdad)
#   4  `mentira.rs`: lo que solo una org de mentira deja provocar: un token que caduca, un `503`, la cuota
#      agotada, un reloj adelantado, un CSV difícil por REST y por Bulk, el umbral hacia Bulk y el aborto
#      de un job
#   y al final: ninguna petición llegó sin su token.
#
# Lo que la org de mentira NO sabe, y queda para la org de verdad (los 24 tests `#[ignore]` de
# `crates/ore-salesforce/tests/laboratorio.rs` y `lectura_real.rs`): el `describe` completo de `Account`,
# la latencia, el rendimiento de Bulk con volumen, la cuota de verdad y CDC (paso 6).
#
#   bash pruebas-de-fuego/o7-salesforce.sh              (en Docker)
#   bash pruebas-de-fuego/o7-salesforce.sh --dentro     (aquí: hace falta cargo, python3 y openssl)
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
RUST_IMG=rust:1-bookworm

# ── dentro: lo que se comprueba ───────────────────────────────────────────────
if [ "${1:-}" = "--dentro" ]; then
  cd "$RAIZ"
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  PY=python3; command -v python3 >/dev/null 2>&1 || PY=python
  PUERTO=${SF_PUERTO:-8794}
  G=http://127.0.0.1:$PUERTO
  W="$(mktemp -d)"
  W="$(cygpath -m "$W" 2>/dev/null || echo "$W")"  # en Windows, los binarios nativos piden C:/…
  trap 'kill $SERVIDOR 2>/dev/null; rm -rf "$W"' EXIT

  # 0 la propia org de mentira reproduce lo medido (si deriva, lo demás no vale)
  "$PY" pruebas-de-fuego/prueba-salesforce-de-mentira.py > "$W/mentira-py.txt" 2>&1     && dice "0 la org de mentira reproduce las rarezas medidas ($(grep -E '^Ran' "$W/mentira-py.txt"))"     || falla "0 la org de mentira: $(tail -8 "$W/mentira-py.txt")"

  # la clave de la app conectada: la privada la usa el conector, la pública verifica la firma la org
  export MSYS_NO_PATHCONV=1
  openssl genrsa -out "$W/kit.key" 2048 2>/dev/null
  openssl rsa -in "$W/kit.key" -pubout -out "$W/kit.pub" 2>/dev/null
  PAGINA=500 CADA_503=13 SF_PUBLICA="$W/kit.pub" "$PY" pruebas-de-fuego/salesforce-de-mentira.py "$PUERTO" > "$W/mentira.log" 2>&1 &
  SERVIDOR=$!
  cargo build --quiet -p ore-conector-kit -p ore-read-salesforce -p ore-federation 2>&1 | tail -3
  for i in $(seq 1 50); do grep -q listo "$W/mentira.log" && break; sleep 0.2; done
  grep -q listo "$W/mentira.log" || { echo "la org de mentira no arranca: $(cat "$W/mentira.log")"; exit 2; }
  T="${CARGO_TARGET_DIR:-$RAIZ/target}/debug"
  X=""; [ -f "$T/ore-kit.exe" ] && X=.exe
  export ORE_SALESFORCE_ENDPOINT=$G ORE_SALESFORCE_CLAVE_FICHERO="$W/kit.key" SALESFORCE_KIT_ENDPOINT=$G

  # 1 el kit
  if "$T/ore-kit$X" --conector "$T/ore-read-salesforce$X" --banco salesforce --exige 1,2,3,4,5,6,7,8,9,10,11,12,13 > "$W/kit.txt" 2>&1; then
    dice "1 el kit: $(grep -c '| \*\*pasa\*\* |' "$W/kit.txt") casos pasan, 1 no aplica (estimar)"
  else
    falla "1 el kit: $(grep -E '\*\*falla\*\*|ore-kit:' "$W/kit.txt" | head -5)"
  fi

  # 2 la pasarela
  if "$T/ore-kit$X" --conector "$T/ore-read-salesforce$X" --banco salesforce --pasarela "$T/ore-federation$X" --exige 1,2,3,4,5,7,8 > "$W/pas.txt" 2>&1; then
    dice "2 la pasarela: $(grep -c '| \*\*pasa\*\* |' "$W/pas.txt") pruebas pasan"
  else
    falla "2 la pasarela: $(grep -E '\*\*falla\*\*|ore-kit:' "$W/pas.txt" | head -5)"
  fi

  # el estado de partida de las pruebas de cambios: una cuenta, un contacto y una línea de oportunidad
  sembrar() { curl -s -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' --data "$2" "$G/_mentira/$1"; }
  CAMPOS_CUENTA='[{"nombre":"NumberOfEmployees","tipo":"int"},{"nombre":"Industry","tipo":"picklist","valores":["Banking","Energy"]}]'
  [ "$(sembrar objeto '{"nombre":"Account","propio":false,"prefijo":"001","campos":'"$CAMPOS_CUENTA"',"filas":[{"Name":"X0-semilla","NumberOfEmployees":"1"}]}')" = 200 ] || falla "sembrar Account"
  [ "$(sembrar objeto '{"nombre":"Contact","propio":false,"prefijo":"003","campos":[{"nombre":"Email","tipo":"email"}]}')" = 200 ] || falla "sembrar Contact"
  [ "$(sembrar objeto '{"nombre":"OpportunityLineItem","propio":false,"prefijo":"00k","campos":[{"nombre":"Quantity","tipo":"double","precision":10,"escala":2}]}')" = 200 ] || falla "sembrar OpportunityLineItem"

  # el `PAGINA=500 CADA_503=13` de arriba es para el kit; las pruebas de abajo cuentan sus propios fallos
  sembrar fallos '{"cada_503":0}' > /dev/null
  export ORE_SALESFORCE_LAB_URL='salesforce://kit.my.salesforce.com/?cliente=3MVGkit&usuario=kit%40lab.ore'
  export ORE_LAB_SALESFORCE_MENTIRA=$G

  # 3 los cambios
  r=$(cargo test --quiet -p ore-salesforce --test cambios_real -- --ignored --test-threads=1 2>&1)
  echo "$r" | grep -q "test result: ok. 7 passed" \
    && dice "3 cambios: altas, actualizaciones y borrados por cursor y queryAll, la Papelera, el testigo y el catálogo (7/7)" \
    || falla "3 cambios: $(echo "$r" | grep -E 'panicked|FAILED|error|assertion' | head -5)"

  # 4 lo que solo la mentira deja provocar
  r=$(cargo test --quiet -p ore-salesforce --test mentira -- --ignored --test-threads=1 2>&1)
  echo "$r" | grep -q "test result: ok. 8 passed" \
    && dice "4 la mentira: token caducado, 503, cuota agotada y por encima del tope, reloj adelantado, CSV difícil por REST y Bulk, umbral y aborto (8/8)" \
    || falla "4 la mentira: $(echo "$r" | grep -E 'panicked|FAILED|error|assertion' | head -6)"

  # y al final: ninguna petición sin su token
  c=$(curl -s "$G/_mentira/cuentas")
  echo "$c" | grep -q '"sin_token": 0' && dice "ninguna petición llegó sin su token" || falla "sin token: $c"
  exit $fallos
fi

# ── fuera: el laboratorio ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker (o --dentro, con cargo, python3 y openssl)"; exit 2; }
export MSYS_NO_PATHCONV=1  # los volúmenes de Docker, en Windows
echo "una org de Salesforce (laboratorio: la org de mentira, páginas de 500 y un 503 cada 13)"
docker run --rm -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/o7-salesforce.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
fallos=${PIPESTATUS[0]}
[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
