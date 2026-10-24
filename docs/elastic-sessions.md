# Elastic Sessions · autoescalado vertical en caliente (diseño, 2026-10-24)

> *Elastic Sessions* es el producto; el mecanismo, el autoescalado vertical en caliente
> (*in-place vertical scaling*) de la sesión. Plan antes que código. Sale de lo medido en R0 (`docs/ejecuciones.md` §8.11,
> `pruebas-de-fuego/estructura/`). Sustituye a las clases fijas L / G de Query y a la parte de
> §8.10 que trataba la admisión como un seguro.

## 0. Lo que se promete

**Ninguna sesión muere por lo que pulsa.** Una consulta o escritura termina, o falla con un
error de SQL que dice qué cambiar. No hay OOM, no hay «killed», no hay que relanzar en otro sitio y
el cliente nunca elige tamaño.

Cómo se consigue, en tres capas que no dependen de acertar una predicción:
1. **Derrame**: DuckDB termina casi todo con la memoria que tenga, si tiene disco (R0·1).
2. **Escalado vertical en caliente** (*in-place vertical scaling*): la sesión gana memoria y CPU **mientras corre**, antes de necesitarla, en
   2–4 s y sin perder lo hecho (R0·2).
3. **Lo que no derrama** se conoce de antemano y se le reserva memoria antes de correr (§5).

La predicción (§8.10: plan, calibración, lo medido) sólo decide **cuánto crecer de una vez**, es
decir, el coste y la latencia. No decide si la consulta sobrevive.

## 1. Lo medido de lo que parte

| hecho (R0, victor) | consecuencia |
|---|---|
| El pod crece de 4 a 16 GiB en 2–4 s (`pods/resize`, sólo memoria), sin reiniciar | crecer es barato: se puede hacer pronto y a menudo |
| DuckDB sube `memory_limit` desde otra conexión y la consulta en marcha lo usa: 45,5 s → 27,0 s (grande desde el principio: 26,6 s) | no hace falta acertar el tamaño al empezar |
| Sin sitio en el nodo: `Infeasible`, y el pod sigue igual | crecer puede no estar disponible; la consulta tiene que poder seguir derramando |
| La `ResourceQuota` del namespace (112 GiB) rechaza el crecimiento al instante | el techo del inquilino ya lo aplica Kubernetes |
| `list`/`string_agg` por grupo: `OutOfMemoryException` (controlada); `median`/`quantile`: el doble del `memory_limit` | la lista cerrada de §5 |
| El nodo de las sesiones (`e2-standard-4`) tenía 7 GB de disco libres; un disco propio por pod tarda 12–17 s en conectarse; el SSD local de `pesado`, al instante | el derrame va en un SSD local del nodo |
| `ore-serve` no tiene permisos sobre la API de Kubernetes (abre los puestos con un commit que Flux aplica) | crecer pide un permiso nuevo y mínimo (§3) |
| Kueue 0.19.3 admite por los `requests` del pod al crearlo y **no ve lo que crece** (S0·a: el pod a 3 GiB, Kueue sigue en 512 MiB y admite otro Job de 3 GiB que queda `Pending` por `Insufficient memory`) | la cuenta de lo crecido es nuestra (§4) |
| **Bajar en caliente funciona** (S0·c): `memory_limit` a 256 MB y DuckDB suelta la memoria en < 5 s (739 → 57 MB), y el pod baja de 3 a 1 GiB en 2 s | el *scale-down* de §3 es como se diseñó |
| **Un OOM del kernel mata el contenedor entero** (cgroup v2: `mode`, `histogram` y `approx_quantile` tumbaron el pod con el banco dentro, aunque el proceso culpable tenía `oom_score_adj` 1000) | lo que puede pasarse de su límite no puede compartir contenedor con el agente (§5, D-E6) |
| Cuota global de 32 vCPU, 12 en uso | cabe un nodo de sesiones de 8 vCPU |

## 2. El *session pool*

**Hoy:** las sesiones caen en `sistema-spot` (`e2-standard-4`, 16 GB, 30 GB de disco compartido con
el sistema), unas 13 por nodo, con 4 GiB de techo cada una. No hay sitio para crecer ni disco para
derramar.

**Propuesta:** un pool `sesiones` propio:
- **máquina**: `n2-highmem-8` (8 vCPU, 64 GB; 57 GiB asignables, medido), spot, con **un SSD local**
  (375 GB) para `/trabajo` y el derrame. Es la de `pesado`, que ya está probada con DuckDB.
