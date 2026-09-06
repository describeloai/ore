# La entidad, medida

> **Estado: medida, no decisión.** Este documento no retira nada ni propone un peldaño. Contesta
> una sola pregunta —*¿qué partes de `Entity` siguen siendo suyas en el paradigma de vistas, y
> cuáles las sabe ya el sustrato?*— con números del corpus, para que la decisión se tome después
> sobre datos y no sobre una intuición.
>
> Guion: [`pruebas-de-fuego/medida-entidad.py`](../pruebas-de-fuego/medida-entidad.py). Se vuelve a
> correr cuando el corpus cambie.

---

## 1. Por qué se mide esto, y por qué la fecha no basta

```text
2026-08-29   02-entity.md      ← el mismo día que 03-binding.md
2026-08-31   05-ejecutor.md
2026-09-02   01-table · 02-view
```

`02-entity` nació el mismo día que el `Binding`, y cruzó la frontera del paradigma con **un campo
añadido**: `00-scope` de v1alpha8 dice literalmente *«`Entity` — sin cambios salvo `apiVersion` y
`backedBy`»*. Todo lo demás se rehízo. Esa asimetría es lo que hay que explicar.

Pero la prueba de la fecha condenó a `03-binding` y a `05-ejecutor` porque **su materia** era justo
lo que el paradigma nuevo sustituyó. La materia de la entidad es otra —qué significa una cosa— y el
sustrato lo dice él mismo, en el sitio donde le niega `labels` a la tabla
([`document.rs:273`](../crates/ore-core/src/document.rs:273)):

> *«Su ubicación la etiqueta el `datasource`, y **lo que significa una columna lo dice la
> entidad**.»*

**El hueco con forma de entidad no es un calzador: es una junta que el sustrato labró a propósito.**
Así que la pregunta no es si sobrevive. Es qué partes.

---

## 2. El sesgo del corpus, que hay que decir antes de enseñar ningún porcentaje

Hay 290 documentos `Entity`, y contar sobre los 290 mide **qué reglas hay**, no qué usa un paquete
de verdad: un caso de conformidad declara lo mínimo que su regla necesita.

```text
conformidad/invalid     101      backedBy  10
conformidad/diff         65      backedBy   0
conformidad/suelto       49      backedBy   0
conformidad/valid        47      backedBy  11
conformidad/canonical    21      backedBy   0
ejemplo (acme-retail)     7      backedBy   2
```

**232 de 290 entidades no tienen respaldo físico de ninguna clase** —ni `Binding` ni `backedBy`—, y
eso no es deuda: son casos que prueban reglas puramente semánticas. Es, en sí, un hecho sobre la
naturaleza de la entidad: **se puede validar sola**, y el corpus lo aprovecha en el 80 % de sus
casos.

El único paquete realista es `examples/acme-retail`, con 7 entidades. Los porcentajes de §3 son
sobre esas 7, y el tamaño de la muestra se dice cada vez.

---

## 3. Las seis partes de §1.3, sobre el paquete realista

| parte | uso | ¿puede decirlo el sustrato? | veredicto |
|---|---|---|---|
| **Significado** · `description`, `aiContext`, `is` | **7/7** | no — vista y tabla tienen **prohibido** clasificar el dato. La vista admite `oos.maturity`, que es su propio estado (`02-view` §4.1) | **suyo** |
| **Identidad** · `primaryKey`, `timeKey`, `uniqueKeys` | **7/7** | a veces, y hoy casi nunca — §5 | **suyo, por ahora** |
| **Historia** · `temporal`, `moved`, `reserved` | **7/7** | no | **suyo** |
| **Conexión** · `relations`, `via` | **5/7** | **sí** — §6 | **el sustrato ya lo sabe** |
| **Sensibilidad** · `labels` en propiedades | **3/7** | no, por diseño | **suyo** |
| **Procedencia** · `derivedFrom` | **3/7** | a medias — compone, no duplica | **suyo, y encaja** |

> **Las seis partes se usan. Ninguna está muerta.** La entidad no está hinchada — no sobra una
> séptima ni sobra una de las seis. Lo que hay que mirar no es *qué parte sobra* sino *qué parte
> dice algo que otro documento ya dijo*, y solo hay una: la Conexión.

Sobre **Procedencia**, que es donde más fácil sería equivocarse: `View.fields` da el linaje
propiedad→columna y `derivedFrom` da propiedad→propiedad. Son **dos aristas distintas del mismo
grafo**, no dos versiones de la misma. `02-entity` §5 justifica `derivedFrom` diciendo que ODCS solo
tiene granularidad de tabla; la vista sí tiene granularidad de columna, pero no computa, y una
propiedad derivada sigue sin tener columna. Componen.

---

## 4. La duplicación real, con número: 38 de 71

