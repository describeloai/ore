# 0078 · Odyssey vector assets: el lago, buscable por significado en milisegundos

**Estado:** borrador (2026-10-28). Visión de producto y siete piezas que se van a investigar **por
separado y en orden** antes de construir. Hasta ahora solo hay medidas del motor: V0, V0b y V0d de
Vearch en victor (§5). Ninguna pieza tiene código.

*Odyssey vector assets* es el producto: de un dataset del lago nace un **vector asset**, una copia
vectorial gobernada, mantenida y servida por ORE. Sobre ella se construyen búsqueda semántica, RAG,
recomendación y agentes. El mecanismo es un **pipeline de eventos del lado de ORE**: cada commit de
la tabla de origen empuja sus filas al embedder y de ahí a un motor vectorial distribuido
(**Vearch**, absorbido y gestionado por nosotros). Nada se sondea periódicamente.

## 1. Lo que se promete

**Lo que se escribe en el lago se puede buscar por significado en milisegundos, de forma nativa y a
bajo coste.** El cliente no monta ningún pipeline, no elige endpoint ni tamaño, y no pierde la
gobernanza al salir de la tabla.

Lo que ve el cliente:

1. Sobre un dataset declara un **vector asset**: qué columnas se embeben, con qué modelo y por qué
   campos se va a filtrar (por ejemplo H3, stock o fecha).
2. ORE embebe lo que ya existe y, a partir de ahí, **cada commit** de la tabla (una escritura, un
   micro-lote de un stream, una copia refrescada) llega al vector asset sin intervención.
3. Lo consulta desde donde ya trabaja: `vector_search()` en Query (SQL), `ore.search()` en el SDK
   de Python y una herramienta para agentes. Lo sirve un cómputo especializado (Vearch), no el
   DuckDB ni el Spark genéricos.
4. Lo que el cliente no puede leer en la tabla tampoco lo encuentra en el índice: las denegaciones
   de 0047 viajan **dentro** del filtro.

### 1.1 Frente a Databricks AI Search (antes Mosaic AI Vector Search)

| | Databricks AI Search | Odyssey vector assets |
|---|---|---|
| Cómo llega el dato | **Pull**: Delta Sync lee el *change data feed*. `TRIGGERED` por lotes o `CONTINUOUS` en streaming; en tablas grandes, minutos | **Push**: el commit emite un evento y el upsert llega al índice sin ciclo de sondeo |
| Gobierno dentro del índice | El índice es un objeto de Unity Catalog, pero **sin seguridad por fila ni por columna**: lo resuelve cada aplicación con filtros | Las denegaciones de la tabla (0047) se aplican dentro del filtro del índice |
| Filtro geo | No existe | H3 como campo de primera clase (radio, anillo, jerarquía) |
| Escala | Standard ~320 M de vectores de 768 dimensiones; storage-optimized más de 1.000 M, con ~250 ms añadidos por consulta | Por medir (§5): HNSW para lo caliente, compresión (IVFPQ) para lo gigante |
| Embeddings | Gestionados (`ai_query` en lote) o propios | Gestionados por ORE, con el modelo del cliente como respaldo |

La diferencia de producto no está en el algoritmo, que en ambos casos es HNSW. Está en tres cosas:
**push desde el commit**, **gobierno de verdad dentro del índice** y **geo de serie**.

## 2. Las dos promesas, medidas por separado

| Régimen | Qué se promete | Cómo se mide |
|---|---|---|
| **Latencia**: stream o escrituras pequeñas (decenas o cientos de filas por commit) | Tiempo desde el commit de una fila hasta que una búsqueda la encuentra | Hora del commit frente a hora en que el vector aparece primero, fila a fila; p50, p99 y máximo, en reposo y con carga |
| **Caudal**: escrituras grandes (de millones de filas) | Tiempo hasta que la carga entera se puede buscar | Filas por segundo de punta a punta y tiempo hasta que la cola llega a cero |

