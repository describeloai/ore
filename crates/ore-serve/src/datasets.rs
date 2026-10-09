//! **Los datasets, servidos** (W3.6b, [0031 §10](../../../docs/decisions/0031-el-puesto.md)):
//! la lista, la ficha con la historia de la tabla, y **el swap** del puntero de
//! un dataset del lago por quien no puede empujar al árbol.
//!
//! | ruta | qué | quién decide |
//! |---|---|---|
//! | `GET /datasets` | los punteros de `copias/` y `datasets/`, con su estado | `ore datasets . --json` |
//! | `GET /datasets/{ns}/{n}` | el puntero y los snapshots de la tabla (`ore-store historia`, con la identidad del pod y su `objectViewer`) | `ore datasets . --ficha ns.n --json` |
//! | `POST /datasets/{ns}/{n}/confirmar` | el puntero de una `Table` del lago pasa a `metadata_location`, con `esperado` como compare-and-set semántico; la `Table` nace con `columnas` si no existe; y **el commit lo empuja este proceso** | `ore datasets . --confirmar ns.n …`, y después `git push` |
//!
//! # Las dos caras del 409
//!
//! `confirmar` puede perder dos carreras y las dos se dicen igual, porque para
//! quien escribe son la misma: *«vuelve a leer y escribe sobre lo que hay
//! ahora»*. La **semántica** —el puntero ya no es `esperado`— la decide `ore`
//! (código 75) sin empujar nada; la de **la forja** —dos empujones a la vez, y
//! el que pierde recibe `[remote rejected]`— la decide git, y `escribiendo` la
//! traduce. Medido (`medida-w3-swap.py`): ocho escritores sobre el mismo
//! puntero dan exactamente uno que gana, y los siete reintentando se
//! serializan en siete rondas.
//!
//! # Lo que este proceso NO hace
//!
//! No escribe en el bucket: el `metadata.json` que se apunta lo escribió el
//! puesto con su identidad, y aquí sólo se comprueba que está (`ore datasets`
//! le pide un HEAD a `ore-store`). `ore-serve` tiene `objectViewer` y le basta.

use std::path::Path;

use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;

use crate::mando;
use crate::rutas::{Servidor, token};

/// **Los datasets que en este árbol se leen de `main` al día** (0044 C.2 ③):
/// los que dice [`crate::git::AL_DIA`], por su nombre corto. `None` si el
/// árbol no es el de una rama con lo no tocado al día.
pub(crate) fn de_main(raiz: &Path) -> Option<std::collections::BTreeSet<String>> {
    let t = std::fs::read_to_string(raiz.join(crate::git::AL_DIA)).ok()?;
    let n = ore_core::parse::parse(&t).ok()?;
    let rutas = n.get("rutas")?.1.entries().to_vec();
    Some(
        rutas
            .iter()
            .filter(|(_, v)| v.as_str() == Some("main"))
            .filter_map(|(ruta, _)| {
                let rel = Path::new(ruta.as_str()?);
                let dentro: std::path::PathBuf = rel.components().skip(1).collect();
                let nodo = std::fs::read_to_string(raiz.join(rel))
                    .ok()
                    .and_then(|t| ore_core::parse::parse(&t).ok());
                ore_core::punteros::clave_de(&dentro, nodo.as_ref())
            })
            .collect(),
    )
}

/// `de: main | rama` en un dataset (o en cada uno de `datasets`) leído en una
/// rama: de dónde sale lo que se enseña.
fn anotar_de(raiz: &Path, mut r: Respuesta) -> Respuesta {
    let Some(de) = de_main(raiz) else {
        return r;
    };
    let marca = |m: &mut std::collections::BTreeMap<String, Json>| {
        if let Some(Json::Str(n)) = m.get("nombre").cloned() {
            let d = if de.contains(&n) { "main" } else { "rama" };
            m.insert("de".into(), Json::s(d));
        }
    };
    if let Json::Obj(m) = &mut r.cuerpo {
        marca(m);
        if let Some(Json::Arr(xs)) = m.get_mut("datasets") {
            for x in xs {
                if let Json::Obj(o) = x {
                    marca(o);
                }
            }
        }
    }
    r
}

