# 0072 · Geo serving — llevar el lago al mapa

**Estado:** propuesto (2026-10-21); E2·0 aprobado (D-S-6..12, 2026-10-22). **Hoja de ruta mientras se desarrolla; al cerrarse, la imagen** de
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
| D-S-3 | ~~En el satélite 3D de Google, agregados con tope~~ **Rehecha por D-S-12** (T6–T11): el satélite también recibe teselas, en glTF | el `<gmp-map-3d>` no acepta un servidor de teselas, pero sí modelos glTF: el cliente pone el árbol de teselas y cada tesela es un modelo, así que lo masivo llega igual que a MapLibre |
| D-S-4 | **La caché es de contenido**: clave = instantánea + capa + tesela + huella del filtro | una tesela de una instantánea no cambia: no se invalida nada, y puede vivir en el motor y en el borde |
| D-S-5 | **Lo precalculado es un dataset derivado, no un kind nuevo** (rehecha el 2026-10-22: sin kind `Tileset`). **Lo que es dato es un dataset; lo que es formato es caché**: la pirámide de agregados es un dataset construido por un Build; las teselas codificadas (MVT, glb), nunca un activo | linaje a su instantánea, salud y reconstrucción de lo tocado vienen con el Build y el dataset; la pirámide se puede consultar como cualquier tabla; guardar bytes codificados obligaría a un activo por formato y por estilo (el glb lleva el color dentro), que es lo que CARTO gobierna y nosotros no |

El dataset entero en Arrow al navegador (deck.gl) queda fuera de la v1: sólo si un caso de análisis lo
pide.

## 5. Lo que queda por saber

**Del producto (lo decide el usuario):**

| # | pregunta |
|---|---|
| P1 | **Respondida (2026-10-22): datos masivos**, por D-S-12. ~~¿El satélite 3D es para **contexto** o tiene que mostrar **datos masivos**?~~ Si es lo segundo: mapa 2D de Google en satélite con deck.gl (`GoogleMapsOverlay` + `MVTLayer`, términos del EEE por verificar) o un satélite con licencia comercial como ráster en MapLibre |
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

### E2·0 · Lo medido (2026-10-21)

**Cómo.** Jobs en t-victor con la imagen del puesto, en un nodo del pool pesado (n2-highmem-8, por
`cola-pesada`), sobre el lago real del inquilino (`pruebas-de-fuego/geo/e2/`, `correr.py`):
- **las capas**, escritas por el escritor de Rust de siempre (`ore-store-gcs escribir`, Hilbert, grupos de
  100 000, bbox por grupo) con la credencial del `stage-create`, y su `metadata.json` por `ore-store aplicar`.
  **Sin confirmar nada en el catálogo** (0052: sólo una persona crea un dataset): se leen por su
  `metadata_location` y se borraron al acabar.
  - `pts_h`: 20 M de puntos en EPSG:25830, con densidad de ciudades (Madrid ~30 %);
  - `pts_m`: los mismos en 3857;
  - `pts_a`: los mismos en 10 ficheros (lo que dejan 10 `append`);
  - `pts_s`: los mismos en un Parquet sin ordenar (lo que escribe Spark);
  - `edif`: 5 M de edificios (polígonos, 4326) del banco.
- **T1 en Rust**: un prototipo de laboratorio (`e2/teselas`, ~350 líneas): `parquet` 58 + `object_store`,
  descarta grupos por la estadística geo de cada uno, reproyecta con `proj4rs` y codifica MVT de puntos a
  mano, con una caché de pies de fichero.
- **T3** sobre los 600 M de viajes de SpatialBench sf100 (`trip.parquet`, 50 GB, sin orden).
- **T5** en el navegador del escritorio (Chrome), con la clave de la consola, en `localhost`.

**T2 · Lo que lee una tesela. Hallazgo principal: el camino del SDK no poda.**

| variante (20 M de puntos, DuckDB 1.5.6, 30 GB) | tesela vacía z14 | Madrid z14 (23 624 puntos, 534 KB) |
|---|---|---|
| `read_parquet` con la columna tal cual y `geom && caja` | **36 ms** | **87 ms** |
| la misma con `geom::GEOMETRY('EPSG:25830')` (lo que hace `_con_geo`) | 516 ms | 539 ms |
| el camino del SDK (`iceberg_scan` + `ST_GeomFromWKB`) | 722 ms | 757 ms |
| columnas de cobertura `x`/`y` (min/max de Parquet) | 98 ms | 161 ms |
| sin orden (`pts_s`), con `&&` | ~500 ms | ~640 ms |

- **El escritor ya deja todo lo necesario:** el bbox de cada grupo (comprobado en `parquet_metadata`) y el
  orden Hilbert hacen que DuckDB lea sólo lo que toca la tesela, **pero sólo si la columna llega al scan
  como la `GEOMETRY` del fichero**.
- **Envolverla la deja sin poda:** con el *cast* del CRS o con `ST_GeomFromWKB` (el SDK, porque Iceberg
  dice `binary`), DuckDB lee las 20 M de filas en cada consulta. Vale para toda consulta espacial del SDK
  sobre el lago (también `ore.geo.intersects_box`), no sólo para el mapa.
- **Diez `append`** (diez ficheros, cada uno ordenado por sí mismo) cuestan como uno sin poda. Hay que
  compactar.
- **Un fallo, de paso:** un `append` de geometría con el SDK falla siempre («`geom` es `LargeBinary` en la
  tabla y llega `Binary`»). `sin_perdida` (`ore-store/src/carga.rs:1124`) no acepta `Binary → LargeBinary`,
  que no pierde nada.

**T1 · El motor: DuckDB frente a Rust** (los mismos ficheros, el mismo nodo de 8 vCPU).

| | DuckDB (`read_parquet`, poda, caché de pies) | Rust (prototipo) |
|---|---|---|
| tesela Madrid z14, 25830 | 87 ms | 198 ms (lee 4 de 200 grupos) |
| tesela Madrid z14, guardada en 3857 | — | **90 ms** (2 de 200 grupos) |
| tesela Madrid z12 (355 551 puntos, 8 MB) | 608 ms | 678 ms / 520 ms en 3857 |
| 1 lector, teselas z10–z14 al azar | 3,5 teselas/s · p50 60 ms | 2,3–3,4 teselas/s · p50 130–183 ms |
| **32 lectores** | **9,8 teselas/s** · p50 1,7 s · p95 14,7 s | **50 teselas/s (25830) · 88 (3857)** · p50 183–360 ms · p95 1,4–2,2 s |
| 64 lectores | 10,8 teselas/s · p95 18,7 s | 47–89 teselas/s · p95 2,7–5,0 s |
| memoria | 30 GB de límite; con los 488 MB del SDK, OOM en z4–z8 | **RSS < 1 GB** con 64 lectores |
| arranque | — | 260 ms |

- **Una tesela suelta, empate.** Con la poda bien hecha, los dos van al bbox de cada grupo y tardan lo
  mismo.
- **Con concurrencia, Rust gana de 5 a 9 veces.** DuckDB no reparte bien muchas consultas a la vez en
  un proceso (cada una quiere todos los hilos); Rust sirve cada tesela en su tarea, sin estado.
- **Reproyectar en Rust cuesta el doble** (`proj4rs` por punto): servir de una copia en 3857, u otra
  transformación más rápida.
- Los p95 de 1–2 s salen de teselas z10–z11 de Madrid con **millones de puntos crudos**, que la vara no
  manda nunca: van agregadas (T3).

**T3 · Dónde deja de llegar lo dinámico.**

| qué | medida |
|---|---|
| puntos crudos por tesela en Madrid (20 M) | z14: 23 624 (534 KB) · z12: 355 551 (8 MB) · z10: 2,9 M (68 MB) |
| polígonos crudos (5 M edificios) | z14: 4 868 (259 KB, 380 ms) · z12: 36 507 (1,5 MB, 870 ms) · z10: 244 291 (7 MB) |
| polígonos válidos tras recortar (z12) | 36 219 de 36 219 |
| agregado H3 al vuelo sobre 20 M (sin poda) | 1,0–3,7 s por tesela |
| dinámico sobre 600 M sin orden | z12: 13,9 s · z8: 16,1 s (2,4 M de puntos, 26 MB) |
| **precalcular la pirámide H3 de 600 M** | **117 s** (res 8, una pasada, 8 vCPU) + 5 s para subir a res 7…3 |
| filas de la pirámide | res 8: 105 M · res 7: 40 M · res 5: 1,1 M · res 3: 22 426 |
| servir desde la pirámide como tabla plana | z4–z8: < 0,2 s · z10 (res 7, 40 M filas): 7,2 s |

- **El tope de una tesela cruda es de ~50 000 elementos (~1 MB).** Por encima, agregado. En una capa densa
  el cambio llega pronto (Madrid: entre z13 y z14 con 20 M de puntos), así que **el motor decide por
  tesela, no por zoom**, con el conteo de la pirámide.
- **Lo masivo sin orden no se sirve al vuelo** (14–16 s por tesela). Precalcular es barato: 2 minutos
  por 600 M en un nodo.
- **La pirámide no se guarda como tabla plana**: sin orden, filtrarla es otra vez leerla entera (7,2 s a
  res 7). Se guarda por tesela (`z/x/y`, como el tileset de CARTO) o con el mismo Hilbert y bbox por grupo.

**T4 · Reproyectar y simplificar.**
- **DuckDB:** guardar en 25830 o en 3857 no cambia nada medible; manda el scan.
- **Rust:** 25830 cuesta ~2× lo de 3857 (`proj4rs` por punto).
- **El recorte de `ST_AsMVTGeom`** deja los polígonos válidos (0 inválidos de 36 219).

**T5 · El satélite 3D de Google** (`<gmp-map-3d>` beta, `Polygon3DElement` hexagonales sobre Madrid,
cámara en vuelo):

| polígonos | 0 | 100 | 250 | 500 | 750 | 1 000 | 5 000 | 10 000 |
|---|---|---|---|---|---|---|---|---|
| fps en vuelo | 60 | 60 | 60 | 51 | 39 | 31 | 7 | 4 |
| añadirlos | — | 77 ms | 161 ms | 291 ms | 321 ms | 468 ms | 2,1 s | 4,1 s |

**Tope del satélite 3D: ~300 polígonos** para 60 fps, y hasta ~500 con una pequeña caída. Es un PC de
escritorio; en un portátil, menos.

**T6 · Datos a escala sobre el 3D de Google (2026-10-21).** Por qué no se puede, desde los primeros
principios: el `<gmp-map-3d>` es un motor cerrado que sólo acepta objetos sueltos (cada polígono es un
elemento DOM con su coste propio: el coste va por objeto, no por vértice), y el camino abierto a la misma
imagen (sus teselas 3D en un motor propio) lo cierra la Map Tiles API en el EEE. Dos vías medidas
(`pruebas-de-fuego/geo/e2/t6_glb.py`, `t6_pagina.html`):

| vía | qué | medida |
|---|---|---|
| **A · muchos datos en pocos objetos** | N hexágonos en **una sola malla glTF** (color por vértice, sin luz) en un `Model3DElement` | 10⁴ a 10⁶ hexágonos: **60 fps en vuelo**, nítidos de cerca, con su oclusión real con edificios y relieve (dentro del motor de Google). Ver T7: la primera medida tenía la malla de pie (corregido) |
| **B · deck.gl encima, sincronizado** | un canvas de deck.gl transparente que copia la cámara del componente cada frame | **3 M de puntos a 60 fps en vuelo**. Pero **no encaja**: alineado en el centro, ~45 px desplazado fuera de él al primer intento. El componente no expone su proyección: encajar es reconstruir su cámara privada. Y no hay oclusión |

- **La vía A funciona.** Cientos de miles de elementos por modelo sobre el satélite 3D de Google, a 60 fps
  y en su sitio por construcción. El servidor generaría glTF por vista o por tesela (la misma pirámide
  H3, otro codificador), con tope de ~3·10⁵ elementos por modelo.
- **A cambio:** el clic es por modelo, no por elemento (se resuelve preguntando al servidor qué hay en ese
  punto); cambiar el estado es regenerar la malla (o trocearla en modelos pequeños por celda y cambiar
  sólo los que cambian); y la malla plana a una altura fija atraviesa el relieve en zonas con desnivel
  (pegarla al terreno pide la altura en el servidor, o modelos más pequeños).
- **La vía B se descarta** mientras Google no publique su proyección: rinde, pero no se puede garantizar
  que los datos caigan donde están.

**T7 · ¿Hay límite? Varios modelos y ficheros ligeros (2026-10-21).** Corrige T6: con `orientation`
`tilt: 0` el `Model3DElement` toma la Z del glTF como «arriba», y la malla quedaba **de pie, como una
cortina vertical** (parecía estar «al norte» y desaparecía vista desde arriba o de cerca). Con
`tilt: 90` queda plana sobre el suelo (comprobado en vista cenital). Repetido todo bien orientado
(`t7_teselas.py`, `t7_pagina.html`):

| caso | modelos | fps lejos / en vuelo / de cerca | se ve entero |
|---|---|---|---|
| 3·10⁵ hexágonos | 1 | 60 / 59 / 60 | sí, y los edificios altos asoman entre ellos (oclusión real) |
| **10⁶** | 1 | **60 / 60 / 60** | sí |
| **3·10⁶** | 16 | **60 / 60 / 60** | sí |
| 3·10⁶ | 64 | 60 / 60 / 60 | sí |

- **No hay un límite a la vista por modelo ni por mapa hasta 3·10⁶ elementos** en este PC: el coste es de
  la GPU, no de Google. El de T6 era el error de orientación.
- **Lo que pesa es el fichero.** El `Model3DElement` sólo pinta **glTF plano en float**: no pinta (y no
  avisa) ni `EXT_meshopt_compression`, ni `KHR_mesh_quantization`, ni `KHR_draco_mesh_compression`. Un
  hexágono plano ocupa ~144 B (6 vértices, 4 triángulos, color por vértice): 10⁶ = 137 MB en bruto.
- **La compresión del transporte lo arregla:** el mismo millón pesa **31,7 MB con gzip y 13,9 MB con
  brotli** (CDN). Descargar 3·10⁶ en bruto, incluso en local, llevó 4,8–6,5 s.
- **Para el diseño:** glTF plano por tesela, servido con brotli y caché de contenido; detalle por nivel
  (de lejos celdas gordas, de cerca sólo lo que se ve), así que en una vista rara vez hacen falta más de
  unos cientos de miles. Pendiente: un portátil modesto y un móvil.

**T8 · El catálogo visual, pegado al relieve (2026-10-21).** El relieve lo dibuja Google, pero no da la
altura de cada punto y un modelo es rígido: cada vértice tiene que llegar con su cota. Fuente: Copernicus
GLO-30 (DSM, 30 m, alturas sobre EGM2008), la tesela N40 W004. Dos zonas: el centro de Madrid y La Pedriza
(`t8_catalogo.py`, `t8_pagina.html`). Coordenadas del glTF en ENU (x este, y norte, z arriba) con
`orientation` 0.

| capa | resultado |
|---|---|
| H3 res 9, rampa secuencial | se ciñe a la sierra; asoma algún peñasco (la malla de Google es más fina que 30 m) · 59 fps |
| rejilla de 250 m, rampa divergente | igual · 60 fps |
| coropleta (Voronoi de 400 «secciones», densificadas) | en la ciudad, **a ras de suelo los edificios la tapan**; a +60 m sobre el suelo se ve entera |
| composición RGB (tres métricas a R, G, B) | funciona · 60 fps |
| columnas H3 y bloques de 500 m extruidos por métrica | base en el relieve, altura por métrica · 60 fps |
| **2·10⁵ columnas H3 res 11** sobre el relieve | **60 fps quieto, 58 en vuelo** |
| **10⁶ columnas H3 res 11** (74 teselas) | **60 fps quieto, 59 en vuelo**, cada columna distinguible de cerca |

- **La referencia de altura de Google, medida:** con `ABSOLUTE` y las cotas de Copernicus tal cual
  (EGM2008), la capa cae sobre el suelo, sin el desfase de ~50 m de geoide frente a elipsoide. En la
  práctica **`ABSOLUTE` es altura sobre el nivel del mar**, como Copernicus.
- **Anclar con `CLAMP_TO_GROUND` y hornear diferencias deja escalones** de varios metros entre teselas: el
  suelo de Google en el ancla no coincide al metro con el DEM. **Con `ABSOLUTE` y las cotas del DEM** todo
  sale de la misma fuente y la capa es continua. Decidido para el diseño: `ABSOLUTE`.
- **El margen** sobre el DEM de 4 m se queda corto frente a la malla de Google (asoman rocas y edificios):
  8–10 m en el campo; en la ciudad, la capa a ras de suelo la tapan los edificios (es oclusión real). Para lo
  urbano: volumétrico, o la capa por encima de los tejados.
- **Lo opaco, opaco.** Con el material en `BLEND` (transparencia) 2·10⁵ columnas van a **1 fps**; en
  `OPAQUE`, a 60. La transparencia, sólo para capas pequeñas.
- **Los modos de apoyo** que tiene que decir una capa: sobre el terreno (DEM + margen), por encima de los
  tejados (altura fija sobre el suelo), volumétrico (base en el DEM, altura por métrica) y absoluto (su
  propia cota). Pegarse a cada tejado como hace Cesium (clasificación en la GPU) no se puede dentro del
  `<gmp-map-3d>`.

**T9 · Los 48 estados de EE. UU. (2026-10-21).** Relieve gratis: Copernicus GLO-90 leído por trozos (la
versión 1/4 de cada COG, ~360 m): **957 teselas en 44 s**, guardadas. Curvatura: cada vértice en ECEF y
luego en el ENU de su tesela (una tesela plana de 1 000 km se separaría ~78 km). Capas (`t9_usa.py`,
`t9_pagina.html`): H3 res 3 a res 6 (617 a 212 990 celdas, 5–73 MB), 951 674 «tiendas» como puntos y su
10 % de más ingresos (95 168) como columnas.

| qué | resultado |
|---|---|
| H3 res 3, el país entero | **cubierto sin huecos**, 60 fps |
| H3 res 4 a 1 800 y 600 km · res 5 sobre Nueva York · res 6 en las Rocosas | 60 fps (54–60 en vuelo) |
| 952 000 puntos | 60 fps; **un punto glTF es 1 píxel**: densidad de lejos, casi invisible de cerca |
| 95 000 columnas «top 10 %» (Nueva York, Los Ángeles) | **60 fps** |

Cuatro lecciones, las cuatro necesarias para el diseño:
- **Z-fighting a distancia.** A miles de km, el buffer de profundidad no separa una capa a decenas de metros
  del suelo: el terreno asoma a manchas incluso en la llanura. La capa se separa **en proporción a la
  distancia a la que se usa** (res 3: 6 km; res 4: 2 km; res 5: 500 m; res 6: 150 m). Cambiar la posición de
  un modelo en vivo lo recarga: la separación va fija por nivel.
- **La envolvente del relieve.** Con triángulos de kilómetros, la cota interpolada entre vértices deja asomar
  las crestas. Cada vértice toma **la cota máxima de su entorno** (filtro de máximo con la ventana del tamaño
  del triángulo): la capa nunca queda por debajo del terreno, a cambio de flotar algo en los valles.
- **Google sólo activa un modelo cuando su ancla ha estado a la vista** (no documentado). Añadido con el ancla
  fuera de pantalla, no se pinta; en cuanto el ancla entra una vez, se pinta siempre. Con teselas de cientos
  de km para las resoluciones finas, el cambio de resolución al acercarse dejaba la pantalla vacía. **El
  arreglo es el diseño de 3D Tiles:** teselas por nivel de detalle, pequeñas de cerca y grandes de lejos, con
  el ancla dentro de la vista que las pide. Además, un `Model3DElement` quitado no se repinta al volver a
  ponerlo: cada cambio, elementos nuevos.
- **Individuos con tamaño, no puntos.** Para ver cada «tienda» de cerca hacen falta marcadores con geometría
  (disco o columna) que crezcan al alejarse, o agregar de lejos y mostrar individuos al acercar.

`maxMemoryMegabytes` del `<gmp-map-3d>` tiene un máximo de 2 048 (no era la causa de nada).

**T10 · Teselado por nivel de detalle, costa a costa (2026-10-21).** Un árbol de teselas en lat/lng (lado
360/2^z grados), un nivel por resolución H3, el ancla de cada tesela en el centroide de sus celdas
(`t10_lod.py`, `t10_pagina.html`):

| nivel | tesela | celdas | teselas | MB |
|---|---|---|---|---|
| res 3 | z 6 (~5,6°) | 617 | 39 | 5 |
| res 4 | z 7 (~2,8°) | 4 353 | 139 | 11 |
| res 5 | z 9 (~0,7°) | 30 443 | 1 806 | 27 |
| res 6 | z 10 (~0,35°) | 212 990 | 7 058 | 80 |
| res 7 (Nueva York, Denver, Los Ángeles) | z 12 (~0,09°) | 21 265 | 1 728 | 9 |
| individuos: 10 % top como columnas (nivel res 5) · 951 270 tiendas de 400 m (res 6) · 168 466 de 120 m (res 7) | z 9 / 10 / 12 | — | 571 / 7 064 / 1 691 | 35 / 356 / 63 |

El cliente, cada 500 ms, elige el nivel por distancia (que una celda ocupe ≥ 14 px) y **pide sólo las teselas
cuyo ancla cae dentro de la vista**; suelta las demás tras ~2,5 s de solape; cada tesela, un elemento nuevo.

| vista | nivel | modelos vivos | fps (asentado) |
|---|---|---|---|
| país | res 3 | 39 | 60 |
| Rocosas a 700 km | res 4 | 27 | 60 |
| Denver a 150 km | res 6 + tiendas de 400 m | 87 | 60 |
| Nueva York a 40 km · Manhattan a 9 km | res 7 + tiendas de 120 m | 136–140 | 60 |
| Los Ángeles a 60 km · a 15 km | res 6 / res 7 + tiendas | 22–23 | 60 |
| de vuelta al país | res 3 | 39 | 60 |

- **El cambio automático de resolución funciona de costa a costa**: 963 modelos creados y soltados en el
  recorrido, sin degradarse. Mientras Google carga un lote nuevo, el fps cae un instante (medido 1 fps justo
  al llegar a una vista) y vuelve a 60.
- **En cada vista hay del orden de 20–140 modelos**, lo mismo a escala de país que de barrio: el coste lo
  pone la pantalla, no el territorio (lo que buscábamos).
- **Los individuos con tamaño se ven** (columnas de 400 m a media distancia, de 120 m de cerca, el 10 % de
  más ingresos más lejos). Lo que queda: el tamaño debería crecer de forma continua con la distancia, no a
  saltos por nivel, y el clic sobre un individuo (por modelo, no por elemento) se resuelve preguntando al
  servidor.
- En producción las teselas no se precalculan todas (aquí fueron ~20 000 ficheros): se generan bajo demanda y
  se guardan en caché por instantánea, como en E2·2.

**T11 · Hasta dónde llegan los números (2026-10-22).** Dos partes, en este PC de escritorio (el producto
nunca se servirá en móvil), con la ventana visible (oculta, Chrome da ~1 fps falsos) y cada medida
comprobada con capturas: con mucha carga el renderizador de Google se congela mientras
`requestAnimationFrame` sigue a 60, así que «60 fps» sólo vale si la imagen cambia al mover la cámara.

*Parte 1, el techo de la GPU* (`t11_techo.py`, cuadrados planos empaquetados en modelos):

