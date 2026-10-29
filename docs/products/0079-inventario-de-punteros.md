# Inventario de los punteros (ADR 0079, E1a·5a)

> **Estado:** medido el 2026-10-28 sobre `main` + E1a·4, solo lectura. Es la lista de cada sitio que
> lee, escribe o recorre la historia de un puntero (`datasets/**`, `copias/**` y, en parte,
> `resultados/**`), y el camino por el que llega. De aquí sale la interfaz `Punteros` (·5b): cada fila
> pasa a ser una llamada a una operación de la interfaz, o desaparece.
>
> Leyenda: **R** lee el puntero de hoy · **W** lo escribe, mueve o borra · **H** lee su historia o
> otra ref en git. «Mixto» = el mismo commit cambia también un documento.

## 0. El resumen en una tabla

| Capa | Sitios | Cómo llega al puntero | Qué pasa con el corte |
|---|---|---|---|
| **ore-serve** (en proceso) | 58 (R 37 · W 18 · H 10) | su espejo: `leyendo_en`/`escribiendo_en(rama)` monta un árbol con `superponer` (`manda`) y `publicar` hace `git add -A` | pasa por `Punteros`; la superposición, `publicar` de punteros y la fusión de punteros en git **desaparecen** |
| **ore-cli llamado por ore-serve** (`mando::correr`) | 13 verbos | lee y escribe ficheros en el árbol que ore-serve le monta | el mismo `Punteros`, que ore-serve le pasa o le contesta |
| **ore-cli en Jobs** (48 copia, copiador, 49 invocación, 53 mantenimiento) | ~14 | clon propio; el shell hace `ls-tree`/`show` («reclaman») y `git add -A datasets` + push | pasan a `Punteros` por HTTP contra ore-serve (es el único con la credencial del catálogo) |
| **Scripts de la malla** | 6 rutas + 2 de copia/traslado | git y ficheros a pelo (`cerrar-ejecucion.py`, `copiar-worker.sh`) | los mismos que los Jobs; la copia (31) y el traslado (`aprovisionar`) dejan de cubrir punteros |
| **SDK** (Python, JVM, Node, Spark) | ~30 | **siempre por HTTP** (`/v1`, `/puestos/{id}/datos`, `/media`, `/assets`) | nada, si las rutas contestan igual |
| **Consola** | ~16 | **siempre por HTTP** (`/assets`, `/historia`, `/paquetes/{n}/copias`, `/arbol`) | nada, salvo la fila «Pointer» de la ficha, que ya miente |
| **Pruebas** | 5 contra la malla viva, 13 locales | clonan y leen o escriben ficheros de punteros | se reescriben contra la API (las 5 vivas, antes del corte) |
| **Otras crates** (ore-store, ore-medios, ore-teselas, ore-piramide) | 0 en git | reciben un `metadata_location` | nada |

**Lo que el corte toca de verdad son dos crates (ore-serve y ore-cli) y cinco Jobs.** Ni el SDK ni la
consola ven git.

## 1. Las operaciones que piden los sitios (lo que será `Punteros`)

| Operación | Quién la pide hoy (resumen) | Hoy la resuelve |
|---|---|---|
| **uno(ref, nombre)** → puntero efectivo y de dónde sale (`main`/`rama`) | `/v1` loadTable y credenciales, `/puestos/{id}/datos`, `/describe`, `/media`, `/paquetes` (copias), `/vistas/…/ejecutar`, funciones (`over`), Builds (entradas), los verbos `cargar`, `esbozar`, `ficha`, `muestra`, `materialize`, `invoke`, `ask` | `leer_en` sobre un árbol superpuesto; `puestos::datos_en` añade su propio «si no, `main`» |
| **todos(ref)** | `/assets` (índice, salud, refresco), `/datasets`, `/v1 … /tables`, `/colecciones`, recoger, retirar base, migrar | `del_arbol` / `todos_en` / `ore datasets --json` |
| **historia(ref, nombre)** y **en(ref, nombre, instante)** | `/historia`, `/promesas/historia`, `as_of` de `/media`, autor y fecha de `/paquetes/{n}/copias`, `fecha_de` (409 de `ejecutar`), `escrita_desde` (cancelar Build), `/assets/{commit}` | `git log`/`show` del fichero, cada uno a su manera |
| **escribir(ref, [cambios], esperado, autor, operación, idempotencia)** — una o varias tablas en una transacción | `/v1` create, commit y `transactions/commit`, `/confirmar`, `publicar-retenida`, commit de colección (`/media/…/commit`), la copia (`escribir_informe`), recoger (datasets y colecciones), `invoke` | `escribir_puntero_sin`, `escribir_informe`, `fs::write` a pelo; persiste `publicar` |
| **retirar(ref, nombre)** y **mover(ref, de → a)** | `DELETE /documentos/Dataset`, `DELETE /paquetes/{n}`, renombrar schema, `retirar_informes_de_nadie`, migrar | borrado y `rename` de ficheros |
| **reclamados()** — lo que nombra cualquier ref viva | recoger-huérfanas (copia, copiador, mantenimiento) | el shell de cada Job: `ls-tree main` + `diff merge-base..R` a un directorio |
| **cambios(rama)** frente a su base y a `main` | `/ramas/{r}/cambios`, proponer, la huella de una propuesta | `cambios::anexos` con tres árboles, `huellas` |
| **ramas:** crear, borrar, traer `main`, fusionar (con y sin alcance) | `/ramas`, `/propuestas/{n}/fusionar`, `poner_al_dia` | git (`derivar`, `fusionar_puntero`, `resolver_punteros`), y **la forja** en la fusión sin alcance |
| **versión(ref)** — la clave de las cachés | `assets_cache`, `ARBOLES`, `cambios_cache`, refresco | el commit de la rama y el de `main` |

