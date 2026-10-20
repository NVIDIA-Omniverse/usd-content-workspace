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

@read_only
Feature: Read-only storage behavior FileObjectAPI

  These scenarios require the example service running read-only
  (FILESERVICE_READ_ONLY=true): IsReadOnly reports true and mutating operations are
  rejected with FAILED_PRECONDITION (gRPC) / 412 Precondition Failed (REST). Read-only
  is enforced centrally at the backend, so folder, metadata, multipart-completion and
  redirect-upload mutations are covered too, not only direct write/delete.

  Background:
    Given a new test namespace called 'read_only_test_alpha'
    And a connection to the storage service
    And an authenticated user

  Scenario Outline: IsReadOnly reports true on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And a resource address for 'nested/path/read-only-capability-<protocol>.usd'
    Then calling '<protocol>' IsReadOnly on that address returns '<return_value>' with is_read_only 'true'
    Examples:
      | protocol | return_value |
      | GRPC     | OK           |
      | REST     | 200          |

  Scenario Outline: Write is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'read-only-write-<protocol>.txt'
    And a blob of '1024' bytes
    Then calling '<protocol>' write on that address returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  Scenario Outline: Delete is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'read-only-delete-<protocol>.txt'
    When calling '<protocol>' delete on that address returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  Scenario Outline: Copy is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'read-only-copy-source-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And determining head resource identity with '<protocol>'
    And memorizing that resource identity as 'source_identity'
    Given a resource address for 'read-only-copy-destination-<protocol>.usd'
    And memorizing that resource address as 'destination_address'
    When calling '<protocol>' copy from memorized 'source_identity' to memorized 'destination_address' returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  Scenario Outline: Move is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'read-only-move-source-<protocol>.txt'
    And an object of size '1024' exists at that address and is readable
    And memorizing that resource address as 'source_file'
    And a resource address '<protocol>_read_only_move_folder' which is enumerable
    And memorizing that resource address as 'destination_folder'
    Then calling '<protocol>' move from address 'source_file' to address 'destination_folder' returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  Scenario Outline: CreateFolder is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'filefolder' API in version 'v1alpha' for feature 'filefolder'
    And the '<protocol>' service reports having native folders 'not as' 'no_empty'
    Given a resource address '<protocol>_read_only_create_folder' which is enumerable
    Then calling '<protocol>' CreateFolder on that address returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  Scenario Outline: DeleteFolder is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'filefolder' API in version 'v1alpha' for feature 'filefolder'
    And the '<protocol>' service reports having native folders 'not as' 'no_empty'
    Given a resource address '<protocol>_read_only_delete_folder' which is enumerable
    And a folder exists at that address
    Then calling '<protocol>' DeleteFolder on that address returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  Scenario Outline: UpdateMetadata is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'read-only-update-metadata-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'author' with value 'John Doe' on that address returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  Scenario Outline: DeleteMetadata is rejected on read-only storage
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'read-only-delete-metadata-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' delete metadata key 'author' on that address returns '<return_code>'
    Examples:
      | protocol | return_code         |
      | GRPC     | FAILED_PRECONDITION |
      | REST     | 412                 |

  # A redirect target may outlive the Write call which issued it. This scenario models a target
  # retained across a writable -> read-only transition and exercises the separate HTTP upload sink.
  Scenario: A redirect upload issued before read-only mode cannot create an object
    Given the service speaks 'REST'
    And the 'REST' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'read-only-preissued-redirect.bin'
    And no object exists at that address
    And a blob of '1024' bytes
    And a redirect upload target was issued before storage became read-only
    When uploading to the pre-issued redirect target returns '412'
    Then calling 'REST' stat on that address returns '404'

  # Multipart sessions can likewise survive a service restart. Prepare the session out of band,
  # as if it were created while writable, then prove both completion transports reject the commit.
  Scenario Outline: Completing a multipart upload issued before read-only mode is rejected
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'read-only-prepared-multipart-<protocol>.bin'
    And no object exists at that address
    And a blob of '1024' bytes
    And a multipart upload was prepared before storage became read-only
    When calling '<protocol>' complete on the prepared multipart upload returns '<return_code>'
    Then calling '<protocol>' stat on that address returns '<stat_return_code>'
    Examples:
      | protocol | return_code         | stat_return_code |
      | GRPC     | FAILED_PRECONDITION | NOT_FOUND       |
      | REST     | 412                 | 404             |

  Scenario Outline: UpdateMetadataBatch returns per-entry failures without changing metadata
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'read-only-update-metadata-batch-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And metadata key 'author' with value 'Before' exists at that address
    And an update metadata batch request entry for key 'author' with value 'After' on that address
    When calling '<protocol>' update metadata batch returns '<batch_return_code>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'FAILED_PRECONDITION'
    When calling '<protocol>' get metadata with keys '["author"]' on that address returns '<get_return_code>'
    Then the metadata response contains key 'author' with value 'Before'
    Examples:
      | protocol | batch_return_code | get_return_code |
      | GRPC     | OK                | OK              |
      | REST     | 200               | 200             |

  Scenario Outline: DeleteMetadataBatch returns per-entry failures without changing metadata
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'read-only-delete-metadata-batch-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And metadata key 'author' with value 'Before' exists at that address
    And a delete metadata batch request entry for key 'author' on that address
    When calling '<protocol>' delete metadata batch returns '<batch_return_code>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'FAILED_PRECONDITION'
    When calling '<protocol>' get metadata with keys '["author"]' on that address returns '<get_return_code>'
    Then the metadata response contains key 'author' with value 'Before'
    Examples:
      | protocol | batch_return_code | get_return_code |
      | GRPC     | OK                | OK              |
      | REST     | 200               | 200             |
