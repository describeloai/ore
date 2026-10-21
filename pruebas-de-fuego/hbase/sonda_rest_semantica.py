"""X0 de HBase, ruta REST: la semantica que el origen necesita (rango de clave, rango de tiempo, versiones,
borrados, caducidad del scanner).  Usa la tabla `sem`, que se crea aqui con el shell.

    python sonda_rest_semantica.py
"""
import base64
import http.client
import json
import subprocess
import time

b64 = lambda s: base64.b64encode(s.encode() if isinstance(s, str) else s).decode()
unb = lambda s: base64.b64decode(s)
H = "127.0.0.1"


def rest(metodo, ruta, cuerpo=None, tipo="application/json", acepta="application/json"):
    c = http.client.HTTPConnection(H, 8080, timeout=60)
    c.request(metodo, ruta, body=cuerpo, headers={"Content-Type": tipo, "Accept": acepta})
    r = c.getresponse()
    return r.status, r.getheader("Location"), r.read()


def shell(*ordenes):
    txt = "\n".join(ordenes) + "\nexit\n"
    return subprocess.run(["docker", "exec", "-i", "hb", "bash", "-c", "$HBASE_HOME/bin/hbase shell -n"],
                          input=txt, capture_output=True, text=True).stdout


def put(fila, col, valor, ts=None):
    celda = {"column": b64(col), "$": b64(valor)}
    if ts:
        celda["timestamp"] = ts
    s, _, cu = rest("PUT", f"/sem/{fila}", json.dumps({"Row": [{"key": b64(fila), "Cell": [celda]}]}))
    assert s == 200, (s, cu)


def scan(**p):
    """Un scanner entero; devuelve [(fila, columna, ts, valor)]."""
    attrs = " ".join(f'{k}="{v}"' for k, v in p.items() if k not in ("filter", "extra"))
    cuerpo = f'<Scanner batch="100" {attrs}/>' if "filter" not in p else None
    if "filter" in p:
        cuerpo = f'<Scanner batch="100" {attrs}><filter>{p["filter"]}</filter></Scanner>'
    s, loc, cu = rest("PUT", "/sem/scanner", cuerpo, tipo="text/xml")
    assert s == 201, (s, cu[:200])
    ruta = loc.split("8080", 1)[1]
    sal = []
    while True:
        s, _, cu = rest("GET", ruta)
        if s == 204:
            break
        assert s == 200, (s, cu[:200])
        for f in json.loads(cu)["Row"]:
            for ce in f["Cell"]:
                sal.append((unb(f["key"]).decode(), unb(ce["column"]).decode(), ce["timestamp"], unb(ce["$"]).decode()))
    rest("DELETE", ruta)
    return sal


print(shell("disable 'sem'", "drop 'sem'")[-0:] and "", end="")
print(shell("create 'sem', {NAME=>'f', VERSIONS=>3}").strip().splitlines()[-1])

# 1. rango de clave: [inicio, fin)
for i in range(10):
    put(f"fila{i}", "f:v", f"v{i}", ts=1_700_000_000_000 + i * 1000)
print("1. rango de clave [fila3,fila6):", [x[0] for x in scan(startRow=b64("fila3"), endRow=b64("fila6"))])

# 2. rango de tiempo de celda: [startTime, endTime)
t = scan(startTime=1_700_000_003_000, endTime=1_700_000_006_000)
print("2. rango de tiempo [t3,t6):", [x[0] for x in t])

# 3. una actualizacion crea una version nueva y SE VE por el rango de tiempo
ahora = int(time.time() * 1000)
put("fila0", "f:v", "v0-nuevo")
t = scan(startTime=ahora - 5000, endTime=ahora + 60000)
print("3. actualizacion visible por tiempo:", [(x[0], x[3]) for x in t])

# 4. versiones: por defecto el scanner da la ultima
print("4. versiones de fila0 por defecto:", [(x[2] > ahora - 5000, x[3]) for x in scan(startRow=b64("fila0"), endRow=b64("fila1"))])
print("   con maxVersions=3:", [x[3] for x in scan(startRow=b64("fila0"), endRow=b64("fila1"), maxVersions=3)])

# 5. un borrado NO se ve por el rango de tiempo: la fila simplemente desaparece
rest("DELETE", "/sem/fila5")
t = scan(startTime=ahora - 5000, endTime=ahora + 60000)
print("5. tras DELETE fila5, por tiempo:", [x[0] for x in t], "| fila5 en el scan total:", any(x[0] == "fila5" for x in scan()))

# 6. un timestamp puesto por el cliente en el futuro envenena el maximo
put("futura", "f:v", "x", ts=ahora + 10 * 86_400_000)
mx = max(x[2] for x in scan())
print(f"6. maximo del timestamp tras una fila fechada +10 dias: +{(mx - ahora) / 86_400_000:.1f} dias")

# 7. caducidad del scanner (sin tocarlo durante 70 s)
s, loc, _ = rest("PUT", "/sem/scanner", '<Scanner batch="2"/>', tipo="text/xml")
ruta = loc.split("8080", 1)[1]
print("7. primer GET:", rest("GET", ruta)[0], end="; ")
time.sleep(70)
print("tras 70 s sin tocar:", rest("GET", ruta)[0])
