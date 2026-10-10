-- Libro · P7·4: lo que el conciliador busca por transferencia y por aviso, con índice. Sin ellos,
-- «cada transferencia, dos movimientos» y «cada aviso, un movimiento» recorren `movimiento` entero.
-- Sin CONCURRENTLY: Prisma manda el fichero de una vez (una transacción implícita), y CONCURRENTLY
-- no puede ir dentro de una. En tablas de millones de filas, esto bloquea las escrituras mientras
-- se construye: en producción, una migración aparte con CONCURRENTLY.
CREATE INDEX "movimiento_transferencia_idx" ON "movimiento"("transferencia");
CREATE INDEX "movimiento_aviso_idx" ON "movimiento"("aviso");
