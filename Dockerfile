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
#                       forja, que es donde vive el árbol. (Y dos ayudantes:
#                       `ore-store-gcs`, que lee SU lago, y `ore-firmar-s3`,
#                       que prefirma sin poder abrir un socket — 0046 E9·3.)
#
#   ore-drivers         todo lo que `ore` puede ejecutar: los cuatro `ore-read-*`
#                       (`ore-read-s3`, un bucket como fuente, desde 0046 E4),
#                       `ore-fetch`, `ore-log`, `ore-sign`, `ore-store-r2`, `ore-store-gcs` y `ore-invoke`,
#                       sobre el SDK de Google Cloud, que ya no es por
#                       `ore-read-bigquery` (habla REST desde A2, ADR 0042)
#                       sino por el Job del aprovisionador, que usa `gcloud`.
#
# Es la misma frontera que el sustrato ya tiene —12 de 14 crates no abren una
# conexión— y la misma que usa la `NetworkPolicy` de la malla.

# ── Compilación, una sola vez para las dos ──────────────────────────────────
# ⭐ LOS BINARIOS, POR SU CONTENIDO (2026-10-03). Las imágenes copian sus
#   ejecutables de `bin`, que es la etapa `binarios` de aquí abajo… o, en Cloud
#   Build, la imagen `ore-binarios:<…>` que ya estaba si la huella de lo que
#   compila Rust no cambió (`ci/huella-de-los-binarios.sh`). Antes, `COPY . .`
#   hacía que CUALQUIER fichero —un ADR, un `.py`— recompilara los diecisiete
#   binarios: ~20 min por commit, la mitad sin tocar Rust.
ARG BINARIOS=binarios

FROM rust:1.90-alpine AS herramientas

# `musl-dev` para el enlazador; OpenSSL estático sólo lo necesitan los que
# hablan TLS. El resto del árbol no arrastra FFI, y eso es lo que afirma de sí
# mismo.
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig curl xz

# cargo-chef: compila las dependencias de un árbol sin su código (la receta son
# los `Cargo.toml` y el `Cargo.lock`). La versión va fijada y comprobada por su
# sha256. (sccache se fue el 2026-10-03: con él `ore-medios` y `ore-cofre`
# morían al arrancar con SIGSEGV, en frío y en caliente; medido.)
RUN curl -sSfL -o /tmp/c.tar.xz https://github.com/LukeMathWalker/cargo-chef/releases/download/v0.1.78/cargo-chef-x86_64-unknown-linux-musl.tar.xz  && echo "aca691abfbfbbe00d482e0ed2249eec3091b65e96a0ce92947fb5b254d48b16d  /tmp/c.tar.xz" | sha256sum -c -  && tar -xJf /tmp/c.tar.xz -C /tmp && mv /tmp/cargo-chef-x86_64-unknown-linux-musl/cargo-chef /usr/local/bin/cargo-chef  && rm -rf /tmp/c.tar.xz /tmp/cargo-chef-x86_64-unknown-linux-musl
WORKDIR /src
# Fuera de /src: el paso `binarios` monta el árbol en /src y lo compilado de
# las dependencias (`deps`) tiene que seguir donde estaba.
ENV CARGO_TARGET_DIR=/t

# ⭐ LAS DEPENDENCIAS, POR SU RECETA (CI paso 2, 2026-10-03). Lo de terceros
#   (~450 crates) no cambia de un commit a otro y era la mayor parte de cada
#   compilación. `receta` saca de los manifiestos lo que hay que compilar;
#   `deps` lo compila con el MISMO perfil, paquetes y guion que los binarios, y
#   cloudbuild.yaml la guarda como `ore-deps:h-<huella de la receta>`. Nuestro
#   código se compila SIEMPRE entero encima: nada nuestro sale de una caché.
FROM herramientas AS receta
# Sin `rust-toolchain.toml`: la receta sale de los manifiestos y no depende del
# compilador; con él, rustup bajaría el toolchain entero sólo para leerlos.
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path /receta.json

