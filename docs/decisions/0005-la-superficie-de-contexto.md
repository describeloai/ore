# 0005 · La superficie de contexto

**Estado:** aceptado · **Fecha:** 2026-08-30 · **Decide:** `ore dev` sirve el contrato, y el servidor no vuelve a filtrar

---

## Contexto

`ore dev` y `ore serve` existen desde el primer README como dos celdas de una tabla —
*«Runtime · producción · `dev` `serve` + Helm»*— y **la frontera entre ellos no está escrita
en ninguna parte**. Lo mismo con MCP: se nombra nueve veces en los dos repositorios y todas
son la misma clase de mención — una celda en una lista de superficies, la definición del
conducto `contextSurface`, el criterio de la fase 3. **Ni una línea sobre qué es una
herramienta, qué devuelve o cómo funciona una sesión.**

Y hay una razón de fondo: `00-overview` §1 pone **el protocolo de servicio fuera de alcance**
de OOS, a propósito. Con v1alpha5 casi todo se derivaba de reglas ya escritas. Aquí la
especificación se aparta y hay que decidir.

## La frontera entre `dev` y `serve`

La tentación es partirlos por **nivel** —`dev` es L1, `serve` es L2— y es un error: obligaría
a renombrar el comando el día que `dev` hable con una fuente de desarrollo. La distinción
buena no es qué nivel alcanzan, sino **qué custodian**:

| | `ore dev` | `ore serve` |
|---|---|---|
| a quién sirve | **un consumidor**, el que lo lanzó | **muchos**, desconocidos |
| transporte | **stdio** | HTTP |
| ciclo de vida | muere con su cliente | sobrevive a sus clientes |
| qué custodia | **nada** | credenciales vivas de todas las fuentes |
| qué debe | nada | autenticación, TLS, auditoría, observabilidad |

> **`ore dev` es un proceso hijo; `ore serve` es un servicio.**

De ahí sale la consecuencia que decide el orden de construcción: **`dev` no abre un puerto**,
así que no le debe autenticación a nadie y **se puede publicar antes de que exista**.
`serve` no: en el instante en que acepta una conexión de red contrae las cuatro obligaciones
de la última fila, y publicarlo sin ellas sería vender lo que no se tiene.

Los dos crecerán hacia L2 cuando haya drivers. La frontera aguanta ese crecimiento porque no
está trazada sobre el nivel.

## Decisión

**`ore dev` sirve el contrato por MCP sobre stdio, y no toca un dato.**

### 1 · El servidor no vuelve a filtrar

Es la decisión que más cosas simplifica, y no es una comodidad: es `DESIGN` §3.8 —*la
política la aplica el motor en un punto único, nunca el consumidor*— aplicada a nosotros
mismos.

> **El contrato ya pasó por el conducto. El servidor sirve lo que el contrato contiene, y
> nunca más de eso.**

`ore export --format graphql` ya ejecutó los cuatro pasos de `01-emision-graphql` §4:
descartar por madurez, descartar por clasificación, aplicar máscaras y podar lo que quedó
vacío. Un segundo filtro en el servidor sería un segundo punto de aplicación — y dos puntos
de aplicación se aplican en el más débil de los dos.

**Consecuencia operativa:** este servidor **no tiene lógica de política**. No lee retículos,
no compara niveles y no consulta `ConduitPolicy`. Lee un SDL y lo sirve. Que sea aburrido es
la propiedad.

Y de ahí sale la regla que gobierna cada herramienta que se añada en el futuro:

> **Toda respuesta se deriva del SDL emitido, no del paquete.** Una herramienta que leyera el
> paquete podría contar lo que el conducto quitó — y lo haría sin que nadie lo notase,
> porque el fallo no tiene aspecto de fallo.

### 2 · Recursos para documentos, herramientas para verbos

MCP distingue **recursos** —cosas que se leen— de **herramientas** —cosas que se invocan—. El
contrato es un documento, así que es un recurso:

| Recurso | Qué es |
|---|---|
| `oos://schema.graphql` | el contrato: el SDL emitido |
| `oos://bundle.json` | el digest del bundle y la identidad del paquete |

Y dos herramientas, porque hay clientes que solo hablan herramientas y porque una lista de
tipos es un verbo, no un documento:

| Herramienta | Qué hace |
|---|---|
| `ontology_schema` | devuelve el SDL |
| `ontology_describe` | **extrae del SDL** el bloque de un tipo |

`ontology_describe` lee del texto emitido y **no del paquete**, que es §1 hecho código: no
puede filtrar porque no tiene de dónde.

### 3 · Cada respuesta lleva su digest

`DESIGN` §3.4 promete que *«¿qué sabía el agente el martes a las 14:32?» se responde con un
commit y una marca de agua*. Aquí eso deja de ser una frase: el digest del bundle viaja en
`oos://bundle.json` y en el `_meta` de cada resultado de herramienta.

No se fija la sesión a un digest todavía — `dev` sirve un árbol de ficheros que su dueño está
editando, y congelarlo sería pelearse con el caso de uso. Fijar la sesión es de `serve`, donde
el artefacto sí es inmutable.

### 4 · La versión del protocolo se devuelve, no se impone

MCP versiona por fecha y el cliente propone la suya en `initialize`. Esta superficie es de
solo lectura y no usa una sola capacidad que haya cambiado entre revisiones, así que se
**devuelve la que el cliente pidió**. Imponer una versión propia rechazaría clientes por una
diferencia que aquí no existe.

## Lo que se acepta a cambio

- **Un agente no puede preguntar por datos.** Es L1, y es lo que dice ser. El criterio de la
  fase 3 —*«el PII vuelve enmascarado»*— es **L2** y necesita drivers; lo que se construye
  aquí es la mitad que ese criterio daba por supuesta y nunca nombró.
- **`ontology_describe` devuelve texto SDL, no una estructura.** Devolver campos tipados
  exigiría un analizador de GraphQL, y el emisor ya produce la forma canónica. El día que
  haga falta, el analizador se escribe; hoy sería inventar trabajo.
- **Sin suscripciones ni `prompts`.** Ninguna de las dos tiene todavía nada que servir.
- **Un solo paquete por proceso.** `dev` toma una ruta. Servir un supergrafo de varios
  paquetes es de `serve`, y arrastra la federación entera.
