# La pregunta — por qué nacen vistas en la ingesta, y cómo se pregunta el cliente sobre sus hechos

*2026-09-18. Investigación previa a la ontología mínima (0029, orden revisado, paso 3). Nada de
esto está decidido: es lo que hay, lo que hacen los demás, y lo que se propone.*

## 1. La incógnita: si la vista es la pregunta, ¿por qué la escribe la máquina?

`02-view` §1: una tabla es **un hecho del origen** —nadie lo acuerda—; una vista es **lo que la
organización decide preguntarle** a ese hecho; una entidad es **lo que la respuesta significa**.
Y sin embargo hoy el inductor deja, por cada tabla que entra, una vista identidad (todas las
filas, todas las columnas, nombres del origen) sin que nadie haya preguntado nada.

Por qué pasa —y es una razón de fontanería, no de doctrina—: en la gramática **todo lo que actúa
apunta a una vista**, nunca a una tabla. `materialized` (la copia) vive en la vista; `over` y
`reads` de una `Function` son vistas (`OOS7014`); `backedBy` de una `Entity` es una vista
(`OOS2022`). Sin una vista no hay sobre qué copiar, ni sobre qué invocar, ni qué respaldar. La
vista identidad es la **costura**: el sitio donde la pregunta irá, puesto antes de que haya
pregunta. De ahí que su valor sea estructural y no semántico.

Lo que sí distingue la gramática, y el árbol todavía no aprovecha: `oos.maturity` en la vista
existe **exactamente para esto** —*«una vista adivinada de un catálogo y una que una organización
acordó preguntarse eran el mismo documento»* (`02-view` §4.1)—. La vista inducida es `DRAFT`
por herencia del paquete; nada la señala como *adivinada* en la consola.

Y hay una diferencia que sí es de doctrina, entre las dos clases de base de 0027:

| base | qué decidió la organización | la vista identidad es… |
|---|---|---|
| **estándar** | «todo lo que entre se copia a mi celda» | **la pregunta que hizo**: *dame el hecho entero, tal cual, aquí*. Legítima |
| **foránea** | «esto es un espejo; leo el origen» | **nada**: nadie preguntó. Sobra hasta que alguien copie una tabla (*Copy into this cluster*) o la estreche en la Forge |

**Propuesta** (para decidir en un ADR, no aquí): la base estándar conserva sus vistas identidad y
la consola las enseña como *«adivinada del catálogo»* (`DRAFT`); la base foránea nace **solo con
tablas**, y la vista aparece con el primer acto que la necesita —copiar una tabla, preguntar en
la Forge, respaldar una entidad—. Así cada vista del árbol corresponde a una decisión, que es lo
que la palabra promete. Coste: el inductor y `vistas_con_copia` distinguen la clase (ya la
conocen); `GET /esquema` deja de dar `view` para una tabla foránea sin pregunta.

## 2. Lo que hacen los demás

Siete productos donde el usuario «se pregunta» sobre hechos y construye significado. Lo que
importa de cada uno: **cuál es la unidad**, **cómo se pregunta**, **cómo se acuerda** y **qué hace
la IA**.

### 2.1 Palantir Foundry — Ontology Manager, Object Explorer

- **Unidad**: *object type* respaldado por **un** dataset (y un dataset respalda **un** object
  type). Propiedades, *link types*, *action types*, *functions*, *interfaces*.
- **Cómo se construye** (`Create object type`): eliges el datasource primero; *«any columns will
  be mapped automatically, but can be discarded during this step»*; `Add as a new property` /
  `Add all unmapped columns as new properties`; el sistema infiere *property ID, display name y
  base type* del nombre de la columna; obligatorio elegir **Primary key** y **Title key**.
  `Create` sólo *stages*; `Save` en Ontology Manager publica. Hay **Ontology proposals** y
  **branching**: el cambio se propone y se revisa antes de entrar.
