# Las cuatro imágenes de `ore`.
#
# Los drivers son binarios SEPARADOS por decisión —ADR 0008: `ore` los busca en
# el `PATH` y habla con ellos por stdin/stdout, así que el motor no enlaza un
# cliente de nube y un driver lo puede escribir cualquiera en cualquier
# lenguaje—. **Eso no obliga a imágenes separadas**: los diez binarios caben en
# una, y a esta escala la diferencia de descarga son segundos.
#
# Son dos, y la razón NO es el tamaño:
#
#   ore        ~7 MB    `scratch`. **No puede salir a la red aunque quiera**:
#                       no lleva certificados ni cliente TLS. Un `validate` que
#                       corre desde aquí no habla con nadie, y eso es una
#                       garantía estructural — más fuerte que una política que
#                       se lo prohíba, porque no hay nada que aplicar.
#
#   ore-informador      el informador de la celda (0026): `curl` y `jq`, y nada
#                       mas. Lee el API server con su Role y empuja a ore-iam.
#
#   ore-serve           el plano de control. Lleva `ore` y `git`, y NADA
#                       más: no puede leer un origen —el `ore` que ejecuta es
#                       el mismo binario sin TLS— y sí puede hablar con la
#                       forja, que es donde vive el árbol.
#
#   ore-drivers         todo lo que `ore` puede ejecutar: los tres `ore-read-*`,
#                       `ore-fetch`, `ore-log`, `ore-sign`, `ore-store-r2`, `ore-store-gcs` y `ore-invoke`,
#                       sobre el SDK de Google Cloud porque `ore-read-bigquery`
#                       delega en `bq` y no habla la API él mismo.
#
# Es la misma frontera que el sustrato ya tiene —12 de 14 crates no abren una
# conexión— y la misma que usa la `NetworkPolicy` de la malla.

# ── Compilación, una sola vez para las dos ──────────────────────────────────
FROM rust:1.90-alpine AS build

# `musl-dev` para el enlazador; OpenSSL estático sólo lo necesitan los que
# hablan TLS. El resto del árbol no arrastra FFI, y eso es lo que afirma de sí
# mismo.
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig

WORKDIR /src
COPY . .

# `--locked`: se construye con el `Cargo.lock` del árbol y no con lo que
# hubiera hoy en el índice.
RUN cargo build --release --locked \
      -p ore-cli -p ore-serve -p ore-iam -p ore-cofre \
      -p ore-read-jsonl -p ore-read-postgres -p ore-read-bigquery \
      -p ore-fetch -p ore-log -p ore-sign -p ore-store -p ore-invoke \
 && for b in ore ore-serve ore-iam ore-cofre ore-read-jsonl ore-read-postgres ore-read-bigquery \
             ore-fetch ore-log ore-sign ore-store-r2 ore-store-gcs ore-invoke; do \
      strip "target/release/$b"; \
    done

# ── 1 · La fina, que no sabe hablar con nadie ───────────────────────────────
FROM scratch AS ore

COPY --from=build /src/target/release/ore            /bin/ore
COPY --from=build /src/target/release/ore-read-jsonl /bin/ore-read-jsonl
# El `PATH` de un `scratch` está vacío, y `ore` busca sus drivers por ahí.
ENV PATH=/bin
USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/bin/ore"]

# ── 2 · La del conjunto ─────────────────────────────────────────────────────
#
# Sobre el SDK de Google Cloud y no sobre `alpine` porque `ore-read-bigquery`
# necesita `bq` en el `PATH`. Trae además las CA del sistema, que es lo que
# convierte «hablar TLS» en «verificar contra quién»: un binario estático sin
# ellas se conecta y no sabe con quién habla.
FROM gcr.io/google.com/cloudsdktool/google-cloud-cli:alpine AS drivers

# ── ⭐ `git` y `psql`, y los dos por el APROVISIONADOR ──────────────────────
#
# Esta imagen ya no es sólo la de los drivers: es también la del Job que da de
# alta un inquilino (`16-el-aprovisionador.yaml`), y ése necesita las cuatro
# herramientas a la vez —`gcloud` para la nube, `curl` para la forja, `git` para
# empujar el compartimento y `psql` para leer la fila—.
#
# ⚠️ `psql` faltaba, y el síntoma no lo dijo: el guion leía la fila con la salida
# de error tapada, así que un `psql: not found` salió como «`prueba` no está
# fundada» — un mensaje que manda a mirar la base de datos cuando lo que falta
# es un binario. Costó un pod de usar y tirar averiguarlo.
#
# ⇒ Y es lo que permite que el Job lea como `aprovisionador`, el papel de la
#   `023`: cuatro columnas de una tabla, en vez del `kubectl exec` que era un
#   `psql` como superusuario dentro del pod de la base.
RUN apk add --no-cache git postgresql-client py3-yaml

