#!/usr/bin/env bash
# ORE 0068 C2·3 2b·3 · UN BUILD HEAVYWEIGHT DE PUNTA A PUNTA, en local: el banco de C2·4 (ore-serve
# release con lo de C2·3, S3 de mentira, cola), dos datasets geo sembrados con el SDK pesado, un
# repositorio `transforms-sql` con `motor: heavyweight` y sus dos `.sql`; `POST /builds` como ana, y
# el AGENTE del trabajo en `ore-puesto-spark:local` (como el Job: `PUESTO`, `TRABAJO`), con el SDK del
# árbol montado encima. Al final, lo que el build y el lago dicen.
#   1 · los binarios: cargo build --release -p ore-cli --bin ore -p ore-serve --bin ore-serve
#       -p ore-store --bin ore-store-r2   (en rust:1, con el volumen ore-target)
#   2 · bash pruebas-de-fuego/geo/c2/build-pesado/medir.sh
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
IMAGEN="${IMAGEN:-ore-puesto-spark:local}"
SDK=(-v "$raiz/puesto/python/ore":/opt/ore/ore:ro)
ex() { MSYS_NO_PATHCONV=1 docker exec c2-banco "$@"; }

docker rm -f c2-banco >/dev/null 2>&1 || true
MSYS_NO_PATHCONV=1 docker run -d --name c2-banco -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
  bash /repo/pruebas-de-fuego/geo/c2/spark-por-v1/banco.sh >/dev/null
for i in $(seq 1 120); do docker logs c2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
docker logs c2-banco 2>&1 | grep -q "banco listo" || { docker logs c2-banco 2>&1 | tail -30; exit 1; }

echo "== 1 · sembrar (SDK pesado, como ana)"
MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco -v "$aqui":/l:ro "${SDK[@]}" "$IMAGEN" \
  python3 /l/sembrar.py 2>/dev/null | tail -2
echo "== 2 · el repositorio Heavyweight y sus .sql"
ex bash /repo/pruebas-de-fuego/geo/c2/build-pesado/repo.sh

for salida in geo.conteo geo.centros geo.anillos geo.fuera; do
  echo "== 3 · POST /builds $salida"
  r=$(ex curl -s -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' \
        -d "{\"output\":\"$salida\"}" http://127.0.0.1:8080/builds)
  echo "$r" | python3 -c "import json,sys; d=json.load(sys.stdin); print('  ', {k: d.get(k) for k in ('build','entorno','codigo','commit','estado','error','motivo')})"
  bid=$(echo "$r" | python3 -c "import json,sys; print(json.load(sys.stdin).get('build',''))")
  [ -n "$bid" ] || continue
  trabajo=$(echo "$r" | python3 -c "import json,sys; d=json.load(sys.stdin); print('%s@%s' % (d['codigo'], d.get('commit') or 'local'))")
  echo "== 4 · el agente del trabajo $bid, en $IMAGEN"
  MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco --memory 4g "${SDK[@]}" \
    -e ORE_SERVE=http://127.0.0.1:8080 -e PUESTO="$bid" -e TRABAJO="$trabajo" -e ORE_SUJETO=agente:local \
    -e TTL=300 -e ORE_MEMORIA_MB=4096 -e PUESTO_DIR=/tmp -e TRABAJO_DIR=/tmp "$IMAGEN" 2>&1 \
    | grep -v -E "WARN|^\s+at |log4j|setLogLevel|NativeCodeLoader|Stage [0-9]|^\s*$" | tail -6 | cut -c1-260 || true
  echo "== 5 · lo que el build dice"
  ex curl -s -H 'x-ore-sujeto: persona:ana' "http://127.0.0.1:8080/builds/$bid" \
    | python3 -c "import json,sys; d=json.load(sys.stdin); e=d.get('error') or {}; m=[l for l in (e.get('mensaje') or '').splitlines() if 'Exception' in l or 'Error' in l][:1]; print('  ', d.get('estado'), '· filas', d.get('filas'), '·', m[0][:220] if m else '')"
  echo "== 6 · lo que el lago dice de $salida"
  t=${salida#geo.}
  ex curl -s -H 'x-ore-sujeto: persona:ana' "http://127.0.0.1:8080/v1/geo/namespaces/default/tables/$t" | python3 -c "
import json, sys
d = json.load(sys.stdin)
if 'error' in d:
    print('  ', d['error']); sys.exit()
m = d['metadata']
s = next(x for x in m['schemas'] if x['schema-id'] == m['current-schema-id'])
snap = m['snapshots'][-1]['summary']
pr = json.loads(snap.get('ore.procedencia') or '{}')
print('   esquema', [(f['name'], f['type']) for f in s['fields']])
print('   ore.geo', {k: v for k, v in m['properties'].items() if k.startswith('ore.geo.')})
print('   filas', snap.get('total-records'), '· procedencia: build', (pr.get('build') or {}).get('id'), '· inputs', pr.get('inputs'))
"
done
