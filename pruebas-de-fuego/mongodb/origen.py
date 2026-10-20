"""Le pregunta al MongoDB directamente, sin pasar por ORE: la verdad contra la que se compara.

    python origen.py anios      <url> <desde ISO>   → `año|comentarios` de sample_mflix.comments
    python origen.py cuenta     <url> <desde ISO>   → cuántos comentarios desde esa fecha
    python origen.py escribir   <url>               → intenta INSERTAR; `denegada` si el rol no escribe
    python origen.py ordinal    <url>               → el operationTime ahora, `segundos:incremento`

Es lo que hace creíble una prueba de fuego: una copia que "casa con el origen" lo dice un
cliente que no es el del conector.
"""
import datetime
import sys

import pymongo
from pymongo.errors import OperationFailure


def cliente(url: str) -> pymongo.MongoClient:
    return pymongo.MongoClient(url, serverSelectionTimeoutMS=20000)


def desde(iso: str) -> datetime.datetime:
    return datetime.datetime.fromisoformat(iso.replace("Z", "+00:00"))


def main() -> int:
    orden, url = sys.argv[1], sys.argv[2]
    c = cliente(url)
    if orden in ("anios", "cuenta"):
        filtro = {"date": {"$gte": desde(sys.argv[3])}}
        col = c["sample_mflix"]["comments"]
        if orden == "cuenta":
            print(col.count_documents(filtro))
        else:
            for r in col.aggregate([{"$match": filtro}, {"$group": {"_id": {"$year": "$date"}, "n": {"$sum": 1}}}, {"$sort": {"_id": 1}}]):
                print(f"{r['_id']}|{r['n']}")
        return 0
    if orden == "escribir":
        try:
            c["ore_prueba"]["no_debe_existir"].insert_one({"x": 1})
        except OperationFailure as e:
            print("denegada" if e.code in (13, 8000) or "not authorized" in str(e) else f"otro error: {e}")
            return 0
        print("ESCRIBIÓ")
        return 1
    if orden == "ordinal":
        t = c.admin.command("hello")["operationTime"]
        print("%010d:%010d" % (t.time, t.inc))
        return 0
    print(f"orden desconocida: {orden}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
