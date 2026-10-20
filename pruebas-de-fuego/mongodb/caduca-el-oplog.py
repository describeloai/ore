"""Trunca el oplog de un MongoDB LOCAL de pruebas para tener una posicion CADUCADA.

Un servidor que nunca perdio historia acepta cualquier posicion anterior (medido: una de
1970 no falla), asi que para probar `ChangeStreamHistoryLost` hay que llenar el oplog.
Reduce el oplog a su minimo (990 MB) y escribe ~1,1 GB. SOLO contra un servidor de
pruebas con permiso de escritura y sin datos que importen.

Uso:  MONGO_URL=mongodb://localhost:27017/?directConnection=true python caduca-el-oplog.py
Imprime T_OLD=<segundos>:<incremento>; con ORE_MONGO_CADUCADA=<segundos con 10 cifras>:<incremento con 10 cifras>
corre el test `una_posicion_caducada_no_se_disfraza` de ore-mongodb.
"""
import json, os, subprocess, sys, time
import pymongo
from bson import Binary

URL = os.environ.get("MONGO_URL", "mongodb://localhost:27017/?directConnection=true")
c = pymongo.MongoClient(URL)
admin = c.admin
op = c.local["oplog.rs"]

print("resize:", admin.command({"replSetResizeOplog": 1, "size": 990}).get("ok"))
t_old = admin.command("hello")["operationTime"]
print("t_old:", t_old)
c.ore_caduca.semilla.insert_one({"_id": 1, "n": "antes"})
t_old = admin.command("hello")["operationTime"]
print("t_old (tras la semilla):", t_old)

blob = Binary(b"x" * 8000)
t0 = time.time()
total = 0
while True:
    lote = [{"b": blob, "i": total + k} for k in range(500)]
    c.ore_caduca.relleno.insert_many(lote, ordered=False)
    total += 500
    if total % 20000 == 0:
        primera = op.find().sort("$natural", 1).limit(1).next()["ts"]
        print(f"{total} docs, {int(time.time()-t0)} s, primera entrada del oplog: {primera}", flush=True)
        if primera > t_old:
            print("el oplog ya no llega a t_old")
            break
    if total > 400000:
        print("no se truncó")
        break

print("T_OLD=%010d:%010d" % (t_old.time, t_old.inc))
