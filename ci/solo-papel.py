"""G1 (ADR 0060): las tres listas de lo que es SOLO PAPEL dicen lo mismo.

`ci.yml` lo escribe tres veces y no puede escribirlo una: `paths-ignore` de
`push` y de `pull_request` son YAML literal (GitHub no deja leerlos de otro
sitio), y `SOLO_PAPEL` es la misma lista para el `case` del paso «y solo si
sigue siendo la punta», que corre antes del checkout. Si se separan, un commit
que el CI no corre podría dejar sin desplegar al de código que tiene delante
—o al revés, uno de código se tomaría por papel—. Esto lo impide.

Las globs de GitHub (`*` no cruza `/`, `**` sí) se traducen a las de `case`
(`*` cruza `/`): `**/` desaparece y `**` es `*`.

    python3 ci/solo-papel.py [.github/workflows/ci.yml]
"""

import re
import sys


def listas(texto):
    ignorar = []
    for m in re.finditer(r"^( +)paths-ignore:\n((?:\1  - .*\n)+)", texto, re.M):
        ignorar.append([re.sub(r"^ *- *", "", l).strip().strip("'\"") for l in m.group(2).splitlines()])
    papel = re.search(r"^  SOLO_PAPEL: *[>|]-?\n((?:    .*\n)+)", texto, re.M)
    return ignorar, (papel.group(1).split() if papel else None)


def como_case(glob):
    return glob.replace("**/", "").replace("**", "*")


def main():
    ruta = sys.argv[1] if len(sys.argv) > 1 else ".github/workflows/ci.yml"
    ignorar, papel = listas(open(ruta, encoding="utf-8").read())
    mal = []
    if len(ignorar) != 2:
        mal.append(f"esperaba dos `paths-ignore` (push y pull_request), hay {len(ignorar)}")
    if not papel:
        mal.append("no encuentro `SOLO_PAPEL` en el `env` del workflow")
    for i, lista in enumerate(ignorar):
        if papel and [como_case(g) for g in lista] != papel:
            mal.append(f"paths-ignore #{i + 1} {lista} no es SOLO_PAPEL {papel}")
    if mal:
        print("✗ " + "\n✗ ".join(mal))
        sys.exit(1)
    print(f"✓ solo papel, lo mismo en los tres sitios: {' '.join(papel)}")


if __name__ == "__main__":
    main()
