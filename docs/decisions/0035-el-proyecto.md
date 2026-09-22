# 0035 · El proyecto: el alcance que falta entre la celda y el asset

**Estado:** resuelto, en construcción (⓪ hecho; los pasos en
[`docs/proyecto.md`](../proyecto.md)) · **Fecha:** 2026-09-23 · **Decide** (cuando se resuelva): qué es un proyecto
—el sitio donde un cliente construye sobre sus conjuntos de datos, con código, pipelines,
linaje y mapas—, dónde persiste, y si es **un segundo registro o una segunda vista del mismo
árbol**. Sigue a [`0030`](0030-el-arbol-en-el-editor.md) (el árbol en el editor),
[`0031`](0031-el-puesto.md) (dónde corre el código) y [`0034`](0034-el-catalogo-de-assets.md)
(el catálogo como índice del árbol).

## El problema

La consola tiene **Projects**: crear un proyecto, entrar, «Create ▸ Code Repository», elegir
una plantilla, guardar. Nada de eso persiste. Y no es que falte un endpoint: **falta el
concepto**. Hoy la única unidad de persistencia del inquilino es **el árbol de su celda** —un
repositorio— y la única jerarquía dentro es `packages/<ns>/<kind>/`; el alcance de todo lo
demás —catálogo, workspace, datasets, puestos— es la celda entera. Projects es el único
producto que puede ser un *mock* porque es el único que no tiene un árbol debajo.

Y es el que más importa: **es el entorno donde el cliente construirá un rango amplio de
aplicaciones sobre sus datos**. Hoy contiene cinco cosas (carpeta, repositorio de código,
pipeline, exploración de linaje, mapa) y mañana más.

La pregunta no es dónde guardar un proyecto. Es si **Projects es un segundo registro, o una
segunda vista del mismo árbol**. Si es un segundo registro —su propia base, sus propios
repos— el inquilino acaba con **dos sistemas de registro y dos gobiernos**: el catálogo diría
una cosa y el proyecto otra, el linaje habría que reconciliarlo, y todo lo que
[`0031` W3.7 gobierno](0031-el-puesto.md) acaba de poner —el conducto de lectura, `derivedFrom`,
quién escribió qué, la rama del puesto— se quedaría fuera del sitio donde se trabaja. Es la
misma costura que 0033 quitó entre «copia» y «tabla del lago» y 0034 entre «catálogo» y
«árbol».

## Lo medido (`pruebas-de-fuego/medida-proyecto.py`, 2026-09-23, en local)

