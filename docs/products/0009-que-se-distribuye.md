# 0009 · Qué se distribuye

**Estado:** aceptado · **Fecha:** 2026-08-31 · **Decide:** se publica el compilador; el ejecutor y los lectores se construyen desde la fuente

---

## El problema

`v0.1.0` publica binarios con atestación SLSA, dos compilaciones idénticas y su digest al
lado. Pero la matriz de `release.yml` construye los **default members** del workspace, y el
ejecutor y los lectores están fuera de esa lista. Así que la primera release del proyecto
publica **un binario de cuatro**.

[`0007`](0007-enlazar-el-evaluador-de-cedar.md) dejó dicho que habría tres familias de
binarios y que todas necesitarían la misma procedencia. O se amplía la matriz, o se dice por
qué no.

## Decisión

> **Se publica `ore`. El resto se construye desde el repositorio.**

La matriz cruza cinco plataformas **sin caché de compilación y sin acciones de terceros**,
porque lo que ese flujo protege no es la comodidad: es la procedencia. Meter ahí un binario
que enlaza OpenSSL y una pila TLS añade un conjunto de dependencias nativas **por
plataforma**, y el día que una falle en un runner la presión será relajar la puerta, no
arreglar el binario.

**Una puerta se dimensiona por lo que puede cruzarla sin excepciones.**

Y hay una asimetría que no es de conveniencia. `ore` se ejecuta en la CI **de otros**, sobre
repositorios que no son nuestros, y es el que la tesis `G1` obliga a poder verificar sin
confiar en nadie. El ejecutor se ejecutaba donde ya hay credenciales de la fuente: quien lo
despliega está construyendo infraestructura, no descargando una herramienta.

### No es un reparto de directorios: es una propiedad

*`default-features = false` es una promesa; un crate aparte es una propiedad.* Que el
compilador no arrastre lo que los otros arrastran no depende de este documento — lo sostienen
tres pruebas sobre el cierre de `Cargo.lock`, en
[`dependencias.rs`](../../crates/ore-cli/tests/dependencias.rs):

| Prueba | Qué falla si alguien lo mueve |
|---|---|
| `el_binario_que_se_distribuye_no_sabe_hablar_por_la_red` | una crate de red entra en el cierre de `ore` |
| `el_evaluador_esta_donde_esta_por_algo` | `cedar-policy` entra en el cierre de `ore` |
| `el_driver_esta_donde_esta_por_algo` | `tokio`, `native-tls` u `openssl` entran en el cierre de `ore` |

## Lo que se acepta a cambio

**Quien quiera L2 necesita una cadena de compilación de Rust.** Es un peaje real, y se paga
donde menos duele: quien despliega un ejecutor ya está montando un servicio con credenciales.

**El binario publicado no puede responder una consulta.** `ore --help` anuncia `serve`, así
que alguien lo descargará esperando un motor y encontrará un compilador. Las notas de la
release lo dicen en su propio bloque, y eso es el mínimo — no la solución.

## Cuándo se revisa

El día que exista un instalador, o que un delegado tenga usuarios fuera de este repositorio.
(`ore-exec`, que era el ejemplo cuando esto se escribió, se retiró; los delegados que quedan son
`ore-read-<tipo>`, `ore-maintain` y `ore-store-r2`.)
Entonces la pregunta vuelve a ser la de [`0007`](0007-enlazar-el-evaluador-de-cedar.md): la
misma atestación para las tres familias, o ninguna. Publicar dos con procedencia y una sin
ella sería peor que lo de hoy, porque la firma pasaría a significar dos cosas distintas.
