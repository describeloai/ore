# 0073 · Elastic Sessions — la sesión de Query crece mientras corre

**Estado:** aceptado (2026-10-25), **en vivo en victor** (`7ecff30c`). Cierra la etapa para el
**paradigma de Query** (sesiones Python: Query, Code Repositories, Preview); lo demás, en §8.
Sustituye a las clases fijas **L / G** de Query (`docs/query.md` §3.7): el cliente nunca elige tamaño y
nada se promociona tras fallar. Se apoya en
`system.default.executions` (`docs/ejecuciones.md`) para recordar y para contar.

*Elastic Sessions* es el producto; el mecanismo, el **escalado vertical en caliente** (*in-place
vertical scaling*, `PATCH pods/resize`) de la sesión: nace pequeña, crece en segundos mientras corre,
empieza crecida si ya se sabe lo que va a necesitar, y vuelve a su base cuando descansa.

## 1. Lo que se promete

**Ninguna sesión muere por lo que pulsa.** Una consulta termina, o falla con un error que dice qué
cambiar. No hay «killed», no hay que relanzar en otro sitio, y nadie elige tamaño.

Tres capas, y ninguna depende de acertar una predicción:

1. **Derramar:** DuckDB termina casi todo con la memoria que tenga, si tiene disco (SSD local).
2. **Crecer en caliente:** la sesión gana memoria y CPU mientras corre, en 2–4 s y sin perder nada.
3. **Lo que no derrama** (§3) se reconoce y se le da más margen; si aun así se desborda, muere su
   proceso (el kernel), no la sesión.

**Prever** (S4) sólo decide en qué escalón **empieza**: coste y latencia, nunca supervivencia. Si
prevé corto, la capa 2 sube; si prevé largo, la sesión baja al descansar.

## 2. Cómo funciona

```
 celda ──► ore-serve ─┬─ ¿ya vista? (S4a: executions) ──► previsto ─┐
                      └─ ¿nueva y SQL? (S4b: su plan) ──► previsto ─┤
                                                                     ▼
            el pod de la sesión ◄── sobre {texto, previsto_mb, previsto_motivo}
              │  vigilante: antes de correr, POST /scale de un golpe hasta lo previsto
              │  mientras corre: al 80 % o al derramar, POST /scale (un escalón)
              ▼
 ore-serve decide (escalón, techo, presupuesto) ──► ore-redimensionar: PATCH pods/resize
              │
 al acabar: la fila de executions (growth, memory_final_mb, growth_refused) ──► recordar
 10 min sin celdas: DuckDB suelta y el pod vuelve a 4 GiB
```

### 2.1 El *session pool*

- **Pool `sesiones`:** `n2-highmem-8` spot (8 vCPU, 64 GB, 57 GiB asignables) con un **SSD local** de
  375 GB para `/trabajo` y el derrame. **Mínimo 0**: se enciende cuando Kueue admite una sesión y se
  apaga ~10 min después de vaciarse; se paga por uso. Taint `ore.dev/sesiones`. Máximo 1 nodo hoy.
- **Kubelet con `singleProcessOOMKill`:** un OOM mata sólo el proceso que se pasa, no el contenedor
  (en cgroup v2, sin esto, cae el pod entero).
- **Qué va ahí:** las sesiones de una celda con `--session-pool`. No van los Builds ni el puesto pesado.
- **Cómo nace una sesión:** la base de siempre (`requests 768Mi / 100m`, `limits 4Gi / 2 CPU`),
  `resizePolicy NotRequired` en memoria y CPU, y `/trabajo` de 64 GiB en el SSD.

### 2.2 El kernel en su proceso

El agente (`agente.py`) no ejecuta celdas: lanza el **kernel** como proceso hijo (como Jupyter) y le
pasa cada celda y cada plan por una tubería. El agente se queda con el bucle, el latido, el editor,
marimo y el cierre; el kernel, con el espacio de nombres, `ore`, DuckDB y el vigilante.

**Si el kernel muere** por memoria, el agente lo sabe por `oom_kill` (`memory.events`), contesta la
celda con `OutOfMemoryError` («the session's N GiB were not enough… the session is still open, but its
variables are gone») y abre otro kernel. Nada se reintenta solo.

### 2.3 Crecer mientras corre

