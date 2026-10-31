//! **El motor en un puesto** (0081 §7.5, C10·2 y C10·3), contra un `ore-serve` de mentira que sirve `loadTable`,
//! `loadView` y `loadCredentials` desde un lago en memoria y presta un token por tabla:
//!
//! - el motor resuelve por `/v1` lo que la sentencia nombra, y lo que nombran sus vistas;
//! - lee cada tabla con **su** credencial prestada, y la de otra tabla no le sirve;
//! - un ejecutor, que recibe el plan sin credenciales, canjea la suya con `loadCredentials`, y la renueva antes de que
//!   caduque;
//! - todo va con el token del pod (`x-ore-pod`), nunca con un portador.

use datafusion::arrow::array::{Int64Array, UInt64Array};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::physical_plan::{ExecutionPlan, collect};
use datafusion_proto::bytes::{
    physical_plan_from_bytes_with_extension_codec, physical_plan_to_bytes_with_extension_codec,
};
use ore_runtime::zocalos::Extensiones;
use ore_runtime::{Contexto, Motor, OreOpciones, Papel, Zocalos, codecs};
use ore_store::almacen::Almacen;
use ore_tabla::foto::Objeto;
use ore_tabla::memoria::LagoEnMemoria;
use ore_tabla::v1::{CatalogoV1, ConCredencial, Credencial, LagosV1};
use ore_tabla::{Nombre, ejecutable};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Lo que el `ore-serve` de mentira vio: método y ruta, y con qué se presentó quien llamaba.
#[derive(Debug, Default)]
struct Visto {
    rutas: Vec<String>,
    portadores: usize,
    pods: usize,
}

/// El token que se presta para una tabla, con su caducidad.
fn token_de(n: &str) -> String {
    format!("token-de-{n}")
}

/// **Un `ore-serve` de mentira** sobre `lago`: `GET …/tables/{t}[/credentials]`, `GET …/views/{v}`. Cada credencial
/// caduca a los `vida_ms`.
fn servidor(lago: Arc<LagoEnMemoria>, vida_ms: i64) -> (String, Arc<Mutex<Visto>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let visto = Arc::new(Mutex::new(Visto::default()));
    let v = visto.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let s = s.unwrap();
            let (lago, v) = (lago.clone(), v.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut linea = String::new();
                r.read_line(&mut linea).unwrap();
                let mut partes = linea.split_whitespace();
                let (metodo, ruta) = (
                    partes.next().unwrap().to_string(),
                    partes.next().unwrap().to_string(),
                );
                let mut cabeceras = HashMap::new();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    let (k, x) = h.split_once(':').unwrap();
                    cabeceras.insert(k.trim().to_ascii_lowercase(), x.trim().to_string());
                }
                let largo: usize = cabeceras
                    .get("content-length")
                    .map_or(0, |n| n.parse().unwrap());
                let mut b = vec![0; largo];
                r.read_exact(&mut b).unwrap();
                {
                    let mut v = v.lock().unwrap();
                    v.rutas.push(format!("{metodo} {ruta}"));
                    v.portadores += cabeceras.contains_key("authorization") as usize;
                    v.pods +=
                        (cabeceras.get("x-ore-pod").map(String::as_str) == Some("el-pod")) as usize;
                }
                let (codigo, cuerpo) = contestar(&lago, &ruta, vida_ms);
                let texto = cuerpo.to_string();
                let mut s = s;
                let _ = write!(
                    s,
                    "HTTP/1.1 {codigo} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{texto}",
                    texto.len()
                );
            });
        }
    });
    (url, visto)
}

fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn contestar(lago: &LagoEnMemoria, ruta: &str, vida_ms: i64) -> (u16, Value) {
    let p: Vec<&str> = ruta.trim_start_matches("/v1/").split('/').collect();
    // Los listados: `{base}/namespaces` y `{base}/namespaces/{schema}/(tables|views)`.
    let foto = lago.fuente().foto().unwrap();
    match p.as_slice() {
        [base, "namespaces"] => {
            let mut ns: Vec<String> = foto
                .objetos
                .keys()
                .filter(|n| n.base == *base)
                .map(|n| n.schema.clone())
                .collect();
            ns.dedup();
            return (
                200,
                json!({"namespaces": ns.iter().map(|n| vec![n]).collect::<Vec<_>>()}),
            );
        }
        [base, "namespaces", schema, clase] => {
            let ids: Vec<Value> = foto
                .objetos
                .iter()
                .filter(|(n, o)| {
                    n.base == *base
                        && n.schema == *schema
                        && matches!(
                            (o, *clase),
                            (Objeto::Dataset { .. }, "tables") | (Objeto::Vista { .. }, "views")
                        )
                })
                .map(|(n, _)| json!({"namespace": [schema], "name": n.nombre}))
                .collect();
            return (200, json!({"identifiers": ids}));
        }
        _ => {}
    }
    // `{base}/namespaces/{schema}/(tables|views)/{nombre}[/credentials]`
    let [base, "namespaces", schema, clase, nombre, resto @ ..] = p.as_slice() else {
        return (404, json!({"error": {"message": "no"}}));
    };
    let n = Nombre::new(base, schema, nombre);
    let prestado = |n: &Nombre| {
        let config = json!({
            "gcs.oauth2.token": token_de(&n.to_string()),
            "gcs.oauth2.token-expires-at": (ahora_ms() + vida_ms).to_string(),
        });
        json!({"config": config, "storage-credentials": [{"prefix": "gs://lago-en-memoria/", "config": config}]})
    };
    match (*clase, lago.objeto(&n), resto) {
        ("tables", Some(Objeto::Dataset { metadata_location }), []) => {
            let t = lago
                .lago()
                .abrir(&metadata_location, &n.to_string())
                .unwrap();
            let mut r = prestado(&n);
            r["metadata-location"] = json!(metadata_location);
            r["metadata"] = serde_json::to_value(t.metadata()).unwrap();
            (200, r)
        }
        ("tables", Some(Objeto::Dataset { .. }), ["credentials"]) => (200, prestado(&n)),
        ("views", Some(Objeto::Vista { sql }), []) => (
            200,
            json!({"metadata": {"current-version-id": 1, "versions": [
                {"version-id": 1, "representations": [{"type": "sql", "sql": sql}]}]}}),
        ),
        _ => (404, json!({"error": {"message": format!("`{n}` no está")}})),
    }
}

fn lote(ids: &[i64], importes: &[i64]) -> RecordBatch {
    RecordBatch::try_from_iter_with_nullable([
        ("id", Arc::new(Int64Array::from(ids.to_vec())) as _, true),
        (
            "importe",
            Arc::new(Int64Array::from(importes.to_vec())) as _,
            true,
        ),
    ])
    .unwrap()
}

fn sembrado() -> Arc<LagoEnMemoria> {
    let l = LagoEnMemoria::default();
    l.escribir(
        &Nombre::new("ventas", "default", "pedidos"),
        vec![lote(&[1, 2, 3], &[50, 150, 300])],
    )
    .unwrap();
    l.escribir(
        &Nombre::new("ventas", "default", "devoluciones"),
        vec![lote(&[2], &[150])],
    )
    .unwrap();
    l.vista(
        &Nombre::new("ventas", "default", "netos"),
        "select p.id from pedidos p left join devoluciones d on p.id = d.id where d.id is null",
    );
    Arc::new(l)
}

/// Los tokens con que se abrió un almacén: el de cada tabla y sólo ése.
type Usados = Arc<Mutex<Vec<String>>>;

