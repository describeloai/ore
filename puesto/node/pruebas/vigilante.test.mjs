// Elastic Sessions N2: crecer solo en Node (docs/products/0073-elastic-sessions.md §2.3, §2.6):
// el vigilante pide lo previsto antes de correr, pide crecer cuando el motor se llena o derrama y lo
// aplica en caliente; y una sesión que creció vuelve a su base tras un rato sin celdas.
import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.ORE_CELDAS = mkdtempSync(join(tmpdir(), "vigilante-"));
const ore = await import("../ore/index.mjs");
const { motor, aplicado } = await import("../ore/reparto.mjs");
const { Vigilante, soltarElMotor } = await import("../vigilante.mjs");
const { bajarSiToca, BAJAR_TRAS } = await import("../kernel.mjs");
const { DuckDBInstance } = await import("@duckdb/node-api");

const ID = "puesto-ana-node";
let idAntes;
beforeEach(() => { idAntes = ore.session.id; ore.session.id = ID; Vigilante.sinPool = false; aplicado.mb = null; aplicado.hilos = null; });
afterEach(() => { ore.session.id = idAntes; Vigilante.sinPool = false; motor.instancia = null; motor.conexion = null; aplicado.mb = null; aplicado.hilos = null; });

/** `pedir` que apunta lo pedido y contesta `respuesta`; `podMb` que mide `pods` (y luego el último). */
function falsos(respuesta, pods = [4096]) {
  const pedidos = [];
  const pedir = async (metodo, ruta, cuerpo) => { pedidos.push([metodo, ruta, cuerpo]); return respuesta; };
  let i = 0;
  const podMb = () => pods[Math.min(i++, pods.length - 1)];
  return { pedidos, pedir, podMb };
}

test("pide lo previsto de un golpe antes de correr, con el motivo del sobre", async () => {
  const f = falsos([200, { estado: "hecho", escalon: 3 }], [4096, 4096, 32768]);
  const v = await new Vigilante("select 1", { previstoMb: 32768, previstoMotivo: "plan" }, { ...f, cada: 20, esperaCrecer: 1 }).empezar();
  v.parar();
  assert.deepEqual(f.pedidos, [["POST", `/puestos/${ID}/scale`, { memoria_mb: 32768, motivo: "plan", de_un_golpe: true }]]);
  assert.equal(v.pedidos[0].estado, "hecho");
});

test("sin previsión, o si ya cabe, no pide; y sin motivo es forecast", async () => {
  for (const previstoMb of [0, null, 4096, 2048]) {
    const f = falsos([200, { estado: "hecho" }]);
    const v = await new Vigilante("", { previstoMb }, { ...f, cada: 20 }).empezar();
    v.parar();
    assert.deepEqual(f.pedidos, [], String(previstoMb));
    assert.equal(v.mirando, false, "parado");
  }
  const f = falsos([200, { estado: "hecho" }], [4096, 4096, 8192]);
  (await new Vigilante("", { previstoMb: 8192 }, { ...f, esperaCrecer: 1 }).empezar()).parar();
  assert.equal(f.pedidos[0][2].motivo, "forecast");
});

test("si lo previsto no cabe, empieza como está y vigila", async () => {
  const f = falsos([200, { estado: "quota" }]);
  const v = await new Vigilante("", { previstoMb: 32768 }, { ...f, cada: 20 }).empezar();
  assert.equal(f.pedidos.length, 1);
  assert.equal(v.mirando, true, "sube de uno en uno");
  v.parar();
});

test("fuera del session pool no vuelve a pedir", async () => {
  const f = falsos([409, { error: "esta sesión no está en el session pool: no crece" }]);
  const v = await new Vigilante("", { previstoMb: 32768 }, f).empezar();
  assert.equal(Vigilante.sinPool, true);
  assert.equal(v.mirando, false);
  const otro = falsos([200, { estado: "hecho" }]);
  (await new Vigilante("", { previstoMb: 32768 }, otro).empezar()).parar();
  assert.deepEqual(otro.pedidos, []);
});

