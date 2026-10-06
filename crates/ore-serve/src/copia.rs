//! La copia en la celda (0027 P1): **la base estándar**.
//!
//! # La copia se induce, no se edita
//!
//! `ore discover` no propone la copia (`inductor.rs`: proponerla sería
//! inventarla), y `ore review` **re-induce el paquete entero** desde el catálogo
//! y las respuestas — una edición a mano entre las dos se pierde. Así que la
//! copia no puede ser un campo que alguien escribe (el verbo por vista de I2,
//! retirado): tiene que salir de la inducción. Y sale de una **regla**: la
//! clase de la base, `"type": "standard"` en `discover.scope.json` —el
//! documento que ya guarda qué entró y de dónde, y que `review` lee y no
//! reescribe—. Con ella, el inductor emite **un `Dataset` con el plan** (0033,
//! `packages/<p>/datasets/`) por cada tabla que **tiene clave** (la del origen o
//! la contestada en `clave`) y `changes: mode: upsert, key` en su tabla. Sin
//! clave, la tabla sale con su vista y espera: una copia que sólo anexa no
//! puede respaldar una entidad (`OOS2021`), y la decisión `clave` de la cola
//! dice que la copia la espera. Contestarla la trae.
//!
//! # Lo que queda de este lado
//!
//! Lo que NO se induce: `conduits.yaml` (en la raíz del árbol, fuera del
//! paquete) tiene que autorizar `materialization.payload`, o el dataset no
//! compila (`OOS4011`, medido); y el Job de la copia (I3) hay que encolarlo
//! cuando la lista de datasets mantenidos cambia. Las dos cosas las hace
//! [`Servidor::tras_inducir`], después de cada inducción que `ore-serve`
//! dispara: el alta (`POST /paquetes {type}`), ascender (`POST /paquetes/{n}/
//! copia` = la clase al alcance + `review --reinducir`) y contestar decisiones.
//!
//! # Lo que no hace
//!
//! No copia nada: eso es el Job `copiar-<resumen>` (I3), que lee lo que el
//! árbol declara. No elige qué gana cuando el origen y la copia se contradicen
//! (functions.md §7.4): es de F5.
use crate::cola;
use crate::mando;
use crate::rutas::{Servidor, de_node, primera_linea, token};
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::{Path, PathBuf};

/// 0049 B8·3: ¿es este YAML una `MediaCollection` mantenida —con origen
/// (`from`) y sin `virtual: true`—? Una escrita (sin `from`) no se copia de
/// ningún sitio.
pub(crate) fn es_coleccion_mantenida(texto: &str) -> bool {
    let Ok(n) = parse::parse(texto) else {
        return false;
    };
    if campo(&n, "kind").as_deref() != Some("MediaCollection") {
        return false;
    }
    let Some((_, spec)) = n.get("spec") else {
        return false;
    };
    spec.get("from").is_some() && campo(spec, "virtual").as_deref() != Some("true")
}

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
}

