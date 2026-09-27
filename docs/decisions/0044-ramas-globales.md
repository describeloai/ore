# 0044 · Ramas globales: una rama es del árbol entero, y el catálogo la enseña

**Estado:** decidido; fases 1 y 2 hechas (2026-09-27), los datos en ramas por decidir; apéndice A
(*scope proposals*) definido y medido, sin construir · **Decide:**
qué es una rama para quien usa ORE, qué se puede hacer en ella, y qué significa «en qué se
diferencia de `main`». Amplía [`0030`](0030-el-arbol-en-el-editor.md) W2 (ramas y propuestas en el
editor) al catálogo, y [`0036`](0036-la-clase-del-repositorio.md) ④ (la rama de un repositorio).

## El problema, como lo vio el cliente

El 2026-09-27, con la cabecera del editor diciendo `main`, un `CREATE VIEW` sobre
`standard_test.public.products` falló con *«no hay ningún schema `public` en la base
`standard_test`»*. El catálogo enseñaba `public` con sus 19 tablas; la sesión no. Medido:

- La sesión (el puesto) trabajaba en la rama personal `<persona>/puesto`, del 23-09 y **12 commits
  por detrás** de `main`, porque sin rama dicha `ore-serve` elegía ésa (0036 ④) y
  `asegurar_rama` la crea pero nunca la pone al día. El editor decía `main`; el puesto miraba
  otro árbol.
- Las ramas ya existían —del árbol entero, en la forja— pero **sólo el code workspace las veía**.
  El catálogo leía siempre `main`, y la mitad de las rutas que usa no miraban la cabecera
  `x-ore-rama`.

Dos pantallas del mismo producto, dos ideas de «dónde estoy». Eso es lo que esto cierra.

## Lo decidido

1. **Una rama es del árbol entero, no de un repositorio.** Una sola rama lleva las bases, los
   schemas, las vistas, los datasets, las entidades y el código de todos los repositorios. Es lo
   que Foundry llama *global branching*: no se ramifica un proyecto, se ramifica el mundo. Lo que
   0036 ④ llamaba «la rama del repositorio» (`<persona>/<repo>`) es un nombre, no otra cosa.
2. **La rama en la que se trabaja es una, y está en la URL** (`?rama=`). El selector del Assets
   Catalog Explorer y el del code workspace son el mismo control sobre las mismas ramas; los
   enlaces entre los dos la conservan; dentro del workspace el panel del catálogo no tiene
   selector propio: hereda el del repositorio. Recargar o compartir un enlace conserva dónde se
   está. La sesión (el puesto) se abre **en esa rama**, siempre dicha: la personal sólo si no hay
   otra, y un puesto abierto en otra rama no se reutiliza.
3. **Trabajar en una rama no toca `main`.** Lo que se crea, renombra o retira desde el catálogo, y
   lo que escribe un `CREATE VIEW` desde el workspace, va a la rama. A `main` se llega con una
   propuesta (pull request) que revisa otra persona (0030 W2). Fuera de la rama por defecto, el
   catálogo lo dice en una franja: *«Viewing branch `x` · based on `main`»*.
4. **En una rama se trabaja con definiciones; los datos, todavía no.** Todo lo que es árbol se
   hace en la rama: vistas, schemas, bases, modelos, código. Lo que **mueve datos** —dar de alta o
   retirar una conexión, copiar una tabla, ascender una base a estándar, rehacer la copia,
   contestar decisiones— escribe la cola de Jobs, y esos Jobs leen `main`: hecho desde una rama
   habría escrito en `main` sin decirlo. Se niega con `409` y el porqué, hasta que una rama tenga
   datos propios (lo que queda fuera, abajo). Una vista SQL no tiene datos: en una rama se lee
   sobre lo mismo que en `main`.