Con una escritura grande **no se puede prometer "al instante"**: se forma cola (V0: 12 min de carga
más 95 s de indexado por millón). La promesa honesta ahí es un caudal.

**Presupuesto de latencia (hipótesis, por validar en cada pieza):**

| Eslabón | Presupuesto p99 |
|---|---|
| Evento del commit (pieza 1) | ≤ 10 ms |
| Lectura de las filas nuevas (pieza 2) | ≤ 30 ms |
| Embedding de un lote pequeño (pieza 4) | ≤ 50 ms en GPU; en CPU, por medir |
| Upsert más indexado en Vearch (pieza 5) | ≤ 20 ms |
| **De punta a punta, commit pequeño** | **≤ 100 ms p50 · ≤ 500 ms p99** |

Si una pieza no cabe en su presupuesto, se cambia el diseño de esa pieza. No se rebaja la promesa
en silencio.

## 3. La cadena

```
 escritura / stream / copia ──► Lago::confirmar ──► el puntero se mueve
                                                         │
   (1) evento del commit: {tabla, snapshot_ant, snapshot_nuevo}  ── outbox con entrega garantizada
                                                         │
   (2) lector incremental: altas, cambios y bajas entre los dos snapshots
                                                         │
   (4) embedder gestionado (lotes; modelo del cliente si el nuestro falla)
                                                         │
   (5) Vearch: upsert idempotente (id de fila + versión) ──► HNSW / IVFPQ + filtros escalares
                                                         │
   (6) vector_search() en Query · ore.search() en el SDK · herramienta de agente
                         ▲
   (3) el kind vector asset declara todo lo anterior; las denegaciones de 0047 viajan dentro
```

Si el índice diverge, se **reconstruye desde el snapshot**. Esa es la red de seguridad que permite
que el camino rápido sea agresivo.

## 4. Las siete piezas (borradores)

Cada pieza se investiga por separado, con su previo, su medida en victor y su condición de "hecho".
El orden de indagación está en §6.

### 4.1 Pieza 1 · El evento del commit (*el producto de eventos de ORE*)

**Qué es:** cada vez que se mueve el puntero de una tabla, ORE publica un evento
`{tabla, snapshot anterior, snapshot nuevo, quién, cuándo}` con **entrega garantizada al menos una
vez**. Es la base de todo Odyssey, y también es un producto propio: Health (0063) podría refrescar
por evento en vez de por `freshness`, `system.default.executions` podría alimentarse de él, y los
webhooks del cliente también.

**Qué hay hoy:** ORE sabe del commit, pero solo como **estado**. `Lago::confirmar`
([`lago.rs`](../../crates/ore-store/src/lago.rs)) escribe el `metadata.json` nuevo y el Job u
ore-serve mueven el puntero. El refresco de S8 (`ore_core::refresco`) sondea según la `freshness`, y
executions (X5) registra cada ingesta. Nadie empuja nada.

**Preguntas abiertas:**
- ¿Dónde se emite? ¿En el único punto que mueve el puntero, o en cada escritor (Job, puesto,
  ore-serve)? Hay que encontrar ese único punto.
- ¿Qué transporte? Una outbox en Postgres con `LISTEN/NOTIFY`, el Redis que ya se usa para
  invalidar en 0058, o un log dedicado. Hay que medir latencia y garantías.
- ¿Cómo se ordenan los commits de una misma tabla? ¿Y cómo se recupera un consumidor que se cae
  (reanudar desde el último snapshot confirmado)?
- Idempotencia: el mismo evento dos veces no debe hacer daño. Se apoya en 0031 («La idempotencia,
  revisada»).

**Hecho cuando:** commit → evento recibido con p99 ≤ 10 ms en victor, sin pérdidas tras matar el
consumidor a mitad de carga.

### 4.2 Pieza 2 · El lector incremental

**Qué es:** dado un par de snapshots, devolver solo las filas añadidas, cambiadas y borradas, sin
leer la tabla entera.

**Qué hay hoy:** Iceberg guarda en cada snapshot qué ficheros añade y cuáles borra. No tenemos un
lector incremental como pieza reutilizable.

