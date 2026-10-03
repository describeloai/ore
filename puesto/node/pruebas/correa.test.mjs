// La correa de Node (0050 L3·3): `node --test puesto/node/pruebas/`.
//
// Un `ore-serve` de mentira —la ficha del puesto, el índice del árbol, sus
// ficheros, el flujo de eventos y la salida— contra el servidor de TypeScript DE
// VERDAD (`ORE_TIPOS_IMAGEN`, o `/opt/ore/tipos/node_modules` en la imagen). Sin
// él, la prueba se salta: lo que se prueba es la correa con su servidor.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from "node:fs";
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
process.env.ORE_TIPOS_NODE ??= join(imagen, "no-hay-tipos");
const { Correa, enDisco } = await import("../correa.mjs");

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

test("enDisco: sólo lo de dentro, y nunca node_modules", () => {
  const t = join(tmpdir(), "t");
  assert.equal(enDisco(pathToFileURL(join(t, "packages", "a.ts")).href, t), join(t, "packages", "a.ts"));
  assert.equal(enDisco(pathToFileURL(join(t, "..", "fuera.ts")).href, t), null);
  assert.equal(enDisco(pathToFileURL(join(t, "packages", "node_modules", "x.ts")).href, t), null);
  assert.equal(enDisco("untitled:1", t), null);
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
  } finally {
    correa.parar();
    flujo?.end();
    srv.closeAllConnections?.();
    srv.close();
    try { rmSync(trabajo, { recursive: true, force: true }); } catch { /* Windows retiene lo abierto */ }
  }
});
