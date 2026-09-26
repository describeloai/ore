# Pruebas de fuego

**Enfrentar lo que emitimos a la implementación de referencia ajena, y lo que
planificamos a una fuente de verdad.**

No están en la suite de Rust y no es por comodidad: **necesitan cosas que la suite
no puede tener**. Una necesita Node y el paquete `graphql`; la otra, un PostgreSQL
en marcha. Meterlas dentro convertiría `cargo test` en algo que no se puede
ejecutar sin red, y eso es justo lo que el compilador no es.

Lo que sí tienen que hacer es **correr solas**. Hasta hoy se ejecutaban a mano:

> **Una prueba que no corre tiene exactamente el mismo aspecto que una que pasa.**

## Qué encontró cada una

**`graphql.mjs`** enfrenta el SDL que emitimos a `graphql-js`, la implementación de
referencia. Encontró defectos que llevaban versiones ahí, y ninguno se veía leyendo:
un esquema puede estar *bien escrito* y ser inválido, y solo lo dice quien lo tiene
que consumir.

**`descubrimiento.sh`** tira del **eslabón vivo** de `discover`. `--from` estaba
cubierto por once pruebas y `--source` por ninguna: el que resuelve
`ore-read-postgres` en el `PATH`, lo ejecuta, le pasa la URL por stdin y analiza
lo que devuelve se podía romper en silencio por cualquiera de los tres sitios. Lo
que comprueba no es que el comando termine, sino **las decisiones que salen** de
un esquema sucio — que contestarlas deja un paquete que `ore validate` acepta, y
que decidir un concepto **quita el campo de la superficie emitida**: se contesta
que `email` es `gdpr.personalEmail` —`high`—, el techo del conducto admite hasta
`medium`, y el campo no está en el SDL. Nadie escribió una etiqueta en una
entidad.

**`fuentes-reales.sh`** ejecuta contra un PostgreSQL real lo que se midió a mano al
construir L2 — el driver, el índice, el refresco y una consulta que cruza dos
familias de fuente. Cada medición de esas era un `echo` en una terminal; aquí es una
aserción.

**`bigquery-real.sh`** lleva BigQuery de punta a punta —catálogo, `discover
--type standard`, `materialize` a Iceberg y la relectura— contra un dataset de
verdad con la semilla de `semilla/bigquery-ventas.sql`, y compara la copia **valor
a valor** con lo sembrado. Nació en rojo a propósito (2026-09-26): afirma lo que la
Fase A tiene que conseguir, y cada aserción en rojo lleva la etiqueta del paso que
la apaga (`[A2]` el transporte REST, `[A3]` el catálogo en el driver, `[A4]` el
contrato de tipos). La línea base medida: 11 en rojo —el texto `'null'` se vuelve
NULL, los microsegundos se pierden, un TIMESTAMP se queda como texto, NUMERIC cae a
`decimal(38, 18)`, `REQUIRED` no llega a Iceberg— y 12 en verde. Tras A2 (REST): 4 en rojo, 1 de A3 y 3 de A4. Tras A3 (el catálogo en el driver): 3, las de A4. **CI no la corre**:
no alcanza BigQuery, y sin `BQ_URL` la prueba se salta diciéndolo. Los tests del
driver leen en su lugar respuestas grabadas de verdad
(`crates/ore-read-bigquery/tests/rest/`, que regenera `grabar-bigquery-rest.py`).

Y hay una que **ya no vive aquí**: la de Cedar. `ore-exec/tests/prueba_de_fuego.rs`
es Rust y entra en `cargo test --workspace` como cualquier otra, porque `cedar-policy`
es una dependencia y no un servicio.

## Cómo se ejecutan

```bash
pruebas-de-fuego/graphql.sh          # necesita node
pruebas-de-fuego/fuentes-reales.sh   # necesita docker
pruebas-de-fuego/descubrimiento.sh   # necesita docker, y `ore` en el PATH
BQ_URL=bigquery://<proyecto>/ventas pruebas-de-fuego/bigquery-real.sh   # a mano: gcloud y la semilla
```

Las tres las corre `ci.yml` en cada empujón, que es el punto. Y cada una tiene
**base de datos propia**: el lector lee todos los esquemas, así que las tablas que
deja una prueba serían entidades de la siguiente.
