# 0064 · Notebooks — marimo en los repositorios Analytics y Models

**Estado:** en estudio (2026-10-14). Se construye en este orden:

1. **Lo que hay** (escrito): lo que ORE ya pone para un notebook, sin marimo.
2. **Lo que es marimo** (escrito, con fuentes): cómo está hecho y cómo lo integra el sector.
3. **Lo medido** (M0, M0·d, M1·a y M1·b, **medidos** el 2026-10-14 en victor; M0·d en un puesto
   real y M1·b por el camino público). **Las medidas del estudio están cerradas.**
4. **La pregunta del editor** (escrita): el editor oficial frente a uno nuestro. **La persona elige
   probar el oficial** (2026-10-14), y M1·a lo hace funcionar entero por un relevo.
5. **El producto** (pendiente): la visión y la decisión, ahora que las medidas están cerradas.
   Hasta entonces nada de lo que sigue es una decisión.

Investigación: [`marimo-estado-del-arte.md`](../investigacion/marimo-estado-del-arte.md).
Medidas y sondas: [`marimo-m0.md`](../investigacion/marimo-m0.md) y
[`pruebas-de-fuego/marimo/`](../../pruebas-de-fuego/marimo/).

## Contexto

En Functions (0050) y Transforms (0055) ORE es dueño de cada punto del pipeline: el código vive en
el árbol, el entorno lo declara el repositorio (la capa), corre en el puesto con la identidad de
su pod, lee y escribe por el catálogo con el gobierno en el servidor, y lo que se publica pasa por
ramas y propuestas. Para `analytics-python` y `models-python` la idea es la misma con una
diferencia: **el runtime y la interfaz del notebook no se construyen, se absorben de marimo**, que
es OSS (Apache-2.0) y encaja bien. El cómputo, la identidad, los datos, el gobierno y el ciclo de
vida del fichero siguen siendo nuestros.

## 1 · Lo que hay

- **El puesto** (0031): un Job por persona y repositorio (`puesto-python:1`, Python 3.14), sin
  internet (`salida-del-puesto`) y **sin entrada** (`deny-all-ingress` en cada inquilino). El
  agente pide trabajo a ore-serve por polling largo y la consola nunca habla con el pod. El LSP ya
  viaja así: flujo de eventos + POST (la correa, 0037).
- **La identidad del puesto** (R1): el token de cuenta de servicio del pod, ligado a ese pod, en
  `/var/run/ore/pod/token`, que se presenta como `x-ore-pod`. El agente se lo pone al SDK **dentro
  de su propio proceso** (`ore.session._cabeceras` / `_proveedor`).
- **La clase** (0036 ⑤): `analytics-python` no escribe (techo en el servidor, 403) y
  `models-python` sí. La clase elige el entorno.
- **Un notebook casero**: `.ipynb` + `CellListViva` en la consola (~260 líneas) sobre el kernel del
  agente (espacio de nombres persistente, salida tipada, `display()`, matplotlib). Sin reactividad,
  sin widgets, sin formato propio en git.
- **Escribir el árbol desde un puesto está prohibido** (W3.7 ①): `/arbol` es 403 para el agente.
  Un fichero sólo entra por la persona.

## 2 · Lo que es marimo (resumen; el detalle y las fuentes, en la investigación)

- 0.25.1 (2026-10-01), Apache-2.0, 0.x con versiones cada 1–3 semanas, de CoreWeave desde 2025-10.
- **Formato**: un `.py` puro, una función `@app.cell` por celda, sin salidas dentro; diffs limpios.
  Cabe en el árbol, las ramas, las propuestas, pyright y la capa tal cual.
- **Arquitectura**: un servidor Starlette/uvicorn sirve un frontend React empaquetado en la rueda.
  En `edit`, un kernel por sesión en un subproceso. Órdenes por POST (`/api/kernel/*`) y mensajes
  del kernel por WebSocket, o por **SSE** (`server.transport = "sse"`, experimental).
