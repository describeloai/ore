# El repositorio — el brief desechable de 0035 ⑥ y 0036

> Se borra cuando el paso ⑦ cierre; lo que quede se dice en
> [`0035`](decisions/0035-el-proyecto.md) y [`0036`](decisions/0036-la-clase-del-repositorio.md)
> («Lo construido»). Aquí sólo lo necesario para construirlo en orden: qué toca cada paso,
> dónde, con qué prueba, y **qué número de la medida cambia**.

**La medida que manda**: `pruebas-de-fuego/medida-el-repositorio.py` (2026-09-22). Cada paso
cierra cuando su fila cambia como dice esta tabla, y no antes.

**La regla que gobierna todo el brief** (0036): **el sitio es la clave de partición**. Todo lo
acotado toma **la ruta de la carpeta** como parámetro y se expresa como «esto, para este
prefijo». Si algo acotado no se puede decir así, está mal puesto.

**Lo que no se toca**: el árbol, el índice y el linaje siguen siendo **uno** (0035 ⑤); compilar
sigue siendo de la celda (atribuido, no partido); leer sigue mandándolo el conducto; y una clase
**ajusta hacia abajo, nunca concede**.

**La forma, para no repetirla** — `packages/<paquete>/<carpeta>/README.md`:

```markdown
---
nombre: New Pipelines Java Transform
plantilla: transforms
plantillaVersion: 1
---
Lo que este repositorio hace, en prosa.
```

Medido en 0035 ⑥: con él dentro, `validate` sale 0 y no lo nombra (demo 122 ms, victor 287 ms),
el índice da los mismos ítems, y recorrer el árbol buscándolos cuesta 25–36 ms.

---

## ① La instancia: el repositorio en el índice

**Dónde**: `crates/ore-core/src/repositorios.rs` (nuevo) + `assets.rs`; `crates/ore-cli/src/activos.rs`.

- `ore_core::repositorios::leer(raiz)` → los `packages/<pkg>/**/README.md` **con `plantilla:`**
  (sin `plantilla` es una carpeta con README, no un repositorio). Reutiliza el encabezado de
  `proyectos.rs` —el mismo `parse.rs`, la misma regla: **uno roto se lista con su porqué**—; se
  factoriza lo común en vez de copiarlo.
- El índice gana `repositorios: [{nombre, plantilla, plantillaVersion, ruta, paquete, carpeta,
  items, proyectos[], version, roto?}]`, y cada ítem gana **`repositorio`** —**singular**, al
  revés que `proyectos`: un ítem está en **un** repositorio (el más hondo que lo contiene) o en
  ninguno. Un proyecto es una lente y se solapa; un repositorio es **el sitio donde vives**, y
  anidarlos es una hondura, no un solape.
- `ore assets` cierra el resumen con los repositorios, como ya hace con los proyectos.
- **Prueba**: `crates/ore-core/tests/assets.rs` gana un caso —dos repositorios en el mismo
  paquete, uno anidado dentro de otro, uno roto y uno sin `plantilla`— y fija que **los ítems no
  cambian**. **Medida** §3: «0 carpetas de cliente» → las que se creen; §1 sigue en `validate 0`.

## ② Los verbos: crear y reescribir la instancia

**Dónde**: `crates/ore-serve/src/repositorios.rs` (nuevo) + `rutas.rs`.

- `POST /repositorios {paquete, carpeta, nombre, plantilla, proyecto?}` → **201** con la ruta, o
  **409** si esa carpeta ya es un repositorio. Escribe **el manifiesto y la semilla de la clase
  en UN commit** (por dentro, lo mismo que `POST /arbol/commit`): una plantilla que deja los
  ficheros a medias no es una plantilla.
- Con `proyecto`, añade además la carpeta a su `contiene` — en **el mismo commit**, porque
  «creado pero no nombrado» es un estado que nadie pidió.
- `PUT /repositorios/{ruta}`: `nombre`, `plantilla` y `plantillaVersion`. Borrar **no tiene verbo
  nuevo**: es el `DELETE /arbol/<carpeta>` de 0035 ③b, que ya se lleva la carpeta entera en un
  commit y dice qué ficheros.
- Leerlos **no tiene ruta**: `GET /assets` ya los trae (①). Como con los proyectos.
- La puerta del agente **no cambia**: `/repositorios` no está en la lista de permitidos, así que
  desde un puesto es **403**. Un repositorio lo crea una persona.
- **Prueba**: `los-documentos.sh` gana un caso: crear (201 con su semilla), repetir (409), verlo
  en `/assets`, renombrar, borrar su carpeta y que **el manifiesto se vaya con ella**.

## ③ La capa por repositorio — el cambio con más valor por línea

**Dónde**: `crates/ore-serve/src/entorno.rs`, `puestos.rs`.

- Hoy `entorno.rs` **une** `pyproject.toml` de la raíz y de **cada** paquete: una sola capa para
  toda la celda. Pasa a resolverse **por alcance**: `raíz + paquete + repositorio`, con su propio
  digest. La raíz y el paquete siguen siendo comunes **a propósito** (lo de todos, para todos).
