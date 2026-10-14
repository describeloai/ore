//! **Aceptar el esquema nuevo del origen** (ADR 0063 S5c).
//!
//! La copia ve que el origen ya no tiene la forma declarada (S5a:
//! `source-schema-changed`, con `columns`). Aceptarla no es tocar un fichero:
//! la `Table` de la fuente la escribe el catálogo, el `Dataset` de cada base
//! que la copia nombra sus columnas, y lo que va detrás —vistas, transforms—
//! puede dejar de compilar. Por eso se hace **en una rama y como propuesta**
//! (0044): se ve qué se rompería antes de que entre en `main`.
//!
//! `POST /datasets/{b}/{s}/{n}/esquema/aceptar`, sólo una persona:
//!
//! 1. la `Table` raíz del dataset y su fuente, en `main`;
//! 2. una rama `<persona>/esquema-<b>-<s>-<n>-<hora>`;
//! 3. en ella, el catálogo de la fuente otra vez (`ore source catalog`, por la
//!    pasarela, con la credencial del custodio): regenera la `Table`;
//! 4. el `fields` de cada `Dataset` que se copia de esa tabla, al día: si es un
//!    espejo (`c: c`), las columnas de ahora; si no, sin las que se fueron;
//! 5. los diagnósticos que esto **añade** al árbol: lo que se rompería;
//! 6. la propuesta, con todo eso dicho.

use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::Path;
use std::process::Command;

/// Lo que se le pide al catálogo y lo que se escribe en la rama.
struct Raiz {
    /// `fuente.schema.tabla`, como la nombra `from.table`.
    tabla: String,
    fuente: String,
    env: String,
}

impl Servidor {
    pub(crate) fn aceptar_esquema(
        &self,
        sujeto: &Identidad,
        b: &str,
        s: &str,
        n: &str,
    ) -> Respuesta {
        let qn = format!("{b}.{s}.{n}");
        // ① La tabla raíz, en `main`.
        let mut hallado = Err(format!("`{qn}` no está en el árbol"));
        let _ = self.leyendo(|raiz| {
            hallado = raiz_de(raiz, &qn);
            Respuesta::ok(Json::obj([]))
        });
        let r = match hallado {
            Ok(r) => r,
            Err(m) => return Respuesta::error(404, m),
        };
        let url = match self.credencial_de_la_fuente(&r.fuente, &r.env) {
            Ok((u, _)) => u,
            Err(e) => return e,
        };
        // ② La rama.
        let nombre = format!(
            "esquema-{}-{}",
            qn.replace(['.', '_'], "-"),
            crate::funciones::corrida_ahora().to_lowercase()
        );
        let creada = self.crear_rama(sujeto, &Json::obj([("nombre", Json::s(&nombre))]).jcs());
        if creada.codigo >= 300 {
            return creada;
        }
        let Some(rama) = texto(&creada.cuerpo, "rama") else {
            return Respuesta::error(502, "la rama se creó y no dijo su nombre");
        };
        // ③–⑤ En la rama.
        let mut resumen = Json::obj([]);
        let escrito = self.escribiendo_en(
            Some(&rama),
            sujeto,
            &format!("Aceptar el esquema nuevo de `{}` (0063 S5c)", r.tabla),
            |raiz| match self.aceptar_en(raiz, &r, &url) {
                Ok(j) => {
                    resumen = j.clone();
                    Respuesta::ok(j)
                }
                Err(e) => e,
            },
        );
        if escrito.codigo >= 300 {
            let _ = self.retirar_rama(&rama);
            return escrito;
        }
        if texto(&escrito.cuerpo, "commit").is_none() {
            let _ = self.retirar_rama(&rama);
            return Respuesta::error(
                409,
                format!(
                    "the source schema of `{}` already matches the declared table: nothing to accept",
                    r.tabla
                ),
            );
        }
        // ⑥ La propuesta.
        let propuesta = self.proponer(
            sujeto,
            &Json::obj([
                ("rama", Json::s(&rama)),
                (
                    "titulo",
                    Json::s(format!("Accept the new source schema of `{}`", r.tabla)),
                ),
                ("descripcion", Json::s(descripcion(&resumen))),
            ])
            .jcs(),
        );
        if propuesta.codigo >= 300 {
            // La rama queda: lo escrito en ella se puede proponer a mano.
            return propuesta;
        }
        Respuesta::creado(Json::obj([
            ("rama", Json::s(&rama)),
            ("propuesta", propuesta.cuerpo),
            ("resumen", resumen),
        ]))
    }