**Preguntas abiertas:**
- Borrados y *merge-on-read*: con ficheros de borrado posicionales o de igualdad, ¿cómo se obtiene
  la lista de ids que hay que borrar del índice?
- Un commit pequeño ¿se lee desde el fichero recién escrito (caliente) o desde GCS? Hay que medir
  el coste de la lectura en frío.
- La compactación reescribe ficheros sin cambiar los datos: no debe disparar re-embeddings.

**Hecho cuando:** commit de 100 filas → filas en memoria del consumidor con p99 ≤ 30 ms; una
compactación produce cero filas.

### 4.3 Pieza 3 · El kind `VectorAsset`

**Qué es:** el documento del árbol que declara el activo:
- el dataset de origen;
- las columnas que se embeben (texto y, más adelante, medios de 0046/0049);
- el modelo y el modelo de respaldo;
- la dimensión (el `Vector<n>` de 0032);
- los campos de filtro (H3 y sus resoluciones, escalares);
- la métrica y el perfil (latencia o volumen).

**Preguntas abiertas:**
- ¿Es una variante de copia o un kind propio? Por cómo se gobierna se parece a una copia (0063);
  por cómo se sirve, a una capa de Geo serving (0072).
- **Gobierno:** ¿cómo se traducen las denegaciones de 0047 (por fila y por columna) a filtros que
  Vearch aplique dentro de la búsqueda? ¿Y qué pasa cuando cambia la política: se reindexa o se
  filtra al consultar?
- Ciclo de vida: presupuesto, expiración y desuso, como en 0072 I5.
- ¿Se guardan también los embeddings en el lago, como una tabla con columna `Vector<n>`? Sirve
  para reconstruir sin volver a embeber y para analítica en DuckDB.

**Hecho cuando:** un `VectorAsset` se declara, compila con diagnósticos y su política de acceso se
expresa como filtro verificable.

### 4.4 Pieza 4 · El embedder gestionado

**Qué es:** el servicio que convierte filas en vectores. El modelo lo gestiona ORE (por defecto
uno abierto, servido como en Model Serving, 0027); si falla, se usa el modelo elegido por el
cliente.

**Preguntas abiertas:**
- **Coste:** una GPU encendida todo el día rompe el "bajo coste". ¿CPU para lo pequeño y GPU bajo
  demanda para lo grande? ¿Lotes por ventana de tiempo, por ejemplo 5 ms?
- Latencia de un lote pequeño en CPU frente a GPU, con modelos de 384 y 768 dimensiones.
- Qué significa "en fallo" (caída, cuota, timeout) y qué pasa con un vector asset cuando cambia el
  modelo: se reembebe entero, con versión del modelo en el índice.
- Medios (imagen, audio): ¿entran en v1 o después?

**Hecho cuando:** lote de 1 a 100 filas → vectores con p99 ≤ 50 ms y un coste por millón de
embeddings medido.

### 4.5 Pieza 5 · Vearch como serving

**Qué es:** Vearch (Apache 2.0, de JD.com y LF AI & Data), con sus tres papeles:
- **master:** metadatos (etcd/Raft);
- **router:** API y reparto de las consultas;
- **partition server:** particiones replicadas con Raft, con el motor Gamma (Faiss + HNSW/IVF y
  filtro escalar en el mismo nodo).

Desplegado y operado por ORE, un espacio por vector asset, serverless para el cliente.

**Preguntas abiertas:**
- ~~Upsert → visible, documento a documento~~: 16 ms p50 en reposo y 71 ms p99 con 1.000 docs/s
  (V0b, §5.1). Abierto: control de admisión para que una carga grande no sature el stream.
- El filtro geo: **no funciona con HNSW** (post-filtro, §5.1), ni en 3.5.9 ni en `master`. Sí
  funciona, exacto y en 3–5 ms, por fuerza bruta con *filter-first* (`master`, §5.3), si ORE
  elige ese camino cuando el filtro es selectivo. Abierto: el filtro de rango amplio (V0e).
