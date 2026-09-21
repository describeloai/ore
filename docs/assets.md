# El índice de assets: de la resolución de 0034 a una realidad en ORE

**Brief de construcción, desechable** (como lo fue `docs/dataset.md` para 0033). Lo que decide
está en [0034 ⑤](decisions/0034-el-catalogo-de-assets.md); esto es cómo se construye, en qué
orden, con qué criterio de hecho, y qué hay que decidir antes de arrancar. Se tira cuando esté
hecho: lo que quede vale se recoge en 0034 («Lo construido»).

**La frase:** el catálogo es `GET /assets` —el árbol compilado a una cabeza, proyectado a ítems
con sus relaciones y sus capas—, ore-core lo proyecta, ore-serve lo sirve de memoria por
commit, y la consola lo lee y nada más.

---

## 0 · La forma, medida antes del código

Antes de escribir `indice()` se escribe **lo que tiene que devolver** sobre dos árboles reales,
a mano, y se comprueba contra ellos. No es conformance de OOS (el índice no es gramática: es
una proyección) pero es el mismo método: el resultado esperado antes del código.

- `pruebas-de-fuego/medida-assets-indice.py`: trae demo y victor (el Job de lectura de
  `medida-migrar-dataset.py`), y sobre cada árbol **cuenta lo que el índice tiene que dar**:
  ítems por kind y por paquete, carpetas (con la regla de ⑤ 6), relaciones esperadas
  (`from` → `sale_de`/`produce`; `backedBy`; `over`/`reads`; `trainedFrom`), cuántos datasets
  son identidad (plan sin `where`/`groupBy`/`having` y `fields` = las columnas de abajo con su
  nombre), cuántas Tables de una foreign tienen `vistaInducida`.
- Esos números son el **oráculo** del paso 1: `indice()` sobre el mismo árbol tiene que darlos.

**Hecho cuando:** la tabla de números por árbol está en el brief (y luego en 0034).

**Hecho (2026-09-21).** `pruebas-de-fuego/medida-assets-indice.py`, sobre demo `b93ed52` y
victor `a6e2b0e`. Los oráculos:

| | demo | victor |
|---|---|---|
| ítems | **25** · Table 11, View 9, Dataset 3, Function 1, Model 1 | **77** · Table 38, View 19, Dataset 19, Model 1 |
| por paquete | `olist` Table 8 + View 8; `olist_copia` Dataset 3, Table 3, View 1, Function 1 | `foreign_test` Table 19 + View 19; `standard_test` Table 19 + Dataset 19 |
| sin paquete | 1 (`Model`) | 1 (`Model`) |
| carpetas (⑤ 6) | todo en `""` («sin clasificar») | todo en `""` |
| relaciones (una dirección; ×2 en el índice) | **14** · `sale_de` 12, `lee` 1, `usa` 1 · rotas 0 | **38** · `sale_de` 38 · rotas 0 |
| datasets identidad | 3 / 3 mantenidos (0 escritos) | 19 / 19 mantenidos (0 escritos) |
| views inducidas (detalle de su Table) / propias | 8 / 1 (la que quedó por la Function) | 19 / 0 |
| tables con `vistaInducida` | 8 / 11 | 19 / 38 |
| punteros | 3 (error 3) · 3/3 datasets con puntero | 19 (copiada 9, error 10) · 19/19 |
| no ítems | ConduitPolicy 1, OntologyConfig 1, Package 8 | ConduitPolicy 1, Lattice 1, OntologyConfig 1, Package 5 |

Lo que los números dicen: (1) **hoy ningún árbol tiene una carpeta del cliente**: el índice nace
con todo «sin clasificar», y ④ empieza cuando alguien cree el primer schema; (2) **cada dataset
de una standard es identidad**, y cada View de una foreign es la inducida: el catálogo de una
base recién creada lista exactamente sus tablas (foreign) o sus datasets (standard), y una
View propia sólo aparece cuando alguien la escribe (demo tiene una); (3) las relaciones son
casi todas `sale_de` (la cadena): `lee`/`usa` sólo donde hay una Function; `respaldada_por`,
`satisface`, `nombra`, `escribe`, `trainedFrom` tienen **cero** ejemplares en los árboles reales
y se prueban sólo sobre el árbol de fuego; (4) la mitad de los punteros de victor están en
`error` (el origen sobre cuota): el índice tiene que enseñar `puntero.estado` y `motivo` desde
el primer día.

