# 0038 · Assets catalog namespaces

**Estado:** **aceptado · en vivo** (2026-09-24) · **Decide:** cómo se nombra lo que un inquilino
tiene en el catálogo de assets: **`base.schema.nombre`**, tres niveles como en Unity Catalog, el
mismo nombre en los documentos, en SQL, por `/v1`, en los SDK y en la consola. Gramática:
[OOS v1alpha13 `01-el-schema`](../../vendor/oos/spec/v1alpha13/01-el-schema.md). Se apoya en
[`0033`](0033-el-dataset.md) (el dataset) y [`0034`](0034-el-catalogo-de-assets.md) (el
catálogo, que se ordena base → schema → tabla, dataset, vista).

## Qué es

| nivel | es | lo declara |
|---|---|---|
| **base** | el paquete (su `namespace`: letras, dígitos y `_`) | `package.yaml` |
| **schema** | el segundo nivel del nombre | un `kind: Schema` en `packages/<base>/<schema>/schema.yaml` |
| **nombre** | una `Table`, `View`, `Dataset`, `Entity`, `Function`, `Action` o `Model` | el documento, con `metadata.schema` |

- **Un nombre es único en su schema**, porque el schema es parte del nombre: `ventas.espana.pedidos`
  y `ventas.francia.pedidos` son dos cosas. Unity identifica una tabla igual
  (`catalog_name.schema_name.table_name`).
- **`default`** es el schema de lo que no dice otro, y existe sin declararse. Ni `default` ni
  `information_schema` se declaran (`OOS1004`).
- **La identidad nunca es la ruta** (OOS `90-canonical-form` §5.2): el schema lo **declara** el
  documento, y la carpeta tiene que estar de acuerdo —`OOS2036` (la carpeta no es la del schema) y
  `OOS2037` (el schema no está declarado)—, igual que `OOS2030` ata el `namespace` al paquete.
  Una carpeta vacía no existe en git; un schema declarado, sí, y lleva su dueño
  ([`0052`](0052-ownership.md)).
- **Un repositorio no es un schema**: vive en el paquete de su proyecto ([`0035`](0035-el-proyecto.md),
  [`0036`](0036-code-repositories.md)), y lo que escribe va a un schema de una base.
- El vocabulario compartido (`Concept`, `Interface`) no tiene schema.

## Las referencias

- **Una parte**: el mismo paquete y el mismo schema de quien la escribe.
- **Tres partes**: completa.
- **A una propiedad**: la propiedad es siempre el último segmento (`hr.rrhh.Employee.salary`).

Dentro del motor la clave es la **forma corta** (`normalize::corto`): `base.nombre` en `default`,
`base.schema.nombre` en otro. Es biyectiva con la completa y es la que ven `qname()`, el índice
(con el `schema` de cada ítem), ore-serve, los SDK y la consola. La **completa**, tres partes
siempre, es la de la forma canónica de v1alpha13 y su `docId`. Un documento de antes de v1alpha13
está en `default` y no cambia de forma canónica ni de digest.

## Las dos partes

`base.nombre` se admite y es `base.default.nombre`, **pero se dice**: el aviso `ORE-SQL-2P` en
`ore sql`, en la celda (junto a su salida), en el editor —que ofrece el nombre de tres partes— y
en un `.sql` como trabajo. Una parte o cuatro, en SQL, es un fallo.

## En cada motor

- **`/v1` (Iceberg REST), como Unity**: `config?warehouse=<base>` devuelve `prefix`, **la base es
  el prefix y el schema el namespace** de un nivel. PyIceberg, DuckDB y Spark nombran entonces
  `ventas.espana.pedidos`, lo mismo que el SQL. Listar, crear, cargar, escribir y
  `commitTransaction` hablan `base.schema.tabla`; un namespace que no está es 404. Sin `prefix`,
  el namespace es la base y lo que hay es lo de `default`. `ore-entrada` deja pasar de la URL sólo
  `warehouse`, y sólo si es un identificador.
- **DuckDB** (`sql()` en los tres SDK, el editor, `ore ask --sql`): **un catálogo por base**
  (`attach ':memory:' as <base>`) con sus schemas dentro. Un nombre de dos partes, DuckDB lo busca
  en `<base>.main`, así que lo de `default` lleva ahí su alias. Un nombre que no está se sugiere
  entero (`¿ventas.espana.clientes?`).
- **Los SDK** (Python, Node, JVM): `write()`, `over()`, `transform()` y `declare()` aceptan dos y
  tres partes y trabajan con la forma corta; fuera de `default`, por las rutas de tres partes.
- **ore-serve**: las rutas de un nombre tienen tres partes —`/datasets/{b}/{s}/{n}`,
  `/vistas/{b}/{s}/{n}/ejecutar`, `/funciones/{b}/{s}/{n}/invocar`,
  `/documentos/{kind}/{b}/{s}/{n}`—, y las de dos partes son `default`.

