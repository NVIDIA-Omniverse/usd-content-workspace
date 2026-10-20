# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

FROM --platform=linux/amd64 python:3.14-slim as builder

WORKDIR /code

# uv decides every package installed into the runtime image, so it is pinned
# to an exact release and installed from PyPI with hash checking: pip rejects
# any wheel whose sha256 is not listed (manylinux x86_64 and aarch64 wheels of
# uv 0.12.5, from PyPI's JSON API). Bump the version and both hashes together.
# It goes under its own prefix, not the site-packages the runtime stage copies,
# so uv itself does not ship in the production image; /opt/uv/bin/uv is the
# wheel's native binary and needs no PYTHONPATH.
RUN printf '%s\n' 'uv==0.12.5 --hash=sha256:3e195ccf1ed60c8bb24a6447ce306441a4181d54b602407e09bc56e963911c15 --hash=sha256:58abfb0f658b39a834307a11223bc170294ea214263b4c99ecc7663720d43544' \
        > /tmp/uv-requirements.txt && \
    pip install --no-cache-dir --require-hashes --prefix /opt/uv \
        -r /tmp/uv-requirements.txt
COPY ./pyproject.toml ./uv.lock /code/
# notification-common is locked as the sibling ../common directory, which is
# outside this folder's build context. Pass it as a named build context:
#   docker build --build-context common=../common .
COPY --from=common . /common/

# --locked, not --frozen: the export refuses a uv.lock that no longer matches
# pyproject.toml, so a dependency edit committed without its lock fails the
# image build instead of shipping the old dependency set.
RUN /opt/uv/bin/uv export --no-dev --locked --no-emit-project > requirements.txt && \
    /opt/uv/bin/uv pip install --system --compile-bytecode --no-deps packaging -r requirements.txt

################################################################################
######################### Production Distroless Stage ##########################
################################################################################

FROM nvcr.io/nvidia/distroless/python:3.14-v4.1.4

WORKDIR /code

# Copy Python packages from builder stage
COPY --from=builder /usr/local/lib/python3.14/site-packages /usr/local/lib/python3.14/site-packages

# Copy application code
COPY . /code

# Set PYTHONPATH to ensure packages are found
ENV PYTHONPATH=/usr/local/lib/python3.14/site-packages

# Run as non-root user
USER nvs

# Distroless has no shell, so we directly invoke Python
ENTRYPOINT ["python", "-um", "app.main"]
