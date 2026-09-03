# Handoff · la copia, registrada una vez

> **Este documento es desechable.** Se borra el día que exista **un solo registro de copias** en
> el árbol: cuando la topología, la carga útil y lo que venga después sean instancias de la misma
> cosa, y no queden dos caminos de refresco ni dos testigos. Un plan que sobrevive a su ejecución
> deja de ser un plan.
>
> Fecha: 2026-09-02 · Sale de terminar T4 de [`handoff-tablas.md`](handoff-tablas.md) y de medir
> el motor por dentro. La dirección de fondo —qué reposa sobre qué— está en
> [`sustrato.md`](sustrato.md), que **no** es desechable; esto es cómo se llega.

---

## 1. Qué es una materialización

`handoff-tablas` dijo del puntero físico: *se registra una vez, con sus dos caras*. Esto dice lo
mismo de la copia.

> **Una materialización es un plan que ya está calculado en algún sitio. Se registra una vez, y
> dice tres cosas: qué contesta, dónde vive, y hasta cuándo fue cierta.**

| cara | pregunta | quién la usa |
|---|---|---|
| **el plan** | ¿qué consulta contesta esta copia? | el View Matcher, para decidir si sirve |
| **el destino** | ¿dónde viven las filas, y en qué forma? | quien la puebla y quien la lee |
| **el testigo** | ¿hasta cuándo fue cierta? | la frescura, para **degradar en vez de mentir** |

Las tres son del **objeto copiado**, no de quien lo consulta — exactamente como `reads` y
`changes` son del objeto y no de la vista. Es la misma lección, un piso más arriba.

### 1.1 · Ya existe, y ya tiene la forma correcta

`ore-view/src/filter_tree.rs` lo define desde M5:

```rust
pub struct Materializacion {
    pub nombre: String,
    pub plan: Nodo,      // expandido: una referencia sin resolver no tiene firma
    pub tabla: Lectura,  // y sus campos DEBEN ser lo que el plan produce
}
```

Le falta el testigo, y le falta **alguien que registre alguna**. Medido:

```
filter_tree.rs     539 líneas  ┐
view_matcher.rs  1.936 líneas  ┘  2.475 líneas · CERO referencias fuera de `ore-view`
```

La costura `ore-cli/src/vista.rs` importa dieciséis símbolos del motor. **Ninguno es de M5.**

---

## 2. El problema, medido: hay tres copias y tres mecanismos

| | **topología** | **carga útil** | **superficie de contexto** |
|---|---|---|---|
| qué la define | entidad + `relations` + `via` | `materialized` de una vista | propiedades podadas por conducto |
| dónde vive | `.oretopo` — CSR firmado | *(en ninguna parte: nadie la puebla)* | la emisión, cada vez |
| quién la refresca | `index refresh`, con marca propia | nadie | nadie: se recalcula |
| testigo | **sí**, en la cabecera del fichero | no | no |
| ¿la conoce el motor? | no | no | no |

Y `plan.rs:427` dice lo que la primera fila esconde:

```rust
pub fn lecturas_de_aristas(&self) -> Vec<(String, Lectura)> {
    for e in self.paquete.entities() {          // ← la entidad
        for (nombre, rv) in rels.entries() {    // ← sus relaciones
            ... f.columnas.get(&clave[0]), f.columnas.get(&via[0])
```

**El índice de topología es una proyección de dos columnas sobre la fuente física de una
entidad**, materializada y refrescada por marca de agua. Es decir: es una vista materializada,
escrita a mano, en el paradigma anterior. `handoff-tablas` §5.1 lo dijo de pasada —*«el índice es
una vista de aristas, es derivable»*— y siguió andando. Aquí se cobra.

La tercera columna es la que todavía no está medida y se dice como proyección: la superficie de
contexto es una proyección con un conducto, y **se vuelve una vista** el día que la capa semántica
repose en el sustrato ([`sustrato.md`](sustrato.md) M2). No entra en este plan; entra en la forma
del registro, para que quepa sin reformarlo.

---

## 3. La decisión de forma: **un registro, y abierto**

> **Un registro de copias que no sabe de qué clase es cada copia.**

El registro conoce las tres caras y nada más. **No** conoce:

- **el formato del destino.** Un CSR, una tabla de un almacén, un fichero. El formato es una
  propiedad del destino, no del registro. Es la misma línea que separa `Table` de `View`: la
  tabla no sabe quién la consulta.
- **para qué sirve la copia.** «Índice de aristas» y «caché de carga útil» son etiquetas
  humanas. Para el registro las dos son un plan calculado en un sitio.
- **cómo se puebla.** Poblar es de quien tenga el cómputo — la misma frase que el motor de vistas
  lleva escrita desde M0.

