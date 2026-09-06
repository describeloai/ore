# El modelo

> **Este documento es el sitio canónico.** Dice qué es cada pieza y de quién responde, y nada
> más: ni cómo se llegó, ni qué se midió, ni qué queda. Cuando otro documento necesite una de
> estas definiciones, **enlaza aquí en vez de repetirla**.
>
> **Y no lleva cifras del corpus.** Se escribió con tres y las tres se movieron el mismo día, al
> añadir seis casos de conformidad. Un documento canónico que caduca con cada fixture es la
> trampa que este existe para cerrar: aquí van las **formas**, y los números viven en la medida
> que los produce y se vuelven a correr.
>
> Existe porque al medirlo salió que no existía: *«la copia es la respuesta»* y *«omitir es
> cerrar»* no estaban enunciados en ningún documento, y *«raíz de lectura»* estaba en cuatro.
> `pruebas-de-fuego/medida-la-documentacion.py`.

---

## 1. Tres documentos, tres preguntas

| | la pregunta que contesta | quién la decide | ¿lleva significado? |
|---|---|---|---|
| **`Table`** | **qué hay ahí fuera** | nadie — es un hecho del origen | no |
| **`View`** | **qué se pregunta** de ese hecho | su `owner` | **no** |
| **`Entity`** | **qué es una fila** de esa respuesta | el modelo | **sí — es la única que puede** |

Y una cuarta cosa que no es un documento: **la copia**. Una vista con `materialized` es la
**respuesta** a su pregunta, materializada — `Q(origen) ⊕ ediciones`
([ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md)). No tiene `kind` propio
porque no es otra cosa: es la misma vista, contestada.

### La tabla es un hecho

Un puntero a una tabla ajena, con dos caras: `reads` —qué admite el origen— y `changes` —qué
cambios emite—. No tiene dueño porque no la decide nadie: describe lo que **existe**. Por eso
`ore discover` puede emitirla mecánicamente y sin inventar.

### La vista es una pregunta

`Q(Table)` o `Q(View)`, y compone. Su vocabulario es exactamente **seleccionar, renombrar,
recortar** — nada más. Tiene dueño, frescura y madurez porque **qué se expone sí lo decide
alguien**.

Y no lleva significado, que es normativo y no una costumbre: sus `labels` solo admiten
`oos.maturity` —el estado del documento—, y **no tipa**. El tipo de un campo baja de la entidad;
lo que ninguna entidad nombra es `String`, que es lo único afirmable de una columna de la que
solo se sabe el nombre.

### La entidad es el significado

Es **el único sitio del repositorio donde hay significado**, y eso es coherente con lo demás:
la tabla no significa nada, la vista tampoco, alguien tenía que llevarlo.

No abstrae la vista ni la esconde: no puede añadir ni quitar un campo (`OOS2022`), y sus
propiedades se llaman como los campos de su vista. Es **la misma forma, clasificada** — y es lo
único que alguien direcciona: Cedar, los rulesets y GraphQL nombran entidades y propiedades.
Ninguno nombra una vista.

Su significado no se queda arriba: **baja**. Las etiquetas viajan por la cadena hasta la copia y
deciden si se puede materializar; los tipos bajan a tipar el plan; una `via` baja a ser el índice
de topología.

---

## 2. La cadena, y la raíz de lectura

Una vista se apoya en otra, y esa en otra, hasta una tabla.

```
View → View → View → Table
```

- **la raíz** es siempre **la tabla**: de ahí salen los hechos y las capacidades;
- **la raíz de lectura** es **la copia más cercana hacia abajo**, o la tabla si no hay ninguna.

La distinción no es de forma. *«Esta vista se materializa»* y *«sus filas salen de una copia»* son
preguntas distintas, y confundirlas se midió: una vista virtual sobre una materializada ya lee de
una copia.

**Y de ahí sale la propiedad que gobierna casi todo lo demás: el significado es de la CADENA, no
de un eslabón.** Una entidad clasifica una vez, la copia ocurre en un eslabón cualquiera, y la
etiqueta tiene que llegar hasta él.

---

## 3. Dos principios, y se usan todo el rato

| | |
|---|---|
| **P2** | **lo derivable no se declara.** El índice de topología son dos columnas que salen de `primaryKey` y `via`: se computa, no se escribe |
| **P4** | **omitir es cerrar, no abrir.** Un conducto sin autorización es `⊥` y no admite nada; una vista sin `materialized` es virtual. La ausencia no es una preferencia |

P4 tiene un corolario que explica media docena de decisiones de este repositorio: **lo que falta
y lo que está bajo se parecen demasiado a lo que está bien.** Por eso las reglas que importan
vigilan la omisión y la rebaja, no el exceso.

---

## 4. Quién responde

`owner` es **quien responde** de un documento: exactamente uno, escrito como handle `team:` o
`user:` para que case con `CODEOWNERS`. No es control de acceso —eso son los conductos y Cedar—:
es responsabilidad, y `ore report` la usa para decir **qué gobierna qué y quién responde**.

Un documento existe **aparte** cuando responde otra persona. Es el criterio de la casa, y está
escrito en el motor: un `Ruleset` es un documento y no un bloque dentro de `Entity` porque *«quien
responde del cumplimiento tiene que poder restringir la ontología sin poder editarla»*.

