# La ontología como repositorio

> **Las definiciones del modelo viven en [`modelo.md`](modelo.md).** Este documento cuenta **cómo se llegó**;
> aquel, **qué hay**. Si los dos dicen algo distinto, manda `modelo.md` — y es un fallo que hay que
> cerrar, no una diferencia de matiz.

> **Estado: formulación de producto.** No es una decisión de esquema ni un peldaño. Es la
> abstracción de superficie que faltaba: **para qué nació `Entity`, qué hace ya la vista mejor que
> ella, y qué naturaleza tiene la capa que se proyecta de nuestro sustrato.** Los números vienen de
> [`entidad.md`](entidad.md) y de [`sustrato.md`](sustrato.md); lo nuevo aquí es la forma.

---

## 1. Para qué nació `Entity`, y qué hace muy bien

Nació para **acuñar ontología mapeada directamente desde un origen**. Su principio §1.1 lo dice sin
rodeos: *«una propiedad declara exactamente lo que alguien necesita saber para usarla con seguridad,
y nada sobre dónde vive»* — y el *dónde vive* era el `Binding`. `Entity` + `Binding` eran **un
nombre con significado y una dirección física**, y esa pareja era el producto entero.

Ese encargo lo cumple, y hay cuatro cosas que hace bien y que **nadie más en el árbol puede hacer**:

| | qué hace | por qué es irreductible |
|---|---|---|
| **Tipa con significado** | `Money<EUR, 2>` | una columna dice `numeric(12,2)`. La moneda y la escala son una **afirmación**, no un hecho físico |
| **Clasifica** | `labels`, el retículo | vista y tabla lo tienen **estructuralmente prohibido** sobre el DATO — y de ahí cuelga el análisis de flujo entero. (La vista admite `oos.maturity` desde `02-view` §4.1: es su propio estado, no el del dato) |
| **Acuña** | `is` → `Concept`, `implements` → `Interface` | es significado contra significado. Ningún objeto físico participa |
| **Versiona el nombre** | `moved`, `reserved` | ⟶ §4. *Y ya no es solo suyo: el mismo mecanismo llegó a la vista y al manifiesto — `01-package` §3.4* |

La cuarta merece pararse, porque es la que decide este documento. La propia spec dice de dónde las
copió:

> **`moved`** *«es el bloque `moved` de Terraform»*. **`reserved`** *«es el campo reservado de
> Protobuf»*, y previene *«el fallo más silencioso y más caro de una ontología viva: una consulta
> antigua que devuelve una cifra correcta para la pregunta equivocada»*.

**Las dos son disciplinas de repositorio**, importadas de dos herramientas que gobiernan cambio en
código. No son metadatos de dato: son **metadatos de versión**.

> **⚠️ Aquí decía «se usan en 7 de 7 entidades — es la parte de `Entity` que más viva está», y era
> una cifra prestada.** Contado sobre `examples/acme-retail`: `temporal` está en 7 de 7, `moved` en
> **2** y `reserved` en **1**; en las 292 del corpus, 3 y 4. El 7/7 es de la parte «Historia», y
> quien la sostiene es la bitemporalidad, que no es una disciplina de repositorio.
>
> Lo que las hace irreductibles no es cuánto se usan: **es que son las únicas que hay.** Y desde
> [`01-package` §3.4](../vendor/oos/spec/v1alpha1/01-package.md) ya no son solo de la entidad — el
> mismo mecanismo cubre el nombre de un campo y el de un documento, con la regla que elige la casa:
> *lo dice el que sobrevive, y si no sobrevive nadie lo dice el paquete*.

---

## 2. Qué de `Entity` hace ya la vista, y más íntegramente

Todo lo que era *dirección*, y algo más que el `Binding` no sabía hacer:

