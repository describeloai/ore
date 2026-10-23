# Las escrituras — de dónde sale lo que hay en el árbol

> **Estado: propuesta (2026-10-23), sin código.** Plan de producto para que **toda** escritura en el
> árbol —desde una hoja de Query, una sesión de un repositorio, un notebook, un Build o la API— quede
> registrada igual, y que el linaje, la clasificación, la salud y la historia de cada activo salgan de
> ese registro. No es un ADR: lo será al cerrar la etapa.

**Por qué ahora.** Los Transforms son la excepción, no la regla: el grueso de lo que se escribe en el
árbol se escribe **desde la sesión** —y, con Query, desde la hoja—. Hoy esas escrituras dejan un rastro
parcial, distinto según el camino, y el gobierno (la clasificación que baja) depende de él.

---

## 1. Lo que hay hoy (comprobado en el código y en victor, 2026-10-23)

Una escritura deja **tres rastros**, cada uno incompleto y leído por alguien distinto:

| rastro | dónde | lo escribe | qué dice | quién lo lee |
|---|---|---|---|---|
| **la procedencia** | el puntero, `datasets/<b>/<s>/<n>.json` (`procedencia`) | el SDK la **manda** al escribir (`_procedencia`, `ore/__init__.py`) | dentro de un `@transform`: `inputs` + `transform`; fuera: `leidas`; un Build añade `build` | el linaje (`ore_core::assets`, sólo `leidas` como respaldo), la salud (`datos_de`) |
| **`derivedFrom`** | el documento del activo (`spec.derivedFrom`) | `ore` al confirmar (`ore-cli/src/datasets.rs` `leyo_de`; colecciones: `ore-serve/src/escritas.rs`) | lo leído, **sólo lo que tiene documento de dataset, vista o colección** | **la clasificación** (`ore_core::flow::lo_que_lee`, gobierno ③) y el linaje |
| **el `Transform`** | `pipeline/<salida>.yaml` | el commit del repositorio | `inputs` declarados | la clasificación y el linaje (sólo Builds) |

Y la historia de un activo existe sin que nadie la enseñe: **cada escritura es un commit del puntero**
en la forja (`git log` del puntero), con la procedencia de esa escritura dentro.

**Los huecos:**

1. **Lo leído que no es un dataset, una vista o una colección se pierde de `derivedFrom`** (`existe`,
   `datasets.rs`, que además busca el documento por `<nombre>.yaml` y no por el árbol: un dataset cuyo
   fichero se llama distinto también se cae): una **Table** de un origen o de una foreign database, una
   lectura en vivo. Por ahí no
   baja la clasificación: un dataset escrito desde la sesión a partir de una tabla clasificada de un
   origen **no hereda su clasificación**. Un Build sí (sus `inputs` declarados entran en `flow.rs`).
2. **El linaje depende del camino.** Con `derivedFrom` vacío, el respaldo es `procedencia.leidas`: lo
   escrito con `ore.write()` en Python lo tiene; lo escrito con SQL en la sesión no (`ore-serve` lo
   envuelve en un `@transform`, que deja `inputs`). Y el respaldo pinta lo que no conoce como **View**
   (`assets.rs`), aunque sea una Table.
3. **Un transform que no existe.** La celda que `ore-serve` genera para el SQL que escribe
   (`celda_de_unidad`, `puestos.rs`) usa `@transform` sólo para acotar lo que lee y escribe, y la
   procedencia queda con `transform: "<nombre del .sql>"` o `"consulta"`. Lo mismo con las media
   collections derivadas.
4. **Lo leído lo dice el cliente.** `leidas` e `inputs` los manda el SDK; una celda que habla con
   `ore-serve` por su cuenta, o un SDK viejo, dice lo que quiera.
5. **No se sabe desde dónde.** Ni la hoja de Query, ni el notebook, ni la sentencia: sólo `puesto`.

En victor (rama `main`, 26 datasets escritos): **12 con `derivedFrom`, 14 sin él**. De los 14:

- 6 Builds, cubiertos por su `Transform`;
- `productos_por_categoria` y `productos_por_plataforma` (SQL en la sesión, `transform: "ejemplo"`)
  leyeron `standard_test.public.products`, **que es un dataset**, pero su documento se llama
  `Products__products.yaml` y `existe` busca `products.yaml`: se cayó de `derivedFrom` sin decirlo;
