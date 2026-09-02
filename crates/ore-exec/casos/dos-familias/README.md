# `dos-familias` — el caso que v1alpha8 **no sabe escribir**

Este caso se queda en v1alpha1, con sus dos `Binding`, y no es que falte
migrarlo: **no hay a qué migrarlo**.

`hr.Employee` se sirve desde dos objetos de dos fuentes distintas —una tabla de
PostgreSQL y un fichero NDJSON— y eso el binding lo expresaba sin esfuerzo,
porque una entidad admitía N bindings. Lo que lo sustituye no puede:

- una entidad tiene **un** `backedBy`;
- una vista sale de **un** sitio: `from` es exactamente una de dos formas;
- y el vocabulario **no tiene junta** — `v1alpha8/00-scope` §6 la deja fuera a
  propósito, porque una junta trae dos raíces y su precio en la regla de flujo
  se decide antes de admitir la operación.

El ejecutor **sí** sabe federar: `Motor::fisicas` devuelve una lista, y la fase
③ pide a cada fuente por separado. Lo que falta es la forma de decirlo.

Así que este caso se queda como el **testigo** de ese hueco. Borrarlo para que
el recuento de `kind: Binding` diera cero habría convertido una limitación real
en un número bonito, que es exactamente el modo de fallo que este proyecto
persigue: el que no produce ningún síntoma.

Mientras esté aquí, dos cosas siguen siendo ciertas y comprobadas: que un
documento v1alpha1 sigue compilando, y que el camino del binding en `link`,
`flow`, `selector` y `plan` sigue vivo. El día que v1alpha8 —o la que venga—
sepa decir «esta entidad sale de estos dos objetos», este fichero se migra y
este README se borra.
