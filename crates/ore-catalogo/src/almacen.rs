//! **El almacén**: la conexión al catálogo de un inquilino (su esquema) y sus transacciones. Se
//! queda tras el corte: ore-serve escribirá con él (E1a·5).

use std::collections::{BTreeMap, HashMap};

use ore_core::json::Json;
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

// ── La puerta (E1a·5c·1): escribir y leer por ruta, como un directorio de punteros ─────────────

/// Lo que quien escribe cree que hay en la ruta (el *compare-and-set*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EsperadoMl {
    Cualquiera,
    /// Que no haya puntero.
    Nada,
    /// Que el de hoy apunte a esta `metadata_location`.
    Ml(String),
}

#[derive(Debug, Clone)]
pub enum QueRuta {
    Retirar,
    /// El puntero entero (un objeto).
    Sustituir(Json),
    /// Sobre lo que había: quita `quitar` y pone `campos` encima.
    Poner {
        campos: BTreeMap<String, Json>,
        quitar: Vec<String>,
    },
}

/// Un cambio: sus rutas (la de su sitio primero; luego la de antes), lo esperado y qué se hace.
#[derive(Debug, Clone)]
pub struct CambioRuta {
    pub rutas: Vec<String>,
    pub esperado: EsperadoMl,
    pub que: QueRuta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalloEscritura {
    /// El cambio `indice` no cumplió lo esperado; `hay` es la `metadata_location` de hoy.
    Conflicto {
        indice: usize,
        hay: Option<String>,
    },
    /// La rama no está en el catálogo.
    SinRama(String),
    /// El cambio `indice` toca una ruta que una fusión en curso tiene cerrada en esta ref
    /// (`OR429`, v3): no se escribió nada; se reintenta cuando la fusión acabe o su cerrojo caduque.
    /// ore-core no tiene un 429 (`punteros::Fallo`): la puerta lo da como `Almacen` (503, que
    /// también se reintenta).
    Cerrado {
        indice: usize,
    },
    Otro(String),
}

/// Una versión de un puntero, de [`Almacen::historia`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRuta {
    /// El sha de git de lo replicado; `c<n>` lo escrito en el catálogo.
    pub id: String,
    /// RFC 3339, en UTC.
    pub cuando: String,
    pub quien: Option<String>,
    pub texto: String,
}

/// Lo leído en una ruta: `(de, texto)`, con `de` = `aqui` (lo de la ref) o `main` (heredado).
pub type Efectivo = (String, String);

fn cambio_en_json(c: &CambioRuta) -> Json {
    let esperado = match &c.esperado {
        EsperadoMl::Cualquiera => Json::Crudo("null".into()),
        EsperadoMl::Nada => Json::s("nada"),
        EsperadoMl::Ml(m) => Json::obj([("ml", Json::s(m))]),
    };
    let que = match &c.que {
        QueRuta::Retirar => Json::s("retirar"),
        QueRuta::Sustituir(p) => Json::obj([("sustituir", p.clone())]),
        QueRuta::Poner { campos, quitar } => Json::obj([(
            "poner",
            Json::obj([
                ("campos", Json::Obj(campos.clone())),
                ("quitar", Json::Arr(quitar.iter().map(Json::s).collect())),
            ]),
        )]),
    };
    Json::obj([
        ("rutas", Json::Arr(c.rutas.iter().map(Json::s).collect())),
        ("esperado", esperado),
        ("que", que),
    ])
}

/// Un campo de texto del `DETAIL` (un objeto JSON) de un error del catálogo; `null` es `None`.
fn del_detalle(detalle: Option<&str>, k: &str) -> Option<String> {
    let n = detalle.and_then(|d| ore_core::parse::parse(d).ok())?;
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|v| *v != "null")
        .map(String::from)
}

/// El `DETAIL` de un `OR409`: `{"indice": n, "hay": s|null}`.
fn conflicto_de(detalle: Option<&str>) -> FalloEscritura {
    let indice = del_detalle(detalle, "indice")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let hay = del_detalle(detalle, "hay");
    FalloEscritura::Conflicto { indice, hay }
}

/// Un fallo de `escribir` (y de `aplicar_definicion`), por su SQLSTATE.
fn fallo_de_escribir(r: &str, e: postgres::Error) -> FalloEscritura {
    match e.as_db_error() {
        Some(d) if d.code().code() == "OR409" => conflicto_de(d.detail()),
        Some(d) if d.code().code() == "OR404" => FalloEscritura::SinRama(r.to_string()),
        Some(d) if d.code().code() == "OR429" => FalloEscritura::Cerrado {
            indice: del_detalle(d.detail(), "indice")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        },
        _ => FalloEscritura::Otro(pg(e)),
    }
}

