# 0031 · El puesto

**Estado:** propuesto (escrito y medido el 2026-09-19; W3.1 hecho) ·
**Fecha:** 2026-09-19 · **Decide:** que W3 —«la sesión viva» de [`0030`](0030-el-arbol-en-el-editor.md)—
no es un kernel Python en un pod sino **el sustrato de ejecución** del que cuelga todo lo que no
es YAML: celdas Python y notebooks, funciones TypeScript y Java, SQL sobre datasets grandes,
inferencia, entrenamiento y fine-tuning; que ese sustrato tiene **una sola unidad, el puesto**
(imagen + identidad de la persona + datos + recursos) con **dos vidas** (sesión y trabajo); que
las dependencias se **declaran en el árbol y las resuelve CI** en una capa sobre imágenes base
**numeradas**; que el código lee y escribe datos **por un SDK con alias** y nunca por el
almacén; que un modelo es **un documento del árbol** con los pesos en el bucket; que la red del
puesto está **cerrada** y se abre por política con nombre; y que la cola gobierna las sesiones
con **prioridad alta y sin tomar prestado**. Sigue a
[`docs/investigacion/w3-estado-del-arte.md`](../investigacion/w3-estado-del-arte.md) y a
[`0029`](0029-donde-corre-una-funcion.md) (dónde corre una función).

## El problema

W0–W2 hicieron del workspace **el sitio** donde una organización lee, escribe, compila, ejecuta
(vistas) y propone su ontología. Todo lo que corre ahí hoy es YAML: `Run` sobre una `View` es el
motor de proyección de `ore-drivers` sobre el Parquet de la copia; una `Function` de inferencia es
un Job (`ore-invoke`, 0029 F4a). Lo que la organización quiere hacer **en el mismo sitio** es lo
que hace en cualquier otro entorno de datos: una celda Python que lee `hr.empleados` como
DataFrame; una función TypeScript que la consola invoca; un SQL que agrega 200 M de filas; adaptar
un modelo existente para que infiera sobre un conjunto; entrenar; fine-tunear. Todo a escala
enterprise, y todo **sin que ni una fila salga de la celda**.

Si se construye para la celda Python y luego se estira a lo demás, se paga tres veces. La
pregunta de 0031 es qué hay que fijar **antes** para que cada cosa de esa lista sea «otra imagen
y otra vida» y no «otro sistema».

## Lo mirado

[`w3-estado-del-arte.md`](../investigacion/w3-estado-del-arte.md): Foundry (Code Workspaces,
Code Repositories, Functions, compute modules), Databricks (serverless, base environments,
GPU), Snowflake (Container Runtime), SageMaker (spaces; Kueue en HyperPod), Colab Enterprise,
Kubeflow Workspaces. Nadie hace algo estructuralmente distinto de lo que teníamos escrito; lo
que hacen todos y no teníamos está abajo con su nombre.

## Decisión

### 1 · La unidad: el puesto, con dos vidas

Un **puesto** es *imagen + identidad de la persona + datos + recursos*. Existe en dos vidas:

| | **sesión** | **trabajo** |
|---|---|---|
| qué | viva, con estado, **una persona**, un nodo | un Job de Kueue que corre una cosa y muere |
| para | el notebook, el REPL, probar, mirar | una invocación, un entrenamiento, una compilación, un SQL grande |
| vida | TTL de inactividad (**30 min**) y tope (**24 h**) | `activeDeadlineSeconds` por clase de trabajo |
| escala | **no crece**: la sesión es de un nodo (Foundry, Snowflake) | lo grande se encola; el sabor decide el nodo |
| hoy | — | `49-la-invocacion.yaml`, `48-la-copia.yaml` **ya son puestos-trabajo** |

Una celda Python, una función TS, un fine-tune y un SQL son **lo mismo con distinta imagen y
distinta vida**. Lo que no cambia entre ellos: el `initContainer` que trae el testigo, la cuenta
`driver` con Workload Identity, el emptyDir de trabajo, `deny-all` con sus excepciones con
nombre, la cola `cola` del inquilino.

### 2 · Un agente en el pod, y el código habla con el SDK

Dentro de cada puesto corre **el agente** (un sidecar nuestro, como el de Foundry: 0,25 vCPU):
obtiene y renueva el token de la persona (RFC 8693, `act` = el puesto), resuelve alias de datos
contra `ore-serve`, sube salidas al bucket y **hace polling de trabajo** en vez de aceptar
conexiones — casa con «sin entrada» y con el patrón *compute module*. El código del cliente no
sabe de pods, tokens ni buckets: importa `ore` y llama al SDK.

### 3 · Runtimes: imágenes base numeradas + una capa por paquete, resuelta en CI

- **Pocas imágenes base por lenguaje, numeradas**: `puesto-python:1`, `puesto-node:1`,
  `puesto-jvm:1` (las tres existen desde W3.4), con su `requirements`/lock **dentro de la
  imagen** (`/entorno-1.txt`; Databricks: entornos 1…5; Foundry: `hawk.lock`). Nunca `latest`.
  Lo que una celda importa hoy importa igual en un año. **Un puesto por persona y entorno**
  (`puesto-<persona>-<entorno>`): la sesión es de una imagen; `sql` corre en cualquiera.
- **Las dependencias se declaran en el árbol** (`pyproject.toml`, `package.json`, `pom.xml` del
  paquete) y **la plataforma las resuelve en una capa** sobre la base. ⭐ Medido antes de elegir
  el cómo (W3.2, `medida-w3-la-capa.py`): la capa **no es una imagen en el registro** sino una
  **caja de ruedas en el bucket** (`ore/puesto/<capa-digest>/`) que un Job del driver resuelve
  para el intérprete del entorno 1 y que el puesto instala al arrancar en un `emptyDir` sin
  alcanzar PyPI (5 s: bajar 1 s + instalar 3 s). Hermética y reproducible igual, sin builder de
  imágenes ni permisos de registro por inquilino. Cambiar una librería es un cambio de código:
  pasa por commit, PR y checks; el informe `entorno/python.json` dice qué ruedas ganaron.
- `pip install` en la celda puede existir como **comodidad sin persistencia** (Snowflake) y sólo
  si la política de red lo abre; nunca es la verdad del entorno.
- `puesto-python:1` existe desde hoy (`Dockerfile` etapa 6, `cloudbuild.yaml`): los nodos de
  `jobs-p` no alcanzan Docker Hub, así que todo lo que corre ahí sale de nuestro registro.

### 4 · Datos: un plano (Arrow/Parquet en el bucket), un SDK con alias, y fallback de rama

- **Un plano de datos**: las copias (W1) ya son Parquet en el bucket del inquilino con sobre
  `ORECOPY1`. Datasets, salidas de trabajos y pesos de modelos van al mismo sitio, particionados,
  con su documento en el árbol. Interactivo = DuckDB/DataFusion en el puesto; masivo = un
  trabajo (motor distribuido cuando se mida; el contrato no cambia).
- **El SDK con alias**: `over("hr.empleados")` resuelve por la ontología a la copia vigente, con
  la identidad de la persona y **la potestad de `ore-iam`** (una vista que no puede leer, no
  llega al DataFrame). El código nunca ve el bucket ni una credencial.
- **Fallback de rama** (Foundry): una rama lee las copias de `main` mientras no tenga las suyas;
  una vista `materialized` nueva en la rama se copia en la rama.
- **Escribir es publicar una salida con nombre**: un dataset, una tabla, un modelo, una función
  — nunca ficheros sueltos. Es lo que hace que el linaje exista.

### 5 · Cómputo: sabores, cuotas y la política de sesiones

