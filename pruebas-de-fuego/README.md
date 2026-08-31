# Pruebas de fuego

**Enfrentar lo que emitimos a la implementación de referencia ajena, y lo que
planificamos a una fuente de verdad.**

No están en la suite de Rust y no es por comodidad: **necesitan cosas que la suite
no puede tener**. Una necesita Node y el paquete `graphql`; la otra, un PostgreSQL
en marcha. Meterlas dentro convertiría `cargo test` en algo que no se puede
ejecutar sin red, y eso es justo lo que el compilador no es.

Lo que sí tienen que hacer es **correr solas**. Hasta hoy se ejecutaban a mano:

> **Una prueba que no corre tiene exactamente el mismo aspecto que una que pasa.**

## Qué encontró cada una

**`graphql.mjs`** enfrenta el SDL que emitimos a `graphql-js`, la implementación de
referencia. Encontró defectos que llevaban versiones ahí, y ninguno se veía leyendo:
un esquema puede estar *bien escrito* y ser inválido, y solo lo dice quien lo tiene
que consumir.

**`fuentes-reales.sh`** ejecuta contra un PostgreSQL real lo que se midió a mano al
construir L2 — el driver, el índice, el refresco y una consulta que cruza dos
familias de fuente. Cada medición de esas era un `echo` en una terminal; aquí es una
aserción.

Y hay una tercera que **ya no vive aquí**: la de Cedar. `ore-exec/tests/prueba_de_fuego.rs`
es Rust y entra en `cargo test --workspace` como cualquier otra, porque `cedar-policy`
es una dependencia y no un servicio.

## Cómo se ejecutan

```bash
pruebas-de-fuego/graphql.sh          # necesita node
pruebas-de-fuego/fuentes-reales.sh   # necesita docker
```

Las dos las corre `ci.yml` en cada empujón, que es el punto.
