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

#### G2·3 · La lectura, en detalle (diseñado el 2026-10-17)

**Objetivo:** que quien trabaja en un puesto (notebook, transform, función) y quien usa los SDK de Python,
Node y JVM **lea, calcule y escriba** geometrías del lago como geometrías, con su CRS, sin traer nada a mano.

**Lo que hay hoy (medido o leído en el código):**
- **Los SDK no leen de ore-serve.** Piden un puntero (`GET /puestos/<id>/datos/<vista>`: `metadata_location`,
  credencial, o la consulta de una `View`) y leen el lago con DuckDB `iceberg_scan` (Python
  `puesto/python/ore/__init__.py` `_fuente_de`/`_iceberg`; Node `index.mjs` `fuenteDe`; JVM `Ore.java`).
  Como la columna es `binary` en Iceberg (G2·0), **DuckDB la entrega como BLOB**: nadie sabe que es una
  geometría ni en qué CRS.
- **`write()` no pasa por ore-serve:** el SDK manda Arrow IPC al binario `ore-store` dentro del puesto, y
  `carga::normalizar` **rechaza** la columna binaria (el brazo de «lo que el contrato 0032 no tiene»).
- **Las imágenes** (`puesto-python`, `puesto-node`, `puesto-jvm`) preinstalan `iceberg`, `avro`, `httpfs`,
  `json` e `icu` (Dockerfile 433, 531 y `Extensiones.java:19`), no `spatial` ni `h3`, y el puesto **no
  tiene salida a internet** para bajarlas (medido en G2·2).
- **La consulta por zona necesita `&&`** (G2·2): `ST_Intersects` solo no descarta grupos.

**Las piezas:**

| paso | qué | dónde | hecho cuando |
|---|---|---|---|
| **G2·3·0 · Medir el paquete de Python** | Qué hace falta para un `GeoDataFrame` con su CRS desde el Arrow de DuckDB: `geopandas` + `shapely` + `pyproj` (T5: +141 MB) frente a `shapely` solo, o `geoarrow-pyarrow` (+9 MB) sin `GeoDataFrame`. Se mide sobre la imagen `puesto-python` de verdad: MB añadidos, tiempo de 1 M de filas a `GeoDataFrame`, y que el CRS llegue. **Propuesta, a confirmar con el número:** `geopandas` en la imagen, porque `GeoDataFrame` es lo que el ecosistema espera (T5) | local, imagen del puesto | la decisión en el ADR |
| **G2·3·1 · El puntero dice qué es geo** | La respuesta de `datos` lleva `geo: {columna: {type, crs, edges}}` para cada columna geo de lo que se lee, sacado de los **tipos del árbol** (el `Dataset`, la `Table`, las columnas de la `View`), que son la fuente de verdad, y no de las propiedades `ore.geo.<col>` de la tabla (esas son para quien lea la tabla sin el árbol). También para los datasets que forman una `View` | `ore-serve` (`puestos.rs`, la respuesta de `datos`) | la respuesta de `datos` de una tabla geo de prueba trae su `geo` |
| **G2·3·2 · El SDK de Python lee geometría** | Con `geo` en el puntero, la fuente envuelve cada columna geo: `ST_GeomFromWKB(col)` (con su CRS, `GEOMETRY('EPSG:25830')` en DuckDB 1.5), así que **dentro del puesto la columna es `GEOMETRY`** y las `ST_*` funcionan sin conversión. A la salida: **Arrow** con `geoarrow.wkb` y su CRS; **pandas** (lo de siempre) da un **`GeoDataFrame`** con su CRS cuando la tabla tiene geometría; **polars**, la columna WKB con su marca. `sql()` igual. Lo mismo para una `View` (sus datasets entran como vistas de DuckDB, ya envueltas) | `puesto/python/ore` | prueba del SDK: una tabla geo leída por `over()` da un `GeoDataFrame` con `OGC:CRS84` o `EPSG:25830`, y `ST_Area` corre en `sql()` |
| **G2·3·3 · El SDK de Python escribe geometría** | `write()` de un `GeoDataFrame` o de un Arrow con `geoarrow.wkb`: `normalizar` acepta una columna **binaria con la marca `geoarrow.wkb`** (la sin marca se sigue rechazando), valida cada WKB contra el tipo (`geo::comprobar`) y conserva la marca, así que el lago toma el camino del escritor geo (G2·1, G2·2). El CRS que da GeoPandas es PROJJSON: se lleva a `AUTORIDAD:CÓDIGO` por su `id` (y si no lo trae, se niega con su porqué). El tipo de la columna que se declara en el `Iceberg` del `write()` (`_tipo_iceberg`) es `binary` | `carga::normalizar`, `puesto/python/ore` (`write`) | un `GeoDataFrame` escrito y vuelto a leer da los mismos WKB, el mismo CRS, y el fichero tiene GEOMETRY + clave `geo` |
| **G2·3·4 · Node y JVM** | La misma envoltura al leer (`fuenteDe`, `Ore.java`): Arrow con `geoarrow.wkb` y su CRS (no hay un `GeoDataFrame` en esos lenguajes; el valor es WKB, y GeoArrow lo entienden sus bibliotecas). Al escribir, lo mismo que G2·3·3 por el mismo `ore-store` | `puesto/node/ore`, `puesto/jvm/ore` | sus pruebas de lectura y escritura con una columna geo |
| **G2·3·5 · Las imágenes** | `spatial` y `h3` (de la comunidad) preinstaladas en las tres imágenes, junto a las cinco de hoy, y cargadas de serie en las conexiones del SDK que tocan una tabla geo. Lo que decida G2·3·0 en `provisto.txt` (Python). Se apunta cuánto crece cada imagen | `Dockerfile` (433, 531), `Extensiones.java`, `provisto.txt` | las imágenes construyen y su tamaño queda en el ADR |
| **G2·3·6 · La firma de una función** | `ore.tipos.Geometry` en las firmas de Functions y Transforms (`Geometry["Point", "EPSG:25830"]`, como `Media[…]`): lo que el documento declara como `Geometry<…>` (lo aplazado de G1·2) | `ore-code` (`firma.rs`, `python/derivar.rs`, el oráculo) | una función con un parámetro `Geometry` deriva su tipo de OOS |
| **G2·3·7 · La consulta por zona, bien hecha** | Un ayudante en los SDK: `ore.geo.en_zona(columna, xmin, ymin, xmax, ymax)` que devuelve `col && caja AND ST_Intersects(col, caja)`, y la guía del SDK que explica por qué (G2·2: sin `&&`, el fichero entero) | los tres SDK, su guía | aparece en la guía y en una prueba |