- Ya es Kueue: `ResourceFlavor jobs` (etiqueta, no pool), `ClusterQueue` por inquilino (10 CPU ·
  36 Gi hoy), `LocalQueue cola`. Se añaden **sabores `gpu` y `spot`** como etiquetas; el pool
  aparece cuando se mida y se pague; la política no se reescribe.
- **Sesiones**: prioridad alta (100 frente a 75 entrenamiento, 50 evaluación, 25 batch),
  **prestar cuota sí, tomar prestada no** (una sesión nunca corre sobre recursos reclamables),
  *preemption* dentro de la organización. Es la guía de Kueue para *interactive spaces*.
- **GPU concedida por cola**, no por pedirla: sólo un inquilino con cuota GPU puede pedir el sabor.

### 6 · Modelos: documentos del árbol, pesos en el bucket

Un kind `Model` (base, adaptador, versión, artefactos en el bucket, la `Function` que lo sirve).
Entrenar o fine-tunear es un **trabajo** que produce una versión; inferir es la `Function` de
0029 (`ore-invoke`). El registro de modelos es **git + bucket**, no una pieza nueva. **Publicar
desde la celda** es un snippet del SDK (`ore.models.publish(...)`), y el modelo está disponible al
instante para el resto (Foundry).

### 7 · Red: cerrada, con nombre, y sólo lectura sin exportación

- `deny-all` sigue; cada apertura es una `NetworkPolicy` con nombre (`salida-al-…`), como hoy.
  Un puesto alcanza: DNS, metadatos, el bucket (Google APIs), `ore-serve`, y **nada más** por
  defecto. Internet, sólo por política nombrada del inquilino (repos privados, un origen).
- **Modo *restricted outputs***: una sesión sobre datos marcados puede leer y **no puede
  publicar ni sacar**; se decide por política del inquilino sobre el dataset.

### 8 · El contrato consola ↔ puesto

`ore-serve` es la puerta: `POST /puestos` (abrir una sesión: imagen, sabor, rama),
`POST /puestos/{id}/ejecutar` (una celda: texto, lenguaje) → salida **tipada** (tabla, texto,
imagen, job, error) al panel de resultados que ya existe; `DELETE` cierra. El editor no sabe de
pods. Los trabajos van por la cola como hoy (Flux rinde el Job) y la consola los ve en Data › Jobs.

### 9 · El código, en tres lenguajes, con cuatro verbos (2026-09-19, tras W3.4)

Con la sesión viva en Python, TS y Java, lo que queda es **una sola superficie de código con
cuatro verbos** —leer, escribir, declarar, correr— y cada verbo con la misma semántica en los
tres lenguajes. Lo que no sea igual en los tres no entra. La promoción de lo escrito en código a
objeto de la ontología (una `Function` publicada, un `Model`) viene **después** de que el entorno
de código sea robusto por sí mismo; no se aborda todavía.

| verbo | hoy | lo que lo hace robusto |
|---|---|---|
| **leer** `over("p.v")`, `sql()` | en los tres, sobre las copias del bucket | **tipos consistentes**: hoy Python da pandas, TS objetos JSON, Java `List<Map>`. La verdad común es **Arrow** (la tabla Arrow de cada lenguaje, y de ahí a lo suyo): timestamps, decimales, nulos, enteros grandes y anidados sobreviven idénticos en los tres. Se mide: el mismo Parquet leído por los tres, campo a campo |
| **escribir** `write("p.salida", tabla)` | no existe | Parquet + sobre `ORECOPY1` al bucket, **con nombre**, y un informe en el árbol para que `over("p.salida")` lo lea desde cualquier lenguaje y sesión. Un dataset derivado es una copia (0027) que produjo código. Idempotente por digest; una escritura sucesora no borra la anterior (0017 §A) |
| **declarar** `transform(inputs, output)` | sólo la `Function` YAML (0029) | **en el código**, igual en los tres (decorador en Python, función en TS, anotación en Java): lo declarado es lo único que la sesión y el trabajo pueden leer y escribir; el resto, 403. Es el material del que después saldrá el documento |
| **correr** | la sesión (W3.1–W3.4) | **el trabajo**: un transform de un commit corre como Job de Kueue con la imagen de su entorno y su capa, lee y escribe lo declarado, deja el informe en el árbol y aparece en Data › Jobs. `Run` desde la sesión (bucle rápido) y como Job (el «build») |

Debajo de los cuatro, dos cimientos: **las dependencias en los tres** (`package.json`,
`pom.xml`/`build.gradle` como ya `pyproject.toml`: W3.4b) y **la rama** (una sesión o un trabajo
en una rama lee las copias de `main` mientras no tenga las suyas y escribe en la suya: §4).

Cotejo con Foundry: sus transforms son declarar+correr (con Spark debajo, que aquí entra sólo
cuando un dataset no quepa en un nodo); sus Functions son las de baja latencia; su Ontology es la
promoción que se deja para luego.

### 10 · Todo es un dataset (2026-09-20, tras medir Iceberg)

La distinción «copia = caché por digest, dataset = estado con historia» que 0032 esbozó al
aparcar Iceberg era una descripción de **cómo está hecha hoy la copia**, no una razón de
arquitectura. Foundry no la hace —un *sync* y un transform producen la misma cosa, un dataset
con transacciones— y §4 ya decía «un plano de datos». La regla, pues, es una sola: **dataset =
bytes en el bucket con historia, con un documento del árbol que los nombra y un puntero que
dice cuál es el estado vigente**. Lo que no tiene bytes en el bucket no es un dataset, por mucho
que sea una tabla.

| en ORE | documento (árbol) | bytes (bucket) | ¿dataset? | en Foundry |
|---|---|---|---|---|
| **`Table`** de una fuente (`olist.customers` en Postgres) | `Table` con `datasource: pg` | ninguno: un **puntero a un objeto de fuera** | **no** | una *source*: tampoco es dataset hasta que se sincroniza |
| **`View`** virtual | `View` | ninguno: una consulta | **no** | — |
| **`View` `materialized` → su copia** | la misma `View` + `copias/<p>_<v>.json` como **puntero** | una tabla Iceberg `datasets/<p>/<v>/{data,metadata}`; cada refresco, un snapshot | **sí** | el dataset de un *sync* (`SNAPSHOT`/`APPEND`) |
| **`write("p.salida", t)`** desde código | una `Table` con `datasource: lago` (nace con la primera escritura) + `datasets/<p>_<t>.json` como puntero | una tabla Iceberg, igual | **sí** | el dataset de un transform |
| **pesos de un modelo** (W3.9) | `Model` + puntero | un dataset de ficheros (prefijo con manifiesto) | sí, después | dataset no estructurado |
| lo que hay **hoy**: `ORECOPY1` | `View` + `copias/*.json` | un objeto sellado por digest | **heredado**: `over()` lo sigue leyendo; se reemplaza en la primera pasada con el escritor nuevo | — |

Lo que se sigue de la regla:

- **La `View` no se «convierte» en nada**: sigue siendo la declaración. Cambia lo que hay
  detrás: una tabla con snapshots en vez de un objeto que se reescribe entero. La consola ve lo
  mismo (`copiada · 71 234 filas`) y además la historia. **La `Table` de una fuente no cambia**:
  sigue apuntando fuera y sigue sin leerse sin copia (0030 W1).
- **Un lector**: `over("p.x")` resuelve por el nombre —View materializada → `copias/`, Table
  del lago → `datasets/`, lo demás → 409 «sin copia hecha»— y lee por el puntero con
  `iceberg_scan`, en los tres lenguajes. El recibo pasa a ser el puntero; el testigo del origen,
  una propiedad del snapshot; la cadena de sucesoras (0017), los snapshots; el digest, lo que
  siempre fue de verdad: la idempotencia («mismo testigo → ningún snapshot nuevo»).