/// Los lagos prestados de un proceso del puesto, sobre el almacén en memoria: cada token sólo abre su tabla.
fn lagos(url: &str, lago: &LagoEnMemoria, usados: &Usados) -> LagosV1 {
    let ruta = std::env::temp_dir().join(format!("ore-pod-{}", std::process::id()));
    std::fs::write(&ruta, "el-pod\n").unwrap();
    let (almacen, usados) = (lago.almacen(), usados.clone());
    let con: ConCredencial = Arc::new(move |bucket: &str, token: &str| {
        assert_eq!(bucket, "lago-en-memoria");
        assert!(token.starts_with("token-de-ventas.default."), "{token}");
        usados.lock().unwrap().push(token.to_string());
        Ok(almacen.clone() as Arc<dyn Almacen>)
    });
    LagosV1::con(url, Credencial::Pod(ruta), con)
}

#[derive(Debug)]
struct ConCodec(ore_tabla::CodecOre);

impl Extensiones for ConCodec {
    fn aplicar(
        &self,
        p: datafusion::execution::SessionStateBuilder,
    ) -> datafusion::execution::SessionStateBuilder {
        p
    }
    fn codecs(&self) -> Option<codecs::Codecs> {
        Some(self.0.clone().codecs())
    }
}

fn motor(catalogo: CatalogoV1) -> Motor {
    let z = Zocalos {
        extensiones: Arc::new(ConCodec(catalogo.codec())),
        catalogo: Arc::new(catalogo),
        ..Zocalos::default()
    };
    Motor::arrancar(Papel::Prueba, OreOpciones::default(), &z).unwrap()
}

fn correr(m: &Motor, sql: &str) -> Vec<RecordBatch> {
    let mut s = m
        .sesion(&Contexto::nuevo("puesto").en_base("ventas").que_escribe())
        .unwrap();
    m.es()
        .block_on(async {
            let p = s.planificar(sql).await?;
            let p = ejecutable(&s, p).await?;
            m.correr(&s, p)?.recoger().await
        })
        .unwrap()
}

fn entero(l: &[RecordBatch]) -> i64 {
    let c = l[0].column(0);
    c.as_any()
        .downcast_ref::<Int64Array>()
        .map(|a| a.value(0))
        .or_else(|| {
            c.as_any()
                .downcast_ref::<UInt64Array>()
                .map(|a| a.value(0) as i64)
        })
        .unwrap()
}

/// C10·2: el motor resuelve por `/v1` lo que nombra la sentencia y lo que nombra su vista, y lee cada tabla con su
/// token. Todo, con el token del pod.
#[test]
fn el_motor_lee_por_v1_con_la_credencial_de_cada_tabla() {
    let lago = sembrado();
    let (url, visto) = servidor(lago.clone(), 3_600_000);
    let usados: Usados = Default::default();
    let m = motor(CatalogoV1::new(lagos(&url, &lago, &usados)));
    assert_eq!(entero(&correr(&m, "select count(*) from netos")), 2);
    let v = visto.lock().unwrap();
    for r in [
        "GET /v1/ventas/namespaces/default/tables/netos",
        "GET /v1/ventas/namespaces/default/views/netos",
        "GET /v1/ventas/namespaces/default/tables/pedidos",
        "GET /v1/ventas/namespaces/default/tables/devoluciones",
    ] {
        assert!(v.rutas.iter().any(|x| x == r), "falta `{r}`: {:?}", v.rutas);
    }
    // El token de `loadTable` se usó tal cual: no hizo falta `loadCredentials`.
    assert!(
        !v.rutas.iter().any(|x| x.ends_with("/credentials")),
        "{:?}",
        v.rutas
    );
    assert_eq!(v.portadores, 0);
    assert_eq!(v.pods, v.rutas.len());
    let usados = usados.lock().unwrap();
    for t in ["pedidos", "devoluciones"] {
        assert!(
            usados.contains(&token_de(&format!("ventas.default.{t}"))),
            "{usados:?}"
        );
    }
}

