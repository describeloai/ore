# El proyecto — el brief desechable de 0035

> Se borra cuando el paso ⑤ cierre; lo que quede se dice en
> [`0035`](decisions/0035-el-proyecto.md) («Lo construido»). Aquí sólo lo que hace falta para
> construir la resolución ⑤ en orden: qué toca cada paso, dónde, con qué prueba, y qué número
> de la medida cambia.

**La medida que manda**: `pruebas-de-fuego/medida-proyecto.py` (2026-09-23). Cada paso cierra
cuando su fila cambia como dice esta tabla, y no antes.

**Lo que no se toca** (⑤ 5): el espacio de nombres (`<paquete>.<nombre>` es del árbol), el
gobierno (una política estrecha protege a todos), la unidad de compilación (el árbol; el
diagnóstico se **atribuye**, no se parte) y el alcance de lectura (manda el conducto, no la
pertenencia). Y **nada en OOS**: el proyecto no es un documento de la ontología.

**La forma, para no repetirla en cada paso** — `proyectos/<nombre>/README.md`:

```markdown
---
nombre: Customer Churn
descripcion: Predicción de abandono sobre los pedidos y la plantilla.
contiene: [ventas/churn, rrhh/nomina]
---
Lo que este proyecto hace, en prosa.
```

`<nombre>` de la carpeta: el identificador (minúsculas, `[a-z0-9-]`). `contiene`: `<paquete>` o
`<paquete>/<carpeta>`, que es lo que el índice ya sabe de cada ítem (0034 ④).

---

## ⓪ La forma, medida ✓

**Dónde**: `pruebas-de-fuego/medida-proyecto.py` (§8 nuevo), sobre los árboles de demo y victor.

- Los oráculos: cuántos ítems caerían en un proyecto hoy (demo 17, victor 58 — **todos fuera**,
  §6), cuántas carpetas de paquete hay en cada árbol, y qué nombres tendrían los proyectos si se
  hicieran de las carpetas que ya existen.
- Que un `proyectos/<n>/README.md` **compila y es invisible** al compilador en un árbol de
  verdad (no sólo en el de la medida), y que `ore validate` no lo nombra.
- Que el encabezado se analiza con lo que ore-core ya tiene (`parse.rs`) y **qué pasa si está
  mal**: sin `nombre`, con `contiene` que no resuelve, con dos proyectos que nombran la misma
  carpeta (se solapan: el índice lo dice, no lo impide).
- **Prueba**: la medida corre y sus números entran en 0035 («Lo construido», ⓪).
- **Hecho**: §8 de `medida-proyecto.py`, sobre demo (`b93ed52`) y victor (`a6e2b0e`).
  Los oráculos de ①: 17 y 58 ítems, **0 carpetas de cliente**, **todos fuera**, y
  `contiene: [olist]` resolvería 8 de 17 (`[foreign_test]`, 19 de 58). El manifiesto es
  invisible (`validate` 0, no lo nombra, el índice igual); roto —sin `nombre`, sin cerrar,
  sin encabezado— **no rompe el árbol**; `contiene` que no resuelve resuelve 0 y no es
  error; y dos proyectos que nombran la misma carpeta se solapan sin que el árbol se entere.

## ① La unidad: el proyecto en el índice ✓

**Dónde**: `crates/ore-core/src/proyectos.rs` (nuevo) + `assets.rs`; `crates/ore-cli/src/activos.rs`.

- `ore_core::proyectos::leer(pkg_raiz)` → los manifiestos de `proyectos/*/README.md`:
  `{nombre, descripcion, contiene, ruta}`. Sin analizador nuevo: el encabezado con `parse.rs`,
  la prosa se ignora. Un manifiesto roto **no rompe el árbol**: se dice en el índice
  (`roto: <por qué>`) y el proyecto se lista igual — el compilador no lo ve, y el catálogo no
  puede mentir.