- Reconstrucción periódica: las altas y bajas degradan el recall (§5.2).
- Réplicas con Raft: qué cuesta en latencia de escritura y qué da en disponibilidad.
- Escalado: `partition_num` se fija al crear el espacio y existe un `PUT partition_num`. ¿Reparte
  en caliente o reconstruye?
- Mil millones de vectores: con HNSW en RAM (6,9 GB por millón) no es "bajo coste". Hay que medir
  IVFPQ (entrenamiento, recall, RAM) y DiskANN, que aparece en las pruebas de 3.5.
- Operación: copias de seguridad, reconstrucción desde el lago, multi-tenencia, cuota de 12 vCPU.

**Hecho cuando:** V0b y V0c (§6) cumplen su presupuesto, o el ADR cambia de motor.

### 4.6 Pieza 6 · El consumo

**Qué es:** tres puertas al mismo índice:
- `vector_search(activo, texto | vector, k, filtro)` como función de tabla en Query (0076): el
  planificador la manda a Vearch y el resto de la consulta sigue en DuckDB;
- `ore.search()` en el SDK;
- una herramienta para agentes (RAG, preguntas en lenguaje natural sobre lo no estructurado).

**Preguntas abiertas:**
- Cómo se combina con SQL: unir el top-k con tablas del lago sin traerse el índice entero.
- Si la consulta llega en texto, el embedding de la consulta pasa por la pieza 4: es latencia que
  se suma.
- Búsqueda híbrida: BM25 más vectores con RRF, como hace AI Search.

### 4.7 Pieza 7 · Las dos promesas, medidas

**Qué es:** el banco permanente de Odyssey en victor. Un generador de commits pequeños (stream) y
uno de cargas grandes, contra la cadena entera, con informe de p50, p99 y máximo por eslabón y de
punta a punta (§2). Es la vara: nada se mezcla sin pasarla (ver «Construir contra producción»).

## 5. Lo medido: V0 de Vearch (2026-10-27, victor)

**Banco:**
- Vearch 3.5.9 standalone (master, router y partition server en un pod) sobre `n2-highmem-4`
  (4 vCPU, 32 GB); cliente en `jobs-s`.
- 1.000.000 de vectores de 768 dimensiones (mezcla de 1.024 gaussianas, normalizados) con HNSW
  (`nlinks 32`, `efConstruction 160`, `efSearch 64`), InnerProduct, `MemoryOnly`.
- Campos escalares: `stock` y la celda H3 a resoluciones 4 y 7.

**Resultados:**

| Medida | Resultado |
|---|---|
| Carga por upsert REST (lotes de 500) | 1.321 docs/s sostenidos; el límite es la CPU de Vearch (3,5 núcleos) |
| Índice frente a lo confirmado | Va por detrás (~50 % a mitad de carga); 130.000 pendientes al acabar y 95 s hasta ponerse al día |
| RAM | 6,9 GB por millón (6,9 KB por documento, 2,25 veces el vector en bruto); disco 11 GB |
| Top-10 sin filtro, 1 cliente | p50 8,7 ms · p99 10 ms · 112 consultas/s · recall@10 0,97 (1,0 con `efSearch` 512) |
| Top-10 sin filtro, 8 clientes | p50 44 ms · p99 116 ms · 162 consultas/s |
| Filtro amplio (`stock > 0`, 70 %) | 1 cliente p50 315 ms; 8 clientes p50 2 s y 4 consultas/s |
| Filtro H3 selectivo | **0 documentos devueltos** aunque los hay. Lo explica V0b: es el post-filtro de HNSW |

### 5.1 V0b: el eslabón de Vearch y el filtro geo (2026-10-28, victor)

Mismo banco, recargado (1.315 docs/s; 7,1 GB de RAM con los dos campos H3 como `long`).

**Upsert → visible, documento a documento** (escribir 1 documento y buscarlo hasta que aparece;
200 intentos; la búsqueda misma cuesta ~9 ms, que van incluidos):

