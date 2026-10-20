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

Feature: Metadata API

  Background:
    Given a new test namespace called 'metadata_test'
    And a connection to the storage service
    And an authenticated user

  Scenario Outline: Get metadata from non-existing resource returns appropriate status
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'missing-<protocol>.usd'
    And no object exists at that address
    When calling '<protocol>' get metadata with keys '["author"]' on that address returns '<return_value>'
    Examples:
      | protocol | return_value     |
      | GRPC     | OK               |
      | REST     | 200              |

  Scenario Outline: Get metadata from existing resource with no metadata returns empty result
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'empty-metadata-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' get metadata with keys '["author"]' on that address returns '<return_value>'
    Then the metadata response contains '0' entries
    Examples:
      | protocol | return_value |
      | GRPC     | OK           |
      | REST     | 200          |

  Scenario Outline: Update metadata on existing resource succeeds and returns ETag
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'update-metadata-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'author' with value 'John Doe' on that address returns '<return_value>'
    Then the metadata response contains an ETag
    Examples:
      | protocol | return_value |
      | GRPC     | OK           |
      | REST     | 201          |

  Scenario Outline: Update metadata on non-existing resource returns appropriate status
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'missing-update-<protocol>.usd'
    And no object exists at that address
    When calling '<protocol>' update metadata key 'author' with value 'John Doe' on that address returns '<return_value>'
    Examples:
      | protocol | return_value     |
      | GRPC     | OK               |
      | REST     | 201              |

  Scenario Outline: Get metadata after update returns the correct value
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'get-after-update-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'author' with value 'Jane Smith' on that address returns '<update_return_value>'
    When calling '<protocol>' get metadata with keys '["author"]' on that address returns '<get_return_value>'
    Then the metadata response contains '1' entries
    And the metadata response contains key 'author' with value 'Jane Smith'
    Examples:
      | protocol | update_return_value | get_return_value |
      | GRPC     | OK                  | OK               |
      | REST     | 201                 | 200              |

  Scenario Outline: Get all metadata using an empty list returns all entries
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'all-keys-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'author' with value 'John Doe' on that address returns '<update_return_value>'
    And calling '<protocol>' update metadata key 'description' with value 'Test file' on that address returns '<update_return_value>'
    When calling '<protocol>' get metadata with keys '[]' on that address returns '<get_return_value>'
    Then the metadata response contains '2' entries
    And the metadata response contains key 'author' with value 'John Doe'
    And the metadata response contains key 'description' with value 'Test file'
    Examples:
      | protocol | update_return_value | get_return_value |
      | GRPC     | OK                  | OK               |
      | REST     | 201                 | 200              |

  Scenario Outline: Delete existing metadata succeeds
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'delete-existing-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'temporary' with value 'delete me' on that address returns '<update_return_value>'
    When calling '<protocol>' delete metadata key 'temporary' on that address returns '<delete_return_value>'
    When calling '<protocol>' get metadata with keys '["temporary"]' on that address returns '<get_return_value>'
    Then the metadata response contains '0' entries
    Examples:
      | protocol | update_return_value | delete_return_value | get_return_value |
      | GRPC     | OK                  | OK                  | OK               |
      | REST     | 201                 | 204                 | 200              |

  Scenario Outline: Delete non-existing metadata succeeds silently
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'delete-missing-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' delete metadata key 'nonexistent' on that address returns '<delete_return_value>'
    Examples:
      | protocol | delete_return_value |
      | GRPC     | OK                  |
      | REST     | 204                 |

  Scenario Outline: Delete metadata from non-existing resource returns appropriate status
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'delete-missing-resource-<protocol>.usd'
    And no object exists at that address
    When calling '<protocol>' delete metadata key 'any' on that address returns '<delete_return_value>'
    Examples:
      | protocol | delete_return_value  |
      | GRPC     | OK                   |
      | REST     | 204                  |

  Scenario Outline: Get specific metadata keys returns only requested keys
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'specific-keys-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'author' with value 'Alice' on that address returns '<update_return_value>'
    And calling '<protocol>' update metadata key 'editor' with value 'Bob' on that address returns '<update_return_value>'
    And calling '<protocol>' update metadata key 'reviewer' with value 'Charlie' on that address returns '<update_return_value>'
    When calling '<protocol>' get metadata with keys '["author", "reviewer"]' on that address returns '<get_return_value>'
    Then the metadata response contains '2' entries
    And the metadata response contains key 'author' with value 'Alice'
    And the metadata response contains key 'reviewer' with value 'Charlie'
    Examples:
      | protocol | update_return_value | get_return_value |
      | GRPC     | OK                  | OK               |
      | REST     | 201                 | 200              |

  Scenario Outline: Update metadata with different data types
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'data-types-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'string_field' with value 'text value' on that address returns '<update_return_value>'
    And calling '<protocol>' update metadata key 'number_field' with 'numeric' value '42' on that address returns '<update_return_value>'
    And calling '<protocol>' update metadata key 'boolean_field' with 'boolean' value 'true' on that address returns '<update_return_value>'
    When calling '<protocol>' get metadata with keys '["string_field", "number_field", "boolean_field"]' on that address returns '<get_return_value>'
    Then the metadata response contains '3' entries
    And the metadata response contains key 'string_field' with value 'text value'
    And the metadata response contains key 'number_field' with numeric value '42'
    And the metadata response contains key 'boolean_field' with boolean value 'true'
    Examples:
      | protocol | update_return_value | get_return_value |
      | GRPC     | OK                  | OK               |
      | REST     | 201                 | 200              |

  Scenario Outline: Metadata operations with invalid resource address return appropriate status
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given an invalid resource address
    When calling '<protocol>' get metadata with keys '["any"]' on that address returns '<get_return_value>'
    And calling '<protocol>' update metadata key 'any' with value 'any' on that address returns '<update_return_value>'
    And calling '<protocol>' delete metadata key 'any' on that address returns '<delete_return_value>'
    Examples:
      | protocol | get_return_value     | update_return_value  | delete_return_value  |
      | GRPC     | INVALID_ARGUMENT     | INVALID_ARGUMENT     | INVALID_ARGUMENT     |
      | REST     | 400                  | 400                  | 400                  |

  Scenario Outline: Conditional update metadata with correct ETag succeeds
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'conditional-update-success-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'author' with value 'Initial Author' on that address returns '<update_return_value>'
    And we memorize the last metadata response as 'initial_update'
    When calling '<protocol>' update metadata key 'author' with value 'Updated Author' using ETag from 'initial_update' on that address returns '<update_return_value>'
    When calling '<protocol>' get metadata with keys '["author"]' on that address returns '<get_return_value>'
    Then the metadata response contains '1' entries
    And the metadata response contains key 'author' with value 'Updated Author'
    Examples:
      | protocol | update_return_value | get_return_value |
      | GRPC     | OK                  | OK               |
      | REST     | 201                 | 200              |

  Scenario Outline: Conditional update metadata with incorrect ETag fails
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'conditional-update-failure-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'author' with value 'Initial Author' on that address returns '<update_return_value>'
    And we memorize the last metadata response as 'initial_update'
    When calling '<protocol>' update metadata key 'author' with value 'Intermediate Author' on that address returns '<update_return_value>'
    When calling '<protocol>' update metadata key 'author' with value 'Final Author' using ETag from 'initial_update' on that address returns '<failed_return_value>'
    When calling '<protocol>' get metadata with keys '["author"]' on that address returns '<get_return_value>'
    Then the metadata response contains '1' entries
    And the metadata response contains key 'author' with value 'Intermediate Author'
    Examples:
      | protocol | update_return_value | failed_return_value  | get_return_value |
      | GRPC     | OK                  | FAILED_PRECONDITION  | OK               |
      | REST     | 201                 | 412                  | 200              |

  Scenario Outline: Conditional delete metadata with correct ETag succeeds
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'conditional-delete-success-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'temporary' with value 'delete me' on that address returns '<update_return_value>'
    And we memorize the last metadata response as 'initial_update'
    When calling '<protocol>' delete metadata key 'temporary' using ETag from 'initial_update' on that address returns '<delete_return_value>'
    When calling '<protocol>' get metadata with keys '["temporary"]' on that address returns '<get_return_value>'
    Then the metadata response contains '0' entries
    Examples:
      | protocol | update_return_value | delete_return_value | get_return_value |
      | GRPC     | OK                  | OK                  | OK               |
      | REST     | 201                 | 204                 | 200              |

  Scenario Outline: Conditional delete metadata with incorrect ETag fails
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'conditional-delete-failure-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata key 'temporary' with value 'delete me' on that address returns '<update_return_value>'
    And we memorize the last metadata response as 'initial_update'
    When calling '<protocol>' update metadata key 'temporary' with value 'updated value' on that address returns '<update_return_value>'
    When calling '<protocol>' delete metadata key 'temporary' using ETag from 'initial_update' on that address returns '<failed_return_value>'
    When calling '<protocol>' get metadata with keys '["temporary"]' on that address returns '<get_return_value>'
    Then the metadata response contains '1' entries
    And the metadata response contains key 'temporary' with value 'updated value'
    Examples:
      | protocol | update_return_value | failed_return_value  | get_return_value |
      | GRPC     | OK                  | FAILED_PRECONDITION  | OK               |
      | REST     | 201                 | 412                  | 200              |

  Scenario Outline: Update metadata on a resource identity works as well, we do not need to use resource addresses
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And a resource address for 'metadata-for-identity-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then calling '<protocol>' update metadata key 'key' with value 'value' on the stat result returns '<metadata_return_value>'
    Examples:
      | protocol | stat_return_value  | metadata_return_value |
      | GRPC     | OK                 | OK                    |
      | REST     | 204                | 201                   |

  Scenario Outline: GetMetadataBatch fetches selected metadata values.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'get-batch-selected-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And an update metadata batch request entry for key 'reviewer' with value 'Charlie' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a get metadata batch request entry for key 'author' on that address
    And a get metadata batch request entry for key 'reviewer' on that address
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with value 'Alice'
    And the metadata batch entry at request index '1' succeeded with value 'Charlie'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: GetMetadataBatch fetches duplicate metadata values.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'get-batch-duplicate-first-<protocol>.usd' as 'first'
    And an object of size '1024' exists at the 'first' address and is readable
    And a resource address for 'get-batch-duplicate-second-<protocol>.usd' as 'second'
    And an object of size '1024' exists at the 'second' address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on the 'first' address
    And an update metadata batch request entry for key 'author' with value 'Alice' on the 'second' address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a get metadata batch request entry for key 'author' on the 'first' address
    And a get metadata batch request entry for key 'author' on the 'second' address
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with value 'Alice'
    And the metadata batch entry at request index '1' succeeded with value 'Alice'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: GetMetadataBatch reports per-entry failures without aborting the rest of the batch.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'get-batch-per-entry-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a get metadata batch request entry for key 'author' on that address
    And a get metadata batch request entry for key 'missing' on that address
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with value 'Alice'
    And the metadata batch entry at request index '1' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: GetMetadataBatch resolves entries addressed by resource identity.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And a resource address for 'get-batch-identity-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And determining head resource identity with '<protocol>'
    And an update metadata batch request entry for key 'author' with value 'Alice' using the head resource identity
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a get metadata batch request entry for key 'author' using the head resource identity
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' succeeded with value 'Alice'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: GetMetadataBatch rejects a plain address sent through the resource identity arm.
    # The resource_identity arm requires an encoded resource identity. A plain
    # address (e.g. a folder URI, which has no identity) must produce a per-entry
    # INVALID_ARGUMENT instead of being resolved as an address. Guards clients
    # that split folder-list metadata fetches between the batch identity arm
    # (files) and per-child lookups (folders): routing a folder URI through the
    # identity arm must fail loudly rather than appear to work locally.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'get-batch-identity-folder-<protocol>'
    And a folder exists at that address
    And a get metadata batch request entry for key 'author' with that address in the resource identity arm
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'INVALID_ARGUMENT'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: GetMetadataBatch rejects a malformed value in the resource identity arm.
    # A value that is neither an encoded resource identity nor a resource address
    # cannot be routed and must produce a per-entry INVALID_ARGUMENT, complementing
    # the plain-address case above.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a get metadata batch request entry for key 'author' with an invalid resource identity
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'INVALID_ARGUMENT'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: GetMetadataBatch rejects the entire request if it has no request entries.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'get-batch-empty-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' get metadata batch returns '<rejected_return_value>'
    Examples:
      | protocol | rejected_return_value |
      | GRPC     | INVALID_ARGUMENT      |
      | REST     | 400                   |

  Scenario Outline: GetMetadataBatch reports a not found error for a nonexistent metadata key entries.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'get-batch-missing-key-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And a get metadata batch request entry for key 'nonexistent' on that address
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: GetMetadataBatch reports with not found error for a nonexistent metadata namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'get-batch-missing-namespace-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a get metadata batch request entry for key 'author' on that address in namespace 'ghost'
    When calling '<protocol>' get metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: QueryMetadata fetches metadata values for a resource.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'query-resource-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    When calling '<protocol>' query metadata on that address returns '<batch_return_value>'
    Then the query metadata response contains '2' entries
    And the query metadata response contains key 'author' with value 'Alice'
    And the query metadata response contains key 'editor' with value 'Bob'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: QueryMetadata fetches metadata values for a resource within a namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'query-namespace-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address in namespace 'custom'
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    When calling '<protocol>' query metadata on that address in namespace 'custom' returns '<batch_return_value>'
    Then the query metadata response contains '1' entries
    And the query metadata response contains key 'author' with value 'Alice'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: QueryMetadata without the namespace is equal to calling it against the "default" namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'query-default-implicit-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address in namespace 'default'
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address in namespace 'default'
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    When calling '<protocol>' query metadata on that address returns '<batch_return_value>'
    Then the query metadata response contains '2' entries
    And the query metadata response contains key 'author' with value 'Alice'
    And the query metadata response contains key 'editor' with value 'Bob'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: QueryMetadata against the "default" namespace is equal to calling it without the namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'query-default-explicit-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    When calling '<protocol>' query metadata on that address in namespace 'default' returns '<batch_return_value>'
    Then the query metadata response contains '2' entries
    And the query metadata response contains key 'author' with value 'Alice'
    And the query metadata response contains key 'editor' with value 'Bob'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: QueryMetadata limits the number of metadata values in the response.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'query-limit-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And an update metadata batch request entry for key 'reviewer' with value 'Charlie' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    When calling '<protocol>' query metadata on that address with a page size of '2' returns '<batch_return_value>'
    Then the query metadata response contains '2' entries
    Examples:
      | protocol | batch_return_value |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch deletes selected metadata values.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-selected-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a delete metadata batch request entry for key 'author' on that address
    And a delete metadata batch request entry for key 'editor' on that address
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded
    And the metadata batch entry at request index '1' succeeded
    And a get metadata batch request entry for key 'author' on that address
    And a get metadata batch request entry for key 'editor' on that address
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    And the metadata batch entry at request index '1' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch deletes metadata values within a namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-namespaces-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Default' on that address
    And an update metadata batch request entry for key 'author' with value 'Alpha' on that address in namespace 'alpha'
    And an update metadata batch request entry for key 'author' with value 'Beta' on that address in namespace 'beta'
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a delete metadata batch request entry for key 'author' on that address in namespace 'alpha'
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' succeeded
    And a get metadata batch request entry for key 'author' on that address
    And a get metadata batch request entry for key 'author' on that address in namespace 'alpha'
    And a get metadata batch request entry for key 'author' on that address in namespace 'beta'
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '3' entries
    And the metadata batch entry at request index '0' succeeded with value 'Default'
    And the metadata batch entry at request index '1' failed with code 'NOT_FOUND'
    And the metadata batch entry at request index '2' succeeded with value 'Beta'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch deletes selected metadata values with precondition.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-precondition-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And we capture the metadata batch entry at request index '0' ETag as 'author_etag'
    And a delete metadata batch request entry for key 'author' using captured ETag 'author_etag' on that address
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' succeeded
    And a get metadata batch request entry for key 'author' on that address
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch reports per-entry failures without aborting the rest of the batch.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-per-entry-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a delete metadata batch request entry for key 'author' using ETag 'stale-etag-value' on that address
    And a delete metadata batch request entry for key 'editor' on that address
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' failed with code 'FAILED_PRECONDITION'
    And the metadata batch entry at request index '1' succeeded
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch reports an invalid argument error for duplicate entries.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-duplicate-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And a delete metadata batch request entry for key 'author' on that address
    And a delete metadata batch request entry for key 'author' on that address
    When calling '<protocol>' delete metadata batch returns '<rejected_return_value>'
    Examples:
      | protocol | rejected_return_value |
      | GRPC     | INVALID_ARGUMENT      |
      | REST     | 400                   |

  Scenario Outline: DeleteMetadataBatch rejects the entire request if it has no request entries.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-empty-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' delete metadata batch returns '<rejected_return_value>'
    Examples:
      | protocol | rejected_return_value |
      | GRPC     | INVALID_ARGUMENT      |
      | REST     | 400                   |

  Scenario Outline: DeleteMetadataBatch reports success for requests against nonexistent metadata keys.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-missing-key-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And a delete metadata batch request entry for key 'nonexistent' on that address
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' succeeded
    And a get metadata batch request entry for key 'nonexistent' on that address
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch reports success for requests against nonexistent metadata namespaces.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-missing-namespace-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And a delete metadata batch request entry for key 'author' on that address in namespace 'ghost'
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' succeeded
    And a get metadata batch request entry for key 'author' on that address in namespace 'ghost'
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch reports a precondition failed error on a precondition mismatch.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-batch-mismatch-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a delete metadata batch request entry for key 'author' using ETag 'stale-etag-value' on that address
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'FAILED_PRECONDITION'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch against the "default" namespace is equal to calling it without the namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-default-explicit-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a delete metadata batch request entry for key 'author' on that address in namespace 'default'
    And a delete metadata batch request entry for key 'editor' on that address in namespace 'default'
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded
    And the metadata batch entry at request index '1' succeeded
    And a get metadata batch request entry for key 'author' on that address
    And a get metadata batch request entry for key 'editor' on that address
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    And the metadata batch entry at request index '1' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: DeleteMetadataBatch without the namespace is equal to calling it against the "default" namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'delete-default-implicit-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And a delete metadata batch request entry for key 'author' on that address
    And a delete metadata batch request entry for key 'editor' on that address
    When calling '<protocol>' delete metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded
    And the metadata batch entry at request index '1' succeeded
    And a get metadata batch request entry for key 'author' on that address in namespace 'default'
    And a get metadata batch request entry for key 'editor' on that address in namespace 'default'
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' failed with code 'NOT_FOUND'
    And the metadata batch entry at request index '1' failed with code 'NOT_FOUND'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: UpdateMetadataBatch updates selected metadata values.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-batch-selected-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    When calling '<protocol>' update metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with an ETag
    And the metadata batch entry at request index '1' succeeded with an ETag
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: UpdateMetadataBatch updates metadata within a namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-batch-namespaces-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Default' on that address
    And an update metadata batch request entry for key 'author' with value 'Alpha' on that address in namespace 'alpha'
    And an update metadata batch request entry for key 'author' with value 'Beta' on that address in namespace 'beta'
    When calling '<protocol>' update metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '3' entries
    And the metadata batch entry at request index '0' succeeded with an ETag
    And the metadata batch entry at request index '1' succeeded with an ETag
    And the metadata batch entry at request index '2' succeeded with an ETag
    And a get metadata batch request entry for key 'author' on that address
    And a get metadata batch request entry for key 'author' on that address in namespace 'alpha'
    And a get metadata batch request entry for key 'author' on that address in namespace 'beta'
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '3' entries
    And the metadata batch entry at request index '0' succeeded with value 'Default'
    And the metadata batch entry at request index '1' succeeded with value 'Alpha'
    And the metadata batch entry at request index '2' succeeded with value 'Beta'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: UpdateMetadataBatch updates selected metadata values with precondition.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-batch-precondition-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And we capture the metadata batch entry at request index '0' ETag as 'author_etag'
    And an update metadata batch request entry for key 'author' with value 'Updated' using captured ETag 'author_etag' on that address
    When calling '<protocol>' update metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' succeeded with an ETag
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: UpdateMetadataBatch reports an invalid argument error for duplicate entries.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-batch-duplicate-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'author' with value 'Bob' on that address
    When calling '<protocol>' update metadata batch returns '<rejected_return_value>'
    Examples:
      | protocol | rejected_return_value |
      | GRPC     | INVALID_ARGUMENT      |
      | REST     | 400                   |

  Scenario Outline: UpdateMetadataBatch rejects the entire request if it has no request entries.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-batch-empty-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    When calling '<protocol>' update metadata batch returns '<rejected_return_value>'
    Examples:
      | protocol | rejected_return_value |
      | GRPC     | INVALID_ARGUMENT      |
      | REST     | 400                   |

  Scenario Outline: UpdateMetadataBatch reports a precondition failed error on a precondition mismatch.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-batch-mismatch-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And calling '<protocol>' update metadata batch returns '<batch_return_value>'
    And an update metadata batch request entry for key 'author' with value 'Updated' using ETag 'stale-etag-value' on that address
    When calling '<protocol>' update metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '1' entries
    And the metadata batch entry at request index '0' failed with code 'FAILED_PRECONDITION'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: UpdateMetadataBatch against the "default" namespace is equal to calling it without the namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-default-explicit-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address in namespace 'default'
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address in namespace 'default'
    When calling '<protocol>' update metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with an ETag
    And the metadata batch entry at request index '1' succeeded with an ETag
    And a get metadata batch request entry for key 'author' on that address
    And a get metadata batch request entry for key 'editor' on that address
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with value 'Alice'
    And the metadata batch entry at request index '1' succeeded with value 'Bob'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |

  Scenario Outline: UpdateMetadataBatch without the namespace is equal to calling it against the "default" namespace.
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'batch-metadata'
    And a resource address for 'update-default-implicit-<protocol>.usd'
    And an object of size '1024' exists at that address and is readable
    And an update metadata batch request entry for key 'author' with value 'Alice' on that address
    And an update metadata batch request entry for key 'editor' with value 'Bob' on that address
    When calling '<protocol>' update metadata batch returns '<batch_return_value>'
    Then the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with an ETag
    And the metadata batch entry at request index '1' succeeded with an ETag
    And a get metadata batch request entry for key 'author' on that address in namespace 'default'
    And a get metadata batch request entry for key 'editor' on that address in namespace 'default'
    And calling '<protocol>' get metadata batch returns '<batch_return_value>'
    And the metadata batch response contains '2' entries
    And the metadata batch entry at request index '0' succeeded with value 'Alice'
    And the metadata batch entry at request index '1' succeeded with value 'Bob'
    Examples:
      | protocol | batch_return_value |
      | GRPC     | OK                 |
      | REST     | 200                |
