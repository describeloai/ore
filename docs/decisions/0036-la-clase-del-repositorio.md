# 0036 · La clase del repositorio: productos dedicados sobre un solo árbol

**Estado:** resuelto (la visión; los siete pasos, en el entregable desechable [`docs/repositorio.md`](../repositorio.md)) · **Fecha:** 2026-09-22 ·
**Decide:** qué es la **clase** de un repositorio (`transforms`, `analytics`, `models`,
`functions`, `semantics`), qué varía con ella —entorno, capacidades, interfaz— y **dónde vive
cada cosa**, para que la partición por instancia sea limpia **desde el primer momento**. Sigue a
[`0035`](0035-el-proyecto.md) ⑥ (el repositorio es la unidad de trabajo),
[`0031`](0031-el-puesto.md) (dónde corre el código, y la capa) y
[`0027`](0027-el-modelo-en-el-arbol.md) (los perfiles de máquina).

## El problema

0035 ⑥ dejó dicho que el repositorio es **la unidad de trabajo** y que hay que persistir la
instancia. Falta lo que hace que esa partición **valga la pena**: cada clase de repositorio es un
**producto distinto**. Un `models` necesita una máquina con GPU y `torch`; un `functions` corre
en milisegundos y no escribe datasets; un `transforms` escribe y declara qué lee. **La
configuración varía, las capacidades varían y la interfaz varía.** Si la partición no se fija
ahora, cada una de esas variaciones se colará por donde pueda —una bandera aquí, un `if` allá— y
el día que haya cinco productos no habrá forma de separarlos.

Y hay una razón medida para no esperar: **hoy el entorno es del árbol entero**. `entorno.rs` lee
`pyproject.toml` de la raíz **y de cada paquete**, une las dependencias, y de esa unión sale
**una** capa (`capa-<digest>`) para todas las sesiones de la celda. Es decir: el día que un
repositorio de modelos declare `torch`, **todas** las sesiones del cliente —las de análisis, las
de funciones— arrancarán bajando `torch`. El entorno único no es una incomodidad de pantalla: es
un acoplamiento que crece con el cliente.

## Lo abstraído: una instancia son tres cosas, y dos se derivan

> **Una instancia = (el sitio, la clase, el nombre).** Todo lo demás **se deriva**, y por eso no
> hay nada más que persistir.

| | qué es | qué se deriva de ello |
|---|---|---|
| **el sitio** | la carpeta: `packages/<paquete>/<carpeta>` | qué ficheros son suyos, quién los tocó (`git log -1 -- <carpeta>`), qué ítems del índice caen dentro, qué propuestas la tocan, qué sesión y qué rama le corresponden |
| **la clase** | `plantilla: transforms` | **el entorno** (lenguaje, dependencias, máquina), **las capacidades** (qué puede hacer su sesión) y **la interfaz** (qué enseña la consola, y con qué se siembra) |
| **el nombre** | «New Pipelines Java Transform» | cómo se llama para las personas, y nada más |

**El sitio es la clave de partición**, y es lo único que no puede cambiar de idea después: todo
lo acotado —la capa, el puesto, la rama, las propuestas, el editor, el diagnóstico— **toma la
ruta como parámetro**. Si algo acotado no se puede expresar como «esto, para este prefijo», es
que está mal puesto.

## Lo simplificado: la clase es una clave, no una configuración

La tentación es que el manifiesto lleve la configuración del repositorio: imagen, dependencias,
memoria, permisos. **No.** El manifiesto lleva **la clase** (y su versión), y la clase es una
entrada de **una tabla del producto**. Tres consecuencias, y las tres simplifican:

1. **La configuración que ya tiene sitio se queda donde está.** Las dependencias de Python van
   en el `pyproject.toml` **del repositorio** —donde el ecosistema las pone—, no en un YAML
   nuestro. Lo único que cambia respecto de hoy es **dónde se mira**: la capa deja de ser la
   unión del árbol y pasa a ser la del repositorio (su `pyproject.toml`, más el de su paquete y
   el de la raíz, que siguen siendo comunes a propósito).
2. **La clase se versiona, y por eso se puede actualizar.** `plantilla: transforms` +
   `plantillaVersion: 3`. La columna **«UPGRADE · Up to date»** de la pantalla de repositorios
   —la que parecía decorativa— es exactamente esto: comparar la versión que el repo declara con
   la que el producto trae, y ofrecer la subida. Es lo que Foundry hace con *upgrade PRs*.
