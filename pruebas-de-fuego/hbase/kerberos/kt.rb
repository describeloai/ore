# La tabla `kt` del laboratorio con Kerberos (ficha 0072, N6): 3000 filas `k0000`..`k2999` con f:a = v<n>, en tres regiones.
#   docker exec hbk bash -c '/opt/hbase/bin/hbase shell -n /tmp/kt.rb'   (tras copiarlo con `docker cp`)
create 'kt', 'f', SPLITS => ['k1000', 'k2000']
(0...3000).each { |i| put 'kt', 'k%04d' % i, 'f:a', "v#{i}" }
count 'kt'
