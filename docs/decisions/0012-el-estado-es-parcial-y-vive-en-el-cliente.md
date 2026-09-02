# 0012 · El estado es parcial y vive en el cliente

**Estado:** aceptado · **Fecha:** 2026-09-01 · **Decide:** dónde vive lo que el mantenimiento
incremental tiene que recordar, y que eso no reabre el ADR 0006

---

## El problema

El [ADR 0006](0006-el-artefacto-de-topologia.md) decidió que **ORE no opera ninguna base de
datos**: su estado son dos artefactos inmutables y una tabla que no es nuestra. Al diseñar el
mantenimiento incremental de vistas apareció algo que parecía contradecirlo.

DBSP —la teoría que hace mecánica la incrementalización— lo dice sin margen: *«aunque `Q` sea
una función pura, `Q^Δ` tiene estado, y ese estado vive enteramente en los operadores de
retardo»*. Una junta necesita los dos lados integrados; un agregado, su entrada. **Ese estado hay
que ponerlo en algún sitio**, y el sitio era una decisión que nadie había tomado.

## Las tres formas del sector

| | Cómo lo resuelve | Qué exige |
|---|---|---|
| **Materialize** | *arrangements*: índices en memoria por clave y tiempo | operar un servicio con memoria |
| **Feldera** | desborda a disco con *checkpoints* incrementales y un log de entrada | operar un almacén |
| **Noria** (OSDI'18) | **estado parcial**: cada operador mantiene solo un subconjunto; los desalojos fluyen hacia delante y las *upqueries* hacia atrás repueblan lo que falte | casi nada |

Las dos primeras poseen sus datos. Nosotros no.

## Decisión

**El estado es parcial, y los bytes viven en el almacenamiento del cliente.** La forma de Noria,
y no por casualidad: en un sistema que posee sus datos la *upquery* va a un operador de más
abajo; en el nuestro, **la de más abajo es la fuente del cliente**, y preguntarle es exactamente
lo que el Pushdown Planner ya sabe planificar.

> **Una *upquery* es un plan.** Y un fallo de estado es una lectura a la fuente.

Lo que ORE guarda sigue siendo metadato: qué claves están calientes, bajo qué identidades
—bundle, topología, marca—, y la política de qué se hace con cada delta según la clave esté
presente, en vuelo o ausente. Está construido en `crates/ore-view/src/state_store.rs` como
contrato de referencia, sobre un Z-set en memoria; **la política es esa y no cambia con el
sitio**. Sostener los bytes es de un programa delegado, con la frontera de siempre: por stdin, y
lo que devuelve no se cree.

### Por qué no reabre el ADR 0006

Parecía que lo hacía, y el primer borrador del plan lo daba por perdido. No: **el estado parcial
no es una base de datos nuestra**. Es un manifiesto con granularidad de clave —el mismo
`cache::Manifiesto` de E1, más fino—, y los bytes viven donde ya vivían. *«ORE no opera ninguna
base de datos»* sigue siendo cierto con la misma literalidad de antes.

### Las reglas que la decisión arrastra

Siete, cada una de un sitio, y las siete con prueba:

- un *miss* produce **una** *upquery*; leer la misma clave ausente dos veces no produce dos;
- un delta para una clave **ausente** se **descarta** — la próxima lectura repone desde la
  fuente, que es la verdad;
- un delta para una clave **en vuelo** se guarda y se aplica **solo si es más nuevo** que el
  relleno — aplicar lo que el relleno ya contenía lo contaría dos veces;
- un delta **más viejo** que lo que hay se descarta;
- un relleno **no pedido** se rechaza — P4;
- un relleno bajo **otro bundle** u otra topología se rechaza — la regla de E1 a granularidad de
  clave;
- se desaloja la clave **menos leída**, sobre un contador lógico, no sobre un reloj.

Y **la marca es un ordinal**: todos los testigos —LSN, SCN, offset, `snapshot-id`— están
totalmente ordenados, y se modelan como `u64`. Sin reloj que leer ni fecha que interpretar.

## Lo que se acepta a cambio

- **Un *miss* cuesta un viaje a la fuente.** Es el precio del estado parcial, y es el precio
  correcto para un sistema que no posee los datos: la alternativa es poseerlos.
- **La frescura de una clave ausente es la de la fuente.** No hay nada que declarar sobre lo que
  no se tiene.
- **Quien adapte el almacén mapea su testigo a un ordinal.** Un `snapshot-id` de Iceberg o un
  LSN ya lo son; una fecha ISO tendría que convertirse, y esa conversión es suya.
- **No hay transacciones entre claves.** Cada clave avanza con su marca. Coordinar dos es un
  problema que no vamos a resolver mejor que nadie, y `02-function` §2 ya retiró el campo que lo
  prometía.
- **La ejecución sigue delegada.** El contrato está escrito y probado; correrlo sobre una tabla
  de verdad es otra pieza, y no está construida.
