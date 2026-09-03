#!/usr/bin/env bash
# R6 · **la definición de listo** de `docs/handoff-refresco.md`.
#
# No dice «verde»: dice **cuántas filas se leyeron del origen en cada acto**, que
# es la unidad que el ADR 0014 fijó para el proyecto. Un refresco que funciona
# pero relee el origen entero no está listo — está poblando otra vez con otro
# nombre, y eso también saldría verde en una prueba que solo mirase el código de
# salida.
#
#   set -a; . ./.env.local; set +a
#   PATH="$PWD/target/debug:$PATH" bash pruebas-de-fuego/refresco.sh
#
# **Esta prueba nace en rojo, y eso es el diseño.** Cada ✗ nombra el peldaño que
# lo cierra, así que su salida ES la lista de trabajo del plan.
set -u

ORE="${ORE:-./target/debug/ore.exe}"
[ -x "$ORE" ] || ORE="./target/debug/ore"

for v in ORE_R2_S3_ENDPOINT ORE_R2_BUCKET ORE_R2_ACCESS_KEY_ID ORE_R2_SECRET_ACCESS_KEY; do
  if [ -z "${!v:-}" ]; then
    echo "falta \$$v — carga el entorno primero:  set -a; . ./.env.local; set +a"
    exit 2
  fi
done
command -v ore-store-r2 >/dev/null || { echo "pon target/debug en el PATH"; exit 2; }

fallos=0
ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; }
mal() { printf '  \033[31m✗\033[0m %s\n      \033[2m→ lo cierra %s\033[0m\n' "$1" "$2"; fallos=$((fallos + 1)); }
cmp_n() { # esperado real texto peldaño
  if [ "$1" = "$2" ]; then ok "$3: $2"; else mal "$3: esperado $1, salió $2" "$4"; fi
}

# ── el terreno ───────────────────────────────────────────────────────────────
D="${TMPDIR:-/tmp}/ore-r6-$$"
rm -rf "$D"; mkdir -p "$D/datos" "$D/tables" "$D/views"

filas() { # n desde
  local n=$1 i=$2
  while [ "$i" -lt $((i + n)) ] && [ "$n" -gt 0 ]; do
    echo "{\"order_id\":\"$i\",\"pais\":\"ES\",\"total\":\"$i.00\"}"
    i=$((i + 1)); n=$((n - 1))
  done
}
filas 1000 1 > "$D/datos/pedidos.jsonl"

cat > "$D/ontology.config.yaml" <<X
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: r6, version: 0.1.0 }
datasources:
  - { name: ficheros, type: jsonl, connectionEnv: FICHEROS_DIR }
  - { name: lago, type: jsonl, connectionEnv: FICHEROS_DIR }
X
cat > "$D/package.yaml" <<'X'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 1.0.0, status: active, domain: sales }
spec: { owner: team:data }
X
cat > "$D/lattice.yaml" <<'X'
apiVersion: oos.dev/v1alpha3
kind: Lattice
metadata: { name: sensitivity, namespace: gdpr }
spec:
  levels: [none, low, high]
X
cat > "$D/conduits.yaml" <<'X'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: ventas }
spec:
  owner: team:security
  conduits:
    materialization.payload:
      gdpr.sensitivity: low
X
cat > "$D/tables/pedidos.yaml" <<'X'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: pedidos, namespace: bus }
spec:
  datasource: ficheros
  object: "pedidos.jsonl"
  columns: { order_id: {}, pais: {}, total: {} }
  reads: { fullScan: cheap }
  changes: { mode: upsert, key: [order_id], witness: snapshot, retention: 7d }
X
cat > "$D/views/copia.yaml" <<'X'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: copia, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: bus.pedidos }
  fields: { id: order_id, pais: pais, total: total }
  materialized: { datasource: lago, table: "cache.pedidos" }
X
export FICHEROS_DIR="$D/datos"

