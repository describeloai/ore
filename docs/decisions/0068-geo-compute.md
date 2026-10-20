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
 C0 vara y sondas ───────────────────┐                          ┌─► C3 join a escala ──┐
 C1·1 SQL ──┐                         │                          │       (C3·d mide los dos)
 C1·2 Python ├─ (en paralelo con C0)  ├─► C2 motor (con Spark) ──┤                      ├─► C4 enriquecer ─► cierre
 C1·3 Java ─┘                         │                          └─► CE Build elástico ─┘
```

**C3 y CE se separan por responsabilidad.** C3 es **qué** se calcula bien en un *join* espacial: las
reglas del planificador (difundir, `LEFT JOIN`, la rejilla, Q2) y la vara. CE es **dónde y con cuántos
nodos** corre cualquier Build pesado, con sus defensas de memoria y de disco. Una regla de C3 vale con un
nodo o con seis; CE no sabe de geometría. La vara (C3·d) mide los dos juntos.

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

**Cierre de C1 · x es la longitud, siempre (hecho en local, 2026-10-19).** Lo que encontró C2·2: los
tres puestos cargaban `spatial` sin `geometry_always_xy`, y el DuckDB 1.5.6 de `spatial` toma
entonces el orden de la autoridad EPSG, que en 4326 es lat/lon. El lago guarda x = longitud
(GeoParquet, WKB) y Sedona trabaja así. **El previo** (`pruebas-de-fuego/geo/c1/ejes/previo.py`)
dice que el hueco era mayor que los puntos traspuestos:

| caso | sin el ajuste | con él |
|---|---|---|
| `ST_Transform` 25830 → 4326 | `POINT (40.4145 -3.7072)` (lat/lon) | `POINT (-3.7072 40.4145)` |
| `ST_Transform` 4326 → 25830 **de un lon/lat del lago** | `POINT (5860699 -565092)`: fuera de España | `POINT (439999.96 4474004.4)` |
| `ST_Distance_Spheroid` Sol–Barcelona, desde lon/lat | **659 km** | **507 km** (la de verdad) |
| `ST_Area_Spheroid` de 0,01° × 0,01° a 40° N | 1,2 km² | 0,9 km² (la de verdad) |
| la ida y vuelta 25830 → 4326 → 25830, `ST_X`, el WKB | iguales | iguales |
| `always_xy := false` en la llamada | lat/lon | lat/lon: quien lo pida, lo tiene |

Antes de `load spatial` el ajuste no existe (lo trae la extensión), y vale para la conexión: los SDK
usan una por proceso. **Puesto en los tres SDK, donde se carga `spatial`:**
- Python: `_cargar`, más `_ejes_xy`, y también en `calcular.py` (la vista con copia) y en
  `sql_functions.py` (las Functions geo desde SQL);
- Node y Java: `cargar`.

Así lo tienen también el puesto pesado (el mismo SDK de Python) y todo lo que corre en DuckDB.
**Por la API pública de cada SDK** (`ejes/medir.sh sdk`): los tres dan `POINT (-3.7072 40.4145)` y
507 km. Lo mismo queda en las suites: `test_ejes_xy.py` (Python), `geo.test.mjs` (Node) y
`GeoPrueba.java` (Java). El SDK, 163 pruebas en `puesto-python` y en `puesto-spark`.

**Lo que cambia para quien ya tenía código:** un Transform que llevara un `ST_Transform` a 4326, o
`ST_Distance_Spheroid` sobre datos del lago, da otro resultado (el bueno) en su próximo build. Lo ya
escrito con los ejes traspuestos se queda como está: en victor se mira al cerrar C1 (columnas en
4326 cuyas «longitudes» caen fuera de ±90 o en el rango de las latitudes de los datos).

**Cierre de C1 · la geometría que devuelve una función, por su invocación (hecho en local).** El
hueco era mayor de lo que se había anotado («`toWire` la resume»):

| SDK | antes | ahora |
|---|---|---|
| Python | **la invocación fallaba**: el contrato entrega la geometría como shapely, y la tabla de Arrow del arnés no la toma (`ArrowInvalid`) | WKB en hex (`_plano` del arnés, `funciones.rs`) |
| Node | `{"bytes": 21}`: la geometría se perdía | WKB en hex (`contract.toWire`) |

Es lo que pide la spec (v1alpha32 `01` §5): en texto (una fila JSON, una muestra), una geometría es
su **WKB en hexadecimal**, nunca WKT ni GeoJSON. En una firma no hay otros bytes que una geometría
(un `Uint8Array` es `Opaque`, que también sale en hex). **Medido con las celdas de verdad**
(`pruebas-de-fuego/geo/c1/invocar/`): una prueba de ore-serve las escribe y se corren en las imágenes
de Python y de Node; las dos dan el mismo `POINT` en WKB en hex. **Y por la misma regla**, el Preview
pesado (pieza 5) da sus filas geo en WKB en hex, y no en WKT.

**Queda, con nombre:** el Preview y la muestra de Lightweight (`table()`) dan una columna binaria en
base64; para una geometría, la spec dice hex. Tocarlo cambia el formato de la consola para todo lo
binario: va con la consola.

**Y para el empuje de la pieza 6:**
- `cola.rs`: un trabajo pesado va a `cola-pesada` (el diseño de la pieza 6).
- `capa-jvm` se construye (C1·3 la había dejado sin construir): 16 jars provistos, entre ellos JTS
  1.20.0 y H3 4.5.0, así que un repositorio que los declare los recibe del puesto.

### C2 · El motor por repositorio

| paso | qué |
|---|---|
| C2·1 | el banner en la consola al crear el repositorio; el repositorio guarda su motor y no se cambia (**en victor**) |
| C2·2 | la imagen pesada con geo (lo que diga C0). **Antes que los Builds** (reordenado 2026-10-18): corren en ella. **En victor** |
| C2·3 | sus Transforms y Builds corren en su motor: DuckDB en el puesto, Spark en `puesto-spark` (piezas abajo; **en victor**) |
| C2·4 | **(era C1·4)** el contrato de lo que escribe un Transform Heavyweight, con su CRS: Spark escribe por el catálogo `/v1` de ORE (lo admite; medido con PyIceberg y DuckDB, nunca con Spark) la geometría en WKB `binary` con su `ore.geo.<col>`, que es el estado de hoy del lago (0067 G2). Spark 4.1 nativo no admite CRS proyectados (0067: EPSG:25830 da `ST_INVALID_CRS_VALUE`) |

**Hecho cuando:** un repositorio de cada clase, creado desde su banner, corre un *join* espacial en su
motor.

**C2 CERRADO (2026-10-19).** En victor, tres repositorios Heavyweight creados desde la consola
(Python, SQL y Java) corrieron el *join* de 20 M puntos × 20 k polígonos en el pool pesado, en Spark +
Sedona, y los tres dieron lo mismo: **20 000 zonas y 11 129 901 pares**. El Lightweight lo corre
DuckDB desde C1. Cómo se llegó, en la pieza 6 de C2·3; lo que queda, con nombre, al final de ella.

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

**Pieza 2 · contra `/v1`, con la imagen (hecha en local).** El laboratorio de C2·4 con
`IMAGEN=ore-puesto-spark:local` (`medir.sh`): como el uid de los puestos, sin `--packages` ni ivy, y
sin decirle extensiones ni serializador; sólo el catálogo, como lo dirá el Build. **Igual que C2·4:**
S1–S3 van (v2, 2 *snapshots*, ORE conserva `ore.geo.geom`), S5 da 31 214 m² en las tres filas y
escribe `ore.geo.zona`, y S6 sigue en el 500 (su 400 es de C2·3).

**El orden de los ejes en EPSG:4326** (`ejes.py`, los dos motores en la misma imagen, el mismo punto):

| motor | `ST_Transform(g, 'EPSG:25830', 'EPSG:4326')` |
|---|---|
| DuckDB 1.5.6, por defecto | `POINT (40.4145 -3.7072)`: **lat/lon**, el orden de la autoridad EPSG |
| DuckDB con `set geometry_always_xy = true` (o `always_xy := true`, o a `'OGC:CRS84'`) | `POINT (-3.7072 40.4145)` |
| Sedona 1.9.1 (GeoTools) | `POINT (-3.7072 40.4145)`; con `'OGC:CRS84'` **falla** |

**Lo que dice:**
1. **Es un hueco de Lightweight, no de Spark:** GeoParquet guarda siempre x = longitud, y DuckDB por
   defecto escribe lat/lon en 4326. Nadie en ORE fija hoy `geometry_always_xy`: un `ST_Transform` a
   4326 en un Transform de SQL, Python, Node o Java deja los puntos traspuestos en el lago. **El
   remedio es una línea por puesto** (`set geometry_always_xy = true` donde se carga `spatial`), y
   entra en el cierre de C1 con su prueba en los tres puestos. **Hecho** (C1, «x es la longitud,
   siempre»): el hueco también estropeaba las distancias y áreas esféricas de un lon/lat del lago.
2. Con él, los dos motores dan el mismo punto con una diferencia de 4·10⁻⁸ grados (unos 4 mm: PROJ
   frente a GeoTools). Para comparar resultados entre motores se tolera menos de 1 cm.
3. Sedona no reconoce `'OGC:CRS84'`, que es el CRS por defecto de Iceberg para la geografía. El arnés
   de Heavyweight (C2·3) lo dice como `EPSG:4326`, que en Sedona ya es lon/lat.

**Pieza 3 · al CI (hecha en local).** `puesto-spark` entra en `IMAGENES` (`ci/huella-de-la-imagen.py`)
y en las etiquetas que se mueven (`puesto-spark:1` y `en-uso`, en `ci.yml`). Su huella sigue la
cadena: **lo que rehace `puesto-python` rehace `puesto-spark`, y no al revés** (la prueba lo dice con
dos casos nuevos). Con `--comprobar`, las 15 imágenes del CI cuadran con el Dockerfile. Queda medir en
el ensayo del CI, con el empuje del cierre, el disco del *runner*: es la imagen más grande que
construye.

**C2·2, cerrado en local.**

#### C2·3 · Los Builds Heavyweight

**Lo que hay (previo, 2026-10-18).** Un Build es un *trabajo*: el mismo Job de un puesto
(`malla/51-el-puesto.yaml`), que corre una celda —el arnés, por HTTP— y termina. La imagen la decide
el lenguaje del Transform (Java → `jvm`; Python y SQL → `python`), con los recursos fijos de la
plantilla (2 CPU / 4 Gi) y `cola::ENTORNOS` sin `spark`. El Build no lee el `Motor`. `/v1` sabe qué
Transform y qué salida escribe (el puesto que llama), pero no sella nada: `ore.geo.*` llega como
propiedad del cliente (lo calcula el `ore-store` del puesto) y se copia sin mirarla. El 500 de S6
nace en `ore-store` (iceberg 0.10 no abre `geometry`), que sale con 69 y `/v1` lo da por
`CommitStateUnknown`; ore-cli puede negarlo antes con 64, que ya es 400. En el clúster no hay pool
pesado (cuota global 4/32 vCPU), y `cq-victor` tiene 1,5 CPU en `jobs`.

**Decidido (2026-10-18):**
- **La API de Python en Heavyweight son los mismos nombres** (`@transform`, `over`, `write`); `over()`
  da un DataFrame de Spark. Quien sabe escribir un Transform lo reconoce; cada motor rinde con su tipo.
- **El pool pesado es `n2-highmem-8` *spot*** (8 vCPU, 64 GB), con escala a cero: Spark come memoria
  y C0 midió con 64 GB.

| pieza | qué |
|---|---|
| 0 | el previo en el banco (abajo) |
| 1 | el motor elige el entorno: `Donde` lleva el `motor`; Heavyweight → `entorno=spark` (`puesto-spark:1`), con plantilla y recursos propios. TypeScript en Heavyweight, 422 |
| 2 | el arnés PySpark (Python y SQL): `over()` → DataFrame; `write()` pasa la geometría de Sedona a WKB, sella `ore.geo` con su SRID y la procedencia, por `/v1` e idempotente. La identidad, por el `AuthManager` del pod; la memoria, por regla |
| 3 | `/v1` comprueba: S6 es 400 con su remedio; un `ore.geo` que contradiga el contrato del Dataset se niega. Y sirve `…/credentials`, para que un Build de más de una hora no se quede sin GCS |
| 4 | Spark Java (`Dataset<Row>`): **en victor** |
| 5 | Preview como Job corto: **en victor** |
| 6 | el pool `pesado`, su *flavor* de Kueue y su cuota, y la celda en victor: un repositorio Heavyweight corre un *join* espacial: **hecha** |

**Pieza 0 · el puesto pesado de verdad (2026-10-18, `pruebas-de-fuego/geo/c2/trabajo-spark/`).**
El banco de C2·4 (ore-serve con su cola y su plantilla) y, en su red, **el agente de Python dentro de
`puesto-spark`** con el límite del pod (4 GiB, `ORE_MEMORIA_MB`). Ana abre el puesto, el agente lo
reclama y cada celda es PySpark.

| caso | resultado |
|---|---|
| el agente corre una celda que monta Spark | **va**: la sesión arranca en 9–12 s dentro del proceso del agente (la JVM es su hija, por py4j). No hace falta otro agente |
| escribir por `/v1` con la identidad del puesto | **va**: `x-ore-puesto` + la del agente, como el SDK. Sin `ore.procedencia` ni `ore.geo`: Spark no los pone, los pondrá el arnés |
| **la identidad que cambia** mientras corre | `header.*` de Iceberg se lee **una vez**, y el token del pod dura 3600 s (el kubelet reescribe el fichero): un Build de más de una hora se quedaría fuera. **El SPI `AuthManager` de Iceberg 1.12** (`rest.auth.type` = una clase) lo resuelve: `AuthPod.java`, 30 líneas, lee la identidad del fichero **en cada petición**. Medido cambiando el fichero a mitad de sesión: **va · 403 «ese puesto no es de este agente» · va** |
| la credencial prestada | GCS presta ~55 min (`gcs.oauth2.token-expires-at`). El cliente de Iceberg sabe refrescarla (`OAuth2RefreshCredentialsHandler`, `/v1/…/credentials`), **pero ORE no sirve ese endpoint** |
| la memoria (4 GiB, `spark.driver.memory` 2g y 3g) | la JVM ocupa **el heap + ~0,8 GiB**; Python, 129 MB. Un `ST_Contains` de **20M puntos × 20k polígonos**: 9,9M pares en 18 s (2g) y 22 s (3g), pico 2,9 y 3,1 GiB, **sin que lo mate el límite** |

**Lo que dice para las piezas:**
1. **Un Build Heavyweight es el puesto de siempre con otra imagen:** el agente, la cola, el
   reclamo y la celda no cambian. La pieza 1 es elegir `spark` y sus recursos.
2. **La identidad va por `AuthPod`** (en la imagen, como jar), no por `header.x-ore-pod`.
3. **`spark.driver.memory` lo pone el arnés desde `ORE_MEMORIA_MB`**, como la JVM del puesto Java
   (`MaxRAMPercentage=50`): el pod menos ~1 GiB de la JVM fuera del heap y ~0,5 GiB de Python. En el
   pod pesado (~52 GiB de límite en un `n2-highmem-8`), del orden de 44g; se confirma en victor.
4. **`…/credentials` entra en la pieza 3**: sin él, un Build pesado de más de ~55 min falla al leer o
   escribir en GCS.

**Pieza 1 · el motor elige el entorno (hecha en local).** `POST /builds` lee el `motor` de cada
repositorio registrado, y `entorno_del_build` decide: Lightweight, donde siempre; Heavyweight, Python y
SQL en `spark`; Java en Heavyweight, **422 `not_buildable`** hasta Spark Java (pieza 4), porque
correrlo en el puesto ligero sería mentir sobre el motor elegido. En la cola, `spark` es un entorno
con imagen (`puesto-spark:1`) que **lleva la capa de Python** (`entorno_de_la_capa`: su imagen sale de
la de Python, y `traer-la-capa` baja ruedas) y **sus recursos**, puestos sobre los de la plantilla:
pide 7 CPU / 48 GiB y limita a 8 CPU / 48 GiB, con `ORE_MEMORIA_MB` = 49152 (⑱). Pedir 7 CPU hace que
Kueue sólo lo admita en el *flavor* pesado (pieza 6): hasta entonces, un Build Heavyweight se queda en
la cola, que es la verdad. Una plantilla que no traiga los recursos de siempre no se adivina (error).
No se abre una sesión `spark`: el pesado lo elige un Build, nunca un lenguaje. Hasta la pieza 2, el
Build pesado corre el arnés de siempre (DuckDB) en la imagen pesada. De paso, `h3` sale de las
sugeridas de Python, porque desde C1·2 lo trae la imagen.

**Pieza 2 · el arnés PySpark, para Python (hecha en local).** La celda de un build Heavyweight dice
antes su motor (`con_el_motor`: `ORE_MOTOR=heavyweight`), y con él **los mismos nombres del SDK son
Spark** (`puesto/python/ore/spark.py`): `over()` da un DataFrame de Spark, `sql()` es Spark SQL y
`write()` escribe un DataFrame de Spark. **Spark escribe sus ficheros y hace el commit por `/v1`; lo de
ORE lo pone el SDK:**
- **la geometría**: cada columna de Sedona sale como WKB, y la tabla dice `ore.geo.<col>` sacado de
  los datos en una pasada (un SRID por columna, o no se escribe, como en Python, Node y Java; el
  subtipo, o `Any`). Al leer, al revés: lo que tiene `ore.geo.<col>` llega como geometría con su
  SRID (`OGC:CRS84` como 4326, que en Sedona ya es lon/lat);
- **la procedencia**, en el resumen del snapshot (`snapshot-property.ore.procedencia`), la misma que
  `write()`, con el build y lo leído;
- **el techo y el Preview**, antes del motor: lo que no se escribe no se escribe con ninguno.
La identidad va por `IdentidadDelPuesto` (el `AuthManager` de la pieza 0, compilado en la imagen; en
el laboratorio, la de cabecera) y el heap sale de `ORE_MEMORIA_MB` (el pod menos 1,5 GiB, y el 80 %:
4 GiB → 2 GiB, 48 GiB → 37 GiB). Medido antes de escribirlo (`pruebas-de-fuego/geo/c2/arnes-spark/`):
`createOrReplace`, `overwrite`, `append`, `tableProperty` y `snapshot-property` van por `/v1`, y
Sedona no toma un `binary` como geometría sin convertir. **De punta a punta, 15/15** (`medir.sh
sdk.py`): escribir puntos y leerlos como geometría con su SRID, un transform de *buffers* (31 214 m²,
lo de DuckDB) con su procedencia, `append` sin perder `ore.geo`, y se niegan la geometría sin CRS, dos
CRS en una columna, `upsert`, un pandas y escribir fuera del techo. El SDK, 154/154 en la imagen.

**Lo que queda, con nombre:**
- **2b · SQL en Heavyweight.** En una sentencia, las entradas se nombran dentro del texto: llegan
  como WKB y `ST_*` no las toma (`over()` sí las convierte). Necesita su diseño (la sentencia ya la
  analiza ORE, que sabe sus `inputs`).
- **`/v1` contesta 400 a un namespace de dos niveles**, y Spark busca así sus tablas de metadatos
  (`t.snapshots`): con un 404 las encontraría. Va a la pieza 3.
- Las **promesas de columna** quedan `unknown` en Heavyweight (las de filas, como `notEmptied`, se
  juzgan igual): el SDK no las cuenta todavía. `upsert` y las tablas ancladas, tampoco.

**Pieza 2b · SQL en Heavyweight: el diseño (aprobado 2026-10-18).** En una sentencia, las entradas
se nombran dentro del texto, y Spark las lee del catálogo como WKB. **ORE reescribe los nombres:**
la consulta para Spark es el mismo texto con cada nombre del árbol que lee sustituido, por su
*span*, por una vista temporal (`ore__<base>__<schema>__<nombre>`) que se registra con `over()`, así
que la geometría se convierte en un solo sitio y nada se reimprime. Descartadas: una regla de
Catalyst que convierta toda tabla de ORE al leerla (Spark resuelve las referencias del padre en la
misma pasada en que nace la relación: la regla cambiaría el tipo de una columna ya resuelta como
`binary`), y que quien escribe convierta a mano. **El dialecto** es el de C0 (P2: ORE no traduce):
el contrato se describe en DuckDB (P3) y lo corre Spark; lo que sólo es de DuckDB falla en el build.

**2b·0 · el previo (`pruebas-de-fuego/geo/c2/sql-spark/`).**
1. **La reescritura por *spans*** (`sonda-spans`, el sqlparser 0.63 de `ore-core` y su regla de las
   CTE): **10/10**. Dos y tres partes, entre comillas, el mismo nombre dos veces, una CTE que se
   llama como una tabla, subconsultas e `IN`, nombres en cadenas y comentarios intactos, varias
   líneas y acentos (las columnas de sqlparser cuentan caracteres, no bytes). **Un hallazgo:** una
   columna cualificada con el nombre entero de la tabla (`geo.lugares.id`) se queda sin su tabla tras
   reescribir el `from`: 2b·1 reescribe también ese prefijo.
2. **El SQL escrito como en DuckDB, en Spark sobre las vistas** (`dialecto.py`): **18/22 van tal
   cual**, entre ellos el *join* con `ST_Contains` y `group by`, `ST_DWithin`, `ST_Distance`,
   `ST_Transform` con 3 argumentos y con 2 (lon/lat), `ST_Union_Agg`, `::`, `ilike`, `count(*)
   filter (where …)`, `string_agg`, `array_agg`, `date_trunc`; y el SRID llega por la vista.
   **No van en Spark:**

| DuckDB | en Spark |
|---|---|
| `qualify …` | una subconsulta con la ventana y un `where` |
| `select * exclude (c)` | `select * except (c)` |
| `list(x)` | `array_agg(x)` (o `collect_list`) |
| `h3_latlng_to_cell(lat, lng, r)` y los `h3_*` | `ST_H3CellIDs(geom, r, false)` y los `ST_H3*` de Sedona |

**2b·1 · `consulta_para_spark` (hecha en local).** En `ore-core` (`sql_del_arbol`), junto a quien ya
sabe qué lee una sentencia: devuelve el texto con cada nombre del árbol que lee sustituido por su
vista (`` `ore__<base>__<schema>__<nombre>` ``, una parte entre comillas invertidas) y las vistas que
hay que registrar, `(vista, nombre corto)`, una vez cada una. La regla de qué es una lectura es la de
siempre (un `from`/`join` de dos o tres partes que no es una CTE), y la columna cualificada con el
nombre entero pasa a la vista. Corta por caracteres, no por bytes. Probada con los casos de la
sonda y los de las columnas cualificadas; `ore-core` 384/384, clippy limpio.

**2b·2 · `sql()` pesado (hecha en local).** **La celda de un SQL transform no cambia:** ya llama
`sql(consulta)`, `_por_posicion` y `_como_la_tabla`, y con el motor pesado cada uno despacha a Spark.
`sql()` pide a ore-serve la consulta para Spark (`POST /puestos/{id}/sql` con `para: "spark"`: el
mismo reclamo del puesto, y nada de las fuentes de DuckDB), registra cada vista con `over()` y la
corre; lo mismo vale para `ore.sql()` desde un `.py`. El `insert` tiene su versión en Spark: una
columna sin nombre toma la de la tabla en su posición, y cada columna va al tipo de la suya (salvo la
geometría, que la tabla guarda en WKB y `write()` convierte). `ore-serve` 246 (los dos de `git` del
*worktree*, aparte), el SDK 160/160 en la imagen, y el laboratorio del SDK pesado sigue 15/15.

**2b·3 · un Build Heavyweight de punta a punta (2026-10-18, `pruebas-de-fuego/geo/c2/build-pesado/`).**
El banco con el `ore-serve` de C2·3, dos datasets geo sembrados con el SDK pesado, un repositorio
`transforms-sql` y otro `transforms-python`, los dos `motor: heavyweight`; `ore transforms generate`
y `ore validate` limpios; `POST /builds` como ana, y **el agente del trabajo en `puesto-spark`**, como
el Job.

| build | resultado |
|---|---|
| SQL · `ST_Contains` entre dos tablas geo, con `group by` | **va** en Spark (`entorno: spark`): 4 filas, **las mismas que sin Spark** (shapely con la resolución de Sedona: `{0: 4, 1: 7, 2: 7, 3: 3}`), y la procedencia con el build y sus dos entradas |
| SQL · una geometría calculada (`ST_Centroid`) | **va**: WKB con `ore.geo.centro` = `Geometry<Point, EPSG:25830>`, sacado de los datos |
| Python · `over()` → `ST_Difference(ST_Buffer(…))` → `write()` | **va**: `Geometry<Polygon, EPSG:25830>` |
| Python · lee lo que no declara | **se niega** (403: «no es una entrada … ni su output: un transform sólo lee lo que declara»), y no queda nada |

**Lo que destapó:**
1. **Spark lee sus entradas por `/v1`, y «lo declarado manda» lo negaba**: mientras corre un
   transform, el catálogo sólo cargaba su output (leer iba por `datos`, que Spark no tiene). Arreglado
   en el catálogo: **una entrada declarada se carga con el mismo conducto que `datos`
   (`lectura_desde_puesto`) y con la credencial que sólo lee**; lo demás sigue siendo 403, y una
   lectura negada se dice como lectura. El laboratorio de la pieza 2 corría como ana, sin techo: no
   lo vio.
2. **`ore.modo`**: Spark no lo ponía, y ORE lo mira para saber si una escritura reemplazó lo que
   había. El SDK pesado lo pone (`sobrescribir` / `anexar`, las palabras de `ore-store`).
3. **El árbol registraba la geometría como `String`** (`centro: { type: String }` en el `Dataset`
   que ORE escribe). Era la deuda de C1 (el contrato del árbol tras un Build), en los dos motores.
   **Arreglado (2026-10-18):** `lago::columnas_oos` lee `ore.geo.<col>` —su `type` si OOS lo entiende;
   si sólo dice el CRS, `Geometry` o `Geometry<Any, CRS>`—, y como esos tipos son de OOS v1alpha32,
   el documento que ORE escribe sube a v1alpha32 cuando lleva una columna geo
   (`con_la_version_de_sus_tipos`: medido, en v1alpha12 no compila, `OOS3008`). En el laboratorio, las
   entradas dicen `Geometry<Point, EPSG:25830>` y `Geometry<Polygon, EPSG:25830>`, las salidas de los
   builds `Geometry<Point, EPSG:25830>` y `Geometry<Polygon, EPSG:25830>` en v1alpha32, la que no
   tiene geometría sigue en v1alpha12, y `ore validate` pasa después de los builds. Queda verlo en
   victor al cerrar C1.

**Pieza 3 · `/v1` comprueba (hecha en local, `pruebas-de-fuego/geo/c2/v1-comprueba/`, 12/12 con
Spark contra el banco).**

| qué | cómo |
|---|---|
| **S6: el `GEOMETRY` nativo de v3 → 400** | `ore datasets` mira el cuerpo antes de tocar el lago (crear, esbozar, commit): un tipo `geometry(…)`/`geography(…)` en un campo, una lista o un mapa es un 400 que dice cómo: «escríbela con `ST_AsBinary` y di su CRS en `ore.geo.<col>`». Ya no es un 500 `CommitStateUnknown`, y no queda nada |
| **`ore.geo.<col>` se sostiene o no entra** | ha de ser un objeto JSON con su `crs` (`AUTORIDAD:CÓDIGO`) y, si dice `type`, un tipo geo de OOS con ese mismo CRS. Al crear y en un `ALTER`: 400 con el porqué. Es de donde sale el tipo del árbol, y hasta hoy no lo miraba nadie |
| **un namespace de dos niveles → 404** | en ORE no hay ninguno (la base es el `prefix`): lo que se busque en él no existe. Con el 404, Spark encuentra sus tablas de metadatos (`ore_geo.default.lugares.snapshots`); con el 400 de antes, fallaban |
| **el porqué que se anuncia, entero** | un error cuya primera línea acaba en «:» llega con las líneas que lo explican (acotado): «no compila con esas columnas:» llegaba sin el diagnóstico |
| **`GET …/tables/{t}/credentials`** (`loadCredentials`) | por el **mismo camino que `loadTable`** (conducto, techo, la de leer para una entrada): las `storage-credentials`, sin bajar el `metadata.json`. Y `loadTable` y crear la tabla **anuncian dónde refrescar, sólo lo que caduca**: la credencial de GCS (`gcs.oauth2.token-expires-at`) con las claves de GCS, una de S3 temporal (`s3.session-token`) con las de S3. Las claves fijas de R2 no: anunciarles un refresco hace que `S3FileIO` exija un `session-token` que no tienen (medido: dejaba de leer) |

Medido además: el `VendedCredentialsProvider` de Iceberg pide `…/credentials` con la identidad del
catálogo y lo lee (con las claves fijas del banco, dice que no son temporales, que es lo esperado).
**Lo que sólo se ve en victor:** el refresco de GCS de verdad, en un build de más de una hora (el
manejador de GCS se identifica con un `AuthManager`, el de la identidad del puesto).

**Queda de esta pieza, con nombre:** mezclar dos CRS en una tabla a lo largo de escrituras (un
`append` en otro CRS) no se niega en el servidor; el `ore.geo` de cada escritura lo pone quien
escribe, de sus datos.

**Pieza 4 · Spark Java (hecha en local).** Un Transform de Java en un repositorio Heavyweight se
escribe con **los mismos nombres** (`import static ore.Ore.*`, `@Transform`, `over`, `sql`, `write`),
y `over()` y `sql()` dan un **`Dataset<Row>`** con la geometría de Sedona y su SRID:

```java
@Transform(inputs = {"geo.zonas"}, output = "geo.anillos_java")
public static Object anillos() throws Exception {
    Dataset<Row> z = over("geo.zonas");
    return write("geo.anillos_java", z.selectExpr("id", "ST_Difference(ST_Buffer(zona, 50), zona) AS anillo"));
}
```

**Corre en la JVM de Spark del puesto pesado, y lo de ORE sigue en un solo sitio.** El build va a
`spark`, con el agente de Python, como uno de Python. Su celda es de Python
(`arnes_java_en_spark`): pone el motor y la procedencia y llama a `_ore._java_en_spark`. Este paso,
`ore.ArnesDeSpark` (en `ore-spark-sdk.jar`, dentro de la imagen), compila el fichero con javac
**dentro del driver**, con el classpath del driver y, del repositorio, sólo lo que usa (lo de
Lightweight). Luego carga la clase con D15 armado y halla el `@Transform`. Python le pone el techo
(el `transform()` de siempre, con lo que declara la anotación) y lo llama. El `ore.Ore` de
Heavyweight sólo reenvía: `over`, `sql` y `write` vuelven al SDK de Python por py4j (`ore.Puente`,
que implementa Python), y los `Dataset<Row>` van por referencia. **La geometría con su CRS, la
procedencia, el techo y `ore.modo` tienen una sola implementación**, y los dos lenguajes dan lo
mismo porque es la misma. `@Transform` y el JSON son los ficheros de Lightweight, copiados en la
imagen: un `@Transform` es el mismo en los dos motores (el servidor lo deriva igual).

Descartado: una imagen pesada con el agente de JShell (el classpath del puesto Java, con Arrow 19 y
Jackson, choca con los jars de Spark, y haría falta otra imagen), y reescribir en Java lo que el SDK
de Python ya hace sobre Spark (dos implementaciones del contrato geo).

**El previo** (`pruebas-de-fuego/geo/c2/spark-java/previo.py`, sin red y como uid 65532): **10/10**.

| caso | resultado |
|---|---|
| javac dentro del driver de Spark | **va**: 0,9 s en frío y 0,8 s en caliente. El lanzador de Spark ya da el classpath expandido, jar a jar |
| Java pide a Python y recibe un `Dataset<Row>` (py4j, al revés) | **va**: `over → selectExpr(ST_Buffer) → write` da 31 214 m² |
| un error de compilación, un NPE del usuario, un `AnalysisException` | llegan con la línea de su fichero |
| un `PermissionError` de Python (el techo) desde Java | llega como `Py4JException` con el traceback; el SDK lo guarda y lo dice como Python lo dijo |

**De punta a punta** (`build-pesado/medir.sh`, el agente en `ore-puesto-spark:local`): cuatro builds
de Java junto a los de SQL y Python.

| build | resultado |
|---|---|
| `geo.anillos_java` (`over` + una clase de apoyo) | **succeeded**, 4 filas, `ore.geo.anillo` `Geometry<Polygon, EPSG:25830>` y su procedencia; **igual que el de Python** (54 852 m²) |
| `geo.conteo_java` (`sql()` con `ST_Contains`) | **succeeded**, **igual que el SQL y que shapely** (`cotejar.py`) |
| `geo.fuera_java` (lee lo que no declara) | **failed**, `runtime` en su línea: el catálogo lo niega, como en Python |
| `geo.rompe_java` (un NPE) | **failed**, `runtime`, línea 12, con el mensaje de Java |

**Los fallos** (`spark-java/fallos.py`, con el SDK de verdad y sin banco): **8/8**. Van al informe,
cada uno con su tipo, su fichero y su línea:
- no compila (en su fichero o en el de apoyo);
- `not-a-transform`;
- D15 (`over()` en un `static`);
- `write()` fuera del techo;
- una geometría sin CRS;
- un `AnalysisException`;
- un método que no escribe, que se dice como en Lightweight.

`comprobar.py` de la imagen carga `ore.Ore`, `ore.ArnesDeSpark`, `ore.Transform` y `ore.Puente`
como uid 65532. El SDK, 160/160 en la imagen; ore-serve, `builds` 20/20.

**Queda, con nombre:**
- el editor de un repositorio Java Heavyweight: el puesto de sesión es el de Java ligero, y su
  `ore.Ore` da `Rows`. El código con `Dataset<Row>` no compila allí (el Preview de Java pesado es
  la pieza 5);
- `upsert` y las tablas ancladas, como en Python.

**Pieza 5 · el Preview, un Job corto (hecha en local).** El Preview de un fichero de un repositorio
Heavyweight **no corre en la sesión**: corre en un trabajo de `spark`, el puesto pesado de su build,
porque la sesión no tiene Spark. Es el mismo Preview de siempre:
- el texto del editor, sin guardar;
- el techo de lo que ese código declara;
- `write()` no escribe, y lo que escribiría va al informe.
Sólo cambia dónde corre.

| qué | cómo |
|---|---|
| **la ruta, la de siempre** | `POST /puestos/{sesion}/preview`. Si el repositorio del fichero (registrado en `main`) es Heavyweight, ORE lanza el trabajo (`ensayar_en_spark`) y contesta **202 `{trabajo, celda: 1, preview}`**. La sesión sólo dice de quién es y en qué rama se lee |
| **sin escribir nada** | la celda del trabajo lleva su `Ensayo`, como la de una sesión: el servidor pone el techo al empezarla y la puerta del Preview (`puerta_del_preview`) niega cualquier escritura hasta su salida. **Y el informe del trabajo no va al árbol** (`trabajos/<id>.json`): un Preview no deja nada |
| **la celda** (`celda_de_preview_pesado`) | Python: el arnés del Preview con el motor dicho. Java: `arnes_java_en_spark` en modo Preview (sin procedencia, con la salida que declara el editor) |
| **el `write()` ensayado en Spark** (`spark.ensayar_escritura`) | lo que comprobaría `write()`: un DataFrame de Spark, el modo y un CRS por columna geo. Y la forma del Preview de Lightweight: las primeras 100 filas (la geometría en **WKB en hexadecimal**, cierre de C1), el total y el tipo con que nacería cada columna (la geometría, `binary` con su `ore.geo`) |
| **SQL** | **422 `not_supported`**, como en Lightweight (0055 P4) |
| **la consola** | espera la celda en `trabajo` si lo hay (`GET /puestos/{trabajo}/celdas/1`), y si no en la sesión. Lo demás, igual |

**Medido de punta a punta** (`pruebas-de-fuego/geo/c2/preview-pesado/`, el agente del trabajo en
`ore-puesto-spark:local`, con el texto del editor cambiado): **7/7**.
- Python y Java ensayan el texto del editor (un *buffer* de 80 m y una columna más que no están en
  el árbol). Cada uno da 4 filas, la geometría `Geometry<Polygon, EPSG:25830>` y su primera fila en
  WKT.
- Leer lo que no declara, escribir fuera del techo (Java) y no compilar fallan con su tipo, su
  fichero y su línea.
- SQL da 422.
- Al final no existe ninguna de las salidas y el árbol no tiene ningún informe de trabajo.
- **El agente, de arrancar a terminar: 13–25 s**. Unos 10 s son la sesión de Spark: en un fallo de
  compilación de Java, Spark arranca antes de compilar.

**Queda, con nombre:**
- **lo que tarda de verdad**: en producción se suman el *pod* y, con el pool a cero, el nodo (pieza
  6). Un Preview pesado que tarda un minuto en un nodo frío es de esperar: no es la sesión ligera;
- el editor de Java de un repositorio Heavyweight (el LSP de la sesión es el de Java ligero);
- comprobar en el navegador que la consola lo enseña (sin ORE con Heavyweight detrás, no se puede
  ver aún).

**Pieza 6 · el pool pesado y la celda en victor (hecha, 2026-10-19).** Tres pasos, cada uno con su go.

*6a · empujar.* C1 y C2 (piezas 0–5) a `main` tras la suite completa, y la consola (el paso *Select
compute* y el Preview que espera en su trabajo). victor quedó en `04a837d6` con `puesto-spark:1` en el
registro. Dos arreglos por el camino: el `@_kw` de `write` fuera de sitio y las pruebas de `h3`, que
se saltan donde no está la extensión (el *runner* trae `spatial` y no `h3`).

*6b · la malla* (`1b614dde`):

| qué | cómo |
|---|---|
| **el pool** | `pesado`: `n2-highmem-8` *spot*, de 0 a 2 nodos, *taint* `ore.dev/pesado`, 100 GB `pd-balanced`, *image streaming*. Lo crea `malla/73-el-pool-pesado.sh` (en seco por defecto; `APLICAR=1`), que antes mira la máquina y la cuota |
| **el *flavor*** | `pesado` en `malla/10-kueue.yaml`: el `nodeLabel` `ore.dev/pool=pesado` y la tolerancia |
| **la cola** | por celda, `cq-<celda>-pesado` **sin cohorte** (7 CPU / 48 GiB: un Build pesado a la vez por celda) y su LocalQueue `cola-pesada`; `cola.rs` manda ahí el trabajo `spark` |
| **la cuota** | la ResourceQuota del inquilino sube a 12 CPU / 64 GiB: con 36 GiB el pod pesado no cabía |
| **dormir** | `dormir.sh` y `dormir-de-noche.py` también llevan el pool a 0 |

*6c · la celda* (`pruebas-de-fuego/geo/c2/celda-victor/`). Primero sin consola: un Job de medida en
`cola-pesada` con `puesto-spark:1` corre el *join* (`frio.sh`, `join.py`). Después, desde la consola,
tres repositorios Heavyweight (`consola/`): `sembrar.py` genera los datos en Spark, y el conteo se
escribe en SQL, en Python y en Java.

| medida | valor |
|---|---|
| **en frío** (el pool a 0) | **115 s**: el nodo, listo a los 46 s; la imagen, 1,8 s con *streaming*; la sesión de Spark, 16 s; el *join*, 26 s |
| **en caliente** | **49 s** |
| el *join* | 20 M × 20 k → **11 129 901 pares**, pico de 10 GiB en un pod de 48 GiB |
| los Builds de la consola | 23–43 s cada uno, en el nodo pesado. **Los tres conteos, iguales** (cotejados con un Preview) |
| el nodo | se retira unos 11 min después del último Job |
| **el coste** | *spot* `n2` en EMEA: 0,00788 $/vCPU·h y 0,001054 $/GiB·h, unos **0,13 $/h** el nodo; un Build en frío, unos **0,03 $** |
| el Preview pesado | 11–15 s con el nodo caliente |

**Lo que arregló la celda** (encontrado en victor, no en el laboratorio):
- **el id de un puesto cabe en 63** con cualquier entorno (`e9772bec`): el Job de un puesto de
  notebooks medía 70, y como Flux aplica la cola de golpe, un Job que no valida paraba la cola entera
  de la celda;
- **la muestra del informe de un Build pesado** (`54947f1e`): `table()` no conocía el DataFrame de
  Spark y el informe decía «could not be read». Ahora trae sólo la cabeza, con la geometría en WKB en
  hex, y cuenta el total sólo si no cabe.

**Queda, con nombre** (ninguna para C2):
- **un reinicio de ore-serve pierde los Builds en curso**: guarda en memoria los puestos y los
  trabajos vivos, y al arrancar barre la cola;
- **el editor de un repositorio Heavyweight**: en Python, pyright no resuelve `pyspark` (la sesión es
  `puesto-python`, sin él; o `pyspark-client` o sus tipos); en Java, el LSP es el de Java ligero;
- `upsert` y las tablas ancladas, en Heavyweight;
- la muestra de Lightweight da la geometría en base64 (va con la consola, cierre de C1);
- la deriva de victor cuenta `jwks-pods`, que es el mismo caso que `jwks` (exento).

### C3 · El *join* espacial a escala

| paso | qué |
|---|---|
| C3·1 | el layout del lago que lo acelera: Hilbert (ya está) y, si la vara lo pide, una columna H3 |
| C3·2 | el *join* eficiente en cada motor: R-tree en DuckDB; particionado o H3 con refinado en Spark |
| C3·3 | lo que ORE recomienda o genera cuando ve un *join* espacial |

**Hecho cuando:** los casos de *join* de la vara cumplen su objetivo en los dos motores.

#### C3 · El previo (2026-10-19, medido en victor)

Medido en el pool `pesado` de victor (`n2-highmem-8` *spot*), con `puesto-spark:1` y los recursos de un
Build pesado (7 CPU / 48 GiB, heap 38 g). Sin persona y sin lago: los datos los genera Spark dentro del pod.
Laboratorio en `pruebas-de-fuego/geo/c3/previo/`: `en-victor.sh` corre un guion como Job de medida en
`cola-pesada`, y `multinodo.sh` corre N pods en el namespace de medida `c3-medida`, que se borra al
acabar. En local no se puede: WSL tiene 6 GB y el caso grande tumbó Docker dos veces.

**1 · Cuándo difunde Spark el lado pequeño, solo** (`difusion.py`, 20 M puntos):

| caso | lo que estima Spark de los polígonos | *join* | tiempo |
|---|---|---|---|
| 6c (20 k buffers), en memoria | 1,1 MB | difunde | 14,0 s |
| 6c, desde Parquet (como lee un Build) | 4,4 MB | difunde | 14,2 s |
| «municipios» (8 000 polígonos de 513 vértices), en memoria | 0,4 MB | difunde | 15,9 s |
| **«municipios», desde Parquet** | **37,2 MB** | **`RangeJoin`** | **28,2 s** |
| «municipios», desde Parquet, difundido a mano | 37,2 MB | difunde | 17,7 s |

Spark difunde si **el tamaño de los ficheros** del lado pequeño baja de
`spark.sql.autoBroadcastJoinThreshold`, que son 10 MB; `sedona.join.autoBroadcastJoinThreshold` no está
puesto. Unos polígonos detallados pasan de ahí con pocas filas, y el *join* tarda 1,6× más. En 6c entró
por poco: 20 k círculos simples ocupan 4 MB. **La regla de ORE hace falta**, y tiene que mirar las
filas, no los bytes en disco.

**2 · Q10, el `LEFT JOIN` espacial** (`q10.py`, 20 M puntos y 22 k zonas, 2 000 de ellas sin puntos):

| forma | *join* | tiempo |
|---|---|---|
| `zonas LEFT JOIN puntos` (la de SpatialBench) | `BroadcastNestedLoopJoin` | > 300 s |
| `puntos LEFT JOIN zonas` | `BroadcastNestedLoopJoin` | > 300 s |
| **reescrita: el *join* interno y, aparte, las zonas sin pareja con 0** | `BroadcastIndexJoin` + `SortMergeJoin` por `id` | **16,4 s**, las 22 000 zonas y las 2 000 vacías |

Sedona 1.9.1 **no indexa un *join* externo con predicado espacial, en ningún sentido**: cae en el
producto cartesiano. El fallo de C0 a los 8 s era eso mismo, sin memoria. La reescritura da el mismo
resultado y usa el índice. Además, **cancelar un producto cartesiano no para sus tareas**, que son un
bucle de CPU: la medida siguiente salió contaminada y hubo que repetirla sola.

**4 · El coste de medir a escala 100.** El dinero no limita (*spot* `n2`: 0,00788 $/vCPU·h y
0,001054 $/GiB·h); la cuota sí, con 28 de 32 vCPU libres:

| opción | nodos | vCPU | cuota | la sesión (~3 h) |
|---|---|---|---|---|
| A · el pool de hoy | 2 × `n2-highmem-8` | 16 | cabe | ~0,8 $ |
| B · el objetivo de C0·5 | 2 × `n2-highmem-16` | 32 | **no** (hacen falta 36: pedir 48) | ~1,6 $ |

A eso se suma generar la escala 100 (17 min en C0) y guardar sus 56 GB mientras dure (céntimos). De
C0 solo quedan los resultados en el bucket: los datos se borraron con su pool.

**El *approach* que propone el previo:**

| pieza | qué |
|---|---|
| **C3·a · la regla de difusión** | en un *join* espacial, ORE difunde el lado que tenga **pocas filas** (en el catálogo están las cuentas de cada dataset), sin mirar sus bytes en disco. Se pone con `sedona.join.autoBroadcastJoinThreshold` o con la pista `broadcast`. El techo, medido: cuánta memoria se come el índice difundido de un lado grande |
| **C3·b · el `LEFT JOIN` espacial** | ORE lo reescribe en la reescritura que ya hace para Spark (`consulta_para_spark`, 2b): el *join* interno más las filas sin pareja (*anti join* por clave). Mientras Sedona no lo indexe |
| ~~C3·c · el Build en varios nodos~~ → **CE, apartado propio** | el Build pesado pasa a ser el Job indexado de N pods de arriba; un Job `batch/v1` de N pods ya lo admite Kueue de una vez. Hay que subir `cq-<celda>-pesado` y la cuota del inquilino a N × 7 CPU / 48 GiB, y el pool a N nodos |
| **C3·d · la vara a escala 100** | SpatialBench entero con C3·a, C3·b y el Build elástico (CE), con **la opción A** (2 × `n2-highmem-8`, el pool de hoy; elegida el 2026-10-19). El objetivo de C0·5 era con 32 vCPU: se mide con 16 y se dice |

**C3·a · difundir lo pequeño (hecho en local, 2026-10-19).** `over()` marca para difundir
(`F.broadcast`) un dataset **geo** con **≤ 150 000 filas y ≤ 48 MiB**, según el snapshot vigente que da
el catálogo (`total-records`, `total-files-size`; los dos de la misma llamada a `/v1` que ya hacía).
La marca sigue en SQL, porque sus vistas salen de `over()`, y en Java, que llama a Python. Sin esos
datos no se marca nada y Spark decide como siempre. `ORE_DIFUNDIR_FILAS` y `ORE_DIFUNDIR_BYTES` lo
cambian (para las pruebas). El umbral, medido en victor (`pruebas-de-fuego/geo/c3/a/memoria.py`, 20 M
puntos, heap 38 g):

| polígonos | Parquet | difundidos | `RangeJoin` (sin difundir) |
|---|---|---|---|
| 8 000 detallados | 37 MB | **17,7 s** | 28,2 s |
| 100 000 | 21,6 MB | **24,9 s**, pico 18,9 GB | 30,6 s, 8,8 GB |
| 300 000 | 64,2 MB | 37,4 s, 27,8 GB | **30,5 s**, 10,8 GB |
| 1 000 000 | 184,6 MB | 113 s, 41 GB de 48 | **36,9 s**, 19 GB |
| 5 000 000 | — | sin memoria | — |

Difundir sólo compensa con el lado pequeño de verdad. Por encima, la partición espacial de Sedona
(`RangeJoin`) es más rápida y gasta la mitad: **el umbral es una frontera de velocidad, no sólo de
memoria**. Pruebas: `test_motor_pesado.py` (`LaDifusion`) y los casos 9 y 9b del arnés de Spark
(20/20).

**C3·b · el `LEFT JOIN` espacial (hecho en local, 2026-10-19).** Antes de escribir nada, la
documentación de Sedona (Optimizer) aclaró el previo: Sedona **sí** indexa un join externo espacial si el
lado que no se conserva va difundido («Left outer - broadcast right»), y documenta una reescritura para
cuando no se puede (Q10: zonas `LEFT JOIN` viajes, con los viajes como lado grande). Medido a mano
(`c3/b/formas.py`, 20 M puntos):

| forma | *join* | tiempo |
|---|---|---|
| `puntos LEFT JOIN zonas` con las zonas marcadas (lo que ya da C3·a) | `BroadcastIndexJoin` | 25,6 s |
| Q10 a la Sedona, con el id que ya tiene la tabla | `BroadcastIndexJoin` + igualdad | 15,5 s |
| Q10 a la Sedona, con id numerado (en un CTE) | lo mismo | 33,1 s |

Las tres dan las mismas cuentas. **La regla** (`puesto/spark/ReglasDeOre.java`, en `spark.sql.extensions`,
compilada en `ore-spark-reglas.jar` por la etapa `puesto-spark`) entra en el analizador (post-hoc):
- detecta un `LEFT` o `RIGHT JOIN` con un predicado de Sedona que mira a los dos lados;
- lo deja como está si el lado que hay que difundir ya va marcado (también bajo una vista o un alias,
  como lo deja `over()`), porque Sedona ya lo indexa;
- si no, lo reescribe: el lado conservado se numera en un CTE (`__ore_k`, `monotonically_increasing_id`;
  Spark calcula una vez un CTE no determinista usado dos veces, así que el número es el mismo), un *join*
  interno espacial devuelve ese número y las columnas del otro lado, y un join externo por igualdad las
  une. Salen las mismas columnas, en el mismo orden.

Probado en local (`c3/b/regla.sh`, 6/6: se reescribe, da lo mismo que la forma a mano, `select *`
conserva columnas y filas, `RIGHT JOIN` igual, no toca lo difundido ni un `LEFT JOIN` sin predicado
espacial), en el arnés de Spark con la regla puesta (20/20) y en `comprobar.py` de la imagen, que ahora
la exige. **En victor, Q10 con 20 M puntos: 34,3 s** (`RIGHT JOIN`, 44,8 s), con las mismas cuentas. El
previo, sin ella: más de 300 s.

**C3·b2 · la rejilla sobre el lado grande (hecho; empujado con CE·2, 2026-10-20).** Es la defensa
del *join* espacial contra el sesgo que midió CE·0 (punto 5): la rejilla de Sedona por defecto deja un
solo core trabajando.
- (`ReglasDeOre.LadoGrandeALaDerecha`, una regla del optimizador). En un join
  interno espacial no difundido pone a la derecha del predicado el lado con más filas (o bytes):
  gira `ST_Contains` ↔ `ST_Within`, `ST_Covers` ↔ `ST_CoveredBy`, y a `ST_Intersects` y `ST_DWithin`
  les da la vuelta. `spark-defaults.conf` hace la rejilla sobre la derecha
  (`sedona.join.spatitionside right`): la configuración medida en CE·0. `ST_Contains(zona, punto)`, la forma
  de siempre, no se toca. `c3/b/regla.sh`: 10/10 (los 6 de C3·b, más los 4 nuevos).

**C3·d · SpatialBench a escala 100 con el Build elástico (medido en victor, 2026-10-20).**
- **Cómo:** la escala 100 se generó en el pool `pesado` (`pruebas-de-fuego/geo/c3/d/generar.sh`) y se
  subió al bucket del banco, `spatialbench/sf100/`. Hay 56 GB, de los que `trip` son 39,4 GB.
  - El driver va en `pesado-driver`, con 1 a 2 ejecutores de 7 cores en `pesado` (`medir.sh`).
  - Imagen desplegada `puesto-spark:1`, con `ReglasDeOre`: la regla de C3·b y la rejilla sobre el lado
    grande. Las consultas son el SQL de SpatialBench tal cual (commit `8e44bdd`), sin las pistas de C0.
  - Límite de 600 s por consulta y huella al estilo de C0.
  - El Parquet se lee con `gcs-connector` 3.0.4 *shaded*, que con el Hadoop de Spark 4.1 necesita
    `parquet.hadoop.vectored.io.enabled=false`.

| consulta | C0 Spark, 1 nodo de 16 vCPU | C3·d, 2 × 8 vCPU | huella |
|---|---|---|---|
| Q1 | 68,6 s | **62,3 s** | = C0 |
| Q2, el SQL original | > 600 s (125 s con la pista de difusión, q2d) | **> 600 s** | — |
| Q3 | 63,5 s | 63,3 s | = C0 |
| Q4 | 75,3 s | 64,4 s | = C0 |
| Q5 | 345 s | **216 s** | = C0 |
| Q6 | > 600 s | **564 s; 504 s repetida** | 19 filas como DuckDB (ver abajo) |
| Q7 | 79,6 s | 157 s | = C0 |
| Q8 | > 600 s | **438 s** | = DuckDB |
| Q9 | 6,5 s | 6,5 s | = C0 |
| Q10 | error | **> 600 s, sin error** | — |
| Q11 | sin memoria | > 600 s; **desalojo por disco** | — |
| Q12 | > 600 s | > 600 s | — |

**Lo que dice:**
1. **Ocho de doce terminan, dos más que en C0**: Q6 y Q8, en 2 nodos de 8 vCPU (la opción A, 16 vCPU
   en total; el objetivo de C0·5 era con 32). Q10 ya no falla: llega al límite. Todo lo que termina da la
   huella de C0 o la de DuckDB.
   - Q6 sale con las 19 filas de DuckDB, pero la huella no coincide.
   - C0 solo guardó dos filas de DuckDB, y las dos son idénticas a las nuestras.
   - La repetición da la misma huella, así que es determinista.
   - **Queda abierto:** cotejar las 17 restantes con DuckDB. El sospechoso es el redondeo a 0,1 s del
     promedio de un intervalo, que C0 ya vio en los microsegundos.
2. **El autoescalado responde en producción:**
   - de 0 a 2 ejecutores en 20 s al arrancar;
   - perdidos los dos a la vez, el driver pidió otros, esperó a que el *autoscaler* les hiciera sitio
     (pool `pesado` al máximo, 2 nodos) y siguió con la consulta siguiente sin intervención.
3. **El fallo nuevo no es memoria: es disco.** A las 11:27 y 11:31 UTC, durante Q11, el kubelet
   **desalojó a los dos ejecutores por `ephemeral-storage`**.
   - El pool `pesado` arranca con 100 GB de `pd-balanced` y sin SSD local.
   - El shuffle de un *join* de 600 M filas lo llena.
   - Es la misma trampa que tumbó el primer intento de C0, que se quedó sin disco con 273 GB.
   - **Los ejecutores de producción tienen el mismo hueco**, porque la plantilla del SDK no pide
     `ephemeral-storage`. Es la defensa de disco que falta junto a las de memoria del Build elástico (CE·6).
4. **Q2 en SQL crudo sigue sin terminar.** Una geometría que sale de una subconsulta escalar se evalúa
   fila a fila. La difusión de C3·a la pone `over()` del SDK, y no hay regla del planificador que la
   ponga en el SQL. Es un hueco de `ReglasDeOre`, no del autoescalado.
5. **El segundo nodo gana en lo pesado** (Q5 −37 %, Q6 y Q8 pasan a terminar) y **no gana nada en lo
   ligero** (Q1, Q3, Q4 y Q9, ±10 %), que es el reparto que ya fijó CE·3: los Builds pequeños se quedan en
   un pod. Q7 es la excepción, el doble de lento, sin causa medida.

**C3·e · Q2 y el cierre de la vara (diseño, 2026-10-20, por aprobar).** El disco del ejecutor que
destapó C3·d no es del *join*: lo llena cualquier *shuffle*, y va en **CE·6**. Aquí queda lo del *join*.

*Q2 · la geometría de una subconsulta escalar.*
- **Lo que pasa:** en `ST_Intersects(t.pickup, (SELECT z.geom … LIMIT 1))`, Spark calcula una vez la
  subconsulta y la deja como valor. Pero Sedona no la reconoce como el lado de un *join*: con 600 M
  filas, en cada una deserializa el polígono del condado y lo cruza sin índice.
  - La forma con la pista de difusión (q2d de C0) planifica un `BroadcastIndexJoin`: 125 s en un nodo.
- **La regla** (`DifundirLaSubconsulta`, en `ReglasDeOre`, junto a las de C3·b y C3·b2):
  - **Cuándo:** un `Filter` cuyo conjunto de condiciones con `AND` tiene un predicado espacial del mapa
    `GIRO` (Contains, Within, Covers, CoveredBy, Intersects o DWithin).
    - Un lado del predicado usa solo columnas del hijo.
    - El otro contiene una `ScalarSubquery` **no correlacionada** y con `maxRows ≤ 1`, como el
      `LIMIT 1` de Q2.
  - **Qué hace:** pasa el predicado a un `Join` interno con el plan de la subconsulta, marcado como
    difundido, y proyecta de vuelta las columnas del hijo. El resto de condiciones se quedan en el
    `Filter`.
  - **Por qué no cambia el resultado:**
    - con 0 filas en la subconsulta, el predicado da `NULL` y el `Filter` lo descarta todo, igual que
      un `Join` interno contra nada;
    - con 1 fila, es el mismo cruce.
    - **No se toca** un predicado dentro de un `OR` o de un `NOT`, porque ahí el `NULL` cambia el
      sentido; ni una subconsulta correlacionada o sin cota de filas.
  - **Pruebas en `regla.py`:**
    - Q2 en forma de subconsulta planifica `BroadcastIndexJoin` y da la misma cuenta que q2d;
    - una subconsulta sin filas da 0;
    - en un `OR` o correlacionada, el plan no cambia.

*Lo que no es Q2 pero se le parece.* Q6 tarda 504 s en cruzar `trip` con las **19 zonas** que pasan el
filtro del rectángulo, y lo hace con un `RangeJoin`.
- Que el lado sea pequeño solo se sabe después del filtro, y Catalyst no lo estima.
- e0 mide cuánto se gana difundiéndolo a mano. **Si es mucho**, una decisión en tiempo de ejecución (el
  lado filtrado se cuenta antes de planificar el *join*) es una pieza aparte, por diseñar. No entra en
  C3·e.

*La vara.*
- **Q10–Q12** se repiten con el disco arreglado y **límite de 1 800 s**, para tener el tiempo real y no
  solo «> 600». El objetivo de C0·5 (< 10 min) era con 32 vCPU.
  - Con la cuota de hoy, `CPUS_ALL_REGIONS` es 32, y el pool `pesado` no pasa de 2 × 8. Si no llegan,
    se dice y se cuantifica cuánto haría falta, sin subir la cuota.
- **Q6 con DuckDB:**
  - un pod en el pool `pesado` baja `trip` y `zone` del bucket a la SSD, corre Q6 en DuckDB 1.5.6 (la
    imagen del puesto) e imprime las 19 filas **sin redondear**;
  - Spark imprime lo mismo (`MUESTRA` en crudo).
  - Si la diferencia es de representación (el promedio de un intervalo), la regla de la huella se
    documenta; si es de valores, es un fallo y se persigue.

*Los pasos* (los de C3·e y los de CE·6 van juntos: una corrida mide los dos, y un empuje los lleva).
- **e0 · previo en victor, sin código de producto** (la escala 100 sigue en el bucket):
  - un pool temporal `c3e-ssd` (n2-highmem-8 spot, SSD local, 0–2) con etiqueta propia, que no toca
    `pesado`;
  - Q10, Q11 y Q12 con el pico de disco por nodo (las estadísticas del kubelet cada 15 s), lo que da el
    tamaño del *request*;
  - Q2 como q2d y Q6 difundido a mano, para cuantificar la regla y la pieza aparte;
  - el cotejo de Q6 con DuckDB.
  - Después, el pool, el namespace y el permiso temporal se borran. Menos de 1 $.
- **e1 · en local:** C3·e, `DifundirLaSubconsulta` con `regla.py`; CE·6, `ephemeral-storage` en las
  plantillas (el SDK y `cola.rs`) y en la política, y `73-el-pool-pesado.sh` con la SSD; el ADR.
- **e2 · el empuje, con permiso:** binario, luego malla, luego recrear el pool `pesado` a 0 nodos (con
  permiso aparte: un Build pesado que llegue en ese minuto espera en Kueue).
- **e3 · la vara entera otra vez en victor**: C3 se cierra, y CE·6 con ella. CE se cierra con CE·5.

### CE · El Build elástico

**Su responsabilidad:** cómo corre un Build pesado (Heavyweight, Spark) en uno o varios nodos, **sea cual
sea el cómputo**: *join*, agregación, ordenación, ventana, UDF o escritura. ORE pone los límites (el suelo,
el techo, la cuota y las defensas) y Spark decide dentro según la carga. No sabe nada de geometría.
- **De dónde viene.** Nació en C3, como C3·c, porque el *join* a escala fue el primer cómputo que pidió más
  de un nodo. Los commits hasta el 2026-10-20 lo llaman CE·0–CE·4: son CE·0–CE·4.
- **Medido también sin *join*.** En C3·d, Q1, Q3, Q5, Q7 y Q9 (filtros, KNN por radio y agregación)
  corrieron elásticas. Q5, una agregación pura, bajó de 345 s a 216 s con el segundo nodo.
- **Lo que es de C3 y no de aquí:** la difusión (C3·a), el `LEFT JOIN` (C3·b), la rejilla sobre el lado
  grande (C3·b2) y la regla de Q2 (C3·e). Son reglas del *join*; el Build elástico las corre como corre
  cualquier otra.

| paso | qué | estado |
|---|---|---|
| CE·0 | el previo | medido en victor, 2026-10-19 |
| CE·1 | la imagen: la entrada del ejecutor y `decom.sh` | en vivo |
| CE·2 | el SDK: `sesion()` en modo Kubernetes | en vivo |
| CE·3 | `ore-serve`: el modo por las filas de entrada, el driver y los avisos, detrás de `--builds-elasticos` | en vivo, *flag* apagado |
| CE·4 | la malla: cola del driver, cohorte, cuota, cuentas, política y red | en vivo, convergida 2026-10-20 |
| CE·5 | el *flag* en las celdas y un Build real con el lago | **en vivo**, 2026-10-20 |
| CE·6 | el disco del ejecutor | **en vivo**, 2026-10-20 |

**CE CERRADO por ahora (2026-10-20).** El Build elástico está en producción en todas las celdas, y
probado de punta a punta con un Build lanzado desde la consola. Lo que queda abierto va con nombre en
la deuda, al final del apartado.

**El previo de C3 que lo originó (punto 3, 2026-10-19): Spark en varios nodos dentro del modelo de un
Build** (`multinodo.sh`). Un **Job indexado** de N pods, uno por nodo, con un Service sin IP para que se encuentren:
- el pod 0 lleva el *master* de Spark *standalone* y el *driver*;
- todos llevan un *worker*;
- `successPolicy` (`succeededIndexes: "0"`, GA en Kubernetes 1.35) da el Job por terminado cuando
  acaba el pod 0, y Kubernetes cierra los demás.

No hace falta instalar nada en el clúster: ni Spark Operator, ni JobSet, ni permisos para crear pods
desde el *driver*. La imagen ya trae `spark-class`. 200 M puntos:

| caso | 1 nodo | 2 nodos | aceleración |
|---|---|---|---|
| sesión con todos los ejecutores | 7,3 s | 7,2 s | — |
| 6c (× 20 k buffers), difundido | 116,3 s | **63,6 s** | **1,83×** |
| «municipios» (× 8 000 detallados), difundido | 134,7 s | **72,5 s** | **1,86×** |

Escala casi lineal. Los pares cambian unas décimas por mil entre corridas porque los puntos son
`rand()` y salen distintos según el reparto; no es un error.

**El diseño (2026-10-19).** El primer diseño, un Job de N pods con N
sacado del historial del Transform, se descartó por reactivo: un Build tenía que fallar o tardar para
que el siguiente llevara más nodos. Antes de rehacerlo se reunió lo que hace la industria y lo que dan
Spark, Kueue y GKE (`docs/investigacion/c3c-spark-elastico.md`). Este es el diseño que sale de ahí.

*La idea.* **ORE pone los límites y Spark decide dentro, según la carga**, igual que Databricks,
Dataproc Serverless o EMR Serverless. El Build arranca con un ejecutor. Si hay tareas esperando, el
driver pide más (*dynamic allocation*) y el autoescalador de GKE enciende los nodos. Un ejecutor ocioso
se devuelve y el nodo vacío se apaga. El mismo Build puede usar un nodo para leer y dos para el join.
**Los nodos dan velocidad, no evitan el OOM**: eso lo hacen las defensas de memoria del punto 4, que
valen igual con un nodo que con seis.

*Lo que no cambia.* Las sesiones interactivas y los Previews pesados siguen en un pod con `local[*]`.
El lago, el SDK, `over()`, C3·a y C3·b quedan como están.

*1 · Las piezas de un Build pesado*

| pieza | dónde | qué es |
|---|---|---|
| **el driver** | el pod del agente de hoy (`trabajo-<quien>-<hex>`), con menos recursos (2 CPU / 16 GiB) y **en un nodo normal** (DE2) | Spark en modo cliente contra `k8s://https://kubernetes.default.svc`; un Service sin IP para él; puertos fijos (7078 driver, 7079 bloques) |
| **los ejecutores** | pods que crea el driver, en el pool `pesado` (*spot*), uno por nodo: 7 cores / 40 g + *overhead* | imagen `puesto-spark:1` por digest, cuenta `ejecutor` sin token de Kubernetes, dueño el pod del driver (mueren con él) |
| **los límites** | `ore-serve` al rendir el Build | `minExecutors 1`, `initialExecutors 1`, `maxExecutors = min(nodos del pool, lo que deja la cuota de la celda)`; `executorIdleTimeout 60s`; `shuffleTracking` |