**La medida en victor (V5 parcial, y lo que dice «hecho»):** un guion de puesto, en la imagen nueva, que
sobre una tabla geo del lago (la copia de `banco_geo_pg.municipios` y los 34 M de edificios escritos por
ORE) hace: `over()` → `GeoDataFrame` con su CRS; una consulta `ST_*` y una H3 con `en_zona`; y `write()`
de vuelta, leído otra vez igual. Con tiempos para 1 M de filas.

**El orden:** primero lo local, con commits en la rama (G2·3·0, ·3, ·2, ·6, ·7, ·4; ·1 en ore-serve se
prueba en sus pruebas). Las imágenes (·5) y la medida en victor exigen **empujar**: es el primer paso de
0067 que no se puede cerrar sin desplegar, y se hace al final, con todo lo de G0–G2 a la vez (OOS antes que
ORE).

**Lo que no entra:** reproyectar al leer (el CRS llega como está; reproyectar es cómputo, E1, o servir
en WGS 84, E2), los teselados y el servir (E2), y el `Geography` esférico en DuckDB, que no tiene
geometría esférica: una `Geography` se lee como `GEOMETRY` con su marca `edges: spherical` en Arrow, y
el cómputo esférico es de E1.

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
- **2026-10-16 · G0, con el go del diseño.** Revertido el código anterior al diseño (`6350dbf2`,
  `00d67d1d`; OOS `65ff514`). Los objetivos de P2 de G0·4 quedan como se propusieron.
  - **G0·1 · El bucket, completo** (salvo Catastro y PNOA, que siguen pendientes). Overture se volvió a
    recortar, porque el volumen local se perdió al reiniciar Docker: edificios 34 104 964, carreteras
    10 581 570, lugares 2 180 990, límites 29 667 (`g0-overture.py`). Lectura del bucket para las cuentas
    del driver, de medios y del puesto de victor. `LEEME.md` al día.
  - **G0·2 · Las fuentes, cargadas; el alta, pendiente de una persona.**
    - **Ficheros** en `ficheros/` (`g0-ficheros.py`).
    - **PostGIS** en Neon, base `banco_geo` (`g0-postgis.sh`, en la celda): `municipios` en
      `geometry(MultiPolygon,25830)`, `lugares` en `geometry(Point,4326)`, `lugares_geog` en
      `geography`, y `carreteras` (300 000 tramos, por los 0,5 GB de Neon) en `geometry(Geometry,4326)`.
    - **BigQuery** `banco_geo`, con `GEOGRAPHY` (`g0-bigquery.py`).
    - **`POST /fuentes` desde el agente da 403:** dar de alta una fuente es gobierno. Las tres
      (`banco_geo_pg`, `banco_geo_bq`, `banco_geo_gcs`) las da de alta una persona. La vara no lo
      necesita: lee los catálogos con las URLs del cofre.
    - **La tabla Iceberg v3 ajena, pendiente:** pyiceberg 0.12 trae `GeometryType` pero **no escribe
      v3** (*«Writing V3 is not yet supported»*). Se escribirá con Spark + Iceberg Java.
  - **G0·3 · La vara** (`pruebas-de-fuego/geo/vara.py`): Jobs en victor, unos 90 s. **Línea base**
    (`vara-20261016-2130.md`):
    - **V1 · rojo.** `Geometry<MultiPolygon, EPSG:25830>` en una `Table` da `OOS3001` («no es un tipo de
      OOS»).
    - **V2 · 0/16.** PostGIS deja solo `physicalType: geometry(MultiPolygon,25830)`, sin `type`.
      BigQuery propone `Opaque`. El par lat/lon queda como `Float` y el WKT como `String`. La geometría
      del GeoParquet llega como `binary`. GeoJSON, GPKG, KML, FlatGeobuf, `.gdb` y el Shapefile en zip
      son **objetos de medios** (familia `geospatial` o `binary`), no tablas. Además, la inducción mete
      las tablas de sistema de PostGIS (`geometry_columns`, `spatial_ref_sys`) como si fueran del
      cliente.
    - **V3, V5, V6 · rojo, sin nada que medir todavía:** ORE no escribe geometría y falta la tabla v3.
    - **V4 · el techo de fuera** (DuckDB del puesto sobre los 34 M de edificios de Overture en el bucket,
      filtrando por `bbox`, 340 grupos de filas):
      - descarta entre el **87 y el 97 %** de los grupos según la ciudad;
      - con un lector, de **0,6 a 2,8 s**;
      - con concurrencia, p95 de **2,6 s con 1 lector, 3,6 s con 8 y 13 s con 32**.

      Objetivo: p95 < 1 s con 32 lectores y ≥ 95 % de descarte. El orden y el tamaño de grupo (G2·2)
      tienen recorrido.
    - **V7:**
      - **PostGIS** entrega **EWKB en hexadecimal** (con el SRID dentro), el CRS solo en
        `sourceType`, y unas 45 000 filas/s en Arrow.
      - **BigQuery** entrega **WKT** (`POINT(-3.71 40.60)`), con el aviso de que `GEOGRAPHY` «por REST
        viaja como texto», y 19 000 filas/s.
      - **GCS:** un GeoParquet no se lee: `geom` es `Binary` en el fichero y la tabla dice `String`.
        Los CSV van a unas 250 000 filas/s.
      - Todo, lejos del objetivo de copia (≥ 100 000 filas/s), salvo los CSV.
  - **Lo que falta para cerrar G0:** el alta de las tres fuentes (una persona), la tabla v3 ajena
    (Spark) y Catastro/PNOA.