- **Cómo se pregunta**: **Object Explorer** —*«a search and analysis tool for answering questions
  about anything in the Ontology»*—: búsqueda por palabra clave, filtros por propiedad, y **los
  gráficos son el filtro** (cada chart es una agregación de una propiedad; pinchar filtra); las
  *Explorations* se guardan y se reabren con datos frescos; de un object set se salta a
  **Quiver** (análisis) o a acciones en bloque.
- **Lo que coger**: (a) el asistente *columnas → propiedades* con descartar, clave y título —es
  nuestro *Model this table* de la Forge, tal cual; (b) *stage + save + proposal + branch*: lo
  tenemos gratis, porque el árbol es git; (c) *el gráfico es el filtro*: la forma más rápida de
  «preguntarse» sin escribir nada.

### 2.2 Metabase — Models, Questions, Data Studio

- **Unidad**: la **Question** (literalmente) y el **Model**: *«a curated dataset saved from a
  question or a SQL query… meant to be used as the starting point for new questions»*.
- **Cómo se construye**: una pregunta en el *query builder* (elegir tabla, columnas, filtros,
  agregaciones) o SQL; `… > Turn this into a model`. Por columna: **Display name**,
  **Description**, **Column type** (semántico), **Visibility** (*Table and detail views* / *Detail
  views only*), **Display as**. Un modelo se puede **persistir** (materializar en la base).
- **Cómo se acuerda**: **Data Studio**: *Library* (lo que el equipo de datos recomienda),
  *Glossary*, *Dependency graph*, *Transforms*. Los modelos «salen más arriba en la búsqueda y se
  destacan cuando alguien empieza una pregunta nueva».
- **Lo que coger**: (a) llamar a la cosa **pregunta** en la interfaz; (b) *«turn this into a
  model»*: la promoción de una pregunta a unidad reutilizable es un acto, no un formulario; (c)
  la visibilidad por columna (*detail views only*) es nuestro `fields`: qué sale y qué no.

### 2.3 Databricks — Unity Catalog metric views y Genie

- **Unidad**: la **metric view** (GA abril 2026): YAML con `source`, `joins`, `filter`,
  `dimensions`, `measures`; *«define the metric once… users can group by any available field»*.
  Se crea por SQL DDL o en el **Catalog Explorer** con un editor YAML validado.
- **Cómo se pregunta**: **Genie**: lenguaje natural sobre un *space* con tablas curadas.
- **Cómo se acuerda**: el *curador* escribe **Instructions** (texto: métricas, joins, filtros,
  convenciones), **Trusted assets** (consultas SQL parametrizadas y funciones: *«when Genie uses a
  trusted asset… it provides a verified answer»*), **Sample questions** (pregunta natural + SQL que
  la contesta) y **Benchmarks** (preguntas de prueba con resultado esperado, puntuadas).
- **Lo que coger**: (a) la definición **es YAML validado** y la UI es un editor con validación —
  igual que nuestro árbol; (b) *trusted assets* = una `Function` de lectura acordada (`STABLE`);
  (c) *benchmarks*: la pregunta con su respuesta esperada es lo que hace medible «el modelo
  responde bien» — encaja con la casa (medir antes de creer).

### 2.4 Snowflake — Semantic Views, Semantic Studio, Autopilot

- **Unidad**: la **semantic view**, objeto de esquema: *logical tables* (con clave primaria),
  *relationships*, *facts*, *dimensions*, *metrics*, *verified queries*, *custom instructions*.
- **Cómo se construye**: Snowsight `Workspaces > Add new > Semantic View`: ubicación, nombre,
  descripción «en términos de negocio», **contexto** (consultas SQL, ficheros de Tableau/Power BI,
  YAML), elegir tablas y columnas (*«no más de 50»*); **Semantic View Autopilot** (GA feb 2026)
  *«analyzes existing tables and auto-generates the semantic view… inferring relationships and
  suggesting metrics»*, usando **el historial de consultas** para proponer relaciones y *verified
  queries*. Luego se refina en **Semantic Studio**.