3. **Lo que la clase no puede es ampliar poder.** Una clase **ajusta hacia abajo**, nunca hacia
   arriba: puede decir «este repositorio no escribe datasets», y no puede decir «éste sí puede
   saltarse el conducto». La concesión sigue negando, no concediendo — y lo que gobierna sigue
   siendo la etiqueta, la declaración y quién escribió (0035 ⑤).

### Las tres caras de una clase, y dónde vive cada una

| cara | qué decide | dónde vive | qué ya existe |
|---|---|---|---|
| **el entorno** | lenguaje, dependencias, máquina | la declaración del repo (`pyproject.toml`) + la tabla de clases | `entorno.rs` (declaración → digest → capa en el bucket) y los **perfiles** de 0027 para la máquina |
| **las capacidades** | qué puede hacer su sesión: leer, escribir datasets, entrenar, publicar funciones | la tabla de clases, como **techo** | la puerta del agente (W3.7 ①) y `@transform(inputs, output)` (W3.7 ⑤), que ya acotan por declaración |
| **la interfaz** | pestañas, acciones, iconos y **la semilla de ficheros** | la consola, con **la misma clave** (`plantilla`) | el `BuildPicker` ya tiene las cinco tarjetas |

Las cinco clases de hoy, con lo que las distingue de verdad:

| clase | entorno | capacidades | qué deja en la ontología |
|---|---|---|---|
| **transforms** | Python/SQL/Java, CPU | lee lo declarado, **escribe su `output`** | un `Dataset` escrito |
| **analytics** | Python, CPU | **sólo lee** | nada: leer no declara |
| **models** | Python, **máquina con perfil** (0027) | lee, entrena, publica un modelo | un `TrainedModel` |
| **functions** | Python/TS, CPU, latencia baja | lee lo suyo, **no escribe datasets** | una `Function` |
| **semantics** | sin runtime | **no ejecuta**: edita documentos | `Entity`/`View`/`Concept` |

Dos de las cinco (`analytics`, `semantics`) **no necesitan nada nuevo**: son el caso de hoy con
menos permisos. Las otras tres son las que pagan la partición.

## Lo cotejado con las mejores prácticas (2026-09-22)

| quién | qué hace | qué nos dice |
|---|---|---|
| **Foundry · Code Repositories** | Los repositorios tienen **tipo**: los de *Transforms* traen previsualización y depuración de transforms; los de *Functions*, acceso nativo a la ontología y ejecución de baja latencia; los de modelos se crean desde plantillas (*Model Adapter Library*, *Model Training*). Y Foundry **genera PRs de actualización** contra los repos activos con mejoras de la plantilla y del runtime, que pueden fundirse automáticamente | la clase **es del producto**, no del cliente; **se versiona y se actualiza sola** (nuestra columna «Up to date»); y el tipo cambia de verdad **las capacidades y la interfaz**, no sólo un icono |
| **Backstage** | Un repositorio lleva un `catalog-info.yaml` con `spec.type` (`service`, `documentation`…), y el *scaffolder* **siembra el repo y registra la entidad** en el catálogo | exactamente nuestra forma: **un fichero en el sitio que declara qué es**, y un catálogo que lo lee. Confirma que el manifiesto va **dentro** del repositorio, no en un registro aparte |
| **Dev Containers** | `.devcontainer/devcontainer.json` **por repositorio** describe el entorno reproducible; las *Features* son unidades componibles de instalación y un `.devcontainer-lock.json` **fija sus versiones** | la configuración va **junto al código**, y se **bloquea** para reproducir. Nuestra capa con su digest (`capa-<12 hex>`) es eso mismo, y **por repositorio** es como debe estar |
| **dbt, Databricks bundles** | un `dbt_project.yml` / `databricks.yml` **por proyecto**, con su entorno y sus objetivos | la unidad de configuración del ecosistema es **la carpeta del proyecto**, no el almacén entero |

