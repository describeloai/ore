FROM maven:3.9-eclipse-temurin-21 AS construir
WORKDIR /src
COPY pom.xml ./
RUN mvn -q -B dependency:copy-dependencies -DoutputDirectory=/out/lib
COPY src ./src
RUN mvn -q -B package -DskipTests && cp target/libro-informes.jar /out/

FROM eclipse-temurin:21-jre
COPY --from=construir /out /app
CMD ["java", "-Dorg.slf4j.simpleLogger.defaultLogLevel=warn", "-cp", "/app/libro-informes.jar:/app/lib/*", "libro.Informes"]
