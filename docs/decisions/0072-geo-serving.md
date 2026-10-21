# 0072 · Geo serving — llevar el lago al mapa

**Estado:** propuesto (2026-10-21). **Hoja de ruta mientras se desarrolla; al cerrarse, la imagen** de
cómo ORE sirve datos geoespaciales a un mapa. Nace del bastión, [`0066 · Geo paradigm`](0066-geo-paradigm.md)
(línea E2, que se desarrolla aquí). Se apoya en [`0067 · Geo formats`](0067-geo-formats.md) (el lago
vectorial: WKB, Hilbert, grupos de 100 000 filas) y en [`0068 · Geo compute`](0068-geo-compute.md)
(DuckDB y Spark con `spatial`/Sedona y H3). Toca [`0047`](0047-ore-access-control.md) (acceso: A8) y
[`0063`](0063-data-health.md) (la frescura de lo precalculado).
Fuera: lo geotemporal (E3: el mapa con tiempo), el ráster (E4: teselas desde COG), los datos de
referencia (E5) y publicar estándares (E7: OGC API Tiles/Features), que se apoyarán en esto.

## 1. Qué es

Lo que hace que **un dataset del catálogo, de cualquier tamaño, se vea y se explore en un mapa**, con
los permisos de quien mira y la frescura de lo que ve:

| pieza | qué |
|---|---|
| el motor de teselas | convierte «esta capa, esta caja, este zoom» en lo que el mapa pinta |
| la puerta | ore-serve: quién pide, qué puede ver, qué instantánea |
| lo precalculado | pirámides para lo masivo, como activo gobernado |
| el sustrato | el orden espacial del lago, que deja leer sólo lo que se ve |
| la consola | el mapa plano (MapLibre) y el satélite 3D (Google) con datos del catálogo |

## 2. La vara (decidida, 2026-10-21)

«Plasmar datasets masivos sin despeinarnos, con precisión, rendimiento y fiabilidad excepcionales.» En
números, para medirla:

**Masivo.** Una capa de **1 000 M de puntos** o **100 M de polígonos** se navega igual que una de 1 000.
Se valida con el banco: AIS (~127 M de posiciones a la semana), SpatialBench sf100 (600 M de viajes) y
los 34 M de edificios de G2.

**Precisión: el mapa nunca miente.**
- **Nada desaparece en silencio.** Si a un zoom no caben todos los elementos, se agregan y el mapa lo
  dice (agregado, y de cuántos). CARTO devuelve vacía la tesela que pasa su límite; ORE nunca.
- **Los agregados son exactos:** el conteo o la suma de una celda cuadra con un `count(*)` sobre el lago.
- **La geometría se simplifica por debajo del píxel** del zoom en que se pinta y sigue siendo válida
  tras recortarla.
- **Reproyectar es correcto** (del CRS del dato a 3857, con el orden de ejes fijado: `geometry_always_xy`).
- **El clic devuelve el dato del lago**, la fila con su geometría exacta, no la simplificada de la tesela.

**Rendimiento** (provisional; lo fija E2·0):

| qué | objetivo |
|---|---|
| tesela con caché | p95 < 50 ms |
| tesela en frío | p95 < 300 ms |
| primer pintado de una vista | < 1 s |
| el navegador | 60 fps |
| concurrencia | 50 personas sobre la misma capa sin que empeore el p95 |

**Fiabilidad.**
- **Determinista:** misma instantánea + mismo filtro + misma tesela = mismos bytes. La caché no se
  invalida nunca y puede vivir en el borde (Cloudflare ya está en el camino).
- **Los permisos fallan cerrados:** si el filtro no se puede escribir, no sale nada.
- **Se degrada con elegancia:** una tesela que se pasa de plazo baja un nivel de detalle (agregado); nunca
  sale vacía ni con error.
- **Sin punto único de fallo:** varias réplicas en el camino del mapa.
- **Fresco y gobernado:** lo precalculado tiene linaje a la instantánea de la que sale y salud en Data
  Health; si el dataset cambia se sabe y se reconstruye sólo lo tocado.

## 3. Lo que sabemos

### 3.1 Las prácticas para pintar datos a escala

Lo fundamental no es la tesela: es **no mandar nunca más de lo que la pantalla puede mostrar**, con un
índice espacial (leer sólo lo visible), nivel de detalle (no lo mismo a z4 que a z16) y una respuesta con
tope.

