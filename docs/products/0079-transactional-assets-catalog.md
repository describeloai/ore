# 0079 · Transactional Assets Catalog: el estado vivo de los datos, fuera de git

**Estado:** borrador (2026-10-28). Diseño y hoja de ruta. No hay código. Nace de la medida E1·0 de
[0078](0078-odyssey-vector-assets.md): hoy el commit de datos cuesta segundos y no escala, y sin
arreglar eso ninguna promesa de milisegundos es posible.

*Transactional Assets Catalog* es la capa que guarda **el estado vivo de cada activo de datos**
(dónde está su snapshot actual en cada rama, su frescura, sus estadísticas y su historia) en una
base transaccional modelada para eso, en vez de en ficheros del árbol de git. **El cliente no la
ve.** Sigue usando su catálogo Iceberg (PyIceberg, DuckDB, Spark, el SDK, la consola) exactamente
igual. Lo único que nota es que commitear es instantáneo y que la concurrencia deja de ser un
problema.

## 1. Por qué

E1·0 (2026-10-28, victor). Mover el puntero de un dataset reproduciendo, fase a fase y dentro del
pod de ore-serve, lo que hace cada escritura:

| Fase | Tiempo |
|---|---|
| Poner al día el espejo del árbol | 110–420 ms |
| Clonar el árbol desde el espejo (1.632 ficheros) | ~600 ms |
| `ore` sobre el árbol | ~100 ms |
| Commit de git | 110–550 ms |
| Push a Forgejo | **~1.500 ms** (1.450–2.280) |
| **Total por commit** | **2,6–3,6 s** (5,2–6,6 s si cae al clon de la forja) |

Y los commits de un árbol van **en serie** (un cerrojo en ore-serve, una forja por inquilino, ambos
con 0,5 CPU de límite): el techo es de **~0,3 commits por segundo por inquilino**, sumando todas sus
tablas. Un stream que confirme una vez por segundo en una sola tabla ya lo satura.

El problema no es de optimización: **cada escritura de datos es hoy un commit de git del árbol
entero.** Git es la herramienta correcta para lo que deciden las personas y la incorrecta para lo
que una máquina cambia muchas veces por segundo.

**Objetivo:** commit de un puntero con evento incluido, p99 ≤ 10 ms; ≥ 1.000 commits/s por celda.
Frente a hoy: unas 600 veces menos latencia (casi 3 órdenes) y unas 3.000 veces más caudal.

## 2. El principio: git guarda lo que se decide, el catálogo guarda lo que ocurre

| Se queda en git (lo deciden personas, se revisa) | Sale al catálogo (lo escriben máquinas, ocurre) |
|---|---|
| `packages/**`: Table, Dataset, View, Function, transforms, ontología | `datasets/**/*.json` (157 en victor) |
| `policies/`, `rulesets/`, `proyectos/`, `conduits.yaml`, `interfaces/`… | `copias/**` (los de antes) |
| Las ramas de git y las propuestas de código | La capa de datos de cada rama |

Lo que hoy se llama "puntero" es **el estado vivo entero del dataset**, y sale entero:

- dónde está: `metadata_location`, `snapshot`, `ubicacion`, `dataset`;
- estadísticas: `filas`, `bytes`, `ficheros`, recuento por columna;
- frescura: `estado`, `datos_de`, `intento {cuando, resultado}`, `leidas`, `operacion`;
- el origen: `testigo` (la marca de agua) y las huellas `plan`, `bundle`, `cabecera`.

La **definición** de un dataset (su `Table`/`Dataset` en `packages/`, su dueño, sus políticas, sus
promesas) no se mueve.

## 3. Las ramas: un concepto, dos almacenes

**El paradigma de ramas no se reinventa ni se migra entero; se migra su capa de datos.** Una rama
sigue siendo una sola cosa con un solo nombre:

- la rama de git guarda sus definiciones (como hoy);
- una **ref del catálogo con el mismo nombre** guarda sus punteros.

Las reglas ya están decididas: [0044 §C.2](0044-ramas-globales.md) tomó a Project Nessie como
"nuestro modelo" y las implementó a mano sobre ficheros (`ore_core::punteros::manda`). Este ADR
las traduce a transacciones:

| Regla de 0044 §C.2 | Hoy, sobre git | En el catálogo |
|---|---|---|
| ① Un puntero por rama; la tabla Iceberg, compartida | Un fichero por rama | Una fila `(ref, activo) → estado` |
| ② La recogida respeta lo que nombre cualquier rama viva | Recorrer todas las ramas de git | Una consulta |
| ③ Lo que la rama no tocó se lee de `main` al día | Superponer el árbol de `main` sobre la rama | "La fila de la rama; si no, la de `main`": un índice |
| ④ Se construye en la rama | Clonar la rama, empujar a la rama | Commit en la ref de la rama |
| ⑤ Fusionar punteros a tres vías, por activo (gana uno, se reconstruye con receta, o conflicto) | Diff de ficheros | Una transacción atómica sobre N activos |

La regla ③ mejora al moverse: deja de exigir "preparar el árbol al día", que hoy es parte del coste.
La ⑤ gana atomicidad real entre tablas.

## 4. Historia, versiones y auditoría

Hoy el `git log` de cada puntero es su historia, y el commit dice quién (autor = la persona;
committer = el servidor: `sub` y `act` de RFC 8693). Eso no se pierde: pasa a un **log de commits
del catálogo**, de solo añadir, con la misma forma que el de Nessie:

`{commit, ref, padre, activos: [{activo, de, a}], sujeto, actor, cuando, operacion, idempotencia}`

Lo que se gana:
- **viaje en el tiempo** de todo el catálogo ("cómo estaba todo a las 10:00", por commit o por
  hora);
- **diff entre ramas** y **tags** baratos;
- el log se expone como **tabla del propio lago** (`system.catalog.commits`), consultable con SQL;
- base para las versiones de datos de [0056](0056-versioning.md) (V3–V6).

## 5. El evento sale de la transacción

El commit es un *compare-and-set*:

```sql
UPDATE estado SET snapshot = :nuevo, ...
 WHERE ref = :rama AND activo = :tabla AND snapshot = :esperado;   -- 0 filas = 409, como hoy
INSERT INTO commits ...;                                            -- la historia
INSERT INTO outbox ...;  NOTIFY catalogo;                           -- el evento
COMMIT;
```

**Si el commit entra, el evento existe; si no, no.** El "evento en el mismo instante" de la pieza 1
de [0078](0078-odyssey-vector-assets.md) sale gratis y exacto, y la outbox es el log re-leíble con
el que un consumidor se pone al día. Una transacción de N tablas (`transactions/commit`) es un
commit con N filas.

## 6. Invisible para el cliente

La cara no cambia:
- el catálogo REST de Iceberg (`/v1/...`, `loadTable`, `updateTable`, `transactions/commit`);
- los SDK, `POST /datasets/.../confirmar`, Query, la consola.

Cambia lo que ore-serve hace detrás. Los códigos de la spec se conservan: `409
CommitFailedException` en el *compare-and-set* y `5xx CommitStateUnknownException` si no se sabe.

## 7. El almacén: Postgres en hoja limpia, no Neon

**Decisión (borrador): un Postgres normal, pequeño y siempre encendido, dedicado al catálogo, con un
esquema modelado para esto. No la pila Neon de [0058](0058-ore-serverless-postgres.md).**

Neon es un producto entero (equivalente a Lakebase): pageserver, tres safekeepers, storage
controller, proxy, Redis, escala a cero y ramas de base de datos. Para el catálogo:

- **La escala a cero es incompatible con estar en el camino de cada escritura.** En 0058 se midió
  el arranque en frío en 39,5 s p50 (3,9 s con pool). El catálogo no puede dormir.
- **El commit en Neon da una vuelta de red más:** el WAL va por consenso a los safekeepers antes de
  confirmar. Para un camino que quiere p99 ≤ 10 ms, cada salto cuenta.
- **El coste:** la pila Neon lleva 14 días apagada (`Pending`) por presupuesto. Encenderla entera
  para guardar unos miles de filas es desproporcionado.
- **Sus ramas de base de datos no son nuestras ramas.** Nuestras ramas son por activo, con reglas
  de fusión propias (§3); las de Neon son copias de la base entera.

**Lo que sí se recicla de 0058:** el conocimiento operativo (imágenes, copias de seguridad y WAL a
GCS, pgbouncer cuando haga falta, monitorización) y el patrón de despliegue en la malla.

**Cómo se optimiza para este fin** (hipótesis que E1a·1 mide):
- tablas estrechas: el estado caliente por `(ref, activo)` en una fila corta; las estadísticas
  grandes (recuento por columna), aparte;
- `fillfactor` bajo en la tabla de estado para que cada commit sea una actualización HOT;
- el log de commits particionado por tiempo, de solo añadir;
- conexiones persistentes desde ore-serve con sentencias preparadas, sin saltos intermedios;
- `synchronous_commit = on` (la durabilidad no se negocia) y WAL en disco rápido;
- `LISTEN/NOTIFY` para el aviso y la outbox para la re-lectura.

**Abierto:** una instancia por celda o una compartida con el inquilino en la clave; réplica en
caliente frente a nodo no-spot (hoy el nodo de sistema es spot y hoy mismo se reemplazó uno).

## 8. Lo que se pierde o se complica

- **"El árbol lo es todo."** Hoy clonar el repo da el estado completo. La capa de datos deja de
  estar ahí: copias de seguridad propias y, quizá, una exportación periódica legible.
- **El modo local** (`Arbol::Directorio`, las pruebas, `ore` sin servidor) necesita un catálogo
  embebido con el mismo esquema (SQLite).
- **Disponibilidad:** si el catálogo cae, no hay escrituras. Necesita alta disponibilidad y copias.
- **Fusionar una propuesta toca dos sistemas** (git y el catálogo): hace falta un protocolo (§9,
  E1a·3).
- **Una pieza más que operar**, crítica.

## 9. Preguntas abiertas

1. ~~**El protocolo de fusión.**~~ Resuelto en E1a·3 (§16): el catálogo prepara (plan y cerrojo),
   git decide, el catálogo aplica; el catálogo no va primero. Un lector ve, durante milisegundos,
   las definiciones nuevas con los punteros viejos.
2. **Quién lee qué campo.** Inventario exacto (E1a·0): hoy 31 ficheros en 4 crates (17 en
   ore-serve, 11 en ore-cli) tocan punteros.
3. **Flux y el árbol.** Comprobar que nada que aplique Flux dependa de `datasets/**`.
4. **Colecciones de medios** (0046/0049) y **teselas** (0072): ¿tienen su propio estado vivo en el
   árbol que deba seguir el mismo camino?
5. **Recogida** (`recoger-huerfanas`): pasa a leer las refs del catálogo.
6. **Cuotas y multi-inquilino** en una instancia compartida.

## 10. Hoja de ruta

| Paso | Qué | Hecho cuando |
|---|---|---|
| ~~E1a·0~~ | **Hecho en lo que es leer código (2026-10-28, §11).** Pendientes: el segundo suelo (medir desde un puesto) y la decisión de dónde vive el Postgres | La tabla completa en este ADR |
| ~~E1a·1~~ | **Hecho (2026-10-28, §12).** p99 ≤ 10 ms hasta ~500 commits/s; techo ~980/s con `NOTIFY` por commit y ~1.670/s sin él, en 0,5–2 CPU compartidas | Commit + evento con p99 ≤ 10 ms y ≥ 1.000 commits/s, medido con concurrencia real |
| ~~E1a·1b~~ | **Hecho (2026-10-28, §13).** Con CPU propia y el evento por decodificación lógica: 1.000/s con commit p99 3,3 ms y evento p99 3,0 ms; ~2.860/s sin que el Postgres sature | Vara de E1a·1 cumplida entera |
| ~~E1a·2~~ | **Hecho (2026-10-28, §15).** 33/33 casos de las cinco reglas y 0 diferencias frente a `ore overlay` en las 18 ramas reales de victor (2.850 combinaciones) | Mismos resultados que `ore_core::punteros::manda` en los casos de 0044 |
| ~~E1a·3~~ | **Hecho (2026-10-28, §16).** Preparar en el catálogo (plan + cerrojo), merge de git, aplicar el plan; un reconciliador remata. 879 corridas con fallos inyectados en cada costura: 0 diferencias frente a la fusión de hoy | Fusión que sobrevive a un fallo entre los dos pasos |
| **E1a·4** | **Preparar el corte** (recortado el 2026-10-28, §17.8): el catálogo de victor dado de alta y cargado con su historia por el andamio, el editor cerrado y el binario en la imagen. **Sin sombra permanente.** ·4a y ·4b hechos (§17.6, §17.7) | Una carga de ensayo en victor con 0 diferencias, y todo lo que el corte necesita, desplegado |
| **E1a·5** | El corte, inquilino a inquilino; git deja de recibir `datasets/**`. En pasos (§19): ·5a inventario (hecho), ·5b la interfaz `Punteros`, ·5c su implementación sobre el catálogo, ·5d los Jobs, ·5e `guardar` y el documentador, ·5f §18 en la malla, ·5g el corte | Escribir en una tabla cuesta milisegundos en victor |

