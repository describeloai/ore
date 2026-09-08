# 0021 · Una persona pertenece a varias organizaciones

**Estado:** aceptado · **Fecha:** 2026-09-08 · **Decide:** que la identidad de una persona **no
lleva dentro** a qué cliente sirve, que la pertenencia es una relación y no un atributo, y que
**la organización viaja en el camino** de cada petición — nunca en el token

---

## El problema

`iam.pertenencia` es muchas-a-muchas desde que se escribió, y **nadie lo decidió**. Salió de
copiar la forma de una tabla de unión. La plataforma decide lo contrario, y lo decide en el
motor: [`014-sujeto.sql`](file) pone `organizacion text not null` en el sujeto y la mete dentro
del IRI —

```
'https://id.paladio.io/' || organizacion || '/sujeto/' || tipo || '/' || id_opaco
```

⇒ allí el IRI de una persona **contiene** su organización. No es una relación: es parte de su
identidad, y `provisionOrganization` se niega en voz alta al caso cruzado — *«ese principal ya
existe en otra organización. ⛔ No se le puede hacer administrador de ésta sin que su identidad
cruce clientes»*.

Dos formas incompatibles conviviendo sin que ninguna estuviera argumentada.

---

## Lo que se miró antes de decidir

[`pruebas-de-fuego/medida-las-tres-decisiones.py`](../../pruebas-de-fuego/medida-las-tres-decisiones.py).

- **El censo, en el clúster:** 2 personas, 2 organizaciones, **una cada una**. Nadie está en dos
  todavía ⇒ el momento barato, el mismo argumento con el que se fijó el `iss` el primer día.
- **El IdP ya lo había contestado.** El mapeador del scope `organization`, medido contra el realm
  vivo, es `oidc-organization-membership-mapper` con `"multivalued": "true"` ⇒ el claim es un
  **array**. Keycloak asume que una persona puede estar en varias, y por eso `76 ANEXO` sacó la
  organización del claim: con un array, `organizacionDeClaims` no sabía cuál elegir.
- **El coste de la alternativa, para nosotros:** con una organización por persona, **nosotros no
  podríamos administrar la plataforma con nuestra propia cuenta**. Operar N clientes exigiría N
  identidades, y el operador es justo quien más las cruza.

---

## La decisión

> ### Una persona es UNA, y pertenece a las organizaciones que la han admitido.

Tres cosas se siguen, y ninguna es opcional:

**① La identidad no lleva la organización dentro.** `iam.persona` es `(emisor, sub)` y nada más.
Su `id` es opaco y no se puede leer para saber de quién es — al revés que su IRI, que lo decía.

**② La organización viaja en el CAMINO, no en el token.**
`/organizaciones/{org}/…`, y cada verbo comprueba la potestad **en esa organización**.

⛔ Y el claim `organization` **no es la autoridad**, ni siquiera ahora que existe. La pertenencia
sale de `iam.pertenencia`, que se escribe redimiendo una invitación. Es la idea de `021-la-
invitacion.sql` y su consecuencia es medible: si la pertenencia saliera del IdP, habría que meter
a cada persona en una Organization de Keycloak, y eso exige `manage-realm` — el realm entero,
**medido en 403** el 2026-08-28. No tener esa credencial es una propiedad, no una carencia.

⇒ El claim, como mucho, es una pista para pintar un selector. Nunca la fuente.

**③ Toda respuesta se acota por organización, y el error NO distingue.** «No perteneces» y «no
llegas» dan el mismo mensaje, que ya es la regla de `potestad::exige`. Lo nuevo es que ahora
**también** hay que aplicarla al orden: comprobar la potestad **antes** de contestar nada sobre
un objeto, incluido si existe.

---

## Lo que costó descubrir esto, y es el primer caso

`revocar` leía la concesión, distinguía «no hay tal concesión» de «ya estaba revocada», y sólo
**después** exigía ser administrador de la organización de esa concesión. Con una organización por
persona era inofensivo. Con varias es **una sonda entre inquilinos**: un administrador de A puede
averiguar, id a id, qué concesiones existen en B y cuáles siguen vivas.

Su propia frase lo dice en `exige` y no la habíamos extendido al orden: *decir «no eres
administrador de esa organización» le confirma a quien pregunta que esa organización existe*.

⇒ Arreglado en el mismo commit que esta decisión: se exige primero, y «no existe» y «no es tuya»
son el mismo error.

---

## Lo que esto NO decide

- **Cómo elige la consola.** El camino lleva la organización; quién la pone en el camino —un
  selector, un segmento de URL, una preferencia— es de la consola y va aparte.
- **`organization:<alias>` de Keycloak**, que ataría una sesión a una organización. No se adopta:
  exigiría que cada organización exista además como Organization del realm y que cada persona sea
  miembro allí — exactamente el acoplamiento que `76 ANEXO` quitó.
- **Si un agente puede cruzar organizaciones.** `iam.concesion.sujeto` es texto para que quepa un
  agente (RFC 8693), y lo que un agente puede alcanzar cuando actúa por alguien que está en dos
  no está contestado aquí.
