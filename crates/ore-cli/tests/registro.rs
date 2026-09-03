//! **El registro de copias**, por la CLI pública.
//!
//! Lo que se afirma aquí no es que el `FilterTree` funcione —eso lo prueban las
//! 143 pruebas de `ore-view` desde M5— sino que **hay un sitio**: que las copias
//! que un paquete tiene dejaron de ser invisibles, vengan de donde vengan.
//!
//! # La afirmación que importa
//!
//! La topología era una vista materializada escrita a mano en el paradigma
//! anterior: `ore-exec` la construye por su cuenta, la refresca con marca de
//! agua propia y nadie la llama copia. Aquí aparece **en el mismo registro y
//! con las mismas tres caras** que una `materialized`, y su ruta de refresco
//! aparece **al lado y por separado**: *estar registrada y estar mantenida son
//! dos cosas*.
//!
//! El inventario de mecanismos —cuántos hay y por qué existe cada uno— lo
//! guarda `registro.rs` en una prueba propia, porque es una afirmación sobre el
//! árbol y no sobre un paquete.

use std::path::{Path, PathBuf};
use std::process::Command;

fn conformidad8(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha8")
        .join(nombre)
        .join("input")
}

fn ejemplo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/oos/examples/acme-retail")
}

/// Un paquete propio para esta prueba. Se usa cuando lo afirmado es del
/// **sustrato**: un ejemplo grande arrastra a la afirmación cosas que no son
/// suyas, y `acme-retail` además está migrado a v1alpha8 a medias.
fn paquete(etiqueta: &str, ficheros: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-reg-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, txt) in ficheros {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
    dir
}

const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
     metadata: { name: x, version: 0.1.0 }\ndatasources:\n  \
     - { name: erp, type: postgres, connectionEnv: ERP_URL }\n  \
     - { name: lago, type: postgres, connectionEnv: LAGO_URL }\n";

const PAQUETE: &str = "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
     metadata: { name: hr, version: 1.0.0, status: active, domain: people }\n\
     spec: { owner: team:data }\n";

fn ver(dir: &Path) -> (bool, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("view")
        .arg(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.success(),
        String::from_utf8_lossy(&s.stdout).into(),
    )
}

/// El bloque del registro va **por paquete** y no por vista, y no es un detalle
/// de presentación: una copia puede contestar la consulta de otra vista, así que
/// la unidad en la que tiene sentido preguntarse *«qué hay ya calculado»* es el
/// paquete entero.
#[test]
fn una_vista_materializada_entra_en_el_registro_con_sus_tres_caras() {
    let (ok, out) = ver(&conformidad8("valid/append-changes-back-an-event"));
    assert!(ok, "el caso es válido:\n{out}");

    assert!(out.contains("registro · 1 copia · nadie 1"), "{out}");
    assert!(out.contains("  app.clics"), "{out}");
    // Las tres caras: qué contesta, dónde vive, hasta cuándo fue cierta.
    assert!(out.contains("    plan      sha256:"), "el plan:\n{out}");
    assert!(
        out.contains("    destino   lago·cache.clics"),
        "el destino:\n{out}"
    );
    assert!(
        out.contains("    testigo   campo `ocurrio_en` · sin poblar"),
        "la marca sale de `changes.witness` de la tabla; el valor no, porque nada \
         puebla una copia todavía:\n{out}"
    );
    // Y la cuarta cosa, que no es de la copia: quién la refresca.
    assert!(out.contains("    refresco  nadie —"), "{out}");
}

/// **El plan del registro es el mismo plan de la vista.** No una reconstrucción
/// parecida: el mismo digest que `ore view` imprime arriba.
///
/// Si divergieran, el View Matcher razonaría sobre un plan que no es el que la
/// vista define, y ofrecería la copia para una consulta que no contesta.
#[test]
fn el_plan_registrado_es_el_mismo_que_el_de_la_vista() {
    let (_, out) = ver(&conformidad8("valid/append-changes-back-an-event"));
    // Solo el digest: la línea de la vista lleva además cuántas se encadenaron.
    let digests: Vec<&str> = out
        .lines()
        .filter(|l| l.trim_start().starts_with("plan      sha256:"))
        .filter_map(|l| l.split_whitespace().find(|w| w.starts_with("sha256:")))
        .collect();
    assert_eq!(digests.len(), 2, "la vista y su copia:\n{out}");
    assert_eq!(
        digests[0], digests[1],
        "el registro guarda OTRO plan que el de la vista:\n{out}"
    );
}

