"""plantilla.py <fichero> [VAR=valor ...] — imprime el fichero con cada ${ORE_PG_*} sustituido.

Los valores salen del entorno (entorno.sh) o de los VAR=valor de la línea, que mandan. Falla si queda
alguna ${ORE_PG_*} sin valor: un manifiesto a medias no se aplica.
"""
import os, re, sys

texto = open(sys.argv[1], encoding="utf-8").read()
valores = {k: v for k, v in os.environ.items() if k.startswith("ORE_PG_")}
for par in sys.argv[2:]:
    k, v = par.split("=", 1)
    valores[k] = v

faltan = sorted({m for m in re.findall(r"\$\{(ORE_PG_[A-Z_]+)\}", texto) if not valores.get(m)})
if faltan:
    sys.exit(f"ERROR {sys.argv[1]}: sin valor para {', '.join(faltan)}")
sys.stdout.write(re.sub(r"\$\{(ORE_PG_[A-Z_]+)\}", lambda m: valores[m.group(1)], texto))
