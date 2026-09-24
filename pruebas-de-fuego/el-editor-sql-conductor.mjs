// EL CONDUCTOR · el cliente de la consola DE VERDAD, obedeciendo ordenes por stdin.
//
// Lo mismo que `el-editor-sql.mjs` (el `servidor-de-lenguaje.ts` de la consola
// tal cual, un Monaco de mentira, `fetch` y `EventSource` a un ore-serve de
// verdad) pero sin guion: lee una orden JSON por linea y contesta `R {json}`.
// Lo maneja `el-editor-sql-a-fondo.py` (P4 escala, P5 robustez, P6 canal).
//
//   node --experimental-strip-types el-editor-sql-conductor.mjs <consola> <base> <puesto> <persona>
import readline from 'node:readline';

const [consola, BASE, PUESTO, PERSONA] = process.argv.slice(2);
const dormir = (ms) => new Promise((r) => setTimeout(r, ms));
const ahora = () => performance.now();

// ── el proxy de la consola, y la cuenta de lo que cruza ─────────────────────
const cuenta = { posts: 0, bytes_post: 0, eventos: 0, bytes_eventos: 0, codigos: {} };
const cabeceras = { 'x-ore-sujeto': PERSONA };
const aServe = (url) => BASE + String(url).replace(/^\/api/, '').replace(/\/lsp\/flujo$/, '/lsp/consola');
const fetchDeVerdad = globalThis.fetch;
globalThis.fetch = async (url, o = {}) => {
  cuenta.posts += 1;
  cuenta.bytes_post += (o.body || '').length;
  const r = await fetchDeVerdad(aServe(url), { ...o, headers: { ...(o.headers || {}), ...cabeceras } });
  cuenta.codigos[r.status] = (cuenta.codigos[r.status] || 0) + 1;
  return r;
};
class EventSource {
  static CLOSED = 2;
  constructor(url) { this.readyState = 0; this.oyentes = {}; this.ctl = new AbortController(); void this.leer(url); }
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
          else if (l.startsWith('data: ')) {
            cuenta.eventos += 1;
            cuenta.bytes_eventos += l.length;
            for (const f of this.oyentes[ev] || []) f({ data: l.slice(6) });
          } else if (l === '') ev = 'message';
        }
      }
      this.readyState = 2;
      this.onerror?.();
    } catch { this.readyState = 2; }
  }
}
globalThis.EventSource = EventSource;

// ── el Monaco de mentira ─────────────────────────────────────────────────────
const proveedores = { hover: {}, completion: {} };
const marcas = new Map(); // ruta → {t, xs}
const modelos = new Map(); // ruta → modelo
const monaco = {
  MarkerSeverity: { Error: 8, Warning: 4, Info: 2 },
  languages: {
    registerHoverProvider: (l, p) => { proveedores.hover[l] = p; return { dispose() {} }; },
    registerCompletionItemProvider: (l, p) => { proveedores.completion[l] = p; return { dispose() {} }; },
  },
  editor: {
    getModels: () => [...modelos.values()],
    setModelMarkers: (m, dueno, xs) => marcas.set(m.uri.path.slice(1), { t: ahora(), dueno, xs }),
  },
};
function modelo(ruta, texto) {
  let m = modelos.get(ruta);
  if (!m) {
    m = {
      texto,
      uri: { path: '/' + ruta, toString: () => 'inmemory://model/' + ruta },
      getValue() { return this.texto; },
      getWordUntilPosition: (p) => ({ startColumn: p.column, endColumn: p.column }),
    };
    modelos.set(ruta, m);
  }
  m.texto = texto;
  return m;
}

const { servidorDeLenguaje } = await import('file:///' + consola.replace(/\\/g, '/') + '/components/code-workspace/servidor-de-lenguaje.ts');
const cliente = (l) => servidorDeLenguaje(PUESTO, monaco, l);
const lc = (texto, cur) => { const a = texto.slice(0, cur); return { lineNumber: a.split('\n').length, column: cur - (a.lastIndexOf('\n') + 1) + 1 }; };