- **El kernel sin el servidor web** existe (`python -m marimo._ipc.launch_kernel`, ZMQ, lo usa la
  extensión de VS Code), pero sobre tipos internos `marimo._*` sin contrato.
- **Cómo lo integra el sector**: nadie reimplementa la UI. **marimohub** (del propio equipo,
  2026-10-02: OIDC, Kubernetes con un pod por kernel, GCS) carga el editor oficial en un iframe
  desde un **subdominio aislado**. Lo mismo hacen Jupyter (server proxy), el operador de
  Kubernetes, Quix y Open OnDemand.
- **Datos**: `mo.sql` sobre DuckDB, SQLAlchemy, Ibis, DB-API, ADBC… y un panel *Data sources* que
  descubre catálogos **PyIceberg**, incluido el REST.

## 3 · Lo medido

**M0** (2026-10-14): imagen sonda `puesto-python:1` + marimo, Job con la forma de un puesto en
t-victor, sin internet.

| | Medida |
|---|---|
| Instalar | Resuelve con `provisto.txt` como restricción: 0 conflictos, +21 paquetes. site-packages +240 MB, de los que **113 MB son el servidor de Copilot**, inútil sin internet. `[sql]` añade `polars` + `sqlglot` (+290 MB). Imagen comprimida 325 → 481 MB; descarga en 14 s. |
| Sin internet | Funciona: el frontend va en la rueda. Arranque 1,4 s con `--skip-update-check` y **4,5 s sin él** (+3,1 s esperando a marimo.io). |
| Memoria | Servidor 90 MiB; + kernel 166 MiB (el kernel es un subproceso de ~76 MiB). |
| Kernel | Celda p50 **13 ms** (p95 15); cadena reactiva de 3 celdas p50 **17 ms**; `kernel-ready` 1,3 s tras abrir el SSE. |
| Red | Pod → ore-serve p50 **1,5 ms**; navegador (desde la máquina de desarrollo) → victor por el túnel de Cloudflare **57 ms** con la conexión reutilizada (219 ms con TLS nuevo). Estimado por el relevo: 75–130 ms por interacción. Falta medirlo de verdad (M1). |
| Guardar | Un **`POST /api/kernel/save`** explícito; la respuesta trae el `.py` generado. Crea `__marimo__/` junto al cuaderno. |

**M0·d** (2026-10-14): el SDK dentro de un kernel de marimo **en un puesto real** de victor
(`models-python`, repositorio `packages/test_project/modelsv1`). Marimo, sqlglot y pyiceberg se
instalaron en el sitio de usuario (`pip --user`, 305 MB, 18 s, sin tocar la imagen), y el servidor
corrió como proceso aparte del agente, igual que correría de verdad.

