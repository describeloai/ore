//! OOS v1alpha18 01 §4 sobre la sintaxis propia: qué es una función, qué
//! firma tiene, y por qué no se deriva cuando no se deriva.

use super::sintaxis::{Clase, Def, Expr, Modulo, local};
use crate::firma::{self, Campo, Derivacion, Fallo, Firma, Funcion, Rango, Salida, Tipo};

/// Los nombres del lenguaje que pueden aparecer en una anotación.
const BUILTINS: &[&str] = &[
    "int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "object", "type",
];

/// Lo que no es un campo de una `@dataclass` aunque lleve anotación.
const NO_SON_CAMPOS: &[&str] = &[
    "typing.ClassVar",
    "dataclasses.InitVar",
    "dataclasses.KW_ONLY",
];

const TIPOS: &str = "una firma usa `int`, `float`, `str`, `bool`, `date`, `time`, `datetime`, \
                     `bytes`, `Decimal`, `list[T]` y `Optional[T]` (o `T | None`); de `ore.tipos`, \
                     `DateTimeTz`, `Money[\"EUR\", 2]`, `Quantity[\"km\", 1]`, `Media[\"a.b.c\"]` y \
                     `Annotated[Decimal, Precision(p, s)]`; y una `@dataclass` del mismo fichero";

fn escalar(q: &str) -> Option<Tipo> {
    Some(match q {
        "builtins.int" => Tipo::Integer,
        "builtins.float" => Tipo::Float,
        "builtins.str" => Tipo::String,
        "builtins.bool" => Tipo::Boolean,
        "datetime.date" => Tipo::Date,
        "datetime.datetime" => Tipo::DateTime,
        "decimal.Decimal" => Tipo::Decimal,
        // v1alpha20 `01` §2.
        "datetime.time" => Tipo::Time,
        "builtins.bytes" => Tipo::Opaque,
        "ore.tipos.DateTimeTz" => Tipo::DateTimeTz,
        _ => return None,
    })
}

/// De dónde se importa un nombre que se suele olvidar importar.
fn de_donde(n: &str) -> Option<&'static str> {
    Some(match n {
        "date" | "datetime" | "time" => "datetime",
        "DateTimeTz" | "Money" | "Quantity" | "Media" | "Precision" => "ore.tipos",
        "Decimal" => "decimal",
        "Optional" | "List" | "Union" | "Annotated" | "ClassVar" => "typing",
        "dataclass" | "field" => "dataclasses",
        _ => return None,
    })
}

pub fn derivar(m: &Modulo, ruta: &str) -> Derivacion {
    let r = Resolutor {
        m,
        en_curso: Default::default(),
    };
    let mut d = Derivacion {
        sintaxis: m.sintaxis.clone(),
        version: m.version.clone(),
        ..Derivacion::default()
    };
    for def in &m.defs {
        let deco = def
            .decoradores
            .iter()
            .find(|x| r.es_function(x, def.indice));
        d.defs.push(firma::Def {
            nombre: def.nombre.clone(),
            rango: def.rango,
            asincrona: def.asincrona,
            decorada: deco.is_some(),
        });
        let Some(deco) = deco else { continue };
        let resultado = if def.asincrona {
            Err(vec![
                Fallo::new(def.rango, format!("`{}` es un `async def`", def.nombre)).ayuda(
                    "el runtime llama al `def` y espera su valor; una corrutina es otra cosa. \
                 Escríbelo como `def`",
                ),
            ])
        } else {
            r.una(def, deco, ruta)
        };
        d.funciones.push(Funcion {
            nombre: def.nombre.clone(),
            rango: def.rango,
            resultado,
        });
    }
    for a in &m.anidadas {
        if a.decoradores.iter().any(|x| r.es_function(x, a.indice)) {
            d.avisos.push(
                Fallo::new(
                    a.rango,
                    format!(
                        "`{}` lleva `@function` y no está en el nivel superior del módulo",
                        a.nombre
                    ),
                )
                .ayuda(
                    "una función es un `def` del módulo, no un método ni un `def` dentro de otro: \
                         así no se publica",
                ),
            );
        }
    }
    d
}