**Lo que no copiamos, y por qué.** Foundry paga la separación con **un repositorio git por cosa**;
nosotros mantenemos **un árbol, un índice, un linaje** (0033, 0034, 0035 ⑤). Así que nuestra
partición es **por prefijo**, no por repositorio: todo lo acotado toma la ruta como parámetro. Se
gana que el linaje cruce repositorios sin importar nada; se paga que **acotar hay que hacerlo
bien en cada plano**, porque nadie lo hace por nosotros.

## La resolución

> **Un repositorio es una instancia de una clase, sobre una carpeta del árbol.** La carpeta es
> la partición; la clase es el producto; el nombre es para las personas.

1. **La identidad se fija ahora**: la ruta de la carpeta. Todo lo acotado se deriva de ella y se
   expresa como «esto, para este prefijo».
2. **El manifiesto guarda tres cosas y ninguna más**: `nombre`, `plantilla` y `plantillaVersion`.
   Ni imagen, ni memoria, ni permisos: la configuración vive donde el ecosistema la pone
   (`pyproject.toml`) y la clase vive en el producto.
3. **La capa se acota al repositorio**, que es el cambio con más valor por línea: hoy el
   `torch` de un repositorio de modelos lastra **todas** las sesiones de la celda.
4. **Las capacidades son un techo por clase**, aplicado donde ya se aplica el gobierno (la puerta
   del agente y la declaración del transform). Una clase **nunca concede**.
5. **La interfaz se resuelve por la misma clave** en la consola: una tabla de clases, no un `if`
   por pantalla.
6. **La clase se actualiza**: `plantillaVersion` contra la del producto, y una propuesta —no un
   commit a la brava— para subirla. La columna «Up to date» dice la verdad o no se enseña.

## Lo construido

### ① La instancia en el índice (`ore_core::repositorios`)

**Lo común, factorizado**: `manifiesto.rs` (nuevo) tiene el encabezado —`encabezado`, `campo`,
`lista`— y lo usan **el proyecto y el repositorio**. Era la tercera vez que se escribía el mismo
`---`…`---`, y copiarlo habría sido tener dos sitios donde arreglar el mismo error.

`repositorios::leer(raiz)` recorre `packages/<pkg>/**/README.md` y se queda **los que dicen
`plantilla`**. Cuatro reglas, y cada una es una decisión:

- **La clave `plantilla` es lo que hace un repositorio.** Un README sin ella es **una carpeta con
  README** —las que `ore init` deja en cada directorio— y no se lista: no es un error, es lo
  normal en un árbol.
- **El README del propio paquete no lo es.** Un repositorio es un sitio **dentro**; hacer del
  paquete entero uno borraría la diferencia entre «el paquete» y «donde se trabaja».
- **Lo roto se lista con su porqué** (`sin `nombre``) y **no se queda ningún ítem**. Misma regla
  que el proyecto y que una relación `rota: true`.
- **`plantillaVersion` se lee como número** y viaja tal cual: es lo que ⑥ comparará.

**En el índice**: `repositorios: [{ruta, nombre, plantilla, plantillaVersion, paquete, carpeta,
items, manifiesto, version, roto?}]` en la raíz, y en cada ítem **`repositorio` en SINGULAR** —o
`null`—. Es el contraste con `proyectos`, que es plural, y no es un capricho: **un proyecto es
una lente y se solapa; un repositorio es el sitio donde se trabaja**, y dos anidados no se
reparten un ítem — **se lo queda el más hondo**, que es donde alguien lo está tocando.

La prueba (`tests/assets.rs`): cuatro READMEs sobre el árbol de fuego —uno repositorio, otro
anidado dentro, uno roto y uno que no lo es porque no dice `plantilla`—, la lista con su clase y
su versión, `view:ventas.pedidosEs` en **su** repositorio y `dataset:ventas.pedidos` en
**ninguno**, y **los ítems sin cambiar**. Más 6 unitarias en `repositorios.rs`.

Y sobre el árbol de verdad (demo `b93ed52`), con dos repos anidados y una carpeta con README:

```text
2 repositorios
  packages/olist/raw                       transforms     0 ítems · New Pipelines Java Transform
  packages/olist/raw/modelo                models         0 ítems · Churn Model
```

`ore validate` sigue saliendo **0**. **Medida** §3: «0 carpetas de cliente» → las que se creen.

### ② Los verbos: nacer entero, y el sitio que ya está cogido

**Dónde**: `crates/ore-core/src/clases.rs` (la tabla) y `crates/ore-serve/src/repositorios.rs`.

