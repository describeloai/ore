# Decisiones

Una decisión llega aquí cuando **había más de una respuesta razonable** y elegir una cierra
puertas: una dependencia que entra o no entra, un formato que otros van a tener que leer, una
frontera que se pone en un sitio y no en otro. Lo que solo tiene una forma sensata de hacerse
no es una decisión — es código, y se explica donde está escrito.

Cada documento dice **qué se aceptó a cambio**. Un registro que solo guarda los motivos a
favor no es un registro: es una colección de justificaciones.

| | Decide | Estado |
|---|---|---|
| [0001](0001-parser-de-yaml.md) · parser de YAML | leer YAML con un analizador propio, y por qué el ecosistema no servía | aceptado |
| [0002](0002-sin-validador-de-json-schema.md) · sin validador de JSON Schema | los esquemas se publican, no se ejecutan; manda el diagnóstico semántico | aceptado |
| [0003](0003-lectura-estructural-de-cedar.md) · lectura estructural de Cedar | leer la **forma** de una política, no evaluar su semántica | aceptado |
| [0004](0004-distribucion-del-binario.md) · distribución del binario | determinismo comprobado, procedencia atestada, binarios crudos | aceptado |
| [0005](0005-la-superficie-de-contexto.md) · la superficie de contexto | qué sirve `ore dev` por MCP, y qué no toca | aceptado |
| [0006](0006-el-artefacto-de-topologia.md) · el artefacto de topología | CSR sellado contra el digest del bundle | aceptado · **sin quien lo escriba** |
| [0007](0007-enlazar-el-evaluador-de-cedar.md) · enlazar el evaluador de Cedar | el ejecutor enlaza `cedar-policy`; el compilador **no** | **sin implementación** |
| [0008](0008-el-protocolo-del-driver.md) · el protocolo del driver | la petición es un fragmento del plan, y traducir es del driver | aceptado |
| [0009](0009-que-se-distribuye.md) · qué se distribuye | se publica el compilador; el ejecutor se construye desde la fuente | aceptado |
| [0010](0010-el-refresco-sustituye.md) · el refresco sustituye | una fila es el conjunto de aristas de su clave, y la marca avanza | aceptado |
| [0011](0011-el-informe-no-lista-incumplimientos.md) · el informe no lista incumplimientos | `ore report` atribuye; el compilador es quien rechaza | aceptado |
| [0012](0012-el-estado-es-parcial-y-vive-en-el-cliente.md) · el estado es parcial y vive en el cliente | el mantenimiento incremental recuerda por clave, en el almacenamiento del cliente; un *miss* es un plan | aceptado |
| [0013](0013-el-protocolo-del-mantenedor.md) · el protocolo del mantenedor | mantener es una **sesión** por stdin, y el dictamen de coste no se obedece a sí mismo | aceptado |
| [0014](0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md) · no se mide el tiempo, se cuenta el trabajo | la unidad de coste es **una fila mirada por un operador**; y medir destapó que la incrementalización estaba escrita y no ocurría | aceptado |
| [0015](0015-el-protocolo-del-almacen.md) · el protocolo del almacén | una copia es un **artefacto nombrado por su digest** —sobre nuestro, carga Parquet— y subirlo es de un programa delegado | aceptado |
| [0016](0016-el-testigo-y-el-rango.md) · el testigo y el rango | preguntarle al origen **hasta dónde está**, y poder pedirle las filas **de un rango** | **A aceptada · B propuesta** |
| [0017](0017-la-escritura-sobre-el-sustrato.md) · la escritura sobre el sustrato | una copia nueva **reserva a su base**, y se reescribe entera hasta que una medida diga lo contrario | **propuesto** |
| [0018](0018-la-ontologia-es-el-sistema-de-registro.md) · la ontología es el sistema de registro | una escritura de la ontología aterriza **en la copia**, nunca en el origen; y escribir obliga a materializar | **aceptado** |
| [0019](0019-un-cambio-es-un-orden-o-una-identidad.md) · un cambio es un orden o una identidad | de dónde sale un código de compatibilidad: un movimiento en un **orden** —con dirección, y por eso con espejo— o una **sustitución** de identidad, que no la tiene. Y el eje es **un público**, no una categoría | **aceptado** |
| [0020](0020-el-plano-de-control.md) · el plano de control | atender a un cliente es de un **programa delegado**, con una lista de verbos **permitidos** y sin identidad no hay superficie | **aceptado** |
| [0021](0021-una-persona-en-varias-organizaciones.md) · una persona en varias organizaciones | la identidad no lleva dentro a qué cliente sirve; la organización viaja **en el camino**, nunca en el token | **aceptado** |
| [0022](0022-el-inquilino-es-un-repositorio.md) · el inquilino es un repositorio | aprovisionar **se escribe, no se aplica**; la unidad es el inquilino y no el clúster; y quién aprueba el alta es un **ajuste de cada cliente** | **aceptado** |
| [0023](0023-donde-vive-un-secreto.md) · dónde vive un secreto | `iam` dice **quién puede** y no guarda nada; el material va **cifrado de sobre** con la llave fuera; quién emite queda `owner`; y **`usar` no es `leer`** | **aceptado** |
| [0024](0024-donde-corre-el-inquilino.md) · dónde corre el inquilino | un plano de control y el árbol en **uno de tres sitios**; el modelo **con el árbol, en otro pool**; la ingesta aterriza **en el inquilino**; el clúster es **un hecho del plano de control**; y desde la E3, **el material y la puerta viven en la celda** — el plano de control guarda quién puede y tiene una puerta para que tiren de ella | **aceptado** |
| [0025](0025-la-celda-tiene-nombre.md) · la celda tiene nombre | **la organización es la cuenta** y **la celda es donde vive un árbol**; una organización tiene **N celdas** con nombre propio, `arbol` y `entrada` se van con la celda, un secreto es de la celda, y la consola **elige celda**; dos verbos —fundar la cuenta, pedir otra celda— y el aprovisionador **con identidad**, que informa `aprovisionada` | **aceptado** |
| [0026](0026-la-celda-informa.md) · la celda informa | el estado vivo de una celda —cuota usada, jobs, salud del control— **lo observa un informador que vive en la celda** con identidad de agente y `Role` de sólo lectura en su namespace, y **lo empuja a `ore-iam`**, que guarda **el último snapshot** y lo devuelve en `GET /celdas`; la overview del clúster se pinta **desde el plano de control**; `ore-serve` no gana ni red ni privilegios | **aceptado** |
| [0027](0027-el-modelo-vive-en-el-arbol.md) · el modelo vive en el árbol | un modelo que una celda usa es **un documento del árbol** (`kind: Model`) que nombra **un perfil certificado** y **un tier**; en compartido deriva a **una suscripción** que el gateway respeta (un pod por modelo, multi-tenencia lógica), en dedicado a **un despliegue** con una cuenta que sólo puede desplegar; los pesos **no pasan por el árbol** (puntero y digest dentro, bytes fuera); la celda alcanza el modelo **sólo por el gateway**; el estado lo trae quien sirve; cierra la E6 de `0024`. Revisado sobre `0028` | **propuesto** |
| [0028](0028-bastion-es-el-producto-sobre-el-stack.md) · Bastion es el producto sobre el stack | la inferencia **no es un motor propio**: es la capa que hace soberano, multi-tenant y gestionado el stack que ya existe (vLLM sobre RTX PRO 6000 GDDR7); la optimización vive **abajo** y Bastion la certifica y la envuelve; el motor propio queda **congelado como referencia**; cada máquina lleva **una etiqueta de soberanía** que el gateway respeta. Medido: 587 tok/s = **2.8 $/M** a 32 usuarios con 7× menos capex | **aceptado** |
| [0029](0029-donde-corre-una-funcion.md) · dónde corre una función | el código de una `Function` corre **en la celda del inquilino** (pool privado, identidad del agente, red cerrada + WASI sin sockets), nunca en `ore-serve` ni en un servicio central; **dos modos con un delegado**: bajo demanda (un `Job` por invocación desde la cola, la figura que existe) y residente (`ore-invoke` con sesión, cuando se mida); lee y escribe **la copia**, nunca el origen; el orden hasta producción: P1 → línea base → F4 → F2·F3 → F5 → verbos de Forge → F6 → residente | **propuesto** |
| [0030](0030-el-arbol-en-el-editor.md) · el árbol en el editor | la **superficie principal** es el code workspace sobre el árbol de la celda (git como memoria, `ore` como compilador, la Forge como motor); cuatro peldaños: W0 el árbol en el editor ✓ · W1 ejecutar la pregunta ✓ · W2 proponer ✓ (ramas, propuestas, commit sin Save, *Commit anyway*: el árbol es de quien lo escribe) · W3 la sesión viva | **propuesto** · W0–W2 hechos |
| [0031](0031-el-puesto.md) · el puesto | W3 es **el sustrato de ejecución** de todo lo que no es YAML: una unidad —el puesto: imagen + identidad + datos + recursos— con dos vidas (sesión de un nodo con TTL, trabajo encolado); un agente en el pod y el código habla con el SDK; imágenes base **numeradas** + capa por paquete resuelta en CI; datos por alias con fallback de rama y salidas con nombre; `Model` como documento con pesos en el bucket; red cerrada con *restricted outputs*; sesiones con prioridad alta y sin tomar prestado | **propuesto** · W3.1–W3.7 hechos, **con el gobierno de lo escrito** (medido, decidido en cinco reglas y construido en seis pasos): desde un puesto sólo entran los verbos, la lectura es un conducto, la clasificación baja por `derivedFrom`, lo escrito es de quien lo escribió y el puesto nace en su rama, y lo declarado lo conoce el servidor |
| [0032](0032-el-contrato-de-tipos.md) · el contrato de tipos | un valor tiene tipo en **cuatro sitios** —origen, árbol, copia, celda— y el contrato es la tabla que los une; la copia deja de ser todo texto (218/218 columnas `string` medidas) y estrecha el escalar de OOS a Arrow (revisa 0015: `Decimal` exacto, `DateTimeTz` en UTC); el escalar llega al árbol (`Table.columns.<c>.type`); la celda ve columnas (Arrow) y la consola un solo JSON; lo que no sobrevive se dice | **borrador** · medido |
| [0033](0033-el-dataset.md) · el dataset | lo que tiene bytes en el lago se nombra con **un** documento, `kind: Dataset` (OOS v1alpha12), que **absorbe el plan** de la vista que lo produce —la costura del gobierno y el refresco se mueven, no se pierden— y lo que `write()` deja; `View` vuelve a ser sólo la pregunta, `Entity` se respalda en View o Dataset, la `Table` no cambia; revisa 0031 §10 (que decidió no hacerlo kind) y llega en dos tiempos: el catálogo lo enseña ya, OOS lo fija medido | **propuesto** · sin medir |
| [0034](0034-el-catalogo-de-assets.md) · el catálogo de assets | el Assets Catalog **es el registro del inquilino y el registro es el árbol**: lee, no autora; los **ítems** (Dataset, Table, View, Entity, Interface, Concept, Function, Action, modelo entrenado y servido) y las **capas** que se ven encima (access, rules, links, history); database = paquete, schema = carpeta del paquete; sólo lo respaldado. Resuelto ⑤: el catálogo es **el índice de assets compilado del árbol a una cabeza** (`GET /assets`: ítems `kind:ns.name`, relaciones en las dos direcciones, acceso, puntero, carpeta), que ore-serve deriva con ore-core y sirve de memoria por commit | **hecho** · `GET /assets`: 815 ms / 102 ms (demo), 1 362 / 50 ms (victor) frío / caliente, una llamada; la consola lo lee (rubix-platform e03128b, local) |
| [0035](0035-el-proyecto.md) · el proyecto | **el alcance que falta** entre la celda y el asset: Projects es el entorno donde el cliente construye sobre sus datos (código, pipelines, linaje, mapas) y hoy no persiste porque el concepto no existe; la pregunta es si es un **segundo registro o una segunda vista del mismo árbol** | **resuelto** · un proyecto es un propósito con dueño, un sitio en el árbol (`proyectos/<n>/README.md`, que **nombra** lo suyo en vez de contenerlo) y unas ramas donde se trabaja: **una lente, no una caja**; organiza y atribuye, no gobierna; un conjunto de proyectos es un **atlas**, no una partición. Medido antes: 5 de los 6 campos son del árbol (sólo `collaborators` no), una carpeta del paquete compila y el índice ya la nombra, un `kind: Project` exige tocar OOS, y lo que hoy NO es del proyecto es la rama, la propuesta y el «compila» (del árbol entero); el alcance costaría 38 rutas / 76 ficheros / 33 llamadas; el aislamiento por proyecto es **cero en cinco planos** (compilar, gobernar, nombrar, ejecutar, leer) y cotejado con Foundry, que lo compra pagando la unidad del registro. **⓪ hecho**: la forma medida sobre demo y victor — el manifiesto es invisible (`validate` 0, el índice igual), roto no rompe el árbol, y hoy los 17 y los 58 ítems quedan fuera de todo proyecto. **① hecho**: `ore_core::proyectos` y el índice con `proyectos` en la raíz y en cada ítem (en plural: se solapan); lo roto se lista con su porqué y no alcanza nada. **② hecho**: `POST/PUT/DELETE /proyectos` (leerlos ya va en `/assets`), el `id` sale del título, el nombre repetido es 409, lo que no resuelve entra y se dice, borrar se lleva la lente y NO lo que nombraba, y desde un puesto es 403. **③a/③b hechos**: Projects lee del índice (sin lectura propia), «Collaborators» se va y en su sitio van los ítems, el modal pregunta qué nombra, borrar dice lo que NO se borra, y `DELETE /arbol` aprende carpetas (una llamada, un commit, y dice qué ficheros se llevó). **⑥**: el REPOSITORIO es la unidad de trabajo —una carpeta con `nombre` y `plantilla` en su README— y por eso hay que persistir la instancia: hoy el workspace es un entorno único sobre el árbol entero (el editor ve 24 ficheros de la celda y 1 del repo, dos repos de la misma persona comparten puesto y `/propuestas` no filtra por ruta); acotar son tres cosas baratas, y la jerarquía queda organización → celda → proyecto → repositorio |
| [0036](0036-la-clase-del-repositorio.md) · la clase del repositorio | **productos dedicados sobre un solo árbol**: si cada clase de repositorio (transforms, analytics, models, functions, semantics) es un producto distinto —entorno, capacidades e interfaz varían—, ¿dónde vive cada variación y cómo se fija la partición desde el primer momento? | **resuelto** · una instancia son **tres cosas y dos se derivan**: el SITIO (la carpeta, que es la CLAVE DE PARTICIÓN: todo lo acotado toma la ruta como parámetro), la CLASE (`plantilla` + `plantillaVersion`, una clave a una tabla del producto, no una configuración) y el NOMBRE. La configuración que ya tiene sitio se queda donde está (`pyproject.toml` del repo) y lo único que cambia es dónde se mira: **la capa deja de ser la unión del árbol** —hoy el `torch` de un repo de modelos lastraría TODAS las sesiones de la celda— y pasa a ser la del repositorio. La clase **ajusta hacia abajo y nunca concede** (el techo se aplica donde ya se aplica el gobierno: la puerta del agente y `@transform`), se versiona y se actualiza por propuesta —que es lo que la columna «UPGRADE · Up to date» significa de verdad—, y la interfaz se resuelve por la misma clave en la consola. Cotejado con Foundry (los repos tienen TIPO, con features distintas, y el producto genera PRs de actualización de la plantilla), Backstage (`catalog-info.yaml` con `spec.type` dentro del repo + scaffolder) y Dev Containers (config junto al código y lock de versiones); lo que no copiamos es su repositorio git por cosa: aquí la partición es **por prefijo**. **① hecho**: `ore_core::repositorios` + `manifiesto.rs` (el encabezado, compartido con los proyectos) y el índice con `repositorios[]` y `repositorio` en SINGULAR —el más hondo se queda el ítem: anidar es hondura, no solape—; sólo `plantilla` hace un repositorio, el README del paquete no lo es, y lo roto se lista sin quedarse nada. **② hecho**: `clases.rs` (la tabla del producto: las cinco con su versión y su semilla) y `POST/PUT /repositorios` — nace ENTERO (manifiesto + semilla + el `contiene` del proyecto, en UN commit), 409 si esa carpeta ya es uno, 422 con las clases que hay, el PUT conserva la prosa, y borrar sigue siendo `DELETE /arbol/<carpeta>` |

