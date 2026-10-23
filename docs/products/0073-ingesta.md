# 0073 · La ingesta: de una tubería de un carril a una tubería particionada

**Llevar los datos de un origen al lago a la velocidad que el hardware permite**, no a la que permite una tubería de
un solo carril. Este documento es **la hoja de ruta de la ingesta** mientras se itera, y será su **imagen de producto**
cuando la ingesta esté cubierta al nivel del estado del arte.

**Estado:** iteración 1 (2026-10-23) — **estado actual, cuellos y principios**. Todavía no hay diseño detallado ni
código; las medidas que faltan para fijar el diseño se añaden en la iteración siguiente (§8). Lo que está medido lo
dice; lo que es hipótesis lo dice también.

Sigue a [`0008`](0008-el-protocolo-del-driver.md) (el protocolo del driver), [`0016`](0016-el-testigo-y-el-rango.md)
(el testigo), [`0053`](0053-ore-federation-engine.md) (la pasarela) y [`0072`](0072-origin-hbase.md) (el origen que
se midió aquí). Toca al copiador residente (ADR 0027 en el código) y a `sellar-flujo` (ADR 0043).

## 0. En una página

- Copiar **1 000 000 filas** de HBase (`bench_filas`, 79 MB en Arrow, 22 MB en Parquet) cuesta hoy **216 s** por el Job
  de siempre, **201 s** por el copiador residente con la pasarela a 1 core y **85 s** con la pasarela a 4 cores.
- El objetivo de producto —del orden de **mil millones de filas en menos de una hora**, o **10 millones en segundos**—
  exige entre **18 y 67 veces** más rendimiento que el mejor resultado de hoy (15 000 filas/s).
- **No es un problema de hardware**: en MB/s los objetivos son modestos (22-79 MB/s). Es un problema de **forma**: la
  tubería tiene **anchura 1** después del driver (un flujo, un escritor, un commit), y no hay nada en su diseño que
  permita subirla.
- Parte del gap **no se explica por la forma**: el mismo driver mide 3,1 s por millón de filas en el laboratorio de
  `0072` y ~67 s dentro del clúster (§3, C4). Eso hay que entenderlo **antes** de dar por buena cualquier cifra de diseño.
- El estado del arte coincide en cinco ideas (§4); de ellas salen los **diez principios** de §5, que son lo que este
  ADR fija. El detalle de cómo se construye viene después, y se apoya en medidas que aún no tenemos.

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
| **Hoy, pasarela a 4 cores** (1 M en 67 s de lectura) | **15 000** | 1,2 |
| **Hoy, pasarela a 1 core** (1 M en 183 s de lectura) | 5 500 | 0,4 |

Estar a **18× de «mil millones por hora»** y a **67× de «10 M en 10 s»** con un hardware que sobra (los objetivos
piden decenas de MB/s) es la prueba de que el límite no está en la máquina.

⭐ **El presupuesto por fila** sale de la misma fórmula. A 1 M filas/s con 16 cores, cada fila puede costar **~16 µs de
CPU**. Hoy, la pasarela gasta del orden de **100-170 µs** por fila (§3, C4).

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
  hasta `paralelo=` (4 por defecto), cada una con su conexión y su hilo ([`0072`](0072-origin-hbase.md) §7.4). Lo que
  sale del driver, sin embargo, es **un solo flujo Arrow**.
- **El almacén es secuencial.** `sellar-flujo` ([`ciclo.rs`](../../crates/ore-store/src/ciclo.rs), ADR 0043) lee un
  `StreamReader` y escribe **lote a lote**, sin tener la copia entera en memoria: es correcto en memoria y
  secuencial en tiempo. Hay un solo iterador y una sola tabla destino.
- **Dos formas de lanzar la pasada.** El *Job* de la cola (`copiar-prog`, tres init containers y Kueue) y el
  *copiador residente* (`ore-copiador`, ADR 0027: `copiar-worker.sh`, 3 pasadas a la vez), que sólo atiende las copias
  acotadas —el alta de una base— y cae al Job si no está o no contesta. Ejecutan el mismo `ore materialize`.
- **Qué se salta.** Si el puntero ya describe la copia, `ore materialize` contesta «ya está» sin leer una fila del
  origen (una pasada sin cambios cuesta ~12 s, casi todos de arranque).