- **reparto**: una sesión nace con `requests 768Mi / 100m` y `limits 4Gi / 2 CPU` (lo de hoy, C1) y
  crece por escalones hasta su techo (§3). En un nodo caben unas 10 sesiones en reposo **y** dos
  creciendo a 16 GiB a la vez. El resto, en §4 (la holgura).
- **caliente en el horario** (08–00 Madrid, 0060): el pool arranca a 1 nodo y escala por
  `requests`. Un nodo a 0 tarda 92 s (medido en Q0), y eso no puede pasarle a quien abre una sesión.
- **`sistema-spot` vuelve a ser del sistema**: ore-serve, cofre, informador y compañía.
- **G desaparece como sitio aparte**: lo que hoy va al puesto pesado es una sesión que ha crecido.
  `pesado` queda para los Builds, y E (Spark elástico) para lo que no cabe en un nodo.

**Coste** (D-E1): un `n2-highmem-8` spot con SSD, 16 h al día, frente al `e2-standard-4` de hoy más
el `pesado` que hoy arranca cada vez que algo va a G. Se mide en la facturación de la primera semana
(S0·d).

## 3. Cuándo y cuánto escalar (*scale steps*, *pre-scale*, *scale-down*)

**Quién decide y quién ejecuta:**
- **el puesto** (el agente, que ve a DuckDB por dentro) **pide**: `POST /puestos/{id}/scale
  {memoria_mb, cpu, motivo}`, con su credencial de agente. El código del cliente no puede pedirlo:
  lo pide el agente, no el SDK;
- **`ore-serve`** comprueba la política (escalón, techo de la sesión, techo de la persona y del
  inquilino) y **redimensiona el pod**. Para eso, su cuenta gana **sólo** `patch` sobre
  `pods/resize` y `get` sobre `pods` en el namespace de su celda. Ese subrecurso cambia únicamente
  CPU y memoria: ni imagen, ni volúmenes, ni la seguridad del pod;
- **el puesto** ve subir su `memory.max` (cgroup) y sube `memory_limit` y `threads` de DuckDB desde
  otra conexión. Es exactamente lo medido en R0·2.

**Cuándo** (lo mira el agente cada 200 ms mientras corre una celda):
- la memoria de DuckDB (`duckdb_memory()`) pasa del 80 % de su `memory_limit`, o
- DuckDB empieza a derramar (aparecen ficheros en su directorio temporal), o
- **antes de correr**, si la admisión prevé más de lo que tiene la sesión (§8.10, capas 2–4): crece
  primero y corre después, en 2–4 s, sin derramar nada.

**Cuánto** (*scale steps*): escalones que se doblan, **4 → 8 → 16 → 32 → 48 GiB** (con CPU 2 → 4 → 6 → 8 → 8), y de
uno en uno salvo que la previsión diga más. El techo de una sesión, 48 GiB, es lo que cabe en un nodo
con holgura (D-E4). Por encima de un nodo, E.

**El reparto dentro del pod:** DuckDB recibe el 60 % de lo que tenga el cgroup, y el **45 %** si la
consulta lleva algo de la lista cerrada (§5). El resto es para Python, el escritor (que en flujo pesa
~1 GB) y lo que pasa del límite.

**Bajar** (*scale-down*): tras 10 minutos sin celdas, el agente baja primero `memory_limit` y luego pide volver al
escalón base. Kubernetes no deja bajar por debajo de lo que se usa, así que se baja en ese orden. La
sesión cuesta lo que pesa y solo mientras lo pesa.

**Si no se puede crecer** (`Infeasible` porque el nodo está lleno, o la cuota): la consulta **sigue
derramando**, más lenta pero termina, y `ore-serve` lo apunta (§6). No se reintenta en otro sitio.

## 4. Kueue, la cuota y la holgura

- **Kueue** sigue admitiendo la sesión al abrirla, por sus `requests` base. No ve lo que crece:
  0.19.3 admite workloads, no redimensionados (a confirmar en S0·a).
- **La `ResourceQuota`** del namespace es la que cuenta lo que crece: medido, rechaza un crecimiento
  que la pase. Es el techo del inquilino. Hoy son 112 GiB de `requests.memory`, y se revisan con el
  pool nuevo (D-E3).
- **El riesgo:** Kueue cree que hay sitio (porque no ve lo crecido) y admite una sesión nueva que la
  `ResourceQuota` luego rechaza al crear el pod, y esa sesión no abre. Se evita con la ClusterQueue de
  sesiones contando el **techo** de cada sesión (48 GiB) y no su base, o con `ore-serve` llevando la
  cuenta de lo crecido y no pidiendo sesiones que no caben. Se decide en S0·a, con la prueba.