| | lo hacía | lo hace | y encima |
|---|---|---|---|
| el **mapeo** | `Binding.properties` | `View.fields` | **compone** — una vista sobre otra vista. Un binding no se componía con nada |
| el **recorte** | `Binding.selector` | `View.where` + proyección | queda dentro del fragmento invertible, luego se puede escribir |
| la **materialización** | `materialization.payload` | `View.materialized` | y es lo que decide `raíz de lectura` |
| el **nombre** | — | `View.fields` | **63 de 71** propiedades ya son campo de su vista, y **38 no añaden más que el tipo** |
| la **arista** | — | la copia | `via` nombra una propiedad, `OOS2022` la fuerza a ser campo: **4 de 4** |

> **El `Binding` era media vista** —eso ya lo dijimos al retirarlo—. Lo que faltaba decir es la otra
> mitad: **`Entity` era la otra media, con un sombrero semántico puesto.**

Y la vista lo hace *más íntegramente* por una razón de forma, no de tamaño: **una vista es un objeto
declarado que se compone con otras vistas**, y un binding era una tabla de correspondencias que no
se componía con nada. Al absorberlo, el mapeo pasó de ser una lista a ser un **álgebra**.

---

## 3. Qué naturaleza se proyecta de nuestro sustrato

Una capa de abstracción debe proyectarse de su sustrato, no aterrizar encima de él. El nuestro tiene
tres piezas y una de ellas es rara:

```text
Table   un hecho físico            el objeto tal cual está
View    UNA PREGUNTA, declarada    Q(Table) — y compone
copia   la respuesta, conservada   Q ⊕ ediciones
```

**Que la vista sea una pregunta declarada es lo que decide todo.** Ya nos dio un resultado que no
buscamos —§8.2 de `sustrato.md`: se puede materializar por defecto porque el conjunto de preguntas
es finito y está escrito— y da este:

> **Si una vista es una pregunta, la ontología es el conjunto de preguntas que una organización ha
> acordado hacerse, más qué significan las respuestas.**

Y un conjunto acordado, versionado, firmado, con historia de nombres y con un lock que dice qué
versión de qué usaste… **eso ya tiene nombre en informática, y es un repositorio.** No hace falta
inventarlo: hay setenta años de madurez ahí.

### La comprobación contra la industria

| | su unidad de capa semántica | ¿en un repo? |
|---|---|---|
| **Palantir Foundry** | *object type*, sobre un índice, con *link types* configurados a mano | no — consola |
| **Cognite** | **data model = un conjunto de `views`, con versión** | no — API |
| **Dremio** | *«Views are the foundation»* — tres capas: preparación, negocio, aplicación | **no, y hay que decirlo** |
| **Snowflake** | *semantic view*, **objeto de esquema** — y antes era un YAML en un *stage* | **al revés** |
| **dbt** | *semantic model* sobre un modelo (=vista), en YAML | **sí, y explícito** |

**La primera mitad converge sin discusión: la unidad es la vista.** Dremio lo dice literalmente
—*«Views are the foundation. A view is a SQL-defined virtual dataset that encapsulates business
logic»*— y recomienda una arquitectura de **tres capas de vistas**: una vista por tabla de origen,
encima la lógica de negocio compartida, encima los conjuntos por consumidor. Cognite llama *data
model* a un conjunto versionado de views. Es la misma forma tres veces.

**La segunda mitad —que eso viva en un repositorio— no converge, y el desacuerdo es informativo.**

#### El que confirma: dbt

Y lo dice en su propia documentación, no en un blog de terceros:

> *«You can also commit them to your git repository to ensure everyone on the data and business
> teams can see and approve them as the true and only source of information.»*

Y la pieza que encaja con §6 de este documento:

> *«Semantic models are the starting points of your data and **correspond to models** in your dbt
> project. **You can create multiple semantic models from each model.**»*

**N modelos semánticos sobre una misma vista.** dbt ya resolvió que la capa de arriba es N:1 sobre
la de abajo, no 1:1.

