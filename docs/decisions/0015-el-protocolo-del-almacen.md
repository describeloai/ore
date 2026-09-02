# 0015 · El protocolo del almacén

**Estado:** aceptado · **Fecha:** 2026-09-02 · **Decide:** que una vista materializada es un
**artefacto nombrado por su digest**, con un sobre nuestro y una carga en Parquet, y que subirlo
es de un programa delegado — porque `ore` no puede abrir un socket

---

## El problema

`materialized` lleva desde v1alpha7 siendo una declaración sin ejecutor. El compilador comprueba
su flujo —`OOS4001`, `OOS4002`, `OOS4011`— y `OOS2020` la exige donde el origen no se deja leer,
y después de eso **no pasa nada**: ninguna línea del árbol escribe una copia.

Con el almacenamiento del lado de ORE —[`sustrato.md`](../sustrato.md) §3.3— eso deja de ser un
hueco de implementación y pasa a ser una decisión de forma, porque hay que contestar tres cosas
que no tienen respuesta obvia:

1. **qué es** una copia, como cosa que se guarda;
2. **quién la escribe**, dado que `ore` no puede;
3. **cómo se nombra**, que es lo que decide si dos escrituras del mismo plan chocan o no.

Y una restricción que no es negociable y que ya está hecha cumplir por una prueba:
[`tests/dependencias.rs`](../../crates/ore-cli/tests/dependencias.rs) lee el `Cargo.lock` y
**falla si aparece una crate de red, de TLS o de FFI en el cierre de `ore-cli`**. El compilador no
es hermético por promesa: lo es porque no tiene el código.

---

## Decisión

> **Una copia es un artefacto: un sobre nuestro alrededor de una carga en Parquet, nombrado por
> su digest, inmutable, y subido por un programa delegado.**

### El sobre

Es la misma figura que el `.oretopo` de [ADR 0006](0006-el-artefacto-de-topologia.md) —magia,
digest del bundle, marca de agua, y luego el CSR—, con otra carga dentro:

```text
"ORECOPY1"        8 bytes
cabecera          JSON canónico, longitud + bytes
  plan            el digest del plan que esta copia contesta
  esquema         qué columnas produce, y de qué tipo
  testigo         { modo, valor } — hasta cuándo fue cierta
  conducto        cuál autorizó la copia
  bundle          contra qué compilación se construyó
carga             Parquet
```

La cabecera es JSON canónico por lo mismo que todo lo demás en este proyecto: dos construcciones
sobre la misma instantánea tienen que dar **los mismos bytes**, o el digest no nombra nada.

### Por qué Parquet dentro, y no un formato nuestro

El sobre es nuestro **porque las tres cosas que lleva no las lleva ningún formato**: qué plan
contesta, hasta cuándo fue cierta, y quién la autorizó. Eso no cabe en un pie de página de
Parquet sin inventarse un convenio, y un convenio inventado es un formato propio con peor prensa.

La carga es Parquet **porque el sobre no tiene por qué saber leer columnas**. Y compra dos cosas
que un formato propio no da:

- **cualquier motor la lee.** Una copia deja de ser un fichero que solo ORE entiende;
- **algún día la escribe el origen.** Snowflake y Databricks escriben Parquet a un destino
  compatible con S3. Con carga propia, las filas tendrían que pasar por nosotros **siempre**;
  con Parquet, esa puerta queda abierta sin decidir hoy que se cruza.

### El nombre es el digest

```text
ore/v1/<sha256 del artefacto entero>
```

Y **no hay ningún puntero mutable en el almacén**. Cuál es la copia vigente lo dice el bundle,
que ya está versionado y firmado. De ahí salen tres cosas que no hay que programar:

- **no hay carrera.** Dos escritores que lleguen al mismo nombre escriben los mismos bytes,
  porque el nombre **es** el contenido;
- **re-materializar es idempotente.** El digest cubre el testigo, así que el mismo plan leído en
  el mismo estado del origen da el mismo objeto — y el trabajo se ahorra con un `HEAD`, antes de
  pedirle una fila a nadie;
- **ramificar sale gratis.** Una rama nombra otro digest. El almacén no se entera.

### Quién escribe

`ore` no puede, así que es la **tercera vez** que este árbol hace lo mismo, y por la misma razón:

| | qué delega | ADR |
|---|---|---|
| `ore-read-<tipo>` | leer filas de un origen | [0008](0008-el-protocolo-del-driver.md) |
| `ore-maintain` | correr el circuito Δ | [0013](0013-el-protocolo-del-mantenedor.md) |
| **`ore-store-<tipo>`** | **sellar y subir el artefacto** | este |

Y el protocolo hereda la línea de 0008 —*«la petición es un fragmento del plan, no SQL»*—
llevada a su sitio:

> **Lo que viaja no son llamadas al almacén: es el artefacto.** El programa recibe la cabecera y
> un flujo de filas por stdin, y devuelve el digest y la ubicación por stdout. No sabe qué es una
> entidad, ni un conducto, ni una vista.