- **La holgura del nodo** (D-E2): para que crecer casi nunca sea `Infeasible`, el nodo reserva sitio
  para N sesiones creciendo. Se consigue con `requests` mayores que lo que se usa (p. ej. 2 GiB de
  base) o con un pod de relleno de prioridad baja que el planificador echa cuando hace falta (el patrón
  de *overprovisioning* del autoescalador), que además mantiene un nodo de más caliente. Se mide
  cuántas sesiones crecen a la vez en victor.

## 5. Los *reserved operators* (la lista cerrada)

Medido (R0·1 y S0·b, un puesto como L: 4 GiB, DuckDB con 1 GB, 40 M filas):

| operación | qué pasa | clase |
|---|---|---|
| agrupar, join, ordenar, ventana (también con marco), `count(distinct)`, `pivot`, CTE recursiva, join espacial, `arg_max`, `ST_Collect`, `ST_Union_Agg`, `create … as` | terminan (derramando si hace falta) | **seguras** |
| `list`, `array_agg`, `string_agg`, `json_group_array` por grupo | `OutOfMemoryException`: falla la consulta, la sesión sigue | **reservadas**: se les da memoria antes de correr |
| `median`, `quantile_cont` | terminan, pero con el **doble** del `memory_limit` | **reservadas**, y DuckDB al 45 % del cgroup |
| **`mode`, `histogram`, `approx_quantile`** por muchos grupos | **OOM del kernel: el contenedor entero muere** | **peligrosas**: memoria reservada de sobra, o donde no puedan tumbar la sesión (D-E6) |

Se detectan en el plan antes de correr (nombres de función en los agregados). Si aun reservando no
caben en un nodo, van a E, que lo reparte.

## 6. Lo que queda en executions

Cada fila gana lo que creció: `growth` (`[{at_ms, from_mb, to_mb, cpu, reason}]`), `memory_final_mb` y,
si no se pudo crecer, por qué (`infeasible` o `quota`). Con eso se ve:
- cuánto hay que crecer al empezar (la previsión aprende de lo que creció, §8.10);
- cuántas veces el nodo no tuvo sitio (la holgura de §4 se ajusta con eso);
- cuánto cuesta cada sesión (Billing, §8.9).

## 7. Los pasos

| paso | qué | listo cuando |
|---|---|---|
| **S0 · previos** (en victor, sin código de producto) | **a)** Kueue 0.19.3 con un pod redimensionado: ¿cambia la cuenta del ClusterQueue? ¿admite de más? **b)** el banco de R0·1 con la lista ampliada (§5) **c)** bajar en caliente: `memory_limit` abajo y luego el pod, ¿DuckDB suelta la memoria? **d)** precio del `n2-highmem-8` spot con SSD en el horario, frente a lo de hoy | las cuatro respuestas en `docs/ejecuciones.md` §8.11 y aquí |
| **S1 · el *session pool*** | `malla`: el pool (`n2-highmem-8`, SSD local, spot, **mínimo 0**, `singleProcessOOMKill: true` en su kubelet, D-E6), su `ResourceFlavor` y su cola, el pod de relleno que lo calienta al abrir Query, `resizePolicy: NotRequired` en `51-el-puesto.yaml`, `/trabajo` en el SSD local; las sesiones nuevas van ahí | **primero** el caso `mode` de r01c en el pool nuevo: muere el proceso, no el contenedor; luego una sesión abre en el pool (en caliente ~4 s; en frío, lo medido) y derrama en su SSD |
| **S2 · `ore-serve` escala pods** | la cuenta con `pods/resize` (sólo en su namespace); `POST /puestos/{id}/scale` con la política (escalones, techos, cuota); **la cuenta de lo crecido** (D-E3); `growth` en executions (lo que era S5); el cliente HTTP mínimo a la API de Kubernetes | prueba de `ore-serve` (política) y en victor: una petición crece el pod en ≤ 5 s |
| **S3 · el agente pide y aplica** | el vigilante (80 % de `memory_limit` o derrame) → pide; al ver crecer el cgroup, sube `memory_limit` y `threads`; el reparto 60 % / 45 %; bajar tras 10 min sin celdas; si el kernel muere por OOM, la celda lo dice y el kernel se reabre | pruebas del SDK; en victor, la escalera 3M / 30M / 300M en **una** sesión, ninguna muerta |
| **S4 · *pre-scale*** | la admisión (§8.10) dice cuánto, y `ore-serve` crece antes de mandar la celda; la lista cerrada (§5) | la de 300M crece al empezar y no derrama |
| **S6 · retirar G** | Query deja de abrir el puesto pesado para las sesiones (`abrir_puesto_en_g`, los espejos de la hoja); `pesado` queda para los Builds | la hoja de Query es un solo puesto que crece |