#### El que corrige una afirmación nuestra: Dremio

La versión anterior de este documento decía *«Dremio le puso ramas y commits (Nessie)»* al conjunto
de vistas. **Es falso, y la fuente lo desmiente.** Nessie versiona el **catálogo** —tablas y vistas
Iceberg como dato, con ramas, commits y merge— pero la guía de capa semántica de Dremio **no
menciona git, ni dbt, ni ramas de Nessie** como parte de la capa semántica. Son dos planos, y el
versionado está en el de abajo.

> **Corrección:** Dremio confirma que la unidad es la vista, y **no** confirma que la capa semántica
> se versione como un repositorio.

#### El que va en contra, y es un argumento serio: Snowflake

Snowflake tenía el modelo semántico como **fichero YAML en un *stage*** —versionable, en un repo— y
**se movió en dirección contraria**: a `SEMANTIC VIEW`, un objeto de esquema de la base de datos.
Su razón está escrita:

> *«Schema-level objects with full RBAC, sharing, and catalog support»* · *«Integrated with
> Snowflake's privilege and sharing systems»*

**El gobierno se lo da estar dentro del motor.** Un fichero en un *stage* no tiene permisos; un
objeto de esquema sí. Es exactamente el argumento contrario al nuestro, y hay que contestarlo en
vez de rodearlo.

**La respuesta es que su solución no nos está disponible, y no por gusto:** un `SEMANTIC VIEW` puede
heredar los permisos de Snowflake porque **vive en Snowflake y todo lo que toca también**. Un
paquete nuestro cruza fuentes por definición —`erp`, `workday`, `snowflake`, un lago— y **no hay un
motor cuyos permisos heredar**. Por eso el gobierno tiene que viajar *con* el artefacto: Cedar, el
retículo, el análisis de flujo, la firma. Lo que en Snowflake es una comodidad, aquí sería un
acoplamiento a un proveedor.

Y conviene notar que no lo mataron: `SYSTEM$READ_YAML_FROM_SEMANTIC_VIEW` exporta el objeto a YAML.
**El fichero sobrevive degradado —de fuente de verdad a serialización—**, que es justo el papel que
tendría en un producto que no puede permitirse el otro.

#### Entonces el hueco, dicho con precisión

> **dbt probó vistas-en-un-repo pero no gobierna** —versiona la transformación, no clasifica ni
> aplica política—. **Foundry, Cognite y Snowflake gobiernan pero no se revisan en un *pull
> request*** — consola, API u objeto de base de datos. **Nadie ha puesto la ontología gobernada en
> el repositorio, sobre las vistas, y cruzando fuentes.**

Y el marcador honesto es **uno a favor, uno en contra y uno que solo confirma la primera mitad** —
que es mejor punto de partida que una unanimidad, porque el que va en contra nos obligó a escribir
por qué su camino no está disponible.

---

## 4. Y esto no es una aspiración: es la parte más construida que tenemos

Lo que hace este documento no es proponer una dirección nueva. Es **nombrar la que ya tomamos sin
decirla**. La prueba está en la superficie del producto:

**16 de los 22 verbos de la CLI son verbos de repositorio.**

```text
paquete      init · lock · pack
verificación lint · validate · test · verify
cambio       diff · review · plan · report · drift-detect
entrega      compile · export · promote
bucle        dev
--------------------------------------------------  16
dato         source · discover · view · materialize · cache · serve   6
```

> **Recontado, y la versión anterior tenía dos erratas y una omisión.** Decía `add` y `check`, que
> **no existen**, y no nombraba `source` ni `cache`, que sí. Y hay que decir lo que el recuento no
> dice: **seis de los veintidós están declarados y no hacen nada** —`lint`, `test`, `plan`,
> `promote`, `drift-detect` y `serve`, en `SIN_IMPLEMENTAR`—, y cinco de esos seis son de
> repositorio.

Y por debajo:

- **`diff.rs` es de los tres ficheros más grandes de `ore-core`** —`vistas.rs`, `governance.rs` y
  él—. Los tres son el sustrato, el gobierno y el cambio, que es exactamente lo que este documento
  dice que somos. *(La versión anterior decía «el segundo, por detrás solo de `vistas.rs`»;
  `governance.rs` lo pasó y nadie lo recontó.)*;
- **v1alpha6 entera** es distribución, firma, transparencia y registro. Su regla es
  `usar(P) ⟹ digest(P) ∈ lock`, y su tesis de diseño es *«el registro no es de confianza»* — que es
  literalmente el modelo de contenido direccionable de git;
- **`moved` y `reserved`** son Terraform y Protobuf, y desde v1alpha8 en sus tres alcances — la propiedad, el campo y el documento. La importación de Terraform estaba a medias: su `moved` renombra **direcciones de recurso**, y el nuestro solo llegaba a los miembros;
- y hay cuatro crates enteros —`ore-registry`, `ore-log`, `ore-sign`, `ore-fetch`— que no tocan un
  dato en su vida.

> **No decidimos ser un repositorio. Ya lo éramos.** Lo que faltaba por decir es de qué: **la unidad
> del repositorio es un conjunto de vistas, y la ontología es lo que se anota encima.**

---

## 5. Qué le hace esto a `Entity`

Deja de ser *un mapeo con significado colgado* y pasa a ser **la capa de anotación de un conjunto
versionado de vistas**. Que es, exactamente, la suma de dos peldaños que ya existían por separado y
que ahora se ve que son el mismo camino:

| | qué hace | resultado, ya medido |
|---|---|---|
| ~~**M2**~~ | ~~los nombres que solo repiten~~ | **22 nombres**, no 38: el resto sostiene la clave, una `via` o una derivada, y el tipo no lo dice nadie más |
| ~~**B0**~~ | ~~la arista, que ya está en la copia~~ | **descartado.** `via` es de donde el índice deriva la arista. De ahí salió lo que sí faltaba: sellar el índice |

> Las dos filas están tachadas y no borradas porque el §1 se apoyaba en ellas. Lo que queda de la
> entidad después de medirlas está en [`modelo.md`](modelo.md) §1.

Y lo que queda es lo del §1: tipar, clasificar, acuñar, versionar el nombre. **Cuatro verbos, todos
irreductibles, todos de significado.** Una entidad más pequeña y más difícil de confundir con otra
cosa.

---

## 6. ¿Es el `Package` esa unidad? Medido

Cognite tiene **dos** unidades: la `view` y el `data model` —el conjunto versionado—. Nosotros
tenemos la `View`, y el conjunto lo lleva el `Package`. Medido con
[`pruebas-de-fuego/medida-paquete.py`](../pruebas-de-fuego/medida-paquete.py):

### 6.1 · El límite cierra, y eso no era obvio

Toda referencia al sustrato se queda dentro del paquete que la escribe:

```text
backedBy    -> View       22 total    22 dentro    0 cruzan
from.view   -> View       16 total    16 dentro    0 cruzan
from.table  -> Table      25 total    25 dentro    0 cruzan
--------------------------------------------------------------
                          63 total    63 dentro    0 cruzan
```

**63 de 63.** Lo único que cruza son 3 `relations.target`, y los tres son `after ⇒ before` de casos
de `diff` —dos versiones del mismo paquete en dos directorios—, o sea **artefacto de la medida, no
un cruce**.

> **El límite del paquete es una costura real del sustrato, no una convención de carpetas.** Una
> vista no se apoya en la tabla de otro paquete, y una entidad no se respalda de la vista de otro.
> Eso es exactamente lo que hace falta para que la unidad sea proyectable: que el sustrato la
> respete sin que nadie se lo pida.

### 6.2 · Y los atributos de repositorio ya están