struct Resolutor<'a> {
    m: &'a Modulo,
    /// Las `@dataclass` que se están derivando como `Struct`, para no entrar
    /// en una que se contiene a sí misma (v1alpha20 `01` §4).
    en_curso: std::cell::RefCell<Vec<String>>,
}

/// El texto de un literal de cadena: en una anotación llega como comillas
/// (v1alpha18 §4.6), y dentro de `Money[…]` o `Media[…]` es un literal.
fn literal(e: &Expr) -> Option<&str> {
    match e {
        Expr::Comillas(_, t, _) | Expr::ComillasRotas(t, _) | Expr::Cadena(t, _) => Some(t),
        _ => None,
    }
}

impl Resolutor<'_> {
    /// El nombre cualificado de una expresión con las ligaduras vigentes
    /// antes de la sentencia `hasta` del nivel superior (todas, con `None`).
    fn cualificar(&self, e: &Expr, hasta: Option<usize>) -> Option<String> {
        let q = match e {
            Expr::Nombre(n, _) => self
                .m
                .ligas
                .iter()
                .rev()
                .find(|l| &l.nombre == n && hasta.is_none_or(|h| l.indice < h))
                .map(|l| l.a.clone())
                .or_else(|| {
                    BUILTINS
                        .contains(&n.as_str())
                        .then(|| format!("builtins.{n}"))
                }),
            Expr::Atributo(v, a, _) => self.cualificar(v, hasta).map(|b| format!("{b}.{a}")),
            _ => None,
        }?;
        Some(match q.strip_prefix("typing_extensions.") {
            Some(resto) => format!("typing.{resto}"),
            None => q,
        })
    }

    fn es_function(&self, d: &Expr, hasta: usize) -> bool {
        let f = match d {
            Expr::Llamada { funcion, .. } => funcion.as_ref(),
            x => x,
        };
        self.cualificar(f, Some(hasta)).as_deref() == Some("ore.function")
    }

    /// Una anotación y hasta dónde se resuelve: entre comillas, o con `from
    /// __future__ import annotations`, al final del módulo; si no, donde se
    /// evalúa (al definir el `def` o la clase).
    fn anotacion<'e>(
        &self,
        e: &'e Expr,
        hasta: Option<usize>,
    ) -> Result<(&'e Expr, Option<usize>), Fallo> {
        match e {
            Expr::Comillas(dentro, _, _) => Ok((dentro.as_ref(), None)),
            Expr::ComillasRotas(_, r) => {
                Err(Fallo::new(*r, "la anotación entre comillas no es Python"))
            }
            x => Ok((
                x,
                if self.m.futuro_anotaciones {
                    None
                } else {
                    hasta
                },
            )),
        }
    }

    fn es_union(&self, e: &Expr, hasta: Option<usize>) -> bool {
        match e {
            Expr::O(..) => true,
            Expr::Indice(base, _, _) => {
                self.cualificar(base, hasta).as_deref() == Some("typing.Union")
            }
            _ => false,
        }
    }

    fn miembros<'e>(
        &self,
        e: &'e Expr,
        hasta: Option<usize>,
        out: &mut Vec<(&'e Expr, Option<usize>)>,
    ) -> Result<(), Fallo> {
        let (e, hasta) = self.anotacion(e, hasta)?;
        match e {
            Expr::O(a, b, _) => {
                self.miembros(a, hasta, out)?;
                self.miembros(b, hasta, out)
            }
            Expr::Indice(_, args, _) if self.es_union(e, hasta) => {
                for a in args {
                    self.miembros(a, hasta, out)?;
                }
                Ok(())
            }
            x => {
                out.push((x, hasta));
                Ok(())
            }
        }
    }

    /// Una anotación → (tipo OOS, si es opcional).
    fn tipo(&self, e: &Expr, hasta: Option<usize>, en_lista: bool) -> Result<(Tipo, bool), Fallo> {
        let (e, hasta) = self.anotacion(e, hasta)?;
        if self.es_union(e, hasta) {
            let mut ms = Vec::new();
            self.miembros(e, hasta, &mut ms)?;
            let nada = ms
                .iter()
                .filter(|(x, _)| matches!(x, Expr::Nada(_)))
                .count();
            let otros: Vec<_> = ms
                .iter()
                .filter(|(x, _)| !matches!(x, Expr::Nada(_)))
                .collect();
            if otros.len() != 1 || nada == 0 {
                return Err(Fallo::new(e.rango(), "una unión que no es `T | None`").ayuda(
                    "un parámetro tiene un tipo; `Optional[T]` o `T | None` dicen que puede faltar. \
                     Dos tipos distintos no son una firma",
                ));
            }
            let (x, h) = otros[0];
            return Ok((self.tipo(x, *h, en_lista)?.0, true));
        }
        if let Expr::Indice(base, args, r) = e {
            let q = self.cualificar(base, hasta);
            match (q.as_deref(), args.as_slice()) {
                (Some("builtins.list" | "typing.List"), [a]) => {
                    if en_lista {
                        return Err(Fallo::new(*r, "una lista de listas")
                            .ayuda("`list<T>` no lleva listas dentro"));
                    }
                    let (t, opcional) = self.tipo(a, hasta, true)?;
                    if opcional {
                        return Err(Fallo::new(a.rango(), "una lista de opcionales")
                            .ayuda("los elementos de una lista no faltan: `list[int]`, no `list[int | None]`"));
                    }
                    return Ok((Tipo::Lista(Box::new(t)), false));
                }
                (Some("typing.Optional"), [a]) => {
                    return Ok((self.tipo(a, hasta, en_lista)?.0, true));
                }
                (Some("typing.Annotated"), [a, metas @ ..]) if !metas.is_empty() => {
                    return self.anotado(a, metas, hasta, en_lista);
                }
                // v1alpha20 `01` §3: la unidad es parte del tipo.
                (Some(q @ ("ore.tipos.Money" | "ore.tipos.Quantity")), args) => {
                    let (ctor, ejemplo) = if q.ends_with("Money") {
                        ("Money", "\"EUR\", 2")
                    } else {
                        ("Quantity", "\"km\", 1")
                    };
                    let [u, p] = args else {
                        return Err(Fallo::new(
                            *r,
                            format!("`{ctor}[…]` lleva la unidad y la precisión"),
                        )
                        .ayuda(format!(
                            "`{ctor}[{ejemplo}]`: la unidad, entre comillas, y los decimales"
                        )));
                    };
                    let unidad = literal(u)
                        .filter(|u| es_unidad(u))
                        .ok_or_else(|| {
                            Fallo::new(
                                u.rango(),
                                format!("la unidad de `{ctor}[…]` es una cadena literal"),
                            )
                            .ayuda(format!("`{ctor}[{ejemplo}]`: {LITERAL}"))
                        })?
                        .to_string();
                    let precision = match p {
                        Expr::Entero(v, _) if (0..=u32::MAX as i64).contains(v) => *v as u32,
                        _ => {
                            return Err(Fallo::new(
                                p.rango(),
                                format!("la precisión de `{ctor}[…]` es un entero literal"),
                            )
                            .ayuda("los decimales, como número: `2`"));
                        }
                    };
                    return Ok((
                        Tipo::Unidad {
                            ctor,
                            unidad,
                            precision,
                        },
                        false,
                    ));
                }
                // v1alpha20 `01` §5.
                (Some("ore.tipos.Media"), args) => {
                    let c = match args {
                        [c] => literal(c).filter(|c| {
                            c.split('.').all(|p| {
                                !p.is_empty()
                                    && p.chars().all(|x| x.is_ascii_alphanumeric() || x == '_')
                            })
                        }),
                        _ => None,
                    };
                    let Some(c) = c else {
                        return Err(Fallo::new(
                            *r,
                            "`Media[…]` nombra una colección, entre comillas",
                        )
                        .ayuda("`Media[\"base.schema.coleccion\"]`"));
                    };
                    return Ok((Tipo::Media(c.to_string()), false));
                }
                _ => return Err(self.sin_traduccion(base, hasta, *r)),
            }
        }
        if let Expr::Nada(r) = e {
            return Err(Fallo::new(*r, "`None` no es un tipo").ayuda(TIPOS));
        }
        // v1alpha20 `01` §4: una `@dataclass` del fichero que no es la vuelta.
        if let Some(n) = self
            .cualificar(e, hasta)
            .as_deref()
            .and_then(|q| q.strip_prefix("<local>."))
            .filter(|n| self.clase(n).is_some_and(|c| self.es_dataclass(c)))
        {
            return self.estructura(n, e.rango()).map(|t| (t, false));
        }
        match self.cualificar(e, hasta).as_deref() {
            Some(q) if let Some(t) = escalar(q) => Ok((t, false)),
            // Sin sus argumentos no dicen qué son.
            Some(q @ ("ore.tipos.Money" | "ore.tipos.Quantity" | "ore.tipos.Media")) => {
                let (n, ejemplo) = match q {
                    "ore.tipos.Money" => ("Money", "Money[\"EUR\", 2]"),
                    "ore.tipos.Quantity" => ("Quantity", "Quantity[\"km\", 1]"),
                    _ => ("Media", "Media[\"base.schema.coleccion\"]"),
                };
                Err(
                    Fallo::new(e.rango(), format!("`{n}` a secas no es un tipo")).ayuda(format!(
                        "`{ejemplo}`: con lo que lo hace un tipo, entre corchetes"
                    )),
                )
            }
            _ => Err(self.sin_traduccion(e, hasta, e.rango())),
        }
    }

    /// `Annotated[T, …]`: `T`, salvo `Annotated[Decimal, Precision(p, s)]`, que
    /// es `Decimal<p, s>` (v1alpha20 `01` §3). Lo demás del `Annotated` no es
    /// firma.
    fn anotado(
        &self,
        a: &Expr,
        metas: &[Expr],
        hasta: Option<usize>,
        en_lista: bool,
    ) -> Result<(Tipo, bool), Fallo> {
        let (t, opcional) = self.tipo(a, hasta, en_lista)?;
        for m in metas {
            let Expr::Llamada {
                funcion,
                argumentos,
                nombrados,
                rango,
                ..
            } = m
            else {
                continue;
            };
            if self.cualificar(funcion, hasta).as_deref() != Some("ore.tipos.Precision") {
                continue;
            }
            if t != Tipo::Decimal {
                return Err(Fallo::new(*rango, "`Precision(p, s)` es de un `Decimal`")
                    .ayuda("`Annotated[Decimal, Precision(12, 2)]`"));
            }
            let ps = match (argumentos.as_slice(), nombrados.is_empty()) {
                ([Expr::Entero(p, _), Expr::Entero(s, _)], true) => Some((*p, *s)),
                _ => None,
            };
            return match ps {
                Some((p, s)) if (1..=38).contains(&p) && (0..=p).contains(&s) => Ok((
                    Tipo::DecimalPs {
                        precision: p as u8,
                        escala: s as u8,
                    },
                    opcional,
                )),
                Some((p, s)) => Err(Fallo::new(
                    *rango,
                    format!("`Precision({p}, {s})` está fuera de rango"),
                )
                .ayuda(
                    "`1 ≤ p ≤ 38` y `0 ≤ s ≤ p`: las cifras en total y las de detrás de la coma",
                )),
                None => Err(
                    Fallo::new(*rango, "`Precision(p, s)` lleva dos enteros literales")
                        .ayuda("`Precision(12, 2)`: doce cifras, dos detrás de la coma"),
                ),
            };
        }
        Ok((t, opcional))
    }

    /// Una `@dataclass` como `Struct<…>`: sus campos, en orden, con su tipo.
    fn estructura(&self, nombre: &str, uso: Rango) -> Result<Tipo, Fallo> {
        if self.en_curso.borrow().iter().any(|n| n == nombre) {
            return Err(Fallo::new(uso, format!("`{nombre}` se contiene a sí misma"))
                .ayuda("una `@dataclass` que se contiene, directa o indirectamente, no tiene un tipo finito"));
        }
        self.en_curso.borrow_mut().push(nombre.to_string());
        let r = self.campos(nombre, uso, &mut Vec::new());
        self.en_curso.borrow_mut().pop();
        match r {
            Ok(cs) => Ok(Tipo::Struct(
                cs.into_iter().map(|c| (c.nombre, c.tipo)).collect(),
            )),
            Err(fs) => Err(fs
                .into_iter()
                .next()
                .unwrap_or_else(|| Fallo::new(uso, format!("`{nombre}` no se deriva")))),
        }
    }

    /// Por qué una anotación no tiene tipo OOS, dicho con lo que el lector
    /// cree que escribió.
    fn sin_traduccion(&self, e: &Expr, hasta: Option<usize>, r: Rango) -> Fallo {
        let q = self.cualificar(e, hasta);
        let nombre = match e {
            Expr::Nombre(n, _) => Some(n.as_str()),
            _ => None,
        };
        match (q.as_deref(), nombre) {
            (None, Some(n)) => match de_donde(n) {
                Some(m) => Fallo::new(r, format!("`{n}` no está importado"))
                    .ayuda(format!("`from {m} import {n}`")),
                None if self.m.ligas.iter().any(|l| l.nombre == n) => Fallo::new(
                    r,
                    format!("`{n}` todavía no existe aquí"),
                )
                .ayuda(format!(
                    "se define más abajo, y la anotación se evalúa al definir la función (Python \
                         3.12): muévelo arriba, o escríbelo entre comillas, `\"{n}\"`"
                )),
                None => Fallo::new(r, format!("`{n}` no está definido")).ayuda(TIPOS),
            },
            (Some(q), Some(n)) if q == local(n) && de_donde(n).is_some() => Fallo::new(
                r,
                format!(
                    "`{n}` aquí es un nombre de este fichero, no el de `{}`",
                    de_donde(n).unwrap_or_default()
                ),
            )
            .ayuda("algo del módulo lo tapa: renómbralo"),
            (Some(q), _) if q.starts_with("<local>.") => Fallo::new(
                r,
                format!(
                    "`{}` es de este fichero y no tiene tipo en OOS",
                    &q["<local>.".len()..]
                ),
            )
            .ayuda(TIPOS),
            (Some(q), _) => Fallo::new(
                r,
                format!(
                    "`{}` no tiene tipo en OOS",
                    q.strip_prefix("builtins.").unwrap_or(q)
                ),
            )
            .ayuda(TIPOS),
            (None, None) => Fallo::new(r, "esta anotación no tiene tipo en OOS").ayuda(TIPOS),
        }
    }

    fn es_dataclass(&self, c: &Clase) -> bool {
        c.decoradores.iter().any(|d| {
            let f = match d {
                Expr::Llamada { funcion, .. } => funcion.as_ref(),
                x => x,
            };
            self.cualificar(f, Some(c.indice)).as_deref() == Some("dataclasses.dataclass")
        })
    }

    fn clase(&self, nombre: &str) -> Option<&Clase> {
        self.m.clases.iter().rev().find(|c| c.nombre == nombre)
    }

    /// Los campos de una `@dataclass` del fichero, con los de sus bases
    /// delante, como los ordena `dataclasses`.
    fn campos(
        &self,
        nombre: &str,
        uso: Rango,
        vistas: &mut Vec<String>,
    ) -> Result<Vec<Campo>, Vec<Fallo>> {
        let Some(c) = self
            .clase(nombre)
            .filter(|_| !vistas.iter().any(|v| v == nombre))
        else {
            return Err(vec![Fallo::new(
                uso,
                format!("`{nombre}` no es una `@dataclass` del fichero"),
            )]);
        };
        if !self.es_dataclass(c) {
            return Err(vec![
                Fallo::new(c.rango, format!("`{nombre}` no es una `@dataclass`")).ayuda(
                    "lo que devuelve una función es un tipo, o una `@dataclass` del mismo fichero: \
                 sus campos son `output`",
                ),
            ]);
        }
        vistas.push(nombre.to_string());
        let mut fallos = Vec::new();
        let mut campos: Vec<Campo> = Vec::new();
        let poner = |campos: &mut Vec<Campo>, x: Campo| match campos
            .iter_mut()
            .find(|y| y.nombre == x.nombre)
        {
            Some(y) => *y = x,
            None => campos.push(x),
        };
        for b in &c.bases {
            match self.cualificar(b, Some(c.indice)) {
                Some(q) if q == "builtins.object" => {}
                Some(q) if q.starts_with("<local>.") => match self.campos(&q["<local>.".len()..], b.rango(), vistas) {
                    Ok(cs) => cs.into_iter().for_each(|x| poner(&mut campos, x)),
                    Err(fs) => fallos.extend(fs),
                },
                _ => fallos.push(
                    Fallo::new(b.rango(), format!("`{nombre}` hereda de algo que no es una `@dataclass` del fichero"))
                        .ayuda("sus campos no se pueden leer sin ejecutar: hereda de una `@dataclass` de aquí, o de nada"),
                ),
            }
        }
        if c.con_palabras {
            fallos.push(Fallo::new(
                c.rango,
                format!("`{nombre}` lleva argumentos de clase (`metaclass=…`)"),
            ));
        }
        for x in &c.campos {
            let base = match self.anotacion(&x.anotacion, Some(c.indice)) {
                Ok((Expr::Indice(b, _, _), h)) => self.cualificar(b, h),
                Ok((e, h)) => self.cualificar(e, h),
                Err(_) => None,
            };
            if base.is_some_and(|q| NO_SON_CAMPOS.contains(&q.as_str())) {
                continue;
            }
            let defecto = match &x.valor {
                None => false,
                Some(Expr::Llamada {
                    funcion, nombrados, ..
                }) if self.cualificar(funcion, Some(c.indice)).as_deref()
                    == Some("dataclasses.field") =>
                {
                    nombrados
                        .iter()
                        .any(|(k, _)| matches!(k.as_deref(), Some("default" | "default_factory")))
                }
                Some(_) => true,
            };
            match self.tipo(&x.anotacion, Some(c.indice), false) {
                Ok((tipo, opcional)) => poner(
                    &mut campos,
                    Campo {
                        nombre: x.nombre.clone(),
                        tipo,
                        requerido: !(defecto || opcional),
                    },
                ),
                Err(f) => fallos.push(f),
            }
        }
        vistas.pop();
        if fallos.is_empty() {
            Ok(campos)
        } else {
            Err(fallos)
        }
    }

    fn una(&self, def: &Def, deco: &Expr, ruta: &str) -> Result<Firma, Vec<Fallo>> {
        let mut fallos = Vec::new();
        let (mut over, mut reads, mut models, mut timeout) = (None, None, None, None);
        let mut hay_over = false;

        // ── §4.2 · los argumentos del decorador, literales ──────────────────
        if let Expr::Llamada {
            posicionales,
            nombrados,
            rango,
            ..
        } = deco
        {
            if *posicionales > 0 {
                fallos.push(
                    Fallo::new(*rango, "`@function` con argumentos posicionales")
                        .ayuda("se dicen por nombre: `over=`, `reads=`, `models=`, `timeout=`"),
                );
            }
            for (k, v) in nombrados {
                match k.as_deref() {
                    Some("over") => {
                        hay_over = true;
                        over = cadena(v, "over", &mut fallos);
                    }
                    Some("timeout") => timeout = cadena(v, "timeout", &mut fallos),
                    Some("reads") => reads = lista(v, "reads", &mut fallos),
                    Some("models") => {
                        models = lista(v, "models", &mut fallos).map(|ms| {
                            ms.into_iter()
                                .map(|x| {
                                    if x.starts_with("modelo/") {
                                        x
                                    } else {
                                        format!("modelo/{x}")
                                    }
                                })
                                .collect()
                        })
                    }
                    Some(otro) => fallos.push(
                        Fallo::new(
                            v.rango(),
                            format!("`@function` no tiene el argumento `{otro}`"),
                        )
                        .ayuda("los suyos son `over`, `reads`, `models` y `timeout`"),
                    ),
                    None => fallos.push(
                        Fallo::new(v.rango(), "`@function(**…)`")
                            .ayuda("los argumentos se leen sin ejecutar: escríbelos uno a uno"),
                    ),
                }
            }
        }

        // ── §4.4 · los parámetros ────────────────────────────────────────────
        for (v, r) in &def.variadicos {
            fallos.push(
                Fallo::new(*r, format!("`{v}` en una función"))
                    .ayuda("la superficie es cerrada: un consumidor solo ve lo que se nombra. Nombra cada parámetro"),
            );
        }
        let mut params: Vec<_> = def
            .posicionales
            .iter()
            .chain(def.solo_nombre.iter())
            .collect();
        if hay_over {
            match def.posicionales.first() {
                Some(p) if !p.defecto => {
                    params.remove(0);
                }
                _ => fallos.push(
                    Fallo::new(def.rango, format!("`{}` trabaja sobre `over` y no recibe la fila", def.nombre))
                        .ayuda("con `over`, el primer parámetro es la fila: posicional y sin valor por defecto"),
                ),
            }
        }
        let mut entrada = Vec::new();
        for p in params {
            let Some(a) = &p.anotacion else {
                fallos.push(
                    Fallo::new(p.rango, format!("`{}` sin anotar", p.nombre)).ayuda(format!(
                        "la firma es lo que un consumidor ve: `{}: str`, `{}: int`…",
                        p.nombre, p.nombre
                    )),
                );
                continue;
            };
            match self.tipo(a, Some(def.indice), false) {
                Ok((tipo, opcional)) => entrada.push(Campo {
                    nombre: p.nombre.clone(),
                    tipo,
                    requerido: !(p.defecto || opcional),
                }),
                Err(f) => fallos.push(f),
            }
        }

        // ── §4.5 · lo que devuelve ───────────────────────────────────────────
        let salida = match &def.retorno {
            None | Some(Expr::Nada(_)) => {
                fallos.push(
                    Fallo::new(
                        def.rango,
                        format!("`{}` no dice lo que devuelve", def.nombre),
                    )
                    .ayuda("anótalo: `-> str`, `-> list[int]`, o una `@dataclass` del fichero"),
                );
                None
            }
            Some(r) => match self.anotacion(r, Some(def.indice)) {
                Err(f) => {
                    fallos.push(f);
                    None
                }
                Ok((x, h)) => {
                    let q = self.cualificar(x, h);
                    let clase = q
                        .as_deref()
                        .and_then(|q| q.strip_prefix("<local>."))
                        .filter(|n| self.clase(n).is_some());
                    match clase {
                        Some(n) => match self.campos(n, x.rango(), &mut Vec::new()) {
                            Ok(cs) => Some(Salida::Campos(cs)),
                            Err(fs) => {
                                fallos.extend(fs);
                                None
                            }
                        },
                        None => match self.tipo(r, Some(def.indice), false) {
                            Ok((t, _)) => Some(Salida::Valor(t)),
                            Err(f) => {
                                fallos.push(f);
                                None
                            }
                        },
                    }
                }
            },
        };

        fallos.sort_by_key(|f| f.rango);
        match salida {
            Some(salida) if fallos.is_empty() => Ok(Firma {
                nombre: def.nombre.clone(),
                entrypoint: format!("{ruta}:{}", def.nombre),
                descripcion: def
                    .docstring
                    .as_deref()
                    .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
                    .map(str::to_string),
                over,
                reads,
                models,
                timeout,
                entrada,
                salida,
            }),
            _ => Err(fallos),
        }
    }
}

