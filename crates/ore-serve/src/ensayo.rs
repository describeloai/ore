//! 0055 P1 · **Preview de un transform**: el código del editor, sin commitear,
//! corrido en la sesión de la persona (D6, D18–D21). Dos rutas:
//!
//! - **`POST /transforms/editor {fichero, contenido}`**: los `@transform` del
//!   texto sin guardar —para el desplegable de Preview—, con la derivación
//!   del commit (`transformar::del_editor`): sin leer el árbol, sin escribir,
//!   sin ejecutar. Y si no se lee, sus `OOS2043` con el texto y la línea de
//!   la puerta del commit. Como `POST /funciones/firma` para las funciones.
//! - **`POST /puestos/{id}/preview {fichero, contenido, transform}`**: la celda
//!   del arnés del build (`builds::arnes_de_preview`) a la cola de la sesión,
//!   con un **techo** —lo que el código del editor declara para ese `def`—
//!   que el servidor pone al empezar la celda y quita con su salida, como el
//!   documento en un build (D21: la misma exigencia). Mientras corre, `write()`
//!   no escribe (lo intercepta el SDK) y el servidor no deja escribir nada a
//!   ese puesto (`rutas::puerta_del_preview`, la credencial prestada sólo lee):
//!   ni snapshot, ni procedencia, ni historial de builds. Principio 2: sólo
//!   Build escribe.
//!
//! El resultado se espera como el de una celda (`GET /puestos/{id}/celdas/{n}`):
//! su ficha lleva `preview` (`puestos::ficha_de_ensayo`) y lo que habría escrito
//! va en `salida.informe.preview`.
use crate::builds::{con_motivo, diagnostico_json};
use crate::puestos::{Ensayo, Transform};
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_core::transformar::DelEditor;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::Path;

/// Lo más largo que se acepta del editor: lo de una celda.
const CONTENIDO_MAXIMO: usize = 256 * 1024;

/// El cuerpo de las dos rutas.
#[derive(Debug, PartialEq)]
struct Pedido {
    /// La ruta del fichero en el árbol (`packages/<p>/<repo>/x.py`).
    fichero: String,
    contenido: String,
    /// El `def` que se ensaya (sólo en `preview`).
    transform: Option<String>,
}

fn pedido(cuerpo: &str) -> Result<Pedido, Respuesta> {
    let n = match ore_core::parse::parse(cuerpo) {
        Ok(n @ ore_core::parse::Node::Mapping { .. }) => n,
        _ => return Err(Respuesta::error(400, "the body is not a JSON object")),
    };
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let Some(fichero) = campo("fichero").map(|f| f.trim().to_string()) else {
        return Err(Respuesta::error(
            422,
            "`fichero` is the path of the file in the tree (`packages/<package>/<repository>/x.py`)",
        ));
    };
    if fichero.starts_with('/')
        || fichero.contains('\\')
        || fichero.chars().any(char::is_control)
        || fichero.split('/').any(|s| s == ".." || s.is_empty())
        || !(fichero.ends_with(".py") || fichero.ends_with(".sql") || fichero.ends_with(".java"))
    {
        return Err(Respuesta::error(
            422,
            format!("`{fichero}` is not a `.py`, `.sql` or `.java` path of the tree"),
        ));
    }
    let Some(contenido) = campo("contenido") else {
        return Err(Respuesta::error(
            422,
            "`contenido` is the file as it is in the editor",
        ));
    };
    if contenido.len() > CONTENIDO_MAXIMO {
        return Err(Respuesta::error(422, "the file is too long (256 KiB)"));
    }
    let transform = campo("transform")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    Ok(Pedido {
        fichero,
        contenido,
        transform,
    })
}

fn runtime(fichero: &str) -> &'static str {
    if fichero.ends_with(".sql") {
        "sql"
    } else if fichero.ends_with(".java") {
        "java"
    } else {
        "python"
    }
}

/// Un transform del editor, como lo lee la consola.
fn transform_json(t: &DelEditor, runtime: &str) -> Json {
    let mut m = vec![
        ("transform", Json::s(&t.clave)),
        ("runtime", Json::s(runtime)),
        ("inputs", Json::Arr(t.inputs.iter().map(Json::s).collect())),
        ("output", Json::s(&t.output)),
    ];
    if let Some(l) = t.linea {
        m.push(("linea", Json::Int(l as i64)));
    }
    if let Some(d) = &t.descripcion {
        m.push(("description", Json::s(d)));
    }
    Json::obj(m)
}

