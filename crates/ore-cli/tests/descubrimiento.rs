//! `discover → review → validate → view`, entero y **sin servidor**.
//!
//! La prueba de fuego `pruebas-de-fuego/descubrimiento.sh` cubre el eslabón
//! vivo —resolver el driver en el PATH, ejecutarlo, pasarle la URL— y necesita
//! un PostgreSQL delante. Lo que necesita servidor es *conseguir* el catálogo;
//! lo que pasa **después** no, y eso es todo lo que v1alpha8 cambió.
//!
//! Así que esto empieza donde acaba el driver, con un catálogo escrito a mano, y
//! afirma lo que T3 tenía que sostener:
//!
//! - lo inducido son **`Table` + `View` + `Entity`**, y **ningún `Binding`**;
//! - la mitad física **no es un borrador**: columnas y las dos caras se copian
//!   del catálogo tal cual, porque son hechos del origen;
//! - `ore validate` sobre lo inducido falla **solo** por decisiones pendientes,
//!   nunca por un código de tabla — y cerradas esas, sale en verde.
//!
//! Por la CLI pública, como el resto: lo que se afirma es lo que un usuario ve.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ore() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ore"))
}

fn correr(dir: &Path, args: &[&str]) -> (bool, String) {
    let s = ore()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&s.stdout),
            String::from_utf8_lossy(&s.stderr)
        ),
    )
}

/// Un catálogo como el que deja `ore-read-postgres`, con las dos caras que
/// **sondeó**: `clientes` tiene identidad de replicación por clave —un tombstone
/// por clave, que es `upsert`— y `eventos` no tiene con qué retirar una fila, así
/// que de su flujo solo salen altas.
const CATALOGO: &str = r#"{
  "source": "crm_prod",
  "tables": [
    { "name": "public.clientes",
      "kind": "table",
      "columns": [
        { "name": "id", "type": "Integer" },
        { "name": "email", "type": "String" },
        { "name": "domicilio", "sourceType": "direccion" }
      ],
      "primaryKey": ["id"],
      "reads": { "predicatePushdown": ["eq"], "fullScan": "cheap" },
      "changes": { "mode": "upsert", "key": ["id"], "witness": "log" } },
    { "name": "public.eventos",
      "kind": "table",
      "columns": [
        { "name": "id", "type": "Integer" },
        { "name": "ocurrio_en", "type": "DateTimeTz" }
      ],
      "primaryKey": ["id"],
      "reads": { "predicatePushdown": ["eq"], "fullScan": "cheap" },
      "changes": { "mode": "append", "witness": "log" } },
    { "name": "public.v_activos",
      "kind": "view",
      "columns": [ { "name": "id", "type": "Integer" } ],
      "reads": { "predicatePushdown": ["eq"], "fullScan": "cheap" },
      "changes": { "mode": "none", "witness": "none" } }
  ]
}"#;

/// Deja un repositorio con el catálogo dentro, ya descubierto.
fn descubierto(etiqueta: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cat.json"), CATALOGO).unwrap();

    let (ok, out) = correr(&dir, &["init", "--name", "demo", "."]);
    assert!(ok, "`ore init` falló:\n{out}");
    let (ok, out) = correr(
        &dir,
        &["source", "add", "--name", "crm_prod", "postgres://u:p@h/db"],
    );
    assert!(ok, "`ore source add` falló:\n{out}");
    let (ok, out) = correr(
        &dir,
        &["discover", "--from", "cat.json", "--out", "packages/ventas"],
    );
    assert!(ok, "`ore discover` falló:\n{out}");
    dir
}

/// Contesta y vuelve a inducir. Las respuestas viven **fuera** del repositorio:
/// `ore validate` carga todo `.yaml` del árbol y le exige `apiVersion`.
fn contestar(dir: &Path, respuestas: &str) {
    let f = dir.with_extension("answers.yaml");
    std::fs::write(&f, respuestas).unwrap();
    let (ok, out) = correr(
        dir,
        &[
            "review",
            "packages/ventas",
            "--answers",
            f.to_str().unwrap(),
        ],
    );
    assert!(ok, "`ore review` falló:\n{out}");
}

/// Lo que cierra las dos preguntas de este catálogo: quién responde del paquete,
/// y si la vista del origen es una entidad o un informe sobre una.
const TODO_CONTESTADO: &str = "answers:\n  \
     dueno/ventas: team:datos\n  \
     vista/public.v_activos: omitir\n";