FROM herramientas AS deps
COPY rust-toolchain.toml ./
COPY ci/compilar-binarios.sh /ci/compilar-binarios.sh
COPY --from=receta /receta.json /receta.json
RUN RECETA=/receta.json sh /ci/compilar-binarios.sh

# En local: la misma compilación que en Cloud Build, aquí dentro.
FROM deps AS build
# Sólo lo que `cargo` lee: lo demás del árbol no puede cambiar un binario (y
# es lo que `ci/huella-de-los-binarios.sh` mira).
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY vendor/oos vendor/oos
# L6·1b: las listas de lo que traen las imágenes de los puestos, que
# `ore-serve` compila dentro (`entorno.rs`, `include_str!`).
COPY puesto/node/provisto.txt puesto/node/provisto.txt
COPY puesto/node/sugeridas.txt puesto/node/sugeridas.txt
COPY puesto/python/provisto.txt puesto/python/provisto.txt
COPY puesto/jvm/jars.txt puesto/jvm/jars.txt
COPY ci/compilar-binarios.sh ci/compilar-binarios.sh
RUN SALIDA=/b sh ci/compilar-binarios.sh

# ── 0 · Los binarios, solos: lo que se reutiliza mientras Rust no cambie ─────
FROM scratch AS binarios
COPY --from=build /b/ /b/

FROM ${BINARIOS} AS bin

# ── 1 · La fina, que no sabe hablar con nadie ───────────────────────────────
FROM scratch AS ore

COPY --from=bin /b/ore            /bin/ore
COPY --from=bin /b/ore-read-jsonl /bin/ore-read-jsonl
# El `PATH` de un `scratch` está vacío, y `ore` busca sus drivers por ahí.
ENV PATH=/bin
USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/bin/ore"]

# ── 2 · La del conjunto ─────────────────────────────────────────────────────
#
# Sobre el SDK de Google Cloud y no sobre `alpine`. Ya no es por `bq`:
# `ore-read-bigquery` habla la API REST él mismo desde A2 (ADR 0042). Lo que
# ata esta base es el aprovisionador (abajo), que necesita `gcloud`; separarlo
# dejaría a los drivers sobre `alpine` con las CA.
#
# Trae además las CA del sistema, que es lo que convierte «hablar TLS» en
# «verificar contra quién»: un binario estático sin ellas se conecta y no sabe
# con quién habla.
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

COPY --from=bin /b/ore                /usr/local/bin/ore
COPY --from=bin /b/ore-read-jsonl     /usr/local/bin/ore-read-jsonl
COPY --from=bin /b/ore-read-postgres  /usr/local/bin/ore-read-postgres
COPY --from=bin /b/ore-read-bigquery  /usr/local/bin/ore-read-bigquery
COPY --from=bin /b/ore-read-s3        /usr/local/bin/ore-read-s3
# La pasarela del Federation Engine (0053 F3): junto a los conectores, que
# lanza como procesos `servir` calientes; los busca en su mismo directorio.
COPY --from=bin /b/ore-federation     /usr/local/bin/ore-federation
COPY --from=bin /b/ore-fetch          /usr/local/bin/ore-fetch
COPY --from=bin /b/ore-log            /usr/local/bin/ore-log
COPY --from=bin /b/ore-sign           /usr/local/bin/ore-sign
COPY --from=bin /b/ore-store-r2       /usr/local/bin/ore-store-r2
COPY --from=bin /b/ore-store-gcs      /usr/local/bin/ore-store-gcs
# El invocador (0029 ⑤): la unica capacidad que anade es hablar con la puerta
# de modelos, con el token de la celda. Vive aqui por lo mismo que el almacen.
COPY --from=bin /b/ore-invoke         /usr/local/bin/ore-invoke

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

