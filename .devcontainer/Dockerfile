FROM mcr.microsoft.com/devcontainers/rust:1-1-bookworm

RUN curl -fsSL https://mise.run | MISE_INSTALL_PATH=/usr/local/bin/mise sh

ENV MISE_YES=1
ENV MISE_DATA_DIR=/opt/mise
ENV PNPM_HOME=/pnpm
ENV PNPM_STORE_DIR=/pnpm/store
ENV PATH=/pnpm:/opt/mise/shims:${PATH}

RUN mkdir -p /opt/mise /pnpm/store /usr/local/cargo/registry /workspace/node_modules /workspace/apps/web/node_modules /workspace/apps/web/.next \
    && chown -R vscode:vscode /opt/mise /pnpm /usr/local/cargo/registry /workspace/node_modules /workspace/apps/web/node_modules /workspace/apps/web/.next

WORKDIR /workspace