**0027 y 0028 son el mismo objeto visto desde los dos lados**: 0027 dice dónde vive un modelo
desplegado en la plataforma —un documento del árbol que Flux aplica en la celda—; 0028 dice qué
lo sirve y por qué no es un motor propio: vLLM certificado sobre GDDR7, con la etiqueta de
soberanía de la máquina viajando hasta el gateway. Si alguna vez la E5 de 0027 elige otra GPU u
otro motor, se abre 0028.

**0016 y 0027 son las propuestas, y el estado es deliberado**: las demás se escribieron después
de construir lo que decidían. 0016 se escribe antes porque toca un protocolo con tres
implementaciones; 0027, porque abre una cuenta y un repositorio nuevos por celda y su E0 es una
medida a mano que puede cambiar la forma. Es también la única que decide **mirando fuera**: sus cinco preguntas abiertas
se contestaron leyendo lo que Debezium, Iceberg, Delta, Snowflake, BigQuery y Airbyte tienen
escrito, y los seis coinciden. Una de esas lecturas —que el modo `field` es *at-least-once* por
construcción— cambió la propuesta en vez de confirmarla.

**0017 y 0018 se leen en este orden y no al revés**: el 0018 decide **qué** aterriza en la copia
—las escrituras de la ontología, y solo ellas— y el 0017 ya había decidido **cómo** sin saber que
iba a hacer falta para esto. Su recibo de sucesión deja de ser una precaución y pasa a ser el
mecanismo de `F5`.