5. **«En qué se diferencia de `main`» se dice por activo y con su significado**, no por fichero
   (`GET /ramas/{r}/cambios`):
   - frente al **punto del que salió** la rama (`merge-base`), no frente al `main` de hoy: lo que
     `main` hizo después no es de la rama;
   - por activo (kind y nombre): nuevo, modificado, borrado o movido, comparando su forma
     canónica —reformatear no es un cambio—; de cada uno, las columnas que gana, pierde o cambian
     de tipo, las claves que cambian y el SQL antes y después;
   - **qué rompe**, de `ore diff`, atado a su activo; y **a quién alcanza**: quién lo lee, hasta
     el final, por el linaje;
   - la salud de la rama (sus diagnósticos) y su distancia: commits por delante, y cuánto ha
     avanzado `main` desde que salió.

   En la consola: una letra en cada hoja del árbol (N, M, D, R; un punto rojo si rompe), el
   resumen en la franja (*«3 changes · 1 breaking · main +2»*), el panel *Changes vs main* con
   *Open pull request*, y «vs main» en el detalle de un activo cambiado.

## Por qué así, frente al cliente

- **Una idea de «dónde estoy» en todo el producto.** El fallo de partida no fue de código: dos
  pantallas contestaban distinto a la misma pregunta. Con la rama en la URL y un solo control, la
  pregunta tiene una respuesta.
- **El valor no es el diff, es el impacto.** Git sabe qué ficheros cambian. ORE sabe qué
  significan: que quitar `pais` de `hr.publica` rompe a quien la consume (`OOS5001`), que
  `hr.ids` la lee, que el paquete exige subir de versión. El cliente lo ve **antes** de proponer,
  y quien revisa, antes de aprobar. Eso no se obtiene de ninguna herramienta de Git.
- **Negar es mejor que escribir en el sitio equivocado.** Una copia lanzada desde una rama que
  aterriza en `main` es exactamente el daño que una rama existe para evitar. Hasta que los datos
  tengan rama, se dice que no, y por qué.

## Lo que se descartó

- **Ramas por repositorio.** Una vista vive en una base, su código en un repositorio y su entidad
  en otra carpeta; ramificarlos por separado es no poder proponer el cambio entero.
- **La rama activa en una cookie o en el estado de cada pantalla.** Una cookie no se ve y no se
  comparte; el estado de cada pantalla es justo el fallo de partida.
- **Comparar contra el `main` de hoy.** Pone los cambios de `main` en la rama, al revés.

## Resultado

- **ORE**: `ore-serve` lee y escribe en la rama de `x-ore-rama` en las rutas del catálogo
  (`/fuentes`, `/paquetes`, su esquema, decisiones y copias, `/modelos`, `/funciones`; crear y
  renombrar schemas, retirar una base, modelar); lo que mueve datos, `409` fuera de la rama por
  defecto (`solo_en_la_de_por_defecto`). `GET /ramas/{r}/cambios` (`cambios.rs`), de memoria por
  las dos cabezas. `POST /puestos` no reutiliza un puesto de otra rama.
- **La consola**: el selector de ramas del catálogo (ramas reales, `?rama=`), la franja, las
  escrituras en la rama, Create › View en la rama, el catálogo del workspace en su rama; las
  marcas, el panel y «vs main».
- **Medido**: `la-propuesta.sh` 3c (el catálogo en la rama: esquema y `/paquetes` con lo de la
  rama y `main` sin ello; el schema nuevo en la rama; ascender, `409`) y 3d (una vista que pierde
  una columna, otra nueva que la lee, una borrada, y `main` que avanza: nuevo/modificado/borrado,
  `OOS5001` y `OOS5007` con su activo, el alcance, adelante 3 y atrás 1, de memoria la segunda
  vez).
- **Medido por el camino**: `ore diff` sólo da `OOS5001` por una columna quitada si la vista es
  SQL (v1alpha14); una estructurada la compara por el linaje. Una rama con vistas de antes enseña
  la columna quitada, pero no la marca como rota.

## Lo que queda fuera

- **Los datos en una rama.** La decisión grande, con su propio ADR. Lo que se propone, a la
  manera de Foundry: lo que la rama no ha tocado se lee de `main` **al día** (hoy los punteros
  viven en el árbol y una rama ve los de `main` congelados en el fork); un dataset o una vista
  materializada nuevos o cambiados se construyen **en la rama**, con sus bytes aparte (una rama de
  Iceberg, o una ruta por rama), y el catálogo dice *«not built on this branch»* hasta entonces;
  al fusionar, esos bytes pasan a ser los de `main`. Hasta entonces, el `409` del punto 4.
- **Crear una rama desde el catálogo** («New branch…», hoy un aviso) y **ponerla al día**: traer
  `main` a una rama que se quedó atrás (el `merge` de 0030 W2 existe en el workspace; el catálogo
  sólo lo avisa).
