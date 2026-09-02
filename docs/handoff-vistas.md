# Handoff · el paradigma de vistas

> **Este documento es desechable.** Se borra el día que `Kind::Binding` deje de existir en el
> árbol. Un plan que sobrevive a su ejecución deja de ser un plan y pasa a ser documentación
> de un pasado que ya nadie comprueba.
>
> Fecha: 2026-09-01 · Escrito después de mirar cómo lo resolvieron Cognite Data Fusion,
> Palantir Foundry y Snowflake, y **decidido**: la vista absorbe al `Binding` por completo.

---

## 1. Qué es una vista

Antes de la forma, la naturaleza. Todo lo demás de este documento se deduce de esta sección,
así que si algo de abajo la contradice, gana esta.

> **Una vista es la unidad de procedencia: lo más pequeño que puede tener nombre, versión,
> dueño y frescura declarada, y que contesta *de dónde sale esto y qué es* — sin poseer el
> dato y sin decidir quién lo ve.**

### 1.1 · Es el primer objeto con nombre propio

Debajo de la vista no hay nada cuya identidad controlemos. La tabla del cliente se renombra,
se migra, se parte en dos y se archiva, y nadie nos pide permiso. Encima de la vista todo es
nuestro: entidades, conceptos, retículos, políticas.

**La vista es el objeto frontera**, y hoy ese sitio lo ocupa el `Binding`, que **no es un
objeto**: es una arista. No tiene versión, ni dueño, ni frescura, ni se puede apuntar. Por eso
no aguanta el peso — se le pidió que fuera el mapa y resulta que hacía falta un nodo.

### 1.2 · No contiene datos, y materializar no cambia eso

Una vista es la **promesa de cómo obtener** un dato, no el dato. Materializarla cambia
**dónde se lee**, no **qué es**.

Esto no es una preferencia: es lo mismo que ya está medido un nivel más arriba. El mismo
paquete como árbol de ficheros y como `.oob` digiere igual — *el contenedor no cambia la
identidad*. Aquí es la misma frase aplicada al plano físico.

> **Una vista virtual y una vista materializada son el mismo documento con un campo
> distinto.**

Foundry las tiene como dos clases de cosa —dataset y *virtual table*— y de ahí le salen
ciudadanos de segunda: en modo virtual no hay `@incremental` con pushdown y hay operaciones
que no valen. Con **una** clase y un campo, materializar es una decisión de operación y no una
mutación de naturaleza.

### 1.3 · Se compone sobre vistas, y por eso no hay «pipeline»

Una vista puede leer de una fuente **o de otras vistas**. Con eso, limpiar e integrar dejan de
necesitar un concepto nuevo:

> **Un pipeline es una cadena de vistas.**

Foundry tiene datasets *y* transforms. Snowflake tiene tablas, vistas, dynamic tables *y*
tasks. Cognite lo hace bien y es de quien lo tomamos: una `View` mapea propiedades de **uno o
varios** containers, las aplana y las renombra, y con `implements` hereda de otras vistas.
Nosotros no tenemos containers, así que nos queda solo la mitad de arriba — que es toda.

### 1.4 · Declara su testigo de versión, y «ninguno» es una respuesta legal

Lo que hace reproducible a una porción de dato ajeno no son los bytes: es un **testigo de
versión** que el origen sepa emitir.

| Origen | Testigo |
|---|---|
| Iceberg, Delta | `snapshot-id` |
| Postgres, Oracle, SQL Server | LSN · SCN · posición CDC |
| Kafka | offsets por partición |
| BigQuery | snapshot de tabla |
| SAP | change pointer · delta token ODP |
| **SFTP, Excel, CSV, REST, CRM** | **ninguno** |

La última fila es la mitad del mercado real y no admite eufemismos. Por eso el campo es
obligatorio y `none` es un valor legal **con precio**: una vista sin testigo **no es
reproducible si no se materializa**, y eso es una consecuencia que se dice al compilar, no una
sorpresa en una auditoría.

Es denegación por defecto (P4) aplicada a la procedencia: lo que no se declara no se supone.

