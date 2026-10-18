# 0068 · Geo compute — calcular con el espacio

**Estado:** propuesto (2026-10-18). **Hoja de ruta mientras se desarrolla; al cerrarse, la imagen** de
lo que es el cómputo geo en ORE. Nace del bastión, [`0066 · Geo paradigm`](0066-geo-paradigm.md)
(línea E1, que se desarrolla aquí), y se apoya en [`0067 · Geo formats`](0067-geo-formats.md), cuyo
sustrato da por hecho: el tipo, el lago, los códecs y los SDK que leen geometría. Toca
[`0040`](0040-sql-views.md) (la vista es SQL), [`0050`](0050-functions-in-code-repositories.md)
(Functions), [`0055`](0055-transforms-in-code-repositories.md) (Transforms) y [`0047`](0047-ore-access-control.md) (acceso).
Fuera: servir y el mapa (E2), lo geotemporal (E3), el ráster (E4), los datos de referencia de serie
(E5) y empujar el cómputo espacial al origen (federación).

## 1. Qué es

Todo lo que **calcula con el espacio** sobre los datos del lago:

| familia | ejemplos |
|---|---|
| medir | área, longitud, distancia |
| relacionar | intersecta, contiene, a menos de X metros |
| construir | buffer, unión, centroide, simplificar |
| cambiar de CRS | reproyectar, siempre explícito |
| indexar en malla | H3: las celdas como claves |
| unir y agregar por espacio | puntos en polígonos, conteo por zona o por celda, el más cercano |
| enriquecer | añadir a unas entidades lo que sale de dónde están |

## 2. La visión del producto

1. **El cliente elige su motor al crear el repositorio**, en un banner, como en Foundry:
   **Lightweight** (DuckDB en un nodo, para lo interactivo y lo mediano) o **Heavyweight** (Spark 4.1,
   para lo distribuido y lo masivo). Una decisión sencilla, que no se cambia a mitad.
2. **El mismo contrato en los dos motores.** Lo que se calcula lleva tipos con CRS, linaje, permisos y
   frescura, lo calcule quien lo calcule. Eso es de ORE, no del motor: el motor calcula y ORE
   garantiza el contrato.
3. **El CRS no se pierde ni se inventa.** Va en el tipo de cada columna, se comprueba al compilar
   (mezclar dos es un error) y reproyectar es explícito. Ningún motor lo garantiza a través de vistas,
   linaje y permisos: es la diferencia de ORE.
4. **El *join* espacial no es un kind: es una operación.** Vive donde vive el SQL o el código: una View
   si es ligero, un Dataset materializado por un Transform o un Build si es pesado, una Function, o la
   sesión.
5. **Enriquecer es un verbo de primera clase.** «Enriquece *X* con *Y* por una relación espacial,
   tomando estas columnas o estos agregados»: ORE genera el cómputo para el motor del repositorio,
   comprueba los CRS, propaga los permisos de *Y* y deja un Dataset nuevo con linaje a los dos y su
   frescura.

### Las capas

| capa | qué | de quién | hito |
|---|---|---|---|
| **Producto** | el banner Lightweight/Heavyweight · enriquecer (consola y código) · el contrato del resultado en Assets | ORE | C2, C4 |
| **Contrato** | tipos con CRS en Views, Datasets, Transforms y Functions · el compilador niega mezclar CRS · linaje, permisos, frescura | ORE (OOS y compilador) | C1 |
| **Frameworks** | SQL (Views, `ore-sql`, Functions desde SQL) · Python (GeoDataFrame, shapely) · Java (JTS) · Spark (lo que escribe un Transform pesado) | ORE sobre librerías estándar | C1·1–C1·4 |
| **Motores** | DuckDB 1.5 con `spatial` y `h3` (Lightweight) · Spark 4.1 con Sedona 1.9.1 (Heavyweight; C0·0) | los motores | C0, C2, C3 |
| **Sustrato** | Iceberg v3 y GeoParquet, WKB, orden Hilbert, grupos de 100 000 filas · drivers con geometría tipada | ORE (0067, hecho) | — |

### Cómo fluye un cómputo

