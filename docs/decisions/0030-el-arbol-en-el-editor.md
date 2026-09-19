# 0030 · El árbol en el editor

**Estado:** propuesto (medido y escrito el 2026-09-18) · **Fecha:** 2026-09-18 · **Decide:** que la
**superficie principal** con la que una organización construye su ontología es el **code
workspace** de la consola —un editor sobre **el árbol de su celda**, con el compilador de la
ontología detrás y git como memoria— y no una aplicación de formularios; que la Ontology Forge
queda como **motor** (`documentos.rs`: leer, escribir compilando antes de empujar, el commit del
sujeto, la regla de no empeorar) y sus pantallas pasan a ser **entradas al workspace**; que el
«entorno» se construye en **cuatro peldaños** (W0 el árbol en el editor · W1 ejecutar la
pregunta · W2 proponer · W3 la sesión viva) y que W0 no añade **ningún runtime**: sirve el árbol
por `ore-serve`, guarda con la figura que ya existe y enseña los diagnósticos de `ore` como
marcadores. Sigue a [`docs/la-pregunta.md`](../la-pregunta.md) y a
[`0029`](0029-donde-corre-una-funcion.md).

## El problema

La ontología **ya es código**: documentos YAML en un repositorio por inquilino, con un
compilador (`ore validate`) que dice qué está mal, dónde y qué hacer, y un servidor
(`ore-serve`) que clona, compila y publica con quién lo pidió. Y sin embargo la consola la
enseñaba como formularios: una pantalla por kind, un campo por clave, y una vista que el
cliente no podía **preguntar** (`la-pregunta` §1: el valor de la vista era estructural, no
semántico). Mientras, en Projects › *Code workspace* había un editor completo —árbol de
ficheros, pestañas, Monaco con tema Carbon, celdas— que **no abría nada real**: árbol,
celdas y contenido eran estado de cliente (`components/code-workspace/types.ts`: *«nothing is
executed or persisted»*).

Lo que hacen los demás confirma la dirección y marca el hueco. **Snowflake Workspaces** es un
IDE en el navegador donde *todo es fichero* y git va detrás; crear una vista te lleva a un
worksheet con `create view … as <select>`, y una semantic view es un YAML con tres entradas
(*Create with CoCo · Guided wizard · Start blank*). **Foundry Code Workspaces** da a cada
persona un contenedor aislado respaldado por un repositorio con ramas, sin salida a internet, y
*proposals* para cambiar la ontología. **dbt** compila un proyecto de ficheros. Ninguno tiene
**el compilador de la ontología dentro del editor**: sus diagnósticos son de SQL o de YAML, no
de significado. Ese es nuestro sitio.

## Lo medido (`pruebas-de-fuego/medida-w0-el-arbol-en-el-editor.py`, 2026-09-18, demo)

| | medido | lo que decide |
|---|---|---|
| el árbol de demo | **801 ficheros, 922 KB**: Table 253, View 253, Entity 250, y el resto | cabe entero en el navegador; el índice se puede servir de una vez |
| `ore validate .` sobre el árbol entero | **2,75 s**, 56 errores (los viejos de demo) | vale **al guardar**; no vale por tecla |
| `ore validate <fichero>` | 0,05 s, pero sólo forma: no ve referencias | la forma va al navegador (esquemas JSON); el significado a `ore-serve` |
| clonar el árbol | 1,43 s en local; **1,04–1,27 s** por petición en demo (`GET /documentos/{kind}`: clon + listar) | «una petición = un clon» aguanta abrir y guardar, que es lo que un editor hace |
| `ore-serve` hoy | GET/PUT/DELETE por kind (Entity, View, Table, Concept, Interface); PUT compila antes de empujar, no empeora, commit del sujeto, 409 si el árbol se movió | la figura de «guardar» existe; falta **cualquier fichero por ruta** |
| diagnósticos | de stderr: código, mensaje, `fichero:línea:col`, ayuda | es un marcador de Monaco; falta severidad y rango |
| la forja | Forgejo 15, una por celda, con API de contenido por ruta y rama, ramas y PRs | W2 la toma prestada; W0 no la necesita |
| el editor | Monaco + `@monaco-editor/react` en la consola; 28 esquemas JSON en `vendor/oos/schemas` | completado y validación de forma **sin red**; `monaco-yaml` |

## Decisión

> ### ① El workspace abre el árbol de la celda, y nada más que el árbol.

El árbol de ficheros del workspace **es** `git ls-files` del inquilino, con el kind de cada
YAML: `packages/<base>/{tables,views,entities,functions}`, `modelos/`, `copias/`,
`resultados/`, `conduits.yaml`, `ontology.config.yaml`. No hay semilla, ni «Untitled», ni
proyecto aparte: el proyecto es la celda. Un fichero se abre por su ruta y se guarda por su
ruta. Lo que hoy es una carpeta de la Forge —*Entities*, *Views*— es un filtro sobre ese árbol.

