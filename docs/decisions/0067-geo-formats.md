# 0067 · Geo formats — los fundamentos del formato geo

**Estado:** propuesto (2026-10-16); **plan G0–G3 esperando el go** (primero G0 + G1). Nace del bastión,
[`0066 · Geo paradigm`](0066-geo-paradigm.md), de donde sale todo lo que este ADR da por sabido: el
vertical (§1), los orígenes (§2), lo que hay en el código (§3), qué es un formato en ORE (§4), la
hipótesis de las cinco formas (§5), las decisiones P1–P8 y las medidas T1–T8 (§8). **Acotado al
formato**: su **representación**, su **distribución** (cómo viaja y cómo se organiza para escala) y su
**adopción** (que todo el ecosistema lo lea y que nosotros leamos el suyo). El cómputo, servir, el mapa,
lo geotemporal como aplicación, el cálculo ráster, la referencia de serie, las redes y la publicación de
estándares son líneas de expansión de 0066 (§7), con sus propios ADRs. Toca
[`0032`](0032-el-contrato-de-tipos.md), [`0033`](0033-el-dataset.md), [`0043`](0043-ore-arrow-stream.md),
[`0051`](0051-ore-null-contract.md), [`0061`](0061-origenes-de-objetos.md) y
[`0065`](0065-postgres-origen-de-verdad.md).

## Decisión (por confirmar con el plan)

1. **El tipo.** `Geometry<subtipo, CRS>` y `Geography<subtipo>` son tipos de primera clase en OOS y en
   el contrato de tipos de ORE. El CRS se guarda **fiel al origen**, y se sirve en WGS 84 (P3).
2. **La representación canónica del vectorial.** Parquet con **el tipo lógico GEOMETRY/GEOGRAPHY nativo
   y la clave GeoParquet `geo` a la vez**, con el CRS explícito en los dos (T3, T5), dentro de tablas
   Iceberg. Mientras `iceberg-rust` no traiga el tipo, la columna es `binary` con propiedades
   `ore.geo.<col>`; cuando lo traiga, pasa a `geometry` de v3 sin reescribir datos (T1).
3. **En el cable, GeoArrow** (`geoarrow.wkb` con el CRS en los metadatos de la extensión): de los
   orígenes al lago y del lago a los SDK. La geometría no pasa nunca por texto.
4. **Organizado para escala desde el principio** (P2): orden espacial al escribir, estadísticas
   espaciales por grupo de filas, columna H3 opcional y particionado según lo que mida la vara.
5. **La adopción se prueba, no se supone**: una matriz de lectores del ecosistema queda en verde en cada
   cambio (G2·5).
6. **Los códecs**: lo caliente en Rust y DuckDB nativos (GeoParquet, WKB/WKT, GeoJSON, FlatGeobuf) y la
   cola vectorial por el GDAL que ya trae `spatial` (Shapefile, GeoPackage, KML, la `.gdb` de Esri). El
   ráster y lo multidimensional, que piden un GDAL completo, van con su línea de expansión (0066 §7, E4).

## El plan

**Tres reglas:**
1. **Polivalencia (P1):** cada etapa fija una pieza común del formato, no una aplicación.
2. **Masivo y concurrente desde el principio (P2):** cada etapa cierra con un objetivo de volumen y de
   latencia medido sobre el banco, no sólo con que funcione.
3. **Lo de siempre:** primero el laboratorio, después producción; el binario antes que la malla; y el
   ADR al cerrar cada etapa.

| etapa | qué |
|---|---|
| **G0 · El banco geo** | datos de verdad y a escala, y la vara que mide el formato (T7: no tenemos ni una columna con geometría real) |
| **G1 · El tipo** | `Geometry` y `Geography` en OOS y en el contrato, GeoArrow, y la inducción |
| **G2 · El lago geo** | escribir y leer el vectorial canónico, organizado para escala, y la matriz de adopción |
| **G3 · Los códecs y los orígenes** | PostGIS, BigQuery, ficheros vectoriales y lat/lon llegan tipados |

Después de G3, **en este mismo ADR y con su propia medida**, la representación canónica (sólo como
formato) de las otras formas de 0066 §5.1: el esquema de las tablas de posiciones (objetos en
movimiento), COG + índice STAC (ráster), COPC (LiDAR) y Zarr (cubos).

### G0 · El banco geo

