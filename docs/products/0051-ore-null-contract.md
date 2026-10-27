# 0051 · ORE Null Contract

**Estado:** **aceptado · en vivo** (2026-10-03) en `victor` y `demo` · **Decide:** cómo dice ORE,
de punta a punta, qué columnas **nunca son nulas**: el origen lo **declara**, las vistas lo
**derivan**, lo materializado lo **impone** y lo que se expone lo **enseña**, con una sola regla
para cambiarlo. Spec: OOS **v1alpha22** `01-nunca-nula`. Cierra el «REQUIRED» de
[0042](0042-origin-rest-bigquery.md) y la nulabilidad de [0032](0032-el-contrato-de-tipos.md). Se
apoya en [0045](0045-source-pointer.md) (el puntero lleva la verdad del objeto) y en
[0043](0043-ore-arrow-stream.md) (al contrato, sin modo seguro).

## Qué es

**La garantía «esta columna nunca es nula», llevada sin perderse del origen al consumidor.** Los
cuatro drivers ya la leían (`NOT NULL` de Postgres, `mode: REQUIRED` de BigQuery, el `nullable` de
un Parquet), y se perdía por el camino: el inductor la escribía como comentario, la spec no tenía
dónde ponerla en la capa física, el almacén escribía toda columna opcional, y GraphQL y el SDK
daban `String` donde el origen garantizaba `String!`.

## La decisión

### Cinco capas, cada una en su sitio

| capa | dónde vive | qué hace |
|---|---|---|
| **declarar** | `Table.columns.<c>.required` (v1alpha22), escrito por el Source Pointer (`ore source induce`) desde el catálogo del driver | trae la garantía del origen como verdad del objeto. **Sólo una garantía**: lo visto en una muestra (JSONL) no es `required` |
| **derivar** | `vista_sql` (`Columna::exige`) y `vistas::nulabilidad_de_vista` | calcula qué columnas de una vista o un dataset nunca son nulas, con reglas conservadoras: **ante la duda, nulable**. Una `View` y un `Dataset` no lo declaran nunca |
| **imponer** | la copia (`ore materialize` → `ore-store`), encendida por celda con `.arbol/nulos.yaml` | escribe `required` en Iceberg lo que nunca es nulo, y niega la copia que traiga un nulo en una de esas columnas |
| **evolucionar** | `lago::esquema_deseado`, `ore drift-detect` | **aflojar, siempre** (el origen manda); **endurecer, sólo** al crear o al reescribirlo todo, nunca sobre datos que no se miran |
| **enseñar** | `ore view`, GraphQL (`ore export --format graphql`), el SDK del puesto (`over()`), `ore validate` | `nunca nula`, `T!`, Arrow no nulable; y un aviso cuando la entidad exige lo que la columna no garantiza |

### Tres palabras, sin pisarse

| dónde | qué dice | quién la escribe |
|---|---|---|
| `Table.columns.<c>.required` | el origen **garantiza** el valor (física) | el Source Pointer, del driver |
| una `View` o un `Dataset` | **derivado** (física) | el compilador, nunca a mano |
| `Entity.properties.<p>.required` | el concepto **exige** el valor (semántica) | quien modela |
| una aserción de un `Ruleset` | se **comprueba** que no hay nulos (calidad) | quien gobierna |

**El `!` sale de la física, nunca de la semántica.** Una propiedad `required` sobre una columna que
puede ser nula no pone el `!` —mentiría a quien se fía de él—: es un **aviso** (v1alpha22 `01` §8),
y lo que la comprueba es un `Ruleset`.

### La abstracción

```
Nulabilidad = Garantizada   // la declara el origen
            | Derivada      // la calcula el compilador
            | Nulable       // por defecto: ante la duda
```

`types::Nulabilidad` (`Nulable < Derivada < Garantizada`; combinar toma la menor) y
`types::Tipado`, que se escribe con `!` **sólo** si nunca es nula: el digest de un plan no cambia
en ningún árbol sin `required`. **La cabecera de una copia no lleva el `!`**: lo que impone va en
un campo aparte, `obligatorias`, que un almacén de antes ignora y que vacío no se escribe.