- `vara_derivado1` (Python) leyó `vara_pg.ore_vara.pedidos`, una Table de un origen Postgres: fuera;
- `contratos_meta` y `paginas_b5` (SQL, `transform: "test"` y `"paginas_b5"`) están anclados a la
  colección que leen (`anchoredTo`), que es su relación: bien;
- `vara_derivado`, `m0d_marimo` y `medida_g2_puntos` (Python) dicen `leidas: []`: o no leyeron nada
  del árbol o el SDK no lo vio, y **no se puede saber**.

Ninguno desde una hoja de Query: aún no se ha escrito desde ella.

## 2. Cómo lo hace Databricks (lo publicado)

- **El linaje lo captura el motor al correr, no lo declara el código.** Unity Catalog registra cada
  lectura y cada escritura de cada consulta —notebooks, el SQL editor, jobs, dashboards, pipelines—,
  hasta la columna, sin cambiar nada en el código
  ([data lineage](https://docs.databricks.com/aws/en/data-governance/unity-catalog/data-lineage)).
- **Un registro por evento, con la entidad que lo hizo.** `system.access.table_lineage` y
  `column_lineage`: origen → destino, `entity_type` (`NOTEBOOK`, `JOB`, `PIPELINE`, `DBSQL_QUERY`,
  `DASHBOARD_V3`… o nada si no hubo entidad), `entity_id`, `entity_run_id`, `entity_metadata`
  (`notebook_id`, `sql_query_id`, `job_info`…), `created_by`, `event_time` y `statement_id`, que enlaza
  con la historia de consultas (el texto, las filas y los bytes escritos)
  ([lineage system tables](https://docs.databricks.com/aws/en/admin/system-tables/lineage),
  [query history](https://docs.databricks.com/aws/en/admin/system-tables/query-history)).
- **La historia de la tabla:** cada escritura es una versión con su operación, quién y cuándo
  (`DESCRIBE HISTORY`, [table history](https://docs.databricks.com/aws/en/delta/history)).
- **Un job o un pipeline es una entidad más**, no un camino aparte: el mismo registro, con su
  `job_run_id`.

## 3. La propuesta

**Una escritura es un hecho que registra el servidor, no algo que declara el cliente.** Cada vez que
algo se escribe en el árbol, `ore-serve` deja **una escritura** con lo que se leyó para hacerla y desde
dónde, y todo lo demás —linaje, clasificación, salud, historia— la lee a ella. Un `Transform` es una
entidad más, con una garantía de más (declara antes lo que leerá y se reconstruye igual), no el único
camino que deja rastro.

### 3.1 La escritura

Una por commit de un activo (dataset, colección), en el commit del puntero —que ya es la historia—:

```
escritura:
  activo:   dataset:ventas.default.limpios        # o media_collection:…
  version:  <snapshot | transacción>
  modo:     sobrescribir | anexar | upsert | crear
  cuando, quien                                   # la persona (o la cuenta del Build)
  filas, bytes
  lee:                                            # TODO lo leído, de cualquier clase
    - { ref: table:origen_pg.public.clientes, version: …, en_vivo: true }
    - { ref: dataset:ventas.default.pedidos, version: <snapshot> }
    - { ref: view:ventas.default.activos, directo: true }      # y lo que la vista leyó, directo: false
  desde:                                          # la entidad, como entity_type de Databricks
    tipo:     query | sesion | notebook | build | api
    id:       queries/limpieza.sql | packages/…/repo | build-…
    ejecucion: <celda o build>
    commit:   <del código, si lo hay>
    sentencia: <el SQL, o la celda>               # lo que corrió, para la historia
```

### 3.2 Lo leído lo cuenta `ore-serve`, no el SDK

Toda lectura de una sesión ya pasa por `ore-serve`: resuelve cada nombre y presta su credencial
(`GET /puestos/{id}/datos/…`, las fuentes de `ore.sql`, las colecciones, `/federation/read`). Lo que se
leyó entre la apertura de la escritura y su confirmación se apunta **ahí**, en el servidor —para una
sentencia SQL, además, lo dice la sentencia ya cotejada—. El SDK deja de mandar `leidas`/`inputs`.

### 3.3 El SQL que escribe deja de fingir un transform

La celda que `ore-serve` genera para una sentencia que escribe sigue acotando lo que lee y escribe a lo
que la sentencia dice (es lo que hoy hace el `@transform`), pero con un **alcance de sentencia**, no
con un transform: la escritura queda `desde: query` (la hoja) o `desde: sesion`, con la sentencia.

### 3.4 Quién lee la escritura

| lector | hoy | con la escritura |
|---|---|---|
| **la clasificación** (`flow.rs`) | `derivedFrom` (sin Tables) + `inputs` del Transform | `derivedFrom`, que se calcula de `lee` **entero**, Tables y lecturas en vivo incluidas |
| **el linaje** (`assets.rs`) | `derivedFrom`, o `leidas`, pintando de View lo desconocido | `lee`, con la clase de cada cosa leída |
| **la salud** (`datos_de`, 0063) | de la procedencia | de `lee[].version` |
| **la historia del activo** (consola) | no hay | `git log` del puntero: quién, cuándo, desde dónde, la sentencia, filas; enlace a la hoja de Query o al Build |
| **los Builds** | `build` en la procedencia | `desde: build` |

`derivedFrom` sigue en el documento —es el contrato de la clasificación, normativo y revisable en un
commit—, pero se escribe **siempre** desde `lee`.

## 4. Las decisiones

| | decisión | recomiendo | por qué |
|---|---|---|---|
| **DE1** | dónde vive la escritura | **en el commit del puntero** (sustituye a `procedencia`) | la forja ya guarda un commit por escritura: la historia no necesita una tabla nueva |
| **DE2** | quién cuenta lo leído | **`ore-serve`** | es por donde pasa toda lectura; lo que dice el cliente no gobierna |
| **DE3** | `derivedFrom` con Tables y en vivo | **sí** | si no, la clasificación de un origen no baja a lo que se escribe desde la sesión |
| **DE4** | las columnas | **después** (tabla primero) | Databricks llega a la columna; aquí el reparto ya sabe las columnas de una sentencia SQL, pero no las de Python |
| **DE5** | los 26 escritos de victor | **rehacer su escritura** con lo que dicen sus punteros, y marcar las que no se pueden saber | no quedan activos sin linaje por haber sido escritos antes |

## 5. El plan

| paso | qué | hecho cuando |
|---|---|---|
| **E0** | **el previo**, sin código: listar cada camino por el que se escribe en el árbol (SDK Python, Java, Node, SQL en la sesión, guiones, colecciones, el catálogo REST desde fuera, el Build, la copia de un origen) y por dónde pasa cada lectura en `ore-serve`; ver si alguna lectura no pasa por él | la lista, y sabido si DE2 cubre todo |
| **E1** | **la escritura en `ore-core`**: el tipo, su JSON, `derivedFrom` desde `lee`, y el linaje y la clasificación leyéndola (con pruebas de una Table clasificada que baja a lo escrito desde la sesión) | las pruebas |
| **E2** | **`ore-serve` cuenta lo leído** por sesión entre apertura y confirmación, y escribe la escritura en el commit del puntero; el SDK deja de mandar la procedencia (y se acepta la vieja mientras convivan) | lo escrito desde Python, SQL y un Build deja la misma escritura, medido en victor |
| **E3** | **el SQL sin transform fingido**: el alcance de sentencia en vez del `@transform`; `desde: query` con la hoja | una escritura desde una hoja de Query sale en el linaje con su hoja |
| **E4** | **la historia en la consola**: una pestaña *History* del activo con cada escritura | se ve quién, cuándo, desde dónde y la sentencia de cada versión |
| **E5** | **los escritos de antes** (DE5) | ningún escrito de victor sin linaje |

Fuentes: [Databricks · data lineage](https://docs.databricks.com/aws/en/data-governance/unity-catalog/data-lineage) ·
[lineage system tables](https://docs.databricks.com/aws/en/admin/system-tables/lineage) ·
[query history](https://docs.databricks.com/aws/en/admin/system-tables/query-history) ·
[table history](https://docs.databricks.com/aws/en/delta/history).
