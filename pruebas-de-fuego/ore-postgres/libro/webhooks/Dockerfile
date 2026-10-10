FROM node:22-slim
WORKDIR /app
COPY package.json ./
RUN npm install --no-audit --no-fund --loglevel=error
COPY servidor.mjs ./
CMD ["node", "servidor.mjs"]
