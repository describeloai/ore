# ver.sh <job>: espera la pasada y enseña sus líneas útiles y los dos punteros
J=$1
kubectl -n t-victor wait --for=condition=complete job/$J --timeout=900s >/dev/null 2>&1 || kubectl -n t-victor get job $J
echo "--- $J: $(kubectl -n t-victor get job $J -o jsonpath='{.status.startTime} → {.status.completionTime}')"
kubectl -n t-victor logs job/$J -c copiar 2>&1 | grep -v "^\s*$" | grep -i -E "pg_test|bq_test|ya est|copia|error|✗|⚠|filas|testigo|commit|push|rehaciendo|nada" | head -30
MSYS_NO_PATHCONV=1 kubectl -n t-victor exec deploy/ore-serve -c ore-serve -- sh -c 'cd /tmp/arbol-m && git -c http.extraheader="Authorization: token $(cat /testigo/forja)" pull -q && for f in datasets/pg_test_s/ore_salud/pedidos.json datasets/bq_test_s/ventas/ore_salud_pedidos.json; do echo "@@ $f · $(git log -1 --format="%ci %h" -- $f)"; tr -d "\n " < $f | sed "s/\"bundle\":\"[^\"]*\",//;s/\"plan\":\"[^\"]*\",//;s/\"cabecera\":\"[^\"]*\",//;s/\"ubicacion\":\"[^\"]*\",//;s/gs:[^\"]*metadata\//…\//"; echo; done'