**La tabla del producto, estrenada**: `clases.rs` trae las cinco con su `titulo`, su `version` y
su **semilla** —`transforms/ejemplo.py`, `analisis/ejemplo.py`, `modelos/entrenar.py`,
`funciones/ejemplo.py`, y `semantics` **sin semilla**, porque lo suyo son documentos del árbol y
sembrar una `Entity` a medias sería sembrar algo que no compila—. La tabla **no está en el
árbol**: si lo estuviera, cada cliente tendría su versión del producto y «actualizar la
plantilla» no querría decir nada. (El **techo** de capacidades y el perfil de máquina son de ⑤;
aquí la tabla sólo nombra y siembra.)

**`POST /repositorios {paquete, carpeta, nombre, plantilla, proyecto?}`** → 201, y:

- **nace entero**: el manifiesto **y la semilla** en **un solo commit** —una plantilla que deja
  los ficheros a medias no es una plantilla—;
- con `proyecto`, su `contiene` se actualiza **en ese mismo commit** (y **conservando la prosa**
  del proyecto): «creado pero no nombrado» es un estado que nadie pidió;
- **409** si esa carpeta ya es un repositorio: dos instancias sobre la misma carpeta serían dos
  sesiones y dos ramas sobre los mismos ficheros;
- **422** si la clase no está en la tabla, **con las que sí están** — así la consola no puede
  ofrecer algo que el servidor rechazaría;
- **404** si no hay paquete: un repositorio vive dentro de uno.

**`PUT /repositorios/{ruta}`** reescribe el manifiesto y **conserva la prosa**: lo que una
persona escribió no lo borra un `PUT`. **404** si esa carpeta no es un repositorio — crear es
`POST`. Y **borrar no estrena verbo**: `DELETE /arbol/<carpeta>` (0035 ③b) se lleva la carpeta
entera en un commit y **el manifiesto se va con ella**.

En ore-serve, el `version` de un repositorio sale de `git log -1 -- <carpeta>`, como el de un
fichero: es el «last edited by» de la lista, sin inventar nada (§2 lo midió en 59–60 ms).

La prueba (`los-documentos.sh` 22, contra un `ore-serve` de verdad): crear deja **un solo commit**
—y `git show --name-only` enseña dentro el manifiesto, la semilla y el README del proyecto—;
`/assets` trae el repositorio con su clase y con quién lo creó, y el proyecto ya lo nombra;
repetir la carpeta es 409 **sin commit**; una clase inventada es 422 **con la lista**; `PUT`
renombra y la prosa sigue; **desde un puesto es 403**; y borrar la carpeta se lleva el
manifiesto y el índice deja de traerlo.

### ③ La capa por repositorio — el acoplamiento que se rompe

**Dónde**: `crates/ore-serve/src/entorno.rs`, `cola.rs`, `puestos.rs`, `rutas.rs` y
`malla/52-la-capa.yaml`.

`declaracion_en(raiz, alcance)`: sin alcance, la unión de la celda —la raíz y todos los
paquetes, como siempre—; con alcance `packages/<p>/<carpeta>`, **la raíz, su paquete y cada
nivel hasta él**. Lo de la raíz y lo del paquete siguen siendo comunes **a propósito**: lo de
todos, para todos. Lo que deja de ser común es **lo de al lado**.

El alcance viaja por **cabecera** (`X-Ore-Raiz`) en `GET`/`POST /entorno`, y en el cuerpo de
`POST /puestos` (`{repositorio}`) — nunca por la URL: ningún dato entra por ahí. Un alcance que
no sea la carpeta de un paquete es **422**; una que no exista, **404**.

Y **el informe pasa a ser uno por digest** (`entorno/<digest>.json`): dos alcances son dos capas,
y un fichero único haría que la segunda borrara a la primera. El de antes
(`entorno/python.json`) se sigue leyendo —un árbol ya resuelto no tiene por qué volver a
resolverse— **sólo si habla del mismo digest**. El Job de la capa (`52-la-capa.yaml`) gana
`ALCANCE` y suma los mismos ficheros que el servidor.

Medido después, contra un `ore-serve` de verdad (§6 de la medida):

