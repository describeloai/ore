# Query — el motor por consulta

> Lo que la consola llama *Query*: donde el cliente consulta sus activos del Assets Catalog —datasets,
> views, bases foráneas, orígenes— con SQL, sin crear un repositorio. La interfaz ya existe
> (`clusters/[celda]/query`, el cuerpo de Code Repositories con `modo="query"`). Este documento es **la
> lógica de negocio que le falta**: que cada consulta corra en el motor que le toca —DuckDB en la sesión,
> DuckDB en un nodo grande o Spark elástico— decidido **antes de correr** por lo que va a leer. No es un
> ADR: es el plan de construcción, con lo medido en victor el 2026-10-23 (Q0) y lo que ya existe y se
> reutiliza.
>
> ✏️ **Replanteado el 2026-10-23 (tarde).** La primera forma (`ore-consulta`, un servicio de trabajadores
> DuckDB compartidos, Q4–Q5) se construyó, se empujó apagada y **se retiró sin medirla**: era un segundo
> ejecutor que sólo sabía `SELECT` y rehacía a la baja lo que el puesto ya hace —escribir en el árbol desde
> SQL (datasets, vistas, schemas, bases, media collections), colecciones, Functions, lecturas en vivo—.
> Lo que hacía falta era **capacidad**, no un motor: Query corre en **el puesto de siempre, del tamaño que
> toca** (§3.2). Lo que se quitó, en §5 (Q4).

---

## 1. Lo que hay hoy (comprobado en el código)

| pieza | qué hace | dónde |
|---|---|---|
| la hoja de Query | una celda `sql` en el puesto Python de la persona; se guarda como commit en `queries/` | `CodeWorkspaceClient.tsx` (`enQuery`) |
| `ore.sql` | DuckDB del puesto; `ore-serve` resuelve los nombres en nombre de la persona y el SDK registra cada uno como vista: Iceberg con credencial prestada, views, colecciones, bases foráneas y lecturas en vivo (`/federation/read`) | `puesto/python/ore/__init__.py` `sql()` |
| las escrituras desde SQL | `create view`, `create schema`, crear base, escribir en el lago: sentencias del árbol | `ore-serve` `sql_que_escribe` |
| el reparto | de una sentencia, por tabla: columnas, filtros empujables, `limit`, atravesando views | `ore_core::reparto::repartir` |
| `explain` | qué va a cada origen y qué queda en DuckDB | `POST /puestos/{id}/explain` |
| el Build elástico (CE) | un driver en `pesado-driver` y ejecutores en `pesado` por *dynamic allocation*, en Kueue, con política y red | `ore/spark.py` `_elastico()`, `cola::al_driver_elastico`, `malla/22` |
| el modo por tamaño | un Build pesado es elástico si sus entradas pasan de 100 M filas (`total-records` del snapshot) | `builds.rs` `filas_de_las_entradas` |

**El hueco:** una celda de Query corre siempre en el DuckDB de la sesión (2 CPU, 4 GiB, DuckDB con 2 GB).
Heavyweight sólo existe para Builds y Previews; no hay ningún sitio que elija el motor de una consulta.

## 2. Lo medido (Q0, victor, 2026-10-23)

Arnés en [`pruebas-de-fuego/query/`](../pruebas-de-fuego/query/), resultados en `resultados/20261023-*.jsonl`.
Datos: el Parquet del banco leído por HTTPS con la identidad del pod (`spatialbench/sf100/trip.parquet`,
600 M filas, 39 GB, 309 grupos de filas ordenados por `t_tripkey`), cortado en 6 M, 60 M y 600 M filas por
`t_tripkey <= k` (la poda por estadísticas lo hace exacto). Nueve consultas: contar, punto, agregar,
top-N, muchos grupos (3 M), distintos, junta con `customer` (3 M), ventana (`lag` por cliente) y caja geo
(`ST_Intersects` sobre la WKB). **`gb` es lo que un enrutador sabría antes de correr**: los bytes de las
columnas proyectadas en los grupos que el filtro deja, de los metadatos del Parquet.

### 2.1 Los dos DuckDB, consulta a consulta (primera vez, en segundos)

| consulta | gb | sesión 6 M | sesión 60 M | **sesión 600 M** | grande 6 M | grande 60 M | **grande 600 M** |
|---|---|---|---|---|---|---|---|
| contar | 0 | 0,44 | 2,0 | 0,08 | 0,25 | 0,67 | 0,04 |
| punto | 0,13 | 0,54 | 0,11 | 0,59 | 0,66 | 0,10 | 0,07 |
| agregar | 4,26 | 0,77 | 6,2 | 54,5 | 0,51 | 1,9 | **22,4** |
| top-N | 10,5 | 0,58 | 5,3 | 54,5 | 0,64 | 2,0 | **16,9** |
| muchos grupos | 3,74 | 1,18 | 5,1 | 91,1 | 0,45 | 1,7 | **16,3** |
| distintos | 2,13 | 0,41 | 2,8 | 54,7 | 0,33 | 1,2 | **10,1** |
| junta | 3,74 | 1,71 | 8,9 | 121,9 | 0,78 | 2,0 | **14,6** |
| ventana | 8,02 | 1,16 | 15,0 | **sin memoria** | 0,60 | 2,9 | **32,7** |
| caja geo | 10,5 | 1,93 | 17,6 | — (parada, ver 2.4) | 1,16 | 6,4 | **50,1** |

