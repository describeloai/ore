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

## ② Servirlo y escribirlo

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

## ③ La consola deja de ser un mock

**Dónde**: rubix-platform — `lib/projects/` (fuera `mock.ts`), `components/projects/*`,
`lib/server/query.ts`, `components/projects/detail/*`.

- `ProjectsHome` lee los proyectos **del índice** (`indiceDeAssets()`, ya se llama en el
  catálogo): nombre, descripción, cuántos ítems, cuándo se tocó (`version`). Crear →
  `POST /proyectos`; el `id` es el nombre de la carpeta.
- El detalle lista **lo que el proyecto nombra**, agrupado por paquete y carpeta, con los iconos
  por kind que 0034 ④ ya puso.
- «Create ▸ Code Repository» deja de ser una tarjeta muerta: crea la carpeta en el paquete que
  se elija, la añade a `contiene` y abre el workspace **acotado a ella**; la plantilla **siembra
  ficheros de verdad** (los 7 del `BuildPicker`, cada una con su semilla mínima que compile).
- `collaborators` **no se inventa**: la ficha lo dice («quién puede ver esto lo decide ore-iam»)
  y el campo se va del modelo hasta que exista.
- **Prueba**: `tsc --noEmit` y a mano contra `demo` con un proyecto de verdad. **Medida** §1:
  «crear añade a una lista en memoria» → crea un commit; «7 plantillas que no escriben nada» →
  escriben.

## ④ La sesión y el diagnóstico, por proyecto

**Dónde**: `crates/ore-serve/src/puestos.rs`, `rutas.rs`; la consola (`lib/server/puestos.ts`,
`components/code-workspace/*`).

- `id_de(persona, entorno, proyecto)` y `x-ore-proyecto` (o `{proyecto}` en el cuerpo de
  `POST /puestos`): una sesión por persona, lenguaje **y proyecto**. Sin proyecto, como hoy.
  La rama por defecto pasa a `<persona>/<proyecto>` cuando lo hay (W3.7 ④ puso `<persona>/puesto`).
- El diagnóstico atribuido (⑤ 5, «compilar»): `GET /assets` ya sabe qué ficheros no compilan;
  con `proyectos` en cada ítem, la consola dice **«tu proyecto compila; el árbol no, por X»**.
  Sin partir la compilación y sin tocar `ore validate`.
- **Prueba**: `el-puesto.sh` gana un caso (dos proyectos, dos puestos de la misma persona, ids
  distintos y ramas distintas). **Medida** §7: «el MISMO puesto» → dos.

## ⑤ Medido de nuevo, y 0035

`medida-proyecto.py` entera; los números en 0035 («Lo construido»); la fila del índice de
decisiones a **hecho**; este brief se borra.