/// **La topología es una copia, y ahora se ve.**
///
/// `acme-retail` no declara ni una `materialized`, y aun así tiene cuatro copias:
/// una por cada relación con `via` de una entidad con clave simple. Las construye
/// `ore-exec` desde siempre; lo nuevo es que estén registradas.
///
/// Y se llega a ellas **por el sustrato**: la fuente física de una entidad es la
/// raíz de la vista que la respalda. Ni un binding de por medio, que es lo que
/// permite que esto siga valiendo con la gramática de v1alpha8.
#[test]
fn la_topologia_entra_en_el_mismo_registro_y_con_su_ruta_aparte() {
    let (ok, out) = ver(&ejemplo());
    assert!(ok, "{out}");

    assert!(
        out.contains("registro · 4 copias · nadie 0 · índice de topología 4"),
        "{out}"
    );
    for esperada in [
        "  hr.Employee.manager",
        "  hr.Employee.department",
        "  supply.Shipment.supplier",
        "  supply.Shipment.sku",
    ] {
        assert!(out.contains(esperada), "falta `{esperada}`:\n{out}");
    }
    // El destino nombra su formato, porque el formato es del destino: el
    // registro no sabe qué es un CSR firmado.
    assert!(
        out.contains("    destino   oretopo·hr.Employee.manager"),
        "{out}"
    );
    // Registrada, y mantenida por otro. Las dos cosas dichas a la vez.
    assert_eq!(
        out.matches("    refresco  índice de topología").count(),
        4,
        "{out}"
    );
    assert!(
        out.contains("`ore index refresh`, con marca de agua propia"),
        "dice por qué tiene ruta propia:\n{out}"
    );
}

/// Un paquete sin copias lo dice, en vez de no decir nada. La diferencia importa:
/// *«no hay copias»* y *«no miré»* se leen igual cuando la salida está vacía.
#[test]
fn un_paquete_sin_copias_lo_dice() {
    let (ok, out) = ver(&conformidad8("valid/view-over-table"));
    assert!(ok, "{out}");
    assert!(
        out.contains("registro · 0 copias · nadie 0 · índice de topología 0"),
        "{out}"
    );
    assert!(
        out.contains("ninguna · el paquete no declara ninguna materialización"),
        "{out}"
    );
}

// ── I2 · el cotejo ───────────────────────────────────────────────────────────

/// **El pago del registro.** `ventas.iberia` es virtual y no declara copia
/// ninguna; `ventas.pedidos` sí, y es la de abajo en su cadena. El cotejo
/// demuestra —por álgebra, no siguiendo la cadena— que la copia la contesta, y
/// dice qué queda por aplicar encima: el `where` que `iberia` añade.
///
/// La diferencia con `raíz de lectura`, que dice algo parecido dos líneas más
/// arriba, es de qué se apoya cada uno: aquella recorre `from` hasta la raíz;
/// esta compara dos planes y **no necesita que haya una cadena**. El día que dos
/// vistas escritas por separado resulten ser la misma consulta, solo una de las
/// dos lo verá.
#[test]
fn una_vista_virtual_se_contesta_desde_la_copia_de_otra() {
    let (ok, out) = ver(&conformidad8("valid/virtual-over-materialized-over-stream"));
    assert!(ok, "{out}");
    assert!(
        out.contains("cotejo    la contesta `ventas.pedidos` · 1 conyunto de compensación"),
        "la copia de `pedidos` contesta a `iberia`, con el `where` de compensación:\n{out}"
    );
    // Y la copia contesta a su propia vista sin nada encima, que es el caso base.
    assert!(
        out.contains("cotejo    la contesta `ventas.pedidos` — su copia · sin compensación"),
        "{out}"
    );
}

