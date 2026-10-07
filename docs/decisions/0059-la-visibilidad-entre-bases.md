# 0059 · La visibilidad entre bases

**Estado:** **aceptado** (2026-10-07) · X0 medido · X1 (spec OOS
[v1alpha28 `01-la-visibilidad`](../../vendor/oos/spec/v1alpha28/01-la-visibilidad.md)) · X2
(ore-core) · X3 (creación) hechos · X5 (victor y demo a v1alpha28) pendiente · **Decide:** un árbol
de organización es **un catálogo**, como un *metastore* de Unity: **sus bases se leen por su
nombre**, sin lista de exportación, y **quién lee qué lo decide el acceso** al servir los datos
(0047 A8). `exports` deja de ser la frontera entre bases y queda como lo que era al nacer: **la
superficie pública de un paquete publicado** hacia otro árbol (`dependencies`). Vale para toda base
—standard, foránea y la del catálogo de una fuente— y en las dos direcciones. Corrige la lectura de
v1alpha8 que hacía de cada base un módulo cerrado dentro de su propio árbol.

## Qué es

`exports` (OOS v1alpha8 `01` §3.2) declara lo que un paquete deja usar a **otro**, con defecto
cerrado: ausente es *nada*. `OOS2028` rechaza una referencia de un paquete a lo que otro no exporta.
Se escribió para el paquete como **módulo** —lo que un equipo publica y otro importa, como el
`module-info` de Java— cuando un árbol solía tener un paquete; lo dijo entonces: *«en el corpus
entero ninguna referencia cruza la frontera de un paquete dentro del mismo árbol»*.

Desde 0038 un árbol es el catálogo de una organización, con una base por fuente, por equipo, por
producto. La regla siguió aplicándose entre esas bases, y se encontró el 2026-10-07 al crear una
vista en `sandbox` sobre la colección de `s3_stuff`: `OOS2028`, porque `s3_stuff` —una base que
creó la inducción— no exporta nada.

## Lo medido (X0, 2026-10-07)

Los árboles de `main` de victor (22 bases) y demo (8), compilados tal cual y sin un solo `exports`:

| árbol | errores de partida | sin `exports` | a dónde cruzan las que se rompen |
|---|---|---|---|
| victor | 5 (ajenos: semillas de repositorios de transforms de antes de 0055) | +57 `OOS2028` | **todas** a bases de catálogo de una fuente (`postgresql_…` 38, `bigquery_…` 6, `s3_demo` 3, …) |
| demo | 0 | +11 `OOS2028` | **todas** a fuentes (`postgresql_…` 11) |

1. `exports` **sólo lo escribe la fuente** (0045 P3′), para que el resto pueda leer lo que cataloga
   —6 de 22 bases en victor, 5 de 8 en demo—. Ninguna persona lo escribió.
2. **Ninguna referencia cruza entre dos bases de usuario** (standard↔standard, foránea↔standard).
   No porque no se quiera: porque no compila. La primera vez que alguien lo intentó, `OOS2028`.
3. En el spec, tres casos esperan `OOS2028` (v1alpha8, v1alpha16 y v1alpha27) y 104 llevan
   `exports`, casi todos fuentes S3.

Y lo que protege, al leer, no es esto: hoy cualquier miembro de la organización lee cualquier dato
(0047, deuda A8). Un manifiesto que se comprueba al compilar no impide leer; impide **escribir la
referencia**.

## Lo que hace la industria

- **Unity Catalog:** las bases (*catalogs*) de un *metastore* se nombran entre sí sin lista de
  exportación (`catalogo.schema.tabla`); quién lee qué es un `GRANT` (`USE CATALOG`, `USE SCHEMA`,
  `SELECT`). Lo que sale del *metastore* hacia otro se comparte aparte, con *Delta Sharing*.
- **Snowflake:** las bases de una cuenta se leen por su nombre con privilegios; salir de la cuenta
  es un *share*.
- **Java, Rust, dbt:** visibilidad cerrada por módulo, pero dentro del **artefacto que se publica**;
  es la frontera que `exports` imitó, y la que conserva.

## La decisión

