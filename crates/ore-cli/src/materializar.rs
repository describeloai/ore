//! `ore materialize` — **el ciclo**, los seis pasos del
//! [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md).
//!
//! | | qué | quién |
//! |---|---|---|
//! | 1 | compilar: el plan, su digest y el conducto que lo autoriza | `ore` |
//! | 2 | comprobar el flujo | `ore` |
//! | 3 | preguntarle al origen su testigo | `ore-read-<tipo> testigo` |
//! | 4 | el puntero del árbol: **si la cabecera es la misma, termina aquí** | `ore` (+ `ore-store-<r2|gcs> buscar`, un HEAD) |
//! | 5 | leer, canalizar y sellar el dataset | `ore-read-<tipo> leer` → `ore-store-<r2|gcs> sellar` · sobre el lago, `ore-store-<r2|gcs> copiar` |
//! | 6 | mover el puntero: `copias/<p>_<v>.json` | `ore` |
//! | — | y **recoger** lo que quedó atrás, si se pide | `ore-store-<r2|gcs> recoger` |
//!
//! # La copia es un dataset (W3.6a, 0031 §10, 2026-09-20)
//!
//! Hasta aquí la copia era un sobre nombrado por su digest y **el estado vivía
//! en el bucket**: un recibo por cabecera decía cuál era la vigente, y `ore`
//! le preguntaba. Ahora la copia es **una tabla Iceberg con su historia**, y el
//! estado vive **en el árbol**: `copias/<p>_<v>.json` es el puntero —qué
//! `metadata.json` es el vigente, con qué cabecera se selló, hasta qué testigo—
//! y el árbol es el catálogo. El commit del Job es el *swap* del puntero; la
//! forja, al rechazar lo que no avanza en línea recta, es el *compare-and-set*.
//! `ore` lee el puntero antes de leer una fila, decide con él si hay que
//! copiar, desde dónde y sobre qué fundir, y lo reescribe al terminar.
//!
//! Sin `--informe`, los punteros viven en `<árbol>/copias`: no hay otro sitio
//! donde puedan vivir, porque el bucket ya no guarda estado.
//!
//! # Lo que `ore` hace y lo que no
//!
//! **No abre un socket, ni para leer ni para escribir.** Compila, decide y
//! canaliza: las filas entran por el stdout de un programa y salen por el stdin
//! de otro. Es la tercera vez que este árbol usa esa figura y la razón es la
//! misma —`tests/dependencias.rs` la hace cumplir leyendo el `Cargo.lock`— pero
//! aquí se ve entera: **`ore` está en medio de dos procesos y no toca la red.**
//!
//! # Por qué el paso 4 va antes que el 5
//!
//! El ADR prometía *«se sabe si hay que copiar sin copiar nada»*. La cabecera
//! —plan, esquema, testigo, clave, conducto— se conoce antes de pedirle una
//! fila a nadie, y el puntero guarda su digest: si coinciden, la copia está y
//! no se lee el origen. Un `HEAD` al `metadata.json` apuntado protege de un
//! bucket que alguien vació.

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
    /// Dónde viven los punteros (`<DIR>/<paquete>_<vista>.json`). Sin él,
    /// `<árbol>/copias`.
    pub informe: Option<&'a Path>,
    /// **Rehacer**: no hacer caso al puntero, leer el origen entero, y
    /// sobrescribir el dataset (un snapshot nuevo; la historia se queda). Para
    /// cuando cambia CÓMO se lee —un driver corregido— o el testigo no se
    /// mueve aunque los datos sí. Sin esto, el puntero manda.
    pub rehacer: bool,
    /// Solo estas vistas (`paquete.vista`); vacío es todas las que declaran copia.
    pub solo: &'a [String],
}

