# El dataset en ORE — brief de construcción (desechable)

**Qué es esto.** El plan por el que `kind: Dataset` (OOS v1alpha12, `vendor/oos f8c73ed`)
pasa de spec a **realidad en ORE**: primero **la unidad** (ore-core sabe qué es un dataset y
la conformance está verde), y después **el plumazo** (ore-cli, ore-serve, malla, SDK, consola y
los árboles cambian a la vez, y lo legacy —`materialized`, `datasource: lago`, `copias/`— se
va). Se tira cuando esté hecho: lo que quede vale se recoge en [0033](decisions/0033-el-dataset.md).

**Lo que ya está.** La decisión (0033), la medida de sitios (0033 «Lo medido»: 14 sitios de
decisión en ORE + `datasets.rs` + 6 de flujo en la consola), la spec y su schema (oos
`f8c73ed`), y la lista de la conformance (13 casos, abajo).

**Lo que no se toca.** La `Table` (v1alpha8) entera; la `View` salvo `from: {dataset}` y la
retirada de `materialized`; el motor de vistas (`ore-view`, `ore-maintain`: planifican sobre
la raíz y no saben de kinds); `ore-store` salvo el nombre del prefijo; los árboles de
conformance v1alpha1–11.

---

## 1 · La unidad: ore-core sabe qué es un dataset

Un solo crate, un solo commit, y su criterio de hecho es **la conformance v1alpha12 en verde
con la de v1alpha1–11 sin un cambio**. Nada de fuera de `ore-core` se toca en este paso.

| | qué | dónde hoy | qué cambia |
|---|---|---|---|
| **D1** | el kind y su forma | `document.rs`: `ApiVersion::V1Alpha12`, `Kind::Dataset` (carpeta `datasets/`), claves de `metadata` (name, namespace, description) y de `spec` (owner, from, fields, where, groupBy, having, freshness, columns, changes, history); `pertenencia.rs` DEL_PAQUETE; `normalize.rs` (`key` en CONJUNTOS) | la `ShapeRule` de las dos formas: **exactamente una** de `from` / `columns`+`changes` (`OOS1004`), lo del plan sólo con `from`, `changes.mode ∈ {append, upsert}`, `upsert` ⇒ `key`, `history` con al menos un eje. Lo que el schema JSON ya niega (21 documentos), negado aquí con el mismo código |
| **D2** | la cadena | `vistas.rs`: `Fuente` (`Vista`/`Tabla`/`Datasource`), `fuente()`, `cadena()`, `raiz()`, `raiz_de_lectura()` (= «la vista `materialized` más cercana»), `Package::view/table/resolve_*` | `Fuente::Dataset(qn)`; `Package::dataset()`/`resolve_dataset()`; `cadena()` **sigue** por un dataset mantenido (tiene `from`) y **toca suelo** en una `Table` o en un dataset **escrito**; `raiz()` devuelve la misma `Raiz` con `tabla: None` y las columnas del escrito cuando el suelo es un dataset; `raiz_de_lectura()` = **el primer `Kind::Dataset` bajando** (ella misma incluida). Es el único sitio del núcleo donde se distinguía la copia, y sigue siéndolo |
| **D3** | lo que expone | `expone()`, `campos()`, `columnas()`, `tipos_de_columnas()`, `agregados()` | un dataset mantenido expone como una vista (`fields` o, sin `fields`, **todo lo que `from` expone con sus nombres**: la identidad, que hoy no existe porque una View exige `fields`); un escrito expone `columns` y sus tipos como una tabla. `OOS2018` contra lo que `from` expone, con `{dataset}` como tercer caso |
| **D4** | los enlaces | `link.rs`: `entities()` (`backedBy` → `resolve_view`), `respaldo()`; el `owner` → `OOS2009` (como `modelos_entrenados`) | `backedBy` resuelve a **View o Dataset** (una función `respaldo()` con dos búsquedas); `from.dataset` → `OOS2018`; ciclo por dataset → `OOS2019`; `OOS2011`/`OOS2022` leídas sobre lo que el respaldo expone |
| **D5** | la costura | `flow.rs:539–580` (`section("materialized")` instancia `materialization.payload`, `OOS4011`/`OOS4002`); `vistas.rs` `OOS2020` (1582), `OOS2021`, `OOS2023` (1635), `OOS2024`, `OOS2025` (1198) | cada regla cambia **su predicado** («declara `materialized`» → «es un dataset mantenido» / «su raíz de lectura es un dataset») y **no su código ni su mensaje**. `OOS2021` además sobre un escrito con `changes.mode: append`. `OOS2025`: la entidad que una Function toca está respaldada por un dataset, directo o por raíz de lectura |
| **D6** | lo que se retira | `document.rs:573,691` (`materialized` como clave de View v1alpha7/8); `diff.rs:593` («deja de copiarse») | en **v1alpha12** `View.materialized` es `OOS1005` con el remedio en el mensaje; en v1alpha7–11 sigue igual. El diff «una vista deja de copiarse» pasa a ser «un dataset desaparece» (ya lo cubre la retirada de documento) |
| **D7** | la conformance | `crates/ore-cli/tests/conformance.rs` (marcador `borrador_de_v1alpha11`) | `conformance/v1alpha12/` en oos (13 casos) + el marcador; **todo lo anterior sin cambio** |

