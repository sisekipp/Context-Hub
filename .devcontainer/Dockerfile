FROM mcr.microsoft.com/devcontainers/rust:1-1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libasound2 libatk-bridge2.0-0 libatk1.0-0 libatspi2.0-0 libcairo2 libcups2 \
        libdbus-1-3 libgbm1 libpango-1.0-0 libx11-6 libxcb1 libxcomposite1 \
        libxdamage1 libxext6 libxfixes3 libxkbcommon0 libxrandr2 \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://mise.run | MISE_INSTALL_PATH=/usr/local/bin/mise sh

ENV MISE_YES=1
ENV MISE_DATA_DIR=/opt/mise
ENV PNPM_HOME=/pnpm
ENV PNPM_STORE_DIR=/pnpm/store
ENV PATH=/pnpm:/opt/mise/shims:${PATH}

RUN mkdir -p /opt/mise /pnpm/store /usr/local/cargo/registry /workspace/node_modules /workspace/apps/web/node_modules /workspace/apps/web/.next \
    && chown -R vscode:vscode /opt/mise /pnpm /usr/local/cargo/registry /workspace/node_modules /workspace/apps/web/node_modules /workspace/apps/web/.next

WORKDIR /workspace
