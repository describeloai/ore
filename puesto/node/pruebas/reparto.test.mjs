// Elastic Sessions N1: el reparto del pod en Node (docs/decisions/0073-elastic-sessions.md §2.3):
// lo que mide el cgroup AHORA, el 60 % para DuckDB (45 % con una reservada) y el 30 % para V8.
import { test } from "node:test";
import assert from "node:assert/strict";
import { podMb, podCpus, tropoMb, heapMb, ajustar, aplicado, RESERVADAS } from "../ore/reparto.mjs";

/** `delCgroup` como si el pod midiera esto. */
const cgroup = (memoria = null, cpu = null) => (f) => ({ "memory.max": memoria, "cpu.max": cpu })[f] ?? null;
const conEntorno = (vars, f) => {
  const antes = Object.fromEntries(Object.keys(vars).map((k) => [k, process.env[k]]));
  Object.assign(process.env, vars);
  try { return f(); } finally {
    for (const [k, v] of Object.entries(antes)) { if (v === undefined) delete process.env[k]; else process.env[k] = v; }
  }
};

test("lo que mide el pod sale del cgroup, no de lo que dijo la plantilla", () => {
  conEntorno({ ORE_MEMORIA_MB: "4096" }, () => {
    assert.equal(podMb(cgroup(String(8 * 2 ** 30))), 8192);
    assert.equal(tropoMb(false, cgroup(String(8 * 2 ** 30))), Math.floor(8192 * 0.6));
    assert.equal(tropoMb(true, cgroup(String(8 * 2 ** 30))), Math.floor(8192 * 0.45));
    // Sin tope en el cgroup (un portátil, Docker sin límite): la plantilla.
    assert.equal(podMb(cgroup("max")), 4096);
    assert.equal(tropoMb(false, cgroup("max")), 2457);
  });
  conEntorno({ ORE_MEMORIA_MB: "" }, () => assert.equal(tropoMb(false, cgroup()), 512));
});

test("el heap de un kernel nuevo es el 30 % del pod de ahora", () => {
  assert.equal(heapMb(cgroup(String(4 * 2 ** 30))), Math.floor(4096 * 0.3));
  assert.equal(heapMb(cgroup(String(32 * 2 ** 30))), Math.floor(32768 * 0.3));
  conEntorno({ ORE_MEMORIA_MB: "" }, () => assert.equal(heapMb(cgroup()), null));
});

test("los núcleos del cgroup", () => {
  for (const [cpu, n] of [["400000 100000", 4], ["150000 100000", 2], ["max 100000", 0], [null, 0]]) {
    assert.equal(podCpus(cgroup(null, cpu)), n, String(cpu));
  }
});

test("las reservadas por su nombre, como en Python", () => {
  for (const sql of ["select k, mode(v) from t group by k", "select MEDIAN (x) from t", "select string_agg(s, ',') from t",
    "select approx_quantile(v, 0.9) from t", "select k, list(v) from t group by k", "select histogram(s) from t"]) {
    assert.ok(RESERVADAS.test(sql), sql);
  }
  for (const sql of ["select model(x) from t", "select * from listing", "select modes from t", "select 1"]) {
    assert.ok(!RESERVADAS.test(sql), sql);
  }
});

test("ajustar se lo dice a DuckDB, y sólo si cambia", async () => {
  const { DuckDBInstance } = await import("@duckdb/node-api");
  const con = await (await DuckDBInstance.create(":memory:")).connect();
  const ajuste = async (k) => String((await con.runAndReadAll(`select current_setting('${k}')`)).getRows()[0][0]);
  const enMb = (v) => { const [n, u] = v.split(" "); return Number(n) * { KiB: 2 ** 10, MiB: 2 ** 20, GiB: 2 ** 30 }[u] / 1e6; };
  aplicado.mb = null; aplicado.hilos = null;
  const hilos = process.env.ORE_HILOS;
  delete process.env.ORE_HILOS;
  try {
    const pod = cgroup(String(8 * 2 ** 30), "400000 100000");
    assert.equal(await ajustar(con, "", pod), 4915);
    assert.ok(Math.abs(enMb(await ajuste("memory_limit")) - 4915) / 4915 < 0.05);
    assert.equal(await ajuste("threads"), "4");
    assert.equal(await ajustar(con, "select mode(v) from t", pod), 3686);
    assert.ok(Math.abs(enMb(await ajuste("memory_limit")) - 3686) / 3686 < 0.05);
  } finally { if (hilos !== undefined) process.env.ORE_HILOS = hilos; }
  aplicado.mb = null; aplicado.hilos = null;
});
