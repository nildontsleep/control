# control — version web (serveur Rust). Construire : docker build -t control .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
# La chaîne est celle de rust-toolchain.toml ; seul le serveur est compilé (il ne dépend pas de l'interface Rune).
RUN rustup show active-toolchain || rustup toolchain install
RUN cargo build --release --locked -p control-server

FROM debian:bookworm-slim
# ffmpeg (vignettes de vidéos, téléchargements, Opus), certificats (TMDB, GitHub…), curl (test de santé).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1000 --home /data control
WORKDIR /app
COPY --from=build /src/target/release/control-server /app/control-server
COPY src/renderer /app/src/renderer
COPY src/web /app/src/web

ENV PORT=5180 \
    CONTROL_HOST=0.0.0.0 \
    CONTROL_DATA=/data \
    CONTROL_LIBRARY=/music \
    CONTROL_ASSETS=/app

RUN mkdir -p /data /music && chown control:control /data
USER control
VOLUME ["/data"]
EXPOSE 5180

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD curl -fsS "http://127.0.0.1:${PORT:-5180}/api/health" >/dev/null || exit 1

# Le lanceur fait tourner le serveur et le redémarre sur sa nouvelle version quand la mise à jour automatique en a
# installé une (et revient à la précédente si elle ne démarre pas).
CMD ["/app/control-server", "--launcher"]
