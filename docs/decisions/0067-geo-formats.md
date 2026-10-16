# 0067 · Geo formats — los fundamentos del formato geo

**Estado:** propuesto (2026-10-16); **plan G0–G3 diseñado por capas, esperando el go**. Nace del bastión,
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
   `ore.geo.<col>` y los ficheros los escribe ORE (B); cuando lo traiga, pasa a `geometry` de v3, de
   la forma que decida la medida de G2·0 (T1, bitácora: B + D).
3. **En el cable, GeoArrow** (`geoarrow.wkb` con el CRS en los metadatos de la extensión): de los
   orígenes al lago y del lago a los SDK. Por el protocolo en texto viaja como WKB en hexadecimal,
   sin pérdida; **nunca como WKT ni GeoJSON**, salvo GeoJSON al servir en la API (el contrato entre
   capas, en el plan).
4. **Organizado para escala desde el principio** (P2): orden espacial al escribir, estadísticas
   espaciales por grupo de filas, columna H3 opcional y particionado según lo que mida la vara.
5. **La adopción se prueba, no se supone**: una matriz de lectores del ecosistema queda en verde en cada
   cambio (G2·5).
6. **Los códecs**: lo caliente en Rust y DuckDB nativos (GeoParquet, WKB/WKT, GeoJSON, FlatGeobuf) y la
   cola vectorial por el GDAL que ya trae `spatial` (Shapefile, GeoPackage, KML, la `.gdb` de Esri). El
   ráster y lo multidimensional, que piden un GDAL completo, van con su línea de expansión (0066 §7, E4).

## El plan

**Por capas, en este orden: G0 → G1 → G2 → G3** (decidido el 2026-10-16). Cada capa se apoya en la de
abajo y no empieza hasta que la anterior cumple su «hecho cuando».

**Cuatro reglas:**
1. **Polivalencia (P1):** cada etapa fija una pieza común del formato, no una aplicación.
2. **Masivo y concurrente desde el principio (P2):** cada etapa cierra con un objetivo de volumen y de
   latencia medido por la vara (G0·3) sobre el banco, no sólo con que funcione.
3. **Ninguna capa inventa lo que decide la de arriba.** El riesgo de ir por capas lo enseñó el G1
   rebobinado: un tipo sin lago acaba con apaños (`Fisico = Texto`, `Binary` sin marca, `Opaque` en
   GraphQL) que son decisiones de diseño tomadas a ciegas. Por eso **el contrato entre capas se fija
   entero aquí, antes de G1** (abajo), y cada capa lo cumple hasta donde llega y deja escrito qué le
   falta y qué capa lo pone.
4. **Lo de siempre:** primero el laboratorio, después producción; el binario antes que la malla; el
   ADR al cerrar cada etapa; commits en la rama, empujar al final.

### El contrato entre capas (fijado antes de G1)

Lo que una geometría **es** en cada sitio por donde pasa. G1 lo implementa hasta la copia; G2 pone lo
que falta en el lago; G3 lo cumple en cada origen.

| sitio | representación | quién la pone |
|---|---|---|
| **OOS** (tipo de columna y de propiedad) | `Geometry<subtipo, CRS>` y `Geography<subtipo>`; por defecto `Any` y `OGC:CRS84` | G1·1 |
| **El valor** | WKB ISO en orden x/y (longitud, latitud en CRS84). Nula ≠ vacía: nula es SQL `NULL`; vacía es una geometría vacía válida (`POINT EMPTY` es el WKB con NaN) | G1·1 |
| **El protocolo del driver, en texto** (0008: una fila JSON por línea) | **WKB en hexadecimal**, sin pérdida y la forma en que PostGIS ya lo emite. El CRS no viaja en el valor: es del tipo de la columna, que da el catálogo. EWKB (con SRID dentro) se acepta y se comprueba contra el CRS de la columna. **Nunca WKT ni GeoJSON**: pierden precisión o el orden de ejes | G1·2 (el contrato), G3 (cada driver) |
| **El protocolo del driver, en Arrow** (0043) | `Binary` con la extensión `geoarrow.wkb` y el CRS en sus metadatos (`{"crs": …}`; `"edges":"spherical"` si es `Geography`) | G1·2, G3 |
| **El contrato de tipos** (`ore-core`, 0032) | `Forma::Geo { subtipo, crs, esferica }`, una forma propia (no un `Fisico`: no es un escalar que se analiza del texto como los demás, se decodifica de hex y se valida como WKB) | G1·2 |
| **La copia** (`ore-store`, la carga) | `RecordBatch` con `Binary` + `geoarrow.wkb` + CRS. Un valor que no es WKB válido sigue la regla de lo que no analiza (0032): la columna entera queda como texto y el informe lo dice | G1·3 |
| **El fichero de datos** | Parquet con el tipo lógico GEOMETRY/GEOGRAPHY, bbox por grupo de filas, la clave GeoParquet `geo` (1.1, con el CRS en PROJJSON y el bbox del fichero) y el CRS en los dos | G2·1 |
| **La tabla Iceberg** | Hoy columna `binary` con la propiedad de tabla `ore.geo.<col>` (JSON: tipo, subtipo, CRS, esférica). Con `iceberg-rust` con tipo (B + D, bitácora), `geometry`/`geography` de v3 | G2·1, G2·4 |
| **La lectura en DuckDB** (puesto, motor, SDK) | `GEOMETRY` de DuckDB, con `spatial` y `h3` en la imagen | G2·3 |
| **El SDK** | Python: un `GeoDataFrame` con su CRS (no bytes); Node y JVM: `geoarrow.wkb` en Arrow | G2·3 |
| **GraphQL y la API JSON** | Un objeto GeoJSON (RFC 7946) en WGS 84, reproyectado al servir (P3). Es la única salida en texto, y es para servir, no para mover datos | G1·2 (el tipo en el esquema), E2 (el servir completo) |