M2 de [`sustrato.md`](sustrato.md) ya lo había dicho —*«los nueve nombres duplicados
desaparecen»*—. Contado sobre las **23** entidades que ya tienen `backedBy`:

```text
nombres de propiedad                       71
  que YA son campo de la vista             63   ← 89 %
    y ademas ANOTAN (labels/is/...)        25
    y solo repiten el nombre y el tipo     38   ← 54 % del total
  que NO estan en la vista                  8   ← derivedFrom, o el hueco de OOS2022
```

> **38 de 71 propiedades no dicen nada que la vista no diga ya.** Nombre y tipo, y el tipo es lo
> único que añaden — un `String` frente a un `varchar(16)`.

Las otras **25 sí anotan**: llevan `labels`, `is`, `description`, `derivedFrom`. Esas son
irreductibles, y son exactamente la forma que M2 propone para todas: **anotar un campo en vez de
redeclararlo.** La medida no descubre la dirección; le pone precio y confirma que el 54 % del
trabajo es mecánico.

---

## 5. Identidad · la hipótesis de duplicación **no se confirma**

Parecía el segundo caso claro: `primaryKey` en la entidad y `changes.key` en la tabla dicen la misma
clave con nombres distintos, y la vista traduce entre los dos. Contado:

```text
entidades con primaryKey                278
  sin backedBy - no se puede cotejar    258
  la cadena no llega a una tabla          5
  la tabla NO declara changes.key        12   ← calla
  la copia YA dice la misma clave         3   ← y coincide
  la copia dice OTRA clave                0
```

**Solo hay 20 casos cotejables, y en 12 de ellos la tabla calla.** `changes.key` es legal siempre
desde v1alpha8 pero obligatorio solo con `mode: upsert`, así que el sustrato **normalmente no sabe
la clave**. Donde los dos hablan coinciden —3 de 3, ninguna discrepancia— pero tres casos no
sostienen una regla.

> **`primaryKey` no es redundante hoy.** Podría serlo solo si `changes.key` pasara a ser
> obligatorio, y eso es una decisión sobre la tabla, no sobre la entidad. Lo dije al revés el turno
> pasado; el número dice que no.

Lo que sí queda anotado, sin coste: **cuando los dos hablan, nunca discrepan.** Si algún día se
quiere derivar una de la otra, no hay que reconciliar nada.

---

## 6. Conexión · el sustrato ya la sabe, 4 de 4

```text
relaciones en el corpus                  27
  la entidad no tiene backedBy           23   ← no cotejable
  el via YA es campo de la vista          4   ← 4 de 4
  el via NO esta en la vista              0
```

Cuatro de cuatro, y no por suerte: `via` nombra una **propiedad**, y `OOS2022` obliga a que toda
propiedad sea campo de su vista o declare `derivedFrom`. **La arista está en la copia por
construcción**, que es lo que M4 dedujo y esto confirma sobre todo el corpus en vez de sobre
acme-retail.

Es la única de las seis partes donde la entidad declara algo que el sustrato ya contiene.

> **⚠️ Aquí decía «y el peldaño que lo cobra —`B0`/`OOS2026`— está medido y en cola».** Ni cobraba
> esto, ni sigue en cola. `B0` era una **regla sobre la vista** —*lo que se atraviesa se debe
> materializar*—, no una supresión en la entidad; y se midió y **no se escribe**.
>
> Que no cobre esto no podría ser de otro modo: `via` es **de donde sale** la arista
> —[`sustrato.md` M4](sustrato.md): *«una proyección de dos columnas… por cada relación con
> `via`»*—. Quitarlo no cobraría una redundancia: **borraría el dato**.
>
> Y lo que mató a `B0` es de la misma familia. La arista que el sustrato contiene son **dos
> columnas**, y exigir `materialized` exigía copiar **la carga entera** —otro conducto, otra
> autorización—. En el ejemplo insignia eso no se puede pagar:
> `pruebas-de-fuego/medida-b0-impagable.py`. Así que `relations` no se retira, y tampoco se le
> cobra nada: se queda **exactamente como está**.

---

## 7. El texto, que es donde está el daño

```text
02-entity.md                            481 lineas
  enunciados normativos (DEBE)           17
    de ellos, nombran el binding          2
  menciones de `binding`                 19
  secciones que lo nombran           10 de 27
```

**2 de 17 normas.** El daño no está en las reglas: está en el encuadre, y son tres sitios concretos.

