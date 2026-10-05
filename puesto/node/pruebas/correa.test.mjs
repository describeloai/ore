// La correa de Node (0050 L3·3): `node --test puesto/node/pruebas/`.
//
// Un `ore-serve` de mentira —la ficha del puesto, el índice del árbol, sus
// ficheros, el flujo de eventos y la salida— contra el servidor de TypeScript DE
// VERDAD (`ORE_TIPOS_IMAGEN`, o `/opt/ore/tipos/node_modules` en la imagen). Sin
// él, la prueba se salta: lo que se prueba es la correa con su servidor.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const AQUI = dirname(fileURLToPath(import.meta.url));
const TIPOS = process.env.ORE_TIPOS_IMAGEN ?? "/opt/ore/tipos/node_modules";
const HAY_SERVIDOR = existsSync(join(TIPOS, "typescript-language-server", "lib", "cli.mjs"));

// Lo que la imagen trae para correr: `ore`, como en `/opt/ore/node_modules`.
const imagen = mkdtempSync(join(tmpdir(), "imagen-"));
symlinkSync(join(AQUI, "..", "ore"), join(imagen, "ore"), process.platform === "win32" ? "junction" : "dir");
process.env.ORE_IMAGEN_NODE ??= imagen;
process.env.ORE_CAPA_NODE ??= join(imagen, "no-hay-capa");
// La caja de tipos de la capa: una `devDependency` de mentira, con sus tipos.
const devDep = join(imagen, "tipos", "node_modules", "fake-dev");
mkdirSync(devDep, { recursive: true });
writeFileSync(join(devDep, "package.json"), '{ "name": "fake-dev", "version": "1.0.0", "type": "module", "main": "index.js", "types": "index.d.ts" }');
writeFileSync(join(devDep, "index.js"), "export const doble = (n) => n * 2n;\n");
writeFileSync(join(devDep, "index.d.ts"), "export declare const doble: (n: bigint) => bigint;\n");
process.env.ORE_TIPOS_NODE ??= join(imagen, "tipos", "node_modules");
const { Correa, enDisco, leerTsc, registroDe, TOPE_REGISTRO } = await import("../correa.mjs");

const REPO = "packages/ventas/riesgo";
const FN = `${REPO}/functions/riesgoInvoiceStatus.ts`;
const ARBOL = {
  [`${REPO}/README.md`]: "---\nnombre: Riesgo\n---\n",
  [`${REPO}/package.json`]: '{ "private": true, "type": "module", "dependencies": {} }\n',
  [`${REPO}/tsconfig.json`]: JSON.stringify({
    compilerOptions: { target: "esnext", module: "nodenext", strict: true, noEmit: true, erasableSyntaxOnly: true, verbatimModuleSyntax: true, allowImportingTsExtensions: true, skipLibCheck: true },
    include: ["**/*.ts"],
  }),
  [`${REPO}/functions/helpers.ts`]: "export function cents(d: string): bigint { return BigInt(d.replace('.', '')); }\n",
  // L5: sus pruebas, con una `devDependency` (fake-dev) y un `describe`.
  [`${REPO}/functions/helpers.test.ts`]: [
    'import { test, describe } from "node:test";',
    'import assert from "node:assert/strict";',
    'import { doble } from "fake-dev";',
    'import { cents } from "./helpers.ts";',
    'test("cents", () => { assert.equal(doble(cents("1.00")), 200n); });',
    'test("falla", () => { console.log("imprime"); assert.equal(cents("2.00"), 300n); });',
    'describe("grupo", () => { test("dentro", () => {}); test.skip("saltada", () => {}); });',
    "",
  ].join("\n"),
  [`${REPO}/functions/borrada.test.ts`]: 'import { test } from "node:test";\ntest("de la rama", () => {});\n',
  [FN]: [
    'import type { Decimal } from "ore";',
    'import { cents } from "./helpers.ts";',
    "enum Estado { pagada }",
    "const n: number = cents('1.00');",
    "/** x */",
    "export default function riesgoInvoiceStatus(a: Decimal<12, 2>): string { return a + n + Estado.pagada; }",
    "",
  ].join("\n"),
};