### 2.2 Lo medido (`bench_filas`: 1 000 000 filas · 79 MB Arrow · 22 MB Parquet · 2026-10-23)

La tabla de origen es `bench:filas` del HBase de la VM `ore-labs` (un nodo, GCE `e2-standard-4`); la lee un pod del
clúster GKE de la misma región.

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

**El experimento que lo separa** (misma copia, `--rehacer`, una variable cambiada):

| | Pasarela a 1 core | Pasarela a 4 cores |
|---|---|---|
| Lectura de 1 M filas (log de la pasarela) | 180-183 s | **67 s** |
| Pasada completa en el copiador | 201 s | **86 s** |
| Períodos limitados | 60 % | **0** |
| CPU de la pasarela durante la lectura | ≈1 core (en el tope) | ≈2,6 cores de media |

Y un dato de escalado, con el límite de 1 core: una lectura cortada por desconexión leyó **532 480 filas en 99,4 s**
(5 357 filas/s) y la completa **1 000 000 en 183 s** (5 464 filas/s): el rendimiento es el mismo, el tiempo crece en
proporción a las filas. **No hay indicio de crecimiento mayor que lineal**, pero no se ha medido más allá de un millón.

## 3. Los cuellos de la estructura actual

Cada cuello dice si está **medido** o es **hipótesis**.

**C1 · La anchura después del driver es 1** *(estructura; del código)*. El driver parte por regiones, pero fusiona en
un flujo; la pasarela lo sirve por una conexión HTTP; `ore materialize` lo canaliza por un pipe. Cualquier paralelismo
que haya dentro del driver se estrecha en la salida.

**C2 · Un solo escritor secuencial y un solo commit** *(estructura; del código)*. `sellar-flujo` consume un único
iterador de lotes. No hay un escritor por trozo, y el commit y el fichero no se reparten. Con 1 M de filas el resultado
es un dataset de 22 MB: **no hay ningún mecanismo que apunte a un tamaño de fichero** (el estado del arte apunta a
128-512 MB, §4).

**C3 · La pasarela, con 1 core, es un tope para las lecturas de copia** *(medido)*. Todas las lecturas de un inquilino
pasan por una sola pasarela con un límite de CPU de 1 core. Es **el** cuello que se midió: pasar a 4 cores baja la
lectura de 180 s a 67 s. Es el cuello que menos enseña, porque se arregla con un número en `malla/`; pero estaba
**escondido** (la CPU del pod de copia es de 75m, y `ore materialize` no gasta nada), y es una instancia de un
patrón: **hay un recurso compartido por todo el inquilino en el camino caliente de cada copia**.

**C4 · El driver gasta mucha más CPU por fila en el clúster que en el laboratorio** *(medido el efecto; causa sin
determinar)*.

| Medida | Condiciones | Resultado |
|---|---|---|
| `0072` §3.1 / §7 | laboratorio local, 4 regiones, un nodo, release, Arrow incluido | **3,1 s** por 10⁶ filas (≈320 000 filas/s) |
| `0072` §3.1 | una sola conexión | 9,5 s (≈105 000 filas/s) |
| Hoy, en el clúster | pasarela a 4 cores | **67 s** (≈15 000 filas/s) |
| Hoy, en el clúster | pasarela a 1 core | 183 s (≈5 500 filas/s) |
| Hoy | CPU por fila, por la pasarela | **~100-170 µs** frente a ≲12 µs en el laboratorio |

Ni siquiera con una sola conexión el laboratorio baja de 9,5 s: **entre 7 y 19 veces** más rápido que el clúster con
el mismo driver. Las condiciones difieren en varias cosas a la vez y **no se han aislado**: el origen está en otra
máquina y se llega por red (no por `localhost`), la tabla puede tener una sola región (con una región, `paralelo=4`
no abre cuatro lectores), la pasarela añade su propio salto HTTP y su serialización, y la imagen del clúster es
distinta de la del laboratorio. **Mientras no se explique este gap, las estimaciones de cuánto ganará el diseño
particionado son una hipótesis.**

**C5 · Costes fijos** *(medido)*. El Job paga **~21 s** de arranque; el copiador residente los evita pero paga **~8 s**
de testigos y clon del árbol, más ~4 s de `ore` preparando; y toda pasada paga ~2 s al final (huérfanas y push).
Con un objetivo de «10 M de filas en 10 s», **ya los costes fijos se comen el presupuesto**.