| paso | qué |
|---|---|
| G0·1 | **Un bucket de pruebas nuestro** (GCS) con datos abiertos a escala real. **Vectorial**: Overture (edificios y lugares de España, GeoParquet), límites administrativos, Catastro (INSPIRE). **Movimiento**: AIS abierto (danés o NOAA, cientos de millones de posiciones). **Ráster**: Sentinel-2 COG y un MDT del IGN. **LiDAR**: un PNOA en COPC. Cada uno con su licencia anotada |
| G0·2 | **Fuentes de prueba en victor**: un Postgres con PostGIS (Neon o Supabase lo traen), un dataset BigQuery con `GEOGRAPHY`, y ficheros Shapefile, GPKG, `.gdb`, KML, GeoJSON, CSV con lat/lon y direcciones en S3/GCS |
| G0·3 | **La vara geo**: un guion repetible (como la vara de Data Health) que mide en un puesto de victor lo que es del formato: escribir N filas geo, leer por zona (cuántos ficheros y grupos se descartan y cuánto tarda), leer con cada lector del ecosistema, el rendimiento de cada códec y la copia de cada origen. Fija **los objetivos de P2 para el formato**: volumen (p. ej. 1 000 M de puntos y 10 M de polígonos) y latencia (lectura por zona p95, con N lectores concurrentes). Se diseña para crecer: las líneas de expansión de 0066 (teselas, *joins*, trayectorias) le añadirán sus medidas |

**Hecho cuando:** los datos están en el bucket y en las fuentes, y la vara corre contra el estado de
hoy y deja la línea base.

### G1 · El tipo

| paso | qué |
|---|---|
| G1·1 | **OOS** (spec en `C:\oos` y *bump* del submódulo). `Geometry<subtipo, CRS>` (Point, LineString, Polygon, Multi\*, GeometryCollection; subtipo opcional) y `Geography<subtipo>` (esférica, siempre WGS 84) como tipos de columna de `Table` y de propiedad de entidad. CRS por código (`EPSG:25830`) o PROJJSON. `physicalType` cita el del origen |
| G1·2 | **El contrato en ORE**: `Fisico::Geometria { crs, subtipo }` en `ore-core` y su Arrow (`geoarrow.wkb` con CRS en los metadatos de la extensión). El contrato de nulos (0051) se extiende a «geometría vacía frente a nula» |
| G1·3 | **La inducción propone el tipo**: el tipo nativo del origen; pares lat/lon numéricos con rango válido (la regla de T7, sin falsos positivos por nombre: la medida decide); WKT/GeoJSON en texto; columnas de dirección marcadas como «geocodificables». Se propone, no se impone: queda en la propuesta para confirmar |

**Hecho cuando:** un árbol declara columnas geo, compila, y la inducción propone bien sobre el banco
(precisión y exhaustividad medidas).

### G2 · El lago geo

| paso | qué |
|---|---|
| G2·1 | **El escritor**: los ficheros de datos los escribe ORE con `parquet` 58 (*feature* `geospatial`), **con el tipo lógico GEOMETRY y la clave GeoParquet `geo` a la vez, y el CRS explícito en los dos** (T3, T5). Se registran en Iceberg con la columna `binary` y propiedades de tabla `ore.geo.<col>` (tipo, CRS) hasta que `iceberg-rust` traiga el tipo. Entonces se migra a `geometry` de v3, sin reescribir datos |
| G2·2 | **El orden y las estadísticas**: ordenar por Hilbert (u opcionalmente por celda H3 + tiempo) al escribir, grupos de filas de tamaño medido, bbox por grupo (nativo), y una columna H3 opcional a la resolución que diga la vara. El particionado de las tablas grandes también lo decide la vara (P2) |
| G2·3 | **La lectura**: `spatial` y `h3` preinstaladas en las imágenes de DuckDB (Python, Node, JVM: unos 79 MB). Se mide el descarte de grupos por bbox en consultas por zona con Iceberg (la extensión `iceberg` de DuckDB) y con Parquet directo |
| G2·4 | **Leer tablas v3 ajenas con `geometry`** (Snowflake, S3 Tables): sin el tipo en `iceberg-rust`, hoy ni se abren (T1). O se contribuye el tipo, o se lee el esquema tolerando el tipo desconocido. Se decide midiendo |
| G2·5 | **La matriz de adopción**: lo que escribe ORE lo leen, sin conversión y con su CRS, DuckDB (núcleo y `spatial`), pyarrow, GeoPandas, Sedona/Spark y GDAL (`ogrinfo`), y se prueba la carga en BigQuery y Snowflake. Una prueba de fuego la deja fija |

**Hecho cuando:** la vara escribe y consulta el volumen objetivo con descarte espacial medido, y
GeoPandas, DuckDB y Sedona leen los ficheros sin conversión.

### G3 · Los códecs y los orígenes

