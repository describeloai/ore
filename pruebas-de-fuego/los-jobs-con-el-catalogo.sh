#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# LOS JOBS CON EL CATÁLOGO TRANSACCIONAL (ADR 0079 ·5d·2)
#
# Lo que cambia en un Job con `ORE_CATALOGO_ESQUEMA` es a quién habla su `ore`:
# `ORE_PUNTEROS="ore-catalogo punteros --esquema <e>"`. Esto corre las MISMAS
# órdenes que los guiones de la malla —la copia (48) y el copiador
# (`materialize --recoger --informe datasets`), el mantenimiento (53,
# `datasets --recoger` y `collections --recoger`), `ore puntero` y
# `cerrar-ejecucion.py`— contra un Postgres de verdad y un lago de mentira
# (el S3 de `de-mentira.py`), y comprueba:
#
#   1  confirmar escribe el puntero en el catálogo y NO en el árbol
#   2  la copia sobre el lago deja su puntero en el catálogo, sin fichero
#   3  `ore puntero` y `cerrar-ejecucion.py` lo leen del catálogo
#   4  el mantenimiento expira lo superado, mueve el puntero EN EL CATÁLOGO y
#      NO borra lo que sólo el catálogo nombra (la copia: tras el corte, ningún
#      fichero de `datasets/` la reclama)
#   5  una tabla escrita en una RAMA del catálogo sobrevive a la recogida de
#      `main`: `reclamados()` ve todas las refs, sin el «reclaman» del shell
#   6  las colecciones: `--recoger` sale bien
#
# El guion entero de cada Job (Keycloak, el cofre, la forja del inquilino) se
# prueba en el corte (·5g); aquí, lo que cambia.
#
#   ORE_CATALOGO_PRUEBAS=postgres://…  ESQUEMA=t_x (creado y vacío)
#   y `ore`, `ore-catalogo`, `ore-store-r2` en el PATH o en target/.
# ═══════════════════════════════════════════════════════════════════════════
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python) || { echo "hace falta python"; exit 2; }
buscar() {
  for n in "${CARGO_TARGET_DIR:-$RAIZ/target}/debug/$1" "$RAIZ/target/release/$1" "$RAIZ/target/debug/$1"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  command -v "$1"
}
ORE="$(buscar ore)" || { echo "no hay \`ore\`"; exit 2; }
CAT="$(buscar ore-catalogo)" || { echo "no hay \`ore-catalogo\`"; exit 2; }
STORE="$(buscar ore-store-r2)" || { echo "no hay \`ore-store-r2\`"; exit 2; }
export PATH="$(dirname "$ORE"):$(dirname "$CAT"):$(dirname "$STORE"):$PATH"
[ -n "${ORE_CATALOGO_PRUEBAS:-}" ] && [ -n "${ESQUEMA:-}" ] || { echo "⚠ sin ORE_CATALOGO_PRUEBAS/ESQUEMA: no se prueba"; exit 0; }

fallos=0
ok()    { echo "  ✓ $1"; }
falla() { echo "✗ $1"; fallos=$((fallos + 1)); }

TMP="${TMPDIR:-/tmp}/ore-jobs-cat-$$"
rm -rf "$TMP"; mkdir -p "$TMP/vacio" "$TMP/calculo"
S3_PID=""
trap 'kill $S3_PID 2>/dev/null; rm -rf "$TMP"' EXIT

"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"
objs() { curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=$1" | grep -o '<Key>[^<]*</Key>' | wc -l | tr -d ' '; }

# ── lo que pone el bloque de los guiones (0079 ·5d) ──────────────────────────
export CATALOGO_URL="$ORE_CATALOGO_PRUEBAS"
export ORE_PUNTEROS="ore-catalogo punteros --esquema $ESQUEMA --sembrar"
unset ORE_REF

A="$TMP/arbol"; mkdir -p "$A"
( cd "$A" && "$ORE" init . --name lago >/dev/null 2>&1 ) || { echo "ore init"; exit 1; }
mkdir -p "$A/packages/ventas/datasets"
cat > "$A/packages/ventas/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 1.0.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: lago }
spec:
  owner: team:data
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y

# Lo que un puesto escribiría: una tabla Iceberg en el bucket (`write()`).
escribe() { # dataset base snapshot → metadata_location
  { echo "{\"dataset\":\"$1\",\"fundir\":false,${2:+\"base\":\"$2\",}\"clave\":[],\"conducto\":\"puesto:ana\",\"esquema\":{\"id\":\"Integer\",\"pais\":\"String\"},\"plan\":\"sha256:escrito\",\"testigo\":{\"modo\":\"snapshot\",\"valor\":\"$3\"}}"
    echo '{"id":"1","pais":"ES"}'; echo '{"id":"2","pais":"PT"}'; echo "{\"id\":\"$3$3\",\"pais\":\"ES\"}"; } \
    | "$STORE" sellar > "$TMP/sellado.json" 2>"$TMP/sellado.err" || { cat "$TMP/sellado.err" >&2; return 1; }
  "$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata_location"])' "$TMP/sellado.json"
}
campo() { "$PY" -c 'import json,sys;print(json.loads(sys.stdin.read()).get(sys.argv[1],""))' "$1"; }
puntero() { "$ORE" puntero "$1" --path "$A"; }
confirmar() { # nombre ml esperado
  "$ORE" datasets "$A" --confirmar "$1" --metadata-location "$2" ${3:+--esperado "$3"} --snapshot 1 --filas 3 \
    --columnas '{"id":"Integer","pais":"String"}' --sujeto persona:ana > "$TMP/conf.out" 2>&1
}