`gb` es el de 600 M (a 6 M y 60 M es la décima y la centésima parte). La segunda vez, con la caché de
ficheros remotos de DuckDB, la sesión baja a 0,1–1,7 s a 6 M y a 0,5–19 s a 60 M.

- **Sesión** = el puesto de hoy: 2 CPU / 4 GiB, DuckDB 2 GB, en el nodo `sistema-spot` (`cq-victor`
  tiene sabor `system`: ahí caen las sesiones reales). Lo pequeño del lago, al instante: `select 1`
  1 ms, contar 2,2 M lugares 0,2 s, una caja sobre los 34 M edificios 4,8 s en frío y 0,3 s después.
  **La ventana a 600 M agotó los 10 GiB de derrame** (`failed to offload data block … 10.0 GiB/10.0 GiB
  used`) con el pod en 4 098 MB, el límite.
- **Grande** = un pod del pool `pesado` (`n2-highmem-8` spot con SSD): 7–8 CPU / 48 GiB, DuckDB 38 GB, 8
  hilos. **Las nueve consultas sobre 600 M en 0,04–50 s, sin derrame**; pico 37 GB (muchos grupos).
  Frente a la sesión: de 2,4× (agregar) a 8× (junta), y termina lo que la sesión no.

### 2.2 Los arranques en frío

| escalón | de pedirlo a correr | desglose |
|---|---|---|
| sesión | ~4 s | el nodo del sistema ya está; la imagen, en caché |
| grande (DuckDB en `pesado`) | **92 s** | Kueue 1 s; el autoescalador pide el nodo 1 s; el nodo con SSD tarda 80 s en registrarse y 6 s en estar `Ready`; la imagen por *streaming*, 1 s; el contenedor arranca a los 92 s |
| elástico (Spark en `pesado-driver` + `pesado`) | **~3 min** | el nodo del driver, 51 s; el pod, 60 s; el servidor de Spark escucha 12–21 s después; el primer ejecutor, ~95 s más si su nodo está a 0 (2 s si está encendido) |

### 2.3 Spark Connect, elástico, en el driver de un Build (sonda `connect.py`)

Un servidor Spark Connect dentro del pod que sería el driver de un Build elástico (cuenta
`driver-pesado`, `cola-driver`, la configuración exacta de `ore.spark._elastico()`, techo 2), y un
cliente en el mismo pod.
- **La imagen lo trae** (`spark-connect_2.13-4.1.0.jar`). Al cliente le faltan `grpcio`,
  `grpcio-status` y `zstandard`: hay ruedas cp314 y se instalaron sin red desde el bucket.
- **Latencia con el servidor en caliente:** 0,14–0,22 s por consulta trivial (la primera, 8 s).
  `range(600 M)` agrupado en 100: 2–3,5 s con un ejecutor; en 20 M grupos, 36–40 s.
- **Escala por la carga:** el 2.º ejecutor se pide a mitad de la consulta pesada y llega cuando ya ha
  acabado (como en CE·5). Otro se pide con dos consultas a la vez, y el sobrante se devuelve a los
  ~67 s de quedarse ocioso. Con `minExecutors 1`, el nodo del primero no se suelta nunca.
- **Aislamiento:** las vistas temporales son de cada sesión. **El catálogo de Spark es compartido**
  (B lee la tabla que A guardó), y Spark Connect no autentica: `user_id` lo dice el cliente. ⇒ un
  servidor es de **una** persona, nunca compartido.
- ⛔ **`reflect('java.lang.System', …)` está permitido**: el SQL de un cliente ejecuta métodos de Java en
  el driver, que es una cuenta que crea pods (la política de la celda los acota, pero el SQL deja de ser
  sólo SQL). `ReglasDeOre` no lo bloquea hoy.
- El cliente de Python no sale del proceso al acabar (hilos de gRPC): hace falta `os._exit`.

### 2.4 Lo que pasó en victor durante la medida (y es un hallazgo de producción)

- **11:00:28.** Mi Job de dialecto (imagen `puesto-spark`, 1,26 GB comprimida, más Spark local) cayó en el
  nodo del sistema. Con la sesión de medida al lado, el nodo se quedó sin disco efímero (8,4 GB
  asignables): el kubelet desalojó **una pasada del `aprovisionador`** y la sesión de medida. La
  siguiente pasada del `aprovisionador` terminó bien.
- **13:17–13:22.** La sesión repetida, sola, derramó sus 10 GiB en la ventana de 600 M. El nodo entró en
  `DiskPressure` con el *taint* `NoSchedule` cinco minutos, hasta que borré el Job. Ninguna pieza de
  la plataforma se reinició; un `aprovisionador` tardó 2,6 min en vez de 30 s.
