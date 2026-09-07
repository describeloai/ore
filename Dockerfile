# La imagen de `ore`, y por qué cabe en `scratch`.
#
# 12 de 14 crates del árbol no abren una conexión: `ore` lee un árbol de
# ficheros y contesta. Compilado contra musl no arrastra ni una dependencia
# dinámica, así que la imagen final **no necesita una distro debajo** — ni
# `libc`, ni certificados, ni un shell.
#
# Va con `ore-read-jsonl` porque es el otro binario que tampoco sale a la red:
# lee ficheros locales. Los dos juntos son la imagen que corre el 90 % de los
# jobs —validar, planificar, `diff`, empaquetar— y la que arranca en un nodo
# Spot que viene de cero, donde el tiempo de descarga es tiempo facturado.
#
# `ore-read-postgres` y `ore-read-bigquery` NO están aquí a propósito: el
# primero enlaza TLS del sistema y el segundo delega en el `bq` del SDK de
# Google Cloud, que son mil megas. La frontera de las imágenes es la misma que
# la del sustrato y la misma que la `NetworkPolicy` de la malla.

FROM rust:1.90-alpine AS build

# `musl-dev` para el enlazador. Nada más: sin OpenSSL, sin pkg-config, sin FFI
# de plataforma — que es exactamente lo que el árbol afirma de sí mismo.
RUN apk add --no-cache musl-dev

WORKDIR /src
COPY . .

# `--locked` para que la imagen se construya con el `Cargo.lock` del árbol y no
# con lo que hubiera hoy en el índice. Y sólo los dos binarios herméticos: pedir
# el workspace entero arrastraría `ore-read-postgres` y su OpenSSL.
RUN cargo build --release --locked \
      -p ore-cli -p ore-read-jsonl \
 && strip target/release/ore target/release/ore-read-jsonl

FROM scratch

# `ore` busca sus drivers como `ore-read-<tipo>` en el PATH, así que los dos
# van al mismo sitio y el PATH por defecto de un `scratch` —vacío— se declara.
COPY --from=build /src/target/release/ore            /bin/ore
COPY --from=build /src/target/release/ore-read-jsonl /bin/ore-read-jsonl
ENV PATH=/bin

# Sin usuario declarado, `scratch` corre como root. Aquí no hay nada que
# escalar —no hay shell ni utilidades— pero el número es lo que la malla mira
# para su `PodSecurity`, así que se dice.
USER 65532:65532

WORKDIR /trabajo
ENTRYPOINT ["/bin/ore"]
