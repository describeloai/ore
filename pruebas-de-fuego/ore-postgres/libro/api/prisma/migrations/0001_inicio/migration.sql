-- Libro · el esquema de partida. Lo que Prisma no expresa va a mano: las comprobaciones.
CREATE TABLE "cuenta" (
    "id" TEXT NOT NULL,
    "titular" TEXT NOT NULL,
    "saldo" BIGINT NOT NULL DEFAULT 0,
    "creada" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "cuenta_pkey" PRIMARY KEY ("id"),
    CONSTRAINT "cuenta_saldo_no_negativo" CHECK ("saldo" >= 0)
);

CREATE TABLE "transferencia" (
    "id" TEXT NOT NULL,
    "origen" TEXT NOT NULL,
    "destino" TEXT NOT NULL,
    "importe" BIGINT NOT NULL,
    "hecha" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "transferencia_pkey" PRIMARY KEY ("id"),
    CONSTRAINT "transferencia_importe_positivo" CHECK ("importe" > 0),
    CONSTRAINT "transferencia_entre_dos" CHECK ("origen" <> "destino")
);

CREATE TABLE "aviso" (
    "clave" TEXT NOT NULL,
    "cuenta" TEXT NOT NULL,
    "importe" BIGINT NOT NULL,
    "recibido" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "aviso_pkey" PRIMARY KEY ("clave"),
    CONSTRAINT "aviso_importe_positivo" CHECK ("importe" > 0)
);

CREATE TABLE "movimiento" (
    "id" BIGSERIAL NOT NULL,
    "cuenta" TEXT NOT NULL,
    "importe" BIGINT NOT NULL,
    "transferencia" TEXT,
    "aviso" TEXT,
    "cuando" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "movimiento_pkey" PRIMARY KEY ("id"),
    CONSTRAINT "movimiento_de_uno" CHECK (("transferencia" IS NULL) <> ("aviso" IS NULL))
);

CREATE TABLE "cierre" (
    "dia" DATE NOT NULL,
    "cuenta" TEXT NOT NULL,
    "entradas" BIGINT NOT NULL,
    "salidas" BIGINT NOT NULL,
    "saldo_final" BIGINT NOT NULL,
    "hecho" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "cierre_pkey" PRIMARY KEY ("dia","cuenta")
);

CREATE INDEX "movimiento_cuenta_id_idx" ON "movimiento"("cuenta", "id");

ALTER TABLE "transferencia" ADD CONSTRAINT "transferencia_origen_fkey" FOREIGN KEY ("origen") REFERENCES "cuenta"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
ALTER TABLE "transferencia" ADD CONSTRAINT "transferencia_destino_fkey" FOREIGN KEY ("destino") REFERENCES "cuenta"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
ALTER TABLE "aviso" ADD CONSTRAINT "aviso_cuenta_fkey" FOREIGN KEY ("cuenta") REFERENCES "cuenta"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
ALTER TABLE "movimiento" ADD CONSTRAINT "movimiento_cuenta_fkey" FOREIGN KEY ("cuenta") REFERENCES "cuenta"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
ALTER TABLE "movimiento" ADD CONSTRAINT "movimiento_transferencia_fkey" FOREIGN KEY ("transferencia") REFERENCES "transferencia"("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "movimiento" ADD CONSTRAINT "movimiento_aviso_fkey" FOREIGN KEY ("aviso") REFERENCES "aviso"("clave") ON DELETE SET NULL ON UPDATE CASCADE;