impl Almacen {
    /// **Escribir**, todos los cambios o ninguno, en una transacción (`escribir` de la v2). La
    /// misma `idempotencia` dos veces devuelve el commit de la primera sin tocar nada.
    pub fn escribir(
        &mut self,
        r: &str,
        sujeto: &str,
        operacion: &str,
        idempotencia: Option<&str>,
        cambios: &[CambioRuta],
    ) -> Result<i64, FalloEscritura> {
        let j = Json::Arr(cambios.iter().map(cambio_en_json).collect()).pretty();
        self.c
            .query_one(
                "SELECT escribir($1, $2, $3, $4, $5::text::jsonb)",
                &[&r, &sujeto, &operacion, &idempotencia, &j],
            )
            .map(|f| f.get(0))
            .map_err(|e| fallo_de_escribir(r, e))
    }

    /// Lo que la ref ve en la primera de `rutas` que tenga algo (la de su sitio, luego la de antes).
    pub fn efectivo(&mut self, r: &str, rutas: &[String]) -> Result<Option<Efectivo>, String> {
        let f = self
            .c
            .query_opt(
                "SELECT f.de, f.puntero::text FROM unnest($2::text[]) WITH ORDINALITY u(a, o),
                        LATERAL efectivo($1, u.a) f ORDER BY u.o LIMIT 1",
                &[&r, &rutas],
            )
            .map_err(pg)?;
        Ok(f.map(|f| (f.get(0), f.get(1))))
    }

    /// Todo lo que la ref ve: `(ruta, de, texto)`, con `de` = `aqui` | `main`. Una rama que no
    /// está no ve nada, como en [`Almacen::efectivo`] (`vista` le daría lo de main).
    pub fn vista_entera(&mut self, r: &str) -> Result<Vec<(String, String, String)>, String> {
        Ok(self
            .c
            .query(
                "SELECT activo, CASE WHEN $1 = 'main' OR de = 'rama' THEN 'aqui' ELSE 'main' END,
                        puntero::text
                   FROM vista($1) WHERE puntero IS NOT NULL
                    AND ($1 = 'main' OR EXISTS (SELECT 1 FROM refs WHERE nombre = $1))
                  ORDER BY activo",
                &[&r],
            )
            .map_err(pg)?
            .into_iter()
            .map(|f| (f.get(0), f.get(1), f.get(2)))
            .collect())
    }

    /// Las versiones de `rutas` que ve la ref, de la más nueva a la más vieja, hasta `hasta`. Una
    /// rama que no está no tiene historia.
    pub fn historia(
        &mut self,
        r: &str,
        rutas: &[String],
        hasta: usize,
    ) -> Result<Vec<VersionRuta>, String> {
        let hasta = i32::try_from(hasta).unwrap_or(i32::MAX);
        Ok(self
            .c
            .query(
                "SELECT id, cuando, quien, puntero::text FROM historia_de($1, $2::text[], $3)
                  WHERE $1 = 'main' OR EXISTS (SELECT 1 FROM refs WHERE nombre = $1)",
                &[&r, &rutas, &hasta],
            )
            .map_err(pg)?
            .into_iter()
            .map(|f| {
                let quien: Option<String> = f.get(2);
                VersionRuta {
                    id: f.get(0),
                    cuando: f.get(1),
                    // El andamio guardó `nombre <correo>`; git log da el nombre.
                    quien: quien.map(|q| match q.split_once(" <") {
                        Some((n, _)) => n.to_string(),
                        None => q,
                    }),
                    texto: f.get(3),
                }
            })
            .collect())
    }

    /// La versión de lo que ve la ref: cambia si y solo si cambia algo suyo o de `main`.
    pub fn version(&mut self, r: &str) -> Result<String, String> {
        Ok(self
            .c
            .query_one("SELECT version_de($1)", &[&r])
            .map_err(pg)?
            .get(0))
    }

    /// Nace `rama` desde `desde` (`main` de hoy, o lo propio de otra rama). Si ya está, nada.
    pub fn crear_rama(&mut self, rama: &str, desde: &str) -> Result<(), String> {
        self.c
            .execute("SELECT crear_rama($1, $2)", &[&rama, &desde])
            .map_err(pg)?;
        Ok(())
    }

    /// `rama` trae `main`: su punto de salida pasa a `main` de hoy.
    pub fn traer_main(&mut self, rama: &str) -> Result<(), String> {
        self.c
            .execute("SELECT traer_main($1)", &[&rama])
            .map_err(pg)?;
        Ok(())
    }

