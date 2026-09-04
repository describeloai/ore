//! `ore verify` — **el cotejo, con un runner de mentira**.
//!
//! El peldaño `F1` de [`docs/functions.md`](../../../docs/functions.md), y lo que afirma es que
//! *«lo que devuelve un delegado no se cree»* es una comprobación y no una
//! frase.
//!
//! # Por qué el runner es de mentira y aun así es honesto
//!
//! No ejecuta wasm —eso es `ore-invoke`, y es `F4`— pero **hace lo mismo que
//! hará el de verdad**: pregunta a `ore` las dos identidades que puede conocer
//! sin abrir nada —el digest del bundle, con `ore compile`, y el del plan de la
//! vista, con `ore view`—, inventa las tres que solo un delegado contesta
//! —topología, marcas de agua y el `Plan`—, y devuelve edits.
//!
//! Que las tres inventadas pasen sin verificar **no es un agujero de esta
//! prueba**: es la frontera, y `ore verify` la imprime como *sin verificar
//! aquí* en vez de callarla.

use std::path::{Path, PathBuf};
use std::process::Command;

fn paquete(etiqueta: &str, ficheros: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-prop-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, txt) in ficheros {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
    dir
}

fn ore(args: &[&str]) -> (bool, String, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(args)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.success(),
        String::from_utf8_lossy(&s.stdout).to_string(),
        String::from_utf8_lossy(&s.stderr).to_string(),
    )
}

/// **El runner de mentira.**
///
/// Devuelve la propuesta que devolvería uno de verdad: las dos identidades que
/// se pueden saber sin abrir nada, preguntadas; las tres delegadas, inventadas;
/// y el edit que le pidan — incluido uno que la función no declaró, que es el
/// caso que hay que atrapar.
fn runner(dir: &Path, funcion: &str, escribe: &str, clave: (&str, &str), valor: &str) -> String {
    let (_, compilado, _) = ore(&["compile", dir.to_str().unwrap()]);
    let bundle = entre(&compilado, "\"bundle\": \"", "\"").expect("`ore compile` sin bundle");
    let (_, vistas, _) = ore(&["view", dir.to_str().unwrap()]);
    let vista = entre(&vistas, "plan      ", " ").expect("`ore view` sin digest de plan");

    format!(
        "{{\n  \"funcion\": \"{funcion}\",\n  \"bajo\": {{\n    \
         \"bundle\": \"{bundle}\",\n    \
         \"plan\": \"sha256:0000000000000000000000000000000000000000000000000000000000000000\",\n    \
         \"testigos\": {{ \"erp.employees\": \"42\" }},\n    \
         \"topologia\": \"1\",\n    \
         \"vista\": \"{vista}\"\n  }},\n  \
         \"edits\": [\n    {{ \"escribe\": \"{escribe}\", \
         \"fila\": {{ \"{}\": \"{}\" }}, \"valor\": \"{valor}\" }}\n  ]\n}}\n",
        clave.0, clave.1
    )
}

fn entre(s: &str, desde: &str, hasta: &str) -> Option<String> {
    let i = s.find(desde)? + desde.len();
    let resto = &s[i..];
    let j = resto.find(hasta)?;
    Some(resto[..j].to_string())
}

fn escribir(dir: &Path, nombre: &str, texto: &str) -> PathBuf {
    let p = dir.join(nombre);
    std::fs::write(&p, texto).unwrap();
    p
}

const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\n\
                      kind: OntologyConfig\n\
                      metadata: { name: f1, version: 0.1.0 }\n\
                      datasources:\n  \
                        - { name: erp, type: postgres, connectionEnv: ERP_URL }\n  \
                        - { name: lago, type: iceberg, connectionEnv: LAGO_URL }\n";

const PAQUETE: &str = "apiVersion: oos.dev/v1alpha1\n\
                       kind: Package\n\
                       metadata: { name: hr, version: 1.0.0, status: active, domain: people }\n\
                       spec: { owner: team:data }\n";

const TABLA: &str = "apiVersion: oos.dev/v1alpha8\n\
                     kind: Table\n\
                     metadata: { name: employees, namespace: erp }\n\
                     spec:\n  \
                       datasource: erp\n  \
                       object: 'public.employees'\n  \
                       columns:\n    \
                         employee_id: { physicalType: 'varchar(16)' }\n    \
                         country: { physicalType: 'char(2)' }\n    \
                         status: { physicalType: 'varchar(16)' }\n  \
                       reads: { predicatePushdown: [eq], fullScan: cheap }\n  \
                       changes: { mode: retract, witness: log, key: [employee_id] }\n";

const ENTIDAD: &str = "apiVersion: oos.dev/v1alpha1\n\
                       kind: Entity\n\
                       metadata: { name: Employee, namespace: hr }\n\
                       spec:\n  \
                         nature: entity\n  \
                         primaryKey: [employeeId]\n  \
                         backedBy: hr.empleados\n  \
                         properties:\n    \
                           employeeId: { type: String }\n    \
                           pais: { type: String }\n    \
                           estado:\n      \
                             type: String\n      \
                             labels: { acme.assurance: reviewed }\n";

