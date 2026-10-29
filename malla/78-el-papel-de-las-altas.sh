#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# EL PAPEL DE LAS ALTAS · quien da de alta a cada inquilino en el catálogo (ADR 0079 ·5g·1b)
#
# El alta de un inquilino (papel t_<org>, su esquema, su CONNECT) la hacía 78-el-catalogo-del-
# inquilino.sh a mano, como superusuario. Con el corte la hace el aprovisionador, y el aprovisionador
# no es superusuario de nada. Medido en el `Cluster catalogo` (·5g·0): basta un papel con
#
#   CREATEROLE                     crea t_<org>, y Postgres 16 le da ADMIN sobre él (puede
#                                  hacerse miembro con SET y nombrarlo dueño del esquema);
#   CONNECT, CREATE … WITH GRANT   crea el esquema y le da CONNECT al inquilino.
#
# No es superusuario, no tiene REPLICATION ni BYPASSRLS, y no es dueño de nada de los inquilinos:
# ⚠️ con CREATEROLE sí puede crear papeles y cambiar la clave de los que creó él. Es el poder del
#   alta, y por eso su URL sólo la lee la cuenta del aprovisionador.
#
# La URL va a Secret Manager (`catalogo-altas`), no a un `Secret`: el aprovisionador la trae a su
# tmpfs como sus otros testigos (16-el-aprovisionador.yaml).
#
# ⭐ EN SECO POR DEFECTO. `APLICAR=1` lo hace. Idempotente: si el papel existe, no se toca (rotar
#   su clave es aparte).
#   bash malla/78-el-papel-de-las-altas.sh
#   APLICAR=1 bash malla/78-el-papel-de-las-altas.sh
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
P=project-8853a180-450d-47be-b83
SECRETO=catalogo-altas
QUIEN=ore-aprovisionador@$P.iam.gserviceaccount.com
APLICAR=${APLICAR:-}

PRIMARIA=$(kubectl -n catalogo get pod -l cnpg.io/cluster=catalogo,cnpg.io/instanceRole=primary -o jsonpath='{.items[0].metadata.name}')
[ -n "$PRIMARIA" ] || { echo "⛔ no hay primaria del Cluster catalogo"; exit 1; }
SQL() { kubectl -n catalogo exec -i "$PRIMARIA" -c postgres -- psql -X -q -t -A -v ON_ERROR_STOP=1 -U postgres -d catalogo; }

hay_papel=$(echo "SELECT count(*) FROM pg_roles WHERE rolname = 'altas';" | SQL)
hay_secreto=$(gcloud secrets describe "$SECRETO" --project "$P" >/dev/null 2>&1 && echo 1 || echo 0)
echo "── el papel altas: $([ "$hay_papel" = 1 ] && echo existe || echo falta) · el secreto $SECRETO: $([ "$hay_secreto" = 1 ] && echo existe || echo falta)"

if [ "$hay_papel" = 1 ] && [ "$hay_secreto" = 0 ]; then
  echo "⛔ el papel existe pero su URL no está en Secret Manager: su clave no está en ningún sitio."
  echo "   Rotarla es aparte (ALTER ROLE altas PASSWORD … y una versión nueva del secreto)."
  exit 1
fi

if [ "$hay_papel" = 0 ]; then
  if [ -z "$APLICAR" ]; then
    echo "   (en seco) CREATE ROLE altas LOGIN CREATEROLE …, y su URL a Secret Manager $SECRETO"
  else
    CLAVE=$(head -c 30 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)
    # Por la entrada estándar y por un fichero: la clave no pasa por la línea de órdenes.
    echo "CREATE ROLE altas LOGIN CREATEROLE PASSWORD '$CLAVE';" | SQL
    F=$(mktemp)
    printf 'postgres://altas:%s@catalogo-db.catalogo.svc.cluster.local:5432/catalogo' "$CLAVE" > "$F"
    if [ "$hay_secreto" = 0 ]; then
      gcloud secrets create "$SECRETO" --project "$P" --replication-policy automatic --data-file "$F" >/dev/null
    else
      gcloud secrets versions add "$SECRETO" --project "$P" --data-file "$F" >/dev/null
    fi
    rm -f "$F"; unset CLAVE
    echo "   papel y secreto creados"
  fi
fi

LO_DEMAS="GRANT CONNECT, CREATE ON DATABASE catalogo TO altas WITH GRANT OPTION;"
if [ -z "$APLICAR" ]; then
  echo "   (en seco) $LO_DEMAS"
  echo "   (en seco) $SECRETO: secretAccessor para $QUIEN"
  echo "── hecho en seco (APLICAR=1 para hacerlo)"
else
  echo "$LO_DEMAS" | SQL
  gcloud secrets add-iam-policy-binding "$SECRETO" --project "$P" --condition=None \
    --role roles/secretmanager.secretAccessor --member "serviceAccount:$QUIEN" >/dev/null
  echo "   $(echo "SELECT 'altas: createrole=' || rolcreaterole || ' super=' || rolsuper || ' replication=' || rolreplication FROM pg_roles WHERE rolname = 'altas';" | SQL)"
  echo "── hecho de verdad"
fi