    /// ¿Está ya la siembra `marca` (un commit con ese `git_sha`)?
    pub fn sembrado(&mut self, marca: &str) -> Result<bool, String> {
        Ok(self
            .c
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM commits WHERE git_sha = $1)",
                &[&marca],
            )
            .map_err(pg)?
            .get(0))
    }

    /// **Siembra** `main` con `(rutas, textos)` en un commit marcado con `git_sha = marca`, si no
    /// está ya: `replicar_main` (idempotente por el sha) bajo un cerrojo de la marca. Devuelve si
    /// sembró. Ver `Catalogo::sembrar_si_hace_falta`.
    pub fn sembrar(
        &mut self,
        marca: &str,
        rutas: &[String],
        textos: &[String],
    ) -> Result<bool, String> {
        let mut tx = self.c.transaction().map_err(pg)?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext('sembrar|' || $1))",
            &[&marca],
        )
        .map_err(pg)?;
        let hay: bool = tx
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM commits WHERE git_sha = $1)",
                &[&marca],
            )
            .map_err(pg)?
            .get(0);
        if hay {
            return Ok(false);
        }
        let blobs: Vec<Option<String>> = vec![None; rutas.len()];
        let textos: Vec<Option<String>> = textos.iter().cloned().map(Some).collect();
        tx.query_one(
            "SELECT replicar_main($1, 'ore-catalogo sembrar', now(), $2, $3, $4)",
            &[&marca, &rutas, &blobs, &textos],
        )
        .map_err(pg)?;
        tx.commit().map_err(pg)?;
        Ok(true)
    }

    /// Lo vivo de todas las refs: `(ref, ruta, texto)`. Lo que la recogida no puede borrar.
    pub fn vivos(&mut self) -> Result<Vec<(String, String, String)>, String> {
        Ok(self
            .c
            .query(
                "SELECT ref, activo, puntero::text FROM estado
                  WHERE NOT borrado AND puntero IS NOT NULL ORDER BY ref, activo",
                &[],
            )
            .map_err(pg)?
            .into_iter()
            .map(|f| (f.get(0), f.get(1), f.get(2)))
            .collect())
    }
}

// ── Las escrituras de definición (E1a·5c·5c·1): cerrojos, plan de fusión y su aplicación ───────

/// Por qué no se pudo [`Almacen::cerrar`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalloCerrar {
    /// Otro dueño vivo tiene cerrada `ruta` en `en` (`OR429`): no se cerró nada.
    Ocupado {
        en: String,
        ruta: String,
        dueno: String,
    },
    Otro(String),
}

/// Un paso del plan de fusión: lo que pasa con una ruta. Ver [`Almacen::plan_fusion`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasoFusion {
    pub ruta: String,
    /// `promocion` | `reconstruir` | `conflicto` (los de `git::Dato` de ore-serve).
    pub caso: String,
    /// `rama` | `main` | `sin elegir`: qué puntero queda en `main`.
    pub resultado: String,
    /// El puntero de la rama que se vio (`None`: no hay, o una lápida).
    pub rama: Option<String>,
    /// El de `main` que se vio.
    pub main: Option<String>,
    /// Si la rama tenía fila propia (lápida incluida).
    pub propio: bool,
}

/// El plan de fusión de una rama en `main`.
#[derive(Debug, Clone)]
pub struct PlanFusion {
    /// Algún conflicto sin elegir: no se aplica (el 409 de hoy).
    pub sin_elegir: bool,
    pub pasos: Vec<PasoFusion>,
    /// El plan tal cual (`jsonb`): lo que viaja en el commit de git y recibe
    /// [`Almacen::aplicar_plan`].
    pub texto: String,
}

/// Un paso que no entró en una aplicación tardía: `main` se movió después del plan y gana.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pendiente {
    pub ruta: String,
    /// Lo de `main` que se queda.
    pub se_queda: Option<String>,
    /// Lo de la rama que no entra.
    pub no_entra: Option<String>,
}

/// Lo que hizo [`Almacen::aplicar_plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aplicada {
    /// El commit del catálogo (`idempotencia = git:<sha>`).
    pub commit: i64,
    /// Ya estaba aplicado: no se tocó nada y esto es lo de la primera vez.
    pub repetida: bool,
    pub tardia: bool,
    /// Las rutas que cambiaron en `main`.
    pub aplicados: Vec<String>,
    pub pendientes: Vec<Pendiente>,
}

/// Por qué no se aplicó un plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalloAplicar {
    /// `OR422`: el plan tiene algo sin elegir o es de otra rama.
    NoSeAplica(String),
    /// `OR500`: `main` o la rama se movieron con el cerrojo puesto (sin `tardia`). Nada se aplicó.
    Protocolo(String),
    SinRama(String),
    Otro(String),
}

