#!/usr/bin/env bash
# El runner de migraciones de `iam`.
#
#     PGHOST=… PGUSER=… PGPASSWORD=… PGDATABASE=iam  bash iam/migrar.sh
#
# ── ⭐⭐ LA REGLA QUE SE COPIA ENTERA ────────────────────────────────────────
#
# De `modelo/migraciones/006-dueno.sql`, y es la única parte de su runner que
# merecía copiarse:
#
#   > **una migración aplicada es INMUTABLE**, y el runner lo exige: si el
#   > contenido de una ya corrida cambia, se lanza — *«lo que corrió y lo que
#   > dice el fichero dejan de ser lo mismo»*.
#
# Sin esto, corregir una migración en su sitio deja un historial que MIENTE: la
# base tiene una forma y el repositorio cuenta otra, y nadie se entera hasta que
# alguien levanta una base nueva y sale distinta.
#
# ⇒ Corregir no es editar: es escribir la siguiente. Cuesta un fichero.
#
# ── Y las otras dos propiedades ─────────────────────────────────────────────
#
#   idempotente     lo ya corrido se salta. Volver a lanzarlo no hace nada
#   una transacción por migración  si una falla, esa no queda a medias — y las
#                   anteriores siguen aplicadas, que es lo correcto: el
#                   historial refleja lo que hay
set -eu

# De donde salen los ficheros. Se puede decir por el entorno porque en el
# cluster llegan de un ConfigMap, y un ConfigMap **no tiene subdirectorios**:
# todas sus claves caen planas en el punto de montaje.
AQUI="$(cd "$(dirname "$0")" && pwd)"
MIG="${IAM_MIGRACIONES:-$AQUI/migraciones}"

: "${PGDATABASE:=iam}"
export PGDATABASE

# Una funcion y no un array: esto corre con el `sh` de Alpine dentro del
# cluster, y los arrays son de bash. Un script que solo funciona en el portatil
# no es el mismo script que se despliega.
psql_() { psql --quiet --no-psqlrc -v ON_ERROR_STOP=1 "$@"; }

# El libro tiene que existir antes de poder consultarlo, y su propia migración
# es la que lo crea. Se aplica siempre —es `if not exists`— y así el arranque en
# frío no es un caso especial que haya que recordar.
psql_ -f "$MIG/001-el-libro.sql" > /dev/null

aplicadas=$(psql_ -At -c "select nombre || ' ' || huella from iam.migracion" || true)

huella_de() { sha256sum "$1" | cut -d' ' -f1; }

nuevas=0
for f in "$MIG"/*.sql; do
  n="$(basename "$f")"
  h="$(huella_de "$f")"
  antes="$(printf '%s\n' "$aplicadas" | awk -v n="$n" '$1==n {print $2}')"

  if [ -n "$antes" ]; then
    if [ "$antes" != "$h" ]; then
      echo "✗ \`$n\` ya se aplicó y su contenido HA CAMBIADO." >&2
      echo "  lo que corrió: $antes" >&2
      echo "  lo que dice el fichero: $h" >&2
      echo "  ⇒ una migración aplicada es inmutable. Corregir es escribir la siguiente." >&2
      exit 1
    fi
    continue
  fi

  echo "· $n"
  psql_ --single-transaction -f "$f" > /dev/null
  psql_ -c "insert into iam.migracion (nombre, huella) values ('$n', '$h')" > /dev/null
  nuevas=$((nuevas + 1))
done

echo
if [ "$nuevas" -eq 0 ]; then
  echo "ok · nada que aplicar"
else
  echo "ok · $nuevas migraciones aplicadas"
fi
psql_ -c "select count(*) as tablas from information_schema.tables where table_schema='iam' and table_type='BASE TABLE'"
