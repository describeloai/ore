# 0038 · Los tres niveles: `base.schema.nombre`, como en Unity Catalog

**Estado:** decidido (P0–P4 hechos; P5–P7 pendientes) · **Fecha:** 2026-09-24 ·
**Decide:** cómo se nombra lo que un inquilino tiene en el catálogo —en los documentos, en SQL,
por `/v1` y en la consola—, ahora que el **schema** es parte del nombre. Sigue a
[`0033`](0033-el-dataset.md) (el dataset), [`0034`](0034-el-catalogo-de-assets.md) ④ (el
schema como carpeta, que esto corrige) y a la gramática,
[OOS v1alpha13](../../vendor/oos/spec/v1alpha13/00-scope.md).

## El problema

Los datos del cliente viven en el **catálogo de assets**, que se ordena como cualquier catálogo:
**base de datos → schema → tabla / dataset / vista**. La consola ya lo pinta así (0034 ④). El
lenguaje no: un nombre era `paquete.nombre`, y el schema, la carpeta entre el paquete y el
fichero, **ordenaba sin nombrar**. Medido (`pruebas-de-fuego/medida-los-tres-niveles.py`,
2e0a384 · 7e6401d · ce736d7):

- Dos `pedidos` en `ventas/espana/` y `ventas/francia/` son **`OOS2035`**: el nombre es único por
  paquete, no por schema.
- **Los árboles no tienen schemas**: el 100% de los assets (76 en el de victor, 5000 en el
  sintético) cae en `""`. Y no por falta de uso: **«Create schema» de la consola no llama al
  servidor** (sólo estado de React; se pierde al recargar) y **una carpeta vacía no existe en
  git**.
- **El origen sí los tiene, y se aplanan**: `discover` escribe `public.ai_insights` como la
  tabla `public_ai_insights` (38 de 38 en `foreign_test`; `identificador()`, inductor.rs).
- **Los punteros ya chocan**: `a_b.c` y `a.b_c` son el mismo `datasets/a_b_c.json`; borrar el
  paquete `ventas` se lleva los punteros de `ventas_eu` (copia.rs).
- El nombre de dos partes está en **~90 sitios de 11 piezas** (analizador, `sql()`, `/v1`,
  `write()` en los tres SDK, LSP, índice, ore-view, consola).

## Lo decidido (2026-09-24)

1. **Tres niveles, como Unity Catalog.** `<base>.<schema>.<nombre>`. La base es **el paquete**
   (su `namespace`: letras, dígitos y `_`); el schema, un segundo nivel del nombre.
2. **Único por schema**, porque el schema es parte del nombre. Comprobado en Unity: la API
   identifica una tabla por `catalog_name.schema_name.table_name`.
3. **`default`** es el schema de lo que no dice otro (Unity crea `default` en cada catálogo).
4. **Dos partes se admiten por ahora** y se resuelven a `base.default.nombre`, pero **se tratan**:
   no pasan en silencio (§ «Las dos partes»).
5. **`discover` lleva el schema del origen al del catálogo**: `foreign_test.public.ai_insights`.
6. **Repositorios y schemas no se mezclan**: un repositorio (README con `plantilla`) vive en el
   paquete de su proyecto; lo que escribe va a un schema de una base del catálogo. Un paquete de
   proyecto (`test-project`) no es una base: su nombre ni siquiera es un `namespace`.

## La gramática (P0, hecho): OOS v1alpha13

La especificación decide lo que no podía decidir ORE: **la identidad nunca es la ruta**
(90-canonical-form §5.2). Así que el schema **se declara** y la carpeta se ata a él, como
`OOS2030` ata el `namespace` al paquete:

- `kind: Schema` (`packages/<base>/<schema>/schema.yaml`): hace existir el schema —una carpeta
  vacía no existe— y lleva su dueño. Ni `default` ni `information_schema`.
