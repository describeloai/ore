# 0066 · Geo paradigm — el bastión del dato geoespacial y geotemporal

**Estado:** abierto (2026-10-16), y no se cierra: es el **bastión del vertical**. Aquí están los
**fundamentos y las primeras aproximaciones** del paradigma geoespacial y geotemporal de ORE: el
vertical, los orígenes, lo que hay, las decisiones de producto, la hipótesis de trabajo y las medidas.
Aquí nacen los conceptos, y desde aquí se expande en ADRs propios, cada uno acotado y cerrable. El
primero es [`0067 · Geo formats`](0067-geo-formats.md) (G0–G3: el banco, el tipo, el lago vectorial y
los códecs). Las siguientes líneas de expansión están en §7.

Toca [`0032`](0032-el-contrato-de-tipos.md) (tipos),
[`0033`](0033-el-dataset.md) (el lago), [`0043`](0043-ore-arrow-stream.md) (Arrow),
[`0046`](0046-ore-media.md) y [`0061`](0061-origenes-de-objetos.md) (objetos y colecciones),
[`0047`](0047-ore-access-control.md) (quién ve qué, también en un mapa),
[`0053`](0053-ore-federation-engine.md) (leer sin copiar) y [`0065`](0065-postgres-origen-de-verdad.md)
(PostGIS).

## 1. El vertical

**El vertical de ORE es la inteligencia geoespacial y geotemporal sobre el lakehouse gobernado.** No es
un mapa encima de otra cosa. Es la capacidad de punta a punta de **integrar, transformar y modelar**
los conjuntos de datos del catálogo de activos hasta convertirlos en **aplicaciones geoespaciales y
geotemporales completas**.

Por eso este ADR parte del **estado del arte del ámbito**, no de copiar a nadie. La comparación con
Foundry sirve para medirse, no de plantilla:
- en Foundry el núcleo es la ontología, y su mapa es una aplicación encima;
- su geografía es un **tipo de propiedad**: `geopoint` es el texto `"lat,lon"` (WGS 84) o un geohash,
  y `geoshape` es el texto de una geometría GeoJSON;
