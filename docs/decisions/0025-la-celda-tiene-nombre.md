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

## El abordaje, por etapas

Cada etapa deja el sistema mejor aunque la siguiente no llegue, y **`demo` no se entera de
ninguna**.

**E1 · La celda con nombre.** Migración 029: `iam.celda` gana `cluster`, `arbol`, `entrada`;
`nombre` pasa a ser el de la celda (`demo`, `prueba`: sembradas con el nombre de su organización
y su árbol y entrada de hoy); el índice único fuera; `celda_de` por nombre de celda. `fundar`
escribe la celda entera; `GET /celdas` la devuelve entera. Los siete lectores leen de la celda.
`cofre.secreto` gana `celda` (sembrado: la única de su organización) y su unicidad cambia.

**E2 · Por celda.** El aprovisionador y el renderizador toman la celda; el CronJob recorre
`iam.celda_de`. Y sus dos arreglos: salida a `t-*:3000` e `identidad:8080`, y el admin del IdP en
el almacén, para que la pasada 2 y el ⑦ corran dentro. Cadencia `*/5`.

**E3 · La identidad del aprovisionador.** Cliente `ore-aprovisionador`, los dos verbos en
`ore-iam`, `aprovisionada` informada. El Job de operador `ore-iam agente` deja de hacer falta.

**E4 · Los dos verbos y la consola.** `POST /organizaciones` y `POST /organizaciones/{org}/celdas`;
`celdaActual` y el selector en Clusters; el onboarding y el botón de *Nuevo* dejan de ser toasts.
Aquí `demo` pide su segunda serverless, y es la prueba de todo lo anterior.
