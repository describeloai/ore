# 0076 · Query

**Consultar los activos de la celda con SQL, sin crear un repositorio y sin elegir el cómputo.** Query es
la página de la consola donde una persona escribe SQL sobre lo que ve en el Assets Catalog —datasets,
views, bases foráneas, orígenes— y pulsa *Run*. Lo que necesite la consulta lo pone la plataforma: la
sesión **crece mientras corre** (*Elastic Sessions*, §4), y nadie elige un tamaño.

**Estado:** en vivo en victor (2026-10-25, `7ecff30c`) para lo que corre en una sesión: leer, escribir en
el árbol desde SQL y crecer hasta 48 GiB. **Sin construir:** Spark para lo que no cabe en un nodo (§5) y el
resultado paginado y descargable (§10). Es la imagen de lo que Query es hoy. Cómo se llegó, con lo medido
en Q0 y las formas que se probaron y se retiraron, en [`docs/query.md`](../query.md). El mecanismo de
*Elastic Sessions*, entero, en [`0073`](../decisions/0073-elastic-sessions.md).

## 1. Lo que se promete

1. **El cliente no elige motor ni tamaño.** Escribe SQL y pulsa *Run*. No hay selector de cómputo,
   ningún «preparando más cómputo» y ningún reintento que se vea. Es lo contrario de Code
   Repositories, donde el cómputo se elige al crear el repositorio.
2. **Ninguna consulta mata su sesión.** Una consulta termina, o falla con un error que dice qué cambiar.
   No hay «killed», y no hay que relanzar en otro sitio (§4).
3. **Nada se promociona después de fallar.** La consulta no se mueve: la sesión gana memoria y CPU en
   caliente, sin relanzar nada, y lo que no cabe en memoria derrama al disco.
4. **El gobierno es el de siempre.** La sesión es de una persona y corre con su identidad. Lo que lee y
   escribe pasa por `ore-serve` con las credenciales acotadas de cada activo, y lo que el reparto niega se
   dice antes de correr.

## 2. Cómo se usa

- **La página:** `clusters/[celda]/query`. Es el cuerpo de Code Repositories en `modo="query"`: el
  editor y la rama, sin Commit, sin Libraries ni Builds, y con Files reducido a las hojas abiertas.
- **Una hoja es SQL y nada más.** Una celda que no sea `sql` es un 422 que dice dónde va: un repositorio
  o un notebook. Así todo lo que se pulsa tiene plan y se puede pesar de una vez (§4.4).
- **Guardar** una hoja es un commit en `queries/` de la rama que se mira. En una rama protegida se pide
  cambiar a otra.
- **Lo que se pulsa corre entero en la sesión.** Un guion de varias sentencias no se parte, porque lo
  que crea la 1 lo lee la 2. Las variables y las tablas `tmp` de DuckDB sobreviven a que la sesión crezca
  y baje.

## 3. Qué lee y qué escribe

**Query no tiene motor propio.** Una hoja es una sesión del puesto Python, con el mismo SDK que un
repositorio: `ore.sql` es el DuckDB de la sesión, y `ore-serve` resuelve cada nombre en nombre de la
persona.

| | qué | cómo |
|---|---|---|
| **lee** | datasets y views del lago, tablas y vistas de bases foráneas, colecciones de media, orígenes en vivo | el SDK registra cada nombre como vista: Iceberg con la credencial prestada del snapshot, views, colecciones y `/federation/read` para lo vivo (con su tope: 100 000 filas, 64 MB, 30 s por lectura) |
| **escribe** | `create or replace dataset … as select`, `insert into … select`, vistas y vistas materializadas, schemas, bases, `create media collection` y las derivadas | las sentencias del árbol (`sql_que_escribe`, 0039), en la rama de la hoja |
| **explica** | qué va a cada origen y qué queda en DuckDB | `ore explain` y `POST /puestos/{id}/explain` |

## 4. Elastic Sessions

La pieza que hace posible §1. Es el producto de primera clase de Query: **la sesión nace pequeña, crece
en segundos mientras la consulta corre, empieza crecida si ya se sabe lo que va a necesitar, y vuelve a
su base cuando descansa.** El mecanismo es el escalado vertical en caliente de Kubernetes
(`PATCH pods/resize`). El detalle, lo medido y la deuda están en [`0073`](../decisions/0073-elastic-sessions.md).

### 4.1 Tres capas, y ninguna depende de acertar

1. **Derramar.** DuckDB termina casi todo con la memoria que tiene si tiene disco: un SSD local con 64 GiB
   de `/trabajo` por sesión.
2. **Crecer en caliente.** Al 80 % de su memoria, o en cuanto derrama, la sesión pide un escalón y lo
   tiene en 2–4 s, con la consulta en marcha.
3. **Lo que no derrama** (la lista cerrada, §4.5) recibe más margen. Si aun así se desborda, muere su
   proceso, el kernel, y la sesión sigue: la celda contesta `OutOfMemoryError` diciendo cuánto tenía.

### 4.2 El recorrido