| práctica | cómo | alcance | quién |
|---|---|---|---|
| teselas vectoriales (MVT) | rejilla z/x/y, cada tesela recortada, simplificada y con tope | ilimitado, con caché y CDN | Mapbox, CARTO, Felt, MapLibre |
| el dataset entero en binario | Arrow/GeoArrow al navegador y GPU (deck.gl) | ~1–5 M de puntos (descarga y RAM) | Kepler.gl, lonboard |
| consulta por ventana | la caja visible agregada a su zoom, al pararse el mapa | ilimitado, cachea mal | visores internos |
| ráster renderizado en el servidor | una imagen de densidad | miles de millones, sin interacción | Datashader |
| agrupar en el cliente | supercluster | ~100 k – 1 M | Leaflet, Google |
| agregados precalculados | H3/quadbin por resolución | ilimitado a zoom bajo | CARTO (tileset de índice espacial) |
| LOD jerárquico 3D | árbol de detalle (3D Tiles, COPC) | ilimitado | Cesium, Google 3D |

La consulta por ventana, el LOD 3D y los agregados acaban siendo teselas con otro nombre: la rejilla fija
gana porque cada tesela se cachea una vez para todos, se pide en paralelo y carga progresivamente.

### 3.2 Medido en 0066 (T6, 2026-10-16)

DuckDB `spatial` (`ST_AsMVT`, `ST_TileEnvelope`) sobre Parquet ordenado por Hilbert, 5 M de puntos y
200 000 polígonos, una máquina de 16 núcleos:

| caso | elementos | tiempo | tamaño |
|---|---|---|---|
| puntos z14 a z10 | 11 a 4 806 | ~105 ms | hasta 118 KB |
| puntos z8 | 76 419 | 246 ms | 1,9 MB |
| puntos z6 | 1,25 M | 2 s | 31,8 MB |
| con filtro de permisos | — | igual o menos | — |
| agregados H3 en z6 / z4 | 992 / 50 celdas | 330 / 380 ms | — |

- Al vuelo funciona hasta zoom medio; el suelo (~100 ms) es abrir el Parquet: pide caché de teselas y de
  metadatos.
- El filtro de permisos es gratis: un `WHERE` más.
- A zoom bajo no se mandan los puntos crudos: se agregan (H3) o se precalculan.

El guion de T6 no se guardó en el repositorio (quedó en el scratchpad de aquella sesión).

### 3.3 CARTO (verificado en su documentación, 0066 bitácora del 2026-10-17)

- **Lo normal es dinámico:** una consulta por tesela recortada a su caja, que simplifica por zoom, agrega
  puntos en una malla, con límite de elementos y plazo, y caché en capas.
- **El *tileset* precalculado** (tabla `z/x/y` + MVT, una fila de metadatos con `bounds`, zooms y
  `tilestats`) es para lo que lo dinámico no alcanza. Se combinan: precalculado a zoom bajo, dinámico a
  zoom alto.
- **No hay frescura automática:** el tileset se regenera a mano.

### 3.4 La consola hoy (`rubix-platform`, mirado el 2026-10-21)

Cambió desde T8 (16 de octubre). El estado nuevo está **sin commitear** (otra sesión trabaja en él):
- **Dos motores** (`components/map/MapClient.tsx:63-65`):
  - **plano: MapLibre 5.24** + react-map-gl 8, bases vectoriales de CARTO (Positron, Dark Matter);
    Esri ya no está;
  - **«Satellite 3D»: Google 3D Maps**, el `<gmp-map-3d>` (`Map3DElement`) de la Maps JavaScript API,
    librería `maps3d`, canal beta. Cesium se probó del 17 al 20 de octubre y se quitó: Google responde
    403 a las Photorealistic 3D Tiles (Map Tiles API) para proyectos facturados en el EEE.
- **Los datos siguen siendo `mock.ts`** (14 objetos). Ninguna llamada a ORE.
- **Se pinta con marcadores**, uno por objeto: DOM en el plano, `Marker3D` en el 3D (tope 1 000 y
  agrupación en rejilla por encima). Su propio banco: 2 000 marcadores a 24 fps, 5 000 a 1 fps.
- La interfaz es rica (capas, leyenda, histograma, selección, dibujo, medidas, edición) y se reutiliza;
  falta lo de abajo.

### 3.5 Lo que acepta cada motor del mapa

