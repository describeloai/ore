# M0 · marimo en el puesto — lo medido (2026-10-14)

Notebooks en los repositorios `analytics-python` y `models-python`: absorber el runtime y la
interfaz de marimo (OSS) y poner nosotros el cómputo, la identidad, los datos y el gobierno, como
ya hacemos en Functions y Transforms. M0 mide antes de decidir. El estado del arte, con fuentes,
está en [`marimo-estado-del-arte.md`](marimo-estado-del-arte.md). La sonda se puede reproducir
desde [`pruebas-de-fuego/marimo/`](../../pruebas-de-fuego/marimo/): imagen `Dockerfile.sonda`, Job
`sonda-m0.yaml` y script `m0.py`. La salida en crudo de victor está en
`m0-victor-2026-10-14.txt`.

**Cómo se midió.** Se construyó una imagen `sonda-marimo:m0` = `puesto-python:1` + `marimo[sql]`
0.25.1. El Job tiene la forma de un puesto (rol de red `puesto`, cuenta `puesto`, cola Kueue
`cola`) y corrió en `t-victor`, en un nodo `sistema-spot`, sin internet y con `deny-all-ingress`.
Las celdas se movieron por **SSE + POST** (`server.transport = "sse"`), que es el transporte que
cabría por la correa.

## Lo medido

| | Medida | Qué decide |
|---|---|---|
| **a · instalar** | marimo 0.25.1 **resuelve con `provisto.txt` como restricción**: 0 conflictos, +21 paquetes (+25 con `[sql]`). | Cabe en la imagen sin tocar lo fijado. |
| | site-packages 544 → 784 MB (marimo solo) → 1 076 MB (`[sql]`: `polars` 2.0 + `sqlglot`). De los 160 MB de marimo, **113 MB son el servidor de Copilot** (`_lsp/copilot`, binarios de 3 sistemas operativos) y 27 MB el frontend. | Quitar Copilot y elegir `sqlglot` sin `polars` deja la imagen en +~150 MB. |
| | La imagen pasa de 325 a 481 MB comprimida y se descarga en **14 s** en un nodo con la base en caché. | — |
| **a · red** | Sin internet: pypi, github y jsdelivr no responden (timeouts de 5–20 s). `index.html` no carga nada externo y el frontend va en la rueda. | Funciona cerrado. |
| | Arranque del servidor: **1,4 s** con `--skip-update-check` y **4,5 s sin él**. La comprobación de versión contra una red que no responde cuesta **+3,1 s** en cada arranque. | `--skip-update-check` es obligatorio. |
| **memoria** | Servidor solo **90 MiB**; con kernel **166 MiB** (3 procesos: el kernel es un subproceso de unos 76 MiB, no los ~150 que cita marimo); con pandas/pyarrow/duckdb/polars y un DataFrame de 1 M **370 MiB**. | Entra en el puesto de hoy (768 MiB pedidos, 4 GiB de límite). |
| **sesión** | SSE abierto → `kernel-ready` en **1,3 s**. | — |
| **celda** | Celda trivial por SSE+POST: **p50 13,1 ms · p95 15,1** (n=40). Cadena reactiva de 3 celdas (cambiar `a` re-ejecuta `b` y `c`): **p50 17,4 ms · p95 22,7** (n=20). | El kernel no es el cuello de botella. |
| **relevo** | Pod → ore-serve (`/salud`): **p50 1,5 ms · p95 1,8**. Navegador (desde aquí) → `victor.ore.paladio.io` por el túnel de Cloudflare: **57 ms** con la conexión reutilizada y **219 ms** con TLS nuevo. | Por el relevo, una interacción costaría unos **75–130 ms**, y el grueso es el túnel público, no el pod. Falta medirlo con el relevo de verdad. |
| **datos** | `import ore` en el kernel → `/opt/ore/ore/__init__.py`. DuckDB `sum(range(1e8))` 0,40 s; DataFrame de 1 M como salida **108 ms y 1,6 KB** (la tabla es paginada: no viajan las filas); `mo.sql` group-by sobre 1 M **43 ms**; `import pandas, pyarrow, duckdb, polars` 0,8 s. | El SDK está a mano, pero **falta M0·d en un puesto real** (la sonda no es un puesto registrado: `over()`/`write()` no tienen identidad). |
| **e · guardar** | Guardar es un **`POST /api/kernel/save`** explícito (`cellIds`, `codes`, `names`, `configs`, `filename`). Escribe el `.py` en disco y **devuelve el código generado en la respuesta**. Se crea `__marimo__/` junto al cuaderno. El `.py` es Python puro con `@app.cell`; las salidas no van en el fichero. | Ese POST es el gancho: el relevo lo intercepta y lo convierte en commit de la persona. |
| **b · kernel sin web** | `python -m marimo._ipc.launch_kernel` (ZMQ en 127.0.0.1) existe y se declara API pública para `marimo-lsp`, pero los mensajes que mueve son tipos internos `marimo._*`. | Una UI propia sobre él sería frágil (ver el estado del arte §4). |
| **f · licencia** | Apache-2.0. Versión 0.x, que sale cada 1–3 semanas. CoreWeave es la dueña desde 2025-10. | Fijar versión y subirla por la imagen, como lo provisto. |

