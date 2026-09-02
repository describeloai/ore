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
| **I5** | la copia existe de verdad — [ADR 0015](decisions/0015-el-protocolo-del-almacen.md) | `ore-cli`, `ore-store-r2` | BigQuery se materializa a sí mismo con **una** consulta, medido contra un dataset real |

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

### I2 · el matcher decide

**Qué.** `cotejar` y `sello` conectados: qué copia contesta una consulta, con qué compensación, y
qué clasificación hereda. Y las restricciones desde `primaryKey` y las relaciones, que
[`view-engine.md`](view-engine.md) §6 lleva pendientes desde que se escribió.

**Y un hallazgo que ya se puede anticipar.** La declaración de OOS dice **dónde** vive la copia y
no **qué columnas** tiene; `Materializacion` exige que la tabla produzca lo que el plan produce.
Construir esa `Lectura` desde el plan es la única opción honesta — y entonces
`Registro::TablaNoCorresponde` **no puede dispararse nunca desde este camino**. Eso dice algo
sobre para quién existe esa comprobación, y hay que escribirlo donde se vea.

### I3 · el testigo

**Qué.** El testigo deja de estar vacío. Vocabulario: el de `changes.witness`. Y el Refresh
Analyzer pasa a ver la cara `D`: hoy `analizar(plan)` decide por la forma del plan y **diría
`INCREMENTAL` de una vista sobre una tabla que declara `changes: { mode: none }`**. El compilador
ya lo sabe; el motor no se lo pregunta.

**Listo cuando.** Una copia vencida contra su `freshness` hace que el motor **declare el estado
degradado**, y hay una prueba que lo provoca. Servir lo viejo como fresco es el fallo que este
proyecto no puede permitirse: para un agente, saber que el contexto está degradado es la
diferencia entre abstenerse y alucinar.

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