Eso es lo que **abierto por diseño** significa aquí, y se mide con una pregunta: *añadir una clase
nueva de copia, ¿cuesta un mecanismo o cuesta registrar un plan?* Si cuesta un mecanismo, el
registro está mal.

### 3.1 · Y su forma ya está decidida

Qué es una copia **como cosa que se guarda** lo cierra el
[ADR 0015](decisions/0015-el-protocolo-del-almacen.md): un sobre nuestro alrededor de una carga en
Parquet, **nombrado por su digest**, inmutable, y subido por un programa delegado porque `ore` no
puede abrir un socket.

Eso le da al registro dos cosas que aquí se daban por pendientes: el **destino** tiene forma —una
clave en un almacén de objetos— y el **testigo** tiene sitio —la cabecera del sobre—. Lo que este
plan sigue teniendo que resolver es **quién las conoce**, que es otra pregunta.

### 3.2 · El testigo entra el primer día, aunque esté vacío

Ninguna pieza del motor sabe hoy fechar nada: `StateStore` lleva un `tic` lógico y su propio
comentario dice *«no un reloj»*. El `.oretopo` sí lo tiene, en la cabecera, porque hizo falta.

El testigo entra en la forma del registro **desde el primer peldaño**, aunque valga `None` en
todas las copias durante tres iteraciones. Retroajustar un testigo es de las cosas que no ocurren:
cuando hace falta, ya hay tres consumidores que asumieron que no existía.

Y ya tiene vocabulario: es `changes.witness` de la tabla — `none`, `snapshot`, `log`, `field`.
No se inventa uno nuevo.

---

## 4. Proyección sobre lo que hay: qué se toca y qué no

| pieza | hoy | después |
|---|---|---|
| `ore-view/filter_tree.rs` | `Materializacion` sin testigo, sin usuarios | gana el testigo; **nada más** |
| `ore-view/view_matcher.rs` | 1.936 líneas apagadas | **nada**: se enciende, no se cambia |
| `ore-view/refresh_analyzer.rs` | `analizar(plan)` — decide por la forma del plan | ve además la cara `D` de la raíz: hoy diría `INCREMENTAL` de algo con `changes: { mode: none }` |
| `ore-cli/src/vista.rs` — la costura | no registra ninguna copia | registra **todas** las que el paquete declara |
| `ore-exec/plan.rs` `lecturas_de_aristas` | fabrica `Lectura` a mano desde entidad y `relations` | emite **un plan**, y lo registra como una copia más |
| `ore-exec/topologia.rs` | un mecanismo | el **formato de almacenamiento** de una copia concreta |
| `ore-exec` `index build/refresh` | ruta de refresco propia | se borra cuando el Δ la cubra, y no antes |
| `ore-core` | | **nada**. El registro es del motor, no de la gramática |

La columna de la derecha de `ore-view` es casi toda «nada», y eso es lo que hace que esto entre de
una pieza: **las piezas ya están escritas y probadas**. Lo que falta no es álgebra, es conectarla.

---

## 5. El despeje va primero, y por qué

La tentación es empezar por lo visible —poblar una copia— y dejar la unificación para después.
Sería el orden equivocado, y se sabe por dónde falla: con dos registros a la vez, el segundo nunca
absorbe al primero. Acaban conviviendo, que es exactamente lo que `v1alpha7/00-scope` §5 dijo que
sería un fallo si pasaba con el binding.

Así que el primer peldaño **no añade ninguna función**. Deja la base despejada:

- una sola noción de copia, con sus tres caras;
- **todas** las copias del árbol registradas en ella, incluida la topología, aunque siga
  refrescándose por su camino viejo;
- y una prueba que diga cuántas hay y por qué camino se refresca cada una — el mismo patrón de
  inventario con motivos que `tests/migracion.rs`, que se escribió por lo mismo.

Después de eso, cada iteración mueve **un** mecanismo al registro sin que nada se apague de golpe.

---

## 6. Los peldaños

> Cada uno dice qué es y **cuándo está listo** con algo que se puede medir. Un peldaño sin
> criterio de listo es una intención.

