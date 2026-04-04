# External tools (not shipped in the repo)

This directory holds third-party tools used for **offline** workflows.
None of these are runtime dependencies of `fucktorio`.

## factorio-sat

SAT-based Factorio balancer generator from
https://github.com/R-O-C-K-E-T/Factorio-SAT

Used to pre-generate N-to-M belt balancer templates that are baked into
`src/bus/balancer_library.py`.

### Setup

```bash
git clone https://github.com/R-O-C-K-E-T/Factorio-SAT.git external/factorio-sat
cd external/factorio-sat
uv venv .venv --python 3.12
.venv/bin/python -m ensurepip --upgrade
.venv/bin/python -m pip install --editable .
```

### Regenerating the balancer library

```bash
# from repo root
uv run python scripts/generate_balancer_library.py
```

This rewrites `src/bus/balancer_library.py` with fresh templates.
Commit the generated file.
