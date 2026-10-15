# ore-serve: dónde está lento y por qué (2026-10-14, en curso)

Investigación, no decisión: primero se mide dónde se va el tiempo y qué lo provoca; los
arreglos se deciden después, con los números delante. Origen: I4 de la idempotencia (ADR
[0031](../decisions/0031-el-puesto.md), «La idempotencia, revisada»). Cada `write()` desde un
puesto de victor tardaba ~21 s, y al abrir el workspace todo ore-serve se paró varios minutos.

## L0 · lo visto (registro de accesos de victor, 2026-10-14)

**No es lentitud repartida: es un atasco.** La mediana de casi todas las rutas es baja
(`/assets` 128 ms, `loadTable` 706 ms, `/fuentes` 605 ms), pero el máximo se dispara a 4–6
minutos. Todas las peticiones largas caen en **la misma ventana** y salen **en cascada**:

| Empieza | Termina | Duración | Petición |
|---|---|---|---|
| 20:36:07 | 20:42:11 | 364 s | `GET /puestos/{}/datos/{}` (el `over()` de un Run) |
| 20:36:23 | **20:41:08** | 284 s | `GET /fuentes` |
| 20:36:23 | 20:42:08 | 344 s | `GET /assets` |
| 20:36–20:39 | 20:41:16–19 | 117–273 s | `loadTable` ×3 |
| 20:39:31 | 20:41:10 | 99 s | `/fuentes`, `/arbol`, `/assets` (la consola, al abrir el workspace) |

- **CPU parada** durante el atasco: 45m de 500m. ore-serve no calculaba: esperaba.
- **La forja no recibió ni una petición de ore-serve entre 20:36:07 y 20:41:10.** La primera
  después (un `git-upload-pack`) tardó 3,1 s, y todo salió detrás.
- **Después**, cada `GET /puestos/{}/datos/{}` (el `over()`) tardó siempre **~15 s**, y cada
  `write()` ~21 s.
- El «`sin`» (sin identidad) de algunas líneas engaña: la clase se calcula **al terminar** la
  petición (`recuento.rs`), y el token caducó durante la espera. Era la consola, no un sondeo.

## L0 · lo que dice el código (dónde podría ser; no cuánto)

1. **Un candado de todo el proceso.** Todo lector del árbol (`/fuentes`, `/arbol`, `/assets`,
   `loadTable`, el `over()` de un puesto) toma `Espejo.estado` (`git.rs`), y lo retiene mientras
   `git fetch` habla con la forja. **Ningún `git` ni `ore` tiene plazo**: ni `http.lowSpeedLimit`,
   ni límite al subproceso. Un `fetch` colgado deja a todo ore-serve esperando sin CPU, y al
   soltarse todo sale en cascada. Encaja con lo visto, pero **no está demostrado**: no había
   registro de esperas.
2. **El `over()` rehace el índice de activos.**
   - `GET /puestos/{id}/datos/{x}` toma el candado de 3 a 5 veces (con un `fetch` cada una) y
     pide la salud del activo, que necesita el índice.
   - La caché del índice es de **4 entradas para todo el proceso** (`assets.rs`), cada puesto
     está en su rama y cualquier commit en `main` las invalida todas.
   - Rehacerlo lanza **un `git log` por activo**: 376 en victor.
   - Más dos subprocesos `ore`: uno presta la credencial, con el intercambio STS de GCS.
3. **Peticiones de minutos sin tope.** Un hilo por conexión, con un tope de 64; nada limita la
   duración de un manejador, y los subprocesos no tienen plazo.
4. `/fuentes` **no habla con ningún origen**: lee `ontology.config.yaml` del árbol. Sus 284 s
   fueron espera al candado.

## L1 · medir (`77aa6642`)

`ore-serve/src/fases.rs` acumula, por petición:
- la espera al candado del espejo;
- los `git` que hablan con la forja y los locales;
- los subprocesos `ore`;
- el índice de activos, de la caché o rehecho, y cuánto.

Todo eso va al final de la línea de acceso. Un `git` de más de 5 s se dice en el momento
(`lento · git …`).

*(Lo medido con L1 en victor, aquí.)*

## L2 · lo que cuesta el índice (medido en el pod de ore-serve de victor, 0,5 CPU)

Sobre un clon del árbol de victor hecho dentro del propio pod: 474 commits, 751 ficheros, 438
documentos en `packages/`, `.git` de 1,4 MB.

