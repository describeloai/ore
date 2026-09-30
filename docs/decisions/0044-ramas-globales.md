# 0044 · Ramas globales: una rama es del árbol entero, y el catálogo la enseña

**Estado:** decidido; fases 1 y 2 hechas (2026-09-27); los datos en ramas, decididos y medidos en el
apéndice C (2026-09-29), sus pasos por hacer; apéndice A
(*scope proposals*) hecho por repositorio y por activos, con lo que arrastra (2026-09-28);
apéndice B (*la rama protegida*) hecho, P2 pendiente · **Decide:**
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

- **Los datos en una rama.** Decidido en el **apéndice C**, que sustituye a lo que sigue. Lo que se proponía, a la
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
fusionar; `la-propuesta.sh` 8e) y su pantalla en Code Repositories. Segundo, en ORE: **el alcance
por activos** (`POST /propuestas {activos: [id…]}`: sus ficheros y, movidos, los de antes; lo que
comparte fichero y el `package.yaml` de su base van con ellos al proponer; `faltan` dice lo que la
rama cambia y a `main` + alcance le falta para compilar; `la-propuesta.sh` 8f). Tercero
(2026-09-28): **lo que arrastra** un alcance de activos, la propuesta **en seco** y su pantalla en
el catálogo —la sección *Pull requests*, «Changes vs main» con el árbol del sidebar y una casilla
por nodo, *From branch*, los orígenes con sus punteros, y lo arrastrado marcado *Required*
(§ A.3)—.

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

### A.3 · Lo que arrastra un alcance de activos (medido y hecho el 2026-09-28)

**Lo medido** (`pruebas-de-fuego/lo-que-arrastra.sh`, antes de construir): una base *standard*
creada **en una rama** desde un origen escribe **nueve** activos —sus dos `Dataset`, su `Package` y
su `Schema`; los dos punteros (`Table`) en el paquete de la fuente (0045), con el `Schema` y el
`Package` de esta; y la `ConduitPolicy` de la copia—. Proponer sólo los datasets llevaba los
datasets y el paquete de la base, **no compilaba** (`OOS2037` sin schema, `OOS2018` sin puntero,
`OOS4011` sin conducto) y `faltan` salía vacío (el diagnóstico nombra `from.table: x` y un schema
suelto, y no casaba). Sólo con las nueve compila. Proponiendo **uno** de los dos datasets, el
`package.yaml` de la fuente exporta los dos punteros: `OOS2027` sin el otro.

**Lo decidido:**

1. **La expansión arrastra lo imprescindible**, hasta que no se añade nada, y **sólo lo que la
   rama cambia** (lo que ya está en `main` no hace falta llevarlo): lo que comparte fichero; el
   `Package` de cada paquete tocado; el `Schema` de cada activo; **lo que cada activo lee** —el
   linaje hacia abajo: `cambios` da `lee` por activo, y el de un `Package` es lo que **exporta**—;
   y, con una copia (un `Dataset`), la `ConduitPolicy` cambiada (el conducto
   `materialization.payload` es de todo el árbol). Cada añadido lleva su porqué
   (`anadidosPorque`: «lo lee `x`», «el schema de `x`», «lo exporta el paquete `x`»…).
2. **`faltan` es la red**, no el camino: casa `campo: nombre` y un schema nombrado suelto, por si
   algo se escapa a la expansión.
3. **En seco.** `POST /propuestas {…, seco}` dice lo que la propuesta llevaría sin abrir PR ni
   dejar derivada: `"alcance"`, sólo lo que arrastra (de la lista de cambios, de memoria);
   `true`, además `main` + eso compilado (diagnósticos, `faltan`, ficheros).
4. **La consola no decide.** Al marcar en «Changes vs main» pregunta en seco, y lo arrastrado sale
   marcado, sin poder quitarse, con *Required* y el porqué; al proponer manda lo elegido y ORE lo
   vuelve a añadir. El `Package` y el `Schema` no son filas: los cubre la casilla de su base, su
   schema o su origen.

**No se arrastra** lo que **lee a** lo propuesto (un consumidor de `main` que se rompe): añadirlo
rompería `main` por otro lado, y se dice en los diagnósticos (A.2 ④).

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

#### Descartado en la medida

- **Copiar los ficheros de la rama sobre `main`** para construir el alcance: pisa lo que `main`
  cambió después.
