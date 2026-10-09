// P5·4 en el laboratorio: @neondatabase/serverless por las dos puertas del proxy de Neon.
//   HTTP (neon(): una consulta, una petición POST /sql) y WebSocket (Pool: el protocolo de
//   Postgres dentro de wss://…/v2). Lee de argv: rol, clave, base, vm (de demo) y vm de victor.
// Imprime una línea por comprobación: «✓ …» o «✗ …»; sale con 1 si alguna falla.
import { neon, Pool, neonConfig } from '@neondatabase/serverless';
import ws from 'ws';
neonConfig.webSocketConstructor = ws;
const [rol, clave, base, vm, vmB] = process.argv.slice(2);
const DOMINIO = 'europe-west1.pg.paladio.io';
const url = (c, v) => `postgresql://${rol}:${encodeURIComponent(c)}@${v}.${DOMINIO}/${base}`;
let fallos = 0;
const bien = (m) => console.log(`  ✓ ${m}`);
const mal = (m) => { console.log(`  ✗ ${m}`); fallos++; };
const corto = (e) => String(e?.message ?? e).split('\n')[0].slice(0, 110);
async function niega(que, f) {
  try { await f(); mal(`${que}: entra`); } catch (e) { bien(`${que}, no: ${corto(e)}`); }
}

// ── HTTP
try {
  const sql = neon(url(clave, vm));
  const [f] = await sql`select current_user as u, 1 + ${41}::int as n`;
  f.u === rol && f.n === 42 ? bien(`HTTP: neon() consulta como ${f.u}, con parámetros (${f.n})`) : mal(`HTTP: ${JSON.stringify(f)}`);
  const t0 = performance.now();
  for (let i = 0; i < 10; i++) await sql`select 1`;
  bien(`HTTP: 10 consultas seguidas, ${((performance.now() - t0) / 10).toFixed(1)} ms cada una`);
  const r = await sql.transaction([sql`create table if not exists p54 (n int)`, sql`insert into p54 values (1), (2)`, sql`select count(*)::int as c from p54`]);
  r[2][0].c >= 2 ? bien('HTTP: una transacción de tres sentencias en una petición') : mal(`HTTP transacción: ${JSON.stringify(r)}`);
} catch (e) { mal(`HTTP: ${corto(e)}`); }
await niega('HTTP con otra contraseña', () => neon(url('mala', vm))`select 1`);
await niega('HTTP con la contraseña de demo en el endpoint de victor', () => neon(url(clave, vmB))`select 1`);

// ── WebSocket
try {
  const pool = new Pool({ connectionString: url(clave, vm) });
  const c = await pool.connect();
  await c.query('begin');
  await c.query('insert into p54 values (3)');
  const { rows: [d] } = await c.query('select count(*)::int as c, pg_backend_pid() as pid from p54');
  await c.query('rollback');
  const { rows: [e] } = await c.query('select count(*)::int as c, pg_backend_pid() as pid from p54');
  d.c === e.c + 1 && d.pid === e.pid ? bien(`WebSocket: Pool con sesión (pid ${d.pid}) y transacción (rollback deshace)`) : mal(`WebSocket: ${JSON.stringify([d, e])}`);
  c.release();
  const t0 = performance.now();
  for (let i = 0; i < 10; i++) await pool.query('select 1');
  bien(`WebSocket: 10 consultas por la conexión abierta, ${((performance.now() - t0) / 10).toFixed(1)} ms cada una`);
  await pool.query('drop table p54');
  await pool.end();
} catch (e) { mal(`WebSocket: ${corto(e)}`); }
await niega('WebSocket con otra contraseña', async () => { const p = new Pool({ connectionString: url('mala', vm) }); try { await p.query('select 1'); } finally { await p.end().catch(() => {}); } });

process.exit(fallos ? 1 : 0);
