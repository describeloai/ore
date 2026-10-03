// El contrato de Node (0050 R3 T2): `node --test puesto/node/pruebas/`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { call, convert, checkOutput, parseType, ContractError, toWire } from "../ore/contract.mjs";

const mal = (f, re) => assert.throws(f, (e) => e instanceof ContractError && re.test(e.message));

test("el tipo canónico se lee", () => {
  assert.deepEqual(parseType("Decimal<12, 2>"), { k: "Decimal", p: 12, s: 2 });
  assert.deepEqual(parseType("list<Struct<id: BigInt, precio: Money<EUR, 2>>>"), {
    k: "list",
    of: { k: "Struct", fields: [["id", { k: "BigInt" }], ["precio", { k: "Money", unit: "EUR", s: 2 }]] },
  });
  assert.deepEqual(parseType("Media<legal.archivo.contratos>"), { k: "Media", collection: "legal.archivo.contratos" });
  assert.throws(() => parseType("list<"));
});

test("los enteros: number exacto o bigint, según lo declarado", () => {
  assert.equal(convert("n", 3, "Integer"), 3);
  assert.equal(convert("n", "3", "Integer"), 3);
  mal(() => convert("n", 3.5, "Integer"), /not an integer/);
  mal(() => convert("n", "9007199254740993", "Integer"), /cannot hold exactly/);
  assert.equal(convert("n", "9007199254740993", "BigInt"), 9007199254740993n);
  assert.equal(convert("n", 7, "BigInt"), 7n);
  mal(() => convert("n", 2 ** 60, "BigInt"), /send it as a string/);
  assert.equal(convert("x", 1.5, "Float"), 1.5);
  mal(() => convert("x", "1.5", "Float"), /`x` is `Float`/);
});

test("un decimal es la cadena de sus cifras, nunca un number", () => {
  assert.equal(convert("d", "41.31", "Decimal<5, 2>"), "41.31");
  assert.equal(convert("d", 41.31, "Decimal<5, 2>"), "41.31");
  mal(() => convert("d", "41.315", "Decimal<5, 2>"), /does not fit/);
  mal(() => convert("d", "1234.5", "Decimal<5, 2>"), /does not fit/);
  assert.equal(convert("m", "12.50", "Money<EUR, 2>"), "12.50");
  mal(() => convert("m", "12.505", "Money<EUR, 2>"), /more than 2 decimals/);
  mal(() => convert("d", "doce", "Decimal"), /not a number/);
});

test("fechas: de calendario como cadena, un instante como Date", () => {
  assert.equal(convert("f", "2026-10-03", "Date"), "2026-10-03");
  mal(() => convert("f", "2026-02-30", "Date"), /YYYY-MM-DD/);
  assert.equal(convert("h", "08:30:15", "Time"), "08:30:15");
  assert.equal(convert("t", "2026-10-03T08:30:00", "DateTime"), "2026-10-03T08:30:00");
  mal(() => convert("t", "2026-10-03T08:30:00Z", "DateTime"), /without a zone/);
  const d = convert("i", "2026-10-03T08:30:00+02:00", "DateTimeTz");
  assert.ok(d instanceof Date);
  assert.equal(d.toISOString(), "2026-10-03T06:30:00.000Z");
  mal(() => convert("i", "2026-10-03T08:30:00", "DateTimeTz"), /an instant carries/);
});

test("bytes, estructuras, listas y referencias", () => {
  assert.deepEqual([...convert("b", "aGk=", "Opaque")], [104, 105]);
  mal(() => convert("b", "no*base64", "Opaque"), /base64/);
  assert.deepEqual(convert("p", { id: "7", total: "1.50" }, "Struct<id: BigInt, total: Decimal<5, 2>>"), { id: 7n, total: "1.50" });
  mal(() => convert("p", { id: 1, otro: 2 }, "Struct<id: Integer>"), /does not declare `otro`/);
  mal(() => convert("l", [1, null], "list<Integer>"), /`l\[1\]` is missing/);
  const ref = { collection: "legal.archivo.contratos", path: "a.pdf" };
  assert.equal(convert("c", ref, "Media<legal.archivo.contratos>"), ref);
  mal(() => convert("c", { ...ref, collection: "otra.b.c" }, "Media<legal.archivo.contratos>"), /an item of `otra.b.c`/);
});

test("la llamada: por nombre, la fila primero, y lo devuelto comprobado", async () => {
  const firma = {
    input: [
      { nombre: "texto", type: "String", required: true },
      { nombre: "veces", type: "Integer" },
    ],
    output: { type: "String" },
  };
  async function repeat(texto, veces = 1) {
    return Array(veces).fill(texto).join(" ");
  }
  assert.equal(await call(repeat, firma, { texto: "a", veces: "2" }), "a a");
  assert.equal(await call(repeat, firma, { texto: "a" }), "a");
  await assert.rejects(call(repeat, firma, { veces: 2 }), (e) => e.side === "input" && e.parameter === "texto");
  await assert.rejects(call(repeat, firma, { texto: "a", otra: 1 }), /no parameter `otra`/);
  await assert.rejects(call(() => 3, firma, { texto: "a" }), (e) => e.side === "output" && /declares `String`/.test(e.message));

  const sobre = { over: "ventas.clientes", input: [{ name: "n", type: "Integer", required: true }], output: { campos: [{ nombre: "total", type: "Decimal<5, 2>", required: true }, { nombre: "nota", type: "String" }] } };
  const r = await call((fila, n) => ({ total: (fila.base * n).toFixed(2) }), sobre, { n: 2 }, { row: { base: 1.25 } });
  assert.deepEqual(r, { total: "2.50", nota: null });
  await assert.rejects(call(() => ({ total: "1.00", sobra: 1 }), sobre, { n: 1 }), /does not declare/);
  await assert.rejects(call(() => ({ nota: "x" }), sobre, { n: 1 }), /without `total`, which is required/);
});

test("lo que vuelve viaja en JSON", () => {
  assert.deepEqual(toWire({ a: 1n, b: new Date("2026-10-03T00:00:00Z"), c: new Uint8Array(3), d: [2n] }), {
    a: "1",
    b: "2026-10-03T00:00:00.000Z",
    c: { bytes: 3 },
    d: ["2"],
  });
  assert.equal(checkOutput("f", undefined, { type: "String" }), undefined);
});