**0015, 0016 y 0017 son las tres preguntas de una copia**: qué es y cómo se nombra; hasta cuándo
fue cierta; y qué pasa cuando dos quieren cambiarla a la vez. La tercera salió de que la primera
celebrase que *«la carrera es inofensiva»* — cierto para leer, **falso en cuanto los escritores
producen cosas distintas**.

**0008, 0015 y 0016 son la misma costura creciendo**: la petición del driver era un fragmento del
plan; el almacén añadió el artefacto; y esto añade lo único que ninguno de los dos podía contestar
— **hasta cuándo era cierto lo que se copió**. Sin ella el ciclo puebla una vez y no mantiene.

**0013 y 0014 salieron del mismo trabajo**: construir el ejecutor delegado dejó por primera vez
un sitio donde el circuito corre, y eso hizo posible medirlo — que fue lo que destapó que dos
integradores no estaban indexados.

**0006, 0008, 0013 y 0015 son la misma frontera puesta cuatro veces**: `ore` no abre sockets, así
que todo lo que toca el mundo —leer un origen, correr el circuito Δ, subir una copia— es un
programa delegado, y lo que viaja entre ellos es **un fragmento del plan o el artefacto**, nunca
una llamada a un sistema concreto. Si alguna vez se relaja una, hay que abrir las cuatro.

