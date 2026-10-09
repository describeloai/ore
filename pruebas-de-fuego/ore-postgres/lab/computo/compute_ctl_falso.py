"""El compute_ctl de mentira del laboratorio de P6 (ADR 0058, P6·0 y P6·1). DESECHABLE.

Cumple lo que se leyó del compute_ctl real (commit 8269bece), para que ore-postgres le hable igual
que a una VM de NeonVM:

  · arranca con CONFIG_JSON (la forma de config.json: {spec, compute_ctl_config}); con "spec": null
    queda en `empty` esperando un /configure (el pool, P6·5);
  · GET  /status     → {status, last_active, start_time, tenant, timeline, error}; status en snake_case;
  · POST /configure  → sólo en `empty` o `running` (si no, 412); contesta cuando queda `running`;
  · POST /terminate  → para Postgres limpio y devuelve {"lsn": …} (dormir, P6·3);
  · last_active como monitor.rs: un backend de cliente que no está idle (sin cloud_admin) es AHORA;
    si no, la hora en que quedó idle el último; un walsender o un autovacuum también es actividad;
  · el token: Bearer con `compute_id` igual al suyo (el real verifica además la firma con el JWKS).

Lo que NO es: el almacenamiento de Neon. Los datos viven en /almacen/<tenant>/<timeline> (un volumen
compartido que hace de pageserver), así que dormir y despertar los conserva.

ARRANQUE_S emula lo que tarda una VM en arrancar antes de que compute_ctl atienda una especificación.
"""
import base64, json, os, subprocess, threading, time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

COMPUTE_ID = os.environ.get("COMPUTE_ID", "")
ARRANQUE_S = float(os.environ.get("ARRANQUE_S", "0"))
INI_BASE = "/etc/pgbouncer.ini"
INI = "/tmp/pgbouncer.ini"
lock = threading.Condition()
st = {"status": "empty", "last_active": None, "start_time": None, "tenant": None, "timeline": None,
      "error": None, "pgdata": None, "max_connections": None, "pgbouncer": None}


def ahora():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def como_postgres(*cmd, check=True, entrada=None):
    return subprocess.run(["gosu", "postgres", *cmd], check=check, capture_output=True, text=True, input=entrada)


def psql(sql, base="postgres", check=True):
    r = como_postgres("psql", "-h", "/tmp", "-p", "5432", "-U", "cloud_admin", "-d", base, "-v", "ON_ERROR_STOP=1",
                      "-Atqc", sql, check=False)
    if check and r.returncode != 0:
        raise RuntimeError(r.stderr.strip())
    return r.stdout.strip()


def ajuste(spec, nombre, defecto=None):
    for a in spec.get("cluster", {}).get("settings", []):
        if a.get("name") == nombre:
            return a.get("value")
    return defecto


def cita(n):
    return '"' + n.replace('"', '""') + '"'


def literal(v):
    return "'" + v.replace("'", "''") + "'"


def arrancar_postgres(spec):
    tenant, timeline = ajuste(spec, "neon.tenant_id", "sin-tenant"), ajuste(spec, "neon.timeline_id", "sin-timeline")
    pgdata = f"/almacen/{tenant}/{timeline}"
    maximas = ajuste(spec, "max_connections", "100")
    if st["pgdata"] == pgdata and st["max_connections"] == maximas:
        return
    if st["pgdata"]:
        como_postgres("pg_ctl", "-D", st["pgdata"], "stop", "-m", "fast", "-w", check=False)
    os.makedirs(pgdata, exist_ok=True)
    subprocess.run(["chown", "-R", "postgres:postgres", f"/almacen/{tenant}"], check=True)
    if not os.path.exists(f"{pgdata}/PG_VERSION"):
        como_postgres("initdb", "-D", pgdata, "-U", "cloud_admin", "--auth-local=trust", "--auth-host=scram-sha-256")
        with open(f"{pgdata}/pg_hba.conf", "w") as f:
            f.write("local all all trust\nhost all cloud_admin 127.0.0.1/32 trust\nhost all cloud_admin ::1/128 trust\n"
                    "host all all all scram-sha-256\n")
    opciones = f"-c port=5432 -c listen_addresses=* -c unix_socket_directories=/tmp -c max_connections={maximas} " \
               "-c password_encryption=scram-sha-256"
    como_postgres("pg_ctl", "-D", pgdata, "-o", opciones, "-l", "/tmp/postgres.log", "start", "-w")
    st.update(pgdata=pgdata, max_connections=maximas, tenant=tenant, timeline=timeline)


def aplicar_datos(spec):
    c = spec.get("cluster", {})
    for r in c.get("roles", []):
        n, v = r["name"], r.get("encrypted_password")
        if n == "cloud_admin":
            continue
        psql(f"do $$ begin if exists (select from pg_roles where rolname = {literal(n)}) then "
             f"alter role {cita(n)} login password {literal(v)}; else create role {cita(n)} login password {literal(v)}; "
             f"end if; end $$;")
    existentes = set(psql("select datname from pg_database").split("\n"))
    for b in c.get("databases", []):
        if b["name"] not in existentes:
            psql(f"create database {cita(b['name'])} owner {cita(b['owner'])}")
    for d in spec.get("delta_operations", []) or []:
        if d.get("action") == "delete_db":
            psql(f"drop database if exists {cita(d['name'])} with (force)")
        elif d.get("action") == "delete_role":
            psql(f"drop role if exists {cita(d['name'])}")