Al cerrar cada paso, el número va a §1/§7 y la decisión a su sección. El ADR se reescribe limpio al
cerrar la etapa.

## 11. E1a·0: lo que hay hoy (2026-10-28)

Inventario leyendo el código y el árbol de victor. Referencias con fichero y línea en las notas de
la sesión; aquí, lo que decide el diseño.

### 11.1 Los números de victor

- 785 commits en el árbol; **59 de datos en los últimos 7 días** (~8 al día), muchos multitabla
  (`Copia: a, b, c`). **10 de esos 59 (17 %) tocan también `packages/`**.
- 157 punteros en `main`; **19 ramas, 6 con punteros propios** (49 punteros entre todas). La
  migración las tiene que conservar exactamente.
- Ningún nodo siempre encendido es no-spot. `idp-db-0` (Postgres 16, la identidad) ya corre en el
  nodo spot de sistema: es el precedente.

### 11.2 Escritores: 19 caminos y 3 primitivas

- Casi todo pasa por `escribir_puntero_sin` (fusiona sobre el anterior), `escribir_informe`
  (sustituye el objeto entero si la pasada va bien) o un `fs::write` crudo (`recoger`).
- Entradas: `/v1` (`updateTable`, `transactions/commit`, `createTable`), `/confirmar`,
  `publicar-retenida`, la copia (Job 48, copiador residente, refresco, rehacer), las colecciones
  (mantenidas y escritas), el mantenimiento (`recoger` de datasets y colecciones), retirar un
  Dataset, retirar una base, renombrar un schema, las migraciones, las fusiones y **el editor**
  (`PUT /arbol/...`, que hoy puede escribir `datasets/**` a mano).
- **El *compare-and-set* semántico por tabla solo existe en `/v1` y `/confirmar`** (código 75 →
  409). Todo lo demás depende del push de la rama: **cualquier commit concurrente en la rama da
  409, aunque sea de otra tabla.** La copia reintenta con rebase (×5); el mantenimiento falla
  entero; ore-serve no reintenta.

### 11.3 Los commits mixtos: la decisión principal

Un commit es mixto cuando mueve un puntero y además cambia una definición en `packages/`. Pasa en:

| Camino | Cuándo toca `packages/` |
|---|---|
| `/v1` `updateTable`, `transactions/commit`, `createTable` | El `Dataset` escrito por máquina (marcado `# Un dataset escrito (0033)`): **nace** en la primera escritura, y se **regenera** si cambian las columnas, `anchoredTo`, las claves de `changes`, **o el `derivedFrom`** (basta con que la sesión haya leído otras tablas). Un documento escrito a mano no se toca nunca |
| `/confirmar`, `publicar-retenida` | Si llegan columnas |
| Commit de una colección escrita | El `derivedFrom` de la `MediaCollection` |
| Retirar Dataset, retirar base, renombrar schema, migraciones, fusiones | Siempre |

Los Jobs de copia, mantenimiento y colecciones mantenidas **no** son mixtos: solo tocan punteros.

**Consecuencia:** el camino caliente de escritura (`/v1`) puede necesitar git en cada escritura que
cambie el linaje, aunque los datos no hayan cambiado de forma. Opciones (decisión pendiente):

- **(a)** El `Dataset` escrito por máquina pasa también al catálogo: es "lo que ocurre", no "lo que
  se decide". Git solo guarda los `Dataset` que escribe una persona.
- **(b)** El puntero va al catálogo al instante y el documento se actualiza en git **después**, solo
  cuando cambia de verdad (nacimiento, esquema, linaje). El commit de datos no espera a git.
- **(c)** Mantener el commit mixto síncrono en esos casos (pierde la promesa en ellos).

**Decidido (2026-10-28): (b), git después.** Cómo funciona:

- El commit de datos **solo toca el catálogo**: el puntero, la historia y el evento, en una
  transacción. Nunca espera a git.
- Un consumidor de la outbox, el **documentador**, mantiene el `Dataset` escrito por máquina en
  git: lo crea al nacer el dataset y lo regenera solo cuando cambian de verdad las columnas,
  `anchoredTo`, `changes` o `derivedFrom`. Agrupa varios cambios seguidos en un solo commit y es
  idempotente: el documento refleja el último estado del catálogo, no cada escritura.
- Un `Dataset` escrito a mano sigue sin tocarse nunca.
- **(a) queda como destino** si se mide que el linaje cambia a menudo y el documentador se vuelve un
  segundo cuello.

Consecuencias que hay que asumir o resolver en E1a·2:
- **Ventana de segundos** entre el commit de datos y el documento en git. Los datos se leen al
  instante por `/v1`; lo que lea `spec.columns` o el linaje del documento verá el estado anterior
  durante esa ventana.
- **Al nacer**, el dataset existe en el catálogo antes que su `Dataset` en git. Lo que se indexa por
  documento (el índice `/assets`) lo verá al llegar el documento, salvo que pase a indexar desde el
  catálogo.
- Si el documentador falla, el documento queda atrás: es un consumidor más, con su retraso medido
  en Health.

Las operaciones de definición (retirar, renombrar, fusionar, migrar) siguen siendo raras y pueden
seguir yendo por git más el catálogo, con el protocolo de E1a·3.

### 11.4 Lectores: ~40, todos desde git

- Casi todos leen del espejo de ore-serve; los Jobs, de un clon propio. **Nada lee punteros desde
  fuera de git.**
- Leen el **árbol entero**: el índice `/assets` (y de él, Health y el refresco cada 60 s),
  `ore datasets` para listar (y también para la ficha y la muestra de uno solo), `/v1` listar
  tablas, retirar base, recoger y migrar.
- Leen **la historia de git**: `/historia`, `/promesas/historia`, el as-of de las colecciones,
  `escrita_desde` de los Builds, el autor y la fecha de `informe_de`. **Hace falta una tabla de
  versiones** en el catálogo.
- El puntero tiene más campos de los listados en §2: `escrito_por`, `motivo`, `comprobado`,
  `senal`, `promesas`, `esquema_origen`, `clave`, `kind`, `transaccion`, `items`, `cambios`,
  `virtual`, `por_copiar`, `escritura`/`procedencia`, `version_retenida`, `uuid`, `tabla`.
- **Flux no reconcilia `datasets/**` ni `copias/**`**: ningún `GitRepository` apunta al árbol del
  inquilino. Lo que cambia son los scripts de los Jobs que clonan el árbol y empujan punteros
  (`48-la-copia`, `53-el-mantenimiento`, `copiar-worker.sh`, `cerrar-ejecucion.py`): pasan a hablar
  con el catálogo.

### 11.5 Ramas y propuestas: 15 comportamientos a conservar

1. La resolución por rama de `manda` (lo propio, lo borrado, y el resto de `main` **al día**),
   guardando el punto de salida de cada rama y actualizándolo al traer `main`.
2. Decir de dónde sale cada puntero (`de: main | rama`), incluido el reintento en `main` de los
   datos del puesto.
3. El *compare-and-set* en la rama contra el puntero **efectivo** (que puede ser el de `main`);
   escribir sobre algo heredado lo hace propio.
4. Las escrituras atómicas árbol + catálogo (colecciones escritas, alta de base).
5. La protección de `main` (423). Hoy `/confirmar`, `publicar-retenida` y las colecciones sin
   cabecera **no** la comprueban: el catálogo debe cerrar ese hueco.
6. `GET /ramas/{r}/cambios`: un puntero cambiado hace al activo "modificado" con `datos: true`;
   `enBase: datos` cuando `main` también lo movió.
7. La huella de la propuesta tiene que incluir el estado de los punteros de su alcance, para que
   reconstruir en la rama invalide las aprobaciones.
8. La tabla D5 por activo, aplicada **atómicamente junto con** el merge de git. Hoy "con receta"
   se decide solo por la ausencia de `escrito_por`.
9. Tras fusionar: reconstruir en `main` lo que tenía receta y traer `main` a la rama.
10. La recogida: lo vivo es la unión de lo que nombra cada rama viva (solo lo propio de cada rama),
    con 24 h de gracia; la recogida mueve el puntero de `main` al expirar.
11. Los Jobs en una rama leen el catálogo resuelto (sustituye a `ore overlay`).
12. La historia por puntero (versiones, quién, operación, filas), mejorada: hoy `/historia` no ve
    las versiones de `main` posteriores al punto de salida de la rama.
13. La auditoría persona frente a máquina, sin entradas vacías, con el motivo de cada movimiento.
14. El ciclo de vida de las ramas (crear hereda todo; borrar libera lo reclamado; las derivadas
    `alcance/*`).
15. `resultados/`: la recogida lo reclama pero `manda` no lo superpone. Decidir si entra.

### 11.6 Lo que el inventario destapó (fallos de hoy, independientes de este ADR)

- **Posible borrado indebido:** `retirar_informes_de_nadie` (la pasada de copia) borra punteros
  cuya `vista` no sea un dataset o colección **mantenidos**. Las colecciones **escritas** llevan
  `vista` y no son mantenidas: la copia podría borrarlas. Sin probar.
- **El editor puede escribir `datasets/**` a mano** (`PUT /arbol/...`): con el catálogo, esa ruta
  debe cerrarse.
- **`main` protegida no se comprueba** en `/confirmar`, `publicar-retenida` y colecciones sin
  cabecera de rama.
- **`transactions/commit` con fallo a mitad** deja `metadata.json` huérfanos hasta la recogida.
- **El mantenimiento empuja sin reintento:** si alguien empujó antes, falla el Job entero.

## 12. E1a·1: el banco del catálogo (2026-10-28, victor)

**Montaje:**
- Postgres 16 normal (la imagen de `idp-db`), `synchronous_commit = on`, `shared_buffers` 256 MB,
  disco `pd-balanced`, en el **nodo spot de sistema compartido** con el resto de la malla:
  0,5 CPU reservada y límite de 2.
- Esquema borrador de §5: `refs`, `estado` (`fillfactor` 70), `commits`, `commit_activos`,
  `outbox`. Una función `commit_punteros` hace en **una llamada** el *compare-and-set* de N activos
  (en orden, para no bloquearse en cruz), la herencia de `main` en una rama, la historia, la outbox
  y el `NOTIFY`, con idempotencia por clave.
- 10.050 punteros (10.000 en `main` y 50 propios en 5 ramas).
- Cliente en `jobs-s` (Python, psycopg 3, 16 procesos con conexión persistente y sentencias
  preparadas). Cada operación hace lo que ore-serve: leer el puntero vigente y commit.
  - Mezcla: 90 % de una tabla (20 % en ramas), 5 % multitabla (5 tablas) y 5 % sobre 10 tablas
    calientes que chocan.
- Un consumidor con `LISTEN` mide el tiempo desde la llamada al commit hasta recibir el evento.
- Latencias medidas desde la hora programada de cada operación, para que una cola no las esconda.

**Resultados (30 s por nivel):**

| Commits/s pedidos | Caudal real | Commit p50 / p99 | Evento p50 / p99 | Choques resueltos |
|---|---|---|---|---|
| 100 | 100 | **2,6 / 4,9 ms** | **2,7 / 5,0 ms** | 0 |
| 500 | 500 | **2,4 / 10,8 ms** | **2,5 / 11,0 ms** | 8 |
| 1.000 | ~910 (satura) | 15,4 / 27,3 ms | 15,5 / 27,4 ms | 165 |
| 2.500 | **983** (techo) | 14,4 / 24,5 ms | — | 398 |
| 2.500, **sin `NOTIFY`** | **1.666** (techo) | 4,8 / 32,9 ms | — | 302 |

Fallos: 0 en todos los niveles. Todos los choques en la misma tabla se resolvieron con un
reintento. Postgres a ~1,35 núcleos en el techo y el cliente a 0,6–0,7, así que el cuello es el
Postgres.