- **Un linaje**: `salida ← transform ← copia de customers ← Table customers ← Postgres`, cada
  eslabón con bytes fechado por snapshot, y el gobierno bajando por el grafo que ya existe.
- **El coste medido** (0032): 0,5 s y 5 objetos por copia frente a 30 ms y 1; en 31 copias son
  15 s dentro de un Job que lee orígenes durante minutos, y los punteros van en el commit que el
  Job ya empuja. A cambio, el refresco incremental es un `append` en vez de fundir y reescribir.

Lo único que queda de la objeción es **de fases**: la copia la sella `ore-store` en Rust, y el
escritor de Iceberg en Rust hay que medirlo antes de sustituir el sellado. De ahí los peldaños
W3.5b–W3.6 de abajo, en ese orden: **primero el lector en el clúster** (todo cuelga de que
DuckDB lea Iceberg en GCS con la identidad del pod, sin internet), luego el escritor de la copia,
luego el swap, luego `write()`.

## Los peldaños de W3

| | qué | acepta |
|---|---|---|
| **W3.0** | medir el puesto (`medida-w3-el-puesto.py`) y esta decisión | los números de abajo |
| **W3.1** ✓ 2026-09-19 | la sesión Python: `puesto-python:1`, el agente (`puesto/python/agente.py`), `POST /puestos` → la cola → Flux, celdas por polling largo, salida tipada; rol `puesto` + `21-el-puesto.yaml` + `72-google-en-privado.sh`; en la consola, Run sobre un `.py` corre en el puesto y `CellListViva` para los notebooks | `el-puesto.sh` 1–5 en CI (el agente de verdad, `over()` sobre una copia ORECOPY1); en victor con rol `puesto`: la copia baja por `private.googleapis.com` en 257 ms y **pypi no contesta**; el agente real en el pod obtiene su token y habla con `ore-serve` |
| **W3.2** ✓ 2026-09-19 | las dependencias del árbol (`[project].dependencies` de `pyproject.toml`, raíz y paquetes) → **la capa**: un Job del driver (`52-la-capa.yaml`) resuelve para el entorno 1 y deja la caja de ruedas en el bucket (`ore/puesto/<capa>/`) y el informe `entorno/python.json` en el árbol; el puesto la instala al arrancar sin internet (`traer-la-capa` → `/capa`); `GET/POST /entorno`; abrir con la capa pendiente la encola y contesta 409 | medido (`medida-w3-la-capa.py`): resolver `polars` 1,8 s + subir 51 MB 1 s; el puesto la baja en 0,9 s y la instala en 3,2 s, `import polars` 160 ms, pypi no contesta; en demo, de punta a punta: el Job resuelve y empuja el informe, el puesto instala 182 MB en 3 s; `el-puesto.sh` 6 |
| **W3.3** ✓ 2026-09-19 (SQL) | SQL sobre el bucket **en la sesión**: `sql("select … from hr.espanoles")` (DuckDB en el puesto; cada `paquete.vista` tras FROM/JOIN se resuelve por ore-serve y se baja una vez); un `.sql` del árbol o una celda SQL van enteros a `sql()`. **Medido antes** (`medida-w3-el-sql.py`, 2 CPU · 3 GB): 200 M de filas → `count(*)` 5 ms, `group by` con agregados **1,9 s**, `where` 1 s, top-n 0,9 s; 1,4 GB al bucket en 11 s y de vuelta en 9,5 s. ⇒ un `count(*)` sobre 200 M **no necesita un Job**: cabe en la sesión con segundos de margen; el trabajo encolado queda para lo que no quepa en un nodo (disco de 50 GB, o más de un nodo) y para entrenar (W3.5) | `el-puesto.sh` 7 |
| **W3.4** ✓ 2026-09-19 (TS y JVM) | **Un puesto por persona y entorno** (`puesto-<persona>-<entorno>`: `python`, `node`, `jvm`; `POST /puestos {lenguaje}` elige la imagen; `sql` corre en los tres). `puesto-node:1` (node 24: los tipos de TS los quita Node, sin transpilador; `@duckdb/node-api`; el agente `puesto/node/agente.mjs` evalúa con el REPL de Node: contexto que dura, `await` arriba; una celda con `import`/`export` —un `.ts` del árbol— se escribe y se importa, y sus exports quedan en el contexto) y `puesto-jvm:1` (JDK 21 sobre noble; `puesto/jvm/ore/Agente.java`: JShell **en proceso**, varios snippets por celda, el valor de la expresión como objeto por `guarda()`, una clase con `main` se declara y se llama; DuckDB por JDBC). El SDK en los tres: `over()`, `sql()`, **`persona()`** (quién abrió el puesto). La plantilla del puesto lleva el hueco del entorno y cada imagen su `CMD`. En la consola, un `.ts`/`.js`/`.java` corre en su sesión; una fila por sesión abierta con su «Stop». **Medido antes** (`medida-w3-ts-jvm.py`): abajo | `el-puesto.sh` 8 (`saludo(persona())` → `hola persona:ana` desde un módulo TS con `export`; `over()`, `sql`) y 9 (lo mismo en Java, y una clase con `main`) en CI; en el clúster, las imágenes salen de `cloudbuild.yaml` |
| **W3.5** · leer | el verbo **leer**, consistente: Arrow como verdad común en los tres SDK; el contrato de tipos ORE ↔ Parquet ↔ lenguaje escrito y medido (`medida-w3-leer.py`: un Parquet con todos los tipos difíciles leído por los tres `over()`/`sql()`, campo a campo, y el caudal a 10 M de filas) | los tres lenguajes leen la misma copia y ven los mismos valores; lo que no sobrevive está dicho, no escondido |
| **W3.5b** ✓ 2026-09-20 · el lector del lago | **hecho**: `GET …/datos/{x}` contesta `metadata_location` (o `clave`, heredado); `over()`/`sql()`/`arrow()` en los tres leen por `iceberg_scan(raíz, version, allow_moved_paths)` —https con el token del pod en el bucket, ruta en local—, sesión en UTC, `autoinstall_known_extensions=false`, la cadena `json·icu·avro·iceberg(·httpfs)` cargada por nombre; las tres imágenes preinstalan las extensiones en `/opt/ore/duckdb` (JVM por `preinstalar/Extensiones.java`) y `node` lleva `ca-certificates`; `ore-serve` guarda la salida de una celda **tal cual** (`Json::Crudo`): `null` y `1.5` llegaban a la consola como cadenas; `el-puesto.sh` 4/8/9 leen `hr.lago` (PyIceberg, catálogo = árbol) con el mismo JSON en los tres. **Medido** (abajo): camino (b), directo del bucket con el token del pod; extensiones preinstaladas; node sin CA; una extensión ausente cuelga 120 s. Antes: **medir primero, en el clúster** (`medida-w3-lago.py`, con `jobs-p`): la extensión `iceberg` de DuckDB **preinstalada** en las tres imágenes (el pod no tiene internet; una extensión por versión de DuckDB: python 1.5.4, node-api 1.5.5, JDBC 1.5.5.1), y **cómo lee DuckDB una tabla Iceberg en `gs://` con la identidad del pod**: secreto GCS por HMAC de la cuenta del puesto, token del servidor de metadatos, o bajar los ficheros que el manifiesto lista (como hoy con el sobre); la latencia de `iceberg_scan` por el puntero desde un puesto. Luego el lector: `GET /puestos/{id}/datos/{x}` contesta `metadata_location` (o `clave`, heredado) y `over()`/`sql()` leen por él en los tres | medido y elegido el camino de lectura; `over("p.v")` lee una tabla Iceberg del bucket de victor desde Python, Node y Java con los mismos 23/23 de 0032 T3; el sobre heredado sigue leyéndose |
| **W3.6a** ✓ 2026-09-20 · la copia es un dataset | **hecho** (abajo, «W3.6a hecho»): `ore-store` escribe la tabla Iceberg en Rust —crear, refrescar fundiendo (sobrescribir), rehacer (sobrescribir), esquema que evoluciona con el lote, expirar y huérfanos— con el almacén de siempre como suelo de Iceberg y **sin catálogo**: el puntero `copias/<p>_<v>.json` (`metadata_location`, `cabecera`, `snapshot`, `testigo`) es el estado, `ore` lo lee antes de leer una fila y lo mueve al terminar, el commit del Job es el *swap* y la forja el CAS; el recibo del bucket se retira; `ore ask` e `ore invoke` leen por el puntero (el resultado de una función también es un dataset, `resultados/<p>_<f>`); `--recoger` expira lo superado (`ORE_RECOGER_EDAD` conserva la historia reciente; el Job, siete días); cotejado con DuckDB y PyIceberg en GCS y con `ore-store-r2` en R2. **Medido** antes: `iceberg-rust` 0.10 escribe lo que hace falta a 2,6 M filas/s con los diez tipos exactos, también en gs:// → **se construye en Rust**. Antes: **medir `iceberg-rust`** desde `ore-store` (`append` de 10 M, un catálogo como *trait* sobre el fichero puntero, tipos de 0032); si escribe, el Job de copia sella Iceberg; si no madura, PyIceberg en la imagen del Job mientras tanto. El puntero: `copias/<p>_<v>.json` con `metadata_location`, `snapshot`, `testigo` (el recibo del bucket se retira); rehacer = snapshot nuevo; `--recoger` = `expire_snapshots` + huérfanos; el refresco con clave = `append`/`upsert` en vez de fundir y reescribir | la pasada de copia de victor deja tablas Iceberg; `medida-w3-tipos.py` las lee; una copia rehecha y una refrescada son dos snapshots de la misma tabla; `over()` no distingue |
| **W3.6b** ✓ 2026-09-20 · el swap y el lago | **hecho** (abajo, «W3.6b hecho»): `POST /datasets/{ns}/{n}/confirmar {metadata_location, esperado, snapshot, filas, columnas}` → `ore datasets --confirmar` decide (CAS semántico: código 75 → 409 con `actual`; el `metadata.json` tiene que estar en el bucket; la `Table` del lago nace tipada con `columnas` en el mismo commit) y `ore-serve` empuja (CAS de la forja → 409, **también en la carrera de verdad**: `[remote rejected] … incorrect old value` era 502 y `git.rs` no lo conocía); `GET /datasets` y `GET /datasets/{ns}/{n}` (la ficha: snapshots con operación, filas, testigo, plan; el esquema de Iceberg; `ore-store historia`); el puesto resuelve una `Table` del lago por `datasets/<p>_<t>.json`; `ore init` declara `datasource: lago` (`LAGO_URL`, la raíz del bucket) y `confirmar` lo declara si falta; `53-el-mantenimiento.yaml`: un CronJob diario por inquilino con `ore datasets . --recoger --edad 7d` sobre punteros, sin compilar ni tocar orígenes. `el-lago.sh` 0–6. **Medido** antes (abajo). Antes: `ore-serve` hace el CAS sobre el puntero y el commit por la forja; el `datasource: lago` nace en el aprovisionador; la `Table` del lago se valida como cualquier tabla; la consola enseña la ficha del dataset con sus snapshots; un CronJob de mantenimiento | dos escritores concurrentes: uno confirma y otro recibe 409 y reintenta (**cuatro a la vez: uno gana, tres 409**); `git log` de un puntero es la historia de la tabla (`GET /arbol/historia/datasets/…`, tres versiones con quién) |
| **W3.6c** · escribir | `write("p.salida", tabla)` en los tres (Python con PyIceberg primero; Node y Java por DuckDB `COPY … TO` Iceberg cuando lo tenga, o por el trabajo): datos y `metadata.json` al bucket con la identidad del pod (`objectCreator` sobre `datasets/`), el puntero por `ore-serve`, la `Table` del lago escrita con el esquema de Arrow la primera vez; 0032 convierte lo que Iceberg no tiene (ns → µs, zona → UTC) y niega `uint64`/`null` diciéndolo | medido: 10 M de filas escritas desde cada lenguaje y leídas desde los otros dos, fidelidad campo a campo, caudal, latencia de un commit en el clúster |
| **W3.4b** · dependencias | las capas de Node (`package.json` → `node_modules` en el bucket) y de la JVM (`pom.xml`/`build.gradle` → jars en el bucket, resueltos con Maven en el Job del driver; nunca Gradle del cliente en la malla) | medido como la de Python: resolver, subir, bajar, cargar |
| **W3.7** · declarar y correr | `transform(inputs, output)` en los tres; el trabajo de código desde un commit (`ore run packages/p/transforms/x.{py,ts,java}`) con entorno + capa; fallback de rama; un `over` no declarado se rechaza | medido: frío del trabajo por entorno, un transform sobre 200 M de filas |
| **W3.8** · baja latencia | funciones TS y Python residentes (0029 ②): un proceso por función con sus vistas calientes, invocado por `ore-serve` | medido: p50/p99, memoria de las vistas calientes, arranque |
| **W3.9** · ML in situ | sabor `gpu`, entrenar y fine-tunear como trabajo, pesos al bucket con documento | un adaptador entrenado desde la celda sirve por una función |