- El índice (0034) gana **dos cosas**: `proyectos: [{nombre, descripcion, contiene, items,
  version}]` en la raíz, y `proyectos: [...]` **en plural** en cada ítem (un ítem puede estar en
  varios: ⑤ 4). Resolver `contiene` es comparar con `paquete` y `carpeta`, que el ítem ya trae.
- `ore assets --json` los enseña; `ore assets` los resume («3 proyectos · 17 ítems · 4 fuera»).
- **Prueba**: `crates/ore-core/tests/assets.rs` gana dos proyectos —uno que nombra un paquete
  entero, otro una carpeta—, uno que se solapa con el anterior, uno vacío y uno roto.
  **Medida** §6: «todo el árbol sería un proyecto» → los ítems se reparten.
- **Hecho**: `proyectos.rs` (`leer`, `Proyecto::alcanza`, 5 pruebas), el índice con `proyectos`
  en la raíz y en cada ítem, `version` del manifiesto en ore-serve y el resumen de `ore assets`.
  Sobre demo: «2 proyectos · 17 ítems · 9 fuera», `churn` con **8** — el oráculo de ⓪.

## ② Servirlo y escribirlo ✓

**Dónde**: `crates/ore-serve/src/proyectos.rs` (nuevo) + `rutas.rs`.

- `GET /assets` ya los trae: **una llamada, la que ya se hace** (0034 ⑤). No hay ruta nueva de
  lectura.
- El verbo de producto, que sí es nuevo porque valida lo que `/arbol` no sabe:
  `POST /proyectos {nombre, descripcion, contiene?}` (201, o 409 si el nombre ya está),
  `PUT /proyectos/{n}` (el manifiesto entero) y `DELETE /proyectos/{n}` (**el manifiesto, no lo
  que nombra**: borrar un proyecto no borra assets — y la respuesta lo dice).
  Escribe por la forja como todo lo demás: commit del sujeto, en su rama.
- La puerta del puesto (W3.7 gobierno ①) **no cambia**: `/proyectos` es escritura, y un agente
  no escribe fuera de los verbos. Un proyecto lo crea una persona.
- **Prueba**: `pruebas-de-fuego/los-documentos.sh` gana un caso: crear, listar por `/assets`,
  renombrar la descripción, un nombre repetido 409, borrar y que **lo que nombraba siga en el
  árbol**.
- **Hecho**: `ore-serve/src/proyectos.rs` (el `id` del título, el manifiesto escrito por el
  servidor con los escalares entre comillas, `sinResolver`, `siguenEnElArbol`) y el caso 20 de
  `los-documentos.sh` — incluido que desde un puesto `POST /proyectos` es **403**.

## ③ La consola deja de ser un mock

**La medida que manda**: `pruebas-de-fuego/medida-la-consola-de-proyectos.py` (2026-09-22), en
0035 («Lo medido para ③»). **Tres cosas, y sólo tres**: el listado real, crear y borrar un
proyecto, y crear y borrar carpetas dentro. Los artefactos de code workspace con persistencia
real son **la iteración siguiente**, y este paso no los toca.

**Lo que la medida obliga a respetar**:

- El listado **no necesita llamada nueva**: `GET /assets` ya da 5 de los 6 campos. El sexto,
  `collaborators`, **no existe**: se va de la tabla (§1).
- No hay papelera: **borrar es un commit**, y lo que el proyecto nombraba **no se borra** (§2).
- Una carpeta **es un fichero dentro** —`README.md`, que el editor ve y el compilador ignora—,
  y `DELETE /arbol/<carpeta>` **hoy es 404**: sólo hay verbo de fichero (§3).
- **`ProjectDetailView.tsx` y `CreateResourceModal.tsx` tienen WIP de otra sesión**: ③b no los
  edita — lo suyo entra por un módulo nuevo que el detalle llamará en una línea cuando aquello
  aterrice (§4).

### ③a · El listado, crear y borrar (ficheros libres) ✓

