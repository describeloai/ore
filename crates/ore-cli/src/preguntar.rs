//! `ore ask` — **ejecutar la pregunta** ([ADR 0030](../../../docs/decisions/0030-el-arbol-en-el-editor.md) W1).
//!
//! Una vista es una pregunta sobre un hecho, y hasta aquí nadie la contestaba:
//! `ore view` compila el plan y dice qué copia lo contesta, `ore materialize`
//! copia, y las filas no salían por ningún sitio. Esto las saca, **en la celda
//! y sin abrir el origen**:
//!
//! | | qué | quién |
//! |---|---|---|
//! | 1 | compilar la vista: el plan, con identidad | `ore` (`vista::cuerpo`, el catálogo expande la cadena) |
//! | 2 | decidir quién contesta: de las copias registradas, cuál sirve este plan y con qué compensación | `ore` (View Matcher, `registro::cotejos`) |
//! | 3 | traer la copia por su nombre | `ore-store-<r2\|gcs> leer` (el nombre lo dejó `datasets/<paquete>_<nombre>.json`) |
//! | 4 | tipar las filas por la cabecera | `ore-view::hoja` |
//! | 5 | ejecutar el plan reescrito sobre ellas | `ore-view::delta_compiler::recomputar` |
//!
//! # Lo que decide, y lo que no
//!
//! **No abre un socket** —la figura de `materialize` e `invoke`: las filas
//! entran por el stdout de un programa— y **no escribe nada**: la respuesta va
//! a stdout, cabecera y filas, y se acaba. Una pregunta que merezca quedarse se
//! materializa (④ del espectro W1), que es otro verbo.
//!
//! **No inventa una copia.** Si ninguna contesta —la vista no tiene copia, ni
//! la tiene su tabla, ni una vecina cuyo plan la implique— se niega con los
//! motivos del cotejo, uno por candidata. Lo que el matcher no sabe demostrar
//! no se sirve: afirmar una implicación que no se prueba es dar filas que no
//! debían salir. Y prefiere **la copia propia** cuando la hay: es la que su
//! informe describe y la que el resto del árbol ya mira.
//!
//! **El límite es del plan.** `--limite N` envuelve el plan en `Limita`, así
//! que lo que se recorta es la respuesta y no lo que se lee: leer menos filas
//! de la copia daría otra pregunta (un agregado sobre la mitad de las filas
//! no es el agregado).
//!
//! # La salida
//!
//! La primera línea es la cabecera: la vista, la copia que contestó (de quién
//! y con qué clave), el digest del plan, cuántos conyuntos de compensación
//! hubo, las columnas con su tipo, las filas de la respuesta, las leídas de la
//! copia y el trabajo (filas miradas, 0014). Debajo, una fila por línea como
//! objeto plano: cadena, entero, booleano, y el decimal **como cadena de
//! dígitos** —JSON no tiene decimal exacto y la cabecera ya dice que lo es—.
//! Una fila con peso `w` sale `w` veces: es un multiconjunto.

use std::collections::BTreeMap;
use std::path::Path;

use ore_core::json::Json;
use ore_core::link::Loaded;
use ore_view::delta_compiler::{Zset, recomputar_contando};
use ore_view::view_matcher::{NoContesta, Rewrite};
use ore_view::{Catalogo, Clasificacion, Nodo, Valor, Vista, esquema};

use crate::lector;
use crate::materializar::{Puntero, programa_del_almacen};
use crate::vista::Vistas as _;

pub struct Opciones<'a> {
    pub vista: &'a str,
    pub limite: Option<u64>,
    /// Decidir sin traer: el plan, quién contesta y con qué; ninguna fila.
    pub seco: bool,
}

type Fallo = (u8, String);

pub fn preguntar(path: &Path, op: &Opciones) -> std::process::ExitCode {
    match correr(path, op) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err((codigo, mensaje)) => {
            eprintln!("error: {mensaje}");
            std::process::ExitCode::from(codigo)
        }
    }
}