**La conformance v1alpha12** (se escribe en oos en el mismo paso, antes del código):

| grupo | casos |
|---|---|
| las dos formas | `a-copy-with-its-plan` ✓ · `a-dataset-written-by-code` ✓ · `a-dataset-with-both-forms` OOS1004 · `a-dataset-with-neither` OOS1004 |
| compone | `a-view-over-a-dataset` ✓ · `a-dataset-over-a-view` ✓ · `a-field-the-dataset-does-not-expose` OOS2018 · `a-chain-that-comes-back` OOS2019 |
| la costura no se pierde | `a-copy-without-a-conduit` OOS4011 · `a-copy-that-leaks-an-entity-label` OOS4002 · `an-append-dataset-backing-an-entity` OOS2021 — **los mismos tres casos de v1alpha7/8 con un Dataset donde había una View, y el mismo código** |
| lo que se retira / no es de aquí | `a-view-that-still-says-materialized` OOS1005 · `a-dataset-in-v1alpha11` OOS1003 |

**Criterio de hecho de la unidad:** `cargo test -p ore-core` y `conformance.rs` verdes
(v1alpha12 13/13; v1alpha1–11 idénticos); `ore validate` sobre un árbol con las dos formas y
una vista y una entidad encima: cero diagnósticos; `ore view` enseña el linaje pasando por el
dataset. Nada más.

---

## 2 · El plumazo: la nueva realidad, de una pieza

Cuando la unidad está, **todo lo que buscaba `materialized` o `datasource: lago` pasa a buscar
`Kind::Dataset`**, en un solo cambio coordinado (un commit por repo: ORE, consola). No hay
periodo con las dos cosas vivas: el árbol de un inquilino es de antes o de después, y lo
decide la migración (§3).

### ore-cli

| sitio | hoy | después |
|---|---|---|
| `inductor.rs` (`ore discover`, base *standard*) | emite `View` + `materialized: {datasource, table: copia.<v>}` por tabla | emite **`Dataset` identidad** por tabla: `from: {table}`, sin `fields`, con `freshness` si la base lo pide. **Ninguna View por tabla**: la base de 30 tablas son 30 Tables + 30 Datasets, y cero vistas que sean la misma cosa. `# Ni freshness ni materialized` → `# Ni freshness` |
| `materializar.rs` | filtra `Kind::View && section("materialized")` (94, 929); escribe `copias/<p>_<v>.json`; `copiar` sobre lago si la raíz es `datasource == "lago"` | itera **datasets mantenidos** del paquete; el plan sale de `raiz()` igual; puntero en **`datasets/<ns>_<n>.json`**; «sobre el lago» = **la raíz de lectura es un dataset** (ya no hay `datasource: lago` que mirar) |
| `datasets.rs` | `asegurar_lago` (declara el datasource `lago`), `asegurar_table` (escribe `Table datasource: lago`), `seguir_esquema`, `puntero_del_lago`; lista `copias/ ∪ datasets/` | `asegurar_dataset` escribe un **Dataset escrito** (`columns` desde Iceberg, `changes` desde lo que `write()` pidió); `asegurar_lago` **se va** (no hay datasource); `seguir_esquema` igual sobre `columns`; una sola carpeta de punteros: `datasets/` |
| `registro.rs` | «las que el paquete declara: una View con `materialized`» | los datasets mantenidos del paquete |
| `preguntar.rs`, `vista.rs`, `invocar.rs` | «la vista no declara `materialized`» (422); dónde sostener una edición; `over` debe declarar copia | `ask`: «ningún dataset contesta a `v`» (la raíz de lectura no es un dataset); `ore view`: la edición se sostiene en el dataset de la cadena; `invoke`: `over` tiene raíz de lectura dataset (0029 ③ con el nuevo predicado) |
| `main.rs`, `autoria.rs` | prosa de ayuda | prosa |