fn leer(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|_| panic!("falta {rel}"))
}

/// **Descubrir dejó de inferir la mitad física y pasó a espejarla.**
///
/// Que el objeto existe, qué columnas tiene, qué se le puede pedir y qué cambios
/// emite son cuatro hechos del origen, no cuatro conjeturas — y por eso salen
/// sin pasar por revisión. Lo que sigue siendo conjetura —qué es una entidad,
/// qué significa una columna— sigue reportándose.
#[test]
fn lo_inducido_son_tablas_vistas_y_entidades_y_ningun_binding() {
    let dir = descubierto("t3-espejo");

    // Ningún `Binding`, en ningún fichero del paquete. Es el recuento que
    // define T3, y se hace sobre el árbol y no sobre una lista de rutas: un
    // documento en un sitio inesperado contaría igual.
    let mut vistos = 0usize;
    let mut pila = vec![dir.join("packages")];
    while let Some(d) = pila.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                pila.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("yaml") {
                vistos += 1;
                let txt = std::fs::read_to_string(&p).unwrap();
                assert!(
                    !txt.contains("kind: Binding"),
                    "el inductor emitió un `Binding` en {}",
                    p.display()
                );
            }
        }
    }
    assert!(vistos >= 5, "esperaba más documentos, vi {vistos}");

    let tabla = leer(
        &dir,
        "packages/ventas/tables/Clientes__public_clientes.yaml",
    );
    let vista = leer(&dir, "packages/ventas/views/Clientes__public_clientes.yaml");
    let entidad = leer(&dir, "packages/ventas/entities/Clientes.yaml");

    // La tabla: el nombre físico ENTERO —es opaco y es del origen— y las dos
    // caras copiadas del catálogo. No se completan ni se corrigen: qué se puede
    // empujar lo sabe quien traduce, y qué cambios salen quien preguntó.
    assert!(tabla.contains("kind: Table"), "{tabla}");
    assert!(
        tabla.contains(r#"object: "public.clientes""#),
        "el nombre físico no viajó entero:\n{tabla}"
    );
    assert!(tabla.contains("mode: upsert"), "{tabla}");
    assert!(tabla.contains("key: [id]"), "{tabla}");
    // Y la columna que NO se supo tipar sigue estando: la tabla es física, no
    // tipa, y perderla sería perder un hecho del origen.
    assert!(
        tabla.contains("domicilio: { physicalType: direccion }"),
        "perdió la columna sin tipo de OOS:\n{tabla}"
    );

    // La vista: expone el objeto con nombres de identificador, y **no propone
    // ni `materialized` ni `freshness`** — son decisiones de operación con
    // coste, y proponerlas sería inventarlas.
    // Se mira la CLAVE con su sangría y no la palabra: el documento la nombra
    // en un comentario, y a propósito — quien lo revise tiene que saber que la
    // ausencia es una decisión y no un olvido.
    assert!(
        vista.contains("from: { table: public_clientes }"),
        "{vista}"
    );
    assert!(!vista.contains("\n  materialized:"), "{vista}");
    assert!(!vista.contains("\n  freshness:"), "{vista}");
    assert!(
        vista.contains("proponerlas sería inventarlas"),
        "la ausencia no se explica, y entonces parece un olvido:\n{vista}"
    );

    // La entidad nombra a la VISTA, nunca a la tabla. Si nombrara la tabla, sus
    // propiedades tendrían que llamarse como las columnas físicas.
    assert!(entidad.contains("backedBy: clientes"), "{entidad}");
    assert!(entidad.contains("oos.maturity: DRAFT"), "{entidad}");
    // Y la columna sin tipo de OOS no se inventa: se dice que se calló.
    assert!(
        entidad.contains("# domicilio:"),
        "omitió una columna en silencio:\n{entidad}"
    );
}