| elementos a la vez | triángulos | geometría | resultado |
|---|---|---|---|
| 5 · 10 · 12 M | 10–24 M | hasta ~1,1 GB | 60 fps, mapa vivo |
| 15 · 20 · 30 M | 30–60 M | 1,3–2,7 GB | el renderizador se congela |

*Parte 2, los 600 M de viajes de SpatialBench sf100* (`previo_e.py`, `previo_f.py`, `t11_planeta.py`,
`t11_planeta.html`). El banco reparte los viajes por todo el planeta; `previo_f` los agrega en un puesto de
victor (lectura de los 600 M en ~38 s, cada resolución en ~45 s) en H3 res 2–5 globales y res 6–7 más los
viajes sueltos en tres zonas densas (África 7–10° N 19–22° E, China 44–46° N 116–118° E, Sinaí 28–30° N
33–36° E: 73,8 M de viajes). El generador local hace el árbol de teselas de T10 con relieve GLO-90 y
envolvente; los viajes, discos de 50 m con un tope de 60 000 por tesela z12 (48,8 M dibujados en 2 641
teselas, 6,7 GB). En total ~66 000 teselas y ~8,3 GB.

| vista | nivel | modelos vivos | fps |
|---|---|---|---|
| el globo | res 2 | 83–148 | 60 |
| África a 2 500 · 700 · 150 km | res 2 · 4 · 5 | 2–4 | 60 |
| África a 40 km | res 7 | 28 | 60 |
| África a 8 km · 3 km | viajes sueltos | 7 | 60 |
| China a 300 km · 7 km | res 5 · viajes | 16 · 10 | 60 |
| Sinaí a 250 km · 6 km | res 5 · viajes | 8 | 60 |

Y forzando viajes sueltos con un radio de carga cada vez mayor:

| viajes cargados | teselas | geometría | resultado |
|---|---|---|---|
| 3,5 M | 59 | ~0,5 GB | 60 fps, vivo |
| 10,7 M | 233 | 1,47 GB | 60 fps, vivo (gira) |
| 18,8 M | 529 | 2,58 GB | 60 fps, vivo (gira) |
| 27,4 M (la zona de África entera) | 1 225 | 3,68 GB | 60 fps, vivo; la mayoría fuera del encuadre |
| ~39 M (África y Sinaí, vistas desde 500 km) | 2 065 | ~5,2 GB | **congelado**: sólo pinta unas manchas |

- **El 3D de Google sirve los 600 M de punta a punta**: del globo a cada viaje, a 60 fps, con 2–150 modelos
  por vista. Lo que no acepta, teselas propias, lo pone el cliente: cada tesela es un `Model3DElement`.
- **El techo, ~12–15 M de elementos a la vez en pantalla y ~20–27 M cargados**, no se alcanza navegando:
  con el nivel de detalle una vista nunca pide más de lo que caben píxeles.
- **La emulación de viewport no sirve para medir aquí:** el lienzo de Google se queda del tamaño real del
  panel. Las medidas son a 420×369 px; a pantalla completa caben más teselas por vista.
- **Deudas visuales:** los viajes salen negros (la rampa de tarifa empieza en el negro de magma) y en relieve
  abrupto (el Sinaí) se entierran: sin envolvente, con el DEM a ~370 m y sólo +25 m. Con la envolvente o
  GLO-30 se arregla. Los hexágonos de res 6–7 saturan el color (se normaliza con la densidad de res 5).
- **Sin probar:** el clic sobre un viaje (el modelo no sabe de elementos; se resuelve preguntando al servidor)
  y el precio y la API en la GA del `<gmp-map-3d>`.

**Lo que E2·0 decide** (aprobado por el usuario el 2026-10-22):

| # | propuesta | por qué |
|---|---|---|
| D-S-6 | **El motor de teselas en Rust**, un servicio propio, puro y con réplicas | 5–9× más teselas por segundo con concurrencia, p95 acotado y < 1 GB de RAM; DuckDB empata en una tesela suelta pero no escala dentro de un proceso |
| D-S-7 | **La geometría llega al scan como la `GEOMETRY` del fichero**, nunca envuelta | sin ello no hay poda (36 ms frente a 720 ms). Lo arregla el motor de teselas leyendo el Parquet, y el SDK deja de envolverla |
| D-S-8 | **Crudo hasta ~50 000 elementos por tesela; agregado por encima, decidido por tesela** con el conteo de la pirámide | 534 KB a z14 en Madrid, 8 MB a z12 |
| D-S-9 | **Para lo grande, la pirámide de agregados como dataset derivado, ordenado por tesela** (leer una tesela = leer un rango). Se usa **por capa y por nivel**: donde una tesela pasa del umbral de D-S-8 sale de la pirámide, donde no, crudo del dataset; lo decide el motor con el conteo por celda, no una persona | 2 min por 600 M; servida como tabla plana sin ordenar no escala; un dataset de 600 M tiene pirámide en los zooms bajos y crudo en los altos (T11), uno de 100 000 nunca la tiene |
| D-S-10 | **Servir de una copia en 3857** cuando la capa no esté en 3857 (o una reproyección más rápida) | la mitad del tiempo en Rust |
| D-S-11 | **El satélite 3D recibe geometría empaquetada en modelos glTF, teselada por nivel de detalle** (no `Polygon3DElement`) | ~300 polígonos sueltos a 60 fps frente a 12 M de elementos empaquetados (T6–T11); los 600 M de punta a punta |
| D-S-12 | **Un motor, dos codificadores: MVT para MapLibre y glb con relieve (3D Tiles) para el satélite de Google**, sobre la misma lectura, la misma pirámide, la misma caché y la misma puerta | lo que cambia entre los dos mapas es sólo el último paso (cómo se escribe la tesela), no cómo se lee, se agrega, se gobierna o se cachea. Ver «Una solución para los dos mapas» |

**Una solución para los dos mapas (D-S-12).** Hasta T6 había dos caminos: teselas para MapLibre y, para
el satélite 3D, un tope de cientos de polígonos. Lo medido en T6–T11 los une: **en los dos mapas una
tesela es un trozo de mapa (zona + nivel de detalle) con lo que hay que pintar ahí; lo que cambia es el
formato del contenido**. El satélite recibe lo mismo que define el estándar OGC 3D Tiles (el de Cesium y
el de las teselas fotorrealistas de la propia Google): un árbol de teselas cuyo contenido es glTF.

```
Iceberg (la verdad: Hilbert, poda por bbox)
  → motor de teselas Rust, puro (D-S-1, D-S-6)
      → crudo bajo demanda (D-S-8) · pirámide H3 por instantánea para lo grande (D-S-9)
      → codificador:   MVT                  |   glb + relieve
                       (MapLibre, plano)    |   (<gmp-map-3d>, satélite 3D)
  → caché de contenido  instantánea / capa / filtro / z / x / y / formato (D-S-4)
  → ore-serve, la puerta (identidad, permisos, URL firmada) → borde
```

| | MVT | glb (3D Tiles) |
|---|---|---|
| geometría | vectorial 2D en coordenadas de tesela | malla en ENU local de la tesela (la Tierra curva), float sin comprimir (Google ignora Draco, meshopt y la cuantización) |
| estilo | lo aplica el navegador | horneado en la tesela (color por vértice): **el estilo entra en la clave de caché** |
| relieve | no hace falta | el motor da a cada vértice la cota del terreno (+ envolvente y margen); el relieve es **un activo del lago** (Copernicus GLO-90/30, gratis, se copia una vez y lo comparten todos los mapas) |
| índice | z/x/y implícito | un árbol (como `tileset.json`) con el ancla de cada tesela; el cliente pide las que caen en vista |
| peso | ~decenas de B por elemento | ~140 B por elemento crudo, ~10× menos con Brotli (T7) |
| clic | el id va en la tesela | Google no expone el elemento dentro del modelo: el clic pregunta al servidor por la posición (`/mapa/fila`), igual para los dos |

Consecuencias: el motor de E2·2 nace con los dos codificadores; la pirámide, la caché, la frescura (0063)
y los permisos (A8) se diseñan una vez; la consola (E2·5) usa el mismo catálogo de capas en los dos mapas.
Riesgos propios del satélite: el `<gmp-map-3d>` sigue en beta, y la *Map Tiles API* de Google (las teselas
3D en bruto) devuelve 403 en el EEE desde julio de 2025; el `<gmp-map-3d>` de Maps JS funciona (T6–T11),
pero hay que confirmarlo por escrito antes de depender de él.

### Los contratos (borrador, 2026-10-22)

**Diseño por contrato primero.** Las capas del mapa masivo (almacenamiento, motor de teselas, puerta, caché y
transporte, cliente; el estado caliente llegará con E3) **sólo se relacionan a través de cuatro contratos de
interfaz**. Cada capa depende de lo que un contrato promete, nunca de cómo está hecha otra por dentro: una
capa se puede rehacer entera (otro codificador, otra caché, otro mapa) sin tocar las demás. Los contratos
son las costuras del sistema; el esqueleto andante (E2·2) es su primera implementación de punta a punta.

| contrato | qué fija | dónde vive | lo produce | lo consume |
|---|---|---|---|---|
| **C1 · el kind `Layer`** | qué es «algo que se pinta» | **OOS** (lo declara el cliente) | el repositorio del cliente | la consola, ore-serve, el motor, el Build de la pirámide |
| **C2 · la dirección de tesela** | el nombre único e inmutable de una tesela y cómo se pide | la API pública de ORE (`/v1`) y `ore-core` | ore-serve | los dos mapas, la caché, el borde |
| **C3 · el protocolo del motor** | lo que ore-serve le pasa al motor y lo que vuelve | protocolo interno de ORE (como 0013 y 0015) y `ore-core` | ore-serve | el motor |
| **C4 · la pirámide de agregados** | el esquema y el orden del dataset derivado | convención de ORE y `ore-core` | el Build | el motor |

**C1 · el kind `Layer`** (diseño a fondo, **aprobado 2026-10-22**; la spec, en OOS v1alpha34).

*Naturaleza.* **Una `Layer` es la forma declarada de pintar un dataset en un mapa**: qué elementos, qué
propiedades viajan con ellos, qué filtro fijo, qué se agrega donde no cabe el crudo, qué dicen sus etiquetas y
cómo se pinta. No guarda datos: lo que se deriva de ella (la pirámide y el índice de etiquetas, C4) es un
dataset, y sus teselas, caché.

*Qué no es.*
- **No es un dataset** ni una `View`: no tiene filas y no se consulta como tabla; se pinta.
- **No es un mapa**: un mapa junta capas, una cámara y un fondo. Guardar mapas (un kind `Map`) queda fuera de
  la v1; en la consola, el mapa es la sesión.
- **No es la preferencia de quien mira**: el filtro del deslizador o un color probado en la consola viajan en la
  petición (K2), no se guardan en la `Layer`.

*Anatomía.*

```yaml
apiVersion: oos.dev/v1alpha34
kind: Layer
metadata:
  name: tiendas
  namespace: retail                     # el paquete; vive en la carpeta de su schema, como todo
  description: Los locales de la cadena en América.
spec:
  owner: team:expansion                 # opcional (v1alpha21)
  source:
    dataset: retail.locales             # un Dataset del lago (K3: uno)
    geometry: ubicacion                 # Geometry/Geography; opcional si el dataset tiene una sola
    id: local_id                        # opcional si el dataset declara clave
  properties: [nombre, ventas, formato] # lo que viaja a la tesela; sin lista, sólo geometría e id
  filter: "estado = 'abierto'"          # opcional; WHERE acotado (K2)
  aggregate:                            # lo que guarda la pirámide donde no cabe el crudo
    metrics: [sum(ventas), avg(ventas)] # count siempre está
  labels:                               # lo que guarda el índice de etiquetas (C4)
    text: ["concat", ["get", "nombre"], " · ", ["get", "ventas"]]
    priority: ["get", "ventas"]         # opcional; sin ella, densidad y desempate (C4)
    mode: thin                          # thin (aclarar) | cluster (agrupar)
  styles:                               # la MapLibre Style Spec (K1); el primero es el de serie
    - name: ventas
      layers:
        - type: circle
          paint:
            circle-color: ["interpolate", ["linear"], ["get", "ventas"], 0, "#2a9d8f", 5000000, "#e76f51"]
            circle-radius: 6
        - type: symbol                  # las etiquetas de labels; aquí sólo su aspecto
          paint: {text-color: "#ffffff"}
    - name: densidad
      layers:
        - type: fill-extrusion          # de lejos, las celdas agregadas
          filter: ["get", "ore:agregado"]
          paint:
            fill-extrusion-color: ["interpolate", ["linear"], ["get", "count"], 1, "#fde725", 500, "#440154"]
            fill-extrusion-height: ["*", ["get", "count"], 40]
```

*Las claves.*

| clave | | |
|---|---|---|
| `metadata.name`, `namespace`, `description` | como todo kind | el nombre es `<base>.<nombre>` (v1alpha13) |
| `spec.owner` | opcional | v1alpha21; sin él, el del schema o el paquete |
| `spec.source.dataset` | **obligatoria** | un `Dataset` (escrito o mantenido) del lago. Una `View` o una `Table` sin copia, todavía no: el motor lee Iceberg (D-S-6) |
| `spec.source.geometry` | condicional | una columna `Geometry<…>` o `Geography<…>` (v1alpha32); obligatoria si el dataset tiene más de una |
| `spec.source.id` | condicional | la columna que identifica cada elemento (clic, etiquetas, desempate); obligatoria si el dataset no declara clave |
| `spec.properties` | opcional | **lo que viaja, y nada más**: sin lista, sólo la geometría y el id. Menos exposición por omisión |
| `spec.filter` | opcional | `WHERE` acotado (K2); puede usar columnas que no viajan: se aplica en el servidor |
| `spec.aggregate.metrics` | opcional | `sum`, `avg`, `min`, `max` de columnas numéricas; `count` siempre. Celda H3 (D-S-9) |
| `spec.labels` | opcional | `text` y `priority` como expresiones de la Style Spec sobre `properties`; `mode` `thin` o `cluster` (C4) |
| `spec.styles` | **obligatoria**, ≥ 1 | cada uno, `name` y `layers`: capas de la Style Spec **sin** `source` ni `source-layer` (están implícitas). Tipos: `fill`, `line`, `circle`, `fill-extrusion`, `heatmap`, `symbol` |

*Las reglas del documento* (los números los asigna la spec):

- `source.dataset` no resuelve a un `Dataset` → error de referencia (familia `OOS2xxx`).
- Una columna de `geometry`, `id`, `properties`, `filter`, `metrics` o `labels` que el dataset no tiene →
  `OOS2xxx`. Una `geometry` que no es geo, una métrica sobre una columna no numérica, un `filter` que no es
  booleano o se sale del subconjunto → `OOS3xxx`.
- **Un estilo o una etiqueta sólo puede leer lo que viaja**: `["get", x]` con `x` fuera de `properties`, de las
  métricas o de las reservadas → `OOS2xxx`. Sin esta regla, el estilo fallaría en silencio en el mapa.
- Una capa de estilo que no valida contra la MapLibre Style Spec, o un `symbol` sin `labels` → `OOS1xxx`.
- **Propiedades reservadas** en cada elemento: `ore:agregado` (si viene de la pirámide), `count` y las métricas
  en los agregados; `ore:` es del sistema y una columna no puede llamarse así.

*Gobierno.*
- **Las etiquetas de confidencialidad bajan**: una `Layer` lleva las de lo que expone (`properties`, geometría,
  id), como una vista; un `filter` sobre una columna etiquetada responde a `OOS4016`, como ya hoy.
- **Leer una capa nunca da más que leer el dataset**: pedir una tesela exige poder leer sus columnas expuestas
  (hasta A8, el dataset entero).
- Vive como todo documento: **propuestas, versión** (su huella es el `{versión}` de C2), **dueño** (0052) y
  **salud** (0063): la de su dataset y la del Build de su pirámide.

*Cómo se llega a una `Layer`* (decidido 2026-10-22). Es un activo del catálogo: vive en su schema, con dueño,
versión, salud y linaje (`dataset → Layer → pirámide`), y se abre en el mapa. **Tres caminos, el mismo documento
y siempre una propuesta**, todos en la v1:
1. **Desde el mapa de la consola** (el principal, E2·5): se elige qué enseñar, cómo pintarlo, etiquetas y filtro,
   y «Guardar como capa» genera el documento.
2. **El YAML en el repositorio**: el formato canónico, el que se revisa.
3. **`CREATE OR REPLACE LAYER`** en el paradigma `.sql`: fuente, `GEOMETRY`, `PROPERTIES`, `WHERE`, `AGGREGATE`;
   el estilo no es SQL, así que nace con uno de serie y se afina en el mapa o en el YAML.

*Lo que no es de la spec, sino de ORE.* Qué partes del estilo pinta cada mapa es capacidad de la implementación,
no gramática: ORE lo dice en el descriptor de C2 (`unsupported`, por mapa) y como nota al validar («no llega al
satélite: `line-dasharray`»). Así la spec no cambia cuando el motor aprende a hornear algo más.

*Se construye en* (el plan): F2·2 · F4·1 · F5·1, F5·2, F5·4 · F6.

*Fuera de la v1.* Varias fuentes (K3), el tiempo (E3), las capas ráster (E4), el kind `Map` y un relieve propio
por capa.

*Aprobados por el usuario el 2026-10-22:*

| # | punto | propuesta |
|---|---|---|
| C1-a | **El texto y la prioridad de las etiquetas, en `labels` de la `Layer`, no en cada estilo** (afina K1) | el índice de etiquetas (C4) es uno por capa; si cada estilo tuviera su texto, habría un índice por estilo. El estilo decide sólo su aspecto |
| C1-b | **Nada viaja si no se declara** (`properties`) | exposición mínima por omisión; el estilo sólo lee lo que viaja |
| C1-c | **Fuente: sólo `Dataset` en la v1** | el motor lee Iceberg; una `View` exigiría calcular al vuelo |
| C1-d | **Crudo y agregado en el mismo estilo**, con `ore:agregado` para distinguirlos | una capa se ve continua al hacer zoom aunque cambie de fuente (D-S-9) |
| C1-e | **Lo que cada mapa no pinta lo dice ORE**, no la spec | la spec no se mueve con el motor |

**C2 · la dirección de tesela** (diseño de producto, **aprobado 2026-10-22**).

*Naturaleza.* **Cómo un mapa pide lo que pinta.** Dos niveles, como TileJSON y 3D Tiles: **el descriptor**
(corto, autorizado, dice qué ve quien mira y dónde está) y **la tesela** (inmutable, cacheable en cualquier
sitio). Todo lo que depende de la persona se resuelve una vez en el descriptor; lo que viaja después ya no
depende de ella.

*Los recursos* (API pública de ORE, `/v1`, servida por ore-serve):

| recurso | qué devuelve | cacheable |
|---|---|---|
| `GET /v1/layers` | las capas que quien mira puede abrir (para el selector del mapa) | no |
| `GET /v1/layers/{layer}/descriptor` | **el descriptor**: dónde están las teselas y cómo pintarlas | unos minutos, por persona |
| `GET /v1/tiles/{layer}/{clave}/{tms}/{z}/{x}/{y}.{mvt\|glb}` | **la tesela** | **para siempre** |
| `GET /v1/labels/{layer}/{clave}/{tms}/{z}/{x}/{y}.json` | las etiquetas de una tesela, para el satélite | para siempre |
| `GET /v1/layers/{layer}/features?at=lng,lat&z=…` (o `?ids=…`) | **el clic**: la ficha del elemento | no |

*El descriptor.* Se pide con el mapa (`map=maplibre|satellite`), el estilo (`style=`, si no el de serie), el
filtro de quien mira (`filter=`, K2) y, opcionalmente, la rama o la propuesta (`branch=`) o una instantánea
(`at=`). ore-serve autoriza, resuelve la instantánea y la clase de visibilidad (`vis`), y devuelve:

- **Para MapLibre, un TileJSON 3.0** (plantilla de teselas, zooms, límites, campos de cada `source-layer`) más el
  estilo ya resuelto como capas de la Style Spec atadas a esa fuente: **MapLibre lo carga tal cual**, sin código
  propio en la consola.
- **Para el satélite, un árbol 3D Tiles 1.1 implícito** (cuádrico en `WorldCRS84Quad`, con la disponibilidad:
  qué teselas tienen datos), en lugar del `arbol.json` de T10–T11. Es el estándar de OGC: lo que el cliente pide,
  un visor 3D Tiles cualquiera también lo entendería.
- Común a los dos: la **instantánea** y su `dataAsOf`, la **salud** de la capa (si está `degraded`, el mapa lo
  dice), **lo que ese mapa no pinta** (`unsupported`, C1-e), la **atribución** («© Copernicus DEM» en el
  satélite), la **caducidad** del descriptor y la **firma** de las teselas.

*La clave.* `{clave}` es **una huella opaca** de todo lo que decide los bytes: la versión de la `Layer`, la
instantánea, `vis`, el estilo (sólo glb: en MVT el estilo es del navegador), el filtro de quien mira y la versión
del relieve (sólo glb). Opaca a propósito: **nadie fabrica una dirección sin pasar por el descriptor**, la URL es
corta y no enseña nada. ore-serve guarda `clave → parámetros` (contenido direccionado: la misma combinación da
siempre la misma clave, y dos personas con la misma visibilidad, estilo y filtro comparten teselas y caché).

*La firma.* Las teselas van firmadas con vida corta (un parámetro aparte, **fuera de la clave de caché**): el
borde la comprueba y cachea por la ruta, así que la caché se comparte sin que una URL filtrada sirva para
siempre. La firma vale por `clave`, no por persona.

*Las etiquetas.* En MapLibre van **dentro de la tesela MVT** (un `source-layer` de etiquetas con su `minzoom`):
MapLibre hace la colisión, como siempre. En el satélite van en **teselas de etiquetas** JSON (posición, texto,
prioridad), cacheables como las otras; el cliente junta las de las teselas a la vista, aplica el tope (~300, T12)
y renueva por diferencia.

*El clic.* Devuelve **las columnas de la fila que quien mira puede leer**, no sólo las `properties` de la capa:
lo que viaja a la tesela es lo mínimo para pintar; la ficha es la fila, con sus permisos. Igual en los dos mapas.

*La frescura.* La tesela nunca cambia; **lo que cambia es el descriptor**. Cuando hay instantánea nueva, el
descriptor da otra `clave` y el mapa pide teselas nuevas; las viejas siguen valiendo hasta caducar en la caché.
El mapa vuelve a pedir el descriptor al caducar y cuando el catálogo avisa de un cambio (lo vivo, E3, empujará en
lugar de esperar). **Una rama o una propuesta se ven en el mapa antes de fusionar**, con `branch=`.

*Las respuestas.*

| caso | respuesta |
|---|---|
| tesela sin elementos | `204`, sin cuerpo (MapLibre y el árbol lo esperan) |
| firma caducada o que no casa | `403`: el mapa pide el descriptor otra vez |
| `clave` desconocida o instantánea ya expirada | `410`: ídem |
| tesela fuera de los límites o del árbol | `404` |
| la capa o la fuente `degraded` | se sirve, y el descriptor lo dice (0063): el mapa no miente |

*La promesa.* **La misma dirección devuelve siempre los mismos bytes** (D-S-4): el motor es determinista (orden
estable, muestreos con semilla fija). Sin eso, la caché sería una mentira.

*Aprobados por el usuario el 2026-10-22:*