```
 Repositorio (motor elegido al crearlo: Lightweight | Heavyweight)
   │
   ├─ View (SQL) ──────────────► DuckDB la describe ─► contrato con CRS ─► compilador
   │                                                   (ore-serve pone el tipo exacto
   │                                                    de lo que pasa tal cual)
   ├─ Transform / Build ───────► el motor del repositorio ─► Dataset con contrato,
   │    (join pesado, H3)          DuckDB o Spark              linaje y frescura
   ├─ Function (Python, Java) ─► shapely, geopandas, JTS ─► firma tipada con CRS
   │
   └─ Enriquecer X con Y ──────► ORE genera el cómputo para el motor, comprueba
        (relación + columnas      los CRS y propaga los permisos de Y ─► un Dataset
         o agregados)             nuevo con linaje a X y a Y
```

### Enriquecer, en concreto

| tienes | contra | obtienes |
|---|---|---|
| puntos (clientes) | polígonos (municipios, códigos postales) | en qué municipio cae cada uno, y sus atributos |
| puntos (activos) | polígonos de riesgo (inundación) | si está en zona de riesgo, y de qué nivel |
| puntos (tiendas) | puntos (competidores) | cuántos a menos de 500 m, y el más cercano |
| polígonos (zonas de venta) | puntos o polígonos (población, edificios) | lo agregado por zona, también ponderado por área |
| celdas H3 | cualquier capa | una malla con variables, base de análisis y de mapas |

La relación: contiene, intersecta, a distancia, el más cercano. Lo que se toma: columnas si hay un
solo match, agregados si hay varios. Sale un Dataset nuevo; las filas de *X* no se tocan.

## 3. Lo que sabemos

- **El sustrato** (0067): `Geometry<subtipo, CRS>` y `Geography` en OOS v1alpha32, con el valor en
  WKB. El lago, ordenado por Hilbert, descarta el 98,5–99,4 % de los grupos de filas al leer por zona
  (V4). Los tres SDK leen geometría con su CRS; la imagen trae `spatial`, `h3`, geopandas, shapely y
  pyproj; `sql()` carga `spatial` solo y `ST_Read` lee los demás formatos. Las Functions de Python
  aceptan `Geometry[…]` en la firma (0067 G2·3·6).
- **Qué sabe ORE de los tipos de una función SQL** (medido 2026-10-17, en 0066 E1): el contrato de una
  View lo describe DuckDB en el puesto (`describe` sobre tablas vacías con los tipos del árbol); el
  compilador no deduce tipos de funciones, solo coteja.
- **El contrato geo en las Views** (E1·1 paso 1, `8d9910fd`, en local): `Geometry` con su CRS en el
  catálogo del SDK y de vuelta, v1alpha32 si lleva geo, el tipo exacto de lo que pasa tal cual
  (`vistas::geo_exacto_de_vista`, por el linaje de `vista_sql`) y el compilador mirando ya los tipos
  de las `columns` de una View (antes, solo de `Table` y `Dataset`).
- **DuckDB 1.5** (medido): `GEOMETRY` con CRS en el núcleo; `ST_Buffer`, `ST_Centroid` y `ST_Union_Agg`
  lo conservan, `ST_Area` y `ST_Distance` dan `DOUBLE`, `h3_latlng_to_cell` da `UBIGINT`; mezclar dos
  CRS se niega al describir; `ST_Transform`, `ST_Point` y `ST_GeomFromText` lo pierden. Su
  `SPATIAL_JOIN` (desde 1.3) hace un R-tree sobre las cajas de un lado: según su blog, 58 M de puntos
  contra 310 polígonos en unos 30 s en un portátil (sin medir aquí).
- **Spark 4.1**: tipos `GEOMETRY` y `GEOGRAPHY` (WKB con SRID) y `ST_*` desde la 4.1.0. Ya lo usamos en
  Java Transforms, y escribe Iceberg v3 con geometría (0067 G2·4).
- **El estado del arte** (2025–2026): SedonaDB (Rust, Arrow y DataFusion; la 0.3 trae *join* espacial
  más grande que la memoria y CRS por fila), Apache Sedona 1.9 distribuido, Databricks con `ST_*`
  nativas (`GEOMETRY` disponible, `GEOGRAPHY` en vista previa), BigQuery solo con `GEOGRAPHY`, CARTO
  empujando el cómputo al warehouse, y **SpatialBench** como benchmark común (un generador de datos a
  escala 1–1000 y 12 consultas). Nadie gobierna el CRS de punta a punta.
- **Los huecos:** las Functions del árbol desde SQL no tipan geometría; Java no tiene JTS; no hay
  *join* espacial medido a escala; no hay motor por repositorio; no existe enriquecer.

## 4. Las decisiones