| | medido | lo que se sigue |
|---|---|---|
| **§1 la superficie** | `Project` tiene **6 campos**: `id`, `name`, `description`, `collaborators`, `createdAt`, `updatedAt`. Cinco son del árbol (la ruta, el manifiesto, `git`); **uno solo no lo es: `collaborators`**. Cableado: crear (a una lista en memoria); `notImplemented`: *Share project*. Dentro se crean **5 productos** (Folder, Code Repository, Pipeline, Lineage exploration, Map) y el «Code Repository» ofrece **5 plantillas** (`transforms`, `analytics`, `models`, `functions`, `semantics`) que **no escriben nada**: el picker devuelve la elección a quien abrió el modal | lo que un proyecto ES cabe casi entero en el árbol; lo único que no es del árbol es **quién colabora**, que es plano de control (ore-iam). Una plantilla es **una semilla de ficheros**: no un concepto nuevo |
| **§2 el árbol** | Una carpeta del cliente **en medio del paquete** compila (`packages/ventas/churn/views/es.yaml`, código 0) y el índice ya la nombra (`carpeta: 'churn'`, 0034 ④). Como manifiesto: **`README.md` se ignora y compila**; un `.yaml` sin kind es **`OOS1002`**; un `kind: Project` es **`OOS1003`** («no es un documento de v1alpha1»). Y un paquete **movido fuera de `packages/`** (`proyectos/churn/ventas/`) compila… **porque desaparece**: el índice pasa a no verlo | el proyecto **no puede estar por encima de los paquetes**: `packages/<ns>/` es donde se busca. O el proyecto es **una carpeta dentro del paquete** (con `README.md`, como el schema de 0034), o es **otra cosa que no vive en el árbol**. Y si alguna vez fuera un documento, `kind: Project` **exige tocar OOS** — que es un vocabulario de significado, no de organización del trabajo |
| **§3 el alcance** | **38 rutas** bajo `clusters/[celda]/` —y `projects` es una de ellas: hoy el proyecto es **una página de la celda**, no un alcance—; **76 ficheros** de la consola nombran `celda`; **22 módulos** de `lib/server`, **0** con noción de proyecto; **33 llamadas** a ore-serve declaradas en `query.ts`, ninguna con proyecto; **71 rutas** en `ore-serve`, ninguna con proyecto | meter un alcance en medio no es una pantalla: es **la consola entera y la superficie de ore-serve**. Es el coste real de la decisión, y hay que pagarlo una vez y en un sitio (el equivalente de lo que `x-ore-rama` hizo con la rama) |
| **§4 ramas** | Dos ramas que tocan **dos proyectos distintos funden limpio**. Pero el espacio de ramas es **uno** (`ana/churn`, `bea/nomina`, `main`: una sola lista para todos), una rama puede tocar **los dos proyectos** y nada lo impide, y un documento **roto en el proyecto B**: `ore validate` del árbol entero falla (`OOS2018`) aunque **escribir en el proyecto A sigue dando 201** (la regla «no empeorar»), y el índice enseña la relación rota de B desde A | lo que hoy **no** es del proyecto: la rama, la propuesta y el estado de compilación. Dos equipos comparten espacio de ramas, y el árbol «no compila» para todos aunque cada uno pueda seguir escribiendo. Es la costura que decide entre **proyecto = carpeta** y **proyecto = repositorio propio** |
| **§5 lo de dentro** | Folder → `PUT /arbol/<ruta>` (la consola, mock). Code Repository → **vive** (el workspace de 0030 + `/puestos` de 0031). Pipeline → vive como lienzo; lo más cerca en el backend es `POST /trabajos` (un fichero del árbol como Job). Lineage exploration → el índice **ya da las relaciones en las dos direcciones** (0034), pero no hay pantalla propia. Map → **nada: es lienzo** | de los cinco, **uno está entero** (el repositorio de código) y dos tienen la mitad de atrás hecha (linaje, pipeline). Un proyecto que contenga tres productos que no existen decide poco: la decisión tiene que servir para el que sí existe |
| **§7 el aislamiento** | **Compilar**: sólo el paquete del proyecto es `OOS2004` (el `datasource` está en el manifiesto raíz) y sólo la carpeta es `OOS2018` (la tabla está un nivel más arriba): **la unidad de compilación es el árbol**. **El gobierno**: una `ConduitPolicy` más estrecha puesta en la carpeta del proyecto A **alcanza también al proyecto B** (`OOS4002` en los dos) — `clearances` combina por el mínimo en todo el árbol: una política de proyecto **no acota, estrecha a todos**. **Los nombres**: dos proyectos del mismo paquete con una `View es` cada uno es `OOS2035` («declarado 2 veces»): `<paquete>.<nombre>` es del árbol, y el puntero (`datasets/<p>_<n>.json`) es plano. **La sesión**: ana pide un puesto para A y otro para B y recibe **el mismo** (`puesto-ana-python`: `id_de(persona, entorno)`), y decir una rama tampoco cambia el id. **El índice**: una cabeza, un índice, sin `proyecto`; y `datos` resuelve cualquier `<paquete>.<nombre>` del árbol, porque el conducto decide **por etiqueta, no por proyecto** | el aislamiento por proyecto hoy es **cero en cinco planos**: compilar, gobernar, nombrar, ejecutar y leer. Ninguno se arregla con una carpeta. Y son cinco decisiones distintas: dos se pueden dejar como están (nombrar y gobernar son **del árbol a propósito**: un nombre cualificado es único y una política que estrecha protege a todos), una es barata (la sesión: `id_de(persona, entorno, proyecto)`), y dos son caras y hay que decidirlas (compilar por proyecto, y el alcance de lectura) |
| **§6 el tamaño** | El índice: demo **17 ítems / 16 KB**, victor **58 / 76 KB**; el árbol de demo, **801 ficheros / 922 KB** (0030). Carpetas de cliente en demo: **una, la raíz** | hoy **todo el árbol sería un proyecto**. No hay nada que partir todavía: la decisión se toma antes de que haya diez, que es cuando duele |

**Lo que la medida deja claro, en una frase:** lo que un proyecto *es* cabe en el árbol
—cinco de seis campos, y la carpeta ya está en el índice—, lo que un proyecto *contiene* ya
vive ahí, y lo que **no** es del árbol son dos cosas precisas: **quién colabora** (plano de
control) y **el aislamiento del trabajo** (la rama, la propuesta y el «compila» son del árbol
entero, no del proyecto).