### ore-serve

| sitio | hoy | después |
|---|---|---|
| `copia.rs` (10) | `vistas_con_copia*` busca `spec.materialized` en `views/*.yaml`; `copiar_tabla` escribe una View con `materialized`; `clase_de` cuenta la clase por la clave | busca `kind: Dataset` con `from` en `datasets/*.yaml`; `copiar_tabla` escribe un **Dataset identidad**; `clase_de`: *standard* = hay datasets mantenidos |
| `rutas.rs:1405` (`/esquema`) | `copiada` = la View de la tabla tiene `materialized` | `copiada` = hay un Dataset con `from: {table: ésta}` sin `fields`/`where` (la identidad); y `/esquema` gana los datasets del paquete (0034 (a)) |
| `funciones.rs:337` | `over` tiene `materialized` | `over` tiene raíz de lectura dataset (`ore_core::vistas::raiz_de_lectura`) |
| `puestos.rs:1269–1318` (`datos_del_puesto`) | View con copia → `copias/`; Table `datasource == "lago"` → `datasets/` | **un solo camino**: el nombre resuelve a un Dataset (o a una View cuya raíz de lectura lo es) → su puntero en `datasets/`; el mensaje «ni Table del lago» → «ni dataset» |
| `documentos.rs` KINDS | — | fila `Dataset` (carpeta `datasets`, «el dataset», `sin_exigencias`): `declare()` desde un puesto ya lo declara |
| `datasets.rs` (`GET /datasets`, ficha, `confirmar`) | lista dos carpetas; `confirmar` hace nacer una `Table` | una carpeta; `confirmar` hace nacer un **Dataset escrito**; la ficha dice **forma** (mantenido / escrito), `de` (`from` o `procedencia`) e **identidad** (0034 (b)) |

### malla, SDK, store

| sitio | hoy | después |
|---|---|---|
| `aprovisionar-inquilino.sh:1049` | `re.search(r"^\s+materialized:")` decide si se rinde el Job de la copia | `re.search(r"^kind:\s*Dataset")` + `from:` (o mejor: `ore datasets --mantenidos` y que lo cuente el CLI) |
| `48-la-copia.yaml`, `cola.rs::rendir_copia` | la lista de «vistas con copia» | la lista de datasets mantenidos (mismo hueco, mismo nombre) |
| `puesto/{python,node,jvm}` | `write()` deja lo que el servidor decida; mensajes «ni Table del lago» | **la API no cambia**; `write(nombre, df, mode, key)` ya lleva lo que `changes` necesita; mensajes «ni dataset». `declare()` con `kind: Dataset` funciona por KINDS |
| `ore-store` (`lago.rs`, `ciclo.rs`, `sobre.rs`) | prefijos `ore/v2/copias/…` y `ore/v2/datasets/…` | **nuevas** escrituras van a `ore/v2/datasets/`; las tablas que ya existen bajo `copias/` **no se mueven**: el puntero lleva `metadata_location` absoluto, y una tabla Iceberg no sabe cómo se llama su prefijo |

### consola (local, un commit)

