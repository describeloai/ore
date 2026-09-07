# El paquete

> **Estado:** construido · **Fecha:** 2026-09-07 · **Dónde:** `crates/ore-core/src/pertenencia.rs`,
> `crates/ore-cli/src/paquete.rs` · **Normativo:** [`01-package`](../vendor/oos/spec/v1alpha1/01-package.md)
>
> Este documento es permanente. No explica cómo se usan los mandos —eso lo dice `--help`— sino
> **por qué hacen lo que hacen**, que es lo que el código no puede llevar encima.

---

## 1. Qué es un paquete, y qué no

Un paquete es **la unidad de propiedad y de versión**: quién responde de esto, en qué versión va, y
qué expone a los demás. No es una carpeta, y no es un espacio de nombres — es las dos cosas a la
vez, y que lo sean es lo que hubo que atar.

Lo que **no** es: una frontera de ejecución. Un paquete cruza fuentes por definición —`erp`,
`workday`, un lago— y el límite no separa datos, separa **responsabilidad**.

> **El límite del paquete es una costura real del sustrato, no una convención de carpetas.** Una
> vista no se apoya en la tabla de otro paquete, y una entidad no se respalda de la vista de otro.

Eso se midió sobre el corpus antes de escribir nada, y por eso los verbos de abajo pueden existir:
si el límite fuera decorativo, mover un documento no significaría nada.

## 2. La pertenencia — `OOS2030`

> Un documento que vive **dentro del directorio de un paquete** DEBE declarar como `namespace` el
> nombre de ese paquete.

### Lo que faltaba

La pertenencia la dice **el directorio**: `01-package` §3.3 midió que el manifiesto no liste sus
documentos, porque *«eso ya lo dice el directorio, y redeclararlo sería declarar lo derivable»*. La
identidad la dice **`metadata.namespace`**, que es con lo que `backedBy`, `from.view` y `exports` se
refieren a todo.

Y **nadie ataba las dos**. Un documento en `packages/ventas` llamado `otro.E` validaba limpio; ese
mismo paquete declarando `exports: [ventas.E]` fallaba con `OOS2027`, porque `exports` habla en
nombre cualificado y el documento se llamaba otra cosa. Las dos mitades de la pinza, y ninguna
cerraba.

La consecuencia práctica: *«mover un documento a otro paquete»* no tenía **un** significado. Eran dos
cosas —el fichero y el nombre— que se movían por separado sin que nada protestara. Y de todo lo que
un movimiento puede dejar a medias, casi todo lo cazaba alguien —`OOS5007` al hacer `diff`, `OOS2028`
al compilar, `OOS2018` al validar—: **el `namespace` era el que no cazaba nadie.**

### Dos poblaciones, no una mezcla

La regla mira el `kind`, y la primera versión no lo hacía. La medida encontró que el árbol tiene
**dos poblaciones**:

| población | casan / no casan | por qué |
|---|---|---|
| contenido gobernado — `Entity` `View` `Table` `Function` `Resolution` `Binding` | `Entity` 273/6 · `View` 127/6 · `Function` 25/0 | alguien lo posee, y se mueve entre paquetes |
| vocabulario compartido — `Lattice` `Ruleset` `Concept` `Interface` `ConduitPolicy` `RequestPolicy` | `Lattice` 2/197 · `Ruleset` 0/37 | `gdpr.sensitivity` tiene que significar lo mismo en `hr` y en `crm`, que es justo lo que lo hace útil |
| estructural — `Package` `OntologyConfig` | — | el manifiesto **es** el nombre y no puede discrepar de sí mismo; la config es del *workspace* |

La primera versión intentó separarlas **por ubicación** —el vocabulario compartido cuelga de la raíz
del *workspace*, que es donde `ore init` lo pone— y así se ahorraba la lista. **No vale, y lo dijo la
suite**: en un árbol *plano*, con el `package.yaml` en la raíz, *todo* está dentro del paquete
—incluido `lattices/`— y no hay un «fuera» al que mover un retículo compartido.

Así que la lista hace falta, y por eso **lleva censo**: añadir un `kind` sin decir de qué población
es no compila la suite. Es la misma ley que el registro de códigos, y por la misma razón — una lista
que se puede ampliar sin mirar deja de significar algo.

### Dos detalles que salieron construyendo

**Corre antes del enlazado.** Si un documento está en el espacio equivocado, *todo lo que lo nombra*
falla —`OOS2018`, `OOS2005`— y esos diagnósticos son la **consecuencia**. `99-errors` §2.1 dice que
gana el código específico: adelantar una consecuencia manda a mirar el fichero equivocado.

**Hay nombres de paquete para los que la regla es insatisfacible.** El esquema deja llamar `oos.dev`
o `mi-paquete` a un paquete —`packageName` admite puntos, guiones y barras, porque el nombre **es
también la coordenada con la que otro lo importa**— y un `namespace` es un `identifier`, que no
admite ninguno de los tres. Hay uno así en el corpus. No se arregla aflojando la regla, así que se
dice: un paquete cuyo nombre no sea un identificador **no puede contener contenido gobernado**, y
`ore package new` se niega **antes de crearlo** en vez de dejar un paquete donde no se puede poner
nada.

