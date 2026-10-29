#!/usr/bin/env python3
"""LA AUDITORÍA DEL CATÁLOGO CONTRA GCS, CADA DÍA (ADR 0079 §18 y §19.5, E1a·5f·5).

Tras el corte, un puntero vivo que no resuelve en GCS (un bug, una escritura a medias) sólo se
vería cuando alguien leyera esa tabla y fallara. Esto lo busca antes. Por cada esquema de inquilino
(`t_*`) con sus tablas:

  1. cada puntero vivo, en cualquier ref, apunta a un `metadata.json` que EXISTE en su bucket;
  2. la historia y el estado cuadran: la última fila de la historia de cada puntero es lo que dice
     el estado (lo que llegó sembrado, sin historia, se cuenta aparte);
  y del servidor:
  3. ningún slot de replicación lógica retiene más de [`TOPE_SLOT`] de WAL (el tope duro es 2 GB:
     un consumidor parado llenaría el disco, §18).

Sale 0 si todo cuadra: un Job fallido es la alarma. Lo lanza el CronJob `auditar-el-catalogo`
(78-auditar-el-catalogo.yaml). Con `--esquema t_x`, sólo ese.
"""
import collections, os, subprocess, sys

HOST = os.environ.get("PGHOST", "catalogo-db.catalogo.svc")
TOPE_SLOT = 1 << 30


def psql(sql):
    s = subprocess.run(["psql", "-X", "-q", "-t", "-A", "-F", "\t", "-v", "ON_ERROR_STOP=1", "-h", HOST,
                        "-d", "catalogo", "-c", sql], capture_output=True, text=True)
    if s.returncode != 0:
        raise RuntimeError(s.stderr.strip())
    return [l.split("\t") for l in s.stdout.splitlines() if l]


def existentes(buckets):
    """Todos los `metadata.json` de cada bucket, en UN listado por bucket: arrancar `gcloud` cuesta
    segundos, y uno por tabla tardaba más de diez minutos con victor (medido)."""
    hay, fallos = set(), []
    for b in sorted(buckets):
        s = subprocess.run(["gcloud", "storage", "ls", f"gs://{b}/**/metadata/*.metadata.json"],
                           capture_output=True, text=True)
        if s.returncode == 0:
            hay.update(l.strip() for l in s.stdout.splitlines() if l.strip())
        elif "matched no objects" not in s.stderr:
            fallos.append(f"gs://{b}: {s.stderr.strip().splitlines()[-1] if s.stderr.strip() else s.returncode}")
    return hay, fallos


def auditar(esquema):
    mal = 0
    vivos = psql(f"SELECT ref, activo, metadata_location FROM {esquema}.estado "
                 f"WHERE NOT borrado AND metadata_location IS NOT NULL ORDER BY 1, 2")
    gs = [(r, a, m) for r, a, m in vivos if m.startswith("gs://")]
    otros = len(vivos) - len(gs)
    hay, fallos = existentes({m.split("/")[2] for _, _, m in gs})
    for f in fallos:
        print(f"  ⚠ no se pudo listar {f}")
        mal += 1
    faltan = [(r, a, m) for r, a, m in gs if m not in hay]
    for r, a, m in faltan[:20]:
        print(f"  FALLA {esquema} · {r} · {a} → {m} no está en GCS")
    mal += len(faltan)
    incoherentes = psql(f"""
        SELECT e.ref, e.activo, coalesce(e.metadata_location, '∅'), coalesce(h.a, '∅')
          FROM {esquema}.estado e
          JOIN LATERAL (SELECT a FROM {esquema}.historia h
                         WHERE h.ref = e.ref AND h.activo = e.activo
                         ORDER BY h.commit_id DESC LIMIT 1) h ON true
         WHERE h.a IS DISTINCT FROM (CASE WHEN e.borrado THEN NULL ELSE e.metadata_location END)""")
    for r, a, m, h in incoherentes[:20]:
        print(f"  FALLA {esquema} · {r} · {a}: el estado dice {m} y la historia {h}")
    mal += len(incoherentes)
    sin_historia = psql(f"SELECT count(*) FROM {esquema}.estado e WHERE NOT EXISTS "
                        f"(SELECT 1 FROM {esquema}.historia h WHERE h.ref = e.ref AND h.activo = e.activo)")[0][0]
    refs = collections.Counter(r for r, _, _ in vivos)
    print(f"{'PASA ' if mal == 0 else 'FALLA'} {esquema} · {len(gs)} punteros vivos en GCS, {len(faltan)} no están · "
          f"{len(incoherentes)} con la historia descuadrada · {sin_historia} sembrados sin historia · "
          f"{otros} fuera de GCS · {len(refs)} ref(s)", flush=True)
    return mal


def main():
    solo = sys.argv[2] if len(sys.argv) > 2 and sys.argv[1] == "--esquema" else None
    esquemas = [e for (e,) in psql(
        "SELECT nspname FROM pg_namespace WHERE nspname LIKE 't\\_%' "
        "AND to_regclass(quote_ident(nspname) || '.estado') IS NOT NULL ORDER BY 1")]
    if solo:
        esquemas = [e for e in esquemas if e == solo]
    mal = sum(auditar(e) for e in esquemas)
    for nombre, retiene in psql("SELECT slot_name, coalesce(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn), 0)::bigint "
                                "FROM pg_replication_slots WHERE slot_type = 'logical'"):
        r = int(retiene)
        estado = "FALLA" if r > TOPE_SLOT else "PASA "
        mal += r > TOPE_SLOT
        print(f"{estado} slot {nombre} retiene {r / (1 << 20):.0f} MB de WAL")
    print(f"esquemas: {len(esquemas)} · fallos: {mal}")
    return 0 if mal == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