| Carga de fondo | Confirmación p50 / p99 | Visible p50 / p99 / máx | No visibles en 30 s |
|---|---|---|---|
| Ninguna | 7,3 / 9,0 ms | **15,7 / 22,8 / 27 ms** | 0 / 200 |
| 500 docs/s | 8,0 / 47 ms | **19,5 / 65,7 / 89 ms** | 0 / 200 |
| 1.000 docs/s | 10,8 / 44 ms | **24,7 / 70,8 / 73 ms** | 0 / 200 |
| Saturada (2 escritores sin freno, ~10 min por encima de lo que indexa) | 18,5 / 200 ms | 116 ms / **más de 30 s** | **≥ 5 %** |

Al acabar la prueba saturada, el índice seguía **44.000 documentos por detrás** y tardó 82 s en
ponerse al día. Las pruebas de 500 y 1.000 docs/s duraron solo ~5 s de fondo cada una; la de
saturación, minutos.

**El filtro H3 en 3.5.9:**
- En un espacio pequeño con FLAT, `IN` sobre la celda como `string` devuelve exactamente lo que
  debe (36 de 36 en la celda, 201 de 201 en el anillo), también con `enable_realtime`.
- Sobre `long`, 3.5.9 **no admite `IN`** (`cannot unmarshal number into … string`); sí admite
  rangos (`>= celda`, `<= celda` funciona).
- **Con HNSW, el filtro se aplica después de recorrer el grafo.** Con 20.000 documentos y un
  filtro de 12 o 69, la búsqueda devuelve 0 aunque la consulta escalar los encuentra.

Barrido de `efSearch` con 1M documentos (recall@10 contra la verdad por fuerza bruta, 50 consultas;
el millón venía de 473.000 altas y bajas):

| Filtro (selectividad) | ef 64 | ef 512 | ef 2048 | ef 8192 | p50 |
|---|---|---|---|---|---|
| Sin filtro (100 %) | 0,888 | 0,932 | 0,976 | 0,976 | 9 → 48 ms |
| `stock > 0` (70 %) | 0,882 | 0,930 | 0,976 | 0,976 | ~420–450 ms |
| Celda H3 r4 y `stock > 0` (0,14 %) | 0,050 | 0,096 | 0,222 | 0,442 | ~420–450 ms |
| Anillo H3 r7 radio 10 y `stock > 0` (0,14 %) | 0,048 | 0,086 | 0,210 | 0,444 | ~400–450 ms |
| Anillo H3 r7 radio 1 y `stock > 0` (0,003 %) | 0,000 | 0,000 | 0,002 | 0,030 | ~400–430 ms |

### 5.2 Lo que V0 y V0b deciden

- **El eslabón de Vearch cabe en el presupuesto mientras no se sature:** visible en 16 ms (p50) en
  reposo y en 71 ms (p99) con 1.000 docs/s de fondo. La promesa de milisegundos **no la rompe el
  motor**.
- **Por encima de lo que indexa (~1.100–1.300 docs/s con una partición en 4 vCPU), la cola
  crece sin límite** y un documento nuevo espera detrás de toda ella. La pieza 5 necesita
  **control de admisión**: separar el carril del stream (latencia) del de las cargas grandes
  (caudal), o repartir en más particiones. Una carga grande nunca debe tapar al stream.
- **Las altas y bajas degradan HNSW:** tras 473.000, el recall sin filtro bajó de 0,968 a 0,888
  (ef 64) y no pasa de 0,976 con ningún ef. Un vector asset vivo necesita **reconstrucción
  periódica** del índice (`forcemerge` o un espacio nuevo en paralelo).
- **El caso geo no funciona en Vearch 3.5.9 con HNSW.** Un filtro selectivo devuelve casi nada,
  aunque se suba `efSearch`. Y todo filtro con `stock > 0` paga ~400 ms al construir el conjunto
  candidato, sea cual sea su selectividad.
