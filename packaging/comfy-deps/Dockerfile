FROM ubuntu:22.04
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY photocraft-comfy-deps /usr/local/bin/photocraft-comfy-deps
COPY LICENSE-MIT LICENSE-APACHE /opt/photocraft/licenses/
USER 1000:1000
ENV PHOTOCRAFT_MODELS_DIR=/models PHOTOCRAFT_DEPS_LISTEN=0.0.0.0:8190
EXPOSE 8190
ENTRYPOINT ["/usr/local/bin/photocraft-comfy-deps"]