Sobre los 301 paquetes del corpus, los obligatorios son universales —**versión semver 100 %, estado
100 %, dominio 100 %**—. Los de gobierno son raros en conformidad porque un caso declara lo mínimo;
en el paquete escrito entero están **todos**:

| | |
|---|---|
| `version: 2.4.0` | semver |
| `dependencies` | **con rango** — `{ package: acme/core-identity, version: "^1.2" }` |
| `sla.breakingChangePolicy.noticePeriod` | **normativo**: `ore diff` falla si un cambio rompedor llega sin preaviso — `OOS5022` |
| `owner: team:people-platform` | *handle*, y la spec dice por qué: **«alinea con CODEOWNERS»** |
| `description.limitations` | dónde **no** se debe usar esto |
| `status` | vocabulario de madurez de ODCS |

Rango semver, lock por digest, política de cambio rompedor comprobada por el diferenciador y un
dueño que apunta a CODEOWNERS. **Eso es un paquete en el sentido pleno**, no una carpeta con nombre.

### 6.3 · Lo que le falta, y son tres cosas concretas

**1 · El paquete no *selecciona* vistas: contiene un directorio.** Un *data model* de Cognite
**lista** sus views con su versión; el nuestro es *«lo que haya en `views/`»*. La consecuencia no es
estética:

> **Una vista pertenece a exactamente un paquete, y no se pueden publicar dos lecturas ontológicas
> sobre las mismas vistas.** dbt permite justo lo contrario —*«you can create multiple semantic
> models from each model»*— y nosotros no, por contención de directorio.

> ### ✅ Cerrado, y pidiendo lo que no hacía falta
>
> **La membresía no se declara, porque es derivable** —el directorio ya la dice, y redeclararla
> sería violar P2—. Lo que se midió
> ([`medida-declaracion.py`](../pruebas-de-fuego/medida-declaracion.py)) es que la pregunta era
> otra: la **visibilidad**. Tres reglas candidatas para derivarla, y la que parecía buena
> —*público = lo que nadie del paquete usa*— publicaba **catorce documentos de casos
> `invalid/`**: desde dentro del paquete, algo publicado y algo muerto se ven igual. La
> información que falta —*«esto lo expongo a propósito»*— **no está escrita en ninguna parte del
> árbol**, así que no hay de dónde derivarla.
>
> De ahí sale `exports` —[`01-package` §3.2](../vendor/oos/spec/v1alpha1/01-package.md)—, con el
> nombre de Java y de Node y no el de Cognite, porque lo suyo es membresía y esto es visibilidad.
> **Ausente significa nada, no todo**: es P4. Y con él llegan `OOS2027` y `OOS2028` — el segundo
> es el que §7 echaba de menos.
>
> La consecuencia que este párrafo pedía **se cumple igual**: dos paquetes pueden publicar dos
> lecturas sobre la misma vista, porque el que la tiene la exporta y los dos la nombran. Lo que
> cambia es que ahora **está declarado quién lo permite** en vez de ocurrir por resolución plana.

**2 · Hoy un paquete no es, de hecho, un conjunto de vistas.** 73 de 337 tienen alguna vista; **216
entidades no tienen respaldo físico de ninguna clase.** Es sesgo de corpus —casi todo es anterior a v1alpha7— pero
mientras dure, la frase describe una intención y no el árbol.

**3 · El ejemplo realista no demuestra la unidad.** `acme-retail` tiene **tres** directorios con
pinta de paquete —`hr`, `customers`, `supply`— y **un solo `package.yaml`**. Los otros dos tienen
entidades, vistas y tablas sin manifiesto. Es deuda del ejemplo, y hasta que se salde no hay dónde
enseñar esto.

### 6.4 · El veredicto

> **`Package` sí es proyectable como unidad ontológica.** La costura cierra 63 de 63 y los atributos
> de repositorio ya están escritos y comprobados. Lo que le falta **no es naturaleza: es una
> declaración** — que el paquete diga de qué vistas se compone, en vez de heredarlo del directorio.

