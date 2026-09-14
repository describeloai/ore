# 0025 · La celda tiene nombre

**Estado:** propuesto · **Fecha:** 2026-09-14 · **Decide:** que **la organización es la cuenta**
—personas, llave, agente, concesiones, factura— y **la celda es donde vive un árbol**; que una
organización tiene **N celdas** con nombre propio; que `arbol` y `entrada` **se van con la celda**;
que un secreto del cofre **es de la celda**; y que la consola **elige celda**, no la deduce

---

## El problema

En Redpanda una organización tiene dos clusters serverless en dos regiones sin que la organización
se duplique. En ORE una organización tiene **una** celda, y no por decisión: por una confusión de
nombres. Medido en `pruebas-de-fuego/medida-la-celda-con-nombre.py`:

```
iam.organizacion   id nombre estado creada_en creada_por   ← la CUENTA
                   arbol entrada                           ← de la CELDA, en la tabla equivocada
                   kek                                     ← la CUENTA: una llave cifra lo de todas sus celdas
iam.celda          nombre = «ore-mesh»  ← el CLUSTER, no la celda · UNA por organización (índice único)
```

La celda **no tiene nombre propio: usa el de la organización**. Namespace `t-demo`, árbol
`t-demo/ontologia`, entrada `demo.ore.paladio.io`, cuentas `ore-cofre-demo`, forja `t-demo`, cola
`cq-demo` — 80 usos de `$NOMBRE` en el aprovisionador, y todos son de la celda salvo tres (la
llave, el agente, `--organizacion`). Un segundo serverless de `demo` no está prohibido más que por
una línea (`celda_una_por_organizacion`); en todo lo demás **colisionaría** con el primero pieza a
pieza, y el aprovisionador diría «ya estaba» en cada paso.

Es la sexta vez de la misma figura —`50-jwks`, `017`, `019`, `022`, `027`—: **nombramos la
instancia (`demo`) donde queríamos nombrar la clase (una celda de `demo`)**.

---

## Lo que se miró antes de decidir

- **Redpanda** (documentación, 2026-09-14): la organización *«contains all of its resources,
  including clusters, users, service accounts, resource groups, networks»*; el plan y la factura
  son de la organización; los *role bindings* se atan a la organización, a un *resource group* o a
  un cluster. Al registrarte te crea una organización con nombre autogenerado
  (`redpanda-org-sl1yl6`) y un cluster `welcome`. La organización **sobrevive a sus miembros**: es
  quien tiene los clusters y la factura; las personas entran y salen.
- **Lo nuestro ya es eso en la mitad de la cuenta**: `iam.organizacion` + `pertenencia`, `fundar`
  crea la organización con su fundador como `owner`, y la `021` deja que una persona esté en
  varias. Lo que no es como Redpanda es que la organización lleva `arbol` y `entrada`.
- **Lo que se queda en la cuenta sin chocar entre celdas**: la llave `ore/<org>` (CMEK por secreto,
  vale para todas sus celdas), el agente `ore-agente-<org>` (`iam.agente.organizacion`: los Jobs de
  cualquier celda lo usan), las concesiones y potestades (quién puede usar un secreto no depende de
  en qué celda corre el Job), y el prefijo del almacén `t-<ns>-cofre-*`, que ya es del namespace.
- **Lo único de la cuenta que la celda empuja**: `cofre.secreto` es único por
  `(organizacion, nombre)` —confirmado en la base—. Dos celdas de `demo` con una fuente `pg` cada
  una chocarían en el nombre.
- **Lo que le pasa a `demo`**: nada. Todos sus nombres técnicos ya son `demo`; si su primera celda
  se llama `demo` y hereda el árbol y la entrada, ni un manifiesto se re-rinde, ni una cuenta se
  crea, ni un DNS se toca. La `022` dijo «el valor por defecto se deriva; el valor no» — y por eso
  la fila puede mudarse de tabla sin que el mundo lo note.

---

## La decisión

> ### ① La organización es la cuenta. La celda es donde vive un árbol.

```
organización   personas · pertenencia · roles · llave · agente · concesiones · potestades · factura
celda          nombre · cluster · tier · región · puerta · árbol · entrada · cofre · Jobs
```

