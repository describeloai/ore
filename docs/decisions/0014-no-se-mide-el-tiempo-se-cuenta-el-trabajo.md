# 0014 · No se mide el tiempo, se cuenta el trabajo

**Estado:** aceptado · **Fecha:** 2026-09-02 · **Decide:** cuál es la unidad de coste del motor
de vistas, y qué salió al usarla

---

## El problema

El Cost Model llevaba desde M6 diciendo de sí mismo que era **una forma sin medidas**: la
decisión estaba construida, los números no eran nuestros. El 5 % venía documentado de Snowflake
y se ofrecía con su procedencia; los coeficientes eran unos y decían serlo.

Medir parecía trivial y no lo era, porque este proyecto **no puede leer el reloj**. El
invariante III dice que la compilación es pura; `dependencias.rs` veta `chrono` y `time` con
todas las letras; el ejecutor vivía fuera del binario precisamente por saber qué hora es —y se
retiró, pero el invariante que lo empujaba fuera sigue comprobándose. Una
calibración con un cronómetro habría metido por la puerta de atrás lo que el proyecto expulsa
por la principal.

Y hay una segunda razón, que resulta ser la buena:

> **Un reloj mide la máquina de quien mide.** Su caché, su carga, su compilador, su día. Dos
> ejecuciones dan dos números, y entonces la cifra no se puede afirmar — solo contar.

## Decisión

> **La unidad de coste es una fila mirada por un operador.**

Es la unidad clásica de un optimizador —*tuples processed*— y tiene las tres propiedades que
hacen falta: es **exacta**, es **entera** y es **reproducible**. La misma entrada da el mismo
número en cualquier máquina, así que una medida cabe en un `assert_eq!` y deja de ser la
anécdota de una tarde.

Cada operador cuenta lo que mira; la suma es `Circuito::trabajo()`. Y el otro lado de la
balanza se cuenta igual: `recomputar_contando` devuelve el resultado **y lo que costó**, en la
misma unidad, sobre los mismos datos. Comparar dos números medidos así es toda la diferencia
entre calibrar y adivinar.

## Lo que salió al medir, que no era lo esperado

### 1 · La incrementalización estaba escrita y no ocurría

Al contar el trabajo de un paso salía **proporcional a la base**, que es exactamente lo que la
incrementalización existe para evitar. La causa: los integradores eran multiconjuntos planos.
`I(a) ⋈ Δb` recorría el integrador entero en cada paso, y el del agregado se releía completo
para encontrar «los grupos que el Δ toca».

Lo peor es que **estaba escrito que no era así**. El Refresh Analyzer viene enumerando desde M6
un estado que dice *«I(izquierda) indexada por [pais]»* y *«un acumulador por grupo»*. La
descripción era correcta; la implementación, no.

> **Medir una pieza es la forma más barata de descubrir que no hace lo que dice.**

Los dos integradores pasan a estar partidos por su clave, y los tres términos de la junta se
recorren **por el lado del delta**. Hay una prueba de que indexarlos no cambió ni una fila.

### 2 · Mantener gana siempre, salvo en el agregado

Con los integradores arreglados, sobre una base de mil filas:

| forma | integradores | Δ=1 · mantener | Δ=1 · recomputar | cruce |
|---|---|---|---|---|
| filtra y proyecta | 0 | 2 | 1 952 | **no hay** |
| junta por una clave | 2 | 5 | 3 023 | **no hay** |
| suma por grupo | 1 | 103 | 2 002 | **20 filas · 2 %** |

Un paso lineal mira el delta; uno de junta, el delta y lo que empareja. **Ninguno mira la
base.** El único operador cuyo paso mira filas que no venían en el delta es el agregado, que
tiene que releer los grupos tocados — antes y después, para restar.

### 3 · Dónde se cruza el agregado es **dato**, no plan

El mismo documento, cambiando solo cuántas filas tiene un grupo:

| grupos | filas por grupo | cruce |
|---|---|---|
| 20 | 50 | **20 filas — 2,0 %** |
| 250 | 4 | **223 filas — 22,3 %** |

Y el 5 % de Snowflake cae **entre los dos**. No es una cifra mala: es una cifra que no puede
ser buena para las dos, porque la diferencia no está en el plan y un umbral solo ve el plan.

## La consecuencia: `Politica::Trabajo`

Si el cruce depende de los datos, la única forma de acertar es **medir en vez de extrapolar**.
`Medida` gana un campo `trabajo` —lo que el paso costó de verdad— y hay una política que
compara eso contra `filas_base × por_fila_recomputo`. Las dos cifras son filas miradas, así que
se restan sin convertir nada.

Y `por_fila_recomputo` también se mide, en un sitio que ya se paga: **la carga inicial de una
sesión de mantenimiento es un recómputo**. Lo que costó, dividido por las filas que entraron,
es el coste por fila de recomputar *esa* vista sobre *esos* datos. `ore-maintain` lo saca de su
primer paso y decide con él a partir del segundo, sin que nadie declare un número.

Sin `trabajo` medido, esa política **no adivina**: lo dice y mantiene. Una política que se
llama «lo medido» y estima sería la peor clase de número escondido.

## Lo que se acepta a cambio

- **Filas miradas no es segundos.** Un operador que mira una fila con veinte columnas cuesta
  más que uno que mira una de dos, y esta unidad no lo ve. Sirve para comparar **dos caminos
  sobre los mismos datos**, que es exactamente la pregunta del Cost Model, y no sirve para
  prometer latencias.
- **Se mide la máquina de referencia.** Un ejecutor que corriera el circuito sobre otra cosa
  —un almacén columnar, un índice en disco— tendría otros números. Lo que no cambia es el
  **método**: `tests/medidas.rs` es el sitio donde se vuelve a medir, y las cifras están
  afirmadas para que un cambio las rompa en vez de deslizarse.
- **El cruce se busca por bisección**, y eso supone que la relación es monótona en el tamaño
  del delta. No se supone en silencio: la prueba comprueba que el punto encontrado cruza y que
  el anterior no.
- **`Politica::Umbral` se queda.** El 5 % sigue disponible con su procedencia, para quien
  quiera la cifra del sector en vez de la suya. Lo que ya no es, es el valor por defecto de
  nada.