## Lo medido (`pruebas-de-fuego/medida-w3-el-puesto.py`, 2026-09-19, victor)

Con `puesto-python:1` (python 3.12-slim · pandas 3.0.6 · pyarrow 25.0.1 · duckdb 1.5.5 · GCS),
un Job de Kueue en `cola` que se queda vivo, identidad `driver`, 1 CPU · 2 Gi:

| | medido | lo que dice |
|---|---|---|
| **el sitio** | `jobs-p` e2-standard-4 on-demand, 0–3, disco 50 GB, taint `ore.dev/jobs`; en `europe-west1-b` hay **L4, T4, H100 (80/mega), H200, B200, RTX PRO 6000** y máquinas `g2` (L4) y `a3` (H100) — consultado, nada lanzado | el sabor `gpu` tiene dónde aterrizar en la misma zona; L4 en `g2-standard-4` es el primer peldaño razonable |
| **frío** (pool a cero) | **110 s** hasta `Running` (el nodo; igual que 0027: ~100 s) | una sesión nueva con el pool a cero tarda casi dos minutos: o el pool tiene **min 1** en horario de trabajo, o la consola lo dice y enseña el progreso; no hay tercera |
| **caliente** (nodo e imagen ya estaban) | **1 s** hasta trabajar; python + `import pandas, pyarrow, duckdb` **1,0 s** | con el nodo caliente la sesión es instantánea; la imagen (`IfNotPresent`) se queda en el nodo |
| **la copia desde dentro** | listar `ore/v1/` 28 objetos **296 ms** · bajar la mayor **10,9 MB en 253 ms** · sobre `ORECOPY1` → Parquet → DataFrame **99 441 × 8 en 82 ms** (23 MB en memoria) · duckdb `count/distinct` **40 ms** | `over("…")` como DataFrame cuesta **medio segundo** de punta a punta con Workload Identity, sin credencial en el pod; el sobre se desenvuelve en tres líneas |
| **¿alcanza internet?** | **SÍ** — `pypi.org:443` en 37 ms | ⚠️ el puesto llevaba `ore.dev/rol: driver`, y `salida-del-driver` abre 0.0.0.0/0 (menos privadas) porque el driver lee orígenes; y `jobs-p` es privado pero **hay Cloud NAT** (`salida-a-origenes`). El puesto de sesión necesita **su propio rol** (`ore.dev/rol: puesto`) con DNS + metadatos + Google APIs por `private.googleapis.com` (199.36.153.8/30 con zona DNS privada: el «paso 2» de `20-driver.yaml`) y **nada más**. Sin eso, «no alcanza internet» es falso |
| **la cola** | las dos Workloads admitidas en `cq-victor` sabor `jobs`; un puesto vivo ocupa `cpu 1 · 2 Gi` de la cuota (10 · 36 Gi) mientras viva; `activeDeadlineSeconds` es el tope | Kueue trata un Job que no termina como cualquier otro: la sesión **es** un Job con tope; el TTL de inactividad lo tiene que poner el agente (terminar el proceso), no la cola |
| PodSecurity | el primer intento avisó `restricted:latest` (runAsNonRoot, seccomp) | la plantilla del puesto lleva `runAsNonRoot: true`, `runAsUser: 65532`, `seccompProfile: RuntimeDefault` |

