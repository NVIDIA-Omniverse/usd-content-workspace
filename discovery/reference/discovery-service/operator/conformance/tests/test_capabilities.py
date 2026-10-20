# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

from concurrent.futures import ThreadPoolExecutor
import time

import grpc
import pytest

from discovery_conformance.capabilities import check_capabilities
from discovery_conformance.generated.nvidia.omniverse.discovery.capabilities.v2beta import (
    capabilities_pb2 as pb,
    capabilities_service_pb2 as service_pb,
    capabilities_service_pb2_grpc as service_grpc,
)


@pytest.fixture
def backend():
    servers = []

    def start(handler):
        class Servicer(service_grpc.CapabilitiesServiceServicer):
            def ListServices(self, request, context):
                return handler(request, context)

        server = grpc.server(ThreadPoolExecutor(max_workers=2))
        service_grpc.add_CapabilitiesServiceServicer_to_server(Servicer(), server)
        port = server.add_insecure_port('127.0.0.1:0')
        server.start()
        servers.append(server)
        return f'127.0.0.1:{port}'

    yield start
    for server in servers:
        server.stop(0).wait()


def test_complete_pages_with_empty_intermediate_and_opaque_token(backend):
    calls = []
    def handler(request, context):
        calls.append((request.page_token, request.page_size, dict(context.invocation_metadata())))
        return {
            '': service_pb.ListServicesResponse(services=[pb.ServiceEntry()], next_page_token='opaque /+= secret'),
            'opaque /+= secret': service_pb.ListServicesResponse(next_page_token='final'),
            'final': service_pb.ListServicesResponse(services=[pb.ServiceEntry(service_name='x')]),
        }[request.page_token]
    results = check_capabilities(backend(handler), plaintext=True, page_size=2)
    assert [r['status'] for r in results] == ['PASS', 'PASS']
    assert results[1]['message'] == 'Complete inventory: 2 services.'
    assert [c[0] for c in calls] == ['', 'opaque /+= secret', 'final']
    assert all(c[1] == 2 and 'authorization' not in c[2] for c in calls)


def test_empty_inventory_and_no_implicit_auth(backend):
    def handler(request, context):
        assert 'authorization' not in dict(context.invocation_metadata())
        return service_pb.ListServicesResponse()
    results = check_capabilities(backend(handler), plaintext=True)
    assert results[1]['status'] == 'PASS'
    assert '0 services' in results[1]['message']


def test_repeated_token_is_redacted_failure(backend):
    target = backend(lambda r, c: service_pb.ListServicesResponse(next_page_token='secret-token'))
    results = check_capabilities(target, plaintext=True)
    assert [r['status'] for r in results] == ['PASS', 'FAIL']
    assert 'secret-token' not in str(results)


def test_page_bound_is_inconclusive(backend):
    target = backend(lambda r, c: service_pb.ListServicesResponse(next_page_token=r.page_token + 'x'))
    results = check_capabilities(target, plaintext=True, max_pages=2)
    assert [r['status'] for r in results] == ['PASS', 'INCONCLUSIVE']


@pytest.mark.parametrize('code,status', [(grpc.StatusCode.UNAVAILABLE, 'INCONCLUSIVE'),
                                        (grpc.StatusCode.UNIMPLEMENTED, 'FAIL'),
                                        (grpc.StatusCode.DEADLINE_EXCEEDED, 'INCONCLUSIVE')])
def test_rpc_failure_redaction(backend, code, status):
    def handler(request, context):
        context.abort(code, 'private backend response and credentials')
    results = check_capabilities(backend(handler), plaintext=True)
    assert all(r['status'] == status for r in results)
    assert 'private' not in str(results)


@pytest.mark.parametrize('options', [{'target': 'http://localhost:1'},
                                   {'target': 'localhost:1', 'ca_bundle': False},
                                   {'target': 'localhost:1', 'max_pages': 0}])
def test_invalid_configuration(options):
    assert all(r['status'] == 'INCONCLUSIVE' for r in check_capabilities(**options))


def test_deadline_is_enforced(backend):
    def handler(request, context):
        time.sleep(0.1)
        return service_pb.ListServicesResponse()
    results = check_capabilities(backend(handler), plaintext=True, timeout=0.01)
    assert results[0]['status'] == 'INCONCLUSIVE'
    assert 'DEADLINE_EXCEEDED' in results[0]['message']


def test_failure_after_first_page_preserves_wire_pass(backend):
    def handler(request, context):
        if request.page_token:
            context.abort(grpc.StatusCode.UNAVAILABLE, 'secret')
        return service_pb.ListServicesResponse(next_page_token='next')
    results = check_capabilities(backend(handler), plaintext=True)
    assert [r['status'] for r in results] == ['PASS', 'INCONCLUSIVE']


def test_tls_is_default_and_rpc_options_are_explicit(monkeypatch):
    from discovery_conformance import capabilities
    calls = {}

    class Channel:
        def __enter__(self):
            return self

        def __exit__(self, *args):
            pass

    def secure_channel(authority, credentials):
        calls['authority'] = authority
        calls['credentials'] = credentials
        return Channel()

    class Stub:
        def __init__(self, channel):
            pass

        def ListServices(self, request, **kwargs):
            calls['options'] = kwargs
            return service_pb.ListServicesResponse()

    monkeypatch.setattr(capabilities.grpc, 'secure_channel', secure_channel)
    monkeypatch.setattr(capabilities.capabilities_service_pb2_grpc, 'CapabilitiesServiceStub', Stub)
    results = check_capabilities('https://backend.example:443', timeout=3, token='explicit')
    assert all(r['status'] == 'PASS' for r in results)
    assert calls['authority'] == 'backend.example:443'
    assert calls['credentials'] is not None
    assert calls['options'] == {'timeout': 3, 'metadata': (('authorization', 'Bearer explicit'),)}


def test_plaintext_credentials_are_rejected_before_channel_creation(monkeypatch):
    def unexpected_channel(*args, **kwargs):
        pytest.fail('Credentials must not be sent over plaintext')
    monkeypatch.setattr(grpc, 'insecure_channel', unexpected_channel)
    results = check_capabilities('localhost:1', plaintext=True, token='secret')
    assert all(result['status'] == 'INCONCLUSIVE' for result in results)
    assert 'secret' not in str(results)