| escalón | memoria | CPU | DuckDB (60 % del pod; 45 % con una reservada) |
|---|---|---|---|
| base | 4 GiB | 2 | 2,4 GB |
| | 8 GiB | 4 | 4,8 GB |
| | 16 GiB | 6 | 9,6 GB |
| | 32 GiB | 8 | 19,2 GB |
| techo | 48 GiB | 8 | 28,8 GB |

- **Quién decide:** `ore-serve`. Elige el escalón (de uno en uno, salvo *de un golpe* hasta lo previsto
  para la celda que corre), aplica el techo y comprueba el presupuesto de la celda. Lo aplica
  `ore-redimensionar`.
- **Si no se puede crecer** (presupuesto, nodo lleno o techo), no es un error: la consulta sigue
  derramando y termina.
- **Bajar:** tras 10 min sin celdas, DuckDB suelta su memoria y el pod vuelve a 4 GiB. La sesión cuesta lo
  que pesa, y solo mientras lo pesa.

### 4.3 Dónde vive: el *session pool*

- **Las máquinas:** `n2-highmem-8` spot (8 vCPU, 64 GB) con SSD local, **mínimo 0**. Se enciende cuando
  llega una sesión y se apaga ~10 min después de vaciarse. Coste: ~0,10–0,13 $/h, solo mientras hay
  sesiones.
- **Un OOM mata un proceso, no el contenedor** (`singleProcessOomKill`), y el kernel corre aparte del
  agente. Sin las dos cosas, lo que moría era la sesión.
- **La cuenta:** Kueue admite por la base y no ve lo que crece. Por eso `ore-serve` suma lo crecido
  contra un **presupuesto de 40 GiB por celda** (`cq-<celda>-sesiones`).

### 4.4 Empezar crecida: prever

Lo medido manda sobre lo estimado, y prever decide **coste y latencia, nunca supervivencia**: si se queda
corto, la capa 2 sube; si se pasa, la sesión baja al descansar.

| | una consulta ya vista | una consulta SQL nueva |
|---|---|---|
| **de dónde** | lo que hizo la última vez, de `system.default.executions` (por `fingerprint`, de la organización) | su plan: `EXPLAIN` de DuckDB en la propia sesión, sin leer nada (plazo 2 s), pesado por lo que el motor **construye** en memoria (`ore_core::admision`) |
| **qué da** | su escalón final; +1 si murió por memoria o no pudo crecer; −1 si le sobró | el menor escalón cuyo reparto da lo que construye; techo 32 GiB |
| **en executions** | `growth[0].reason = "forecast"` | `growth[0].reason = "plan"` |

### 4.5 La lista cerrada

Medida con una sesión de 4 GiB, DuckDB con 1 GB y 40 M filas:

| | operaciones | qué pasa |
|---|---|---|
| **seguras** | agrupar, join, ordenar, ventana, `count(distinct)`, `pivot`, CTE recursiva, join espacial, `create … as` | terminan, derramando si hace falta |
| **reservadas** | `list`, `array_agg`, `string_agg`, `json_group_array` por grupo; `median`, `quantile_cont` | fallan con un error, o terminan con el doble de memoria; la sesión sigue |
| **peligrosas** | `mode`, `histogram`, `approx_quantile` por muchos grupos | se pasan hasta que el sistema mata el kernel |

Con una reservada o una peligrosa, DuckDB baja al 45 % del pod y el plan les da margen.

## 5. Lo que no cabe en un nodo: Spark (sin construir)

Por encima de lo que 48 GiB y el derrame hacen con holgura (o un join espacial con los dos lados grandes,
que DuckDB no termina), la consulta irá a **Spark elástico**: el driver de un Build elástico (0068 CE) con
ejecutores que llegan mientras corre. Es Q7 de [`docs/query.md`](../query.md) §5, y lo que ya se decidió es:

- **Un driver por persona activa.** Spark Connect no autentica: en un servidor compartido, la identidad la
  pone el cliente.
- **Solo SQL comprobado.** La traducción de DuckDB a Spark (sqlglot en modo `RAISE` más una lista
  prohibida) tiene que pasar. Si no pasa, la consulta se queda en la sesión.
- **Sin Java desde SQL.** Una regla en `ReglasDeOre` niega `reflect`, `java_method` y `try_reflect`.
- **Nunca para una lectura en vivo.**

Lo de en medio es DuckDB, no Spark. DuckDB es entre 2 y 25 veces más rápido mientras cabe en un nodo
(0068 C0), y Spark vale por lo que no cabe, no por la velocidad.

## 6. Lo que queda de cada ejecución

Cada *Run* deja una fila en `system.default.executions` ([`docs/ejecuciones.md`](../ejecuciones.md)):
quién, desde dónde, cuándo, cómo acabó, la duración, el pico de memoria, el derrame y lo que leyó y
escribió. De *Elastic Sessions* añade `growth` (cada cambio de tamaño y por qué), `memory_final_mb` y
`growth_refused` (`infeasible`, `deferred` o `quota`). Con esa fila se recuerda (§4.4), se dimensiona el
nodo y se cuenta el coste. Es información de la plataforma, no algo que se le pida al cliente.

## 7. Decisiones