- **MapLibre:** fuente `vector` (MVT), `raster` y `geojson`. Las teselas entran directas.
- **`<gmp-map-3d>` de Google: no acepta teselas propias.** Pide sus teselas por dentro. Acepta marcadores
  (también interactivos), polilíneas, polígonos, modelos glTF y *popovers*, uno a uno; ni GeoJSON
  directo, ni capas tipo deck.gl, ni WebGL propio. Está en **Preview** (gratis y sin SLA; de pago por uso
  en la GA). La vía de Google con datos propios en 3D (deck.gl sobre Photorealistic 3D Tiles) es la que
  da 403 en el EEE.
  Fuentes: [referencia de Map3DElement](https://developers.google.com/maps/documentation/javascript/reference/3d-map),
  [3D Maps](https://developers.google.com/maps/documentation/javascript/3d/overview),
  [deck.gl con Google Maps](https://deck.gl/docs/api-reference/google-maps/overview).

### 3.6 Dónde puede correr (código de 2026-10-21)

- **ore-serve** (Rust) es la puerta: la consola llega a él (Cloudflare → `cloudflared` → Envoy →
  `ore-serve:8080`, una sola regla en `malla/43-la-entrada.yaml`), tiene identidad, rama e instantánea, y
  ya devuelve binario (`Salida::Bytes`). Pero **«decide y no sirve»**: una prueba le prohíbe enlazar
  `iceberg` y `ore-store` (`crates/ore-cli/tests/dependencias.rs`), y lee datos lanzando subprocesos
  (`ore datasets --muestra`). Es **una réplica con `Recreate`**, 500m/512Mi.
- **ore-motor** no es Rust: es `puesto/python/ore_motor.py` (205 líneas) en la imagen `puesto-python:1`,
  DuckDB 1.5.6 con `spatial` y `h3` en la imagen. **Motor puro sin identidad** (0057 B4·3): sólo lo
  alcanza ore-serve (`malla/58-el-motor.yaml`), lee el lago con `iceberg_scan` y la credencial prestada
  que le pasan. Atiende **una petición a la vez** (candado global, un DuckDB nuevo por petición), sin
  caché, una réplica de 1 CPU/1 GiB, y devuelve Arrow que ore-serve pasa a JSON.
- **No hay nada de teselas en el código** (ni MVT, ni `/tiles`, ni `z/x/y`). En Rust no hay crates de
  geometría de cómputo (ni `geo`, ni `geozero`, ni `mvt`, ni `h3o`); hay `wkb`, `geojson` y `wkt`.

### 3.7 El orden espacial del lago (código de 2026-10-21)

**A medias.**
- **Sí:** el escritor de Rust (`ore-store`, camino del SDK Python, copias y colecciones) ordena por
  **Hilbert** el centro del bbox de cada geometría (`lago_geo::ordenar_hilbert`), en grupos de 100 000
  filas con **bbox por grupo** (parquet 58 `geospatial`) y la clave GeoParquet `geo` con un bbox por
  fichero. G2·2 midió el descarte: 98,5–99,4 % de los grupos con Hilbert, 86–97 % sin orden (consultas con
  `&&`).
- **No:**
  - **Spark no ordena**, y sus ficheros no llevan bbox por grupo (escribe WKB en tablas v2);
  - el orden **no se declara** a Iceberg (`SortOrder::unsorted_order`, sin partición);
  - **los `append` lo degradan** (sólo se ordena el lote nuevo) y **no hay compactación**;
  - **el manifiesto no lleva cotas** (`lower_bounds`/`upper_bounds`) ni hay columnas de cobertura: no se
    descarta un fichero sin abrirlo (el suelo de ~100 ms de T6);
  - sólo se ordena por la primera columna geo;
  - sin medir: si el `&&` sigue podando a través de `iceberg_scan` y la envoltura `ST_GeomFromWKB` del SDK
    (G2·2 midió `read_parquet` directo).

### 3.8 El permiso al leer

El filtro de fila y columna al leer **no existe**: es la deuda A8 de 0047 («leer datos no pregunta»).
Hoy una lectura comprueba la organización y el flujo por dataset entero. «Cada uno ve sólo lo que puede
ver» depende de A8.

## 4. Las decisiones

| # | decisión | por qué |
|---|---|---|
| D-S-1 | **El motor de teselas es un motor puro, sin identidad**; ore-serve es la única puerta | el patrón de ore-motor: ore-serve autentica, resuelve dataset, rama e instantánea, escribe el filtro dentro de la consulta y presta la credencial; el motor calcula lo que le llega, sólo acepta a ore-serve y escala en réplicas sin estado. Un fallo del motor no puede saltarse un permiso porque no lo decide |
| D-S-2 | **En MapLibre, siempre teselas (MVT)** | un solo camino para cualquier tamaño: el motor decide dentro de cada tesela si van los elementos o su agregado según el zoom. MapLibre filtra y colorea por propiedades de la tesela; el clic pide la fila al lago |
| D-S-3 | **En el satélite 3D de Google, agregados con tope** | el `<gmp-map-3d>` no acepta teselas: lo masivo llega agregado (celdas H3 como polígonos, cientos o miles) desde el mismo motor; lo pequeño, como marcadores |
| D-S-4 | **La caché es de contenido**: clave = instantánea + capa + tesela + huella del filtro | una tesela de una instantánea no cambia: no se invalida nada, y puede vivir en el motor y en el borde |
| D-S-5 | **Lo precalculado es un activo gobernado** (el kind `Tileset` de 0066, sólo si se precalcula) | linaje a su instantánea, salud, reconstrucción de lo tocado: lo que CARTO no da |

El dataset entero en Arrow al navegador (deck.gl) queda fuera de la v1: sólo si un caso de análisis lo
pide.

## 5. Lo que queda por saber

**Del producto (lo decide el usuario):**

| # | pregunta |
|---|---|
| P1 | ¿El satélite 3D es para **contexto** o tiene que mostrar **datos masivos**? Si es lo segundo: mapa 2D de Google en satélite con deck.gl (`GoogleMapsOverlay` + `MVTLayer`, términos del EEE por verificar) o un satélite con licencia comercial como ráster en MapLibre |
| P2 | A8 de 0047: ¿cómo se expresa el filtro de fila y columna al leer? Es requisito del «cada uno ve lo que puede ver» |
| P3 | ¿Los objetivos de §2 valen? Lo fija E2·0 con números |

**Técnico (lo mide E2·0):**

| # | qué |
|---|---|
| T1 | **El motor:** A, ore-motor reforzado (DuckDB `ST_AsMVT`, sin candado, réplicas, bytes) frente a B, un servicio Rust nuevo (iceberg-rust + `parquet` leyendo sólo los grupos cuyo bbox toca la tesela + codificación MVT). Tiempo por tesela en frío y con caché, RAM, arranque y concurrencia (50 a la vez) |
| T2 | **El lago real:** qué descarta hoy una tesela sobre una tabla de ORE (Hilbert del escritor, Spark sin orden, tras varios `append`), y cuánto ganan las cotas en el manifiesto o las columnas de cobertura |
| T3 | **El umbral de lo precalculado:** a partir de qué tamaño y zoom el dinámico no llega, sobre el banco (AIS, sf100, edificios) |
| T4 | **El coste de reproyectar y simplificar** por tesela (25830 → 3857) |
| T5 | **El satélite 3D:** cuántos polígonos H3 aguanta el `<gmp-map-3d>` a 60 fps |

## 6. El plan

**E2·0 · El previo** (sin código de producto, en victor salvo lo que sea de navegador): T1–T5. Decide el
motor, los objetivos y los pasos.

Los pasos, provisionales hasta E2·0:

| paso | qué | dónde |
|---|---|---|
| E2·1 | **El sustrato:** Hilbert también en Spark (y por todas las columnas geo), cotas o cobertura en el manifiesto, compactar los `append` | `ore-store` · `puesto/python/ore/spark.py` |
| E2·2 | **El motor de teselas:** MVT y agregado H3, puro, con réplicas y caché de contenido | según T1 |
| E2·3 | **La puerta:** `/tiles/...` y `/mapa/fila` en ore-serve (identidad, instantánea, filtro, credencial); ore-serve con más de una réplica | `ore-serve` · malla |
| E2·4 | **Lo precalculado:** el kind `Tileset` construido como un Build, con linaje y salud | OOS · `ore-core` · Build |
| E2·5 | **La consola:** capas del catálogo en lugar de `mock.ts`; fuente `vector` en MapLibre; agregados en el satélite 3D | rubix-platform |

**Hecho cuando:** una capa del catálogo de 1 000 M de elementos se añade al mapa, se navega dentro de
los objetivos de §2 con 50 personas a la vez, y cada una ve sólo lo que puede ver.

## 7. Fuera de este ADR

| qué | dónde |
|---|---|
| el mapa con tiempo (deslizador, animación, teselas por ventana) | E3 |
| teselas ráster desde COG | E4 |
| datos de referencia de serie en el mapa | E5 |
| OGC API Tiles/Features sobre esto | E7 |
| editar formas desde el mapa con propuesta (0066 E2·4) | después de E2·5 |

## 8. Riesgos

- **A8 sin decidir** bloquea el «cada uno ve lo que puede ver»: sin él, el mapa sólo se puede abrir a quien
  ya puede leer el dataset entero.
- **ore-serve, una réplica con `Recreate`:** un despliegue corta el mapa de todos.
- **El `<gmp-map-3d>` en Preview:** su API y su precio pueden cambiar en la GA.
- **iceberg-rust 0.10** no escribe cotas de geometría; esperar a 0.11 + parquet 59 (0067) o escribir
  columnas de cobertura.

## 9. Bitácora

- **2026-10-21 ·** Abierto, desde la línea E2 de 0066. Recogido: las prácticas para pintar a escala, T6 y
  CARTO, la consola de hoy (MapLibre plano + Google 3D Maps, todavía con `mock.ts`), lo que acepta el
  `<gmp-map-3d>` (no teselas propias), ore-serve frente a ore-motor y el orden espacial del lago.
  Decididos con el usuario: la vara (§2), el motor puro sin identidad (D-S-1) y siempre teselas en
  MapLibre (D-S-2). Siguiente: E2·0.