## Lo cotejado: qué es un proyecto en Foundry (2026-09-23)

Lo que más se parece a lo que la consola dibuja, y el único que lo tiene resuelto de punta a
punta. De su documentación:

- **Un proyecto es a la vez un límite conceptual y un límite de seguridad.** Organiza
  personas, recursos y carpetas «para un propósito», y es **«the primary boundary for
  discretionary role grants»**. La frase que más importa: **«work and its output must live in
  the same project»**.
- **Los roles bajan por contención**: `Viewer`/`Editor`/`Owner` en el proyecto valen para todo
  lo que contiene, y una organización puede **prohibir** conceder a nivel de fichero o carpeta
  para que el proyecto sea el único sitio donde se concede.
- **Lo obligatorio no es del proyecto**: *markings*, controles por clasificación y
  organizaciones se aplican aparte, **por encima de los roles**, y **se propagan por
  derivación**: lo que sale de un dato marcado hereda su marca, cruce o no el proyecto.
- **Cruzar de proyecto es explícito**: una *project reference* (import). Se importa como
  **entrada**, nunca como salida: escribir fuera de tu proyecto es `AccessOutsideProjectDenied`.
  Y exige dos permisos distintos, uno en el recurso de origen y otro en el proyecto de destino.
- **La carpeta es organización; el proyecto es la frontera.** Dentro hay carpetas; fuera, sólo
  referencias.

**Lo que esto le dice a 0035, punto por punto:**

| Foundry | nosotros, hoy (medido) |
|---|---|
| el proyecto es el sitio donde se **conceden** los roles | no concedemos nada en el árbol: la concesión es de **ore-iam** y **niega, no concede** (memoria del producto). Si el proyecto fuera frontera de seguridad, lo sería **en el plano de control**, no en el árbol |
| las marcas **se propagan por derivación**, crucen o no el proyecto | es exactamente lo que hicimos en W3.7 ③ con `derivedFrom`: la clasificación baja por el grafo y **no respeta carpetas**. Coincidimos, y por la misma razón |
| «el trabajo y su salida viven en el mismo proyecto»; escribir fuera es un error con nombre | nosotros no tenemos dónde escribir eso: `write()` acepta cualquier `<paquete>.<tabla>` del árbol. El equivalente nuestro **ya existe y es otro**: `@transform(inputs, output)` acota lo que un trabajo lee y escribe (W3.7 ⑤), pero por **declaración**, no por pertenencia |
| cruzar de proyecto es un **import explícito**, y sólo como entrada | en el árbol, una vista de un paquete puede leer de otro sin pedir permiso a nadie: el vocabulario es **uno** (`<paquete>.<nombre>`) y ésa es la propiedad que hace que el catálogo y el linaje sean uno |

La conclusión del cotejo, y es una advertencia: **Foundry paga el aislamiento con la unidad del
registro.** Sus proyectos son fronteras porque su ontología vive fuera de ellos y el linaje
cruza sistemas. Nosotros hicimos lo contrario en 0033, 0034 y 0031: **un árbol, un índice, un
linaje**. Copiar su frontera entera nos costaría exactamente eso; copiar su *idea* —que el
trabajo tenga un sitio con dueño— no.

> ### ⑤ La resolución: un proyecto es un propósito con dueño, un sitio en el árbol y unas ramas donde se trabaja — **una lente, no una caja**.

**Un proyecto no es un registro, es un recorte con nombre del único que hay.** Y no es una
frontera: **organiza y atribuye, no gobierna**. Lo que gobierna sigue siendo la etiqueta (el
conducto), la declaración (`@transform`, `derivedFrom`) y quién escribió (`escrito_por`), y las
tres cruzan proyectos **a propósito** — como las *markings* de Foundry cruzan los suyos. Es la
misma frase que el producto ya tiene para la concesión («niega, no concede»), dicha en el plano
de la organización del trabajo.

**1 · De qué se compone.** Tres partes, cada una en su plano, y la medida obliga a separarlas:

| parte | qué es | dónde vive |
|---|---|---|
| **el sitio** | `proyectos/<nombre>/README.md`: el manifiesto, y lo que el proyecto **nombra** | el árbol |
| **la obra** | las ramas donde se trabaja y la propuesta con la que se publica | el árbol (git) |
| **la gente** | colaboradores y roles | el plano de control (**ore-iam**), nunca el árbol |

