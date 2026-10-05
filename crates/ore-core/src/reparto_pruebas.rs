//! Las reglas del reparto, una a una, y los casos `plan/` de la spec (v1alpha24).

use super::*;
use std::path::{Path, PathBuf};

/// Un árbol con una fuente `pg` (encendida), dos tablas y dos vistas.
fn arbol(caso: &str, conducto: bool, encendida: bool, extra: &[(&str, &str)]) -> Package {
    let raiz: PathBuf =
        std::env::temp_dir().join(format!("ore-reparto-{}-{caso}", std::process::id()));
    let _ = std::fs::remove_dir_all(&raiz);
    let mut ficheros: Vec<(String, String)> = vec![
        (
            "ontology.config.yaml".into(),
            format!(
                "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: {{ name: prueba, version: 0.1.0 }}\n\
                 datasources:\n  - name: pg\n    type: postgres\n    connectionEnv: PG_URL\n{}",
                if encendida { "    federation: true\n" } else { "" }
            ),
        ),
        (
            "conduits.yaml".into(),
            format!(
                "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: {{ name: prueba }}\n\
                 spec:\n  owner: team:prueba\n  conduits:\n    contextSurface.workspace: {{ oos.maturity: DRAFT }}\n{}",
                if conducto { "    federation.read: { oos.maturity: DRAFT }\n" } else { "" }
            ),
        ),
        (
            "packages/pg/package.yaml".into(),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: pg, version: 0.1.0, status: draft, domain: pg }\n\
             spec: { owner: \"team:prueba\" }\n"
                .into(),
        ),
        (
            "packages/pg/public/schema.yaml".into(),
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: public, namespace: pg }\nspec: { owner: team:prueba }\n"
                .into(),
        ),
        (
            "packages/pg/public/tables/clientes.yaml".into(),
            tabla(
                "clientes",
                &[("id", "Integer"), ("pais", "String"), ("alta", "Date")],
                "    fullScan: cheap\n    predicatePushdown: [eq, in, range, isNull]\n",
            ),
        ),
        (
            "packages/pg/public/tables/pedidos.yaml".into(),
            tabla(
                "pedidos",
                &[("id", "Integer"), ("cliente", "Integer"), ("total", "Integer"), ("estado", "String")],
                "    fullScan: cheap\n    predicatePushdown: [eq, neq, in, range, like, isNull]\n",
            ),
        ),
        (
            "packages/pg/views/v_clientes.yaml".into(),
            vista(
                "v_clientes",
                "SELECT id AS ident, pais AS p FROM pg.public.clientes",
                &["ident", "p"],
            ),
        ),
        (
            "packages/pg/views/v_resumen.yaml".into(),
            vista(
                "v_resumen",
                "SELECT pais, count(*) AS n FROM pg.public.clientes GROUP BY pais",
                &["pais", "n"],
            ),
        ),
    ];
    for (f, t) in extra {
        ficheros.push((f.to_string(), t.to_string()));
    }
    for (f, t) in ficheros {
        let p = raiz.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }
    crate::validate::cargar_paquete(&raiz).0
}

fn tabla(nombre: &str, cols: &[(&str, &str)], reads: &str) -> String {
    let cols: String = cols
        .iter()
        .map(|(c, t)| format!("    {c}: {{ type: {t} }}\n"))
        .collect();
    format!(
        "apiVersion: oos.dev/v1alpha22\nkind: Table\nmetadata: {{ name: {nombre}, namespace: pg, schema: public }}\n\
         spec:\n  datasource: pg\n  object: \"public.{nombre}\"\n  columns:\n{cols}  reads:\n{reads}  changes:\n    key: [id]\n"
    )
}

fn vista(nombre: &str, sql: &str, cols: &[&str]) -> String {
    let cols: String = cols
        .iter()
        .map(|c| format!("    {c}: {{ type: String }}\n"))
        .collect();
    format!(
        "apiVersion: oos.dev/v1alpha24\nkind: View\nmetadata: {{ name: {nombre}, namespace: pg }}\n\
         spec:\n  owner: team:prueba\n  dialect: duckdb\n  sql: |\n    {sql}\n  columns:\n{cols}"
    )
}