| | qué | dónde | listo cuando |
|---|---|---|---|
| **I0** | el árbol en verde | `ore-core`, `vendor/oos` | `OOS2022` con puerta de versión; v1alpha7 vuelve a **13/13**; `acme-retail` valida |
| **I1** | **el despeje**: un registro, todas dentro | `ore-cli/vista.rs`, `ore-view/filter_tree.rs` | `ore view` lista las copias del paquete con sus tres caras; una prueba cuenta cuántas hay y por qué camino se refresca cada una |
| **I2** | el matcher decide | `ore-cli/vista.rs` | `cotejar` contesta, con compensación y sello; las restricciones desde `primaryKey` y relaciones, conectadas |
| **I3** | el testigo deja de estar vacío | `ore-view`, la costura | una copia dice hasta cuándo fue cierta; `freshness` **degrada** en vez de mentir, y se prueba con una copia vencida |
| **I4** | la topología es una copia más | `ore-exec` | `lecturas_de_aristas` emite un plan registrado; `.oretopo` es solo su formato; **borrar su ruta de refresco propia no pierde ninguna prueba** |
| **I5** | la copia existe de verdad — [ADR 0015](decisions/0015-el-protocolo-del-almacen.md) | `ore-cli`, `ore-store-r2` | una vista se puebla contra un dataset real y el artefacto queda en R2 **nombrado por su digest**; un segundo intento con el mismo testigo **no sube ni un byte** |

### I0 · el árbol en verde

**Qué.** `OOS2022` —*una propiedad de una entidad no es campo de su vista*— entró en el árbol y es
**retroactivo**: rompe `conformance/v1alpha7` de 13/13 a 12/13 y tumba `acme-retail` y tres
pruebas de `cache.rs` detrás.

La regla es correcta para v1alpha8 y falsa para las anteriores, y su propia ayuda lo dice: *«con
bindings esto era legal porque otro binding podía cubrirla; en v1alpha8 no hay otro»*. Falta la
puerta de versión, como `spec_keys_en`.

Y con la puerta puesta, `acme-retail` sigue rojo **con razón**: `hr.Employee.nationalId` no tiene
campo en ninguna vista y nunca lo tuvo. Se le da columna en `workday_worker` y campo en
`empleados` — que además vuelve a hacer cierta la traza que el ejemplo enseña.

**No hace.** No toca la regla. Solo la acota a la versión que la introdujo.

### I1 · el despeje

**Qué.** La costura construye una `Materializacion` por cada copia que el paquete declara y las
mete en un `FilterTree`. Y la topología entra también, aunque su refresco siga siendo el suyo:
**estar registrada y estar mantenida son dos cosas**, y separarlas es lo que permite mover una sin
la otra.

`Materializacion` gana el testigo, vacío.

**Práctica.** *Una operación, tres consumidores*, otra vez. El registro se construye **una** vez y
lo miran `ore view`, el planificador y —en I4— el ejecutor. Si se construyera en tres sitios,
divergiría en el que ninguna prueba ejerce.

**Listo cuando.** `ore view` imprime, por paquete, las copias que hay con sus tres caras; y una
prueba enumera **cuántas** y **por qué camino se refresca cada una**, de modo que añadir un
mecanismo nuevo la ponga roja.

**No hace.** No puebla nada, no refresca nada, no borra nada. Es un despeje: su valor es que
después de él **hay un sitio**.

#### Hecho · lo que quedó, y lo que se midió al hacerlo

El sitio es [`ore-cli/src/registro.rs`](../crates/ore-cli/src/registro.rs), y no un trozo de
`vista.rs`, por lo que decía la nota de práctica: lo van a mirar tres. `Materializacion` ganó
`Testigo { marca, valor }` con el vocabulario de `changes.witness` —`Marca::{Ninguna, Instantanea,
Registro, Campo}`— y `valor: None` en todas, que es lo que hoy son.

`ore view` cierra ahora con el registro del paquete:

```
registro · 4 copias · nadie 0 · índice de topología 4
  hr.Employee.manager
    plan      sha256:34119a2a…
    destino   oretopo·hr.Employee.manager
    testigo   sin poblar
    refresco  índice de topología — `ore index refresh`, con marca de agua propia y ajena al circuito Δ
```

**El reparto por camino sale con los ceros**, y esa es la línea que hace de guardia: un mecanismo
que deja de usarse sigue apareciendo hasta que alguien lo borre a mano, así que borrarlo es una
decisión y no un olvido. Lo demás lo sujeta un `match` exhaustivo sobre `Camino` más la cuenta
escrita en `los_caminos_de_refresco_estan_enumerados_y_cada_uno_dice_por_que`: un mecanismo nuevo
**no compila** hasta que alguien diga por qué no le valía ninguno de los que ya estaban.

**Y una medida que no se esperaba tan pronto.** `acme-retail` no declara ni una `materialized`, y
tiene **cuatro copias**. Todas de topología, y se llega a ellas **por el sustrato**: la fuente
física de una entidad es la raíz de la vista que la respalda —`backedBy`—, sin un binding de por
medio. La reconstrucción vale igual en v1alpha8, que era la duda.

**Dos cosas que salieron por el camino y conviene no volver a descubrir:**