COPY --from=bin /b/ore       /usr/local/bin/ore
COPY --from=bin /b/ore-serve /usr/local/bin/ore-serve
# W1 ④ (0030): `POST /vistas/{ns}/{n}/ejecutar` corre `ore ask`, y quien trae
# la copia del bucket es este programa — con la identidad del pod (`objectViewer`,
# aprovisionador ③b) y nunca un origen. `ore` sigue sin abrir un socket.
COPY --from=bin /b/ore-store-gcs /usr/local/bin/ore-store-gcs
# 0046 E9·3: servir un ítem de una colección VIRTUAL es prefirmar su URL en el
# origen con la credencial de la fuente, que este proceso trae del cofre como el
# agente de la celda. Lo hace `ore-firmar-s3`, que NO PUEDE abrir un socket
# (`ore-sigv4`, vigilado en `ore-cli/tests/dependencias.rs`): con la credencial
# en la mano, esta imagen sigue sin nada que lea un origen. `ore-read-s3`, no.
COPY --from=bin /b/ore-firmar-s3 /usr/local/bin/ore-firmar-s3
# 0046 E9b: si la fuente es un ROL del cliente (`role_arn`, sin clave), antes de
# firmar se canjea la identidad de la celda por una credencial temporal en STS.
# `ore-serve` no habla TLS; lo hace `ore-asumir-rol`, que habla con STS y con el
# metadata server y NO enlaza el cliente de S3 ni la firma (vigilado igual):
# canjea un token, no lee un bucket.
COPY --from=bin /b/ore-asumir-rol /usr/local/bin/ore-asumir-rol
# 0049 B2·4: `ore-medios`, el índice de las colecciones, la firma en lote y la
# puerta de lectura, en un proceso vivo. Viaja en esta imagen —alpine con
# certificados, y ya con `ore-store-gcs`— y corre en SU Deployment con su
# comando: es otro proceso, con su sitio en la red, y `ore-serve` sigue sin
# enlazar TLS (`ore-cli/tests/dependencias.rs`). Se compila aparte (arriba).
COPY --from=bin /b/ore-medios /usr/local/bin/ore-medios
# 0050 L6·2·1: la ficha y la búsqueda de una librería (npm, PyPI, Maven Central)
# para el panel Libraries. `ore-serve` no habla TLS: lo lanza, y `ore-packages`
# habla con esos registros y con nadie más (`HOSTS`). No lee buckets ni orígenes
# (vigilado en `ore-cli/tests/dependencias.rs`). La salida a 443 ya la da
# `salida-del-control`.
COPY --from=bin /b/ore-packages /usr/local/bin/ore-packages

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

COPY --from=bin /b/ore-iam /bin/ore-iam

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/bin/ore-iam"]

# ── 5 · El custodio ─────────────────────────────────────────────────────────
#
# ⛔⛔ Y AQUÍ SE PIERDE LA GARANTÍA DE `scratch` A PROPÓSITO, otra vez. Igual que
# `ore-serve` la perdió para tener `git`, éste la pierde para tener `gcloud`:
# **es lo único con lo que habla con el KMS**, y no habla con el KMS — habla con
# él. Era el patrón de `ore-read-bigquery` con `bq`, hasta que BigQuery pasó a
# REST (ADR 0042: allí `bq` perdía valores en silencio; aquí no hay valores).
#
# Lo que se compra cediéndola:
#
#   · ni OAuth ni criptografía en Rust para el KMS;
#   · la autenticación es Workload Identity y la resuelve `gcloud` sola. **No
#     hay una sola llave en el clúster.**
#
# ✏️ 0046 E9·3 (2026-09-30): el ALMACÉN (Secret Manager) ya no pasa por `gcloud`.
#   Arrancarlo costaba 3,6 s por llamada, y resolver un secreto eran dos: ~8 s
#   en cada ítem servido de una colección virtual. Ahora va por su API con
#   `ore-gcp` —el token del metadata server y el TLS de la plataforma, como
#   `ore-store` y `ore-read-bigquery`—: 0,35 s. `gcloud` se queda en la imagen
#   sólo para el KMS de la mudanza (`ore-cofre mudar`).
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