| dónde | qué dice | qué le pasa |
|---|---|---|
| **§1.1** el principio | *«la columna física… pertenecen al binding»* | la otra mitad del corte se llama `View` desde el 2 de septiembre — y es **mejor mitad**: un objeto declarado, no una tabla de mapeo |
| **§1.4** lo que no contiene | *«Consultas y vistas — OOS no las modela en v1alpha1»* | **excluye por escrito lo único sobre lo que hoy se apoya**. Es la firma del sesgo |
| **§9.1** la emisión | *«Emitir una entidad **sin** binding **DEBE** fallar»* | **norma muerta**: obliga contra un `kind` retirado. Es una de las 2 |

---

## 8. El diagnóstico

La entidad no es la abstracción equivocada, y no está hinchada. Es **dos cosas con un nombre** —el
error que este proyecto ya cazó dos veces, con `Property`, que era dos cosas, y con `Binding`, que
era media vista:

> **una declaración de tipo** —significado, etiquetas, `is`, temporalidad, procedencia: cinco
> partes que nadie más puede decir— **más una redeclaración del sustrato**: 38 nombres repetidos y
> una arista que la copia ya contiene.

Y es el problema espejo del que medimos en Cognite: **su view hace dos trabajos** —mapear y ser
tipo lógico con `implements`— y la nuestra hace uno. Aquí es la **entidad** la que hace dos.

Adelgazarla no es sustituirla. Es el camino que M2 abrió, y ahora se ve
que son el mismo:

| | qué hace | precio medido |
|---|---|---|
| **M2** | los nombres que solo repiten; `properties` pasa a **anotar** | 38 de 71 entonces, 67 de 121 hoy — y las anotaciones se quedan |
| ~~**B0** · `OOS2026`~~ | **medido y descartado.** No quitaba y tampoco obligaba: exigía el conducto de la **carga** para copiar **dos columnas** | **cero** — deja la entidad como estaba |

---

## 9. Lo que esta medida **no** decide

- **Si `changes.key` debe volverse obligatorio.** Es lo único que haría derivable `primaryKey`, y
  es una decisión sobre la tabla. Sin medir.
- **Qué pasa con `02-entity` §9.1 y la emisión a Ossie.** Hay una norma muerta y una tabla de
  emisión con tres filas huérfanas. Es trabajo en `C:\oos` y merece su propio peldaño.
- **Si `02-entity` pasa a histórico o se reescribe.** A diferencia de `03-binding` y `05-ejecutor`,
  aquí lo que caduca es el **encuadre**, no la materia — y un documento no se retira por tener el
  prólogo viejo. Reescribir §1.1, §1.4 y §9.1 puede bastar; hay que decidirlo, no suponerlo.
- **`L2`.** Su definición nombra bindings y por eso no puede juzgar a una implementación que solo
  tiene tablas y vistas. Sigue pendiente y es independiente de todo lo anterior.
- **La otra mitad de la migración.** 35 entidades siguen con `Binding` y sin `backedBy`, y
  `acme-retail` solo tiene 2 de 7 migradas. Eso es deuda de corpus, no de modelo.

---

## 10. Y una que sí se decidió, midiéndola: **`Entity` no necesita ser un documento**

> **Estado: medido.** Guion: [`pruebas-de-fuego/medida-lastre-entidad.py`](../pruebas-de-fuego/medida-lastre-entidad.py).
> Esta sección contesta una pregunta que §9 no se hacía, y la contesta con un recuento en vez de
> con una opinión.

El §8 dijo que la entidad es *«dos cosas con un nombre»* y que adelgazarla —M2— la deja en
las cuatro irreductibles. Lo que no se preguntó es lo siguiente:

> **Adelgazada, ¿queda algo que exija un documento aparte?**

### 10.1 · El reparto: ningún campo se queda sin sitio

Cada clave del vocabulario real de `Entity` —`document.rs`, no de memoria— contra un documento de
vista fusionado, sobre las 292 entidades del corpus:

```text
name · namespace     292   se funden con los de la vista
backedBy              25   DESAPARECE — es la dirección entre los dos documentos
nature               287   ┐
primaryKey           286   │
uniqueKeys · timeKey   19  │  anotan la UNIDAD
temporal · moved · reserved 15
implements · principal 19  │
relations             25   ┘  y NO se retira: de ahí sale la arista
properties           291   se funde con `fields`: cada campo, anotado

dentro de cada propiedad, todo anota UN CAMPO de la vista:
type 291 · labels 137 · is 31 · derivedFrom 14 · description 14 · enum 12
expression 8 · required 6 · examples 5 · aiContext 2 · confidence 2 · temporal 1
```

**Diecisiete claves de `Entity`, trece de una propiedad, y todas aterrizan.** La única que no
aterriza es `backedBy`, y no aterriza porque **no significa nada**: es la dirección entre dos
ficheros, y sin dos ficheros no existe.

### 10.2 · El residuo de forma es **uno** en todo el corpus

```text
derivedFrom a una propiedad de la MISMA entidad    25
derivedFrom a la de OTRA                            1
    customers.Customer -> customers.Order.totalAmount
```