> ### ② `ore-serve` sirve el árbol por ruta con la misma figura que los documentos.

Tres verbos, y ninguno nuevo en su fondo:

| verbo | qué | de dónde sale |
|---|---|---|
| `GET /arbol` | el índice: ruta, tamaño, kind (si es un documento), commit | `git ls-files` sobre el clon + la cabecera de cada YAML |
| `GET /arbol/<ruta>` | el texto del fichero y su commit (`autor`, `cuando`) | el clon |
| `PUT /arbol/<ruta>` | escribe el fichero, **compila antes de empujar** (`empeora`: no empeorar), commit del sujeto, 409 si el árbol se movió; devuelve **los diagnósticos** del árbol tras el guardado, con `fichero:línea:col`, código, mensaje, ayuda y severidad | `documentos.rs::escribir_documento`, generalizado a una ruta |
| `DELETE /arbol/<ruta>` | retira el fichero con la misma regla | ídem |
| `GET /arbol/diagnosticos` | los diagnósticos del árbol tal como está | `diagnosticos_de` |

La ruta se valida como la de un documento: bajo la raíz, sin `..`, sin `.git/`, y **no** puede
tocar lo gobernado que se induce (`discover.*.json`: eso lo escribe `review`). Lo que hoy hace
`PUT /documentos/{kind}/{ns}/{n}` sigue valiendo; es un caso particular.

> ### ③ Guardar es compilar. Los diagnósticos son marcadores.

No hay «validar» aparte: `PUT` compila y devuelve lo que el árbol dice, y el editor lo pinta
donde toca (`{startLineNumber, startColumn, code, message, severity}` con la `ayuda` en el
tooltip). Un guardado que **empeora** el árbol es 422 con los diagnósticos **nuevos** y no se
publica: el editor lo enseña en línea y el fichero queda sucio. Un guardado que no empeora,
aunque el árbol siga con sus errores viejos, se publica: es la regla de la Forge y de retirar,
y la misma con la que un modelo entra. La forma (claves, tipos, `apiVersion`) la valida el
navegador con los esquemas de OOS por kind, sin red: es lo que evita mandar al servidor lo que
no puede compilar.

> ### ④ Crear una vista abre el workspace con el fichero escrito, no guardado.

*Assets Catalog › esquema › Create › View › Standard | Materialized* no abre un modal: abre el
workspace con `packages/<base>/views/<nombre>.yaml` **pre-rellenado** desde la tabla elegida
—`from`, `fields` con todas las columnas, `owner` el de la base; y con `materialized` si es
*Materialized*— y el cursor en el nombre. Es el `create view … as <select>` de Snowflake en
nuestro dialecto, que es YAML. Guardar es ②; si es *Materialized*, `tras_inducir` hace lo de
siempre (conducto, Job 48, informe). La misma puerta sirve para *Function* (`functions/`) y
para *Model this* (`entities/` con `backedBy`), que dejan de ser formularios.

> ### ⑤ Los cuatro peldaños, y qué cierra cada uno.

| | qué | acepta |
|---|---|---|
| **W0** | el árbol en el editor: ①–④ | demo (801 ficheros) abierto en el workspace; abrir una vista, romper una referencia, guardar → 422 con el marcador en su línea; arreglar, guardar → commit de la persona en la forja y Data › Jobs sin novedad; *Create › View › Standard* deja el fichero pre-rellenado |
| **W1** ✓ 2026-09-18 | ejecutar la pregunta: *Run* sobre una `View` = el plan + filas de la copia (motor de proyección/filtro/agregado sobre el Parquet del bucket, en `ore-drivers`) o del origen para una foránea | una vista con `where` y `groupBy` devuelve filas en la celda sin abrir el origen de una base estándar — en victor, `standard_postgre_3.products`: 20 filas en 1,6 s desde la copia. Lo que queda alrededor (aviso, frío, panel, bundle) está medido en 0027 |
| **W2** | proponer: rama por persona, PR en la forja, diff y diagnósticos de la rama, merge → Flux | dos personas, dos ramas, una revisión |
| **W3** | la sesión viva: un pod por persona en la celda para celdas Python/notebook con la copia legible, sin salida, Kueue, TTL | una celda Python lee `over` como DataFrame y no alcanza internet |

## Lo que esto cierra, y lo que no

