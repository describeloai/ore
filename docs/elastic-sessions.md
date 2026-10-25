# Elastic Sessions · autoescalado vertical en caliente de la sesión

> **Estado (2026-10-24):** S0–S3 hechos. S1 en vivo en t-victor; S2 y S3 en `main`, con la prueba
> final en victor pendiente (§5). S4a (prever, pedir lo previsto y recordarlo tras reiniciar) en `main`, con su prueba en la final (§5); S4b, diseñado en §1.8. Este documento es la imagen de cómo es hoy; el ADR se
> escribe limpio al cerrar la etapa. Lo medido que lo respalda, en §7; los guiones, en
> `pruebas-de-fuego/estructura/`.

*Elastic Sessions* es el producto; el mecanismo, el escalado vertical en caliente (*in-place vertical
scaling*) de la sesión. Sustituye a las clases fijas L / G de Query: una sesión nace pequeña y crece
mientras corre, sin que nadie elija tamaño.

## 0. Lo que se promete

**Ninguna sesión muere por lo que pulsa.** Una consulta termina, o falla con un error que dice qué
cambiar. No hay «killed», no hay que relanzar en otro sitio, y el cliente nunca elige tamaño.

Tres capas, y ninguna depende de acertar una predicción:
1. **Derrame:** DuckDB termina casi todo con la memoria que tenga, si tiene disco (SSD local).
2. **Crecer en caliente:** la sesión gana memoria y CPU mientras corre, en 2–4 s y sin perder nada.
3. **Lo que no derrama** (§2) se detecta y se le da más margen; y si aun así se desborda, muere su
   proceso, no la sesión.

La predicción (S4) sólo decide cuánto crecer **antes** de correr: coste y latencia, no supervivencia.

## 1. Cómo funciona

### 1.1 El *session pool*

- **Pool `sesiones`:** `n2-highmem-8` spot (8 vCPU, 64 GB, 57 GiB asignables) con un **SSD local** de
  375 GB para `/trabajo` y el derrame. **Mínimo 0**: el autoescalador lo enciende cuando Kueue admite una
  sesión y lo quita ~10 min después de vaciarse; se paga por uso. Taint `ore.dev/sesiones`, etiqueta
  `ore.dev/pool=sesiones`. Máximo 1 nodo mientras se desarrolla.
- **Kubelet con `singleProcessOOMKill`:** un OOM mata sólo el proceso que se pasa, no el contenedor
  entero (en cgroup v2, sin esto, cae todo el pod).
- **A qué va ahí:** las **sesiones** de una celda con `--session-pool` (`ore-serve`): Query, Code
  Repositories, Preview. No van los trabajos (Builds) ni el puesto pesado.
- **Cómo nace una sesión:** la base de hoy (`requests 768Mi / 100m`, `limits 4Gi / 2 CPU`),
  `resizePolicy NotRequired` en memoria y CPU, y `/trabajo` de 64 GiB en el SSD (`ORE_DISCO_MB`).
- **Abrir:** en caliente ~1,3 s; con el pool a 0, ~99 s (el nodo). Esconder ese frío es S1b.

### 1.2 El kernel en su proceso

El agente (`agente.py`) **no ejecuta celdas**: lanza el **kernel** como proceso hijo y le pasa cada
celda y cada plan por una tubería, como Jupyter (el kernel no es el servidor). El agente se queda con el
bucle de trabajo, el latido, el editor (LSP), marimo y el cierre por inactividad; el kernel tiene el
espacio de nombres, `ore`, DuckDB y la medida del consumo.

**Si el kernel muere** (un OOM de una operación peligrosa), el agente lo sabe por `oom_kill` en
`memory.events` y contesta la celda: «this cell ran out of memory: the session's N GiB were not enough…
the session is still open, but its variables are gone». Abre otro kernel y la celda siguiente corre en
la misma sesión. Nada se reintenta solo.