**Y la decisión 3, comprobada:** un `.yaml` sin `kind` en una carpeta del paquete es
**OOS1002** (`falta apiVersion`: el compilador valida todo `.yaml`); un `README.md` en la
carpeta se ignora, y un documento movido a `packages/olist/ventas/` compila igual (`ok · sin
errores`, y `ore datasets` lo sigue viendo). **Un schema es una carpeta con un `README.md`**
(su descripción, en la primera línea); sin cambio en OOS.

## 1 · La unidad: `ore_core::assets::indice`

Un módulo nuevo en ore-core, `assets.rs`, con una función pura:

```rust
pub fn indice(pkg: &Package, punteros: &BTreeMap<String, Json>, cabeza: &Cabeza) -> Json
```

`pkg` es el árbol compilado (`validate::cargar_paquete`); `punteros` son los
`datasets/<p>_<n>.json` ya leídos (los lee quien llama: ore-serve o el CLI, no ore-core, que no
sabe de ficheros de estado); `cabeza` es `{cabeza, rama, generado}`.

**D1 · Los ítems.** Uno por documento de `packages/<p>/…` con kind de ②: `Dataset`, `Table`,
`View`, `Entity`, `Interface`, `Concept`, `Function`, `Action`, `TrainedModel`. `Model` (raíz)
y los `Concept` importados (`vendor/*.oob`) entran con `paquete: null`. `Package`,
`OntologyConfig`, `ConduitPolicy`, `Lattice`, `Ruleset`, `discover.*.json` no son ítems (los
tres primeros alimentan `acceso`; el último, `paquetes[].source/type`).

- `ref` = `kind:namespace.name` (minúscula el kind: `dataset:`, `table:`, `view:`, `entity:`,
  `interface:`, `concept:`, `function:`, `action:`, `trainedmodel:`, `model:`).
- Del documento: `kind`, `namespace`, `name`, `displayName` (`x-rubix-displayName` si está),
  `description`, `owner`, `labels` (`metadata.labels`), `ruta`.
- Derivado: `paquete`, `carpeta` (⑤ 6: entre `packages/<p>/` y el fichero, quitando la carpeta
  del kind si es la primera), `version` (**no** en ore-core: lo pone ore-serve desde
  `/arbol/historia`, ver D6).

**D2 · `define` y `expone`.** Para lo que tiene plan (View, Dataset mantenido):
`define.from` (la `ref` de abajo), `define.identidad` (sin `where`/`groupBy`/`having` y
`expone_en` = las columnas de abajo con su nombre), `define.fields` (cuántos), `where`,
`groupBy`, `having` (booleanos), `freshness`. Para un Dataset escrito: `define.columns` y
`define.changes`. Para una Table: `detalle.object`, `detalle.datasource`, `detalle.reads`,
`detalle.changes`, `detalle.columns` (con `physicalType`), `detalle.vistaInducida` (⑤ 5: la
View del paquete cuyo `from` es esta tabla, identidad, y nadie más que el inductor la nombra:
`__` en el fichero). `expone`: lo que sale con tipo, por `vistas::raiz`/`expone_en` y las
`columns` de la raíz (lo que `ore view` imprime en `esquema`).

**D3 · `puntero`.** Sólo en un Dataset: el resumen de su puntero (`estado`, `motivo`, `filas`,
`snapshot`, `ubicacion`, `metadata_location`, `cuando`, `leidas`). Si no hay puntero,
`puntero: null` («declarado, sin copiar»).

**D4 · `relaciones`.** Tipadas, **en las dos direcciones**, derivadas:

| de | tipo (en el ítem) | tipo (en el otro) |
|---|---|---|
| `from: {table\|view\|dataset}` (View, Dataset) | `sale_de` | `produce` |
| `procedencia.leidas` del puntero de un escrito | `sale_de` | `produce` |
| `backedBy` (Entity) | `respaldada_por` | `respalda` |
| `over`, `reads` (Function, Action) | `lee` | `leido_por` |
| `effects.writes` (Function) | `escribe` | `escrito_por` |
| `trainedFrom` (TrainedModel) | `sale_de` | `produce` |
| `implements` (Entity → Interface) | `satisface` | `satisfecha_por` |
| `concept` de una propiedad (Entity → Concept) | `nombra` | `nombrado_por` |
| `model` (Function → Model) | `usa` | `usado_por` |

