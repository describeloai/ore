# marimo: estado del arte (octubre de 2026), visto desde ORE

Investigación hecha el 2026-10-14 con fuentes primarias siempre que se pudo: docs.marimo.io, el repositorio marimo-team en GitHub (código en `main`), PyPI y el blog de marimo.io. Lo marcado **no verificado** no lo pude confirmar en una fuente primaria. Las rutas de código citadas son de `main` en la fecha de consulta, no de una etiqueta concreta.

---

## 0. Resumen para decidir

- **Licencia y madurez.** marimo usa Apache-2.0. La última versión es la **0.25.1 (2026-10-01)**. Sigue en 0.x y publica una versión cada 1–3 semanas. El soporte de **Python 3.14** está declarado en los clasificadores. CoreWeave compró la empresa (anunciado el 2025-10-30) y se comprometió a mantener el proyecto abierto.
- **Arquitectura.** Hay un servidor Starlette/uvicorn y un frontend React servido por el propio paquete; los recursos van dentro de la wheel de 50,9 MB. La comunicación es HTTP (`/api/...`, especificada en OpenAPI 3.1 dentro del repositorio) más un WebSocket por sesión. En **edit**, el kernel corre en un **subproceso** por sesión; en **run**, en un **hilo** por sesión. Hay además "kernels IPC" en subproceso que se hablan por **ZeroMQ** sobre 127.0.0.1.
- **No existe un protocolo de kernel público y estable** para una UI propia. Lo más cercano es `python -m marimo._ipc.launch_kernel`: el código lo declara "API pública usada por consumidores externos como marimo-lsp", pero los mensajes que transporta son tipos internos (`marimo._runtime.commands`, `marimo._messaging`). La extensión de VS Code usa justo eso y renderiza con el frontend de marimo empaquetado. La API de agentes (`marimo._code_mode`) se declara expresamente "internal, no versioning guarantees".
- **El patrón que usa el propio equipo** para incrustar marimo en una plataforma con identidad, cómputo y almacenamiento propios es **marimohub** (Apache-2.0, anunciado el 2026-10-02). Usa el editor de marimo **tal cual, en un iframe**, apuntando a un *sandbox* por sesión. El acceso va por **subdominio aislado** (por defecto) o por **proxy autenticado en el mismo origen** (`/proxy/<token>/`, que su documentación desaconseja salvo en entornos de confianza). Es el modelo de referencia más cercano a ORE.
- **Guardado.** El servidor escribe el `.py` en su disco local (autoguardado al segundo por defecto). No hay *hook* ni almacenamiento enchufable para el propio cuaderno. Quien quiera *commit* en otro lado (marimohub, Quix) **recoge el fichero desde fuera**.
- **Iceberg REST.** El panel *Data sources* detecta catálogos PyIceberg, incluido `RestCatalog`, a partir de las variables `PYICEBERG_CATALOG__<nombre>__*` o de `.pyiceberg.yaml`. Los catálogos **no sirven como motor de SQL**. `mo.sql(..., engine=conn)` acepta DuckDB, SQLAlchemy, Ibis, ClickHouse, chDB, DB-API 2.0 y ADBC.
- **Lo que ORE tendría que resolver sí o sí:**
  1. El pod no acepta conexiones entrantes, así que hace falta un túnel HTTP+WebSocket a través de ore-serve, o la variante SSE experimental, que no cubre LSP, terminal ni RTC.
  2. Recoger los guardados y convertirlos en *commits* desde el plano de control.
  3. Desactivar o redirigir lo que intenta salir a Internet: comprobación de versiones, IA e instalación de paquetes.

---

## 1. Licencia, versión, cadencia, Python, tamaño y dependencias

| Punto | Dato | Fuente |
|---|---|---|
| Licencia | Apache-2.0 (`license_expression`) | PyPI JSON, repositorio |
| Versión actual | **0.25.1**, publicada el **2026-10-01** | pypi.org/project/marimo |
| Cadencia reciente | 0.23.12 (1 jul), 0.23.13 (8 jul), 0.23.14 (10 jul), 0.23.15 (23 jul), 0.23.16 (31 jul), 0.24.0 (17 ago), 0.24.1 (10 sep), 0.24.2 (11 sep), 0.25.0 (23 sep), 0.25.1 (1 oct). Sale una versión cada 1–3 semanas y las *minor* cada 4–6 semanas. Aún no hay 1.0. | github.com/marimo-team/marimo/releases |
| Python | `requires_python >=3.10`. Clasificadores 3.10–**3.14**. pyzmq solo se exige con `python_version < "3.15"`. Que funcione en 3.14 *free-threaded* (3.14t): **no verificado**. | PyPI JSON |
| Tamaño | La wheel `marimo-0.25.1-py3-none-any.whl` pesa **50,9 MB** e incluye el frontend compilado. El sdist pesa 50,2 MB. | PyPI |
| Dependencias núcleo | click, jedi, markdown, pymdown-extensions, pygments, tomlkit, pyyaml, **uvicorn**, **starlette**, python-multipart, **websockets**, **loro** (CRDT), docutils, psutil, itsdangerous, **narwhals**, packaging, **msgspec**, **pyzmq** (>=27.1) y typing-extensions (<3.12) | PyPI JSON |
| Extras | `sql` (duckdb, polars[pyarrow], sqlglot), `sandbox` (uv>=0.9.21), `lsp` (python-lsp-server, python-lsp-ruff), `mcp` (mcp>=2), `otel` (OpenTelemetry 1.28), `recommended` (sql+sandbox, marimo-lens, altair, pydantic-ai-slim[openai], ruff, nbformat…) | PyPI JSON |
| Popularidad | ~23,2k estrellas y 1,3k *forks* | GitHub |
| Propiedad | CoreWeave anunció la compra de Marimo Inc. el **2025-10-30**. Se compromete a mantenerlo abierto "bajo licencia permisiva" y a integrarlo con W&B. molab corre ya sobre CoreWeave. | coreweave.com/news, marimo.io/blog/reintroducing-molab |