COPY --from=bin /b/ore-cofre /usr/local/bin/ore-cofre

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

# ⭐ ORE 0050 P1 · Python 3.14 (antes 3.12, que sólo recibe parches de seguridad).
#   Medido (P0): las 27 versiones fijadas instalan iguales, las pruebas del SDK
#   21/21, las extensiones del lago y pyright sin errores. El intérprete nombra
#   la capa (`entorno::ABI_PYTHON`, fijado contra esta línea por una prueba).
FROM python:3.14-slim AS puesto-python

# ⭐ ORE 0050 L6·1b · FIJADA: `puesto/python/provisto.txt` dice versión a versión
#   lo que se instala (antes, `pandas pyarrow duckdb google-cloud-storage` sin
#   versión: cada construcción podía traer otra cosa). La misma lista la lee
#   ore-serve para enseñar lo que la sesión trae.
COPY puesto/python/provisto.txt /opt/ore/provisto-fijado.txt
RUN pip install --no-cache-dir -r /opt/ore/provisto-fijado.txt \
 && pip freeze > /entorno-1.txt \
 && python -c "import pandas, pyarrow, duckdb, google.cloud.storage as s; print('entorno 1 ·', pandas.__version__, pyarrow.__version__, duckdb.__version__)"
# ⭐⭐ LO QUE ESTA IMAGEN PONE, ESCRITO PARA QUE OTRO LO LEA (0031 W3.2 · el
#   orden). Es el gemelo exacto de `puesto/jvm/jars.txt`: el Job que resuelve
#   la capa lo lee para NO bajar lo que aquí ya está —el `pyarrow` y el
#   `duckdb` contra los que el SDK está compilado, y todo lo que arrastran—.
#   Sin esto, declarar `pandas` en un repositorio metía otro `numpy` en la
#   capa, y un ABI mal casado no da excepción: mata al intérprete.
#
# ⛔ Y se genera AQUÍ, de la instalación de verdad; y si no es exactamente la
#   lista fijada (L6·1b), la construcción falla: lo que ore-serve enseña y lo
#   que la sesión trae no pueden ser dos cosas.
RUN mkdir -p /opt/ore && pip freeze | sort > /opt/ore/provisto.txt \
 && { grep -v '^#' /opt/ore/provisto-fijado.txt | grep . | sort | diff - /opt/ore/provisto.txt \
      || { echo "✗ pip freeze no es puesto/python/provisto.txt: regenera la lista"; exit 1; }; } \
 && test "$(wc -l < /opt/ore/provisto.txt)" -ge 4 \
 && echo "provisto por la imagen: $(wc -l < /opt/ore/provisto.txt) paquetes" >> /entorno-1.txt
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
COPY --from=bin /b/ore-store-gcs /usr/local/bin/ore-store-gcs
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
#   sale de `python:3.14-slim` y ahí no hay Node. Es lo que cuesta el mejor de
#   los dos, y se paga una vez por imagen, no por sesión.
#
# ⭐ LO QUE ESTO COMPRA, Y QUE EL NAVEGADOR NO PUEDE TENER: aquí están **el SDK**
#   (`/opt/ore/ore`) y **la capa del repositorio** (`/capa`, lo que el árbol
#   declara en su `pyproject.toml`; va al final de `sys.path`, no en el
#   `PYTHONPATH`: la imagen manda, 0037 ③c). Monaco puede colorear; saber
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
RUN node -e "const d=require('@duckdb/node-api');(async()=>{const i=await d.DuckDBInstance.create();const c=await i.connect();await c.run(\"set extension_directory = '/opt/ore/duckdb'\");for(const e of ['iceberg','avro','httpfs','json','icu'])await c.run('install '+e);await c.run('load iceberg');await c.run('load httpfs');console.log('lago ·',d.version(),(await c.runAndReadAll('select extension_name from duckdb_extensions() where loaded')).getColumns()[0].sort().join(' '))})().catch(e=>{console.error(e);process.exit(1)})" \
 && du -sh /opt/ore/duckdb >> /entorno-1.txt
