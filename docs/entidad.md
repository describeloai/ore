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

Es la única de las seis partes donde la entidad declara algo que el sustrato ya contiene, y el
peldaño que lo cobra —`B0`/`OOS2026`— está medido y en cola.

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

Adelgazarla no es sustituirla. Es el camino que M2 y `B0` ya abrieron por separado, y ahora se ve
que son el mismo:

| | qué quita | precio medido |
|---|---|---|
| **M2** | los 38 nombres que solo repiten; `properties` pasa a **anotar** | 38 de 71, y 25 anotaciones que se quedan |
| **B0** · `OOS2026` | la arista, que ya está en la copia | 2 entidades |

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
