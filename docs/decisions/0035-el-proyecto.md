# 0035 · El proyecto: el alcance que falta entre la celda y el asset

**Estado:** medido (la medida del 2026-09-23; **sin decidir**: la resolución se escribe sobre
estos números) · **Fecha:** 2026-09-23 · **Decide** (cuando se resuelva): qué es un proyecto
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
| **§1 la superficie** | `Project` tiene **6 campos**: `id`, `name`, `description`, `collaborators`, `createdAt`, `updatedAt`. Cinco son del árbol (la ruta, el manifiesto, `git`); **uno solo no lo es: `collaborators`**. Cableado: crear (a una lista en memoria); `notImplemented`: *Share project*. Dentro se crean **5 productos** (Folder, Code Repository, Pipeline, Lineage exploration, Map) y el «Code Repository» ofrece **7 plantillas** (`analytics`, `functions`, `models`, `semantics`, `transforms`, …) que **no escriben nada**: el picker devuelve la elección a quien abrió el modal | lo que un proyecto ES cabe casi entero en el árbol; lo único que no es del árbol es **quién colabora**, que es plano de control (ore-iam). Una plantilla es **una semilla de ficheros**: no un concepto nuevo |
| **§2 el árbol** | Una carpeta del cliente **en medio del paquete** compila (`packages/ventas/churn/views/es.yaml`, código 0) y el índice ya la nombra (`carpeta: 'churn'`, 0034 ④). Como manifiesto: **`README.md` se ignora y compila**; un `.yaml` sin kind es **`OOS1002`**; un `kind: Project` es **`OOS1003`** («no es un documento de v1alpha1»). Y un paquete **movido fuera de `packages/`** (`proyectos/churn/ventas/`) compila… **porque desaparece**: el índice pasa a no verlo | el proyecto **no puede estar por encima de los paquetes**: `packages/<ns>/` es donde se busca. O el proyecto es **una carpeta dentro del paquete** (con `README.md`, como el schema de 0034), o es **otra cosa que no vive en el árbol**. Y si alguna vez fuera un documento, `kind: Project` **exige tocar OOS** — que es un vocabulario de significado, no de organización del trabajo |
| **§3 el alcance** | **38 rutas** bajo `clusters/[celda]/` —y `projects` es una de ellas: hoy el proyecto es **una página de la celda**, no un alcance—; **76 ficheros** de la consola nombran `celda`; **22 módulos** de `lib/server`, **0** con noción de proyecto; **33 llamadas** a ore-serve declaradas en `query.ts`, ninguna con proyecto; **71 rutas** en `ore-serve`, ninguna con proyecto | meter un alcance en medio no es una pantalla: es **la consola entera y la superficie de ore-serve**. Es el coste real de la decisión, y hay que pagarlo una vez y en un sitio (el equivalente de lo que `x-ore-rama` hizo con la rama) |
| **§4 ramas** | Dos ramas que tocan **dos proyectos distintos funden limpio**. Pero el espacio de ramas es **uno** (`ana/churn`, `bea/nomina`, `main`: una sola lista para todos), una rama puede tocar **los dos proyectos** y nada lo impide, y un documento **roto en el proyecto B**: `ore validate` del árbol entero falla (`OOS2018`) aunque **escribir en el proyecto A sigue dando 201** (la regla «no empeorar»), y el índice enseña la relación rota de B desde A | lo que hoy **no** es del proyecto: la rama, la propuesta y el estado de compilación. Dos equipos comparten espacio de ramas, y el árbol «no compila» para todos aunque cada uno pueda seguir escribiendo. Es la costura que decide entre **proyecto = carpeta** y **proyecto = repositorio propio** |
| **§5 lo de dentro** | Folder → `PUT /arbol/<ruta>` (la consola, mock). Code Repository → **vive** (el workspace de 0030 + `/puestos` de 0031). Pipeline → vive como lienzo; lo más cerca en el backend es `POST /trabajos` (un fichero del árbol como Job). Lineage exploration → el índice **ya da las relaciones en las dos direcciones** (0034), pero no hay pantalla propia. Map → **nada: es lienzo** | de los cinco, **uno está entero** (el repositorio de código) y dos tienen la mitad de atrás hecha (linaje, pipeline). Un proyecto que contenga tres productos que no existen decide poco: la decisión tiene que servir para el que sí existe |
| **§6 el tamaño** | El índice: demo **17 ítems / 16 KB**, victor **58 / 76 KB**; el árbol de demo, **801 ficheros / 922 KB** (0030). Carpetas de cliente en demo: **una, la raíz** | hoy **todo el árbol sería un proyecto**. No hay nada que partir todavía: la decisión se toma antes de que haya diez, que es cuando duele |

**Lo que la medida deja claro, en una frase:** lo que un proyecto *es* cabe en el árbol
—cinco de seis campos, y la carpeta ya está en el índice—, lo que un proyecto *contiene* ya
vive ahí, y lo que **no** es del árbol son dos cosas precisas: **quién colabora** (plano de
control) y **el aislamiento del trabajo** (la rama, la propuesta y el «compila» son del árbol
entero, no del proyecto).

## Lo que queda por decidir (y con qué se decide)

1. **Dónde vive el proyecto.** `packages/<ns>/<proyecto>/` con `README.md` (§2 dice que
   compila y que el índice ya lo nombra) frente a un `kind` propio (§2: exige tocar OOS) o un
   objeto del plano de control (§1: sólo `collaborators` lo pide).
2. **Si un proyecto tiene su propio aislamiento.** Convención de ramas `<proyecto>/<rama>` y
   propuestas que el servidor acota al subárbol, frente a un repositorio por proyecto (§4).
   El precio de lo segundo: el índice compila **un** árbol.
3. **Qué se paga por el alcance.** §3 dice el tamaño: 38 rutas, 76 ficheros, 33 llamadas.
4. **Qué es «Create ▸ Pipeline» y «Map»** cuando dejen de ser lienzo (§5) — porque el
   contenedor no puede decidirse contra contenidos que no existen.

## Lo que esto no decide

- Quién puede ver un proyecto: **ore-iam**, que es un producto aparte (0034 lo dejó anotado).
- Qué es el catálogo: [`0034`](0034-el-catalogo-de-assets.md). Un proyecto **no** es otro
  catálogo; si acaba siendo una carpeta, el catálogo lo enseña como enseña un schema.
