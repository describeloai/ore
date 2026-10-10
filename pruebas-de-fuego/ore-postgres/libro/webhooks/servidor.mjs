// libro-webhooks (ADR 0058, P7): los avisos de pago del banco. Una función de borde: cada aviso,
// una consulta por HTTP con `@neondatabase/serverless` (POST /sql al proxy), sin conexión que dure.
//
//   POST /avisos  {clave, cuenta, importe}  → {nuevo: true|false}
//
// El banco reintenta hasta que le contestan 2xx, y a veces manda el mismo aviso dos veces: la
// clave lo hace idempotente. Apuntar el aviso, su movimiento y el saldo es UNA sentencia (CTE),
// así que por HTTP es atómico sin transacción interactiva.
import http from 'node:http';
import { neon } from '@neondatabase/serverless';

const sql = neon(process.env.DATABASE_URL);
const PUERTO = Number(process.env.PUERTO ?? 8081);

const APUNTAR = `
  with nuevo as (
    insert into aviso (clave, cuenta, importe) values ($1, $2, $3)
    on conflict (clave) do nothing
    returning clave, cuenta, importe
  ), mov as (
    insert into movimiento (cuenta, importe, aviso) select cuenta, importe, clave from nuevo
  )
  update cuenta c set saldo = c.saldo + n.importe from nuevo n where c.id = n.cuenta
  returning c.id`;

const json = (res, codigo, cuerpo) => {
  const b = JSON.stringify(cuerpo);
  res.writeHead(codigo, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(b) });
  res.end(b);
};

http.createServer((req, res) => {
  if (req.method === 'GET' && req.url === '/salud') return json(res, 200, { ok: true });
  if (req.method !== 'POST' || req.url !== '/avisos') return json(res, 404, { error: 'no' });
  let b = '';
  req.on('data', (c) => (b += c));
  req.on('end', async () => {
    try {
      const { clave, cuenta, importe } = JSON.parse(b || '{}');
      if (!clave || !cuenta || !(Number(importe) > 0)) return json(res, 400, { error: 'clave, cuenta e importe > 0' });
      const filas = await sql.query(APUNTAR, [clave, cuenta, importe]);
      json(res, 200, { nuevo: filas.length > 0 });
    } catch (e) {
      json(res, 503, { error: String(e?.message ?? e).split('\n')[0].slice(0, 300), codigo: e?.code ?? null });
    }
  });
}).listen(PUERTO, () => console.log(`libro-webhooks · :${PUERTO}`));
