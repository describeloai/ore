# La entidad, medida

> **Las definiciones del modelo viven en [`modelo.md`](modelo.md).** Este documento cuenta **cómo se llegó**;
> aquel, **qué hay**. Si los dos dicen algo distinto, manda `modelo.md` — y es un fallo que hay que
> cerrar, no una diferencia de matiz.

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

**216 de 323 entidades no tienen respaldo físico de ninguna clase** —ni `Binding` ni `backedBy`—, y
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

## 4. La duplicación que se creyó real: ~~38 de 71~~

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

> ⚠️ **Esa conclusión se midió después y no se sostiene.** El error está en la última frase: *«el
> tipo es lo único que añaden»* es cierto, y por eso mismo **no sobra** — la vista es física y no
> tipa, así que la propiedad es la única fuente del tipo. Quitarla no cobra una repetición: deja el
> campo en `String` **sin un solo error**.
>
> Recontado: **82** propiedades solo declaran su tipo, y **57 de ellas sostienen** la clave
> primaria (49), una `via` (5) o el origen de una derivada (3). Lo que sobra son **25 nombres**
> —no 25 propiedades— y quitarlos es `OOS2022` con los papeles cambiados.
>
> La cifra se mueve con el corpus: subió de 76 a 82 al añadir seis casos de conformidad en un solo
> día. Lo que no se mueve es la proporción —**dos tercios sostienen algo**— y por eso es la
> proporción, y no el número, lo que está en [`modelo.md`](modelo.md).
> `pruebas-de-fuego/medida-m2.py`.

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

> ⚠️ **La segunda mitad se midió, y no era media entidad: eran 22 nombres.**
> `pruebas-de-fuego/medida-que-queda-de-la-entidad.py`.
>
> La arista **no** la contiene la copia — `via` es de donde el índice la deriva, y quitarla borra
> el dato; eso mató a `B0`. Y de los «38 nombres repetidos», hoy 76: **54 sostienen** la clave
> primaria, una `via` o el origen de una derivada, y de los 22 que quedan sobra **el nombre**, no
> la propiedad — el tipo sigue siendo la única fuente, porque *la vista no tipa*.
>
> El diagnóstico acertó la forma —había dos cosas— y se equivocó en el tamaño de la segunda. La
> entidad no es «dos cosas con un nombre»: es **una**, con 22 nombres de más.

Y es el problema espejo del que medimos en Cognite: **su view hace dos trabajos** —mapear y ser
tipo lógico con `implements`— y la nuestra hace uno. Aquí es la **entidad** la que hace dos.

Adelgazarla no es sustituirla. Es el camino que M2 abrió, y ahora se ve
que son el mismo:

| | qué hace | precio medido |
|---|---|---|
| **M2** | ~~los nombres que solo repiten~~ → **22 nombres**, y solo el nombre | de 76 que solo tipan, 54 sostienen clave, `via` o derivada. Y quitar el tipo degrada en silencio: `quantity` pasa de `Integer` a `String` **sin un error** |
| ~~**B0** · `OOS2026`~~ | **medido y descartado.** No quitaba y tampoco obligaba: exigía el conducto de la **carga** para copiar **dos columnas** | **cero** — deja la entidad como estaba. Lo que sí salió de ahí es el sello del índice, encendido en `04-flow` §4.2 |

---

## 9. Lo que esta medida **no** decide

- **Si `changes.key` debe volverse obligatorio.** Es lo único que haría derivable `primaryKey`, y
  es una decisión sobre la tabla. Medido a medias: hoy lo declaran **8 de 61** tablas, así que
  derivarlo de ahí serviría para el 13 % y haría obligatorio un campo que hoy es opcional en el 87 %
  restante. La decisión sigue sin tomar.
- **Qué pasa con `02-entity` §9.1 y la emisión a Ossie.** Hay una norma muerta y una tabla de
  emisión con tres filas huérfanas. Es trabajo en `C:\oos` y merece su propio peldaño.
- **Si `02-entity` pasa a histórico o se reescribe.** A diferencia de `03-binding` y `05-ejecutor`,
  aquí lo que caduca es el **encuadre**, no la materia — y un documento no se retira por tener el
  prólogo viejo. Reescribir §1.1, §1.4 y §9.1 puede bastar; hay que decidirlo, no suponerlo.