| | |
|---|---|
| **quién lo ve** | el **vigilante**, un hilo en el kernel que mira cada 200 ms la memoria de DuckDB contra su `memory_limit`, y si derrama |
| **cuándo pide** | al 80 % del `memory_limit`, o en cuanto derrama |
| **a quién** | `POST /puestos/{id}/scale {memoria_mb, motivo}`, con la credencial del pod de esa sesión en esa apertura |
| **qué decide `ore-serve`** | el escalón: **4 → 8 → 16 → 32 → 48 GiB**, CPU 2 → 4 → 6 → 8 → 8; de uno en uno, salvo *de un golpe* hasta lo previsto (§2.6); techo 48 GiB; y que quepa en el presupuesto de la celda (§2.5) |
| **quién lo aplica** | `ore-redimensionar` (`crates/ore-k8s`): `PATCH pods/resize` y espera a que el pod diga el tamaño nuevo (`hecho`, `infeasible`, `deferred`). `ore-serve` no habla TLS: lo lanza |
| **qué hace el kernel** | sube `memory_limit` y los hilos de DuckDB **desde otra conexión, con la consulta en marcha** |
| **el reparto** | DuckDB, el **60 %** de lo que mide el pod (su cgroup); el **45 %** con una reservada o peligrosa (§3); se recalcula antes de cada `sql()` |
| **si no se puede** | `quota`, `infeasible`, `deferred` o el techo no son un error: la consulta sigue derramando y termina |

### 2.4 Bajar

Tras **10 min sin celdas** (`ORE_BAJAR_TRAS`), si creció: el kernel baja `memory_limit` a 256 MB
(DuckDB suelta en < 5 s) y el agente pide la base. Kubernetes no baja un pod por debajo de lo que usa;
por eso, en ese orden. La sesión cuesta lo que pesa y sólo mientras lo pesa.

### 2.5 La cuenta de lo crecido

**Kueue admite por la base y no ve lo que crece** (medido). La cuenta la lleva `ore-serve`, que es quien
redimensiona: suma lo que el nodo guarda a las sesiones vivas del pool (768Mi la base; el tope de cada
escalón por encima) contra el **presupuesto de la celda, 40 GiB** (la cuota de `cq-<celda>-sesiones`; una
prueba ata las dos cifras). Al crecer, si no cabe: `quota`. Al abrir, si la base no cabe: `409`. La
`ResourceQuota` del namespace (112 GiB) sigue siendo el techo del inquilino.

### 2.6 Empezar crecida: prever

**Lo medido manda sobre lo estimado.**

| | S4a · una consulta ya vista | S4b · una consulta SQL nueva |
|---|---|---|
| **de dónde** | `ore-serve` recuerda `fingerprint → escalón` (`prevision.rs`, en memoria, 10 000; al arrancar, de la tabla de la organización en executions, 7 días) | el plan de la sesión (`POST /puestos/{id}/plan`, `EXPLAIN` sin leer nada, plazo 2 s), pesado por lo que el motor **construye** en memoria (`admision::peso`, `Entrada.memoria`) |
| **qué recuerda / estima** | la última ejecución de cada consulta: su escalón final; **+1** si murió por memoria o le negaron crecer; **−1** si empezó por la previsión, no creció y su pico cabía en la mitad del escalón de abajo | el menor escalón cuyo reparto da lo que construye (un join, su lado de construcción; una agrupación, un estado por grupo, o la entrada entera si un agregado la guarda; `ORDER_BY`/`WINDOW`, la entrada); peligrosas ×2; **techo 32 GiB** |
| **cuándo** | al dar la celda al pod | al recibir la celda, si la sesión está libre (si no, el plan esperaría su turno) |
| **en executions** | `growth[0].reason = "forecast"` | `growth[0].reason = "plan"` |

El sobre de la celda lleva `previsto_mb` y `previsto_motivo`; el vigilante lo pide **antes** de correr,
*de un golpe*, y `ore-serve` sólo deja llegar *de un golpe* hasta lo previsto para la celda que corre.

### 2.7 Sin G

En las celdas del session pool, Query **no manda nada a G** (el puesto pesado): `gen-inquilino.py` quita
`--query-g` donde pone `--session-pool`. Lo grande crece en su sesión. G queda para los Builds, y para
las celdas que aún no tienen el pool.

### 2.8 Lo que queda en executions