**Lo que decide:**
- **El diseño funciona.** Frente a hoy (2.600–3.600 ms y ~0,3 commits/s), el commit cuesta
  **2,5 ms** y el techo es de **~1.000–1.700 commits/s** en una fracción de un nodo compartido:
  unas 1.000 veces menos latencia y 3.000–5.500 veces más caudal.
- **La vara se cumple hasta ~500 commits/s** (p99 ≈ 11 ms). Por encima, el Postgres satura y la
  latencia sube a 15–25 ms.
- **`NOTIFY` por commit cuesta casi la mitad del caudal** (983 frente a 1.666/s): Postgres
  serializa las notificaciones con un cerrojo global al confirmar. A escala, **el evento no debe ser
  un `NOTIFY` por commit**: la outbox se queda (es la verdad del evento, en la misma transacción) y
  el aviso sale de la **decodificación lógica** del WAL o de un único avisador que lee la outbox
  (E1a·1b).
- **El *compare-and-set* por tabla funciona bajo choque real** y quita el 409 entre tablas
  distintas que hoy da git.
- **Para cruzar los 1.000/s con p99 ≤ 10 ms** hace falta medir el Postgres con CPU propia (no un
  nodo spot compartido) y el aviso sin `NOTIFY` (E1a·1b). La carga real de hoy (~8 commits de datos
  al día en victor) está muy por debajo.

**Aviso de método:** la primera tanda dio un techo falso porque la clave de idempotencia del
generador se repetía entre ejecuciones y muchas llamadas devolvían el commit ya existente sin
trabajo. La tabla de arriba es la repetición limpia, con clave única por ejecución y el caudal
medido con el tiempo real transcurrido.

## 13. E1a·1b: CPU propia y el evento por decodificación lógica (2026-10-28, victor)

**Qué cambia frente a E1a·1:**
- El Postgres corre en un nodo **dedicado y no spot** (`pesado-driver`, n2-highmem-4), con 3 CPU
  reservadas.
- `wal_level = logical` y una publicación solo de la `outbox`. El commit ya no hace `NOTIFY`: el
  evento sale del **flujo de replicación lógica** (`pgoutput`), que Postgres emite al confirmar,
  en orden y sin ningún cerrojo añadido al commit. Un consumidor se suscribe con un slot y mide el
  retraso de cada fila de la outbox.
- El mismo esquema, la misma función de commit y la misma mezcla de carga. Cliente con 24 procesos.

**Resultados (30 s por nivel):**

| Aviso | Pedidos/s | Caudal real | Commit p50 / p99 | Evento p50 / p99 | Postgres |
|---|---|---|---|---|---|
| lógico | 500 | 487 | **1,1 / 2,2 ms** | **1,2 / 2,3 ms** | ~0,27 CPU |
| lógico | 1.000 | 974 | **1,1 / 3,3 ms** | **1,1 / 3,0 ms** | ~0,43 CPU |
| lógico | 2.000 | 1.948 | 1,5 / 12,0 ms | 1,4 / 43 ms | ~0,8 CPU |
| lógico | 3.000 | 2.860 | 2,9 / 12,8 ms | 5,5 / 59 ms | ~1,06 CPU |
| `NOTIFY` | 1.000 | 974 | 1,1 / 5,8 ms | 1,1 / 4,3 ms | ~0,68 CPU |
| `NOTIFY` | 3.000 | 2.489 | 6,5 / 15,3 ms | 6,6 / 25 ms | — |

0 fallos; todos los choques en la misma tabla se resolvieron con un reintento.

**Lo que decide:**
- **La vara se cumple entera y con margen:** 1.000 commits/s con el commit en **3,3 ms p99** y el
  evento en **3,0 ms p99**, usando menos de medio núcleo.
- **El evento sale de la decodificación lógica**, no de `NOTIFY`: no frena el commit y entrega en
  orden y sin huecos (el problema clásico de leer una outbox por id no existe aquí). La outbox
  sigue siendo la verdad del evento, dentro de la transacción.
- **Coste:** ~0,43 CPU por cada 1.000 commits/s con aviso lógico (~0,68 con `NOTIFY`).
- **El techo del Postgres no se alcanzó.** A 3.000/s usaba ~1 de sus 3 núcleos; el límite lo puso
  el generador en Python (1,4 núcleos de un nodo de 2). A 2.000–3.000/s, la cola del evento (p99
  43–59 ms) es también del consumidor en Python, que procesa cada fila en un solo hilo: un
  consumidor nativo (Rust, como el resto de ORE) es parte del producto, no del banco.
- **Frente a hoy** (2.600–3.600 ms y ~0,3 commits/s): unas 1.000 veces menos latencia y casi
  10.000 veces más caudal, con medio núcleo.

**Abierto para producción:**
- El nodo: E1a·1 en el nodo spot compartido coincidió con una reparación de ese nodo (§14); el
  catálogo va a un sitio propio y no spot, o con réplica en caliente.
- Un slot de replicación retiene WAL mientras su consumidor no confirme: el consumidor confirma
  cada segundo, y hace falta vigilar el retraso del slot (y un tope de WAL retenido).

## 14. Incidencia durante E1a·1 (2026-10-28)

- 13:43–13:56 (Madrid): el banco de E1a·1 escribió a fondo (fsync en cada commit) en un disco
  `pd-balanced` montado en el **nodo spot de sistema**.
- 13:53: el detector de problemas del nodo marcó `ReadOnlyFileSystemDetected`.
- 13:59: GKE dio el nodo por averiado y lo **recreó** (`repair.recreateInstance`). Durante ~5–8
  minutos, la malla de sistema (Kueue, identidad, servicios de las celdas) se reinició.
- Kueue tardó en volver, y mientras su webhook no respondía, **los ReplicaSets de las celdas no
  pudieron crear pods**. Se recuperaron solos al volver Kueue (victor y demo, todos `1/1`).
- **No se puede descartar que la carga del banco lo provocara**, aunque el disco era un volumen
  aparte del de arranque. Desde entonces, los bancos van a nodos dedicados.
- **Fallo previo destapado:** el ReplicaSet de victor acumula 245 fallos de creación en 2,5 días
  por el webhook de Kueue sin endpoints. Cada vez que Kueue cae, las celdas no pueden recrear pods.
  Es un riesgo de la malla, aparte de este ADR.

## 15. E1a·2: las ramas en el catálogo (2026-10-28)

**Qué se probó:** que la capa de datos de las ramas (los punteros por rama) se comporta en el
catálogo **exactamente igual** que hoy sobre git. Los datos (Parquet en GCS) y las definiciones (en
git) no se movieron; solo los punteros.

**El modelo** (esquema y funciones en Postgres; prototipo, no producción):
- `refs(nombre, base, punto_salida, protegida)`: el punto de salida es un commit del catálogo y se
  mueve al traer `main`.
- `estado`: en `main`, una fila por tabla viva; en una rama, **solo lo propio** (lo cambiado o
  creado, y una lápida por lo borrado). Lo demás se lee de `main` al día.
- `historia`: cada movimiento de un puntero en cualquier ref.
- Funciones: `efectivo` (la regla `manda`), `vista`, `escribir` (*compare-and-set* contra el puntero
  efectivo, con cerrojo por activo para cubrir el nacimiento), `crear_rama`, `borrar_rama`, `traer`,
  `fusionar` (tres vías, todo o nada), `vivos` (recogida), `cambios`, `huella`, `historia_de`.

**P1–P8: 33 de 33 casos pasan** (Postgres 16 en el pool dedicado `jobs-s`):

| Prueba | Qué comprueba |
|---|---|
| P1 | Las siete combinaciones de `manda`: la rama cambió, creó o borró; no tocó (manda `main` **de hoy**); `main` creó o retiró después de la salida |
| P2 | Escribir sobre lo heredado lo hace propio sin tocar `main`; 409 si otro movió el puntero (en la rama o en `main`); idempotencia |
| P3 | Traer `main` mueve el punto de salida; lo fusionado y lo igual a `main` deja de ser propio |
| P4 | La tabla D5: gana `main`, gana la rama (también un borrado), reconstruir con receta, conflicto con "qué se pierde"; **sin elegir no se aplica nada** |
| P5 | Recogida: vive `main` de hoy y lo propio de cada rama; lo heredado no alarga lo viejo; una rama borrada deja de reclamar |
| P6 | "Modificado con datos", `enBase: datos` y la huella que cambia al reconstruir en la rama |
| P7 | `main` protegida: 423 sin propuesta (cierra el hueco de hoy en `/confirmar` y colecciones) |
| P8 | La historia desde una rama, **incluidas las versiones de `main` posteriores a la salida** (hoy no se ven) |

**El examen con victor real:**
- Extracción **de solo lectura** en el pod de ore-serve: `main` (157 punteros) y, por cada una de las
  18 ramas, sus punteros propios (diff contra su punto de salida: 24 creados y 25 cambiados) y el
  resultado de **`ore overlay`**, el código real de hoy.
- Cargado en el catálogo en 97 ms. Para cada rama y cada tabla, el puntero efectivo del catálogo
  frente al de `ore overlay`: **0 diferencias en 2.850 combinaciones**, resueltas en 1,9 ms por rama
  (todas sus tablas).
- **Control de que el examen puede fallar:** sin cargar los punteros propios salen 47 diferencias,
  justo en las tablas de las ramas.

**Números de paso:** leer el puntero efectivo en una rama, **0,34 ms** (hoy, ~6 procesos de git por
montaje del árbol); crear una rama, **1,7 ms** (no copia nada).

**Lo que deja abierto:**
- El examen compara la **versión** de cada tabla (`metadata_location`), no el resto del estado vivo
  (frescura, estadísticas): esos campos viajan con la fila y se comprueban en la migración (E1a·4).
- Los datos reales de victor no tienen ramas que **borren** tablas; ese caso lo cubre P1 y P4.
- No se reprodujeron en orden los 266 commits históricos (opcional del plan).
- El criterio "con receta" se conserva tal cual (sin `escrito_por`), y la fusión coordinada con el
  merge de git es E1a·3.

## 16. E1a·3: fusionar una propuesta entre git y el catálogo (2026-10-28)

**El problema.** Hoy fusionar es **un** merge de Forgejo: definiciones (`packages/`) y punteros
(`datasets/**`) entran juntos o no entran. Con el catálogo, la fusión toca dos sistemas sin
transacción común, y entre los dos pasos puede caerse ore-serve, colarse una escritura en una tabla
del alcance, repetirse la petición o mirar un lector.

**El protocolo: git decide; el catálogo prepara antes y aplica después.**

1. **Preparar** (catálogo, una transacción): la fusión a tres vías de E1a·2 se **ensaya** y se
   deshace. Si queda algo sin elegir, no se sigue. Si no, se guarda la **intención** (propuesta,
   rama, alcance, elecciones y el **plan**: qué puntero entra en `main` y cuál se queda) y se ponen
   **cerrojos** sobre las tablas del alcance, en `main` y en la rama. Quien escriba en ellas recibe
   un 429 reintentable; el resto de tablas no se entera.
2. **Git**: el merge de siempre, con `Fusion: <n>` en el mensaje. Es el único paso que puede decir
   que no (conflicto de definiciones, protección, no compila); si lo dice, la intención se cancela y
   los cerrojos se sueltan.
3. **Aplicar** (catálogo, una transacción ligada al commit de git, idempotente por propuesta): se
   aplica **el plan**, no se recalcula; lo que entra es lo que se revisó y lo que git fusionó. Historia,
   eventos y un solo commit de fusión (clave `fusion:<n>`). Se sueltan los cerrojos.

**El reconciliador** (en el banco, cada 0,1 s; en producción, en cada réplica de ore-serve):
- intención preparada y la fusión **está** en git → aplicar;
- preparada, **caducada** y sin fusión en git → cancelar;
- fusión en git sin intención hecha → **fusión tardía** (abajo).

**Por qué el catálogo no va primero:** aplicar punteros emite eventos que consumen otros (Vearch,
Health). Si git dijera que no después, habría que deshacer algo ya visto. Git va en medio porque es
quien decide: *git guarda lo que se decide*.

**Decisiones tomadas al construirlo:**
- **El cerrojo no caduca por tiempo; caduca la intención.** La resuelve el reconciliador mirando
  git, así que nunca se suelta un cerrojo de una fusión que git ya hizo.
- **El plan se fija al preparar.** Con el cerrojo puesto, `main` y la rama no pueden moverse; si
  aun así se movieran, `aplicar` se niega (error de protocolo) en vez de aplicar a ciegas. El
  control sin cerrojo lo demuestra.
