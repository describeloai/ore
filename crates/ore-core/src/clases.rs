//! **Las clases de repositorio** (0036): la tabla del producto.
//!
//! Un repositorio guarda en su manifiesto **una clave** (`plantilla`) y **una
//! versión** (`plantillaVersion`), y nada más. Qué significa esa clave —con qué
//! ficheros nace, qué puede hacer su sesión, qué enseña la consola— vive
//! **aquí**, en el producto, y se versiona con él. Es lo que Foundry hace con
//! los tipos de repositorio y sus PRs de actualización.
//!
//! # Por qué la tabla no está en el árbol
//!
//! Porque si estuviera, cada cliente tendría una versión distinta del producto
//! y «actualizar la plantilla» no querría decir nada. El árbol guarda **qué
//! clase soy**; el producto sabe **qué es esa clase hoy**.
//!
//! # Una plantilla es UN ÁRBOL DE FICHEROS (⑧a)
//!
//! Medido antes de escribirlo (`medida-la-plantilla.py`): las cinco clases
//! nacían con **dos ficheros y CERO líneas de código** —el manifiesto y un
//! comentario que describía lo que habría que escribir—, y ninguna sembraba
//! `pyproject.toml`, así que toda instancia heredaba la capa de la celda: el
//! motor de la capa por repositorio (③) estaba hecho y **la plantilla no lo
//! usaba**. Una plantilla es, desde aquí, lo que una instancia necesita para
//! ser útil el primer minuto: la disposición, un ejemplo que **corre** y **el
//! fichero donde se declara su entorno**.
//!
//! # El lenguaje es una CLASE, no un campo
//!
//! `transforms-python` y `transforms-java` son dos plantillas con **dos
//! versiones**: con una sola clase, el día que cambie una, o se suben las dos
//! o se miente en la columna «UPGRADE». `familia` y `lenguaje` están para
//! AGRUPARLAS en la consola; quien identifica es `id`, como siempre.
//!
//! # Lo que una clase NO puede
//!
//! **Conceder.** Una clase ajusta hacia abajo —«este repositorio no escribe
//! datasets»— y nunca hacia arriba: lo que gobierna sigue siendo la etiqueta
//! (el conducto), la declaración (`@transform`) y quién escribió. El techo se
//! aplica donde ya se aplica el gobierno, no aquí.

/// Una clase de repositorio, tal como el producto la trae hoy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clase {
    /// La clave que el manifiesto guarda. **Es la identidad**, y lleva el
    /// lenguaje dentro (`transforms-python`).
    pub id: &'static str,
    /// Qué clase de trabajo es, sin el lenguaje: para agrupar en la consola.
    /// No identifica nada — dos familias iguales son dos plantillas distintas.
    pub familia: &'static str,
    /// En qué se escribe. `""` cuando no hay código (`semantics` edita
    /// documentos del árbol). Tampoco identifica: agrupa.
    pub lenguaje: &'static str,
    /// Cómo se llama en la consola.
    pub titulo: &'static str,
    /// Qué se hace aquí, en una frase. La consola la enseña en su tarjeta: la
    /// tenía escrita a mano, y así había dos descripciones de lo mismo.
    pub descripcion: &'static str,
    /// La versión de la plantilla en **este** producto. Sube cuando la semilla
    /// o lo que la clase promete cambian.
    pub version: i64,
    /// Con qué ficheros nace, relativos a la carpeta del repositorio. El
    /// manifiesto no está aquí: lo escribe quien crea.
    pub semilla: &'static [(&'static str, &'static str)],
    /// **El techo** (0036 ⑤): ¿lo que corre aquí puede **escribir datos**?
    ///
    /// `false` no concede nada nuevo a nadie: **quita**. Un `analytics` no
    /// escribe aunque su código lo declare, y eso se aplica donde ya se aplica
    /// el gobierno —el catálogo y el `confirmar`—, no en el SDK.
    pub escribe: bool,
    /// ¿Hay sesión que abrir? `semantics` edita documentos del árbol: no
    /// ejecuta, y pedirle un puesto es un 422 que lo dice.
    pub ejecuta: bool,
    /// El perfil de máquina que pide la clase (0027). `None`: el de hoy.
    /// Todavía no lo usa nadie — lo dice la ADR, no el código.
    pub perfil: Option<&'static str>,
    /// La prosa con que nace su manifiesto (`README.md`), con los huecos de la
    /// semilla: la guía de quien lo abre. `None`: la frase de siempre. No es
    /// semilla porque el manifiesto no lo es —lo escribe quien crea, y su
    /// prosa es de quien la edite: actualizar no la pisa—.
    pub guia: Option<&'static str>,
}

/// El ejemplo de `transforms-python`: **código, no un comentario**. Corre tal
/// cual en cuanto las dos referencias apuntan a algo (como el
/// `SOURCE_DATASET_PATH` de Foundry), y es el mismo transform que la prueba de
/// fuego ejercita contra agentes de verdad (`el-puesto.sh` 10 y 11).
const TRANSFORMS_PY: &str = "\
# A transform DECLARES what it reads and what it writes, and the server
# enforces it (ADR 0031 · W3.7): while it runs, the session only resolves its
# `inputs` and only writes its `output`. Anything else is a PermissionError.
#
# The session already provides `transform`, `over` and `write`; the import
# changes nothing at run time, it lets your editor know them (ADR 0037 ③a).
#
# Names have three parts, `database.schema.name` (ADR 0038): in the `default`
# schema, `database.name` is enough. Replace both references with yours and
# click Run.

from ore import transform, over, write

INPUT = \"my_db.my_schema.my_dataset\"
OUTPUT = \"my_db.my_schema.my_summary\"


