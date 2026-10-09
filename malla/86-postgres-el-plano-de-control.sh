#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# LA BASE DEL PLANO DE CONTROL DE POSTGRES (ADR 0058, P4·1) — una vez, a mano
#
#   base     `ore_postgres`, en el mismo Postgres que el storage_controller
#            (`storcon-db`, 81): una base más, con la misma copia diaria
#   papel    `ore_postgres`, dueño de su base y de nada más (sin superusuario)
#   secreto  `ore-postgres-db` en ore-pg: usuario, clave y url. La clave sale de
#            /dev/urandom directa al Secret y a psql por la entrada estándar:
#            no se imprime, no va en ningún argumento y no está en el repositorio
#
# Idempotente: si el Secret ya existe no se toca (rotar es otra operación) y el
# papel y la base sólo se crean si faltan. Las tablas las crea `ore-postgres` al
# arrancar (sus migraciones van dentro del binario).
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
NS=ore-pg
POD=storcon-db-0

psql_su() { kubectl -n $NS exec -i $POD -- sh -c 'psql -v ON_ERROR_STOP=1 -qAt -U "$POSTGRES_USER" -d postgres'; }

echo "── el Secret ore-postgres-db"
if kubectl -n $NS get secret ore-postgres-db >/dev/null 2>&1; then
  echo "   ya existe (no se toca: rotarla es otra operación)"
  CLAVE=""
else
  CLAVE=$(head -c 30 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)
  kubectl -n $NS create secret generic ore-postgres-db --from-literal=usuario=ore_postgres \
    --from-literal=clave="$CLAVE" \
    --from-literal=url="postgresql://ore_postgres:$CLAVE@storcon-db.$NS.svc.cluster.local:5432/ore_postgres" >/dev/null
  echo "   creado"
fi

echo "── el papel ore_postgres"
if [ "$(echo "select 1 from pg_roles where rolname = 'ore_postgres'" | psql_su)" = 1 ]; then
  echo "   ya existe"
  [ -z "$CLAVE" ] || { echo "✗ el Secret es nuevo pero el papel ya estaba: su clave no casa. Bórralo a mano y repite."; exit 1; }
else
  if [ -z "$CLAVE" ]; then
    CLAVE=$(kubectl -n $NS get secret ore-postgres-db -o jsonpath='{.data.clave}' | base64 -d)
  fi
  printf "create role ore_postgres login password '%s';\n" "$CLAVE" | psql_su
  echo "   creado"
fi
unset CLAVE

echo "── la base ore_postgres"
if [ "$(echo "select 1 from pg_database where datname = 'ore_postgres'" | psql_su)" = 1 ]; then
  echo "   ya existe"
else
  echo "create database ore_postgres owner ore_postgres;" | psql_su
  echo "   creada"
fi
# Nadie más se conecta a ella: ni `public`, ni el storage_controller.
echo "revoke all on database ore_postgres from public;" | psql_su

# ── P4·3·1 · la llave propia (Ed25519, PKCS#8) con que ore-postgres le habla a cada compute_ctl ──
# Se genera en memoria y va por la entrada estándar de kubectl: ni disco, ni argumentos, ni pantalla.
echo "── el Secret ore-postgres-llaves"
if kubectl -n $NS get secret ore-postgres-llaves >/dev/null 2>&1; then
  echo "   ya existe (no se toca: rotarla es otra operación)"
else
  python -c '
import json
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
pem = Ed25519PrivateKey.generate().private_bytes(serialization.Encoding.PEM,
      serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()
print(json.dumps({"apiVersion": "v1", "kind": "Secret",
  "metadata": {"name": "ore-postgres-llaves", "namespace": "ore-pg"},
  "stringData": {"privada.pem": pem}}))' | kubectl apply -f - >/dev/null
  echo "   creado"
fi

# ── P5·1 · el token del proxy: lo montan el proxy (lo manda) y ore-postgres (lo compara) ──
# Aleatorio, en memoria y por la entrada estándar de kubectl, como la llave de arriba.
echo "── el Secret ore-postgres-proxy"
if kubectl -n $NS get secret ore-postgres-proxy >/dev/null 2>&1; then
  echo "   ya existe (no se toca: rotarlo es otra operación)"
else
  python -c '
import json, secrets
print(json.dumps({"apiVersion": "v1", "kind": "Secret",
  "metadata": {"name": "ore-postgres-proxy", "namespace": "ore-pg"},
  "stringData": {"token": secrets.token_urlsafe(48)}}))' | kubectl apply -f - >/dev/null
  echo "   creado"
fi
echo "ok"