### G0 · El banco geo

**Objetivo:** datos de verdad y a escala, en los mismos sitios donde los tendría un cliente, y una
vara que mide el formato. Sin la vara, ninguna capa tiene contra qué decir que está hecha (T7: hoy no
tenemos ni una columna con geometría real).

| paso | qué | dónde | hecho cuando |
|---|---|---|---|
| **G0·1 · El bucket** | Datos abiertos a escala real, con su licencia en `LEEME.md`. Ya están AIS (7 días daneses), Sentinel-2 (una escena sobre Madrid), el MDT Copernicus GLO-30 y una muestra COPC. **Falta subir Overture** (edificios 34 M, carreteras 10,6 M, lugares 2,2 M, límites de España: está en local) y quedan pendientes Catastro (INSPIRE) y un PNOA-LiDAR | `gs://…-banco-geo` (europe-west1), lectura para las cuentas del driver y de medios de victor | todo listado en `LEEME.md` está en el bucket y se lee desde una celda de victor |
| **G0·2 · Las fuentes** | Lo que tendría un cliente, sacado del banco. **Ficheros** (en `ficheros/` del bucket, una fuente GCS): los municipios de la Comunidad de Madrid en GeoJSON, Shapefile en zip, GeoPackage, KML, FlatGeobuf, la `.gdb` de Esri y GeoParquet; los lugares de Madrid en CSV con lat/lon, CSV con dirección y CSV con WKT. **PostGIS** (una base `banco_geo` en Neon, que trae PostGIS): municipios en `geometry(MultiPolygon, 25830)` (como los da una IDE española), lugares en `geometry(Point, 4326)` y en `geography`, carreteras de Madrid en `geometry(LineString, 4326)`, con índice GiST. **BigQuery** (dataset `banco_geo`): lugares y municipios en `GEOGRAPHY`. **Iceberg v3 ajeno**: una tabla con `geometry` escrita por Spark con la biblioteca Java de Iceberg, en `lakehouse/` del bucket (lo que tendría quien viene de Snowflake o S3 Tables). Las cuatro, dadas de alta como fuentes de victor | victor; las credenciales en el cofre, como siempre | cada fuente cataloga sus tablas, y la vara las encuentra |
| **G0·3 · La vara geo** | Un guion repetible (como la de Data Health, `salud/vara.py`) que corre en un puesto de victor (medir en prod) y deja un informe con fecha. **Los casos**, cada uno con el paso del plan que lo pone en verde: **V1** declarar y compilar una columna geo (G1); **V2** la inducción sobre las fuentes del banco, con precisión y exhaustividad (G1·4); **V3** escribir N geometrías al lago, filas/s y MB (G2·1); **V4** leer por zona: grupos de filas y ficheros descartados, latencia p50/p95 con 1, 8 y 32 lectores concurrentes (G2·2); **V5** la matriz de adopción, quién lee el fichero sin conversión y con su CRS (G2·5); **V6** abrir la tabla v3 ajena (G2·4); **V7** la copia de cada origen del banco: filas/s, tipo y CRS a la llegada (G3). Se diseña para crecer: las líneas de expansión de 0066 le añaden sus casos | `pruebas-de-fuego/geo/vara.py` | corre contra el estado de hoy y deja la **línea base** (casi todo en rojo, y eso es lo esperado) |
| **G0·4 · Los objetivos de P2** | Los números que el resto del plan tiene que cumplir, **propuestos aquí y aprobados con el go**. Propuesta: **volumen**, los 34 M de edificios (polígonos) y 1 000 M de posiciones (AIS de varias semanas, o sintético hasta llegar); **escritura**, ≥ 1 M de geometrías/s por puesto (T3 midió 2 M de puntos en 0,87 s en local); **lectura por zona** (una ciudad sobre los 34 M de edificios), p95 < 1 s con 32 lectores concurrentes y ≥ 95 % de grupos de filas descartados; **copia**, PostGIS y BigQuery a ≥ 100 000 filas/s | el ADR | aprobados |