- **Fusión tardía** (el único caso residual). La petición a la forja seguía en vuelo cuando la
  intención caducó y se canceló, y la forja la termina después. Se aplica el plan cancelado. Lo que
  `main` movió entretanto se queda (gana lo más reciente) y sale como **pendiente**, y lo pendiente
  sigue siendo propio de la rama: no se pierde nada y queda dicho.

**El banco** (Postgres 16 y cliente en `jobs-s`; la forja, un repo git desnudo: se prueba el merge
de git, no la API de Forgejo):
- **Oráculo:** cada escenario se fusiona también **como hoy**, con los punteros en el árbol, un solo
  merge de git y los conflictos resueltos por la regla de hoy (receta → `main`, elección → el lado
  elegido, sin elegir → no se fusiona). El `main` final del catálogo y de git (punteros y árbol de
  definiciones) tiene que ser idéntico.
- **Escenarios:** uno a mano con un caso de cada (solo la rama, solo `main`, conflicto elegido
  rama y `main`, receta, borrar, crear, sin tocar) y 120 al azar; 106 se fusionan, 8 los rechaza
  git y 7 quedan sin elegir. Los mismos desenlaces que hoy.
- **Fallos inyectados en cada costura:** caída tras preparar (caduca, se cancela y la persona
  reintenta), tras git, a mitad de aplicar (*rollback*), tras aplicar (respuesta perdida y
  reintento), dos personas fusionando a la vez, un escritor en `main` y otro en la rama sobre el
  alcance durante la fusión (más otro fuera del alcance), y la fusión tardía con una escritura entre
  la cancelación y el merge tardío.
- Por corrida se comprueba además: sin cerrojos ni intenciones colgadas, exactamente una fusión en
  git y una en el catálogo (o ninguna), un solo commit de fusión y el reconciliador sin errores.

**Resultados:** **879 corridas, 0 diferencias**, y 8/8 casos fijos:
- dos propuestas a la vez con alcances distintos entran las dos;
- con alcances que se pisan, la segunda recibe 429;
- escribir en `main` o en la rama sobre el alcance en curso da 429, y fuera del alcance entra;
- la misma propuesta repetida mientras está en curso no prepara otra;
- cancelar suelta los cerrojos.

Las 106 fusiones tardías convergieron; en 80 se coló una escritura y quedó como pendiente.
**Controles:** sin cerrojo fallan las corridas con escritor (lo para la guarda de `aplicar`); sin
reconciliador no convergen las cuatro caídas que lo necesitan.

| Medida (p50 / p99) | Banco |
|---|---|
| Preparar (incluye mirar git y conectar) | 35 / 68 ms |
| Git (merge + push en el banco) | 80 / 120 ms |
| Aplicar | 8 / 35 ms |
| **Cerrojo puesto** (preparar → aplicar) | 88 / 148 ms |
| Recuperar tras caída después de git o a mitad de aplicar | 25 / 103 ms (≈ un tic) |
| Recuperar tras caída después de preparar | 1,06 s (= plazo de 1 s + un tic) |
| Fusión tardía rematada | 78 / 111 ms |
| Escritor sobre el alcance, espera hasta entrar | 201 / 247 ms (había una pausa de 0,3 s) |

**Lo que eso significa con Forgejo:** el merge real cuesta ~1,5 s (E1·0), así que el cerrojo
sobre las tablas del alcance durará ~1,6 s por fusión, y solo al fusionar. Un lector ve definiciones
nuevas con punteros viejos lo que dura aplicar (milisegundos), o un tic del reconciliador si
ore-serve cae justo ahí. Es la misma clase de ventana que la decisión (b).

**Lo que queda abierto:**
- **El plazo en producción** tiene que superar el *timeout* de la llamada a Forgejo (p. ej. 30 s).
  Una caída justo tras preparar deja las tablas del alcance cerradas hasta entonces: es el precio de
  no soltar nunca un cerrojo de una fusión que git pudo haber hecho.
- **La rama borrada tras fusionar** (Forgejo la borra): `borrar_rama` en el catálogo después de
  aplicar, salvo que tenga pendientes. Se cablea en E1a·4.
- **El reconciliador en ore-serve:** leer el log de `main` desde lo último visto, no entero.
- **Retirar, renombrar y migrar** son operaciones de definición que tocan punteros y usarán el
  mismo patrón (intención, git, aplicar).
- Probado contra un git desnudo, no contra Forgejo; la latencia real entra en la cuenta de arriba.

## 17. E1a·4: la sombra en victor (diseño, 2026-10-28)

> **Recortado el 2026-10-28 (§17.8).** El seguidor permanente y la semana de sombra (·4d) no se
> hacen: validaban la copia desde git, un camino que muere en el corte, y la fidelidad del modelo
> ya la probó ·4a con la historia entera. El andamio queda como **la mudanza** del día del corte.
> Lo que sigue en esta sección hasta §17.5 es el diseño original; vale lo que §17.8 no cambia.

**Qué es:** el catálogo vive de verdad junto a victor y recibe **cada** movimiento de puntero,
pero **git sigue siendo la verdad** y nadie lee del catálogo salvo un comparador. Es la primera
iteración fuera del banco: si la sombra falla, el producto no se entera.

**Vara:** cero diferencias entre git y el catálogo durante 7 días de tráfico real y provocado, con
la fila del puntero entera (no solo `metadata_location`).

### 17.1 Un cambio sobre lo hablado: seguir a git, no escribir dos veces

La idea era la doble escritura en las tres primitivas (§11.2). El previo encontró dónde corren:
`ore-serve`, sí, pero también **los Jobs** (`48-la-copia`, `53-el-mantenimiento`,
`copiar-worker.sh`, `cerrar-ejecucion.py`), cada uno en su pod con un clon propio que empuja a la
forja. Escribir dos veces obligaría a dar la credencial del Postgres a cada Job y a tocar 19
caminos **antes** del corte.

**El diseño: un seguidor.** Una pieza mira la forja, ve cada commit y lo replica en el catálogo.

| | Doble escritura | Seguidor de git |
|---|---|---|
| Piezas tocadas | ore-serve, 4 Jobs y `ore-cli` | una nueva; ningún escritor cambia |
| Cobertura | la que se cablee | toda, por construcción (también el editor y los Jobs) |
| Riesgo en la escritura de hoy | sí | ninguno |
| Historia | desde el día del despliegue | **la entera**: replica los 266 commits de datos (lo que E1a·2 dejó sin hacer) |
| Prueba el catálogo como escritor | sí | no: eso ya lo probaron los bancos E1a·1–3 y es el cambio de E1a·5 |

**Consecuencia para E1a·5:** los escritores pasan uno a uno a hablar con el catálogo **a través de
ore-serve** (nadie más tiene la credencial), y el documentador de §11.3 lleva a git lo que haga
falta. El seguidor ignora los commits del documentador, así que los dos conviven durante el paso.

### 17.2 Lo que el previo dejó medido (victor, solo lectura, 2026-10-28)

- **Cuota:** 6 de 32 CPU (`CPUS_ALL_REGIONS`) y 0 de 24 en E2: un nodo pequeño cabe.
- **Mirar la forja:** `ls-remote`, 60–90 ms con 44 refs. Clon desnudo, 3,6 s y 2,2 MB.
- **Historia:** 787 commits en `main` (14 de fusión), 266 de datos desde el 2026-09-21. El
  `diff-tree` de los 266 cuesta 1,6 s, así que **la carga inicial con la historia entera son
  segundos**.
- **Punteros:** 157 en `main`, máximo 1,96 KB y 182 KB en total, con **37 claves distintas** y
  heterogéneas (`estado`, `snapshot`, `columnas`, `items`, `promesas`…). Por eso la fila guarda
  el JSON entero en `jsonb`, más columnas calientes (`metadata_location`, `escrito_por`).
- **Ramas:** 19, de ellas 3 derivadas `alcance/*`.
- **Credencial y red:** el patrón ya existe. Un contenedor de inicio trae `t-victor-forja-token`
  de Secret Manager (como `ore-serve`), y un pod con `ore.dev/rol: control` y la cuenta `ore-serve`
  ya sale a la forja con las políticas de hoy.
- **Credencial de Postgres por inquilino:** también existe. `aprovisionar-inquilino.sh` crea un
  papel por inquilino en `idp-db` y guarda su URL como secreto; se copia.
- **Cliente:** `postgres` 0.19 ya está en el `Cargo.lock` (`ore-iam`, `ore-cofre`), sin nada que
  descargar. No enlaza en Windows: se compila y se prueba en Docker.
- **Disco C:** 53 GB libres (el mínimo para compilar es 40).

### 17.3 Las piezas

1. **El nodo.** Un pool `catalogo` de e2-medium **no spot** con un nodo fijo, *taint*
   `ore.dev/catalogo`, en `malla/76-el-pool-del-catalogo.sh` (en seco por defecto, como 73 y 74).
   Cuota 6 → 8 de 32. E1a·1b midió 1.000 commits/s con 0,43 CPU, así que sobra.
2. **El Postgres.** `malla/76-el-catalogo.yaml`:
   - namespace `catalogo` y StatefulSet de `postgres:16` (la imagen de `idp-db`) con
     `wal_level=logical`;
   - PVC de 10 Gi con la clase `retiene`;
   - `NetworkPolicy` que solo deja entrar desde `t-victor` (`rol: control`);
   - copias con `pg_dump` a GCS, como en 62.

   **Una instancia compartida, un esquema y un papel por inquilino** (`t_victor`). Esto cierra la
   pregunta 6 de §9.
3. **El esquema v1:** lo de E1a·2 y E1a·3 unido.
   - `estado` lleva el puntero entero en `jsonb`;
   - `commits` lleva `git_sha` (único: replicar es idempotente), autor y fecha de git;
   - la `outbox` y su publicación.

   Versionado en el binario (`esquema_version`) y aplicado al arrancar.
4. **`ore-catalogo`: una librería que se queda y un andamio que se va.** Es un crate nuevo, como
   `ore-iam`: usa `postgres` 0.19, y `ore-core` por `punteros::manda`. Lleva un binario fino
   encima, que viaja en la imagen `serve` (ya lleva `git`): **no hay imagen nueva**, solo un
   Deployment propio.

   | Módulo | Qué hace | En E1a·4 | Después del corte |
   |---|---|---|---|
   | `esquema` | El esquema versionado (`esquema_version`), aplicado al arrancar | sí | se queda |
   | `almacen` | Las transacciones del catálogo: aplicar un commit (CAS, historia y outbox), leer el puntero efectivo, `vista`, ramas | lo que necesita el replicar | se queda; ore-serve escribe con él en E1a·5 |
   | `cargar` | **El andamio:** sigue a git y replica en el catálogo | sí | **se apaga** al cortar cada inquilino |
   | `auditar` | Compara el catálogo con una referencia: hoy, git; tras el corte, GCS y su propia coherencia (§18) | contra git | se queda, como guardián |
   | `salud` | `GET /salud`: retraso, diferencias, última auditoría, eventos | sí | se queda |
   | `reconciliar`, `rele` | Las fusiones a medias (E1a·3) y la outbox hacia sus consumidores | no | E1a·5 |

   El binario tiene dos modos: **`seguir`** (cargar + auditar + salud, el de la sombra) y
   **`guardar`** (auditar + reconciliar + relé + salud, el de después del corte). El andamio es un
   solo módulo, así que quitarlo es borrar un fichero y un modo.

   **`cargar`, cómo funciona:**
   - **Espejo y vigilancia:** un espejo desnudo propio y `ls-remote` cada segundo (sin *webhook*
     en la sombra).
   - **`main`:** por primer padre desde el último sha seguido. Cada commit de git se convierte en
     un commit del catálogo con sus punteros enteros, en orden e idempotente por `git_sha`.
   - **La carga inicial** es el mismo camino desde el primer commit con datos, y tras cada commit
     replicado se comprueba que el `main` del catálogo es igual al árbol de ese commit.
   - **Ramas:** lo propio se recalcula (diff contra el *merge-base* con `main`) y se sustituye en
     una transacción. El punto de salida es el commit del catálogo de ese *merge-base*. Las ramas
     borradas en git se borran.
   - **El evento real:** la outbox se llena en la misma transacción. Un consumidor lógico de prueba
     mide lo que tarda el evento desde que la forja tiene el commit.

   **`auditar` en la sombra** (cada 5 min y tras cada pasada): por cada rama, `vista(r)` frente a
   `manda` sobre los ficheros de git, **campo a campo** del JSON canónico.
5. **En `t-victor`:**
   - el Deployment `ore-catalogo` (rol `control`, cuenta `ore-serve`) con un contenedor de inicio
     que trae el testigo de la forja y el de su papel en el catálogo;
   - la `NetworkPolicy` `salida-al-catalogo`.

   Mientras dure la sombra se declara en la malla solo para victor; se generaliza al render del
   inquilino en E1a·5.