impl Servidor {
    /// `GET /datasets`. En una rama, cada uno dice `de` (0044 C.2 ③).
    pub(crate) fn datasets(&self, rama: Option<&str>) -> Respuesta {
        self.leyendo_en(rama, |raiz| {
            anotar_de(
                raiz,
                self.ore_json(raiz, &["datasets".into(), ".".into(), "--json".into()]),
            )
        })
    }

    /// `GET /datasets/{ns}[/{schema}]/{n}`.
    pub(crate) fn ficha_del_dataset(
        &self,
        rama: Option<&str>,
        ns: &str,
        schema: &str,
        n: &str,
    ) -> Respuesta {
        if let Err(m) = token(ns).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let nombre = ore_core::normalize::corto(ns, schema, n);
        self.leyendo_en(rama, move |raiz| {
            let r = self.ore_json(
                raiz,
                &[
                    "datasets".into(),
                    ".".into(),
                    "--ficha".into(),
                    nombre,
                    "--json".into(),
                ],
            );
            anotar_de(raiz, r)
        })
    }

    /// `POST /datasets/{ns}/{n}/confirmar` con
    /// `{metadata_location, esperado?, snapshot?, filas?, columnas?}`.
    ///
    /// ⭐ Con `x-ore-rama`, el puntero se mueve **en esa rama** (0044 C D2), como
    ///   en `/v1`: antes iba a `main` dijera lo que dijera la cabecera (medido,
    ///   D0 M5). Sin ella, lo de siempre.
    pub(crate) fn confirmar_dataset(
        &self,
        rama: Option<&str>,
        sujeto: &Identidad,
        ns: &str,
        schema: &str,
        n: &str,
        cuerpo: &str,
    ) -> Respuesta {
        if let Err(m) = token(ns).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let c = match ore_core::parse::parse(cuerpo) {
            Ok(c) if !cuerpo.trim().is_empty() => c,
            _ => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        let campo = |k: &str| {
            c.get(k)
                .and_then(|(_, v)| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        let Some(ml) = campo("metadata_location") else {
            return Respuesta::error(
                422,
                "falta `metadata_location`: el `metadata.json` que se escribió en el bucket",
            );
        };
        let nombre = ore_core::normalize::corto(ns, schema, n);
        let mut args: Vec<String> = vec![
            "datasets".into(),
            ".".into(),
            "--confirmar".into(),
            nombre.clone(),
            "--json".into(),
            "--metadata-location".into(),
            ml,
            "--sujeto".into(),
            sujeto.persona.clone(),
        ];
        // ⭐ Si el dataset nace con esta escritura, es de quien escribe (ADR 0049 ·
        //   el dueño); si ya estaba, `--owner` no cambia el suyo.
        match self.dueno_de_quien_crea(sujeto) {
            Ok(d) => args.extend(["--owner".into(), d]),
            Err(r) => return r,
        }
        if let Some(e) = campo("esperado") {
            args.push("--esperado".into());
            args.push(e);
        }
        if let Some(s) = campo("snapshot") {
            args.push("--snapshot".into());
            args.push(s);
        }
        if let Some(f) = campo("filas") {
            args.push("--filas".into());
            args.push(f);
        }
        if let Some((_, cols)) = c.get("columnas") {
            // Las columnas viajan como el JSON que llegó: `ore` las analiza.
            args.push("--columnas".into());
            args.push(Json::de_node(cols).jcs());
        }
        let mensaje = format!("confirmar dataset `{nombre}`");
        let hacer = |raiz: &std::path::Path| {
            let s = match mando::correr(&self.binario, raiz, &args) {
                Ok(s) => s,
                Err(e) => return Respuesta::error(500, e.to_string()),
            };
            let cuerpo = s
                .stdout
                .lines()
                .rev()
                .find(|l| l.trim_start().starts_with('{'))
                .and_then(|l| ore_core::parse::parse(l).ok())
                .map(|n| Json::de_node(&n));
            match s.codigo {
                0 => {
                    let nuevo = matches!(&cuerpo, Some(Json::Obj(m)) if m.get("puntero_nuevo") == Some(&Json::Bool(true)));
                    let j = cuerpo.unwrap_or_else(|| Json::obj([("tabla", Json::s(&nombre))]));
                    if nuevo {
                        Respuesta::creado(j)
                    } else {
                        Respuesta::ok(j)
                    }
                }
                75 => {
                    let mut r = Respuesta::error(
                        409,
                        s.stderr
                            .lines()
                            .find_map(|l| l.strip_prefix("error: "))
                            .unwrap_or("el puntero ya no es el esperado")
                            .to_string(),
                    );
                    if let (Json::Obj(m), Some(Json::Obj(c))) = (&mut r.cuerpo, cuerpo) {
                        for (k, v) in c {
                            m.entry(k).or_insert(v);
                        }
                    }
                    r
                }
                64 => Respuesta::error(400, primera_de(&s.stderr)),
                65 => Respuesta::error(422, primera_de(&s.stderr)),
                _ => Respuesta::error(502, primera_de(&s.stderr)),
            }
        };
        match rama {
            None => self.escribiendo(sujeto, &mensaje, hacer),
            Some(_) => self.escribiendo_en(rama, sujeto, &mensaje, hacer),
        }
    }

    /// `GET /colecciones` (0046 E8·1d): cada `MediaCollection` del árbol, con
    /// su forma, su origen y el estado de su puntero.
    pub(crate) fn colecciones(&self, rama: Option<&str>) -> Respuesta {
        self.leyendo_en(rama, |raiz| {
            self.ore_json(raiz, &["collections".into(), ".".into(), "--json".into()])
        })
    }

    /// `GET /colecciones/{b}[/{schema}]/{n}`: la colección y la historia de sus
    /// transacciones.
    pub(crate) fn ficha_de_la_coleccion(
        &self,
        rama: Option<&str>,
        b: &str,
        schema: &str,
        n: &str,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let nombre = ore_core::normalize::corto(b, schema, n);
        self.leyendo_en(rama, move |raiz| {
            self.ore_json(
                raiz,
                &[
                    "collections".into(),
                    ".".into(),
                    "--ficha".into(),
                    nombre,
                    "--json".into(),
                ],
            )
        })
    }

    /// `GET /describe/{kind}/{b}/{s}/{n}` (0049 B8·3): `describe table|object
    /// table|dataset|view|media collection b.s.n` —`kind` with `-` for the
    /// space—, as `DESCRIBE TABLE EXTENDED`: `{rows: [[col_name, data_type,
    /// comment], …]}`, its columns and then `# Detail`. From the tree and the
    /// pointer of the branch, never from the origin.
    pub(crate) fn describir(
        &self,
        rama: Option<&str>,
        kind: &str,
        b: &str,
        schema: &str,
        n: &str,
    ) -> Respuesta {
        use ore_core::document::Kind;
        if let Err(m) = token(b).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let quiere = match kind {
            "table" => Kind::Table,
            "object-table" => Kind::ObjectTable,
            "dataset" => Kind::Dataset,
            "view" => Kind::View,
            "media-collection" => Kind::MediaCollection,
            _ => {
                return Respuesta::error(
                    422,
                    "`kind` is `table`, `object-table`, `dataset`, `view` or `media-collection`",
                );
            }
        };
        let corto = ore_core::normalize::corto(b, schema, n);
        let que = kind.replace('-', " ");
        self.leyendo_en(rama, move |raiz| {
            let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
            let qn = format!("{b}.{schema}.{n}");
            let Some(d) = pkg.docs.iter().find(|d| {
                d.kind != Kind::Package
                    && (d.qname().as_deref() == Some(qn.as_str())
                        || d.qname().as_deref() == Some(corto.as_str()))
            }) else {
                return Respuesta::error(
                    404,
                    format!("there is no {que} `{corto}` in this branch"),
                );
            };
            if d.kind != quiere {
                return Respuesta::error(
                    422,
                    format!("`{corto}` is a `{:?}`, not a {que}", d.kind),
                );
            }
            let puntero =
                ore_core::punteros::leer_en(&raiz.join("datasets"), &corto).map(|(_, p)| p);
            let conducto = std::fs::read_to_string(raiz.join("conduits.yaml"))
                .is_ok_and(|t| t.contains("materialization.payload"));
            let filas = ore_core::assets::describir(&pkg, d, puntero.as_ref(), conducto);
            Respuesta::ok(Json::obj([(
                "rows",
                Json::Arr(
                    filas
                        .into_iter()
                        .map(|f| Json::Arr(f.iter().map(Json::s).collect()))
                        .collect(),
                ),
            )]))
        })
    }

    /// `GET /colecciones/{b}/{s}/{n}/items?estado=&desde=&limite=`: sus ítems,
    /// por estado (`actual` por defecto) y en páginas de hasta mil.
    pub(crate) fn items_de_la_coleccion(
        &self,
        rama: Option<&str>,
        b: &str,
        schema: &str,
        n: &str,
        consulta: &std::collections::BTreeMap<String, String>,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let estado = consulta
            .get("estado")
            .map(String::as_str)
            .unwrap_or("actual");
        if !["actual", "retirado", "perdido", "todos"].contains(&estado) {
            return Respuesta::error(422, "`estado` es `actual`, `retirado`, `perdido` o `todos`");
        }
        let numero = |k: &str, defecto: usize| -> Result<usize, Respuesta> {
            match consulta.get(k) {
                None => Ok(defecto),
                Some(v) => v
                    .parse::<usize>()
                    .map_err(|_| Respuesta::error(422, format!("`{k}` es un número"))),
            }
        };
        let (desde, limite) = match (numero("desde", 0), numero("limite", 100)) {
            (Ok(d), Ok(l)) => (d, l.clamp(1, 1000)),
            (Err(r), _) | (_, Err(r)) => return r,
        };
        let nombre = ore_core::normalize::corto(b, schema, n);
        let estado = estado.to_string();
        self.leyendo_en(rama, move |raiz| {
            self.ore_json(
                raiz,
                &[
                    "collections".into(),
                    ".".into(),
                    "--items".into(),
                    nombre,
                    "--estado".into(),
                    estado,
                    "--desde".into(),
                    desde.to_string(),
                    "--limite".into(),
                    limite.to_string(),
                    "--json".into(),
                ],
            )
        })
    }

    /// **Servir** (0046 E9·2): de cada huella, su ítem y una URL firmada a sus
    /// bytes, que vive 5 minutos (`ttl`, de 30 s a 1 h). `ore-serve` no pasa
    /// bytes: autoriza y firma, y el lago sirve —con rangos, a la velocidad
    /// del lago (103 MB/s medidos en el clúster)—. La URL es un portador: se
    /// anota **quién la pidió, de qué, y cuánto vive** en la actividad de la
    /// organización (`coleccion:servir`), porque el lago no sabe quién lee.
    ///
    /// De una **virtual** (E9·3) los bytes están en el origen: la URL la firma su
    /// lector con la credencial de la fuente, que `ore` no tiene. Si la pide
    /// (`{necesita: {fuente, env}}`, 69), este proceso la lee del cofre **como el
    /// agente de la celda** y vuelve a correr `ore` con ella en el entorno del
    /// hijo, y sólo ahí.
    ///
    /// Quién puede: quien lee los ítems de la colección —la pertenencia a la
    /// organización (0047 A9′)—. La decisión por recurso (`coleccion:leer`
    /// sobre esta colección) llega con 0047 A8: el motor de `ore-iam` sólo
    /// decide sobre la organización todavía.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn servir_items(
        &self,
        rama: Option<&str>,
        p: &ore_entrada::http::Peticion,
        b: &str,
        schema: &str,
        n: &str,
        huellas: &[String],
        ttl: Option<u64>,
        una: bool,
    ) -> Respuesta {
        if let Err(m) = token(b).and(token(schema)).and(token(n)) {
            return Respuesta::error(422, m);
        }
        let nombre = ore_core::normalize::corto(b, schema, n);
        let mut args: Vec<String> = vec![
            "collections".into(),
            ".".into(),
            "--servir".into(),
            nombre,
            "--json".into(),
        ];
        for h in huellas {
            args.push(format!("--huella={h}"));
        }
        if let Some(t) = ttl {
            args.push("--ttl".into());
            args.push(t.to_string());
        }
        let r = self.leyendo_en(rama, |raiz| {
            let mut s = match mando::correr(&self.binario, raiz, &args) {
                Ok(s) => s,
                Err(e) => return Respuesta::error(500, e.to_string()),
            };
            if s.codigo == 69
                && let Some((fuente, env)) = necesita(&s.stdout)
            {
                let (valor, caduca) = match self.credencial_de_la_fuente(&fuente, &env) {
                    Ok(v) => v,
                    Err(r) => return r,
                };
                // ⭐ 0046 E9b: una URL firmada con una credencial temporal deja de
                //   valer cuando caduca la credencial, diga lo que diga su
                //   `X-Amz-Expires`. Se pide, entonces, lo que le queda.
                let args = match caduca {
                    Some(c) => con_ttl_hasta(&args, ttl, c, ahora_ms()),
                    None => args.clone(),
                };
                s = match mando::correr_con(&self.binario, raiz, &args, &[(env, valor)]) {
                    Ok(s) => s,
                    Err(e) => return Respuesta::error(500, e.to_string()),
                };
            }
            let codigo = match s.codigo {
                0 => 200,
                64 => 422,
                65 => 404,
                66 => 409,
                _ => 502,
            };
            if codigo != 200 {
                return Respuesta::error(codigo, primera_de(&s.stderr));
            }
            match s
                .stdout
                .lines()
                .rev()
                .find(|l| l.trim_start().starts_with('{'))
                .and_then(|l| ore_core::parse::parse(l).ok())
            {
                Some(n) => Respuesta::ok(Json::de_node(&n)),
                None => Respuesta::error(502, "`ore collections --servir` no devolvió JSON"),
            }
        });
        if r.codigo != 200 {
            return r;
        }
        let Json::Obj(m) = &r.cuerpo else { return r };
        let items = match m.get("items") {
            Some(Json::Arr(a)) => a.clone(),
            _ => Vec::new(),
        };
        if !items.is_empty() {
            self.contar_lo_servido(p, rama, m, &items);
        }
        if !una {
            return r;
        }
        match items.into_iter().next() {
            Some(Json::Obj(mut item)) => {
                for k in ["coleccion", "segundos", "caduca_ms"] {
                    if let Some(v) = m.get(k) {
                        item.insert(k.into(), v.clone());
                    }
                }
                Respuesta::ok(Json::Obj(item))
            }
            _ => Respuesta::error(404, "ningún ítem de la colección lleva esa huella"),
        }
    }

    /// Lo servido, a la actividad: quién, qué colección, qué huellas y blobs,
    /// y cuánto viven las URLs. Por el buzón: la respuesta no lo espera.
    pub(crate) fn contar_lo_servido(
        &self,
        p: &ore_entrada::http::Peticion,
        rama: Option<&str>,
        m: &std::collections::BTreeMap<String, Json>,
        items: &[Json],
    ) {
        let Some(buzon) = self.buzon.as_ref() else {
            return;
        };
        let de = |k: &str| -> Vec<Json> {
            items
                .iter()
                .filter_map(|i| match i {
                    Json::Obj(o) => o.get(k).cloned(),
                    _ => None,
                })
                .collect()
        };
        let virtual_ = m.get("virtual") == Some(&Json::Bool(true));
        let mut detalle = vec![
            ("huellas", Json::Arr(de("huella"))),
            if virtual_ {
                // Del origen: qué versión de qué clave.
                (
                    "versiones",
                    Json::Arr(
                        de("clave")
                            .into_iter()
                            .zip(de("version"))
                            .map(|(c, v)| Json::Arr(vec![c, v]))
                            .collect(),
                    ),
                )
            } else {
                ("blobs", Json::Arr(de("blob")))
            },
            (
                "segundos",
                m.get("segundos").cloned().unwrap_or(Json::Int(0)),
            ),
        ];
        if let Some(r) = rama.filter(|r| !r.trim().is_empty()) {
            detalle.push(("rama", Json::s(r.trim())));
        }
        let coleccion = match m.get("coleccion") {
            Some(Json::Str(c)) => c.clone(),
            _ => String::new(),
        };
        let mut e = crate::acceso::evento(
            "coleccion:servir",
            &format!("colecciones/{}", coleccion.replace('.', "/")),
            "hecho",
            self.decision_de_quien_llama(p),
            None,
        );
        e.detalle = Some(Json::obj(detalle));
        let token = p
            .cabeceras
            .get("authorization")
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })
            .map(|v| v.trim().to_string());
        buzon.echar(token, e);
    }

