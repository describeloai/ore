//! Sonda del terreno de `autorizar()`. **No es una prueba: es una medición.**
//!
//! Antes de escribir la fase ① hay que saber qué contesta Cedar de verdad ante
//! el almacén de entidades que nuestra proyección implica — y no deducirlo
//! leyendo, que es como se llegó a `context.purpose in [...]`.
//!
//! Se ejecuta con:
//!
//! ```text
//! cargo test -p ore-exec --test terreno -- --nocapture
//! ```

use cedar_policy::{
    Authorizer, Context, Entities, Entity, EntityUid, PolicySet, RestrictedExpression, Schema,
};
use ore_exec::Motor;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;

fn ejemplo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("vendor/oos/examples/acme-retail")
        .leak()
}

fn uid(s: &str) -> EntityUid {
    EntityUid::from_str(s).unwrap_or_else(|e| panic!("uid `{s}`: {e}"))
}

fn cadena(v: &str) -> RestrictedExpression {
    RestrictedExpression::new_string(v.to_string())
}

/// El almacén que la proyección implica: cada propiedad clasificada es un
/// `Property` cuyos padres son sus etiquetas efectivas.
fn almacen(m: &Motor, principal: &Entity) -> Entities {
    let lat = ore_core::flow::lattices(&m.paquete);
    let efectivas = ore_core::flow::efectivas(&m.paquete, &lat);

    let mut es: Vec<Entity> = Vec::new();
    for (prop, etiquetas) in &efectivas {
        let padres: HashSet<EntityUid> = etiquetas
            .iter()
            .map(|(ret, nivel)| uid(&format!("Label::\"{ret}:{nivel}\"")))
            .collect();
        es.push(Entity::new_no_attrs(
            uid(&format!("Property::\"{prop}\"")),
            padres,
        ));
    }
    es.push(principal.clone());
    Entities::from_entities(es, Some(&m.esquema)).expect("el almacén tiene que casar con el esquema")
}

fn tiro(
    etiqueta: &str,
    a: &Authorizer,
    ps: &PolicySet,
    ents: &Entities,
    esquema: &Schema,
    p: &str,
    accion: &str,
    r: &str,
    ctx: Vec<(&str, RestrictedExpression)>,
) {
    let contexto = Context::from_pairs(ctx.into_iter().map(|(k, v)| (k.to_string(), v)));
    let contexto = match contexto {
        Ok(c) => c,
        Err(e) => {
            println!("  {etiqueta:<34} CONTEXTO INVÁLIDO · {e}");
            return;
        }
    };
    let req = cedar_policy::Request::new(uid(p), uid(accion), uid(r), contexto, Some(esquema));
    match req {
        Err(e) => println!("  {etiqueta:<34} PETICIÓN INVÁLIDA · {e}"),
        Ok(req) => {
            let resp = a.is_authorized(&req, ps, ents);
            let porque: Vec<String> = resp.diagnostics().reason().map(|p| p.to_string()).collect();
            let errores: Vec<String> = resp.diagnostics().errors().map(|e| e.to_string()).collect();
            println!(
                "  {etiqueta:<34} {:?}{}{}",
                resp.decision(),
                if porque.is_empty() {
                    String::new()
                } else {
                    format!(" · por {}", porque.join(", "))
                },
                if errores.is_empty() {
                    String::new()
                } else {
                    format!(" · ERRORES {errores:?}")
                }
            );
        }
    }
}