**«ORE decide los nodos», en concreto:** ORE fija el techo (pool y cuota) y el suelo (1). El número de
cada momento lo decide la carga: las tareas pendientes de Spark. Si se cumplen los 400 M o los 240 s
del diseño anterior, no es por regla escrita, sino porque la carga los pide.

*2 · Las decisiones* (DE1–DE4, las del Build elástico; no confundir con D1–D5 del ADR)

| | decisión | por qué | lo que se pierde |
|---|---|---|---|
| **DE1** | ~~los ejecutores, fuera de Kueue~~ → **en Kueue, medido en CE·0 (punto 7)**. Antes: Kueue admite el driver; a los ejecutores los limitan la `ResourceQuota` de la celda, `maxExecutors` y el máximo del pool | la vía de Kueue para pods sueltos es un apaño que su equipo cerró como «not planned», y `SparkApplication` rechaza el *dynamic allocation*. El pool de ejecutores lo usan solo ejecutores, así que Kueue no se equivoca sobre lo suyo | no hay reparto justo entre celdas: si dos coinciden, la que llegó primero tiene más nodos y la otra espera ejecutores (Spark sigue con los que tiene). El informe lo dice |
| **DE2** | **el driver fuera de *spot***, en un pool normal pequeño (`pesado-driver`: `n2-highmem-4`, 0–1 nodos, dos drivers caben) con su propio sabor de Kueue | perder un ejecutor se recomputa; perder el driver pierde el Build entero. Es lo que recomiendan todas | ~0,25 $/h mientras haya un Build. Cuota: 10 + 4 + 16 = 30 de 32 |
| **DE3** | **si *spot* se lleva un ejecutor, se recomputa** (sin `fallbackStorage` en esta pieza) | con 15 s de aviso no da tiempo a migrar un *shuffle*; `decommission` activado para lo que sí dé tiempo | un Build alcanzado por *spot* tarda más. Si el previo lo mide caro, `fallbackStorage` en GCS va después |
| **DE4** | **`spark.authenticate`** entre el driver y sus ejecutores | la política de red deja que los pods de Spark de una celda se hablen; sin esto, el Build de una persona podría pedir bloques al de otra en la misma celda | nada: es un secreto por aplicación que Spark genera y pasa a sus ejecutores |