# El SDK al lado del agente (`./ore/index.mjs`, como en `puesto/node/`) y como
# paquete `ore` para las celdas-módulo (un enlace en node_modules). ⛔ Medido en
# victor el 2026-09-19: sólo en node_modules, el agente moría al arrancar con
# ERR_MODULE_NOT_FOUND y `node --check` no lo veía — por eso la comprobación
# de abajo ARRANCA el agente (`--comprobar`: importa, crea el kernel, corre una celda).
COPY puesto/node/agente.mjs /opt/ore/agente.mjs
# La correa del servidor de TypeScript (0050 L3·3): el agente la importa. Y el
# informe con el que corre las pruebas del repositorio (L5, `ore/probar`).
COPY puesto/node/correa.mjs /opt/ore/correa.mjs
COPY puesto/node/informe-de-pruebas.mjs /opt/ore/informe-de-pruebas.mjs
COPY puesto/node/ore        /opt/ore/ore
COPY --from=bin /b/ore-store-gcs /usr/local/bin/ore-store-gcs
RUN ln -s ../ore /opt/ore/node_modules/ore \
 && ORE_CELDAS=/tmp/comprobar node /opt/ore/agente.mjs --comprobar
# ⭐ L3·2 · LO QUE SÓLO TIPA, APARTE: el `tsc`, el servidor de lenguaje y los
#   tipos del Node que corre, en `/opt/ore/tipos` —nunca en el `node_modules`
#   de lo que corre—. Las versiones, las de `provisto.txt` (las líneas con
#   ` tipos`): si no coinciden, la construcción falla aquí. 29 MB (medido el
#   2026-10-03), sin scripts de instalación.
RUN mkdir -p /opt/ore/tipos && cd /opt/ore/tipos && echo '{"private":true}' > package.json \
 && npm install --no-audit --no-fund --ignore-scripts $(grep ' tipos$' /opt/ore/provisto.txt | cut -d' ' -f1) \
 && for l in $(grep ' tipos$' /opt/ore/provisto.txt | cut -d' ' -f1); do n="${l%@*}"; v="${l##*@}"; \
      [ "$(node -p "require('/opt/ore/tipos/node_modules/$n/package.json').version")" = "$v" ] \
        || { echo "✗ provisto.txt dice $n@$v y la imagen trae otra"; exit 1; }; done \
 && echo "tipos · $(node node_modules/typescript/bin/tsc -v) · typescript-language-server $(node node_modules/typescript-language-server/lib/cli.mjs --version) · $(du -sm node_modules | cut -f1) MB" >> /entorno-1.txt \
 && rm -rf /root/.npm
