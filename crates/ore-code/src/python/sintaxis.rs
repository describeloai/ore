//! El **único** fichero que toca el parser de Ruff.
//!
//! `ruff_python_parser` se publica como crate interno (0.0.x) y su API cambia
//! entre versiones. Aquí su árbol se convierte en una sintaxis propia, pequeña
//! y estable —solo lo que una firma puede ser—, y la derivación (§4) se escribe
//! sobre ella. Renovar Ruff toca este fichero y ningún otro.

use crate::firma::{Fallo, Rango};
use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{self as ast, PythonVersion, Stmt};
use ruff_python_parser::typing::parse_type_annotation;
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
use ruff_text_size::{Ranged, TextRange};

/// Una expresión, reducida a lo que un decorador o una anotación pueden ser.
#[derive(Debug, Clone)]
pub enum Expr {
    Nombre(String, Rango),
    Atributo(Box<Expr>, String, Rango),
    /// `X[a]` o `X[a, b]`.
    Indice(Box<Expr>, Vec<Expr>, Rango),
    /// `a | b`.
    O(Box<Expr>, Box<Expr>, Rango),
    Nada(Rango),
    Cadena(String, Rango),
    /// Un entero literal (v1alpha20: `Precision(12, 2)`, `Money["EUR", 2]`).
    Entero(i64, Rango),
    Lista(Vec<Expr>, Rango),
    Llamada {
        funcion: Box<Expr>,
        posicionales: usize,
        /// Los posicionales, en orden (v1alpha20: los de `Precision(p, s)`).
        argumentos: Vec<Expr>,
        /// `None` como nombre es un `**algo`.
        nombrados: Vec<(Option<String>, Expr)>,
        rango: Rango,
    },
    /// Una anotación entre comillas, ya analizada, y su texto: dentro de
    /// `Money["EUR", 2]` o `Media["a.b.c"]` la cadena es un literal y no una
    /// referencia adelantada (v1alpha20).
    Comillas(Box<Expr>, String, Rango),
    /// Entre comillas, y lo de dentro no es Python; con su texto.
    ComillasRotas(String, Rango),
    Otra(Rango),
}

