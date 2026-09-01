# El motor

> **Estado:** en construcción · **Fecha:** 2026-09-01
>
> La mitad de arriba —qué es el motor y por qué— es permanente. La de abajo —los peldaños—
> es desechable y se borra cuando el último se pone en verde.

---

## 1. La cuña

Todo lo que sigue cuelga de una frase, y conviene tenerla delante porque decide cada
elección de las de abajo:

> **Las superficies ontológicas se construyen federando el dato, y almacenando el contexto
> y la topología.**

Nadie más lo hace así. Foundry consigue consistencia **poseyendo el dato**: lo materializa
en su *object backend* y entonces transacciona. Es una solución buena y es la razón de que
haya que meterlo todo dentro. La industria regulada no puede: su dato está en BigQuery, en
SAP, en un mainframe, y sacarlo es el proyecto de tres años que nadie aprueba.

Lo que decidió el [ADR 0006](decisions/0006-el-artefacto-de-topologia.md) es la salida:

> **La consistencia no viene de poseer el dato. Viene de poseer el significado y la
> correspondencia, y de decir la verdad sobre la frescura de lo demás.**

Y de ahí sale exactamente qué es el motor. **No es un almacén y no es un runtime: es lo que
custodia las dos superficies que sí poseemos, y lo que sabe decir cuándo lo que no poseemos
ha dejado de servir.**

---

## 2. Los tres planos, y qué le falta a cada uno

Medido contra el árbol, no contra la intención:

| Plano | Qué es | Identidad | Firma | Log | Distribución | Refresco |
|---|---|---|---|---|---|---|
| **Contexto** | el bundle compilado | ✅ `digest::bundle` | ✅ | ✅ | ✅ registro | por commit |
| **Topología** | claves de join y aristas | ✅ **`Topologia::version`** | ❌ | ❌ | ❌ | ✅ `index refresh` |
| **Carga útil** | tabla en el lago del cliente | — | — | — | — | ✅ **`ore cache check`** |

La asimetría entre la primera fila y las otras dos es el trabajo. **El plano de contexto está
terminado** —tiene identidad determinista, se firma, entra en un log de transparencia
verificable y se publica en un registro—. Los otros dos tenían formato y no tenían nada de eso.

### 2.1 · Por qué la topología no tenía identidad, y ahora sí

El artefacto `ORETOPO1` lleva dentro tres afirmaciones y **solo dos tenían nombre**:

| | Contesta | Cambia cuando |
|---|---|---|
| digest del bundle | qué significaba | se recompila el modelo |
| **cuerpo** → `version` | **qué se corresponde con qué** | aparece o desaparece una arista |
| marca de agua | hasta cuándo era cierto | **cada refresco** |

Faltaba la de en medio, que es justamente la que el ADR 0006 promete —*«misma versión →
mismo conjunto de claves en una travesía»*— y la que [`functions.md`](functions.md) §4.1 pide
que una propuesta cite. Ese renglón no tenía a qué apuntar.

Y tiene que ser **del cuerpo**, no del fichero: si incluyera la marca, refrescar sin que
ninguna arista cambiara diría *«otra correspondencia»* cuando la travesía da exactamente el
mismo conjunto de claves, y un auditor comparando dos propuestas concluiría que cambió algo
que no cambió.

### 2.2 · Por qué la caché no es un subsistema, y aun así hay que construir algo

El ADR 0006 ya decidió lo difícil: la carga útil es **una tabla Iceberg en el lago del
cliente**, con su catálogo, sus herramientas y su mecanismo nativo de frescura. No es nuestra,
y no debe serlo — inventar un almacén ahí sería contradecir la frase de §1.

Lo que la decisión dejó sin dueño es una pregunta que la tabla no puede contestar:

> **¿Esta caché puede servir esta consulta?**

Una tabla con filas dentro tiene el mismo aspecto sirva o no sirva. Lo que hace falta es
metadato —pequeño, nuestro, versionado— que diga **bajo qué se escribieron esas filas**. Eso
es el manifiesto de caché, y es la mitad del tercer plano que sí nos toca.

**Los bytes son del cliente. La afirmación sobre bajo qué se escribieron es nuestra.**

### 2.3 · Cuatro motivos para no servir, y no se arreglan igual

| Veredicto | Qué pasó | Se arregla |
|---|---|---|
| `ReglaDistinta` | se materializó bajo **otro bundle** | **reconstruyendo** |
| `CorrespondenciaDistinta` | otra versión de topología | reconstruyendo |
| `Incompleta` | no tiene esa propiedad | ampliando la materialización |
| `Rancia` | la marca no llega al SLA | **refrescando** |

La primera fila es la que justifica el módulo entero, y el ADR 0006 la nombra al final sin que
nada la comprobara:

> *Refrescar responde a que el dato cambió; reconstruir, a que la REGLA cambió. Un efecto
> computado bajo una regla nueva sobre datos enmascarados con la vieja es la clase de fallo que
> no tiene aspecto de fallo.*