### 1.3 Crecer

| | |
|---|---|
| **quién lo ve** | el **vigilante**, un hilo en el kernel (donde vive DuckDB) que mira cada 200 ms mientras corre una celda: la memoria de DuckDB contra su `memory_limit`, y si derrama |
| **cuándo pide** | al 80 % del `memory_limit`, o en cuanto derrama |
| **a quién** | `POST /puestos/{id}/scale {memoria_mb, motivo}`, con la credencial del pod de esa sesión en esa apertura (R1) |
| **qué decide `ore-serve`** | el escalón (`elastica.rs`): **4 → 8 → 16 → 32 → 48 GiB**, con CPU 2 → 4 → 6 → 8 → 8; de uno en uno salvo `de_un_golpe`; techo 48 GiB (por encima, E); y que quepa en el presupuesto de la celda (§1.5) |
| **quién lo aplica** | `ore-redimensionar` (crate `ore-k8s`): `PATCH pods/resize` (memoria y CPU, *strategic merge*) y espera a que el pod diga el tamaño nuevo. `ore-serve` no habla TLS; lo lanza. Responde `hecho`, `infeasible`, `deferred` o `en_curso` |
| **qué hace el kernel al crecer** | sube `memory_limit` y los hilos de DuckDB **desde otra conexión, con la consulta en marcha** |
| **el reparto** | DuckDB, el **60 %** de lo que mide el pod ahora (su cgroup); el **45 %** si la sentencia lleva una reservada o peligrosa (§2). Se recalcula antes de cada `sql()` |
| **si no se puede** | `quota`, `infeasible`, `deferred` o el techo no son un error: la consulta sigue derramando y termina; en esa celda no se vuelve a pedir. Fuera del session pool, nunca |

### 1.4 Bajar

Tras **10 min sin celdas** (`ORE_BAJAR_TRAS`), si la sesión creció: el kernel baja `memory_limit` a 256 MB
(DuckDB suelta en < 5 s) y el agente pide la base. Kubernetes no baja un pod por debajo de lo que usa,
por eso va en ese orden. La sesión cuesta lo que pesa y sólo mientras lo pesa.

### 1.5 La cuenta de lo crecido

- **Kueue** admite al abrir por la **base** de cada sesión y **no ve lo que crece** (medido).
- **`ore-serve`**, que es quien redimensiona, lleva la cuenta: suma lo que el nodo les guarda a las
  sesiones vivas del pool (768Mi la base, el tope en cada escalón por encima) contra el presupuesto de la
  celda, **40 GiB** (la cuota de `cq-<celda>-sesiones`; una prueba ata las dos cifras).
  - **al crecer**, si no cabe: `quota` (con lo ocupado y el presupuesto). Bajar siempre cabe;
  - **al abrir**, si la base no cabe: `409` con lo ocupado, en vez de una sesión admitida sin nodo.
- **La `ResourceQuota`** del namespace (112 GiB) sigue siendo el techo del inquilino: ve lo crecido.

### 1.6 Lo que queda en executions

Cada fila de `system.default.executions` dice, de la ejecución:
- `growth`: JSON `[{at_ms, from_mb, to_mb, cpu_m, reason}]`, lo que creció mientras corría;
- `memory_final_mb`: el tope con que acabó (sólo en el session pool);
- `growth_refused`: `infeasible`, `deferred` o `quota`, si quiso crecer y no pudo.

Con eso aprende la previsión de S4, se ajusta la holgura del nodo y se cuenta el coste (Billing).

### 1.7 Crecer antes: *pre-scale* (S4a)

Crecer mientras corre (§1.3) sigue siempre. S4a sólo hace que una consulta **ya vista** empiece en el
escalón en que acabó la otra vez, de un golpe, en vez de subir de uno en uno, derramando entre medias o
muriendo por el camino (las peligrosas de §2). Es lo que se previene de verdad: un `mode` que la otra vez
murió en 16 GiB empieza ahora en 32 y termina. Si la previsión se queda corta, el vigilante sigue
subiendo; si se pasa, la sesión baja a los 10 min (§1.4).