impl Expr {
    pub fn rango(&self) -> Rango {
        match self {
            Expr::Nombre(_, r)
            | Expr::Atributo(_, _, r)
            | Expr::Indice(_, _, r)
            | Expr::O(_, _, r)
            | Expr::Nada(r)
            | Expr::Cadena(_, r)
            | Expr::Entero(_, r)
            | Expr::Lista(_, r)
            | Expr::Llamada { rango: r, .. }
            | Expr::Comillas(_, _, r)
            | Expr::ComillasRotas(_, r)
            | Expr::Otra(r) => *r,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub nombre: String,
    pub rango: Rango,
    pub anotacion: Option<Expr>,
    pub defecto: bool,
}

#[derive(Debug, Clone)]
pub struct Def {
    /// La sentencia del nivel superior: hasta dónde llegan las ligaduras.
    pub indice: usize,
    pub nombre: String,
    pub rango: Rango,
    pub asincrona: bool,
    pub decoradores: Vec<Expr>,
    pub posicionales: Vec<Param>,
    pub solo_nombre: Vec<Param>,
    /// `*args` y `**kwargs`, como se escribieron.
    pub variadicos: Vec<(String, Rango)>,
    pub retorno: Option<Expr>,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CampoDeClase {
    pub nombre: String,
    pub anotacion: Expr,
    pub valor: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Clase {
    pub indice: usize,
    pub nombre: String,
    pub rango: Rango,
    pub decoradores: Vec<Expr>,
    pub bases: Vec<Expr>,
    /// `class A(metaclass=…)`.
    pub con_palabras: bool,
    pub campos: Vec<CampoDeClase>,
}

/// Un nombre que el módulo liga: `import x`, `from m import s as n`, un
/// `def`, una clase, una asignación. `a` es el nombre cualificado.
#[derive(Debug, Clone)]
pub struct Liga {
    pub indice: usize,
    pub nombre: String,
    pub a: String,
    /// La cadena literal a la que la liga una asignación del nivel superior
    /// —`PEDIDOS = "ventas.pedidos"`, sin `if` ni `try` alrededor—: lo que un
    /// argumento de `@transform` puede nombrar (OOS v1alpha25 `01` §5.1).
    pub valor: Option<String>,
}

/// Un `def` decorado que no está en el nivel superior.
#[derive(Debug, Clone)]
pub struct Anidada {
    pub indice: usize,
    pub nombre: String,
    pub rango: Rango,
    pub decoradores: Vec<Expr>,
}

#[derive(Debug, Default)]
pub struct Modulo {
    pub futuro_anotaciones: bool,
    pub ligas: Vec<Liga>,
    pub defs: Vec<Def>,
    pub clases: Vec<Clase>,
    pub anidadas: Vec<Anidada>,
    pub sintaxis: Vec<Fallo>,
    pub version: Vec<Fallo>,
}

fn rango(r: TextRange) -> Rango {
    Rango {
        inicio: r.start().to_u32(),
        fin: r.end().to_u32(),
    }
}

pub fn leer(fuente: &str) -> Modulo {
    let opciones = ParseOptions::from(Mode::Module).with_target_version(PythonVersion {
        major: super::PYTHON_DEL_PUESTO.0,
        minor: super::PYTHON_DEL_PUESTO.1,
    });
    let analizado = parse_unchecked(fuente, opciones);
    let mut m = Modulo {
        sintaxis: analizado
            .errors()
            .iter()
            .map(|e| Fallo::new(rango(e.location), e.error.to_string()))
            .collect(),
        version: analizado
            .unsupported_syntax_errors()
            .iter()
            .map(|e| {
                Fallo::new(rango(e.range), e.to_string())
                    .ayuda("el puesto corre Python 3.12: escríbelo con la sintaxis de 3.12")
            })
            .collect(),
        ..Modulo::default()
    };
    limites_de_cpython(analizado.tokens(), &mut m.sintaxis);
    let ast::Mod::Module(modulo) = analizado.into_syntax() else {
        return m;
    };
    let l = Lector { fuente };
    for (i, s) in modulo.body.iter().enumerate() {
        match s {
            Stmt::ImportFrom(x)
                if x.module
                    .as_ref()
                    .is_some_and(|n| n.as_str() == "__future__")
                    && x.names.iter().any(|a| a.name.as_str() == "annotations") =>
            {
                m.futuro_anotaciones = true;
                l.ligas(std::slice::from_ref(s), i, true, &mut m);
            }
            Stmt::FunctionDef(f) => {
                m.ligas.push(Liga {
                    indice: i,
                    nombre: f.name.as_str().into(),
                    a: local(f.name.as_str()),
                    valor: None,
                });
                m.defs.push(l.def(f, i));
                l.anidadas(&f.body, i, &mut m);
            }
            Stmt::ClassDef(c) => {
                m.ligas.push(Liga {
                    indice: i,
                    nombre: c.name.as_str().into(),
                    a: local(c.name.as_str()),
                    valor: None,
                });
                m.clases.push(l.clase(c, i));
                l.anidadas(&c.body, i, &mut m);
            }
            _ => l.ligas(std::slice::from_ref(s), i, true, &mut m),
        }
    }
    m
}

/// Hasta dónde se baja en una expresión. Una firma real no pasa de cuatro o
/// cinco (`Optional[list[Decimal]]`); CPython rechaza más de 200 paréntesis.
const PROFUNDIDAD: usize = 64;

/// Los dos límites del tokenizador de CPython que Ruff no pone: más de 200
/// paréntesis abiertos a la vez (`MAXLEVEL`) y más de 100 niveles de sangría
/// (`MAXINDENT`). Lo que CPython no analiza no es Python para el puesto, y así
/// la prueba diferencial coincide también en eso.
fn limites_de_cpython(tokens: &ast::token::Tokens, out: &mut Vec<Fallo>) {
    let (mut parentesis, mut sangria) = (0usize, 0usize);
    for t in tokens {
        match t.kind() {
            TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => {
                parentesis += 1;
                if parentesis == 201 {
                    out.push(
                        Fallo::new(rango(t.range()), "too many nested parentheses")
                            .ayuda("CPython no abre más de 200 paréntesis a la vez"),
                    );
                }
            }
            TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => {
                parentesis = parentesis.saturating_sub(1)
            }
            TokenKind::Indent => {
                sangria += 1;
                if sangria == 101 {
                    out.push(
                        Fallo::new(rango(t.range()), "too many levels of indentation")
                            .ayuda("CPython no pasa de 100 niveles de sangría"),
                    );
                }
            }
            TokenKind::Dedent => sangria = sangria.saturating_sub(1),
            _ => {}
        }
    }
}

pub fn local(n: &str) -> String {
    format!("<local>.{n}")
}

struct Lector<'a> {
    fuente: &'a str,
}

impl Lector<'_> {
    /// Las ligaduras de unas sentencias del nivel superior, entrando en `if`
    /// y `try` (`if TYPE_CHECKING:`, `try: import … except ImportError:`).
    /// `superior`: las sentencias son las del módulo, sin nada alrededor; solo
    /// ahí una asignación a una cadena literal guarda su [`Liga::valor`].
    fn ligas(&self, sentencias: &[Stmt], i: usize, superior: bool, m: &mut Modulo) {
        for s in sentencias {
            match s {
                Stmt::Import(x) => {
                    for a in &x.names {
                        let nombre = a.name.as_str();
                        match &a.asname {
                            Some(n) => m.ligas.push(Liga {
                                indice: i,
                                nombre: n.as_str().into(),
                                a: nombre.into(),
                                valor: None,
                            }),
                            None => {
                                let primero = nombre.split('.').next().unwrap_or(nombre);
                                m.ligas.push(Liga {
                                    indice: i,
                                    nombre: primero.into(),
                                    a: primero.into(),
                                    valor: None,
                                });
                            }
                        }
                    }
                }
                Stmt::ImportFrom(x) => {
                    let base = match &x.module {
                        Some(n) if x.level == 0 => n.as_str().to_string(),
                        _ => "<relativo>".to_string(),
                    };
                    for a in &x.names {
                        if a.name.as_str() == "*" {
                            continue;
                        }
                        let nombre = a.asname.as_ref().unwrap_or(&a.name).as_str();
                        m.ligas.push(Liga {
                            indice: i,
                            nombre: nombre.into(),
                            a: format!("{base}.{}", a.name.as_str()),
                            valor: None,
                        });
                    }
                }
                Stmt::FunctionDef(f) => {
                    m.ligas.push(Liga {
                        indice: i,
                        nombre: f.name.as_str().into(),
                        a: local(f.name.as_str()),
                        valor: None,
                    });
                    self.anidada(f, i, m);
                }
                Stmt::ClassDef(c) => {
                    m.ligas.push(Liga {
                        indice: i,
                        nombre: c.name.as_str().into(),
                        a: local(c.name.as_str()),
                        valor: None,
                    });
                    self.anidadas(&c.body, i, m);
                }
                Stmt::Assign(x) => {
                    // Solo `N = "…"`: un destino, un nombre, una cadena. `A = B =
                    // "…"` o `A, B = …` ligan, pero no a algo que se lea así.
                    let valor = match (x.targets.as_slice(), x.value.as_ref()) {
                        ([ast::Expr::Name(_)], ast::Expr::StringLiteral(v)) if superior => {
                            Some(v.value.to_str().to_string())
                        }
                        _ => None,
                    };
                    for t in &x.targets {
                        nombres(t, &mut |n| {
                            m.ligas.push(Liga {
                                indice: i,
                                nombre: n.into(),
                                a: local(n),
                                valor: valor.clone(),
                            })
                        });
                    }
                }
                Stmt::AnnAssign(x) if x.value.is_some() => {
                    let valor = match (x.target.as_ref(), x.value.as_deref()) {
                        (ast::Expr::Name(_), Some(ast::Expr::StringLiteral(v))) if superior => {
                            Some(v.value.to_str().to_string())
                        }
                        _ => None,
                    };
                    nombres(&x.target, &mut |n| {
                        m.ligas.push(Liga {
                            indice: i,
                            nombre: n.into(),
                            a: local(n),
                            valor: valor.clone(),
                        })
                    });
                }
                // Religan sin ser una asignación: `N += "…"`, `for N in …`. Para
                // un `@transform`, un nombre ligado dos veces no se lee.
                Stmt::AugAssign(x) => {
                    nombres(&x.target, &mut |n| {
                        m.ligas.push(Liga {
                            indice: i,
                            nombre: n.into(),
                            a: local(n),
                            valor: None,
                        })
                    });
                }
                Stmt::For(x) => {
                    nombres(&x.target, &mut |n| {
                        m.ligas.push(Liga {
                            indice: i,
                            nombre: n.into(),
                            a: local(n),
                            valor: None,
                        })
                    });
                }
                Stmt::TypeAlias(x) => {
                    nombres(&x.name, &mut |n| {
                        m.ligas.push(Liga {
                            indice: i,
                            nombre: n.into(),
                            a: local(n),
                            valor: None,
                        })
                    });
                }
                Stmt::If(x) => {
                    self.ligas(&x.body, i, false, m);
                    for c in &x.elif_else_clauses {
                        self.ligas(&c.body, i, false, m);
                    }
                }
                Stmt::Try(x) => {
                    self.ligas(&x.body, i, false, m);
                    for h in &x.handlers {
                        let ast::ExceptHandler::ExceptHandler(h) = h;
                        self.ligas(&h.body, i, false, m);
                    }
                    self.ligas(&x.orelse, i, false, m);
                    self.ligas(&x.finalbody, i, false, m);
                }
                _ => {}
            }
        }
    }

    fn anidada(&self, f: &ast::StmtFunctionDef, i: usize, m: &mut Modulo) {
        if !f.decorator_list.is_empty() {
            m.anidadas.push(Anidada {
                indice: i,
                nombre: f.name.as_str().into(),
                rango: rango(f.name.range),
                decoradores: f
                    .decorator_list
                    .iter()
                    .map(|d| self.expr(&d.expression))
                    .collect(),
            });
        }
        self.anidadas(&f.body, i, m);
    }

    /// Los `def` decorados dentro de una clase o de otro `def`, a cualquier
    /// profundidad.
    fn anidadas(&self, cuerpo: &[Stmt], i: usize, m: &mut Modulo) {
        for s in cuerpo {
            match s {
                Stmt::FunctionDef(f) => self.anidada(f, i, m),
                Stmt::ClassDef(c) => self.anidadas(&c.body, i, m),
                _ => {}
            }
        }
    }

    fn def(&self, f: &ast::StmtFunctionDef, i: usize) -> Def {
        let p = &f.parameters;
        let param = |x: &ast::ParameterWithDefault| Param {
            nombre: x.parameter.name.as_str().into(),
            rango: rango(x.parameter.range),
            anotacion: x.parameter.annotation.as_deref().map(|a| self.anotacion(a)),
            defecto: x.default.is_some(),
        };
        Def {
            indice: i,
            nombre: f.name.as_str().into(),
            rango: rango(f.name.range),
            asincrona: f.is_async,
            decoradores: f
                .decorator_list
                .iter()
                .map(|d| self.expr(&d.expression))
                .collect(),
            posicionales: p
                .posonlyargs
                .iter()
                .chain(p.args.iter())
                .map(param)
                .collect(),
            solo_nombre: p.kwonlyargs.iter().map(param).collect(),
            variadicos: p
                .vararg
                .iter()
                .map(|v| (format!("*{}", v.name.as_str()), rango(v.range)))
                .chain(
                    p.kwarg
                        .iter()
                        .map(|v| (format!("**{}", v.name.as_str()), rango(v.range))),
                )
                .collect(),
            retorno: f.returns.as_deref().map(|r| self.anotacion(r)),
            docstring: docstring(&f.body),
        }
    }

    fn clase(&self, c: &ast::StmtClassDef, i: usize) -> Clase {
        let (bases, con_palabras) = match &c.arguments {
            Some(a) => (
                a.args.iter().map(|b| self.expr(b)).collect(),
                !a.keywords.is_empty(),
            ),
            None => (Vec::new(), false),
        };
        let campos = c
            .body
            .iter()
            .filter_map(|s| match s {
                Stmt::AnnAssign(x) => match x.target.as_ref() {
                    ast::Expr::Name(n) => Some(CampoDeClase {
                        nombre: n.id.as_str().into(),
                        anotacion: self.anotacion(&x.annotation),
                        valor: x.value.as_deref().map(|v| self.expr(v)),
                    }),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        Clase {
            indice: i,
            nombre: c.name.as_str().into(),
            rango: rango(c.name.range),
            decoradores: c
                .decorator_list
                .iter()
                .map(|d| self.expr(&d.expression))
                .collect(),
            bases,
            con_palabras,
            campos,
        }
    }

    /// Una anotación: como una expresión, pero una cadena es código
    /// (`"Decimal"`, `list["Linea"]`) y se analiza con la gramática de tipos.
    fn anotacion(&self, e: &ast::Expr) -> Expr {
        self.convertir(e, true, 0)
    }

    fn expr(&self, e: &ast::Expr) -> Expr {
        self.convertir(e, false, 0)
    }

    /// Recursiva, y por eso con fondo: una anotación de mil niveles no es una
    /// firma, y no puede ser la forma de desbordar la pila de `ore-serve`.
    fn convertir(&self, e: &ast::Expr, es_anotacion: bool, nivel: usize) -> Expr {
        let r = rango(e.range());
        if nivel > PROFUNDIDAD {
            return Expr::Otra(r);
        }
        let n = nivel + 1;
        let sub = |x: &ast::Expr| Box::new(self.convertir(x, es_anotacion, n));
        match e {
            ast::Expr::Name(n) => Expr::Nombre(n.id.as_str().into(), r),
            ast::Expr::Attribute(a) => Expr::Atributo(sub(&a.value), a.attr.as_str().into(), r),
            ast::Expr::Subscript(s) => {
                let args = match s.slice.as_ref() {
                    ast::Expr::Tuple(t) => t
                        .elts
                        .iter()
                        .map(|x| self.convertir(x, es_anotacion, n))
                        .collect(),
                    x => vec![self.convertir(x, es_anotacion, n)],
                };
                Expr::Indice(sub(&s.value), args, r)
            }
            ast::Expr::BinOp(b) if b.op == ast::Operator::BitOr => {
                Expr::O(sub(&b.left), sub(&b.right), r)
            }
            ast::Expr::NoneLiteral(_) => Expr::Nada(r),
            ast::Expr::StringLiteral(s) if es_anotacion => {
                let texto: String = s.value.to_str().into();
                match parse_type_annotation(s, self.fuente) {
                    Ok(p) => {
                        Expr::Comillas(Box::new(self.convertir(p.expression(), true, n)), texto, r)
                    }
                    Err(_) => Expr::ComillasRotas(texto, r),
                }
            }
            ast::Expr::StringLiteral(s) => Expr::Cadena(s.value.to_str().into(), r),
            ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
                value: ast::Number::Int(i),
                ..
            }) => match i.as_i64() {
                Some(v) => Expr::Entero(v, r),
                None => Expr::Otra(r),
            },
            ast::Expr::List(l) => Expr::Lista(
                l.elts.iter().map(|x| self.convertir(x, false, n)).collect(),
                r,
            ),
            ast::Expr::Call(c) => Expr::Llamada {
                funcion: Box::new(self.convertir(&c.func, false, n)),
                posicionales: c.arguments.args.len(),
                argumentos: c
                    .arguments
                    .args
                    .iter()
                    .map(|x| self.convertir(x, false, n))
                    .collect(),
                nombrados: c
                    .arguments
                    .keywords
                    .iter()
                    .map(|k| {
                        (
                            k.arg.as_ref().map(|a| a.as_str().to_string()),
                            self.convertir(&k.value, false, n),
                        )
                    })
                    .collect(),
                rango: r,
            },
            _ => Expr::Otra(r),
        }
    }
}

fn nombres(t: &ast::Expr, f: &mut impl FnMut(&str)) {
    match t {
        ast::Expr::Name(n) => f(n.id.as_str()),
        ast::Expr::Tuple(x) => x.elts.iter().for_each(|e| nombres(e, f)),
        ast::Expr::List(x) => x.elts.iter().for_each(|e| nombres(e, f)),
        ast::Expr::Starred(x) => nombres(&x.value, f),
        _ => {}
    }
}

fn docstring(cuerpo: &[Stmt]) -> Option<String> {
    match cuerpo.first() {
        Some(Stmt::Expr(x)) => match x.value.as_ref() {
            ast::Expr::StringLiteral(s) => Some(s.value.to_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}