**Dónde**: `lib/server/query.ts`, `lib/projects/proyectos.ts` (nuevo, sustituye a `mock.ts`),
`components/projects/ProjectsHome.tsx`, `ProjectCreateModal.tsx`, `ProjectContextMenu.tsx`,
`app/(workspace)/clusters/[celda]/projects/page.tsx`.

- `query.ts` gana tres filas: `POST /proyectos`, `PUT /proyectos/{id}`, `DELETE /proyectos/{id}`
  (la figura ya existe: 26 llamadas de escritura declaradas).
- `ProjectsHome` lee **del índice** (`assets`, la llamada del catálogo): `id`←`nombre`,
  `name`←`titulo`, `description`, `updatedAt`←`version.cuando`, y **cuántos ítems** nombra.
  La columna **Collaborators se va** y en su sitio va **Items**; la ficha dice que quién lo ve lo
  decide ore-iam.
- Crear: el modal gana **`contiene`** (los paquetes y carpetas del índice, a elegir) → `POST`;
  el `id` lo da el servidor. Renombrar → `PUT`. «Move to trash» pasa a **«Delete project»**, con
  el aviso de lo que **sigue en el árbol**.
- Un proyecto **roto** se lista con su porqué (el índice ya lo trae) en vez de desaparecer.
- **Prueba**: `tsc --noEmit`, y a mano contra un `ore-serve` local con acme-retail: crear →
  aparece → renombrar → borrar → `hr` sigue. **Medida** §1/§2: «3 filas en memoria» → las del
  árbol; «4 acciones sin handler» → 1 (Copy link).

### ③b · Las carpetas dentro de un proyecto ✓ (salvo el cableado en el detalle)

**Dónde**: ORE (`crates/ore-serve/src/arbol.rs`) y la consola
(`lib/projects/carpetas.ts`, nuevo).

- **ORE primero**: `DELETE /arbol/<ruta>` aprende **directorios** — borra lo que cuelga en **un
  commit**, y la respuesta dice **qué ficheros se llevó**. Hoy es 404 (§3), y la alternativa es
  N llamadas desde la consola, que no es una operación: es una racha.
- `lib/projects/carpetas.ts`: `crearCarpeta(paquete, ruta, nombre)` → `PUT /arbol/<…>/README.md`
  (una carpeta es un fichero dentro), `borrarCarpeta(ruta)` → `DELETE /arbol/<…>`, y
  `carpetasDe(proyecto)` a partir del índice. **Ninguna toca los ficheros con WIP ajeno.**
- Cuando el proyecto nombra **más de un paquete**, crear una carpeta **pregunta en cuál**; con
  uno solo, no pregunta (§3).
- **Prueba**: un caso más en `los-documentos.sh` (crear la carpeta, que `/arbol` la vea, que el
  índice la nombre en cuanto cae un documento, borrarla entera en un commit). **Medida** §3:
  «`DELETE /arbol/<carpeta>` 404» → 200 con lo que se llevó.
- **Y el cableado en el detalle** (`ProjectDetailView.tsx`) espera a que la otra sesión suelte
  el fichero: es **una línea por acción** contra `carpetas.ts`.

## ④ El repositorio: la instancia, y la sesión acotada a ella → [`docs/repositorio.md`](repositorio.md)

> Reescrito tras **0035 ⑥** (la sesión no es «por proyecto» sino **por repositorio**) y
> **[0036](decisions/0036-la-clase-del-repositorio.md)** (cada clase es un producto distinto:
> entorno, capacidades e interfaz). Creció lo bastante como para tener brief propio: los siete
> pasos están en **[`docs/repositorio.md`](repositorio.md)**.
>
> Lo que este brief conserva de ④: el **diagnóstico atribuido** (⑤ 5, «compilar») — con
> `proyectos` y `repositorios` en cada ítem, la consola dice «tu repositorio compila; el árbol
> no, por X», sin partir la compilación y sin tocar `ore validate`.

## ⑤ Medido de nuevo, y 0035

`medida-proyecto.py` entera; los números en 0035 («Lo construido»); la fila del índice de
decisiones a **hecho**; este brief se borra.
