//! `ore review` — la cara interactiva de una cola que ya existía.
//!
//! # Contestar no edita: vuelve a inducir
//!
//! La tentación era escribir sobre los ficheros ya emitidos —abrir
//! `entities/Clientes.yaml`, sustituir el comentario de la clave que falta por
//! un `primaryKey`— y es la vía equivocada por dos motivos que se ven en cuanto
//! se prueba con un catálogo sucio:
//!
//! 1. **Hay respuestas que no caben en una edición local.** Resolver una
//!    colisión de nombres crea DOS entidades donde no había ninguna; unir una
//!    familia fechada borra tres ficheros y escribe uno con tres bindings. Eso no
//!    es retocar un documento: es inducir otro.
//! 2. **Un documento retocado deja de ser derivable.** Todo lo que el inductor
//!    garantiza —el escapado de una descripción, el orden de las columnas del
//!    origen, cuándo hace falta `toKey`— habría que volver a garantizarlo aquí,
//!    en otro sitio y con otro código.
//!
//! Así que `review` hace lo único que conserva las dos cosas: **vuelve a inducir
//! el mismo catálogo con las decisiones tomadas.** Lo que sale de aquí es siempre
//! `inducir(catálogo, respuestas)`, nunca una inducción retocada, y eso hace que
//! revisar sea reproducible — las mismas respuestas dan el mismo paquete, byte a
//! byte, sin volver a tocar la fuente.
//!
//! El precio está dicho y no disimulado: **una edición a mano entre `discover` y
//! `review` se pierde.** Por eso el informe dice qué ficheros reescribe y cuáles
//! retira, en vez de dejarlo en que el directorio quede distinto.
//!
//! # Por qué no vuelve a leer la fuente
//!
//! `discover --source` deja el catálogo que leyó en `discover.catalog.json`, y
//! `review` lee ese. No es un caché: es lo que hace que **revisar sea puro** —
//! sin red, sin credenciales y sin driver, igual que el inductor—, y de paso lo
//! que hace que `discover --source` sea reproducible como `discover --from`.
//! Contestar diez preguntas contra una fuente que cambia debajo produciría un
//! paquete que no corresponde a ninguna instantánea.
//!
//! # Y el modo no interactivo no es un extra
//!
//! `--answers` existe porque **una prueba que no corre tiene exactamente el mismo
//! aspecto que una que pasa**, y una cola que solo se puede contestar a mano no
//! se puede contestar en CI. Es el mismo vocabulario que la cola serializa: cada
//! decisión lleva su `id` y sus `options`, que son la izquierda y la derecha de
//! una línea del fichero.

use crate::inductor::{self, Clase, Decisiones, Induccion, Pendiente, Respuesta};
use crate::vocabulario::Vocabulario;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{IsTerminal, Write as _};
use std::path::Path;
use std::process::ExitCode;

const CATALOGO: &str = "discover.catalog.json";
const COLA: &str = "discover.pending.json";
/// Las respuestas dadas hasta ahora.
///
/// Que exista un fichero **no** contradice que la respuesta escriba en lo
/// inducido: no es un registro de decisiones tomadas al margen del paquete, es
/// la ENTRADA de la que el paquete sale. `paquete = inducir(catálogo,
/// respuestas)` con los dos ficheros a la vista, y borrar una línea devuelve su
/// decisión a la cola.
///
/// Sin él, revisar cincuenta tablas en dos sentadas sería imposible: la segunda
/// pasada volvería a inducir con solo lo contestado en ella y desharía la
/// primera, en silencio y sobre ficheros que ya parecían buenos.
const RESPUESTAS: &str = "discover.answers.json";
/// Los directorios que el inductor gobierna por completo. Un `.yaml` que esté
/// aquí y que la inducción nueva no produzca es un resto, y un resto miente.
///
/// `tables` y `views` entraron con v1alpha8, y **se midió lo que costaba que no
/// estuvieran**: contestar `omitir` sobre una vista del origen retiraba su
/// entidad y dejaba su tabla y su vista en el paquete. Validaba —una vista sin
/// entidad es legal— así que el resto no daba ningún síntoma: el paquete
/// seguía afirmando un objeto que alguien había dicho que no entra.
///
/// `bindings` se queda aunque el inductor ya no escriba ninguno, y por eso
/// mismo: en un paquete descubierto con la versión anterior, la primera
/// revisión retira los que quedaron. Sacarlo de aquí los dejaría al lado de las
/// tablas que los sustituyen, diciendo lo mismo dos veces.
const GOBERNADOS: [&str; 5] = ["entities", "bindings", "concepts", "tables", "views"];