**Hecho cuando:** los datos están en el bucket y en las fuentes, la vara corre y deja la línea base, y
los objetivos de P2 están aprobados.

### G1 · El tipo

**Objetivo:** que una geometría sea un tipo de verdad en todo lo que no es el lago: la especificación,
el contrato, el protocolo, la copia hasta el lote de Arrow, la inducción y los esquemas que se sirven.
Cumple el contrato entre capas hasta la carga; lo que falta en el fichero lo pone G2.

| paso | qué | dónde | hecho cuando |
|---|---|---|---|
| **G1·1 · OOS** | `Geometry<subtipo, CRS>` y `Geography<subtipo>` como tipos de columna de `Table` y de propiedad de entidad, en una versión nueva (v1alpha32). **Lo que fija:** los subtipos (los siete de OGC y `Any`; con Z, M y ZM); el CRS por código `AUTH:CODE` (`EPSG:25830`, `OGC:CRS84`), con PROJJSON como forma larga; `Geography` siempre en WGS 84 y con aristas esféricas; WKB ISO x/y como valor; nula ≠ vacía; dentro de `list<…>` y `struct<…>`; el ensanchamiento (`Geometry<Point>` → `Geometry<Any>` es seguro; cambiar el CRS no); `physicalType` cita el del origen; un error propio para el tipo mal formado; y la tabla de correspondencias con PostGIS, BigQuery, Snowflake, SQL Server, Oracle, GeoParquet e Iceberg v3. Casos de conformidad válidos e inválidos | `C:\oos`, *bump* de `vendor/oos` | la spec y sus casos pasan en la suite de conformidad |
| **G1·2 · El contrato** | `Forma::Geo` en `ore-core` (no un `Fisico`, ver el contrato entre capas); el texto del protocolo (hex WKB, EWKB comprobado contra el CRS) y su Arrow (`geoarrow.wkb` + CRS); la validación de WKB (bien formado, subtipo que casa con el declarado); el contrato de nulos (0051) con la vacía; y el tipo en los esquemas que se sirven: un escalar `GeoJSON` en GraphQL y el SDK generado que lo declara | `ore-core` (`types.rs`, `tipos.rs`), `ore-driver`, `ore-graph`, el generador del SDK | pruebas unitarias de ida y vuelta: hex → WKB → Arrow → hex, con nula, vacía, Z/M y un WKB roto |
| **G1·3 · La copia hasta el lote** | La carga decodifica el hex, valida y construye el `Binary` con la extensión y el CRS. Un valor roto sigue la regla de lo que no analiza: la columna queda texto y el informe lo dice. **Hasta G2·1 el escritor de `iceberg` pierde la marca** (T1): los bytes ya son los definitivos, la marca la pone G2. Eso se dice en el informe de la copia, no se oculta | `ore-store` (`carga.rs`) | una copia de una fuente de prueba con geometría produce WKB válido en el lago, y la vara V1 está en verde |
| **G1·4 · La inducción** | Propone, no impone. Cuatro reglas, cada una medida sobre el banco y sobre las 18 fuentes reales de T7: (a) **el tipo nativo del origen** (PostGIS, `GEOGRAPHY`, GeoParquet) da el tipo directamente, sin pregunta; (b) **un par lat/lon**: dos columnas numéricas no enteras cuyos nombres llevan `lat…`/`lon…` con el mismo resto **y** cuyos valores caen en rango (−90..90, −180..180) en una muestra; (c) **texto con WKT o GeoJSON** en una muestra; (d) **direcciones** (calle, ciudad, código postal, país), que sólo se marcan como geocodificables. Lo que no es nativo queda como pregunta en la propuesta; la respuesta se guarda y la copia la aplica en G3·4 | `ore-cli` (`inductor.rs`, `revision.rs`) | **V2**: precisión ≥ 0,95 y exhaustividad ≥ 0,9 en el banco, y ni un falso positivo en las 18 fuentes reales (`longitud_focal_mm`) |

