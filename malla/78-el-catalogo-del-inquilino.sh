#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# EL CATÁLOGO DEL INQUILINO · darlo de alta en el catálogo transaccional (ADR 0079 §17.8, E1a·4c·2)
#
# En el Postgres compartido (76), cada inquilino tiene lo suyo y nada más:
#
#   el papel     t_<org>, que entra con su clave (de /dev/urandom, sin imprimirse) y sólo a la
#                base `catalogo`;
#   el esquema   t_<org>, del papel: ahí crea `ore-catalogo` sus tablas (la versión del esquema
#                la aplica el binario al conectar). Nadie más tiene USAGE: un papel no ve el
#                esquema de otro;
#   el evento    la publicación eventos_t_<org> FOR TABLES IN SCHEMA, sólo inserciones: lo que el
#                inquilino cree en su esquema sale por la decodificación lógica, y el consumidor se
#                queda con `outbox`. La crea el alta: crear una publicación no es cosa del papel
#                de un inquilino (y una de esquema la crea un superusuario);
#   la llave     el Secret `catalogo` (clave `url`) en el namespace de la celda, t-<org>: lo lee
#                quien escribe en el catálogo desde la celda.
#
# ⭐ EN SECO POR DEFECTO. `APLICAR=1` lo hace. Idempotente: lo que ya existe no se toca (un papel
#   que existe conserva su clave; para rotarla, aparte).
#   bash malla/78-el-catalogo-del-inquilino.sh victor
#   APLICAR=1 bash malla/78-el-catalogo-del-inquilino.sh victor
#   NS_SECRETO=catalogo NOMBRE_SECRETO=catalogo-prueba1 APLICAR=1 bash malla/78-…sh prueba1   # pruebas
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail
ORG=${1:?uso: $0 <org>}
case "$ORG" in *[!a-z0-9]*|'') echo "⛔ la organización va en minúsculas y cifras: \`$ORG\`"; exit 64 ;; esac
PAPEL="t_$ORG"
NS_SECRETO=${NS_SECRETO:-t-$ORG}
NOMBRE_SECRETO=${NOMBRE_SECRETO:-catalogo}
APLICAR=${APLICAR:-}
# La primaria del `Cluster catalogo` (0079 ·5f), como su superusuario por el socket local.
PRIMARIA=$(kubectl -n catalogo get pod -l cnpg.io/cluster=catalogo,cnpg.io/instanceRole=primary -o jsonpath='{.items[0].metadata.name}')
[ -n "$PRIMARIA" ] || { echo "⛔ no hay primaria del Cluster catalogo"; exit 1; }
SQL() { kubectl -n catalogo exec -i "$PRIMARIA" -c postgres -- psql -X -q -t -A -v ON_ERROR_STOP=1 -U postgres -d catalogo; }

kubectl get ns "$NS_SECRETO" >/dev/null 2>&1 || { echo "⛔ no existe el namespace $NS_SECRETO"; exit 1; }
echo "── $ORG: papel y esquema $PAPEL, publicación eventos_$PAPEL, Secret $NOMBRE_SECRETO en $NS_SECRETO"

hay_papel=$(echo "SELECT count(*) FROM pg_roles WHERE rolname = '$PAPEL';" | SQL)
hay_secreto=$(kubectl -n "$NS_SECRETO" get secret "$NOMBRE_SECRETO" >/dev/null 2>&1 && echo 1 || echo 0)
if [ "$hay_papel" = 1 ] && [ "$hay_secreto" = 0 ]; then
  echo "⛔ el papel $PAPEL existe pero falta su Secret en $NS_SECRETO: su clave no está en ningún"
  echo "   sitio. Rotarla es aparte (ALTER ROLE … PASSWORD y el Secret), no se hace a ciegas aquí."
  exit 1
fi

if [ "$hay_papel" = 0 ]; then
  if [ -z "$APLICAR" ]; then
    echo "   (en seco) CREATE ROLE $PAPEL LOGIN … y el Secret $NOMBRE_SECRETO en $NS_SECRETO"
  else
    CLAVE=$(head -c 30 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)
    # Por la entrada estándar y por un fichero: la clave no pasa por la línea de órdenes.
    echo "CREATE ROLE $PAPEL LOGIN PASSWORD '$CLAVE';" | SQL
    F=$(mktemp)
    printf 'postgres://%s:%s@catalogo-db.catalogo.svc.cluster.local:5432/catalogo' "$PAPEL" "$CLAVE" > "$F"
    kubectl -n "$NS_SECRETO" create secret generic "$NOMBRE_SECRETO" --from-file=url="$F" >/dev/null
    rm -f "$F"; unset CLAVE
    echo "   papel y Secret creados"
  fi
else
  echo "   el papel ya existe (con su Secret): no se toca"
fi

LO_DEMAS="
REVOKE CONNECT ON DATABASE catalogo FROM PUBLIC;
GRANT CONNECT ON DATABASE catalogo TO $PAPEL;
CREATE SCHEMA IF NOT EXISTS $PAPEL AUTHORIZATION $PAPEL;
DO \$\$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_publication WHERE pubname = 'eventos_$PAPEL') THEN
    CREATE PUBLICATION eventos_$PAPEL FOR TABLES IN SCHEMA $PAPEL WITH (publish = 'insert');
  END IF;
END \$\$;"
if [ -z "$APLICAR" ]; then
  echo "$LO_DEMAS" | sed 's/^/   (en seco) /'
  echo "── hecho en seco (APLICAR=1 para hacerlo)"
else
  echo "$LO_DEMAS" | SQL
  echo "   esquema y publicación: $(echo "SELECT (SELECT nspowner::regrole FROM pg_namespace WHERE nspname = '$PAPEL') || ' · ' || (SELECT count(*) FROM pg_publication WHERE pubname = 'eventos_$PAPEL') || ' publicación';" | SQL)"
  echo "── hecho de verdad"
fi