# ── ⛔⛔ DOS PYTHON EN ESTA IMAGEN, Y `apk` INSTALA PARA EL QUE NADIE USA ────
#
# Esto empezo siendo `apk add py3-yaml`, por la comprobacion ⑦: el que converge
# ANALIZA como YAML todo lo que va a rendir antes de tocar a ningun inquilino, y
# sin analizador `--comprobar-plantillas` **falla** en vez de saltarselo — a
# proposito, porque una comprobacion ausente y una que pasa se leen igual en un
# registro, y esa confusion es la que dejo pasar un `44-el-catalogo.yaml` roto.
#
# Y el paquete se instalo, y no sirvio de nada. Medido dentro de la imagen:
#
#     /usr/local/bin/python3   3.14, el del Cloud SDK  <- lo que resuelve `python3`
#     /usr/bin/python3         3.12, el de Alpine      <- donde `apk` puso pyyaml
#
# ⇒ La convergencia fallo con «no hay analizador de YAML aqui» teniendo el
#   paquete presente (`apk info -e py3-yaml` lo confirmaba). Un fallo que se lee
#   como «falta instalar algo» cuando lo que pasa es que se instalo AL LADO.
#
# ⭐ Asi que se instala en LOS DOS, y a proposito: `py3-yaml` arriba para el de
#   Alpine, `pip` aqui para el que `python3` resuelve de verdad. Cuesta unos
#   cientos de kilobytes y compra que deje de importar cual se use — que es
#   mejor que acertar con uno y dejar la trampa puesta para el siguiente.
#
#   Es la misma leccion que `psql`: lo que importa no es que la herramienta
#   exista en la imagen, sino que la encuentre quien la llama.
#
# ⚠️ Y el `import yaml` de despues no es adorno: si `pip` no llega a pypi desde
#   Cloud Build, la construccion FALLA aqui en vez de publicar una imagen que
#   se niega a converger. El fallo se paga donde hay alguien mirando.
#
# ⚠️ Y hasta que esta imagen se publique, el `CronJob` de convergencia se niega
#   a correr. Es el fallo por el lado bueno, y conviene saber que es ese.
RUN python3 -m pip install --no-cache-dir --break-system-packages pyyaml && python3 -c "import sys, yaml; print('pyyaml listo para', sys.executable)"

COPY --from=build /src/target/release/ore                /usr/local/bin/ore
COPY --from=build /src/target/release/ore-read-jsonl     /usr/local/bin/ore-read-jsonl
COPY --from=build /src/target/release/ore-read-postgres  /usr/local/bin/ore-read-postgres
COPY --from=build /src/target/release/ore-read-bigquery  /usr/local/bin/ore-read-bigquery
COPY --from=build /src/target/release/ore-fetch          /usr/local/bin/ore-fetch
COPY --from=build /src/target/release/ore-log            /usr/local/bin/ore-log
COPY --from=build /src/target/release/ore-sign           /usr/local/bin/ore-sign
COPY --from=build /src/target/release/ore-store-r2       /usr/local/bin/ore-store-r2
COPY --from=build /src/target/release/ore-store-gcs      /usr/local/bin/ore-store-gcs
# El invocador (0029 ⑤): la unica capacidad que anade es hablar con la puerta
# de modelos, con el token de la celda. Vive aqui por lo mismo que el almacen.
COPY --from=build /src/target/release/ore-invoke         /usr/local/bin/ore-invoke

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/usr/local/bin/ore"]

# ── 3 · El plano de control ─────────────────────────────────────────────────
#
# Sobre `alpine` y no sobre `scratch`, y hay que decir por qué se pierde la
# garantía estructural de la primera: **este proceso necesita `git`**, porque el
# árbol vive en la forja y cada petición clona.
#
# Lo que NO se pierde:
#
#   · el `ore` que ejecuta es el MISMO binario sin certificados ni TLS, así que
#     un `discover` desde aquí sigue sin poder hablar con un origen;
#   · `mando::HERMETICOS` es una lista de PERMITIDOS, así que un verbo que
#     toque el mundo se niega antes de intentarlo;
#   · y la `NetworkPolicy` de la malla decide a dónde puede ir `git`.
#
# Tres cerraduras sobre la misma puerta, y ninguna es la imagen. Se dice porque
# la de `ore` SÍ lo era, y perder una garantía sin nombrarla es como se pierden.
FROM alpine:3.22 AS serve