thread_local! {
    /// La rama (no la de por defecto) en la que esta petición mueve datos.
    static EN_RAMA: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// **Esta petición mueve datos en una rama** (0044 C.2 ④) mientras viva: la
/// copia que encole es la de la rama, con lo que la rama cambió. Una marca y no
/// un parámetro porque el encolado está en lo hondo de `tras_inducir`, que
/// llaman el alta, ascender, modelar y las decisiones.
pub(crate) struct EnRama;

impl EnRama {
    pub(crate) fn poner(rama: &str) -> EnRama {
        EN_RAMA.with(|c| *c.borrow_mut() = Some(rama.to_string()));
        EnRama
    }
}

impl Drop for EnRama {
    fn drop(&mut self) {
        EN_RAMA.with(|c| *c.borrow_mut() = None);
    }
}

fn rama_de_la_copia() -> Option<String> {
    EN_RAMA.with(|c| c.borrow().clone())
}

impl Servidor {
    /// **Ascender** una base foránea a estándar: `POST /paquetes/{n}/copia`.
    /// La clase al alcance y `ore review --reinducir`: el inductor aplica la
    /// regla sobre lo que ya hay contestado. Luego lo de siempre tras inducir.
    pub(crate) fn ascender(&self, raiz: &Path, paquete: &str, sujeto: &Identidad) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, "no hay tal paquete");
        }
        let alcance = dir.join("discover.scope.json");
        let Ok(texto) = std::fs::read_to_string(&alcance) else {
            return Respuesta::error(
                422,
                format!(
                    "`{paquete}` no es una base: no tiene `discover.scope.json` (no salió de un alta con `only`)"
                ),
            );
        };
        if clase_de(&dir) == "standard" {
            return Respuesta::error(409, format!("`{paquete}` ya es una base estándar"));
        }
        let con_clase = match parse::parse(&texto).map(|n| de_node(&n)) {
            Ok(Json::Obj(mut m)) => {
                m.insert("type".into(), Json::s("standard"));
                Json::Obj(m).pretty()
            }
            _ => {
                return Respuesta::error(422, "`discover.scope.json` no analiza como un objeto");
            }
        };
        if let Err(e) = std::fs::write(&alcance, &con_clase) {
            return Respuesta::error(500, format!("no se pudo escribir el alcance: {e}"));
        }
        let salida = mando::correr(
            &self.binario,
            raiz,
            &[
                "review".into(),
                dir.to_string_lossy().into_owned(),
                "--reinducir".into(),
            ],
        );
        match salida {
            Err(e) => {
                let _ = std::fs::write(&alcance, &texto);
                Respuesta::error(500, e.to_string())
            }
            Ok(s) if !s.bien() => {
                let _ = std::fs::write(&alcance, &texto);
                Respuesta::error(
                    422,
                    format!(
                        "`ore review --reinducir` devolvió {}: {}",
                        s.codigo,
                        primera_linea(&s.stdout, &s.stderr)
                    ),
                )
            }
            Ok(s) => {
                let mut campos = vec![
                    ("package", Json::s(paquete)),
                    ("type", Json::s("standard")),
                    ("informe", Json::s(s.stdout.trim())),
                ];
                campos.extend(self.tras_inducir(raiz, paquete, sujeto));
                Respuesta::creado(Json::obj(campos))
            }
        }
    }

    /// **Retirar una base**: `DELETE /paquetes/{n}`. El paquete fuera del
    /// árbol —una base es lo que alguien eligió, y puede dejar de elegirlo—;
    /// sólo una base (con alcance): la fuente entera que el Job de catálogo
    /// dejó no se retira por aquí (409). Si algo del árbol la nombra (otra
    /// vista, una función), `validate` lo dice y nada se borra. Y la cola al
    /// día: si declaraba copias, el Job se reencola con las que quedan.
    pub(crate) fn retirar_paquete(
        &self,
        raiz: &Path,
        paquete: &str,
        sujeto: &Identidad,
    ) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, "no hay tal paquete");
        }
        if !dir.join("discover.scope.json").is_file() {
            return Respuesta::error(
                409,
                format!(
                    "`{paquete}` no es una base: es la fuente entera que dejó el Job de catálogo, y se retira con la fuente"
                ),
            );
        }
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let tenia_copias = !vistas_con_copia_de(&dir).is_empty();
        // fuera del árbol a un sitio temporal, por si hay que volver a ponerlo
        let aparte = raiz.join(format!(".retirando-{paquete}"));
        if let Err(e) = std::fs::rename(&dir, &aparte) {
            return Respuesta::error(500, format!("no se pudo retirar el paquete: {e}"));
        }
        // Retirar sólo puede empeorar el árbol por lo que NOMBRA a la base
        // retirada: un diagnóstico nuevo que no la nombra estaba tapado por una
        // fase anterior del validador (la de la propia base), no causado.
        let nombra = |d: &Json| !texto_de(d).contains(paquete);
        if let Err(r) = self.empeora_salvo(raiz, &antes, &format!("retirar `{paquete}`"), nombra) {
            let _ = std::fs::rename(&aparte, &dir);
            return r;
        }
        if let Err(e) = std::fs::remove_dir_all(&aparte) {
            return Respuesta::error(500, format!("no se pudo borrar el paquete: {e}"));
        }
        // ⭐ Sus punteros en el árbol, fuera en el mismo commit: un recibo de
        //   una vista que ya no está es un recibo de nadie. Los de SU base —el
        //   nombre que dice cada puntero, no un prefijo del fichero: medido
        //   (`medida-los-punteros.sh` M4), `ventas_` se llevaba los de
        //   `ventas_eu` (0038 P2)—.
        let mut recibos = 0usize;
        for (nombre, (ruta, _)) in
            ore_core::punteros::todos_en(&raiz.join(ore_core::punteros::CARPETA))
        {
            if ore_core::punteros::partes(&nombre).is_some_and(|(b, _, _)| b == paquete)
                && std::fs::remove_file(&ruta).is_ok()
            {
                recibos += 1;
            }
        }
        // Y el de antes que quedara debajo de uno de su sitio (`todos_en` da uno
        // por nombre): tampoco es de nadie.
        let _ = std::fs::remove_dir_all(raiz.join(ore_core::punteros::CARPETA).join(paquete));
        let mut campos = vec![
            ("package", Json::s(paquete)),
            ("retirado", Json::Bool(true)),
            ("recibos", Json::Int(recibos as i64)),
        ];
        // ⭐ 0046 E5′: retirar una base NO toca su fuente. Sus punteros son
        //   hechos del origen y los escribió el catálogo; se van cuando el
        //   objeto desaparece del origen, no cuando deja de leerlos una base.
        if tenia_copias {
            // ⭐ Se encola AUNQUE no quede ninguna vista con copia: esa pasada es
            //   la que recoge del almacén lo que la base retirada dejó
            //   (`materialize --recoger` → `recoger-huerfanas`). Sin ella, el
            //   bucket acumularía copias de nadie (medido el 2026-09-18).
            let quedan = vistas_con_copia(raiz);
            let encolado = self.encolar_copia(&quedan, sujeto);
            campos.push((
                "encolado",
                Json::s(if quedan.is_empty() {
                    format!("{encolado} · sin vistas: la pasada que recoge lo huérfano")
                } else {
                    encolado
                }),
            ));
        }
        Respuesta::ok(Json::obj(campos))
    }

    /// **Modelar una tabla** de una base: `POST /paquetes/{n}/tablas/{objeto}/
    /// modelar` → `ore model` (la tabla a `entities` del alcance y la
    /// re-inducción). Lo que Foundry llama *promote to object type*: la tabla
    /// gana su `Entity` y sus decisiones, y su copia, si la base es estándar,
    /// pasa a esperar la clave. Y lo de siempre tras inducir.
    pub(crate) fn modelar(
        &self,
        raiz: &Path,
        paquete: &str,
        objeto: &str,
        sujeto: &Identidad,
    ) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        // el objeto es como el catálogo lo nombra —`public.pedidos`—: con punto
        if objeto.is_empty()
            || objeto.len() > 128
            || !objeto
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return Respuesta::error(
                422,
                "el objeto es el nombre físico que el catálogo le da (`public.pedidos`)",
            );
        }
        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, "no hay tal paquete");
        }
        let salida = mando::correr(
            &self.binario,
            raiz,
            &[
                "model".into(),
                dir.to_string_lossy().into_owned(),
                objeto.into(),
            ],
        );
        match salida {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => {
                let motivo = primera_linea(&s.stdout, &s.stderr);
                let codigo = if motivo.contains("ya está modelada") {
                    409
                } else if motivo.contains("no está en el alcance") {
                    404
                } else {
                    422
                };
                Respuesta::error(codigo, motivo)
            }
            Ok(s) => {
                let mut campos = vec![
                    ("package", Json::s(paquete)),
                    ("object", Json::s(objeto)),
                    ("informe", Json::s(s.stdout.trim())),
                ];
                campos.extend(self.tras_inducir(raiz, paquete, sujeto));
                Respuesta::creado(Json::obj(campos))
            }
        }
    }

    /// **Copiar una tabla** de una base foránea: `POST /paquetes/{n}/tablas/
    /// {objeto}/copiar` → `ore copy` (la tabla a `copies` del alcance y la
    /// re-inducción). La excepción a la clase: la base sigue foránea y esa tabla
    /// se copia. 409 si ya se copia o si la base es estándar; 404 fuera del
    /// alcance. Y lo de siempre tras inducir.
    pub(crate) fn copiar_tabla(
        &self,
        raiz: &Path,
        paquete: &str,
        objeto: &str,
        sujeto: &Identidad,
    ) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        // ⛔ 0057 (OOS v1alpha27): una base foránea expone su fuente y no
        //   copia nada (OOS2049). Copiar es de una base estándar.
        if es_foranea_v27(&raiz.join("packages").join(paquete)) {
            return Respuesta::error(
                409,
                format!(
                    "`{paquete}` es una base foránea: expone su fuente y no copia. Copiar es de \
                     una base estándar"
                ),
            );
        }
        if objeto.is_empty()
            || objeto.len() > 128
            || !objeto
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return Respuesta::error(
                422,
                "el objeto es el nombre físico que el catálogo le da (`public.pedidos`)",
            );
        }
        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, "no hay tal paquete");
        }
        let salida = mando::correr(
            &self.binario,
            raiz,
            &[
                "copy".into(),
                dir.to_string_lossy().into_owned(),
                objeto.into(),
            ],
        );
        match salida {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => {
                let motivo = primera_linea(&s.stdout, &s.stderr);
                let codigo = if motivo.contains("ya se copia") {
                    409
                } else if motivo.contains("no está en el alcance") {
                    404
                } else {
                    422
                };
                Respuesta::error(codigo, motivo)
            }
            Ok(s) => {
                let mut campos = vec![
                    ("package", Json::s(paquete)),
                    ("object", Json::s(objeto)),
                    ("informe", Json::s(s.stdout.trim())),
                ];
                campos.extend(self.tras_inducir(raiz, paquete, sujeto));
                Respuesta::creado(Json::obj(campos))
            }
        }
    }

    /// **Una colección que pasa a mantenida** (0049 B8·3): `alter media
    /// collection … set managed`, o una que nace mantenida (`create … from
    /// object table` sin `virtual`). Mantenida quiere decir sus bytes en el
    /// lago, así que su copia se encola al escribirla —en la rama donde se
    /// escribe (`EnRama`), y al fusionar `main` adopta lo copiado—. Con las
    /// reglas de siempre: sin conducto no se encola y se dice (OOS4011). Lo que
    /// devuelve va en la respuesta (`copy`); nunca tumba lo escrito.
    pub(crate) fn tras_mantener(
        &self,
        raiz: &Path,
        paquete: &str,
        coleccion: &str,
        sujeto: &Identidad,
    ) -> Json {
        let dir = raiz.join("packages").join(paquete);
        if let Err(r) = autorizar_conducto(raiz, &dir, paquete) {
            return Json::obj([
                ("queued", Json::Bool(false)),
                (
                    "reason",
                    Json::s(
                        "the conduit `materialization.payload` waits for the package owner (OOS4011): the copy is queued when it is answered",
                    ),
                ),
                ("conduit", r.cuerpo),
            ]);
        }
        let encolado = self.encolar_copia(&[format!("{paquete}.{coleccion}")], sujeto);
        Json::obj([
            ("queued", Json::Bool(!encolado.starts_with("NO"))),
            ("detail", Json::s(encolado)),
        ])
    }

    /// **Después de cada inducción que este servidor dispara** (alta, ascender,
    /// decisiones): si la base es estándar, el conducto autorizado y el Job de
    /// la copia en la cola con todas las vistas del árbol que la declaran. Lo
    /// que devuelve va en la respuesta: `copias {declaradas, copiadas}` y
    /// `encolado`. Nunca tumba lo inducido, que ya está escrito.
    pub(crate) fn tras_inducir(
        &self,
        raiz: &Path,
        paquete: &str,
        sujeto: &Identidad,
    ) -> Vec<(&'static str, Json)> {
        let dir = raiz.join("packages").join(paquete);
        let mut campos = vec![("copias", copias_de(raiz, paquete))];
        // Hay algo que mantener si la base es estándar, si alguna vista declara
        // copia una a una (una foránea con tablas copiadas) O si tiene
        // colecciones mantenidas (0046 E8·1: una foránea con sus colecciones
        // virtuales también tiene transacciones que hacer).
        let colecciones = colecciones_de(&dir);
        let copia = clase_de(&dir) == "standard"
            || !vistas_con_copia_de(&dir).is_empty()
            || colecciones.iter().any(|(_, virtual_)| !virtual_);
        if !copia && colecciones.is_empty() {
            return campos;
        }
        // El conducto es de lo que copia bytes: una colección virtual no cruza
        // `materialization.payload` (spec 02 §6), y una foránea que sólo tiene
        // de ésas no lo pide.
        if copia && let Err(r) = autorizar_conducto(raiz, &dir, paquete) {
            // Sin conducto la copia no compila (OOS4011): un Job ahora fallaría
            // y, con la misma lista, no se volvería a encolar. Se espera al
            // dueño; la pasada de decisiones que lo traiga encola entonces.
            campos.push(("conducto", r.cuerpo));
            campos.push((
                "encolado",
                Json::s("NO encolado: el conducto espera al dueño del paquete; se encola al contestar `dueno`"),
            ));
            return campos;
        }
        // ⭐ En una rama, lo que ELLA cambió y lo que depende de ello (0044 C.2 ④),
        //   colecciones incluidas (D7d); lo demás se lee de `main` al día (③).
        let todas = if rama_de_la_copia().is_some() {
            lo_mantenido_de_la_rama(raiz)
        } else {
            let mut t = vistas_con_copia(raiz);
            t.extend(colecciones_de_todos(raiz));
            t
        };
        let encolado = if todas.is_empty() {
            "nada que encolar: ninguna vista declara copia todavía (esperan su clave)".to_string()
        } else {
            self.encolar_copia(&todas, sujeto)
        };
        campos.push(("encolado", Json::s(encolado)));
        campos
    }

    /// `GET /paquetes/{n}/copias`: los datasets mantenidos del paquete, con su
    /// tabla y su clave. Lo que el Job de I3 va a copiar.
    pub(crate) fn copias(&self, raiz: &Path, paquete: &str) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, "no hay tal paquete");
        }
        let mut lista = Vec::new();
        // ⭐ 0045 P2: la tabla de cada copia se resuelve por el árbol: puede vivir
        //   en el paquete de su fuente, no en la carpeta hermana.
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        {
            // La raíz del paquete y la de cada schema (0038 P5).
            let rutas = crate::rutas::yamls_del_kind(&dir, "datasets");
            for p in rutas {
                let Ok(texto) = std::fs::read_to_string(&p) else {
                    continue;
                };
                let Ok(n) = parse::parse(&texto) else {
                    continue;
                };
                if campo(&n, "kind").as_deref() != Some("Dataset") {
                    continue;
                }
                let Some((_, spec)) = n.get("spec") else {
                    continue;
                };
                // Un dataset mantenido: lleva `from`. Uno escrito no se copia.
                let Some((_, de)) = spec.get("from") else {
                    continue;
                };
                let nombre = n
                    .get("metadata")
                    .and_then(|(_, m)| campo(m, "name"))
                    .unwrap_or_default();
                let tabla = spec.get("from").and_then(|(_, f)| campo(f, "table"));
                let rel = p.strip_prefix(raiz).unwrap_or(&p).to_path_buf();
                let clave = pkg
                    .docs
                    .iter()
                    .find(|d| d.path == p || d.path.ends_with(&rel))
                    .and_then(|d| match ore_core::vistas::fuente(d) {
                        Some(ore_core::vistas::Fuente::Tabla(qn)) => pkg.table(&qn),
                        _ => None,
                    })
                    .and_then(|t| t.section("changes"))
                    .and_then(|ch| ch.get("key"))
                    .map(|(_, k)| de_node(k))
                    .unwrap_or(Json::Arr(Vec::new()));
                // Su schema (0038): el informe es el del puntero de su forma corta.
                let schema = n
                    .get("metadata")
                    .and_then(|(_, m)| campo(m, "schema"))
                    .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
                let en_su_schema = if schema == ore_core::normalize::SCHEMA_POR_DEFECTO {
                    nombre.clone()
                } else {
                    format!("{schema}.{nombre}")
                };
                lista.push(Json::obj([
                    ("dataset", Json::s(nombre.clone())),
                    ("schema", Json::s(&schema)),
                    ("table", tabla.map(Json::s).unwrap_or(Json::Bool(false))),
                    ("from", de_node(de)),
                    ("key", clave),
                    ("copia", informe_de(raiz, paquete, &en_su_schema)),
                ]));
            }
        }
        Respuesta::ok(Json::obj([("copias", Json::Arr(lista))]))
    }

    /// El Job de la copia a la cola de trabajo, rendido de la plantilla que el
    /// aprovisionador dejó allí. Devuelve una frase que dice qué pasó — nunca
    /// tumba la decisión, que ya está escrita.
    /// `POST /paquetes/{n}/copia/rehacer`: encola el Job de la copia en modo
    /// rehacer para las vistas con copia de ESTE paquete. Es lo que hace
    /// falta cuando el recibo miente —cambió cómo se lee, o el testigo no se
    /// mueve aunque los datos sí— y no tiene otra puerta: sin esto, la copia
    /// rota es para siempre (medida W1 §B, `products` en demo).
    pub(crate) fn rehacer_copia(
        &self,
        raiz: &Path,
        paquete: &str,
        sujeto: &Identidad,
    ) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("paquete: {m}"));
        }
        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, format!("no hay ningún paquete `{paquete}`"));
        }
        let vistas: Vec<String> = vistas_con_copia_de(&dir)
            .into_iter()
            .map(|v| format!("{paquete}.{v}"))
            .collect();
        if vistas.is_empty() {
            return Respuesta::error(
                409,
                format!("`{paquete}` no tiene ninguna vista con copia: no hay nada que rehacer"),
            );
        }
        let instante = crate::funciones::corrida_ahora();
        let Some(forja) = &self.cola else {
            return Respuesta::error(
                409,
                "este servidor no sabe de ninguna cola (`--cola`): no se puede encolar",
            );
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return Respuesta::error(502, e.to_string()),
        };
        let cdir = prestado.ruta();
        let plantilla = match std::fs::read_to_string(cdir.join(cola::PLANTILLA_COPIA)) {
            Ok(t) => t,
            Err(_) => {
                return Respuesta::error(
                    409,
                    format!(
                        "la cola no trae `{}`; hay que converger este inquilino",
                        cola::PLANTILLA_COPIA
                    ),
                );
            }
        };
        let rama = rama_de_la_copia();
        let (fichero, texto) =
            match cola::rendir_rehacer(&plantilla, &vistas, &instante, rama.as_deref()) {
                Ok(v) => v,
                Err(e) => return Respuesta::error(409, e),
            };
        if let Err(e) = std::fs::write(cdir.join(&fichero), &texto) {
            return Respuesta::error(502, format!("no se pudo escribir `{fichero}`: {e}"));
        }
        let job = texto
            .lines()
            .find_map(|l| l.strip_prefix("  name: copiar-rehacer-"))
            .map(|h| format!("copiar-rehacer-{h}"))
            .unwrap_or_default();
        match forja.publicar(
            cdir,
            sujeto,
            &format!("Rehacer la copia de {} ({instante})", vistas.join(", ")),
        ) {
            Ok(c) => Respuesta {
                codigo: 202,
                cuerpo: Json::obj([
                    ("package", Json::s(paquete)),
                    ("vistas", Json::Arr(vistas.iter().map(Json::s).collect())),
                    ("instante", Json::s(&instante)),
                    ("job", Json::s(&job)),
                    ("fichero", Json::s(&fichero)),
                    (
                        "encolado",
                        Json::s(format!("encolado como `{fichero}` · commit {c}")),
                    ),
                ]),
            },
            Err(e) => Respuesta::error(502, format!("NO encolado: {e}")),
        }
    }

    /// **Reconstruir en `main`** lo que se fusionó con receta (0044 C.2 ⑤): la
    /// copia de `main`, con todas sus vistas, como tras inducir. La pasada ve
    /// que la definición cambió y lo rehace; `main` sirve lo de antes mientras.
    pub(crate) fn reconstruir_en_main(&self, sujeto: &Identidad) -> String {
        let r = self.leyendo(|raiz| {
            let mut t = vistas_con_copia(raiz);
            t.extend(colecciones_de_todos(raiz));
            Respuesta::ok(Json::s(if t.is_empty() {
                "nada que reconstruir".to_string()
            } else {
                self.encolar_copia(&t, sujeto)
            }))
        });
        match r.cuerpo {
            Json::Str(s) => s,
            otro => otro.jcs(),
        }
    }

    fn encolar_copia(&self, vistas: &[String], sujeto: &Identidad) -> String {
        let Some(forja) = &self.cola else {
            return "NO encolado: este servidor no sabe de ninguna cola (`--cola`); lo rendirá la convergencia".into();
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return format!("NO encolado: {e}"),
        };
        let dir = prestado.ruta();
        let plantilla = match std::fs::read_to_string(dir.join(cola::PLANTILLA_COPIA)) {
            Ok(t) => t,
            Err(_) => {
                return format!(
                    "NO encolado: la cola no trae `{}`; hay que converger este inquilino",
                    cola::PLANTILLA_COPIA
                );
            }
        };
        let rama = rama_de_la_copia();
        let (fichero, texto) = match cola::rendir_copia(&plantilla, vistas, rama.as_deref()) {
            Ok(v) => v,
            Err(e) => return format!("NO encolado: {e}"),
        };
        if let Err(e) = std::fs::write(dir.join(&fichero), &texto) {
            return format!("NO encolado: no se pudo escribir `{fichero}`: {e}");
        }
        if !forja.hay_cambios(dir) {
            return format!("ya encolado como `{fichero}`");
        }
        let en = rama
            .as_deref()
            .map(|r| format!(" en `{r}`"))
            .unwrap_or_default();
        match forja.publicar(dir, sujeto, &format!("Copiar {}{en}", vistas.join(", "))) {
            Ok(c) => format!("encolado como `{fichero}`{en} · commit {c}"),
            Err(e) => format!("NO encolado: {e}"),
        }
    }
}