| # | decisión |
|---|---|
| D1 | **El motor, por repositorio**, elegido al crearlo en un banner: Lightweight (DuckDB) o Heavyweight (Spark 4.1) |
| D2 | **El *join* espacial es una operación, no un kind** |
| D3 | **Enriquecer entra aquí**, como verbo de primera clase |
| D4 | **La vara es nuestro banco más SpatialBench**: el banco da el «hecho cuando» del producto; SpatialBench compara y dimensiona motores con una carga que otros publican |
| D5 | **El CRS va en el tipo y se comprueba al compilar**; reproyectar es explícito (heredado de 0066) |

## 5. Lo que queda por saber (lo responde C0)

1. ~~**Spark geo:** ¿bastan las `ST_*` nativas de 4.1, o hace falta Sedona?~~ **Respondida en C0·0: Sedona.**
2. **El dialecto:** las diferencias entre DuckDB y Spark (nombres, CRS frente a SRID, H3,
   `ST_Transform`). Decide cuánto traduce ORE y cuánto escribe el cliente en el de su motor.
3. **Las Views:** ¿corren siempre en DuckDB, o en el motor del repositorio que las define?
4. **El *join* a escala:** R-tree geométrico, malla H3 con refinado, o los dos según el tamaño.
5. **Los objetivos con número** (latencia, filas/s, escala) de cada motor.
6. **La forma de enriquecer:** un kind propio, un tipo de Transform o un operador de Pipelines.

## 6. El plan

```
 C0 vara y sondas ───────────────────┐
 C1·1 SQL ──┐                         ├─► C1·4 Spark ─┐
 C1·2 Python ├─ (en paralelo con C0)  C2 motor ───────┼─► C3 join a escala ─► C4 enriquecer ─► cierre
 C1·3 Java ─┘                         └───────────────┘
```

C1·1–C1·3 son trabajo de framework y no esperan a ninguna decisión de motor: van en paralelo con C0.
C1·4, el resto de C2, C3 y C4 esperan a C0. De E1 en 0066: E1·1 es C1·1, E1·2 es C1·2 (casi entero
desde 0067 G2·3), E1·3 es C1·3 y E1·4 es C4. C0, C2 y C3 son nuevos: E1 daba por hecho un motor.

### C0 · La vara y las sondas (sin código de producto)

| paso | qué |
|---|---|
| C0·1 | **El banco de cómputo** en victor: Overture (34 M edificios, 2,2 M lugares), municipios, AIS y lo de 0067 |
| C0·2 | **SpatialBench** a escala 10 y 100: sus 12 consultas y sus datos |
| C0·3 | **Los motores:** DuckDB y Spark 4.1 nativo (y Sedona si hace falta) en filtros por zona, *joins* punto-en-polígono, cercanía y agregación H3 |
| C0·4 | **La tabla de dialectos:** nombres, CRS frente a SRID, H3, `ST_Transform` |
| C0·5 | **Los objetivos con número**, por motor, para aprobar |

**Hecho cuando:** informe en victor, las preguntas de §5 respondidas y los objetivos aprobados.

#### C0·0 · El previo (hecho 2026-10-18)

Lo que no sabíamos antes de medir, resuelto en una sentada. Guiones en `pruebas-de-fuego/geo/c0/`.