fn opciones() -> Opciones<'static> {
    Opciones {
        desde_puesto: false,
        exigir_interruptor: true,
        conectores: None,
    }
}

fn r(pkg: &Package, sql: &str) -> Reparto {
    match repartir(sql, pkg, &opciones()) {
        Ok(r) => r,
        Err(n) => panic!("negado {}: {}", n.codigo, n.mensaje),
    }
}

fn de<'a>(r: &'a Reparto, tabla: &str) -> &'a Lectura {
    r.lecturas
        .iter()
        .find(|l| l.tabla == tabla)
        .unwrap_or_else(|| panic!("no se lee `{tabla}`: {:?}", r.lecturas))
}

fn uno(c: &str, op: &str, v: &str) -> Filtro {
    Filtro {
        columna: c.into(),
        operador: op.into(),
        valor: Valor::Uno(v.into()),
    }
}

const C: &str = "pg.public.clientes";
const P: &str = "pg.public.pedidos";

// ── 1 · columnas ──

#[test]
fn se_piden_las_columnas_que_se_usan() {
    let pkg = arbol("columnas", true, true, &[]);
    let x = r(&pkg, "SELECT id FROM pg.public.clientes WHERE pais = 'ES'");
    assert!(x.entendida);
    let l = de(&x, C);
    assert_eq!(l.columnas, vec!["id", "pais"]);
    assert_eq!(l.empujados, vec![uno("pais", "eq", "ES")]);
    assert_eq!(l.tipo, "postgres");
    assert_eq!(l.objeto, "public.clientes");
}

#[test]
fn un_asterisco_pide_todas() {
    let pkg = arbol("todas", true, true, &[]);
    let x = r(&pkg, "SELECT * FROM pg.public.clientes");
    assert_eq!(de(&x, C).columnas, vec!["id", "pais", "alta"]);
}

#[test]
fn contar_pide_una_columna_y_lo_dice() {
    let pkg = arbol("contar", true, true, &[]);
    let x = r(&pkg, "SELECT count(*) FROM pg.public.clientes LIMIT 1");
    let l = de(&x, C);
    assert_eq!(l.columnas, vec!["id"]);
    assert_eq!(l.limit, None, "un LIMIT sobre un agregado no se empuja");
    assert!(!l.avisos.is_empty());
}

// ── 2 · filtros ──

#[test]
fn se_empuja_lo_que_la_tabla_admite_y_lo_demas_queda_en_el_motor() {
    let pkg = arbol("admite", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.clientes WHERE pais IN ('ES', 'FR') AND upper(pais) = 'ES' AND alta >= DATE '2026-01-01'",
    );
    let l = de(&x, C);
    assert_eq!(
        l.empujados,
        vec![
            Filtro {
                columna: "pais".into(),
                operador: "in".into(),
                valor: Valor::Lista(vec!["ES".into(), "FR".into()])
            },
            uno("alta", "ge", "2026-01-01"),
        ]
    );
    assert_eq!(l.en_el_motor, vec!["upper(pais) = 'ES'"]);
}

#[test]
fn una_familia_que_la_tabla_no_admite_no_se_empuja() {
    let pkg = arbol("familia", true, true, &[]);
    // `clientes` no admite `like` ni `neq`.
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.clientes WHERE pais LIKE 'E%' AND pais <> 'FR'",
    );
    let l = de(&x, C);
    assert!(l.empujados.is_empty());
    assert_eq!(l.en_el_motor.len(), 2);
}

#[test]
fn el_literal_a_la_izquierda_y_between() {
    let pkg = arbol("between", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.pedidos WHERE 100 < total AND id BETWEEN 1 AND 9",
    );
    assert_eq!(
        de(&x, P).empujados,
        vec![
            uno("total", "gt", "100"),
            uno("id", "ge", "1"),
            uno("id", "le", "9")
        ]
    );
}

