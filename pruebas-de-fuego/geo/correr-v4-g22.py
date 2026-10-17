"""Lanza vara-v4-g22.py (ADR 0067 G2·2) en un puesto de victor, con `spatial` y `h3` traídas del bucket
(puesto-extensiones.py delante). Deja el registro en pruebas-de-fuego/geo/v4-g22-<fecha>.log."""
import importlib.util, os, time
S = os.path.dirname(os.path.abspath(__file__)) + os.sep
spec = importlib.util.spec_from_file_location("vara", S + "vara.py"); v = importlib.util.module_from_spec(spec); spec.loader.exec_module(v)
ext = open(S + "puesto-extensiones.py", encoding="utf-8").read().split('if __name__ == "__main__":')[0]
import sys
# `python correr-v4-g22.py ""` mide solo H3 y particiones; `"b_hilbert_100k,c_hilbert_25k"`, esas variantes.
solo = f"SOLO = {[x for x in sys.argv[1].split(',') if x]!r}\n" if len(sys.argv) > 1 else ""
guion = ext + "\n" + solo + open(S + "vara-v4-g22.py", encoding="utf-8").read()
fecha = time.strftime("%Y%m%d-%H%M")
log = v.job(f"geo-v4-g22-{int(time.time()) % 10**6}", ["python3", "-u", "-c", guion], imagen=v.PUESTO, cuenta="puesto", plazo=3600, memoria="4Gi")
open(S + f"v4-g22-{fecha}.log", "w", encoding="utf-8", newline="\n").write(log)
print("\n".join(l for l in log.splitlines() if l.startswith(("V4", "##")) or "Error" in l or "Traceback" in l or "Reason" in l))
