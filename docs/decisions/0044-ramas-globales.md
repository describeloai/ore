# 0044 · Ramas globales: una rama es del árbol entero, y el catálogo la enseña

**Estado:** decidido; fases 1 y 2 hechas (2026-09-27), los datos en ramas por decidir · **Decide:**
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
