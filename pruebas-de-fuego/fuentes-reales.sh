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
CASO_J=crates/ore-exec/casos/jerarquia
CASO_2=crates/ore-exec/casos/dos-familias
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

# ── 3 · El indice se construye desde la fuente, y es determinista ────────────
ore-exec index build "$CASO_J" --url-env ERP_URL --marca 2026-08-31T09:30:00Z -o "$T/a.oretopo" 2>/dev/null
ore-exec index build "$CASO_J" --url-env ERP_URL --marca 2026-08-31T09:30:00Z -o "$T/b.oretopo" 2>/dev/null
cmp -s "$T/a.oretopo" "$T/b.oretopo" || falla "dos construcciones sobre la misma instantanea difieren"

cadena=$(ore-exec index traverse "$CASO_J" --indice "$T/a.oretopo" \
  --relacion hr.Employee.manager --desde emp-42 --saltos 3 | sort | tr '\n' ' ')
igual "$cadena" "ceo jefa " "la travesia devuelve la cadena de mando"

# ── 4 · El refresco SUSTITUYE, no suma ──────────────────────────────────────
#
# Un cambio de jefe tiene que dejar UNA arista, no dos. Sumando, la cadena de
# mando tendria dos ramas y un cambio se pareceria a una ampliacion.
psql "$PG_URL" -q -c "UPDATE employees SET manager_id='ceo', updated_at='2026-08-31T11:00:00Z' WHERE employee_id='emp-42'"
ore-exec index refresh "$CASO_J" --anterior "$T/a.oretopo" --marca 2026-08-31T11:30:00Z -o "$T/c.oretopo" 2>/dev/null

cadena=$(ore-exec index traverse "$CASO_J" --indice "$T/c.oretopo" \
  --relacion hr.Employee.manager --desde emp-42 --saltos 3 | sort | tr '\n' ' ')
igual "$cadena" "ceo " "tras el cambio, la cadena de emp-42 es solo el ceo"

# Y un refresco que no avanza no es un refresco.
if ore-exec index refresh "$CASO_J" --anterior "$T/c.oretopo" \
     --marca 2026-08-31T10:00:00Z -o "$T/d.oretopo" >/dev/null 2>&1; then
  falla "acepto una marca anterior a la que ya tenia"
fi

# ── 5 · Una consulta que cruza DOS FAMILIAS de fuente ────────────────────────
#
# `baseSalary` de PostgreSQL y `alias` de un FICHERO, ensamblados por la clave.
# Si el mismo plan sirve a un servidor y a un fichero, la peticion estaba cortada
# por el sitio correcto.
salida=$(ore-exec responder "$CASO_2" --entidad hr.Employee \
  --props hr.Employee.employeeId,hr.Employee.baseSalary,hr.Employee.alias \
  --claves emp-42 --sujeto emp-42 --roles analyst --claims employeeId=emp-42 \
  --purpose compensation_review --emisor https://id.example --audiencia ore \
  --instante 2026-08-31T12:00:00Z 2>/dev/null)
igual "$salida" 'alias="la jefa de datos"  baseSalary="52000"  employeeId="emp-42"' \
  "una entidad ensamblada desde dos familias"

# ── 6 · Y el estado degradado se declara ────────────────────────────────────
ore-exec index build "$CASO_J" --url-env ERP_URL --marca 2026-08-31T09:00:00Z -o "$T/viejo.oretopo" 2>/dev/null
if ! ore-exec responder "$CASO_J" --indice "$T/viejo.oretopo" --entidad hr.Employee \
     --props hr.Employee.baseSalary --claves emp-42 --sujeto emp-42 --roles analyst \
     --claims employeeId=emp-42 --purpose compensation_review \
     --emisor https://id.example --audiencia ore \
     --instante 2026-08-31T12:00:00Z --sla 30m 2>&1 >/dev/null | grep -q DEGRADADO; then
  falla "no declaro el estado degradado con la marca de agua vencida"
fi

echo "seis escenarios contra fuentes reales, en verde"
