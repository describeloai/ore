//! **El almacén**: la conexión al catálogo de un inquilino (su esquema) y sus transacciones. Se
//! queda tras el corte: ore-serve escribirá con él (E1a·5).

use std::collections::{BTreeMap, HashMap};

use postgres::{Client, NoTls};

use crate::git::{Cambio, Commit};

/// Una diferencia entre lo que el catálogo ve y lo que se esperaba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diferencia {
    pub rama: String,
    pub activo: String,
    /// `falta` | `sobra` | `origen` (rama frente a main) | `contenido`.
    pub motivo: String,
    /// En `contenido`, los campos de primer nivel que difieren.
    pub campos: String,
}

/// Lo que se espera que vea una ref: activo, de dónde (`rama` | `main`) y el texto del puntero.
pub struct Esperado {
    pub activo: String,
    pub de: &'static str,
    pub texto: String,
}

pub struct Almacen {
    c: Client,
}

/// Las ramas del catálogo: nombre → (punta de la rama, punta de main) con las que se calcularon.
pub type Calculadas = BTreeMap<String, (Option<String>, Option<String>)>;

/// Un nombre de esquema seguro de interpolar: el del inquilino (`t_victor`).
pub fn esquema_valido(e: &str) -> bool {
    !e.is_empty()
        && e.len() <= 63
        && e.starts_with(|c: char| c.is_ascii_lowercase())
        && e.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn pg(e: postgres::Error) -> String {
    match e.as_db_error() {
        Some(d) => format!("{} ({})", d.message(), d.code().code()),
        None => e.to_string(),
    }
}

/// Los textos de unos cambios, en arrays paralelos (activo, blob, texto; `None` = borrado).
type Arrays = (Vec<String>, Vec<Option<String>>, Vec<Option<String>>);

fn en_arrays(cambios: &[Cambio], textos: &HashMap<String, String>) -> Result<Arrays, String> {
    let mut a = (Vec::new(), Vec::new(), Vec::new());
    for c in cambios {
        let texto = match &c.blob {
            Some(b) => Some(textos.get(b).cloned().ok_or(format!("falta el blob {b}"))?),
            None => None,
        };
        a.0.push(c.ruta.clone());
        a.1.push(c.blob.clone());
        a.2.push(texto);
    }
    Ok(a)
}

impl Almacen {
    /// Conecta al esquema del inquilino y le aplica la versión del esquema. El esquema no se crea
    /// aquí: lo crea el alta (`malla/78-el-catalogo-del-inquilino.sh`), con su papel como dueño
    /// y su publicación. Un papel de inquilino no puede crear esquemas: eso sería salirse del suyo.
    pub fn conectar(url: &str, esquema: &str) -> Result<Almacen, String> {
        if !esquema_valido(esquema) {
            return Err(format!("esquema no válido: `{esquema}`"));
        }
        // Sin TLS, como `ore-iam`: de pod a pod dentro del clúster.
        let mut c =
            Client::connect(url, NoTls).map_err(|e| format!("no se pudo conectar: {}", pg(e)))?;
        let hay: bool = c
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
                &[&esquema],
            )
            .map_err(pg)?
            .get(0);
        if !hay {
            return Err(format!(
                "el esquema `{esquema}` no existe: se da de alta con malla/78-el-catalogo-del-inquilino.sh"
            ));
        }
        let suyo: bool = c
            .query_one(
                "SELECT has_schema_privilege($1, 'USAGE, CREATE')",
                &[&esquema],
            )
            .map_err(pg)?
            .get(0);
        if !suyo {
            return Err(format!(
                "este papel no tiene acceso al esquema `{esquema}`: cada inquilino entra sólo en el suyo"
            ));
        }
        c.batch_execute(&format!("SET search_path TO {esquema};"))
            .map_err(pg)?;
        crate::esquema::aplicar(&mut c).map_err(pg)?;
        Ok(Almacen { c })
    }

    /// La punta de git con la que se calculó una ref (`main`: el último commit replicado).
    pub fn seguido(&mut self, rama: &str) -> Result<Option<String>, String> {
        Ok(self
            .c
            .query_opt("SELECT git_sha FROM refs WHERE nombre = $1", &[&rama])
            .map_err(pg)?
            .and_then(|f| f.get(0)))
    }

    pub fn ultimo_commit_main(&mut self) -> Result<i64, String> {
        Ok(self
            .c
            .query_one(
                "SELECT coalesce(max(id), 0) FROM commits WHERE ref = 'main'",
                &[],
            )
            .map_err(pg)?
            .get(0))
    }

    /// Replica un commit de main en una transacción: sus punteros (si movió alguno), su sitio en
    /// el mapa del andamio y la punta seguida. Devuelve el commit del catálogo vigente.
    pub fn replicar_commit_main(
        &mut self,
        c: &Commit,
        textos: &HashMap<String, String>,
        vigente: i64,
    ) -> Result<i64, String> {
        let mut tx = self.c.transaction().map_err(pg)?;
        let id = if c.cambios.is_empty() {
            vigente
        } else {
            let (activos, blobs, txt) = en_arrays(&c.cambios, textos)?;
            tx.query_one(
                "SELECT replicar_main($1, $2, ($3::text)::timestamptz, $4, $5, $6)",
                &[&c.sha, &c.autor, &c.cuando, &activos, &blobs, &txt],
            )
            .map_err(pg)?
            .get(0)
        };
        tx.execute(
            "INSERT INTO andamio_mapa VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&c.sha, &id],
        )
        .map_err(pg)?;
        tx.execute(
            "UPDATE refs SET git_sha = $1 WHERE nombre = 'main'",
            &[&c.sha],
        )
        .map_err(pg)?;
        tx.commit().map_err(pg)?;
        Ok(id)
    }

    /// Los punteros de main: activo → blob del que se copió.
    pub fn punteros_main(&mut self) -> Result<BTreeMap<String, String>, String> {
        Ok(self
            .c
            .query(
                "SELECT activo, coalesce(git_blob, '') FROM estado WHERE ref = 'main'",
                &[],
            )
            .map_err(pg)?
            .iter()
            .map(|f| (f.get(0), f.get(1)))
            .collect())
    }

    /// El commit del catálogo vigente en un commit de primer padre de main.
    pub fn punto_de(&mut self, sha: &str) -> Result<Option<i64>, String> {
        Ok(self
            .c
            .query_opt(
                "SELECT commit_id FROM andamio_mapa WHERE git_sha = $1",
                &[&sha],
            )
            .map_err(pg)?
            .map(|f| f.get(0)))
    }

    /// Las ramas del catálogo (sin main): nombre → (punta, main) con las que se calcularon.
    pub fn ramas(&mut self) -> Result<Calculadas, String> {
        Ok(self
            .c
            .query(
                "SELECT nombre, git_sha, git_main FROM refs WHERE nombre <> 'main'",
                &[],
            )
            .map_err(pg)?
            .iter()
            .map(|f| (f.get(0), (f.get(1), f.get(2))))
            .collect())
    }

    /// Sustituye lo propio de una rama, en una transacción.
    pub fn replicar_rama(
        &mut self,
        nombre: &str,
        punta: &str,
        main: &str,
        punto: i64,
        propios: &[Cambio],
        textos: &HashMap<String, String>,
    ) -> Result<(), String> {
        let (activos, blobs, txt) = en_arrays(propios, textos)?;
        self.c
            .execute(
                "SELECT replicar_rama($1, $2, $3, $4, $5, $6, $7)",
                &[&nombre, &punta, &main, &punto, &activos, &blobs, &txt],
            )
            .map_err(pg)?;
        Ok(())
    }

    pub fn borrar_rama(&mut self, nombre: &str) -> Result<(), String> {
        self.c
            .execute("SELECT borrar_rama($1)", &[&nombre])
            .map_err(pg)?;
        Ok(())
    }

    /// Compara lo que ve una ref con lo esperado: existencia, origen y el puntero campo a campo
    /// (igualdad de `jsonb`: sin depender del orden de las claves ni del formato).
    pub fn auditar_ref(
        &mut self,
        rama: &str,
        esperado: &[Esperado],
    ) -> Result<Vec<Diferencia>, String> {
        let activos: Vec<&str> = esperado.iter().map(|e| e.activo.as_str()).collect();
        let des: Vec<&str> = esperado.iter().map(|e| e.de).collect();
        let textos: Vec<&str> = esperado.iter().map(|e| e.texto.as_str()).collect();
        let filas = self
            .c
            .query(
                "WITH e AS (SELECT * FROM unnest($2::text[], $3::text[], $4::text[]) AS e(activo, de, texto)),
                      v AS (SELECT * FROM vista($1))
                 SELECT activo, motivo, coalesce(campos, '') FROM (
                   SELECT coalesce(e.activo, v.activo) AS activo,
                          CASE WHEN e.activo IS NULL THEN 'sobra'
                               WHEN v.activo IS NULL THEN 'falta'
                               WHEN e.de <> v.de THEN 'origen'
                               WHEN v.puntero IS DISTINCT FROM e.texto::jsonb THEN 'contenido' END AS motivo,
                          CASE WHEN e.activo IS NOT NULL AND v.activo IS NOT NULL THEN
                            (SELECT string_agg(coalesce(x.key, y.key), ',' ORDER BY coalesce(x.key, y.key))
                               FROM jsonb_each(v.puntero) x FULL JOIN jsonb_each(e.texto::jsonb) y ON x.key = y.key
                              WHERE x.value IS DISTINCT FROM y.value) END AS campos
                     FROM e FULL JOIN v ON v.activo = e.activo) d
                 WHERE motivo IS NOT NULL ORDER BY activo",
                &[&rama, &activos, &des, &textos],
            )
            .map_err(pg)?;
        Ok(filas
            .iter()
            .map(|f| Diferencia {
                rama: rama.to_string(),
                activo: f.get(0),
                motivo: f.get(1),
                campos: f.get(2),
            })
            .collect())
    }

    /// Cuántos commits, punteros vivos en main, filas de historia y eventos hay.
    pub fn cuentas(&mut self) -> Result<[i64; 4], String> {
        let f = self
            .c
            .query_one(
                "SELECT (SELECT count(*) FROM commits), (SELECT count(*) FROM estado WHERE ref = 'main'),
                        (SELECT count(*) FROM historia), (SELECT count(*) FROM outbox)",
                &[],
            )
            .map_err(pg)?;
        Ok([f.get(0), f.get(1), f.get(2), f.get(3)])
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_esquema_del_inquilino() {
        assert!(esquema_valido("t_victor"));
        assert!(esquema_valido("t_demo2"));
        assert!(!esquema_valido(""));
        assert!(!esquema_valido("T_victor"));
        assert!(!esquema_valido("t-victor"));
        assert!(!esquema_valido("t_victor; DROP SCHEMA x"));
        assert!(!esquema_valido("1t"));
    }

    #[test]
    fn los_cambios_en_arrays() {
        let cs = vec![
            Cambio {
                ruta: "datasets/a.json".into(),
                blob: Some("b1".into()),
            },
            Cambio {
                ruta: "datasets/b.json".into(),
                blob: None,
            },
        ];
        let textos = HashMap::from([("b1".to_string(), "{}".to_string())]);
        let (a, b, t) = en_arrays(&cs, &textos).unwrap();
        assert_eq!(a, vec!["datasets/a.json", "datasets/b.json"]);
        assert_eq!(b, vec![Some("b1".to_string()), None]);
        assert_eq!(t, vec![Some("{}".to_string()), None]);
        assert!(en_arrays(&cs[..1], &HashMap::new()).is_err());
    }
}