RUN apk add --no-cache git ca-certificates

COPY --from=build /src/target/release/ore       /usr/local/bin/ore
COPY --from=build /src/target/release/ore-serve /usr/local/bin/ore-serve
# W1 ④ (0030): `POST /vistas/{ns}/{n}/ejecutar` corre `ore ask`, y quien trae
# la copia del bucket es este programa — con la identidad del pod (`objectViewer`,
# aprovisionador ③b) y nunca un origen. `ore` sigue sin abrir un socket.
COPY --from=build /src/target/release/ore-store-gcs /usr/local/bin/ore-store-gcs

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/usr/local/bin/ore-serve"]

# ── 3b · El informador de la celda (0026 E2) ────────────────────────────────
#
# Alpine, `curl` (con verificacion de certificados: el API server se habla con
# `--cacert` del pod) y `jq`. Ni `gcloud`, ni `ore`, ni `git`: el agente lo trae
# un init con la imagen de drivers, y este contenedor solo mide y empuja. Es
# OTRO proceso con OTRO privilegio, y no puede hacer lo que hace `ore-serve`.
FROM alpine:3.22 AS informador

RUN apk add --no-cache curl ca-certificates jq

COPY malla/informar.sh /usr/local/bin/informar
RUN chmod 0755 /usr/local/bin/informar

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/informar"]

# ── 4 · El plano de identidad y acceso ──────────────────────────────────────
#
# Sobre `scratch`, como `ore` y a diferencia de `ore-serve`. Y no es una
# casualidad: **este proceso tampoco sabe hablar TLS**.
#
#   la base        Postgres en claro, de pod a pod dentro del clúster — la
#                  misma elección que ya hace Keycloak con esa misma base
#   las llaves     un FICHERO. No se va a buscar el JWKS (ADR 0020, enmienda)
#   hacia fuera    nada
#
# ⇒ Un binario que administra personas y concesiones y **no puede abrir una
#   conexión TLS a ningún sitio**. No es una promesa: no lleva el código.
FROM scratch AS iam

COPY --from=build /src/target/release/ore-iam /bin/ore-iam

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/bin/ore-iam"]

# ── 5 · El custodio ─────────────────────────────────────────────────────────
#
# ⛔⛔ Y AQUÍ SE PIERDE LA GARANTÍA DE `scratch` A PROPÓSITO, otra vez. Igual que
# `ore-serve` la perdió para tener `git`, éste la pierde para tener `gcloud`:
# **es lo único con lo que habla con el KMS**, y no habla con el KMS — habla con
# él. Es el patrón de `ore-read-bigquery` con `bq`.
#
# Lo que se compra cediéndola:
#
#   · ni TLS, ni OAuth, ni criptografía en Rust. Tres cosas cuyo modo de fallo
#     es silencioso y en la dirección insegura, que es la frase que este árbol
#     ya tiene escrita para las firmas;
#   · la autenticación es Workload Identity y la resuelve `gcloud` sola. **No
#     hay una sola llave en el clúster.**
#
# Y lo que NO se pierde, que es lo que hace aceptable la cesión:
#
#   · este binario no puede abrir la llave de otro inquilino — su cuenta de
#     Google alcanza UNA clave, y eso lo demuestra `99-el-cerrojo-de-la-llave`;
#   · su usuario de base de datos no alcanza `iam` más allá de lo que necesita
#     para autorizar (`020`);
#   · y su NetworkPolicy sólo le abre el DNS, el servidor de metadatos y Google.
#
# ⇒ Tres cerraduras sobre la misma puerta, y ninguna es la imagen. Se dice
#   porque la de `ore` SÍ lo era, y perder una garantía sin nombrarla es como se
#   pierden.
FROM gcr.io/google.com/cloudsdktool/google-cloud-cli:alpine AS cofre

COPY --from=build /src/target/release/ore-cofre /usr/local/bin/ore-cofre

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/usr/local/bin/ore-cofre"]