- `metadata.schema` en el contenido gobernado (Entity, View, Table, Dataset, Function, Action,
  TrainedModel); sin ella, `default`. El vocabulario compartido no la lleva.
- `OOS2036` (la carpeta no es la del schema) y `OOS2037` (el schema no está declarado).
- Referencias: **una parte** = mismo paquete y schema; **dos** = `base.nombre` → `base.default`;
  **tres** = completo. A una propiedad: la propiedad es el último segmento, siempre.
- Un documento anterior no cambia de forma canónica ni de digest: está en `default`.

## Lo que ORE hace con ello

**Los motores, como Unity.**
- **`/v1` (Iceberg REST): la base es el `prefix`** (el `warehouse` del cliente) **y el schema, el
  namespace de un nivel**. Es lo que hace el Iceberg REST de Unity, y Spark nombra entonces
  `ventas.espana.pedidos`, el mismo nombre que el SQL. Los namespaces de dos niveles (`0x1F`) no
  hacen falta —y hoy ni llegarían: ore-entrada tira la query y no decodifica `%1F`—.
- **DuckDB: un catálogo por base** (`ATTACH ':memory:' AS <base>`, schemas dentro): 0.33 ms por
  vista con 5000. `default` se escribe sin comillas. Pero **dos partes van a `main`**, no a
  `default`: mientras se admitan, cada vista de `default` lleva su alias en `main`.

**Los punteros**: `datasets/<base>/<schema>/<nombre>.json`, un separador que ningún nombre lleva
—arregla el choque de hoy—. El lago no se mueve: el puntero guarda `metadata_location`.

**Las dos partes, tratadas.** Donde aparezcan —un `.sql`, una celda, un YAML— resuelven a
`base.default.nombre` **y lo dicen**: un aviso con código propio en `ore validate`, en el editor
(el LSP ofrece el nombre de tres partes) y en la salida de la celda. Más adelante, error.

## El plan

| # | Paso | Tamaño |
|---|---|---|
| **P0** | La gramática: OOS v1alpha13 (spec, esquemas, errores) y esta ADR | hecho |
| **P1** | La identidad en el núcleo: `qname()`, `qualify()`, `metadata_keys()`, `pertenencia` (`OOS2036`/`2037`), `sin_propiedad()`, el índice; la conformidad de v1alpha13, medida. Sin schema = `default`: los árboles de hoy compilan sin tocarlos | hecho |
| **P2** | Los punteros `datasets/<base>/<schema>/<nombre>.json` y su migración | hecho |
| **P3** | SQL de tres partes (medido y partido, § P3; hecho): P3a el analizador (hecho), P3b ore-serve (hecho), P3c los tres SDK (ATTACH + alias, hecho), P3d el LSP (hecho), P3e `ore ask` (hecho; `ore datasets`, en P4); las dos partes con aviso `ORE-SQL-2P` | grande |
| **P4** | `/v1` como Unity (base = prefix, schema = namespace) y `ore datasets` con tres partes | hecho |
| **P5** | `discover` con el schema del origen | pequeño |
| **P6** | La consola: schemas de verdad (crear/renombrar al servidor), refs de tres partes, borradores en la carpeta del schema | medio |
| **P7** | Las semillas (SQL, Python, Java) escribiendo en la base y el schema que el asistente pregunte | pequeño |

Cada paso termina en verde de punta a punta antes del siguiente.

## P1, hecho: la identidad en el núcleo

**La clave del motor es la forma CORTA** (`normalize::corto`): `p.n` en `default`, `p.s.n` en
otro schema. Es biyectiva con la completa —lo de `default` tiene dos partes y lo demás tres—, así
que dos documentos nunca comparten clave, y es la que todo el que ya habla con el motor escribe:
`qname()`, las búsquedas, `OOS2035`, el índice y los consumidores de fuera (ore-serve, los SDK, la
consola) ven lo mismo que antes para todo lo de hoy. La **completa** (`completo`, tres partes con
`default`) es la de la forma canónica de un documento de v1alpha13 y su `docId`.