| paso | qué |
|---|---|
| G3·1 | **PostGIS**: `geometry`/`geography` como WKB con su SRID (hoy llega como EWKB en texto, 0065 PG2), en Arrow como `geoarrow.wkb` |
| G3·2 | **BigQuery** `GEOGRAPHY` → WKB (hoy es `Opaque`) |
| G3·3 | **Ficheros vectoriales** en los almacenes de objetos: GeoParquet como formato de `Table`; GeoJSON, Shapefile (también en zip), GeoPackage, KML/KMZ, FlatGeobuf y la File Geodatabase de Esri por el GDAL que trae `spatial` (T2/T4). Catálogo con tipo de geometría, CRS, extensión y número de elementos. Mantenida para los que no son nativos de nube, virtual para GeoParquet y FlatGeobuf |
| G3·4 | **Lat/lon y direcciones**: la geometría se construye al copiar desde los pares lat/lon confirmados (G1·3). Las direcciones sólo se **marcan** como geocodificables: geocodificar es transversal (0066 §7) |
| G3·5 | **El resto de bases con geometría** según llegue cada origen de la lista (0066 §2) (Oracle Spatial, SQL Server, Snowflake, Databricks, MySQL), con el mismo contrato |

**Hecho cuando:** cada origen del banco llega al lago con su geometría tipada y su CRS, la vara de copia
cumple el volumen y la matriz de adopción (G2·5) sigue en verde.

## Lo que no se hace aquí

El cómputo (ST_\*, H3, *joins*, enriquecer), servir y el mapa, lo geotemporal como aplicación, el
cálculo ráster, la referencia de serie, las redes y el 3D, y la publicación OGC API/STAC. Son las líneas
de expansión E1–E7 de [`0066`](0066-geo-paradigm.md) §7, cada una con su ADR, sobre lo que fija este.

## Bitácora

- **2026-10-16 ·** Abierto, separado de 0066 cuando éste pasó a ser el bastión. El plan G0–G3 sale de
  allí, acotado al formato: la vara mide sólo lo del formato, se añade la matriz de adopción (G2·5), y
  las direcciones sólo se marcan (geocodificar es transversal).
- **2026-10-16 · El código de G0·1–G1·3 se rebobina:** se hizo sin un enfoque ni una hoja de ruta
  diseñados. Primero el diseño de G0–G3, después el código. El banco (G0·1) queda en el bucket, salvo
  Overture, que no se llegó a subir y sólo existe en local.
- **2026-10-16 · Iceberg sin tipo geo: B + D (decidido).**
  - **Por qué Snowflake y S3 Tables sí lo tienen:** la v3 de la especificación ya define `geometry` y
    `geography`. Snowflake tiene su propia implementación, y S3 Tables se apoya en la de Java, que es
    la de referencia. ORE usa `iceberg-rust`, que va por detrás en v3.
  - **Estado de `iceberg-rust`** (corrige T1): la última versión sigue siendo la 0.10.1, pero el tipo
    está en revisión en una serie de tres PR de wirybeaver:
    [2933](https://github.com/apache/iceberg-rust/pull/2933) (tipos y CRS, lista para revisión),
    [3341](https://github.com/apache/iceberg-rust/pull/3341) (lectura, borrador) y
    [3342](https://github.com/apache/iceberg-rust/pull/3342) (escritura con el tipo lógico Parquet,
    borrador).
    - Aún no escriben los límites espaciales en el manifiesto, ni la clave GeoParquet `geo`.
    - `main` ya está en Arrow 59.2: la próxima versión lleva a ORE de Arrow 58 a 59.
  - **B · escritor propio, como puente:** ORE escribe el Parquet definitivo (GEOMETRY nativo con CRS,
    `geoarrow.wkb` y la clave `geo`) y lo registra en Iceberg con la columna `binary` y propiedades
    `ore.geo.<col>`. Lo provisional es sólo el tipo de la columna en Iceberg.
  - **D · contribuir:** probar la serie de PR contra el banco y aportar lo que falte (los límites
    espaciales del manifiesto).
  - **Aparte:** preparar el salto a Arrow 59 sin que coincida con G2.
  - **Riesgo, se mide antes de comprometerse:** `binary` → `geometry` no es una promoción válida en
    Iceberg. Hay que medir si reescribir sólo los metadatos mantiene legibles los snapshots viejos y
    DuckDB y Spark lo aceptan. Si no, las tablas geo nacen en v3 cuando llegue el tipo, y las
    existentes se migran con una reescritura medida. Se corrige así el «sin reescribir datos» de la
    decisión 2.