6. **El editor** (fallo de §11.6, decidido cerrarlo aquí): `PUT /arbol/...` contesta 422 en
   `datasets/**` y `copias/**`, porque los punteros no se editan a mano. Es un cambio pequeño en
   ore-serve.

### 17.4 El orden, a tiro fijo

| Paso | Qué | Dónde | Hecho cuando |
|---|---|---|---|
| **·4a** | **Corrección sin desplegar nada.** Un banco en `jobs-s` con Postgres, `ore-catalogo` en modo `seguir` (compilado en Docker) y una copia del árbol de victor (el desnudo de 2,2 MB, sacado del pod de ore-serve). Carga inicial de los 266 commits con la comprobación paso a paso, las 19 ramas y el comparador. Además: replicar dos veces, matarlo a mitad y reanudar, borrar una rama, una `alcance/*` | banco | 0 diferencias en cada paso de la historia y en todas las ramas; reanudar da lo mismo |
| **·4b** | **El nodo y el Postgres.** El script del pool (en seco y luego de verdad) y el manifiesto; la copia a GCS y una restauración de prueba. Después, Kueue y los Deployments de las celdas en `1/1` | malla | Postgres arriba en un nodo no spot, con copia restaurada |
| ~~·4c~~ | *(Sustituido por §17.8.)* **El seguidor en victor.** Primer empuje: el binario, el editor cerrado y el manifiesto del Postgres. Con la imagen ya desplegada (`ore.dev/commit`), segundo empuje: el Deployment y la política (binario antes que malla) | victor | carga inicial en victor con 0 diferencias, retraso < 2 s, comparador en verde |
| ~~·4d~~ | *(Fuera por el recorte, §17.8.)* **El periodo.** 7 días de tráfico real más uno provocado: escrituras `/v1` en un dataset de banco, copias, y crear rama, escribir, proponer y fusionar con alcance, retirar y renombrar en ramas desechables | victor | 0 diferencias en 7 días; retraso y evento p99 medidos y anotados aquí |

*(Antes del recorte.)* **Dos empujes en total**, con las guardas del CI pasadas en local antes del primero y las pruebas
solo de `-p ore-catalogo` (y de lo tocado en ore-serve) dentro de Docker.

**Por despejar al empezar** (minutos):
- que Flux `malla` pueda crear objetos en `t-victor` sin que `inquilino-victor` los pode;
- si la cuenta `ore-serve` lee un secreto nuevo o hace falta el aprovisionador;
- que haya e2-medium no spot en `europe-west1-b` (se ve al crear el pool).

### 17.5 Fuera de E1a·4

- Ningún lector cambia y ningún escritor cambia (salvo el editor cerrado): la escritura sigue
  costando segundos hasta E1a·5.
- La sombra en la página de Health de ORE: deuda, porque basta con `GET /salud`.
- La alta disponibilidad del Postgres se decide antes del corte, no ahora.

### 17.6 ·4a: el andamio contra el árbol de victor (2026-10-28)

**Hecho.** El crate `ore-catalogo` (librería y binario, modo `seguir` y `auditar`) contra una copia
desnuda del árbol real de victor, con Postgres 16 en `jobs-s`. Nada se desplegó.

- **Carga inicial con la historia entera:** 773 commits de primer padre de `main` (266 con
  punteros), 4.100 movimientos en la historia y en la outbox, y las 18 ramas, en **7–9 s** con
  la verificación en cada paso encendida. **0 pasos mal en 773**: tras cada commit, `main` del
  catálogo era el árbol de ese commit.
- **Auditoría:** 3.007 combinaciones (ref, puntero) comparadas con `manda` sobre git, campo a campo
  del puntero entero: **0 diferencias**.
- **Repetir** no hace nada (0 commits, 0 ramas).
- **Caída en un punto conocido** (tras 300 commits) y **dos `kill -9` a mitad** (con 155 y 469
  commits replicados): al reanudar, 0 diferencias y **el mismo estado byte a byte** que la carga de
  un tirón (estado, historia, eventos y puntos de salida).
- **La forja se mueve:** `main` cambia un puntero y crea otro, una rama cambia uno y borra otro (la
  lápida), otra rama se borra y nace una nueva. La pasada incremental tarda 0,8–0,9 s y da 0
  diferencias.
- **Control:** con un campo cambiado, un puntero quitado y una lápida rota a mano, la auditoría
  encuentra las tres cosas (39 diferencias, porque las ramas heredan las dos de `main`).
- **Tamaño:** 10 MB el esquema de victor, con la historia.
- **Código:** 10 pruebas unitarias del parseo de git, la regla y los argumentos; `clippy -D
  warnings` y `fmt` limpios.

**Lo que esto despeja:** el modelo (punteros heterogéneos en `jsonb`, ramas como lo propio más
lápidas, punto de salida por la *merge-base*) representa victor entero, su historia incluida.

**Lo que deja para ·4c:**
- Cuando `main` avanza, se recalculan las 18 ramas (unos 250 procesos de git, la mayor parte de
  esos 0,9 s). Mejora: recalcular solo las ramas cuya *merge-base* cambió.
- `GET /salud` por HTTP: hoy sale una línea JSON por pasada.
- Las ramas se replican como foto de lo propio, sin la historia de sus commits.
- Entrar en la imagen `serve` y en las huellas del CI.

### 17.7 ·4b: el nodo y el Postgres (2026-10-28)

**Hecho.** Lo que hay ahora en el clúster:

- **El nodo:** el pool `catalogo`, un e2-medium **no spot** con un nodo fijo y sin autoescalado,
  creado con `malla/76-el-pool-del-catalogo.sh` (en seco y luego `APLICAR=1`). Cuota de 6 a 8 de
  32. **No entra en el horario**: el catálogo no se duerme (§7), y de noche sigue encendido
  (~24 $/mes). El mismo script crea el namespace, el Secret `catalogo-db` (clave de `/dev/urandom`,
  sin imprimirse) y el enlace de Workload Identity de `catalogo/copias` con `ore-copias`.
- **La base:** `malla/76-el-catalogo.yaml`, con el StatefulSet `catalogo-db` (Postgres 16.15, la
  imagen de `idp-db`), PVC de 10 Gi `retiene`, `wal_level=logical`, `max_slot_wal_keep_size=2GB`
  (el tope de §18) y `synchronous_commit=on`. Una `NetworkPolicy` solo deja entrar al plano de
  control de `t-victor` y al propio namespace. Está en la lista de la malla; se aplicó a mano y Flux
  lo adopta al empujar (`prune: false`).
- **La copia:** el CronJob `copia-del-catalogo`, a las 12:00, como la del IdP: `pg_dump -Fc`,
  comprobado con `pg_restore --list`, a `gs://…-copias/catalogo/`.
- **La restauración probada:** `malla/77-restaurar-el-catalogo.sh` baja el último volcado, lo
  restaura en una base nueva del mismo servidor y compara cada esquema de inquilino con la base viva.

**La prueba, con datos reales:**
1. Victor entero, cargado por `ore-catalogo` en un esquema de prueba desde `jobs-s`: 773 commits, 18
   ramas, 0 diferencias y 9,1 s.
2. Copia a GCS: 0,9 MB.
3. Restauración: **el esquema restaurado es idéntico al vivo** (estado, historia, commits y ramas).
4. **Control:** con una fila cambiada en la viva, la restauración sale FALLA.

Después se borró el esquema de prueba: el catálogo queda vacío para ·4c. Kueue y las 16 piezas de
`t-victor` y `t-demo`, en `1/1` tras añadir el nodo.

**Lo que esto no es todavía:** sin réplica ni archivado continuo del WAL. Son requisito del corte
(§18), no de la sombra.

### 17.8 ·4c recortado: preparar el corte (plan, 2026-10-28)

**Por qué el recorte.** Tras el corte nadie copia de git: los escritores escriben en el catálogo.
Una semana de sombra solo validaría la copia desde git, que muere ese día, y no el camino nuevo,
que es donde está el riesgo (E1a·5). Lo que hace falta de E1a·4 es lo que el corte usa.

**El corte de victor, como queda (E1a·5):** parar las escrituras unos segundos; una pasada de
`cargar`, que es incremental y solo trae lo que falta desde la carga de ensayo; auditar; conmutar. El
andamio se usa ahí por última vez y se borra.

| Paso | Qué | Hecho cuando |
|---|---|---|
| **·4c·1** | **`ore-catalogo`:** conecta sin `CREATE SCHEMA` y la v1 deja de crear la publicación (las dos cosas, del alta). Se repite el banco de ·4a entero. (Recalcular solo las ramas cambiadas y `/salud` por HTTP pasan a E1a·5: los pide el modo `guardar`, no la mudanza) | Los 12 casos de ·4a pasan |
| **·4c·2** | **El alta del inquilino en el catálogo:** `malla/78-el-catalogo-del-inquilino.sh <org>`, en seco por defecto. Crea el papel `t_<org>` (clave de `/dev/urandom`), el esquema `t_<org>` suyo, la publicación `eventos_t_<org> FOR TABLES IN SCHEMA` y el Secret `catalogo` en `t-<org>` | Probado en el Postgres de verdad con dos inquilinos de prueba: cada papel solo ve su esquema |
| **·4c·3** | **El editor cerrado:** `PUT` y `DELETE /arbol/…` y `POST /arbol/commit` contestan 422 en `datasets/**` y `copias/**`. Antes, comprobar que la consola y el SDK no escriben punteros por ahí | Prueba en `ore-serve` (`-p ore-serve`, en Docker) y 0 usos legítimos |
| **·4c·4** | **La imagen y el CI:** `ore-catalogo` en `ci/compilar-binarios.sh` (estático), un `COPY` en la etapa `serve` y las huellas y sus pruebas. Las guardas del CI en local, solo de lo tocado | Guardas verdes en local y `main` mirado antes |
| **·4c·5** | **El único empuje (PR), con tu visto bueno:** el crate y el `Cargo.lock`, el CI y el Dockerfile, el editor, la malla 76/77/78 y su lista, y los ADR 0078/0079 con el README | Fusionado y la imagen desplegada en victor (`ore.dev/commit`) |
| **·4c·6** | **El alta y la carga de ensayo en victor:** `78 victor` de verdad y `malla/79-cargar-el-catalogo.sh victor`, un Job de una vez en `t-victor` (rol `control`, cuenta `ore-serve`, imagen `ore-serve:main`) que corre `cargar` con verificación de pasos y la auditoría | 0 diferencias en victor. Los datos se quedan: el corte solo trae lo que falte |

**Ya no hay:** Deployment permanente, `NetworkPolicy` del seguidor, segundo empuje ni ·4d.

**Estado (2026-10-28): ·4c·1 a ·4c·4 hechos en local, sin commit.**

- **·4c·1:** `ore-catalogo` conecta sin crear el esquema (si no existe, dice que se dé de alta con 78)
  y comprueba que el papel tiene acceso al suyo (si no, lo dice y no entra). La v1 ya no crea la
  publicación. Pruebas, `clippy` y `fmt` en verde.
- **·4c·2:** `78-el-catalogo-del-inquilino.sh`, probado en el Postgres de verdad con dos inquilinos
  de prueba, **10/10**:
  - con su papel (no el superusuario), `ore-catalogo` carga victor entero con 0 diferencias, y al
    repetir no hace nada;
  - un papel no lee ni escribe el esquema del otro, no crea esquemas ni publicaciones y no borra la
    suya;
  - `ore-catalogo` con el papel de A no entra en el esquema de B;
  - un esquema sin alta da error y no se crea.

  La publicación es `FOR TABLES IN SCHEMA … WITH (publish = 'insert')`: lleva todas las tablas del
  esquema, no solo la outbox, así que **el consumidor se queda con la outbox** (a E2). Para dar de
  baja un papel hay que quitarle antes el `CONNECT` a la base: queda para cuando haga falta una baja.
- **·4c·3:** el editor cerrado en sus tres puertas con `ruta_editable`; leer, la historia y las
  versiones de un puntero siguen abiertas. Ni la consola ni las pruebas del CI escriben punteros por
  `/arbol`. Las 343 pruebas de `ore-serve` en verde.
- **·4c·4:** `ore-catalogo` en `ci/compilar-binarios.sh` (y en la puerta de "cada servidor arranca"),
  el `COPY` en la etapa `serve` y la prueba de las huellas (`ore-catalogo` usa `ore-core`). Las tres
  huellas y sus pruebas en verde, y `fmt --all`.