- **Squash al fusionar**: rompe la reconciliación de la rama; se fusiona con `merge`.
- **El alcance por carpeta** (el `X-Ore-Raiz` de hoy) como alcance de lo que se fusiona: parte
  documentos que comparten fichero y no sabe de renombrados.

## Apéndice B · La rama protegida: `main` libre o protegida (hecho el 2026-09-28)

**Estado:** B.1–B.6 hechos y desplegados (ORE `eab95db`…`7af06ea`, consola `54afb2b` y `57ef007`;
`la-propuesta.sh` 6, 8, 8e, 8g y 8h); P2 pendiente (§ B.7).

### El problema

La regla de 0030 W2 —«dos personas, una revisión»— se cumplía al fusionar y en ningún otro sitio:
quien propone no aprobaba ni fusionaba lo suyo, pero **cualquiera con sesión escribía en `main`
sin propuesta**. La revisión no protegía nada (quien quisiera saltársela escribía en `main`) y sí
encerraba a quien trabaja solo: abría su PR y no podía fusionarla nunca. ore-serve no sabe quién
es admin (su `Identidad` es persona, agente, correo y nombre; ninguna de las potestades de
`ore-iam` es sobre el árbol), así que «el admin puede» no era una salida sin P2.

### Lo decidido

- **B.1 · La política es de `main`, y es un fichero del árbol:** `.arbol/ramas.yaml`,
  `main: { protegida: true }`. Se **lee siempre de la rama por defecto** (si no, una propuesta
  podría aflojar la regla que se le aplica), por la API de la forja sin clonar. Sin fichero, o
  roto, `main` es **libre**: lo que el árbol ya era. `GET /ramas` dice `protegida`.
  - *Por qué un fichero:* cambiarla tiene que poder ir por una propuesta (B.4), y una propuesta
    sólo lleva contenido del árbol. La protección de rama de la forja es un ajuste (ni PR ni
    historia), `ontology.config.yaml` es el manifiesto de la spec (la forja no es ontología: otra
    implementación de OOS sin forja no tiene ramas) e IAM no lo ve ore-serve.
  - *Por qué ahí:* el compilador no entra en carpetas ocultas (`validate.rs`, `recolectar`): es
    maquinaria del árbol, como `.github/`; y no en `.ore/`, que es la caché y está en el
    `.gitignore`. Precedente: el repositorio (0036), un fichero de ORE que el compilador no ve.
- **B.2 · Protegida, el árbol no se escribe en `main` sin rama: `423`.** Una sola guarda, en
  `escribiendo_en` sin rama —el único camino por el que se escribe el árbol en `main`: editor,
  commit, documentos, el `CREATE VIEW` del puesto, bases, proyectos, repositorios—. **Las
  operaciones de la celda no** (`escribiendo` a secas: fuentes, modelos, datasets, copiar a la
  celda, decisiones): dar de alta una fuente sólo existe en `main`, y protegerla no puede dejar
  la celda sin fuentes.
- **B.3 · Quién fusiona lo dice la política** (`quien_fusiona`, una regla para la propuesta de rama
  entera, la de alcance y el detalle). **Libre:** fusiona cualquiera, la autora también, con
  revisión o sin ella; el merge lo dice («fusionada sin revisión por X»). **Protegida:** 0030 W2,
  otra persona con una aprobación vigente. En las dos, **nadie aprueba lo suyo**. Si la política
  no se lee, no se fusiona. `GET /propuestas/{n}` da `fusion: {puede, porque}` para quien mira, y
  la consola lo obedece (*Merge without review*) en vez de copiar la regla.
- **B.4 · Cambiarla:** `PUT /ramas/main/proteccion {protegida}`. **Proteger** una `main` libre se
  escribe ya (un commit de la persona). **Liberar** una protegida **no se escribe**: abre una
  propuesta con sólo la política (`<persona>/libera-main`) que otra persona aprueba y fusiona,
  como cualquier cambio de una `main` protegida —si no, protegerla no protegería nada—. Hasta P2
  lo pide cualquiera con sesión, lo mismo que hoy cualquiera escribe en una `main` libre.
- **B.5 · Dónde se ve:** la vista *Branches* (Code Repositories y el catálogo, la misma): la
  etiqueta *Protected* sale de ORE, y en la fila de `main` un candado abre el modal que explica
  qué cambia. En la rama protegida el panel de commit no commitea, y el selector de ramas del
  catálogo enseña el candado.
