# control — version web (serveur). Construire : docker build -t control .
FROM node:22-bookworm-slim

# ffmpeg (vignettes de vidéos, téléchargements), unzip et xz (installation de yt-dlp et deno depuis l'interface).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg unzip xz-utils ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev --ignore-scripts && npm cache clean --force
COPY src ./src
COPY server ./server

ENV NODE_ENV=production \
    PORT=5180 \
    CONTROL_HOST=0.0.0.0 \
    CONTROL_DATA=/data \
    CONTROL_LIBRARY=/music

RUN mkdir -p /data /music && chown node:node /data
USER node
VOLUME ["/data"]
EXPOSE 5180

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD node -e "fetch('http://127.0.0.1:'+(process.env.PORT||5180)+'/api/health').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

CMD ["node", "server/index.js"]
