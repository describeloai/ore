//! **El árbol de la fuente, al día con `changes.list`** (ADR 0082, paso 4).
//!
//! Listar una carpeta de Drive es una petición por carpeta: con un árbol grande,
//! cada `listar` de un proceso que vive (`ore-medios`, `servir`) las repite
//! todas. Drive da el *feed* de cambios (X0 E1): un `startPageToken` y, desde él,
//! sólo lo que cambió. Aquí:
//!
//! 1. la primera vez, se pide el token **antes** de recorrer (lo que cambie
//!    mientras se recorre se repite después, y repetir un cambio no estropea
//!    nada) y se recorre el ámbito entero;
//! 2. después, una sola petición (`changes.list` desde el token guardado) y se
//!    aplica lo que trae al índice: un fichero nuevo o cambiado se anota, uno
//!    borrado, en la papelera, o movido fuera del ámbito se quita —con todo lo
//!    que colgaba de él—, y una carpeta que entra se recorre entera, porque sus
//!    hijos no cambiaron y el feed no los menciona;
//! 3. si el *feed* falla (un token caducado, un `400`), se tira el índice y se
//!    vuelve al recorrido completo: nunca se sirve un árbol dudoso.
//!
//! El *feed* es el de **todo lo que la cuenta ve**, no el de la carpeta: un
//! cambio en otra carpeta compartida se mira y se ignora. Y vive en el proceso:
//! un driver de una sola pasada (`catalogo`, `versiones`) paga una petición de
//! más por el token y nada más; lo gana quien se queda (`ore-medios`, `servir`).

use crate::{CAMPOS, Clase, Fallo, Gdrive, Item, codificar, con_rutas};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

static COMPLETOS: AtomicUsize = AtomicUsize::new(0);
static INCREMENTALES: AtomicUsize = AtomicUsize::new(0);

/// Cuántos árboles se armaron recorriendo entero y cuántas veces se puso al
/// día con el *feed* de cambios.
pub fn recorridos() -> (usize, usize) {
    (COMPLETOS.load(Relaxed), INCREMENTALES.load(Relaxed))
}

/// Una entrada del *feed* de cambios.
#[derive(Debug, Clone)]
pub struct Cambio {
    pub id: String,
    /// El fichero ya no se ve (borrado, o la cuenta perdió el acceso).
    pub quitado: bool,
    pub item: Option<Item>,
}

/// **El *feed* entero desde un token**, página a página: `pagina(token)` pide
/// una y devuelve su JSON. Devuelve los cambios en orden y el token nuevo
/// (`newStartPageToken`). Fuera de la red para probarlo.
pub fn paginar_cambios(
    mut pagina: impl FnMut(&str) -> Result<String, String>,
    desde: &str,
) -> Result<(Vec<Cambio>, String), String> {
    let mut cambios = Vec::new();
    let mut token = desde.to_string();
    loop {
        let texto = pagina(&token)?;
        let n = ore_base::parse::parse(&texto)
            .map_err(|e| format!("the Drive changes feed does not parse: {e:?}"))?;
        if let Some((_, cs)) = n.get("changes") {
            for c in cs.items() {
                let id = c
                    .get("fileId")
                    .and_then(|(_, v)| v.as_str().map(String::from))
                    .unwrap_or_default();
                let quitado = c.get("removed").and_then(|(_, v)| v.as_str()) == Some("true");
                let item = c.get("file").map(|(_, f)| Item::de(f));
                cambios.push(Cambio { id, quitado, item });
            }
        }
        if let Some(nuevo) = n
            .get("newStartPageToken")
            .and_then(|(_, v)| v.as_str().map(String::from))
            .filter(|t| !t.is_empty())
        {
            return Ok((cambios, nuevo));
        }
        token = n
            .get("nextPageToken")
            .and_then(|(_, v)| v.as_str().map(String::from))
            .filter(|t| !t.is_empty())
            .ok_or("the Drive changes feed gave neither `nextPageToken` nor `newStartPageToken`")?;
    }
}

/// Lo que hay en el ámbito de la fuente, con el token desde el que seguirlo.
#[derive(Debug)]
pub struct Indice {
    /// La carpeta del ámbito (la del prefijo de la fuente).
    raiz: String,
    /// Su ruta bajo la raíz de la fuente: vacía o acabada en `/`.
    base: String,
    token: String,
    /// Todo lo que cuelga de ella (carpetas, ficheros, accesos directos…), por ID.
    nodos: HashMap<String, Item>,
}

/// El árbol con sus rutas armadas.
#[derive(Debug, Default)]
pub struct Vista {
    pub items: Vec<Item>,
    pub saltados: usize,
    /// Las carpetas, con su ruta acabada en `/`.
    pub carpetas: HashSet<String>,
}