**Con el rol `puesto`** (tras `21-el-puesto.yaml` y la zona `googleapis-en-privado`, mismo día):
frío **85 s** (nodo 56 s · pull de la imagen 18 s), caliente **3 s**; la copia **257 ms** por
`private.googleapis.com`; **`pypi.org:443` no contesta** (timeout). Lo que la política promete, medido.

**Lo que cambia en la decisión tras medir:** (a) el rol `puesto` y su política de red van en W3.1,
no después; (b) el frío de 110 s obliga a decidir `min 1` en horario o progreso visible — se
mide el coste de `min 1` (un e2-standard-4) frente a la espera; (c) el TTL de inactividad es del
agente, y el tope de sesión del Job.

## Lo medido para W3.4 (`pruebas-de-fuego/medida-w3-ts-jvm.py`, 2026-09-19, victor)

Antes de escribir los agentes: en local (node 22.14) y en el puesto (rol `puesto`, `jobs-p`,
imágenes de `mirror.gcr.io` — la malla las alcanza: node 81 MB en 9,7 s, temurin 184 MB en 7,5 s).

| | medido | lo que dice |
|---|---|---|
| **TS sin transpilador** | `stripTypeScriptTypes` 74 ms la primera vez, <2 ms después; node 24 tiene `process.features.typescript = "strip"` y **importa un `.ts` tal cual en 4 ms** | no hace falta `tsc` ni `esbuild` en la imagen: los tipos se quitan y las posiciones se conservan |
| **el REPL de Node como kernel** | `repl.start().eval`: `{ES:1,PT:1}` en 1,8 ms · una variable persiste (0,4 ms) · `const x = await …` queda en el contexto (11 ms) · `console.log` capturado por el `output` del REPL · un error síncrono **no llega al callback**: va al dominio del REPL (sin el oído propio, la celda no contesta nunca) | el mismo contrato que el kernel Python, con `await` arriba de regalo; el dominio se intercepta |
| **la JVM** | arranque **1,36 s** (incluye compilar el guion por el *source launcher*); JShell **local: 25 ms** de crear, primera celda 237 ms (carga clases), 22–70 ms después; error de compilación 24 ms; excepción 62 ms; el motor **remoto (JDI) 770 ms** sólo en arrancar | JShell en proceso y no otra JVM: la sesión es la misma JVM que el agente, y así el valor de una expresión se recoge como objeto (`guarda()`), no como `toString` |
| **las imágenes** (Docker Hub, comprimidas) | node:24-slim 81 MB · node:24-alpine 62 · temurin:21-jdk-alpine 184 · **temurin:21-jdk-noble 211** · jre-alpine 74 (sin `jdk.jshell`) · python:3.12-slim 43 | el JDK es obligatorio (JShell); noble y no alpine porque el JDBC de DuckDB no trae natives para musl; el pull de 211 MB cabe en el arranque en frío (el nodo tarda 56 s) |
| **el Job entero** | 84 s en frío (pool a cero) con los dos contenedores | igual que Python: el frío es el nodo, no el lenguaje |

**Lo que cambia tras medir:** (a) las imágenes no traen transpilador ni JVM aparte; (b) los
agentes de Node y de la JVM son *el mismo agente* (polling, salida tipada, TTL, 410) con el
kernel del lenguaje; (c) `persona()` entra en los tres SDK: es lo que hace que «la función del
árbol contesta con la identidad de la persona» sea una llamada y no una promesa.

**Lo que queda de W3.4:** la capa de Node (`package.json` → `node_modules` en el bucket) y la de
la JVM (`pom.xml`/Gradle → jars): hoy las dos imágenes nacen con lo que traen (DuckDB y el SDK)
y el árbol no declara dependencias para ellas; se mide como se midió la de Python (W3.2).

## Lo medido para W3.5 · leer (`pruebas-de-fuego/medida-w3-leer.py`, 2026-09-19, en local)

Un Parquet con 23 columnas de tipos difíciles (i8…i64 con 2⁵³+1, u64 máximo, NaN/inf/−0,
decimal(18,4) y decimal(38,10), timestamp sin zona/UTC/Europe·Madrid/ns, date, time, lista,
struct, map, binario, diccionario, todo-nulo) leído con los **SDK de verdad** por `over()` y
`sql()` de Python (pandas), Node (`@duckdb/node-api`) y Java (DuckDB JDBC), cotejado campo a
campo con pyarrow. Se perdona la *forma* (`T` o espacio, el desfase en que se pinta un instante,
el `toString` de una lista) y no el *valor*.

| | pierde | por qué |
|---|---|---|
| **Python · pandas** | `i64`/`u64` **con nulos → float64** (2⁵³+1 llega como …992); NaN → null al pasar a JSON; decimal(38,10) → float; map → lista de pares | pandas clásico no tiene enteros nulables ni decimales: es el `to_pandas()` por defecto. Con `types_mapper=pd.ArrowDtype` los conserva |
| **Node** | `i64`/`u64` llegan **como cadena** (valor exacto, tipo perdido); decimal(38,10) → número (pierde dígitos); map → `[{key,value}]`; −0 → 0 | el conversor JSON de DuckDB protege el valor con una cadena; un `bigint` de JS lo daría exacto y tipado |
| **Java** | timestamp **ns → 1970** y time → `00:00` (bugs del mapeo JDBC en `Ore.llano`); decimal(38,10) → double (`llano` lo convierte); NaN/inf → null; binario como `DuckDBBlobResult{…}`; **map → `{}`** | el camino `ResultSet.getObject` + un `llano` escrito a mano: cada tipo es un caso, y cuatro están mal |
| **los tres** | **decimal(38,10)** no sobrevive en ninguno; **NaN/inf** no viajan en JSON | un decimal exacto sólo viaja como cadena o como tipo decimal; JSON no tiene NaN |

**A escala (10 M filas · 4 columnas · 71 MB):** `over()` entero en **Python 755 ms (13 M
filas/s)**, **Node 34,6 s (0,3 M/s)**, **Java 15,7 s (0,6 M/s)**; `sql()` con `group by` en
61–141 ms en los tres (es DuckDB en todos). ⇒ El cuello no es leer Parquet ni DuckDB: es
**materializar filas como objetos** (`{col: valor}` por fila en JS, `Map` por fila en Java).
Python va 20–40× más rápido porque pandas es columnar.

**Lo que dice para el contrato (W3.5):**

1. **La verdad es columnar y Arrow**, no «filas de objetos». `over()` devuelve la tabla Arrow
   del lenguaje —pyarrow (con `to_pandas(types_mapper=pd.ArrowDtype)` o polars como comodidad),
   Arrow JS (`apache-arrow`: Int64 como `BigInt`, columnas tipadas) y Arrow Java (`VectorSchemaRoot`,
   que DuckDB JDBC exporta con `arrowExportStream`)— y las filas-objeto son una vista sobre ella,
   no la forma de entrega. Es lo que hace que 10 M de filas cuesten lo mismo en los tres.