- ~~**`L2`.**~~ **Resuelto, y no como se esperaba.** Se midió, y el problema no era que nombrara
  bindings: era que L2 y L3 **no son niveles de conformidad**. La propia especificación ya lo decía
  tres documentos más allá —una comprobación sobre datos *«no es certificable por una suite de
  ficheros»*—, así que la tabla prometía una escalera con dos peldaños que no se podían pisar. Hoy
  son dos tablas: **niveles** (L0, L1) y **capacidades** (lectura, materialización, mantenimiento,
  actuación). `pruebas-de-fuego/medida-los-niveles.py`.
- **La otra mitad de la migración.** **268 de 320** entidades siguen sin `backedBy`, y
  `acme-retail` solo tiene 2 de 7 migradas. Eso es deuda de corpus, no de modelo — pero se nota en
  cada medida: el corpus de conformidad no ejercita el paradigma nuevo casi en ningún sitio, y por
  eso las medidas de estos cinco peldaños han tenido que apoyarse en el único ejemplo completo.

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
268 de 320 entidades no tienen `backedBy`     deuda de migración, no capacidad
 52 de  52 parejas resueltas son 1:1          hoy nada se pierde por cardinalidad
 52 de  52 tienen nombres DISTINTOS           `hr.Employee` ← `empleados`
