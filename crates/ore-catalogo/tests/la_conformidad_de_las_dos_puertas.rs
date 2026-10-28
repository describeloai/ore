//! **La conformidad de las dos puertas** (0079 §19.2, E1a·5c·2): el mismo guion contra `Ficheros`
//! (un árbol de git, confirmando tras cada escritura) y contra `Catalogo` (Postgres) da lo mismo,
//! operación por operación y fallo por fallo. Y otra vez contra el catálogo **por el protocolo**:
//! en memoria y con `ore-catalogo punteros` de verdad, como hijo.
//!
//! Los punteros se comparan por sus campos (`Json::de_node_fiel`), nunca por `texto`: el texto sólo
//! es comparable dentro de una implementación. La historia, por cuántas y sus `metadata_location`
//! en orden: los ids y las horas son de cada almacén.
//!
//! Necesita `ORE_CATALOGO_PRUEBAS` (ver `la_puerta_en_el_catalogo.rs`); sin ella, lo dice y no
//! mide nada. Cada prueba trabaja en su esquema y lo borra al acabar.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ore_catalogo::Catalogo;
use ore_core::json::Json;
use ore_core::punteros::protocolo::{Canal, Proceso, Servidor};
use ore_core::punteros::{
    Cambio, Contexto, De, Esperado, Fallo, Ficheros, Leido, Punteros, Que, Ramas,
};
use postgres::{Client, NoTls};

// ── El andamiaje ──────────────────────────────────────────────────────────

/// Un esquema propio, que se borra al soltarse.
struct Prueba {
    url: String,
    esquema: String,
}

impl Drop for Prueba {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.url, NoTls) {
            let _ = c.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.esquema));
        }
    }
}

fn prueba() -> Option<Prueba> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let Ok(url) = std::env::var("ORE_CATALOGO_PRUEBAS") else {
        eprintln!("⚠ sin ORE_CATALOGO_PRUEBAS: la conformidad del catálogo no se prueba aquí");
        return None;
    };
    let esquema = format!(
        "t_c{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    );
    let mut c = Client::connect(&url, NoTls).expect("conectar");
    c.batch_execute(&format!("CREATE SCHEMA {esquema}"))
        .expect("crear el esquema");
    Some(Prueba { url, esquema })
}

/// Un directorio temporal, que se borra al soltarse.
struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(nombre: &str) -> Dir {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "ore-conformidad-{nombre}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Dir(d)
}

fn git(d: &Path, args: &[&str]) {
    let o = Command::new("git")
        .current_dir(d)
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

fn confirmar(d: &Path) {
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "--allow-empty", "-m", "v"]);
}

fn ml(tabla: &str, n: u32) -> String {
    format!("gs://b/ore/v2/catalogo/ventas/default/{tabla}/metadata/{n:05}-x.metadata.json")
}

/// **La siembra**: un árbol con lo que hay en un inquilino de hoy. Lo de antes, los dos de un
/// nombre, nulos y decimales, otro schema, una colección, una copia de antes y un informe.
fn sembrar_en(d: &Path) {
    let ficheros = [
        (
            "datasets/ventas_pedidos.json",
            format!(
                "{{\"metadata_location\": \"{}\", \"dataset\": \"datasets/ventas_pedidos\"}}",
                ml("pedidos", 1)
            ),
        ),
        (
            "datasets/ventas/default/clientes.json",
            format!(
                "{{\"metadata_location\": \"{}\", \"clave\": \"k1\"}}",
                ml("clientes", 1)
            ),
        ),
        (
            "datasets/ventas_dup.json",
            format!("{{\"metadata_location\": \"{}\"}}", ml("dup", 9)),
        ),
        (
            "datasets/ventas/default/dup.json",
            format!("{{\"metadata_location\": \"{}\"}}", ml("dup", 1)),
        ),
        (
            "datasets/ventas/default/nulos.json",
            format!(
                "{{\"metadata_location\": \"{}\", \"hora\": null, \"tasa\": 1.5, \"filas\": 3}}",
                ml("nulos", 1)
            ),
        ),
        (
            "datasets/ventas/eu/sin.json",
            "{\"estado\": \"error\"}".to_string(),
        ),
        (
            "datasets/raro.json",
            format!(
                "{{\"nombre\": \"otra.cosa\", \"metadata_location\": \"{}\"}}",
                ml("raro", 1)
            ),
        ),
        (
            "datasets/medios/default/fotos.json",
            "{\"kind\": \"MediaCollection\", \"metadata_location\": \"gs://b/ore/v2/colecciones/medios/fotos/metadata/00001-x.metadata.json\"}".to_string(),
        ),
        (
            "copias/ventas_viejo.json",
            "{\"metadata_location\": \"gs://b/ore/v2/datasets/ventas_viejo/metadata/00001-x.metadata.json\"}".to_string(),
        ),
        (
            "resultados/f_1.json",
            "{\"resultado\": {\"dataset\": \"resultados/f\"}}".to_string(),
        ),
    ];
    for (r, t) in ficheros {
        let f = d.join(r);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, t + "\n").unwrap();
    }
}