Fuentes: https://pypi.org/pypi/marimo/json · https://pypi.org/project/marimo/ · https://github.com/marimo-team/marimo/releases · https://investors.coreweave.com/news/news-details/2025/CoreWeave-Acquires-Marimo-to-Unify-the-Generative-AI-Developer-Workflow/default.aspx

---

## 2. Arquitectura y opciones de `marimo edit` útiles detrás de un proxy

### 2.1 Servidor y protocolo
- El servidor ASGI es **Starlette + uvicorn** (ambos son dependencias núcleo). Los *endpoints* están en `marimo/_server/api/endpoints/`: ai, assets, cache, config, datasources, document, editing, execution, export, file_explorer, files, health, home, login, **lsp**, packages, secrets, sql, storage, **terminal** y ws.
- **La API HTTP está especificada** en `packages/openapi/api.yaml` (OpenAPI 3.1.0, 92 rutas). Ejemplos: `/api/kernel/run`, `/instantiate`, `/set_ui_element_value`, `/save`, `/interrupt`, `/restart_session`, `/takeover`, `/scratchpad/run`, `/api/datasources/*`, `/api/packages/*`, `/api/storage/*`, `/api/ai/*`, `/auth/login`, `/auth/token`, `/@file/{…}`. El mismo fichero define las notificaciones del kernel hacia el navegador (`op`): `cell-op`, `kernel-ready`, `completed-run`, `interrupted`, `datasets`, `data-source-connections`, etc. El paquete npm que la genera, `@marimo-team/marimo-api`, es **`private: true`**: no se publica y no tiene garantía de estabilidad.
- Del kernel al navegador los mensajes van por **WebSocket**. Hay un transporte alternativo **SSE experimental**: `MARIMO_SERVER_TRANSPORT=sse` o `server.transport="sse"`. **El terminal y el LSP siguen necesitando WebSocket**, la RTC (`rtc_v2`) se desactiva con SSE y los proxies no deben hacer *buffering* de `/sse`.
- Salud: `/health` y `/healthz` devuelven 200, y `/api/status` devuelve JSON.

### 2.2 Modelo de procesos del kernel (`marimo/_session/managers/kernel.py`)
- **Edit:** un `multiprocessing.Process` por sesión con contexto *spawn*. El hijo se conecta al padre por `connection.Listener(AF_INET)`. Es un proceso para poder interrumpirlo con SIGINT y matar su grupo de procesos.
- **Run:** un `threading.Thread` por sesión, "para minimizar memoria" porque un proceso costaría unos 150 MB. Ese hilo no se puede interrumpir ni matar.
- **Kernels IPC** (`marimo/_ipc/`): se arrancan como subproceso con `python -m marimo._ipc.launch_kernel`. Leen `KernelArgs` en JSON por stdin y escriben `KERNEL_READY` y `KERNEL_INFO <pid> <exe>` por stdout. Se comunican por **ZeroMQ PUSH/PULL sobre `tcp://127.0.0.1`** con puertos aleatorios y msgpack (msgspec). Los canales son `control`, `ui_element`, `completion`, `input` y `stream`. Se usan con `--sandbox`: desde la 0.25.0 hay un entorno por cuaderno y el editor arranca separado de los kernels. El docstring dice textualmente que la ruta del módulo es **API pública** para consumidores externos como marimo-lsp.
- `app_host.py` es un proceso "AppHost" que sirve varios kernels de **run** como hilos, multiplexados por ZMQ. Parece ligado a `experimental.isolate_apps`, pero esa relación está **no verificada**.

### 2.3 Opciones de `marimo edit` (docs.marimo.io/cli)
| Opción | Qué hace |
|---|---|
| `--host` (por defecto 127.0.0.1), `-p/--port` | Dirección de escucha |
| `--headless` | No abre el navegador |
| `--base-url /ruta` | Sirve bajo un subpath. Caso real en Open OnDemand: hace falta que el proxy **no recorte** el prefijo. |
| `--proxy host:puerto` | "Dirección del proxy inverso". Su efecto exacto en el código está **no verificado**. |
| `--token/--no-token` | Token activo **por defecto en edit** (aleatorio) e inactivo en run (allí es determinista a partir del código) |
| `--token-password`, `--token-password-file` (también `-` para stdin) | Fija el token |
| `--allow-origins` | CORS, repetible |
| `--skip-update-check` | Salta la comprobación de versión. También `MARIMO_SKIP_UPDATE_CHECK=1`. |
| `--watch` | Recarga el cuaderno cuando cambia en disco |
| `--session-ttl N` | Segundos que dura la sesión tras desconectarse el WebSocket. En edit, por defecto nunca se cierra; en run, 120 s. |
| `--timeout N` | Apaga el servidor tras N minutos sin conexiones |
| `--sandbox[=uv|pixi]`, `--no-sandbox` | Entorno aislado por cuaderno (PEP 723) |
| `--trusted/--untrusted` | Con `--untrusted`, los cuadernos remotos se ejecutan en Docker |
| `--skew-protection` | Middleware contra desajustes de versión entre frontend y servidor |
| `--mcp` (+ `--mcp-allow-remote`) | Servidor MCP en `/mcp/server` (experimental) |

