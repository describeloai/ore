# 0066 · Geo formats — el dato geoespacial y geotemporal, de punta a punta

**Estado:** abierto (2026-10-16). Es el **bastión de trabajo del vertical**: aquí se acumula lo
aprendido, lo investigado, lo medido y, después, lo decidido y construido. Todavía **no hay plan por
pasos**: primero las preguntas (§6). **Decide**, cuando se cierre: cómo representa, tipa, calcula y
sirve ORE el dato geoespacial y geotemporal. Toca [`0032`](0032-el-contrato-de-tipos.md) (tipos),
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

### 5.4 Orden tentativo (se fija tras §6)

1. El tipo y el vectorial canónico, con GeoArrow desde PostGIS y BigQuery.
2. ST_*, H3, MVT y la vista de mapa: la primera aplicación geo de punta a punta.
3. Objetos en movimiento.
4. Ráster (COG + STAC).
5. Zarr, COPC y la publicación OGC API.

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

## 7. Bitácora

Lo aprendido, investigado, medido y decidido se apunta aquí, con fecha, en el orden en que pasa.

- **2026-10-16 ·** Abierto. Recogido el vertical (§1), la lista limpia de orígenes (§2), el estado del
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
