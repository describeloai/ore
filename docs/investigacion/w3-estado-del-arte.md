# W3 · el estado del arte de los entornos de trabajo (2026-09-19)

Antes de escribir 0031 («el puesto») miramos cómo construyen **encima del sustrato** quienes ya
venden este entorno: Palantir Foundry, Databricks, Snowflake, AWS SageMaker, Google Vertex/Colab
Enterprise, y la pila abierta sobre Kubernetes (Kubeflow Workspaces, Kueue). El sustrato —bucket
por inquilino, árbol en la forja, organización, malla, identidad, cola— es nuestro y no está en
cuestión; lo que se coteja son las capas de arriba: **la sesión, el runtime, las dependencias,
los datos desde código, las salidas, los modelos, las ramas y la red**.

Fuentes primarias (documentación de cada proveedor), enlazadas al final. Donde la documentación
da un número, va el número.

## 1 · Cómo lo hace cada uno

| | unidad interactiva | recursos | ociosidad | dependencias | datos desde código | modelos | ramas | red |
|---|---|---|---|---|---|---|---|---|
| **Foundry · Code Workspaces** | un contenedor por workspace (JupyterLab, RStudio, VS Code) con un **sidecar de Foundry** (0,25 vCPU · 3 GiB); **un solo nodo** («no es para transformaciones a escala: eso es Spark») | perfil elegible al crear o después (slider); por defecto 0,75 CPU · 6 GB gratis; más CPU/RAM/GPU = pago por uso; GPU sólo si el proyecto tiene *resource queue* con GPU | apagado por inactividad **30 min** por defecto, máx. 6 h; sesión máx. **24 h** | **Maestro**: `meta.yaml` (lo pedido) + `hawk.lock` (lo resuelto); el entorno resuelto se restaura igual en workspace, transform, app y modelo | panel *Data*: alias de dataset → SDK (`foundry.transforms`), pandas/polars/arrow; escribir = *transform* publicado con su JobSpec | *model adapter* (interfaz estándar en un `.py`) + snippet de publicación desde la celda; disponible al instante en el resto de la plataforma | el workspace está atado a una rama del *Code Repository*; publicar/sincronizar publica en esa rama | cerrada; salidas sólo a URLs con *Network Policy*; *restricted outputs* = modo sólo lectura sin exportación |
| **Foundry · Code Repositories** | no hay sesión: editor + **Checks en cada commit** (resuelven el entorno con Hawk, definen entradas/salidas) | el *build* corre en Spark o en un contenedor (`@sidecar`) | — | `conda_recipe/meta.yaml`; el lock en `.maestro/`; cambiar una librería es un cambio de código que pasa por PR y rama protegida | *transforms* Python/Java/SQL/R/contenedor | idem; *compute modules* = tu imagen + sidecar que **hace polling de invocaciones** | **datos ramificados con el código**: los JobSpecs se publican en la rama; un *build* en `develop` cae a `master` (*fallback*) para lo que la rama no tiene | idem |
| **Foundry · Functions** | *serverless* (versiones bajo demanda, «preferido») o *deployed* (contenedor de larga vida, una versión) | — | — | por lenguaje (TS, Python) | por el OSDK, nunca por conexión | — | por versión | idem |
| **Databricks · serverless** | sesión sin clúster que provisionar; plano de cómputo gestionado | tamaño de memoria en el panel *Environment*; GPU serverless sólo **A10 y H100** | — | **versión de entorno** (1…5, con `requirements-env-N.txt` para reproducir en local) + **base environments** definidos por el admin en YAML, **precompilados y cacheados** («arrancan rápido»), máx. **10** por workspace; `%pip`/`%uv` para lo puntual; repos pip privados configurables | Unity Catalog | MLflow / Unity Catalog; dos entornos GPU: mínimo y «AI» con PyTorch/Transformers | git folders | plano serverless con su política de red |
| **Snowflake · Notebooks (Container Runtime)** | un contenedor por notebook en un *compute pool*; **un nodo entero por notebook** («`MAX_NODES` > 1») | pools de sistema CPU y GPU; el pool se elige | **1 h** por defecto, hasta 72 h; sesión hasta 7 días | imagen base CPU/GPU verificada; lo que está en la base **no se puede cambiar de versión**; `pip` sólo con *External Access Integration*; lo instalado **no persiste** entre sesiones | `session.table().to_pandas()`, `DataConnector` optimizado | Model Registry, `log_model`; APIs de entrenamiento distribuido | — | cerrada salvo EAI |
| **AWS · SageMaker Studio (JupyterLab spaces)** | un *space* (privado o compartido) = una instancia EC2 + un volumen EBS | tipo de instancia elegible, se cambia | apagado por inactividad configurable | imagen *SageMaker Distribution* (PyTorch, TF, pandas…); *lifecycle configs* | S3 | SageMaker | git integrado | VPC-only |
| **Google · Colab Enterprise / Workbench** | *runtime* creado desde una **plantilla** (máquina, aceleradores, disco, red) | por plantilla | **180 min** por defecto, 10–1440 | contenedor propio en Workbench | GCS / BigQuery | Vertex | — | «turn off public internet access» en la plantilla |
| **Kubeflow Workspaces (2.0)** | CRD `Workspace` + `WorkspaceKind` (jupyter-lab, vs-code, rstudio): el usuario elige **3 desplegables y 2 volúmenes** (kind, pod-config `small_cpu`/`big_gpu`, imagen) | *pod-configs* definidos por el admin | *culling* de pods ociosos | imágenes por kind | volúmenes | — | — | NetworkPolicy |
| **Kueue para sesiones (SageMaker HyperPod)** | los *interactive spaces* entran como cargas de Kueue | cuota por equipo en su `ClusterQueue` | — | — | — | — | — | — |

