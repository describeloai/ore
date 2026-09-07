# Las imágenes de `ore`.
#
# Los drivers son binarios SEPARADOS por decisión —ADR 0008: `ore` los busca
# como `ore-read-<tipo>` en el `PATH` y habla con ellos por stdin/stdout, así
# que el motor no enlaza un cliente de nube y un driver lo puede escribir
# cualquiera en cualquier lenguaje—. **Eso no obliga a imágenes separadas**:
# los diez binarios cabrían en una. Lo que las separa es el peso de lo que cada
# uno arrastra de fuera, y sólo importa porque el pool de jobs arranca DESDE
# CERO: en un nodo nuevo, el tamaño de la imagen es tiempo facturado.
#
#   ore        `ore` + `ore-read-jsonl`   nada de fuera        ~7 MB
#   postgres   + `ore-read-postgres`      TLS del sistema      ~15 MB
#   bigquery   + `ore-read-bigquery`      el `bq` del SDK      ~110 MB de descarga
#
# La frontera es la misma que la del sustrato: 12 de 14 crates no abren una
# conexión, y la misma que usa la `NetworkPolicy` de la malla.

# ── Compilación, una sola vez para todas ────────────────────────────────────
FROM rust:1.90-alpine AS build

# `musl-dev` para el enlazador; `openssl-dev` y `openssl-libs-static` sólo los
# necesita `ore-read-postgres`, que enlaza `native-tls`. El resto del árbol no
# arrastra FFI, y eso es lo que afirma de sí mismo.
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig

WORKDIR /src
COPY . .

# `--locked`: la imagen se construye con el `Cargo.lock` del árbol y no con lo
# que hubiera hoy en el índice.
RUN cargo build --release --locked \
      -p ore-cli -p ore-read-jsonl -p ore-read-postgres \
 && strip target/release/ore \
          target/release/ore-read-jsonl \
          target/release/ore-read-postgres

# ── La imagen fina: lo que no sale a la red ─────────────────────────────────
FROM scratch AS ore

# `ore` busca sus drivers en el `PATH`, así que los dos van al mismo sitio y el
# `PATH` —vacío en un `scratch`— se declara.
COPY --from=build /src/target/release/ore            /bin/ore
COPY --from=build /src/target/release/ore-read-jsonl /bin/ore-read-jsonl
ENV PATH=/bin
USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/bin/ore"]

# ── La imagen con Postgres ──────────────────────────────────────────────────
#
# No es `scratch` por una sola razón: hablar TLS con un servidor exige VERIFICAR
# su certificado, y para eso hacen falta las CA del sistema. Un binario estático
# sin ellas se conecta y no puede decir contra quién.
FROM alpine:3.22 AS postgres

RUN apk add --no-cache ca-certificates
COPY --from=build /src/target/release/ore                /bin/ore
COPY --from=build /src/target/release/ore-read-jsonl     /bin/ore-read-jsonl
COPY --from=build /src/target/release/ore-read-postgres  /bin/ore-read-postgres
USER 65532:65532
WORKDIR /trabajo
ENTRYPOINT ["/bin/ore"]