## 2. ore-serve (`crates/ore-serve/src`)

### 2.1 La capa git (desaparece para los punteros)

| Sitio | | Qué |
|---|---|---|
| `git.rs:1685` `superponer` | R+W | la regla `manda` sobre un árbol: blobs de rama, base y `main`, escribe los de `main`, deja `.ore-al-dia.json` |
| `git.rs:1568/1595/1647` `clonar_rama_al_dia`, `ocultar_al_dia`, `deshacer_al_dia` | R+W | prepara la rama para escribir y, al terminar, hace propio lo heredado que cambió |
| `git.rs:619` `publicar` | W | `git add -A` + commit + push: **la persistencia de toda escritura** |
| `git.rs:1394/1416` `clave_para_leer`, `clave_de` | — | clave de caché = commit de rama + commit de `main` |
| `git.rs:543` `resolver_punteros` | R+W | punteros cambiados a los dos lados de un `traer` |
| `git.rs:782/864` `derivar`, `fusionar_puntero` | R+W | propuesta con alcance: promover, reconstruir o conflicto por puntero (mixto) |
| `git.rs:964` `huellas` | H | la huella de la propuesta incluye el diff de punteros |
| `propuestas.rs:1757` `fusionar` sin alcance | W | **la forja fusiona los punteros como texto**, sin `manda` |
| `propuestas.rs:832/863` `crear_rama`, `retirar_rama` | W | la rama copia o suelta todos los punteros |
| `propuestas.rs:2026` `reconstruir_en_main` | W (indirecto) | encola una copia que escribe en `main` |

### 2.2 Lecturas

