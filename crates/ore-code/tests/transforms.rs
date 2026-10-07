//! OOS v1alpha25 `01` §5.1: lo que un `@transform` declara, leído sin
//! ejecutar.

use ore_code::python;
use ore_code::transform::{Produccion, documento};

fn uno(fuente: &str) -> Result<Produccion, Vec<String>> {
    let d = python::derivar(fuente, "etl/t.py");
    assert!(d.sintaxis.is_empty(), "{:?}", d.sintaxis);
    assert_eq!(d.transforms.len(), 1, "{:?}", d.transforms);
    d.transforms[0]
        .resultado
        .clone()
        .map_err(|fs| fs.into_iter().map(|f| f.mensaje).collect())
}

#[test]
fn literales_y_constantes_del_modulo() {
    let p = uno(
        "from ore import transform\n\nPEDIDOS = \"ventas.pedidos\"\nSALIDA: str = \"ventas.resumen\"\n\n\
         @transform(inputs=[PEDIDOS, \"ventas.clientes\", PEDIDOS], output=SALIDA)\n\
         def resumen():\n    \"\"\"\n    El total por país.\n\n    Más.\n    \"\"\"\n",
    )
    .expect("se deriva");
    assert_eq!(p.runtime, "python");
    assert_eq!(p.entrypoint, "etl/t.py:resumen");
    assert_eq!(p.descripcion.as_deref(), Some("El total por país."));
    // Sin repetir: la primera vez cuenta.
    assert_eq!(p.inputs, ["ventas.pedidos", "ventas.clientes"]);
    assert_eq!(p.output, "ventas.resumen");
    assert_eq!(p.nombre(), "ventas__resumen");
}

#[test]
fn una_coleccion_se_nombra_con_ore_collection() {
    for importar in [
        "import ore\nfrom ore import transform\n",
        "import ore as o\nfrom ore import transform\n",
    ] {
        let col = if importar.contains(" as o") {
            "o.collection"
        } else {
            "ore.collection"
        };
        let p = uno(&format!(
            "{importar}CONTRATOS = \"ventas.contratos\"\n\n\
             @transform(inputs=[{col}(CONTRATOS)], output={col}(\"ventas.resumenes\"))\n\
             def resumir():\n    ...\n"
        ))
        .expect("se deriva");
        assert_eq!(p.inputs, ["ventas.contratos"]);
        assert_eq!(p.output, "ventas.resumenes");
        assert_eq!(p.descripcion, None);
    }
}

#[test]
fn lo_que_no_se_lee_sin_ejecutar_es_oos2043() {
    for (fuente, se_dice) in [
        (
            "A = \"x.a\"\nA = \"x.b\"\n@transform(inputs=[A], output=\"x.c\")\ndef f(): ...\n",
            "is bound 2 times",
        ),
        (
            "B = \"x\"\nA = B + \".a\"\n@transform(inputs=[A], output=\"x.c\")\ndef f(): ...\n",
            "is not bound to a string literal",
        ),
        (
            "@transform(inputs=[A], output=\"x.c\")\ndef f(): ...\nA = \"x.a\"\n",
            "is bound after the `def`",
        ),
        (
            "if True:\n    A = \"x.a\"\n@transform(inputs=[A], output=\"x.c\")\ndef f(): ...\n",
            "is not bound to a string literal",
        ),
        (
            "A = \"x.a\"\nA += \"b\"\n@transform(inputs=[A], output=\"x.c\")\ndef f(): ...\n",
            "is bound 2 times",
        ),
        (
            "@transform(inputs=[\"x.\" + \"a\"], output=\"x.c\")\ndef f(): ...\n",
            "is not a string literal",
        ),
        (
            "@transform(inputs=(\"x.a\",), output=\"x.c\")\ndef f(): ...\n",
            "`inputs` is not a literal list",
        ),
        (
            "@transform(inputs=[\"x.a\"])\ndef f(): ...\n",
            "without `output`",
        ),
        (
            "@transform(inputs=[], output=\"x.c\", mode=\"append\")\ndef f(): ...\n",
            "has no argument `mode`",
        ),
        (
            "@transform\ndef f(): ...\n",
            "without `inputs=` or `output=`",
        ),
        (
            "@transform(inputs=[], output=collection(\"x.c\"))\ndef f(): ...\n",
            "is not a string literal",
        ),
    ] {
        let fuente = format!("from ore import transform\n{fuente}");
        let fallos = uno(&fuente).expect_err(&fuente);
        assert!(
            fallos.iter().any(|f| f.contains(se_dice)),
            "{fuente}\n{fallos:?}"
        );
    }
}

#[test]
fn solo_el_transform_de_ore_y_solo_arriba() {
    // Un `transform` que no es el de `ore` no es un transform.
    let d = python::derivar(
        "from otra import transform\n@transform(inputs=[], output=\"x.c\")\ndef f(): ...\n",
        "t.py",
    );
    assert!(d.transforms.is_empty());
    assert!(!d.defs[0].transformada);
    // Dentro de una clase: un aviso, no un transform.
    let d = python::derivar(
        "from ore import transform\nclass C:\n    @transform(inputs=[], output=\"x.c\")\n    \
         def f(self): ...\n",
        "t.py",
    );
    assert!(d.transforms.is_empty());
    assert!(
        d.avisos[0].mensaje.contains("`@transform`"),
        "{:?}",
        d.avisos
    );
    // Con alias, sí; y `inputs` vacía vale.
    let d = python::derivar(
        "from ore import transform as t\n@t(inputs=[], output=\"x.c\")\ndef f(): ...\n",
        "t.py",
    );
    assert!(d.defs[0].transformada);
    assert_eq!(
        d.transforms[0].resultado.as_ref().map(|p| p.inputs.len()),
        Ok(0)
    );
}

#[test]
fn el_documento_del_caso_de_conformidad() {
    let p = uno(
        "from ore import transform, over, write\n\nPEDIDOS = \"ventas.pedidos\"\n\n\n\
         @transform(inputs=[PEDIDOS, \"ventas.clientes\"], output=\"ventas.resumen\")\n\
         def resumen():\n    \"\"\"El total por país.\"\"\"\n    \
         return write(\"ventas.resumen\", over(PEDIDOS), mode=\"overwrite\")\n",
    )
    .expect("se deriva");
    let p = Produccion {
        entrypoint: "etl/transforms/resumen.py:resumen".into(),
        ..p
    };
    let esperado = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../vendor/oos/conformance/v1alpha25/valid/a-python-transform/input/packages/\
             ventas/etl/pipeline/ventas.resumen.yaml",
    ))
    .expect("el submódulo trae el caso")
    .replace("\r\n", "\n");
    assert_eq!(documento(&p, "ventas"), esperado);
}