    /// La credencial de una fuente, del cofre, **como el agente de la celda**
    /// (0046 E9·3). El cofre decide (`usar` sobre `fuente-<n>`) y lo anota.
    ///
    /// ⭐ Si la fuente es un ROL de AWS (0046 E9b: `role_arn` en la URL, sin
    ///   clave), se canjea por una credencial temporal con `ore-asumir-rol` —este
    ///   proceso no habla TLS— y se guarda mientras le quede
    ///   [`crate::agente::VIGENTE`]: servir no vuelve al custodio ni a STS en
    ///   una hora. Devuelve también cuándo caduca, que acota la URL firmada.
    pub(crate) fn credencial_de_la_fuente(
        &self,
        fuente: &str,
        env: &str,
    ) -> Result<(String, Option<u64>), Respuesta> {
        if token(fuente).is_err() || !crate::agente::variable_admisible(env) {
            return Err(Respuesta::error(
                422,
                format!(
                    "la fuente `{fuente}` declara `{env}`: sólo una `<ALGO>_URL` lleva su credencial"
                ),
            ));
        }
        let (Some(agente), Some(cofre), Some(org)) =
            (&self.agente, &self.cofre, &self.organizacion)
        else {
            return Err(Respuesta::error(
                503,
                "esta celda no sabe traer la credencial de una fuente: le falta el agente \
                 (`--agente-fichero`, `--idp`) o el custodio (`--cofre`, `--organizacion`)",
            ));
        };
        credencial_del_cofre(agente, cofre, org, fuente, &self.binario)
    }
}