| # | punto | propuesta |
|---|---|---|
| C2-a | **Clave opaca** en lugar de los parámetros en la ruta | nadie fabrica direcciones; URL corta; comparte caché entre iguales |
| C2-b | **TileJSON 3.0 para MapLibre y 3D Tiles 1.1 implícito para el satélite** | los dos estándares; sustituye a nuestro `arbol.json` |
| C2-c | **Firma aparte de la clave**, por `clave` y de vida corta | caché compartida en el borde sin URLs eternas |
| C2-d | **Etiquetas dentro del MVT y en teselas de etiquetas para el satélite** | MapLibre colisiona solo; el satélite junta y pone el tope |
| C2-e | **El clic devuelve la fila legible**, no sólo `properties` | pintar con lo mínimo, consultar con permisos |
| C2-f | **Frescura por el descriptor** y vista previa de ramas con `branch=` | la tesela no se invalida nunca; las propuestas se ven en el mapa |

*Se construye en* (el plan): F2·4 · F3·2 · F5·1–F5·3, F5·5. Se comprueba en F2·6 y se congela en F4·5.

**C3 · el protocolo del motor** (diseño de producto, **aprobado 2026-10-22**).

*Naturaleza.* **El contrato interno entre la puerta y el motor: ore-serve decide quién y qué; el motor calcula
cómo.** El motor es una función pura (D-S-1): recibe una tesela con todo resuelto y devuelve sus bytes. No conoce
personas, catálogo ni políticas; no puede saltarse un permiso porque nunca lo decide. Por eso escala en réplicas,
se cachea por la clave y un fallo suyo no filtra nada.

*La petición*, lo que ore-serve le manda:

| parte | qué | por qué así |
|---|---|---|
| la tesela | `tms`, `z/x/y`, formato (`mvt`, `glb` o etiquetas) y la `clave` de C2 | la clave le sirve de caché propia y de rastro |
| la fuente | la instantánea del dataset (su `metadata.json`), la geometría y el id; la de la pirámide si existe (C4); la versión del relieve (glb) | lee lo que le dicen, en la instantánea que le dicen: determinista |
| lo permitido | **el predicado de filas ya compuesto** (visibilidad ∧ `filter` de la capa ∧ filtro de quien mira), **analizado y en forma estructurada, nunca texto SQL**; las columnas permitidas (`properties` ∩ máscara) | el motor no analiza ni concatena SQL: no hay inyección posible y no puede «olvidar» la visibilidad |
| cómo pintar | el estilo resuelto y la configuración de etiquetas (sólo glb y etiquetas) | el horneado es del motor; la elección, de la `Layer` |
| los límites | el umbral crudo/agregado (D-S-8), el tiempo límite, el tamaño máximo | ore-serve fija el presupuesto de cada petición |
| la credencial | prestada, de vida corta, **de sólo lectura y recortada a los ficheros de esa instantánea** | aunque el motor fallara, no puede leer otra cosa |

*La respuesta*, lo que el motor devuelve:

- **Los bytes** con su tipo, o **vacío** explícito (ore-serve lo convierte en `204`).
- **La cuenta**: elementos, origen (crudo, agregado al vuelo o pirámide), grupos de filas leídos y descartados,
  bytes leídos, milisegundos por fase. Alimenta las métricas, la salud (0063) y las medidas de la vara.
- **Los fallos, con tipo:** fuente expirada (→ `410` en C2), tiempo agotado (→ `503`, reintentable), petición
  inválida o de otra versión del protocolo (→ error interno, nunca a la persona).

*La regla de la densidad: el motor nunca muestrea.* Si una tesela pasa del umbral y no hay pirámide (todavía, o
para ese nivel), **agrega al vuelo** en H3 desde el crudo. Si no le da tiempo, falla con `503` y el descriptor
marca la capa como «construyendo»: el mapa espera o enseña menos detalle, **nunca una muestra que parezca el
todo**. «El mapa nunca miente» (§2).
*(Precisado el 2026-10-22 en «El alcance v1»: con pirámide, pasarse de plazo baja un nivel de agregado, nunca `503`;
una `Layer` no se sirve hasta tener su primera pareja completa.)*

*Las garantías del motor.*
- **Puro:** sólo habla con el lago (la instantánea que le dicen) y el bucket del relieve; sólo acepta a ore-serve.
- **Sin estado:** cualquier réplica contesta cualquier petición.
- **Determinista:** la misma petición da los mismos bytes (orden estable, sin azar sin semilla): lo exige C2.
- **Acotado:** tiempo, memoria y concurrencia por réplica con tope; una tesela cara no tumba a las demás.
- **Observable:** la cuenta de cada tesela, siempre.

*La caché, por capas*, todas con la clave de C2: **el borde** (F5·5) → **ore-serve** → **el motor**, que además
guarda lo que reutilizan todas las teselas: los pies de los Parquet, los bloques del relieve.

*Versiones.* La petición lleva la versión del protocolo; el motor acepta la suya y la anterior, así que
ore-serve y el motor se despliegan por separado. Transporte interno por HTTP, como ore-motor.

*Aprobados por el usuario el 2026-10-22:*

| # | punto | propuesta |
|---|---|---|
| C3-a | **El predicado viaja analizado y estructurado**, nunca como texto SQL | sin inyección; el motor no puede omitir la visibilidad |
| C3-b | **El motor nunca muestrea**: agrega al vuelo o falla con `503` y «construyendo» | el mapa no miente |
| C3-c | **Credencial recortada a la instantánea** | un motor comprometido no lee otra cosa |
| C3-d | **Determinista y sin estado** | lo exige la caché de C2 y escalar en réplicas |
| C3-e | **Versión del protocolo N y N−1** | ore-serve y el motor se despliegan por separado |

*Se construye en* (el plan): F2·3, donde se define a fondo · F3·2–F3·3 · F4·1 · F5·1–F5·2.

*Cómo se construye* (lo fácil primero, sobre lo que ya hay):
1. **Los tipos de la petición y la respuesta en `ore-core`**, compartidos por ore-serve y el motor: un solo
   sitio donde vive el contrato, y el compilador comprueba los dos lados.
2. **El motor como binario nuevo, `ore-teselas`**, a partir del prototipo `e2/teselas` (T1): ya lee Parquet por
   grupos y codifica MVT; se le añade leer la instantánea de Iceberg y aplicar el predicado estructurado.
3. **El cliente en ore-serve**, como el que ya llama a ore-motor: arma la petición, presta la credencial y
   traduce los fallos a C2.
4. **En la malla de victor, como ore-motor:** un Deployment con réplicas, un Service y una NetworkPolicy que sólo
   deja entrar a ore-serve.
5. **Medido en victor** con la vara: p95 por tesela en frío y en caché, RAM, y 50 a la vez.

**C4 · la pirámide y el índice de etiquetas** (diseño de producto, **aprobado 2026-10-22**).

*Naturaleza.* **Lo que una `Layer` necesita precalculado para servirse a escala, guardado como datasets
derivados de ella.** Tres piezas, las tres dato (D-S-5), construidas por el sistema desde una instantánea del
dataset, con linaje a ella y a la versión de la `Layer`:

| pieza | qué guarda | para qué |
|---|---|---|
| **el mapa de densidad** | cuántos elementos hay en cada tesela de cada nivel, en las dos rejillas (`WebMercatorQuad`, `WorldCRS84Quad`) | que el motor decida crudo o agregado sin leer nada más (D-S-8), y la disponibilidad del árbol 3D Tiles (C2) |
| **la pirámide** | por celda H3 y resolución: `count` y las métricas de `aggregate` | servir lo denso (D-S-9) |
| **el índice de etiquetas** | por elemento: id, posición, texto resuelto, prioridad y su nivel de aparición en cada rejilla | elegir las etiquetas de cada vista (C1 `labels`) |

Es pequeño frente al dataset: para los 600 M, la pirámide son ~1,2 M de filas (T11) y el mapa de densidad, del
orden de un millón; se construyen en minutos (T3: 2 min).

*Cómo se guarda.* **Como el resto del lago: Iceberg, ordenado por Hilbert y con la caja de cada grupo de filas**,
partido por resolución (la pirámide) o por nivel (densidad y etiquetas). **Sin claves de tesela**: leer una tesela
es descartar por caja los grupos que no la tocan, lo mismo que con el crudo (D-S-7, T2). Así **una sola pirámide
sirve a las dos rejillas**; el borrador anterior (columnas `tms/z/x/y`) obligaba a una por rejilla.

*Qué se construye.* Sólo lo que hace falta:
- **El mapa de densidad, siempre** (es barato, y el descriptor lo necesita).
- **La pirámide, sólo si alguna tesela pasa del umbral** (D-S-8), y sólo hasta la resolución en que el crudo ya
  cabe en todas: una capa de 100 000 elementos no tiene; una de 600 M la tiene en los zooms bajos (T11).
- **El índice de etiquetas, si la `Layer` declara `labels`.** Con `mode: cluster`, los grupos salen de la pirámide
  (su texto, la cuenta: «812 locales»; ponerles el nombre de la región espera a los datos de referencia, E5).

*Cuándo.* Lo construye un **Build del sistema** (0055), no código del cliente, al haber **instantánea nueva del
dataset** o **versión nueva de la `Layer`** que toque lo precalculado (`filter`, `aggregate`, `labels`); un
cambio sólo de estilo no reconstruye nada. Mientras se construye, **el descriptor sigue sirviendo la pareja
anterior completa** (instantánea del dataset + su pirámide): nunca mezcla una instantánea con la pirámide de otra.
Si va por detrás del dataset, la salud lo dice (`behind-source`, 0063). Reconstruir sólo lo tocado (el changelog
de Iceberg), después.

*Dónde se ve.* Son datasets: aparecen en el linaje de la `Layer` y se pueden consultar (una pirámide de ventas por
celda H3 es un dato útil), con el mismo dueño y la misma visibilidad que la capa. En la lista del catálogo, plegados
bajo su `Layer`.

*Los dos límites, nombrados.*
- **El filtro de quien mira no tiene pirámide**: la pirámide es de la `Layer` con su `filter`; con un deslizador
  encima, el motor agrega al vuelo (C3-b). Para capas enormes con filtros interactivos frecuentes, la salida será
  una pirámide con dimensiones (agregados también por una categoría o por tiempo): después, si se mide que hace
  falta.
- **La visibilidad (A8)**: un agregado sobre todas las filas revela lo que alguien no puede ver. Hasta A8, una
  `Layer` con pirámide sólo se sirve a quien lee el dataset entero; después, la pirámide será por clase de `vis`.

*El nivel de aparición de las etiquetas*, como se decidió: el punto nunca se esconde, sólo su texto; por densidad,
de lejos a cerca, una rejilla del tamaño de una etiqueta en pantalla; monótono (lo que aparece no desaparece al
acercarse); desempate determinista (prioridad, cercanía al centro de la casilla, id); por construcción, ninguna
vista pasa del tope (~300, T12). Se calcula **para cada rejilla**, porque el tamaño de la casilla en el suelo
depende de la proyección.

*Aprobados por el usuario el 2026-10-22:*

| # | punto | propuesta |
|---|---|---|
| C4-a | **Tres piezas: densidad, pirámide, etiquetas**, las tres datasets derivados (*revisado 2026-10-25, D-F2-15*: la pirámide es de la `Layer`, no un dataset) | la densidad decide crudo/agregado y la disponibilidad del árbol sin leer el crudo |
| C4-b | **Ordenadas por Hilbert con caja por grupo, sin claves de tesela** (cambia el borrador) | una pirámide para las dos rejillas, la misma poda que el crudo |
| C4-c | **Sólo se construye lo necesario**, por un Build del sistema al cambiar la instantánea o lo precalculado de la `Layer` | una capa pequeña no paga nada; el estilo no reconstruye |
| C4-d | **El descriptor sirve siempre una pareja completa** (instantánea + su pirámide) | el mapa nunca mezcla versiones |
| C4-e | **Visibles en el linaje y consultables**, plegados bajo su `Layer` (*revisado 2026-10-25, D-F2-15*: la pirámide se ve en la página de su capa y no tiene linaje propio) | son dato; no esconderlos |
| C4-f | **Los dos límites**: el filtro de quien mira agrega al vuelo; con pirámide, sólo quien lee todo hasta A8 | honestos; con salida nombrada |

*Se construye en* (el plan): F4·0–F4·2, F4·5 · F5·2. Lo que queda fuera, en «El alcance v1».

**El relieve, lo que sostiene a C3** (decidido 2026-10-22). El glb lleva la cota del terreno en cada vértice
(T9–T11). El relieve **no es dato del cliente: es infraestructura de la plataforma**, como las fuentes con que se
dibujan las etiquetas. No entra en el catálogo, no tiene tipo en OOS, y el cliente ni lo ve ni lo toca.

- **Dónde:** un bucket de la plataforma, compartido por los inquilinos y de sólo lectura para el motor:
  `relieve/copernicus-glo90/<versión>/` con los **COG tal cual** (se leen por ventana y en sus versiones
  reducidas, como en T9–T11) y un índice interno (tesela, extensión, resolución, sin-dato).
- **Quién:** ORE, con un trabajo interno por versión de Copernicus: la copia y **la envolvente** (el máximo por
  nivel, T9) como derivado, para que el motor no la recalcule por tesela. GLO-90 entero ahora; GLO-30 después
  (arregla los viajes enterrados de T11 en vistas cercanas).
- **En el motor:** lector de COG en Rust por rangos de bytes, con caché en memoria de los bloques más pedidos.
- **En la dirección (C2):** la versión del relieve entra en la de las teselas glb; cambiarla no invalida nada.
- **Lo único que ve el cliente:** la atribución, «© Copernicus DEM», en el descriptor de C2.
- **Más adelante:** un relieve propio del cliente (un LiDAR de su terreno) sí sería dato suyo, de su catálogo, y
  la `Layer` podría declararlo en lugar del de la plataforma.

**Las preguntas de los contratos** (2026-10-22):

| # | pregunta | estado |
|---|---|---|
| K1 | ¿El lenguaje de estilo? | **decidida: la MapLibre Style Spec**, sin lenguaje propio. En plano la interpreta MapLibre; en el satélite el motor **la hornea en el glb** con un evaluador de expresiones en Rust contrastado con las pruebas oficiales de la spec: `fill`, `line`, `circle`, `fill-extrusion` por elemento; lo que depende del zoom, por nivel; `heatmap`, como ráster horneado sobre una malla con relieve. **`symbol` (etiquetas e iconos) no se hornea ni se pega al suelo** (decidido): el servidor elige las de cada vista (texto, icono, posición, prioridad, sin solaparse) y el satélite las dibuja como `Marker3DElement`; se acepta un tope del orden de **300 por vista** hasta medirlo (T12). Lo que el satélite aún no pinta lo avisa la validación de la `Layer` (diagnóstico), nunca desaparece en silencio. Lo que el satélite hace distinto: tamaños en metros por nivel (no en píxeles), estilo nuevo = teselas nuevas, resalte por consulta al servidor, transparencia sólo en capas pequeñas |
| K2 | ¿La sintaxis de `filter`? | **decidida: el `WHERE` de SQL, acotado** (comparaciones, `IN`, `BETWEEN`, `AND`/`OR`, `IS NULL`; sin subconsultas ni funciones arbitrarias) para empujarlo a la lectura y sumarle con garantías el de visibilidad. **Dos filtros:** el de la `Layer` (fijo) y el de quien mira (la interfaz lo escribe al mover un deslizador): éste viaja en la petición y su huella entra en la dirección (C2), que sigue inmutable |
| K3 | ¿Varias fuentes en una `Layer`? | **decidida: no en la v1**; dos datasets, dos capas |
| K4 | ¿La rejilla del satélite? | **decidida: `WorldCRS84Quad`**, ajustando la de T10 |

**T12 · las etiquetas en el satélite (medido 2026-10-22,** `t12_fondo.py`, `t12_etiquetas.html`**).** Marcadores
`Marker3DElement` con etiqueta e icono sobre una capa de 5 M de cuadrados (60 fps sola), en este PC de escritorio,
ventana visible, panel de 529×517 px; cada tanda comprobada con captura.

| marcadores | alta | fps quieto | fps en vuelo |
|---|---|---|---|
| 100 | 26–290 ms | 60 | 60 (40 en la primera, en frío) |
| 300 | 49 ms | 60 | 60 |
| 1 000 | 218 ms | 60 | 44 |
| 3 000 | 688 ms | 38 | 18 |

| renovación | coste por vez | fps |
|---|---|---|
| todos (300) cada 500 ms | 81 ms | 25; las etiquetas no llegan a pintarse |
| todos (1 000) cada 1 s | 372 ms | 11 |
| **el 10 % (30 de 300) cada 1 s** | 11 ms | **60** |
| el 10 % (100 de 1 000) cada 1 s | 29 ms | 55 |

- **El tope de K1 se confirma con holgura: 300 por vista, a 60 fps quieto y en vuelo**; hasta ~1 000 vale quieto.
- **Google no evita el solape de las etiquetas**: la selección sin solapes por vista la hace el servidor (como decía
  K1), no es opcional.
- **Renovar es por diferencia**: el cliente sólo quita y pone lo que entra y sale de la vista (11 ms por segundo
  para 30); sustituirlas todas de golpe hunde el mapa.

### El plan: construir definiendo (2026-10-22)

**El enfoque.** Se construye **por fases que atraviesan todas las capas**, no capa a capa: cada fase deja algo
que se ve en un mapa de victor y se mide contra la vara (§2). Los contratos se **definen a fondo justo antes de
su primer uso real** (C3 en la fase 2, C4 en la 4) y **se congelan en dos pasadas de coherencia**, cuando lo
construido ya ha dicho la verdad. Cada fase, como siempre: un previo que despeja lo desconocido, la spec en OOS
primero si la toca, pasos en local con commit en la rama, medido en victor, un empuje y el ADR al cerrar.

| fase | qué deja hecho | contratos | capas que toca |
|---|---|---|---|
| **F1 · el sustrato** | la poda del lago funciona por el SDK | — | almacenamiento |
| **F2 · el esqueleto andante** | **una `Layer` real del catálogo, pintada en MapLibre** | C1, C2, C3 finos | todas |
| **F3 · el satélite por el mismo camino** | **la misma `Layer` en los dos mapas** | glb en C2 y C3 | motor, cliente, relieve |
| **F4 · la escala** | **los 600 M de SpatialBench como capa del catálogo**, en los dos mapas | C4 | almacenamiento (Build), motor |
| **F5 · completar los contratos** | filtros, etiquetas, clic, SQL, firma y borde, ramas | C1, C2 enteros | puerta, caché, cliente |
| **F6 · la consola** | crear y estilar capas desde el mapa | C1 (camino 1) | cliente |

**Los dos ejes.** C1–C4 son **el qué** (los contratos); F1–F6 son **el cuándo**: **un solo plan**, y cada fase
construye un trozo de varios contratos y de varias capas a la vez. Una fase es una columna; lo que cada contrato
dice en «Se construye en» es su fila. **Una sola numeración, la de las fases.**

| | F1 sustrato | F2 esqueleto | F3 satélite | F4 escala | F5 completar | F6 consola |
|---|---|---|---|---|---|---|
| **C1 · `Layer`** | | fina | | `aggregate` | `filter`, `labels`, SQL | «Guardar como capa» |
| **C2 · dirección** | | fina (TileJSON, clave) | 3D Tiles, glb | | filtro, clic, firma, ramas | |
| **C3 · motor** | | a fondo; crudo MVT | glb, relieve, estilo | pirámide, degradación | filtro, etiquetas | |
| **C4 · precalculado** | | | | densidad, pirámide, disparo | etiquetas | |
| **las capas (v1)** | el lago | motor, puerta, cliente | relieve, cliente | lago a escala, réplicas | borde, GLO-30, `vis` | cliente |

**F1 · el sustrato** (independiente: puede ir a la vez que F2·0)

*F1·0, el previo, medido en victor el 2026-10-22* (`previo_g.py`, `previo_h.py`):

| incógnita | lo medido | la decisión |
|---|---|---|
| **leer por el SDK sin perder la poda** (20 M de puntos, tesela z14 de Madrid) | hoy, `iceberg_scan` + envoltorio: **~660 ms en cualquier tesela** (lee todo); sin el CRS o con la caja empujada antes, igual (es la función, no el *cast*). **Columnas de cobertura** filtradas dentro de `iceberg_scan`: **~126 ms**, igual que la geometría nativa del Parquet (~115 ms) y conservando el envoltorio (las `ST_*` y GeoArrow) | **la poda va por columnas de cobertura** (aprobado por el usuario) |
| **de dónde sale el fallo del `append`** | DuckDB entrega la geometría como `binary` y la tabla la guarda como `LargeBinary`; los textos pasan | se arregla en `ore-store` (`Binary → LargeBinary` no pierde nada): vale para cualquier escritor |
| **cómo escribe Spark** (20 M de puntos con cobertura, catálogo Iceberg local en un nodo pesado) | sin orden (hoy): Madrid z14 lee **24 de 64 grupos, 11,6 M de filas**; Hilbert con grupos de ~100 000 filas: **2 de 210, 199 000 filas** (58× menos), escribir pasa de 6 a 20 s; Hilbert con el grupo de serie (128 MB): 5 de 16, 6,2 M; `zorder` de Iceberg después: 10 de 203, 991 000, y 38 s | **Spark escribe cobertura + orden Hilbert + grupos de ~100 000 filas**, como `ore-store` |

- Spark guarda la geometría como `binary` corriente, sin la caja por grupo que deja `ore-store` (Iceberg Java aún no
  escribe el tipo `geometry`): **ordenar no basta, hace falta algo con estadísticas**, y eso son las columnas de
  cobertura. Por eso son la vía común: el SDK, Spark y, en F4·3, el manifiesto (cotas por fichero).
- **La forma de la cobertura**, propuesta: la de **GeoParquet 1.1** (*covering*): una columna `<geom>_bbox` de tipo
  estructura `{xmin, ymin, xmax, ymax}` por cada columna geo, declarada en los metadatos del fichero. Es el estándar;
  F1·1 comprueba que DuckDB empuja el filtro a los campos de la estructura (si no, cuatro columnas planas).

*Los pasos:*
- F1·1 **`ore-store`:** escribe la cobertura de cada columna geo y acepta `Binary → LargeBinary` en `sin_perdida`.
  **El SDK** filtra por la cobertura en sus ayudas espaciales (`ore.geo.*`) antes del envoltorio, y el motor (C3) la
  usa siempre. Hecho cuando la tesela de Madrid por el SDK baja de ~660 a ~130 ms, un `append` geo funciona y la
  suite del SDK pasa entera dentro de la imagen.
- F1·2 **Spark** (`ore/spark.py`): la misma cobertura, el orden Hilbert (la misma clave que `ore-store`) y grupos de
  ~100 000 filas al escribir. Hecho cuando un dataset escrito desde Spark se poda como uno de `ore-store`.
- F1·3 **Medido** en victor sobre los edificios del banco, escritos desde el SDK y desde Spark.

*F1·3, medido en victor el 2026-10-22* (`e2/f1_3.py`; 1,33 M de edificios de Madrid, EPSG:4326, el 90 % sobrescrito
y el resto anexado, por `ore.write` al lago y por `ore.spark.write` a un catálogo local del pod):

| | grupos | Madrid z14: grupos (filas) leídos | con cobertura | sin ella | vacía z14 |
|---|---|---|---|---|---|
| **SDK** (`ore-store`, 100 000 filas por grupo) | 14 | 4 (400 000) | **107 ms** | 187 ms | 77 frente a 198 ms |
| **Spark** (~4 MB por grupo: ~27 000 polígonos) | 48 | 6 (152 000) | 13 ms (disco local) | 99 ms | 3 frente a 101 ms |