const RETICULO: &str = "apiVersion: oos.dev/v1alpha2\n\
                        kind: Lattice\n\
                        metadata: { name: assurance, namespace: acme }\n\
                        spec:\n  \
                          axis: integrity\n  \
                          levels: [untrusted, inferred, reviewed, attested]\n";

const FUNCION: &str = "apiVersion: oos.dev/v1alpha8\n\
                       kind: Function\n\
                       metadata: { name: activar, namespace: hr }\n\
                       spec:\n  \
                         runtime: wasm\n  \
                         entrypoint: dist/activar.wasm\n  \
                         effects:\n    \
                           - writes: hr.Employee.estado\n      \
                             to: 'ACTIVO'\n  \
                         endorsements:\n    \
                           - endorser: attested\n      \
                             attestation: attestations/activar.intoto.jsonl\n";

/// La vista **se materializa**, y no es un detalle del fixture: desde el
/// [ADR 0018] una vista por la que la ontología escribe tiene que tener dónde
/// sostener la edición — `OOS2025`. Sin esto el paquete no compila, que es
/// justo lo que la regla existe para conseguir.
fn vista(campos: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha8\n\
         kind: View\n\
         metadata: {{ name: empleados, namespace: hr }}\n\
         spec:\n  \
           owner: team:rrhh\n  \
           from: {{ table: erp.employees }}\n  \
           fields:\n{campos}  \
           materialized: {{ datasource: lago, table: 'cache.hr_empleados' }}\n"
    )
}

const CONDUCTO: &str = "apiVersion: oos.dev/v1alpha1\n\
                        kind: ConduitPolicy\n\
                        metadata: { name: hr }\n\
                        spec:\n  \
                          owner: team:security\n  \
                          conduits:\n    \
                            materialization.payload:\n      \
                              acme.assurance: reviewed\n";

const CAMPOS: &str = "    employeeId: employee_id\n    pais: country\n    estado: status\n";

fn arbol(vista_txt: &str) -> Vec<(&'static str, String)> {
    vec![
        ("ontology.config.yaml", CONFIG.to_string()),
        ("package.yaml", PAQUETE.to_string()),
        ("tables/employees.yaml", TABLA.to_string()),
        ("views/empleados.yaml", vista_txt.to_string()),
        ("entities/Employee.yaml", ENTIDAD.to_string()),
        ("lattices/assurance.yaml", RETICULO.to_string()),
        ("conduits.yaml", CONDUCTO.to_string()),
        ("functions/activar.yaml", FUNCION.to_string()),
    ]
}

fn montar(etiqueta: &str, vista_txt: &str) -> PathBuf {
    let v = arbol(vista_txt);
    let refs: Vec<(&str, &str)> = v.iter().map(|(a, b)| (*a, b.as_str())).collect();
    paquete(etiqueta, &refs)
}