fn correr(path: &Path, op: &Opciones) -> Result<(), Fallo> {
    // ── ① El árbol, y que compile donde la vista vive ────────────────────────
    if !path.is_dir() {
        return Err((66, format!("`{}` no es un directorio", path.display())));
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(path);
    // Se pregunta a una vista, o a un dataset (0033): los dos tienen plan.
    // Si una vista y su dataset se llaman igual, se pregunta a la vista: es
    // la pregunta sobre lo que se tiene, y lee de su dataset.
    let v: &Loaded = pkg
        .view(op.vista)
        .or_else(|| pkg.dataset(op.vista))
        .ok_or_else(|| {
            (
                65,
                format!(
                    "no hay ninguna `View` ni `Dataset` `{}` en el árbol",
                    op.vista
                ),
            )
        })?;
    let mio = crate::materializar::paquete_del_fichero(path, &v.path);
    for d in ore_core::validate_package(path) {
        if d.code == ore_core::Code::Oos2013 {
            continue;
        }
        let suyo = crate::materializar::paquete_del_fichero(path, &d.file);
        if suyo.is_none() || suyo == mio {
            return Err((
                65,
                format!(
                    "el árbol no compila donde esta vista vive:\n{}",
                    d.render(path)
                ),
            ));
        }
    }

    // ── ② El plan, y quién lo contesta ───────────────────────────────────────
    let tipos = crate::vista::tipos_de_raiz(&pkg);
    let catalogo = Catalogo::con(pkg.of_view().iter().filter_map(|d| {
        Some(Vista::nueva(
            &crate::vista::nodo_de(d)?,
            crate::vista::cuerpo(&pkg, d, &tipos),
        ))
    }));
    let plan = catalogo
        .expandir(&crate::vista::nodo_de(v).unwrap_or_default())
        .map_err(|e| {
            (
                65,
                format!(
                    "el plan de `{}` no se expande: {}",
                    op.vista,
                    e.como_texto()
                ),
            )
        })?;
    let lat = ore_core::flow::lattices(&pkg);
    let clasificacion = Clasificacion {
        reticulos: lat.clone(),
        de_raiz: crate::vista::etiquetas_de_raiz(&pkg, &lat),
    };
    let inventario = crate::registro::construir(&pkg, &catalogo, &tipos);
    let restricciones = crate::registro::restricciones(&pkg);
    let cotejos = crate::registro::cotejos(&inventario, &plan, &clasificacion, &restricciones);
    let (de, rw, puntero) = elegir(op.vista, cotejos, |nombre| Puntero::hecho(path, nombre))?;

    // El plan que se ejecuta: el reescrito sobre la copia, y el límite encima.
    let ejecutable = match op.limite {
        Some(n) => Nodo::Limita {
            entrada: Box::new(rw.plan.clone()),
            n,
        },
        None => rw.plan.clone(),
    };
    let columnas: BTreeMap<String, Json> = esquema(&rw.plan)
        .map_err(|e| {
            (
                70,
                format!("el plan reescrito no cuadra: {}", e.como_texto()),
            )
        })?
        .into_iter()
        .map(|(c, t)| (c, Json::s(t.to_string())))
        .collect();
    let hoja =
        hoja_de(&rw.plan).ok_or((70, "el plan reescrito no lee de una copia".to_string()))?;

    eprintln!("{}", op.vista);
    eprintln!(
        "  contesta `{de}`{} · {} · copia {}",
        if de == op.vista { " (su copia)" } else { "" },
        match rw.compensation.len() {
            0 => "sin compensación".to_string(),
            1 => "1 conyunto de compensación".to_string(),
            n => format!("{n} conyuntos de compensación"),
        },
        puntero.nombre()
    );
    if op.seco {
        println!("{}", cabecera(op, &puntero, &rw, &columnas, 0, 0, 0).jcs());
        return Ok(());
    }

    // ── ④ Traer y tipar ──────────────────────────────────────────────────────
    let programa = programa_del_almacen().map_err(|e| (78, e))?;
    let leido = lector::ejecutar(&programa, &["leer".into()], Some(&puntero.peticion_leer()))
        .map_err(|e| (69, con_ayuda(e)))?;
    let (_, base, leidas) = ore_view::hoja::de_leer(&leido).map_err(|e| (69, e))?;

    // ── ⑤ Ejecutar ───────────────────────────────────────────────────────────
    let bases: BTreeMap<(String, String), Zset> = [(hoja, base)].into_iter().collect();
    let (z, trabajo) = recomputar_contando(&ejecutable, &bases).map_err(|e| {
        (
            70,
            format!("la pregunta no se pudo ejecutar: {}", e.como_texto()),
        )
    })?;
    let filas: u64 = z.presentes().map(|(_, w)| w as u64).sum();
    println!(
        "{}",
        cabecera(op, &puntero, &rw, &columnas, filas, leidas, trabajo).jcs()
    );
    for (f, w) in z.presentes() {
        let linea = Json::Obj(f.iter().map(|(k, v)| (k.clone(), plano(v))).collect()).jcs();
        for _ in 0..w {
            println!("{linea}");
        }
    }
    eprintln!("  {filas} filas · {leidas} leídas · {trabajo} miradas");
    Ok(())
}

/// **Quién contesta.** De las copias cuyo plan sirve a la pregunta, la propia
/// si está hecha; si no, la primera hecha. Una que contesta y no está hecha
/// se dice —es lo accionable: copiarla—; sin ninguna que conteste, los
/// motivos de todas.
fn elegir(
    vista: &str,
    cotejos: Vec<(String, Result<Rewrite, NoContesta>)>,
    hecha: impl Fn(&str) -> Result<Puntero, String>,
) -> Result<(String, Rewrite, Puntero), Fallo> {
    let contestan: Vec<(&String, &Rewrite)> = cotejos
        .iter()
        .filter_map(|(n, r)| r.as_ref().ok().map(|rw| (n, rw)))
        .collect();
    let propia = contestan.iter().find(|(n, _)| *n == vista);
    let ajenas = contestan.iter().filter(|(n, _)| *n != vista);
    let mut sin_hacer = Vec::new();
    for (n, rw) in propia.into_iter().chain(ajenas) {
        match hecha(n) {
            Ok(p) => return Ok(((*n).clone(), (*rw).clone(), p)),
            Err(porque) => sin_hacer.push(format!("  `{n}` la contesta, pero {porque}")),
        }
    }
    if !sin_hacer.is_empty() {
        return Err((
            65,
            format!(
                "ninguna copia hecha contesta a `{vista}`:
{}
  Copia (ore materialize) y vuelve a preguntar",
                sin_hacer.join(
                    "
"
                )
            ),
        ));
    }
    let mut msg = format!(
        "ningún dataset contesta a `{vista}`: no hay un dataset en su cadena, y ninguno de los registrados sirve su plan"
    );
    for (n, r) in &cotejos {
        if let Err(e) = r {
            msg.push_str(&format!(
                "
  `{n}` no la contesta"
            ));
            for l in e.como_texto().lines() {
                msg.push_str(&format!(
                    "
    {}",
                    l.trim()
                ));
            }
        }
    }
    msg.push_str(
        "
  Declara un `Dataset` con `from` sobre ella —o sobre una que la implique— y copia (ore materialize)",
    );
    Err((65, msg))
}

/// La hoja del plan reescrito: la tabla donde vive la copia.
fn hoja_de(n: &Nodo) -> Option<(String, String)> {
    match n {
        Nodo::Lee(l) => Some((l.datasource.clone(), l.objeto.clone())),
        Nodo::Referencia(_) => None,
        Nodo::Proyecta { entrada, .. }
        | Nodo::Filtra { entrada, .. }
        | Nodo::Agrupa { entrada, .. }
        | Nodo::Limita { entrada, .. }
        | Nodo::Distingue(entrada) => hoja_de(entrada),
        Nodo::Une { izquierda, .. } => hoja_de(izquierda),
        Nodo::Unifica(v) => v.iter().find_map(hoja_de),
    }
}

#[allow(clippy::too_many_arguments)]
fn cabecera(
    op: &Opciones,
    puntero: &Puntero,
    rw: &Rewrite,
    columnas: &BTreeMap<String, Json>,
    filas: u64,
    leidas: u64,
    trabajo: u64,
) -> Json {
    Json::obj([
        ("vista", Json::s(op.vista)),
        ("copia", puntero.como_json()),
        ("plan", Json::s(rw.plan.digest())),
        ("compensacion", Json::Int(rw.compensation.len() as i64)),
        ("columnas", Json::Obj(columnas.clone())),
        (
            "limite",
            op.limite.map_or(Json::Bool(false), |n| Json::Int(n as i64)),
        ),
        ("filas", Json::Int(filas as i64)),
        ("leidas", Json::Int(leidas as i64)),
        ("trabajo", Json::Int(trabajo as i64)),
    ])
}

/// Un valor como escalar de JSON. El decimal va como cadena de dígitos: JSON
/// no tiene decimal exacto, y la cabecera ya dice el tipo de la columna.
fn plano(v: &Valor) -> Json {
    match v {
        Valor::Cadena(s) | Valor::Decimal(s) => Json::s(s),
        Valor::Entero(n) => Json::Int(*n),
        Valor::Booleano(b) => Json::Bool(*b),
    }
}

fn con_ayuda(f: lector::Fallo) -> String {
    let mut s = f.mensaje;
    for l in f.ayuda {
        s.push('\n');
        s.push_str(&l);
    }
    s
}