**2 · El manifiesto, y por qué un `README.md`.** Medido (§2): un `.yaml` sin kind es `OOS1002`,
un `kind: Project` es `OOS1003` —habría que tocar OOS, que es vocabulario de **significado**, no
de organización del trabajo—, y un `README.md` **se ignora y compila en cualquier sitio**. Así
que el manifiesto es un README con encabezado, legible por una persona en el editor y por el
índice sin analizador nuevo:

```markdown
---
nombre: Customer Churn
descripcion: Predicción de abandono sobre los pedidos y la plantilla.
contiene: [ventas/churn, rrhh/nomina]
---
Lo que este proyecto hace, en prosa.
```

`contiene` nombra **paquetes o carpetas de paquete** (`<paquete>` o `<paquete>/<carpeta>`), que
es lo que el índice ya sabe decir de cada ítem (0034 ④). Un proyecto **vacío** es legal: un
propósito antes de que haya nada.

**3 · Lo que el proyecto NO es** —y se dice para que no vuelva—: no es un espacio de nombres
(eso es el paquete), no es una unidad de compilación (eso es el árbol: `OOS2004`/`OOS2018`), no
es una frontera de seguridad (eso es ore-iam), y no es un repositorio. «Create ▸ Code
Repository» **no crea otro git**: crea la carpeta dentro del árbol, la añade a `contiene` y abre
el workspace acotado a ella; la plantilla es **una semilla de ficheros** en el commit de
creación.

**4 · Un conjunto de proyectos es un atlas, no una partición.** Los proyectos **no parten el
árbol: lo recorren**. Pueden solaparse, compartir paquetes, leerse entre sí y dejar cosas fuera
de todos —hoy, de hecho, está todo fuera (§6)—. Por eso cada ítem del índice lleva
`proyectos: [...]` **en plural**, y por eso la jerarquía queda así:

```
organización   →   celda              →   proyecto            →   assets y código
(la cuenta)        un árbol,              un propósito,           lo que hay y lo que
                   un índice,             un recorte,             se está construyendo
                   un gobierno            ramas y gente
```

La celda garantiza que haya **una** verdad; el proyecto hace que un equipo pueda trabajar sin
verla entera. Y el catálogo y el proyecto son las dos caras del mismo árbol: **Assets es el
árbol idealizado** (qué hay, compilado, gobernado, de solo lectura) y **Project es el árbol en
obra** (qué se construye, por quién, en qué rama). A la par, y con un solo registro debajo.

**5 · El aislamiento, plano por plano** (lo que §7 midió, contestado):

| plano | qué se hace |
|---|---|
| **nombrar** | **nada**: `<paquete>.<nombre>` es del árbol a propósito; es lo que hace que el catálogo y el linaje sean uno |
| **gobernar** | **nada**: una política que estrecha protege a todos (`clearances` combina por el mínimo). Es la propiedad, no el defecto |
| **compilar** | **no se parte**: se **atribuye**. `ore validate` ya da la ruta de cada error y la ruta dice el proyecto: la consola puede decir «tu proyecto compila; el árbol no, por `rrhh/nomina`» sin cambiar la unidad de compilación. Lo caro se vuelve innecesario |
| **ejecutar** | **se acota, y es barato**: `id_de(persona, entorno, proyecto)` — hoy la misma persona en dos proyectos recibe el mismo puesto |
| **leer** | **no se acota por pertenencia**: el alcance sigue siendo el conducto. Si la pertenencia negara, el linaje dejaría de ser uno. Es lo contrario de Foundry, y es deliberado |

**6 · Lo que se acepta a cambio.** Que un proyecto **no proteja**: quien alcanza la celda
alcanza sus datos según la etiqueta, no según el proyecto —y quien quiera lo contrario lo pedirá
a ore-iam, que es donde se concede—. Que el «compila» siga siendo del árbol, y que un proyecto
roto se vea desde los demás (atribuido, pero visible). Y que dos proyectos puedan nombrar la
misma carpeta: se solapan, y el índice lo dice en vez de impedirlo.

## Lo que la resolución contesta de lo que quedaba abierto

1. **Dónde vive** → `proyectos/<nombre>/README.md` con encabezado, y lo que contiene lo
   **nombra** en vez de contenerlo: un `kind` propio exigiría tocar OOS (`OOS1003`) y una
   carpeta dentro del paquete ataría el proyecto a un solo paquete. Los colaboradores, a
   ore-iam.