| sitio | después |
|---|---|
| `SchemaDetail.tsx`, `CatalogClient.tsx` | «Create › View › Standard / Materialized» → **«Create › View»** (la pregunta) y **«Create › Dataset»** (from table, identidad o con plan). Y la sección **Datasets** del schema (0034 ②) leyendo `GET /datasets` filtrado por paquete |
| `lib/server/borrador-de-vista.ts` | sin `materialized`; nuevo `borrador-de-dataset.ts` que escribe `kind: Dataset` con `from: {table}` |
| `DatabaseTypeSelect.tsx` | «`materialized` en todas = standard» → «hay datasets mantenidos» |
| `ramas.tsx`, `workspaces/page.tsx` | `?nuevo=dataset&base=&tabla=` en vez de `materialized=1` |
| `lib/server/documentos.ts`, `como-sql.ts`, `ontology/{datos,Views,Explore}.tsx` | el tipo de View pierde `materialized`; la etiqueta «materializada / virtual» desaparece (una vista es siempre la pregunta); el comentario SQL dice «lee de: dataset x» |
| `lib/banco/*`, `ontology/acme.ts` | el mock migra igual que un árbol |

---

## 3 · La migración: los árboles de antes pasan a después

Un verbo nuevo, **`ore migrate v1alpha12 <árbol>`** (hoy no existe `migrate`: las migraciones
anteriores fueron a mano con un recuento en `tests/migracion.rs`), mecánico y sin opinión:

| antes | después | nota |
|---|---|---|
| `View v` con `materialized` | **`Dataset v` con el plan de `v` dentro** (`from`, `fields`, `where`, `groupBy`, `having`, `freshness`); la View **se va** | lo que se tiene, con su plan: quien la leía (`from: {view: v}`, `backedBy: v`) sigue leyendo la copia. Decidido al construir: «View sin `materialized` + Dataset `from: {view: v}`» dejaba la vista **virtual** y a quien la respaldaba leyendo del origen, no de la copia |
| …y una `Function`, `Action` o `TrainedModel` nombra `v` como vista (`over`, `reads`, `trainedFrom`) | además, `View v { from: {dataset: v}, fields: identidad }` | la pregunta sobre su dataset, con el mismo nombre (distinto kind): `over: v` sigue teniendo raíz de lectura dataset |
| `Table t` con `datasource: lago` | `Dataset t { columns, changes }` (`changes` de `Table.changes`: `mode` y `key`) | y `datasource: lago` sale de `ontology.config.yaml` si nadie más lo usa |
| `copias/<p>_<v>.json` | `datasets/<p>_<v>.json` | gana `tabla` (= `vista`) y `dataset: copias/<p>_<v>` (el prefijo del bucket, que no cambia: los bytes no se mueven) |
| `View x { from: {view: v} }` donde `v` pasó a ser sólo Dataset | `from: {dataset: v}` | |
| `Entity e { backedBy: v }` | igual | resuelve a lo que quede con ese nombre |

**Criterio:** el árbol migrado compila con **los mismos diagnósticos** (o menos) que antes,
`GET /datasets` devuelve **la misma lista** (mismos punteros, mismos snapshots), `ask`/`sql`
sobre cada vista devuelve **las mismas filas**, y `ore view` el mismo linaje. Se corre sobre
`demo` y `victor` en el clúster **como medida, antes de aplicarlo** (lectura; sin instancias),
y se aplica por el aprovisionador (un commit firmado `ore migrate`) o a mano.

---

## 4 · Orden, criterio de hecho y lo que se mide