- **La poda funciona venga de quien venga**, por la misma `geom_bbox` y la misma consulta del SDK; el `append` geo
  funciona en las dos vías.
- **El grupo por bytes poda mejor que por filas en polígonos** (2,6× menos filas leídas): `ore-store` corta en
  100 000 filas sea cual sea el tamaño; para puntos da igual, para polígonos sobra. Va con F2·3 (D-F2-7).
- Lo que salió al medir, arreglado en `ore/spark.py`: un `append` **vacío** fallaba («no CRS»: no hay filas de las
  que sacarlo; ahora vale el de la tabla) y quién tiene la cobertura se pregunta al catálogo por el que se escribe,
  no aparte al `/v1`. Arnés de Spark 24/24.

**F2 · el esqueleto andante** (MVT, sólo crudo, `vis` = «lee el dataset entero»)

*F2·0, el previo (2026-10-22).* Antes de construir, lo que hay que saber para ir a tiro fijo: el código
que ya tenemos, la consola y la práctica del ámbito (CARTO, Martin, PostGIS `ST_AsMVT`, Tippecanoe, pg_tileserv,
PMTiles; la spec MVT 2.1 y TileJSON 3.0). Tres lecturas y tres medidas.

*Lo que ya sabemos (las lecturas):*

| | lo que hay | lo que implica |
|---|---|---|
| **el prototipo** (`e2/teselas`, T1) | Rust, `parquet 58` + `object_store`, poda por `geo_statistics` del grupo, sólo **puntos**, codificador MVT propio sin propiedades, `proj4rs` fijo a UTM30; 50–88 teselas/s con 32 lectores, RSS < 1 GB | es la semilla del motor, pero le faltan polígonos, recorte, simplificación, propiedades y leer Iceberg |
| **el motor, aparte** | `crates/ore-cli/tests/dependencias.rs:261` prohíbe que `ore-serve` enlace `iceberg`, `parquet` o TLS | **un binario propio, `ore-teselas`**, como `ore-motor` (C3 ya lo decía); el cliente en `ore-serve` es el de `al_motor` (`preview.rs:567`) con cuerpo binario |
| **leer la instantánea** | `iceberg 0.10` sólo en `ore-store`; ningún `TableScan` con predicado; `muestra.rs:152` recorre manifiestos → ficheros vivos; el manifiesto **no lleva cotas** (`lago_geo.rs:513`) | el motor lista los ficheros por el manifiesto y **poda grupos por las estadísticas de `geom_bbox`** (valen para `ore-store` y Spark; `geo_statistics` sólo las deja `ore-store`); sin cotas por fichero hay que abrir cada pie: **caché de pies** en el motor |
| **la credencial** | `prestar` da un token de STS acotado **al dataset**, no a la instantánea (`gcs.rs:638`) | F2 la usa tal cual; recortarla a la instantánea (C3-c) queda para F5 |
| **la caché HTTP** | `ore-entrada` fuerza `cache-control: no-store` (`http.rs:571`); la consulta pasa por una lista blanca; Cloudflare sin reglas de caché | la tesela inmutable necesita su propia salida (`immutable`, `ETag`) y una regla de caché en el borde |
| **el acceso** | leer datos no pregunta a Cedar (A8 abierta, 0047) | `vis` = «lee el dataset entero» es honesto en F2; fallar cerrado ya se puede escribir |
| **la consola** | MapLibre 5.24 + `react-map-gl` 8; ninguna capa remota; el navegador **no ve el token** (cookie `httpOnly`); el patrón es un *route handler* de Next que pone el `Bearer` en el servidor | el **descriptor** va por la consola (servidor, con el `Bearer`); las **teselas**, o por el mismo *proxy* (sin CDN, y con la carrera de renovación de `proxy.ts:173`) o **directas a ore-serve con la firma de C2** (cacheables en el borde) |
| **la práctica** | todos: trabajadores sin estado, caché en memoria, caché HTTP/CDN delante, plazo por tesela; presupuesto ~**500 KB** comprimidos y ~200 000 elementos por tesela (Tippecanoe); cadena: a 3857 → a la tesela → **simplificar < 1 px** → **recortar con margen** → a la rejilla entera → sin repetidos → **fuera anillos < 4 u²** → **orientar y validar**; orden estable (Hilbert + id) para que la tesela sea idéntica byte a byte | es la cadena de C3; DuckDB tiene `ST_AsMVT` desde 1.4 (sirve de **referencia** para contrastar el nuestro) |

*Las decisiones (D-F2-1..6, aprobadas el 2026-10-22):*
- **D-F2-1 · El motor es `ore-teselas`**, binario aparte desde `e2/teselas`, con su Deployment y su NetworkPolicy
  como `58-el-motor.yaml`; lee la instantánea por el manifiesto con `iceberg`/`parquet` de `ore-store`.
- **D-F2-2 · La poda es por `geom_bbox`** (estadísticas min/max de la estructura) para todo escritor, y caché de pies
  por fichero; las cotas por fichero en el manifiesto, a F4.
- **D-F2-3 · La cadena de geometría es la de consenso** (arriba), con la crate `geo` (simplificación, recorte por
  `i_overlay` en enteros, orientación) y **el codificador propio** del prototipo ampliado (propiedades, polígonos,
  líneas): pequeño, determinista y nuestro; contrastado contra `ST_AsMVT` de DuckDB.
- **D-F2-4 · El crudo tiene suelo de zoom**: el descriptor dice `minzoom` (lo que cabe en el presupuesto) y por
  debajo MapLibre no pide; en F2 no hay agregado (F4) ni muestreo (nunca).
- **D-F2-5 · Las teselas van directas a ore-serve con la firma de C2** (por clave, fuera de la clave de caché), y
  el descriptor por la consola; así se cachean en el borde y no pasan por la sesión. Si F2·0c lo tumba, el *proxy*.
- **D-F2-6 · Determinismo como contrato**: orden Hilbert + id, claves y valores por orden de aparición, campos del
  protobuf en orden fijo; el `ETag` es la huella.

*Las medidas del previo (lo que no se sabe sin medir; son parte de F2·0, no fases):*
- **F2·0a · El polígono en Rust**: el prototipo con la cadena de D-F2-3 sobre los edificios escritos en F1·3 (SDK y
  Spark): ms por tesela de z10 a z16, KB, elementos, validez (`geo::Validation`) y contraste con `ST_AsMVT`.
  Fija el `minzoom` del crudo (D-F2-4) y el presupuesto.
- **F2·0b · El salto**: lo que añade ore-serve → motor (HTTP interno por conexión) y la entrada (Envoy, túnel) a una
  tesela, frente al motor solo; con 32 lectores.
- **F2·0c · El borde y el navegador**: una tesela con firma, `Cache-Control: immutable` y CORS desde el dominio de la
  celda: ¿la cachea Cloudflare (`cf-cache-status: HIT`) y la carga MapLibre desde la consola? Necesita una regla de
  caché en Cloudflare (cambio fuera del clúster: con permiso).

*Lo medido en el previo (2026-10-22, `f2/`: `motor/` el motor de laboratorio, `m1.py`–`m3.py`):*

**F2·0a · El polígono en Rust** (los edificios de F1·3 por el SDK; Madrid, en caliente; RDP de 1 u):

| z | elementos | KB (gzip) | ms | inválidos | `ST_AsMVT` de DuckDB |
|---|---|---|---|---|---|
| 12 | 36 417 | 1 192 (678) | 981 | 193 | 37 872 · 1 672 ms |
| 13 | 10 955 | 445 (**287**) | 947 | 29 | 11 031 · 730 ms |
| 14 | 5 087 | 226 (156) | 979 | 4 | 5 093 · 378 ms |
| 16 | 430 | 22 (16) | 642 | 2 | 431 · 212 ms |

- **La cadena es correcta**: los mismos elementos que DuckDB (la diferencia son los diminutos que se descartan a
  propósito), un 28 % menos de vértices, ningún anillo nulo, ids únicos, orientación de la spec 2.1.
- **El suelo del crudo de estos edificios es z13** (287 KB en gzip; z12 ya son 678 KB): D-F2-4 se calcula por la
  densidad del dataset, no se fija a mano.
- **El motor lee de más**: en z14 descodifica 400 000 polígonos para servir 5 000 (el 99 % «fuera»); por eso DuckDB,
  que filtra la estructura `geom_bbox` antes de leer `geom`, va 2,6× más rápido. Con 32 lectores, 27 teselas/s,
  p50 1,1 s, 2,5 GB. **El arreglo es leer primero `geom_bbox`, filtrar las filas y sólo entonces `geom`**
  (`RowFilter` de `parquet`, materialización tardía) **y saltar páginas por su índice**, más el grupo por bytes.
- **Tras la rejilla entera, un 0,08–0,5 % de los polígonos no es válido** (z14–z12): hace falta reparar al final,
  como PostGIS. La simplificación que conserva la topología (Visvalingam) sólo baja los inválidos un tercio y cuesta
  hasta 8× en los zooms bajos: RDP y reparación.

**F2·0b · El salto**: una conexión nueva al motor por tesela, como hace `ore-serve` con `ore-motor`, **no se nota**
(32 lectores: p50 1 133 ms el motor solo, 1 129 por la puerta; 27,3 frente a 26,6 teselas/s). No hacen falta
conexiones persistentes en F2; lo que cuesta es el motor.

**F2·0c · El borde y el navegador** (una ruta temporal `/f2-teselas/` en la entrada real de victor: Cloudflare →
túnel → Envoy → el motor):
- La tesela llega con `Cache-Control: immutable`, `ETag` y CORS; **el `ETag` es el mismo en cada petición** (el
  determinismo de D-F2-6, de punta a punta).
- **MapLibre 5.24 la pinta desde otro origen con la firma en la URL** (`f2/m3.html`), sin error de CORS.
- Sin regla de caché, Cloudflare **no la guarda** (`cf-cache-status: DYNAMIC`: `.mvt` no está entre lo que cachea de
  serie) y el borde suma ~220–360 ms al motor. **Deuda:** una regla de caché en `ontoforge.org` (la zona de los
  *custom hostnames*: la cadena es `victor.ore.paladio.io` → `ore-mesh.ore.paladio.io` → `entrada.ontoforge.org`)
  no casó (siguió `DYNAMIC`); no se depuró para no parar el plan. Se cierra en F2·4, con la ruta real de las teselas.
- Para llegar, Envoy necesita poder salir al motor (hoy sólo sale a `ore-serve`): en F2 las teselas pasan por
  `ore-serve`, así que no cambia nada; si algún día van directas, es una política más.

*Lo que cambia en las decisiones:*
- **D-F2-2** se precisa: la poda es por grupo (`geom_bbox`), **por fila antes de descodificar** y por página.
- **D-F2-3** suma **reparar al final** (lo que la rejilla rompe) y RDP como simplificación.
- **D-F2-7 · El grupo de filas de un dataset geo se corta por bytes** (~4 MB, como Spark) también en `ore-store`.

*Los pasos después del previo:*
- F2·1 **El esqueleto de punta a punta**: el codificador MVT del prototipo leyendo Iceberg con la instantánea y la
  poda por caja; ore-serve llamando a un motor interno como a ore-motor; MapLibre cargando un TileJSON.

  *F2·1, el diseño (por aprobar, 2026-10-22).* **Un dataset geo real del catálogo de victor, pintado en el mapa de
  la consola por el camino de producción** (consola → ore-serve → `ore-teselas` → el lago), fino en todo salvo en
  ser real: desplegado, autenticado y medido. Lo que el previo dejó para F2·3 (filtrar por fila antes de
  descodificar, reparar, grupo por bytes, suelo calculado) **no** entra: el esqueleto prueba el camino, no la marca.

  | pieza | qué hace en F2·1 | dónde |
  |---|---|---|
  | **`ore-teselas`** | el motor de `f2/motor` hecho crate: lee la instantánea (metadata → manifiestos → ficheros vivos, el código de `ore-store` `muestra.rs` hecho público), poda por `geom_bbox`, la cadena de F2·0a, MVT; caché de pies; GCS y S3 (el laboratorio) con **la credencial que le pasan** | `crates/ore-teselas`, binario en la imagen `ore` |
  | **C3 v1, fino** | `POST /v1/teselas` · `{protocolo: 1, tms, z, x, y, formato: "mvt", fuente: {metadata_location, geometria, crs, id?}, limites: {plazo_ms, max_bytes}, credencial}` → 200 MVT + `x-ore-cuenta` (grupos, filas, elementos, ms por fase) · 204 vacía · 422 inválida · 503 plazo | tipos en `ore-core` |
  | **C2 fino: el descriptor** | `GET /v1/layers/{base.dataset}/descriptor` (persona, `Bearer`): resuelve el dataset en `main`, comprueba su columna geo, calcula la **clave** (huella de dataset + `metadata_location` + protocolo) y la guarda (clave → fuente, en memoria); devuelve **TileJSON 3.0** con `tiles` firmadas, `minzoom`, `maxzoom` 16, `vector_layers`, `dataAsOf` y la atribución | `ore-serve` |
  | **C2 fino: la tesela** | `GET /v1/tiles/{layer}/{clave}/WebMercatorQuad/{z}/{x}/{y}.mvt?e=…&f=…`, **sin `Bearer`**: comprueba la firma (HMAC de clave y caducidad; 403), la clave (410 si no la conoce: el cliente pide otro descriptor), pide la credencial (`prestar`, en caché hasta que caduque) y llama al motor; responde `immutable`, `ETag` y CORS | `ore-serve` (`al_motor` con cuerpo binario) |
  | **`ore-entrada`** | una salida de bytes que fija su `cache-control`; `e` y `f` en la lista blanca de la consulta; la ruta de teselas sin `Bearer` (la firma es la autorización) | `crates/ore-entrada` |
  | **la malla** | `59-las-teselas.yaml`: Deployment (1 réplica), Service, NetworkPolicies (entra `ore-serve`; sale DNS y Google), cuenta sin token; el secreto de la firma por celda | `malla/` |
  | **la consola** | sobre **«Add data»** del mapa (Layers → el árbol del Assets Catalog, con su rama): cada activo elegido pide su descriptor por una acción de servidor (con el `Bearer`, en su rama por `x-ore-rama`); `MapaPlano` añade su fuente vectorial y un relleno del color de la capa; Layers dice si pinta («from zoom 13»), si no tiene geometría o si falló | `rubix-platform` |

  *Las decisiones (K-F2·1-a..f, por aprobar):*
  - **a · La capa es el dataset hasta F2·2**: la URL ya es `/v1/layers/{layer}`; F2·2 cambia sólo cómo se resuelve.
  - **b · La tesela se autoriza por la firma**, no por la sesión (D-F2-5): HMAC con un secreto de la celda,
    caducidad en la URL, fuera de la clave de caché.
  - **c · El `minzoom` del crudo es un parámetro** (13 para los edificios, lo medido) hasta que F2·3 lo calcule.
  - **d · El motor viaja en la imagen `ore`** (como `ore-store-*`), con su propio Deployment.
  - **e · La tabla clave → fuente vive en memoria** de `ore-serve` (una réplica); si no está, 410 y se pide otro
    descriptor. Persistirla llega con varias réplicas (F4·4).
  - **f · La consola sólo trae el descriptor**; las teselas van del navegador a la entrada de la celda.
  - **g · Una capa MVT con los tres tipos mezclados** (punto, línea, polígono); el cliente los separa con un filtro
    `geometry-type`, sin que el descriptor diga qué trae (F2·1·6).
  - **h · Una `GeometryCollection` se descompone**: un elemento por tipo con el mismo id; los «multi» se funden en el
    suyo. Z y M se descartan (MVT es 2D). Las curvas no se pintan: el lago ya las rechaza al escribir (sólo admite los
    tipos 1–7) y el motor, si le llega una, la cuenta en `descartados`.
  - **i · Los puntos de filas distintas no se funden** aunque caigan en el mismo píxel: el mapa nunca miente, y la
    densidad se resuelve agregando (F2·3), no escondiendo filas. Sólo los repetidos de un mismo multipunto, uno.
  - **j · Las líneas se simplifican con RDP abierto a 1 u** y se recortan con Liang–Barsky contra el marco con su margen;
    la que sale y vuelve a entrar se parte en trozos de una MultiLineString.

  *Los pasos:*
  - F2·1·1 **`ore-teselas` y C3 v1** con sus pruebas: la tesela de oro (idéntica byte a byte), el contraste con
    `ST_AsMVT` en el laboratorio, 204 / 422 / 503.
  - F2·1·2 **ore-serve y `ore-entrada`**: descriptor, tesela, firma, clave, credencial; pruebas de 403 / 410 / 204.
  - F2·1·3 **El laboratorio de punta a punta** (el banco local: ore-serve + `ore-teselas` + el lago S3 + una página
    MapLibre), igual que prod.
  - F2·1·4 **La malla y el CI**; un empuje (con permiso) y desplegado en victor.
  - F2·1·5 **La consola** y **el dataset**: los edificios del banco como dataset geo del catálogo de victor (lo crea
    una persona, 0052) y vistos en el mapa. *Rehecho el 2026-10-23:* no hay `?capa=` ni *route handler*: la consola
    ya tenía **«Add data»** (Layers → elegir activos del Assets Catalog, en una rama), que listaba la capa sin
    pintarla. F2·1·5 la pinta: `descriptorDeCapa` (acción de servidor, `map/capas.ts`), `useDescriptores` (pide al
    entrar la capa, olvida al quitarla, **renueva** ante un 403/410 de una tesela, una vez por minuto),
    `CapasVectoriales` (la fuente y su relleno en `MapaPlano`) y el estado en Layers. En `ore-serve`, el descriptor
    lee la rama de `x-ore-rama` (con la vuelta a `main` de `datos_en`); la clave no la lleva: la lleva su
    `metadata_location`. El banco de la consola trae una base `geo` con `edificios` y pide su descriptor al
    laboratorio de F2·1·3 (`RUBIX_BANCO_TESELAS`): el camino entero en local.
  - F2·1·6 **Puntos, líneas y colecciones** (K-F2·1-g..j, 2026-10-24), para medir también con los viajes de
    `query.default`: `geometrias()` lee los siete tipos WKB (ISO y EWKB); puntos en el marco con su margen, líneas con
    RDP abierto y Liang–Barsky, polígonos como estaban; un `MoveTo(n)` el punto, `LineTo` sin cerrar la línea; el
    orden, Hilbert + id + tipo. `x-ore-cuenta` suma `puntos`, `lineas`, `poligonos` y `descartados`. Pruebas del motor
    12/12 (con un lector de MVT que comprueba la codificación); el laboratorio 22/22 (`geo.viajes_puntos` 200 000 y
    `geo.viajes_lineas` 50 000: cada tesela sólo su tipo, sin descartados, `ETag` estable). En la consola,
    `CapasVectoriales` pone círculo, línea y relleno con borde, cada uno con su filtro. Sólo cambia el binario
    `ore-teselas`: ni `ore-serve`, ni C3, ni la malla. Los datasets de victor los crea una persona (0052):
    `query.default.trip_3m_puntos` (`ST_Point` de la recogida) y `trip_3m_lineas` (`ST_MakeLine` recogida→destino;
    rectas largas al azar: la poda por `geom_bbox` casi no descarta, el caso de F2·3).
  - *Hecho cuando:* los edificios se ven y se navegan en la consola de victor por ese camino, y se mide: teselas
    p50 / p95 desde el registro de `ore-serve`, la cuenta del motor y el primer pintado (la vara pide < 1 s: lo que
    falte es F2·3).
- F2·2 **C1 fino en OOS v1alpha34:** `Layer` con `source`, `properties` y `styles`; el compilador lo valida y el
  catálogo lo lista con su linaje.
