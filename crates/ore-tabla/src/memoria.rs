//! **Un lago en memoria** (0081 §7.5, C4): un almacén que guarda los objetos en un mapa, para las pruebas y la
//! vara de ORE. Las tablas se escriben con el mismo `Lago` de `ore-store` que en producción: lo único que no es
//! de verdad es dónde quedan los bytes.

use crate::escritura::{FalloCommit, Puerta};
use crate::foto::{Foto, Fuente, Nombre, Objeto};
use async_trait::async_trait;
use datafusion::arrow::record_batch::RecordBatch;
use iceberg::spec::Schema;
use iceberg::table::Table;
use iceberg::{TableRequirement, TableUpdate};
use ore_runtime::Fallo;
use ore_store::almacen::Almacen;
use ore_store::lago::{Lago, Operacion};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Un almacén en memoria que sólo crea, como GCS. Cuenta lo que se lee.
#[derive(Default)]
pub struct Memoria(Mutex<BTreeMap<String, Vec<u8>>>, AtomicUsize);

impl Almacen for Memoria {
    fn base(&self) -> String {
        "gs://lago-en-memoria".into()
    }
    fn leer(&self, clave: &str) -> Result<Option<String>, String> {
        Ok(self
            .leer_bytes(clave)?
            .map(|b| String::from_utf8_lossy(&b).to_string()))
    }
    fn existe(&self, clave: &str) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().contains_key(clave))
    }
    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
        let mut o = self.0.lock().unwrap();
        if o.contains_key(clave) {
            return Ok(false);
        }
        o.insert(clave.into(), cuerpo.to_vec());
        Ok(true)
    }
    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefijo))
            .cloned()
            .collect())
    }
    fn borrar(&self, clave: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(clave);
        Ok(())
    }
    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
        self.1.fetch_add(1, Ordering::Relaxed);
        Ok(self.0.lock().unwrap().get(clave).cloned())
    }
}

/// **Un lago en memoria y su foto**, que se cambia a mano: lo que un catálogo de verdad haría al commitear.
#[derive(Debug)]
pub struct LagoEnMemoria {
    almacen: Arc<Memoria>,
    lago: Lago,
    foto: Arc<FotoFija>,
    puerta: Arc<PuertaEnMemoria>,
}

impl Default for LagoEnMemoria {
    fn default() -> Self {
        let almacen = Arc::new(Memoria::default());
        let lago = Lago::nuevo(almacen.clone());
        let foto = Arc::new(FotoFija::default());
        LagoEnMemoria {
            puerta: Arc::new(PuertaEnMemoria {
                lago: lago.clone(),
                foto: foto.clone(),
                claves: Mutex::new(HashMap::new()),
                commits: Mutex::new(0),
            }),
            almacen,
            lago,
            foto,
        }
    }
}

impl std::fmt::Debug for Memoria {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Memoria({} objetos)", self.0.lock().unwrap().len())
    }
}

impl LagoEnMemoria {
    pub fn lago(&self) -> &Lago {
        &self.lago
    }

    /// La foto de este lago, como fuente del catálogo.
    pub fn fuente(&self) -> Arc<dyn Fuente> {
        self.foto.clone()
    }

    /// La puerta del commit de este lago.
    pub fn puerta(&self) -> Arc<dyn Puerta> {
        self.puerta.clone()
    }

    /// Cuántos commits movieron un puntero (los repetidos con la misma clave no cuentan).
    pub fn commits(&self) -> usize {
        *self.puerta.commits.lock().unwrap()
    }

    /// Lo que la foto tiene hoy en `nombre`.
    pub fn objeto(&self, nombre: &Nombre) -> Option<Objeto> {
        self.foto.objeto(nombre)
    }

