// EL EDITOR DE SQL · el cliente de la consola DE VERDAD, fuera del navegador.
//
// Carga `components/code-workspace/servidor-de-lenguaje.ts` de la consola tal
// cual (Node quita los tipos: --experimental-strip-types) y le da lo que el
// navegador le daria: un Monaco de mentira que apunta lo que el cliente pinta y
// registra, `fetch` y un `EventSource` que van a un ore-serve de verdad (lo que
// hace el proxy `/api/puestos/...` de la consola, con la persona en la cabecera).
// Lo lanza `el-editor-sql.py`, que levanta ore-serve y el agente de verdad.
//
//   node --experimental-strip-types el-editor-sql.mjs <consola> <base> <puesto> <persona>
const [consola, BASE, PUESTO, PERSONA] = process.argv.slice(2);
const dormir = (ms) => new Promise((r) => setTimeout(r, ms));

// ── lo que el proxy de la consola hace: /api/puestos/... → ore-serve ─────────
const cabeceras = { 'x-ore-sujeto': PERSONA };
const aServe = (url) => BASE + String(url).replace(/^\/api/, '').replace(/\/lsp\/flujo$/, '/lsp/consola');
const fetchDeVerdad = globalThis.fetch;
globalThis.fetch = (url, o = {}) => fetchDeVerdad(aServe(url), { ...o, headers: { ...(o.headers || {}), ...cabeceras } });

class EventSource {
  static CLOSED = 2;
  constructor(url) {
    this.readyState = 0;
    this.oyentes = {};
    this.ctl = new AbortController();
    void this.leer(url);
  }
  addEventListener(ev, f) { (this.oyentes[ev] ||= []).push(f); }
  close() { this.readyState = 2; this.ctl.abort(); }
  async leer(url) {
    try {
      const r = await fetchDeVerdad(aServe(url), { headers: { ...cabeceras, accept: 'text/event-stream' }, signal: this.ctl.signal });
      this.readyState = 1;
      const dec = new TextDecoder();
      let resto = '', ev = 'message';
      for await (const trozo of r.body) {
        resto += dec.decode(trozo, { stream: true });
        let i;
        while ((i = resto.indexOf('\n')) >= 0) {
          const l = resto.slice(0, i).replace(/\r$/, '');
          resto = resto.slice(i + 1);
          if (l.startsWith('event: ')) ev = l.slice(7);
          else if (l.startsWith('data: ')) for (const f of this.oyentes[ev] || []) f({ data: l.slice(6) });
          else if (l === '') ev = 'message';
        }
      }
    } catch { /* cerrado */ }
  }
}
globalThis.EventSource = EventSource;

// ── un Monaco de mentira: lo que el cliente registra y pinta ────────────────
const proveedores = { hover: {}, completion: {} };
const marcas = new Map(); // uri del modelo → [marcas]
const modelos = [];
const monaco = {
  MarkerSeverity: { Error: 8, Warning: 4, Info: 2 },
  languages: {
    registerHoverProvider: (l, p) => { proveedores.hover[l] = p; return { dispose() {} }; },
    registerCompletionItemProvider: (l, p) => { proveedores.completion[l] = p; return { dispose() {} }; },
  },
  editor: {
    getModels: () => modelos,
    setModelMarkers: (m, dueno, xs) => marcas.set(m.uri.path, { dueno, xs }),
  },
};
function modelo(ruta, texto) {
  const m = {
    texto,
    uri: { path: '/' + ruta, toString: () => 'inmemory://model/' + ruta },
    getValue() { return this.texto; },
    getWordUntilPosition: (p) => ({ startColumn: p.column, endColumn: p.column }),
  };
  modelos.push(m);
  return m;
}

const { servidorDeLenguaje } = await import('file:///' + consola.replace(/\\/g, '/') + '/components/code-workspace/servidor-de-lenguaje.ts');
const R = {};
const esperar = async (f, ms = 8000) => { const t = Date.now(); while (Date.now() - t < ms) { const v = f(); if (v) return v; await dormir(50); } return null; };

// 1 · el cliente de SQL abre un .sql
const sql = servidorDeLenguaje(PUESTO, monaco, 'sql');
const ms = modelo('consultas/a.sql', 'select * from ');
await sql.abrir('consultas/a.sql', ms.texto);
const pos = (m, col) => ({ lineNumber: 1, column: col });
R.registrado_sql = !!proveedores.completion.sql && !!proveedores.hover.sql;
let r = await proveedores.completion.sql.provideCompletionItems(ms, pos(ms, 15));
R.tras_from = r.suggestions.map((s) => s.label);

// 2 · EL ARREGLO: el modelo cambia y se pide completion YA, antes de que
//     ArbolFileView mande el texto (200 ms después). Sin el arreglo, el servidor
//     tendría 'select * from ' y ofrecería nombres, no columnas.
ms.texto = 'select v. from hr.ventas v';
r = await proveedores.completion.sql.provideCompletionItems(ms, pos(ms, 10));
R.alias_sin_esperar = r.suggestions.map((s) => s.label);

// 3 · el error: una columna mal escrita, pintada en su sitio
ms.texto = 'select v.totl from hr.ventas v';
sql.cambiar('consultas/a.sql', ms.texto);
const pintado = await esperar(() => { const x = marcas.get('/consultas/a.sql'); return x && x.xs.length ? x : null; });
R.marca = pintado && { dueno: pintado.dueno, n: pintado.xs.length, linea: pintado.xs[0].startLineNumber, col: pintado.xs[0].startColumn, sev: pintado.xs[0].severity, source: pintado.xs[0].source, msg: pintado.xs[0].message };

// 4 · el hover del dataset
const h = await proveedores.hover.sql.provideHover(ms, pos(ms, 22));
R.hover = h ? h.contents[0].value : null;

// 5 · ¿arrancó pyright por abrir un .sql? (lo dice el log del agente: lo mira el .py)
console.log('MARCA antes-del-py');
await dormir(300);

// 6 · el cliente de Python, en el mismo puesto y el mismo canal
const py = servidorDeLenguaje(PUESTO, monaco, 'python');
R.dos_clientes = py !== sql;
const mp = modelo('transforms/x.py', 'x = 1\n');
await py.abrir('transforms/x.py', mp.texto);
r = await proveedores.completion.python.provideCompletionItems(mp, pos(mp, 2));
R.python = r.suggestions.map((s) => s.label);
await dormir(1500);
R.marcas_py = (marcas.get('/transforms/x.py') || { xs: [] }).xs.map((x) => x.message);
R.marcas_sql_final = (marcas.get('/consultas/a.sql') || { xs: [] }).xs.length;

console.log('RESULTADO ' + JSON.stringify(R));
sql.apagar();
py.apagar();
process.exit(0);