# ── 6 · El puesto: Python, entorno 1 (0031 W3) ──────────────────────────────
#
# La imagen BASE de una sesión Python del workspace. No sale del `build` de
# arriba: no lleva un binario nuestro, lleva un RUNTIME — y por eso se numera
# como lo hacen Databricks y Foundry (entorno 1, 2, 3…), nunca como `latest`:
# lo que una celda importa hoy tiene que importar igual dentro de un año.
#
# ⛔ Sin `pip` en caliente como verdad. Lo que hay es lo que hay; lo que un
#   paquete del árbol declare se resuelve en CI en una CAPA sobre ésta (0031).
#   El `pip freeze` de abajo deja en la imagen el `requirements` que reproduce
#   el entorno en local, como el `requirements-env-N.txt` de Databricks.
#
# ⚠️ Los nodos de `jobs-p` no alcanzan Docker Hub (privados, sin NAT): TODO lo
#   que corra ahí sale de nuestro registro, y por eso esta imagen existe antes
#   que la medida de W3 — no hay forma de medir un puesto Python sin ella.
# ═══════════════════════════════════════════════════════════════════════════
# EL SERVIDOR DE LENGUAJE DE PYTHON (0037 ③a) — se trae aquí, donde hay npm
#
# `pyright` es un programa de Node (no hay versión nativa), y se instala con el
# npm de esta etapa para no meter npm en la imagen del puesto: de aquí sólo
# viajan el paquete y, abajo, el binario `node`.
# ═══════════════════════════════════════════════════════════════════════════
FROM node:24-slim AS pyright
# ⛔ FIJADA. Un servidor de lenguaje que se actualiza solo cambia lo que el
#   editor subraya sin que nadie lo decida.
ARG PYRIGHT=1.1.414
RUN npm install --no-audit --no-fund --omit=dev --prefix /opt/p pyright@${PYRIGHT} \
 && node /opt/p/node_modules/pyright/index.js --version

FROM python:3.12-slim AS puesto-python

RUN pip install --no-cache-dir pandas pyarrow duckdb google-cloud-storage \
 && pip freeze > /entorno-1.txt \
 && python -c "import pandas, pyarrow, duckdb, google.cloud.storage as s; print('entorno 1 ·', pandas.__version__, pyarrow.__version__, duckdb.__version__)"
# ⭐ Las extensiones de DuckDB que leen el LAGO (0031 §10, medido en
#   `medida-w3-lago.py`): `iceberg` (con `avro`, `httpfs`, `json`, `icu` detrás)
#   PREINSTALADAS aquí, donde hay red, en /opt/ore/duckdb —de la versión exacta
#   de DuckDB de este enlace—. El pod no sale a internet: sin esto, la primera
#   celda que las pidiera se colgaría 120 s contra la NetworkPolicy. El SDK abre
#   DuckDB con `extension_directory` ahí y `autoinstall_known_extensions=false`.
RUN python -c "import duckdb; c = duckdb.connect(); c.execute(\"set extension_directory = '/opt/ore/duckdb'\"); [c.execute('install ' + e) for e in ('iceberg', 'avro', 'httpfs', 'json', 'icu')]; c.execute('load iceberg'); c.execute('load httpfs'); print('lago ·', duckdb.__version__, sorted(r[0] for r in c.execute('select extension_name from duckdb_extensions() where loaded').fetchall()))" \
 && du -sh /opt/ore/duckdb >> /entorno-1.txt

# El agente y el SDK (`puesto/python/`): lo unico nuestro en la imagen. `ore`
# se importa desde la celda; el agente lo pone en el `sys.path` por estar al lado.
COPY puesto/python/agente.py /opt/ore/agente.py
COPY puesto/python/ore       /opt/ore/ore
# ⭐ El escritor del lago (W3.6c, 0031 §11 ②): `write()` manda la tabla por IPC a
#   `ore-store-gcs`, que escribe los ficheros con la credencial que el catálogo
#   prestó —acotada a la tabla— y devuelve el commit. Es el mismo binario de la
#   copia (`ore-drivers`); el puesto lo lleva, y no lleva `ore`.
COPY --from=build /src/target/release/ore-store-gcs /usr/local/bin/ore-store-gcs
RUN python -c "import sys; sys.path.insert(0, '/opt/ore'); import ore, ast; ast.parse(open('/opt/ore/agente.py').read()); print('agente y sdk listos')"

