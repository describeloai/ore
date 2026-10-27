// Elastic Sessions N1: el kernel de Node en su proceso (docs/products/0073-elastic-sessions.md
// §2.2): lo de una celda queda para la siguiente; si el kernel se queda sin memoria muere él, la
// celda lo dice y la siguiente corre en un kernel nuevo, en la misma sesión.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

process.env.ORE_CELDAS = mkdtempSync(join(tmpdir(), "kernel-"));
const { KernelAparte, oomKills } = await import("../kernel.mjs");

const conHeap = (mb, f) => {
  const antes = process.env.ORE_HEAP_MB;
  process.env.ORE_HEAP_MB = String(mb);
  try { return f(); } finally { if (antes === undefined) delete process.env.ORE_HEAP_MB; else process.env.ORE_HEAP_MB = antes; }
};

test("corre en otro proceso y guarda lo de una celda para la siguiente", async () => {
  const k = new KernelAparte();
  try {
    assert.equal((await k.correr("const y: number = 21", "typescript")).tipo, "vacia");
    assert.equal((await k.correr("y * 2", "typescript")).texto, "42");
    const pid = await k.correr("process.pid", "typescript");
    assert.notEqual(pid.texto, String(process.pid));
    assert.equal((await k.correr("noExiste + 1", "typescript")).nombre, "ReferenceError");
  } finally { k.cerrar(); }
});

test("sin heap, la celda dice OutOfMemoryError y la siguiente corre en un kernel nuevo", async () => {
  const k = conHeap(64, () => new KernelAparte());
  try {
    await k.correr("const z = 7", "typescript");
    const r = await k.correr("const a = []; while (true) a.push(new Array(1e6).fill(1))", "typescript");
    assert.equal(r.nombre, "OutOfMemoryError", JSON.stringify(r));
    assert.match(r.mensaje, /heap \(64 MB\)/);
    assert.match(r.mensaje, /kernel restarted/);
    // La sesión sigue: otra celda contesta, y lo de antes ya no está.
    assert.equal((await k.correr("1 + 1", "typescript")).texto, "2");
    assert.equal((await k.correr("z", "typescript")).nombre, "ReferenceError");
  } finally { k.cerrar(); }
});

test("un kernel que se va es KernelDied, y el siguiente contesta", async () => {
  const k = new KernelAparte();
  try {
    const r = await k.correr("process.exit(3)", "typescript");
    assert.equal(r.nombre, "KernelDied", JSON.stringify(r));
    assert.match(r.mensaje, /exit 3/);
    assert.equal((await k.correr("40 + 2", "typescript")).texto, "42");
  } finally { k.cerrar(); }
});

test("las muertes por memoria del pod salen de memory.events", () => {
  assert.equal(oomKills(() => "low 0\nhigh 0\nmax 3\noom 1\noom_kill 2\noom_group_kill 0"), 2);
  assert.equal(oomKills(() => null), null);
});
