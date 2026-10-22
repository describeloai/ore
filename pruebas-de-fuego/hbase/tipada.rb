# La tabla `tipada` del laboratorio de HBase (ficha 0072, N4): las filas las escribe **Java**, con
# `org.apache.hadoop.hbase.util.Bytes.toBytes`, que es el oraculo de las codificaciones de `ore-hbase`.
# Se pasa al shell de HBase por la entrada estandar:
#
#   docker exec -i hb bash -c '$HBASE_HOME/bin/hbase shell -n' < tipada.rb
#
# Clave: un `int` (`i32be`). Se parte en tres regiones por los bytes `00000000` y `80000000`: los positivos caen en una y
# los negativos en otra, porque en complemento a dos los negativos ordenan DESPUES de los positivos.
#
#   f:edad   int      (n.remainder(1_000_000))    -> i32be
#   f:corto  short    (n.remainder(30000))        -> i16be
#   f:largo  long     (n * 1_000_000_007)         -> i64be
#   f:flotante float  (n * 0.5)                   -> f32be
#   f:doble  double   (n / 3.0)                   -> f64be
#   f:ok     boolean  (n par)                     -> bool
#   f:total  BigDecimal (n*100+5, escala 2)       -> decimal
#   f:ms     long     (1_700_000_000_000 + n*1000) -> epoch-ms
#   f:nombre String   ("n=" + n)                  -> utf8
#   f:texto  String   (n.to_s)                    -> texto (un Integer escrito como texto)

import org.apache.hadoop.hbase.util.Bytes
import org.apache.hadoop.hbase.client.Put
import org.apache.hadoop.hbase.client.ConnectionFactory
import org.apache.hadoop.hbase.HBaseConfiguration
import org.apache.hadoop.hbase.TableName
import java.math.BigDecimal
import java.math.BigInteger

create 'tipada', 'f', SPLITS => ['\x00\x00\x00\x00', '\x80\x00\x00\x00']

# Y una con clave compuesta: usuario (texto que acaba en `#`) + dia (un `int`), partida por usuario.
create 'compuesta', 'f', SPLITS => ['beto#', 'carla#']

conn = ConnectionFactory.createConnection(HBaseConfiguration.create)
tabla = conn.getTable(TableName.valueOf('tipada'))
[-2147483648, -100, -5, 0, 7, 100, 100000, 2147483647].each do |n|
  p = Put.new(Bytes.toBytes(n.to_java(:int)))
  f = Bytes.toBytes('f')
  doble = n.to_f / 3.0
  p.addColumn(f, Bytes.toBytes('edad'), Bytes.toBytes(n.remainder(1_000_000).to_java(:int)))
  p.addColumn(f, Bytes.toBytes('corto'), Bytes.toBytes(n.remainder(30000).to_java(:short)))
  p.addColumn(f, Bytes.toBytes('largo'), Bytes.toBytes((n * 1_000_000_007).to_java(:long)))
  p.addColumn(f, Bytes.toBytes('flotante'), Bytes.toBytes((n * 0.5).to_java(:float)))
  p.addColumn(f, Bytes.toBytes('doble'), Bytes.toBytes(doble.to_java(:double)))
  p.addColumn(f, Bytes.toBytes('ok'), Bytes.toBytes((n % 2 == 0).to_java(:boolean)))
  p.addColumn(f, Bytes.toBytes('total'), Bytes.toBytes(BigDecimal.new(BigInteger.valueOf(n * 100 + 5), 2)))
  p.addColumn(f, Bytes.toBytes('ms'), Bytes.toBytes((1_700_000_000_000 + n * 1000).to_java(:long)))
  p.addColumn(f, Bytes.toBytes('nombre'), Bytes.toBytes("n=#{n}"))
  p.addColumn(f, Bytes.toBytes('texto'), Bytes.toBytes(n.to_s))
  tabla.put(p)
end
tabla.close
compuesta = conn.getTable(TableName.valueOf('compuesta'))
['ana', 'beto', 'carla', 'dani'].each do |u|
  [-3, -1, 0, 2].each do |d|
    p = Put.new(Bytes.add(Bytes.toBytes(u + '#'), Bytes.toBytes(d.to_java(:int))))
    p.addColumn(Bytes.toBytes('f'), Bytes.toBytes('v'), Bytes.toBytes("#{u}:#{d}"))
    compuesta.put(p)
  end
end
compuesta.close
conn.close