/// **El criterio de T3.** Lo inducido falla solo por lo que alguien tiene que
/// decidir, nunca por un código de tabla — y contestado, sale en verde.
///
/// La distinción es la que separa un descubrimiento útil de uno que da trabajo:
/// un `OOS2004` o un `OOS2018` sobre lo que el inductor acaba de escribir sería
/// el inductor emitiendo algo roto, y nadie sabría si el error es del origen o
/// suyo.
#[test]
fn lo_inducido_solo_falla_por_lo_que_falta_decidir() {
    let dir = descubierto("t3-verde");

    let (ok, out) = correr(&dir, &["validate", "."]);
    assert!(!ok, "tenía que faltar por decidir:\n{out}");
    // Los que salen son **decisiones**, escritas en la voz del compilador: quién
    // responde del paquete, y que una vista del origen sin clave no tiene
    // identidad que nadie pueda inventarle.
    assert!(out.contains("OOS2009") || out.contains("OOS2010"), "{out}");
    // Y ninguno de tabla, de referencia ni de forma: lo físico que salió de
    // aquí no está a medias. Un `OOS2018` sobre lo que el inductor acaba de
    // escribir sería el inductor emitiendo algo roto, y quien lo leyera no
    // sabría si el error es del origen o suyo.
    for codigo in [
        "OOS2004", "OOS2018", "OOS2019", "OOS2020", "OOS2021", "OOS1004",
    ] {
        assert!(!out.contains(codigo), "salió `{codigo}`:\n{out}");
    }

    contestar(&dir, TODO_CONTESTADO);

    let (ok, out) = correr(&dir, &["validate", "."]);
    assert!(ok, "lo revisado no valida:\n{out}");
    assert!(out.contains("ok · sin errores"), "{out}");

    // Y el dueño llega a los DOS documentos que lo llevan. Es una decisión, no
    // dos: contestarla una vez y que la vista se quedara en `cambiame` sería
    // una pregunta escondida.
    let vista = leer(&dir, "packages/ventas/views/Clientes__public_clientes.yaml");
    assert!(vista.contains(r#"owner: "team:datos""#), "{vista}");
}

/// Y el motor de vistas contesta sobre lo inducido: **las dos caras que el
/// driver sondeó llegan hasta `ore view`** sin que nadie las escriba a mano.
#[test]
fn las_caras_sondeadas_llegan_hasta_el_motor() {
    let dir = descubierto("t3-caras");
    contestar(&dir, TODO_CONTESTADO);

    let (ok, out) = correr(&dir, &["view", "."]);
    assert!(ok, "{out}");
    assert!(
        out.contains(
            "caras     reads: eq · fullScan: cheap · changes: upsert · witness: log · key: id"
        ),
        "{out}"
    );
    // La que solo anexa se ve como lo que es. Es la cara que `OOS2021` mira el
    // día que alguien quiera materializarla para respaldar algo mutable.
    assert!(out.contains("changes: append"), "{out}");
    // Nada se copia todavía, y no porque se haya decidido: porque proponerlo
    // habría sido inventarlo.
    assert!(out.contains("flujo     virtual"), "{out}");
}

/// **Lo omitido se va del paquete, y ahora son tres sitios.**
///
/// Se midió con la simulación de la prueba de fuego: contestar `omitir` sobre
/// una vista del origen retiraba su entidad y **dejaba su tabla y su vista**.
/// El paquete seguía validando —una vista sin entidad es legal— así que el
/// resto no daba ningún síntoma: afirmaba un objeto que alguien acababa de
/// decir que no entra.
///
/// La causa es de las que se repiten: una lista de directorios «que el inductor
/// gobierna» escrita a mano, y dos directorios nuevos que no se añadieron a
/// ella. Es el mismo modo de fallo que `IMPLEMENTADAS` en el arnés de
/// conformidad, y por eso vale la pena tener la prueba y no solo el arreglo.
#[test]
fn lo_que_alguien_omite_no_deja_resto_en_ningun_directorio() {
    let dir = descubierto("t3-resto");

    // La primera pasada la emite: nadie ha dicho todavía que no entre.
    for f in [
        "packages/ventas/entities/V_activos.yaml",
        "packages/ventas/tables/V_activos__public_v_activos.yaml",
        "packages/ventas/views/V_activos__public_v_activos.yaml",
    ] {
        assert!(dir.join(f).exists(), "no emitió {f}");
    }

    contestar(&dir, TODO_CONTESTADO);

    for f in [
        "packages/ventas/entities/V_activos.yaml",
        "packages/ventas/tables/V_activos__public_v_activos.yaml",
        "packages/ventas/views/V_activos__public_v_activos.yaml",
    ] {
        assert!(!dir.join(f).exists(), "dejó de resto {f}");
    }
    // Y lo que nadie omitió sigue entero.
    assert!(
        dir.join("packages/ventas/tables/Clientes__public_clientes.yaml")
            .exists()
    );
}