2. **Qué aislamiento tiene** → ⑤ 5, plano por plano: nombrar y gobernar no se tocan, compilar
   se **atribuye** en vez de partirse, ejecutar se acota (barato) y leer **no** se acota por
   pertenencia. Un repositorio por proyecto compraría las cinco y pagaría la unidad del
   registro: es lo que Foundry paga, y no lo pagamos.
3. **Qué se paga por el alcance** → lo que §3 mide (38 rutas, 76 ficheros, 33 llamadas) se paga
   **sólo donde hace falta**: el índice trae `proyectos` en la misma llamada que ya se hace
   (0034), y el único sitio que gana un identificador nuevo es la sesión (`x-ore-proyecto`).
4. **Qué es «Create ▸ Pipeline» y «Map»** cuando dejen de ser lienzo (§5): **sigue abierto**, y
   no bloquea — la resolución sirve para el producto que sí está entero (el repositorio de
   código) y admite los otros el día que existan, porque el proyecto los **nombra** en vez de
   contenerlos.

## Lo construido

### ⓪ La forma, medida sobre los árboles de verdad (§8 de `medida-proyecto.py`)

Antes de escribir una línea de `proyectos.rs`, la forma resuelta —
`proyectos/<nombre>/README.md` con encabezado— se puso **sobre demo (`b93ed52`, 66 ficheros)
y victor (`a6e2b0e`, 125)**, traídos por un Job de lectura como en 0034. Los oráculos del
paso ①:

| | demo | victor |
|---|---|---|
| ítems del índice | **17** (2 paquetes, 1 ítem fuera de `packages/`) | **58** (2 paquetes, 1 fuera) |
| carpetas de cliente (candidatas a proyecto hoy) | **0** | **0** |
| ítems que quedarían fuera de todo proyecto | **17** (todos) | **58** (todos) |
| los nombres que saldrían de lo que ya hay | `olist`, `olist_copia` | `foreign_test`, `standard_test` |
| `contiene: [<el primero>]` resolvería | **8 de 17** | **19 de 58** |

Y lo que hacía falta comprobar en un árbol de verdad y no en el de juguete:

- **El manifiesto es invisible.** Con `proyectos/churn/README.md` dentro, `ore validate`
  **sale 0 y no lo nombra** en los dos árboles, y el índice da **los mismos ítems que antes**
  (17 y 58). La forma no toca al compilador ni al catálogo: es exactamente lo que §2 predijo
  del `README.md`, ahora sobre 125 ficheros reales.
- **El encabezado se analiza**, y roto **no rompe el árbol**: sin `nombre`, sin cerrar la raya
  y sin encabezado, el manifiesto no se entiende —«sin `nombre`», «el encabezado no cierra»,
  «sin encabezado»— y **`ore validate` sigue saliendo 0**. Un proyecto roto es un proyecto que
  el índice tendrá que marcar (`roto:`), nunca un árbol que no compila.
- **`contiene` que no resuelve resuelve 0 ítems**, y no es un error: un proyecto puede nombrar
  algo que todavía no existe, y un proyecto vacío es legal (⑤ 2).
- **Dos proyectos que nombran la misma carpeta**: `validate` sale 0 y **el árbol no se entera**.
  El solape existe y no lo impide nadie — por eso el índice lo dirá con `proyectos` **en
  plural** (⑤ 4), en vez de prohibirlo.

**Lo que ⓪ deja dicho para ①**: hoy **todo queda fuera** (17 y 58, confirmando §6 sobre los
árboles reales), así que el primer proyecto de un cliente **no reparte nada**: nombra. Y el
índice tiene que poder decir tres cosas que el compilador nunca dirá — qué proyecto hay, qué
resuelve cada uno, y cuál está roto.

### ① El proyecto en el índice (`ore_core::proyectos` + `assets::indice`)

`crates/ore-core/src/proyectos.rs`: `leer(raiz)` → los `proyectos/*/README.md`, por nombre de
carpeta. El encabezado se analiza **con el analizador del árbol** (`parse.rs`): ni dependencia
nueva ni formato nuevo. Y tres reglas, que son la decisión hecha código:

- **No falla nunca.** Lo que no se entiende se devuelve con `roto: <por qué>` —«sin `nombre`»,
  «el encabezado no cierra», «sin encabezado»— y **se lista igual**. Un proyecto roto no alcanza
  nada, pero tampoco desaparece: es la misma regla que una relación `rota: true` del índice.