### Las reglas de derivar

Tal cual y renombrar conservan; un literal sí, `NULL` no; `CAST` sí, `TRY_CAST` no; `CASE` sólo
con `ELSE` y todas sus ramas; `COALESCE`, con el argumento que menos exige; `COUNT` y los rangos de
ventana sí, los demás agregados no; aritmética y comparaciones propagan, **dividir no** (por cero,
DuckDB da nulo con `//` y `%`); `IS [NOT] NULL` y `<=>` nunca son nulos; una función conocida que
propaga, sí, una desconocida, no; el lado que genera nulos de un `LEFT`, `RIGHT` o `FULL JOIN`,
nulable; `UNION` exige los dos lados; `ROLLUP`, `CUBE` y `GROUPING SETS` dejan la clave nulable; y
lo que una conjunción del `WHERE` afirma (`IS NOT NULL`, una comparación, `IN`, `BETWEEN`, `LIKE`)
no es nulo aguas abajo.

## Cómo se opera

- **Un árbol nuevo** nace con el paradigma: el inductor escribe `required` desde el primer
  catálogo. **Uno que ya existía** se pone al día re-induciendo sus fuentes desde el catálogo que
  guarda (`pruebas-de-fuego/la-fuente-al-dia.py`): es la misma operación para cualquier cambio
  futuro del inductor.
- **Imponer se enciende por celda** con un commit de `.arbol/nulos.yaml`:

  ```yaml
  imponer: true
  ```

  Sin el fichero, o sin entenderlo, apagado. Encender sólo rehace las copias que imponen algo; las
  demás conservan su cabecera byte a byte. **Apagar es aflojar**: `git revert` del commit y una
  pasada, y cada columna vuelve a opcional con su mismo id.
- **Un nulo en lo que nunca es nulo** niega la copia antes de tocar la tabla, con la columna, la
  fila y qué hacer: *«la columna `id` de `…` nunca es nula —lo garantiza su origen— y la fila N
  trae un nulo: la copia no se escribe. Si el origen dejó de garantizarlo, vuelve a catalogar la
  fuente y la columna se afloja»*. El puntero queda en `error` con ese motivo y la copia de antes
  sigue servida. **Falla cerrada: no sirve una cifra falsa.**
- **El esquema nuevo y el snapshot van en un solo commit** (`Lago::instantanea` recibe el esquema):
  una escritura que falla a mitad no deja la tabla `required` sobre ficheros con nulos.
- **Herramientas de operación** (un Job en el inquilino, imagen `ore-drivers`, el token de la forja
  de Secret Manager, nada impreso): `pruebas-de-fuego/nunca-nula-en-vivo.py <celda>` — ensayo en
  seco, `--encender`, `--rehacer` (borra el Job `copiar-<resumen>` y Flux lo recrea),
  `--comprobar` (cada copia contra su `metadata.json`) y `--avisos` (lo que `ore validate` avisa).

## Lo medido

### La espiga (2026-10-02)

Una tabla Iceberg real escrita con el `Lago` de `ore-store` y leída con DuckDB, PyIceberg y Spark
3.5.6. Lo que decidió el diseño:

- `iceberg` 0.10.1 crea la columna `required` y **su escritor ya rechaza el nulo**; aflojar en el
  sitio funciona (`AddSchema`, mismo id).
- **Pero deja endurecer en el sitio con nulos dentro**, y entonces **Spark contesta `count(id) =
  4` y 0 nulos cuando hay uno**: se fía de la marca y la cifra sale mal sin error. La guarda que
  no deja endurecer sin reescribir es **nuestra**.
- **DuckDB ignora `required`**, y el puesto lee con DuckDB: GraphQL y el SDK tienen que sacar la
  garantía **del árbol**, no del lago. Y GraphQL ignoraba el `required` escalar: sólo la clave
  salía `ID!`.
- Las vistas de hoy son SQL: derivar va en `vista_sql`, que tuvo que aprender qué relación está del
  lado que genera nulos de un join.

### En vivo, antes de tocar nada (P0)

