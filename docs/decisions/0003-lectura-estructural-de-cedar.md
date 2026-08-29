# 0003 · Lectura estructural de Cedar

**Estado:** aceptado · **Fecha:** 2026-08-29 · **Decide:** leer la forma, no evaluar la semántica

---

## Contexto

Siete de los veinte casos de `conformance/diff/` comparan políticas Cedar entre dos
versiones de un paquete. Cinco códigos dependen de ello:

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

`ore-core::cedar` lee **identidad, efecto, obligaciones y conjunciones de `when`**. No
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
`context.purpose in ["x"]` dicen lo mismo, y una implementación que las comparase como
cadenas emitiría `OOS5013` —«perdiste una condición»— cada vez que alguien reescribiera la
primera forma en la segunda. El caso `widen-purposes` de la suite es exactamente esa
trampa, y falla si se compara texto.

## Lo que se acepta a cambio

Un fichero Cedar con sintaxis válida pero exótica —anotaciones interpoladas, condiciones
escritas con `||` en lugar de conjunciones— puede leerse peor de lo que un evaluador lo
leería. Es aceptable mientras el emisor de OOS sea quien genera estos ficheros y la suite
sea el árbitro: **un caso de conformidad que esta lectura no supere es un defecto de este
módulo, no una excusa.**