```text
GET /entorno sin alcance (la celda)   ['duckdb','polars'] → capa-fd59afa41442
  el repositorio de modelos           ['duckdb','polars','torch'] → capa-cafe46ed3e46
  el de análisis, al lado             ['duckdb','polars']         → capa-fd59afa41442
  ¿carga con el `torch` del vecino?   NO · dos alcances, dos capas
```

Y el matiz que la medida deja claro, y que es **peor** de lo que la ADR decía: sin alcance, el
`pyproject.toml` de un repositorio **ni siquiera se lee** —`torch` no aparece en la declaración
de la celda—. Es decir, hasta hoy un repositorio **no podía declarar nada**: sus dependencias
tenían que subir al paquete o a la raíz, y **ahí las baja todo el mundo**. Lo que ③ arregla no
es sólo que no pesen: es que **puedan existir donde tienen que existir**.

### ④ Lo acotado: el editor, la sesión, la rama y las propuestas

Tres cosas, y ninguna cara — como la medida prometía:

- **El editor** (`arbol.rs`): `GET /arbol` acepta `X-Ore-Raiz: packages/<p>/<carpeta>` y devuelve
  **sólo lo suyo**, diciendo en `raiz` a qué se acotó. **La cabeza no cambia**: es la del árbol,
  porque el árbol es uno — se acota **qué se lista**, no de qué commit se habla.
- **La sesión** (`puestos.rs`): `id_de(persona, entorno, repositorio)` toma **la última carpeta**
  del alcance, que es como se llama el repositorio para quien trabaja, y la rama por defecto pasa
  a `<persona>/<repo>`. Sin repositorio, todo como antes.
- **Las propuestas** (`propuestas.rs`): con `X-Ore-Raiz`, sólo las que **tocan sus ficheros**. Se
  mira **los ficheros y no el nombre de la rama**: una rama se llama como quien la abrió quiera,
  y la pregunta es «¿esto cambia lo mío?». Cuesta una llamada por propuesta, así que **sólo se
  paga cuando se pide**, y si la forja no sabe decir qué ficheros toca una, **no se esconde**:
  más vale enseñar de más que callar un cambio que sí es tuyo.

Un `POST /trabajos` **sigue sin repositorio**: no es una sesión, corre y termina. Acotarlo por la
carpeta de su fichero sería otra decisión, y se toma cuando una medida la pida.

**Medida §4, antes y ahora**:

| | antes | ahora |
|---|---|---|
| `GET /arbol` | **24 ficheros** de la celda; del repo, 1 | con `X-Ore-Raiz`, **sólo los suyos**, y `raiz` dicha |
| la cabeza | — | **la misma**: el árbol es uno |
| dos repos, misma persona | **el mismo** `puesto-ana-python` | **dos**: `puesto-ana-python-raw` y `…-clean` |
| la rama | `ana/puesto` para los dos | `ana/raw` y `ana/clean` |
| `/propuestas` | **no filtraba** | filtra por ruta, y dice el `alcance` |
| un alcance inválido · uno que no existe | — | **422** · **404** |

La prueba (`los-documentos.sh` 23): dos repositorios en el mismo paquete; el acotado trae el
manifiesto y la semilla **y nada más**, el de al lado ve lo suyo, sin cabecera vuelve la celda
entera, y los dos errores salen con su código.

### ⑤ La clase: el techo y la versión

**El techo** (`clases.rs` gana `escribe`, `ejecuta`, `perfil`): una clase **quita, nunca
concede**. `transforms` y `models` escriben datos; `analytics` y `functions`, **no**;
`semantics` **ni siquiera ejecuta**. Y se aplica en dos sitios, los dos **donde ya se decide
quién escribe**:

- al **abrir**: pedir un puesto en un repositorio `semantics` es **422** con el porqué —lo suyo
  son documentos del árbol—, y abrir un pod para eso sería abrirlo para nada;
- al **escribir**: el catálogo (`/v1`, todo lo que no es `GET`) y `datasets/…/confirmar` miran la
  clase del puesto y contestan **403**. En el servidor, **no en el SDK**: lo que se comprueba en
  el cliente se rodea pidiendo a pelo, y eso ya se midió en W3.7 ⑤.

El puesto recuerda **dónde vive** (`repositorio`) y **de qué clase es**, y su ficha lo dice
(`plantilla`, `escribe`): quien mire la sesión ve el techo que tiene.

