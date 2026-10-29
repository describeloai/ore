#!/usr/bin/env python3
"""LA CONMUTACIÓN DEL CATÁLOGO, MEDIDA (ADR 0079 §18 y §19.5, E1a·5f·4).

Un cliente que escribe sin parar por el Service `catalogo-db`, como lo haría ore-serve: una conexión
nueva por escritura (lo que hace su pool cuando una conexión se rompe) y una fila por escritura en
`conmutacion.pulso`. Mientras, alguien mata la primaria. Dice:

  - cuánto tiempo no se pudo escribir (del primer fallo al primer acierto después);
  - cuántas escrituras fallaron (las que un cliente reintentaría, como un 503 de ore-serve);
  - **si alguna escritura que el cliente vio confirmada falta después** (lo que no puede pasar).

  python3 conmutar.py <segundos>     (en un pod del namespace `catalogo`, con PGUSER/PGPASSWORD)
"""
import os, subprocess, sys, time

HOST = "catalogo-db.catalogo.svc"
DURA = float(sys.argv[1]) if len(sys.argv) > 1 else 120


def psql(sql, timeout=5):
    s = subprocess.run(["psql", "-X", "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", "-h", HOST,
                        "-d", "catalogo", "-c", sql], capture_output=True, text=True, timeout=timeout,
                       env={**os.environ, "PGCONNECT_TIMEOUT": "3"})
    if s.returncode != 0:
        raise RuntimeError(s.stderr.strip().splitlines()[-1] if s.stderr.strip() else "fallo")
    return s.stdout.strip()


def main():
    psql("CREATE SCHEMA IF NOT EXISTS conmutacion; DROP TABLE IF EXISTS conmutacion.pulso; "
         "CREATE TABLE conmutacion.pulso (n int PRIMARY KEY, cuando timestamptz NOT NULL DEFAULT now(), "
         "servidor inet DEFAULT inet_server_addr())")
    t0 = time.time()
    confirmadas, fallos, n = [], [], 0
    servidores = []
    while time.time() - t0 < DURA:
        n += 1
        t = time.time() - t0
        try:
            r = psql(f"INSERT INTO conmutacion.pulso (n) VALUES ({n}) RETURNING servidor")
            confirmadas.append((n, t))
            if not servidores or servidores[-1][1] != r:
                servidores.append((t, r))
                print(f"  {t:6.1f} s · escribe en {r}", flush=True)
        except Exception as e:
            fallos.append((n, t, str(e)[:120]))
            if len(fallos) == 1 or fallos[-2][1] < t - 2:
                print(f"  {t:6.1f} s · ✗ {str(e)[:120]}", flush=True)
        time.sleep(0.1)
    # El hueco: del primer fallo al primer acierto después.
    hueco = None
    if fallos:
        f0 = fallos[0][1]
        despues = [t for _, t in confirmadas if t > f0]
        hueco = (despues[0] - f0) if despues else None
    # Lo confirmado, ¿está?
    for _ in range(30):
        try:
            hay = set(int(x) for x in psql("SELECT n FROM conmutacion.pulso", timeout=15).split())
            break
        except Exception:
            time.sleep(2)
    perdidas = [c for c, _ in confirmadas if c not in hay]
    print(f"escrituras: {n} · confirmadas: {len(confirmadas)} · fallidas: {len(fallos)} · "
          f"sin escribir: {hueco:.1f} s" if hueco is not None else
          f"escrituras: {n} · confirmadas: {len(confirmadas)} · fallidas: {len(fallos)}")
    print(f"servidores: {' → '.join(s for _, s in servidores)}")
    print(f"confirmadas que faltan: {len(perdidas)} {perdidas[:10]}")
    psql("DROP SCHEMA conmutacion CASCADE")
    return 0 if not perdidas else 1


if __name__ == "__main__":
    sys.exit(main())
