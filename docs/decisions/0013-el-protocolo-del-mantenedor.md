# 0013 · El protocolo del mantenedor

**Estado:** aceptado · **Fecha:** 2026-09-02 · **Decide:** que el mantenimiento incremental es
una **sesión** por stdin, y que el dictamen de coste **no se obedece a sí mismo**

---

## El problema

El [ADR 0012](0012-el-estado-es-parcial-y-vive-en-el-cliente.md) decidió **dónde** vive lo que
el mantenimiento incremental tiene que recordar, y dejó escrito qué faltaba: *«sostener los
bytes es de un programa delegado, con la frontera de siempre: por stdin, y lo que devuelve no
se cree»*. Faltaba el programa.

Y con él, tres preguntas que ningún ADR anterior contesta, porque hasta ahora **ningún
delegado tenía memoria**. `ore-fetch`, `ore-sign`, `ore-log` y los `ore-read-<tipo>` son todos
funciones: entra una petición, sale una respuesta, el proceso muere. Un mantenedor no puede
serlo — el estado de una junta es su integrador, y un proceso por delta tendría que recibirlo
y devolverlo entero en cada paso.

## Decisión

> **El mantenimiento es una sesión: una línea la abre, una línea es una orden, una línea es su
> respuesta. La sesión ES el estado, y cerrarla es tirarlo.**

El transporte no cambia —stdin/stdout, NDJSON, verbo explícito en `argv`— porque es el mismo
que el [ADR 0008](0008-el-protocolo-del-driver.md) fijó y por las mismas razones. Lo que cambia
es que el proceso **dura**, y esa es toda la diferencia.

```text
ore-maintain mantener
```

```json
{"plan":{…},"clave":["pais"],"bundle":"sha256:…","capacidad":128,
 "capacidades":{"lago":{"predicatePushdown":["eq"],"fullScan":"forbidden"}}}
{"op":"leer","clave":[{"s":"ES"}]}
{"op":"rellenar","clave":[{"s":"ES"}],"filas":[…],"marca":7,"bundle":"sha256:…"}
{"op":"delta","marca":8,"hojas":[{"datasource":"lago","objeto":"pedidos","filas":[…]}]}
```

Lo que viaja es **el plan y filas**. Nunca un dialecto, nunca SQL, nunca una credencial — las
tres ausencias son las del driver, heredadas.

## Las tres decisiones de forma

### 1 · Una vista que no se mantiene no abre sesión

La primera línea falla si el Refresh Analyzer dice `FULL`, y el error trae **todos** los
motivos. Es la pieza de M6 puesta donde vale: quien intenta mantener un `PROMEDIO` se entera
antes de mandar una fila, no a la tercera hora de refrescos y por la factura.

### 2 · Un fallo devuelve una *upquery*; con capacidades, devuelve **la petición**

Leer una clave ausente devuelve el plan que la repone. Y si la sesión declaró qué sabe hacer
cada origen, devuelve además lo que el Pushdown Planner le pediría a cada hoja — que es lo que
convierte el *miss* en algo que se le puede pasar a un `ore-read-<tipo>` tal cual.

Con una consecuencia que se ve funcionando y no se había dicho nunca: **un fallo que el origen
no sabe contestar se rechaza sin abrir nada.** Si repoblar exige recorrer entera una tabla
cuya fuente prohíbe el recorrido, eso se sabe aquí, antes de la conexión. Declarar en vez de
intentar, un piso más abajo.

**La URL no viaja.** Un mantenedor que conociera la credencial sería un sitio más donde vive
un secreto, y `05-ejecutor` §6.2 separa a propósito la identidad que refresca de la que
responde. La petición sale sin ella y la completa quien la tiene.

### 3 · El dictamen viaja, y **no se obedece solo**

Cada paso trae su dictamen del Cost Model con las medidas que entraron. Y el paso **se aplica
igual**, aunque el dictamen diga `RECOMPUTAR`.

No es dejadez. Es que **este proceso no puede recomputar**: recomputar es releer la fuente, y
la fuente es del cliente. Y si el mantenedor se saltara el paso «porque sale caro», sus
integradores se quedarían sin ver ese Δ y **todos los pasos siguientes darían mal** — una junta
cuyo integrador se perdió un alta ya no empareja.

> Un dictamen que se obedeciera a sí mismo a mitad de un circuito sería una optimización que
> corrompe el estado.

Así que el mantenedor mantiene, y quien recibe el dictamen decide si recomputa — y recomputar
aquí es **abrir otra sesión**, que es exactamente lo que un recómputo es: un estado nuevo.

## Lo que se acepta a cambio

- **La sesión es el estado, y no sobrevive al proceso.** Persistir entre sesiones es de quien
  invoca: se vuelve a abrir y se rellena. Es coherente con el estado parcial —un almacén que
  arranca vacío solo produce fallos, y un fallo es un plan— pero significa que **arrancar
  cuesta una upquery por clave caliente**.
- **La medida de la base es parcial.** `filas_base` es lo que el almacén sostiene, no lo que la
  fuente tiene, salvo que quien llama lo diga con `base`. Subestimar la base infla la razón
  delta/base y empuja el dictamen hacia recomputar: se pierde velocidad, nunca frescura. Es la
  dirección correcta en la que equivocarse, y se dice en vez de disimularse.
- **Una línea rota no cierra la sesión.** Se contesta con un error y se sigue: cerrarla tiraría
  el estado de todas las anteriores por una coma mal puesta. El precio es que un cliente que
  ignore los errores puede creer que va bien.
- **Un proceso vivo por vista.** Es el coste de tener memoria, y es el que Noria, Materialize y
  Feldera pagan también. Lo que no se paga es el almacenamiento: los bytes siguen sin ser
  nuestros.

## Lo que esto cierra, y lo que no

Cierra el primero de los dos pendientes de [`docs/view-engine.md`](../view-engine.md) §6: **el
Delta Compiler y el Partial State Store dejan de ser solo semántica y contrato.** Hay un
proceso donde los dos corren, y la afirmación que lo sostiene está probada a través del
protocolo: *lo que sale de mantener es lo que saldría de recomputar*.

No cierra **las medidas**. El dictamen sale en cada paso con números reales de entrada, pero
los coeficientes con los que se decide siguen siendo `Politica::sin_medir` —unos— o la cifra
de Snowflake, que es suya. Calibrarlos es lo siguiente.