| | |
|---|---|
| **la llave** | el `fingerprint` de executions (la sentencia normalizada; la de un guion, entera). Por **celda** (la organización), no por persona: la misma consulta sobre los mismos datos pide lo mismo |
| **quién recuerda** | `ore-serve` (`prevision.rs`), en memoria, `fingerprint → escalón`, lo último que vio de cada una (tope 10 000, se olvida la más vieja). Aprende de cada fila de executions al apuntarla, así que hace falta `--executions` |
| **qué aprende de una fila** | sólo del session pool (`memory_final_mb`). Terminó `ok`: su escalón final. Murió de `OutOfMemoryError` o se le negó crecer (`growth_refused`): **uno más** (quería más). Si empezó por la previsión, terminó sin crecer y su pico (`peak_memory_mb`) cabe en la mitad del tope del escalón de abajo: **uno menos**. Un error de otro tipo no enseña nada |
| **cuándo prevé** | al darle la celda al pod (`GET …/siguiente`): si lo recordado está por encima del escalón que tiene, el sobre de la celda lleva `previsto_mb`. Al darla, no al recibirla: así cuenta lo aprendido de la celda de justo antes |
| **quién pide** | el vigilante, al empezar la celda y **antes** de correrla: `POST /scale {memoria_mb: previsto_mb, de_un_golpe, motivo: "forecast"}`, espera a que el cgroup lo mida, y la celda corre ya con el reparto nuevo (se calcula antes de cada `sql()`). Después vigila como siempre |
| **qué decide `ore-serve`** | lo de siempre (el techo, el presupuesto: si no cabe, `quota` y empieza en la base) **y** que *de un golpe* sólo llegue hasta lo previsto para la celda que corre: el pod no puede saltar a 48 porque sí |
| **qué queda en executions** | `growth[0].reason = "forecast"`, de 4096 al escalón previsto, al principio de la fila |
| **al reiniciar `ore-serve`** | con `--session-pool`, lo recordado se calienta de la tabla de la organización en executions: `ore-store leer` sólo con las columnas que hacen falta (`columnas`), las de los últimos 7 días, en el orden en que terminaron (manda la última) |

Java, Node y marimo no piden nada (§9).

### 1.8 Prever por el plan (S4b, en diseño)

S4a sólo sirve a partir de la segunda vez. S4b prevé la **primera**: una consulta SQL nueva se pesa por su
plan antes de correr y, si va a necesitar claramente más que la base, empieza crecida. Desde la segunda
vez manda S4a: **lo medido manda sobre lo estimado** (D-S4-5).

Casi todo existe ya, de Query A1–A3 (la admisión que elegía L/G):

| | hoy | S4b |
|---|---|---|
| **el plan** | `POST /puestos/{id}/plan`: el agente corre `EXPLAIN (FORMAT JSON)` en la sesión, con lo que la sentencia nombra registrado y sin leer nada; plazo 2 s | igual |
| **el peso** | `admision::peso`: filas y bytes, el mayor entre **lo que pasa** por un operador y **lo que construye** en memoria (el lado derecho de un `HASH_JOIN`, `HASH_GROUP_BY`, `ORDER_BY`, `WINDOW`…) | separar lo que construye (`memoria`): lo que sólo pasa (un `scan`, un `filter`) no ocupa memoria |
| **la decisión** | `admitir` → L, G o E | `escalon_por_el_plan` → el menor escalón cuyo reparto (60 %, o 45 % con una reservada) da lo que construye por el factor medido en S4b·0; peligrosas de §2, sin derrame posible, con más margen |
| **cuándo** | sólo con `--query-g`, para elegir dónde | al recibir una celda `sql` de una sesión del pool **sin recuerdo de S4a**; lo previsto va en la celda y `al_dar` lo usa si S4a no dice nada |

