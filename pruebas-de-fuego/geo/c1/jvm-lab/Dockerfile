# ORE 0068 C1·3 · La imagen del puesto de la JVM para el laboratorio: las líneas 611–628 del Dockerfile
# (DuckDB JDBC, jars.txt, las extensiones y el SDK compilado) sin ore-store-gcs (obligaría a compilar Rust).
#   docker build -f pruebas-de-fuego/geo/c1/jvm-lab/Dockerfile [--build-arg DUCKDB_JDBC=1.5.6.0] -t ore-puesto-jvm:local .
FROM eclipse-temurin:21-jdk-noble AS puesto-jvm

ARG DUCKDB_JDBC=1.5.6.0
COPY puesto/jvm /opt/ore/src
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

