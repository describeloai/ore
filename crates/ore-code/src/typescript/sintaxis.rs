//! El **único** fichero que toca el parser de oxc.
//!
//! `oxc_parser` publica una 0.x cada semana y su árbol cambia. Aquí se
//! convierte en una sintaxis propia, pequeña y estable —solo lo que la firma de
//! una función de TypeScript puede ser (OOS v1alpha23 `01`)—, y la derivación
//! se escribe sobre ella. Renovar oxc toca este fichero y ningún otro.
//!
//! El árbol de oxc vive en un *arena*: se suelta entero, sin recorrerlo, así
//! que un fichero hostil no tumba el proceso al soltarlo (lo que con Ruff
//! obliga a soltarlo dentro de `stacker`). Lo que sí recorre la pila es
//! analizarlo y visitarlo, y eso corre en la pila de [`super::derivar`].

use crate::firma::{Fallo, Rango};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    self, BindingPattern, Declaration, ExportDefaultDeclarationKind, Expression, FormalParameter,
    ImportDeclarationSpecifier, ModuleExportName, ObjectPropertyKind, PropertyKey, Statement,
    TSLiteral, TSSignature, TSType, TSTypeName, TSTypeOperatorOperator, VariableDeclarationKind,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

/// Un tipo, reducido a lo que una firma puede ser.
#[derive(Debug, Clone)]
pub enum T {
    /// Una palabra clave: `string`, `number`, `boolean`, `bigint`, `null`,
    /// `undefined`, `void`, `never`, `any`, `unknown`, `object`, `symbol`.
    Palabra(&'static str, Rango),
    /// `A`, `ore.Money<"EUR", 2>`: el nombre, en partes, y sus argumentos.
    Ref {
        nombre: Vec<String>,
        args: Vec<T>,
        rango: Rango,
    },
    /// `T[]`; `readonly T[]` es lo mismo.
    Lista(Box<T>, Rango),
    /// `a | b | …`.
    Union(Vec<T>, Rango),
    /// Una cadena literal (`"EUR"`) en un tipo.
    Cadena(String, Rango),
    /// Un número literal (`2`), con lo que se escribió.
    Numero(String, Rango),
    /// `{ a: T; b?: U }`.
    Objeto(Vec<Miembro>, Rango),
    /// Cualquier otra forma de tipo, con lo que es, para decirlo.
    Otro(&'static str, Rango),
}

impl T {
    pub fn rango(&self) -> Rango {
        match self {
            T::Palabra(_, r)
            | T::Ref { rango: r, .. }
            | T::Lista(_, r)
            | T::Union(_, r)
            | T::Cadena(_, r)
            | T::Numero(_, r)
            | T::Objeto(_, r)
            | T::Otro(_, r) => *r,
        }
    }
}

/// Una propiedad de un `interface` o de un literal de objeto, o lo que no lo es.
#[derive(Debug, Clone)]
pub struct Miembro {
    /// `None`: no es una propiedad con nombre (un método, una firma de índice,
    /// una clave calculada…); `problema` dice qué es.
    pub nombre: Option<String>,
    pub rango: Rango,
    pub tipo: Option<T>,
    pub opcional: bool,
    pub problema: Option<&'static str>,
}

/// Un parámetro de la función.
#[derive(Debug, Clone)]
pub struct Param {
    /// `None` si es una desestructuración (`{ a, b }: X`).
    pub nombre: Option<String>,
    pub rango: Rango,
    pub tipo: Option<T>,
    /// `x?: T`.
    pub opcional: bool,
    /// `x: T = valor`.
    pub defecto: bool,
}

/// La función que el fichero exporta por defecto.
#[derive(Debug, Clone)]
pub struct Funcion {
    /// `None`: `export default function (…)`, sin nombre.
    pub nombre: Option<String>,
    /// El nombre, o la palabra `function` si no lo tiene.
    pub rango: Rango,
    pub asincrona: bool,
    pub generadora: bool,
    pub genericos: Option<Rango>,
    pub this: Option<Rango>,
    pub params: Vec<Param>,
    /// `...resto`.
    pub resto: Option<Rango>,
    pub retorno: Option<T>,
    /// El comentario JSDoc justo encima de `export default`, sin `/**` ni `*/`.
    pub jsdoc: Option<String>,
}

/// Lo que el fichero exporta por defecto.
#[derive(Debug, Clone)]
pub enum Defecto {
    Funcion(Funcion),
    /// Otra cosa —una flecha, una clase, un valor, `export { f as default }`—,
    /// dicha con palabras, y dónde.
    Otra(&'static str, Rango),
}

/// Un valor de `config`: lo que se puede leer sin ejecutar, o dónde no.
#[derive(Debug, Clone)]
pub enum Valor {
    Cadena(String, Rango),
    Lista(Vec<Valor>, Rango),
    /// Las claves, en orden; `None` como clave es un `...spread` o una clave
    /// calculada.
    Objeto(Vec<(Option<String>, Valor)>, Rango),
    Otro(&'static str, Rango),
}

impl Valor {
    pub fn rango(&self) -> Rango {
        match self {
            Valor::Cadena(_, r) | Valor::Lista(_, r) | Valor::Objeto(_, r) | Valor::Otro(_, r) => {
                *r
            }
        }
    }
}

/// `export const config = …`, o por qué no lo es.
#[derive(Debug, Clone)]
pub struct Config {
    pub rango: Rango,
    /// Lo que está mal en la declaración misma (`let`, sin exportar…).
    pub problema: Option<&'static str>,
    pub valor: Option<Valor>,
}

/// Un nombre que el fichero importa: `import { Money as M } from "ore"` liga
/// `M` a `("ore", Some("Money"))`; `import * as o from "ore"`, `o` a
/// `("ore", None)`.
#[derive(Debug, Clone)]
pub struct Importado {
    pub local: String,
    pub modulo: String,
    /// `None`: el espacio de nombres entero; `Some("default")`: el defecto.
    pub nombre: Option<String>,
}

/// Un `interface` o un `type` del nivel superior.
#[derive(Debug, Clone)]
pub struct DeclTipo {
    pub nombre: String,
    pub rango: Rango,
    pub genericos: bool,
    pub cuerpo: Cuerpo,
}

#[derive(Debug, Clone)]
pub enum Cuerpo {
    Interface {
        /// Lo que extiende: el nombre, en partes, si lleva argumentos, y dónde.
        extends: Vec<(Vec<String>, bool, Rango)>,
        miembros: Vec<Miembro>,
    },
    Alias(T),
}

/// Un fichero `.ts`, reducido a lo que la firma necesita.
#[derive(Debug, Clone, Default)]
pub struct Modulo {
    /// Lo que no es TypeScript.
    pub sintaxis: Vec<Fallo>,
    /// TypeScript que no se borra, se compila: el runtime no lo ejecuta
    /// (v1alpha23 `01` §8).
    pub no_borrable: Vec<Fallo>,
    pub importados: Vec<Importado>,
    pub tipos: Vec<DeclTipo>,
    /// Todo nombre del nivel superior que no es un `interface` ni un `type`
    /// (una clase, un `enum`, una función, una variable): tapa un global o un
    /// importado con su nombre.
    pub otros_nombres: Vec<String>,
    pub defecto: Option<Defecto>,
    pub config: Option<Config>,
}

fn rango(s: Span) -> Rango {
    Rango {
        inicio: s.start,
        fin: s.end,
    }
}

pub fn leer(fuente: &str) -> Modulo {
    let alloc = Allocator::default();
    let r = Parser::new(&alloc, fuente, SourceType::ts()).parse();
    let mut m = Modulo::default();
    for e in r.diagnostics.errors() {
        let rg = e
            .labels
            .first()
            .map(|l| Rango {
                inicio: l.offset(),
                fin: l.offset() + l.len(),
            })
            .unwrap_or_default();
        let mut f = Fallo::new(rg, e.message.to_string());
        if let Some(h) = &e.help {
            f = f.ayuda(h.to_string());
        }
        m.sintaxis.push(f);
    }
    let p = &r.program;
    let mut borrable = Borrable {
        fallos: Vec::new(),
        en_declare: 0,
    };
    borrable.visit_program(p);
    m.no_borrable = borrable.fallos;

    let mut exportado_por_nombre: Vec<(String, Rango)> = Vec::new();
    for st in &p.body {
        match st {
            Statement::ImportDeclaration(i) => importacion(i, &mut m.importados),
            Statement::ExportDefaultDeclaration(d) => {
                m.defecto = Some(defecto(d, fuente, &p.comments));
            }
            Statement::ExportDeclaration(d) => {
                declaracion(&d.declaration, true, &mut m);
            }
            Statement::ExportNamedDeclaration(d) => {
                for s in &d.specifiers {
                    let local = nombre_exportado(&s.local);
                    let fuera = nombre_exportado(&s.exported);
                    if fuera == "default" {
                        m.defecto = Some(Defecto::Otra(
                            "`export { … as default }`, que no es una declaración",
                            rango(s.span),
                        ));
                    }
                    exportado_por_nombre.push((local, rango(s.span)));
                }
            }
            st => {
                if let Some(d) = st.as_declaration() {
                    declaracion(d, false, &mut m);
                }
            }
        }
    }
    // `const config = {…}; export { config }`: se exporta, pero no en su
    // declaración, y así no se lee como la configuración.
    if let Some((_, r)) = exportado_por_nombre.iter().find(|(n, _)| n == "config")
        && m.config.as_ref().is_none_or(|c| c.problema.is_some())
    {
        m.config = Some(Config {
            rango: *r,
            problema: Some("se exporta aparte, no en su declaración"),
            valor: None,
        });
    }
    m
}

fn nombre_exportado(n: &ModuleExportName) -> String {
    match n {
        ModuleExportName::IdentifierName(i) => i.name.as_str().to_string(),
        ModuleExportName::IdentifierReference(i) => i.name.as_str().to_string(),
        ModuleExportName::StringLiteral(s) => s.value.as_str().to_string(),
    }
}

fn importacion(i: &ast::ImportDeclaration, fuera: &mut Vec<Importado>) {
    let modulo = i.source.value.as_str().to_string();
    for s in i.specifiers.iter().flatten() {
        let (local, nombre) = match s {
            ImportDeclarationSpecifier::ImportSpecifier(x) => {
                (x.local.name.as_str(), Some(nombre_exportado(&x.imported)))
            }
            ImportDeclarationSpecifier::ImportDefaultSpecifier(x) => {
                (x.local.name.as_str(), Some("default".to_string()))
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(x) => {
                (x.local.name.as_str(), None)
            }
        };
        fuera.push(Importado {
            local: local.to_string(),
            modulo: modulo.clone(),
            nombre,
        });
    }
}

fn declaracion(d: &Declaration, exportada: bool, m: &mut Modulo) {
    match d {
        Declaration::TSInterfaceDeclaration(i) => m.tipos.push(DeclTipo {
            nombre: i.id.name.as_str().to_string(),
            rango: rango(i.id.span),
            genericos: i.type_parameters.is_some(),
            cuerpo: Cuerpo::Interface {
                extends: i
                    .extends
                    .iter()
                    .map(|h| {
                        (
                            nombre_de_tipo(&h.type_name),
                            h.type_arguments.is_some(),
                            rango(h.span),
                        )
                    })
                    .collect(),
                miembros: i.body.body.iter().map(miembro).collect(),
            },
        }),
        Declaration::TSTypeAliasDeclaration(a) => m.tipos.push(DeclTipo {
            nombre: a.id.name.as_str().to_string(),
            rango: rango(a.id.span),
            genericos: a.type_parameters.is_some(),
            cuerpo: Cuerpo::Alias(tipo(&a.type_annotation)),
        }),
        Declaration::VariableDeclaration(v) => {
            for x in &v.declarations {
                let BindingPattern::BindingIdentifier(id) = &x.id else {
                    continue;
                };
                let n = id.name.as_str();
                if n == "config" {
                    let problema = if !exportada {
                        Some("no se exporta en su declaración")
                    } else if v.kind != VariableDeclarationKind::Const {
                        Some("no es `const`")
                    } else if v.declarations.len() > 1 {
                        Some("comparte la declaración con otras variables")
                    } else {
                        None
                    };
                    m.config = Some(Config {
                        rango: rango(x.span),
                        problema,
                        valor: x.init.as_ref().map(valor),
                    });
                }
                m.otros_nombres.push(n.to_string());
            }
        }
        Declaration::FunctionDeclaration(f) => {
            if let Some(id) = &f.id {
                m.otros_nombres.push(id.name.as_str().to_string());
            }
        }
        Declaration::ClassDeclaration(c) => {
            if let Some(id) = &c.id {
                m.otros_nombres.push(id.name.as_str().to_string());
            }
        }
        Declaration::TSEnumDeclaration(e) => m.otros_nombres.push(e.id.name.as_str().to_string()),
        Declaration::TSNamespaceDeclaration(n) => {
            m.otros_nombres.push(n.id.name.as_str().to_string())
        }
        Declaration::TSImportEqualsDeclaration(i) => {
            m.otros_nombres.push(i.id.name.as_str().to_string())
        }
        Declaration::TSExternalModuleDeclaration(_) | Declaration::TSGlobalDeclaration(_) => {}
    }
}

fn defecto(
    d: &ast::ExportDefaultDeclaration,
    fuente: &str,
    comentarios: &[ast::Comment],
) -> Defecto {
    let f = match &d.declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(f) => f,
        ExportDefaultDeclarationKind::ClassDeclaration(c) => {
            return Defecto::Otra("una clase", rango(c.span));
        }
        ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => {
            return Defecto::Otra("un `interface`", rango(i.span));
        }
        otra => {
            let que = match otra.as_expression() {
                Some(Expression::ArrowFunctionExpression(_)) => "una función flecha",
                Some(Expression::FunctionExpression(_)) => "una expresión de función",
                Some(Expression::Identifier(_)) => "un nombre, no la declaración de la función",
                Some(Expression::CallExpression(_)) => "lo que devuelve una llamada",
                _ => "un valor",
            };
            return Defecto::Otra(que, rango(d.span));
        }
    };
    let mut params = Vec::new();
    for p in &f.params.items {
        params.push(parametro(p));
    }
    Defecto::Funcion(Funcion {
        nombre: f.id.as_ref().map(|i| i.name.as_str().to_string()),
        rango: f
            .id
            .as_ref()
            .map(|i| rango(i.span))
            .unwrap_or_else(|| rango(f.span)),
        asincrona: f.r#async,
        generadora: f.generator,
        genericos: f.type_parameters.as_ref().map(|t| rango(t.span)),
        this: f.this_param.as_ref().map(|t| rango(t.span)),
        params,
        resto: f.params.rest.as_ref().map(|r| rango(r.span)),
        retorno: f.return_type.as_ref().map(|t| tipo(&t.type_annotation)),
        jsdoc: jsdoc(d.span, fuente, comentarios),
    })
}

/// El último `/** … */` que acaba justo antes de `antes`, con solo espacio en
/// medio: el de la declaración.
fn jsdoc(antes: Span, fuente: &str, comentarios: &[ast::Comment]) -> Option<String> {
    let c = comentarios
        .iter()
        .rev()
        .find(|c| c.span.end <= antes.start)?;
    let texto = fuente.get(c.span.start as usize..c.span.end as usize)?;
    let entre = fuente.get(c.span.end as usize..antes.start as usize)?;
    if !c.is_block() || !texto.starts_with("/**") || texto == "/**/" || !entre.trim().is_empty() {
        return None;
    }
    Some(texto[3..texto.len() - 2].to_string())
}

fn parametro(p: &FormalParameter) -> Param {
    let nombre = match &p.pattern {
        BindingPattern::BindingIdentifier(i) => Some(i.name.as_str().to_string()),
        _ => None,
    };
    Param {
        nombre,
        rango: rango(p.span),
        tipo: p.type_annotation.as_ref().map(|t| tipo(&t.type_annotation)),
        opcional: p.optional,
        defecto: p.initializer.is_some(),
    }
}

fn nombre_de_tipo(n: &TSTypeName) -> Vec<String> {
    match n {
        TSTypeName::IdentifierReference(i) => vec![i.name.as_str().to_string()],
        TSTypeName::QualifiedName(q) => {
            let mut v = nombre_de_tipo(&q.left);
            v.push(q.right.name.as_str().to_string());
            v
        }
        TSTypeName::ThisExpression(_) => vec!["this".to_string()],
    }
}

fn tipo(t: &TSType) -> T {
    match t {
        TSType::TSStringKeyword(k) => T::Palabra("string", rango(k.span)),
        TSType::TSNumberKeyword(k) => T::Palabra("number", rango(k.span)),
        TSType::TSBooleanKeyword(k) => T::Palabra("boolean", rango(k.span)),
        TSType::TSBigIntKeyword(k) => T::Palabra("bigint", rango(k.span)),
        TSType::TSNullKeyword(k) => T::Palabra("null", rango(k.span)),
        TSType::TSUndefinedKeyword(k) => T::Palabra("undefined", rango(k.span)),
        TSType::TSVoidKeyword(k) => T::Palabra("void", rango(k.span)),
        TSType::TSNeverKeyword(k) => T::Palabra("never", rango(k.span)),
        TSType::TSAnyKeyword(k) => T::Palabra("any", rango(k.span)),
        TSType::TSUnknownKeyword(k) => T::Palabra("unknown", rango(k.span)),
        TSType::TSObjectKeyword(k) => T::Palabra("object", rango(k.span)),
        TSType::TSSymbolKeyword(k) => T::Palabra("symbol", rango(k.span)),
        TSType::TSParenthesizedType(p) => tipo(&p.type_annotation),
        TSType::TSArrayType(a) => T::Lista(Box::new(tipo(&a.element_type)), rango(a.span)),
        TSType::TSTypeOperatorType(o) if o.operator == TSTypeOperatorOperator::Readonly => {
            match tipo(&o.type_annotation) {
                T::Lista(x, _) => T::Lista(x, rango(o.span)),
                // `readonly` solo vale delante de una lista o de una tupla.
                _ => T::Otro("`readonly` de algo que no es una lista", rango(o.span)),
            }
        }
        TSType::TSUnionType(u) => T::Union(u.types.iter().map(tipo).collect(), rango(u.span)),
        TSType::TSTypeReference(r) => T::Ref {
            nombre: nombre_de_tipo(&r.type_name),
            args: r
                .type_arguments
                .as_ref()
                .map(|a| a.params.iter().map(tipo).collect())
                .unwrap_or_default(),
            rango: rango(r.span),
        },
        TSType::TSLiteralType(l) => match &l.literal {
            TSLiteral::StringLiteral(s) => T::Cadena(s.value.as_str().to_string(), rango(l.span)),
            TSLiteral::NumericLiteral(n) => T::Numero(
                n.raw
                    .map(|r| r.as_str().to_string())
                    .unwrap_or_else(|| n.value.to_string()),
                rango(l.span),
            ),
            TSLiteral::BooleanLiteral(_) => T::Otro("un booleano literal", rango(l.span)),
            _ => T::Otro("un literal", rango(l.span)),
        },
        TSType::TSTypeLiteral(o) => {
            T::Objeto(o.members.iter().map(miembro).collect(), rango(o.span))
        }
        TSType::TSTupleType(x) => T::Otro("una tupla", rango(x.span)),
        TSType::TSFunctionType(x) => T::Otro("una función", rango(x.span)),
        TSType::TSIntersectionType(x) => T::Otro("una intersección (`A & B`)", rango(x.span)),
        TSType::TSTypeOperatorType(x) => T::Otro("`keyof` o `unique`", rango(x.span)),
        TSType::TSTemplateLiteralType(x) => T::Otro("un tipo de plantilla", rango(x.span)),
        TSType::TSTypeQuery(x) => T::Otro("`typeof`", rango(x.span)),
        TSType::TSConditionalType(x) => T::Otro("un tipo condicional", rango(x.span)),
        TSType::TSMappedType(x) => T::Otro("un tipo mapeado", rango(x.span)),
        TSType::TSIndexedAccessType(x) => {
            T::Otro("un acceso por índice (`A[\"b\"]`)", rango(x.span))
        }
        TSType::TSImportType(x) => T::Otro("`import(…)`", rango(x.span)),
        otro => T::Otro("un tipo que la firma no lee", rango(otro.span())),
    }
}

fn clave(k: &PropertyKey) -> Option<String> {
    match k {
        PropertyKey::StaticIdentifier(i) => Some(i.name.as_str().to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.as_str().to_string()),
        _ => None,
    }
}

fn miembro(s: &TSSignature) -> Miembro {
    let otro = |problema, sp: Span| Miembro {
        nombre: None,
        rango: rango(sp),
        tipo: None,
        opcional: false,
        problema: Some(problema),
    };
    match s {
        TSSignature::TSPropertySignature(p) => {
            let nombre = if p.computed { None } else { clave(&p.key) };
            Miembro {
                problema: nombre.is_none().then_some("una clave calculada"),
                nombre,
                rango: rango(p.span),
                tipo: p.type_annotation.as_ref().map(|t| tipo(&t.type_annotation)),
                opcional: p.optional,
            }
        }
        TSSignature::TSMethodSignature(x) => otro("un método", x.span),
        TSSignature::TSIndexSignature(x) => otro("una firma de índice (`[k: string]: T`)", x.span),
        TSSignature::TSCallSignatureDeclaration(x) => otro("una firma de llamada", x.span),
        TSSignature::TSConstructSignatureDeclaration(x) => otro("una firma de `new`", x.span),
    }
}

/// Lo que `config` vale, en lo que se puede leer: `as const`, `satisfies` y los
/// paréntesis se borran sin cambiar el valor.
fn valor(e: &Expression) -> Valor {
    match e {
        Expression::TSAsExpression(x) => valor(&x.expression),
        Expression::TSSatisfiesExpression(x) => valor(&x.expression),
        Expression::ParenthesizedExpression(x) => valor(&x.expression),
        Expression::StringLiteral(s) => Valor::Cadena(s.value.as_str().to_string(), rango(s.span)),
        Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
            match t.quasis.first().and_then(|q| q.value.cooked) {
                Some(c) => Valor::Cadena(c.as_str().to_string(), rango(t.span)),
                None => Valor::Otro("una plantilla", rango(t.span)),
            }
        }
        Expression::TemplateLiteral(t) => Valor::Otro("una plantilla con `${…}`", rango(t.span)),
        Expression::ArrayExpression(a) => Valor::Lista(
            a.elements
                .iter()
                .map(|x| match x.as_expression() {
                    Some(e) => valor(e),
                    None => Valor::Otro("un hueco o un `...spread`", rango(a.span)),
                })
                .collect(),
            rango(a.span),
        ),
        Expression::ObjectExpression(o) => Valor::Objeto(
            o.properties
                .iter()
                .map(|p| match p {
                    ObjectPropertyKind::ObjectProperty(p) if !p.computed && !p.method => {
                        (clave(&p.key), valor(&p.value))
                    }
                    ObjectPropertyKind::ObjectProperty(p) => (
                        None,
                        Valor::Otro("una clave calculada o un método", rango(p.span)),
                    ),
                    ObjectPropertyKind::SpreadProperty(s) => {
                        (None, Valor::Otro("un `...spread`", rango(s.span)))
                    }
                })
                .collect(),
            rango(o.span),
        ),
        Expression::Identifier(i) => Valor::Otro("una variable", rango(i.span)),
        Expression::CallExpression(c) => Valor::Otro("una llamada", rango(c.span)),
        otro => Valor::Otro("una expresión", rango(otro.span())),
    }
}

/// Lo que TypeScript no borra sino que compila (`--erasableSyntaxOnly`), en
/// cualquier sitio del fichero: un `enum`, un `namespace` con valores, las
/// propiedades de parámetro de un constructor, `import x = require(…)` y
/// `export =`. Dentro de un `declare` solo hay tipos, y nada se compila.
struct Borrable {
    fallos: Vec<Fallo>,
    en_declare: usize,
}

const ERASABLE: &str = "Node 24 ejecuta TypeScript borrando los tipos (`--erasableSyntaxOnly`); \
                        esto genera código y no se borra";

impl<'a> Visit<'a> for Borrable {
    fn visit_ts_enum_declaration(&mut self, it: &ast::TSEnumDeclaration<'a>) {
        if !it.declare && self.en_declare == 0 {
            self.fallos.push(
                Fallo::new(
                    rango(it.span),
                    format!("`enum {}` no se borra", it.id.name.as_str()),
                )
                .ayuda(format!(
                    "{ERASABLE}. Un objeto `as const` y una unión de literales dicen lo mismo"
                )),
            );
        }
    }

    fn visit_ts_namespace_declaration(&mut self, it: &ast::TSNamespaceDeclaration<'a>) {
        if it.declare || self.en_declare > 0 {
            self.en_declare += 1;
            walk::walk_ts_namespace_declaration(self, it);
            self.en_declare -= 1;
            return;
        }
        if instanciado(&it.body) {
            self.fallos.push(
                Fallo::new(
                    rango(it.span),
                    format!(
                        "`namespace {}` tiene valores y no se borra",
                        it.id.name.as_str()
                    ),
                )
                .ayuda(format!(
                    "{ERASABLE}. Un módulo es un fichero: expórtalo desde otro `.ts`"
                )),
            );
        }
        walk::walk_ts_namespace_declaration(self, it);
    }

    fn visit_ts_global_declaration(&mut self, it: &ast::TSGlobalDeclaration<'a>) {
        self.en_declare += 1;
        walk::walk_ts_global_declaration(self, it);
        self.en_declare -= 1;
    }

    fn visit_ts_external_module_declaration(&mut self, it: &ast::TSExternalModuleDeclaration<'a>) {
        self.en_declare += 1;
        walk::walk_ts_external_module_declaration(self, it);
        self.en_declare -= 1;
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if it.accessibility.is_some() || it.readonly || it.r#override {
            self.fallos.push(
                Fallo::new(
                    rango(it.span),
                    "una propiedad de parámetro (`constructor(private x)`) no se borra",
                )
                .ayuda(format!(
                    "{ERASABLE}. Declara la propiedad en la clase y asígnala en el constructor"
                )),
            );
        }
        walk::walk_formal_parameter(self, it);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &ast::TSImportEqualsDeclaration<'a>) {
        if it.import_kind.is_value() && self.en_declare == 0 {
            self.fallos.push(
                Fallo::new(rango(it.span), "`import x = …` no se borra")
                    .ayuda(format!("{ERASABLE}. `import x from \"…\"`")),
            );
        }
    }

    fn visit_ts_export_assignment(&mut self, it: &ast::TSExportAssignment<'a>) {
        if self.en_declare == 0 {
            self.fallos.push(
                Fallo::new(rango(it.span), "`export =` no se borra")
                    .ayuda(format!("{ERASABLE}. `export default …`")),
            );
        }
    }
}

/// Si un `namespace` tiene algo más que tipos.
fn instanciado(b: &ast::TSNamespaceDeclarationBody) -> bool {
    match b {
        ast::TSNamespaceDeclarationBody::TSNamespaceDeclaration(n) => {
            !n.declare && instanciado(&n.body)
        }
        ast::TSNamespaceDeclarationBody::TSModuleBlock(m) => m.body.iter().any(|s| {
            let d = match s {
                Statement::ExportDeclaration(e) => &e.declaration,
                s => match s.as_declaration() {
                    Some(d) => d,
                    None => return !matches!(s, Statement::EmptyStatement(_)),
                },
            };
            match d {
                Declaration::TSInterfaceDeclaration(_) | Declaration::TSTypeAliasDeclaration(_) => {
                    false
                }
                Declaration::TSNamespaceDeclaration(n) => !n.declare && instanciado(&n.body),
                Declaration::VariableDeclaration(v) => !v.declare,
                Declaration::FunctionDeclaration(f) => !f.declare,
                Declaration::ClassDeclaration(c) => !c.declare,
                Declaration::TSEnumDeclaration(e) => !e.declare,
                _ => true,
            }
        }),
    }
}
