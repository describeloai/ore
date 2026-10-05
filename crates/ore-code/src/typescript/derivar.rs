//! OOS v1alpha23 `01` §3–§7 sobre la sintaxis propia: qué es una función de
//! TypeScript, qué firma tiene, y por qué no se deriva cuando no se deriva.

use super::sintaxis::{
    Config, Cuerpo, DeclTipo, Defecto, Funcion as Fn, Miembro, Modulo, T, Valor,
};
use crate::firma::{Campo, Def, Derivacion, Fallo, Firma, Funcion, Rango, Salida, Tipo};
use std::cell::RefCell;

const TIPOS: &str = "una firma usa `string`, `number`, `boolean`, `bigint`, `Date`, `Uint8Array`, \
                     `T[]` y `T | null`; de `ore`, `Integer`, `Decimal<p, s>`, `Money<\"EUR\", 2>`, \
                     `Quantity<\"km\", 1>`, `LocalDate`, `LocalTime`, `LocalDateTime` y \
                     `Media<\"a.b.c\">`; y un `interface` o un `type` de objeto del mismo fichero";

const LITERAL: &str = "se lee sin ejecutar el fichero: escribe el valor tal cual, entre comillas";

/// Los tipos del SDK que la firma lee (v1alpha23 `01` §6.3).
const DEL_SDK: &[&str] = &[
    "Integer",
    "Decimal",
    "Money",
    "Quantity",
    "LocalDate",
    "LocalTime",
    "LocalDateTime",
    "Media",
    "Config",
];

/// Los globales que la firma lee, salvo que el fichero los tape.
const GLOBALES: &[&str] = &["Date", "Uint8Array", "Array", "ReadonlyArray", "Promise"];

/// Si `ruta` es la de un fichero que puede ser una función (v1alpha23 `01`
/// §3): un `.ts` —no un `.d.ts`, ni una prueba— con un directorio `functions`
/// en su ruta.
pub fn es_de_funciones(ruta: &str) -> bool {
    let mut partes: Vec<&str> = ruta.split('/').collect();
    let Some(fichero) = partes.pop() else {
        return false;
    };
    fichero.ends_with(".ts")
        && !fichero.ends_with(".d.ts")
        && !fichero.ends_with(".test.ts")
        && !fichero.ends_with(".spec.ts")
        && partes.contains(&"functions")
}

/// El nombre que la función publica: el del fichero, sin `.ts`.
pub fn nombre_del_fichero(ruta: &str) -> &str {
    let f = ruta.rsplit('/').next().unwrap_or(ruta);
    f.strip_suffix(".ts").unwrap_or(f)
}

fn es_identificador(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|p| p.is_ascii_alphabetic())
        && c.all(|p| p.is_ascii_alphanumeric() || p == '_')
        && n.len() <= 128
}

pub fn derivar(m: &Modulo, ruta: &str) -> Derivacion {
    let mut d = Derivacion {
        sintaxis: m.sintaxis.clone(),
        version: m.no_borrable.clone(),
        ..Derivacion::default()
    };
    let es = es_de_funciones(ruta);
    if let Some(Defecto::Funcion(f)) = &m.defecto {
        d.defs.push(Def {
            nombre: f.nombre.clone().unwrap_or_default(),
            rango: f.rango,
            asincrona: f.asincrona,
            decorada: es,
            transformada: false,
        });
    }
    let Some(defecto) = &m.defecto else {
        return d;
    };
    if !es {
        return d;
    }
    let nombre = nombre_del_fichero(ruta).to_string();
    let r = Resolutor {
        m,
        en_curso: RefCell::default(),
    };
    let (rango, resultado) = match defecto {
        Defecto::Funcion(f) => (f.rango, r.una(f, &nombre, ruta)),
        Defecto::Otra(que, rango) => (
            *rango,
            Err(vec![
                Fallo::new(*rango, format!("lo que se exporta por defecto es {que}")).ayuda(
                    format!(
                        "una función es `export default function {nombre}(…): T {{ … }}`; un \
                         módulo de ayuda de `functions/` no exporta nada por defecto"
                    ),
                ),
            ]),
        ),
    };
    d.funciones.push(Funcion {
        nombre,
        rango,
        resultado,
    });
    d
}

