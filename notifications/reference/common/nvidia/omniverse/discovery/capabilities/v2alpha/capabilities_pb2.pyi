# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from google.protobuf.internal import containers as _containers
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class ServiceEntry(_message.Message):
    __slots__ = ("service_name", "grpc_services", "rest_services")
    SERVICE_NAME_FIELD_NUMBER: _ClassVar[int]
    GRPC_SERVICES_FIELD_NUMBER: _ClassVar[int]
    REST_SERVICES_FIELD_NUMBER: _ClassVar[int]
    service_name: str
    grpc_services: _containers.RepeatedCompositeFieldContainer[GrpcService]
    rest_services: _containers.RepeatedCompositeFieldContainer[RestService]
    def __init__(self, service_name: _Optional[str] = ..., grpc_services: _Optional[_Iterable[_Union[GrpcService, _Mapping]]] = ..., rest_services: _Optional[_Iterable[_Union[RestService, _Mapping]]] = ...) -> None: ...

class GrpcService(_message.Message):
    __slots__ = ("endpoints",)
    ENDPOINTS_FIELD_NUMBER: _ClassVar[int]
    endpoints: _containers.RepeatedCompositeFieldContainer[GrpcEndpoint]
    def __init__(self, endpoints: _Optional[_Iterable[_Union[GrpcEndpoint, _Mapping]]] = ...) -> None: ...

class GrpcEndpoint(_message.Message):
    __slots__ = ("service_name",)
    SERVICE_NAME_FIELD_NUMBER: _ClassVar[int]
    service_name: str
    def __init__(self, service_name: _Optional[str] = ...) -> None: ...

class RestService(_message.Message):
    __slots__ = ("endpoints",)
    ENDPOINTS_FIELD_NUMBER: _ClassVar[int]
    endpoints: _containers.RepeatedCompositeFieldContainer[RestEndpoint]
    def __init__(self, endpoints: _Optional[_Iterable[_Union[RestEndpoint, _Mapping]]] = ...) -> None: ...

class RestEndpoint(_message.Message):
    __slots__ = ("name", "versions")
    NAME_FIELD_NUMBER: _ClassVar[int]
    VERSIONS_FIELD_NUMBER: _ClassVar[int]
    name: str
    versions: _containers.RepeatedCompositeFieldContainer[RestVersion]
    def __init__(self, name: _Optional[str] = ..., versions: _Optional[_Iterable[_Union[RestVersion, _Mapping]]] = ...) -> None: ...

class RestVersion(_message.Message):
    __slots__ = ("version", "url_detail")
    VERSION_FIELD_NUMBER: _ClassVar[int]
    URL_DETAIL_FIELD_NUMBER: _ClassVar[int]
    version: str
    url_detail: RestVersionDetail
    def __init__(self, version: _Optional[str] = ..., url_detail: _Optional[_Union[RestVersionDetail, _Mapping]] = ...) -> None: ...

class RestVersionDetail(_message.Message):
    __slots__ = ("path",)
    PATH_FIELD_NUMBER: _ClassVar[int]
    path: str
    def __init__(self, path: _Optional[str] = ...) -> None: ...
