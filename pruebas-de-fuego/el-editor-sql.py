"""EL EDITOR DE SQL · de punta a punta: el cliente de la consola, ore-serve, el
agente y su servidor de SQL, todos de verdad.

  el cliente    `components/code-workspace/servidor-de-lenguaje.ts` de la
                consola, en Node (`el-editor-sql.mjs`), con un Monaco de mentira
  ore-serve     el banco de `medida-el-catalogo-como-resolutor.py` (un arbol con
                hr.ventas, hr.clientes, ventas.pedidos; un puesto abierto)
  el agente     `puesto/python/agente.py`, con su correa: lo de SQL a
                `ore.lsp_sql` en su proceso, lo demas a un pyright de mentira

Afirma: el cliente de SQL registra sus proveedores; tras FROM ofrece los
datasets; con el modelo cambiado y SIN esperar al didChange, la completion ya
ve el texto nuevo (las columnas del alias); la columna mal escrita se pinta en
su sitio con el dueño del servidor de lenguaje; el hover del dataset; abrir un
.sql NO arranca pyright (el log del agente); el cliente de Python es otro,
contesta el suyo, y ninguno pinta lo del otro.

    python pruebas-de-fuego/el-editor-sql.py [--consola C:/rubix-platform]

Necesita Node >= 22.6 (--experimental-strip-types), target/release y lo del
banco. No toca la consola que corre (`next dev`) ni el cluster.
"""
import importlib.util
import json
import os
import subprocess
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
RAIZ = os.path.dirname(AQUI)
CONSOLA = sys.argv[sys.argv.index("--consola") + 1] if "--consola" in sys.argv else "C:/rubix-platform"
sys.argv[:] = [sys.argv[0], "--filas", "200"]
_sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
R = importlib.util.module_from_spec(_sp)
_sp.loader.exec_module(R)

PYRIGHT_DE_MENTIRA = r'''
import json, sys
f = sys.stdin.buffer
while True:
    largo = None
    while True:
        l = f.readline()
        if not l:
            sys.exit(0)
        l = l.strip()
        if not l:
            break
        if l.lower().startswith(b"content-length:"):
            largo = int(l.split(b":")[1])
    m = json.loads(f.read(largo))
    out = []
    if "id" in m and "method" in m:
        out.append({"jsonrpc": "2.0", "id": m["id"], "result": {"items": [{"label": "desde_pyright"}]} if m["method"] == "textDocument/completion" else {"capabilities": {}}})
    if m.get("method") == "textDocument/didOpen":
        out.append({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": m["params"]["textDocument"]["uri"], "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "severity": 2, "message": "aviso de pyright"}]}})
    for o in out:
        b = json.dumps(o).encode()
        sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
        sys.stdout.buffer.flush()
'''

bien = True


def afirma(cond, que, detalle=""):
    global bien
    print("  %s %s%s" % ("·" if cond else "✗", que, ("" if cond else "  →  %s" % detalle)))
    bien = bien and cond


def main():
    b = R.Banco()
    agente = log = None
    try:
        b.levantar()
        R.escribir(b.tmp + "/pyright-de-mentira.py", PYRIGHT_DE_MENTIRA)
        log = open(b.tmp + "/agente.log", "w", encoding="utf-8")
        env = dict(os.environ, ORE_SERVE=b.directo, PUESTO=R.PUESTO, ORE_SUJETO="agente:local",
                   ORE_ALMACEN="dir:" + b.tmp, TTL="120", ORE_MEMORIA_MB="1024",
                   ORE_LSP="%s %s" % (os.path.basename(sys.executable), (b.tmp + "/pyright-de-mentira.py").replace("\\", "/")),
                   TRABAJO_DIR=b.tmp, PYTHONUNBUFFERED="1", PYTHONIOENCODING="utf-8")
        agente = subprocess.Popen([sys.executable, RAIZ + "/puesto/python/agente.py"], env=env, stdout=log, stderr=subprocess.STDOUT)
        b.procesos.append(agente)
        time.sleep(2)
        p = subprocess.run(["node", "--experimental-strip-types", "--no-warnings", AQUI + "/el-editor-sql.mjs",
                            CONSOLA, b.directo, R.PUESTO, "persona:ana"], capture_output=True, text=True,
                           encoding="utf-8", timeout=120)
        salida = p.stdout
        res = next((json.loads(l[10:]) for l in salida.splitlines() if l.startswith("RESULTADO ")), None)
        if res is None:
            print(salida[-3000:], p.stderr[-3000:])
            raise SystemExit("el cliente no terminó")
        time.sleep(0.5)
        registro = open(b.tmp + "/agente.log", encoding="utf-8", errors="replace").read()
        i_sql = registro.find("servidor de SQL en marcha")
        i_py = registro.find("servidor de lenguaje arrancado")
        print("EL EDITOR DE SQL · el cliente de la consola + ore-serve + el agente, de verdad")
        afirma(res["registrado_sql"], "el cliente de SQL registra completion y hover para `sql`")
        afirma("hr.ventas" in res["tras_from"] and "ventas.pedidos" in res["tras_from"], "tras FROM, los datasets del árbol", res["tras_from"])
        afirma({"id", "pais", "total", "cuando"} <= set(res["alias_sin_esperar"]),
               "con el modelo cambiado y SIN esperar al didChange, la completion ve el texto nuevo (las columnas de `v`)", res["alias_sin_esperar"])
        m = res["marca"] or {}
        afirma(m.get("dueno") == "servidor-de-lenguaje" and m.get("n") == 1 and (m.get("linea"), m.get("col")) == (1, 8) and m.get("sev") == 8 and "totl" in m.get("msg", ""),
               "la columna mal escrita, pintada en su sitio (L1:C8, error)", m)
        afirma(bool(res["hover"]) and "hr.ventas" in res["hover"] and "Dataset" in res["hover"], "el hover del dataset", res["hover"])
        afirma(i_sql >= 0 and (i_py < 0 or i_py > i_sql) and "servidor de lenguaje arrancado" not in registro[: registro.find("servidor de SQL en marcha") + 1],
               "abrir un .sql NO arranca pyright (el agente lo arranca después, con el .py)", registro[-800:])
        afirma(i_py > i_sql, "el .py sí lo arranca", registro[-800:])
        afirma(res["dos_clientes"], "un cliente por lenguaje en el mismo puesto")
        afirma(res["python"] == ["desde_pyright"], "la completion de Python la contesta el suyo", res["python"])
        afirma(res["marcas_py"] == ["aviso de pyright"], "el cliente de Python pinta lo de pyright y nada del de SQL", res["marcas_py"])
        afirma(res["marcas_sql_final"] == 1, "y el de SQL sigue con su marca (ninguno pinta lo del otro)", res["marcas_sql_final"])
        print("rc=%d" % (0 if bien else 1))
    finally:
        if agente:
            agente.terminate()
            agente.wait(10)
        try:
            log.close()
        except Exception:
            pass
        b.cerrar()
    sys.exit(0 if bien else 1)


if __name__ == "__main__":
    main()