| | responde de |
|---|---|
| `Package` | el paquete |
| `View` | qué se expone y con qué frescura |
| `ConduitPolicy` | **el techo**: hasta dónde admite cada conducto |
| `Ruleset` | la exigencia regulatoria, y es independiente de a quién apunta |
| `OntologyConfig` | **el suelo**: la clasificación mínima de cada fuente |
| `Lattice` | **la escala**, y desde qué nivel se exige cobertura |

Los dos últimos llegaron los últimos, y por la misma razón que el primero: *un techo del que nadie
responde es un hueco* — y el suelo es el lado silencioso, porque bajarlo desclasifica en cascada
sin dar ningún síntoma.

**`Entity` no tiene `owner`, ni lo admite**, y no es un olvido: pasarse de etiqueta ya arrastra un
responsable —`requiresGovernance` obliga a que una regla cubra la propiedad, y toda regla declara
dueño (`OOS8001`)— y quedarse corto lo acota el suelo, que ahora sí responde.

---

## 5. Lo que se copia, y lo que se sella

Todo lo que sale de su origen pasa por un **conducto** con una autorización, y toda copia es una
salida. Hay tres, y las tres se comprueban al compilar:

| se copia | conducto | quién lo declara |
|---|---|---|
| la **carga** de una vista | `materialization.payload` | `spec.materialized` |
| las **aristas** de una travesía | `materialization.topology` | **nadie** — se deriva de `relations` con `via` (P2) |
| el eje de un `Binding` | el que nombre su eje | el binding, camino de v1alpha1 |

La segunda es la que se lee al revés con más facilidad: **una entidad con propiedades `critical`
se puede atravesar por un conducto que solo admite `low`**, porque por la arista viajan la clave y
el enlace y nada más. El sello no mira la entidad: mira las dos columnas que se copian.

---

## 6. Qué se certifica, y qué solo se anuncia

Una implementación no «es conforme» a secas: lo es **a un nivel**, y no todo lo que hace se puede
certificar. Por eso son dos listas.

| | | |
|---|---|---|
| **niveles** | `L0` validador · `L1` servidor de contexto | se comprueban con **una suite de ficheros**, así que se declaran y se verifican |
| **capacidades** | lectura · materialización · mantenimiento · actuación | se **anuncian y se demuestran**. La especificación no las certifica |

La frontera es una sola frase:

> **Un nivel falla al compilar. Una capacidad falla al responder.**

Y de ahí sale la propiedad que hace que esto sea un estándar y no una plataforma: **toda la
garantía de gobernanza vive en L0**. Un auditor comprueba que un paquete no filtra información
clasificada ejecutando un validador sobre el repositorio, **sin acceso a un solo dato**. Cada vez
que una preocupación cruza de tiempo de ejecución a tiempo de compilación —el sello del índice, el
dueño del suelo— esa garantía crece.

Detalle normativo en [`00-overview` §3.2](../vendor/oos/spec/v1alpha1/00-overview.md).

### Lo que este modelo todavía no sabe escribir

Una entidad servida desde **dos fuentes distintas**. El binding lo expresaba sin esfuerzo
—una entidad admitía N bindings— y lo que lo sustituye no puede: una entidad tiene **un**
`backedBy`, una vista sale de **un** sitio, y el vocabulario **no tiene junta**
([`v1alpha8/00-scope`](../vendor/oos/spec/v1alpha8/00-scope.md) §6 la deja fuera a propósito,
porque una junta trae dos raíces y su precio en la regla de flujo se decide antes de admitir
la operación).

Esto lo atestiguaba un paquete —`casos/dos-familias`— que se retiró con `Binding`. Su README
avisaba de que borrarlo *«habría convertido una limitación real en un hueco invisible»*, así
que el testigo se queda aquí: **no es que falte migrarlo, es que no hay a qué migrarlo.**

---

## 7. Lo que este modelo **no** dice

Cuatro lecturas que se midieron y no se sostienen. Están aquí para que no vuelvan:

| | |
|---|---|
| *«la entidad redeclara el sustrato»* | Se midió tres veces. Lo redundante son **nombres, y solo nombres**: de las propiedades que solo declaran tipo, dos tercios sostienen la clave primaria, una `via` o una derivada. El tipo, la etiqueta, la clave y la arista no los dice nadie más |
| *«la arista ya está en la copia, así que `via` sobra»* | `via` es **de donde** el índice deriva la arista. Quitarla borra el dato |
| *«lo que se atraviesa se debe materializar»* | Pedía el conducto de **la carga** para copiar **dos columnas**. Lo que faltaba era sellar el índice, no prohibir la travesía |
| *«fusionar entidad y vista es mover un fichero»* | El significado es de la cadena y la vista es un eslabón. La fusión sigue siendo una decisión abierta, no una mudanza |

---

## 8. Dónde está lo demás

Este documento no cuenta cómo se llegó a nada. Eso vive en:

| | |
|---|---|
| [`sustrato.md`](sustrato.md) | la tesis del sustrato y sus medidas |
| [`entidad.md`](entidad.md) | qué persiste de `Entity`, peldaño a peldaño |
| [`ontologia-como-repositorio.md`](ontologia-como-repositorio.md) | la formulación de producto, y el cotejo con Cognite, Foundry y Dremio |
| `pruebas-de-fuego/medida-*.py` | cada número afirmado arriba, reproducible |
| `vendor/oos/spec/` | lo normativo. **Manda sobre esto** |
