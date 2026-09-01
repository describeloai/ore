# Handoff · `discover` de punta a punta

> **Este documento es desechable.** Se borra el día que la fila `1` de la tabla de fases se
> ponga en verde. Un plan que sobrevive a su ejecución deja de ser un plan y pasa a ser
> documentación de un pasado que ya nadie comprueba.
>
> Fecha: 2026-09-01 · Reescrito **después** de ejecutar lo que la versión anterior planeaba.
> De cinco prioridades quedan una y media, y la mitad no es código.

---

## 1. Dónde estamos

La cadena de punta a punta de la fase 1 **existe entera y se ejecuta en CI**:

```
ore init  →  ore source add  →  ore discover --source  →  ore review  →  ore validate
   ✅              ✅                    ✅                    ✅             ✅
```

Lo que cerró la versión anterior de este documento:

| | Qué era | Qué es hoy |
|---|---|---|
| **P0** · el eslabón vivo | `discover --source` no tenía **ninguna** prueba | `pruebas-de-fuego/descubrimiento.sh` + su trabajo de CI, con base de datos propia |
| **P2** · `ore review` | no implementado | `revision.rs`, un formulario por clase, `--answers` para CI |
| **P3** · la costura del paquete | `--out` fuera de un repo daba `OOS2004` sin avisar | avisa, y dice cómo se arregla |
| **P4** · las tres pequeñas | medidas y no arregladas | las tres arregladas, con prueba cada una |

### Lo que decide la forma de `review`, y conviene no deshacer

**Contestar no edita: vuelve a inducir.** `inducir` pasó a ser función de tres cosas
—`inducir_con(catálogo, decisiones, vocabulario)`— y `review` es exactamente eso con las
respuestas puestas. La alternativa —abrir `entities/Clientes.yaml` y sustituir el comentario de la
clave que falta— se cae por dos sitios: hay respuestas que **no caben en una edición
local** —resolver una colisión crea dos entidades donde no había ninguna; unir una familia
fechada borra tres ficheros y escribe uno con tres bindings— y un documento retocado deja
de estar garantizado por lo que el inductor garantiza.

De ahí salen los tres apuntes que `discover` deja al lado del paquete, y **los tres son
`.json` a propósito**: `ore validate` carga todo `.yaml` del árbol y le exige `apiVersion`,
así que un apunte con esa extensión rompe el paquete al que pertenece. Lo dijo ejecutarlo.

| | Qué es | Por qué |
|---|---|---|
| `discover.catalog.json` | el catálogo tal y como se leyó | hace `--source` reproducible como `--from`, y **`review` puro**: sin red, sin credencial, sin driver |
| `discover.pending.json` | la cola, con `id` y `options` por decisión | una cola que se puede leer y no contestar no sirve |
| `discover.answers.json` | lo contestado hasta ahora | `paquete = inducir(catálogo, respuestas)`. Sin él, la segunda sentada desharía la primera **en silencio** |

Ese último fichero es la única desviación de *«la respuesta escribe en los ficheros
inducidos, no en un estado aparte»*, y la frase sigue siendo cierta leída como se escribió:
no es un registro paralelo de decisiones tomadas al margen del paquete, es **la entrada de
la que el paquete sale**. Borrar una línea devuelve su decisión a la cola.

### Las once preguntas

Nueve son del catálogo, y son la taxonomía que la versión anterior de este documento midió.
Las otras dos no las hace el catálogo, y las dos salieron de ejecutar el criterio:

| # | Clase | `id` | Se cierra con |
|---|---|---|---|
| 1 | colisión de identificador | `colision/<Nombre>` | un nombre por tabla, o cuál se queda |
| 2 | sin clave primaria | `clave/<tabla>` | las columnas, u `omitir` |
| 3 | columna sin tipo de OOS | `tipo/<tabla>.<columna>` | un tipo —`Money<EUR, 2>` vale—, u `omitir` |
| 4 | ninguna columna tipable | `vacio/<tabla>` | `omitir` |
| 5 | no es una tabla | `vista/<tabla>` | `entidad` u `omitir` |
| 6 | cero filas | `filas/<tabla>` | `mantener` u `omitir` |
| 7 | ¿el mismo concepto? | `concepto/<columna>.<tipo>` | un concepto publicado —se **apunta**— o un nombre nuevo —se **acuña**— o `no` |
| 8 | ¿una relación? | `relacion/<tabla>.<columna>` | `si` o `no` |
| 9 | fragmentadas por fecha | `familia/<raíz>` | `separadas`, `omitir`, o la columna de tiempo |
| 10 | **quién responde** | `dueno/<paquete>` | `team:<handle>` o `user:<handle>` |
| 11 | **cómo se clasifica lo acuñado** | `clasificacion/<concepto>` | `<eje>: <nivel>`, o `sin_clasificar` |