- Formas de pasar el token: el parámetro `?access_token=…`, una página de login o **Basic auth** con el token como contraseña. Tras el login se crea una cookie de sesión. `MARIMO_SESSION_SECRET` firma esas cookies (hay que fijarlo para que sobrevivan a reinicios o funcionen entre réplicas) y `MARIMO_SESSION_COOKIE_SECURE` las marca como seguras.
- **Acceso a Internet en tiempo de ejecución:**
  - **Comprobación de versión:** hace un GET a `https://marimo.io/api/oss/latest-version` como mucho una vez al día, con timeout de 3 s (`marimo/_cli/upgrade.py`). Se desactiva con `--skip-update-check` o `MARIMO_SKIP_UPDATE_CHECK=1`.
  - **Telemetría del paquete:** no encontré ninguna, pero **no verificado** de forma exhaustiva. La política de privacidad de marimo.io y molab menciona PostHog solo para sus webs.
  - **Recursos del frontend:** `frontend/index.html` no referencia ningún host externo y las fuentes (Lora, PT Sans, Fira Mono) van empaquetadas. Que ningún componente cargue algo de una CDN de forma diferida (mermaid, KaTeX, widgets…) está **no verificado**. Pyodide y las exportaciones WASM sí dependen de CDN salvo con `--offline` (0.25.0).
  - **IA, Copilot e instalación de paquetes** salen a la red cuando se usan (secciones 8 y 10).

Fuentes: https://docs.marimo.io/cli/ · https://docs.marimo.io/guides/deploying/ · https://docs.marimo.io/guides/deploying/authentication/ · https://docs.marimo.io/guides/configuration/ · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_session/managers/kernel.py · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_ipc/connection.py · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_ipc/launch_kernel.py · https://raw.githubusercontent.com/marimo-team/marimo/main/packages/openapi/api.yaml · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_cli/upgrade.py · https://raw.githubusercontent.com/marimo-team/marimo/main/frontend/index.html · https://discourse.openondemand.org/t/proxy-problem-with-marimo/4206

---

## 3. Incrustación programática (ASGI)

- `marimo.create_asgi_app(include_code=…, quiet=…)` ofrece `.with_app(path=, root=)` (encadenable), `.with_dynamic_directory(path=, directory=)` (recoge cuadernos nuevos sin reiniciar) y `.build()`. El resultado se monta en FastAPI o Starlette con `app.mount("/", server.build())`.
- **Autenticación propia:** se hace con middleware **ASGI puro** que pone `scope["user"]` y `scope["meta"]` tanto en `http` como en `websocket`. La documentación desaconseja `BaseHTTPMiddleware` porque no cubre WebSocket. Recomienda también no pasar por la autenticación los recursos estáticos que se sirven bajo la ruta de cada cuaderno. En el cuaderno, `mo.app_meta().request` expone headers, cookies, query_params, `user` y `meta`.
- **Limitación:** la documentación de `create_asgi_app` solo describe el modo **app (run)**. Que admita modo **edit** está **no verificado**; no hay ejemplo.
- Varios cuadernos: `marimo edit <carpeta>` abre una página de inicio con explorador de ficheros, y `marimo run <carpeta>` monta una galería con tarjetas OpenGraph.

Fuentes: https://docs.marimo.io/guides/deploying/programmatically/ · https://docs.marimo.io/guides/deploying/authentication/ · https://docs.marimo.io/guides/apps/

---

## 4. Kernel sin el frontend web, VS Code, Jupyter, Kubernetes y molab

### 4.1 Extensión de VS Code (marimo-team/marimo-lsp, publicada como `marimo-team.vscode-marimo`)
- Se anunció el **2025-11-18**. Es nativa sobre la *Notebook API* de VS Code, funciona también en Cursor y usa licencia Apache-2.0.
- Tiene dos partes: un **servidor de lenguaje en Python sobre pygls** (`src/marimo_lsp/server.py`) y una extensión TypeScript.
  - El servidor recibe `notebookDocument/didOpen|didChange|didSave|didClose`.
  - Arranca **kernels** (`Kernels.launch`, que reutiliza `marimo._ipc.launch_kernel`).
  - Envía comandos internos de marimo (`ExecuteCellCommand`, `UpdateUIElementCommand`, `CreateNotebookCommand`, … de `marimo._runtime.commands`).
  - Reenvía al cliente cada mensaje del kernel como la notificación LSP **`marimo/kernelNotification`**, deserializada con `marimo._messaging.serde`. El cliente le habla con la petición `marimo/command`.