*3 · La seguridad: el driver puede crear pods*

El código del cliente corre en el proceso del driver, así que puede usar su permiso. Tres capas:
1. **Una cuenta propia** del driver pesado (`driver-pesado`), con un `Role` en el namespace: `pods`,
   `services` y `configmaps` (crear, ver, borrar). Una sesión interactiva no la lleva nunca.
2. **Una ValidatingAdmissionPolicy** sobre lo que crea esa cuenta, y solo eso:
   - la imagen `puesto-spark` por digest y la cuenta `ejecutor`, sin token;
   - volúmenes solo `configMap` y `emptyDir`;
   - sin `hostNetwork`, sin privilegios ni *capabilities*, `runAsNonRoot`;
   - límite de memoria ≤ 48 GiB, dueño un Pod, la etiqueta `spark-role: executor` y la tolerancia del
     pool `pesado`.

   Un Service o un ConfigMap que cree también pasan por ella: solo los de Spark, con su etiqueta.
3. **Pod Security `restricted`** en el namespace, por si la política tuviera un hueco.

La política no puede comprobar que el dueño sea un driver de verdad. El peor caso es un pod de ejecutor
falso con nuestra imagen, sin token y dentro de la cuota de su celda: lo mismo que ya puede hacer un
Build.

*4 · Las defensas contra el OOM* (en `spark-defaults.conf` y `sesion()`, con un nodo o con seis):

