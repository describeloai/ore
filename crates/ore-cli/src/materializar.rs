//! `ore materialize` — **el ciclo**, los seis pasos del
//! [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md).
//!
//! | | qué | quién |
//! |---|---|---|
//! | 1 | compilar: el plan, su digest y el conducto que lo autoriza | `ore` |
//! | 2 | comprobar el flujo | `ore` |
//! | 3 | preguntarle al origen su testigo | *el driver — **sin protocolo todavía*** |
//! | 4 | el recibo: **si está, termina aquí** | `ore-store-r2 buscar` |
//! | 5 | leer, canalizar, sellar y subir | `ore-read-<tipo> leer` → `ore-store-r2 sellar` |
//! | 6 | registrar la copia | `ore` |
//!
//! # Lo que `ore` hace y lo que no
//!
//! **No abre un socket, ni para leer ni para escribir.** Compila, decide y
//! canaliza: las filas entran por el stdout de un programa y salen por el stdin
//! de otro. Es la tercera vez que este árbol usa esa figura y la razón es la
//! misma —`tests/dependencias.rs` la hace cumplir leyendo el `Cargo.lock`— pero
//! aquí se ve entera: **`ore` está en medio de dos procesos y no toca la red.**
//!
//! # Por qué el paso 4 va antes que el 5, y qué costó que fuera verdad
//!
//! El ADR prometía *«se sabe si hay que copiar sin copiar nada»* con un `HEAD`
//! sobre el nombre del artefacto. **Eso no se podía hacer**: el nombre es el
//! digest del artefacto entero, carga incluida, así que para calcularlo hay que
//! haber leído ya todas las filas — el `HEAD` ahorraba la subida y no la
//! lectura, que es el trabajo caro.
//!
//! Lo arregla el **recibo**: un objeto de 71 bytes en
//! `ore/v1/plan/<sha256 de la cabecera>` que contiene la clave del artefacto. La
//! cabecera se conoce antes de pedirle una fila a nadie, así que ahí sí se puede
//! preguntar. Sigue sin haber puntero mutable: el nombre del recibo también es
//! su contenido.

use std::collections::BTreeMap;
use std::path::Path;

use ore_core::link::{Loaded, Package};
use ore_core::vistas;
use ore_view::{Catalogo, Clasificacion, Vista, comprobar, esquema, linaje};

/// El conducto que una vista materializada instancia. El mismo que mira
/// `ore view`, y no una copia: si divergieran, una vista podría compilar en un
/// sitio y no en el otro.
const CONDUCTO: &str = crate::vista::CONDUCTO;

pub fn materializar(path: &Path, seco: bool) -> std::process::ExitCode {
    let pkg = match crate::cargar_valido(path, true) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let declaradas: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::View && d.section("materialized").is_some())
        .collect();
    if declaradas.is_empty() {
        println!("sin copias · ninguna vista del paquete declara `materialized`");
        return std::process::ExitCode::SUCCESS;
    }

    // ── ① Compilar ──────────────────────────────────────────────────────────
    //
    // El mismo catálogo y la misma clasificación que `ore view`, y por el mismo
    // motivo por el que el registro se construye una vez: dos compilaciones de
    // lo mismo divergen en la que ninguna prueba ejerce.
    let lat = ore_core::flow::lattices(&pkg);
    let tipos = crate::vista::tipos_de_raiz(&pkg);
    let vistas_todas: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::View)
        .collect();
    let catalogo = Catalogo::con(vistas_todas.iter().filter_map(|v| {
        Some(Vista::nueva(
            &v.qname()?,
            crate::vista::cuerpo(&pkg, v, &tipos),
        ))
    }));
    let clasificacion = Clasificacion {
        reticulos: lat.clone(),
        de_raiz: crate::vista::etiquetas_de_raiz(&pkg, &lat),
    };
    let conductos = ore_core::flow::clearances(&pkg, &lat);
    let bundle = ore_core::digest::bundle(&pkg);

    let mut fallos = 0usize;
    for v in &declaradas {
        let Some(qn) = v.qname() else { continue };
        println!("{qn}");
        match una(
            &pkg,
            path,
            v,
            &qn,
            &catalogo,
            &clasificacion,
            &conductos,
            &bundle,
            seco,
        ) {
            Ok(linea) => println!("  {linea}"),
            Err(e) => {
                for l in e.lines() {
                    println!("  {l}");
                }
                fallos += 1;
            }
        }
    }
    if fallos > 0 {
        eprintln!(
            "error: {fallos} de {} no se materializaron",
            declaradas.len()
        );
        return std::process::ExitCode::from(65);
    }
    std::process::ExitCode::SUCCESS
}