pub struct Fallo {
    pub codigo: u8,
    pub mensaje: String,
    pub ayuda: Vec<String>,
}

fn fallo(codigo: u8, mensaje: impl Into<String>, ayuda: &[&str]) -> Fallo {
    Fallo {
        codigo,
        mensaje: mensaje.into(),
        ayuda: ayuda.iter().map(|s| (*s).to_string()).collect(),
    }
}

pub fn review(raiz: &Path, respuestas: Option<&Path>) -> ExitCode {
    match intentar(raiz, respuestas) {
        Ok(informe) => {
            print!("{informe}");
            ExitCode::SUCCESS
        }
        Err(f) => {
            eprintln!("error: {}", f.mensaje);
            for l in &f.ayuda {
                eprintln!("{l}");
            }
            ExitCode::from(f.codigo)
        }
    }
}

fn intentar(raiz: &Path, respuestas: Option<&Path>) -> Result<String, Fallo> {
    let ruta = raiz.join(CATALOGO);
    let texto = std::fs::read_to_string(&ruta).map_err(|e| {
        fallo(
            66, // EX_NOINPUT
            format!("no se pudo leer `{}`: {e}", ruta.display()),
            &[
                "  Es el catálogo que `ore discover --out <ruta>` deja al lado de lo que",
                "  induce. `review` no habla con la fuente —no sabe—, así que sin él no",
                "  hay nada que volver a inducir.",
            ],
        )
    })?;
    let catalogo = inductor::Catalogo::leer(&texto)
        .map_err(|m| fallo(65, format!("`{}` no analiza: {m}", ruta.display()), &[]))?;

    let paquete = nombre_del_paquete(raiz);

    // El vocabulario se lee del REPOSITORIO, no del paquete: un vocabulario
    // publicado es un paquete sin entidades que otros importan, así que mirar
    // solo aquí dejaría la séptima pregunta sin más respuesta que acuñar.
    let voc = match crate::raiz_del_repositorio(raiz) {
        Some(r) => Vocabulario::leer(&r),
        None => Vocabulario::default(),
    };

    // Lo ya contestado en pasadas anteriores, y la cola que queda con ello
    // puesto: preguntar otra vez lo que alguien ya decidió es la forma más
    // rápida de que deje de contestar.
    let mut dec = acumuladas(raiz)?;
    let antes = inductor::inducir_con(&catalogo, &paquete, &dec, &voc);

    let nuevas = match respuestas {
        Some(p) => {
            let t = std::fs::read_to_string(p)
                .map_err(|e| fallo(66, format!("no se pudo leer `{}`: {e}", p.display()), &[]))?;
            Decisiones::leer(&t).map_err(|m| {
                fallo(
                    65,
                    format!("`{}` {m}", p.display()),
                    &[
                        "  Se espera un mapa `answers` de identificador a respuesta:",
                        "",
                        "    answers:",
                        "      clave/public.log_eventos: [id_evento]",
                        "      tipo/public.pedidos.importe: \"Money<EUR, 2>\"",
                        "      vista/public.v_clientes_activos: omitir",
                        "",
                        "  Los identificadores son los `id` de `discover.pending.json`.",
                    ],
                )
            })?
        }
        None => {
            if !std::io::stdin().is_terminal() {
                return Err(fallo(
                    66,
                    "no hay terminal donde preguntar",
                    &[
                        "  `ore review` pregunta, y sin terminal no hay a quién.",
                        "  Contesta en diferido: `ore review <ruta> --answers <fichero>`.",
                    ],
                ));
            }
            preguntar(&antes.pendientes)?
        }
    };

    if nuevas.is_empty() {
        return Ok(informe_sin_cambios(&antes));
    }
    let cuantas = nuevas.len();
    dec.fundir(nuevas);

    // Y aquí está todo: la revisión es la misma inducción con las decisiones
    // tomadas. Nada de lo de abajo retoca un documento.
    let despues = inductor::inducir_con(&catalogo, &paquete, &dec, &voc);
    let retirados = escribir(raiz, &despues, &dec)?;

    Ok(informe(&antes, &despues, cuantas, &retirados))
}