| pregunta | respuesta | cómo se supo |
|---|---|---|
| ¿Qué `ST_*` trae Spark 4.1? | **Cinco, y ninguna calcula:** `st_asbinary`, `st_geomfromwkb`, `st_geogfromwkb`, `st_setsrid`, `st_srid`. Ni predicados, ni `ST_Buffer`, ni `ST_Area`, ni `ST_Transform`, ni H3, ni *join* espacial. El SRID va en el tipo (`GEOMETRY(4326)`); `GEOMETRY(ANY)` sólo en memoria | `SHOW FUNCTIONS LIKE 'st_*'` en `apache/spark:4.1.0`, y la referencia de 4.1 y 4.2 |
| ¿Sedona sobre Spark 4.1? | **Sí, 1.9.1** (`sedona-spark-shaded-4.1_2.13` + `geotools-wrapper:1.9.1-33.5`, Java 17). Predicados, `ST_Area`, `ST_Transform` 4326→25830, `ST_H3CellIDs`, y el *join* con índice: `BroadcastIndexJoin` sobre R-tree para `ST_Contains` (500 zonas × 200 000 puntos, 272 917 pares en 3,1 s) y `DistanceJoin` para `ST_DWithin`. **Una trampa:** Spark 4.1 trae `jts-core-1.20.0` en `/opt/spark/jars`; con `--packages` Sedona va a otro cargador y el *join* revienta (`IllegalAccessError` en `IndexSerde`). Los jars van en el classpath de la aplicación (en la imagen, en `/opt/spark/jars`) | `sedona-en-spark41.sh` |
| ¿SpatialBench? | Generador en Rust (`cargo install spatialbench-cli`), Parquet con WKB. Tablas: `trip` (puntos de recogida y destino), `zone` y `building` (polígonos), y tres sin geometría. Escala 1 ≈ 1,8 GB (6 M viajes), 10 ≈ 7 GB (60 M), 100 ≈ 56 GB (600 M). 12 consultas: filtros, punto-en-polígono, `ST_DWithin`, agregación espacial, autojoin de polígonos y KNN. Publicado (sep-2025, una máquina): SedonaDB las 12 a escala 10; DuckDB 1.4 falla Q10–Q12 a escala 10. No hay cifras de Sedona en Spark ni a escala 100: las nuestras serán las primeras. **En local:** compilar el generador, 9 min; la escala 1, 12 min 45 s, casi todo bajando `zone` de Overture por red (≈ 4 GB de entrada, 4,5 GiB de memoria): `trip` 348 MB, `zone` 1,3 GB, `building` 1,9 MB. La escala 100 se genera, pues, cerca de los datos y con memoria de sobra | la documentación, y SF1 generado en local (volumen `c0-sb`) |
| ¿Cuánto AIS hay? | 18,2 M de posiciones al día, **~127 M en los 7 días** (CSV en zip, ~600 MB/día; `Timestamp, MMSI, Latitude, Longitude, SOG, COG…`). Para los 1 000 M de 0067 G0·4 faltan semanas: se completa con SpatialBench (`trip` a escala 100 son 600 M de puntos) | contado en flujo desde el bucket |
| ¿Dónde se mide Spark? | Hoy **no hay Spark en victor**: Java Transforms corre en el puesto JVM, no en un clúster. La cuota ya no es la de antes: **32 vCPU globales (4 en uso), 200 N2 y 24 E2 en europe-west1**. Spark se mide en un pool efímero de GKE (spot, p. ej. n2-standard-16), que se borra al acabar | `gcloud compute … quotas` |
| ¿Lee el agente el banco? | Sí: el bucket se lista y se lee con la cuenta de la sesión | `gcloud storage ls` |
| ¿Cabe en la máquina de desarrollo? | Sí, sin compactar: el disco de Docker es de 110 GB y usa 24; lo nuevo reutiliza el hueco de dentro. En C: quedan 20 GB. La escala 100 no se genera aquí: se genera en el pool efímero | `docker system df` |

**Lo que cambia en el plan:**
- **P1 queda respondida:** Heavyweight es Spark 4.1 **con Sedona 1.9.1**. Lo nativo de 4.1 sólo sirve para leer y escribir el tipo.
- C0·3 mide DuckDB 1.5.6 frente a Spark + Sedona. SedonaDB entra como referencia del estado del arte, no como candidato (D1 no cambia).
- C0·2 genera la escala 100 en el pool efímero, no en local.

### C1 · El contrato en los frameworks

| paso | qué | ¿espera a C0? |
|---|---|---|
| C1·1 SQL | empujar el paso 1 (hecho en local) · Functions del árbol desde SQL con geometría · `ore-sql` y el motor con tipos geo | no |
| C1·2 Python | cierre: TypeScript con geometría en la firma y lo que falte | no |
| C1·3 Java | JTS y la geometría por Arrow, en Functions y Java Transforms | poco (la versión de JTS) |
| C1·4 Spark | el contrato de lo que escribe un Transform Heavyweight, con su CRS | sí |

**Hecho cuando:** una View, un Transform Lightweight y uno Heavyweight dan el mismo contrato con CRS
para la misma operación, y el compilador niega mezclar CRS.

### C2 · El motor por repositorio

| paso | qué |
|---|---|
| C2·1 | el banner en la consola al crear el repositorio; el repositorio guarda su motor y no se cambia |
| C2·2 | sus Transforms y Builds corren en su motor: DuckDB en el puesto, Spark con la pila de Java Transforms |
| C2·3 | la imagen pesada con geo (lo que diga C0) |

**Hecho cuando:** un repositorio de cada clase, creado desde su banner, corre un *join* espacial en su
motor.

### C3 · El *join* espacial a escala