- **Dos cosas que salieron por el camino:**
  - **`salida-al-catalogo`:** las celdas niegan toda salida por defecto, así que su plano de control
    no llegaba al catálogo. La política va en `76-el-catalogo.yaml` (solo victor, al render en E1a·5)
    y ya está aplicada.
  - **El Job de `79-cargar-el-catalogo.sh`** lleva `rol: control` para la red, pero el Service
    `ore-serve` elige por esa etiqueta. Una sonda que nunca da listo lo deja fuera del Service.
- **Para el empuje:** `main` va 21 commits por delante de la base de este árbol. El PR se monta sobre
  el `main` de ahora, solo con las rutas de este trabajo, y las guardas se repiten ahí.

## 18. Salud y fiabilidad tras el corte (requisito de E1a·5)

Tras el corte, el catálogo de un inquilino **no puede ir con retraso**, porque no es copia de nada:
cada escritura es una transacción suya. Los inquilinos nuevos nacen así y nunca pasan por el
andamio. Lo que sí puede fallar, y quién lo cubre:

| Riesgo | Cobertura |
|---|---|
| Perder el catálogo (los datos de GCS seguirían, pero no se sabría qué versión de cada tabla es la viva) | WAL archivado sin parar en GCS (recuperación a cualquier instante), volcado diario y una **restauración probada** periódica. Último recurso: reconstruir `main` desde el `metadata.json` más reciente de cada tabla en GCS (se pierden las ramas) |
| Que el Postgres se caiga | Nodo no spot y **réplica en caliente con conmutación**, decidida antes del corte. Mientras tanto, ore-serve contesta 503 reintentable, nunca un dato malo |
| Un puntero vivo que no resuelve en GCS (un bug, una escritura a medias) | `auditar` contra GCS: cada puntero vivo existe, historia y estado cuadran, y cada ref tiene su rama en git |
| Consumidores que se atrasan (Vearch, Health, el documentador) | El retraso de cada uno, medido y con alarma. **Un slot lógico parado retiene WAL hasta llenar el disco:** se vigila y se le pone tope (`max_slot_wal_keep_size`) |
| Operaciones a medias (fusionar, retirar, renombrar) | `reconciliar` (E1a·3) |

La parte de Postgres (copias, réplica, restaurar) es de la malla; el resto, del modo `guardar` de
`ore-catalogo`. **No se corta ningún inquilino (E1a·5) sin la réplica, el archivado del WAL y una
restauración probada.**

## 19. E1a·5: el corte (plan, 2026-10-28)

**Tras el corte**, en cada inquilino: escribir un puntero es una transacción del catálogo (CAS,
historia y evento); leerlo, listar una rama o pedir su historia sale de `vista(ref)` e `historia`;
las ramas son una fila en `refs` y su rama de git; git guarda solo lo que se decide, y el `Dataset`
escrito por máquina llega por el documentador. Los ficheros `datasets/**` y `copias/**` **salen de
git en un commit**: un lector olvidado falla en voz alta en vez de leer viejo.

**El camino: una sola puerta.** Todo lo que lee o escribe un puntero pasa por una interfaz,
`Punteros`, y nada más sabe dónde viven. Primero con la implementación de git, sin cambiar nada; luego
con la del catálogo, contra **la misma suite**. Una guarda en las pruebas (como la de las
dependencias) prohíbe tocar `datasets/`, `copias/` o `punteros::leer_en`/`todos_en` fuera de la
implementación de git: lo que se escriba mañana pasa por la puerta por fuerza.

| Paso | Qué | Hecho cuando |
|---|---|---|
| **·5a** | Inventario de cada sitio: [`0079-inventario-de-punteros.md`](0079-inventario-de-punteros.md) | **Hecho (2026-10-28)** |
| **·5b** | La interfaz y su implementación de git; todos los sitios de ore-serve y ore-cli pasan por ella; la guarda | Las pruebas de ore-serve y ore-cli, igual que hoy |
| **·5c** | La implementación sobre `ore-catalogo` | La misma suite, en verde con las dos |
| **·5d** | Los Jobs (48, copiador, 49, 53, `cerrar-ejecucion.py`) dejan su clon para los punteros y hablan con ore-serve | Sus pruebas, con las dos |
| **·5e** | `guardar`: relé de la outbox, documentador, reconciliar, auditar contra GCS, `/salud` | Banco en victor |
| **·5f** | §18 en la malla: réplica, WAL a GCS, restauración probada (en paralelo) | Restaurar a un instante, medido |
| **·5g** | El corte de victor: binario con el interruptor y, en otro empuje, la malla que lo enciende; 503 reintentable unos segundos, `cargar` incremental, auditar, conmutar, el commit que quita los punteros | 0 diferencias; escribir en ms |

**Lo que dejó ·5a** (detalle en el inventario):

- El corte toca **dos crates (ore-serve y ore-cli) y cinco Jobs**. El SDK (~30 sitios) y la consola
  (~16) van siempre por HTTP: no cambian si las rutas contestan igual.
- ore-serve: 58 sitios (R 37 · W 18 · H 10). ore-cli: 41. Buena parte de ore-serve es ore-cli por
  debajo (`/v1`, `/datasets`, `/colecciones`, preview y ejecutar vista corren `ore datasets|collections|ask`
  sobre un directorio): **`Punteros` vive en una crate que usan los dos.**
- Las operaciones que piden todos los sitios son nueve: `uno`, `todos`, `historia`/`en`,
  `escribir` (una o varias tablas, con CAS, autor, operación e idempotencia), `retirar`/`mover`,
  `reclamados`, `cambios(rama)`, las ramas (crear, borrar, traer, fusionar) y **`versión(ref)`**, la
  clave de las cachés que hoy es un commit.
- **`manda` está escrita cinco veces** y **la historia se reconstruye con `git log` en siete sitios**,
  que discrepan entre sí. Con la puerta queda una de cada.
- Tres ideas de qué carpetas son punteros (`datasets`; `datasets` + `copias`; `datasets` + `copias` +
  `resultados`). La puerta decide una, y con ella §11.5 punto 15.

**La decisión de ·5b:** cómo llega `Punteros` a ore-cli. Dentro de ore-serve, ore-cli recibe la
implementación por la que le llaman. En un Job, ore-cli no tiene la credencial del catálogo (solo
ore-serve, §17.1): su implementación es un cliente HTTP contra ore-serve.

### 19.1 ·5b: la puerta `Punteros` (diseño, 2026-10-28)

**Una restricción lo decide casi todo:** el binario `ore` no abre un socket. La guarda
`el_binario_que_se_distribuye_no_sabe_hablar_por_la_red` veta `postgres`, `reqwest`, `tokio` y TLS en
su cierre, y para el lago ya usa ese patrón: `ore` habla con `ore-store-<gcs>` por stdin y stdout.
Los punteros siguen el mismo camino.

**Las piezas:**

| Pieza | Dónde | Qué |
|---|---|---|
| El rasgo `Punteros` y sus tipos | `ore_core::punteros` | Lo único público del módulo. La ref va fijada al construirlo |
| `Ficheros` | `ore_core::punteros::ficheros` | Lo de hoy: el directorio que ya montó quien llama (superpuesto si es rama) y `git log` para la historia. Persiste quien publica, como hoy. **Las piezas de ficheros (`leer_en`, `todos_en`, `ruta_en`, `CARPETA`…) pasan a ser privadas:** fuera de aquí no compilan. Se queda después del corte para las pruebas y el uso local sin catálogo |
| `Proceso` | `ore-cli` | Habla el protocolo por líneas JSON con el binario que diga `ORE_PUNTEROS`, un hijo por ejecución de `ore`. Sin la variable, `ore` usa `Ficheros` |
| `Catalogo` | `ore-catalogo` (·5c) | Sobre `almacen`: `vista`, `historia`, una transacción por `escribir` |
| `ore-catalogo punteros` | binario (·5c y ·5d) | Sirve el protocolo por stdin: dentro del pod de ore-serve, con el Postgres (`CATALOGO_URL`); en un Job, contra `POST /punteros` de ore-serve. Un protocolo, tres transportes: en proceso (ore-serve), por proceso (`ore`) y por HTTP (Jobs) |

**La forma** (escrita en ·5b·1: `crates/ore-core/src/punteros/puerta.rs`; aquí, el resumen):

```rust
pub trait Punteros {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo>;          // Leido { puntero, de: Main | Rama }
    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo>;
    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo>; // puntero, cuándo, quién, operación, id
    fn escribir(&self, cambios: &[Cambio], ctx: &Contexto) -> Result<(), Fallo>;           // atómico: todos o ninguno
    fn reclamados(&self) -> Result<BTreeSet<String>, Fallo>;                // lo que nombra cualquier ref viva
    fn version(&self) -> Result<String, Fallo>;                             // la clave de las cachés
}
// Cambio { nombre, que: Poner { campos, quitar } | Retirar | Mover { a }, esperado: Cualquiera | Nada | Ml(..) }
// Contexto { autor, operacion, idempotencia }
// Leido { puntero: Node, texto, de: Aqui | Main }: Node, para que los sitios migren sin cambiar su lectura.
// Sin `cambios` ni `en`: las tres lecturas de `/ramas/{r}/cambios` son tres puertas (base, rama, main);
// `/assets/{commit}` sigue siendo de git hasta ·5c, que decide su equivalente en el catálogo.
```

- **`escribir` es un parche:** pone campos y quita otros sobre lo que había, deriva `dataset` del
  `metadata_location` y retira el de antes. Lo que hoy hacen `escribir_puntero_sin`,
  `escribir_informe` y tres reescrituras en el sitio (dos `recoger` e `invoke`), que no siempre hacen
  lo mismo, queda en un solo sitio.
- **El CAS va en el rasgo** (`esperado`). Hoy solo lo tienen `/v1` y `/confirmar`. Con `Ficheros` se
  comprueba contra el directorio, y con `Catalogo`, en la transacción. **Varias tablas = una llamada.**
- **Las ramas** (crear, borrar, traer `main`, fusionar) van en un rasgo aparte que solo llama
  `propuestas`. En `Ficheros` no hacen nada, porque ya lo hace git. En `Catalogo` son `refs` y el
  protocolo de E1a·3. En ·5b se ponen las llamadas en su sitio.

**Cómo llega a cada sitio:**

- **ore-serve:** la costura ya existe. Todo pasa por `leyendo_en(rama, |dir| …)` y
  `escribiendo_en(rama, …, |dir| …)`. El cierre deja de recibir un `&Path` y recibe un `Lugar`, con
  el directorio (para los documentos) y los `Punteros` de esa ref. Cuando ore-serve llama a `ore`
  (`mando::correr`), el `Lugar` le pone `ORE_PUNTEROS` y `ORE_REF`.
- **ore-cli:** cada verbo recibe `&dyn Punteros` construido una vez en `main`. Los 41 sitios dejan los
  ficheros.
- **Las cachés** (`assets_cache`, `ARBOLES`, `cambios_cache`, el refresco) pasan a tener como clave
  `version()`. En `Ficheros` sigue siendo el commit de la rama y el de `main`, así que no cambia nada.

**Lo que se queda fuera de la puerta en ·5b:**

- La superposición (`superponer`, `al_dia.rs`) y la fusión de punteros en git (`derivar`,
  `resolver_punteros`). Son cómo funciona `Ficheros` en una rama: preparan el directorio que la
  puerta lee. Mueren con el último inquilino cortado, así que no se unifican.
- Los Jobs, que siguen con su clon y `Ficheros` hasta ·5d.
- `resultados/`, que sigue en git. `reclamados` lo lee de ahí. Es el punto 15 de §11.5: se propone
  dejarlo fuera.

**La guarda, por fuerza mayor:**

1. **Al compilar:** las piezas de ficheros son privadas de `ore_core::punteros`, así que nada fuera
   puede leer un puntero sin pasar por el rasgo.
2. **En las pruebas:** una prueba recorre `crates/*/src` y falla si encuentra `"datasets"`,
   `"copias"`, `"datasets/` o `"copias/` fuera de la lista de sitios permitidos (la implementación,
   `git.rs` y los Jobs hasta ·5d). Un sitio nuevo de mañana no compila o no pasa.

**Cambios de comportamiento que se aceptan a sabiendas (los demás, cero):**

- **La historia, una sola.** Sigue lo heredado de `main` (como hace hoy medios) y ve la ruta de antes.
  Hoy `/historia`, `/promesas/historia` y los Builds no hacen ninguna de las dos cosas.