**Hecho cuando:** un árbol declara columnas geo y compila; la copia de una fuente con geometría deja
WKB válido en el lago; la inducción cumple V2.

### G2 · El lago geo

**Objetivo:** que el fichero de datos sea el canónico (el contrato entre capas, fila «el fichero»),
organizado para escala, leído por nuestro motor con cómputo espacial y por todo el ecosistema sin
conversión. Es donde vive B + D.

| paso | qué | dónde | hecho cuando |
|---|---|---|---|
| **G2·0 · El paso a v3, medido antes de escribir** | El riesgo de B (bitácora): `binary` → `geometry` no es una promoción válida en Iceberg. Se mide si reescribir sólo los metadatos deja legibles los snapshots viejos y si DuckDB (`iceberg`) y Spark (Java) lo aceptan. **El resultado decide** si las tablas geo nacen en v2 con `binary` y se promueven, o si nacen ya preparadas para una reescritura única. También se mide qué versión de la tabla leen DuckDB y Spark | laboratorio | la decisión, escrita en el ADR |
| **G2·1 · El escritor** | Para las columnas geo, ORE escribe los ficheros de datos con `parquet` 58 (*feature* `geospatial`) en vez de dejar que `iceberg` derive el esquema: tipo lógico GEOMETRY/GEOGRAPHY, bbox por grupo de filas, clave GeoParquet `geo` 1.1 y el CRS en los dos. Los registra en Iceberg (la columna `binary` y la propiedad `ore.geo.<col>`) con las métricas que Iceberg pide del fichero. Las tablas sin geo no cambian de camino | `ore-store` (`lago.rs`) | **V3** cumple el objetivo de escritura, y el fichero lleva las dos marcas |
| **G2·2 · El orden y las estadísticas** | Ordenar por la curva de Hilbert sobre el centro del bbox al escribir (opcionalmente por celda H3 + tiempo para lo que se mueve); el tamaño de grupo de filas que mida la vara; una columna H3 opcional (a la resolución que diga la vara); y el particionado de las tablas grandes. **Todo lo decide V4**, con varias combinaciones medidas sobre los 34 M de edificios y las posiciones | `ore-store`, declarable en el `Dataset` | **V4** cumple el objetivo de lectura por zona |
| **G2·3 · La lectura** | `spatial` y `h3` preinstaladas en las imágenes de DuckDB (Python, Node, JVM; unos 79 MB); el SDK de Python entrega un `GeoDataFrame` con su CRS (con `geopandas` en la imagen, +141 MB, o `geoarrow-pyarrow`, +9 MB: se decide midiendo); Node y JVM, `geoarrow.wkb` | `Dockerfile` de las imágenes, el SDK | una consulta `ST_*` y una H3 corren en un puesto sobre el lago geo, y el SDK devuelve un `GeoDataFrame` con su CRS |
| **G2·4 · Las tablas v3 ajenas, y D** | Abrir la tabla v3 de G0·2: o con `iceberg-rust` con la serie de PR (2933, 3341, 3342) cuando salga, o tolerando el tipo desconocido mientras tanto (el esquema se lee, la columna llega como `geoarrow.wkb`). **D:** probar la serie de PR contra el banco, comentarlo upstream, y aportar los límites espaciales del manifiesto | `ore-store`; `apache/iceberg-rust` | **V6** en verde, y la contribución abierta |
| **G2·5 · La matriz de adopción** | Lo que escribe ORE lo leen, sin conversión y con su CRS: DuckDB (núcleo y `spatial`), pyarrow, GeoPandas, Sedona/Spark y GDAL (`ogrinfo`); y se carga en BigQuery y en Snowflake. Una prueba de fuego la deja fija y corre en cada cambio del escritor | `pruebas-de-fuego/geo/` | **V5** en verde para todos |

**Hecho cuando:** la vara escribe y consulta el volumen objetivo con descarte espacial medido (V3, V4),
la matriz de adopción está en verde (V5) y la tabla v3 ajena se abre (V6).