#[test]
fn un_or_no_se_empuja() {
    let pkg = arbol("or", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.clientes WHERE pais = 'ES' OR pais = 'FR'",
    );
    assert!(de(&x, C).empujados.is_empty());
}

#[test]
fn lo_que_el_conector_no_sabe_no_se_empuja() {
    let pkg = arbol("conector", true, true, &[]);
    let mut m = BTreeMap::new();
    m.insert(
        "postgres".to_string(),
        Conector {
            operadores: ["eq".to_string()].into_iter().collect(),
            limit: false,
            order_by: false,
        },
    );
    let o = Opciones {
        conectores: Some(&m),
        ..opciones()
    };
    let x = repartir(
        "SELECT id FROM pg.public.clientes WHERE pais = 'ES' AND alta > DATE '2026-01-01' LIMIT 5",
        &pkg,
        &o,
    )
    .unwrap();
    let l = de(&x, C);
    assert_eq!(l.empujados, vec![uno("pais", "eq", "ES")]);
    assert_eq!(l.limit, None);
}

// ── 3 · juntas ──

#[test]
fn en_una_junta_cada_tabla_lleva_lo_suyo() {
    let pkg = arbol("junta", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT c.id, p.total FROM pg.public.clientes c JOIN pg.public.pedidos p ON c.id = p.cliente \
         WHERE c.pais = 'ES' AND p.total > 100 LIMIT 10",
    );
    let c = de(&x, C);
    let p = de(&x, P);
    assert_eq!(c.empujados, vec![uno("pais", "eq", "ES")]);
    assert_eq!(p.empujados, vec![uno("total", "gt", "100")]);
    assert_eq!(c.columnas, vec!["id", "pais"]);
    assert_eq!(p.columnas, vec!["cliente", "total"]);
    assert_eq!(
        (c.limit, p.limit),
        (None, None),
        "con una junta, el LIMIT no baja"
    );
    assert!(c.en_el_motor.contains(&"c.id = p.cliente".to_string()));
}

#[test]
fn el_lado_que_puede_quedar_a_nulo() {
    let pkg = arbol("nulo", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT c.id FROM pg.public.clientes c LEFT JOIN pg.public.pedidos p \
         ON c.id = p.cliente AND p.estado = 'pagado' AND c.pais = 'ES' \
         WHERE p.total > 100 AND p.id IS NULL",
    );
    let c = de(&x, C);
    let p = de(&x, P);
    // El ON del lado de dentro se filtra antes; el del lado conservado, no.
    assert!(p.empujados.contains(&uno("estado", "eq", "pagado")));
    assert!(c.empujados.is_empty());
    // El WHERE que rechaza el nulo se empuja (el motor lo vuelve a mirar); IS NULL no.
    assert!(p.empujados.contains(&uno("total", "gt", "100")));
    assert!(!p.empujados.iter().any(|f| f.operador == "isNull"));
}

#[test]
fn full_join_no_empuja_su_on() {
    let pkg = arbol("full", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT c.id FROM pg.public.clientes c FULL JOIN pg.public.pedidos p ON c.id = p.cliente AND p.estado = 'x'",
    );
    assert!(de(&x, P).empujados.is_empty());
}

// ── 4 · limit ──

#[test]
fn el_limit_baja_con_su_orden_y_su_offset() {
    let pkg = arbol("limit", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.clientes WHERE pais = 'ES' ORDER BY alta DESC LIMIT 10 OFFSET 5",
    );
    let l = de(&x, C);
    assert_eq!(l.limit, Some(15));
    assert_eq!(l.orden, vec![("alta".to_string(), true)]);
    assert_eq!(l.columnas, vec!["id", "pais", "alta"]);
}

#[test]
fn el_limit_no_baja_si_algo_queda_en_el_motor() {
    let pkg = arbol("limit-motor", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.clientes WHERE upper(pais) = 'ES' LIMIT 10",
    );
    assert_eq!(de(&x, C).limit, None);
    let x = r(
        &pkg,
        "SELECT DISTINCT pais FROM pg.public.clientes LIMIT 10",
    );
    assert_eq!(de(&x, C).limit, None);
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.clientes ORDER BY upper(pais) LIMIT 10",
    );
    assert_eq!(de(&x, C).limit, None);
}