Lo que hay que tener en cuenta:
- **El plan cuesta**: un viaje al agente antes de la celda (en ms; el plazo, 2 s). Sin plan a tiempo,
  sin previsión: la celda corre y el vigilante sube como siempre.
- **DuckDB estima mal los grupos**: un `group by` suele estimarse con tantos grupos como filas. Por eso
  S4b·0 mide antes cuánto se equivoca, y la previsión por el plan **no pasa de 32 GiB** (D-S4-6): lo que
  sobre, S4a lo baja en la siguiente.
- **Se distingue en executions**: `reason = "plan"`, no `forecast` (D-S4-7). Para aprender, las dos son
  previsión.

Los pasos:

| paso | listo cuando |
|---|---|
| **S4b·0 · medir el plan contra lo real** (previo, en victor) | para la escalera, joins, agrupaciones y un `mode`: lo que el plan dice que construye frente a `peak_memory_mb` y el escalón final de executions; de ahí el factor, y si la estimación de grupos sirve |
| **S4b·1 · el peso de la memoria** (`ore-core`) ✅ en local | `Entrada.memoria` y `admision::riesgo` (`ore-core`), `escalon_por_el_plan` (`elastica.rs`), puros, con planes reales de DuckDB 1.5.6 en `tests/planes/` (`mode_300m`, `agrupa_300m`). La memoria de un operador: el lado de construcción de un `HASH_JOIN`; en una agrupación, un estado por grupo, o lo que le llega entero si un agregado lo guarda (`mode`, `median`, `list`…); `ORDER_BY` y `WINDOW`, lo que les llega. **Sin calibrar**: `FACTOR_DEL_PLAN = 1`, `MARGEN_PELIGROSA = 2`, techo 32 GiB. Medido en el plan: para 1 M grupos de 300 M filas DuckDB estima 150 M, así que una agrupación `count`/`sum` de 300 M pide 8 GiB que no necesita |
| **S4b·2 · `ore-serve` prevé por el plan** | una celda `sql` nueva del pool pide el plan, y la celda lleva lo previsto; `reason = "plan"` |
| **S4b·3 · medir** | un `mode` grande nunca visto empieza crecido y termina; uno pequeño no pide nada |

## 2. La lista cerrada

Medido con la forma de una sesión (4 GiB, DuckDB con 1 GB, 40 M filas):

| operación | qué pasa | clase |
|---|---|---|
| agrupar, join, ordenar, ventana (también con marco), `count(distinct)`, `pivot`, CTE recursiva, join espacial, `arg_max`, `ST_Collect`, `ST_Union_Agg`, `create … as` | terminan, derramando si hace falta | **seguras** |
| `list`, `array_agg`, `string_agg`, `json_group_array` por grupo | `OutOfMemoryException`: falla la consulta, la sesión sigue | **reservadas** |
| `median`, `quantile_cont` | terminan, con el **doble** del `memory_limit` | **reservadas** |
| **`mode`, `histogram`, `approx_quantile`** por muchos grupos | se pasan hasta que el sistema mata su proceso | **peligrosas** |

Con una reservada o peligrosa en el texto, DuckDB va al 45 %. Una peligrosa que aun así se desborda mata
el kernel, no la sesión (§1.2). Darles memoria **antes** de correr, por el plan, es S4.

## 3. Dónde está cada pieza