Y esa es, exactamente, la pieza que Cognite tiene y nosotros no. No hace falta un `kind` nuevo entre
la vista y el paquete: hace falta que el manifiesto **nombre** su conjunto.

---

## 7. ¿Rompe eso la contención por directorio? **No hay contención que romper**

El recuento de §6.1 —63 de 63— no podía contestar si eso lo *impone* alguien o es como está escrito
el corpus. Así que se construye el árbol que el corpus no tiene y se le pregunta al motor:
[`pruebas-de-fuego/medida-contencion.py`](../pruebas-de-fuego/medida-contencion.py).

**Dos miembros. `infra` tiene la tabla y la vista; `rrhh` tiene solo la entidad, con
`backedBy: hr.empleados` — que vive en el otro.**

```text
1. validate del WORKSPACE      ok · sin errores
2. pack     del WORKSPACE      un solo .oob llamado «infra»,
                               y dentro Package:infra Y Package:rrhh
3. validate del MIEMBRO solo   error[OOS2018] `backedBy: hr.empleados` no existe
4. pack     del MIEMBRO solo   se niega: «no valida, así que no se empaqueta»
```

> ### La contención no la sostiene el compilador. La sostiene el empaquetador.

El enlazado resuelve por **nombre cualificado sobre el árbol entero** —`Package` es
`{ root, docs }`, una lista plana— y no consulta jamás a qué miembro pertenece un documento. Los
`miembros` existen y son lo que dice [`link.rs:88`](../crates/ore-core/src/link.rs:88), pero solo los
usan `sync`, el candado y la superficie de significado. **Nadie los usa para resolver una
referencia.**

De ahí tres consecuencias, y las tres cambian el diseño:

**1 · El 63 de 63 no lo garantiza ninguna regla.** Es disciplina de quien escribió el corpus. Hoy se
puede escribir un workspace que valida perfectamente y cuyos miembros no se pueden publicar por
separado, y nada avisa hasta que alguien intenta empaquetar uno.

**2 · Y cuando avisa, nombra lo que no es.** `OOS2018` dice *«`backedBy: hr.empleados` no existe»*.
Existe. Está en el paquete de al lado y no se ha declarado dependencia. **El diagnóstico correcto es
otro**, y hoy no hay código que lo diga.

**3 · Declarar el conjunto no rompe la contención: la crea.** Es la primera cosa que haría
comprobable, en `validate` y no en `pack`, una propiedad que hoy solo se descubre al publicar.

### 7.1 · Y el digest aguanta

La preocupación legítima era el digest, porque
[`canonical-form` §5.2](../vendor/oos/spec/v1alpha1/90-canonical-form.md) hace que **las rutas que
entran en él sean relativas al paquete** —es lo que valida el caso `package-layout-equivalence`, y
lo que permite migrar de disposición plana a multipaquete moviendo ficheros—.

Una lista declarada **no lo toca**: vive en `package.yaml`, que ya está dentro del paquete, y sus
rutas siguen siendo relativas. Y si algún día la lista nombrara una vista de una **dependencia**
—como hace Cognite—, la maquinaria para eso ya existe y es `dependencies` + el lock por digest.
**Compone en vez de chocar.**

### 7.2 · La respuesta, entonces

> **Sí: un paquete debe ser de facto un conjunto de vistas, y declararlo es lo que hace que lo
> sea.** Hoy no lo es ni de hecho ni de derecho — es un directorio, y la costura que parecía cerrada
> la cierra un mensaje de error tardío que además nombra lo que no es.

Y sale barato, que es lo raro cuando algo es además lo natural: **no hace falta un `kind` nuevo, ni
tocar el digest, ni prohibir la disposición por directorio** — que puede seguir siendo el defecto,
igual que `packages/*` es hoy el defecto de `workspace.members`. Lo que cambia es que deja de ser lo
único.