**C6 · No hay rutas distintas para lo entero y lo incremental en el camino caliente** *(estructura)*. La pasada
sin cambios es barata (12 s), pero la que sí cambia relee la fuente entera, salvo que el origen sepa decir qué cambió
(HBase con `_ts`, Phoenix con `ROW_TIMESTAMP`, Mongo con `_id`; Cassandra no: su testigo no es fiable). El coste de lo
entero crece con la tabla; el de lo incremental, con el cambio.

**C7 · La puerta de la CI tarda lo que el job más lento** *(medido, operativo)*. El despliegue depende de que **todos**
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
   *No hay medidas publicadas de cuánto más rápido es; habría que medirlo aquí.*
3. **Se escribe en paralelo hacia ficheros de tamaño razonable.** En Iceberg el objetivo por defecto es **512 MB** y el
   rango recomendado **128-512 MB**: ficheros muy pequeños disparan el coste de metadatos, ficheros muy grandes
   reducen el paralelismo de lectura ([AWS](https://docs.aws.amazon.com/prescriptive-guidance/latest/apache-iceberg-on-aws/best-practices-read.html),
   [Dremio](https://www.dremio.com/?p=25802)). *No se encontró una fuente fiable sobre cuántos escritores en paralelo
   conviene por commit.*
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
| **P2** | **El origen declara cómo se parte; el núcleo reparte.** Un origen sólo implementa dos cosas: `particionar` (qué trozos independientes tiene la tabla) y `leer(trozo)` (un flujo Arrow). | §4.1, §6 |
| **P3** | **Un carril es un lector y un escritor, sin estado compartido,** y hay **un solo commit atómico** al final que añade todos los ficheros. Un trozo que falla se reintenta solo, sin empezar de cero. | §4.3, C2 |
| **P4** | **Arrow de punta a punta, en lotes.** Nada fila a fila fuera del driver, nada que reserve memoria proporcional a la tabla. | §4.4, ADR 0043 |
| **P5** | **El tamaño del fichero lo manda un objetivo (128-512 MB),** no la pasada. Un dataset pequeño es un fichero; uno grande, muchos. | §4.3, C2 |
| **P6** | **Saltarse el servidor es una implementación más de `leer(trozo)`,** no otra tubería: cuando el origen lo permite (HFiles de un snapshot de HBase), el trozo es un fichero en vez de un rango de claves. | §4.2 |
| **P7** | **Lo incremental antes que lo entero.** Lo entero es la excepción justificada (`--rehacer`), y cada origen declara qué testigo tiene (`_ts`, `ROW_TIMESTAMP`, `_id`) o que no lo tiene. | §4.5, C6 |
| **P8** | **Los costes fijos no están en el camino caliente.** El arranque, el clon y los testigos se pagan antes de que llegue la petición o se reparten entre varias; una pasada es el trabajo, no el montaje. | C5 |
| **P9** | **Un núcleo, N orígenes.** Especializar es escribir `particionar` y `leer`; el reparto, el reintento, la escritura, el commit, el puntero y los informes **no** se repiten por origen. | §4, §6 |
| **P10** | **Cada etapa se mide por separado y el número se queda en este documento.** No hay objetivo sin su medida, ni cifra de diseño sin su origen (medida, fuente o hipótesis declarada). | L1-L4 |

## 6. Lo aprendido

Lo que se descubrió **trabajando** y no estaba en el diseño. Varias cosas de las que se dijeron durante la iteración
resultaron incompletas o falsas; se dejan aquí.

**L1 · Medir los límites del contenedor antes de culpar al código.** El cuello visible era «el almacén» y el pod de
copia gastaba 75m de CPU; el cuello real estaba en otro pod, con el 60 % de sus períodos limitados. `cpu.stat`
(`nr_throttled`, `throttled_usec`) habría ahorrado una hora. *Regla:* una lectura que no gasta CPU en quien la pide
es una lectura que **espera**; se mira a quién.

**L2 · «El driver no sabe repartir» era falso.** `ore-read-hbase` reparte por regiones desde `0072`. Lo que no reparte
es lo que viene después. El cuello es la forma del conjunto, no una pieza.

**L3 · Las cifras del laboratorio no se trasladan solas.** `0072` midió 3,1 s por millón; dentro del clúster salen
67 s. Hasta que el gap se explique (C4), cualquier estimación del tipo «esto bajará a X» es una apuesta.

**L4 · El copiador residente ahorra el arranque, no el rendimiento.** Su pasada tarda 201 s frente a ~216 s del Job:
ahorra ~15-20 s de montaje y nada de lectura. Es la mejora correcta para **muchas copias pequeñas**, y no la
respuesta a **una tabla grande**. Por eso las dos cosas (P8 y P1) son principios distintos.

**L5 · Una pasada que no cambia nada cuesta 12 s.** Eso fija el suelo de lo incremental hoy, y dice que el 90 % de
ese tiempo es montaje (testigos, clon), no trabajo.

**L6 · Lo operativo también es parte del cuello.** El despliegue pasa por Flux y por una CI cuya puerta es el job más
lento; la etiqueta `main` del registro decide qué corre. Un cambio manual (imagen, límites, reinicios) **se deshace
solo** en la siguiente reconciliación de Flux. Las pruebas que cambien límites o imágenes deben hacerse sabiendo que
son temporales, y el cambio definitivo va a `malla/`.

**L7 · Dos despliegues rotos seguidos tuvieron una sola causa.** El binario del copiador no estaba en la imagen
publicada porque la CI de `main` fallaba, y fallaba porque la prueba de huellas no conocía un fichero que el
Dockerfile ya copiaba (PR #16). *Regla:* una prueba que copia a mano la lista de lo que el Dockerfile copia se desincroniza;
mejor que la derive del propio Dockerfile.

## 7. La forma de la solución (esbozo)

Sólo lo que los principios ya determinan; el detalle se escribe cuando haya las medidas de §8.

- **Interfaz del driver** (P2): `particionar(objeto, n_objetivo) → [trozo{rango, estimación}]` y
  `leer(trozo) → flujo Arrow`. Hoy un driver implementa, en la práctica, `leer(todo)`; el reparto por regiones de
  HBase se mueve **de dentro del driver al núcleo**.
- **Núcleo común** (P1, P3, P5, P9): reparte los trozos entre N trabajadores, cada uno con su lector y su escritor
  (un fichero de tamaño objetivo), reintenta por trozo y hace **un** commit Iceberg que añade todos los ficheros.
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

## 8. Hoja de ruta

Cada fase termina con **su número en este documento** (P10). Las fases posteriores a la B dependen de lo que se mida
en la siguiente iteración, y por eso no llevan todavía criterios de cierre.

| Fase | Qué | Estado |
|---|---|---|
| **A** | Subir el límite de CPU de la pasarela a lo que la medida diga (PR en `malla/`). Dejar la línea base de este ADR en una prueba repetible. | pendiente |
| **B** | Explicar el gap de C4: aislar cada diferencia entre el laboratorio y el clúster. | pendiente |
| **C** | Interfaz `particionar`/`leer` y reparto en el núcleo, con HBase. | por diseñar |
| **D** | Un segundo origen de forma distinta contra la misma interfaz. | por diseñar |
| **E** | Escritura en paralelo y commit único; tamaño de fichero objetivo. | por diseñar |
| **F** | Vía de snapshot para HBase, **si** las medidas muestran que el servidor es el límite. | por decidir |
| **G** | Costes fijos fuera del camino caliente (copiador caliente, testigos y clon reutilizados). | por diseñar |

**Siguiente iteración de este documento:** las medidas que faltan para fijar C, D, E y F, y con ellas el diseño
detallado de la interfaz y del reparto.

## Fuentes

- [HBase · snapshot scanner](https://hbase.apache.org/docs/architecture/snapshot-scanner/)
- [DataStax Spark connector · `CassandraTableScanRDD`](https://docs.datastax.com/en/dse-spark-connector-api/6.8/com/datastax/spark/connector/rdd/CassandraTableScanRDD.html)
- [ConnectorX (Towards Data Science)](https://towardsdatascience.com/connectorx-the-fastest-way-to-load-data-from-databases-a65d4d4062d5)
- [AWS · Iceberg, lectura](https://docs.aws.amazon.com/prescriptive-guidance/latest/apache-iceberg-on-aws/best-practices-read.html)
- [Dremio · escritura inteligente en Iceberg](https://www.dremio.com/?p=25802)
- [Teradata · Arrow Flight SQL](https://www.teradata.com/insights/data-architecture/how-fast-is-arrow-flight-sql-with-teradata)