/// **Un esquema de prueba** en un Postgres de verdad, que se borra al soltarse: el de las pruebas
/// de esta crate y el de las de ore-serve (la conformidad de la fusión), que no enlaza `postgres`.
pub struct EsquemaDePrueba {
    url: String,
    pub esquema: String,
}

impl EsquemaDePrueba {
    /// Crea `t_<prefijo><pid>_<n>` y conecta el almacén a él (con el esquema aplicado).
    pub fn crear(url: &str, prefijo: &str) -> Result<(EsquemaDePrueba, Almacen), String> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let esquema = format!(
            "t_{prefijo}{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        );
        let mut c = Client::connect(url, NoTls).map_err(pg)?;
        c.batch_execute(&format!("CREATE SCHEMA {esquema}"))
            .map_err(pg)?;
        let p = EsquemaDePrueba {
            url: url.to_string(),
            esquema,
        };
        let a = Almacen::conectar(url, &p.esquema)?;
        Ok((p, a))
    }
}

impl Drop for EsquemaDePrueba {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.url, NoTls) {
            let _ = c.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.esquema));
        }
    }
}

impl Almacen {
    /// **Cierra** `rutas` en cada una de `refs` para `dueno` durante `plazo` (`cerrar` de la v3).
    /// El mismo dueño renueva. Mientras tanto, `escribir` en esas rutas y refs contesta
    /// [`FalloEscritura::Cerrado`]. Espera a quien esté escribiendo en ellas ahora.
    pub fn cerrar(
        &mut self,
        refs: &[String],
        rutas: &[String],
        dueno: &str,
        plazo: std::time::Duration,
    ) -> Result<(), FalloCerrar> {
        self.c
            .execute(
                "SELECT cerrar($1::text[], $2::text[], $3, make_interval(secs => $4))",
                &[&refs, &rutas, &dueno, &plazo.as_secs_f64()],
            )
            .map(|_| ())
            .map_err(|e| match e.as_db_error() {
                Some(d) if d.code().code() == "OR429" => FalloCerrar::Ocupado {
                    en: del_detalle(d.detail(), "ref").unwrap_or_default(),
                    ruta: del_detalle(d.detail(), "ruta").unwrap_or_default(),
                    dueno: del_detalle(d.detail(), "dueno").unwrap_or_default(),
                },
                _ => FalloCerrar::Otro(pg(e)),
            })
    }