/// **El label seal, que es lo que no tiene nadie más.**
///
/// `hr.empleados` filtra por una columna `high` y la copia no la expone. Si al
/// reescribir se recalculase el linaje sobre la tabla copiada, la etiqueta
/// desaparecería y la consulta reescrita parecería limpia. Aquí viaja — y viaja
/// **con el nombre que le da quien pregunta**, `dni`, no el de la copia.
#[test]
fn el_sello_de_la_copia_se_hereda_en_la_reescritura() {
    let (ok, out) = ver(&conformidad8(
        "valid/materialized-view-over-table-within-clearance",
    ));
    assert!(ok, "{out}");
    assert!(
        out.contains("sello heredado: dni {gdpr.sensitivity:high}"),
        "la etiqueta cruza la reescritura y el renombre:\n{out}"
    );
}

/// Una candidata que **no** contesta lo dice con el motivo exacto, y el motivo
/// es útil: la copia de topología solo lleva dos columnas, así que no puede
/// servir una consulta que pide una tercera.
///
/// Que el índice invertido la ofreciera **no es un fallo**: comparten hoja, que
/// es todo lo que el Filter Tree mira. Decidir es del cotejo, y decide.
/// El paquete es **propio** y mínimo, y no `acme-retail`, a propósito: lo que se
/// afirma es del cotejo, y un ejemplo grande metería en la afirmación cosas que
/// no son suyas. `acme-retail` solo se usa arriba, donde lo afirmado **es** el
/// mecanismo heredado.
#[test]
fn una_candidata_que_no_expone_la_columna_dice_cual() {
    let dir = paquete(
        "sin-columna",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", PAQUETE),
            (
                "tables/employees.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
                 metadata: { name: employees, namespace: erp }\nspec:\n  \
                 datasource: erp\n  object: \"public.employees\"\n  \
                 columns: { employee_id: {}, dept_id: {}, salario: {} }\n  \
                 reads: { fullScan: cheap }\n  changes: { mode: retract, witness: log }\n",
            ),
            (
                "views/empleados.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { table: erp.employees }\n  \
                 fields: { employeeId: employee_id, departmentId: dept_id, salario: salario }\n",
            ),
            (
                "entities/Employee.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  \
                 primaryKey: [employeeId]\n  backedBy: empleados\n  properties:\n    \
                 employeeId: { type: String }\n    departmentId: { type: String }\n    \
                 salario: { type: String }\n  relations:\n    department:\n      \
                 target: hr.Employee\n      cardinality: many_to_one\n      via: [departmentId]\n",
            ),
        ],
    );
    let (ok, out) = ver(&dir);
    assert!(ok, "{out}");
    assert!(
        out.contains("cotejo    `hr.Employee.department` no la contesta"),
        "{out}"
    );
    assert!(
        out.contains("`salario` no se deriva de la materialización: no la expone"),
        "dice qué columna falta, y no `false`:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Las restricciones se cuentan aunque sean cero**, y por lo mismo que los
/// caminos de refresco: con cero referenciales, ninguna materialización con una
/// hoja de más podrá contestar nunca, y sin esta línea ese «no la contesta»
/// parece un fallo del cotejo en vez de una declaración que falta.
///
/// Se afirma sobre un caso de conformidad y **no** sobre `acme-retail`. Allí
/// salen `4 únicas · 0 referenciales`, pero eso no mide el sustrato: mide que
/// cinco de sus siete entidades no declaran `backedBy` todavía. Fijarlo en una
/// prueba haría que **terminar** la migración del ejemplo pusiera roja esta
/// afirmación, que es exactamente al revés de lo que una prueba debe hacer.
#[test]
fn las_restricciones_se_cuentan_y_los_ceros_tambien() {
    // La que sale de `changes.key` de una tabla `upsert`, que la especificación
    // exige: sin ella el mantenedor no sabe qué retracta un tombstone.
    let (_, v) = ver(&conformidad8("valid/virtual-over-materialized-over-stream"));
    assert!(
        v.contains("restricciones  1 única · 0 referenciales"),
        "la clave del `upsert`:\n{v}"
    );
    // Y un paquete sin ninguna las cuenta igual, que es el caso que explica un
    // «no la contesta» sin culpar al cotejo.
    let (_, t) = ver(&conformidad8("valid/view-over-table"));
    assert!(
        t.contains("restricciones  0 únicas · 0 referenciales"),
        "{t}"
    );
}

/// **La referencial, que ningún fichero del repositorio produce.**
///
/// Y por eso este paquete se escribe aquí: sin él, la única rama de
/// `restricciones` que construye una `Restriccion::Referencial` viajaría sin que
/// nadie la haya ejecutado nunca. La medida que la hizo falta es de
/// `acme-retail`: sus relaciones `required: true` apuntan a entidades sin
/// `backedBy`, así que allí la rama se cae antes de llegar.
///
/// Dos entidades, las dos con respaldo, y una relación **obligatoria** entre
/// ellas. Eso es lo que hace falta para poder afirmar que una junta de más ni
/// pierde ni duplica: la unicidad del destino sale de su `primaryKey`, y que
/// toda fila del origen case sale de `required: true`.
#[test]
fn una_relacion_obligatoria_entre_entidades_con_respaldo_da_una_referencial() {
    let ficheros: &[(&str, &str)] = &[
        ("ontology.config.yaml", CONFIG),
        ("package.yaml", PAQUETE),
        (
            "tables/employees.yaml",
            "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
             metadata: { name: employees, namespace: erp }\nspec:\n  \
             datasource: erp\n  object: \"public.employees\"\n  columns:\n    \
             employee_id: {}\n    dept_id: {}\n  reads:\n    fullScan: cheap\n  \
             changes: { mode: retract, witness: log }\n",
        ),
        (
            "tables/departments.yaml",
            "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
             metadata: { name: departments, namespace: erp }\nspec:\n  \
             datasource: erp\n  object: \"public.departments\"\n  columns:\n    \
             dept_pk: {}\n    nombre: {}\n  reads:\n    fullScan: cheap\n  \
             changes: { mode: retract, witness: log }\n",
        ),
        (
            "views/empleados.yaml",
            "apiVersion: oos.dev/v1alpha8\nkind: View\n\
             metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
             from: { table: erp.employees }\n  fields:\n    \
             employeeId: employee_id\n    departmentId: dept_id\n",
        ),
        (
            "views/departamentos.yaml",
            "apiVersion: oos.dev/v1alpha8\nkind: View\n\
             metadata: { name: departamentos, namespace: hr }\nspec:\n  owner: team:hr\n  \
             from: { table: erp.departments }\n  fields:\n    \
             departmentId: dept_pk\n    nombre: nombre\n",
        ),
        (
            "entities/Employee.yaml",
            "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
             metadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  \
             primaryKey: [employeeId]\n  backedBy: empleados\n  properties:\n    \
             employeeId: { type: String }\n    departmentId: { type: String }\n  \
             relations:\n    department:\n      target: hr.Department\n      \
             cardinality: many_to_one\n      via: [departmentId]\n      required: true\n",
        ),
        (
            "entities/Department.yaml",
            "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
             metadata: { name: Department, namespace: hr }\nspec:\n  nature: entity\n  \
             primaryKey: [departmentId]\n  backedBy: departamentos\n  properties:\n    \
             departmentId: { type: String }\n    nombre: { type: String }\n",
        ),
    ];
    let dir = paquete("referencial", ficheros);
    let (ok, out) = ver(&dir);
    assert!(ok, "{out}");
    assert!(
        out.contains("restricciones  2 únicas · 1 referencial"),
        "las dos claves primarias y el enlace obligatorio:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── I3 · el testigo ──────────────────────────────────────────────────────────

/// **La marca entra en el registro, y sale de la tabla.**
///
/// *Qué prueba qué versión de los datos se leyó* es una propiedad del objeto, no
/// de quien lo consulta — como `reads` y como `changes.mode`. Una vista no puede
/// fechar mejor que su origen.
#[test]
fn el_testigo_de_una_copia_lleva_la_marca_de_su_tabla() {
    let (_, log) = ver(&conformidad8("valid/stream-table-materialized"));
    assert!(log.contains("testigo   registro · sin poblar"), "{log}");

    let (_, campo) = ver(&conformidad8("valid/append-changes-back-an-event"));
    assert!(
        campo.contains("testigo   campo `ocurrio_en` · sin poblar"),
        "`witness: field` trae además QUÉ columna ordena el avance:\n{campo}"
    );
}

/// **El criterio de I3: una frescura que no se puede comprobar se declara
/// degradada.**
///
/// Y degradada, no inválida — la diferencia es toda la gracia. Declarar
/// `freshness` sobre una tabla que no emite testigo es **legal**: nadie miente,
/// simplemente no hay con qué fechar la copia. Lo que no puede pasar es que se
/// sirva lo viejo como fresco sin que nadie lo diga.
///
/// > Para un agente, saber que el contexto está degradado es la diferencia entre
/// > abstenerse y alucinar.
///
/// El mismo paquete provoca la otra mitad del peldaño: `changes: { mode: none }`
/// hace que el Refresh Analyzer diga `FULL`, cuando por la forma del plan —un
/// filtro sobre una hoja— habría dicho `INCREMENTAL`.
#[test]
fn una_copia_que_no_puede_fecharse_declara_el_estado_degradado() {
    let dir = paquete(
        "sin-testigo",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", PAQUETE),
            (
                "lattices/sensitivity.yaml",
                "apiVersion: oos.dev/v1alpha3\nkind: Lattice\n\
                 metadata: { name: sensitivity, namespace: gdpr }\nspec:\n  \
                 levels: [none, low, high]\n",
            ),
            (
                "conduits.yaml",
                "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\n\
                 metadata: { name: hr }\nspec:\n  owner: team:security\n  conduits:\n    \
                 materialization.payload:\n      gdpr.sensitivity: high\n",
            ),
            (
                "tables/employees.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
                 metadata: { name: employees, namespace: erp }\nspec:\n  \
                 datasource: erp\n  object: \"public.employees\"\n  \
                 columns: { employee_id: {}, country: {} }\n  \
                 reads: { fullScan: cheap }\n  \
                 changes: { mode: none, witness: none }\n",
            ),
            (
                "views/empleados.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { table: erp.employees }\n  freshness: 10m\n  \
                 fields: { employeeId: employee_id, pais: country }\n  \
                 where: { country: ES }\n  \
                 materialized: { datasource: lago, table: \"cache.empleados\" }\n",
            ),
        ],
    );
    let (ok, out) = ver(&dir);
    assert!(
        ok,
        "no es un error de compilación, es una degradación:\n{out}"
    );

    // La copia promete una frescura que nada puede verificar, y se dice.
    assert!(
        out.contains("frescura  10m · DEGRADADA"),
        "la línea de frescura:\n{out}"
    );
    assert!(
        out.contains("la copia no puede decir hasta cuándo fue cierta"),
        "y dice por qué:\n{out}"
    );
    assert!(
        out.contains("degradado · 1 copia declara una frescura que no se puede comprobar"),
        "y se resume al final, donde se ve sin leer todo:\n{out}"
    );

    // La otra mitad: sin cambios que lleguen no hay nada que mantener, aunque
    // el álgebra del plan sea perfectamente incrementalizable.
    assert!(
        out.contains("refresco  REFRESH_MODE = FULL"),
        "el analizador mira ahora la cara `D`:\n{out}"
    );
    assert!(
        out.contains("declara no emitir cambios"),
        "y dice cuál es el motivo:\n{out}"
    );

    // Y el testigo del registro lo dice también, con el vocabulario de OOS.
    assert!(out.contains("testigo   sin poblar"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