test("cada mensaje del editor cuenta como actividad (el TTL del puesto)", async () => {
  let n = 0;
  const c = new Correa({ id: "x", pedir: async () => [200, {}] }, { cabeceras: async () => ({}) }, { alActividad: () => { n++; } });
  c.entregarAlServidor = async () => {};
  c.probar = async () => {};
  c.comprobar = async () => {};
  await c.escribir('{"jsonrpc":"2.0","method":"textDocument/didChange","params":{}}');
  await c.escribir('{"jsonrpc":"2.0","id":1,"method":"ore/probar","params":{}}');
  await c.escribir('{"jsonrpc":"2.0","id":2,"method":"ore/comprobar","params":{}}');
  assert.equal(n, 3);
  // Sin retrollamada, como antes: nada que contar, nada que romper.
  const s = new Correa({ id: "x", pedir: async () => [200, {}] }, { cabeceras: async () => ({}) });
  s.entregarAlServidor = async () => {};
  await s.escribir('{"jsonrpc":"2.0","method":"initialized","params":{}}');
});

test("node(): además de la lista, el registro de spec, como en una terminal", { timeout: 60_000 }, async () => {
  const dir = mkdtempSync(join(tmpdir(), "ore-registro-"));
  try {
    mkdirSync(join(dir, "r"), { recursive: true });
    writeFileSync(join(dir, "r", "a.test.mjs"), [
      'import { test } from "node:test";',
      'import assert from "node:assert/strict";',
      'test("suma", () => { console.log("hola desde la prueba"); assert.equal(1 + 1, 2); });',
      'test("falla", () => { assert.equal(2 + 2, 5); });',
    ].join("\n"));
    const c = new Correa({ id: "x", pedir: async () => [200, {}] }, { cabeceras: async () => ({}) }, { trabajo: dir });
    const r = await c.node(["r/a.test.mjs"], []);
    assert.deepEqual(r.pruebas.map((p) => `${p.nombre}:${p.estado}`).sort(), ["falla:fallo", "suma:ok"]);
    assert.match(r.registro, /^\$ node --test r\/a\.test\.mjs/);
    assert.match(r.registro, /hola desde la prueba/);
    assert.match(r.registro, /✔ suma/);
    assert.match(r.registro, /✖ falla/);
    assert.match(r.registro, /ℹ tests 2/);
    assert.match(r.registro, /exit code 1\n$/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("registroDe: el comando, stderr si el reporter no lo dijo, y recortado por delante", () => {
  const r = registroDe({ ficheros: ["r/a.test.ts"], nombres: ["suma"], spec: "✔ suma (1ms)\n", errores: "SyntaxError: algo", codigo: 1 });
  assert.equal(r, '$ node --test --test-name-pattern="^suma$" r/a.test.ts\n\n✔ suma (1ms)\n\nSyntaxError: algo\n\nexit code 1\n');
  assert.match(registroDe({ ficheros: [], vencido: true, tope: 3000 }), /stopped after 3 s/);
  const largo = registroDe({ ficheros: ["x"], spec: "y".repeat(TOPE_REGISTRO + 5000) });
  assert.ok(largo.length <= TOPE_REGISTRO + 80 && largo.startsWith("… ("), largo.slice(0, 60));
});

test("enDisco: sólo lo de dentro, y nunca node_modules", () => {
  const t = join(tmpdir(), "t");
  assert.equal(enDisco(pathToFileURL(join(t, "packages", "a.ts")).href, t), join(t, "packages", "a.ts"));
  assert.equal(enDisco(pathToFileURL(join(t, "..", "fuera.ts")).href, t), null);
  assert.equal(enDisco(pathToFileURL(join(t, "packages", "node_modules", "x.ts")).href, t), null);
  assert.equal(enDisco("untitled:1", t), null);
});

test("leerTsc: los diagnósticos de tsc, con su fichero, su línea y su mensaje entero", () => {
  const r = leerTsc([
    "packages/a/r/functions/f.ts(3,1): error TS1294: This syntax is not allowed when 'erasableSyntaxOnly' is enabled.",
    "packages\\a\\r\\functions\\g.ts(4,7): error TS2322: Type 'bigint' is not assignable to type 'number'.",
    "  Something more about it.",
    "error TS5083: Cannot read file 'x/tsconfig.json'.",
    "",
  ].join("\n"));
  assert.equal(r.errores, 3);
  assert.equal(r.ficheros, 2);
  assert.deepEqual(r.diagnosticos[0], { fichero: "packages/a/r/functions/f.ts", linea: 3, columna: 1, codigo: "TS1294", mensaje: "This syntax is not allowed when 'erasableSyntaxOnly' is enabled.", severidad: "error" });
  assert.equal(r.diagnosticos[1].fichero, "packages/a/r/functions/g.ts");
  assert.match(r.diagnosticos[1].mensaje, /\nSomething more about it\.$/);
  assert.equal(r.diagnosticos[2].fichero, undefined);
  assert.deepEqual(leerTsc(""), { diagnosticos: [], errores: 0, ficheros: 0 });
});

test("la correa: el repositorio en disco, su tsconfig, sus vecinos y los tipos de ore", { skip: !HAY_SERVIDOR && "sin typescript-language-server", timeout: 90_000 }, async () => {
  const trabajo = mkdtempSync(join(tmpdir(), "trabajo-"));
  const pedidos = [];
  const entradas = [];
  const salidas = [];
  let flujo = null;
  const srv = createServer((req, res) => {
    const u = new URL(req.url, "http://x");
    pedidos.push(`${req.method} ${u.pathname} ${req.headers["x-ore-raiz"] ?? ""}`.trim());
    const json = (c, j) => { res.writeHead(c, { "content-type": "application/json" }); res.end(JSON.stringify(j)); };
    if (u.pathname === "/puestos/p1") return json(200, { id: "p1", repositorio: REPO, rama: "main" });
    if (u.pathname === "/arbol") {
      assert.equal(req.headers["x-ore-raiz"], REPO);
      return json(200, { ficheros: Object.entries(ARBOL).map(([ruta, t]) => ({ ruta, bytes: t.length })) });
    }
    if (u.pathname.startsWith("/arbol/")) {
      const ruta = decodeURIComponent(u.pathname.slice(7));
      return ruta in ARBOL ? json(200, { ruta, texto: ARBOL[ruta] }) : json(404, { error: "no" });
    }
    if (u.pathname === "/puestos/p1/lsp/agente") {
      res.writeHead(200, { "content-type": "text/event-stream" });
      flujo = res;
      for (const m of entradas.splice(0)) res.write(`event: lsp\ndata: ${m}\n\n`);
      return;
    }
    if (u.pathname === "/puestos/p1/lsp/salida") {
      let b = "";
      req.on("data", (d) => (b += d));
      req.on("end", () => { salidas.push(...JSON.parse(b).mensajes.map((x) => JSON.parse(x))); json(202, {}); });
      return;
    }
    json(404, { error: u.pathname });
  });
  await new Promise((ok) => srv.listen(0, "127.0.0.1", ok));
  const p = {
    id: "p1",
    servidor: `http://127.0.0.1:${srv.address().port}`,
    async pedir(metodo, ruta, cuerpo, plazo = 30_000, cab = {}) {
      const r = await fetch(this.servidor + ruta, { method: metodo, headers: { "content-type": "application/json", ...cab }, body: cuerpo === undefined ? undefined : JSON.stringify(cuerpo) });
      const t = await r.text();
      return [r.status, t ? JSON.parse(t) : null];
    },
  };
  const correa = new Correa(p, { cabeceras: async () => ({}) }, { trabajo });
  const mandar = (m) => { const s = JSON.stringify({ jsonrpc: "2.0", ...m }); if (flujo) flujo.write(`event: lsp\ndata: ${s}\n\n`); else entradas.push(s); };
  const esperar = async (pred, ms = 60_000, desde = 0) => {
    const t0 = Date.now();
    for (;;) {
      const x = salidas.slice(desde).find(pred);
      if (x) return x;
      if (Date.now() - t0 > ms) throw new Error(`no llegó: ${JSON.stringify(salidas).slice(0, 400)}`);
      await new Promise((ok) => setTimeout(ok, 50));
    }
  };
  try {
    void correa.escuchar();
    const uri = pathToFileURL(join(trabajo, FN)).href;
    const mismo = (a) => decodeURIComponent(a ?? "").toLowerCase() === decodeURIComponent(uri).toLowerCase();
    mandar({ id: 1, method: "initialize", params: { processId: null, rootUri: pathToFileURL(trabajo).href, capabilities: { textDocument: { publishDiagnostics: {}, hover: { contentFormat: ["markdown"] } } } } });
    mandar({ method: "initialized", params: {} });
    mandar({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId: "typescript", version: 1, text: ARBOL[FN] } } });
    await esperar((m) => m.id === 1 && m.result);

    // 1 · el repositorio, en disco: el código, el tsconfig y el package.json; lo demás no.
    for (const r of [FN, `${REPO}/functions/helpers.ts`, `${REPO}/tsconfig.json`, `${REPO}/package.json`]) {
      assert.ok(existsSync(join(trabajo, r)), r);
    }
    assert.ok(!existsSync(join(trabajo, REPO, "README.md")));
    assert.ok(existsSync(join(trabajo, "packages", "node_modules", "ore")), "el árbol de tipos tiene ore");
    assert.ok(existsSync(join(trabajo, "packages", "node_modules", "@types", "node")), "y los tipos de Node");

    // 2 · lo que dice el servidor es lo que diría tsc: el enum (sólo con el
    //   tsconfig en disco), el bigint en un number, y el vecino SÍ se resuelve.
    const d = await esperar((m) => m.method === "textDocument/publishDiagnostics" && mismo(m.params.uri) && m.params.diagnostics.length);
    const codigos = d.params.diagnostics.map((x) => x.code);
    assert.ok(codigos.includes(1294), `TS1294 (enum): ${JSON.stringify(codigos)}`);
    assert.ok(codigos.includes(2322), `TS2322 (bigint → number): ${JSON.stringify(codigos)}`);
    assert.ok(!codigos.includes(2307), `el vecino se resuelve: ${JSON.stringify(codigos)}`);

    // 3 · los tipos de ore, al pasar por encima.
    mandar({ id: 2, method: "textDocument/hover", params: { textDocument: { uri }, position: { line: 0, character: 15 } } });
    const h = await esperar((m) => m.id === 2);
    assert.match(JSON.stringify(h.result), /type Decimal<P extends number/);

    // 4 · el espejo: lo que el editor cambia, también en disco.
    const nuevo = ARBOL[FN].replace("enum Estado { pagada }", 'const Estado = { pagada: "pagada" } as const;');
    const antes = salidas.length;
    mandar({ method: "textDocument/didChange", params: { textDocument: { uri, version: 2 }, contentChanges: [{ text: nuevo }] } });
    // Lo que llegue DESPUÉS del cambio: sigue el TS2322 y ya no está el enum.
    const tras = await esperar((m) => m.method === "textDocument/publishDiagnostics" && mismo(m.params.uri) && m.params.diagnostics.length, 60_000, antes);
    assert.deepEqual(tras.params.diagnostics.map((x) => x.code).filter((c) => c === 1294 || c === 2322), [2322]);
    assert.equal(readFileSync(join(trabajo, FN), "utf8"), nuevo);

    // 5 · L4 · `ore/comprobar`: tsc sobre el repositorio TAL COMO ESTÁ EN LA RAMA
    //   (el enum vuelve: lo de la pestaña no estaba guardado), con su árbol de tipos.
    mandar({ id: 3, method: "ore/comprobar", params: {} });
    const c = await esperar((m) => m.id === 3, 120_000);
    assert.equal(c.result.repositorio, REPO, JSON.stringify(c.result));
    const suyos = c.result.diagnosticos.filter((x) => x.fichero === FN).map((x) => `${x.codigo}@${x.linea}`);
    assert.ok(suyos.includes("TS1294@3"), JSON.stringify(c.result));
    assert.ok(suyos.includes("TS2322@4"), JSON.stringify(c.result));
    assert.ok(!c.result.diagnosticos.some((x) => x.codigo === "TS2307"), "ore y el vecino se resuelven");
    assert.equal(c.result.ficheros, 1);
    assert.ok(c.result.ms > 0);

    // 6 · L5 · `ore/probar`: lo guardado MÁS los borradores de la consola; una
    //   prueba que pasa (con su devDependency y su vecino), una que falla con lo
    //   esperado y lo obtenido, una saltada en su `describe`, un borrador nuevo,
    //   un fichero que no carga, y lo que la rama ya no tiene, fuera.
    delete ARBOL[`${REPO}/functions/borrada.test.ts`];
    const borradores = [
      { ruta: `${REPO}/functions/nuevo.test.ts`, texto: 'import { test } from "node:test";\ntest("sin guardar", () => {});\n' },
      { ruta: `${REPO}/functions/roto.spec.ts`, texto: "const x: number = ;\n" },
      { ruta: "packages/otro/fuera.test.ts", texto: "no es de este repositorio" },
    ];
    mandar({ id: 4, method: "ore/probar", params: { borradores } });
    const t = (await esperar((m) => m.id === 4, 120_000)).result;
    assert.equal(t.repositorio, REPO, JSON.stringify(t));
    assert.deepEqual(t.todos, [`${REPO}/functions/helpers.test.ts`, `${REPO}/functions/nuevo.test.ts`, `${REPO}/functions/roto.spec.ts`]);
    assert.ok(!existsSync(join(trabajo, REPO, "functions", "borrada.test.ts")), "lo que la rama no tiene, fuera");
    assert.ok(!existsSync(join(trabajo, "packages", "otro")), "un borrador de otro repositorio no entra");
    const de = (n) => t.pruebas.find((p) => p.nombre === n);
    assert.equal(de("cents").estado, "ok", JSON.stringify(t.pruebas));
    assert.equal(de("falla").estado, "fallo");
    assert.equal(de("falla").linea, 6);
    assert.equal(de("falla").esperado, "300n");
    assert.equal(de("falla").obtenido, "200n");
    assert.deepEqual(de("dentro").ruta, ["grupo"]);
    assert.equal(de("saltada").estado, "saltada");
    assert.equal(de("sin guardar").estado, "ok");
    const roto = t.ficheros.find((f) => f.ruta === `${REPO}/functions/roto.spec.ts`);
    assert.equal(roto.estado, "error", JSON.stringify(t.ficheros));
    assert.match(roto.mensaje, /Expression expected/);
    assert.match(t.ficheros.find((f) => f.ruta === `${REPO}/functions/helpers.test.ts`).salida, /imprime/);
    assert.deepEqual(t.resumen, { total: 5, ok: 3, fallo: 1, saltada: 1, ficherosConError: 1 });

    // 7 · y sólo lo pedido: un fichero, y en él una prueba por su nombre.
    mandar({ id: 5, method: "ore/probar", params: { ficheros: [`${REPO}/functions/helpers.test.ts`], nombres: ["cents"] } });
    const u = (await esperar((m) => m.id === 5, 120_000)).result;
    assert.deepEqual(u.pruebas.filter((p) => p.estado !== "saltada").map((p) => p.nombre), ["cents"], JSON.stringify(u.pruebas));

    // 8 · un bucle infinito no cuelga el puesto: el tope lo para, con su grupo de
    //   procesos. Sólo en Linux (la imagen): en Windows matar al corredor deja
    //   huérfano al proceso del fichero, girando.
    if (process.platform !== "win32") {
      process.env.ORE_TOPE_PRUEBAS_MS = "3000";
      const bucle = [{ ruta: `${REPO}/functions/bucle.test.ts`, texto: 'import { test } from "node:test";\ntest("gira", () => { for (;;) {} });\n' }];
      mandar({ id: 6, method: "ore/probar", params: { borradores: bucle, ficheros: [`${REPO}/functions/bucle.test.ts`] } });
      const b = (await esperar((m) => m.id === 6, 30_000)).result;
      delete process.env.ORE_TOPE_PRUEBAS_MS;
      assert.match(b.error ?? "", /tardaron más de 3 s/, JSON.stringify(b));
      assert.ok(b.ms < 15_000, `${b.ms} ms`);
    }
  } finally {
    correa.parar();
    flujo?.end();
    srv.closeAllConnections?.();
    srv.close();
    try { rmSync(trabajo, { recursive: true, force: true }); } catch { /* Windows retiene lo abierto */ }
  }
});