- **Lo borrado en el árbol.** Un activo borrado en la rama no está en su árbol: sale en el panel y
  en la cuenta, no tachado en su sitio.
- **El nombre de una rama personal** (`<sujeto>/<nombre>`) se enseña entero: hay que resolver el
  sujeto a un nombre, o enseñar sólo lo legible.
- **La vista detallada** (*Actions ▸ Detailed view*) todavía no sabe de ramas.

## Apéndice A · Scope proposals: se ramifica el mundo, se propone lo que es tuyo

**Estado:** visión de producto y, en A.2, su definición técnica medida (2026-09-27). Primer
entregable hecho en ORE: **el alcance por repositorio** (`POST /propuestas {alcance}`: la derivada
`alcance/<rama>/<carpeta>`, la huella de lo revisado, `main` + alcance validado, la rama al día al
fusionar; `la-propuesta.sh` 8e). Falta su pantalla en Code Repositories; después, el alcance por
activos desde el catálogo.

### El problema

Con el punto 1, una rama es del árbol entero; con 0030 W2, una propuesta es **la rama entera**
contra `main`. Juntas, obligan a algo que ningún cliente quiere: quien arregla una línea de un
transform en Code Repositories propone, sin saberlo, todo lo que esa rama lleve —las vistas que
alguien está rehaciendo en el catálogo, un schema a medio renombrar, la función de otro
repositorio—. Y al revés: quien quiere publicar un cambio de datos en el catálogo arrastra el
código a medias que viva en la misma rama.

El error sería resolverlo con **ramas por aplicación** (lo descartado arriba): volveríamos a no
poder proponer un cambio entero, y a tener dos ideas de «dónde estoy». La rama tiene que seguir
siendo una. Lo que tiene que dejar de ser entera es **la propuesta**.

### Cómo lo resolvió Foundry, y lo que tomamos

Foundry tiene dos clases de rama —la de una aplicación (un repositorio, un pipeline), que sólo
fusiona ese recurso, y la global, que cruza aplicaciones—. En la global **cada recurso se apunta a
la rama** cuando se toca y **se revisa por separado** (quien puede editarlo, o los aprobadores de
su política de protección); un rechazo bloquea la propuesta entera. Pero la propuesta global **se
fusiona entera**: para dejar un recurso fuera hay que **sacarlo de la rama** (vuelve a lo de
`main`), y su documentación avisa de que eso puede romper la rama —comprobar el linaje queda en
manos de quien lo hace—. Lo «parcial» que describe es un fallo a medias al fusionar, que no se
deshace.

Tomamos la revisión por recurso, y no las dos clases de rama. Y damos el paso que Foundry deja al
usuario: **la propuesta sabe qué lleva y ORE comprueba que se sostiene sola**. Una rama global, y
propuestas con alcance.

### Lo que es una scope proposal

1. **Una propuesta lleva un alcance: un conjunto de recursos de la rama**, no la rama. Lo que no
   está en el alcance no llega a `main` y sigue en la rama, intacto, para otra propuesta.
2. **Cada aplicación propone lo suyo por defecto.**
   - Desde **Code Repositories**: el alcance es lo que la rama cambia en *ese* repositorio. Una PR
     de código sigue siendo, para quien la abre, una PR de código.
   - Desde el **Assets Catalog**: el alcance son los activos cambiados —todos, una base, un
     schema, o los que se marquen en *Changes vs main*—. Es la propuesta que habla de datos: qué
     nace, qué cambia de forma, qué rompe y a quién.
   - El alcance por defecto se puede ampliar o recortar antes de proponer; lo que no se puede es
     no saber qué lleva.
3. **Un recurso es uno aunque tenga dos caras.** ORE es rico justo aquí: un `.sql` de un
   repositorio **es** una vista del catálogo; un `.ts` **es** una función que el catálogo
   promociona y otros reutilizan. No son dos cosas que haya que sincronizar, sino la fuente y el
   activo del mismo recurso. (Medido en A.2: **hoy ese vínculo no está escrito** en el árbol; es
   parte de lo que hay que construir.) Por eso:
   - el alcance se cuenta **en recursos, no en pantallas**: proponer el `.sql` desde el
     repositorio es proponer la vista, y la propuesta la enseña también como activo;
   - proponer la vista desde el catálogo lleva su `.sql`, y Code Repositories la enseña como
     fichero;
   - un recurso sólo puede estar en **una** propuesta abierta a la vez: si ya va en otra, se dice
     en cuál.