objetos() {
  python - <<'PY'
import os, boto3
c = boto3.client("s3", endpoint_url=os.environ["ORE_R2_S3_ENDPOINT"],
                 aws_access_key_id=os.environ["ORE_R2_ACCESS_KEY_ID"],
                 aws_secret_access_key=os.environ["ORE_R2_SECRET_ACCESS_KEY"],
                 region_name=os.environ.get("ORE_R2_REGION", "auto"))
print(c.list_objects_v2(Bucket=os.environ["ORE_R2_BUCKET"]).get("KeyCount", 0))
PY
}
leidas() { sed -n 's/.*· \([0-9]*\) leidas ·.*/\1/p' <<<"$1" | head -1; }
# Cuántas filas quedan **en la copia**. Sin esta, la prueba medía el trabajo y
# no la corrección: un refresco que leyera 10 filas y sellara una copia de 10
# pasaría, y esa copia estaría mal. **Trabajo proporcional al cambio** y **copia
# entera** son dos cosas, y hacen falta las dos.
copiadas() { sed -n 's/.*  \([0-9]*\) filas ·.*/\1/p' <<<"$1" | head -1; }

echo
echo "══ los cinco actos · el trabajo se cuenta en filas, no en segundos ══"
antes=$(objetos)

a1=$("$ORE" materialize "$D" 2>&1)
cmp_n 1000 "$(leidas "$a1")" "① primera materialización, filas leídas" "I5 (hecho)"
cmp_n 1000 "$(copiadas "$a1")" "① filas EN LA COPIA" "I5 (hecho)"
n1=$(objetos); cmp_n 2 "$((n1 - antes))" "① objetos nuevos (artefacto + recibo)" "I5 (hecho)"

a2=$("$ORE" materialize "$D" 2>&1)
if grep -q "ya está" <<<"$a2"; then ok "② sin tocar el origen: 0 filas leídas"
else mal "② releyó el origen sin que cambiara" "el recibo · I5"; fi
n2=$(objetos); cmp_n 0 "$((n2 - n1))" "② objetos nuevos" "el recibo · I5"

filas 10 1001 >> "$D/datos/pedidos.jsonl"
a3=$("$ORE" materialize "$D" 2>&1)
cmp_n 10 "$(leidas "$a3")" "③ +10 filas: leídas" "R2 y R3"
cmp_n 1010 "$(copiadas "$a3")" "③ filas EN LA COPIA" "la copia entera, no solo el incremento"
n3=$(objetos); cmp_n 2 "$((n3 - n2))" "③ objetos nuevos" "R2"

sed -i '1,3s/"pais":"ES"/"pais":"PT"/' "$D/datos/pedidos.jsonl"
a4=$("$ORE" materialize "$D" 2>&1)
cmp_n 3 "$(leidas "$a4")" "④ 3 filas modificadas: leídas" "R2 y R3"
cmp_n 1010 "$(copiadas "$a4")" "④ filas EN LA COPIA" "la copia entera, no solo el incremento"
n4=$(objetos); cmp_n 2 "$((n4 - n3))" "④ objetos nuevos" "R2"

"$ORE" materialize "$D" --recoger >/dev/null 2>&1
n5=$(objetos)
# **Dos**: el artefacto vigente y su recibo. Las tres copias anteriores
# siguen siendo ciertas hasta su marca, y por eso recoger es EXPLICITO — pero
# cuando se pide, el almacen queda acotado y no crece con los refrescos.
cmp_n 2 "$((n5 - antes))" "⑤ tras recoger, objetos que quedan" "R5"

echo
echo "══ las cuatro negativas · valen igual que los actos ══"