/// **Lo declarado se aplica; lo que se sale, no.**
///
/// Las dos mitades del «listo cuando» de `F1` que van de la superficie, sobre
/// el mismo paquete y con el mismo runner: lo único que cambia entre las dos
/// llamadas es **qué propiedad devuelve**.
#[test]
fn un_edit_fuera_de_los_effects_declarados_no_se_aplica() {
    let dir = montar("superficie", &vista(CAMPOS));

    // Dentro: `estado` es lo que la función declara escribir.
    let buena = escribir(
        &dir,
        "buena.json",
        &runner(
            &dir,
            "hr.activar",
            "hr.Employee.estado",
            ("employeeId", "E-1"),
            "ACTIVO",
        ),
    );
    let (ok, out, err) = ore(&["verify", buena.to_str().unwrap(), dir.to_str().unwrap()]);
    assert!(ok, "tenía que aceptar:\n{out}\n{err}");
    assert!(out.contains("bundle    coincide"), "{out}");
    assert!(out.contains("vista     coincide"), "{out}");
    assert!(
        out.contains("ok · la propuesta cae dentro de lo que el paquete autoriza"),
        "{out}"
    );
    // Y las tres delegadas se dicen sin verificar, en vez de callarse.
    assert!(out.contains("topologia 1 · sin verificar aquí"), "{out}");
    assert!(
        out.contains("testigo   erp.employees = 42 · sin verificar aquí"),
        "{out}"
    );

    // Fuera: `pais` existe, la entidad la tiene, y la función NO la declaró.
    let mala = escribir(
        &dir,
        "mala.json",
        &runner(
            &dir,
            "hr.activar",
            "hr.Employee.pais",
            ("employeeId", "E-1"),
            "PT",
        ),
    );
    let (ok, out, err) = ore(&["verify", mala.to_str().unwrap(), dir.to_str().unwrap()]);
    assert!(!ok, "tenía que rechazar:\n{out}");
    assert!(
        err.contains("escribe `hr.Employee.pais`, que no está en sus `effects:`"),
        "{err}"
    );
    assert!(
        err.contains("declara `hr.Employee.estado`"),
        "el rechazo tiene que decir qué SÍ declaraba: {err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **Cambiar la vista cambia el digest, y el cotejo lo nota.**
///
/// Es la mitad del «listo cuando» que hace auditable *por dónde se iba a
/// entrar*, y la razón por la que la quinta identidad existe: una vista recorta
/// filas, así que una propuesta decidida bajo una vista **no autoriza lo mismo**
/// bajo otra.
///
/// La propuesta se genera contra un paquete y se verifica contra el mismo con
/// la vista cambiada. Nada más se toca.
#[test]
fn cambiar_la_vista_invalida_una_propuesta_ya_hecha() {
    let dir = montar("vista-cambia", &vista(CAMPOS));
    let p = escribir(
        &dir,
        "p.json",
        &runner(
            &dir,
            "hr.activar",
            "hr.Employee.estado",
            ("employeeId", "E-1"),
            "ACTIVO",
        ),
    );
    let (ok, _, _) = ore(&["verify", p.to_str().unwrap(), dir.to_str().unwrap()]);
    assert!(ok, "de partida tiene que valer");

    // La vista deja de exponer `pais`. La entidad la sigue declarando, así que
    // el paquete deja de compilar por `OOS2022` — y eso ya es una respuesta:
    // no se puede verificar contra un paquete que no compila.
    //
    // Para aislar el cambio de vista se recorta la vista Y la entidad, que es
    // lo que haría alguien de verdad al estrechar el recorte.
    let entidad_sin_pais = ENTIDAD.replace("    pais: { type: String }\n", "");
    std::fs::write(
        dir.join("views/empleados.yaml"),
        vista("    employeeId: employee_id\n    estado: status\n"),
    )
    .unwrap();
    std::fs::write(dir.join("entities/Employee.yaml"), &entidad_sin_pais).unwrap();

    let (ok, out, err) = ore(&["verify", p.to_str().unwrap(), dir.to_str().unwrap()]);
    assert!(!ok, "tenía que rechazar:\n{out}\n{err}");
    assert!(out.contains("vista     NO coincide"), "{out}");
    assert!(out.contains("la vista cambió:"), "{out}");
    assert!(
        err.contains("no se pudo confirmar por dónde entra"),
        "{err}"
    );
    // Y el bundle también cambió, porque cambió el paquete: se dicen las dos.
    assert!(out.contains("bundle    NO coincide"), "{out}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// **Digiere igual dos veces**, visto desde fuera.
///
/// El determinismo está probado en `ore-core` sobre la estructura; esto lo
/// afirma sobre **el artefacto que viaja**: dos runners sobre el mismo paquete
/// producen los mismos bytes y el mismo nombre. Es el replay para un auditor,
/// y sin esto sería una promesa.
#[test]
fn dos_invocaciones_sobre_el_mismo_paquete_dan_la_misma_propuesta() {
    let dir = montar("determinismo", &vista(CAMPOS));
    let a = runner(
        &dir,
        "hr.activar",
        "hr.Employee.estado",
        ("employeeId", "E-1"),
        "ACTIVO",
    );
    let b = runner(
        &dir,
        "hr.activar",
        "hr.Employee.estado",
        ("employeeId", "E-1"),
        "ACTIVO",
    );
    assert_eq!(a, b, "el mismo runner sobre el mismo paquete");

    let pa = escribir(&dir, "a.json", &a);
    let (_, sa, _) = ore(&["verify", pa.to_str().unwrap(), dir.to_str().unwrap()]);
    let pb = escribir(&dir, "b.json", &b);
    let (_, sb, _) = ore(&["verify", pb.to_str().unwrap(), dir.to_str().unwrap()]);
    let da = entre(&sa, "propuesta ", "\n").expect("digest");
    let db = entre(&sb, "propuesta ", "\n").expect("digest");
    assert_eq!(da, db);
    assert!(da.starts_with("sha256:"), "{da}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Un edit que no nombra la fila con la clave de su entidad no dice qué fila.
///
/// Sobra o falta: las dos son el mismo defecto. Se prueba la que un runner
/// descuidado produciría —nombrarla con la **columna** en vez de con la
/// propiedad—, que es exactamente el error que la separación de idiomas existe
/// para hacer visible.
#[test]
fn un_edit_que_nombra_la_fila_con_la_columna_no_dice_que_fila() {
    let dir = montar("clave", &vista(CAMPOS));
    let p = escribir(
        &dir,
        "p.json",
        &runner(
            &dir,
            "hr.activar",
            "hr.Employee.estado",
            ("employee_id", "E-1"),
            "ACTIVO",
        ),
    );
    let (ok, out, err) = ore(&["verify", p.to_str().unwrap(), dir.to_str().unwrap()]);
    assert!(!ok, "tenía que rechazar:\n{out}");
    assert!(
        err.contains("nombra la fila con [employee_id] y su entidad se identifica con [employeeId]"),
        "{err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