### 1.5 · Tiene dos identidades, y confundirlas sería el fallo silencioso

| | Qué es | Qué contesta |
|---|---|---|
| **digest de la declaración** | parte del digest del bundle | **qué es** esta vista |
| **testigo del contenido** | el token del origen al leer | **hasta cuándo era cierto** |

Son los dos ejes de `05-ejecutor` §7 bajados un piso. Una vista cuya declaración cambió es
otra vista aunque los datos sean los mismos; una vista cuyo testigo avanzó es la misma vista
con dato más nuevo. **Meterlos en un solo número haría indistinguible «cambió la regla» de
«cambió el dato»**, que es exactamente el fallo que `cache::ReglaDistinta` existe para separar.

### 1.6 · Propaga la clasificación hacia arriba, y por eso puede negarse a compilar

Esto es lo único de esta lista que no tiene nadie, y es la razón de que el resto valga.

Foundry propaga *markings* por el linaje y los aplica **al acceder**. Cognite declara el
linaje —cada propiedad de una vista nombra su container y su `containerPropertyIdentifier`—
pero no lo confronta con una clasificación. dbt no sabe qué es `gdpr.sensitivity`.

Con vistas encadenadas y etiquetas efectivas, la frase que ya emitimos sobre una
materialización se puede emitir sobre **un eslabón de la cadena**:

```text
error[OOS4002]  hr.empleados_limpios  ←  hr.empleados_crudos.nationalId
                etiqueta del origen      : gdpr.sensitivity = critical
                autorización del conducto: gdpr.sensitivity = medium
```

> **El linaje se comprueba al compilar, no se observa al ejecutar.**

### 1.7 · La frescura se declara en la vista, no en la consulta

`freshness: 15m` es el `TARGET_LAG` de las *dynamic tables* de Snowflake: declaras cuánto
retraso toleras y el motor dice la verdad sobre si lo cumple. No orquestas.

Ya está construido: es contra esto que `cache::Veredicto::Rancia` mide. Hoy el SLA llega por
bandera porque no tenía dónde vivir.

---

## 2. Qué **no** es una vista

Igual de importante, y una de las tres sale de una limitación ajena que conviene no heredar.

### 2.1 · No decide quién ve qué

Las *restricted views* de Foundry fusionan proyección y seguridad en un objeto, y el precio
está documentado: **una restricted view no puede ser entrada de un transform.** La proyección
segura es un nodo terminal, así que gobernar corta la cadena.

Nosotros no tenemos ese problema **si no lo creamos**: la seguridad es el `ConduitPolicy` y las
políticas Cedar, aplicados al compilar y al autorizar. Una vista dice **qué existe y qué es
físicamente**; el conducto dice **quién puede**. Mantenerlos separados es lo que deja que una
vista gobernada siga siendo componible.

> De las cinco caras que la industria le cuelga a la palabra «vista» —referencia, proyección,
> semántica, frescura, seguridad— **la vista se queda cuatro. La quinta se queda donde está.**

### 2.2 · No lleva significado

`is:`, los conceptos, los retículos y las interfaces siguen en la entidad. La vista es física.
Si una vista supiera qué significa una columna habría dos sitios diciéndolo, y el día que
discrepen ninguno dirá cuál manda.

### 2.3 · No es un motor de cómputo

La transformación se **declara**, y la ejecuta quien tenga el cómputo — empujada al origen o
delegada, como todo lo demás aquí. Es la misma frontera que `ore-fetch`, `ore-sign`,
`ore-log`, `ore-read-<tipo>` y `ore-invoke`.

Y arrastra la decisión difícil, que **no está tomada** y es de §6 · V2: el vocabulario de
operaciones es cerrado —seleccionar, renombrar, convertir, filtrar, unir, deduplicar por
clave— y lo que no quepa entra como **expresión opaca declarada**, que cuesta la garantía de
análisis y **tiene que declarar sus entradas y salidas igual**, para que las etiquetas sigan
fluyendo de forma conservadora. Es la misma figura que `effects:` en una función.