| | decisión | por qué |
|---|---|---|
| **DQ1** · la forma | la sesión de la persona, no un servicio de consultas | un solo runtime que ya lee y escribe todo; `ore-consulta`, un segundo ejecutor de solo `SELECT`, se construyó y se retiró sin medir (2026-10-23) |
| **SQL puro** | una hoja solo ejecuta SQL | todo lo pulsado tiene plan; Python vive en Code Repositories y en los notebooks |
| **sin elección** | ni selector de cómputo ni clases que se vean | el escalado es de la plataforma; ver §1 |
| **D-E4** · el techo | 48 GiB por sesión; por encima, Spark | lo que cabe en un nodo `n2-highmem-8` con su sistema |
| **D-E5** · spot | las sesiones en spot | si GCP recupera el nodo, la sesión se reabre y la celda que corría lo dice |
| **D-S4-5** · prever | lo medido (la repetición) manda sobre lo estimado (el plan) | el optimizador se equivoca en los joins; la historia, no |
| **D-S6** · G | Query no usa el puesto pesado en las celdas del *session pool* | lo grande crece en su sesión; G queda para los Builds |
| **DQ4** · Spark | un driver por persona activa | ver §5 |

## 8. Lo medido

- **Por qué hacía falta** (Q0, 2026-10-23, [`docs/query.md`](../query.md) §2). Con la sesión fija de
  4 GiB, las nueve consultas de la escalera tardaban hasta 122 s a 600 M filas, y la ventana se quedaba
  sin memoria. Con 48 GiB terminaban todas en 0,04–50 s, sin derrame.
- **La prueba final** (2026-10-25, una sesión de Query de victor, `7ecff30c`). La consulta es
  `select k % 1000000 g, count(*), sum(v) from (select range k, random() v from range(N)) group by g`:

  | N | duración | crecimiento | acabó en |
  |---|---|---|---|
  | 3 M | 3,1 s | — | 4 GiB |
  | 30 M | 1,5 s | — | 4 GiB |
  | 300 M, nueva | 24,5 s | `plan` 4 → 8 antes de correr; el vigilante 8 → 16 → 32 | 32 GiB |
  | 300 M, otra vez | 23,5 s | nada que pedir | 32 GiB |
  | 10 min sin celdas | — | vuelve a la base | 4 GiB |

- **Abrir una sesión:** 1,2–1,3 s con el nodo encendido. 99 s si el *session pool* está a 0 (el nodo
  tarda 93 s).

## 9. Dónde está cada pieza

| pieza | dónde |
|---|---|
| la página | `rubix-platform`: `app/(workspace)/clusters/[celda]/query`, `CodeWorkspaceClient` con `modo="query"` |
| la hoja de Query y su plan | `ore-serve`: `POST /puestos {query: true}` (`hoja_de_query`), `POST /puestos/{id}/plan`; el agente, `ore._plan` |
| leer y escribir | el SDK, `puesto/python/ore/__init__.py` (`sql()`); `ore-serve`, `sql_que_escribe`; el reparto, `ore_core::reparto` |
| crecer, prever y contar | [`0073`](../decisions/0073-elastic-sessions.md) §6: `elastica.rs`, `prevision.rs`, `puestos.rs`, `ore-k8s` (`ore-redimensionar`), `agente.py` (`KernelAparte`, `Vigilante`) |
| el *session pool* | `malla/75-el-session-pool.sh`, `malla/10-kueue.yaml`, `malla/11-el-inquilino.yaml`; las celdas, `CELDAS_CON_SESSION_POOL` en `malla/gen-inquilino.py` |
| executions | `crates/ore-serve/src/ejecuciones.rs`, `crates/ore-core/src/sistema.rs` |

## 10. Deuda abierta

| | hoy | lo que falta |
|---|---|---|
| **Spark** (Q7) | lo que no cabe en 48 GiB derrama hasta su tope | el driver por persona, la traducción comprobada y la regla contra `reflect` (§5) |
| **el resultado** (Q6) | lo pinta la hoja | progreso, *Cancel*, el resultado en Arrow paginado y descargable desde `ore-serve` |
| **Recent runs** | el panel de la consola es una maqueta | leerlo de `system.default.executions`, con la ficha de cada ejecución y su `growth` |
| **las demás celdas** | solo victor tiene el *session pool*; las otras siguen con el puesto fijo y G (`--query-g`) | el pool para todas |
| **calentar al abrir** (S1b) | con el pool a 0, la primera sesión espera ~99 s | encender el nodo al abrir la página de Query |
| **calibrar el plan** | en la prueba final pidió 8 GiB para una consulta que acabó en 32 | ajustar el factor por familia de operador con más filas de executions |
| **crecer por uso** | crece al 80 % de uso, aunque derramar habría salido más barato | decidir si se crece solo al derramar |
| **verlo y cobrarlo** | `growth` solo en executions | la ficha en la consola; GiB·h por ejecución en Billing |

El resto de la deuda de *Elastic Sessions* (Node, Java, marimo, Builds, la holgura del nodo) es de otros
paradigmas: [`0073`](../decisions/0073-elastic-sessions.md) §8.
