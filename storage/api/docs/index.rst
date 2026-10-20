==========================================
Omniverse Storage API Developer Guide
==========================================

The Omniverse Storage API are gRPC and REST API specifications for the data access layer within the Omniverse platform.

Using the Storage API, you are now able to overlay a `Storage Adapter` on top of your existing storage system and interface directly with Omniverse.

This guide is intended for developers that want to learn how to use or build adapters for customer storage systems to be used within the Omniverse platform.

The Service Example will show you how to build a `Storage Adapter` that can be used within the Omniverse platform.

The Client Example will show you how to connect to a `Storage Adapter` and access your data. The data access will be the same for any Storage Adapter that passes our Conformance Test Suite.

A full reference implementation of the APIs is provided in the `filesystem_example` directory, which serves versioned files
from a filetree build to store head content and versions in separate subtrees.

To test any new storage service implementation for conformance to the API, especially return values in error conditions,
there is a runnable conformance test suite in the `conformance_tests` subdirectory.

.. toctree::
    :maxdepth: 2

    user_manual
    
    client_example_include.rst

    service_example_include.rst

    Filesystem example <local_filesystem_example_include.rst>

    Conformance test suite <conformance_tests_include.rst>

    Changes <changes.rst>