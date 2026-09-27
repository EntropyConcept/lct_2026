# syntax=docker/dockerfile:1
FROM rust:1.95.0-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY web ./web
COPY norms.xlsx city-example.json demo.json ./
COPY dataset ./dataset
RUN cargo build --release --locked

FROM build AS test
# Networking tests exercise short deadlines; avoid contention between tests.
RUN cargo test --release --locked -- --test-threads=1

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home dispatch \
    && mkdir -p /app /var/lib/dispatch/routing-cache \
    && chown -R dispatch:dispatch /var/lib/dispatch
WORKDIR /app
COPY --from=build /build/target/release/dispatch-sat /usr/local/bin/dispatch-sat
COPY web/vendor/LEAFLET-LICENSE /usr/share/doc/dispatch-sat/LEAFLET-LICENSE
COPY dataset ./dataset
COPY norms.xlsx city-example.json demo.json ./
ENV DISPATCH_BIND=0.0.0.0 \
    DISPATCH_ROUTING_CACHE=/var/lib/dispatch/routing-cache
USER dispatch
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8080/api/norms > /dev/null || exit 1
ENTRYPOINT ["dispatch-sat"]
CMD ["serve", "8080"]