Una organización tiene **N celdas**. Una celda tiene **una** organización. La celda es la unidad
técnica —el namespace, la forja, el `ore-serve`, el cofre— y la organización es quien puede y quien
paga. Es lo que Redpanda separa en *Organization IAM* y *Clusters*, con las mismas dos palabras.

> ### ② La celda tiene nombre propio, y el nombre es el namespace.

`iam.celda.nombre` pasa a ser el nombre de la celda —lo elige quien la pide, con el alfabeto de la
`017`—, y **de él se deriva todo lo técnico**: `t-<celda>`, `t-<celda>/ontologia`,
`<celda>.ore.paladio.io`, `ore-cofre-<celda>`, `cq-<celda>`, `t-<celda>-cofre-*`. El clúster
donde corre pasa a una columna aparte, `cluster` (`ore-mesh`), que es carretera.

`arbol` y `entrada` **se van de `iam.organizacion` a `iam.celda`**, con sus lectores: `fundar`,
`GET /organizaciones`, el aprovisionador, el grant de la `023` y `entradaActual` en la consola. El
índice `celda_una_por_organizacion` **se retira**; en su lugar, `(organizacion, nombre)` único y
`nombre` único global — porque el namespace y el DNS son globales.

> ### ③ El aprovisionador y el renderizador trabajan por celda.

`aprovisionar-inquilino.sh <celda>`: lee la celda, y de ella la organización — para la llave, el
agente y `--organizacion`. `gen-inquilino.py` rinde por el nombre de la celda. **Es un parámetro,
no una reescritura**: ya sustituían por un nombre; cambia de cuál.

> ### ④ Un secreto del cofre es de la celda.

`cofre.secreto` gana `celda` y su unicidad pasa a `(celda, nombre)`. Quién puede **usarlo** sigue
siendo de la organización (`iam.concesion`), porque la potestad es de la cuenta; **dónde está** es
de la celda, porque el material vive en su almacén (0024-⑤) bajo su prefijo. Una fuente `pg` en dos
celdas son dos secretos con dos materiales y una misma regla de quién puede.

> ### ⑤ La consola elige celda.

Hoy `entradaActual` deduce **la** dirección del árbol de la organización, y todo el plano `arbol`
de `query.ts` va a ella. Con N celdas eso es una mentira por omisión. La consola pasa a tener
**`celdaActual`** —una cookie, como la sesión; la ruta cuando la celda va en la URL— y el plano
`arbol` se resuelve por ella. Sources, Catalog y la sonda son **de la celda que miras**, y la vista
de Clusters es donde se elige, como en Redpanda al entrar en un cluster. Con una sola celda no se
pregunta: se entra.

> ### ⑥ Dos verbos, dos momentos.

```
POST /organizaciones                {nombre}          funda la CUENTA y su primera celda (onboarding)
POST /organizaciones/{org}/celdas   {nombre, tier}    pide OTRA celda a una cuenta que ya existe
```

El primero es lo que hoy hace `fundar` como Job de operador, por HTTP y con **quien pide como
dueño** — el token ya trae emisor, `sub` y correo; sólo falta el nombre. El segundo es el botón de
*Clusters → Nuevo*: `compartido` da una celda más en `ore-mesh`; `dedicado` y `byoc` son E4/E5 de
la 0024 y contestan lo que puedan hasta entonces. Los dos escriben `iam.celda` con `estado
activa` y **sin `aprovisionada`**: eso lo informa el aprovisionador cuando termina (0025-⑦), y la
consola pinta *Provisioning* hasta que lo haga.

> ### ⑦ El aprovisionador tiene identidad, y con ella informa.

Lo que la medida del verbo destapó: desde dentro, el CronJob **no alcanza las forjas de los
inquilinos ni el IdP**, así que la pasada 2 y el paso ⑦ sólo han corrido desde fuera; y el agente
se registra con un Job de operador porque «quien crea el cliente no puede escribir `iam`, y quien
escribe `iam` no sabe el `sub` hasta que el cliente existe». Lo que hace el sector: **identidad de
máquina con credenciales cortas, una por carga**, y el acceso directo a la base como «la última
excepción del zero-trust». La `023` acertó al no darle SQL; el camino es HTTP:

