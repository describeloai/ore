// Elastic Sessions N0 (docs/decisions/0073-elastic-sessions.md §8, Node) · Antes de darle a Node
// la elasticidad de Python, tres cosas que no sabemos, en un pod con la forma de una sesión
// (4 GiB, en el session pool) y la imagen de hoy (`puesto-node:1`, @duckdb/node-api):
//   A · qué tope se pone DuckDB si nadie se lo dice, y qué heap le da V8 dentro del cgroup;
//   B · si una SEGUNDA conexión contesta mientras otra corre una consulta grande, y si subir
//       `memory_limit` desde ella con la consulta en marcha sirve (el lanzador redimensiona el pod
//       a 16 GiB al ver `pide-crecer`, como en R0·2);
//   C · una consulta grande SIN tope, en un proceso hijo: ¿termina o mata el proceso?
// Una línea JSON por medida.
import fs from "node:fs";
import os from "node:os";
import v8 from "node:v8";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const req = createRequire("/opt/ore/");
const { DuckDBInstance } = await import(pathToFileURL(req.resolve("@duckdb/node-api")).href);

const linea = (d) => console.log(JSON.stringify(d));
const T = process.env.TRABAJO || "/trabajo";
const DERRAME = `${T}/derrame`;
const GRUPOS = Number(process.env.GRUPOS || "60000000");
// Agrupar GRUPOS claves distintas: segura (derrama), y grande para 4 GiB.
const Q = `select count(*) n, sum(sv) s from (select k, sum(v) sv from
  (select range k, random() v from range(${GRUPOS})) group by k)`;

const cgroupMb = () => {
  try {
    const v = fs.readFileSync("/sys/fs/cgroup/memory.max", "utf8").trim();
    return v === "max" ? null : Math.round(Number(v) / 2 ** 20);
  } catch {
    return null;
  }
};
const derramadoMb = () => {
  let n = 0;
  const ir = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = `${d}/${e.name}`;
      if (e.isDirectory()) ir(p);
      else n += fs.statSync(p).size;
    }
  };
  try {
    ir(DERRAME);
  } catch {}
  return Math.round(n / 2 ** 20);
};
const filas = async (c, sql) => (await c.runAndReadAll(sql)).getRows();
const ajuste = async (c, k) => String((await filas(c, `select current_setting('${k}')`))[0][0]);

// ── C, en el hijo: la consulta grande con lo que DuckDB elija ────────────────
if (process.argv[2] === "C") {
  const inst = await DuckDBInstance.create(":memory:", { temp_directory: DERRAME });
  const c = await inst.connect();
  linea({ evento: "C-empieza", memory_limit: await ajuste(c, "memory_limit") });
  const t0 = Date.now();
  const r = await filas(c, Q);
  linea({ evento: "C-termina", s: (Date.now() - t0) / 1000, n: String(r[0][0]), derramado_mb: derramadoMb() });
  process.exit(0);
}

fs.mkdirSync(DERRAME, { recursive: true });

// ── A · los topes por defecto ────────────────────────────────────────────────
{
  const inst = await DuckDBInstance.create(":memory:");
  const c = await inst.connect();
  linea({
    evento: "A-topes",
    cgroup_mb: cgroupMb(),
    maquina_mb: Math.round(os.totalmem() / 2 ** 20),
    cpus: os.availableParallelism(),
    duckdb_memory_limit: await ajuste(c, "memory_limit"),
    duckdb_threads: await ajuste(c, "threads"),
    v8_heap_limit_mb: Math.round(v8.getHeapStatistics().heap_size_limit / 2 ** 20),
    node: process.version,
    uv_threadpool: process.env.UV_THREADPOOL_SIZE || "4 (por defecto)",
  });
}

// ── B · la segunda conexión, con la consulta en marcha ───────────────────────
{
  const tope = 1024; // MB: como el reparto de una sesión pequeña, para que derrame
  const inst = await DuckDBInstance.create(":memory:", {
    memory_limit: `${tope}MB`,
    threads: "2",
    temp_directory: DERRAME,
  });
  const a = await inst.connect();
  const b = await inst.connect();
  const muestras = [];
  let corre = true;
  let subido = null;
  const t0 = Date.now();
  const consulta = filas(a, Q).then((r) => {
    corre = false;
    return r;
  });
  let pedido = false;
  const cg0 = cgroupMb();
  while (corre) {
    const t1 = Date.now();
    let uso = null;
    let error = null;
    try {
      uso = Math.round(Number((await filas(b, "select coalesce(sum(memory_usage_bytes), 0) from duckdb_memory()"))[0][0]) / 2 ** 20);
    } catch (e) {
      error = String(e).slice(0, 200);
    }
    muestras.push({ t: (t1 - t0) / 1000, contesta_ms: Date.now() - t1, uso_mb: uso, derramado_mb: derramadoMb(), error });
    if (!pedido && Date.now() - t0 > 3000) {
      pedido = true;
      linea({ evento: "pide-crecer", t: (Date.now() - t0) / 1000 });
    }
    const cg = cgroupMb();
    if (!subido && cg && cg0 && cg > cg0) {
      const nuevo = Math.floor((cg * 60) / 100);
      const t2 = Date.now();
      let error2 = null;
      try {
        await filas(b, `set memory_limit='${nuevo}MB'`);
        await filas(b, "set threads to 4");
      } catch (e) {
        error2 = String(e).slice(0, 200);
      }
      subido = { t: (t2 - t0) / 1000, cgroup_mb: cg, memory_limit: await ajuste(b, "memory_limit"), set_ms: Date.now() - t2, error: error2 };
      linea({ evento: "B-subido", ...subido });
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  const r = await consulta;
  const lat = muestras.map((m) => m.contesta_ms).sort((x, y) => x - y);
  linea({
    evento: "B-fin",
    s: (Date.now() - t0) / 1000,
    n: String(r[0][0]),
    muestras: muestras.length,
    contesta_p50_ms: lat[Math.floor(lat.length / 2)] ?? null,
    contesta_max_ms: lat[lat.length - 1] ?? null,
    errores: muestras.filter((m) => m.error).length,
    uso_max_mb: Math.max(0, ...muestras.map((m) => m.uso_mb ?? 0)),
    derramado_max_mb: Math.max(0, ...muestras.map((m) => m.derramado_mb)),
    subido,
  });
  linea({ evento: "B-muestras", primeras: muestras.slice(0, 5), ultimas: muestras.slice(-3) });
}

// ── C · sin tope, en un hijo: ¿termina o muere? ──────────────────────────────
fs.rmSync(DERRAME, { recursive: true, force: true });
fs.mkdirSync(DERRAME, { recursive: true });
await new Promise((ok) => {
  const t0 = Date.now();
  const h = spawn(process.execPath, [process.argv[1], "C"], { stdio: ["ignore", "inherit", "inherit"] });
  h.on("exit", (codigo, senal) => {
    linea({ evento: "C-hijo", codigo, senal, s: (Date.now() - t0) / 1000, cgroup_mb: cgroupMb() });
    ok();
  });
});
linea({ evento: "fin" });
