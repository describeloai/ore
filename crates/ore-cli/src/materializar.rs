//! `ore materialize` — **el ciclo**, los seis pasos del
//! [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md).
//!
//! | | qué | quién |
//! |---|---|---|
//! | 1 | compilar: el plan, su digest y el conducto que lo autoriza | `ore` |
//! | 2 | comprobar el flujo | `ore` |
//! | 3 | preguntarle al origen su testigo | `ore-read-<tipo> testigo` |
//! | 4 | el recibo: **si está, termina aquí** | `ore-store-<r2|gcs> buscar` |
//! | 5 | leer, canalizar, sellar y subir | `ore-read-<tipo> leer` → `ore-store-<r2|gcs> sellar` |
//! | 6 | registrar la copia | `ore` |
//! | — | y **recoger** lo que quedó atrás, si se pide | `ore-store-<r2|gcs> recoger` |
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

/// Lo que se pide al ciclo, además del árbol.
pub struct Opciones<'a> {
    pub seco: bool,
    pub recoger: bool,
    pub informe: Option<&'a Path>,
    /// **Rehacer**: no preguntar al recibo, leer el origen entero, y dejar el
    /// recibo apuntando a lo nuevo (el artefacto superado se borra). Para
    /// cuando cambia CÓMO se lee —un driver corregido— o el testigo no se
    /// mueve aunque los datos sí. Sin esto, el recibo manda.
    pub rehacer: bool,
    /// Solo estas vistas (`paquete.vista`); vacío es todas las que declaran copia.
    pub solo: &'a [String],
}

