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

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
}

/// El fichero de un documento `kind` con ese nombre, bajo `packages/<n>/<dir>/`.
fn fichero_de(dir: &Path, kind: &str, nombre: &str) -> Option<(PathBuf, String, Node)> {
    let es = std::fs::read_dir(dir).ok()?;
    let mut rutas: Vec<PathBuf> = es
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    rutas.sort();
    for p in rutas {
        let Ok(texto) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(n) = parse::parse(&texto) else {
            continue;
        };
        if campo(&n, "kind").as_deref() != Some(kind) {
            continue;
        }
        let meta = n.get("metadata").map(|(_, m)| m);
        if meta.and_then(|m| campo(m, "name")).as_deref() == Some(nombre) {
            return Some((p, texto, n));
        }
    }
    None
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
        // Hay algo que copiar si la base es estándar O alguna vista declara
        // copia una a una (una foránea con tablas copiadas).
        if clase_de(&dir) != "standard" && vistas_con_copia_de(&dir).is_empty() {
            return campos;
        }
        if let Err(r) = autorizar_conducto(raiz, &dir, paquete) {
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
        let todas = vistas_con_copia(raiz);
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
                let clave = tabla
                    .as_deref()
                    .and_then(|t| {
                        // Las tablas de su mismo schema: la carpeta hermana.
                        fichero_de(
                            &p.parent()
                                .and_then(Path::parent)
                                .unwrap_or(&dir)
                                .join("tables"),
                            "Table",
                            t.rsplit('.').next().unwrap_or(t),
                        )
                    })
                    .and_then(|(_, _, tn)| {
                        tn.get("spec")
                            .and_then(|(_, s)| s.get("changes"))
                            .and_then(|(_, c)| c.get("key"))
                            .map(|(_, k)| de_node(k))
                    })
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
        let (fichero, texto) = match cola::rendir_rehacer(&plantilla, &vistas, &instante) {
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
        let (fichero, texto) = match cola::rendir_copia(&plantilla, vistas) {
            Ok(v) => v,
            Err(e) => return format!("NO encolado: {e}"),
        };
        if let Err(e) = std::fs::write(dir.join(&fichero), &texto) {
            return format!("NO encolado: no se pudo escribir `{fichero}`: {e}");
        }
        if !forja.hay_cambios(dir) {
            return format!("ya encolado como `{fichero}`");
        }
        match forja.publicar(dir, sujeto, &format!("Copiar {}", vistas.join(", "))) {
            Ok(c) => format!("encolado como `{fichero}` · commit {c}"),
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
pub(crate) fn clase_de(dir: &Path) -> &'static str {
    if !dir.join("discover.scope.json").is_file() && !dir.join("discover.catalog.json").is_file() {
        return "standard";
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
