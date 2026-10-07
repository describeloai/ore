"""especificacion.py — lo que hoy hace a mano la prueba y mañana hará el plano de control (P4):

  especificacion.py spec  <vm> <tenant> <timeline>   → la especificación de compute_ctl (config.json)
  especificacion.py token <vm>                       → un JWT para hablarle a ese compute_ctl (:3080)

Con ORE_PG_AUTH=si (entorno.sh lo pone si el almacenamiento tiene `almacen-jwt`) la especificación
lleva `storage_auth_token`: un token de scope `tenant` firmado con la privada del almacenamiento.

La clave Ed25519 con que se firman los tokens se crea la primera vez en $ORE_PG_TRABAJO/jwt.pem y su
JWKS va dentro de la especificación (`compute_ctl_config.jwks`). Medido en B.5: la clave de ejemplo de
Neon está mal codificada, y `compute_ctl` exige `compute_id` en el token.
"""
import base64, hashlib, json, os, pathlib, sys, time

TRABAJO = pathlib.Path(os.environ["ORE_PG_TRABAJO"])
NS = os.environ["ORE_PG_NS"]


def clave():
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    pem = TRABAJO / "jwt.pem"
    if not pem.exists():
        k = Ed25519PrivateKey.generate()
        pem.write_bytes(k.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                        serialization.NoEncryption()))
    k = serialization.load_pem_private_key(pem.read_bytes(), password=None)
    x = k.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    kid = hashlib.sha256(x).hexdigest()[:16]
    jwk = {"use": "sig", "key_ops": ["verify"], "alg": "EdDSA", "kid": kid, "kty": "OKP", "crv": "Ed25519",
           "x": base64.urlsafe_b64encode(x).rstrip(b"=").decode()}
    return pem.read_text(), jwk


def ajuste(nombre, valor, tipo):
    return {"name": nombre, "value": valor, "vartype": tipo}


def spec(vm, tenant, timeline):
    _, jwk = clave()
    sk = ",".join(f"safekeeper-{i}.{NS}.svc.cluster.local:5454" for i in range(3))   # Services ClusterIP: IP estable (P2·6)
    ajustes = [
        ajuste("fsync", "on", "bool"), ajuste("wal_level", "logical", "enum"),
        ajuste("wal_log_hints", "on", "bool"), ajuste("log_connections", "on", "bool"),
        ajuste("port", "55433", "integer"), ajuste("shared_buffers", "128MB", "string"),
        ajuste("max_connections", "100", "integer"), ajuste("listen_addresses", "0.0.0.0", "string"),
        ajuste("max_wal_senders", "10", "integer"), ajuste("max_replication_slots", "10", "integer"),
        ajuste("wal_sender_timeout", "5s", "string"), ajuste("wal_keep_size", "0", "integer"),
        ajuste("password_encryption", "md5", "enum"), ajuste("restart_after_crash", "off", "bool"),
        ajuste("synchronous_standby_names", "walproposer", "string"),
        ajuste("shared_preload_libraries", "neon,pg_cron,timescaledb,pg_stat_statements", "string"),
        ajuste("neon.safekeepers", sk, "string"),
        ajuste("neon.tenant_id", tenant, "string"), ajuste("neon.timeline_id", timeline, "string"),
        ajuste("neon.pageserver_connstring", f"host=pageserver-0.{NS}.svc.cluster.local port=6400", "string"),
        ajuste("max_replication_write_lag", "500MB", "string"),
        ajuste("max_replication_flush_lag", "10GB", "string"),
        ajuste("cron.database", "postgres", "string"),
        ajuste("neon.max_file_cache_size", "1GB", "string"), ajuste("neon.file_cache_size_limit", "1GB", "string"),
        ajuste("neon.file_cache_path", "/var/db/postgres/compute/file.cache", "string"),
    ]
    spec = {
        "spec": {
            "format_version": 1.0,
            "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime()),
            "operation_uuid": "00000000-0000-0000-0000-000000000000",
            "suspend_timeout_seconds": 60,
            "cluster": {
                "cluster_id": NS, "name": vm, "state": "restarted",
                # contraseña md5 de cloud_admin = md5('cloud_admin' + 'cloud_admin'), la del compose de Neon
                "roles": [{"name": "cloud_admin", "encrypted_password": "b093c0d3b281ba6da1eacc608620abd8",
                           "options": None}],
                "databases": [], "settings": ajustes,
            },
            "delta_operations": [],
        },
        "compute_ctl_config": {"jwks": {"keys": [jwk]}},
    }
    # P2·4: si el almacenamiento exige autenticación, el cómputo lleva un token de scope `tenant`
    # firmado con la privada del almacenamiento (la que acuñará el plano de control en P4)
    if os.environ.get("ORE_PG_AUTH") == "si":
        spec["spec"]["storage_auth_token"] = token_de_tenant(tenant)
    return spec


def token_de_tenant(tenant):
    import jwt
    return jwt.encode({"scope": "tenant", "tenant_id": tenant},
                      pathlib.Path(os.environ["ORE_PG_PRIVADA"]).read_text(), algorithm="EdDSA")


def token(vm):
    import jwt
    pem, jwk = clave()
    return jwt.encode({"compute_id": vm, "exp": int(time.time()) + 3600}, pem, algorithm="EdDSA",
                      headers={"kid": jwk["kid"]})


if __name__ == "__main__":
    orden = sys.argv[1]
    if orden == "spec":
        print(json.dumps(spec(*sys.argv[2:5]), indent=1))
    elif orden == "token":
        print(token(sys.argv[2]))
    else:
        sys.exit(__doc__)