## 2 · Lo que se repite (las buenas prácticas, a grandes rasgos)

1. **Un contenedor por persona, con perfil de recursos elegible y apagado por inactividad.** Todos.
   Los números: 30 min (Foundry) · 60 min (Snowflake) · 180 min (Colab); y un tope de sesión
   (24 h Foundry, 7 días Snowflake). Nadie deja un puesto vivo sin límite.
2. **La sesión es de un nodo; lo masivo es un trabajo.** Foundry lo dice sin rodeos («single
   node… otras aplicaciones usan Spark»); Snowflake da un nodo entero por notebook; Databricks
   separa notebook serverless de *jobs*. La sesión sirve para escribir y probar; la escala se
   ejecuta como trabajo encolado.
3. **Las dependencias se declaran y las resuelve la plataforma, nunca `pip` en caliente como
   verdad.** Foundry: `meta.yaml` → `hawk.lock` resuelto en los *Checks*, restaurado igual en
   todas partes. Databricks: versiones de entorno numeradas + *base environments* del admin,
   precompilados y cacheados, límite de 10. Snowflake: imagen base intocable, lo instalado no
   persiste. Cambiar una librería es un cambio de código que pasa por PR.
4. **Red cerrada por defecto; se abre por política con nombre.** *Network Policies* y *Sources*
   (Foundry), *External Access Integration* (Snowflake), «sin internet público» (Colab), plano
   serverless (Databricks). Y un modo **sólo lectura sin exportación** (*restricted outputs*).
5. **Los datos se leen por un SDK con alias, no por credenciales del almacén.** Panel *Data* y
   `foundry.transforms` (Foundry), `session.table` y `DataConnector` (Snowflake), Unity Catalog
   (Databricks). El código dice *qué* dataset; la plataforma decide *cómo* y con *qué identidad*.
6. **Escribir es publicar una salida con nombre**, no `write()` a un sitio. Un *transform* con su
   JobSpec, una tabla en el catálogo, un modelo en el registro.
7. **Las ramas ramifican código y datos juntos, con *fallback* a `main`.** Foundry: los JobSpecs se
   publican en la rama y un *build* en `develop` lee de `master` lo que la rama no tiene.