/// `materialization.payload` autorizado en `conduits.yaml`, que nace con el
/// dueño del paquete si no estaba. Idempotente: si ya está, no escribe. Medido
/// (I2): sin esto un `materialized` no compila (`OOS4011`); y medido (I4b):
/// con `{}` la autorización es ⊥ —sólo `STABLE`— y una vista inducida es
/// `DRAFT` (`OOS4002`), así que admite `oos.maturity: DRAFT`.
///
/// Y con él, **`contextSurface.workspace`** (0031 W3.7 gobierno ②): por dónde
/// sale un dataset hacia el código de un puesto. Nace igual, con lo mismo; un
/// árbol que ya tenía el uno gana el otro. Mientras no esté, la lectura se
/// coteja con `materialization.payload` (`flow::lectura_desde_puesto`).
fn autorizar_conducto(raiz: &Path, dir: &Path, paquete: &str) -> Result<(), Respuesta> {
    let conduits = raiz.join("conduits.yaml");
    match std::fs::read_to_string(&conduits) {
        Ok(t) => {
            let mut nuevo = t.trim_end().to_string();
            for c in [
                "materialization.payload",
                ore_core::flow::CONDUCTO_DEL_PUESTO,
            ] {
                if !t.contains(c) {
                    nuevo.push_str(&format!("\n    {c}: {{ oos.maturity: DRAFT }}"));
                }
            }
            if nuevo != t.trim_end() {
                nuevo.push('\n');
                std::fs::write(&conduits, &nuevo).map_err(|e| {
                    Respuesta::error(500, format!("no se pudo escribir `conduits.yaml`: {e}"))
                })?;
            }
        }
        Err(_) => {
            // El dueño del conducto es el del paquete — y un paquete recién
            // inducido lleva `cambiame` hasta que se conteste `dueno`. Con él
            // no se escribe nada: un `owner: cambiame` en la raíz del árbol
            // no lo re-induce nadie y se quedaría. El conducto nace cuando
            // el dueño esté (la pasada de decisiones vuelve a pasar por aquí).
            let owner = std::fs::read_to_string(dir.join("package.yaml"))
                .ok()
                .and_then(|t| parse::parse(&t).ok())
                .and_then(|n| n.get("spec").and_then(|(_, s)| campo(s, "owner")))
                .filter(|o| o != "cambiame");
            let Some(owner) = owner else {
                return Err(Respuesta::error(
                    409,
                    format!(
                        "`conduits.yaml` no nace hasta que `{paquete}` tenga dueño (la decisión `dueno`): la copia compila cuando se conteste"
                    ),
                ));
            };
            let texto = format!(
                "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: {{ name: {paquete} }}\nspec:\n  owner: {owner}\n  conduits:\n    # 0027 P1: la copia en la celda de las vistas que lo declaren. Admite\n    # DRAFT porque una vista recién inducida lo es y la copia es el registro\n    # del inquilino, no una superficie de consumo. Con retículos propios\n    # (sensibilidad, residencia) aquí se dice hasta qué etiqueta — y eso lo\n    # decide alguien, no esto.\n    materialization.payload: {{ oos.maturity: DRAFT }}\n    # W3.7 gobierno: por dónde sale un dataset hacia el código de un puesto.\n    {}: {{ oos.maturity: DRAFT }}\n",
                ore_core::flow::CONDUCTO_DEL_PUESTO
            );
            std::fs::write(&conduits, &texto).map_err(|e| {
                Respuesta::error(500, format!("no se pudo escribir `conduits.yaml`: {e}"))
            })?;
        }
    }
    Ok(())
}