/// `(transforms, diagnostics)` del texto, con los diagnósticos en la forma
/// de los de un build (`code`, `message`, `file`, `line`, `column`, `help`).
fn derivar(p: &Pedido) -> (Vec<DelEditor>, Vec<Json>) {
    let (ts, ds) = ore_core::transformar::del_editor(Path::new(&p.fichero), &p.contenido);
    let ds = ds
        .iter()
        .map(|d| diagnostico_json(Path::new(""), d))
        .collect();
    (ts, ds)
}

/// `POST /transforms/editor {fichero, contenido}` → 200 `{fichero,
/// transforms: [{transform, runtime, inputs, output, linea, description?,
/// previewable?}], diagnostics: […]}`. En Python, `transform` es el nombre del
/// `def`; en SQL (P4), el ordinal de la sentencia, y `previewable` dice si su
/// Preview corre ya (0049 B10·4: la colección que da su consulta). Un texto
/// que no se lee no es un error de la petición: es 200 sin transforms y con su
/// `OOS2043`.
pub(crate) fn transforms_del_editor(cuerpo: &str) -> Respuesta {
    let p = match pedido(cuerpo) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let (ts, ds) = derivar(&p);
    let rt = runtime(&p.fichero);
    let ensayables = if rt == "sql" {
        sentencias_que_se_ensayan(&p.contenido)
    } else {
        Default::default()
    };
    let lista = ts
        .iter()
        .map(|t| {
            let mut j = transform_json(t, rt);
            if let (true, Json::Obj(m)) = (rt == "sql", &mut j) {
                m.insert(
                    "previewable".into(),
                    Json::Bool(ensayables.contains(&t.clave)),
                );
            }
            j
        })
        .collect();
    Respuesta::ok(Json::obj([
        ("fichero", Json::s(&p.fichero)),
        ("transforms", Json::Arr(lista)),
        ("diagnostics", Json::Arr(ds)),
    ]))
}