- F2·3 **El motor a escala de producción** (rehecho el 2026-10-25 con el banco de victor; antes «C3 a fondo»): el
  motor de teselas en Rust, puro, con réplicas, crudo, MVT, determinista, **la geometría simplificada por debajo del
  píxel y válida tras recortarla**, **la reproyección con `geometry_always_xy`**, y límites de tiempo, memoria y
  concurrencia, **diseñado desde las variables de producción y medido en un banco en victor antes de mezclar**.
  - *Ya en main (fab368d5)*: **los turnos** (tantas teselas a la vez como CPU; lo que espera cuenta en su plazo; sin
    turno a tiempo, 503 rápido y no un pod caído) y **la criba por fila** (D-F2-2: un `RowFilter` sobre las cuatro
    hojas de `<geom>_bbox`; `geom` sólo de las filas cuya caja toca la tesela; `x-ore-cuenta` suma
    `filas_en_grupos`). En el banco nada cayó ni pasó de memoria: lo que falla es el tiempo.
  - *Las variables de producción* (lo que el diseño tiene que aguantar):
    - **GCS desde un puesto de victor** (`sonda_gcs.py`): un GET por rango de 64 KB a 1 MB tarda **~55 ms p50 y
      ~100 ms p95** (4 MB: 106 / 166 ms; 8 KB no es más rápido); **4 en paralelo no cuestan más**, 16 a la vez
      salen a 130 ms p50 (con el cliente de Python en 1 CPU).
    - **El pod**: `ore-teselas` 1 CPU / 2 GiB; la cola del pool system admite 1 CPU / 4 GiB en total.
    - **El cliente**: MapLibre pide **12–15 teselas de 512 px a la vez** por vista de 1600×900, y otra tanda al
      mover.
    - **Los datos**: puntos y polígonos de `ore-store` en orden Hilbert (grupos de 100 000 filas, ~4,5 MB, un
      fichero de 135 MB para 3 M); las líneas al azar (cada grupo cubre NYC entera); de 3 M hoy a **1 000 M** en
      la vara (§2), y Spark sin orden.
  - *Medido* (`banco.py`: el `ore-teselas` de `main` al lado de un puesto, con los límites de la malla; el
    generador pide como MapLibre; 2026-10-25, `banco-20261025-1118.log` y, para las líneas en z15,
    `banco-20261025-1103.log`). *Aislada*: de una en una, plazo de 60 s. *Vista*: las 12–15 a la vez, plazo de
    10 s (el de producción):

    | dataset | z | aislada p50 / p95 | `leer` (p50) | grupos leídos (filas) | elementos | vista: llegan |
    |---|---|---|---|---|---|---|
    | `trip_3m_puntos` | 13 | 4,0 / 7,1 s | 4,0 s | 3 de 30 (300 000) | 23 301 | 2 de 12 |
    | | 14 | 2,5 / 5,0 s | 2,5 s | 2 (200 000) | 5 822 | 3 de 12 |
    | | 15 | 2,4 / 4,2 s | 2,4 s | 2 (200 000) | 1 454 | 5 de 12 |
    | | 16 | 2,2 / 3,3 s | 2,2 s | 2 (200 000) | 370 | 4 de 15 |
    | `trip_3m_lineas` | 13 | — (6,4–8,1 MB, pasa los 2 MB) | | | | 0 de 12 |
    | | 15 | 42,9 / 53,4 s | 41,9 s | 28 de 30 (2,8 M; 812 000 tocan la caja) | 100 107 | 0 de 12 |
    | | 16 | 40,0 / 41,8 s | 39,1 s | 26 (2,6 M; 779 000) | 49 459 | 0 de 15 |
    | edificios de Madrid (de G2) | 13 | 6,9 / 8,9 s | 6,8 s | 6 de 14 (600 000) | 11 181 | 1 de 8 |
    | | 14 | 5,1 / 6,4 s | 5,1 s | 5 (500 000) | 2 663 | 1 de 12 |
    | | 15 | 5,0 / 6,3 s | 5,0 s | 5 (500 000) | 1 031 | 2 de 12 |
    | | 16 | 4,9 / 6,0 s | 4,9 s | 5 (500 000) | 397 | 3 de 12 |

  - *Lo que dicen los números:*
    1. **El tiempo es espera de GCS, no CPU.** `leer` es el 99 % de cada tesela; descodificar y codificar no
       pasan de 60 ms ni con 23 000 puntos. Sirviendo en frío, el motor usó **61 m de CPU y 8 MiB**. Sale ~1 s
       por grupo leído, sea cual sea el zoom: 2,2 s / 55 ms ≈ **40 GET en serie** por tesela. El lector es
       síncrono (`ParquetRecordBatchReaderBuilder` sobre `Rangos`): pide la cola del fichero (64 KB) en cada
       tesela aunque el pie ya esté en caché, y después cada trozo de columna y cada página, uno tras otro.
    2. **Los turnos por CPU no casan con un trabajo que espera**: 1 CPU = 1 turno = una tesela de 2–7 s a la vez;
       una vista de 12 son 25–80 s de cola, y con el plazo de 10 s llegan de 0 a 5.
    3. **La poda por grupo funciona en lo Hilbert** (2 de 30 grupos), pero el grupo es enorme para la tesela:
       en z16 se leen 200 000 filas para 370 puntos.
    4. **Las líneas no se arreglan en el motor**: no les falta un índice, es densidad. En z16 **49 000 líneas
       cruzan de verdad cada tesela** (y 779 000 tocan su caja); en z13 la tesela pesa 6–8 MB. Ningún orden ni
       ninguna poda hace eso legible en 300 ms ni en el navegador: en esos zooms va agregado (C4).
    5. **El presupuesto**: a 300 ms en frío (p95) con un GET de ~100 ms (p95) caben **dos oleadas a GCS** y
       ~100 ms de CPU. Hoy son ~40 idas.
  - *El rediseño (D-F2-8 a D-F2-12, por aprobar):*
    - **D-F2-8 · Una tesela son a lo sumo dos oleadas a GCS, en paralelo.** El pie y el índice de páginas viven en
      memoria, y con el pie en caché no se relee la cola. La primera oleada trae las páginas de las hojas de
      `<geom>_bbox` que el índice de columnas no descarta; la segunda, las páginas de `geom` e `id` de las filas
      que pasan. Cada oleada, todas sus páginas a la vez (asíncrono), con los huecos de menos de 64 KB
      fusionados. `x-ore-cuenta` suma `peticiones`, `oleadas` y `bytes`: es lo que el banco vigila.
    - **D-F2-9 · La espera y la CPU se separan.** Muchas teselas pueden esperar a GCS a la vez (con tope por
      memoria, no por CPU); los turnos (= CPU) sólo cubren descodificar, recortar y codificar. Un rango que ya
      está en camino no se pide dos veces: las 12 teselas de una vista comparten sus 2–3 grupos.
    - **D-F2-10 · Una caché de páginas por contenido.** Las páginas de un fichero inmutable van a una LRU en RAM
      con tope (1 GiB de los 2), con clave fichero + rango; la tesela codificada, a su caché (D-S-4) y al borde.
      Una tesela con caché no toca GCS: p95 < 50 ms.
    - **D-F2-11 · La copia de servicio: una cuarta pieza de C4.** Para 1 000 M, el lago tal cual no llega
      (grupos de 100 000 filas, Spark sin orden, líneas que cruzan la ciudad). El mismo Build del sistema que
      hace la densidad y la pirámide (C4-c: sólo si hace falta) escribe por `Layer` un dataset derivado (D-S-5,
      con linaje y salud) con las geometrías largas **subdivididas** (como `ST_Subdivide`: cada trozo con tope de
      vértices y de tamaño, con el id de su fila). Los trozos se guardan **en orden Hilbert y con páginas
      pequeñas (~64 KB)**, sin claves de tesela (C4-b), así que vale para las dos rejillas. Una tesela del crudo
      es entonces un rango contiguo que se localiza en el índice de páginas en memoria: una oleada.
      - **Por debajo del suelo, la pirámide** (F4·1 se adelanta en F2·3·2, para los tres tipos). El suelo es el zoom más bajo en
        que la tesela más densa cabe en el presupuesto (elementos y bytes): sale del mapa de densidad (D-F2-4
        calculado).
      - **Mientras se construye**, el motor lee el lago tal cual (D-F2-8 a D-F2-10) y la salud dice
        `behind-source`.
    - **D-F2-12 · Lo que no llega se degrada, no se cae.** Si una tesela pasa su plazo o su tope, sale del nivel
      agregado de encima, marcada (§2). Mientras no haya pirámide, sigue el 503 rápido de hoy.
    - *El tamaño*: sin la espera, la CPU medida por tesela es ≤ 60 ms, así que una vista de 12 son ≤ 0,7 s en
      1 CPU. Se mide antes de subir de 1 CPU; las réplicas (F4·4) se dimensionan por la CPU usada.
  - *Los pasos* (cada uno medido en el banco antes de mezclar; nada se mezcla sin pasar su vara):
    - **F2·3·0 · Contar las idas**: `peticiones`, `oleadas` y `bytes` en la cuenta, sin cambiar nada más. El banco
      confirma las ~40 idas por tesela.
    - **F2·3·1 · El lector por oleadas, la espera separada y la caché** (D-F2-8 a D-F2-10), sobre el lago tal
      cual. *Vara*: puntos y edificios en z13–16 con p95 < 300 ms en frío y < 50 ms con caché, la vista de 12 a
      la vez entera en < 1 s y sin 503.
      *En main sin pasar la vara entera (2026-10-25, por decisión del usuario, para probarlo en la consola)*:
      `lector.rs` (las oleadas con los huecos de < 64 KB fundidos, la caché de páginas con tope
      `ORE_TESELAS_CACHE_MB`, un rango en camino no se pide dos veces, tope de GET `ORE_TESELAS_GETS`); la
      cobertura se poda también por página (índice de columnas); los turnos de CPU sólo para la criba y lo que
      sigue, y en la puerta `ORE_TESELAS_EN_VUELO`; la caché de teselas codificadas (`ORE_TESELAS_TESELAS_MB`,
      D-S-4); mimalloc en musl (una tesela con caché, 13 → 10 ms a 1 CPU). Medido en el banco
      (`banco-20261025-1318.log`): puntos y edificios, la vista entera **12 de 12** en todos los zooms
      (0,5–1,9 s; antes 0–5 de 12); en frío z13, **2 oleadas y 6–7 GET** (p50 274–320 ms, p95 432–856 ms;
      antes 4–7 s y ~40 GET); líneas z16, 40 → 2,4 s. Con la caché de teselas (dos pasadas más: A con las
      cachés, B con las dos a 0, todo en frío): **con caché, p95 1–6 ms** y la vista entera en 60–417 ms
      (✓); **en frío**, la tesela p95 156–227 ms (puntos) y 177–365 ms (edificios) en z14–16, y la vista
      entera 0,48–0,96 s en z14–16. *Falta para la vara, sólo en frío y en z13* (lo más denso): la vista,
      1,0–1,6 s, y la tesela, p95 391–802 ms. Ahí el cuello es CPU con un turno (`turno_ms` 0,5–0,7 s en la
      vista): descodificar páginas de `geom` de 20 000 filas para quedarse con unas pocas. Lo cierran las
      páginas pequeñas de la copia de servicio y el suelo calculado (F2·3·3).
    - **F2·3·2 · La capa a cualquier zoom** (rehecho el 2026-10-25 con una prueba de concepto; antes «la copia
      de servicio y la pirámide de las líneas»). Lo pide el usuario: **una capa se ve a cualquier distancia**;
      el «from zoom 13» de la consola (por debajo del suelo no se pedía nada y el mapa quedaba vacío, sin aviso)
      no es producto. Por debajo del suelo, la pirámide de C4 (F4·0–F4·1 se adelantan para los tres tipos);
      por encima, el crudo del motor, como hasta ahora.
      - *La prueba de concepto* (`pruebas-de-fuego/geo/f2/piramide/`: `pc.py` construye en un Job de victor
        con **tippecanoe 2.79** —de conda-forge con micromamba, 11 s, sin permisos— y deja un **PMTiles** en el
        bucket del banco; `pc3.py` lo sirve desde GCS; `anidado.py` cuenta el parpadeo; `ver.html` lo pinta en
        MapLibre con el protocolo `pmtiles`). Los datos: `trip_3m_puntos` y `trip_3m_lineas` de victor (3 M
        cada uno, **sintéticos y uniformes**: SpatialBench, no taxis) y los edificios reales de la comunidad de
        Madrid (1,33 M polígonos, de G2):

        | capa | cómo, bajo el suelo | construir | fichero | tesela más pesada |
        |---|---|---|---|---|
        | puntos, 3 M | aclarados (`--drop-densest-as-needed`), z0–16 | 5,7 min, 489 MB | 74 MB | 67 KB (z10); 6–22 KB desde z13 |
        | edificios, 1,33 M | aclarados con tippecanoe, z0–16 y z0–12 | **no acaba en 60 min** (y con `--detect-shared-borders`, OOM a 3,5 GiB) | — | — |
        | | **agregados** en rejilla, z0–12 | 40 s (DuckDB 23 s + tippecanoe 14 s) | 2,4 MB | 15 KB |
        | | crudo z13–14 (para verlo; en el diseño lo sirve el motor) | 169 s | 64 MB | 420 KB |
        | líneas | aclaradas, una muestra de 100 000 (1/30) | 2,5 min | 287 MB | **~400 KB, al tope, de z0 a z12** |
        | | **agregadas** en rejilla, las 3 M, z0–13 | 5,5 min (el muestreo, 287 s) | 4,6 MB | 27 KB |

        *Anidado*: de 75 000 puntos vistos en una muestra de teselas de z8 a z15, **0 desaparecen al acercarse**
        (un punto de z sigue en z+1: sin parpadeo). Se cuentan sólo los de dentro de la tesela: los del margen
        son de las vecinas y en z+1 caen en otras hijas (contarlos daba un 2–4 % falso).
        *Servir desde GCS* (PC3, `pc3-20261025-2206.log`: un Job de victor en Python, conexiones con keep-alive,
        las 12–15 teselas de una vista de 1600×900 a la vez):
        - **con el directorio en memoria** (el servidor vivo, otra zona cada vez): la vista entera **75–380 ms**,
          tesela p95 **55–340 ms** (una vez 535 ms: 30 GET al cargar hojas del directorio), 1 GET por tesela;
        - **un proceso recién abierto** (cabecera, raíz, hojas y teselas): la vista **190–580 ms**, tesela p95
          180–500 ms. Frente al crudo de hoy en z13 en frío (vista 1,0–1,6 s), siempre por debajo de 1 s.
      - *Lo que dicen los números:*
        1. **Los puntos se aclaran**: barato (3 M en 6 min), teselas pequeñas en todos los zooms y anidado.
        2. **Los polígonos y las líneas no se aclaran: se agregan.** Aclarar 1,3 M de polígonos no acaba, y las
           líneas llenan el tope con 1/30 de los datos. Agregados: una **rejilla de 32 × 32 celdas por tesela**
           en WebMercator; la más fina se calcula una vez y las demás salen de ella (`cx >> s`), así todos los
           zooms cuentan lo mismo. Cada celda lleva `n`, `a` y la **cobertura por m²** (área construida / área
           de la celda; km de línea por km²): por celda a secas el color dependía del zoom (en las líneas, la
           mediana pasaba de 8 800 en z13 a 34 000 en z10).
        3. **Servir la pirámide es un GET por tesela** con el directorio en memoria: la vara de frío (p95 < 300 ms,
           vista < 1 s) se cumple holgada. Lo caro en frío es el directorio: se carga entero al abrir.
        4. **La pirámide llega hasta el suelo, no hasta z16**: por encima sirve el motor. Construir de z0 a z16
           es lo que no acababa.
        5. **Construir tiene que ir acotado**: tippecanoe sin tope no acaba, y DuckDB, muestreando las 3 M líneas,
           **derramó 10 GB al disco del nodo del sistema**: el desalojo se llevó también un pod del
           `aprovisionador` de `ore-system` (volvió solo en 5 min).
        6. **La escala no está medida**: 3 M de puntos son 6 min con 4 CPU; tippecanoe es casi lineal, así que
           1 000 M (§2) serían horas. Hasta medirlo (F4·5) vale para lo que hay en victor.
      - *Las decisiones (D-F2-13 a D-F2-18, por aprobar; revisan C4-a y C4-e):*
        - **D-F2-13 · Una capa se ve a cualquier zoom.** De z0 al suelo − 1, la pirámide; del suelo arriba, el crudo.
          El descriptor anuncia `minzoom` 0; la consola no tiene «from zoom N». El suelo, por tipo hasta que F2·2
          lo exponga en la `Layer` y F2·3·3 lo calcule: **13** para puntos y polígonos, **14** para líneas.
        - **D-F2-14 · Cada tipo, a su manera.** Los **puntos se aclaran** con selección anidada: cada punto entra en
          un zoom y no sale. Los **polígonos y las líneas se agregan** en la rejilla, con la cobertura por m².
          En la consola se funden: la capa agregada baja su opacidad en ~0,6 de zoom mientras sube la del
          detalle (`interpolate` por zoom), y la leyenda dice «densidad».
        - **D-F2-15 · La pirámide es de la `Layer`, no un dataset** (revisa C4-a y C4-e): son teselas ya pintadas,
          no una tabla (no hay nada que consultar), nacen y mueren con su capa, y en el Assets Catalog serían ruido.
          Un PMTiles por capa e instantánea, en el almacenamiento de ORE del inquilino
          (`piramides/<base>/<schema>/<nombre>/<instantánea>-<huella>.pmtiles`, la huella = versión de
          `ore-piramide` + opciones), con su **manifiesto** al lado (tipo, suelo, zooms, bytes, tiempos). Se ve en
          la página de la capa (estado, tamaño, instantánea), **hereda su acceso** y no tiene linaje propio: es una
          caché gestionada. `ore-teselas` la sirve por rango con **el directorio entero en memoria (un GET por
          tesela)**, con la misma caché de teselas, firma y clave que el crudo; C3 dice si la tesela es de la
          pirámide o del crudo. Hasta F2·2 la capa es el dataset (F2·1-a).
        - **D-F2-16 · `ore-piramide`: un binario Rust y tippecanoe, en una imagen propia y mínima.** Nada de
          `puesto-python` (ajena a esto: el SDK y DuckDB sólo eran comodidad de la prueba). El binario lee la
          instantánea como `ore-teselas` (`ore-store`, la credencial del Job), descodifica el WKB con el mismo
          código, **aclara** (puntos) o **agrega en memoria** (la rejilla fina como mapa de celdas: acotada por el
          número de celdas, no por las filas, sin derramar a disco) y **le pasa los elementos a tippecanoe por la
          entrada estándar** (sin fichero intermedio); sube el PMTiles y el manifiesto con `ore-store`.
          tippecanoe, **compilado de su fuente con versión fijada** (BSD-2) en una etapa aparte. La imagen:
          Debian slim + los dos binarios + sqlite y zlib (decenas de MB). Un generador de teselas propio (sin
          tippecanoe) sólo si F4·5 mide que no llega a 1 000 M.
        - **D-F2-17 · Un Job del sistema, como la copia, y acotado.** No es un Build de 0055 (esos corren código del
          cliente en su puesto): es un Job de ORE como el de la copia (`malla/48-la-copia.yaml`): una plantilla en
          la malla que el aprovisionador deja rendida en la cola, `ore-serve` sustituye la capa y la instantánea, y
          Flux lo crea. **El nombre sale del contenido** (capa + instantánea + huella): dos disparos iguales son el
          mismo Job. Va en la cola pesada (`cola-pesada`, fuera del pool del sistema: el previo desalojó el
          `aprovisionador`), con disco efímero pedido y un `emptyDir` con tope, memoria y plazo; tippecanoe con
          tope de tiempo. Si no acaba, la capa sigue con lo que tenga, marcada, y no se lleva nada por delante.
        - **D-F2-18 · El descriptor dispara; no hay vigilante.** Al pedir el descriptor de una capa, `ore-serve` mira
          si su instantánea tiene pirámide (el manifiesto). Si no, encola el Job (idempotente por nombre) y sirve
          la pirámide de la instantánea anterior marcada `behind-source`; sin ninguna, el crudo desde el suelo y
          «construyendo» en el descriptor. Una capa que nadie mira no gasta nada (C4-c). El Job, al acabar, borra
          las pirámides de la capa salvo la nueva y la anterior (las URLs firmadas en vuelo). **Permisos**: con un
          filtro de fila de quien mira (A8), la pirámide no se sirve (agrega todo): sale el crudo, que sí filtra
          (C4-f: falla cerrado).
      - *Los pasos de despliegue* (de la prueba a producción; cada uno con su vara en el banco de victor antes de
        mezclar). Se empuja en **dos veces**, binario antes que malla: los pasos 1–3 y 5 (con el disparo apagado)
        en un PR; con eso desplegado, el 4 (la plantilla en la malla) y encender el disparo. La consola, aparte,
        en `rubix-platform`.
        1. **F2·3·2·1 · Servir la pirámide** (`ore-teselas`): el lector de PMTiles en Rust (el de la prueba,
           `lector_pmtiles.py`, validado 340/340 contra la librería oficial), el directorio entero en memoria, los
           GET por `Paginas::traer` (un rango en camino no se pide dos veces) y la caché de teselas; C3 con
           `piramide` en la petición. *Vara*, con los PMTiles de la prueba ya en el bucket del banco: las tres capas
           en z0–12, la tesela p95 < 300 ms en frío y < 50 ms con caché, la vista entera < 1 s.
           *Hecho en la rama (2026-10-26), vara pasada*: `crates/ore-teselas/src/piramide.rs` (cabecera, raíz y
           todas las hojas al abrir, dos oleadas y una vez por pirámide; la tesela, un GET por `Paginas::traer`;
           descomprimida y con la capa renombrada a la del descriptor, sin tocar lo demás); en C3,
           `fuente.piramide = {ubicacion, credencial}` (la suya: la pirámide no vive con el dataset): los zooms de
           la pirámide salen de ella y los demás del crudo; una pirámide rota falla y no cae al crudo; la clave de
           la caché lleva la pirámide (la pareja, C4-d); `x-ore-cuenta` dice `piramide`. Contra tippecanoe: las
           1 840 teselas de muestra de las cuatro pirámides de la prueba, las mismas que la librería oficial
           (`validar_lector.sh`). Medido en victor (`banco-20261026-1001.log`, el motor de la rama al lado, los
           límites de la malla): en frío la tesela p50 34–46 ms y **p95 39–83 ms** (un GET; la primera, que abre
           la pirámide, hasta 230 ms), **con caché 1–2 ms**, y **la vista entera de 8 a 99 ms**, en z9–12 de las
           tres capas.
        2. **F2·3·2·2 · El binario `ore-piramide`** (crate nuevo): leer, aclarar o agregar, tippecanoe por la
           entrada estándar, subir el PMTiles y el manifiesto; topes de tiempo y de celdas. *Vara*, en un Job del
           banco con el binario copiado: las tres capas de la prueba con los mismos números (celdas, tamaños,
           anidado 0 %) y sin pasar de la memoria pedida ni escribir a disco más que el PMTiles.
           *Hecho en la rama (2026-10-26)*: `crates/ore-piramide` (biblioteca + binario: la petición en JSON por
           la entrada estándar; lee la instantánea con `ore-store` y los trozos por rango de `ore-teselas`, con su
           WKB; aclara o agrega; tippecanoe por la entrada estándar con tope de tiempo; sube el PMTiles en flujo
           y cotejado —sha256 + crc32c— y su manifiesto; sin credencial, la identidad del pod:
           `gcs::Cuenta::de_la_identidad`). **El zoom base de los puntos, tres por encima del suelo** (`-B16`
           con suelo 13, lo de la prueba): con el base en el máximo de la pirámide o en el suelo, z9–12
           llegaban al tope de 500 KB y tippecanoe aclaraba tesela a tesela, y **hasta el 55 % de los puntos
           desaparecían al acercarse**; con `-B16`, 0 %. El salto que queda en el suelo (el crudo los trae todos)
           es del suelo calculado (F2·3·3). Medido en victor (`banco-piramide-20261026-1023.log` y la repetición
           de los puntos), leyendo el lago con la identidad del pod y subiendo a `piramides/banco/` del lago
           (borrado después): **puntos 3 M**, 5,3 min (leer 81 s, tippecanoe 234 s), 20 MB, tesela máx. 52 KB,
           **anidado 0 %** de z8 a z12; **líneas 3 M**, 4 min (leer y agregar 221 s, tippecanoe 20 s), 4,8 MB,
           140 402 celdas, tesela máx. 27 KB. *Falta*: los polígonos, sólo con pruebas (área, centroide, la
           rejilla y una construcción entera con un tippecanoe falso): en el lago de victor no hay ninguno, y
           escribir los edificios del banco pide el catálogo, que hoy no responde (`ore-iam` 502 «connection
           closed»). Leer es de un hilo (81 s los puntos): en paralelo por fichero, si F4·5 lo pide.
        3. **F2·3·2·3 · La imagen `ore-piramide`**: una etapa que compila tippecanoe de su fuente (versión fijada), el
           binario de la etapa Rust de siempre, y la final en Debian slim. En el CI: en las listas de imágenes
           (construir, huella, desplegar) y en `ci/prueba-huella-de-la-imagen.py`. *Vara*: el CI de main en verde
           y la imagen en Artifact Registry con su huella.
           *Hecho en la rama (2026-10-26), falta el CI de main*: en el `Dockerfile`, la etapa `tippecanoe`
           (Debian trixie, la 2.79.0 de su fuente con el tarball cotejado por sha256, sólo `tippecanoe` y
           `tile-join`) y la imagen `piramide` (Debian slim + sqlite, zlib y la biblioteca de C++ + los dos
           binarios; `ORE_TIPPECANOE` puesto; usuario 65532). `ore-piramide` en `ci/compilar-binarios.sh`
           (paquete, binario y la puerta de arranque, ahora con la entrada estándar cerrada), en
           `ci/huella-de-la-imagen.py` (`piramide` → `ore-piramide`) y en las etiquetas que mueve `ci.yml`.
           La prueba de la huella, 19 de 19, con dos casos nuevos: la versión de tippecanoe y la huella de
           `ore-piramide` rehacen esa imagen y sólo ésa. Construida en local (`ore-piramide:local`, con la etapa
           `bin` de `motor/binarios-locales.Dockerfile`): **148 MB** (no «decenas»: la base Debian y la
           biblioteca de C++ son la mayor parte; adelgazarla, si pesa, con tippecanoe estático), y **de punta a
           punta** contra el lago de victor (`piramide/imagen_local.py`): las 3 M líneas dan la misma pirámide
           que el binario en el banco, byte a byte en tamaño (140 402 celdas, 4 780 243 bytes).
        4. **F2·3·2·4 · El Job del sistema**: la plantilla `malla/59-la-piramide.yaml` (como `48-la-copia.yaml`:
           cuenta `driver`, `cola-pesada`, `emptyDir` con tope, disco efímero pedido, plazo), el aprovisionador la
           deja rendida en la cola y `ore-serve` sabe rendirla (`cola::rendir_piramide`, nombre por contenido, con
           su prueba como las de la copia); la netpol de salida a GCS. *Vara*: en victor, el Job construye las tres
           capas, y durante la construcción el nodo no pasa a `DiskPressure` ni se desaloja nada.
           *Hecho en la rama (2026-10-26)*: `malla/59-la-piramide.yaml`, el Job de `ore-piramide` con la cuenta
           `driver` y su rol de red (lee y escribe el lago por la 443 de `20-driver.yaml`: **no hizo falta otra
           netpol**), en `cola-pesada` (la única cola que cubre `ephemeral-storage`: 48 Gi pedidos y `/trabajo`
           con tope de 40 Gi), 2–4 CPU y 4–8 GiB, plazo de una hora, sin testigo de la forja ni secretos: la
           petición sólo lleva ubicaciones. `gen-inquilino.py` la rinde como `plantilla-piramide.txt` (y la
           guarda ㉒ exige el tope por debajo de lo pedido, la cola pesada, la cuenta del driver y ningún
           secreto); el aprovisionador la deja en la cola y conserva los `59-la-piramide-*` que encola
           `ore-serve`. `cola::rendir_piramide`: **una ranura por tabla** (`59-la-piramide-<resumen de la
           tabla>.yaml`: una instantánea nueva sustituye al Job de la anterior y Flux lo poda) y el nombre del Job
           por contenido. El disparo de D-F2-18, ya de verdad: con `ORE_PIRAMIDE_DISPARO=1`, el descriptor de una
           capa sin pirámide la encola (una vez cada 15 min por pirámide, para no clonar la cola en cada
           descriptor) y dice `construyendo`; si no puede, `ausente` con el motivo. **El interruptor sigue apagado
           en la malla**: se enciende cuando la plantilla esté en la cola de la celda. Y `ore-piramide` **sale en
           seco si el manifiesto ya está** (se sube lo último): el Job que Flux recrea al caducar el TTL no rehace
           nada. Medido en victor (`piramide/banco-job-20261026-1239.log`, la plantilla rendida para victor y la
           petición letra por letra de `ore-serve`, la imagen de la rama en una etiqueta de banco, borrada
           después): **las dos capas a la vez en 259 s** (95 s de ellos, encender el nodo pesado; Kueue admite las
           dos en la cuota de 7 CPU), **0 desalojos y ningún `DiskPressure`**; relanzadas, **en seco en 14 s**. En
           la segunda vuelta GCE **reclamó el nodo spot a mitad** y el Job rehízo el pod en otro y acabó: el
           reintento basta. *Los puntos sin id*: la petición de `ore-serve` no lleva columna `id` (el descriptor
           no la conoce), y la pirámide pesa **575 KB frente a 1,53 MB** con id; **las 76 teselas son idénticas**
           (los mismos puntos en el mismo sitio en cada zoom) y por ids el anidado es **0 % de z8 a z12**
           (`piramide/cotejo-sin-id-20261026.log`). *Falta*: los polígonos (sin capa en el lago, el catálogo no
           responde) y que la celda converja con la plantilla (paso 8).
        5. **F2·3·2·5 · El descriptor, el disparo y los permisos** (`ore-serve`): `minzoom` 0, el suelo, el tipo y el
           estado de la pirámide en el descriptor; la clave con la pareja instantánea + pirámide (C4-d); la ruta de
           la tesela elige pirámide o crudo por zoom; el disparo de D-F2-18 detrás de un interruptor; la pirámide
           no se sirve a quien tiene filtro de fila. *Vara*: pruebas de `ore-serve` (pareja, disparo idempotente,
           falla cerrado) y, en victor, una instantánea nueva acaba con su pirámide sin tocar nada a mano.
           *Hecho en la rama (2026-10-26), lo de `ore-serve` y el motor; el disparo de verdad espera al paso 4*:
           **la pirámide vive bajo su tabla** (`<tabla>/_ore_piramides/<instantánea>-<receta>.pmtiles`, la
           receta `r1` sube cuando `ore-piramide` cambia lo que construye): así **la credencial que se presta
           para el dataset la lee** (sin una segunda, y el motor sigue sin identidad propia), la recogida de la
           tabla no la toca (sólo mira `metadata/` y `data/`) y la de los datasets se la lleva con su dataset
           (D-F2-15 precisado: «de la `Layer`» es, hasta F2·2, del dataset). En el motor, `POST /v1/piramide`
           dice si está, sus zooms y su manifiesto (y la deja abierta), y la credencial de `fuente.piramide` es
           opcional (la del dataset). En `ore-serve` (`teselas.rs`): el descriptor pregunta al motor; con
           pirámide, `minzoom` 0, `ore.piramide = {estado: lista, minzoom, maxzoom, tipo}` y `ore.suelo`; sin
           ella, el suelo y `estado: ausente` (o `construyendo` con `ORE_PIRAMIDE_DISPARO=1`, apagado: el Job
           es el paso 4); la clave lleva la pareja; la tesela lleva `fuente.piramide` sólo por debajo del suelo;
           si `datos` trae `filtro_de_quien_mira` (A8, cuando exista), `estado: no` y sin pirámide. Pruebas:
           `ore-serve` 8 de `teselas` (3 nuevas: el nombre, la pareja y z5 por la pirámide, el filtro),
           `ore-teselas` 25 (el resumen, la credencial por defecto).
        6. **F2·3·2·6 · Retención y salud**: el Job borra las pirámides viejas (todas menos la nueva y la anterior);
           la salud de la capa dice `behind-source` mientras construye y el fallo del Job si lo hay (0063).
           *Vara*: tras dos instantáneas seguidas quedan dos pirámides, y un Job que falla se ve en la salud.
           *Hecho en la rama (2026-10-26)*:
           - **La retención la hace el Job** al acabar bien (`retirar_viejas`). Se queda la nueva, las de
             instantáneas MÁS NUEVAS (otra rama, o una que ganó la carrera: nunca se borra lo de delante) y
             la anterior (la que se servía, por si alguien la tiene abierta). Las demás se borran con su
             manifiesto y su fallo; lo que no es una pirámide no se toca. El orden lo da el número del
             `metadata.json` de la instantánea.
           - **El fallo se ve en el descriptor.** Al fallar, `ore-piramide` deja `<pirámide>.fallo.json`
             (`error`, `cuando`, versión); al acabar bien lo quita. El motor lo devuelve en el 404 de
             `/v1/piramide`, con lo que un `ore-serve` viejo sigue leyendo «no está». El descriptor dice
             `ore.piramide = {estado: fallida, motivo}`, se sirve desde el suelo y no se vuelve a pedir: el
             mismo Job es el mismo fichero de la cola, y se reintenta solo cuando el TTL se lo lleva y Flux
             lo recrea. La consola lo dice («zoomed-out view failed», con el motivo).
           - **Desviación: el estado de la pirámide NO va al bloque de salud de 0063.** La salud de un
             activo se calcula del puntero, sin E/S ni reloj (0063 D-B), y sus datos no están detrás del
             origen: lo que va detrás es la vista de lejos de la capa. Por eso `construyendo` y `fallida`
             viven en el descriptor de la capa (su `Layer` en F2·2), no en `behind-source`.
           - Pruebas: `ore-piramide` 12 (la retención), `ore-teselas` 25 (el fallo) y `ore-serve` 12 (el
             descriptor `fallida`, que no vuelve a pedir).
        7. **F2·3·2·7 · La consola** (`rubix-platform`): fuera el «from zoom N»; el fundido de D-F2-14 y la leyenda
           «densidad» (lo de `ver.html`, con la rampa ajustada a lo que hay en pantalla para las líneas); el estado
           de la pirámide en la página de la capa. *Vara*: tipos y pruebas de la consola, y desplegado.
           *Diseño (2026-10-26), sobre lo que la consola recibe de verdad*:
           - **7·0 · El suelo lo decide quien construye** (backend, hecho en la rama). La consola no pasa
             `minzoom` y ni ella ni `ore-serve` saben el tipo antes de construir, así que una capa de líneas
             tendría suelo 13 y serviría desde z13 el crudo que no cabe. Ahora `ore-serve` pide la pirámide
             sin suelo, y `ore-piramide` lo elige por la primera geometría: 13 para puntos y polígonos, 14
             para líneas; lo dice el manifiesto. Con pirámide `lista`, el suelo del descriptor es su último
             zoom + 1. Pruebas: `ore-piramide` 11 y `ore-serve` 11.
           - **7·1 · Dos fuentes con la misma URL.** Una sola fuente corta en seco: MapLibre pide `floor(zoom)`
             y en 12,9 tiene pirámide y en 13,0 crudo, nunca las dos. Con pirámide, una fuente «lejos»
             (`maxzoom` suelo − 1, que MapLibre agranda por encima) y otra «cerca» (`minzoom` suelo). Cada una
             sólo pide su tramo.
           - **7·2 · Cada tipo, a su manera** (lo medido en `ver.html`):
             - Los puntos: `heatmap` lejos, que se va hacia los círculos aclarados; éstos se funden con los
               del crudo en [suelo, suelo + 0,5].
             - Lo agregado: celdas con rampa sobre `cobertura`, que se van en [suelo, suelo + 0,6] mientras
               entra el crudo. Se reconocen por `has cobertura`. La rampa sale del color de la capa y se
               ajusta a lo que hay en pantalla (p5–p95), una regla para líneas y polígonos.
           - **7·3 · Renovar no quita la capa.** Hoy `pedir` pasa por `cargando` y la fuente desaparece al
             caducar la firma. Con `construyendo`, la consola vuelve a preguntar cada 60 s y el viejo se pinta
             hasta que llega el nuevo.
           - **7·4 · El panel.** Fuera «from zoom N»; en su lugar, el estado de la pirámide y, en las
             agregadas, la leyenda de densidad.
           - **7·5 · Los tipos.** `TileJson.ore` pasa a llevar `suelo` y `piramide`.
           - **La *vara***: `typecheck` y `next build`; la consola no tiene marco de pruebas, así que las piezas
             puras van aparte (`estiloDeCapa.ts`). Lo visual se mide en el paso 8.
        8. **F2·3·2·8 · De punta a punta en victor**: PRs en verde, `ore.dev/commit` del clúster igual a la punta de
           main, la malla aplicada por el aprovisionador, y en la consola de victor las tres capas recorridas de z9 a
           z16: **sin pantalla en blanco, 0 % de parpadeo, la vista < 1 s en frío y < 50 ms con caché**, y nada
           desalojado. Con eso, F2·3·2 hecho.
           *En victor (2026-10-26), sin tocar nada a mano salvo lanzar la convergencia antes de la hora en
           punto*:
           - **Lo que entró, en orden (binario antes que malla):**
             - ORE #33 en `main` (56c4b97b, CI verde) y desplegado (`ore.dev/commit` de `ore-serve` y
               `ore-teselas` igual).
             - La convergencia dejó `plantilla-piramide.txt` en la cola de las 5 celdas.
             - ORE #36 encendió `ORE_PIRAMIDE_DISPARO=1` (d08be36e) y la convergencia lo llevó al
               `ore-serve` de victor.
             - La consola, rubix-platform #17, en producción (Vercel, b137f372).
           - **El disparo, de punta a punta** (`piramide/disparo_victor.py`, `disparo-victor-20261026-1910.log`):
             - Un Job de medida con el agente de la celda pidió los descriptores de las cuatro capas, como la
               consola. Cada uno contestó `construyendo` y encoló su Job (commit en la cola, Flux, Kueue,
               `cola-pesada`).
             - Tres pirámides acabaron en ~4 min (`trip_3m_puntos`, `trip_3m`, `trip_3m_lineas`) y `trip_30m`
               (30 M puntos) en 29 min. Al acabar, el descriptor da `lista` y `minzoom` 0.
             - **0 desalojos y ningún `DiskPressure`.**
             - **Las líneas salieron con suelo 14 solas** (7·0): `ore-piramide` lo eligió por el tipo y el
               descriptor lo tomó de la pirámide.
           - **Las teselas de la pirámide**: 49–128 ms en frío y 5–10 ms con caché, de 21 a 266 KB (las de
             `trip_30m`, las grandes).
           - **Lo que NO pasa: el crudo de estos datos en el suelo.** No lo trae la pirámide; lo destapa
             poder llegar al suelo desde lejos.
             - **Las líneas a z14**: 503, la tesela pesa 2,8 MB y el límite es 2 MB; a z15, 1,4 MB y 1 s en
               frío. Para estas líneas sintéticas el suelo bueno es 15 o más: es el suelo calculado (F2·3·3).
             - **`trip_30m` a z13: `ore-teselas` muere por memoria** (OOMKilled con el límite de 2 GiB) y,
               mientras se reinicia, toda tesela de la celda da 502. La tesela grande se mide después de
               codificarla; antes ya no cabe en memoria. **Es lo primero que arreglar**, antes de dejar
               recorrer `trip_30m` en la consola:
               - el motor tiene que rendirse con 503 antes de reventar (contar lo leído contra un presupuesto
                 de memoria, D-F2-12);
               - y el suelo calculado tiene que subir el de esta capa.
             - **Los puntos**: el crudo a z13 pesa 255 KB y tarda 0,6–1,4 s en frío. El salto de 26 KB (z12,
               aclarados) a 255 KB (z13, todos) es el que ya se había visto: el del suelo calculado.
           - *Falta*:
             - **El recorrido de z9 a z16 en la consola de victor**, en el navegador: pide entrar con una
               persona, y eso no lo hace el agente.
             - **La retención con dos instantáneas seguidas**: el lago no ha tenido una segunda.
             - **Los polígonos**: no hay ninguna capa de polígonos en el lago.
        *Fuera de estos pasos*: el suelo y `aggregate` en la `Layer` de OOS (F2·2), calcular el suelo (F2·3·3) y
        rehacer sólo lo tocado (F4·2).
    - **F2·3·3 · El crudo a escala y la geometría**: la copia de servicio (D-F2-11: subdividida, en orden Hilbert,
      con páginas pequeñas), que cierra z13 en frío; simplificación por debajo del píxel, validez tras recortar,
      reproyección con `geometry_always_xy` y el suelo calculado.
    - **F2·3·4 · 50 a la vez** sobre la misma capa sin que empeore el p95, y la degradación (D-F2-12).
  - *Hecho cuando*: el banco en victor pasa la vara de §2 (frío p95 < 300 ms, caché < 50 ms, primer pintado
    < 1 s, 50 a la vez) con los puntos, las líneas y los edificios.
