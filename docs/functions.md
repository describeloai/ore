# El motor de funciones

> **Estado:** en diseño · **Fecha:** 2026-09-01
>
> La mitad de arriba —la forma y por qué— es permanente. La de abajo —los peldaños— es
> desechable y se borra cuando el último se pone en verde.

---

## 1. Qué falta, medido

`v1alpha2/02-function` define el documento y ORE lo valida. **Nada lo ejecuta.**

```
declarar          ✅  kind: Function — runtime, entrypoint, effects, endorsements
leer              ✅  ore-exec plan  — autorizadas, podadas, filtros, ensamblaje
ejecutar          ❌  ore-exec solo sabe `validar`, `plan` e `index`
aplicar           ❌
```

Y falta con ella la propiedad que la especificación promete y que hoy no tiene consumidor:

> *Un agente no recibe credenciales, **recibe una superficie**. Fuera de las funciones
> declaradas no hay forma de escribir nada — no porque se prohíba, sino porque no hay canal.*

---

## 2. El problema, y por qué no es el que resuelve la competencia

Las tres capas que casi todo el mundo mezcla:

| | Qué resuelve | Estado en la industria |
|---|---|---|
| **aislamiento** | que el código no se salga | resuelto — Firecracker, el sandbox de wasm |
| **elasticidad** | arrancar y escalar a cero | resuelto — wasm 1–5 ms, microVM ~125 ms |
| **autoridad** | **qué puede tocar** | el rol de IAM, que habla de infraestructura |
| **consistencia** | **sobre dato que no posees** | **nadie** |

Lambda te da un sandbox y un rol: *esta función puede leer ese bucket*. No hay forma de decir
*«puede escribir `Pedido.estado` y nada más»*, ni *«solo si un humano lo aprobó»*.

Foundry es el que más se acerca y su diseño es bueno: un *edit store* por ejecución, los edits
colapsados, una transacción atómica, y `@Edits([task])` declarando qué tipos se editan. Pero
esa declaración **vive en el código**: para saber qué puede causar la plataforma hay que leer
TypeScript. Y su consistencia se apoya en **poseer el dato**.

Y las funciones del almacén —*remote functions* de BigQuery, *external functions* de
Snowflake— no son cómputo junto al dato: el almacén agrupa filas y las **manda por HTTP** a
Cloud Run o a API Gateway. **El dato sale**, y con él se acaba el gobierno: dentro de aquel
contenedor ya nadie sabe que aquellas columnas eran `gdpr.sensitivity: high`.

---

## 3. La consistencia primero, y de dónde sale

Está desarrollado en el [ADR 0006](decisions/0006-el-artefacto-de-topologia.md). En una línea:

> **La consistencia no viene de poseer el dato. Viene de poseer el significado y la
> correspondencia, y de decir la verdad sobre la frescura de lo demás.**

Los dos planos que una función necesita —**qué significa** y **qué se corresponde con qué**—
son artefactos nuestros, inmutables por versión y ya construidos. El tercero, la carga útil, no
es nuestro **a propósito**, y su frescura se declara con la marca de agua en vez de prometerse.

**Consecuencia práctica:** este motor no está bloqueado por almacenamiento. La caché de carga
útil hace falta para que una función vaya **rápida**, no para que sea **correcta**.

---

## 4. La forma: la función **no aplica, propone**

Un `Plan` entra, una `Propuesta` sale. La función es **pura**: recibe valores, no una conexión.
No lee durante la ejecución y no escribe durante la ejecución.

```text
Plan ──► función ──► Propuesta ──► (verificar) ──► (aplicar)
```

Y una `Propuesta` no lleva solo qué escribir: lleva **bajo qué se decidió**.

### 4.1 · Las cuatro identidades

| | Contesta |
|---|---|
| **digest del bundle** | bajo qué significado se decidió — y si sigue vigente |
| **versión de topología** | con qué correspondencia se resolvieron las claves |
| **marcas de agua** | hasta cuándo era cierto el dato que se leyó |
| **el `Plan`** | qué se leyó, qué se podó y **por qué** |

Con las cuatro dentro, una `Propuesta` se contesta sola: *¿se puede reproducir?*, *¿se computó
sobre dato rancio?*, *¿el significado sigue vigente?*. Sin ellas, las tres son sospechas.

### 4.2 · Lo que eso regala

| Propiedad | Por qué se sostiene |
|---|---|
| **determinismo** | mismas identidades + mismos valores → misma `Propuesta`, y su digest lo prueba. Es **replay para un auditor** |
| **simulacro gratis** | la `Propuesta` **es** el simulacro. No hay dos caminos que puedan divergir |
| **idempotencia** | una `Propuesta` tiene identidad, así que *«¿esto ya se aplicó?»* pasa a ser contestable |
| **alcance atómico** | declara qué fuentes toca, así que `transaction.scope: single-datasource` se comprueba **antes**, no es una sorpresa en ejecución |
| **auditoría completa** | el par `(Plan, Propuesta)` es la historia entera: qué se leyó, qué se podó y por qué, y qué se iba a escribir |