- **La causa es de hoy, no de Query:** `ore._duckdb()` pone `temp_directory` en `/trabajo` (un `emptyDir`)
  sin `max_temp_directory_size`, y el pod de la sesión no pide `ephemeral-storage`. Una sesión que
  derrama llena el disco del nodo, y el kubelet desaloja a otros: es lo que CE·6 arregló para los
  ejecutores ("cae ese pod, no el nodo"), pero las sesiones siguen sin ese tope. Query lo haría frecuente.
  Es el paso Q1, antes que nada.

### 2.5 El dialecto: el SQL de DuckDB, traducido a Spark con sqlglot (`dialecto.py`)

Cada consulta corre en DuckDB tal cual y, traducida con sqlglot 30.23 (el de la imagen de notebooks),
en Spark local, sobre los mismos Parquet (escala 0,1). Se comparan los resultados.

| corpus | iguales | fallan en Spark (con error) | **distintas (en silencio)** | no traduce |
|---|---|---|---|---|
| TPC-H (22) | **22** | — | — | — |
| TPC-DS (99) | **99** | — | — | — |
| `idiomas.sql` (64: lo «amigable» de DuckDB) | 45 | 11 | 6 | 2 |

- **Fallan con error (bien: se ve):** `* replace`, `len(lista)`, corte de texto `s[1:3]`,
  `quantile_cont(x, p)`, `asof join`, `union by name`, `columns(…)`, `read_parquet` relativo,
  `summarize`, `->` de JSON, `h3_*`. **No traducen:** `pivot` y `unpivot`.
- **Distintas en silencio (mal):**
  - `qualify … limit` — sqlglot mete el `LIMIT` **dentro** de la subconsulta, antes del filtro: da
    menos filas. Es un error de traducción;
  - `x::int` de un decimal — DuckDB redondea y Spark trunca (194030 frente a 194029);
  - `list(x order by y)` — pasa a `collect_list` **sin el orden** (aquí coincidió por suerte);
  - `median` (redondeo de la interpolación), `approx_count_distinct` (aproximado por diseño) y
    `describe` (otro formato): esperables.
- ⇒ **El SQL analítico estándar viaja; lo amigable de DuckDB, no siempre, y a veces sin avisar.** Mandar
  una consulta a Spark exige una comprobación previa con una lista de construcciones prohibidas, no sólo
  "traduce".

### 2.6 Lo que dice la industria (para no inventar)

- **MotherDuck, dual execution:** el planificador de DuckDB parte la consulta entre el cliente y la nube
  por dónde están los datos, y `EXPLAIN` marca cada operador como local o remoto.
- **Databricks, Intelligent Workload Management:** predice el coste de cada consulta al llegar (con
  modelos aprendidos), la admite si hay capacidad o la encola y escala.
- **Spark Connect:** cliente por sesión, sin autenticación propia (se delega en un proxy); los
  proyectos que lo sirven a varias personas usan un servidor por persona.

Lo que hacemos encaja con los dos primeros, pero con una regla explícita y medida en vez de un modelo:
ORE ya sabe, antes de correr, qué columnas y qué filtros lee cada tabla (el reparto) y cuántos bytes
tiene cada columna de cada fichero (los manifiestos de Iceberg).

### 2.7 Q0b · la forma compartida (victor, 2026-10-23, `q0b.py` y `q0b_spark.py`)

Un pod del pool `pesado` (8 CPU / 48 GiB), como un trabajador de `ore-consulta`, con una instancia de
DuckDB por consulta; y un Spark Connect con un catálogo REST de mentira que apunta las cabeceras.

**1 · Una instancia por consulta.**

| | medido |
|---|---|
| crear y configurar una instancia, con `httpfs` y `spatial` | **199 ms** p50, 238 ms p95, y casi todo es cargar las extensiones (198 ms) |
| lo que ocupa ociosa | 5,4 MB |
| de «llega la consulta» a la primera fila (contar con filtro sobre 2,2 M lugares en el bucket) | 0,88–1,06 s: instancia 190–207 ms, vista 85–165 ms, lectura 574–684 ms |
| 8 consultas L a la vez (2 hilos cada una) | p50 2,4 s, p95 6,7 s, 2,2 por segundo |
| 32 a la vez | p50 7,8 s, p95 13,9 s, 3,5 por segundo: 64 hilos en 8 CPU, y cada instancia vuelve a leer del bucket |
| 4 consultas G a la vez, 8 GB de tope cada una | **las cuatro terminan** (muchos grupos 69 s, ventana 178–182 s, sin error); el pod llegó a **36,8 GB con 32 GB de topes**: un 12 % por encima de la suma |

⇒ Las extensiones se cargan una vez: un **montón de instancias ya preparadas**, a las que la admisión pone
rutas, secreto y tope y cierra (`lock_configuration`). Sin eso, 200 ms de más por consulta. La admisión
reparte **CPU, no sólo memoria**: con 2 hilos por consulta, unas 4 L a la vez por trabajador antes de que
se estorben. Los topes se reparten sobre un ~80 % del pod. Y no hay caché compartida entre instancias:
cada consulta vuelve a leer del bucket (queda abierto, §6).

