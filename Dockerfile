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
#   ore-serve           el plano de control. Lleva `ore` y `git`, y NADA
#                       más: no puede leer un origen —el `ore` que ejecuta es
#                       el mismo binario sin TLS— y sí puede hablar con la
#                       forja, que es donde vive el árbol.
#
#   ore-drivers         todo lo que `ore` puede ejecutar: los tres `ore-read-*`,
#                       `ore-fetch`, `ore-log`, `ore-sign` y `ore-store-r2`,
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
      -p ore-cli -p ore-serve -p ore-iam -p ore-read-jsonl -p ore-read-postgres -p ore-read-bigquery \
      -p ore-fetch -p ore-log -p ore-sign -p ore-store-r2 \
 && for b in ore ore-serve ore-iam ore-read-jsonl ore-read-postgres ore-read-bigquery \
             ore-fetch ore-log ore-sign ore-store-r2; do \
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

COPY --from=build /src/target/release/ore                /usr/local/bin/ore
COPY --from=build /src/target/release/ore-read-jsonl     /usr/local/bin/ore-read-jsonl
COPY --from=build /src/target/release/ore-read-postgres  /usr/local/bin/ore-read-postgres
COPY --from=build /src/target/release/ore-read-bigquery  /usr/local/bin/ore-read-bigquery
COPY --from=build /src/target/release/ore-fetch          /usr/local/bin/ore-fetch
COPY --from=build /src/target/release/ore-log            /usr/local/bin/ore-log
COPY --from=build /src/target/release/ore-sign           /usr/local/bin/ore-sign
COPY --from=build /src/target/release/ore-store-r2       /usr/local/bin/ore-store-r2

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

USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/usr/local/bin/ore-serve"]

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