- `pyproject.toml` distingue entre la versión de marimo **empaquetada** (servidor y frontend) y la versión mínima del **kernel del usuario**. Es decir, las salidas se pintan con el frontend de marimo empaquetado en la extensión. El detalle exacto del *renderer* está **no verificado**.
- Conclusión: **sí se puede mover el kernel sin el servidor web de marimo**, pero solo apoyándose en módulos `marimo._*` privados. El único punto que se declara estable es la ruta `marimo._ipc.launch_kernel`. Pintar las salidas exige el frontend de marimo: son HTML con elementos propios `marimo-*` y plugins de UI.

### 4.2 Otras superficies
- **marimo pair** (abril y julio de 2026): agentes que ejecutan código en el kernel vivo mediante `marimo._code_mode`, que el propio docstring declara "Internal, agent-only API… No versioning guarantees". En la 0.25.0 hay una CLI para agentes en *preview* (`marimo pair docs|execute|notebook list|prompt`).
- **marimo-studio** (2026-10-01): vistas propias (HTML, React, Svelte) sobre un cuaderno mediante `<marimo-cell>`, `<marimo-output>`, `mo-value` y el evento `marimo-value-updated`, con "provider API" para otros frontends. Es una vía oficial para tener **UI propia sobre la reactividad**, aunque orientada a informes y apps, no al editor. Licencia **no verificada**.
- **Islands** (`marimo-island`): incrustan salidas en HTML ajeno, pero el runtime es Pyodide. El equipo las considera "early feature".
- **Jupyter:** `marimo-jupyter-extension` (Apache-2.0) es un fork de jupyter-marimo-proxy. Lanza marimo detrás del servidor de Jupyter (*server proxy*) con autenticación por token. Se configura con `c.MarimoProxyConfig` (`sandbox`, `timeout`, `idle_timeout`, `session_ttl`, `watch`, `allow_origins`, `skip_update_check`…) y requiere `marimo[sandbox]>=0.25.0`. Funciona con cualquier *authenticator* o *spawner* de JupyterHub.
- **Kubernetes:** existe **marimo-operator** (Apache-2.0, `MarimoNotebook` `marimo.io/v1alpha1`, 27 estrellas, sin *releases*). Clona un git en un PVC, admite montajes S3, sshfs y rsync, *sidecars* como git-sync o SSH, y autenticación por contraseña en un Secret. Se complementa con el *plugin* `kubectl marimo edit|run|sync`. El modelo es "**un pod por cuaderno** con marimo editando sobre un PVC".
- **Docker:** la guía usa `marimo run app.py --host 0.0.0.0 -p 8080` con `HEALTHCHECK` en `/health`. La imagen oficial por defecto es `ghcr.io/marimo-team/marimo:latest` (citada en la guía de Kubernetes). Las variantes de etiqueta están **no verificadas**.
- **molab** (servicio alojado): contenedores con marimo, primero sobre Modal y ahora "nativo sobre CoreWeave". Usa uv para dependencias, Cloudflare R2 como almacenamiento y Logfire para monitorizar. Da 4 CPU y 32 GB, con un límite de 12 h por sesión y 90 min de inactividad. Su arquitectura interna (cómo enruta los kernels) **no es pública**.

Fuentes: https://marimo.io/blog/vscode · https://github.com/marimo-team/marimo-lsp · https://raw.githubusercontent.com/marimo-team/marimo-lsp/main/src/marimo_lsp/server.py · https://raw.githubusercontent.com/marimo-team/marimo-lsp/main/src/marimo_lsp/sessions.py · https://raw.githubusercontent.com/marimo-team/marimo-lsp/main/AGENTS.md · https://marimo.io/blog/notebooks-as-a-tool-for-agents · https://marimo.io/blog/introducing-marimo-studio · https://marimo.io/blog/marimo-anywhere · https://github.com/marimo-team/marimo-jupyter-extension · https://docs.marimo.io/guides/deploying/deploying_kubernetes/ · https://github.com/marimo-team/marimo-operator · https://docs.marimo.io/guides/deploying/deploying_docker/ · https://marimo.io/blog/announcing-molab · https://marimo.io/blog/reintroducing-molab

### 4.3 marimohub: el patrón de referencia
- Se anunció el **2026-10-02** con licencia **Apache-2.0**. Está escrito en TypeScript (Node 24) y su web propia usa Vite. Es *stateless* y **no usa base de datos**: fuente, versiones, *snapshots* y auditoría viven en el almacén de objetos, que debe soportar **escrituras condicionales atómicas**.
- Tiene tres *backends* intercambiables:
  - **Almacenamiento:** S3, **GCS**, Azure, R2, CAIOS, MinIO o sistema de ficheros.
  - **Cómputo:** **Kubernetes** (un **pod por kernel**), Modal, Fargate, Docker, Podman, E2B, CoreWeave, Cloudflare o local.
  - **Identidad:** **OIDC** (Keycloak entra aquí por ser OIDC genérico, aunque no se nombra), proxy SSO de confianza, Google IAP o Cloudflare Access.
