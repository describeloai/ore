// `ore` · el SDK del puesto para TS/JS (0031 W3.4). El mismo contrato que
// `puesto/python/ore`:
//
//   over("<paquete>.<vista>")   → las filas de la copia de esa vista (objetos)
//   sql("select … from p.v")    → las filas del resultado (DuckDB en el puesto)
//   persona()                   → quién abrió el puesto (`persona:…`)
//
// El código nunca ve el bucket ni una credencial: pregunta a `ore-serve` QUÉ
// copia es (con la identidad del puesto, que la resuelve en nombre de la
// persona y con su potestad) y baja el artefacto con la identidad del pod
// (el token del servidor de metadatos; sin librería de Google: la API JSON de
// GCS a pelo, que es lo único que hace falta). El sobre `ORECOPY1` se
// desenvuelve aquí; la carga es Parquet, y DuckDB (`@duckdb/node-api`) la lee.
//
// Fuera del clúster (las pruebas de fuego) el almacén es un directorio:
// `ORE_ALMACEN=dir:/ruta` lee `ore/v1/<clave>` de ahí.
import { mkdirSync, existsSync, readFileSync, writeFileSync, renameSync, accessSync, constants } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const MAGIA = "ORECOPY1";

export const puesto = {
  servidor: (process.env.ORE_SERVE ?? "http://127.0.0.1:8080").replace(/\/+$/, ""),
  id: process.env.PUESTO ?? "",
  bucket: process.env.BUCKET ?? "",
  almacen: process.env.ORE_ALMACEN ?? "gcs",
  // Quién abrió el puesto: lo pone el agente al arrancar (de la ficha).
  persona: "",
  // El token lo pone el agente (`agente.mjs`) y lo renueva; una celda no lo ve.
  _cabeceras: {},
  /** `[código, cuerpo]` de una petición a ore-serve; el cuerpo, JSON o `{error}`. */
  async pedir(metodo, ruta, cuerpo, plazoMs = 30_000) {
    const cab = { accept: "application/json", ...this._cabeceras };
    if (cuerpo !== undefined) cab["content-type"] = "application/json";
    const r = await fetch(this.servidor + ruta, {
      method: metodo,
      headers: cab,
      body: cuerpo === undefined ? undefined : JSON.stringify(cuerpo),
      signal: AbortSignal.timeout(plazoMs),
    });
    const texto = await r.text();
    if (!texto.trim()) return [r.status, null];
    try { return [r.status, JSON.parse(texto)]; } catch { return [r.status, { error: texto.trim() }]; }
  },
};

/** Quién abrió el puesto (`persona:…`). Lo sabe el agente desde que reclama. */
export function persona() {
  if (!puesto.persona) throw new Error("persona(): el agente aún no sabe quién abrió el puesto");
  return puesto.persona;
}

// ── el almacén ─────────────────────────────────────────────────────────────
let tokenDelPod = { valor: "", caduca: 0 };

async function tokenDeGoogle() {
  if (Date.now() < tokenDelPod.caduca - 60_000) return tokenDelPod.valor;
  const r = await fetch("http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token", {
    headers: { "Metadata-Flavor": "Google" },
    signal: AbortSignal.timeout(10_000),
  });
  if (!r.ok) throw new Error(`el servidor de metadatos contestó ${r.status}`);
  const t = await r.json();
  tokenDelPod = { valor: t.access_token, caduca: Date.now() + (t.expires_in ?? 300) * 1000 };
  return tokenDelPod.valor;
}

async function bajar(bucket, clave) {
  if (puesto.almacen.startsWith("dir:")) {
    return readFileSync(join(puesto.almacen.slice(4).replace(/\/+$/, ""), clave));
  }
  if (puesto.almacen === "gcs") {
    const url = `https://storage.googleapis.com/storage/v1/b/${encodeURIComponent(bucket)}/o/${encodeURIComponent(clave)}?alt=media`;
    const r = await fetch(url, { headers: { authorization: "Bearer " + (await tokenDeGoogle()) }, signal: AbortSignal.timeout(600_000) });
    if (!r.ok) throw new Error(`GCS contestó ${r.status} por ${clave}: ${(await r.text()).slice(0, 200)}`);
    return Buffer.from(await r.arrayBuffer());
  }
  throw new Error(`ORE_ALMACEN=${JSON.stringify(puesto.almacen)} no es un almacén: vale \`gcs\` o \`dir:<ruta>\``);
}

function desenvolver(crudo) {
  if (crudo.subarray(0, 8).toString("latin1") !== MAGIA) throw new Error("el artefacto no es una copia de ORE (sin `ORECOPY1`)");
  const n = crudo.readUInt32LE(8);
  return { cabecera: JSON.parse(crudo.subarray(12, 12 + n).toString("utf8")), carga: crudo.subarray(12 + n) };
}

