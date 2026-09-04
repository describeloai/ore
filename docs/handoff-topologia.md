# Handoff · la topología, del paradigma viejo al nuevo

> **Este documento es desechable.** Se borra el día que `B4` se ponga en verde. Un plan que
> sobrevive a su ejecución deja de ser un plan y pasa a ser documentación de un pasado que ya
> nadie comprueba.
>
> Fecha: 2026-09-04 · Escrito **después** de medir, no antes. Todo lo que afirma abajo tiene una
> cifra o un fichero detrás, y donde no la tiene lo dice.

---

## 1. Qué pasó, en tres frases

`ore-exec` se retiró: era el camino de lectura del paradigma de entidades y bindings, nacido un
día antes que el motor de vistas. Con él se fue **el único productor del índice de topología**, que
quedó **definido y sin poblar** — un estado que no teníamos.

Al mirarlo apareció que ese índice **es una vista materializada** y lleva siéndolo desde antes de
que hubiera vistas. Este documento lo trae al paradigma nuevo **retirando el viejo**, no mudándolo.

---

## 2. La tesis, y por qué se sostiene sola

> Por cada **relación con `via`** de una entidad con clave simple, el índice guardaba una
> proyección de **dos columnas** sobre su fuente física: **la clave** y **la columna del enlace**.

Y esa proyección **ya está en otro sitio**, por una cadena de tres eslabones que no inventa nada:

| | |
|---|---|
| 1 | **`via` nombra una PROPIEDAD** — `via: [managerId]`, y `aristas.rs` la resuelve por el mapa de la vista |
| 2 | **una propiedad de una entidad es campo de su vista** — `OOS2022`, o declara `derivedFrom` |
| 3 | luego **si la vista se materializa, la copia contiene la arista**: clave y enlace son dos de sus campos |

**Medido, y el número importa porque el eslabón 2 es una regla y no una costumbre:**

```text
corpus (vendor/oos)      entidades con `via`             25
                         sin `backedBy` · camino viejo   23   ← la conformidad no ejercita esto

acme-retail              supply.Shipment   via en su vista: SÍ · vista VIRTUAL
                         hr.Employee       via en su vista: SÍ · vista MATERIALIZADA
                         otras tres        sin `backedBy`
```

**Dos de dos.** Y `hr.Employee` tiene sus aristas dentro de su copia **hoy mismo**, sin que nadie
lo haya pretendido.

### 2.1 · Y la travesía no necesita junta

Es lo que hace que esto sea pequeño, y se comprobó rescatando la firma antes de borrarla:

```rust
fn vecinos(&self, relacion: &str, clave: &str) -> Vec<&str>
```

**Claves entran, claves salen.** La fase ② lo decía: *«el índice, en local → un conjunto de
CLAVES»*. Así que un salto es

```text
Filtra(clave = K)  ∘  Proyecta(via)  ∘  Lee(copia)
```

— recortar, proyectar, leer. **Entero dentro del vocabulario de hoy.** La junta haría falta para
*devolver filas unidas*, que es otra cosa, es `B2`, y **no entra aquí**: `00-scope` §6.1 la excluyó
con tres razones y solo una ha caducado.

---

## 3. Lo que se retira

| | por qué |
|---|---|
| **`Camino::IndiceDeTopologia`** del registro de copias | una arista no es una copia: son dos columnas de la copia de su entidad. Hoy el registro enumera cuatro copias de `acme-retail` que **nadie puebla** |
| **la marca de agua propia y el refresco propio** | eran *«ajenos al circuito Δ»* por su propia confesión. Con la arista dentro de la copia, la refresca `ore materialize` |
| **el formato `ORETOPO1`** | ya se fue con `ore-exec`. Aquí se retira su **hueco**: deja de ser algo que falta |

Y una que **no** se retira y conviene decirlo: `ore_core::aristas` **se queda**. Sigue siendo la
derivación de qué aristas declara un paquete, y es lo que `B0` necesita para saber qué se atraviesa.

---

## 4. Los peldaños