- **El 423** de `main` protegida en `/confirmar`, `publicar-retenida` y el commit de colección se
  decide en ·5c, no aquí.

**Los pasos, cada uno con su suite en verde:**

| Paso | Qué | Hecho cuando |
|---|---|---|
| ·5b·1 | El rasgo, los tipos y `Ficheros` en ore-core (envolviendo lo de hoy) | **Hecho (2026-10-28):** `ore_core::punteros::{Punteros, Ramas, Ficheros}`, 7 pruebas propias (parche, CAS de todos o ninguno, el de antes, lo heredado y su historia, `reclamados`, `version`); ore-cli, ore-serve y ore-catalogo compilan sin tocar |
| ·5b·2 | ore-cli entero por el rasgo: las dos primitivas y las tres reescrituras pasan a `escribir` | **Hecho:** los 41 sitios salvo los del mundo git (`al_dia`, `migrar*`, los `resultados/` de `invoke`); el commit de N tablas es una sola escritura |
| ·5b·3 | ore-serve: `Lugar`, los 58 sitios, las siete historias a `historia`/`en` y las cachés a `version` | **Hecho:** `lugar.rs` (una función construye la puerta, otra las `Ramas`); seis historias por `historia()`; las claves de caché ya eran `version()` |
| ·5b·4 | Las piezas, privadas, y la guarda | **Hecho:** las piezas de ficheros en `punteros::crudo`; la guarda `ore-core/tests/la_puerta_de_los_punteros.rs` con 8 sitios permitidos y su razón |
| ·5b·5 | `Proceso` y el protocolo, probados contra un servidor de prueba que envuelve `Ficheros` | **Hecho:** `ore punteros-servir`; el protocolo, especificado en `ore-cli/src/puerta.rs`; la suite de ore-cli, igual directa y por el proceso |

Al terminar ·5b, los ~100 sitios hablan con una sola puerta, y la puerta ya cruza un proceso. ·5c solo
añade `Catalogo` detrás y pasa la misma suite. **Un empuje al final, sin cambio visible** (salvo la
historia), comprobado en victor antes de pedir que se pruebe.

**Lo que dejó ·5b (2026-10-28):**

- **Medido en local, en Docker:**
  - ore-cli pasa igual directo y por el proceso. Solo falla la prueba de fin de línea, que necesita el
    `.git` del worktree.
  - ore-serve pasa 350 pruebas. Fallan las 2 de `git.rs`, que también dependen del worktree.
  - ore-core pasa con la guarda; clippy `-D warnings` y `fmt` limpios en las cuatro crates.
- **`Ficheros` escribe entero o nada:** fuera de la carpeta y renombrando encima (0073 §9 H4), y no deja
  carpetas vacías al retirar.
- **`reclamados` no reclama de menos:** un puntero sin `dataset` reclama su nombre de siempre y el de
  sus bytes.
- **Para ·5c:**
  1. La puerta no dice su ref. `Lugar::en_una_rama` mira el `.ore-al-dia.json`; con el catálogo, la ref
     con la que se abre.
  2. `reclamados` no separa lo de esta ref de lo de las demás. ore-cli lo resta con `de` (`Ajenos`); el
     catálogo puede darlo partido.
  3. Sin `ramas()` enganchados: las ramas de los repositorios de código y las que nacen por un empuje
     directo del puesto.
  4. `datasets::de_main` (el `de` del listado en una rama) sigue leyendo el fichero, para no volver a
     leer cada puntero (F7b·1). Se va con ·5c.
- **Cambios a sabiendas:**
  - La historia sigue lo heredado de `main` y la ruta de antes.
  - Retirar una base no sigue adelante si no puede quitar sus punteros (500).
  - Las fechas de `/paquetes/{n}/copias` son las del commit.
  - Renombrar un schema reescribe los punteros movidos.

### 19.2 ·5c: el catálogo detrás de la puerta (diseño y previo, 2026-10-28)

**Qué es:** la segunda implementación de `Punteros`, `Catalogo`, sobre el Postgres de E1a·4, probada
contra la misma suite que `Ficheros` y **enchufada pero apagada**: un interruptor por inquilino que
nadie enciende hasta el corte (·5g).

**Lo que ya se sabe (mirado al diseñar):**

- **ore-serve puede enlazar Postgres.** Su guarda (`ore_serve_no_sabe_leer_el_lago_ni_un_origen`)
  veta TLS, clientes HTTP y el lago, no el driver. El binario `ore` sí lo veta: por eso existe
  `Proceso`.
- **El catálogo guarda cada puntero por su ruta en el árbol** (`datasets/b/s/n.json`), como lo dejó el
  andamio. La puerta habla por nombres (`b.s.n`). Y la v1 avisa de que escribir con CAS y fusionar
  «entra en la v2». **·5c empieza por el esquema v2.**
- **ore-serve llama a `ore` en cada `loadTable`** (y en `/datasets`, `/colecciones`, preview y
  ejecutar vista). Si cada `ore` arranca `ore-catalogo punteros` y abre una conexión a Postgres, eso
  se paga en cada petición. Hay que medirlo antes de elegir.

#### El previo (medio día; sin código de producto)

Cada pregunta, con cómo se contesta y qué decide.

| | Pregunta | Cómo | Decide |
|---|---|---|---|
| **P1** | ¿Cuánto cuesta abrir la puerta por proceso en cada `ore`? Arrancar `ore-catalogo punteros`, conectar y un `uno`, p50/p99, desde un pod de t-victor | Un Job de banco en `jobs-s` con el binario de release (como ·4a), contra un esquema de prueba cargado con victor | Si es bajo frente a lo que tarda hoy `ore datasets --cargar`: `ORE_PUNTEROS` directo. Si no: **un relevo**. ore-serve sirve la puerta por un socket local con su propio pool, y `ore` le habla por un relevo mínimo, sin conectar a Postgres cada vez |
| **P2** | `uno`, `todos(ref)`, `historia` y `version` sobre victor cargado (157 punteros en `main`, 19 ramas, ~800 commits), en milisegundos | El mismo Job | Si `version()` puede ir en el camino caliente de `/assets` (una consulta por petición) o necesita su propia caché |
| **P3** | ¿Qué pruebas montan punteros como ficheros o **miran el fichero** después de que `ore` escriba? | Leer las pruebas de ore-cli y ore-serve y clasificarlas: (a) siembran con ficheros y leen por `ore`; (b) comprueban el fichero escrito | Cómo pasa la suite con `Catalogo`. (a) se resuelve **sembrando**: al abrirse en modo prueba, el catálogo carga el directorio como un commit (el `cargar` de la mudanza, sin historia). (b) se reescriben para leer por la puerta. Sale con número |
| **P4** | Cada escritura de ore-serve y de ore-cli: ¿solo puntero, mixta (puntero y `Dataset` escrito por máquina) o de definición (retirar, renombrar, fusionar)? | Del inventario (§2.3 y §3), una por una | La regla de cada clase con el catálogo: **solo puntero** = una transacción y nada de git; **mixta** = el catálogo primero y el documento a git después (lo que no llegue lo arregla el documentador, ·5e: decisión (b) de §11.3); **definición** = el protocolo de E1a·3 |
| **P5** | ¿Quién usa el `commit` de la historia y `/assets/{commit}`? | `rubix-platform` y las pruebas | Si `Version.id` puede ser el `git_sha` de lo replicado y el id del catálogo después del corte, y qué es `/assets/{commit}` sin commits de git |
| **P6** | `main` protegida: ¿quién escribe hoy en `main` sin propuesta y debe seguir haciéndolo? | Del inventario: la copia, el mantenimiento, `/confirmar`, `publicar-retenida`, las colecciones | Si el 423 va en el catálogo con excepciones por operación, o se queda como hoy |

**Por qué basta medio día:** P1 y P2 son un Job con el binario ya compilado y el banco de ·4a; P3 a
P6 son lectura. No hace falta el PR #58 fusionado: el esquema de prueba se carga como en ·4a.

#### Lo que contestó el previo (2026-10-28)

Medido contra `catalogo-db` desde un pod de `jobs-s` (de nodo a nodo, como en producción), con victor
cargado en un esquema de prueba: 773 commits, 157 punteros en `main`, 18 ramas, 4.100 versiones y
**0 diferencias** en 3.007 comparados, en 6,8 s. El esquema y el pod, borrados al acabar. La línea
base, en el ore-serve de victor y sobre su propio árbol (solo lectura).

**P1. Conectar cuesta lo mismo que la mitad de un `loadTable`: no se conecta en cada `ore`.**

| | Medida |
|---|---|
| Hoy, `ore datasets --cargar` (lo que corre cada `loadTable`) | **~110 ms** (50 seguidas, 5,5 s), con la lectura del `metadata.json` en GCS |
| Hoy, solo arrancar `ore` | ~5 ms |
| Conectar a `catalogo-db` y un `SELECT 1` (SCRAM; el DNS no pesa) | **p50 24 ms · p90 31 · p99 66** |

Un `ORE_PUNTEROS` que conecta en cada `ore` sumaría ~25–65 ms a cada `loadTable`, y ore-serve llama a
`ore` más de una vez en algunas rutas. **Decisión: la puerta heredada.** ore-serve lanza `ore` con dos
tuberías heredadas (`ORE_PUNTEROS=fd:3,4`) y sirve el protocolo de `Proceso` desde su lado, con su
pool. Ni proceso de más ni conexión por llamada. `ore-catalogo punteros` (proceso y conexión propios)
queda para los Jobs (·5d), que conectan una vez por pasada.

**P2. Con la conexión abierta, todo es de milisegundos; `version()` cabe en el camino caliente.**

| Operación (200 veces, conexión abierta) | p50 | p90 | p99 |
|---|---|---|---|
| `uno` en `main` | 0,66 ms | 0,91 | 2,0 |
| `uno` en una rama | 0,90 | 1,4 | 2,0 |
| `todos` en `main` (157) | 8,0 | 10,2 | 22,8 |
| `todos` en una rama (158) | 8,5 | 14,0 | 17,9 |
| `historia` (50 versiones) | 3,9 | 5,1 | 11,3 |
| `version` | 0,59 | 0,76 | 1,3 |
| `reclamados` (197) | 1,0 | 1,3 | 1,7 |
| `escribir` (CAS, historia, outbox) | 2,8 | 3,4 | 4,8 |

`escribir` se midió sobre tablas temporales, sin WAL. La cifra con commit de verdad y el evento por
decodificación lógica es la de E1a·1b: ≤ 3,3 ms p99. `uno` usa el índice de la clave
(`estado_pkey`): 0,1 ms en el servidor.

**P3. Sembrar una vez por árbol cubre casi todas las pruebas; se reescriben 7.**

| | ore-cli (`tests/`) | ore-cli (`src`) | ore-serve |
|---|---|---|---|
| Siembran ficheros y leen por la puerta | 4 | 1 | 8 |
| Miran el fichero escrito | 3 | 1 | 0 |
| Usan la historia de git del puntero | 0 | 0 | 2 |
| Prueban `Ficheros` mismo (se quedan así) | — | 10 | — |

- **Sembrar al abrir, una sola vez por árbol** (una marca en el catálogo), con el `cargar` de la
  mudanza sin historia. Volver a sembrar en cada apertura pisaría lo que `ore` escribió, porque eso no
  llega al directorio.
- **Se reescriben 7**, para escribir o leer por la puerta:
  - 2 escriben ficheros a mitad del escenario: `la_coleccion_es_un_activo.rs:90` y
    `puestos.rs:7027`.
  - 3 miran el fichero: `la_coleccion…:197`, `la_copia_de_una_fuente_cerrada.rs:69` y
    `schemas.rs:172`. Sus igualdades byte a byte pasan a ser por campos, porque `Leido.texto` solo es
    comparable dentro de una implementación.
  - 2 de historia en git pasan a versiones escritas por la puerta: `builds.rs:3351` y
    `medios.rs:1198`.
- Fuera de Rust, `pruebas-de-fuego/el-lago.sh` (§5 y §11) cuenta versiones por
  `/arbol/historia/datasets/…`. Pasa a `/historia/dataset:…` antes del corte.

**P4. Las escrituras, por clase.** El `Dataset` escrito por máquina se regenera solo si cambian las
columnas, `changes` o `derivedFrom` (`asegurar_dataset_de`). La mixta es la excepción, no la regla.