Fusionado, esa línea es una referencia entre unidades **que no es `from`**, y el vocabulario de la
vista no tiene ninguna. Es el único sitio donde la fusión pediría gramática nueva.

De 8 propiedades que no son campo de su vista, 2 son `derivedFrom` —que caben— y las 6 restantes
son casos inválidos a propósito que ya no compilan bajo `OOS2022`.

### 10.3 · Y eso corrige una de las dos razones de la partición

`sustrato.md` §3.4 justifica partir en dos con dos motivos: que una vista pueda existir antes de
que nadie modele, **y que varias entidades se respalden de la misma**. El segundo, contado:

```text
vistas que respaldan N entidades:
   1 entidad(es): 25 vistas
vistas distintas respaldando: 25
```

**Uno a uno, sin excepción.** El caso `one-object-many-entities` es v1alpha1, con *bindings*. La
mitad N:1 de la razón **no tiene respaldo empírico en el paradigma de vistas**.

Y la otra mitad se sostiene sin exigir dos `kind`: el caso *«significado sin datos»* ya tiene los
suyos —`Concept` e `Interface`, que no se sientan sobre nada—, y `sustrato.md` §3.4 es normativo al
decir que una entidad **sí** se sienta sobre una vista, *«porque promete filas, y una promesa de
filas necesita quién las conteste»*.

### 10.4 · Lo que cuesta, y no es conceptual

```text
267 de 292 entidades no tienen `backedBy`, y 207 no tienen nada
 25 de  25 parejas resueltas tienen nombres DISTINTOS  (hr.Employee <- empleados)
```

Las 267 son deuda de migración, no una capacidad. Los 25 nombres sí son precio: **cada fusión mata
un nombre**, y hay que poder decir en qué se convirtió. Es exactamente para lo que existe `moved`
— que la vista todavía no tiene.

### 10.5 · Y una colisión, con precedente resuelto

`labels`, `description` y `aiContext` existen **en los dos documentos con sujetos distintos**: en la
vista, el estado del documento —desde [`02-view` §4.1](../vendor/oos/spec/v1alpha8/02-view.md)—; en
la entidad, la clasificación **del dato**, que `flow::propagar` hereda a todas sus propiedades.
Fusionados serían una clave con dos significados, que es el modo de fallo que este proyecto
persigue.

No es un bloqueo: **`Concept` ya las lleva en los dos sitios**, y `document.rs` explica por qué no
se confunden — `metadata.labels` clasifica *este documento*, `spec.labels` clasifica *el dato*. La
unidad fusionada usaría esa misma partición.

### 10.6 · El veredicto, y la escalera que ya estaba

> **`Entity` no es una abstracción: es un fichero de anotaciones.** Después de M2 no queda
> en ella nada que necesite un documento propio, y `backedBy` es el precio de tenerlo.

Lo que impide fusionar hoy **no es el modelo: es que la vista todavía no es ciudadana del
repositorio**. Y eso convierte la escalera que ya estábamos subiendo en la precondición de la
fusión, no en un rodeo:

| | | por qué es precondición |
|---|---|---|
| **1** | madurez en la vista ✅ | la unidad puede decir en qué estado está |
| **2** | la declaración ✅ — `exports`, [`01-package` §3.2](../vendor/oos/spec/v1alpha1/01-package.md) | el paquete dice qué deja usar a otro. Resultó ser **visibilidad**, no membresía: lo segundo lo dice el directorio |
| **3** | `ore diff` ve el sustrato ✅ **entero** — 13 mutaciones mudas → 0 | sin esto, fusionar esconde el cambio donde nadie lo mira. Hecho en el [ADR 0019](decisions/0019-un-cambio-es-un-orden-o-una-identidad.md): `OOS5019`, `OOS5020` y `OOS5007` con el sujeto devuelto, y `OOS5028`/`OOS5029` para el recorte |
| **4** | `moved` en la vista y en el manifiesto ✅ | sin esto, los renombrados de la fusión eran roturas mudas. Y al medirlo salió que `moved` renombraba **miembros**, no documentos: hacía falta el alcance ancho, que es la semántica original de Terraform — `01-package` §3.4 |
| **5** | ~~`B0`~~ · M2 | `B0` **se midió y no se escribe**: exigía el conducto de la carga para copiar dos columnas, y dejaba el ejemplo insignia sin compilar sin que el autor pudiera arreglarlo — `pruebas-de-fuego/medida-b0-impagable.py`. **M2** sigue en pie, y arrastra una pieza que no existe: el mapeo `physicalType` → tipo OOS **en el núcleo** — hoy vive una vez por driver y solo en el descubrimiento |
| **6** | la fusión | ya no es un rediseño: es borrar `backedBy` y mover un fichero |