// ── 5 · vistas, WITH, subconsultas ──

#[test]
fn una_vista_limpia_lleva_el_filtro_y_el_limit_a_su_tabla() {
    let pkg = arbol("vista", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT ident FROM pg.v_clientes WHERE p = 'ES' LIMIT 3",
    );
    let l = de(&x, C);
    assert_eq!(l.empujados, vec![uno("pais", "eq", "ES")]);
    assert_eq!(l.columnas, vec!["id", "pais"]);
    assert_eq!(l.limit, Some(3));
}

#[test]
fn una_vista_que_agrega_no_deja_pasar_el_filtro() {
    let pkg = arbol("vista-agrega", true, true, &[]);
    let x = r(&pkg, "SELECT * FROM pg.v_resumen WHERE pais = 'ES'");
    let l = de(&x, C);
    assert!(l.empujados.is_empty());
    assert_eq!(l.columnas, vec!["pais"]);
}

#[test]
fn un_with_y_una_subconsulta_limpia() {
    let pkg = arbol("with", true, true, &[]);
    let x = r(
        &pkg,
        "WITH c AS (SELECT id, pais FROM pg.public.clientes) SELECT id FROM c WHERE pais IN ('ES')",
    );
    assert_eq!(de(&x, C).empujados.len(), 1);
    let x = r(
        &pkg,
        "SELECT x.ident FROM (SELECT id AS ident, pais FROM pg.public.clientes) AS x WHERE x.pais = 'FR'",
    );
    assert_eq!(de(&x, C).empujados, vec![uno("pais", "eq", "FR")]);
}

#[test]
fn una_subconsulta_con_limit_no_deja_pasar_lo_de_fuera() {
    let pkg = arbol("sub-limit", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT * FROM (SELECT id, pais FROM pg.public.clientes LIMIT 5) t WHERE pais = 'ES'",
    );
    let l = de(&x, C);
    assert!(
        l.empujados.is_empty(),
        "filtrar antes del LIMIT cambia las filas"
    );
    assert_eq!(l.limit, Some(5));
}

#[test]
fn una_subconsulta_en_el_where() {
    let pkg = arbol("in-sub", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT id FROM pg.public.clientes WHERE id IN (SELECT cliente FROM pg.public.pedidos WHERE total > 10)",
    );
    assert_eq!(de(&x, P).empujados, vec![uno("total", "gt", "10")]);
    assert_eq!(de(&x, P).columnas, vec!["cliente", "total"]);
    assert_eq!(de(&x, C).columnas, vec!["id"]);
}

#[test]
fn una_subconsulta_correlacionada_pide_la_columna_de_fuera() {
    let pkg = arbol("correlada", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT c.id FROM pg.public.clientes c WHERE EXISTS          (SELECT 1 FROM pg.public.pedidos p WHERE p.cliente = c.id AND p.estado = 'abierto' AND c.alta > DATE '2026-01-01')",
    );
    assert_eq!(de(&x, C).columnas, vec!["id", "alta"]);
    assert_eq!(de(&x, P).empujados, vec![uno("estado", "eq", "abierto")]);
    assert!(
        de(&x, C).empujados.is_empty(),
        "lo correlacionado no se empuja"
    );
}

#[test]
fn un_asterisco_con_exclude_pide_todas_por_si_acaso() {
    let pkg = arbol("exclude", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT * EXCLUDE (alta) FROM pg.public.clientes WHERE pais = 'ES'",
    );
    assert!(x.entendida);
    assert_eq!(de(&x, C).columnas, vec!["id", "pais", "alta"]);
}

// ── A · una lectura por tabla ──