# ── 1 · confirmar: el puntero al catálogo, no al árbol ───────────────────────
ML1=$(escribe catalogo/ventas/default/salida "" 1) || { falla "1 · el escritor"; exit 1; }
confirmar ventas.salida "$ML1" || falla "1 · confirmar: $(cat "$TMP/conf.out")"
[ "$(puntero ventas.salida | campo metadata_location)" = "$ML1" ] || falla "1 · el puntero no está en el catálogo: $(puntero ventas.salida)"
[ -e "$A/datasets/ventas/default/salida.json" ] && falla "1 · el puntero quedó en el árbol: con el catálogo no hay fichero"
[ -f "$A/packages/ventas/datasets/salida.yaml" ] || falla "1 · el Dataset (el documento) tenía que nacer en el árbol"
[ "$fallos" = 0 ] && ok "1 · confirmar escribe el puntero en el catálogo, y el documento en el árbol"

# ── 2 · la copia (48, el copiador): sobre el lago, su puntero al catálogo ─────
cat > "$A/packages/ventas/datasets/es.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: es, namespace: ventas }
spec:
  owner: team:ventas
  from: { dataset: ventas.salida }
  fields: { id: id, pais: pais }
  where: { pais: ES }
Y
f0=$fallos
( cd "$A" && "$ORE" materialize . --vista ventas.es --recoger --reclaman "$TMP/vacio" --informe datasets --calculado "$TMP/calculo" ) \
  > "$TMP/mat.txt" 2>&1 || falla "2 · materialize: $(tail -5 "$TMP/mat.txt")"
P=$(puntero ventas.es)
[ "$(printf '%s' "$P" | campo estado)" = copiada ] && [ "$(printf '%s' "$P" | campo filas)" = 2 ] || falla "2 · la copia: $P · $(tail -3 "$TMP/mat.txt")"
[ -e "$A/datasets/ventas/default/es.json" ] && falla "2 · el informe de la copia quedó en el árbol"
ML_ES=$(printf '%s' "$P" | campo metadata_location)
[ "$fallos" = "$f0" ] && ok "2 · la copia sobre el lago deja su puntero (copiada, 2 filas) en el catálogo, sin fichero"

# ── 3 · `ore puntero` y `cerrar-ejecucion.py` leen del catálogo ──────────────
f0=$fallos
[ "$(puntero ventas.nunca; echo "rc=$?")" = "rc=66" ] || falla "3 · sin puntero tenía que ser 66"
"$PY" - "$RAIZ/malla/cerrar-ejecucion.py" "$A" <<'EOF' || falla "3 · cerrar-ejecucion no lee el puntero del catálogo"
import importlib.util, sys
s = importlib.util.spec_from_file_location("cerrar", sys.argv[1]); m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
p = m.puntero(sys.argv[2], "ventas.es")
assert p and p.get("estado") == "copiada" and str(p.get("filas")) == "2", p
assert m.puntero(sys.argv[2], "ventas.nunca") is None
EOF
[ "$fallos" = "$f0" ] && ok "3 · \`ore puntero\` (66 si no hay) y \`cerrar-ejecucion.py\` leen el puntero del catálogo"