**La versión**: el índice añade a cada repositorio `plantillaActual` (la del producto) y
`actualizable` (`plantillaVersion` < la actual). Es lo que hace verdadera la columna
**«UPGRADE · Up to date»**, y subirla será **una propuesta con su diff**. Una clase que este
producto no conoce **no rompe nada**: se lista, con `plantillaActual: null` — el árbol de un
cliente puede venir de una versión posterior.

La prueba de fuego (`el-puesto.sh` **17**, con agentes de verdad y sin clúster): se crean un
`analytics` y un `semantics`; el `semantics` **no abre sesión** (422 «no ejecuta»); el
`analytics` abre, su ficha dice `plantilla: analytics · escribe: false`, **lee** (`datos` →
200) y **no escribe**: `POST /v1/namespaces/hr/tables` a pelo es **403**, y un
`@transform(inputs, output)` que lo declara **tampoco** escribe — y **no queda puntero**. El
script entero sigue verde (1–17, python, node y jvm).

Y §7 de la medida, en local: `transforms` → puesto con `escribe: true`; `analytics` → `escribe:
false`; `semantics` → 422; y los tres repositorios con `v1` frente a la `v1` del producto,
`actualizable: false`.

### ⑥ La consola (rubix-platform, commit local `1ff602e`)

- **La lista** (`/repositories`): cada fila es **una instancia**, del índice. Nombre y ruta, el
  **icono por clase**, «last edited by» y «last edited» **de git**, y la columna **UPGRADE**, que
  dice la verdad o no se enseña: compara `plantillaVersion` con `plantillaActual`, y si el
  producto **no conoce** la clase dice «—», no «Up to date». Abrir una fila abre el workspace
  **acotado**; el menú ofrece **subir la plantilla** y **borrar** (la carpeta entera, en un
  commit, avisando antes).
- **«Save» escribe**: una llamada y un commit (manifiesto + semilla + el `contiene` del
  proyecto), y después `/workspaces?repositorio=…`. La ubicación sale **del árbol**: lo que el
  proyecto nombra, o los paquetes de la celda; **una carpeta vive dentro de un paquete**, y con
  más de uno **se elige**. El nombre **es la carpeta** —su alfabeto ya cabe en el del servidor—,
  y un 409 se enseña sin perder lo escrito.
- **El árbol acotado llega a la consola**: `X-Ore-Raiz` viaja **por cabecera** desde `query.ts`
  (ningún dato entra por la URL) y el editor abre su carpeta; la cabeza sigue siendo la del árbol.
- **El detalle del proyecto** deja de pintar `SAMPLE_PROJECT_FILES`: `contenidoDelProyecto` cruza
  lo que el proyecto **nombra** con lo que el árbol tiene —sus carpetas, sus repositorios y sus
  ítems—, y **el `id` de cada entrada es su ruta**, así que abrir una carpeta es abrir esa ruta y
  borrarla es borrar esa ruta. Crear una carpeta **escribe**; y **«Move to trash» deja de ser una
  mentira**: no hay papelera, es un commit que se lleva la carpeta entera y lo dice antes.
  Favoritos y «compartido conmigo» llegan **vacíos siempre**: en un árbol no existen, y lo que no
  se hace es fingir que guardan algo.

**Lo que queda fuera de ⑥, y con dueño**: la pestaña *Pull requests* de la lista (el servidor ya
filtra por ruta desde ④; falta la pantalla), y el aviso de que una clase que **no ejecuta** no
enseñe el botón de ejecutar en su workspace. Y la verificación es `tsc --noEmit` limpio: **no
puedo abrir sesión OIDC**, así que la consola no se probó a mano — el backend sí, de punta a
punta, en `los-documentos.sh` y `el-puesto.sh`.

### ⑥ La consola (rubix-platform, commit local `1ff602e`)

- **La lista** (`/repositories`): cada fila es **una instancia**, del índice. Nombre y ruta, el
  **icono por clase**, «last edited by» y «last edited» **de git**, y la columna **UPGRADE**, que
  dice la verdad o no se enseña: compara `plantillaVersion` con `plantillaActual`, y si el
  producto **no conoce** la clase dice «—», no «Up to date». Abrir una fila abre el workspace
  **acotado**; el menú ofrece **subir la plantilla** y **borrar** (la carpeta entera, en un
  commit, avisando antes).