impl Indice {
    pub fn nuevo(raiz: &str, base: &str, token: &str, items: Vec<Item>) -> Indice {
        Indice {
            raiz: raiz.to_string(),
            base: base.to_string(),
            token: token.to_string(),
            nodos: items.into_iter().map(|i| (i.id.clone(), i)).collect(),
        }
    }

    fn contiene(&self, carpeta: &str) -> bool {
        carpeta == self.raiz
            || self
                .nodos
                .get(carpeta)
                .is_some_and(|n| n.clase == Clase::Carpeta)
    }

    /// Quita un nodo y todo lo que colgaba de él.
    fn quitar(&mut self, id: &str) {
        let mut fuera: HashSet<String> = HashSet::from([id.to_string()]);
        loop {
            let mas: Vec<String> = self
                .nodos
                .values()
                .filter(|n| fuera.contains(&n.padre) && !fuera.contains(&n.id))
                .map(|n| n.id.clone())
                .collect();
            if mas.is_empty() {
                break;
            }
            fuera.extend(mas);
        }
        for f in fuera {
            self.nodos.remove(&f);
        }
    }

    /// Aplica un cambio. Devuelve la carpeta que **entró** en el ámbito, si es
    /// el caso: sus hijos no cambiaron y el *feed* no los dice, hay que
    /// recorrerla.
    pub fn aplicar(&mut self, c: Cambio) -> Option<String> {
        let Some(i) = c.item.filter(|_| !c.quitado) else {
            self.quitar(&c.id);
            return None;
        };
        if i.papelera || !self.contiene(&i.padre) {
            self.quitar(&c.id);
            return None;
        }
        let nueva = i.clase == Clase::Carpeta && !self.nodos.contains_key(&i.id);
        let id = i.id.clone();
        self.nodos.insert(id.clone(), i);
        nueva.then_some(id)
    }

    /// El árbol, con cada ruta única entre hermanos (los repetidos llevan su ID).
    pub fn rutas(&self) -> Vista {
        let mut por_padre: HashMap<&str, Vec<Item>> = HashMap::new();
        for n in self.nodos.values() {
            por_padre
                .entry(n.padre.as_str())
                .or_default()
                .push(n.clone());
        }
        let mut v = Vista::default();
        v.carpetas.insert(self.base.clone());
        let mut pendientes = vec![(self.raiz.clone(), self.base.clone())];
        while let Some((id, base)) = pendientes.pop() {
            for h in con_rutas(por_padre.remove(id.as_str()).unwrap_or_default(), &base) {
                match h.clase {
                    Clase::Carpeta => {
                        let dir = format!("{}/", h.ruta);
                        v.carpetas.insert(dir.clone());
                        pendientes.push((h.id.clone(), dir));
                    }
                    Clase::Fichero | Clase::Nativo(_) => v.items.push(h),
                    Clase::Acceso | Clase::Otro => v.saltados += 1,
                }
            }
        }
        v.items.sort_by(|a, b| a.ruta.cmp(&b.ruta));
        v
    }
}

