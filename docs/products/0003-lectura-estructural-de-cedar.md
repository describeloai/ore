# 0003 · Lectura estructural de Cedar

**Estado:** aceptado · **Fecha:** 2026-08-29 · **Decide:** leer la forma, no evaluar la semántica

---

## Contexto

Hay casos de `conformance/diff/` que comparan políticas Cedar entre dos versiones de un
paquete: **uno por cada código que depende de ello**, y son estos cinco:

| Código | Qué hay que saber |
|---|---|
| `OOS5013` | un `permit` perdió una condición |
| `OOS5014` | un `forbid` desapareció o se debilitó |
| `OOS5015` | el conjunto de finalidades creció |
| `OOS5016` | `minGroupSize` bajó |
| `OOS5017` | apareció un desclasificador donde no había ninguno |

La opción evidente es enlazar `cedar-policy`, la implementación de referencia. La
alternativa es leer la forma del fichero aquí.

## La distinción que decide

Hay dos preguntas muy distintas que se pueden hacer sobre una política:

1. **¿Este principal puede hacer esta acción sobre este recurso?** Es una pregunta
   semántica, exige un evaluador, y la respuesta correcta es `cedar-policy`. Nadie
   debería reimplementarla.
2. **¿Esta versión concede más que la anterior?** Es una pregunta *sintáctica sobre dos
   textos*. No hay principal, ni acción, ni recurso, ni contexto que evaluar.

`ore diff` solo hace la segunda. Y la segunda no mejora por tener un evaluador detrás: un
evaluador dice si una petición concreta pasa, no si el conjunto de peticiones que pasan ha
crecido — eso último es indecidible en general y, en la práctica, se responde comparando
estructura.

## Decisión

`ore-core::cedar` lee **identidad, efecto, obligaciones, conjunciones de `when`, las
máscaras de `@oosMask` y las etiquetas que la política menciona** —`Label::"…"`—. Las dos
últimas las añadió v1alpha3: la primera para resolver la máscara con sujeto, y la segunda
para responder *«¿hay una política sobre esta propiedad?»* sin evaluar nada. Leer
`Label::"…"` no es un atajo — es el vocabulario que nuestra propia proyección a esquema
Cedar genera. No
evalúa, no resuelve jerarquías de entidades, no valida contra un esquema.

Y el límite queda escrito: **si algún día `ore` necesitara decidir una autorización, la
respuesta es enlazar `cedar-policy`, no ampliar ese fichero.**

## Las dos consecuencias que importan

**`@id` es obligatorio, y ahora se ve por qué.** Sin él, mover una política de línea
parecería borrarla y crear otra: `OOS5014` saltaría en cada reformateo del fichero. La
identidad de una política tiene que sobrevivir a su posición, igual que el digest de un
documento se indexa por `kind:qualifiedName` y no por ruta (`90-canonical-form` §5.2). Es
la misma regla aplicada dos veces.

**Las finalidades se comparan como conjunto, no como texto.** `context.purpose == "x"` y
`["x"].contains(context.purpose)` dicen lo mismo, y una implementación que las comparase
como cadenas emitiría `OOS5013` —«perdiste una condición»— cada vez que alguien
reescribiera la primera forma en la segunda. El caso `widen-purposes` de la suite es
exactamente esa trampa, y falla si se compara texto.

> **Y aquí me equivoqué al escribirlo.** La segunda forma decía
> `context.purpose in ["x"]`, que **no es Cedar válido**: `in` es el operador de jerarquía
> de entidades y `purpose` es un `String`. Vivió en este ADR, en un caso de conformidad y
> en el ejemplo de referencia hasta que un validador de Cedar la miró — porque hasta M0
> nadie había enfrentado una política contra el esquema. La lectura estructural la leía
> igual, que es justo por qué nadie se enteró: **`purposes()` extrae las cadenas
> entrecomilladas y le da igual el operador**, así que el bug no producía ningún síntoma.

## Lo que se acepta a cambio

Un fichero Cedar con sintaxis válida pero exótica —anotaciones interpoladas, condiciones
escritas con `||` en lugar de conjunciones— puede leerse peor de lo que un evaluador lo
leería. Es aceptable mientras el emisor de OOS sea quien genera estos ficheros y la suite
sea el árbitro: **un caso de conformidad que esta lectura no supere es un defecto de este
módulo, no una excusa.**