- **2026-10-16 · La tabla Iceberg v3 ajena (G0·2), solo el esquema.** Ningún escritor publicado escribe
  hoy datos `geometry` en Iceberg v3:
  - **pyiceberg 0.12** no escribe v3;
  - **Spark 4.1 + Iceberg 1.11** no traduce el `GEOMETRY` de Spark (*«Not a supported type»*);
  - **la API Java de Iceberg 1.11** crea la tabla, pero su escritor Parquet dice *«Unsupported type for
    Parquet: geometry»*. Las PR que lo traen (`apache/iceberg` 17161, 17509) llegaron a `main` después
    de 1.11.

  Queda en el banco una tabla **v3 con `geom: geometry` y sin filas**
  (`g0-iceberg-v3.sh` + `.scala`, catálogo JDBC en `lakehouse/catalogo.db`). Sirve para V6 en lo que
  hoy rompe `iceberg-rust`, que es abrir el esquema. Los datos se le añaden cuando salga Iceberg 1.12.
  **Lo dice el propio ecosistema:** el tipo geo de Iceberg está recién llegando también en Java y en
  Python, no solo en Rust.
- **G0 cerrado, salvo el alta de las tres fuentes** (la hace una persona) y Catastro/PNOA, que pasan a
  deuda del banco.
- **2026-10-16 · G0 CERRADO.** Las tres fuentes están dadas de alta en victor y su catálogo se lee
  (`fuentes-del-banco.sh`):
  - `banco_geo_pg`: las cuatro tablas geo, más las de sistema de PostGIS;
  - `banco_geo_bq`: `lugares` y `municipios`;
  - `banco_geo_gcs`: 4 tablas y 6 conjuntos de objetos.

  Catastro y PNOA quedan como deuda del banco. Siguiente: G1.
- **2026-10-16 · G1·1 hecho: OOS v1alpha32** (`C:\oos` `3597dda`; `vendor/oos` en `15781970`; sin
  empujar). Sale del contrato entre capas:
  - **`00-scope`**: lo medido en la vara de G0 (0/16 tipadas, `OOS3001` al declarar). Queda fuera un CRS
    sin código de autoridad: PROJJSON es como se escribe en Parquet, no como se declara.
  - **`01-la-geometria`**:
    - `Geometry<subtipo, crs>` y `Geography<subtipo>`, con valores por defecto `Any` y `OGC:CRS84` y
      una forma canónica que los omite;
    - el valor en WKB ISO x/y; el SRID no va en el valor (un EWKB se acepta si casa con el CRS);
      nula ≠ vacía; el subtipo y las dimensiones se cumplen al escribir;
    - la correspondencia con Arrow, Parquet (las dos marcas), Iceberg, DuckDB, PostGIS, BigQuery,
      Snowflake, SQL Server, Oracle y GeoParquet; `physicalType` cita el del origen;
    - §4.1: mientras el formato de tabla no tiene el tipo, binario + contrato, y pasar al tipo cambia
      el esquema, no los bytes (sin prometer «sin reescribir», por el riesgo de G2·0);
    - §5: **en texto, hex WKB, nunca WKT ni GeoJSON**; en GraphQL, el escalar `GeoJSON` en WGS 84;
    - §6: el ensanche, dónde vale (columna, propiedad, `Struct`, `list`; **no como clave**) y los
      errores: `OOS3001` antes de v1alpha32, `OOS3008` para un tipo geo mal formado o una columna geo
      como clave.
  - **14 casos de conformidad**, 4 válidos y 10 inválidos, con el molde ligero de v1alpha31.
  - **`OOS3008`** en el registro de errores.
  - **Los casos los corre ORE en G1·2** (`borrador_de_v1alpha32` en la suite de conformidad): hasta
    entonces ningún marcador recorre `conformance/v1alpha32`. Al empujar, `C:\oos` va antes que ORE:
    el submódulo apunta a un commit de OOS que todavía no está en su remoto.
