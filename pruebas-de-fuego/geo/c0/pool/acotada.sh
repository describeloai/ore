#!/usr/bin/env bash
# ORE 0068 C0·1–C0·3 · La medida a escala, versión acotada, en el pod c0 del pool c0-medida.
#   kubectl -n c0-medida exec c0 -- bash -c 'nohup bash /trabajo/c0/pool/acotada.sh > /trabajo/acotada.log 2>&1 &'
#
# Lo que la hace robusta (el primer intento murió por disco a las 12:31 y perdió lo medido):
#  · cada resultado se sube al bucket según sale (computo/c0/acotada/<motor>-<datos>-<caso>.json);
#  · idempotente: al arrancar recupera del bucket lo ya medido y salta cada fase y caso hechos, así que
#    si el pod muere basta con crear otro y relanzar;
#  · el disco: se borran el CSV del AIS y los originales de Overture al preparar el banco, DuckDB
#    tiene un tope de 80 GB de desborde y Spark limpia su directorio tras cada consulta;
#  · memoria: DuckDB 48 GB y Spark 40 GB de heap (el límite del pod es 58 GiB).
# Lo acotado: Spark con límite de 600 s; sin Q2 ni B2 sin difusión (ya se sabe que se planifican
# mal); Q10–Q12 de DuckDB a escala 10 no se repiten (agotaron 1 200 s en el primer intento) y nada
# que agotara a escala 10 se lanza a escala 100.
set -uo pipefail
T=/trabajo
B=project-8853a180-450d-47be-b83-banco-geo
P=computo/c0/acotada
export C0_EXT=$T/ext SB_QUERIES=$T/sb-queries HOME=$T/home
SAL=$T/resultados/acotada.jsonl
mkdir -p $T/resultados $T/hecho
marca() { echo "[$(date +%H:%M:%S)] $*"; }
tok() { curl -s -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token | python3 -c "import json,sys;print(json.load(sys.stdin)['access_token'])"; }

# ── Lo ya medido, desde el bucket ─────────────────────────────────────────────────────────
recuperar() {
  python3 - "$(tok)" "$B" "$P" "$SAL" <<'EOF'
import json, sys, urllib.request, urllib.parse
tok, b, p, sal = sys.argv[1:]
h = {"Authorization": "Bearer " + tok}
url = f"https://storage.googleapis.com/storage/v1/b/{b}/o?prefix={urllib.parse.quote(p + '/')}"
items = json.load(urllib.request.urlopen(urllib.request.Request(url, headers=h))).get("items", [])
with open(sal, "w") as f:
    for it in items:
        u = f"https://storage.googleapis.com/storage/v1/b/{b}/o/{urllib.parse.quote(it['name'], safe='')}?alt=media"
        f.write(urllib.request.urlopen(urllib.request.Request(u, headers=h)).read().decode().strip() + "\n")
print(f"recuperados {len(items)}")
EOF
}
subir() { # línea JSON por stdin → un objeto por caso
  while IFS= read -r l; do
    case "$l" in "{"*) ;; *) continue;; esac
    echo "$l" >> $SAL; echo "$l" | cut -c1-160
    n=$(echo "$l" | python3 -c "import json,sys;r=json.load(sys.stdin);print('%s-%s-%s' % (r['motor'], r['datos'].strip('/').replace('/', '_'), r['caso']))")
    curl -sf -X POST -H "Authorization: Bearer $(tok)" -H "Content-Type: application/json" --data-binary "$l" \
      "https://storage.googleapis.com/upload/storage/v1/b/$B/o?uploadType=media&name=$P/$n.json" >/dev/null || echo "AVISO: no subió $n"
  done
}
hecho() { grep "\"motor\": \"$1\"" $SAL 2>/dev/null | grep "\"datos\": \"$2\"" | grep -q "\"caso\": \"$3\""; }
agoto() { grep "\"motor\": \"$1\"" $SAL 2>/dev/null | grep "\"datos\": \"$T/sb/sf10\"" | grep -q "\"caso\": \"$2\", \"estado\": \"limite\""; }