- **Lo que coger**: (a) *la máquina propone, la persona refina*: es nuestro `discover` llevado
  un paso más (proponer vistas y entidades, no sólo tablas); (b) el **historial de consultas**
  como evidencia de qué se pregunta de verdad — nosotros tenemos el equivalente en el log del
  gateway y en los resultados de funciones; (c) *verified queries* + *custom instructions* son
  `oos.maturity` sobre la pregunta y el `prompt` de una `Function`.

### 2.5 Looker — LookML: views, dimensions, measures, Explores

- **Unidad**: ficheros de código (`.view.lkml`, `.model.lkml`): una *view* describe una tabla
  con *dimensions* y *measures*; un *Explore* dice qué tablas y cómo se unen. **Developer Mode**
  edita en el IDE, con git detrás (ramas, PR).
- **Lo que coger**: es la prueba de que *la unidad puede ser un documento en git y la interfaz
  un IDE* sin que el analista deje de preguntar: el *Explore* es la superficie de pregunta
  (elegir dimensiones y medidas, filtrar, pivotar) sobre lo que el modelador acordó en código.
  Es exactamente nuestra partición Forge (código, acuerdo) / consumo (pregunta).

### 2.6 dbt Semantic Layer — MetricFlow

- **Unidad**: *semantic models* en YAML con **entities** (claves, deciden los joins), **measures**
  y **dimensions**; *metrics* encima. Todo en el repo del proyecto, con CI.
- **Lo que coger**: la separación *entity / measure / dimension* como vocabulario mínimo con el
  que un no técnico distingue «quién» de «cuánto» de «por qué corte».

### 2.7 Microsoft Fabric — semantic models (Direct Lake)

- **Unidad**: el *semantic model* sobre tablas del lakehouse: `New semantic model` → elegir
  tablas → *Web modeling*: relaciones, medidas DAX. Copilot usa el modelo como contexto.
- **Lo que coger**: poco nuevo; confirma el patrón «elige tablas → relaciones → medidas → la IA
  pregunta encima».

## 3. La correspondencia con OOS

| ellos | OOS | nuestro estado |
|---|---|---|
| dataset / table / logical table | `Table` | ✓ inducida del catálogo |
| question / model / view / metric view | `View` | ✓ sólo identidad, inducida; **nadie la estrecha** |
| object type / entity / semantic model | `Entity` | ✓ gramática; ✓ Forge I1 (leer/escribir); ✗ *Model this table* desde una vista |
| glossary / semantic type / concept | `Concept` | ✓ gramática y Forge |
| trusted asset / verified query / instructions | `Function` (lectura, `STABLE`) + `prompt` | ✓ F4a de lectura (I4); ✗ acordar (`oos.maturity` en la función) |
| action type | `Action` (v1alpha10) | ✗ |
| proposal / branch / stage + save | git: rama, commit, PR | ✓ el árbol ya lo es; ✗ la Forge no lo enseña como *propuesta* |
| autopilot / auto-mapping | `discover` | ✓ tablas; ✗ proponer vistas/entidades con un modelo |
| explorer: filtros, gráficos que filtran | consumo sobre la copia | ✗ (después de F5) |
| benchmarks | medidas de la casa | ✓ como método; ✗ como objeto del árbol |

## 4. Lo que se propone construir: «Preguntar» en la consola

Tres superficies, en este orden, y ninguna inventa un kind.

**A · Preguntar** (Assets Catalog › tabla › *Ask a question*, y Ontology Forge › Views › *New*).
Un constructor de vista con las seis caras y nada más: columnas (marcar/renombrar), filas (`where`
con la gramática cerrada), sobre otra vista (`from.view`), copia (*keep it in this cluster*),
tolerancia, responsable. **Vista previa** sobre la copia cuando la hay (`ore-store leer` con
límite: sin abrir el origen) o sobre el origen para una foránea (por el driver). Guarda una `View`
`DRAFT` en el árbol — un commit del sujeto. Es el equivalente del *query builder* de Metabase y
del *Explore* de Looker, con la disciplina de que **la pregunta es un documento**.