- **2026-10-16 · G1·2 hecho: el contrato en ORE.**
  - **Tipos** (`ore-core/types.rs`):
    - `Geometry { subtipo, crs }` y `Geography { subtipo }`, con su forma canónica;
    - `OOS3001` en un documento anterior a v1alpha32; `OOS3008` para un tipo geo mal formado, con el
      porqué (incluidas las dimensiones en `Any` y `Geometry<>`);
    - `geo_como_clave`: una columna geo en la `key` de una `Table`/`Dataset` o en la `primaryKey` de
      una entidad es `OOS3008`;
    - las columnas de `View` siguen sin comprobarse por tipo, como hasta ahora: añadirlas puede sacar
      diagnósticos en árboles que hoy compilan, y va aparte.
  - **La forma** (`tipos.rs`): `Forma::Geo { subtipo, crs, esferica }` y los metadatos de
    `geoarrow.wkb`, con el CRS siempre explícito y `edges: spherical` en una `Geography`. `Fisico` de
    una geometría es texto, como lo compuesto: es lo que viaja por el protocolo, y su valor lo lee
    `geo`.
  - **El valor** (`ore-core/geo.rs`, nuevo, sin dependencias):
    - el hex WKB de ida y vuelta (acepta el `\x` de un `bytea`, y da una pista si llega un WKT);
    - la lectura **entera** de WKB ISO y EWKB en cualquier orden de bytes, con recuentos comprobados
      contra lo que queda y la profundidad acotada;
    - la normalización a WKB ISO *little-endian* sin SRID (el mismo valor, los mismos bytes, venga de
      PostGIS o de un GeoParquet);
    - la vacía (`POINT EMPTY`, recuento 0) como valor, distinta de la nula;
    - el subtipo y las dimensiones contra la columna; el SRID contra el CRS (`0` no dice nada; `4326`
      casa con `OGC:CRS84`); el rango de una `Geography`; y el bbox.
  - **Arrow** (`ore-store/carga.rs`): `campo(nombre, forma)` da el `Field` con
    `ARROW:extension:name = geoarrow.wkb` y su CRS.
  - **GraphQL**: el escalar `GeoJSON` (RFC 7946, WGS 84), declarado en el SDL como todo escalar
    propio.
  - **El SDK de las firmas** (`ore.tipos` en Functions y Transforms) **pasa a G2·3**, con el
    `GeoDataFrame`. Es otra superficie (`ore-code/firma.rs`), y sin lectura geo en el puesto no tiene
    qué entregar.
  - **Pruebas:**
    - `ore-core`, 378 de 378, con 6 nuevas de WKB: el EWKB de PostGIS que midió la vara, big-endian y
      `Z`, vacía, lo compuesto, lo roto y el tipo de la columna;
    - `ore-store`, `una_geometria_va_y_vuelve_por_arrow`: hex → WKB → Arrow con la marca → hex, con
      nula, vacía, `Z` y un WKB roto;
    - **conformidad: `borrador_de_v1alpha32` 14/14**, el resto de la suite 31/31.
  - **Lo que no hace todavía:** la copia por texto sigue dejando una geometría como `string`. Llevarla
    por `geo::valor_de_texto` y `campo` es G1·3.
- **2026-10-16 · G1·3 hecho: la copia hasta el lote.**
  - **`carga::lote`**: una columna cuyo tipo es una geometría va por `columna_geo`. Cada valor pasa por
    `geo::valor_de_texto` (hex → WKB validado contra subtipo, dimensiones, SRID y rango → WKB ISO) y
    la columna sale como `Binary` con su `campo` marcado (`geoarrow.wkb` y el CRS).
  - **La regla de lo que no analiza**, la de siempre: un valor que no es una geometría de la columna
    (un WKT, un polígono en una columna de puntos) deja la columna entera como texto, y
    `sin_estrechar` dice cuántos, el primero y por qué.
  - **`carga::leer`** (lo que se lee del lago para fundir) devuelve una columna binaria como su hex
    ISO, así que fundir y volver a sellar da los mismos bytes.
  - **Hallazgo de la prueba de punta a punta:** lo que se escribe `Binary` vuelve de la tabla Iceberg
    como `LargeBinary` (así traduce `iceberg` su `binary`). Sin aceptarlo, fundir sobre una copia con
    geometría habría fallado en producción.
  - **Pruebas** (`ore-store` 64/64):
    - `la_copia_lleva_la_geometria_al_lote`: el EWKB de PostGIS, una nula, Parquet de ida y vuelta, y
      un WKT y un polígono que dejan la columna como texto;
    - `una_copia_con_geometria_llega_al_lago`: `sellar` en el lago en memoria da una tabla Iceberg
      con `geom: binary` y WKB ISO; fundir sobre ella conserva la de antes y añade una `POINT EMPTY`.
  - **V1 en verde, en local** con el `ore` de la rama (`vara-v1-local.sh`). Sobre el catálogo real
    de `banco_geo_pg`, la `Table` de municipios con `geom: Geometry<MultiPolygon, EPSG:25830>` en
    v1alpha32 da `ok · sin errores`; en v1alpha31, `OOS3001 … es un tipo de v1alpha32`. En victor se
    verá con la vara cuando se despliegue (al empujar). El V1 de `vara-celda.sh` ahora sube el
    documento a v1alpha32.
  - **Hasta G2·1** el escritor de `iceberg` pierde la marca `geoarrow.wkb` en el fichero (T1): los
    bytes ya son los definitivos, y las dos marcas de Parquet las pone G2·1.