| | antes | ahora (árbol con `OntologyConfig` v1alpha28) |
|---|---|---|
| una base lee otra del mismo árbol | sólo si la otra lo exporta (`OOS2028`) | **compila** |
| quién puede leerla | cualquier miembro (A8 no existe) | cualquier miembro **hasta A8**; con A8, quien tenga el permiso sobre la base, el schema o el objeto |
| dónde se decide | al compilar, por un manifiesto | **al leer**, por el acceso: el préstamo de la credencial, ore-motor, `/federation/read` (el sitio único que 0047 ya nombra, M7) |
| `exports` | visibilidad entre bases | la frontera del artefacto: lo que un paquete publicado deja usar a quien lo importa |
| una base foránea | expone lo que la fuente exporta | expone lo que su `include` alcanza |

**La puerta es el `OntologyConfig`.** El árbol entero cambia de reglas a la vez, como la base
congelada de v1alpha27 (`OOS2051`); un árbol con un config anterior compila igual que antes, y los
`exports` que ya hay no estorban ni conceden.

**Lo que no cambia:** que una referencia exista (`OOS2018`), los conductos (`OOS4011` leer en vivo,
`OOS4002` no rebajar etiquetas), la federación encendida (`OOS2051`), una base foránea sin datos
propios (`OOS2049`).

## Lo que se pierde, y por qué se acepta

Con la regla de antes, para que otra base leyera algo había que **publicarlo a propósito**. Ahora
cualquier base lo nombra. La señal *«esto lo expongo»* deja el manifiesto y pasa al acceso, y
**hasta A8 no la sustituye nada**: cualquiera que pueda escribir en el árbol puede escribir una
referencia a cualquier base de él.

Se acepta porque esa señal no protegía lo que parecía —no impedía leer, sólo nombrar— y porque en
lo medido no la escribía nadie más que la fuente, para abrir lo que cataloga. La seguridad real de
leer es A8, y esta decisión le deja el sitio: un `GRANT` por base, schema u objeto, que valga igual
para una standard que para una foránea (en la foránea, además, `federation.read` y el interruptor
de la fuente).

## El plan

| paso | qué | estado |
|---|---|---|
| X0 | medir: qué sostiene `exports` hoy, en el spec y en victor y demo | ✓ (arriba) |
| X1 | spec OOS v1alpha28: `00-scope`, `01-la-visibilidad`, el schema del `OntologyConfig`, notas en v1alpha1 `01` §3.2 y v1alpha27 `01` §3; tres casos | ✓ (oos `6eba833`) |
| X2 | ore-core: `ApiVersion::V1Alpha28`, `exporta::arbol_es_catalogo` (la puerta), `OOS2028` sólo fuera de un catálogo, la foránea expone sin `exports`. Conformidad v1alpha28 3/3, el resto igual; victor y demo en v1alpha28: 0 `OOS2028` con y sin `exports`, y la vista de `sandbox` compila | ✓ (`8d01db6`) |
| X3 | creación: en un árbol v1alpha28 la fuente deja de escribir `exports` al catalogar (lo que había se queda), y `ore package` deja de aconsejar exportar | ✓ |
| X4 | este ADR | ✓ |
| X5 | victor y demo a v1alpha28: una línea en su `ontology.config.yaml`. Sin migración: ya compilan con y sin `exports` | pendiente del visto bueno |

## Lo que este ADR no decide

- **El acceso a los datos (0047 A8):** el permiso por base, schema u objeto, quién lo concede y
  desde dónde en la consola. Es la otra mitad del modelo de Unity, y su diseño es de 0047.
- **Compartir fuera del árbol** con otra organización sin publicar un paquete (lo que Unity llama
  *Delta Sharing*).
- **Comprobar la frontera del artefacto en el compilador:** una dependencia se resuelve por el lock
  y el registro; el compilador de un árbol no abre los documentos de otro.

## Deuda y pistas

- **A8** es lo que devuelve la señal que esto quita. Hasta entonces, la consola no debería sugerir
  que una base es privada.
- Los `exports` que las fuentes ya escribieron siguen en los árboles. No estorban; se pueden quitar
  cuando un árbol pase a v1alpha28, o dejar.
- Victor no compila hoy por 5 errores ajenos a esto (`OOS2013` ×4 y `OOS2043` en semillas viejas de
  `test_project`): un árbol que no compila esconde los errores nuevos.
