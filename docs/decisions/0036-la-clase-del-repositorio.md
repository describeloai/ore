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
