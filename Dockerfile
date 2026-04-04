FROM ghcr.io/anthropics/claude-code:latest

# uv — manages Python version + deps (matches project conventions in CLAUDE.md)
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:$PATH"

# Rust
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
ENV PATH="/root/.cargo/bin:$PATH"

WORKDIR /workspace