    /// **Suelta** todo lo de `dueno`. Devuelve cuántos cerrojos soltó.
    pub fn soltar(&mut self, dueno: &str) -> Result<usize, String> {
        let n: i32 = self
            .c
            .query_one("SELECT soltar($1)", &[&dueno])
            .map_err(pg)?
            .get(0);
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// **El plan de fusión** de `rama` en `main`, sin escribir nada: la regla de hoy
    /// (`git::fusionar_puntero` de ore-serve) comparando el puntero **entero**, con la base en
    /// `main` en el punto de salida de la rama (ver `plan_fusion` en `esquema_v3.sql`).
    ///
    /// - `rutas`: el alcance; `None` = todo lo propio de la rama (la fusión sin alcance).
    /// - `elegidos`: `ruta → rama | main`, **por ruta**. Lo de hoy va por nombre (`b.s.n`): quien
    ///   llama lo pasa a las rutas de ese nombre (la de su sitio y, si quiere, la de antes).
    pub fn plan_fusion(
        &mut self,
        rama: &str,
        rutas: Option<&[String]>,
        elegidos: &BTreeMap<String, String>,
    ) -> Result<PlanFusion, String> {
        let elegidos = Json::Obj(
            elegidos
                .iter()
                .map(|(k, v)| (k.clone(), Json::s(v)))
                .collect(),
        )
        .pretty();
        let filas = self
            .c
            .query(
                "WITH r AS (SELECT plan_fusion($1, $2::text[], $3::text::jsonb) j)
                 SELECT r.j::text, r.j->>'estado', t.x->>'activo', t.x->>'caso', t.x->>'resultado',
                        nullif((t.x->'rama')::text, 'null'), nullif((t.x->'main')::text, 'null'),
                        coalesce((t.x->>'propio')::boolean, false)
                   FROM r LEFT JOIN LATERAL jsonb_array_elements(r.j->'pasos') WITH ORDINALITY t(x, o)
                     ON true
                  ORDER BY t.o",
                &[&rama, &rutas, &elegidos],
            )
            .map_err(pg)?;
        let Some(f) = filas.first() else {
            return Err("plan_fusion no devolvió nada".into());
        };
        let estado: String = f.get(1);
        Ok(PlanFusion {
            sin_elegir: estado == "sin elegir",
            texto: f.get(0),
            pasos: filas
                .iter()
                .filter_map(|f| {
                    Some(PasoFusion {
                        ruta: f.get::<_, Option<String>>(2)?,
                        caso: f.get(3),
                        resultado: f.get(4),
                        rama: f.get(5),
                        main: f.get(6),
                        propio: f.get(7),
                    })
                })
                .collect(),
        })
    }

    /// **Aplica un plan** de [`Almacen::plan_fusion`] (`texto`), el que viajó en el commit de git
    /// `sha`: en una transacción e idempotente por `git:<sha>`. Pone en `main` lo que gana la rama
    /// y pone la rama al día (lo fusionado deja de ser suyo; su punto de salida pasa a `main` de
    /// hoy). Con `tardia` (el cerrojo caducó antes de aplicar), lo que `main` movió desde el plan
    /// se queda y sale en [`Aplicada::pendientes`]; sin ella es [`FalloAplicar::Protocolo`].
    pub fn aplicar_plan(
        &mut self,
        rama: &str,
        sha: &str,
        sujeto: &str,
        plan: &str,
        tardia: bool,
    ) -> Result<Aplicada, FalloAplicar> {
        let f = self
            .c
            .query_one(
                "WITH r AS (SELECT aplicar_plan($1, $2, $3, $4::text::jsonb, $5) j),
                      p AS (SELECT t.x, t.o FROM r, jsonb_array_elements(coalesce(r.j->'pendientes', '[]'))
                                                    WITH ORDINALITY t(x, o))
                 SELECT (j->>'commit')::bigint, coalesce((j->>'repetida')::boolean, false),
                        coalesce((j->>'tardia')::boolean, false),
                        ARRAY(SELECT jsonb_array_elements_text(coalesce(j->'aplicados', '[]'))),
                        ARRAY(SELECT x->>'activo' FROM p ORDER BY o),
                        ARRAY(SELECT nullif((x->'se_queda')::text, 'null') FROM p ORDER BY o),
                        ARRAY(SELECT nullif((x->'no_entra')::text, 'null') FROM p ORDER BY o)
                   FROM r",
                &[&rama, &sha, &sujeto, &plan, &tardia],
            )
            .map_err(|e| match e.as_db_error() {
                Some(d) if d.code().code() == "OR422" => FalloAplicar::NoSeAplica(pg(e)),
                Some(d) if d.code().code() == "OR500" => FalloAplicar::Protocolo(pg(e)),
                Some(d) if d.code().code() == "OR404" => FalloAplicar::SinRama(rama.to_string()),
                _ => FalloAplicar::Otro(pg(e)),
            })?;
        let rutas: Vec<String> = f.get(4);
        let se_queda: Vec<Option<String>> = f.get(5);
        let no_entra: Vec<Option<String>> = f.get(6);
        Ok(Aplicada {
            commit: f.get(0),
            repetida: f.get(1),
            tardia: f.get(2),
            aplicados: f.get(3),
            pendientes: rutas
                .into_iter()
                .zip(se_queda)
                .zip(no_entra)
                .map(|((ruta, se_queda), no_entra)| Pendiente {
                    ruta,
                    se_queda,
                    no_entra,
                })
                .collect(),
        })
    }

    /// **Aplica una definición** (retirar, renombrar) que git confirmó en `sha`:
    /// [`Almacen::escribir`] con `idempotencia = git:<sha>`, con su CAS y su
    /// [`FalloEscritura::Cerrado`].
    pub fn aplicar_definicion(
        &mut self,
        r: &str,
        sha: &str,
        sujeto: &str,
        operacion: &str,
        cambios: &[CambioRuta],
    ) -> Result<i64, FalloEscritura> {
        let j = Json::Arr(cambios.iter().map(cambio_en_json).collect()).pretty();
        self.c
            .query_one(
                "SELECT aplicar_definicion($1, $2, $3, $4, $5::text::jsonb)",
                &[&r, &sha, &sujeto, &operacion, &j],
            )
            .map(|f| f.get(0))
            .map_err(|e| fallo_de_escribir(r, e))
    }

    /// **El reconciliador**: ¿está ya aplicado lo del commit de git `sha`?
    pub fn aplicado(&mut self, sha: &str) -> Result<bool, String> {
        Ok(self
            .c
            .query_one("SELECT aplicado($1)", &[&sha])
            .map_err(pg)?
            .get(0))
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