# ═══════════════════════════════════════════════════════════════════════════
# ⭐ EL SERVIDOR DE LENGUAJE, DENTRO (0037 ③a)
#
# Medido antes de meterlo (`medida-los-servidores-de-lenguaje.py`, corriendo los
# servidores de verdad con un cliente LSP): pyright arranca en 0,5 s, da el
# primer diagnóstico en 3,8 s, propone 46 cosas en 0,10 s y pide 210 MB. La
# alternativa sin Node —`jedi-language-server`, 10 MB de memoria— propone UNA en
# 3,7 s y sólo ve errores de sintaxis. El pod pide 1 CPU / 2 GiB: cabe.
#
# ⛔ Y EL PRECIO ESTÁ DICHO: el binario `node` son ~127 MB, porque esta imagen
#   sale de `python:3.12-slim` y ahí no hay Node. Es lo que cuesta el mejor de
#   los dos, y se paga una vez por imagen, no por sesión.
#
# ⭐ LO QUE ESTO COMPRA, Y QUE EL NAVEGADOR NO PUEDE TENER: aquí están **el SDK**
#   (`/opt/ore/ore`) y **la capa del repositorio** (`/capa`, el PYTHONPATH con lo
#   que el árbol declara en su `pyproject.toml`). Monaco puede colorear; saber
#   qué devuelve `over()` sólo se puede saber donde vive `over`.
COPY --from=node:24-slim /usr/local/bin/node /usr/local/bin/node
COPY --from=pyright /opt/p/node_modules/pyright /opt/ore/pyright

# ⭐⭐ Y LA CONSTRUCCIÓN LO DEMUESTRA, que es la diferencia entre meter un
#   programa y meter una promesa: se analiza una semilla como la que siembra
#   `transforms-python` —con su `from ore import …`— contra el SDK de ESTA
#   imagen, y se exige CERO errores. El día que el SDK deje de resolverse, la
#   imagen no se construye; sin esto nos enteraríamos por un subrayado rojo en
#   la pantalla de alguien.
RUN mkdir -p /tmp/lsp && cd /tmp/lsp \
 && printf '{"extraPaths":["/opt/ore"],"typeCheckingMode":"basic"}' > pyrightconfig.json \
 && printf 'from ore import over, write, transform\n\n\n@transform(inputs=["a.b"], output="a.c")\ndef f():\n    return write("a.c", over("a.b"))\n' > ejemplo.py \
 && node /opt/ore/pyright/index.js --outputjson ejemplo.py > /tmp/lsp/salida.json \
 && python -c "import json; d=json.load(open('/tmp/lsp/salida.json'))['summary']; assert d['errorCount']==0, d; print('pyright ·', d)" \
 && node /opt/ore/pyright/index.js --version >> /entorno-1.txt \
 && du -sh /opt/ore/pyright /usr/local/bin/node >> /entorno-1.txt \
 && rm -rf /tmp/lsp

USER 65532:65532
WORKDIR /trabajo
# El agente es el CMD: la plantilla del puesto (51-el-puesto.yaml) no lleva
# `command`, y así vale para los tres entornos.
CMD ["python3", "/opt/ore/agente.py"]

# ═══════════════════════════════════════════════════════════════════════════
# Etapa 7 · puesto-node:1 — el ENTORNO 1 de TS/JS (0031 W3.4)
#
# Node 24 quita los tipos de TypeScript por sí mismo (sin tsc ni esbuild:
# `process.features.typescript = "strip"`; medido en victor: un `.ts` se
# importa tal cual en 4 ms). DuckDB (`@duckdb/node-api`) es lo que hace que
# `over()`/`sql()` lean las copias Parquet, como en Python. El SDK va como
# paquete `ore` en `/opt/ore/node_modules`: una celda-módulo (con `import`/
# `export`) escrita en /trabajo lo resuelve por el enlace que hace el agente.
# ═══════════════════════════════════════════════════════════════════════════
FROM node:24-slim AS puesto-node

# ⛔ `node:24-slim` no trae `ca-certificates` (medido en `medida-w3-lago.py`):
#   Node verifica TLS con su manojo propio, pero DuckDB (OpenSSL) no puede
#   verificar a `storage.googleapis.com` y `iceberg_scan` por https falla con
#   «Problem with the SSL CA cert». Se instala; es lo único de apt en la imagen.
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates >/dev/null \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /opt/ore
RUN npm install --no-audit --no-fund --omit=dev @duckdb/node-api@1.5.5-r.5 \
 && npm ls --depth=0 > /entorno-1.txt \
 && node -e "const d=require('@duckdb/node-api'); console.log('entorno 1 · node', process.version, '· duckdb', d.version())"