| pieza | dónde |
|---|---|
| el pool | `malla/75-el-session-pool.sh` (en seco por defecto; `APLICAR=1`) |
| el sabor y las colas | `malla/10-kueue.yaml` (`sesiones`), `malla/11-el-inquilino.yaml` (`cq-<celda>-sesiones`, `cola-sesiones`) |
| qué celdas lo usan | `malla/gen-inquilino.py`: `CELDAS_CON_SESSION_POOL` (hoy `{victor}`) → `--session-pool` en su `ore-serve` |
| el permiso de redimensionar | `malla/40-ore-serve.yaml`: `Role ore-serve-redimensiona` (`get`/`list` pods, `patch pods/resize`, en su namespace) y la salida a la API |
| una sesión al pool | `crates/ore-serve/src/cola.rs`, `al_session_pool` |
| `scale`, la política y la cuenta | `crates/ore-serve/src/puestos.rs` (`escalar_puesto`, la apertura), `elastica.rs` (`decidir_hasta`) |
| la previsión (S4a) | `crates/ore-serve/src/prevision.rs` (`ensena`, `al_dar`, `calentar`); se aprende al apuntar la fila, se prevé al dar la celda (`puestos.rs`) y se calienta al arrancar (`ejecuciones.rs`, `calentar_la_prevision`) |
| redimensionar el pod | `crates/ore-k8s` (`ore-redimensionar`) |
| executions | `crates/ore-serve/src/ejecuciones.rs`, `crates/ore-core/src/sistema.rs` |
| el kernel aparte, el vigilante, lo previsto, bajar | `puesto/python/agente.py` (`KernelAparte`, `Vigilante` con `previsto_mb`, `bajar_si_toca`) |
| el reparto | `puesto/python/ore/__init__.py` (`_pod_mb`, `_tropo_mb`, `_ajustar`, `_RESERVADAS`) |
| pruebas | `crates/ore-k8s`, `elastica.rs`, `prevision.rs`, `cola.rs`; `puesto/python/pruebas/test_kernel_aparte.py`, `test_vigilante.py` |

## 4. Los pasos

| paso | estado | listo cuando |
|---|---|---|
| **S0 · previos** | ✅ medido | Kueue no ve lo crecido; bajar funciona; la lista cerrada; el precio |
| **S1 · el session pool** | ✅ en vivo en t-victor | las peligrosas mueren solas (`s1-mode`); una sesión de Query abre en el pool |
| **S2a · `ore-serve` redimensiona** | ✅ en `main` · ⏳ prueba final | `scale` a 8 GiB sube `memory.max` sin reiniciar en ≤ 5 s |
| **S2b · la cuenta y executions** | ✅ en `main` · ⏳ prueba final | por encima del presupuesto, `quota`; la fila dice lo crecido |
| **S3a · el kernel en su proceso** | ✅ en `main` · ⏳ prueba final | un `mode()` grande da error y `select 42` responde en la misma sesión |
| **S3b · crecer solo** | ✅ en `main` · ⏳ prueba final | la escalera 3M → 30M → 300M en una sesión que crece sola; a los 10 min, a la base |
| **S1b · calentar al abrir Query** | pendiente | abrir la página de Query enciende el nodo; la primera sesión no espera los ~99 s |
| **S4a·1 · `ore-serve` recuerda y prevé** | ✅ en `main` · ⏳ prueba final | `prevision.rs` con sus reglas en pruebas; el sobre de la celda lleva `previsto_mb`; *de un golpe* sólo hasta lo previsto |
| **S4a·2 · el vigilante pide lo previsto** | ✅ en `main` · ⏳ prueba final | con `previsto_mb`, la celda empieza crecida; `growth[0].reason = "forecast"` |
| **S4a·3 · recordar tras reiniciar** | ✅ en `main` · ⏳ prueba final | `ore-serve` arranca con lo de executions de los últimos 7 días |
| **S4b · prever por el plan** | S4b·1 ✅ en local; siguiente S4b·2; S4b·0 (calibrar) después, con S4a servida (§1.8) | una consulta nueva con una peligrosa sobre muchas filas empieza crecida |
| **S6 · retirar G** | pendiente | Query deja de abrir el puesto pesado; `pesado` queda para los Builds |

## 5. La prueba final (S2–S3, en victor)