4. **El alcance tiene que sostenerse solo sobre `main`.** Lo que se fusiona es `main` más el
   alcance, y eso es lo que se valida (`ore validate`, `ore diff`), no la rama. Si el alcance lee
   algo que sólo existe en la rama —la vista nueva que el transform usa, la columna que la función
   espera—, ORE no lo deja fusionar a medias: dice qué falta y **propone añadirlo al alcance**.
   Las dependencias entre recursos, que en Foundry son responsabilidad de quien fusiona, aquí las
   calcula el linaje que ORE ya tiene.
5. **Se revisa por recurso y la aprueba quien debe.** Siguen las dos personas de 0030 W2; además,
   cada recurso del alcance pide la aprobación de sus dueños, y lo que el alcance **rompe** aguas
   abajo pide la de los dueños de lo roto. Un rechazo en un recurso bloquea la propuesta; quitar
   ese recurso del alcance la desbloquea.
6. **Una propuesta, dos lentes.** La misma propuesta `#n` se ve como ficheros y diff de líneas en
   Code Repositories, y como activos, columnas, SQL, impacto y (más adelante) filas en el
   catálogo. Revisar en una o en otra es revisar lo mismo; aprobar en una aprueba en las dos.
7. **Sólo la propuesta del catálogo promociona datos.** Una propuesta de código mueve
   definiciones y nunca bytes. Cuando las ramas tengan datos (el ADR pendiente), los datos que la
   rama construyó o ingirió llegan a `main` sólo por una propuesta cuyo alcance son esos activos,
   con su pestaña de datos y su elección —reconstruir en `main` o promocionar—.
8. **Después de fusionar, la rama sigue viva y al día.** Lo fusionado deja de aparecer como
   cambio de la rama; lo que quedó fuera sigue ahí, con sus marcas en el árbol, listo para la
   siguiente propuesta.

### Por qué así, frente al cliente

- **Cada cual propone lo que es suyo, sin perder el mundo.** El ingeniero de un repositorio hace
  una PR pequeña; el dueño de un dominio de datos publica un cambio de datos; y los dos trabajan
  en la misma rama cuando el cambio los cruza.
- **Lo que se revisa es lo que se fusiona.** Hoy se revisa la rama y se fusiona la rama; con
  alcance, se revisa el alcance y se valida `main` más el alcance. No hay nada en `main` que nadie
  haya visto.
- **La riqueza de ORE se convierte en seguridad.** Como un fichero y su activo son lo mismo, ORE
  sabe qué depende de qué, qué rompe y de quién es. Eso es lo que permite partir una rama sin
  romper `main`, y lo que ningún Git genérico puede hacer.

### Lo que se descarta

- **Ramas por aplicación**, además de las globales: dos ideas de «dónde estoy», y el cambio que
  cruza repositorio y catálogo otra vez imposible de proponer entero.
- **Proponer la rama entera desde cualquier pantalla** (lo de hoy): obliga a publicar lo que no es
  tuyo.
- **Un tipo de PR para código y otro para datos**: el mismo recurso tendría dos aprobaciones que
  pueden contradecirse.

### A.2 · Definición técnica (medida el 2026-09-27)

Medido en el código de ORE y de la consola, con `git` sobre un repositorio de prueba y con
`ore validate` sobre árboles sintéticos, y cotejado con lo que hace la industria (Foundry, GitHub
merge queue y CODEOWNERS, Nx *affected*, dbt *defer*, SQLMesh `--select-model`, Terraform
`-target`, Nessie, lakeFS).

#### Lo que hay hoy (medido)