8. **Un modelo es un artefacto con interfaz estándar** (*model adapter*), publicado desde la
   sesión y registrado; entrenar con GPU es la misma sesión o trabajo con un sabor GPU, y la GPU
   se **concede por cola de recursos**, no por pedirla.
9. **Un agente de plataforma dentro del pod.** El *sidecar* de Foundry (identidad, datos,
   publicación); los *compute modules* son «tu contenedor + sidecar que hace polling de
   invocaciones». El código del cliente no habla con la plataforma: habla con el sidecar.
10. **Funciones: serverless por versión, preferido a desplegado.** Foundry recomienda serverless
    porque «distintas versiones de una función se ejecutan bajo demanda, y actualizar es seguro».
11. **Gobierno de la cola para lo interactivo.** La guía de Kueue en HyperPod: prioridad alta a
    las sesiones (100 frente a 75 entrenamiento, 50 evaluación, 25 batch), **prestar cuota sí,
    tomar prestada no** (una sesión nunca corre sobre recursos reclamables), y *preemption*
    dentro del equipo.

## 3 · Cotejo con nuestro planteamiento

Lo que escribimos antes de mirar (los seis puntos de «el puesto») frente a lo que hacen:

| nuestro punto | veredicto | lo que añadimos tras mirar |
|---|---|---|
| **1 · el puesto** (imagen + identidad + datos + recursos; dos vidas: sesión y trabajo) | **validado**: es el *code workspace* de Foundry, el `Workspace` de Kubeflow, el nodo por notebook de Snowflake | la sesión es **de un nodo** por diseño (no crece); lo grande se manda como trabajo. TTL de inactividad (empezar en 30 min) y tope de sesión (24 h). Un **agente en el pod** (sidecar) que habla con ore-serve: identidad, datos, salidas; el código del cliente sólo ve el SDK |
| **2 · un runtime por lenguaje, sin instalación en caliente** | **validado**, con matiz | todos permiten **añadir** dependencias, pero **declaradas** y **resueltas por la plataforma**: lo nuestro es el modelo Foundry (`meta.yaml` → lock en CI) sobre imágenes base pocas y numeradas (Databricks: versiones de entorno, máx. 10 bases). Es decir: pocas imágenes base por lenguaje, y una **capa por paquete** construida en CI desde lo declarado en el árbol. Un `pip install` en la celda puede existir como comodidad **sin persistencia** (Snowflake), nunca como verdad |
| **3 · un plano de datos Arrow/Parquet en el bucket** | **validado** | falta el **SDK con alias**: `over("hr.empleados")` resuelve por la ontología a la copia, con la identidad de la persona; el código nunca ve el bucket. Y el **fallback de rama**: la rama lee las copias de `main` mientras no tenga las suyas (Foundry) |
| **4 · cómputo por sabor, cuota por organización** | **validado** (ya es Kueue) | la **política de sesiones**: prioridad alta, prestar sí / tomar prestado no, *preemption* intra-equipo; GPU concedida por cola (Foundry, Databricks A10/H100, Snowflake pool GPU) |
| **5 · modelos como documentos del árbol** | **validado** | la interfaz estándar (*model adapter*) ya la tenemos en la `Function`; los pesos van al bucket; **publicar desde la celda** (un snippet) y que el modelo esté disponible al instante en el resto |
| **6 · contrato consola ↔ puesto** | **validado** | *serverless por versión* para funciones (lo de 0029 F4a va por ahí); y el patrón *compute module*: el puesto **hace polling** de trabajo por el sidecar, no recibe conexiones — casa con «sin entrada» |

Lo que **no teníamos** y hay que tener desde el principio:

- **Escribir = publicar una salida con nombre** (dataset, tabla, modelo, función), no ficheros
  sueltos. Es lo que hace que el linaje exista.
- **Modo sólo lectura sin exportación** (*restricted outputs*) para datos sensibles: la sesión
  puede leer y no puede sacar. Con la red cerrada lo tenemos casi gratis; falta la política.