### 2.4 · No es un almacén

Ni inventamos formato de tabla ni de fichero. Iceberg y Parquet ya están, y Foundry migró ahí.
Lo único que inventamos sigue siendo lo que ya inventamos: la forma canónica, el `.oob`,
`ORETOPO1` y el manifiesto.

---

## 3. La forma

Lo normativo es `vendor/oos/spec/v1alpha7/01-view.md`; esto es el boceto del que salió, con
las dos cosas que cambiaron al escribir el esquema: **`from` es una fuente, no una lista**
—sin junta en el vocabulario, dos fuentes no tendrían forma de combinarse— y **`where`**, que es
el `selector` mudado y que el boceto había olvidado.

```yaml
apiVersion: oos.dev/v1alpha7
kind: View
metadata:
  name: empleados
  namespace: hr
spec:
  owner: team:rrhh

  # De dónde. Una fuente declarada, u OTRA VISTA (`from: { view: … }`).
  from:
    datasource: hr_workday
    object: "Worker"

  # El testigo. Obligatorio; `none` es legal y tiene precio.
  # `none | snapshot | log | field` — todos ordinales.
  version:
    witness: none

  # Cuánto retraso se tolera. Es `TARGET_LAG`.
  freshness: 15m

  # Qué sale y cómo se llama. Es `Binding.properties`, mudado.
  fields:
    employeeId: "Worker_Reference.ID"
    baseSalary:
      column: "Compensation_Data.Base_Pay.Amount"
      physicalType: "decimal(18,2)"

  # Qué filas son suyas. Es `Binding.selector`, mudado: la misma gramática cerrada.
  where:
    "Worker_Status": Active

  # Qué sabe hacer el origen. Es `Binding.capabilities`, mudado.
  capabilities:
    predicatePushdown: [eq, in]
    fullScan: forbidden
    requiredFilters: [employeeId]

  # Dónde vive lo materializado, SI se materializa. Ausente = virtual.
  materialized:
    datasource: lago
    table: "lago.cache.hr_employee"
```

Y la entidad:

```yaml
kind: Entity
spec:
  backedBy: hr.empleados
```

### 3.1 · La flecha se invierte, y es la decisión de forma más importante

Hoy el `Binding` nombra a la entidad: `targetEntity: hr.Employee`. **Mañana la entidad nombra
a la vista.**

No es cosmético. Con la flecha de hoy, lo físico tiene que conocer lo semántico, así que una
vista no puede existir hasta que alguien haya modelado una entidad. Invertida:

- una vista **existe antes** de que nadie modele nada — que es exactamente el flujo que se
  quiere: *descubrir, elegir qué exponer y con qué frescura, y modelar después*;
- **varias entidades** pueden respaldarse de la misma vista sin duplicarla;
- lo físico deja de saber de significado, que es §2.2 hecho estructura.

Es la misma dirección que Cognite: un container no sabe de vistas, y una vista no sabe de data
models.

---

## 4. Qué absorbe, qué se muda y qué se queda

### 4.1 · El `Binding` desaparece

| Hoy | Mañana |
|---|---|
| `Binding.datasourceRef` + `source` | `View.from` |
| `Binding.properties` | `View.fields` |
| `Binding.capabilities` | `View.capabilities` |
| `Binding.materialization` | `View.materialized` + `View.freshness` |
| `Binding.targetEntity` | **invertido** → `Entity.backedBy` |
| `Kind::Binding` | **no existe** |

### 4.2 · Dos cosas dejan de ser conceptos propios

**El manifiesto de caché.** `cache::Entrada` lleva entidad, propiedades, bundle, topología,
marca, tabla y datasource. Eso **es una vista con `materialized:` puesto**. No se tira nada:
E1 y E4 se reencuadran y los cuatro veredictos siguen siendo los correctos, porque preguntan
lo correcto. Lo que cambia es que dejan de vivir en un fichero aparte.

