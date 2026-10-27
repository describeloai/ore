# 0002 · Sin validador de JSON Schema

**Estado:** aceptado · **Fecha:** 2026-08-29 · **Decide:** comprobaciones de forma nativas

---

## Contexto

OOS publica siete esquemas JSON en `schemas/v1alpha1/`. Son **artefactos normativos**: la
mitad sintáctica de L0. La reacción natural es enlazar un validador 2020-12 y dejar que
`OOS1004` sea su salida.

## Por qué no

**Peso.** Los validadores disponibles pesan entre 73 y 192 crates, y arrastran FFI de
plataforma. `ore-core` no enlaza hoy contra el sistema operativo, y esa propiedad —además
de hacer trivial la compilación cruzada— es coherente con lo que el compilador afirma
ser: un paso puro.

**Y sobre todo, el mensaje.** Un validador genérico produce esto:

```
/spec/primaryKey: minItems
```

La tesis del proyecto es que **el error es el producto**. Ese es exactamente el error que
criticamos en las herramientas que queremos sustituir. Lo que hace falta es:

```
error[OOS2010]: hr.AuditLog declara `nature: entity` y no tiene `primaryKey`
  → entities/AuditLog.yaml:6:3
  ayuda: un log de auditoría suele ser `nature: event` con `timeKey`
```

Un validador no puede escribir la segunda línea porque no sabe qué es un log de auditoría.
La regla de precedencia de `99-errors` §2.1 lo dice como norma: **un código semántico gana
sobre `OOS1004` aunque el esquema JSON también detecte el fallo.** Delegar en el validador
sería rendirse en el único sitio donde se nota.

## Decisión

`ore-core::document` comprueba las siete formas de manera nativa y tipada, con posición y
con ayuda accionable. Los esquemas JSON siguen siendo normativos y siguen siendo para
quien los necesita: editores, linters en otros lenguajes, acciones de CI.

## Cómo se evita la deriva

Los esquemas y estas comprobaciones podrían separarse en silencio. No pueden: **la suite
de conformidad es el árbitro de ambos.** Un caso que el esquema rechaza y ORE acepta —o al
revés— es un fallo rojo, no una discrepancia de interpretación.
