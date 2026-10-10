// libro-api (ADR 0058, P7): el backend de Libro. Node y Prisma, por el pool (`-pooler`).
//
//   POST /cuentas                {id, titular}                → alta (idempotente)
//   GET  /cuentas/:id                                         → {id, titular, saldo}
//   GET  /cuentas/:id/movimientos?limite=N                    → los últimos N
//   POST /transferencias         {id, origen, destino, importe} → la transferencia; `id` es la
//        clave de idempotencia del cliente: reintentarla devuelve la misma, no transfiere dos veces
//   GET  /salud                                               → sin tocar la base (no la despierta)
//
// Una transferencia es una transacción `serializable` interactiva: el conflicto (P2034) se
// reintenta aquí, y el cliente nunca lo ve. Sin fondos, 409.
import http from 'node:http';
import { PrismaClient, Prisma } from '@prisma/client';

const prisma = new PrismaClient();
const PUERTO = Number(process.env.PUERTO ?? 8080);
const REINTENTOS = 20;

const json = (res, codigo, cuerpo) => {
  const b = JSON.stringify(cuerpo, (_, v) => (typeof v === 'bigint' ? Number(v) : v));
  res.writeHead(codigo, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(b) });
  res.end(b);
};
const leer = (req) => new Promise((ok, mal) => {
  let b = '';
  req.on('data', (c) => (b += c));
  req.on('end', () => { try { ok(b ? JSON.parse(b) : {}); } catch (e) { mal(e); } });
});
const espera = (ms) => new Promise((r) => setTimeout(r, ms));

class SinFondos extends Error {}

async function transferir({ id, origen, destino, importe }) {
  for (let i = 0; ; i++) {
    try {
      return await prisma.$transaction(async (tx) => {
        const ya = await tx.transferencia.findUnique({ where: { id } });
        if (ya) return { ...ya, repetida: true };
        const t = await tx.transferencia.create({ data: { id, origen, destino, importe: BigInt(importe) } });
        const { count } = await tx.cuenta.updateMany({
          where: { id: origen, saldo: { gte: BigInt(importe) } },
          data: { saldo: { decrement: BigInt(importe) } },
        });
        if (count !== 1) throw new SinFondos();
        await tx.cuenta.update({ where: { id: destino }, data: { saldo: { increment: BigInt(importe) } } });
        await tx.movimiento.createMany({
          data: [
            { cuenta: origen, importe: -BigInt(importe), transferencia: id },
            { cuenta: destino, importe: BigInt(importe), transferencia: id },
          ],
        });
        return t;
      }, { isolationLevel: Prisma.TransactionIsolationLevel.Serializable, maxWait: 30000, timeout: 30000 });
    } catch (e) {
      // P2034: conflicto de escritura o de serialización; P2002: otra petición con la misma clave
      // ganó a la vez (la siguiente vuelta la encuentra hecha).
      const conflicto = e instanceof Prisma.PrismaClientKnownRequestError && (e.code === 'P2034' || e.code === 'P2002');
      if (conflicto && i < REINTENTOS) { await espera(5 + Math.random() * 20 * (i + 1)); continue; }
      throw e;
    }
  }
}

const servidor = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://x');
  const partes = url.pathname.split('/').filter(Boolean);
  try {
    if (req.method === 'GET' && url.pathname === '/salud') return json(res, 200, { ok: true });
    if (req.method === 'POST' && url.pathname === '/cuentas') {
      const { id, titular } = await leer(req);
      if (!id || !titular) return json(res, 400, { error: 'id y titular' });
      await prisma.cuenta.createMany({ data: [{ id, titular }], skipDuplicates: true });
      return json(res, 200, await prisma.cuenta.findUnique({ where: { id } }));
    }
    if (req.method === 'GET' && partes[0] === 'cuentas' && partes.length === 2) {
      const c = await prisma.cuenta.findUnique({ where: { id: partes[1] } });
      return c ? json(res, 200, c) : json(res, 404, { error: 'no existe' });
    }
    if (req.method === 'GET' && partes[0] === 'cuentas' && partes[2] === 'movimientos') {
      const limite = Math.min(Number(url.searchParams.get('limite') ?? 20), 200);
      return json(res, 200, await prisma.movimiento.findMany({
        where: { cuenta: partes[1] }, orderBy: { id: 'desc' }, take: limite,
      }));
    }
    if (req.method === 'POST' && url.pathname === '/transferencias') {
      const t = await leer(req);
      if (!t.id || !t.origen || !t.destino || !(Number(t.importe) > 0) || t.origen === t.destino) {
        return json(res, 400, { error: 'id, origen, destino distintos e importe > 0' });
      }
      return json(res, 200, await transferir(t));
    }
    json(res, 404, { error: 'no' });
  } catch (e) {
    if (e instanceof SinFondos) return json(res, 409, { error: 'sin fondos' });
    // El resto es nuestro (o de la base): 503, con el código de Prisma para clasificarlo.
    json(res, 503, { error: String(e?.message ?? e).split('\n').filter(Boolean).slice(-1)[0]?.slice(0, 300), codigo: e?.code ?? e?.errorCode ?? null });
  }
});
servidor.keepAliveTimeout = 65000;
servidor.listen(PUERTO, () => console.log(`libro-api · :${PUERTO}`));
