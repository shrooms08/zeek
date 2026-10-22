# One image per Zeek testnet service on Railway (docs/deployment.md). The
# binary is built on the operator's Mac (scripts/railway-deploy.sh) and
# must match the sha256 recorded next to it, so Railway's builder only
# copies files and the running binary is exactly the one whose hash was
# printed.
FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates sqlite3 tini \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . /app/
RUN sha256sum -c /app/SHA256SUMS \
  && chmod 755 /app/entrypoint.sh /app/import.sh /app/ops.sh /app/zeek-*
# The engine and the keyholders shut down cleanly on SIGINT (Ctrl-C).
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/bin/tini", "--", "/app/entrypoint.sh"]
