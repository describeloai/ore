# Escenario de cliente · HBase `ventas` (0072): un cluster que anuncia SUS nombres internos (aqui 127.0.0.1) y se lee con
# `traduccion=`; cuatro tablas de las que un cliente tiene:
#
#   ventas:pedidos   clave `int` (4 bytes, Java), decimal/fecha/short escritos con Bytes.toBytes, 4 regiones   -> tipos binarios
#   ventas:clientes  todo texto UTF-8 (nombre, ciudad, alta)                                                   -> lo «inferible» sin declarar
#   ventas:sesiones  clave compuesta `usuario#` + `int` (el dia)                                               -> clave compuesta
#   ventas:lecturas  1000 filas con marcas de tiempo de ayer; `lecturas_cambios.rb` actualiza y borra          -> incremental por `_ts`
#
#   docker exec -i hb bash -c '$HBASE_HOME/bin/hbase shell -n' < ventas.rb

import org.apache.hadoop.hbase.util.Bytes
import org.apache.hadoop.hbase.client.Put
import org.apache.hadoop.hbase.client.ConnectionFactory
import org.apache.hadoop.hbase.HBaseConfiguration
import org.apache.hadoop.hbase.TableName
import java.math.BigDecimal
import java.math.BigInteger
import java.util.ArrayList

create_namespace 'ventas'
create 'ventas:pedidos', 'f', SPLITS => ['\x00\x00\x13\x88', '\x00\x00\x27\x10', '\x00\x00\x3a\x98']
create 'ventas:clientes', 'f'
create 'ventas:sesiones', 'f', SPLITS => ['beto#', 'carla#']
create 'ventas:lecturas', 'f'

conn = ConnectionFactory.createConnection(HBaseConfiguration.create)
f = Bytes.toBytes('f')
rnd = java.util.Random.new(42)
estados = ['nuevo', 'pagado', 'enviado', 'entregado', 'devuelto']
ahora = java.lang.System.currentTimeMillis

# pedidos: 20000 filas, clave int
t = conn.getTable(TableName.valueOf('ventas:pedidos'))
lote = ArrayList.new
(0...20000).each do |i|
  p = Put.new(Bytes.toBytes(i.to_java(:int)))
  p.addColumn(f, Bytes.toBytes('cliente'), Bytes.toBytes((1 + rnd.nextInt(2000)).to_java(:int)))
  p.addColumn(f, Bytes.toBytes('total'), Bytes.toBytes(BigDecimal.new(BigInteger.valueOf(500 + rnd.nextInt(250000)), 2)))
  p.addColumn(f, Bytes.toBytes('fecha'), Bytes.toBytes((ahora - rnd.nextInt(2_000_000_000)).to_java(:long)))
  p.addColumn(f, Bytes.toBytes('estado'), Bytes.toBytes(estados[rnd.nextInt(5)]))
  p.addColumn(f, Bytes.toBytes('unidades'), Bytes.toBytes((1 + rnd.nextInt(20)).to_java(:short)))
  lote.add(p)
  if lote.size >= 1000
    t.put(lote)
    lote.clear
  end
end
t.put(lote) if lote.size > 0
t.close

# clientes: 2000 filas, todo texto
t = conn.getTable(TableName.valueOf('ventas:clientes'))
ciudades = ['Madrid', 'Barcelona', 'Valencia', 'Sevilla', 'Bilbao']
lote = ArrayList.new
(1..2000).each do |i|
  p = Put.new(Bytes.toBytes(format('c%05d', i)))
  p.addColumn(f, Bytes.toBytes('nombre'), Bytes.toBytes("Cliente #{i}"))
  p.addColumn(f, Bytes.toBytes('ciudad'), Bytes.toBytes(ciudades[rnd.nextInt(5)]))
  p.addColumn(f, Bytes.toBytes('alta'), Bytes.toBytes(format('2025-%02d-%02d', 1 + rnd.nextInt(12), 1 + rnd.nextInt(28))))
  lote.add(p)
end
t.put(lote)
t.close

# sesiones: usuario# + dia (int)
t = conn.getTable(TableName.valueOf('ventas:sesiones'))
lote = ArrayList.new
['ana', 'beto', 'carla', 'dani'].each do |u|
  (0...200).each do |d|
    p = Put.new(Bytes.add(Bytes.toBytes(u + '#'), Bytes.toBytes(d.to_java(:int))))
    p.addColumn(f, Bytes.toBytes('paginas'), Bytes.toBytes((1 + rnd.nextInt(60)).to_java(:int)))
    p.addColumn(f, Bytes.toBytes('ms'), Bytes.toBytes((rnd.nextInt(3_600_000)).to_java(:long)))
    lote.add(p)
  end
end
t.put(lote)
t.close

# lecturas: 1000 filas con marca de AYER (explicita), para que el incremental tenga un antes
t = conn.getTable(TableName.valueOf('ventas:lecturas'))
ayer = ahora - 86_400_000
lote = ArrayList.new
(0...1000).each do |i|
  p = Put.new(Bytes.toBytes(format('s%04d', i)))
  p.addColumn(f, Bytes.toBytes('sensor'), ayer + i, Bytes.toBytes("sensor-#{i % 20}"))
  p.addColumn(f, Bytes.toBytes('valor'), ayer + i, Bytes.toBytes((rnd.nextDouble * 100).to_java(:double)))
  lote.add(p)
end
t.put(lote)
t.close
conn.close
puts 'ventas listo'
