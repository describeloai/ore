// ORE 0068 · cierre de C1 · Los ejes por la API del SDK de Node (`sql`), sin servidor.
import { sql } from "/opt/ore/ore/index.mjs";

const [{ p }] = await sql("select ST_AsText(ST_Transform(ST_Point(440000, 4474000), 'EPSG:25830', 'EPSG:4326')) p");
const [{ d }] = await sql("select ST_Distance_Spheroid(ST_Point(-3.7072, 40.4145), ST_Point(2.1734, 41.3851)) / 1000 d");
const ok = String(p).startsWith("POINT (-3.70") && d > 500 && d < 515;
console.log(`${ok ? "OK   " : "FALLA"} · node · ${p} · Sol–Barcelona ${Number(d).toFixed(1)} km`);