- **`contiene` nombra, no contiene.** `alcanza(paquete, carpeta)`: `ventas` alcanza el paquete
  entero; `ventas/churn`, la carpeta **y lo que cuelga de ella** (`churn/v2` sí, `churnalot`
  no). Un proyecto vacío es legal.
- **El proyecto no es un ítem**, y no cambia nada de lo que ya había: mismos ítems, mismas
  relaciones, mismo `validate`.

El índice gana **dos cosas** (0034): `proyectos: [{nombre, titulo, descripcion, contiene, items,
ruta, version, roto?}]` en la raíz, y **`proyectos: []` en cada ítem** —en plural, porque se
solapan—. En ore-serve, un proyecto tiene `version` como cualquier otro fichero: **quién lo
creó y cuándo sale de su propio manifiesto**, sin preguntarle nada al árbol.

La prueba (`crates/ore-core/tests/assets.rs`): cinco manifiestos sobre el árbol de fuego —el
paquete entero (11 ítems), la carpeta del cliente (1), **otro que la nombra también** (1), uno
vacío (0) y uno roto (0)—, `view:ventas.pedidosEs` con **tres proyectos a la vez**, el modelo de
la raíz **fuera de todos**, y los ítems sin cambiar.

Y sobre el árbol de verdad, `ore assets` en demo (`b93ed52`) con `churn` (`contiene: [olist]`) y
uno roto dentro:

```text
2 proyectos · 17 ítems · 9 fuera
  churn                          8 ítems · olist
  roto                         ROTO: sin `nombre`
```

**8 de 17**, que es exactamente el oráculo de ⓪, y `ore validate` sigue saliendo 0.
**Medida §6 contestada**: «todo el árbol sería un proyecto» → los ítems se reparten, y lo que
queda fuera se cuenta en vez de esconderse.

### ② Servirlo y escribirlo (`ore-serve/src/proyectos.rs`)

**Leerlos no tiene ruta.** `GET /assets` ya los trae desde ①: una llamada, la que la consola ya
hace. Lo nuevo es sólo lo que el árbol desnudo no sabe hacer:

| verbo | qué hace |
|---|---|
| `POST /proyectos {nombre, descripcion?, contiene?}` | 201 con el `id` **sacado del título** (`Customer Churn` → `customer-churn`), o **409** si ese nombre ya está |
| `PUT /proyectos/{id}` | el manifiesto **entero**, como un documento; **404** si no está (crear es `POST`, y así el verbo dice cuál de las dos cosas pasó) |
| `DELETE /proyectos/{id}` | se va **la lente, no lo que nombraba**, y la respuesta lo dice: `siguenEnElArbol: [hr, sales]` |

Y tres cosas que **no** hace, cada una por una razón de la resolución:

- **No compila antes de escribir.** Un documento pasa por `empeora` porque puede romper el
  árbol; un proyecto **no puede** —⓪ lo midió—. No hay puerta que poner.
- **No exige que `contiene` resuelva.** Lo que todavía no existe se **dice**
  (`sinResolver: [hr/nomina]`), como `sinHablar` en los documentos, y entra igual: un proyecto es
  un propósito, no un inventario.
- **No sabe quién colabora.** Eso es ore-iam.

El manifiesto lo escribe el servidor, así que los escalares van entre comillas y de una línea:
un título con `---` dentro **no puede cerrar el encabezado** ni abrir otra clave (prueba en el
módulo). Y **un proyecto lo crea una persona**: `/proyectos` no está en la puerta del agente
(W3.7 gobierno ①), así que desde un puesto es **403** — la lista de permitidos hizo su trabajo
sin tocarla, que era la promesa de P4.

La prueba (`los-documentos.sh` 20, contra un `ore-serve` de verdad con forja): crear es un commit
del sujeto con asunto «crear un proyecto»; el nombre repetido es 409 **y no hay commit**; el que
nombra lo que no existe entra y lo dice; `GET /assets` los trae en orden con sus ítems (4 y 0),
con el `version` de **su propio manifiesto** (`persona.ana`) y con cada ítem diciendo sus
`proyectos`; `PUT` reescribe el manifiesto entero; y al borrar, `hr` y `sales` **siguen en el
árbol** —`GET /documentos/Entity/hr/Employee` sigue dando 200 y el índice, los mismos ítems—.

## Lo medido para ③ (`pruebas-de-fuego/medida-la-consola-de-proyectos.py`, 2026-09-22)