# FUERA de $D, y no es un detalle: creado dentro, `ore view "$D"` cargaba los
# dos paquetes a la vez y fallaba — y la negativa `d`, que mira su salida,
# pasaba EN FALSO por no encontrar el texto que buscaba.
N="$D-neg"; rm -rf "$N"; mkdir -p "$N"; cp -r "$D"/*.yaml "$D/tables" "$D/views" "$N/" 2>/dev/null
sed -i 's/mode: upsert, key: \[order_id\], witness: snapshot/mode: append, witness: field, field: total/' \
  "$N/tables/pedidos.yaml"
if "$ORE" validate "$N" >/dev/null 2>&1; then
  mal "a · {witness: field, mode: append} con \`materialized\` compila" "R0"
else
  ok "a · {witness: field, mode: append} no compila"
fi

# Sobre el VALOR y no sobre una palabra: lo que se afirma es que `7d` —lo que la
# tabla declara— salga por la boca de la herramienta. Como se llame la linea es
# cosa de quien la escriba.
if "$ORE" view "$D" 2>&1 | grep -qE "horizonte .*7d"; then
  ok "b · la retención declarada se mira"
else
  mal "b · \`changes.retention: 7d\` está declarada y nadie la nombra" "R1"
fi

# Estructural, y a propósito: la afirmación es sobre el PROTOCOLO, no sobre una
# ejecución. Mientras `Peticion` no tenga el campo, ningún driver puede negarse a
# un rango porque ningún rango le puede llegar.
#
# Y se llaman `start`/`end` y no `desde`/`hasta` porque la industria ya les puso
# nombre: Iceberg lee con `start-snapshot-id`, BigQuery con `start_timestamp`, y
# a la columna que ordena, medio sector la llama *cursor field*. La regla queda
# escrita en `ore-driver`: donde la industria tiene un nombre, se usa el suyo.
if grep -q "pub start" crates/ore-driver/src/lib.rs 2>/dev/null; then
  ok "c · la petición sabe llevar un rango"
else
  mal "c · la petición no tiene \`start\`/\`end\`, así que nadie puede negarse a un rango" "R3"
fi

# Y este mira `materialize --seco` y **no** `ore view`, que es donde estaba antes.
#
# Dos versiones fallaron aquí. La primera buscaba «testigo   sin poblar» y no lo
# encontraba porque I3 puso la marca delante. La segunda lo encontraba siempre, y
# por un motivo de fondo: **`ore view` es el compilador, y el compilador es
# hermético.** No abre nada, así que no puede preguntarle al origen dónde está —
# y su línea del registro dice «sin poblar» aunque el ciclo sí sepa fecharse.
#
# El valor del testigo existe donde se puede existir: en el paso ③ del ciclo, que
# sí ejecuta el driver. Preguntárselo a `ore view` era pedirle a la pieza
# hermética que contestara lo único que exige abrir una conexión.
if "$ORE" materialize "$D" --seco 2>&1 | grep -qE "testigo sin poblar"; then
  mal "d · el testigo no lleva valor: un origen que retrocede no se puede detectar" "R2"
else
  ok "d · el testigo lleva valor"
fi

echo
echo "══ limpieza ══"
python - <<'PY'
import os, boto3
c = boto3.client("s3", endpoint_url=os.environ["ORE_R2_S3_ENDPOINT"],
                 aws_access_key_id=os.environ["ORE_R2_ACCESS_KEY_ID"],
                 aws_secret_access_key=os.environ["ORE_R2_SECRET_ACCESS_KEY"],
                 region_name=os.environ.get("ORE_R2_REGION", "auto"))
B = os.environ["ORE_R2_BUCKET"]
ll = [{"Key": o["Key"]} for o in c.list_objects_v2(Bucket=B).get("Contents", [])]
if ll:
    c.delete_objects(Bucket=B, Delete={"Objects": ll})
print(f"  borrados {len(ll)} · el bucket queda con {c.list_objects_v2(Bucket=B).get('KeyCount', 0)}")
PY
rm -rf "$D" "$D-neg"

echo
if [ "$fallos" -eq 0 ]; then
  echo "listo · las tres invariantes se sostienen"
  echo "  ① sin cambio, cero trabajo   ② con cambio, trabajo proporcional"
  echo "  ③ almacén acotado             ④ y la copia, entera"
else
  echo "$fallos afirmacion(es) sin cumplir — y eso es la lista de trabajo de handoff-refresco.md"
fi
exit "$fallos"