- **El navegador carga el editor de marimo en un iframe:**
  - `MARIMOHUB_SANDBOX_EXPOSURE=subdomain` (por defecto): el kernel queda en un origen hermano aislado, con DNS y TLS comodín. El hub no autentica ese tráfico directo, así que lo tiene que proteger el *ingress*.
  - `proxy`: el tráfico pasa por `/proxy/<token>/` del hub, con autorización por sesión **en cada petición** y WebSockets que se cierran al vencer la autorización. Al ser el mismo origen, "un cuaderno malicioso puede programar el plano de control". Por eso exige `MARIMOHUB_SANDBOX_PROXY_ACK_UNTRUSTED=true`.
  - `MARIMOHUB_SANDBOX_AUTH=on` añade un token marimo de 256 bits por sesión, guardado en un fichero (`--token-password-file`). Con `off` (por defecto) se lanza con `--no-token`.
- Las sesiones de editor pueden ser compartidas (un *sandbox* por cuaderno, todos los editores en la misma sesión marimo) o exclusivas (con *takeover*). Los guardados se atribuyen a quien arrancó el *sandbox*. Tiene *sync* desde GitHub (en solo lectura), *warm pools* y IA gestionada con la clave en el servidor.
- El mecanismo exacto para devolver el `.py` del *sandbox* al almacén **no está documentado** (**no verificado**). Las docs hablan de un "workspace mirror" y del "normal save path".

Fuentes: https://marimo.io/blog/introducing-marimohub · https://github.com/marimo-team/marimohub · https://marimohub.docs.marimo.io/getting-started · https://marimohub.docs.marimo.io/compute · https://marimohub.docs.marimo.io/security · https://marimohub.docs.marimo.io/editor-sessions

---

## 5. Formato de fichero, guardado y vigilancia de cambios

- **Python puro.** Cada celda es una función decorada con `@app.cell` que recibe sus referencias y devuelve sus definiciones. Hay además `with app.setup:` para imports y constantes, y `@app.function` y `@app.class_definition` para funciones y clases reutilizables que se pueden importar desde otros módulos. Las salidas **no** se guardan en el `.py`.
- **Diffs limpios en git:** editar una celda cambia solo su función y la versión de marimo cambia en una sola línea (`__generated_with`), según MEP-0001. Las celdas SQL son azúcar sobre `mo.sql(...)`. También hay un formato Markdown alternativo.
- `marimo convert` acepta `.ipynb`, `.md` y `.py` (`-o` para la salida). `marimo export` produce html, html-wasm, ipynb, md, pdf, script, **session** (JSON) y thumbnail.
- **Guardado:** `POST /api/kernel/save` llama a `session.app_file_manager.save(...)`, valida que la ruta quede dentro del directorio de trabajo y escribe en disco. La configuración por defecto es `save.autosave="after_delay"` y `autosave_delay=1000` ms. **No hay almacenamiento enchufable ni *hook* de guardado documentado** para el cuaderno.
- **Ficheros auxiliares junto al cuaderno:**
  - `__marimo__/cache/`: `mo.persistent_cache`, que la documentación recomienda ignorar en git.
  - `__marimo__/session/`: *snapshot* de salidas para reabrir sin ejecutar (desde la 0.11.7). Que siga activo por defecto está **no verificado**.
  - `layouts/`: disposiciones grid y slides, que sí deben versionarse.
- **`--watch`:** si otro proceso reescribe el fichero, el editor lo sincroniza y marca las celdas afectadas como *stale* (`runtime.watcher_on_save="lazy"`, o `"autorun"`). En run, recarga la página. Con watchdog instalado usa eventos y sin él hace *polling*. **No hay resolución de conflictos documentada** cuando el editor y un proceso externo escriben a la vez.

Fuentes: https://marimo.io/blog/python-not-json · https://github.com/marimo-team/meps/blob/main/mep-0001.md · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_server/api/endpoints/files.py · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_config/config.py · https://docs.marimo.io/guides/editor_features/watching/ · https://newreleases.io/project/pypi/marimo/release/0.11.7

---

## 6. Integraciones de datos

- **`mo.sql(f"...", engine=conn)`:** el motor por defecto es DuckDB en memoria. Admite conexión DuckDB propia, SQLAlchemy, SQLModel, Ibis, ClickHouse Connect, chDB, **DB-API 2.0** y **ADBC**. Los motores en código están en `marimo/_sql/engines/`: adbc, clickhouse, dbapi, duckdb, ibis, **pyiceberg**, redshift, sqlalchemy.
  - Las conexiones definidas como variables en una celda se autodetectan y aparecen en el desplegable de la celda SQL.
  - No hay una API pública documentada para registrar un motor propio (subclase de `BaseEngine`): **no verificado**. La vía práctica es pasar una conexión DuckDB, DB-API o ADBC.
  - Salida configurable con `runtime.default_sql_output`: auto, native, polars, lazy-polars o pandas. Con `MARIMO_SQL_DEFAULT_LIMIT` se fija un límite de filas.
- **Panel *Data sources*:** muestra esquemas, tablas y columnas, con autodescubrimiento configurable (`[tool.marimo.datasources] auto_discover_*`). Desde la 0.23.16 también detecta orígenes **del entorno**: PyIceberg, Postgres, MySQL, Trino, AWS, PySpark…
- **Iceberg REST:** hay soporte explícito de **catálogos PyIceberg** (`RestCatalog(name, warehouse, uri, token)`, además de SQL, Glue, DynamoDB…).
  - Se detectan desde `PYICEBERG_CATALOG__<nombre>__*` o `.pyiceberg.yaml` y se añaden con el botón "+".
  - **Aparecen en el panel pero "no se pueden usar como motor en celdas SQL"**. Primero hay que cargar la tabla (`catalog.load_table(...).to_polars()`) y luego consultarla.
  - Con *vended credentials* el flujo debería funcionar, porque lo resuelve PyIceberg, pero no lo he **verificado** contra un catálogo con credenciales delegadas.
