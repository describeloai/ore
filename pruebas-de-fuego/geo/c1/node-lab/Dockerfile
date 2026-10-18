# ORE 0068 C1·2 · La imagen del puesto de Node para el laboratorio: las líneas 511–535 del Dockerfile
# (npm, @duckdb/node-api y las extensiones) sin ore-store-gcs, que obligaría a compilar Rust.
#   docker build -f pruebas-de-fuego/geo/c1/node-lab/Dockerfile -t ore-puesto-node:local .
FROM node:24-slim AS puesto-node

# ⛔ `node:24-slim` no trae `ca-certificates` (medido en `medida-w3-lago.py`):
#   Node verifica TLS con su manojo propio, pero DuckDB (OpenSSL) no puede
#   verificar a `storage.googleapis.com` y `iceberg_scan` por https falla con
#   «Problem with the SSL CA cert». Se instala; es lo único de apt en la imagen.
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates >/dev/null \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /opt/ore
# ⚠️ La versión de `@duckdb/node-api` es la de `puesto/node/provisto.txt` (0050
#   R3 T5b): la capa de Node no la copia y avisa si un repositorio pide otra.
#   UNA LISTA, DOS LECTORES; si no coinciden, la construcción falla aquí.
COPY puesto/node/provisto.txt /opt/ore/provisto.txt
RUN npm install --no-audit --no-fund --omit=dev @duckdb/node-api@1.5.5-r.5 \
 && v=$(node -p "JSON.parse(require('fs').readFileSync('node_modules/@duckdb/node-api/package.json','utf8')).version") \
 && { grep -qx "@duckdb/node-api@$v" /opt/ore/provisto.txt || { echo "✗ provisto.txt no dice @duckdb/node-api@$v, la de la imagen"; exit 1; }; } \
 && npm ls --depth=0 > /entorno-1.txt \
 && node -e "const d=require('@duckdb/node-api'); console.log('entorno 1 · node', process.version, '· duckdb', d.version())"
# ⭐ Las extensiones de DuckDB del lago, preinstaladas (ver la etapa de Python).
#   Y `nanoarrow` (community, 0057 B4·2·2): `@duckdb/node-api` no registra Arrow
#   (medido en 1.5.5), y lo que el SDK lee en vivo de un origen llega en Arrow
#   IPC; `read_arrow` lo lee de un fichero. Sin red en el puesto: va aquí.
RUN node -e "const d=require('@duckdb/node-api');(async()=>{const i=await d.DuckDBInstance.create();const c=await i.connect();await c.run(\"set extension_directory = '/opt/ore/duckdb'\");for(const e of ['iceberg','avro','httpfs','json','icu','spatial'])await c.run('install '+e);await c.run('install nanoarrow from community');await c.run('install h3 from community');await c.run('load spatial');await c.run('load h3');await c.run('load iceberg');await c.run('load httpfs');await c.run('load nanoarrow');console.log('lago ·',d.version(),(await c.runAndReadAll('select extension_name from duckdb_extensions() where loaded')).getColumns()[0].sort().join(' '))})().catch(e=>{console.error(e);process.exit(1)})" \
 && du -sh /opt/ore/duckdb >> /entorno-1.txt
# El SDK al lado del agente (`./ore/index.mjs`, como en `puesto/node/`) y como