#[test]
fn el_terreno() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let a = Authorizer::new();

    // El principal: una `Employee` con sus reclamaciones, miembro de un rol.
    let attrs: HashMap<String, RestrictedExpression> = HashMap::from([
        ("employeeId".to_string(), cadena("emp-42")),
        ("departmentId".to_string(), cadena("finanzas")),
    ]);
    let analista = Entity::new(
        uid("Employee::\"emp-42\""),
        attrs.clone(),
        HashSet::from([uid("Role::\"hr_analyst\"")]),
    )
    .expect("el principal se construye");
    let ents = almacen(&m, &analista);

    println!("\n── ① propiedad clasificada `critical`, con finalidad declarada ──");
    tiro(
        "analista · read · baseSalary",
        &a,
        &m.politicas,
        &ents,
        &m.esquema,
        "Employee::\"emp-42\"",
        "Action::\"read\"",
        "Property::\"hr.Employee.baseSalary\"",
        vec![("purpose", cadena("compensation_review"))],
    );

    println!("\n── ② la misma, con una finalidad que la política no admite ──");
    tiro(
        "analista · read · baseSalary",
        &a,
        &m.politicas,
        &ents,
        &m.esquema,
        "Employee::\"emp-42\"",
        "Action::\"read\"",
        "Property::\"hr.Employee.baseSalary\"",
        vec![("purpose", cadena("marketing"))],
    );

    println!("\n── ③ el `forbid` explícito sobre el DNI ──");
    tiro(
        "cualquiera · export · nationalId",
        &a,
        &m.politicas,
        &ents,
        &m.esquema,
        "Employee::\"emp-42\"",
        "Action::\"export\"",
        "Property::\"hr.Employee.nationalId\"",
        vec![("purpose", cadena("regulatory_reporting"))],
    );

    println!("\n── ④ un principal SIN rol: mismo recurso, misma finalidad ──");
    let sin_rol = Entity::new(uid("Employee::\"emp-99\""), attrs, HashSet::new())
        .expect("el principal se construye");
    let ents2 = almacen(&m, &sin_rol);
    tiro(
        "sin rol · read · baseSalary",
        &a,
        &m.politicas,
        &ents2,
        &m.esquema,
        "Employee::\"emp-99\"",
        "Action::\"read\"",
        "Property::\"hr.Employee.baseSalary\"",
        vec![("purpose", cadena("compensation_review"))],
    );

    println!("\n── ⑤ una petición SIN `purpose` ──");
    tiro(
        "analista · read · baseSalary",
        &a,
        &m.politicas,
        &ents,
        &m.esquema,
        "Employee::\"emp-42\"",
        "Action::\"read\"",
        "Property::\"hr.Employee.baseSalary\"",
        vec![],
    );

    println!("\n── ⑥ una reclamación que el esquema no declara ──");
    tiro(
        "analista · read · baseSalary",
        &a,
        &m.politicas,
        &ents,
        &m.esquema,
        "Employee::\"emp-42\"",
        "Action::\"read\"",
        "Property::\"hr.Employee.baseSalary\"",
        vec![
            ("purpose", cadena("compensation_review")),
            ("clearance", cadena("SECRET")),
        ],
    );

    println!("\n── ⑦ el recurso como INSTANCIA, no como propiedad ──");
    tiro(
        "analista · read · Employee emp-7",
        &a,
        &m.politicas,
        &ents,
        &m.esquema,
        "Employee::\"emp-42\"",
        "Action::\"read\"",
        "Employee::\"emp-7\"",
        vec![("purpose", cadena("compensation_review"))],
    );

    println!("\n── ⑧ propiedad de OTRO paquete, sin política que la alcance ──");
    tiro(
        "analista · read · Customer.email",
        &a,
        &m.politicas,
        &ents,
        &m.esquema,
        "Employee::\"emp-42\"",
        "Action::\"read\"",
        "Property::\"customers.Customer.email\"",
        vec![("purpose", cadena("compensation_review"))],
    );
    println!();
}

/// Dos preguntas que deciden el diseño, y que no se contestan leyendo.
#[test]
fn como_se_llaman_las_politicas_y_que_pasa_con_un_atributo_de_mas() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");

    println!("\n── ⑨ ¿con qué nombre conoce Cedar a cada política? ──");
    for p in m.politicas.policies() {
        println!(
            "  id de Cedar: {:<10} @id nuestro: {:?}",
            p.id().to_string(),
            p.annotation("id")
        );
    }

    println!("\n── ⑩ un principal con una reclamación que el esquema no declara ──");
    let attrs: HashMap<String, RestrictedExpression> = HashMap::from([
        ("employeeId".to_string(), cadena("emp-42")),
        ("departmentId".to_string(), cadena("finanzas")),
        ("clearance".to_string(), cadena("SECRET")),
    ]);
    let e = Entity::new(
        uid("Employee::\"emp-42\""),
        attrs,
        HashSet::from([uid("Role::\"hr_analyst\"")]),
    )
    .expect("la entidad suelta se construye");
    match Entities::from_entities([e], Some(&m.esquema)) {
        Ok(_) => println!("  el almacén lo ACEPTA — el esquema no lo impide"),
        Err(err) => println!("  el almacén lo RECHAZA · {err}"),
    }
    println!();
}