/// Lo contestado en pasadas anteriores. Que no haya fichero es lo normal la
/// primera vez y no es un error; que lo haya y no analice sí, porque entonces el
/// paquete de al lado salió de algo que ya no se puede volver a leer.
fn acumuladas(raiz: &Path) -> Result<Decisiones, Fallo> {
    let ruta = raiz.join(RESPUESTAS);
    let Ok(t) = std::fs::read_to_string(&ruta) else {
        return Ok(Decisiones::default());
    };
    Decisiones::leer(&t).map_err(|m| {
        fallo(
            65,
            format!("`{}` {m}", ruta.display()),
            &["  Lo escribe `ore review`. Si se editó a mano, revisa el mapa `answers`."],
        )
    })
}

/// El nombre del paquete lo dice **el paquete**, no el directorio.
///
/// `discover --name` pudo ponerle uno distinto del nombre de la carpeta, y
/// derivarlo otra vez de la carpeta renombraría el paquete al revisarlo — que es
/// una de esas cosas que no rompen nada hasta que rompen todo.
fn nombre_del_paquete(raiz: &Path) -> String {
    let del_directorio = || {
        raiz.file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "inducido".into())
    };
    let Ok(t) = std::fs::read_to_string(raiz.join("package.yaml")) else {
        return del_directorio();
    };
    let Ok(arbol) = ore_core::parse::parse(&t) else {
        return del_directorio();
    };
    arbol
        .get("metadata")
        .and_then(|(_, m)| m.get("name"))
        .and_then(|(_, v)| v.as_str())
        .map(String::from)
        .unwrap_or_else(del_directorio)
}

// ── Escribir lo inducido, y retirar lo que sobra ────────────────────────────

/// Escribe la inducción nueva y devuelve lo que retiró.
///
/// Retirar no es limpieza: si una respuesta dijo que una vista no es una
/// entidad, dejar su `entities/…yaml` de la pasada anterior haría que el paquete
/// siguiera afirmándola. Se borra **solo** dentro de los directorios que el
/// inductor gobierna entero, y se dice cuál.
fn escribir(raiz: &Path, ind: &Induccion, dec: &Decisiones) -> Result<Vec<String>, Fallo> {
    let mut retirados = Vec::new();
    let nuevos: BTreeSet<&String> = ind.ficheros.keys().collect();
    for dir in GOBERNADOS {
        let Ok(entradas) = std::fs::read_dir(raiz.join(dir)) else {
            continue;
        };
        for e in entradas.flatten() {
            let ruta = e.path();
            if ruta.extension().and_then(|x| x.to_str()) != Some("yaml") {
                continue;
            }
            let Some(base) = ruta.file_name().and_then(|x| x.to_str()) else {
                continue;
            };
            let rel = format!("{dir}/{base}");
            if !nuevos.contains(&rel) {
                std::fs::remove_file(&ruta).map_err(|e| {
                    fallo(
                        73,
                        format!("no se pudo retirar `{}`: {e}", ruta.display()),
                        &[],
                    )
                })?;
                retirados.push(rel);
            }
        }
    }

    crate::escribir_paquete(ind, raiz).map_err(|(codigo, mensaje)| fallo(codigo, mensaje, &[]))?;

    // Y la cola, que ahora es más corta. Dejarla como estaba sería lo peor de
    // las dos opciones: un fichero que dice que quedan catorce decisiones al
    // lado de un paquete donde no queda ninguna.
    let cola = ruta_cola(raiz);
    std::fs::write(&cola, self::cola(ind)).map_err(|e| {
        fallo(
            73,
            format!("no se pudo escribir `{}`: {e}", cola.display()),
            &[],
        )
    })?;

    let dadas = raiz.join(RESPUESTAS);
    std::fs::write(&dadas, dec.json().pretty()).map_err(|e| {
        fallo(
            73,
            format!("no se pudo escribir `{}`: {e}", dadas.display()),
            &[],
        )
    })?;
    Ok(retirados)
}