| paso | qué | hecho cuando |
|---|---|---|
| **0** | conformance v1alpha12 escrita en oos (13 casos), en rojo | **hecho** (oos `9255ad5`): el marcador `borrador_de_v1alpha12` da **1 / 13** (sólo `a-dataset-in-v1alpha11`, que ya es OOS1003), sin regresiones |
| **1** | la unidad (D1–D7) | **hecho** 2026-09-21: conformance **13/13**, v1alpha1–11 sin cambio (15/15 marcadores), `cargo test --workspace` verde, clippy `-D warnings` limpio. Fuera de ore-core sólo `ore-cli/vista.rs` (dos `match` exhaustivos sobre `Fuente`, que ganó `Dataset`). Lo que salió al construir: `raiz_de_lectura` conserva `materialized` para v1alpha7/8 (compat); `suelo()` es la función nueva por la que OOS2021/2024 leen las caras de una `Table` o de un dataset escrito; `expone_en(pkg, d)` es lo que un dataset expone (identidad sin `fields`); `proyectar` sigue usando `campos` para una vista (un test de `ore view` sella los agregados por derivación, no por flujo) |
| **2** | `ore migrate v1alpha12` + medida sobre `demo`/`victor` (cuántos documentos cambian, y que compila igual) | **hecho** 2026-09-21: `crates/ore-cli/src/migrar.rs` (`--seco` es la medida); `pruebas-de-fuego/medida-migrar-dataset.py` trae el árbol de cada forja por un Job de lectura y lo mide en local. **demo** (`febfa77`, 34 docs): 3 Views con `materialized` → 3 Datasets; 2 Views se van y 1 queda como la pregunta sobre su dataset (una Function la nombra en `over`); `lago` sale del manifiesto; 3 punteros movidos; `ore validate` 0 → 0; `ore datasets` la misma lista (3). **victor** (`b8cf669`, 73 docs): 32 Views, **las 32** con `materialized` → 32 Datasets y **cero Views** (nadie preguntaba: eran la tabla con otro nombre); 32 punteros; 0 → 0; la misma lista (32). Ninguna `Table` del lago en ninguno de los dos. Lo que salió: el ciclo de `cadena()` se miraba por nombre y una vista y su dataset se llaman igual (ahora por kind+nombre); el puntero movido gana `tabla` (el nombre se lee de ahí en `datasets/`) y `dataset: copias/<p>_<v>` (los bytes no se mueven) |
| **3a** | el plumazo en Rust y malla (ore-cli, ore-serve, malla, los cinco scripts de fuego) | **hecho** 2026-09-21: todo lo que buscaba `materialized` / `datasource: lago` busca `Kind::Dataset` (`es_copia`, `raiz_de_lectura`, `suelo`, `destino_de`); los punteros viven en `datasets/` (los migrados conservan `dataset: copias/…`); el inductor emite `datasets/<sufijo>` con el plan y ninguna vista por encima; `write()` deja un Dataset escrito (`columns` + `changes`) con el owner del paquete; `asegurar_lago` se va; lo declarado sin `apiVersion` es v1alpha12. `el-lago.sh` 0–14, `el-puesto.sh` 1–11 (python, node, jvm), `la-copia-se-decide.sh` 0–10, `la-pregunta-se-contesta.sh` 0–9, `refresco.sh` (lo semántico; los conteos de objetos son la rareza `KeyCount` del S3 de mentira) verdes **con Dataset**; `gen-inquilino --comprobar` verde; `cargo test --workspace`, clippy `-D warnings`. Lo que salió: un dataset sin `fields` proyectaba nada (identidad por `expone_en`); el testigo de un dataset sobre otro dataset sale del puntero, no de la raíz de la cadena; los `datasets/` inducidos se retiran en la re-inducción como las vistas (GOBERNADOS por `__`); `over` sobre un Dataset es OOS7014, la Function pregunta por una View encima; el 400 de escribir un mantenido dice qué es |
| **3b** | los bordes: mensajes de los tres SDK, grep de residuos a cero fuera de compat | **hecho** 2026-09-21: el 404 de `over()` en python/node/jvm dice «ninguna `View` ni `Dataset`» y las cabeceras de `write()` dicen que deja un `Dataset` escrito; los tests de `datasets.rs` (`seguir_esquema`, `con_cambios`, `columnas_del_documento`) se ejercitan sobre un Dataset escrito y no sobre una Table del lago; los comentarios de malla y del store no nombran `datasource: lago`. `grep 'section("materialized")\|datasource: lago' crates/*/src malla puesto` fuera de `migrar.rs`: **2**, ambos compat o ayuda —`vista.rs::destino_de` (una vista de v1alpha7/8 con `materialized`, donde ella dijo) y el `--help` de `ore migrate`—. `copias/` sólo en punteros migrados (`dataset: copias/…`) y en los nombres de tabla de los tests del store. el-puesto 1–11 verde otra vez; `cargo test --workspace`, clippy `-D warnings` |
| **4** | migrar `demo` y `victor`; CI verde; el aprovisionador converge | **hecho** 2026-09-21: `pruebas-de-fuego/migrar-a-dataset.py` (un Job por inquilino corre `ore migrate v1alpha12 .` dentro con el `ore` de la imagen; exige diagnósticos iguales o menos y los mismos punteros; `--empujar` lo firma `ore migrate`, `--comprobar` pregunta al ore-serve vivo). **demo** `febfa77 → 97d12b3` (3 datasets, 1 vista queda por su Function, 0 → 0, 12 ficheros); **victor** `d10b1ed → c5a1342` (32 datasets, 0 vistas, 0 → 0, 111 ficheros). `GET /datasets` del ore-serve vivo: demo 3, victor 32, los mismos que antes. El aprovisionador convergió solo a los 5 min («datasets mantenidos: …»), rindió el Job de la copia, y la copia corrió por el camino nuevo: raíces por `ore view .`, credenciales del cofre, `ore materialize --informe datasets`, punteros empujados a `datasets/` (demo `b93ed52`, victor `20a8251`; 9 de 32 copiadas en victor —las otras 23 y las 3 de demo, el Postgres de origen sobre cuota, ajeno a esto y como estaba). Lo que salió en el camino: la CI sacó dos scripts sin convertir y la costura del catálogo (`nodo_de`); `48-la-copia` seguía en `copias/` y resolvía fuentes con un awk sobre `kind: View` |
| **5** | la consola | **hecho** 2026-09-21 (rubix-platform `e95155a`, local): `GET /datasets` por el proxy (`lib/server/datasets.ts`), la ficha del esquema lista Datasets en una standard (con el estado del puntero) y Tables en una foreign, Create › Dataset encima de View (planos; se va Standard | Materialized), `borradorDeDocumento(kind)` escribe v1alpha12 sobre el dataset inducido; `materialized` fuera del tipo View, de como-sql, de la ontología, del banco. `tsc` limpio; `next build` no se corrió (el `next dev` del usuario ocupa `.next`). Pendiente y fuera de este paso: la página del code workspace es WIP de la sesión de Forge y sigue leyendo `?nuevo=view&materialized=1`; `borradorDeVista` queda de puente hasta que lea `?nuevo=dataset` |
| **6** | 0033 pasa a «hecho»; este brief se borra | |

