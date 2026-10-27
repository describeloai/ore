# 0018 · La ontología es el sistema de registro

**Estado:** **aceptado** · **Fecha:** 2026-09-03 · **Decide:** dónde aterriza una escritura que sale
de la ontología — y, con ello, dónde está el centro de gravedad del producto

> Corrige [`functions.md`](../functions.md) §4.3, que decía *«el origen es la verdad»*, y **retira
> la contradicción** que ese documento tenía con el [ADR 0017](0017-la-escritura-sobre-el-sustrato.md),
> el cual ya había dicho que escribir en el origen es otro producto. Gana el 0017.

---

## El problema, y de dónde venía

`functions.md` §4.3 mandaba los efectos **al sistema de origen**, a través de la vista. Suena
prudente —*«la copia es derivada; el origen es la verdad»*— y tiene tres defectos, en orden de
gravedad.

### 1 · Traslada el centro de gravedad al sistema del que el cliente se quiere ir

Si la propuesta entera es que **la ontología pasa a ser el centro**, escribir en el sistema legacy
lo reinstala como sistema de registro. El cliente migra su modelo y no migra su gravedad.

### 2 · Exige una credencial que llevamos toda la vida diciendo que no se pide

`01-table` §2 dice que el puntero es de solo lectura. Databricks dice lo mismo y más fuerte —
*«Unity Catalog will not issue write credentials under any circumstance»*. Aplicar sobre el origen
necesitaría exactamente esa credencial, **sobre el sistema que menos se controla**.

### 3 · El sesgo es datable, y no era una decisión: era una herencia

`effects[].datasourceRef` está en la gramática **desde v1alpha2** — cuando una entidad se
respaldaba de un `Binding` y no existían ni `Table` ni `View`. Un efecto nombraba su fuente física
porque no había otra cosa que nombrar.

`F0a` quitó **el campo** y conservó **el destino**. Y la frase *«el origen es la verdad»* se
escribió en la reescritura de `functions.md`, o sea **después** del sustrato: una justificación
nueva para una herencia vieja. Es el modo en que un sesgo sobrevive a la refactorización que
debía eliminarlo.

---

## Lo que hacen los demás, y lo que su mecánica delata

**Foundry no escribe en el origen.** Una *Action* manda su instrucción al Object Data Funnel, que
la aplica al índice y la vuelca periódicamente a un **writeback dataset propio**. Y hay una frase
que lo cierra del todo:

> *«In order for a user to be able to take an action defined in an action type configuration, **a
> writeback dataset must be created**.»*

No es que **pueda** usar uno: sin él **la Action no existe**. Una *virtual table* puede respaldar
un object type, y aun así las ediciones no vuelven por ella. Para tocar un sistema externo llama a
un **webhook**, y admite por escrito que eso no es transaccional.

**Cognite escribe en el container**, que es almacenamiento **primario**. No hay origen detrás: su
container **es** el sistema de registro.

> Los dos escriben en almacenamiento propio. **Ninguno invierte una vista para tocar el sistema de
> nadie.**

Y hay un detalle de Foundry que dice más que el resto junto: tiene **edit-only properties** —
propiedades que existen **solo en la ontología**, sin columna en ningún origen. Su ontología
sostiene hechos que ningún sistema tuvo nunca. Eso es la diferencia entre **ser** un sistema de
registro y **reflejar** otros.

---

## Decisión A · el destino de un efecto es nuestro almacén, nunca el origen

> **Una escritura que sale de la ontología aterriza en la copia. El puntero sigue siendo de solo
> lectura, sin matices y sin excepciones.**

La copia deja de ser `Q(origen)` y pasa a ser:

```text
copia  =  Q(origen)  ⊕  ediciones
```

Que es, con otro nombre, el *writeback dataset* de Foundry: *«every time a writeback dataset is
built, the history of edits is reapplied to get the final state»*.

### Y esto **no** convierte la copia en primaria por la puerta de atrás

Sigue siendo **derivada**: lo que cambia es de qué. Antes derivaba de una cosa —el origen— y ahora
de dos —el origen y nuestro propio registro de ediciones—. Las dos son entradas declaradas y
reproducibles, que es lo que «derivada» quiere decir.

### Lo que se cae solo al decidir esto

**No hace falta invertir la vista.** Medido: la copia guarda **el vocabulario de la vista**
—`employeeId`, no `employee_id`— porque su esquema sale del plan. Un edit ontológico nombra una
propiedad, la propiedad es un campo de la vista, y el campo **es una columna de la copia**. Cae
directo.

**Y no hace falta la cuarta delegación.** `ore-write-<tipo>` desaparece del plan: escribir aterriza
en `ore-store-r2`, que ya existe, ya habla el protocolo y ya sabe sellar y fundir. La corrección
**quita** una pieza en vez de añadirla, que es la señal de que la dirección es la buena.

---

## Decisión B · escribir obliga a materializar

> **Una vista por la que la ontología escribe DEBE declarar `materialized`.** Una vista virtual no
> tiene dónde sostener una edición.

Es **exactamente simétrico** con `OOS2020` —*lo que no se puede leer se debe materializar*— y por
eso comparte su forma: una condición sobre la vista que se comprueba **al compilar**, sin abrir
nada.

### Y aquí está la frase que deshace la falsa dicotomía

No hay que elegir entre *espejo* y *registro*. Se elige **por vista**, y la frontera es declarada:

