# ORE 0072 F2·3·2·3 · La etapa `bin` del Dockerfile con sólo los binarios de esta carpeta (los musl de
# `compilar.sh`), para construir en local una imagen sin compilar los treinta:
#   docker build -f binarios-locales.Dockerfile -t ore-bin-local .            (desde esta carpeta)
#   docker build --target piramide --build-arg BINARIOS=ore-bin-local -t ore-piramide:local C:\ORE-pg
FROM scratch
COPY ore-piramide /b/ore-piramide