| Clase | Cuáles | Con el catálogo |
|---|---|---|
| **Solo puntero** | `/v1` commit y transacciones (casi siempre), `/confirmar`, `publicar-retenida`, la copia, recoger, `invoke` | Una transacción. Git no se toca |
| **Mixta** | `/v1` cuando nace la tabla o cambian sus columnas o su linaje; el commit de colección (`derivedFrom`) | Primero el catálogo y después el documento a git. Si git falla, el dato ya está y el documento lo pone el documentador (·5e, decisión (b)). **Por eso el corte necesita ·5e** |
| **Definición** | Retirar un `Dataset`, retirar una base, renombrar un schema, fusionar una propuesta | El protocolo de E1a·3: preparar, git, aplicar, y el reconciliador |

**P5. El id de una versión es opaco; `/assets/{commit}` no lo usa nadie.**

- La consola usa `versiones[].commit` solo como clave de React. Puede ser el `git_sha` de lo replicado y
  el número del catálogo después. Se ordena por el orden de la puerta, nunca por el id.
- `/assets/{commit}` no tiene clientes. En ·5c pasa a dar solo los documentos de ese commit, sin
  punteros. Su validador acepta hexadecimal: un id decimal del catálogo se tomaría por un sha, así que
  nunca se reutiliza para eso.
- **`/assets` `cabeza` sigue siendo la cabeza de git de los documentos**: es el `If-Match` del editor.
  La clave de caché pasa a `version()`, pero `cabeza` no.

**P6. `main` protegida: en ·5c, igual que hoy.** Hoy escriben en `main` sin propuesta la copia, el
mantenimiento, `/confirmar`, `publicar-retenida` y el commit de colección. `/v1` da 423. `Catalogo`
reproduce exactamente eso (la puerta recibe la operación en `Contexto`). Cerrar el hueco de §11.6 es
una decisión aparte y no cabe en un cambio que promete no cambiar nada.

#### Los pasos (los cierra lo que diga el previo)

| Paso | Qué | Hecho cuando |
|---|---|---|
| ·5c·1 | **Esquema v2.** El puntero por su nombre (`b.s.n`), con la ruta guardada para el andamio y la auditoría. `escribir(ref, cambios, autor, operación, idempotencia)` en una transacción: CAS contra el puntero **efectivo** (lo heredado de `main` cuenta), historia, outbox. Lo escrito sobre algo heredado pasa a ser de la rama. `reclamados` partido (esta ref y las demás). Migración v1 → v2 sobre lo cargado | **Hecho (2026-10-28):** `esquema_v2.sql` y la API de `almacen` (`escribir`, `efectivo`, `vista_entera`, `historia`, `version`, `crear_rama`, `traer_main`, `vivos`). **Por ruta, no por nombre:** el catálogo se comporta como un directorio de punteros (la de su sitio y la de antes) y quien abre la puerta resuelve el nombre, como `Ficheros`; el andamio no cambia y no hay migración de claves. 7 pruebas contra Postgres 16 (`ORE_CATALOGO_PRUEBAS`): CAS de todos o ninguno, el de antes, rama sobre lo heredado y su historia, lápidas, idempotencia, 8 que compiten (gana 1), v1 → v2 |
| ·5c·2 | **`Catalogo`** (`Punteros` y `Ramas`) y **`ore-catalogo punteros`**, que sirve el protocolo de `Proceso` | **Hecho (2026-10-28):** `ore_catalogo::Catalogo` (`abrir`, `sobre`/`soltar` para el pool, `con_arbol` para los `resultados/` que siguen en git), `sembrar_si_hace_falta` (una vez por árbol, marcado `sembrado:<raíz>`) y `ore-catalogo punteros [--sembrar]`. **La prueba de conformidad** pasa un guion de ~60 observaciones por `Ficheros`, `Catalogo`, el protocolo en memoria y un hijo real, y exige lo mismo. Destapó que `Ficheros` degradaba `null` y decimales a cadenas al poner encima: arreglado |
| ·5c·3 | **ore-serve:** `Lugar::de` elige por inquilino (`--punteros catalogo`, apagado), con la ref de la costura y un pool de conexiones. **La puerta heredada** (P1): `ore` recibe dos tuberías (`ORE_PUNTEROS=fd:3,4`) y ore-serve le sirve el protocolo con su pool. `/assets/{commit}` pasa a ser solo documentos (P5) | **Hecho (2026-10-28):** `--punteros ficheros\|catalogo` (apagado) y `--catalogo-esquema`; `Catalogo` enchufado en `lugar::abrir_catalogo` con un pool de 8 conexiones (una conexión que falló no vuelve). **La puerta heredada** (`heredada.rs`): dos tuberías con sus números reales (`ORE_PUNTEROS=fd:<lee>,<escribe>`, no 3 y 4: `dup2` podía pisar el descriptor con el que `std` avisa de un `exec` fallido), un hilo por `ore`, cerradas al acabar; en local, +2 ms por `--cargar` (debug). La ref llega a `Lugar::de` por una guarda de hilo (`EnRef`). `/assets/{commit}` sin punteros; `cabeza`, la de git. **Pendiente para ·5c·4:** las pruebas de ore-serve no lanzan `ore`, así que forzar la puerta heredada en ellas no prueba casi nada: hace falta un arnés de punta a punta (ore-serve y `ore` por HTTP) |
| ·5c·4 | **La suite con las dos:** ore-cli y ore-serve con `Catalogo`, sembrando una vez por árbol (P3). Se reescriben las 7 pruebas de P3, y `el-lago.sh` pasa a `/historia` | Las mismas cuentas que con `Ficheros` |
| ·5c·5 | **Las escrituras por clase** (P4). Solo puntero: sin git. Mixta: catálogo y luego git. Definición y fusión de propuestas: el protocolo de E1a·3 en Rust (preparar, git, aplicar y su reconciliador), que hasta hoy es el banco en Python | El banco de E1a·3 (fallos a mitad, dos fusiones a la vez, escritores durante la fusión) pasa contra el código de verdad. `main` protegida, como hoy (P6) |
| ·5c·6 | **Banco en victor con el interruptor encendido en una celda de prueba**, sin tocar victor: `loadTable`, `/assets`, un commit de `/v1` y una fusión, comparados con `Ficheros` | Latencias medidas y anotadas aquí. Escribir un puntero en milisegundos |

#### ·5c·5 por partes: primero el puntero y la mixta (diseño, 2026-10-28)

·5c·5 se parte en tres. **·5c·5a** (solo puntero) y **·5c·5b** (mixta) son acotadas y van ahora.
**·5c·5c** (definición: retirar, renombrar y fusionar, con el protocolo de E1a·3 en Rust) va
aparte, con su propio diseño.

**Lo que ya pasa hoy con el interruptor encendido.** `escribiendo` y `escribiendo_en` ya no
publican si el árbol no cambió (`hay_cambios`). Como el puntero va al catálogo por la puerta, una
escritura de solo puntero **ya no hace commit ni push**. Pero sigue pagando **el árbol de escribir**:
pone al día el espejo, clona de él y, en una rama, superpone lo de `main` (`clonar`,
`clonar_rama_al_dia`, `deshacer_al_dia`). Todo eso existe solo para que `ore` pueda escribir
ficheros, y con el catálogo el único fichero que puede escribir es el `Dataset`.

**La idea: dos fases, y la segunda solo si hace falta.**

1. **Fase 1, en el árbol de leer** (el compartido de `leyendo_en`, el que ya usa cada `loadTable`):
   - `ore` corre con `--documentos-aparte`. Escribe los punteros por la puerta heredada (una
     transacción del catálogo) y **no escribe ningún documento en el árbol**, que es compartido y de
     solo lectura.
   - Si el `Dataset` tiene que regenerarse (nace la tabla, cambian sus columnas, `changes` o
     `derivedFrom`; lo decide `asegurar_dataset_de` como hoy), lo devuelve en su respuesta:
     `documentos: [{ruta, texto}]`.
   - El 423 de `main` protegida se comprueba igual que hoy, con la política del árbol, antes de
     escribir nada.
2. **Fase 2, solo si hay documentos** (eso es la mixta): `escribiendo_en(rama)` con un cierre que
   escribe esos ficheros y nada más, y `publicar`.

| | ·5c·5a · solo puntero | ·5c·5b · mixta |
|---|---|---|
| **Cuáles** | `/v1` commit y `transactions/commit` sin cambio de documento, `/confirmar`, `publicar-retenida` | `/v1` `createTable` (siempre nace), commit con columnas o linaje nuevos, commit de colección con `derivedFrom` nuevo (`escritas.rs`) |
| **Git** | **Nada.** Ni árbol de escribir, ni commit, ni push | Un commit con solo el documento, **después** del catálogo |
| **Si algo falla** | El catálogo dice que no (409 por el CAS, 503) y no se escribe nada | El dato ya está y la respuesta es 200. Git se reintenta con la rebase de siempre (×3, reescribiendo el mismo documento, que es de máquina). Si aún falla: `x-ore-documento: pendiente` en la respuesta y una línea en el registro; lo arregla el documentador (·5e). Es la decisión (b) de §11.3 |
| **Coste esperado** | El de un `loadTable` de hoy más una transacción (≤ 3,3 ms p99) | El de hoy, pero solo en lo raro |

**Lo que cambia, pieza a pieza:**

- **ore-cli:** `--documentos-aparte` en los verbos que escriben (`datasets --commit`, `--crear`,
  `--confirmar`, `--publicar-retenida`). Es lo que ya decide `asegurar_dataset_de`, pero devuelto en
  vez de escrito.
- **ore-serve:**
  - Con el catálogo, las rutas de arriba usan las dos fases. Con `Ficheros`, el camino de hoy sin
    tocar.
  - `escritas.rs` (colecciones) hace lo mismo por su lado: el puntero por la puerta y el
    `derivedFrom` a git solo si cambia.
  - La clave de idempotencia de la petición (ADR 0031) llega a `Contexto.idempotencia`. Así, una
    petición repetida tras escribir en el catálogo y antes de contestar no escribe dos veces.
- **Fuera:** retirar, renombrar, fusionar (·5c·5c); los Jobs (·5d); el documentador (·5e).

**Hecho cuando:**

- **·5c·5a:**
  - Pruebas de ore-cli: con `--documentos-aparte`, cada verbo escribe el mismo puntero que sin él y no
    deja ningún fichero de documento; devuelve el documento exactamente cuando hoy lo escribiría.
  - En el banco de ·5c·6: un `/v1` commit sin cambio de documento hace **0 operaciones de git de
    escritura** (lo dice la línea de acceso de ore-serve) y tarda lo de un `loadTable` más
    milisegundos.
- **·5c·5b:**
  - Prueba de ore-serve: la fase 2 confirma solo el documento.
  - En el banco: un `createTable` deja el puntero en el catálogo y un commit con su `Dataset`. Con la
    forja parada, contesta 200 y `pendiente`.

**Estado (2026-10-28): ·5c·5a y ·5c·5b hechos** (`2eb00133`, en local):

- **ore-cli:** `--documentos-aparte` en `--commit`, `--crear`, `--confirmar` y
  `--publicar-retenida`. Solo crea una copia temporal del árbol cuando hay un documento que
  regenerar; si solo cambia el puntero, no copia nada.
- **ore-serve:** las dos fases viven en `dos_fases.rs` y solo se activan con el catálogo.
  - Con `Ficheros`, cada ruta toma el camino y los argumentos de antes.
  - La clave de idempotencia llega a `Contexto` también en `createTable`, `/confirmar` y
    `publicar-retenida`.
  - `pendiente` sale como `"ore.documento"` en el cuerpo y en la cabecera `x-ore-documento`.
- **Pruebas:** 4 de ore-cli comparan dos árboles gemelos (el marcado queda idéntico, el puntero es
  el mismo y el documento devuelto es el fichero que se habría escrito). 8 de ore-serve usan una
  forja `file://` de verdad: con solo puntero, `main` no se mueve y no se abre árbol de escribir; la
  mixta deja un commit con solo el documento; con un `pre-receive` que lo rechaza todo, salen 3
  intentos, un 200 y `pendiente`.
- **Sin probar todavía:** las pruebas de ore-serve fuerzan las dos fases con `Ficheros` detrás, no
  con el catálogo de verdad. Eso queda para el banco de ·5c·6.

**Orden y paralelo:** el previo primero. Luego ·5c·1 solo, porque es el contrato del almacén. Después
·5c·2 y ·5c·3 en paralelo (crates distintas). ·5c·4 y ·5c·5 cuando estén los dos. ·5c·6 al final. Un
empuje, con el interruptor apagado: en producción no cambia nada.

**Lo que ·5c no hace:** encender el interruptor en ningún inquilino (·5g), los Jobs (·5d), el
documentador y `guardar` (·5e), ni §18 (·5f).