# ── Fases ─────────────────────────────────────────────────────────────────────────────────
recuperar
fase() { [ -f $T/hecho/$1 ]; }
listo() { touch $T/hecho/$1; marca "fase $1 hecha"; }

fase preparar || { marca "preparar el pod"; bash $T/c0/pool/preparar-pod.sh 2>&1 | grep -E "^==|spatial y h3" ; listo preparar; }
fase banco || {
  marca "banco (7 días de AIS)"
  C0_MEMORIA=48GB python3 $T/c0/preparar-banco.py $T/ov $T/ov/ais $T/banco && rm -rf $T/banco/tmp $T/ov && listo banco
}
for s in 10 100; do
  fase sf$s || { marca "SpatialBench escala $s"; rm -rf $T/sb/sf$s; $T/cargo/bin/spatialbench-cli -s $s --format=parquet --output-dir $T/sb/sf$s && listo sf$s; }
done
df -h $T | tail -1

# Los tres de DuckDB que agotaron a escala 10 en el primer intento (leídos a las 12:22).
for q in q10 q11 q12; do
  hecho duckdb $T/sb/sf10 $q || echo "{\"motor\": \"duckdb\", \"conjunto\": \"spatialbench\", \"datos\": \"$T/sb/sf10\", \"caso\": \"$q\", \"estado\": \"limite\", \"segundos\": 1200, \"nota\": \"primer intento, 12:22\"}" | subir
done

duck() { # datos caso límite
  hecho duckdb "$1" "$2" && return
  C0_MEMORIA=48GB C0_TMP=$T/duck-tmp python3 $T/c0/correr.py duckdb "$4" "$1" - "$3" "$2" 2>&1 | subir
  rm -rf $T/duck-tmp
}
CP=$(ls $T/ivy/jars/*.jar | tr '\n' ':')
chispa() { # datos caso límite conjunto
  hecho spark "$1" "$2" && return
  /opt/spark/bin/spark-submit --master 'local[16]' --driver-memory 40g \
    --driver-class-path "$CP" --conf spark.executor.extraClassPath="$CP" \
    --conf spark.local.dir=$T/spark-tmp --conf spark.sql.shuffle.partitions=64 \
    --conf spark.sql.extensions=org.apache.sedona.sql.SedonaSqlExtensions \
    --conf spark.serializer=org.apache.spark.serializer.KryoSerializer \
    --conf spark.kryo.registrator=org.apache.sedona.core.serde.SedonaKryoRegistrator \
    $T/c0/correr.py spark "$4" "$1" - "$3" "$2" 2>&1 | subir
  rm -rf $T/spark-tmp
}

# Ensayo: una consulta por motor, para ver que todo está en su sitio antes de lo largo.
marca "ensayo"; duck $T/sb/sf10 q1 1200 spatialbench; chispa $T/sb/sf10 q1 600 spatialbench

marca "duckdb"
for q in q1 q2 q3 q4 q5 q6 q7 q8 q9; do duck $T/sb/sf10 $q 1200 spatialbench; done
for q in q1 q2 q3 q4 q5 q6 q7 q8 q9 q10 q11 q12; do agoto duckdb $q || duck $T/sb/sf100 $q 1200 spatialbench; done
for q in b1 b2 b3 b4; do duck $T/banco $q 1200 banco; done

marca "spark"
for q in b1 b2d b3d b4; do chispa $T/banco $q 600 banco; done
for q in q1 q2d q3 q4 q5 q6 q7 q8 q9 q10 q11 q12; do chispa $T/sb/sf10 $q 600 spatialbench; done
for q in q1 q2d q3 q4 q5 q6 q7 q8 q9 q10 q11 q12; do agoto spark $q || chispa $T/sb/sf100 $q 600 spatialbench; done

marca fin