- F2·4 **C2 fino en ore-serve:** `/v1/layers`, el descriptor TileJSON, la clave opaca, `/v1/tiles` y su caché;
  **los permisos fallan cerrados** (si el predicado no se puede escribir, no sale nada).
- F2·5 **La consola:** el mapa MapLibre carga una `Layer` del catálogo por su descriptor (fuera `mock.ts` para ella).
- F2·6 **Medido** en victor contra la vara y **primera pasada de coherencia** (C1–C3 frente a lo construido).
- *Hecho cuando:* los edificios del banco (34 M de polígonos) se ven y se navegan en la consola de victor.

**F3 · el satélite por el mismo camino**
- F3·1 **El relieve:** GLO-90 al bucket de la plataforma, su envolvente y el lector de COG con caché en el motor.
- F3·2 **El codificador glb** (ENU, relieve, `WorldCRS84Quad`), **el descriptor 3D Tiles 1.1** y **Brotli** en
  MVT y glb.
- F3·3 **El estilo horneado:** el evaluador de la Style Spec en Rust, contrastado con sus pruebas, para `fill`,
  `line`, `circle` y `fill-extrusion`; `unsupported` en el descriptor.
- F3·4 **La consola:** el satélite carga el árbol 3D Tiles (nivel de detalle, renovación, tope de memoria).
- *Hecho cuando:* la misma `Layer`, sin tocarla, se ve en los dos mapas.

**F4 · la escala**
- F4·0 **C4 a fondo** y su **previo**: dónde agrega (DuckDB con H3 en un puesto, como T11, o el motor Rust con
  `h3o`); cómo se poda una celda (columnas de caja `xmin…ymax`, para que las cotas de cada grupo descarten); cómo
  dispara un Build del sistema (0055) al haber instantánea nueva.
- F4·1 **La densidad y la pirámide**, con `aggregate` en la `Layer`: el Build escribe la densidad y sólo los
  niveles necesarios; el descriptor saca de la densidad la disponibilidad del árbol 3D Tiles; el motor elige por tesela y **se
  degrada con elegancia**: si una tesela se pasa de plazo, sale el nivel de agregado de encima, marcado.
- F4·2 **El disparo** (instantánea nueva o `Layer` cambiada → Build; el descriptor cambia de pareja de golpe;
  `behind-source` mientras tanto; un cambio de estilo no construye nada) y **reconstruir sólo lo tocado** (el changelog de Iceberg), como pide §2.
- F4·3 **El lago a escala:** compactar los `append`, **columnas de cobertura** (caja por fila) para que el
  manifiesto descarte ficheros enteros, y **la copia en 3857** de las capas en otro CRS (D-S-10).
- F4·4 **ore-serve con varias réplicas**, sin `Recreate`: sin punto único en el camino del mapa.
- F4·5 **Medido**: **1 000 M de puntos** (SpatialBench a mayor escala o AIS de varias semanas), los 600 M y
  los 34 M de edificios como `Layer`, en los dos mapas, **con 50 a la vez**; **segunda pasada de coherencia**: los
  contratos v1 quedan congelados.

**F5 · completar los contratos**
- F5·1 **Filtros:** `filter` de la `Layer` y el de quien mira (en la clave).
- F5·2 **Etiquetas:** `labels` en la `Layer`; el índice con el nivel de aparición por rejilla, `thin` y `cluster`;
  dentro del MVT y en teselas de etiquetas con marcadores en el satélite. Hecho cuando 10 000 locales de prueba
  nunca pasan del tope por vista, son estables al moverse y todos acaban apareciendo.
- F5·3 **El clic** (`/features`, la fila legible, con su geometría exacta).
- F5·4 **`CREATE OR REPLACE LAYER`.**
- F5·5 **La firma y el borde** (Cloudflare), **`branch=`** y **la retención** (una instantánea expirada da `410` y el
  mapa se recupera solo).
- F5·6 `heatmap` como ráster horneado en el satélite.
- F5·7 **GLO-30** en el relieve: nada enterrado de cerca (T11).
- F5·8 **`vis` real**, en cuanto 0047 decida A8 (la dependencia de fuera, abajo).

**F6 · la consola** (E2·5): el selector de capas, «Guardar como capa» desde el mapa, los estilos, y el aviso de
lo agregado («agregado, de cuántos») y de la salud.

### El índice de la capa: la iteración I (borrador, 2026-10-26)

**Por qué.** El crudo (del suelo hacia arriba) sólo se sirve si la poda descarta: con orden espacial una tesela
cuesta lo que contiene, sin orden cuesta la tabla entera (§E2·0: 87 ms frente a ~640 ms con 20 M; 13,9 s con
600 M; `trip_30m` tumbó `ore-teselas` en el paso 8). Las tablas de los clientes (telco, retail, seguros) son
masivas, vivas, casi siempre particionadas por tiempo, y muchas no son nuestras (foreign databases, Iceberg
ajeno, BigQuery). Reordenar la tabla del cliente se descartó con lo medido en H0 (abajo): obliga a que un
`replace` sea invisible en ~30 sitios, a conservar el row lineage, a un rebase contra el cliente y a un riesgo de
pérdida, y aun así no arregla las particionadas ni las ajenas.

**La decisión: la capa se sirve de un índice suyo, no de la tabla.** Igual que una base de datos responde a un
`SELECT` leyendo un índice: lo que se pinta son las filas de la tabla del cliente en un snapshot, preparadas
para pintar. La tabla del cliente no se toca nunca.

| lo que hace quien mira | de dónde sale |
|---|---|
| lejos (por debajo del suelo) | la pirámide |
| cerca (del suelo hacia arriba) | **el índice de la capa** |
| clic en un elemento (F5·3) | la tabla del cliente, la fila entera por su id |
| una fuente pequeña (cabe en el presupuesto por tesela) | la tabla del cliente, sin índice, como hoy |

#### Las garantías

| | garantía |
|---|---|
| **I-1 · Exacto** | El índice tiene las mismas filas que la fuente en su snapshot. Se comprueba al construirlo (cuenta y suma de ids) y, si no cuadra, no se publica. |
| **I-2 · Coherente** | Se sirve entero o nada. El descriptor empareja índice y pirámide del mismo snapshot de la fuente: lejos y cerca coinciden siempre. |
| **I-3 · Desechable** | Nunca es la verdad: se puede borrar en cualquier momento y se rehace desde la fuente. |
| **I-4 · Invisible** | No sale en el catálogo, ni en la historia, ni en el linaje, ni en las promesas. Lo paga el sistema. |
| **I-5 · Mismos permisos** | Hereda los de la fuente; lleva las columnas del filtro de filas de quien mira, y si el filtro no se puede aplicar no sale nada. |
| **I-6 · Acotado** | Profundidad ≤ 5 tras compactar: una tesela lee del orden de lo que contiene. |
| **I-7 · Dice de cuándo es** | El descriptor dice qué snapshot de la fuente se pinta; si va por detrás, la consola lo marca («datos de las hh:mm»). Nunca mezcla dos momentos. |

#### De qué consta

- **Las columnas**: el id estable de la fila; la geometría en WKB y **ya en 3857** (reproyectar por tesela
  costaba el doble, T1), partida en trozos si es grande (una línea que cruza la ciudad no puede tener una caja
  que lo toque todo, D-F2-11), con el número de trozo; su caja (`_bbox`); las propiedades que usa la capa
  (estilo, etiquetas, filtro); las columnas del filtro de quien mira; la de tiempo, si la hay.
- **Por dentro**: una tabla Iceberg del sistema; Hilbert sobre una rejilla congelada al nacer (2³² por eje, en
  la extensión de los datos con margen); grupos de 100 000 filas; compactación por niveles (L0 lo recién
  llegado, cada nivel ordenado y sin solaparse, razón 10); las cotas de la caja en el manifiesto, para descartar
  ficheros sin abrirlos. No sigue la partición de la fuente: una tabla particionada por día queda ordenada por
  espacio entera (profundidad ~4–5 en vez de ≥ días).
- **La receta** (`i1`, como `r1` en la pirámide): cambiarla rehace los índices.

#### El manifiesto de la capa

Un único fichero por fuente y receta dice qué se sirve: `{fuente, snapshot_fuente, indice: metadata_location,
piramide, suelo, profundidad, rejilla, filas, suma_de_ids, creado}`. Se sustituye entero (compare-and-set por
generación): el descriptor lee sólo esto, y cambiar de snapshot es cambiar de manifiesto de golpe.
- Fuente del lago: bajo la tabla (`<tabla>/_ore_capa/`, junto a `_ore_piramides/`).
- Foreign database: en una ruta del sistema con el id de la fuente.

#### El ciclo de vida

- **Nace al usarse.** El descriptor que hoy encola la pirámide encola el Job de la capa: lee la fuente una vez
  y saca la pirámide y el índice (orden completo por rangos de claves en paralelo, memoria acotada), comprueba
  I-1 y publica el manifiesto. Una fuente que cabe en el presupuesto por tesela no tiene índice.