impl Gdrive {
    /// El token desde el que se siguen los cambios, si ya hay árbol armado (las
    /// pruebas miran que avance).
    pub fn token_de_seguimiento(&self) -> Option<String> {
        self.indice
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|ix| ix.token.clone())
    }

    /// El token desde el que Drive cuenta los cambios, ahora.
    fn token_de_cambios(&self) -> Result<String, Fallo> {
        let url = format!(
            "{}/drive/v3/changes/startPageToken?supportsAllDrives=true",
            self.fuente.endpoint
        );
        let t = self.texto(&url)?;
        ore_base::parse::parse(&t)
            .ok()
            .and_then(|n| {
                n.get("startPageToken")
                    .and_then(|(_, v)| v.as_str().map(String::from))
            })
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Fallo::propio("Drive gave no `startPageToken`"))
    }

    /// Todo lo que cuelga de una carpeta, sin rutas: lo que el índice anota.
    fn descendientes(&self, carpeta: &str) -> Result<Vec<Item>, Fallo> {
        let mut pendientes = vec![carpeta.to_string()];
        let mut vistas = HashSet::new();
        let mut todo = Vec::new();
        while let Some(id) = pendientes.pop() {
            if !vistas.insert(id.clone()) {
                continue;
            }
            let q = format!("{} in parents and trashed=false", crate::literal(&id));
            for h in self.lista(&q)? {
                if h.clase == Clase::Carpeta {
                    pendientes.push(h.id.clone());
                }
                todo.push(h);
            }
        }
        Ok(todo)
    }

    /// Pone el índice al día con el *feed*, y recorre las carpetas que entraron.
    fn actualizar(&self, ix: &mut Indice) -> Result<(), Fallo> {
        let url = |t: &str| {
            format!(
                "{}/drive/v3/changes?pageToken={}&pageSize=1000&includeRemoved=true&supportsAllDrives=true&includeItemsFromAllDrives=true&spaces=drive&fields={}",
                self.fuente.endpoint,
                codificar(t),
                codificar(&format!(
                    "nextPageToken,newStartPageToken,changes(fileId,removed,file({CAMPOS}))"
                ))
            )
        };
        let (cambios, nuevo) =
            paginar_cambios(|t| self.texto(&url(t)).map_err(|f| f.motivo()), &ix.token)
                .map_err(Fallo::propio)?;
        let mut nuevas = Vec::new();
        for c in cambios {
            nuevas.extend(ix.aplicar(c));
        }
        for id in nuevas {
            // Puede haberse quitado después, en el mismo feed.
            if ix.nodos.contains_key(&id) {
                for d in self.descendientes(&id)? {
                    ix.nodos.insert(d.id.clone(), d);
                }
            }
        }
        ix.token = nuevo;
        Ok(())
    }

    /// **El árbol del ámbito de la fuente, al día.**
    fn al_dia(&self) -> Result<Vista, Fallo> {
        let mut g = self.indice.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ix) = g.as_mut() {
            match self.actualizar(ix) {
                Ok(()) => {
                    INCREMENTALES.fetch_add(1, Relaxed);
                    return Ok(ix.rutas());
                }
                // Un token caducado o un feed que falla: se tira y se recorre.
                Err(_) => *g = None,
            }
        }
        let token = self.token_de_cambios()?;
        let ambito = self.fuente.prefijo.trim_end_matches('/');
        let raiz = self.carpeta_de(ambito)?;
        let items = self.descendientes(&raiz)?;
        let ix = Indice::nuevo(&raiz, &self.fuente.prefijo, &token, items);
        COMPLETOS.fetch_add(1, Relaxed);
        let v = ix.rutas();
        *g = Some(ix);
        Ok(v)
    }

    /// **Todo lo que hay bajo una ruta**, recursivo: los ficheros y los nativos
    /// con su ruta, y cuántos accesos directos y otros se saltaron (de todo el
    /// ámbito de la fuente). La primera vez recorre; después sigue el *feed* de
    /// cambios de Drive. Lo que cae fuera del ámbito de la fuente (el prefijo de
    /// su URL) se recorre sin índice.
    pub fn recorrer(&self, ruta: &str) -> Result<(Vec<Item>, usize), Fallo> {
        let ruta = ruta.trim_matches('/');
        let dir = if ruta.is_empty() {
            String::new()
        } else {
            format!("{ruta}/")
        };
        if !dir.starts_with(&self.fuente.prefijo) {
            return self.recorrer_completo(ruta);
        }
        let v = self.al_dia()?;
        if !v.carpetas.contains(&dir) {
            return Err(Fallo::nuevo(
                404,
                "notFound",
                &format!("the folder `{ruta}` of `{ruta}` is not in the Drive folder"),
            ));
        }
        {
            let mut c = self.conocidos.lock().unwrap_or_else(|e| e.into_inner());
            c.clear();
            for i in &v.items {
                c.insert(i.ruta.clone(), i.clone());
            }
        }
        let items = v
            .items
            .into_iter()
            .filter(|i| i.ruta.starts_with(&dir))
            .collect();
        Ok((items, v.saltados))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn item(id: &str, nombre: &str, tipo: &str, padre: &str) -> Item {
        Item::de(
            &ore_base::parse::parse(&format!(
                r#"{{"id":"{id}","name":"{nombre}","mimeType":"{tipo}","parents":["{padre}"],"size":"3","md5Checksum":"344fd2323dd33d5c058b7cf27de029e8","headRevisionId":"r-{id}","version":"1"}}"#
            ))
            .unwrap(),
        )
    }
    const CARP: &str = "application/vnd.google-apps.folder";
    const CSV: &str = "text/csv";

    fn base() -> Indice {
        Indice::nuevo(
            "RAIZ",
            "",
            "10",
            vec![
                item("a", "a.csv", CSV, "RAIZ"),
                item("sub", "sub", CARP, "RAIZ"),
                item("c", "c.csv", CSV, "sub"),
                item(
                    "acc",
                    "atajo",
                    "application/vnd.google-apps.shortcut",
                    "RAIZ",
                ),
            ],
        )
    }

    fn rutas(ix: &Indice) -> Vec<String> {
        ix.rutas().items.into_iter().map(|i| i.ruta).collect()
    }

    fn cambio(i: Item) -> Cambio {
        Cambio {
            id: i.id.clone(),
            quitado: false,
            item: Some(i),
        }
    }

    #[test]
    fn el_arbol_arma_sus_rutas_y_cuenta_lo_que_salta() {
        let v = base().rutas();
        let r: Vec<_> = v.items.iter().map(|i| i.ruta.as_str()).collect();
        assert_eq!(r, vec!["a.csv", "sub/c.csv"]);
        assert_eq!(v.saltados, 1);
        assert!(v.carpetas.contains("") && v.carpetas.contains("sub/"));
    }

    #[test]
    fn un_fichero_nuevo_o_renombrado_se_anota() {
        let mut ix = base();
        assert!(ix.aplicar(cambio(item("n", "n.csv", CSV, "sub"))).is_none());
        assert!(rutas(&ix).contains(&"sub/n.csv".to_string()));
        // Renombrado y movido a la raíz: el mismo ID, otra ruta.
        ix.aplicar(cambio(item("n", "nuevo.csv", CSV, "RAIZ")));
        let r = rutas(&ix);
        assert!(r.contains(&"nuevo.csv".to_string()) && !r.contains(&"sub/n.csv".to_string()));
    }

    #[test]
    fn lo_borrado_o_en_la_papelera_o_fuera_del_ambito_se_quita() {
        let mut ix = base();
        ix.aplicar(Cambio {
            id: "a".into(),
            quitado: true,
            item: None,
        });
        let mut papelera = item("c", "c.csv", CSV, "sub");
        papelera.papelera = true;
        ix.aplicar(cambio(papelera));
        assert!(rutas(&ix).is_empty());
        // Un fichero que sólo se ve en otra carpeta compartida no entra.
        assert!(
            ix.aplicar(cambio(item("x", "x.csv", CSV, "OTRA")))
                .is_none()
        );
        assert!(rutas(&ix).is_empty());
    }

    #[test]
    fn quitar_una_carpeta_quita_lo_que_colgaba_de_ella() {
        let mut ix = base();
        ix.aplicar(cambio(item("n", "n.csv", CSV, "sub")));
        // `sub` se mueve fuera del ámbito: sus hijos desaparecen con ella.
        ix.aplicar(cambio(item("sub", "sub", CARP, "OTRA")));
        assert_eq!(rutas(&ix), vec!["a.csv"]);
        assert!(!ix.rutas().carpetas.contains("sub/"));
    }

    #[test]
    fn una_carpeta_que_entra_pide_recorrerla() {
        let mut ix = base();
        let nueva = ix.aplicar(cambio(item("nueva", "nueva", CARP, "RAIZ")));
        assert_eq!(nueva.as_deref(), Some("nueva"));
        // Una carpeta que ya estaba (renombrada) no.
        assert!(
            ix.aplicar(cambio(item("sub", "otra", CARP, "RAIZ")))
                .is_none()
        );
        assert!(rutas(&ix).contains(&"otra/c.csv".to_string()));
    }

    #[test]
    fn dos_ficheros_con_el_mismo_nombre_llevan_su_id() {
        let mut ix = base();
        ix.aplicar(cambio(item("a2", "a.csv", CSV, "RAIZ")));
        let r = rutas(&ix);
        assert!(
            r.contains(&"a~a.csv".to_string()) && r.contains(&"a~a2.csv".to_string()),
            "{r:?}"
        );
    }

    #[test]
    fn el_feed_se_recorre_por_paginas_hasta_el_token_nuevo() {
        let mut pedidas = Vec::new();
        let (cs, nuevo) = paginar_cambios(
            |t| {
                pedidas.push(t.to_string());
                Ok(match t {
                    "10" => r#"{"nextPageToken":"11","changes":[{"fileId":"a","removed":true},{"fileId":"b","file":{"id":"b","name":"b.csv","mimeType":"text/csv","parents":["RAIZ"]}}]}"#.into(),
                    "11" => r#"{"newStartPageToken":"12","changes":[{"fileId":"c","file":{"id":"c","name":"c","trashed":true}}]}"#.into(),
                    o => panic!("{o}"),
                })
            },
            "10",
        )
        .unwrap();
        assert_eq!((nuevo.as_str(), pedidas.len(), cs.len()), ("12", 2, 3));
        assert!(cs[0].quitado && cs[0].item.is_none());
        assert_eq!(cs[1].item.as_ref().unwrap().padre, "RAIZ");
        assert!(cs[2].item.as_ref().unwrap().papelera);
        let e = paginar_cambios(|_| Ok("{}".into()), "1").unwrap_err();
        assert!(e.contains("neither"), "{e}");
    }
}
