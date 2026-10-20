# Observability

This module provides observability utilities including metrics, logging, panic handling, and initialization helpers.

## Source

This code is copied from the the Omniverse Core Frontend coreapi-framework-prototype in the Omniverse Core Frontend repository.

## Temporary Solution

This code is currently being extracted into a standalone library intended to be shared across Omniverse services. However, that library is not yet ready for use.

As an interim solution, we have copied the necessary code into this crate. Once the shared observability library is available and stable, we will migrate to using that library instead of maintaining this local copy.

## Contents

- `metrics.rs` - Metric registration and collection utilities
- `logger.rs` - Logging configuration and setup
- `crash.rs` - Panic handler installation
- `init.rs` - Observability initialization helpers

