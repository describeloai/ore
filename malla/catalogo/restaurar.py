#!/usr/bin/env python3
"""LA RESTAURACIÓN PROBADA DEL CATÁLOGO, CADA SEMANA (ADR 0079 §18 y §19.5, E1a·5f·3).

«Una copia que nadie ha restaurado nunca no es una copia: es un fichero.» Esto restaura la copia
de verdad —la copia base y el WAL que CloudNativePG archiva en GCS— en un `Cluster` desechable y
comprueba que trae lo que el vivo tenía:

  1. un LATIDO: una fila nueva en el vivo (`restauracion.latido`), su WAL cerrado y archivado;
  2. `catalogo-restaurado`: un Cluster de una instancia que nace de la copia (hasta el final del
     WAL archivado), con un disco que se borra con él;
  3. el latido está, y cada esquema de inquilino (`t_*`) tiene la firma del vivo (la de 77: estado,
     historia, commits y ramas);
  4. el Cluster desechable, borrado. Sale 0 si todo cuadra.

Sin `kubectl`: habla con la API con la cuenta del Job (`restaurador`, sólo `clusters` y su Secret).
Lo lanza el CronJob `restaurar-el-catalogo` (77-restaurar-el-catalogo-cada-semana.yaml); a mano:
`kubectl -n catalogo create job --from=cronjob/restaurar-el-catalogo restaurar-ahora`.
"""
import base64, json, os, ssl, subprocess, sys, time, urllib.request

NS = "catalogo"
VIVO = "catalogo"
DESECHABLE = "catalogo-restaurado"
API = "https://kubernetes.default.svc"
SA = "/var/run/secrets/kubernetes.io/serviceaccount"
TOKEN = open(f"{SA}/token").read().strip()
CTX = ssl.create_default_context(cafile=f"{SA}/ca.crt")
ESPERA_ARCHIVO_S = 300
ESPERA_CLUSTER_S = 1500