| causa | defensa |
|---|---|
| un lado difundido grande | C3·a (≤ 150 k filas, ≤ 48 MiB), ya hecho |
| `collect()` o `toPandas()` grande en el driver | `spark.driver.maxResultSize` acorde al driver (4 g): un error claro, no un `OOMKilled` |
| sesgo en un join por igualdad | AQE `skewJoin`, activado por defecto |
| **sesgo en un join espacial** (AQE no lo ve) | `sedona.join.numpartition` según los ejecutores vivos (la guía de Sedona: de 2 a 8 veces las particiones), y KDB-tree. Lo fija el previo, punto 5 |
| la memoria de Python (UDF) | `spark.executor.pyspark.memory` con un tope, y *overhead* 0,4: lo que se pase falla con un error de Python |
| particiones grandes | `maxPartitionBytes` 128 MB; AQE junta las pequeñas |
| un Build sin defensa | el informe dice **qué tarea** se quedó sin memoria y en qué etapa, no solo «OOM» |

*5 · La red* (NetworkPolicy de la celda, fija en la malla, no una por Build):
- **driver:** recibe en 7078 y 7079 solo de pods `spark-role: executor` de su namespace, y sale además
  al API server;
- **ejecutores:** 7079 entre ellos y salida al driver; sin internet, solo Google por la vía privada, como
  el puesto.

