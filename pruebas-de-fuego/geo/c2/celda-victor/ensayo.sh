#!/usr/bin/env bash
# ORE 0068 6c · El ensayo, en local, de lo que se hará en victor desde la consola: el banco de C2·4, los tres
# repositorios Heavyweight de `consola/` (puntos rebajados), y los cinco builds en orden —sembrar puntos y
# zonas, y el join en SQL, Python y Java—, cada uno con el agente del trabajo en `ore-puesto-spark:local`.
# Al final, los tres conteos tienen que ser el mismo.
#   bash pruebas-de-fuego/geo/c2/celda-victor/ensayo.sh
set -euo pipefail
raiz="$(cd "$(dirname "$0")/../../../.." && pwd -W 2>/dev/null || pwd)"
IMAGEN="${IMAGEN:-ore-puesto-spark:local}"
SDK=(-v "$raiz/puesto/python/ore":/opt/ore/ore:ro)
ex() { MSYS_NO_PATHCONV=1 docker exec c2-banco "$@"; }
docker rm -f c2-banco >/dev/null 2>&1 || true
MSYS_NO_PATHCONV=1 docker run -d --name c2-banco -v "$raiz":/repo:ro -v ore-target:/repo/target:ro rust:1 \
  bash /repo/pruebas-de-fuego/geo/c2/spark-por-v1/banco.sh >/dev/null
for i in $(seq 1 120); do docker logs c2-banco 2>&1 | grep -q "banco listo" && break; sleep 1; done
ex env PUNTOS="${PUNTOS:-200000}" bash /repo/pruebas-de-fuego/geo/c2/celda-victor/ensayo-repo.sh
for salida in geo.puntos_20m geo.zonas_20k geo.conteo_sql geo.conteo_py geo.conteo_java; do
  r=$(ex curl -s -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' \
        -d "{\"output\":\"$salida\"}" http://127.0.0.1:8080/builds)
  bid=$(echo "$r" | python3 -c "import json,sys; print(json.load(sys.stdin).get('build',''))")
  [ -n "$bid" ] || { echo "FALLA · $salida · $r" | cut -c1-300; continue; }
  trabajo=$(echo "$r" | python3 -c "import json,sys; d=json.load(sys.stdin); print('%s@%s' % (d['codigo'], d.get('commit') or 'local'))")
  t0=$(date +%s)
  MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco --memory 4g "${SDK[@]}" \
    -e ORE_SERVE=http://127.0.0.1:8080 -e PUESTO="$bid" -e TRABAJO="$trabajo" -e ORE_SUJETO=agente:local \
    -e TTL=300 -e ORE_MEMORIA_MB=4096 -e PUESTO_DIR=/tmp -e TRABAJO_DIR=/tmp "$IMAGEN" >/dev/null 2>&1 || true
  ex curl -s -H 'x-ore-sujeto: persona:ana' "http://127.0.0.1:8080/builds/$bid" | python3 -c "
import json,sys; d=json.load(sys.stdin); e=d.get('error') or {}
print('%s · $salida · %s · filas %s · %ss %s' % ('OK   ' if d.get('estado')=='succeeded' else 'FALLA', d.get('estado'), d.get('filas'), $(( $(date +%s) - t0 )), (e.get('mensaje') or '')[:200]))"
done
MSYS_NO_PATHCONV=1 docker run --rm --network container:c2-banco "${SDK[@]}" -e ORE_MOTOR=heavyweight -e ORE_SERVE=http://127.0.0.1:8080 \
  -e ORE_SUJETO=persona:ana -e ORE_MEMORIA_MB=4096 "$IMAGEN" python3 -c "
import sys; sys.path.insert(0, '/opt/ore'); import ore
ore.session._cabeceras = {'x-ore-sujeto': 'persona:ana'}
c = {t: sorted((r.id, r.n) for r in ore.over('geo.' + t).collect()) for t in ('conteo_sql', 'conteo_py', 'conteo_java')}
n = {t: sum(x[1] for x in v) for t, v in c.items()}
print(('OK   ' if c['conteo_sql'] == c['conteo_py'] == c['conteo_java'] else 'FALLA') + ' · los tres conteos', n)
" 2>/dev/null
