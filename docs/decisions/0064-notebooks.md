# 0064 · Notebooks — marimo en los repositorios Analytics y Models

**Estado:** en estudio (2026-10-14). Se construye en este orden:

1. **Lo que hay** (escrito): lo que ORE ya pone para un notebook, sin marimo.
2. **Lo que es marimo** (escrito, con fuentes): cómo está hecho y cómo lo integra el sector.
3. **Lo medido** (M0 y M0·d, **medidos** el 2026-10-14 en victor, el último en un puesto real).
4. **La pregunta del editor** (escrita): el editor oficial frente a uno nuestro.
5. **El producto** (pendiente): la visión y la decisión se escriben **al cerrar las medidas**
   (M1, el relevo de verdad). Hasta entonces nada de lo que sigue es una decisión.

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

- **M1 · el relevo de verdad**: navegador → túnel → ore-serve → correa → servidor de marimo, con
  SSE, con el iframe y desde el subdominio aislado. Latencia por interacción, reconexión, y
  cuántas sesiones aguanta ore-serve.
- **Guardar → commit**: interceptar `POST /api/kernel/save` y el ritmo del autoguardado (1 s) frente
  a la forja (un commit no puede ser por segundo).
- **El presupuesto de memoria** de un puesto de notebooks (agente + pyright + servidor + kernel +
  datos) y la petición que le toca a la clase.

## 5 · El producto

Pendiente. Se escribe al cerrar las medidas, con la visión de producto completa.
