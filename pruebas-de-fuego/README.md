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
verdad con la semilla de `bigquery-semilla.sql`, y compara la copia **valor
a valor** con lo sembrado. Nació en rojo a propósito (2026-09-26): afirma lo que la
Fase A tiene que conseguir, y cada aserción en rojo lleva la etiqueta del paso que
la apaga (`[A2]` el transporte REST, `[A3]` el catálogo en el driver, `[A4]` el
contrato de tipos). La línea base medida: 11 en rojo —el texto `'null'` se vuelve
NULL, los microsegundos se pierden, un TIMESTAMP se queda como texto, NUMERIC cae a
`decimal(38, 18)`, `REQUIRED` no llega a Iceberg— y 12 en verde. Tras A2 (REST): 4 en rojo, 1 de A3 y 3 de A4. Tras A3 (el catálogo en el driver): 3, las de A4. Tras A4 (`Decimal<p, s>`): todo en verde, y `required` en Iceberg se dice como [B], pendiente de la Fase B. **CI no la corre**:
no alcanza BigQuery, y sin `BQ_URL` la prueba se salta diciéndolo. Los tests del
driver leen en su lugar respuestas grabadas de verdad
(`crates/ore-read-bigquery/tests/rest/`, que regenera `grabar-bigquery-rest.py`).

**`mongodb.sh`** lleva MongoDB de punta a punta contra un **Atlas de verdad** y con el usuario de
**solo lectura** —alta, `discover`, una copia calculada en DuckDB, la misma por la pasarela— y rompe
cada protección a propósito para ver que muerde: TLS apagado por URL (las tres formas), una contraseña
que Atlas rechaza, un operador fuera del vocabulario, una columna de tipos mezclados que no se filtra
como SQL, un tipo declarado que el dato no lleva, el tiempo cortado en el servidor, una escritura
con el rol de la fuente, un changelog que se intenta filtrar, y que ninguna orden escriba la
contraseña. Lo que se compara con el origen lo dice **otro cliente** (`mongodb/origen.py`, pymongo):
una copia que casa porque el conector se compara consigo mismo no demuestra nada. Con `MONGO_URL` (un
replica set local que escribe) corre además el kit de conformidad, el kit por la pasarela y los
cambios de verdad; con `MONGO_CADUCAR=1`, trunca su oplog para ver una posición **caducada** de
verdad. `SOLO=4` corre sólo las protecciones (un minuto). Y la propia prueba se prueba: sabotear una
protección del conector la pone roja en la comprobación que debía (ficha 0069, M6).

**`cassandra.sh`** lleva Cassandra de punta a punta contra un **Cassandra de verdad** (un `cassandra:5`
con autenticación) y con un rol de **solo lectura** que el propio script crea con `SELECT` sobre un solo
keyspace: alta, `discover`, dos copias calculadas en DuckDB (los pedidos de un cliente, con la partición y el
rango de clustering **empujados** al servidor, y una tabla de 150 000 filas por rangos de token), la misma por la
pasarela —que **descubre** el conector de su carpeta, sin `--tipos`— y el kit de conformidad. Rompe cada
protección a propósito: la verificación de TLS apagada (cuatro formas), una contraseña que Cassandra rechaza (y
entrar sin ninguna), un servidor al que no se llega, un operador fuera del vocabulario, bytes, colecciones y
duraciones que no se filtran como SQL, un tipo declarado que el dato no lleva (y un decimal que perdería
cifras), el tiempo cortado en el cliente, una escritura con el rol de la fuente, un `orderBy`, y **un keyspace
al que el rol no llega**: no se lee, no se cataloga y no se explora aunque `system_schema` lo liste. Y lo que
Cassandra no sabe filtrar (`<>`, `IS NULL`, `LIKE`) lo filtra el conector y **casa con lo que cuenta otro
cliente** (`cassandra/origen.py`, el driver de DataStax para Python), que es lo único que hace creíble la
comparación. Además: la salida es byte a byte la misma en dos lecturas (cuatro rangos leen a la vez), una
partición solo-estática no sale como fila, y ninguna orden escribe la contraseña. Necesita
`pip install cassandra-driver` (y `pyasyncore pyasynchat` desde Python 3.12). `SOLO=4` corre sólo las
protecciones (20 segundos); la semilla (24 000 pedidos y 150 000 lecturas) tarda ~2 minutos la primera vez.
La sección 6 prueba el **TLS de verdad** contra un `cassandra:5` con certificado propio (`bash pruebas-de-fuego/cassandra/tls-lab.sh`
lo levanta y da `CASSANDRA_TLS_ADMIN_URL` y `CASSANDRA_TLS_CA`): se acepta por la IP y por el nombre, **no** se acepta otra CA
—y se dice por qué y que no se envió la credencial—, y un cliente sin TLS contra un servidor que lo exige sabe qué probar.
Y la propia prueba se prueba: sabotear una protección del conector la pone roja en la comprobación que debía
(ficha 0070, M6).