- **`Registro::TablaNoCorresponde` no puede dispararse nunca desde aquí.** La declaración de OOS
  dice **dónde** vive la copia y no **qué columnas** tiene, así que los campos del destino se
  construyen desde el plan — la única opción honesta. La comprobación existe para quien conozca el
  destino por otra vía, o sea el ejecutor leyendo un almacén de verdad. El handoff lo anticipaba
  para I2; aparece en I1 porque construir el destino es de I1.
- **`ore-exec` no depende de `ore-view`.** Así que I4 no es «llamar a esta función»: o el ejecutor
  gana la dependencia, o el registro se muda. Es una decisión de I4 y está sin tomar.

### I2 · el matcher decide

**Qué.** `cotejar` y `sello` conectados: qué copia contesta una consulta, con qué compensación, y
qué clasificación hereda. Y las restricciones desde `primaryKey` y las relaciones, que
[`view-engine.md`](view-engine.md) §6 lleva pendientes desde que se escribió.

**Y un hallazgo que ya se puede anticipar.** La declaración de OOS dice **dónde** vive la copia y
no **qué columnas** tiene; `Materializacion` exige que la tabla produzca lo que el plan produce.
Construir esa `Lectura` desde el plan es la única opción honesta — y entonces
`Registro::TablaNoCorresponde` **no puede dispararse nunca desde este camino**. Eso dice algo
sobre para quién existe esa comprobación, y hay que escribirlo donde se vea.

#### Hecho · la copia deja de ser hoja

`ore view` dice ahora de cada vista qué copias la contestan. La línea que paga el peldaño sale del
caso `virtual-over-materialized-over-stream`:

```
ventas.iberia
  empuje    rechazado · `fullScan: forbidden` … y este plan no le baja ningún filtro
  cotejo    la contesta `ventas.pedidos` · 1 conyunto de compensación
```

`iberia` es **virtual** y no declara copia ninguna. Que la copia de `pedidos` la conteste lo
demuestra el álgebra, no la cadena: `raíz de lectura` dice algo parecido dos líneas más arriba,
pero recorriendo `from` hasta abajo. **El cotejo compara dos planes y no necesita que haya
cadena** — el día que dos vistas escritas por separado resulten ser la misma consulta, solo uno de
los dos lo verá.

Y el sello viaja: `sello heredado: dni {gdpr.sensitivity:high}`. La copia filtró por una columna
`high` que **no expone**; recalcular el linaje sobre su tabla habría perdido la etiqueta, y la
consulta reescrita habría parecido limpia. Cruza además el renombre — la etiqueta sale con el
nombre que le da quien pregunta.

**Las restricciones bajan de tres sitios**, y el reparto se imprime aunque sea cero, por lo mismo
que los caminos: con cero referenciales ninguna junta de más podrá probarse nunca, y sin la línea
ese «no la contesta» parece un fallo del cotejo en vez de una declaración que falta.

| de dónde | qué garantiza |
|---|---|
| `changes.key` de una tabla `upsert` | única — la especificación **la exige**: sin ella el mantenedor no sabe qué retracta un *tombstone* |
| `primaryKey` y `uniqueKeys` de una entidad | única, sobre la raíz de la vista que la respalda |
| una relación con `via` **y `required: true`** | referencial |

**Solo `required: true`**, y no toda relación: una referencial afirma que *toda* fila de un lado
casa con una del otro, que es justo lo que prueba que la junta no pierde. `manager` con
`required: false` son los empleados sin jefe, y una junta interna los tira. Declararla sería darle
al matcher permiso para perder filas en silencio.

**Y una medida incómoda.** `acme-retail` da **4 únicas y 0 referenciales**, aunque tiene cuatro
relaciones `required: true`. El motivo: apuntan a `Department`, `Supplier` y `Sku`, y **cinco de
sus siete entidades no declaran `backedBy`**. Sin respaldo no hay raíz física, y sin raíz la
referencial no se puede bajar a columnas. Es decir: el ejemplo está migrado a v1alpha8 **a
medias**, y eso es trabajo del repositorio de la especificación.

**Y qué se hace con esa medida, que es lo que casi sale mal.** Fijarla en una prueba habría hecho
que **terminar** la migración del ejemplo pusiera roja una afirmación sobre el sustrato — al revés
de lo que una prueba debe hacer. Así que la medida se queda aquí, escrita, y las pruebas del
sustrato se afirman sobre casos de conformidad y sobre paquetes propios y mínimos.

> **`acme-retail` solo aparece donde lo afirmado *es* el mecanismo heredado** — que la topología
> entra en el registro con su ruta aparte. Para todo lo demás arrastraría a la afirmación cosas
> que no son suyas.

La regla, dicha una vez: **un ejemplo de la superficie ontológica no es una medida del sustrato.**