| origen | driver | columnas | `required` |
|---|---|---|---|
| olist (`demo` y `victor`) | Postgres | 342 | **88** |
| standard (`victor`) | Postgres | 157 | **67** |
| `bigquery_20260927_1428` (`victor`) | BigQuery | 13 | **3** |
| S3 (`victor`): 10 CSV, 1 JSONL, 1 Parquet | S3 | 69 | **0** |

En el lago, 61 columnas candidatas en 19 copias: **0 nulos**. Ningún origen había mentido (sobre
poco dato: las tablas grandes de hoy no garantizan nada).

### La derivación, contra un motor (P4)

`pruebas-de-fuego/nunca-nula.py`: tablas y vistas al azar contra DuckDB 1.5.6. **8 semillas, 2 084
vistas, 2 946 afirmaciones «nunca nula», 0 nulos.** Y la prueba caza lo que debe: sin la regla del
`LEFT JOIN`, 6 fallos; con `//` y `%` propagando, 35.

### Declarar, en vivo (P3′, 2026-10-02)

| | `victor` | `demo` |
|---|---|---|
| lo que cambió | **21 tablas ganan `required` en 70 columnas**; y lo pendiente de 0045 E5′ (67 punteros, 3 schemas) | ninguna tabla existente (olist no garantiza nada en lo copiado); E5′: 229 punteros, 8 schemas |
| forja | `305a726` | `740e182` |
| lo comprobado | `validate` 0 → 0; los mismos 27 datasets con los mismos snapshots; la copia en seco: 23 «ya está», 0 por calcular | `validate` 0 → 0; los mismos 3 datasets |

### Imponer, en vivo (P6, 2026-10-02)

- **La prueba de fuego** (`pruebas-de-fuego/nunca-nula-se-impone.sh`, jsonl → S3 de mentira →
  `ore-store-r2`): **4 de 4**. Apagado, la copia de antes; encendido, `id` `required` con el mismo
  id de columna (PyIceberg lo lee `required`) y la copia sin garantías «ya está»; un nulo
  inyectado falla con su fila, el puntero en `error` sigue en la copia de antes y **no hay ningún
  `metadata.json` nuevo**; apagar afloja.
- **`demo`** (forja `606dae8`): ninguna copia impone nada; 0 columnas `required` en el bucket.
- **`victor`** (forja `b9793da`): de 34 copias, **21 imponen**; la pasada rehízo 20, **sin negar un
  solo nulo**, y las 3 sin garantías siguieron «ya está». En el bucket: **20 copias con 66 columnas
  `required`**, y en cada una lo `required` de Iceberg es exactamente lo que su cabecera impone.

### Enseñar (P7, 2026-10-03)

- **GraphQL**: un campo que no es clave sale `T!` cuando la columna del mismo nombre de lo que
  respalda la entidad nunca es nula. Caso de conformidad `v1alpha22/emit/a-guaranteed-column-is-
  non-null`: `email` garantizado → `String!`; `nota` → `String`; `apodo`, `required` en la entidad
  sin garantía → `String`. v1alpha22: **6/6**.
- **El SDK**: `GET /puestos/{id}/datos/{vista}` lleva `nunca_nulas`, y `over()` marca esas columnas
  no nulables en Arrow (una que traiga un nulo no se marca, y se avisa).
- **El aviso**: `ore validate` avisa, sin fallar, de cada propiedad `required` cuya columna puede
  ser nula. En vivo (imagen `6504902`, que ya avisa: lo comprueba un árbol mínimo dentro del Job), `validate` sale con 0 y **0 avisos** en `demo` y en `victor`: ninguna entidad exige lo que su columna no garantiza. `ore-serve` sirve `6504902` en las dos celdas.

## Lo que no se hace

- **Imponer en el aterrizaje crudo.** Se impone donde se materializa una copia.
- **`required` escrito a mano en vistas o datasets.** Se deriva.
- **Endurecer una tabla Iceberg existente sin reescribirla.**
- **Imponer lo que escribe `write()`** (un dataset escrito): ahí no hay nada que derivar.
- **Claves primarias o únicas impuestas.** En la industria son informativas, y `changes.key` ya
  tiene su papel.
- **Convertir una observación en garantía.**

