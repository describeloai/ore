#!/usr/bin/env python3
"""
LA FORJA DE MENTIRA — la API de Gitea/Forgejo que `ore-serve` usa para W2
(0030), sobre un repositorio pelado de verdad.

    python3 forja-de-mentira.py <repo.git> <puerto>

Lo que `crates/ore-serve/src/forja.rs` pide, y nada más:

    GET    /api/v1/version
    GET    /api/v1/repos/{o}/{r}                    default_branch
    GET    /api/v1/repos/{o}/{r}/branches           [{name, commit{id}}]
    POST   /api/v1/repos/{o}/{r}/branches           {new_branch_name, old_branch_name}
    DELETE /api/v1/repos/{o}/{r}/branches/{b}
    GET    /api/v1/repos/{o}/{r}/pulls?state=       [pr]
    POST   /api/v1/repos/{o}/{r}/pulls              {head, base, title, body}
    GET    /api/v1/repos/{o}/{r}/pulls/{n}          pr
    PATCH  /api/v1/repos/{o}/{r}/pulls/{n}          {state: closed}
    GET    /api/v1/repos/{o}/{r}/pulls/{n}/files    [{filename, status, additions, deletions}]
    GET    /api/v1/repos/{o}/{r}/pulls/{n}.diff     texto
    GET    /api/v1/repos/{o}/{r}/pulls/{n}/reviews  [review]
    POST   /api/v1/repos/{o}/{r}/pulls/{n}/reviews  {event, body}
    POST   /api/v1/repos/{o}/{r}/pulls/{n}/merge    {Do, merge_message_field, delete_branch_after_merge}

Las ramas y los diffs son git de verdad sobre el repositorio; las PRs y las
reviews viven en memoria. Como la de verdad, contesta 401 sin testigo, 404 a
lo que no está, 409 a una rama que ya existe y a un merge con conflictos, y
`mergeable` lo calcula con `git merge-tree`. Y como la de verdad, el autor
de todo es un solo usuario: `serve-mentira`.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlparse, parse_qs

REPO = os.path.abspath(sys.argv[1])
PUERTO = int(sys.argv[2])
USUARIO = {"id": 2, "login": "serve-mentira"}
PRS = []
REVIEWS = {}


def git(*args, cwd=None, entrada=None):
    r = subprocess.run(["git"] + (["--git-dir", REPO] if cwd is None else []) + list(args),
                       cwd=cwd, capture_output=True, text=True, input=entrada)
    return r.returncode, r.stdout, r.stderr


def ahora():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha_de(rama):
    rc, out, _ = git("rev-parse", "--verify", "refs/heads/%s" % rama)
    return out.strip() if rc == 0 else None


def ramas():
    rc, out, _ = git("for-each-ref", "refs/heads", "--format=%(refname:short) %(objectname)")
    return [l.split() for l in out.splitlines() if l.strip()]


def mergeable(base, head):
    rc, out, err = git("merge-tree", "--write-tree", "refs/heads/%s" % base, "refs/heads/%s" % head)
    return rc == 0


def pr_publica(pr):
    p = dict(pr)
    p["head"] = {"ref": pr["head"], "sha": sha_de(pr["head"]) or pr.get("head_sha", "")}
    p["base"] = {"ref": pr["base"], "sha": sha_de(pr["base"]) or ""}
    p["mergeable"] = (not pr["merged"]) and pr["state"] == "open" and bool(sha_de(pr["head"])) and mergeable(pr["base"], pr["head"])
    p["user"] = USUARIO
    return p


def ficheros(pr):
    rc, out, _ = git("diff", "--numstat", "refs/heads/%s...refs/heads/%s" % (pr["base"], pr["head"]))
    rc2, estados, _ = git("diff", "--name-status", "refs/heads/%s...refs/heads/%s" % (pr["base"], pr["head"]))
    estado = {}
    for l in estados.splitlines():
        partes = l.split("\t")
        if len(partes) >= 2:
            estado[partes[-1]] = {"A": "added", "M": "modified", "D": "deleted"}.get(partes[0][0], "modified")
    salida = []
    for l in out.splitlines():
        partes = l.split("\t")
        if len(partes) == 3:
            salida.append({"filename": partes[2], "status": estado.get(partes[2], "modified"),
                           "additions": int(partes[0]) if partes[0].isdigit() else 0,
                           "deletions": int(partes[1]) if partes[1].isdigit() else 0})
    return salida


def fusionar(pr, mensaje, borrar):
    tmp = tempfile.mkdtemp(prefix="forja-merge-")
    try:
        rc, _, err = git("clone", "-q", REPO, tmp, cwd=".")
        if rc != 0:
            return 500, err
        for args in (["checkout", "-q", pr["base"]],
                     ["-c", "user.name=serve-mentira", "-c", "user.email=serve@mentira", "merge", "--no-ff", "-m", mensaje, "origin/%s" % pr["head"]],
                     ["push", "-q", "origin", "HEAD:%s" % pr["base"]]):
            rc, _, err = git(*args, cwd=tmp)
            if rc != 0:
                return 409, err
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    pr["merged"] = True
    pr["merged_at"] = ahora()
    pr["state"] = "closed"
    if borrar:
        git("update-ref", "-d", "refs/heads/%s" % pr["head"])
    return 200, ""


class Manejador(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def contestar(self, codigo, cuerpo=None, texto=None):
        datos = (texto if texto is not None else json.dumps(cuerpo if cuerpo is not None else {})).encode("utf-8")
        self.send_response(codigo)
        self.send_header("Content-Type", "text/plain" if texto is not None else "application/json")
        self.send_header("Content-Length", str(len(datos)))
        self.end_headers()
        self.wfile.write(datos)

    def cuerpo(self):
        n = int(self.headers.get("Content-Length", "0") or 0)
        crudo = self.rfile.read(n) if n else b""
        try:
            return json.loads(crudo.decode("utf-8")) if crudo else {}
        except ValueError:
            return {}

    def atender(self, metodo):
        u = urlparse(self.path)
        seg = [s for s in u.path.split("/") if s]
        q = parse_qs(u.query)
        if seg[:2] != ["api", "v1"]:
            return self.contestar(404, {"message": "no es la API"})
        seg = seg[2:]
        if seg == ["version"]:
            return self.contestar(200, {"version": "forja-de-mentira"})
        if not self.headers.get("Authorization"):
            return self.contestar(401, {"message": "token required"})
        if len(seg) < 3 or seg[0] != "repos":
            return self.contestar(404, {"message": "not found"})
        resto = seg[3:]
        d = self.cuerpo() if metodo in ("POST", "PATCH") else {}

        if resto == []:
            return self.contestar(200, {"default_branch": "main", "name": seg[2], "owner": {"login": seg[1]}})
        if resto == ["branches"] and metodo == "GET":
            return self.contestar(200, [{"name": n, "commit": {"id": sha}} for n, sha in ramas()])
        if resto == ["branches"] and metodo == "POST":
            nueva, vieja = d.get("new_branch_name", ""), d.get("old_branch_name", "main")
            if sha_de(nueva):
                return self.contestar(409, {"message": "branch already exists"})
            if not sha_de(vieja):
                return self.contestar(404, {"message": "old branch not found"})
            rc, _, err = git("update-ref", "refs/heads/%s" % nueva, "refs/heads/%s" % vieja)
            return self.contestar(201 if rc == 0 else 422, {"name": nueva} if rc == 0 else {"message": err.strip()})
        if len(resto) >= 2 and resto[0] == "branches" and metodo == "DELETE":
            nombre = "/".join(resto[1:])
            if not sha_de(nombre):
                return self.contestar(404, {"message": "branch not found"})
            git("update-ref", "-d", "refs/heads/%s" % nombre)
            return self.contestar(204, texto="")
        if resto == ["pulls"] and metodo == "GET":
            estado = q.get("state", ["open"])[0]
            prs = [p for p in PRS if estado == "all" or p["state"] == estado]
            return self.contestar(200, [pr_publica(p) for p in prs])
        if resto == ["pulls"] and metodo == "POST":
            head, base = d.get("head", ""), d.get("base", "main")
            if not sha_de(head) or not sha_de(base):
                return self.contestar(404, {"message": "branch not found"})
            if any(p["state"] == "open" and p["head"] == head for p in PRS):
                return self.contestar(409, {"message": "pull request already exists for these targets"})
            pr = {"id": len(PRS) + 1, "number": len(PRS) + 1, "title": d.get("title", ""), "body": d.get("body", ""),
                  "state": "open", "merged": False, "merged_at": None, "head": head, "base": base,
                  "created_at": ahora(), "updated_at": ahora()}
            PRS.append(pr)
            REVIEWS[pr["number"]] = []
            return self.contestar(201, pr_publica(pr))
        if resto and resto[0] == "pulls" and len(resto) >= 2:
            nombre = resto[1]
            es_diff = nombre.endswith(".diff")
            try:
                n = int(nombre[:-5] if es_diff else nombre)
            except ValueError:
                return self.contestar(404, {"message": "not found"})
            pr = next((p for p in PRS if p["number"] == n), None)
            if pr is None:
                return self.contestar(404, {"message": "pull request not found"})
            if es_diff:
                rc, out, _ = git("diff", "refs/heads/%s...refs/heads/%s" % (pr["base"], pr["head"]))
                return self.contestar(200, texto=out)
            if len(resto) == 2 and metodo == "GET":
                return self.contestar(200, pr_publica(pr))
            if len(resto) == 2 and metodo == "PATCH":
                if d.get("state") == "closed" and not pr["merged"]:
                    pr["state"] = "closed"
                if d.get("title"):
                    pr["title"] = d["title"]
                return self.contestar(201, pr_publica(pr))
            if resto[2:] == ["files"]:
                return self.contestar(200, ficheros(pr))
            if resto[2:] == ["reviews"] and metodo == "GET":
                return self.contestar(200, REVIEWS[n])
            if resto[2:] == ["reviews"] and metodo == "POST":
                if d.get("event") == "APPROVED":
                    return self.contestar(422, {"message": "you cannot approve your own pull request"})
                r = {"id": len(REVIEWS[n]) + 1, "body": d.get("body", ""), "state": d.get("event", "COMMENT"),
                     "user": USUARIO, "submitted_at": ahora()}
                REVIEWS[n].append(r)
                return self.contestar(200, r)
            if resto[2:] == ["merge"] and metodo == "POST":
                if pr["state"] != "open":
                    return self.contestar(405, {"message": "pull request is not open"})
                cod, err = fusionar(pr, d.get("merge_message_field") or "Merge pull request #%d" % n, bool(d.get("delete_branch_after_merge")))
                return self.contestar(cod, {"message": err.strip()} if cod != 200 else {}, texto="" if cod == 200 else None)
        return self.contestar(404, {"message": "not found"})

    def do_GET(self):
        self.atender("GET")

    def do_POST(self):
        self.atender("POST")

    def do_PATCH(self):
        self.atender("PATCH")

    def do_DELETE(self):
        self.atender("DELETE")


if __name__ == "__main__":
    HTTPServer(("127.0.0.1", PUERTO), Manejador).serve_forever()