**Fuera de G2, pero sin coincidir con él:** el salto de Arrow 58 a 59 que traerá la próxima versión de
`iceberg` (su `main` ya está en 59.2). Se hace en su propio paso, antes o después de G2, nunca dentro.

### G3 · Los códecs y los orígenes

**Objetivo:** que cada origen del banco llegue al lago con su geometría tipada y su CRS, cumpliendo el
contrato entre capas en el driver (hex WKB en texto o `geoarrow.wkb` en Arrow).

| paso | qué | dónde | hecho cuando |
|---|---|---|---|
| **G3·1 · PostGIS** | `geometry(subtipo, SRID)` y `geography` → `Geometry<subtipo, EPSG:SRID>` / `Geography<subtipo>`, leídos de `geometry_columns` y `geography_columns` en el catálogo. Los valores salen como hex WKB (`ST_AsBinary`, o el EWKB nativo comprobado). Una columna `geometry` sin restricción de SRID ni de tipo es `Geometry<Any>` con el CRS del primer valor, comprobado en todos | `ore-read-postgres` | **V7** PostGIS: tipo y CRS correctos en las cuatro tablas, y el objetivo de copia |
| **G3·2 · BigQuery** | `GEOGRAPHY` → `Geography<Any>`, con `ST_ASBINARY` en la consulta del driver (hoy es `Opaque`) | `ore-read-bigquery` | **V7** BigQuery |
| **G3·3 · Ficheros vectoriales** | En los almacenes de objetos (0061). **GeoParquet** como formato de `Table`, nativo y virtual. **FlatGeobuf** nativo (Rust, `flatgeobuf`). **GeoJSON, Shapefile (también en zip), GeoPackage, KML/KMZ y la `.gdb` de Esri** por el GDAL que trae `spatial` de DuckDB, como colección mantenida. El catálogo da tipo de geometría, CRS, extensión y número de elementos | `ore-read-objetos` y el puesto | **V7** ficheros: los siete formatos llegan tipados y con su CRS |
| **G3·4 · Lat/lon, WKT y direcciones** | La copia construye `Geometry<Point, OGC:CRS84>` desde un par lat/lon confirmado en la inducción (G1·4), y la geometría desde una columna WKT/GeoJSON confirmada. Las direcciones sólo quedan marcadas: geocodificar es transversal (0066 §7) | `ore-store` (la carga) | **V7** CSV: los tres CSV del banco llegan con el punto (o la marca) |
| **G3·5 · El resto de bases** | Oracle Spatial, SQL Server, Snowflake, Databricks y MySQL, con el mismo contrato, según llegue cada origen de la lista de 0066 §2 | cada driver | su caso en V7 |

**Hecho cuando:** cada origen del banco llega al lago con su geometría tipada y su CRS, V7 cumple el
objetivo de copia, y la matriz de adopción (V5) sigue en verde.

### Después de G3

En este mismo ADR y con su propia medida, la representación canónica (sólo como formato) de las otras
formas de 0066 §5.1: el esquema de las tablas de posiciones (objetos en movimiento, con el AIS del
banco), COG + índice STAC (ráster), COPC (LiDAR) y Zarr (cubos).

### El código rebobinado

Los commits `316a4642` (G1·1–G1·2) y `8184f801` (G1·3) de la rama, y `ee9d51e` en `C:\oos`, se hicieron
antes de este diseño. Con el go se revierten y G1 se rehace desde aquí. Lo que se aprendió en ellos
está en el contrato entre capas: los valores por defecto del tipo, nula ≠ vacía, la regla del par
lat/lon y por qué el físico no era `Texto`.

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
- **2026-10-16 · El plan, diseñado por capas** (G0 → G1 → G2 → G3, decisión del usuario frente a la
  rebanada vertical). Para que ir por capas no repita los apaños del G1 rebobinado, **el contrato entre
  capas se fija entero antes de G1**: lo que una geometría es en la spec, el valor, el protocolo (hex WKB
  en texto, `geoarrow.wkb` en Arrow), el contrato de tipos (`Forma::Geo`, no un `Fisico`), la copia, el
  fichero, la tabla Iceberg, la lectura, el SDK y GraphQL (GeoJSON). Cada etapa con objetivo, pasos con
  su sitio en el código y su «hecho cuando» en un caso de la vara (V1–V7). Añadidos frente al plan
  anterior: G0·4 (los objetivos de P2, propuestos para aprobar), la tabla v3 ajena en el banco, G2·0 (el
  paso a v3 medido antes de escribir) y D dentro de G2·4.