- **B.6 · Nace libre.** No se elige al crear la celda (el formulario es de infraestructura) ni en
  *Governance*: es una propiedad de la rama, y se gestiona donde están las ramas, como en GitHub
  y GitLab.

### B.7 · Lo que queda (P2)

- **ore-serve pregunta a IAM** (como ya hace `ore-cofre`): una potestad
  `propuesta:fusionar-sin-revision` para ORGADMIN y ACCOUNTADMIN, con la que el admin **aprueba su
  propia propuesta de liberar `main`** y fusiona con `main` protegida sin revisión (queda dicho).
- **Quién cambia la política:** con P2, sólo quien tenga la potestad.
- **Aprobaciones por dueño** (A.2, «lo que falta»): la política podrá exigir la del dueño de cada
  paquete que la propuesta toca, en vez de «otra persona».
- **Los orígenes van a `main` sin mediación** (B.2): una conexión se gobierna con potestad
  (`fuente:crear`), no con revisión —como en Foundry o Databricks—; sus punteros sí van por rama
  (0045, A.3).

## Apéndice C · Los datos en una rama (medido y decidido el 2026-09-29)

**Estado:** decidido, medido (D0) y cotejado; **D1 y D2 hechos y en vivo** (2026-09-30: la
pasada del mantenimiento en `demo` recoge con lo que reclaman las demás ramas); **D3, D4 y D5
hechos en local** (`pruebas-de-fuego/los-datos-en-una-rama.sh` 1–15, consola incluida), en vivo
tras desplegar; D6 por hacer (§ C.6). Es lo que el
punto 4 y «Lo que queda fuera» dejaban para después: aquí, y no en un ADR aparte, porque una rama
con datos es la misma rama global con una cosa más.

### El problema

Una rama ya tiene **definiciones** propias (punto 3) y **ya puede tener bytes**: un puesto, `/v1`
o un trabajo en una rama escriben el puntero del dataset en esa rama (0031 §11 ⑦; 0033, «vive en
la rama»). Lo que no sabe de ramas es todo lo demás:

- **el Job de la copia** clona `main`, materializa y empuja a `main` (`malla/48-la-copia.yaml`);
  por eso copiar, rehacer, ascender y decidir fuera de `main` son `409`
  (`solo_en_la_de_por_defecto`);
- **la recogida** —la del Job de la copia y la del mantenimiento nocturno (`53`)— cuenta sólo con
  los punteros de `main`;
- **el fallback** (0031 §4) está sólo en el puesto (`datos_del_puesto`), y lee los punteros de
  `main` **congelados** en el punto del que salió la rama;
- **fusionar** es fusionar ficheros: los punteros (`datasets/**/*.json`) se mezclan como texto.

### C.1 · Lo medido (D0)

`pruebas-de-fuego/medida-los-datos-en-una-rama.sh`, en local: la forja pelada, el S3 de mentira,
`ore-serve` como catálogo y PyIceberg por `/v1` con `x-ore-rama` (el camino de un puesto).
`main` tiene `ventas.base` (10 filas) y su copia mantenida `ventas.copiaBase`; sale la rama
`bea/datos`; `main` anexa 10 a `base`, crea `soloMain` (4) y rehace la copia (20); la rama anexa 5 a
`base` y crea `nueva` (3).

| | `main` | la rama |
|---|---|---|
| antes de nada | base 20 · soloMain 4 · copiaBase 20 | base 15 · nueva 3 · copiaBase 10 |
| **M1** · `main` recoge (`ore datasets --recoger --edad 7d`, `ore materialize --recoger`) | intacto | **base y nueva rotas** (32 → 23 objetos: 4 ficheros «que nadie nombraba» y 1 dataset huérfano) |
| **M2** · la rama copia (`ore materialize --recoger` en un clon de la rama) | **soloMain y copiaBase rotas** | copiaBase 15, en **la misma tabla** que la de `main` |
| **M3** · `git merge` de la rama | conflicto de texto en `base.json` y `copiaBase.json` | |
| **M4** · lo que `main` ganó después | | soloMain `404`; copiaBase 10 (`main` tiene 20) |
| **M5** · `POST /datasets/…/confirmar` con `x-ore-rama` | **el puntero va a `main`** | nada |