| Sitio | | Qué | Ruta |
|---|---|---|---|
| `assets.rs:617/627` `indice_con` | R | todos (`del_arbol`, **solo `datasets/`**) + `de` | `GET /assets`, `/health` |
| `assets.rs:1326` `assets_al_dia` | R | el índice, con caché | `/assets`, salud |
| `assets.rs:1377` `assets_de_commit` | H | todos a un commit | `GET /assets/{commit}` |
| `assets.rs:1224/1249` `salud_del_activo`, `con_salud` | R | la salud por el índice | ejecutar vista, preview, `/puestos/{id}/datos`, invocar |
| `refresco.rs:~63` | R | el índice, para las edades | refresco (`--refrescar`), `GET /refresco` |
| `datasets.rs:40/68` `de_main`, `anotar_de` | R | `.ore-al-dia.json` y punteros de antes | `/datasets*`, `/assets` |
| `datasets.rs:94` | R | todos (`ore datasets --json`) | `GET /datasets` |
| `datasets.rs:104` | R | uno (`--ficha`) | `GET /datasets/{…}` |
| `datasets.rs:309/316/410/472` | R | colecciones (`ore collections …`) | `GET /colecciones[…]`, items, resolver |
| `datasets.rs:346/394` `describir` | R | uno (literal `"datasets"`) | `GET /describe/…` |
| `catalogo.rs:1380` `tablas` | R | todos con `metadata_location` | `GET /v1/…/tables` |
| `catalogo.rs:~833/1337/1363` `cargar`, `cargar_para_leer` | R | uno (`--cargar [--prestar]`) | `GET/HEAD /v1/…/tables/{t}`, credenciales; tras cada escritura `/v1` |
| `catalogo.rs:~973` | R | uno (`--esbozar`) | stage-create |
| `catalogo.rs:1227` `cargar_vista` | R? | `ore ask --sql --catalogo` | `GET /v1/…/views/{v}` (sin verificar) |
| `medios.rs:237/512` `fijar_en`, `fijar_colecciones` | R | ML y transacción de cada entrada (literal `"datasets"`) | `POST /builds`, preview y transform del puesto, funciones |
| `medios.rs:527/640` `media` | R | una colección | `GET/POST /media/…` |
| `copia.rs:1284` `copias_de` | R | una por copia declarada, **N por paquete** | `GET /paquetes`, tras inducir |
| `copia.rs:1309` `informe_de` | R | uno | `GET /paquetes/{n}/copias` |
| `documentos.rs:1384/1402` | R | uno (¿lo escribió una máquina?) | `DELETE /documentos/Dataset/…` |
| `cambios.rs:113/124` `anexos` | R/H | uno en tres árboles: base, rama **sin superponer**, `main` | `GET /ramas/{r}/cambios`, proponer |
| `preguntar.rs:~181/194` | R | uno | `POST /vistas/…/ejecutar` (409) |
| `preguntar.rs:66` | R | `ore ask --vista` | `POST /vistas/…/ejecutar` |
| `funciones.rs:581/606` `copia_de` | R | la copia de `over`, **solo `main`** | `POST /funciones/…/invocar` |
| `puestos.rs:6072/6156` `datos_de` | R | uno | por `datos_o_vista` |
| `puestos.rs:4308` `datos_en` | R | uno, **con su propio «si no, `main`» en 404/409** | `GET /puestos/{id}/datos/{v}`, `/sql`, preview por el motor, `/v1/layers/…` |
| `puestos.rs:~4962/~5050` | R | `ore ask --sql`, `ore datasets --cargar --prestar` | lo mismo |
| `preview.rs:~183` `del_lago` | R | `ore datasets --muestra` | `GET /preview/…` |
| `builds.rs:1416` `filas_de_las_entradas` | R | cada entrada | `POST /builds` (tamaño) |
| `arbol.rs:169/212` `indice_en`, `leer` | R | ficheros de punteros, crudos | `GET /arbol`, `GET /arbol/{ruta}` |

### 2.3 Escrituras

| Sitio | | Qué | Ruta | Nota |
|---|---|---|---|---|
| `datasets.rs:193` `confirmar_dataset` | W | uno, CAS `--esperado` | `POST /datasets/…/confirmar` | mixto; en `main` **sin 423** |
| `datasets.rs:137` `publicar_retenida` | W | uno | `POST /datasets/…/publicar-retenida` | sin 423 |
| `catalogo.rs:~992` createTable | W | uno (`--crear`) | `POST /v1/…/tables` | mixto |
| `catalogo.rs:~1056` updateTable | W | uno (`--commit`, CAS por requisitos, idempotencia) | `POST /v1/…/tables/{t}` | mixto |
| `catalogo.rs:~1133` commitTransaction | W | **varios en un commit** | `POST /v1/…/transactions/commit` | mixto |
| `escritas.rs:265–388` `cerrar_transaccion` | R+W | colección: CAS con ore-medios, `fs::write` (literal `"datasets"`) | `POST /media/…/transactions/{t}/commit` | mixto; sin 423 |
| `copia.rs:225/274/284` `retirar_paquete` | R+W | todos los de la base + `remove_dir_all` | `DELETE /paquetes/{n}` | mixto |
| `documentos.rs:1466` | W | el de su sitio y el de antes | `DELETE /documentos/Dataset/…` | mixto |
| `schemas.rs:125` → ore-cli `schemas.rs:575` | W | mueve `datasets/<p>/<s>/` | `POST /paquetes/{n}/schemas/{s}/renombrar` | mixto |
| `arbol.rs:52` `ruta_editable` | guarda | 422 en el editor (E1a·4) | `PUT/DELETE /arbol`, `POST /arbol/commit` | |

### 2.4 Historia desde git