test("una consulta que derrama le hace pedir; con quota, termina igual", async () => {
  const inst = await DuckDBInstance.create(":memory:", { temp_directory: mkdtempSync(join(tmpdir(), "derrame-")) });
  const con = await inst.connect();
  await con.run("set memory_limit = '64MB'");
  await con.run("set threads to 2");
  motor.instancia = inst;
  motor.conexion = con;
  aplicado.mb = 64;
  const f = falsos([200, { estado: "quota" }]);
  const v = await new Vigilante("select count(distinct x)", {}, { ...f, cada: 50 }).empezar();
  const r = await con.runAndReadAll("select count(distinct x) n from (select random() x from range(8000000))");
  v.parar();
  assert.ok(Number(r.getRows()[0][0]) > 1_000_000);
  assert.equal(f.pedidos.length, 1, JSON.stringify(f.pedidos));
  assert.equal(f.pedidos[0][2].memoria_mb, 8192, "el doble de lo que mide el pod");
  assert.match(f.pedidos[0][2].motivo, /^(spilling|memory at \d+%)$/);
  assert.equal(v.mirando, false, "con quota, en esta celda no se vuelve a pedir");
});

test("al crecer sube el memory_limit desde la otra conexión", async () => {
  const inst = await DuckDBInstance.create(":memory:");
  const con = await inst.connect();
  motor.instancia = inst;
  motor.conexion = con;
  const memoria = process.env.ORE_MEMORIA_MB;
  process.env.ORE_MEMORIA_MB = "8192"; // el pod «mide» 8 GiB tras crecer (sin cgroup, la plantilla)
  try {
    const f = falsos([200, { estado: "hecho" }], [4096, 8192]);
    const v = new Vigilante("", {}, { ...f, esperaCrecer: 1 });
    assert.equal(await v.crecer("spilling"), true);
    v.parar();
    const lim = String((await con.runAndReadAll("select current_setting('memory_limit')")).getRows()[0][0]);
    assert.match(lim, /^4\.\d GiB$/, lim); // el 60 % de 8 GiB
  } finally { if (memoria === undefined) delete process.env.ORE_MEMORIA_MB; else process.env.ORE_MEMORIA_MB = memoria; }
});

test("antes de bajar, el motor suelta", async () => {
  const inst = await DuckDBInstance.create(":memory:");
  motor.conexion = await inst.connect();
  const r = await soltarElMotor(0);
  assert.ok("motor_mb" in r);
  const lim = String((await motor.conexion.runAndReadAll("select current_setting('memory_limit')")).getRows()[0][0]);
  assert.match(lim, /^2\d\d\.\d MiB$/, lim);
  assert.equal(aplicado.mb, 256);
});

test("la que creció y lleva un rato sin celdas vuelve a la base; si no, nada", async () => {
  const memoria = process.env.ORE_MEMORIA_MB;
  process.env.ORE_MEMORIA_MB = "4096";
  try {
    const hecho = [];
    const kernel = { bajar: async () => { hecho.push("bajar"); return { motor_mb: 40 }; } };
    const puesto = { id: ID, pedir: async (...a) => { hecho.push(a); return [200, { estado: "hecho" }]; } };
    const hace = (s) => Date.now() - s * 1000;
    assert.equal(await bajarSiToca(kernel, puesto, hace(BAJAR_TRAS + 5), () => 4096), null, "no creció");
    assert.equal(await bajarSiToca(kernel, puesto, hace(10), () => 16384), null, "trabaja");
    assert.deepEqual(hecho, []);
    const r = await bajarSiToca(kernel, puesto, hace(BAJAR_TRAS + 5), () => 16384);
    assert.equal(r.codigo, 200);
    assert.equal(hecho[0], "bajar", "el motor, primero");
    assert.deepEqual(hecho[1].slice(0, 3), ["POST", `/puestos/${ID}/scale`, { memoria_mb: 4096, motivo: `idle ${BAJAR_TRAS} s` }]);
  } finally { if (memoria === undefined) delete process.env.ORE_MEMORIA_MB; else process.env.ORE_MEMORIA_MB = memoria; }
});