**`phoenix/`** es el laboratorio de Apache Phoenix (ficha 0071): `Dockerfile` (HBase + Phoenix + el Phoenix Query
Server, que no tiene imagen oficial: `docker build -t ore-phoenix-lab pruebas-de-fuego/phoenix` y
`docker run -d --name phx -p 8765:8765 ore-phoenix-lab`), `semilla.py` (todos los tipos, tabla con sal, vista,
`ROW_TIMESTAMP`, multi-tenant, esquema ajeno y 262 144 pedidos, ~3 minutos) y `trazas.py`, que graba los bytes del
cable (`trazas/`, los fixtures de `ore-avatica` y de `ore-phoenix`). **`phoenix/pasarela.py`** prueba el conector
**como binario** y por la **pasarela de verdad** contra ese laboratorio, sin fiarse de él: abre el flujo Arrow
con `pyarrow` y cuenta (262 144 filas, ni una de más, en orden), rompe cada protección a propósito (un operador que no existe, un `like` con barra, una tabla o una columna que no existe, un servidor al que no se llega, `verify=false`, una contraseña por HTTP en
claro) y comprueba que **la contraseña no sale** ni del binario ni de la pasarela; la pasarela **descubre** el
conector de su carpeta, el presupuesto de filas de la pasarela corta una lectura larga, cuatro lecturas a la vez
salen enteras, y `DELETE /v1/read/{id}` corta una a mitad. Necesita `pip install pyarrow` y los binarios
(`ORE_READ_PHOENIX`, `ORE_FEDERATION`). **El kit de conformidad** corre contra el mismo laboratorio y carga su propia semilla (10⁶ filas duplicadas dentro de Phoenix, ~70 s la primera vez): `PHOENIX_KIT_URL=phoenix://127.0.0.1:8765 ore-kit --conector target/release/ore-read-phoenix --banco phoenix --exige todos` (con `--features ore-conector-kit/phoenix`; 11 pasan, 3 no aplican).

Y hay una que **ya no vive aquí**: la de Cedar. `ore-exec/tests/prueba_de_fuego.rs`
es Rust y entra en `cargo test --workspace` como cualquier otra, porque `cedar-policy`
es una dependencia y no un servicio.

## Cómo se ejecutan

```bash
pruebas-de-fuego/graphql.sh          # necesita node
pruebas-de-fuego/fuentes-reales.sh   # necesita docker
pruebas-de-fuego/descubrimiento.sh   # necesita docker, y `ore` en el PATH
BQ_URL=bigquery://<proyecto>/ventas pruebas-de-fuego/bigquery-real.sh   # a mano: gcloud y la semilla
MONGO_ATLAS_URL=mongodb+srv://<lector>:<clave>@<cluster>/ore_prueba pruebas-de-fuego/mongodb.sh   # a mano: un Atlas de desarrollo, usuario de solo lectura
```

Las tres las corre `ci.yml` en cada empujón, que es el punto. Y cada una tiene
**base de datos propia**: el lector lee todos los esquemas, así que las tablas que
deja una prueba serían entidades de la siguiente.