Lo que dice: **el modelo aguanta** —antes de recoger, cada lado lee exactamente lo suyo, y dos
cadenas de metadata conviven bajo el mismo prefijo sin pisarse—; lo que no aguanta es lo que
rodea al modelo. La recogida rompe **en los dos sentidos**, y lo primero **ya pasa hoy**: lo que
un puesto escribe en una rama dura hasta la siguiente pasada nocturna de `main`. Que copiaBase
siga viva en la rama tras M1 es suerte: su snapshot aún no ha caducado (7 días).

### C.2 · Lo decidido

1. **Un puntero por rama; la tabla, compartida.** El estado de un dataset en una rama es su
   puntero en esa rama (lo que ya hay): la tabla Iceberg es una, y cada rama nombra su
   `metadata.json`. Ni una ruta por rama ni las refs de Iceberg (una ref es de una tabla, y una
   rama es del árbol entero; 0031 §7).
2. **La recogida cuenta con todas las ramas.** Se conserva lo que nombre el puntero de **cualquier
   rama viva**, no sólo el de `main`. Una rama reclama **sólo sus punteros propios**: los que
   difieren del punto del que salió; lo que heredó de `main` no alarga la vida a los bytes viejos de
   `main`. **Una rama que se borra deja de reclamar**, y lo suyo se va en la pasada siguiente.
3. **Lo que la rama no tocó se lee de `main` al día.** Si el puntero de la rama es igual al del
   punto del que salió, o no existe y `main` lo tiene, manda el de `main` **de hoy**. La regla vive
   una vez (`ore_core::punteros`) y la usan `ore-serve`, `ore` y el puesto; la respuesta dice de
   dónde sale (`de: main | rama`), y el catálogo, *«from main»* o *«built on this branch»*.
4. **Se construye en la rama.** Copiar, rehacer, ascender y decidir en una rama encolan el Job **en
   esa rama**: clona la rama, lee lo no tocado de `main` al día (3), empuja a la rama. Construye lo
   que la rama cambió y, a elección, **lo afectado** aguas abajo (el alcance que `GET
   /ramas/{r}/cambios` ya calcula). Dar de alta o retirar una conexión sigue siendo de `main`: es
   de la celda, no del árbol (B.2).
5. **Fusionar punteros: la regla de git, y una excepción.** Un puntero es un fichero del árbol y se
   fusiona **de tres vías**, por activo, nunca como texto:

   | el activo | resultado |
   |---|---|
   | la rama no lo tocó | gana `main` |
   | sólo lo tocó la rama | gana la rama: su puntero pasa a `main`, sin mover un byte (lo que se revisó es lo que se publica) |
   | los dos, **con receta** (un Dataset con `from`, o la salida de un trabajo o transform) | **se reconstruye en `main`** con la definición fusionada; los bytes de la rama sirvieron para revisar. Mientras, `main` sirve lo que tenía |
   | los dos, **sin receta** (un `write()` suelto, PyIceberg, DuckDB) | **conflicto** en ese activo: se elige `main` o la rama, y la consola dice qué se pierde (lo que `main` escribió desde el punto de salida). Lo aprueba el dueño (A.5) |

   La excepción hace desaparecer el conflicto donde el resultado es reproducible; sólo la última
   fila le pide algo a una persona. Una propuesta de código (por repositorio) no lleva punteros
   (A.7).

### C.3 · Cotejo con la industria

| fuera | qué hace | aquí |
|---|---|---|
| **Project Nessie** | ramas de catálogo sobre tablas Iceberg: en cada rama, cada tabla es un puntero a su `metadata.json`; la GC marca lo vivo recorriendo **todas** las referencias con nombre | **es nuestro modelo** (1) y nuestra recogida (2) |
| **Iceberg** | refs por tabla; la caducidad respeta lo que nombra una rama o un tag; WAP: escribir en una rama y `fast_forward` a `main` «mueve un puntero, no datos» | la promoción de (5) es el `fast_forward`; las refs no, porque son de una tabla |
| **lakeFS** | un objeto sólo se borra si no está en el HEAD de **ninguna** rama; retención por rama; fusión de tres vías por fichero, sin mezclar filas: conflicto, o gana un lado entero | (2) y la regla de (5) |
| **Delta / Unity Catalog** | un `VACUUM` del origen rompe sus *shallow clones* (`FileNotFoundException`); Unity Catalog lo arregla sabiendo qué ficheros necesita cada clon | **es M1 con otro nombre**; y (2) es lo que hizo Unity Catalog |
| **Foundry** | *fallback branches*: lo que no está construido en la rama se lee de `main`; al fusionar, **se reconstruye** en `main` (lo afectado, lo modificado o nada); los *true conflicts* se eligen a mano; una rama inactiva (35 días) pierde sus datos. Su Code Workbook copiaba las transacciones de la rama avisando de que lo de `main` posterior desaparecía | (3), (4) con lo afectado, y la excepción de (5); la rama inactiva, pendiente (C.7) |
| **dbt** | `--defer`: lo que no se construye se resuelve contra producción; `state:modified+`: lo cambiado **y lo que depende** | (3) al día, no congelado; (4) con lo afectado |

