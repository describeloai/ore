"""El ORÁCULO de `ore-code`: la derivación de OOS v1alpha18 01 §4 escrita otra
vez, con el `ast` de CPython —la referencia del lenguaje— en vez de con el
parser de Ruff.

Es una prueba **diferencial**: dos implementaciones de la misma regla sobre
dos analizadores distintos tienen que decir lo mismo de cada fichero del
corpus. Lo que una lee mal, la otra lo delata.

    python3 oraculo.py                 # escribe corpus/esperado.json
    python3 oraculo.py --stdlib SALIDA # qué ficheros de la stdlib analiza CPython

Se ejecuta a mano (o en Docker) cuando cambia la regla o el corpus, y
`esperado.json` se compromete: `cargo test` no necesita Python.
"""
import ast
import json
import os
import sys

AQUI = os.path.dirname(os.path.abspath(__file__))

ESCALARES = {
    "builtins.int": "Integer",
    "builtins.float": "Float",
    "builtins.str": "String",
    "builtins.bool": "Boolean",
    "datetime.date": "Date",
    "datetime.datetime": "DateTime",
    "decimal.Decimal": "Decimal",
}
BUILTINS = {"int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "object", "type"}
LISTAS = {"builtins.list", "typing.List"}
NO_SON_CAMPOS = {"typing.ClassVar", "dataclasses.InitVar", "dataclasses.KW_ONLY"}


class NoSeDeriva(Exception):
    pass


def nombres(t):
    if isinstance(t, ast.Name):
        yield t.id
    elif isinstance(t, (ast.Tuple, ast.List)):
        for x in t.elts:
            yield from nombres(x)
    elif isinstance(t, ast.Starred):
        yield from nombres(t.value)


class Modulo:
    def __init__(self, m):
        self.m = m
        self.futuro = any(
            isinstance(s, ast.ImportFrom) and s.module == "__future__" and any(a.name == "annotations" for a in s.names)
            for s in m.body
        )
        self.ligas = []  # (índice de la sentencia del nivel superior, nombre, cualificado)
        for i, s in enumerate(m.body):
            self.recoger([s], i)
        self.clases = {s.name: (i, s) for i, s in enumerate(m.body) if isinstance(s, ast.ClassDef)}

    def recoger(self, sentencias, i):
        for s in sentencias:
            if isinstance(s, ast.Import):
                for a in s.names:
                    if a.asname:
                        self.ligas.append((i, a.asname, a.name))
                    else:
                        n = a.name.split(".")[0]
                        self.ligas.append((i, n, n))
            elif isinstance(s, ast.ImportFrom):
                base = s.module if (s.level == 0 and s.module) else "<relativo>"
                for a in s.names:
                    if a.name != "*":
                        self.ligas.append((i, a.asname or a.name, base + "." + a.name))
            elif isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                self.ligas.append((i, s.name, "<local>." + s.name))
            elif isinstance(s, ast.Assign):
                for t in s.targets:
                    for n in nombres(t):
                        self.ligas.append((i, n, "<local>." + n))
            elif isinstance(s, ast.AnnAssign) and s.value is not None:
                for n in nombres(s.target):
                    self.ligas.append((i, n, "<local>." + n))
            elif isinstance(s, ast.TypeAlias) and isinstance(s.name, ast.Name):
                self.ligas.append((i, s.name.id, "<local>." + s.name.id))
            elif isinstance(s, ast.If):
                self.recoger(s.body, i)
                self.recoger(s.orelse, i)
            elif isinstance(s, ast.Try) or (hasattr(ast, "TryStar") and isinstance(s, ast.TryStar)):
                self.recoger(s.body, i)
                for h in s.handlers:
                    self.recoger(h.body, i)
                self.recoger(s.orelse, i)
                self.recoger(s.finalbody, i)

    def cualificar(self, e, hasta):
        """El nombre cualificado de una expresión, con las ligaduras vigentes
        antes de la sentencia `hasta` (todas, si es None)."""
        if isinstance(e, ast.Name):
            q = None
            for i, n, c in self.ligas:
                if n == e.id and (hasta is None or i < hasta):
                    q = c
            if q is None and e.id in BUILTINS:
                q = "builtins." + e.id
            if q and q.startswith("typing_extensions."):
                q = "typing." + q[len("typing_extensions."):]
            return q
        if isinstance(e, ast.Attribute):
            b = self.cualificar(e.value, hasta)
            if b is None:
                return None
            q = b + "." + e.attr
            if q.startswith("typing_extensions."):
                q = "typing." + q[len("typing_extensions."):]
            return q
        return None

    def anotacion(self, e, hasta):
        """(expresión, hasta) de una anotación: entre comillas, o con
        `from __future__ import annotations`, se resuelve al final del módulo."""
        if isinstance(e, ast.Constant) and isinstance(e.value, str):
            try:
                return ast.parse(e.value.strip(), mode="eval").body, None
            except SyntaxError:
                raise NoSeDeriva("anotación entre comillas que no es Python")
        return e, (None if self.futuro else hasta)

    def miembros(self, e, hasta):
        e, hasta = self.anotacion(e, hasta)
        if isinstance(e, ast.BinOp) and isinstance(e.op, ast.BitOr):
            return self.miembros(e.left, hasta) + self.miembros(e.right, hasta)
        if isinstance(e, ast.Subscript) and self.cualificar(e.value, hasta) == "typing.Union":
            args = e.slice.elts if isinstance(e.slice, ast.Tuple) else [e.slice]
            return [x for a in args for x in self.miembros(a, hasta)]
        return [(e, hasta)]

    def tipo(self, e, hasta, en_lista=False):
        """Una anotación → (tipo OOS, opcional)."""
        e, hasta = self.anotacion(e, hasta)
        if (isinstance(e, ast.BinOp) and isinstance(e.op, ast.BitOr)) or (
            isinstance(e, ast.Subscript) and self.cualificar(e.value, hasta) == "typing.Union"
        ):
            ms = self.miembros(e, hasta)
            nada = [m for m in ms if isinstance(m[0], ast.Constant) and m[0].value is None]
            otros = [m for m in ms if not (isinstance(m[0], ast.Constant) and m[0].value is None)]
            if len(otros) != 1 or not nada:
                raise NoSeDeriva("una unión que no es T | None")
            return self.tipo(otros[0][0], otros[0][1], en_lista)[0], True
        if isinstance(e, ast.Subscript):
            base = self.cualificar(e.value, hasta)
            args = e.slice.elts if isinstance(e.slice, ast.Tuple) else [e.slice]
            if base in LISTAS and len(args) == 1:
                if en_lista:
                    raise NoSeDeriva("lista de listas")
                t, opc = self.tipo(args[0], hasta, True)
                if opc:
                    raise NoSeDeriva("lista de opcionales")
                return "list<%s>" % t, False
            if base == "typing.Optional" and len(args) == 1:
                return self.tipo(args[0], hasta, en_lista)[0], True
            if base == "typing.Annotated" and len(args) >= 2:
                return self.tipo(args[0], hasta, en_lista)
            raise NoSeDeriva("tipo sin traducción")
        q = self.cualificar(e, hasta)
        if q in ESCALARES:
            return ESCALARES[q], False
        raise NoSeDeriva("tipo sin traducción: %s" % q)

    def es_dataclass(self, i, c):
        for d in c.decorator_list:
            f = d.func if isinstance(d, ast.Call) else d
            if self.cualificar(f, i) == "dataclasses.dataclass":
                return True
        return False

    def campos(self, nombre, vistas=()):
        if nombre not in self.clases or nombre in vistas:
            raise NoSeDeriva("no es una @dataclass del fichero")
        i, c = self.clases[nombre]
        if not self.es_dataclass(i, c):
            raise NoSeDeriva("no es una @dataclass")
        campos = {}
        for b in c.bases:
            q = self.cualificar(b, i)
            if q == "builtins.object":
                continue
            if not (q and q.startswith("<local>.")):
                raise NoSeDeriva("hereda de algo que no es una @dataclass del fichero")
            for n, v in self.campos(q[len("<local>."):], vistas + (nombre,)).items():
                campos[n] = v
        if c.keywords:
            raise NoSeDeriva("argumentos de clase")
        for x in c.body:
            if not (isinstance(x, ast.AnnAssign) and isinstance(x.target, ast.Name)):
                continue
            ann, h = self.anotacion(x.annotation, i)
            base = ann.value if isinstance(ann, ast.Subscript) else ann
            if self.cualificar(base, h) in NO_SON_CAMPOS:
                continue
            t, opc = self.tipo(x.annotation, i)
            defecto = x.value is not None
            if isinstance(x.value, ast.Call) and self.cualificar(x.value.func, i) == "dataclasses.field":
                defecto = any(k.arg in ("default", "default_factory") for k in x.value.keywords)
            campos[x.target.id] = [x.target.id, t, not (defecto or opc)]
        return campos


def literal(e, lista):
    if lista:
        if not isinstance(e, ast.List) or not all(isinstance(x, ast.Constant) and isinstance(x.value, str) for x in e.elts):
            raise NoSeDeriva("no es una lista de cadenas")
        return [x.value for x in e.elts]
    if not (isinstance(e, ast.Constant) and isinstance(e.value, str)):
        raise NoSeDeriva("no es una cadena")
    return e.value


def derivar(texto, ruta):
    m = Modulo(ast.parse(texto))
    out = []
    for i, s in enumerate(m.m.body):
        if not isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        decos = [d for d in s.decorator_list if m.cualificar(d.func if isinstance(d, ast.Call) else d, i) == "ore.function"]
        if not decos:
            continue
        try:
            if isinstance(s, ast.AsyncFunctionDef):
                raise NoSeDeriva("async")
            out.append(una(m, i, s, decos[0], ruta))
        except NoSeDeriva as e:
            out.append({"name": s.name, "error": "OOS2043", "por": str(e)})
    return out


def una(m, i, s, deco, ruta):
    doc = {"name": s.name, "entrypoint": "%s:%s" % (ruta, s.name)}
    args = {}
    if isinstance(deco, ast.Call):
        if deco.args:
            raise NoSeDeriva("argumento posicional")
        for k in deco.keywords:
            if k.arg in ("over", "timeout"):
                args[k.arg] = literal(k.value, False)
            elif k.arg in ("reads", "models"):
                args[k.arg] = literal(k.value, True)
            else:
                raise NoSeDeriva("argumento %s" % k.arg)
    cuerpo = s.body
    if cuerpo and isinstance(cuerpo[0], ast.Expr) and isinstance(cuerpo[0].value, ast.Constant) and isinstance(cuerpo[0].value.value, str):
        lineas = [l.strip() for l in cuerpo[0].value.value.splitlines() if l.strip()]
        if lineas:
            doc["description"] = lineas[0]
    for k in ("over", "reads"):
        if k in args:
            doc[k] = args[k]
    if "models" in args:
        doc["models"] = [x if x.startswith("modelo/") else "modelo/" + x for x in args["models"]]
    if "timeout" in args:
        doc["timeout"] = args["timeout"]
    a = s.args
    if a.vararg or a.kwarg:
        raise NoSeDeriva("*args/**kwargs")
    posicionales = a.posonlyargs + a.args
    defaults = [None] * (len(posicionales) - len(a.defaults)) + list(a.defaults)
    params = [(p, d is not None) for p, d in zip(posicionales, defaults)]
    params += [(p, d is not None) for p, d in zip(a.kwonlyargs, a.kw_defaults)]
    if "over" in doc:
        if not posicionales or defaults[0] is not None:
            raise NoSeDeriva("sin la fila")
        params = params[1:]
    entrada = []
    for p, con_defecto in params:
        if p.annotation is None:
            raise NoSeDeriva("sin anotar: " + p.arg)
        t, opc = m.tipo(p.annotation, i)
        entrada.append([p.arg, t, not (con_defecto or opc)])
    doc["input"] = entrada
    if s.returns is None or (isinstance(s.returns, ast.Constant) and s.returns.value is None):
        raise NoSeDeriva("sin retorno")
    r, h = m.anotacion(s.returns, i)
    q = m.cualificar(r, h)
    if q and q.startswith("<local>.") and q[len("<local>."):] in m.clases:
        doc["output"] = {"campos": list(m.campos(q[len("<local>."):]).values())}
    else:
        doc["output"] = {"valor": m.tipo(s.returns, i)[0]}
    return doc


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--stdlib":
        lib = os.path.dirname(os.__file__)
        veredicto = {}
        for raiz, dirs, ficheros in os.walk(lib):
            dirs[:] = sorted(d for d in dirs if d not in ("site-packages", "__pycache__"))
            for f in sorted(ficheros):
                if f.endswith(".py"):
                    p = os.path.join(raiz, f)
                    try:
                        texto = open(p, encoding="utf-8").read()
                    except (UnicodeDecodeError, OSError):
                        continue
                    try:
                        ast.parse(texto)
                        veredicto[p] = True
                    except (SyntaxError, ValueError):
                        veredicto[p] = False
        json.dump(veredicto, open(sys.argv[2], "w", encoding="utf-8"), indent=0)
        print("stdlib:", len(veredicto), "ficheros,", sum(veredicto.values()), "los analiza CPython", sys.version.split()[0])
        return
    corpus = os.path.join(AQUI, "corpus", "funciones")
    esperado = {}
    for f in sorted(os.listdir(corpus)):
        texto = open(os.path.join(corpus, f), encoding="utf-8").read()
        try:
            esperado[f] = derivar(texto, "funciones/" + f)
        except SyntaxError as e:
            esperado[f] = {"sintaxis": str(e)}
    with open(os.path.join(AQUI, "corpus", "esperado.json"), "w", encoding="utf-8", newline="\n") as fh:
        json.dump(esperado, fh, indent=1, ensure_ascii=False)
        fh.write("\n")
    print("funciones:", sum(len(v) for v in esperado.values() if isinstance(v, list)), "en", len(esperado), "ficheros")


if __name__ == "__main__":
    main()
