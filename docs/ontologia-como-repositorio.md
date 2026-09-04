# La ontología como repositorio

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
| **Clasifica** | `labels`, el retículo | vista y tabla lo tienen **estructuralmente prohibido** — y de ahí cuelga el análisis de flujo entero |
| **Acuña** | `is` → `Concept`, `implements` → `Interface` | es significado contra significado. Ningún objeto físico participa |
| **Versiona el nombre** | `moved`, `reserved` | ⟶ §4 |

La cuarta merece pararse, porque es la que decide este documento. La propia spec dice de dónde las
copió:

> **`moved`** *«es el bloque `moved` de Terraform»*. **`reserved`** *«es el campo reservado de
> Protobuf»*, y previene *«el fallo más silencioso y más caro de una ontología viva: una consulta
> antigua que devuelve una cifra correcta para la pregunta equivocada»*.

**Las dos son disciplinas de repositorio**, importadas de dos herramientas que gobiernan cambio en
código. No son metadatos de dato: son **metadatos de versión**. Y en el paquete realista se usan en
**7 de 7** entidades — es la parte de `Entity` que más viva está.

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

| | su unidad de capa semántica |
|---|---|
| **Palantir Foundry** | *object type*, sobre un índice, con *link types* configurados a mano |
| **Cognite** | **data model = un conjunto de `views`, con versión** — literalmente esto |
| **Dremio** | *virtual datasets* —vistas— en un catálogo **con ramas y commits** (Nessie) |
| **Snowflake** | *semantic views* y ficheros YAML de modelo semántico |
| **dbt** | modelos = vistas, **en un repo git**, con tests, docs, linaje y paquetes |

*(Foundry y Cognite están cotejados con fuentes en `sustrato.md` §8.4. Dremio, Snowflake y dbt van
aquí por conocimiento del sector y **no se han cotejado en esta sesión**.)*

**La convergencia es de una nitidez incómoda: la unidad de la capa semántica es un conjunto
versionado de vistas.** Cognite lo llama así sin metáfora. Dremio le puso ramas. dbt demostró a
escala industrial que vistas-en-un-repo funciona.

Y ahí está el hueco, que es exactamente donde estamos:

> **dbt probó las vistas en un repositorio. Foundry y Cognite probaron la ontología sobre una copia.
> Nadie ha puesto la ontología *gobernada* en el repositorio, sobre las vistas.** dbt versiona la
> transformación pero no clasifica ni gobierna; Foundry y Cognite gobiernan pero se configuran en
> una consola, no se revisan en un *pull request*.

---

## 4. Y esto no es una aspiración: es la parte más construida que tenemos

Lo que hace este documento no es proponer una dirección nueva. Es **nombrar la que ya tomamos sin
decirla**. La prueba está en la superficie del producto:

**18 de los 22 verbos de la CLI son verbos de repositorio.**

```text
paquete      init · add · lock · pack
verificación check · lint · validate · test · verify
cambio       diff · review · plan · report · drift-detect
entrega      compile · export · promote
bucle        dev
--------------------------------------------------  18
dato         discover · view · materialize · serve      4
```

Y por debajo:

- **`diff.rs` es el segundo fichero más grande de `ore-core`** —1.408 líneas—, por detrás solo de
  `vistas.rs`. Diferenciar dos versiones de una ontología es la segunda cosa que más código nos ha
  costado;
- **v1alpha6 entera** es distribución, firma, transparencia y registro. Su regla es
  `usar(P) ⟹ digest(P) ∈ lock`, y su tesis de diseño es *«el registro no es de confianza»* — que es
  literalmente el modelo de contenido direccionable de git;
- **`moved` y `reserved`** son Terraform y Protobuf dentro de la entidad;
- y hay cuatro crates enteros —`ore-registry`, `ore-log`, `ore-sign`, `ore-fetch`— que no tocan un
  dato en su vida.

> **No decidimos ser un repositorio. Ya lo éramos.** Lo que faltaba por decir es de qué: **la unidad
> del repositorio es un conjunto de vistas, y la ontología es lo que se anota encima.**

---

## 5. Qué le hace esto a `Entity`

Deja de ser *un mapeo con significado colgado* y pasa a ser **la capa de anotación de un conjunto
versionado de vistas**. Que es, exactamente, la suma de dos peldaños que ya existían por separado y
que ahora se ve que son el mismo camino:

| | qué quita | precio medido |
|---|---|---|
| **M2** | los 38 nombres que solo repiten — `properties` **anota** en vez de redeclarar | 38 de 71 |
| **B0** · `OOS2026` | la arista, que ya está en la copia | 2 entidades |

Y lo que queda es lo del §1: tipar, clasificar, acuñar, versionar el nombre. **Cuatro verbos, todos
irreductibles, todos de significado.** Una entidad más pequeña y más difícil de confundir con otra
cosa.

---

## 6. La pregunta que esto abre, y no cierra

Cognite tiene **dos** unidades: la `view` y el `data model` —el conjunto versionado—. Nosotros
tenemos la `View`, y el conjunto lo lleva el `Package`.

> **¿Es nuestro `Package` el *data model*?** Si lo es, hay que decirlo y darle la superficie que
> eso implica. Si no lo es, falta una pieza entre la vista y el paquete, y no la hemos echado de
> menos todavía porque ningún cliente nos ha pedido dos ontologías sobre el mismo sustrato.

Sin medir, y probablemente lo próximo que haya que medir de esta cara del producto.
