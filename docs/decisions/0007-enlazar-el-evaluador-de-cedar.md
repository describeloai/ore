# 0007 · Enlazar el evaluador de Cedar

**Estado:** aceptado · **Fecha:** 2026-08-31 · **Decide:** el evaluador se enlaza, y vive fuera del compilador

---

## Contexto

El [ADR 0003](0003-lectura-estructural-de-cedar.md) decidió leer la **forma** de una
política sin evaluar su semántica, y dejó escrita la puerta de salida:

> *«si algún día `ore` necesitara decidir una autorización, la respuesta es enlazar
> `cedar-policy`, no ampliar ese fichero.»*

Ese día es la fase ① del plan de `05-ejecutor` §3: **autorizar poda el plan**, por
principal y por petición. Así que este ADR **no revoca al 0003 — lo ejecuta**. Lo que
el 0003 no podía saber, porque L2 todavía no estaba especificado, es **dónde** vive lo
que se enlaza.

## La medición

Con el mismo cierre transitivo que mide
[`dependencias.rs`](../../crates/ore-cli/tests/dependencias.rs) sobre `Cargo.lock`:

| Raíz | Cierre | Vetadas dentro |
|---|---:|---|
| `ore-cli` | **32** | ninguna |
| `ore-read-postgres` | **114** | **19** — `tokio`, `postgres`, `native-tls`, `openssl`, `schannel`, `security-framework`, `mio`, `socket2`, `getrandom`… |
| `cedar-policy` 4.12 | **153** | **4** — `chrono`, `time`, `time-core`, `time-macros` |

De las 153, **18 ya están en el árbol** y **135 son nuevas**: enlazarlo dentro de
`ore-cli` dejaría su cierre en **167**.

Y el contraste entre las dos filas de abajo es el hallazgo, no el tamaño:

> **El driver está vetado por saber hablar por la red. El evaluador está vetado por
> saber qué hora es.** Dos motivos distintos, la misma costura.

`cedar-policy-core` depende de `chrono` porque Cedar 4 tiene una extensión `datetime`:
una política puede decir *«antes del 1 de enero»*. Entre las 135 nuevas aparecen además
`windows-sys` —FFI de plataforma, exactamente lo que el manifiesto del espacio de
trabajo rechazó por escrito para algo tan pequeño como los colores de `clap`—, `psm` y
`stacker`, que llevan ensamblador.

### La medición que estuvo a punto de cambiar la decisión

La arista a `chrono` es **directa pero opcional**: `cedar-policy-core/datetime` la
activa, y `datetime` está en `default`. Lo evidente era desactivarla y quedarse sin
reloj.

Se midió, y **no funciona**: con `default-features = false` el `Cargo.lock` sale
**idéntico** —162 entradas, y `chrono`, `time`, `time-core` y `time-macros` dentro—
porque el fichero de bloqueo **no depende de las features** a propósito, para que
activar una no lo reescriba.

Eso no es una limitación del guardián: es la razón de ser del guardián, y ya estaba
escrita en `lector.rs` para otra cosa —*«un binario sin código de red no puede hacer una
llamada; uno con una pila TLS enlazada que promete no usarla tiene una política»*—.
Aplicada aquí sale sola:

> **`default-features = false` es una promesa. Un crate aparte es una propiedad.**

## La distinción que decide

El 0003 separó dos preguntas sobre una política. Este separa dos sobre el motor, y la
frontera no es el tamaño del árbol de dependencias:

| | El compilador | El evaluador |
|---|---|---|
| Contesta | *¿qué dice este documento?* | *¿puede **este** principal?* |
| Necesita | el árbol de ficheros | una **petición** |
| Es función de | sus entradas | sus entradas **y del instante** |
| Dos ejecuciones iguales | mismo resultado, siempre (G1) | pueden diferir sin que nada cambie |

`ore validate` no tiene peticiones. Meterle un evaluador no le añadiría una dependencia:
le añadiría **una capacidad que su invariante prohíbe**. El reloj es la evidencia; la
capacidad es el argumento.

## Decisión

**El evaluador se enlaza, y vive en `crates/ore-exec`.**

