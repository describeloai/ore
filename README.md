# ORE · Ontology Runtime Engine

**El motor que convierte un repositorio ontológico en un paquete vivo.**

> Un `Ontology Repository` es texto declarativo en Git: exacto, revisable y **inerte**.
> ORE lo compila, lo coteja, lo firma y lo sirve — gobernado, tipado y en tiempo real —
> a tus aplicaciones y a tus agentes.

Apache-2.0 · implementación de referencia de [**OOS**](https://github.com/oos-dev/oos) ·
**pre-alfa, no existe todavía**

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
| **Compilador** | CI | `lint` `validate` `test` `diff` `plan` `compile` `promote` `export` | **nada. Sin red, sin credenciales, sin reloj** |
| **Runtime** | producción | `dev` `serve` + Helm | credenciales vivas de todas las fuentes |

**Ocho de quince comandos no abren un socket** — la fila del compilador entera. De ahí sale un argumento que no es
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
release publicada: la versión del workspace es `0.0.0`, y el flujo se niega a
publicar esa — no es una versión, es el valor de partida.

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

**La fase 0 está cerrada.** Los 89 casos de la suite de conformidad de OOS están
en verde, y con ellos existen seis comandos: `validate`, `compile`, `diff`,
`export`, `source add` y **`dev`**. Los **nueve** restantes que anuncia
`ore --help` **no están implementados** y lo dicen al ejecutarse.

Y hay una quinta columna que la tabla de abajo no tenía: **las superficies de
emisión**, que no son una fase sino un eje propio. `export` habla cuatro
formatos —ODCS, Cedar, OOS canónico y **GraphQL**— y el cuarto certifica los
ocho casos de `v1alpha5`, incluidos los cuatro peldaños de *listo* que ese
borrador define.

| Fase | Qué | Criterio de éxito | |
|:---:|---|---|:---:|
| **0** | esquemas, `ore validate`, `ore compile`, el runner de conformidad | compila el ejemplo de referencia y emite un digest estable | ✅ |
| **1** | `source add` · `discover` · `review` sobre PostgreSQL | apuntar a un esquema sucio de ~50 tablas y que un arquitecto diga *«está un 80% bien»* tras contestar cinco preguntas | ◐ |
| **2** | retículos, conductos, propagación, chequeo de flujo, Cedar embebido | `ore validate` falla con la cadena causal completa ante PII que alcanza un conducto no autorizado | ◐ |
| **3** | `ore dev` + servidor MCP + obligaciones en lectura | un agente pregunta por MCP y el PII vuelve enmascarado **sin que el agente haya hecho nada** | ◐ |
| **E** | **emisión** · ODCS · Cedar · **GraphQL** | el esquema emitido lo acepta un motor ajeno, el techo del conducto quita de él **exactamente** lo gobernado, y **una mutación que exige firma humana no puede devolver su resultado** | ✅ |

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

De la fase 1 existe **`source add`**, que es la parte que no tenía ninguna
pregunta abierta: separa el secreto de la conexión, deriva lo derivable —`type`
del esquema de la URL, el nombre de la variable del manifiesto— y **marca la
residencia como decisión pendiente en vez de adivinarla desde el nombre del
host**. No abre un socket: la sonda es introspección y va con `discover`.

Lo que bloquea a `discover` no es código: **no existe todavía un vocabulario de
conceptos publicado**. `kind: Property` solo aparece dentro de casos de
conformidad, y sin conceptos a los que mapear, las *«cinco preguntas»* del
criterio vuelven a ser cinco ensayos — justo lo que v1alpha4 existe para impedir.

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
emit       5/5    invalid 41/41  valid    4/4
                                 TOTAL  89/89

BORRADOR v1alpha2 · efectos y derivación  24/24
BORRADOR v1alpha3 · gobierno              30/30
BORRADOR v1alpha4 · significado           28/28
BORRADOR v1alpha5 · emisión a GraphQL     11/11
```

Los cuatro se reproducen con `cargo test -p ore-cli --test conformance -- --nocapture`,
y se cuentan **aparte** a propósito: un número que mezclara una especificación
cerrada con tres en curso ya no se sabría qué mide.

### v1alpha4, y por qué llegó antes que su especificación

`Property`, `Interface`, `is`, `implements` y la familia `OOS9xxx` están
implementados con `02-property` y `03-interface` **todavía sin escribir**, y no es
un adelanto: el alcance de esa versión pide enfrentar el vocabulario a algo que lo
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

### L2 · el camino hasta el ejecutor v1

[`05-ejecutor`](vendor/oos/spec/v1alpha1/05-ejecutor.md) está cerrado y es normativo:
dice **qué** debe hacer un L2. Esta sección dice **en qué orden lo construye ORE**, y
es deliberadamente **desechable** — su contrato de caducidad está abajo.

Las etapas se numeran **M0–M4** y no continúan la tabla de arriba a propósito: las
fases 0–3 son capas del producto, y esto es el interior de una sola de sus filas. El
criterio de orden sí es el mismo —**riesgo retirado**—, y aquí hay uno que no se
parece a los demás porque es el único irreversible.

#### La medición que ordena el resto

Antes de la primera línea, con el mismo cierre que mide
[`dependencias.rs`](crates/ore-cli/tests/dependencias.rs): `cedar-policy` 4 arrastra
**153 crates**, de las que **135 son nuevas** en este árbol. Cuatro están en su lista
de vetadas — `chrono`, `time`, `time-core` y `time-macros`, vetadas como *«el reloj;
la compilación no lo lee»*.

`cedar-policy-core` depende de `chrono` porque Cedar 4 tiene una extensión `datetime`,
y una política puede decir *«antes del 1 de enero»*. La arista es opcional, y
desactivarla **no sirve**: se midió, y el `Cargo.lock` sale idéntico, porque no depende
de las features a propósito.

> **Un evaluador de políticas necesita reloj. Un compilador no puede tenerlo**
> —invariante III, la compilación es pura.

De ahí sale la forma del ejecutor entero, y no hay que discutirla: **el guardián que
se escribió para el driver ya la decide**. Enlazar el evaluador dentro de `ore-cli`
llevaría su cierre de 32 a **167** y dispararía
`el_binario_que_se_distribuye_no_sabe_hablar_por_la_red` nombrando `chrono`. El
razonamiento entero está en el
[ADR 0007](docs/decisions/0007-enlazar-el-evaluador-de-cedar.md).

> **El ejecutor vive donde vive el driver: fuera.** `crates/ore-exec`, miembro del
> espacio de trabajo y **fuera de `default-members`**, igual que `ore-read-postgres`.
> Y `ore serve` **delega** en él por PATH, exactamente como `lector.rs` delega en
> `ore-read-<tipo>`.

Lo que eso compra es una frase que deja de ser una promesa: **el binario que compila
no sabe servir, y se demuestra midiendo, no leyendo el código.**

#### M0 · Autorizar de verdad

**Qué.** Nace `crates/ore-exec` y enlaza `cedar-policy`. Contesta una sola pregunta:
dado un bundle, un principal con sus atributos, una acción y un recurso — qué dice la
política, y qué máscara aplica.

**Por qué primero.** §3 hace normativo que autorizar vaya delante, y da la razón:
autorizar al final es haber abierto ya la conexión. Esa razón vale también para el
orden de construcción, porque todo lo que viene después toma su forma de lo que ①
poda. Y retira la única decisión irreversible de todo L2.

**Listo cuando:**

- ~~**ADR 0007**~~ ✅ [escrito](docs/decisions/0007-enlazar-el-evaluador-de-cedar.md):
  el [ADR 0003](docs/decisions/0003-lectura-estructural-de-cedar.md) —lectura
  estructural en compilación— sigue en pie, y **evaluar es otra decisión que vive en
  otro artefacto**. No lo revoca: **ejecuta la puerta de salida que él mismo dejó
  escrita**.
- ~~Los atributos del principal **llegan**, se **verifican**, y una petición sin ellos se
  **rechaza**~~ ✅ — y el emisor contra el que verificar lo declara ahora
  [`06-request`](vendor/oos/spec/v1alpha1/06-request.md), que era el hueco. Emisor,
  audiencia y reclamaciones se comprueban; **la firma criptográfica no**, y se dice: para
  validarla hace falta la red (JWKS), que es una capacidad que se decide con `serve`.
- El esquema que carga el evaluador es **el que emite
  `ore export --format cedarschema`**, no un segundo esquema. Si divergieran, la
  política se habría validado contra uno y se evaluaría contra otro — que es
  exactamente lo que la prueba de fuego de `00-overview` §4.1.1 fue a buscar.
- ~~Dos principales, mismo recurso, veredictos distintos, y el veredicto **nombra la
  política que decidió**~~ ✅ — con **nuestro** `@id`, porque Cedar los nombra por posición
  y eso es justo la identidad que el ADR 0003 rechazó. Y el veredicto trae además las
  obligaciones, las máscaras y los **ámbitos**, que es lo que la fase ③ necesita para saber
  qué filtro empujar.
- ~~El `Deny` deja de ser mudo~~ ✅ — se distinguen tres: **prohibida** por un `forbid` que
  se nombra, **sin política** que la alcance (que no es un fallo: es P4), y **ninguna casó**
  entre las que sí la alcanzan, nombrándolas. Cedar devuelve el mismo `Deny` para las tres.
- ~~El hueco de la jerarquía es una **condición nombrada**, no un `false`
  silencioso~~ ✅ — y `resource in principal` **se retiró**: el recurso de una
  autorización es una propiedad, no una fila. Lo que quedó es la mitad del
  principal —`principal in Employee::"…"`—, que sí es expresable y sí necesita el
  índice: sin él evalúa a falso, así que el veredicto dice **jerarquía no
  disponible** en vez de fingir que ninguna política casó.

**M0 está cerrado.** Los cinco criterios, y el crate no abre nada: el almacén de
entidades sale del bundle.

**No hace:** ni plan, ni fuente, ni una sola fila.

**Guardianes.** `cierre_de("ore-cli")` sigue en **32**: el binario que compila no ha
ganado un reloj. `propios`, en `dependencias.rs`, gana `ore-exec` — si no, la
medición deja de medir lo que dice que mide. Y `el_driver_esta_donde_esta_por_algo`
gana un hermano: `cierre_de("ore-exec") > cierre_de("ore-cli") * 4` — hoy daría 4,8×.

#### M1 · El plan es un artefacto, y se rechaza sin abrir una conexión

**Qué.** Bundle + consulta + principal → un `Plan` con las cuatro fases de §3, o una
de las condiciones de §9.

**Por qué aquí.** Porque todo lo que viene después es una **comprobación contra el
plan**, y porque planificar es puro: se prueba con la misma maquinaria de casos que
L0. **La primera versión del ejecutor no toca un dato**, y eso no es una limitación —
es lo que la hace verificable.

**Listo, y así se comprueba:**

- ~~`ore-exec plan` imprime las cuatro fases en orden~~ ✅, con `(datasourceRef,
  objeto, proyección, claves)` y **los filtros** de cada lectura.
- ~~Los rechazos nombran el binding y el campo~~ ✅ — `fullScan: forbidden` sobre
  `hr.workday`, `capabilities` ausente (§5.1), y *no autorizado* cuando ① lo poda todo.
- ~~Una propiedad con máscara `redact` **no aparece en la proyección**~~ ✅ — y no se
  redacta después: se **quita antes de pedirla**.
- ~~El plan se produce **sin ninguna fuente configurada**~~ ✅, y hay una prueba que
  falla si alguien pone la variable.
- ~~**Mismas entradas → el mismo plan, byte a byte**~~ ✅ — y con `Json::jcs()`, que es
  **la forma canónica del bundle**: G1 aplicado a L2 y no una segunda definición de
  determinismo que podría divergir de la primera.
- ~~Casos en `crates/ore-exec/casos/`~~ ✅ — `jerarquia`, `sin-capacidades`, `redactado`.

**Y un defecto que salió de ejecutarlo, no de leerlo.** `nationalId` está autorizada
para `read` y el binding de Workday **no la mapea**: el plan decía ✓ en ① y la columna
desaparecía de ③ **en silencio**. Que un binding no lo mapee todo es legal; callarlo,
no. Ahora se poda con motivo.

**No hace:** ninguna travesía —el índice es de M3— y ninguna fila.

#### M2 · La carga útil — el segundo verbo del driver

**Qué.** Hoy `ore-read-<tipo>` recibe una URL por stdin y emite un **catálogo**. Gana
un segundo verbo: recibe una **petición** y emite **filas**.

Y la afirmación que hay que acertar aquí, porque es la que decide si esto escala a
cientos de fuentes:

> **La petición es la misma para todos los drivers.** Es un fragmento del plan, no
> SQL. **Traducir es del driver.** Añadir una familia de fuentes es escribir un
> traductor, no tocar el ejecutor.

**Por qué antes que la travesía.** Porque §5.1 ya lo dijo: *el camino principal
funciona sin declarar nada*. La búsqueda por clave sola **ya contesta una consulta**,
así que ③ es el primer trozo que vale por sí mismo.

**Listo, y medido contra un PostgreSQL de verdad:**

- ~~El protocolo en un ADR~~ ✅ [0008](docs/decisions/0008-el-protocolo-del-driver.md) —
  **la petición es un fragmento del plan, no SQL**, y traducir es del driver. Añadir una
  familia de fuentes es escribir un traductor, no tocar el planificador.
- ~~`ore-read-postgres` contesta una búsqueda por clave~~ ✅ — y el **filtro del ámbito**
  llegó hasta el `WHERE`: dos claves pedidas, **una fila devuelta**, porque
  `cost_center = "finanzas"` dejó fuera a la otra.
- ~~**El SQL solo pide las columnas proyectadas**~~ ✅ — con pruebas puras, sin servidor,
  porque un aserto que exigiera una base de datos no se ejecutaría nunca. `SELECT *` no
  existe: una propiedad `redact` no está en el plan, luego no está en la petición, luego
  **no puede estar en el SQL**.
- ~~Un plan con proyección vacía no llega a lanzar el driver~~ ✅ — y si llega, el driver
  la rechaza: *«no hay nada que pedir»*.
- ~~La conexión es de solo lectura~~ ✅ — `SET SESSION CHARACTERISTICS AS TRANSACTION READ
  ONLY`, y el servidor contesta `cannot execute INSERT in a read-only transaction`. **La
  propiedad se compra pidiéndosela**, no prometiéndola.
- ~~El formato de fila decidido y razonado~~ ✅ — NDJSON, con **propiedades** por clave y
  no columnas físicas, y con el disparador de Arrow IPC escrito al lado: la caché de carga
  útil.

**Queda:** el proceso que refresca y el que responde toman credenciales distintas — hoy
quien invoca elige la URL, que es la mitad; la otra mitad es que colapsarlas avise.

**No hace:** ni junta entre fuentes, que es ④; ni travesía; ni caché.

#### M3 · La travesía — el artefacto de topología

**Qué.** El [ADR 0006](docs/decisions/0006-el-artefacto-de-topologia.md) decidió la
forma; esto la construye. CSR, inmutable, mapeado en memoria, reconstruido por
ventana y firmado.

**Listo, y medido contra un PostgreSQL de verdad:**

- ~~`ore-exec index build` produce el artefacto desde las fuentes declaradas~~ ✅, y
  dos construcciones sobre la misma instantánea dan **el mismo fichero byte a byte**.
  G1 otra vez: un índice que difiere entre nodos hace que dos nodos contesten distinto
  a la misma pregunta.
- ~~Una travesía de N saltos devuelve claves **sin abrir una conexión**~~ ✅ — `emp-42`
  → `jefa`, `ceo`. Es la frase que hace asequible la ley de §2, y ya está medida.
- ~~El artefacto lleva su marca de agua y el digest del bundle~~ ✅ — y uno de otro
  bundle **no se carga**: las aristas serían de un modelo y las políticas de otro, y
  esa junta no falla, devuelve filas.
- ~~El artefacto **no está** en el árbol~~ ✅ — con un guardián que busca por **magia y
  no por extensión**, porque renombrarlo es exactamente lo que haría quien quiera
  colarlo. *La mitad OCI llega con la imagen, que todavía no existe.*
- ~~Se cierra el hueco de M0~~ ✅ — `principal in Employee::"ceo"` casa para quien está
  dos saltos por debajo, y la fase ② deja de recibir las claves de fuera.

**Y el protocolo se validó solo.** Las aristas se leen con una petición de la fase ③
cuya proyección se llama `desde` y `hasta`; como las filas salen con nombres de
propiedad, lo que el driver devuelve **ya es una arista**. El driver no se entera de
que esto es un índice.

**No hace:** ni refresco incremental —reconstruir por ventana es el coste declarado en
el ADR 0006— ni `mmap`, que es una dependencia y se paga cuando el artefacto sea lo
bastante grande para que se note.

#### M4 · La respuesta, con sus dos ejes

**Qué.** ④ ensambla sobre flujos ya reducidos, y la respuesta lleva lo que la hace
auditable.

**Y ④ es un ensamblador por clave, no un motor de consulta.** La ley de §2 lo hace
suficiente: el índice ya decidió qué claves se piden, así que sobre flujos que llegan
reducidos no queda nada que optimizar. DataFusion —y con él Arrow y un modelo de
coste— **es una decisión de M5**, cuando exista caché de carga útil: es sobre lo
materializado donde un optimizador se gana el sueldo. Se decide **con v1 delante**.

**Listo cuando:**

- Toda respuesta puede acompañarse del **digest del bundle** y de la **marca de agua**
  de lo materializado que intervino (§7): *qué significaba* y *hasta cuándo era
  cierto*. Sin el segundo eje, *«¿qué sabía el agente el martes a las 14:32?»* no
  tiene respuesta.
- Superar `freshnessSLA` produce **estado degradado declarado en la respuesta**, y no
  un dato viejo con aspecto de fresco.
- `ore serve` existe **como delegación**: resuelve `ore-exec` en el PATH igual que
  `lector.rs` resuelve `ore-read-<tipo>`, y sin él instalado **falla diciéndolo**.
- La respuesta se deriva del **contrato emitido, no del paquete** (§4). Hay un caso
  donde el conducto quitó una propiedad y el ejecutor **no puede contarla** — ni
  siquiera para decir cuántas hay.
- **Una consulta cruza dos fuentes de familias distintas** y devuelve una entidad
  ensamblada. Es el criterio que faltaba: sin él, todo lo anterior puede ponerse en
  verde **sin haber demostrado nunca que la forma escala**, que es lo único que se pidió
  desde el principio. La segunda familia más barata y honesta es BigQuery — el lector ya
  la habla, y trae otra forma de clave y otros tipos sin compartir transporte.
- Y con eso se cumple, por fin, el criterio que la fase 3 ya tenía escrito: **un
  agente pregunta por MCP y el PII vuelve enmascarado sin que el agente haya hecho
  nada** — ahora con un dato de verdad, que es lo que lo hacía L2.

#### Lo que v1 **no** es, dicho antes de empezar

| | Por qué queda fuera |
|---|---|
| **Caché de carga útil** (`payload`) | v1 **materializa el grafo, no las filas**. La travesía hace asequible la ley de §2; la caché **habilita consultas que §2 rechazaría** (§5.2), y eso es estrictamente aditivo |
| **Identidad delegada** (nivel 1 de §6.2) | es lo que hace que el motor deje de ser el único muro, y es un hito propio. v1 se queda en *referencia a secreto*, que es lo que `source add` ya produce |
| **Reconstrucción por cambio de máscara** (§7.1) | no hay nada horneado que reconstruir hasta que exista caché de carga útil |
| **Escritura** | es L3 y exige una `Function`. No es alcance recortado: es normativo |
| **DataFusion y un modelo de coste** | ④ es un ensamblador por clave, y §2 lo hace suficiente. Un optimizador se gana el sueldo sobre lo materializado, así que la decisión va con la caché — M5, y con v1 delante |
| **Alta disponibilidad y sincronía entre nodos** | frontera abierta, abajo |

#### Cuándo se borra esto

Cuando M4 esté en verde, esta sección **desaparece** y se colapsa en una fila de la
tabla de arriba. No se archiva, ni se deja «por si acaso»:

> **Un plan que sobrevive a su ejecución deja de ser un plan y pasa a ser
> documentación de un pasado que ya nadie comprueba** — y eso tiene exactamente el
> mismo aspecto que documentación de un presente que sí.

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