2. **Un contrato de tipos escrito**: los tipos ORE ↔ Arrow/Parquet ↔ cada lenguaje, con lo que
   *no* sobrevive dicho: decimal viaja exacto (como decimal donde lo hay, como cadena donde no);
   un instante con zona es UTC; NaN/inf se dicen como `"NaN"`/`"inf"`; el binario en base64;
   map como objeto de claves-texto o lista de pares, una de las dos, igual en los tres.
3. **El JSON de la consola** (la salida `tabla` de los agentes) obedece ese contrato: hoy cada
   agente tiene su `llano` y los tres discrepan.
4. Arrow JS y Arrow Java tienen sus **costes** que hay que medir antes de meterlos en la imagen:
   Arrow JS no tiene decimales de verdad (cuatro `Uint32` sin aritmética) ni ns sin pérdida;
   Arrow Java pesa ~10 MB de jars y pide `--add-opens=java.base/java.nio`. Se mide con la misma
   matriz.

## Lo medido para W3.5b · el lector del lago (`pruebas-de-fuego/medida-w3-lago.py`, 2026-09-20, demo, `jobs-p`)

Un Job con **tres contenedores** —`puesto-python:1`, `puesto-node:1`, `puesto-jvm:1`, rol `puesto`:
sin internet— leyendo una tabla Iceberg de 10 M de filas y la de los 19 tipos que Iceberg
admite, escritas en el bucket de demo desde fuera (PyIceberg) y borradas al acabar:

| | python | node | jvm |
|---|---|---|---|
| DuckDB del enlace | 1.5.5 · linux_amd64 | 1.5.5 · linux_amd64 | 1.5.5 · linux_amd64 |
| `LOAD iceberg` + `httpfs` **preinstaladas por nombre** (`~/.duckdb/extensions/v1.5.5/linux_amd64/`: iceberg, avro, httpfs, json, icu) | ✓ 550 ms | ✓ 545 ms | ✓ 619 ms |
| **(b) directo de `gs://`**: token del servidor de metadatos como `BEARER_TOKEN` de un secreto HTTP + `iceberg_scan('https://storage.googleapis.com/<bucket>/<raíz>', version='<la del puntero>', allow_moved_paths=true)` · 10 M filas | count 482 · group by 753 · filtro 507 ms | 395 · 712 · 549 ms | 359 · 534 · 422 ms |
| **(c) bajar y leer**: la API JSON con el token (como hoy el sobre), 5 objetos, 39 MB, y `iceberg_scan` en local | listar 36 + bajar 618 ms · count 5 ms | 48 + 786 · 6 ms | — |
| los 19 tipos por (b), cotejados con la verdad | **19/19** | **19/19** | **19/19** |
| `INSTALL spatial` (una extensión que no está) | **se rinde a los 120 s**: la NetworkPolicy tira el paquete | | |

Lo que decide:

- **El camino de lectura es (b), directo del bucket con la identidad del pod y sin credencial
  nueva**: el token que el agente ya tiene, la API XML de GCS por `private.googleapis.com`,
  y DuckDB leyendo Iceberg con poda (el filtro sobre 10 M en 0,4–0,5 s sin bajar los 39 MB).
  (c) queda como lo que es: lo que hace el sobre hoy, y vale para una copia que se vaya a
  recorrer entera varias veces (bajar una vez, 0,6 s; luego 5 ms). HMAC (a) no hace falta.
- **El puntero basta**: a DuckDB se le da **la raíz de la tabla y la versión** (las dos salen
  del `metadata_location` del puntero) con `allow_moved_paths`; con el fichero directo
  resolvía `…/metadata.json/metadata/snap…` (404). Nada se lista.
- **Las imágenes cambian en tres cosas**: (1) preinstalar las cinco extensiones de la
  versión exacta de DuckDB de cada enlace al construir (`INSTALL` en el `Dockerfile`, que sí
  tiene red); (2) `node:24-slim` **no trae `ca-certificates`** y DuckDB (OpenSSL) no puede
  verificar a Google —en la medida se le dio el manojo de la imagen de Python por
  `SSL_CERT_FILE`; la imagen lo lleva—; (3) el SDK abre DuckDB con
  `autoinstall_known_extensions = false`: una extensión que falte tiene que fallar en el acto
  con su nombre, no colgar la celda **dos minutos** contra una red que no contesta.
- **Java lee por Arrow igual**: `arrowExportStream` es del `ResultSet`, venga de un Parquet
  local o de `iceberg_scan` por https; `Filas` y el JSON de 0032 T3 no cambian.

## Lo medido para W3.6a · iceberg-rust como escritor (`pruebas-de-fuego/medida-w3-iceberg-rust.py`, 2026-09-20, en local y contra el bucket de demo)

Un crate desechable (`pruebas-de-fuego/medida-w3-iceberg-rust/`, fuera del workspace) con
`iceberg` 0.10.1 + `iceberg-storage-opendal` (fs, gcs), cotejado con DuckDB:

| | resultado |
|---|---|
| **escribir 10 M de filas** (lotes de 1 M, `fast_append`) | Parquet 3,8 s (**2,6 M filas/s**, snappy, 70,7 MB, 1 fichero) + **commit 24 ms** (manifiesto, lista, `metadata.json`); DuckDB cuenta 10 M por la raíz y la versión en 8 ms |
| **append de 1 M** (snapshot 2) | 233 + 47 ms, **sin reescribir los 10 M**; DuckDB cuenta 11 M |
| **columna nueva** (`add_column`) | ✓ 21 ms; DuckDB la lee como `null` en las 11 M |
| **promover `int → long`** | **✗ no existe en 0.10** (sólo add/delete/rename). Hasta que lo tenga: un cambio de tipo en la copia es una tabla sucesora, como hoy |
| **expirar snapshots** | ✓ 19 ms, pero `retain_last(1)` solo no retira nada: 0.10 expira por edad (`expire_older_than_ms`) — el mantenimiento se escribe con las dos |
| **los diez físicos de 0032 §1** escritos desde Arrow | **10/10** exactos en DuckDB (bigint, double, boolean, varchar, decimal(38,18), decimal(10,2), date, time, timestamp, timestamptz). Un detalle: Iceberg nombra la zona `"+00:00"` y no `"UTC"` (misma física): el escritor la casa; y el lote tiene que llevar los ids de campo de Iceberg en el esquema de Arrow (`schema_to_arrow_schema` de la tabla) |
| **en el bucket** (opendal `gs://` con el token de gcloud; en el pod sería el del servidor de metadatos) | 1 M: escribir 1,3 s + commit 0,75 s desde esta máquina (4–5 PUT en serie); append 1,1 + 1,1 s; DuckDB lo lee de vuelta por https en 1,0 s; **10/10** tipos desde el bucket |
| **lo que pesa** | **298 crates** en el cierre (ore-store hoy: 100), binario **64 MB** sin `strip` (ore-store-gcs: 7,6 MB), y **arrow/parquet 58** frente al 56 del árbol: hay que subir `ore-store` |

**Lo que decide**: `iceberg-rust` **escribe lo que el Job de copia necesita** —tabla, append,
columna nueva, los diez tipos, GCS con la identidad del pod— al mismo caudal que el sobre de
hoy (2,6 M filas/s frente a 2,5). No hace falta PyIceberg en la imagen del Job. Lo que cuesta
es tamaño (crates y binario) y la subida de arrow a 58; lo que falta (promoción de tipos,
expirar por número) tiene rodeo y no bloquea. **W3.6a se construye en Rust, en `ore-store`.**

## W3.6a hecho · la copia sella Iceberg (2026-09-20)

Construido tal como la medida lo dejó, con tres decisiones que la medida no tomó y que se
tomaron al abrir `iceberg` 0.10 por dentro:

1. **Sin catálogo, y sin `opendal`.** `iceberg-rust` quiere un `Catalog` para confirmar y un
   `Storage` para escribir. El catálogo es el árbol —el puntero en `copias/`—, así que no hay
   ninguno: `ore-store` abre la tabla por su `metadata_location` (`Table::builder`), aplica los
   cambios a los metadatos (`TableUpdate::apply`, con sus requisitos comprobados) y escribe el
   siguiente `metadata.json`; **nadie lo apunta hasta que el Job hace `git push`**, y la forja,
   al rechazar lo que no avanza en línea recta, es el *compare-and-set*. El `Storage` es el
   `Almacen` de siempre (`gcs.rs`, `r2.rs`) con el traje de Iceberg: el mismo token del pod, el
   mismo SigV4, un transporte y no dos (la medida usó `iceberg-storage-opendal`; el producto
   no lo lleva). Una pasada que perdiera la carrera del push dejaría un snapshot que ningún
   puntero nombra, y `--recoger` lo retira en la siguiente.
2. **Sobrescribir, a mano.** 0.10 sólo trae `fast_append`. Un refresco (fundir lo que había con
   el incremento) y un rehacer producen un snapshot `overwrite` construido con las piezas
   públicas de la crate: un manifiesto con los ficheros nuevos como `ADDED`, otro con los vivos
   del snapshot anterior como `DELETED`, la lista de manifiestos, el `Snapshot` con sus totales,
   `AddSnapshot` + `SetSnapshotRef`. DuckDB y PyIceberg lo leen como lo que es (cotejado en
   GCS: vigente 1010 filas tras dos refrescos, `snapshot_from_id` del primero → 1000, tipos
   exactos, `sum(decimal)` exacto). El refresco con clave sigue siendo **fundir y reescribir**
   —3,8 s para 10 M según la medida— y no un *upsert* con *equality deletes*: eso es lo que
   0.10 no escribe todavía, y queda como mejora, no como bloqueo.
3. **El esquema evoluciona con el lote, por id.** No hay promoción de tipos en 0.10, y no hace
   falta: el esquema deseado se construye entero desde el lote, una columna que conserva
   nombre y tipo conserva su id y la que cambió de tipo (o es nueva) recibe uno nuevo — la
   forma legal de Iceberg para un cambio que no es promoción. Como cada sobrescritura
   reescribe todos los datos, ningún fichero vivo lleva un id con dos tipos, y los snapshots
   anteriores se leen con su propio esquema. Sin tablas sucesoras.

**El protocolo del almacén, revisado** (`ore-store <verbo>`, y 0015 queda enmendado por esto):
`buscar {metadata_location}` → `{existe}` (un HEAD: el puntero lo trae el árbol);
`sellar {dataset, base?, fundir, …cabecera}` + filas → `{metadata_location, snapshot, ubicacion,
operacion: creada|refrescada|sobrescrita, filas, bytes, ficheros, retirados, columnas,
sin_estrechar, esquema_cambiado}`; `recoger[-seco] {dataset, metadata_location, edad_ms?}` →
expira los snapshots superados más viejos que `edad_ms` (sin él, todos) y retira lo que ningún
snapshot que quede nombra, y devuelve el `metadata_location` nuevo si expiró alguno —**el
puntero se mueve a él**—; `recoger-huerfanas {datasets, claves}` → los datasets que ningún
puntero reclama y los sobres `ore/v1/` que ningún puntero nombra; `leer {metadata_location}`
(o `{clave}`, heredado) → la cabecera sellada (una propiedad del snapshot, `ore.cabecera`) y las
filas. `anterior` desaparece: sobre qué fundir y desde dónde leer lo dice el puntero.

**El ciclo en `ore materialize`**: ④ lee el puntero; si su `cabecera` (el digest de la de ahora)
coincide y el `metadata.json` existe, «ya está» sin leer una fila; ⑤ lee el incremento sólo si
el plan es el mismo, hay clave y el testigo ordena (`desde` = el testigo del puntero), y pide
`fundir`; con `--rehacer`, o con otro plan, o sin clave, lee entero y sobrescribe; ⑥ escribe el
puntero, y `--recoger` después (también en «ya está»). Sin `--informe`, los punteros viven en
`<árbol>/copias`. En seco no se toca el árbol. **Una pasada que falla no borra el puntero**:
conserva el dataset que había y pone encima `estado: error` y el motivo. Un puntero heredado
(`clave`, sin `metadata_location`) se trata como «no está»: la primera pasada resella como
dataset, y `recoger-huerfanas` retira el sobre cuando ningún puntero lo nombre.

**Lo que sale de las pruebas** (`la-pregunta-se-contesta.sh` 0–9 y `la-invocacion-se-decide.sh`
0–7 contra el S3 de mentira, `refresco.sh` contra GCS, `almacen-r2.sh` contra R2, los tres
verdes): crear = 4 objetos (`metadata.json`, lista, manifiesto, datos); un refresco = 5 (un
manifiesto más, el de los retirados); «ya está» = 0; `--recoger` deja el snapshot vigente y los
`metadata.json` que el registro de la tabla lista (acotado: `previous-versions-max`). Lo que
pesa: el cierre de `ore-store` pasa de 100 a ~300 crates y `arrow`/`parquet` a 58 (sólo ahí);
los binarios van con `strip` en la imagen.

**Lo que queda para W3.6b/c**: la política de retención de la historia como declaración (hoy
`ORE_RECOGER_EDAD` en el Job, siete días) y el CronJob de mantenimiento; el *upsert* con
*equality deletes* cuando `iceberg-rust` lo escriba; el swap por `ore-serve` para lo que no
escribe el Job (`write()`); la ficha del dataset con sus snapshots en la consola.

## Lo medido para W3.6b · el swap y el lago (`pruebas-de-fuego/medida-w3-swap.py`, 2026-09-20, en local con una forja pelada y el S3 de mentira)

| | resultado |
|---|---|
| **la carrera** (8 hilos, el mismo puntero, el mismo `If-Match`, `PUT /arbol/datasets/…`) | **1 gana y 7 pierden**, la forja tiene un commit más y el puntero es el del que ganó. Pero los 7 llegaban como **502 «git: To file://…»** y no como 409: bajo una carrera de verdad git no dice `non-fast-forward` sino `[remote rejected] … (incorrect old value provided)` (o `failed to update ref`, `cannot lock ref`), y `git.rs` sólo conocía la primera cara. Arreglado en la misma medida: 1 × 200, **7 × 409**, y los 7 reintentando con la cabeza nueva se serializan en **7 rondas exactas** (~1,5 s cada una: clonar + compilar dos veces + commit + push; para un puntero, compilar el árbol sobra) |
| **el `If-Match` de commit** | los 8 lo pasan (clonaron la misma cabeza): el CAS que decide es el de la forja. Para un dataset hace falta además el **semántico** —`esperado` = el `metadata_location` sobre el que se construyó— para contestar 409 sin empujar cuando el puntero ya se movió |
| **`datasource: lago`** | el tipo es abierto en OOS: `{ name: lago, type: lago, connectionEnv: LAGO_URL }` con una `Table` tipada encima y una `View` sobre ella **compila** (`ore validate` 0, `ore view` la traza como cualquier tabla). `materialize` de una View sobre el lago pide `ore-read-lago`, que no existe; `ore ask` dice «ninguna copia contesta»; `datos_de` del puesto sólo resuelve Views (`packages/<ns>/views`): **una Table del lago da 404** |
| **300 snapshots** sobre una tabla (`ore-store-r2`, una fila por snapshot) | `metadata.json` crece **~1 KB por snapshot** (1 → 283 KB) y el registro de metadatos se corta en 100; `leer` abre los 300 en 137 ms; `recoger` expira 299 y retira 1 395 objetos en 2,8 s, y el fichero queda en 16 KB (1 snapshot + 100 del registro) → la retención es por **edad**, y siete días de refrescos horarios (~170 KB) no pesan |
| **el barrido** | `ore materialize --recoger` al día tarda 443 ms y **le pregunta el testigo al origen** (26 ms de driver): un mantenimiento no puede pasar por ahí. El bucket de demo hoy: 3 objetos (la capa de un puesto) y ningún sobre; victor: 28 recibos y 10 sobres heredados que la primera pasada nueva resellará |
| **la historia** | `GET /arbol/historia/datasets/…` da las versiones del puntero (hash, autor, cuándo) y `GET /arbol/version/{hash}/…` cada una con su `snapshot`: **`git log` del puntero es la historia de la tabla**, sin nada nuevo |