| Sitio | Qué | Ruta | Nota |
|---|---|---|---|
| `historia.rs:22` | `git log -200` + `show` | `GET /historia/{ref}` | ignora `.ore-al-dia.json`; no ve la ruta de antes |
| `promesas.rs:~165` | `git log` + `show` | `GET /promesas/historia/{ref}` | igual |
| `medios.rs:277/396/411` | log + `cat-file`, **arranca en `main` si lo heredó** | `?as_of=` de `/media` | su propia `manda` |
| `copia.rs:1332` `commit_de` | `git log -1` | `GET /paquetes/{n}/copias` | |
| `preguntar.rs:217` (`git.rs:435` `fecha_de`) | `git log -1` | `POST /vistas/…/ejecutar` (409) | |
| `builds.rs:1933/2137` `escrita_desde` | `git log -1 %ct` | `POST /builds/{id}/cancel` | |
| `arbol.rs:732/792` | historia y versión de cualquier fichero | `GET /arbol/historia/…`, `/arbol/version/…` | crudo |

## 3. ore-cli (`crates/ore-cli/src`)

| Sitio | | Qué | Corre como |
|---|---|---|---|
| `al_dia.rs:47/106–140/169–228` | H+R+W | **la segunda copia de `superponer`** (`ore overlay`) | Job 48 en rama |
| `datasets.rs:237` `punteros` | R | todos + clasificar por el documento | librería (listar, ficha, muestra, recoger) |
| `datasets.rs:297` `Ajenos::de` | R | los de otras ramas desde `--reclaman` | Jobs 48, 53, copiador |
| `datasets.rs:352` `reclamados` | R | `datasets` + `copias` + `resultados` | recoger-huérfanas |
| `datasets.rs:399/454/516` listar, ficha, muestra | R | | vía ore-serve |
| `datasets.rs:618/673` `recoger` | R+W | reescribe ML tras expirar, `fs::write` | CronJob 53 |
| `datasets.rs:1498` `puntero_del_lago` | R | uno (base del CAS) | los 7 verbos de abajo |
| `datasets.rs:1533` `escribir_puntero_sin` | W | **primitiva de escritura 1** | |
| `datasets.rs:1978/2307/2401` `commit` | R+W | N tablas | `/v1` commit y transacciones (mixto) |
| `datasets.rs:2488/2715/3113` publicar-retenida, crear, confirmar | R+W | uno | vía ore-serve (mixto) |
| `datasets.rs:2926/3005` cargar, esbozar | R | uno | `/v1`, `/puestos/{id}/datos` |
| `datasets.rs:3052` retención | R+W | uno | solo CLI |
| `colecciones.rs:192/257` | R | una | `/colecciones` |
| `colecciones.rs:802/838` `recoger` | R+W | reescribe tras retención | CronJob 53 |
| `coleccion.rs:900`, `materializar.rs:269/310/428/626/978/1031/1126` | R | uno | Job 48, copiador |
| `materializar.rs:2375` `origen_del_lago`, `1668` `Puntero::hecho` | R | uno, **`<raiz>/datasets` fijo** | Job 48; ask e invoke |
| `materializar.rs:2116` `escribir_informe` | R+W | **primitiva de escritura 2** | Job 48, copiador |
| `materializar.rs:1788` `retirar_informes_de_nadie` | R+**borra** | los que ya nadie declara | Job 48, copiador |
| `invocar.rs:166/349/500/516` | R+W | entrada y `resultados/` | Job 49 |
| `preguntar.rs:162/319/621` | R | copias candidatas | `POST /vistas/…/ejecutar` |
| `schemas.rs:575/654` renombrar | R+mueve | | vía ore-serve (mixto) |
| `migrar.rs:405/485`, `migrar_v14.rs:343` | R+mueve | | solo CLI |

`ore-core`: `punteros.rs` (las piezas), `assets.rs:590/696` y `salud.rs:46` reciben punteros ya leídos.

## 4. Jobs y scripts de la malla

| Sitio | | Qué |
|---|---|---|
| `48-la-copia.yaml:212/221/499–507/514–541/478–495` | H+R+W | clon, `ore overlay`, «reclaman», `materialize --recoger`, `publicar` (rebase ×5) |
| `copiar-worker.sh:84/126–133/136–150/155/161–171` | H+R+W | lo mismo para el copiador; además **lee el puntero con `grep`/`sed`** |
| `cerrar-ejecucion.py:56–100` | R | abre `datasets/<b>/<s>/<n>.json` a pelo (Jobs 44, 48, 49, copiador) |
| `53-el-mantenimiento.yaml:125–164` | H+R+W | clon, «reclaman», `recoger`, push a `main` **sin reintento** |
| `49-la-invocacion.yaml:222–245` | R+W | `ore invoke`, push de `resultados/` sin reintento |
| `31-copias-de-la-forja.yaml:272/296` | — | la única copia de hoy de los punteros (con su historia) |
| `aprovisionar-inquilino.sh:1446` | — | traslada el árbol entero con `push --mirror` |