# Y SE PRUEBA AQUÍ: una función con los tipos de `ore`, su prueba con
# `node:test`, y el `tsconfig` de la plantilla (L1): `tsc` limpio; y un `enum`
# es TS1294 —lo que Node no puede correr, `tsc` lo dice—.
RUN mkdir -p /tmp/t/functions /tmp/t/node_modules/@types && cd /tmp/t \
 && ln -s /opt/ore/ore node_modules/ore \
 && ln -s /opt/ore/tipos/node_modules/@types/node node_modules/@types/node \
 && ln -s /opt/ore/tipos/node_modules/undici-types node_modules/undici-types \
 && printf '%s\n' '{"private":true,"type":"module"}' > package.json \
 && printf '%s\n' '{"compilerOptions":{"target":"esnext","module":"nodenext","strict":true,"noEmit":true,"erasableSyntaxOnly":true,"verbatimModuleSyntax":true,"allowImportingTsExtensions":true,"skipLibCheck":true},"include":["**/*.ts"],"exclude":["node_modules"]}' > tsconfig.json \
 && printf '%s\n' 'import type { Decimal } from "ore";' 'export default function f(a: Decimal<12, 2>): string { return a; }' > functions/f.ts \
 && printf '%s\n' 'import { test } from "node:test";' 'import assert from "node:assert/strict";' 'import f from "./f.ts";' 'test("f", () => assert.equal(f("1.00"), "1.00"));' > functions/f.test.ts \
 && node /opt/ore/tipos/node_modules/typescript/bin/tsc -p . \
 && node --test > /dev/null \
 && printf '%s\n' 'enum A { x }' 'export {};' > functions/malo.ts \
 && ! node /opt/ore/tipos/node_modules/typescript/bin/tsc -p . > salida.txt \
 && grep -q TS1294 salida.txt \
 && cd / && rm -rf /tmp/t
# ⭐ L3·3 · Y LA CORREA CON SU SERVIDOR DE VERDAD: un `ore-serve` de mentira
#   (la ficha, el índice, los ficheros, el flujo) contra el
#   `typescript-language-server` de `/opt/ore/tipos`: el repositorio llega al
#   disco, el `enum` es TS1294 (sólo con su `tsconfig` en disco), el vecino se
#   resuelve, `Decimal` es el de `ore`, y lo que se edita se refleja. Con el
#   contrato de Node (R3 T2), las pruebas de `puesto/node/pruebas`.
# `--test-reporter=tap`: desde Node 23 el informe por defecto es `spec` también
#   sin terminal (`ℹ pass 5`), y el `grep` de abajo busca el de TAP (`# pass 5`).
COPY puesto/node/pruebas /opt/ore/pruebas
RUN node --test --test-reporter=tap "/opt/ore/pruebas/*.test.mjs" > /tmp/pruebas.txt 2>&1 \
      || { cat /tmp/pruebas.txt; exit 1; } \
 && grep -E '^# (pass|fail)' /tmp/pruebas.txt >> /entorno-1.txt \
 && ! grep -q '^# skipped [1-9]' /tmp/pruebas.txt \
 && rm -rf /opt/ore/pruebas /tmp/pruebas.txt

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
COPY --from=bin /b/ore-store-gcs /usr/local/bin/ore-store-gcs
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
# ⭐ Y EL REPARTO DEL POD SE DECIDE, no se hereda. Medido
#   (`medida-la-celda-que-no-cabe.py` §1): en un contenedor de 4 GiB la JVM se
#   queda por defecto con el 25% —989 MB— y las otras tres cuartas partes no
#   las reclama nadie explícitamente. Aquí se dice la mitad, que es donde vive
#   lo que `over()` materializa; Arrow y DuckDB tienen lo suyo con su propio
#   tope (`Ore.tropoMb`), y el resto queda para metaspace, hilos y el JDK.
CMD ["java", "-XX:+UseSerialGC", "-XX:MaxRAMPercentage=50", "--add-opens=java.base/java.nio=ALL-UNNAMED", "-cp", "/opt/ore/clases:/opt/ore/lib/*:/capa/*", "ore.Agente"]

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
    ! ls /tmp/p/t/jars | grep -q jackson || exit 1; \
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
    ! ls /tmp/q/t/jars | grep -q 'jackson-databind' || exit 1; \
    grep -q 'jackson-databind 2.19.0 lo arrastra algo que declaraste' /tmp/q/t/informe.json; \
    cat /tmp/q/t/informe.json >> /capa-jvm.txt; rm -rf /tmp/q

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/opt/ore/resolver.sh"]