**2 · El aislamiento** (`enable_external_access = false`, `allowed_directories` con el prefijo de sus
datasets y su derrame, configuración cerrada). **Negado:** otro prefijo del bucket, otro bucket, un prefijo
parecido (`vectorialX/`), `/etc/passwd`, `/proc/self/environ`, el token de Kubernetes, el derrame de otra
consulta, el servidor de metadatos de Google (el token del pod), la tabla de otra persona, reabrir el
acceso, ampliar las rutas, subir la memoria, cargar o instalar extensiones, adjuntar un fichero, escribir en
local o en el bucket, leer `/bin/sh`. **Permitido:** leer lo suyo, crear un secreto (que no sirve de nada
sin rutas), ver sus propios secretos.
- ⛔ **El agujero:** `vectorial/../spatialbench/sf100/customer.parquet` **leyó** fuera del prefijo, y con
  `%2e%2e` también. `allowed_directories` compara la URL como texto y no la normaliza.
- ⇒ La lista de rutas es la segunda capa, no la primera. La primera es la **credencial acotada por
  dataset** que presta `ore-serve` (W3.7 ②b): con ella, GCS niega lo de fuera aunque DuckDB lo pida.
  Además, la admisión rechaza `..` y `%2e` en cualquier ruta, y el trabajador corre **sin identidad
  propia** sobre el lago (una cuenta sin Workload Identity), así que el token del pod no abre nada.

**3 · De *Run* a empezar.** Lo de DuckDB está arriba (~0,2 s de instancia, o ~0 con instancias ya
preparadas). Lo que tarda `ore-serve` en resolver los nombres y prestar las credenciales **no está medido**
(no hay registro con tiempos de `POST /puestos/{id}/sql`): se mide en Q3, con la admisión ya hecha.

**4 · Spark compartido** (`q0b_spark.py`):
- una sesión **puede registrar su propio catálogo** en tiempo de ejecución, y cada petición lleva la
  cabecera de **su** sesión (Ana, «ana»; Bea, «bea»);
- ⛔ pero **Bea puede ponerse la cabecera de Ana** y el catálogo la manda tal cual, y **puede reescribir el
  catálogo que puso el servidor al arrancar** (`lago_srv`), aunque `isModifiable` diga `False`;
- una sesión nueva con el servidor caliente: 1 ms en crearse y 200–270 ms la primera consulta (la primera
  de todas, 0,8 s + 4,8 s de calentar la JVM).
- ⇒ En un Spark compartido, la identidad la dice el cliente. Sólo valdría con código nuestro en el
  servidor: un interceptor que autentique cada sesión, un `AuthManager` que tome la identidad de ahí (no
  de la configuración) y una regla que niegue `SET spark.sql.catalog.*`. Hasta tenerlo y demostrarlo, **un
  driver por persona**.

**5 · Un trabajador nuevo.** En un nodo caliente: admitido en 2 s y sirviendo antes de los 8 s (dos veces).
En un nodo a 0: nodo listo a los 94 s y corriendo a los 103 s (como en Q0). ⇒ El escalado por adelantado
cabe **dentro de un nodo encendido** en segundos. Un nodo nuevo hay que pedirlo ~100 s antes de necesitarlo,
o tener uno de sobra caliente.

## 3. El diseño

### 3.1 Tres principios

1. **El sitio se decide antes de correr, nunca después de fallar.** `ore-serve` estima lo que cada consulta
   va a leer y a ocupar (§3.4) y la admite en el motor y con los recursos que le tocan. No hay «probar en
   pequeño y subir si falla»: eso es reactivo y cuesta dos ejecuciones. Si la estimación duda, se elige
   lo grande: correr una consulta pequeña en un puesto grande y caliente cuesta milisegundos; una
   consulta que se cae, minutos.
2. **Cada motor es resiliente por construcción.** El DuckDB de un puesto tiene memoria y derrame con tope
   (Q1), por debajo de lo que su pod pide, así que lo que no cabe en memoria tarda más, pero no muere y no
   tumba el nodo. Spark adapta el plan (AQE) y añade ejecutores **mientras la consulta corre**
   (*dynamic allocation*, medido en CE y en §2.3): empieza con lo que hay y crece.
3. **No hay arranque en frío en el camino de la persona.** La consulta empieza siempre sobre capacidad que
   ya está encendida. Lo que se añade (nodos, ejecutores, puestos) se pide **por adelantado**, por la
   carga admitida y la prevista, y llega mientras se trabaja, nunca delante de una consulta. *(Lo que
   esto cuesta con puestos por persona es lo que mide Q4·0.)* Los 92 s y
   los ~3 min de §2.2 son justo lo que esta pieza hace desaparecer.

⇒ **El cliente no elige el motor ni ve el cómputo.** Escribe SQL y pulsa *Run*: no hay selector, no hay
«preparando más cómputo» y no hay reintentos visibles. Es lo contrario de Code Repositories, donde el
cliente elige el cómputo al crear el repositorio.

### 3.2 La forma: el puesto de la persona, del tamaño que toca

