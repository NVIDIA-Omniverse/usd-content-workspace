# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""The ``uv run start`` entry point.

It lives outside the internal-only ``scripts/`` tree because the published
source ships this entry point and ``scripts/`` is not published.
"""

import os

from notification_common.scripts.app_scripts import start as app_start

# The REST server binds notification_common's ``rest_port`` setting, which reads
# OMNI_EVENTS_REST_PORT and defaults to 8000; this service's local default is
# 8001. The image runs ``app.main`` directly, so it binds 8000 unless the
# variable is set (the Helm chart sets it from ``rest.port``, the conformance
# compose file and CI job set 8001); this default applies only to
# ``uv run start``.
DEFAULT_REST_PORT = "8001"


def start() -> None:
    """Run the service locally, defaulting the REST port to 8001."""
    os.environ.setdefault("OMNI_EVENTS_REST_PORT", DEFAULT_REST_PORT)
    app_start()