- `ore-exec` es miembro del espacio de trabajo —entra en `Cargo.lock`, luego entra en el
  radar del guardián— y **queda fuera de `default-members`**, igual que
  `ore-read-postgres`. El build por defecto del repositorio sigue siendo el compilador.
- **`ore-core` no depende de `cedar-policy`.** La lectura estructural del 0003 se queda
  donde está y sigue siendo la que responden `ore diff` y `ore validate`.
- `ore serve` **delega** en el binario `ore-exec` resolviéndolo por PATH, exactamente
  como `lector.rs` resuelve `ore-read-<tipo>`. Sin él instalado, falla diciéndolo.

Lo que compra es una frase que deja de ser una promesa: **el binario que compila no sabe
servir, y se demuestra midiendo.**

### Las dos alternativas descartadas, y por qué

**Una feature en `ore-cli`.** Medida arriba: el `Cargo.lock` no cambia, el guardián salta
igual, y el artefacto distribuido dependería de qué banderas usó quien lo construyó. Es
la definición de una política en lugar de una propiedad.

**Invocar el CLI de Cedar como subproceso**, la misma figura que los drivers. Se descarta
por dónde cae: ① **poda el plan**, así que la autorización está en el camino de la
planificación y no en el de los datos — un subproceso por decisión convertiría lo único
que hoy es puro y barato en la parte cara. Y el evaluador necesita el almacén de
entidades en memoria para resolver `resource in principal`.

## Las dos consecuencias que importan

**El esquema que carga el evaluador tiene que ser el que emite `ore export --format
cedarschema`.** No un segundo esquema equivalente: **el mismo**. Si divergieran, la
política se habría validado contra uno y se evaluaría contra otro, y ninguna prueba lo
vería — el fallo no tiene aspecto de fallo. La prueba de fuego de
[`00-overview`](../../vendor/oos/spec/v1alpha1/00-overview.md) §4.1.1 existe justamente
porque esa proyección puede estar mal y validar limpia.

**Entra el reloj, y con él un tercer eje de auditoría.** `05-ejecutor` §7 fijó dos:
el **digest** —qué significaba— y la **marca de agua** —hasta cuándo era cierto—. Una
política con `datetime` añade el que faltaba: **cuándo se autorizó**. Sin él, *«la misma
pregunta devolvió cosas distintas»* no se puede distinguir de un fallo, porque tiene
exactamente el mismo aspecto.

> Toda respuesta que dependa de una política con `datetime` **debe poder acompañarse del
> instante contra el que se evaluó.** Es el mismo argumento que produjo los otros dos
> ejes, aplicado a la entrada que este ADR introduce.

## Lo que se acepta a cambio

- **El guardián sobreestima.** Mide el cierre del `Cargo.lock`, no lo que el enlazador
  mete en el binario: muchas de las 135 son opcionales y no se compilarían nunca. Es un
  cambio de precisión por auditabilidad, y es deliberado — leer el lock no necesita
  toolchain, ni red, ni resolución de features, y **no lo puede desmentir una bandera
  que alguien cambie en una matriz de CI**.
- **Un segundo artefacto que distribuir.** ADR 0004 describe un binario; ahora son tres
  familias —`ore`, `ore-exec`, `ore-read-<tipo>`—, y las tres necesitan la misma
  atestación de procedencia. La alternativa era un binario que lo hace todo, que es la
  que este documento descarta.
- **El cierre de `ore-exec` no está medido contra el árbol real**, solo aparte. La cifra
  exacta se fija cuando el crate exista, y `CIERRE` gana un hermano en el guardián.
- **`ore-exec` no compila en una máquina sin MinGW ni MSVC**, y se midió al crearlo:
  `error calling dlltool 'dlltool.exe': program not found`, sobre `windows-sys` y
  `parking_lot_core`. Es el mismo muro que obliga a construir el driver en Docker, y
  llega por la misma puerta que el reloj. **El compilador entero sigue construyéndose
  sin nada de eso** — la herramienta demuestra la costura sin que nadie se lo pida, que
  es lo que el manifiesto del espacio de trabajo ya decía del driver.