> **Y esa frase se quedó corta.** Si son el mismo artefacto, el índice de topología **es una
> vista materializada** y no una pieza aparte con su propio productor, su propio formato y su
> propia marca de agua. Se escribió aquí antes de tener vistas y nadie le sacó la consecuencia; al
> retirar `ore-exec` —que era su único productor— se quedó **definida y sin poblar**, que es
> exactamente el estado que obliga a sacársela.

**0006 y 0015 son el mismo artefacto con dos cargas**: aristas en CSR y filas en Parquet, con el
mismo sobre sellado contra el digest del bundle. La segunda salió de mirar la primera y ver que
la topología ya era una vista materializada escrita a mano.

**0008 y 0013 son el mismo protocolo con y sin memoria**: el driver es una función —entra una
petición, sale una respuesta, el proceso muere— y el mantenedor es una sesión, porque el estado
de una junta no cabe en una petición. El transporte es el mismo a propósito.

**0006 y 0012 se leen juntas**: la primera dice que ORE no opera ninguna base de datos; la
segunda, dónde vive entonces lo que el mantenimiento incremental tiene que recordar, y por qué
eso no contradice a la primera.

**0007 y 0009 son la misma decisión mirada en dos momentos**: la primera saca el evaluador del
compilador, la segunda decide qué pasa con lo que quedó fuera el día de publicar. Si alguna
vez se revisa una, hay que abrir la otra.

