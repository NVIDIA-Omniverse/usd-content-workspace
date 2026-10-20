# Notifications

Back to the [usd-content-workspace README](../README.md).

- [Protobuf contracts](api/README.md) for the publisher and consumer APIs
- [API usage guide](reference/event-consumer-service/docs/index.md) for both services: getting started, publishing and consuming events, permissions, and the API reference
- [Event Aggregation Service](reference/event-aggregation-service/README.md) — publishes events to RabbitMQ
- [Event Consumer Service](reference/event-consumer-service/README.md) — delivers published events to consumers
- [Shared library](reference/common/README.md) (`notification_common`) used by both services

Each service carries its OpenAPI specification, `conformance_tests/` suite, Helm charts, `BUILDING.md`, and `AGENTS.md`. Versions are in each service's [aggregation](reference/event-aggregation-service/CHANGELOG.md) and [consumer](reference/event-consumer-service/CHANGELOG.md) changelogs.