S0 va primero (iterar a tiro fijo): Kueue (a) y bajar en caliente (c) pueden cambiar el diseño de §3 y
§4.

## 7b. S0 · medido (2026-10-24)

- **a · Kueue**: no ve lo que crece (arriba, §1). D-E3 deja de ser una opción: la cuenta de lo crecido
  la lleva `ore-serve` (o la cola cuenta el techo de cada sesión), o Kueue admitirá sesiones que no abren.
- **b · la lista cerrada**: §5. Tres operaciones matan el contenedor.
- **c · bajar en caliente**: funciona (§1).
- **d · precio**: un `n2-highmem-8` spot en europe-west1 cuesta ~0,095–0,133 $/h según la fuente, y el SSD
  local ~0,08 $/GB-mes (375 GB ≈ 30 $/mes). En el horario de 16 h: **~60–85 $/mes por nodo**, frente a
  ~33 $/mes del `e2-standard-4` spot de hoy (tarifas públicas de terceros; se confirma en la facturación).

## 7c. S1 · en código (2026-10-24), sin aplicar

- `malla/75-el-session-pool.sh`: el pool `sesiones` (`n2-highmem-8` spot, 0–1, SSD local,
  `singleProcessOOMKill: true`, taint y etiqueta `ore.dev/pool=sesiones`). En seco por defecto;
  en seco contra el proyecto: cabe en la cuota (4 + 8 de 32).
- `malla/10-kueue.yaml`: el sabor `sesiones`. `malla/11-el-inquilino.yaml`: `cq-<celda>-sesiones`
  (6 CPU / 40Gi de base, sin cohorte) y `cola-sesiones`. Pasan el *dry-run* del servidor en t-victor.
- `ore-serve --session-pool` (`cola::al_session_pool`): una sesión —no un trabajo ni el puesto
  pesado— va a `cola-sesiones`, con `resizePolicy NotRequired` en memoria y CPU, y `/trabajo` de
  64 GiB en la SSD (`ORE_DISCO_MB` con él). Nace con la base de hoy. Sin el flag, nada cambia. El
  Job rendido pasa el *dry-run* del servidor.
- `dormir.sh` y `dormir-de-noche.py` también llevan `sesiones` a 0.
- La primera prueba: `bash pruebas-de-fuego/estructura/medir.sh s1-mode` (`agrupa_alta`, `mode`,
  `histogram`, `approx_quantile` con la forma de una sesión): pasa si las tres peligrosas salen
  `killed`, el banco llega a `fin` y el Job termina bien.

**El orden para aplicarlo** (binario antes que malla, y pool antes que colas):
1. `APLICAR=1 bash malla/75-el-session-pool.sh` (crea el pool a 0: no cuesta nada hasta que algo
   lo pida);
2. converger la malla (el sabor y las colas de cada celda);
3. `medir.sh s1-mode`: si las tres salen `killed` con el Job vivo, D-E6 queda cerrada;
4. desplegar `ore-serve` con este código y encender `--session-pool` en t-victor; medir la
   apertura en frío (nodo a 0) y en caliente, y que una sesión derrama en su SSD.

### S1 · aplicado y medido en victor (2026-10-24)

- **Pool `sesiones` creado** (`APLICAR=1 bash malla/75-el-session-pool.sh`): 0–1, SSD, RUNNING. La clave
  del fichero de gcloud es `singleProcessOomKill` (la de la documentación, `singleProcessOOMKill`, la
  rechaza el SDK 579). Sabor `sesiones`, `cq-victor-sesiones` y `cola-sesiones` aplicados en t-victor.
- **D-E6 CERRADA: `s1-mode` pasa.** Con la forma de una sesión (4 GiB, DuckDB a 1 GB, 40 M filas):

  | caso | S0·b (pool de hoy) | S1 (session pool) |
  |---|---|---|
  | `agrupa_alta` | ok | ok, 6,4 s, derrama 755 MB en la SSD |
  | `mode` | **el contenedor entero muere** | `killed` (−9) sólo su proceso, 31,6 s |
  | `histogram` | **el contenedor entero muere** | `killed` (−9) sólo su proceso, 35,3 s |
  | `approx_quantile` | **el contenedor entero muere** | `killed` (−9) sólo su proceso, 2,8 s |

  El banco (el que hace de agente) sigue vivo, llega a `fin` y el Job termina **bien** (`1/`).