- su almacenamiento no cambia ([docs](https://palantir.com/docs/foundry/geospatial/ontology/)).

ORE puede ir más allá: guardar la geometría **como geometría**, con estadísticas espaciales, en el
mismo formato abierto del lago.

**Los clientes:**
- movilidad (Uber), telecomunicaciones (Vodafone), energía, renovables, *utilities*;
- inmobiliario, construcción;
- cadena de suministro y logística a gran escala (tierra, mar y aire);
- seguros, banca y servicios financieros;
- gobiernos centrales, regionales y locales: gestión de emergencias, protección civil, agricultura de
  precisión y sector forestal.

**Ninguno tiene «un origen geoespacial».** Su información está repartida:
- en **sistemas operativos** (ERP, CRM, SCADA, telemática), donde la geografía suele ser una dirección
  o un par lat/lon;
- en unas pocas **islas GIS** (Esri, PostGIS, Oracle Spatial).

El trabajo es leer de todas, unir y llegar al *awareness*.

## 2. Los orígenes del vertical (la lista limpia, 2026-10-16)

Lo que se lee hoy y lo que se va a construir, con la prioridad acordada. «Prioritario» es *to-do*;
«nice-to-have» se hace cuando se pida.

| categoría | ya hay | prioritario (to-do) | nice-to-have |
|---|---|---|---|
| **Objetos** | S3 (y compatibles: R2, MinIO…), GCS, Azure Blob/ADLS, SFTP, SharePoint/OneDrive (0046, 0061) | — | — |
| **Bases de datos** | PostgreSQL (PostGIS; RDS, Cloud SQL, AlloyDB, Azure, Supabase, Neon: 0065), BigQuery | **Oracle** (Spatial) · **SQL Server** (con Azure SQL, Synapse y Fabric por TDS) · **Snowflake** · **Databricks** · **MySQL** (con MariaDB, Aurora, TiDB y PlanetScale) | MongoDB · Elasticsearch/OpenSearch · ClickHouse · Redshift · SAP HANA. Por JDBC: Teradata, DB2, Informix, Sybase, Cassandra, DynamoDB, Cosmos DB |
| **Protocolos genéricos** | — | **JDBC** · **REST API desde código** (credenciales del cofre más *egress policies*; cubre todas las APIs de dominio: telemática, agricultura, clima, Copernicus…) · **SMB** | FTP/FTPS, NFS, LDAP: descartados por ahora |
| ***Streaming*** (el producto ORE Streaming aún no existe) | — | **Kafka** (Confluent, MSK, Redpanda, Event Hubs) · **MQTT** (y su perfil industrial **Sparkplug B**) · **Google Pub/Sub** · **Amazon Kinesis** | OPC UA en suscripción, AMQP, webhooks |
| **Feeds de dominio** (sobre *streaming* o REST) | — | **AIS** (marítimo) · **ADS-B** (aéreo) · **GTFS-Realtime** · **CAP** (alertas) | — |
| **Servicios GIS** | — | **ArcGIS Feature Services** · **OGC WFS / OGC API Features** · **STAC API** | WCS / OGC API Coverages · OGC API EDR · SensorThings · ArcGIS Image Services · Copernicus / Sentinel Hub / openEO |
| **Lakehouse y catálogos abiertos** | — | **Iceberg REST Catalog** · **Delta Sharing** · **Unity Catalog** · **AWS Glue** | Hive Metastore · Snowflake Sharing · Fabric OneLake · Hudi/Paimon · BigLake/Dataplex |
| **Aplicaciones de negocio** | — | **SAP** · **Salesforce** · **Dynamics 365 / Dataverse** · **IBM Maximo** · **ServiceNow** | Oracle Fusion/EBS/OTM · Guidewire/Duck Creek · Blue Yonder/Manhattan/Infor · Workday · NetSuite/HubSpot/Zendesk/Jira |
| **Industrial / OT** | — | **AVEVA PI** · **OPC UA** · **MQTT Sparkplug B** · **AWS IoT SiteWise / Azure IoT + Data Explorer / Cognite Data Fusion** | — |
| **Colaboración** | SharePoint/OneDrive | **Google Drive (con Sheets como tablas)** · **Box** · **Dropbox** | — |

**Capas de mapa, que no son orígenes** (se pintan, no se ingieren): WMS, WMTS, XYZ, MVT, PMTiles.
**Descubrimiento**: CSW / OGC API Records.

**Transversales** que la lista hace imprescindibles:
- **geocodificación y normalización de direcciones**: Catastro, CartoCiudad, OSM/Overture y un
  proveedor (HERE, Google, Esri) donde haga falta precisión;
- **el incremental de cada origen**: CDC de Salesforce, ODP de SAP, *snapshots* de Iceberg/Delta,
  *offsets* de Kafka;
- **OT siempre en sólo lectura y nunca contra la red de control**: la réplica del historiador en la
  DMZ, o la nube del cliente;
- **las *egress policies*** para código del cliente que sale a internet («REST API desde código»).
  En los drivers no hacen falta (0065 PG3).

## 3. Lo que hay hoy (medido en el código, 2026-10-16)

- **El lago:** Iceberg con ficheros Parquet (`iceberg = "0.10.1"`, `crates/ore-store`).
- **Tabular desde objetos:** Parquet (esquema exacto), CSV/TSV (esquema inferido y propuesto) y JSONL
  (`ore-read-objetos/src/medio.rs:16-20`). Nada más: ni Excel, ni Avro, ni GeoParquet *como geo*.
- **Familia `geospatial`** (OOS v1alpha29): GeoJSON, KML/KMZ, GPX, GML, GeoPackage, Shapefile y
  LAS/LAZ se **reconocen** (`ore-core/src/medios.rs:193-199`), pero **no se abren**: se sirven como
  descarga. Además:
  - GeoTIFF/COG cae en `image`, NetCDF en `scientific` y DWG/DXF en `model3d`;
  - GRIB, Zarr, PMTiles, FlatGeobuf e IFC no se reconocen.
- **No existe un tipo de geometría.** `Fisico` tiene Texto, Entero, Real, Lógico, Decimal, Fecha, Hora,
  FechaHora e Instante (`ore-core/src/tipos.rs:45-66`). La geometría llega como texto:
  - el `GEOGRAPHY` de BigQuery es `Opaque` (`ore-read-bigquery/src/catalogo.rs:148`);
  - PostGIS llega como su EWKB en hexadecimal (0065 PG2).
- **No hay vista previa de mapa.** Está pendiente la P3 de las familias de medios: GeoJSON/KML/GPX en
  MapLibre. Tampoco hay código de mapa en ORE. En el puesto sí hay `shapely` entre los paquetes
  sugeridos.
- **Lo que sí juega a favor:**
  - los drivers y la federación ya hablan Arrow (0043, 0053);
  - las colecciones virtuales y mantenidas ya saben leer en sitio o copiar (0046, 0061);
  - el control de acceso se aplica igual a cualquier lectura (0047).

## 4. Qué es «un formato» en ORE

Un formato es cómo están organizados los bytes. En ORE aparece en **tres sitios**:
1. **En el origen del cliente: lo que leemos.** El cliente no aprovisiona nada; sus ficheros están donde
   estén. «Soportar» un formato es poder **catalogarlo** (columnas, tipo de geometría, CRS, extensión,
   número de elementos) y **leerlo**.
2. **En el lago: lo que guardamos.** Aquí no hay elección: hay **una representación canónica** por
   forma de dato (§5). Lo que se copia se convierte a ella. Lo que no es tabla (un COG, un LAZ) se
   guarda tal cual, con sus metadatos y su huella.
3. **En la salida: lo que servimos.** Teselas para pintar, GeoJSON por API, exportaciones y servicios
   estándar.

**Leer sin copiar**: los formatos nativos de nube (GeoParquet, COG, COPC, Zarr, FlatGeobuf) se leen
**en el bucket del cliente** por rangos de bytes, como una colección o tabla **virtual**. Los de siempre
(Shapefile, KML, GeoPackage, `.gdb`) se leen enteros, así que se **copian y convierten** (colección
**mantenida**).

## 5. La propuesta (hipótesis de trabajo, por validar en §6)

**El principio:** en el lago, **una representación canónica, abierta y nativa de nube para cada forma
del dato geo**, con la geometría como tipo de primera clase. Todo lo demás son **códecs** de entrada y
salida. El lago sigue siendo Iceberg con Parquet, extendido donde el sector ya lo ha extendido.

### 5.1 Las cinco formas y su representación canónica

| forma | en el lago | por qué |
|---|---|---|
| **Vectorial** (entidades con geometría y atributos) | **Iceberg v3** con `geometry`/`geography` nativos (WKB), metadatos **GeoParquet 1.1** (CRS en PROJJSON, columna *bbox*), **orden espacial** al escribir (Hilbert) y una columna **H3** | es hacia donde han convergido Iceberg, Parquet, DuckDB, Sedona, BigQuery y Snowflake; con estadísticas espaciales, una consulta por zona descarta ficheros sin abrirlos |
| **Ráster e imagen** | **COG** como fichero de verdad, catalogado con **STAC**, más una **tabla Iceberg índice** (escena o tesela, huella, fecha, bandas, nubosidad) | el sector ya no guarda ráster dentro de tablas: guarda COG y lo indexa. La tabla índice une el ráster con lo demás |
| **Cubos multidimensionales** (clima, meteorología, océano) | **Zarr v3 / GeoZarr**, con su tabla índice | es hacia donde van ECMWF, Copernicus y NASA para tiempo × espacio × variable |
| **Nubes de puntos** (LiDAR) | **COPC**, con su tabla índice | se lee por zona y por nivel de detalle sin bajar el fichero entero |
| **Objetos en movimiento** (flotas, AIS, ADS-B, sensores móviles) | **una tabla Iceberg de posiciones** (`id, t, punto, atributos`) ordenada por **celda + tiempo**, más **una tabla derivada de trayectorias** (líneas con medida temporal, segmentadas por paradas o viajes) | el corazón de lo geotemporal: el índice espaciotemporal y la trayectoria como objeto de primera clase |

Para más adelante: **redes** (carreteras, red eléctrica, tuberías) como grafo de nodos y aristas con
geometría, y **3D/BIM** (3D Tiles, IFC).

### 5.2 Las cuatro capas

1. **El tipo (OOS).**
   - `Geometry<subtipo, CRS>` y `Geography` en `Table` y entidades, y tipos de activo para `Raster`,
     `Datacube` y `PointCloud`, con su tabla índice.
   - **La disciplina del CRS:** se guarda fiel al origen (por ejemplo ETRS89/UTM, con su metadato) y se
     ofrece en WGS 84 para servir y unir. Nunca se reproyecta en silencio.
   - La inducción propone el tipo: columnas lat/lon, WKT, GeoJSON, y la geometría de PostGIS,
     BigQuery, Oracle o SQL Server.
2. **En memoria y en el cable: GeoArrow.** La geometría viaja de un origen al lago **sin pasar por
   texto**, con los tipos de extensión de GeoArrow sobre el Arrow que ya usan drivers y federación.
3. **El cómputo.**
   - Funciones **ST_*** (OGC Simple Features) y **H3** en el SQL y en los transforms.
   - *Joins* espaciales y espaciotemporales.
   - Ráster por ventana: estadísticas zonales.
4. **Servir: ORE como proveedor de estándares.**
   - **MVT** dinámicas y **PMTiles** precalculadas desde las tablas.
   - Teselas ráster dinámicas desde COG.
   - Todo filtrado por los permisos del catálogo (0047).
   - Y la publicación de los datos gobernados como **OGC API – Features / Tiles / Coverages** y
     **STAC**, para que QGIS, ArcGIS y cualquier cliente GIS de los clientes se conecten a ORE como a
     un servidor estándar.

### 5.3 Los códecs

- **Entrada:**
  - **en Rust nativo**, lo caliente: GeoParquet, GeoJSON, FlatGeobuf, WKB/WKT, y CSV/Excel/Sheets
    con lat/lon;
  - **con GDAL aislado** (servicio o contenedor convertidor), la cola larga: File Geodatabase de Esri
    (`.gdb`), Shapefile, GeoPackage, KML/KMZ, GML, GRIB, NetCDF, JPEG2000.
- **Salida:** exportar a Shapefile, GeoPackage, GeoJSON o CSV bajo petición, y teselas.

### 5.4 Orden (fijado en 2026-10-16)

Primero los fundamentos del formato, en [`0067`](0067-geo-formats.md): el banco, el tipo, el lago
vectorial canónico y los códecs. Después, las líneas de expansión de §7, cada una con su ADR.

## 6. Lo que hay que saber antes de diseñar los pasos

### 6.1 Del producto (lo decide el usuario)

| # | pregunta | por qué importa |
|---|---|---|
| P1 | **¿Cuál es la primera aplicación geo de punta a punta** que tiene que salir (por ejemplo flotas en tiempo casi real, emergencias, renovables, riesgo de seguros), y para qué cliente o piloto? | decide qué forma va primero (vectorial, movimiento, ráster) y qué orígenes se necesitan ya |
| P2 | **¿Qué volúmenes reales** hay: posiciones al día, número de activos, TB de ráster, frecuencia de actualización? | decide el particionado, el orden espacial, la granularidad H3 y si hace falta *streaming* desde el principio |
| P3 | **¿Política del CRS?** Propuesta: fiel al origen más WGS 84 para servir. ¿Hay clientes que exijan trabajar en su CRS proyectado (ETRS89/UTM)? | decide si las medidas (áreas, distancias) se hacen en proyectado o en geografía esférica |
| P4 | **¿Qué es «la aplicación geo» en la consola?** ¿Un mapa por activo, un constructor de mapas y paneles, una app que se publica? ¿Hay ya decisión de librería (MapLibre, deck.gl) o diseños? | decide la capa de servir y qué teselas hacen falta |
| P5 | **¿Hay que publicar hacia fuera como servidor estándar** (OGC API y STAC para QGIS/ArcGIS de los clientes)? ¿Es venta o es más tarde? | es una pieza de peso; si vende, va antes |
| P6 | **¿Hasta dónde el cómputo de ráster** dentro de la plataforma: indexar y servir, o también procesar (índices, estadísticas zonales, mosaicos)? | define si hace falta un motor de ráster o basta con COG + STAC |
| P7 | **¿3D, BIM y redes** entran en el primer año? | si entran, condicionan el tipo y el servir desde el principio |
| P8 | **¿Qué datos de referencia** se dan «de serie» a todos los clientes (OSM/Overture, Catastro, límites administrativos, Copernicus)? | un catálogo de referencia compartido cambia la forma de unir y de geocodificar |

### 6.2 Técnico (lo mido yo, sin código de producto)

| # | qué medir | por qué |
|---|---|---|
| T1 | **`iceberg` de Rust 0.10:** ¿lee y escribe tablas v3? ¿Tiene los tipos `geometry`/`geography` y sus límites? Y, si no, ¿cómo convive GeoParquet 1.1 con Iceberg v2? | decide si el vectorial canónico es Iceberg v3 ya o GeoParquet sobre v2 mientras tanto |
| T2 | **El motor de consulta de ORE** (`ore-sql`, `ore-motor`, DuckDB si está): ¿qué funciones espaciales y H3 hay o se pueden traer (DuckDB spatial, Sedona en la JVM, DataFusion geo)? | decide dónde vive el cómputo ST_* |
| T3 | **GeoArrow en Rust** (`geoarrow-rs`) y su madurez; Arrow 58, que es lo que usan los drivers | decide si la geometría viaja nativa ya |
| T4 | **El coste de GDAL**: tamaño de imagen, compilación, aislamiento (contenedor aparte o *bindings*), y qué cubre `.gdb` con OpenFileGDB | decide Rust nativo frente a GDAL para cada códec |
| T5 | **El lado Python y Java del puesto y los transforms:** GeoPandas/Shapely/PyArrow-GeoArrow en Python, Sedona/JTS en Java. ¿Qué hay, qué falta y cómo viaja la geometría por el SDK? | el usuario escribe transforms geo ahí desde el primer día |
| T6 | **Servir teselas**: generar MVT al vuelo desde Iceberg/Parquet con filtro de permisos (tiempo por tesela), frente a PMTiles precalculadas | decide la arquitectura de la capa de servir |
| T7 | **Lo que hay hoy en BigQuery y PostGIS de verdad** (victor: el Neon `pg_quality` y las tablas de BigQuery): columnas geo, CRS, tamaños | el primer caso de prueba sale de datos que ya tenemos |
| T8 | **La consola** (`rubix-platform`): qué componentes de mapa existen y qué espera el diseño | encaja la vista de mapa con lo que ya hay |

## 7. Las líneas de expansión

Lo que viene **después de los fundamentos de [`0067`](0067-geo-formats.md)**, apoyado en ellos. Cada
línea se abre con su propio ADR cuando llegue, con sus medidas y su plan. Lo de abajo es la primera
aproximación, escrita el 2026-10-16 con lo medido en T1–T8. Las tres reglas de 0067 (polivalencia,
masivo desde el principio y laboratorio antes que producción) valen igual.

**Transversales**, que acompañan a las líneas cuando las piden:
- **geocodificación** (orígenes con direcciones, referencia de serie);
- **seguir y contribuir a `iceberg-rust`** (tipos geo y Arrow 59, T1/T3);
- **un servicio GDAL moderno aparte** para ráster y multidimensional (T4);
- **las *egress policies*** para código del cliente (§2).

### E1 · El cómputo

| paso | qué |
|---|---|
| E1·1 | **SQL**: ST_\* (OGC) y H3 en el editor, las vistas, `ore-motor` y los transforms SQL, con `ore-sql` sabiendo los tipos geo |
| E1·2 | **Python**: el SDK entrega GeoDataFrame (`from_arrow` sobre `geoarrow.wkb`) y acepta GeoDataFrame al escribir; `shapely`, `geopandas`, `pyproj` y `h3` en la imagen (+150 MB, T5) |
| E1·3 | **Java**: JTS y H3 (unos 2,3 MB) y la geometría por Arrow |
| E1·4 | **Enriquecer**: un *join* espacial (puntos en polígonos, agregación por zona o celda) como verbo de primera clase, base de E5 |

**Hecho cuando:** los *joins* espaciales de la vara (por ejemplo 1 000 M de puntos contra los
municipios) cumplen su objetivo en los tres lenguajes.

### E2 · Servir y el mapa

| paso | qué |
|---|---|
| E2·1 | **El endpoint de teselas**: MVT al vuelo desde el lago con el filtro de permisos del catálogo (0047), con caché de teselas y de metadatos (T6: unos 100 ms de suelo sin caché) |
| E2·2 | **Zoom bajo**: agregación H3 automática y pirámides precalculadas (PMTiles) para lo grande. La vara decide los umbrales |
| E2·3 | **La consola**: las capas salen del catálogo (fuera `mock.ts`), con fuente `vector` de MapLibre y deck.gl para millones de puntos. La interfaz que ya existe se reutiliza (T8). Vista previa de mapa en la ficha del activo (la P3 de las familias de medios). Mapa base y satélite con licencia comercial |
| E2·4 | **Acciones desde el mapa**: dibujar o editar una forma escribe en la propiedad tipada, con propuesta y gobierno |

**Hecho cuando:** un usuario añade al mapa una capa del catálogo de 100 M de elementos, navega con
la latencia objetivo con N usuarios concurrentes, y cada uno ve sólo lo que puede ver.

### E3 · Lo geotemporal

| paso | qué |
|---|---|
| E3·1 | **El modelo**: una tabla de posiciones (`id, t, geometría, atributos`) como tipo de activo, ordenada por celda + tiempo, y su tabla derivada de trayectorias: líneas con medida temporal, segmentadas por paradas o viajes |
| E3·2 | **El cómputo**: trayectoria de un objeto, quién estuvo en una zona entre dos fechas, cercanía espaciotemporal (encuentros) y paradas |
| E3·3 | **El mapa**: deslizador de tiempo, animación y estela, con teselas por ventana de tiempo |
| E3·4 | **La entrada continua**: micro-lotes ya; *streaming* cuando exista ORE Streaming (§2) |

**Hecho cuando:** la vara reproduce los cientos de millones de posiciones AIS del banco con consultas
espaciotemporales y animación en el mapa dentro de objetivo.

### E4 · Ráster

| paso | qué |
|---|---|
| E4·1 | **COG + STAC**: el ráster como objeto COG con su tabla índice Iceberg (huella, fecha, bandas, nubosidad), catalogado como activo. Los orígenes STAC de §2 entran aquí |
| E4·2 | **Un servicio GDAL aparte**, moderno (3.10+, imagen OSGeo), para convertir a COG y calcular (T4) |
| E4·3 | **Cálculo ráster**: índices (NDVI…), álgebra de bandas, pendiente y orientación, estadísticas zonales contra el vectorial, y mosaicos. Por transforms |
| E4·4 | **Teselas ráster dinámicas** desde COG con permisos, en el mismo mapa |

### E5 · Referencia de serie

| paso | qué |
|---|---|
| E5·1 | **El catálogo público**: límites administrativos, Overture (edificios, lugares, carreteras), Catastro, INE y Copernicus. Activos compartidos para todas las organizaciones (0059), sin copia por cliente, con licencia y procedencia |
| E5·2 | **Enriquecer** (E1·4) contra ellos desde la interfaz y desde código |
| E5·3 | **Premium y de terceros**: el modelo de licencia (como el de CARTO), cuando haya demanda |

### E6 · Redes y 3D

- **Redes** como grafo de nodos y aristas con geometría: conectividad, afectados aguas abajo y rutas.
- **LiDAR** en COPC con su tabla índice.
- **Cubos** en Zarr (clima y meteorología).
- **3D** (modelos de ciudad, 3D Tiles) en el mapa.

Cada uno se abre con su propia medida cuando llegue.

### E7 · Publicar estándares

OGC API Features y Tiles sobre lo de E2, y STAC sobre lo de E4, con los mismos permisos. Largo plazo
(P5); el sustrato no lo impide desde 0067.

## 8. Bitácora

Lo aprendido, investigado, medido y decidido se apunta aquí, con fecha, en el orden en que pasa.

- **2026-10-16 ·** Abierto (como «0066 · Geo formats»). Recogido el vertical (§1), la lista limpia de orígenes (§2), el estado del
  código (§3), el modelo de «formato» (§4), la hipótesis de trabajo (§5) y las preguntas (§6). Sin
  código.
- **2026-10-16 · Decisiones de producto (P1–P4):**
  - **P1 · Polivalencia.** No hay una primera aplicación: la visión se apoya en la flexibilidad de los
    estándares, que son el fundamento, y en las capas de abstracción que cada caso de uso vaya
    pidiendo. El orden de §5.4 lo deciden la base común y las medidas, no un vertical concreto.
  - **P2 · El sustrato soporta cargas masivas y concurrencia desde el primer momento.** El
    particionado, el orden espacial, las estadísticas espaciales y el servir con concurrencia son
    requisitos del primer paso, no una optimización posterior.
  - **P3 · CRS:** fiel al origen y WGS 84 para servir. Aprobado.
  - **P4 · La aplicación es un mapa**, todavía lejos de estar terminada.
  - **P5–P8**, explicadas y pendientes de respuesta: publicar como servidor estándar (OGC API/STAC
    para el QGIS o ArcGIS de los clientes); calcular con ráster, no sólo guardarlo y pintarlo; el
    valor de redes (alto), 3D (medio-alto) y BIM (más tarde); datos de referencia de serie para todos.
  - **T1–T8 son medidas**, y son el fundamento del plan: dan los números y la información fehaciente
    con la que se diseña.
- **2026-10-16 · Decisiones P6–P8:**
  - **P6 · Ráster: guardarlo y calcular con él.** Hay que poder hacer índices, estadísticas zonales,
    pendientes o láminas de inundación dentro de la plataforma, no sólo pintar. Hace falta cómputo
    ráster (§5.2, capa 3).
  - **P7 · Redes y 3D entran; BIM todavía no.** Redes como grafo con geometría: conectividad,
    afectados aguas abajo, rutas. 3D: modelos de ciudad y LiDAR.
  - **P8 · Datos de referencia de serie: sí.** El referente es el **Data Observatory de CARTO**
    ([catálogo](https://carto.com/data-library),
    [suscripción](https://docs.carto.com/carto-user-manual/data-observatory/subscribing-to-public-and-premium-datasets),
    [enriquecer](https://carto.com/blog/enrich-spatial-analysis-bigquery-snowflake/)):
    - un catálogo de más de 12 000 datasets espaciales: demografía, geografías, movilidad, POIs,
      tráfico, medio ambiente, financieros, inmobiliario, comportamiento y derivados;
    - **públicos** (suscripción gratuita, licencia perpetua) y **premium** (de terceros como
      Mastercard, Experian o SafeGraph, con licencia que CARTO tramita);
    - suscribirse no copia por ETL: el dataset **aparece en el warehouse del cliente** (BigQuery,
      Snowflake, Databricks, Redshift, PostgreSQL o el de CARTO);
    - se usa con su *Analytics Toolbox*: más de 100 funciones espaciales, H3/Quadbin y
      **enriquecimiento**, un *join* espacial que agrega variables del catálogo sobre los polígonos o
      la malla del cliente.

    En ORE encaja como **activos públicos compartidos del catálogo**: visibles desde todas las
    organizaciones (0059), sin copia por cliente (0053), con licencia y procedencia en el activo y un
    verbo de **enriquecer**. Queda por investigar qué se ofrece de serie (OSM/Overture, límites, Catastro,
    INE, Copernicus) y cómo se licencia lo premium.
  - **P5**, explicada con ventajas e inconvenientes, pendiente de decisión.
- **2026-10-16 · P5 decidida:** publicar como servidor estándar (OGC API Features/Tiles, STAC) es un
  **objetivo a largo plazo**. El sustrato se diseña para que no lo impida, y se construye después del
  mapa propio.
- **2026-10-16 · Medidas T1–T4** (guiones y salidas en el *scratchpad* de la sesión; números de una
  máquina local, para comparar opciones, no para dimensionar producción):

  **T1 · Iceberg y la geometría.**
  - **`iceberg` de Rust 0.10.1** (la última, 2026-08-01, la que usa ORE):
    - **sabe de v3** (metadatos, *row lineage*, `UpgradeFormatVersion`; `ore-store` ya acepta
      `format-version: 3`, `ciclo.rs:1835`);
    - **pero no tiene los tipos `geometry`/`geography`**: `PrimitiveType` acaba en `Binary`
      (`spec/datatypes.rs:210-250`). *(Corregido el 2026-10-16: sí hay PR, ver la bitácora de [`0067`](0067-geo-formats.md).)*
  - **Consecuencia 1:** hoy ORE ni siquiera podría **abrir** una tabla v3 ajena con columna `geometry`
    (Snowflake ya la tiene en GA desde 2026-05-07, y AWS S3 Tables también). Afecta a los orígenes
    lakehouse (§2).
  - **Consecuencia 2:** el escritor de `iceberg` deriva el esquema Arrow del de Iceberg
    (`writer/file_writer/parquet_writer.rs:527`). Una geometría pasaría como `Binary` sin tipo ni
    estadísticas.
  - **Lo que sí está:** `parquet` 58.4 (el de ORE), con su *feature* `geospatial`, escribe y lee los
    **tipos lógicos GEOMETRY/GEOGRAPHY** de Parquet y sus **estadísticas espaciales por grupo de
    filas** (bbox), y los ve en Arrow como la extensión `geoarrow.wkb`
    (`arrow/schema/extension.rs:68-80`, `column/writer/mod.rs:1227`). Además, el escritor de ORE ya pasa
    `WriterProperties` (`lago.rs:937`), por donde pueden ir los metadatos GeoParquet.
  - **Referencia:** [Snowflake v3 GA con geometry/geography](https://docs.snowflake.com/release-notes/2026/other/2026-05-07-iceberg-v3-ga),
    [S3 Tables v3](https://dev.classmethod.jp/articles/20261012-amazon-s3-tables-iceberg-v3/),
    [Iceberg geo (Wherobots)](https://wherobots.com/blog/iceberg-geo-technical-insights-and-implementation-strategies/).

  **T2 · El motor de consulta: DuckDB 1.5.6** (el mismo en el puesto, `ore-motor`, Node y la JVM).
  - **El tipo `GEOMETRY` ya está en el núcleo de DuckDB**, sin extensión.
  - Las extensiones de la imagen están preinstaladas y cerradas (`iceberg`, `avro`, `httpfs`, `json`,
    `icu`; `Dockerfile:433`, con `autoinstall_known_extensions=false`). **No están `spatial` ni `h3`.**
  - **`spatial`:** 78 MB, **167 funciones ST_\***, y un **GDAL vectorial de 54 drivers** dentro
    (OpenFileGDB, GPKG, Shapefile, KML, FlatGeobuf; **sin GTiff, GRIB ni netCDF**).
  - **`h3`** (de la comunidad): 512 KB, 76 funciones.
  - **5 M de puntos** (península): generar 2,4 s; ordenar por Hilbert 1,5 s; a Parquet 1,2 s (135 MB).
    Una consulta por zona (Madrid, 0,5°) tarda 0,39 s con `ST_Intersects` y 0,11 s con el bbox (`&&`).
    H3 res 8 de 5 M puntos: 0,52 s. GDAL lee y escribe GeoJSON, Shapefile, GPKG, FlatGeobuf y KML con
    20 000 elementos en 0,04–0,44 s.
  - **Lo que DuckDB escribe en Parquet:** metadatos **GeoParquet 1.0** (un bbox **por fichero** en la
    clave `geo`), **no** el tipo lógico GEOMETRY nativo, y sin bbox por grupo de filas.

  **T3 · GeoArrow en Rust, y lo que casa con lo nuestro.**
  - Versiones: `geoarrow` 0.9 (2026-09-11) exige **Arrow 59**, igual que `geoparquet` 0.9 y
    `geodatafusion` 0.6 (DataFusion 55). ORE está en **Arrow 58**, fijado por `iceberg` 0.10.1. Con
    Arrow 58 casan **`geoarrow` 0.8** y `parquet-geospatial` 58.4. Para Arrow 59 hay que esperar a una
    versión de `iceberg` que lo use.
  - **Prueba en Rust** (Arrow 58 + `parquet` 58.4 geospatial): 2 M puntos en 0,87 s y 71 MB, con tipo
    lógico `Geometry` y **bbox por grupo de filas**.
  - **El mismo fichero, en DuckDB 1.5.6:** sin extensión lo lee como `GEOMETRY`, ve el bbox de cada
    grupo (`parquet_metadata().geo_bbox`), y con `spatial` filtra por zona.
  - **La interoperabilidad Rust (escritura) ↔ DuckDB (lectura) del Parquet geo nativo funciona hoy.**
  - Otros: `geo` 0.33, `h3o` 0.11, `wkb` 0.9, `flatgeobuf` 6, `geozero` 0.15, `proj` 0.31, `gdal` 0.19.

  **T4 · El coste de GDAL.**
  - **Vectorial:** viene **gratis** con la extensión `spatial` de DuckDB (78 MB), con la File
    Geodatabase de Esri incluida.
  - **GDAL completo** (Debian bookworm, `gdal-bin`): **GDAL 3.6.2** (de 2023, viejo), **+263 MB**,
    144 drivers ráster y 79 vectoriales: GTiff, COG, GRIB, netCDF, Zarr, JP2, HDF5, OpenFileGDB, GPKG,
    Shapefile, LIBKML, FlatGeobuf. Para compilar los *bindings* de Rust hacen falta **+507 MB** más
    (`libgdal-dev`).
  - Un GDAL moderno (3.10+, con GeoParquet y Zarr v3) no viene en Debian estable: habría que traer la
    imagen oficial de OSGeo o compilarlo.

  **Lo que dicen T1–T4 juntos:**
  1. **El dato vectorial puede ir ya como Parquet geo nativo** (tipo GEOMETRY y bbox por grupo),
     escrito desde Rust y leído por nuestro DuckDB. Lo que falta es que **Iceberg lo sepa**: o
     escribimos los ficheros de datos con nuestro escritor y los registramos en Iceberg como `binary`
     (v2/v3) hasta que `iceberg-rust` traiga el tipo, o contribuimos el tipo a `iceberg-rust`.
  2. **El cómputo ST_\* y H3 está a una extensión de distancia** (`spatial` + `h3`, unos 79 MB) en el
     motor que ya tenemos, en todas sus versiones (Python, Node, JVM).
  3. **GDAL vectorial ya lo tenemos dentro de `spatial`.** El ráster y lo multidimensional (COG, GRIB,
     netCDF, Zarr) piden un GDAL completo y moderno, mejor como **servicio convertidor aparte** que
     dentro de cada imagen.
  4. **El cuello de botella de versiones es `iceberg-rust`**: fija Arrow 58 y no tiene tipos geo.
     Merece seguimiento y, probablemente, contribución.
- **2026-10-16 · Medidas T5–T8:**

  **T5 · Python y Java del puesto, y cómo viaja la geometría.**
  - **Python** (pyarrow 25.0.1, pandas 3.0.6, DuckDB 1.5.6, lo que lleva la imagen).
    - `shapely` y `h3` están en las sugeridas, no instaladas.
    - pyarrow lee el Parquet GEOMETRY nativo como `binary` **con** la marca `geoarrow.wkb` en los
      metadatos del campo. DuckDB → Arrow (el camino del SDK) también lo entrega como `geoarrow.wkb`.
    - A pandas llega como `bytearray` (WKB): sin tipo geo.
    - **GeoPandas:** `read_parquet` **rechaza** un fichero que sólo lleve el tipo nativo («Missing geo
      metadata»: exige la clave GeoParquet `geo`). En cambio, `GeoDataFrame.from_arrow` sobre
      `geoarrow.wkb` sí funciona: 2 M puntos en 2,2 s, y de DuckDB a Arrow a GeoDataFrame en 3,9 s.
      `to_parquet` escribe la clave `geo` (GeoParquet) sin tipo nativo, y DuckDB lo lee como
      `GEOMETRY`.
    - **El CRS se perdió por el camino** (`crs=None`): el fichero de la prueba no lo declaraba
      explícitamente. Hay que escribirlo siempre explícito, en los dos sitios.
    - Coste: `shapely` + `h3`, +12 MB; `geopandas` + `pyproj`, +141 MB; `geoarrow-pyarrow`, +9 MB.
  - **JVM:** no hay librería de geometría. JTS 1.20 pesa 1,2 MB, Sedona-common 1.9, 2,1 MB y H3 4.1.1,
    1,1 MB. DuckDB por JDBC es el mismo motor (con `spatial` y `h3`).
  - **Conclusión:** **escribir los dos metadatos a la vez** (el tipo lógico GEOMETRY nativo **y** la
    clave GeoParquet `geo`, con CRS explícito) hace el fichero legible por todo el ecosistema:
    DuckDB, GeoPandas, Sedona y los lectores antiguos. Y el SDK debe entregar un GeoDataFrame, no
    bytes.

  **T6 · Teselas vectoriales (MVT) al vuelo.** DuckDB `spatial` tiene `ST_AsMVT`, `ST_AsMVTGeom` y
  `ST_TileEnvelope`. Prueba: 5 M de puntos y 200 000 polígonos en Parquet ordenado por Hilbert,
  máquina local de 16 núcleos, cada tesela leyendo del fichero como un servicio sin estado.

  | caso | elementos | tiempo | tamaño |
  |---|---|---|---|
  | puntos z14 / z12 / z10 (Madrid) | 11 / 285 / 4 806 | ~105 ms | 0–118 KB |
  | puntos z8 | 76 419 | 246 ms | 1,9 MB |
  | puntos z6 | 1 252 859 | 2 033 ms | **31,8 MB** |
  | puntos con filtro de permisos (5 de 50 grupos), z12 / z8 / z6 | 31 / 7 719 / 125 416 | 88 / 87 / 216 ms | 1 KB–3,2 MB |
  | polígonos z14 → z8 | 2 → 3 097 | 54–185 ms | hasta 263 KB |
  | puntos agregados en H3 res 5 en z6 / res 3 en z4 | 992 / 50 celdas | 330 / 380 ms | — |

  - **Al vuelo funciona hasta zoom medio** (unos 100 ms de suelo por tesela, que es abrir el Parquet).
  - **El filtro de permisos es gratis**, incluso acelera, porque es un `WHERE` más.
  - **A zoom bajo no se pueden mandar los puntos crudos** (31 MB por tesela): hay que **agregar**
    (H3, ~350 ms) o **precalcular** (PMTiles) las pirámides de lo grande.
  - Para producción, el suelo de ~100 ms pide caché de teselas y de metadatos del Parquet.

  **T7 · La geografía que hay en las fuentes reales** (`geo-t7-censo.sh`, Jobs en `t-victor` y
  `t-demo`; 18 fuentes):
  - **Ninguna columna con tipo geo del origen.** Ningún Postgres tiene PostGIS (uno tiene `vector`), y
    BigQuery no tiene `GEOGRAPHY`.
  - **La geografía es implícita:**
    - **3 tablas con pares lat/lon en `numeric`** (`flujos_migratorios_estorninos`, `hangars`,
      `registros_instalaciones_ngs`);
    - **15 columnas de dirección o lugar** (ciudad, estado, código postal, país, dirección: Olist,
      `clientes`, `censo_poblacional`…).

    Hubo un falso positivo por nombre (`longitud_focal_mm`).
  - **Confirma la tesis de §1** con nuestros propios datos: la geometría casi nunca llega como tal.
    Llega como coordenadas en números o como direcciones, y la **inducción del tipo** y la
    **geocodificación** son la puerta de entrada.
  - **Hace falta un banco de pruebas geo propio:** un Postgres con PostGIS, BigQuery `GEOGRAPHY`,
    GeoParquet, Shapefile, `.gdb`, COG y posiciones con tiempo, con volúmenes de verdad.

  **T8 · El mapa de la consola** (`rubix-platform`).
  - **Base:** **MapLibre GL 5.24 + react-map-gl 8**, unas 3 500 líneas en `components/map`: globo,
    mapa base CARTO Positron, satélite de Esri, dibujo y edición de formas, medidas, panel de capas,
    histograma, selección, menú de solapes, «añadir al mapa».
  - **Pero todo sobre datos de ejemplo** (`mock.ts`: tres tipos de objeto declarados a mano) y sin
    API.
  - **Pinta un marcador DOM por objeto** (`MapDataLayer`): sirve para cientos, no para millones.
  - **Fuentes `geojson`**, sin teselas vectoriales ni ráster propias.
  - **El satélite de Esri es de uso no comercial** (lo dice `basemaps.ts`): para la GA hace falta un
    proveedor con licencia.
  - **Conclusión:** la interfaz está avanzada y es reutilizable; lo que falta es lo de abajo.
    - capas por **teselas vectoriales** de ORE (fuente `vector` de MapLibre, o deck.gl para millones
      de puntos y lo geotemporal);
    - un **endpoint de teselas** con permisos;
    - **ráster** desde COG;
    - y la conexión con el catálogo en vez de `mock.ts`.
- **2026-10-16 · Reestructurado.** El plan G0–G10 mezclaba los fundamentos del formato con el producto
  entero del vertical. Este ADR pasa a ser el **bastión** (fundamentos y primeras aproximaciones del
  paradigma geo y geotemporal; de aquí nacen los conceptos y desde aquí se expande). **G0–G3 se van a
  [`0067 · Geo formats`](0067-geo-formats.md)**, acotado al formato: su representación, su distribución
  y su adopción. G4–G10 quedan aquí como líneas de expansión E1–E7 (§7).