*6 · El informe del Build.* Ejecutores (máximo, media y nodos), cuánto esperó cada uno, si el pool o la
cuota cortaron (`pidió 4, el pool dio 2`), ejecutores perdidos por *spot*, bytes derramados a disco, y
el OOM con su tarea si lo hubo. Lo lee el *listener* de Spark del driver; el SDK lo manda con el
resultado.

*7 · El previo (CE·0), antes de construir.* En victor, en un namespace de medida con la cuenta, el
`Role`, la política y la red de arriba copiados; el driver es un Job a mano. Se borra al acabar.

| # | qué se mide | qué decide |
|---|---|---|
| 1 | el escalado a mitad de un Build: carga → pod pendiente → nodo → ejecutor | `schedulerBacklogTimeout`, y si compensa |
| 2 | que la política niega un pod arbitrario creado desde el código y deja pasar el ejecutor | la política, cerrada |
| 3 | cuánto retiene el *shuffle tracking* un nodo que sobra | `shuffleTracking.timeout` |
| 4 | expulsar un nodo de ejecutores a mitad | DE3 (recomputar) o `fallbackStorage` |
| 5 | un join espacial sesgado con un ejecutor | `numpartition` y las defensas de Sedona |
| 6 | una UDF de Python que se pasa de memoria | `pyspark.memory` y el *overhead* |
| 7 | dos Builds de dos celdas a la vez | lo que dice el informe en DE1 |
| 8 | la imagen del ejecutor en un nodo nuevo | — |
| 9 | **un Build pequeño** elástico en frío frente al de hoy (un pod) | si el pequeño elástico se queda dentro de 30 s del de hoy, **todo Build es elástico**; si no, los pequeños siguen en un pod y se decide por el tamaño de las entradas |