#[test]
fn dos_apariciones_se_leen_una_vez_con_lo_comun() {
    let pkg = arbol("fusion", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT a.id FROM pg.public.clientes a JOIN pg.public.clientes b ON a.id = b.id \
         WHERE a.pais = 'ES' AND b.pais = 'ES' AND b.alta > DATE '2026-01-01' LIMIT 1",
    );
    assert_eq!(x.lecturas.len(), 1);
    let l = &x.lecturas[0];
    assert_eq!(l.apariciones, 2);
    assert_eq!(l.empujados, vec![uno("pais", "eq", "ES")]);
    assert!(l.en_el_motor.iter().any(|t| t.contains("alta")));
    assert_eq!(l.limit, None);
}

// ── B · lo que no se entiende ──

#[test]
fn lo_que_no_se_analiza_se_lee_sin_empujar() {
    let pkg = arbol("b", true, true, &[]);
    let x = r(&pkg, "PIVOT pg.public.clientes ON pais USING count(*)");
    assert!(!x.entendida);
    let l = de(&x, C);
    assert_eq!(l.columnas, vec!["id", "pais", "alta"]);
    assert!(l.empujados.is_empty());
    assert!(!x.avisos.is_empty());
}

#[test]
fn lo_que_no_se_analiza_sigue_bajo_el_coste() {
    let prohibida = tabla(
        "secreta",
        &[("id", "Integer"), ("pais", "String")],
        "    fullScan: forbidden\n    predicatePushdown: [eq]\n",
    );
    let pkg = arbol(
        "b-coste",
        true,
        true,
        &[("packages/pg/public/tables/secreta.yaml", prohibida.as_str())],
    );
    let n = repartir(
        "PIVOT pg.public.secreta ON pais USING count(*)",
        &pkg,
        &opciones(),
    )
    .unwrap_err();
    assert_eq!(n.codigo, "OOS2044");
}

// ── 6 · coste y gobierno ──

#[test]
fn sin_interruptor_o_sin_conducto_no() {
    let pkg = arbol("apagada", true, false, &[]);
    let n = repartir("SELECT id FROM pg.public.clientes", &pkg, &opciones()).unwrap_err();
    assert_eq!((n.http, n.codigo.as_str()), (403, "federacion"));
    let pkg = arbol("sin-conducto", false, true, &[]);
    let n = repartir("SELECT id FROM pg.public.clientes", &pkg, &opciones()).unwrap_err();
    assert_eq!(n.codigo, "OOS4011");
}

#[test]
fn lo_del_lago_no_es_del_reparto() {
    let pkg = arbol("lago", true, true, &[]);
    let x = r(
        &pkg,
        "SELECT * FROM otro.lago.tabla JOIN pg.public.clientes c ON c.id = tabla.id WHERE c.pais = 'ES'",
    );
    assert_eq!(x.lecturas.len(), 1);
    assert_eq!(de(&x, C).empujados, vec![uno("pais", "eq", "ES")]);
}

// ── la spec: conformance/v1alpha24/plan ──

#[test]
fn los_casos_plan_de_la_spec() {
    let raiz =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/oos/conformance/v1alpha24/plan");
    let mut casos: Vec<PathBuf> = std::fs::read_dir(&raiz)
        .expect("submódulo sin inicializar")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("case.yaml").exists())
        .collect();
    casos.sort();
    assert!(casos.len() >= 9, "faltan casos en {}", raiz.display());
    let mut mal = Vec::new();
    for c in &casos {
        let caso = std::fs::read_to_string(c.join("case.yaml")).unwrap();
        let espera = caso
            .lines()
            .find_map(|l| l.strip_prefix("expects:"))
            .unwrap()
            .trim()
            .to_string();
        let sql = std::fs::read_to_string(c.join("query.sql")).unwrap();
        let (pkg, _) = crate::validate::cargar_paquete(&c.join("input"));
        let o = Opciones {
            exigir_interruptor: false,
            ..opciones()
        };
        let sale = match repartir(&sql, &pkg, &o) {
            Ok(_) => "accept".to_string(),
            Err(n) => n.codigo,
        };
        if sale != espera {
            mal.push(format!(
                "{}: esperaba {espera}, salió {sale}",
                c.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(mal.is_empty(), "{}", mal.join("\n"));
}
