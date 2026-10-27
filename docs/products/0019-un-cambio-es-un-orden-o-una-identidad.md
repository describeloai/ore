# 0019 · Un cambio es un orden o una identidad

**Estado:** **aceptado** · **Fecha:** 2026-09-05 · **Decide:** de dónde sale un código de
compatibilidad — y, con ello, cuántos hacen falta para meter el sustrato en `ore diff`

> Nace de una pregunta que parecía de contabilidad —*¿cinco códigos nuevos o menos?*— y resultó ser
> de naturaleza. Lo que se decide aquí no es una lista: es **de qué se deriva la lista**.

---

## El problema

`ore diff` no veía el sustrato. Medido con mutaciones de una sola variable sobre casos conformes
—[`medida-diff-sustrato.py`](../../pruebas-de-fuego/medida-diff-sustrato.py)—: **13 de 19 validaban
en verde y `diff` no decía nada**, incluido invertir el `where` de una vista raíz, que cambia qué
filas son la respuesta para todos sus consumidores.

La pregunta obvia era cuántos códigos escribir. Y no tenía respuesta obvia: el criterio de la casa
es **un código por síntoma, no por causa** —`OOS5023` agrupa seis causas a propósito—, así que
contar campos no sirve. Hacía falta saber **qué es un cambio**.

---

## Lo que ya estaba escrito, y solo era la mitad

`91-versioning` §4 tiene el dibujo desde v1alpha1:

```text
◀── restringir ──────────────────── relajar ──▶
rompe al CONSUMIDOR                 rompe la GOBERNANZA
```

Dos direcciones de **un orden**. Es la contravarianza de Liskov —*debilita precondiciones, refuerza
postcondiciones*— y es lo mismo que Confluent formaliza como `BACKWARD`/`FORWARD`: un orden, dos
direcciones, y cuál importa depende de quién se mueve primero.

**Y el sustrato no cabe en ese dibujo.** Repuntar una vista a otro objeto no restringe ni relaja:
**sustituye**. Por eso el eje `INDEX` existía aunque la cabecera solo nombrase dos direcciones.

---

## Decisión A · hay una tercera clase, y no tiene dirección

> **Un cambio es un movimiento en un ORDEN o una sustitución de una IDENTIDAD. No hay un tercer
> caso.**

Clasificados los 25 códigos `OOS5xxx` que había, **ninguno se queda fuera**:

| clase | cuántos | cuáles |
|---|---|---|
| **sustitución** | 6 | `5006` clave · `5010` unidad · `5018` clave de join · `5019` binding · `5020` materialización · `5027` `via` |
| **orden** | 17 | de ellos **dos pares espejo** —`5009`/`5011` y `5012`/`5026`— y trece de una sola dirección |
| meta | 2 | `5021` y `5022`: no son cambios, son la comprobación sobre los cambios |

Y la clasificación no se impone: **explica** dos cosas que el registro ya hacía sin decir por qué.

**Por qué unos tienen espejo y otros no.** Lo tienen los órdenes cuyas **dos** direcciones se
observan; no lo tienen los de una sola, porque añadir una propiedad o ensanchar un tipo es
genuinamente seguro. `OOS5026` *«estuvo veinticinco códigos sin existir»* — el espejo que faltaba se
encontró tarde, **y esta partición lo habría predicho**.

**Por qué los seis de sustitución son los del plano físico.** Clave, unidad, clave de join, binding,
materialización, `via`. Son los vecinos del sustrato, y que meter el sustrato añada sobre todo
sustituciones no es casualidad: es lo que el sustrato es.

---

## Decisión B · el eje no es una categoría: es **un público**

`91-versioning` §3 lo decía y se leía como taxonomía:

> *«Un cambio no es rompedor en abstracto: **lo es respecto a alguien**.»*

Y su tabla tiene una columna que se llama *«a quién afecta»*: aplicaciones y agentes; seguridad y
cumplimiento; operación del runtime; otros equipos. **Son las cuatro personas que pueden sufrir un
cambio**, no cuatro clases de cambio.

De ahí sale la regla de asignación, y está escrita en el motor desde antes de este documento:

```rust
// una etiqueta que se mueve
let code = if j > i { Code::Oos5009 } else { Code::Oos5011 };
let axis = if j > i { Axis::Consumer } else { Axis::Policy };
```

> **Una comparación, y la DIRECCIÓN elige a la vez el código y el público.** El par espejo no existe
> porque haya dos direcciones observables: existe porque **cada dirección le duele a otro**.

Y el reverso, comprobado sobre los seis de sustitución:

> **Una sustitución no tiene dirección, así que tiene UN eje — y el eje lo decide de quién era la
> identidad sustituida.** La que el consumidor nombra es `CONSUMER` —`5006`, `5010`, `5027`—; la que
> produce el artefacto materializado es `INDEX` —`5018`, `5019`, `5020`—.