## 5. Fuera de Rust

- **SDK** (`puesto/python`, `jvm`, `node`, `spark`): ~30 sitios, **todos por HTTP**: `/puestos/{id}/datos`,
  `/v1` (loadTable, stage-create, commit, `transactions/commit`), `/media/…`, `/assets`, `/describe`,
  `/puestos/{id}/sql`. Ninguno ve git.
- **Consola** (`rubix-platform`): ~16, todos por HTTP: `/assets`, `/datasets/…`, `/historia/{ref}`,
  `/promesas[/historia]`, `/paquetes/{b}/copias`, `/preview`, `/paquetes/…/copia/rehacer`,
  `/datasets/…/esquema/aceptar`, el editor `/arbol` (cerrado a punteros desde E1a·4) y `/v1/layers`.
  `AssetDetail.tsx:383` pinta «Pointer = `datasets/<ns>_<n>.json`», la ruta de antes: ya hoy es falsa.
- **Pruebas que leen punteros de git:** contra la malla viva, `salud/vara.py:270/602`,
  `medida-lo-que-parece-roto.py`, `medida-migrar-dataset.py`, `la-fuente-al-dia.py` (+ `migrar-a-dataset.py`),
  `medida-w2-historial.py`; locales, `el-lago.sh`, `el-puesto.sh`, `la-copia-se-decide.sh`,
  `los-datos-en-una-rama.sh` y nueve más.

## 6. Lo que destapó

**De diseño (deciden ·5b):**
1. **`manda` está escrita cinco veces:** `superponer` (ore-serve), `al_dia.rs` (ore-cli), el «si no, `main`»
   de `datos_en`, `desde_donde` de medios, y el «reclaman» de tres scripts (por diff contra la base, no
   por `manda`). Con `Punteros` queda una: `vista(ref)` del catálogo.
2. **Tres ideas de qué carpetas son punteros:** `{datasets}` (índice y escritores), `{datasets, copias}`
   (la superposición), `{datasets, copias, resultados}` (reclamar). `copias/` no sale en `/assets`;
   `resultados/` se reclama pero no se superpone (§11.5 punto 15).
3. **La historia la reconstruye cada uno con `git log`,** y discrepan: solo medios sigue lo heredado de
   `main`; historia, promesas y Builds no ven la ruta de antes. Con `historia` del catálogo, una.
4. **Cachés con clave de commit** (`assets_cache`, `ARBOLES`, `cambios_cache`, refresco): sin commit no
   cambian. Necesitan **`versión(ref)`** del catálogo.
5. **La lógica está en ore-cli, no en ore-serve:** `/v1`, `/datasets`, `/colecciones`, preview y
   ejecutar vista corren `ore datasets|collections|ask` sobre un directorio. `Punteros` tiene que vivir
   en una crate que usen los dos.
6. **Ya hay un segundo almacén de punteros:** las tablas del sistema (`ore-store anexar/puntero`, en GCS).
   Fuera del alcance; se anota.

**Fallos de hoy, independientes del corte:**
- Los Jobs 53 y 49 empujan a `main` **sin traer ni reintentar**: un empuje concurrente los tumba; en 53,
  con instantáneas ya expiradas y el puntero viejo en git (sin verificar la consecuencia).
- **Posible:** el nombre en el lago del listado de una colección, cuando el puntero no trae `dataset`,
  se calcula distinto en ore-cli (`colecciones/b/default/n`) y en ore-medios (`colecciones/b/n`, que
  además nunca lee ese campo). Se ve en el código; la consecuencia, sin probar.
- **Posible:** `datasets.rs:237` decide «mantenido» o «escrito» mirando solo
  `packages/<ns>/datasets/<n>.yaml`, sin el schema: dos datasets de igual nombre en schemas distintos
  comparten clase. Sin probar.
- La fila «Pointer» de la consola apunta a la ruta de antes.
- Ya conocidos (§11.6): sin 423 en `/confirmar`, `publicar-retenida` y el commit de colección; la
  fusión sin alcance la hace la forja como texto.