Lo que no se copia: en Foundry, crear o borrar un recurso en una rama afecta a `main` al momento;
aquí lo creado en una rama se queda en ella (punto 3).

### C.4 · Lo que se descarta

- **Una ruta por rama** (`ramas/<r>/…`): duplica los bytes que la rama no toca y obliga a mover o
  copiar al fusionar. Medido: no hace falta, dos cadenas de metadata conviven.
- **Las refs de Iceberg como ramas:** son de una tabla; una rama es del árbol (0031 §7).
- **Combinar** los dos lados (reaplicar los anexados de la rama sobre `main`, el `cherrypick` de
  Iceberg): nadie lo hace solo, es un tercer camino con sus condiciones para un caso raro, y quien
  lo necesite fusiona con `main` y vuelve a ejecutar su `write()`.
- **Fusionar los punteros como texto** (M3) y **promocionar siempre** (el Code Workbook): lo
  primero no fusiona, lo segundo borra lo de `main` sin decirlo.

### C.5 · Por qué así, frente al cliente

- **Una rama es de verdad un sitio aparte.** Hoy lo es para las definiciones y a medias para los
  datos: lo que escribe se borra de noche, y lo que lee está parado en el día que salió.
- **El cliente de Foundry reconoce el flujo**: *build on branch*, *from main*, reconstruir al
  fusionar. Y el de Nessie o lakeFS reconoce la mecánica.
- **Nada se pierde sin decirlo.** El único caso que decide una persona es el único donde algo se
  pierde, y se dice qué.

### C.6 · Los pasos

Cada paso termina medido en local (una prueba de fuego) **y en vivo**; ningún paso deja algo roto
para el siguiente. Orden: D1 primero porque sin él lo demás produce bytes que se borran; D5
último porque sin D3 y D4 no hay datos de rama que fusionar. Lo que cambia `malla/` va **después**
del binario que lo entiende (binario primero, malla después).

**D1 · La recogida cuenta con todas las ramas** (ORE; sin consola)

- *D1·0, medir en vivo:* en las forjas de `demo`, `victor` y `prueba`, ¿qué ramas tienen hoy
  punteros propios (distintos del punto de salida)? Es el daño que ya está ocurriendo; se dice
  antes de arreglarlo.
- *D1a, el binario:* `ore datasets --recoger` y `ore materialize --recoger` aceptan `--reclaman
  <fichero>`: los punteros propios de las demás ramas (JSONL). Entran en `recoger-huerfanas` (el
  dataset se reclama) y en `recoger` por tabla (`ore-store`: lo alcanzable es la unión de lo que
  nombran todas las `metadata_location` de esa tabla, no sólo la de esta rama). `ore` no aprende
  git: la lista la hace quien clona.
- *D1b, la malla:* `48` y `53` (y el `53` de colecciones) construyen ese fichero en el clon, que ya
  trae todas las ramas: por cada `origin/<r>`, los punteros que difieren de su `merge-base` con la
  rama que se recoge.
- *Prueba:* la medida pasa a prueba de fuego, `pruebas-de-fuego/los-datos-en-una-rama.sh`: M1 y
  M2 dejan los dos lados legibles; borrar la rama libera lo suyo en la pasada siguiente.

**D2 · `confirmar` en la rama** (ORE)

- `POST /datasets/…/confirmar` usa `x-ore-rama` como `/v1`, con la misma regla de `main`
  protegida (B). *Prueba:* M5 da el puntero en la rama y no en `main`.

**D3 · Lo no tocado, de `main` al día** (ORE + consola)

- *D3a:* la regla en `ore_core::punteros` (puntero de la rama, del punto de salida y de `main` →
  el que manda y de dónde), con sus pruebas unitarias.
