//! La derivación de TypeScript contra OOS v1alpha23 `01`, caso a caso.

use super::*;
use crate::emitir;
use crate::firma::{Firma, Salida, Tipo};

const RUTA: &str = "textos/functions/repeat.ts";

fn firma(ruta: &str, fuente: &str) -> Firma {
    let d = derivar(fuente, ruta);
    assert!(d.sintaxis.is_empty(), "sintaxis: {:?}", d.sintaxis);
    assert!(d.version.is_empty(), "no borrable: {:?}", d.version);
    assert_eq!(d.funciones.len(), 1, "{d:?}");
    match &d.funciones[0].resultado {
        Ok(f) => f.clone(),
        Err(fs) => panic!("no se deriva: {fs:#?}"),
    }
}

/// Los mensajes de por qué no se deriva.
fn fallos(ruta: &str, fuente: &str) -> Vec<String> {
    let d = derivar(fuente, ruta);
    assert_eq!(d.funciones.len(), 1, "{d:?}");
    match &d.funciones[0].resultado {
        Ok(f) => panic!("se derivó y no debía: {f:?}"),
        Err(fs) => fs.iter().map(|f| f.mensaje.clone()).collect(),
    }
}

fn tipo_de(param: &str, imports: &str) -> Result<Tipo, Vec<String>> {
    let fuente = format!(
        "{imports}\nexport default function repeat(x: {param}): string {{ return \"\"; }}\n"
    );
    let d = derivar(&fuente, RUTA);
    match &d.funciones[0].resultado {
        Ok(f) => Ok(f.entrada[0].tipo.clone()),
        Err(fs) => Err(fs.iter().map(|f| f.mensaje.clone()).collect()),
    }
}

const ORE: &str = "import type { Integer, Decimal, Money, Quantity, LocalDate, LocalTime, LocalDateTime, Media } from \"ore\";";

#[test]
fn que_es_un_fichero_de_funciones() {
    assert!(es_de_funciones("functions/a.ts"));
    assert!(es_de_funciones("x/functions/billing/quoteOrder.ts"));
    assert!(!es_de_funciones("x/lib/a.ts"));
    assert!(!es_de_funciones("x/functions/a.d.ts"));
    assert!(!es_de_funciones("x/functions/a.test.ts"));
    assert!(!es_de_funciones("x/functions/a.spec.ts"));
    assert!(!es_de_funciones("x/functions/a.tsx"));
    assert!(!es_de_funciones("x/functions/a.js"));
    assert!(!es_de_funciones("functions.ts"));
    assert!(!es_de_funciones("x/myfunctions/a.ts"));
    assert_eq!(
        nombre_del_fichero("x/functions/quoteOrder.ts"),
        "quoteOrder"
    );
}

#[test]
fn una_funcion_sobre_sus_parametros() {
    let f = firma(
        RUTA,
        r#"import type { Integer } from "ore";

export default function repeat(text: string, times: Integer = 1): string {
  return Array(times).fill(text).join(" ");
}
"#,
    );
    assert_eq!(f.nombre, "repeat");
    assert_eq!(f.entrypoint, RUTA);
    assert_eq!(f.runtime(), "node");
    assert_eq!(f.api_version(), "oos.dev/v1alpha23");
    assert_eq!(
        emitir::documento(&f, "ventas"),
        "# generado por ore desde textos/functions/repeat.ts · se edita el código, no este fichero\n\
         apiVersion: oos.dev/v1alpha23\n\
         kind: Function\n\
         metadata: { name: repeat, namespace: ventas }\n\
         spec:\n  runtime: node\n  entrypoint: textos/functions/repeat.ts\n  input:\n    \
         text: { type: String, required: true }\n    times: { type: Integer }\n  output: { type: String }\n"
    );
    // Con dueño, sigue siendo v1alpha23: es la más baja que la describe.
    assert!(
        emitir::documento_con_dueno(&f, "ventas", Some("user:ana"))
            .contains("apiVersion: oos.dev/v1alpha23\n")
    );
}

