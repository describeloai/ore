# ORE · Ontology Runtime Engine

**El motor que convierte un repositorio ontológico en un paquete vivo.**

> Un `Ontology Repository` es texto declarativo en Git: exacto, revisable y **inerte**.
> ORE lo compila, lo coteja, lo firma y lo sirve — gobernado, tipado y en tiempo real —
> a tus aplicaciones y a tus agentes.

Apache-2.0 · implementación de referencia de [**OOS**](https://github.com/oos-dev/oos) ·
**alfa · `v0.1.0`** — el compilador existe y está en verde; el resto está en fases, y el
[estado](#estado) dice cuáles

---

## Qué es, y qué no es

ORE **no es un estándar**. [OOS](https://github.com/oos-dev/oos) lo es; ORE es su
implementación de referencia. Y ORE **no permite versionar tu ontología en Git** — Git ya
versiona.

> **ORE permite ejecutar.** Ese es el hueco que nadie había cubierto, y es todo su motivo
> de existir.

| No es | Eso es |
|---|---|
| un data warehouse ni un motor de almacenamiento | tu almacén de siempre |
| un ETL | tus pipelines de siempre |
| un LLM, ni un proxy de LLM | tu modelo de siempre |
| un catálogo de metadatos | describe; ORE ejecuta |

**Sí es un índice secundario distribuido**: mantiene la topología del grafo localmente y
deja la carga útil en origen. No mueve tus datos a ningún sitio nuevo — mueve las aristas
que los conectan, y solo cuando el repositorio lo declara.

---

## El arco

```
   FUENTE            ARTEFACTO           GRAFO             SUPERFICIE
   Ontology     →    Ontology      →     en memoria   →    MCP · GraphQL · SDK
   Repository        Bundle
   texto en Git      firmado             mmap + índice     lo que consume un agente

   commit SHA        sha256:…            digest + marca    endpoint versionado
                                         de agua
```

Un consumidor nunca ve un repositorio ni un bundle: ve una superficie.
Un operador nunca despliega un repositorio: despliega un bundle.
Un humano nunca edita un bundle: edita la fuente.

---

## Tres caras, tres fronteras de confianza

Bajo un solo binario hay tres productos, y confundirlos es lo que atasca una revisión de
seguridad.

| Cara | Momento | Comandos | Qué toca |
|---|---|---|---|
| **Scaffolder** | autoría | `init` `source add` `discover` `review` `drift-detect` | metadatos de producción y, si se pide, un LLM |
| **Compilador** | CI | `lint` `validate` `test` `diff` `plan` `report` `compile` `promote` `export` | **nada. Sin red, sin credenciales, sin reloj** |
| **Runtime** | producción | `dev` `serve` + Helm | credenciales vivas de todas las fuentes |

**La fila del compilador entera no abre un socket.** De ahí sale un argumento que no es
marketing:

> El paso que decide **qué significan las cosas** es el único que no puede filtrar nada.

Y el scaffolder queda **fuera del camino de ejecución de confianza**: escribe ficheros,
nunca escribe en el grafo. Lo que produce es una propuesta en `DRAFT`, y la única vía de
propuesta a verdad es un commit revisado. **La IA aporta velocidad sin aportar autoridad.**

---

## Lo que hace que valga la pena

### La gobernanza se demuestra al compilar

```console
$ ore compile

error[OOS4001]: flujo de información no autorizado

  hr.Employee.baseSalary ──derivación──▶ totalCompensation ──binding──▶ materialization.cache

  etiqueta del origen      : gdpr.sensitivity = critical   (declarada)
  etiqueta de la derivada  : gdpr.sensitivity = critical   (computada, join)
  autorización del conducto: gdpr.sensitivity = low

  → declarado en   packages/hr/entities/Employee.yaml:22
  → propagado por  packages/hr/entities/Employee.yaml:31
  → alcanza        packages/hr/bindings/warehouse.yaml:12

  ayuda: baja el modo a `passthrough`, aplica un desclasificador autorizado
         (`mask`, `aggregate`), o eleva la autorización del conducto en
         conduits.yaml — lo último requiere revisión de CODEOWNERS.
```

**Nadie clasificó `totalCompensation`.** El compilador lo hizo propagando desde sus
orígenes, y por eso la etiqueta sigue siendo cierta dentro de seis meses.

Sin conexión a la base de datos. Sin credenciales. Sin un solo dato leído. **Un auditor
externo lo verifica clonando el repositorio.**

### Las tres fronteras se declaran — y la tercera faltaba

Un bundle dice por dónde **entra** el dato (`datasources`) y por dónde **sale**
(`ConduitPolicy`). La entrada de identidad no se declaraba en ninguna parte, y es **la única
entrada que decide** en vez de ser gobernada. Sin sitio donde decir qué es un principal, su
forma se tomaba prestada de un recurso: el DNI de un empleado acababa entrando en el esquema
de autorización como atributo obligatorio **del que pregunta**.

> **Lo que decide el acceso no puede estar sujeto al acceso que decide.**

Un `RequestPolicy` cierra esa frontera: qué atributos entran con una petición, de qué
reclamación sale cada uno, quién los firma (`issuer`, `audience`) y qué finalidades existen.
Con ella, una política que exige un rol **no compila** si nadie declara de dónde vienen los
roles, y una que limita por finalidad tampoco si esa finalidad no la declara nadie.

**Y el recorte de filas es un filtro, no una máscara.** Cedar gobierna propiedades: decide si
`baseSalary` se puede leer, no *qué filas*. Un `scope` de un `Ruleset` declara el recorte
—«solo las filas cuyo `managerId` es el del que pregunta»— y el compilador lo baja al plan
como un filtro sobre la columna que el binding mapea. *Una máscara recorta el valor; un
ámbito recorta la fila.* Y un ámbito **falla al compilar** si la propiedad no existe o si
ningún binding la mapea, en vez de descubrirse al responder.

**Cada techo tiene dueño.** Elevar la autorización de un conducto es *la* decisión de
seguridad de este modelo, así que un `ConduitPolicy` declara `owner` —un `team:<handle>`, que
es lo que se alinea con CODEOWNERS— y de él **heredan las políticas de Cedar**, que eran la
otra superficie sin dueño propio. `ore report` es el registro que sale de ahí
([ADR 0011](docs/decisions/0011-el-informe-no-lista-incumplimientos.md)).

### Nadie escribe un binding a mano

```console
$ ore source add --name crm_prod postgres://acme:••••@db.internal:5432/crm
  ✓ conectado · PostgreSQL 16.2 · 47 tablas
  ✓ credencial en .env.local (añadido a .gitignore)
  ✓ residency: <sin declarar>              ← decisión pendiente

$ ore discover --source crm_prod --out packages/customers
  12 entidades · 38 relaciones desde claves foráneas · 4 tablas puente
  6 critical · 11 high · 3 sin decidir (conf. 0.31)
  ⚠ 3 decisiones te esperan:  ore review

$ ore review
  notes : text — confianza 0.31. El texto libre suele contener PII incidental.
  ¿Sensibilidad?
  › high     puede contener PII   (recomendado)
    none     confirmo que no
```

**No escriben ficheros: contestan preguntas.** Y las políticas tampoco —
`ore policy add --template gdpr.purpose-limitation` pide cuatro respuestas y emite Cedar
correcto. Escribir Cedar a mano es la vía de escape del 10%, no el camino normal.

### El despliegue es reconciliación

```
merge a main → webhook → ORE descarga el delta → recompila → hot reload sin downtime
```

Estado deseado en Git, estado real en el grafo, un reconciliador que los converge.
**Es ArgoCD aplicado a la semántica.**

Y su corolario es la razón de compra en sector regulado: si el grafo es siempre función
determinista de un commit, *«¿qué sabía el agente el martes a las 14:32?»* se responde con
**un commit y una marca de agua**.

---

## Arquitectura

### Dos planos, y no se mezclan

| Plano | Qué sirve | Dónde vive | Latencia |
|---|---|---|---|
| **Contexto** | entidades, relaciones, tipos, políticas, linaje | **artefacto mapeado en memoria** | µs–ms |
| **Datos** | filas y valores | consulta federada al origen | la de la fuente |

Casi todo lo que un agente necesita para no alucinar vive en el plano de contexto, que es
local y compilado. **El plano de contexto no quiere una base de datos**: son megabytes que
no cambian entre despliegues, así que la forma correcta es una estructura de solo lectura
mapeada en memoria — acceso sin deserialización, arranque en milisegundos, sin
calentamiento.

### Sin estado por defecto

> **ORE es *stateless* por defecto y *stateful* por declaración.**

Un repositorio con todos sus bindings en `passthrough` **no necesita base de datos alguna**:
arranca, mapea el bundle y sirve. El almacenamiento aparece solo cuando un binding declara
`mode: index` o `cache`.

Tres consecuencias: el binario abierto es **apto para producción** en el caso mayoritario;
la complejidad operativa es proporcional a la ambición y no un peaje de entrada; y un pod
de ORE es un contenedor que se puede matar y recrear libremente.

### Superficie de servicio

Tres, y ninguna más en v1: **MCP** —donde está la distribución del ecosistema agéntico, y
que sustituye a seis adaptadores de framework—, **GraphQL** para aplicaciones, y un
**protocolo nativo** para alto rendimiento. JSON-LD y Cypher son `ore export --format`, no
camino caliente.

---

## Niveles, y por qué ORE no tiene privilegios

| Nivel | Qué hace | ¿Acceso a datos? |
|:---:|---|:---:|
| **L0** · Validador | valida, normaliza, comprueba el flujo, emite digest | **no** |
| **L1** · Servidor de contexto | entidades, relaciones, tipos, políticas, linaje | **no** |
| **L2** · Ejecutor | resuelve bindings, aplica obligaciones, federa | sí |
| **L3** · Actor | ejecuta funciones y **verifica el acto que un endoso declara** | sí, con escritura |

**ORE ejecuta la suite de conformidad de OOS como un consumidor externo cualquiera**, por
su CLI pública y sin acceso privilegiado a sus estructuras internas.

No es escrúpulo: es lo único que impide que la especificación acabe teniendo la forma de su
implementación de referencia. **ORE debe ser reemplazable para que OOS valga algo, y OOS
debe ser adoptable sin ORE para que ORE valga algo.**

---

## Instalación

> **Ninguno de estos canales existe todavía.** No hay tap, ni crate publicada, ni
> imagen, ni instalador. Hoy ORE se construye desde el repositorio con
> `cargo build`. Lo que sigue es la forma que tendrá la distribución, no una
> instrucción que funcione.

Lo que **sí** existe es el flujo que los alimenta a todos:
[`release.yml`](.github/workflows/release.yml) publica binarios por plataforma
desde un tag, y trata a su propio artefacto como `ore` trata a un bundle — la
suite tiene que pasar, el binario se construye **dos veces** para comprobar que
las dos dan el mismo `sha256`, y sale con **atestación de procedencia** y sus
checksums al lado ([ADR 0004](docs/decisions/0004-distribucion-del-binario.md)).

```bash
gh attestation verify ore-0.1.0-x86_64-unknown-linux-musl --repo describeloai/ore
```

Los cinco canales de abajo son envoltorios sobre eso. Y todavía no hay ninguna
release: **`v0.1.0`**, la primera. Publica el binario `ore` para cinco
plataformas, con su atestación de procedencia y sus checksums al lado.

Los delegados **no se distribuyen todavía**: enlazan lo que el compilador no
puede enlazar —un driver trae una pila TLS, el almacén trae Parquet— y su
distribución es una decisión aparte, con las mismas exigencias de procedencia
([ADR 0009](docs/decisions/0009-que-se-distribuye.md)).

```bash
brew install oos-dev/tap/ore
cargo install ore-cli
curl -fsSL https://get.ore.dev | sh
pnpm add -g @oos-dev/ore            # binario Rust envuelto en npm
docker run ghcr.io/oos-dev/ore
```

Binario estático nativo en Rust, distribuido con la misma simplicidad que `docker`,
`kubectl` o `terraform`.

---

## Estado

**La fase 0 está cerrada.** Los 90 casos del árbol `v1alpha1` de la suite de
conformidad están en verde, y con ellos existen estos comandos: `validate`,
`compile`, `diff`, `export`, `source add`, **`dev`** y **`report`**. Los demás
que anuncia `ore --help` **no están implementados** y lo dicen al ejecutarse.

`ore report` es el registro de **qué gobierna qué y quién responde**, y lo que lo
define es lo que no puede ser: **no es una lista de incumplimientos**, porque una
propiedad sin la clase que exige su clasificación no compila. Eso lo separa del
*compliance status report* de GitLab, que evalúa cada doce horas sobre algo ya
desplegado; aquí se evalúa al compilar, así que la pregunta deja de ser *¿está
gobernado?* y pasa a ser **¿quién responde, y por qué vía?**

Tampoco lista todas las propiedades: de las **40** clasificadas del ejemplo, **29
no exigen nada**, y un informe que las listara sería el 72% de filas diciendo
*«nada que gobernar»*. Lo que exige lo decide `requiresGovernance`, no la
clasificación.

Y hay una quinta columna que la tabla de abajo no tenía: **las superficies de
emisión**, que no son una fase sino un eje propio. `export` habla cuatro
formatos —ODCS, Cedar, OOS canónico y **GraphQL**— y el cuarto deja el borrador
de `v1alpha5` entero en verde, incluidos los cuatro peldaños de *listo* que ese
borrador define. El marcador imprime la cuenta al correr; aquí no se copia.

| Fase | Qué | Criterio de éxito | |
|:---:|---|---|:---:|
| **0** | esquemas, `ore validate`, `ore compile`, el runner de conformidad | compila el ejemplo de referencia y emite un digest estable | ✅ |
| **1** | `source add` · `discover` · `review` sobre PostgreSQL | apuntar a un esquema sucio de ~50 tablas y que un arquitecto diga *«está un 80% bien»* tras contestar cinco preguntas | ◐ |
| **2** | retículos, conductos, propagación, chequeo de flujo, Cedar embebido | `ore validate` falla con la cadena causal completa ante PII que alcanza un conducto no autorizado | ◐ |
| **3** | `ore dev` + servidor MCP + obligaciones en lectura | un agente pregunta por MCP y el PII vuelve enmascarado **sin que el agente haya hecho nada** | ◐ |
| **L2** | **el ejecutor** · autorizar · planificar · federar | **una consulta cruza dos familias de fuente y devuelve una entidad ensamblada**, con el plan rechazable sin abrir una conexión y la respuesta acompañada de sus tres ejes | ✅ |
| **E** | **emisión** · ODCS · Cedar · **GraphQL** | el esquema emitido lo acepta un motor ajeno, el techo del conducto quita de él **exactamente** lo gobernado, y **una mutación que exige firma humana no puede devolver su resultado** | ✅ |

**La fila `L2` tampoco lleva número, y por lo mismo que la `E`.** Es un nivel de
conformidad, no una capa del producto: la cruzan la 1 —los drivers— y la 3 —lo que se
sirve—. Se construyó en cinco hitos con criterios de listo, y ese plan **se borró el
día que el último se puso verde**: un plan que sobrevive a su ejecución deja de ser un
plan y pasa a ser documentación de un pasado que ya nadie comprueba.

Lo que dejó, y se puede ejecutar: dos lectores —`ore-read-postgres` y `ore-read-jsonl`—
y el protocolo que comparten en `ore-driver`. La segunda familia es **un fichero y no otra base de
datos** a propósito: si el mismo plan sirve a un servidor y a un fichero, la petición
estaba cortada por el sitio correcto.

> ### Y el ejecutor de aquel L2 **se retiró**
>
> `ore-exec` —autorizar con Cedar, planificar, federar por clave, y el índice de topología con sus
> verbos `index build · refresh · traverse`— nació el **2026-08-31**, un día antes que el motor de
> vistas y tres antes de que la palabra *vista* existiera en el núcleo. Era el camino de lectura
> **del paradigma de entidades y bindings**: pedirle a la fuente por clave y ensamblar N flujos,
> que es lo que hacía falta cuando N bindings cubrían una entidad.
>
> En el paradigma de vistas eso no aplica. `View = Q(Table)`: **leer una vista es ejecutar `Q`**, y
> responder es decidir si contesta el origen o una copia, qué se empuja y qué queda de residuo —
> que es lo que hace el motor de vistas, con modelo de coste y *view matching*. La fase ③ de aquel
> ejecutor era esa misma tarea hecha en el vocabulario anterior, y su fase ④ existía para reparar
> algo que solo ocurría en el modelo viejo.
>
> Lo que se llevó con él, dicho porque son huecos y no adornos:
>
> - **el evaluador de Cedar** — era el único que enlazaba `cedar-policy`, y hoy nadie evalúa una
>   política. El árbol emite el esquema y comprueba su forma, y ahí se para;
> - **el productor del índice de topología** — su forma se sigue derivando del paquete
>   (`ore_core::aristas`) y el registro de copias lo sigue enumerando, pero **nadie lo puebla**;
> - **la travesía** — seguir el grafo para resolver un conjunto de claves.
>
> Quién planifica y quién responde una lectura en el paradigma de vistas **está sin decidir**, y
> eso es más honesto que un binario que contestaba la pregunta de otro modelo.

### Y lo que se afirma aquí está medido

Cada afirmación de arriba fue un `echo` en una terminal mientras se construía. Un `echo`
demuestra algo **una vez**, así que viven en [`pruebas-de-fuego/`](pruebas-de-fuego/) y las
corre la CI en cada empujón, contra sistemas reales y no contra dobles:

| | Contra qué | Qué caería si se rompe |
|---|---|---|
| `descubrimiento.sh` | un **PostgreSQL sucio** | que `discover --source` resuelve el driver en el `PATH`, le pasa la URL por stdin y analiza lo que devuelve; que de un esquema con colisión de nombres, tabla sin clave, tipo compuesto, vista y familia fechada salen **las decisiones que tienen que salir**; y que contestarlas deja un paquete que `ore validate` acepta |
| `fuentes-reales.sh` | un **PostgreSQL** de verdad | que el driver solo pide lo proyectado, que la sesión es de solo lectura, que dos índices de la misma instantánea son idénticos, que el refresco **sustituye**, y que una consulta cruza dos familias de fuente |
| `graphql.sh` | **`graphql-js`**, un motor ajeno | que el SDL emitido lo acepta alguien que no somos nosotros |

Es el primer peldaño de *listo* de `v1alpha5`, y no una aserción nuestra sobre nuestro propio
formato. La razón de que estén ahí y no en una libreta:

> **Una prueba que no corre tiene exactamente el mismo aspecto que una que pasa.**

De la fase 3 existe **`ore dev`**: sirve el contrato por MCP sobre stdio y **no
toca un dato**. Su criterio de éxito —*«el PII vuelve enmascarado»*— es **L2** y
necesita drivers; lo que hay hoy es la mitad **L1** que ese criterio daba por
supuesta y nunca nombró. La frontera con `serve` no es el nivel sino **qué
custodian**: `dev` es un proceso hijo que muere con su cliente y no abre un
puerto; `serve` es un servicio que sobrevive a sus clientes y por eso les debe
autenticación ([ADR 0005](docs/decisions/0005-la-superficie-de-contexto.md)).

**La fila `E` no es una fase, y por eso no lleva número.** Las cuatro de arriba
se ordenaron por riesgo retirado y describen capas del producto; la emisión las
cruza todas: usa el compilador de la 0, el gobierno de la 2 y es lo que la 3 va
a servir. Tenerla dentro de una fase habría obligado a elegir cuál miente.

De la fase 1 está la cadena entera: **`source add` · `discover` · `review`**.

**`source add`** separa el secreto de la conexión, deriva lo derivable —`type` del
esquema de la URL, el nombre de la variable del manifiesto— y **marca la
residencia como decisión pendiente en vez de adivinarla desde el nombre del
host**. No abre un socket: la sonda es introspección y va con `discover`.

**`discover` son dos actos separados**, y la costura entre ellos es la decisión
que sostiene lo demás: **leer** un catálogo y **proponer** una ontología fallan por
separado, así que se piden por separado. El lector conoce el sistema de tipos de
*su* fuente —una receta para BigQuery, un `ore-read-<tipo>` en el `PATH` para el
resto—; el inductor conoce el de OOS y es puro. Y la regla que gobierna lo que
sale: **se emite lo que es un hecho y se reporta lo que es una conjetura.** Una
tabla es una entidad; una columna `id_cliente` que *parece* apuntar a `clientes`
es una conjetura, y `01-package` §5 dice qué hacer con ella — marcarla, nunca
inventarla.

**`review` es la cara de esa cola**, y no edita lo inducido: **vuelve a inducir**
el catálogo con las decisiones tomadas. Hay respuestas que no caben en una
edición local —resolver una colisión de nombres crea dos entidades donde no había
ninguna; unir una familia fechada borra tres ficheros y escribe uno con tres
bindings—, así que lo que sale es siempre `inducir(catálogo, respuestas)` y
contestar dos veces lo mismo produce el mismo paquete byte a byte. Tiene
`--answers` porque una cola que solo se contesta a mano **no se puede probar**.

### Y por qué la séptima pregunta es la que importa

De las once clases de decisión, diez ordenan el modelo. Una lo **gobierna**.

*«¿El mismo concepto?»* tiene dos respuestas y hacen cosas distintas. Si el
repositorio publica vocabulario, la pregunta ofrece **candidatos con la
clasificación que se hereda al elegirlos**, y elegir uno no escribe nada: apunta
con `is`. Si no hay ninguno, contestar con un nombre lo **acuña** —`is` exige que
el concepto exista, `OOS2001`, y una referencia colgando sería peor que no
preguntar— y entonces aparece la pregunta que faltaba: **cómo se clasifica**. Un
concepto sin etiquetas no gobierna nada, y eso no lo puede decidir el silencio.

Porque la etiqueta de un concepto es la **tercera fuente** de la clasificación
efectiva, y la clasificación efectiva es lo que poda la superficie emitida. Está
medido de punta a punta en `pruebas-de-fuego/descubrimiento.sh`: se contesta que
`email` es `gdpr.personalEmail` —`high`— con el techo de `contextSurface` en
`medium`, y el campo **no está** en el SDL que un agente puede pedir. Nadie
escribió una etiqueta en una entidad.

> Contestar la séptima pregunta es lo que hace desaparecer un campo de la
> superficie. Las otras diez ordenan el modelo; esta lo cierra.

Lo que sigue faltando es **contenido**: un paquete de conceptos publicado que
otros importen. Acuñar uno por columna repetida sigue siendo la inflación que
`02-property` §6.2 nombra — la diferencia es que ahora hay a quién apuntar en
cuanto alguien publica el primero.

La fase 2 va a medias y conviene decir por dónde: **el criterio de éxito ya se
cumple** —retículos, conductos, propagación y la regla de flujo son las nueve
comprobaciones `OOS4xxx`, y `ore validate` falla con la cadena causal ante un
dato clasificado que alcanza un conducto no autorizado. Lo que falta es Cedar
**embebido**: hoy ORE *lee* las políticas para comparar versiones y *proyecta* el
esquema, pero no evalúa una autorización. Eso exige enlazar `cedar-policy`, y es
una decisión distinta de las que ya están tomadas
([ADR 0003](docs/decisions/0003-lectura-estructural-de-cedar.md)).

De la fase 3, la parte que faltaba **decir** ya está dicha: el enmascarado que ese
criterio describe es un desclasificador de v1alpha1 aplicado por objetivo y sin
sujeto, y `ore validate` ya comprueba que baje de verdad (`OOS8003`). Lo que falta
es ejecutarlo, que es L2. El camino hasta ahí está al final de esta
sección.

```
canonical  9/9    diff  22/22   digest  8/8
emit       5/5    invalid 42/42  valid    4/4
                                 TOTAL  90/90

BORRADOR v1alpha2 · efectos y derivación  24/24
BORRADOR v1alpha3 · gobierno              31/31
BORRADOR v1alpha4 · significado           28/28
BORRADOR v1alpha5 · emisión a GraphQL     11/11
```

Y hay dos comprobaciones que **no** están en `cargo test`, porque necesitan cosas que un
compilador hermético no puede necesitar — Node y un servidor:

| | Qué enfrenta | Qué encontró |
|---|---|---|
| **`graphql-js`** | el SDL que emitimos, a la implementación de referencia | defectos con versiones de antigüedad, ninguno visible leyendo |
| **fuentes reales** | el plan, a un PostgreSQL y a un fichero | seis escenarios que se midieron a mano al construir L2 |
| **`cedar-policy`** | las políticas de un paquete, al esquema que ese paquete proyecta | está en Rust y entra en `cargo test --workspace` como cualquier otra |

Las tres las corre `ci.yml` en cada empujón, y esa es la mitad importante:

> **Una prueba que no corre tiene exactamente el mismo aspecto que una que pasa.**

Los cuatro contadores se reproducen con `cargo test -p ore-cli --test conformance -- --nocapture`,
y se cuentan **aparte** a propósito: un número que mezclara una especificación
cerrada con tres en curso ya no se sabría qué mide.

### v1alpha4, y por qué llegó antes que su especificación

`Property`, `Interface`, `is`, `implements` y la familia `OOS9xxx` están
implementados **antes** de que se escribieran `02-property` y `03-interface`, y no
fue un adelanto: el alcance de esa versión pide enfrentar el vocabulario a algo que lo
use *«antes de escribir los esquemas»*. El motor fue esa prueba, y encontró tres
defectos que no se ven leyendo —uno de ellos con **cuatro versiones de
antigüedad**, un `$def` que contradecía la regla de forma canónica del propio
proyecto e iba sin detectar porque ningún documento lo referenciaba.

Lo que se implementó cabe casi entero en algo que ya existía: la herencia desde un
concepto es **una tercera fuente** en la propagación de `flow`, al lado de la
entidad y del `datasource`, y `OOS4012` la gobierna sin que se le haya tocado una
letra desde v1alpha1.

Y con las tres decisiones de la especificación cerradas, el compilador las tiene
las tres construidas. Dos costaron código —la exigencia **categórica** de un
concepto, que entra como tercer origen en la cobertura, y la clausura por
**subsunción** al resolver un objetivo `implements`— y la tercera no costó
ninguno: que una `Function` no pueda apuntar a una interfaz es una prohibición
que se cumple por no existir el campo.

Ninguna de las tres añadió un código. La subsunción no añadió tampoco un campo:
`I ⊑ J` se computa de la inclusión entre sus `requires`, y **lo derivable no se
declara, luego no se puede escribir mal**.

**Y estar en verde no es estar terminado.** Un `kind` atraviesa doce estaciones
—despacho, forma, referencias, tipos, flujo, gobierno, significado, forma
canónica, sellado, compatibilidad, emisión y dependencia— y `Property` e
`Interface` **las atraviesan las doce**, con un caso por tránsito.

La fase 1 cerró la forma canónica: `CONJUNTOS` gana los tres campos, y
`normalize.rs` gana `MAPAS_DE_CONJUNTOS` porque la clave inmediata no alcanza a
`Lattice.requiresGovernance` —sus listas cuelgan del nombre de un nivel—, que
llevaba **una versión entera** siendo sensible al orden sin que nadie mirase.

La fase 2 cerró la compatibilidad: `Shape` gana `conceptos` —como `Prop`, porque
un concepto declara lo mismo que una propiedad, así que sus cambios pasan por
las mismas dos funciones— e `interfaces`. Cinco cambios, **un solo código
nuevo**.

La fase 4 cerró la dependencia, y de paso midió un hueco que no es de v1alpha4
ni de ninguna versión: **una referencia entre paquetes no se comprueba contra
las dependencias declaradas**. Dos paquetes en el mismo árbol con un `is` que
los cruza —o con una etiqueta de un retículo ajeno, que es v1alpha1— validan sin
declarar nada, porque `Package` es una bolsa plana sin noción de a qué
`package.yaml` pertenece cada fichero. Es L0 y decidible, no es el resolutor, y
exige antes una decisión del modelo de empaquetado.

La fase 3 cerró la emisión: `odcs.rs` funde el concepto en la propiedad antes de
emitir, resolviéndolo contra la forma canónica y **no contra el paquete** — un
bundle se basta a sí mismo para emitir. Emite `x-oos-is` porque es lo que
permite **deshacer** la fusión al importar, y de paso el importador dejó de
sellar siempre `v1alpha1`: la vuelta producía un documento que declara una
versión donde `is` no existe.

El criterio de «listo» nunca había estado escrito, y por eso cada borrador
terminó en una estación distinta — [`00-scope`](vendor/oos/spec/v1alpha4/00-scope.md) §8.
Al escribirlo y medirlo salió que `Shape` tampoco tenía `Function`, `Resolution`
ni `Ruleset`, y que la forma canónica de **v1alpha1** estaba rota. Las cuatro
filas están ahora en verde; las cinco cosas que la reutilización de códigos no
pudo cubrir están escritas en §8.6.

Lo que **no** se implementó es la otra mitad, y la frontera es la misma tabla de
arriba: proponer mapeos es del scaffolder, necesita fuente y modelo, y es fase 1.
El compilador solo hace lo suyo — **decir que no**: un documento que no está en
`DRAFT` no puede contener una sola conjetura (`OOS9003`).

### Frontera abierta

El binario es **Apache-2.0 y plenamente apto para producción** en un nodo y una región,
sin límite de uso, para siempre. Lo que se construye encima —alta disponibilidad
multi-región, sincronización de índices entre nodos, conectores enterprise, plano de
gobernanza— es otra cosa y vive en otro sitio.

Un motor abierto que solo sirviera en un portátil no capturaría ningún estándar, y sin
estándar no hay nada que construir encima.

---

## Relación con OOS

| | |
|---|---|
| **OOS** define el artefacto | régimen de identidad, modelo de compilación, contrato de conformidad, vocabulario de gobernanza |
| **ORE** define la ergonomía y la ejecución | cómo un humano llega a un paquete conforme, y cómo se sirve |

Que sean dos cosas es deliberado: **otro proveedor podría construir una experiencia de
autoría mejor que esta y seguir produciendo paquetes conformes.** Eso es exactamente lo que
un estándar hace posible, y la razón de que el motor no se guarde nada.

Las decisiones que cerraron puertas —y lo que se aceptó a cambio de cada una— están en
[`docs/decisions/`](docs/decisions/README.md).