Cada arista: `{tipo, ref}`. Una `ref` que no resuelve (un enlace roto que `validate` ya
denuncia) se emite igual con `rota: true`: el índice enseña lo que hay, no lo arregla.

**D5 · `acceso`.** Lo que ore-core computa por documento: `clasificacion` (las labels
efectivas de la raíz: `etiquetas_de_raiz` de ore-cli pasa a ore-core, o se reimplementa desde
`flow::lattices`) y `conductos` (para lo que copia: si `materialization.payload` compila,
`flow::clearances` + `comprobar`, lo que `ore view` imprime en `flujo`). Sin ore-iam.

**D6 · `version`.** `{commit, cuando, sujeto}` del último commit que tocó el fichero. Lo pone
**ore-serve** (tiene la forja), no ore-core: `indice()` deja `version: null` y ore-serve lo
rellena con un `git log -1 --format … -- <ruta>` por fichero **en el clon que ya tiene**
(125 ficheros × un `git log` local ≈ decenas de ms; se mide en 3).

**D7 · `paquetes[]`.** Por paquete: `name`, `type`, `scoped`, `source`, `owner`, `carpetas`
(las que hay), `items` (cuántos), y los contadores que `/paquetes` ya da.

**Tests** (`crates/ore-core/tests/assets.rs`): sobre un árbol de fuego escrito a mano con un
ejemplar de cada kind y cada relación; y los oráculos de 0 sobre demo y victor, cuando el
árbol se pueda traer (marcados `#[ignore]` fuera de CI, como las medidas).

**Hecho cuando:** los tests verdes; `ore assets <árbol> --json` (un verbo de lectura en
ore-cli, para la medida y para mirar) da los números del oráculo sobre demo y victor;
`cargo test --workspace`, clippy, rustfmt.

## 2 · `GET /assets` en ore-serve

- `assets.rs` en ore-serve: `GET /assets` (`?rama=`, `?commit=`). Con identidad, como todo.
- **La caché por cabeza.** `Api::ramas()` ya da `(rama, cabeza)` sin clonar: una llamada HTTP a
  la forja (ms). Si `cabeza` es la cacheada, se sirve de memoria; si no, `leyendo` (clon),
  `cargar_paquete`, leer `datasets/*.json`, `indice()`, `version` por fichero, y a la caché
  (`Mutex<HashMap<(rama, cabeza), Arc<Json>>>`, con un tope: las últimas N cabezas). Con
  `Arbol::Directorio` (los tests, el banco) no hay caché: siempre se calcula.
- `?commit=h`: `git checkout h` en el clon, y se cachea igual por `(rama, h)`.
- La respuesta lleva `cabeza`, `rama`, `generado`, `desde_cache: bool` (para la medida).

**Hecho cuando:** un test de ore-serve con `Arbol::Directorio` sobre el árbol de fuego de 1
sirve el índice; `plano-de-control` (el script de fuego) gana un caso `GET /assets` que
comprueba `items` y una relación en las dos direcciones; en CI verde.

## 3 · La medida, viva

`pruebas-de-fuego/medida-assets-catalog.py` gana `GET /assets`: código, ms **en frío** (primer
cálculo) y **en caliente** (misma cabeza), bytes; y se compara con lo que hoy cuesta pintar el
catálogo (~6 llamadas ≈ 3–5 s). Y comprueba, contra victor: `dataset:standard_test.orders`
dice `identidad: true`, `sale_de` su tabla, su puntero, y la tabla dice `vistaInducida` si es
foreign. Los números van a 0034.

## 4 · La consola sobre el índice (rubix-platform, commits locales)

- `lib/server/assets.ts`: `indiceDeAssets()` por el proxy (`{de: 'assets'}`), con los tipos
  del índice.