# ── 4 · el mantenimiento (53): expira, mueve el puntero EN EL CATÁLOGO y no
#        borra lo que sólo el catálogo nombra ──────────────────────────────────
f0=$fallos
ML2=$(escribe catalogo/ventas/default/salida "$ML1" 2) || falla "4 · segunda escritura"
confirmar ventas.salida "$ML2" "$ML1" || falla "4 · confirmar la segunda: $(cat "$TMP/conf.out")"
ANTES=$(objs ore/v2/catalogo/ventas/default/salida/)
ES_ANTES=$(objs ore/v2/catalogo/ventas/default/es/)
"$ORE" datasets "$A" --recoger --edad 0 --gracia 0 --reclaman "$TMP/vacio" > "$TMP/rec.txt" 2>&1 || falla "4 · recoger: $(tail -5 "$TMP/rec.txt")"
DESPUES=$(objs ore/v2/catalogo/ventas/default/salida/)
[ "$DESPUES" -lt "$ANTES" ] || falla "4 · recoger no expiró nada ($ANTES → $DESPUES): $(tail -5 "$TMP/rec.txt")"
ML3=$(puntero ventas.salida | campo metadata_location)
[ -n "$ML3" ] && [ "$ML3" != "$ML2" ] || dice_ml="(el puntero sigue en $ML3)"
[ "$(objs ore/v2/catalogo/ventas/default/es/)" = "$ES_ANTES" ] || falla "4 · ⛔ la recogida borró la copia que sólo nombra el catálogo ($ES_ANTES → $(objs ore/v2/catalogo/ventas/default/es/))"
( cd "$A" && "$ORE" datasets . --muestra ventas.salida --limite 5 ) > "$TMP/m.txt" 2>&1 || falla "4 · tras recoger, ventas.salida no se lee: $(tail -3 "$TMP/m.txt")"
( cd "$A" && "$ORE" datasets . --muestra ventas.es --limite 5 ) > "$TMP/m.txt" 2>&1 || falla "4 · tras recoger, la copia no se lee: $(tail -3 "$TMP/m.txt")"
[ -e "$A/datasets/ventas/default/salida.json" ] && falla "4 · recoger escribió el puntero en el árbol"
[ "$fallos" = "$f0" ] && ok "4 · el mantenimiento expira ($ANTES → $DESPUES objetos), el puntero sigue legible en el catálogo y la copia (sólo en el catálogo) no se toca ${dice_ml:-}"

# ── 5 · lo de una rama sobrevive a la recogida de `main`, sin «reclaman» ──────
f0=$fallos
"$PY" - <<EOF || falla "5 · no se pudo crear la rama en el catálogo"
import json, subprocess
p = subprocess.Popen(["ore-catalogo", "punteros", "--esquema", "$ESQUEMA"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def pide(j):
    p.stdin.write(json.dumps(j) + "\n"); p.stdin.flush(); r = json.loads(p.stdout.readline())
    assert "error" not in r, r
    return r
pide({"op": "abrir", "raiz": "$A"})
pide({"op": "crear", "rama": "ana/x", "desde": "main"})
EOF
ML_R=$(escribe catalogo/ventas/default/rama "" 7) || falla "5 · escribir en la rama"
export ORE_REF=ana/x; confirmar ventas.rama "$ML_R" || falla "5 · confirmar en la rama: $(cat "$TMP/conf.out")"; unset ORE_REF
[ -z "$(puntero ventas.rama)" ] || falla "5 · lo de la rama se ve en main"
R_ANTES=$(objs ore/v2/catalogo/ventas/default/rama/)
"$ORE" datasets "$A" --recoger --edad 0 --gracia 0 --reclaman "$TMP/vacio" > "$TMP/rec2.txt" 2>&1 || falla "5 · recoger: $(tail -5 "$TMP/rec2.txt")"
[ "$R_ANTES" -gt 0 ] && [ "$(objs ore/v2/catalogo/ventas/default/rama/)" = "$R_ANTES" ] || falla "5 · ⛔ la recogida de main borró lo de la rama ($R_ANTES → $(objs ore/v2/catalogo/ventas/default/rama/))"
[ "$fallos" = "$f0" ] && ok "5 · lo escrito en una rama del catálogo sobrevive a la recogida de main ($R_ANTES objetos), sin el «reclaman» del shell"

# ── 6 · las colecciones (53) ─────────────────────────────────────────────────
"$ORE" collections "$A" --recoger --reclaman "$TMP/vacio" > "$TMP/col.txt" 2>&1 && ok "6 · \`collections --recoger\` con el catálogo sale bien" || falla "6 · collections --recoger: $(tail -3 "$TMP/col.txt")"

[ "$fallos" = 0 ] && { echo "todo en verde"; exit 0; }
echo "✗ $fallos fallos"; exit 1