```

Las 268 son deuda de migración. Los 52 nombres sí son precio: **cada fusión mata un nombre**, y hay
que poder decir en qué se convirtió — para eso existe `moved`, que la vista **ya tiene** desde el
peldaño 4.

Y el 1:1 hay que leerlo con cuidado: dice que hoy no se pierde nada, no que no se pierda nada. La
gramática admite n:1 y el propio ejemplo dice para qué —*«dos entidades pueden respaldarse de la
misma sin duplicar el mapeo»*—. Lo que se perdería es una capacidad que nadie ha usado todavía.

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

> ⚠️ **Y esto también se midió, quitando una entidad de verdad.** Sin `supply.Shipment` el
> repositorio **compila igual** — y sus nueve campos pasan a `String`, sus dos aristas dejan de
> existir, y el análisis de flujo se queda sin nada que sellar.
>
> De los trece campos de su gramática, el sustrato podría decir **dos y medio** —`primaryKey`
> (`changes.key` lo dice en 8 de 61 tablas), `uniqueKeys`, y el nombre de una propiedad—. Los
> otros diez no tienen dónde vivir: el tipo, la etiqueta, la arista, el tiempo, la forma, la
> flecha.
>
> Así que no es un fichero de anotaciones: es **el único sitio del repositorio donde hay
> significado**, y eso es exactamente coherente con lo demás — la tabla es un hecho y no significa
> nada, la vista es una pregunta y no lleva significado. Alguien tenía que llevarlo.
>
> Lo que sigue en pie del §10 es lo otro: que no necesite un **documento** propio no es lo mismo
> que no haga falta. La fusión sigue siendo mover el significado a la vista, no borrarlo.

Lo que impide fusionar hoy **no es el modelo: es que la vista todavía no es ciudadana del
repositorio**. Y eso convierte la escalera que ya estábamos subiendo en la precondición de la
fusión, no en un rodeo:

| | | por qué es precondición |
|---|---|---|
| **1** | madurez en la vista ✅ | la unidad puede decir en qué estado está |
| **2** | la declaración ✅ — `exports`, [`01-package` §3.2](../vendor/oos/spec/v1alpha1/01-package.md) | el paquete dice qué deja usar a otro. Resultó ser **visibilidad**, no membresía: lo segundo lo dice el directorio |
| **3** | `ore diff` ve el sustrato ✅ **entero** — 13 mutaciones mudas → 0 | sin esto, fusionar esconde el cambio donde nadie lo mira. Hecho en el [ADR 0019](decisions/0019-un-cambio-es-un-orden-o-una-identidad.md): `OOS5019`, `OOS5020` y `OOS5007` con el sujeto devuelto, y `OOS5028`/`OOS5029` para el recorte |
| **4** | `moved` en la vista y en el manifiesto ✅ | sin esto, los renombrados de la fusión eran roturas mudas. Y al medirlo salió que `moved` renombraba **miembros**, no documentos: hacía falta el alcance ancho, que es la semántica original de Terraform — `01-package` §3.4 |
| **5** | ~~`B0`~~ · ~~M2~~ | Los dos medidos, y ninguno adelgaza la entidad. `B0` pedía el conducto de la **carga** para copiar **dos columnas**, y de ahí salió lo que sí faltaba: el **sello del índice**, encendido en `04-flow` §4.2 con tres casos. **M2** es 22 nombres, no 38: el resto sostiene la clave, una `via` o una derivada, y el tipo no lo repite nadie. Lo que queda de M2 es `OOS2022` con los papeles cambiados, sin pieza nueva |
| **6** | ~~la fusión~~ — **cerrada** | No estaba bloqueada: era un **error de categoría**. La vista es π∘σ —un renombre y un recorte— y un renombre no puede crear significado. §10.8 |

### 10.7 · El peldaño 6, medido — y el criterio no había que inventarlo

Esta escalera se subió entera para llegar aquí, y el último peldaño nunca se había medido. Al
medirlo salieron tres cosas.

**Una que la apoya, y es la única que sobrevivió.** El criterio de cuándo algo merece documento
propio está escrito en el motor, en la ayuda de la regla que obliga a `Ruleset` a tener dueño:

> *«es independiente del dueño de los paquetes a los que apunta: **ahí está la razón de que esto
> sea un documento y no un bloque dentro de `Entity`**. En un entorno regulado, quien responde del
> cumplimiento tiene que poder restringir la ontología sin poder editarla.»*

**Un documento existe aparte cuando responde otra persona.** Y `Entity` **no tiene `owner`, ni lo
admite**; `View` lo exige. Por el criterio de la casa, la entidad no merece documento propio.

**Una que la bloquea, y es una sola.**

> **La entidad es de la CADENA, no de un eslabón.**

Clasifica una vez y su etiqueta baja hasta la copia, esté donde esté — `flow::vistas_materializadas`
sube las etiquetas por dos vías justo para eso, y su comentario dice qué pasa sin la segunda:
*«una entidad puede declarar `nationalId: high` sobre una vista de tres eslabones, y **la de abajo,
que es la que se materializa, no lo sabe**»*. Meter el significado en un `kind: View` lo clava en un
eslabón, y la que se copia es la de abajo. **Eso no es mover un fichero.**

Y ahí está la diferencia con Cognite, que no es de gusto: **ellos son dueños del almacenamiento y
nosotros federamos.** Un *container* es suyo y no hay cadena, así que su `view` puede mapear y
significar a la vez. Nosotros tenemos `View → View → View → Table` y la copia ocurre en un eslabón
cualquiera. Adoptar su forma sin tener su premisa es lo que este peldaño estaba a punto de hacer.

**Y una tercera, que no era de la fusión y valía por sí sola.** Nadie direcciona una vista —Cedar,
los rulesets y GraphQL nombran entidades— y **lo que fija el mínimo no respondía ante nadie**. Eso
se midió aparte y se cerró: `04-flow` §3.3, `owner` en `OntologyConfig` y en `Lattice`.

### 10.8 · El peldaño 6, cerrado — no por una decisión, por una categoría

El bloqueo de §10.7 describía un **mecanismo**: *«la de abajo, que es la que se materializa, no lo
sabe»*. Y ese mecanismo estaba roto, no era una propiedad del modelo — `flow::vistas_materializadas`
solo resolvía la cadena hacia abajo. Se arregló y se probó en las dos direcciones
(`el_sello_da_lo_mismo_suba_o_baje_la_cadena`), así que el peldaño volvió a estar abierto y hubo
que volver a preguntar.

La respuesta que vino no fue *«ahora sí se puede»*. Fue que la pregunta estaba mal hecha.

**La vista es π y σ, y nada más.** Su vocabulario en v1alpha8 es `from`, `fields`, `where` —
restringir filas y renombrar columnas—, y eso **no se buscó: se descubrió al migrar**
([`00-scope` §6.1](../vendor/oos/spec/v1alpha8/00-scope.md)). Tres razones independientes cierran
la misma frontera: el precio en la regla de flujo, la invertibilidad de `Q⁻¹` para poder escribir
a través, y la mantenibilidad incremental —que es por donde Snowflake llegó al mismo fragmento.

Y de ahí sale el cierre, que no es una preferencia:

> **Un renombre no puede crear significado.** Si `fields: {employeeId: worker_id}` es una
> biyección, entonces `employeeId` significa exactamente lo que signifique `worker_id`. La vista
> es **transparente por construcción**. Un significado colgado ahí cuelga de un alias.

Mirado desde el repositorio entero, lo que hay es una **escalera de cuatro peldaños**, y ninguno
sobra porque cada uno contesta otra pregunta:

| | contesta | lo que declara |
|---|---|---|
| `datasources[].labels` | **dónde vive** | fija el **suelo** de todo lo que salga de esa fuente |
| `Table` | **qué se puede hacer** | `columns`, `reads`, `changes` — capacidad, no significado |
| `View` | **qué parte, y cómo se llama** | π y σ, el fragmento invertible |
| `Entity` | **qué es** | etiquetas, clave, relaciones, el retículo |

Los tres primeros están ocupados por algo que **no es significado**: una ubicación, una capacidad
y un renombre. Así que:

> **La fusión no es una decisión bloqueada: es un error de categoría.** Si se retira `Entity`, el
> significado no tiene dónde caer.

Con eso caen los tramos 0 a 3 del espectro medido en `pruebas-de-fuego/medida-espectro-fusion.py`.
El tramo 0 —*«DECIDIR que la vista puede llevar significado»*— tiene respuesta, y es que no.

**Y el criterio de §10.7 sigue siendo cierto, y ahora es subordinado.** *«Un documento existe
aparte cuando responde otra persona»*, y `Entity` no admite `owner` mientras `View` lo exige. Eso
sigue diciendo que la entidad no merece **documento** propio. No dice que quepa dentro de una
pregunta: quién responde y dónde cabe el significado son dos cosas distintas, y la segunda es la
que decide si la fusión es posible.

**Lo que sobrevive del peldaño 6** es lo único que siempre fue real: `backedBy` es una flecha de
la entidad a la vista y podría ser la contraria. Eso es mover un puntero, no fundir dos documentos.

#### Y el tramo 4, que además era falso

El espectro decía: *«218 entidades llegan por `Binding`, que no caduca, así que `Entity` tendría
que convivir para siempre»*. Se derivó de «no tienen `backedBy`» y se **leyó** como «llegan por
binding». Contando los bindings de verdad
(`pruebas-de-fuego/medida-lo-que-se-simplifica.py` §D):

- quedan **tres** ficheros `kind: Binding` en todo el corpus, los tres en `docs/vision/`, que
  `examples.rs` excluye a propósito **porque no valida**;
- **213 de 218** de esas entidades no tienen binding en su paquete: no llegan por el camino viejo,
  no llegan por ninguno;
- **218 de 218 son `<a8`** — la única excepción es la fixture de `unsupported-apiversion`, cuyo
  objeto es que la versión se rechace;
- en `acme-retail` son **5 de 7** —`Customer`, `Order`, `Department`, `Sku`, `Supplier`— sin
  sustrato ninguno, y `ore validate` dice `ok · sin errores`. `normalize::sin_respaldo` existe y
  **solo la leen los emisores**.

O sea que `Entity` no sobrevive porque una versión vieja no caduque. Sobrevive porque es el único
sitio de la escalera donde cabe un significado. Y queda un hueco nombrado, que es de otro peldaño:
**ninguna versión, incluida v1alpha8, exige que una entidad tenga vista.**

#### Lo que no cierra esto

El argumento de la política —*Cedar nombra `hr.Employee.nationalId`, y con el significado en la
vista una segunda vista sobre la misma tabla quedaría sin gobernar*— es **estructural y hoy no
tiene sujeto**: el corpus tiene **127 bases con exactamente una vista cada una**, cero abanico. Se
deja escrito para que nadie lo cite como si estuviera ejercido. Lo que cierra el peldaño es la
transparencia de π∘σ, que no depende de cuántas rutas haya.