- el aprovisionador es un cliente del IdP (`ore-aprovisionador`, claim `rubix_tipo=aprovisionador`)
  y `ore-iam` acepta de esa clase, y sólo de ella, dos verbos: `POST
  /organizaciones/{org}/agentes` (el agente, con el `sub` que acaba de crear) y `POST
  /celdas/{celda}/aprovisionada` (la pasada, la fecha, el detalle). Con huella, como todo.
- `iam.celda.aprovisionada` es el `status` que el reconciliador informa; `estado` es el `spec`
  administrativo. `aprovisionando` como estado **se retira**: era `status` disfrazado de `spec`.

---

## Lo que se acepta a cambio

- ⚠️ **Una migración que mueve columnas**, no sólo las crea: `arbol` y `entrada` cambian de tabla y
  siete lectores cambian de consulta el mismo día. Se hace en una, con la 029, y con `demo` y
  `prueba` como prueba de que nada cambia de nombre.
- ⚠️ **El aprovisionador cambia de argumento**: de organización a celda. El CronJob recorre celdas,
  no organizaciones. `converger-inquilinos.sh` se llama como se llama, y hace lo mismo por celda.
- ⛔ **La consola gana un estado**: la celda que miras. Es una cookie más y una pregunta más al
  entrar; con una celda no se nota, con dos es la diferencia entre ver el árbol correcto y otro.
- ⚠️ **Quién puede pedir una celda** es una decisión de producto que esta ADR deja **en el
  `owner` de la organización, y una serverless por organización de entrada**; el segundo y
  siguientes, cuando haya factura. El número es configuración de plataforma, no una constante.
- ⚠️ **Un nombre de celda es global** (namespace, DNS, cuentas de Google). Dos organizaciones no
  pueden tener las dos una celda `ventas`. Se dice al pedirla, con un 409 y la frase.

---

## El abordaje — y por qué es así y no de golpe

La visión es una tabla; el camino es lo que puede salir mal. Lo que hace robusto este abordaje
no es el orden de las etapas sino **cuatro reglas que valen en todas**:

**R1 · Ensanchar y recortar, nunca mover.** Una columna no cambia de tabla: **se duplica** en la
nueva, se rellena, los lectores cambian **uno a uno con medida**, y sólo cuando la medida dice
«cero lectores» se recorta la vieja. Entre medias las dos existen y **dicen lo mismo**, y una
comprobación lo exige. Es el patrón *expand/contract*; su precio es una etapa más, y su ganancia
es que en ningún momento hay un despliegue a medias que no pueda volver atrás.

**R2 · `demo` y `prueba` son los canarios, y no se enteran.** Cada etapa se comprueba contra los
dos inquilinos vivos con la misma frase: *ni un nombre técnico cambia, ni un manifiesto se
re-rinde, ni una cuenta se crea, ni un DNS se toca*. Lo mide un guion antes y después, y la
igualdad de los dos resultados es la puerta de la etapa. Y `prueba` va **siempre primero**.

**R3 · Cada etapa tiene entrada, salida y vuelta atrás escritas antes de empezar.** La entrada es
una medida que hoy da rojo; la salida es la misma medida en verde y las tres comprobaciones de CI
en verde; la vuelta atrás es **una** orden que se ha probado en `prueba` antes de tocar `demo`.
Una etapa sin vuelta atrás probada no empieza.

**R4 · Lo que sea acto, es de un reconciliador idempotente; lo que sea verdad, es una fila.**
Nada del camino se hace «a mano y luego se escribe»: si un paso lo hace una persona con `kubectl`
o `gcloud`, es un paso que la segunda celda va a necesitar y nadie va a recordar. Se hace por el
aprovisionador, con «ya estaba», o no se hace.

Y **una medida de invariantes** —`medida-la-celda-tiene-nombre.py`, la guarda— que se corre en
cada puerta y dice, para cada organización viva: sus celdas; que cada celda tiene *exactamente
una* de cada cosa técnica (namespace, forja, cofre, árbol, entrada, puerta, cola); que la
organización y la celda dicen lo mismo mientras las dos columnas existan; que ningún nombre de
celda se repite; y que `demo.ore.paladio.io` y `prueba.ore.paladio.io` contestan `200`.