- `Loaded::schema()` (lo declarado o `default`, en los siete kinds del catálogo) y `qname()` corto;
  las búsquedas (`entity/view/table/dataset`) aceptan las dos formas (`a_corto`).
- `normalize::qualify_catalogo` / `link::cualificar`: una parte = su paquete **y su schema**; dos =
  `default`; tres = completa. `qualify` sigue siendo la del vocabulario compartido. Cambiadas donde
  lo nombrado es del catálogo: enlazado, vistas, `fuente`, `actuar` (`call`), `cedar_schema`,
  `governance` (la función de un deber), `exporta` (según el kind de destino) y el índice
  (`ref_doc`, `ref_qn` con el schema de quien enlaza).
- `sin_propiedad`: la propiedad es **siempre** el último segmento (con tres niveles, adivinar por
  el número de puntos cortaba `hr.rrhh.Employee` a `hr.rrhh`; y `Employee.salary` no se cortaba).
- `ApiVersion::V1Alpha13`, `Kind::Schema` (en `DEL_PAQUETE`: su namespace es el paquete),
  `metadata.schema` desde v1alpha13 (antes, o en el vocabulario, `OOS1005` con su porqué) y
  `schema.rs` (`OOS2036`, `OOS2037`, `default`/`information_schema`, el `owner`), tras la
  pertenencia y antes del enlazado.
- Forma canónica de v1alpha13: N1 a tres partes (y `from.dataset`, que v1alpha12 no expandía),
  N2 escribe `schema: default`, `docId` de tres partes. Lo anterior, igual.
- El índice lleva `schema` en cada ítem (`null` en el vocabulario).

**Medido**: conformidad v1alpha13 **15/15** (OOS `d0f2628`, los `expects` medidos antes de
escribirse) y las demás versiones enteras; sobre el árbol de victor el binario de antes y el de
después dan el mismo `validate`, los mismos 58 ítems con las mismas refs y **el mismo digest de
bundle**; el índice sólo gana el campo `schema`.

## P2, hecho: los punteros

Medido antes (`pruebas-de-fuego/medida-los-punteros.sh`), y con tres hallazgos que no eran de
nombres sino de **borrar lo que no se debía**:

- **P2a** · El Job de la copia (`ore materialize --recoger`) le pasaba a `recoger-huerfanas`
  sólo los datasets de las vistas mantenidas: **un dataset escrito se borraba entero del
  bucket** (M1: 4 → 0 objetos) y su puntero quedaba apuntando a nada; `resultados/`, igual,
  también en la pasada diaria. Ahora se reclama **todo puntero del árbol**
  (`datasets::reclamados`), y el de una vista retirada se quita antes. el-lago 10b.
- **P2b** · `recoger-huerfanas` cortaba el nombre a dos segmentos (un dataset anidado era
  huérfano aunque se reclamara, M3) y el mantenimiento de una tabla borraba lo que hubiera bajo
  su ubicación: `datasets/ventas_x` es prefijo de `datasets/ventas_x/default/n`. Ahora se
  reclama por prefijo hasta una `/`, y una tabla sólo toca sus `metadata/` y `data/`.
- **P2c** · Los punteros, en su sitio. `ore_core::punteros` decide dónde vive el de cada
  nombre (`datasets/<base>/<schema>/<n>.json`), lee el de antes (`<p>_<n>.json`) mientras
  quede, dice qué nombre es cada fichero y los lista a cualquier profundidad; todo el que leía
  o escribía uno a mano pasa por ahí (`ore datasets`, `ore materialize`, el índice, ore-serve:
  copias, informes, retirar un Dataset o un paquete, preguntar, funciones, puestos).
  **Quien escribe un puntero lo deja en su sitio y retira el de antes**, y `ore migrate` los
  mueve todos (y los de `copias/`) escribiendo el `dataset` que el fichero ya no dice. Borrar
  un paquete retira los punteros **de su base** —el nombre que dice cada uno, no un prefijo del
  fichero (M4)—.