| Pieza | Estado | Dónde |
|---|---|---|
| Identidad de un activo | `Kind:qname` (`doc_id`), **nunca la ruta**; un fichero YAML puede llevar varios documentos; un documento vive en un solo fichero | `normalize.rs:531`, `validate.rs:651` |
| Qué es documento | sólo `.yaml`/`.yml`, `.cedar`, `.oob`, `ontology.lock`; **`.sql`, `.py`, `.ts`, `.json` no** | `validate.rs:847` |
| Un repositorio | una carpeta bajo `packages/<p>/…` con un `README.md` que lo declara; cada fichero es del repositorio más hondo que lo contiene; **no tiene dueño** | `repositorios.rs`, 0036 |
| `.sql` → View | un `CREATE VIEW` en el puesto escribe un **documento aparte** (`views/<n>.yaml`, v1alpha14, con el SQL dentro); el `.sql` del repositorio y la vista **no se enlazan** | `puestos.rs:2578`, `__init__.py:899` |
| `.ts` → Function | **no existe**: sólo hay clase `functions-python`; `Function.spec.source` es texto libre que nada resuelve | `clases.rs:414`, 0036 |
| Dueños | `spec.owner` (`team:` o `user:`) en Package, View, Dataset, Schema y otros; obligatorio en Dataset y Package, y en View al escribirla por `/documentos`; **no** en Table; el índice lo expone | `pertenencia.rs:140`, `assets.rs:786` |
| Linaje | `lee_directo` y `respaldo`, puros sobre cualquier directorio; **no** siguen las aristas de Function, Action ni TrainedModel | `vistas.rs:348`, `cambios.rs:111` |
| Propuesta | PR de la forja de **la rama entera** a `main`; dos personas; se fusiona si compila (`ore validate` de la rama) y sin conflicto; **la forja borra la rama al fusionar** (`delete_branch_after_merge`) | `propuestas.rs:645`, `forja.rs:239` |
| Alcance de hoy | `GET /propuestas` con `X-Ore-Raiz`: **filtra la lista** por prefijo de fichero; no cambia lo que se fusiona; la consola no lo usa | `propuestas.rs:435` |
| Punteros de datos | `datasets/<p>/<schema>/<n>.json`: fuera de los documentos, **`cambios` no los ve** | `punteros.rs:28` |

#### Lo que se probó

1. **Construir un alcance con git funciona.** `main` + `git diff -M <merge-base> <rama> -- <rutas>`
   aplicado con `git apply --3way`: si `main` tocó otras líneas del mismo fichero, entra limpio y
   conserva lo de `main`; si tocó las mismas, conflicto con el fichero nombrado. Copiar los
   ficheros de la rama (`git checkout <rama> -- <rutas>`) **no** vale: pisa lo que `main` cambió
   después.
2. **Un renombrado no se parte.** Con sólo el destino en el alcance, `main` acaba con el
   documento dos veces (`OOS2035`, identidad repetida). El alcance es de recursos, y un recurso
   movido lleva su ruta de antes y la de después.
3. **Las dependencias las dice el compilador.** `main` + una vista que lee otra que sólo existe
   en la rama: `error[OOS2018]: hr.ids lee hr.publica, que no existe`. Con `hr.publica` en el
   alcance, `ok`. El diagnóstico nombra **qué falta**, y eso es la sugerencia de «añádelo».
4. **La rama se pone al día sola.** Tras fusionar la derivada en `main` y traer `main` a la rama
   (`traer`, lo que ya hace el *Merge* del workspace), `cambios` deja de contar lo fusionado; lo
   que la rama siga cambiando después vuelve a contar. Sin traer `main`, lo fusionado **sigue
   saliendo** como pendiente.

#### Lo que dice la industria, y lo que tomamos

- **Validar base + alcance, nunca la rama** (GitHub merge queue, Nx *affected*): se valida lo que
  de verdad va a ser `main`.
- **Cierre de dependencias**, hacia arriba obligatorio y hacia abajo validado (SQLMesh incluye
  siempre lo que cambia aguas abajo; Terraform `-target` no, y HashiCorp lo desaconseja por eso).
  Foundry lo deja al usuario y avisa de que se puede romper la rama: ése es el hueco que
  cerramos.
- **Una propuesta abierta por recurso**, **regenerar la derivada, no editarla** (una rama copiada
  se queda atrás), **reconciliar la rama por contenido, no por commit** (el squash y la copia por
  rutas engañan al `rebase`) y **fusión atómica** (Foundry puede quedar fusionado a medias y no
  lo deshace; un solo `merge` de git no puede).
- **Aprobación por dueño de ruta o recurso**, a la manera de CODEOWNERS, con la regla también
  protegida.

#### La definición