#[allow(clippy::too_many_arguments)]
fn una(
    pkg: &Package,
    raiz_pkg: &Path,
    v: &Loaded,
    qn: &str,
    catalogo: &Catalogo,
    clasificacion: &Clasificacion,
    conductos: &BTreeMap<String, ore_core::flow::Labels>,
    bundle: &str,
    seco: bool,
) -> Result<String, String> {
    // ── ① El plan, su digest y su esquema ───────────────────────────────────
    let plan = catalogo
        .expandir(qn)
        .map_err(|e| format!("el plan no se expande · {}", e.como_texto()))?;
    let esq = esquema(&plan).map_err(|d| format!("el esquema no tipa · {}", d.como_texto()))?;

    // ── ② El flujo ──────────────────────────────────────────────────────────
    //
    // Antes de leer nada, y no después: una copia que no compila es una fuga
    // que ya ocurrió. Es la misma comprobación que `ore view` hace, con la
    // misma clasificación — no una segunda.
    let lin = linaje(&plan).map_err(|d| format!("el linaje no se sigue · {}", d.como_texto()))?;
    let autoriza: ore_view::Etiquetas = conductos
        .get(CONDUCTO)
        .map(|ls| {
            ls.iter()
                .map(|(k, (n, _))| (k.clone(), n.clone()))
                .collect()
        })
        .unwrap_or_default();
    let veredicto = comprobar(&lin, clasificacion, &autoriza);
    if !veredicto.compila() {
        let mut s = format!("`{CONDUCTO}` NO compila · no se materializa");
        for f in &veredicto.fugas {
            for l in f.como_texto().lines() {
                s.push('\n');
                s.push_str(l);
            }
        }
        return Err(s);
    }

    // ── ③ El testigo ────────────────────────────────────────────────────────
    let r = vistas::raiz(pkg, v).map_err(|e| format!("sin raíz · {e:?}"))?;
    let testigo = testigo(pkg, v);

    // ── ④ El recibo ─────────────────────────────────────────────────────────
    let cabecera = cabecera(&plan.digest(), &esq, &testigo, bundle);
    let buscado = almacen("buscar", &cabecera, None)?;
    if buscado
        .get("existe")
        .and_then(|(_, x)| x.as_str())
        .is_some_and(|s| s == "true")
    {
        let clave = buscado
            .get("clave")
            .and_then(|(_, x)| x.as_str())
            .unwrap_or("?");
        return Ok(format!(
            "ya está · {clave}\n  el recibo lo dijo sin leer una sola fila del origen"
        ));
    }
    if seco {
        return Ok(format!(
            "haría falta copiarla · testigo {}\n  el recibo no está: {}",
            testigo.1.as_deref().unwrap_or("sin poblar"),
            buscado
                .get("recibo")
                .and_then(|(_, x)| x.as_str())
                .unwrap_or("?")
        ));
    }

    // ── ⑤ Leer, canalizar, sellar ───────────────────────────────────────────
    let filas = leer(raiz_pkg, &r)?;
    let salida = almacen("sellar", &cabecera, Some(&filas))?;

    // ── ⑥ Registrar ─────────────────────────────────────────────────────────
    let campo = |k: &str| {
        salida
            .get(k)
            .and_then(|(_, x)| x.as_str())
            .unwrap_or("?")
            .to_string()
    };
    Ok(format!(
        "copiada · {}\n  {} filas · {} bytes · subido: {}",
        campo("clave"),
        campo("filas"),
        campo("bytes"),
        campo("subido")
    ))
}

/// **③ · El testigo, y el hueco que este peldaño deja abierto.**
///
/// Lo que se puede saber sin abrir nada es el **modo** —`changes.witness` de la
/// tabla raíz— y eso sale de la gramática. Lo que **no** se puede saber es el
/// **valor**: hasta dónde está el origen ahora mismo. Eso solo lo sabe el
/// origen, y el protocolo del driver tiene dos verbos —`catalogo` y `leer`— y
/// ninguno lo pregunta.
///
/// Así que el valor sale vacío, igual que en el registro. La consecuencia hay
/// que decirla porque es grande: **con el testigo vacío, dos materializaciones
/// del mismo plan en momentos distintos dan la misma cabecera**, el recibo dice
/// «ya está», y la copia no se refresca nunca. Sirve para poblar una vez; no
/// sirve para mantener.
///
/// Cerrarlo es un verbo más en [ADR 0008](../../../docs/decisions/0008-el-protocolo-del-driver.md)
/// —`testigo <url> <objeto>`— y es una decisión de protocolo, no de este módulo.
fn testigo(pkg: &Package, v: &Loaded) -> (String, Option<String>) {
    // La marca sale de `registro::marca_de`, que I3 ya escribio: una derivacion
    // y no dos, por lo mismo de siempre.
    let modo = match crate::registro::marca_de(pkg, v) {
        ore_view::Marca::Ninguna => "none",
        ore_view::Marca::Instantanea => "snapshot",
        ore_view::Marca::Registro => "log",
        ore_view::Marca::Campo(_) => "field",
    };
    (modo.to_string(), None)
}