/// A qué se refiere un nombre de tipo.
enum Que<'m> {
    /// Un tipo del SDK, `ore`.
    Sdk(String),
    Global(&'static str),
    Local(&'m DeclTipo),
    /// Otra cosa con nombre: de otro módulo, o de este fichero sin ser un tipo
    /// de objeto. Dicho con palabras.
    Ajeno(String),
    SinLigar,
}

struct Resolutor<'m> {
    m: &'m Modulo,
    /// Los objetos que se están derivando como `Struct`, para no entrar en uno
    /// que se contiene a sí mismo.
    en_curso: RefCell<Vec<String>>,
}

impl<'m> Resolutor<'m> {
    fn resolver(&self, nombre: &[String]) -> Que<'m> {
        match nombre {
            [n] => {
                if let Some(d) = self.m.tipos.iter().find(|d| &d.nombre == n) {
                    return Que::Local(d);
                }
                if self.m.otros_nombres.iter().any(|x| x == n) {
                    return Que::Ajeno(format!(
                        "`{n}` es un nombre de este fichero que no es un tipo de objeto"
                    ));
                }
                if let Some(i) = self.m.importados.iter().rev().find(|i| &i.local == n) {
                    return match (&i.nombre, i.modulo.as_str()) {
                        (Some(x), "ore") if x != "default" => Que::Sdk(x.clone()),
                        _ => Que::Ajeno(format!("`{n}` se importa de `{}`", i.modulo)),
                    };
                }
                match GLOBALES.iter().find(|g| *g == n) {
                    Some(g) => Que::Global(g),
                    None => Que::SinLigar,
                }
            }
            [ns, x] => match self.m.importados.iter().rev().find(|i| &i.local == ns) {
                Some(i) if i.modulo == "ore" && i.nombre.is_none() => Que::Sdk(x.clone()),
                Some(i) => Que::Ajeno(format!("`{ns}.{x}` es de `{}`", i.modulo)),
                None => Que::Ajeno(format!("`{ns}.{x}`")),
            },
            _ => Que::Ajeno(format!("`{}`", nombre.join("."))),
        }
    }

    /// Un tipo → (tipo OOS, si es opcional).
    fn tipo(&self, t: &T, en_lista: bool) -> Result<(Tipo, bool), Fallo> {
        match t {
            T::Palabra(p, r) => match *p {
                "string" => Ok((Tipo::String, false)),
                "number" => Ok((Tipo::Float, false)),
                "boolean" => Ok((Tipo::Boolean, false)),
                "bigint" => Ok((Tipo::Bigint, false)),
                "null" | "undefined" => Err(Fallo::new(*r, format!("`{p}` solo no es un tipo"))
                    .ayuda(
                        "`T | null` dice que un valor puede faltar; `null` a secas no dice cuál",
                    )),
                otra => Err(Fallo::new(*r, format!("`{otra}` no tiene tipo en OOS")).ayuda(TIPOS)),
            },
            T::Union(ms, r) => {
                let mut planos = Vec::new();
                aplanar(ms, &mut planos);
                let nada = planos
                    .iter()
                    .filter(|x| matches!(x, T::Palabra("null" | "undefined", _)))
                    .count();
                let otros: Vec<_> = planos
                    .iter()
                    .filter(|x| !matches!(x, T::Palabra("null" | "undefined", _)))
                    .collect();
                if otros.len() == 1 && nada > 0 {
                    return Ok((self.tipo(otros[0], en_lista)?.0, true));
                }
                if !otros.is_empty()
                    && otros
                        .iter()
                        .all(|x| matches!(x, T::Cadena(..) | T::Numero(..)))
                {
                    return Err(Fallo::new(*r, "una unión de literales").ayuda(
                        "OOS no tiene un enumerado en la firma: escribe `string` y comprueba el \
                         valor en el código",
                    ));
                }
                Err(Fallo::new(*r, "una unión que no es `T | null`").ayuda(
                    "un parámetro tiene un tipo; `T | null` o `T | undefined` dicen que puede \
                     faltar. Dos tipos distintos no son una firma",
                ))
            }
            T::Lista(x, r) => self.lista(x, *r, en_lista),
            T::Cadena(_, r) | T::Numero(_, r) => {
                Err(Fallo::new(*r, "un literal no es un tipo").ayuda(TIPOS))
            }
            T::Objeto(ms, r) => self.estructura_de(ms, "{ … }", *r).map(|t| (t, false)),
            T::Otro(que, r) => {
                Err(Fallo::new(*r, format!("{que} no tiene tipo en OOS")).ayuda(TIPOS))
            }
            T::Ref {
                nombre,
                args,
                rango,
            } => self.referencia(nombre, args, *rango, en_lista),
        }
    }

    fn lista(&self, x: &T, r: Rango, en_lista: bool) -> Result<(Tipo, bool), Fallo> {
        if en_lista {
            return Err(
                Fallo::new(r, "una lista de listas").ayuda("`list<T>` no lleva listas dentro")
            );
        }
        let (t, opcional) = self.tipo(x, true)?;
        if opcional {
            return Err(Fallo::new(x.rango(), "una lista de opcionales").ayuda(
                "los elementos de una lista no faltan: `number[]`, no `(number | null)[]`",
            ));
        }
        Ok((Tipo::Lista(Box::new(t)), false))
    }

    fn referencia(
        &self,
        nombre: &[String],
        args: &[T],
        r: Rango,
        en_lista: bool,
    ) -> Result<(Tipo, bool), Fallo> {
        let escrito = nombre.join(".");
        let sin_args = |t: Tipo| {
            if args.is_empty() {
                Ok((t, false))
            } else {
                Err(Fallo::new(
                    r,
                    format!("`{escrito}` no lleva argumentos de tipo"),
                ))
            }
        };
        match self.resolver(nombre) {
            Que::Global(g) => match (g, args) {
                ("Array" | "ReadonlyArray", [x]) => self.lista(x, r, en_lista),
                ("Date", _) => sin_args(Tipo::DateTimeTz),
                ("Uint8Array", _) => sin_args(Tipo::Opaque),
                ("Promise", _) => Err(Fallo::new(
                    r,
                    "`Promise<…>` solo es lo que devuelve una función `async`",
                )
                .ayuda("un parámetro llega resuelto: escribe su tipo")),
                _ => Err(
                    Fallo::new(r, format!("`{g}` con esos argumentos no es un tipo"))
                        .ayuda("`Array<T>`, con un tipo"),
                ),
            },
            Que::Sdk(x) => self.del_sdk(&x, &escrito, args, r),
            Que::Local(d) => {
                if !args.is_empty() || d.genericos {
                    return Err(Fallo::new(r, format!("`{}` es genérico", d.nombre))
                        .ayuda("un tipo de la firma es concreto: escribe sus campos sin parámetros de tipo"));
                }
                match &d.cuerpo {
                    Cuerpo::Interface { .. } | Cuerpo::Alias(T::Objeto(..)) => {
                        self.estructura(d, r).map(|t| (t, false))
                    }
                    Cuerpo::Alias(_) => Err(Fallo::new(r, format!(
                        "`{}` es un alias de otro tipo, no un objeto",
                        d.nombre
                    ))
                    .ayuda("en la firma se escribe el tipo mismo; un `type` del fichero es un objeto, `type X = { … }`")),
                }
            }
            Que::Ajeno(que) => Err(
                Fallo::new(r, format!("{que}, y no tiene tipo en OOS")).ayuda(format!(
                    "{TIPOS}. Un tipo de otro fichero no se lee: la derivación es de este"
                )),
            ),
            Que::SinLigar => {
                let n = nombre.last().map(String::as_str).unwrap_or_default();
                let f = Fallo::new(r, format!("`{escrito}` no está definido"));
                Err(if DEL_SDK.contains(&n) {
                    f.ayuda(format!("es del SDK: `import type {{ {n} }} from \"ore\"`"))
                } else {
                    f.ayuda(TIPOS)
                })
            }
        }
    }

    /// Un tipo de `ore` (v1alpha23 `01` §6.3).
    fn del_sdk(&self, x: &str, escrito: &str, args: &[T], r: Rango) -> Result<(Tipo, bool), Fallo> {
        let sin = |t: Tipo| {
            if args.is_empty() {
                Ok((t, false))
            } else {
                Err(Fallo::new(
                    r,
                    format!("`{escrito}` no lleva argumentos de tipo"),
                ))
            }
        };
        match x {
            "Integer" => sin(Tipo::Integer),
            "LocalDate" => sin(Tipo::Date),
            "LocalTime" => sin(Tipo::Time),
            "LocalDateTime" => sin(Tipo::DateTime),
            "Decimal" => match args {
                [] => Ok((Tipo::Decimal, false)),
                [p, s] => match (entero(p), entero(s)) {
                    (Some(p), Some(s)) if (1..=38).contains(&p) && s <= p => Ok((
                        Tipo::DecimalPs {
                            precision: p as u8,
                            escala: s as u8,
                        },
                        false,
                    )),
                    (Some(p), Some(s)) => Err(Fallo::new(r, format!("`Decimal<{p}, {s}>` está fuera de rango"))
                        .ayuda("`1 ≤ p ≤ 38` y `0 ≤ s ≤ p`: las cifras en total y las de detrás de la coma")),
                    _ => Err(Fallo::new(r, "`Decimal<p, s>` lleva dos enteros literales")
                        .ayuda("`Decimal<12, 2>`: doce cifras, dos detrás de la coma")),
                },
                _ => Err(Fallo::new(r, "`Decimal` lleva la precisión y la escala, o nada")
                    .ayuda("`Decimal<12, 2>`, o `Decimal` sin decir cuántas")),
            },
            "Money" | "Quantity" => {
                let ejemplo = if x == "Money" { "\"EUR\", 2" } else { "\"km\", 1" };
                match args {
                    [T::Cadena(u, ur), p] => {
                        if !es_unidad(u) {
                            return Err(Fallo::new(*ur, format!("`{u}` no es una unidad")).ayuda(format!("`{x}<{ejemplo}>`")));
                        }
                        let Some(p) = entero(p) else {
                            return Err(Fallo::new(p.rango(), format!("la precisión de `{x}<…>` es un entero literal"))
                                .ayuda("los decimales, como número: `2`"));
                        };
                        Ok((
                            Tipo::Unidad {
                                ctor: if x == "Money" { "Money" } else { "Quantity" },
                                unidad: u.clone(),
                                precision: p,
                            },
                            false,
                        ))
                    }
                    [u, _] => Err(Fallo::new(u.rango(), format!("la unidad de `{x}<…>` es una cadena literal"))
                        .ayuda(format!("`{x}<{ejemplo}>`: {LITERAL}"))),
                    _ => Err(Fallo::new(r, format!("`{x}<…>` lleva la unidad y la precisión"))
                        .ayuda(format!("`{x}<{ejemplo}>`: la unidad, entre comillas, y los decimales"))),
                }
            }
            "Media" => match args {
                [T::Cadena(c, _)]
                    if c.split('.').all(|p| {
                        !p.is_empty() && p.chars().all(|x| x.is_ascii_alphanumeric() || x == '_')
                    }) =>
                {
                    Ok((Tipo::Media(c.clone()), false))
                }
                _ => Err(Fallo::new(r, "`Media<…>` nombra una colección, entre comillas")
                    .ayuda("`Media<\"base.schema.coleccion\">`")),
            },
            "Config" => Err(Fallo::new(r, "`Config` es el tipo de `config`, no el de un valor")
                .ayuda(TIPOS)),
            otro => Err(Fallo::new(r, format!("`ore` no tiene el tipo `{otro}`")).ayuda(TIPOS)),
        }
    }

    /// Un `interface` o un `type` de objeto como `Struct<…>`.
    fn estructura(&self, d: &DeclTipo, uso: Rango) -> Result<Tipo, Fallo> {
        if self.en_curso.borrow().iter().any(|n| n == &d.nombre) {
            return Err(
                Fallo::new(uso, format!("`{}` se contiene a sí mismo", d.nombre)).ayuda(
                    "un objeto que se contiene, directa o indirectamente, no tiene un tipo finito",
                ),
            );
        }
        self.en_curso.borrow_mut().push(d.nombre.clone());
        let r = self.campos(d, uso, &mut Vec::new());
        self.en_curso.borrow_mut().pop();
        r.map(struct_de).map_err(|fs| primero(fs, uso, &d.nombre))
    }

    fn estructura_de(&self, ms: &[Miembro], que: &str, uso: Rango) -> Result<Tipo, Fallo> {
        self.miembros(ms)
            .map(struct_de)
            .map_err(|fs| primero(fs, uso, que))
    }

    /// Los campos de un objeto del fichero, con los de lo que extiende delante.
    fn campos(
        &self,
        d: &DeclTipo,
        uso: Rango,
        vistas: &mut Vec<String>,
    ) -> Result<Vec<Campo>, Vec<Fallo>> {
        if vistas.iter().any(|v| v == &d.nombre) {
            return Err(vec![Fallo::new(
                uso,
                format!("`{}` se extiende a sí mismo", d.nombre),
            )]);
        }
        if d.genericos {
            return Err(vec![
                Fallo::new(d.rango, format!("`{}` es genérico", d.nombre)).ayuda(
                    "un tipo de la firma es concreto: escribe sus campos sin parámetros de tipo",
                ),
            ]);
        }
        vistas.push(d.nombre.clone());
        let r = match &d.cuerpo {
            Cuerpo::Alias(T::Objeto(ms, _)) => self.miembros(ms),
            Cuerpo::Alias(otro) => Err(vec![Fallo::new(otro.rango(), format!("`{}` no es un objeto", d.nombre))
                .ayuda("lo que devuelve una función es un tipo, o un objeto del mismo fichero: sus campos son `output`")]),
            Cuerpo::Interface { extends, miembros } => {
                let mut fallos = Vec::new();
                let mut campos: Vec<Campo> = Vec::new();
                for (n, con_args, r) in extends {
                    match self.resolver(n) {
                        Que::Local(b) if !con_args && matches!(b.cuerpo, Cuerpo::Interface { .. }) => {
                            match self.campos(b, *r, vistas) {
                                Ok(cs) => cs.into_iter().for_each(|x| poner(&mut campos, x)),
                                Err(fs) => fallos.extend(fs),
                            }
                        }
                        _ => fallos.push(Fallo::new(*r, format!("`{}` extiende algo que no es un `interface` del fichero", d.nombre))
                            .ayuda("sus campos no se leen sin el compilador: extiende un `interface` de aquí, o nada")),
                    }
                }
                match self.miembros(miembros) {
                    Ok(cs) => cs.into_iter().for_each(|x| poner(&mut campos, x)),
                    Err(fs) => fallos.extend(fs),
                }
                if fallos.is_empty() { Ok(campos) } else { Err(fallos) }
            }
        };
        vistas.pop();
        r
    }

    fn miembros(&self, ms: &[Miembro]) -> Result<Vec<Campo>, Vec<Fallo>> {
        let mut fallos = Vec::new();
        let mut campos = Vec::new();
        for m in ms {
            let Some(nombre) = &m.nombre else {
                fallos.push(Fallo::new(m.rango, format!("{} en un objeto de la firma", m.problema.unwrap_or("algo que no es una propiedad")))
                    .ayuda("un objeto de la firma tiene propiedades con nombre y tipo: `total: number`"));
                continue;
            };
            let Some(t) = &m.tipo else {
                fallos.push(
                    Fallo::new(m.rango, format!("`{nombre}` sin tipo"))
                        .ayuda(format!("`{nombre}: string`, `{nombre}: number`…")),
                );
                continue;
            };
            match self.tipo(t, false) {
                Ok((tipo, opcional)) => campos.push(Campo {
                    nombre: nombre.clone(),
                    tipo,
                    requerido: !(m.opcional || opcional),
                }),
                Err(f) => fallos.push(f),
            }
        }
        if fallos.is_empty() {
            Ok(campos)
        } else {
            Err(fallos)
        }
    }

    /// Si un tipo de vuelta es un objeto: sus miembros, o la declaración.
    fn objeto(&self, t: &T) -> Option<Result<Vec<Campo>, Vec<Fallo>>> {
        match t {
            T::Objeto(ms, _) => Some(self.miembros(ms)),
            T::Ref {
                nombre,
                args,
                rango,
            } if args.is_empty() => match self.resolver(nombre) {
                Que::Local(d)
                    if matches!(
                        d.cuerpo,
                        Cuerpo::Interface { .. } | Cuerpo::Alias(T::Objeto(..))
                    ) =>
                {
                    Some(self.campos(d, *rango, &mut Vec::new()))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn una(&self, f: &Fn, nombre: &str, ruta: &str) -> Result<Firma, Vec<Fallo>> {
        let mut fallos = Vec::new();

        // ── §3 · el nombre es el del fichero ──────────────────────────────────
        if !es_identificador(nombre) {
            fallos.push(Fallo::new(f.rango, format!("`{nombre}.ts` no da un nombre de función"))
                .ayuda("el nombre publicado es el del fichero: empieza por una letra y sigue con letras, cifras o `_` (`quoteOrder.ts`)"));
        }
        match &f.nombre {
            None => fallos.push(Fallo::new(f.rango, "la función exportada por defecto no tiene nombre")
                .ayuda(format!("`export default function {nombre}(…)`: el mismo nombre que el fichero"))),
            Some(n) if n != nombre => fallos.push(Fallo::new(f.rango, format!("la función se llama `{n}` y el fichero `{nombre}.ts`"))
                .ayuda("el nombre publicado es el del fichero, y la declaración dice el mismo: renombra uno de los dos")),
            Some(_) => {}
        }
        if f.generadora {
            fallos.push(
                Fallo::new(f.rango, "una función generadora (`function*`)")
                    .ayuda("el runtime llama a la función y espera su valor"),
            );
        }
        if let Some(r) = f.genericos {
            fallos.push(
                Fallo::new(r, "una función genérica")
                    .ayuda("la firma es concreta: un consumidor ve tipos, no parámetros de tipo"),
            );
        }
        if let Some(r) = f.this {
            fallos.push(
                Fallo::new(r, "un parámetro `this`")
                    .ayuda("una función no tiene `this`: nombra lo que recibe"),
            );
        }

        // ── §4 · config ───────────────────────────────────────────────────────
        let (mut over, mut reads, mut models, mut timeout) = (None, None, None, None);
        let mut hay_over = false;
        if let Some(c) = &self.m.config {
            leer_config(
                c,
                &mut fallos,
                &mut hay_over,
                &mut over,
                &mut reads,
                &mut models,
                &mut timeout,
            );
        }

        // ── §5.1 · los parámetros ─────────────────────────────────────────────
        if let Some(r) = f.resto {
            fallos.push(Fallo::new(r, "un parámetro `...resto`")
                .ayuda("la superficie es cerrada: un consumidor solo ve lo que se nombra. Nombra cada parámetro"));
        }
        let mut params: Vec<_> = f.params.iter().collect();
        if hay_over {
            match params.first() {
                Some(p) if !p.defecto && !p.opcional => {
                    params.remove(0);
                }
                _ => fallos.push(
                    Fallo::new(
                        f.rango,
                        format!("`{nombre}` trabaja sobre `over` y no recibe la fila"),
                    )
                    .ayuda(
                        "con `over`, el primer parámetro es la fila: sin valor por defecto ni `?`",
                    ),
                ),
            }
        }
        let mut entrada = Vec::new();
        for p in params {
            let Some(n) = &p.nombre else {
                fallos.push(Fallo::new(p.rango, "un parámetro desestructurado")
                    .ayuda("los parámetros se pasan por su nombre: `pedido: Pedido`, y dentro, `pedido.total`"));
                continue;
            };
            let Some(t) = &p.tipo else {
                fallos.push(
                    Fallo::new(p.rango, format!("`{n}` sin anotar")).ayuda(format!(
                        "la firma es lo que un consumidor ve: `{n}: string`, `{n}: Integer`…"
                    )),
                );
                continue;
            };
            match self.tipo(t, false) {
                Ok((tipo, opcional)) => entrada.push(Campo {
                    nombre: n.clone(),
                    tipo,
                    requerido: !(p.defecto || p.opcional || opcional),
                }),
                Err(x) => fallos.push(x),
            }
        }

        // ── §5.2 · lo que devuelve ────────────────────────────────────────────
        let salida = match &f.retorno {
            None => {
                fallos.push(Fallo::new(f.rango, format!("`{nombre}` no dice lo que devuelve"))
                    .ayuda("anótalo: `): string`, `): Promise<Total>`; TypeScript lo infiere, y una inferencia no se lee sin el compilador"));
                None
            }
            Some(t) => {
                let t = self.sin_promesa(t, f.asincrona, &mut fallos);
                match t {
                    None => None,
                    Some(T::Palabra(p @ ("void" | "undefined" | "null" | "never"), r)) => {
                        fallos.push(Fallo::new(*r, format!("`{nombre}` devuelve `{p}`"))
                            .ayuda("una función devuelve un valor: su `output`. Lo que solo hace algo no es una función"));
                        None
                    }
                    Some(t) => match self.objeto(t) {
                        Some(Ok(cs)) => Some(Salida::Campos(cs)),
                        Some(Err(fs)) => {
                            fallos.extend(fs);
                            None
                        }
                        None => match self.tipo(t, false) {
                            Ok((x, _)) => Some(Salida::Valor(x)),
                            Err(x) => {
                                fallos.push(x);
                                None
                            }
                        },
                    },
                }
            }
        };

        fallos.sort_by_key(|f| f.rango);
        match salida {
            Some(salida) if fallos.is_empty() => Ok(Firma {
                nombre: nombre.to_string(),
                entrypoint: ruta.to_string(),
                descripcion: f.jsdoc.as_deref().and_then(descripcion),
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

    /// `Promise<T>` → `T`: lo que una función `async` devuelve, resuelto.
    fn sin_promesa<'t>(&self, t: &'t T, asincrona: bool, fallos: &mut Vec<Fallo>) -> Option<&'t T> {
        match t {
            T::Ref {
                nombre,
                args,
                rango,
            } if matches!(self.resolver(nombre), Que::Global("Promise")) => match args.as_slice() {
                [x] => Some(x),
                _ => {
                    fallos.push(
                        Fallo::new(*rango, "`Promise` sin el tipo que resuelve")
                            .ayuda("`Promise<string>`"),
                    );
                    None
                }
            },
            _ if asincrona => {
                fallos.push(
                    Fallo::new(t.rango(), "una función `async` devuelve `Promise<T>`")
                        .ayuda("escribe lo que resuelve dentro: `Promise<string>`"),
                );
                None
            }
            _ => Some(t),
        }
    }
}

fn leer_config(
    c: &Config,
    fallos: &mut Vec<Fallo>,
    hay_over: &mut bool,
    over: &mut Option<String>,
    reads: &mut Option<Vec<String>>,
    models: &mut Option<Vec<String>>,
    timeout: &mut Option<String>,
) {
    if let Some(p) = c.problema {
        fallos.push(
            Fallo::new(c.rango, format!("`config` {p}")).ayuda(
                "`export const config = { … }`, en el nivel superior: así se lee sin ejecutar",
            ),
        );
        return;
    }
    let Some(Valor::Objeto(claves, _)) = &c.valor else {
        let que = c
            .valor
            .as_ref()
            .map(que_es)
            .unwrap_or("una declaración sin valor");
        fallos.push(
            Fallo::new(
                c.valor.as_ref().map(Valor::rango).unwrap_or(c.rango),
                format!("`config` no es un objeto literal: es {que}"),
            )
            .ayuda("`export const config = { timeout: \"30s\" }`"),
        );
        return;
    };
    for (k, v) in claves {
        match k.as_deref() {
            Some("over") => {
                *hay_over = true;
                *over = cadena(v, "over", fallos);
            }
            Some("timeout") => *timeout = cadena(v, "timeout", fallos),
            Some("reads") => *reads = lista(v, "reads", fallos),
            Some("models") => {
                *models = lista(v, "models", fallos).map(|ms| {
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
            Some(otra) => fallos.push(
                Fallo::new(v.rango(), format!("`config` no tiene la clave `{otra}`"))
                    .ayuda("las suyas son `over`, `reads`, `models` y `timeout`"),
            ),
            None => fallos.push(
                Fallo::new(
                    v.rango(),
                    "una clave de `config` que no se lee sin ejecutar",
                )
                .ayuda("escribe cada clave tal cual: `timeout: \"30s\"`"),
            ),
        }
    }
}

fn cadena(v: &Valor, k: &str, fallos: &mut Vec<Fallo>) -> Option<String> {
    match v {
        Valor::Cadena(s, _) => Some(s.clone()),
        x => {
            fallos.push(
                Fallo::new(
                    x.rango(),
                    format!("`{k}` no es una cadena literal: es {}", que_es(x)),
                )
                .ayuda(LITERAL),
            );
            None
        }
    }
}

fn lista(v: &Valor, k: &str, fallos: &mut Vec<Fallo>) -> Option<Vec<String>> {
    let Valor::Lista(xs, _) = v else {
        fallos.push(
            Fallo::new(
                v.rango(),
                format!("`{k}` no es una lista literal de cadenas: es {}", que_es(v)),
            )
            .ayuda(LITERAL),
        );
        return None;
    };
    let mut out = Vec::new();
    for x in xs {
        match x {
            Valor::Cadena(s, _) => out.push(s.clone()),
            x => fallos.push(
                Fallo::new(
                    x.rango(),
                    format!(
                        "un elemento de `{k}` no es una cadena literal: es {}",
                        que_es(x)
                    ),
                )
                .ayuda(LITERAL),
            ),
        }
    }
    Some(out)
}

fn que_es(v: &Valor) -> &'static str {
    match v {
        Valor::Cadena(..) => "una cadena",
        Valor::Lista(..) => "una lista",
        Valor::Objeto(..) => "un objeto",
        Valor::Otro(que, _) => que,
    }
}

fn aplanar<'t>(ms: &'t [T], out: &mut Vec<&'t T>) {
    for m in ms {
        match m {
            T::Union(xs, _) => aplanar(xs, out),
            x => out.push(x),
        }
    }
}

fn entero(t: &T) -> Option<u32> {
    match t {
        T::Numero(s, _) if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) => s.parse().ok(),
        _ => None,
    }
}

/// Una unidad de `Money` o `Quantity`: lo que cabe entre `<` y `,` en el tipo.
fn es_unidad(u: &str) -> bool {
    !u.is_empty()
        && !u
            .chars()
            .any(|c| c.is_whitespace() || "<>,:\"'".contains(c))
}

fn poner(campos: &mut Vec<Campo>, x: Campo) {
    match campos.iter_mut().find(|y| y.nombre == x.nombre) {
        Some(y) => *y = x,
        None => campos.push(x),
    }
}

fn struct_de(cs: Vec<Campo>) -> Tipo {
    Tipo::Struct(cs.into_iter().map(|c| (c.nombre, c.tipo)).collect())
}

fn primero(fs: Vec<Fallo>, uso: Rango, que: &str) -> Fallo {
    fs.into_iter()
        .next()
        .unwrap_or_else(|| Fallo::new(uso, format!("`{que}` no se deriva")))
}

/// La primera línea de texto de un JSDoc, antes de sus etiquetas.
fn descripcion(doc: &str) -> Option<String> {
    doc.lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .take_while(|l| !l.starts_with('@'))
        .find(|l| !l.is_empty())
        .map(str::to_string)
}