- **Se pone al día por lo que cambió** en la fuente:

  | cambio en la fuente | qué hace el índice | retraso objetivo |
  |---|---|---|
  | `append` | sólo los ficheros nuevos (Iceberg los dice entre dos snapshots) → L0 | minutos |
  | `merge` o borrados | quita esas filas por id con deletion vectors; lo nuevo, a L0 | minutos |
  | `overwrite` total o cambio de esquema | se rehace; el anterior se sirve mientras tanto | lo que tarde la construcción |
  | foreign database | por marca de agua (columna de fecha o id), CDC donde lo haya; si no, re-lectura entera | según la fuente |
  | retención de la fuente (días viejos fuera) | deletion vectors por la columna de tiempo; la compactación los barre | minutos |

  Las tablas del lago avisan en su commit (`Lago::confirmar`: todo escritor pasa por ahí, H0·C1). Las ajenas se
  comprueban al pedir el descriptor.
- **Se compacta** cuando la profundidad pasa de 5 (niveles: el coste va con lo que entra, no con el tamaño).
- **Muere** si nadie lo pide en N días, o si la fuente desaparece.

#### Los pasos (borrador)

| paso | qué | hecho cuando |
|---|---|---|
| **I0 · previo** | Lo que falta por saber: (a) el id estable de cada tipo de fuente (tablas del lago, Postgres con y sin clave, BigQuery) y qué hacer sin él; (b) qué cambió entre dos snapshots con `iceberg-rust` (ficheros añadidos y retirados); (c) cuánto cuesta partir geometrías grandes; (d) la construcción entera por rangos sobre los 600 M de SpatialBench en victor, una vez | las cuatro respuestas aquí |
| **I1 · el contrato** | La receta, las columnas, el manifiesto de la capa con su compare-and-set y la comprobación de I-1, sin construir nada | pruebas: un manifiesto que no cuadra no se publica; dos escritores a la vez, gana uno |
| **I2 · construirlo entero** | El Job de la capa saca índice y pirámide de una lectura: orden por rangos, 3857, trozos, cotas en el manifiesto, L1 | en el laboratorio, I-1 cuadra y la profundidad es ≤ 2 |
| **I3 · servirlo** | El descriptor lee el manifiesto; `ore-teselas` descarta ficheros por el manifiesto y luego grupos; el clic va a la fuente; la marca de desfase | una capa sin índice sigue igual; con índice, la tesela lee lo que contiene |
| **I4 · ponerse al día** | `append` → L0, borrados por id, `overwrite` → rehacer; el aviso desde el commit | un `append` sale en el mapa en minutos y nunca mezcla snapshots |
| **I5 · compactar y morir** | Niveles con profundidad ≤ 5; retirar por desuso; apagado por defecto (binario antes que malla) | 30 `append` seguidos dejan profundidad ≤ 5 |
| **I6 · las foreign databases** | Marca de agua o CDC, y re-lectura entera como último recurso | una tabla de Postgres se pinta y se pone al día |
| **I7 · medido en victor** | `trip_30m`, los 600 M y una foreign database como capas, recorridas en la consola | la vara de §2 en el crudo, nada visible para el cliente salvo la marca de desfase |
| **I8 · el tope de memoria** (D-F2-12) | `ore-teselas` cuenta lo que va a leer y contesta 503 con el motivo antes de reventar | una fuente sin índice y desordenada no tumba el motor |

#### I0 · previo (2026-10-26)

**Lo que aporta al producto.** I0 no construye nada: despeja lo que haría tropezar a I1–I8. De sus cuatro
respuestas salen la identidad de cada elemento del mapa (lo que el clic abre y lo que se borra al cambiar la
fuente), cómo se entera el índice de lo que cambió, qué geometrías se parten y cuánto cuesta construir un índice
de verdad. Con ellas, la promesa de I-1 («lo que se pinta es exactamente tu tabla en el snapshot X») se puede
cumplir para cualquier fuente, y no sólo para las que tienen clave.

- **(a) · La identidad de cada fila: la de la fuente si la tiene, y si no, la del fichero.**
  - **Hoy no hay una identidad común.** Tienen clave `Entity.primaryKey`, `ore.clave` (los upsert), Postgres
    (PK, índices únicos, replica identity), MongoDB (`_id`), Cassandra, Phoenix y HBase. BigQuery declara PK
    pero no la hace cumplir, y ORE no comprueba que una clave sea única en los datos. **No tienen nada**: los
    datasets del lago escritos con `append` u `overwrite` (casi todos), lo que escriben Spark, PyIceberg o
    DuckDB sin `ore.clave`, los ficheros de objetos, y Postgres sin PK. El row lineage de Iceberg (`_row_id`) no
    se puede usar (H0·A2), y ORE no lee los `identifier-field-ids`.
  - **De paso:** el motor sólo acepta ids enteros, y `ore-serve` no le pasa ninguno: hoy las teselas salen sin id
    de elemento. El clic (`/features`) no está hecho.
  - **La decisión propuesta, por capas:**
    1. **Fuente del lago (Iceberg): la identidad es la posición, `(fichero, fila)`.** Es la de Iceberg: un
       borrado por posición la nombra tal cual, y un `merge` o un `overwrite` retiran ficheros enteros. El
       índice guarda de cada fila su fichero y su posición, así que ponerse al día es exacto sin ninguna clave
       semántica: fichero retirado → sus filas fuera; fichero nuevo → sus filas dentro; borrado por posición →
       esa fila fuera. El clic lee esa fila de ese fichero (un grupo y una página).
    2. **Foreign database con clave** (PK, `_id`, clave de partición): la clave. El protocolo del driver ya
       sabe traer filas por clave por lotes (`claveColumnas`/`claves`), que es lo que el clic necesita.
    3. **Sin clave ni posición estable** (Postgres sin PK, ficheros de objetos que cambian): no hay forma
       exacta de ponerse al día por partes. Al cambiar la fuente, el índice se rehace entero, y el clic va por
       la posición dentro de ese snapshot.
  - **El id del elemento en la tesela** es un `u64` sacado de la identidad, sea cual sea su tipo. La identidad
    original va en el índice para el clic. La «suma de ids» de I-1 es la suma de esos `u64` (con
    desbordamiento): vale para cualquier tipo y con duplicados.
- **(b) · Qué cambió entre dos snapshots: se hace con lo que trae `iceberg-rust`.** No hay lectura incremental
  hecha, pero están las piezas: `parent_snapshot_id` para recorrer de un snapshot al otro, el
  `added_snapshot_id` de cada manifiesto (se saltan los que no son del tramo) y el `status` y `snapshot_id` de
  cada entrada (añadido o retirado en ese snapshot). Salen los ficheros de datos añadidos y retirados y los
  borrados por posición nuevos. Si un snapshot del tramo ya expiró, no se puede saber y el índice se rehace.
  `ore-store` ya recorre manifiestos así (`ficheros_vivos`), así que es una función nueva junto a ella.
- **(c) · Partir las geometrías: las líneas largas sí, los polígonos no** (`i0/trozos.py`, GEOS, rejilla de
  z14 de 2 446 m):

  | datos | se parten (más de 1 celda / más de 4) | filas del índice | caja que mira una tesela |
  |---|---|---|---|
  | líneas de viaje (mediana 3 km) | 55 % / 5,5 % | 2,8× / 1,45× | 4× / 1,7× menos |
  | carreteras (~50 km) | 100 % | 26× | **17× menos** |
  | edificios | 0 % | 1× | igual |
  | áreas de 3–25 km | 100 % | **181×** | 1,8× menos |

  - **Una línea larga sí se parte:** su caja lo toca todo, y partida por celdas la tesela mira 17× menos.
  - **Un polígono grande no:** de verdad ocupa su caja (partirlo multiplica las filas por 181 para mirar 1,8×
    menos), y de él se encargan la pirámide y el recorte por tesela.
  - **La regla propuesta:** se parten las líneas cuya caja pasa de 4 celdas del suelo. Los polígonos, sólo si
    son muy huecos para su caja (un río, una costa: área / área de la caja < 0,1).
  - **El coste:** 0,5 µs por geometría que no se parte; 2,4 ms por carretera de 50 km en Python con GEOS. En
    Rust (`geo`) será menos, y sólo lo pagan las líneas largas.
- **(d) · Construir un índice entero: 240 M filas en menos de 2 minutos** (`i0/construir.py` en un nodo pesado de
  victor, 8 vCPU, DuckDB: los viajes de SpatialBench sf100 desde GCS → 3857, caja, Hilbert sobre la extensión,
  ordenar fuera de memoria, Parquet con grupos de 100 000):

  | filas | extensión (1.ª pasada) | índice (ordenar y escribir) | filas/s del índice | escrito | disco de pico | profundidad |
  |---|---|---|---|---|---|---|
  | 60 M | 5,9 s | 20,1 s | 3,0 M | 3,2 GB | 2,4 GB | 2,7 |
  | 240 M | 22,1 s | 89,8 s | 2,7 M | 12,7 GB | 12,7 GB | 2,4 |

  - **Escala casi lineal** (×4 filas, ×4,3 de tiempo), a ~2,1 M filas/s contando la pasada de la extensión: 600 M
    en ~5 min y 20 000 M en ~2,6 h en un nodo (~21 vCPU·h). I-1 se comprueba al final: las filas y la suma de
    ids cuadran.
  - **El disco manda, no la memoria**: el índice ocupa ~53 B por punto. Con un nodo de 48 GiB de disco, cada
    trabajador construye hasta ~750 M filas. Una tabla de 20 000 M son ~27 rangos de claves en paralelo, cada
    uno con sus ficheros, que no se pisan.
  - **Profundidad 2,4–2,7 con una sola pasada global**: es el orden de verdad que la copia por tramos de hoy no
    da (H0·B1).
  - **DuckDB construye ~3× más rápido que nuestro escritor** (0,81 M filas/s, 0067 V3), con ordenación fuera de
    memoria ya hecha y probada. **Propuesta para I2**: DuckDB ordena y escribe los Parquet del índice, y
    `ore-store` los registra en la tabla Iceberg del índice (`lago_geo::data_file` desde los pies, con las cotas
    de la caja) y comprueba I-1. Cada pieza hace lo que mejor sabe hacer.
  - Limpio: el Job y su disco se borran al acabar.

**Lo que I0 cambia en el plan:**
- **I1** fija la identidad por capas (posición en el lago, clave fuera, re-construir sin ninguna) y el id `u64`
  de la tesela.
- **I2** se construye con DuckDB más `ore-store`, por rangos de claves cuando la tabla no cabe en un nodo, y
  aplica la regla de partir.
- **I3** pasa el id al motor (hoy no lo recibe) y abre el clic por la identidad.
- **I4** se pone al día con la diferencia de manifiestos: ficheros retirados, añadidos y borrados por posición.

#### I1 · el contrato (diseño, 2026-10-26)

**Lo que aporta al producto.** I1 es la forma del índice antes de que exista ninguno: qué columnas lleva, cómo se
nombra cada elemento del mapa, qué fichero dice qué se sirve y cuándo un índice no se publica. Es la pieza que
hace cumplir las garantías por construcción y no por disciplina: un índice que no cuadra con su fuente (I-1) o una
pareja índice–pirámide de dos momentos distintos (I-2) no se pueden publicar, porque el tipo no lo permite.
I2–I6 sólo rellenan este contrato. Por eso va primero, va entero en código puro y se prueba en segundos, sin
nube.

**Dónde vive.** Un módulo `indice.rs` en `ore-teselas`, junto a `piramide.rs`. Ya depende de `ore-store` (el
`Almacen`), lo usa el Job (`ore-piramide`, que pasará a ser el Job de la capa) y lo sirve el motor. `ore-serve`
no gana dependencias: igual que con la pirámide (`POST /v1/piramide`), preguntará al motor (I3).

**Las seis piezas:**

1. **La receta `i1`**: todo lo que decide cómo sale el índice, en un solo sitio. Cambiar algo es otra receta
   y otra ubicación, nunca reescribir un índice en su sitio.
   - 3857, grupos de 100 000 filas, rejilla de 2³² por eje sobre la extensión con un 10 % de margen.
   - Se parten las líneas que pasan de 4 celdas del suelo y los polígonos con área / área de la caja < 0,1.
   - Profundidad máxima 5 antes de compactar.
2. **La identidad** (I0·a), un tipo con tres casos:
   - `Posicion { fichero, fila }` en el lago. `fichero` es un ordinal en la tabla de ficheros del índice, y el
     id de la tesela es exacto, sin hash: `ordinal << 40 | fila` (hasta 16 M ficheros de hasta 10¹² filas).
   - `Clave(valores)` fuera del lago. El id es un hash de 64 bits de la forma canónica de la clave. Con 20 000 M
     filas se esperan ~10 colisiones: sólo afectan al resaltado en el mapa, porque el clic usa la clave
     guardada, no el hash.
   - `Instantanea { fila }` sin nada estable: vale dentro de su snapshot, y al cambiar la fuente el índice se
     rehace.
3. **Las columnas del índice**, fijas por la receta:

   | columna | |
   |---|---|
   | `_ore_id` `int64` | el id del elemento en la tesela (el `u64` con sus bits tal cual: Iceberg no tiene enteros sin signo) |
   | `_ore_fichero` `int32` + `_ore_fila` `int64` | la posición (`Posicion`) |
   | `_ore_clave` `string` | la clave canónica (`Clave`) |
   | `_ore_trozo` `int32` | el número de trozo (0 si no se partió) |
   | `geom` (WKB en 3857) + `geom_bbox` | la geometría y su caja |
   | las de la capa, las del filtro de quien mira, la de tiempo | con su nombre en la fuente |

4. **El manifiesto de la capa**, la única verdad sobre qué se sirve:
   `{receta, generacion, fuente: {tipo, nombre, ubicacion}, snapshot_fuente, secuencia_fuente, identidad, indice: metadata_location,
   ficheros: <clave del objeto con la tabla de ordinales>, piramide: {clave, snapshot_fuente}, suelo, rejilla,
   profundidad, filas, suma_ids, creado}`.
   La tabla de ficheros va aparte, porque una fuente grande tiene decenas de miles.
5. **Publicar sin carreras, con lo que el `Almacen` ya sabe.** El `Almacen` sólo crea si no existe (`subir`:
   `ifGenerationMatch=0` en GCS, igual en S3/R2). No hace falta un verbo nuevo:
   - cada generación es un objeto que no se reescribe nunca, nombrado al revés
     (`manifiesto-<u64::MAX − generación>.json`);
   - **el vigente** es `primera(prefijo)`: una sola petición;
   - **publicar** es crear la generación siguiente. Si ya existe, otro ganó: se lee la suya y se decide (si es
     de un snapshot más nuevo, se abandona; si no, se reintenta encima);
   - **retirar**: se quedan las tres últimas generaciones, y con ellas sus índices; lo demás se borra.
6. **La puerta de publicación**: `publicar(manifiesto, comprobacion)` exige una `Comprobacion` que sólo se
   puede construir así:
   - **I-1:** las filas y la suma de ids de la fuente, contadas por el Job al leerla, iguales a las del índice
     escrito;
   - **I-2:** la pirámide es del mismo `snapshot_fuente`.

   Si no cuadra, el error dice qué no cuadró y no se publica nada.

**Las pruebas** (`cargo test -p ore-teselas`, con un `Almacen` en memoria; ninguna toca la nube):
- un manifiesto ida y vuelta, igual byte a byte;
- un índice con una fila de menos, o con otra suma, no se publica (I-1);
- una pirámide de otro snapshot no se empareja (I-2);
- dos publicadores a la vez: gana uno, el otro lee la generación ganadora y se retira si es más nueva;
- el vigente sale de una sola petición, y la retirada deja tres generaciones;
- el id de una `Posicion` es exacto y reversible; el de una `Clave`, estable entre ejecuciones y con
  independencia del orden de columnas;
- otra receta, otra ubicación: dos recetas conviven sin pisarse.

**Las tres iteraciones** (cada una compila, pasa sus pruebas y es un commit; ninguna toca la nube):

| iteración | qué | sus pruebas |
|---|---|---|
| **I1·1 · los tipos** | La receta `i1`, la identidad y sus ids, el esquema Arrow de las columnas del índice y el manifiesto con su ida y vuelta. Sin `Almacen` | ids exactos y reversibles (`Posicion`) o estables (`Clave`); el manifiesto va y vuelve igual; dos recetas, dos ubicaciones |
| **I1·2 · la puerta** | `Comprobacion` (I-1: filas y suma de ids; I-2: el snapshot de la pirámide), que sólo se construye si cuadra, con errores que dicen qué falló. Sin `Almacen` | una fila de menos, otra suma o una pirámide de otro snapshot: no hay `Comprobacion` |
| **I1·3 · publicar** | Las generaciones en el `Almacen`: crear la siguiente sólo si no existe, el vigente con `primera`, el abandono si otro ganó con un snapshot más nuevo, la retirada a tres. `publicar` exige la `Comprobacion` de I1·2 | con un `Almacen` en memoria: dos a la vez gana uno; el vigente en una petición; quedan tres generaciones |

*I1·1 hecho (2026-10-26)*: [`ore-teselas/src/indice.rs`](../../crates/ore-teselas/src/indice.rs) con la receta `i1`, la
rejilla congelada y su clave Hilbert de 64 bits, `TipoDeIdentidad`/`Identidad` con sus ids, el esquema del índice
y el `Manifiesto` (JSON indentado, decimales como cadena). 9 pruebas, `ore-teselas` 33/33 y clippy limpio.
- **El hash de una clave queda fijado** en una prueba (`["42"]` → `0xd646f2fabdd13a7e`): si cambiara, cambiarían
  todos los ids publicados.
- **Una frontera de la curva, vista en una prueba:** dos puntos a un milímetro, justo en el centro de la
  rejilla, quedan lejos en la curva, porque Hilbert salta de cuadrante. Es la curva y no un fallo, y lo absorben
  los grupos (una tesela que cruza la frontera toca dos tramos de la curva, no la tabla). La medida de
  profundidad de I0·d ya lo incluye.

*I1·2 hecho (2026-10-27)*: la puerta, en el mismo módulo.
- `Cuenta` cuenta filas y suma de ids (con desbordamiento). No depende del orden y se junta entre trabajadores;
  del índice cuenta sólo el trozo 0 de cada fila.
- `comprobar(snapshot, fuente, índice, pirámide)` devuelve la `Comprobacion` o **todos** los `Descuadre` a la
  vez: filas, suma de ids (las mismas filas pero otras, incluida una repetida con otra perdida) y la pirámide de
  otro snapshot.
- `Comprobacion` tiene los campos privados, y `cubre(manifiesto)` exige que el manifiesto diga lo comprobado:
  otro snapshot, otras filas o una pirámide no comprobada no pasan.
- 3 pruebas más; `ore-teselas` 36/36 y clippy limpio.

*I1·3 hecho (2026-10-27): **I1 cerrado.***
- `publicar(almacén, prefijo, manifiesto, comprobación)` crea la generación siguiente sólo si no existe, y
  devuelve `Hecho` o `Superado`. Si otro crea la misma generación a la vez, vuelve a mirar el vigente.
- **Quién gana una carrera lo dice la fuente, no el reloj.** El manifiesto lleva `secuencia_fuente` (el número
  de secuencia de Iceberg; fuera del lago, la marca de agua). Un índice de un snapshot igual o más viejo que el
  vigente no se publica nunca.
- `vigente` hace dos peticiones: `primera` para saber cuál y `leer` para traerlo.
- `retirar(…, 3)` borra los manifiestos viejos y la tabla de su índice, salvo que la use uno de los que quedan
  (una puesta al día de I4 añade a la misma tabla).
- 5 pruebas con un almacén en memoria que sólo crea y puede meter un rival a mitad de la subida;
  `ore-teselas` 41/41 y clippy limpio.

*I2·1 hecho (2026-10-27)*: registrar en una tabla nueva los Parquet que otro escribió.
- **`Lago::registrar(ubicación, nombre, esquema Arrow, ficheros)`** crea la tabla del índice y su primer snapshot
  con ficheros ya escritos. Sólo acepta ficheros bajo su ubicación, con el id de campo de la tabla en cada
  columna (Iceberg lee por id: un Parquet de DuckDB sin ids se leería vacío en Spark), y Parquet de verdad (se
  lee sólo el pie). Con geometría, la tabla nace v3 con `ore.geo.<col>`.
- **`Lago::esquema_de_tabla_nueva`** da los ids que Iceberg reparte al crear: los que DuckDB pondrá con
  `FIELD_IDS` (I2·2). `Lago::crear_en` crea la tabla fuera de la raíz de los datasets.
- **El snapshot se apila con el código de siempre:** el tramo de `preparar` que monta manifiestos, lista y
  snapshot pasó a `Lago::apilar`, compartido por escribir y registrar (sin copiarlo). Las 85 pruebas que ya había
  de `ore-store` pasan igual.
- **Las cotas de la caja van al manifiesto (H0·C2)**, en `lago_geo::data_file`. Esto vale también para toda tabla
  geo que escriba ORE desde ahora. Son conservadoras: si un grupo con valores no trae estadísticas, ese campo se
  queda sin cota, porque una cota de sólo algunos grupos descartaría filas que están.
- **Un fallo de I1·1, cazado aquí:** el esquema usaba `UInt64`, `UInt32` y `UInt16`, e Iceberg no tiene enteros
  sin signo. Pasan a `Int64`/`Int32`, y el id `u64` va con sus bits tal cual. Una prueba en `ore-teselas` crea ya
  la tabla Iceberg con el esquema de I1.
- `ore-store` 87/87, `ore-teselas` 41/41 y clippy limpio en los dos.

**Lo que I1 no hace** (es de los pasos siguientes): construir (I2), servir (I3), ponerse al día (I4).

**Hecho cuando**: las pruebas pasan, y el contrato está escrito aquí tal como queda en el código.

#### Lo medido para decidirlo (H0, 2026-10-26)

Antes del índice se pensó en reordenar la tabla del cliente en su sitio (la «iteración H»). H0 lo midió, y lo que
sigue es lo que llevó a descartarlo (A) y lo que se queda para el índice (B y C).

**A · ¿Se podía reordenar bien la tabla del cliente? (leyendo el código)**
- **A1 · El `replace`: no viene hecho, se construye con piezas que sí hay.** `iceberg-rust` 0.10 sólo trae
  `fast_append`. ORE ya hace sus propios snapshots (`Append` y `Overwrite`, [lago.rs](../../crates/ore-store/src/lago.rs)
  `instantanea`) con piezas públicas, y un `replace` es lo mismo: los ficheros nuevos `ADDED` (con la secuencia
  de datos que se elija), los sustituidos `DELETED` y el resto `EXISTING` (`add_existing_file`). Falta:
  - una variante `Reemplazar` con sus totales (hoy sólo `Anexar | Sobrescribir`);
  - **el rebase**: hoy el commit exige `main` en el snapshot exacto (`RefSnapshotIdMatch`). Eso ya hace que el
    cliente gane (un `replace` a la vez que una escritura se rechaza), pero no hay reintento: releer, comprobar
    que los ficheros sustituidos siguen vivos y sin borrados nuevos, y apilar encima. Es nuestro.
  - De paso, **C1 queda respondida: sí.** Todo escritor confirma por el mismo sitio: la carga, los SDK, y
    Spark, PyIceberg y DuckDB por REST (`ciclo.rs` `aplicar` → `Lago::confirmar`). La profundidad se calcula ahí.