| | Medida | Qué dice |
|---|---|---|
| Identidad | Una celda que llama a ore-serve sin más: **401** («esta ruta necesita un sujeto»). Con un proveedor que lee `/var/run/ore/pod/token` y lo manda como `x-ore-pod`: **200** (21–30 ms). | El kernel **no hereda** la identidad: el agente la pone en su propio proceso. El token del pod basta, sin la credencial del agente. **Hace falta que el SDK se configure solo** fuera del agente. |
| `over()` | 100 000 filas × 7 columnas: **4,1 s**. En el editor viaja una tabla paginada de **21,6 KB**. | — |
| `sql()` | `count(*)` sobre el mismo dataset: **2,1 s**. | — |
| ore-serve como catálogo | `pyiceberg.RestCatalog(uri=ORE_SERVE, header.x-ore-pod, X-Iceberg-Access-Delegation: vended-credentials)`: lista namespaces, carga la tabla y escanea 1 000 filas con la credencial prestada en **2,8 s**. **Marimo descubre `cat` solo** y lo enseña en su panel como fuente `iceberg`. | El lago aparece en el editor sin adaptador. Pide `pyiceberg` (+pydantic), que hoy no está en lo provisto. |
| `write()` reactivo | Cambiar `n` al **mismo** valor re-ejecuta la celda que escribe y devuelve `repeated: true` **con el mismo snapshot**; otro valor da un snapshot nuevo. Cada escritura tarda 3,3–5,8 s. | La clave de operación protege de la re-ejecución reactiva… |
| …y un fallo | …pero con `overwrite`, **volver a un contenido que la tabla tuvo antes (A → B → A) devuelve `repeated: true` y no escribe**: la tabla se queda en B sin avisar. Se reproduce fuera de marimo (5 escrituras, 3 perdidas). La clave se coteja con **toda la ancestría** (`ore-cli/src/datasets.rs`). | Hallazgo colateral grave, ver abajo. Con un slider que va y vuelve sería lo normal. |
| Memoria | Servidor + kernel con el DataFrame de 100 000 filas: **506 MiB**; al terminar (pyiceberg, pandas, escrituras): **730 MiB**. El puesto pide 768 MiB y tiene 4 GiB de límite, y comparte el pod con el agente y pyright. | La petición de memoria de una clase de notebooks tiene que subir, o se desaloja al primer DataFrame. |
| TTL | El agente cierra el puesto tras 1 800 s sin «celdas ni editor». Lo que pasa en marimo no cuenta. | La actividad del notebook tiene que contar, como la del editor (0050 P4). |
| Semántica | Un nombre que empieza por `_` es **local a su celda** (me rompió la primera pasada). | Lo tienen que decir las semillas y la guía. |

**M1·a** (2026-10-14): **el editor oficial a través de un relevo, sin entrada al pod.** Laboratorio
desechable en t-victor ([`pruebas-de-fuego/marimo/m1/`](../../pruebas-de-fuego/marimo/m1/)):
- una sonda con la forma de un puesto corre marimo (`--base-url /m`, transporte SSE) y un
  **túnel que sale del pod** (`tunel.py`: un WebSocket hacia fuera, una petición por hilo);
- un relevo (`relevo.py`, ~170 líneas, el papel de ore-serve) recibe al navegador y responde a
  trozos tal como llega, niega lo que no sale del puesto y anota cada guardado;
- dos NetworkPolicies temporales abren sólo sonda → relevo:8080, y el navegador llega al relevo por
  `kubectl port-forward`.

Todo se borró al acabar.

