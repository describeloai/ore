// `Binary` en el SDK de Node (OOS v1alpha35, ORE 0076): unos bytes son `Binary`,
// en DuckDB `BLOB`, en Iceberg `binary`, y en JSON base64.
//
//   node --test puesto/node/pruebas/binary.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { __binario, toJson, arrowName } from "../ore/index.mjs";

let duckdb = true;
try {
  await import("@duckdb/node-api");
} catch {
  duckdb = false;
}

const { tipoDuck, tipoIceberg, inferredType } = __binario;

test("unos bytes de una fila se infieren binary, sean Uint8Array o Buffer", () => {
  assert.equal(inferredType(new Uint8Array([1, 2])), "binary");
  assert.equal(inferredType(Buffer.from([1, 2])), "binary");
  assert.equal(inferredType("x"), "string");
});

test("binary es BLOB en DuckDB y binary en Iceberg", () => {
  assert.equal(tipoDuck("binary"), "BLOB");
  assert.equal(tipoIceberg("firma", "binary"), "binary");
  assert.equal(tipoIceberg("firma", "large_binary"), "binary");
  assert.equal(arrowName("BLOB"), "binary");
});

test("unos bytes salen en base64 en JSON", () => {
  assert.equal(toJson(new Uint8Array([0xde, 0xad, 0xbe, 0xef])), "3q2+7w==");
  assert.equal(toJson(new Uint8Array(0)), "");
});

test("write() saca un Parquet con una columna binary que DuckDB lee igual", { skip: !duckdb && "sin @duckdb/node-api" }, async () => {
  const { readFileSync, writeFileSync, mkdtempSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const bytes = await __binario.parquetDe_(
    ["id", "firma"],
    { id: "int64", firma: "binary" },
    [[1, 2, 3], [new Uint8Array([0xde, 0xad]), Buffer.from([]), null]],
  );
  const f = join(mkdtempSync(join(tmpdir(), "ore-bin-")), "t.parquet");
  writeFileSync(f, bytes);
  const con = await __binario.duckdb();
  const r = await con.runAndReadAll(`select id, firma, octet_length(firma) as n from read_parquet('${f.replaceAll("\\", "/")}') order by id`);
  const filas = r.getRowObjects().map((x) => ({ id: Number(x.id), firma: x.firma === null ? null : toJson(x.firma), n: x.n === null ? null : Number(x.n) }));
  assert.deepEqual(filas, [{ id: 1, firma: "3q0=", n: 2 }, { id: 2, firma: "", n: 0 }, { id: 3, firma: null, n: null }]);
  const t = await con.runAndReadAll(`select typeof(firma) from read_parquet('${f.replaceAll("\\", "/")}') limit 1`);
  assert.equal(t.getRows()[0][0], "BLOB");
  assert.ok(readFileSync(f).length > 0);
});

test("una columna binary con un valor que no son bytes se niega con su nombre", { skip: !duckdb && "sin @duckdb/node-api" }, async () => {
  await assert.rejects(
    __binario.parquetDe_(["firma"], { firma: "binary" }, [["no son bytes"]]),
    /column `firma` is binary: its values are bytes/,
  );
});
