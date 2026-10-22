# Medida de rendimiento · HBase `bench:filas`: 1 000 000 de filas, clave `int` (4 bytes), 8 regiones, 5 columnas tipadas.
# Sirve para medir cuanto tarda el camino de la copia (lectura -> Parquet -> lago) con un millon de filas.
#
#   docker exec -i hb bash -c '$HBASE_HOME/bin/hbase shell -n' < millon.rb

import org.apache.hadoop.hbase.util.Bytes
import org.apache.hadoop.hbase.client.Put
import org.apache.hadoop.hbase.client.ConnectionFactory
import org.apache.hadoop.hbase.HBaseConfiguration
import org.apache.hadoop.hbase.TableName
import java.math.BigDecimal
import java.math.BigInteger
import java.util.ArrayList

N = 1_000_000

create_namespace 'bench'
splits = (1..7).map { |i| Bytes.toStringBinary(Bytes.toBytes((i * N / 8).to_java(:int))) }
create 'bench:filas', 'f', SPLITS => splits

conn = ConnectionFactory.createConnection(HBaseConfiguration.create)
f = Bytes.toBytes('f')
rnd = java.util.Random.new(11)
ahora = java.lang.System.currentTimeMillis
paises = ['ES', 'PT', 'FR', 'DE', 'IT', 'NL', 'BE']
t = conn.getTable(TableName.valueOf('bench:filas'))
lote = ArrayList.new
(0...N).each do |i|
  p = Put.new(Bytes.toBytes(i.to_java(:int)))
  p.addColumn(f, Bytes.toBytes('pais'), Bytes.toBytes(paises[rnd.nextInt(7)]))
  p.addColumn(f, Bytes.toBytes('importe'), Bytes.toBytes(BigDecimal.new(BigInteger.valueOf(100 + rnd.nextInt(10_000_000)), 2)))
  p.addColumn(f, Bytes.toBytes('cantidad'), Bytes.toBytes(rnd.nextInt(500).to_java(:int)))
  p.addColumn(f, Bytes.toBytes('creada'), Bytes.toBytes((ahora - rnd.nextInt(2_000_000_000)).to_java(:long)))
  p.addColumn(f, Bytes.toBytes('activa'), Bytes.toBytes(rnd.nextInt(5) != 0))
  lote.add(p)
  if lote.size >= 5000
    t.put(lote)
    lote.clear
  end
end
t.put(lote) if lote.size > 0
t.close
conn.close
puts 'millon listo'
