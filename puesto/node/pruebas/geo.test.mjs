// La geometría en el SDK de Node (ORE 0067 G2·3·4): `node --test "puesto/node/pruebas/*.test.mjs"`.
// En la imagen del puesto de Node `spatial` está (/opt/ore/duckdb): `bash pruebas-de-fuego/geo/c1/node-en-la-imagen.sh`. Fuera, sin él, las de DuckDB se saltan.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { __geo } from "../ore/index.mjs";

const { conGeo, crsDeGeo, parquetDe_, duckdb } = __geo;

test("el CRS de una columna geo, por su tipo", () => {
  assert.equal(crsDeGeo("geometry"), "OGC:CRS84");
  assert.equal(crsDeGeo("geometry(EPSG:25830)"), "EPSG:25830");
  assert.equal(crsDeGeo("Geometry<Point, EPSG:25830>"), "EPSG:25830");
  assert.equal(crsDeGeo("Geometry<Point>"), "OGC:CRS84");
  assert.equal(crsDeGeo("Geography<LineString>"), "OGC:CRS84");
  assert.equal(crsDeGeo("string"), null);
  assert.equal(crsDeGeo(undefined), null);
});

// Un cuadrado de 1000 × 1000 en EPSG:25830, en WKB.
function cuadrado() {
  const b = Buffer.alloc(1 + 4 + 4 + 4 + 5 * 16);
  let o = 0;
  b.writeUInt8(1, o); o += 1; b.writeUInt32LE(3, o); o += 4; b.writeUInt32LE(1, o); o += 4; b.writeUInt32LE(5, o); o += 4;
  for (const [x, y] of [[440000, 4474000], [441000, 4474000], [441000, 4475000], [440000, 4475000], [440000, 4474000]]) {
    b.writeDoubleLE(x, o); o += 8; b.writeDoubleLE(y, o); o += 8;
  }
  return b;
}

async function conSpatial() {
  try { await (await duckdb()).run("load spatial"); return true; } catch { return false; }
}

test("write(): la geometría sale en el Parquet como GEOMETRY con su CRS", async (t) => {
  if (!(await conSpatial())) return t.skip("sin spatial");
  const parquet = await parquetDe_(["id", "geom"], { id: "string", geom: "geometry(EPSG:25830)" }, [["a", "b"], [cuadrado(), null]]);
  const f = join(tmpdir(), `geo-${Date.now()}.parquet`);
  writeFileSync(f, parquet);
  const con = await duckdb();
  const lt = (await con.runAndReadAll(`select logical_type from parquet_schema('${f}') where name = 'geom'`)).getRows()[0][0];
  assert.match(String(lt), /GeometryType/);
  assert.match(String(lt), /25830/);
  const area = (await con.runAndReadAll(`select ST_Area(geom) from '${f}' where id = 'a'`)).getRows()[0][0];
  assert.equal(area, 1_000_000);
});

test("leer: con `geo` en el puntero, el BLOB es GEOMETRY con su CRS", async (t) => {
  if (!(await conSpatial())) return t.skip("sin spatial");
  const con = await duckdb();
  const f = join(tmpdir(), `blob-${Date.now()}.parquet`);
  await con.run(`copy (select 'a' id, from_hex('${cuadrado().toString("hex")}') geom) to '${f}' (format parquet)`);
  const sinGeo = await conGeo(`read_parquet('${f}')`, {});
  assert.equal(String((await con.runAndReadAll(`select typeof(geom) from ${sinGeo}`)).getRows()[0][0]), "BLOB");
  const fuente = await conGeo(`read_parquet('${f}')`, { geo: { geom: { type: "Geometry<Polygon, EPSG:25830>", crs: "EPSG:25830" } } });
  const [[tipo, area]] = (await con.runAndReadAll(`select typeof(geom), ST_Area(geom) from ${fuente}`)).getRows();
  assert.match(String(tipo), /GEOMETRY/);
  assert.equal(area, 1_000_000);
});

test("intersectsBox: el filtro lleva && y ST_Intersects, y filtra", async (t) => {
  const { intersectsBox } = await import("../ore/index.mjs");
  assert.equal(intersectsBox("t.geom", 0, 0, 5, 5), '("t"."geom" && ST_MakeEnvelope(0, 0, 5, 5) AND ST_Intersects("t"."geom", ST_MakeEnvelope(0, 0, 5, 5)))');
  assert.throws(() => intersectsBox("geom", "1", 0, 2, 2), TypeError);
  assert.throws(() => intersectsBox("geom", 2, 0, 1, 2), RangeError);
  if (!(await conSpatial())) return t.skip("sin spatial");
  const con = await duckdb();
  const n = (await con.runAndReadAll(`select count(*) from (select ST_Point(1, 2) geom union all select ST_Point(10, 20)) where ${intersectsBox("geom", 0, 0, 5, 5)}`)).getRows()[0][0];
  assert.equal(Number(n), 1);
});

// ORE 0067, medido en victor: lo que escribe `write()` llega sin `geo` en el puntero; el SDK lo toma
// de las propiedades `ore.geo.<col>` de la tabla (la de `cuerpos/geo-v3-ore` la selló ORE).
test("sin geo en el puntero, lo toma de la tabla", async () => {
  const { readdirSync } = await import("node:fs");
  const raiz = process.env.ORE_CUERPOS ?? new URL("./cuerpos/geo-v3-ore", import.meta.url).pathname;
  const ml = join(raiz, "metadata", readdirSync(join(raiz, "metadata")).find((f) => f.endsWith(".metadata.json")));
  const geo = await __geo.geoDeLaTabla(ml);
  assert.equal(geo.geom.crs, "EPSG:25830");
  assert.equal(geo.geom.type, "Geometry<Any, EPSG:25830>");
});
