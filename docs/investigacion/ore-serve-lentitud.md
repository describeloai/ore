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