| paso | qué |
|---|---|
| C3·1 | el layout del lago que lo acelera: Hilbert (ya está) y, si la vara lo pide, una columna H3 |
| C3·2 | el *join* eficiente en cada motor: R-tree en DuckDB; particionado o H3 con refinado en Spark |
| C3·3 | lo que ORE recomienda o genera cuando ve un *join* espacial |

**Hecho cuando:** los casos de *join* de la vara cumplen su objetivo en los dos motores.

### C4 · Enriquecer

| paso | qué |
|---|---|
| C4·1 | la forma (lo decide C0): kind, tipo de Transform u operador de Pipelines |
| C4·2 | las relaciones (contiene, intersecta, a distancia, el más cercano) y lo que se toma (columnas, agregados, ponderado por área) |
| C4·3 | el cómputo generado por motor, con la comprobación de CRS |
| C4·4 | el gobierno: los permisos de la capa se heredan, linaje a las dos fuentes, frescura |
| C4·5 | la consola: enriquecer desde la ficha del activo |

**Hecho cuando:** «enriquece clientes con municipios» y «cuenta competidores a 500 m» dan el mismo
resultado que PostGIS, heredan las etiquetas de la capa y se marcan viejos cuando ella cambia.

### Cierre

El ADR pasa de hoja de ruta a imagen.

## 7. Fuera de este ADR

| qué | dónde |
|---|---|
| empujar filtros y *joins* espaciales al origen (PostGIS, BigQuery) | federación, más adelante |
| datos de referencia de serie (Overture, Catastro, INE) | 0066 E5 |
| trayectorias y encuentros | 0066 E3 |
| teselas y mapa | 0066 E2 |
| cálculo ráster (zonal, NDVI) | 0066 E4 |

## 8. Riesgos

1. **El dialecto entre motores.** Si DuckDB y Spark difieren mucho en las `ST_*`, el cliente escribe en
   el de su motor y ORE garantiza el contrato, no la portabilidad del SQL. C0·4 dice cuánto pesa.
2. **El coste de Spark.** La cuota ya no aprieta (32 vCPU globales desde C0·0), pero Heavyweight
   necesita su dimensionado y su precio por hora.
3. **El disco de Docker en la máquina de desarrollo** tumbó dos compilaciones el 2026-10-17. En C0·0 hay
   86 GB libres dentro del disco virtual: basta con limpiar dentro al cerrar cada etapa.
4. **El classpath de Sedona.** Dos JTS en dos cargadores rompen el *join* (C0·0). La imagen pesada
   lleva los jars de Sedona en `/opt/spark/jars`, nunca por `--packages`.

## 9. Bitácora

- **2026-10-18 · Abierto.** Marco acordado antes de escribir: qué es, la visión, lo que sabemos (con lo
  medido en 0066 E1 y E1·1 paso 1), el estado del arte y las decisiones D1–D5 (el motor por
  repositorio, como en Foundry; enriquecer entra aquí). Los hitos C0–C4 recogen E1·1–E1·4 de 0066 y
  añaden la vara, el motor por repositorio y el *join* a escala. Siguiente: C0, y en paralelo C1·1.
- **2026-10-18 · C0·0, el previo.** Spark 4.1 sólo trae cinco `ST_*` de entrada y salida: Heavyweight es
  Spark + Sedona 1.9.1, que corre sobre 4.1 con los jars en el classpath de la aplicación (P1 respondida).
  El AIS son ~127 M de posiciones; SpatialBench se instala con `cargo`; Spark se mide en un pool
  efímero (la cuota es ahora de 32 vCPU). Siguiente: C0·1–C0·5.
- Fuentes del estado del arte: [SedonaDB](https://sedona.apache.org/1.8.1/blog/2025/09/24/introducing-sedonadb-a-single-node-analytical-database-engine-with-geospatial-as-a-first-class-citizen/),
  [SedonaDB 0.3.0](https://sedona.apache.org/1.9.1/blog/2026/03/01/sedonadb-030-release/),
  [SpatialBench](https://wherobots.com/blog/sedonadb-spatialbench-spatial-query-engine-apache-sedona/),
  [Databricks ST](https://learn.microsoft.com/en-us/Azure/databricks/sql/language-manual/sql-ref-st-geospatial-functions),
  [Spark, tipos geo](https://spark.apache.org/docs/4.2.0/sql-ref-geospatial-types.html),
  [DuckDB, spatial joins](https://duckdb.org/2025/08/08/spatial-joins).