**Y llega con v1alpha8**, no retroactivamente. Es la misma puerta que `OOS2028`: un documento
anterior se escribió cuando el `namespace` no significaba pertenencia, y aplicárselo cambiaría lo
que significa algo ya publicado.

## 3. Los cuatro verbos

Hasta esta iteración **un paquete solo nacía descubriendo una fuente**: `ore init` deja `packages/`
vacío y el único que escribía un `package.yaml` era el inductor.

| verbo | qué hace | qué no |
|---|---|---|
| `package new <n> --owner <h>` | el manifiesto, y nada más | no crea `views/` ni `tables/`: un directorio vacío no viaja en git |
| `package move <qname> --to <p>` | las cuatro cosas de abajo, o ninguna | no toca `exports` |
| `package split <p> --con … [--to <q>]` | sin `--to`, **enumera componentes y dice el precio**; con `--to`, mueve la clausura | no elige el corte |
| `package merge <o> --into <d>` | funde y deja **lápida** | no resuelve colisiones de nombre |

Los tres últimos son el mismo `planificar` con distinto número de documentos, y el mismo `Taller`.

### `new` es un verbo aparte, y no por comodidad

Crear un paquete es un **acto de gobierno** —necesita dueño, versión y estado— y mover un documento
no lo es. Fallan por separado, que es el mismo motivo por el que `discover` se partió en `--source` y
`--from`.

Escribe **`status: draft`** y no `active`: `01-package` §2.3 deriva de `status` la `oos.maturity`
**por defecto** de lo que el paquete contenga, y uno recién creado no contiene nada. Llamarlo
`active` sería afirmar `STABLE` sobre lo que no existe.

Y **`owner` se pregunta, no se deriva**: es quién responde. Sin él se escribe `cambiame`, que **no
valida** — un handle inventado dejaría el paquete sin nadie que responda aparentando lo contrario.

No escribe YAML: llama a `inductor::documento_paquete`, el mismo emisor que usa la inducción. Un
manifiesto escrito a mano y uno inducido tienen que ser **el mismo texto**.

### Mover son cuatro cosas, y hay que hacer las cuatro

1. **el fichero**, al mismo subdirectorio del destino — el reparto en carpetas lo decide el árbol
   que ya hay, no el mando;
2. **su `namespace`**, que con `OOS2030` es una sola cosa con lo anterior;
3. **el `moved` en el manifiesto de origen**, sin el cual el nombre que desaparece es un `OOS5007`;
4. **y lo que lo nombraba**, reapuntado *por posición* — incluida la forma corta, porque un documento
   que compartía espacio con él lo llamaba a secas.

Hacer tres de las cuatro deja el árbol peor que antes, y ésa es la forma que se repite en todo lo de
abajo.

### El `Taller`, y por qué un bucle de `move` no basta

`move` es atómico por documento; **un bucle de `move` no lo es**. Si el tercero falla queda medio
partido, y medio partido no es un estado que nadie quiera revisar.

Y hace falta algo más que juntar escrituras: **cada movimiento tiene que ver lo que hicieron los
anteriores**. El manifiesto ya lleva un `moved` cuando se añade el segundo; un documento que se movió
y luego resulta que nombraba a otro que también se mueve hay que reapuntarlo *en su ruta nueva*. Por
eso el taller lleva las dos cosas —el contenido y a dónde fue cada fichero— y **retira lo mudado al
final**: si algo falla antes, no se ha perdido nada.

### `split` dice el precio, no lo busca

La pregunta que decidió el verbo entero: *¿la clausura de un documento es pequeña, o es el paquete
entero?* Se midió sobre dos árboles reales, y la respuesta tiene **dos mitades**:

| árbol | forma | corte |
|---|---|---|
| recién descubierto (`ore discover` de BigQuery) | 30 documentos en **10 componentes de exactamente 3** — `Table` + `View` + `Entity` por objeto | mover una componente entera deja **cero** referencias cruzando |
| modelado (`examples/acme-retail`) | cada uno de los tres paquetes es **una sola componente** — `customers` 2, `hr` 4, `supply` 5 | el corte más barato cuesta **exactamente un cruce**, en los tres |

Sobre el primero, `split` calcula algo que una persona no calcula bien. Sobre el segundo **no hay
corte gratis**, y eso no lo convierte en un mal corte —puede ser justo el límite que se quería
trazar— pero sí cambia lo que el mando tiene que hacer: **decir el precio antes de mover**, no
minimizarlo por su cuenta. Elegir el límite de un dominio no es suyo.

Y la arista se cuenta **sin dirección**, que costó verlo: mover un documento no rompe sólo a quien lo
nombra, también hace cruzar *lo que él nombra*.

### `merge` deja una lápida, y por eso funciona

El razonamiento que casi bloquea este verbo era estructural y sonaba bien: los tres alcances de
`moved` anuncian **dentro de un artefacto que sobrevive**, y un paquete que desaparece se lleva su
manifiesto, así que no habría dónde poner el anuncio. Hacía falta un cuarto alcance.