**Lo que se mide y se apunta en 0033 al cerrar:** documentos por árbol antes/después;
`materialized` en `crates/*/src` (hoy ~90 ocurrencias) → lo que quede es compat v1alpha7/8;
`ms` de `ore validate` sobre `demo` antes/después; el `el-lago.sh` 14 (copiar sobre lago) con el
nuevo predicado.

---

## 5 · Decisiones que hay que tomar antes de arrancar (di sí o no)

1. **Una sola carpeta de punteros: `datasets/`** y `copias/` se va (79 sitios en 18 ficheros;
   `git mv` en los árboles; el bucket no se mueve). Propuesta: **sí** — un kind, un puntero;
   si «copia» sobrevive como carpeta, sobrevive la distinción que 0033 retira.
2. **`discover` para una base *standard* emite Dataset identidad y ninguna View por tabla.**
   Propuesta: **sí** — es la razón de 0033; las vistas las escribe quien pregunta.
3. **`ore migrate` como verbo del CLI** (y no un script): lo corre el aprovisionador y queda
   como la migración de v1alpha8 debió quedar. Propuesta: **sí**.
4. **La conformance antes del código** (paso 0 antes del 1). Propuesta: **sí** — es el método.
5. **`freshness` en la View se va con `materialized`** (respondido 2026-09-21: el dataset la
   absorbe; una vista virtual lee en el momento y no tiene retraso que tolerar). En v1alpha12
   las dos claves son `OOS1005` con el remedio. D6 las retira juntas; la migración lleva
   `freshness` al Dataset.