Cierra la superficie: **un sitio** donde la ontología se lee, se escribe, se compila y se
guarda, para la persona y para el modelo (una `Function` de lectura que proponga vistas y
entidades escribe en el mismo árbol por la misma puerta). No cierra: ejecutar (W1), proponer
(W2), correr código que no sea YAML (W3), ni la calidad de lo que un modelo proponga.

## W2, medido antes de construir (`pruebas-de-fuego/medida-w2-proponer.py`, 2026-09-18, victor)

La intuición era que la lógica ya está. La medida dice **dónde está y dónde no**, pieza a pieza,
con el testigo de `serve-victor` desde su pod (cuatro PRs de medida abiertas y cerradas; `main`
sin tocar):

| | medido | qué dice para W2 |
|---|---|---|
| **la forja** (Forgejo 15.0.7, API gitea 1.22) | crear rama por API **1,4 s** · clonar la rama **0,7 s** · empujar el commit **1,5 s** · abrir la PR **0,7–1,6 s** · `mergeable: true` · ficheros de la PR **100 ms** · `.diff` **80 ms** (744 B) · review `COMMENT` **0,5 s** | todo lo que W2 necesita existe y cuesta segundos; ninguna pieza nueva en la forja |
| **lo que la forja NO hace** | `serve-victor` es el autor de TODAS las PRs y la forja **no deja aprobar la propia (422)**; el testigo no escribe comentarios de issue (403, ámbito); `main` **no está protegida** (ore-serve y el Job del catálogo empujan directo) y `branch_protections` es 403 para un colaborador; no hay CODEOWNERS (404); borrar la rama **no cierra la PR**, cerrarla es un `PATCH` | **la revisión es nuestra, no de la forja**: la forja guarda rama, PR, diff y comentarios; quién puede fusionar y que no sea quien propuso lo decide `ore-serve` con la identidad de la sesión, y lo deja escrito como review `COMMENT` (`persona:bea aprueba`) y en la huella. Proteger `main` rompería W0/W1 (todo empuja a main): se protege **por convención del servidor**, no por la forja |
| **el diagnóstico** | `ore validate` de la rama **80 ms** tras el clon (y con un fichero mal puesto: OOS2035 con su línea, medido de paso) · `ore diff main rama` **100 ms**: JSON semántico con `changes[{axis, code, …}]`, `requiredBump` y `verdicts` (una vista nueva → `OOS5021`, `patch`, `CONSUMER: compatible`) | la PR enseña **dos diffs**: el de la forja (líneas) y el de `ore diff` (significado: qué cambia para quien consume), más los diagnósticos de la rama — los tres ya se calculan |
| **ore-serve** | 5 rutas tocan el árbol (`GET /arbol`, `/arbol/diagnosticos`, `GET/PUT/DELETE /arbol/{ruta}`); `clonar()` sin `--branch` (= main); `publicar()` hace `push origin HEAD` (= main); **0** menciones a rama/PR | el hueco está aquí y es pequeño: `leyendo`/`escribiendo` ganan la rama (`?rama=` en las cinco rutas; sin ella, main como hoy), y nacen las rutas de W2: `GET/POST /ramas`, `GET/POST /propuestas`, `GET /propuestas/{n}` (ficheros, diff de líneas, `ore diff`, diagnósticos de la rama), `POST /propuestas/{n}/revisar`, `POST /propuestas/{n}/fusionar`, `DELETE /propuestas/{n}` |
| **la identidad** | autor del commit = la persona (`sub`), committer `ore-serve` (RFC 8693, ya); autor de la PR en la forja = `serve-<n>`, la persona sólo en el cuerpo; 17 potestades en ore-iam, **ninguna** sobre el árbol; dueño del paquete = `team:<org>`, sin equipos en la forja | la PR lleva `sub` en el cuerpo (como el commit lleva el autor); **`dos personas`** = quien fusiona ≠ quien propuso, comprobado por `ore-serve` contra el `sub` de la PR; quién puede fusionar: hoy cualquiera con sesión en la organización escribe en main, así que W2 no puede exigir menos que eso — una potestad `propuesta:fusionar` es de la iteración siguiente, no de esta |
| **la consola** | `BranchesView` (181 líneas, `Rama{nombre, porDefecto, protegida, checks, pr}`), `PullRequestsView` (403 líneas: lista Open/Merged/Closed, detalle, *Files changed*, *Comments*, nueva PR, `PullRequest{numero, titulo, descripcion, estado, head, base}`, `CambioDeFichero`, `Comentario`), botones *Branches · Commit · Pull requests* en la barra — todo con mocks y «todavía no implementado» ×12; **0** llamadas de `lib/server` a ramas o PRs | las pantallas son las de W2 y las formas casan con lo que la forja devuelve: se cablean, no se rediseñan. Lo que falta de superficie: en qué rama estoy (el editor guarda en main hoy) y el botón *Commit* que hoy no hace nada |
| **Flux** | `inquilino-victor` y `trabajo-victor` miran **sólo `main`** (5 m de respaldo); merge → main → aviso **~4 s** (17-el-aviso, medido hoy) | una rama no despliega nada, que es exactamente lo que se quiere; el merge es lo que despliega, y ya avisa |