- **2026-10-16 · G1·4 hecho: la inducción propone el tipo geo.**
  - **El tipo nativo, sin preguntar** (`geo_del_catalogo`, al empezar la inducción): `tipo_geo_nativo`
    lee el `physicalType` del origen.
    - `geometry(MultiPolygon,25830)` → `Geometry<MultiPolygon, EPSG:25830>`, con `PointZ`/`…M`/`…ZM`
      como dimensiones.
    - `geography(…,4326)` y el `GEOGRAPHY` de BigQuery → `Geography<…>`, aunque el driver dijera
      `Opaque` o nada.
    - Una `geometry` sin SRID no dice su CRS y no se adivina: queda la pregunta de tipo.

    La `Table` y la entidad con una columna geo salen en v1alpha32 (`lleva_geo`).
  - **Las tablas de sistema de PostGIS** (`spatial_ref_sys`, `geometry_columns`,
    `geography_columns`, `raster_*`) ya no se inducen como si fueran del cliente.
  - **Se pregunta, no se impone** (la clase `geo/` de la cola):
    - el **par lat/lon** (`geo/<tabla>`, `si`/`no`), por nombre con el mismo resto y solo entre
      columnas numéricas;
    - el **texto que dice geometría** (`geo/<tabla>.<col>`, `wkt`/`geojson`/`no`);
    - las **direcciones** (`geo/<tabla>:direccion`, `geocodificable`/`no`): una columna que lo diga
      entera (`direccion`, `address`, `ubicacion`…) o dos partes (ciudad y estado, ciudad y código
      postal); una sola parte es un atributo.

    Las respuestas las aplica la copia (G3·4).
  - **El rango de la pareja lat/lon no lo comprueba la inducción**: el catálogo no trae muestra ni
    mínimos y máximos. Pasa a G3·4, donde la copia construye el punto (fuera de rango: la regla de lo
    que no analiza). Es un cambio respecto al plan, que pedía el rango aquí.
  - **V2, medida en local** con el `ore` de la rama sobre los catálogos reales del banco
    (`v2-juzgar.py`, que la vara de la celda lleva ya dentro):
    - **exhaustividad 9/16**, que son **9/9 de lo que G1 puede**: los 4 nativos de PostGIS, los 2 de
      BigQuery, el par, el WKT y las direcciones;
    - los 7 que faltan son de **G3·3**: la clave `geo` del GeoParquet, y GeoJSON, GPKG, KML,
      FlatGeobuf, `.gdb` y Shapefile, que hoy el driver da como objetos de medios;
    - **precisión 1,00**.
  - **Falsos positivos en las fuentes reales: ninguno.** 11 catálogos de victor y demo (389 tablas,
    las mismas bases repetidas), con 7 propuestas distintas:
    - los 3 pares lat/lon de T7, sin `longitud_focal_mm`;
    - las direcciones de Olist (`customer_city`/`customer_state` y las de `sellers`),
      `clientes.direccion` y los orígenes y destinos de `flujos_migratorios_estorninos`.

    `observatorios.ubicacion_geografica` se escapaba, y entra con `ubicacion`.
  - **Pruebas:** `ore-cli` 163/163, con `el_tipo_geo_nativo_se_da_sin_preguntar` y
    `el_texto_geo_y_las_direcciones_se_preguntan`; la suite entera en verde salvo la prueba de CR, que
    en Docker no puede llamar a `git` desde un worktree (en la CI sí).
  - **Lo que esto deja en la copia:** una `Table` de BigQuery con `Geography` llega hoy como WKT por el
    driver, y la copia la dejaría como texto con su aviso hasta G3·2 (`ST_ASBINARY`). PostGIS ya entrega
    hex EWKB, que la copia acepta.
