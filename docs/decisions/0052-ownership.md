# 0052 · Ownership

**Estado:** **aceptado · en vivo** (2026-10-02) · **Decide:** de quién es cada activo de la
plataforma: **todo lo que nace con `owner` es de quien lo crea**, `user:<handle>` de esa persona, y
el handle lo da el plano de control. Gramática: `owner` en OOS (en `Package` desde v1alpha1; en
`Entity`, `Function`, `ObjectTable` y `Model` desde
[v1alpha21 `01-el-dueno`](../../vendor/oos/spec/v1alpha21/01-el-dueno.md)). Se apoya en
[`0047`](0047-ore-access-control.md) (el puente de las celdas al plano de control) y en
[`0048`](0048-ore-idp.md) (la persona y su nombre de usuario).

## Qué es

`owner` dice **quién responde** de un activo: a quién se pregunta por él, quién decide cambiarlo o
retirarlo. Un handle: `user:<h>` o `team:<h>` (`OOS2009` si no tiene esa forma). **No concede ni
niega acceso**: quién puede leer, escribir o invocar lo deciden el gobierno del flujo y ORE Access
Control. Son dos preguntas, y cada una tiene su sitio.

La CLI no sabe quién la ejecuta: lo pregunta (la decisión `dueno`), y sin respuesta escribe
`cambiame`. **La plataforma sí lo sabe**: cada petición llega con la identidad de una persona, y el
plano de control la conoce. Ownership es esa regla, una y en un sitio.

## La regla

> **Todo lo que nace con `owner` es de quien lo crea: `user:<handle>` de esa persona.**

- **El handle lo da el plano de control, una vez.** `ore-iam` se lo asigna a cada persona la primera
  vez que ve su token, desde el nombre de usuario que eligió al registrarse (`preferred_username`;
  si no hay, su correo; regla `ore_core::pertenencia::handle_de`, empate con `-2`, `-3`…), y lo
  guarda en `iam.persona.handle` (migración 048: único, con la forma de `OOS2009`). **No cambia**:
  es lo que queda escrito en el árbol.
- **Las celdas lo preguntan por el puente**: `POST /access/v1/quien` (0047), con caché en
  `ore-acceso` (`Acceso::quien`). En `ore-serve`, la regla es una función:
  `Servidor::dueno_de_quien_crea` (`acceso.rs`). Sin plano de control —un banco de pruebas— se
  deriva del sujeto con la misma regla.
- **Desde un puesto, la persona**, no el agente: lo que una celda crea es de quien abrió el puesto
  (el `sub`; el `act` va en el commit). **Un agente no es dueño de nada** (403).
- **No se hereda del contenedor.** Una colección que Ana crea en la base de Bea es de Ana; un schema
  nuevo no toma el `owner` de su base; una vista SQL no toma el de su schema.
- **Editar no es transferir.** Reescribir un documento sin decir `owner` conserva el que tenía
  (`PUT /documentos`, `ore datasets`, la función que se regenera de su `@function`). Un `owner`
  explícito se respeta: **transferir es escribirlo**.
- **Sin respuesta no se inventa.** Si el plano de control no contesta, 503: lo que se iba a crear no
  nace con un dueño que no es de nadie.

## Dónde se aplica

| se crea | por |
|---|---|
| una base (vacía o inducida) y su schema | el alta de la consola; `ore discover --owner` |
| el paquete de un origen | el Job de catálogo, que recibe el dueño como `DUENO` |
| el sitio de un proyecto | `/proyectos` |
| `View`, `MediaCollection`, `Dataset`, `TrainedModel`; y `Entity`, `Function`, `ObjectTable` y `Model` | `PUT /documentos` |
| una vista desde el puesto | `create view` |
| un dataset | el catálogo REST `/v1` y `/datasets` |
| un modelo | `POST /modelos` |
| una función | al guardar su código o al sembrar un repositorio |

El SDK y la CLI no inventan un dueño: sin `dueno`, el documento va sin `owner` y lo pone el
servidor. `owner` es **obligatorio** en `Package`, `Dataset`, `MediaCollection` y `TrainedModel`, y
opcional en `Entity`, `Function`, `ObjectTable` y `Model`. `Action`, `Concept`, `Interface` y
`Table` no lo llevan: una tabla es un hecho del origen, no algo que alguien crea en la plataforma.

## Por qué así

Antes el alta contestaba `dueno` con `team:<organización>` («el árbol es de la organización»), el
SQL del puesto heredaba el del schema o la base y el SDK escribía `team:<base>`. Ninguno decía de
quién era nada ni a quién preguntar: en una organización, todo era de todos. **Quién pulsó ya iba en
el commit; ahora quién responde también es alguien**, y lo dice un solo sitio que conoce a las
personas. La decisión `dueno` sigue existiendo para lo que la CLI induce sin plataforma, y
transferir una base es contestarla otra vez.

## Aceptación

`los-verbos.sh` (el handle de ore-iam y el puente), `la-copia-se-decide.sh` (el origen con
`owner: user:ana`), `el-lago.sh` (el dataset escrito es de quien lo escribe), `el-puesto.sh` (lo que
crea un puesto es de quien lo abrió, no de su base) y `los-modelos.sh` (el modelo de quien lo da de
alta).