**Query no tiene motor propio.** Una hoja de Query es una sesión de puesto Python, como hoy: el mismo
SDK, el mismo `ore.sql`, el mismo SQL del árbol que **lee y escribe** (`sql_que_escribe`: `create or
replace dataset … as select`, `insert into … select`, vistas, vistas materializadas, schemas, bases,
`create media collection` y las derivadas, guiones de varias sentencias, 0039), las colecciones, las
Functions y las lecturas en vivo con su gobierno. Lo único que falta es **el tamaño**: hoy la hoja corre
siempre en el puesto ligero (2 CPU, 4 GiB), y lo grande no cabe (§2.1).

Todo lo demás ya existe y se cablea:

| pieza | qué es | de dónde sale |
|---|---|---|
| **la admisión** (en `ore-serve`) | antes de correr, para lo que la persona pulsa (una sentencia o un guion entero): reparto + estimación → clase L, G o E, con su memoria e hilos (§3.4) | Q2 y Q3, hechos: `ore explain --estimate`, `ore_core::admision`, `POST /consultas` |
| **L: el puesto ligero** | la sesión de la hoja, como hoy | `51-el-puesto.yaml` |
| **G: el puesto pesado** | **la misma sesión, en el pool `pesado`** (7 CPU / 48 GiB / 240 GiB de derrame, `cola-pesada`): la misma imagen, el mismo SDK y la misma celda; sólo cambian los recursos | 0068 C2·3, `cola.rs` |
| **E: Spark elástico** | el driver de un Build elástico y sus ejecutores por *dynamic allocation* | 0068 CE, `ore/spark.py` `_elastico()` |
| **sin espera** | puestos pesados ya encendidos, que se dan a quien la admisión mande a G | **lo único nuevo**; se decide con Q4·0 |

Lo que la persona pulsa **corre entero en una clase**: un guion no se parte entre puestos, porque lo que
crea la sentencia 1 lo lee la 2. La clase la pone su sentencia más grande. El estado de la sesión
(variables, tablas `tmp` de DuckDB) es del puesto en el que corrió; una hoja de Query no depende de él
entre ejecuciones.

**Por qué no un servicio compartido.** Q0b (§2.7) midió que una instancia de DuckDB por consulta, cerrada,
aísla a varias personas en un mismo proceso. Pero un proceso compartido obliga a un ejecutor distinto del
puesto, con la identidad prestada por consulta, y ese ejecutor tiene que volver a aprender todo lo que el
puesto ya hace. Si el coste de un puesto pesado por persona activa no se sostiene, lo que se comparte es
**la capacidad** (nodos encendidos, puestos preabiertos), no el motor.

### 3.3 Las tres clases

| clase | dónde corre | para | de *Run* a empezar |
|---|---|---|---|
| **L** | el puesto ligero de la hoja (DuckDB, 2 hilos, ~2 GB) | ≤ ~0,5 GB leídos y ≤ ~50 M filas | lo de hoy (la sesión ya está abierta) |
| **G** | el puesto pesado de la persona (DuckDB, 8 hilos, ~38 GB, derrame en la SSD) | lo que cabe en un nodo: medido hasta 600 M filas / 39 GB en ≤ 50 s | lo que mida Q4·0: un puesto preabierto, o uno nuevo en un nodo caliente (< 8 s, §2.7) |
| **E** | el Spark caliente, con ejecutores que crecen mientras corre | **sólo lo que no cabe en un nodo**: más de 38 GB de trabajo + 250 GiB de derrame, o un *join* espacial de dos lados grandes (C0: DuckDB no termina Q10–Q12) | segundos (driver y primer ejecutor encendidos); los demás ejecutores llegan durante la consulta |

**La clase intermedia es DuckDB, no Spark.** C0 midió DuckDB entre 2 y 25 veces más rápido que Spark
mientras cabe en un nodo, y G hace los 600 M en lo que la sesión tarda con 60 M. Spark vale por lo que no
cabe, no por la velocidad.

### 3.4 La regla de admisión (primera versión, con los números de Q0)

```
fuerza DuckDB:  lecturas en vivo · colecciones · Functions del árbol · lo que no pasa el chequeo
                de dialecto (§2.5)                                    → L o G, nunca E
L   si  gb ≤ 0,5  y  filas ≤ 50 M  y  la estimación es completa
G   si  no es L y cabe en un nodo:  memoria estimada ≤ 38 GB, derrame estimado ≤ 250 GiB
E   si  no cabe en un nodo, o un join espacial de dos lados por encima de la difusión de C3·a
sin estimación (un nombre sin snapshot, una sentencia que el reparto no entiende)  → L (Q4·1b; antes G)
```

- La estimación sale del reparto (columnas y filtros por tabla, ya existe) más los manifiestos de Iceberg
  (bytes por columna y fichero, mínimos y máximos): los mismos `gb` que Q0 cruzó con el tiempo real.
- La memoria de lo bloqueante (junta, ventana, distintos, muchos grupos) crece con las filas y no con los
  bytes; por eso la regla mira las dos cifras.
- Los umbrales son opciones de `ore-serve` (`--query-umbrales`), no constantes, y se ajustan con lo que
  cada consulta midió al acabar (la memoria y el derrame reales quedan en su registro).

### 3.5 La seguridad y el gobierno

