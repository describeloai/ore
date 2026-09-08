#!/usr/bin/env bash
# LAS MIGRACIONES, CONTRA UNA BASE CON DATOS.
#
# ── ⛔⛔ Por qué esto existe, y costó dos vueltas ───────────────────────────
#
# `los-verbos.sh` corre las migraciones en cada vuelta. Todas. Y aun así dejó
# pasar DOS fallos de orden dentro de la misma migración:
#
#   ① un `update … set rol = null` sobre una columna cuyo `not null` no se
#      quitaba hasta la migración SIGUIENTE
#   ② un `update … set rol = 'ORGADMIN'` ANTES del `insert` que crea ese rol
#
# Los dos revientan contra una base con filas. Ninguno se ve contra una vacía,
# porque un `update` que no toca nada no puede violar nada.
#
#   ⇒ **Una suite de migraciones que sólo corre contra una base vacía no prueba
#     migraciones: prueba sintaxis.**
#
# Los encontró el clúster, de uno en uno, a base de aplicar y fallar. Esto los
# habría encontrado en noventa segundos.
#
# ── Cómo ───────────────────────────────────────────────────────────────────
#
#   1. base limpia
#   2. se aplican las migraciones HASTA un corte
#   3. se siembra `pruebas-de-fuego/semilla/<corte>.sql` — datos con la forma
#      que tiene el esquema EN ESE PUNTO
#   4. se aplican las que faltan, y tienen que pasar
#
# ⭐ La semilla va por corte y no es genérica a propósito: el esquema cambia de
#   forma —`papel` se llamó así hasta la `012`— así que un fichero que valiera
#   para todos los puntos no valdría de verdad para ninguno. Añadir un corte es
#   añadir un `.sql`, que es una decisión con fecha, como las migraciones.
#
#   PG_URL=postgres://postgres:x@localhost:5432 \
#   PGHOST=localhost PGUSER=postgres PGPASSWORD=x \
#     bash pruebas-de-fuego/las-migraciones.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PG_URL="${PG_URL:-postgres://postgres:x@localhost:5432}"
SEMILLAS="$RAIZ/pruebas-de-fuego/semilla"
TMP="$(mktemp -d)"

falla() { echo "✗ $*" >&2; rm -rf "$TMP"; exit 1; }
dice()  { echo "  · $*"; }
trap 'rm -rf "$TMP"' EXIT

command -v psql >/dev/null || falla "hace falta psql"
[ -d "$SEMILLAS" ] || falla "no hay semillas en $SEMILLAS"

# Cada semilla se llama como la migración HASTA la que se aplica.
for SEMILLA in "$SEMILLAS"/*.sql; do
  CORTE="$(basename "$SEMILLA" .sql)"
  BASE="iam_mig_${CORTE//-/_}"
  BASE="${BASE:0:40}"

  psql "$PG_URL/postgres" -qtAc "drop database if exists $BASE" >/dev/null 2>&1
  psql "$PG_URL/postgres" -qtAc "create database $BASE" >/dev/null 2>&1 \
    || falla "no se pudo crear $BASE"

  # ── hasta el corte ───────────────────────────────────────────────────────
  # ⭐ Se copian las de antes a un directorio aparte y se apunta ahí. Es lo que
  #   `IAM_MIGRACIONES` existe para permitir —lo escribió el runner porque un
  #   ConfigMap no tiene subdirectorios— y aquí sirve para lo mismo: decir
  #   «aplica sólo hasta aquí» sin tocar el árbol.
  mkdir -p "$TMP/$CORTE"
  for M in "$RAIZ/iam/migraciones"/*.sql; do
    N="$(basename "$M")"
    cp "$M" "$TMP/$CORTE/$N"
    [ "${N%%-*}" = "$CORTE" ] && break
  done
  CUANTAS=$(ls "$TMP/$CORTE" | wc -l)

  PGDATABASE="$BASE" IAM_MIGRACIONES="$TMP/$CORTE" \
    bash "$RAIZ/iam/migrar.sh" > "$TMP/$CORTE.hasta" 2>&1 \
    || falla "$CORTE · las primeras $CUANTAS no aplican: $(tail -5 "$TMP/$CORTE.hasta")"

  # ── la semilla ───────────────────────────────────────────────────────────
  psql "$PG_URL/$BASE" -v ON_ERROR_STOP=1 -q -f "$SEMILLA" > "$TMP/$CORTE.semilla" 2>&1 \
    || falla "$CORTE · la semilla no entra —¿cambió el esquema en ese punto?—: $(tail -5 "$TMP/$CORTE.semilla")"

  # ── y el resto, que es lo que se está probando ───────────────────────────
  PGDATABASE="$BASE" bash "$RAIZ/iam/migrar.sh" > "$TMP/$CORTE.resto" 2>&1 \
    || falla "$CORTE · ⛔ LAS MIGRACIONES DE DESPUES DE LA $CORTE NO SOBREVIVEN A UNA BASE CON DATOS:
$(tail -8 "$TMP/$CORTE.resto")"

  APLICADAS=$(grep -c '^·' "$TMP/$CORTE.resto" || echo 0)
  dice "corte $CORTE · $CUANTAS + semilla + $APLICADAS mas, sobre datos"
  psql "$PG_URL/postgres" -qtAc "drop database $BASE" >/dev/null 2>&1
done

echo "✓ las migraciones aguantan una base que ya tenia filas."