**Lo que decide**: (1) el CAS es la forja y ya funciona; lo que faltaba era **decirlo** (409) y
un endpoint que lo haga por quien no puede empujar, con `esperado` semántico y sin compilar el
árbol; (2) el lago es una fuente que ya compila; lo que falta es **resolverla** en el puesto
(`datasets/<p>_<t>.json`) y que nazca con el árbol; leerla como origen de `materialize`/`ask`
(`ore-read-lago`) es otro peldaño; (3) el mantenimiento es **un verbo sobre punteros** —sin
compilar, sin tocar orígenes— con retención por edad, en un CronJob.

## W3.6b hecho · el swap y el lago (2026-09-20)

**`ore datasets`** (`crates/ore-cli/src/datasets.rs`): el verbo sobre punteros. Lista
(`copias/` y `datasets/`, con clase, estado, filas, `metadata_location`); `--ficha p.x`
(el puntero más `ore-store historia`: los snapshots del más nuevo al más viejo con operación,
filas, ficheros, bytes, añadidas/retiradas, plan y testigo; el esquema de Iceberg; el uuid);
`--recoger [--edad 7d] [--seco]` (por cada dataset `ore-store recoger` con la edad, y si expiró
algo **el puntero se mueve** al `metadata.json` nuevo; al final `recoger-huerfanas` con todos
los datasets y los sobres que algún puntero nombre); y **`--confirmar p.t --metadata-location
… [--esperado …] [--snapshot] [--filas] [--columnas {json}] [--sujeto]`**: el swap.

**El swap**, paso a paso: el paquete existe (422 si no); el `metadata.json` está en el bucket
(`ore-store buscar`, un HEAD; 422 si no); **el puntero es el `esperado`** —vacío si el dataset
nace— o **código 75** con `actual` (409 en `ore-serve`, y quién lo movió está en el árbol);
el mismo puntero otra vez es idempotente y no deja commit; `datasource: lago` se declara si
falta; la `Table` existe (y es del lago: una de otra fuente es 422) o **nace con `columnas`**
(tipos de OOS; 422 sin columnas; se compila el documento y se revierte si no compila); y el
puntero `datasets/<p>_<t>.json` con `estado`, `dataset`, `tabla`, `metadata_location`,
`snapshot`, `filas`, `escrito_por`. `ore-serve` (`datasets.rs`) clona, corre esto y empuja
(`escribiendo`): 201 si el puntero nace, 200 si se mueve, 409 en cualquiera de las dos caras
del CAS, 422/400 con lo que `ore` dijo. **`git.rs` aprendió la cara de la carrera**: `[remote
rejected] … (incorrect old value provided)`, `failed to update ref` y `cannot lock ref` son
`Adelantado` como `non-fast-forward`, y el mensaje es la línea del rechazo y no «To file://…».

**El lago como fuente**: `ore init` declara `{ name: lago, type: lago, connectionEnv: LAGO_URL }`
(la raíz del bucket, que no es un secreto; `48-la-copia.yaml` y `53-el-mantenimiento.yaml` la
llevan); el puesto (`datos_de`) resuelve una `View` con copia por `copias/` y una `Table` del
lago por `datasets/`; una `Table` de otra fuente es 409 «no un dataset» y lo que no está, 404.
Leer el lago como origen de `materialize`/`ask` (`ore-read-lago`, que el driver reciba el
puntero) queda para el peldaño que lo necesite.

**El mantenimiento**: `malla/53-el-mantenimiento.yaml`, un CronJob diario por inquilino (04:00,
`Forbid`, Kueue, SA `driver`, el testigo de la forja del almacén) que clona, corre `ore datasets
. --recoger --edad 7d` y empuja los punteros que se movieron. Sobre punteros: no compila el
árbol ni abre un origen (la medida: `materialize --recoger` al día le pregunta el testigo al
driver). Rendido por `gen-inquilino.py` con las demás plantillas.

**Probado**: `el-lago.sh` 0–6 (forja pelada + S3 de mentira + `ore-store-r2` de escritor):
la Table tipada y el puntero nacen en un commit del sujeto y el árbol compila; la lista y la
ficha; el CAS en sus cinco negativas sin dejar commit; **cuatro escritores a la vez: uno gana,
tres 409**; tres versiones del puntero con quién; recoger en seco no toca y de verdad retira
(34 → 8 objetos) y mueve el puntero. Unitarias: `datos_de` resuelve las dos clases; las tres
caras del adelantado en `git.rs`; edad, punteros y `asegurar_lago` en `datasets.rs`.

**Lo que queda para W3.6c**: `write()` en los tres lenguajes (datos y `metadata.json` al bucket
con la identidad del pod —hoy `driver`, `objectAdmin` sobre todo el bucket; una cuenta de
puesto con `objectCreator` sobre `datasets/` es del aprovisionador— y `confirmar` por
`ore-serve` con las columnas de Arrow según 0032); la ficha en la consola (`GET /datasets/{ns}/{n}`
ya la sirve); la retención declarada por dataset.

## Lo mirado para W3.6c · escribir (2026-09-20)

[`w3-escribir-estado-del-arte.md`](../investigacion/w3-escribir-estado-del-arte.md): el catálogo
REST de Iceberg (`requirements` + `updates`, `CommitStateUnknown`, `Idempotency-Key`,
credenciales prestadas), Delta 4.1 (*catalog-managed*), Foundry (transacciones), Nessie, lakeFS,
DuckLake, Polaris y BigLake, y los escritores por lenguaje (Java, PyIceberg, iceberg-rust,
DuckDB, iceberg-js, Icebird). **Valida** «el árbol es el catálogo y el swap es el commit»
(`confirmar {metadata_location, esperado}` es un `updateTable` con `assert-ref-snapshot-id`).
**Corrige** tres cosas para la spec del verbo: el 5xx/*timeout* es «mira antes de reintentar»;
una clave de operación para que la celda reejecutada no deje dos snapshots; la retención como
propiedades `history.expire.*` de la tabla y no como variable del CronJob. **Trae** dos ideas:
`ore-serve` hablando el catálogo REST de Iceberg (así PyIceberg, Java y DuckDB escriben sin SDK
nuestro, y Node —sin escritor de producción— manda Arrow al agente y escribe `ore-store`), y el
token acotado a la tabla (*Credential Access Boundary* de GCS) en vez de una cuenta de puesto del
aprovisionador. Lo que hay que medir está en su §4; la spec del verbo (un §11) viene después.

## Lo que se aparca

- El motor distribuido para lo masivo (Ray/Spark sobre la cola): el contrato (Parquet en el
  bucket + documento en el árbol + trabajo encolado) lo admite; se elige cuando haya un dataset
  que no quepa en un nodo.
- `restricted.googleapis.com` (el /30 con Cloud DNS privado): el paso 2 de `20-driver.yaml`.
- Sesiones compartidas (dos personas en el mismo notebook): SageMaker lo tiene; no antes de W3.1.