Y lo que solo se ve con el lago de verdad, en CE·5:
- **las credenciales del lago en los ejecutores**: viajan en el `FileIO` de Iceberg. Si refrescarlas
  llama al catálogo desde un ejecutor, `IdentidadDelPuesto` lee el token del pod, que el ejecutor no
  tiene;
- que la imagen pip de Spark trae lo que pide un ejecutor de Kubernetes: `spark-kubernetes`, el
  `entrypoint.sh` y el `decom.sh` de la distribución. Si no, CE·1 los añade.

*8 · Los pasos*

| paso | qué | dónde |
|---|---|---|
| CE·0 | el previo de arriba | victor, namespace de medida |
| CE·1 | la imagen: la entrada de ejecutor y `decom.sh` | local |
| CE·2 | el SDK: `sesion()` en modo Kubernetes con los límites y las defensas; el *listener* del informe | local (el arnés de Spark) |
| CE·3 | `ore-serve`: el Build pesado con la cuenta del driver, su Service, la plantilla de ejecutor (ConfigMap), los límites y el informe. Detrás de `--builds-elasticos` (apagado: el Build de hoy) | local |
| CE·4 | la malla: el pool `pesado-driver` y su sabor, la cuenta, el `Role`, la política, la red y la cuota. **Binario, luego malla, luego *flag***, cada uno por separado | victor |
| CE·5 | Builds reales: uno pequeño y uno grande que escala, con el lago, y un nodo expulsado a mano | victor |

**Hecho cuando:** un Build grande escala solo y escribe lo mismo que con un nodo, más rápido; uno
pequeño no paga por ello (punto 9); ninguno de la vara acaba en `OOMKilled` sin un error que diga qué
tarea fue; y el informe dice cuántos nodos usó y qué lo limitó.

**CE·0 · el previo del Build elástico (medido en victor, 2026-10-19).** Laboratorio en
`pruebas-de-fuego/geo/c3/c0/`:
- `c0.sh` corre un driver a mano en el namespace de medida `c3c0`, con Pod Security `restricted`, las
  cuentas, el Role, la política y la red del diseño (`laboratorio.yaml`);
- el driver va en el pool `pesado-driver`, creado con `malla/74-el-pool-del-driver.sh`: n2-highmem-4
  normal, de 0 a 1 nodos;
- los ejecutores, en `pesado` (*spot*);
- `dos-celdas.sh` lanza dos celdas a la vez.

La imagen es `puesto-spark:1` sin cambios: trae `spark-kubernetes` 4.1.0, y la entrada del ejecutor y
`decom.sh` van en un ConfigMap (`ejecutor.sh`, `decom.sh`).