@transform(inputs=[INPUT], output=OUTPUT)
def summarize():
    table = over(INPUT, format=\"arrow\")
    return write(OUTPUT, table)


written = summarize()
print(\"rows\", written[\"rows\"])
";

/// El `pyproject.toml` de una instancia de Python: **el sitio donde declarar**.
///
/// ⭐ Nace VACÍO a propósito. Lo que hacía falta no era una dependencia: era el
///   fichero — sin él, un repositorio no puede declarar nada y se come la capa
///   de la celda (0036 ③). Sembrar `polars` «por si acaso» costaría construir
///   una capa para algo que el ejemplo no usa.
const PYPROJECT_PY: &str = "\
# The dependencies of THIS repository (ADR 0036 ③). What you declare here is
# its own: it resolves into its own layer, and neighbouring repositories don't
# load it. Whatever the package or the tree root declares, everyone still gets.
#
# It starts empty: declaring something nobody uses would build a layer for
# nothing. Uncomment the line below, save and reopen the repository — the layer
# resolves itself and the session starts with it.

[project]
name = \"repository\"
version = \"0.1.0\"
dependencies = []
# dependencies = [\"polars\"]
";

/// El `pom.xml` de una instancia de Java: **el sitio donde declarar** (0037 ③c).
///
/// ⭐ Nace VACÍO, por lo mismo que el de Python: lo que faltaba no era una
///   biblioteca, era el fichero. Hasta ③c un repositorio de Java no podía usar
///   ni una, porque la capa (0036 ③) leía sólo `pyproject.toml`.
///
/// ⛔ Y dice lo que NO hace: de aquí se lee `<dependencies>` y nada más. Un pom
///   trae `<build>`, `<plugins>`, `<profiles>`; no se honran, y por eso se
///   avisa en el propio fichero en vez de dejar que alguien lo descubra cuando
///   su plugin no corra.
const POM_JVM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!--
  The dependencies of THIS repository (ADR 0036 iii, 0037 iii.c). What you
  declare here is its own: it resolves into its own layer, and neighbouring
  repositories don't load it. Whatever the package or the tree root declares,
  everyone still gets.

  It starts empty: declaring something nobody uses would build a layer for
  nothing. Uncomment the example below, save and reopen the repository - the
  layer resolves itself and the session starts with it.

  ONLY "dependencies" IS READ FROM THIS FILE. "dependencyManagement", "build",
  "plugins" and "profiles" are not honoured, and nothing pretends they are.
  Every dependency needs its "version" - without one nothing can pin it - and
  "test" scope never reaches the session: the layer is what it takes to RUN.

  AND THE IMAGE WINS: Arrow, Jackson, slf4j and the rest of the SDK come with
  the session. If you declare another version of any of them, the session's
  version is used and the layer report tells you so.
-->
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>repository</groupId>
  <artifactId>repository</artifactId>
  <version>0.1.0</version>

  <dependencies>
    <!-- <dependency>
      <groupId>org.apache.commons</groupId>
      <artifactId>commons-lang3</artifactId>
      <version>3.17.0</version>
    </dependency> -->
  </dependencies>
</project>
"#;

/// El mismo transform, en Java, y **como un fichero Java de verdad** (0037 ③b):
/// un import estático y una clase con `main`.
///
/// ⭐ No es un capricho de estilo: era un SNIPPET DE JSHELL —`var` y llamadas
///   sueltas en el tope— y ningún editor entiende eso. Medido con jdtls sobre
///   esta misma semilla: cuatro errores de sintaxis sobre nuestra propia
///   plantilla. Como unidad de compilación, cero.
///
/// ⛔ Y el ejecutor no cambia: el agente ya parte la celda en snippets y, si
///   declara una clase con `static void main(`, la llama — lo dice su propio
///   comentario desde W3.4. Medido con un JShell 21 de verdad: esto corre e
///   imprime, con el `public` y el import estático dentro.
const TRANSFORMS_JAVA: &str = "\
// A transform DECLARES what it reads and what it writes, and the server
// enforces it (ADR 0031 · W3.7): while it runs, the session only resolves its
// `inputs` and only writes its `output`.
//
// `transform`, `over` and `write` come from the SDK (`ore.Ore`) and the session
// already provides them: the static import changes nothing at run time, it lets
// your editor know them (ADR 0037 ③b).
//
// The file name IS the name of the public class: rename one, rename the other.
//
// Names have three parts, `database.schema.name` (ADR 0038): in the `default`
// schema, `database.name` is enough. Replace both references with yours and
// click Run.
import static ore.Ore.*;

import java.util.List;
import java.util.Map;

public class Example {
    static final String INPUT = \"my_db.my_schema.my_dataset\";
    static final String OUTPUT = \"my_db.my_schema.my_summary\";

    public static void main(String[] args) throws Exception {
        Map<String, Object> written = transform(\"summarize\", List.of(INPUT), OUTPUT,
                () -> write(OUTPUT, over(INPUT)));
        System.out.println(\"rows \" + written.get(\"rows\"));
    }
}
";

/// El transform escrito en SQL: **un `.sql` de verdad** (0038 P7), una sentencia
/// que escribe.
///
/// ⭐ Antes era un `.py` con la consulta en una cadena, y con motivo: una celda
///   SQL a secas leía pero no escribía (0036). Desde el SQL del árbol (0037 y
///   6d451da) un `.sql` es la unidad —en la sesión y como trabajo—, y el que
///   escribe es UNA sentencia `create or replace dataset … as select …`: lo que
///   lee y lo que escribe lo dice la propia sentencia (`ore sql`), sin
///   `@transform` que lo repita. Medido en `medida-la-semilla-sql.sh`.
const TRANSFORMS_SQL: &str = "\
-- A transform written in SQL: ONE statement that writes. The statement itself
-- says what it reads and what it writes, and the server enforces it.
--
-- What gets written is a dataset: `CREATE OR REPLACE DATASET … AS SELECT`
-- overwrites; `INSERT INTO … SELECT` appends; `INSERT OR REPLACE INTO … SELECT`
-- upserts. A plain `SELECT` reads and writes nothing.
--
-- Names have three parts, `database.schema.name` (ADR 0038): in the `default`
-- schema, `database.name` is enough. Replace both with yours and click Run.

CREATE OR REPLACE DATASET my_db.my_schema.my_summary AS
SELECT country, count(*) AS n
FROM my_db.my_schema.my_dataset
GROUP BY country
";

const ANALYTICS_PY: &str = "\
# An analysis READS and declares nothing. Its class enforces that: an
# `analytics` repository writes no data even if the code asks to (ADR 0036 ⑤).
# Here you explore, count and decide what to do next.
#
# The session already provides `over` and `sql`; the import changes nothing at
# run time, it lets your editor know them (ADR 0037 ③a).

from ore import over, sql

# Names have three parts, `database.schema.name` (ADR 0038): in the `default`
# schema, `database.name` is enough.
SOURCE = \"my_db.my_schema.my_dataset\"

rows = over(SOURCE)
print(SOURCE, \"→\", len(rows), \"rows\")
print(sql(f\"select count(*) as n from {SOURCE}\"))
";

const MODELS_PY: &str = "\
# Training a model. Declare the result as a `TrainedModel` with its
# `trainedFrom`: that is what keeps the lineage unbroken (ADR 0029).
#
# The session already provides `over` and `declare`; the import changes nothing
# at run time, it lets your editor know them (ADR 0037 ③a).

from ore import over, declare

# Names have three parts, `database.schema.name` (ADR 0038): in the `default`
# schema, `database.name` is enough.
INPUT = \"my_db.my_schema.my_dataset\"
MODEL = \"my_db.my_schema.my_model\"
DATABASE, SCHEMA, NAME = MODEL.split(\".\")

rows = over(INPUT)
# ... train with whatever you declare in this repository's pyproject.toml ...
digest = \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"

print(declare({
    \"kind\": \"TrainedModel\",
    \"metadata\": {\"name\": NAME, \"namespace\": DATABASE, \"schema\": SCHEMA},
    \"spec\": {\"owner\": \"team:changeme\", \"trainedFrom\": [INPUT], \"digest\": digest},
}))
";

/// La semilla de `functions-python`: **solo el código** (0050 G1, G2, G4).
///
/// v8: un ejemplo que enseña lo que el contrato cumple (G3) —`Decimal` para el
/// dinero, `date`, opcionales y una salida con un campo que puede faltar— y
/// cómo se usa: Run en la sesión corre su bloque `if __name__ == "__main__"`
/// (la sesión ejecuta el fichero como `__main__`; el arnés, como
/// `ore_funcion`, así que al invocarla no corre), desde código con
/// `ore.get_function(...)` y desde Pipelines. Nombres en inglés, como la consola.
///
/// `@function` y las anotaciones del `def` son el contrato, y el documento
/// `Function` se deriva de ellos (OOS v1alpha18 01 §4): no se siembra, lo
/// escribe el commit que crea el repositorio, en `functions/` del paquete
/// —Assets → Functions—, como con cualquier `@function` que se guarde. Nace
/// **sin `over`**, sobre sus parámetros, para que compile e invoque en cuanto
/// el repositorio existe. `{{paquete}}`, `{{carpeta}}` y `{{funcion}}` los pone
/// [`sembrar`]: el `def` se llama como la función, para que dos repositorios
/// del mismo paquete no choquen.
const FUNCTIONS_PY: &str = "\
# A published FUNCTION: `{{paquete}}.{{funcion}}` (ORE 0050).
#
# You write Python; the platform does the rest. `@function` and the annotations
# of the `def` are its contract (what it takes and what it returns), and the
# docstring is its description. On commit it is published under Assets →
# Functions; ore writes its document, so don't edit that.
#
#   · Run, in your session: runs the `if __name__ == \"__main__\"` block below.
#   · From other code:   `ore.get_function(\"{{paquete}}.{{funcion}}\")`, then call it.
#   · From Pipelines:    the Function operator, with its parameters.
#
# Types are enforced: \"2026-09-15\" arrives as a `date` and 120.50 as an exact
# `Decimal`; returning another type is an error that points to its line.
from dataclasses import dataclass
from datetime import date
from decimal import Decimal

from ore import function


@dataclass
class InvoiceStatus:
    status: str                        # \"paid\", \"current\" or \"overdue\"
    outstanding: Decimal
    days: int                          # until due; negative once overdue
    surcharge: Decimal | None = None   # only when overdue


@function(timeout=\"30s\")
def {{funcion}}(amount: Decimal, due: date, paid: Decimal = Decimal(\"0\"),
                today: date | None = None) -> InvoiceStatus:
    \"\"\"The status of an invoice: what is outstanding, the days until it is due and the surcharge if it is overdue.\"\"\"
    today = today or date.today()
    outstanding = max(amount - paid, Decimal(\"0\"))
    days = (due - today).days
    if outstanding == 0:
        return InvoiceStatus(\"paid\", outstanding, days)
    if days >= 0:
        return InvoiceStatus(\"current\", outstanding, days)
    surcharge = (outstanding * Decimal(\"0.0005\") * -days).quantize(Decimal(\"0.01\"))  # 0.05% per day
    return InvoiceStatus(\"overdue\", outstanding, days, surcharge)


# Over a dataset (one call per row), what it reads and the models it calls are
# declared in the decorator:
#     over=\"my_db.my_schema.invoices\"     the row arrives as the first parameter
#     reads=[\"my_db.my_schema.clients\"]   read it with `ore.over`
#     models=[\"extractor\"]                call it with `ore.model`

if __name__ == \"__main__\":
    print({{funcion}}(Decimal(\"120.50\"), \"2026-09-15\", today=\"2026-10-02\"))
    # InvoiceStatus(status='overdue', outstanding=Decimal('120.50'), days=-17, surcharge=Decimal('1.02'))
";

/// El `pyproject.toml` de `functions-python` (0050 P3, v11): **lo que sus
/// funciones usan, declarado como en TypeScript** —el SDK y las herramientas,
/// a la versión exacta que trae la sesión—. No cuesta nada: `ore` es el SDK de
/// la sesión (`entorno::SDK_PYTHON` en ore-serve) y `pytest` está en
/// `puesto/python/provisto.txt` (un test lo compara), y lo declarado que la
/// sesión trae no pide capa. Dice también lo que NO se honra, en el propio
/// fichero. El de las otras clases de Python (`PYPROJECT_PY`) no cambia.
const PYPROJECT_FUNCTIONS_PY: &str = r##"# What the functions of this repository use (ORE 0050). It is this repository's
# own: it resolves into its own layer, which its neighbours don't load.
#
#   [project].dependencies    what your functions run with, as PEP 508
#                             ("polars>=1.30", "requests"); `ore` is the SDK
#   [dependency-groups].dev   what you only need to test and type-check
#                             (pytest, hypothesis, stubs): never reaches a call
#
# Both already declare what the session brings, at its exact versions: they
# install nothing. Add a package and commit: the platform resolves it (about a
# minute), with the session's packages as constraints, and writes pylock.toml
# next to this file (don't edit that one).
#
# Only these two lists are read: optional-dependencies, other groups, [tool.uv]
# and local paths or git URLs are not. Packages install from wheels only.

[project]
name = "{{carpeta}}"
version = "0.1.0"
requires-python = ">=3.14"
dependencies = [
  "ore==1.0.0",
]

[dependency-groups]
dev = [
  "pytest==9.1.1",
]
"##;

/// Sus pruebas (P3): pytest, que trae la sesión. Un `test_*.py` nunca es una
/// función (no lleva `@function`). Se importa el ejemplo por su módulo: pytest
/// pone en el `sys.path` la carpeta de la prueba (`functions/`, sin
/// `__init__.py`). Medido con pytest 9 sobre la semilla sembrada
/// (`puesto/python/pruebas/test_semilla.py`, «nace en verde»).
const TEST_PY: &str = r##"# Tests for `{{funcion}}`, with pytest (the session brings it). Every
# `test_*.py` of the repository is a test file, and it is never published as a
# function.
#
# Call the function with the types of its contract, as the platform delivers
# them: Decimal for money, date for dates.
from datetime import date
from decimal import Decimal

import pytest

from example import {{funcion}}

TODAY = date(2026, 10, 2)


def test_an_overdue_invoice_carries_a_surcharge_of_005_percent_per_day():
    r = {{funcion}}(Decimal("120.50"), date(2026, 9, 15), today=TODAY)
    assert (r.status, r.outstanding, r.days, r.surcharge) == ("overdue", Decimal("120.50"), -17, Decimal("1.02"))


def test_a_paid_invoice_has_nothing_outstanding():
    r = {{funcion}}(Decimal("120.50"), date(2026, 10, 15), paid=Decimal("120.50"), today=TODAY)
    assert (r.status, r.outstanding, r.days, r.surcharge) == ("paid", Decimal("0"), 13, None)


def test_a_partial_payment_leaves_the_rest_outstanding():
    r = {{funcion}}(Decimal("120.50"), date(2026, 10, 15), paid=Decimal("20.25"), today=TODAY)
    assert (r.status, r.outstanding) == ("current", Decimal("100.25"))


@pytest.mark.parametrize("amount, surcharge", [("10.00", "0.00"), ("30.00", "0.02")])
def test_the_surcharge_rounds_half_to_even_to_the_cent(amount, surcharge):
    # One day late: 0.005 rounds to 0.00, and 0.015 to 0.02.
    r = {{funcion}}(Decimal(amount), date(2026, 10, 1), today=TODAY)
    assert r.surcharge == Decimal(surcharge)
"##;

const GITIGNORE_PY: &str = r##"# Packages are resolved by the platform from pyproject.toml; never committed.
.venv/
# Python and tooling caches.
__pycache__/
*.py[cod]
.pytest_cache/
.mypy_cache/
.ruff_cache/
.DS_Store
"##;

/// La guía del repositorio de Python (P3): la prosa de su manifiesto, como
/// `GUIA_TS`. Lo que dice del contrato es lo que `ore.contrato` y
/// `ore.tipos` hacen cumplir.
const GUIA_PY: &str = r##"# Python functions

Typed functions over your data, written in plain Python and published by the
platform. You write a function and its annotations; the platform derives its
contract, checks every call against it, and runs it on Python 3.14.

## What is here

```
functions/
  example.py         a function: `{{funcion}}`
  test_example.py    its tests, with pytest (never published)
pyproject.toml       the packages your functions use
pylock.toml          the exact versions installed (written by the platform)
```

The document of each function (`functions/<name>.yaml` in the package) is
written by the platform on commit. Edit the code, not the document.

## A function

A function is a `def` decorated with `@function` from `ore`. Its annotations
are its input and output, its docstring is its description, and it is named
like the `def`.

```python
from ore import function


@function
def letter_name(first: str, last: str, title: str | None = None) -> str:
    """A person's name as it is printed on a letter."""
    return " ".join(p for p in (title, first, last.upper()) if p)
```

- **Optional parameters** have a default value; `X | None` accepts nulls.
- **Return a `@dataclass`** to return several fields, as `InvoiceStatus` does;
  a field with a default may be missing.
- **Helpers** are plain functions and modules: only `@function` publishes.

## Types

The annotations are the contract, and it is enforced at the boundary: a call
with a wrong value fails before your code runs, and so does a wrong result.

| In Python | What arrives | Example |
|---|---|---|
| `str`, `bool`, `int`, `float` | as is | `"ES"`, `True`, `42`, `0.75` |
| `Decimal` | an exact decimal, never a float | `Decimal("120.50")` |
| `Annotated[Decimal, Precision(p, s)]` | a `Decimal<p, s>` | |
| `Money["EUR", 2]`, `Quantity["km", 1]` | a `Decimal`; the unit is in the type | `Decimal("9.99")` |
| `date` | a calendar date | `date(2026, 9, 15)` |
| `DateTimeTz` | an instant with its zone | |
| `Media["db.schema.collection"]` | a reference to a media item | |
| `list[T]`, `T \| None` | a list, a nullable value | |

`Precision`, `Money`, `Quantity`, `DateTimeTz` and `Media` come from `ore.tipos`.

## Configuration

In the decorator, in literal values only (it is read without running the file):

```python
@function(
    timeout="30s",
    over="my_db.my_schema.invoices",     # one call per row: the row is the first parameter
    reads=["my_db.my_schema.clients"],   # what else it reads, with `ore.over`
    models=["extractor"],                # the registered models it calls, with `ore.model`
)
```

## Try, test, publish

- **Run**, in your session: runs the file as `__main__`, so its
  `if __name__ == "__main__":` block is where you try the function. When the
  function is called by the platform, that block does not run.
- **Dry Run** (f(x), next to Results): run the function you are editing with a
  form, before committing.
- **Tests**: `test_*.py` files use pytest, which the session brings. Import the
  function from its module (`from example import {{funcion}}`) and call it
  with the types of its contract.
- **Commit**: the function is published under Assets → Functions with its
  contract. From other code: `ore.get_function("{{paquete}}.{{funcion}}")`;
  from Pipelines, the Function operator.

## Python packages

Declare them in `pyproject.toml`, as PEP 508 requirements:

```toml
[project]
dependencies = ["ore==1.0.0", "polars>=1.30"]

[dependency-groups]
dev = ["pytest==9.1.1", "hypothesis"]
```

The platform resolves them into a layer when you commit (the first time, it
takes a minute) and commits the exact versions it installed as `pylock.toml`,
next to `pyproject.toml`: a standard lock (PEP 751). It is rewritten on every
resolution; don't edit it. Good to know:

- `pyproject.toml` already declares what the session brings: `ore` (this SDK)
  and `pytest`, at the exact versions the session has. They install nothing.
- `dependencies` is what your functions run with. The `dev` group (pytest
  plugins, hypothesis, type stubs) is resolved together with them, in the same
  lock, but apart: your session and your tests have it, a call never does.
- The packages the session brings (pandas, pyarrow, duckdb, numpy and what
  they need; the Libraries panel lists them) are **constraints**: what you
  declare is resolved to work with their versions, and a version that conflicts
  with them is an error that says so. Two versions of the same package can't
  live in one Python.
- Packages install from wheels only: nothing runs code when it installs, so a
  package that only publishes sources is not supported.
- `optional-dependencies`, other dependency groups, `[tool.uv]` and local paths
  or git URLs are not read.
- An open session keeps the libraries it started with: restart it to use new
  ones."##;

/// Si un fichero de la semilla se siembra en `paquete`. Un documento gobernado
/// (`functions/*.yaml`) solo vive en un paquete cuyo nombre puede ser
/// `namespace` (`OOS2030`): uno con guion —el de un proyecto de antes del
/// 2026-10-01, `test-project`, hasta `ore migrate proyectos`— no puede
/// tenerlo, y sembrarlo haría que el commit que crea el repositorio no
/// compilara. Ahí nace el código solo, como en la v4.
pub fn se_siembra(rel: &str, paquete: &str) -> bool {
    !rel.ends_with(".yaml") || crate::pertenencia::puede_ser_namespace(paquete)
}

/// La función de `functions-typescript` (0050 R3 T6): la MISMA que la de
/// Python —`InvoiceStatus`, con decimales exactos, fechas de calendario y un
/// opcional—, con la marca de TypeScript (OOS v1alpha23): la exportación por
/// defecto de un `.ts` de `functions/`, que se llama como el fichero —por eso
/// el nombre va también en la RUTA de la semilla, `{{funcionTs}}`—, su
/// `config` al lado y los tipos de `ore`. Los decimales, como céntimos
/// (`bigint`) y sin biblioteca: lo que el contrato entrega es la cadena con
/// sus cifras. Medido con Node: da lo mismo que el ejemplo de Python
/// (`overdue · 120.50 · -17 · 1.02`).
const FUNCTIONS_TS: &str = r##"// A published FUNCTION: `{{paquete}}.{{funcionTs}}` (ORE 0050).
//
// You write TypeScript; the platform does the rest. The default export of a
// `.ts` under `functions/` is the function, and it is named like its file. Its
// parameter and return types are its contract, and the JSDoc above it is its
// description. On commit it is published under Assets → Functions; ore writes
// its document, so don't edit that.
//
//   · Dry Run (f(x), next to Results): try it with a form, without committing.
//   · Its tests sit next to it, in `{{funcionTs}}.test.ts`.
//   · Once committed, it is invoked with its parameters (see README.md).
//
// Types are enforced at the boundary: a decimal arrives as a string with its
// digits ("120.50", never a float), a calendar date as "2026-09-15", and an
// `Integer` has to be exact. npm packages go in `dependencies` of package.json.
import type { Decimal, Integer, LocalDate } from "ore";

export const config = { timeout: "30s" };

interface InvoiceStatus {
  status: string; // "paid", "current" or "overdue"
  outstanding: Decimal<12, 2>;
  days: Integer; // until due; negative once overdue
  surcharge?: Decimal<12, 2>; // only when overdue
}

/** The status of an invoice: what is outstanding, the days until it is due and the surcharge if it is overdue. */
export default function {{funcionTs}}(
  amount: Decimal<12, 2>,
  due: LocalDate,
  paid: Decimal<12, 2> = "0",
  today?: LocalDate,
): InvoiceStatus {
  const day = today ?? new Date().toISOString().slice(0, 10);
  const days = Math.round((Date.parse(due) - Date.parse(day)) / 86_400_000);
  const left = cents(amount) - cents(paid);
  const outstanding = left > 0n ? left : 0n;
  if (outstanding === 0n) return { status: "paid", outstanding: decimal(outstanding), days };
  if (days >= 0) return { status: "current", outstanding: decimal(outstanding), days };
  // 0.05% per day, rounded to the cent (half to even, like Python's Decimal).
  const tenThousandths = outstanding * 5n * BigInt(-days);
  let surcharge = tenThousandths / 10_000n;
  const rest = tenThousandths % 10_000n;
  if (rest > 5_000n || (rest === 5_000n && surcharge % 2n === 1n)) surcharge += 1n;
  return { status: "overdue", outstanding: decimal(outstanding), days, surcharge: decimal(surcharge) };
}

// Exact money without a library: decimals as whole cents (`bigint`).
function cents(d: string): bigint {
  const negative = d.startsWith("-");
  const [whole, fraction = ""] = d.replace(/^[-+]/, "").split(".");
  const c = BigInt(whole || "0") * 100n + BigInt((fraction + "00").slice(0, 2));
  return negative ? -c : c;
}

function decimal(c: bigint): string {
  const a = c < 0n ? -c : c;
  return `${c < 0n ? "-" : ""}${a / 100n}.${String(a % 100n).padStart(2, "0")}`;
}

// Over a dataset (one call per row), what it reads and the models it calls are
// declared in `config`:
//     over: "my_db.my_schema.invoices"     the row arrives as the first parameter
//     reads: ["my_db.my_schema.clients"]   read it with `over` from "ore"
"##;

/// Dónde declara un repositorio de TypeScript sus paquetes de npm (0050 R3
/// T5b): `dependencies` y —L3·1— `devDependencies`, y nada más se honra.
///
/// ⭐ L6·1b (v5): nace declarando su SDK y sus herramientas, con la versión
///   exacta que trae la sesión —como un repositorio de Foundry declara su
///   `functions-api`—: el repositorio dice con qué trabaja. No cuesta nada:
///   todo eso lo trae la imagen (`puesto/node/provisto.txt`, un test lo
///   compara), y lo declarado que la imagen trae no pide capa
///   (`entorno::solo_provistas` en ore-serve). Lo que el SDK usa por dentro
///   (`@duckdb/node-api`) y el servidor de lenguaje no se declaran: no son del
///   cliente.
const PACKAGE_JSON_TS: &str = r##"{
  "private": true,
  "type": "module",
  "scripts": {
    "test": "node --test"
  },
  "dependencies": {
    "ore": "1.0.0"
  },
  "devDependencies": {
    "@types/node": "24.19.1",
    "typescript": "5.9.3"
  }
}
"##;

/// Sus pruebas (L1): el corredor de Node (`node:test`), sin nada que instalar.
/// Un `.test.ts` nunca es una función (v1alpha23 `01` §3). Medido con Node:
/// 4/4, y `tsc` limpio en `strict`.
const TEST_TS: &str = r##"// Tests for `{{funcionTs}}`, with Node's own runner (`node:test`): no
// library to install. `node --test` runs every `*.test.ts` of the repository,
// and a test file is never published as a function.
//
// Call the function the way the platform does: decimals and dates arrive as
// strings with their digits, so that is what a test passes.
import { test } from "node:test";
import assert from "node:assert/strict";
import {{funcionTs}} from "./{{funcionTs}}.ts";

test("an overdue invoice carries a surcharge of 0.05% per day", () => {
  assert.deepEqual({{funcionTs}}("120.50", "2026-09-15", "0", "2026-10-02"), {
    status: "overdue",
    outstanding: "120.50",
    days: -17,
    surcharge: "1.02",
  });
});

test("a paid invoice has nothing outstanding", () => {
  assert.deepEqual({{funcionTs}}("120.50", "2026-10-15", "120.50", "2026-10-02"), {
    status: "paid",
    outstanding: "0.00",
    days: 13,
  });
});

test("a partial payment leaves the rest outstanding", () => {
  const r = {{funcionTs}}("120.50", "2026-10-15", "20.25", "2026-10-02");
  assert.equal(r.status, "current");
  assert.equal(r.outstanding, "100.25");
});

test("the surcharge rounds half to even, to the cent", () => {
  // 10.00 one day late is 0.005 → 0.00; 30.00 is 0.015 → 0.02.
  assert.equal({{funcionTs}}("10.00", "2026-10-01", "0", "2026-10-02").surcharge, "0.00");
  assert.equal({{funcionTs}}("30.00", "2026-10-01", "0", "2026-10-02").surcharge, "0.02");
});
"##;

/// Lo que la plataforma corre —Node 24, que borra los tipos— dicho para el
/// editor y `tsc` (L1): `strict`, sin emitir, solo sintaxis borrable, y los
/// `import` con su `.ts`. Medido con `tsc` 5.9: un `enum` es TS1294.
const TSCONFIG_TS: &str = r##"{
  // What the platform runs: Node 24, which executes `.ts` as is by erasing its
  // types. These options keep the editor and `tsc` to that same rule.
  "compilerOptions": {
    "target": "esnext",
    "module": "nodenext",
    "strict": true,
    // Nothing is compiled: `tsc` only checks.
    "noEmit": true,
    // Only TypeScript that erases to JavaScript: no `enum`, `namespace` or
    // parameter properties (use a union of strings for an enum).
    "erasableSyntaxOnly": true,
    "verbatimModuleSyntax": true,
    // Imports name the file as it is, `./helpers.ts`, as Node needs them.
    "allowImportingTsExtensions": true,
    "skipLibCheck": true
  },
  "include": ["**/*.ts"],
  "exclude": ["node_modules"]
}
"##;

const GITIGNORE_TS: &str = r##"# Packages are resolved by the platform from package.json; never committed.
node_modules/
# Local tooling output.
*.tsbuildinfo
.DS_Store
"##;

/// La guía del repositorio (L1): la prosa de su manifiesto. Sus ejemplos,
/// medidos: `tsc` limpio en `strict` y corren con Node.
const GUIA_TS: &str = r##"# TypeScript functions

Typed functions over your data, written in plain TypeScript and published by the
platform. You write a function and its types; the platform derives its contract,
checks every call against it, and runs it on Node 24. There is no build step:
the `.ts` you commit is what runs.

## What is here

```
functions/
  {{funcionTs}}.ts         a function: its file, its name
  {{funcionTs}}.test.ts    its tests (never published)
package.json               the npm packages your functions use
package-lock.json          the exact versions installed (written by the platform)
tsconfig.json              the rules the editor and `tsc` check against
```

The document of each function (`functions/<name>.yaml` in the package) is
written by the platform on commit. Edit the code, not the document.

## A function

A function is the **default export** of a `.ts` file under `functions/`, and it
is named like its file. Its parameter types are its input, its return type is
its output, and the JSDoc right above it is its description.

```ts
// functions/letterName.ts

/** A person's name as it is printed on a letter. */
export default function letterName(first: string, last: string, title?: string): string {
  return [title, first, last.toUpperCase()].filter(Boolean).join(" ");
}
```

- **One file, one function.** Other exports of the file are helpers, not functions.
- **Optional parameters** have a default value, a `?`, or `| null`.
- **Return an object** to return several fields: an `interface` or a `type` of
  the same file becomes the output, field by field, as `InvoiceStatus` does.
- **`async` works:** declare `Promise<T>`, and `T` is the output.
- **Keep a function self-contained for now:** the types of its signature have to
  be declared in its own file.

## Types

The signature is the contract, and it is enforced at the boundary: a call with a
wrong value fails before your code runs, and so does a wrong result.

| In TypeScript | What arrives | Example |
|---|---|---|
| `string`, `boolean` | as is | `"ES"`, `true` |
| `number` | a float | `0.75` |
| `Integer` (from `ore`) or `bigint` | an exact integer | `42` |
| `Decimal<p, s>` | a string with its digits, never a float | `"120.50"` |
| `Money<"EUR", 2>`, `Quantity<"km", 1>` | a decimal string; the unit is in the type | `"9.99"` |
| `LocalDate`, `LocalTime`, `LocalDateTime` | an ISO string | `"2026-09-15"` |
| `Date` | an instant: a `Date` (an ISO string in a call needs its zone) | `"2026-09-15T10:00:00Z"` |
| `Uint8Array` | bytes | |
| `Media<"db.schema.collection">` | a reference to a media item | |
| `T[]`, `T \| null` | a list, a nullable value | |

Decimals are strings so that no digit is lost: do exact arithmetic on them (the
example converts to cents with `bigint`) or use a decimal package from npm.

## Configuration

An optional `config`, next to the function, in literal values only (it is read
without running the file):

```ts
export const config = {
  over: "my_db.my_schema.invoices",     // one call per row: the row is the first parameter
  reads: ["my_db.my_schema.clients"],   // what else it reads, with `over` from "ore"
  timeout: "30s",
};
```

```ts
import { over } from "ore";

/** Whether an invoice belongs to a client at risk. */
export default async function atRisk(invoice: Record<string, unknown>, limit: number = 1000): Promise<boolean> {
  const clients = await over("my_db.my_schema.clients");
  const client = clients.find((c) => c.id === invoice.client_id);
  return Number(invoice.amount) > limit && client?.segment === "new";
}
```

Calling registered models from TypeScript (`models`) is not available yet; it is
in Python functions.

## Try, test, publish

- **Dry Run** (f(x), next to Results): run the function you are editing with a
  form, before committing. It shows the result, or the error with its line.
- **Tests**: `*.test.ts` files use Node's own runner (`node:test`); `node --test`
  runs them all. Call the function the way the platform does, with decimals and
  dates as strings.
- **Commit**: the function is published under Assets → Functions with its
  contract, and invoked with its parameters
  (`POST /funciones/{{paquete}}/<name>/invocar`); the result is stored on your branch.

## npm packages

Declare them in `dependencies` of `package.json`, with versions or ranges as in
npm:

```json
{ "dependencies": { "date-fns": "^4.1.0" } }
```

The platform resolves them into a layer when the session opens (the first time,
it takes a minute) and commits the exact versions it installed as
`package-lock.json`, next to `package.json`: a standard npm lock, so `npm ci`
reproduces it anywhere. It is rewritten on every resolution; don't edit it.
Good to know:

- `package.json` already declares what the session brings: `ore` (this SDK),
  `typescript` and `@types/node` (the Node that runs, 24), at the exact versions
  the session has. They install nothing: a repository that declares only these
  opens its session without a layer. The session's version is always the one
  used; a different one only warns, and the Libraries panel shows both.
- `dependencies` is what your functions run with. `devDependencies` (types such
  as `@types/lodash`) are resolved together with them, in the same lock, but
  apart: they type your code and never reach the runtime.
- Install scripts of packages never run, so a package that compiles native code
  when it installs is not supported. Pure JavaScript packages and those that
  ship prebuilt binaries work.
- `peerDependencies`, `overrides` and local paths (`file:`, `link:`) are not read.

## TypeScript, as Node runs it

Node runs TypeScript by erasing its types, so only syntax that erases is valid:
no `enum` (use a union of strings, `"paid" | "overdue"`), no `namespace`, no
parameter properties in constructors. `tsconfig.json` enforces the same rule in
the editor. Imports name the file as it is: `import x from "./helpers.ts"`."##;

/// Rellena los huecos de una semilla: `{{paquete}}` (el `namespace` de lo que
/// declare), `{{carpeta}}` (dónde vive el repositorio, para su `entrypoint`),
/// `{{funcion}}` (un nombre de función **único en el paquete**, sacado de la
/// carpeta: dos repositorios en el mismo paquete no siembran el mismo) y
/// `{{funcionTs}}`, el mismo en `camelCase` para TypeScript (R3 T6), donde el
/// nombre es el del fichero: se rellena también en la RUTA de la semilla. Una
/// semilla sin huecos sale igual.
pub fn sembrar(contenido: &str, paquete: &str, carpeta: &str) -> String {
    let ultimo = carpeta.rsplit('/').next().unwrap_or(carpeta);
    let mut f: String = ultimo
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !f.starts_with(|c: char| c.is_ascii_alphabetic()) {
        f.insert_str(0, "f_");
    }
    // `funciones-de-riesgo` → `funcionesDeRiesgoInvoiceStatus`.
    let mut ts = String::new();
    for (i, parte) in f.split('_').filter(|p| !p.is_empty()).enumerate() {
        let p = parte.to_ascii_lowercase();
        if i == 0 {
            ts.push_str(&p);
        } else {
            let mut c = p.chars();
            if let Some(x) = c.next() {
                ts.push(x.to_ascii_uppercase());
                ts.push_str(c.as_str());
            }
        }
    }
    contenido
        .replace("{{paquete}}", paquete)
        .replace("{{carpeta}}", carpeta)
        .replace("{{funcionTs}}", &format!("{ts}InvoiceStatus"))
        .replace("{{funcion}}", &format!("{f}_invoice_status"))
}

/// Las cinco clases de hoy. Añadir una es una fila más, y subir su `version`
/// es lo que hace que un repositorio se pueda actualizar.
pub const CLASES: &[Clase] = &[
    Clase {
        id: "transforms-python",
        familia: "transforms",
        lenguaje: "python",
        escribe: true,
        ejecuta: true,
        perfil: None,
        guia: None,
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets using Python.",
        // 2 porque la plantilla CAMBIÓ (⑧a): lo escrito con la de antes —un
        // comentario y sin `pyproject.toml`— sale `actualizable: true`, que es
        // lo que la columna «UPGRADE» existe para decir.
        // 4: la semilla nombra en tres partes (0038 P7).
        // 6: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // Actualizar deja lo de antes (`transforms/ejemplo.py`): es de quien lo tenga.
        version: 6,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("transforms/example.py", TRANSFORMS_PY),
        ],
    },
    Clase {
        id: "transforms-java",
        familia: "transforms",
        lenguaje: "java",
        escribe: true,
        ejecuta: true,
        perfil: None,
        guia: None,
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets using Java on the JVM.",
        // 3 porque la plantilla CAMBIÓ (0037 ③c): lo escrito con la de antes
        // —sin `pom.xml`— sale `actualizable: true`, que es lo que la columna
        // «UPGRADE» existe para decir.
        // 4: la semilla nombra en tres partes (0038 P7).
        // 6: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // Actualizar deja lo de antes (`transforms/Ejemplo.java`): es de quien lo tenga.
        version: 6,
        semilla: &[
            ("pom.xml", POM_JVM),
            ("transforms/Example.java", TRANSFORMS_JAVA),
        ],
    },
    Clase {
        id: "transforms-sql",
        familia: "transforms",
        lenguaje: "sql",
        escribe: true,
        ejecuta: true,
        perfil: None,
        guia: None,
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets writing SQL.",
        // 4: un `.sql` de verdad, en tres partes y SIN `pyproject.toml` (0038 P7):
        // la consulta corre en el entorno de Python, pero no puede usar nada de
        // lo que ese fichero declare — en un repositorio de SQL sólo confundía.
        // Actualizar deja lo de antes (`ejemplo.py`, `pyproject.toml`): es de
        // quien lo tenga.
        // 5: lo que se escribe es un DATASET (`create or replace dataset`): una
        // Table es un puntero a un objeto de un origen y no se crea desde SQL.
        // 6: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // Actualizar deja lo de antes (`transforms/ejemplo.sql`): es de quien lo tenga.
        version: 6,
        semilla: &[("transforms/example.sql", TRANSFORMS_SQL)],
    },
    Clase {
        id: "analytics-python",
        familia: "analytics",
        lenguaje: "python",
        escribe: false,
        ejecuta: true,
        perfil: None,
        guia: None,
        titulo: "Analytics",
        descripcion: "Analyze your datasets using your preferred data science environment.",
        // 4: la semilla nombra en tres partes (0038 P7).
        // 5: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // Actualizar deja lo de antes (`analisis/ejemplo.py`): es de quien lo tenga.
        version: 5,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("analysis/example.py", ANALYTICS_PY),
        ],
    },
    Clase {
        id: "models-python",
        familia: "models",
        lenguaje: "python",
        escribe: true,
        ejecuta: true,
        perfil: None,
        guia: None,
        titulo: "Models",
        descripcion: "Create, test and train models for machine learning, forecasting and more.",
        // 4: la semilla nombra en tres partes (0038 P7).
        // 5: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // Actualizar deja lo de antes (`modelos/entrenar.py`): es de quien lo tenga.
        version: 5,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("models/train.py", MODELS_PY),
        ],
    },
    Clase {
        id: "functions-python",
        familia: "functions",
        lenguaje: "python",
        escribe: false,
        ejecuta: true,
        perfil: None,
        guia: Some(GUIA_PY),
        titulo: "Functions",
        descripcion: "Write typed Python functions over your datasets and models, invocable with parameters.",
        // 4: la semilla nombra en tres partes (0038 P7).
        // 5: nace con una función de verdad, la pareja contrato + código (0050 P5).
        // 6: el código es la fuente, `@function` con anotaciones (0050 G1).
        // 7: sin documento: lo escribe el commit, en el paquete (0050 G2).
        // 8: un ejemplo de verdad: Decimal, date, opcionales y Run (0050 G4).
        // 9: el SDK en inglés (S3).
        // 10: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // Actualizar deja lo de antes (`funciones/ejemplo.py`): es de quien lo tenga.
        // 11: el repositorio entero, como TypeScript (0050 P3): el
        //    `pyproject.toml` declara el SDK (`ore==1.0.0`) y pytest en el grupo
        //    `dev`, con las versiones de la sesión; sus pruebas (pytest),
        //    `.gitignore` y la guía en la prosa del manifiesto. Actualizar
        //    FUSIONA la declaración (`declaracion::fusionar`): añade lo que
        //    falta y no toca lo que ya hay.
        version: 11,
        semilla: &[
            ("pyproject.toml", PYPROJECT_FUNCTIONS_PY),
            (".gitignore", GITIGNORE_PY),
            ("functions/example.py", FUNCTIONS_PY),
            ("functions/test_example.py", TEST_PY),
        ],
    },
    // 0050: la familia `functions` en dos lenguajes, como `transforms` en
    // tres. TypeScript corre en la sesión (`puesto-node`); invocarla como
    // `Function` es R3 (`runtime: node`).
    Clase {
        id: "functions-typescript",
        familia: "functions",
        lenguaje: "typescript",
        escribe: false,
        ejecuta: true,
        perfil: None,
        guia: Some(GUIA_TS),
        titulo: "Functions",
        descripcion: "Write typed TypeScript functions over your datasets, invocable with parameters.",
        // 2: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // 3: una función de verdad (0050 R3 T6): la exportación por defecto de
        //    un `.ts` de `functions/`, con `config` y los tipos de `ore`, y el
        //    `package.json` donde se declaran los paquetes de npm. El commit que
        //    lo crea escribe su documento. Actualizar deja lo de antes
        //    (`functions/example.ts`, un módulo de ayuda): es de quien lo tenga.
        // 4: el repositorio entero (L1): sus pruebas (`node --test`), el
        //    `tsconfig.json` de lo que Node corre, `.gitignore`, y la guía en
        //    la prosa del manifiesto.
        // 5: el `package.json` declara el SDK y las herramientas (`ore`,
        //    `typescript`, `@types/node`) con las versiones de la sesión (L6·1b).
        //    Actualizar lo PROPONE con su diff: quien ya declaró paquetes ve
        //    qué cambiaría antes de fusionar.
        version: 5,
        semilla: &[
            ("package.json", PACKAGE_JSON_TS),
            ("tsconfig.json", TSCONFIG_TS),
            (".gitignore", GITIGNORE_TS),
            ("functions/{{funcionTs}}.ts", FUNCTIONS_TS),
            ("functions/{{funcionTs}}.test.ts", TEST_TS),
        ],
    },
    // `semantics` no siembra código: lo suyo son documentos del árbol, y
    // sembrar una `Entity` a medias sería sembrar algo que no compila.
    Clase {
        id: "semantics",
        familia: "semantics",
        lenguaje: "",
        descripcion: "Define object types, links and actions: the semantic layer over your data.",
        escribe: false,
        ejecuta: false,
        perfil: None,
        guia: None,
        titulo: "Semantics",
        version: 1,
        semilla: &[],
    },
];

/// Lo que se escribió con la clave de antes de ⑧a sigue resolviendo: un árbol
/// de un cliente puede tener `plantilla: transforms` escrito ayer, y no se
/// rompe por haberle puesto el lenguaje al nombre. No es una clase más —no se
/// lista ni se ofrece—: es una clave vieja que apunta a la de hoy.
const ANTIGUAS: &[(&str, &str)] = &[
    ("transforms", "transforms-python"),
    ("analytics", "analytics-python"),
    ("models", "models-python"),
    ("functions", "functions-python"),
];

/// Una familia de plantillas: lo que agrupa a las clases que hacen lo mismo
/// en distintos lenguajes. **No es una clase**: no se crea, no tiene versión y
/// no se guarda en ningún manifiesto — es cómo se dice el grupo.
///
/// ⭐ Vive aquí, y no en la consola, por lo mismo que las clases: la consola
///   tenía las cinco descripciones escritas a mano, y el día que cambiara una
///   habría dos textos para lo mismo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Familia {
    pub id: &'static str,
    pub titulo: &'static str,
    /// Qué se hace en esta familia, **sin nombrar lenguaje**: es la frase de
    /// la tarjeta que agrupa, y detrás hay una plantilla por lenguaje.
    pub descripcion: &'static str,
}

pub const FAMILIAS: &[Familia] = &[
    Familia {
        id: "transforms",
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets using Python, SQL or Java.",
    },
    Familia {
        id: "analytics",
        titulo: "Analytics",
        descripcion: "Analyze your datasets using your preferred data science environment.",
    },
    Familia {
        id: "models",
        titulo: "Models",
        descripcion: "Create, test and train models for machine learning, forecasting and more.",
    },
    Familia {
        id: "functions",
        titulo: "Functions",
        descripcion: "Write reusable code for pipelines, transforms and applications.",
    },
    Familia {
        id: "semantics",
        titulo: "Semantics",
        descripcion: "Define object types, links and actions: the semantic layer over your data.",
    },
];

pub fn familia(id: &str) -> Option<&'static Familia> {
    FAMILIAS.iter().find(|f| f.id == id)
}

pub fn de(id: &str) -> Option<&'static Clase> {
    let id = ANTIGUAS
        .iter()
        .find(|(viejo, _)| *viejo == id)
        .map_or(id, |(_, nuevo)| *nuevo);
    CLASES.iter().find(|c| c.id == id)
}

/// El entorno de puesto que pide una clase por su lenguaje: `python`, `node`
/// o `jvm`. `None` cuando no hay código (`semantics`) o el lenguaje no corre
/// en ninguno — y entonces manda lo que pida quien abre, como antes de ⑧b.
pub fn entorno_de(clase: &Clase) -> Option<&'static str> {
    match clase.lenguaje {
        "python" | "sql" => Some("python"),
        "typescript" | "javascript" | "node" => Some("node"),
        "java" | "jvm" => Some("jvm"),
        _ => None,
    }
}

/// Los nombres, para decirlos en un error.
pub fn nombres() -> String {
    CLASES.iter().map(|c| c.id).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// 0050 P5: un repositorio `functions-python` nace con una función que
    /// COMPILA, en el árbol de verdad —el commit que lo crea pasa por la puerta
    /// de «no empeorar»—. También en un paquete con guion (`test-project`, el
    /// de un proyecto de antes en `victor`), y dos repositorios en el mismo
    /// paquete no chocan.
    /// Lo que el commit escribe de la semilla, byte a byte.
    const DOCUMENTO: &str = "\
# generado por ore desde {{carpeta}}/functions/example.py:{{funcion}} · se edita el def, no este fichero
apiVersion: oos.dev/v1alpha18
kind: Function
metadata:
  name: {{funcion}}
  namespace: {{paquete}}
  description: 'The status of an invoice: what is outstanding, the days until it is due and the surcharge if it is overdue.'
spec:
  runtime: python
  entrypoint: {{carpeta}}/functions/example.py:{{funcion}}
  input:
    amount: { type: Decimal, required: true }
    due: { type: Date, required: true }
    paid: { type: Decimal }
    today: { type: Date }
  output:
    status: { type: String, required: true }
    outstanding: { type: Decimal, required: true }
    days: { type: Integer, required: true }
    surcharge: { type: Decimal }
  limits: { timeout: '30s' }
";

    #[test]
    fn la_semilla_es_lo_que_su_codigo_da() {
        // La semilla es solo código, y se deriva entera: el documento que el
        // commit escribe de ella es este.
        assert!(
            de("functions-python")
                .unwrap()
                .semilla
                .iter()
                .all(|(r, _)| !r.ends_with(".yaml"))
        );
        let (paquete, carpeta) = ("ventas", "funciones-de-riesgo");
        let py = sembrar(FUNCTIONS_PY, paquete, carpeta);
        let d = ore_code::python::derivar(&py, &format!("{carpeta}/functions/example.py"));
        assert!(d.sintaxis.is_empty() && d.version.is_empty(), "{:?}", d);
        let [f] = d.funciones.as_slice() else {
            panic!("una función: {:?}", d.funciones)
        };
        let firma = f.resultado.as_ref().expect("la plantilla se deriva");
        assert_eq!(
            ore_code::emitir::documento(firma, paquete),
            sembrar(DOCUMENTO, paquete, carpeta)
        );
    }

    /// R3 T6: la semilla de TypeScript es una función de verdad —se deriva, y
    /// donde nace, el commit que la crea escribe su documento—, con un nombre
    /// único en el paquete en su fichero.
    /// L6·1b: lo que la semilla v5 declara es exactamente lo que trae la
    /// imagen de Node, a la misma versión. Si la imagen cambia y la semilla
    /// no, aquí falla: un repositorio nuevo nacería avisando.
    #[test]
    fn la_semilla_de_typescript_declara_lo_que_trae_la_sesion() {
        let trae: Vec<(&str, &str)> = include_str!("../../../puesto/node/provisto.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let nv = l.split_whitespace().next()?;
                let i = nv.rfind('@').filter(|i| *i > 0)?;
                Some((&nv[..i], &nv[i + 1..]))
            })
            .collect();
        let n = crate::parse::parse(PACKAGE_JSON_TS).unwrap();
        let mut vistas = 0;
        for campo in ["dependencies", "devDependencies"] {
            let (_, deps) = n.get(campo).unwrap_or_else(|| panic!("{campo}"));
            for (k, v) in deps.entries() {
                let (nombre, v) = (k.as_str().unwrap(), v.as_str().unwrap());
                assert!(
                    trae.contains(&(nombre, v)),
                    "{nombre}@{v} no es lo que trae la sesión: {trae:?}"
                );
                vistas += 1;
            }
        }
        assert_eq!(vistas, 3);
    }

    #[test]
    fn la_semilla_de_typescript_es_una_funcion() {
        let c = de("functions-typescript").unwrap();
        let (paquete, carpeta) = ("ventas", "funciones-de-riesgo");
        assert_eq!(
            sembrar("functions/{{funcionTs}}.ts", paquete, carpeta),
            "functions/funcionesDeRiesgoInvoiceStatus.ts"
        );
        assert_eq!(
            sembrar("{{funcionTs}}", paquete, "a/2024"),
            "f2024InvoiceStatus"
        );
        let ts = sembrar(FUNCTIONS_TS, paquete, carpeta);
        let ruta = format!(
            "{carpeta}/{}",
            sembrar("functions/{{funcionTs}}.ts", paquete, carpeta)
        );
        let d = ore_code::typescript::derivar(&ts, &ruta);
        assert!(d.sintaxis.is_empty() && d.version.is_empty(), "{d:?}");
        let [f] = d.funciones.as_slice() else {
            panic!("una función: {:?}", d.funciones)
        };
        let firma = f.resultado.as_ref().expect("la plantilla se deriva");
        assert_eq!(firma.nombre, "funcionesDeRiesgoInvoiceStatus");
        assert_eq!(firma.timeout.as_deref(), Some("30s"));
        let entrada: Vec<String> = firma
            .entrada
            .iter()
            .map(|c| {
                format!(
                    "{}: {}{}",
                    c.nombre,
                    c.tipo,
                    if c.requerido { "" } else { "?" }
                )
            })
            .collect();
        assert_eq!(
            entrada,
            [
                "amount: Decimal<12, 2>",
                "due: Date",
                "paid: Decimal<12, 2>?",
                "today: Date?"
            ]
        );
        let doc = ore_code::emitir::documento(firma, paquete);
        assert!(doc.contains("runtime: node\n"), "{doc}");
        assert!(
            doc.contains("    surcharge: { type: 'Decimal<12, 2>' }\n"),
            "{doc}"
        );

        // Donde nace, con dos repositorios en el paquete: cada uno su función
        // y su documento, que el commit que los crea escribe.
        let raiz = std::env::temp_dir().join(format!("ore-semilla-ts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        let pkg = raiz.join("packages").join(paquete);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            raiz.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n",
        )
        .unwrap();
        std::fs::write(
            pkg.join("package.yaml"),
            format!(
                "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {paquete}, version: 0.1.0, status: draft, domain: {paquete} }}\nspec: {{ owner: \"team:x\" }}\n"
            ),
        )
        .unwrap();
        for carpeta in ["funciones-de-riesgo", "otra"] {
            for (rel, contenido) in c.semilla {
                let f = pkg.join(carpeta).join(sembrar(rel, paquete, carpeta));
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(f, sembrar(contenido, paquete, carpeta)).unwrap();
            }
        }
        let (p, _) = crate::validate::cargar_paquete(&raiz);
        crate::generar::aplicar(&crate::generar::plan(&p)).unwrap();
        assert!(
            pkg.join("functions/funcionesDeRiesgoInvoiceStatus.yaml")
                .exists()
        );
        assert!(pkg.join("functions/otraInvoiceStatus.yaml").exists());
        // L1: la prueba no es una función, y no tiene documento.
        let prueba =
            pkg.join("funciones-de-riesgo/functions/funcionesDeRiesgoInvoiceStatus.test.ts");
        assert!(prueba.exists());
        let texto = std::fs::read_to_string(&prueba).unwrap();
        assert!(
            texto.contains("import funcionesDeRiesgoInvoiceStatus from \"./funcionesDeRiesgoInvoiceStatus.ts\";"),
            "{texto}"
        );
        assert!(!ore_code::puede_tener_funciones(
            "funciones-de-riesgo/functions/funcionesDeRiesgoInvoiceStatus.test.ts",
            &texto
        ));
        assert!(
            std::fs::read_dir(pkg.join("functions")).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".test"))
        );
        let d = crate::validate::validate_package(&raiz);
        assert!(
            d.is_empty(),
            "{:?}",
            d.iter()
                .map(|x| format!("{} {}", x.code.as_str(), x.message))
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// L1: la guía nace con sus huecos rellenos y nombra lo que la semilla
    /// trae; sólo `functions-typescript` la tiene por ahora.
    #[test]
    fn la_guia_de_python_nombra_lo_que_siembra() {
        let c = de("functions-python").unwrap();
        let guia = sembrar(c.guia.unwrap(), "ventas", "riesgo");
        assert!(!guia.contains("{{"), "{guia}");
        assert!(guia.contains("ventas.riesgo_invoice_status"), "{guia}");
        for (ruta, _) in c.semilla {
            let f = ruta.rsplit('/').next().unwrap();
            if f != ".gitignore" {
                assert!(guia.contains(f), "la guía no nombra `{f}`");
            }
        }
        assert!(guia.contains("pylock.toml") && guia.contains("ore==1.0.0"));
        assert!(crate::sdk::nombres_de_antes_en(&guia).is_empty());
    }

    /// P3: lo que la semilla v11 declara es exactamente lo que trae la sesión
    /// de Python: el SDK con su versión y pytest a la de la imagen.
    #[test]
    fn la_semilla_de_python_declara_lo_que_trae_la_sesion() {
        let sdk = include_str!("../../../puesto/python/ore/__init__.py");
        assert!(sdk.contains("__version__ = \"1.0.0\""));
        assert!(PYPROJECT_FUNCTIONS_PY.contains("\"ore==1.0.0\""));
        let trae = include_str!("../../../puesto/python/provisto.txt");
        let pytest = trae
            .lines()
            .find(|l| l.starts_with("pytest=="))
            .expect("la sesión trae pytest");
        assert!(
            PYPROJECT_FUNCTIONS_PY.contains(&format!("\"{pytest}\"")),
            "{pytest}"
        );
        // Y nace sin huecos ni nombres de antes.
        let t = sembrar(PYPROJECT_FUNCTIONS_PY, "ventas", "riesgo");
        assert!(!t.contains("{{") && t.contains("name = \"riesgo\""));
    }

    #[test]
    fn la_guia_de_typescript_nombra_lo_que_siembra() {
        let c = de("functions-typescript").unwrap();
        let guia = sembrar(c.guia.unwrap(), "ventas", "riesgo");
        assert!(!guia.contains("{{"), "{guia}");
        assert!(guia.contains("riesgoInvoiceStatus.test.ts"), "{guia}");
        assert!(guia.contains("/funciones/ventas/<name>/invocar"), "{guia}");
        for (ruta, _) in c.semilla {
            let f = ruta.rsplit('/').next().unwrap();
            let f = sembrar(f, "ventas", "riesgo");
            if f != ".gitignore" {
                assert!(guia.contains(&f), "la guía no nombra `{f}`");
            }
        }
        assert!(crate::sdk::nombres_de_antes_en(&guia).is_empty());
        assert!(CLASES.iter().filter(|c| c.guia.is_some()).count() == 2);
    }

    #[test]
    fn la_semilla_de_functions_compila_donde_nace() {
        let c = de("functions-python").unwrap();
        for paquete in ["ventas", "test-project"] {
            let raiz =
                std::env::temp_dir().join(format!("ore-semilla-{paquete}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&raiz);
            let pkg = raiz.join("packages").join(paquete);
            std::fs::create_dir_all(&pkg).unwrap();
            std::fs::write(
                raiz.join("ontology.config.yaml"),
                "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n",
            )
            .unwrap();
            std::fs::write(
                pkg.join("package.yaml"),
                format!(
                    "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {paquete}, version: 0.1.0, status: draft, domain: {paquete} }}\nspec: {{ owner: \"team:x\" }}\n"
                ),
            )
            .unwrap();
            for carpeta in ["funciones-de-riesgo", "otra"] {
                for (rel, contenido) in c.semilla.iter().filter(|(r, _)| se_siembra(r, paquete)) {
                    let f = pkg.join(carpeta).join(sembrar(rel, paquete, carpeta));
                    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                    std::fs::write(f, sembrar(contenido, paquete, carpeta)).unwrap();
                }
            }
            // Lo que hace el commit que lo crea (G2): generar lo sembrado.
            let (p, _) = crate::validate::cargar_paquete(&raiz);
            crate::generar::aplicar(&crate::generar::plan(&p)).unwrap();
            let contrato = pkg.join("functions/funciones_de_riesgo_invoice_status.yaml");
            if paquete == "ventas" {
                assert!(pkg.join("functions/otra_invoice_status.yaml").exists());
                // El código vive en `<carpeta>/functions/`; su documento, no.
                assert!(
                    !pkg.join(
                        "funciones-de-riesgo/functions/funciones_de_riesgo_invoice_status.yaml"
                    )
                    .exists()
                );
                let yaml = std::fs::read_to_string(&contrato).unwrap();
                assert!(
                    yaml.contains("name: funciones_de_riesgo_invoice_status"),
                    "{yaml}"
                );
                assert!(
                    yaml.contains(
                        "entrypoint: funciones-de-riesgo/functions/example.py:funciones_de_riesgo_invoice_status"
                    ),
                    "{yaml}"
                );
                assert!(!yaml.contains("{{"), "{yaml}");
            } else {
                // `test-project` no puede ser `namespace`: el código sí, el contrato no.
                assert!(!contrato.exists());
                assert!(
                    pkg.join("funciones-de-riesgo/functions/example.py")
                        .exists()
                );
            }
            let d = crate::validate::validate_package(&raiz);
            assert!(
                d.is_empty(),
                "{paquete}: {:?}",
                d.iter()
                    .map(|x| format!("{} {}", x.code.as_str(), x.message))
                    .collect::<Vec<_>>()
            );
            let _ = std::fs::remove_dir_all(&raiz);
        }
        // Una carpeta que empieza por número sigue dando un nombre válido.
        assert!(sembrar("{{funcion}}", "p", "a/2024").starts_with("f_2024"));
    }

    #[test]
    fn el_techo_quita_y_nunca_concede() {
        // Lo que escribe datos es lo que produce un Dataset o un modelo; leer y
        // publicar una `Function` no escriben.
        assert!(de("transforms-python").unwrap().escribe);
        assert!(de("transforms-java").unwrap().escribe);
        assert!(de("models-python").unwrap().escribe);
        assert!(!de("analytics-python").unwrap().escribe);
        assert!(!de("functions-python").unwrap().escribe);
        // Y lo que no ejecuta es lo que sólo edita documentos.
        assert!(!de("semantics").unwrap().ejecuta);
        assert!(!de("functions-typescript").unwrap().escribe);
        assert!(CLASES.iter().filter(|c| c.ejecuta).count() == 7);
    }

    /// ⭐ Una plantilla trae su entorno y CÓDIGO, no un comentario (⑧a).
    #[test]
    fn la_plantilla_es_un_arbol_de_ficheros() {
        let t = de("transforms-python").unwrap();
        let rutas: Vec<&str> = t.semilla.iter().map(|(r, _)| *r).collect();
        assert!(rutas.contains(&"pyproject.toml"), "{rutas:?}");
        assert!(rutas.contains(&"transforms/example.py"), "{rutas:?}");
        let (_, py) = t
            .semilla
            .iter()
            .find(|(r, _)| *r == "transforms/example.py")
            .unwrap();
        let codigo = py
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count();
        assert!(
            codigo >= 5,
            "el ejemplo tiene que CORRER, no describir: {codigo}"
        );
        assert!(py.contains("@transform("), "{py}");
    }

    /// La clave de antes de ⑧a resuelve, y no se lista como una clase más.
    #[test]
    fn la_clave_vieja_sigue_resolviendo() {
        assert_eq!(de("transforms").unwrap().id, "transforms-python");
        assert_eq!(de("models").unwrap().id, "models-python");
        assert!(!nombres().split(", ").any(|n| n == "transforms"));
        assert!(nombres().split(", ").any(|n| n == "transforms-python"));
    }

    /// ⭐ LO QUE LA SEMILLA USA, LA SEMILLA LO IMPORTA (0037 ③a).
    ///
    /// La sesión pone `over`, `write`, `transform`, `sql` y `declare` en el
    /// espacio de la celda —son literalmente `ore.over`, `ore.write`… (lo hace
    /// `Kernel.__init__` del agente)—, así que durante mucho tiempo la semilla
    /// no importó nada y corría igual.
    ///
    /// ⛔ Pero un fichero que usa nombres que no declara **no lo entiende
    ///   ningún editor**. Medido con pyright sobre esta misma semilla: tres
    ///   errores —«"transform" is not defined», «"over" is not defined»,
    ///   «"write" is not defined»— sobre nuestra propia plantilla, que es el
    ///   primer regalo que recibiría quien la abre. Con el import: cero.
    ///
    /// Y el atajo de enseñarle al editor lo que la sesión inyecta NO existe:
    /// un `builtins.pyi` en el `stubPath` de pyright no añade a typeshed, lo
    /// reemplaza (medido: acto seguido `print` deja de existir).
    /// S3 · Lo que una semilla llama del SDK, por sus nombres de hoy: ninguno de
    /// antes (S5 los retira) en el código de ninguna plantilla, en ningún lenguaje.
    #[test]
    fn la_semilla_habla_el_sdk_en_ingles() {
        for c in CLASES {
            for (ruta, texto) in c.semilla {
                let antes = crate::sdk::nombres_de_antes_en(texto);
                assert!(antes.is_empty(), "{} · {ruta}: {antes:?}", c.id);
            }
        }
    }

    #[test]
    fn la_semilla_importa_lo_que_usa() {
        // Lo que la sesión pone, y que por tanto podría no importarse.
        const PUESTOS: [&str; 5] = ["over", "write", "transform", "sql", "declare"];
        // ⭐ Y en Java lo mismo, con su forma: un import estático de `ore.Ore`
        //   trae los cinco de golpe. Sin él, jdtls y `javac` no los resuelven.
        for c in CLASES {
            if c.lenguaje == "java" {
                for (ruta, texto) in c.semilla {
                    if !ruta.ends_with(".java") {
                        continue;
                    }
                    let usa = PUESTOS.iter().any(|n| texto.contains(&format!("{n}(")));
                    if usa {
                        assert!(
                            texto.contains("import static ore.Ore.*;"),
                            "{} usa el SDK y no lo importa: un editor lo subrayaría en rojo",
                            c.id
                        );
                    }
                    assert!(
                        texto.contains("public class ") && texto.contains("static void main("),
                        "{} no es una unidad de compilación: jdtls y javac no entienden un snippet",
                        c.id
                    );
                    assert!(
                        !texto.contains("package "),
                        "{} declara `package`: JShell no lo acepta y el árbol no tiene carpetas de paquete",
                        c.id
                    );
                }
            }
            if c.lenguaje != "python" {
                continue;
            }
            for (ruta, texto) in c.semilla {
                if !ruta.ends_with(".py") {
                    continue;
                }
                let importado: Vec<&str> = texto
                    .lines()
                    .filter(|l| l.starts_with("from ore import "))
                    .flat_map(|l| l.trim_start_matches("from ore import ").split(", "))
                    .map(str::trim)
                    .collect();
                for nombre in PUESTOS {
                    // Usado como llamada o como decorador, no en la prosa.
                    let usa = texto.contains(&format!("{nombre}("))
                        || texto.contains(&format!("@{nombre}("));
                    if usa {
                        assert!(
                            importado.contains(&nombre),
                            "{} usa `{nombre}` y no lo importa: un editor lo subrayaría en rojo",
                            c.id
                        );
                    }
                }
            }
        }
    }

    /// ⭐ Ninguna plantilla es ya un cartel: todas traen código (⑧b), y las de
    ///   Python traen además el fichero donde se declara su entorno.
    #[test]
    fn ninguna_plantilla_es_un_cartel() {
        for c in CLASES {
            if c.semilla.is_empty() {
                assert_eq!(c.id, "semantics", "sólo `semantics` nace sin ficheros");
                continue;
            }
            let codigo: usize = c
                .semilla
                .iter()
                .filter(|(r, _)| {
                    !r.ends_with(".toml") && !r.ends_with(".xml") && !r.ends_with(".json")
                })
                .flat_map(|(_, t)| t.lines())
                .filter(|l| {
                    let l = l.trim();
                    !l.is_empty()
                        && !l.starts_with('#')
                        && !l.starts_with("//")
                        && !l.starts_with("--")
                })
                .count();
            assert!(codigo >= 4, "{} tiene {codigo} líneas de código", c.id);
            // ⭐ 0037 ③c: y en Java lo mismo, con su fichero. Un repositorio
            //   sin dónde declarar se come la capa de la celda, o —como pasaba
            //   en la JVM hasta ③c— no puede usar ni una biblioteca.
            // `sql` no: una consulta no usa dependencias de Python (0038 P7).
            if let Some(donde) = match c.lenguaje {
                "python" => Some("pyproject.toml"),
                "java" => Some("pom.xml"),
                "typescript" => Some("package.json"),
                _ => None,
            } {
                assert!(
                    c.semilla.iter().any(|(r, _)| *r == donde),
                    "{} no trae `{donde}`: dónde declarar su entorno",
                    c.id
                );
            }
        }
    }

    /// ⭐ (0038 P7) Las semillas nombran en TRES partes, `base.schema.nombre`:
    ///   ni el `<paquete>.<dataset>` de antes —que no es un nombre y no corría—
    ///   ni dos partes, que son `default` y el SQL avisa (`ORE-SQL-2P`).
    #[test]
    fn las_semillas_nombran_en_tres_partes() {
        for c in CLASES {
            for (ruta, texto) in c.semilla {
                // Un documento generado no lleva comentarios: lo que enseña a
                // nombrar está en el código del que sale (0050 G1).
                // Ni lo que no nombra datos: `.gitignore`, una prueba.
                if ruta.ends_with(".toml")
                    || ruta.ends_with(".xml")
                    || ruta.ends_with(".json")
                    || ruta.ends_with(".gitignore")
                    || ruta.ends_with(".test.ts")
                    || ruta
                        .rsplit('/')
                        .next()
                        .is_some_and(|f| f.starts_with("test_"))
                    || ore_code::emitir::es_generado(texto)
                {
                    continue;
                }
                assert!(!texto.contains("<paquete>"), "{} · {ruta}", c.id);
                assert!(
                    texto.contains("my_db.my_schema."),
                    "{} · {ruta} no nombra en tres partes",
                    c.id
                );
            }
        }
        // y la de SQL es un `.sql`, no una cadena dentro de Python
        let sql = de("transforms-sql").unwrap();
        assert!(
            sql.semilla
                .iter()
                .any(|(r, _)| *r == "transforms/example.sql")
        );
        assert!(!sql.semilla.iter().any(|(r, _)| r.ends_with(".py")));
        assert!(!sql.semilla.iter().any(|(r, _)| *r == "pyproject.toml"));
    }

    /// Cada clase pertenece a una familia que existe, y cada familia tiene
    /// al menos una clase: una tarjeta que agrupa nada no se puede pintar.
    #[test]
    fn las_familias_y_las_clases_se_corresponden() {
        for c in CLASES {
            assert!(familia(c.familia).is_some(), "{} sin familia", c.id);
        }
        for f in FAMILIAS {
            assert!(
                CLASES.iter().any(|c| c.familia == f.id),
                "la familia `{}` no tiene ninguna clase",
                f.id
            );
        }
    }

    #[test]
    fn el_entorno_sale_del_lenguaje() {
        assert_eq!(entorno_de(de("transforms-python").unwrap()), Some("python"));
        assert_eq!(entorno_de(de("transforms-java").unwrap()), Some("jvm"));
        // `sql` corre donde corre python: no hay imagen de SQL.
        assert_eq!(entorno_de(de("transforms-sql").unwrap()), Some("python"));
        assert_eq!(entorno_de(de("semantics").unwrap()), None);
        assert_eq!(
            entorno_de(de("functions-typescript").unwrap()),
            Some("node")
        );
    }

    #[test]
    fn las_clases_estan_y_ninguna_siembra_fuera_de_su_carpeta() {
        // 8 desde 0050: `functions-typescript`.
        assert_eq!(CLASES.len(), 8);
        for c in CLASES {
            assert!(de(c.id).is_some());
            for (ruta, _) in c.semilla {
                assert!(!ruta.starts_with('/') && !ruta.contains(".."), "{ruta}");
                assert!(
                    !ruta.eq_ignore_ascii_case("README.md"),
                    "la semilla no pisa el manifiesto"
                );
            }
        }
    }
}