#[test]
fn config_y_una_vuelta_asincrona_que_es_un_interface() {
    let f = firma(
        "riesgo/functions/riesgo.ts",
        r#"import type { Decimal } from "ore";

export const config = {
  over: "ventas.clientes",
  reads: ["ventas.pedidos"],
  models: ["extractor"],
  timeout: "60s",
} as const;

interface Riesgo {
  nivel: string;
  total: number;
  motivo?: string;
}

/**
 * El riesgo de un cliente por lo que ha comprado.
 *
 * @param umbral a partir de cuánto es alto
 */
export default async function riesgo(cliente: Record<string, unknown>, umbral: Decimal<12, 2>, moneda: string = "EUR"): Promise<Riesgo> {
  return { nivel: "bajo", total: 0 };
}
"#,
    );
    assert_eq!(
        emitir::documento(&f, "ventas"),
        "# generado por ore desde riesgo/functions/riesgo.ts · se edita el código, no este fichero\n\
         apiVersion: oos.dev/v1alpha23\n\
         kind: Function\n\
         metadata:\n  name: riesgo\n  namespace: ventas\n  description: El riesgo de un cliente por lo que ha comprado.\n\
         spec:\n  runtime: node\n  entrypoint: riesgo/functions/riesgo.ts\n  over: ventas.clientes\n  \
         reads: [ventas.pedidos]\n  models: [modelo/extractor]\n  input:\n    \
         umbral: { type: 'Decimal<12, 2>', required: true }\n    moneda: { type: String }\n  \
         output:\n    nivel: { type: String, required: true }\n    total: { type: Float, required: true }\n    \
         motivo: { type: String }\n  limits: { timeout: '60s' }\n"
    );
}