- **Almacenamiento remoto (panel Files):** detecta variables `obstore.store.ObjectStore` y `fsspec.AbstractFileSystem` (S3, GCS, Azure, HTTP…). Desde la 0.24 hay HF Hub y un explorador de carpetas locales y de GitHub, y desde la 0.25.1 SFTP y SSH.
- **Dataframes:** tabla interactiva con paginación, búsqueda, orden, filtros, explorador de columnas (requiere altair, y vegafusion para tablas grandes) y constructor de gráficos. Además `mo.ui.table(selection=...)`, `mo.ui.dataframe` (transformaciones sin código que generan código) y `mo.ui.data_explorer`. narwhals es dependencia núcleo, lo que hace que el código sea agnóstico de pandas/polars. Que la paginación sea en servidor sobre *lazy frames* está **no verificado** con documentación primaria.
- **Widgets:** `mo.ui.*` (slider, dropdown, run_button, form, file_browser, chat, altair_chart, plotly…) son reactivos: cambiar el valor re-ejecuta las celdas dependientes. **anywidget** está soportado (`mo.ui.anywidget`) y la 0.23.14 añadió composición y *hot reload*. El ESM lo aporta cada widget y algunos lo importan de CDN, lo cual depende del widget.

Fuentes: https://docs.marimo.io/guides/working_with_data/sql/ · https://github.com/marimo-team/marimo/tree/main/marimo/_sql/engines · https://docs.marimo.io/guides/working_with_data/remote_storage/ · https://docs.marimo.io/guides/working_with_data/dataframes/ · https://docs.marimo.io/guides/integrating_with_marimo/custom_ui_plugins/

---

## 7. Reactividad y efectos laterales

- marimo analiza estáticamente las definiciones y referencias de cada celda y monta un DAG. Ejecutar una celda **re-ejecuta sus dependientes**. No sigue mutaciones: `x.append()` no dispara nada. Borrar una celda borra sus variables y re-ejecuta o marca como *stale* a las dependientes.
- **`runtime.on_cell_change="lazy"`:** en vez de ejecutar las dependientes, las marca *stale*. Si se ejecuta una celda con ancestros *stale*, primero se ejecutan esos ancestros.
- **`runtime.auto_instantiate`:** en `DEFAULT_CONFIG` vale **False**, es decir, no ejecuta todo al abrir en edit. Su docstring dice True; vale el código. No afecta a `marimo run`.
- **Celdas desactivadas:** bloquean a sus descendientes. Al reactivarlas se ejecutan si algún ancestro cambió mientras estaban desactivadas.
- **`mo.stop(cond)` y `mo.ui.run_button()`:** el patrón recomendado para celdas caras o con efectos. La celda solo continúa tras pulsar el botón.
- **Caché:**
  - `mo.cache` funciona en memoria y `mo.persistent_cache` en disco, ambos como decorador o *context manager*.
  - `runtime.cache_cells=true` cachea todas las celdas.
  - `[cache] store` admite tipos **`file`, `redis`, `rest` y `tiered`**, y `cache.verification` admite off, on o strict.
  - Desde la 0.25.1 hay un manifiesto de caché por cuaderno y los comandos `marimo cache dir|size`.
- **Escrituras:** la documentación no trata la idempotencia. La reactividad puede **re-ejecutar una celda que escribe** cada vez que cambia un ancestro, un widget, o al recargar con `watcher_on_save="autorun"`. Para el `write()` de ORE: modo `lazy`, `mo.stop` con `run_button` y la clave de idempotencia (que deduplica re-ejecuciones idénticas) son complementarios. Esto es inferencia mía, no documentación.

Fuentes: https://docs.marimo.io/guides/reactivity/ · https://docs.marimo.io/guides/configuration/runtime_configuration/ · https://docs.marimo.io/guides/expensive_notebooks/ · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_config/config.py

---

## 8. Gestión de paquetes

- **Proyecto:** un manifiesto compartido (`pyproject.toml` + `uv.lock`/`pixi.lock`) y arranque con `uv run marimo edit`. Gestores admitidos: pip, uv, poetry, pixi y rye (`package_management.manager`).
- **Sandbox:** `--sandbox[=uv|pixi]` con dependencias en metadatos **PEP 723** dentro del propio `.py`. Desde la 0.25.0 hay **un entorno por cuaderno**: instalar desde el editor reescribe los metadatos y sincroniza. Hay UI de recuperación y el kernel se lanza como **subproceso IPC**.
- **Instalación desde el editor:** el panel Packages y el aviso que aparece en un `ModuleNotFoundError`. Los *endpoints* son `/api/packages/add|remove|sync|sandbox` y `/api/kernel/install_missing_packages`.
- **Desactivarla:** **no encontré una opción documentada** que oculte el panel o el aviso. Lo más cercano:
  - `[tool.marimo.venv] path=…, writable=false`, que por defecto ya es false: marimo no instala en ese venv. La documentación lo limita al "Sandboxed Home" (`marimo edit --sandbox carpeta/`). Que también oculte la UI está **no verificado**.
  - Sin acceso a Internet, la instalación simplemente fallará.
  - Alternativa robusta, inferida: bloquear en el proxy de ORE las rutas `/api/packages/*` y `/api/kernel/install_missing_packages`.