pub fn materializar(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let (seco, recoger) = (op.seco, op.recoger);
    let punteros = op
        .informe
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.join("copias"));
    // En seco no se toca el árbol: el puntero es el estado, y una pasada que
    // sólo dice qué haría no puede dejarlo diciendo «pendiente».
    let informe: Option<&Path> = if seco { None } else { Some(&punteros) };
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
            match recoger_huerfanas(&[], &[]) {
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

    // Los sobres heredados que los punteros nombran ANTES de esta pasada: una
    // vista que se resella como dataset deja de nombrar el suyo, y aun así el
    // sobre se queda hasta la pasada siguiente — el árbol que la consola lee
    // sigue apuntándolo hasta que el commit de esta pasada se empuje.
    let heredados: Vec<String> = declaradas
        .iter()
        .filter_map(|v| v.qname())
        .filter_map(|qn| leer_puntero(&punteros, &qn))
        .filter_map(|p| campo_de(&p, "clave"))
        .collect();

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
            &punteros,
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
    // `recoger` (dentro de `una`) limpia DENTRO de cada dataset vigente. Lo que
    // ningún puntero reclama —la base que se retiró, la vista que dejó de
    // declarar copia— sólo se sabe mirando el árbol entero: TODAS las vistas
    // con copia, con o sin `--vista`, porque una pasada parcial no puede tomar
    // por huérfano lo que no le tocaba. Y los sobres heredados (`ore/v1/`) que
    // algún puntero todavía nombre por `clave` se quedan hasta que se resellen.
    let reclamados: Vec<String> = declaradas
        .iter()
        .filter_map(|v| v.qname())
        .map(|qn| dataset_de(&qn))
        .collect();
    if recoger && !seco {
        match recoger_huerfanas(&reclamados, &heredados) {
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
    punteros: &Path,
) -> Result<(String, ore_core::json::Json), String> {
    use ore_core::json::Json;
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
    // **Una raíz del lago no tiene driver** (0031 «(d)»): su puntero está en
    // el árbol (`datasets/<objeto>.json`) y no en una URL, y se copia en Arrow
    // por `ore-store copiar`, sin pasar por el protocolo de texto de 0008.
    let del_lago = lector::declaracion(raiz_pkg, &r.datasource)
        .map(|(t, _)| t == "lago")
        .unwrap_or(false);
    let origen_del_lago = if del_lago {
        Some(origen_del_lago(pkg, raiz_pkg, v, &r)?)
    } else {
        None
    };
    let testigo = match &origen_del_lago {
        Some(o) => o.testigo.clone(),
        None => testigo(pkg, raiz_pkg, v, &r)?,
    };

    // ── ④ El puntero ────────────────────────────────────────────────────────
    //
    // Lo que el árbol dice de esta copia. La cabecera de ahora se compara con
    // la que el puntero guarda: si es la misma, la copia está y no se lee una
    // fila. Un HEAD al `metadata.json` apuntado, para no decir «ya está» de un
    // bucket que alguien vació.
    let cabecera = cabecera(&plan.digest(), &esq, &testigo, &clave);
    let huella = ore_core::digest::de_bytes(cabecera.as_bytes());
    let dataset = dataset_de(qn);
    let puntero = leer_puntero(punteros, qn);
    // Con cualquier `estado`: un puntero que dice `error` por una pasada que
    // falló sigue nombrando el dataset que la anterior dejó, y ese dataset es
    // sobre el que se construye. Si no, cada fallo transitorio estrenaría uno.
    let hecho: Option<String> = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "metadata_location"));
    // La misma cabecera que la última vez, y el dataset sigue ahí: al día.
    let al_dia = !rehacer
        && hecho.is_some()
        && puntero
            .as_ref()
            .and_then(|p| campo_de(p, "cabecera"))
            .as_deref()
            == Some(huella.as_str())
        && {
            let b = almacen(
                "buscar",
                &Json::obj([("metadata_location", Json::s(hecho.as_deref().unwrap_or("")))]).jcs(),
                None,
            )?;
            campo_de(&b, "existe").as_deref() == Some("true")
        };
    if al_dia {
        let ml = hecho.clone().unwrap_or_default();
        // El puntero de antes, tal cual, con el estado de hoy. Las filas y las
        // cuentas son las de la copia que ya estaba: nadie las recontó.
        let mut m = match puntero.as_ref().map(Json::de_node) {
            Some(Json::Obj(m)) => m,
            _ => Default::default(),
        };
        m.insert("estado".into(), Json::s("al-dia"));
        m.insert("bundle".into(), Json::s(bundle));
        m.insert("leidas".into(), Json::Int(0));
        m.remove("rehecha");
        let recogidas = recoger_dataset(recoger && !seco, &dataset, &ml, &mut m)?;
        return Ok((
            format!(
                "ya está · {ml}\n  el puntero lo dijo sin leer una sola fila del origen{recogidas}"
            ),
            Json::Obj(m),
        ));
    }
    if seco {
        return Ok((
            format!(
                "{} · testigo {}\n  el puntero {}: {}",
                if rehacer {
                    "se rehará entera"
                } else {
                    "haría falta copiarla"
                },
                testigo.1.as_deref().unwrap_or("sin poblar"),
                if rehacer {
                    "no decide"
                } else if hecho.is_some() {
                    "es de otra cabecera"
                } else {
                    "no está"
                },
                hecho.as_deref().unwrap_or("(sin dataset)")
            ),
            Json::obj([("estado", Json::s("pendiente"))]),
        ));
    }

    // ── ⑤ Leer el INCREMENTO, canalizar, fundir y sellar ────────────────────
    //
    // El puntero dice sobre qué dataset se construye (`base`) y hasta dónde
    // llegaba (`testigo`); con eso, al origen se le pide **solo lo que falta**
    // y al almacén se le dice **que funda**. Sólo si el plan es el mismo —la
    // misma proyección, los mismos filtros— y el testigo ordena: un plan que
    // cambió se copia entero y sobrescribe. Rehacer es leer entero: sin rango
    // desde el que partir. Y si no hay dataset —primera vez, o un puntero
    // heredado que nombra un sobre— esto es exactamente lo que era: una copia
    // entera, y el dataset nace.
    let base = hecho.clone();
    let mismo_plan = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "plan"))
        .is_some_and(|p| p == plan.digest());
    let desde: Option<String> = if rehacer || base.is_none() || !mismo_plan || clave.is_empty() {
        None
    } else {
        puntero
            .as_ref()
            .and_then(|p| p.get("testigo").map(|(_, t)| t.clone()))
            .and_then(|t| campo_de(&t, "valor"))
            .filter(|v| {
                // La mayor de las que quedan por debajo de la actual. Un
                // testigo POSTERIOR no es una base: sería leer hacia atrás.
                testigo
                    .1
                    .as_deref()
                    .is_some_and(|actual| v.as_str() < actual)
            })
    };

    // **Si el testigo del origen ordena.** Se toma del que el origen ACABA DE
    // contestar y no del que la tabla declara: cuando discrepan manda el origen
    // —`testigo()` ya lo avisa— y pedir un rango sobre un orden que el servidor
    // dice no tener sería pedirlo contra el documento en vez de contra el
    // mundo.
    let ordena = testigo.0 == "log";
    let mut fundir = desde.is_some() && (cursor.is_some() || ordena);
    // La petición del sellado lleva `dataset`, `base` y `fundir`, y la cabecera
    // que se sella **no**: qué contiene la copia y cómo se construyó son dos
    // cosas.
    let peticion_de = |fundir: bool, extra: &str| {
        let mut e = format!("{{\"dataset\":\"{dataset}\",\"fundir\":{fundir},");
        if let Some(b) = &base {
            e.push_str(&format!("\"base\":\"{b}\","));
        }
        e.push_str(extra);
        cabecera.replacen('{', &e, 1)
    };
    if let Some(o) = &origen_del_lago {
        // Sobre el lago se copia entera: un snapshot es una identidad, no un
        // orden, y la cabecera igual ya corta el ciclo en ④ sin leer nada.
        let salida = almacen("copiar", &peticion_de(false, &o.origen_json(&r)?), None)?;
        let leidas = campo_de(&salida, "leidas")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        return informe_de(
            &salida,
            leidas,
            &dataset,
            &huella,
            &plan.digest(),
            &testigo,
            bundle,
            rehacer,
            recoger && !seco,
        );
    }
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
            fundir = false;
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
    // Se cuenta aquí y no en el banco de pruebas porque es la unidad que
    // [ADR 0014](../../../docs/decisions/0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md)
    // fijó para el proyecto: **una fila mirada**. Una cifra que solo existiera
    // dentro de una prueba no sería una unidad, sería un apaño.
    let leidas = filas.lines().filter(|l| !l.trim().is_empty()).count();
    let salida = almacen("sellar", &peticion_de(fundir, ""), Some(&filas))?;
    informe_de(
        &salida,
        leidas,
        &dataset,
        &huella,
        &plan.digest(),
        &testigo,
        bundle,
        rehacer,
        recoger && !seco,
    )
}

