# Escenario de cliente · HBase `saas` (0072): un SaaS multi-tenant donde cada tabla llama distinto a su clave de fila,
# asi que la conexion lleva VARIAS `clave=` (`clave=cuenta_id&clave=factura_id&clave=evento`):
#
#   saas:cuentas   clave de texto `cuenta-<tenant>-NNNN`; plan, pais, alta (texto)                      -> inferible sin declarar
#   saas:facturas  clave `int` (4 bytes); total decimal, emitida epoch-ms, pagada bool, tenant texto     -> tipos binarios
#   saas:eventos   clave compuesta `tenant#` + (Long.MAX - ts) en 8 bytes (lo ultimo, primero);
#                  pre-partida por tenant; tipo y bytes texto, peso int                                  -> clave compuesta, un tenant = un rango
#
#   docker exec -i hb bash -c '$HBASE_HOME/bin/hbase shell -n' < saas.rb

import org.apache.hadoop.hbase.util.Bytes
import org.apache.hadoop.hbase.client.Put
import org.apache.hadoop.hbase.client.ConnectionFactory
import org.apache.hadoop.hbase.HBaseConfiguration
import org.apache.hadoop.hbase.TableName
import java.math.BigDecimal
import java.math.BigInteger
import java.util.ArrayList

create_namespace 'saas'
create 'saas:cuentas', 'f'
create 'saas:facturas', 'f', SPLITS => ['\x00\x00\x07\xd0', '\x00\x00\x0f\xa0']
create 'saas:eventos', 'f', SPLITS => ['globex#', 'initech#']

conn = ConnectionFactory.createConnection(HBaseConfiguration.create)
f = Bytes.toBytes('f')
rnd = java.util.Random.new(7)
tenants = ['acme', 'globex', 'initech']
ahora = java.lang.System.currentTimeMillis

# cuentas: 50 por tenant
t = conn.getTable(TableName.valueOf('saas:cuentas'))
planes = ['free', 'pro', 'enterprise']
paises = ['ES', 'PT', 'FR', 'DE', 'IT']
lote = ArrayList.new
tenants.each do |ten|
  (1..50).each do |i|
    p = Put.new(Bytes.toBytes(format('cuenta-%s-%04d', ten, i)))
    p.addColumn(f, Bytes.toBytes('tenant'), Bytes.toBytes(ten))
    p.addColumn(f, Bytes.toBytes('plan'), Bytes.toBytes(planes[rnd.nextInt(3)]))
    p.addColumn(f, Bytes.toBytes('pais'), Bytes.toBytes(paises[rnd.nextInt(5)]))
    p.addColumn(f, Bytes.toBytes('alta'), Bytes.toBytes(format('2025-%02d-%02d', 1 + rnd.nextInt(12), 1 + rnd.nextInt(28))))
    lote.add(p)
  end
end
t.put(lote)
t.close

# facturas: 6000 filas, clave int
t = conn.getTable(TableName.valueOf('saas:facturas'))
lote = ArrayList.new
(0...6000).each do |i|
  p = Put.new(Bytes.toBytes(i.to_java(:int)))
  p.addColumn(f, Bytes.toBytes('tenant'), Bytes.toBytes(tenants[rnd.nextInt(3)]))
  p.addColumn(f, Bytes.toBytes('total'), Bytes.toBytes(BigDecimal.new(BigInteger.valueOf(900 + rnd.nextInt(900000)), 2)))
  p.addColumn(f, Bytes.toBytes('emitida'), Bytes.toBytes((ahora - rnd.nextInt(2_000_000_000)).to_java(:long)))
  p.addColumn(f, Bytes.toBytes('pagada'), Bytes.toBytes(rnd.nextInt(4) != 0))
  lote.add(p)
  if lote.size >= 1000
    t.put(lote)
    lote.clear
  end
end
t.put(lote) if lote.size > 0
t.close

# eventos: 1500 por tenant; clave `tenant#` + 8 bytes (Long.MAX - ts)
t = conn.getTable(TableName.valueOf('saas:eventos'))
tipos = ['login', 'export', 'invitacion', 'pago', 'error']
lote = ArrayList.new
tenants.each do |ten|
  (0...1500).each do |i|
    ts = ahora - i * 60_000
    inversa = java.lang.Long::MAX_VALUE - ts
    p = Put.new(Bytes.add(Bytes.toBytes(ten + '#'), Bytes.toBytes(inversa.to_java(:long))))
    p.addColumn(f, Bytes.toBytes('tipo'), Bytes.toBytes(tipos[rnd.nextInt(5)]))
    p.addColumn(f, Bytes.toBytes('bytes'), Bytes.toBytes(rnd.nextInt(5_000_000).to_java(:int)))
    lote.add(p)
  end
end
t.put(lote)
t.close
conn.close
puts 'saas listo'
