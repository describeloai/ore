# 0034 · El catálogo de assets: el registro del árbol, leído

**Estado:** propuesto (decidido el 2026-09-21; medido el mismo día contra `demo` y `victor`: «Lo medido» abajo; nada construido) ·
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

## Lo medido (2026-09-21): qué sirve hoy el backend, cómo, y qué falta

`pruebas-de-fuego/medida-assets-catalog.py`: un Job por inquilino, con el token del agente,
pide cada ruta de ② al ore-serve **vivo** y se cruza con `GET /arbol`. demo: 66 ficheros
(Table 11, View 9, Dataset 3, Function 1, Model 1, ConduitPolicy 1). victor: 125 (Table 38,
View 19, Dataset 19, Lattice 1, ConduitPolicy 1, Model 1), con dos bases: `foreign_test`
(19 Tables **y 19 Views**) y `standard_test` (19 Tables y 19 Datasets, 9 copiados).

**Cómo lo sirve.** Cada petición cuesta lo mismo, **~0,5 s en demo y ~0,9 s en victor**, sea
`/arbol` (5–12 KB) o `/documentos/Concept` (17 B): es el coste fijo de leer el árbol de la
forja por petición, no del tamaño. `/modelos` cuesta **6 s** (va al registro). La ficha de un
dataset copiado, 1,3 s (va al almacén por los snapshots). Pintar el catálogo de una celda hoy
son 1 (`/paquetes`) + N bases × (`/esquema` + `/copias`) + 1 (`/datasets`) llamadas: con 2
bases, ~6 llamadas ≈ 3–5 s.

| ruta | qué da | forma |
|---|---|---|
| `GET /arbol` | **todo** el registro: cada fichero con `ruta`, `bytes` y `kind` (si es documento) | plano; sin `namespace`, sin `name`, sin `spec`, sin carpeta más que la ruta |
| `GET /paquetes` | por paquete: `name`, `type`, `scoped`, `source`, `tablas`, `copias`, `modeladas`, `decisionesPendientes` | lo que la lista de bases necesita |
| `GET /paquetes/{n}/esquema` | **sólo `tables`** (+ `entities`, 0 en los dos) con `object`, `datasource`, `columns`, `modeled`, `copied`, `view`, `dataset`, `entity` | en un paquete-fuente (sin scope) devuelve el catálogo entero del origen (48 tablas): no es una base |
| `GET /documentos/{kind}` | el documento entero (`apiVersion`, `metadata`, `spec`, `fichero`, `paquete`) de **Entity, View, Table, Concept, Interface, TrainedModel, Dataset** | una llamada por kind, del árbol entero; **404** para Function, Action, Model, Ruleset, Lattice, ConduitPolicy |
| `GET /datasets` | el puntero de cada dataset: copiado → `plan` (digest), `columnas`, `filas`, `snapshot`, `bundle`, `testigo`, `leidas`, `ubicacion`, `esquema_cambiado`…; en error → `clase`, `estado`, `motivo`, `nombre`, `vista` | rico cuando hay bytes; no dice `from` ni `fields` |
| `GET /datasets/{ns}/{n}` | lo anterior + `snapshots[]` (`id`, `operacion`, `filas`, `bytes`, `cuando_ms`, `idempotencia`) | la capa History de los bytes |
| `GET /funciones` | cada Function con `over`, `output`, `effects`, `model`, `prompt`, `resultados` | forma propia, no la del documento |
| `GET /arbol/historia/{ruta}` | `versiones[]` (`hash`, `autor`, `sujeto`, `cuando`, `mensaje`) | la capa History del documento; una llamada por ítem |
| `GET /arbol/{ruta}` | `texto`, `kind`, `commit`, `cabeza` | el documento, tal cual |
| `GET /conceptos` | 0 en los dos | |

**Lo que falta, contra (a)–(d):**

- **(a) Todos los ítems de un paquete, por carpeta, en una llamada: no existe.** `/esquema`
  da sólo tablas (en `olist` faltan 8 Views; en `standard_test`, 19 Datasets; en `olist_copia`,
  3 Datasets + 1 View + 1 Function); `/documentos/{kind}` es por kind y del árbol entero (7
  llamadas ≈ 4–6 s para una base); `/arbol` tiene todo pero sin nombre ni namespace. Y **la
  carpeta hoy es la del kind** (`tables/`, `views/`, `datasets/`, `functions/`): ningún árbol
  tiene una carpeta elegida por el cliente, así que ④ (schema = carpeta) parte de cero y la
  consola sigue derivando el schema del nombre físico (`olist.customers` → `olist`).
- **(b) Qué define un dataset y si es identidad: a medias.** El puntero da `plan` (un digest) y
  `vista`, no `from` ni `fields`; el documento (`/documentos/Dataset`) da `from` y `fields`,
  pero decir «identidad» exige cruzarlo con las `columns` de su Table. Nadie lo dice en una
  respuesta.
- **(c) Las capas: sólo History.** `/arbol/historia` y los `snapshots` de la ficha existen y
  cuestan 0,6–1,3 s por ítem. **Access**: `Lattice`, `ConduitPolicy`, `RequestPolicy` están en
  el árbol (victor: 1 + 1) y `/documentos` los niega (404); no hay «qué clasificación lleva X
  y por qué conductos sale». **Rules**: `Ruleset` 404 (y 0 en los árboles). **Links**:
  ninguna ruta dice «de qué sale X» ni «quién usa X»: el linaje por columna sólo lo da `ore
  view` (CLI), la procedencia (`leidas`) sólo está en el puntero de un escrito, `backedBy` /
  `trainedFrom` / `reads` sólo dentro de cada documento.
- **(d) Function y Action por `/documentos`: no.** `/funciones` sirve la Function (demo: 1)
  con forma propia; `Action` no se sirve por ningún sitio; `Model` sólo por `/modelos`.

**Lo que la medida saca, además:** una base **foreign** deja hoy una View por tabla
(`foreign_test`: 19 + 19; `olist`: 8 + 8) — la pregunta identidad que `over` necesita — y ② la
lista como Tables. Hay que decidir si la View inducida de una foreign es un ítem del catálogo
o un detalle de la Table.

**Lo que esto ordena:** primero **(a)**, una ruta `GET /paquetes/{n}/items` que dé cada
documento del paquete con `kind`, `namespace`, `name`, `ruta` (la carpeta es lo que hay entre
el paquete y el fichero), `owner`, `description`, y para un Dataset el resumen de su puntero y
`define` (`from`, `fields` o `identidad: true`): es (a) y (b) en una llamada, ~0,5 s, y es lo
que la consola pinta en el árbol y en la ficha del esquema. Después **(d)** (Function y Action
por `/documentos`, que es dejar entrar dos kinds en `KINDS`), y **(c)** por capas: Access
(`Lattice`/`ConduitPolicy` por `/documentos` + «qué aplica sobre X»), Links («qué usa X» desde
el mismo índice de items), History ya está.

## Lo que se acepta a cambio

- El catálogo **no crea** tablas, datasets, funciones ni modelos: los produce la fuente, el
  código o Forge. Es la consecuencia de ①, y es lo que hace que el catálogo sea de fiar.
- Hasta que 0033 ② exista, el catálogo enseña un Dataset leyendo dos documentos distintos por
  debajo (View materializada / Table del lago). La forma no cambia cuando llegue; cambia de
  dónde lee.
- El schema como carpeta obliga a que `/esquema` (o su sucesor) agrupe por carpeta y a que
  la consola deje de derivarlo del nombre físico.
