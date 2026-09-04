# Casos · paquetes OOS de verdad, para las pruebas

Vivían en `crates/ore-exec/casos/` y se mudaron aquí al retirar aquel crate:
**no eran suyos**. Son paquetes OOS completos que las pruebas del compilador y
las de fuego usan como terreno real, y no dependen de quién los lea.

| | qué afirma | quién lo usa |
|---|---|---|
| `dos-familias` | dos entidades sin relación entre ellas, cada una con su fuente | `informe.rs`, `migracion.rs`, `fuentes-reales.sh` |
| `con-vista` | el paradigma de vistas, con `backedBy` | `migracion.rs` |
| `jerarquia` | una relación reflexiva —un jefe es un empleado— sobre PostgreSQL | `fuentes-reales.sh` |

Se quedaron atrás tres —`con-tabla`, `redactado`, `sin-capacidades`— porque solo
los ejercía `ore-exec` y con él se fueron. Lo que afirmaban —redacción por Cedar
y negativa por capacidades— no lo comprueba nada hoy, y decirlo es mejor que
dejar el hueco sin nombre.