| | Medida | Qué dice |
|---|---|---|
| Funciona | En el navegador integrado: la portada, el cuaderno, «Run all», el slider (widget vivo), la tabla interactiva (histogramas, Columns, Visualize, Explore), la celda SQL con su motor DuckDB y el markdown reactivo. Arrastrar el slider recalcula la tabla. El pod sigue sin internet y nadie entra en él. | **El diseño es viable**: el editor oficial entero funciona por SSE detrás de un prefijo y de un relevo al que el pod llama. |
| Lo negado | El frontend pide `/api/packages/sandbox` al abrir; el relevo contesta 403 y el editor sigue. | El relevo de verdad debe contestar algo inocuo ahí, no un error. |
| Latencia | Celda trivial / cadena reactiva de 3 celdas (p50): **directo en el pod 12,9 / 16,8 ms**; **por relevo + túnel dentro del clúster 15,3 / 20,1 ms**; desde la máquina de desarrollo por `port-forward` 148 / 152 ms. | **El relevo y el túnel añaden ~2,5–3 ms.** Lo de fuera es el `port-forward` (pasa por el API server): por el túnel de Cloudflare medimos 57 ms en M0, así que de punta a punta se esperan **~75 ms** (se confirma en M1·b). |
| Primera carga | Un cuaderno pide **251 ficheros, 14 MiB**, **sin comprimir** (marimo ignora `Accept-Encoding`) y **sin `Cache-Control`** (sólo `ETag`). Todo atraviesa el túnel hasta el pod (p50 9,5 ms por fichero dentro del clúster). Los más grandes: `loro_wasm` 3,1 MB, `node-sql-parser` 2,4 MB, `index` 1,3 MB. gzip deja el `_static` entero en un tercio (25 → 8 MiB). | **Los ficheros del frontend no tienen que pasar por el pod.** Son idénticos para todos los puestos de una versión y llevan hash en el nombre: se pueden servir desde fuera (ore-serve o la caché de Cloudflare), comprimidos e inmutables. Al pod sólo deberían llegar el HTML, `/api` y `/sse`. |
| Guardar | En 10 s escribiendo en cinco tandas hubo **3 POST de guardado** (uno por pausa de más de 1 s), y cada uno lleva **el cuaderno entero** (~1 KB). El fichero del pod refleja lo escrito. | Un commit por guardado no puede ser. El relevo tiene que **acumular** (borrador) y hacer el commit al agrupar o al guardar a mano. El cuerpo ya trae todo lo necesario para escribir el fichero sin pedírselo al pod. |
| Reconexión | Recargar la página **conserva el kernel y sus salidas**. Pero si se recarga antes de que el relevo note que el flujo SSE viejo se cerró (tardó hasta 15 s), la pestaña nueva entra como **lectora** («connected as a reader · Take over»). | El relevo de verdad tiene que propagar la desconexión del navegador **al instante**, no al siguiente latido. |
| Sesiones | Marimo abre **una sesión (un kernel) por fichero**, y una segunda conexión al mismo fichero no es dueña (403 en sus POST). En `edit` las sesiones **no caducan** al desconectarse: las cuatro de las medidas siguieron vivas, a 75 MiB cada una vacía. El cuaderno con 138 000 filas: 276 MiB. El servidor: 146. El túnel: 29. | Hay que fijar `--session-ttl` (o cerrarlas desde el relevo) y contar un kernel por cuaderno abierto en el presupuesto de memoria del puesto. |
| Service worker | El navegador integrado no registró `public-files-sw.js`, aunque el relevo lo sirve bien (200). | Sin verificar en un navegador normal. Sólo sirve los ficheros de `public/`. |

**M1·b** (2026-10-14): **el mismo relevo por el camino público de verdad**: navegador → Cloudflare
→ túnel `ore-malla` → Envoy (`puerta-tunel`) → relevo → túnel saliente → marimo.

**Sin tocar DNS ni Cloudflare**: un `HTTPRoute` con prefijo `/m1-lab` en el nombre de siempre
(`victor.ore.paladio.io`); Envoy da el prefijo más largo y ore-serve siguió en `/`, con `/salud` 200
antes, durante y después.

El editor quedó en internet, así que se cerró por tres lados:
- un token con cookie (en un Secret, nunca en el repo, borrado al acabar);
- una sonda **sin credenciales** (cuenta `default` sin token montado ni Workload Identity) y sin
  internet;
- políticas **nuevas y aditivas**, incluida la salida de Envoy hacia el relevo; no se editó ninguna
  de Flux.

Manifiestos en [`m1b.yaml`](../../pruebas-de-fuego/marimo/m1/m1b.yaml), y el corte en
[`corte.py`](../../pruebas-de-fuego/marimo/m1/corte.py). Todo se borró al acabar.