// ── Los formularios ─────────────────────────────────────────────────────────

/// La forma de una respuesta. Tres, y son las tres de `Respuesta`: preguntar es
/// rellenar una de ellas.
enum Forma {
    /// Una palabra: un tipo, un concepto, `si`, `no`, `omitir`.
    Palabra,
    /// Varias columnas: una clave primaria.
    Columnas,
    /// Un nombre por cada sujeto: una colisión.
    Nombres,
    /// `eje: nivel`, una o varias: la clasificación de un concepto.
    Etiquetas,
}

/// El formulario de cada clase.
///
/// El `match` es exhaustivo a propósito: una clase de pregunta nueva **no
/// compila** hasta que alguien decide cómo se contesta. Es la única forma de que
/// el inductor no pueda estrenar una pregunta que nadie sabe responder.
fn forma(clase: Clase) -> Forma {
    match clase {
        Clase::Colision => Forma::Nombres,
        Clase::Clasificacion => Forma::Etiquetas,
        Clase::Clave => Forma::Columnas,
        Clase::Tipo
        | Clase::Vacio
        | Clase::Vista
        | Clase::Filas
        | Clase::Concepto
        | Clase::Relacion
        | Clase::Familia
        | Clase::Dueno => Forma::Palabra,
    }
}

/// Lo que se pide, dicho en la voz de cada clase. Un `¿Cuál?` genérico haría que
/// las nueve preguntas se parecieran, y no se parecen en nada.
fn peticion(clase: Clase) -> &'static str {
    match clase {
        Clase::Colision => "Un nombre de entidad para cada una",
        Clase::Clave => "Qué columnas forman la clave",
        Clase::Tipo => "Qué tipo de OOS es, o `omitir` para dejar la columna fuera",
        Clase::Vacio => "`omitir` confirma que esta tabla no entra en el paquete",
        Clase::Vista => "`entidad` si esta vista ES la entidad; `omitir` si es un informe",
        Clase::Filas => "`mantener` si está viva y vacía; `omitir` si es un resto",
        Clase::Concepto => "El nombre del concepto que comparten, o `no`",
        Clase::Relacion => "`si` si es una relación de verdad; `no` si es coincidencia",
        Clase::Familia => "`separadas`, `omitir`, o la columna de tiempo para unirlas",
        Clase::Dueno => "Quién responde por este paquete: `team:<handle>` o `user:<handle>`",
        Clase::Clasificacion => {
            "Cómo se clasifica: `<eje>: <nivel>`, o `sin_clasificar` si no es sensible"
        }
    }
}