---

## 8. Y lo que destapó por el camino: la materia de `ore pack` — **corregido**

> **Estado: arreglado.** Lo que sigue describe el fallo tal como se midió, porque el porqué de la
> corrección no se entiende sin él. Lo que se hizo está en §8.5.

Medido en [`empaquetar.rs:354`](../crates/ore-cli/src/empaquetar.rs:354):

```rust
fn identidad(pkg: &Package) -> Result<(String, String), Fallo> {
    let d = pkg.docs.iter().find(|d| d.kind == Kind::Package)  // ← el PRIMERO
```

**`identidad()` supone que hay exactamente un `Package` y coge el primero, sin comprobar si hay
otro.** De ahí sale todo lo demás.

### 8.1 · La identidad la decide el orden del directorio

El mismo árbol, con el miembro dependiente renombrado de `rrhh` a `aaa` y nada más:

```text
packages/{infra, rrhh}   ->   .oob dice  package: infra   sha256:fd94b8c2…
packages/{infra, aaa}    ->   .oob dice  package: aaa     sha256:c8486c86…
```

**Renombrar una carpeta cambia el nombre que el paquete afirma de sí mismo**, sin tocar una sola
definición. Y `01-distribucion` §2 está escrito contra exactamente esto:

> *«`package` y `version` **DEBEN** estar, y son la identidad que el fichero **declara**. **Un
> fichero renombrado es un fichero que miente**, así que la identidad va dentro.»*

La identidad va dentro para que renombrar el fichero no pueda mentir. Lo que no se previó es que
**renombrar un directorio cambia la identidad de dentro.** El mismo fallo, por una puerta que la
regla no cubría.

### 8.2 · Y el digest es honrado, que es lo que hace esto sutil

Los dos digests son distintos, y **correctamente**: el contenido cambió —el manifiesto ajeno que
viaja dentro cambió de nombre—. El digest nunca miente sobre el contenido.

> **Lo que miente es la coordenada.** `usar(P) ⟹ digest(P) ∈ lock` garantiza *«recibiste lo que el
> lock nombra»*; no garantiza *«lo que el lock nombra es un paquete»*. La regla se cumple y aun así
> deja pasar un `.oob` que dice llamarse `infra` y lleva dentro el manifiesto de `rrhh` con todos
> sus documentos.

### 8.3 · La maquinaria correcta ya existe, y `pack` no la usa

| | ¿respeta los miembros? |
|---|---|
| `lock` · [`candado.rs:269`](../crates/ore-cli/src/candado.rs:269) | **sí** — `miembros()` devuelve un mapa |
| verificación de sobres · [`sync.rs:105`](../crates/ore-core/src/sync.rs:105) | **sí** — `publicables(&solo(pkg, &miembros, fichero))` |
| `pack` · [`empaquetar.rs:358`](../crates/ore-cli/src/empaquetar.rs:358) | **no** — el primero |
| `dev` (MCP) · [`mcp.rs:61`](../crates/ore-cli/src/mcp.rs:61) | **no** — el primero, misma línea |

`sync` ya compone las dos piezas en el orden bueno: **acota al miembro y luego filtra.** `pack`
filtra sin acotar. Son **dos** sitios con la suposición, no uno.

Y [`publicables()`](../crates/ore-core/src/link.rs:117) quita el `OntologyConfig` y el lock **pero
no otros `Package`** — se escribió para el caso de un paquete solo.

### 8.4 · Por qué no lo cogió ningún test

`crates/ore-cli/tests/empaquetar.rs` tiene **`el_manifiesto_del_workspace_no_viaja()`**. O sea: ya
nos preguntamos una vez si un manifiesto que no es del paquete se cuela en la publicación, dijimos
que no debe, y lo dejamos comprobado.