- *D3b:* `ore` la usa con `--respaldo <main> --base <merge-base>` (lo necesita el Job en una rama,
  D4); `ore-serve` en `/v1` `loadTable`, la ficha y la lista de datasets, y el puesto (sustituye
  el fallback de `datos_del_puesto` por la regla). Las respuestas dicen `de`.
- *D3c, consola:* en una rama, cada dataset dice *«from main»* o *«built on this branch»*.
- *Prueba:* M4 da soloMain 4 y copiaBase 20 desde la rama mientras la rama no los toque.
- *Hecho así (D3a, D3b):* la regla es `ore_core::punteros::manda`, sobre la huella del blob en
  la rama, en su `merge-base` con `main` y en `main`. No la aplica cada lector —hay más de treinta
  sitios que leen punteros, en `ore` y en `ore-serve`—: **el árbol de la rama se prepara al día**
  (`Forja::superponer`), en `datasets/` y `copias/`, y deja `.ore-al-dia.json` con lo que sale de
  `main`. Lo hacen los árboles de lectura del espejo (la caché lleva la cabeza de `main` en la
  clave), `/assets` y **también la escritura** (`escribiendo_en`): si no, `/v1` cargaría el snapshot
  de `main` y confirmaría contra el congelado de la rama. Antes de publicar, lo superpuesto que la
  escritura no tocó vuelve a como está en la rama (`deshacer_al_dia`): sólo lo escrito pasa a ser
  suyo. Medido en la prueba: una rama que anexa a un dataset heredado lo hace sobre el de `main` de
  hoy (20 + 5 = 25), no sobre el congelado.

**D4 · Construir en la rama** (ORE + malla + consola)

- *D4a, el binario:* `ore-serve` encola la copia con la rama (`cola::rendir_copia` con
  `RAMA`; el nombre del Job y del fichero de la cola la llevan; un fichero por rama, que se retira
  con la rama). Copiar, rehacer, ascender y decidir en una rama dejan de ser `409`; dar de alta y
  retirar una fuente lo siguen siendo. Qué construye: lo que la rama cambió (`cambios.rs`) y,
  con `afectados: true`, lo que alcanza.
- *D4b, la malla:* `48` con `RAMA`: clona la rama, trae `main` para el respaldo (D3), recoge con
  `--reclaman` (D1) y empuja a la rama.
- *D4c, consola:* *Build on branch* y *Build affected* donde hoy sale el `409`.
- Una copia en una rama **no se refresca sola**: se construye cuando se pide (el refresco
  periódico es de `main`).
- *Prueba:* la rama construye copiaBase (15), `main` sigue en 20, y la recogida de los dos lados
  los deja vivos.
- *Hecho así (D4a–D4c):* copiar, ascender, modelar, decidir y rehacer en una rama escriben en su
  árbol (`Servidor::moviendo_datos`) y encolan con la rama (`copia::EnRama`, una marca de la
  petición: el encolado está en lo hondo de `tras_inducir`). Lo que se construye lo calcula
  `ore-serve` (`vistas_de_la_rama`): los mantenidos cuyo documento difiere del punto de salida
  —lo recién escrito cuenta— y lo que sale de ellos por `from: { dataset }`, hasta el final. La
  cola lleva `48-la-copia-rama-<h>.yaml` (`copiar-rama-<h>`, la rama dentro del resumen), que la
  convergencia conserva como los de rehacer; una plantilla sin el hueco `RAMA` es un error, no una
  copia en `main`. En el Job (`malla/48`, con `RAMA` vacío es `main`, lo de siempre): clona la
  rama, **`ore overlay . --main origin/main`** —lo de ③ en el clon, con la misma regla—, construye
  sólo `VISTAS`, reclama con todo `main` y lo propio de las demás, **`ore overlay . --undo`** y
  empuja a la rama. Dar de alta o retirar una fuente sigue siendo `409` en una rama (es de la
  celda). En la consola, «Copy into cell» ya mandaba la rama; «Copy now» (rehacer, desde el code
  workspace) la manda ahora. Medido en la prueba 11: la pasada del Job en la rama construye
  copiaBase con la base de la rama (25) y `main` sigue en 20; sólo lo construido pasa a ser suyo.

**D5 · Fusionar punteros** (ORE + consola)