La consola es lo que queda, y no se toca entera de una vez. Esta iteración son **tres cosas**:
el **listado real**, **crear y borrar un proyecto**, y **crear y borrar carpetas dentro**. Lo
de crear artefactos de code workspace con persistencia real se mide aquí sólo para saber dónde
acaba ésta. Lo que sale:

| | medido | lo que se sigue |
|---|---|---|
| **§1 el listado** | `Project` tiene 6 campos y la tabla 4 columnas (Name, Collaborators, Last updated, Actions). **Cinco de los seis los da ya `GET /assets`**: `id`←`proyectos[].nombre`, `name`←`titulo`, `description`, `createdAt`/`updatedAt`←`version.cuando`. El sexto, `collaborators`, **no existe en ningún sitio**. 3 filas de ejemplo en memoria; 4 acciones sin handler (Open, Rename, Copy link, Move to trash) y lo único cableado es «New project» → `setCreatedProjects` | el listado **no necesita una llamada nueva**: es la del catálogo. Y la columna Collaborators **se va** (o se queda diciendo que la decide ore-iam): inventarla sería el único campo del producto que no responde a nada |
| **§2 crear y borrar** | El modal recoge **2 campos** (`name`, `description`) y `POST /proyectos` pide 3: falta **`contiene`** — tal cual, un proyecto nacería vacío. El `id` de hoy es `proj-1`/`local-0-<nombre>`; el servidor da `customer-churn`. **«Papelera» no existe en ORE**: 0 ficheros en `crates/` la nombran — en un árbol, borrar es un commit | crear y borrar son dos llamadas que ya existen; lo que falta es **preguntar qué nombra** el proyecto, y **decir la verdad al borrar**: no hay papelera, y lo que el proyecto nombraba **no se borra** (`siguenEnElArbol`) |
| **§3 las carpetas** | **Git no guarda una carpeta vacía** (commit con `packages/hr/ingesta/` dentro: 1 fichero, la carpeta no está). `PUT /arbol/packages/hr/ingesta/README.md` **entra (201)** y `GET /arbol` la ve (23 ficheros), pero el **índice no la nombra** (`carpetas: ['']`): el índice cuenta **ítems**, no carpetas. En cuanto cae un documento dentro, sí (`['', 'ingesta']`), y **la hondura se guarda entera** (`['', 'ingesta', 'ingesta/2026']`). **`DELETE /arbol/<carpeta>` es 404**: sólo hay verbo de fichero. Y un proyecto que nombra **dos paquetes** (`['hr','sales']`, 5 ítems) no dice en cuál va la carpeta nueva | una carpeta **es un fichero dentro** —y el fichero natural es un `README.md`, que el editor ve y el compilador ignora: el mismo truco que el manifiesto del proyecto—. Borrarla es borrar lo que tiene dentro: **o N llamadas, o `DELETE /arbol/<carpeta>` aprende directorios** (una llamada, un commit). Y «Create ▸ Folder» **tiene que preguntar en qué paquete** cuando el proyecto nombra más de uno; con uno solo, no hay nada que preguntar |
| **§4 el coste** | 11 ficheros, **2 103 líneas**. `query.ts` ya declara `assets` y `arbol` y tiene **26 llamadas de escritura**: una más es una fila de la tabla. Pero **`ProjectDetailView.tsx` (753 líneas) y `CreateResourceModal.tsx` tienen WIP de otra sesión** | (a) y (b) —el listado y crear/borrar— caen en ficheros **libres**; (c), las carpetas, cae **justo en el fichero que otra sesión tiene a medias**. Se parte la iteración por ahí: lo de fuera se cablea, y lo de dentro espera o entra por un módulo nuevo que el detalle llame en una línea |
| **§5 el viaje** | Contra un `ore-serve` de verdad, los seis pasos **funcionan hoy**: crear 201 · listar 200 (`proyectos: ['customer-churn']`) · la carpeta 201 · listar 200 · borrar la carpeta 200 · borrar el proyecto 200 con `siguenEnElArbol: ['hr']`. Entre **1,3 y 1,8 s** cada uno (clon por petición, forja local) | no falta backend para las tres cosas: falta **cablear**. Y el tiempo dice que la consola tiene que enseñar que está trabajando, no fingir que fue instantáneo |

### ③a La consola deja de ser un mock: el listado, crear, editar y borrar