/// **⑥ El puntero nuevo, y recoger lo que quedó atrás** — de lo que el almacén
/// contestó a `sellar` o a `copiar`, que es la misma línea.
#[allow(clippy::too_many_arguments)]
fn informe_de(
    salida: &ore_core::parse::Node,
    leidas: usize,
    dataset: &str,
    huella: &str,
    plan: &str,
    testigo: &(String, Option<String>),
    bundle: &str,
    rehacer: bool,
    recoger: bool,
) -> Result<(String, ore_core::json::Json), String> {
    use ore_core::json::Json;
    //
    // La recogida va **después** de que el snapshot nuevo esté confirmado y no
    // antes: así, si algo se corta en medio, lo que sobra es un snapshot de
    // más y no uno de menos.
    let campo = |k: &str| campo_de(salida, k).unwrap_or_else(|| "?".into());
    let entero = |k: &str| campo(k).parse::<i64>().unwrap_or(0);
    // Por columna, cuántas filas la traen: lo que salga vacío se dice aquí y
    // va al informe. Una copia con todas sus filas y sin sus números era
    // `copiada` igual, y nadie lo veía sin abrir el artefacto (medida W1 §B).
    let columnas: BTreeMap<String, Json> = salida
        .get("columnas")
        .map(|(_, o)| {
            o.entries()
                .iter()
                .filter_map(|(k, v)| {
                    Some((
                        k.as_str()?.to_string(),
                        Json::Int(v.as_str()?.parse().ok()?),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let vacias: Vec<&str> = columnas
        .iter()
        .filter(|(_, n)| matches!(n, Json::Int(0)))
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
    // Las columnas que el contrato de tipos (0032) quería estrechar y se
    // quedaron como texto porque un valor no analizó. La copia está bien y es
    // legible; lo que no está es el tipo, y hay que decirlo donde se lee.
    let sin_estrechar: BTreeMap<String, Json> = salida
        .get("sin_estrechar")
        .map(|(_, o)| {
            o.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), Json::s(v.as_str()?))))
                .collect()
        })
        .unwrap_or_default();
    let aviso_tipos = if sin_estrechar.is_empty() {
        String::new()
    } else {
        format!(
            "\n  ⚠ {} columnas sin estrechar (se quedan como texto): {}",
            sin_estrechar.len(),
            sin_estrechar
                .iter()
                .map(|(c, p)| format!(
                    "{c} — {}",
                    match p {
                        Json::Str(s) => s.as_str(),
                        _ => "?",
                    }
                ))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    let operacion = campo("operacion");
    let esquema_cambiado = campo("esquema_cambiado") == "true";
    let como = match operacion.as_str() {
        "creada" => "el dataset nace".to_string(),
        "refrescada" => {
            format!("refrescada: {leidas} filas fundidas sobre las que había · snapshot nuevo")
        }
        _ if rehacer => "rehecha: sobrescrita entera · snapshot nuevo, la historia se queda".into(),
        _ => "sobrescrita entera (otro plan, u otra copia sin clave) · snapshot nuevo".into(),
    };
    let esquema_txt = if esquema_cambiado {
        "\n  el esquema de la tabla cambió con el plan"
    } else {
        ""
    };
    let mut m: BTreeMap<String, Json> = [
        ("estado", Json::s("copiada")),
        ("rehecha", Json::Bool(rehacer)),
        ("operacion", Json::s(&operacion)),
        ("metadata_location", Json::s(campo("metadata_location"))),
        ("snapshot", Json::s(campo("snapshot"))),
        ("ubicacion", Json::s(campo("ubicacion"))),
        ("dataset", Json::s(dataset)),
        ("cabecera", Json::s(huella)),
        ("plan", Json::s(plan)),
        // Procedencia: de qué árbol salió. NO está en la cabecera, y por eso
        // está aquí.
        ("bundle", Json::s(bundle)),
        ("filas", Json::Int(entero("filas"))),
        ("leidas", Json::Int(leidas as i64)),
        ("bytes", Json::Int(entero("bytes"))),
        ("ficheros", Json::Int(entero("ficheros"))),
        ("columnas", Json::Obj(columnas)),
        ("columnas_sin_estrechar", Json::Obj(sin_estrechar)),
        ("esquema_cambiado", Json::Bool(esquema_cambiado)),
        (
            "testigo",
            Json::obj([
                ("modo", Json::s(&testigo.0)),
                ("valor", Json::s(testigo.1.clone().unwrap_or_default())),
            ]),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let recogidas = recoger_dataset(recoger, dataset, &campo("metadata_location"), &mut m)?;
    Ok((
        format!(
            "copiada · {}\n  {} filas · {leidas} leidas · {} bytes · {como}{esquema_txt}{recogidas}{aviso_columnas}{aviso_tipos}",
            campo("metadata_location"),
            campo("filas"),
            campo("bytes"),
        ),
        Json::Obj(m),
    ))
}

/// **`--recoger` sobre un dataset**: expira los snapshots superados y retira
/// del bucket lo que ningún snapshot que quede nombra. Si expiró alguno hay un
/// `metadata.json` nuevo, y el puntero se mueve a él aquí mismo. Va DESPUÉS de
/// sellar y también en «ya está»: si fuera sólo al sellar, un almacén lleno de
/// snapshots viejos no se limpiaría nunca mientras nada cambiara.
///
/// La edad que se conserva la dice la tabla (`history.expire.*`, 0031 §11 ⑥)
/// y, para lo que la tabla no diga, `ORE_RECOGER_EDAD` (segundos). **Sin
/// ninguna de las dos no se expira nada**: la política vive en la tabla y en el
/// CronJob de mantenimiento (`53`), no en un defecto.
fn recoger_dataset(
    hacer: bool,
    dataset: &str,
    metadata_location: &str,
    m: &mut BTreeMap<String, ore_core::json::Json>,
) -> Result<String, String> {
    use ore_core::json::Json;
    if !hacer || metadata_location.is_empty() {
        return Ok(String::new());
    }
    let mut peticion = vec![
        ("dataset", Json::s(dataset)),
        ("metadata_location", Json::s(metadata_location)),
    ];
    if let Some(s) = std::env::var("ORE_RECOGER_EDAD")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
    {
        peticion.push(("edad_ms", Json::s((s * 1000).to_string())));
    }
    let g = almacen("recoger", &Json::obj(peticion).jcs(), None)?;
    let n = |k: &str| campo_de(&g, k).unwrap_or_else(|| "0".into());
    if let Some(ml) = campo_de(&g, "metadata_location")
        && ml != metadata_location
    {
        m.insert("metadata_location".into(), Json::s(ml));
    }
    let (expirados, ficheros) = (n("expirados"), n("ficheros"));
    Ok(if expirados == "0" && ficheros == "0" {
        String::new()
    } else {
        format!(
            "\n  recogidos {expirados} snapshot(s) superado(s) y {ficheros} fichero(s) que nadie nombraba"
        )
    })
}

/// El nombre del dataset de una vista en el bucket: `copias/<paquete>_<vista>`
/// (bajo `ore/v2/`). El mismo nombre que su puntero en el árbol, sin `.json`.
pub(crate) fn dataset_de(qn: &str) -> String {
    format!("copias/{}", qn.replace('.', "_"))
}

/// **Lo que hace falta para leer una copia hecha**: su puntero. Un dataset
/// (`metadata_location`) o, mientras quede alguno, un sobre heredado
/// (`clave`). Lo que `ore ask` e `ore invoke` le piden a `ore-store leer`.
#[derive(Debug, Clone)]
pub(crate) struct Puntero {
    pub vista: String,
    pub dataset: String,
    pub metadata_location: Option<String>,
    pub clave: Option<String>,
}

impl Puntero {
    /// El puntero de una vista **si su copia está hecha** (`copiada` o
    /// `al-dia`, y con algo que leer); si no, por qué no.
    pub fn hecho(raiz: &Path, vista: &str) -> Result<Puntero, String> {
        let ruta = raiz
            .join("copias")
            .join(format!("{}.json", vista.replace('.', "_")));
        let n = std::fs::read_to_string(&ruta)
            .ok()
            .and_then(|t| ore_core::parse::parse(&t).ok())
            .ok_or_else(|| {
                format!(
                    "su copia no está hecha: no hay `{}` (ore materialize)",
                    ruta.display()
                )
            })?;
        let estado = campo_de(&n, "estado").unwrap_or_default();
        let p = Puntero {
            vista: vista.to_string(),
            dataset: campo_de(&n, "dataset").unwrap_or_else(|| dataset_de(vista)),
            metadata_location: campo_de(&n, "metadata_location"),
            clave: campo_de(&n, "clave"),
        };
        if !matches!(estado.as_str(), "copiada" | "al-dia")
            || (p.metadata_location.is_none() && p.clave.is_none())
        {
            return Err(format!("su copia no está: el informe dice `{estado}`"));
        }
        Ok(p)
    }

    /// Cómo se nombra: el `metadata_location` del dataset, o la clave del sobre.
    pub fn nombre(&self) -> &str {
        self.metadata_location
            .as_deref()
            .or(self.clave.as_deref())
            .unwrap_or("")
    }

    /// La petición de `ore-store leer`.
    pub fn peticion_leer(&self) -> String {
        use ore_core::json::Json;
        match &self.metadata_location {
            Some(ml) => Json::obj([
                ("dataset", Json::s(&self.dataset)),
                ("metadata_location", Json::s(ml)),
            ])
            .jcs(),
            None => Json::obj([("clave", Json::s(self.clave.as_deref().unwrap_or("")))]).jcs(),
        }
    }

    /// Cómo se cuenta en una cabecera: `{de, metadata_location}` o `{de, clave}`.
    pub fn como_json(&self) -> ore_core::json::Json {
        use ore_core::json::Json;
        let mut pares = vec![("de", Json::s(&self.vista))];
        match &self.metadata_location {
            Some(ml) => pares.push(("metadata_location", Json::s(ml))),
            None => pares.push(("clave", Json::s(self.clave.as_deref().unwrap_or("")))),
        }
        Json::obj(pares)
    }
}

/// El puntero de una vista, si está: `<dir>/<paquete>_<vista>.json`.
pub(crate) fn leer_puntero(dir: &Path, qn: &str) -> Option<ore_core::parse::Node> {
    let ruta = dir.join(format!("{}.json", qn.replace('.', "_")));
    std::fs::read_to_string(ruta)
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok())
}

/// Un campo escalar de un nodo, si está y no está vacío.
pub(crate) fn campo_de(n: &ore_core::parse::Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// **El puntero de la copia** (`copias/<paquete>_<vista>.json`, o `--informe
/// DIR`): un JSON por vista, y es dos cosas a la vez. **El estado** —qué
/// `metadata.json` es el vigente, con qué cabecera se selló, hasta qué
/// testigo—, que `ore` lee antes de leer una fila; y **el informe** para quien
/// no alcanza ni el origen ni el almacén —`ore-serve`, la consola, el puesto—:
/// cuántas filas, qué columnas, con qué testigo. Lo escribe el Job de la celda
/// y lo empuja al árbol, y ahí el commit dice cuándo y quién: es el catálogo.
///
/// `ore-store recoger-huerfanas` con los datasets que el árbol reclama y los
/// sobres heredados que algún puntero todavía nombra.
fn recoger_huerfanas(datasets: &[String], claves: &[String]) -> Result<String, String> {
    use ore_core::json::Json;
    let entrada = Json::obj([
        (
            "datasets",
            Json::Arr(datasets.iter().map(Json::s).collect()),
        ),
        ("claves", Json::Arr(claves.iter().map(Json::s).collect())),
        ("seco", Json::Bool(false)),
    ])
    .jcs();
    let r = almacen("recoger-huerfanas", &entrada, None)?;
    let n = |k: &str| campo_de(&r, k).unwrap_or_else(|| "0".into());
    Ok(format!(
        "huérfanas: {} dataset(s) que ningún puntero reclama ({} objeto(s)) y {} objeto(s) heredado(s) de `ore/v1/`, fuera del almacén ({} datasets vigentes)",
        n("huerfanos"),
        n("objetos"),
        n("heredados"),
        n("datasets")
    ))
}

/// Los punteros `copias/<paquete>_<vista>.json` de vistas que ya no están en
/// el árbol, fuera: un puntero de nadie en el árbol es tan engañoso como un
/// dataset de nadie en el almacén.
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
            && std::fs::remove_file(&ruta).is_ok()
        {
            println!("  informe de `{v}` retirado: la vista ya no está en el árbol");
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
    // Una pasada que falla NO borra el puntero: el dataset de la anterior
    // sigue ahí y sigue siendo cierto hasta su marca. Se conserva lo que decía
    // y se le pone encima el estado de hoy y su motivo.
    if m.get("estado") == Some(&Json::s("error"))
        && let Ok(previo) = std::fs::read_to_string(&ruta)
        && let Ok(n) = ore_core::parse::parse(&previo)
        && let Json::Obj(mut anterior) = Json::de_node(&n)
    {
        anterior.remove("motivo");
        for (k, v) in m {
            anterior.insert(k, v);
        }
        m = anterior;
    }
    m.insert("vista".into(), Json::s(qn));
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

/// **La raíz de una vista cuando es una tabla del lago** (0031 «(d)»): lo que
/// `ore-read-<tipo> testigo` contestaría, sacado de donde está —el puntero
/// `datasets/<objeto>.json` que `write()` (o `ore datasets --commit`) dejó en
/// el árbol— y lo que `ore-store copiar` necesita para abrirla.
struct OrigenDelLago {
    /// `(modo, valor)`, como el de un driver: `snapshot` con el id del snapshot
    /// vigente de la tabla, o `none`.
    testigo: (String, Option<String>),
    metadata_location: String,
    dataset: String,
}

impl OrigenDelLago {
    /// El campo `origen` de la petición de `copiar`, ya con la coma final para
    /// entrar en la cabecera: la tabla, la proyección y los filtros de la vista.
    fn origen_json(&self, r: &vistas::Raiz) -> Result<String, String> {
        use ore_core::json::Json;
        // La misma petición que un driver recibiría (`proyeccion`, `filtros`),
        // con las mismas negativas: un `where` con varios valores no cabe.
        let p = peticion("", r, None, None, None, false)?;
        let n = ore_core::parse::parse(&p).map_err(|e| format!("{e:?}"))?;
        let mut o = vec![
            ("dataset", Json::s(&self.dataset)),
            ("metadata_location", Json::s(&self.metadata_location)),
        ];
        if let Some((_, f)) = n.get("filtros") {
            o.push(("filtros", Json::de_node(f)));
        }
        if let Some((_, p)) = n.get("proyeccion") {
            o.push(("proyeccion", Json::de_node(p)));
        }
        Ok(format!("\"origen\":{},", Json::obj(o).jcs()))
    }
}

fn origen_del_lago(
    pkg: &Package,
    raiz_pkg: &Path,
    v: &Loaded,
    r: &vistas::Raiz,
) -> Result<OrigenDelLago, String> {
    let dataset = format!("datasets/{}", r.objeto);
    let ruta = raiz_pkg.join(format!("{dataset}.json"));
    let nombre = r.tabla.as_deref().unwrap_or(&r.objeto);
    let puntero = std::fs::read_to_string(&ruta)
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok())
        .ok_or_else(|| {
            format!(
                "`{nombre}` es una tabla del lago y no tiene puntero (`{}`): nadie la escribió todavía",
                ruta.display()
            )
        })?;
    let metadata_location = campo_de(&puntero, "metadata_location").ok_or_else(|| {
        format!(
            "el puntero de `{nombre}` (`{}`) no dice `metadata_location`",
            ruta.display()
        )
    })?;
    // La marca sale de la gramática, como en `testigo()`. Una tabla del lago
    // se fecha por snapshot —cada escritura es uno— o no se fecha; un `log` o
    // un `field` sobre ella no tienen quién los conteste.
    let testigo = match crate::registro::marca_de(pkg, v) {
        ore_view::Marca::Ninguna => ("none".to_string(), None),
        ore_view::Marca::Instantanea => (
            "snapshot".to_string(),
            campo_de(&puntero, "snapshot").filter(|s| s != "0"),
        ),
        otra => {
            return Err(format!(
                "`{nombre}` es una tabla del lago y declara `witness: {}`: una tabla del lago se fecha con `snapshot` (o `none`)",
                match otra {
                    ore_view::Marca::Registro => "log".to_string(),
                    ore_view::Marca::Campo(c) => format!("field ({c})"),
                    _ => "?".to_string(),
                }
            ));
        }
    };
    Ok(OrigenDelLago {
        testigo,
        metadata_location,
        dataset,
    })
}

/// La cabecera de la copia, en JSON canónico y en **una** línea, que es lo que
/// el protocolo del almacén espera y lo que el dataset guarda como propiedad
/// de su snapshot. Su digest va al puntero (`cabecera`), y es lo que decide
/// «ya está».
///
/// **Sin el bundle** (2026-09-18). Iba, y era el digest del árbol entero: un
/// commit en cualquier paquete cambiaba la cabecera de todas las vistas y se
/// releían los orígenes sin que nada suyo cambiara (16 de 19 commits en
/// `victor`; `medida-lo-que-parece-roto.py` §4). Plan, esquema, clave, testigo
/// y conducto ya nombran lo que la copia contiene; de qué árbol salió es
/// procedencia y va al puntero (`bundle`), no a la llave.
fn cabecera(
    plan: &str,
    esq: &BTreeMap<String, ore_core::types::Type>,
    testigo: &(String, Option<String>),
    clave: &[String],
) -> String {
    use ore_core::json::Json;
    let t = match &testigo.1 {
        Some(v) => Json::obj([("modo", Json::s(&testigo.0)), ("valor", Json::s(v))]),
        None => Json::obj([("modo", Json::s(&testigo.0))]),
    };
    Json::obj([
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
