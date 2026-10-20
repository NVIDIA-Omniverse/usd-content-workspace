# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from enum import Enum
from typing import Any, Dict, List

from pydantic import BaseModel, Field, ConfigDict

from notification_common.errors.errors import AuthenticationError, AuthorizationError


class PermissionClientError(Exception):
    """Base class for all permission client related errors."""


class PermissionInvocationError(PermissionClientError):
    """Raised when the permission service invocation fails."""


class PermissionResponseError(PermissionClientError):
    """Raised when the permission service response is invalid."""


class PermissionAuthenticationError(AuthenticationError):
    """Raised when the permission service returns a 401 error."""


class PermissionAuthorizationError(AuthorizationError):
    """Raised when the permission service returns a 403 error."""


class AuthorizationDecision(str, Enum):
    DENY = "deny"
    ALLOW = "allow"
    SKIP = "skip"


class Action(BaseModel):
    """An action to be authorized."""

    service: str
    name: str


class Resource(BaseModel):
    """A resource to be authorized against."""

    id: str
    type: str
    data: Dict[str, Any] | None = None

    # make model hashable so we use it in a function signature annotated with @lru_cache
    model_config = ConfigDict(frozen=True)


class Principal(BaseModel):
    """The principal requesting authorization."""

    sub: str
    info: Dict[str, Any] | None = None


class AuthorizationRequestBase(BaseModel):
    token: str


class AuthorizationRequest(AuthorizationRequestBase):
    """A request to authorize a single action."""

    action: Action
    principal: Principal | None = None
    resource: Resource | None = None
    context: Dict[str, Any] | None = None


class AuthorizationResponse(BaseModel):
    """Response from an authorization request."""

    decision: AuthorizationDecision
    reason: str | None = None


class AuthorizationBatch(BaseModel):
    """A batch of actions to authorize."""

    principal: Principal | None = None
    actions: List[Action]
    resource: Resource | None = None
    context: Dict[str, Any] | None = None


class AuthorizationBatchCondition(str, Enum):
    """Condition for batch authorization."""

    AND = "and"
    OR = "or"


class AuthorizationBatchRequest(AuthorizationRequestBase):
    """Request to authorize multiple actions."""

    batches: List[AuthorizationBatch]
    condition: AuthorizationBatchCondition = Field(
        default=AuthorizationBatchCondition.AND
    )


class AuthorizationBatchResponse(BaseModel):
    """Response from a batch authorization request."""

    summary: AuthorizationResponse | None = None
    decisions: List[Dict[str, AuthorizationResponse]]