/// **Quién canjea la credencial de una fuente por una corta** (ADR 0061 O0·4),
/// por su tipo: un bucket de S3 por rol (`role_arn`), `ore-asumir-rol` (0046
/// E9b). `None`: la credencial guardada es la que se usa. GCS, Azure y
/// SharePoint no canjean nada aquí (ADR 0061 O2·3, O3·3, O5·3): sus URLs no llevan secreto, y quien
/// lee —el driver, `ore-medios`— obtiene su token él mismo (con su cuenta,
/// suplantando, o federada en la app de Entra del cliente).
pub(crate) fn canjeador_de(valor: &str) -> Option<&'static str> {
    let esquema = valor.split_once("://").map(|(e, _)| e)?;
    match esquema {
        "s3" if valor.contains("role_arn=") => Some("ore-asumir-rol"),
        _ => None,
    }
}

/// **La credencial de una fuente, del custodio, como el agente de la celda**:
/// lo que hace [`Servidor::credencial_de_la_fuente`] una vez comprobado que la
/// celda sabe traerla. Libre para que la use también `ore-serve
/// federar-probar` (0053 F3·4) y, en F4, el coordinador.
pub(crate) fn credencial_del_cofre(
    agente: &crate::agente::Agente,
    cofre: &str,
    org: &str,
    fuente: &str,
    binario: &Path,
) -> Result<(String, Option<u64>), Respuesta> {
    {
        if let Some((url, caduca)) = agente.temporal(fuente, ahora_ms()) {
            return Ok((url, Some(caduca)));
        }
        let t = agente.token().map_err(|e| Respuesta::error(503, e))?;
        let valor = match ore_entrada::http::pedir(
            "GET",
            cofre,
            &format!("/organizaciones/{org}/secretos/fuente-{fuente}"),
            Some(&t),
            None,
        ) {
            Ok((200, b)) => crate::agente::valor_de(&b).ok_or_else(|| {
                Respuesta::error(
                    502,
                    format!("el custodio no dio el valor de `fuente-{fuente}`"),
                )
            }),
            Ok((404, _)) => Err(Respuesta::error(
                409,
                format!("`fuente-{fuente}` no está en el custodio: la fuente no tiene credencial"),
            )),
            Ok((c, b)) => Err(Respuesta::error(
                502,
                format!(
                    "el custodio contestó {c} a `fuente-{fuente}`: {}",
                    b.trim().chars().take(120).collect::<String>()
                ),
            )),
            Err(e) => Err(Respuesta::error(
                503,
                format!("el custodio no contesta: {e}"),
            )),
        }?;
        let Some(canjeador) = canjeador_de(&valor) else {
            return Ok((valor, None));
        };
        let canjeador = binario.with_file_name(canjeador);
        let s = mando::con_entrada(&canjeador, &["--sesion", "ore-serve"], &valor)
            .map_err(|e| Respuesta::error(500, e))?;
        if s.codigo != 0 {
            return Err(Respuesta::error(
                502,
                format!(
                    "no se pudo asumir el rol de `{fuente}`: {}",
                    primera_de(&s.stderr).trim_start_matches("✗ ")
                ),
            ));
        }
        let n = ore_core::parse::parse(s.stdout.trim())
            .map_err(|_| Respuesta::error(502, "`ore-asumir-rol` no devolvió JSON".to_string()))?;
        let url = n.get("url").and_then(|(_, v)| v.as_str()).map(String::from);
        let caduca = n
            .get("caduca_ms")
            .and_then(|(_, v)| v.as_str())
            .and_then(|c| c.parse::<u64>().ok());
        let (Some(url), Some(caduca)) = (url, caduca) else {
            return Err(Respuesta::error(
                502,
                "`ore-asumir-rol` no dio la credencial temporal",
            ));
        };
        agente.guardar_temporal(fuente, url.clone(), caduca);
        Ok((url, Some(caduca)))
    }
}