# ⭐ Las extensiones de DuckDB del lago, preinstaladas (ver la etapa de Python).
RUN node -e "const d=require('@duckdb/node-api');(async()=>{const i=await d.DuckDBInstance.create();const c=await i.connect();await c.run(\"set extension_directory = '/opt/ore/duckdb'\");for(const e of ['iceberg','avro','httpfs','json','icu'])await c.run('install '+e);await c.run('load iceberg');await c.run('load httpfs');console.log('lago ·',d.version(),(await c.runAndReadAll('select extension_name from duckdb_extensions() where loaded')).getColumns()[0].sort().join(' '))})().catch(e=>{console.error(e);process.exit(1)})" \
 && du -sh /opt/ore/duckdb >> /entorno-1.txt
# El SDK al lado del agente (`./ore/index.mjs`, como en `puesto/node/`) y como
# paquete `ore` para las celdas-módulo (un enlace en node_modules). ⛔ Medido en
# victor el 2026-09-19: sólo en node_modules, el agente moría al arrancar con
# ERR_MODULE_NOT_FOUND y `node --check` no lo veía — por eso la comprobación
# de abajo ARRANCA el agente (`--comprobar`: importa, crea el kernel, corre una celda).
COPY puesto/node/agente.mjs /opt/ore/agente.mjs
COPY puesto/node/ore        /opt/ore/ore
COPY --from=build /src/target/release/ore-store-gcs /usr/local/bin/ore-store-gcs
RUN ln -s ../ore /opt/ore/node_modules/ore \
 && ORE_CELDAS=/tmp/comprobar node /opt/ore/agente.mjs --comprobar

USER 65532:65532
WORKDIR /trabajo
CMD ["node", "/opt/ore/agente.mjs"]

# ═══════════════════════════════════════════════════════════════════════════
# Etapa 8 · puesto-jvm:1 — el ENTORNO 1 de Java (0031 W3.4)
#
# JDK 21 (hace falta el JDK: `jdk.jshell` no va en el JRE). El agente evalúa
# cada celda con JShell EN PROCESO (`executionEngine("local")`: 25 ms de crear,
# 20–250 ms por celda; el motor remoto por JDI tarda 770 ms — medido en
# victor). El SDK (`ore.Ore`) va compilado en /opt/ore/clases y se importa
# estático en la sesión. DuckDB por JDBC lee el Parquet y lo entrega por
# ARROW (`arrowExportStream`, 0032 T3/T4: exacto en los 23 tipos, 14,6 M
# filas/s; el mapeo de JDBC tenía cuatro tipos mal): los jars de Arrow Java
# los dice `puesto/jvm/jars.txt` (una lista, tres lectores) y hacen falta
# `--add-opens=java.base/java.nio` (arrow-memory-unsafe). Sobre Ubuntu (glibc)
# y no alpine: el JDBC de DuckDB no trae natives para musl.
# ═══════════════════════════════════════════════════════════════════════════
FROM eclipse-temurin:21-jdk-noble AS puesto-jvm

ARG DUCKDB_JDBC=1.5.5.1
COPY puesto/jvm /opt/ore/src
COPY --from=build /src/target/release/ore-store-gcs /usr/local/bin/ore-store-gcs
RUN mkdir -p /opt/ore/lib /opt/ore/clases \
 && curl -fsSL -o /opt/ore/lib/duckdb_jdbc.jar \
      "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/${DUCKDB_JDBC}/duckdb_jdbc-${DUCKDB_JDBC}.jar" \
 && grep -v '^#' /opt/ore/src/jars.txt | while read -r g v; do n="${g##*/}-$v.jar"; \
      curl -fsSL --retry 3 --retry-all-errors -o "/opt/ore/lib/$n" "https://repo1.maven.org/maven2/$g/$v/$n" || exit 1; done \
 && echo "duckdb_jdbc ${DUCKDB_JDBC} · $(ls /opt/ore/lib | wc -l) jars" > /entorno-1.txt && java -version 2>> /entorno-1.txt