**El nombre en el lago** de un dataset que nace es `catalogo/<base>/<schema>/<n>`, no
`datasets/<base>/…`: `datasets/ventas_x` (de antes) sería prefijo de `datasets/ventas_x/default/n`,
y la credencial prestada al primero —acotada a su prefijo— alcanzaría el segundo. Lo que ya
existe no se mueve. Y **el `dataset` de un puntero sale de su `metadata_location`** siempre que
se sabe: es donde están los bytes, lo que tiene que reclamar, aunque lo haya elegido otro
escritor (el swap de `confirmar`).

Lo que queda de nombres de dos partes (`/v1`, los SDK, `ore datasets --tabla p.t`) es de P3/P4.

## P3: medido, y el analizador hecho

**Medido** (`pruebas-de-fuego/medida-el-sql-de-tres-partes.sh`, un puesto de Python de verdad
sobre un árbol con `ventas.espana` declarado): con dos partes, todo; con tres, **nada llega**, y
cada pieza falla por su cuenta y con su frase —`write()` y `over()` las rechazan en el SDK,
`sql()` y la celda las dejan pasar crudas a DuckDB (`Catalog "ventas" does not exist`: el
tokenizador descartaba `a.b.c` en silencio), la celda que escribe no se reconoce como tal, el LSP
ofrece `ventas.clientes` para lo que está en `espana` (un nombre que no existe) y
`/v1/namespaces/espana/tables` da 200 vacío—. El núcleo, en cambio, ya estaba: `validate`, el
índice (`schema` de cada ítem) y los punteros.

La partición, por dependencias: **P3a** el analizador → **P3b** ore-serve (la celda y el
aviso en su salida, `datos_del_puesto`, `declarar_transform`, `datos_de`, el `Dataset` que
nace en un schema en v1alpha13 y en su carpeta) → **P4** `/v1` (base = prefix, schema =
namespace; `write()` escribe por ahí, así que va antes que los SDK) → **P3c** los tres SDK →
**P3d** el LSP → **P3e** `ore datasets --tabla` (y el `identificador()` que se queda con el
último nivel del namespace) y `ore ask --sql`.

**P3a, hecho.** `sql_del_arbol::Nombre` lleva `schema` y `referencia()` es la forma corta (lo
de `default` sigue siendo `p.n` para todos los que ya lo consumen); `completo()`, las tres.
`analizar` acepta `base.schema.nombre`; `base.nombre` es `default` **con un aviso**
`ORE-SQL-2P` por nombre (`Unidad::avisos`: no para la frase; `Fallo::codigo`); una parte o
cuatro, fallo. `cotejar` dice el schema que no está declarado. `nombres_a_resolver` y
`escribe_en_el_arbol` reconocen `a.b.c` (en su forma corta) en vez de descartarlo. `ore sql`
enseña los avisos (`aviso[ORE-SQL-2P]`, y `avisos` en `--json`).

**P3b, hecho.** ore-serve, el lado del puesto: `datos_del_puesto` acepta dos o tres partes (a
su forma corta: la clave del árbol, la de los punteros y la del transform); `declarar_transform`
también; `datos_de` busca en el árbol compilado por la forma corta —antes, por carpetas
(`packages/<p>/datasets|views|tables`), que no ven el schema—, y `datos_de_vista` parte los
datasets de la pregunta con su schema. **El aviso llega a la celda**: `avisos_de_celda` (el
tokenizador: lo que se lee y el destino, en su sitio, una vez) da los `ORE-SQL-2P` de una celda
`sql`, lea o escriba, y los de un `.sql` como trabajo salen de `Unidad::avisos`; van en la ficha
de la celda (`avisos`, diagnósticos con `severidad: aviso`), junto a la salida. Medido después:
el servidor ya resuelve `ventas.espana.clientes` (el 409 de «nadie lo escribió todavía»), y lo
que falla con tres partes es ya sólo del SDK (`write()`, `over()`, `transform()`, el DuckDB de
`sql()`) y de `/v1`. **`ore datasets` con tres partes pasa a P4**: trabaja con pares
`(paquete, tabla)` y carpetas, y es lo que `/v1` llama —con base = prefix y schema =
namespace—; el `Dataset` que nace en un schema (v1alpha13, en su carpeta) se escribe ahí.