- La solución existe en `master` de Vearch (*filter-first* con `disable_filter_first`, índices
  `BITMAP`/`INVERTED` y compuestos, mayo–julio 2026), pero **no en ninguna versión publicada**
  (la última imagen es 3.5.9, de febrero). **V0d** la mide construyendo la imagen desde `master`.
- **La RAM marca el coste:** a escala de mil millones, la compresión es obligatoria (V0c).

### 5.3 V0d: Vearch desde `master` (2026-10-28, victor)

**Banco:**
- Vearch compilado dentro de un pod desde el commit `bae78b18` de `master` (27-jul-2026), con
  la imagen oficial `vearch/vearch-dev-env:3.5.9`. Sin DiskANN, que la imagen de desarrollo no
  trae. AVX-512. Compila en ~5 min con 4 vCPU.
- 200.000 vectores de 768 dimensiones, HNSW igual que en V0b, H3 como `string` con índice
  `SCALAR` y con índice `BITMAP`.

**Lo que hay en `master` y lo que no:**
- El *filter-first* (`disable_filter_first`) existe **solo en el índice FLAT**
  (`gamma_index_flat.cc`). HNSW sigue filtrando después de recorrer el grafo.
- Pero cualquier índice cae al FLAT si la petición lleva **`is_brute_search: 1`**
  (`engine.cc`; el modo `2` solo actúa mientras el índice no está construido). **Elegir el camino
  por consulta es tarea de quien llama**, no de Vearch.

**Resultados (recall@10 contra fuerza bruta; 50 consultas; p50):**

| Filtro (candidatos) | HNSW, ef 64 | HNSW, ef 2048 | Fuerza bruta (`is_brute_search: 1`) |
|---|---|---|---|
| Sin filtro (200.000) | 0,998 · 4,3 ms | 1,000 · 23,5 ms | 1,000 · 33 ms |
| `stock > 0` (139.950) | 0,998 · 56 ms | 1,000 · 76 ms | 1,000 · 77 ms |
| Celda H3 r4 y `stock > 0` (279) | 0,074 · 56 ms | 0,410 · 76 ms | **1,000 · 56 ms** |
| Anillo H3 r7 radio 10 y `stock > 0` (271) | 0,062 · 56 ms | 0,392 · 77 ms | **1,000 · 56 ms** |
| Anillo H3 r7 radio 1 y `stock > 0` (6) | 0,000 · 59 ms | — | **1,000 · 60 ms** |
| Anillo H3 r7 radio 1, **sin** stock | — | — | **3,4 ms** |
| Anillo H3 r7 radio 10, **sin** stock | — | — | **4,3–5,1 ms** |

`SCALAR` y `BITMAP` sobre H3 dan los mismos números. Upsert → visible con `master` y 200k:
**8,4 ms p50 / 12,3 ms p99** en reposo; **12 ms p50 / 52 ms p99** con 1.000 docs/s de fondo
(V0b, con 1M y la 3.5.9: 15,7 / 22,8 y 24,7 / 70,8). Carga: 1.534 docs/s.

**Lo que V0d decide:**
- **El caso geo funciona con Vearch, pero no solo.** ORE tiene que **elegir el camino en cada
  consulta**: HNSW si el filtro deja pasar mucho, fuerza bruta (*filter-first*) si es selectivo. La
  selectividad de un filtro H3 se puede estimar (cuántos documentos hay por celda) antes de
  preguntar. Esta regla va en la pieza 6 (consumo), no en el motor.
- **Un filtro de rango amplio (`stock > 0`) cuesta lo mismo con cualquier camino**: Vearch
  construye su conjunto entero antes de cruzarlo con el resto (~56 ms con 200k, ~400 ms con 1M).
  Hay que medir representarlo como campo de baja cardinalidad con `BITMAP` (V0e).
- Hace falta `master` (o la versión que lo publique) para el *filter-first* del FLAT. Que
  `is_brute_search` con filtro sea igual de rápido en 3.5.9 no está medido (V0e).

## 6. Orden de indagación