impl Servidor {
    /// Corre `ore` y devuelve la última línea JSON de su salida tal cual; lo
    /// que no es 0 es 502 con lo que dijo.
    fn ore_json(&self, raiz: &Path, args: &[String]) -> Respuesta {
        let s = match mando::correr(&self.binario, raiz, args) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(500, e.to_string()),
        };
        if !s.bien() {
            return Respuesta::error(
                if s.codigo == 65 { 404 } else { 502 },
                primera_de(&s.stderr),
            );
        }
        match s
            .stdout
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .and_then(|l| ore_core::parse::parse(l).ok())
        {
            Some(n) => Respuesta::ok(Json::de_node(&n)),
            None => Respuesta::error(502, "`ore datasets` no devolvió JSON"),
        }
    }
}

/// `{necesita: {fuente, env}}` en la salida de `ore collections --servir`: la
/// credencial que falta para firmar una virtual.
fn necesita(stdout: &str) -> Option<(String, String)> {
    stdout.lines().find_map(|l| {
        let n = ore_core::parse::parse(l.trim()).ok()?;
        let (_, x) = n.get("necesita")?;
        let c = |k: &str| x.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        Some((c("fuente")?, c("env")?))
    })
}

/// Un segmento de la ruta con sus `%XX` resueltos: una huella lleva `/`, `+`,
/// `=` y `:` (`crc64nvme:<base64>`), y viaja codificada. `None` si no es UTF-8.
pub(crate) fn sin_porcentajes(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn primera_de(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start_matches("error: ").to_string())
        .unwrap_or_else(|| "falló sin decir por qué".into())
}