- `GET /entorno` y `POST /entorno` aceptan el alcance **por cabecera** (`X-Ore-Raiz`), no por
  query: **ningún dato entra por la URL** (ore-entrada la descarta). `POST /puestos {repositorio}`
  usa **su** capa.
- El informe deja de ser sólo `entorno/python.json`: uno por alcance
  (`entorno/<digest>.json`, con qué alcance lo pidió).
- **Prueba**: un caso en `el-puesto.sh` —un repo declara una dependencia que el otro no; dos
  capas, dos digests, y la sesión del segundo **no la baja**—. **Medida** nueva en §6 (la que
  este paso añade): «una capa para la celda» → una por repositorio.

## ④ Lo acotado: el editor, la sesión, la rama y las propuestas

**Dónde**: `crates/ore-serve/src/{arbol,puestos,propuestas}.rs`.

Son **tres cosas y ninguna cara** (§4 las midió):

- **El editor**: `GET /arbol` acepta `X-Ore-Raiz: packages/<pkg>/<carpeta>` y devuelve **sólo lo
  suyo**, con la cabeza del árbol igual. Sin cabecera, como hoy.
- **La sesión**: `id_de(persona, entorno, repositorio)` y la rama por defecto
  `<persona>/<repo>`. Sin repositorio, como hoy (`<persona>/puesto`).
- **Las propuestas**: `GET /propuestas` filtra por prefijo de ruta (`X-Ore-Raiz`): la pestaña
  *Pull requests* de un repositorio son **las que tocan sus ficheros**.
- **Prueba**: `el-puesto.sh` (dos repos, dos puestos de la misma persona, ids y ramas distintas)
  y `los-documentos.sh` (el árbol acotado, y las propuestas de un prefijo). **Medida** §4: «24
  ficheros de la celda / el MISMO puesto / `ana/puesto` / no filtra» → «los suyos / dos /
  `ana/<repo>` / filtra».

## ⑤ La clase: la tabla del producto, con techo y versión

**Dónde**: `crates/ore-core/src/clases.rs` (nuevo, la tabla) + `puestos.rs` (el techo) + el índice.

- Una entrada por clase: `{id, version, lenguajes, escribe, perfil?, semilla}` para las cinco
  (`transforms`, `analytics`, `models`, `functions`, `semantics`). **La tabla es del producto**,
  no del árbol: el manifiesto sólo guarda la clave y la versión.
- **El techo**: la clase **ajusta hacia abajo** donde ya se aplica el gobierno —la puerta del
  agente y la declaración del transform—. Un `analytics` **no escribe** aunque lo declare; un
  `semantics` **no ejecuta**. Nunca al revés: una clase no concede.
- **La versión**: el índice dice `actualizable: true` cuando `plantillaVersion` < la del
  producto. Subirla es **una propuesta** con su diff, no un commit a la brava — que es lo que la
  columna «UPGRADE · Up to date» significa de verdad (0036, cotejado con Foundry).
- **Prueba**: `el-puesto.sh`: desde un repo `analytics`, escribir es **403** aunque el transform
  declare `output`; desde el `transforms` de al lado, **201**.

## ⑥ La consola: muchos sitios donde trabajar, en vez de uno

**Dónde**: rubix-platform — `lib/server/repositorios.ts` (nuevo), `components/code-workspace/BuildPicker.tsx`,
la lista nueva de *Code repositories*, `components/projects/detail/ProjectDetailView.tsx` y
`CreateResourceModal.tsx` (**ya sí**: el WIP ajeno deja de ser motivo para no tocarlos).

- **La lista** (la de la captura): nombre, ruta, icono por clase, *last edited by* / *last
  edited* —del árbol, §2— y la pestaña *Pull requests* con las suyas. Del índice: **una llamada**.
- **«Save» del BuildPicker** deja de navegar en falso: crea la instancia (`POST /repositorios`)
  y abre `/workspaces/<repo>`, que carga el árbol **acotado** (④).
- **El detalle del proyecto** enseña sus repositorios como lo que son —carpetas con clase— y sus
  carpetas con `crearCarpetaEnElArbol` / `borrarCarpetaDelArbol` (ya escritas en 0035 ③b): lo
  que queda es **una línea por acción**. Se van `pinned`, `tags`, `sharedBy` y `Trash` —no tienen
  dónde vivir—, y la portada pasa a ser el **manifiesto** del proyecto.
- **Prueba**: `tsc --noEmit` y a mano contra un `ore-serve` local. **Medida** §5: «dos columnas
  no existen» → existen; «Save navega» → crea; «la ruta del workspace es una sola» → una por repo.

## ⑦ Medido de nuevo, y las dos ADR

`medida-el-repositorio.py` entera (con la sección nueva de la capa); los números en 0035 ⑥ y en
0036 («Lo construido»); las filas del índice de decisiones a **hecho**; este brief se borra.