### I3 · el testigo

**Qué.** El testigo deja de estar vacío. Vocabulario: el de `changes.witness`. Y el Refresh
Analyzer pasa a ver la cara `D`: hoy `analizar(plan)` decide por la forma del plan y **diría
`INCREMENTAL` de una vista sobre una tabla que declara `changes: { mode: none }`**. El compilador
ya lo sabe; el motor no se lo pregunta.

**Listo cuando.** Una copia vencida contra su `freshness` hace que el motor **declare el estado
degradado**, y hay una prueba que lo provoca. Servir lo viejo como fresco es el fallo que este
proyecto no puede permitirse: para un agente, saber que el contexto está degradado es la
diferencia entre abstenerse y alucinar.

#### Hecho · la marca sí, el valor no — y con eso ya se degrada

**El testigo son dos cosas y solo una depende de que algo se pueble.** La **marca** —con qué se
fecharía— sale de `changes.witness` de la tabla, y es una propiedad del objeto como `reads` y como
`mode`: *una vista no puede fechar mejor que su origen*. El **valor** —hasta cuándo fue cierta—
necesita que alguien la haya poblado, y eso es I5.

Separarlas es lo que hace que este peldaño valga sin I5:

```
  frescura  10m · DEGRADADA — la tabla declara `witness: none`,
            así que la copia no puede decir hasta cuándo fue cierta
…
degradado · 1 copia declara una frescura que no se puede comprobar
```

> **Se sabe que una frescura no se va a poder comprobar nunca sin haber poblado nada.** No hace
> falta esperar a que una copia venza: basta con que su origen no sepa fecharse.

Y **no cambia el código de salida**, que es lo que lo hace útil: declarar `freshness` sobre una
tabla sin testigo es legal, nadie miente. Es una **degradación**, no una fuga — y la diferencia es
la razón de ser del peldaño.

**La otra mitad: el Refresh Analyzer mira la cara `D`.** `analizar_con(plan, emite)` recibe qué
cambios emite cada hoja, y una que declara `mode: none` da `FULL` con su motivo, donde antes daba
`INCREMENTAL` por la forma del plan. Son **dos preguntas distintas** y la pieza solo contestaba
una:

| | |
|---|---|
| ¿se puede incrementalizar este plan? | sobre el **álgebra** |
| ¿se va a mantener esta vista? | además sobre el **origen** |

Un plan lineal sobre una tabla sin cambios es impecable como álgebra y no se refresca nunca,
porque no llega ningún Δ.

**Y la invariante sobrevivió, reforzada.** `Circuito::compilar` ganó su `_con` también: si solo lo
hubiera ganado el analizador, este diría `FULL` de un plan que el compilador construye sin
protestar, y habría otra vez dos definiciones de «mantenible». Ahora la invariante se enuncia
mejor: *valen lo mismo **para el mismo conocimiento de los orígenes***, y la prueba que las
compara recorre los planes **dos veces**, con el mapa vacío y con una hoja que no emite.

Una hoja **ausente** del mapa sigue siendo `INCREMENTAL`: ausencia es *no se declaró*, no *no
emite*. Es la regla de siempre — sin declaración no se supone ninguna — y es lo que deja intacto
a quien construya un plan a mano.

**Lo que se queda fuera, dicho.** La topología entra en el registro **sin marca**, y no por
olvido: la suya es una fecha que el operador pasa a `index refresh --marca`, y esa no está en el
vocabulario de `changes.witness`. Otro síntoma de lo mismo — mientras tenga ruta de refresco
propia, tiene también testigo propio.

### I4 · la topología es una copia más

**Qué.** `lecturas_de_aristas` deja de fabricar `Lectura` a mano: emite el **plan** de la vista de
aristas —`Proyecta([clave, via], …)`— y lo registra. El `.oretopo` deja de ser un mecanismo y pasa
a ser el formato de almacenamiento de esa copia. Su marca de agua pasa a ser el testigo del
registro.

**Teoría.** Es la misma operación que hizo la tabla con el puntero: había N sitios describiendo lo
físico y pasó a haber uno. Aquí hay dos describiendo una copia y pasa a haber uno.

**Listo cuando.** Se puede **borrar** la ruta de refresco propia de la topología sin perder una
sola prueba. Mientras no se pueda, I4 no está hecho — aunque el registro ya la liste.

**No hace.** No cambia el formato del fichero. Un CSR determinista y firmado es una buena idea y
sigue siéndolo.

#### Lo que I1 e I2 le cambiaron, y una decisión ya tomada