## P4, hecho: `/v1` como Unity

**Medido** (`pruebas-de-fuego/medida-v1-como-unity.py`, un catálogo de mentira que apunta cada
petición): PyIceberg con `warehouse=ventas` y DuckDB con `ATTACH 'ventas'` piden
`/v1/config?warehouse=ventas` y usan el `prefix` que vuelve —`/v1/ventas/namespaces/espana/…`—;
DuckDB nombra entonces `ventas.espana.pedidos` y `ventas.default.pedidos`, el mismo nombre que el
SQL. Sin `warehouse`, las rutas de siempre.

- **ore-entrada** dejaba fuera la cadena de consulta entera («ningún dato entra por la URL»), y
  `warehouse` está ahí por la spec. Entra **sólo** `warehouse` (`CONSULTA_ADMITIDA`), y sólo si
  es un identificador: es un nombre, no un dato.
- **`/v1`**: `config?warehouse=<base>` da `overrides.prefix` (una base que no está, 404). Con
  `prefix`, **la base es el prefix y el namespace su schema**: `namespaces` son `default` y los
  declarados, las tablas y vistas las de ese schema, y crear, cargar, escribir y
  `commitTransaction` nombran `base.schema.tabla` (a los identificadores del cuerpo se les pone
  la base delante). Sin `prefix`, el namespace es la base y lo que hay es de `default`: nada
  cambia para quien no lo pide. Un namespace que no está es 404, no una lista vacía.
- **`ore datasets`** trabaja con `Tabla { base, schema, tabla }`: `--tabla`, `--crear`,
  `--cargar`… aceptan tres partes; un `Dataset` que nace en un schema se escribe en
  `packages/<base>/<schema>/datasets/` en v1alpha13 con su `metadata.schema` (el árbol compila);
  en un schema no declarado, no; `--tabla` manda sobre el `identifier` del cuerpo; e
  `identificador()` lee `[base, schema]` (antes se quedaba con el último nivel).

el-lago 15, con PyIceberg y DuckDB de verdad: `espana.pedidos2` nace por el catálogo con su
documento, su puntero en `datasets/ventas/espana/` y su lago en `catalogo/ventas/espana/`, y
DuckDB lee `ventas.espana.pedidos2`.

## P3c, hecho: los tres SDK

Python, Node y la JVM, lo mismo en los tres:

- **Los nombres**: `write()`, `over()`, `transform()` y `_por_posicion` aceptan `base.nombre` y
  `base.schema.nombre`, y trabajan con la forma corta (`ventas.default.x` es `ventas.x`).
- **`sql()`**: cada nombre del árbol es una vista de DuckDB con sus tres niveles —`attach if
  not exists ':memory:' as <base>`, `"<base>"."<schema>"."<n>"`— y lo de `default` lleva su
  alias en `"<base>"."main"."<n>"`, donde DuckDB busca un nombre de dos partes (medido: así
  resuelven `ventas.pedidos`, `ventas.default.pedidos` y `ventas.espana.clientes`, y
  `ventas.clientes` falla sugiriendo `ventas.espana.clientes`). Fuera el `create schema "p"` en
  `memory`: con un catálogo del mismo nombre, la referencia es ambigua (medido).
- **Dentro de una vista de un catálogo adjunto, un schema sin cualificar se busca en ESE
  catálogo** (medido en el-puesto 10b): los datasets de una View (`"__ore_dataset"."p.n"`) se
  nombran con `memory.` delante.