**No hacía falta.** Un paquete no tiene que desaparecer: se queda como **lápida** —`status: retired`,
cero documentos, y un `moved` por cada uno de los que se fueron— y `moved.to` ya cruza de paquete. Se
midió con tres experimentos y su control:

| cómo se deja el origen | qué dice `ore diff` |
|---|---|
| lápida | `changes: []` · **compatible** · minor |
| vacío y `retired`, **sin** el anuncio | `OOS5007` · **breaking** · major |
| el manifiesto **borrado** | `OOS5007` + `OOS5021` |

El control es la mitad del valor: sin el anuncio duele, así que que la lápida salga limpia no es que
nadie esté mirando. Y el estado tampoco hubo que inventarlo — `01-package` §2.3 adopta el enum de
ODCS *verbatim*, y `retired` es uno de los cinco.

**Las colisiones se niegan.** Si los dos paquetes tienen un documento con el mismo nombre, la fusión
no es mecánica: es una decisión por colisión, y perder un documento no lo decide una herramienta.

## 4. `OOS2031` — depender de una lápida

La lápida le vale a `ore diff`, que compara **dos versiones del mismo paquete**. A quien importe el
paquete **desde fuera** no le decía nada: resolvía, compilaba, y nadie le contaba que lo que
importaba era una piedra con un nombre. `dependencies` sólo miraba duplicados —`OOS2003`— y la forma
de la referencia —`OOS2002`—.

> Depender de un paquete `retired` se dice, y el diagnóstico **nombra los destinos**: la lápida los
> sabe, documento a documento, en su `moved`.

La regla alcanza a lo que está **en el árbol**. Una dependencia de otro artefacto se resuelve por el
lock, y su estado es del registro: decirlo aquí exigiría red, y la compilación dejaría de ser
hermética.

## 5. Lo que quedó fuera, y no es lo mismo que pendiente

**`exports` no lo tocan los verbos.** Es *«esto lo expongo a propósito»*, una frase de gobierno, y un
mando que ensancha la superficie pública por su cuenta contradice la frase para la que la lista
existe. Se dice la línea y no se escribe.

**Cuándo se puede retirar una lápida** es una pregunta de producto, no de código, y su respuesta ya
tiene sitio: `sla.breakingChangePolicy`, el único campo normativo del SLA, que ya dice con cuánta
antelación hay que anunciar un cambio rompedor. Nada de lo construido la necesita para funcionar hoy.

**El corte más barato no se busca**, por lo de §3.

## 6. Las reglas que no hay que deshacer

- **La pertenencia se comprueba por `kind` y con censo.** Añadir un `kind` sin clasificarlo no
  compila. Sin eso, la ausencia significaría dos cosas.
- **`pertenencia::check` corre antes de `link`.** Gana el código específico sobre su consecuencia.
- **Los tres verbos que mueven comparten `planificar` y `Taller`.** Una segunda ruta divergiría en la
  que ninguna prueba ejerce, que es lo que ya le pasó a la topología.
- **Se reapunta por posición**, no por texto: un `sed` sobre el documento tocaría comentarios y
  cadenas que dicen lo mismo sin ser referencias.
- **Va el paquete entero o ninguno.** Si un movimiento falla, nada se ha movido, y el mensaje lo
  dice.
- **`DEL_PAQUETE` es pública y la usan dos.** `OOS2030` decide sobre ella y `move` mueve exactamente
  eso; dos listas con los mismos nombres divergirían.

## 7. Dónde está cada cosa

| | |
|---|---|
| la regla y su censo | `crates/ore-core/src/pertenencia.rs` |
| `OOS2031` | `crates/ore-core/src/link.rs` |
| los cuatro verbos | `crates/ore-cli/src/paquete.rs` |
| pruebas del mando | `crates/ore-cli/tests/paquete.rs` — 16 |
| normativo | `01-package` §3.5 (pertenencia) y §3.6 (la lápida) |
| conformidad | `v1alpha8/invalid/a-document-outside-its-package` |
| el terreno, antes de nada | [`medida-terreno-paquete.py`](../pruebas-de-fuego/medida-terreno-paquete.py) |
| la base que ya había, y lo que faltaba | [`medida-agrupar-vistas-en-paquetes.py`](../pruebas-de-fuego/medida-agrupar-vistas-en-paquetes.py) |
| el corte, y sus dos mitades | [`medida-el-corte-de-un-paquete.py`](../pruebas-de-fuego/medida-el-corte-de-un-paquete.py) |
| la lápida y el cuarto alcance | [`medida-el-cuarto-alcance.py`](../pruebas-de-fuego/medida-el-cuarto-alcance.py) |
| si la iteración está cerrada | [`medida-tres-prioridades.py`](../pruebas-de-fuego/medida-tres-prioridades.py) §A |

> El censo de las dos poblaciones —los `273/6` y `2/197` de §2— se corrió sobre el corpus mientras
> se escribía la regla y **su registro es el comentario de `pertenencia.rs`**, no un script. Se dice
> aquí para que nadie lo busque en `pruebas-de-fuego/` y concluya que no se midió.