fn preguntar(pendientes: &[Pendiente]) -> Result<Decisiones, Fallo> {
    let mut dec = Decisiones::default();
    if pendientes.is_empty() {
        return Ok(dec);
    }
    println!(
        "  {} decisiones. Enter deja una sin contestar y sigue viva.\n",
        pendientes.len()
    );
    for (i, p) in pendientes.iter().enumerate() {
        println!(
            "  [{}/{}] {} — {}",
            i + 1,
            pendientes.len(),
            p.sujeto,
            p.que
        );
        println!("  {}", p.porque);
        println!("  {}", p.id);
        if !p.opciones.is_empty() && !matches!(forma(p.clase), Forma::Nombres) {
            println!("  · {}", p.opciones.join("  · "));
        }
        match forma(p.clase) {
            Forma::Nombres => {
                let mut m = std::collections::BTreeMap::new();
                for sujeto in &p.opciones {
                    let sugerido = sugerencia(sujeto);
                    let l = leer(&format!("  {sujeto} → [{sugerido}] "))?;
                    let l = l.trim();
                    if l.eq_ignore_ascii_case(inductor::OMITIR) {
                        dec.responder(p.id.clone(), Respuesta::Palabra(inductor::OMITIR.into()));
                        m.clear();
                        break;
                    }
                    m.insert(
                        sujeto.clone(),
                        if l.is_empty() {
                            sugerido
                        } else {
                            l.to_string()
                        },
                    );
                }
                if !m.is_empty() {
                    dec.responder(p.id.clone(), Respuesta::Mapa(m));
                }
            }
            Forma::Columnas => {
                let l = leer(&format!("  {}\n> ", peticion(p.clase)))?;
                let cols = elegidas(l.trim(), &p.opciones);
                if !cols.is_empty() {
                    dec.responder(p.id.clone(), Respuesta::Lista(cols));
                } else if l.trim().eq_ignore_ascii_case(inductor::OMITIR) {
                    dec.responder(p.id.clone(), Respuesta::Palabra(inductor::OMITIR.into()));
                }
            }
            Forma::Etiquetas => {
                let l = leer(&format!("  {}\n> ", peticion(p.clase)))?;
                let l = l.trim();
                if l.is_empty() {
                } else if l.eq_ignore_ascii_case("sin_clasificar") {
                    dec.responder(p.id.clone(), Respuesta::Palabra("sin_clasificar".into()));
                } else {
                    // Se admiten varias: un concepto puede estar clasificado en
                    // más de un eje, y obligar a contestar la pregunta una vez
                    // por retículo la partiría por donde no se parte.
                    let mut m = std::collections::BTreeMap::new();
                    for t in l.split([',', ';']).map(str::trim).filter(|t| !t.is_empty()) {
                        let elegido = numero(t, &p.opciones).unwrap_or_else(|| t.to_string());
                        if let Some((eje, nivel)) = elegido.split_once(':') {
                            m.insert(eje.trim().to_string(), nivel.trim().to_string());
                        }
                    }
                    if !m.is_empty() {
                        dec.responder(p.id.clone(), Respuesta::Mapa(m));
                    }
                }
            }
            Forma::Palabra => {
                let l = leer(&format!("  {}\n> ", peticion(p.clase)))?;
                let l = l.trim();
                if !l.is_empty() {
                    // Un número vale por su opción: escribir `Money<EUR, 2>` a
                    // mano diez veces es donde aparecen las erratas.
                    let v = numero(l, &p.opciones).unwrap_or_else(|| l.to_string());
                    dec.responder(p.id.clone(), Respuesta::Palabra(v));
                }
            }
        }
        println!();
    }
    Ok(dec)
}

/// Un nombre de entidad razonable para una tabla que colisiona: el cualificado
/// entero. No se ofrece como decisión tomada —hay que aceptarlo—, pero teclear
/// `PublicClientes` y `VentasClientes` a mano es donde salen las erratas.
fn sugerencia(tabla: &str) -> String {
    let mut s = String::new();
    for parte in tabla.split(['.', '/']).filter(|p| !p.is_empty()) {
        let mut c = parte.chars();
        if let Some(p) = c.next() {
            s.push_str(&p.to_uppercase().collect::<String>());
            s.push_str(c.as_str());
        }
    }
    s
}

/// Las columnas elegidas, por número o por nombre. Lo que no case con ninguna
/// opción se descarta: una clave con una columna inexistente apunta a nada.
fn elegidas(linea: &str, opciones: &[String]) -> Vec<String> {
    linea
        .split([' ', ',', '\t'])
        .filter(|t| !t.is_empty())
        .filter_map(|t| numero(t, opciones).or_else(|| opciones.iter().find(|o| *o == t).cloned()))
        .collect()
}

/// `2` es la segunda opción. Fuera de rango, no es un número: es lo que se
/// escribió, y puede ser un tipo paramétrico que empiece por dígito.
fn numero(t: &str, opciones: &[String]) -> Option<String> {
    let n: usize = t.parse().ok()?;
    opciones.get(n.checked_sub(1)?).cloned()
}

fn leer(prompt: &str) -> Result<String, Fallo> {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut l = String::new();
    match std::io::stdin().read_line(&mut l) {
        // Fin de entrada: quien revisa se fue. Lo contestado hasta aquí vale.
        Ok(0) => Ok(String::new()),
        Ok(_) => Ok(l),
        Err(e) => Err(fallo(74, format!("no se pudo leer la respuesta: {e}"), &[])),
    }
}

// ── El informe ──────────────────────────────────────────────────────────────

/// Una cuenta y su sustantivo. `1 decisiones` no lo escribiría nadie.
fn plural(n: usize, uno: &str, varios: &str) -> String {
    format!("{n} {}", if n == 1 { uno } else { varios })
}