/// C10·2: `information_schema` lista la base por sus nombres (`listNamespaces`, `listTables`, `listViews`) sin cargar
/// nada, así que nada cuenta como leído; las columnas de una tabla la cargan a ella sola.
#[test]
fn information_schema_lista_sin_cargar() {
    let lago = sembrado();
    let (url, visto) = servidor(lago.clone(), 3_600_000);
    let usados: Usados = Default::default();
    let m = motor(CatalogoV1::new(lagos(&url, &lago, &usados)));
    let l = correr(
        &m,
        "select table_name, table_type from information_schema.tables where table_schema = 'default' order by 1",
    );
    let texto = datafusion::arrow::util::pretty::pretty_format_batches(&l)
        .unwrap()
        .to_string();
    for t in [
        "devoluciones | BASE TABLE",
        "netos        | VIEW",
        "pedidos      | BASE TABLE",
    ] {
        assert!(texto.contains(t), "{texto}");
    }
    let cargadas = |v: &Visto| {
        v.rutas
            .iter()
            .filter(|r| r.contains("/tables/") || r.contains("/views/"))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(
        cargadas(&visto.lock().unwrap()).is_empty(),
        "{:?}",
        visto.lock().unwrap().rutas
    );
    let l = correr(
        &m,
        "select count(*) from information_schema.columns where table_name = 'pedidos'",
    );
    assert_eq!(entero(&l), 2);
    let v = visto.lock().unwrap();
    assert!(
        cargadas(&v).contains(&"GET /v1/ventas/namespaces/default/tables/pedidos".to_string()),
        "{:?}",
        v.rutas
    );
}

/// C10·2: un ejecutor recibe el escaneo sin credenciales y canjea la suya con `loadCredentials`; si caduca pronto, la
/// renueva en cada uso.
#[test]
fn un_ejecutor_canjea_su_credencial_y_la_renueva() {
    let lago = sembrado();
    // Caduca en un minuto: menos que el margen de renovar, así que cada uso la vuelve a pedir.
    let (url, visto) = servidor(lago.clone(), 60_000);
    let usados: Usados = Default::default();
    let catalogo = CatalogoV1::new(lagos(&url, &lago, &usados));
    let m = motor(catalogo.clone());
    let mut s = m
        .sesion(&Contexto::nuevo("puesto").en_base("ventas"))
        .unwrap();
    let fisico: Arc<dyn ExecutionPlan> = m.es().block_on(async {
        let p = s
            .planificar("select id from pedidos")
            .await
            .unwrap()
            .logico()
            .unwrap();
        s.estado().create_physical_plan(&p).await.unwrap()
    });
    // El ejecutor: otro proceso, otros lagos, ninguna credencial sembrada.
    let ejecutor = CatalogoV1::new(lagos(&url, &lago, &usados))
        .codec()
        .codecs();
    let (_, fisico_c) = codecs::del_papel(Some(ejecutor));
    let bytes = physical_plan_to_bytes_with_extension_codec(fisico, fisico_c.as_ref()).unwrap();
    let antes = visto.lock().unwrap().rutas.len();
    let vuelta = physical_plan_from_bytes_with_extension_codec(
        &bytes,
        s.estado().task_ctx().as_ref(),
        fisico_c.as_ref(),
    )
    .unwrap();
    assert_eq!(
        visto.lock().unwrap().rutas.len(),
        antes,
        "decodificar pidió algo"
    );
    for _ in 0..2 {
        let l = m
            .es()
            .block_on(collect(vuelta.clone(), s.estado().task_ctx()))
            .unwrap();
        assert_eq!(l.iter().map(|l| l.num_rows()).sum::<usize>(), 3);
    }
    let v = visto.lock().unwrap();
    let canjes = v
        .rutas
        .iter()
        .filter(|r| *r == "GET /v1/ventas/namespaces/default/tables/pedidos/credentials")
        .count();
    assert!(
        canjes >= 2,
        "el ejecutor no canjeó ni renovó: {:?}",
        v.rutas
    );
    assert_eq!(v.portadores, 0);
}