- **Números de entorno**: versionar las imágenes base como «entorno 1, 2, 3» con su
  `requirements` reproducible en local, no como *latest*.
- **Tope de sesión** además de TTL de inactividad.

## 4 · Lo que va a 0031

El ADR «el puesto» fija: la unidad (sesión de un nodo / trabajo encolado), el agente en el pod,
el SDK con alias y fallback de rama, las imágenes base numeradas + capa por paquete desde el
árbol, la política de Kueue para sesiones, el `Model` como documento con pesos en el bucket, las
salidas con nombre, y la red cerrada con *restricted outputs*. Y antes de construir, la medida:
frío del puesto en `jobs-p` (pool caliente y frío), peso y bajada de una imagen Python con
pandas/pyarrow, leer una copia real en DataFrame desde dentro, Kueue con un pod de larga vida,
y qué GPU hay en la región y a qué precio (consultado, no lanzado).

## Fuentes

- Palantir Foundry: [Code Workspaces · overview](https://www.palantir.com/docs/foundry/code-workspaces/overview) · [JupyterLab](https://www.palantir.com/docs/foundry/code-workspaces/jupyterlab) · [getting started (auto-shutdown)](https://www.palantir.com/docs/foundry/code-workspaces/getting-started) · [compute usage](https://www.palantir.com/docs/foundry/code-workspaces/compute-usage) · [security / restricted outputs](https://www.palantir.com/docs/foundry/code-workspaces/security) · [external systems](https://www.palantir.com/docs/foundry/code-workspaces/external-systems) · [Python environment overview](https://www.palantir.com/docs/foundry/transforms-python/environment-overview) · [Code Repositories overview](https://www.palantir.com/docs/foundry/code-repositories/overview) · [branching](https://www.palantir.com/docs/foundry/data-integration/branching) · [fallback branches](https://www.palantir.com/docs/foundry/pipeline-builder/branches-fallback-branches) · [model adapters](https://www.palantir.com/docs/foundry/integrate-models/model-adapter-creation) · [train in Code Workspaces](https://www.palantir.com/docs/foundry/integrate-models/model-asset-code-workspaces) · [GPU training](https://www.palantir.com/docs/foundry/model-integration/gpu-training) · [compute modules overview](https://www.palantir.com/docs/foundry/compute-modules/overview) · [container transforms](https://www.palantir.com/docs/foundry/transforms-container/container-overview) · [functions: deployed vs serverless](https://www.palantir.com/docs/foundry/functions/functions-deployed)
- Databricks: [serverless compute](https://docs.databricks.com/aws/en/compute/serverless/) · [environments](https://docs.databricks.com/aws/en/compute/environments-mode) · [base environments](https://docs.databricks.com/aws/en/admin/workspace-settings/base-environment) · [serverless environment versions](https://docs.databricks.com/aws/en/release-notes/serverless/environment-version/five) · [serverless GPU](https://docs.databricks.com/aws/en/compute/serverless/gpu)
- Snowflake: [Notebooks on Container Runtime](https://docs.snowflake.com/en/developer-guide/snowflake-ml/notebooks-on-spcs) · [compute setup](https://docs.snowflake.com/en/user-guide/ui-snowsight/notebooks-in-workspaces/notebooks-in-workspaces-compute-setup)
- AWS: [SageMaker Studio JupyterLab spaces](https://docs.aws.amazon.com/sagemaker/latest/dg/studio-updated-jl.html) · [task governance for interactive spaces with Kueue](https://docs.aws.amazon.com/sagemaker/latest/dg/task-governance.html)
- Google: [Colab Enterprise runtime templates](https://cloud.google.com/colab/docs/create-runtime-template) · [idle shutdown](https://docs.cloud.google.com/colab/docs/idle-shutdown)
- Kubeflow: [Notebooks 2.0 / Workspaces](https://github.com/kubeflow/notebooks/issues/85) · [culling](https://www.deploykf.org/guides/tools/kubeflow-notebooks/)
- Kueue: [concepts · workload](https://kueue.sigs.k8s.io/docs/concepts/workload/)