fn informe_sin_cambios(antes: &Induccion) -> String {
    if antes.pendientes.is_empty() {
        return "  Sin decisiones pendientes. No hay nada que revisar.\n".to_string();
    }
    format!(
        "  Ninguna respuesta nueva: los {} ficheros siguen como estaban.\n\
         \x20 {} esperando.\n",
        antes.ficheros.len(),
        plural(
            antes.pendientes.len(),
            "decisión sigue",
            "decisiones siguen"
        )
    )
}

fn informe(antes: &Induccion, despues: &Induccion, nuevas: usize, retirados: &[String]) -> String {
    let cerradas = antes
        .pendientes
        .len()
        .saturating_sub(despues.pendientes.len());
    let mut s = format!(
        "  ✓ {}\n\
         \x20 ✓ {} · {} reescritos en el paquete\n",
        plural(nuevas, "respuesta nueva", "respuestas nuevas"),
        plural(cerradas, "decisión cerrada", "decisiones cerradas"),
        despues.ficheros.len()
    );
    for r in retirados {
        let _ = writeln!(
            s,
            "  · retirado {r} — la inducción con las respuestas puestas ya no lo produce"
        );
    }
    let conceptos = despues
        .ficheros
        .keys()
        .filter(|k| k.starts_with("concepts/"))
        .count();
    if conceptos > 0 {
        let _ = writeln!(
            s,
            "  · {conceptos} conceptos acuñados: `is` exige que existan, y ahora existen"
        );
    }

    // Una respuesta que no llega a ninguna pregunta no puede pasar por una
    // decisión tomada: se dice, con su identificador, para que se vea la errata.
    if !despues.huerfanas.is_empty() {
        s.push('\n');
        let _ = writeln!(
            s,
            "  {} sin decisión que le corresponda en este catálogo:",
            plural(despues.huerfanas.len(), "respuesta", "respuestas")
        );
        for h in &despues.huerfanas {
            let _ = writeln!(s, "  · {h}");
        }
    }

    s.push('\n');
    if despues.pendientes.is_empty() {
        s.push_str("  Sin decisiones pendientes. Todo lo que hay aquí lo decidió alguien.\n");
    } else {
        let _ = writeln!(
            s,
            "  {} esperando:\n",
            plural(
                despues.pendientes.len(),
                "decisión sigue",
                "decisiones siguen"
            )
        );
        for p in &despues.pendientes {
            let _ = writeln!(s, "  · {} — {}", p.sujeto, p.que);
            if let Some(o) = inductor::sugerencias(p) {
                let _ = writeln!(s, "    → {o}");
            }
            let _ = writeln!(s, "    {}", p.id);
        }
        s.push('\n');
    }
    s.push_str("  ore validate <ruta>   ·   lo que el compilador dice de esto\n");
    s
}

/// El fichero de la cola, reescrito. Es la misma forma que escribe `discover`,
/// porque es la misma cola: contestar no la cambia de sitio, la acorta.
pub fn cola(ind: &Induccion) -> String {
    inductor::informe_json(ind).pretty()
}

pub fn ruta_cola(raiz: &Path) -> std::path::PathBuf {
    raiz.join(COLA)
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_opcion_se_puede_elegir_por_numero_o_por_nombre() {
        let ops = vec!["id_evento".to_string(), "ts".to_string()];
        assert_eq!(elegidas("1 ts", &ops), vec!["id_evento", "ts"]);
        assert_eq!(elegidas("id_evento", &ops), vec!["id_evento"]);
        // Lo que no es ninguna columna no entra: una clave que nombra algo
        // inexistente apunta a nada.
        assert!(elegidas("no_existe 9", &ops).is_empty());
    }

    /// Un tipo paramétrico se escribe entero y no es el número de nada.
    #[test]
    fn un_tipo_escrito_a_mano_no_se_confunde_con_una_opcion() {
        let ops = vec!["String".to_string(), "Decimal".to_string()];
        assert_eq!(numero("2", &ops).as_deref(), Some("Decimal"));
        assert_eq!(numero("Money<EUR, 2>", &ops), None);
        assert_eq!(numero("7", &ops), None);
    }

    #[test]
    fn la_sugerencia_de_una_colision_cualifica_el_nombre() {
        assert_eq!(sugerencia("public.clientes"), "PublicClientes");
        assert_eq!(sugerencia("ventas.clientes"), "VentasClientes");
    }
}
