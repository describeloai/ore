# 0034 · El catálogo de assets: el registro del árbol, leído

**Estado:** resuelto (decidido y medido el 2026-09-21; la resolución ⑤ es la visión del producto; nada construido) ·
**Fecha:** 2026-09-21 · **Decide:** que el Assets Catalog de la consola **es el sistema de
registro del inquilino** y que ese sistema **es el árbol**: el catálogo lee, no autora; que
lo que registra son **ítems** —lo que tiene bytes, lo que apunta a fuera, las preguntas, la
lógica, la semántica—, cada uno un documento del árbol (y un puntero si tiene bytes); que las
políticas, las reglas, los enlaces y la historia **no son ítems sino capas** que se ven sobre
cada uno; que se organiza en **database** (paquete) y **schema** (carpeta del paquete); que
sólo enseña lo que el árbol respalda; y —la resolución, ⑤— que el catálogo **es un índice
compilado del árbol a una cabeza**, `GET /assets`, que ore-serve deriva con ore-core y sirve
de memoria por commit, con los ítems, sus relaciones y sus capas en una sola representación:
la idealización del árbol en la consola, frente al code workspace, que es una instancia de
trabajo sobre una parte. Es la spec del catálogo; el modelo del dataset que lo sostiene es
[`0033`](0033-el-dataset.md).

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
| **Dataset** | lo que el cliente **tiene**: la copia de una tabla (con su plan), lo que `write()`, un transform o un trabajo dejó | `Dataset` v1alpha12 (0033, hecho) | tabla Iceberg en el lago; puntero `datasets/<p>_<n>.json` con `procedencia` | `GET /datasets`, `/datasets/{ns}/{n}` (ficha: snapshots, retención, procedencia, `escrito_por`), `/documentos/Dataset` | sí, parcial (0033 paso 5: lista por base standard y ficha con el puntero) |
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
lista como Tables. Resuelto en ⑤: es un **detalle de la Table**, no un ítem.

**Lo que esto ordena:** primero **(a)**, una ruta `GET /paquetes/{n}/items` que dé cada
documento del paquete con `kind`, `namespace`, `name`, `ruta` (la carpeta es lo que hay entre
el paquete y el fichero), `owner`, `description`, y para un Dataset el resumen de su puntero y
`define` (`from`, `fields` o `identidad: true`): es (a) y (b) en una llamada, ~0,5 s, y es lo
que la consola pinta en el árbol y en la ficha del esquema. Después **(d)** (Function y Action
por `/documentos`, que es dejar entrar dos kinds en `KINDS`), y **(c)** por capas: Access
(`Lattice`/`ConduitPolicy` por `/documentos` + «qué aplica sobre X»), Links («qué usa X» desde
el mismo índice de items), History ya está.

## Lo cotejado (2026-09-21): cómo nombran y forman su índice los que ya lo tienen

Antes de fijar el nombre y la forma del índice se miró qué hacen los sistemas de registro
que funcionan, y qué de cada uno vale aquí.