**No cambian:** el puesto, ligero o pesado, es de una persona y corre con su identidad, como hoy; lo que
lee y escribe pasa por `ore-serve` con las credenciales acotadas de siempre. No hay proceso compartido
entre personas, así que las tres capas de Q0b (§2.7) no hacen falta aquí. Si un puesto pesado preabierto
se da a una persona, se le da **antes de su primera celda y sólo a ella**, y no vuelve a otra: lo que
la persona deja en `/trabajo` no lo ve nadie más.

- **Spark, uno por persona.** En un servidor compartido una sesión puede ponerse la cabecera de otra y
  reescribir el catálogo del servidor (§2.7). Un Spark compartido queda para cuando haya un interceptor
  que autentique cada sesión, un `AuthManager` que tome la identidad de ahí y una regla contra
  `SET spark.sql.catalog.*`, medidos con la misma sonda.
- **SQL sin Java:** una regla de chequeo en `ReglasDeOre` (`injectCheckRule`) niega `reflect`,
  `java_method` y `try_reflect`. Vale también para los Builds.
- **El disco:** cada puesto lleva su tope de derrame (Q1, y 240 GiB en el pesado); si algo se pasa, cae
  esa celda, no el nodo (§2.4).

### 3.6 Lo que el cliente ve

- *Run* y el resultado. Mientras corre, el progreso (DuckDB lo da; Spark, por etapas) y *Cancel*.
- El resultado se pagina y se descarga desde `ore-serve`, sin el tope de 200 filas de hoy: el puesto lo
  deja en Arrow y `ore-serve` lo sirve por páginas.
- Para soporte y para ajustar la regla, cada consulta deja registrado dónde corrió, por qué, cuánto leyó y
  cuánta memoria y derrame usó. Es información de la plataforma, no algo que se le pida al cliente.

## 4. Las decisiones que te tocan

| | decisión | recomiendo | por qué |
|---|---|---|---|
| **DQ1** | la forma | **el puesto de la persona, del tamaño que la admisión decide** (§3.2) | ✏️ **decidido 2026-10-23**, tras retirar `ore-consulta`: un solo runtime que ya lee y escribe todo; lo que falta es capacidad, no un motor |
| **DQ2** | la capacidad caliente | **la decide Q4·0** con lo medido: un puesto pesado preabierto por celda en horario, o ninguno y el arranque en nodo caliente | sin nada encendido vuelve el arranque en frío (§2.2); un puesto pesado por persona activa es un nodo pesado por persona. Orden de coste: la deuda B de CE estimaba ~6 $/día por un nodo de driver y uno pesado en horario |
| **DQ3** | la cuota | **la cola del puesto pesado (`cola-pesada`) se reparte entre Builds y Query**, con prioridad para Query, o una cola propia | si comparten la cuota pesada de la celda (7 CPU / 48 GiB, un pod), un Build deja a Query sin puesto o al revés. Una cola propia necesita subir la cuota global de CPU (32 hoy, pico 26) |
| **DQ4** | E compartido o por persona | **un driver por persona activa** (decidido por Q0b, §2.7) | en un Spark compartido la identidad la pone el cliente. Cuesta ~0,25 $/h por persona mientras está en Query (el nodo `pesado-driver`, donde caben dos drivers) |
| **DQ5** | el dialecto en E | **sólo con la traducción comprobada** (sqlglot en modo `RAISE` + la lista de §2.5); si no pasa, la consulta va a G | hay errores de traducción que no se ven (`qualify` con `limit`) |

## 5. El plan de construcción

Como siempre: cada paso con su previo, en local con commit, el binario antes que la malla y la malla antes
que el *flag*; un empuje al final de cada tramo que toque producción.