Con `ore-serve` y `puesto-python` de `main` servidos, y t-victor convergido:
1. la escalera 3M → 30M → 300M en **una** sesión de Query: crece sola; `growth` en cada fila;
2. `select k % 1000000 g, mode(v) …` grande: `OutOfMemoryError` legible; `select 42` responde después;
3. una petición por encima del presupuesto: `quota`, y `growth_refused` en la fila;
4. 10 min sin celdas: la sesión vuelve a 4 GiB.

Con S4a, además:

5. la de 300M otra vez: un solo crecimiento, `forecast`, al principio; derrama ~0 y tarda menos que la primera;
6. el `mode` grande otra vez: empieza un escalón por encima de donde murió y termina (o vuelve a morir, y la siguiente sube uno más, hasta el techo).

## 6. Decisiones

| | decisión | estado |
|---|---|---|
| **D-E1** · la máquina | `n2-highmem-8` spot con SSD, **mínimo 0**, por uso; mínimo 1 en el horario sólo en la rampa a producción | decidida |
| **D-E2** · la holgura del nodo | que crecer casi nunca sea `infeasible`: `requests` base mayores o un pod de relleno de baja prioridad | abierta: se mide cuántas crecen a la vez |
| **D-E3** · la cuenta de lo crecido | la lleva `ore-serve` (contar el techo en la cola dejaría una sesión por nodo) | decidida, hecha |
| **D-E4** · el techo de una sesión | 48 GiB; por encima, E | decidida |
| **D-E5** · spot | sesiones en spot; si GCP recupera el nodo, la sesión se reabre y la celda que corría lo dice | decidida; revisar si pasa a menudo |
| **D-E6** · que un OOM no tumbe la sesión | `singleProcessOOMKill` en el pool **y** el kernel en su proceso (sin él, el proceso que muere es el agente) | decidida, hecha |
| **D-S4-1** · de quién es lo recordado | de la celda (la organización), por `fingerprint`; no de la persona. Un filtro de filas por persona lo corrige el vigilante | decidida, hecha (S4a·1) |
| **D-S4-2** · qué manda | **la última** ejecución de cada `fingerprint`, con sus reglas (§1.7): sube con un OOM o un `quota`, baja sólo con un pico pequeño. Sin medias ni máximos: los datos cambian | decidida, hecha (S4a·1) |
| **D-S4-3** · *de un golpe* | sólo hasta lo previsto para la celda que corre; sin previsión, de uno en uno. Cierra el «el pod puede pedir 48» de §8 | decidida, hecha (S4a·1) |
| **D-S4-5** · repetición o plan | lo medido (S4a) manda sobre lo estimado (S4b): el plan sólo para lo que no se ha visto | propuesta |
| **D-S4-6** · el techo del plan | la previsión por el plan no pasa de 32 GiB; lo que falte lo sube el vigilante | propuesta, a revisar con S4b·0 |
| **D-S4-7** · cómo se dice | `reason = "plan"` frente a `forecast`; para aprender (`ensena`), las dos son previsión | propuesta |
| **D-S4-4** · dónde se recuerda | en memoria de `ore-serve`, calentada de executions al arrancar; no una tabla nueva | decidida, hecha (S4a·1, S4a·3) |

## 7. Lo medido

**Crecer y bajar (R0, S0):**

| | |
|---|---|
| crecer de 4 a 16 GiB en caliente (`pods/resize`) | 2–4 s, sin reiniciar |
| DuckDB con `memory_limit` subido desde otra conexión, en marcha | 45,5 s → 27,0 s (grande desde el principio: 26,6 s) |
| un nodo sin sitio | `Infeasible`; el pod sigue igual |
| bajar en caliente | DuckDB de 739 a 57 MB en < 5 s; el pod de 3 a 1 GiB en 2 s |
| Kueue 0.19.3 con un pod crecido a 3 GiB | sigue contando 512 MiB y admite otro que queda `Pending` |
| la `ResourceQuota` | ve y rechaza lo crecido que la pasa |

