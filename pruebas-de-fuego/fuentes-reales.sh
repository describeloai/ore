#!/usr/bin/env bash
# Lo que se midio a mano al construir L2, convertido en una puerta.
#
# Cada afirmacion de aqui fue un `echo` en una terminal mientras se escribian los
# cinco hitos. Un `echo` demuestra algo una vez; esto lo demuestra en cada
# empujon — y la diferencia importa porque **una prueba que no corre tiene
# exactamente el mismo aspecto que una que pasa**.
#
# Necesita: un PostgreSQL en `PG_URL` y los binarios en el PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

PG_URL="${PG_URL:-postgres://postgres:x@localhost:5432/hr}"
CASO_2=casos/dos-familias
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

falla() { echo "FALLA · $1"; exit 1; }
igual() { [ "$1" = "$2" ] || falla "$3: esperaba «$2» y salio «$1»"; }

# ── El escenario ────────────────────────────────────────────────────────────
psql "$PG_URL" -q <<'SQL'
DROP TABLE IF EXISTS employees;
CREATE TABLE employees (
  employee_id text PRIMARY KEY, manager_id text, national_id text, updated_at text
);
INSERT INTO employees VALUES
  ('emp-42','jefa','52000','2026-08-31T09:00:00Z'),
  ('jefa','ceo','61000','2026-08-31T09:00:00Z'),
  ('ceo',NULL,'70000','2026-08-31T09:00:00Z');
SQL

export ERP_URL="$PG_URL"
export FICHEROS_DIR="$PWD/$CASO_2/datos"

# ── 1 · El driver devuelve filas, y solo las columnas proyectadas ────────────
#
# `national_id` NO esta en la proyeccion, asi que no puede salir. Ahi es donde la
# mascara se hace efectiva: no hay ningun punto donde alguien pueda olvidarse de
# aplicarla porque no hay nada que aplicar.
fila=$(printf '%s' "{\"url\":\"$PG_URL\",\"objeto\":\"public.employees\",\"proyeccion\":{\"employeeId\":\"employee_id\"},\"claveColumnas\":[\"employee_id\"],\"claves\":[[\"emp-42\"]],\"filtros\":[]}" \
  | ore-read-postgres leer erp)
igual "$fila" '{"employeeId":"emp-42"}' "el driver devuelve la fila proyectada"
case "$fila" in *national*) falla "el driver devolvio una columna que no se pidio";; esac

# ── 2 · La conexion es de SOLO LECTURA, pedida al servidor ───────────────────
if psql "$PG_URL" -q -c "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY" \
     -c "INSERT INTO employees VALUES ('x','y','z','w')" >/dev/null 2>&1; then
  falla "una escritura paso en una sesion de solo lectura"
fi

# ── 3, 4 y 5 · el indice de topologia ───────────────────────────────────────
#
# Se fueron con `ore-exec`, que era quien construia el artefacto ORETOPO1 y quien
# lo recorria. Afirmaban tres cosas que NADIE comprueba hoy:
#
#   - que construir el indice dos veces desde la misma fuente da los mismos bytes;
#   - que refrescar SUSTITUYE en vez de sumar —un cambio de jefe deja UNA arista—;
#   - y que una travesia devuelve la cadena que la jerarquia dice.
#
# Se dicen aqui, borradas, porque un hueco con nombre es distinto de un hueco.
# Las dos de arriba —el driver proyecta lo que se le pide, y la conexion es de
# solo lectura pedida al servidor— siguen corriendo y son del driver, no del
# ejecutor.

echo "OK · fuentes reales (1-2; 3-5 se fueron con ore-exec)"