## Lo que sale, para el ADR

Lo que decido abajo es una propuesta, pendiente del visto bueno.

- **D1 · La interfaz: el frontend oficial de marimo en un iframe de la consola**, no una UI
  propia. Es lo que hace todo el sector (marimohub del propio equipo, Jupyter, el operador de
  Kubernetes, Quix, Open OnDemand): la identidad y la autorización van en el proxy, y el editor se
  usa tal cual. Una UI propia exigiría pintar salidas `marimo-*` sobre mensajes internos sin
  contrato.
- **D2 · La red: un túnel saliente desde el puesto, no entrada.** `deny-all-ingress` está vivo en
  cada inquilino y el agente ya hace de correa para el LSP (flujo de eventos + POST). Con el
  transporte SSE de marimo todo el tráfico del editor es HTTP plano: órdenes por POST y kernel →
  navegador por SSE. Así cabe en la correa generalizada y en el túnel de Cloudflare sin WebSocket.
  Lo que SSE deja fuera (LSP de marimo, terminal, RTC) ya lo tenemos (pyright) o no lo queremos
  todavía. **Riesgo:** SSE es *experimental* aguas arriba; la salida de emergencia es WebSocket por
  el mismo túnel.
- **D3 · Origen aislado.** El iframe se sirve desde un subdominio propio de la celda, no desde el
  origen de la consola ni de ore-serve: marimohub advierte que, en el mismo origen, un cuaderno
  malicioso puede programar el plano de control. Ojo con el comodín DNS (ver la memoria del
  incidente de iam).
- **D4 · Guardar = commit de la persona.** ore-serve intercepta `POST /api/kernel/save` y escribe
  el `.py` en la rama del puesto, firmado por quien abrió la sesión. El pod sigue sin escribir el
  árbol (W3.7 ①). El autoguardado de marimo (1 s) no puede ser un commit por segundo: o borrador en
  ore-serve y commit al guardar a mano, o agrupar por inactividad (lo de Quix: 5 s). La dirección
  contraria (el árbol cambia) entra con `--watch`. `__marimo__/` no entra en el árbol.
- **D5 · La imagen: una variante para las clases de cuadernos** (`analytics`, `models`) con marimo
  fijado, **sin Copilot** y con `sqlglot` sin `polars` obligatorio. Functions y Transforms no pagan
  esos MB. La clase ya elige el entorno (0036 ⑤).
- **D6 · Datos de verdad, no adaptadores.** ore-serve **es** un catálogo Iceberg REST, y marimo
  detecta catálogos PyIceberg (`PYICEBERG_CATALOG__ore__*`) en su panel *Data sources*: el lago
  aparece en el editor sin escribir nada. El DuckDB de `ore.sql()`, expuesto como variable, es
  motor de `mo.sql`. El gobierno sigue donde está (el techo de la clase, el conducto, 403 en el
  servidor).
- **D7 · Cerrado por defecto.** `--skip-update-check` (medido: +3,1 s sin él);
  `completion.copilot=false`; IA apagada o con `custom_providers.base_url` → nuestro Model Serving
  (0027), para que el LLM sea el de la plataforma y no salga del inquilino; el relevo niega
  `/api/packages/*`, `/api/kernel/install_missing_packages`, `/terminal` y `/mcp` (las
  dependencias van por el `pyproject.toml` del repositorio y la capa, como siempre).
- **D8 · Reactividad y escrituras** (`models-python` escribe): `on_cell_change = "lazy"` por
  defecto en esa clase, `mo.ui.run_button` en la semilla para las celdas que escriben, y la clave
  de operación de `write()` como red. `auto_instantiate` está en false en 0.25.1: abrir un
  cuaderno no ejecuta nada.

## M0·d · medido el mismo día en un puesto real

En el ADR [0064](../decisions/0064-notebooks.md) §3 (sonda `pruebas-de-fuego/marimo/m0d.py`,
salidas `m0d-victor-2026-10-14-{a,b}.txt`). Lo que queda por medir está en el ADR.