- Seguridad: la documentación dice que el *sandbox* aísla paquetes, **no** ficheros ni red.

Fuentes: https://docs.marimo.io/guides/package_management/ · https://docs.marimo.io/guides/package_management/installing_packages/ · https://github.com/marimo-team/marimo/releases/tag/0.25.0 · https://www.cubic.dev/pr/marimo-team/marimo/pull/10696

---

## 9. Apps, WASM y escala

- **`marimo run`:** convierte el cuaderno en app con código oculto por defecto (`--include-code` para mostrarlo), trazas redactadas (`--show-tracebacks` para verlas) y disposiciones vertical, grid o slides. Varios ficheros o una carpeta dan una galería.
  - Cada sesión de navegador tiene su **kernel en un hilo** dentro del mismo proceso, con `--session-ttl` de 120 s por defecto. Eso implica que todas las sesiones comparten la memoria y la GIL del proceso.
  - Con `--sandbox` y varios ficheros usa kernels IPC. El proceso AppHost y `isolate_apps` (experimental) existen, pero su alcance está **no verificado**.
- **Escalar:** la documentación no da recetas. Solo recomienda HTTP/2 por el límite de conexiones por origen. Para varias réplicas hacen falta `MARIMO_SESSION_SECRET` común y afinidad de sesión; esto último es inferencia, porque el kernel vive en un proceso concreto.
- **WASM/Pyodide:** `marimo export html-wasm`, con `--single-file` y `--offline` desde la 0.25.0. Hay *islands* y las integraciones quarto, jupyter-book y mdx. No todo el ecosistema funciona en Pyodide.

Fuentes: https://docs.marimo.io/guides/apps/ · https://docs.marimo.io/guides/deploying/ · https://marimo.io/blog/marimo-anywhere

---

## 10. IA

- La IA se configura en la sección `[ai]` de `marimo.toml`, en `pyproject.toml` o en PEP 723.
- **Proveedores:** open_ai, anthropic, google, bedrock, azure, ollama, github (Copilot), openrouter, wandb, opencode_go y **`[ai.custom_providers.<nombre>] base_url=…, api_key="env:VAR"`** para cualquier servicio compatible con OpenAI. Admiten además `extra_headers`, `ssl_verify`, `ca_bundle_path` y `client_pem`. Los modelos se fijan en `[ai.models] chat_model, edit_model, autocomplete_model`.
- **Desactivarla:** `ai.enabled=false` oculta el chat y "Generate with AI". El autocompletado se controla aparte con `completion.copilot=false|"github"|"codeium"|"custom"`. También existen `ai.allow_provider_config` y `sharing.molab|html|wasm=false`.
- **Las llamadas al LLM salen del servidor Python**, no del navegador: `POST /api/ai/chat|completion|inline_completion`. En ORE eso significa que saldrían **desde el pod**, así que se puede apuntar `base_url` a un *endpoint* de ore-serve. Copilot de GitHub necesita Node.js.
- **Modos del chat:** manual, ask (herramientas de solo lectura más MCP), agent (edita celdas) y code_mode (kernel).
- **MCP (experimental):**
  - Como servidor, `marimo edit --mcp` expone `/mcp/server` y se autentica con `?access_token`.
  - Como cliente, `[mcp] presets=["marimo","context7"]`. La documentación dice que los servidores personalizados "aún no", aunque el código ya define `mcpServers`.
- marimohub ofrece **IA gestionada** con la clave guardada en el servidor.

Fuentes: https://docs.marimo.io/guides/editor_features/ai_completion/ · https://docs.marimo.io/guides/configuration/llm_providers/ · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_server/api/endpoints/ai.py · https://docs.marimo.io/guides/editor_features/mcp/

---

## 11. Colaboración, reconexión y reinicio

- **Reconexión:** en edit, la sesión y el kernel sobreviven a que se cierre la pestaña. Lo producido mientras tanto se envía al reconectar (desde la 0.2.0) y la sesión vive según `--session-ttl` (por defecto nunca expira en edit). Si el **servidor** se reinicia, el estado del kernel se pierde. Lo único que puede restaurar salidas sin ejecutar es el *snapshot* de `__marimo__/session`.
- Existe `/api/kernel/takeover` para tomar una sesión abierta en otra pestaña. El detalle de su comportamiento está **no verificado**.
- **RTC:** el código incluye `loro` (CRDT) y el flag `experimental.rtc_v2`. Con SSE se desactiva. **No encontré documentación oficial vigente**: qué se sincroniza y en qué estado está quedan **no verificados**. marimohub en modo compartido pone a varios editores en una sola sesión, "sin CRDT propio ni montajes por usuario".
- **Reinicio:** `/api/kernel/restart_session` vacía la memoria del programa. En edit, interrumpir envía SIGINT al grupo de procesos del kernel.

