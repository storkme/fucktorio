FROM node:24

# Install system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
  less git procps sudo fzf zsh gh jq nano vim \
  && apt-get clean && rm -rf /var/lib/apt/lists/*

# Install Claude Code via npm (official method)
RUN npm install -g @anthropic-ai/claude-code@latest

# Give node user sudo for workspace setup
RUN echo "node ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/node

# Switch to non-root user (required by --dangerously-skip-permissions)
USER node

# uv — manages Python version + deps (matches project conventions in CLAUDE.md)
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/home/node/.local/bin:$PATH"

# Rust (needed for PyO3 A* pathfinder)
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/home/node/.cargo/bin:$PATH"

# wasm-pack (for WASM bundle)
RUN curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

WORKDIR /workspace

COPY --chown=node:node docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["bash"]
