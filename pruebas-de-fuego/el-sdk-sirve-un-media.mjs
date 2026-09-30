// 0046 E9·4 · el SDK de Node sirve un `Media<c>`: `media`, `medias` y `mediaDe`, contra un
// ore-serve de mentira. Sin red ni cluster:  node pruebas-de-fuego/el-sdk-sirve-un-media.mjs
import http from "node:http";
import assert from "node:assert/strict";
const visto = [];
const srv = http.createServer((req, res) => {
  let b = "";
  req.on("data", (c) => (b += c));
  req.on("end", () => {
    const r = (c, d) => { const t = JSON.stringify(d); res.writeHead(c, { "content-length": Buffer.byteLength(t) }); res.end(t); };
    if (req.method === "POST") {
      const c = JSON.parse(b); visto.push(["POST", req.url, c.huellas.length]);
      return r(200, { items: c.huellas.filter((h) => h !== "nada").map((h) => ({ huella: h, url: "https://" + h })), segundos: Number(c.ttl ?? 300), caduca_ms: 1 });
    }
    visto.push(["GET", req.url, req.headers["x-ore-puesto"]]);
    if (req.url === "/puestos/p1/datos/legal.registro") return r(200, { media: { documento: "legal.archivo.contratos" } });
    if (req.url === "/colecciones/legal/archivo/contratos/items/crc64nvme%3Aab%2Fc%3D") return r(200, { url: "https://x", tipo: "application/pdf" });
    return r(404, { error: "ningún ítem de la colección lleva esa huella" });
  });
});
await new Promise((ok) => srv.listen(0, "127.0.0.1", ok));
process.env.ORE_SERVE = `http://127.0.0.1:${srv.address().port}`;
process.env.PUESTO = "p1";
const ore = await import(new URL("../puesto/node/ore/index.mjs", import.meta.url));
assert.deepEqual(await ore.mediaDe("legal.registro"), { documento: "legal.archivo.contratos" });
assert.equal((await ore.media("legal.archivo.contratos", "crc64nvme:ab/c=")).tipo, "application/pdf");
await assert.rejects(ore.media("legal.archivo.contratos", "otra"), (e) => e.codigo === 404);
const m = await ore.medias("legal.archivo.contratos", [...Array(250).keys()].map((i) => "h" + i).concat(["h0", "nada"]), { ttl: 60 });
assert.equal(Object.keys(m).length, 250);
assert.equal(m.h7.segundos, 60);
assert.deepEqual(visto.filter((v) => v[0] === "POST").map((v) => v[2]), [100, 100, 51]);
assert.equal((await ore.media("legal.archivo.contratos", "h1", { ttl: 45 })).url, "https://h1");
await assert.rejects(ore.media("contratos", "x"));
assert.ok(visto.filter((v) => v[0] === "GET").every((v) => v[2] === "p1"));
srv.close();
console.log("sdk node · media, medias y mediaDe en verde");
