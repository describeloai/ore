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
const FUNCTIONS_TS: &str = r#"// A published FUNCTION: `{{paquete}}.{{funcionTs}}` (ORE 0050).
//
// You write TypeScript; the platform does the rest. The default export of a
// `.ts` under `functions/` is the function, and it is named like its file. Its
// parameter and return types are its contract, and the JSDoc above it is its
// description. On commit it is published under Assets → Functions; ore writes
// its document, so don't edit that.
//
//   · Dry Run (f(x), next to Results): try it with a form, without committing.
//   · From Pipelines: the Function operator, with its parameters.
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
"#;

/// Dónde declara un repositorio de TypeScript sus paquetes de npm (0050 R3
/// T5b): `dependencies`, y nada más se honra. Vacío: sin capa, y se ve dónde.
const PACKAGE_JSON_TS: &str = r#"{
  "private": true,
  "type": "module",
  "dependencies": {}
}
"#;

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
        version: 10,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("functions/example.py", FUNCTIONS_PY),
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
        titulo: "Functions",
        descripcion: "Write typed TypeScript functions over your datasets, invocable with parameters.",
        // 2: la semilla, en inglés: prosa, identificadores y rutas (SDK S4a).
        // 3: una función de verdad (0050 R3 T6): la exportación por defecto de
        //    un `.ts` de `functions/`, con `config` y los tipos de `ore`, y el
        //    `package.json` donde se declaran los paquetes de npm. El commit que
        //    lo crea escribe su documento. Actualizar deja lo de antes
        //    (`functions/example.ts`, un módulo de ayuda): es de quien lo tenga.
        version: 3,
        semilla: &[
            ("package.json", PACKAGE_JSON_TS),
            ("functions/{{funcionTs}}.ts", FUNCTIONS_TS),
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
                if ruta.ends_with(".toml")
                    || ruta.ends_with(".xml")
                    || ruta.ends_with(".json")
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
