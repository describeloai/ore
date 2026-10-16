"""ADR 0065: un guion de medida en un Job del inquilino, con la credencial del agente.

    python pruebas-de-fuego/salud/pg0-en-la-celda.py pg_quality            (pg0-de-verdad.sh, victor)
    python pruebas-de-fuego/salud/pg0-en-la-celda.py - pg1-censo.sh demo   (el censo de PG1, demo)

Las URLs salen del cofre dentro del Job y no tocan este PC. Imprime el registro del Job.
"""
import json
import os
import subprocess
import sys
import time

S = os.path.dirname(os.path.abspath(__file__)) + os.sep


def sh(args, entrada=None):
    p = subprocess.run(args, input=entrada, capture_output=True, text=True, encoding='utf-8')
    return p.returncode, (p.stdout or '') + (p.stderr or '')


def main():
    fuente = sys.argv[1]
    guion = sys.argv[2] if len(sys.argv) > 2 else 'pg0-de-verdad.sh'
    tenant = sys.argv[3] if len(sys.argv) > 3 else 'victor'
    ns = 't-' + tenant
    nombre = 'pg0-%d' % (time.time() * 1000 % 10 ** 9)
    # job-base.json es de victor: el inquilino se cambia en el texto (etiquetas, secretos, organización).
    base = open(S + 'job-base.json', encoding='utf-8').read()
    if tenant != 'victor':
        base = base.replace('t-victor', ns).replace('"victor"', '"%s"' % tenant)
    d = json.loads(base)
    d['metadata']['name'] = nombre
    k = d['spec']['template']['spec']['containers'][0]
    k['command'] = ['/bin/sh', '-c', open(S + guion, encoding='utf-8').read()]
    k['imagePullPolicy'] = 'Always'  # :main se mueve; la cache del nodo daria el binario de antes
    k['env'] = [e for e in k['env'] if e['name'] not in ('FUENTE', 'NS')] + [
        {'name': 'FUENTE', 'value': fuente}, {'name': 'NS', 'value': ns}]
    d['spec']['activeDeadlineSeconds'] = 600
    d['spec']['backoffLimit'] = 0
    c, o = sh(['kubectl', 'create', '-f', '-'], json.dumps(d))
    if c != 0:
        raise SystemExit('kubectl create: ' + o[-300:])
    fin = time.time() + 700
    while time.time() < fin:
        _, o = sh(['kubectl', '-n', ns, 'get', 'job', nombre, '-o', 'jsonpath={.status.succeeded}/{.status.failed}'])
        if o.startswith('1/') or o.endswith('/1'):
            break
        time.sleep(5)
    _, o = sh(['kubectl', '-n', ns, 'logs', 'job/' + nombre, '-c', 'medir'])
    print(o)
    sh(['kubectl', '-n', ns, 'delete', 'job', nombre, '--wait=false'])


if __name__ == '__main__':
    main()