- **`/v1` con prefix**: `write()` y el préstamo de la credencial piden
  `/v1/<base>/namespaces/<schema>/tables/<n>` (P4).

Medido después (`medida-el-sql-de-tres-partes.sh`): con el agente de Python, `write()`, `sql()`,
`over()` y la celda que escribe, con `default` y con `espana`, de punta a punta; `ore validate`
limpio y el índice con su schema. el-puesto: tres partes desde Python, Node y Java.

## P3d y P3e, hechos: el editor y `ore ask`

**El LSP** (`puesto/python/ore/lsp_sql.py`) tenía su catálogo por `paquete.nombre` —con tres
partes ofrecía `ventas.clientes` para lo que está en `espana`, un nombre que no existe (medido)—
y un `create schema "p"` en su DuckDB. Ahora: un recorrido de nombres (`a.b` y `a.b.c`, a la
forma corta); el catálogo por la clave de cada ítem (su `schema` del índice), un catálogo de
DuckDB por base y lo de `default` con su alias en `main`, como `sql()`; tras FROM, los nombres
enteros (`ventas.default.pedidos`) y las bases; tras `base.`, sus schemas; tras `base.schema.`,
sus nombres; **el aviso `ORE-SQL-2P`** como diagnóstico (severidad aviso, `code`), una vez por
nombre; un nombre que no está se sugiere entero (`¿ventas.espana.clientes?`); y el hover, por
el nombre entero.

**`ore ask --sql --catalogo`** (lo que `loadView` sirve) nombraba cada dataset `"p"."n"`, que
sin `prefix` es lo que el cliente ve. Con `--base` (ore-serve lo pone cuando la petición trae
`prefix`) el namespace es el schema: lo de esa base se nombra `"schema"."n"` y lo de otra
`"base"."schema"."n"`. el-lago 15: una View en `espana` se lista en su schema (y no en la base
sin prefix), con `default-namespace: [espana]` y su SQL sobre `"espana"."pedidos2"`.

## P5, hecho: `discover` lleva el schema del origen

`foreign_test.public.ai_insights` es `foreign_test.public.ai_insights`, y no
`foreign_test.default.public_ai_insights` (medido: `medida-discover-con-schema.py`). El schema
es el segmento anterior a la tabla (`public.x` → `public`; `proyecto.dataset.x` → `dataset`);
lo del origen va a `<schema>/tables|views|datasets|entities`, en v1alpha13 con
`metadata.schema`, y el paquete declara cada schema en `<schema>/schema.yaml` (con `owner` sólo
si es un handle: el `cambiame` del paquete ya lleva la pregunta, y dos `OOS2009` por lo mismo
sobran). La tabla se llama por su nombre (`clientes`, no `rubix_demo_ventas_clientes`); la
pregunta de colisión lleva el schema sólo cuando la misma entidad sale en más de uno (las
respuestas de antes siguen valiendo); una relación a otro schema se escribe en tres partes.
`review` limpia también `<schema>/<dir>`.

**P5b · `ore package`** (medido al mover una base descubierta): la Table y su View se llaman
igual, y el grafo por `qname` las fundía en un nodo. Ahora la clave es `Kind:qname`; el
documento `Schema` no es un nodo; mover algo de un schema copia su declaración al destino, y
fundir retira el `Schema` que el destino ya declara.

**ore-serve**: los `.yaml` de un kind se leen de la raíz del paquete y de cada schema
(`yamls_del_kind`); el esquema de una base dice el `schema` de cada tabla y entidad; y las rutas
hablan tres partes —`/datasets/{b}/{s}/{n}` (y `confirmar`), `/vistas/{b}/{s}/{n}/ejecutar`,
`/documentos/{kind}/{ns}/{s}/{n}`—, con las de dos partes para `default`. `/funciones` sigue en
dos partes.

## P6a, hecho: los schemas de verdad (crear y renombrar)