    /// Los pasos ③–⑤, en el clon de la rama.
    fn aceptar_en(&self, raiz: &Path, r: &Raiz, url: &str) -> Result<Json, Respuesta> {
        let clave = |d: &Json| {
            format!(
                "{}\u{1}{}",
                texto(d, "codigo").unwrap_or_default(),
                texto(d, "mensaje").unwrap_or_default()
            )
        };
        let antes: std::collections::BTreeSet<String> =
            self.diagnosticos_de(raiz)?.iter().map(clave).collect();
        let columnas_antes = columnas_de(raiz, &r.tabla).unwrap_or_default();

        // ③ El catálogo de la fuente, otra vez: regenera sus `Table`.
        let salida = Command::new(&self.binario)
            .args([
                "source",
                "catalog",
                &r.fuente,
                "--out",
                &format!("packages/{}/discover.catalog.json", r.fuente),
                "--path",
            ])
            .arg(raiz)
            .current_dir(raiz)
            .env(&r.env, url)
            .env("ORE_PASARELA", crate::federado::pasarela())
            .output()
            .map_err(|e| Respuesta::error(500, format!("no arranca `ore`: {e}")))?;
        if !salida.status.success() {
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&salida.stdout),
                String::from_utf8_lossy(&salida.stderr)
            );
            let cola: Vec<&str> = log.lines().collect();
            return Err(Respuesta::error(
                502,
                format!(
                    "the source catalog of `{}` failed: {}",
                    r.fuente,
                    crate::por_la_pasarela::tapar(
                        &cola[cola.len().saturating_sub(5)..].join(" · "),
                        url
                    )
                ),
            ));
        }
        let Some(columnas) = columnas_de(raiz, &r.tabla) else {
            return Err(Respuesta::error(
                409,
                format!(
                    "`{}` is no longer in its source: there is no schema to accept",
                    r.tabla
                ),
            ));
        };
        let nombres =
            |v: &[String]| -> std::collections::BTreeSet<String> { v.iter().cloned().collect() };
        let (a, d) = (nombres(&columnas_antes), nombres(&columnas));
        let added: Vec<String> = d.difference(&a).cloned().collect();
        let removed: Vec<String> = a.difference(&d).cloned().collect();

        // ④ Los `Dataset` que se copian de ella.
        let mut datasets = Vec::new();
        for (ruta, qn) in datasets_de(raiz, &r.tabla) {
            if al_dia(&ruta, &columnas, &removed)? {
                datasets.push(Json::s(qn));
            }
        }

        // ⑤ Lo que se rompería: los diagnósticos que esto añade.
        let despues = self.diagnosticos_de(raiz)?;
        let rompe: Vec<Json> = despues
            .into_iter()
            .filter(|x| !antes.contains(&clave(x)))
            .collect();
        let l = |v: Vec<String>| Json::Arr(v.into_iter().map(Json::s).collect());
        Ok(Json::obj([
            ("tabla", Json::s(&r.tabla)),
            ("added", l(added)),
            ("removed", l(removed)),
            ("datasets", Json::Arr(datasets)),
            ("rompe", Json::Arr(rompe)),
        ]))
    }
}

