# syntax=docker/dockerfile:1
# check=skip=FromPlatformFlagConstDisallowed

# The upstream compiler image is AMD64-only. Its musl cross toolchains produce
# both target architectures, including when Docker runs on an ARM64 host.
FROM --platform=linux/amd64 ghcr.io/motis-project/docker-cpp-build@sha256:b2f2a0256c2c67c63e011a4f778e236a0f86cded0c6db99a3e84f704ebab250f AS build
WORKDIR /src
RUN git clone --depth 1 --branch v2.11.3 https://github.com/motis-project/motis.git . \
    && test "$(git rev-parse HEAD)" = b228a4519d196d9dd01b5ce80be46e642abc953e
# Cache upstream dependencies independently of our patch and compiler output.
RUN /opt/pkg -l \
    && test "$(git -C deps/osr rev-parse HEAD)" = a7b2ec2728544304ef1d8397b3042abc8d10f7e7
COPY scripts/motis-strict-streets.patch /tmp/strict-streets.patch
COPY scripts/motis-routing-performance.patch /tmp/routing-performance.patch
ARG TARGETARCH
RUN cmake --preset "linux-${TARGETARCH}-release" -B build/docker \
    && test "$(git -C deps/osr rev-parse HEAD)" = a7b2ec2728544304ef1d8397b3042abc8d10f7e7 \
    && git -C deps/osr apply --check /tmp/strict-streets.patch \
    && git -C deps/osr apply /tmp/strict-streets.patch \
    && git -C deps/osr apply --check /tmp/routing-performance.patch \
    && git -C deps/osr apply /tmp/routing-performance.patch
ARG MOTIS_BUILD_JOBS=2
RUN cmake --build build/docker --target motis --parallel "$MOTIS_BUILD_JOBS"

FROM alpine:3.22 AS runtime
RUN apk add --no-cache ca-certificates curl \
    && addgroup -g 10001 motis \
    && adduser -D -u 10001 -G motis motis \
    && mkdir -p /data \
    && chown motis:motis /data
COPY --from=build /src/build/docker/motis /usr/local/bin/motis
COPY --from=build /src/LICENSE /usr/share/licenses/motis/LICENSE
COPY --from=build /src/deps/osr/LICENSE /usr/share/licenses/osr/LICENSE
WORKDIR /data
USER motis
EXPOSE 8081
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=5 \
    CMD curl --fail --silent http://127.0.0.1:8081/api/v1/health > /dev/null || exit 1
ENTRYPOINT ["motis"]
CMD ["server", "--data", "/data"]
