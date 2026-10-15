#!/bin/sh
# L3 · ¿cuánto tarda `git` en rendirse ante una forja que no contesta? (2026-10-14)
#
# ore-serve no pone plazo a ningún `git` (git.rs) y retiene el candado del espejo
# mientras `git fetch` habla con la forja. En victor, la forja no recibió nada entre
# 20:36:07 y 20:41:10 (303 s) mientras todo ore-serve esperaba. Esto cronometra a git,
# sin ajustes (como ore-serve), en tres formas de no contestar:
#   A · la conexión no se establece (los SYN se descartan: una política de red, un
#       pod que se fue sin que el Service lo supiera);
#   B · se establece y el servidor no dice nada;
#   C · el servidor manda las cabeceras y se queda a medias.
# Y lo mismo con `http.lowSpeedLimit=1000 http.lowSpeedTime=10` (el arreglo
# candidato) para ver que corta.
#
#   docker run --rm --cap-add NET_ADMIN -v "$PWD:/l3:ro" alpine:3.22 sh /l3/l3-git-sin-plazo.sh
set -u
apk add -q --no-cache git iptables python3 >/dev/null 2>&1
git --version
TOPE=${TOPE:-360}
python3 - <<'PY' &
import socket, threading, time
def b():  # acepta y calla
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", 9001)); s.listen(50)
    while True:
        c, _ = s.accept(); threading.Thread(target=lambda c=c: time.sleep(10**6), daemon=True).start()
def c():  # cabeceras y a medias
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", 9002)); s.listen(50)
    while True:
        k, _ = s.accept()
        def h(k=k):
            k.recv(65536)
            k.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 100000\r\n\r\n001e# service=git-upload-pack\n")
            time.sleep(10**6)
        threading.Thread(target=h, daemon=True).start()
threading.Thread(target=b, daemon=True).start(); threading.Thread(target=c, daemon=True).start()
time.sleep(10**7)
PY
sleep 1
# A · los SYN hacia 9000 se descartan
iptables -A OUTPUT -p tcp --dport 9000 -j DROP

mide() { # caso url [config…]
  caso=$1; url=$2; shift 2
  t0=$(cut -d" " -f1 /proc/uptime)
  r=$(GIT_TERMINAL_PROMPT=0 timeout $TOPE git "$@" ls-remote --heads "$url" 2>&1 | tail -1)
  rc=$?
  t1=$(cut -d" " -f1 /proc/uptime)
  s=$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.1f", b-a}')
  echo "L3 · $caso · ${s} s · $(echo "$r" | cut -c1-110)"
}
mide "A sin plazo (SYN descartados)" http://127.0.0.1:9000/r.git &
mide "B sin plazo (acepta y calla)" http://127.0.0.1:9001/r.git &
mide "C sin plazo (cabeceras y a medias)" http://127.0.0.1:9002/r.git &
mide "A con lowSpeed 1000/10" http://127.0.0.1:9000/r.git -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=10 &
mide "B con lowSpeed 1000/10" http://127.0.0.1:9001/r.git -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=10 &
mide "C con lowSpeed 1000/10" http://127.0.0.1:9002/r.git -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=10 &
wait