- **«Save» escribe**: una llamada y un commit (manifiesto + semilla + el `contiene` del
  proyecto), y después `/workspaces?repositorio=…`. La ubicación sale **del árbol**: lo que el
  proyecto nombra, o los paquetes de la celda; **una carpeta vive dentro de un paquete**, y con
  más de uno **se elige**. El nombre **es la carpeta** —su alfabeto ya cabe en el del servidor—,
  y un 409 se enseña sin perder lo escrito.
- **El árbol acotado llega a la consola**: `X-Ore-Raiz` viaja **por cabecera** desde `query.ts`
  (ningún dato entra por la URL) y el editor abre su carpeta; la cabeza sigue siendo la del árbol.
- **El detalle del proyecto** deja de pintar `SAMPLE_PROJECT_FILES`: `contenidoDelProyecto` cruza
  lo que el proyecto **nombra** con lo que el árbol tiene —sus carpetas, sus repositorios y sus
  ítems—, y **el `id` de cada entrada es su ruta**, así que abrir una carpeta es abrir esa ruta y
  borrarla es borrar esa ruta. Crear una carpeta **escribe**; y **«Move to trash» deja de ser una
  mentira**: no hay papelera, es un commit que se lleva la carpeta entera y lo dice antes.
  Favoritos y «compartido conmigo» llegan **vacíos siempre**: en un árbol no existen, y lo que no
  se hace es fingir que guardan algo.

**Lo que queda fuera de ⑥, y con dueño**: la pestaña *Pull requests* de la lista (el servidor ya
filtra por ruta desde ④; falta la pantalla), y el aviso de que una clase que **no ejecuta** no
enseñe el botón de ejecutar en su workspace. Y la verificación es `tsc --noEmit` limpio: **no
puedo abrir sesión OIDC**, así que la consola no se probó a mano — el backend sí, de punta a
punta, en `los-documentos.sh` y `el-puesto.sh`.

## Lo medido para ⑧ (`pruebas-de-fuego/medida-la-plantilla.py`, 2026-09-23, sobre el árbol de victor)

La pregunta viene de una pantalla de Foundry puesta al lado de la nuestra: allí una instancia
nueva de transforms nace con un árbol de ficheros que **compila y corre**
(`src/main/java/<p>/datasets/*.java`, `resources/`, `test/`, seis ejemplos). ¿Con qué nace la
nuestra? Medido creando una de cada clase en el paquete de un proyecto (0035 ⑦.1):

| clase | ficheros | bytes | líneas de **código** | de comentario |
|---|---|---|---|---|
| `transforms` | 2 | 477 | **0** | 10 |
| `analytics` | 2 | 367 | **0** | 9 |
| `models` | 2 | 344 | **0** | 7 |
| `functions` | 2 | 359 | **0** | 5 |
| `semantics` | 1 | 114 | **0** | 0 |

**Cero líneas de código en las cinco.** Lo que sembramos no es una plantilla: es el manifiesto
y un fichero que *describe* lo que habría que escribir. Al abrir la instancia, el editor enseña
exactamente eso — dos ficheros (`§3`), uno de ellos un comentario.

Y lo que más duele, porque el motor ya está hecho (`§2`):

```
la celda          → {"declarado": [], "estado": "sin-dependencias"}
la instancia      → {"alcance": "packages/<proyecto>/transforms_uno",
                     "declarado": [], "estado": "sin-dependencias"}     ← la semilla no declara nada
sembrando un `pyproject.toml` A MANO:
la instancia      → {"declarado": ["polars","torch"], "digest": "capa-c20f431674d8",
                     "estado": "pendiente"}                            ← ③ funciona
```

La capa **por repositorio** de ③ está construida y medida, y **la plantilla no la usa**: no
siembra ninguna declaración, así que toda instancia nace heredando la capa de la celda — que es
justo lo que ③ existía para romper. Hicimos el motor y no sembramos nada que lo arrancara.

Y lo barato: **hoy no hay ni un repositorio en ningún árbol real** (`§4`), así que la plantilla
puede crecer sin migrar a nadie.