| # | qué | resultado |
|---|---|---|
| 1 | **escalar a mitad de un Build** (200 M × 20 k, máximo 2) | empieza con 1 ejecutor y **pide el 2.º él solo**: 44 s desde que lo pide hasta el nodo listo y el ejecutor dentro. **79,4 s**, frente a 116 s en un pod (previo) y 64 s con 2 nodos fijos |
| 2 | **la política** | **9/9**: niega otra imagen, la cuenta del driver, quitar el dueño, 100 Gi, salir del pool pesado y un ConfigMap cualquiera (422). Un `hostPath` lo para antes Pod Security (403), y un Secret, el Role (403). El ejecutor de Spark pasa. **Hallazgo:** sin la regla del pool, un «ejecutor» hecho a mano cayó en el nodo del sistema; la regla se añadió |
| 3 | **lo que retiene el *shuffle*** | con `shuffleTracking.timeout 120s` y ocioso 60 s, el segundo nodo se devuelve **152 s** después del join |
| 4 | **perder un ejecutor a mitad** (borrado con 15 s de gracia, como *spot*) | Spark lo repone en 18 s y el Build da **las mismas cuentas** (111 249 591 pares): 83,9 s. DE3 (recomputar) basta |
| 5 | **join espacial sesgado** (100 M puntos, 90 % en 200 m², 300 k zonas, `RangeJoin`, 1 ejecutor) | **con la rejilla de Sedona por defecto** (`spatitionside=left`, la de las zonas, que son uniformes): **un solo core** trabajando, más de 9 min y lejos de acabar. **Con la rejilla sobre los puntos** (`right`): los 7 cores, **687 s, 655 M pares, sin OOM** (22 GB). La referencia uniforme: 1121 s, 839 M pares. Por par, el sesgado con la rejilla buena (0,95 M/s) va como el uniforme (0,75 M/s). **El sesgo espacial no da OOM: da un solo core. Se arregla haciendo la rejilla sobre el lado grande** |
| 6 | **UDF de Python que pide 8 GB** con `pyspark.memory 4g` | **`MemoryError` de Python** en 19 s, sin pod muerto |
| 7 | **dos celdas a la vez** (dos Builds grandes, máximo 2 cada uno) | **ejecutores fuera de Kueue (DE1 tal cual): B pasa unos 100 s sin ningún ejecutor** hasta que A acaba. **Con los ejecutores en Kueue** (cola de la celda, cohorte, 1 ejecutor garantizado por celda, `reclaimWithinCohort`): Kueue echa el ejecutor prestado de A, B tiene el suyo en **32 s**, A sigue con 1 y B sube a 2 cuando A acaba. Los dos dan las cuentas bien |
| 8 | **la imagen en un nodo nuevo** | el ejecutor, **4 s** (*streaming*); el driver, 13 s |
| 9 | **un Build pequeño del todo en frío** (los dos pools a 0; 20 M × 20 k) | **~170 s**, frente a **115 s** del pod único de hoy (6c): +53 s, por encima de los 30 s del criterio. El camino es en serie: el nodo del driver (49 s), el driver arranca y pide el ejecutor (26 s), el nodo del ejecutor (37 s) y el join (51 s). **Los Builds pequeños siguen en un pod**, como hoy; el elástico, para los grandes |

Otros detalles:
- **La sesión espera 30 s** a que se registren los ejecutores (`minRegisteredResourcesRatio`). Se pone a
  0: las tareas ya esperan a su ejecutor.
- **Al cerrar, Spark intenta borrar Services y PVC** por etiqueta y el Role se lo niega. Es ruido en el
  log, sin efecto. No se le da ese permiso.

**Lo que cambia en el diseño:**
- **DE1 se invierte: los ejecutores van en Kueue**, como pods sueltos (la vía de Kueue para
  `spark-submit`: `queue-name`, `managed` y `pod-suspending-parent`). Cada celda tiene **un ejecutor
  garantizado** en su `ClusterQueue` y presta y recupera en la cohorte. El riesgo (Kueue no lo da por API
  estable) se cubre fijando la versión de Kueue y pasando `dos-celdas.sh` antes de subirla.
- **La rejilla de Sedona, sobre el lado grande** (`sedona.join.spatitionside`). En CE·2, el SDK la fija
  con las filas del catálogo, igual que C3·a.
- **Pequeño en un pod, grande elástico.** Lo decide `ore-serve` al encolar con las filas de las entradas,
  que da el catálogo (lo mismo que ya lee C3·a). El umbral, provisional en 100 M filas, se fija en CE·5 con
  la vara. No es reactivo: el modo sale del tamaño estimado antes de correr, nunca de un fallo.
- Se confirman DE2, DE3 y DE4, la política (más la regla del pool) y la red.
- El pool `pesado-driver` queda creado y a 0 nodos (`malla/74`): no cuesta nada mientras no haya Build.


**CE·1–CE·3 · hechos en local (2026-10-20).** Con lo que cambió CE·0: los ejecutores van en Kueue, lo
pequeño va en un pod y la rejilla se hace sobre el lado grande.

- **CE·1 · la imagen.** `puesto/spark/ejecutor.sh` (la rama `executor` del `entrypoint.sh` de Spark 4.1.0)
  y `decom.sh` (el `preStop`: SIGPWR a la JVM, que es el PID 1) se instalan en `/opt/ore/spark/`. Spark
  por pip ya trae `spark-kubernetes`.
- **CE·2 · el SDK** (`ore/spark.py`). `sesion()` sigue siendo local salvo con `ORE_SPARK_ELASTICO=<techo>`.
  En ese caso este pod es el driver: Spark va contra la API de Kubernetes con la cuenta del pod y lleva
  los puertos fijos de CE·0. La asignación es dinámica: mínimo 1, máximo el techo, ocioso 60 s,
  *shuffle tracking* 120 s y `minRegisteredResourcesRatio` a 0. Lleva además *decommission* y
  `spark.authenticate`. Cada ejecutor (34g + 4g + 4g de Python, 7 cores) va a `cola-pesada` como pod
  suelto de Kueue, con la plantilla que exige la política (cuenta `ejecutor` sin token, pool pesado,
  `/opt/ore/spark/ejecutor.sh`). Pruebas: `test_motor_pesado.py` (`ElBuildElastico`); la suite del SDK en la imagen
  (167/167) y el arnés de Spark (20/20) con la imagen derivada `ore-puesto-spark:c3c`.
- **CE·3 · `ore-serve`.**
  - Con `--builds-elasticos FILAS`, al abrir un Build pesado suma las filas de sus entradas (el
    `total-records` del snapshot vigente, por `ore datasets --cargar`). Por encima del umbral, el Build
    es elástico con techo `TECHO_DE_EJECUTORES` (2, el pool); por debajo, o si no se sabe, un pod.
  - Lo dice en los `avisos` del Build: `compute`, `inputRows`, `maxExecutors` y `reason`.
  - `cola::al_driver_elastico` hace driver el trabajo pesado ya rendido: 1,5 CPU / 12 GiB, la cuenta
    `driver-pesado`, `ore.dev/spark: driver` en el pod, y `ORE_SPARK_ELASTICO`, `ORE_SPARK_IMAGEN`, su
    nombre y su IP. Sigue en `cola-pesada`, y Kueue le da el sabor del driver.
  - Si la plantilla cambia sin enterarse, falla.
  - Sin el *flag*, nada cambia. Pruebas: el modo por el tamaño, las filas del snapshot vigente y el
    driver sobre la plantilla real de la malla; ore-serve 257/259 (fallan los 2 de git en Docker, por el
    worktree), clippy y fmt limpios.

**CE·4 · la malla, escrita en local y comprobada en seco (2026-10-20).**
- **`22-el-build-elastico.yaml`**, una plantilla nueva del inquilino que da de alta `gen-inquilino.py` y
  la kustomization:
  - las cuentas `driver-pesado` (la misma cuenta de Google que el puesto, más su enlace de Workload
    Identity en el aprovisionador ③) y `ejecutor` (sin token);
  - el Role (pods y ConfigMaps);
  - la ValidatingAdmissionPolicy de la celda, la de CE·0 más estas reglas:
    - la imagen como `…/puesto-spark:1` o por digest;
    - sin root ni privilegios;
    - el ejecutor a `cola-pesada`, para que no se salte Kueue.
  - la red `spark-del-driver` y `spark-del-ejecutor`:
    - 7078/7079 entre los pods `ore.dev/spark`;
    - el driver al API server;
    - los ejecutores a Google por la vía privada.
  - la cola del driver, `cq-<celda>-driver` / `cola-driver`, con 1,5 CPU / 12 GiB en el sabor
    `pesado-driver`. `cola::al_driver_elastico` manda ahí al driver.
- **`10-kueue.yaml`**: el sabor `pesado-driver`, con el pool de `74-el-pool-del-driver.sh`, que ya
  existe en victor a 0 nodos.
- **`11-el-inquilino.yaml`**:
  - `cq-<celda>-pesado` entra en la cohorte `pesado` con `reclaimWithinCohort: Any`, que es lo medido en
    CE·0;
  - la cuota pasa a 20 CPU / 112 GiB.
- **Pod Security `restricted` se queda fuera.** La política ya exige al ejecutor lo que exigiría
  `restricted`, y antes de ponerla en la celda hay que comprobar los puestos de hoy.
- **Comprobado en seco:**
  - `gen-inquilino.py --comprobar` y `--comprobar-plantillas` dan verde;
  - victor rendido: `22` y el sabor pasan `kubectl apply --dry-run=server`, CEL de la política incluido;
  - `kubectl diff` de `11` en victor da exactamente la cohorte, el *reclaim* y la cuota.

**El orden del empuje**, como siempre: binario, luego malla, luego *flag*.
1. **El código** (C3·a, C3·b y CE·1–CE·3; CE·4 solo cambia la cola del driver en `cola.rs`). El *flag* está
   apagado, así que ningún Build se hace elástico. **Pero van en vivo para todo Build pesado** la
   difusión de C3·a, la regla del `LEFT JOIN` de C3·b y la rejilla sobre el lado grande.
2. **La malla**: el sabor, `22` y `11` en cada celda. Va con el aprovisionador (el enlace de Workload
   Identity) y la convergencia de los inquilinos.
3. **El *flag*** (`--builds-elasticos` en `40-ore-serve.yaml`, para victor) y CE·5.

**Lo que el previo no ha despejado, y entra en la primera pieza que lo toque:**
- **la identidad de los ejecutores con el lago de verdad**: en multinodo leen y escriben en GCS desde
  otros pods. El `AuthPod` del *driver* ya está; falta ver que las credenciales prestadas les llegan;
- ~~**Kueue con un Job de N pods en `cola-pesada`**~~: resuelto por DE1 invertida (ejecutores como pods
  sueltos en Kueue), medido en CE·0 y en C3·d;
- ~~**Q6, Q8, Q11 y Q12 a escala 100**~~: medidas en C3·d.

**CE·6 · el disco del ejecutor (diseño, 2026-10-20, por aprobar).** Lo destapó C3·d en Q11, pero vale
para cualquier *shuffle*: un `groupBy`, un `distinct`, una ordenación o una ventana sobre 600 M filas
llenan el disco igual que un *join*. Es la defensa de disco que faltaba junto a las de memoria del
diseño (punto 4). El tamaño del *request* sale del previo e0 de C3·e, que mide las dos cosas en la misma
corrida.

- **Lo que hay:**
  - el pool `pesado` arranca con 100 GB de `pd-balanced`, sin SSD local;
  - Spark en Kubernetes vuelca el shuffle a un `emptyDir` (`spark-local-dir-1`), que vive en ese disco;
  - la plantilla del SDK no pide `ephemeral-storage`.
  - Cuando el disco baja del umbral del kubelet (10 %), **desaloja a todo el nodo**: los dos ejecutores
    a la vez.
- **Lo que no vale:** agrandar el `pd-balanced`.
  - `SSD_TOTAL_GB` es de 500 GB en la región y el `pd-balanced` cuenta en ella (hoy 70 usados; con los
    dos pools llenos, 250).
  - `pd-standard` no cuenta, pero rinde ~60 MB/s a 500 GB: lento para un shuffle.
- **Lo que se propone:**
  1. **SSD local NVMe en el pool `pesado`**, `--ephemeral-storage-local-ssd count=1`: 375 GB por nodo.
     - Kubernetes pone ahí el almacenamiento efímero, con lo que el `emptyDir` del shuffle y el
       *spill* de DuckDB de un Build de un pod caen en la SSD.
     - `LOCAL_SSD_TOTAL_GB` no tiene límite y `PREEMPTIBLE_LOCAL_SSD_GB` es 0. Las spot gastan la cuota
       normal, como ya pasa con las CPU; e0 lo confirma creando el pool.
     - Cuesta unos céntimos la hora por nodo, solo mientras el nodo existe (el pool escala a 0).
     - Es varias veces más rápida que el `pd-balanced`, así que el shuffle también gana.
     - Un pool no cambia de disco en caliente: `73-el-pool-pesado.sh` lo **recrea** (borrar y crear)
       mientras está a 0 nodos, con la misma etiqueta y el mismo *taint*. Kueue y la malla no cambian.
  2. **El ejecutor pide disco**: `ephemeral-storage` con *request* y *limit* en la plantilla del SDK
     (`_plantilla_del_ejecutor`) y en la de C3·d.
     - El planificador lo cuenta.
     - Si un ejecutor se pasa, **el kubelet mata a ese pod, no al nodo**. Spark repite sus tareas en
       otro (la misma defensa que CE·0 midió al perder un ejecutor, punto 4).
     - El tamaño sale de e0: el pico medido en Q10/Q11 más un margen, por debajo de lo asignable del nodo.
     - El pod de un Build pesado de un nodo (`RECURSOS_DEL_PUESTO_PESADO`) pide lo mismo.
  3. **La política** de la celda (`22`) añade un tope de `ephemeral-storage` a lo que crea el driver,
     como ya hace con la memoria.