### 4.3 · Y lo que cuesta, dicho claro

**No hay lecturas dinámicas.** La función no puede decidir a mitad de vuelo que necesita otra
tabla. Si necesita más, lo declara y el `Plan` crece; lo iterativo son varias invocaciones —que
de paso las hace reanudables.

Es menos expresivo que Foundry, donde una función navega enlaces sobre la marcha. Y es **el
mismo cambio que hace todo lo demás aquí**: Cedar no tiene bucles, el compilador no tiene
reloj. La expresividad acotada es lo que hace analizable una cosa. Se paga en comodidad y se
cobra en que **el efecto se puede mirar antes de que ocurra**.

---

## 5. Por qué `runtime: wasm` no era una moda

**WASI 0.2 es capability-based**: un componente arranca **sin autoridad ambiente** y solo puede
hacer aquello para lo que el host le pasa un *handle*. Un componente sin importación de red no
puede abrir un socket, **y la garantía se sostiene aunque el proceso anfitrión sí pueda**.

Compárese con lo que `02-function` §1 ya decía, escrito antes de elegir runtime:

> *…no porque se prohíba, sino porque **no hay canal**.*

Esa frase es la definición de seguridad por capacidades. Así que `effects:` no tiene que ser
una política que alguien comprueba: puede ser **la lista de handles que el host entrega**. La
declaración y la ejecución pasan a ser la misma cosa, y eso no se puede hacer sobre Lambda.

Con la forma de §4 hay además una simplificación que conviene ver: como la función **no lee ni
escribe**, el sandbox que necesita es aún más pequeño que el de WASI — no le hace falta ninguna
capacidad. Recibe valores y devuelve valores.

---

## 6. La frontera: `ore` declara y verifica; ejecutar se delega

La de siempre, y por la razón de siempre. Un runtime de wasm es una dependencia grande con FFI,
y `dependencias.rs` veta exactamente eso.

| | Dónde |
|---|---|
| computar el `Plan`, cotejar la `Propuesta` contra `effects:`, correr el flujo sobre ella, comprobar endosos | **dentro** |
| ejecutar el módulo | `ore-invoke`, un programa del usuario en el PATH |

La petición va por **stdin**, como en `ore-fetch`, `ore-sign` y `ore-log`. Y **lo que devuelve
no se cree**: cada edit propuesto se coteja contra los efectos declarados, y lo que quede fuera
se rechaza. Es lo mismo que hace `ore pack` con una firma que le devuelve `ore-sign`.

---

## 7. Los peldaños

> **Desde aquí es desechable.**

### F0 · La `Propuesta` como artefacto

El contrato de invocación y el cotejo, **sin ejecutar nada**: un runner de mentira devuelve
edits y `ore` comprueba que uno fuera de `effects:` se rechaza.

**Listo cuando:** un efecto fuera de la superficie declarada no se aplica, y la `Propuesta`
lleva las cuatro identidades y digiere igual dos veces.

Va primera porque **es lo único que nadie más tiene** y no necesita runtime.

### F1 · El flujo sobre la propuesta

`flow` y `governance` corriendo sobre los edits propuestos, no sobre el árbol.

**Listo cuando:** una función que proponga escribir en un destino por debajo de la
clasificación de lo que leyó **no compila su propuesta**, con el mismo código que hoy lo dice
de una materialización.

### F2 · Los endosos

Comprobar las atestaciones **antes** de invocar. Es verificación de firmas: reutiliza P2 entero.

**Listo cuando:** una función cuyo endoso no verifica no llega a ejecutarse.

### F3 · `ore-invoke`

wasm + WASI 0.2, una capacidad por efecto declarado.

**Listo cuando:** un módulo que intente abrir un socket falla **por no tener canal**, no por
una comprobación.

### F4 · Aplicar

Atómico, idempotente por el digest de la propuesta, y de alcance comprobado antes.

**Listo cuando:** aplicar dos veces la misma propuesta produce el mismo estado, y una que toque
dos fuentes se rechaza antes de escribir en la primera.

---

## 8. Lo que **no** entra, y no por falta de tiempo

**Lecturas dinámicas.** §4.3. Es la decisión que hace analizable todo lo demás.

**Un runtime dentro de `ore`.** No se reabre: es la propiedad que `dependencias.rs` comprueba.

**Transacciones distribuidas.** `02-function` §2 ya retiró `transaction.scope` como campo
porque solo admitía un valor: *mejor un error que un campo*. Cruzar dos fuentes de forma atómica
es un problema que no vamos a resolver mejor que nadie; lo que sí se puede es **decir que no se
hace, y comprobarlo antes de escribir**.

**La caché de carga útil.** Es del ADR 0006 y va por su cuenta: hace falta para la latencia, no
para la corrección.