def api(metodo, ruta, cuerpo=None):
    r = urllib.request.Request(API + ruta, method=metodo,
                               data=json.dumps(cuerpo).encode() if cuerpo is not None else None)
    r.add_header("Authorization", f"Bearer {TOKEN}")
    r.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(r, context=CTX, timeout=30) as f:
            return f.status, json.loads(f.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def psql(host, usuario, clave, sql, base="catalogo"):
    s = subprocess.run(["psql", "-X", "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", "-h", host,
                        "-U", usuario, "-d", base, "-c", sql],
                       capture_output=True, text=True, env={**os.environ, "PGPASSWORD": clave})
    if s.returncode != 0:
        raise RuntimeError(f"psql en {host}: {s.stderr.strip()}")
    return s.stdout.strip()


FIRMA = """SELECT md5(coalesce(string_agg(ref||'|'||activo||'|'||coalesce(puntero::text,'-')||'|'||borrado,
           E'\\n' ORDER BY ref, activo), '')) || ' · ' || (SELECT count(*) FROM {s}.historia) || ' historia · '
         || (SELECT count(*) FROM {s}.commits) || ' commits · '
         || coalesce((SELECT string_agg(nombre||'@'||coalesce(punto_salida::text,'-'), ',' ORDER BY nombre)
                      FROM {s}.refs), '') FROM {s}.estado"""


def firmas(host, u, c):
    # Los de un inquilino con sus tablas: uno recién dado de alta (78) las crea al conectar ore-serve.
    esquemas = psql(host, u, c, "SELECT nspname FROM pg_namespace WHERE nspname LIKE 't\\_%' "
                    "AND to_regclass(quote_ident(nspname) || '.estado') IS NOT NULL ORDER BY 1")
    return {s: psql(host, u, c, FIRMA.format(s=s)) for s in esquemas.split() if s}


def secreto(nombre):
    c, j = api("GET", f"/api/v1/namespaces/{NS}/secrets/{nombre}")
    if c != 200:
        raise RuntimeError(f"el Secret {nombre}: {c}")
    d = j["data"]
    return base64.b64decode(d["username"]).decode(), base64.b64decode(d["password"]).decode()


def desechable(vivo):
    """El Cluster que nace de la copia del vivo: lo mismo que él (imagen, pool) y nada que archivar."""
    s = vivo["spec"]
    return {
        "apiVersion": "postgresql.cnpg.io/v1", "kind": "Cluster",
        "metadata": {"name": DESECHABLE, "namespace": NS,
                     "labels": {"ore.dev/tenant": "system", "ore.dev/rol": "restauracion"}},
        "spec": {
            "instances": 1,
            "imageName": s["imageName"],
            "enableSuperuserAccess": True,
            "bootstrap": {"recovery": {"source": VIVO}},
            "externalClusters": [{"name": VIVO, "barmanObjectStore": {
                **{k: v for k, v in s["backup"]["barmanObjectStore"].items() if k in ("destinationPath", "googleCredentials")},
                "serverName": VIVO}}],
            # El disco se borra con el Cluster (la clase por defecto, no `retiene`).
            "storage": {"size": s["storage"]["size"]},
            "affinity": {k: v for k, v in s.get("affinity", {}).items() if k in ("nodeSelector", "tolerations")},
            "resources": {"requests": {"cpu": "100m", "memory": "256Mi"}, "limits": {"memory": "1Gi"}},
            "serviceAccountTemplate": s.get("serviceAccountTemplate", {}),
        },
    }


def borrar():
    c, _ = api("DELETE", f"/apis/postgresql.cnpg.io/v1/namespaces/{NS}/clusters/{DESECHABLE}")
    return c in (200, 202, 404)


def main():
    t0 = time.time()
    u, c = secreto("catalogo-superuser")
    vivo_host = f"{VIVO}-rw.{NS}.svc"
    # Un desechable que quedara de otra pasada, fuera.
    borrar()
    for _ in range(60):
        if api("GET", f"/apis/postgresql.cnpg.io/v1/namespaces/{NS}/clusters/{DESECHABLE}")[0] == 404:
            break
        time.sleep(5)

    # ── 1 · el latido, y su WAL en GCS ──────────────────────────────────────────────────────
    psql(vivo_host, u, c, "CREATE SCHEMA IF NOT EXISTS restauracion; CREATE TABLE IF NOT EXISTS "
         "restauracion.latido (n bigserial PRIMARY KEY, cuando timestamptz NOT NULL DEFAULT now())")
    n = psql(vivo_host, u, c, "INSERT INTO restauracion.latido DEFAULT VALUES RETURNING n")
    wal = psql(vivo_host, u, c, "SELECT pg_walfile_name(pg_switch_wal())")
    for _ in range(ESPERA_ARCHIVO_S // 5):
        hecho = psql(vivo_host, u, c, "SELECT coalesce(last_archived_wal, '') FROM pg_stat_archiver")
        if hecho[:24] >= wal:
            break
        time.sleep(5)
    else:
        print(f"✗ el WAL {wal} del latido no llegó a GCS en {ESPERA_ARCHIVO_S} s"); return 1
    vivas = firmas(vivo_host, u, c)
    print(f"· latido {n} en el WAL {wal}, archivado · {len(vivas)} esquema(s) de inquilino", flush=True)

    # ── 2 · el Cluster desechable, desde la copia ───────────────────────────────────────────
    cv, vivo = api("GET", f"/apis/postgresql.cnpg.io/v1/namespaces/{NS}/clusters/{VIVO}")
    if cv != 200:
        print(f"✗ el Cluster vivo: {cv}"); return 1
    cc, r = api("POST", f"/apis/postgresql.cnpg.io/v1/namespaces/{NS}/clusters", desechable(vivo))
    if cc not in (200, 201):
        print(f"✗ no se pudo crear {DESECHABLE}: {cc} {json.dumps(r)[:400]}"); return 1
    try:
        listo = False
        for _ in range(ESPERA_CLUSTER_S // 15):
            _, j = api("GET", f"/apis/postgresql.cnpg.io/v1/namespaces/{NS}/clusters/{DESECHABLE}")
            st = j.get("status", {})
            if st.get("readyInstances") == 1:
                listo = True
                break
            time.sleep(15)
        if not listo:
            print(f"✗ {DESECHABLE} no estuvo listo en {ESPERA_CLUSTER_S} s: {st.get('phase')}"); return 1
        print(f"· {DESECHABLE} restaurado en {time.time() - t0:.0f} s", flush=True)

        # ── 3 · lo que trae ────────────────────────────────────────────────────────────────
        ur, cr = secreto(f"{DESECHABLE}-superuser")
        host = f"{DESECHABLE}-rw.{NS}.svc"
        tiene = psql(host, ur, cr, f"SELECT count(*) FROM restauracion.latido WHERE n = {n}")
        mal = 0
        if tiene != "1":
            print(f"FALLA el latido {n} no está en la copia"); mal += 1
        else:
            print(f"PASA  el latido {n} está en la copia")
        restauradas = firmas(host, ur, cr)
        for s, f in vivas.items():
            g = restauradas.get(s)
            if g == f:
                print(f"PASA  {s}  ({f[:60]})")
            else:
                print(f"FALLA {s}  vivo {f[:60]} · restaurado {str(g)[:60]}"); mal += 1
        print(f"esquemas: {len(vivas)} · distintos o ausentes: {mal} · {time.time() - t0:.0f} s en total")
        return 0 if mal == 0 else 1
    finally:
        print(f"· {DESECHABLE} borrado" if borrar() else f"⚠ no se pudo borrar {DESECHABLE}")


if __name__ == "__main__":
    sys.exit(main())
