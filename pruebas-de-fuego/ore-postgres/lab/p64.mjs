// P6·4 en el laboratorio: despertar por HTTP y por WebSocket con @neondatabase/serverless.
// argv: rol, clave, base, vm. Imprime «http N ms» y «ws N ms» (o el error) y sale con 1 si falla.
import { neon, Pool, neonConfig } from '@neondatabase/serverless';
import ws from 'ws';
neonConfig.webSocketConstructor = ws;
const [rol, clave, base, vm, via] = process.argv.slice(2);
const url = `postgresql://${rol}:${encodeURIComponent(clave)}@${vm}.europe-west1.pg.paladio.io/${base}`;
const t0 = performance.now();
try {
  let n;
  if (via === 'http') {
    [{ n }] = await neon(url)`select count(*)::int as n from t`;
  } else {
    const pool = new Pool({ connectionString: url });
    ({ rows: [{ n }] } = await pool.query('select count(*)::int as n from t'));
    await pool.end();
  }
  console.log(`${via} ${n} ${Math.round(performance.now() - t0)}`);
} catch (e) {
  console.log(`${via} error ${String(e?.message ?? e).split('\n')[0].slice(0, 120)}`);
  process.exit(1);
}