# ═══════════════════════════════════════════════════════════════════════════
# Etapa 9b · capa-node:1 — QUIEN RESUELVE LA CAPA DE NODE (0050 R3 T5b)
#
# El gemelo de `capa-jvm:1` para npm, por lo mismo: aparte, porque alcanza el
# registro de npm y no tiene ni el testigo de la forja ni credencial de la
# nube. La misma base que `puesto-node:1`, para que lo que se resuelve y lo que
# lo corre sean el mismo Node.
#
# ⛔ `npm install --ignore-scripts` (la decisión de T5b): ningún paquete
#   ejecuta su código al instalarse. Lo que compila nativo no funcionará, y el
#   informe lo dice.
# ═══════════════════════════════════════════════════════════════════════════
FROM node:24-slim AS capa-node

COPY puesto/node/provisto.txt /opt/ore/provisto.txt
COPY puesto/node/capa/capa.mjs /opt/ore/capa.mjs
COPY puesto/node/capa/resolver.sh /opt/ore/resolver.sh
RUN chmod 0755 /opt/ore/resolver.sh \
 && echo "node $(node -v) · npm $(npm -v) · $(grep -vc '^#' /opt/ore/provisto.txt) paquete(s) provistos por el puesto" > /capa-node.txt

# ── ⭐ Y SE PRUEBA AQUÍ, donde hay alguien mirando ─────────────────────────
#
# Un repositorio que declara `dayjs` (se instala) y OTRA `@duckdb/node-api`
# (no se copia: manda el contenedor, y el aviso sale en el informe), más lo
# que no se honra (`devDependencies`, lo local). Medido así en local el
# 2026-10-03 (Node 22, npm): la misma resolución da la misma caja, byte a byte.
# Y (L2) el lock para el repositorio: el de npm, sin el nombre de la caja.
RUN set -e; mkdir -p /tmp/p/arbol; \
    printf '%s' '{"dependencies":{"dayjs":"1.11.13","@duckdb/node-api":"1.4.0","util":"file:../u"},"devDependencies":{"typescript":"5.8.3","@types/lodash":"4.17.20"}}' \
      > /tmp/p/arbol/package.json; \
    TRABAJO=/tmp/p/t /opt/ore/resolver.sh /tmp/p/arbol ""; \
    tar -tzf /tmp/p/t/capa.tgz | grep -qx 'node_modules/dayjs/package.json'; \
    ! tar -tzf /tmp/p/t/capa.tgz | grep -q -e '@duckdb' -e 'typescript' -e '@types' || exit 1; \
    tar -tzf /tmp/p/t/tipos.tgz | grep -qx 'node_modules/@types/lodash/package.json'; \
    ! tar -tzf /tmp/p/t/tipos.tgz | grep -q -e 'dayjs' -e 'typescript/' || exit 1; \
    grep -q '"estado": "lista"' /tmp/p/t/informe.json; \
    grep -q '"dayjs@1.11.13"' /tmp/p/t/informe.json; \
    grep -q '"dev:@types/lodash@4.17.20"' /tmp/p/t/informe.json; \
    grep -q 'pediste @duckdb/node-api 1.4.0, y esta sesión trae la 1.5.5-r.5' /tmp/p/t/informe.json; \
    grep -q 'pediste typescript 5.8.3, y esta sesión trae la 5.9.3' /tmp/p/t/informe.json; \
    grep -q '"suma"' /tmp/p/t/informe.json; \
    grep -q '"sumaTipos"' /tmp/p/t/informe.json; \
    grep -q '"node_modules/dayjs"' /tmp/p/t/lock-del-repositorio.json; \
    grep -A4 '"node_modules/@types/lodash"' /tmp/p/t/lock-del-repositorio.json | grep -q '"dev": true'; \
    ! grep -q -e '"capa"' -e '@duckdb' /tmp/p/t/lock-del-repositorio.json || exit 1; \
    S1=$(sha256sum /tmp/p/t/capa.tgz /tmp/p/t/tipos.tgz | cut -c1-64); rm -rf /tmp/p/t; \
    TRABAJO=/tmp/p/t /opt/ore/resolver.sh /tmp/p/arbol "" >/dev/null; \
    [ "$S1" = "$(sha256sum /tmp/p/t/capa.tgz /tmp/p/t/tipos.tgz | cut -c1-64)" ]; \
    cat /tmp/p/t/informe.json >> /capa-node.txt; rm -rf /tmp/p /root/.npm

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/opt/ore/resolver.sh"]

