#!/usr/bin/env bash
# El laboratorio de CockroachDB (ADR 0080 §4): un nodo con el usuario `kit` y la base `kit`, inseguro o seguro. Lo usa el CI y se puede
# usar en local.
#
#   bash pruebas-de-fuego/cockroach/laboratorio.sh <nombre> <versión> <puerto-sql> [seguro]
#   bash pruebas-de-fuego/cockroach/laboratorio.sh ore-crdb-243 v24.3.37 26258
#   bash pruebas-de-fuego/cockroach/laboratorio.sh ore-crdb-254-tls v25.4.18 26270 seguro
#
# Inseguro (`--insecure`): `kit` sin contraseña, con todo sobre la base `kit`. El conector sólo lo admite con
# `ORE_COCKROACH_LABORATORIO=1`.
# Seguro: CA y certificado propios (`localhost`, `127.0.0.1`), TLS y SCRAM. `kit` escribe en `kit` (contraseña `kit-secreto`) y
# `kit_ro` sólo lee (`kit-ro-secreto`). La CA queda en `$CRDB_CA_SALIDA` (por defecto `./<nombre>-ca.crt`), para darla como
# `sslrootcert=`.
#
# Idempotente: si el contenedor ya corre se reusa y sólo se aplica el SQL. `REHACER=1` lo borra y lo crea de nuevo.
set -eu
NOMBRE=${1:?nombre del contenedor}
VERSION=${2:?versión de CockroachDB con la v (v22.2.19, v24.3.37, v25.4.18…)}
PUERTO=${3:?puerto SQL en el anfitrión}
MODO=${4:-inseguro}
IMAGEN="cockroachdb/cockroach:$VERSION"
# Cada nodo reserva por defecto el 25 % de la memoria que ve para caché y otro 25 % para SQL: en la VM de Docker Desktop (unos 6 GB) tres o
# cuatro nodos se pisan, la VM empieza a usar swap y aparecen `10054` (conexión cerrada) y lecturas que se triplican. Con esto cada nodo se queda
# por debajo de 1 GB. Sólo vale para los nodos que se crean de nuevo.
MEMORIA=${MEMORIA:---cache=256MiB --max-sql-memory=256MiB}
# Git Bash reescribe las rutas que empiezan por `/`: los comandos de dentro del contenedor las llevan tal cual.
export MSYS_NO_PATHCONV=1

if [ "${REHACER:-0}" = 1 ]; then
  docker rm -f "$NOMBRE" >/dev/null 2>&1 || true
  docker volume rm -f "$NOMBRE-certs" >/dev/null 2>&1 || true
fi

if ! docker ps --format '{{.Names}}' | grep -qx "$NOMBRE"; then
  docker rm -f "$NOMBRE" >/dev/null 2>&1 || true
  if [ "$MODO" = seguro ]; then
    # Los certificados viven en un volumen con nombre: montar un directorio de Windows rompe los permisos de las llaves, y si el
    # fichero no existe Docker crea un DIRECTORIO con ese nombre.
    docker volume create "$NOMBRE-certs" >/dev/null
    docker run --rm -v "$NOMBRE-certs:/certs" "$IMAGEN" cert create-ca --certs-dir=/certs --ca-key=/certs/ca.key
    docker run --rm -v "$NOMBRE-certs:/certs" "$IMAGEN" cert create-node localhost 127.0.0.1 "$NOMBRE" --certs-dir=/certs --ca-key=/certs/ca.key
    docker run --rm -v "$NOMBRE-certs:/certs" "$IMAGEN" cert create-client root --certs-dir=/certs --ca-key=/certs/ca.key
    docker run -d --name "$NOMBRE" -p "$PUERTO:26257" -v "$NOMBRE-certs:/certs" "$IMAGEN" \
      start-single-node --certs-dir=/certs $MEMORIA >/dev/null
  else
    docker run -d --name "$NOMBRE" -p "$PUERTO:26257" "$IMAGEN" start-single-node --insecure --accept-sql-without-tls $MEMORIA >/dev/null
  fi
fi

if [ "$MODO" = seguro ]; then
  SQL=(docker exec -i "$NOMBRE" ./cockroach sql --certs-dir=/certs --host=localhost:26257)
else
  SQL=(docker exec -i "$NOMBRE" ./cockroach sql --insecure --host=localhost:26257)
fi

for _ in $(seq 1 60); do
  "${SQL[@]}" -e 'SELECT 1' >/dev/null 2>&1 && break
  sleep 2
done
"${SQL[@]}" -e 'SELECT 1' >/dev/null

if [ "$MODO" = seguro ]; then
  "${SQL[@]}" <<'SQL'
CREATE DATABASE IF NOT EXISTS kit;
CREATE USER IF NOT EXISTS kit WITH PASSWORD 'kit-secreto';
CREATE USER IF NOT EXISTS kit_ro WITH PASSWORD 'kit-ro-secreto';
GRANT ALL ON DATABASE kit TO kit;
GRANT CONNECT ON DATABASE kit TO kit_ro;
ALTER DEFAULT PRIVILEGES FOR ROLE kit IN SCHEMA kit.public GRANT SELECT ON TABLES TO kit_ro;
SET CLUSTER SETTING kv.rangefeed.enabled = true;
SQL
  "${SQL[@]}" -e 'ALTER ROLE kit WITH VIEWACTIVITY' >/dev/null 2>&1 || true
  "${SQL[@]}" -e 'GRANT SYSTEM VIEWACTIVITY TO kit' >/dev/null 2>&1 || true
  SALIDA=${CRDB_CA_SALIDA:-./$NOMBRE-ca.crt}
  # `docker.exe` en Windows no entiende `/c/…` de Git Bash.
  docker cp "$NOMBRE:/certs/ca.crt" "$(cygpath -m "$SALIDA" 2>/dev/null || echo "$SALIDA")"
  echo "$NOMBRE: $("${SQL[@]}" --format=tsv -e 'SELECT version()' | tail -1 | cut -c1-40) seguro en el puerto $PUERTO; CA en $SALIDA"
else
  "${SQL[@]}" <<'SQL'
CREATE DATABASE IF NOT EXISTS kit;
CREATE USER IF NOT EXISTS kit;
GRANT ALL ON DATABASE kit TO kit;
SET CLUSTER SETTING kv.rangefeed.enabled = true;
SQL
  # El banco del kit (caso 10) mira las consultas vivas con `SHOW CLUSTER QUERIES`: en 22.2 exige VIEWACTIVITY aun para las propias; desde
  # 24.3 basta ser el mismo usuario. El conector no lo necesita. Según la versión vale una forma u otra: se prueban las dos.
  "${SQL[@]}" -e 'ALTER ROLE kit WITH VIEWACTIVITY' >/dev/null 2>&1 || true
  "${SQL[@]}" -e 'GRANT SYSTEM VIEWACTIVITY TO kit' >/dev/null 2>&1 || true
  echo "$NOMBRE: $("${SQL[@]}" --format=tsv -e 'SELECT version()' | tail -1 | cut -c1-40) inseguro en el puerto $PUERTO"
fi