- *D5a:* en la derivada de una propuesta de activos (A.2 ③, que ya se regenera sobre el `main` de
  hoy) los punteros **no se copian de la rama**: se resuelven por la tabla de (5). Con receta:
  queda el de `main` y, tras fusionar, se encola la reconstrucción en `main` (D4 en `main`). Sin
  receta y con los dos lados movidos: `409` con la lista, hasta que la propuesta lleve `datos:
  {"<activo>": "main" | "rama"}`. Una propuesta por repositorio no lleva `datasets/` (A.7).
- *D5b:* `GET /propuestas/{n}` (y el `seco`) da `datos: [{activo, caso, resultado, se_pierde?}]`.
- *D5c, consola:* la pestaña *Data* de la propuesta del catálogo: *promote*, *rebuild on main*, o
  el conflicto con su elección y lo que se pierde.
- Tras fusionar la rama sigue viva (A.8): sus punteros ya son los de `main`, y dejan de reclamar
  solos (D1).
- *Prueba:* `la-propuesta.sh`, un bloque con las cuatro filas de (5).
- *Hecho así (D5a–D5c):* `GET /ramas/{r}/cambios` trata el puntero de un Dataset como un anexo
  suyo (como los `discover.*` de un Package): un puntero propio lo hace «modificado» con `datos` y
  lo lleva en `ficheros`, aunque su definición no cambie; si `main` también lo movió, `enBase:
  datos`. En la derivada (`Forja::derivar`) los punteros **entran en la huella** —si la rama
  reconstruye tras proponer, hay que revisar otra vez— **pero no en el parche**: cada uno se
  resuelve por la tabla de (5) (`Forja::fusionar_puntero`), y «con receta» es el puntero de una
  copia —lo escribió su pasada, sin `escrito_por`—. La propuesta (y su seco) dice `datos: [{activo,
  caso, resultado, se_pierde}]`; fusionar con uno `sin elegir` es `409` con la lista, y el cuerpo
  `{"datos": {"<dataset>": "main" | "rama"}}` lo decide. Tras fusionar, lo que tenía receta se
  reconstruye en `main` (`reconstruir_en_main`: la copia de `main`), y **poner la rama al día**
  (`traer`) resuelve cada puntero movido en los dos lados por fichero entero: lo fusionado toma el
  de `main`, lo demás sigue siendo de la rama (`fusionar_en_rama`, el «traer main» del workspace,
  deja siempre el de la rama). En la consola, el modal de *Merge* pide la elección cuando ORE la
  pide —*Keep main* / *Take this branch*, con lo que se pierde— y dice qué se promociona y qué se
  reconstruye. Medido en la prueba 12–15: `main` lee base 25 (el `metadata.json` de la rama, sin
  mover un byte) y nueva 3; copiaBase sigue en 20 y su reconstrucción queda encolada; la rama se
  pone al día y lo fusionado deja de ser suyo.

**D6 · Cerrar**

- Este apéndice pasa a hecho con sus medidas en vivo; el README de decisiones; el `409` del
  punto 4 se reescribe (lo que queda en `main` es la conexión).

### C.7 · Lo que queda fuera

- **La rama inactiva** (Foundry: 35 días y sus datos se van): hoy una rama vive hasta que se borra.
- **La caducidad de los snapshots de una rama**: el mantenimiento expira los de `main`; los de
  una rama viven lo que viva la rama.
- **Combinar** los dos lados (C.4) y **las filas en el diff** de una propuesta (A.6, «más adelante»).
- **Dar de alta una conexión en una rama** (B.2: es de la celda).
- **Las colecciones mantenidas en una rama** (0046 E8): la copia de una rama construye datasets;
  las colecciones siguen en la pasada de `main`.
- **Contestar decisiones desde la consola en una rama:** ORE ya lo hace en la rama; la consola
  todavía no manda la rama en `contestar-decisiones`.
- **Las propuestas de rama entera** (sin alcance, 0030 W2) las fusiona la forja: dos punteros
  movidos chocan como texto y la forja dice que no es fusionable (`409`). Los datos se promocionan
  por la propuesta del catálogo (A.7).
- **La salida de un trabajo o de un transform** cuenta hoy «sin receta» (su puntero lleva
  `escrito_por`): reconstruirla sería volver a correr el trabajo en `main`, y eso no se encola solo.
- **La vida del fichero de la cola de una rama:** se conserva como los de rehacer; retirarlo
  con la rama queda por hacer (hoy el Job, con su `ttl`, vuelve a pasar).
