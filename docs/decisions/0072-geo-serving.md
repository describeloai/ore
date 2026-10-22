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

**C2 · la dirección de tesela** (diseño de producto, 2026-10-22; por aprobar).

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

*Por aprobar:*

| # | punto | propuesta |
|---|---|---|
| C2-a | **Clave opaca** en lugar de los parámetros en la ruta | nadie fabrica direcciones; URL corta; comparte caché entre iguales |
| C2-b | **TileJSON 3.0 para MapLibre y 3D Tiles 1.1 implícito para el satélite** | los dos estándares; sustituye a nuestro `arbol.json` |
| C2-c | **Firma aparte de la clave**, por `clave` y de vida corta | caché compartida en el borde sin URLs eternas |
| C2-d | **Etiquetas dentro del MVT y en teselas de etiquetas para el satélite** | MapLibre colisiona solo; el satélite junta y pone el tope |
| C2-e | **El clic devuelve la fila legible**, no sólo `properties` | pintar con lo mínimo, consultar con permisos |
| C2-f | **Frescura por el descriptor** y vista previa de ramas con `branch=` | la tesela no se invalida nunca; las propuestas se ven en el mapa |

**C3 · el protocolo del motor.** El motor no sabe de personas (D-S-1); recibe todo resuelto:

| va | qué |
|---|---|
| la tesela | `tms`, `z/x/y`, formato |
| la fuente | la `metadata.json` de la instantánea del dataset y, si existe, de la pirámide (C4) |
| lo permitido | el predicado de filas (visibilidad + `filter`) ya escrito, las columnas permitidas (`properties` ∩ máscara) |
| el estilo | resuelto (sólo para glb) |
| la credencial | prestada y de vida corta, sólo de lectura |

| vuelve | qué |
|---|---|
| los bytes | con su tipo |
| la cuenta | elementos, origen (crudo o pirámide), grupos leídos/descartados, ms; para medir y para la salud |

Sólo acepta a ore-serve, sin estado entre peticiones (réplicas), y su caché interna, si la tiene, usa la
dirección de C2 como clave.

**C4 · la pirámide de agregados.** Un dataset derivado por `Layer` (las métricas son de la `Layer`),
construido por un Build desde una instantánea del dataset, con linaje a ella:

| columna | qué |
|---|---|
| `tms`, `z`, `x`, `y` | la tesela donde cae la celda: **el orden del fichero** (leer una tesela = un rango) |
| `res`, `celda` | resolución y celda H3 (`u64`) |
| `count` y las métricas de `aggregate` | los agregados |

- Un nivel H3 por nivel de zoom, elegido para que una celda ocupe ≥ 14 px (T10).
- Sólo se construyen los niveles donde alguna tesela pasa del umbral; el resto sale crudo.
- **Riesgo nombrado:** un agregado calculado sobre todas las filas revela lo que alguien no puede ver. Hasta A8,
  una `Layer` con pirámide sólo se sirve a quien puede leer el dataset entero; después, la pirámide será por
  clase de visibilidad.

**El índice de etiquetas** (parte de C4, 2026-10-22). Una pantalla enseña unos cientos de etiquetas legibles
(T12); una capa puede tener millones de elementos etiquetables. El Build le da a cada elemento su **nivel de
aparición**:

- **El punto nunca se esconde, sólo su texto:** los elementos van siempre en la tesela (glb o MVT); lo que se
  reparte por niveles es la etiqueta.
- **El nivel sale de la densidad:** por cada zoom, de lejos a cerca, una rejilla del tamaño de una etiqueta en
  pantalla; en cada casilla libre aparece una, las que chocan esperan al nivel siguiente. **Monótono**: lo que
  aparece no desaparece al acercarse. Por construcción, ninguna vista pasa del tope (≤ ~300, T12).
- **Desempate determinista** (sin parpadeos): la prioridad del cliente (`symbol-sort-key` del estilo, K1); si
  no hay, la más cercana al centro de la casilla; al final, el identificador.
- **Aclarar o agrupar**, lo elige el estilo: las etiquetas de los elementos o, de lejos, grupos con su cuenta
  («Texas · 812 locales») que se abren al acercarse (de la misma pirámide).
- Columnas: `id`, la posición, `minzoom`, la prioridad y el texto ya resuelto. Servir una vista es «las de esta
  caja con `minzoom ≤ z`»; el satélite las pinta como marcadores, renovadas por diferencia.

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

Los pasos (E2·0 aprobado el 2026-10-22):

| paso | qué | dónde |
|---|---|---|
| E2·1 | **El sustrato:** la geometría sin envolver en el SDK (D-S-7), `Binary → LargeBinary` en `sin_perdida` (el `append` de geometría), Hilbert también en Spark (y por todas las columnas geo), compactar los `append`; cotas o cobertura en el manifiesto para descartar ficheros enteros | `ore-store` · `puesto/python/ore` |
| E2·2 | **El motor de teselas:** MVT y glb con relieve (D-S-12) y agregado H3, puro, con réplicas y caché de contenido; el relieve como activo del lago | según T1 y T6–T11 |
| E2·3 | **La puerta:** `/tiles/...` y `/mapa/fila` en ore-serve (identidad, instantánea, filtro, credencial); ore-serve con más de una réplica | `ore-serve` · malla |
| E2·4 | **Lo precalculado:** la pirámide de agregados como dataset derivado, construida por un Build (D-S-5, D-S-9), con linaje y salud | `ore-core` · Build |
| E2·5 | **La consola:** capas del catálogo en lugar de `mock.ts`, las mismas en los dos mapas; fuente `vector` en MapLibre; el árbol de teselas glb en el satélite 3D | rubix-platform |

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
