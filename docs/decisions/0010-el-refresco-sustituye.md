# 0010 · El refresco sustituye, no suma

**Estado:** aceptado · **Fecha:** 2026-08-31 · **Decide:** una fila es el conjunto de aristas de su clave, y la marca DEBE avanzar

---

## El problema

Reconstruir el artefacto de topología lee la tabla entera cada vez.
[`05-ejecutor`](../../vendor/oos/spec/v1alpha1/05-ejecutor.md) §6.1 define la marca de agua
—`watermark`—, y con ella el refresco lee **solo lo que la marca deja fuera**: un `gt` sobre
la columna que el binding mapea. Lo que queda por decidir es cómo se fusiona ese delta con el
artefacto anterior.

## Decisión

> **Una fila *es* el conjunto de aristas de su clave. Si vuelve en el delta, sustituye.**

Y el artefacto nuevo lleva una marca que **DEBE** ser posterior a la del anterior. Un
refresco que no avanza no es un refresco: `refresh` falla en vez de escribir un artefacto que
parece fresco y no lo es.

## Lo encontró ejecutarlo, no razonarlo

La primera versión **sumaba**. Sobre una fila nueva, sumar y sustituir dan exactamente el
mismo resultado, así que la diferencia solo aparece cuando una fila **cambia**: `emp-42`
cambia de jefe, y con la suma quedan las dos aristas — la cadena de mando tiene dos ramas y
un cambio se parece a una ampliación. Una autorización que dependa de *«es tu subordinado»*
diría que sí por el jefe viejo.

Ninguna prueba escrita mirando el código lo habría visto. Salió al escribir el escenario de
[`fuentes-reales.sh`](../../pruebas-de-fuego/fuentes-reales.sh) contra un PostgreSQL de
verdad: crear, refrescar, y **volver a preguntar**.

> Una prueba que no corre tiene exactamente el mismo aspecto que una que pasa. Y una que solo
> corre sobre el caso fácil, también.

## Lo que se acepta a cambio

**Un refresco incremental no ve una fila borrada.** No es un defecto de la implementación: una
fila que ya no está **no cambió después de nada**, así que ninguna marca de agua la trae. Se
avisa al ejecutar, no se disimula; quien necesite exactitud sobre borrados reconstruye con
`index build`, que es lineal sobre la fuente y sigue existiendo por esto.

**Solo `poll`.** De las tres estrategias que `05-ejecutor` §6 nombra, `cdc` exige una fuente
que emita cambios —otra frontera de confianza, y otra decisión— y `table_version` es de la
caché de carga útil, no de aquí.

**La bandera se llama `--anterior`.** Se llamó `--desde` durante una tarde, hasta que se vio
que `--desde` ya significaba *una clave de partida* en `index traverse`. Una bandera con dos
significados dentro del mismo binario es un error esperando a que alguien tenga prisa.