**El índice de topología.** Es la materialización de una **vista de aristas** — y la prueba de
que siempre lo fue está escrita en el ADR 0006: *«el driver no se entera de que esto es un
índice: las aristas se leen con una petición de la fase ③ cuya proyección se llama `desde` y
`hasta`»*.

### 4.3 · Lo que no se mueve

El retículo, el conducto, el concepto, `is:`, la interfaz, la forma canónica, el digest, la
firma, el log de transparencia, el registro, `ore diff`, el informe. **La pieza que se añade no
invalida el núcleo** — y eso es lo que hace que una feature de este tamaño sea absorbible en
vez de una reescritura.

### 4.4 · Lo que cambia de salida

`discover` hoy salta de catálogo a entidades. Con vistas propone **vistas primero** —qué
expones, de dónde, con qué testigo y con qué frescura— y entidades después. Es el flujo que
ya usan quienes venden esto, y el que el inductor debería haber tenido.

---

## 5. Y una consecuencia que no se buscaba: Apache Ossie

`00-overview` §387 registra desde v1alpha1 por qué no podemos emitir a Ossie:

> *«su `Dataset` exige `source` y cada `Field` exige `expression`: ambos viven en el
> `Binding`. Una `Entity` sola no puede ser un modelo Ossie válido.»*

Una vista lleva `source` y expresiones. **Con vista, un paquete OOS pasa a ser emitible a
Ossie**, que desde el 27 de enero de 2026 es especificación v1.0 en la incubadora de Apache
con Snowflake, dbt, Salesforce y Denodo dentro. La especificación llevaba cuatro versiones
anotando el impedimento; la vista lo quita de paso.

---

## 6. Los peldaños

> **Desde aquí es desechable.**

| | Qué | Dónde |
|---|---|---|
| **V0** | `kind: View` que compila: esquema, normalización, digest — **hecho**: `crates/ore-core/src/vistas.rs`, `spec/v1alpha7`, trece casos en `conformance/v1alpha7` | `ore-core` ✅ local |
| **V1** | `Entity.backedBy`, y la fase ③ lee de vistas | `ore-exec`, CI |
| **V2** | **vista sobre vistas** — el vocabulario de operaciones | `ore-core` |
| **V3** | el flujo de etiquetas **atraviesa la cadena y se niega a compilar** | `ore-core` |
| **V4** | `discover` propone vistas primero | `ore-cli` |
| **V5** | el `Binding` se borra, y este documento con él | todo |

**V0 y V1 conviven con el `Binding`**, y es el único momento del proyecto en que dos cosas
dirán lo mismo. Está acotado a dos peldaños y es el puente; V5 lo cierra. Si el puente se
queda puesto, hemos fallado.

**V0 llegó con parte de V2 y de V3 dentro**, porque no se podían separar: una vista sobre otra
es un `from: { view }` y la cadena se compone en `vistas::raiz`; y la etiqueta de una entidad
baja por la cadena hasta la vista que se materializa (`flow::vistas_materializadas`), que es
el caso `materialized-view-leaks-entity-label`. Lo que queda de V2 es el vocabulario más allá
de seleccionar, renombrar y recortar; lo que queda de V3 es el linaje **por columna** con la
arista `INDIRECT`, que es del motor de vistas y no del núcleo.

**V2 es el peldaño difícil** y no está diseñado. Lo que decide si esto es un producto o un dbt
peor es el vocabulario cerrado y el precio de la escapatoria — §2.3.

**V3 es el que vale dinero.** Todo lo anterior es fontanería que otros ya tienen.

---

## 7. Lo que **no** entra, y no por falta de tiempo

**Milisegundos.** Una lectura de propiedades es tan fresca como el origen porque va federada;
lo materializado y las travesías son de ventana. Para cliente, contrato, activo, contador y
pedido eso es lo correcto. Para telemetría de red a nivel de señal, no — y decirlo al revés
sería la clase de promesa que este proyecto no hace.

**Un motor de cómputo.** §2.3.

**Formato de tabla o de fichero propio.** §2.4.

**Seguridad dentro de la vista.** §2.1, y con la medida ajena que lo justifica.