**El plan ya existe, y no está en `ore-exec`.** `ore-cli/src/registro.rs::topologia` lo construye
desde el sustrato —`backedBy` → raíz de la vista— y lo registra con sus tres caras. Así que I4 ya
no es *«escribir el plan»*: es **borrar el segundo sitio que lo describe** y hacer que `ore-exec`
lea el primero.

**Dónde vive el registro entonces.** Hoy en `ore-cli`, y `ore-exec` no depende de él ni de
`ore-view`. La salida es un crate propio —`ore-registro`, sobre `ore-core` y `ore-view`— del que
cuelguen los dos. Y **cuesta cero**, medido sobre el `Cargo.lock`:

| crate | cierre transitivo |
|---|---|
| `ore-core` | 25 |
| `ore-view` | 26 |
| `ore-cli` | 34 |
| `ore-exec` | **163** |

`ore-view` aporta a `ore-exec` **exactamente ninguna crate nueva**. La dirección es la única que
respeta la estratificación —el ejecutor está arriba, el álgebra abajo— y `dependencias.rs` sigue
valiendo: vigila el cierre de `ore-cli`, y este no crece.

#### Y el final al que apunta, que es más limpio que el peldaño

`Camino::IndiceDeTopologia` existe porque la topología se **deriva** de `relations`. Mientras se
derive, el registro tiene un alimentador de sustrato y otro de ontología — una pata en cada
paradigma.

> **El final es que la topología deje de derivarse y pase a ser una vista declarada**: dos campos,
> `materialized`, y ya. Entonces `Camino::IndiceDeTopologia` desaparece, la enumeración baja a un
> camino, y el registro se queda con **un solo alimentador**.

Es la misma frase de [`01-table.md`](../vendor/oos/spec/v1alpha8/01-table.md) §2 aplicada aquí:
*materializar es una decisión sobre una consulta*. La topología también.

#### Hecho · una derivación, y no hacía falta ningún crate nuevo

**La nota de arriba proponía un crate `ore-registro` y que `ore-exec` ganara `ore-view`.** Al
mirar el código de cerca hay una salida más pequeña, y la diferencia es de dónde estaba el
duplicado:

> Lo que no se puede duplicar es **la derivación** —qué aristas hay y de qué columnas salen—, no
> la **representación**. La derivación es una lectura de la gramática; la representación es de
> cada consumidor.

Así que vive en [`ore-core/src/aristas.rs`](../crates/ore-core/src/aristas.rs), al lado de
`vistas::respaldo` y `vistas::datasources_de`, que también van de una entidad a lo físico. Los dos
consumidores ya dependían de `ore-core`, y **ninguno necesita al otro**:

| | qué construye encima |
|---|---|
| `ore-exec/plan.rs::lecturas_de_aristas` | la `Lectura` de la fase ③ |
| `ore-cli/src/registro.rs::topologia` | el plan `Proyecta(Lee)` del motor de vistas |

Los dos pasaron de **derivar** a **traducir**. Cero crates nuevos, cero aristas nuevas en el grafo
de dependencias.

**Y de paso se recuperó el camino del binding.** El registro solo miraba `backedBy`; el ejecutor
miraba binding **y** `backedBy`. Unificar por el más pobre habría dejado sin topología, en
silencio, a un paquete v1alpha7 con bindings — legal mientras v1alpha1 lo sea. La derivación
común hace los dos, y `un_binding_da_sus_aristas_igual_que_una_vista` lo ejerce: **ningún otro
fichero del árbol lo hacía**, porque no hay un caso con bindings *y* relaciones a la vez.

**Cómo se comprueba que hay un sitio.** Dos afirmaciones literales, en dos crates:
`ore-exec/tests/plan.rs::las_aristas_del_ejecutor_son_las_del_registro_de_copias` y
`ore-cli/tests/registro.rs::la_topologia_entra_en_el_mismo_registro_y_con_su_ruta_aparte` escriben
los mismos cuatro nombres. Si la derivación se mueve, **las dos se mueven juntas**; si alguien
reintroduce una local, una se queda atrás y se ve.

#### Lo que este peldaño **no** alcanzó, y por qué el criterio estaba mal puesto

El *listo cuando* de arriba dice: *borrar la ruta de refresco propia sin perder una sola prueba*.
**No se puede, y no por falta de trabajo: el criterio va después de I5.**

`ore-exec index build/refresh` es hoy **lo único que puebla una copia en todo el árbol**. Borrarlo
dejaría el `.oretopo` sin quien lo escriba y tumbaría `pruebas-de-fuego/fuentes-reales.sh`, que lo
ejerce de punta a punta contra un Postgres real. Para que esa ruta sobre hace falta que otra
escriba copias — que es I5 — y que el circuito Δ cubra esta.

Así que el criterio se parte en dos, y el primero **ya está**:

| | |
|---|---|
| **una derivación** | ✅ `ore_core::aristas`, con las dos afirmaciones cruzadas |
| **una ruta de refresco** | ⏳ después de I5, y de que el Δ cubra la topología |

Y sigue en pie lo que esto apunta: mientras la topología se **derive** de `relations`, el registro
tiene una pata en cada paradigma. El final es que pase a ser una vista **declarada**, y entonces
`Camino::IndiceDeTopologia` desaparece solo.

#### Sobre el bloqueo que hubo

**`ore-exec` no se pudo construir durante T1–T4 ni al escribir I1–I3.** `cedar-policy` arrastra
`psm` y `stacker`, que compilan C, y no había compilador:

```
error occurred in cc-rs: failed to find tool "gcc.exe": program not found
```

Resuelto instalando `mingw-w64-x86_64-gcc` (MSYS2). **Y al construirlo por fin salieron dos
pruebas rojas que llevaban ahí desde T4**, las dos consecuencia de la migración y ninguna un fallo
del código: `fullScan` lo declara ahora la vista que respalda y no un binding, y la propiedad sin
fuente dejó de poder ocurrir en `acme-retail` —`OOS2022` la convierte en error de compilación en
v1alpha8— así que su afirmación se mudó al caso v1alpha7.

**La lección, que es de proceso y no de código:** un crate que no se puede construir en la máquina
de trabajo acumula rojos que nadie ve, y los acumula en silencio durante cuatro peldaños. El árbol
entero verde en local —**533 pruebas**— es una condición de trabajo, no un resultado.

### I5 · la copia existe

**Qué.** Poblar y refrescar. Y antes, la deuda de T3: **la receta de BigQuery no emite sus dos
caras** —hoy una tabla suya sale con `reads: {}` y `changes: { mode: none }`—, así que no empuja
nada y nada se puede refrescar.

**Y lo que la decisión del sobre le quitó.** Este peldaño se escribió diciendo que si origen y
destino eran el mismo BigQuery, materializar sería **una consulta** —`bq query
--destination_table`— sin que una fila pasara por ORE. **Eso dejó de ser cierto** al decidir que
el artefacto es nuestro: BigQuery no sabe producir nuestro sobre, así que las filas pasan por la
máquina que ejecuta.

No se pierde para siempre, y por eso la carga es Parquet y no un formato propio: el día que
Snowflake o Databricks escriban el Parquet directo a un destino compatible con S3, **cambia quién
produce la carga y no cambia el sobre**. Es la puerta que el ADR 0015 dejó abierta a propósito.

Y lo que sí sobrevive del atajo es lo que más valía: el paso 4 del ciclo. **Se sabe si hay que
copiar sin copiar nada**, con un `HEAD` sobre el nombre del digest.

**Listo cuando.** Una vista declarada `materialized` se puebla contra un dataset real de
BigQuery, el artefacto queda en R2 **nombrado por su digest**, `ore view` dice de ella qué
contesta, dónde vive y hasta cuándo fue cierta — y **un segundo intento con el mismo testigo no
sube ni un byte**.

#### Hecho · el almacén, medido contra un R2 de verdad

[`crates/ore-store-r2`](../crates/ore-store-r2) es la **tercera delegación** del árbol, y hereda
la línea de 0008 llevada a su sitio: *lo que viaja no son llamadas al almacén, es el artefacto*.
Cabecera y filas por **stdin**, una línea JSON por **stdout**, y no sabe qué es una entidad, ni un
conducto, ni una vista.

Medido, con el bucket devuelto a cero objetos:

```
1 · sube la primera vez                  subido=true
    el nombre ES el digest               ore/v1/<sha256 del artefacto>
2 · el mismo testigo, otra vez           subido=false   ← ni un byte
    y al mismo nombre
3 · otro testigo                         otro nombre, y sube
4 · un `Integer` que llega como texto     se niega, y dice cuál columna
```

Y la vuelta entera, releyendo de R2: el sha256 del objeto **es su nombre**, la magia es
`ORECOPY1`, la cabecera es el JSON canónico con los cinco campos, y la carga la lee **pyarrow** —
un motor ajeno, que es la mitad de por qué se eligió Parquet.

**Dos cosas salieron por el camino y las dos son del árbol, no del almacén.**

`ureq` con `native-tls` **no cablea el TLS solo**: sin engancharlo a mano, cada petición sale con
*«no TLS backend is configured»*, que es otro error que se lee como una cosa y es otra — el
tercero de esta familia, con el `1010` de Cloudflare y el `SignatureDoesNotMatch`.