#[test]
fn la_tabla_de_tipos_entera() {
    let f = firma(
        "pedidos/functions/lineas.ts",
        r#"import type { Integer, LocalDate, LocalDateTime, LocalTime, Money as Dinero } from "ore";
import type * as ore from "ore";

type Linea = {
  producto: string;
  precio: Dinero<"EUR", 2>;
  notas?: string | null;
};

interface Pedido {
  id: bigint;
  lineas: readonly Linea[];
  entrega: LocalTime;
}

/** Las líneas de un pedido que pesan. */
export default function lineas(
  pedido: Pedido,
  corte: Date,
  firma: Uint8Array,
  tasa: ore.Decimal<5, 4>,
  dia: LocalDate,
  visto: LocalDateTime | null,
  intentos: Integer,
  ratio: number,
  activo: boolean,
  peso?: ore.Quantity<"kg", 3>,
): Array<Linea> {
  return [...pedido.lineas];
}
"#,
    );
    let linea = "Struct<producto: String, precio: Money<EUR, 2>, notas: String>";
    let ent: Vec<(String, String, bool)> = f
        .entrada
        .iter()
        .map(|c| (c.nombre.clone(), c.tipo.to_string(), c.requerido))
        .collect();
    let esperado: Vec<(String, String, bool)> = [
        (
            "pedido",
            format!("Struct<id: Integer, lineas: list<{linea}>, entrega: Time>"),
            true,
        ),
        ("corte", "DateTimeTz".into(), true),
        ("firma", "Opaque".into(), true),
        ("tasa", "Decimal<5, 4>".into(), true),
        ("dia", "Date".into(), true),
        ("visto", "DateTime".into(), false),
        ("intentos", "Integer".into(), true),
        ("ratio", "Float".into(), true),
        ("activo", "Boolean".into(), true),
        ("peso", "Quantity<kg, 3>".into(), false),
    ]
    .into_iter()
    .map(|(a, b, c)| (a.to_string(), b, c))
    .collect();
    assert_eq!(ent, esperado);
    assert_eq!(
        f.salida,
        Salida::Valor(Tipo::Lista(Box::new(match &f.entrada[0].tipo {
            Tipo::Struct(cs) => match &cs[1].1 {
                Tipo::Lista(x) => (**x).clone(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        })))
    );
    assert_eq!(
        f.descripcion.as_deref(),
        Some("Las líneas de un pedido que pesan.")
    );
    // El contrato de Node sabe que `id` llega como `bigint` y `intentos` como
    // `number`, aunque el documento diga `Integer` en los dos.
    assert!(
        f.entrada[0]
            .tipo
            .forma()
            .starts_with("Struct<id: BigInt, lineas: list<")
    );
    assert_eq!(f.entrada[6].tipo.forma(), "Integer");
}

#[test]
fn media_de_una_coleccion() {
    assert_eq!(
        tipo_de("Media<\"legal.archivo.contratos\">", ORE),
        Ok(Tipo::Media("legal.archivo.contratos".into()))
    );
    assert!(tipo_de("Media<\"no es\">", ORE).is_err());
    assert!(tipo_de("Media", ORE).is_err());
}

#[test]
fn los_nombres_se_leen_por_lo_que_son() {
    // Sin importar: no es el de `ore`, y la ayuda lo dice.
    let e = tipo_de("Integer", "").unwrap_err();
    assert!(e[0].contains("no está definido"), "{e:?}");
    // De otro módulo.
    assert!(tipo_de("Integer", "import type { Integer } from \"otro\";").is_err());
    // Un `Money` del fichero tapa al de `ore`.
    assert!(
        tipo_de(
            "Money<\"EUR\", 2>",
            "import type { Money } from \"ore\";\ntype Money<U, S> = string;"
        )
        .is_err()
    );
    // Un `Date` del fichero tapa al global.
    assert!(tipo_de("Date", "class Date {}").is_err());
    // El defecto de `ore` no es un espacio de nombres de tipos.
    assert!(tipo_de("o.Integer", "import o from \"ore\";").is_err());
    // `import { type X }` vale como `import type`.
    assert_eq!(
        tipo_de("Integer", "import { type Integer } from \"ore\";"),
        Ok(Tipo::Integer)
    );
}

#[test]
fn lo_que_no_tiene_tipo() {
    for t in [
        "Record<string, number>",
        "Map<string, number>",
        "Set<string>",
        "[string, number]",
        "any",
        "unknown",
        "object",
        "\"corto\" | \"largo\"",
        "string | number",
        "number[][]",
        "(number | null)[]",
        "Promise<string>",
        "() => void",
        "A & B",
        "keyof X",
    ] {
        assert!(tipo_de(t, ORE).is_err(), "{t} se derivó");
    }
    assert!(tipo_de("Decimal<40, 2>", ORE).unwrap_err()[0].contains("fuera de rango"));
    assert!(tipo_de("Decimal<5>", ORE).is_err());
    assert!(tipo_de("Money<EUR, 2>", ORE).is_err());
    assert_eq!(tipo_de("Decimal", ORE), Ok(Tipo::Decimal));
    assert_eq!(
        tipo_de("ReadonlyArray<string>", ORE),
        Ok(Tipo::Lista(Box::new(Tipo::String)))
    );
    assert_eq!(tipo_de("(string)", ORE), Ok(Tipo::String));
}

#[test]
fn el_nombre_es_el_del_fichero() {
    let e = fallos(
        RUTA,
        "export default function repetir(text: string): string { return text; }",
    );
    assert!(e[0].contains("se llama `repetir`"), "{e:?}");
    let e = fallos(
        RUTA,
        "export default function (text: string): string { return text; }",
    );
    assert!(e[0].contains("no tiene nombre"), "{e:?}");
    let e = fallos(
        "x/functions/quote-order.ts",
        "export default function quoteOrder(): string { return \"\"; }",
    );
    assert!(e.iter().any(|m| m.contains("no da un nombre")), "{e:?}");
}

#[test]
fn lo_exportado_por_defecto_que_no_es_una_funcion() {
    for (fuente, que) in [
        ("export default (text: string): string => text;", "flecha"),
        (
            "const repeat = (t: string): string => t;\nexport default repeat;",
            "un nombre",
        ),
        ("export default class Repeat {}", "una clase"),
        ("export default 42;", "un valor"),
        (
            "function repeat(): string { return \"\"; }\nexport { repeat as default };",
            "as default",
        ),
    ] {
        let e = fallos(RUTA, fuente);
        assert!(e[0].contains(que), "{fuente}: {e:?}");
        assert!(derivar(fuente, RUTA).defs.is_empty(), "{fuente}");
    }
}

#[test]
fn sin_export_default_es_un_modulo_de_ayuda() {
    let d = derivar(
        "export function unir(xs: string[]): string { return xs.join(\" \"); }",
        "x/functions/unir.ts",
    );
    assert!(d.funciones.is_empty() && d.defs.is_empty());
    // Y fuera de `functions/`, la función está (un `entrypoint` puede nombrarla)
    // pero no es una función publicada.
    let d = derivar(
        "export default function repeat(): string { return \"\"; }",
        "x/lib/repeat.ts",
    );
    assert!(d.funciones.is_empty());
    assert_eq!(d.defs.len(), 1);
    assert!(!d.defs[0].decorada);
}

#[test]
fn config_se_lee_sin_ejecutar() {
    let base = "export default function repeat(text: string): string { return text; }";
    let con = |c: &str| format!("{c}\n{base}");
    let f = firma(
        RUTA,
        &con("export const config = { timeout: `30s` } satisfies Config;"),
    );
    assert_eq!(f.timeout.as_deref(), Some("30s"));
    for (c, msg) in [
        (
            "const PLAZO = \"30s\";\nexport const config = { timeout: PLAZO };",
            "no es una cadena literal",
        ),
        (
            "export const config = { sources: [\"crm\"] };",
            "no tiene la clave `sources`",
        ),
        ("export let config = { timeout: \"30s\" };", "no es `const`"),
        (
            "const config = { timeout: \"30s\" };\nexport { config };",
            "se exporta aparte",
        ),
        ("const config = { timeout: \"30s\" };", "no se exporta"),
        (
            "export const config = { ...otra };",
            "no se lee sin ejecutar",
        ),
        (
            "export const config = { timeout: `${1}s` };",
            "no es una cadena literal",
        ),
        (
            "export const config = { reads: \"ventas.pedidos\" };",
            "no es una lista literal",
        ),
        ("export const config = hazla();", "no es un objeto literal"),
    ] {
        let e = fallos(RUTA, &con(c));
        assert!(e.iter().any(|m| m.contains(msg)), "{c}: {e:?}");
    }
}

#[test]
fn los_parametros() {
    let e = fallos(
        RUTA,
        "export default function repeat(text, n: number): string { return text; }",
    );
    assert!(e[0].contains("`text` sin anotar"), "{e:?}");
    let e = fallos(
        RUTA,
        "export default function repeat(text: string, ...partes: string[]): string { return text; }",
    );
    assert!(e[0].contains("resto"), "{e:?}");
    let e = fallos(
        RUTA,
        "export default function repeat({ text }: { text: string }): string { return text; }",
    );
    assert!(e[0].contains("desestructurado"), "{e:?}");
    let e = fallos(
        RUTA,
        "export default function repeat<T>(x: T): string { return \"\"; }",
    );
    assert!(e.iter().any(|m| m.contains("genérica")), "{e:?}");
    let e = fallos(
        "r/functions/riesgo.ts",
        "export const config = { over: \"ventas.clientes\" };\nexport default function riesgo(): string { return \"\"; }",
    );
    assert!(e[0].contains("no recibe la fila"), "{e:?}");
    // Con `over`, la fila no entra en `input` y su anotación no se lee.
    let f = firma(
        "r/functions/riesgo.ts",
        "export const config = { over: \"ventas.clientes\" };\nexport default function riesgo(fila: any, n: number): string { return \"\"; }",
    );
    assert_eq!(f.entrada.len(), 1);
    assert_eq!(f.over.as_deref(), Some("ventas.clientes"));
}

#[test]
fn lo_que_devuelve() {
    for (r, msg) in [
        (" function repeat(text: string)", "no dice lo que devuelve"),
        (" function repeat(text: string): void", "devuelve `void`"),
        (
            " async function repeat(text: string): Promise<void>",
            "devuelve `void`",
        ),
        (
            " function repeat(text: string): undefined",
            "devuelve `undefined`",
        ),
        (
            " async function repeat(text: string): Promise",
            "sin el tipo",
        ),
    ] {
        let fuente = format!("export default{r} {{ }}");
        let e = fallos(RUTA, &fuente);
        assert!(e.iter().any(|m| m.contains(msg)), "{r}: {e:?}");
    }
    let e = fallos(
        RUTA,
        "export default async function repeat(text: string): string { return text; }",
    );
    assert!(e[0].contains("devuelve `Promise<T>`"), "{e:?}");
    // Un objeto literal en la vuelta es el mapa de `output`.
    let f = firma(
        RUTA,
        "export default function repeat(text: string): { resultado: string; longitud: number } { return null!; }",
    );
    assert!(matches!(&f.salida, Salida::Campos(cs) if cs.len() == 2));
    // Un `type` de objeto también, y `extends` pone los heredados delante.
    let f = firma(
        RUTA,
        "interface Base { id: string; total: number }\ninterface R extends Base { total: bigint; nota?: string }\nexport default function repeat(): R { return null!; }",
    );
    let Salida::Campos(cs) = &f.salida else {
        panic!()
    };
    let v: Vec<_> = cs
        .iter()
        .map(|c| (c.nombre.as_str(), c.tipo.to_string(), c.requerido))
        .collect();
    assert_eq!(
        v,
        vec![
            ("id", "String".into(), true),
            ("total", "Integer".into(), true),
            ("nota", "String".into(), false)
        ]
    );
}

#[test]
fn los_objetos() {
    for (decl, msg) in [
        ("interface X { f(): void }", "un método"),
        ("interface X { [k: string]: number }", "firma de índice"),
        ("interface X { a: X }", "se contiene a sí mismo"),
        ("interface X<T> { a: T }", "genérico"),
        (
            "interface X extends Y { a: string }",
            "no es un `interface` del fichero",
        ),
        ("type X = string;", "alias de otro tipo"),
        ("interface X { a }", "sin tipo"),
    ] {
        let e = tipo_de("X", decl).unwrap_err();
        assert!(e.iter().any(|m| m.contains(msg)), "{decl}: {e:?}");
    }
    // Uno de otro fichero no se lee.
    let e = tipo_de("X", "import type { X } from \"./tipos.ts\";").unwrap_err();
    assert!(e[0].contains("se importa de `./tipos.ts`"), "{e:?}");
}

#[test]
fn lo_que_no_se_borra() {
    for (fuente, msg) in [
        ("enum Modo { A, B }", "enum Modo"),
        ("namespace N { export const x = 1; }", "namespace N"),
        (
            "class C { constructor(private x: string) {} }",
            "propiedad de parámetro",
        ),
        ("import fs = require(\"fs\");", "import x ="),
        ("function f() { enum Dentro { A } }", "enum Dentro"),
    ] {
        let d = derivar(
            &format!("{fuente}\nexport default function repeat(): string {{ return \"\"; }}"),
            RUTA,
        );
        assert!(
            d.version.iter().any(|f| f.mensaje.contains(msg)),
            "{fuente}: {:?}",
            d.version
        );
    }
    // Lo que sí se borra.
    for fuente in [
        "declare enum E { A }",
        "namespace T { export interface X { a: string } }",
        "declare namespace D { const x: number; }",
        "declare global { interface Window { x: string } }",
        "const enum_ = 1;",
        "let x = <string>(\"a\");",
        "class C { constructor(x: string) {} }",
    ] {
        let d = derivar(
            &format!("{fuente}\nexport default function repeat(): string {{ return \"\"; }}"),
            RUTA,
        );
        assert!(d.version.is_empty(), "{fuente}: {:?}", d.version);
    }
}

#[test]
fn la_sintaxis_rota_se_dice_y_no_se_detiene() {
    let d = derivar("export default function repeat(a: string { }", RUTA);
    assert!(!d.sintaxis.is_empty());
}

#[test]
fn un_fichero_hostil_no_tumba_el_proceso() {
    for hostil in [
        format!(
            "type X = {}string{};",
            "(".repeat(50_000),
            ")".repeat(50_000)
        ),
        format!("const x = {}1{};", "[".repeat(100_000), "]".repeat(100_000)),
        format!("const x = {}1;", "-".repeat(100_000)),
        format!("const x = 1{};", "+1".repeat(100_000)),
    ] {
        let _ = derivar(
            &format!("{hostil}\nexport default function repeat(): string {{ return \"\"; }}"),
            RUTA,
        );
    }
}