| Operación | Tiempo |
|---|---|
| clon completo de la forja | 3,2 s |
| `git fetch --prune` sin cambios | 90–150 ms |
| `git ls-remote --heads` (16 ramas) | ~90 ms |
| `git status --porcelain --ignored` | 99 ms |
| cargar y compilar el paquete (`ore validate`) | 380 ms |
| **`git log -1` por documento (lo que hace `version_de`)** | **438 × 26 ms = 11,6 s** |

**Los ~15 s de cada `over()` son esto.** Rehacer el índice es ~97 % lanzar un proceso `git log`
por documento. Cargar el paquete, hablar con la forja y lo demás no llegan a medio segundo. Como
la caché del índice casi nunca acierta (4 entradas, una rama por puesto, invalidada por cada
commit en `main`), se paga en casi cada lectura. Crece con el árbol: con el doble de documentos,
el doble de tiempo.

## L1 en producción · lo que cazó (victor, 2026-10-15)

**08:45–08:55, ore-serve murió por memoria (`OOMKilled`, límite 512 Mi).** En esos 10 minutos el
registro sólo tiene 28 `GET /fuentes` y 2 `/ramas`, pero hay **16 clones lentos (5–12 s)**: son de
peticiones que seguían en marcha cuando murió (sin línea de acceso). La consola manda `/fuentes`
en ráfagas (5 a la vez a las 08:50:32), y cada una espera el candado hasta 6,9 s.

**09:12–09:14, el catálogo con cambios de rama** (la persona: «cambio de rama y no funciona»):

| Petición | Total | Dónde se va |
|---|---|---|
| `GET /assets` en `main` | 0,25 s | índice de la caché, 0 ms |
| `GET /assets` en una rama | **20,2 s y 24,6 s** | **índice rehecho 17,6 s y 18,4 s**, con **474 y 436 `git` locales** |
| `GET /fuentes` en una rama | 0,9–8,3 s | **89–94 `git` locales** (montar la rama sobre `main`) para leer UN fichero, más hasta 4,8 s de candado |
| `GET /ramas` | 1,1–1,8 s | repetida cada pocos segundos |

Lo de L2, confirmado en producción: fuera de `main` la caché falla siempre y el índice se rehace
con un `git log` por documento. La memoria subió a 115 MiB (sin morir) y los procesos `git`/`ore`
vivos de 7 a 29: los rebuilds se solapan.

## L3 · git sin plazo (Docker, git 2.49.1, `pruebas-de-fuego/lentitud/l3-git-sin-plazo.sh`)

| Caso | Sin plazo (ore-serve hoy) | Con `http.lowSpeedLimit=1000`, `lowSpeedTime=10` |
|---|---|---|
| A · la conexión no se establece (SYN descartados) | **135 s** | 135 s (no cubre la conexión) |
| B · conecta y calla | **más de 6 min, sin rendirse** | **10 s** |
| C · cabeceras y a medias | **más de 6 min, sin rendirse** | **10 s** |

Un `git` contra una forja que calla puede quedarse colgado **para siempre con el candado
tomado**: es el atasco del 14. `lowSpeedLimit` corta B y C; para A (lo que dicta el kernel:
`tcp_syn_retries=6` en el pod) hace falta además un plazo al proceso (matarlo a los N s).

## L4 · la memoria (medido en el pod de victor)

- Un clon local del árbol añade ~9 MiB (1 MiB anónimo); `ore validate` (cargar y compilar el
  paquete), ~8 MiB. **Los clones solos no explican el OOM**: harían falta ~50 a la vez.
- `/tmp`, donde se clona, está en la capa del contenedor, no en memoria (`emptyDir`).
- Lo que se come la memoria está **en el propio proceso de ore-serve** al servir el catálogo:
  los índices rehechos en paralelo (uno por petición y rama, cada uno con el paquete entero y su
  JSON). Sin medir todavía por petición.
- **ore-serve es el PID 1 y no recoge a sus huérfanos**: se acumulan `git` zombis (6 en 16 min).
  No gastan memoria, pero sí PIDs; con un `init` (`tini`) en la imagen, desaparecen.

## Los arreglos, por lo que más se nota