async function resolver(vista) {
  if (typeof vista !== "string" || vista.split(".").length !== 2) throw new Error(`se quiere \`<paquete>.<vista>\`, no ${JSON.stringify(vista)}`);
  const [codigo, r] = await puesto.pedir("GET", `/puestos/${puesto.id}/datos/${vista}`);
  if (codigo === 409) throw new Error(`la copia de \`${vista}\` no está hecha: ${r?.error ?? ""}`);
  if (codigo === 404) throw new Error(`no hay ninguna \`View\` \`${vista}\` en el árbol`);
  if (codigo !== 200) throw new Error(`ore-serve contestó ${codigo} por \`${vista}\`: ${r?.error ?? JSON.stringify(r)}`);
  return r;
}

function copiasPorDefecto() {
  if (process.env.ORE_COPIAS) return process.env.ORE_COPIAS;
  try { accessSync("/trabajo", constants.W_OK); return "/trabajo/copias"; } catch { return join(tmpdir(), "ore-copias"); }
}

/** La copia de la vista como Parquet local, bajado UNA vez por sesión (la
 *  clave es el digest del artefacto: una clave nueva es otra copia). */
async function parquetDe(vista) {
  const r = await resolver(vista);
  const d = copiasPorDefecto();
  mkdirSync(d, { recursive: true });
  const f = join(d, r.clave.replaceAll("/", "_") + ".parquet");
  if (!existsSync(f)) {
    const { carga } = desenvolver(await bajar(r.bucket || puesto.bucket, r.clave));
    writeFileSync(f + ".parte", carga);
    renameSync(f + ".parte", f);
  }
  return [f, r];
}

// ── DuckDB ─────────────────────────────────────────────────────────────────
let conexion = null;

async function duckdb() {
  if (!conexion) {
    const { DuckDBInstance } = await import("@duckdb/node-api");
    const inst = await DuckDBInstance.create(":memory:");
    conexion = await inst.connect();
    if (process.env.ORE_HILOS) await conexion.run(`set threads to ${Number(process.env.ORE_HILOS)}`);
  }
  return conexion;
}

const VISTAS_EN_SQL = /\b(?:from|join)\s+([a-z_][a-z0-9_]*)\.([a-z_][a-z0-9_]*)\b/gi;

/** Las filas de un resultado de DuckDB, como objetos con valores llanos (JSON).
 *  DuckDB da los enteros grandes y los decimales como CADENAS por si no caben
 *  en un `number`; un `count(*)` es un BIGINT y se quiere el 3, no el "3":
 *  vuelven a número cuando caben (si no, se quedan en cadena). */
async function filasDe(con, texto) {
  const r = await con.runAndReadAll(texto);
  const numericas = new Set(r.columnNames().filter((_, i) => /^(U?BIGINT|U?HUGEINT|DECIMAL)/.test(String(r.columnTypes()[i]))));
  const filas = r.getRowObjectsJson();
  if (numericas.size) {
    for (const f of filas) {
      for (const c of numericas) {
        const v = f[c];
        if (typeof v === "string" && /^-?\d+(\.\d+)?$/.test(v)) {
          const n = Number(v);
          if (Number.isFinite(n) && (v.includes(".") || Number.isSafeInteger(n))) f[c] = n;
        }
      }
    }
  }
  return filas;
}

/** La copia de `<paquete>.<vista>` como filas (objetos). */
export async function over(vista) {
  const [f] = await parquetDe(vista);
  return filasDe(await duckdb(), `select * from read_parquet('${f.replaceAll("'", "''").replaceAll("\\", "/")}')`);
}

/** SQL (DuckDB) sobre las copias: cada `paquete.vista` tras FROM/JOIN se
 *  resuelve, se baja una vez y queda como vista `paquete.vista`. */
export async function sql(texto) {
  if (typeof texto !== "string" || !texto.trim()) throw new Error("sql() quiere una consulta");
  const con = await duckdb();
  const vistas = new Set([...texto.matchAll(VISTAS_EN_SQL)].map((m) => `${m[1]}.${m[2]}`));
  for (const v of [...vistas].sort()) {
    const [esquema, nombre] = v.split(".");
    const [f] = await parquetDe(v);
    await con.run(`create schema if not exists "${esquema}"`);
    await con.run(`create or replace view "${esquema}"."${nombre}" as select * from read_parquet('${f.replaceAll("'", "''").replaceAll("\\", "/")}')`);
  }
  return filasDe(con, texto);
}

export default { over, sql, persona, puesto };