Cada fila de `system.default.executions` dice, además: `growth` (`[{at_ms, from_mb, to_mb, cpu_m,
reason}]`, cada cambio de tamaño y por qué), `memory_final_mb` (el tope con que acabó) y `growth_refused`
(`infeasible`, `deferred` o `quota`). Con eso se recuerda (§2.6), se dimensiona el nodo y se cuenta el
coste.

## 3. La lista cerrada

Medido con la forma de una sesión (4 GiB, DuckDB con 1 GB, 40 M filas):

| operación | qué pasa | clase |
|---|---|---|
| agrupar, join, ordenar, ventana, `count(distinct)`, `pivot`, CTE recursiva, join espacial, `arg_max`, `ST_Collect`, `ST_Union_Agg`, `create … as` | terminan, derramando si hace falta | **seguras** |
| `list`, `array_agg`, `string_agg`, `json_group_array` por grupo | `OutOfMemoryException`: falla la consulta, la sesión sigue | **reservadas** |
| `median`, `quantile_cont` | terminan, con el **doble** del `memory_limit` | **reservadas** |
| **`mode`, `histogram`, `approx_quantile`** por muchos grupos | se pasan hasta que el sistema mata su proceso | **peligrosas** |

Con una reservada o peligrosa, DuckDB va al 45 % y el plan les da margen (§2.6). La lista vive dos veces,
en el SDK (`_RESERVADAS`) y en `ore-core` (`admision::riesgo`).

## 4. Decisiones

| | decisión |
|---|---|
| **D-E1** · la máquina | `n2-highmem-8` spot con SSD local, **mínimo 0**, por uso; mínimo 1 en horario sólo en la rampa a producción |
| **D-E3** · la cuenta de lo crecido | la lleva `ore-serve`; contar el techo en la cola dejaría una sesión por nodo |
| **D-E4** · el techo de una sesión | 48 GiB; por encima, E |
| **D-E5** · spot | sesiones en spot; si GCP recupera el nodo, la sesión se reabre y la celda que corría lo dice |
| **D-E6** · que un OOM no tumbe la sesión | `singleProcessOOMKill` en el pool **y** el kernel en su proceso; sin lo segundo, el que muere es el agente |
| **D-S4-1** · de quién es lo recordado | de la celda (la organización), por `fingerprint`, no de la persona; un filtro de filas por persona lo corrige el vigilante |
| **D-S4-2** · qué manda | la **última** ejecución de cada consulta, con sus reglas; sin medias ni máximos, porque los datos cambian |
| **D-S4-3** · *de un golpe* | sólo hasta lo previsto para la celda que corre; sin previsión, de uno en uno. El pod no puede pedir 48 porque sí |
| **D-S4-4** · dónde se recuerda | en memoria de `ore-serve`, calentada de executions al arrancar; no una tabla nueva |
| **D-S4-5** · repetición o plan | lo medido (S4a) manda sobre lo estimado (S4b) |
| **D-S4-6** · el techo del plan | 32 GiB; lo que falte, el vigilante |
| **D-S4-7** · cómo se dice | `reason = "plan"` frente a `forecast`; para aprender, las dos son previsión |
| **D-S6** · G | fuera de Query en las celdas del pool; G sigue para Builds |

## 5. Lo medido

**Los previos (R0, S0, 2026-10-24):**

| | |
|---|---|
| crecer de 4 a 16 GiB en caliente (`pods/resize`) | 2–4 s, sin reiniciar |
| DuckDB con `memory_limit` subido desde otra conexión, en marcha | 45,5 s → 27,0 s (grande desde el principio: 26,6 s) |
| un nodo sin sitio | `Infeasible`; el pod sigue igual |
| bajar en caliente | DuckDB de 739 a 57 MB en < 5 s; el pod de 3 a 1 GiB en 2 s |
| Kueue 0.19.3 con un pod crecido | sigue contando la base y admite otro que queda `Pending` |
| la `ResourceQuota` | ve y rechaza lo crecido que la pasa |

**El session pool (S1, 2026-10-24):**

| | pool de antes | session pool |
|---|---|---|
| `agrupa_alta` | ok | ok, 6,4 s, derrama 755 MB en el SSD |
| `mode` / `histogram` / `approx_quantile` | **el contenedor entero muere** | muere sólo su proceso (−9); el padre sigue |
| abrir una sesión, en frío (pool a 0) | — | 99 s (el nodo, 93 s) |
| abrir una sesión, en caliente | ~4 s | 1,2–1,3 s |

