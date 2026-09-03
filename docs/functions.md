# El motor de funciones

> **Estado:** en diseño · **Fecha:** 2026-09-03 · **Reescrito contra el sustrato.**
>
> La mitad de arriba —la forma y por qué— es permanente. La de abajo —los peldaños— es
> desechable y se borra cuando el último se pone en verde.
>
> La versión anterior es de 2026-09-01, **anterior a que `Table` y `View` fueran el sustrato**.
> Su forma sobrevivió entera; su camino apuntaba a un mundo que ya no existe.
> [§9](#9--qué-cambió-al-reescribirlo) dice exactamente qué se movió y qué no.

---

## 1. La frase

> **Una función no escribe. Propone.** Y lo que propone se escribe **a través de una vista**, o
> no se escribe.

La primera mitad ya estaba y no se toca: es lo que hace que el efecto se pueda mirar antes de que
ocurra. La segunda es nueva y es todo lo que este documento añade.

---

## 2. Qué falta, medido

| | |
|---|---|
| peldaños construidos | **cero**. No existe `Propuesta`, ni `ore-invoke`, ni ningún aplicador |
| lo que sí hay | `ore-core/src/effect.rs`, que comprueba **forma y etiquetas** de `effects` |
| y un hueco que nadie había mirado | **`effects` está huérfano** |

### 2.1 · El hueco

Un efecto declara hoy esto:

```yaml
effects:
  - writes: supply_chain.PurchaseOrder.status   # una PROPIEDAD de una ENTIDAD
    datasourceRef: postgres_erp                 # y una FUENTE, directamente
```

**Va de la propiedad a la fuente sin nada en medio.** Ese «nada en medio» era el `Binding`: el
documento que decía qué columna es `status` en `postgres_erp`. Y el binding se retiró en
v1alpha8.

Comprobado en el código: `effect.rs` resuelve `writes` a una **propiedad de entidad** y comprueba
sus etiquetas de integridad. **Ninguna línea del árbol traduce esa propiedad a una columna.**
`datasourceRef` se lee, se transporta, y no resuelve a ningún destino físico.

Es el mismo hueco que `OOS2022` cerró para la lectura, un piso más abajo y en el otro sentido:
*«con bindings esto era legal porque otro podía cubrirlo; en v1alpha8 no hay otro»*. Nadie lo ha
notado porque no hay ejecutor — y por eso conviene notarlo ahora y no cuando lo haya.

---

## 3. La forma: la función no aplica, propone

Un `Plan` entra, una `Propuesta` sale. La función es **pura**: recibe valores, no una conexión. No
lee durante la ejecución y no escribe durante la ejecución.

```text
Plan ──► función ──► Propuesta ──► (verificar) ──► (aplicar, por la vista)
```

Y una `Propuesta` no lleva solo qué escribir: lleva **bajo qué se decidió**.

### 3.1 · Las cinco identidades

| | Contesta |
|---|---|
| **digest del bundle** | bajo qué significado se decidió — y si sigue vigente |
| **versión de topología** | con qué correspondencia se resolvieron las claves |
| **marcas de agua** | hasta cuándo era cierto el dato que se leyó |
| **el `Plan`** | qué se leyó, qué se podó y **por qué** |
| **el digest del plan de la vista** | **por dónde se va a escribir**, y bajo qué recorte |

La quinta es nueva y sale del sustrato. *Bajo qué vista se decidió escribir* es tan auditable como
*bajo qué bundle*: una vista recorta filas, y una propuesta que escribe a través de ella solo
puede tocar las que la vista deja ver. Sin esa identidad, *«¿podía esta función tocar esta
fila?»* no tiene respuesta local.

Con las cinco dentro, una `Propuesta` se contesta sola: *¿se puede reproducir?*, *¿se computó
sobre dato rancio?*, *¿el significado sigue vigente?*, *¿por dónde iba a entrar?*

### 3.2 · Lo que eso regala

| Propiedad | Por qué se sostiene |
|---|---|
| **determinismo** | mismas identidades + mismos valores → misma `Propuesta`, y su digest lo prueba. Es **replay para un auditor** |
| **simulacro gratis** | la `Propuesta` **es** el simulacro. No hay dos caminos que puedan divergir |
| **idempotencia** | una `Propuesta` tiene identidad, así que *«¿esto ya se aplicó?»* pasa a ser contestable |
| **alcance atómico** | declara qué fuentes toca, así que escribir en dos se rechaza **antes**, no es una sorpresa en ejecución |
| **auditoría completa** | el par `(Plan, Propuesta)` es la historia entera |

### 3.3 · Y lo que cuesta, dicho claro

**No hay lecturas dinámicas.** La función no puede decidir a mitad de vuelo que necesita otra
tabla. Si necesita más, lo declara y el `Plan` crece; lo iterativo son varias invocaciones —que de
paso las hace reanudables.

Es menos expresivo que Foundry, donde una función navega enlaces sobre la marcha. Y es **el mismo
cambio que hace todo lo demás aquí**: Cedar no tiene bucles, el compilador no tiene reloj. La
expresividad acotada es lo que hace analizable una cosa.

---

## 4. Y ahora la mitad que faltaba: por dónde entra

[`sustrato.md`](sustrato.md) §3.4 lo dejó escrito para la lectura:

> La capa ontológica se sienta **sobre la vista**. La vista se apoya en la tabla o en su copia, y
> cuál de las dos es una decisión de abajo que la ontología no ve.

**La escritura es la misma frase leída al revés**, y no hay que inventar nada:

> **El destino de un efecto se deriva, no se declara.** Entidad → `backedBy` → vista → raíz →
> tabla. Exactamente el camino que ya recorre la lectura.

Por eso **`datasourceRef` desaparece del efecto**. Declararlo sería un segundo sitio que puede
discrepar del primero, y este árbol ya sabe cómo termina eso: la tabla existe porque había N
sitios describiendo lo físico y pasó a haber uno.

`writes` **se queda tal cual**. Nombrar la propiedad es correcto: es el idioma de la ontología, y
la ontología no debe saber en qué columna cae.

### 4.1 · Escribir es `Q⁻¹`, y la mitad difícil ya está resuelta

[`v1alpha8/00-scope`](../vendor/oos/spec/v1alpha8/00-scope.md) §6.1 lo decidió al migrar, con una
corroboración que no venía de aquí:

> *«Lo que la vista sabe hacer es exactamente el fragmento invertible, y eso no se buscó: se
> descubrió al migrar el árbol.»*

| operación de la vista | ¿invertible? |
|---|---|
| renombrar | sí — es una biyección |
| recortar por partición | sí — la fila escrita cumple el predicado o se cae de la vista |
| proyectar | **parcialmente** — faltan columnas, así que la escritura es *parcial*, no ambigua |
| juntar, agregar, deduplicar, limitar | **no**, y por eso no están en el vocabulario |

Y **la pregunta ya es computable con lo que hay**. `linaje` da, por cada campo de salida, de qué
columna raíz sale y **por qué arista**:

- `Directo(Identidad)` — invertible;
- `Directo(Transformacion)`, `Directo(Agregacion)`, cualquier `Indirecto` — **no**.

Así que *«esta función escribe a través de una vista que no se puede invertir»* se rechaza **al
compilar**, con la misma máquina que ya rechaza una copia que fuga. No hace falta motor nuevo:
hace falta llamar al que hay.

### 4.2 · Y la tercera cara de la tabla

Leer y cambiar están declarados —`reads` es la cara `I`, `changes` la cara `D`—. Escribir es la
tercera, y le toca el mismo trato: **el objeto declara qué acepta, y el planificador lo respeta
sin abrir una conexión.**

```yaml
writes:
  mode: <qué acepta>        # el vocabulario, en §7.1, es una decisión abierta
  key: [ ... ]              # con qué identifica la fila que actualiza
```

La simetría con `changes` no es estética. `changes` dice **qué sale** del objeto; `writes` dice
**qué entra**. Y las dos preguntas tienen el mismo dueño —el objeto— y la misma consecuencia: una
tabla que no declara `writes` **no se escribe**, igual que una que declara `reads: none` no se
consulta. La ausencia es una negativa, que es la doctrina de esta casa desde v1alpha1.

### 4.3 · Y no se escribe en la copia

Conviene decirlo porque los dos caminos existen ahora y se parecen:

| | qué escribe | quién |
|---|---|---|
| **materializar** | **la copia**, en nuestro almacén | `ore materialize` · [ADR 0015](decisions/0015-el-protocolo-del-almacen.md) |
| **aplicar un efecto** | **el origen**, a través de la vista | esto |

Una función que escribiera en la copia estaría escribiendo en una respuesta cacheada, y el origen
la contradiría en el refresco siguiente. **La copia es derivada; el origen es la verdad.**

---

## 5. La frontera: `ore` declara y verifica; ejecutar se delega

La de siempre, y por la razón de siempre. Un runtime de wasm es una dependencia grande con FFI, y
`dependencias.rs` veta exactamente eso.

| | Dónde |
|---|---|
| computar el `Plan`, cotejar la `Propuesta` contra `effects:`, correr el flujo sobre ella, comprobar endosos, **decidir si la vista se invierte** | **dentro** |
| ejecutar el módulo | `ore-invoke`, delegado |
| **escribir en el origen** | `ore-write-<tipo>`, delegado |

La última fila es nueva, y es la **cuarta delegación** del árbol. Cae junto a las otras tres por
la misma razón —`ore` no abre sockets— y hereda su protocolo: lo que viaja no son llamadas al
origen, es **la propuesta ya bajada a columnas**.

Y **lo que devuelve un delegado no se cree**: cada edit propuesto se coteja contra los efectos
declarados, y lo que quede fuera se rechaza. Es lo mismo que hace `ore pack` con una firma.

---

## 6. Los peldaños

> **Desde aquí es desechable.**

**Listo es [F6](#f6--la-definición-de-listo)**, no que los cinco anteriores estén escritos.

| | qué | cuesta |
|---|---|---|
| **F0** | la cara `W`, y el efecto pierde su fuente | gramática · v1alpha9 |
| **F1** | la `Propuesta` como artefacto | nada de protocolo |
| **F2** | el flujo sobre la propuesta | nada |
| **F3** | los endosos | nada |
| **F4** | `ore-invoke` | delegado nuevo |
| **F5** | aplicar **por la vista** | delegado nuevo |
| **F6** | **la definición de listo** | es la prueba |

### F0 · La cara `W`, y el efecto pierde su fuente

**Qué.** `Table.writes`, y `datasourceRef` fuera de `effects`. Es lo único de la lista que toca la
gramática, y va primero porque **sin él los demás no tienen a dónde escribir**.

> **Es `M1` de [`sustrato.md`](sustrato.md), no un peldaño aparte.** Allí se ve desde abajo —la
> tabla gana su tercera cara— y aquí desde arriba —un efecto necesita un destino—, y es la misma
> línea de código. Lo que se añade aquí es lo que aquel documento no podía saber: que el efecto
> **pierde su `datasourceRef`**.

Y con él llega la regla que `linaje` ya puede contestar: una función cuyo efecto atraviese una
vista **no invertible** no compila, con código propio, al lado de `OOS2020`, `OOS2021` y
`OOS2023` — que son las otras que miran las caras del objeto.

**Listo cuando.** Un efecto sobre una vista que agrega **no compila** y el mensaje nombra la
arista que lo impide; uno sobre una vista que solo renombra y recorta **sí**; y `ore view` dice,
por vista, **si se puede escribir a través de ella**.

> ### ✅ Hecho, y en dos peldaños porque al medirlo eran dos
>
> **`F0a` · la cara `W`.** `Table.writes` es un **conjunto** —`insert`, `update`, `delete`— y no
> un modo, y eso no se decidió aquí: `information_schema.views` de SQL expone tres columnas
> separadas y no un modo. No hay `upsert` —es la suma de dos— ni `writes.key` —la fila se
> identifica con `changes.key`, que ya existía—. `OOS2024` la exige con `update` o `delete`;
> `OOS7012` rechaza el efecto cuyo objeto no acepta que lo actualicen; `datasourceRef` sale del
> efecto y `OOS7008` pasa a derivar la fuente. `ore view` contesta por vista.
>
> **`F0b` · la guarda de invertibilidad.** `OOS7013`, sobre la cadena entera.
>
> ### Y una parte del «listo cuando» que **no se pudo cumplir**, dicha entera
>
> *«Un efecto sobre una vista que agrega no compila»* **no se puede probar con un documento**,
> porque no se puede declarar una vista que agregue: el vocabulario de `View` en v1alpha8 es
> exactamente el fragmento invertible. Lo comprobado en su lugar:
>
> - la guarda se ejerce **construyendo el paquete a mano**, con el constructor que la gramática
>   todavía no tiene — igual que el IR de `ore-view` prueba `Agrupa` sin que ningún documento lo
>   produzca;
> - y un **censo** ata la clasificación al vocabulario: añadir una clave a `View` sin decir si se
>   invierte no compila la suite. Falsificado añadiendo `groupBy` y viéndolo saltar.
>
> La medida que lo destapó: la costura solo construye cuatro nodos del IR —`Referencia`, `Lee`,
> `Filtra`, `Proyecta`—, cada campo es siempre `Expr::campo`, y `Une`/`Agrupa`/`Limita` no
> aparecen en ningún camino que salga de un documento.

### F1 · La `Propuesta` como artefacto

**Qué.** El contrato de invocación y el cotejo, **sin ejecutar nada**: un runner de mentira
devuelve edits y `ore` comprueba que uno fuera de `effects:` se rechaza. Y la `Propuesta` lleva
las **cinco** identidades.

Va aquí porque **es lo único que nadie más tiene** y no necesita runtime.

**Listo cuando.** Un efecto fuera de la superficie declarada no se aplica; la propuesta digiere
igual dos veces; y **cambiar la vista cambia el digest**, que es lo que hace auditable por dónde
se iba a entrar.

> ### ✅ Hecho. `ore verify`, y las tres cláusulas comprobadas
>
> `ore_core::propuesta` tiene la forma y el cotejo; `ore verify` es el verbo. **No ejecuta la
> función, no abre el origen y no escribe** — que sea contestable sin runtime es lo que hace que
> el simulacro salga gratis.
>
> | cláusula | cómo se comprueba |
> |---|---|
> | fuera de la superficie no se aplica | un runner devuelve un edit sobre `pais`, la función declara `estado`, y el rechazo **dice qué sí declaraba** |
> | digiere igual dos veces | dos invocaciones dan los mismos bytes y el mismo nombre, visto desde fuera del proceso |
> | cambiar la vista cambia el digest | se genera contra un paquete y se verifica contra el mismo con la vista estrechada |
>
> Y una cuarta que no estaba pedida y salió del sustrato: un edit tiene que **nombrar la fila con
> la clave de su entidad**, en propiedades y no en columnas. Un runner descuidado que devuelva
> `employee_id` en vez de `employeeId` se rechaza, y eso es exactamente el error que separar los
> dos idiomas existe para hacer visible.
>
> ### La forma que la medida impuso
>
> **La `Propuesta` es un documento, no un `struct` enlazado**, y no por gusto: tres de sus cinco
> identidades viven en crates que `ore-cli` **no puede** tener en su cierre —`ore-exec` trae
> Cedar, `ore-store-r2` trae Parquet y TLS— y `tests/dependencias.rs` lo hace cumplir leyendo el
> `Cargo.lock`. Así que topología, marcas de agua y el `Plan` **llegan por el protocolo**, en JSON
> canónico, igual que llega la cabecera de un sobre.
>
> `ore verify` las imprime como **«sin verificar aquí»** en vez de callarlas. Decir *no lo he
> mirado* es una respuesta; omitirlo no lo es.
>
> El reparto que eso obliga, y que es el mismo que ya usa `ore view`: **el cotejo de la superficie
> vive en el núcleo** —es gramática— y **la comparación de la vista vive en la costura** —es
> álgebra, y el núcleo no ve el motor—.
>
> `Plan::digest()` se añadió en `ore-exec`, que no lo tenía: el par `(Plan, Propuesta)` es la
> historia entera **porque los dos se nombran por su contenido**, no porque uno lleve al otro
> dentro.

### F2 · El flujo sobre la propuesta

**Qué.** `flow` y `governance` corriendo sobre los edits propuestos, no sobre el árbol.

**Listo cuando.** Una función que proponga escribir en un destino por debajo de la clasificación
de lo que leyó **no compila su propuesta**, con el mismo código que hoy lo dice de una copia.

### F3 · Los endosos

**Qué.** Comprobar las atestaciones **antes** de invocar. Es verificación de firmas: reutiliza P2.

**Listo cuando.** Una función cuyo endoso no verifica **no llega a ejecutarse**.

### F4 · `ore-invoke`

**Qué.** wasm + WASI 0.2, una capacidad por efecto declarado.

**Listo cuando.** Un módulo que intente abrir un socket falla **por no tener canal**, no por una
comprobación.

### F5 · Aplicar por la vista

**Qué.** El cuarto delegado. `ore` baja la propuesta a columnas siguiendo la vista, y
`ore-write-<tipo>` la escribe. Atómico, idempotente por el digest de la propuesta, y de alcance
comprobado antes.

**Y una escritura parcial es lo normal, no la excepción.** Una vista con tres campos escribe tres
columnas y deja el resto — que es exactamente lo que `proyectar` invierte *parcialmente*.

**Listo cuando.** Aplicar dos veces la misma propuesta produce el mismo estado; una que toque dos
fuentes se rechaza **antes** de escribir en la primera; y un `ore-write-<tipo>` que no sepa
honrar algo **se niega en vez de aproximarlo**, que es la regla que `rango_servible` ya fijó para
el otro lado.

### F6 · La definición de listo

**Qué.** Nada nuevo: una prueba de fuego con **números afirmados**, al modo de
`pruebas-de-fuego/refresco.sh`. Nace roja y su salida es la lista de trabajo.

Los actos, sobre un origen de verdad:

| | qué pasa | qué se afirma |
|---|---|---|
| 1 | una función propone un cambio | la propuesta lleva las **cinco** identidades, y digiere igual dos veces |
| 2 | se aplica | **una** fila cambia en el origen, y es la que la vista dejaba ver |
| 3 | se aplica **otra vez** | **cero** escrituras · la idempotencia por digest |
| 4 | se lee la vista | el cambio está, sin recompilar nada |

Y las negativas, que valen igual:

| | se provoca | tiene que pasar |
|---|---|---|
| a | un efecto por una vista que agrega | **no compila** · F0 |
| b | un edit fuera de `effects:` | se rechaza, y se dice cuál |
| c | una propuesta bajo un bundle viejo | se rechaza nombrando los dos digests |
| d | dos fuentes en un efecto | se rechaza **antes de escribir en la primera** |
| e | un endoso que no verifica | no se llega a invocar |

**Listo cuando.** Los cuatro actos dan sus números, las cinco negativas fallan por su motivo, y
**el origen queda como estaba** salvo la fila que tenía que cambiar.

---

## 7. Lo que falta decidir · con datos, no con opinión

Tres, y **no se cierran aquí**. Se cierran como se cerraron las cinco del
[ADR 0016](decisions/0016-el-testigo-y-el-rango.md): mirando lo que tienen escrito quienes ya lo
resolvieron. Ahí seis sistemas coincidieron, y una de las lecturas **cambió la propuesta en vez
de confirmarla**.

### 7.1 · El vocabulario de `writes.mode`

`changes.mode` habla el de Flink —`append`, `retract`, `upsert`— porque era el que ya existía. La
cara `W` necesita el suyo, y probablemente ya existe también.

**Dónde mirar:** las *vistas auto-actualizables* de SQL —PostgreSQL las tiene con reglas
publicadas y expone `information_schema.views.is_updatable`, que es **literalmente esta pregunta,
estandarizada**—; el `MERGE` de SQL:2003; las *Actions* de Foundry; y el `apply` sobre containers
de Cognite.

**Sospecha, para poder equivocarse por escrito:** que sea `insert`, `update`, `upsert`, `delete`
como conjunto, y no un modo único — porque una tabla puede aceptar altas y no borrados, y eso hoy
no se puede decir.

> **✅ Decidida, y la sospecha acertó a medias.** Conjunto, sí: `information_schema.views` expone
> `is_insertable_into`, `is_updatable` e `is_trigger_deletable` **por separado**. Pero **sin
> `upsert`**: es `insert` más `update`, y el conjunto ya lo dice sin una cuarta palabra.
> `changes.mode` sí lo tiene, porque allí no es una suma sino otra codificación.

### 7.2 · Qué exige una escritura parcial

Proyectar es invertible **parcialmente**. La pregunta es si eso basta o si la vista **debe cubrir
la clave** para que la fila escrita sea identificable.

**Sospecha:** la clave es obligatoria, y `changes.key` vuelve a servir — es el mismo campo que
hizo posible fundir un incremento.

> **✅ Decidida, y la medida fue en contra de la sospecha en su mitad importante.** `changes.key`
> vuelve a servir, sí, y no hay una segunda. Pero **la clave se le exige a la TABLA, no a la
> vista**, y el número es la razón: de 20 vistas v1alpha8 sobre una tabla resuelta, **17 se apoyan
> en una tabla sin clave**. Exigírsela a la vista habría dejado el 85 % del corpus sin poder
> escribirse, y no por culpa de la vista. Las 3 que sí la tienen la cubren enteras, así que donde
> aplica no cuesta nada.
>
> Que una vista **no exponga** la clave no es un error: la tabla cumple, la vista es legal, el
> paquete compila. Simplemente por esa vista no se entra, y `ore view` lo dice — que es la
> respuesta que ninguna otra salida daba.

**Dónde mirar:** las condiciones exactas que PostgreSQL exige a una vista auto-actualizable, y qué
hace Cognite cuando una vista mapea propiedades de **varios** containers.

### 7.3 · Dónde vive la propuesta aplicada

Una propuesta tiene digest, así que *«¿ya se aplicó?»* es contestable — **si alguien lo recuerda**.
¿Es un artefacto en el almacén, como una copia? ¿Es un registro en el origen? ¿Es del cliente?

**Dónde mirar:** dónde guarda Foundry el historial de *Actions*, y cómo Debezium recuerda su
*offset* — que es el mismo problema con otro nombre, y su respuesta fue **fuera del origen**.

---

## 8. Lo que **no** entra, y no por falta de tiempo

**Lecturas dinámicas.** §3.3. Es la decisión que hace analizable todo lo demás.

**Un runtime dentro de `ore`.** No se reabre: es la propiedad que `dependencias.rs` comprueba.

**Transacciones distribuidas.** `02-function` §2 ya retiró `transaction.scope` porque solo admitía
un valor: *mejor un error que un campo*. Cruzar dos fuentes de forma atómica no lo vamos a
resolver mejor que nadie; lo que sí se puede es **decir que no se hace, y comprobarlo antes de
escribir**.

**Escribir en la copia.** §4.3. La copia es derivada; el origen es la verdad.

**Que la entidad deje de repetir.** Es M2 de [`sustrato.md`](sustrato.md), y va después: esto se
construye sobre lo despejado, no a la vez.

---

## 9. Qué cambió al reescribirlo

Para que se vea qué sobrevivió y qué no, y no haya que compararlo a mano.

| | |
|---|---|
| **la forma** —*no aplica, propone*— | **intacta**. Era buena y es anterior al sustrato por casualidad, no por error |
| las cuatro identidades | **cinco**: entra el plan de la vista |
| `effects.datasourceRef` | **fuera**. El destino se deriva |
| F1, F2, F3 · propuesta, flujo, endosos | **intactos**. No sabían que existía un sustrato y no les hacía falta |
| F4 · `ore-invoke` | **intacto**. Un runtime de wasm no tiene nada que ver con esto |
| F5 · aplicar | **reescrito**. Decía *«atómico e idempotente»* y no decía **por dónde**; ahora dice por la vista, si `Q` se invierte |
| y uno delante | **F0**, la cara `W`. Sin él los demás no tienen destino |

**Lo que esto enseña, y vale más que el documento:** la mitad permanente aguantó un cambio de
paradigma completo debajo, y la desechable no. Es exactamente la línea que separa las dos, y la
prueba de que estaba bien puesta.
