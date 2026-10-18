# ORE 0068 C2·2 pieza 0 · EL PREVIO de la imagen pesada: un prototipo, no la etapa.
#   Lo que mide (con `sonda.py`, sin red): Spark 4.1 en Java 21 (el laboratorio de C2·4 iba en el 17),
#   PySpark y Sedona en Python 3.14 (el del puesto de Python), el tamaño, y que todo esté DENTRO.
#
# ⭐ Variante A: el Python del puesto (python:3.14-slim) + el JDK de Java Transforms
#   (temurin 21, copiado) + Spark por `pip install pyspark`, que trae los jars de Spark.
#   Si A no da, la B es el tarball de Apache sobre temurin.
FROM eclipse-temurin:21-jdk-noble AS jdk

FROM python:3.14-slim AS a
ARG SPARK=4.1.0
ARG ICEBERG=1.12.0
ARG SEDONA=1.9.1
ARG GEOTOOLS=1.9.1-33.5
ENV JAVA_HOME=/opt/java/openjdk PATH=/opt/java/openjdk/bin:$PATH
COPY --from=jdk /opt/java/openjdk /opt/java/openjdk
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends curl procps >/dev/null && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir -q "pyspark==${SPARK}" "apache-sedona==${SEDONA}"
RUN J="$(python -c 'import pyspark, os; print(os.path.join(os.path.dirname(pyspark.__file__), "jars"))')" \
 && M=https://repo1.maven.org/maven2 \
 && for a in \
      "org/apache/iceberg/iceberg-spark-runtime-4.1_2.13/${ICEBERG}/iceberg-spark-runtime-4.1_2.13-${ICEBERG}.jar" \
      "org/apache/iceberg/iceberg-gcp-bundle/${ICEBERG}/iceberg-gcp-bundle-${ICEBERG}.jar" \
      "org/apache/iceberg/iceberg-aws-bundle/${ICEBERG}/iceberg-aws-bundle-${ICEBERG}.jar" \
      "org/apache/sedona/sedona-spark-shaded-4.1_2.13/${SEDONA}/sedona-spark-shaded-4.1_2.13-${SEDONA}.jar" \
      "org/datasyslab/geotools-wrapper/${GEOTOOLS}/geotools-wrapper-${GEOTOOLS}.jar"; do \
      curl -fsSL --retry 3 -o "$J/${a##*/}" "$M/$a" || exit 1; done \
 && echo "$J" > /spark-jars.txt
# ⚠️ Medido (pieza 0): el `UnixLoginModule` de Hadoop pide el NOMBRE del uid, y 65532 (el de todos
#   los puestos) no tiene: `LoginException … invalid null input: name`. `HADOOP_USER_NAME` no basta.
RUN echo 'ore:x:65532:65532:ore:/tmp:/usr/sbin/nologin' >> /etc/passwd \
 && echo 'ore:x:65532:' >> /etc/group
# ⚠️ Medido (pieza 0): Sedona en Python (`createDataFrame` con shapely) pide pandas y no lo instala.
#   Y pyarrow: `toPandas` por Arrow y el SDK de ORE.
RUN pip install --no-cache-dir -q pandas pyarrow
COPY sonda.py /sonda.py
USER 65532:65532
WORKDIR /tmp