/// La cabecera del sobre, en JSON canónico y en **una** línea, que es lo que el
/// protocolo del almacén espera.
fn cabecera(
    plan: &str,
    esq: &BTreeMap<String, ore_core::types::Type>,
    testigo: &(String, Option<String>),
    bundle: &str,
) -> String {
    use ore_core::json::Json;
    let t = match &testigo.1 {
        Some(v) => Json::obj([("modo", Json::s(&testigo.0)), ("valor", Json::s(v))]),
        None => Json::obj([("modo", Json::s(&testigo.0))]),
    };
    Json::obj([
        ("bundle", Json::s(bundle)),
        ("conducto", Json::s(CONDUCTO)),
        (
            "esquema",
            Json::Obj(
                esq.iter()
                    .map(|(c, t)| (c.clone(), Json::s(t.to_string())))
                    .collect(),
            ),
        ),
        ("plan", Json::s(plan)),
        ("testigo", t),
    ])
    .jcs()
}

/// **⑤ · Las filas, del programa que sabe hablar con el origen.**
///
/// La petición es **un fragmento del plan, no SQL** —[ADR 0008]— y aquí se ve
/// para qué servía: la misma petición vale para PostgreSQL y para un directorio
/// de NDJSON, y `ore` no distingue.
///
/// # Lo que se niega, y por qué negarse es lo correcto
///
/// El `where` de la cadena viaja como filtros, y la petición solo sabe expresar
/// **una igualdad por columna**. Un `pais: [ES, PT]` no cabe. Se podría filtrar
/// aquí, en `ore`, sin abrir nada — pero no se hace todavía, y mientras no se
/// haga **hay que negarse**: una copia que trajera filas que la vista excluye no
/// falla, se sirve. Y sería exactamente el fallo que este árbol no comete.
fn leer(raiz_pkg: &Path, r: &vistas::Raiz) -> Result<String, String> {
    let mut filtros = Vec::new();
    for (columna, valores) in &r.filtros {
        match valores.as_slice() {
            [uno] => filtros.push(ore_core::json::Json::Arr(vec![
                ore_core::json::Json::s(columna),
                ore_core::json::Json::s("eq"),
                ore_core::json::Json::s(uno),
            ])),
            varios => {
                return Err(format!(
                    "el `where` sobre `{columna}` tiene {} valores y la petición solo expresa una \
                     igualdad.\nCopiar sin ese recorte traería filas que la vista excluye, así que \
                     no se copia",
                    varios.len()
                ));
            }
        }
    }

    let (tipo, env) = lector::declaracion(raiz_pkg, &r.datasource)
        .map_err(|f| format!("la fuente `{}` · {}", r.datasource, f.mensaje))?;
    let url = lector::url(raiz_pkg, &env, &r.datasource)
        .map_err(|f| format!("la fuente `{}` · {}", r.datasource, f.mensaje))?;

    use ore_core::json::Json;
    let peticion = Json::obj([
        ("url", Json::s(&url)),
        ("objeto", Json::s(&r.objeto)),
        (
            "proyeccion",
            Json::Obj(
                r.columnas
                    .iter()
                    .map(|(campo, col)| (campo.clone(), Json::s(col)))
                    .collect(),
            ),
        ),
        ("filtros", Json::Arr(filtros)),
    ])
    .jcs();

    lector::ejecutar(
        &format!("ore-read-{tipo}"),
        &["leer".to_string()],
        Some(&peticion),
    )
    .map_err(|f| {
        let mut s = f.mensaje;
        for l in f.ayuda {
            s.push('\n');
            s.push_str(&l);
        }
        s
    })
}

use crate::lector;

/// El almacén, delegado. `ore` **no abre un socket**: escribe por el stdin de un
/// programa y lee su stdout.
fn almacen(
    verbo: &str,
    cabecera: &str,
    filas: Option<&str>,
) -> Result<ore_core::parse::Node, String> {
    let mut entrada = String::from(cabecera);
    entrada.push('\n');
    if let Some(f) = filas {
        entrada.push_str(f);
    }
    let salida = lector::ejecutar("ore-store-r2", &[verbo.to_string()], Some(&entrada))
        .map_err(|f| f.mensaje)?;
    ore_core::parse::parse(&salida)
        .map_err(|e| format!("lo que devolvió `ore-store-r2` no analiza: {e:?}\n{salida}"))
}