Con eso, un código deja de elegirse:

```text
código  =  (orden, dirección)   ó   (identidad, de quién era)
eje     =  lo elige la dirección · o el dueño de la identidad
```

**Y los cuatro ejes bastaron para el sustrato**, que es la comprobación de que son públicos y no
planos: el sustrato no trajo un público nuevo.

---

## Decisión C · lo que había que hacer no era extender `diff`: era repararlo

El eje `INDEX` pregunta *«¿sigue siendo válido el artefacto materializado?»* — **una pregunta sobre
el plano físico, desde v1alpha1**, y era contestable porque el `Binding` llevaba dentro `source` y
`materialization`.

> **v1alpha8 no le añadió un plano a `diff`. Le movió el plano de debajo.** El binding se partió en
> `Table` + `View` y `diff` se quedó mirando a `Kind::Binding`: el eje no dejó de existir, **dejó de
> tener sujeto**.

Eso explica lo que no encajaba: por qué `OOS5019` y `OOS5020` tienen el texto normativo correcto con
el sujeto equivocado. **La regla nunca fue sobre el binding — era sobre el plano físico, y el
binding solo era donde ese plano vivía.**

De las diez mutaciones mudas, **cinco se cubren devolviendo un sujeto** —`5001`, `5007`, `5019`,
`5020`— y el resto pide código nuevo. **Un solo sitio del sustrato es genuinamente nuevo**: el
`where`. El `selector` del binding recortaba y `diff` nunca lo comparó, y además es **el único
cambio del modelo que el análisis de flujo no puede ver por construcción** — `flow` clasifica
columnas y un recorte mueve filas: cambiar `[ES, PT]` por `[ES]` no mueve una sola etiqueta.

Por la Decisión A lleva par espejo: `OOS5028` estrecha —`CONSUMER`— y `OOS5029` ensancha —`POLICY`—.

---

## Lo que se aceptó a cambio

**Un cambio incomparable emite dos códigos.** `false` por `true` no es más estrecho ni más ancho:
pierde filas y gana filas. Se decidió **no** inventar un tercer código para el caso disjunto — los
dos que hay lo describen entero— a cambio de que un informe pueda tener dos entradas para una línea
cambiada.

**Se compara el efecto y no la sintaxis, y eso cuesta resolver la cadena.** El recorte se acumula en
**columnas físicas de la raíz**, así que un renombre en un eslabón intermedio no inventa un cambio
—hay prueba— pero un recorte de la vista de abajo sale en **todas** las de arriba. Es información,
no ruido: a esas vistas les pasa. El precio es que un informe crece con la profundidad de la cadena.

**`diff` no puede usar el digest del plan, y no es una preferencia.** `ore-view` depende de
`ore-core` y `diff` vive en `ore-core`: usarlo sería un ciclo. Se compara estructuralmente, que es
lo que ya se hacía con el `Binding`. La consecuencia aceptada es que **dos escrituras equivalentes
del mismo recorte deben converger por la forma canónica**, no por el álgebra.

**Y quedó deuda dicha, no escondida** —aflojar `freshness` y estrechar `reads`/`changes`—, **saldada
en el peldaño siguiente** con `OOS5030`, `OOS5031` y `OOS5032`. Con ellos las mutaciones mudas
pasan de 13 a **cero**.
Perder un campo de una vista **ya no**: la vista tiene `moved` desde
[`02-view` §4.2](../../vendor/oos/spec/v1alpha8/02-view.md), y con la válvula puesta `OOS5001` ganó
el sujeto. Era la única de las tres que esperaba a otra cosa.

---

## Lo que este documento no decide

- **Si un `where` que se ensancha es solo `POLICY` o también algo más.** Servir filas que el
  contrato excluía es *«conceder más en silencio»*, pero **el retículo no ve filas**: ninguna
  etiqueta se mueve. Es el primer sitio del modelo donde *relajar* ocurre fuera del plano que el
  gobierno sabe mirar, y merece su propia medida.
- ~~**Cuántos códigos piden `freshness` y las capacidades.**~~ **Medido, y la pregunta estaba mal
  planteada**: presuponía que son la misma clase de cambio, y no lo son. `ore discover` emite
  `reads` y `changes` desde el catálogo y **no** emite `freshness` —*«sería exactamente
  inventar»*—, así que aquello es un **hecho** y esto una **promesa**.

  Son **tres**: `OOS5030` la promesa que se afloja —`CONSUMER`, bloquea—, `OOS5031` lo que la
  fuente admite y `OOS5032` lo que emite —`INDEX`, informan—. El primero se separa por **público**,
  y los otros dos entre sí por **remedio**: replanificar frente a rehacer la copia. Los dos
  criterios ya estaban en este documento; lo que faltaba era no meter un hecho y una promesa en la
  misma bolsa.