| sistema | qué es su registro | cómo se dirige un ítem | relaciones | versión | lo que vale aquí |
|---|---|---|---|---|---|
| **dbt** (`manifest.json`) | **un artefacto compilado** de todo el proyecto por cada `parse`: `metadata` (versión del esquema, `generated_at`, `invocation_id`), `nodes`, `sources`, `exposures`, `macros`, `parent_map`, `child_map` | `unique_id` = `<resource_type>.<package>.<name>`, clave del diccionario | `depends_on` en cada nodo **y** los dos mapas derivados (padres, hijos) | el manifiesto es de una invocación; se compara estado contra estado | **el índice es un artefacto compilado del árbol entero, con las relaciones ya derivadas en las dos direcciones**, y cada nodo lleva `path`/`original_file_path`, `checksum`, `columns`, `description` |
| **Backstage** (Software Catalog) | entidades `apiVersion`/`kind`/`metadata`/`spec` —la misma forma que OOS— más `relations` que **el catálogo deriva** de `spec` con procesadores | `kind:namespace/name` | tipadas y **siempre en pareja** (`ownedBy`/`ownerOf`, `dependsOn`/`dependencyOf`, `partOf`/`hasPart`…), emitidas en las dos direcciones | cada entidad, por su origen (git) | **la dirección `kind:ns/name`** (que ore-cli ya usa: `dataset:<qn>`), y que **las relaciones no se autoran: se derivan** de lo que el documento ya dice |
| **OpenMetadata** | entidades con `id`, `name`, `fullyQualifiedName`, `displayName`, `description`, `owners`, `tags`, `version`, `updatedAt`, `updatedBy`, `href`, `changeDescription` | `fullyQualifiedName` jerárquico (`service.db.schema.table`) | `EntityReference` («lo justo para pintar sin desreferenciar»: tipo, nombre, fqn, displayName); linaje como aristas upstream/downstream con profundidad | versión numérica por entidad con `changeDescription` (campos añadidos, cambiados, borrados) | **la referencia mínima** (kind + nombre + displayName) dentro de cada ítem, y el linaje como aristas con profundidad |
| **DataHub** | grafo: entidad (URN) + **aspectos** (la unidad de escritura: ownership, tags, glosario…) + relaciones anotadas en los aspectos, recorribles en las dos direcciones | `urn:li:<tipo>:(<claves>)` | declaradas como claves foráneas anotadas; se consultan por dirección y tipo | aspectos versionados (historia completa) o de serie temporal (perfiles, calidad) | **separar lo versionado (el documento) de lo temporal (el puntero, los snapshots)**; y que las capas son aspectos sobre el ítem, no ítems |
| **Iceberg REST** / **Unity Catalog** | el catálogo en tres niveles (`catalog.schema.table`), `information_schema` como vista de sistema | el nombre en tres niveles | ninguna: es un directorio de tablas | del metadata de cada tabla (snapshots) | **la organización en niveles fijos** que el cliente reconoce: database › schema › ítem |
| **Foundry** (Compass) | recursos con `rid`, en **carpetas** que el cliente organiza; el tipo del recurso (dataset, transform, ontology object) es del recurso, no de la carpeta | `rid` opaco; ruta de carpeta | por el recurso (transform: inputs/outputs; dataset: procedencia) | por recurso (transacciones) | **la carpeta es del cliente y el tipo es del ítem**: exactamente ④ |

**Lo que el cotejo fija:** (1) el índice es **un artefacto compilado** de todo el árbol, no
una suma de rutas (dbt); (2) la forma de un ítem es la del documento —OOS ya es
`kind/metadata/spec`, como Backstage— más lo **derivado**: relaciones en las dos direcciones,
carpeta, puntero; (3) la dirección es `kind:namespace.name` (Backstage), que ORE ya tiene; (4)
cada ítem lleva la **referencia mínima** de lo que enlaza, para pintar sin otra llamada
(OpenMetadata); (5) lo versionado (el documento, por commit) y lo temporal (el puntero, los
snapshots) van separados (DataHub); (6) la organización es en niveles fijos y la carpeta es del
cliente (Unity, Foundry). Ninguno de los seis autora en el catálogo lo que se deriva del
origen; todos lo compilan.

**El nombre.** «Catálogo» ya es el del origen (`ore source catalog`, `discover.catalog.json`);
«índice» ya es `GET /arbol`; «registro» es el de las copias (`registro.rs`); «manifiesto» es
`ontology.config.yaml`. Lo que la consola llama a esta sección es *Assets*, y lo que ② llama a
lo que registra es *ítem* —un asset—. La ruta se llama **`GET /assets`** y lo que devuelve, **el
índice de assets**; un ítem se dirige como **`kind:namespace.name`**.

> ### ⑤ La resolución: el catálogo es el índice de assets, compilado del árbol a una cabeza.

**El problema que resuelve.** El code workspace es una **instancia de trabajo**: una rama,
una parte del árbol, un puesto que lee lo que necesita y escribe lo que produce. El catálogo
es lo contrario: **la representación idealizada del árbol entero** —todo lo que el cliente
tiene y lo que significa, con quién responde, de qué sale, quién lo usa, cómo está gobernado
y en qué versión—. Son dos lecturas del mismo árbol con necesidades opuestas, y el backend
hoy sólo tiene rutas del primer tipo: una pregunta, una lectura de la forja, 0,5–0,9 s
(«Lo medido»). Por eso el catálogo se pinta a trozos, a golpes de `/esquema`, y no puede
decir lo que el árbol ya sabe.

**La decisión.**

