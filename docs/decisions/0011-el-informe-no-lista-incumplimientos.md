# 0011 · El informe no lista incumplimientos

**Estado:** aceptado · **Fecha:** 2026-08-31 · **Decide:** `ore report` es un registro de atribución, y detrás hay una sola computación

---

## El problema

*«¿Estamos gobernados?»* es la pregunta que un equipo de cumplimiento hace todos los
trimestres, y la herramienta que la contesta es un informe de incumplimientos. Aquí esa
pregunta **no tiene sentido**: una propiedad cuya clasificación exige gobierno y no lo tiene
no compila (`OOS8001`). Un informe de incumplimientos sobre un bundle compilado sería una
tabla vacía por construcción — y una tabla que siempre está vacía no se lee: se ignora.

## Decisión

> **La pregunta no es *¿está gobernado?* Es *¿quién responde, y por qué vía?***

Cada fila es **propiedad × naturaleza exigida × qué la descarga**, con el `owner` de la regla
al lado. Y solo aparece lo que `requiresGovernance` exige: en el ejemplo de referencia hay 40
propiedades clasificadas y **once** exigen algo. Listar las otras veintinueve sería casi tres
cuartas partes de las filas diciendo *«nada que gobernar»*, que es la forma más eficaz de que
nadie mire las once que importan.

El pie de la tabla dice lo único que un auditor necesita saber sobre el resto:

> *once propiedades exigen gobierno, y las once lo tienen: si alguna no lo tuviera, esto no
> habría compilado.*

### Una sola computación, y por qué importa

`cobertura_atribuida()` es la fuente. `cobertura_efectiva()` se deriva de ella y el chequeo de
`OOS8001` la lee. Antes eran **dos computaciones de lo mismo**, con un comentario en el código
advirtiendo de que dos definiciones serían dos semánticas — y ya lo eran: una ignoraba las
propiedades nombradas directamente por una regla, así que el informe y el error podían
discrepar sobre la misma propiedad.

Un informe que discrepa del compilador es peor que no tener informe: **el compilador tiene
razón por construcción, y el informe es lo que lee un humano.**

### Contra qué se comparó

| | Cuándo evalúa | Sobre qué |
|---|---|---|
| GitLab · *compliance status report* | cada doce horas | lo desplegado; un control externo pasa de `pending` a `fail` a las seis |
| Databricks · Unity Catalog | continuo | *securables* con dueño, en un catálogo vivo |
| `ore report` | **al compilar** | un artefacto que no existe si el gobierno falta |

La diferencia no es de frecuencia: allí el informe **mide un sistema en marcha**, y aquí mide
un artefacto. Por eso puede permitirse no tener estado, ni servicio, ni reloj.

## Lo que se acepta a cambio

**No dice nada del runtime.** Un `ore report` en verde no promete que nadie leyó un dato sin
permiso; promete que **si alguien lo leyó, hay una regla y un dueño que responden de esa
lectura**. Lo que pasa en el momento de la petición lo gobierna el ejecutor, y su registro es
la respuesta con sus tres ejes.

**Y se construyó sin pruebas.** Se ejecutó a mano contra el ejemplo, se miró la salida y se
dio por bueno — exactamente la crítica que este ciclo le hizo a todo lo demás, aplicada a su
autor. Las cinco pruebas de [`informe.rs`](../../crates/ore-cli/tests/informe.rs) llegaron
después, y una de ellas fija el pie de la tabla: la única frase del binario que afirma algo
sobre lo que **no** se lista.