| | Medida | Qué dice |
|---|---|---|
| Latencia | Por el camino público con la conexión reutilizada (como un navegador): **celda p50 78–80 ms (p95 ~87)**, **cadena de 3 celdas p50 ~85 ms**. En el pod, sin relevo: 13,9 / 17,1. Con una TLS nueva por petición sale ~200 ms: es el apretón de manos. | Lo estimado en M1·a (~75 ms). La reactividad se nota inmediata. |
| SSE por Cloudflare | `kernel-ready` en **1,6 s** por el camino público: Cloudflare **no retiene** el flujo. | El transporte SSE vale tal cual detrás de Cloudflare. |
| El corte de la puerta | Un SSE quieto por el camino público **muere a los 301 s sin `close`**: es el `requestTimeout: 300s` de `plazo-de-la-puerta`. Los latidos de marimo (cada **20 s**) no lo evitan. | Hay que tolerarlo, o sacar la ruta de los cuadernos de ese techo con su propia política. |
| …y el frontend | Con el corte simulado en el relevo a los 60 s: el frontend **reconecta solo** (abre un SSE nuevo) y sigue siendo dueño. Al mover el slider después, **111 507 filas recalculadas en 107 ms**. | **El corte se lleva sin que la persona lo note**: una reconexión cada 5 minutos, que reenvía el estado (19–138 KB medidos). |
| Recargar | Con el relevo preguntando cada 0,5 s si el navegador se fue: **recargar entra como dueño y conserva el estado**. Pero **dos recargas en menos de 1 s siguen entrando como lectora** (es una carrera). | ore-serve sabe que es **la misma persona** y puede pedir el *takeover* (`/api/kernel/takeover`) por ella. |
| Ficheros del frontend | Cloudflare los **comprime solo en brotli** (`index` 1,35 MB → 399 KB, `loro_wasm` 3,1 MB → 957 KB: ×3,4). Con `Cache-Control: public, max-age=31536000, immutable` (que pone el relevo; marimo no lo pone) **cachea los `.js`** (MISS → HIT: el segundo no llega al pod). El **`.wasm` sale `DYNAMIC`**: Cloudflare no lo cachea por defecto. | La clave de la caché es la URL, y la URL cuelga del `--base-url` de marimo. **Un prefijo por puesto duplicaría la caché por puesto**: los ficheros deben tener una ruta común por versión de marimo. El `.wasm` necesita una regla de caché o servirse desde otro sitio. |
| Clientes que no son navegadores | Cloudflare niega el user-agent de `urllib` con el **error 1010** (Browser Integrity Check). | No afecta al navegador. Sí a la CLI y al SDK si un día hablan con los cuadernos por la puerta. |
| El túnel | Al reiniciar el relevo, **el túnel del pod se reconecta solo** en ~1 s (cierre 1012 → abierto). | El relevo se puede desplegar sin tirar las sesiones abiertas (perdiendo sólo lo que estaba en vuelo). |

## 4 · La pregunta del editor: el oficial o uno nuestro

**Lo que pesa más es el hecho medido de que las salidas de marimo son HTML con elementos
`marimo-*`.** Una tabla, un slider o un gráfico no son datos que una UI ajena pinte a su manera:
son componentes web del frontend de marimo que hablan con el kernel (`/api/kernel/set_ui_element_value`,
`/function_call`). Un editor nuestro no sólo tendría que editar celdas: tendría que **reimplementar
cada componente y su protocolo**, que es interno y cambia cada 1–3 semanas.

**Lo que da el editor oficial:**

- **Todo el producto desde el primer día**: el grafo reactivo visible (celdas *stale*, dependencias,
  minimapa), más de 40 widgets `mo.ui` y anywidget, el visor de DataFrames (paginación, filtros,
  explorador de columnas, constructor de gráficos), celdas SQL con el panel *Data sources* (que ya
  ve nuestro catálogo), layouts de app (grid, slides), markdown/LaTeX/mermaid, el explorador de
  variables, el depurador y el scratchpad.
- **Las mejoras gratis**: subir marimo es mover una versión fijada en la imagen, con una prueba de
  contrato en el CI. Un editor propio persigue cada versión.
- **El ecosistema tal cual**: la documentación, los tutoriales, los notebooks de la comunidad y los
  agentes (`marimo pair`) funcionan igual. Quien venga de marimo o molab está en casa.