**Medido sobre el repositorio del cliente**, con un `total` que alguien clasifica como crítico
un martes:

```text
1 · antes                sirve · rubix_lake.cache.ventas_pedidos                        → 0
2 · alguien pone         labels: { gdpr.sensitivity: critical }  en ventas.Pedidos.total
    ore validate .       ok · forbid-critical-egress — ventas.Clientes.nif, ventas.Pedidos.total
3 · la misma cache       regla distinta · se materializó bajo `sha256:c19d4aec…`         → 65
                         remedio: reconstruir
```

**Ni una fila de la tabla cambió entre 1 y 3.** El `total` sigue ahí en claro, la marca de agua
sigue fresca y el SLA se cumple. Lo único que cambió es qué significa exportarlo — y por eso
*«refresca»* es el consejo equivocado: refrescar reescribe las filas bajo la misma pregunta, y
la pregunta es la que cambió.

---

## 3. La frontera, otra vez

La de siempre. Escribir la tabla exige un catálogo, credenciales y un escritor de Parquet;
decidir si sirve es aritmética sobre un fichero que ya está en el árbol.

| | Dónde |
|---|---|
| calcular la versión de un índice, decidir si una caché sirve, verificar una firma | **dentro** |
| leer las aristas de una fuente, escribir la tabla, firmar | delegado |

Por eso `ore cache check` vive en el compilador y no en el ejecutor: no abre la tabla, no lee
el reloj —el instante llega con la consulta, como en todo lo demás— y no necesita credenciales.
Y por eso `--bundle` **no es una bandera**: el digest sale del árbol. Dejar que quien pregunta
lo teclee sería dejarle contestar por su cuenta la única pregunta que la caché no puede
contestar sola.

---

## 4. Los peldaños

> **Desde aquí es desechable.**

### E0 · La topología tiene identidad ✅

`Topologia::version` — el digest de las aristas, separado del bundle y de la marca.
`ore-exec index build` la imprime; `ore-exec index id` la lee de un artefacto ya escrito.

**Listo cuando:** refrescar sin aristas nuevas no cambia la versión, una arista de más sí, y la
versión sobrevive al ida y vuelta por el fichero. ✅

### E1 · El manifiesto de caché ✅

`ore_core::cache` y `ore cache check`. El veredicto es puro y el digest sale del árbol.

**Listo cuando:** una caché escrita bajo otro bundle no sirve, **y el remedio que se propone no
es refrescar**. ✅ — y medido sobre el repositorio de un cliente, no sobre una maqueta.

### E2 · La topología se firma y se distribuye

Lo que el ADR 0006 promete —*«se construye una vez, se firma, se distribuye y se mapea»*— y de
lo que solo existe la construcción. Es P2 y P3 otra vez, sobre otro enunciado: la versión, la
marca y el bundle contra el que se construyó.

**Listo cuando:** un índice con la firma quitada no se carga, y su entrada en el log prueba
inclusión.

Y arrastra la restricción de §4 del ADR, que no se puede olvidar al distribuirlo: **el
artefacto de topología contiene datos.** El bundle viaja en la imagen; este no.

### E3 · `ore-cache`, el programa delegado

Escribir y leer la tabla. Fuera, por stdin, como `ore-fetch`, `ore-sign` y `ore-log`. Lo que
devuelve **no se cree**: lo que escriba se anota en el manifiesto con el bundle y la versión de
topología del momento, que es lo que E1 comprueba después.

**Listo cuando:** una materialización deja una entrada en el manifiesto que `ore cache check`
acepta, y la misma entrada deja de aceptarse en cuanto el paquete se recompila con otra
clasificación.

### E4 · El plan consulta la caché

Hoy `plan` va siempre a la fuente. La fase ③ tiene que preguntar primero al manifiesto, y la
respuesta tiene que **decir de dónde salió cada lectura** — una respuesta que no distingue
caché de origen no se puede auditar.

**Listo cuando:** un plan cuya caché sirve no abre ninguna conexión, y la respuesta lo dice.

---

## 5. Lo que **no** entra, y no por falta de tiempo

**Una base de datos.** Está decidido en el ADR 0006 con cinco razones y una medida ajena. No se
reabre.

**Escribir Parquet dentro de `ore`.** Es la dependencia nativa que `dependencias.rs` veta, y el
motivo por el que E3 es un programa aparte.

**Aislamiento transaccional sobre el dato del cliente.** No lo tenemos y decir lo contrario
sería la clase de promesa que este proyecto no hace. Lo que sí se puede es **acotar la
mentira**: el digest dice qué significaba y la marca hasta cuándo era cierto.

**Las abstracciones.** Funciones, acciones, agentes. Van encima de esto y no antes: una función
que cite una versión de topología que no existe está citando una promesa.
