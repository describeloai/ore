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
# Y contra el almacén de la celda (GCS con el token de la cuenta, sin clave;
# `ore-store-gcs`, 2026-09-17), los mismos números:
#
#   ORE_STORE=gcs ORE_GCS_BUCKET=<bucket> ORE_GCS_TOKEN=$(gcloud auth print-access-token) \
#   PATH="$PWD/target/debug:$PATH" bash pruebas-de-fuego/refresco.sh
#
# **Esta prueba nace en rojo, y eso es el diseño.** Cada ✗ nombra el peldaño que
# lo cierra, así que su salida ES la lista de trabajo del plan.
#
# Desde W3.6a (0031 §10) la copia es un DATASET: una tabla Iceberg en el bucket
# y un puntero en el árbol (`copias/<p>_<v>.json`). Los objetos que se cuentan
# son los de la tabla —`metadata.json`, lista de manifiestos, manifiestos,
# ficheros de datos— y el bucket queda acotado por `--recoger`, que expira los
# snapshots superados y retira lo que ningún snapshot nombra. `ORE_RECOGER_EDAD`
# (segundos) conserva la historia reciente; aquí no se pone: se expira todo lo
# superado, que es lo que «recoger» significó siempre.
set -u

ORE="${ORE:-./target/debug/ore.exe}"
[ -x "$ORE" ] || ORE="./target/debug/ore"

ALMACEN="${ORE_STORE:-r2}"
case "$ALMACEN" in
  r2)  NECESITA="ORE_R2_S3_ENDPOINT ORE_R2_BUCKET ORE_R2_ACCESS_KEY_ID ORE_R2_SECRET_ACCESS_KEY" ;;
  gcs) NECESITA="ORE_GCS_BUCKET ORE_GCS_TOKEN" ;;
  *)   echo "ORE_STORE=$ALMACEN no es un almacén: r2 o gcs"; exit 2 ;;
esac
for v in $NECESITA; do
  if [ -z "${!v:-}" ]; then
    echo "falta \$$v — carga el entorno primero:  set -a; . ./.env.local; set +a"
    exit 2
  fi
done
command -v "ore-store-$ALMACEN" >/dev/null || { echo "pon target/debug en el PATH (falta ore-store-$ALMACEN)"; exit 2; }
echo "almacén: $ALMACEN"

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
    printf '{"order_id":"%s","pais":"ES","total":"%s.00","actualizado_en":"%010d"}
' "$i" "$i" "$i"
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
metadata: { name: pedidos, namespace: ventas }
spec:
  datasource: ficheros
  object: "pedidos.jsonl"
  columns: { order_id: {}, pais: {}, total: {}, actualizado_en: {} }
  reads: { fullScan: cheap }
  changes: { mode: upsert, key: [order_id], witness: field, field: actualizado_en, retention: 7d }
X
cat > "$D/views/copia.yaml" <<'X'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: copia, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: ventas.pedidos }
  fields: { id: order_id, pais: pais, total: total, cuando: actualizado_en }
  materialized: { datasource: lago, table: "cache.pedidos" }
X
export FICHEROS_DIR="$D/datos"