> **Listo es [`B4`](#b4--la-definición-de-listo)**, no que los tres anteriores estén escritos.

| | qué | cuesta |
|---|---|---|
| **B0** | la regla: *lo que se atraviesa se debe materializar* | gramática · un código |
| **B1** | la arista deja de ser una copia aparte | solo retirar |
| **B2** | el salto | un verbo en el almacén |
| **B3** | la cadena | nada nuevo |
| **B4** | **la definición de listo** | es la prueba |

### B0 · `OOS2026` — lo que se atraviesa se debe materializar

**Qué.** El **tercer gemelo** de una familia que ya tiene dos:

| | |
|---|---|
| `OOS2020` | lo que **no se puede leer** se debe materializar |
| `OOS2025` | lo que **se escribe** se debe materializar |
| **`OOS2026`** | lo que **se atraviesa** se debe materializar |

*«Se atraviesa»* es derivable y no se declara: la entidad tiene una `relations` con `via`. El sujeto
de la regla es **la vista que la respalda**, igual que en `OOS2025`, y por eso vive con ella en
`02-view` §5 y en `vistas::comprobar`.

**Su precio, contado antes de escribirla:** una entidad del corpus —`supply.Shipment`— tendría que
declarar `materialized`. Una. Las otras tres que no pasarían no es por esto: **no están migradas**,
y para ellas la travesía murió con `ore-exec` y no vuelve.

> **Y esto es lo que hay que aceptar de frente:** hoy una entidad se puede atravesar sin
> materializar nada, y esta regla lo prohíbe. Es exactamente la misma clase de decisión que
> `OOS2020` —obligar a materializar lo que no se puede servir de otra manera— y se toma con el
> mismo criterio.

**Listo cuando.** Una entidad con `via` respaldada por una vista **virtual** no compila, y el
mensaje **nombra la relación** que lo impide; la misma con la vista materializada compila; y una
entidad **sin** relaciones no ve la regla, ni para bien ni para mal.

### B1 · La arista deja de ser una copia aparte

**Qué.** `Camino::IndiceDeTopologia` sale de `registro.rs`, y con él la enumeración de copias que
nadie puebla. El registro pasa a decir la verdad: las copias de un paquete son **las vistas
materializadas**, y las aristas viven dentro.

**Cuidado con no perder lo que aquello sí decía.** La prueba
`los_caminos_de_refresco_estan_enumerados_y_cada_uno_dice_por_que` existe para que *«hay N
mecanismos de refresco»* no haya que recordarlo leyendo código. Con un solo camino la prueba pierde
sujeto: **o se borra diciendo por qué, o se reescribe sobre lo que queda.**

**Listo cuando.** `ore view` sobre `acme-retail` deja de enumerar cuatro copias de topología; el
registro no lista **ninguna** copia sin productor; y la prueba de los caminos de refresco dice lo
que hay, no lo que había.

### B2 · El salto

**Qué.** Dada una entidad, una relación y una clave, devolver **las claves vecinas** leyendo la
copia. Sin delegado nuevo: `ore` compila el plan y se pone **en medio**, como ya hace
`ore materialize`; `ore-store-r2` lee la copia.

Hace falta **un verbo en el almacén** que hoy no está: `buscar`, `anterior`, `sellar` y `recoger`
saben de sobres y recibos, y ninguno devuelve filas. `carga::leer()` ya existe dentro del crate,
así que el verbo es una puerta, no una pieza.

> **Y una decisión que este documento no toma, con recomendación.** El recorte por clave, ¿lo hace
> el almacén al leer, o `ore` al recibir las filas? Recomendado **el almacén**: es la misma figura
> del empuje al origen —el que tiene el dato aplica lo que sabe aplicar— y evita traer la copia
> entera por un salto. Lo contrario es más simple y se puede medir después.

**Listo cuando.** Un salto devuelve los vecinos correctos, **no abre ninguna conexión al origen**,
y una relación cuya entidad no está materializada **no llega aquí**: la paró `B0` al compilar.

### B3 · La cadena

**Qué.** `k` saltos encadenados. No hay pieza nueva: es `B2` en un bucle, con el conjunto de claves
del salto anterior.

**Listo cuando.** La cadena de mando de un empleado sale entera y en orden; y el número de lecturas
de copia es **exactamente `k`**, no `k`·algo — que es la afirmación que distingue una travesía de
un escaneo repetido.

### B4 · La definición de listo

**Qué.** Nada nuevo: una prueba de fuego con **números afirmados**, al modo de
`pruebas-de-fuego/refresco.sh`. **Nace roja**, y su salida es la lista de trabajo.

Los actos, sobre `casos/jerarquia`. **Y ese paquete ya sirve, medido:**

```yaml
# entities/Employee.yaml          # views/empleados.yaml
relations:                        fields:
  manager:                          employeeId:  employee_id   ← la clave
    target: hr.Employee             managerId:   manager_id    ← el enlace
    cardinality: many_to_one        baseSalary:  national_id
    via: [managerId]                actualizado: updated_at
                                  # y NO declara `materialized`
```

La relación es **reflexiva** —un jefe es un empleado—, la vista **ya expone la clave y el enlace**
sin que nadie lo pretendiera, y **no se materializa**. O sea que *tal cual está* es la negativa
`a`: hoy tendría que fallar con `OOS2026`. Añadirle `materialized` —una línea— lo convierte en el
positivo.

**Eso es la prueba entera en un paquete que ya existe**, y es lo que hace que `B4` no tenga que
inventar terreno. Los actos, con la vista materializada:

| | qué pasa | qué se afirma |
|---|---|---|
| 1 | se materializa la vista de la entidad | la copia lleva **la clave y la columna del enlace** entre sus campos |
| 2 | un salto desde una clave | los vecinos correctos · **1** lectura de copia · **0** conexiones al origen |
| 3 | una cadena de `k` saltos | la cadena entera · **`k`** lecturas · **0** conexiones al origen |
| 4 | se vuelve a saltar sin refrescar | **0** lecturas nuevas si el resultado ya se tenía, o se dice que no se cachea |

Y las negativas, que valen igual:

| | se provoca | tiene que pasar |
|---|---|---|
| a | una entidad con `via` y vista **virtual** | **no compila** · `OOS2026`, nombrando la relación |
| b | una entidad **sin** relaciones y vista virtual | **compila** — la regla no tiene sujeto |
| c | un salto por una relación que no existe | se rechaza nombrándola, **sin leer nada** |
| d | una `via` **compuesta** | se descarta como hoy, y se **dice**: `aristas` la salta a propósito |

**Listo cuando.** Los cuatro actos dan sus números, las cuatro negativas fallan por su motivo, y
—esto es lo que la prueba existe para afirmar— **ninguna travesía abre una conexión al origen**.
Ese era el trato entero del índice de topología, y es el que hay que seguir cumpliendo sin él.

---

## 5. Lo que **no** entra, y no por falta de tiempo

**La junta en el vocabulario de la vista.** Es `B2` del análisis original —otro `B2`, y por eso
este documento numera sus peldaños con el mismo prefijo pero no es lo mismo— y sirve para
**devolver filas unidas**, no para atravesar. `00-scope` §6.1 la excluyó con tres razones
convergentes; **una ha caducado** —la invertibilidad, desde que el ADR 0018 manda la escritura a la
copia— y las otras dos siguen en pie: el precio en la regla de flujo, y la mantenibilidad
incremental.

> Del lado del motor está casi construida: `Une` aparece en **10 de 12 módulos** de `ore-view`, y
> el delta compiler la mantiene incrementalmente y está medido. El coste está **en la costura**:
> `Raiz` es singular, y hay 53 usos de `.datasource`, 42 de `.objeto` y 28 de `.tabla` que lo
> asumen. Reabrirlo de rebote sería exactamente lo que aquella sección avisa de no hacer.

**Elegir formato de carga.** La travesía multi-salto es donde CSR ganaba por construcción, y la
cota está escrita en [`sustrato.md`](sustrato.md) M4: `k`·grado contra `k`·página, y quién gana es
**propiedad del dato**. `B4` produce el primer número real; elegir con uno solo sería extrapolar,
que es lo que el [ADR 0014](decisions/0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md) prohíbe.

**La travesía del camino viejo.** Una entidad respaldada por un `Binding` no tiene vista que
materializar. Su travesía murió con `ore-exec` **y no vuelve**. Sigue compilando todo lo demás,
como siempre.

**Quién responde una lectura.** `ore-exec` era el camino de lectura del paradigma anterior y no hay
sustituto decidido. Esto repone **la travesía**, que devuelve claves; **servir propiedades sigue sin
dueño**, y decirlo aquí es mejor que dejar que alguien lo deduzca de que la travesía funciona.

**`ore cache check`.** Se quedó sin consumidor al retirar `ore-exec` y sin productor de la versión
de topología que transporta. No se toca aquí: la pregunta que contesta es la que el View Matcher
hace, y decidir si son una o dos es trabajo aparte.

---

## 6. Dónde sigue esto cuando se acabe

Este documento se borra en `B4`. Lo que **no** se borra es
[`sustrato.md`](sustrato.md) **M4**, que es donde vive la tesis y donde están las medidas que la
sostienen. Y el registro de decisiones, si `OOS2026` acaba mereciendo un ADR: hoy no lo parece
—es una regla más de la familia de `OOS2020`, no una decisión de arquitectura— pero si al
construirlo aparece que sí, se escribe.
