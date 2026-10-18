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
 C1·1 SQL ──┐                         │
 C1·2 Python ├─ (en paralelo con C0)  ├─► C2 motor (con Spark) ─► C3 join a escala ─► C4 enriquecer ─► cierre
 C1·3 Java ─┘                         │
```

C1·1–C1·3 son trabajo de framework y no esperan a ninguna decisión de motor: van en paralelo con C0.
C2 (con lo que era C1·4), C3 y C4 esperan a C0. De E1 en 0066: E1·1 es C1·1, E1·2 es C1·2 (casi entero
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

**Resultado (CERRADO 2026-10-18).** El detalle va en las subsecciones de abajo.

- **Heavyweight es Spark 4.1 + Sedona 1.9.1.** Lo nativo de Spark 4.1 sólo lee y escribe el tipo. Los
  jars de Sedona van en el classpath de la aplicación: dos JTS en dos cargadores rompen el *join*.
- **Los dos motores dan lo mismo** en todo lo que terminó en ambos, y el SQL espacial es casi portable
  (SpatialBench sólo cambia Q5 y Q12). ORE no traduce el SQL.
- **DuckDB es el rápido mientras cabe**: entre 2 y 25 veces sobre un nodo de 16 vCPU. El banco entero
  (34 M de edificios, 115,7 M de posiciones AIS) le cuesta entre 7 y 18 s por caso.
- **Spark + Sedona termina lo que DuckDB no**: Q5 a escala 100 en 345 s y Q12 en 257 s.
- **Heavyweight tiene que difundir el lado pequeño por sí mismo.** Sin difusión, el punto-en-polígono
  pasa de 14,5 s a más de 600 s.
- **Las Views se describen en DuckDB y se ejecutan en el motor del repositorio.** Enriquecer es un
  *join* difundido: un operador, no un kind.
- **Los objetivos de C0·5 están aprobados**, con Heavyweight referido a 2 nodos.
- **Abierto para C3:**
  - Q10, el `LEFT JOIN` espacial, falla en Sedona;
  - Q6 y Q8 a escala 100 pasan de 600 s;
  - Spark en varios nodos no se ha medido.

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

#### C0·1–C0·3 · El laboratorio (local, 2026-10-18)

**Para qué:** comprobar que los casos corren y dan lo mismo en los dos motores antes de pagar por
medir a escala. Las cifras son de una máquina de desarrollo (Docker con 16 hilos y 5,8 GiB, 4 GB por
motor): sirven para comparar formas, no como objetivos.

**El banco de cómputo** (`preparar-banco.py`): Parquet con la geometría en WKB, ordenado por Hilbert,
el mismo para los dos motores. Contiene:
- `edificios`: 34,1 M, 6,4 GB, con copia en EPSG:25830;
- `lugares`: 2,2 M, también con copia en 25830;
- `municipios`: 8 655 polígonos, la `locality` de Overture;
- `ais`: un día, 18,1 M de posiciones.

**Los casos del banco:**

| caso | qué mide |
|---|---|
| B1 | filtro por zona: los edificios del término de Madrid |
| B2 | punto en polígono, la forma de enriquecer: cada lugar con su municipio |
| B3 | cercanía en metros: pares lugar–edificio a 20 m o menos |
| B4 | agregación H3: posiciones AIS por celda de resolución 8 |

**Cómo se corre** (`correr.sh`, `correr.py` y `cotejar.py`):
- las 12 de SpatialBench, en el dialecto de cada motor, sacadas de su repositorio (commit `8e44bdd`);
- los casos B1–B4;
- un límite de 600 s por caso;
- para cotejar, una huella de cada resultado: los flotantes y los `Decimal` a 4 decimales, y los
  intervalos en segundos.

| caso | DuckDB 1.5.6 | Spark 4.1 + Sedona 1.9.1 | ¿igual? |
|---|---|---|---|
| B1 zona | 8,0 s | 24,7 s | sí (159 802 edificios) |
| B2 punto en polígono | 29,6 s | > 600 s; **37 s difundiendo `municipios`** | sí (2 300 045 lugares en 7 992 municipios) |
| B3 cercanía 20 m | 7,0 s | 298 s; 151 s difundiendo `lugares` | sí (8 881 138 pares) |
| B4 H3 res. 8 | 1,6 s | 7,7 s | sí (115 602 celdas) |
| SB Q1 KNN por radio | 0,3 s | 11,5 s | sí |
| SB Q2 zona por subconsulta | 0,5 s | 392 s; **7,9 s como join difundido** | sí |
| SB Q3 | 0,5 s | 7,1 s | sí |
| SB Q4 | 1,9 s | 92 s | sí |
| SB Q5 agregación espacial | 13,7 s | 33 s | sí |
| SB Q6 | 2,0 s | 101 s | sí, la muestra (la huella se rehizo tras normalizar) |
| SB Q7–Q9 | 0,8 / 1,0 / 0,1 s | 7,9 / 5,2 / 1,5 s | sí |
| SB Q10 | 355 s | el proceso muere: sin memoria | — |
| SB Q11 | sin memoria | ídem | — |
| SB Q12 KNN join | > 600 s | ídem | — |

**Lo que dice:**
1. **Los dos motores dan lo mismo** en todo lo que terminó. El dialecto de SpatialBench sólo cambia en
   Q5 y Q12, así que el SQL espacial es casi portable (P2 se inclina a «ORE no traduce»).
2. **En una máquina, DuckDB gana siempre**, entre 3 y 40 veces. Es lo esperado: Spark paga el arranque
   de cada etapa y la lectura de WKB en la JVM. Lightweight no es un modo pobre: es el rápido mientras
   cabe.
3. **Spark sin difusión se planifica mal.** Hay dos trampas, y las dos se arreglan sin cambiar la
   semántica:
   - el lado pequeño, por encima de los 10 MB de `autoBroadcastJoinThreshold`, lleva a un `RangeJoin`
     que reparte las dos tablas;
   - una geometría en subconsulta escalar se evalúa fila a fila, sin índice.

   **Heavyweight tiene que difundir el lado pequeño por sí mismo:** una regla del planificador de ORE
   o la configuración de Sedona, no una pista que escriba el cliente. Entra en C3.
4. **Q10–Q12 no caben a 4 GB en ninguno de los dos.** Es la frontera que SpatialBench publicó para
   DuckDB, y es justo donde Heavyweight tiene que demostrar su valor: con más memoria y varios nodos.
5. **Tres trampas del laboratorio, ya resueltas en los guiones:**
   - Docker no lee bien un Parquet de 4 GB por el montaje de Windows (`Cannot allocate memory`);
   - el directorio temporal de la sesión no es visible para Docker;
   - un motor que muere a media corrida perdía lo ya medido.

   Lo de dentro va ahora a volúmenes, y los resultados se escriben desde fuera.

#### C0·1–C0·3 · A escala (pool efímero, 2026-10-18)

**Dónde:** un nodo spot `n2-standard-16` (16 vCPU, 64 GB) en un pool `c0-medida` de `ore-mesh`. Se
creó para medir y se borró al acabar, junto con su espacio de nombres y su permiso sobre el bucket.
Los dos motores, por turnos y solos en el nodo:
- DuckDB 1.5.6 con 48 GB;
- Spark 4.1 + Sedona 1.9.1 en `local[16]`, con 40 GB y una consulta por proceso.

**Los límites:** 1 200 s en DuckDB y 600 s en Spark (la versión acotada). Lo que agotó a escala 10 no
se lanzó a escala 100.

**Los datos:**
- SpatialBench a escala 10 (60 M viajes) y a escala 100 (600 M);
- el banco con los **7 días de AIS (115,7 M posiciones)**.

**Los guiones:** `pool/acotada.sh` sube cada resultado al bucket según sale, es idempotente y pone
topes al disco. Resultados en `resultados/20261018-pool.jsonl`.

| caso | DuckDB SF10 | Spark SF10 | DuckDB SF100 | Spark SF100 |
|---|---|---|---|---|
| Q1 KNN por radio | 5,8 s | 11,1 s | 10,5 s | 68,6 s |
| Q2 zona (Spark: join difundido) | 1,5 s | 18,2 s | 10,9 s | 125 s |
| Q3 | 2,1 s | 13,4 s | 11,1 s | 63,5 s |
| Q4 punto en polígono | 7,4 s | 17,4 s | 22,9 s | 75,3 s |
| Q5 agregación espacial | 11,2 s | 54,0 s | **sin memoria** (80 GB de desborde) | **345 s** |
| Q6 | 2,0 s | 55,1 s | 91,6 s | > 600 s |
| Q7 | 1,9 s | 13,9 s | 34,4 s | 79,6 s |
| Q8 cercanía viaje–edificio | 3,9 s | 26,0 s | 36,0 s | > 600 s |
| Q9 IoU de edificios | 0,3 s | 7,3 s | 0,5 s | 6,5 s |
| Q10 LEFT JOIN espacial | > 1 200 s | error en 8 s | — | error |
| Q11 doble punto en polígono | > 1 200 s | > 600 s | — | — |
| Q12 KNN join | > 1 200 s | **257 s** | — | > 600 s |

| caso del banco (34 M edificios, 115,7 M AIS) | DuckDB | Spark + Sedona |
|---|---|---|
| B1 zona | 12,0 s | 13,9 s |
| B2 punto en polígono | 18,4 s | **14,5 s** (difundiendo) |
| B3 cercanía 20 m | 12,4 s | 86,7 s (difundiendo) |
| B4 H3 res. 8 sobre 115,7 M | 6,6 s | 29,5 s |

**Lo que dice la escala:**
1. **Las huellas coinciden en todo lo que terminó en los dos motores**, también entre las variantes
   difundidas y las originales.
2. **DuckDB sigue siendo el rápido hasta donde llega**, entre 2 y 25 veces. Pero a escala 100 ya no
   llega a todo:
   - Q5 se queda sin memoria y sin disco de desborde;
   - Q10–Q12 no acaban ni a escala 10.
3. **Spark + Sedona termina lo que DuckDB no**: Q5 a escala 100 en 345 s y Q12 a escala 10 en 257 s.
   En un solo nodo es más lento en todo lo demás. Heavyweight vale por lo que no cabe, no por la
   velocidad.
4. **Con la difusión, Spark empata o gana en punto-en-polígono sobre el banco** (B2: 14,5 s frente a
   18,4 s), que es la forma de enriquecer.
5. **Lo abierto en Spark:**
   - Q10 (`LEFT JOIN` con predicado espacial) falla en 8 s: Sedona no indexa el *join* externo (por
     confirmar con la traza entera);
   - Q6 y Q8 a escala 100 pasan de 600 s;
   - Spark en varios nodos no se ha medido.

   Las tres cosas van a C3.

#### Las preguntas de §5, respondidas por C0

| # | respuesta |
|---|---|
| P1 Spark geo | Spark 4.1 **con Sedona 1.9.1** (C0·0) |
| P2 el dialecto | **Casi portable:** SpatialBench sólo cambia Q5 y Q12 entre los dos, y los once casos que terminan en ambos dan lo mismo. **ORE no traduce el SQL:** documenta las diferencias, y la tabla se escribe con C1·1 |
| P3 dónde corren las Views | **Se describen siempre en DuckDB** (es el compilador de tipos de E1·1) y **se ejecutan en el motor del repositorio**. Como el SQL es casi el mismo, una View sirve en los dos |
| P4 el *join* a escala | **Índice difundido** (R-tree) cuando un lado cabe en memoria, que es el caso de enriquecer. Partición espacial (`RangeJoin`) sólo cuando los dos son grandes. **Heavyweight difunde el lado pequeño por sí mismo**: regla de ORE, no pista del cliente (C3). H3 como estrategia de *join* no hace falta todavía |
| P5 objetivos | En C0·5, aprobados |
| P6 la forma de enriquecer | El punto-en-polígono difundido (B2) **es** enriquecer, y es un *join*. Recomendación: **un operador** (Transform o Pipelines) que genera ese *join* con la difusión puesta, **no un kind nuevo**. Se decide en C4·1 |

#### C0·5 · Los objetivos (aprobados 2026-10-18)

En un nodo de 16 vCPU y 64 GB. Son la línea base medida hoy; C2 y C3 no pueden empeorarla.

| motor | objetivo |
|---|---|
| **Lightweight** (DuckDB) | Q1–Q9 de SpatialBench escala 10 en **< 15 s** cada una; Q1–Q4, Q7–Q9 a escala 100 en **< 60 s**. B1–B4 sobre el banco entero en **< 20 s**. Lo que no cabe (Q5 a escala 100, Q10–Q12) **falla con un error claro** que aconseja Heavyweight, sin tumbar el puesto |
| **Heavyweight** (Spark + Sedona) | **Termina todo SpatialBench a escala 100** (hoy no terminan Q6, Q8, Q10, Q11 ni Q12) en **< 10 min** por consulta con **2 nodos** de 16 vCPU (C3; 32 vCPU, lo que cabe en la cuota global de hoy). Enriquecer (B2) a la altura de DuckDB (**< 20 s**) en un nodo. Q10 sin error |

**Lo que queda fuera de C0:** Spark en varios nodos (C3) y DuckDB en la forma del producto, dentro
de un puesto de victor (C2).

### C1 · El contrato en los frameworks

| paso | qué | ¿espera a C0? |
|---|---|---|
| paso | qué | estado |
|---|---|---|
| C1·1 SQL | Views con su tipo exacto · Functions del árbol desde SQL con geometría (UDF sobre WKB + macro con el CRS) · la vista con copia (`volcar` marca, `calcular` carga `spatial`, `sellar-arrow` sella geo) · Transform SQL · el editor (errores dichos como ORE, completar y explicar `st_`/`h3_`) | hecho en local |
| C1·2 Python | `h3` en la imagen · `Geometry`/`Geography` en las firmas de TypeScript · DuckDB 1.5.6 también en Node. `pyogrio` fuera (decidido: los ficheros geo los lee `ST_Read`) | hecho en local |
| C1·3 Java | Java solo tiene Transforms (no Functions). La geometría calculada llega (el JDBC no la exporta sin transacción) y vuelve con su CRS · JTS 1.20.0 y H3 4.5.0 · JDBC 1.5.6.0: **los tres puestos, el mismo DuckDB** | hecho en local |
| ~~C1·4 Spark~~ | **pasa a C2** (decidido 2026-10-18): Heavyweight todavía no existe en el producto, y su contrato se mide con su motor | — |

**Hecho cuando:** una View, un Transform SQL y uno de Python o Java dan el mismo contrato con CRS para
la misma operación, y los tres niegan mezclar CRS. El de Spark, en C2. Al cerrar: la suite completa, un
solo empuje y una celda en victor (el contrato que expone el árbol tras un Build real, y una geometría
devuelta por una invocación HTTP, que hoy `toWire` resume).

### C2 · El motor por repositorio

| paso | qué |
|---|---|
| C2·1 | el banner en la consola al crear el repositorio; el repositorio guarda su motor y no se cambia (hecho en local) |
| C2·2 | la imagen pesada con geo (lo que diga C0). **Antes que los Builds** (reordenado 2026-10-18): corren en ella |
| C2·3 | sus Transforms y Builds corren en su motor: DuckDB en el puesto, Spark con la pila de Java Transforms |
| C2·4 | **(era C1·4)** el contrato de lo que escribe un Transform Heavyweight, con su CRS: Spark escribe por el catálogo `/v1` de ORE (lo admite; medido con PyIceberg y DuckDB, nunca con Spark) la geometría en WKB `binary` con su `ore.geo.<col>`, que es el estado de hoy del lago (0067 G2). Spark 4.1 nativo no admite CRS proyectados (0067: EPSG:25830 da `ST_INVALID_CRS_VALUE`) |

**Hecho cuando:** un repositorio de cada clase, creado desde su banner, corre un *join* espacial en su
motor.

#### C2·4 · El laboratorio de Spark contra `/v1` (2026-10-18)

**Montaje** (`pruebas-de-fuego/geo/c2/spark-por-v1/`): un `ore-serve` de verdad (*release*, en `rust:1`)
con su S3 de mentira, y Spark 4.1 + Iceberg 1.12.0 (+ Sedona 1.9.1) en la red de ese contenedor, con
identidad de cabecera y `vended-credentials`. Es el primer *commit* desde Spark que se mide: hasta
hoy, por `/v1` sólo habían escrito PyIceberg y DuckDB.

| caso | resultado |
|---|---|
| S1 · crear el *namespace* y la tabla, `INSERT` | **va**: Iceberg v2, `geom` en `binary`, y **ORE conserva `ore.geo.geom`** (`{"crs":"EPSG:25830","type":"Geometry<Point, EPSG:25830>"}`), con su `ore.dataset` |
| S2–S3 · leer, añadir un segundo *commit* | **va**: 2 *snapshots*, 3 filas |
| S5 · Sedona calcula (`ST_Buffer` en EPSG:25830) y escribe WKB con su `ore.geo.zona` | **va**: el área de cada *buffer*, 31 214 m², **la misma que da DuckDB** (C1·1) |
| S6 · el `GEOMETRY(4326)` nativo de Spark (v3) | **ORE lo niega, pero mal**: un 500 `CommitStateUnknownException` (`ore-store-r2` no abre el tipo `geometry` de v3; `iceberg` 0.10). «Estado desconocido» le dice a Spark que quizá se escribió: tiene que ser un 400 que diga cómo escribirla |

**Lo que dice:**
1. **El contrato de Heavyweight es el de hoy del lago:** WKB en `binary` con su `ore.geo.<col>` (CRS y
   tipo), escrito por `/v1`. Los SDK de ORE la leen como geometría (0067: `ore.geo.*` de la tabla).
2. Spark la deja en v2 sin el tipo lógico `GEOMETRY` del Parquet ni la clave GeoParquet que pone el
   escritor de ORE: un lector de fuera ve bytes. Se iguala cuando el lago promueva (0067: `iceberg`
   0.11).
3. **El `ore.geo.<col>` lo tiene que poner ORE, no el cliente:** a mano es una cadena JSON que nadie
   comprueba. Lo pone el Transform Heavyweight desde el contrato de su salida (C2·3).
4. El `GEOMETRY` nativo de Spark (y los CRS proyectados, que Spark 4.1 no admite) se niega con un 400 y
   su remedio (`ST_AsBinary`).

#### C2·2 · La imagen pesada: el previo (2026-10-18)

**Prototipo** (pieza 0; su Dockerfile se quitó al llegar la etapa, la sonda sigue en
`pruebas-de-fuego/geo/c2/puesto-spark/`): el Python del puesto (`python:3.14-slim`) con
el JDK de Java Transforms (temurin 21, copiado) y Spark por `pip install pyspark==4.1.0`, que trae sus
jars; encima, Iceberg 1.12.0 (runtime y los *bundles* de GCP y de AWS), Sedona 1.9.1 (shaded y
`geotools-wrapper`) y `apache-sedona` de Python. La sonda corre **sin red** (`--network none`).

| caso | resultado |
|---|---|
| Spark 4.1 en **Java 21** (el laboratorio de C2·4 iba en el 17) | **va**: la sesión local con Iceberg y Sedona arranca en 8 s |
| PySpark 4.1 y Sedona en **Python 3.14** | **va**, el mismo Python que el puesto. Sedona en Python pide `pandas` y no lo instala; `pyarrow` lo pide `toPandas` por Arrow (y el SDK) |
| las clases, todas dentro | `GCSFileIO` (el lago de producción es GCS: `/v1` presta `gcs.oauth2.token`), `S3FileIO` (R2 y el laboratorio), `RESTCatalog`, Sedona |
| `ST_Buffer` en EPSG:25830 | **31 214 m²**, la misma que DuckDB y que C2·4; también desde un DataFrame de shapely |
| `ST_Transform` 25830→4326 **sin red** | **por defecto, no**: proj4sedona (el de Sedona 1.9) **baja la definición EPSG de spatialreference.org**. Con `spark.sedona.crs.geotools=all`, GeoTools la lleva dentro: `POINT (-3.7072 40.4145)`, en orden lon/lat. `true` y `vector` siguen saliendo a la red |
| el uid 65532 (el de todos los puestos) | **Spark no arranca**: el `UnixLoginModule` de Hadoop pide el nombre del usuario (`invalid null input: name`), y `HADOOP_USER_NAME` no basta. Un `ore` en `/etc/passwd` lo arregla |
| `javac` (Spark Java) y la CLI `spark-sql` | **van** (la CLI tarda 19 s en arrancar) |
| el tamaño | **1,05 GB comprimida** (la de Python, 1,53 GB sin comprimir). De ello: Spark 779 MB (692 MB de jars, de los que Sedona 79, el *bundle* de GCP 69 y el de AWS 66), el JDK 295, pyarrow 167 |

**Lo que dice para la etapa (pieza 1):** la variante A vale y no hace falta el tarball de Apache. La
imagen lleva `/etc/passwd` con `ore`, `pandas` y `pyarrow`, y un `spark-defaults.conf` con
`spark.sedona.crs.geotools=all`: **sin eso, un `ST_Transform` en el puesto falla** (el puesto no sale a
internet). Queda por mirar en la pieza 2 el orden de los ejes frente a DuckDB, que en 4326 puede dar
lat/lon: un mismo Transform no puede dar puntos distintos según el motor.

**Pieza 1 · la etapa `puesto-spark` (hecha en local).** `FROM puesto-python`: **es el puesto de Python
más Spark**, con el mismo pandas, pyarrow, shapely y DuckDB fijados, y el mismo SDK. Encima: el JDK de
temurin 21, `puesto/spark/provisto.txt` (pyspark 4.1.0, py4j, apache-sedona 1.9.1 y attrs, con
`--no-deps`), `puesto/spark/jars.txt` (Iceberg y Sedona, en el formato de `puesto/jvm/jars.txt`),
`puesto/spark/spark-defaults.conf` (Iceberg y Sedona siempre, Kryo, `crs.geotools=all`, Arrow, sin
interfaz web) y `ore` en `/etc/passwd`. El catálogo `ore` no va en la imagen: su URI y su identidad
son de cada Build (C2·3). La construcción se demuestra como uid 65532 (`puesto/spark/comprobar.py`), y
`en-la-imagen.sh` pasa la sonda sin red: **16/16**, sin `SPARK_LOCAL_IP`. **1,26 GB comprimida**; sobre
`puesto-python` añade 1,13 GB sin comprimir (Spark 824 MB y el JDK 308), en capas que el nodo baja una
vez.

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
- **2026-10-18 · C0·1–C0·3 medidos.** Primero el laboratorio local y luego un pool efímero
  (`n2-standard-16` spot, borrado al acabar). Los dos motores dan lo mismo. DuckDB es el rápido
  mientras cabe; Spark + Sedona termina lo que DuckDB no (Q5 a escala 100, Q12). Spark necesita que ORE
  difunda el lado pequeño. P1–P4 y P6 respondidas; los objetivos de C0·5, propuestos.
  - **Un tropiezo:** el primer intento en el pool murió a las 12:31 por disco efímero lleno
    (273 GB) y perdió lo medido.
  - **Lo que lo arregló:** subir cada resultado según sale, poder reanudar y poner topes de disco.
  - Siguiente: aprobar los objetivos y cerrar C0.
- **2026-10-18 · C0 CERRADO.** Objetivos de C0·5 aprobados, con Heavyweight referido a 2 nodos (32 vCPU,
  la cuota de hoy). Siguiente: C1, el contrato en los frameworks.
- Fuentes del estado del arte: [SedonaDB](https://sedona.apache.org/1.8.1/blog/2025/09/24/introducing-sedonadb-a-single-node-analytical-database-engine-with-geospatial-as-a-first-class-citizen/),
  [SedonaDB 0.3.0](https://sedona.apache.org/1.9.1/blog/2026/03/01/sedonadb-030-release/),
  [SpatialBench](https://wherobots.com/blog/sedonadb-spatialbench-spatial-query-engine-apache-sedona/),
  [Databricks ST](https://learn.microsoft.com/en-us/Azure/databricks/sql/language-manual/sql-ref-st-geospatial-functions),
  [Spark, tipos geo](https://spark.apache.org/docs/4.2.0/sql-ref-geospatial-types.html),
  [DuckDB, spatial joins](https://duckdb.org/2025/08/08/spatial-joins).