- **A2 · Row lineage y borrados: no se conservan, y hoy nadie los usa.**
  - `iceberg-rust` no lee ni escribe `_row_id` ni `_last_updated_sequence_number`, y ORE tampoco. Un
    `replace` heredaría `first_row_id` nuevos: para un motor externo con linaje v3, todas las filas
    «nuevas». Como el escritor geo escribe sus Parquet (`lago_geo`), **materializar las dos columnas**
    (ids reservados 2147483540 y 2147483539) en los ficheros reescritos es barato: dos `int64`.
  - ORE no escribe borrados; lee los de posición de otros motores y niega los de igualdad. La primera versión
    reescribe sólo ficheros sin borrados (o aplica los de posición y retira los que sólo apuntaban a ellos);
    con borrados de igualdad, la tabla no se reordena. La secuencia de datos de lo reescrito, la mínima de lo
    sustituido (como `RewriteFiles` de Iceberg), para que los borrados de igualdad sigan valiendo.
  - *Fallo de paso*: un `Anexar` escribe `total-delete-files: 0` aunque la tabla tenga borrados de Spark.
- **A3 · Qué se mueve hoy con un snapshot nuevo: ~30 sitios.** Nada filtra `Operation::Replace`. Lo grave:
  - **La cascada de copias**: una copia está al día si el `metadata_location` de su entrada no cambió
    (`materializar.rs` `al_dia_con`). Un `replace` las daría a todas por viejas y se re-copiarían enteras,
    aguas abajo, con sus pirámides. Igual `ore invoke` (llamaría al modelo otra vez por cada fila).
  - **La pirámide y las teselas**: la clave de la pirámide es el nombre del `metadata.json`
    (`teselas.rs` `ubicacion_de_la_piramide`), igual que la del descriptor y la caché del motor: pirámide
    reconstruida entera, mapa vacío por debajo del suelo mientras tanto, cachés del navegador perdidas.
  - **El puntero**: escrito como un commit normal re-juzga las promesas (las de columna pasan a `unknown`),
    borra `version_retenida` y `procedencia`, cambia `escrito_por` (dueño), y en ramas sale en «Assets
    changed» o como conflicto al fusionar.
  - **La historia**: fila nueva en `/historia` y snapshot `replace` en la ficha.
  - **La idempotencia (0031)**: «sin cambios» mira `ore.operacion` del snapshot vigente; tras un `replace`
    vacío, una sobrescritura idéntica vuelve a escribir. Y `ore.cabecera` se perdería.
  - **La expiración** cuenta todo snapshot para el mínimo que se guarda, y el almacenamiento se duplica hasta
    que expira el viejo.
  - No se mueven: executions, Builds (nada los marca viejos solos), la versión de 0056 (sólo `Function`),
    `derivedFrom` y las estadísticas.
  - **Hay un precedente que hace justo lo que queremos**: `recoger` mueve sólo el `metadata_location` del
    puntero (`datasets.rs`), sin re-juzgar ni regenerar nada.

**B y C · la escala y lo que se ve (código + simulación local).** Lo de B2 y B4 es una propiedad del orden y no
del hierro: se simula en local (`pruebas-de-fuego/geo/h0/rejilla.py` y `amplificacion.py`, con sus registros).
B1 sale de lo ya medido.
- **C2 · Las cajas no están en los manifiestos.** El escritor de ORE no pone cotas por fichero
  (`lago_geo::data_file`, «sin cotas inferior y superior»); Spark (Iceberg Java) sí las pone a los campos
  anidados. Y el motor no mira manifiestos: abre el pie de **cada** fichero vivo y poda por grupo. Con una tabla
  de cliente (20 000 M filas ≈ 15 000–20 000 ficheros) una tesela en frío son miles de pies. Por tanto:
  - el escritor pone las cotas de los cuatro campos de `<col>_bbox` en el `DataFile` (ya tiene los pies en
    la mano);
  - la profundidad se calcula con el manifiesto, sin abrir ficheros;
  - el motor descarta ficheros por el manifiesto antes de abrir pies (con el tope de H6).
- **B1 · Coste de ordenar.** El escritor en Rust da 0,81 M filas/s por proceso con Hilbert (0067 V3), y Spark
  ~1 M filas/s en 8 vCPU (F1·2). **20 000 M filas ≈ 7 h·proceso**; con 16 trabajadores, media hora.
  - **El hallazgo: hoy una tabla masiva no sale ordenada de la copia.** El escritor ordena por tramos de
    ~512 MB de Arrow (5–10 M filas). Cada tramo cubre toda la extensión, así que 20 000 M filas son miles de
    tramos que se pisan, y con 50 tramos la poda ya es la de una tabla sin orden (B2).
  - El orden completo de una tabla grande lo da el Job, por rangos: muestrear claves, partir el espacio de
    claves en rangos y que cada trabajador escriba el suyo (ficheros que no se pisan, memoria acotada).
- **B2 · La rejilla: congelada por tabla, no fija sobre el mundo.** 5 M puntos de una ciudad, grupos de 100 000:

  | orden | profundidad | grupos que toca una tesela z14 |
  |---|---|---|
  | sin orden | 50 | 50 de 50 |
  | Hilbert 2¹⁶ sobre la tabla (hoy) | 1,7 | 2 |
  | Hilbert 2¹⁶ o 2³² fija sobre el mundo | 2,7 | 3 |
  | 5 / 10 / 50 tramos ordenados por su cuenta | 10 / 24 / 50 | 10 / 20 / 50 |
  | 3 niveles (90/9/1 %) | 5,9 | 6 |

  - **La resolución no es el problema** (2¹⁶ y 2³² sobre el mundo dan lo mismo). Lo es la alineación: los
    cortes grandes de la curva parten la ciudad, y se lee 1,5× más.
  - **Decisión propuesta:** la rejilla de cada tabla se congela al nacer (su extensión con margen, en
    `ore.geo.rejilla`), con 2³² por eje. Las claves son estables entre escrituras; lo que cae fuera se pega al
    borde.
  - **La profundidad predice lo leído**: correlación de 0,98 (log–log) entre profundidad y filas leídas por
    tesela. Una tanda ordenada por su cuenta suma ~1 a la profundidad.
- **B3 · El particionado por tiempo.**
  - El escritor de ORE escribe siempre sin particionar (`partition` vacía). Las tablas que crea Spark con una
    especificación sí están particionadas, y el escritor geo escribiría en ellas sin partición: un riesgo que
    hay que cerrar.
  - El motor no poda por partición. Y aunque lo hiciera, **una tabla particionada por día y ordenada dentro de
    cada partición tiene profundidad ≥ número de particiones** para un mapa de todo el periodo: 365 en un
    año.
  - Reordenar en su sitio no lo arregla, porque la partición la decide el cliente. Para esas tablas hay dos
    salidas: una capa con ventana de tiempo (lo geotemporal), o la copia de servicio en orden espacial
    (D-F2-11, F2·3·3). **Se decide aparte; en H, esas tablas se ordenan por partición y quedan señaladas.**
- **B4 · Reescribir: niveles, no «sólo lo que pisa».** Los datos nuevos de un cliente caen por todo el espacio,
  así que «reescribir los ficheros que pisa» acaba siendo reescribir todo. Con 20 000 M filas y 30 días:

  | patrón | todo al pasar 8 tandas | niveles (razón 10) | escalonado |
  |---|---|---|---|
  | telco, `append` cada hora (120 M/día) | 65 000 M filas/día (23 h·proceso), prof. 8 | **2 400 M/día (1 h·proceso), prof. ~5** | 480 M/día, prof. ~20 |
  | retail, `append` diario (50 M) | 2 600 M/día, prof. 8 | **750 M/día, prof. ~4** | 150 M/día, prof. ~15 |
  | seguros, `merge` diario del 1 % | 2 700 M/día, prof. 8 | **1 500 M/día, prof. ~4** | 300 M/día, prof. ~15 |

  - **Decisión propuesta: compactación por niveles** (L0 = lo recién escrito; cada nivel ordenado y sin
    solaparse por dentro; razón 10). La profundidad queda acotada en ~4–5 y el coste es proporcional a lo que
    entra.
- **B5 · Sin rebase no hay mantenimiento.** Con el requisito de hoy (main en el snapshot exacto):

  | caso | sin rebase | con rebase |
  |---|---|---|
  | trozo de 50 M (4 min) frente a `append` cada minuto | 98 % aborta | 0 % |
  | nivel de 5 000 M (90 min) frente a `append` cada hora | 78 % | 0 % |
  | nivel de 5 000 M frente a un `merge` diario que lo toca todo | 6 % | 6 % (se reintenta) |

  - Con rebase, un `append` nunca choca. Sólo choca lo que reescribe los mismos ficheros. Los commits parciales
    (trozos de minutos) dejan el choque cerca de cero.

### El alcance v1 de cada capa (2026-10-22)

**Sin cabos sueltos.** Cada capa tiene una lista cerrada de lo que es su v1: lo que la vara de §2 exige. Cada
punto está en una fase. **Al cerrar F6, el pegamento v1 y todas las capas v1 están completos.** Lo que no está
en la v1 no queda pendiente: está **fuera de alcance, con su motivo y lo que lo abriría**.

| capa | su v1 (y la fase) | lo que pide de §2 |
|---|---|---|
| **almacenamiento frío** | geometría sin envolver y `append` (F1·1) · Hilbert desde Spark (F1·2) · compactar, cobertura en el manifiesto, copia en 3857 (F4·3) · retención con recuperación (F5·5) | masivo; reproyectar bien |
| **cómputo de lo precalculado** | densidad, pirámide, disparo, pareja completa (F4·1–2) · reconstruir sólo lo tocado (F4·2) · índice de etiquetas (F5·2) | agregados exactos; fresco y gobernado |
| **motor de teselas** | crudo MVT, simplificación y recorte válidos, reproyección, determinismo, límites, réplicas (F2·3) · glb, relieve, estilo horneado, Brotli (F3) · pirámide y degradación elegante (F4·1) · predicado compuesto (F5·1) · etiquetas (F5·2) · `heatmap` (F5·6) | el mapa nunca miente; determinista; se degrada con elegancia |
| **la puerta (ore-serve)** | `/v1/layers`, descriptor, clave, teselas, caché, fallar cerrado (F2·4) · 3D Tiles (F3·2) · varias réplicas (F4·4) · filtro de quien mira, clic, firma, ramas (F5) · `vis` real (F5·8) | permisos; sin punto único; 50 a la vez |
| **transporte y caché** | caché en ore-serve y en el motor (F2) · Brotli (F3·2) · borde con firma (F5·5) | p95 con caché < 50 ms |
| **relieve** | GLO-90, envolvente, lector COG (F3·1) · GLO-30 (F5·7) | nada enterrado |
| **cliente** | MapLibre por TileJSON (F2·5) · satélite por 3D Tiles (F3·4) · marcadores con tope y diferencia (F5·2) · selector, «Guardar como capa», estilos, avisos (F6) | 60 fps; primer pintado < 1 s; el mapa lo dice |
| **la spec (OOS)** | `Layer` fina (F2·2) · `aggregate` (F4·1) · `filter`, `labels`, SQL (F5) | — |

**Fuera de alcance, con su disparador:**

| qué | por qué fuera | lo abre |
|---|---|---|
| **el estado caliente (lo vivo)** | es otra capa con su propio pegamento | **su ADR, E3**: no es un cabo de 0072 |
| la pirámide con dimensiones | §2 se cumple sin ella para la capa con su `filter` | un filtro de quien mira sobre una capa de > 100 M que pase del p95 en frío |
| la pirámide por clase de `vis` | depende de A8 | F5·8, si A8 filtra filas |
| varias fuentes por `Layer`, el kind `Map`, un relieve propio | K3; no lo pide la vara | un caso de producto que lo pida |
| el ráster del cliente, OGC API | E4, E7 | sus líneas en 0066 |
| móvil | el producto nunca se sirve en móvil | — |

**La dependencia de fuera:** A8 (0047) decide cómo se expresa el filtro de fila y columna. No es un cabo suelto
de este plan sino una entrada: F5·8 se hace cuando A8 esté decidida, y si no lo está al llegar, el plan espera ahí
y lo dice. Conviene abrirla en 0047 en paralelo.

**Dos ajustes de coherencia** que salen de cruzar el alcance con §2:
- **C3-b se precisa:** con una pirámide, una tesela que se pasa de plazo **baja un nivel de agregado** (§2, «se
  degrada con elegancia»), nunca `503`; y una `Layer` **no aparece en el descriptor hasta que tiene su primera
  pareja completa** (C4-d), así que ninguna tesela densa espera a una pirámide. El `503` queda para un fallo de
  infraestructura.
- **El filtro de quien mira** sobre una capa densa agrega al vuelo y puede pasar del p95: tiene un plazo más largo
  y el mapa lo dice; la pirámide con dimensiones es lo que lo cerraría (arriba).

**Hecho cuando (el plan entero):** una capa del catálogo de 1 000 M de elementos se añade al mapa, se navega
dentro de los objetivos de §2 con 50 personas a la vez en los dos mapas, y cada una ve sólo lo que puede ver
(con F5·8); y todas las filas de la tabla de alcance están hechas.

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
- **2026-10-21 · E2·0 medido** (§6, «Lo medido»). La poda del lago funciona en el fichero y la pierde el
  SDK al envolver la geometría; Rust sirve 5–9× más teselas con concurrencia que DuckDB; crudo hasta ~50 000
  elementos por tesela; la pirámide de 600 M en 2 minutos; el satélite 3D aguanta ~300 polígonos. De paso, el
  `append` de geometría del SDK falla (`carga.rs:1124`). Propuestas D-S-6 a D-S-11, para aprobar.
  - **Lo que costó:** dos Jobs grandes se quedaron suspendidos por Kueue (la base del Job va a `cola`; los
    pesados van a `cola-pesada` y sin el `nodeSelector` de sistema) y un DuckDB con los 488 MB del SDK.
    Todo retirado: las capas del lago (3,4 GB bajo `medida_e2_*`), el binario del banco y el nodo pesado.
- **2026-10-22 · T11 medido** (§6). El techo de la GPU, ~12–15 M de elementos a la vez; los 600 M de
  SpatialBench navegables sobre el 3D de Google, del globo a los viajes sueltos, a 60 fps; congelado a partir
  de ~39 M cargados. D-S-11 rehecha: modelos glTF teselados, no polígonos sueltos. Deudas: color y entierro de
  los viajes, el clic.
  - **Lo que costó:** el generador se paró al escribir el primer modelo (faltaba `import struct`) y la
    emulación de viewport engañaba al lienzo de Google. Retirado: los datos locales (~10 GB con la clave en
    la página) y la entrada de launch; en el lago no quedaba nada bajo `medida_e2_sb_*`; el pool pesado, a 0.
- **2026-10-22 · D-S-12 propuesta**, con el usuario: un motor, dos codificadores (MVT y glb con relieve,
  3D Tiles) sobre la misma pirámide, caché y puerta. Rehace D-S-3 y responde P1 (el satélite muestra datos
  masivos). Recogido también en el informe «Mapa masivo en tiempo real».
- **2026-10-22 · E2·0 aprobado** por el usuario: D-S-6 a D-S-12. Antes, D-S-5 y D-S-9 rehechas: no hay kind
  `Tileset`; la pirámide de agregados es un dataset derivado (un Build), ordenado por tesela y usado por capa y
  nivel; las teselas codificadas son siempre caché. «Lo que es dato es un dataset; lo que es formato es caché.»
- **2026-10-22 · Los contratos (borrador)**, §6: diseño por contrato primero; C1 el kind `Layer` (OOS, kind
  propio, decidido), C2 la dirección de tesela (inmutable, TileJSON/3D Tiles, tms de OGC), C3 el protocolo del
  motor, C4 la pirámide de agregados. K1–K4 por decidir.
- **2026-10-22 · K1–K4 cerradas.** K1: la MapLibre Style Spec, horneada por el motor en glb; las etiquetas, como
  marcadores elegidos por el servidor (nunca pegadas al suelo), con tope de ~300 por vista hasta medir T12.
- **2026-10-22 · T12 medido.** 300 marcadores con etiqueta e icono a 60 fps sobre 5 M de elementos (1 000 quieto,
  3 000 ya no); Google no evita el solape (lo elige el servidor) y la renovación va por diferencia. Retirado: la
  página con la clave y la entrada de launch.
- **2026-10-22 · El índice de etiquetas** (C4): nivel de aparición por densidad, monótono, desempate
  determinista, aclarar o agrupar.
- **2026-10-22 · El relieve:** infraestructura de la plataforma, no dato del cliente; COG de Copernicus en un
  bucket de la plataforma con su envolvente, versión en la dirección glb, atribución en el descriptor.
- **2026-10-22 · C1 diseñado a fondo** (§6, «Los contratos»): naturaleza, anatomía, claves, reglas, gobierno;
  C1-a a C1-e por aprobar.
- **2026-10-22 · C1 aprobado** (C1-a a C1-e).
- **2026-10-22 · Cómo se llega a una `Layer`:** mapa, YAML y `CREATE OR REPLACE LAYER`, los tres en la v1.
- **2026-10-22 · C2 diseñado** a nivel de producto: descriptor y tesela, clave opaca, TileJSON y 3D Tiles 1.1, firma, etiquetas, clic, frescura; C2-a a C2-f por aprobar.
- **2026-10-22 · C2 aprobado (C2-a a C2-f) y el plan rehecho**: seis fases que atraviesan las capas (F1 sustrato, F2 esqueleto andante, F3 satélite, F4 escala, F5 completar contratos, F6 consola); C3 y C4 se definen a fondo antes de su primer uso; dos pasadas de coherencia (F2·6 y F4·2).
- **2026-10-22 · C3 diseñado** a nivel de producto (C3-a a C3-e por aprobar); hojas de ruta acotadas en C1, C2 y C3.
- **2026-10-22 · C3 aprobado** (C3-a a C3-e) y cómo se construye (tipos en `ore-core`, binario `ore-teselas` desde el prototipo, cliente en ore-serve, malla como ore-motor).
- **2026-10-22 · C4 diseñado** a nivel de producto: densidad, pirámide e índice de etiquetas; Hilbert sin claves de tesela (cambia el borrador); C4-a a C4-f por aprobar.
- **2026-10-22 · C4 aprobado** (C4-a a C4-f).
- **2026-10-22 · El alcance v1 de cada capa**: lista cerrada por capa contra §2, repartida en F1–F6 (entran Hilbert en Spark, compactar, cobertura, 3857, réplicas de ore-serve, reconstruir sólo lo tocado, GLO-30, retención); fuera de alcance con disparador; A8 como dependencia; C3-b precisado.
- **2026-10-22 · Limpio:** una sola numeración (las fases F1–F6); cada contrato dice en una línea dónde se construye; fuera los pasos C4·x (pasados a F4 y F5); la matriz de los dos ejes al principio del plan.
- **2026-10-22 · F1·0 medido:** la poda por columnas de cobertura (~660 → ~126 ms, aprobado); el `append` por `Binary`/`LargeBinary`; Spark con cobertura + Hilbert + grupos de ~100 000 filas (58× menos filas que hoy). F1·1–F1·3 rehechos.
- **2026-10-22 · F1 en main y F1·3 medido; F2·0 hecho (el HIT del borde, deuda para F2·4):** la poda por `geom_bbox` vale para el SDK y para Spark (Madrid z14: 107 frente a 187 ms); el motor de laboratorio hace polígonos correctos (como `ST_AsMVT`), el suelo del crudo de los edificios es z13, el salto a `ore-serve` no cuenta y MapLibre carga la tesela firmada por la entrada real. Lo que el previo cambia: filtrar por fila antes de descodificar, reparar tras la rejilla y el grupo por bytes (D-F2-7).
- **2026-10-25 · F2·3 rehecho con el banco de victor** (`banco.py`, `sonda_gcs.py`): el tiempo es espera de GCS (~40 GET en serie por tesela, ~55 ms cada uno; el motor a 61 m de CPU), no CPU; las líneas al azar son densidad, no índice. D-F2-8 a D-F2-12 por aprobar: dos oleadas por tesela, la espera separada de la CPU, caché de páginas, la copia de servicio subdividida en Hilbert (cuarta pieza de C4) y degradar en vez de caer.
- **2026-10-25 · F2·3·2 rehecho: la capa a cualquier zoom** (prueba de concepto en `pruebas-de-fuego/geo/f2/piramide/`, tippecanoe + PMTiles en victor): los puntos se aclaran (3 M en 6 min, tesela ≤ 67 KB, 0 % de parpadeo); los polígonos y las líneas se agregan en rejilla (1,33 M edificios en 40 s, 3 M líneas en 5,5 min, ≤ 27 KB), porque aclararlos no acaba o llena el tope; servida desde GCS con el directorio en memoria, la vista entera en 75–380 ms. D-F2-13 a D-F2-18 por aprobar (la pirámide, de la Layer y no un dataset: revisa C4-a y C4-e; ore-piramide, binario Rust + tippecanoe en imagen propia, como Job del sistema al estilo de la copia; el descriptor dispara), con ocho pasos de despliegue; la copia de servicio del crudo pasa a F2·3·3.
- **2026-10-26 · F2·3·2·4 hecho en la rama: el Job del sistema** (`malla/59-la-piramide.yaml`, `cola-pesada`, la cuenta `driver`, una ranura por tabla; el disparo del descriptor la encola, apagado en la malla; en seco si ya está): en victor, las dos capas en 259 s, sin desalojos ni `DiskPressure`, y sobrevive a que GCE reclame el nodo spot. La pirámide de puntos sin id (la de `ore-serve`) es la misma selección que con id, en 575 KB frente a 1,53 MB.
- **2026-10-26 · F2·3·2 en victor**: ORE #33 y #36 (el disparo) y la consola (rubix-platform #17) en producción. Las cuatro capas construyeron su pirámide solas desde el descriptor (3 en ~4 min, la de 30 M puntos en 29 min), sin desalojos, y las líneas con suelo 14. Destapa el crudo en el suelo: líneas a z14 de 2,8 MB (503) y `ore-teselas` OOMKilled con la z13 de `trip_30m`, lo primero que arreglar (presupuesto de memoria del motor y suelo calculado, F2·3·3). Falta el recorrido en la consola con una persona.
- **2026-10-26 · La iteración H (borrador)**: mantener el orden Hilbert de las tablas geo (profundidad en cada commit, reparar sólo lo dañado y sólo lo usado, `replace` invisible). H0·A respondido: el `replace` se construye con piezas de `iceberg-rust` y le falta el rebase; el row lineage no se conserva hoy; un snapshot nuevo mueve ~30 sitios, lo peor la cascada de copias y la pirámide. C1, sí: todo escritor confirma por `Lago::confirmar`.
- **2026-10-26 · H0·B y C2**: la profundidad predice lo leído (0,98); la rejilla, congelada por tabla (la fija sobre el mundo lee 1,5×); hoy una tabla masiva no sale ordenada de la copia (tramos); compactación por niveles (telco: 1 h·proceso/día frente a 23 al reescribirlo todo); sin rebase aborta el 78–98 %; las cajas no están en los manifiestos y el motor abre todos los pies; particionar por día deja profundidad ≥ días.
- **2026-10-26 · La iteración I (borrador)**: la capa se sirve de un índice suyo (tabla Iceberg del sistema, exacta, coherente, desechable e invisible), no de la tabla del cliente, que no se toca nunca. Sustituye a la iteración H: se cae lo que dependía de reescribir la fuente (los dos punteros, el rebase, el row lineage) y se cubren las particionadas por tiempo y las foreign databases. Pasos I0–I8.
- **2026-10-26 · I0 hecho**: la identidad, por posición `(fichero, fila)` en el lago y por clave fuera; la diferencia de snapshots, con piezas de `iceberg-rust`; se parten las líneas largas y no los polígonos; un índice de 240 M filas se construye en 112 s en un nodo (DuckDB, profundidad 2,4, I-1 cuadra).