const LITERAL: &str = "se lee sin ejecutar el fichero: escribe el valor tal cual, entre comillas";

/// Una unidad de `Money` o `Quantity`: lo que cabe entre `<` y `,` en el tipo.
fn es_unidad(u: &str) -> bool {
    !u.is_empty()
        && !u
            .chars()
            .any(|c| c.is_whitespace() || "<>,:\"'".contains(c))
}

fn cadena(v: &Expr, k: &str, fallos: &mut Vec<Fallo>) -> Option<String> {
    match v {
        Expr::Cadena(s, _) => Some(s.clone()),
        x => {
            fallos.push(
                Fallo::new(x.rango(), format!("`{k}` no es una cadena literal")).ayuda(LITERAL),
            );
            None
        }
    }
}

fn lista(v: &Expr, k: &str, fallos: &mut Vec<Fallo>) -> Option<Vec<String>> {
    let Expr::Lista(xs, _) = v else {
        fallos.push(
            Fallo::new(
                v.rango(),
                format!("`{k}` no es una lista literal de cadenas"),
            )
            .ayuda(LITERAL),
        );
        return None;
    };
    let mut out = Vec::new();
    for x in xs {
        match x {
            Expr::Cadena(s, _) => out.push(s.clone()),
            x => fallos.push(
                Fallo::new(
                    x.rango(),
                    format!("un elemento de `{k}` no es una cadena literal"),
                )
                .ayuda(LITERAL),
            ),
        }
    }
    Some(out)
}