    /// **Escribe un dataset** con estos lotes (una tabla nueva, o una versión nueva si ya estaba: sobrescribe) y
    /// lo pone en la foto. Devuelve su `metadata_location`.
    pub fn escribir(&self, nombre: &Nombre, lotes: Vec<RecordBatch>) -> Result<String, String> {
        let arrow = lotes.first().ok_or("sin lotes")?.schema();
        let previo = match self.foto.objeto(nombre) {
            Some(Objeto::Dataset { metadata_location }) => {
                Some(self.lago.abrir(&metadata_location, &nombre.to_string())?)
            }
            _ => None,
        };
        let (tabla, esquema) = match previo {
            Some(t) => {
                let e = t.metadata().current_schema().as_ref().clone();
                (t, e)
            }
            None => {
                let e = Lago::esquema_de_tabla_nueva(&arrow)?;
                let dataset = format!(
                    "catalogo/{}/{}/{}",
                    nombre.base, nombre.schema, nombre.nombre
                );
                (self.lago.crear(&dataset, e.clone(), HashMap::new())?, e)
            }
        };
        let iceberg =
            Arc::new(iceberg::arrow::schema_to_arrow_schema(&esquema).map_err(|e| e.to_string())?);
        let lotes = lotes
            .into_iter()
            .map(|l| {
                RecordBatch::try_new(iceberg.clone(), l.columns().to_vec())
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let p = self.lago.preparar(
            &tabla,
            esquema,
            lotes,
            Operacion::Sobrescribir,
            HashMap::new(),
        )?;
        let nueva = self.lago.confirmar(&tabla, p.cambios, p.requisitos)?;
        let ml = nueva
            .metadata_location()
            .ok_or("la tabla nueva sin metadata_location")?
            .to_string();
        self.foto.poner(
            nombre.clone(),
            Objeto::Dataset {
                metadata_location: ml.clone(),
            },
        );
        Ok(ml)
    }

    /// Pone una vista en la foto.
    pub fn vista(&self, nombre: &Nombre, sql: &str) {
        self.foto
            .poner(nombre.clone(), Objeto::Vista { sql: sql.into() });
    }

    /// Cuántas veces se leyó un objeto del almacén: lo que un plan que viaja no debe mover (C8).
    pub fn objetos_leidos(&self) -> usize {
        self.almacen.1.load(Ordering::Relaxed)
    }

    /// Cuántas veces se leyó la foto: una por consulta.
    pub fn lecturas(&self) -> usize {
        *self.foto.lecturas.lock().unwrap()
    }
}

/// Una foto que se cambia a mano.
#[derive(Debug, Default)]
pub struct FotoFija {
    foto: Mutex<Foto>,
    lecturas: Mutex<usize>,
}

impl FotoFija {
    fn poner(&self, nombre: Nombre, objeto: Objeto) {
        self.foto.lock().unwrap().objetos.insert(nombre, objeto);
    }

    fn objeto(&self, nombre: &Nombre) -> Option<Objeto> {
        self.foto.lock().unwrap().objetos.get(nombre).cloned()
    }

    fn quitar(&self, nombre: &Nombre) {
        self.foto.lock().unwrap().objetos.remove(nombre);
    }

    /// El `metadata_location` vigente de un dataset, o nada.
    fn vigente(&self, nombre: &Nombre) -> Option<String> {
        match self.objeto(nombre) {
            Some(Objeto::Dataset { metadata_location }) => Some(metadata_location),
            _ => None,
        }
    }
}

/// **La puerta del lago en memoria**: el mismo contrato que la de `/v1` (el CAS contra lo que se leyó y la clave de
/// idempotencia), sobre la foto.
#[derive(Debug)]
pub struct PuertaEnMemoria {
    lago: Lago,
    foto: Arc<FotoFija>,
    /// Lo que contestó cada clave: la misma clave otra vez contesta lo mismo.
    claves: Mutex<HashMap<String, String>>,
    commits: Mutex<usize>,
}

#[async_trait]
impl Puerta for PuertaEnMemoria {
    async fn esbozar(&self, nombre: &Nombre, esquema: Schema) -> Result<Table, String> {
        let dataset = format!(
            "catalogo/{}/{}/{}",
            nombre.base, nombre.schema, nombre.nombre
        );
        self.lago.crear(&dataset, esquema, HashMap::new())
    }

    async fn confirmar(
        &self,
        nombre: &Nombre,
        tabla: &Table,
        requisitos: Vec<TableRequirement>,
        cambios: Vec<TableUpdate>,
        clave: &str,
    ) -> Result<String, FalloCommit> {
        if let Some(ml) = self.claves.lock().unwrap().get(clave) {
            return Ok(ml.clone());
        }
        let esperado = tabla.metadata_location().map(String::from);
        let cotejar = || {
            let hay = self.foto.vigente(nombre);
            if hay != esperado {
                return Err(FalloCommit::Conflicto { hay });
            }
            Ok(())
        };
        cotejar()?;
        let nueva = self
            .lago
            .confirmar_async(tabla, cambios, requisitos)
            .await
            .map_err(FalloCommit::Otro)?;
        let ml = nueva
            .metadata_location()
            .ok_or_else(|| FalloCommit::Otro("sin metadata_location".into()))?
            .to_string();
        cotejar()?;
        self.foto.poner(
            nombre.clone(),
            Objeto::Dataset {
                metadata_location: ml.clone(),
            },
        );
        self.claves
            .lock()
            .unwrap()
            .insert(clave.to_string(), ml.clone());
        *self.commits.lock().unwrap() += 1;
        Ok(ml)
    }

    async fn retirar(
        &self,
        nombre: &Nombre,
        esperado: &str,
        clave: &str,
    ) -> Result<(), FalloCommit> {
        if self.claves.lock().unwrap().contains_key(clave) {
            return Ok(());
        }
        let hay = self.foto.vigente(nombre);
        if hay.as_deref() != Some(esperado) {
            return Err(FalloCommit::Conflicto { hay });
        }
        self.foto.quitar(nombre);
        self.claves
            .lock()
            .unwrap()
            .insert(clave.to_string(), String::new());
        *self.commits.lock().unwrap() += 1;
        Ok(())
    }
}

impl Fuente for FotoFija {
    fn foto(&self) -> Result<Foto, Fallo> {
        *self.lecturas.lock().unwrap() += 1;
        Ok(self.foto.lock().unwrap().clone())
    }
}
