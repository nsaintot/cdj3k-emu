FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        cryptsetup-bin \
        mount \
        util-linux \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /work
COPY decrypt.sh /usr/local/bin/decrypt.sh
RUN chmod +x /usr/local/bin/decrypt.sh

ENTRYPOINT ["/usr/local/bin/decrypt.sh"]