## Dónde vive cada cosa

- **El documento**: `packages/<base>/<schema>/<kind>/…`; lo de `default`, en la raíz del paquete.
- **El puntero** de un dataset: `datasets/<base>/<schema>/<n>.json`, con un separador que ningún
  nombre lleva (`a_b.c` y `a.b_c` no chocan). `ore_core::punteros` es el único que decide dónde
  vive uno; el de antes de 0038 (`<p>_<n>.json`, sólo `default`) se lee, y quien escribe un
  puntero lo deja en su sitio y retira el de antes. `ore migrate` los mueve todos.
- **Los bytes** de un dataset que nace: `catalogo/<base>/<schema>/<n>` en el lago. Lo que ya
  existía no se mueve: el puntero guarda su `metadata_location`. Lo que se recoge como huérfano es
  lo que ningún puntero del árbol reclama, por prefijo hasta una `/`.
- **Los resultados** de una función: `resultados/<b>_<f>` en `default` y `resultados/<b>/<s>/<f>`
  en otro schema.

## Crear y renombrar un schema

- **Crear**: `ore package schema new <b> <s>` (`POST /paquetes/{b}/schemas`) escribe su
  `schema.yaml`, con `owner` de quien lo crea. Se niegan `default`, `information_schema`, una
  carpeta de kind, lo que no es un identificador y lo que ya existe sin mirar mayúsculas (en SQL
  son el mismo).
- **Renombrar**: `ore package schema rename <b> <viejo> <nuevo>`
  (`POST /paquetes/{b}/schemas/{s}/renombrar`) es **un commit** que mueve la carpeta, reescribe
  `metadata`, las referencias de tres partes en `.yaml` y `.sql`, y los punteros (los bytes no se
  mueven), y deja un `moved` por nombre en el manifiesto para que `ore diff` lo vea como lo que
  es. El código (`.py`, `.java`) no se reescribe: se dice qué queda por tocar. **La puerta**: si
  el árbol de después tiene un diagnóstico que el de antes no tenía —traducido al nombre nuevo—,
  no se escribe nada.
- **La consola** crea y renombra por el servidor, pinta como schemas sólo `default` y los
  declarados (una carpeta como `transforms/` no lo es), y lo que crea desde un schema —Create ›
  View, Dataset; Run— nace y corre en él.

## Lo que trae una fuente

`discover` lleva **el schema del origen al del catálogo**: `public.ai_insights` es
`foreign_test.public.ai_insights`, cada tabla con su nombre y cada schema declarado. Si un schema
descubierto se renombra, el alcance (`discover.scope.json`, `schemas: {origen: nuevo}`) lo
recuerda, y la siguiente inducción emite ya en el nuevo. **La re-inducción retira sólo lo que
escribió el inductor** (lo marca su primera línea, `# ore discover: …`): lo escrito a mano en un
schema descubierto, o un schema creado desde el catálogo, no se toca nunca.

## Los pasos que cita el código

| | qué |
|---|---|
| **P0** | la gramática, OOS v1alpha13 |
| **P1** | la identidad en el núcleo: forma corta y completa, `OOS2036`/`2037`, el índice |
| **P2** | los punteros y el lago por schema; lo huérfano se reclama por todo puntero del árbol |
| **P3** | SQL de tres partes: el analizador (P3a), ore-serve (P3b), los SDK (P3c), el editor (P3d), `ore ask` (P3e) |
| **P4** | `/v1` como Unity y `ore datasets` con tres partes |
| **P5** | `discover` con el schema del origen |
| **P6** | crear y renombrar (P6a), la consola en su schema (P6b), funciones y Jobs (P6c), el catálogo pinta los declarados (P6d) |
| **P7** | las semillas nombran en tres partes (`mi_base.mi_schema.mi_dataset`) |

## Aceptación

- **Conformance** v1alpha13: 15/15, y las demás versiones enteras. Sobre el árbol de victor, el
  binario de antes y el de después dan el mismo `validate`, los mismos ítems y el mismo digest.
- `los-schemas.sh` (crear, renombrar en un commit, re-inducir en el schema nuevo); `el-lago.sh` 10b
  y 15 (lo huérfano; PyIceberg y DuckDB por `/v1` con prefix); `el-puesto.sh` 10 (tres partes desde
  Python, Node y Java, `declare()` en un schema); `la-invocacion-se-decide.sh` 8 y
  `la-copia-se-decide.sh` 8b (funciones y copias fuera de `default`); `tests/reinduccion.rs`.
- Lo medido antes de decidir: `medida-los-tres-niveles.py`, `medida-los-punteros.sh`,
  `medida-el-sql-de-tres-partes.sh`, `medida-v1-como-unity.py`, `medida-discover-con-schema.py`,
  `medida-renombrar-schema.py`, `medida-borrador-en-schema.sh`, `medida-forge-con-schema.sh` y
  `medida-catalogo-schemas.sh`.