async function completion(l, ruta, texto, cur) {
  const m = modelo(ruta, texto);
  const t0 = ahora();
  const r = await proveedores.completion[l].provideCompletionItems(m, lc(texto, cur ?? texto.length));
  return { ms: ahora() - t0, n: r.suggestions.length, labels: r.suggestions.map((s) => s.label).slice(0, 60), bytes: JSON.stringify(r.suggestions).length };
}

// ArbolFileView: el texto al servidor 200 ms despues de la ultima tecla; Monaco
// pide completion al teclear una palabra o un punto.
async function teclear(o) {
  const c = cliente(o.lenguaje);
  let reloj = null;
  const lat = [];
  const vacias = [];
  for (let k = 1; k <= o.objetivo.length; k++) {
    const texto = o.objetivo.slice(0, k);
    modelo(o.ruta, texto);
    if (reloj) clearTimeout(reloj);
    reloj = setTimeout(() => c.cambiar(o.ruta, texto), o.debounce ?? 200);
    if (/[\w.]/.test(o.objetivo[k - 1]) && o.completion !== false) {
      const r = await completion(o.lenguaje, o.ruta, texto, k);
      lat.push(r.ms);
      if (r.n === 0) vacias.push(k);
    }
    await dormir(o.ms_tecla ?? 80);
  }
  return { lat, vacias: vacias.length };
}

const ordenes = {
  async abrir(o) { const c = cliente(o.lenguaje); modelo(o.ruta, o.texto); const t0 = ahora(); await c.abrir(o.ruta, o.texto); return { ms: ahora() - t0 }; },
  async cerrar(o) { cliente(o.lenguaje).cerrar(o.ruta); return {}; },
  async completion(o) { return completion(o.lenguaje, o.ruta, o.texto, o.cur); },
  async hover(o) {
    const m = modelo(o.ruta, o.texto);
    const t0 = ahora();
    const h = await proveedores.hover[o.lenguaje].provideHover(m, lc(o.texto, o.cur));
    return { ms: ahora() - t0, valor: h ? h.contents[0].value : null };
  },
  async cambiar(o) { modelo(o.ruta, o.texto); cliente(o.lenguaje).cambiar(o.ruta, o.texto); return {}; },
  async marcas(o) {
    const t0 = ahora();
    const antes = marcas.get(o.ruta)?.t ?? -1;
    while (ahora() - t0 < (o.esperar_ms ?? 5000)) {
      const x = marcas.get(o.ruta);
      if (x && x.t > antes && (!o.no_vacias || x.xs.length)) return { ms: ahora() - t0, n: x.xs.length, xs: x.xs.map((y) => [y.startLineNumber, y.startColumn, y.message.slice(0, 90)]) };
      await dormir(10);
    }
    const x = marcas.get(o.ruta);
    return { ms: null, n: x ? x.xs.length : null, xs: x ? x.xs.map((y) => [y.startLineNumber, y.startColumn, y.message.slice(0, 90)]) : [] };
  },
  async teclear(o) { return teclear(o); },
  async a_la_vez(o) { const t0 = ahora(); const rs = await Promise.all(o.tareas.map((t) => teclear(t))); return { ms: ahora() - t0, rs }; },
  async juntas(o) {
    const t0 = ahora();
    const rs = await Promise.all(o.ordenes.map(async (x) => { const t = ahora(); const r = await ordenes[x.orden](x); return { ...r, fin_ms: ahora() - t }; }));
    return { ms: ahora() - t0, rs };
  },
  async cuenta() { return { ...cuenta }; },
  async apagar(o) { cliente(o.lenguaje).apagar(); return {}; },
};

const rl = readline.createInterface({ input: process.stdin });
for await (const linea of rl) {
  if (!linea.trim()) continue;
  const o = JSON.parse(linea);
  if (o.orden === 'salir') break;
  try {
    const r = await ordenes[o.orden](o);
    process.stdout.write('R ' + JSON.stringify({ ok: true, ...r }) + '\n');
  } catch (e) {
    process.stdout.write('R ' + JSON.stringify({ ok: false, error: String(e && e.stack || e).slice(0, 500) }) + '\n');
  }
}
process.exit(0);