La credencial del almacén **no entra nunca en el espacio de direcciones de `ore`**, que es la
misma doctrina que `source add` aplica desde v1alpha1: *declara dónde buscar el secreto, no cuál
es*.

### El ciclo

| | |
|---|---|
| 1 | `ore` compila: el plan, su digest y el conducto que lo autoriza |
| 2 | comprueba el flujo — **ya existe** |
| 3 | le pregunta al origen su testigo |
| 4 | calcula `digest(plan, testigo)` y hace un **`HEAD`**. Si está, **termina aquí** |
| 5 | si no: `ore-read-<tipo>` produce filas, `ore` las canaliza, `ore-store-r2` sella y sube |
| 6 | registra la copia |

El paso 4 es el que paga el diseño: **se sabe si hay que copiar sin copiar nada**.

---

## Medido contra un R2 de verdad

Antes de escribir esto, no después. Sobre el bucket `materialized-views`, que quedó con cero
objetos:

| | |
|---|---|
| listar · escribir · releer · borrar | OK, los bytes coinciden |
| `If-None-Match: *` en `PutObject` | **la honra** — la segunda escritura da `PreconditionFailed (412)` |
| `ChecksumSHA256` enviado en el `PUT` | **lo valida el servidor**: uno equivocado da `BadDigest (400)`, y `head` lo devuelve |
| multiparte, 2 partes | OK, sha256 coincide al releer |
| enumerar por prefijo con `Delimiter` | OK |
| `CopyObject` | OK |
| **`CopyObject` con `If-None-Match: *`** | **NO protege**: la segunda copia al mismo nombre se acepta |

La última fila es la que cambió el diseño, y por eso está aquí y no en un comentario.

### Las dos rutas de subida, y por qué la segunda no pierde nada

**Cabe en un `PUT`:** se construye el artefacto en local, se conoce su digest antes de subir, y se
sube directo al nombre definitivo con `If-None-Match: *` y `ChecksumSHA256`. Garantías completas,
y la integridad **la valida el servidor** en vez de confiar en el cliente.

**No cabe:** multiparte a una clave de preparación, digest calculado al vuelo, y `CopyObject` al
nombre definitivo. Ahí se pierde el condicional — y **no importa**:

> Cuando el nombre es el contenido, la carrera es inofensiva. Dos escritores que lleguen a la vez
> escriben los mismos bytes.

El condicional nunca fue el mecanismo de corrección: es un ahorro. **El ahorro de verdad está en
el paso 4**, que evita la lectura entera, y ese no depende del almacén.

### Y una que no se ha medido

Si `ChecksumSHA256` funciona en una subida **multiparte**. En S3 el checksum de un multiparte es
compuesto y no es el hash del objeto, así que probablemente haya que verificarlo releyendo. Está
sin medir y se dice, en vez de suponerse.

---

## Lo que se acepta a cambio

**Un binario más que resolver en el `PATH`.** Es el precio de la herméticidad, y ya se paga dos
veces. Quien no materialice no lo necesita.

**Las filas pasan por la máquina que ejecuta.** Hoy, con `ore` canalizando de un proceso a otro.
Es lo que hace que el empuje al origen sea una optimización futura y no una condición: el día que
Snowflake escriba el Parquet directo, cambia quién produce la carga y **no cambia el sobre**.

**El almacén acumula objetos que nadie nombra.** Una copia cuyo digest ya no está en ningún bundle
es basura, y recogerla es otra decisión — con una propiedad buena de partida: **nada la referencia
por nombre mutable**, así que borrarla no puede romper a nadie que la estuviera usando por
casualidad.

---

## Lo que esto cierra, y lo que no

**Cierra** las tres preguntas del problema: qué es una copia, quién la escribe y cómo se nombra.

**No cierra**, y va aparte:

- **el registro** — que el motor sepa qué copias existen y decida si una contesta una consulta.
  Es I1 e I2 de [`handoff-materializacion.md`](../handoff-materializacion.md), y este ADR solo le
  da el testigo con el que trabajar;
- **la identidad de solo lectura.** Hoy el bucket solo tiene credencial de escritura.
  `05-ejecutor` §6.2 pide que quien refresca y quien responde puedan ser distintos, y el R2 Data
  Catalog ofrece una salida que conviene mirar: su `/v1/config` devuelve un `s3.signer.uri`, o
  sea que **puede vender credenciales acotadas por tabla**. Sin decidir;
- **la cara `writes`.** Esto escribe en **una copia**, que la vista declara. Escribir en el
  **origen** es otra cosa y necesita `Table.writes` — M1 de [`sustrato.md`](../sustrato.md).

Y una nota operativa que costó un rato encontrar: **el borde de Cloudflare rechaza peticiones sin
`User-Agent` reconocible** con `error code: 1010`, que se lee como un fallo de autenticación y no
lo es. Cualquier cliente nuestro tiene que mandar uno.