- **2026-10-16 · G2·0 medido: el paso de `binary` a `geometry`** (`pruebas-de-fuego/geo/g20/`,
  laboratorio local). Los ficheros de datos los escribe Rust (`parquet` 58): uno con GEOMETRY nativo
  y otro con el mismo WKB plano. Se leen con Iceberg Java 1.11 + Spark 4.1 y con DuckDB 1.5.6.

  | | Spark 4.1 | DuckDB `iceberg` | Iceberg Java genérico |
  |---|---|---|---|
  | **A · hoy**: columna `binary`, fichero GEOMETRY nativo | lee | lee (BLOB; `ST_GeomFromWKB`) | **falla**: *Unsupported logical type: GEOMETRY* |
  | A · columna `binary`, fichero plano | lee | lee (BLOB) | lee |
  | paso por la API (`updateColumn`) | — | — | **rechazado**: *Cannot change column type: binary → geometry* |
  | paso solo en metadatos, tabla **v2** | — | — | **rechazado**: *Invalid schema for v2* (`geometry` es de v3) |
  | paso solo en metadatos, tabla **v3** | — | — | aceptado (mismo id de campo, esquema nuevo) |
  | **C · tras el paso**, fichero nativo | **falla** la versión actual (*Cannot convert unsupported type to Spark: geometry*); **lee el snapshot de antes** | **lee como `GEOMETRY`** | falla (el tipo lógico) |
  | C · tras el paso, fichero plano | falla la actual; lee el de antes | **falla** (*BLOB → GEOMETRY*) | lee (`ByteBuffer`) |

  - **Decisión G2·0:**
    1. **Las tablas geo nacen en v3**, con la columna `binary` y la propiedad `ore.geo.<col>`. Una
       tabla v2 no puede pasar nunca a `geometry` sin reescribirse.
    2. **Los ficheros, con el tipo lógico GEOMETRY** (G2·1, como se decidió). Es lo que deja a DuckDB,
       nuestro motor, leer la columna como `GEOMETRY` en cuanto se dé el paso, sin reescribir un
       byte. Con ficheros planos, el paso rompería a DuckDB.
    3. **El paso se puede dar solo en metadatos, sin reescribir datos, y conserva legibles los
       snapshots de antes.** Lo escribe ORE en sus metadatos, porque la API de Iceberg no lo admite
       como promoción. Así queda confirmado, y acotado a v3, el «sin reescribir» que G1·1 había dejado
       en duda.
    4. **Pero no se da todavía.** Hoy rompería a Spark y al lector genérico de Java. Se da por tabla
       cuando la matriz de adopción (G2·5) pase sobre una tabla ya promovida, y esa fila entra en la
       matriz.
  - **Lo que se acepta a cambio:** con ficheros GEOMETRY nativos, el lector genérico de Iceberg Java
    1.11 no lee la tabla ni con la columna `binary`. Spark y DuckDB sí. Afecta a quien use la API
    genérica de Java (Flink, herramientas propias). Entra en la matriz de G2·5 y en D: comprobarlo con
    Iceberg 1.12 y, si sigue, informar upstream.
  - **Otro detalle medido:** `parquet` 58 omite el CRS en el tipo lógico cuando es `OGC:CRS84` (el
    de por defecto en Parquet). Está bien según la spec de Parquet, pero G2·1 lo escribe explícito
    en la clave `geo` (T5).
- **2026-10-16 · G2·1 hecho: el escritor de las tablas geo.**
  - **v3 en el escritor de siempre.** Hallazgo: `ore-store` armaba los snapshots a mano con
    manifiestos v2, y una tabla v3 los rechazaba (*first-row-id must be set for format version >=
    v3*). Ahora, como hace la transacción de `iceberg`, una tabla v3 lleva manifiestos y lista v3 y su
    `row_range` en el snapshot. Prueba: `una_tabla_v3_acepta_los_snapshots_del_escritor`.
  - **`lago_geo.rs`.** Si los lotes traen una columna con la marca `geoarrow.wkb`:
    - la tabla pasa a v3, y cada columna geo deja `ore.geo.<col>` con `{"type", "crs", "edges"}`
      (`carga::campo` lleva el tipo de OOS en `ore:type`);
    - los ficheros los escribe ORE con `parquet` (*feature* `geospatial`), que pone el tipo lógico
      GEOMETRY/GEOGRAPHY y el bbox por grupo de filas;
    - la clave GeoParquet `geo` 1.1.0 lleva `WKB`, los tipos vistos, el bbox, `edges` y el **CRS en
      PROJJSON**;
    - se parten al mismo tamaño objetivo que los de `iceberg` y se registran con su `DataFile`.

    Las tablas sin geo no cambian de camino.
  - **El PROJJSON viaja con el binario** (`crs_projjson.json`, 403 KB, 172 CRS: WGS 84/CRS84, ETRS89 y
    sus UTM, ED50, las 120 UTM de WGS 84, Web Mercator, LAEA, los nacionales europeos y
    norteamericanos habituales). Lo generó `crs-projjson.py` con pyproj 3.7.2 / PROJ 9.5.1.
    - **Medido:** pyproj no acepta un PROJJSON mínimo (solo autoridad y código), así que hace falta
      el objeto entero.
    - Un CRS que no está en la tabla va como `crs: null` en la clave (desconocido). El código sigue
      en el tipo lógico y en la propiedad. Ampliar la tabla es regenerarla.
  - **El `DataFile`** lleva filas, tamaño, valores y nulos por columna de primer nivel. **Sin cotas**
    en el manifiesto: las funciones de `iceberg` que las calculan son privadas. Las estadísticas por
    grupo de filas siguen en el fichero, y la geometría tiene su bbox. Queda como deuda: la poda por
    manifiesto de las columnas escalares de una tabla geo.
  - **Pruebas** (`ore-store` 65/65): `una_copia_con_geometria_llega_al_lago` comprueba la tabla v3,
    `ore.geo.geom`, el tipo lógico GEOMETRY y la clave `geo` con el PROJJSON de `OGC:CRS84`. Fundir
    lee de nuevo esos ficheros. Workspace entero sin avisos y conformidad 31/31.
  - **V3, medida en local** (`examples/vara-v3-escribir.rs`, 1 M de puntos, almacén en memoria):
    - **escribir 2,18 M filas/s** (objetivo ≥ 1 M: cumple);
    - **tipar 0,97 M filas/s** (hex → WKB validado → Arrow);
    - la copia entera por texto da 143 000 filas/s, y el resto del tiempo es el camino de texto de
      `sellar` (JSON fila a fila), no el formato. Por eso G3 debe traer la geometría de los drivers
      en Arrow;
    - el fichero: 21,4 MB para 1 M de puntos.
  - **Adopción, primera comprobación** (`v5-adopcion-local.py`, la matriz entera es G2·5):
    - **DuckDB** lo lee como `GEOMETRY` con `geo_bbox` por grupo;
    - **GeoPandas** `read_parquet` da 1 M de filas con `OGC:CRS84`;
    - **pyarrow** ve la clave `geo`, el PROJJSON y la marca `geoarrow.wkb`, pero su `logical_type`
      sale `None` (esta versión no lo nombra; a la matriz).
  - **Lo que queda para G2·2:** hoy cada fichero es un solo grupo de filas de hasta 1 M (lo de
    `parquet` por defecto), sin orden espacial.