/// Lo que vive una URL firmada si nadie dice `ttl` (el de `ore collections --servir`).
const TTL_POR_DEFECTO: u64 = 300;

pub(crate) fn ahora_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Los argumentos de `ore collections --servir` con un `--ttl` que no pasa de lo
/// que le queda a la credencial (menos 30 s de holgura), y nunca de 30 s.
fn con_ttl_hasta(args: &[String], ttl: Option<u64>, caduca_ms: u64, ahora_ms: u64) -> Vec<String> {
    let queda = caduca_ms.saturating_sub(ahora_ms) / 1000;
    let tope = queda.saturating_sub(30).max(30);
    let pedido = ttl.unwrap_or(TTL_POR_DEFECTO);
    if pedido <= tope {
        return args.to_vec();
    }
    let mut out: Vec<String> = Vec::with_capacity(args.len() + 2);
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--ttl" {
            i += 2;
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    out.push("--ttl".into());
    out.push(tope.to_string());
    out
}

#[cfg(test)]
mod pruebas {
    /// ADR 0061 O0·4: el canje es del tipo de la fuente; lo que no se canjea,
    /// se usa como está.
    #[test]
    fn el_canje_es_del_tipo_de_la_fuente() {
        use super::canjeador_de;
        assert_eq!(
            canjeador_de("s3://b/?region=x&role_arn=arn:aws:iam::1:role/r"),
            Some("ore-asumir-rol")
        );
        assert_eq!(
            canjeador_de("s3://b/?access_key_id=a&secret_access_key=s"),
            None
        );
        assert_eq!(
            canjeador_de("postgres://u:p@h/db?role_arn=x"),
            None,
            "otro tipo"
        );
        assert_eq!(canjeador_de("sin esquema role_arn="), None);
        assert_eq!(
            canjeador_de("gs://cubo/?suplantar=l@c.iam.gserviceaccount.com"),
            None
        );
        assert_eq!(canjeador_de("az://cuenta/cubo?tenant=t&cliente=c"), None);
        assert_eq!(
            canjeador_de("sharepoint://contoso.sharepoint.com/D?tenant=t&cliente=c"),
            None
        );
    }

