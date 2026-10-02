# identidad/ — los realms de ORE

El IdP (Keycloak 26.0.7, `malla/60-idp.yaml`) corre en el clúster de ORE, y desde el
2026-09-30 **su definición también vive aquí** (ADR 0048, ORE IdP). Antes estaba en la plataforma
(`C:\Rubix\deploy\identidad`), escribiendo para un clúster donde ya no vive el IdP.

| fichero | qué |
|---|---|
| `realm.mjs` | **la definición**: flujos de entrada y de reposición (AAL2: passkey o TOTP), passkeys, factores mínimos, correo, clientes de la consola. Vino de la plataforma tal cual |
| `ore.mjs` | **lo que ORE añade** (las audiencias `ore-serve` y `modelos`, `ore-agente`, el ámbito `basic`, el registro abierto, la consola en local) y **el manifiesto**: `realmsDeOre()` es el ÚNICO realm deseado |
| `aplicar.mjs` | **el reconciliador**: deja el realm vivo como `realmsDeOre()` dice (flujos, ajustes, clientes). `--verificar` sólo mira |

```bash
node identidad/ore.mjs                 # → malla/61-realms.yaml
bash pruebas-de-fuego/el-segundo-factor.sh   # AAL2, y que el artefacto sea lo que se emite
```

El realm vivo: un `KeycloakRealmImport` sólo CREA (se salta uno que existe), así que
`61-realms.yaml` no está en `malla/kustomization.yaml` y lo vivo lo concilia `aplicar.mjs`
con el admin de arranque (la contraseña por tubería, nunca por argumento):

```bash
kubectl port-forward -n identidad pod/idp-0 18080:8080
export ORE_IDP_ADMIN="$(kubectl -n identidad get secret idp-initial-admin -o jsonpath='{.data.username}' | base64 -d)"
kubectl -n identidad get secret idp-initial-admin -o jsonpath='{.data.password}' | base64 -d | node identidad/aplicar.mjs --verificar
```

Lo que NO cambia al mudarse: el realm `rubix`, el emisor `https://login.paladio.io/realms/rubix`,
los flujos `browser-rubix`/`reposicion-rubix` y los claims `rubix_tipo`/`rubix_celda`. Cambiar
cualquiera cambia el `iss` o los tokens (ver la migración `034`).