Fuentes: https://newreleases.io/project/pypi/marimo/release/0.2.0 · https://docs.marimo.io/faq/ · https://raw.githubusercontent.com/marimo-team/marimo/main/marimo/_config/config.py · https://marimohub.docs.marimo.io/editor-sessions

---

## 12. Despliegues conocidos dentro de otras plataformas

| Plataforma | Patrón | Fuente |
|---|---|---|
| **marimohub** (oficial, OSS) | Iframe del editor marimo hacia un *sandbox* por sesión (pod en Kubernetes), por subdominio aislado o por proxy autenticado | marimohub.docs.marimo.io |
| **molab** (oficial, SaaS) | Contenedores marimo en Modal y luego CoreWeave. Interno no publicado. | marimo.io/blog/announcing-molab |
| **JupyterHub / JupyterLab** | `marimo-jupyter-extension`: marimo detrás del *server proxy* de Jupyter, con token entre proxy y marimo | github.com/marimo-team/marimo-jupyter-extension |
| **Kubernetes** | marimo-operator: un pod por cuaderno, PVC, git-sync como *sidecar* y *port-forward* | github.com/marimo-team/marimo-operator |
| **Quix Cloud** ("dev sessions") | Sesión marimo ligada a un despliegue, 1 GB persistente y **auto-commit al git** tras 5 s de inactividad, con *pull* cada 5 s. La forma de exponerlo (iframe o proxy) está **no verificada**. | docs.quix.io/…/marimo-devsessions.html |
| **Open OnDemand** (Fred Hutch, comunidad) | Proxy inverso con `--proxy`, `--base-url /node/<host>/<port>`, `--headless` y `--no-token` | discourse.openondemand.org/t/4206 |
| **Hugging Face Spaces** | Plantilla Docker de `marimo run` (*app*) | docs.marimo.io/guides/deploying/deploying_hugging_face/ |
| **SageMaker Studio** | Puente comunitario dentro de JupyterLab (no oficial) | github.com/scttfrdmn/aws-marimo-sagemaker |
| **Databricks** | Conectores SQL/Ibis y adaptador comunitario `marimo-databricks-connect` como Databricks App. marimo no admite kernels Jupyter personalizados. | github.com/brookpatten/marimo-databricks-connect |
| Posit Workbench, Snowflake, Hex o Deepnote incrustando el editor | **No encontré evidencia** | — |

**Patrón dominante:** nadie reimplementa la UI de marimo. Todos sirven **el frontend de marimo tal cual**, detrás de un proxy inverso o en un iframe, y ponen la identidad y la autorización en el proxy o el hub. La única UI "propia" es la de VS Code, y la mantiene el propio equipo de marimo sobre APIs internas.

---

## 13. Implicaciones para ORE (inferencias, no fuentes)

1. **Túnel.** El puesto no acepta conexiones entrantes, pero marimo necesita que el navegador llegue a su HTTP y WebSocket. Las opciones son:
   - Que el agente del puesto multiplexe HTTP+WS a través de su *long-poll* con ore-serve. Es lo mismo que ya hacemos con el LSP, generalizado.
   - Usar `MARIMO_SERVER_TRANSPORT=sse` para quitar el WS del kernel. El terminal y el LSP siguen en WS, aunque el LSP ya lo tenemos y el terminal se puede omitir.

   Arrancar con `marimo edit --headless --host 127.0.0.1 --base-url /puestos/<id>/marimo --skip-update-check --token-password-file …` (o `--no-token` si ore-serve autoriza en cada petición, como el modo proxy de marimohub).
2. **Origen aislado.** marimohub advierte de que servir el kernel en el **mismo origen** que el plano de control permite que un cuaderno malicioso lo programe. Para la consola de ORE conviene un subdominio propio para el iframe de marimo, o aceptar el riesgo de forma explícita.
3. **Guardado y commit.** marimo escribe el `.py` en el disco del pod con autoguardado al segundo, y el pod no puede escribir el árbol. Dos vías:
   - Interceptar `POST /api/kernel/save` en ore-serve.
   - Que el agente vigile el fichero y suba el contenido para que la persona haga *commit*.

   La dirección contraria (otra persona cambia el árbol) entra con `--watch` y queda *stale*. Excluir `__marimo__/` (caché y sesión) del árbol.
4. **Datos.** Para que el panel muestre ore-serve como catálogo Iceberg REST, basta con `PYICEBERG_CATALOG__ore__URI=…` y su token en el entorno del kernel. Para SQL, que `ore.sql()` exponga su conexión DuckDB como variable, así marimo la detecta como motor. La comprobación con *vended credentials* sigue pendiente.
5. **Red.** Hay que poner `--skip-update-check`, `ai.enabled=false` o un `custom_providers` hacia ore-serve, `completion.copilot=false` y bloquear `/api/packages/*` en el proxy, porque no hay interruptor documentado.
6. **Kernel por puesto.** En edit hay un subproceso por cuaderno abierto, así que hay que medir memoria. El arranque *spawn* de Python más marimo es del orden de cientos de MB (marimo cita unos 150 MB solo por copiar un proceso). Está **sin medir en un puesto**.
7. **Una UI propia sobre `marimo._ipc` sería costosa y frágil:** depende de mensajes internos sin contrato y obliga a pintar salidas `marimo-*`. Lo realista es el iframe del frontend oficial, quizá con marimo-studio más adelante para vistas de app.
