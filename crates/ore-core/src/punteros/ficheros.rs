//! **La puerta sobre un directorio del árbol** (0079 §19.1): lo de antes del
//! corte, y lo de las pruebas y el uso local sin catálogo.
//!
//! El directorio lo prepara quien llama: en una rama, ya superpuesto (lo que la
//! rama no tocó, escrito con lo de `main` al día, y la lista en
//! [`super::AL_DIA`]). Escribir es escribir el fichero; **lo confirma quien
//! publica el árbol** (un `git add -A`), como siempre. La historia sale de
//! `git log`, si el directorio es un árbol de git.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::puerta::{
    Cambio, Contexto, De, Esperado, Fallo, Leido, Punteros, Que, Ramas, Reclamados, Version,
};
use crate::json::Json;
use crate::parse::Node;

/// La puerta sobre la raíz de un árbol.
#[derive(Debug, Clone)]
pub struct Ficheros {
    raiz: PathBuf,
    /// Donde están los punteros: `<raiz>/datasets`, o el `--informe` de la copia.
    carpeta: PathBuf,
    /// Lo que reclaman las demás ramas (`--reclaman`): un directorio con sus
    /// punteros, que monta el Job.
    ajenos: Option<PathBuf>,
}

impl Ficheros {
    /// Sobre `<raiz>/datasets`.
    pub fn en(raiz: &Path) -> Ficheros {
        Ficheros {
            raiz: raiz.to_path_buf(),
            carpeta: raiz.join(super::CARPETA),
            ajenos: None,
        }
    }

    /// Con los punteros en otra carpeta (el `--informe` de la copia).
    pub fn con_carpeta(mut self, carpeta: &Path) -> Ficheros {
        self.carpeta = carpeta.to_path_buf();
        self
    }

    /// Con lo que reclaman las demás ramas. ⛔ Un directorio que no existe es un
    /// error en [`Punteros::reclamados`], no «ninguna rama»: recoger sin lo que
    /// reclaman las demás es borrarlo (D0 M1).
    pub fn con_ajenos(mut self, dir: Option<&Path>) -> Ficheros {
        self.ajenos = dir.map(Path::to_path_buf);
        self
    }

    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    /// La ruta de un fichero, relativa a la raíz y con `/`.
    fn rel(&self, f: &Path) -> Option<String> {
        f.strip_prefix(&self.raiz)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
    }

    /// Lo que dice el [`super::AL_DIA`] del árbol: `(main, rutas → de)`.
    fn al_dia(&self) -> (Option<String>, BTreeMap<String, String>) {
        let Some(n) = std::fs::read_to_string(self.raiz.join(super::AL_DIA))
            .ok()
            .and_then(|t| crate::parse::parse(&t).ok())
        else {
            return (None, BTreeMap::new());
        };
        let main = texto_de(&n, "main").filter(|m| !m.is_empty());
        let rutas = match n.get("rutas") {
            Some((_, Node::Mapping { entries, .. })) => entries
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect(),
            _ => BTreeMap::new(),
        };
        (main, rutas)
    }

    fn leido(&self, f: &Path, texto: String, rutas: &BTreeMap<String, String>) -> Option<Leido> {
        let puntero = crate::parse::parse(&texto).ok()?;
        let de = match self.rel(f) {
            Some(r) if rutas.get(&r).map(String::as_str) == Some("main") => De::Main,
            _ => De::Aqui,
        };
        Some(Leido { puntero, texto, de })
    }