- **El mismo componente para apps** (`marimo run`) cuando toque publicarlas.
- **Nuestro trabajo es el marco, no la UI**: el relevo, la identidad, guardar → commit y el
  cierre (lo que no puede salir ni instalarse). Es donde está nuestro valor y lo que nadie hará
  por nosotros.

**Lo que cuesta el editor oficial:**

- **Dos editores en el producto**: Monaco + pyright para `.py`/`.sql` y CodeMirror dentro del
  iframe para los notebooks. Su explorador de ficheros, su panel de paquetes y su terminal duplican
  (o contradicen) los nuestros y hay que apagarlos o negarlos en el relevo.
- **La apariencia no es la de la consola**: se puede tematizar con CSS (`custom_css`, modo oscuro),
  pero no queda idéntica.
- **Integración en el borde, no dentro**: marimo no expone una API de incrustación (no hay contrato
  `postMessage`). Build, Preview, propuestas, linaje y comentarios van **alrededor** del iframe.
- **JS de terceros en el navegador** con acceso al kernel: obliga al origen aislado y a fijar y
  revisar cada versión (cadena de suministro).
- **APIs no contractuales que interceptamos** (`/api/kernel/save`, `/sse` experimental): pueden
  cambiar. Se mitiga fijando versión y con pruebas de contrato.
- **Gobierno ajeno**: 0.x y de CoreWeave. Apache-2.0 permite bifurcar si cambiara de rumbo.

**Lo que daría un editor nuestro:**

- Una sola experiencia: Monaco, nuestro LSP, nuestras tablas y el tema Carbon.
- Integración profunda: Build/Preview por celda, linaje, propuestas y comentarios en línea.
- El protocolo de red, nuestro (sin depender del SSE experimental), y sin JS ajeno en el navegador.

**Lo que costaría un editor nuestro:**

- Reimplementar la edición de notebooks, el estado reactivo, **cada renderizador de salidas y cada
  widget con su protocolo**, el visor de DataFrames, la UX de SQL, el panel de datos y los
  layouts. Partimos de las ~260 líneas del notebook casero.
- Sobre mensajes **internos** de `marimo._runtime` y `marimo._messaging`. La extensión de VS Code
  aguanta porque la mantiene el mismo equipo que cambia esos mensajes.
- Meses hasta la paridad y una persecución perpetua.

**El camino intermedio que hay que tener presente**: el editor oficial ahora, con **nuestro marco
alrededor** (barra propia fuera del iframe, guardar → commit, Libraries, Build). Y, para *apps* y
vistas propias más adelante, **marimo-studio** (2026-10-01), que es la vía oficial para un frontend
propio sobre la reactividad de marimo (licencia sin verificar).

## Hallazgos colaterales (fuera de este ADR, con tarea propia)

- **`write()` pierde escrituras A → B → A** (arriba): la clave de operación se coteja con la
  ancestría en vez de con la base de la escritura. Afecta a toda sesión, no sólo a marimo.
- **`bq.ventas.ore_e2e_sintetica` está roto en victor**: su lista de manifiestos
  (`metadata/snap-2127061337434120255-…avro`) da 404 en el bucket, y el índice sigue diciendo
  2 000 000 filas. `over()` y `sql()` fallan igual dentro y fuera de marimo.

## Lo que falta medir antes de decidir

Lo que el estudio deja abierto se mide al construir, porque necesita la pieza de verdad:

- **El origen aislado** (un nombre propio para el iframe): M1·b lo evitó a propósito para no tocar
  el DNS. Su autenticación (cookie del origen aislado frente a la sesión de la consola) se
  prueba con el relevo de verdad.
- **El presupuesto de memoria** de un puesto de notebooks con el agente y pyright de verdad.
  M0·d y M1·a dan las piezas: servidor ~146 MiB, ~75 MiB por kernel vacío, los datos aparte.

## 5 · El producto

Pendiente. Se escribe al cerrar las medidas, con la visión de producto completa.