| paso | qué | dónde | hecho cuando |
|---|---|---|---|
| **Q0** | las medidas de §2 | victor | **hecho (2026-10-23)** |
| **Q0b** | el previo de la forma compartida: instancia por consulta, aislamiento, latencia, Spark compartido, trabajador nuevo | victor | **hecho (2026-10-23, §2.7)**: instancia sí (con montón preparado), aislamiento sí con la credencial acotada delante, Spark uno por persona, trabajador nuevo < 8 s en nodo caliente; la latencia de la admisión pasa a Q3 |
| **Q1** | **la defensa del disco** (no espera a Query): `/trabajo` con `sizeLimit: 4Gi` y `ORE_DISCO_MB` en `51-el-puesto.yaml` (y no `ephemeral-storage` en el contenedor: la cola de las sesiones no cubre ese recurso y Kueue no admitiría el pod); los tres SDK derraman hasta la mitad (`max_temp_directory_size`), y Python dice por qué falló; `cola.rs` sube el tope a 240 GiB para el puesto pesado, y no exige que una celda sin converger lo traiga; `gen-inquilino` ㉑ vigila que las dos cifras digan lo mismo | SDK, `ore-serve`, malla | **hecho en la rama `q1-disco-de-la-sesion`, sin empujar (2026-10-23).** Medido en victor con la forma nueva y el SDK de la rama: la ventana de 600 M se para a los **27 s con 1,9 GiB** derramados (antes: 10 GiB y 121 s), con el error legible, el pod vivo, `/trabajo` vuelto a 0 y el nodo del sistema **sin `DiskPressure`** ni desalojos. Para producción: el binario (ore-serve) y las imágenes, **luego** converger las celdas (la plantilla) |
| **Q2** | **el estimador**, en seco: `reparto` también para datasets y views del lago; por tabla, filas y bytes desde el manifiesto del snapshot vigente, lo devuelve `explain` (la caché por `metadata-location`, pendiente: hoy cada `explain` lee los manifiestos) | `ore-core`, `ore-serve` | **hecho en la rama, sin medir en victor (2026-10-23).** `ore-store estimar` (manifiestos, poda por mínimos y máximos, bytes de las columnas usadas), el reparto con los datasets del lago (`del_lago`), `ore explain --estimate` y el `explain` del puesto con estimación. Pruebas en ficheros reales en memoria; queda medir contra tablas grandes del lago de victor (±20 % de los `gb` de Q0) |
| **Q3** | **la admisión**: la regla de §3.4, pura y con sus pruebas (`ore_core::admision`: L, G o E, con su memoria e hilos sobre ~80 % del trabajador; sin estimación, G; nada en vivo va a E), el rechazo de `..`/`%2e` en los literales, y `POST /consultas {texto, rama?}` en `ore-serve`, que corre `ore explain --estimate` en la rama y contesta la clase, la razón, lo que lee y los ms. Resolver los nombres y prestar las credenciales de la consulta pasa a Q4, que es quien los usa | `ore-core`, `ore-serve` | la escalera de Q0 se admite: 6 M → L; 60 M y 600 M → G (60 M pasa de 50 M filas: con el trabajador caliente, G es más rápido); sin estimación → G; una ruta con `..` → 422; y medido cuánto tarda decidir |
| **Q4** | ~~`ore-consulta`, los trabajadores DuckDB~~ (con Q4b y Q5) — **retirado el 2026-10-23**, sin medir (Q4·1–Q4·3 y Q5·1–Q5·3 se empujaron apagados en `8c52d977`; Q4b no llegó a hacerse). Era un segundo ejecutor de sólo `SELECT` (§3.2). Se quitó: `puesto/python/ore_consulta.py` y sus pruebas; `ore._instancia(con)` del SDK; en `ore-serve` la cola de trabajadores y sus rutas (`GET /consultas/trabajo`, `POST /consultas/{id}/resultado`, `GET /consultas/{id}`, `GET /consultas/capacidad`), `--consultas`, `--consultas-techo` y el escalado; `ore_core::escalado`; `cola::rendir_consulta`; la cuenta `consulta` en `ore-entrada`; la malla (`58-la-consulta.yaml`, `58-la-consulta-de-quita-y-pon.yaml`, `plantilla-consulta.txt`, las exclusiones de `11` y `41`); y el arnés de su medida (`q4/encender.sh`, `q4/medir.py`). **Se queda** la admisión (Q3: `POST /consultas` sólo admite) y la siembra de la escalera (`pruebas-de-fuego/query/sembrar.py`). Lo retirado se recupera de `c586b4cf`, `26d48236` y `c54c25e9` si hiciera falta | — | **retirado** |
| **Q4·0** | ~~el previo de la forma nueva~~ — **aplazado el 2026-10-23**: se cablea primero y se mide con Query operativo y la escalera sembrada (`sembrar.py`). Lo que pedía medir (arranque del pesado para una persona, la escalera y una escritura de 600 M desde la hoja, el coste) pasa a Q4·3 | victor | — |
| **Q4·1a** | **la sesión de una hoja, también en G**: `ore-serve` abre el puesto de la hoja en el pool pesado (`cola-pesada`, los recursos de `cola.rs`) cuando se lo piden por dentro; misma imagen, mismo SDK, misma celda, sólo cambian los recursos. Ni la consola ni la persona lo piden: no hay selector | `ore-serve` | una sesión abierta en G corre la misma celda que en L | **hecho en local (2026-10-23)**: `Servidor::abrir_puesto_en_g` (sin quien lo llame hasta Q4·1b) abre `puesto-<persona>-spark` con la capa de Python; `POST /puestos` sigue abriendo sólo L. En G la celda corre en DuckDB con 48 GiB (sin `ORE_MOTOR`, que sólo pone un Build) |
| **Q4·1b** | **la hoja corre donde la admisión dice**: al pulsar *Run*, `ore-serve` admite lo pulsado (§3.2: entero, en la clase de su sentencia más grande) y lo manda al puesto L de la hoja o a su puesto G, abriéndolo si no lo hay; la celda es la misma (`sql_que_escribe`, guiones, colecciones, Functions, en vivo). **El cliente no elige ni ve el tamaño**: el escalado es nuestro. E corre en G con un aviso hasta Q7 | `ore-serve`, consola | lo pequeño sigue en L sin coste añadido; lo grande va a G sin que la persona haga nada | **hecho en local (2026-10-23), apagado tras `--query-g`**: en `POST /puestos/{id}/ejecutar`, una celda `sql` del puesto de una hoja (Python, sin repositorio ni trabajo) pasa por la admisión; si dice G o E, `ore-serve` abre el puesto G de la persona en la rama de la hoja, le encola la celda tal cual (un guion entero) y deja en la hoja **espejos** con los números que siguen en ella, que reciben cuándo empezó y la salida de G (`reflejar`, al dar la celda y al recibir su salida): `GET …/celdas/{n}` y el flujo de la hoja no cambian, y la consola tampoco. **Cambio sobre §3.4:** lo que no se estima (un `create schema`, lo que el reparto no entiende, un error) se queda en L —§3.4 decía G—: no se enciende un nodo de 64 GB sin saber que hace falta. Deuda: un espejo cuyo puesto G se pierde a media celda no recibe salida |
| **Q4·2** | **sin espera** (si Q4·0 lo pide): puestos pesados preabiertos sin dueño, que se dan a una persona antes de su primera celda y se van solos tras un rato sin uso; cuántos, por la actividad de la celda | `ore-serve`, malla | ninguna consulta G espera un arranque de nodo con la celda en horario |
| **Q4·3** | **medir con Query operativo** (lo que era Q4·0): sembrada la escalera, el arranque de G para una persona (nodo encendido y sin él), las nueve consultas y una escritura de 600 M desde la hoja, y el coste | victor | DQ2 y DQ3 decididas con números; sabido si hace falta Q4·2 |
| **Q6** | **el resultado y el ciclo**: progreso, *Cancel*, el resultado en Arrow paginado y descargable | puesto, `ore-serve`, consola | 10 M filas de resultado se pagina y se descarga sin pasar entero por la memoria de `ore-serve` |
| **Q7** | **la clase E**: un driver de Spark por persona activa (el de CE), encendido al entrar en Query y no a la primera consulta, sqlglot con el chequeo de §2.5 y la regla contra `reflect` en `ReglasDeOre` | imagen, SDK, `ore-serve`, `ReglasDeOre` | un *join* espacial que no cabe en un nodo empieza en segundos y gana ejecutores mientras corre; `reflect` da un error claro; lo de la lista prohibida va a G |
| **Q8** | **cierre**: la vara de punta a punta desde la consola en victor (la escalera como datasets del lago, más Q10–Q12 de SpatialBench para E, más concurrencia) y los objetivos | victor | los objetivos de abajo, medidos |