**Lo que ⑧ tiene que construir**, entonces, y en este orden: que una plantilla sea **un árbol de
ficheros** (disposición, ejemplos que **corren**, y su declaración de entorno —`pyproject.toml`
o el fichero de construcción que toque—, que es lo que enciende la capa de ③), y que el
**lenguaje sea una clase y no un campo**: `transforms-python` y `transforms-java` son dos
plantillas con dos versiones, y con una sola clase, el día que cambie una, o subes las dos o
mientes en la columna «UPGRADE».

### ⑧a · `transforms-python`: una plantilla es un árbol de ficheros (hecho)

**El lenguaje pasa a ser una clase**: `transforms` es ahora `transforms-python`. `familia`
(«transforms») y `lenguaje` («python») están para **agrupar en la consola** y no identifican
nada — identifica `id`, como siempre. Y la clave de antes **sigue resolviendo** (`ANTIGUAS`):
un árbol escrito ayer no se rompe porque hoy le pongamos el lenguaje al nombre. No es una clase
más: no se lista ni se ofrece.

**La semilla deja de ser un cartel.** `transforms-python` nace con:

- `transforms/ejemplo.py` — **código que corre**: es el mismo `@transform(inputs, output)` que
  la prueba de fuego ejercita contra agentes de verdad (`el-puesto.sh` 10 y 11), con dos
  referencias de marcador que hay que cambiar, como el `SOURCE_DATASET_PATH` de Foundry.
- `pyproject.toml` — **el sitio donde declarar**, y nace **vacío a propósito**. Lo que faltaba
  no era una dependencia: era el fichero. Sin él un repositorio no puede declarar nada y se
  come la capa de la celda (③); con él, añadir una línea le da la suya. Sembrar `polars` «por
  si acaso» costaría construir una capa para algo que el ejemplo no usa.

**Versión 2**, porque la plantilla cambió: lo escrito con la de antes sale `actualizable: true`,
que es exactamente para lo que existe la columna «UPGRADE» (⑤).

**Medido después** (la misma medida, sobre el árbol de victor):

| | antes de ⑧a | después |
|---|---|---|
| ficheros de una instancia nueva | 2 | **3** |
| líneas de código | **0** | **12** |
| bytes | 477 | 1 289 |
| su capa al declarar en SU `pyproject.toml` | *no había dónde* | `sin-dependencias` → **`capa-c20f431674d8`** |

Y el caso **22** de `los-documentos.sh` lo fija: la semilla trae su entorno y su ejemplo, el
ejemplo **es código** (≥5 líneas que hacen algo, y `@transform(` dentro), la instancia nace
`sin-dependencias`, y declarar en su `pyproject.toml` le da **capa propia** — el fichero
sembrado es lo que lo enciende.

**Lo que queda para ⑧b**: las otras cuatro clases siguen en **0 líneas de código**;
`transforms-java` (la de tu captura); la consola —las tarjetas agrupadas por familia y
lenguaje, que hoy están escritas a mano en `BuildPicker`—; el **422** de crear una instancia en
un paquete de datos (`discover.*`) en vez de en el de un proyecto; y el upgrade como propuesta
con su diff.

## Lo que esto no decide

- **Qué máquina pide cada clase** (CPU/GPU, tamaño): es 0027 y su lista de certificación; aquí
  sólo queda dicho que la clase **elige perfil**, y que sin perfil se cae al de hoy.
- **Si una clase puede traer un runtime que no sea Python**: `functions` en TypeScript existe en
  Foundry y aquí no; la puerta es 0031 §3 (los entornos del puesto) y no se abre en esta ADR.
- **Cómo se migra un repositorio de una clase a otra**: hoy es editar el manifiesto. Si alguna
  vez duele, será una propuesta con su diff, como todo lo demás.
- **Quién puede crear repositorios de qué clase**: eso es ore-iam, plano de control.

**Fuentes del cotejo:** [Foundry · Code Repositories overview](https://www.palantir.com/docs/foundry/code-repositories/overview) ·
[Foundry · Repository upgrades](https://www.palantir.com/docs/foundry/code-repositories/repository-upgrades) ·
[Foundry · Train a model in Code Repositories](https://www.palantir.com/docs/foundry/model-integration/tutorial-train-code-repositories/index.html) ·
[Backstage · Writing templates](https://backstage.io/docs/features/software-templates/writing-templates/) ·
[Dev Container specification](https://containers.dev/implementors/spec/) ·
[devcontainers/spec](https://github.com/devcontainers/spec)