| Paso | Qué | Por qué en este orden |
|---|---|---|
| ~~V0b~~ | **Hecho (2026-10-28, §5.1).** El eslabón cabe sin saturar; el filtro geo no funciona en 3.5.9 | Validaba el motor antes de construir nada alrededor |
| ~~V0d~~ | **Hecho (2026-10-28, §5.3).** Con `master` y `is_brute_search`, el filtro geo es exacto en 3–5 ms; el rango amplio sigue costando | Decidía si Vearch sirve para geo: sirve, si ORE elige el camino por consulta |
| **E1** · *ahora* | Pieza 1: el único punto que mueve el puntero de una tabla, el evento emitido en ese mismo instante, el transporte (objetivo p99 ≤ 10 ms a escala) y su fiabilidad y salud. **E1·0 medido (2026-10-28):** hoy ese punto es el empujón de ore-serve a la forja, y cuesta 2,6–3,6 s por commit, en serie (~0,3 commits/s por inquilino). **E1 pasa a ser [0079 · Transactional Assets Catalog](0079-transactional-assets-catalog.md)** (E1a·0–5): el estado vivo de los datos fuera de git, y el evento dentro de la misma transacción | Es la base y un producto propio; no depende de Vearch |
| **I1** · *en paralelo con E1* | Imagen propia de Vearch construida en el CI desde un commit fijo de `master` (`bae78b18`), publicada en nuestro registro. **No se despliega en la malla** | Reproducible y lista para los bancos y para V1; sin coste en marcha |
| **V0e** | Con la imagen de I1: el filtro amplio barato (`stock > 0` como campo de baja cardinalidad con `BITMAP`, o fuera del filtro); la regla de §5.3 con 1M; y si `is_brute_search` con filtro funciona igual en 3.5.9 | Sin esto, "cerca y con stock" cuesta 60–400 ms |
| **E2** | Pieza 2: lector incremental, borrados, compactación | Sin él, el evento no sirve |
| **E3** | Pieza 4: embedder en CPU frente a GPU, coste por millón | Probablemente el eslabón más caro |
| **V0c** | Vearch: réplicas con Raft, 2 particiones, IVFPQ con 1M y extrapolación, `PUT partition_num` | Coste y escala reales de la pieza 5 |
| **V1** | **Vearch entra en la malla** (escala a cero sin uso) y se mide la cadena mínima de punta a punta (piezas 1, 2, 4 y 5) contra un stream de prueba y una carga grande | La primera medida de la promesa (pieza 7) |
| **K1** | Piezas 3 y 6: kind, gobierno dentro del índice, la regla HNSW/fuerza bruta por selectividad y el consumo en Query, SDK y agentes | Se diseña sobre una cadena que ya cumple |

Cada paso deja su número en §5 y su decisión aquí. El ADR se reescribe limpio al cerrar cada
etapa, como imagen del estado actual y no como diario.

## 7. Lo que se acepta a cambio

- **Dos sistemas, una verdad.** El lago y Vearch no comparten transacción: hay ventanas en que el
  índice va por detrás. Se acepta a cambio de la latencia, con reconstrucción desde el snapshot y
  upsert idempotente.
- **Una copia más de los datos.** El vector asset duplica (en forma de vectores) lo que hay en la
  tabla. Por eso el gobierno tiene que viajar dentro y la retirada tiene que ser real.
- **Un motor que no es nuestro y con poca comunidad.** Vearch tiene menos ecosistema que Milvus o
  Qdrant, parte de la documentación en chino y una API que cambió en 3.x. Se acepta porque se
  opera dentro y el contrato hacia el cliente es nuestro (`vector_search`, SDK); el motor se puede
  cambiar sin que el cliente lo note.
- **RAM y GPU cuestan.** "Bajo coste" depende de la compresión (pieza 5) y de no tener una GPU
  encendida sin trabajo (pieza 4). Con la cuota actual (12 vCPU) solo cabe un banco, no un
  servicio.

## 8. Fuera, por ahora

- Embeddings de medios (imagen, audio, vídeo): después de texto.
- Búsqueda híbrida BM25: después de la vectorial.
- Multi-región y CMK.