Y **`dependencias.rs` se puso rojo, que es exactamente para lo que existe**: `hmac 0.12` arrastra
`digest 0.10` entera al lado de la `0.11` que el árbol ya usa, y el cierre de `ore-cli` pasó de 34
a 35. La salida no fue subir la constante: HMAC **no es una primitiva** —es una construcción de
seis líneas sobre SHA-256, con vectores oficiales— así que se escribe y la crate se va. El guardián
no detectó un fallo: detectó una dependencia que no hacía falta.

#### Lo que falta para cerrar I5, y qué lo bloquea

| | |
|---|---|
| el almacén delegado | ✅ `ore-store-r2`, verificado contra R2 |
| el sobre y la carga | ✅ `ORECOPY1` + Parquet, deterministas y releídos por un motor ajeno |
| **el ciclo en `ore`** | ⏳ compilar el plan, `digest(plan, testigo)`, `HEAD`, canalizar las filas |
| **BigQuery** | 🚫 **bloqueado** |

#### Hecho · la deuda de T3, contra el dataset real

La receta de BigQuery emitía `reads: {}` y `changes: { mode: none, witness: none }` para **todo**,
y el inductor lo decía con dos comentarios honestos: *«el driver no declaró»*, *«el driver no
sondeó»*. Ahora sondea, con la misma doctrina que `ore-read-postgres`: **solo lo que el servidor
afirma.**

Una consulta más a `INFORMATION_SCHEMA.TABLE_OPTIONS` y dos columnas más a la que ya había — sin
una segunda llamada, que es la propiedad que esta receta compró midiendo.

**La cara `I`:**

| lo que dice el servidor | lo que emite | por qué |
|---|---|---|
| `require_partition_filter = true` | `fullScan: forbidden` + `requiredFilters` | BigQuery **rechaza** la consulta. No es cara: no se puede |
| cualquier otro objeto legible | `fullScan: expensive` | se factura por bytes leídos. `cheap` empujaría al planificador a recorrerlo, y eso es una factura |

Los operadores salen enteros —`eq, neq, in, range, like, isNull`— porque `reads` describe **el
objeto** y BigQuery los contesta todos.

**La cara `D`**, y es la misma tabla de tres filas que la de Postgres:

| | |
|---|---|
| no es `BASE TABLE` | `{none, none}` — una vista no tiene flujo propio; una materializada **se refresca** |
| sin `enable_change_history` | `{none, none}` — sin historial no sale ningún cambio |
| `enable_change_history = true` | `{retract, log}` — el historial trae los borrados |

**Medido contra `trino-k8s.rubix_demo_ventas`**, doce objetos: `pedidos` particionada por
`creado_en` y agrupada por `cod_pais`; `mv_pedidos_por_pais` materializada; dos vistas. Todas
salen con `fullScan: expensive` y `changes: none`, que es lo correcto para lo que declaran.

**Y dos ramas que no se han podido medir en vivo, dichas en vez de supuestas:** `forbidden` —
ninguna tabla del dataset exige filtro de partición— y `{retract, log}` — ninguna tiene el
historial encendido, y encenderlo es modificar el dataset de otro. Las dos se prueban en
`lector.rs`.

**Dos cosas del entorno, no del código.** `bq` no arrancaba —`python3.14: command not found`— y
revive fijando `CLOUDSDK_PYTHON` al Python que sí hay; el propio módulo ya citaba ese error como
ejemplo de por qué la salida del programa delegado se muestra **literal**. Y `gcloud` había
caducado: un `gcloud auth login` interactivo, que no se puede hacer desde aquí.

**Y una que parecía un fallo y no lo era.** El dataset tiene doce objetos y `discover` emitió
diez. Las dos que faltan son `pedidos` y `Pedidos` —el duplicado que nadie se atrevió a borrar— y
no se cayeron en silencio: están en la cola de revisión, `rubix_demo_ventas.Pedidos ·
rubix_demo_ventas.pedidos — colisionan en 'Pedidos'`. Preguntar en vez de inventar, funcionando.

---

## 7. Lo que **no** entra, y no por falta de tiempo

**La cara `writes`.** Materializar escribe en **una copia**, y la copia la declara la vista.
Escribir en **el origen** es otra cosa y necesita `Table.writes` — es M1 de
[`sustrato.md`](sustrato.md), y va después.

**Que la entidad deje de repetir.** M2 de `sustrato.md`. Este plan es del sustrato; ese es de la
abstracción, y el orden importa: se construye sobre lo despejado, no a la vez.

**La superficie de contexto como vista.** Cabe en la forma del registro y no se toca aquí. Se
vuelve una vista cuando la capa semántica repose en el sustrato, no antes.

**Un almacén.** ORE no opera ninguna base de datos, y esto no lo cambia: se registra dónde vive
una copia y se delega en quien tenga el cómputo. La misma frase que el motor lleva desde M0.