# Cuántos objetos hay en el bucket, por fuera del delegado: la prueba no se fía
# de que el almacén cuente lo suyo. En R2 con boto3; en GCS con `gcloud`, que en
# local ya tiene la cuenta.
GCLOUD=$(command -v gcloud.cmd || command -v gcloud || true)
objetos() {
  if [ "$ALMACEN" = "gcs" ]; then
    "$GCLOUD" storage ls "gs://$ORE_GCS_BUCKET/**" 2>/dev/null | grep -c . || true
    return
  fi
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

INF="$D/informes"
export -n ORE_RECOGER_EDAD 2>/dev/null; unset ORE_RECOGER_EDAD
a1=$("$ORE" materialize "$D" --informe "$INF" 2>&1)
cmp_n 1000 "$(leidas "$a1")" "① primera materialización, filas leídas" "I5 (hecho)"
cmp_n 1000 "$(copiadas "$a1")" "① filas EN LA COPIA" "I5 (hecho)"
# el informe (P1 I3): lo que quien no alcanza el almacen sabe de la copia
cmp_n copiada "$(sed -n 's/.*"estado": *"\([a-z-]*\)".*/\1/p' "$INF/ventas_copia.json" | head -1)" "① el informe dice el estado" "P1 I3"
cmp_n 1000 "$(sed -n 's/.*"filas": *\([0-9]*\).*/\1/p' "$INF/ventas_copia.json" | head -1)" "① el informe cuenta las filas" "P1 I3"
# y por columna (medida W1 §B): una copia con las filas y sin los valores era `copiada` igual
cmp_n 1000 "$(tr -d ' \n' < "$INF/ventas_copia.json" | sed -n 's/.*"columnas":{[^}]*"total":\([0-9]*\).*/\1/p' | head -1)" "① el informe cuenta cada columna" "W1 §B"
cmp_n creada "$(sed -n 's/.*"operacion": *"\([a-z-]*\)".*/\1/p' "$INF/ventas_copia.json" | head -1)" "① el puntero dice que el dataset nace" "W3.6a"
n1=$(objetos); cmp_n 4 "$((n1 - antes))" "① objetos nuevos (metadata.json + lista + manifiesto + datos)" "W3.6a"

a2=$("$ORE" materialize "$D" --informe "$INF" 2>&1)
if grep -q "ya está" <<<"$a2"; then ok "② sin tocar el origen: 0 filas leídas"
else mal "② releyó el origen sin que cambiara" "el puntero · I5"; fi
n2=$(objetos); cmp_n 0 "$((n2 - n1))" "② objetos nuevos" "el puntero · I5"
cmp_n al-dia "$(sed -n 's/.*"estado": *"\([a-z-]*\)".*/\1/p' "$INF/ventas_copia.json" | head -1)" "② el informe dice al-dia" "P1 I3"
cmp_n 1000 "$(sed -n 's/.*"filas": *\([0-9]*\).*/\1/p' "$INF/ventas_copia.json" | head -1)" "② y conserva las filas de la copia que ya estaba" "P1 I3"

filas 10 1001 >> "$D/datos/pedidos.jsonl"
a3=$("$ORE" materialize "$D" --informe "$INF" 2>&1)
cmp_n 10 "$(leidas "$a3")" "③ +10 filas: leídas" "R2 y R3"
cmp_n 1010 "$(copiadas "$a3")" "③ filas EN LA COPIA" "la copia entera, no solo el incremento"
cmp_n refrescada "$(sed -n 's/.*"operacion": *"\([a-z-]*\)".*/\1/p' "$INF/ventas_copia.json" | head -1)" "③ el puntero dice refrescada (fundida sobre lo que había)" "W3.6a"
# un snapshot que sobrescribe: metadata.json + lista + 2 manifiestos (los
# ficheros nuevos, y los del snapshot anterior como retirados) + datos
n3=$(objetos); cmp_n 5 "$((n3 - n2))" "③ objetos nuevos (un snapshot más)" "R2"

sed -i '1,3s/"pais":"ES"/"pais":"PT"/; 1,3s/"actualizado_en":"[0-9]*"/"actualizado_en":"0000002000"/' "$D/datos/pedidos.jsonl"
a4=$("$ORE" materialize "$D" --informe "$INF" 2>&1)
cmp_n 3 "$(leidas "$a4")" "④ 3 filas modificadas: leídas" "R2 y R3"
cmp_n 1010 "$(copiadas "$a4")" "④ filas EN LA COPIA" "la copia entera, no solo el incremento"
n4=$(objetos); cmp_n 5 "$((n4 - n3))" "④ objetos nuevos (un snapshot más)" "R2"
cmp_n 1010 "$(sed -n 's/.*"filas": *\([0-9]*\).*/\1/p' "$INF/ventas_copia.json" | head -1)" "④ el informe cuenta la copia entera" "P1 I3"

a5=$("$ORE" materialize "$D" --recoger --informe "$INF" 2>&1)
n5=$(objetos)
# **Ocho**: el snapshot vigente (lista + 2 manifiestos + datos) y los cuatro
# `metadata.json` que el registro de metadatos de la tabla sigue listando (los
# tres de los actos y el de expirar). Los dos snapshots anteriores siguen
# siendo ciertos hasta su marca, y por eso recoger es EXPLICITO — pero cuando
# se pide, el almacen queda acotado y no crece con los refrescos.
cmp_n 8 "$((n5 - antes))" "⑤ tras recoger, objetos que quedan" "R5"
if grep -q "recogidos 2 snapshot(s) superado(s)" <<<"$a5"; then ok "⑤ expiraron los 2 snapshots superados"
else mal "⑤ no expiró los 2 snapshots superados: $a5" "W3.6a"; fi
ML5=$(sed -n 's/.*"metadata_location": *"\([^"]*\)".*/\1/p' "$INF/ventas_copia.json" | head -1)
case "$ML5" in */metadata/00003-*) ok "⑤ el puntero se movió al metadata.json de expirar" ;; *) mal "⑤ el puntero no se movió al expirar: $ML5" "W3.6a" ;; esac
a6=$("$ORE" materialize "$D" --informe "$INF" 2>&1)
if grep -q "ya está" <<<"$a6"; then ok "⑥ tras recoger, sigue al día: 0 filas leídas"
else mal "⑥ recoger dejó la copia sin puntero válido" "W3.6a"; fi

echo
echo "══ las cuatro negativas · valen igual que los actos ══"

# FUERA de $D, y no es un detalle: creado dentro, `ore view "$D"` cargaba los
# dos paquetes a la vez y fallaba — y la negativa `d`, que mira su salida,
# pasaba EN FALSO por no encontrar el texto que buscaba.
N="$D-neg"; rm -rf "$N"; mkdir -p "$N"; cp -r "$D"/*.yaml "$D/tables" "$D/views" "$N/" 2>/dev/null
sed -i 's/mode: upsert, key: \[order_id\], witness: field/mode: append, witness: field/' \
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
if [ "$ALMACEN" = "gcs" ]; then
  N=$(objetos)
  [ "$N" -gt 0 ] && "$GCLOUD" storage rm "gs://$ORE_GCS_BUCKET/**" >/dev/null 2>&1
  echo "  borrados $N · el bucket queda con $(objetos)"
else
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
fi
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
