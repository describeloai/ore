//! ORE 0068 C2·3 2b·0 · La sonda de la reescritura por spans: cada nombre del árbol que una sentencia
//! LEE (un `TableFactor::Table` de 2 o 3 partes que no es una CTE: la regla de `sql_del_arbol`) se
//! sustituye en el TEXTO por su vista temporal de Spark, `ore__<base>__<schema>__<nombre>`, sin
//! reimprimir nada más. Una línea por caso: `OK|FALLA · caso` y, debajo, lo que sale.
use sqlparser::ast::{ObjectName, ObjectNamePart, Query, Spanned, TableFactor, Visit, Visitor};
use sqlparser::dialect::DuckDbDialect;
use sqlparser::parser::Parser;
use std::ops::ControlFlow;

struct Nombres {
    ctes: Vec<Vec<String>>,
    // (desde, hasta) en bytes, y la vista
    trozos: Vec<(usize, usize, String)>,
    texto_lineas: Vec<usize>, // el byte donde empieza cada línea
}

impl Nombres {
    fn byte(&self, l: sqlparser::tokenizer::Location) -> usize {
        // línea y columna en base 1; la columna cuenta caracteres
        let ini = self.texto_lineas[(l.line - 1) as usize];
        ini + (l.column as usize - 1)
    }
}

fn idents(n: &ObjectName) -> Vec<&sqlparser::ast::Ident> {
    n.0.iter()
        .filter_map(|p| match p {
            ObjectNamePart::Identifier(i) => Some(i),
            _ => None,
        })
        .collect()
}

impl Visitor for Nombres {
    type Break = ();
    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        let n = q
            .with
            .as_ref()
            .map(|w| w.cte_tables.iter().map(|c| c.alias.name.value.clone()).collect())
            .unwrap_or_default();
        self.ctes.push(n);
        ControlFlow::Continue(())
    }
    fn post_visit_query(&mut self, _q: &Query) -> ControlFlow<()> {
        self.ctes.pop();
        ControlFlow::Continue(())
    }
    fn pre_visit_table_factor(&mut self, t: &TableFactor) -> ControlFlow<()> {
        if let TableFactor::Table { name, args: None, .. } = t {
            let p = idents(name);
            if p.len() == 1 && self.ctes.iter().any(|s| s.contains(&p[0].value)) {
                return ControlFlow::Continue(());
            }
            let (base, schema, nombre) = match p.as_slice() {
                [b, s, n] => (b.value.clone(), s.value.clone(), n.value.clone()),
                [b, n] => (b.value.clone(), "default".to_string(), n.value.clone()),
                _ => return ControlFlow::Continue(()),
            };
            // del principio de la primera parte al final de la última (con sus comillas)
            let ini = self.byte(p[0].span.start);
            let fin = self.byte(p[p.len() - 1].span.end);
            self.trozos.push((ini, fin, format!("ore__{base}__{schema}__{nombre}")));
        }
        ControlFlow::Continue(())
    }
}

fn reescribir(texto: &str) -> Result<(String, Vec<String>), String> {
    let s = Parser::parse_sql(&DuckDbDialect {}, texto).map_err(|e| e.to_string())?;
    // Las columnas de sqlparser cuentan caracteres: el texto, en caracteres, para cortar bien.
    let chars: Vec<char> = texto.chars().collect();
    let mut lineas = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            lineas.push(i + 1);
        }
    }
    let mut v = Nombres { ctes: vec![], trozos: vec![], texto_lineas: lineas };
    let _ = s.visit(&mut v);
    let mut t = v.trozos.clone();
    t.sort();
    let mut out: Vec<char> = Vec::new();
    let mut i = 0;
    let mut vistas = vec![];
    for (a, b, vista) in t {
        out.extend(&chars[i..a]);
        out.extend(vista.chars());
        vistas.push(format!("{} ← {}", vista, chars[a..b].iter().collect::<String>()));
        i = b;
    }
    out.extend(&chars[i..]);
    let _ = s.first().map(|x| x.span());
    Ok((out.into_iter().collect(), vistas))
}

fn main() {
    let casos: &[(&str, &str, &[&str])] = &[
        ("dos partes y un join", "select z.id, count(*) n from geo.zonas z join geo.lugares l on ST_Contains(z.zona, l.geom) group by z.id",
         &["from ore__geo__default__zonas z join ore__geo__default__lugares l"]),
        ("tres partes", "select * from geo.default.lugares", &["from ore__geo__default__lugares"]),
        ("el mismo nombre dos veces", "select a.id from geo.lugares a join geo.lugares b on a.id = b.id + 1",
         &["from ore__geo__default__lugares a join ore__geo__default__lugares b"]),
        ("entre comillas", "select * from \"geo\".\"lugares\"", &["from ore__geo__default__lugares"]),
        ("una CTE con el nombre de una tabla (una parte) no es del árbol",
         "with lugares as (select * from geo.lugares where id > 1) select * from lugares",
         &["(select * from ore__geo__default__lugares where id > 1) select * from lugares"]),
        ("en una subconsulta y en un IN", "select * from geo.zonas where id in (select id from geo.lugares)",
         &["from ore__geo__default__zonas where id in (select id from ore__geo__default__lugares)"]),
        ("un nombre en una cadena o un comentario no se toca",
         "select 'geo.lugares' as t -- geo.lugares\nfrom geo.lugares",
         &["select 'geo.lugares' as t -- geo.lugares\nfrom ore__geo__default__lugares"]),
        ("una columna cualificada con la tabla no es una lectura",
         "select geo.lugares.id from geo.lugares", &["select geo.lugares.id from ore__geo__default__lugares"]),
        ("con acentos antes (columnas en caracteres)", "select 'año' a, * from geo.lugares", &["from ore__geo__default__lugares"]),
        ("varias líneas", "select *\n  from\n    geo.zonas z\n  join geo.lugares l on true",
         &["    ore__geo__default__zonas z\n  join ore__geo__default__lugares l"]),
    ];
    let mut bien = 0;
    for (que, sql, espera) in casos {
        match reescribir(sql) {
            Ok((t, vistas)) => {
                let ok = espera.iter().all(|e| t.contains(e));
                bien += ok as usize;
                println!("{} · {que}\n    {}\n    {:?}", if ok { "OK   " } else { "FALLA" }, t.replace('\n', "⏎"), vistas);
            }
            Err(e) => println!("FALLA · {que} · no se analiza: {e}"),
        }
    }
    println!("\n{bien}/{} casos", casos.len());
}