fn texto(j: &Json, k: &str) -> Option<String> {
    match j {
        Json::Obj(m) => match m.get(k) {
            Some(Json::Str(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// La `Table` de la que sale el dataset `qn`, y su fuente.
fn raiz_de(raiz: &Path, qn: &str) -> Result<Raiz, String> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let d = pkg
        .docs
        .iter()
        .find(|d| d.kind == ore_core::document::Kind::Dataset && d.qname().as_deref() == Some(qn))
        .ok_or_else(|| format!("`{qn}` no es un dataset del árbol"))?;
    let tabla = d
        .section("from")
        .and_then(|f| f.get("table"))
        .and_then(|(_, t)| t.as_str())
        .map(|t| ore_core::link::cualificar(t, d))
        .ok_or_else(|| {
            format!("`{qn}` no se copia de una `Table`: no hay esquema de origen que aceptar")
        })?;
    let t = pkg
        .docs
        .iter()
        .find(|x| {
            x.kind == ore_core::document::Kind::Table
                && x.qname().as_deref() == Some(tabla.as_str())
        })
        .ok_or_else(|| format!("la `Table` `{tabla}` no está en el árbol"))?;
    let fuente = t
        .section("datasource")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("`{tabla}` no dice su `datasource`"))?
        .to_string();
    let (_, env) = crate::por_la_pasarela::declaracion(raiz, &fuente)?;
    Ok(Raiz { tabla, fuente, env })
}

/// Las columnas que declara la `Table` `tabla`, en su orden. `None`: no está.
fn columnas_de(raiz: &Path, tabla: &str) -> Option<Vec<String>> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let t = pkg.docs.iter().find(|x| {
        x.kind == ore_core::document::Kind::Table && x.qname().as_deref() == Some(tabla)
    })?;
    Some(
        t.section("columns")
            .map(|c| {
                c.entries()
                    .iter()
                    .filter_map(|(k, _)| k.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// Los `Dataset` que se copian de `tabla`: su fichero y su nombre.
fn datasets_de(raiz: &Path, tabla: &str) -> Vec<(std::path::PathBuf, String)> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    pkg.docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Dataset)
        .filter(|d| {
            d.section("from")
                .and_then(|f| f.get("table"))
                .and_then(|(_, t)| t.as_str())
                .is_some_and(|t| ore_core::link::cualificar(t, d) == tabla)
        })
        .filter_map(|d| Some((d.path.clone(), d.qname()?)))
        .collect()
}

/// **El `fields` de un dataset, al día con las columnas de ahora.** Un espejo
/// (cada campo se llama como su columna, que es lo que induce el alta de una
/// base) pasa a ser el espejo de las columnas nuevas. Uno con nombres propios
/// sólo pierde los campos de columnas que ya no están: lo nuevo no se inventa
/// nombre. Devuelve si cambió. Sólo se toca el bloque `fields`, y sólo en un
/// fichero que escribió la inducción.
fn al_dia(ruta: &Path, columnas: &[String], quitadas: &[String]) -> Result<bool, Respuesta> {
    let texto = std::fs::read_to_string(ruta)
        .map_err(|e| Respuesta::error(500, format!("no se pudo leer `{}`: {e}", ruta.display())))?;
    let Some(nuevo) = fields_al_dia(&texto, columnas, quitadas) else {
        return Ok(false);
    };
    std::fs::write(ruta, nuevo).map_err(|e| {
        Respuesta::error(
            500,
            format!("no se pudo escribir `{}`: {e}", ruta.display()),
        )
    })?;
    Ok(true)
}

fn fields_al_dia(texto: &str, columnas: &[String], quitadas: &[String]) -> Option<String> {
    if !texto.starts_with("# ore discover:") {
        return None;
    }
    let lineas: Vec<&str> = texto.lines().collect();
    let i = lineas.iter().position(|l| *l == "  fields:")?;
    let fin = lineas[i + 1..]
        .iter()
        .position(|l| !l.starts_with("    "))
        .map_or(lineas.len(), |k| i + 1 + k);
    let pares: Vec<(String, String)> = lineas[i + 1..fin]
        .iter()
        .filter_map(|l| {
            let (k, v) = l.trim().split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    let espejo = !pares.is_empty() && pares.iter().all(|(k, v)| k == v);
    let nuevos: Vec<(String, String)> = if espejo {
        columnas.iter().map(|c| (c.clone(), c.clone())).collect()
    } else {
        pares
            .iter()
            .filter(|(_, v)| !quitadas.contains(v))
            .cloned()
            .collect()
    };
    if nuevos == pares {
        return None;
    }
    let mut salida: Vec<String> = lineas[..=i].iter().map(|l| l.to_string()).collect();
    salida.extend(nuevos.iter().map(|(k, v)| format!("    {k}: {v}")));
    salida.extend(lineas[fin..].iter().map(|l| l.to_string()));
    Some(salida.join("\n") + "\n")
}

/// Lo que la propuesta dice, para quien la revisa.
fn descripcion(resumen: &Json) -> String {
    let lista = |k: &str| -> Vec<String> {
        match resumen {
            Json::Obj(m) => match m.get(k) {
                Some(Json::Arr(v)) => v
                    .iter()
                    .filter_map(|x| match x {
                        Json::Str(s) => Some(format!("`{s}`")),
                        Json::Obj(_) => Some(format!(
                            "`{}` {}",
                            texto(x, "codigo").unwrap_or_default(),
                            texto(x, "mensaje").unwrap_or_default()
                        )),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    };
    let o = |v: Vec<String>| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.join(", ")
        }
    };
    let rompe = lista("rompe");
    format!(
        "The source of `{}` changed shape (ADR 0063 S5c). This branch re-reads its catalog and brings the copies up to date.\n\n\
         - Columns added: {}\n- Columns removed: {}\n- Datasets updated: {}\n\n{}",
        texto(resumen, "tabla").unwrap_or_default(),
        o(lista("added")),
        o(lista("removed")),
        o(lista("datasets")),
        if rompe.is_empty() {
            "Nothing else in the tree stops compiling.".to_string()
        } else {
            format!(
                "**What this would break ({}):**\n\n{}",
                rompe.len(),
                rompe
                    .iter()
                    .take(20)
                    .map(|r| format!("- {r}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    )
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const DATASET: &str = "# ore discover: lo escribe la inducción y lo regenera en cada pasada. Lo que no lleva esta línea, no lo toca.
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata:
  name: pedidos
spec:
  owner: \"user:victor\"
  from: { table: pg_quality.ore_vara.pedidos }
  fields:
    id: id
    cliente: cliente
    total: total
";

    #[test]
    fn un_espejo_pasa_a_las_columnas_de_ahora() {
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let nuevo =
            fields_al_dia(DATASET, &c(&["id", "comprador", "total"]), &c(&["cliente"])).unwrap();
        assert!(
            nuevo.contains("    comprador: comprador\n") && !nuevo.contains("cliente"),
            "{nuevo}"
        );
        assert!(
            nuevo.contains("  from: { table: pg_quality.ore_vara.pedidos }"),
            "lo demás, igual"
        );
        // Ya al día: no se toca.
        assert_eq!(
            fields_al_dia(&nuevo, &c(&["id", "comprador", "total"]), &[]),
            None
        );
    }

    /// 0063 S7a·3: aceptar el esquema reescribe `fields`, no las promesas. La
    /// que mide una columna que se fue queda, y la propuesta la enseña como
    /// `OOS2052` (paso ⑤): no se borra en silencio lo que alguien prometió.
    #[test]
    fn aceptar_el_esquema_no_toca_las_promesas() {
        let t = format!(
            "{}  promises:\n    - id: cliente_presente\n      column: cliente\n      metric: nullValues\n      mustBe: 0\n",
            DATASET.replace("v1alpha13", "v1alpha31")
        );
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let nuevo = fields_al_dia(&t, &c(&["id", "comprador", "total"]), &c(&["cliente"])).unwrap();
        assert!(
            nuevo.ends_with("  promises:\n    - id: cliente_presente\n      column: cliente\n      metric: nullValues\n      mustBe: 0\n"),
            "{nuevo}"
        );
        let d = ore_core::validate::validate_document(std::path::Path::new("x.yaml"), &nuevo);
        assert_eq!(
            d.iter().map(|x| x.code).collect::<Vec<_>>(),
            vec![ore_core::code::Code::Oos2052],
            "{d:?}"
        );
    }

    #[test]
    fn con_nombres_propios_solo_se_quita_lo_que_se_fue() {
        let t = DATASET.replace("    cliente: cliente", "    quien: cliente");
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let nuevo = fields_al_dia(&t, &c(&["id", "comprador", "total"]), &c(&["cliente"])).unwrap();
        assert!(
            !nuevo.contains("quien") && !nuevo.contains("comprador"),
            "{nuevo}"
        );
        // Lo que no escribió la inducción, no se toca.
        assert_eq!(
            fields_al_dia(
                &t.replacen("# ore discover:", "# a mano:", 1),
                &c(&["id"]),
                &c(&["cliente"])
            ),
            None
        );
    }
}