| # | Arreglo | Medido → esperado |
|---|---|---|
| **F1** | el índice sin un `git log` por documento: una pasada de `git log --name-only` | 436–474 procesos y 17,6–18,4 s → 1 proceso |
| F2 | plazos a git (`http.lowSpeedLimit`/`lowSpeedTime` y matar a los ~30 s) | cuelgue sin fin → 10 s (B, C); 135 s → 30 s (A) |
| F3 | `/fuentes` y las lecturas de un fichero sin montar la rama (`git show <rama>:<fichero>`) | 89–94 `git` → 1 |
| F4 | caché del índice por rama, más grande, sin invalidarla entera con cada commit en `main` | acierto 0 % fuera de `main` → alto |
| F5 | `tini` como PID 1 de ore-serve | `git` zombis acumulados → 0 |
| F6 | la consola: `/fuentes` y `/ramas` sin ráfagas | 5 `/fuentes` a la vez; `/ramas` cada pocos segundos |

**F1 (2026-10-15).** `versiones_de` (`ore-serve/src/assets.rs`): una pasada de
`git log -z --name-only` sobre toda la historia, y cada ruta —documento, proyecto o carpeta de
repositorio— se queda con el primer commit que la toca a ella o a algo debajo.
- Medido **antes de escribirlo**, en el pod de victor: la pasada tarda **190–219 ms**, frente a
  11,6 s.
- Sobre una copia del árbol de victor, **la misma versión** que `git log -1 -- <ruta>` para 599
  documentos y 144 carpetas (0 distintas).
- Prueba: `una_pasada_da_la_misma_version_que_una_por_ruta`.

**F2 (2026-10-15).** Plazos a git en `Forja::git` (`ore-serve/src/git.rs`):
- `http.lowSpeedLimit=1000`, `http.lowSpeedTime=10` en el entorno de cada `git`;
- **`PLAZO_GIT` = 60 s** al proceso: `salida_con_plazo` lo mata y no espera a su hijo HTTP;
- un cuelgue se dice (`plazo · git … · matado a los 60 s`) y la petición recibe «la forja no
  responde: no contestó en 60 s» en vez de quedarse colgada con el candado tomado.

Pruebas:
- `una_forja_que_calla_se_abandona_en_segundos`: el caso B de L3, que sin plazo pasaba de 6
  minutos, se abandona en ~10 s con «Operation too slow»;
- `lo_que_se_pasa_del_plazo_se_mata`.

**Medido tras F1 y F2 en victor (2026-10-15, la persona cambiando de rama):**
- `/assets` en rama, de 20–25 s a **5,5–6,3 s**: índice 0,5–1,7 s (antes 17,6–18,4), pero
  **197–235 `git` locales, 4,8–5,2 s**;
- `/fuentes` en rama, 2 s con **89 `git`**.

Lo que queda es **montar la rama**. `superponer` lanzaba un `git cat-file` por cada puntero que
`main` movió, y `ocultar_al_dia` dos más por puntero (`cat-file -e` y `update-index`). Es un
coste por puntero y por petición: con 10.000 activos, ~20.000 procesos por cada vez que alguien
mira una rama.

**F3a (2026-10-15).** La superposición, por lotes:
- los blobs de `main`, en **un** `git cat-file --batch` (`blobs_por_lote`; `salida_con_plazo`
  gana la entrada por stdin);
- «¿está en HEAD?», de **un** `ls-tree`;
- `--skip-worktree`, en lotes de 200.

La decisión (`manda`) no cambia. Pruebas: ore-serve entero (234), `la-propuesta.sh` 1–10 y
`los-datos-en-una-rama.sh` 1–15 (lectura, escritura, cambios y fusión en una rama, contra la
forja).

**F3b (2026-10-15).** Leer una rama sin clonarla y sin un candado para todo (`ore-serve/src/git.rs`):
- el `fetch` del espejo, **compartido**: quien llega mientras otro trae, o antes de que empiece el
  siguiente, se sirve de ese;
- la lista de árboles, con un candado corto, y cada árbol **montado fuera de él, una vez por
  clave** (quien pide la misma rama mientras se monta, espera a ese montaje);
- `/assets` sin commit lee el árbol de lectura compartido, no un clon para escribir;
  `ARBOLES` 8 → 16.

**F4 (2026-10-15).** La caché del índice (`ore-serve/src/assets.rs`):
- la clave sale del espejo (`clave_para_leer`), no de dos `ls-remote`;
- caben 64 índices mientras no pasen de `PESO` bytes de JSON;
- un índice se hace **una sola vez por clave** (`Cache::turno`).

`PESO` empezó en 48 MiB y L5 lo tumbó: en memoria un índice pesa **~7 veces su JSON**. Bajó a
8 MiB (`43d67add`).