**⇒ Cómo se construye W2**, en el orden en que la medida lo dicta: (1) `ore-serve` sabe de ramas
(`?rama=` en las rutas del árbol; `clonar(rama)`, `publicar` a la misma rama); (2) las rutas de
propuestas sobre la API de la forja, con la revisión y el «dos personas» en el servidor; (3) la
consola cablea las tres pantallas y el botón *Commit* pasa a ser «proponer»; (4) prueba de fuego:
dos sujetos, dos ramas, una revisión, merge → Flux → Job. Lo que se aparca: proteger `main` en
la forja, CODEOWNERS y `propuesta:fusionar`.

**(1) y (2) hechos (2026-09-18, `la-propuesta.sh` 1–9, en CI).** La rama va en la cabecera
**`X-Ore-Rama`** y no en la URL (ningún dato entra por la URL): las cinco rutas del árbol leen y
escriben EN esa rama —`clonar_rama`, y `publicar` empuja a `HEAD`, que es la rama— y sin cabecera
son `main` como siempre; el gate «no empeora» sigue en la rama. `forja.rs` es la API de la forja
por `http::pedir` (que aprendió `chunked`: la forja es Go), `Api::de(url)` saca destino y
`dueño/repo` de la URL del árbol, y `--forja-api` la dice aparte cuando el árbol va por `file://`
(el banco: `forja-de-mentira.py`, git de verdad para ramas y diffs, PRs y reviews en memoria, un
solo usuario como la de verdad). Las rutas: `GET/POST /ramas`, `DELETE /ramas/{n}` (409 con
propuesta abierta, 422 la de por defecto), `GET/POST /propuestas`, `GET /propuestas/{n}`
(ficheros, diff de líneas, `semantico` = `ore diff base rama`, diagnósticos de la rama,
revisiones), `POST …/revisar {veredicto: aprobar|pedir-cambios|comentar, texto}`, `POST
…/fusionar`, `DELETE …/{n}`. **La persona viaja en el cuerpo**: la PR nace con `sub: <persona>`
en la primera línea y cada revisión es una review `COMMENT` que empieza `revision: <persona>
<veredicto>` — la forja guarda, el servidor decide: quien propone no aprueba ni fusiona lo suyo
(422), sin aprobación de otra persona no se fusiona (422), la rama tiene que compilar (422 con
los diagnósticos) y no tener conflictos (409); el commit de merge dice quién propuso, quién
revisó y quién fusionó, y la rama se retira al fusionar. Y el panel de *Commit* del
workspace pedía lo que un `PUT` por fichero no da: **`POST /arbol/commit`** (2026-09-19) — varios
ficheros en UN commit con el mensaje de la persona, y en `seco` lo que ese commit sería: `A`/`M`/`D`
y +/− los dice git (`status --porcelain`, `diff --cached --numstat`) sobre el clon, con el gate de
siempre (`la-propuesta` 3b). En la consola, «sin commitear» son los borradores de la sesión:
guardar (Ctrl+S) commitea uno, *Commit* los manda todos. **Version history** no
necesitó inventar nada (`medida-w2-historial.py`, victor: 12 versiones en `ontology.config.yaml`,
`git log --follow` <10 ms tras el clon): `GET /arbol/historia/{ruta}` (persona, committer, mensaje
por commit) y `GET /arbol/version/{hash}/{ruta}` (el texto de entonces, byte a byte); restaurar es
guardar ese texto, un commit nuevo. Y el *Merge* del menú es `POST /ramas/{rama}/fusionar {desde}`:
`git merge --no-ff` de la persona con el gate, conflicto 409 con el fichero, y a `main` nunca a
mano (422: se propone). Lo que la consola tiene que cablear
está en `propuestas.rs`; lo que no se hizo: encolar la copia de una vista `materialized` que
llegue por merge (hoy `PUT /arbol` tampoco lo hace) y proteger `main` en la forja.

## Lo que se aparca

- El *language server* de verdad (marcadores por tecla) espera a que un clon deje de costar un
  segundo o a que W3 tenga una copia de trabajo viva por persona.
- Las pantallas de la Forge no se borran: se enlazan al workspace y se dejan de crecer.
- La severidad y el rango en la salida de `ore` (hoy todo es `error` con posición de inicio):
  se añaden cuando un marcador los pida.