**0019 explica el registro entero de `OOS5xxx`, incluido lo que ya estaba**: por qué unos códigos
tienen espejo y otros no —lo tienen los órdenes cuyas dos direcciones se observan— y por qué los
seis de sustitución son justo los del plano físico. Se escribió al meter el sustrato en `ore diff`,
y lo primero que dijo es que eso **no era extender `diff` sino repararlo**: el eje `INDEX` existe
desde v1alpha1 y v1alpha8 le movió el sujeto de debajo.

**0008, 0013, 0015 y 0020 son la misma frontera cuatro veces**: `ore` no puede abrir un socket, así
que leer un origen, correr el circuito Δ, subir un artefacto y atender a un cliente viven fuera. Las
tres primeras sacaban del compilador algo que tenía que salir; la cuarta lo saca **para que el
proceso que mira a internet no sea el que decide qué significan las cosas**, que es la propiedad y
no el efecto secundario.

**0018 y 0022 son el mismo argumento en dos planos**: la ontología es el sistema de registro de lo que los datos significan, y el repositorio del inquilino lo es de la forma de su compartimento. Los dos compran lo mismo — historia, firma, revisión y `revert`— y los dos lo compran **no programándolo**. La segunda añade el motivo que la primera no necesitaba: quien pudiera aplicar el compartimento de un inquilino podría leer el de todos.

**0011, 0018 y 0023 dicen la misma frase en tres sitios**: quién AFIRMA algo y quién lo GUARDA no son la misma pieza. El informe atribuye y el compilador rechaza; la ontología es el sistema de registro y el origen no; `iam` dice quién puede y el material vive en otro sitio con la llave en un tercero. La última añade el corolario que las otras no necesitaban: **un segundo motor de autorización es una segunda respuesta a la misma pregunta**, y por eso Vault entra como custodio o no entra.