// ── El guion ──────────────────────────────────────────────────────────────

/// Lo observado: `(qué, cómo salió)`, en orden.
type Diario = Vec<(String, String)>;

fn leido(l: &Leido) -> String {
    format!("{:?} {}", l.de, Json::de_node_fiel(&l.puntero).jcs())
}

fn o<T>(r: Result<T, Fallo>, f: impl FnOnce(T) -> String) -> String {
    match r {
        Ok(x) => f(x),
        Err(e) => format!("fallo {} {e:?}", e.codigo()),
    }
}

fn poner(nombre: &str, campos: Vec<(&str, Json)>, quitar: &[&str], e: Esperado) -> Cambio {
    Cambio {
        nombre: nombre.into(),
        que: Que::Poner {
            campos: campos.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            quitar: quitar.iter().map(|q| q.to_string()).collect(),
        },
        esperado: e,
    }
}

fn retirar(nombre: &str, e: Esperado) -> Cambio {
    Cambio {
        nombre: nombre.into(),
        que: Que::Retirar,
        esperado: e,
    }
}

fn sustituir(nombre: &str, j: Json, e: Esperado) -> Cambio {
    Cambio {
        nombre: nombre.into(),
        que: Que::Sustituir(j),
        esperado: e,
    }
}

/// **El guion**, sobre una puerta ya sembrada. `tras` se llama tras cada escritura que salió
/// bien (en git, confirmar).
// La última escritura guarda su versión, que nadie lee ya.
#[allow(unused_assignments)]
fn guion(p: &dyn Punteros, tras: &dyn Fn()) -> Diario {
    let mut d: Diario = Vec::new();
    let ctx = Contexto {
        autor: "persona:ana".into(),
        operacion: "commit".into(),
        idempotencia: None,
    };
    let mut version = p.version().expect("version");
    let apunta = |d: &mut Diario, q: &str, s: String| d.push((q.to_string(), s));

    macro_rules! uno {
        ($n:expr) => {{
            let s = o(p.uno($n), |l| l.as_ref().map_or("nada".into(), leido));
            apunta(&mut d, &format!("uno {}", $n), s);
        }};
    }
    macro_rules! todos {
        ($q:expr) => {{
            let s = o(p.todos(), |t| {
                t.iter()
                    .map(|(k, l)| format!("{k} = {}", leido(l)))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            apunta(&mut d, $q, s);
        }};
    }
    macro_rules! historia {
        ($n:expr, $h:expr) => {{
            let s = o(p.historia($n, $h), |vs| {
                let mls: Vec<String> = vs
                    .iter()
                    .map(|v| {
                        v.puntero
                            .get("metadata_location")
                            .and_then(|(_, x)| x.as_str())
                            .unwrap_or("-")
                            .to_string()
                    })
                    .collect();
                format!("{} {mls:?}", vs.len())
            });
            apunta(&mut d, &format!("historia {} {}", $n, $h), s);
        }};
    }
    macro_rules! escribir {
        ($q:expr, $cs:expr) => {{
            let r = p.escribir(&$cs, &ctx);
            let bien = r.is_ok();
            if bien {
                tras();
            }
            apunta(&mut d, $q, o(r, |()| "bien".into()));
            let v = p.version().expect("version");
            apunta(
                &mut d,
                &format!("{} · la versión", $q),
                format!("cambió: {}", v != version),
            );
            version = v;
        }};
    }
    macro_rules! reclamados {
        ($q:expr) => {{
            let s = o(p.reclamados(), |r| format!("{r:#?}"));
            apunta(&mut d, $q, s);
        }};
    }

    // Lo sembrado.
    for n in [
        "ventas.pedidos",
        "ventas.clientes",
        "ventas.dup",
        "ventas.nulos",
        "ventas.eu.sin",
        "medios.fotos",
        "ventas.nadie",
        "otra.cosa",
        "sin_base",
        "a.b.c.d",
    ] {
        uno!(n);
    }
    todos!("todos sembrados");
    historia!("ventas.pedidos", 10);
    historia!("ventas.nadie", 10);
    historia!("sin_base", 10);
    reclamados!("reclamados sembrados");

    // El de antes se lee, y al escribir pasa a su sitio y se retira.
    escribir!(
        "poner sobre el de antes",
        [poner(
            "ventas.pedidos",
            vec![("metadata_location", Json::s(ml("pedidos", 2)))],
            &["dataset"],
            Esperado::Ml(ml("pedidos", 1)),
        )]
    );
    uno!("ventas.pedidos");
    todos!("todos tras el de antes");
    historia!("ventas.pedidos", 10);

    // Cada conflicto, con su nombre, lo esperado y lo que hay; nada cambia.
    escribir!(
        "esperado viejo",
        [poner(
            "ventas.pedidos",
            vec![("estado", Json::s("x"))],
            &[],
            Esperado::Ml(ml("pedidos", 1)),
        )]
    );
    escribir!(
        "nacer sobre lo que está",
        [poner("ventas.clientes", vec![], &[], Esperado::Nada)]
    );
    escribir!(
        "esperado sobre lo que no está",
        [poner(
            "ventas.nadie",
            vec![],
            &[],
            Esperado::Ml(ml("nadie", 1))
        )]
    );

    // Varias tablas: todas o ninguna.
    escribir!(
        "dos tablas, la segunda no cumple",
        [
            poner(
                "ventas.nuevo",
                vec![("metadata_location", Json::s(ml("nuevo", 1)))],
                &[],
                Esperado::Nada,
            ),
            poner(
                "ventas.clientes",
                vec![("metadata_location", Json::s(ml("clientes", 2)))],
                &[],
                Esperado::Ml(ml("clientes", 7)),
            ),
        ]
    );
    uno!("ventas.nuevo");
    escribir!(
        "dos tablas",
        [
            poner(
                "ventas.nuevo",
                vec![
                    ("metadata_location", Json::s(ml("nuevo", 1))),
                    ("filas", Json::Int(10)),
                    ("filas", Json::Int(11)),
                ],
                &[],
                Esperado::Nada,
            ),
            poner(
                "ventas.clientes",
                vec![("metadata_location", Json::s(ml("clientes", 2)))],
                &["clave"],
                Esperado::Ml(ml("clientes", 1)),
            ),
        ]
    );
    uno!("ventas.nuevo");
    uno!("ventas.clientes");

    // Los nombres que no lo son, y quién gana cuando hay más de un fallo.
    escribir!(
        "un conflicto antes de un nombre",
        [
            poner("ventas.clientes", vec![], &[], Esperado::Nada),
            poner("sin_base", vec![], &[], Esperado::Cualquiera),
        ]
    );
    escribir!(
        "un nombre",
        [poner("sin_base", vec![], &[], Esperado::Cualquiera)]
    );
    escribir!(
        "un nombre tras uno que vale",
        [
            poner("ventas.x", vec![], &[], Esperado::Nada),
            poner("a.b.c.d", vec![], &[], Esperado::Cualquiera),
        ]
    );
    uno!("ventas.x");

    // Sustituir: un objeto, o un 503 sin tocar nada.
    escribir!(
        "sustituir por algo que no es un objeto",
        [sustituir(
            "ventas.clientes",
            Json::s("x"),
            Esperado::Cualquiera
        )]
    );
    escribir!(
        "un conflicto antes de algo que no es un objeto",
        [
            poner("ventas.nuevo", vec![], &[], Esperado::Nada),
            sustituir("ventas.clientes", Json::Int(1), Esperado::Cualquiera),
        ]
    );
    escribir!(
        "sustituir sin metadata_location",
        [sustituir(
            "ventas.clientes",
            Json::obj([
                ("estado", Json::s("error")),
                ("dataset", Json::s("datasets/ventas_clientes")),
            ]),
            Esperado::Ml(ml("clientes", 2)),
        )]
    );
    uno!("ventas.clientes");
    escribir!(
        "esperado sobre uno sin metadata_location",
        [poner(
            "ventas.clientes",
            vec![],
            &[],
            Esperado::Ml(ml("clientes", 2))
        )]
    );

    // Poner sobre nulos y decimales (la diferencia conocida: ver `NULOS`), y retirar.
    escribir!(
        "poner sobre nulos",
        [poner(
            "ventas.nulos",
            vec![("estado", Json::s("ok"))],
            &[],
            Esperado::Ml(ml("nulos", 1)),
        )]
    );
    uno!("ventas.nulos");
    escribir!(
        "retirar nulos",
        [retirar("ventas.nulos", Esperado::Cualquiera)]
    );
    uno!("ventas.nulos");

    // Retirar: el de su sitio y el de antes; la historia se queda.
    escribir!(
        "retirar",
        [retirar("ventas.pedidos", Esperado::Ml(ml("pedidos", 2)))]
    );
    uno!("ventas.pedidos");
    historia!("ventas.pedidos", 10);
    escribir!(
        "retirar los dos",
        [retirar("ventas.dup", Esperado::Cualquiera)]
    );
    uno!("ventas.dup");
    escribir!(
        "retirar lo que no está",
        [retirar("ventas.nadie", Esperado::Cualquiera)]
    );

    historia!("ventas.clientes", 10);
    historia!("ventas.clientes", 1);
    historia!("ventas.clientes", 0);
    todos!("todos al final");
    reclamados!("reclamados al final");
    d
}

/// La observación en la que las dos difieren a sabiendas: `Ficheros` pone encima con
/// `Json::de_node`, que degrada el nulo y el decimal a cadenas; el catálogo (`jsonb`) los
/// conserva. Si `Ficheros` se arregla, esto lo dirá.
const NULOS: &str = "uno ventas.nulos";

fn comparar(f: &Diario, c: &Diario, quien: &str) {
    assert_eq!(
        f.len(),
        c.len(),
        "{quien}: el mismo número de observaciones"
    );
    let mut nulos = 0;
    for ((qf, sf), (qc, sc)) in f.iter().zip(c) {
        assert_eq!(qf, qc);
        if qf == NULOS && sf.contains("\"estado\":\"ok\"") {
            nulos += 1;
            assert!(
                sf.contains("\"hora\":\"null\"") && sf.contains("\"tasa\":\"1.5\""),
                "Ficheros: {sf}"
            );
            assert!(
                sc.contains("\"hora\":null") && sc.contains("\"tasa\":1.5"),
                "{quien}: {sc}"
            );
            continue;
        }
        assert_eq!(sf, sc, "{quien}: «{qf}» no da lo mismo");
    }
    assert_eq!(nulos, 1, "la diferencia conocida, vista una vez");
}

/// El guion contra `Ficheros`: un árbol de git sembrado y confirmado.
fn con_ficheros() -> Diario {
    let d = tmp("ficheros");
    git(&d.0, &["init", "-q", "-b", "main"]);
    git(&d.0, &["config", "user.name", "quien"]);
    git(&d.0, &["config", "user.email", "quien@invalido"]);
    git(&d.0, &["config", "commit.gpgsign", "false"]);
    sembrar_en(&d.0);
    confirmar(&d.0);
    let p = Ficheros::en(&d.0);
    guion(&p, &|| confirmar(&d.0))
}

fn arbol_sembrado() -> Dir {
    let d = tmp("catalogo");
    sembrar_en(&d.0);
    d
}

// ── Las pruebas ───────────────────────────────────────────────────────────

#[test]
fn el_mismo_guion_da_lo_mismo_con_ficheros_y_con_el_catalogo() {
    let Some(pr) = prueba() else { return };
    let d = arbol_sembrado();
    let c = Catalogo::abrir(&pr.url, &pr.esquema, None)
        .unwrap()
        .con_arbol(&d.0);
    assert!(c.sembrar_si_hace_falta(&d.0).unwrap(), "la primera siembra");
    let dc = guion(&c, &|| ());
    let df = con_ficheros();
    // Que el guion mide algo: lo que dice `Ficheros`, en unos cuantos puntos.
    let dice = |q: &str, n: usize, s: &str| {
        let v: Vec<&String> = df.iter().filter(|(x, _)| x == q).map(|(_, v)| v).collect();
        assert!(v[n].contains(s), "«{q}» #{n}: {}", v[n]);
    };
    dice("uno ventas.pedidos", 0, &ml("pedidos", 1));
    dice("uno ventas.dup", 0, &ml("dup", 1));
    dice("uno sin_base", 0, "fallo 422");
    dice("poner sobre el de antes", 0, "bien");
    dice("poner sobre el de antes · la versión", 0, "cambió: true");
    dice("historia ventas.pedidos 10", 1, "2 [");
    dice("historia ventas.pedidos 10", 2, "2 [");
    dice("esperado viejo", 0, "fallo 409");
    dice("esperado viejo · la versión", 0, "cambió: false");
    dice("dos tablas, la segunda no cumple", 0, "ventas.clientes");
    dice("uno ventas.nuevo", 0, "nada");
    dice("uno ventas.nuevo", 1, "11");
    dice("un conflicto antes de un nombre", 0, "fallo 409");
    dice("un nombre tras uno que vale", 0, "fallo 422");
    dice("sustituir por algo que no es un objeto", 0, "fallo 503");
    dice(
        "un conflicto antes de algo que no es un objeto",
        0,
        "fallo 409",
    );
    dice("historia ventas.clientes 10", 0, "3 [");
    dice("todos sembrados", 0, "otra.cosa = ");
    dice("reclamados sembrados", 0, "resultados/f");
    dice("reclamados sembrados", 0, "copias/ventas_viejo");
    comparar(&df, &dc, "el catálogo");

    // Sembrar otra vez no hace nada: no pisa lo escrito por la puerta.
    assert!(!c.sembrar_si_hace_falta(&d.0).unwrap());
    let otra = Catalogo::abrir(&pr.url, &pr.esquema, Some("main")).unwrap();
    assert!(!otra.sembrar_si_hace_falta(&d.0).unwrap());
    assert!(c.uno("ventas.pedidos").unwrap().is_none());
}

/// El servidor del protocolo, en memoria: las mismas líneas, sin hijo.
struct EnMemoria(Mutex<Servidor>);

impl Canal for EnMemoria {
    fn ida_y_vuelta(&self, linea: &str) -> Result<String, Fallo> {
        Ok(self.0.lock().unwrap().atender(linea))
    }
}

#[test]
fn y_por_el_protocolo_en_memoria() {
    let Some(pr) = prueba() else { return };
    let d = arbol_sembrado();
    let s = ore_catalogo::puerta::servidor(pr.url.clone(), pr.esquema.clone(), true);
    let p = Proceso::por(Box::new(EnMemoria(Mutex::new(s))), &d.0, None, None, None);
    let dc = guion(&p, &|| ());
    comparar(&con_ficheros(), &dc, "el protocolo en memoria");
}

#[test]
fn y_por_el_protocolo_con_ore_catalogo_punteros() {
    let Some(pr) = prueba() else { return };
    let d = arbol_sembrado();
    // La URL va por `env`: `ORE_PUNTEROS` son palabras, y así no se toca el entorno de la prueba.
    let orden = format!(
        "env CATALOGO_URL={} {} punteros --esquema {} --sembrar",
        pr.url,
        env!("CARGO_BIN_EXE_ore-catalogo"),
        pr.esquema
    );
    let p = Proceso::hijo(&orden, &d.0, None, None, None);
    let dc = guion(&p, &|| ());
    comparar(&con_ficheros(), &dc, "ore-catalogo punteros");
    drop(p);
}

#[test]
fn una_rama_ve_main_al_dia_y_escribe_lo_suyo() {
    let Some(pr) = prueba() else { return };
    let d = arbol_sembrado();
    let main = Catalogo::abrir(&pr.url, &pr.esquema, None).unwrap();
    main.sembrar_si_hace_falta(&d.0).unwrap();
    let ctx = Contexto::default();

    // Una rama que no está no ve nada y no admite escribir.
    let r = Catalogo::abrir(&pr.url, &pr.esquema, Some("r")).unwrap();
    assert!(r.uno("ventas.clientes").unwrap().is_none());
    assert!(r.todos().unwrap().is_empty());
    assert!(r.historia("ventas.clientes", 10).unwrap().is_empty());
    let e = r
        .escribir(
            &[poner("ventas.clientes", vec![], &[], Esperado::Cualquiera)],
            &ctx,
        )
        .unwrap_err();
    assert_eq!(e.codigo(), 503, "{e}");

    // Nace: lo lee de main al día.
    main.crear("r", "main").unwrap();
    assert_eq!(r.uno("ventas.clientes").unwrap().unwrap().de, De::Main);
    assert_eq!(r.todos().unwrap().len(), main.todos().unwrap().len());
    assert_eq!(r.historia("ventas.clientes", 10).unwrap().len(), 1);
    // Lo que escribe la rama es suyo; main no lo ve.
    let v = r.version().unwrap();
    r.escribir(
        &[poner(
            "ventas.clientes",
            vec![("metadata_location", Json::s(ml("clientes", 5)))],
            &[],
            Esperado::Ml(ml("clientes", 1)),
        )],
        &ctx,
    )
    .unwrap();
    assert_ne!(r.version().unwrap(), v);
    let l = r.uno("ventas.clientes").unwrap().unwrap();
    assert_eq!(l.de, De::Aqui);
    assert_eq!(r.historia("ventas.clientes", 10).unwrap().len(), 2);
    let m = main.uno("ventas.clientes").unwrap().unwrap();
    assert_eq!(
        m.puntero
            .get("metadata_location")
            .and_then(|(_, x)| x.as_str()),
        Some(ml("clientes", 1).as_str())
    );
    // Lo que main escribe después, la rama lo ve (y su versión cambia).
    let v = r.version().unwrap();
    main.escribir(
        &[poner(
            "ventas.nuevo",
            vec![("metadata_location", Json::s(ml("nuevo", 1)))],
            &[],
            Esperado::Nada,
        )],
        &ctx,
    )
    .unwrap();
    assert_ne!(r.version().unwrap(), v);
    assert_eq!(r.uno("ventas.nuevo").unwrap().unwrap().de, De::Main);
    // Lo que reclama la rama cuenta mientras vive.
    let ds = "catalogo/ventas/default/clientes".to_string();
    assert!(main.reclamados().unwrap().ubicaciones[&ds].contains(&ml("clientes", 5)));
    main.traer("r").unwrap();
    main.borrar("r").unwrap();
    assert!(!main.reclamados().unwrap().ubicaciones[&ds].contains(&ml("clientes", 5)));
    assert!(r.uno("ventas.clientes").unwrap().is_none());
}

#[test]
fn la_misma_clave_de_idempotencia_es_una_escritura() {
    let Some(pr) = prueba() else { return };
    let c = Catalogo::abrir(&pr.url, &pr.esquema, None).unwrap();
    let ctx = Contexto {
        autor: "persona:ana".into(),
        operacion: "commit".into(),
        idempotencia: Some("k-1".into()),
    };
    let cambio = |n| {
        [poner(
            "ventas.pedidos",
            vec![("metadata_location", Json::s(ml("pedidos", n)))],
            &[],
            Esperado::Cualquiera,
        )]
    };
    c.escribir(&cambio(1), &ctx).unwrap();
    let v = c.version().unwrap();
    c.escribir(&cambio(2), &ctx).unwrap();
    assert_eq!(c.version().unwrap(), v, "la segunda no escribió");
    let h = c.historia("ventas.pedidos", 10).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].quien.as_deref(), Some("persona:ana"));
    // Sin cambios no hay commit.
    c.escribir(&[], &Contexto::default()).unwrap();
    assert_eq!(c.version().unwrap(), v);
}