**B · Modelar** (desde una vista: *Model this*): el asistente de Foundry — columnas → propiedades
con descartar, clave (**obligatoria**, con lo que el catálogo sabe), título, conceptos por
propiedad (`is`, el glosario), relaciones — y sale una `Entity` con `backedBy` la vista. Es lo que
C1 dejó fuera del catálogo y lo que la ontología mínima necesita.

**C · Acordar**: el peldaño `DRAFT → REVIEWED → STABLE` de `oos.maturity` visible en la vista, en
la entidad y en la función, con quién y cuándo (el commit); y una `Function` de lectura `STABLE`
enseñada como *trusted*: la pregunta que la organización da por buena, con su `prompt` como
*instructions*. El PR del árbol es la *proposal*; la consola sólo tiene que enseñarla.

Y una cuarta, después de F5, que es donde todo esto se cobra: **Explorar** sobre las entidades
(filtros, gráficos que filtran, objeto a objeto), porque ahí es donde un cliente ve que preguntar
sirvió para algo.

**Lo que la IA puede hacer aquí sin inventar nada**: la máquina propone y la persona refina
(Snowflake): una `Function` de lectura sobre el propio catálogo —`over` las tablas, `output` una
vista y una entidad sugeridas— es F4a tal como quedó ayer, apuntada al árbol en vez de a Olist.

## Fuentes

- Palantir: [Create an object type](https://www.palantir.com/docs/foundry/object-link-types/create-object-type) · [Ontology overview](https://www.palantir.com/docs/foundry/ontology/overview) · [Object Explorer](https://www.palantir.com/docs/foundry/object-explorer/overview) · [Explore with charts](https://www.palantir.com/docs/foundry/object-explorer/explore-charts)
- Metabase: [Models](https://www.metabase.com/docs/latest/data-modeling/models) · [Semantic layer](https://www.metabase.com/features/models) · [Meet Data Studio](https://www.metabase.com/blog/meet-data-studio-semantic-layer)
- Databricks: [Unity Catalog metric views](https://docs.databricks.com/aws/en/uc-semantics/metric-views/) · [Model metric views](https://docs.databricks.com/aws/en/uc-semantics/metric-views/basic-modeling) · [Curate an effective Genie Agent](https://docs.databricks.com/aws/en/genie/best-practices) · [Tune Genie Agent quality](https://docs.databricks.com/aws/en/genie-agents/tune-quality)
- Snowflake: [Using Snowsight to create and manage semantic views](https://docs.snowflake.com/en/user-guide/views-semantic/ui) · [Cortex Analyst](https://docs.snowflake.com/en/user-guide/snowflake-cortex/cortex-analyst) · [Native Semantic Views](https://www.snowflake.com/en/blog/engineering/native-semantic-views-ai-bi/) · [Snowflake Semantic Views guide (Atlan)](https://atlan.com/know/snowflake/snowflake-semantic-views/)
- Looker: [LookML terms and concepts](https://cloud.google.com/looker/docs/lookml-terms-and-concepts) · [Introduction to LookML](https://docs.cloud.google.com/looker/docs/what-is-lookml)
- dbt: [Semantic models](https://docs.getdbt.com/docs/build/semantic-models) · [About MetricFlow](https://docs.getdbt.com/docs/build/about-metricflow)
- Fabric: [Develop Direct Lake semantic models](https://learn.microsoft.com/en-us/fabric/fundamentals/direct-lake-develop) · [Power BI semantic models](https://learn.microsoft.com/en-us/fabric/data-warehouse/semantic-models)