    #[test]
    fn la_url_no_vive_mas_que_la_credencial_temporal() {
        let a = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let base = a(&["collections", ".", "--servir", "b.s.n", "--json"]);
        // Le queda una hora: los 300 s por defecto caben, nada cambia.
        assert_eq!(super::con_ttl_hasta(&base, None, 3_600_000, 0), base);
        // Le quedan 200 s: se piden 170, aunque nadie dijera `ttl`.
        let r = super::con_ttl_hasta(&base, None, 200_000, 0);
        assert_eq!(r[r.len() - 2..], a(&["--ttl", "170"])[..]);
        // Un `--ttl 3600` con 20 minutos: 1170, y el viejo fuera.
        let con = a(&["collections", ".", "--ttl", "3600", "--json"]);
        let r = super::con_ttl_hasta(&con, Some(3600), 1_200_000, 0);
        assert_eq!(r, a(&["collections", ".", "--json", "--ttl", "1170"]));
        // Nunca por debajo de 30 s (lo mínimo que `ore` acepta).
        let r = super::con_ttl_hasta(&base, None, 10_000, 0);
        assert_eq!(r[r.len() - 1], "30");
    }

    use super::sin_porcentajes;

    /// Una huella de S3 viaja en la ruta con su `/`, `+`, `=` y `:` codificados.
    #[test]
    fn la_huella_vuelve_de_la_ruta() {
        assert_eq!(
            sin_porcentajes("crc64nvme%3Aab%2Fc%2Bd%3D").as_deref(),
            Some("crc64nvme:ab/c+d=")
        );
        assert_eq!(sin_porcentajes("sin-nada").as_deref(), Some("sin-nada"));
        assert_eq!(sin_porcentajes("%zz"), None);
        assert_eq!(sin_porcentajes("%FF"), None, "no es UTF-8");
        assert_eq!(sin_porcentajes("a%2").as_deref(), Some("a%2"));
    }

    #[test]
    fn lo_que_ore_necesita_para_una_virtual() {
        assert_eq!(
            super::necesita("{\"necesita\":{\"fuente\":\"s3_x\",\"env\":\"S3_X_URL\"}}\n"),
            Some(("s3_x".into(), "S3_X_URL".into()))
        );
        assert_eq!(super::necesita("{\"items\":[]}"), None);
    }
}