La consola tenía «Create schema» y el doble clic de renombrar, y los dos cambiaban el estado del
navegador: un schema creado desaparecía al recargar. Ahora van al árbol.

**`ore package schema new <p> <s>`** escribe `<p>/<s>/schema.yaml` (con `--description` y
`--owner`); una carpeta que ya está se adopta. Se niegan `default`, `information_schema`, las
carpetas de kind (`tables`…) y lo que no es un identificador (65); lo que ya existe, sin mirar
mayúsculas —para SQL son el mismo— (73); un paquete que no hay (66).

**`ore package schema rename <p> <viejo> <nuevo>`**. Medido antes
(`medida-renombrar-schema.py`, un árbol descubierto con lo que lo nombra desde fuera): con la
carpeta, la metadata y las referencias de tres partes, el árbol compila igual que antes; sin el
`moved`, `ore diff` ve veinte `OOS5007` (cada Entity y cada View «borradas»); con él, un
`minor`. Son seis cosas:

1. la carpeta, entera: lo de dentro se nombra en una parte y viaja sin tocarlo;
2. `metadata.name` del `Schema` y `metadata.schema` de lo que declara, por posición;
3. lo que lo nombra en tres partes (`<p>.<viejo>.x`) en los `.yaml` y `.sql` del árbol —por
   texto: una referencia de tres partes no se confunde con nada, y así llega a un `exports`, a
   un `writes: p.s.E.prop` o a SQL—; en un manifiesto, lo que sigue a `from:` es historia y no
   se toca. **El código (`.py`, `.java`…) no se reescribe: se dice** (`aMano`);
4. los punteros, de `datasets/<p>/<viejo>/` a `datasets/<p>/<nuevo>/`: los bytes del lago no
   se mueven —el puntero dice dónde están, y sigue siendo cierto—;
5. un `moved` por nombre en el manifiesto;
6. **la regla del alcance**: lo descubierto vive en la carpeta del schema del origen, y la
   siguiente inducción (`review`, `model`, `copy`) lo volvería a emitir allí —dos carpetas, las
   mismas tablas—. `discover.scope.json` guarda `schemas: {origen: nuevo}` y el inductor lo
   aplica (`Regla::schemas`) al emitir: dónde y cómo se llama lo emitido; las preguntas siguen
   con el schema del origen. Volver al nombre del origen quita la entrada. El catálogo entero
   de una fuente (sin alcance) no se renombra (73): su Job lo re-induce tal como el origen lo
   nombra.

**La puerta** es la de siempre —el árbol no empeora— y la pasa `ore`: si la compilación de
después tiene un diagnóstico que la de antes no tenía, se deshace todo (65). Lo de antes se
compara traducido: un `OOS2010` sobre `ventas.viejo.X` que ya estaba es el mismo defecto sobre
`ventas.nuevo.X`, no uno nuevo (sin esto, cualquier árbol con errores previos en el schema
rechazaría todo renombrado).

**ore-serve**: `POST /paquetes/{p}/schemas` `{name, description?, owner?}` → 201 y
`POST /paquetes/{p}/schemas/{s}/renombrar` `{to, since?}` → 200, por el camino de `model` y
`copy` (un clon, `ore`, un commit del sujeto; 65/66/73 → 422/404/409). `los-schemas.sh`: el
índice trae el schema recién creado aunque esté vacío; renombrar es UN commit y el árbol queda
con los mismos diagnósticos; copiar una tabla después re-induce en el schema nuevo.

**La consola** (rubix-platform): en una base del árbol, el modal crea por el servidor (y sigue
abierto si dice que no) y el doble clic renombra por el servidor, con el código que queda por
tocar como aviso aparte; `default` y renombrar una base se dicen en vez de fingirse en local.

Queda de P6: las referencias de tres partes en los editores de la consola y los borradores en
la carpeta del schema.

## Lo que no cambia

El paquete sigue siendo la base (nada que migrar), el lago físico igual, y el proyecto sigue
siendo una lente sobre el catálogo (0035).