**Objetivos propuestos** (para aprobar en Q8):
- **De *Run* a empezar:** < 300 ms de más en L y G, segundos en E. Nunca un arranque de nodo delante.
- **L:** una consulta de ≤ 0,5 GB en < 3 s.
- **G:** cada consulta de la escalera de 600 M en < 60 s.
- **E:** los *joins* espaciales de SpatialBench a escala 100 en < 10 min.
- **Ninguna consulta muere por memoria o disco** en la vara: lo que no cabe en memoria derrama, y lo que
  no cabe en un nodo se admitió en E.
- **Con concurrencia:** la espera en cola p95 bajo el objetivo que fije Q0b.

## 6. Lo que queda abierto, con nombre

- **Q1, sin empujar.** Hasta que la plantilla esté en las celdas, una sesión que derrama sigue pudiendo llenar el nodo del sistema. Otras plantillas que derraman en `/trabajo` sin tope (`48-la-copia.yaml`, `58-el-motor.yaml`) no entran en Q1: mirarlas con el mismo patrón.
- **`allowed_directories` no normaliza las URL** (§2.7): avisar a DuckDB. La admisión sigue rechazando
  `..` y `%2e` en las rutas.
- **Lo retirado en la celda:** `ore-consulta` nunca se encendió en victor (Deployment a 0, sin
  `--consultas`). Al empujar, mirar que Flux quita el Deployment, la cuenta y sus políticas, y que
  `empujar-plantilla.py` saca `plantilla-consulta.txt` de la cola.
- **Spark compartido entre personas:** interceptor, `AuthManager` propio y regla contra
  `SET spark.sql.catalog.*` antes de reabrirlo.
- **El token de GCS de más de ~55 min** (deuda de CE): una consulta E larga.
- **La estimación de memoria de lo bloqueante** es por filas, no por cardinalidad: los manifiestos no dan
  distintos. Se ajusta con la vara de Q8.
- **Las lecturas en vivo** siguen con su tope (100 000 filas, 64 MB, 30 s por lectura): lo que lo pase no
  es un problema de motor, es una copia.
- **ORE Serverless Postgres** (0058) no entra en este enrutado: tiene su propio editor.
- Lo medido en §2.3 sobre Spark Connect sirve para el Spark caliente de la clase E y para un cliente externo
  (BI) cuando toque.

Fuentes: [MotherDuck · dual execution](https://motherduck.com/docs/key-tasks/running-hybrid-queries.md) ·
[Databricks · SQL warehouse behavior (IWM)](https://docs.databricks.com/aws/compute/sql-warehouse/warehouse-behavior) ·
[Spark Connect overview](https://spark.apache.org/docs/latest/spark-connect-overview.html) ·
ADR 0068 C0 y CE (`docs/decisions/0068-geo-compute.md` en `origin/main`).
