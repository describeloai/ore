# El storage_controller en local (P2·1)

Prueba sólo con Docker, sin GCP ni cuota. Levanta:
- el controller y su Postgres;
- un broker;
- un pageserver cuyo almacenamiento remoto es un volumen local;
- un plano de control de juguete (`stub.py`) que acepta `/notify-attach` y `/notify-safekeepers` y los apunta.

Sirve para comprobar el contrato del controller antes de P2·4 (ADR 0058, B.11 P2·1).

```bash
docker compose up -d
curl -s localhost:1234/control/v1/node                     # el pageserver se registró solo (metadata.json)
curl -XPOST localhost:1234/v1/tenant -d '{"new_tenant_id":"<32 hex>"}' -H 'Content-Type: application/json'
curl -XPOST localhost:1234/v1/tenant/<t>/timeline -d '{"new_timeline_id":"<32 hex>","pg_version":17}' -H 'Content-Type: application/json'
docker compose down -v
```

El paso `permisos` sólo existe aquí: los volúmenes de Docker nacen de root y `neon` es el uid 1000. En GKE lo resuelve `fsGroup: 1000`.