/// El `@transform` que se ensaya, o por qué no: 422 con los diagnósticos si
/// el texto no lo deriva (`OOS2043`), 404 si no hay ningún `def` así.
fn elegido(p: &Pedido, def: &str) -> Result<DelEditor, Respuesta> {
    let (ts, crudos) = ore_core::transformar::del_editor(Path::new(&p.fichero), &p.contenido);
    if let Some(t) = ts.iter().find(|t| t.clave == def) {
        return Ok(t.clone());
    }
    if !crudos.is_empty() {
        let ds: Vec<Json> = crudos
            .iter()
            .map(|d| diagnostico_json(Path::new(""), d))
            .collect();
        let primero = crudos.first();
        let donde = primero
            .and_then(|d| d.pos)
            .map(|x| format!("line {}: ", x.line))
            .unwrap_or_default();
        return Err(Respuesta {
            codigo: 422,
            cuerpo: Json::obj([
                (
                    "error",
                    Json::s(format!(
                        "`{def}` cannot be previewed because `{}` does not derive it: {donde}{}",
                        p.fichero,
                        primero.map(|d| d.message.as_str()).unwrap_or_default()
                    )),
                ),
                ("diagnostics", Json::Arr(ds)),
                ("motivo", Json::s("diagnostics")),
            ]),
        });
    }
    Err(con_motivo(
        Respuesta::error(
            404,
            if ts.is_empty() && p.fichero.ends_with(".java") {
                format!(
                    "`{}` has nothing to preview: no method of its class has `@Transform` (`import ore.Transform;`)",
                    p.fichero
                )
            } else if ts.is_empty() {
                format!(
                    "`{}` has nothing to preview: no top-level def of it has `@transform` (`from ore import transform`)",
                    p.fichero
                )
            } else {
                format!(
                    "`{def}` is not a `@transform` def of `{}` in the editor (its transforms: {})",
                    p.fichero,
                    ts.iter()
                        .map(|t| format!("`{}`", t.clave))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        ),
        "not_found",
    ))
}

/// 0049 B10·4 · Los ordinales de las sentencias de un `.sql` cuyo Preview corre
/// ya: las que dan una colección por su consulta.
fn sentencias_que_se_ensayan(contenido: &str) -> std::collections::BTreeSet<String> {
    use ore_core::sql_del_arbol::guion::{Sentencia, guion};
    guion(contenido)
        .map(|ts| {
            ts.iter()
                .enumerate()
                .filter(|(_, t)| matches!(t.sentencia, Sentencia::ColeccionDerivada { .. }))
                .map(|(i, _)| (i + 1).to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// 0049 B10·4 · **La celda del Preview de una colección que da su consulta**:
/// la sentencia `n` del texto del editor, si es `create or replace media
/// collection … as select …`; `None` si es otra. La consulta sobre los primeros
/// ítems, como el build (ítem a ítem), y lo que serían sus ficheros al informe
/// (`preview`), sin crear la colección ni escribir nada —ni leer los bytes de un
/// ítem que se copiaría—. Corre dentro del techo del Preview, como el de Python.
fn celda_de_preview_sql(p: &Pedido, def: &str, output: &str) -> Option<String> {
    use ore_core::sql_del_arbol::guion::{Sentencia, guion};
    let n: usize = def.parse().ok()?;
    let trozos = guion(&p.contenido).ok()?;
    let Sentencia::ColeccionDerivada { consulta, .. } = &trozos.get(n.checked_sub(1)?)?.sentencia
    else {
        return None;
    };
    let cad = |s: &str| Json::s(s).jcs();
    let col = consulta
        .lee
        .first()
        .map(ore_core::sql_del_arbol::Nombre::referencia)
        .unwrap_or_default();
    Some(format!(
        "# El Preview de una colección que da su consulta (ORE 0049 B10·4): {fichero}:{def}, sin escribir nada\n\
         {guarda}import os as _os\n\
         \n\
         if not hasattr(_ore, \"_ensayar_ficheros\"):\n    \
             raise RuntimeError(\"this session runs an older ORE SDK, which cannot preview this: restart the session\")\n\
         # Un Preview no es un build: ni su procedencia ni su informe.\n\
         _os.environ.pop(\"ORE_BUILD\", None)\n\
         _ore._ensayo({salida}, {d})\n\
         try:\n    \
             _visto = _ore._ensayar_ficheros({salida}, {col}, {consulta})\n\
         finally:\n    \
             _ore._ensayo(None)\n\
         print(\"%s · preview of %s:%s · %d files from %d items · nothing was written\" % \
         (_visto[\"output\"], {fich}, {d}, _visto[\"total\"], _visto[\"items\"]))\n\
         _ore._para_el_informe({{\"preview\": _visto}})\n",
        fichero = p.fichero.replace('\n', " "),
        guarda = ore_core::sdk::guarda_python(),
        salida = cad(output),
        d = cad(def),
        col = cad(&col),
        consulta = cad(&consulta.consulta),
        fich = cad(&p.fichero),
    ))
}

impl Servidor {
    /// `POST /puestos/{id}/preview {fichero, contenido, transform}` → 202
    /// `{celda, puesto, preview: {transform, runtime, inputs, output, linea}}`;
    /// o el no, con `motivo` como en `POST /builds`.
    pub(crate) fn ensayar_en_puesto(
        &self,
        sujeto: &Identidad,
        id: &str,
        cuerpo: &str,
    ) -> Respuesta {
        if crate::puestos::es_agente(sujeto) {
            return con_motivo(
                Respuesta::error(403, "an agent does not preview: a person does"),
                "forbidden",
            );
        }
        let p = match pedido(cuerpo) {
            Ok(p) => p,
            Err(r) => return con_motivo(r, "invalid"),
        };
        let Some(def) = p.transform.clone() else {
            return con_motivo(
                Respuesta::error(
                    422,
                    "a Preview names the `@transform` it runs: `transform` is the name of its def",
                ),
                "invalid",
            );
        };
        let t = match elegido(&p, &def) {
            Ok(t) => t,
            Err(r) => return r,
        };
        // 0049 B10·4: de SQL, por ahora sólo la colección que da su consulta
        // (lo demás, ADR 0055 P4). `transform` es el ordinal de la sentencia.
        let sql = if runtime(&p.fichero) == "sql" {
            match celda_de_preview_sql(&p, &def, &t.output) {
                Some(c) => Some(c),
                None => {
                    return con_motivo(
                        Respuesta::error(
                            422,
                            "Preview of SQL statements comes later (ADR 0055 P4): build it, or run its `SELECT`; \
                             `create or replace media collection … as select` previews already (ADR 0049 B10)",
                        ),
                        "not_supported",
                    );
                }
            }
        } else {
            None
        };
        // Las colecciones de `inputs`, fijadas en la rama de la sesión, como al
        // declarar un transform (fuera del candado: leer el árbol hace un `fetch`).
        let rama = self.con_los_puestos(|l| l.get(id).map(|x| x.rama.clone()));
        let fijadas = match rama {
            Some(r) => self.fijar_colecciones(r.as_deref(), &t.inputs),
            None => Default::default(),
        };
        let rt = runtime(&p.fichero);
        // 0055 T1·7 (JT4): Java, con el arnés del build en modo Preview: el texto
        // del editor en lugar del suyo, y los demás `.java` del paquete como
        // están en la rama de la sesión.
        let celda = if rt == "java" {
            let rama = self.con_los_puestos(|l| l.get(id).and_then(|x| x.rama.clone()));
            let mut fuentes = Vec::new();
            let r = self.leyendo_en(rama.as_deref(), |raiz| {
                match crate::builds::fuentes_java(raiz, &p.fichero, Some(&p.contenido)) {
                    Ok(f) => {
                        fuentes = f;
                        Respuesta::ok(Json::obj([]))
                    }
                    Err(r) => r,
                }
            });
            if r.codigo != 200 {
                return crate::builds::con_motivo_por_defecto(r);
            }
            crate::builds::arnes_java(
                &p.fichero,
                &p.fichero,
                &def,
                &fuentes,
                crate::builds::ModoJava::Preview { output: &t.output },
            )
        } else {
            sql.unwrap_or_else(|| {
                crate::builds::arnes_de_preview(&p.fichero, &p.contenido, &def, &t.output)
            })
        };
        let lenguaje = if rt == "java" { "java" } else { "python" };
        let ensayo = Ensayo {
            techo: Transform {
                nombre: def.clone(),
                inputs: t.inputs.clone(),
                output: t.output.clone(),
                fijadas,
                techo: true,
            },
            fichero: p.fichero.clone(),
            def,
        };
        let mut r = self.encolar_ensayo(sujeto, id, lenguaje, celda, ensayo);
        if r.codigo == 202 {
            if let Json::Obj(m) = &mut r.cuerpo {
                m.insert("preview".into(), transform_json(&t, rt));
            }
            r
        } else {
            crate::builds::con_motivo_por_defecto(r)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bien<T>(r: Result<T, Respuesta>) -> T {
        match r {
            Ok(x) => x,
            Err(r) => panic!("{} {}", r.codigo, r.cuerpo.jcs()),
        }
    }

    const FUENTE: &str = "from ore import transform, over, write\n\n\n\
        @transform(inputs=[\"ventas.clientes\"], output=\"ventas.limpios\")\n\
        def limpios():\n    return write(\"ventas.limpios\", over(\"ventas.clientes\"))\n\n\n\
        @transform(inputs=[\"ventas.default.clientes\", \"ventas.pedidos\"], output=\"ventas.otros\")\n\
        def otros():\n    return write(\"ventas.otros\", over(\"ventas.pedidos\"))\n";

    fn cuerpo(fichero: &str, contenido: &str, transform: Option<&str>) -> String {
        let mut m = vec![
            ("fichero", Json::s(fichero)),
            ("contenido", Json::s(contenido)),
        ];
        if let Some(t) = transform {
            m.push(("transform", Json::s(t)));
        }
        Json::obj(m).jcs()
    }

    #[test]
    fn el_editor_lista_sus_transforms_en_su_orden() {
        let r = transforms_del_editor(&cuerpo("packages/ventas/etl/limpios.py", FUENTE, None));
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        let j = r.cuerpo.jcs();
        assert!(j.contains(r#""diagnostics":[]"#), "{j}");
        let limpios = j.find(r#""transform":"limpios""#).expect(&j);
        let otros = j.find(r#""transform":"otros""#).expect(&j);
        assert!(limpios < otros, "{j}");
        assert!(j.contains(r#""linea":4"#), "{j}");
        assert!(j.contains(r#""linea":9"#), "{j}");
        // En forma corta, como el techo y el documento.
        assert!(
            j.contains(r#""inputs":["ventas.clientes","ventas.pedidos"]"#),
            "{j}"
        );
        assert!(j.contains(r#""runtime":"python""#), "{j}");
    }

    /// 0055 T1·7 (JT4) · Un `.java` del editor: sus `@Transform`, con su runtime,
    /// su línea y su descripción; y uno que no se lee, su `OOS2043` con la línea.
    #[test]
    fn el_editor_lista_los_transforms_de_java() {
        let java = "import static ore.Ore.*;\nimport ore.Transform;\n\npublic class Limpios {\n    \
                    static final String IN = \"ventas.default.clientes\";\n\n    /** Los limpios. */\n    \
                    @Transform(inputs = {IN}, output = \"ventas.limpios\")\n    \
                    public static Object limpios() throws Exception { return write(\"ventas.limpios\", over(IN)); }\n}\n";
        let r = transforms_del_editor(&cuerpo("packages/ventas/etl/Limpios.java", java, None));
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        let j = r.cuerpo.jcs();
        assert!(j.contains(r#""transform":"limpios""#), "{j}");
        assert!(j.contains(r#""runtime":"java""#), "{j}");
        assert!(j.contains(r#""linea":8"#), "{j}");
        assert!(j.contains(r#""inputs":["ventas.clientes"]"#), "{j}");
        assert!(j.contains(r#""description":"Los limpios.""#), "{j}");
        assert!(j.contains(r#""diagnostics":[]"#), "{j}");
        let roto = java.replace("{IN}", "{\"ventas.\" + \"clientes\"}");
        let j = transforms_del_editor(&cuerpo("packages/ventas/etl/Limpios.java", &roto, None))
            .cuerpo
            .jcs();
        assert!(j.contains(r#""transforms":[]"#), "{j}");
        assert!(
            j.contains(r#""code":"OOS2043""#) && j.contains(r#""line":8"#),
            "{j}"
        );
    }

    #[test]
    fn un_texto_que_no_se_lee_es_oos2043_con_su_linea() {
        let roto = FUENTE.replace("def otros():", "def otros(:");
        let r = transforms_del_editor(&cuerpo("packages/ventas/etl/limpios.py", &roto, None));
        assert_eq!(r.codigo, 200);
        let j = r.cuerpo.jcs();
        assert!(j.contains(r#""transforms":[]"#), "{j}");
        assert!(j.contains(r#""code":"OOS2043""#), "{j}");
        assert!(j.contains(r#""line":10"#), "{j}");
        assert!(j.contains("this file is not Python the session"), "{j}");
        assert!(
            j.contains(r#""file":"packages/ventas/etl/limpios.py""#),
            "{j}"
        );

        // Y el Preview de un `def` de ese texto: 422, con lo mismo.
        let p = bien(pedido(&cuerpo(
            "packages/ventas/etl/limpios.py",
            &roto,
            Some("limpios"),
        )));
        let r = elegido(&p, "limpios").unwrap_err();
        assert_eq!(r.codigo, 422);
        let j = r.cuerpo.jcs();
        assert!(j.contains(r#""motivo":"diagnostics""#), "{j}");
        assert!(j.contains("line 10: this file is not Python"), "{j}");
    }

    #[test]
    fn se_elige_un_def_por_su_nombre_entre_varios() {
        let p = bien(pedido(&cuerpo(
            "packages/ventas/etl/limpios.py",
            FUENTE,
            Some("otros"),
        )));
        let t = bien(elegido(&p, "otros"));
        assert_eq!(t.output, "ventas.otros");
        assert_eq!(t.inputs, ["ventas.clientes", "ventas.pedidos"]);
        let r = elegido(&p, "nadie").unwrap_err();
        assert_eq!(r.codigo, 404);
        let j = r.cuerpo.jcs();
        assert!(j.contains("`limpios`, `otros`"), "{j}");
        assert!(j.contains(r#""motivo":"not_found""#), "{j}");
    }

    /// La puerta del Preview: lo que pasa sin escribir, y lo que no.
    #[test]
    fn en_un_preview_solo_pasa_lo_que_no_escribe() {
        use crate::rutas::no_escribe;
        for pasa in [
            "puestos/p/transform",
            "puestos/p/sql",
            "puestos/p/latido",
            "puestos/p/celdas/3/salida",
            "federation/read",
            "colecciones/legal/archivo/contratos/items/resolver",
            "media/legal/archivo/contratos/urls",
            "v1/ventas/namespaces/default/tables/limpios/metrics",
        ] {
            let seg: Vec<&str> = pasa.split('/').collect();
            assert!(no_escribe(&seg), "{pasa}");
        }
        for no in [
            "v1/ventas/namespaces/default/tables/limpios",
            "v1/ventas/namespaces/default/tables",
            "v1/transactions/commit",
            "documentos",
            "paquetes",
            "datasets/ventas/limpios/confirmar",
            "media/legal/archivo/contratos/transactions",
            "media/legal/archivo/contratos/transactions/t1/commit",
            "arbol/packages/x.yaml",
            "puestos",
        ] {
            let seg: Vec<&str> = no.split('/').collect();
            assert!(!no_escribe(&seg), "{no}");
        }
    }

    #[test]
    fn el_pedido_dice_un_fichero_del_arbol_y_su_texto() {
        for malo in [
            r#"{"contenido": "x"}"#,
            r#"{"fichero": "/x.py", "contenido": "x"}"#,
            r#"{"fichero": "a/../x.py", "contenido": "x"}"#,
            r#"{"fichero": "x.txt", "contenido": "x"}"#,
            r#"{"fichero": "x\ny.py", "contenido": "x"}"#,
            r#"{"fichero": "x.py"}"#,
        ] {
            assert_eq!(pedido(malo).unwrap_err().codigo, 422, "{malo}");
        }
        assert_eq!(pedido("[]").unwrap_err().codigo, 400);
        let largo = "#".repeat(CONTENIDO_MAXIMO + 1);
        assert_eq!(
            pedido(&cuerpo("x.py", &largo, None)).unwrap_err().codigo,
            422
        );
    }

    /// 0049 B10·4: de un `.sql`, la colección que da su consulta se ensaya —la
    /// consulta sobre los primeros ítems, lo que serían sus ficheros al informe,
    /// sin escribir—; otra sentencia, todavía no (`None`: el 422 de siempre).
    #[test]
    fn el_preview_de_una_coleccion_que_da_su_consulta() {
        let sql = "select count(*) from ventas.docs;\n\
                   create or replace media collection ventas.paginas media image formats (png) as\n\
                   select p.name, p.data from ventas.docs as c cross join lateral functions.f(c._item) as p;\n\
                   create or replace dataset ventas.r as select c.path from ventas.docs as c;\n";
        let p = bien(pedido(&cuerpo(
            "packages/ventas/etl/paginas.sql",
            sql,
            Some("2"),
        )));
        let t = bien(elegido(&p, "2"));
        assert_eq!(
            (t.inputs.as_slice(), t.output.as_str()),
            (&["ventas.docs".to_string()][..], "ventas.paginas")
        );
        let c = celda_de_preview_sql(&p, "2", &t.output).expect("se ensaya");
        // y el editor lo sabe: la 2 se ensaya, la 3 (un dataset) todavía no
        let j = transforms_del_editor(&cuerpo("packages/ventas/etl/paginas.sql", sql, None))
            .cuerpo
            .jcs();
        assert!(
            j.contains(r#"{"inputs":["ventas.docs"],"linea":2,"output":"ventas.paginas","previewable":true,"runtime":"sql","transform":"2"}"#)
                && j.contains(r#""previewable":false"#),
            "{j}"
        );
        assert!(c.contains(&ore_core::sdk::guarda_python()), "{c}");
        assert!(
            c.contains("_ore._ensayo(\"ventas.paginas\", \"2\")")
                && c.contains("_ore._ensayar_ficheros(\"ventas.paginas\", \"ventas.docs\", \"select p.name, p.data from ventas.docs as c cross join lateral functions.f(c._item) as p\")")
                && c.contains("_ore._ensayo(None)")
                && c.contains("_ore._para_el_informe({\"preview\": _visto})")
                && c.contains("_os.environ.pop(\"ORE_BUILD\", None)"),
            "{c}"
        );
        if let Ok(dir) = std::env::var("ORE_CELDAS_GENERADAS") {
            let _ = std::fs::create_dir_all(&dir);
            std::fs::write(Path::new(&dir).join("92-preview-coleccion.py"), &c).unwrap();
        }
        // la que escribe un dataset (y lo que no es transform), todavía no
        assert!(celda_de_preview_sql(&p, "3", "ventas.r").is_none());
        assert!(celda_de_preview_sql(&p, "1", "x").is_none());
        assert!(celda_de_preview_sql(&p, "9", "x").is_none());
        assert!(celda_de_preview_sql(&p, "uno", "x").is_none());
    }
}