# ⭐ Las extensiones de DuckDB del lago, preinstaladas (ver la etapa de Python):
#   con el JDBC, que es el DuckDB de este enlace, como programa de un fichero
#   (un fallo tumba la construcción; JShell se lo tragaría).
RUN java -cp /opt/ore/lib/duckdb_jdbc.jar /opt/ore/src/preinstalar/Extensiones.java /opt/ore/duckdb  && du -sh /opt/ore/duckdb >> /entorno-1.txt
RUN javac -Xlint:-options --release 21 -cp "/opt/ore/lib/*" -d /opt/ore/clases /opt/ore/src/ore/*.java \
 && java --add-opens=java.base/java.nio=ALL-UNNAMED -cp "/opt/ore/clases:/opt/ore/lib/*" ore.Agente --comprobar

USER 65532:65532
WORKDIR /trabajo
# ⭐⭐ Y LA CAPA VA LA ÚLTIMA (0037 ③c): `/capa/*` son las bibliotecas que el
#   repositorio declaró en su `pom.xml`, y van DETRÁS de las de la imagen a
#   propósito. Medido (`medida-la-capa-de-la-jvm.py` §6): cuando una clase está
#   en dos sitios del classpath, QUIEN GANA LO DECIDE EL ORDEN, no la versión.
#   El SDK está compilado contra los jars de `/opt/ore/lib`, así que manda el
#   contenedor; el Job que resuelve ya evita el duplicado con `provided`, y
#   esto es el cinturón por si algo se colara igual.
#
# ⚠️ Un `dir/*` en el classpath lo expande el LANZADOR (aquí) pero NO lo
#   expanden ni `JShell.addToClasspath` ni `System.getProperty`: por eso
#   `Agente.java` lo expande él cuando se lo pasa a JShell. Sin capa, `/capa/*`
#   no aporta nada y no estorba.
CMD ["java", "-XX:+UseSerialGC", "--add-opens=java.base/java.nio=ALL-UNNAMED", "-cp", "/opt/ore/clases:/opt/ore/lib/*:/capa/*", "ore.Agente"]

# ═══════════════════════════════════════════════════════════════════════════
# Etapa 9 · capa-jvm:1 — QUIEN RESUELVE LA CAPA DE LA JVM (0037 ③c)
#
# ⭐ Una imagen aparte, y por lo que NO lleva. El puesto no alcanza Central
#   (21-el-puesto.yaml) y esta imagen sí, pero a cambio no tiene ni testigo de
#   la forja ni credencial de la nube: lo que resuelve no puede publicar nada,
#   y lo que publica —el contenedor `ore-drivers` del Job— no sale a Central.
#   Dos capacidades que hoy nadie tiene juntas.
#
# ⛔ Y NO va dentro de `puesto-jvm:1`: Maven en el puesto serían 9 MB y una
#   herramienta de construcción en todas las sesiones para algo que sólo hace
#   un Job. Ni dentro de `ore-drivers`: un JDK de 180 MB en la imagen que
#   arrastran TODOS los Jobs del driver.
#
# El JDK ya lo trae la base —la misma que `puesto-jvm:1`, para que lo que se
# resuelve y lo que lo corre sean la misma versión— y Maven se baja del
# archivo canónico de Apache CON SU SHA-512 ESCRITO AQUÍ: un tarball que no
# cuadre rompe la construcción en vez de entrar en la imagen.
# ═══════════════════════════════════════════════════════════════════════════
FROM eclipse-temurin:21-jdk-noble AS capa-jvm

ARG MAVEN=3.9.9
ARG MAVEN_SHA512=a555254d6b53d267965a3404ecb14e53c3827c09c3b94b5678835887ab404556bfaf78dcfe03ba76fa2508649dca8531c74bca4d5846513522404d48e8c4ac8b
# ⚠️ LA MISMA que `puesto-jvm`, y tiene que seguir siéndolo: es uno de los jars
#   que la imagen del puesto pone, así que entra en la lista de `provided`.
ARG DUCKDB_JDBC=1.5.5.1

RUN curl -fsSL -o /tmp/maven.tar.gz \
      "https://archive.apache.org/dist/maven/maven-3/${MAVEN}/binaries/apache-maven-${MAVEN}-bin.tar.gz" \
 && echo "${MAVEN_SHA512}  /tmp/maven.tar.gz" | sha512sum -c - \
 && mkdir -p /opt/maven && tar -xzf /tmp/maven.tar.gz -C /opt/maven --strip-components=1 \
 && rm /tmp/maven.tar.gz && ln -s /opt/maven/bin/mvn /usr/local/bin/mvn && mvn -v

# La lista de jars de la imagen del puesto: UNA LISTA, Y AHORA CUATRO LECTORES
# —el Dockerfile del puesto, `el-puesto.sh`, las medidas y esto—. Que el que
# resuelve y el que corre lean el MISMO fichero es lo que evita que el
# `provided` se quede corto el día que se añada un jar.
COPY puesto/jvm/jars.txt /opt/ore/jars.txt
COPY puesto/jvm/capa /opt/ore/capa-src
RUN mkdir -p /opt/ore/capa \
 && javac -Xlint:-options --release 21 -d /opt/ore/capa /opt/ore/capa-src/Capa.java \
 && cp /opt/ore/capa-src/resolver.sh /opt/ore/resolver.sh && chmod 0755 /opt/ore/resolver.sh \
 && java -cp /opt/ore/capa Capa provisto /opt/ore/jars.txt "${DUCKDB_JDBC}" > /opt/ore/provisto.txt \
 && test "$(wc -l < /opt/ore/provisto.txt)" -ge 14 \
 && echo "maven ${MAVEN} · $(wc -l < /opt/ore/provisto.txt) jars provistos por el puesto" > /capa-jvm.txt

# ── ⭐ Y SE PRUEBA AQUÍ, donde hay alguien mirando ─────────────────────────
#
# Un repositorio que declara `jackson-databind 2.19.0` cuando la imagen lleva
# la 2.18.2 es EL caso difícil de ③c, y la construcción falla si deja de
# comportarse como se midió: no se copia ni un jackson —gana el contenedor—,
# se copia lo que sí es suyo, y el choque sale como una frase en el informe.
RUN set -e; mkdir -p /tmp/p/arbol; \
    printf '%s\n' '<project xmlns="http://maven.apache.org/POM/4.0.0">' \
      '<modelVersion>4.0.0</modelVersion><dependencies>' \
      '<dependency><groupId>com.fasterxml.jackson.core</groupId><artifactId>jackson-databind</artifactId><version>2.19.0</version></dependency>' \
      '<dependency><groupId>org.apache.commons</groupId><artifactId>commons-lang3</artifactId><version>3.17.0</version></dependency>' \
      '</dependencies></project>' > /tmp/p/arbol/pom.xml; \
    TRABAJO=/tmp/p/t /opt/ore/resolver.sh /tmp/p/arbol ""; \
    ls /tmp/p/t/jars; \
    test -f /tmp/p/t/jars/commons-lang3-3.17.0.jar; \
    ! ls /tmp/p/t/jars | grep -q jackson; \
    grep -q '"estado": "lista"' /tmp/p/t/informe.json; \
    grep -q 'pediste com.fasterxml.jackson.core:jackson-databind 2.19.0, y esta sesión trae la 2.18.2' /tmp/p/t/informe.json; \
    grep -q '"sumas"' /tmp/p/t/informe.json; \
    cat /tmp/p/t/informe.json >> /capa-jvm.txt; rm -rf /tmp/p

# ── ⭐ Y EL OTRO CAMINO DEL MISMO CHOQUE (0037 ③c · d) ─────────────────────
#
# Aquí el repositorio NO pide jackson: pide `jackson-dataformat-yaml 2.19.0`,
# que ARRASTRA jackson 2.19.0. Maven resuelve esto EN SILENCIO —medido—, así
# que el aviso no puede salir de su registro: sale de resolver aparte lo que el
# repositorio querría sin contenedor y cruzarlo con lo que el contenedor pone.
# Si eso deja de funcionar, la construcción falla aquí y no en el árbol de un
# cliente seis meses después.
RUN set -e; mkdir -p /tmp/q/arbol; \
    printf '%s\n' '<project xmlns="http://maven.apache.org/POM/4.0.0">' \
      '<modelVersion>4.0.0</modelVersion><dependencies>' \
      '<dependency><groupId>com.fasterxml.jackson.dataformat</groupId><artifactId>jackson-dataformat-yaml</artifactId><version>2.19.0</version></dependency>' \
      '</dependencies></project>' > /tmp/q/arbol/pom.xml; \
    TRABAJO=/tmp/q/t /opt/ore/resolver.sh /tmp/q/arbol ""; \
    ls /tmp/q/t/jars; \
    test -f /tmp/q/t/jars/jackson-dataformat-yaml-2.19.0.jar; \
    test -f /tmp/q/t/jars/snakeyaml-2.4.jar; \
    ! ls /tmp/q/t/jars | grep -q 'jackson-databind'; \
    grep -q 'jackson-databind 2.19.0 lo arrastra algo que declaraste' /tmp/q/t/informe.json; \
    cat /tmp/q/t/informe.json >> /capa-jvm.txt; rm -rf /tmp/q

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/opt/ore/resolver.sh"]