- ⚠️ **Kubernetes marca el contenedor `OOMKilled` aunque salga con 0**: la razón dice que hubo un OOM,
  no que muriera. `ore-serve` no puede leer `OOMKilled` como «la sesión murió»; manda el código de salida.
  El agente (S3) es quien sabe qué celda murió y lo dice.
- **Apertura en frío, primera vez**: Job 17:01:09 → el autoescalador pide nodo 17:01:43 (34 s; un pool
  recién creado tarda en verse) → nodo 17:03:09 → contenedor 17:03:21: **~2 min 10 s**.
- **Apertura medida** (`s1_apertura.sh`: un Job con la forma de una sesión en `cola-sesiones`, hasta que
  el proceso importa duckdb; como `arranque` de Q0):

  | | admitido | nodo listo | contenedor | listo |
  |---|---|---|---|---|
  | **en frío** (pool a 0) | 1,4 s | 93,0 s | 99,1 s | **99,1 s** |
  | **en caliente** (×2) | 1,2–1,3 s | — | 1,2–1,3 s | **1,2–1,3 s** |

  En caliente abre tan rápido como hoy en el nodo del sistema; en frío, lo de Q0 (92 s): es lo que
  calentar al abrir Query (S1b) tiene que esconder. A la sesión real se le suman Flux y el agente, igual
  en los dos pools.

- **`--session-pool` encendido en t-victor** (18:56Z): `gen-inquilino.py` lleva la lista
  `CELDAS_CON_SESSION_POOL = {victor}` y la convergencia lo escribe sólo en su `ore-serve`; las demás
  celdas tienen la cola pero no el flag. Forzar una convergencia entera sin esperar a la hora en punto:
  `kubectl create job -n ore-system converger-ahora-<algo> --from=cronjob/aprovisionador` (~8 min las
  cinco celdas).

## 8. Decisiones abiertas

- **D-E1 · la máquina del pool. DECIDIDA (2026-10-24):** `n2-highmem-8` spot con un SSD local desde ya, con
  **mínimo 0**: se paga por uso (el autoescalador lo quita ~10 min después de vaciarse) y se calienta con un
  pod de relleno al abrir Query. Mínimo 1 en el horario, sólo en la rampa a producción.
- **D-E2 · la holgura.** Propuesta: `requests` base de 2 GiB (unas 25 sesiones en reposo por nodo,
  con sitio para que 2 crezcan a 16 GiB) y medir.
- **D-E3 · la cuota del inquilino. DECIDIDA tras S0·a:** la cuenta de lo crecido la lleva `ore-serve` (S2);
  contar el techo en la cola dejaría una sesión por nodo. Antes se proponía: la `ResourceQuota` como techo (ya funciona), y la
  ClusterQueue de sesiones contando el techo de cada sesión, si S0·a confirma que Kueue no ve lo que
  crece.
- **D-E4 · el techo de una sesión.** Propuesta: 48 GiB; por encima, E.
- **D-E5 · spot.** Una sesión en un nodo spot puede perder el nodo (con 30 s de aviso). Propuesta:
  sesiones en spot, como hoy; si GCP recupera el nodo, la sesión se reabre en otro y la celda que
  corría dice por qué. El estado de la sesión vive en el lago y en el árbol, no en el pod. Revisar si
  se mide que pase a menudo.
- **D-E6 · el agente y el código, en contenedores distintos. CERRADA (S1, 2026-10-24): basta `singleProcessOOMKill`, medido.** En cgroup v2 un OOM mata el contenedor
  entero (S0·b). Hoy el agente, DuckDB, el SDK y el escritor viven en uno: si una operación peligrosa se
  desborda, cae la sesión. Propuesta: el agente en un contenedor y el código (kernel, DuckDB, escritor) en
  otro, del mismo pod. Cada uno tiene su cgroup y se redimensiona por separado, y si el del código muere,
  el agente lo dice en la celda y lo reabre, con la sesión viva. Se mide en S1.
  **Camino elegido:** GKE expone `singleProcessOOMKill` en la configuración del kubelet del pool
  (`NodeKubeletConfig`): con él, un OOM mata sólo el proceso que se pasa (el kernel) y no el contenedor. Es
  ajuste del pool nuevo, no cambia el pod. Si la prueba de S1 no lo confirma, el plan B es el contenedor
  aparte (*native sidecar*).