**El session pool (S1):**

| | pool de hoy | session pool |
|---|---|---|
| `agrupa_alta` | ok | ok, 6,4 s, derrama 755 MB en el SSD |
| `mode` / `histogram` / `approx_quantile` | **el contenedor entero muere** | muere sólo su proceso (−9); el padre sigue |
| abrir una sesión, en frío (pool a 0) | — | 99 s (nodo listo a los 93 s) |
| abrir una sesión, en caliente | ~4 s | 1,2–1,3 s |

**Disco y coste:** el nodo de sesiones de antes tenía 7 GB libres; un disco propio por pod tarda
12–17 s en conectarse, el SSD local, nada. Un `n2-highmem-8` spot con SSD cuesta ~0,10–0,13 $/h (≈ 60–85
$/mes si estuviera encendido 16 h al día; con mínimo 0, sólo lo usado).

## 8. Operar

- **El pool:** `bash malla/75-el-session-pool.sh` (en seco); `APLICAR=1` lo crea; `RECREAR=1` si le
  falta el SSD o el kubelet. ⛔ En el fichero de gcloud la clave es `singleProcessOomKill` (la de la
  documentación de GKE, `singleProcessOOMKill`, la rechaza).
- **Una celda más en el pool:** añadirla a `CELDAS_CON_SESSION_POOL` y converger. ⛔ Nunca con
  `kubectl patch` sobre `ore-serve`: Flux es su dueño y lo deshace.
- **Converger ya**, sin esperar a la hora en punto:
  `kubectl create job -n ore-system converger-ahora-<algo> --from=cronjob/aprovisionador` (~8 min).
- ⚠️ Kubernetes marca el contenedor `OOMKilled` aunque salga con 0: dice que hubo un OOM dentro, no que
  la sesión muriera.
- ⚠️ El código de la sesión comparte contenedor con el agente y puede leer la credencial del pod: lo que
  protege es la política (un escalón cada vez, el techo, el presupuesto), no quién pide.

## 9. Deuda abierta

Elastic Sessions es hoy **Python, en sesión** (Query, Code Repositories, Preview). Lo demás corre en el
pool o cerca, pero sin todo el mecanismo. Se normaliza cuando toque, cada uno con su paso:

| paradigma | hoy | lo que falta |
|---|---|---|
| **Node** (`puesto/node/agente.mjs`) | va al pool y se puede redimensionar, pero la celda corre en el proceso del agente (el REPL de Node) y nadie pide crecer: se queda en 4 GiB, y un OOM tumba la sesión | el kernel en un proceso hijo, su vigilante y el reparto del cgroup |
| **Java** (`puesto/jvm/ore/Agente.java`) | lo mismo: JShell en modo `local`, en la JVM del agente | JShell en modo remoto (su JVM aparte), su vigilante y el reparto del cgroup |
| **Analytics y Models** (marimo, `python-notebooks`) | el pod va al pool, pero las celdas las ejecuta marimo, un proceso aparte que no pasa por el kernel: sin vigilante (el reparto sí se ajusta en cada `ore.sql()`), y si marimo muere, nadie lo relanza | relanzar marimo y decirlo en el cuaderno; su vigilante |
| **Builds** (un trabajo: `trabajo-…`) | tienen el kernel aparte (un OOM falla el build con un error claro), pero no van al session pool: tamaño fijo en los nodos de siempre; lo grande va a Spark (`pesado`) | decidir si un Build ligero crece como una sesión o sigue fijo y escala a Spark |
| **Calentar al abrir** (S1b) | la primera sesión con el pool a 0 espera ~99 s | un pod de relleno al abrir la página de Query |
| **La holgura del nodo** (D-E2) | sin medir | cuántas sesiones crecen a la vez en victor, y el `requests` base o el relleno |