- **2026-10-17 · G2·2 hecho: el orden y las estadísticas, decididos por V4 en victor.**
  - **Las variantes:** los 34 M de edificios de Overture, escritos cinco veces con DuckDB
    (`geoparquet_version 'BOTH'`: GEOMETRY nativo, `geo_bbox` por grupo y clave `geo`), en
    `gs://…-banco-geo/g22/`. **Corrige T2:** DuckDB 1.5.6 **sí** escribe el tipo lógico nativo si se le
    pide `V2` o `BOTH`; por defecto (`V1`) solo pone la clave.
  - **Cómo se midió:** en un puesto de victor contra GCS (`vara-v4-g22.py`, lanzado con
    `correr-v4-g22.py`), con `spatial` y `h3` traídas del bucket, porque el puesto no tiene salida a
    internet (`puesto-extensiones.py`; meterlas en la imagen es G2·3). Pod de 4 GiB, el máximo de la
    cola. Dos pasadas completas con números casi iguales (`v4-g22-abcd.log`, `v4-g22-20261017-1106.log`)
    y la de H3 y particiones (`v4-g22-20261017-1115.log`).

  | variante | descarte de grupos | 1 lector p95 | 8 lectores p95 | 32 lectores p95 |
  |---|---|---|---|---|
  | A · tal cual, grupo de 100 000 | 86–97 % | 2,3–2,5 s | 17–29 s | 80–86 s |
  | **B · Hilbert, 100 000** | **98,5–99,4 %** | **0,62–0,74 s** | 5,2–5,8 s | 14,6–14,9 s |
  | C · Hilbert, 25 000 | 99,3–99,8 % | 0,60–0,71 s | 4,2–4,7 s | 15,7–17,4 s |
  | D · Hilbert, 400 000 | 96,5–98,8 % | 1,3 s | sin memoria (110 MB por lector) | — |
  | E · Hilbert, particionado por H3 r3 (90 ficheros) | 5–7 ficheros | 1,0 s | 5,3 s | 20,2 s |

  - **Hallazgo: `ST_Intersects` solo no descarta nada.** Tarda 19–30 s por zona, el fichero entero.
    `geom && caja` sí usa `geo_bbox` (0,4–2,9 s, como las columnas `bbox` de Overture). La consulta
    por zona exacta es `geom && caja AND ST_Intersects(geom, caja)`. Quien genere SQL sobre el lago
    (el cómputo, E1; el servir, E2) tiene que añadir el `&&`, y la guía del SDK lo dirá (G2·3).
  - **Decisión G2·2:**
    1. **Orden Hilbert al escribir:** el índice de Hilbert del centro del bbox, en una rejilla de
       2¹⁶ × 2¹⁶ sobre la extensión de los datos, global, con las nulas y vacías al final. Es lo que
       más gana: entre 3 y 4 veces menos latencia y descarte del 99 %.
    2. **Grupos de 100 000 filas.** Misma latencia que 25 000 con la cuarta parte de metadatos;
       400 000 es más lento y no cabe en memoria con lectores concurrentes.
    3. **Sin columna H3 de serie.** Precalculada no gana nada (0,49 s frente a 0,43 s calculándola al
       vuelo sobre la zona ya descartada). Se calcula cuando se pida (E1).
    4. **Sin particionado** a este tamaño: un solo fichero ordenado descarta mejor que 90 ficheros
       por celda. Se vuelve a medir con los 1 000 M de posiciones (lo geotemporal, después de G3).
  - **El objetivo de P2 con 32 lectores** (p95 < 1 s) **no se cumple en un solo pod**: da 15 s. Es la
    CPU de un pod de 4 núcleos atendiendo 32 consultas, no el fichero (con un lector, 0,6 s). Queda
    para la capa de servir (E2): varios lectores, caché de metadatos y de teselas (0066 T6).
  - **En el escritor** (`lago_geo.rs`): `ordenar_hilbert` antes de escribir y
    `GRUPO_DE_FILAS = 100_000`. Pruebas `la_curva_de_hilbert` y `el_orden_hilbert_junta_lo_cercano`;
    `ore-store` 67/67.
  - **V3 de nuevo:** escribir baja de 2,18 a **0,81 M filas/s**, por debajo del objetivo de 1 M. Cada
    WKB se lee tres veces (tipar, ordenar y resumir la clave `geo`). Deuda medida: leerlo una vez.
    1 M de puntos dan 10 grupos de 100 000.
