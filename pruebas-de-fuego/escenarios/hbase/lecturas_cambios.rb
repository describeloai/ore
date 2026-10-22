# Escenario de cliente · HBase `ventas:lecturas`: lo que pasa ENTRE dos refrescos incrementales (0072, N5):
# 50 filas actualizadas (valor nuevo, marca de ahora), 30 filas nuevas y 10 borradas (que el incremental NO ve).
#
#   docker exec -i hb bash -c '$HBASE_HOME/bin/hbase shell -n' < lecturas_cambios.rb

import org.apache.hadoop.hbase.util.Bytes
import org.apache.hadoop.hbase.client.Put
import org.apache.hadoop.hbase.client.Delete
import org.apache.hadoop.hbase.client.ConnectionFactory
import org.apache.hadoop.hbase.HBaseConfiguration
import org.apache.hadoop.hbase.TableName

conn = ConnectionFactory.createConnection(HBaseConfiguration.create)
t = conn.getTable(TableName.valueOf('ventas:lecturas'))
f = Bytes.toBytes('f')
rnd = java.util.Random.new(7)
(0...50).each do |i|
  p = Put.new(Bytes.toBytes(format('s%04d', i * 3)))
  p.addColumn(f, Bytes.toBytes('valor'), Bytes.toBytes((1000 + rnd.nextDouble * 100).to_java(:double)))
  t.put(p)
end
(1000...1030).each do |i|
  p = Put.new(Bytes.toBytes(format('s%04d', i)))
  p.addColumn(f, Bytes.toBytes('sensor'), Bytes.toBytes("sensor-#{i % 20}"))
  p.addColumn(f, Bytes.toBytes('valor'), Bytes.toBytes((rnd.nextDouble * 100).to_java(:double)))
  t.put(p)
end
(900...910).each { |i| t.delete(Delete.new(Bytes.toBytes(format('s%04d', i)))) }
t.close
conn.close
puts 'cambios aplicados'
