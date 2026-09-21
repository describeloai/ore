# 0034 · El catálogo de assets: el registro del árbol, leído

**Estado:** propuesto (decidido el 2026-09-21; nada construido ni medido todavía) ·
**Fecha:** 2026-09-21 · **Decide:** que el Assets Catalog de la consola **es el sistema de
registro del inquilino** y que ese sistema **es el árbol**: el catálogo lee, no autora; que
lo que registra son **ítems** —lo que tiene bytes, lo que apunta a fuera, las preguntas, la
lógica, la semántica—, cada uno un documento del árbol (y un puntero si tiene bytes); que las
políticas, las reglas, los enlaces y la historia **no son ítems sino capas** que se ven sobre
cada uno; que se organiza en **database** (paquete) y **schema** (carpeta del paquete); y que
sólo enseña lo que el árbol respalda. Es la spec del catálogo; el modelo del dataset que lo
sostiene es [`0033`](0033-el-dataset.md).

## El problema

El catálogo nació como un explorador de warehouse (database → schema → tables) y hasta hoy es
«el registro del árbol sólo para la tabla»: lista las `Table` de un paquete
(`/paquetes/{n}/esquema`) y nada más, con `views: []` fijo, y ofrecía crear Volumes, Topics,
Models, Functions, Docker images sin nada detrás (retirado el 2026-09-21). Mientras, el árbol
ya registra —firmado, con historia y `revert`— todo lo que una sesión de código produce y todo
lo que Forge escribe: datasets con procedencia, vistas, entidades, interfaces, conceptos,
funciones, acciones, modelos entrenados, trabajos. El backend lo sirve; el catálogo no lo
enseña. Y lo que el cliente necesita del catálogo es exactamente eso: **un solo sitio donde
esté registrado todo lo que tiene y lo que significa**, con quién responde, de qué sale y quién
puede.

## La decisión

> ### ① El catálogo lee el árbol. No hay segundo registro.

Lo que el catálogo enseña sale de documentos del árbol y de sus punteros, por las rutas de
`ore-serve`. Se **autora** en los code workspaces (0030) o desde una celda (`declare()`, 0031
W3.7 ①); lo único que el catálogo escribe son organización (crear una base, un schema) y
descripciones, y también son commits al árbol. Un ítem que no esté en el árbol no está en el
catálogo; uno que esté, está, con la firma de quien lo puso.

> ### ② Los ítems: lo que se registra.

| ítem | qué es | documento (árbol) | bytes | ruta que lo sirve hoy | estado en el catálogo |
|---|---|---|---|---|---|
| **Dataset** | lo que el cliente **tiene**: la copia de una tabla (con su plan), lo que `write()`, un transform o un trabajo dejó | hoy `View` + `materialized` / `Table datasource: lago` → `Dataset` (0033) | tabla Iceberg en el lago; puntero `copias/` o `datasets/` con `procedencia` | `GET /datasets`, `/datasets/{ns}/{n}` (ficha: snapshots, retención, procedencia, `escrito_por`) | no |
| **Table** | el puntero a lo físico de una fuente, con sus dos caras | `Table` v1alpha8 | ninguno | `/paquetes/{n}/esquema` | sí (el único) |
| **View** | la pregunta sobre un hecho (proyecta, filtra, agrega) | `View` | ninguno (si es materializada, su copia es un Dataset) | `/documentos/View` | no |
| **Entity** | qué significa una fila: propiedades con concepto, clave, naturaleza, relaciones, etiquetas; respaldada en una View o un Dataset | `Entity` | — | `/documentos/Entity` | no |
| **Interface** | la forma que varias entidades satisfacen | `Interface` | — | `/documentos/Interface` | no |
| **Concept** | el vocabulario: qué ES un dato (glosario) | `Concept` (+ los importados de `vendor/*.oob`) | — | `/conceptos` | no |
| **Function** | lógica con contrato, invocable, first class (`reads`/`over`/`effects`; wasm o modelo) | `Function` v1alpha10 | el módulo (`entrypoint`) | `GET /funciones`, `/invocar`, `/resultados` (no por `/documentos` todavía) | no |
| **Action** | la invocación sin código | `Action` v1alpha10 | — | el árbol (no por `/documentos` todavía) | no |
| **Modelo entrenado** | pesos con digest, versión, `trainedFrom` | `TrainedModel` v1alpha11 | ficheros en el lago (dataset de ficheros) | `/documentos/TrainedModel` | no |
| **Modelo servido** | un perfil certificado que se invoca | `Model` v1alpha9 (raíz, sin namespace) | — | `/modelos` | no (se lista aparte: es vocabulario del árbol, no del paquete) |