La décima salió de ejecutar el criterio, no de leer: el inductor escribe `owner: cambiame`
porque no puede inventar un handle, y `cambiame` **no valida** —`OOS2009`—. Era la única
decisión entre contestar la cola entera y un paquete en verde, y no estaba en la cola.

La undécima salió de medir qué hacía un concepto acuñado, y la respuesta era **nada**. La
etiqueta de un concepto es la tercera fuente de la clasificación efectiva, y sin ella la
columna que lo habla sale servida en la superficie emitida exactamente igual que si nadie
hubiera contestado. Solo aparece cuando se acuña: apuntar a un concepto publicado hereda su
clasificación, y volver a preguntarla sería reabrir una decisión ajena.

Y el formulario de cada clase vive en un `match` **exhaustivo** en `revision.rs`: una clase
nueva sin formulario no compila, que es la única forma de que el inductor no estrene una
pregunta que nadie sabe contestar.

---

## 2. Lo que queda

### P1 · Publicar un vocabulario mínimo

**El código ya no bloquea: falta el contenido.** La pregunta 7 lee los `kind: Concept` del
repositorio, ofrece los del tipo de la columna —el que se llama igual primero, luego el que
la nombra entre sus `synonyms`— y enseña **la clasificación que se hereda al elegirlos**.
Elegir uno no escribe nada. Acuñar uno nuevo sí, y abre la pregunta 11.

Lo que falta es un paquete de conceptos **que otros importen**, fuera de `conformance/`. La
forma está definida y probada —`v1alpha4/valid/vocabulary-package-has-no-entities`—, así que
esto no es diseño: es elegir el contenido.

**Listo cuando:** apuntar a un vocabulario publicado sea el camino normal y acuñar la
excepción.

**Aviso de alcance, que no ha cambiado:** el criterio es *mínimo*. Acuñar un concepto por
columna repetida es la inflación que `02-property` §6.2 nombra. Empezar por lo que se repite
entre **fuentes distintas**.

**Y una cosa medida de paso, que decide DÓNDE se publica.** Un paquete de vocabulario
co-alojado en el mismo repositorio valida solo —`ore validate packages/gdpr` → verde—, pero
`ore validate .` carga el árbol entero **como un único paquete**, y ahí un concepto que
nadie habla es `OOS9004`: la excepción de la regla es *«un paquete SIN entidades»*, y el
árbol entero tiene. O el vocabulario vive en su propio repositorio y se importa por
`dependencies`, o todo lo que publique tiene que estar hablado. No es un defecto de nada de
lo de arriba, pero se elige una vez y hay que elegirla.

### El criterio de la fase 1, que es medio código y medio persona

> *Apuntar a un esquema sucio de ~50 tablas y que un arquitecto diga «está un 80% bien»
> tras contestar cinco preguntas.*

De las dos mitades, la de código está. La otra **no se puede afirmar desde aquí**: hace
falta un esquema real de ese tamaño y alguien que lo juzgue. Hasta que eso ocurra la fila
`1` sigue en `◐`, y este documento sigue existiendo.

Lo que sí conviene medir cuando se haga, porque es donde se va a notar:

- **Cinco preguntas, no cincuenta.** Un esquema de 50 tablas con `email` en veinte da UNA
  pregunta de concepto, pero puede dar veinte de `filas` y quince de `clave`. Si la cola
  sale con cien decisiones, el problema no es `review`: es que falta agrupar por clase.
- **Cuántas de las cinco son de concepto.** Con vocabulario publicado deberían serlo casi
  todas: son las únicas que valen por todas las apariciones a la vez, y las únicas que
  cambian lo que se sirve.
- **La colisión tapa lo que hay detrás.** Mientras dos tablas colisionan no se emite
  ninguna, así que las preguntas sobre sus columnas no aparecen hasta la segunda pasada.
  Es correcto —no hay documento donde poner ese tipo— y hace la revisión escalonada.

---

## 3. Lo que **no** entra, y sigue sin entrar

- **`drift-detect`**: es la fase 1 también, pero es otro verbo y otro riesgo. No bloquea
  `discover`.
- **El LLM.** El criterio dice *«un arquitecto dice que está un 80% bien tras contestar
  cinco preguntas»*, no *«un modelo lo adivina»*. Lo que `discover` produce es una propuesta
  en `DRAFT`, y la única vía de propuesta a verdad es un commit revisado.
- **Cualquier cosa que meta red en `ore`.** El binario no sabe hablar por la red, y eso es
  una propiedad comprobada por `dependencias.rs`, no una promesa. `review` tampoco: lee el
  catálogo que `discover` dejó, y por eso es puro.
- **Unir una familia sin poder honrarlo.** `familia/<raíz>` con una columna de tiempo emite
  una entidad y un binding por hermana, y se **niega** si el eje no está en todas o si no
  comparten clave primaria: la unión tendría filas sin sitio en el tiempo, o una identidad
  que no es la de ninguna.