1. **Unidad de alcance.** Dos clases:
   - un **documento**, por su `doc_id`: su fichero, y si se movió, la ruta de antes y la de
     después; si su fichero lleva otros documentos, van juntos (se dice);
   - un **fichero de repositorio** que no es documento (`.sql`, `.py`, `.ts`…), por su ruta.

   Los grupos —un repositorio, una base, un schema, «todo»— se expanden a unidades al proponer.
   Proponer «todo» es la propuesta de hoy.

   **El alcance de un repositorio no lleva activos** (hecho): lo que la rama cambia bajo su
   carpeta **menos los documentos del catálogo** —todo `.yaml` (uno sin `kind` no compila,
   `OOS1002`: no hay YAML «de repositorio»), `.oob`, `.cedar`, `ontology.lock`—, que se quedan
   en la rama y la respuesta nombra (`documentosFuera`). Y **un movimiento que cruza el borde**
   de la carpeta no se parte: `409` (llevar sólo el borrado dejaría a `main` sin el fichero).
2. **La propuesta guarda su alcance** en su cuerpo en la forja (como hoy `sub:`), en una línea
   `alcance:` con las unidades. Una unidad sólo está en **una** propuesta abierta: `proponer` da
   `409` y dice cuál la lleva.
3. **La derivada.** `propuestas/<n>` = `main` + el diff de la rama desde su `merge-base`, limitado
   a las rutas del alcance (`-M`, `apply --3way`). La PR de la forja sale de la derivada, no de la
   rama. Se **regenera** al abrir, cuando la rama o `main` avanzan (se detecta por las dos
   cabezas, como la memoria de `cambios`) y **siempre justo antes de fusionar**. Si no aplica
   limpio: `409` con los ficheros en conflicto, y la salida es traer `main` a la rama.
4. **Las comprobaciones, sobre la derivada:**
   - `ore validate`: si falla con una referencia a algo que la rama tiene y `main` no
     (`OOS2018` y parientes), la respuesta dice **qué unidad añadir**;
   - `ore diff main derivada`: lo que rompe, con su activo, y a quién alcanza **en `main`**
     —incluidos consumidores fuera del alcance—;
   - las dos personas de 0030 W2.
5. **Dos lentes, sin nada nuevo en ORE.** La derivada **es una rama**: *Files changed* sale de la
   forja sobre su PR (justo el alcance) y *Assets changed* es `GET /ramas/propuestas/<n>/cambios`,
   que ya existe.
6. **Fusionar** es un `merge` de la derivada en `main`: atómico. La forja borra **la derivada**
   (`delete_branch_after_merge` pasa a ser justo lo correcto), y ORE **trae `main` a la rama
   global** con `traer`. Si eso choca, la rama queda como estaba y se avisa: la fusión ya ocurrió
   y es buena.
7. **Datos.** Una propuesta nunca mueve bytes (hoy nada en una rama los tiene: el `409` de la
   decisión 4). Cuando los tenga, los punteros de `datasets/…/*.json` tendrán que entrar en
   `cambios` y en el alcance de la propuesta del catálogo, y sólo en ella.

#### Lo que falta para cumplir A.1, y es trabajo aparte

- **El vínculo fuente ↔ activo.** Hoy el `.sql` y su vista son dos ficheros sin relación, y la
  función en TypeScript no existe. Hasta escribir el vínculo (el documento que nombra su fuente,
  o la fuente que nombra su documento: se decide en la spec), un `.sql` y su vista son **dos
  unidades**, y el compilador los une sólo cuando uno lee al otro. Proponer uno sin el otro es
  posible y no rompe nada.
- **Aprobación de los dueños.** `spec.owner` dice el equipo; falta **quién es** el equipo
  (pertenencia en `ore-iam`, sin medir), un dueño para los repositorios y para las tablas, y el
  linaje de Function, Action y TrainedModel para saber a quién alcanza un cambio fuera de las
  vistas.
- **La consola.** Hoy lista todas las propuestas (no usa `X-Ore-Raiz`) y propone la rama entera.

#### Descartado en la medida

- **Copiar los ficheros de la rama sobre `main`** para construir el alcance: pisa lo que `main`
  cambió después.
- **Squash al fusionar**: rompe la reconciliación de la rama; se fusiona con `merge`.
- **El alcance por carpeta** (el `X-Ore-Raiz` de hoy) como alcance de lo que se fusiona: parte
  documentos que comparten fichero y no sabe de renombrados.
