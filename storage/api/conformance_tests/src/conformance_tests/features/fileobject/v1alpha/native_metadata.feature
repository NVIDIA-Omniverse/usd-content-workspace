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

Feature: Native metadata

  Background:
    Given a new test namespace called 'native_metadata_test_alpha'
    And a connection to the storage service
    And an authenticated user

  Scenario Outline: Native metadata support query reports supported services
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'support-query-supported-<protocol>.bin'
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    Then calling '<protocol>' native metadata support on that address returns '<support_return_value>' and supports native metadata is 'true'
    Examples:
      | protocol | support_return_value |
      | GRPC     | OK                   |
      | REST     | 200                  |

  # If the support query endpoint is absent, conformance treats native metadata as unavailable
  # and skips native metadata scenarios. This scenario is for services that implement the
  # query and explicitly report that native metadata is unsupported for the address.
  Scenario Outline: Native metadata support query reports present-but-unsupported services
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'support-query-unsupported-<protocol>.bin'
    And no object exists at that address
    And the '<protocol>' service does not support native metadata for fileobject
    Then calling '<protocol>' native metadata support on that address returns '<support_return_value>' and supports native metadata is 'false'
    Examples:
      | protocol | support_return_value |
      | GRPC     | OK                   |
      | REST     | 200                  |

  Scenario Outline: Native metadata supported direct body write round trips through FileObject results
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'supported-roundtrip-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'canonical.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with 'canonical' native metadata returns '<write_return_value>'
    Then the last response contains 'canonical' native metadata
    When we memorize the last response as 'written_version'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' Read on the memorized response 'written_version' returns '<read_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' ReadFromAddress on that address the service returns '<read_from_address_return_value>'
    Then the last response contains 'canonical' native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address contains 'canonical' native metadata
    Examples:
      | protocol | write_return_value | stat_return_value | read_return_value | read_from_address_return_value | enumerate_return_value |
      | GRPC     | OK                 | OK                | OK                | OK                             | OK                     |
      | REST     | 201                | 204               | 200               | 200                            | 200                    |

  Scenario Outline: Native metadata supported direct body write round trips through ListStat results
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And the '<protocol>' service supports the 'filefolder' API in version 'v1alpha' for feature 'filefolder'
    Given a resource address 'supported-liststat-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'liststat.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with 'canonical' native metadata returns '<write_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' ListStat with native metadata on the memorized address 'parent_address' returns '<liststat_return_value>'
    Then the last ListStat result for that address contains 'canonical' native metadata
    Examples:
      | protocol | write_return_value | liststat_return_value |
      | GRPC     | OK                 | OK                    |
      | REST     | 201                | 200                   |

  Scenario Outline: Native metadata supported omitted and explicit empty writes do not expose stale client metadata
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'supported-empty-<protocol>-<metadata_case>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'empty.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with 'canonical' native metadata returns '<write_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' write on that address with '<metadata_case>' native metadata returns '<write_return_value>'
    Then the last response does not contain 'canonical' native metadata
    When we memorize the last response as 'latest_version'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response does not contain 'canonical' native metadata
    When calling '<protocol>' Read on the memorized response 'latest_version' returns '<read_return_value>'
    Then the last response does not contain 'canonical' native metadata
    When calling '<protocol>' ReadFromAddress on that address the service returns '<read_from_address_return_value>'
    Then the last response does not contain 'canonical' native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address does not contain 'canonical' native metadata
    Examples:
      | protocol | metadata_case | write_return_value | stat_return_value | read_return_value | read_from_address_return_value | enumerate_return_value |
      | GRPC     | omitted       | OK                 | OK                | OK                | OK                             | OK                     |
      | GRPC     | empty         | OK                 | OK                | OK                | OK                             | OK                     |
      | REST     | omitted       | 201                | 204               | 200               | 200                            | 200                    |
      | REST     | empty         | 201                | 204               | 200               | 200                            | 200                    |

  Scenario Outline: Native metadata supported writes keep metadata scoped to each version
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'supported-version-scope-<protocol>.bin'
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a blob of '128' bytes
    When calling '<protocol>' write on that address with 'canonical' native metadata returns '<write_return_value>'
    Then the last response contains 'canonical' native metadata
    When we memorize the last response as 'first_version'
    Given a blob of '256' bytes
    When calling '<protocol>' write on that address with 'alternate' native metadata returns '<write_return_value>'
    Then the last response contains 'alternate' native metadata
    When we memorize the last response as 'second_version'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains 'alternate' native metadata
    When calling '<protocol>' Read on the memorized response 'first_version' returns '<read_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' Read on the memorized response 'second_version' returns '<read_return_value>'
    Then the last response contains 'alternate' native metadata
    Examples:
      | protocol | write_return_value | stat_return_value | read_return_value |
      | GRPC     | OK                 | OK                | OK                |
      | REST     | 201                | 204               | 200               |

  Scenario Outline: Native metadata supported writes round trip through EnumerateVersions
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And the '<protocol>' service supports the 'versioning' API in version 'v1alpha' for feature 'versioning'
    Given a resource address for 'supported-enumerate-versions-<protocol>.bin'
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a blob of '128' bytes
    When calling '<protocol>' write on that address with 'canonical' native metadata returns '<write_return_value>'
    Then the last response contains 'canonical' native metadata
    When we memorize the last response as 'first_version'
    Given a blob of '256' bytes
    When calling '<protocol>' write on that address with 'alternate' native metadata returns '<write_return_value>'
    Then the last response contains 'alternate' native metadata
    When we memorize the last response as 'second_version'
    When calling '<protocol>' EnumerateVersions exhaustively on that address returns '<enumerate_versions_return_value>'
    Then the EnumerateVersions result for memorized response 'first_version' contains 'canonical' native metadata
    And the EnumerateVersions result for memorized response 'second_version' contains 'alternate' native metadata
    Examples:
      | protocol | write_return_value | enumerate_versions_return_value |
      | GRPC     | OK                 | OK                              |
      | REST     | 201                | 200                             |

  Scenario Outline: Native metadata supported FileObject metadata is independent from MetadataService metadata
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And the '<protocol>' service supports the 'metadata' API in version 'v1alpha' for feature 'metadata'
    Given a resource address for 'supported-metadata-service-independent-<protocol>.bin'
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a blob of '128' bytes
    When calling '<protocol>' write on that address with 'shared-key' native metadata returns '<write_return_value>'
    Then the last response contains 'shared-key' native metadata
    When calling '<protocol>' update metadata key 'shared_key' with value 'metadata service value' on that address returns '<metadata_update_return_value>'
    And calling '<protocol>' get metadata with keys '["shared_key"]' on that address returns '<metadata_get_return_value>'
    Then the metadata response contains key 'shared_key' with value 'metadata service value'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains 'shared-key' native metadata
    And the last response contains native metadata key 'shared_key' with value 'fileobject native value'
    Examples:
      | protocol | write_return_value | metadata_update_return_value | metadata_get_return_value | stat_return_value |
      | GRPC     | OK                 | OK                           | OK                        | OK                |
      | REST     | 201                | 201                          | 200                       | 204               |

  Scenario Outline: Native metadata supported redirect upload round trips through FileObject results
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'supported-redirect-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'redirect.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a 'medium' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with 'redirect' upload preference and 'canonical' native metadata the first roundtrip returns '<write_return_value>'
    Then uploading a file following a redirect succeeds
    When calling '<protocol>' complete redirect upload returns '<completion_return_value>'
    Then the last response contains 'canonical' native metadata
    When we memorize the last response as 'written_version'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' Read on the memorized response 'written_version' returns '<read_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' ReadFromAddress on that address the service returns '<read_from_address_return_value>'
    Then the last response contains 'canonical' native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address contains 'canonical' native metadata
    Examples:
      | protocol | write_return_value | completion_return_value | stat_return_value | read_return_value | read_from_address_return_value | enumerate_return_value |
      | GRPC     | OK                 | OK                      | OK                | OK                | OK                             | OK                     |
      | REST     | 300                | 200                     | 204               | 200               | 200                            | 200                    |

  Scenario Outline: Native metadata supported multipart upload round trips through FileObject results
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'supported-multipart-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'multipart.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a 'large' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with 'multipart' upload preference and 'canonical' native metadata the first roundtrip returns '<write_return_value>'
    Then uploading a file with '<protocol>' in multiple parts returns '<completion_return_value>' on completion
    And the last response contains 'canonical' native metadata
    When we memorize the last response as 'written_version'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' Read on the memorized response 'written_version' returns '<read_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' ReadFromAddress on that address the service returns '<read_from_address_return_value>'
    Then the last response contains 'canonical' native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address contains 'canonical' native metadata
    Examples:
      | protocol | write_return_value | completion_return_value | stat_return_value | read_return_value | read_from_address_return_value | enumerate_return_value |
      | GRPC     | OK                 | OK                      | OK                | OK                | OK                             | OK                     |
      | REST     | 300                | 200                     | 204               | 200               | 200                            | 200                    |

  Scenario Outline: Native metadata supported multipart upload binds metadata to upload state
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'supported-multipart-binding-<protocol>.bin'
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a 'large' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with 'multipart' upload preference and 'canonical' native metadata the first roundtrip returns '<write_return_value>'
    And we memorize the last response as 'canonical_upload'
    When calling '<protocol>' write on that address with 'multipart' upload preference and 'alternate' native metadata the first roundtrip returns '<write_return_value>'
    And we memorize the last response as 'alternate_upload'
    Then uploading a file with '<protocol>' in multiple parts using the memorized write response 'alternate_upload' returns '<completion_return_value>' on completion
    And the last response contains 'alternate' native metadata
    When we memorize the last response as 'alternate_version'
    Then uploading a file with '<protocol>' in multiple parts using the memorized write response 'canonical_upload' returns '<completion_return_value>' on completion
    And the last response contains 'canonical' native metadata
    When we memorize the last response as 'canonical_version'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' Read on the memorized response 'alternate_version' returns '<read_return_value>'
    Then the last response contains 'alternate' native metadata
    When calling '<protocol>' Read on the memorized response 'canonical_version' returns '<read_return_value>'
    Then the last response contains 'canonical' native metadata
    Examples:
      | protocol | write_return_value | completion_return_value | stat_return_value | read_return_value |
      | GRPC     | OK                 | OK                      | OK                | OK                |
      | REST     | 300                | 200                     | 204               | 200               |

  Scenario Outline: Native metadata supported REST write validation rejects invalid input before creating state
    Given the service speaks 'REST'
    And the 'REST' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'supported-invalid-rest-<upload_preference>-<metadata_case>.bin'
    And no object exists at that address
    And the 'REST' service supports native metadata for fileobject
    And a blob of '128' bytes
    When calling 'REST' write on that address with '<upload_preference>' upload preference and '<metadata_case>' native metadata the first roundtrip returns '400'
    Then calling 'REST' stat on that address returns '404'
    Examples:
      | upload_preference | metadata_case      |
      | body              | malformed-json     |
      | body              | non-object-json    |
      | body              | non-finite         |
      | redirect          | malformed-json     |
      | redirect          | non-object-json    |
      | redirect          | non-finite         |
      | multipart         | malformed-json     |
      | multipart         | non-object-json    |
      | multipart         | non-finite         |

  Scenario Outline: Native metadata supported gRPC write validation rejects invalid input before creating state
    Given the service speaks 'GRPC'
    And the 'GRPC' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'supported-invalid-grpc-<upload_preference>-<metadata_case>.bin'
    And no object exists at that address
    And the 'GRPC' service supports native metadata for fileobject
    And a blob of '128' bytes
    When calling 'GRPC' write on that address with '<upload_preference>' upload preference and '<metadata_case>' native metadata the first roundtrip returns 'INVALID_ARGUMENT'
    Then calling 'GRPC' stat on that address returns 'NOT_FOUND'
    Examples:
      | upload_preference | metadata_case      |
      | body              | non-finite         |
      | redirect          | non-finite         |
      | multipart         | non-finite         |

  Scenario Outline: Native metadata supported invalid write does not mutate an existing object
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'supported-invalid-existing-<protocol>-<metadata_case>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'existing.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service supports native metadata for fileobject
    And a native metadata test object of size '1024' with rand seed '90210' and 'canonical' native metadata exists at that address using '<protocol>'
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    And we memorize the last response as 'before_write_stat'
    Given a blob of '2048' bytes
    When calling '<protocol>' write on that address with '<upload_preference>' upload preference and '<metadata_case>' native metadata the first roundtrip returns '<write_return_value>'
    Then calling '<protocol>' stat on that address returns '<stat_return_value>'
    And the resource identity returned is the same as the one in the memorized response 'before_write_stat'
    And the '<protocol>' result's resource info has size '1024'
    Then the last response contains 'canonical' native metadata
    When calling '<protocol>' ReadFromAddress with mode 'body' downloads the data using the specified preference and it has the correct content for rand seed '90210'
    Then the last response contains 'canonical' native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address contains 'canonical' native metadata
    Examples:
      | protocol | upload_preference | metadata_case      | write_return_value | stat_return_value | enumerate_return_value |
      | GRPC     | body              | non-finite         | INVALID_ARGUMENT   | OK                | OK                     |
      | REST     | body              | malformed-json     | 400                | 204               | 200                    |

  Scenario Outline: Native metadata supported write preserves invalid-address precedence
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'supported-invalid-guard-<protocol>.bin'
    And the '<protocol>' service supports native metadata for fileobject
    And a blob of '128' bytes
    And an invalid resource address
    Then calling '<protocol>' write on that address with 'canonical' native metadata returns '<return_value>'
    Examples:
      | protocol | return_value     |
      | GRPC     | INVALID_ARGUMENT |
      | REST     | 400              |

  Scenario Outline: Native metadata supported write preserves write-authorization precedence
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'supported-no-write-guard-<protocol>.bin'
    And the '<protocol>' service supports native metadata for fileobject
    Given a resource address for 'supported-no-write-<protocol>.bin'
    And an object of size '1024' exists at that address and is readable
    And user has no permissions to write at that address
    And a blob of '128' bytes
    Then calling '<protocol>' write on that address with 'canonical' native metadata returns '<return_value>'
    Examples:
      | protocol | return_value      |
      | GRPC     | PERMISSION_DENIED |
      | REST     | 403               |

  Scenario Outline: Native metadata support query rejects malformed addresses
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given an invalid resource address
    Then calling '<protocol>' native metadata support on that address returns '<return_value>'
    Examples:
      | protocol | return_value     |
      | GRPC     | INVALID_ARGUMENT |
      | REST     | 400              |

  Scenario Outline: Native metadata support query rejects version addresses
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'support-query-version-<protocol>.bin'
    And a version-like resource address for that address
    Then calling '<protocol>' native metadata support on that address returns '<return_value>'
    Examples:
      | protocol | return_value     |
      | GRPC     | INVALID_ARGUMENT |
      | REST     | 400              |

  Scenario Outline: Native metadata support query preserves write authorization errors
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'support-query-no-write-<protocol>.bin'
    And an object of size '1024' exists at that address and is readable
    And user has no permissions to write at that address
    Then calling '<protocol>' native metadata support on that address returns '<return_value>'
    Examples:
      | protocol | return_value      |
      | GRPC     | PERMISSION_DENIED |
      | REST     | 403               |

  Scenario Outline: Native metadata unsupported ordinary write exposes empty response metadata
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'unsupported-ordinary-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'ordinary.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service does not support native metadata for fileobject
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with data returns a created response
    Then the last response contains empty or absent native metadata
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains empty or absent native metadata
    Then calling '<protocol>' Read on that response returns '<read_return_value>'
    Then the last response contains empty or absent native metadata
    When calling '<protocol>' ReadFromAddress on that address the service returns '<read_from_address_return_value>'
    Then the last response contains empty or absent native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address contains empty or absent native metadata
    Examples:
      | protocol | stat_return_value | read_return_value | read_from_address_return_value | enumerate_return_value |
      | GRPC     | OK                | OK                | OK                             | OK                     |
      | REST     | 204               | 200               | 200                            | 200                    |

  Scenario Outline: Native metadata unsupported ordinary write exposes empty ListStat metadata
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    And the '<protocol>' service supports the 'filefolder' API in version 'v1alpha' for feature 'filefolder'
    Given a resource address 'unsupported-liststat-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'ordinary.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service does not support native metadata for fileobject
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with data returns a created response
    Then the last response contains empty or absent native metadata
    When calling '<protocol>' ListStat with native metadata on the memorized address 'parent_address' returns '<liststat_return_value>'
    Then the last ListStat result for that address contains empty or absent native metadata
    Examples:
      | protocol | liststat_return_value |
      | GRPC     | OK                    |
      | REST     | 200                   |

  Scenario Outline: Native metadata unsupported explicit empty write exposes empty response metadata
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'unsupported-empty-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'empty.bin' within the given address
    And no object exists at that address
    And the '<protocol>' service does not support native metadata for fileobject
    And a 'small' blob according to the upload options for that address, using '<protocol>'
    When calling '<protocol>' write on that address with 'empty' native metadata returns '<write_return_value>'
    Then the last response contains empty or absent native metadata
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    Then the last response contains empty or absent native metadata
    Then calling '<protocol>' Read on that response returns '<read_return_value>'
    Then the last response contains empty or absent native metadata
    When calling '<protocol>' ReadFromAddress on that address the service returns '<read_from_address_return_value>'
    Then the last response contains empty or absent native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address contains empty or absent native metadata
    Examples:
      | protocol | write_return_value | stat_return_value | read_return_value | read_from_address_return_value | enumerate_return_value |
      | GRPC     | OK                 | OK                | OK                | OK                             | OK                     |
      | REST     | 201                | 204               | 200               | 200                            | 200                    |

  Scenario Outline: Native metadata unsupported non-empty write does not create an object
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unsupported-new-object-<protocol>.bin'
    And no object exists at that address
    And the '<protocol>' service does not support native metadata for fileobject
    And a blob of '128' bytes
    When calling '<protocol>' write on that address with 'canonical' native metadata returns '<write_return_value>'
    Then calling '<protocol>' stat on that address returns '<stat_return_value>'
    Examples:
      | protocol | write_return_value | stat_return_value |
      | GRPC     | UNIMPLEMENTED      | NOT_FOUND         |
      | REST     | 501                | 404               |

  Scenario Outline: Native metadata unsupported non-empty write does not mutate an existing object
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address 'unsupported-existing-<protocol>' which is enumerable
    And memorizing that resource address as 'parent_address'
    And a new object address 'existing.bin' within the given address
    And a test object of size '1024' with rand seed '90210' at that address
    And the '<protocol>' service does not support native metadata for fileobject
    When calling '<protocol>' stat on that address returns '<stat_return_value>'
    And we memorize the last response as 'before_write_stat'
    Given a blob of '2048' bytes
    When calling '<protocol>' write on that address with 'canonical' native metadata returns '<write_return_value>'
    Then calling '<protocol>' stat on that address returns '<stat_return_value>'
    And the resource identity returned is the same as the one in the memorized response 'before_write_stat'
    And the '<protocol>' result's resource info has size '1024'
    When calling '<protocol>' ReadFromAddress with mode 'body' downloads the data using the specified preference and it has the correct content for rand seed '90210'
    Then the last response contains empty or absent native metadata
    Then calling '<protocol>' Enumerate exhaustively on the address 'parent_address' returns '<enumerate_return_value>'
    And the last enumerate result for that address contains empty or absent native metadata
    Examples:
      | protocol | write_return_value | stat_return_value | enumerate_return_value |
      | GRPC     | UNIMPLEMENTED      | OK                | OK                     |
      | REST     | 501                | 204               | 200                    |

  Scenario Outline: Native metadata unsupported write preserves invalid-address precedence
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unsupported-invalid-guard-<protocol>.bin'
    And the '<protocol>' service does not support native metadata for fileobject
    And a blob of '128' bytes
    And an invalid resource address
    Then calling '<protocol>' write on that address with 'canonical' native metadata returns '<return_value>'
    Examples:
      | protocol | return_value     |
      | GRPC     | INVALID_ARGUMENT |
      | REST     | 400              |

  Scenario Outline: Native metadata unsupported write preserves write-authorization precedence
    Given the service speaks '<protocol>'
    And the '<protocol>' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unsupported-no-write-guard-<protocol>.bin'
    And the '<protocol>' service does not support native metadata for fileobject
    Given a resource address for 'unsupported-no-write-<protocol>.bin'
    And an object of size '1024' exists at that address and is readable
    And user has no permissions to write at that address
    And a blob of '128' bytes
    Then calling '<protocol>' write on that address with 'canonical' native metadata returns '<return_value>'
    Examples:
      | protocol | return_value      |
      | GRPC     | PERMISSION_DENIED |
      | REST     | 403               |

  Scenario Outline: Native metadata unsupported REST write validates invalid native metadata before unsupported
    Given the service speaks 'REST'
    And the 'REST' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unsupported-invalid-rest-<metadata_case>.bin'
    And no object exists at that address
    And the 'REST' service does not support native metadata for fileobject
    And a blob of '128' bytes
    When calling 'REST' write on that address with '<metadata_case>' native metadata returns '400'
    Then calling 'REST' stat on that address returns '404'
    Examples:
      | metadata_case      |
      | malformed-json     |
      | non-object-json    |
      | non-finite         |

  Scenario Outline: Native metadata unsupported gRPC write validates invalid native metadata before unsupported
    Given the service speaks 'GRPC'
    And the 'GRPC' service supports the 'fileobject' API in version 'v1alpha' for feature 'fileobject'
    Given a resource address for 'unsupported-invalid-grpc-<metadata_case>.bin'
    And no object exists at that address
    And the 'GRPC' service does not support native metadata for fileobject
    And a blob of '128' bytes
    When calling 'GRPC' write on that address with '<metadata_case>' native metadata returns 'INVALID_ARGUMENT'
    Then calling 'GRPC' stat on that address returns 'NOT_FOUND'
    Examples:
      | metadata_case      |
      | non-finite         |