- **2026-10-17 · G2·3 diseñado** (en el plan, «G2·3 · La lectura, en detalle»): el puntero de `datos` dice qué columnas son geo (desde los tipos del árbol), los SDK envuelven la columna con `ST_GeomFromWKB` y su CRS, pandas da un `GeoDataFrame`, `normalizar` acepta el binario con la marca `geoarrow.wkb`, `spatial` y `h3` en las tres imágenes, `ore.tipos.Geometry` en las firmas y un `en_zona` con `&&`. Lo local primero; las imágenes y la medida en victor, al empujar. Esperando el go.
- **2026-10-17 · G2·3·0 medido: el paquete de Python** (`g230-paquete-python.py`,
  `g230b-sin-pyogrio.py`, en la imagen `puesto-python:1` real, de 325 MB, sobre el millón de puntos que
  escribe ORE).

  | opción | añade a la imagen | 1 M de filas desde el Arrow de DuckDB | CRS |
  |---|---|---|---|
  | `geopandas` completo | 157 MB (de ellos `pyogrio` y su GDAL, 106 MB) | `GeoDataFrame`, 1,13 s | sí (`EPSG:25830`) |
  | **`geopandas` + `shapely` + `pyproj`, sin `pyogrio`** | **49 MB** | **`GeoDataFrame`, 0,88 s** | **sí** |
  | `shapely` solo | 9 MB | geometrías sueltas, 1,14 s | no (shapely no lo lleva) |
  | `geoarrow-pyarrow` | 9 MB | Arrow tipado, no un DataFrame | se perdió |

  - **DuckDB ya entrega `geoarrow.wkb` con el CRS en PROJJSON completo** (trae PROJ dentro), también
    con `::GEOMETRY('EPSG:25830')`. El Arrow del SDK sale bien sin ningún paquete.
  - **De vuelta:** `GeoDataFrame.to_arrow(geometry_encoding="WKB")` da `geoarrow.wkb` con el CRS en
    PROJJSON en 0,94 s. Es lo que recibirá `write()` (G2·3·3), que lo pasa a `AUTORIDAD:CÓDIGO` por su
    `id`.
  - **Decisión G2·3·0:** `geopandas` 1.2.0 + `shapely` 2.2.0 + `pyproj` 3.8.0 en `provisto.txt`, **sin
    `pyogrio`**: +49 MB (325 → unos 374 MB, un 15 %). Leer ficheros (`read_file`) lo hace ORE con
    DuckDB y GDAL de `spatial`; quien quiera `read_file` en su código añade `pyogrio` en la capa de su
    repositorio. Con geometría, pandas da un `GeoDataFrame`; si la imagen no trae `geopandas` (una
    anterior), da el Arrow con su marca y lo avisa.
- **2026-10-17 · G2·3·3 hecho: el SDK de Python escribe geometría.**
  - **`carga::normalizar`** (lo que `ore-store escribir` hace con el Arrow IPC de un `write()`):
    - una columna binaria (también `LargeBinary` y `BinaryView`) **con la marca `geoarrow.wkb`** es una
      geometría (`geo_que_llega`), y sin la marca un binario se sigue negando;
    - su CRS sale de la marca (`crs_de_geoarrow`): una cadena `AUTORIDAD:CÓDIGO` (la de ORE), o
      PROJJSON con su `id` o `ids` (la de GeoPandas y DuckDB), en objeto o en cadena;
    - **sin CRS se niega** (uno que no se sabe no es `OGC:CRS84`, `01-la-geometria` §1), y también
      un PROJJSON sin código de autoridad;
    - cada WKB se lee y valida (`geo::comprobar`, que también comprueba el SRID de un EWKB) y queda en
      WKB ISO; un valor roto se dice con su fila;
    - sale con la marca canónica de ORE y `ore:type` (`Geometry<Any, …>`), así que el lago toma el
      escritor geo (G2·1, G2·2).
  - **Hallazgo de la prueba de punta a punta:** una tabla geo **nueva** escrita por `write()` no se
    podía confirmar (*Cannot downgrade FormatVersion from v3 to v2*). Los cimientos de la tabla
    llevaban la versión de partida (v2) delante de la subida a v3, y quien la rehace desde cero la
    construía en v3 y luego intentaba bajarla. Ahora los cimientos llevan ya v3.
  - **El SDK de Python:** `_arrow_de` reconoce un `GeoDataFrame` (antes que pandas, porque es su
    subclase) y lo pasa a Arrow con `to_arrow(geometry_encoding="WKB")`. `_es_geo` reconoce la columna
    marcada (en los metadatos o, con `geoarrow-pyarrow`, en el tipo), y la tabla que se esboza la
    declara `binary`.
  - **Pruebas:**
    - `ore-store` 69/69, con `normalizar_acepta_la_geometria_marcada` (CRS de GeoPandas, de ORE,
      `ids`, sin CRS, sin código, SRID que no casa, binario sin marca) y
      `un_write_con_geometria_llega_al_escritor_geo` (IPC como lo da GeoPandas, con `EPSG:25830` en
      PROJJSON → tabla v3, `ore.geo.geom`, GEOMETRY y clave `geo` con su CRS, y se lee igual);
    - el SDK, con `test_geo_al_escribir` en la imagen del puesto con `geopandas` 1.2.0: 106 pruebas,
      todas en verde.
  - **Lo que queda para G2·3·2:** leerla de vuelta como geometría. Hoy el SDK la lee como BLOB, porque
    el puntero aún no dice qué columnas son geo (G2·3·1).