pub fn materializar(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let (seco, recoger, informe) = (op.seco, op.recoger, op.informe);
    // ⭐ La copia exige que compile SU paquete —y lo de la raíz del árbol:
    //   conductos, retículos—, no el inquilino entero. Medido en `demo` (P1
    //   I5): una base foránea con `dueno` sin contestar (`owner: cambiame`,
    //   OOS2009) bloqueaba la copia de otra base que sí compilaba. Un
    //   diagnóstico en un paquete que no declara copias se dice y no para.
    let (pkg, rotos) = match cargar_para_copiar(path) {
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
        // Y lo que quedó de las que hubo: la pasada que limpia.
        if recoger && !seco {
            match recoger_huerfanas(&[]) {
                Ok(l) => println!("  {l}"),
                Err(e) => println!("  {e}"),
            }
        }
        if let Some(dir) = informe {
            retirar_informes_de_nadie(dir, &[]);
        }
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
    let mut vistas = 0usize;
    for v in &declaradas {
        let Some(qn) = v.qname() else { continue };
        if !op.solo.is_empty() && !op.solo.contains(&qn) {
            continue;
        }
        vistas += 1;
        println!("{qn}");
        // Su paquete no compila: esta copia no se intenta, y el informe lo dice.
        if let Some(d) = paquete_del_fichero(path, &v.path).and_then(|p| rotos.get(&p)) {
            let motivo = format!("su paquete no compila · {}", d.lines().next().unwrap_or(""));
            println!("  {motivo}");
            if let Some(dir) = informe
                && let Err(e) = escribir_informe(
                    dir,
                    &qn,
                    &ore_core::json::Json::obj([
                        ("estado", ore_core::json::Json::s("error")),
                        ("motivo", ore_core::json::Json::s(&motivo)),
                    ]),
                )
            {
                println!("  {e}");
            }
            fallos += 1;
            continue;
        }
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
            recoger,
            op.rehacer,
        ) {
            Ok((linea, parte)) => {
                println!("  {linea}");
                if let Some(dir) = informe
                    && let Err(e) = escribir_informe(dir, &qn, &parte)
                {
                    println!("  {e}");
                    fallos += 1;
                }
            }
            Err(e) => {
                for l in e.lines() {
                    println!("  {l}");
                }
                if let Some(dir) = informe
                    && let Err(e) = escribir_informe(
                        dir,
                        &qn,
                        &ore_core::json::Json::obj([
                            ("estado", ore_core::json::Json::s("error")),
                            (
                                "motivo",
                                ore_core::json::Json::s(e.lines().next().unwrap_or("")),
                            ),
                        ]),
                    )
                {
                    println!("  {e}");
                }
                fallos += 1;
            }
        }
    }
    // ── Lo huérfano (2026-09-18) ─────────────────────────────────────────────
    //
    // `recoger` (dentro de `una`) borra las copias superadas de CADA plan
    // vigente. Lo que ningún plan vigente reclama —la base que se retiró, la
    // vista que dejó de declarar copia— sólo se sabe mirando el árbol entero:
    // los planes de TODAS las vistas con copia, con o sin `--vista`, porque una
    // pasada parcial no puede tomar por huérfano lo que no le tocaba.
    let reclamados: Vec<String> = declaradas
        .iter()
        .filter_map(|v| v.qname())
        .filter_map(|qn| catalogo.expandir(&qn).ok().map(|p| p.digest()))
        .collect();
    if recoger && !seco {
        match recoger_huerfanas(&reclamados) {
            Ok(l) => println!("{l}"),
            Err(e) => println!("{e}"),
        }
    }
    if let Some(dir) = informe {
        let vivas: Vec<String> = declaradas.iter().filter_map(|v| v.qname()).collect();
        retirar_informes_de_nadie(dir, &vivas);
    }

    if !op.solo.is_empty() && vistas == 0 {
        eprintln!(
            "error: ninguna de las vistas pedidas ({}) declara copia en este árbol",
            op.solo.join(", ")
        );
        return std::process::ExitCode::from(65);
    }
    if fallos > 0 {
        eprintln!("error: {fallos} de {vistas} no se materializaron");
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
    recoger: bool,
    rehacer: bool,
) -> Result<(String, ore_core::json::Json), String> {
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

    // El *cursor field*: la columna que ordena el avance, si el testigo es por
    // columna. `None` significa que el rango va sobre la posición del propio
    // origen —un LSN, un snapshot— y no sobre ninguna columna.
    let cursor = match crate::registro::marca_de(pkg, v) {
        ore_view::Marca::Campo(c) => Some(c),
        _ => None,
    };
    let clave = crate::registro::clave_de(pkg, v);

    // ── ③ El testigo ────────────────────────────────────────────────────────
    let r = vistas::raiz(pkg, v).map_err(|e| format!("sin raíz · {e:?}"))?;
    let testigo = testigo(pkg, raiz_pkg, v, &r)?;

    // ── ④ El recibo ─────────────────────────────────────────────────────────
    let cabecera = cabecera(&plan.digest(), &esq, &testigo, &clave, bundle);
    let buscado = almacen("buscar", &cabecera, None)?;

    // La recogida va **aquí**, en cuanto se sabe cuál es la cabecera vigente, y
    // no después de sellar. El motivo salió al probarla: si va al final, el
    // retorno temprano de *«ya está»* se la salta — y ese es justamente el caso
    // en el que un almacén lleno de copias viejas no se limpia nunca.
    let recogidas = if recoger && !seco {
        let g = almacen("recoger", &cabecera, None)?;
        g.get("superadas")
            .and_then(|(_, x)| x.as_str())
            .filter(|n| *n != "0")
            .map(|n| format!("\n  recogidas {n} copia(s) superada(s)"))
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Con `rehacer` el recibo no decide: se lee el origen aunque esté.
    if !rehacer
        && buscado
            .get("existe")
            .and_then(|(_, x)| x.as_str())
            .is_some_and(|s| s == "true")
    {
        let clave = buscado
            .get("clave")
            .and_then(|(_, x)| x.as_str())
            .unwrap_or("?");
        return Ok((
            format!(
                "ya está · {clave}\n  el recibo lo dijo sin leer una sola fila del origen{recogidas}"
            ),
            ore_core::json::Json::obj([
                ("estado", ore_core::json::Json::s("al-dia")),
                ("clave", ore_core::json::Json::s(clave)),
                ("plan", ore_core::json::Json::s(plan.digest())),
                (
                    "testigo",
                    ore_core::json::Json::obj([
                        ("modo", ore_core::json::Json::s(&testigo.0)),
                        (
                            "valor",
                            ore_core::json::Json::s(testigo.1.clone().unwrap_or_default()),
                        ),
                    ]),
                ),
                ("leidas", ore_core::json::Json::Int(0)),
            ]),
        ));
    }
    if seco {
        return Ok((
            format!(
                "{} · testigo {}\n  el recibo {}: {}",
                if rehacer {
                    "se rehará entera"
                } else {
                    "haría falta copiarla"
                },
                testigo.1.as_deref().unwrap_or("sin poblar"),
                if rehacer { "no decide" } else { "no está" },
                buscado
                    .get("recibo")
                    .and_then(|(_, x)| x.as_str())
                    .unwrap_or("?")
            ),
            ore_core::json::Json::obj([("estado", ore_core::json::Json::s("pendiente"))]),
        ));
    }

    // ── ⑤ Leer el INCREMENTO, canalizar, fundir y sellar ────────────────────
    //
    // Aquí se juntan las dos mitades que estaban separadas. El almacén dice
    // sobre qué copia se puede construir y hasta dónde llegaba; con eso, al
    // origen se le pide **solo lo que falta** y al almacén se le dice **sobre
    // qué fundirlo**.
    //
    // Y si no hay anterior —primera vez, o un testigo que no ordena— las dos
    // salen vacías y esto es exactamente lo que era: una copia entera.
    // Rehacer es leer entero: sin base sobre la que fundir ni rango desde el
    // que partir. Lo que se copió antes con un lector que callaba columnas no
    // es una base, es lo que se está sustituyendo.
    let previa = if rehacer {
        ore_core::parse::parse("{}").map_err(|e| format!("{e:?}"))?
    } else {
        almacen("anterior", &cabecera, None)?
    };
    let base = previa
        .get("clave")
        .and_then(|(_, x)| x.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let desde = previa
        .get("testigo")
        .and_then(|(_, x)| x.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    // **Si el testigo del origen ordena.** Se toma del que el origen ACABA DE
    // contestar y no del que la tabla declara: cuando discrepan manda el origen
    // —`testigo()` ya lo avisa— y pedir un rango sobre un orden que el servidor
    // dice no tener sería pedirlo contra el documento en vez de contra el
    // mundo.
    let ordena = testigo.0 == "log";
    let filas = match leer(
        raiz_pkg,
        &r,
        cursor.as_deref(),
        desde.as_deref(),
        testigo.1.as_deref(),
        ordena,
    ) {
        Ok(f) => f,
        // **Y si el driver no sabe servir ese rango, se copia entera y se
        // dice.** No es tragarse un error: para una COPIA, leer de más es leer
        // el objeto entero, que es exactamente lo que esto hacía hasta ahora y
        // sigue siendo correcto —lo que cambia es el trabajo, no el resultado—.
        //
        // Lo que no se hace es callarlo. Un origen que declara un testigo
        // ordenado y un lector que no sabe recorrerlo es un hueco real, y hasta
        // hoy era invisible porque la petición ni siquiera llevaba el rango.
        Err(e) if ordena && desde.is_some() && e.contains("changelog") => {
            eprintln!(
                "  aviso · el origen se fecha con `log` y `ore-read-{}` no sabe leer su \
                 changelog, así que esta copia se rehace entera:\n         {e}",
                lector::declaracion(raiz_pkg, &r.datasource)
                    .map(|(t, _)| t)
                    .unwrap_or_default()
            );
            leer(
                raiz_pkg,
                &r,
                cursor.as_deref(),
                desde.as_deref(),
                testigo.1.as_deref(),
                false,
            )?
        }
        Err(e) => return Err(e),
    };
    // **Cuántas filas se le pidieron al origen.** Coincide con las que van a la
    // copia mientras la lectura sea entera; en cuanto un driver sirva el rango,
    // las dos cifras se separan — y **esa diferencia es la medida** de si el
    // refresco es proporcional al cambio o al tamaño.
    //
    // La petición ya sabe llevarlo, por columna y por posición. Lo que queda es
    // que alguien lo sirva, y hoy no lo hace nadie: `pruebas-de-fuego/
    // medida-el-rango-por-posicion.py` §D dice por qué, familia por familia.
    //
    // Se cuenta aquí y no en el banco de pruebas porque es la unidad que
    // [ADR 0014](../../../docs/decisions/0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md)
    // fijó para el proyecto: **una fila mirada**. Una cifra que solo existiera
    // dentro de una prueba no sería una unidad, sería un apaño.
    let leidas = filas.lines().filter(|l| !l.trim().is_empty()).count();
    // La petición del sellado lleva `base`, y la cabecera que se sella **no**:
    // qué contiene la copia y cómo se construyó son dos cosas.
    let mut peticion = match &base {
        Some(b) => cabecera.replacen('{', &format!("{{\"base\":\"{b}\","), 1),
        None => cabecera.clone(),
    };
    if rehacer {
        peticion = peticion.replacen('{', "{\"rehacer\":true,", 1);
    }
    let salida = almacen("sellar", &peticion, Some(&filas))?;

    // ── ⑥ Registrar, y recoger lo que quedó atrás ───────────────────────────
    //
    // La recogida va **después** de que la copia nueva esté arriba y no antes:
    // así, si algo se corta en medio, lo que sobra es una copia de más y no una
    // de menos.
    let campo = |k: &str| {
        salida
            .get(k)
            .and_then(|(_, x)| x.as_str())
            .unwrap_or("?")
            .to_string()
    };
    let entero = |k: &str| campo(k).parse::<i64>().unwrap_or(0);
    // Por columna, cuántas filas la traen: lo que salga vacío se dice aquí y
    // va al informe. Una copia con todas sus filas y sin sus números era
    // `copiada` igual, y nadie lo veía sin abrir el artefacto (medida W1 §B).
    let columnas: BTreeMap<String, ore_core::json::Json> = salida
        .get("columnas")
        .map(|(_, o)| {
            o.entries()
                .iter()
                .filter_map(|(k, v)| {
                    Some((
                        k.as_str()?.to_string(),
                        ore_core::json::Json::Int(v.as_str()?.parse().ok()?),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let vacias: Vec<&str> = columnas
        .iter()
        .filter(|(_, n)| matches!(n, ore_core::json::Json::Int(0)))
        .map(|(c, _)| c.as_str())
        .collect();
    let aviso_columnas = if vacias.is_empty() || entero("filas") == 0 {
        String::new()
    } else {
        format!(
            "\n  ⚠ {} de {} columnas sin ningún valor: {}",
            vacias.len(),
            columnas.len(),
            vacias.join(", ")
        )
    };
    let superada = campo("superada");
    let superada = if superada == "?" {
        String::new()
    } else {
        superada
    };
    // Tres desenlaces de un rehacer, y los tres se dicen: la cabecera cambió
    // (el testigo se movió: recibo nuevo, y la anterior queda para `recoger`),
    // la misma cabecera con otros bytes (el recibo se movió y la superada se
    // borró), o los mismos bytes (nada que mover).
    let rehecha = if rehacer {
        if campo("recibo_nuevo") == "true" {
            "\n  rehecha: bajo una cabecera nueva (el testigo se movió); la anterior queda superada"
                .to_string()
        } else if superada.is_empty() {
            "\n  rehecha: los mismos bytes, el recibo no se movió".to_string()
        } else {
            format!("\n  rehecha: el recibo apunta a la nueva y se borró {superada}")
        }
    } else {
        String::new()
    };
    Ok((
        format!(
            "copiada · {}\n  {} filas · {leidas} leidas · {} bytes · subido: {}{recogidas}{aviso_columnas}{rehecha}",
            campo("clave"),
            campo("filas"),
            campo("bytes"),
            campo("subido")
        ),
        ore_core::json::Json::obj([
            ("estado", ore_core::json::Json::s("copiada")),
            ("rehecha", ore_core::json::Json::Bool(rehacer)),
            ("superada", ore_core::json::Json::s(superada)),
            ("clave", ore_core::json::Json::s(campo("clave"))),
            ("digest", ore_core::json::Json::s(campo("digest"))),
            ("plan", ore_core::json::Json::s(plan.digest())),
            ("filas", ore_core::json::Json::Int(entero("filas"))),
            ("leidas", ore_core::json::Json::Int(leidas as i64)),
            ("bytes", ore_core::json::Json::Int(entero("bytes"))),
            ("columnas", ore_core::json::Json::Obj(columnas)),
            (
                "subido",
                ore_core::json::Json::Bool(campo("subido") == "true"),
            ),
            (
                "testigo",
                ore_core::json::Json::obj([
                    ("modo", ore_core::json::Json::s(&testigo.0)),
                    (
                        "valor",
                        ore_core::json::Json::s(testigo.1.clone().unwrap_or_default()),
                    ),
                ]),
            ),
        ]),
    ))
}

/// **El informe de la copia** (`--informe DIR`): un JSON por vista, para que
/// quien no alcanza ni el origen ni el almacén —`ore-serve`, la consola— sepa
/// qué copia hay y cuánto tiene. Lo escribe el Job de la celda y lo empuja al
/// árbol, y ahí el commit dice cuándo y quién. No es el registro (0015: el
/// recibo vive en el almacén y no hay puntero mutable): es lo que la última
/// pasada dijo, como el snapshot del informador.
///
/// Con «ya está» no se conocen las filas —nadie las contó—: se conservan las
/// del informe anterior si la clave es la misma, y se dice `al-dia`.
/// `ore-store recoger-huerfanas` con los planes que el árbol reclama.
fn recoger_huerfanas(planes: &[String]) -> Result<String, String> {
    use ore_core::json::Json;
    let entrada = Json::obj([
        (
            "planes",
            Json::Arr(planes.iter().map(|p| Json::s(p)).collect()),
        ),
        ("seco", Json::Bool(false)),
    ])
    .jcs();
    let r = almacen("recoger-huerfanas", &entrada, None)?;
    let n = |k: &str| {
        r.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("0")
            .to_string()
    };
    Ok(format!(
        "huérfanas: {} copia(s) de planes que ninguna vista reclama y {} artefacto(s) sin recibo, fuera del almacén ({} recibos, {} planes vigentes)",
        n("huerfanas"),
        n("sueltos"),
        n("recibos"),
        n("planes")
    ))
}

/// Los informes `copias/<paquete>_<vista>.json` de vistas que ya no están en
/// el árbol, fuera: un recibo de nadie en el árbol es tan engañoso como una
/// copia de nadie en el almacén.
fn retirar_informes_de_nadie(dir: &Path, vivas: &[String]) {
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entradas.flatten() {
        let ruta = e.path();
        if ruta.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&ruta) else {
            continue;
        };
        let vista = ore_core::parse::parse(&t).ok().and_then(|n| {
            n.get("vista")
                .and_then(|(_, v)| v.as_str().map(String::from))
        });
        if let Some(v) = vista
            && !vivas.contains(&v)
        {
            if std::fs::remove_file(&ruta).is_ok() {
                println!("  informe de `{v}` retirado: la vista ya no está en el árbol");
            }
        }
    }
}

fn escribir_informe(dir: &Path, qn: &str, parte: &ore_core::json::Json) -> Result<(), String> {
    use ore_core::json::Json;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("no se pudo crear `{}`: {e}", dir.display()))?;
    let ruta = dir.join(format!("{}.json", qn.replace('.', "_")));
    let mut m = match parte {
        Json::Obj(m) => m.clone(),
        _ => Default::default(),
    };
    m.insert("vista".into(), Json::s(qn));
    if m.get("estado") == Some(&Json::s("al-dia"))
        && let Ok(previo) = std::fs::read_to_string(&ruta)
        && let Ok(n) = ore_core::parse::parse(&previo)
        && n.get("clave").and_then(|(_, c)| c.as_str())
            == m.get("clave").and_then(|c| {
                if let Json::Str(s) = c {
                    Some(s.as_str())
                } else {
                    None
                }
            })
    {
        for k in ["filas", "bytes", "digest"] {
            if let Some((_, v)) = n.get(k)
                && let Some(t) = v.as_str()
            {
                m.insert(
                    k.into(),
                    t.parse::<i64>().map(Json::Int).unwrap_or(Json::s(t)),
                );
            }
        }
    }
    std::fs::write(&ruta, Json::Obj(m).pretty() + "\n")
        .map_err(|e| format!("no se pudo escribir `{}`: {e}", ruta.display()))
}

/// Carga el árbol y lo rechaza sólo si no compila **la raíz** (conductos,
/// retículos, config). Un paquete que no compila se lleva **sus** vistas —salen
/// como `error`, con el primer diagnóstico— y no las de los demás: una base a
/// medio decidir no bloquea las copias del inquilino. Lo roto en paquetes sin
/// copia se dice y no para.
fn cargar_para_copiar(
    path: &Path,
) -> Result<(Package, BTreeMap<String, String>), std::process::ExitCode> {
    if !path.is_dir() {
        eprintln!("error: `{}` no es un directorio de paquete", path.display());
        return Err(std::process::ExitCode::from(66)); // EX_NOINPUT
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(path);
    let con_copia: std::collections::BTreeSet<String> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::View && d.section("materialized").is_some())
        .filter_map(|d| paquete_del_fichero(path, &d.path))
        .collect();
    let diags: Vec<_> = ore_core::validate_package(path)
        .into_iter()
        .filter(|d| d.code != ore_core::Code::Oos2013)
        .collect();
    // paquete → su primer diagnóstico; `None` es la raíz del árbol
    let mut rotos: BTreeMap<Option<String>, ore_core::Diagnostic> = BTreeMap::new();
    let mut ajenos = 0usize;
    for d in diags {
        let p = paquete_del_fichero(path, &d.file);
        if let Some(p) = &p
            && !con_copia.contains(p)
        {
            ajenos += 1;
        }
        rotos.entry(p).or_insert(d);
    }
    if ajenos > 0 {
        eprintln!(
            "aviso · {ajenos} diagnóstico(s) en paquetes que no declaran copia — no bloquean la copia"
        );
    }
    if let Some(d) = rotos.get(&None) {
        eprintln!("{}", d.render(path));
        return Err(std::process::ExitCode::from(65)); // EX_DATAERR
    }
    let por_paquete: BTreeMap<String, String> = rotos
        .into_iter()
        .filter_map(|(p, d)| Some((p?, d.render(path))))
        .filter(|(p, _)| con_copia.contains(p))
        .collect();
    for (p, d) in &por_paquete {
        eprintln!("aviso · el paquete `{p}` no compila: sus copias salen como error\n{d}");
    }
    Ok((pkg, por_paquete))
}

/// `packages/<p>/...` → `p`; `None` para lo que vive en la raíz del árbol.
pub(crate) fn paquete_del_fichero(raiz: &Path, fichero: &Path) -> Option<String> {
    let rel = fichero.strip_prefix(raiz).unwrap_or(fichero);
    let mut partes = rel.components();
    match partes.next()?.as_os_str().to_str()? {
        "packages" => Some(partes.next()?.as_os_str().to_string_lossy().into_owned()),
        _ => None,
    }
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
fn testigo(
    pkg: &Package,
    raiz_pkg: &Path,
    v: &Loaded,
    r: &vistas::Raiz,
) -> Result<(String, Option<String>), String> {
    // La **marca** sale de la gramática, por `registro::marca_de` — una
    // derivación y no dos. Es lo que el origen dice que sabe hacer.
    let declarado = match crate::registro::marca_de(pkg, v) {
        ore_view::Marca::Ninguna => "none",
        ore_view::Marca::Instantanea => "snapshot",
        ore_view::Marca::Registro => "log",
        ore_view::Marca::Campo(_) => "field",
    };
    // Una tabla que declara no fecharse no se le pregunta. Preguntar igual
    // seria darle la oportunidad de contradecir su propia declaracion, y la
    // declaracion es la que compila.
    if declarado == "none" {
        return Ok(("none".to_string(), None));
    }

    // Y el **valor** sale del origen, que es el unico que lo sabe. El orden
    // importa y es el que Debezium documenta: se pregunta ANTES de leer las
    // filas, asi que lo que cambie durante la copia se re-entrega en el
    // siguiente refresco. Al reves se perderia, y perder es peor que repetir.
    let (tipo, env) = lector::declaracion(raiz_pkg, &r.datasource)
        .map_err(|f| format!("la fuente `{}` · {}", r.datasource, f.mensaje))?;
    let url = lector::url(raiz_pkg, &env, &r.datasource)
        .map_err(|f| format!("la fuente `{}` · {}", r.datasource, f.mensaje))?;
    // La coordenada lleva el *cursor* cuando el testigo es por columna: el driver
    // necesita saber CUAL ordena, porque un fichero o una tabla saben fecharse de
    // mas de una forma y la que vale es la que la tabla declara.
    let mut coord = vec![
        ("objeto", ore_core::json::Json::s(&r.objeto)),
        ("url", ore_core::json::Json::s(&url)),
    ];
    if let ore_view::Marca::Campo(c) = crate::registro::marca_de(pkg, v) {
        coord.push(("cursor", ore_core::json::Json::s(&c)));
    }
    let peticion = ore_core::json::Json::obj(coord).jcs();

    let salida = lector::ejecutar(
        &format!("ore-read-{tipo}"),
        &["testigo".to_string()],
        Some(&peticion),
    )
    .map_err(|f| {
        let mut s = f.mensaje;
        for l in f.ayuda {
            s.push('\n');
            s.push_str(&l);
        }
        s
    })?;
    let n = ore_core::parse::parse(&salida)
        .map_err(|e| format!("lo que devolvió el testigo no analiza: {e:?}\n{salida}"))?;
    let modo = n
        .get("modo")
        .and_then(|(_, x)| x.as_str())
        .unwrap_or("none")
        .to_string();
    let valor = n
        .get("valor")
        .and_then(|(_, x)| x.as_str())
        .map(String::from);

    // **Y si los dos no concuerdan, manda el origen — pero se dice.** La tabla
    // declara `log` y el servidor contesta `none` cuando la decodificacion
    // logica esta apagada: el documento no miente, esta desactualizado. Callarlo
    // dejaria una copia con un testigo vacio y nadie sabria por que no se
    // refresca.
    // **Y aquí NO se comprueba la deriva entera, a propósito.** Atlas aborta un
    // `migrate apply` si el esquema derivó, y el argumento vale: una copia viaja
    // sellada con la clasificación de los campos de la vista, y si el origen
    // dejó de empujar la proyección el sello miente (`OOS2029`). Pero un chequeo
    // completo es una consulta facturada en el camino caliente, y `migrate
    // apply` es raro y deliberado mientras esto corre en un horario.
    //
    // La regla que sale de ahí y vale para todo el árbol: **lo que el driver ya
    // contesta se comprueba gratis; lo que hay que ir a preguntar es otro acto**
    // —`ore drift-detect`—. Esto es la mitad gratis: el testigo viene en la
    // respuesta que ya se pide.
    if modo != declarado {
        eprintln!(
            "aviso · `{}` declara `witness: {declarado}` y el origen contesta `{modo}`: manda el \
             origen, y esta copia se fecha con lo que hay",
            r.tabla.as_deref().unwrap_or(&r.objeto)
        );
    }
    Ok((modo, valor))
}

/// La cabecera del sobre, en JSON canónico y en **una** línea, que es lo que el
/// protocolo del almacén espera.
fn cabecera(
    plan: &str,
    esq: &BTreeMap<String, ore_core::types::Type>,
    testigo: &(String, Option<String>),
    clave: &[String],
    bundle: &str,
) -> String {
    use ore_core::json::Json;
    let t = match &testigo.1 {
        Some(v) => Json::obj([("modo", Json::s(&testigo.0)), ("valor", Json::s(v))]),
        None => Json::obj([("modo", Json::s(&testigo.0))]),
    };
    Json::obj([
        ("bundle", Json::s(bundle)),
        ("clave", Json::Arr(clave.iter().map(Json::s).collect())),
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
fn leer(
    raiz_pkg: &Path,
    r: &vistas::Raiz,
    cursor: Option<&str>,
    desde: Option<&str>,
    hasta: Option<&str>,
    // Si el testigo del origen ORDENA. Decide si un rango sin columna tiene
    // sentido: `log` si, `snapshot` no.
    ordena: bool,
) -> Result<String, String> {
    let (tipo, env) = lector::declaracion(raiz_pkg, &r.datasource)
        .map_err(|f| format!("la fuente `{}` · {}", r.datasource, f.mensaje))?;
    let url = lector::url(raiz_pkg, &env, &r.datasource)
        .map_err(|f| format!("la fuente `{}` · {}", r.datasource, f.mensaje))?;
    let peticion = peticion(&url, r, cursor, desde, hasta, ordena)?;

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

/// **La petición, armada aparte y sin tocar nada.**
///
/// Se separa de [`leer`] por lo mismo que `ore-sql` separa la traducción del
/// transporte: que el rango salga bien tiene que ser un **aserto**, y un aserto
/// que exigiera lanzar un proceso ajeno no se ejecutaría nunca en la suite.
fn peticion(
    url: &str,
    r: &vistas::Raiz,
    cursor: Option<&str>,
    desde: Option<&str>,
    hasta: Option<&str>,
    ordena: bool,
) -> Result<String, String> {
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

    use ore_core::json::Json;
    let mut campos: Vec<(&'static str, Json)> =
        vec![("url", Json::s(url)), ("objeto", Json::s(&r.objeto))];
    // **El rango, solo si hay de dónde partir.** Sin `desde` esto es una lectura
    // entera, que es lo que la primera materialización necesita.
    //
    // Y va **sobre una columna o sobre la posición del origen**, que son los dos
    // casos que el protocolo define y hasta ahora solo se ponía el primero:
    // `cursor` es `None` cuando el testigo no es por columna, y con la guarda
    // vieja —`(Some(cursor), Some(desde))`— `desde` y `hasta` se calculaban, se
    // pasaban aquí y **se tiraban**. La lectura era entera siempre, y este mismo
    // fichero lo tenía anotado tres líneas más abajo.
    //
    // Lo que decide cuál de los dos es **si el testigo ordena**:
    //
    // - `log` es una posición en un flujo de cambios: ordena, así que «lo que
    //   hay entre A y B» significa algo y el rango va sin `cursor`.
    // - `snapshot` es una **identidad** de versión y no ordena —dos digests no
    //   se comparan—, así que no admite rango. Lo que sí admite es un pin, y ese
    //   ya está explotado: el testigo entra en la cabecera, y una cabecera igual
    //   da el recibo que corta el ciclo en el paso ④ sin leer nada.
    if let Some(d) = desde.filter(|_| cursor.is_some() || ordena) {
        if let Some(c) = cursor {
            campos.push(("cursor", Json::s(c)));
        }
        campos.push(("start", Json::s(d)));
        // Y `end` acota por arriba con el testigo que el origen acaba de dar,
        // para que lo copiado y su marca sean el mismo instante.
        if let Some(h) = hasta {
            campos.push(("end", Json::s(h)));
        }
    }
    campos.push((
        "proyeccion",
        Json::Obj(
            r.columnas
                .iter()
                .map(|(campo, col)| (campo.clone(), Json::s(col)))
                .collect(),
        ),
    ));
    campos.push(("filtros", Json::Arr(filtros)));
    Ok(Json::obj(campos).jcs())
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
    let programa = programa_del_almacen()?;
    // Con su stderr: «`ore-store-gcs` falló (1)» a secas costó una pasada en
    // `demo` sin saber por qué. Lo que el almacén dice es lo único accionable.
    let salida =
        lector::ejecutar(&programa, &[verbo.to_string()], Some(&entrada)).map_err(|f| {
            let mut s = f.mensaje;
            for l in f.ayuda {
                s.push('\n');
                s.push_str(&l);
            }
            s
        })?;
    ore_core::parse::parse(&salida)
        .map_err(|e| format!("lo que devolvió `{programa}` no analiza: {e:?}\n{salida}"))
}

/// `ore-store-r2` o `ore-store-gcs`, según `ORE_STORE`: `r2` (un S3 con clave
/// estática, el de siempre) o `gcs` (Google Cloud Storage con el token de la
/// cuenta que corre: la celda). Los dos hablan el mismo protocolo y sellan el
/// mismo nombre; cambia dónde y con qué credencial. Sin la variable, `r2`, como
/// hasta el 2026-09-17. Un valor que no es uno de los dos es un error dicho, no
/// un binario que no se encuentra.
pub fn programa_del_almacen() -> Result<String, String> {
    match std::env::var("ORE_STORE").as_deref() {
        Err(_) | Ok("") | Ok("r2") => Ok("ore-store-r2".into()),
        Ok("gcs") => Ok("ore-store-gcs".into()),
        Ok(otro) => Err(format!(
            "`ORE_STORE={otro}` no es un almacén: vale `r2` (S3 con clave estática) o `gcs` (Google Cloud Storage con Workload Identity)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raiz() -> vistas::Raiz {
        vistas::Raiz {
            datasource: "erp".into(),
            objeto: "public.employees".into(),
            columnas: [("id".to_string(), "employee_id".to_string())]
                .into_iter()
                .collect(),
            filtros: Vec::new(),
            agrega: Default::default(),
            tabla: None,
        }
    }

    /// **El rango sobre una columna**: lleva `cursor`, y `start` es exclusivo.
    #[test]
    fn con_cursor_el_rango_va_sobre_la_columna() {
        let p = peticion(
            "x://y",
            &raiz(),
            Some("actualizado"),
            Some("7"),
            Some("9"),
            false,
        )
        .expect("petición");
        assert!(p.contains("\"cursor\":\"actualizado\""), "{p}");
        assert!(p.contains("\"start\":\"7\""), "{p}");
        assert!(p.contains("\"end\":\"9\""), "{p}");
    }

    /// **El rango sobre la posición del origen**: sin `cursor`, y solo cuando el
    /// testigo ordena.
    ///
    /// Es lo que faltaba: con la guarda vieja —`(Some(cursor), Some(desde))`—
    /// `desde` y `hasta` llegaban aquí y se tiraban, así que un origen con
    /// `witness: log` se releía entero en cada refresco y nadie lo veía.
    #[test]
    fn sin_cursor_y_con_testigo_que_ordena_el_rango_va_sobre_la_posicion() {
        let p = peticion("x://y", &raiz(), None, Some("7"), Some("9"), true).expect("petición");
        assert!(!p.contains("\"cursor\""), "{p}");
        assert!(p.contains("\"start\":\"7\""), "{p}");
        assert!(p.contains("\"end\":\"9\""), "{p}");
    }

    /// Y un testigo que **no ordena** no lleva rango, aunque haya de dónde
    /// partir: dos `snapshot` no se comparan, así que «lo que hay entre A y B»
    /// no significa nada. Lo que ese testigo permite es un pin, y ese ya lo
    /// explota el recibo del paso ④.
    #[test]
    fn un_testigo_que_no_ordena_no_lleva_rango() {
        let p = peticion(
            "x://y",
            &raiz(),
            None,
            Some("sha256:abc"),
            Some("sha256:def"),
            false,
        )
        .expect("petición");
        assert!(!p.contains("\"start\""), "{p}");
        assert!(!p.contains("\"end\""), "{p}");
    }

    /// Sin `desde` no hay rango en ningún caso: es la primera copia, y una
    /// primera copia es entera por definición.
    #[test]
    fn la_primera_copia_no_lleva_rango() {
        let p = peticion("x://y", &raiz(), Some("actualizado"), None, Some("9"), true)
            .expect("petición");
        assert!(!p.contains("\"start\""), "{p}");
    }
}
