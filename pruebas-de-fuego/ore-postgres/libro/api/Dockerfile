FROM node:22-slim
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends openssl ca-certificates >/dev/null && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY package.json ./
RUN npm install --no-audit --no-fund --loglevel=error
COPY prisma ./prisma
RUN npx prisma generate >/dev/null
COPY servidor.mjs ./
# Al arrancar, las migraciones (por DIRECT_URL) y después el servidor (por DATABASE_URL, el pool).
CMD ["sh", "-c", "npx prisma migrate deploy && exec node servidor.mjs"]