    fn git(&self, args: &[&str]) -> Result<String, Fallo> {
        let o = Command::new("git")
            .current_dir(&self.raiz)
            .args(args)
            .output()
            .map_err(|e| Fallo::Almacen(format!("git no arrancó: {e}")))?;
        if o.status.success() {
            Ok(String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            Err(Fallo::Almacen(format!(
                "`git {}`: {}",
                args.join(" "),
                String::from_utf8_lossy(&o.stderr).trim()
            )))
        }
    }

    /// Lo que hay escrito hoy para `nombre`: la `metadata_location` (para el
    /// *compare-and-set*) y el nodo.
    fn de_hoy(&self, nombre: &str) -> Option<Node> {
        super::leer_en(&self.carpeta, nombre).map(|(_, n)| n)
    }
}

fn texto_de(n: &Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// El puntero que se guarda: su `dataset`, el de su `metadata_location` si se
/// sabe (donde están los bytes: lo que reclama).
fn con_su_dataset(mut m: BTreeMap<String, Json>) -> Json {
    if let Some(Json::Str(ml)) = m.get("metadata_location")
        && let Some(d) = super::dataset_de_ubicacion(ml)
    {
        m.insert("dataset".into(), Json::s(d));
    }
    Json::Obj(m)
}

impl Punteros for Ficheros {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo> {
        if super::partes(nombre).is_none() {
            return Err(Fallo::Nombre(nombre.into()));
        }
        let (_, rutas) = self.al_dia();
        Ok([
            super::ruta_en(&self.carpeta, nombre),
            super::legado_en(&self.carpeta, nombre),
        ]
        .into_iter()
        .flatten()
        .find_map(|f| {
            let t = std::fs::read_to_string(&f).ok()?;
            self.leido(&f, t, &rutas)
        }))
    }

    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        let (_, rutas) = self.al_dia();
        Ok(super::todos_en(&self.carpeta)
            .into_iter()
            .filter_map(|(k, (f, _))| {
                let t = std::fs::read_to_string(&f).ok()?;
                Some((k, self.leido(&f, t, &rutas)?))
            })
            .collect())
    }

    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo> {
        if super::partes(nombre).is_none() {
            return Err(Fallo::Nombre(nombre.into()));
        }
        let rutas: Vec<String> = [
            super::ruta_en(&self.carpeta, nombre),
            super::legado_en(&self.carpeta, nombre),
        ]
        .into_iter()
        .flatten()
        .filter_map(|f| self.rel(&f))
        .collect();
        if rutas.is_empty() || hasta == 0 {
            return Ok(Vec::new());
        }
        // Lo heredado de `main` tiene la historia de `main` (0044 C.2 ③).
        let (main, de) = self.al_dia();
        let desde = match main {
            Some(m)
                if rutas
                    .iter()
                    .any(|r| de.get(r).map(String::as_str) == Some("main")) =>
            {
                m
            }
            _ => "HEAD".to_string(),
        };
        let limite = format!("-{hasta}");
        let mut args = vec![
            "log",
            &limite,
            "--format=%H%x00%cI%x00%an",
            desde.as_str(),
            "--",
        ];
        args.extend(rutas.iter().map(String::as_str));
        let log = self.git(&args)?;
        let commits: Vec<(String, String, String)> = log
            .lines()
            .filter_map(|l| {
                let mut p = l.split('\0');
                Some((p.next()?.into(), p.next()?.into(), p.next()?.into()))
            })
            .collect();
        // De cada commit, el fichero que estuviera: el de su sitio o el de antes.
        let pedidos: Vec<String> = commits
            .iter()
            .flat_map(|(h, _, _)| rutas.iter().map(move |r| format!("{h}:{r}")))
            .collect();
        let textos = cat_file(&self.raiz, &pedidos)?;
        let mut out = Vec::new();
        for (i, (h, cuando, quien)) in commits.into_iter().enumerate() {
            let texto = (0..rutas.len()).find_map(|j| textos[i * rutas.len() + j].clone());
            let Some(texto) = texto else { continue };
            let Ok(puntero) = crate::parse::parse(&texto) else {
                continue;
            };
            out.push(Version {
                id: h,
                cuando,
                quien: Some(quien).filter(|q| !q.is_empty()),
                puntero,
                texto,
            });
        }
        Ok(out)
    }

    fn escribir(&self, cambios: &[Cambio], _ctx: &Contexto) -> Result<(), Fallo> {
        // Primero se comprueba todo; luego se escribe todo.
        let mut hoy = Vec::with_capacity(cambios.len());
        for c in cambios {
            if super::partes(&c.nombre).is_none() {
                return Err(Fallo::Nombre(c.nombre.clone()));
            }
            let previo = self.de_hoy(&c.nombre);
            let ml = previo
                .as_ref()
                .and_then(|n| texto_de(n, "metadata_location"));
            let cumple = match &c.esperado {
                Esperado::Cualquiera => true,
                Esperado::Nada => previo.is_none(),
                Esperado::Ml(m) => ml.as_deref() == Some(m.as_str()),
            };
            if !cumple {
                return Err(Fallo::Conflicto {
                    nombre: c.nombre.clone(),
                    esperado: c.esperado.clone(),
                    hay: ml,
                });
            }
            hoy.push(previo);
        }
        for (c, previo) in cambios.iter().zip(hoy) {
            let aqui = super::ruta_en(&self.carpeta, &c.nombre)
                .ok_or_else(|| Fallo::Nombre(c.nombre.clone()))?;
            let escrito = match &c.que {
                Que::Retirar => None,
                Que::Sustituir(Json::Obj(m)) => Some(con_su_dataset(m.clone())),
                Que::Sustituir(_) => {
                    return Err(Fallo::Almacen(format!(
                        "el puntero de `{}` no es un objeto",
                        c.nombre
                    )));
                }
                Que::Poner { campos, quitar } => {
                    let mut m = match previo.as_ref().map(Json::de_node) {
                        Some(Json::Obj(m)) => m,
                        _ => BTreeMap::new(),
                    };
                    for k in quitar {
                        m.remove(k);
                    }
                    for (k, v) in campos {
                        m.insert(k.clone(), v.clone());
                    }
                    Some(con_su_dataset(m))
                }
            };
            match escrito {
                Some(j) => {
                    if let Some(p) = aqui.parent() {
                        std::fs::create_dir_all(p).map_err(|e| {
                            Fallo::Almacen(format!("no se pudo crear `{}`: {e}", p.display()))
                        })?;
                    }
                    std::fs::write(&aqui, j.pretty() + "\n").map_err(|e| {
                        Fallo::Almacen(format!("no se pudo escribir `{}`: {e}", aqui.display()))
                    })?;
                    super::retirar_legado(&self.carpeta, &c.nombre, &aqui);
                }
                None => {
                    for f in [Some(aqui), super::legado_en(&self.carpeta, &c.nombre)]
                        .into_iter()
                        .flatten()
                    {
                        if f.is_file() {
                            std::fs::remove_file(&f).map_err(|e| {
                                Fallo::Almacen(format!("no se pudo borrar `{}`: {e}", f.display()))
                            })?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        let mut r = Reclamados::default();
        // Los de este árbol: los punteros, las copias de antes y los
        // resultados de las funciones (sus informes nombran su dataset).
        let mut dirs: Vec<(PathBuf, &str)> = ["datasets", "copias", "resultados"]
            .into_iter()
            .map(|d| (self.raiz.join(d), d))
            .collect();
        if !dirs.iter().any(|(d, _)| *d == self.carpeta) {
            dirs.push((self.carpeta.clone(), super::CARPETA));
        }
        if let Some(a) = &self.ajenos {
            if !a.is_dir() {
                return Err(Fallo::Almacen(format!(
                    "`--reclaman {}` no es un directorio: sin lo que reclaman las demás ramas no se recoge",
                    a.display()
                )));
            }
            dirs.push((a.clone(), super::CARPETA));
        }
        for (dir, clase) in dirs {
            for f in super::ficheros(&dir) {
                let Some(n) = std::fs::read_to_string(&f)
                    .ok()
                    .and_then(|t| crate::parse::parse(&t).ok())
                else {
                    continue;
                };
                let ml = texto_de(&n, "metadata_location");
                // El nombre lo dice el puntero; si no, su `metadata_location`; si
                // no, el de siempre (`<clase>/<stem>`).
                let d = texto_de(&n, "dataset")
                    .or_else(|| ml.as_deref().and_then(super::dataset_de_ubicacion))
                    .or_else(|| {
                        let s = f.file_stem()?.to_str()?;
                        Some(format!("{clase}/{s}"))
                    });
                if let Some(d) = d {
                    if let Some(ml) = ml {
                        if texto_de(&n, "kind").as_deref() == Some("MediaCollection") {
                            r.colecciones.insert((d.clone(), ml.clone()));
                        }
                        r.ubicaciones.entry(d.clone()).or_default().insert(ml);
                    }
                    r.datasets.insert(d);
                }
                // El informe de una corrida nombra su dataset dentro de `resultado`.
                if let Some(d) = n.get("resultado").and_then(|(_, x)| texto_de(x, "dataset")) {
                    r.datasets.insert(d);
                }
                if let Some(c) = texto_de(&n, "clave") {
                    r.claves.insert(c);
                }
            }
        }
        Ok(r)
    }

    fn version(&self) -> Result<String, Fallo> {
        let head = self.git(&["rev-parse", "HEAD"])?;
        let (main, _) = self.al_dia();
        Ok(match main {
            Some(m) => format!("{}+{m}", head.trim()),
            None => head.trim().to_string(),
        })
    }
}

/// En git, crear, borrar y traer una rama ya mueve sus punteros.
impl Ramas for Ficheros {
    fn crear(&self, _rama: &str, _desde: &str) -> Result<(), Fallo> {
        Ok(())
    }
    fn borrar(&self, _rama: &str) -> Result<(), Fallo> {
        Ok(())
    }
    fn traer(&self, _rama: &str) -> Result<(), Fallo> {
        Ok(())
    }
}

/// Los textos de `<commit>:<ruta>` de una vez (`git cat-file --batch`), en el
/// orden pedido; `None` donde no existe.
fn cat_file(raiz: &Path, pedidos: &[String]) -> Result<Vec<Option<String>>, Fallo> {
    use std::io::Write;
    if pedidos.is_empty() {
        return Ok(Vec::new());
    }
    let mut hijo = Command::new("git")
        .current_dir(raiz)
        .args(["cat-file", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Fallo::Almacen(format!("git no arrancó: {e}")))?;
    let mut entrada = hijo
        .stdin
        .take()
        .ok_or_else(|| Fallo::Almacen("git sin stdin".into()))?;
    let todo: String = pedidos.iter().map(|p| format!("{p}\n")).collect();
    // En otro hilo: con muchas versiones, la salida llena su tubo antes de que
    // acabe la entrada.
    let escritor = std::thread::spawn(move || {
        let _ = entrada.write_all(todo.as_bytes());
    });
    let salida = hijo
        .wait_with_output()
        .map_err(|e| Fallo::Almacen(format!("git cat-file: {e}")))?;
    let _ = escritor.join();
    let b = salida.stdout;
    let mut out = Vec::with_capacity(pedidos.len());
    let mut i = 0;
    while out.len() < pedidos.len() && i < b.len() {
        let Some(fin) = b[i..].iter().position(|&c| c == b'\n') else {
            break;
        };
        let cabecera = String::from_utf8_lossy(&b[i..i + fin]).into_owned();
        i += fin + 1;
        let partes: Vec<&str> = cabecera.split(' ').collect();
        match partes.as_slice() {
            [_, "blob", n] => {
                let n: usize = n
                    .parse()
                    .map_err(|_| Fallo::Almacen(format!("git cat-file: `{cabecera}`")))?;
                let fin = (i + n).min(b.len());
                out.push(Some(String::from_utf8_lossy(&b[i..fin]).into_owned()));
                i = fin + 1;
            }
            _ => out.push(None),
        }
    }
    out.resize(pedidos.len(), None);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ore-ficheros-{nombre}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn git(d: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .current_dir(d)
            .args(args)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn repo(nombre: &str) -> PathBuf {
        let d = tmp(nombre);
        git(&d, &["init", "-q", "-b", "main"]);
        git(&d, &["config", "user.name", "quien"]);
        git(&d, &["config", "user.email", "quien@invalido"]);
        git(&d, &["config", "commit.gpgsign", "false"]);
        d
    }

    fn confirmar(d: &Path, m: &str) -> String {
        git(d, &["add", "-A"]);
        git(d, &["commit", "-q", "--allow-empty", "-m", m]);
        git(d, &["rev-parse", "HEAD"])
    }

    fn ml(n: u32) -> String {
        format!("gs://b/ore/v2/catalogo/ventas/default/pedidos/metadata/{n:05}-x.metadata.json")
    }

    fn poner(nombre: &str, campos: &[(&str, Json)], esperado: Esperado) -> Cambio {
        Cambio {
            nombre: nombre.into(),
            que: Que::Poner {
                campos: campos
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
                quitar: Vec::new(),
            },
            esperado,
        }
    }

    fn campo(l: &Leido, k: &str) -> Option<String> {
        texto_de(&l.puntero, k)
    }

    #[test]
    fn escribir_pone_encima_y_el_dataset_sale_de_su_metadata() {
        let d = tmp("poner");
        let p = Ficheros::en(&d);
        let ctx = Contexto::default();
        p.escribir(
            &[poner(
                "ventas.pedidos",
                &[
                    ("metadata_location", Json::s(ml(1))),
                    ("estado", Json::s("copiada")),
                    ("motivo", Json::s("x")),
                ],
                Esperado::Nada,
            )],
            &ctx,
        )
        .unwrap();
        assert!(d.join("datasets/ventas/default/pedidos.json").is_file());
        let l = p.uno("ventas.pedidos").unwrap().unwrap();
        assert_eq!(
            campo(&l, "dataset").as_deref(),
            Some("catalogo/ventas/default/pedidos")
        );
        assert_eq!(l.de, De::Aqui);
        // Encima de lo que había: lo que no se nombra se queda; lo quitado, no.
        p.escribir(
            &[Cambio {
                nombre: "ventas.pedidos".into(),
                que: Que::Poner {
                    campos: vec![("metadata_location".into(), Json::s(ml(2)))],
                    quitar: vec!["motivo".into()],
                },
                esperado: Esperado::Ml(ml(1)),
            }],
            &ctx,
        )
        .unwrap();
        let l = p.uno("ventas.pedidos").unwrap().unwrap();
        assert_eq!(campo(&l, "estado").as_deref(), Some("copiada"));
        assert_eq!(campo(&l, "motivo"), None);
        assert_eq!(campo(&l, "metadata_location"), Some(ml(2)));
    }

    #[test]
    fn el_compare_and_set_es_de_todos_o_ninguno() {
        let d = tmp("cas");
        let p = Ficheros::en(&d);
        let ctx = Contexto::default();
        p.escribir(
            &[poner(
                "ventas.a",
                &[("metadata_location", Json::s(ml(1)))],
                Esperado::Nada,
            )],
            &ctx,
        )
        .unwrap();
        // Dos tablas; la segunda no cumple: ninguna se escribe.
        let e = p
            .escribir(
                &[
                    poner(
                        "ventas.b",
                        &[("metadata_location", Json::s(ml(5)))],
                        Esperado::Nada,
                    ),
                    poner(
                        "ventas.a",
                        &[("metadata_location", Json::s(ml(3)))],
                        Esperado::Ml(ml(2)),
                    ),
                ],
                &ctx,
            )
            .unwrap_err();
        assert_eq!(e.codigo(), 409);
        assert!(matches!(&e, Fallo::Conflicto { hay: Some(h), .. } if *h == ml(1)));
        assert!(p.uno("ventas.b").unwrap().is_none());
        assert_eq!(
            campo(&p.uno("ventas.a").unwrap().unwrap(), "metadata_location"),
            Some(ml(1))
        );
        // Nacer sobre algo que ya está, también.
        let e = p
            .escribir(&[poner("ventas.a", &[], Esperado::Nada)], &ctx)
            .unwrap_err();
        assert_eq!(e.codigo(), 409);
        // Un nombre que no lo es: 422.
        let e = p
            .escribir(&[poner("sin_base", &[], Esperado::Cualquiera)], &ctx)
            .unwrap_err();
        assert_eq!(e.codigo(), 422);
    }

    #[test]
    fn el_de_antes_se_lee_y_al_escribir_se_retira() {
        let d = tmp("legado");
        std::fs::create_dir_all(d.join("datasets")).unwrap();
        std::fs::write(
            d.join("datasets/ventas_pedidos.json"),
            format!(
                "{{\"metadata_location\": \"{}\", \"dataset\": \"datasets/ventas_pedidos\"}}\n",
                ml(1)
            ),
        )
        .unwrap();
        let p = Ficheros::en(&d);
        let l = p.uno("ventas.pedidos").unwrap().unwrap();
        assert_eq!(campo(&l, "metadata_location"), Some(ml(1)));
        assert_eq!(p.todos().unwrap().len(), 1);
        // Sustituir sin `metadata_location`: el `dataset` que diga se queda.
        p.escribir(
            &[Cambio {
                nombre: "ventas.pedidos".into(),
                que: Que::Sustituir(Json::obj([
                    ("dataset", Json::s("datasets/ventas_pedidos")),
                    ("estado", Json::s("error")),
                ])),
                esperado: Esperado::Ml(ml(1)),
            }],
            &Contexto::default(),
        )
        .unwrap();
        assert!(!d.join("datasets/ventas_pedidos.json").exists());
        let l = p.uno("ventas.pedidos").unwrap().unwrap();
        assert_eq!(
            campo(&l, "dataset").as_deref(),
            Some("datasets/ventas_pedidos")
        );
        assert_eq!(campo(&l, "metadata_location"), None);
        // Retirar: ni el de su sitio ni el de antes.
        p.escribir(
            &[Cambio {
                nombre: "ventas.pedidos".into(),
                que: Que::Retirar,
                esperado: Esperado::Cualquiera,
            }],
            &Contexto::default(),
        )
        .unwrap();
        assert!(p.uno("ventas.pedidos").unwrap().is_none());
        assert!(p.todos().unwrap().is_empty());
    }

    #[test]
    fn lo_heredado_de_main_lo_dice_y_tiene_la_historia_de_main() {
        let d = repo("historia");
        let p = Ficheros::en(&d);
        let ctx = Contexto::default();
        let mut shas = Vec::new();
        for n in 1..=3 {
            p.escribir(
                &[poner(
                    "ventas.pedidos",
                    &[("metadata_location", Json::s(ml(n)))],
                    Esperado::Cualquiera,
                )],
                &ctx,
            )
            .unwrap();
            shas.push(confirmar(&d, &format!("v{n}")));
        }
        let h = p.historia("ventas.pedidos", 10).unwrap();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].id, shas[2]);
        assert_eq!(texto_de(&h[0].puntero, "metadata_location"), Some(ml(3)));
        assert_eq!(texto_de(&h[2].puntero, "metadata_location"), Some(ml(1)));
        assert_eq!(h[0].quien.as_deref(), Some("quien"));
        assert_eq!(p.historia("ventas.pedidos", 2).unwrap().len(), 2);

        // Una rama que salió en v1 y no lo tocó: el árbol superpuesto dice que
        // lo lee de main, y su historia es la de main (v3, v2, v1).
        git(&d, &["checkout", "-q", "-b", "r", &shas[0]]);
        std::fs::write(
            d.join("datasets/ventas/default/pedidos.json"),
            format!("{{\"metadata_location\": \"{}\"}}\n", ml(3)),
        )
        .unwrap();
        let al_dia = Json::obj([
            ("main", Json::s(&shas[2])),
            ("base", Json::s(&shas[0])),
            (
                "rutas",
                Json::Obj(
                    [(
                        "datasets/ventas/default/pedidos.json".to_string(),
                        Json::s("main"),
                    )]
                    .into(),
                ),
            ),
        ]);
        std::fs::write(d.join(super::super::AL_DIA), al_dia.pretty()).unwrap();
        let r = Ficheros::en(&d);
        assert_eq!(r.uno("ventas.pedidos").unwrap().unwrap().de, De::Main);
        assert_eq!(r.historia("ventas.pedidos", 10).unwrap().len(), 3);
        assert_eq!(
            r.version().unwrap(),
            format!("{}+{}", shas[0], shas[2]),
            "la versión de una rama son su commit y el de main"
        );
        // Sin el fichero (un árbol sin superponer), la de la rama: solo v1.
        std::fs::remove_file(d.join(super::super::AL_DIA)).unwrap();
        assert_eq!(r.historia("ventas.pedidos", 10).unwrap().len(), 1);
        assert_eq!(r.uno("ventas.pedidos").unwrap().unwrap().de, De::Aqui);
    }

    #[test]
    fn la_historia_sigue_al_de_antes() {
        let d = repo("historia-legado");
        std::fs::create_dir_all(d.join("datasets")).unwrap();
        std::fs::write(
            d.join("datasets/ventas_pedidos.json"),
            format!("{{\"metadata_location\": \"{}\"}}\n", ml(1)),
        )
        .unwrap();
        confirmar(&d, "de antes");
        let p = Ficheros::en(&d);
        p.escribir(
            &[poner(
                "ventas.pedidos",
                &[("metadata_location", Json::s(ml(2)))],
                Esperado::Ml(ml(1)),
            )],
            &Contexto::default(),
        )
        .unwrap();
        confirmar(&d, "a su sitio");
        let h = p.historia("ventas.pedidos", 10).unwrap();
        assert_eq!(h.len(), 2, "las dos: la de su sitio y la de antes");
        assert_eq!(texto_de(&h[1].puntero, "metadata_location"), Some(ml(1)));
    }

    #[test]
    fn sin_git_no_hay_historia_ni_version() {
        let d = tmp("sin-git");
        let p = Ficheros::en(&d);
        assert_eq!(p.version().unwrap_err().codigo(), 503);
    }

    #[test]
    fn reclamados_junta_lo_de_aqui_y_lo_de_las_demas_ramas() {
        let d = tmp("reclamados");
        let ajenos = tmp("ajenos");
        let p = Ficheros::en(&d).con_ajenos(Some(&ajenos));
        p.escribir(
            &[poner(
                "ventas.pedidos",
                &[
                    ("metadata_location", Json::s(ml(1))),
                    ("clave", Json::s("k1")),
                ],
                Esperado::Nada,
            )],
            &Contexto::default(),
        )
        .unwrap();
        std::fs::create_dir_all(d.join("resultados")).unwrap();
        std::fs::write(
            d.join("resultados/f_1.json"),
            "{\"resultado\": {\"dataset\": \"resultados/f\"}}\n",
        )
        .unwrap();
        std::fs::create_dir_all(ajenos.join("r/datasets/ventas/default")).unwrap();
        std::fs::write(
            ajenos.join("r/datasets/ventas/default/pedidos.json"),
            format!(
                "{{\"metadata_location\": \"{}\", \"kind\": \"MediaCollection\"}}\n",
                ml(7)
            ),
        )
        .unwrap();
        let r = p.reclamados().unwrap();
        let ds = "catalogo/ventas/default/pedidos".to_string();
        assert!(r.datasets.contains(&ds));
        assert!(r.datasets.contains("resultados/f"));
        assert!(r.claves.contains("k1"));
        assert_eq!(
            r.ubicaciones[&ds],
            [ml(1), ml(7)].into_iter().collect(),
            "lo de aquí y lo de la otra rama"
        );
        assert!(r.colecciones.contains(&(ds, ml(7))));
        // ⛔ Un `--reclaman` que no está es un error, no «ninguna rama».
        let p = Ficheros::en(&d).con_ajenos(Some(&d.join("no-esta")));
        assert_eq!(p.reclamados().unwrap_err().codigo(), 503);
    }
}