1. **Un índice, un sitio.** `ore-serve` compila el árbol entero con ore-core —ya lo hace para
   validar: `cargar_paquete` tiene todos los documentos, y `cadena`, `raiz_de_lectura`, `suelo`,
   `expone_en`, `respaldo`, `flow::lattices` y `flow::clearances` saben lo que cada uno es y
   con qué se enlaza— y de esa compilación **proyecta el índice de assets**: `ore_core::assets::indice(pkg, punteros)`.
   Es una función pura del árbol (y de sus punteros) a un JSON; se prueba sobre los árboles de
   fuego y sobre los reales.

2. **Versionado por la cabeza; no hay segundo registro.** `GET /assets` calcula el índice una
   vez por commit y lo sirve de memoria: la clave es `cabeza`. Un push lo invalida. No se
   guarda en el árbol ni en otra base: se deriva, y cumple ①. `?rama=x` es el mismo índice
   sobre otra rama (lo que un workspace propone, antes de fusionar); `?commit=h`, el catálogo
   como era. El índice lleva `cabeza`, `rama` y `generado`.

3. **La forma.**

   ```jsonc
   {
     "cabeza": "a6e2b0e", "rama": "main", "generado": "2026-09-21T19:40:00Z",
     "paquetes": [ { "name": "standard_test", "type": "standard", "scoped": true, "source": "postgresql_20260918_1920",
                     "owner": "team:victor", "carpetas": ["", "ventas"], "items": 38 } ],
     "items": {
       "dataset:standard_test.orders": {
         "ref": "dataset:standard_test.orders", "kind": "Dataset", "namespace": "standard_test", "name": "orders",
         "displayName": null, "description": "…", "owner": "team:victor",
         "paquete": "standard_test", "carpeta": "", "ruta": "packages/standard_test/datasets/Orders__olist_orders.yaml",
         "define": { "from": "table:standard_test.olist_orders", "identidad": true, "fields": 8, "where": false, "groupBy": false, "freshness": null },
         "expone": [ { "name": "order_id", "type": "String" }, "…" ],
         "puntero": { "estado": "copiada", "filas": 99441, "snapshot": "…", "ubicacion": "datasets/standard_test_orders", "cuando": "…" },
         "relaciones": [ { "tipo": "sale_de", "ref": "table:standard_test.olist_orders" },
                          { "tipo": "leido_por", "ref": "view:standard_test.pedidosGrandes" } ],
         "acceso": { "clasificacion": { "oos.maturity": "DRAFT" }, "conductos": ["materialization.payload"] },
         "version": { "commit": "c5a1342", "cuando": "…", "sujeto": "persona:victor" }
       },
       "table:standard_test.olist_orders": { "…": "…", "detalle": { "object": "olist.orders", "datasource": "postgresql_20260918_1920",
                                                                    "vistaInducida": "view:standard_test.orders" } }
     }
   }
   ```

   - **`ref`** = `kind:namespace.name`, clave del diccionario (Backstage; dbt). Un `Model` o
     un `Concept` importado, sin namespace, va como `model:v2-lite`.
   - **Lo del documento** (`kind`, `namespace`, `name`, `description`, `owner`, `ruta`) y **lo
     derivado**: `paquete`, `carpeta` (④), `define` (qué lo define: `from`, si es identidad —el
     plan no proyecta ni filtra ni agrega y expone lo de abajo con sus nombres—, y qué claves
     del plan usa), `expone` (las columnas/campos que salen, con tipo), `puntero` (sólo en un
     Dataset; el resumen de `datasets/<p>_<n>.json`), `version` (el último commit del fichero:
     lo que `/arbol/historia` da primero).
   - **`relaciones`**, tipadas y **en las dos direcciones**, derivadas de lo que el documento
     dice (Backstage): `sale_de`/`produce` (`from`, `trainedFrom`, la `procedencia` de un
     escrito), `respalda`/`respaldada_por` (`backedBy`), `lee`/`leido_por` (`reads`, `over`),
     `escribe`/`escrito_por` (`effects.writes`), `satisface`/`satisfecha_por` (Interface),
     `nombra`/`nombrado_por` (Concept). Cada arista lleva la `ref` del otro lado; su referencia
     mínima (kind, nombre, displayName) se resuelve en el mismo índice.
   - **`acceso`**: lo que ore-core ya computa por documento —la clasificación (labels y
     retículo) y los conductos que compilan (`flow::clearances`)—. Es la capa Access **del
     plano de datos**. La **concesión de `ore-iam`** (quién puede) es del plano de control y
     no entra en el índice: cómo el plano de control infiere sobre el de datos —quién ve qué—
     es **un producto en sí mismo** y queda anotado aquí como lo que esto no decide.
   - **Rules** (`Ruleset`): cuando exista en los árboles, `relaciones` gana `regido_por`/`rige`
     por el mismo camino.