Pruebas: ore-serve 237, `la-propuesta.sh` 1–10, `los-datos-en-una-rama.sh` 1–15. Los pasos 16, 17
y 20 de este último (las colecciones en una rama) fallan igual con F3a: con F3a no llegaron a
correr porque faltaba `ore-read-s3`. No son de F3b ni de F4.

## L5 · a escala (Docker, ore-serve release con 0,5 CPU y 512 MiB, forja `file://`, 50 ramas, 221 commits en `main`)

`pruebas-de-fuego/lentitud/l5.sh`. Cada cifra es p50 / máx en ms y errores/peticiones. Los
errores son agotar los 300 s o el servidor muerto.

| Escenario | Antes (F1+F2) | F3b+F4 |
|---|---|---|
| 1.000 · 1 usuario · `/assets` rama | 55.204, 2 peticiones en 60 s | **1.968 / 2.505**, 59 peticiones |
| 1.000 · 1 · `/fuentes` rama | 7.866 | **1.018 / 2.491** |
| 1.000 · 10 · `/assets` rama | 170.441 / 191.658 | **4.905 / 32.424** |
| 1.000 · 10 · `/fuentes` rama | 41.068 / 59.610 | 11.213 / 17.289 |
| 1.000 · 50 | 300 s, 22/67 errores, 252 MiB | **OOM** (511 MiB), 513 peticiones |
| 10.000 · 1 | 300 s en `/assets` rama, 114 MiB | 24.523 y luego **OOM** |
| 10.000 · 10 | todo ~300 s, 476 MiB | 141–191 s, 363 MiB |
| 10.000 · 50 | todo 300 s, 493 MiB | **OOM** |

Los candados dejan de contar (0–1,3 s, antes 246 s). Lo que queda es **CPU** (montar una rama:
9–15 s de `git` local con 10 a la vez) y **memoria**: hay más peticiones en marcha a la vez, y
nada las limita.

## L6 · la memoria de UN índice a 10.000 activos (`l6-memoria.sh`, una petición cada vez)

| | |
|---|---|
| JSON de `/assets` | **11,7 MB** (main) y 11,8 MB (rama) |
| hacerlo | 9–18 s de índice, más 1–9 s de `git` local |
| memoria anónima tras el primero | **224 MiB**; con dos, 296 MiB |
| pico del cgroup al hacer el primero | **431 MiB** de 512, sin carga |
| caché | 11,7 MB > `PESO`: sólo se guarda el último, y en la segunda vuelta falla siempre |

**Conclusión de L5 y L6: a 10.000 activos el problema ya no son los candados ni `git`. Es la
forma del índice.** `/assets` es el catálogo ENTERO —11,7 MB de JSON— hecho en memoria de una vez
(un pico de ~400 MiB) y rehecho entero para cada rama y cada commit de `main`, aunque una rama
comparte con `main` casi todos sus paquetes. Ningún ajuste de topes lo arregla. Hace falta un índice
**por paquete, direccionado por contenido** (el hash del árbol git de cada paquete, que es el mismo
en todas las ramas que no lo tocan) y **servido por partes**.

## L6 en victor (2026-10-15, `43d67add`, la persona cambiando de rama)

| Petición | Antes de F1 | F1+F2 | F3b+F4 |
|---|---|---|---|
| `/assets` en rama | 20–25 s | 5,5–6,3 s, ~200 `git` | **0,7–1,2 s** (índice 0,3–0,7 s), 12–15 `git` |
| `/assets` vuelta a una rama | — | igual (fallaba siempre) | **0,64 s**, de la caché (0,4 s son el `fetch`) |
| `/assets` en `main`, en frío | — | — | 2,0 s |
| `/fuentes` en rama | 0,9–8,3 s, 89–94 `git` | 2 s, 89 `git` | **0,36–1,0 s**, 4–10 `git` |
| `/ramas/{}/{}/cambios` | — | — | 2,1–3,4 s (`ore` 0,35–1,1 s, `git` 0,6–0,8 s) |
| `/ramas` | 1,1–1,8 s | — | 0,8–1,3 s |

Memoria: 25 MiB anónimos, 20 MiB de ficheros y **pico de 100 MiB** con 5 árboles montados, de 512
MiB. Sin reinicios. **23 procesos `git` zombis** en ~20 minutos: lo de F5 (`tini`) sigue pendiente.