/// **Lo que se construye en una rama** (0044 C.2 ④): los datasets mantenidos
/// que la rama cambió —su documento difiere del punto del que salió, o no
/// estaba; lo escrito ahora mismo cuenta— y lo que depende de ellos (`from: {
/// dataset }`, hasta el final): lo cambiado y lo afectado, el
/// `state:modified+` de dbt. Lo demás se lee de `main` al día (③).
fn lo_mantenido_de_la_rama(raiz: &Path) -> Vec<String> {
    let base = ["origin/main", "main"]
        .iter()
        .find_map(|m| crate::documentos::git(raiz, &["merge-base", "HEAD", m]))
        .map(|s| s.trim().to_string());
    let cambiados: std::collections::BTreeSet<String> = match &base {
        Some(b) => {
            let mut c: std::collections::BTreeSet<String> =
                crate::documentos::git(raiz, &["diff", "--name-only", b, "--", "packages"])
                    .unwrap_or_default()
                    .lines()
                    .map(String::from)
                    .collect();
            c.extend(
                crate::documentos::git(
                    raiz,
                    &[
                        "ls-files",
                        "--others",
                        "--exclude-standard",
                        "--",
                        "packages",
                    ],
                )
                .unwrap_or_default()
                .lines()
                .map(String::from),
            );
            c
        }
        None => Default::default(),
    };
    // (paquete.dataset, ruta, de qué dataset sale)
    let mut mantenidos: Vec<(String, String, Option<String>)> = Vec::new();
    let Ok(paquetes) = std::fs::read_dir(raiz.join("packages")) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = paquetes
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in &dirs {
        let paquete = d
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        for p in crate::rutas::yamls_del_kind(d, "datasets") {
            let Some(n) = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| parse::parse(&t).ok())
            else {
                continue;
            };
            if campo(&n, "kind").as_deref() != Some("Dataset") {
                continue;
            }
            let Some((_, de)) = n.get("spec").and_then(|(_, s)| s.get("from")) else {
                continue;
            };
            let Some(v) = n.get("metadata").and_then(|(_, m)| campo(m, "name")) else {
                continue;
            };
            let nombre = match n.get("metadata").and_then(|(_, m)| campo(m, "schema")) {
                Some(s) if s != ore_core::normalize::SCHEMA_POR_DEFECTO => format!("{s}.{v}"),
                _ => v,
            };
            let ruta = p
                .strip_prefix(raiz)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            mantenidos.push((format!("{paquete}.{nombre}"), ruta, campo(de, "dataset")));
        }
    }
    let mut suyos: std::collections::BTreeSet<String> = mantenidos
        .iter()
        .filter(|(_, ruta, _)| base.is_none() || cambiados.contains(ruta))
        .map(|(qn, _, _)| qn.clone())
        .collect();
    loop {
        let antes = suyos.len();
        for (qn, _, de) in &mantenidos {
            if de.as_ref().is_some_and(|d| suyos.contains(d)) {
                suyos.insert(qn.clone());
            }
        }
        if suyos.len() == antes {
            break;
        }
    }
    let mut out: Vec<String> = mantenidos
        .into_iter()
        .map(|(qn, _, _)| qn)
        .filter(|qn| suyos.contains(qn))
        .collect();
    // ⭐ D7d: y las colecciones —virtuales y mantenidas— que la rama cambió, o
    //   cuyo `ObjectTable` cambió (lo que depende de ello, como un dataset).
    let tablas: std::collections::BTreeSet<String> = cambiados
        .iter()
        .filter(|r| r.ends_with(".yaml"))
        .filter_map(|r| {
            let n = parse::parse(&std::fs::read_to_string(raiz.join(r)).ok()?).ok()?;
            (campo(&n, "kind").as_deref() == Some("ObjectTable")).then_some(())?;
            let base = r.strip_prefix("packages/")?.split('/').next()?.to_string();
            let (_, m) = n.get("metadata")?;
            let schema = campo(m, "schema")
                .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string());
            Some(format!("{base}.{schema}.{}", campo(m, "name")?))
        })
        .collect();
    for d in &dirs {
        let paquete = d
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        for (nombre, _, ruta, tabla) in colecciones_con_ruta(d) {
            let ruta = ruta
                .strip_prefix(raiz)
                .unwrap_or(&ruta)
                .to_string_lossy()
                .replace('\\', "/");
            let de_tabla = tabla.as_deref().and_then(tres_partes);
            if base.is_none()
                || cambiados.contains(&ruta)
                || de_tabla.is_some_and(|t| tablas.contains(&t))
            {
                out.push(format!("{paquete}.{nombre}"));
            }
        }
    }
    out
}