Lo que **no** es un ítem: el **trabajo** (`trabajos/<id>.json`) es una corrida —va a Data › Jobs
y a la procedencia del dataset que dejó—; la **fuente** (connection) es de donde apuntan las
Tables, y vive en Data Origins; **Volumes, Unstructured, Topics, Services, Docker images** no
existen en ORE y no se ofrecen (lo no estructurado vuelve como dataset de ficheros, que un
`TrainedModel` ya es).

**Sólo lo respaldado.** Un ítem entra en el catálogo cuando su ruta existe y se midió contra un
árbol real (0034 se mide ítem a ítem antes de pintarlo); una fila permanentemente vacía no se
enseña deshabilitada: no se enseña.

> ### ③ Las capas: lo que se ve sobre cada ítem, y no es un ítem.

Son documentos que **aplican sobre** ítems, y en la ficha son pestañas, no entradas del árbol:

| capa | de dónde sale | qué enseña en la ficha de X |
|---|---|---|
| **Access** (políticas; hoy «ACP», mock) | `Lattice` (niveles), `labels` de columnas y propiedades, `ConduitPolicy` (qué conducto admite qué), `RequestPolicy`, las concesiones de `ore-iam`, el esquema Cedar de Functions/Actions | qué clasificación lleva X, por qué conductos puede salir, quién tiene concesión, qué funciones y acciones lo tocan |
| **Rules** | `Ruleset` | qué reglas apuntan a X |
| **Links** (linaje y relaciones) | `ore view` (linaje por columna), `procedencia` del dataset, `backedBy`, `trainedFrom`, `reads`/`effects` de Functions, relaciones de Entity | de qué sale X, qué sale de X, quién lo usa |
| **History** | `git log` del documento (`/arbol/historia`) + los snapshots de la tabla (la ficha del dataset) | versiones, con quién y cuándo |

Estas capas se **definen en los code workspaces** (o en Forge) y el catálogo las **lee**; una
política no se edita desde la ficha de una tabla.

> ### ④ La organización: database y schema.

- **Database** = un paquete del árbol (`packages/<p>`, con `scoped: true`: alguien eligió qué
  entra). *Standard* copia (sus tablas elegidas son Datasets); *foreign* apunta (son Tables).
- **Schema** = **una carpeta dentro del paquete**, elegida por el cliente para clasificar y
  organizar: `packages/<p>/<schema>/…`. El compilador ya recorre el paquete recursivamente y el
  `kind` es el discriminante, no el directorio, así que un ítem en una carpeta compila igual y
  sigue en el mismo `namespace`. Crear un schema es un commit con la carpeta (y un fichero
  mínimo con su descripción, porque git no guarda carpetas vacías); mover un ítem a un schema es
  mover el documento (`git mv`; Forge ya renombra). El nombre físico del origen (`public`,
  `dbo`) deja de ser el schema y pasa a ser un dato de la Table.
- Un ítem vive en **un** schema; sin carpeta, en la raíz del paquete («sin clasificar»).

## Lo que se mide antes de pintar

Contra `demo` y `victor`, ítem a ítem: qué devuelve cada ruta de ②, en cuánto, y qué le falta
para (a) listar **todos los ítems de un paquete, por carpeta, en una llamada** (hoy `/esquema`
da sólo tablas y sin carpeta), (b) decir de cada dataset **qué lo define y si es identidad**
(la copia que se llama como su tabla), (c) servir las capas de ③ por ítem (hoy no hay ruta de
«qué aplica sobre X»), y (d) `Function` y `Action` por `/documentos`. El resultado dice el orden
de las pantallas; nada se pinta con mock.

## Lo que se acepta a cambio

- El catálogo **no crea** tablas, datasets, funciones ni modelos: los produce la fuente, el
  código o Forge. Es la consecuencia de ①, y es lo que hace que el catálogo sea de fiar.
- Hasta que 0033 ② exista, el catálogo enseña un Dataset leyendo dos documentos distintos por
  debajo (View materializada / Table del lago). La forma no cambia cuando llegue; cambia de
  dónde lee.
- El schema como carpeta obliga a que `/esquema` (o su sucesor) agrupe por carpeta y a que
  la consola deje de derivarlo del nombre físico.
