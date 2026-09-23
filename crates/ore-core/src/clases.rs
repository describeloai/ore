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
# Un transform DECLARA qué lee y qué escribe, y el servidor lo hace cumplir
# (ADR 0031 · W3.7): mientras corre, la sesión sólo resuelve sus `inputs` y
# sólo escribe su `output`. Pedir otra cosa es un PermissionError, no un aviso.
#
# `transform`, `over` y `write` los pone la sesión: aquí no se importa nada.
# Cambia las dos referencias por las tuyas y dale a Run.

ENTRADA = \"<paquete>.<dataset>\"
SALIDA = \"<paquete>.<resumen>\"


@transform(inputs=[ENTRADA], output=SALIDA)
def resumir():
    tabla = over(ENTRADA, como=\"arrow\")
    return write(SALIDA, tabla)


escrito = resumir()
print(\"filas\", escrito[\"filas\"])
";

/// El `pyproject.toml` de una instancia de Python: **el sitio donde declarar**.
///
/// ⭐ Nace VACÍO a propósito. Lo que hacía falta no era una dependencia: era el
///   fichero — sin él, un repositorio no puede declarar nada y se come la capa
///   de la celda (0036 ③). Sembrar `polars` «por si acaso» costaría construir
///   una capa para algo que el ejemplo no usa.
const PYPROJECT_PY: &str = "\
# Las dependencias de ESTE repositorio (ADR 0036 ③). Lo que declares aquí es
# SUYO: se resuelve en su propia capa y no la cargan sus vecinos. Lo que está
# en el paquete o en la raíz del árbol lo sigue teniendo todo el mundo.
#
# Nace vacío: declarar algo que nadie usa sería construir una capa para nada.
# Descomenta la línea de abajo, guarda y abre el repositorio — la capa se
# resuelve sola y la sesión nace con ella.

[project]
name = \"transforms\"
version = \"0.1.0\"
dependencies = []
# dependencies = [\"polars\"]
";

const ANALYTICS: &str = "\
# Un análisis: lee, y no declara nada — leer no escribe.
#
#   from ore import datos
#
#   pedidos = datos(\"ventas.pedidos\")
#   print(pedidos.head())
#
# Lo que esta sesión alcanza lo decide el conducto por la etiqueta del dato,
# no esta carpeta.
";

const MODELS: &str = "\
# El entrenamiento de un modelo. Lo que salga se declara como `TrainedModel`
# con su `trainedFrom`, que es lo que hace que el linaje no se corte.
#
#   from ore import datos
#
#   filas = datos(\"ventas.pedidos\")
#   # ... entrenar ...
";

const FUNCTIONS: &str = "\
# Una función de lectura: `over`, `output`, y sin efectos. Se declara como
# `Function` en `functions/` y se invoca con `ore invoke` (ADR 0029).
#
#   def clasificar(fila):
#       return {\"etiqueta\": \"alta\" if fila[\"total\"] > 100 else \"baja\"}
";

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
        // 2 porque la plantilla CAMBIÓ (⑧a): lo escrito con la de antes —un
        // comentario y sin `pyproject.toml`— sale `actualizable: true`, que es
        // lo que la columna «UPGRADE» existe para decir.
        version: 2,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("transforms/ejemplo.py", TRANSFORMS_PY),
        ],
    },
    Clase {
        id: "analytics",
        familia: "analytics",
        lenguaje: "python",
        escribe: false,
        ejecuta: true,
        perfil: None,
        titulo: "Analytics",
        version: 1,
        semilla: &[("analisis/ejemplo.py", ANALYTICS)],
    },
    Clase {
        id: "models",
        familia: "models",
        lenguaje: "python",
        escribe: true,
        ejecuta: true,
        perfil: None,
        titulo: "Models",
        version: 1,
        semilla: &[("modelos/entrenar.py", MODELS)],
    },
    Clase {
        id: "functions",
        familia: "functions",
        lenguaje: "python",
        escribe: false,
        ejecuta: true,
        perfil: None,
        titulo: "Functions",
        version: 1,
        semilla: &[("funciones/ejemplo.py", FUNCTIONS)],
    },
    // `semantics` no siembra código: lo suyo son documentos del árbol, y
    // sembrar una `Entity` a medias sería sembrar algo que no compila.
    Clase {
        id: "semantics",
        familia: "semantics",
        lenguaje: "",
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
const ANTIGUAS: &[(&str, &str)] = &[("transforms", "transforms-python")];

pub fn de(id: &str) -> Option<&'static Clase> {
    let id = ANTIGUAS
        .iter()
        .find(|(viejo, _)| *viejo == id)
        .map_or(id, |(_, nuevo)| *nuevo);
    CLASES.iter().find(|c| c.id == id)
}

/// Los nombres, para decirlos en un error.
pub fn nombres() -> String {
    CLASES.iter().map(|c| c.id).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_techo_quita_y_nunca_concede() {
        // Lo que escribe datos es lo que produce un Dataset o un modelo; leer y
        // publicar una `Function` no escriben.
        assert!(de("transforms-python").unwrap().escribe);
        assert!(de("models").unwrap().escribe);
        assert!(!de("analytics").unwrap().escribe);
        assert!(!de("functions").unwrap().escribe);
        // Y lo que no ejecuta es lo que sólo edita documentos.
        assert!(!de("semantics").unwrap().ejecuta);
        assert!(CLASES.iter().filter(|c| c.ejecuta).count() == 4);
    }

    /// ⭐ Una plantilla trae su entorno y CÓDIGO, no un comentario (⑧a).
    #[test]
    fn la_plantilla_es_un_arbol_de_ficheros() {
        let t = de("transforms-python").unwrap();
        let rutas: Vec<&str> = t.semilla.iter().map(|(r, _)| *r).collect();
        assert!(rutas.contains(&"pyproject.toml"), "{rutas:?}");
        assert!(rutas.contains(&"transforms/ejemplo.py"), "{rutas:?}");
        let (_, py) = t
            .semilla
            .iter()
            .find(|(r, _)| *r == "transforms/ejemplo.py")
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
        assert!(!nombres().split(", ").any(|n| n == "transforms"));
        assert!(nombres().split(", ").any(|n| n == "transforms-python"));
    }

    #[test]
    fn las_cinco_clases_estan_y_ninguna_siembra_fuera_de_su_carpeta() {
        assert_eq!(CLASES.len(), 5);
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