def aplicar_pgbouncer(spec):
    ajustes = dict(spec.get("pgbouncer_settings") or {})
    lineas = [l for l in open(INI_BASE).read().splitlines() if l.split("=", 1)[0].strip() not in ajustes]
    salida = []
    for l in lineas:
        salida.append(l)
        if l.strip() == "[pgbouncer]":
            salida += [f"{k}={v}" for k, v in ajustes.items()]
    with open(INI, "w") as f:
        f.write("\n".join(salida) + "\n")
    os.chmod(INI, 0o644)
    p = st["pgbouncer"]
    if p and p.poll() is None:
        p.send_signal(1)  # SIGHUP: relee el ini, como el RELOAD de tune_pgbouncer
    else:
        st["pgbouncer"] = subprocess.Popen(["gosu", "postgres", "pgbouncer", INI])


def aplicar(spec, primera):
    try:
        if primera and ARRANQUE_S:
            time.sleep(ARRANQUE_S)
        arrancar_postgres(spec)
        aplicar_datos(spec)
        aplicar_pgbouncer(spec)
        with lock:
            st.update(status="running", error=None)
            lock.notify_all()
    except Exception as e:  # noqa: BLE001
        with lock:
            st.update(status="failed", error=str(e)[:500])
            lock.notify_all()


def vigilar():
    """last_active, con las consultas de compute_tools/src/monitor.rs."""
    while True:
        time.sleep(0.5)
        if st["status"] not in ("running", "configuration"):
            continue
        try:
            filas = psql("select state, to_char(state_change at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') "
                         "from pg_stat_activity where backend_type = 'client backend' and pid != pg_backend_pid() "
                         "and usename != 'cloud_admin'", check=False)
            ultimo = None
            for f in filter(None, filas.split("\n")):
                estado, cambio = f.split("|", 1)
                if estado != "idle":
                    ultimo = ahora()
                    break
                ultimo = max(ultimo or cambio, cambio)
            otros = psql("select (select count(*) from pg_stat_replication where application_name != 'walproposer') "
                         "+ (select count(*) from pg_stat_activity where backend_type = 'autovacuum worker')", check=False)
            if otros not in ("", "0"):
                ultimo = ahora()
            if ultimo and (st["last_active"] is None or ultimo > st["last_active"]):
                st["last_active"] = ultimo
        except Exception:  # noqa: BLE001
            pass


class H(BaseHTTPRequestHandler):
    def contestar(self, codigo, cuerpo):
        b = json.dumps(cuerpo).encode()
        self.send_response(codigo)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def autorizado(self):
        t = self.headers.get("Authorization", "")
        try:
            carga = t.removeprefix("Bearer ").split(".")[1]
            carga += "=" * (-len(carga) % 4)
            return json.loads(base64.urlsafe_b64decode(carga)).get("compute_id") == COMPUTE_ID
        except Exception:  # noqa: BLE001
            return False

    def do_GET(self):
        if not self.autorizado():
            return self.contestar(401, {"error": "token"})
        if self.path == "/status":
            return self.contestar(200, {k: st[k] for k in ("status", "last_active", "start_time", "tenant", "timeline", "error")})
        self.contestar(404, {"error": "no"})

    def do_POST(self):
        if not self.autorizado():
            return self.contestar(401, {"error": "token"})
        largo = int(self.headers.get("Content-Length", "0"))
        cuerpo = json.loads(self.rfile.read(largo) or b"{}")
        if self.path == "/configure":
            spec = cuerpo.get("spec")
            if spec is None or "compute_ctl_config" not in cuerpo:
                return self.contestar(422, {"error": "missing field `spec` or `compute_ctl_config`"})
            with lock:
                if st["status"] not in ("empty", "running"):
                    return self.contestar(412, {"error": f"invalid compute status: {st['status']}"})
                primera = st["status"] == "empty"
                st["status"] = "configuration_pending" if primera else "configuration"
            threading.Thread(target=aplicar, args=(spec, primera), daemon=True).start()
            with lock:
                lock.wait_for(lambda: st["status"] in ("running", "failed"))
            if st["status"] == "failed":
                return self.contestar(500, {"error": st["error"]})
            return self.contestar(200, {"status": "running"})
        if self.path == "/terminate":
            lsn = None
            if st["pgdata"]:
                lsn = psql("select pg_current_wal_lsn()", check=False) or None
                como_postgres("pg_ctl", "-D", st["pgdata"], "stop", "-m", "fast", "-w", check=False)
            if st["pgbouncer"]:
                st["pgbouncer"].terminate()
            with lock:
                st["status"] = "terminated"
            return self.contestar(200, {"lsn": lsn})
        self.contestar(404, {"error": "no"})

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    st["start_time"] = ahora()
    config = json.loads(os.environ.get("CONFIG_JSON") or '{"spec": null}')
    threading.Thread(target=vigilar, daemon=True).start()
    if config.get("spec") is not None:
        st["status"] = "init"
        threading.Thread(target=aplicar, args=(config["spec"], True), daemon=True).start()
    ThreadingHTTPServer(("0.0.0.0", 3080), H).serve_forever()