**Dónde** (rubix-platform, commit local `a8983e3`): `lib/server/proyectos.ts` (nuevo),
`lib/server/query.ts`, `components/catalog/assets-types.ts`, `ProjectsHome.tsx`,
`ProjectCreateModal.tsx`, `ProjectContextMenu.tsx`, `projects/page.tsx`, `projects/[id]/page.tsx`
y `projects/acciones.ts` (nuevo).

- **Las filas son los proyectos del árbol**, de la llamada que el catálogo ya hace
  (`GET /assets`). Un proyecto es una lente y no un segundo registro, así que **no tiene lectura
  propia** — una ruta `GET /proyectos` habría sido un segundo sitio donde mirar, y dos sitios
  pueden diferir.
- **La columna «Collaborators» se fue**, y es la decisión más visible de 0035 en pantalla: quién
  ve un proyecto lo decide ore-iam, que no existe. En su sitio va lo que el árbol **sí** sabe:
  **cuántos ítems nombra**. En la ficha, `collaborators` lleva sólo **quién lo creó** (el commit
  de su manifiesto), que es cierto.
- **El modal gana `contiene`**: qué nombra el proyecto, **elegido de lo que hay** (no se escribe
  a mano, para no nombrar lo que no existe sin querer); se puede dejar vacío a propósito, y lo
  que aún no está se dice (`sinResolver`) y entra igual.
- **«Move to trash» pasa a «Delete project…»** con el aviso de lo que **no** se borra. No hay
  papelera: en un árbol, borrar es un commit (§2).
- Un manifiesto **roto se lista con su porqué** en vez de desaparecer, y **lo que queda fuera de
  todo proyecto se cuenta al pie**: un atlas, no una partición.
- `query.ts` gana tres mandatos y, con ellos, `Plan.metodo` aprende **`PUT`** — «reescribe lo
  que ya existe», frente al `POST` que lo crea.

### ③b Las carpetas: una llamada, un commit, y dice qué se llevó

**Dónde**: `crates/ore-serve/src/arbol.rs` (ORE) y, en la consola, `lib/server/carpetas.ts` y
`lib/server/arbol.ts` (commit local `eb874a3`).

- **`DELETE /arbol/{ruta}` aprende carpetas.** En un árbol **no hay carpetas vacías** —git no
  las guarda—, así que una carpeta **es lo que tiene dentro**: se la lleva **en un commit**, por
  la misma puerta de siempre («el árbol no empeora»), y la respuesta dice **qué ficheros se
  llevó** (`carpeta: true`, `ficheros: [...]`). Borrar sin decir qué es lo que nadie puede
  revisar después. Si al irse algo deja de compilar: **422 y no se borra nada** —lo de dentro se
  devuelve entero desde memoria—.
- **Crear una carpeta es escribir un `README.md` dentro**: el editor lo ve, el compilador lo
  ignora. El mismo truco que el manifiesto del proyecto, por la misma razón. Y se avisa de que
  **no aparecerá en el catálogo hasta que tenga un documento**, porque el índice cuenta ítems y
  no carpetas (§3) — decirlo es la diferencia entre «no se creó» y «aún no hay nada dentro».
- **Una carpeta vive dentro de un paquete**, no «en el proyecto»: el proyecto nombra, no
  contiene. `paquetesDe(proyecto)` da en cuáles puede caer; con más de uno, **se pregunta**.
- **Lo que no se hizo, y por qué**: `ProjectDetailView.tsx` y `CreateResourceModal.tsx` tienen
  trabajo a medias de otra sesión (§4) y **no se han tocado**. Las dos acciones están escritas y
  probadas por debajo; cablearlas allí es una línea por acción.

La prueba (`los-documentos.sh` 21): el `README` entra y compila; con sólo él, el índice **no**
nombra la carpeta; con un documento dentro, **sí**; borrar la carpeta cuando otra vista usa lo
de dentro es **422 sin commit y sin perder nada**; y quitada esa vista, la carpeta se va con
**sus dos ficheros en un solo commit** («retirar `packages/hr/ingesta`»), el índice deja de
nombrarla y repetir el borrado es 404.

## Lo que esto no decide

- Quién puede ver un proyecto: **ore-iam**, que es un producto aparte (0034 lo dejó anotado).
- Qué es el catálogo: [`0034`](0034-el-catalogo-de-assets.md). Un proyecto **no** es otro
  catálogo; si acaba siendo una carpeta, el catálogo lo enseña como enseña un schema.