4. **Dos velocidades.** El índice sirve el árbol lateral, las listas, la ficha básica y las
   capas Links y Access: una llamada, y 0 ms tras el primer cálculo por commit (hoy, ~6
   llamadas ≈ 4 s). Lo **pesado** sigue por ítem y bajo demanda, por las rutas que ya existen:
   los snapshots (`/datasets/{ns}/{n}`, va al almacén), la historia git (`/arbol/historia`), el
   texto del documento (`/arbol/{ruta}`). El índice no las duplica; las enlaza.

5. **La View inducida de una base foreign es un detalle de su Table, no un ítem.** Una foreign
   deja hoy una View identidad por tabla —la pregunta que `over` necesita—; en el índice va en
   `table.detalle.vistaInducida` y el árbol lateral no la lista. Las vistas que el cliente
   escriba después sobre esas tablas —tantas como quiera— sí son ítems: son suyas.

6. **Schema = carpeta, con la regla de hoy.** `carpeta` es lo que hay entre `packages/<p>/` y el
   fichero **quitando la carpeta del kind** (`tables/`, `views/`, `datasets/`, `entities/`,
   `functions/`, `models/`…): así los árboles de hoy caen enteros en la raíz («sin clasificar»)
   y una carpeta `ventas/datasets/x.yaml` o `ventas/x.yaml` es el schema `ventas`. La consola
   deja de derivar el schema del nombre físico; el nombre físico es `table.detalle.object`.

7. **La consola lee el índice y nada más para el catálogo.** `comoDatabase` y las N llamadas
   a `/esquema` + `/copias` desaparecen; el explorador pinta `paquetes › carpetas › items`,
   la ficha pinta el ítem, y las pestañas Links y Access leen `relaciones` y `acceso` del mismo
   ítem; History y los snapshots piden lo pesado al abrirse. Lo que la consola escribe
   (crear base, crear schema, descripciones) sigue siendo un commit por las rutas de hoy, y
   el índice lo refleja en el siguiente `cabeza`.

**Lo que se mide antes y después.** Antes: la forma sobre los árboles reales (`indice()` en
local sobre demo y victor: ítems, relaciones, bytes, ms). Después: `GET /assets` vivo —una
llamada: código, ms en frío y en caliente, bytes— y el catálogo de la consola pintado de él
sin mock; y que el ítem `dataset:standard_test.orders` dice `identidad: true`, `sale_de` su
tabla, y su puntero.

**El orden.** (0) el índice en ore-core con sus tests; (1) `GET /assets` en ore-serve con la
caché por cabeza; (2) la medida; (3) la consola sobre el índice; (4) `Function` y `Action` por
`/documentos` (dos kinds más en `KINDS`, para la ficha y el texto); (5) las capas que faltan por
ítem cuando los árboles las tengan (`Ruleset`).

## Lo que se acepta a cambio

- El catálogo **no crea** tablas, datasets, funciones ni modelos: los produce la fuente, el
  código o Forge. Es la consecuencia de ①, y es lo que hace que el catálogo sea de fiar.
- Hasta que 0033 ② exista, el catálogo enseña un Dataset leyendo dos documentos distintos por
  debajo (View materializada / Table del lago). La forma no cambia cuando llegue; cambia de
  dónde lee.
- El schema como carpeta obliga a que el índice agrupe por carpeta y a que la consola deje de
  derivarlo del nombre físico.
- Un índice compilado por commit es **memoria en ore-serve** (decenas de KB por árbol; victor,
  125 ficheros, cabe de sobra) y un cálculo por push (lo que `validate` ya cuesta). A cambio,
  el catálogo pasa de ~6 llamadas y 4 s a una y 0 ms.
- El índice **deriva** relaciones y acceso; si un documento cambia de forma, cambia el índice y
  no la consola. Es la razón de hacerlo en ore-core y no en la consola.
- La concesión de `ore-iam` —quién puede ver qué— **no** está en el índice: es del plano de
  control, y cómo ese plano infiere sobre el de datos es un producto aparte, no un campo.
