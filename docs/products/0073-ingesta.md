# 0073 · La ingesta: de una tubería de un carril a una tubería particionada

**Llevar los datos de un origen al lago a la velocidad que el hardware permite**, no a la que permite una tubería de
un solo carril. Este documento es **la hoja de ruta de la ingesta** mientras se itera, y será su **imagen de producto**
cuando la ingesta esté cubierta al nivel del estado del arte.

**Estado:** iteración 2 (2026-10-23) — **estado actual, cuellos, principios y lo medido**. El primer cuello grande
tenía una causa concreta y barata (el asignador de memoria de musl, §3 C4, PR #18); el origen, ya sin ese cuello,
satura en el laboratorio (§3 C5). El diseño detallado de la tubería particionada **no está escrito**: se escribe sobre
la pipeline completa medida con el driver arreglado (§8). Lo que está medido lo dice; lo que es hipótesis, también.

Sigue a [`0008`](0008-el-protocolo-del-driver.md) (el protocolo del driver), [`0016`](0016-el-testigo-y-el-rango.md)
(el testigo), [`0053`](0053-ore-federation-engine.md) (la pasarela) y [`0072`](0072-origin-hbase.md) (el origen que
se midió aquí). Toca al copiador residente (ADR 0027 en el código) y a `sellar-flujo` (ADR 0043).

## 0. En una página

- Copiar **1 000 000 filas** de HBase (`bench_filas`, 79 MB en Arrow, 22 MB en Parquet) cuesta hoy **216 s** por el Job
  de siempre, **201 s** por el copiador residente con la pasarela a 1 core y **85 s** con la pasarela a 4 cores.
- **Casi todo ese tiempo era el driver, y la causa era el asignador de memoria.** El `malloc` de musl (la imagen es
  Alpine) tiene un candado global; con 4 lectores en paralelo, el mismo driver leía el millón en **67 s** (136 s de CPU,
  54 s de sistema) y, con `mimalloc`, en **7,4 s** (14 s de CPU). Un cambio de 11 líneas (PR #18).
- **Ya sin ese cuello, el límite es el origen del laboratorio:** 10 M de filas en ~62 s y 50 M en 374 s
  (≈135 000-155 000 filas/s), con los 4 vCPU de la VM de HBase saturados y **133 MB** de memoria en el cliente.
- El objetivo de producto —del orden de **mil millones de filas en menos de una hora**, o **10 millones en segundos**—
  exige **entre 1,1 y 7 veces** lo que da el mejor driver contra este origen (135 000-250 000 filas/s), y mucho más de lo que da la tubería
  completa de hoy. Con un solo flujo, un solo escritor y un solo commit **no se llega**; ese es el argumento de fondo
  de este ADR, y sigue en pie, pero ya no hay que usarlo para explicar el primer 10×, que era un fallo de implementación.
- El estado del arte coincide en cinco ideas (§4); de ellas salen los **diez principios** de §5, que son lo que este
  ADR fija.

## 1. El objetivo

El tiempo de una ingesta es

```
T  =  filas / ( carriles × filas_por_segundo_por_carril )  +  costes_fijos
```

Dos palancas —**anchura** (carriles) y **eficiencia** (filas/s por carril)— y un término que no depende de las filas.
Con nuestro tamaño de fila (79 B en Arrow):

| Objetivo | Filas/s | MB/s de Arrow |
|---|---|---|
| 1 000 M de filas en 1 h | 280 000 | **22** |
| 100 M de filas en 1 min | 1 700 000 | 130 |
| 10 M de filas en 10 s | 1 000 000 | **79** |
| Pipeline de hoy, pasarela a 4 cores (1 M en 67 s de lectura) | 15 000 | 1,2 |
| Pipeline de hoy, pasarela a 1 core (1 M en 183 s de lectura) | 5 500 | 0,4 |
| **Driver solo, mimalloc, 4 lectores, 2 cores de cliente** (1 M en 7,4 s) | **135 000** | 10 |
| **Driver solo, mimalloc, 16 lectores, 6 cores de cliente** (1 M en 4,0 s) | **250 000** | 20 |
| **Driver solo, mimalloc, 8 lectores, 50 M en 374 s** | **134 000** | 10,6 |

⭐ **El presupuesto por fila** sale de la misma fórmula. A 1 M filas/s con 16 cores, cada fila puede costar **~16 µs de
CPU**. El driver con `mimalloc` gasta **~14 µs** por fila de 6 celdas: **dentro del presupuesto**. Con el `malloc` de
musl y 4 lectores gastaba **136 µs**.

## 2. El estado actual

### 2.1 La tubería de una copia

```
 ore materialize  (en el Job o en el copiador residente)
        │  POST /v1/read   perfil: "copia"
        ▼
 ore-federation   (la pasarela; 1 por inquilino; límite de 1 core, 1 GiB)
        │  lanza y habla con el driver
        ▼
 ore-read-hbase servir        ← parte la tabla por regiones (paralelo=4) PERO fusiona en UN flujo
        │  Arrow IPC por HTTP, un solo flujo
        ▼
 ore materialize ──pipe──▶ ore-store sellar-flujo    ← un iterador, lote a lote, una sola tabla
        │
        ▼
 Parquet + commit Iceberg en GCS  ──▶  puntero en la forja (git push)
```

- **El driver ya reparte por dentro.** `ore-read-hbase` parte la tabla en una parte por región (o grupo de regiones),
  hasta `paralelo=` (de 1 a 16; 4 por defecto), cada una con su conexión y su hilo ([`0072`](0072-origin-hbase.md)
  §7.4). Lo que sale del driver, sin embargo, es **un solo flujo Arrow**.
- **El almacén es secuencial.** `sellar-flujo` ([`ciclo.rs`](../../crates/ore-store/src/ciclo.rs), ADR 0043) lee un
  `StreamReader` y escribe **lote a lote**, sin tener la copia entera en memoria: es correcto en memoria y
  secuencial en tiempo. Hay un solo iterador y una sola tabla destino. Usa `iceberg` 0.10 de Rust
  (`RollingFileWriterBuilder` y `fast_append`).
- **Dos formas de lanzar la pasada.** El *Job* de la cola (`copiar-prog`, tres init containers y Kueue) y el
  *copiador residente* (`ore-copiador`, ADR 0027: `copiar-worker.sh`, 3 pasadas a la vez), que sólo atiende las copias
  acotadas —el alta de una base— y cae al Job si no está o no contesta. Ejecutan el mismo `ore materialize`.
- **Qué se salta.** Si el puntero ya describe la copia, `ore materialize` contesta «ya está» sin leer una fila del
  origen (una pasada sin cambios cuesta ~12 s, casi todos de arranque).
- **La imagen de los drivers es Alpine (musl).** `ore-read-hbase` es dinámico (Kerberos); no define ningún asignador.

### 2.2 Lo medido (`bench_filas`: 1 000 000 filas · 79 MB Arrow · 22 MB Parquet · 2026-10-23)

La tabla de origen es `bench:filas` del HBase de la VM `ore-labs` (un nodo, GCE `e2-standard-4`, **8 regiones** en un
mismo RegionServer); la lee un pod del clúster GKE de la misma región.

**Dónde se va el tiempo en el Job (216 s):**

| Tramo | Duración |
|---|---|
| Arranque del pod (tres init containers) | ~21 s |
| `ore` prepara: compila, abre la fuente, 49 punteros | 4,3 s |
| **Tubería: leer → Arrow → almacén → Parquet → GCS → commit** | **184,8 s** |
| Barrido de huérfanas | 0,3 s |
| Publicar el puntero | 1,6 s |

**Qué hace cada proceso durante esos 185 s** (muestreo cada 2 s, dos ejecuciones):
- `ore materialize` consume **1 s de CPU en ~100 s**: espera, no trabaja.
- `ore-read-hbase servir`, dentro de la pasarela, acumula **4 min 41 s de CPU** en tres lecturas.
- La pasarela marca **999m de CPU con límite de 1 core**, y `cpu.stat` da **4 532 períodos limitados de 7 543 (60 %)**,
  753 s de espera acumulada.

**El experimento del límite de la pasarela** (misma copia, `--rehacer`, una variable cambiada):

| | Pasarela a 1 core | Pasarela a 4 cores |
|---|---|---|
| Lectura de 1 M filas (log de la pasarela) | 180-183 s | **67 s** |
| Pasada completa en el copiador | 201 s | **86 s** |
| Períodos limitados | 60 % | **0** |

**El driver solo, dentro del clúster** (`ore-read-hbase leer`, 7 columnas, 79 MB de Arrow, salida a `/dev/null`, pod
con límite de 2 cores; dos repeticiones por caso, diferencias < 1 s):

| Asignador | `paralelo` | Lectura | CPU del cliente (usuario + sistema) |
|---|---|---|---|
| musl | 1 | 22-24 s | 11,3 s + 3,7 s = **15 s** |
| musl | 4 (el defecto) | 67-68 s | 82 s + 54 s = **136 s** |
| musl | 8 | 71,5 s | no medida |
| **mimalloc** | 1 | 18-19 s | 9,5-10,6 s + 0,5 s = **~11 s** |
| **mimalloc** | 4 | **7,4 s** | 13,3 s + 0,9 s = **14 s** |

Con 4 lectores, el pod con musl hizo **445 000 cambios de contexto en 12 s** frente a 13 000 con un lector (33×): el
patrón de hilos bloqueados en un candado. El experimento **sólo cambia el asignador** (`LD_PRELOAD` de la
`libmimalloc` de Alpine sobre el mismo binario).

**El coste crece con las celdas, no con las filas** (musl, `paralelo=1`, 10⁶ filas):

| Columnas leídas | Lectura |
|---|---|
| sólo la clave | 7,2 s |
| clave + 1 celda | 9,8 s |
| las 7 (clave, `_ts`, 5 celdas) | 26-29 s |

**A escala, driver con `mimalloc`, pod con 6 cores** (para quitar el tope de 2 cores del cliente):

| Tabla (regiones) | Filas | `paralelo` | Lectura | Filas/s | Pico de memoria del cliente |
|---|---|---|---|---|---|
| `bench:filas` (8) | 1 M | 4 / 8 / 16 | 6,0 / 6,0 / 4,0 s | 167 000 / 167 000 / 250 000 | 70 / 126 / 128 MB |
| `bench:filas_10m` (16) | 10 M | 8 | 62-65 s | ~155 000 | 131 MB |
| `bench:filas_10m` (16) | 10 M | 16 | 83,5 s | 120 000 | 253 MB |
| `bench:filas_50m` (80) | 50 M | 8 | **374 s** | **134 000** | **133 MB** |

Durante la lectura de 10 M, el contenedor de HBase marcó **~385 % de CPU: los 4 vCPU de la VM, saturados.**

## 3. Los cuellos de la estructura actual

Cada cuello dice si está **medido** o es **hipótesis**.

**C1 · La anchura después del driver es 1** *(estructura; del código)*. El driver parte por regiones, pero fusiona en
un flujo; la pasarela lo sirve por una conexión HTTP; `ore materialize` lo canaliza por un pipe. Cualquier paralelismo
que haya dentro del driver se estrecha en la salida. **Con el driver arreglado, este es el siguiente cuello
probable** (el driver ya da ~135 000 filas/s en un flujo): cuánto aguantan la pasarela y el escritor Parquet con ese
caudal **no está medido** (§8, fase B).

**C2 · Un solo escritor secuencial y un solo commit** *(estructura; del código)*. `sellar-flujo` consume un único
iterador de lotes. No hay un escritor por trozo, y el commit y el fichero no se reparten. Con 1 M de filas el resultado
es un dataset de 22 MB: **no hay ningún mecanismo que apunte a un tamaño de fichero** (el estado del arte apunta a
128-512 MB, §4).

**C3 · La pasarela, con 1 core, es un tope para las lecturas de copia** *(medido)*. Todas las lecturas de un inquilino
pasan por una sola pasarela con un límite de CPU de 1 core. Pasar a 4 cores bajó la lectura de 180 s a 67 s. Estaba
**escondido**: la CPU del pod de copia es de 75m y `ore materialize` no gasta nada. **Con el driver arreglado la
pasarela necesita mucha menos CPU** (el driver pasa de 136 s a 14 s de CPU por millón de filas), así que el límite
correcto se fija **después** de medir (§8, fase B) y no antes.

**C4 · El asignador de memoria de musl** *(medido; PR #18)*. La causa de que el driver, con el reparto en paralelo por
defecto, fuera **más lento que con un solo lector** y gastara 136 s de CPU por millón de filas. Mismo binario, mismo
origen, mismo millón de filas:

| `paralelo` | musl | mimalloc |
|---|---|---|
| 1 | 22-24 s · 15 s CPU | 18-19 s · ~11 s CPU |
| 4 | **67-68 s · 136 s CPU** | **7,4 s · 14 s CPU** |

La lectura reserva memoria por celda; musl guarda un candado global en `malloc`, y varios hilos reservando a la vez se
bloquean entre sí (54 s de CPU de sistema, 445 000 cambios de contexto en 12 s). El reparto por regiones **ya
funcionaba**: el asignador lo anulaba. La comparación con el laboratorio de `0072` (3,1 s por millón) se explica, al
comparar lo comparable, por esto y por la anchura de la fila: el laboratorio leía una celda por fila y aquí hay cinco.
*(Un primer análisis atribuyó el gap a que el driver gastaba ~100-170 µs por fila: era la patología del asignador, no
el coste del driver, que es de ~14 µs.)*

**C5 · El origen satura** *(medido, en el laboratorio)*. Con el driver arreglado y CPU de sobra en el cliente, los
4 vCPU de la VM de HBase se saturan al ~385 % y el rendimiento se queda en **~135 000-155 000 filas/s** (250 000 en
tablas pequeñas que caben en caché). Más lectores **empeoran** (16 lectores sobre 10 M: 83 s frente a 62 s con 8), porque
compiten por el mismo servidor. Este es **el techo de un RegionServer de 4 vCPU compartido con otros tres contenedores**,
no el de HBase en general: en un clúster con varios RegionServers, o leyendo los HFiles de un snapshot sin pasar por
ellos (§4.2), el techo se mueve. **No se ha medido ninguna de las dos.**

**C6 · Costes fijos** *(medido)*. El Job paga **~21 s** de arranque; el copiador residente los evita pero paga **~8 s**
de testigos y clon del árbol, más ~4 s de `ore` preparando; y toda pasada paga ~2 s al final (huérfanas y push).
Con un objetivo de «10 M de filas en 10 s», **los costes fijos ya se comen el presupuesto**.

**C7 · No hay rutas distintas para lo entero y lo incremental en el camino caliente** *(estructura)*. La pasada
sin cambios es barata (12 s), pero la que sí cambia relee la fuente entera, salvo que el origen sepa decir qué cambió
(HBase con `_ts`, Phoenix con `ROW_TIMESTAMP`, Mongo con `_id`; Cassandra no: su testigo no es fiable). El coste de lo
entero crece con la tabla; el de lo incremental, con el cambio.

**C8 · La puerta de la CI tarda lo que el job más lento** *(medido, operativo)*. El despliegue depende de que **todos**
los jobs estén en verde; dos de ellos (`conectores` y `hbase-kerberos`) tardan ~20 min o más. No afecta al
rendimiento de la ingesta, pero sí al ciclo de iteración de esta hoja de ruta (§6, L6).

## 4. Lo que enseña el estado del arte

Cinco ideas, con la fuente que las respalda. **Donde la fuente no da una cifra, no se inventa una.**

1. **Se lee por rangos independientes, en paralelo, no por un scan secuencial.** En Cassandra, el Spark connector
   parte el anillo de tokens en piezas que siguen la propiedad de los nodos, con un tamaño por defecto de **512 MB**
   por partición ([DataStax](https://docs.datastax.com/en/dse-spark-connector-api/6.8/com/datastax/spark/connector/rdd/CassandraTableScanRDD.html)).
   En SQL, ConnectorX parte la consulta y asigna **un hilo por partición**; el proyecto reporta hasta **21× menos
   tiempo** con 4 cores frente a las alternativas ([resumen](https://towardsdatascience.com/connectorx-the-fastest-way-to-load-data-from-databases-a65d4d4062d5)).
2. **Cuando se puede, se evita el servidor del origen.** HBase ofrece `TableSnapshotInputFormat`: lee **los HFiles
   directamente del almacenamiento**, con un split por región y sin RegionServers; la documentación lo presenta como
   la ruta de máximo rendimiento ([HBase](https://hbase.apache.org/docs/architecture/snapshot-scanner/)).
   *No hay medidas publicadas de cuánto más rápido es; habría que medirlo aquí.* **El §3 C5 dice por qué importa:**
   cuando el cliente es eficiente, el RegionServer es lo que satura.
3. **Se escribe en paralelo hacia ficheros de tamaño razonable, y se confirma de una vez.** En Iceberg el objetivo por
   defecto es **512 MB** y el rango recomendado **128-512 MB** ([AWS](https://docs.aws.amazon.com/prescriptive-guidance/latest/apache-iceberg-on-aws/best-practices-read.html),
   [Dremio](https://www.dremio.com/?p=25802)). El commit es **optimista**: el escritor prepara los ficheros y el
   manifiesto, y mueve el puntero de metadatos con un *compare-and-swap*; si otro escritor llegó antes, el commit se
   rechaza y se reintenta contra la nueva base. **Varios ficheros de datos caben en un solo commit** (`newAppend()`,
   `appendFile()` por fichero, `commit()`), y en un *append* el reintento es barato porque el manifiesto se reutiliza
   ([Dremio](https://www.dremio.com/blog/apache-iceberg-concurrency-commit-conflicts/), [Iceberg Go](https://go.iceberg.apache.org/concurrent-writes.html)).
   *Las fuentes son blogs y la documentación de la implementación en Go; no confirman el comportamiento de
   `iceberg-rust`.* Nuestro almacén usa `RollingFileWriterBuilder` y `fast_append` de `iceberg` 0.10 y **su catálogo es
   el puntero de la forja** (el commit es el *swap* de ese puntero, con rebase si la forja avanzó), así que la
   atomicidad del añadido de N ficheros es la del `fast_append` y la del `git push`, no una cosa nueva.
4. **El formato es columnar de punta a punta.** Arrow evita convertir fila a fila; Arrow Flight SQL, con dos nodos de
   8 vCPU, sostuvo **270-350 MB/s** en una prueba de Teradata, **15-30×** JDBC ([Teradata](https://www.teradata.com/insights/data-architecture/how-fast-is-arrow-flight-sql-with-teradata)).
   *Es la cifra de otro motor.*
5. **Lo incremental gana a lo entero.** No releer lo que no cambió es más barato que cualquier lectura rápida
   (consecuencia de la fórmula de §1: el término que no depende de las filas es el que queda al final).

⛔ **Lo que la búsqueda no dio:** ninguna cifra de filas/s por segundo de HBase, Cassandra, Phoenix o Mongo que sea
comparable con la nuestra. Todo número de «cuánto mejora» de este documento sale de **nuestras** medidas, no de la
literatura.

## 5. Los principios

Lo que este ADR **fija**, y contra lo que se juzgará cada decisión de diseño.

| # | Principio | De dónde sale |
|---|---|---|
| **P1** | **La anchura va antes que la velocidad por carril.** El tiempo es `filas / (carriles × tasa)`; con un carril, ninguna optimización llega a los objetivos. | §1, C1, C2 |
| **P2** | **El origen declara cómo se parte; el núcleo reparte.** Un origen sólo implementa dos cosas: `particionar` (qué trozos independientes tiene la tabla) y `leer(trozo)` (un flujo Arrow). | §4.1, §7 |
| **P3** | **Un carril es un lector y un escritor, sin estado compartido,** y hay **un solo commit atómico** al final que añade todos los ficheros. Un trozo que falla se reintenta solo, sin empezar de cero. | §4.3, C2 |
| **P4** | **Arrow de punta a punta, en lotes.** Nada fila a fila fuera del driver, nada que reserve memoria proporcional a la tabla. | §4.4, ADR 0043, §2.2 (133 MB a 50 M) |
| **P5** | **El tamaño del fichero lo manda un objetivo (128-512 MB),** no la pasada. Un dataset pequeño es un fichero; uno grande, muchos. | §4.3, C2 |
| **P6** | **Saltarse el servidor es una implementación más de `leer(trozo)`,** no otra tubería: cuando el origen lo permite (HFiles de un snapshot de HBase), el trozo es un fichero en vez de un rango de claves. | §4.2, C5 |
| **P7** | **Lo incremental antes que lo entero.** Lo entero es la excepción justificada (`--rehacer`), y cada origen declara qué testigo tiene (`_ts`, `ROW_TIMESTAMP`, `_id`) o que no lo tiene. | §4.5, C7 |
| **P8** | **Los costes fijos no están en el camino caliente.** El arranque, el clon y los testigos se pagan antes de que llegue la petición o se reparten entre varias; una pasada es el trabajo, no el montaje. | C6 |
| **P9** | **Un núcleo, N orígenes.** Especializar es escribir `particionar` y `leer`; el reparto, el reintento, la escritura, el commit, el puntero y los informes **no** se repiten por origen. | §4, §7 |
| **P10** | **Cada etapa se mide por separado y el número se queda en este documento.** No hay objetivo sin su medida, ni cifra de diseño sin su origen (medida, fuente o hipótesis declarada). Un experimento cambia **una** variable. | L1-L4, L8 |

## 6. Lo aprendido

Lo que se descubrió **trabajando** y no estaba en el diseño. Varias cosas que se dijeron durante la iteración
resultaron incompletas o falsas; se dejan aquí.

**L1 · Medir los límites del contenedor antes de culpar al código.** El cuello visible era «el almacén» y el pod de
copia gastaba 75m de CPU; el cuello real estaba en otro pod, con el 60 % de sus períodos limitados. `cpu.stat`
(`nr_throttled`, `throttled_usec`) habría ahorrado una hora. *Regla:* una lectura que no gasta CPU en quien la pide
es una lectura que **espera**; se mira a quién.

**L2 · «El driver no sabe repartir» era falso.** `ore-read-hbase` reparte por regiones desde `0072`. Lo que no reparte
es lo que viene después. El cuello es la forma del conjunto, no una pieza.

**L3 · Las cifras del laboratorio sí se trasladan, si se compara lo comparable.** `0072` midió 3,1 s por millón con una
celda por fila; dentro del clúster salían 67 s con cinco celdas por fila y el asignador de musl. Con el mismo driver
bajo `mimalloc`, 7,4 s. El «gap sin explicar» eran tres diferencias a la vez (celdas por fila, reparto en paralelo
roto por el asignador, nada más); se separaron cambiando **una** variable cada vez.

**L4 · El copiador residente ahorra el arranque, no el rendimiento.** Su pasada tarda 201 s frente a ~216 s del Job:
ahorra ~15-20 s de montaje y nada de lectura. Es la mejora correcta para **muchas copias pequeñas**, y no la
respuesta a **una tabla grande**. Por eso las dos cosas (P8 y P1) son principios distintos.

**L5 · Una pasada que no cambia nada cuesta 12 s.** Eso fija el suelo de lo incremental hoy, y dice que el 90 % de
ese tiempo es montaje (testigos, clon), no trabajo.

**L6 · Lo operativo también es parte del cuello.** El despliegue pasa por Flux y por una CI cuya puerta es el job más
lento; la etiqueta `main` del registro decide qué corre. Un cambio manual (imagen, límites, reinicios) **se deshace
solo** en la siguiente reconciliación de Flux, y los pods recreados pierden lo que se haya copiado dentro. Las pruebas
que cambien límites o imágenes deben hacerse sabiendo que son temporales, y el cambio definitivo va a `malla/`.

**L7 · Dos despliegues rotos seguidos tuvieron una sola causa.** El binario del copiador no estaba en la imagen
publicada porque la CI de `main` fallaba, y fallaba porque la prueba de huellas no conocía un fichero que el
Dockerfile ya copiaba (PR #16). *Regla:* una prueba que copia a mano la lista de lo que el Dockerfile copia se
desincroniza; mejor que la derive del propio Dockerfile.

**L8 · En musl, toda lectura en paralelo lleva asignador propio.** El `malloc` de musl es correcto y lento bajo
contención; un binario que reserva memoria por celda y reparte en hilos lo paga con creces. *Regla:* ante un
programa multihilo que gasta más CPU de sistema que de usuario, el primer experimento es un `LD_PRELOAD` de otro
asignador sobre el **mismo** binario: cuesta minutos, no cambia el código y separa la causa de las sospechas. Los demás
drivers y `ore-store` son musl; **sin medir, no se les cambia nada**.

**L9 · Medir con holgura, o se mide el tope.** Con el cliente limitado a 2 cores, 4, 8 y 16 lectores daban lo mismo
(el cliente era el cuello); con 6 cores, 16 lectores bajaron un millón de filas de 6,0 a 4,0 s. Antes de decir «más
paralelismo no ayuda» se comprueba que lo que limita no es el que mide.

**L10 · Las tablas de prueba tienen que parecerse a las de producción.** `0072` medía una celda por fila; la tabla
real de la conversación tiene cinco celdas más `_ts` (que obliga a leer filas enteras). El coste de HBase sube con las
celdas (7,2 s la clave sola, 26-29 s las siete columnas). Los bancos de pruebas de la ingesta (`bench:filas`,
`bench:filas_10m`, `bench:filas_50m`) llevan el mismo esquema, y las de 10 M y 50 M están **pre-divididas** (16 y 80
regiones).

## 7. La forma de la solución (esbozo)

Sólo lo que los principios ya determinan; el detalle se escribe cuando exista la medida de la pipeline completa (§8).

- **Interfaz del driver** (P2): `particionar(objeto, n_objetivo) → [trozo{rango, estimación}]` y
  `leer(trozo) → flujo Arrow`. Hoy un driver implementa, en la práctica, `leer(todo)`; el reparto por regiones de
  HBase se mueve **de dentro del driver al núcleo**.
- **Núcleo común** (P1, P3, P5, P9): reparte los trozos entre N trabajadores, cada uno con su lector y su escritor
  (un fichero de tamaño objetivo), reintenta por trozo y hace **un** commit Iceberg que añade todos los ficheros
  (§4.3: `fast_append` con los N ficheros, y el *swap* del puntero de la forja).
- **Especialización** (P6, P7): cada origen declara sus trozos, su testigo y, si procede, su vía de saltarse el servidor.

|  | Rango natural | `leer(trozo)` | Testigo |
|---|---|---|---|
| HBase | regiones / rangos de clave | scan por rango, o HFile de un snapshot | `_ts` de celda |
| Cassandra | rangos de tokens | CQL por rango de tokens | ninguno fiable |
| Phoenix | guideposts / rangos de clave | SQL con rango | `ROW_TIMESTAMP` |
| MongoDB | rangos de `_id` | cursor por rango | `_id` / change streams |

⭐ Un riesgo de diseño que se deja escrito: si la interfaz se diseña sólo con HBase, quedará sesgada hacia rangos de
claves ordenados. **Se valida con un segundo origen de forma distinta** (tokens sin orden, o `_id`) antes de darla por
buena.

⭐ **Un dato de dimensionado ya medido** (C5): el mejor reparto contra este origen fue de **8 lectores sobre 16
regiones** (155 000 filas/s); con 16 lectores empeoró. El número de trozos concurrentes no es «tantos como regiones»,
sino lo que el origen aguanta. `particionar` devuelve los trozos; **cuántos se leen a la vez lo decide el núcleo**, con
un tope por origen que el origen declara.

## 8. Hoja de ruta

Cada fase termina con **su número en este documento** (P10).

| Fase | Qué | Estado |
|---|---|---|
| **A** | `mimalloc` en `ore-read-hbase` para musl ([PR #18](https://github.com/describeloai/ore/pull/18)). Verificar tras desplegar que `paralelo=4` baja de ~67 s a ~7 s sin `LD_PRELOAD`. | en CI |
| **B** | **Línea base de la pipeline completa con el driver arreglado:** copia de 1 M y de 10 M de filas, y qué etapa limita ahora (pasarela, escritor Parquet, commit). Con ella se fija el límite de CPU de la pasarela en `malla/` y se decide si ese PR de CPU hace falta. | pendiente de A |
| **C** | Medir la misma lectura con `mimalloc` en los otros drivers (Cassandra, Phoenix, Mongo, Postgres) y en `ore-store`/`ore materialize`, uno a uno, cambiando sólo el asignador. | pendiente |
| **D** | Interfaz `particionar`/`leer` y reparto en el núcleo, con HBase. | por diseñar (tras B) |
| **E** | Un segundo origen de forma distinta contra la misma interfaz. | por diseñar |
| **F** | Escritura en paralelo, tamaño de fichero objetivo y un solo commit con N ficheros. | por diseñar |
| **G** | Vía de snapshot para HBase, **si** una medida en un clúster con más de un RegionServer muestra que el servidor sigue siendo el límite (C5). | por decidir |
| **H** | Costes fijos fuera del camino caliente (copiador caliente, testigos y clon reutilizados). | por diseñar |

**Siguiente iteración de este documento:** la pipeline completa medida (fase B) y, con ella, el diseño detallado de la
interfaz y del reparto (fases D-F).

## Fuentes

- [HBase · snapshot scanner](https://hbase.apache.org/docs/architecture/snapshot-scanner/)
- [DataStax Spark connector · `CassandraTableScanRDD`](https://docs.datastax.com/en/dse-spark-connector-api/6.8/com/datastax/spark/connector/rdd/CassandraTableScanRDD.html)
- [ConnectorX (Towards Data Science)](https://towardsdatascience.com/connectorx-the-fastest-way-to-load-data-from-databases-a65d4d4062d5)
- [AWS · Iceberg, lectura](https://docs.aws.amazon.com/prescriptive-guidance/latest/apache-iceberg-on-aws/best-practices-read.html)
- [Dremio · escritura inteligente en Iceberg](https://www.dremio.com/?p=25802)
- [Dremio · concurrencia y conflictos de commit en Iceberg](https://www.dremio.com/blog/apache-iceberg-concurrency-commit-conflicts/)
- [Iceberg Go · escrituras concurrentes](https://go.iceberg.apache.org/concurrent-writes.html)
- [Teradata · Arrow Flight SQL](https://www.teradata.com/insights/data-architecture/how-fast-is-arrow-flight-sql-with-teradata)