- `comoDatabase` + `esquemaDelPaquete` + `datasetsDelArbol` + `copiasDelPaquete` en el
  catálogo **se van**; `catalog/page.tsx` trae el índice una vez y `CatalogClient` pinta
  `paquetes › carpetas › items` (el árbol lateral por `carpeta`; «sin clasificar» = `""`).
- La ficha por kind lee el ítem: Dataset (la de 0033 paso 5, ahora con `define`, `relaciones`
  y `acceso`), Table (con `detalle`, y la View inducida como un dato, no una fila), View,
  Entity, Interface, Concept, Function, Action, TrainedModel: una ficha genérica con lo común
  (descripción, owner, `expone`, relaciones en dos listas —de qué sale, quién lo usa—,
  `acceso`) y lo particular por kind.
- Pestañas: **Overview** (descripción, `expone`), **Links** (`relaciones`), **Access**
  (`acceso`; el mock de ACP se retira), **History** (`version` del índice + `/arbol/historia`
  al abrirse; en un Dataset, los snapshots por `/datasets/{ns}/{n}` al abrirse).
- Lo que la consola escribe sigue igual (crear base, descripciones); crear **schema** es nuevo:
  un commit con `packages/<p>/<schema>/README.md` (la descripción; §7.3) por `PUT /arbol/{ruta}`.
- `tsc` limpio; `next build` no se corre (el `next dev` del usuario ocupa `.next`).

## 5 · `Function` y `Action` por `/documentos`

Dos filas más en `KINDS` (`functions/`, `actions/`), para que la ficha pida el texto y el
workspace lo abra por la misma puerta que los demás. Test en `los-documentos.sh`.

## 6 · Orden, criterio de hecho y lo que se mide

| paso | qué | hecho cuando |
|---|---|---|
| **0** | la forma, medida: los oráculos de demo y victor | **hecho** 2026-09-21: la tabla en §0; decisión 3 comprobada (schema = carpeta con `README.md`) |
| **1** | `ore_core::assets::indice` + `ore assets --json` | tests verdes; los oráculos cuadran; workspace verde |
| **2** | `GET /assets` con caché por cabeza | test de ore-serve; `plano-de-control` con el caso; CI verde |
| **3** | la medida viva | frío/caliente/bytes en 0034; `identidad`, `sale_de`, `vistaInducida` comprobados en victor |
| **4** | la consola sobre el índice | el catálogo se pinta de una llamada; `comoDatabase` fuera; fichas por kind; `tsc` limpio |
| **5** | `Function`/`Action` por `/documentos` | `los-documentos.sh` verde |
| **6** | 0034 → hecho («Lo construido»); este brief se borra | |

**Lo que se mide y se anota en 0034:** ítems/relaciones/bytes por árbol (0), ms de `indice()`
en local (1), ms frío y caliente de `GET /assets` (3) frente a las ~6 llamadas de hoy, y las
llamadas que hace la consola para pintar el catálogo (antes/después).

## 7 · Decisiones que hay que tomar antes de arrancar (di sí o no)

1. **`version` la pone ore-serve, no ore-core** (un `git log` local por fichero en el clon que
   ya tiene). Propuesta: **sí** — ore-core no sabe de git; y se mide en 3 lo que cuesta.
2. **La clasificación y los conductos (`acceso`) se calculan en ore-core** moviendo
   `etiquetas_de_raiz` de ore-cli a ore-core (hoy vive en `ore-cli/vista.rs`). Propuesta:
   **sí** — el índice no puede depender del CLI, y `ore view` lo seguirá usando de allí.
3. **Un schema es una carpeta con un `README.md`** (su descripción). Comprobado en 0: un
   `.yaml` sin `kind` es OOS1002 (el compilador valida todo `.yaml`); un `README.md` se ignora
   y lo movido a la carpeta compila igual. Sin cambio en OOS. **Decidido: sí.**
4. **`ore assets` como verbo del CLI** (lectura, `--json`): para la medida y para mirar el
   índice sin ore-serve. Propuesta: **sí**.
5. **La caché guarda las últimas N cabezas por rama (N = 4)** y nada persiste. Propuesta:
   **sí** — un push la invalida; un `?commit=` viejo se recalcula si no está.
6. **El paso 4 se hace después del 3, no en paralelo**: la consola pinta un índice medido.
   Propuesta: **sí** — es el método.