> **Lo que falta es su hermano: `el_manifiesto_de_otro_paquete_no_viaja`.** Misma clase de error,
> uno cazado y otro no, y ningún test de `pack` monta dos miembros.

### 8.5 · Lo que se hizo, y la medida que decidió el diseño

Lo único no decidido era qué debe hacer `pack` sobre una raíz con varios miembros: **empaquetar cada
uno, o negarse pidiendo que se apunte a un miembro.** Parecía que negarse era lo conservador. La
medida dice que no:

```text
apuntar `pack` al directorio de un miembro, en los 4 árboles multipaquete del corpus
  8 miembros  ·  4 empaquetan  ·  4 FALLAN
    OOS4003   el retículo vive en la raíz
    OOS2001   el concepto es de otro paquete
    OOS2004   los `datasources` están en el manifiesto raíz
```

> **Negarse habría sido dar un consejo que no funciona la mitad de las veces.** Un miembro sacado de
> su árbol pierde lo que cuelga de la raíz y lo gobierna.

Así que **un `.oob` por miembro, empaquetado en el contexto del workspace**, que además es lo
coherente con el criterio que se pidió seguir: la unidad del `lock` también es el árbol entero, y
direcciona los miembros por nombre.

| | |
|---|---|
| **qué lleva cada `.oob`** | los documentos de su miembro **más los que no son de ningún miembro** — el retículo, el `Ruleset`, la política de conductos viajan con todos porque gobiernan a todos |
| **qué no lleva** | los documentos de otro miembro, y su manifiesto |
| **dónde salen** | `-o <directorio>`, con `<último segmento>-<versión>.oob` — la misma forma que escribe el candado al vendorizar |
| **sin `-o`** | error: por stdout solo cabe un artefacto, y se listan los miembros |
| **un solo miembro** | **sin cambios**. `acme-retail` sigue dando 17 documentos y el mismo `sha256:30cd95b6…` |

Y `dev` no se niega —un workspace de varios miembros es entrada legítima del bucle de desarrollo—
así que lo que se arregla es la respuesta: **anuncia todas las coordenadas**, `alfa@1.0.0,
beta@2.0.0`, en vez de la primera por orden de directorio.

**Tres tests nuevos**, y uno que se apoyaba en el fallo:

- `el_manifiesto_de_otro_paquete_no_viaja` — el hermano que faltaba, afirmando la propiedad entera:
  cada `.oob` declara su coordenada, ninguno lleva el manifiesto del otro, y el concepto de un
  miembro se va con su dueño;
- `varios_miembros_por_stdout_no_caben`;
- `un_workspace_se_anuncia_con_todos_sus_miembros`, en `contexto.rs`;
- y `un_oob_que_no_es_el_que_se_pidio_no_se_escribe`, en `candado.rs`, **construía un árbol de dos
  manifiestos para cambiar la identidad publicada** — se apoyaba justo en el fallo. Lo que necesita
  no es un árbol de dos paquetes: es un paquete que diga otra cosa, así que ahora reescribe el
  manifiesto del miembro en vez de añadir uno en la raíz.

---

### Fuentes

- [Dremio · Semantic Layer: The Definitive Guide](https://www.dremio.com/blog/semantic-layer-the-definitive-guide/) ·
  [What is Nessie, Catalog Versioning and Git-for-Data?](https://www.dremio.com/blog/what-is-nessie-catalog-versioning-and-git-for-data/)
- [Snowflake · YAML specification for semantic views](https://docs.snowflake.com/en/user-guide/views-semantic/semantic-view-yaml-spec) ·
  [Cortex Analyst semantic model specification](https://docs.snowflake.com/user-guide/snowflake-cortex/cortex-analyst/semantic-model-spec)
- [dbt · About MetricFlow](https://docs.getdbt.com/docs/build/about-metricflow)
- Foundry y Cognite, cotejados en [`sustrato.md` §8.4](sustrato.md)