/// `base.schema.nombre` de una referencia de una, dos o tres partes (dos es
/// `base.nombre`, en el schema de por defecto).
fn tres_partes(r: &str) -> Option<String> {
    let p: Vec<&str> = r.split('.').collect();
    match p.as_slice() {
        [b, s, n] => Some(format!("{b}.{s}.{n}")),
        [b, n] => Some(format!(
            "{b}.{}.{n}",
            ore_core::normalize::SCHEMA_POR_DEFECTO
        )),
        _ => None,
    }
}

/// `paquete.dataset` de cada dataset mantenido del árbol, en orden.
fn vistas_con_copia(raiz: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(paquetes) = std::fs::read_dir(raiz.join("packages")) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = paquetes
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let paquete = d
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        out.extend(
            vistas_con_copia_de(&d)
                .into_iter()
                .map(|v| format!("{paquete}.{v}")),
        );
    }
    out
}

/// Lo que un diagnóstico dice y dónde, junto: para saber si nombra a alguien.
pub(crate) fn texto_de(d: &Json) -> String {
    let Json::Obj(m) = d else {
        return String::new();
    };
    ["donde", "mensaje", "ayuda"]
        .iter()
        .filter_map(|k| match m.get(*k) {
            Some(Json::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// **Las colecciones mantenidas de UN paquete** (`collections/*.yaml` con
/// `from`, 0046 E8·1), por su nombre en el paquete como las vistas —con su
/// schema delante si no es el de por defecto— y si son virtuales.
fn colecciones_de(dir: &Path) -> Vec<(String, bool)> {
    colecciones_con_ruta(dir)
        .into_iter()
        .map(|(n, v, _, _)| (n, v))
        .collect()
}

/// Lo mismo, con dónde vive cada una y de qué `ObjectTable` sale.
fn colecciones_con_ruta(dir: &Path) -> Vec<(String, bool, PathBuf, Option<String>)> {
    let mut out = Vec::new();
    for p in crate::rutas::yamls_del_kind(dir, "collections") {
        let Ok(n) = std::fs::read_to_string(&p)
            .map_err(|_| ())
            .and_then(|t| parse::parse(&t).map_err(|_| ()))
        else {
            continue;
        };
        if campo(&n, "kind").as_deref() != Some("MediaCollection") {
            continue;
        }
        let Some((_, spec)) = n.get("spec") else {
            continue;
        };
        if spec.get("from").is_none() {
            continue;
        }
        let virtual_ = campo(spec, "virtual").as_deref() == Some("true");
        let tabla = spec.get("from").and_then(|(_, f)| campo(f, "objectTable"));
        if let Some(v) = n.get("metadata").and_then(|(_, m)| campo(m, "name")) {
            let nombre = match n.get("metadata").and_then(|(_, m)| campo(m, "schema")) {
                Some(s) if s != ore_core::normalize::SCHEMA_POR_DEFECTO => format!("{s}.{v}"),
                _ => v,
            };
            out.push((nombre, virtual_, p.clone(), tabla));
        }
    }
    out
}

/// Las colecciones mantenidas de todo el árbol, con su paquete delante: lo
/// que el Job de la copia recibe en `VISTAS` junto a los datasets.
fn colecciones_de_todos(raiz: &Path) -> Vec<String> {
    let Ok(paquetes) = std::fs::read_dir(raiz.join("packages")) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = paquetes
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.iter()
        .flat_map(|d| {
            let paquete = d
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            colecciones_de(d)
                .into_iter()
                .map(move |(c, _)| format!("{paquete}.{c}"))
        })
        .collect()
}

/// Los datasets mantenidos de UN paquete (`datasets/*.yaml` con `from`), por
/// nombre y en orden.
fn vistas_con_copia_de(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    // La raíz del paquete y la de cada schema (0038 P5).
    for p in crate::rutas::yamls_del_kind(dir, "datasets") {
        let Ok(texto) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(n) = parse::parse(&texto) else {
            continue;
        };
        if campo(&n, "kind").as_deref() != Some("Dataset") {
            continue;
        }
        if n.get("spec").and_then(|(_, s)| s.get("from")).is_none() {
            continue;
        }
        // En su schema, `<schema>.<nombre>`: con el paquete delante es su forma
        // corta (0038), la del puntero.
        if let Some(v) = n.get("metadata").and_then(|(_, m)| campo(m, "name")) {
            match n.get("metadata").and_then(|(_, m)| campo(m, "schema")) {
                Some(s) if s != ore_core::normalize::SCHEMA_POR_DEFECTO => {
                    out.push(format!("{s}.{v}"))
                }
                _ => out.push(v),
            }
        }
    }
    out
}

/// **La clase de una base** (0027 P1 I4a): `standard` o `foreign`.
///
/// Se lee de `discover.scope.json` —el documento que ya guarda qué entró y de
/// dónde— porque es una REGLA sobre lo que entre después («todo lo que se traiga
/// a esta base se copia»), y las vistas de hoy no pueden decir nada de las de
/// mañana. Ausente = `foreign`: es lo que todas las bases eran antes de que
/// existiera la palabra, y por eso no hay ninguna migración.
///
/// ⭐ 0039: una base que no salió de ningún origen —ni `discover.scope.json` ni
///   `discover.catalog.json`: `create standard database b` en un guion, `ore
///   package new`— es **standard**: lo que tenga sólo puede vivir en el lago.
/// ⭐ 0057: si el paquete es una base foránea de v1alpha27 (`spec.foreign`).
pub(crate) fn es_foranea_v27(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("package.yaml"))
        .ok()
        .and_then(|t| parse::parse(&t).ok())
        .is_some_and(|n| n.get("spec").and_then(|(_, s)| s.get("foreign")).is_some())
}

pub(crate) fn clase_de(dir: &Path) -> &'static str {
    // ⭐ 0057 (v1alpha27): una base con `spec.foreign` lo declara ella.
    if es_foranea_v27(dir) {
        return "foreign";
    }
    if !dir.join("discover.scope.json").is_file() && !dir.join("discover.catalog.json").is_file() {
        return "standard";
    }
    // ⭐ 0057: el catálogo de una fuente, sin alcance, no es una base: `source`.
    if !dir.join("discover.scope.json").is_file() {
        return "source";
    }
    let declarada = std::fs::read_to_string(dir.join("discover.scope.json"))
        .ok()
        .and_then(|t| parse::parse(&t).ok())
        .and_then(|n| campo(&n, "type"));
    match declarada.as_deref() {
        Some("standard") => "standard",
        _ => "foreign",
    }
}

/// **Cuántas copias declara un paquete, y cuántas están hechas.** Los datasets
/// mantenidos son la CONSECUENCIA de la clase; esto los cuenta para que una
/// base estándar con datasets sin copiar se vea como lo que es —una deriva— y
/// no como una tercera clase.
pub(crate) fn copias_de(raiz: &Path, paquete: &str) -> Json {
    let dir = raiz.join("packages").join(paquete);
    let declaradas = vistas_con_copia_de(&dir);
    let copiadas = declaradas
        .iter()
        .filter(|v| {
            ore_core::punteros::leer_en(
                &raiz.join(ore_core::punteros::CARPETA),
                &format!("{paquete}.{v}"),
            )
            .and_then(|(_, n)| campo(&n, "estado"))
            .is_some_and(|e| e == "copiada" || e == "al-dia")
        })
        .count();
    Json::obj([
        ("declaradas", Json::Int(declaradas.len() as i64)),
        ("copiadas", Json::Int(copiadas as i64)),
    ])
}

/// Lo que la última pasada del Job dejó en el puntero
/// (`datasets/<paquete>/default/<nombre>.json`, o el de antes), con quién y
/// cuándo (el commit). Sin puntero: `pendiente` — se decidió y nadie ha
/// copiado todavía.
fn informe_de(raiz: &Path, paquete: &str, vista: &str) -> Json {
    let dir = raiz.join(ore_core::punteros::CARPETA);
    let Some((ruta, _)) = ore_core::punteros::leer_en(&dir, &format!("{paquete}.{vista}")) else {
        return Json::obj([("estado", Json::s("pendiente"))]);
    };
    let rel = ruta
        .strip_prefix(raiz)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let Ok(texto) = std::fs::read_to_string(&ruta) else {
        return Json::obj([("estado", Json::s("pendiente"))]);
    };
    let mut j = match parse::parse(&texto).map(|n| de_node(&n)) {
        Ok(Json::Obj(m)) => m,
        _ => return Json::obj([("estado", Json::s("ilegible")), ("fichero", Json::s(rel))]),
    };
    if let Some((quien, cuando)) = commit_de(raiz, &rel) {
        j.insert("copiado_por".into(), Json::s(quien));
        j.insert("cuando".into(), Json::s(cuando));
    }
    Json::Obj(j)
}

fn commit_de(raiz: &Path, rel: &str) -> Option<(String, String)> {
    let s = std::process::Command::new("git")
        .current_dir(raiz)
        .args(["log", "-1", "--format=%an%x1f%aI", "--", rel])
        .output()
        .ok()?;
    if !s.status.success() {
        return None;
    }
    let texto = String::from_utf8_lossy(&s.stdout);
    let (a, b) = texto.trim().split_once('\u{1f}')?;
    if a.is_empty() {
        return None;
    }
    Some((a.to_string(), b.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Lo que se construye en una rama** (0044 C.2 ④): lo que ella cambió
    /// —aunque aún no esté confirmado— y lo que depende de ello, hasta el
    /// final; lo que no tocó, no.
    #[test]
    fn en_una_rama_se_construye_lo_cambiado_y_lo_afectado() {
        let raiz = std::env::temp_dir().join(format!("ore-serve-rama-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        let d = raiz.join("packages/v/datasets");
        std::fs::create_dir_all(&d).unwrap();
        let git = |args: &[&str]| {
            let s = std::process::Command::new("git")
                .current_dir(&raiz)
                .args(args)
                .env("GIT_AUTHOR_NAME", "s")
                .env("GIT_AUTHOR_EMAIL", "s@x")
                .env("GIT_COMMITTER_NAME", "s")
                .env("GIT_COMMITTER_EMAIL", "s@x")
                .output()
                .unwrap();
            assert!(
                s.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&s.stderr)
            );
        };
        let ds = |n: &str, from: &str| {
            format!(
                "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: {{ name: {n}, namespace: v }}\nspec:\n  owner: team:x\n  from: {{ {from} }}\n"
            )
        };
        std::fs::write(d.join("a.yaml"), ds("a", "table: v.t")).unwrap();
        std::fs::write(d.join("b.yaml"), ds("b", "dataset: v.a")).unwrap();
        std::fs::write(d.join("c.yaml"), ds("c", "dataset: v.b")).unwrap();
        std::fs::write(d.join("x.yaml"), ds("x", "table: v.u")).unwrap();
        git(&["init", "-q", "-b", "main"]);
        git(&["add", "-A"]);
        git(&["commit", "-qm", "main"]);
        git(&["checkout", "-qb", "bea/datos"]);
        // La rama cambia `a` (sin confirmar) y crea `n`, que no depende de nadie.
        std::fs::write(d.join("a.yaml"), ds("a", "table: v.t2")).unwrap();
        std::fs::write(d.join("n.yaml"), ds("n", "table: v.w")).unwrap();
        assert_eq!(lo_mantenido_de_la_rama(&raiz), ["v.a", "v.b", "v.c", "v.n"]);

        // ⭐ D7d: y las colecciones. `main` ya tenía `s3.docs.fotos` (sin tocar)
        //   y `s3.docs.pdfs`, cuyo `ObjectTable` la rama cambia; y la rama da de
        //   alta una base con su colección virtual.
        let s3 = raiz.join("packages/s3/docs");
        let col = |n: &str, t: &str, extra: &str| {
            format!(
                "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: {{ name: {n}, namespace: s3, schema: docs }}\nspec:\n  owner: team:x\n  media: document\n  formats: [pdf]\n  from: {{ objectTable: {t} }}\n{extra}"
            )
        };
        let tabla = |n: &str, prefijo: &str| {
            format!(
                "apiVersion: oos.dev/v1alpha16\nkind: ObjectTable\nmetadata: {{ name: {n}, namespace: s3, schema: docs }}\nspec:\n  owner: team:x\n  prefix: {prefijo}\n"
            )
        };
        git(&["stash", "-u", "-q"]);
        git(&["checkout", "-q", "main"]);
        std::fs::create_dir_all(s3.join("collections")).unwrap();
        std::fs::create_dir_all(s3.join("objects")).unwrap();
        let esquema = |n: &str, base: &str| {
            format!(
                "apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: {{ name: {n}, namespace: {base} }}
"
            )
        };
        std::fs::write(s3.join("schema.yaml"), esquema("docs", "s3")).unwrap();
        std::fs::write(s3.join("objects/t_fotos.yaml"), tabla("t_fotos", "f/")).unwrap();
        std::fs::write(s3.join("objects/t_pdfs.yaml"), tabla("t_pdfs", "p/")).unwrap();
        std::fs::write(
            s3.join("collections/fotos.yaml"),
            col("fotos", "s3.docs.t_fotos", ""),
        )
        .unwrap();
        std::fs::write(
            s3.join("collections/pdfs.yaml"),
            col("pdfs", "s3.docs.t_pdfs", ""),
        )
        .unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "las colecciones de main"]);
        git(&["checkout", "-q", "bea/datos"]);
        git(&["merge", "-q", "main"]);
        git(&["stash", "pop", "-q"]);
        std::fs::write(s3.join("objects/t_pdfs.yaml"), tabla("t_pdfs", "p2/")).unwrap();
        let nueva = raiz.join("packages/legal/archivo/collections");
        std::fs::create_dir_all(&nueva).unwrap();
        std::fs::write(
            raiz.join("packages/legal/archivo/schema.yaml"),
            esquema("archivo", "legal"),
        )
        .unwrap();
        std::fs::write(
            nueva.join("contratos.yaml"),
            col("contratos", "s3.docs.t_pdfs", "  virtual: true\n").replace(
                "namespace: s3, schema: docs",
                "namespace: legal, schema: archivo",
            ),
        )
        .unwrap();
        assert_eq!(
            lo_mantenido_de_la_rama(&raiz),
            [
                "v.a",
                "v.b",
                "v.c",
                "v.n",
                "legal.archivo.contratos",
                "s3.docs.pdfs"
            ]
        );
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// **Lo que se encola por una colección** (0046 E8·1d): una base foránea
    /// que sólo tiene una colección virtual también tiene transacciones que
    /// hacer, con el nombre que el Job busca en `ore view` —paquete, schema y
    /// nombre—; y una colección escrita (sin `from`) no se mantiene.
    #[test]
    fn las_colecciones_mantenidas_van_a_la_cola() {
        let raiz = std::env::temp_dir().join(format!("ore-serve-cola-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        let archivo = raiz.join("packages/legal/archivo");
        std::fs::create_dir_all(archivo.join("collections")).unwrap();
        std::fs::write(
            archivo.join("schema.yaml"),
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: archivo, namespace: legal }\n",
        )
        .unwrap();
        let coleccion = |nombre: &str, resto: &str| {
            format!(
                "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: {{ name: {nombre}, namespace: legal, schema: archivo }}\nspec:\n  owner: team:legal\n  media: document\n  formats: [pdf]\n{resto}"
            )
        };
        std::fs::write(
            archivo.join("collections/contratos.yaml"),
            coleccion(
                "contratos",
                "  from: { objectTable: s3.docs.contratos }\n  virtual: true\n",
            ),
        )
        .unwrap();
        std::fs::write(
            archivo.join("collections/copiados.yaml"),
            coleccion("copiados", "  from: { objectTable: s3.docs.contratos }\n"),
        )
        .unwrap();
        std::fs::write(
            archivo.join("collections/escritos.yaml"),
            coleccion("escritos", ""),
        )
        .unwrap();
        std::fs::write(
            raiz.join("packages/legal/discover.scope.json"),
            r#"{"type": "foreign"}"#,
        )
        .unwrap();

        let dir = raiz.join("packages/legal");
        assert_eq!(clase_de(&dir), "foreign");
        assert_eq!(
            colecciones_de(&dir),
            [
                ("archivo.contratos".to_string(), true),
                ("archivo.copiados".to_string(), false)
            ]
        );
        assert_eq!(
            colecciones_de_todos(&raiz),
            ["legal.archivo.contratos", "legal.archivo.copiados"]
        );
        let _ = std::fs::remove_dir_all(&raiz);
    }
}