---

### E0 · La guarda, y la copia — ✓ 2026-09-14

Antes de tocar nada: la medida de invariantes escrita y en verde con el modelo de hoy; una copia
de la base de `iam` (`pg_dump`) al bucket de copias con fecha, y su restauración **probada** en una
base vacía — la misma disciplina que la `31` exige a la forja. Sin E0 no hay vuelta atrás de E1.

### E1 · Ensanchar: la celda con nombre, sin quitar nada — ✓ 2026-09-14 (029; guarda igual antes y después salvo el modelo; `discrepancias` = 0)

Migración 029, **sólo añade**: `iam.celda` gana `cluster` (rellenado con el `nombre` de hoy),
`arbol` y `entrada` (rellenados desde la organización), y `nombre` pasa a ser el de la celda
(rellenado con el nombre de la organización: `demo`, `prueba`). El índice
`celda_una_por_organizacion` se queda **todavía**. Y una vista `iam.discrepancias`, que la guarda
lee, dice si alguna organización y su celda **no dicen lo mismo**. `cofre.secreto` gana `celda`,
rellenada con la única celda de su organización; la unicidad vieja se queda.

- entrada: la guarda dice «la celda no tiene nombre propio»
- salida: `iam.celda` con las tres columnas, `iam.discrepancias` vacía, `demo`/`prueba` iguales
  antes y después, `los-verbos.sh` con un caso nuevo: fundar escribe la celda entera
- vuelta atrás: `alter table … drop column` × 4 y la columna `celda` de `cofre.secreto`. Una orden,
  probada en la base restaurada de E0.

### E2 · Cambiar los lectores, uno a uno, con la doble verdad en pie — ✓ 2026-09-14 (guarda: 0 lectores)

Cada lector pasa de `organizacion.arbol/entrada` a `celda.arbol/entrada` **en su propio commit**
y con su propia prueba: `fundar` y `GET /organizaciones` (`los-verbos.sh`); el aprovisionador
(`consulta` lee de `celda_de`, que ya existe: `--seco` contra `demo` da lo mismo que ayer); el
grant de la `023`; y en la consola `entradaActual` → `celdaActual` con **una** celda todavía, así
que devuelve lo mismo. La guarda cuenta lectores de la columna vieja en el código, y la etapa acaba
cuando cuenta **cero**.

- vuelta atrás: `git revert` del commit del lector; la columna vieja sigue ahí y sigue diciendo lo
  mismo.

### E3 · Recortar — ✓ 2026-09-14 (030 y 031, en dos: la 030 quita el `not null` y el índice; la 031 borra cuando el `fundar` nuevo ya corre)

Migración 030: `organizacion.arbol` y `organizacion.entrada` fuera; el índice único fuera;
`(organizacion, nombre)` único y `nombre` único global en `iam.celda`; `cofre.secreto` único por
`(celda, nombre)`. **Sólo** cuando la guarda lleve una etapa entera diciendo «cero lectores».

- vuelta atrás: la copia de E0 más las escrituras desde entonces son **pocas y conocidas** (la
  huella las lista); se re-añaden las columnas desde `iam.celda`. Probada en `prueba`.

### E4 · Por celda: el aprovisionador y el renderizador — ✓ 2026-09-14 (`medida-por-celda.py`: byte a byte contra `20017d4`)

`aprovisionar-inquilino.sh <celda>`; la organización se lee de la celda. El CronJob recorre
`celda_de`. Con `demo` y `prueba` como celdas que se llaman como su organización, **el resultado
rendido es byte a byte el de hoy** — y eso es la puerta: se rinde con el guion viejo y el nuevo y
se comparan. Aquí van también los dos arreglos que la medida del verbo destapó (salida a
`t-*:3000` e `identidad:8080`; el admin del IdP al almacén) y la cadencia `*/5`.

- vuelta atrás: el guion anterior, que sigue en git y sigue aceptando el nombre de la organización.

Lo hecho, y lo que la pasada **desde dentro** destapó al leer por fin su registro:

- `gen-inquilino.py <celda> --organizacion <org>`: lo único que es de la cuenta son
  `ore-serve --organizacion`, `ore init --name` y el mensaje del primer commit; con otra
  organización cambian esas 8 líneas y ninguna más (medido).
- El CronJob **no podía hacer tres de sus pasos y lo decía en cada pasada**: `gcloud projects
  describe` fallaba (el papel no lee el proyecto) y salía un agente del almacén `service-@…`,
  con lo que la CMEK no se concedía; la condición del cofre sobre el proyecto fallaba con
  «Policy modification failed»; ⑦ no tenía admin del IdP. Tres pasadas «al día» con tres
  `ERROR:` cada una. Arreglos: el número del proyecto es una constante; el papel gana
  `projectIamAdmin` **condicionado** a `modifiedGrantsByRole = [secretmanager.admin]`
  (`papel-del-aprovisionador.yaml` dice lo que se concentra); y un usuario `aprovisionador` en
  el realm maestro con **sólo** `manage-clients` de `rubix-dev-realm`, clave en el almacén como
  `idp-admin` (`68-el-admin-del-aprovisionador.sh`) — no `temp-admin`, que es de arranque y de
  todo.
- ⑧ escribe el `CNAME` de cada celda en nuestra zona aunque el comodín ya resolviera: la relación
  queda escrita donde se lee. El papel gana `dns.*` de registros, no de zonas.
- La red, en los dos sentidos: `salida-del-aprovisionador` a `cargas/forja:3000` e
  `identidad:8080`; `entrada-al-idp` admite al pod del aprovisionador. Una regla de salida sin su
  entrada es la mitad de una regla.
- Y la guarda de la E4 mira la **última pasada** y cuenta sus `ERROR:`: un reconciliador que
  acaba en verde con errores dentro es lo que había.
- Medido tras el push: la pasada `aprovisionador-29823225` desde dentro, **2m47s** (era 6m34s:
  el tiempo eran los `gcloud` que fallaban y reintentaban), `0` líneas `ERROR:`, las dos forjas
  vistas, ⑦ resuelto, ⑤ concedido, los dos `CNAME` en `ore-paladio-io`; `demo` y `prueba` 200.
  Faltaba una cosa más que sólo la pasada real dijo: la API de Resource Manager estaba deshabilitada.

### E5 · La identidad del aprovisionador

Cliente `ore-aprovisionador` en el IdP; `ore-iam` acepta `rubix_tipo=aprovisionador` en dos
verbos y en ninguno más; `iam.celda.aprovisionada` informada al final de cada pasada. El Job de
operador `ore-iam agente` **se queda** hasta que una pasada entera de `prueba` haya registrado su
agente por el verbo, con huella. Luego se retira.

- vuelta atrás: quitar el cliente del IdP; los verbos contestan 401 y el Job de operador sigue
  valiendo.

### E6 · Los dos verbos y la consola

`POST /organizaciones` y `POST /organizaciones/{org}/celdas`; `celdaActual` y el selector en
Clusters. **La prueba de aceptación de toda la ADR**: `prueba` pide su segunda serverless desde la
consola, y a los diez minutos tiene `t-<nombre>` con su forja, su cofre, su árbol sembrado, su
puerta en el DNS —escrita por el aprovisionador— y *Running* en Clusters; y `t-prueba` no ha
cambiado ni un byte. Después la segunda se **borra** por el mismo camino, la guarda vuelve a verde,
y sólo entonces `demo`.

- vuelta atrás: los verbos se desmontan; la consola vuelve a los toasts. Ninguna fila queda a
  medias porque cada verbo es una transacción con huella.

---

## Lo que este abordaje NO hace, y por qué

- **No mueve columnas en una migración.** Un `rename` es una etapa sin vuelta atrás con lectores
  en vuelo. Ensanchar y recortar cuesta una etapa más y no cuesta ningún fin de semana.
- **No estrena la segunda celda en `demo`.** La estrena `prueba` — E6 — y sólo cuando todo lo
  anterior lleva una etapa entera en verde.
- **No hace ningún paso a mano.** Los cuatro que hoy lo son (fundar, el agente, la pasada 2, el
  ⑦) son exactamente lo que E4–E6 quitan; hacerlos a mano una vez más sería sumar un quinto.