**La prueba final, en una sesión de Query de victor (2026-10-25, `7ecff30c`):** `select k % 1000000 g,
count(*), sum(v) from (select range k, random() v from range(N)) group by g`, de las filas de executions:

| N | duración | pico | crecimiento | acabó en |
|---|---|---|---|---|
| 3 M | 3,1 s | 248 MB | — | 4 GiB |
| 30 M | 1,5 s | 265 MB | — | 4 GiB |
| 300 M, nueva | 24,5 s | 14,3 GB · derrama 41 MB | **`plan`** 4 → 8 antes de correr; el vigilante 8 → 16 → 32 al 80 % | 32 GiB |
| 300 M, otra vez (la sesión aún en 32) | 23,5 s | 14,4 GB · derrama 0 | — (ya tenía lo previsto) | 32 GiB |
| 10 min sin celdas | — | el motor suelta todo (0 MB) | bajar a la base (`200 hecho`), 10:38:15, ~10 min 30 s tras la última | 4 GiB |

Lo que dice: las pequeñas no piden nada; una consulta nunca vista **empieza crecida por su plan**; el
plan se quedó corto (8 GiB) y **el vigilante lo corrigió en caliente**, sin error y sin derrame que
cuente; la fila registra cada paso con su motivo; la segunda vez no pide lo que ya tiene; y al
descansar vuelve sola a 4 GiB.

**El coste:** un `n2-highmem-8` spot con SSD, ~0,10–0,13 $/h, sólo mientras hay sesiones (mínimo 0).

## 6. Dónde está cada pieza

| pieza | dónde |
|---|---|
| el pool | `malla/75-el-session-pool.sh` (en seco; `APLICAR=1`) |
| el sabor y las colas | `malla/10-kueue.yaml` (`sesiones`), `malla/11-el-inquilino.yaml` (`cq-<celda>-sesiones`, `cola-sesiones`) |
| qué celdas lo usan | `malla/gen-inquilino.py`: `CELDAS_CON_SESSION_POOL` (hoy `{victor}`) → `--session-pool` y sin `--query-g` |
| el permiso de redimensionar | `malla/40-ore-serve.yaml`: `Role ore-serve-redimensiona` (`get`/`list` pods, `patch pods/resize`) y la salida a la API |
| una sesión al pool | `crates/ore-serve/src/cola.rs` (`al_session_pool`) |
| `scale`, la política, la cuenta | `crates/ore-serve/src/puestos.rs` (`escalar_puesto`), `elastica.rs` (`decidir_hasta`, `escalon_por_el_plan`) |
| prever | `crates/ore-serve/src/prevision.rs` (`ensena`, `al_dar`, `calentar`), `puestos.rs` (`prever_por_el_plan`), `ejecuciones.rs` (`calentar_la_prevision`); el peso, `crates/ore-core/src/admision.rs` (`Entrada.memoria`, `riesgo`) |
| redimensionar el pod | `crates/ore-k8s` (`ore-redimensionar`) |
| executions | `crates/ore-serve/src/ejecuciones.rs`, `crates/ore-core/src/sistema.rs`; leer pocas columnas, `ore-store leer` con `columnas` |
| el kernel, el vigilante, bajar | `puesto/python/agente.py` (`KernelAparte`, `Vigilante`, `bajar_si_toca`) |
| el reparto | `puesto/python/ore/__init__.py` (`_pod_mb`, `_tropo_mb`, `_ajustar`, `_RESERVADAS`) |
| pruebas | `ore-k8s`, `elastica.rs`, `prevision.rs`, `admision.rs` (planes reales en `crates/ore-core/tests/planes/`), `cola.rs`; `puesto/python/pruebas/test_kernel_aparte.py`, `test_vigilante.py`; `pruebas-de-fuego/estructura/` |

## 7. Operar

- **El pool:** `bash malla/75-el-session-pool.sh` (en seco); `APLICAR=1` lo crea; `RECREAR=1` si le falta
  el SSD o el kubelet. ⛔ En el fichero de gcloud la clave es `singleProcessOomKill` (la de la
  documentación de GKE, `singleProcessOOMKill`, la rechaza).
- **Una celda más:** añadirla a `CELDAS_CON_SESSION_POOL` y converger. ⛔ Nunca `kubectl patch` sobre
  `ore-serve`: Flux es su dueño y lo deshace.
- **Converger ya:** `kubectl create job -n ore-system converger-ahora-<algo> --from=cronjob/aprovisionador`
  (~8 min).