| la vista | qué es | qué la delata |
|---|---|---|
| sin escrituras desde la ontología | **un espejo perfecto** — puede quedarse virtual, y cada lectura va al origen | no hay `Function` con un efecto sobre una entidad que la respalde |
| con escrituras | **un registro** — se materializa, y la copia es el estado | la hay, y entonces `materialized` es obligatorio |

> **Ser un registro íntegro no impide ser un buen espejo**, mientras no haya ediciones. Y en cuanto
> las hay, hay materialización.

El mismo paquete puede tener las dos clases a la vez, y **el compilador dice cuál es cuál** sin que
nadie tenga que acordarse.

---

## Lo que esto retira · el inventario, para ejecutarlo aparte

Este ADR decide; retirar es trabajo con pruebas y se hace en su propio peldaño. Lo que queda
obsoleto, medido:

| superficie | qué le pasa | por qué |
|---|---|---|
| **`Table.writes`** — spec §3.1 y §5, `schemas/v1alpha8/table.schema.json`, `document.rs`, `escrituras()` | **se retira** | declaraba qué acepta **el origen**, y ya no se escribe en el origen |
| **`OOS7012`** — *efecto sobre una tabla que no acepta `update`* | **se retira**, como `OOS7010` | la pregunta deja de existir |
| **`OOS7013`** — la invertibilidad | **cambia de dueño**: pasa a ser regla del **otro producto** —escribir en el origen— y sale de este | con `B`, el edit cae en la copia en coordenadas de la vista: no hay `Q` que invertir |
| **`OOS2024`** — la clave | **sobrevive con otro sujeto**: no la exige `writes`, la exige **ser escribible** | para tocar una fila de la copia hay que identificarla, y la clave sigue siendo `changes.key` |
| la línea `escritura` de `ore view` | **cambia de pregunta** | de *«¿la tabla acepta `update`?»* a *«¿se materializa y expone la clave?»* |
| los casos `effect-on-a-table-that-does-not-accept-update`, `writes-update-without-a-key`, `a-function-writes-through-its-view` | **se rehacen** | afirman la regla vieja |
| `datasource-ref-in-an-effect` | **se queda** | quitar `datasourceRef` del efecto sigue siendo correcto, y ahora **más**: el efecto no nombra nada físico porque no toca nada físico |

**Y lo que no se toca, que es más de lo que parece:** `F1` entero —la `Propuesta`, las cinco
identidades, el cotejo, `ore verify`— es indiferente a dónde aterriza. La guarda de invertibilidad
de `vistas.rs` y su censo se quedan donde están: siguen siendo la respuesta a *«¿esta vista se
puede deshacer?»*, que es una pregunta legítima aunque este producto ya no la haga.

> **El ADR 0017 sale reforzado.** Su Decisión A —*el recibo del sucesor se escribe sobre su base,
> con `If-None-Match`*— deja de ser una precaución y pasa a ser **el mecanismo**: una edición
> produce una copia sucesora, y la carrera entre dos escritores es exactamente lo que aquello
> resolvió. Y su Decisión B —copy-on-write hasta que una medida diga otra cosa— gana un motivo
> nuevo para revisarse, porque las ediciones son escrituras pequeñas y frecuentes, que es
> justamente el perfil que aquel documento nombró como el que la rompe.

---

## Lo que esto **no** decide, y se dice como abierto porque es superficie de producto

Tres, y las tres se deciden mirando lo que el producto promete, no lo que el compilador puede.

### 1 · Leer lo que acabas de escribir

Si una vista materializada sostiene ediciones, `raíz de lectura` tiene que preferir la copia — o
una consulta devolvería el estado anterior a la edición que acaba de aceptarse.

Foundry lo garantiza por escrito, con seguimiento de *offsets* en Funnel: una lectura posterior a
una modificación **contiene** esa modificación. Nosotros tenemos `freshness` declarada por vista, y
las dos cosas no dicen lo mismo: una promete frescura **respecto al origen** y la otra respecto a
**mis propias escrituras**. Puede que hagan falta las dos palabras.

### 2 · Qué gana cuando el origen cambia debajo

Foundry tiene política y es explícita: **reaplica el historial de ediciones** en cada
reconstrucción, o sea que la edición gana. La nuestra no está escrita, y escribirla es barato ahora
y caro después. El caso que la fuerza: se edita `estado` de una fila y el refresco siguiente trae
esa misma fila con otro `estado` desde el origen.

### 3 · Si la ontología puede sostener hechos que ningún origen tuvo

Hoy no: `OOS2022` exige que toda propiedad sea campo de su vista o declare `derivedFrom`. Foundry
sí, con sus *edit-only properties*.

**Es la decisión más grande de las tres** y la que dice si somos un registro con todas las
consecuencias o uno a medias. No se toma aquí porque no es una decisión de mecánica: es de
producto.

---

## Fuentes

- [Palantir · how user edits are applied](https://www.palantir.com/docs/foundry/object-edits/how-edits-applied) ·
  [action types](https://www.palantir.com/docs/foundry/action-types/overview) ·
  [allow editing](https://www.palantir.com/docs/foundry/object-link-types/allow-editing) ·
  [edit-only properties](https://www.palantir.com/docs/foundry/object-link-types/edit-only-properties) ·
  [virtual tables](https://www.palantir.com/docs/foundry/data-integration/virtual-tables)
- [Cognite · containers, views, data models](https://docs.cognite.com/cdf/dm/dm_concepts/dm_containers_views_datamodels)
- [Databricks · foreign tables y credenciales](https://docs.databricks.com/aws/en/query-federation/)
