#!/usr/bin/env python3
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

"""
End-to-End tests for Thumbnail Service
Tests HTTP API, gRPC API, and task queue integration
"""

import json
import requests
import subprocess
import time
# import concurrent.futures  # Commented out for now - used for load testing
from dataclasses import dataclass
from typing import Dict, List, Optional
import sys
import os

@dataclass
class TestConfig:
    test_resource_url: str = "https://i.pinimg.com/736x/b0/ef/ba/b0efba4b7e0ff7c606ac4ab13973105a.jpg"
    http_port: int = 3001
    grpc_port: int = 50052
    task_queue_port: int = 8001

@dataclass
class TestResult:
    name: str
    success: bool
    message: str
    data: Optional[Dict] = None

class ThumbnailE2ETest:
    def __init__(self, config: TestConfig):
        self.config = config
        self.port_forward_processes = []
        self.results: List[TestResult] = []

    def setup_port_forwarding(self) -> bool:
        """Setup port forwarding for all services"""
        print("🔧 Setting up port forwarding...")

        services = [
            ("thumbnail-service", self.config.http_port, 3000),
            ("thumbnail-service", self.config.grpc_port, 50051),
            ("thumbnail-task-queue-dev-api", self.config.task_queue_port, 8000)
        ]

        for service_name, local_port, remote_port in services:
            try:
                cmd = f"kubectl port-forward service/{service_name} {local_port}:{remote_port}"
                process = subprocess.Popen(cmd.split(), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                self.port_forward_processes.append(process)
                print(f"  Started port-forward for {service_name}: {local_port} -> {remote_port}")
            except Exception as e:
                print(f"❌ Failed to setup port-forward for {service_name}: {e}")
                return False

        # Wait for port forwards to be ready
        print("⏳ Waiting for port forwards to be ready...")
        time.sleep(5)

        # Verify connections
        for service_name, local_port, _ in services:
            if not self._check_port(local_port):
                print(f"❌ Port forward for {service_name} on port {local_port} is not ready")
                return False

        print("✅ Port forwarding setup complete")
        return True

    def _check_port(self, port: int) -> bool:
        """Check if a port is accessible"""
        try:
            import socket
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.settimeout(1)
            result = sock.connect_ex(('localhost', port))
            sock.close()
            return result == 0
        except:
            return False

    def cleanup(self):
        """Clean up port forwarding processes"""
        print("🧹 Cleaning up port forwards...")
        for process in self.port_forward_processes:
            try:
                process.terminate()
                process.wait(timeout=5)
            except:
                try:
                    process.kill()
                except:
                    pass

    def test_http_create_thumbnails(self) -> TestResult:
        """Test HTTP API thumbnail creation"""
        print("🧪 Testing HTTP Create Thumbnails...")

        url = f"http://localhost:{self.config.http_port}/thumbnails"
        payload = {
            "resource_identity": self.config.test_resource_url,
            "specification": []
        }
        headers = {
            "Content-Type": "application/json",
            "Authorization": "Bearer test-token"
        }

        try:
            response = requests.post(url, json=payload, headers=headers, timeout=10)

            if response.status_code == 200:
                data = response.json()
                if 'thumbnails' in data and len(data['thumbnails']) > 0:
                    thumbnail = data['thumbnails'][0]
                    task_id = thumbnail.get('task_id')
                    status = thumbnail.get('status')

                    if task_id and status:
                        return TestResult(
                            name="HTTP Create Thumbnails",
                            success=True,
                            message=f"Success: task_id={task_id}, status={status}",
                            data=data
                        )
                    else:
                        return TestResult(
                            name="HTTP Create Thumbnails",
                            success=False,
                            message="Response missing task_id or status",
                            data=data
                        )
                else:
                    return TestResult(
                        name="HTTP Create Thumbnails",
                        success=False,
                        message="Response missing thumbnails array",
                        data=data
                    )
            else:
                return TestResult(
                    name="HTTP Create Thumbnails",
                    success=False,
                    message=f"HTTP {response.status_code}: {response.text}",
                    data={"status_code": response.status_code}
                )

        except Exception as e:
            return TestResult(
                name="HTTP Create Thumbnails",
                success=False,
                message=f"Request failed: {str(e)}"
            )

    def test_task_queue_health(self) -> TestResult:
        """Test task queue health endpoint"""
        print("🧪 Testing Task Queue Health...")

        url = f"http://localhost:{self.config.task_queue_port}/health"

        try:
            response = requests.get(url, timeout=5)

            if response.status_code == 200:
                return TestResult(
                    name="Task Queue Health",
                    success=True,
                    message="Task queue is healthy",
                    data={"response": response.text}
                )
            else:
                return TestResult(
                    name="Task Queue Health",
                    success=False,
                    message=f"Health check failed: HTTP {response.status_code}",
                    data={"status_code": response.status_code}
                )

        except Exception as e:
            return TestResult(
                name="Task Queue Health",
                success=False,
                message=f"Health check failed: {str(e)}"
            )

    # def test_concurrent_requests(self) -> TestResult:
    #     """Test concurrent HTTP requests"""
    #     print("🧪 Testing Concurrent Requests...")
    #
    #     def make_request(request_id: int) -> bool:
    #         url = f"http://localhost:{self.config.http_port}/thumbnails"
    #         payload = {
    #             "resource_identity": self.config.test_resource_url,
    #             "specification": []
    #         }
    #         headers = {
    #             "Content-Type": "application/json",
    #             "Authorization": f"Bearer test-token-{request_id}"
    #         }
    #
    #         try:
    #             response = requests.post(url, json=payload, headers=headers, timeout=10)
    #             if response.status_code == 200:
    #                 data = response.json()
    #                 return 'thumbnails' in data and len(data['thumbnails']) > 0
    #             return False
    #         except:
    #             return False
    #
    #     # Run 5 concurrent requests
    #     with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
    #         futures = [executor.submit(make_request, i) for i in range(5)]
    #         results = [future.result() for future in concurrent.futures.as_completed(futures)]
    #
    #     successful_requests = sum(results)
    #
    #     if successful_requests >= 4:
    #         return TestResult(
    #             name="Concurrent Requests",
    #             success=True,
    #             message=f"Success: {successful_requests}/5 requests successful",
    #             data={"successful": successful_requests, "total": 5}
    #         )
    #     else:
    #         return TestResult(
    #             name="Concurrent Requests",
    #             success=False,
    #             message=f"Failed: Only {successful_requests}/5 requests successful",
    #             data={"successful": successful_requests, "total": 5}
    #         )

    def test_grpc_api(self) -> TestResult:
        """Test gRPC API (requires grpcurl)"""
        print("🧪 Testing gRPC API...")

        # Check if grpcurl is available
        try:
            subprocess.run(["grpcurl", "--version"], capture_output=True, check=True)
        except (subprocess.CalledProcessError, FileNotFoundError):
            return TestResult(
                name="gRPC API",
                success=False,
                message="grpcurl not available - install with: go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest"
            )

        # Test gRPC service
        try:
            # List services first
            list_cmd = ["grpcurl", "-plaintext", f"localhost:{self.config.grpc_port}", "list"]
            list_result = subprocess.run(list_cmd, capture_output=True, text=True, timeout=10)

            if list_result.returncode != 0:
                return TestResult(
                    name="gRPC API",
                    success=False,
                    message=f"gRPC server not accessible: {list_result.stderr}"
                )

            # Test CreateThumbnails
            grpc_payload = {
                "resource_identity": self.config.test_resource_url,
                "specification": [{
                    "image_to_image": {
                        "processor_identity": 0,  # PROCESSOR_IDENTITY_UNSPECIFIED = 0, IMAGE_TO_IMAGE = 1
                        "encoding_format": 1,     # JPEG = 1
                        "width": 512,
                        "height": 512,
                        "transform_type": 1,      # FIT = 1
                        "crop_direction": 1       # CENTER = 1
                    }
                }]
            }

            grpc_cmd = [
                "grpcurl", "-plaintext",
                "-d", json.dumps(grpc_payload),
                f"localhost:{self.config.grpc_port}",
                "nvidia.omniverse.thumbnail.v1alpha.ThumbnailService/CreateThumbnails"
            ]

            grpc_result = subprocess.run(grpc_cmd, capture_output=True, text=True, timeout=10)

            if grpc_result.returncode == 0:
                try:
                    response_data = json.loads(grpc_result.stdout)
                    if 'thumbnails' in response_data and len(response_data['thumbnails']) > 0:
                        # Handle both camelCase (taskId) and snake_case (task_id) formats
                        task_id = response_data['thumbnails'][0].get('taskId') or response_data['thumbnails'][0].get('task_id')
                        status = response_data['thumbnails'][0].get('status')
                        return TestResult(
                            name="gRPC API",
                            success=True,
                            message=f"Success: task_id={task_id}, status={status}",
                            data=response_data
                        )
                    else:
                        return TestResult(
                            name="gRPC API",
                            success=False,
                            message="gRPC response missing thumbnails",
                            data={"response": grpc_result.stdout}
                        )
                except json.JSONDecodeError:
                    return TestResult(
                        name="gRPC API",
                        success=False,
                        message="Invalid JSON response from gRPC",
                        data={"response": grpc_result.stdout}
                    )
            else:
                return TestResult(
                    name="gRPC API",
                    success=False,
                    message=f"gRPC call failed: {grpc_result.stderr}",
                    data={"stderr": grpc_result.stderr}
                )

        except Exception as e:
            return TestResult(
                name="gRPC API",
                success=False,
                message=f"gRPC test failed: {str(e)}"
            )

    def run_all_tests(self) -> bool:
        """Run all e2e tests"""
        print("🚀 Starting Thumbnail Service E2E Tests")
        print(f"📋 Test Resource: {self.config.test_resource_url}")
        print()

        # Check kubectl context
        try:
            context_result = subprocess.run(
                ["kubectl", "config", "current-context"],
                capture_output=True, text=True, check=True
            )
            current_context = context_result.stdout.strip()
            print(f"📍 Current kubectl context: {current_context}")

            expected_context = os.getenv("KUBE_CONTEXT", "").strip()
            if expected_context and current_context != expected_context:
                print(f"⚠️  Not using {expected_context} context. Switching...")
                subprocess.run(
                    ["kubectl", "config", "use-context", expected_context],
                    check=True
                )
            elif not expected_context:
                print("ℹ️  KUBE_CONTEXT not set; using current kubectl context")
        except subprocess.CalledProcessError:
            print("❌ Failed to check/set kubectl context")
            return False

        # Setup port forwarding
        if not self.setup_port_forwarding():
            return False

        try:
            # Run tests
            tests = [
                self.test_http_create_thumbnails,
                self.test_task_queue_health,
                self.test_grpc_api,
            ]

            print("\n🧪 Running Tests:")
            print("=" * 50)

            for test_func in tests:
                result = test_func()
                self.results.append(result)

                status = "✅ PASS" if result.success else "❌ FAIL"
                print(f"{status} {result.name}: {result.message}")

                if result.data and len(str(result.data)) < 200:
                    print(f"   Data: {result.data}")
                print()

            # Summary
            passed = sum(1 for r in self.results if r.success)
            total = len(self.results)

            print("=" * 50)
            print(f"📊 Test Summary: {passed}/{total} tests passed")

            if passed == total:
                print("🎉 All tests passed!")
                return True
            else:
                print("❌ Some tests failed")
                return False

        finally:
            self.cleanup()

def main():
    """Main entry point"""
    config = TestConfig()
    tester = ThumbnailE2ETest(config)

    try:
        success = tester.run_all_tests()
        sys.exit(0 if success else 1)
    except KeyboardInterrupt:
        print("\n⚠️  Tests interrupted by user")
        tester.cleanup()
        sys.exit(1)
    except Exception as e:
        print(f"❌ Test execution failed: {e}")
        tester.cleanup()
        sys.exit(1)

if __name__ == "__main__":
    main()