- **Ver cómo creció una consulta:** `select started_at, duration_ms, peak_memory_mb, spill_mb,
  memory_final_mb, growth, growth_refused from system.default.executions order by started_at desc`.
- ⚠️ Kubernetes marca `OOMKilled` aunque el contenedor salga con 0: hubo un OOM dentro, no murió la sesión.
- ⚠️ El código de la sesión comparte contenedor con el agente y puede leer la credencial del pod: lo que
  protege es la política (un escalón cada vez, *de un golpe* sólo hasta lo previsto, el techo, el
  presupuesto), no quién pide.

## 8. Deuda abierta

Cerrado para **Query** (Python, en sesión). Lo demás se normaliza cuando toque, cada uno con su paso:

| | hoy | lo que falta |
|---|---|---|
| **Node** (`puesto/node/agente.mjs`) | **va al pool** en las celdas con `--session-pool`, pero la celda corre en el REPL del proceso del agente y nadie pide crecer. **Medido (N0, 2026-10-25, `pruebas-de-fuego/estructura/n0_node.mjs`):** sin `memory_limit`, DuckDB toma el **80 % del cgroup** (3,1 GiB de 4; no la máquina) y sus hilos; V8, 2.240 MB de heap, fijo al arrancar (juntos pasan del pod); una segunda conexión contesta con otra consulta en marcha (p50 1 ms, máx. 27 ms) y `set memory_limit` en caliente tarda 2 ms; sin tope a 16 GiB, 12,7 GiB y termina | N1: el reparto del cgroup (60 %, sitio para V8) y el kernel en un proceso hijo (`fork`, con `--max-old-space-size` del escalón); N2: su vigilante, lo previsto y bajar |
| **Java** (`puesto/jvm/ore/Agente.java`) | **va al pool** igual, pero JShell corre en la JVM del agente (`executionEngine("local")`) con un reparto **fijo al arrancar** sobre `ORE_MEMORIA_MB`: heap 50 % (`-XX:MaxRAMPercentage=50`), Arrow 20 %, DuckDB 20 %. El heap máximo de una JVM no sube tras arrancar: aunque el pod crezca, el heap no. Un OOM tumba la sesión | JShell remoto (su JVM aparte, que nace con el heap del escalón), su vigilante; DuckDB sí puede subir en caliente. Medir antes el coste de arrancar la JVM remota |
| **Analytics y Models** (marimo) | el pod va al pool, pero las celdas las ejecuta marimo, fuera del kernel: sin vigilante; si marimo muere, nadie lo relanza | relanzar marimo y decirlo; su vigilante |
| **Builds** (Python y Java) | fuera del pool, tamaño fijo; lo grande, a Spark (Build elástico, 0068 CE·5, por filas). Python con el kernel aparte (un OOM falla el build con un error claro); Java con la JVM del agente, sin ese informe | decidir si un Build ligero crece como una sesión (en Python la pieza existe: falta el vigilante y el pool) |
| **Las demás celdas** | sólo victor tiene el pool; las otras siguen con L/G (`--query-g`) | el pool para todas (la lista desaparece y el flag pasa a la plantilla) |
| **Calentar al abrir** (S1b) | con el pool a 0, la primera sesión espera ~99 s | encender el nodo al abrir la página de Query |
| **La holgura del nodo** (D-E2) | sin medir | cuántas sesiones crecen a la vez; `requests` base mayores o un pod de relleno |
| **Calibrar el plan** | `FACTOR_DEL_PLAN = 1`: en la prueba final pidió 8 GiB para una consulta que acabó en 32 | el factor por familia de operador, con más filas de executions (`plan` frente a `memory_final_mb`) |
| **Crecer por uso, no por necesidad** | el vigilante crece al 80 % de uso; DuckDB usa lo que tiene, así que una consulta que derramaría barato crece igual (más rápida, R0, pero más cara) | decidir si el 80 % se queda o se crece sólo al derramar |
| **Comprobar en vivo** | el `mode` grande y `quota` se midieron en S0/S1 y en pruebas, no en la sesión de Query de la prueba final | pasarlos en la próxima sesión larga |
| **Recordar al arrancar** | `ore-serve` lee la tabla de la organización entera y filtra 7 días después | filtrar por fecha en el lago |
| **Verlo y cobrarlo** | `growth` sólo en executions | la ficha de la ejecución en la consola; GiB·h por ejecución en Billing |