**CE·6 · hecho en local (2026-10-20), opción A** (lo mínimo para encender el *flag*: el disco, una
medida corta de Q11 y CE·5; Q2 y la vara entera quedan para después).
- **Medido antes de escribir:** un nodo `n2-highmem-8` *spot* con una SSD local se crea sin problemas de
  cuota y da **281 GiB de `ephemeral-storage` asignables** (de 368). Se creó en un pool temporal y se
  borró en 3 min.
- **250 Gi por pod pesado**, como *request* y como *limit*. Va un ejecutor por nodo, y con ese tope ni
  un ejecutor que se pase lleva el nodo al umbral de desalojo del kubelet (~37 GiB libres): cae ese
  pod, no el nodo.
  - **El ejecutor** (`spark.py`, `EJECUTOR_DISCO`): en su plantilla. Spark añade CPU y memoria a esos
    recursos, no los sustituye.
  - **El puesto pesado de un pod** (`cola.rs`, `RECURSOS_DEL_PUESTO_PESADO`): un Build o un Preview
    pesados derraman igual.
  - **El driver no** (`DEL_PESADO_AL_DRIVER` lo quita): no vuelca *shuffle* y su cola no cubre disco.
- **La cola pesada cubre el disco** (`11`, `ephemeral-storage` 250Gi por celda en el sabor `pesado`).
  Kueue, con su configuración por defecto, no admite un pod que pide un recurso que la cola no declara.
- **La política** (`22`) exige al ejecutor un límite de disco de como mucho 250Gi.
- **El pool** (`73`): SSD local (`--ephemeral-storage-local-ssd count=1`). `RECREAR=1` borra y crea el
  pool de antes, solo si está a 0 nodos.
- **Comprobado:** `gen-inquilino` en verde; `11` y `22` de victor pasan `dry-run=server` (el CEL nuevo
  incluido) y su diff es solo el disco; el SDK, 175/175 en `puesto-python` y `test_motor_pesado` 17/17
  en la imagen de Spark.
- **El orden del empuje, al revés que otras veces.** Aquí el binario es el que necesita lo nuevo:
  el puesto pesado pide 250Gi, que no existen sin la SSD ni se admiten sin la cola. Así que:
  1. el pool con la SSD (`RECREAR=1`, a 0 nodos);
  2. la malla (`11` y `22`). Es compatible hacia atrás: el *flag* está apagado, así que no hay
     ejecutores que la política pueda parar;
  3. el binario y las imágenes;
  4. una medida corta de Q11 en victor;
  5. el *flag* (CE·5).

**CE·6 · en vivo y medido (2026-10-20).** El pool `pesado` se recreó con la SSD a 0 nodos. Las dos
celdas convergieron con la cola y la política nuevas, y victor y demo quedaron desplegados en `f144e409`.
- **Un tropiezo del camino:** la política de `22` cita `puesto-spark@sha256:` como prefijo, y
  `simular-limpieza.py` lo leía como una imagen sin proteger.
  - `plano-de-control` estuvo en rojo desde el empuje de la malla de CE·4 (`d3365988`), y ningún commit
    de main se desplegó en ese tiempo.
  - Arreglado en `f144e409`: el comprobador solo cuenta como cita un digest completo.
- **Q11 sola a escala 100**, con 2 ejecutores y 1 800 s de límite:
  - **el disco ya no es el problema**: el pico fue de 27 GiB usados en el nodo (18 GiB el ejecutor), a
    los 265 s, y luego se quedó plano, con más de 320 GiB libres en cada nodo, sin desalojos;
  - el ejecutor lleva los 250Gi de la plantilla, y Spark le suma la CPU y la memoria;
  - **Q11 sigue sin terminar en 30 min**: tras el *shuffle* son 25 min de cálculo del doble *join*. Es
    velocidad del *join* (C3), no fiabilidad de CE.
  - En C3·d el disco se llenó porque las 12 consultas corrían en una sola sesión y el `shuffleTracking`
    conserva el *shuffle* de las anteriores. Un Build largo con muchas etapas puede acumular igual, y el
    tope de 250Gi es lo que lo contiene.
  - Los ejecutores tardaron más en frío que en C3·d (el 1.º a los 115 s, el 2.º a los 234 s), en un pool
    recién creado.

**CE·5 · el primer Build real (victor, 2026-10-20).** `--builds-elasticos` en `40-ore-serve.yaml`
(la plantilla es la misma para todas las celdas), con un umbral de **prueba** de 10 M filas, para que el
Build de 6c (20 M puntos × 20 000 zonas, `conteo.py`, lanzado desde la consola) fuera elástico.
- **El primer intento destapó un fallo** que solo salía con ore-serve en medio:
  - ore-serve reconocía como puesto solo un pod con la cuenta `puesto`;
  - el driver corre como `driver-pesado` y recibía 401 en bucle;
  - `d83b088c` acepta las dos cuentas, y lo demás no cambia (namespace, token ligado a un pod,
    audiencia que declara ore-serve). `ejecutor` sigue sin ser nadie.
- **El segundo, de punta a punta, 3 min 59 s en frío, 20 000 filas escritas:**

| hora | qué |
|---|---|
| +0 s | ore-serve crea el driver (avisos: elástico, filas de entrada, techo 2) |
| +23 s | el nodo `pesado-driver` |
| +57 s | el driver arranca y ore-serve lo acepta |
| +69 s | pide el 1.er ejecutor |
| +182 s | el ejecutor corre, en un nodo `pesado` encendido para él (**~100 s**) |
| +205 s | **pide el 2.º él solo, por la carga** |
| +273 s | termina antes de que el 2.º arranque, que se descarta |

- **El lago, desde los ejecutores:** el *join* lo hizo el ejecutor. Leyó las entradas y escribió la
  salida con las credenciales prestadas que viajan en el `FileIO`, y el driver hizo el commit. La
  incógnita del previo queda despejada para Builds de menos de ~55 min (ver la deuda).
- **El umbral definitivo, 100 M filas.** Con 20 M, el modo elástico en frío (4 min) tarda el doble que
  un pod (~2 min en C2). Confirma lo que midió CE·0: lo pequeño no paga el arranque.

**Dónde se van los 4 minutos, y por qué no es un problema del optimizador.** Unos 2,5 minutos son
encender VMs que estaban a 0 para no costar nada (el nodo del driver, ~25 s; el del ejecutor, ~100 s)
más arrancar el driver (~35 s). El cálculo son ~90 s (49 s en caliente en C2). La industria compra los
segundos con **capacidad caliente**, no con un optimizador:
- Databricks Serverless tiene VMs encendidas, compartidas entre sus clientes;
- EMR Serverless da segundos solo con capacidad pre-inicializada;
- Dataproc Serverless tarda ~1 min, y los clústeres clásicos 3–7 min.

Decidido (2026-10-20): **se hará algo como Databricks, pero no ahora**. Las opciones quedan en la deuda.
Para datos de hasta decenas de millones de filas, la respuesta en segundos ya existe: un repositorio
Lightweight.

**Lo que el Build elástico no hace todavía (deuda, con nombre):**
- **Decide el modo por las filas de entrada, no por el trabajo.** Un Build con pocas filas y cálculo
  pesado (un `explode` que multiplica filas, una UDF cara) se queda en un pod: falla limpio por sus
  defensas de memoria, pero no escala. Opción por diseñar: arrancar en un pod y pasar a elástico si
  Spark ve trabajo pendiente.
- **Solo Heavyweight.** Un repositorio Lightweight (DuckDB) es un pod: escala en vertical, no en nodos
  (D1 del ADR).
- **El informe del Build** (diseño, punto 6: ejecutores usados, qué los limitó, bytes derramados) no está
  escrito. Hoy los avisos dicen el modo, las filas y el techo.
- **El arranque en frío: 2–4 min por encender VMs.** Opciones, por decidir cuando toque:
  - **B · ventana caliente:** un nodo de driver y uno pesado encendidos en horario (08–00, como la
    plataforma); ~1 min por Build y unos 6 $/día; es configuración del autoescalado;
  - **C · Spark siempre vivo**, como un SQL warehouse: segundos, pero es un diseño nuevo (aislar a los
    inquilinos dentro de un Spark compartido es lo difícil).
- **Un Build de más de ~55 min:** el token de GCS prestado caduca. Iceberg intentaría refrescarlo desde
  el ejecutor, que no tiene identidad (sin token, por diseño) ni red hacia ore-serve. Sin probar; el
  primer Build largo lo dice.
- **El nodo `pesado` con SSD tardó ~100 s en encenderse**, frente a 37 s en CE·0 sin SSD. Por medir si
  es la SSD (su formateo) o fue esa vez.

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
- **2026-10-19 · C2 CERRADO.** Las piezas 4 (Spark Java) y 5 (el Preview, un Job corto), el cierre de
  C1 (los ejes y la geometría en WKB en hex) y la pieza 6: el empuje, el pool pesado con su *flavor*,
  su cola y su cuota, y la celda en victor. 20 M × 20 k en 115 s en frío y 49 s en caliente, unos
  0,03 $ por Build; SQL, Python y Java, el mismo conteo. Siguiente: C3, el *join* a escala.
- **2026-10-19 · C3, el previo.** Medido en victor: Spark difunde el lado pequeño solo por los bytes de
  sus ficheros (10 MB), así que hace falta una regla de ORE; el `LEFT JOIN` espacial cae en el producto
  cartesiano (> 300 s) y su reescritura tarda 16,4 s; un Build en varios nodos cabe en un Job indexado
  con Spark *standalone* y escala 1,83× con 2 nodos. El *approach*, C3·a a C3·d. Siguiente: el go.
- **2026-10-20 · C3·c4 en vivo y C3·d medido.** La malla convergida en demo y victor (cola del driver,
  cohorte `pesado`, cuota, cuentas con Workload Identity, política y red). SpatialBench a escala 100 con
  2 × 8 vCPU: 8 de 12 terminan (Q6 y Q8 por primera vez), huellas de C0 o DuckDB. Lo nuevo: el disco
  efímero desaloja a los ejecutores en Q11. Siguiente: C3·e (disco, regla de Q2, Q10–Q12).
- **2026-10-20 · CE CERRADO por ahora.** CE·6 en vivo (SSD local, 250Gi por pod pesado; Q11 sin
  desalojo, pico de disco 27 GiB). El flag encendido en todas las celdas. El primer Build real desde la
  consola destapó el 401 del driver (`d83b088c`) y luego fue de punta a punta: elástico, escala solo,
  el lago desde los ejecutores, 20 000 filas. Umbral definitivo, 100 M filas. El arranque en frío
  (2–4 min, VMs) queda como deuda con opciones; se hará algo como Databricks, más adelante. Siguiente:
  lo que queda de C3.
- **2026-10-20 · El ADR, reorganizado.** El Build elástico sale de C3 a su propio apartado, **CE**
  (antes C3·c0–c4, ahora CE·0–CE·4, y sus decisiones DE1–DE4). C3 se queda con el *join*: C3·a, C3·b, la
  rejilla (C3·b2, antes en c2), la vara (C3·d) y Q2 (C3·e). El disco del ejecutor pasa a CE·6, porque lo
  llena cualquier *shuffle*. Se anota la deuda de CE: el modo por filas y no por trabajo, solo
  Heavyweight y el informe del Build. Siguiente: e0, que mide C3·e y CE·6 en la misma corrida.
- Fuentes del estado del arte: [SedonaDB](https://sedona.apache.org/1.8.1/blog/2025/09/24/introducing-sedonadb-a-single-node-analytical-database-engine-with-geospatial-as-a-first-class-citizen/),
  [SedonaDB 0.3.0](https://sedona.apache.org/1.9.1/blog/2026/03/01/sedonadb-030-release/),
  [SpatialBench](https://wherobots.com/blog/sedonadb-spatialbench-spatial-query-engine-apache-sedona/),
  [Databricks ST](https://learn.microsoft.com/en-us/Azure/databricks/sql/language-manual/sql-ref-st-geospatial-functions),
  [Spark, tipos geo](https://spark.apache.org/docs/4.2.0/sql-ref-geospatial-types.html),
  [DuckDB, spatial joins](https://duckdb.org/2025/08/08/spatial-joins).