# ── 5 · El IdP (0048 I2) ─────────────────────────────────────────────────────
#
# Keycloak 26.0.7, COCIDO. Vino de la plataforma (`C:\Rubix\idp\Dockerfile`) tal cual,
# con el tema del correo dentro. Sus razones, resumidas —el texto entero está en su
# historial y en `malla/60-idp.yaml`—:
#
#   · `kc.sh build` en la construcción: el sistema de ficheros puede ser de sólo
#     lectura, cada arranque ahorra ~40 s y la etiqueta es NUESTRA;
#   · las cinco opciones de BUILD se leyeron del StatefulSet que el operador tenía en
#     pie (KC_DB, KC_CACHE, KC_CACHE_STACK, KC_HEALTH_ENABLED, KC_METRICS_ENABLED): con
#     `startOptimized: true` el operador ya no puede cambiarlas, y un juego distinto
#     arranca EN VERDE con otra configuración;
#   · la imagen y el CR quedan ACOPLADOS: cambiar `db.vendor`, el caché o las métricas
#     en el CR sin recocer es fallar en verde.
#
# ⚠️ CI la construye (`idp:main`, `idp:<sha>`), pero la que corre la fija
#   `malla/60-idp.yaml`: cambiarla reinicia el login, y se decide aparte.

FROM quay.io/keycloak/keycloak:26.0.7 AS idp-constructor

# ⭐ Las cinco de BUILD, y sólo ésas. Cada una está justificada arriba contra lo que el
#   operador ya tenía puesto — no contra lo que parecería razonable.
ENV KC_DB=postgres
ENV KC_CACHE=ispn
ENV KC_CACHE_STACK=kubernetes
ENV KC_HEALTH_ENABLED=true
ENV KC_METRICS_ENABLED=true

# ⛔ ÉSTE es el paso entero. Todo lo que este `RUN` escriba en `/opt/keycloak` es lo que
#   deja de escribirse en cada arranque — y por eso el sistema de ficheros puede volver a
#   ser de sólo lectura al otro lado.
RUN /opt/keycloak/bin/kc.sh build

# ── La imagen final: la misma base, con el build ya dentro ──────────
#
# ⚠️ Se parte OTRA VEZ de la imagen oficial en vez de seguir en la del constructor: así lo
#    que se envía no arrastra nada de lo que el build necesitó para correr.
FROM quay.io/keycloak/keycloak:26.0.7 AS idp

COPY --from=idp-constructor /opt/keycloak/ /opt/keycloak/

# ⭐ 0048 I2 · EL TEMA DEL CORREO, COCIDO. El realm dice `emailTheme: 'rubix'` (la marca
#   y el idioma de la reposición de contraseña, `identidad/realm.mjs`), y en el clúster
#   viejo el tema llegaba por un ConfigMap montado. En ORE no llegaba por ningún sitio: el
#   día que el realm tenga correo, Keycloak no habría encontrado su plantilla. Un tema de
#   CORREO no lleva CSS ni fuentes: es FreeMarker y textos, y no necesita `kc.sh build`.
COPY identidad/tema/rubix /opt/keycloak/themes/rubix

# ⛔ El ENTRYPOINT se repite a propósito. La imagen base ya lo trae, pero un `COPY` sobre
#   `/opt/keycloak` es exactamente el sitio donde un cambio de la base podría dejarlo
#   apuntando a algo que ya no está. Declararlo aquí cuesta una línea.
ENTRYPOINT ["/opt/keycloak/bin/kc.sh"]
