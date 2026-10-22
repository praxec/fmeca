# fmeca — MCP server (stdio). Multi-stage build → slim runtime.
FROM rust:1.99.1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release --bin fmeca-mcp && cp target/release/fmeca-mcp /fmeca-mcp

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 10001 app
COPY --from=build /fmeca-mcp /usr/local/bin/fmeca-mcp
USER app
LABEL io.modelcontextprotocol.server.name="io.github.praxec/fmeca"
# The gateway spawns this container with `docker run -i` and speaks MCP over stdio.
ENTRYPOINT ["fmeca-mcp"]