## Lo que queda abierto

- **La ficha de la consola** no enseña todavía «nunca nula · lo garantiza el origen» (el dato ya
  está en la API de columnas de `ore-serve` desde P3).
- **`ore-read-postgres` no lee `vector`** (pgvector): `brain_embeddings` de `victor` no se copia,
  con o sin P6.
- **`motivo_de`** sólo reconoce un stderr que empieza por `error:` (el del almacén); el de un
  driver deja el puntero en «`ore-read-postgres` falló (1)».
- **El riesgo del diseño**: un origen que deja de garantizar una columna y nadie vuelve a
  catalogar. La copia falla cerrada en el primer nulo, con el mensaje que dice qué hacer.
- **Las claves de columna de `Table`** se comprueban enteras sólo desde v1alpha22: hay árboles con
  `labels` en columnas de tablas viejas, y cerrarlo hacia atrás cambiaría lo que significan.

## Deudas que cierra

- [0042](0042-origin-rest-bigquery.md) · «REQUIRED → `required` en Iceberg».
- [0032](0032-el-contrato-de-tipos.md) · la nulabilidad, que T2 llamó «cosmética» y no lo era.
- `ore-store` · «todo opcional».
- El inductor · la garantía del origen escrita como comentario.
- GraphQL · el `required` escalar, ignorado.

## Historia

| paso | qué | dónde |
|---|---|---|
| espiga E1–E7 | lo que el diseño necesitaba saber, medido y tirado | — |
| P0 | la garantía en vivo, medida | — |
| P1 · P2 | `Nulabilidad`; la spec v1alpha22 | ORE `bab2fb1`, OOS `a678ef1` |
| P3 | declarar | ORE `88c6c09` |
| P4 | derivar | ORE `84ce9b6` |
| P5 | evolucionar antes de imponer | ORE `c97fa9b` |
| P3′ | los árboles vivos, al día | forjas `305a726` (`victor`), `740e182` (`demo`) |
| P6 | imponer | ORE `d69b90a`; forjas `606dae8`, `b9793da` |
| P7 | enseñar: GraphQL, el SDK y el aviso | ORE `73a6c03`, OOS `5f577fb` |

## Fuentes

- [Apache Iceberg spec](https://iceberg.apache.org/spec/) · [valores por defecto en Iceberg v3](https://www.dremio.com/blog/dremio-iceberg-v3-default-column-values/) · [Snowflake: NOT NULL ADD COLUMN en Iceberg v3](https://docs.snowflake.com/en/release-notes/bcr-bundles/2026_05/bcr-2351)
- [BigQuery: modificar esquemas](https://docs.cloud.google.com/bigquery/docs/managing-table-schemas) · [BigQuery: esquemas](https://docs.cloud.google.com/bigquery/docs/schemas)
- [Databricks: restricciones](https://docs.databricks.com/aws/en/tables/constraints) · [Delta Lake: restricciones](https://docs.delta.io/delta-constraints/) · [Snowflake: restricciones](https://docs.snowflake.com/en/sql-reference/sql/create-table-constraint)
- [dbt: model contracts](https://docs.getdbt.com/docs/mesh/govern/model-contracts)
- [Spark: NullPropagation](https://jaceklaskowski.gitbooks.io/mastering-spark-sql/content/spark-sql-Optimizer-NullPropagation.html) · [Spark, Parquet y los nulos](https://medium.com/@weshoffman/apache-spark-parquet-and-troublesome-nulls-28712b06f836) · [Calcite RelDataTypeFactory](https://calcite.apache.org/javadocAggregate/org/apache/calcite/rel/type/RelDataTypeFactory.html)
- [Debezium, PostgreSQL](https://debezium.io/documentation/reference/connectors/postgresql.html) · [Airbyte #68516](https://github.com/airbytehq/airbyte/pull/68516) · [Airbyte #76476](https://github.com/airbytehq/airbyte/issues/76476)
- [Foundry: data expectations](https://www.palantir.com/docs/foundry/pipeline-builder/dataexpectations-overview) · [ODCS v3.2.0](https://bitol-io.github.io/open-data-contract-standard/v3.2.0/schema/)
